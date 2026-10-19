#![allow(clippy::unwrap_used)]
use std::sync::Arc;
use vw_stream::{BudgetedBytes, Cancellation, MemoryKind, SessionBudget, StreamError};

#[test]
fn sibling_endpoints_share_the_actual_parent_limit_and_rollback_failure() {
    let parent = SessionBudget::new(100).unwrap();
    let left = parent.child(100).unwrap();
    let right = parent.child(100).unwrap();
    let retained = left.reserve(MemoryKind::DisplayFrame, 80).unwrap();
    assert!(matches!(
        right.reserve(MemoryKind::Transfer, 21),
        Err(StreamError::Limit)
    ));
    assert_eq!(right.used_bytes(), 0);
    assert_eq!(parent.used_bytes(), 80);
    let second = right.reserve(MemoryKind::Transfer, 20).unwrap();
    assert_eq!(parent.used_bytes(), 100);
    drop(retained);
    drop(second);
    assert_eq!(parent.used_bytes(), 0);
}

#[test]
fn escaped_payload_retains_both_endpoint_and_parent_after_endpoint_drop() {
    let parent = SessionBudget::new(64).unwrap();
    let endpoint = parent.child(32).unwrap();
    let payload = Arc::new(
        BudgetedBytes::allocate(
            &endpoint,
            MemoryKind::Transfer,
            32,
            &Cancellation::default(),
        )
        .unwrap(),
    );
    let escaped = payload.clone();
    drop(endpoint);
    drop(payload);
    assert_eq!(parent.used_bytes(), 32);
    drop(escaped);
    assert_eq!(parent.used_bytes(), 0);
}

#[test]
fn adopted_allocation_cannot_hide_spare_capacity_or_lose_parent_credit() {
    let parent = SessionBudget::new(64).unwrap();
    let endpoint = parent.child(64).unwrap();
    let reservation = endpoint.reserve(MemoryKind::Transfer, 4).unwrap();
    let mut pixels = Vec::with_capacity(32);
    pixels.extend_from_slice(&[1, 2, 3, 4]);
    assert!(matches!(
        BudgetedBytes::from_parts(pixels, reservation),
        Err(StreamError::Limit)
    ));
    assert_eq!(parent.used_bytes(), 0);
    assert_eq!(endpoint.used_bytes(), 0);
}
