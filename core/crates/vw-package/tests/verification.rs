mod common;
use common::*;
use std::cell::Cell;
use vw_instructions::{DocumentView, EditMetadata, EntryMethod, MarkerPlacement, Role};
use vw_package::*;
use vw_proto::v1;

fn marked(texts: &[&str]) -> std::result::Result<Fixture, Box<dyn std::error::Error>> {
    let mut f = fixture(64, 64, 8)?;
    for (index, text) in texts.iter().enumerate() {
        let n = index as u64;
        let view = DocumentView::new(f.host.project(), &f.host.revision()?, &f.document)?;
        let plan = view.place_marker(
            EditMetadata {
                transaction_id: id(400 + n)?,
                device: device(),
                first_lamport: 2000 + n * 100,
                created_at_ms: 10,
            },
            MarkerPlacement {
                object_id: id(200 + n)?,
                instruction_id: id(300 + n)?,
                layer_id: id(3)?,
                point: v1::PointD {
                    x: 12.0 + index as f64,
                    y: 12.0,
                },
                bounds: Some(v1::RectD {
                    x: 10.0,
                    y: 10.0,
                    w: 20.0,
                    h: 20.0,
                }),
                element_eids: vec![],
                style: v1::Style {
                    stroke: Some(v1::Color { rgba: 0xf03c32ff }),
                    width: 2.0,
                    ..Default::default()
                },
                role: Role::Change,
                text: (*text).to_owned(),
                entry_method: EntryMethod::PhoneKeyboard,
                language: "en-US".to_owned(),
            },
        )?;
        plan.submit(&mut f.host, &device(), 11)?;
    }
    Ok(f)
}
fn package(f: &Fixture, number: u64) -> std::result::Result<Package, Box<dyn std::error::Error>> {
    let mut o = options()?;
    o.package_id = id(number)?;
    compiled(f, o)
}
fn review(
    original: &Package,
    after: &Package,
) -> std::result::Result<FailureReview, Box<dyn std::error::Error>> {
    Ok(FailureReview {
        fence: FailureReviewFence {
            original: VerificationPackageIdentity::from_package(original),
            after: VerificationPackageIdentity::from_package(after),
            comparison_receipt_id: id(500)?,
            comparison_receipt_blake3: "d".repeat(64),
            version: 3,
        },
        markers: vec![
            FailureMarkerAssessment {
                original_number: 1,
                verdict: FailureVerdict::Fail,
            },
            FailureMarkerAssessment {
                original_number: 2,
                verdict: FailureVerdict::Pass,
            },
            FailureMarkerAssessment {
                original_number: 3,
                verdict: FailureVerdict::Fail,
            },
        ],
    })
}
#[test]
fn deterministic_failure_package_keeps_original_numbers_and_existing_png_owners() -> TestResult {
    let original = package(&marked(&["first", "second", "third"])?, 99)?;
    let after = package(&marked(&[])?, 100)?;
    let r = review(&original, &after)?;
    let prepared = package(&marked(&["first", "third"])?, 101)?;
    let pngs: Vec<_> = prepared
        .files()
        .filter(|(p, _)| p.ends_with(".png"))
        .map(|(p, b)| (p.to_owned(), b.to_vec()))
        .collect();
    let output = compile_verification_failure_package(
        &original,
        &after,
        &r,
        Some(prepared),
        Limits::default(),
        |_| true,
        &NeverCancel,
    )?
    .ok_or("failure package")?;
    let origin = output
        .manifest()
        .extensions
        .verification_origin
        .as_ref()
        .ok_or("lineage")?;
    assert_eq!(origin.original_marker_numbers, vec![1, 3]);
    assert_eq!(origin.original_package_id, original.manifest().package_id);
    assert_eq!(origin.original_manifest_sha256, original.manifest_sha256());
    assert_eq!(
        output
            .manifest()
            .markers
            .iter()
            .map(|m| m.number)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    let prompt = std::str::from_utf8(output.file("prompt.md").ok_or("prompt")?)?;
    assert!(prompt.contains("Follow-up marker 2 refers to original marker 3."));
    assert!(prompt.ends_with(&format!("{UNTRUSTED_TEXT_NOTICE}\n")));
    for (path, bytes) in pngs {
        assert_eq!(output.file(&path).ok_or("PNG")?, bytes);
    }
    let repeated = compile_verification_failure_package(
        &original,
        &after,
        &r,
        Some(package(&marked(&["first", "third"])?, 101)?),
        Limits::default(),
        |_| true,
        &NeverCancel,
    )?
    .ok_or("repeat")?;
    assert_eq!(files(&output), files(&repeated));
    Package::from_files(files(&output), Limits::default(), &NeverCancel)?;
    Ok(())
}
#[test]
fn all_pass_returns_none_without_preparing_a_new_package_and_pending_refuses() -> TestResult {
    let original = package(&marked(&["first", "second", "third"])?, 99)?;
    let after = package(&marked(&[])?, 100)?;
    let mut r = review(&original, &after)?;
    for marker in &mut r.markers {
        marker.verdict = FailureVerdict::Pass;
    }
    assert!(
        compile_verification_failure_package(
            &original,
            &after,
            &r,
            None,
            Limits::default(),
            |_| true,
            &NeverCancel
        )?
        .is_none()
    );
    r.markers[1].verdict = FailureVerdict::Pending;
    assert!(select_verification_failures(&original, &after, &r).is_err());
    Ok(())
}
#[test]
fn changed_pair_target_receipt_version_duplicate_unknown_or_missing_marker_refuses() -> TestResult {
    let original = package(&marked(&["first", "second", "third"])?, 99)?;
    let after = package(&marked(&[])?, 100)?;
    let r = review(&original, &after)?;
    let mut wrong = r.clone();
    wrong.fence.original.target = "openai".to_owned();
    assert!(select_verification_failures(&original, &after, &wrong).is_err());
    let mut wrong = r.clone();
    wrong.fence.after.manifest_sha256 = "e".repeat(64);
    assert!(select_verification_failures(&original, &after, &wrong).is_err());
    let mut wrong = r.clone();
    wrong.fence.version = 0;
    assert!(select_verification_failures(&original, &after, &wrong).is_err());
    for number in [0, 1, 4] {
        let mut wrong = r.clone();
        wrong.markers[1].original_number = number;
        assert!(select_verification_failures(&original, &after, &wrong).is_err());
    }
    let mut wrong = r.clone();
    wrong.markers.pop();
    assert!(select_verification_failures(&original, &after, &wrong).is_err());
    for changed in [false, true] {
        let mut wrong = r.clone();
        if changed {
            wrong.fence.comparison_receipt_id = id(501)?;
        } else {
            wrong.fence.comparison_receipt_blake3 = "e".repeat(64);
        }
        assert!(matches!(
            compile_verification_failure_package(
                &original,
                &after,
                &wrong,
                None,
                Limits::default(),
                |current| current == &r,
                &NeverCancel
            ),
            Err(Error::Stale)
        ));
    }
    let mut changed_decision = r.clone();
    changed_decision.markers[1].verdict = FailureVerdict::Fail;
    assert!(matches!(
        compile_verification_failure_package(
            &original,
            &after,
            &changed_decision,
            None,
            Limits::default(),
            |current| current == &r,
            &NeverCancel
        ),
        Err(Error::Stale)
    ));
    Ok(())
}
#[test]
fn changed_prepared_instruction_or_source_and_self_reference_refuse() -> TestResult {
    let original = package(&marked(&["first", "second", "third"])?, 99)?;
    let after = package(&marked(&[])?, 100)?;
    let r = review(&original, &after)?;
    for (texts, identity) in [
        (vec!["first", "second"], 101),
        (vec!["first", "third"], 99),
        (vec!["first", "third"], 100),
        (vec!["first"], 101),
    ] {
        assert!(
            compile_verification_failure_package(
                &original,
                &after,
                &r,
                Some(package(&marked(&texts)?, identity)?),
                Limits::default(),
                |_| true,
                &NeverCancel
            )
            .is_err()
        );
    }
    let changed_source = fixture(65, 64, 8)?;
    assert!(
        compile_verification_failure_package(
            &original,
            &after,
            &r,
            Some(package(&changed_source, 101)?),
            Limits::default(),
            |_| true,
            &NeverCancel
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn stale_review_at_each_fence_or_cancellation_refuses_without_mutating_inputs() -> TestResult {
    let original = package(&marked(&["first", "second", "third"])?, 99)?;
    let after = package(&marked(&[])?, 100)?;
    let r = review(&original, &after)?;
    let old = files(&original);
    let captured = files(&after);
    for stop in 1..=3 {
        let calls = Cell::new(0);
        assert!(matches!(
            compile_verification_failure_package(
                &original,
                &after,
                &r,
                Some(package(&marked(&["first", "third"])?, 101)?),
                Limits::default(),
                |_| {
                    calls.set(calls.get() + 1);
                    calls.get() != stop
                },
                &NeverCancel
            ),
            Err(Error::Stale)
        ));
    }
    struct Cancel;
    impl Cancellation for Cancel {
        fn is_cancelled(&self) -> bool {
            true
        }
    }
    assert!(matches!(
        compile_verification_failure_package(
            &original,
            &after,
            &r,
            None,
            Limits::default(),
            |_| true,
            &Cancel
        ),
        Err(Error::Cancelled)
    ));
    struct Flipped<'a>(&'a Cell<bool>);
    impl Cancellation for Flipped<'_> {
        fn is_cancelled(&self) -> bool {
            self.0.get()
        }
    }
    let cancelled = Cell::new(false);
    assert!(matches!(
        compile_verification_failure_package(
            &original,
            &after,
            &r,
            None,
            Limits::default(),
            |_| {
                cancelled.set(true);
                true
            },
            &Flipped(&cancelled)
        ),
        Err(Error::Cancelled)
    ));
    assert_eq!(files(&original), old);
    assert_eq!(files(&after), captured);
    Ok(())
}
#[test]
fn final_normal_verifier_refuses_bad_lineage_and_budget_without_ignoring_proofs() -> TestResult {
    let original = package(&marked(&["first", "second", "third"])?, 99)?;
    let after = package(&marked(&[])?, 100)?;
    let r = review(&original, &after)?;
    let output = compile_verification_failure_package(
        &original,
        &after,
        &r,
        Some(package(&marked(&["first", "third"])?, 101)?),
        Limits::default(),
        |_| true,
        &NeverCancel,
    )?
    .ok_or("output")?;
    for numbers in [vec![1, 1], vec![0, 3], vec![1], vec![1, 513]] {
        let mut changed = files(&output);
        let item = changed
            .iter_mut()
            .find(|(p, _)| p == "manifest.json")
            .ok_or("manifest")?;
        let mut manifest: serde_json::Value = serde_json::from_slice(&item.1)?;
        manifest["extensions"]["verification_origin"]["original_marker_numbers"] =
            serde_json::json!(numbers);
        item.1 = serde_json::to_vec(&manifest)?;
        assert!(Package::from_files(changed, Limits::default(), &NeverCancel).is_err());
    }
    let tiny = Limits {
        memory_bytes: 16 * 1024 * 1024,
        ..Limits::default()
    };
    assert!(matches!(
        compile_verification_failure_package(
            &original,
            &after,
            &r,
            Some(package(&marked(&["first", "third"])?, 101)?),
            tiny,
            |_| true,
            &NeverCancel
        ),
        Err(Error::Limit(_))
    ));
    Ok(())
}
