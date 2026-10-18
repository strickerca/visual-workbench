//! Root manifest/admission contract tests. These descriptive imported identities
//! never establish a review verdict, retained OS target or image alignment.
mod common;
use common::*;
use serde_json::{Value, json};
use vw_instructions::{DocumentView, EditMetadata, EntryMethod, MarkerPlacement, Role};
use vw_package::*;
use vw_proto::v1;

type TestValue<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn package(count: usize) -> TestValue<Package> {
    let mut f = fixture(32, 32, 8)?;
    for index in 0..count {
        let index = u64::try_from(index)?;
        let view = DocumentView::new(f.host.project(), &f.host.revision()?, &f.document)?;
        let plan = view.place_marker(
            EditMetadata {
                transaction_id: id(100 + index)?,
                device: device(),
                first_lamport: 3000 + index * 100,
                created_at_ms: 10,
            },
            MarkerPlacement {
                object_id: id(10 + index)?,
                instruction_id: id(20 + index)?,
                layer_id: id(3)?,
                point: v1::PointD { x: 10.0, y: 10.0 },
                bounds: None,
                element_eids: vec![],
                style: v1::Style {
                    stroke: Some(v1::Color { rgba: 0xf03c32ff }),
                    width: 2.0,
                    ..Default::default()
                },
                role: Role::Change,
                text: "Fixture owner instruction".into(),
                entry_method: EntryMethod::PhoneKeyboard,
                language: "en-US".into(),
            },
        )?;
        plan.submit(&mut f.host, &device(), 11)?;
    }
    compiled(&f, options()?)
}
fn origin(numbers: &[u32]) -> TestValue<Value> {
    Ok(json!({
        "original_package_id": id(201)?,
        "original_target": "generic",
        "original_manifest_sha256": "a".repeat(64),
        "comparison_receipt_id": id(202)?,
        "comparison_receipt_blake3": "b".repeat(64),
        "original_marker_numbers": numbers,
    }))
}
fn with_origin(package: &Package, origin: Value) -> TestValue<Vec<(String, Vec<u8>)>> {
    let mut output = files(package);
    let manifest = output
        .iter_mut()
        .find(|(name, _)| name == "manifest.json")
        .ok_or("fixture manifest")?;
    let mut value: Value = serde_json::from_slice(&manifest.1)?;
    value["extensions"]["verification_origin"] = origin;
    manifest.1 = serde_json::to_vec(&value)?;
    Ok(output)
}

#[test]
fn ordinary_packages_omit_lineage_and_absent_or_null_imports_remain_ordinary() -> TestResult {
    let ordinary = package(1)?;
    assert!(ordinary.manifest().extensions.verification_origin.is_none());
    let manifest: Value =
        serde_json::from_slice(ordinary.file("manifest.json").ok_or("manifest")?)?;
    assert!(manifest["extensions"].get("verification_origin").is_none());
    let reopened = Package::from_files(files(&ordinary), Limits::default(), &NeverCancel)?;
    assert!(reopened.manifest().extensions.verification_origin.is_none());
    assert_eq!(reopened.manifest_sha256(), ordinary.manifest_sha256());
    let explicit_null = Package::from_files(
        with_origin(&ordinary, Value::Null)?,
        Limits::default(),
        &NeverCancel,
    )?;
    assert!(
        explicit_null
            .manifest()
            .extensions
            .verification_origin
            .is_none()
    );
    Ok(())
}

#[test]
fn public_lineage_preserves_mapping_order_and_every_existing_image_and_proof() -> TestResult {
    let ordinary = package(2)?;
    let reopened = Package::from_files(
        with_origin(&ordinary, origin(&[512, 7])?)?,
        Limits::default(),
        &NeverCancel,
    )?;
    let lineage = reopened
        .manifest()
        .extensions
        .verification_origin
        .as_ref()
        .ok_or("origin")?;
    assert_eq!(lineage.original_package_id, id(201)?);
    assert_eq!(lineage.comparison_receipt_id, id(202)?);
    assert_eq!(lineage.original_marker_numbers, [512, 7]);
    assert_eq!(reopened.manifest().markers[0].number, 1);
    assert_eq!(reopened.manifest().markers[1].number, 2);
    assert_ne!(reopened.manifest_sha256(), ordinary.manifest_sha256());
    for (name, bytes) in ordinary
        .files()
        .filter(|(name, _)| *name != "manifest.json")
    {
        assert_eq!(reopened.file(name).ok_or("payload")?, bytes);
    }
    // The imported original number need not exist locally. This validates a
    // descriptive format only; the failure compiler must check the actual original.
    Ok(())
}

