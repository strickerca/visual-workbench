#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::MAX_MEMORY;
use super::*;
use crate::*;
use std::sync::Arc;
use vw_model::{AssetId, DeviceId, Id};
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn id(n: u8) -> String {
    Id::from_parts(1, [n; 10]).unwrap().to_string()
}
fn cancel() -> Arc<Cancellation> {
    Arc::new(Cancellation::new())
}
fn temp() -> tempfile::TempDir {
    tempfile::tempdir_in(if cfg!(target_os = "android") {
        std::env::current_dir().unwrap()
    } else {
        std::env::temp_dir()
    })
    .unwrap()
}
fn source(depth: u8) -> Vec<u8> {
    let mut bytes = vec![];
    {
        let mut encoder = png::Encoder::new(&mut bytes, 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(if depth == 16 {
            png::BitDepth::Sixteen
        } else {
            png::BitDepth::Eight
        });
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().unwrap();
        let pixels = if depth == 16 {
            (0..256)
                .flat_map(|i| {
                    [1001 + i as u16, 2345, 65001, 40001]
                        .into_iter()
                        .flat_map(u16::to_be_bytes)
                })
                .collect::<Vec<_>>()
        } else {
            [31, 63, 127, 128].repeat(256)
        };
        writer.write_image_data(&pixels).unwrap();
        writer.finish().unwrap();
    }
    bytes
}
async fn fixture(
    root: &std::path::Path,
    depth: u8,
) -> (Arc<ProjectSession>, Arc<AiService>, Arc<AiRequest>, Vec<u8>) {
    let bytes = source(depth);
    let project = create_image_project(
        CreateImageProject {
            path: root.join("project").to_string_lossy().into(),
            project_id: id(1),
            document_id: id(2),
            layer_id: id(3),
            device_id: DeviceId::from_bytes([1; 16]).to_string(),
            title: "AI proof fixture".into(),
            source: bytes.clone(),
            now_ms: 1,
        },
        cancel(),
    )
    .await
    .unwrap();
    let service = create_ai_service(root.to_string_lossy().into(), true, cancel())
        .await
        .unwrap();
    let info = project.info().await.unwrap();
    let binding = WorkflowBinding {
        project_id: info.project_id,
        document_id: id(2),
        host_seq: info.host_seq,
        state_hash: info.state_hash,
    };
    let mut config =
        configuration::Configuration::parse(include_bytes!("provider-2026-10-02.json")).unwrap();
    config.provider.capabilities.min_pixels = 256;
    config.provider.capabilities.max_pixels = 4096;
    config.provider.capabilities.max_long_edge = 64;
    config.provider.capabilities.experimental_above_pixels = 4096;
    let options = AiPrepareOptions {
        binding: binding.clone(),
        selections: vec![],
        intent_id: id(4),
        instruction: "Offline fixture only".into(),
        configuration_fingerprint: config.fingerprint().unwrap(),
        estimate: None,
        feather_px: 0,
        allow_16bit_provider_copy: depth == 16,
        assume_untagged_srgb: false,
        memory_budget_bytes: MAX_MEMORY,
    };
    let mask = vw_mask::Mask::rectangle(
        vw_mask::Size::new(16, 16).unwrap(),
        vw_mask::Rect {
            x: 4.0,
            y: 4.0,
            width: 8.0,
            height: 8.0,
        },
    )
    .unwrap();
    let prepared = vw_ai::Prepared::new(
        &bytes,
        &[mask],
        vw_ai::PrepareOptions {
            provider: config.provider.clone(),
            revision: vw_ai::RevisionBinding {
                project_id: Id::try_from(binding.project_id.clone()).unwrap(),
                host_seq: binding.host_seq,
                state_hash: binding.state_hash.clone(),
            },
            intent_id: Id::try_from(id(4)).unwrap(),
            instructions: vec![vw_ai::Instruction {
                role: vw_ai::InstructionRole::Change,
                text: "Offline fixture".into(),
            }],
            feather_px: 0,
            estimated_tokens: Some(vw_ai::config::Tokens {
                text_input: 1,
                image_input: 1,
                image_output: 1,
            }),
            source_policy: vw_ai::SourcePolicy {
                assume_untagged_srgb: false,
                allow_16bit_provider_copy: depth == 16,
            },
            limits: vw_ai::Limits {
                memory_bytes: MAX_MEMORY - RAW_RESERVE,
            },
        },
        &vw_ai::NeverCancel,
    )
    .unwrap();
    let metadata = project
        .worker
        .call(|s| Ok(s.project()?.assets.values().next().unwrap().clone()))
        .await
        .unwrap();
    let request = AiRequest::new(
        project.clone(),
        service.clone(),
        prepared,
        options,
        metadata,
        config,
        prepare::RequestPermit::acquire().unwrap(),
    )
    .unwrap();
    request
        .worker
        .call(|s| {
            let response = s.prepared.mock_response([240, 70, 20, 90]).unwrap();
            s.raw = Arc::new(response.image_png().to_vec());
            let raw =
                vw_raster::decode(s.raw.as_slice(), vw_raster::DecodeLimits::default()).unwrap();
            s.raw_asset = Some(result::asset(
                &raw,
                &AssetId::hash(s.raw.as_slice()),
                s.raw.len() as u64,
            ));
            let completed = s.prepared.finish(response, &vw_ai::NeverCancel).unwrap();
            s.candidate_id = completed.proof().result_pixels_sha256.clone();
            s.base = Some(completed);
            s.send_consumed = true;
            Ok(())
        })
        .await
        .unwrap();
    (project, service, request, bytes)
}
async fn options(project: &Arc<ProjectSession>, request: &Arc<AiRequest>) -> AiSaveOptions {
    let info = project.info().await.unwrap();
    AiSaveOptions {
        expected: request.describe().binding,
        metadata: WorkflowMetadata {
            transaction_id: id(8),
            device_id: info.device_id,
            first_lamport: info.next_lamport,
            created_at_ms: 8,
        },
        result_id: id(9),
        layer_id: id(10),
        object_id: id(11),
        replace_object_id: None,
    }
}
async fn close(project: Arc<ProjectSession>, service: Arc<AiService>, request: Arc<AiRequest>) {
    request.shutdown().await.unwrap();
    drop(request);
    project.close().await.unwrap();
    drop(project);
    service.shutdown().await.unwrap();
    drop(service);
}
#[tokio::test]
async fn partial_mask_is_private_and_repeated_brushes_derive_from_immutable_base() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let (p, s, r, _) = fixture(dir.path(), 16).await;
    let original = p.info().await.unwrap();
    let first = r.candidate(cancel()).await.unwrap();
    let brush = |expected: String, next: u8, clear| AiAcceptanceBrush {
        expected_candidate_id: expected,
        next_candidate_id: id(next),
        points: vec![Point { x: 6.0, y: 6.0 }],
        radius: 2.0,
        opacity: 128,
        subtract: false,
        clear_first: clear,
    };
    let a = r
        .accept_brush(brush(first.candidate_id, 20, true), cancel())
        .await
        .unwrap();
    let region = AiRegion {
        x: 0,
        y: 0,
        width: 16,
        height: 16,
    };
    let one = r
        .compare(
            a.candidate_id.clone(),
            AiCompareMode::After,
            region,
            cancel(),
        )
        .await
        .unwrap();
    let b = r
        .accept_brush(brush(a.candidate_id, 21, true), cancel())
        .await
        .unwrap();
    let two = r
        .compare(
            b.candidate_id.clone(),
            AiCompareMode::After,
            region,
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(one.rgba_srgb, two.rgba_srgb);
    assert_eq!(b.proof.changed_unaccepted, Some(0));
    assert_eq!(
        b.proof.unaccepted_sha256_before,
        b.proof.unaccepted_sha256_after
    );
    assert_eq!(p.info().await.unwrap(), original);
    assert!(matches!(
        r.accept_brush(brush(b.candidate_id, 20, true), cancel())
            .await,
        Err(AiEditError::Stale)
    ));
    assert!(matches!(
        r.accept_brush(brush(first.request_id, 22, false), cancel())
            .await,
        Err(AiEditError::Stale)
    ));
    close(p, s, r).await;
}
#[tokio::test]
async fn partial_save_retries_reopens_renders_exact_16bit_exterior_and_undo_restores() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let path = dir.path().join("project");
    let (p, s, r, bytes) = fixture(dir.path(), 16).await;
    let before = p.info().await.unwrap();
    let c = r.candidate(cancel()).await.unwrap();
    let c = r
        .accept_brush(
            AiAcceptanceBrush {
                expected_candidate_id: c.candidate_id,
                next_candidate_id: id(20),
                points: vec![Point { x: 6.0, y: 6.0 }],
                radius: 2.0,
                opacity: 255,
                subtract: false,
                clear_first: true,
            },
            cancel(),
        )
        .await
        .unwrap();
    let opts = options(&p, &r).await;
    let receipt = r
        .save(c.candidate_id.clone(), opts.clone(), cancel())
        .await
        .unwrap();
    let draw = p.render_list(id(2), None, cancel()).await.unwrap();
    assert!(draw.items.iter().any(|item|matches!(&item.shape,DrawShape::Result{result_id,asset_id} if result_id==&id(9)&&asset_id==&receipt.composite_asset_id)));
    let retry = r.save(c.candidate_id, opts, cancel()).await.unwrap();
    assert_eq!(receipt.revision, retry.revision);
    assert_eq!(receipt.proof.changed_unaccepted, Some(0));
    let expected_asset = receipt.composite_asset_id.clone();
    let source_copy = bytes.clone();
    p.worker
        .call(move |state| {
            struct Assets(vw_store::BlobStore);
            impl vw_raster::AssetResolver for Assets {
                fn image(
                    &self,
                    id: &str,
                ) -> std::result::Result<vw_raster::DecodedImage, vw_raster::RasterError>
                {
                    let bytes = self
                        .0
                        .read(&AssetId::try_from(id.to_owned()).unwrap())
                        .unwrap();
                    vw_raster::decode(&bytes, vw_raster::DecodeLimits::default())
                }
            }
            assert_eq!(
                state.blobs.read(&AssetId::hash(&source_copy)).unwrap(),
                source_copy
            );
            let source =
                vw_raster::decode(&source_copy, vw_raster::DecodeLimits::default()).unwrap();
            let expected = vw_raster::decode(
                &state
                    .blobs
                    .read(&AssetId::try_from(expected_asset).unwrap())
                    .unwrap(),
                vw_raster::DecodeLimits::default(),
            )
            .unwrap();
            let rendered = vw_raster::render_document(
                &source,
                state.project().unwrap(),
                &Id::try_from(id(2)).unwrap(),
                &Assets(state.blobs.clone()),
                vw_raster::RenderOptions {
                    memory_budget_bytes: MAX_MEMORY,
                    include_guides: false,
                    assume_untagged_srgb: false,
                },
            )
            .unwrap();
            assert_eq!(rendered.pixels, expected.pixels);
            let candidate = &state.project().unwrap().results[&Id::try_from(id(9)).unwrap()];
            let mut proof: serde_json::Value =
                serde_json::from_str(&candidate.definition.proof_json).unwrap();
            proof["proof"]["acceptance"]["after_sha256"] =
                serde_json::Value::String("0".repeat(64));
            assert!(
                result::validate_materialized(
                    &serde_json::to_string(&proof).unwrap(),
                    &AssetId::hash(&source_copy),
                    &AssetId::try_from(candidate.definition.composite_asset_id.clone()).unwrap(),
                    candidate
                        .acceptance_mask_asset_id
                        .as_ref()
                        .map(AssetId::as_str)
                )
                .is_err()
            );
            let (vw_raster::Pixels::Rgba16(old), vw_raster::Pixels::Rgba16(new)) =
                (&source.pixels, &rendered.pixels)
            else {
                panic!("depth");
            };
            assert_eq!(&old[..4], &new[..4]);
            assert_eq!(new[3], 40001);
            Ok(())
        })
        .await
        .unwrap();
    let info = p.info().await.unwrap();
    let undone = p
        .undo_redo(
            EditOptions {
                transaction_id: id(30),
                document_id: id(2),
                device_id: info.device_id,
                lamport: info.next_lamport,
                created_at_ms: 30,
            },
            false,
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(undone.state_hash, before.state_hash);
    let info = p.info().await.unwrap();
    p.undo_redo(
        EditOptions {
            transaction_id: id(31),
            document_id: id(2),
            device_id: info.device_id,
            lamport: info.next_lamport,
            created_at_ms: 31,
        },
        true,
        cancel(),
    )
    .await
    .unwrap();
    close(p, s, r).await;
    let reopened = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let info = reopened.info().await.unwrap();
    let tile = reopened
        .ai_result_pixels(
            WorkflowBinding {
                project_id: info.project_id,
                document_id: id(2),
                host_seq: info.host_seq,
                state_hash: info.state_hash,
            },
            id(9),
            AiRegion {
                x: 0,
                y: 0,
                width: 16,
                height: 16,
            },
            MAX_MEMORY,
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(tile.rgba_srgb.len(), 1024);
    reopened.close().await.unwrap();
}
#[tokio::test]
async fn save_rejects_changed_review_cancelled_commit_and_postsave_brush() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let (p, s, r, _) = fixture(dir.path(), 8).await;
    let before = p.info().await.unwrap();
    let c = r.candidate(cancel()).await.unwrap();
    let mut opts = options(&p, &r).await;
    opts.expected.state_hash = "a".repeat(64);
    assert!(matches!(
        r.save(c.candidate_id.clone(), opts, cancel()).await,
        Err(AiEditError::Stale)
    ));
    assert_eq!(before, p.info().await.unwrap());
    let token = cancel();
    token.cancel();
    assert!(matches!(
        r.save(c.candidate_id.clone(), options(&p, &r).await, token)
            .await,
        Err(AiEditError::Cancelled)
    ));
    assert_eq!(before, p.info().await.unwrap());
    r.save(c.candidate_id.clone(), options(&p, &r).await, cancel())
        .await
        .unwrap();
    assert!(matches!(
        r.accept_brush(
            AiAcceptanceBrush {
                expected_candidate_id: c.candidate_id,
                next_candidate_id: id(20),
                points: vec![Point { x: 2.0, y: 2.0 }],
                radius: 1.0,
                opacity: 255,
                subtract: false,
                clear_first: true
            },
            cancel()
        )
        .await,
        Err(AiEditError::Stale)
    ));
    close(p, s, r).await;
}
#[tokio::test]
async fn wrong_send_binding_never_reads_credentials_or_reserves_and_shutdown_releases_permit() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let (p, s, r, _) = fixture(dir.path(), 8).await;
    r.worker
        .call(|state| {
            state.send_consumed = false;
            Ok(())
        })
        .await
        .unwrap();
    assert!(matches!(
        r.send("wrong".into(), 1, false, cancel()).await,
        Err(AiEditError::Estimate)
    ));
    {
        use vw_ai::budget::BudgetLedger;
        assert!(
            s.state
                .lock()
                .unwrap()
                .ledger
                .entry(&r.describe().request_id)
                .unwrap()
                .is_none()
        );
    }
    close(p, s, r).await;
    let permit = prepare::RequestPermit::acquire().unwrap();
    drop(permit);
}
#[test]
fn ledger_provisioning_is_exclusive_and_missing_history_never_resets() {
    let dir = temp();
    let path = dir.path().to_str().unwrap();
    assert!(private::open(path, false).is_err());
    let opened = private::open(path, true).unwrap();
    assert!(matches!(private::open(path, false), Err(AiEditError::Busy)));
    drop(opened);
    std::fs::remove_file(dir.path().join("ai-budget-v1/budget.sqlite")).unwrap();
    assert!(private::open(path, true).is_err());
    assert!(!dir.path().join("ai-budget-v1/budget.sqlite").exists());
}
#[test]
fn materialized_receipt_rejects_malformed_and_oversized_proof() {
    let source = AssetId::hash(b"source");
    let composite = AssetId::hash(b"composite");
    assert!(result::validate_materialized("{}", &source, &composite, None).is_err());
    assert!(
        result::validate_materialized(&"x".repeat(16 * 1024 + 1), &source, &composite, None)
            .is_err()
    );
}

