#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::{
    io::Cursor,
    io::Read,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

type Held = Arc<Mutex<Vec<(FakeReader, StagedInput)>>>;
struct FakeReader {
    bytes: Cursor<Vec<u8>>,
    opens: Arc<AtomicUsize>,
    closes: Arc<AtomicUsize>,
    fail_close: Arc<AtomicBool>,
    held: Held,
    cancel_after_read: Option<Arc<Cancellation>>,
    invalid_count: bool,
    fail_read: bool,
}
impl Reader for FakeReader {
    fn open(&mut self) -> Result<()> {
        self.opens.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize> {
        if self.fail_read {
            return Err(Error::Storage);
        }
        let count = self.bytes.read(bytes).map_err(io_error)?;
        if let Some(cancel) = &self.cancel_after_read {
            cancel.cancel();
        }
        Ok(if self.invalid_count { CHUNK + 1 } else { count })
    }
    fn close(&mut self) -> Result<()> {
        self.closes.fetch_add(1, Ordering::AcqRel);
        if self.fail_close.load(Ordering::Acquire) {
            Err(Error::Storage)
        } else {
            Ok(())
        }
    }
    fn quarantine(self, input: StagedInput) {
        let held = self.held.clone();
        held.lock().unwrap().push((self, input));
    }
}
struct Fixture {
    root: tempfile::TempDir,
    budget: SessionBudget,
    bytes: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        let mut bytes = Vec::new();
        {
            let mut png = png::Encoder::new(&mut bytes, 1, 1);
            png.set_color(png::ColorType::Rgb);
            png.set_depth(png::BitDepth::Eight);
            png.write_header()
                .unwrap()
                .write_image_data(&[21, 42, 84])
                .unwrap();
        }
        Self {
            root: tempfile::tempdir().unwrap(),
            budget: SessionBudget::new(256 * 1024 * 1024).unwrap(),
            bytes,
        }
    }
    fn limits(&self) -> GiantInputLimits {
        GiantInputLimits {
            max_original_bytes: 1024 * 1024,
            max_work_disk_bytes: 8 * 1024 * 1024,
            assume_untagged_srgb: true,
        }
    }
    fn input(&self) -> StagedInput {
        StagedInput::create(
            self.root.path(),
            &self.limits(),
            self.budget.clone(),
            &Cancellation::new(),
        )
        .unwrap()
    }
    fn reader(&self) -> FakeReader {
        FakeReader {
            bytes: Cursor::new(self.bytes.clone()),
            opens: Arc::new(AtomicUsize::new(0)),
            closes: Arc::new(AtomicUsize::new(0)),
            fail_close: Arc::new(AtomicBool::new(false)),
            held: Arc::new(Mutex::new(Vec::new())),
            cancel_after_read: None,
            invalid_count: false,
            fail_read: false,
        }
    }
    fn empty(&self) {
        assert_eq!(self.budget.used_bytes(), 0);
        assert_eq!(std::fs::read_dir(self.root.path()).unwrap().count(), 0);
    }
}

#[test]
fn uri_admission_precedes_filesystem_and_reader_open() {
    let f = Fixture::new();
    let first = f.input();
    let second = f.input();
    assert_eq!(f.budget.used_bytes(), 8 * 1024 * 1024);
    let absent = f.root.path().join("never-created");
    assert!(matches!(
        StagedInput::create(&absent, &f.limits(), f.budget.clone(), &Cancellation::new()),
        Err(Error::Backpressure)
    ));
    assert!(!absent.exists());
    first.release().unwrap();
    second.release().unwrap();
    f.empty();
}

#[test]
fn cancellation_before_open_closes_holder_without_opening_source() {
    let f = Fixture::new();
    let input = f.input();
    let reader = f.reader();
    let opens = reader.opens.clone();
    let closes = reader.closes.clone();
    let cancel = Cancellation::new();
    cancel.cancel();
    assert!(matches!(
        copy_reader(input, reader, f.limits(), 1000, &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(opens.load(Ordering::Acquire), 0);
    assert_eq!(closes.load(Ordering::Acquire), 1);
    f.empty();
}

#[test]
fn cancellation_after_actual_read_joins_close_before_releasing_admission() {
    let f = Fixture::new();
    let input = f.input();
    let mut reader = f.reader();
    let cancel = Arc::new(Cancellation::new());
    reader.cancel_after_read = Some(cancel.clone());
    let closes = reader.closes.clone();
    assert!(matches!(
        copy_reader(input, reader, f.limits(), 1000, &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(closes.load(Ordering::Acquire), 1);
    f.empty();
}

#[test]
fn failed_reader_close_retains_exact_reader_staging_and_original_quota() {
    let f = Fixture::new();
    let input = f.input();
    let reader = f.reader();
    let held = reader.held.clone();
    let fail = reader.fail_close.clone();
    fail.store(true, Ordering::Release);
    assert!(matches!(
        copy_reader(input, reader, f.limits(), 1000, &Cancellation::new()),
        Err(Error::Storage)
    ));
    assert_eq!(f.budget.used_bytes(), 4 * 1024 * 1024);
    assert_eq!(held.lock().unwrap().len(), 1);
    let second = f.input();
    assert!(matches!(
        StagedInput::create(
            f.root.path(),
            &f.limits(),
            f.budget.clone(),
            &Cancellation::new()
        ),
        Err(Error::Backpressure)
    ));
    second.release().unwrap();
    // Actual retry, never a caller-asserted retirement flag.
    fail.store(false, Ordering::Release);
    let (mut reader, input) = held.lock().unwrap().pop().unwrap();
    reader.close().unwrap();
    drop(reader);
    input.release().unwrap();
    f.empty();
}

#[test]
fn byte_disk_read_and_foreign_count_failures_close_and_remove_partial_input() {
    for mode in 0..4 {
        let f = Fixture::new();
        let input = f.input();
        let mut reader = f.reader();
        let mut limits = f.limits();
        // Real append limits belong to the admitted staged owner, not a later
        // caller-supplied replacement. Set those private test fixtures directly.
        let mut input = input;
        match mode {
            0 => {
                input.original_limit = 33;
                limits.max_original_bytes = 33;
            }
            1 => {
                input.disk_limit = 64;
                limits.max_work_disk_bytes = 64;
            }
            2 => reader.fail_read = true,
            _ => reader.invalid_count = true,
        }
        let closes = reader.closes.clone();
        assert!(copy_reader(input, reader, limits, 1000, &Cancellation::new()).is_err());
        assert_eq!(closes.load(Ordering::Acquire), 1);
        f.empty();
    }
}

#[test]
fn copied_bytes_transfer_the_same_admission_to_the_existing_picked_owner() {
    let f = Fixture::new();
    let input = f.input();
    let reader = f.reader();
    let closes = reader.closes.clone();
    let picked = copy_reader(input, reader, f.limits(), 1000, &Cancellation::new()).unwrap();
    assert_eq!(closes.load(Ordering::Acquire), 1);
    assert_eq!(f.budget.used_bytes(), 4 * 1024 * 1024);
    // The actual already-admitted file is byte-identical, before preparation.
    let root = std::fs::read_dir(f.root.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(std::fs::read(root.join("picked.uri")).unwrap(), f.bytes);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(picked.close_worker()).unwrap();
    f.empty();
}
