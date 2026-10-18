#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;

fn id(n: u8) -> String {
    vw_model::Id::from_parts(1, [n; 10]).unwrap().to_string()
}
fn key(n: u8) -> VerificationPackageKey {
    VerificationPackageKey {
        package_id: id(n),
        target: "generic".into(),
        manifest_sha256: format!("{n:02x}").repeat(32),
    }
}
fn pair() -> VerificationPair {
    VerificationPair {
        original: key(1),
        after: key(2),
        receipt_id: id(3),
        receipt_blake3: "ab".repeat(32),
    }
}
fn marker() -> vw_package::PackageMarker {
    vw_package::PackageMarker {
        number: 1,
        kind: "box".into(),
        role: "change".into(),
        instruction: "literal original".into(),
        bbox_compiled: [1.0, 2.0, 3.0, 4.0],
        bbox_document: [1.0, 2.0, 3.0, 4.0],
        point_compiled: [2.5, 3.5],
        point_document: [2.5, 3.5],
        element_refs: Vec::new(),
        crop_image: "marker_1".into(),
    }
}
#[test]
fn translation_requires_the_exact_failed_set_and_refuses_duplicate_unknown_or_nonfinite() {
    let translation = |n| VerificationMarkerTranslation {
        original_number: n,
        dx: 1.0,
        dy: -1.0,
    };
    assert!(prepare::translations(&[1, 3], vec![translation(3), translation(1)]).is_ok());
    for invalid in [
        vec![],
        vec![translation(1)],
        vec![translation(1), translation(1)],
        vec![translation(1), translation(2)],
    ] {
        assert!(prepare::translations(&[1, 3], invalid).is_err());
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut invalid = translation(1);
        invalid.dx = value;
        assert!(prepare::translations(&[1], vec![invalid]).is_err());
    }
}
#[test]
fn canonical_translation_keeps_extent_and_offset_and_refuses_full_source_overflow() {
    let original = marker();
    let geometry = prepare::translated(
        &original,
        &VerificationMarkerTranslation {
            original_number: 1,
            dx: 2.6,
            dy: 1.2,
        },
        20,
        20,
    )
    .unwrap();
    assert_eq!(geometry.point, [5.5, 4.5]);
    assert_eq!(geometry.bounds, [4.0, 3.0, 3.0, 4.0]);
    for (number, dx, dy) in [
        (0, 0.0, 0.0),
        (1, -2.0, 0.0),
        (1, 17.0, 0.0),
        (1, 0.0, 15.0),
        (1, f64::NAN, 0.0),
    ] {
        assert!(
            prepare::translated(
                &original,
                &VerificationMarkerTranslation {
                    original_number: number,
                    dx,
                    dy
                },
                20,
                20
            )
            .is_err()
        );
    }
    let mut edge = marker();
    edge.bbox_document = [17.0, 2.0, 3.0, 4.0];
    edge.point_document = [18.5, 3.5];
    assert!(
        prepare::translated(
            &edge,
            &VerificationMarkerTranslation {
                original_number: 1,
                dx: 0.1,
                dy: 0.0
            },
            20,
            20
        )
        .is_err()
    );
}
#[test]
fn private_snapshot_conversion_checks_pair_before_cloning_notes_and_omits_notes_from_failure_review()
 {
    let pair = pair();
    let native_pair = vw_store::VerificationReviewPair {
        project_id: vw_model::Id::try_from(id(4)).unwrap(),
        original: pair.original.native().unwrap(),
        after: pair.after.native().unwrap(),
        comparison_receipt_id: vw_model::Id::try_from(pair.receipt_id.clone()).unwrap(),
        comparison_receipt_blake3: pair.receipt_blake3.clone(),
        marker_count: 1,
    };
    let mut snapshot = VerificationReviewSnapshot {
        pair: pair.clone(),
        project_id: id(4),
        marker_count: 1,
        version: 1,
        markers: vec![VerificationMarkerReview {
            original_number: 1,
            verdict: VerificationVerdict::Fail,
            note: "private manual note".into(),
        }],
    };
    let native = snapshot.native(&pair, &native_pair).unwrap();
    let public = dto::failure_review(&native);
    assert!(
        public.fence.version == 1
            && public.markers.len() == 1
            && public.markers[0].original_number == 1
    );
    assert!(dto::snapshot(&pair, native) == snapshot);
    snapshot.markers[0].note = "x".repeat(32 * 1024 + 1);
    assert!(snapshot.native(&pair, &native_pair).is_err());
    snapshot.markers[0].note = "nul\0private".into();
    assert!(snapshot.native(&pair, &native_pair).is_err());
    snapshot.markers[0].note.clear();
    snapshot.pair.receipt_blake3 = "cd".repeat(32);
    assert!(matches!(
        snapshot.native(&pair, &native_pair),
        Err(PackageError::Stale)
    ));
}
#[test]
fn exact_uuidv7_pair_and_error_boundary_preserve_stale_cancel_and_private_storage_failures() {
    assert!(pair().validate().is_ok());
    let mut invalid = pair();
    invalid.receipt_id = "00000000-0000-4000-8000-000000000000".into();
    assert!(invalid.validate().is_err());
    invalid = pair();
    invalid.after.package_id = invalid.original.package_id.clone();
    assert!(invalid.validate().is_err());
    assert!(matches!(
        store_error(vw_store::VerificationReviewError::Conflict),
        PackageError::Stale
    ));
    assert!(matches!(
        store_error(vw_store::VerificationReviewError::Cancelled),
        PackageError::Cancelled
    ));
    assert!(matches!(
        store_error(vw_store::VerificationReviewError::Corrupt),
        PackageError::Integrity
    ));
    assert!(matches!(
        store_error(vw_store::VerificationReviewError::Pending),
        PackageError::Invalid
    ));
}
