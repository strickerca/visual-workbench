//! Synthetic joined-owner native composition; no OS capture/alignment evidence.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::sync::Arc;
use support::*;
use vw_core::*;
const BUDGET: u64 = 256 * 1024 * 1024;

fn key(info: &PackageInfo) -> VerificationPackageKey {
    VerificationPackageKey {
        package_id: info.package_id.clone(),
        target: info.target.clone(),
        manifest_sha256: info.manifest_sha256.clone(),
    }
}
async fn marked(
    project: &Arc<ProjectSession>,
    n: u8,
    text: &str,
    role: InstructionRole,
    bounds: Option<QueryRect>,
) {
    let document = project
        .instruction_document(id(2), BUDGET, cancel())
        .await
        .unwrap();
    let info = project.info().await.unwrap();
    let plan = project
        .clone()
        .prepare_instruction(
            document.binding,
            WorkflowMetadata {
                transaction_id: id(n),
                device_id: info.device_id,
                first_lamport: info.next_lamport,
                created_at_ms: TIME + i64::from(n),
            },
            InstructionCommand::PlaceMarker {
                object_id: id(n + 1),
                instruction_id: id(n + 2),
                layer_id: id(3),
                point: Point { x: 10.5, y: 11.5 },
                bounds,
                element_eids: vec![],
                style: ObjectStyle {
                    rgba: 0xff2020ff,
                    width: 2.0,
                    screen_constant_width: true,
                    fill: None,
                },
                role,
                text: text.into(),
                entry_method: InstructionEntryMethod::PhoneKeyboard,
                language: "en-US".into(),
            },
            BUDGET,
            cancel(),
        )
        .await
        .unwrap();
    plan.commit(cancel()).await.unwrap();
    plan.dispose();
}
async fn publish(
    project: &Arc<ProjectSession>,
    catalog: &Arc<PackageCatalog>,
    package: u8,
    document: u8,
) -> PackageInfo {
    let info = project.info().await.unwrap();
    let compiled = project
        .compile_package(
            PackageCompileOptions {
                binding: WorkflowBinding {
                    project_id: info.project_id,
                    document_id: id(document),
                    host_seq: info.host_seq,
                    state_hash: info.state_hash,
                },
                package_id: id(package),
                created_at_ms: TIME + i64::from(package),
                target: PackageTarget::Generic { max_long_edge: 512 },
                semantic_snapshot_id: None,
                include_window_title: false,
                assume_untagged_srgb: true,
                allow_depth_reduction: false,
                memory_budget_bytes: BUDGET,
            },
            cancel(),
        )
        .await
        .unwrap();
    let published = catalog.publish(compiled.clone(), cancel()).await.unwrap();
    compiled.dispose();
    published.info
}

