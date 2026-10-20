//! Explicit image-output metadata survives durable storage and portable archives.
#![allow(clippy::unwrap_used)]

use vw_model::{AssetId, DeviceId, Document, IMAGE_OUTPUT_LAYER_KIND, Id, Project};
use vw_proto::v1 as pb;
use vw_store::{BlobStore, ImportLimits, ProjectStore, export_vwbz, import_vwbz};

fn id(number: u8) -> Id {
    Id::from_parts(1_700_000_000_000 + u64::from(number), [6; 10]).unwrap()
}

fn device() -> DeviceId {
    DeviceId::from_bytes([3; 16])
}

fn temporary() -> tempfile::TempDir {
    let mut builder = tempfile::Builder::new();
    builder.prefix("vw-output-layer-test-");
    #[cfg(target_os = "android")]
    {
        builder
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap()
    }
    #[cfg(not(target_os = "android"))]
    {
        builder.tempdir().unwrap()
    }
}

fn project(original: &[u8]) -> Project {
    let asset = AssetId::hash(original);
    let mut project = Project::new(id(1), "Synthetic output fixture".into(), device());
    project.assets.insert(
        asset.clone(),
        pb::AddAsset {
            asset_id: asset.to_string(),
            format: "png".into(),
            width: 40,
            height: 30,
            orientation: 1,
            bit_depth: 8,
            color_space: "sRGB".into(),
            byte_size: original.len() as u64,
            source: "import".into(),
            metadata_json: "{}".into(),
            ..Default::default()
        },
    );
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Synthetic source".into(),
                primary_asset_id: asset.to_string(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 0,
        },
    );
    project
}

fn output_transaction(base: pb::Revision) -> pb::Transaction {
    let kinds = [
        pb::op::Kind::CreateLayer(pb::CreateLayer {
            layer_id: Some(id(3).to_proto()),
            document_id: Some(id(2).to_proto()),
            page_index: -1,
            name: "Synthetic output".into(),
            kind: IMAGE_OUTPUT_LAYER_KIND.into(),
            order_key: "V".into(),
        }),
        pb::op::Kind::CreateObject(pb::CreateObject {
            document_id: Some(id(2).to_proto()),
            state: Some(pb::ObjectState {
                object_id: Some(id(4).to_proto()),
                layer_id: Some(id(3).to_proto()),
                order_key: "V".into(),
                transform: Some(pb::Affine {
                    a: 2.0,
                    d: 2.0,
                    e: -4.0,
                    f: -6.0,
                    ..Default::default()
                }),
                style: Some(pb::Style {
                    stroke: Some(pb::Color { rgba: 0 }),
                    width: 0.0,
                    ..Default::default()
                }),
                role: pb::Role::None as i32,
                created_by: device().to_string(),
                created_at_ms: 50,
                shape: Some(pb::object_state::Shape::Crop(pb::RectD {
                    x: 2.0,
                    y: 3.0,
                    w: 20.0,
                    h: 10.0,
                })),
                ..Default::default()
            }),
        }),
    ];
    pb::Transaction {
        txn_id: Some(id(10).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device().to_string(),
        base_revision: Some(base),
        created_at_wall_ms: 50,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: device().to_string(),
                    lamport: 20 + index as u64,
                }),
                kind: Some(kind),
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn output_layer_commit_reopens_with_exact_ack_and_roundtrips_through_archive() {
    let temporary = temporary();
    let path = temporary.path().join("output.vwb");
    let original = b"immutable synthetic output original";
    let asset = AssetId::hash(original);
    let mut store = ProjectStore::create(&path, project(original), device(), 0).unwrap();
    assert_eq!(BlobStore::new(&path).unwrap().put(original).unwrap(), asset);
    let transaction = output_transaction(store.revision().unwrap());
    let accepted = store.commit(&transaction, &device(), 100).unwrap();
    assert!(!accepted.duplicate);
    let revision = store.revision().unwrap();
    let checkpoint = store.checkpoint_bytes().unwrap();
    let canonical = store.project().canonical_bytes().unwrap();
    assert_eq!(
        store
            .project()
            .active_image_output(&id(2))
            .unwrap()
            .unwrap()
            .output_size(),
        [40, 20]
    );
    drop(store);

    let mut reopened = ProjectStore::open(&path).unwrap();
    assert_eq!(reopened.revision().unwrap(), revision);
    assert_eq!(reopened.checkpoint_bytes().unwrap(), checkpoint);
    assert_eq!(reopened.project().canonical_bytes().unwrap(), canonical);
    assert_eq!(
        reopened.project().layers[&id(3)].definition.kind,
        IMAGE_OUTPUT_LAYER_KIND
    );
    let retried = reopened.commit(&transaction, &device(), 200).unwrap();
    assert!(retried.duplicate);
    assert_eq!(retried.ack, accepted.ack);
    reopened.integrity_check().unwrap();
    let database = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    assert_eq!(
        database
            .query_row(
                "SELECT kind FROM layers WHERE layer_id=?1",
                [id(3).to_string()],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        IMAGE_OUTPUT_LAYER_KIND
    );
    assert_eq!(database.query_row(
        "SELECT COUNT(*) FROM objects JOIN layers USING(layer_id) WHERE object_id=?1 AND layers.kind='output'",
        [id(4).to_string()], |row| row.get::<_, i64>(0),
    ).unwrap(), 1);
    drop(database);

    let archive = temporary.path().join("output.vwbz");
    let report = export_vwbz(&mut reopened, &archive).unwrap();
    assert_eq!(report.blobs, 1);
    assert_eq!(report.state_hash, reopened.project().state_hash().unwrap());
    let imported_path = temporary.path().join("imported.vwb");
    let imported = import_vwbz(&archive, &imported_path, ImportLimits::default()).unwrap();
    assert_eq!(imported.revision().unwrap(), revision);
    assert_eq!(imported.checkpoint_bytes().unwrap(), checkpoint);
    assert_eq!(imported.project().canonical_bytes().unwrap(), canonical);
    assert_eq!(
        imported
            .project()
            .active_image_output(&id(2))
            .unwrap()
            .unwrap()
            .output_size(),
        [40, 20]
    );
    assert_eq!(
        BlobStore::new(&imported_path)
            .unwrap()
            .read(&asset)
            .unwrap(),
        original
    );
    imported.integrity_check().unwrap();
    drop(imported);
    drop(reopened);
    temporary.close().unwrap();
}
