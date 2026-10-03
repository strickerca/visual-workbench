#![allow(dead_code)]
use std::sync::Arc;
use vw_core::*;
use vw_model::{AssetId, DeviceId, Id};
use vw_raster::*;

pub const TIME: i64 = 1_700_000_000_000;
pub fn id(n: u8) -> String {
    Id::from_parts(TIME as u64, [n; 10]).unwrap().to_string()
}
pub fn device() -> String {
    DeviceId::from_bytes([7; 16]).to_string()
}
pub fn cancel() -> Arc<Cancellation> {
    Arc::new(Cancellation::new())
}
pub fn source() -> Vec<u8> {
    let image = DecodedImage {
        width: 64,
        height: 64,
        pixels: Pixels::Rgba8(vec![255; 64 * 64 * 4]),
        icc: None,
        source_asset: AssetId::hash(b"synthetic-ffi-fixture"),
        original_available: true,
        orientation_applied: 1,
    };
    vw_raster::export(
        &image,
        &ExportRequest {
            format: ExportFormat::Png8,
            region: None,
            revision: vw_proto::v1::Revision {
                host_seq: 0,
                state_hash: vec![0; 32],
            },
            alpha: AlphaPolicy::Preserve,
            color: ColorPolicy::Preserve,
            allow_depth_reduction: false,
            memory_budget_bytes: 256 * 1024 * 1024,
            capture_session: None,
            frame_id: None,
        },
    )
    .unwrap()
    .bytes
}
pub fn create(path: &std::path::Path) -> CreateImageProject {
    CreateImageProject {
        path: path.to_string_lossy().into(),
        project_id: id(1),
        document_id: id(2),
        layer_id: id(3),
        device_id: device(),
        title: "FFI deterministic fixture".into(),
        source: source(),
        now_ms: TIME,
    }
}
pub fn stroke() -> StrokeOptions {
    StrokeOptions {
        gesture_id: id(4),
        transaction_id: id(5),
        object_id: id(6),
        document_id: id(2),
        layer_id: id(3),
        device_id: device(),
        lamport: 1,
        created_at_ms: TIME + 1,
        family: "pen".into(),
        width: 4.0,
        rgba: 0x203040ff,
        stabilization: 0.0,
        pressure_curve: vec![],
    }
}
pub fn batch() -> SampleBatch {
    SampleBatch {
        sequence: 1,
        x: vec![10.0, 20.0, 30.0],
        y: vec![12.0, 24.0, 18.0],
        t_ms: vec![0, 8, 16],
        pressure: vec![0.5, 0.75, 1.0],
        tilt: vec![],
        orientation: vec![],
    }
}
pub fn options() -> ExportOptions {
    ExportOptions {
        document_id: id(2),
        format: vw_core::ImageFormat::Png8,
        marked: true,
        region: None,
        matte_rgb: None,
        convert_to_srgb: false,
        assume_untagged_srgb: true,
        allow_depth_reduction: false,
        memory_budget_bytes: 256 * 1024 * 1024,
    }
}
pub fn edit(n: u8, lamport: u64) -> EditOptions {
    EditOptions {
        transaction_id: id(n),
        document_id: id(2),
        device_id: device(),
        lamport,
        created_at_ms: TIME + i64::from(n),
    }
}
pub async fn draw(project: &Arc<ProjectSession>) -> ExportResult {
    let gesture = project.clone().begin_stroke(stroke()).await.unwrap();
    gesture.append_samples(batch()).await.unwrap();
    gesture.commit(cancel()).await.unwrap();
    project.export_image(options(), cancel()).await.unwrap()
}
