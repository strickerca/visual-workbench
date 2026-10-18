#![allow(clippy::unwrap_used)]
//! Real source-only contract, explicitly gated on verified LPAC runtime artifacts.
#![cfg(windows)]
use pdfium_render::prelude::Pdfium;
use std::io::Write;
use vw_pdf::{
    PageRedaction, PcRedactionOptions, PcRedactionPlan, PdfCancellation, PdfLimits, PdfRect,
    PdfRuntime, PdfTileRequest, QpdfRuntime, RasterRedactionOptions, RasterRedactionPlan,
    RasterSecurityPolicy,
};

fn sensitive_pdf(extra_text: bool) -> Vec<u8> {
    fn stream(bytes: &[u8]) -> Vec<u8> {
        [
            format!("<< /Length {} >>\nstream\n", bytes.len()).as_bytes(),
            bytes,
            b"\nendstream",
        ]
        .concat()
    }
    let second = if extra_text {
        b"0 1 0 rg 1700 100 40 30 re f BT /F1 9 Tf 1700 400 Td (UNSUPPORTED OUTSIDE TEXT) Tj ET\n"
            .as_slice()
    } else {
        b"0 1 0 rg 1700 100 40 30 re f\n".as_slice()
    };
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Names [(private.txt) 9 0 R] >> >> >>".to_vec(),
        b"<< /Type /Pages /Count 2 /Kids [3 0 R 6 0 R] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 2048 512] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream(b"BT /F1 9 Tf 20 400 Td (SECRET ORIGINAL) Tj ET 1 0 0 rg 1700 400 30 30 re f\n"),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 2048 512] /Resources << /Font << /F1 5 0 R >> >> /Contents 7 0 R >>".to_vec(),
        stream(second), b"<< /Author (SECRET METADATA) >>".to_vec(),
        b"<< /Type /Filespec /F (private.txt) /EF << /F 10 0 R >> >>".to_vec(), stream(b"SECRET ATTACHMENT ORIGINAL"),
    ];
    let mut bytes = b"%PDF-1.7\n%synthetic\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        writeln!(bytes, "{} 0 obj", index + 1).unwrap();
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    writeln!(bytes, "xref\n0 {}\n0000000000 65535 f ", objects.len() + 1).unwrap();
    for offset in offsets {
        writeln!(bytes, "{offset:010} 00000 n ").unwrap();
    }
    writeln!(
        bytes,
        "trailer\n<< /Size {} /Root 1 0 R /Info 8 0 R >>\nstartxref\n{xref}\n%%EOF",
        objects.len() + 1
    )
    .unwrap();
    bytes
}
fn regions() -> Vec<PageRedaction> {
    vec![PageRedaction {
        page: 0,
        region: PdfRect {
            left: 18.0,
            top: 100.0,
            right: 150.0,
            bottom: 120.0,
        },
    }]
}
#[test]
#[ignore = "requires separately verified qpdf12.4.2/PDFium7881 pins and centrally capped LPAC process"]
fn pc_copy_removes_covered_text_preserves_unaffected_polygons_and_qpdf_verifies_every_resource()
-> Result<(), Box<dyn std::error::Error>> {
    let pdf_path =
        std::env::var_os("VW_PDFIUM_TEST_LIBRARY").ok_or("verified PDFium path required")?;
    let qpdf_path =
        std::env::var_os("VW_QPDF_TEST_LIBRARY").ok_or("verified qpdf path required")?;
    let limits = PdfLimits::default();
    let cancel = PdfCancellation::default();
    // SAFETY: central runner explicitly isolates this ignored test alone and
    // retains approved artifact/ancestor pins until all native owners retire.
    let cleanup =
        unsafe { QpdfRuntime::from_verified_library(std::path::Path::new(&qpdf_path), limits) }?;
    let original = sensitive_pdf(false);
    let original_hash = blake3::hash(&original);
    let admission = cleanup.admit_source(&original, None, &cancel)?;
    let bindings = Pdfium::bind_to_library(std::path::Path::new(&pdf_path))
        .map_err(|_| "verified PDFium binding unavailable")?;
    // SAFETY: same centrally verified exclusive ABI7881/OS worker contract.
    let mut runtime = unsafe { PdfRuntime::from_verified_bindings(bindings, limits) }?;
    let mut source = runtime.open(original, None, &cancel)?;
    let plan = PcRedactionPlan::prepare(
        &mut source,
        admission,
        regions(),
        PcRedactionOptions {
            pixels_per_point: 1.0,
            security: RasterSecurityPolicy::RefuseEncrypted,
        },
        &cancel,
    )?;
    assert_eq!(plan.work().preserved_paths, 2);
    assert_eq!(plan.work().removed_objects, 1);
    assert_eq!(plan.work().tiles, 1);
    let mut fresh = Vec::new();
    let intermediate = plan.write_unverified(&mut source, &mut fresh, &cancel, &mut |_| {})?;
    assert_eq!(
        source.info().source_blake3,
        original_hash.to_hex().to_string()
    );
    drop(source);
    let (mut output, cleaned) = cleanup.rewrite_and_audit(&plan, intermediate, &fresh, &cancel)?;
    for forbidden in [
        b"SECRET ORIGINAL".as_slice(),
        b"SECRET METADATA",
        b"SECRET ATTACHMENT",
        b"/Font",
        b"/EmbeddedFiles",
        b"/Info",
        b"/Prev",
    ] {
        assert!(
            !output
                .windows(forbidden.len())
                .any(|part| part == forbidden)
        );
    }
    let metadata = format!(
        r#"{{"schema_version":1,"host_seq":42,"state_hash":"{}","revision":"r42-aaaaaaaa","source_asset":"{}"}}"#,
        "a".repeat(64),
        original_hash.to_hex()
    );
    let cleaned = cleaned.embed_metadata(&mut output, &metadata, limits, &cancel)?;
    vw_pdf::verify_pdf_copy_metadata(&output, &metadata, limits.output_bytes, &cancel)?;
    let mut generated = runtime.open(output, None, &cancel)?;
    generated.verify_export_metadata(&metadata, &cancel)?;
    let verified = plan.verify(&mut generated, cleaned, &cancel)?;
    assert_eq!(verified.work().preserved_paths, 2);
    assert_eq!(
        generated.text_chunk(0, 0, 4096, &cancel)?.total_characters,
        0
    );
    assert_eq!(
        generated.text_chunk(1, 0, 4096, &cancel)?.total_characters,
        0
    );
    let tile = generated.render_tile(
        0,
        PdfTileRequest {
            x: 0.0,
            y: 0.0,
            pixels_per_point: 1.0,
            width: 160,
            height: 128,
            include_annotations: false,
        },
        &cancel,
    )?;
    for y in 100..120 {
        for x in 18..150 {
            let start = ((y * 160 + x) * 4) as usize;
            assert_eq!(&tile.rgba[start..start + 4], &[0, 0, 0, 255]);
        }
    }
    drop(generated);
    // A page with unrelated text visibly refuses the PC subset. The full-page
    // alternative is separately selected, never silently substituted.
    let original = sensitive_pdf(true);
    let admission = cleanup.admit_source(&original, None, &cancel)?;
    let mut source = runtime.open(original, None, &cancel)?;
    assert!(
        PcRedactionPlan::prepare(
            &mut source,
            admission,
            regions(),
            PcRedactionOptions {
                pixels_per_point: 1.0,
                security: RasterSecurityPolicy::RefuseEncrypted
            },
            &cancel
        )
        .is_err()
    );
    assert!(
        RasterRedactionPlan::prepare(
            &mut source,
            regions(),
            RasterRedactionOptions {
                pixels_per_point: 1.0,
                security: RasterSecurityPolicy::RefuseEncrypted
            },
            &cancel
        )
        .is_ok()
    );
    Ok(())
}
