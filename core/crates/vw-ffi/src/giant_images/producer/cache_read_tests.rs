#![allow(clippy::unwrap_used)]
use super::*;
use std::io::Cursor;
#[test]
fn post_metadata_growth_reads_only_one_sentinel_and_returns_the_exact_admission() {
    let budget = SessionBudget::new(4).unwrap();
    let mut reader = Cursor::new(b"abc-more".as_slice());
    assert!(matches!(
        read_fixed(&mut reader, 3, &budget, &Cancellation::new()),
        Err(Error::Invalid)
    ));
    assert_eq!(reader.position(), 4);
    assert_eq!(budget.used_bytes(), 0);
}
#[test]
fn fixed_payload_refuses_truncation_and_preserves_success_capacity_and_lifetime() {
    let budget = SessionBudget::new(4).unwrap();
    assert!(matches!(
        read_fixed(Cursor::new(b"ab"), 3, &budget, &Cancellation::new()),
        Err(Error::Invalid)
    ));
    assert_eq!(budget.used_bytes(), 0);
    let owner = read_fixed(Cursor::new(b"abc"), 3, &budget, &Cancellation::new()).unwrap();
    assert_eq!(owner.bytes(), b"abc");
    assert_eq!(owner.bytes.capacity(), 4);
    assert_eq!(budget.used_bytes(), 4);
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}
#[test]
fn fixed_payload_admission_and_cancel_happen_before_read() {
    struct NoRead;
    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("must not read"))
        }
    }
    let budget = SessionBudget::new(3).unwrap();
    assert!(matches!(
        read_fixed(NoRead, 3, &budget, &Cancellation::new()),
        Err(Error::Memory { .. })
    ));
    let cancel = Cancellation::new();
    cancel.cancel();
    assert!(matches!(
        read_fixed(NoRead, 1, &budget, &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(budget.used_bytes(), 0);
}
