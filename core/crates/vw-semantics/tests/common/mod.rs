#![allow(dead_code)]
use vw_model::{DeviceId, Document, Id, Project};
use vw_ops::HostSequencer;
use vw_proto::v1;
use vw_semantics::*;
pub type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
pub fn id(n: u64) -> std::result::Result<Id, vw_model::ModelError> {
    Id::from_parts(2000 + n, [3; 10])
}
pub fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
pub fn fixture() -> std::result::Result<HostSequencer, Box<dyn std::error::Error>> {
    let mut project = Project::new(id(1)?, "Semantic fixture".into(), device(1));
    project.documents.insert(
        id(2)?,
        Document {
            definition: v1::CreateDocument {
                document_id: Some(id(2)?.to_proto()),
                kind: v1::DocumentKind::Capture as i32,
                schema_version: 1,
                title: "Capture".into(),
                capture: Some(v1::CaptureInfo {
                    capture_session_id: Some(id(3)?.to_proto()),
                    frame_id: u64::MAX,
                    geometry: Some(v1::CaptureGeometry {
                        source_kind: "window".into(),
                        window_handle: 777,
                        monitor_id: "private-monitor-fixture".into(),
                        client_rect_host: Some(v1::RectI {
                            x: -1920,
                            y: -100,
                            w: 800,
                            h: 600,
                        }),
                        dpi_scale: 1.5,
                        geometry_revision: 7,
                        timestamp_ns: 8000,
                    }),
                    platform: "windows".into(),
                    app_name: "Fixture".into(),
                    window_title: "private-title-fixture".into(),
                    lossless: true,
                    degraded: false,
                    captured_at_ms: 1000,
                }),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 1000,
        },
    );
    Ok(HostSequencer::new(project, device(1))?)
}
pub fn view(
    host: &HostSequencer,
) -> std::result::Result<CaptureView<'_>, Box<dyn std::error::Error>> {
    Ok(CaptureView::new(
        host.project(),
        &host.revision()?,
        &id(2)?,
    )?)
}
pub fn element(local: &str, parent: Option<&str>, bounds: [f64; 4]) -> CapturedElement {
    CapturedElement {
        local_id: local.into(),
        parent_local_id: parent.map(str::to_owned),
        name: format!("Name {local}"),
        role: "button".into(),
        automation_id: Some(format!("automation-{local}")),
        resource_id: None,
        html_id: None,
        bounds,
        text: "literal captured text".into(),
        enabled: true,
        focused: false,
    }
}
pub fn elements() -> Vec<CapturedElement> {
    vec![
        element("root", None, [0.0, 0.0, 800.0, 600.0]),
        element("button", Some("root"), [50.0, 60.0, 100.0, 40.0]),
    ]
}
pub fn prepare(view: &CaptureView<'_>, n: u64, raw: Vec<CapturedElement>) -> Result<Snapshot> {
    view.prepare(
        view.context(),
        id(n)?,
        Platform::Uia,
        -7,
        250,
        BoundsSpace::CapturePixels,
        raw,
        &NeverCancel,
    )
}
pub fn meta(n: u64) -> Result<EditMetadata> {
    Ok(EditMetadata {
        transaction_id: id(10_000 + n)?,
        device: device(1),
        lamport: n,
        created_at_ms: 2000,
    })
}
pub fn stored() -> std::result::Result<HostSequencer, Box<dyn std::error::Error>> {
    let mut host = fixture()?;
    let view = view(&host)?;
    let snapshot = prepare(&view, 10, elements())?;
    let plan = view.plan(&snapshot, meta(1)?, &NeverCancel)?;
    plan.submit(&mut host, &device(1), 2100)?;
    Ok(host)
}
pub fn payload(project: &Project) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(project
        .semantic_snapshots
        .get(&id(10)?)
        .ok_or("snapshot")?
        .definition
        .elements_json_zstd
        .clone())
}
pub fn with_payload(
    mut project: Project,
    bytes: Vec<u8>,
) -> std::result::Result<Project, Box<dyn std::error::Error>> {
    project
        .semantic_snapshots
        .get_mut(&id(10)?)
        .ok_or("snapshot")?
        .definition
        .elements_json_zstd = bytes;
    Ok(project)
}
pub fn load(project: &Project) -> Result<Snapshot> {
    CaptureView::new(
        project,
        &v1::Revision {
            host_seq: 1,
            state_hash: project.state_hash()?.bytes().to_vec(),
        },
        &id(2)?,
    )?
    .load(&id(10)?, &NeverCancel)
}