#[tokio::test]
async fn context_recovers_all_visible_change_masks_and_preserves_instruction_roles() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let (p, s, r, _) = fixture(dir.path(), 8).await;
    let doc = p.selection_document(id(2), cancel()).await.unwrap();
    let info = p.info().await.unwrap();
    let selected = p
        .apply_selection(
            SelectionEdit {
                binding: doc.binding,
                transaction_id: id(40),
                device_id: info.device_id.clone(),
                lamport: info.next_lamport,
                created_at_ms: 40,
                target: SelectionTarget::New {
                    object_id: id(41),
                    layer_id: id(42),
                },
                operation: SelectionOperation::Rectangle {
                    rectangle: QueryRect {
                        x: 4.0,
                        y: 4.0,
                        width: 4.0,
                        height: 4.0,
                    },
                    combine: SelectionCombine::Add,
                },
                memory_budget_bytes: 256 * 1024 * 1024,
            },
            cancel(),
        )
        .await
        .unwrap();
    p.worker
        .call(|state| {
            let info = state.info()?;
            let kinds = vec![
                vw_proto::v1::op::Kind::SetProperty(vw_proto::v1::SetProperty {
                    object_id: Some(Id::try_from(id(41)).unwrap().to_proto()),
                    property: "role".into(),
                    value: Some(vw_proto::v1::PropertyValue {
                        value: Some(vw_proto::v1::property_value::Value::Role(
                            vw_proto::v1::Role::Change as i32,
                        )),
                    }),
                }),
                vw_proto::v1::op::Kind::SetInstruction(vw_proto::v1::SetInstruction {
                    instruction_id: Some(Id::try_from(id(43)).unwrap().to_proto()),
                    document_id: Some(Id::try_from(id(2)).unwrap().to_proto()),
                    target_object_ids: vec![],
                    role: vw_proto::v1::Role::Preserve as i32,
                    text: "Keep the literal original caption".into(),
                    entry_method: "pc_keyboard".into(),
                    language: "en-US".into(),
                }),
            ];
            let ops = kinds
                .into_iter()
                .enumerate()
                .map(|(i, kind)| vw_proto::v1::Op {
                    op_id: Some(vw_proto::v1::OpId {
                        device_id: info.device_id.clone(),
                        lamport: info.next_lamport + i as u64,
                    }),
                    kind: Some(kind),
                })
                .collect();
            let txn = vw_proto::v1::Transaction {
                txn_id: Some(Id::try_from(id(44)).unwrap().to_proto()),
                project_id: Some(Id::try_from(id(1)).unwrap().to_proto()),
                device_id: info.device_id.clone(),
                base_revision: Some(state.store()?.revision()?),
                created_at_wall_ms: 44,
                gesture_id: None,
                ops,
            };
            state.commit(&txn, &DeviceId::try_from(info.device_id)?, 44)?;
            Ok(())
        })
        .await
        .unwrap();
    let info = p.info().await.unwrap();
    let binding = WorkflowBinding {
        project_id: info.project_id,
        document_id: id(2),
        host_seq: info.host_seq,
        state_hash: info.state_hash,
    };
    let context = p
        .ai_context(binding.clone(), 256 * 1024 * 1024, cancel())
        .await
        .unwrap();
    assert_eq!(context.change_selections, vec![selected.snapshot.selection]);
    assert_eq!(context.instructions[0].role, "preserve");
    assert_eq!(
        context.instructions[0].text,
        "Keep the literal original caption"
    );
    p.worker
        .call(move |state| {
            assert!(
                crate::masks::ai_bridge::inputs(
                    state,
                    &binding,
                    &[],
                    MAX_MEMORY,
                    &Cancellation::new()
                )
                .is_err()
            );
            let (_, instructions) = context::collect(state, &binding).unwrap();
            assert_eq!(instructions[0].role, vw_ai::InstructionRole::Preserve);
            Ok(())
        })
        .await
        .unwrap();
    close(p, s, r).await;
}

