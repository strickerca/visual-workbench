#![allow(clippy::expect_used)]
use super::super::receiver_tests::WORKERS;
use super::*;
use crate::{GiantImageError, session_resources};
use vw_stream::{BudgetedBytes, Cancellation};

#[tokio::test]
async fn foreign_charge_and_retained_native_bytes_share_admission_across_new_handles() {
    let _native = WORKERS.lock().await;
    let process = session_resources::shared_budget().expect("common process budget");
    let foreign = session_resources::shared_session_resources().expect("foreign pool handle");
    let cap = process.limit_bytes();
    assert_eq!(foreign.limit_bytes(), cap);
    let foreign_charge = foreign
        .reserve_foreign(8192)
        .expect("foreign rendering admission");

    let native = ByteBudget::new(cap).expect("native endpoint");
    assert_eq!(native.pool().limit_bytes(), cap);
    assert!(!native.pool().same_pool(&process));
    // Our small foreign charge makes this request fail at the shared parent.
    // No whole-process reservation succeeds or starves unrelated test owners.
    assert!(matches!(
        native.reserve(cap),
        Err(SessionError::Backpressure)
    ));
    assert_eq!(
        native.used(),
        0,
        "failed parent admission rolls back its child"
    );

    let mut pixels = BudgetedBytes::allocate(
        native.pool(),
        MemoryKind::DisplayFrame,
        4096,
        &Cancellation::default(),
    )
    .expect("actual native pixel allocation");
    pixels.bytes_mut().fill(0x6b);
    let pixels = Arc::new(pixels);
    let late_reader = pixels.clone();
    let endpoint = native.pool().clone();
    assert!(pixels.charge().belongs_to(&endpoint));
    assert_eq!(native.used(), 4096);
    assert!(foreign.used_bytes() >= 8192 + 4096);

    // Retire the foreign rendering reservation and its API handle. The native
    // allocation still owns the original endpoint and the actual shared parent.
    foreign_charge.close().expect("foreign retirement");
    foreign_charge
        .close()
        .expect("idempotent foreign retirement");
    drop(foreign_charge);
    drop(foreign);
    drop(native);
    drop(pixels);
    assert_eq!(endpoint.used_bytes(), 4096);
    assert_eq!(late_reader.bytes(), &[0x6b; 4096]);

    let reopened = session_resources::shared_session_resources().expect("fresh foreign handle");
    assert!(reopened.used_bytes() >= 4096);
    assert!(matches!(reopened.reserve_foreign(cap),
        Err(GiantImageError::Memory { estimated, budget }) if budget == cap && estimated > cap));
    let successor = ByteBudget::new(cap).expect("fresh native endpoint");
    assert!(!successor.pool().same_pool(&endpoint));
    assert_eq!(successor.used(), 0);
    assert!(matches!(
        successor.reserve(cap),
        Err(SessionError::Backpressure)
    ));
    assert_eq!(successor.used(), 0);

    drop(late_reader);
    assert_eq!(
        endpoint.used_bytes(),
        0,
        "only the actual last reader returns native credit"
    );
}

#[tokio::test]
async fn endpoint_children_keep_exact_limits_and_distinct_identity_under_platform_parent() {
    let _native = WORKERS.lock().await;
    let process = session_resources::shared_budget().expect("common process budget");
    #[cfg(target_os = "android")]
    assert_eq!(process.limit_bytes(), vw_stream::PHONE_SESSION_BYTES);
    #[cfg(not(target_os = "android"))]
    assert_eq!(process.limit_bytes(), vw_stream::DESKTOP_SESSION_BYTES);

    assert!(matches!(ByteBudget::new(0), Err(SessionError::Invalid)));
    assert!(matches!(
        ByteBudget::new(MAX_RECEIVER_BYTES + 1),
        Err(SessionError::Invalid)
    ));
    let first = ByteBudget::new(8192).expect("first endpoint");
    let cloned = first.clone();
    let second = ByteBudget::new(8192).expect("second endpoint");
    assert!(cloned.pool().same_pool(first.pool()));
    assert!(!second.pool().same_pool(first.pool()));
    assert_eq!(first.pool().limit_bytes(), 8192);
    assert_eq!(second.pool().limit_bytes(), 8192);
    let pixels = BudgetedBytes::allocate(
        first.pool(),
        MemoryKind::DisplayFrame,
        8192,
        &Cancellation::default(),
    )
    .expect("exact endpoint capacity");
    assert!(matches!(cloned.reserve(1), Err(SessionError::Backpressure)));
    assert!(matches!(
        second.reserve(8193),
        Err(SessionError::Backpressure)
    ));
    assert_eq!(first.used(), 8192);
    assert_eq!(second.used(), 0);
    drop(first);
    drop(pixels);
    assert_eq!(cloned.used(), 0);

    // Android's actual parent cap is smaller than the public maximum. Refuse
    // an oversized requested child rather than silently replacing its limit.
    if process.limit_bytes() < MAX_RECEIVER_BYTES {
        assert!(matches!(
            ByteBudget::new(process.limit_bytes() + 1),
            Err(SessionError::Backpressure)
        ));
    }
}
