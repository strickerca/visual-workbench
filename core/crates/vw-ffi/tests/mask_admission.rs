//! Source-only regressions for the T2.01 accounting/facade patch handoff.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use support::*;
use vw_core::*;
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Object, Project};
use vw_ops::{HostSequencer, Replica};
use vw_proto::v1 as pb;

fn key(n: u8) -> Id {
    Id::try_from(id(n)).unwrap()
}
fn fixture(width: u32, height: u32) -> Project {
    let device = DeviceId::try_from(device()).unwrap();
    let original = AssetId::hash(b"absent synthetic original");
    let mask = AssetId::hash(b"absent synthetic mask");
    let mut project = Project::new(key(1), "Admission fixture".into(), device.clone());
    for asset in [&original, &mask] {
        project.assets.insert(
            asset.clone(),
            pb::AddAsset {
                asset_id: asset.to_string(),
                format: "png".into(),
                width,
                height,
                orientation: 1,
                bit_depth: 8,
                has_alpha: false,
                color_space: "untagged".into(),
                byte_size: 20,
                source: "import".into(),
                metadata_json: "{}".into(),
                ..Default::default()
            },
        );
    }
    project.documents.insert(
        key(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(key(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Synthetic image".into(),
                primary_asset_id: original.to_string(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: TIME,
        },
    );
    project.layers.insert(
        key(20),
        Layer {
            definition: pb::CreateLayer {
                layer_id: Some(key(20).to_proto()),
                document_id: Some(key(2).to_proto()),
                page_index: -1,
                name: "Selection".into(),
                kind: "mask".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    project.objects.insert(
        key(21),
        Object {
            document_id: key(2),
            state: pb::ObjectState {
                object_id: Some(key(21).to_proto()),
                layer_id: Some(key(20).to_proto()),
                order_key: "V".into(),
                transform: Some(pb::Affine {
                    a: 1.0,
                    d: 1.0,
                    ..Default::default()
                }),
                style: Some(pb::Style {
                    stroke: Some(pb::Color { rgba: 0x2878ffff }),
                    width: 1.0,
                    ..Default::default()
                }),
                role: pb::Role::None as i32,
                created_by: device.to_string(),
                created_at_ms: TIME,
                shape: Some(pb::object_state::Shape::SelectionRaster(
                    pb::RasterMaskRef {
                        mask_asset_id: mask.to_string(),
                        bounds: Some(pb::RectD {
                            x: 0.0,
                            y: 0.0,
                            w: f64::from(width),
                            h: f64::from(height),
                        }),
                        feather: 0.0,
                    },
                )),
                ..Default::default()
            },
        },
    );
    project.validate().unwrap();
    project
}
fn temporary() -> tempfile::TempDir {
    if cfg!(target_os = "android") {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    }
}

#[tokio::test]
async fn morphology_work_limit_precedes_missing_blob_access() {
    let dir = temporary();
    let path = dir.path().join("project");
    let device = DeviceId::try_from(device()).unwrap();
    drop(vw_store::ProjectStore::create(&path, fixture(10_000, 1_000), device, TIME).unwrap());
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let doc = project.selection_document(id(2), cancel()).await.unwrap();
    let request = SelectionEdit {
        binding: doc.binding,
        transaction_id: id(30),
        device_id: support::device(),
        lamport: doc.revision.next_lamport,
        created_at_ms: TIME + 30,
        target: SelectionTarget::Existing {
            selection: SelectionVersion {
                object_id: id(21),
                asset_id: AssetId::hash(b"absent synthetic mask").to_string(),
                version: 0,
            },
        },
        operation: SelectionOperation::Feather { radius: 64 },
        memory_budget_bytes: 256 * 1024 * 1024,
    };
    assert!(matches!(
        project.apply_selection(request, cancel()).await,
        Err(SelectionError::Limit)
    ));
    // No source or mask exists; a decoder/read would instead return Storage.
    assert_eq!(project.info().await.unwrap(), doc.revision);
    assert_eq!(std::fs::read_dir(path.join("blobs")).unwrap().count(), 0);
    project.close().await.unwrap();
}

#[tokio::test]
async fn canonical_workspace_is_reserved_before_missing_input_decode() {
    let dir = temporary();
    let path = dir.path().join("project");
    let device = DeviceId::try_from(device()).unwrap();
    drop(vw_store::ProjectStore::create(&path, fixture(64, 64), device, TIME).unwrap());
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let doc = project.selection_document(id(2), cancel()).await.unwrap();
    let request = SelectionEdit {
        binding: doc.binding,
        transaction_id: id(30),
        device_id: support::device(),
        lamport: doc.revision.next_lamport,
        created_at_ms: TIME + 30,
        target: SelectionTarget::Existing {
            selection: SelectionVersion {
                object_id: id(21),
                asset_id: AssetId::hash(b"absent synthetic mask").to_string(),
                version: 0,
            },
        },
        operation: SelectionOperation::Invert,
        memory_budget_bytes: 20 * 1024 * 1024,
    };
    assert!(matches!(
        project.apply_selection(request, cancel()).await,
        Err(SelectionError::Memory { .. })
    ));
    assert_eq!(project.info().await.unwrap(), doc.revision);
    project.close().await.unwrap();
}

#[test]
fn accounting_charges_small_elements_and_stops_at_the_requested_cap() {
    let mut project = fixture(64, 64);
    let first = project.assets.values_mut().next().unwrap();
    first.icc_profile = vec![0; 4096];
    first.color_space = "ICC".into();
    let host = HostSequencer::new(project, DeviceId::try_from(device()).unwrap()).unwrap();
    let estimate = host.workspace_estimate_bytes(32 * 1024 * 1024).unwrap();
    assert!(
        estimate > 4096 * 256,
        "byte vectors must charge Value nodes, not only encoded characters"
    );
    assert_eq!(host.workspace_estimate_bytes(1000).unwrap(), 1001);
    assert_eq!(
        host.workspace_estimate_bytes(estimate - 1).unwrap(),
        estimate
    );
    assert_eq!(host.workspace_estimate_bytes(estimate).unwrap(), estimate);
}

#[test]
fn accounting_retains_deleted_journal_and_replica_inverse_costs() {
    let device = DeviceId::try_from(device()).unwrap();
    let mut host = HostSequencer::new(fixture(64, 64), device.clone()).unwrap();
    let txn = pb::Transaction {
        txn_id: Some(key(30).to_proto()),
        project_id: Some(key(1).to_proto()),
        device_id: device.to_string(),
        base_revision: Some(host.revision().unwrap()),
        created_at_wall_ms: TIME + 30,
        gesture_id: None,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device.to_string(),
                lamport: 1,
            }),
            kind: Some(pb::op::Kind::DeleteObject(pb::DeleteObject {
                object_id: Some(key(21).to_proto()),
            })),
        }],
    };
    let mut replica = Replica::new(device.clone(), host.snapshot().unwrap()).unwrap();
    let before_replica = replica.workspace_estimate_bytes(32 * 1024 * 1024).unwrap();
    replica.queue(txn.clone()).unwrap();
    assert!(replica.workspace_estimate_bytes(32 * 1024 * 1024).unwrap() > before_replica);
    host.submit(txn, &device, TIME + 30).unwrap();
    let visible_only = HostSequencer::new(host.project().clone(), device).unwrap();
    assert!(
        host.workspace_estimate_bytes(32 * 1024 * 1024).unwrap()
            > visible_only
                .workspace_estimate_bytes(32 * 1024 * 1024)
                .unwrap()
    );
}
