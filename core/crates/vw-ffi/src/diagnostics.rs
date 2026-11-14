//! Bounded, privacy-safe diagnostics from the core.
//!
//! The public error types are deliberately coarse (`CoreError::Invalid`), which
//! used to make a failure in use impossible to explain. This module keeps the
//! reason: every conversion into a public error records where it came from.
//!
//! Privacy rule (BUILD_SPECIFICATION 4.21): an event holds only compile-time
//! strings and numbers. There is no way to record a path, an identifier,
//! document text or any other run-time string, because the API takes
//! `&'static str` and `u64` only. Events live in a fixed-size in-memory ring
//! and leave the process only when the app asks for a snapshot.
use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard, OnceLock},
    time::Instant,
};

const CAPACITY: usize = 512;

/// What an event's number means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CoreDiagnosticKind {
    /// An operation was refused or failed; `value` is 0.
    Failure,
    /// A measured duration; `value` is microseconds.
    DurationMicros,
    /// A size or count; `value` is the number.
    Count,
}

/// One diagnostic event. `area` and `name` come from a fixed set compiled into
/// the core, never from run-time data.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CoreDiagnosticEvent {
    /// Position in the process-wide sequence, starting at 1.
    pub sequence: u64,
    /// Milliseconds since the first event of this process.
    pub elapsed_ms: u64,
    pub kind: CoreDiagnosticKind,
    pub area: String,
    pub name: String,
    pub value: u64,
}

/// The retained events after a given sequence number.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CoreDiagnostics {
    pub events: Vec<CoreDiagnosticEvent>,
    /// How many older events the ring has already discarded.
    pub dropped: u64,
    /// The newest sequence number issued; pass it back to read only later events.
    pub latest_sequence: u64,
}

struct Entry {
    sequence: u64,
    elapsed_ms: u64,
    kind: CoreDiagnosticKind,
    area: &'static str,
    name: &'static str,
    value: u64,
}

struct Ring {
    started: Instant,
    events: VecDeque<Entry>,
    issued: u64,
    dropped: u64,
}

fn ring() -> MutexGuard<'static, Ring> {
    static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
    RING.get_or_init(|| {
        Mutex::new(Ring {
            started: Instant::now(),
            events: VecDeque::with_capacity(CAPACITY),
            issued: 0,
            dropped: 0,
        })
    })
    .lock()
    // A panic while holding this lock cannot leave the ring inconsistent:
    // every update below is a single push or pop.
    .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn push(kind: CoreDiagnosticKind, area: &'static str, name: &'static str, value: u64) {
    let mut ring = ring();
    if ring.events.len() == CAPACITY {
        ring.events.pop_front();
        ring.dropped = ring.dropped.saturating_add(1);
    }
    ring.issued = ring.issued.saturating_add(1);
    let entry = Entry {
        sequence: ring.issued,
        elapsed_ms: u64::try_from(ring.started.elapsed().as_millis()).unwrap_or(u64::MAX),
        kind,
        area,
        name,
        value,
    };
    ring.events.push_back(entry);
}

/// Record why an operation was refused or failed.
pub(crate) fn failure(area: &'static str, cause: &'static str) {
    push(CoreDiagnosticKind::Failure, area, cause, 0);
}

/// Record a size or count.
pub(crate) fn count(area: &'static str, name: &'static str, value: u64) {
    push(CoreDiagnosticKind::Count, area, name, value);
}

/// Measures from creation to drop and records the duration.
pub(crate) struct Timer {
    area: &'static str,
    name: &'static str,
    started: Instant,
}

/// Start timing a named stage; the duration is recorded when the value drops.
pub(crate) fn timer(area: &'static str, name: &'static str) -> Timer {
    Timer {
        area,
        name,
        started: Instant::now(),
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let micros = u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX);
        push(
            CoreDiagnosticKind::DurationMicros,
            self.area,
            self.name,
            micros,
        );
    }
}

/// The retained core diagnostics newer than `after_sequence` (0 for all).
/// Reading does not remove anything.
#[uniffi::export]
pub fn core_diagnostics(after_sequence: u64) -> CoreDiagnostics {
    let ring = ring();
    CoreDiagnostics {
        events: ring
            .events
            .iter()
            .filter(|entry| entry.sequence > after_sequence)
            .map(|entry| CoreDiagnosticEvent {
                sequence: entry.sequence,
                elapsed_ms: entry.elapsed_ms,
                kind: entry.kind,
                area: entry.area.to_owned(),
                name: entry.name.to_owned(),
                value: entry.value,
            })
            .collect(),
        dropped: ring.dropped,
        latest_sequence: ring.issued,
    }
}

// Static causes for the errors that public conversions would otherwise erase.

