#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::cell::Cell;
use vw_model::{DeviceId, Id, Project};
use vw_store::*;

fn id(n: u64) -> Id {
    Id::from_parts(1_700_000_000_000 + n, [9; 10]).unwrap()
}
fn pair() -> VerificationReviewPair {
    VerificationReviewPair {
        project_id: id(1),
        original: VerificationPackageKey {
            package_id: id(2),
            target: "generic".into(),
            manifest_sha256: "a".repeat(64),
        },
        after: VerificationPackageKey {
            package_id: id(3),
            target: "generic".into(),
            manifest_sha256: "b".repeat(64),
        },
        comparison_receipt_id: id(4),
        comparison_receipt_blake3: "c".repeat(64),
        marker_count: 3,
    }
}
fn fixture(path: &std::path::Path) -> ProjectStore {
    let device = DeviceId::from_bytes([7; 16]);
    ProjectStore::create(
        path,
        Project::new(id(1), "Synthetic review".into(), device.clone()),
        device,
        0,
    )
    .unwrap()
}
fn change(number: u32, verdict: VerificationVerdict, note: &str) -> VerificationReviewChange {
    VerificationReviewChange {
        original_number: number,
        verdict,
        note: note.to_owned(),
    }
}
fn raw(path: &std::path::Path) -> String {
    rusqlite::Connection::open(path.join("project.sqlite"))
        .unwrap()
        .query_row(
            "SELECT value FROM settings WHERE key GLOB 'vw.verification-review.v1.*'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn reopen_preserves_exact_pair_notes_and_versions_without_canonical_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("p");
    let mut store = fixture(&path);
    let before = store.checkpoint_bytes().unwrap();
    let p = pair();
    let pending = store.verification_review(&p).unwrap();
    assert_eq!(pending.version, 0);
    assert!(matches!(
        pending.failed_markers(),
        Err(VerificationReviewError::Pending)
    ));
    let first = store
        .change_verification_review(
            &p,
            0,
            &id(10),
            change(1, VerificationVerdict::Fail, "private draft é"),
            || false,
        )
        .unwrap();
    assert_eq!(first.version, 1);
    assert!(store.checkpoint_bytes().unwrap() == before);
    drop(store);
    let mut store = ProjectStore::open(&path).unwrap();
    assert!(store.verification_review(&p).unwrap() == first);
    assert!(store.checkpoint_bytes().unwrap() == before);
}
#[test]
fn exact_lost_reply_retry_recovers_and_changed_cas_or_content_refuses() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("p");
    let mut store = fixture(&path);
    let p = pair();
    let first = store
        .change_verification_review(
            &p,
            0,
            &id(10),
            change(1, VerificationVerdict::Fail, "draft"),
            || false,
        )
        .unwrap();
    let saved = raw(&path);
    assert!(
        store
            .change_verification_review(
                &p,
                0,
                &id(10),
                change(1, VerificationVerdict::Fail, "draft"),
                || false
            )
            .unwrap()
            == first
    );
    for (expected, note) in [(0, "changed"), (1, "draft")] {
        assert!(matches!(
            store.change_verification_review(
                &p,
                expected,
                &id(10),
                change(1, VerificationVerdict::Fail, note),
                || false
            ),
            Err(VerificationReviewError::Conflict)
        ));
    }
    assert!(matches!(
        store.change_verification_review(
            &p,
            0,
            &id(11),
            change(2, VerificationVerdict::Pass, ""),
            || false
        ),
        Err(VerificationReviewError::Conflict)
    ));
    assert_eq!(raw(&path), saved);
}
#[test]
fn every_cancel_checkpoint_rolls_back_and_keeps_prior_record_byte_exact() {
    for stop in 1..=3 {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("p");
        let mut store = fixture(&path);
        let p = pair();
        let first = store
            .change_verification_review(
                &p,
                0,
                &id(10),
                change(1, VerificationVerdict::Fail, "keep"),
                || false,
            )
            .unwrap();
        let saved = raw(&path);
        let count = Cell::new(0);
        assert!(matches!(
            store.change_verification_review(
                &p,
                1,
                &id(11),
                change(2, VerificationVerdict::Pass, "new"),
                || {
                    count.set(count.get() + 1);
                    count.get() == stop
                }
            ),
            Err(VerificationReviewError::Cancelled)
        ));
        assert_eq!(raw(&path), saved);
        assert!(store.verification_review(&p).unwrap() == first);
    }
}
#[test]
fn pending_unknown_and_replaced_pair_refuse_while_all_pass_has_no_follow_up() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("p");
    let mut store = fixture(&path);
    let p = pair();
    for number in [0, 4] {
        assert!(matches!(
            store.change_verification_review(
                &p,
                0,
                &id(10),
                change(number, VerificationVerdict::Fail, ""),
                || false
            ),
            Err(VerificationReviewError::Invalid)
        ));
    }
    let mut snapshot = store.verification_review(&p).unwrap();
    for number in 1..=3 {
        snapshot = store
            .change_verification_review(
                &p,
                snapshot.version,
                &id(10 + u64::from(number)),
                change(number, VerificationVerdict::Pass, ""),
                || false,
            )
            .unwrap();
    }
    assert!(snapshot.failed_markers().unwrap().is_none());
    let mut changed = p.clone();
    changed.after.manifest_sha256 = "d".repeat(64);
    assert!(matches!(
        store.change_verification_review(
            &changed,
            snapshot.version,
            &id(20),
            change(1, VerificationVerdict::Fail, ""),
            || false
        ),
        Err(VerificationReviewError::Conflict)
    ));
    let stale = snapshot.clone();
    snapshot = store
        .change_verification_review(
            &p,
            snapshot.version,
            &id(21),
            change(2, VerificationVerdict::Fail, "do not export"),
            || false,
        )
        .unwrap();
    assert_eq!(snapshot.failed_markers().unwrap().unwrap(), vec![2]);
    assert!(matches!(
        store.check_verification_review(&stale),
        Err(VerificationReviewError::Conflict)
    ));
    store.check_verification_review(&snapshot).unwrap();
}
#[test]
fn row_and_utf8_note_budgets_preserve_existing_records() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("p");
    let mut store = fixture(&path);
    let p = pair();
    let first = store
        .change_verification_review(
            &p,
            0,
            &id(10),
            change(1, VerificationVerdict::Fail, "keep"),
            || false,
        )
        .unwrap();
    assert!(matches!(
        store.change_verification_review(
            &p,
            first.version,
            &id(11),
            change(1, VerificationVerdict::Fail, &"é".repeat(16385)),
            || false
        ),
        Err(VerificationReviewError::Invalid)
    ));
    for n in 1..64 {
        let mut next = p.clone();
        next.comparison_receipt_id = id(100 + n);
        store
            .change_verification_review(
                &next,
                0,
                &id(500 + n),
                change(1, VerificationVerdict::Fail, ""),
                || false,
            )
            .unwrap();
    }
    let mut last = p.clone();
    last.comparison_receipt_id = id(1000);
    assert!(matches!(
        store.change_verification_review(
            &last,
            0,
            &id(1001),
            change(1, VerificationVerdict::Fail, ""),
            || false
        ),
        Err(VerificationReviewError::Limit)
    ));
    assert!(store.verification_review(&p).unwrap() == first);
}
#[test]
fn corrupt_rows_and_failed_schema_write_do_not_replace_private_progress() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("p");
    let mut store = fixture(&path);
    let p = pair();
    store
        .change_verification_review(
            &p,
            0,
            &id(10),
            change(1, VerificationVerdict::Fail, "keep"),
            || false,
        )
        .unwrap();
    let saved = raw(&path);
    let sql = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    sql.execute_batch("CREATE TRIGGER reject_review BEFORE UPDATE ON settings BEGIN SELECT RAISE(ABORT, 'synthetic refusal'); END;").unwrap();
    assert!(
        store
            .change_verification_review(
                &p,
                1,
                &id(11),
                change(2, VerificationVerdict::Pass, ""),
                || false
            )
            .is_err()
    );
    assert_eq!(raw(&path), saved);
    sql.execute_batch("DROP TRIGGER reject_review;").unwrap();
    for invalid in [
        saved.replace("\"schema\":1", "\"schema\":2"),
        saved.replace("\"version\":1", "\"version\":1,\"version\":1"),
        saved.replace("\"original_number\":1", "\"original_number\":0"),
        "{}".to_owned(),
    ] {
        sql.execute(
            "UPDATE settings SET value=?1 WHERE key GLOB 'vw.verification-review.v1.*'",
            [&invalid],
        )
        .unwrap();
        assert!(matches!(
            store.verification_review(&p),
            Err(VerificationReviewError::Corrupt)
        ));
        assert_eq!(raw(&path), invalid);
    }
    sql.execute(
        "UPDATE settings SET value=zeroblob(?1) WHERE key GLOB 'vw.verification-review.v1.*'",
        [i64::try_from(VERIFICATION_REVIEW_MAX_BYTES + 1).unwrap()],
    )
    .unwrap();
    assert!(matches!(
        store.verification_review(&p),
        Err(VerificationReviewError::Corrupt)
    ));
}
#[test]
fn aggregate_note_failure_preserves_prior_draft_and_original_failed_numbers() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("p");
    let mut store = fixture(&path);
    let mut p = pair();
    p.marker_count = 5;
    let mut snapshot = store.verification_review(&p).unwrap();
    for number in 1..=4 {
        snapshot = store
            .change_verification_review(
                &p,
                snapshot.version,
                &id(10 + u64::from(number)),
                change(number, VerificationVerdict::Fail, &"x".repeat(32768)),
                || false,
            )
            .unwrap();
    }
    let saved = raw(&path);
    assert!(matches!(
        store.change_verification_review(
            &p,
            snapshot.version,
            &id(20),
            change(5, VerificationVerdict::Fail, "x"),
            || false
        ),
        Err(VerificationReviewError::Limit)
    ));
    assert_eq!(raw(&path), saved);
    let final_review = store
        .change_verification_review(
            &p,
            snapshot.version,
            &id(21),
            change(5, VerificationVerdict::Pass, ""),
            || false,
        )
        .unwrap();
    assert_eq!(
        final_review.failed_markers().unwrap().unwrap(),
        vec![1, 2, 3, 4]
    );
}
#[test]
fn archive_export_requires_private_prefix_removal_without_mutating_source() {
    // This intentionally gates root activation: the documented deletion MUST be
    // composed into archive.rs before the newly registered store can be shipped.
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("p");
    let mut store = fixture(&path);
    let p = pair();
    let note = "synthetic-private-owner-note-never-export";
    let snapshot = store
        .change_verification_review(
            &p,
            0,
            &id(10),
            change(1, VerificationVerdict::Fail, note),
            || false,
        )
        .unwrap();
    let sql = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    sql.execute(
        "INSERT INTO settings(key,value) VALUES ('synthetic.unrelated','preserve')",
        [],
    )
    .unwrap();
    let destination = temp.path().join("copy.vwbz");
    export_vwbz(&mut store, &destination).unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(destination).unwrap()).unwrap();
    let extracted = temp.path().join("export.sqlite");
    let mut file = std::fs::File::create(&extracted).unwrap();
    std::io::copy(&mut zip.by_name("project.sqlite").unwrap(), &mut file).unwrap();
    drop(file);
    let copy = rusqlite::Connection::open(&extracted).unwrap();
    let rows: i64 = copy
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key GLOB 'vw.verification-review.v1.*'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
    let unrelated: String = copy
        .query_row(
            "SELECT value FROM settings WHERE key='synthetic.unrelated'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unrelated, "preserve");
    drop(copy);
    assert!(
        !std::fs::read(extracted)
            .unwrap()
            .windows(note.len())
            .any(|w| w == note.as_bytes())
    );
    assert!(store.verification_review(&p).unwrap() == snapshot);
}