#[tokio::test]
async fn budget_update_is_fingerprinted_and_preserves_provider_configuration() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let service = create_ai_service(dir.path().to_string_lossy().into(), true, cancel())
        .await
        .unwrap();
    let original = service.configuration(cancel()).await.unwrap();
    let updated = service
        .configure_daily_budget(original.fingerprint.clone(), 7_000_000, cancel())
        .await
        .unwrap();
    assert_ne!(original.fingerprint, updated.fingerprint);
    assert_eq!(updated.daily_soft_budget_microusd, 7_000_000);
    let a: serde_json::Value = serde_json::from_str(&original.json).unwrap();
    let b: serde_json::Value = serde_json::from_str(&updated.json).unwrap();
    assert_eq!(a["provider"], b["provider"]);
    assert_eq!(a["provenance"], b["provenance"]);
    assert!(matches!(
        service
            .configure_daily_budget(original.fingerprint, 1, cancel())
            .await,
        Err(AiEditError::Stale)
    ));
    service.shutdown().await.unwrap();
    drop(service);
    let reopened = create_ai_service(dir.path().to_string_lossy().into(), false, cancel())
        .await
        .unwrap();
    assert_eq!(
        reopened.configuration(cancel()).await.unwrap().fingerprint,
        updated.fingerprint
    );
    reopened.shutdown().await.unwrap();
    drop(reopened);
}

