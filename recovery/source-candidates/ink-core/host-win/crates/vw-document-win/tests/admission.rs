#![allow(clippy::unwrap_used)]
use std::time::Duration;
use vw_document_protocol::{CopyKind, JobLimits, Operation, Request, Tile};
use vw_document_win::{ArtifactEntry, DocumentFormat, Glyph, Notice, ResultMetadata, RuntimeManifest, WorkerLimits};

fn request(operation: Operation) -> Request { Request { operation, password: None,
    limits: JobLimits { input_bytes: 64 * 1024 * 1024, output_bytes: 64 * 1024 * 1024, memory_bytes: 256 * 1024 * 1024 } } }
fn entry(name: &str) -> ArtifactEntry { ArtifactEntry { file_name: name.to_owned(), bytes: 1, sha256: [1; 32] } }
fn runtime() -> RuntimeManifest { RuntimeManifest { entries: vec![entry("vw-document-worker.exe"), entry("pdfium.dll")] } }

#[test]
fn executable_and_pdfium_are_exact_required_flat_manifest_names() {
    runtime().validate().unwrap();
    for name in ["other.exe", "../pdfium.dll", "c:pdfium.dll", "folder/pdfium.dll", "PDFIUM.dll", "con.dll", "pdfium..dll", "x.dll:secret"] {
        let mut value = runtime(); value.entries[1].file_name = name.to_owned(); assert!(value.validate().is_err(), "name boundary");
    }
}
#[test]
fn artifact_inventory_rejects_duplicates_unknown_types_and_missing_hashes() {
    let mut value = runtime(); value.entries.push(entry("pdfium.dll")); assert!(value.validate().is_err());
    value = runtime(); value.entries[0].sha256 = [0; 32]; assert!(value.validate().is_err());
    value = runtime(); value.entries.push(entry("notes.txt")); assert!(value.validate().is_err());
}
#[test]
fn total_runtime_and_per_file_bytes_are_admitted_without_reading_any_artifact() {
    let mut value = runtime(); value.entries[0].bytes = 256 * 1024 * 1024 + 1; assert!(value.validate().is_err());
    value = runtime(); for entry in &mut value.entries { entry.bytes = 256 * 1024 * 1024; }
    value.entries.push(entry("dependency.dll")); assert!(value.validate().is_err());
}
#[test]
fn job_memory_is_exact_and_deadlines_have_hard_admission_bounds() {
    let request = request(Operation::PdfInfo);
    WorkerLimits { wall_time: Duration::from_secs(120), process_memory_bytes: request.limits.memory_bytes }.validate(&request).unwrap();
    for seconds in [0, 121] { assert!(WorkerLimits { wall_time: Duration::from_secs(seconds), process_memory_bytes: request.limits.memory_bytes }.validate(&request).is_err()); }
    assert!(WorkerLimits { wall_time: Duration::from_secs(10), process_memory_bytes: request.limits.memory_bytes + 1 }.validate(&request).is_err());
}
#[test]
fn rgba_metadata_is_bound_to_exact_page_pose_dimensions_and_native_scale() {
    let request = request(Operation::PdfTile { page: 2, tile: Tile { x: 5., y: 6., scale: 1.2, width: 12, height: 13 }, annotations: true });
    let value = ResultMetadata::RgbaTile { format: DocumentFormat::Pdf, page: Some(2), origin: [5., 6.], requested_scale: 1.2,
        native_scale: f64::from(1.2_f32), width: 12, height: 13, opaque_paper: true };
    value.validate_for(&request).unwrap(); assert_eq!(value.data_bytes(), 12 * 13 * 4);
    let mut wrong = value.clone(); if let ResultMetadata::RgbaTile { page, .. } = &mut wrong { *page = Some(3); } assert!(wrong.validate_for(&request).is_err());
    let mut wrong = value; if let ResultMetadata::RgbaTile { native_scale, .. } = &mut wrong { *native_scale = 1.2; } assert!(wrong.validate_for(&request).is_err());
}
#[test]
fn successful_normal_copy_can_never_claim_redaction() {
    let request = request(Operation::PdfCopy { copy: CopyKind::Flattened, overlays: vec![] });
    let value = ResultMetadata::PdfCopy { copy: CopyKind::Flattened, pages: 1, bytes: 100, redacted: true };
    assert!(value.validate_for(&request).is_err());
}
#[test]
fn glyph_indices_and_next_cursor_must_match_the_requested_chunk() {
    let request = request(Operation::PdfText { page: 0, start: 12, count: 2 });
    let mut value = ResultMetadata::PdfText { page: 0, start: 12, total_characters: 20, next: Some(14), glyphs: vec![
        Glyph { index: 12, unicode: Some('A'), bounds: None, generated: false }, Glyph { index: 13, unicode: None, bounds: None, generated: true }] };
    value.validate_for(&request).unwrap();
    if let ResultMetadata::PdfText { next, .. } = &mut value { *next = Some(15); }
    assert!(value.validate_for(&request).is_err());
}
#[test]
fn selected_glyphs_must_have_geometry_intersecting_the_selected_area() {
    let request = request(Operation::PdfSelection { page: 0, rectangle: [0., 0., 10., 10.] });
    let mut value = ResultMetadata::PdfSelection { page: 0, glyphs: vec![Glyph { index: 0, unicode: Some('S'), bounds: Some([1., 1., 2., 2.]), generated: false }] };
    value.validate_for(&request).unwrap();
    if let ResultMetadata::PdfSelection { glyphs, .. } = &mut value { glyphs[0].bounds = Some([11., 1., 12., 2.]); }
    assert!(value.validate_for(&request).is_err());
}
#[test]
fn metadata_refuses_unknown_duplicate_or_wrong_operation_records() {
    let request = request(Operation::SvgInfo);
    for bytes in [br#"{"kind":"svg_info","width":1,"height":1,"notices":[],"secret":"x"}"#.as_slice(),
        br#"{"kind":"svg_info","width":1,"width":2,"height":1,"notices":[]}"#] {
        assert!(ResultMetadata::parse_for(bytes, &request).is_err());
    }
    assert!(ResultMetadata::PdfInfo { pages: 1, encrypted: false, notices: vec![] }.validate_for(&request).is_err());
}
#[test]
fn notices_cannot_silently_change_format_or_repeat() {
    let request = request(Operation::SvgInfo);
    for notices in [vec![Notice::SystemFontsDisabled], vec![Notice::ResourceDenied, Notice::ResourceDenied]] {
        assert!(ResultMetadata::SvgInfo { width: 1., height: 1., notices }.validate_for(&request).is_err());
    }
}
#[test]
fn result_debug_omits_extracted_document_text() {
    let value = ResultMetadata::PdfText { page: 0, start: 0, total_characters: 1, next: None,
        glyphs: vec![Glyph { index: 0, unicode: Some('秘'), bounds: None, generated: false }] };
    assert!(!format!("{value:?}").contains('秘'));
}

#[test]
fn malformed_tile_sizes_have_a_non_panicking_refusal_size_even_before_validation() {
    let value = ResultMetadata::RgbaTile { format: DocumentFormat::Svg, page: None, origin: [0., 0.], requested_scale: 1.,
        native_scale: 1., width: u32::MAX, height: u32::MAX, opaque_paper: false };
    assert_eq!(value.data_bytes(), u64::MAX);
    assert!(!format!("{value:?}").is_empty());
    assert!(value.validate_for(&request(Operation::SvgTile { tile: Tile { x: 0., y: 0., scale: 1., width: 1, height: 1 } })).is_err());
}
