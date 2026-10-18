#![allow(clippy::unwrap_used)]
use vw_pdf::{PdfLimits, RasterRedactionError, admit_raster_redaction};

#[test]
fn whole_document_is_charged_even_if_only_one_page_has_a_region() {
    let work = admit_raster_redaction(&[[64.0, 32.0], [64.0, 32.0]], 2.0, 1, PdfLimits::default())
        .unwrap();
    assert_eq!(work.pages, 2);
    assert_eq!(work.pixels, 2 * 128 * 64);
    assert_eq!(work.tiles, 2);
    assert_eq!(work.pdf_objects, 8);
}

#[test]
fn output_must_fit_reopen_input_before_rendering() {
    let limits = PdfLimits {
        input_bytes: 1024,
        output_bytes: 1024 * 1024,
        ..PdfLimits::default()
    };
    assert!(matches!(
        admit_raster_redaction(&[[10.0, 10.0]], 1.0, 1, limits),
        Err(RasterRedactionError::Limit)
    ));
}

#[test]
fn resolution_is_not_reduced_to_pass_a_work_limit() {
    assert!(matches!(
        admit_raster_redaction(&[[10_000.0, 10_000.0]], 4.0, 1, PdfLimits::default()),
        Err(RasterRedactionError::Limit)
    ));
    let limits = PdfLimits {
        input_bytes: 256 * 1024 * 1024,
        ..PdfLimits::default()
    };
    assert!(admit_raster_redaction(&[[1000.0, 1000.0]], 4.0, 1, limits).is_ok());
}

#[test]
fn image_object_limit_is_checked_before_output_that_could_not_verify() {
    let limits = PdfLimits {
        objects_per_page: 1,
        ..PdfLimits::default()
    };
    assert!(matches!(
        admit_raster_redaction(&[[513.0, 10.0]], 1.0, 1, limits),
        Err(RasterRedactionError::Limit)
    ));
}

#[test]
fn all_hostile_scale_and_dimension_boundaries_refuse() {
    for scale in [0.0, -1.0, f64::NAN, f64::INFINITY, 8.0001] {
        assert!(admit_raster_redaction(&[[10.0, 10.0]], scale, 1, PdfLimits::default()).is_err());
    }
    for size in [
        [0.0, 10.0],
        [-1.0, 10.0],
        [10.0, f64::NAN],
        [65_536.0, 10.0],
    ] {
        assert!(admit_raster_redaction(&[size], 1.0, 1, PdfLimits::default()).is_err());
    }
}

#[test]
fn empty_or_excessive_region_headers_refuse_without_allocating_masks() {
    for count in [0, 4097, usize::MAX] {
        assert!(matches!(
            admit_raster_redaction(&[[10.0, 10.0]], 1.0, count, PdfLimits::default()),
            Err(RasterRedactionError::Invalid)
        ));
    }
    assert!(admit_raster_redaction(&[], 1.0, 1, PdfLimits::default()).is_err());
}

#[test]
fn page_dimension_admission_matches_the_effective_native_scale() {
    // The requested double product is exactly6, whereas the actual FS_MATRIX
    // coefficient is1.2000000476837158 and requires a seventh edge sample.
    let work = admit_raster_redaction(&[[5.0, 5.0]], 1.2, 1, PdfLimits::default()).unwrap();
    assert_eq!(work.pixels, 49);
    assert_eq!(work.tiles, 1);
}
