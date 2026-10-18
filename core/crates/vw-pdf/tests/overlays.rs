#![allow(clippy::unwrap_used)]
use vw_pdf::{PdfCancellation, PdfError, PdfOverlay, PdfPoint, validate_overlays};
fn triangle() -> PdfOverlay {
    PdfOverlay {
        page: 0,
        contours: vec![vec![
            PdfPoint { x: 0.0, y: 0.0 },
            PdfPoint { x: 10.0, y: 0.0 },
            PdfPoint { x: 0.0, y: 10.0 },
        ]],
        rgba: [1, 2, 3, 128],
        transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    }
}

#[test]
fn compound_hole_and_highlighter_stay_one_paint_object() {
    let mut path = triangle();
    path.contours.push(vec![
        PdfPoint { x: 1.0, y: 1.0 },
        PdfPoint { x: 1.0, y: 2.0 },
        PdfPoint { x: 2.0, y: 1.0 },
    ]);
    let before = path.clone();
    assert!(validate_overlays(std::slice::from_ref(&path), 1, &PdfCancellation::default()).is_ok());
    assert_eq!(path, before);
    assert_eq!(path.rgba[3], 128);
}

#[test]
fn affine_mapping_is_d_space_and_rejects_singular_or_nonfinite_input() {
    let mut path = triangle();
    path.transform = [0.0, 2.0, -3.0, 0.0, 7.0, 11.0];
    assert_eq!(
        path.document_point(PdfPoint { x: 2.0, y: 5.0 }).unwrap(),
        PdfPoint { x: -8.0, y: 15.0 }
    );
    path.transform = [1.0, 2.0, 2.0, 4.0, 0.0, 0.0];
    assert!(matches!(
        validate_overlays(std::slice::from_ref(&path), 1, &PdfCancellation::default()),
        Err(PdfError::Invalid)
    ));
    path.transform[0] = f64::NAN;
    assert!(validate_overlays(&[path], 1, &PdfCancellation::default()).is_err());
}

#[test]
fn outer_and_inner_headers_are_bounded_before_point_traversal() {
    let cancel = PdfCancellation::default();
    assert!(matches!(
        validate_overlays(&vec![triangle(); 4097], 1, &cancel),
        Err(PdfError::Limit)
    ));
    let mut malformed = triangle();
    malformed.contours = vec![Vec::new(); 16_385];
    assert!(matches!(
        validate_overlays(&[malformed], 1, &cancel),
        Err(PdfError::Limit)
    ));
}

#[test]
fn point_capacity_is_charged_even_for_a_small_visible_triangle() {
    let regular = triangle();
    let expected = validate_overlays(
        std::slice::from_ref(&regular),
        1,
        &PdfCancellation::default(),
    )
    .unwrap();
    let mut reserved = regular;
    reserved.contours[0].reserve_exact(1000);
    let actual = validate_overlays(&[reserved], 1, &PdfCancellation::default()).unwrap();
    assert!(actual >= expected + 1000 * std::mem::size_of::<PdfPoint>() as u64);
}

#[test]
fn cancellation_and_missing_pages_refuse_before_mutation() {
    let cancel = PdfCancellation::default();
    cancel.cancel();
    assert!(matches!(
        validate_overlays(&[triangle()], 1, &cancel),
        Err(PdfError::Cancelled)
    ));
    assert!(matches!(
        validate_overlays(&[triangle()], 0, &PdfCancellation::default()),
        Err(PdfError::Invalid)
    ));
}