async fn grow_unrelated_documents_without_touching_request_source(project: &Arc<ProjectSession>) {
    project
        .worker
        .call(|state| {
            let mut grown = state.project()?.clone();
            let template = grown.documents[&Id::try_from(id(2)).unwrap()].clone();
            for n in 0..600u32 {
                let mut entropy = [77u8; 10];
                entropy[..4].copy_from_slice(&n.to_be_bytes());
                let key = Id::from_parts(2, entropy).unwrap();
                let mut document = template.clone();
                document.definition.document_id = Some(key.to_proto());
                document.definition.title = "x".repeat(4096);
                grown.documents.insert(key, document);
            }
            // A real validated canonical replica supplies the large borrowed state;
            // no provider fixture or forged estimator substitutes for production.
            let device = state.store()?.local_device()?;
            let host = vw_ops::HostSequencer::new(grown, device.clone()).unwrap();
            state.replica = Some(vw_ops::Replica::new(device, host.snapshot().unwrap()).unwrap());
            assert!(
                crate::workflow::canonical_workspace(state, crate::workflow::MAX_MEMORY, false)
                    .is_ok()
            );
            assert!(
                crate::workflow::canonical_workspace(state, crate::workflow::MAX_MEMORY, true)
                    .is_err()
            );
            Ok(())
        })
        .await
        .unwrap();
}
#[tokio::test]
async fn complete_result_admission_refuses_a_project_that_still_fits_read_admission() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let (p, s, r, _) = fixture(dir.path(), 8).await;
    grow_unrelated_documents_without_touching_request_source(&p).await;
    let reserve = r.save_admission;
    p.worker
        .call(move |state| {
            assert!(
                crate::workflow::canonical_workspace(state, crate::workflow::MAX_MEMORY, false)
                    .is_ok()
            );
            assert!(matches!(reserve.check(state), Err(AiEditError::Limit)));
            Ok(())
        })
        .await
        .unwrap();
    {
        use vw_ai::budget::BudgetLedger;
        assert!(
            s.state
                .lock()
                .unwrap()
                .ledger
                .entry(&r.describe().request_id)
                .unwrap()
                .is_none()
        );
    }
    close(p, s, r).await;
}
#[tokio::test]
async fn post_prepare_growth_is_admitted_before_stale_hash_or_provider_attempt() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let (p, s, r, _) = fixture(dir.path(), 8).await;
    r.worker
        .call(|state| {
            state.send_consumed = false;
            Ok(())
        })
        .await
        .unwrap();
    grow_unrelated_documents_without_touching_request_source(&p).await;
    // The old binding is now stale. Limit must win before check_binding hashes
    // the oversized new state, before request validation and before credentials.
    assert!(matches!(
        r.send("deliberately-wrong-request".into(), 1, false, cancel())
            .await,
        Err(AiEditError::Limit)
    ));
    r.worker
        .call(|state| {
            assert!(!state.send_consumed);
            Ok(())
        })
        .await
        .unwrap();
    {
        use vw_ai::budget::BudgetLedger;
        assert!(
            s.state
                .lock()
                .unwrap()
                .ledger
                .entry(&r.describe().request_id)
                .unwrap()
                .is_none()
        );
    }
    close(p, s, r).await;
}
