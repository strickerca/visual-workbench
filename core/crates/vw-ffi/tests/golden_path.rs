//! The whole product loop on two in-process devices over loopback:
//! pair, share, mark up on the phone, sync, export, package for an agent,
//! receive the agent's result and open the verification review.
//!
//! This proves the pieces work together. It is not a device result: there is
//! no pen, screen, capture or real agent here.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use std::sync::Arc;
use support::*;
use vw_core::*;
use vw_model::DeviceId;

const BUDGET: u64 = 256 * 1024 * 1024;
const INSTRUCTION: &str = "Make this button green";

fn temp() -> tempfile::TempDir {
    if cfg!(target_os = "android") {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    }
}

fn package_key(info: &PackageInfo) -> VerificationPackageKey {
    VerificationPackageKey {
        package_id: info.package_id.clone(),
        target: info.target.clone(),
        manifest_sha256: info.manifest_sha256.clone(),
    }
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

async fn read(catalog: &Arc<PackageCatalog>, info: &PackageInfo, path: &str) -> Vec<u8> {
    catalog
        .read_file(
            info.package_id.clone(),
            info.target.clone(),
            info.manifest_sha256.clone(),
            path.into(),
            4 * 1024 * 1024,
            cancel(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn pair_share_mark_sync_export_package_and_verify() {
    let root = temp();

    // Pair once by QR. The PC is the fixture host device; the phone is the client.
    let (pc_service, phone_service) = paired().await;
    let phone_id = phone_service.local_device().await.unwrap().device_id;
    assert_ne!(phone_id, device());

    // See: the PC opens an image and shares the project. The phone receives a
    // complete local copy and connects.
    let pc = create_image_project(create(&root.path().join("pc")), cancel())
        .await
        .unwrap();
    let server = pc_service
        .host_project(
            pc.clone(),
            phone_id.clone(),
            vec![SessionEndpoint {
                carrier: AppCarrier::TcpAdb,
                address: "127.0.0.1:0".into(),
            }],
        )
        .await
        .unwrap();
    let phone = phone_service
        .receive_project(
            ReceiveProjectOptions {
                path: root.path().join("phone").to_string_lossy().into(),
                peer_device_id: device(),
                endpoints: server.endpoints(),
                expected_project_id: Some(id(1)),
                max_total_blob_bytes: 16 * 1024 * 1024,
            },
            cancel(),
        )
        .await
        .unwrap();
    let link = phone_service
        .connect_project(phone.clone(), device(), server.endpoints())
        .await
        .unwrap();
    converged(&server, &link).await;

    // Point: a pen stroke drawn on the phone.
    let before_marks = pc.info().await.unwrap();
    let mut pen = stroke();
    pen.device_id = phone_id.clone();
    pen.lamport = phone.info().await.unwrap().next_lamport;
    let gesture = phone.clone().begin_stroke(pen).await.unwrap();
    gesture.append_samples(batch()).await.unwrap();
    gesture.commit(cancel()).await.unwrap();

    // Say: a marker with its instruction, also placed on the phone.
    let document = phone
        .instruction_document(id(2), BUDGET, cancel())
        .await
        .unwrap();
    let phone_info = phone.info().await.unwrap();
    let plan = phone
        .clone()
        .prepare_instruction(
            document.binding,
            WorkflowMetadata {
                transaction_id: id(20),
                device_id: phone_id.clone(),
                first_lamport: phone_info.next_lamport,
                created_at_ms: TIME + 20,
            },
            InstructionCommand::PlaceMarker {
                object_id: id(21),
                instruction_id: id(22),
                layer_id: id(3),
                point: Point { x: 10.5, y: 11.5 },
                bounds: Some(QueryRect {
                    x: 6.0,
                    y: 7.0,
                    width: 12.0,
                    height: 10.0,
                }),
                element_eids: vec![],
                style: ObjectStyle {
                    rgba: 0xff2020ff,
                    width: 2.0,
                    screen_constant_width: true,
                    fill: None,
                },
                role: InstructionRole::Change,
                text: INSTRUCTION.into(),
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

    // Sync: both devices reach the same revision and state.
    converged(&server, &link).await;
    let pc_info = pc.info().await.unwrap();
    let phone_info = phone.info().await.unwrap();
    assert!(pc_info.host_seq > before_marks.host_seq);
    assert_eq!(pc_info.host_seq, phone_info.host_seq);
    assert_eq!(pc_info.state_hash, phone_info.state_hash);
    assert_ne!(pc_info.state_hash, before_marks.state_hash);
    let pc_items = pc.document_snapshot(id(2), cancel()).await.unwrap();
    let phone_items = phone.document_snapshot(id(2), cancel()).await.unwrap();
    assert_eq!(pc_items.render.items.len(), 2);
    assert_eq!(phone_items.render.items.len(), 2);

    // Export: the marked image is byte-identical from either device, and
    // differs from the clean image.
    let pc_marked = pc.export_image(options(), cancel()).await.unwrap();
    let phone_marked = phone.export_image(options(), cancel()).await.unwrap();
    assert_eq!(pc_marked.bytes, phone_marked.bytes);
    let mut clean_options = options();
    clean_options.marked = false;
    let pc_clean = pc.export_image(clean_options, cancel()).await.unwrap();
    assert_ne!(pc_marked.bytes, pc_clean.bytes);

    // Send: the PC compiles one package for an agent and publishes it.
    let catalog = open_package_catalog(root.path().to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let package = publish(&pc, &catalog, 40, 2).await;
    let manifest: serde_json::Value =
        serde_json::from_slice(&read(&catalog, &package, "manifest.json").await).unwrap();
    let manifest_text = manifest.to_string();
    assert!(
        manifest_text.contains(INSTRUCTION),
        "the package must carry the instruction written on the phone"
    );
    assert!(
        package
            .images
            .iter()
            .any(|image| image.role == "clean_source"),
        "the package must carry the clean source image"
    );
    assert_eq!(package.binding.host_seq, pc_info.host_seq);
    assert_eq!(package.binding.state_hash, pc_info.state_hash);

    // The agent acts and the result is recaptured as a second project, here a
    // synthetic one. Its package and the agent's returned image arrive.
    let mut after_options = create(&root.path().join("after"));
    after_options.project_id = id(11);
    after_options.document_id = id(12);
    after_options.layer_id = id(13);
    let after = create_image_project(after_options, cancel()).await.unwrap();
    let after_package = publish(&after, &catalog, 41, 12).await;
    let clean = after_package
        .images
        .iter()
        .find(|image| image.role == "clean_source")
        .unwrap();
    let returned = read(&catalog, &after_package, &clean.path).await;
    let inbox = open_mcp_result_inbox(root.path().to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let receipt = inbox
        .submit(
            catalog.clone(),
            McpInboxSubmission {
                receipt_id: id(50),
                package_id: package.package_id.clone(),
                target: package.target.clone(),
                manifest_sha256: package.manifest_sha256.clone(),
                connection: "ab".repeat(16),
                created_at_ms: TIME + 50,
                text: None,
                png: Some(returned),
                note: "agent result".into(),
            },
            cancel(),
        )
        .await
        .unwrap();

    // Verify: the review opens over the exact before/after pair with one
    // pending verdict for the one marker.
    let pair = VerificationPair {
        original: package_key(&package),
        after: package_key(&after_package),
        receipt_id: receipt.receipt_id,
        receipt_blake3: receipt.receipt_blake3,
    };
    let coordinates = catalog
        .verification_coordinate_pair(pair.clone(), cancel())
        .await
        .unwrap();
    assert!(coordinates.pair == pair);
    let review = pc
        .verification_review(
            after.clone(),
            catalog.clone(),
            inbox.clone(),
            pair,
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(review.marker_count, 1);
    assert!(
        review
            .markers
            .iter()
            .all(|marker| marker.verdict == VerificationVerdict::Pending)
    );

    link.close().await.unwrap();
    server.close().await.unwrap();
    phone.close().await.unwrap();
    after.close().await.unwrap();
    pc.close().await.unwrap();
    assert!(DeviceId::try_from(phone_id).is_ok());
}