#[tokio::test]
async fn durable_manual_cas_checks_real_pair_and_compiles_only_failed_markers_against_full_after_source()
 {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("original");
    let mut original = create_image_project(create(&path), cancel()).await.unwrap();
    marked(
        &original,
        20,
        "  first literal ```owner```\n",
        InstructionRole::Preserve,
        Some(QueryRect {
            x: 6.0,
            y: 7.0,
            width: 12.0,
            height: 10.0,
        }),
    )
    .await;
    marked(
        &original,
        30,
        "second point",
        InstructionRole::Explain,
        None,
    )
    .await;
    let mut after_options = create(&root.path().join("after"));
    after_options.project_id = id(11);
    after_options.document_id = id(12);
    after_options.layer_id = id(13);
    let after = create_image_project(after_options, cancel()).await.unwrap();
    let original_info = original.info().await.unwrap();
    let after_info = after.info().await.unwrap();
    let catalog = open_package_catalog(root.path().to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let original_package = publish(&original, &catalog, 40, 2).await;
    let after_package = publish(&after, &catalog, 41, 12).await;
    let clean = after_package
        .images
        .iter()
        .find(|image| image.role == "clean_source")
        .unwrap();
    let png = catalog
        .read_file(
            after_package.package_id.clone(),
            after_package.target.clone(),
            after_package.manifest_sha256.clone(),
            clean.path.clone(),
            4 * 1024 * 1024,
            cancel(),
        )
        .await
        .unwrap();
    let inbox = open_mcp_result_inbox(root.path().to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let receipt = inbox
        .submit(
            catalog.clone(),
            McpInboxSubmission {
                receipt_id: id(50),
                package_id: original_package.package_id.clone(),
                target: original_package.target.clone(),
                manifest_sha256: original_package.manifest_sha256.clone(),
                connection: "ab".repeat(16),
                created_at_ms: TIME + 50,
                text: None,
                png: Some(png),
                note: "synthetic comparison".into(),
            },
            cancel(),
        )
        .await
        .unwrap();
    let pair = VerificationPair {
        original: key(&original_package),
        after: key(&after_package),
        receipt_id: receipt.receipt_id,
        receipt_blake3: receipt.receipt_blake3,
    };
    let pending = original
        .verification_review(
            after.clone(),
            catalog.clone(),
            inbox.clone(),
            pair.clone(),
            cancel(),
        )
        .await
        .unwrap();
    assert!(
        pending.version == 0
            && pending.marker_count == 2
            && pending
                .markers
                .iter()
                .all(|m| m.verdict == VerificationVerdict::Pending)
    );
    let note = "private manual note, never portable";
    let one = original
        .change_verification_review(
            after.clone(),
            catalog.clone(),
            inbox.clone(),
            pair.clone(),
            0,
            id(60),
            1,
            VerificationVerdict::Fail,
            note.into(),
            cancel(),
        )
        .await
        .unwrap();
    let retry = original
        .change_verification_review(
            after.clone(),
            catalog.clone(),
            inbox.clone(),
            pair.clone(),
            0,
            id(60),
            1,
            VerificationVerdict::Fail,
            note.into(),
            cancel(),
        )
        .await
        .unwrap();
    assert!(one == retry && one.version == 1);
    assert!(matches!(
        original
            .change_verification_review(
                after.clone(),
                catalog.clone(),
                inbox.clone(),
                pair.clone(),
                0,
                id(60),
                1,
                VerificationVerdict::Fail,
                "altered retry".into(),
                cancel()
            )
            .await,
        Err(PackageError::Stale)
    ));
    assert!(matches!(
        original
            .compile_verification_failures(
                after.clone(),
                catalog.clone(),
                inbox.clone(),
                pair.clone(),
                one.clone(),
                vec![],
                id(90),
                TIME + 90,
                BUDGET,
                cancel()
            )
            .await,
        Err(PackageError::Invalid)
    ));
    let reviewed = original
        .change_verification_review(
            after.clone(),
            catalog.clone(),
            inbox.clone(),
            pair.clone(),
            1,
            id(61),
            2,
            VerificationVerdict::Pass,
            String::new(),
            cancel(),
        )
        .await
        .unwrap();
    original.close().await.unwrap();
    original = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    assert!(
        original
            .verification_review(
                after.clone(),
                catalog.clone(),
                inbox.clone(),
                pair.clone(),
                cancel()
            )
            .await
            .unwrap()
            == reviewed
    );
    let shift = VerificationMarkerTranslation {
        original_number: 1,
        dx: 3.0,
        dy: 4.0,
    };
    assert!(matches!(
        original
            .compile_verification_failures(
                after.clone(),
                catalog.clone(),
                inbox.clone(),
                pair.clone(),
                reviewed.clone(),
                vec![VerificationMarkerTranslation { dx: 100.0, ..shift }],
                id(90),
                TIME + 90,
                BUDGET,
                cancel()
            )
            .await,
        Err(PackageError::Invalid)
    ));
    let failures = original
        .compile_verification_failures(
            after.clone(),
            catalog.clone(),
            inbox.clone(),
            pair.clone(),
            reviewed.clone(),
            vec![shift],
            id(90),
            TIME + 90,
            BUDGET,
            cancel(),
        )
        .await
        .unwrap()
        .unwrap();
    let output = catalog.publish(failures.clone(), cancel()).await.unwrap();
    assert_eq!(output.info.package_id, id(90));
    assert_eq!(output.info.marker_count, 1);
    assert_eq!(output.info.binding.project_id, after_info.project_id);
    assert_eq!(output.info.source_asset_id, after_package.source_asset_id);
    let bytes = catalog
        .read_file(
            output.info.package_id.clone(),
            output.info.target.clone(),
            output.info.manifest_sha256.clone(),
            "manifest.json".into(),
            4 * 1024 * 1024,
            cancel(),
        )
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        manifest["markers"][0]["instruction"],
        "  first literal ```owner```\n"
    );
    assert_eq!(manifest["markers"][0]["role"], "preserve");
    assert_eq!(manifest["markers"][0]["kind"], "box");
    assert_eq!(
        manifest["markers"][0]["point_document"],
        serde_json::json!([13.5, 15.5])
    );
    assert_eq!(
        manifest["markers"][0]["bbox_document"],
        serde_json::json!([9.0, 11.0, 12.0, 10.0])
    );
    let origin = manifest["extensions"]["verification_origin"]
        .as_object()
        .unwrap();
    assert_eq!(origin.len(), 6);
    assert_eq!(origin["original_marker_numbers"], serde_json::json!([1]));
    for entry in manifest["files"].as_array().unwrap() {
        let bytes = catalog
            .read_file(
                output.info.package_id.clone(),
                output.info.target.clone(),
                output.info.manifest_sha256.clone(),
                entry["path"].as_str().unwrap().into(),
                4 * 1024 * 1024,
                cancel(),
            )
            .await
            .unwrap();
        assert!(
            !bytes
                .windows(note.len())
                .any(|window| window == note.as_bytes())
        );
    }
    assert!(!String::from_utf8_lossy(&bytes).contains(note));
    failures.dispose();
    assert_eq!(original.info().await.unwrap(), original_info);
    assert_eq!(after.info().await.unwrap(), after_info);
    let all_pass = original
        .change_verification_review(
            after.clone(),
            catalog.clone(),
            inbox.clone(),
            pair.clone(),
            2,
            id(62),
            1,
            VerificationVerdict::Pass,
            String::new(),
            cancel(),
        )
        .await
        .unwrap();
    assert!(matches!(
        original
            .compile_verification_failures(
                after.clone(),
                catalog.clone(),
                inbox.clone(),
                pair.clone(),
                reviewed,
                vec![shift],
                id(91),
                TIME + 91,
                BUDGET,
                cancel()
            )
            .await,
        Err(PackageError::Stale)
    ));
    assert!(
        original
            .compile_verification_failures(
                after.clone(),
                catalog.clone(),
                inbox.clone(),
                pair.clone(),
                all_pass,
                vec![],
                id(91),
                TIME + 91,
                16 * 1024 * 1024,
                cancel()
            )
            .await
            .unwrap()
            .is_none()
    );
    let cancelled = cancel();
    cancelled.cancel();
    assert!(matches!(
        original
            .verification_review(
                after.clone(),
                catalog.clone(),
                inbox.clone(),
                pair.clone(),
                cancelled
            )
            .await,
        Err(PackageError::Cancelled)
    ));
    inbox
        .retire(
            pair.receipt_id.clone(),
            pair.receipt_blake3.clone(),
            cancel(),
        )
        .await
        .unwrap();
    assert!(matches!(
        original
            .verification_review(
                after.clone(),
                catalog.clone(),
                inbox.clone(),
                pair,
                cancel()
            )
            .await,
        Err(PackageError::Stale)
    ));
    inbox.shutdown().await.unwrap();
    catalog.shutdown().await.unwrap();
    original.close().await.unwrap();
    after.close().await.unwrap();
}
