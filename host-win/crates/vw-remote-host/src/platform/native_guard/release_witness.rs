//! Preallocated, fixed-size diagnostic. No allocation, lock or IO on release.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use vw_remote::{Binding, wire::PenReleaseWitness};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

pub struct Record {
    frequency: u64,
    started: AtomicU64,
    completed: AtomicU64,
    held: AtomicBool,
    published: AtomicBool,
}
impl Default for Record {
    fn default() -> Self {
        Self::new()
    }
}
fn counter() -> u64 {
    let mut value = 0i64;
    // SAFETY: The API writes one initialized local integer.
    if unsafe { QueryPerformanceCounter(&mut value) }.is_ok() && value > 0 {
        value as u64
    } else {
        0
    }
}
impl Record {
    pub fn new() -> Self {
        let mut frequency = 0i64;
        // SAFETY: Read-only frequency query before input, never in release.
        let valid = unsafe { QueryPerformanceFrequency(&mut frequency) }.is_ok() && frequency > 0;
        Self {
            frequency: if valid { frequency as u64 } else { 0 },
            started: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            held: AtomicBool::new(false),
            published: AtomicBool::new(false),
        }
    }
    pub fn release(&self, held: bool, destroy: impl FnOnce()) {
        let started = counter();
        destroy();
        let completed = counter();
        self.publish(held, started, completed);
    }
    fn publish(&self, held: bool, started: u64, completed: u64) {
        // Exactly one Injector owns this record. Store only that actual first
        // destructor; a later getter cannot restart or synthesize its timing.
        if self.published.load(Ordering::Relaxed) {
            return;
        }
        self.held.store(held, Ordering::Relaxed);
        self.started.store(started, Ordering::Relaxed);
        self.completed.store(completed, Ordering::Relaxed);
        self.published.store(true, Ordering::Release);
    }
    pub fn witness(&self, binding: Binding) -> Option<PenReleaseWitness> {
        if !self.published.load(Ordering::Acquire) {
            return None;
        }
        let completed = self.completed.load(Ordering::Relaxed);
        let started = self.started.load(Ordering::Relaxed);
        if started == 0 || completed < started || self.frequency == 0 {
            return None;
        }
        Some(PenReleaseWitness {
            binding,
            held_before_release: self.held.load(Ordering::Relaxed),
            release_started_qpc: started,
            release_completed_qpc: completed,
            qpc_frequency: self.frequency,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_runs_even_when_diagnostic_clock_is_unavailable() {
        let record = Record {
            frequency: 0,
            started: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            held: AtomicBool::new(false),
            published: AtomicBool::new(false),
        };
        let mut destroyed = false;
        record.release(true, || destroyed = true);
        assert!(destroyed);
        assert_eq!(record.frequency, 0);
    }
    #[test]
    fn first_real_release_fields_cannot_be_replaced() {
        let record = Record::new();
        record.publish(true, 100, 101);
        record.publish(false, 200, 201);
        assert_eq!(record.started.load(Ordering::Acquire), 100);
        assert_eq!(record.completed.load(Ordering::Acquire), 101);
        assert!(record.held.load(Ordering::Acquire));
    }
    #[test]
    fn invalid_first_clock_cannot_be_replaced_by_later_valid_clock() {
        let record = Record::new();
        record.publish(true, 0, 0);
        record.publish(false, 200, 201);
        assert_eq!(record.started.load(Ordering::Acquire), 0);
        assert_eq!(record.completed.load(Ordering::Acquire), 0);
        assert!(record.held.load(Ordering::Acquire));
    }
}
