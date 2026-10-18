#![allow(clippy::unwrap_used)]
//! Run explicitly, in the centrally bounded native worker, after artifact/hash
//! verification. Ignoring this test in a normal suite is not PDF acceptance.
use pdfium_render::prelude::Pdfium;
use std::io::Write;
use vw_pdf::{
    PdfCancellation, PdfCopyKind, PdfError, PdfLimits, PdfOverlay, PdfPoint, PdfRuntime,
    PdfTileRequest,
};

fn fixture(rotation: u8) -> Vec<u8> {
    let content = b"q 1 0 0 rg 20 180 20 20 re f Q\nBT /F1 12 Tf 20 120 Td (SAFE TEXT) Tj ET\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 2 /Kids [3 0 R 6 0 R] >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /CropBox [10 20 110 220] /Rotate {} /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>", u32::from(rotation) * 90).into_bytes(),
        [format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(), content, b"endstream"].concat(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 32 32] /Resources << >> /Contents 7 0 R >>".to_vec(),
        b"<< /Length 0 >>\nstream\nendstream".to_vec(),
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
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
        objects.len() + 1
    )
    .unwrap();
    output
}

fn rectangle(page: u32) -> PdfOverlay {
    PdfOverlay {
        page,
        contours: vec![
            vec![
                PdfPoint { x: 2.0, y: 2.0 },
                PdfPoint { x: 28.0, y: 2.0 },
                PdfPoint { x: 28.0, y: 28.0 },
                PdfPoint { x: 2.0, y: 28.0 },
            ],
            vec![
                PdfPoint { x: 10.0, y: 10.0 },
                PdfPoint { x: 10.0, y: 20.0 },
                PdfPoint { x: 20.0, y: 20.0 },
                PdfPoint { x: 20.0, y: 10.0 },
            ],
        ],
        rgba: [255, 0, 0, 128],
        transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    }
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> &[u8] {
    let index = ((y * width + x) * 4) as usize;
    &pixels[index..index + 4]
}

#[test]
#[ignore = "requires verified PDFium 7881 and a centrally bounded isolated worker"]
fn verified_pdfium_worker_contract() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var_os("VW_PDFIUM_TEST_LIBRARY")
        .ok_or("explicit verified PDFium path required")?;
    let bindings = Pdfium::bind_to_library(std::path::Path::new(&path))
        .map_err(|_| "verified PDFium binding unavailable")?;
    // SAFETY: this ignored integration case is dispatched alone inside the
    // central runner's isolated, resource-capped process using its verified DLL.
    let mut runtime =
        unsafe { PdfRuntime::from_verified_bindings(bindings, PdfLimits::default()) }?;
    let cancel = PdfCancellation::default();
    for rotation in 0..4 {
        let original = fixture(rotation);
        let mut doc = runtime.open(original.clone(), None, &cancel)?;
        assert_eq!(doc.info().pages, 2);
        let info = doc.page_info(0, &cancel)?;
        let center = info
            .geometry
            .pdf_to_document(PdfPoint { x: 30.0, y: 190.0 })?;
        let tile = doc.render_tile(
            0,
            PdfTileRequest {
                x: center.x,
                y: center.y,
                pixels_per_point: 1.0,
                width: 1,
                height: 1,
                include_annotations: true,
            },
            &cancel,
        )?;
        assert_eq!(tile.rgba, [255, 0, 0, 255]);
        let text = doc.text_chunk(0, 0, 4096, &cancel)?;
        let mapped: String = text
            .glyphs
            .iter()
            .filter_map(|glyph| glyph.unicode)
            .collect();
        assert!(mapped.contains("SAFE TEXT"));
        assert!(text.glyphs.iter().any(|glyph| glyph.bounds.is_some()));
        let mut untouched = Vec::new();
        assert!(
            !doc.export(PdfCopyKind::Original, &[], &mut untouched, &cancel)?
                .redacted
        );
        assert_eq!(untouched, original);
        for kind in [PdfCopyKind::Annotated, PdfCopyKind::Flattened] {
            let mut copy = Vec::new();
            let receipt = doc.export(kind, &[rectangle(1)], &mut copy, &cancel)?;
            assert_eq!(receipt.bytes, copy.len() as u64);
            assert!(!receipt.redacted);
            // Release the immutable original before opening the private copy;
            // runtime lifetime statically disallows two public document owners.
            drop(doc);
            let mut reopened = runtime.open(copy, None, &cancel)?;
            assert_eq!(
                reopened.page_info(1, &cancel)?.annotations,
                if kind == PdfCopyKind::Annotated { 1 } else { 0 }
            );
            let tile = reopened.render_tile(
                1,
                PdfTileRequest {
                    x: 0.0,
                    y: 0.0,
                    pixels_per_point: 1.0,
                    width: 32,
                    height: 32,
                    include_annotations: true,
                },
                &cancel,
            )?;
            let filled = pixel(&tile.rgba, 32, 5, 5);
            assert_eq!(filled[0], 255);
            assert!((126..=128).contains(&filled[1]));
            assert_eq!(filled[1], filled[2]);
            assert_eq!(filled[3], 255);
            assert_eq!(pixel(&tile.rgba, 32, 15, 15), [255, 255, 255, 255]);
            drop(reopened);
            doc = runtime.open(original.clone(), None, &cancel)?;
        }
        assert!(matches!(
            doc.render_tile(
                2,
                PdfTileRequest {
                    x: 0.0,
                    y: 0.0,
                    pixels_per_point: 1.0,
                    width: 1,
                    height: 1,
                    include_annotations: false
                },
                &cancel
            ),
            Err(PdfError::Invalid)
        ));
        let interrupted = PdfCancellation::default();
        interrupted.cancel();
        let mut no_output = Vec::new();
        assert!(matches!(
            doc.export(PdfCopyKind::Flattened, &[], &mut no_output, &interrupted),
            Err(PdfError::Cancelled)
        ));
        assert!(no_output.is_empty());
    }
    assert!(matches!(
        runtime.open(b"%PDF-1.7\nnot a document".to_vec(), None, &cancel),
        Err(PdfError::Malformed)
    ));
    Ok(())
}
