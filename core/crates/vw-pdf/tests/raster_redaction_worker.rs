#![allow(clippy::unwrap_used)]
//! Explicit verified/capped-worker contract, never a silent fallback unit test.
use pdfium_render::prelude::Pdfium;
use std::io::Write;
use vw_pdf::{
    PageRedaction, PdfCancellation, PdfLimits, PdfRect, PdfRuntime, PdfTileRequest,
    RasterRedactionOptions, RasterRedactionPlan, RasterSecurityPolicy,
};

fn synthetic_sensitive_pdf() -> Vec<u8> {
    fn stream(bytes: &[u8]) -> Vec<u8> {
        [
            format!("<< /Length {} >>\nstream\n", bytes.len()).as_bytes(),
            bytes,
            b"\nendstream",
        ]
        .concat()
    }
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Names [(private.txt) 9 0 R] >> >> >>".to_vec(),
        b"<< /Type /Pages /Count 2 /Kids [3 0 R 6 0 R] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 64 64] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream(b"BT /F1 8 Tf 5 40 Td (SECRET ORIGINAL) Tj ET\n"),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 64 64] /Resources << /Font << /F1 5 0 R >> >> /Contents 7 0 R >>".to_vec(),
        stream(b"BT /F1 8 Tf 5 40 Td (VISIBLE OUTSIDE) Tj ET\n"),
        b"<< /Author (SECRET METADATA) >>".to_vec(),
        b"<< /Type /Filespec /F (private.txt) /EF << /F 10 0 R >> >>".to_vec(),
        stream(b"SECRET ATTACHMENT ORIGINAL"),
    ];
    let mut output = b"%PDF-1.7\n%synthetic\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        writeln!(output, "{} 0 obj", index + 1).unwrap();
        output.extend_from_slice(object);
        output.extend_from_slice(b"\nendobj\n");
    }
    let xref = output.len();
    writeln!(output, "xref\n0 {}\n0000000000 65535 f ", objects.len() + 1).unwrap();
    for offset in offsets {
        writeln!(output, "{offset:010} 00000 n ").unwrap();
    }
    writeln!(
        output,
        "trailer\n<< /Size {} /Root 1 0 R /Info 8 0 R >>\nstartxref\n{xref}\n%%EOF",
        objects.len() + 1
    )
    .unwrap();
    output
}

#[test]
#[ignore = "requires verified PDFium 7881 in a centrally capped isolated process"]
fn phone_copy_reconstructs_every_page_and_verifies_no_text_or_original_resources()
-> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var_os("VW_PDFIUM_TEST_LIBRARY")
        .ok_or("explicit verified PDFium path required")?;
    let bindings = Pdfium::bind_to_library(std::path::Path::new(&path))
        .map_err(|_| "verified PDFium binding unavailable")?;
    // SAFETY: central runner dispatches this ignored case alone after the native
    // artifact/ABI and OS process memory/deadline/file-access gates have passed.
    let mut runtime =
        unsafe { PdfRuntime::from_verified_bindings(bindings, PdfLimits::default()) }?;
    let cancel = PdfCancellation::default();
    let mut source = runtime.open(synthetic_sensitive_pdf(), None, &cancel)?;
    assert!(source.text_chunk(0, 0, 4096, &cancel)?.total_characters > 0);
    assert!(source.text_chunk(1, 0, 4096, &cancel)?.total_characters > 0);
    let plan = RasterRedactionPlan::prepare(
        &mut source,
        vec![PageRedaction {
            page: 0,
            region: PdfRect {
                left: 0.0,
                top: 0.0,
                right: 64.0,
                bottom: 64.0,
            },
        }],
        RasterRedactionOptions {
            pixels_per_point: 1.0,
            security: RasterSecurityPolicy::RefuseEncrypted,
        },
        &cancel,
    )?;
    let mut output = Vec::new();
    let mut last = None;
    let pending = plan.write_unverified(&mut source, &mut output, &cancel, &mut |progress| {
        last = Some(progress)
    })?;
    let mut tampered = Vec::new();
    let wrong_receipt = plan.write_unverified(&mut source, &mut tampered, &cancel, &mut |_| {})?;
    tampered.extend_from_slice(b"\n% changed after private completion\n");
    assert_eq!(last.unwrap().pages_complete, 2);
    for forbidden in [
        b"SECRET ORIGINAL".as_slice(),
        b"SECRET METADATA",
        b"SECRET ATTACHMENT",
        b"/EmbeddedFiles",
        b"/Font",
        b"/Info",
        b"/Prev",
    ] {
        assert!(
            !output
                .windows(forbidden.len())
                .any(|window| window == forbidden)
        );
    }
    drop(source);
    let mut changed = runtime.open(tampered, None, &cancel)?;
    assert!(plan.verify(&mut changed, wrong_receipt, &cancel).is_err());
    drop(changed);
    let mut reconstructed = runtime.open(output, None, &cancel)?;
    let receipt = plan.verify(&mut reconstructed, pending, &cancel)?;
    assert_eq!(receipt.pages(), 2);
    assert_eq!(
        reconstructed
            .text_chunk(0, 0, 4096, &cancel)?
            .total_characters,
        0
    );
    assert_eq!(
        reconstructed
            .text_chunk(1, 0, 4096, &cancel)?
            .total_characters,
        0
    );
    let tile = reconstructed.render_tile(
        0,
        PdfTileRequest {
            x: 0.0,
            y: 0.0,
            pixels_per_point: 1.0,
            width: 64,
            height: 64,
            include_annotations: true,
        },
        &cancel,
    )?;
    assert!(
        tile.rgba
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 0, 0, 255])
    );
    let outside = reconstructed.render_tile(
        1,
        PdfTileRequest {
            x: 0.0,
            y: 0.0,
            pixels_per_point: 1.0,
            width: 64,
            height: 64,
            include_annotations: true,
        },
        &cancel,
    )?;
    assert!(outside.rgba.chunks_exact(4).any(|pixel| pixel[0] < 200));
    Ok(())
}