pub(crate) fn io_cause(error: &std::io::Error) -> &'static str {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound => "file not found",
        ErrorKind::PermissionDenied => "permission denied",
        ErrorKind::AlreadyExists => "file already exists",
        ErrorKind::StorageFull => "storage full",
        ErrorKind::QuotaExceeded => "storage quota exceeded",
        ErrorKind::ReadOnlyFilesystem => "read-only file system",
        ErrorKind::InvalidData => "invalid data",
        ErrorKind::InvalidInput => "invalid input",
        ErrorKind::UnexpectedEof => "unexpected end of file",
        ErrorKind::WriteZero => "write returned zero",
        ErrorKind::TimedOut => "timed out",
        ErrorKind::Interrupted => "interrupted",
        ErrorKind::OutOfMemory => "out of memory",
        ErrorKind::ResourceBusy => "resource busy",
        ErrorKind::NotADirectory => "not a directory",
        ErrorKind::IsADirectory => "is a directory",
        ErrorKind::DirectoryNotEmpty => "directory not empty",
        ErrorKind::CrossesDevices => "crosses devices",
        _ => "other input or output error",
    }
}

pub(crate) fn model_cause(error: &vw_model::ModelError) -> &'static str {
    use vw_model::ModelError;
    match error {
        ModelError::InvalidId(what) | ModelError::Invalid(what) | ModelError::Missing(what) => what,
        ModelError::Entropy => "random source unavailable",
        ModelError::UnsupportedKind(_) => "unsupported document kind",
        ModelError::UnsupportedSchema(_) => "unsupported document schema",
        ModelError::Serialization(_) => "document serialization",
        ModelError::Geometry(_) => "document geometry",
    }
}

pub(crate) fn ops_cause(error: &vw_ops::OpsError) -> &'static str {
    use vw_ops::OpsError;
    match error {
        OpsError::Model(inner) => model_cause(inner),
        OpsError::Serialization(_) => "operation serialization",
        OpsError::Invalid(what) => what,
        OpsError::NotFound => "operation target not found",
        OpsError::AlreadyExists => "operation target already exists",
        OpsError::IdCollision => "transaction id collision",
        OpsError::DeviceMismatch => "device mismatch",
        OpsError::RevisionMismatch => "revision mismatch",
        OpsError::Locked => "target locked",
        OpsError::CounterExhausted => "counter exhausted",
        OpsError::UndoOwnership => "undo ownership",
    }
}

pub(crate) fn store_cause(error: &vw_store::StoreError) -> &'static str {
    use vw_store::StoreError;
    match error {
        StoreError::Io(inner) => io_cause(inner),
        StoreError::Database(_) => "database failure",
        StoreError::Json(_) => "store serialization",
        StoreError::Zip(_) => "archive format",
        StoreError::Model(inner) => model_cause(inner),
        StoreError::Operation(inner) => ops_cause(inner),
        StoreError::Invalid(what) | StoreError::Corrupt(what) => what,
        StoreError::UnsupportedSchema(_) => "unsupported store schema",
        StoreError::Busy => "store busy",
        StoreError::LowSpace => "low disk space",
    }
}

pub(crate) fn ink_cause(error: &vw_ink::InkError) -> &'static str {
    use vw_ink::InkError;
    match error {
        InkError::UnsupportedVersion(_) => "unsupported brush version",
        InkError::Invalid(what) => what,
        InkError::TimeOrder => "sample times out of order",
        InkError::Closed => "stroke closed",
        InkError::EmptyStroke => "empty stroke",
        InkError::ResourceLimit => "stroke resource limit",
        InkError::Allocation => "stroke allocation",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn events_are_ordered_bounded_and_readable_after_a_sequence() {
        let before = core_diagnostics(0).latest_sequence;
        failure("test", "first cause");
        count("test", "items", 7);
        drop(timer("test", "stage"));
        let snapshot = core_diagnostics(before);
        let mine: Vec<_> = snapshot
            .events
            .iter()
            .filter(|event| event.area == "test")
            .collect();
        assert_eq!(mine.len(), 3);
        assert_eq!(mine[0].kind, CoreDiagnosticKind::Failure);
        assert_eq!(mine[0].name, "first cause");
        assert_eq!(
            (mine[1].kind, mine[1].value),
            (CoreDiagnosticKind::Count, 7)
        );
        assert_eq!(mine[2].kind, CoreDiagnosticKind::DurationMicros);
        assert!(
            mine.windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
        assert!(core_diagnostics(snapshot.latest_sequence).events.is_empty());

        for _ in 0..(CAPACITY + 10) {
            count("test", "flood", 1);
        }
        let flooded = core_diagnostics(0);
        assert_eq!(flooded.events.len(), CAPACITY);
        assert!(flooded.dropped >= 10);
    }

    #[test]
    fn causes_are_static_reasons_not_runtime_text() {
        let missing = std::io::Error::new(std::io::ErrorKind::NotFound, "/secret/path/name.png");
        assert_eq!(io_cause(&missing), "file not found");
        let model = vw_model::ModelError::Invalid("exhausted order interval");
        assert_eq!(model_cause(&model), "exhausted order interval");
        let wrapped = vw_store::StoreError::Operation(vw_ops::OpsError::Model(model));
        assert_eq!(store_cause(&wrapped), "exhausted order interval");
        assert_eq!(
            store_cause(&vw_store::StoreError::LowSpace),
            "low disk space"
        );
    }
}
