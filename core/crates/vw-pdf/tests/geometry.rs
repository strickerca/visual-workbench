#![allow(clippy::unwrap_used)]
use vw_pdf::{PageGeometry, PdfError, PdfLimits, PdfPoint, PdfRect, PdfTileRequest};

#[test]
fn all_four_crop_rotation_corners_have_independent_known_answers() {
    let raw = PdfPoint { x: 40.0, y: 80.0 };
    let expected = [(30.0, 140.0), (60.0, 30.0), (70.0, 60.0), (140.0, 70.0)];
    for (rotation, (x, y)) in expected.into_iter().enumerate() {
        let page = PageGeometry::new(
            [10.0, 20.0, 110.0, 220.0],
            rotation as u8,
            PdfLimits::default(),
        )
        .unwrap();
        assert_eq!(page.pdf_to_document(raw).unwrap(), PdfPoint { x, y });
        assert_eq!(page.document_to_pdf(PdfPoint { x, y }).unwrap(), raw);
        assert_eq!(
            (page.width(), page.height()),
            if rotation % 2 == 0 {
                (100.0, 200.0)
            } else {
                (200.0, 100.0)
            }
        );
    }
}

#[test]
fn inverse_roundtrips_across_negative_crop_origins_and_rotations() {
    for rotation in 0..4 {
        let page = PageGeometry::new(
            [-130.0, -70.0, 870.0, 630.0],
            rotation,
            PdfLimits::default(),
        )
        .unwrap();
        for x in [-12.5, 0.0, 0.25, 349.75, 700.0] {
            for y in [-1.0, 0.0, 10.5, 699.0] {
                let point = PdfPoint { x, y };
                assert_eq!(
                    page.pdf_to_document(page.document_to_pdf(point).unwrap())
                        .unwrap(),
                    point
                );
            }
        }
    }
}

#[test]
fn glyph_box_uses_all_four_corners_under_rotation() {
    let page = PageGeometry::new([10.0, 20.0, 110.0, 220.0], 1, PdfLimits::default()).unwrap();
    assert_eq!(
        page.pdf_box_to_document([30.0, 50.0, 45.0, 75.0]).unwrap(),
        PdfRect {
            left: 30.0,
            top: 20.0,
            right: 55.0,
            bottom: 35.0
        }
    );
}

#[test]
fn malformed_geometry_is_refused_before_native_use() {
    let limits = PdfLimits::default();
    for crop in [
        [0.0, 0.0, 0.0, 2.0],
        [1.0, 0.0, 0.0, 2.0],
        [0.0, f64::NAN, 1.0, 2.0],
        [0.0, 0.0, 65_536.0, 1.0],
    ] {
        assert!(PageGeometry::new(crop, 0, limits).is_err());
    }
    assert!(PageGeometry::new([0.0, 0.0, 1.0, 2.0], 4, limits).is_err());
}

#[test]
fn tile_admission_rejects_noninvertible_nonfinite_and_oversized_requests() {
    let page = PageGeometry::new([0.0, 0.0, 100.0, 200.0], 0, PdfLimits::default()).unwrap();
    let valid = PdfTileRequest {
        x: 0.0,
        y: 0.0,
        pixels_per_point: 2.0,
        width: 512,
        height: 512,
        include_annotations: true,
    };
    assert_eq!(valid.validate(page).unwrap(), 512 * 512 * 4);
    for scale in [0.0, -1.0, f64::NAN, f64::INFINITY, 65.0] {
        assert!(valid_with_scale(valid, scale).validate(page).is_err());
    }
    assert!(
        PdfTileRequest {
            width: 513,
            ..valid
        }
        .validate(page)
        .is_err()
    );
    assert!(PdfTileRequest { x: 100.0, ..valid }.validate(page).is_err());
    assert!(PdfTileRequest { y: -0.1, ..valid }.validate(page).is_err());
}
fn valid_with_scale(mut request: PdfTileRequest, value: f64) -> PdfTileRequest {
    request.pixels_per_point = value;
    request
}

#[test]
fn input_capacity_and_budget_are_admitted_not_only_length() {
    let limits = PdfLimits {
        memory_bytes: 16 * 1024 * 1024,
        ..PdfLimits::default()
    };
    assert!(limits.admit_input(1024, 1024).is_ok());
    assert!(matches!(
        limits.admit_input(1024, 12 * 1024 * 1024),
        Err(PdfError::Memory { .. })
    ));
    assert!(limits.admit_input(8, 7).is_err());
    assert!(
        PdfLimits {
            pages: u32::MAX,
            ..limits
        }
        .validate()
        .is_err()
    );
}

#[test]
fn rectangle_overlap_has_no_false_positive_for_touching_edges() {
    let a = PdfRect {
        left: 0.0,
        top: 0.0,
        right: 10.0,
        bottom: 10.0,
    };
    assert!(!a.intersects(PdfRect {
        left: 10.0,
        top: 0.0,
        right: 20.0,
        bottom: 10.0
    }));
    assert!(a.intersects(PdfRect {
        left: 9.0,
        top: 9.0,
        right: 20.0,
        bottom: 10.0
    }));
    assert!(PdfRect { right: -1.0, ..a }.validate().is_err());
}
