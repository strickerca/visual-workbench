//! Bounded numeric diagnostics; no target, device, ticket, or network identifiers.
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};
const NAMES: [&str; 11] = [
    "capture_polls",
    "capture_idle",
    "encoded_frames",
    "config_enqueued",
    "media_enqueued",
    "config_received",
    "media_received",
    "media_no_config",
    "media_retired",
    "media_admitted",
    "frame_taken",
];
#[derive(Default)]
pub(super) struct Diagnostics([AtomicU64; 11]);
impl Diagnostics {
    pub(super) fn increment(&self, index: usize) {
        let _ = self.0[index].try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
            Some(v.saturating_add(1))
        });
    }
    pub(super) fn snapshot(&self) -> BTreeMap<&'static str, u64> {
        NAMES
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, self.0[i].load(Ordering::Relaxed)))
            .collect()
    }
}
/// A completed IDR acknowledges only the generation sampled before its poll.
/// Idle and backpressure never complete a generation; concurrent requests survive.
#[cfg(any(windows, test))]
#[derive(Default)]
pub(super) struct Keyframes(AtomicU64);
#[cfg(any(windows, test))]
impl Keyframes {
    pub(super) fn request(&self) {
        let _ = self.0.try_update(Ordering::AcqRel, Ordering::Acquire, |v| {
            Some(v.saturating_add(1))
        });
    }
    pub(super) fn generation(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
    pub(super) fn pending(&self, completed: u64) -> bool {
        let requested = self.generation();
        requested == u64::MAX || requested != completed
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_does_not_consume_request() {
        let requests = Keyframes::default();
        requests.request();
        for _ in 0..5 {
            assert!(requests.pending(0));
        }
        let emitted = requests.generation();
        assert!(!requests.pending(emitted));
    }
    #[test]
    fn concurrent_request_survives_older_idr() {
        let requests = Keyframes::default();
        requests.request();
        let sampled = requests.generation();
        requests.request();
        assert!(requests.pending(sampled));
        assert!(!requests.pending(requests.generation()));
    }
    #[test]
    fn backpressure_retains_generation() {
        let requests = Keyframes::default();
        requests.request();
        let sampled = requests.generation();
        requests.request();
        assert!(requests.pending(0));
        assert!(requests.pending(sampled));
    }
    #[test]
    fn saturated_generation_stays_pending() {
        let requests = Keyframes(AtomicU64::new(u64::MAX));
        requests.request();
        assert!(requests.pending(u64::MAX));
    }
    #[test]
    fn diagnostics_are_fixed_bounded_numeric_and_saturate() {
        let counts = Diagnostics::default();
        counts.0[0].store(u64::MAX, Ordering::Relaxed);
        counts.increment(0);
        counts.increment(10);
        let snapshot = counts.snapshot();
        assert_eq!(snapshot.len(), 11);
        assert_eq!(snapshot["capture_polls"], u64::MAX);
        assert_eq!(snapshot["frame_taken"], 1);
    }
}
