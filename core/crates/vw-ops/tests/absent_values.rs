//! A write that leaves an optional property absent (a recolour of an unfilled
//! shape, a result update without an acceptance mask) is journaled with null
//! on both sides. A checkpoint holding such a record must load again, and the
//! record must still undo.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use vw_model::{AssetId, DeviceId, Id, Project};
use vw_ops::HostSequencer;
use vw_proto::v1::{self as pb, object_state::Shape, op::Kind, property_value::Value};

const CLOCK: u64 = 1_700_000_000_000;
fn id(n: u64) -> Id {
    Id::from_parts(CLOCK + n, [0x31; 10]).unwrap()
}
fn uid(n: u64) -> Option<pb::Uuid> {
    Some(id(n).to_proto())
}
fn owner() -> DeviceId {
    DeviceId::from_bytes([1; 16])
}
fn submit(host: &mut HostSequencer, n: u64, kinds: Vec<Kind>) -> pb::TxnAck {
    let txn = pb::Transaction {
        txn_id: uid(10_000 + n),
        project_id: uid(1),
        device_id: owner().to_string(),
        base_revision: Some(host.revision().unwrap()),
        created_at_wall_ms: 100,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: owner().to_string(),
                    lamport: n * 100 + i as u64 + 1,
                }),
                kind: Some(kind),
            })
            .collect(),
        gesture_id: None,
    };
    host.submit(txn, &owner(), CLOCK as i64).unwrap().ack
}
fn style(rgba: u32) -> pb::Style {
    // No fill: the optional fill colour is absent.
    pb::Style {
        stroke: Some(pb::Color { rgba }),
        width: 1.0,
        ..Default::default()
    }
}
fn fixture() -> HostSequencer {
    let asset = pb::AddAsset {
        asset_id: AssetId::hash(b"immutable fixture original").to_string(),
        format: "png".into(),
        width: 128,
        height: 64,
        orientation: 1,
        bit_depth: 8,
        color_space: "sRGB".into(),
        byte_size: 26,
        source: "import".into(),
        metadata_json: "{}".into(),
        ..Default::default()
    };
    let mut host = HostSequencer::new(
        Project::new(id(1), "Absent values fixture".into(), owner()),
        owner(),
    )
    .unwrap();
    submit(
        &mut host,
        1,
        vec![
            Kind::AddAsset(asset.clone()),
            Kind::CreateDocument(pb::CreateDocument {
                document_id: uid(2),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Fixture document".into(),
                primary_asset_id: asset.asset_id,
                capture: None,
            }),
            Kind::CreateLayer(pb::CreateLayer {
                layer_id: uid(3),
                document_id: uid(2),
                page_index: -1,
                name: "Fixture layer".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            }),
            Kind::CreateObject(pb::CreateObject {
                document_id: uid(2),
                state: Some(pb::ObjectState {
                    object_id: uid(4),
                    layer_id: uid(3),
                    order_key: "V".into(),
                    transform: Some(pb::Affine {
                        a: 1.0,
                        d: 1.0,
                        ..Default::default()
                    }),
                    style: Some(style(0x112233ff)),
                    role: pb::Role::None as i32,
                    created_by: owner().to_string(),
                    created_at_ms: CLOCK as i64,
                    shape: Some(Shape::Rect(pb::RectD {
                        x: 0.0,
                        y: 0.0,
                        w: 1.0,
                        h: 1.0,
                    })),
                    ..Default::default()
                }),
            }),
            Kind::AddResult(pb::AddResult {
                result_id: uid(5),
                document_id: uid(2),
                provider: "synthetic-provider".into(),
                model: "synthetic-model".into(),
                request_json: "{}".into(),
                ..Default::default()
            }),
        ],
    );
    host
}
fn reload(host: &HostSequencer) -> HostSequencer {
    let bytes = host.checkpoint_bytes().unwrap();
    let loaded = HostSequencer::from_checkpoint_bytes(&bytes);
    assert!(loaded.is_ok(), "checkpoint refused: {:?}", loaded.err());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.checkpoint_bytes().unwrap(), bytes);
    loaded
}

#[test]
fn a_checkpoint_with_a_write_that_kept_a_value_absent_loads_and_undoes() {
    let mut host = fixture();
    reload(&host);
    // Recolour the unfilled rectangle: its fill is absent before and after.
    submit(
        &mut host,
        2,
        vec![Kind::SetProperty(pb::SetProperty {
            object_id: uid(4),
            property: "style".into(),
            value: Some(pb::PropertyValue {
                value: Some(Value::Style(style(0x445566ff))),
            }),
        })],
    );
    // Reject the result: its acceptance mask is absent before and after.
    submit(
        &mut host,
        3,
        vec![Kind::UpdateResult(pb::UpdateResult {
            result_id: uid(5),
            status: "rejected".into(),
            acceptance_mask_asset_id: String::new(),
        })],
    );
    let mut loaded = reload(&host);
    // Undoing both from the loaded checkpoint gives what the original gives.
    for (n, target) in [(4, 3), (5, 2)] {
        let undo = vec![Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: uid(10_000 + target),
        })];
        let original = submit(&mut host, n, undo.clone());
        let restored = submit(&mut loaded, n, undo);
        assert_eq!(original, restored);
        assert_eq!(
            host.checkpoint_bytes().unwrap(),
            loaded.checkpoint_bytes().unwrap()
        );
    }
    let object = &host.project().objects[&id(4)];
    assert_eq!(object.state.style, Some(style(0x112233ff)));
    assert_eq!(host.project().results[&id(5)].status, "pending");
    reload(&host);
}

#[test]
fn an_entity_change_with_no_value_on_either_side_is_still_refused() {
    let mut host = fixture();
    submit(
        &mut host,
        2,
        vec![Kind::DeleteObject(pb::DeleteObject { object_id: uid(4) })],
    );
    let bytes = host.checkpoint_bytes().unwrap();
    // Blank the deleted object's recorded value and re-seal the checkpoint.
    let prefix = b"VisualWorkbench.HostCheckpoint.v1\n";
    let mut value: serde_json::Value = serde_json::from_slice(&bytes[prefix.len() + 32..]).unwrap();
    let mut blanked = 0;
    for record in value["records"].as_object_mut().unwrap().values_mut() {
        for change in record["changes"].as_array_mut().unwrap() {
            if change["address"]["path"].as_array().unwrap().is_empty() && change["after"].is_null()
            {
                change["before"] = serde_json::Value::Null;
                blanked += 1;
            }
        }
    }
    assert_eq!(blanked, 1);
    let payload = serde_json::to_vec(&value).unwrap();
    let mut tampered = prefix.to_vec();
    tampered.extend_from_slice(&AssetId::hash(&payload).bytes());
    tampered.extend_from_slice(&payload);
    assert!(HostSequencer::from_checkpoint_bytes(&tampered).is_err());
}
