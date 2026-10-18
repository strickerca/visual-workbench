#![allow(clippy::unwrap_used)]
use super::*;

#[test]
fn final_hash_includes_header_rewrites_and_ignores_the_write_order() {
    let mut file = tempfile::tempfile().unwrap(); file.write_all(b"placeholder-body").unwrap();
    file.seek(SeekFrom::Start(0)).unwrap(); file.write_all(b"finalheader").unwrap();
    let mut hash = blake3::Hasher::new(); let length = file.metadata().unwrap().len();
    hash_final_file(&mut file, length, &mut hash, &Cancellation::new()).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap(); let mut bytes = Vec::new(); file.read_to_end(&mut bytes).unwrap();
    assert_eq!(hash.finalize(), blake3::hash(&bytes));
    assert_ne!(hash.finalize(), blake3::hash(b"placeholder-bodyfinalheader"));
}

#[test]
fn final_hash_refuses_cancelled_and_wrong_length_receipts() {
    let mut file = tempfile::tempfile().unwrap(); file.write_all(b"complete output").unwrap();
    let mut hash = blake3::Hasher::new(); let length = file.metadata().unwrap().len();
    assert!(matches!(hash_final_file(&mut file, length - 1, &mut hash, &Cancellation::new()), Err(TransferError::Storage)));
    let cancel = Cancellation::new(); cancel.cancel();
    assert!(matches!(hash_final_file(&mut file, length, &mut hash, &cancel), Err(TransferError::Cancelled)));
}