#[test]
fn lineage_rejects_self_reference_target_mismatch_bad_hashes_and_malformed_ids() -> TestResult {
    let ordinary = package(1)?;
    let malformed = [
        ("original_package_id", json!(ordinary.manifest().package_id)),
        ("original_package_id", json!("not-an-id")),
        ("comparison_receipt_id", json!("not-an-id")),
        ("original_target", json!("claude")), // Known, but unlike this package's target.
        ("original_target", json!("arbitrary-app")),
        ("original_manifest_sha256", json!("A".repeat(64))),
        ("original_manifest_sha256", json!("a".repeat(63))),
        ("comparison_receipt_blake3", json!("g".repeat(64))),
        ("comparison_receipt_blake3", json!("b".repeat(65))),
    ];
    for (key, value) in malformed {
        let mut invalid = origin(&[1])?;
        invalid[key] = value;
        assert!(
            Package::from_files(
                with_origin(&ordinary, invalid)?,
                Limits::default(),
                &NeverCancel
            )
            .is_err(),
            "{key}"
        );
    }
    Ok(())
}

#[test]
fn lineage_requires_nonempty_unique_in_range_mapping_with_current_marker_count() -> TestResult {
    let ordinary = package(2)?;
    for numbers in [
        json!([]),
        json!([1]),
        json!([1, 1]),
        json!([0, 1]),
        json!([1, 513]),
        json!([1, 2, 3]),
        json!([-1, 1]),
        json!([1.5, 2]),
        json!(["1", 2]),
        json!([4_294_967_296u64, 2]),
    ] {
        let mut invalid = origin(&[1, 2])?;
        invalid["original_marker_numbers"] = numbers;
        assert!(
            Package::from_files(
                with_origin(&ordinary, invalid)?,
                Limits::default(),
                &NeverCancel
            )
            .is_err()
        );
    }
    let empty = package(0)?;
    assert!(
        Package::from_files(
            with_origin(&empty, origin(&[])?)?,
            Limits::default(),
            &NeverCancel
        )
        .is_err()
    );
    assert!(
        Package::from_files(
            with_origin(&empty, origin(&[1])?)?,
            Limits::default(),
            &NeverCancel
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn lineage_denies_private_unknown_missing_and_duplicate_members() -> TestResult {
    let ordinary = package(1)?;
    let mut unknown = origin(&[1])?;
    unknown["private_selector"] = json!("fixture-only-not-a-retained-target");
    assert!(
        Package::from_files(
            with_origin(&ordinary, unknown)?,
            Limits::default(),
            &NeverCancel
        )
        .is_err()
    );
    let mut missing = origin(&[1])?;
    missing
        .as_object_mut()
        .ok_or("object")?
        .remove("comparison_receipt_id");
    assert!(
        Package::from_files(
            with_origin(&ordinary, missing)?,
            Limits::default(),
            &NeverCancel
        )
        .is_err()
    );
    let mut duplicate = with_origin(&ordinary, origin(&[1])?)?;
    let manifest = duplicate
        .iter_mut()
        .find(|(name, _)| name == "manifest.json")
        .ok_or("manifest")?;
    let text = std::str::from_utf8(&manifest.1)?.replacen(
        "\"original_target\":\"generic\"",
        "\"original_target\":\"generic\",\"original_target\":\"generic\"",
        1,
    );
    assert_ne!(text.as_bytes(), manifest.1.as_slice());
    manifest.1 = text.into_bytes();
    assert!(Package::from_files(duplicate, Limits::default(), &NeverCancel).is_err());
    Ok(())
}

#[test]
fn lineage_collection_preflight_precedes_typed_allocation_and_cannot_escape_its_key() -> TestResult
{
    let limits = Limits {
        markers: 1,
        ..Limits::default()
    };
    // A typed parser would reject the first null; the scanner instead reaches
    // the second entry and refuses before deserializing or growing a vector.
    let incomplete = vec![(
        "manifest.json".into(),
        br#"{"extensions":{"verification_origin":{"original_marker_numbers":[null,null"#.to_vec(),
    )];
    assert!(matches!(
        Package::from_files(incomplete, limits, &NeverCancel),
        Err(Error::Limit("JSON collection"))
    ));
    let escaped = vec![(
        "manifest.json".into(),
        br#"{"extensions":{"verification_origin":{"original_marker_nu\u006dbers":[null,null"#
            .to_vec(),
    )];
    assert!(matches!(
        Package::from_files(escaped, limits, &NeverCancel),
        Err(Error::Integrity)
    ));
    Ok(())
}

#[test]
fn valid_lineage_cannot_bypass_existing_package_payload_integrity() -> TestResult {
    let ordinary = package(1)?;
    let mut corrupt = with_origin(&ordinary, origin(&[1])?)?;
    corrupt
        .iter_mut()
        .find(|(name, _)| name == "images/clean_source.png")
        .ok_or("image")?
        .1
        .push(1);
    assert!(matches!(
        Package::from_files(corrupt, Limits::default(), &NeverCancel),
        Err(Error::Integrity)
    ));
    Ok(())
}
