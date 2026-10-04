#![allow(dead_code)]
use vw_instructions::*;
use vw_model::{DeviceId, Document, Id, Layer, Project};
use vw_ops::HostSequencer;
use vw_proto::v1::{self, op::Kind};
pub type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
pub fn id(n: u64) -> std::result::Result<Id, vw_model::ModelError> {
    Id::from_parts(1000 + n, [7; 10])
}
pub fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
pub fn fixture() -> std::result::Result<HostSequencer, Box<dyn std::error::Error>> {
    let mut project = Project::new(id(1)?, "Instruction fixture".into(), device(1));
    project.documents.insert(
        id(2)?,
        Document {
            definition: v1::CreateDocument {
                document_id: Some(id(2)?.to_proto()),
                kind: v1::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Canvas".into(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 1000,
        },
    );
    project.layers.insert(
        id(3)?,
        Layer {
            definition: v1::CreateLayer {
                layer_id: Some(id(3)?.to_proto()),
                document_id: Some(id(2)?.to_proto()),
                page_index: -1,
                name: "Marks".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    Ok(HostSequencer::new(project, device(1))?)
}
pub fn meta(n: u64) -> std::result::Result<EditMetadata, vw_model::ModelError> {
    Ok(EditMetadata {
        transaction_id: id(10_000 + n)?,
        device: device(1),
        first_lamport: n * 1000 + 1,
        created_at_ms: 2000 + n as i64,
    })
}
pub fn placement(n: u64) -> std::result::Result<MarkerPlacement, vw_model::ModelError> {
    Ok(MarkerPlacement {
        object_id: id(10 + n)?,
        instruction_id: id(100 + n)?,
        layer_id: id(3)?,
        point: v1::PointD { x: 1.2, y: 2.8 },
        bounds: Some(v1::RectD {
            x: 0.2,
            y: 1.8,
            w: 9.1,
            h: 7.1,
        }),
        element_eids: vec!["fixture-element".into()],
        style: v1::Style {
            stroke: Some(v1::Color { rgba: 0x112233ff }),
            width: 2.0,
            ..Default::default()
        },
        role: Role::Change,
        text: format!("Instruction {n}"),
        entry_method: EntryMethod::PhoneKeyboard,
        language: "en-US".into(),
    })
}
pub fn place(
    host: &mut HostSequencer,
    n: u64,
) -> std::result::Result<EditPlan, Box<dyn std::error::Error>> {
    let view = DocumentView::new(host.project(), &host.revision()?, &id(2)?)?;
    let plan = view.place_marker(meta(n)?, placement(n)?)?;
    plan.submit(host, &device(1), 3000)?;
    Ok(plan)
}
pub fn edit(
    view: &DocumentView<'_>,
    instruction: &Id,
    value: &str,
    method: EntryMethod,
) -> Result<InstructionEdit> {
    let old = &view.instruction(instruction)?.definition;
    Ok(InstructionEdit {
        instruction_id: instruction.clone(),
        targets: old
            .target_object_ids
            .iter()
            .map(|v| Id::from_proto(Some(v)))
            .collect::<std::result::Result<_, _>>()?,
        role: Role::from_canonical(old.role)?,
        text: value.into(),
        entry_method: method,
        language: old.language.clone(),
    })
}
pub fn raw(host: &mut HostSequencer, n: u64, kinds: Vec<Kind>) -> TestResult {
    let meta = meta(n)?;
    let txn = v1::Transaction {
        txn_id: Some(meta.transaction_id.to_proto()),
        project_id: Some(host.project().id.to_proto()),
        device_id: meta.device.to_string(),
        base_revision: Some(host.revision()?),
        created_at_wall_ms: meta.created_at_ms,
        gesture_id: None,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| v1::Op {
                op_id: Some(v1::OpId {
                    device_id: meta.device.to_string(),
                    lamport: meta.first_lamport + i as u64,
                }),
                kind: Some(kind),
            })
            .collect(),
    };
    host.submit(txn, &device(1), 4000)?;
    Ok(())
}
pub fn view(
    host: &HostSequencer,
) -> std::result::Result<DocumentView<'_>, Box<dyn std::error::Error>> {
    Ok(DocumentView::new(
        host.project(),
        &host.revision()?,
        &id(2)?,
    )?)
}
