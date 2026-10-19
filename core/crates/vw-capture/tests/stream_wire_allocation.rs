//! Heap-allocation regression for the private stream pipe's admitted JSON DTO.
//! A separate test binary keeps the metering allocator out of product code.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    io::Cursor,
};
use vw_capture::stream_wire::{self, Header};
use vw_stream::{Cancellation, SessionBudget};

struct Meter;
#[global_allocator]
static ALLOCATOR: Meter = Meter;
thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}
fn record(bytes: usize) {
    let _ = TRACK.try_with(|active| {
        if active.get() {
            let _ = ALLOCATED.try_with(|sum| sum.set(sum.get().saturating_add(bytes)));
        }
    });
}
// SAFETY: every operation delegates the same pointer/layout contract to the
// system allocator. The thread-local counter observes successful allocations
// without changing their ownership, alignment or deallocation behavior.
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies GlobalAlloc's valid layout contract.
        let value = unsafe { System.alloc(layout) };
        if !value.is_null() {
            record(layout.size());
        }
        value
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegation preserves the caller's layout and zeroing request.
        let value = unsafe { System.alloc_zeroed(layout) };
        if !value.is_null() {
            record(layout.size());
        }
        value
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: pointer and layout are forwarded unchanged to their allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: pointer, original layout and new size are forwarded unchanged.
        let value = unsafe { System.realloc(ptr, layout, new_size) };
        if !value.is_null() {
            record(new_size);
        }
        value
    }
}
fn measured<T>(work: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATED.with(|sum| sum.set(0));
    TRACK.with(|active| active.set(true));
    let value = work();
    TRACK.with(|active| active.set(false));
    let bytes = ALLOCATED.with(Cell::get);
    (value, bytes)
}
fn wire(json: &[u8]) -> Vec<u8> {
    let mut result = (json.len() as u32).to_le_bytes().to_vec();
    result.extend_from_slice(json);
    result
}

#[test]
fn accepted_escape_and_unicode_headers_allocate_within_the_held_charge() {
    let strings = [
        "a".repeat(1_000),
        "\\\"".repeat(240),
        "\n\t\r".repeat(160),
        "🙂".repeat(240),
        "\\u007f".repeat(120),
        "\\".repeat(500),
        "\"".repeat(500),
        "\u{0008}".repeat(400),
    ];
    for value in strings {
        let header = Header::Ready {
            request_hash: value,
            connection_epoch: 1,
            source_generation: 1,
            ring_frames: 8,
            ring_bytes: 8 * 1024 * 1024,
            endpoint_bytes: 128 * 1024 * 1024,
            border_visible: true,
        };
        let json = serde_json::to_vec(&header).expect("serialize fixture");
        let input = wire(&json);
        let budget = SessionBudget::new(128 * 1024 * 1024).expect("budget");
        let (result, allocated) = measured(|| {
            let packet = stream_wire::read_packet(
                &mut Cursor::new(input),
                &budget,
                &Cancellation::default(),
            )
            .expect("admitted header");
            let held = budget.used_bytes();
            drop(packet);
            held
        });
        // Cumulative successful Rust heap requests exceed or equal the peak.
        // The header byte allocation is separately charged while parsing, then
        // dropped; the DTO workspace remains held in the returned packet.
        assert!(
            allocated as u64 <= result + json.len() as u64,
            "allocated {allocated}, admitted {}",
            result + json.len() as u64
        );
        assert_eq!(budget.used_bytes(), 0);
    }
}

#[test]
fn malformed_error_path_and_oversized_string_stay_under_global_ceiling() {
    let malformed = [
        format!(
            "{{\"reply\":\"ready\",\"request_hash\":\"{}\"}}",
            "\\u007f".repeat(120)
        ),
        format!(
            "{{\"reply\":\"ready\",\"request_hash\":\"{}\"}}",
            "\\\"".repeat(240)
        ),
        format!(
            "{{\"reply\":\"ready\",\"request_hash\":\"{}\"}}",
            "x".repeat(8_533)
        ),
    ];
    for json in malformed {
        let bytes = json.as_bytes();
        let budget = SessionBudget::new(128 * 1024 * 1024).expect("budget");
        let (result, allocated) = measured(|| {
            stream_wire::read_packet(
                &mut Cursor::new(wire(bytes)),
                &budget,
                &Cancellation::default(),
            )
        });
        assert!(result.is_err());
        assert!(
            allocated as u64 <= 64 * 1024 + 512 * 4096 + 8 * 16 * 1024 + bytes.len() as u64,
            "malformed header allocated {allocated}"
        );
        assert_eq!(budget.used_bytes(), 0);
    }
}
