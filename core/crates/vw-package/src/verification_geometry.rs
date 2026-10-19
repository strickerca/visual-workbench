//! Read-only geometry for checked package images. These matrices describe pixel
//! coordinates; they do not establish registration, capture permission or a verdict.
use crate::{Error, ImageMapping, Package, Result};
use vw_geom::{Affine, Point, Rect};

/// Original D, saved O and actual padded clean-image pixels remain distinct.
/// Native verification owners may project this fixed metadata into their DTO;
/// it creates no source, image, mutable-project or capture owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VerificationImageGeometry {
    pub source_size: [u32; 2],
    pub output_size: [u32; 2],
    pub image_size: [u32; 2],
    pub content_size: [u32; 2],
    pub crop_document: Rect,
    pub document_to_image: Affine,
    pub image_to_document: Affine,
    pub has_output_frame: bool,
}
impl VerificationImageGeometry {
    /// The whole ROI must survive the saved crop. A clipped/omitted instruction
    /// is not silently replaced by a smaller comparison or padded image pixels.
    pub fn document_region(self, region: Rect) -> Result<Rect> {
        let crop = self.crop_document;
        if region.width() <= 0.0
            || region.height() <= 0.0
            || region.left() < crop.left()
            || region.top() < crop.top()
            || region.right() > crop.right()
            || region.bottom() > crop.bottom()
        {
            return Err(Error::Invalid("verification region outside output crop"));
        }
        self.document_to_image
            .map_rect(region)
            .map_err(geometry_error)
    }
}

/// Derive one canonical D→O→clean-image mapping from an already checked package.
/// Package validation has authenticated the clean PNG dimensions/hash and exact
/// overview mapping. No PNG decode, allocation, IO or scalar Kotlin approximation
/// is needed for this metadata query. Target-normalized marker coordinates are
/// deliberately not used: these matrices address physical retained-image pixels.
pub fn verification_image_geometry(package: &Package) -> Result<VerificationImageGeometry> {
    let manifest = package.manifest();
    let mut clean = manifest
        .images
        .iter()
        .filter(|image| image.role == "clean_source");
    let image = clean.next().ok_or(Error::Integrity)?;
    if clean.next().is_some()
        || manifest.compiled_for.scale != manifest.extensions.overview_mapping.scale()
    {
        return Err(Error::Integrity);
    }
    geometry(
        [manifest.source.width, manifest.source.height],
        manifest.extensions.image_output,
        manifest.extensions.overview_mapping,
        [image.width, image.height],
    )
}

/// Map a before-image pixel into the after image for an independently measured
/// D-space translation: A(B^-1(q) + d). This operation neither measures d nor
/// asserts that any two pixels/images align. Source/CAS/ROI and provenance checks
/// remain with the existing verification owner before accepting a comparison.
pub fn verification_comparison_transform(
    before: VerificationImageGeometry,
    after: VerificationImageGeometry,
    translation_document: Point,
) -> Result<Affine> {
    let translate = Affine::new(
        1.0,
        0.0,
        0.0,
        1.0,
        translation_document.x(),
        translation_document.y(),
    )
    .map_err(geometry_error)?;
    before
        .image_to_document
        .then(translate)
        .map_err(geometry_error)?
        .then(after.document_to_image)
        .map_err(geometry_error)
}

fn geometry_error(_: vw_geom::GeometryError) -> Error {
    Error::Invalid("verification image geometry")
}
fn geometry(
    source: [u32; 2],
    output: Option<vw_raster::ImageOutputMetadata>,
    mapping: ImageMapping,
    image: [u32; 2],
) -> Result<VerificationImageGeometry> {
    if source.contains(&0)
        || mapping.scale_numerator == 0
        || mapping.scale_denominator == 0
        || mapping.scale_numerator > mapping.scale_denominator
    {
        return Err(Error::Integrity);
    }
    let (matrix, extent, crop) = if let Some(value) = output {
        let frame = value.frame()?;
        if frame.source_size() != source {
            return Err(Error::Integrity);
        }
        (
            frame.matrix().map_err(geometry_error)?,
            frame.output_size(),
            frame.crop(),
        )
    } else {
        (Affine::IDENTITY, source, [0, 0, source[0], source[1]])
    };
    let content = extent.map(|n| {
        (u64::from(n) * u64::from(mapping.scale_numerator))
            .div_ceil(u64::from(mapping.scale_denominator)) as u32
    });
    if mapping.source_rectangle != [0, 0, i64::from(extent[0]), i64::from(extent[1])]
        || [mapping.content_width, mapping.content_height] != content
        || [mapping.width, mapping.height] != image
        || image[0] < content[0]
        || image[1] < content[1]
    {
        return Err(Error::Integrity);
    }
    let scale = mapping.scale();
    let document_to_image = matrix
        .then(Affine::new(scale, 0.0, 0.0, scale, 0.0, 0.0).map_err(geometry_error)?)
        .map_err(geometry_error)?;
    Ok(VerificationImageGeometry {
        source_size: source,
        output_size: extent,
        image_size: image,
        content_size: content,
        crop_document: Rect::new(
            f64::from(crop[0]),
            f64::from(crop[1]),
            f64::from(crop[2]),
            f64::from(crop[3]),
        )
        .map_err(geometry_error)?,
        image_to_document: document_to_image.inverse().map_err(geometry_error)?,
        document_to_image,
        has_output_frame: output.is_some(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use vw_geom::{ImageOutputFrame, QuarterTurn};
    fn framed(rotation: QuarterTurn) -> VerificationImageGeometry {
        let frame =
            ImageOutputFrame::new([100, 80], [10, 20, 32, 40], rotation, [320, 512]).unwrap();
        let mapping = ImageMapping::new([0, 0, 320, 512], 256, 64).unwrap();
        geometry(
            [100, 80],
            Some(vw_raster::ImageOutputMetadata::new(frame)),
            mapping,
            [160, 256],
        )
        .unwrap()
    }
    #[test]
    fn crop_quarter_turn_and_package_scale_have_independent_pixel_goldens() {
        let value = framed(QuarterTurn::Clockwise90);
        assert_eq!(
            value.document_to_image.coefficients(),
            [0.0, 8.0, -4.0, 0.0, 240.0, -80.0]
        );
        assert_eq!(
            value
                .document_to_image
                .map(Point::new(12.0, 25.0).unwrap())
                .unwrap(),
            Point::new(140.0, 16.0).unwrap()
        );
        assert_eq!(
            value
                .document_region(Rect::new(12.0, 25.0, 3.0, 7.0).unwrap())
                .unwrap(),
            Rect::new(112.0, 16.0, 28.0, 24.0).unwrap()
        );
    }
    #[test]
    fn all_quarter_turns_agree_with_the_existing_compiler_mapping() {
        let target = crate::Target::Generic { max_long_edge: 256 };
        for rotation in [
            QuarterTurn::Zero,
            QuarterTurn::Clockwise90,
            QuarterTurn::Half,
            QuarterTurn::Clockwise270,
        ] {
            let value = framed(rotation);
            let frame =
                ImageOutputFrame::new([100, 80], [10, 20, 32, 40], rotation, [320, 512]).unwrap();
            let metadata = vw_raster::ImageOutputMetadata::new(frame);
            let canonical =
                crate::geometry::DocumentMapping::new([100, 80], Some(metadata)).unwrap();
            let mapping = ImageMapping::new([0, 0, 320, 512], 256, 64).unwrap();
            for p in [[10.0, 20.0], [40.0, 60.0], [12.5, 25.5]] {
                let point = Point::new(p[0], p[1]).unwrap();
                let actual = value.document_to_image.map(point).unwrap();
                let expected = mapping.point(canonical.point(p).unwrap(), &target);
                assert!(
                    (actual.x() - expected[0]).abs() < 1e-9
                        && (actual.y() - expected[1]).abs() < 1e-9
                );
                let back = value.image_to_document.map(actual).unwrap();
                assert!((back.x() - p[0]).abs() < 1e-9 && (back.y() - p[1]).abs() < 1e-9);
            }
        }
    }
    #[test]
    fn translation_is_conjugated_between_different_output_grids() {
        let before = framed(QuarterTurn::Clockwise90);
        let frame =
            ImageOutputFrame::new([100, 80], [4, 5, 50, 45], QuarterTurn::Zero, [100, 90]).unwrap();
        let mapping = ImageMapping::new([0, 0, 100, 90], 256, 64).unwrap();
        let after = geometry(
            [100, 80],
            Some(vw_raster::ImageOutputMetadata::new(frame)),
            mapping,
            [100, 90],
        )
        .unwrap();
        let transform =
            verification_comparison_transform(before, after, Point::new(2.0, 3.0).unwrap())
                .unwrap();
        assert_eq!(
            transform.map(Point::new(140.0, 16.0).unwrap()).unwrap(),
            Point::new(20.0, 46.0).unwrap()
        );
    }
    #[test]
    fn padding_and_equal_image_sizes_do_not_imply_an_identity_grid() {
        let mapping = ImageMapping::new([0, 0, 16, 12], 256, 64).unwrap();
        let ordinary = geometry([16, 12], None, mapping, [64, 64]).unwrap();
        assert_eq!(ordinary.output_size, [16, 12]);
        assert_eq!(ordinary.content_size, [16, 12]);
        let frame =
            ImageOutputFrame::new([16, 12], [0, 0, 16, 12], QuarterTurn::Clockwise90, [12, 16])
                .unwrap();
        let mapping = ImageMapping::new([0, 0, 12, 16], 256, 64).unwrap();
        let rotated = geometry(
            [16, 12],
            Some(vw_raster::ImageOutputMetadata::new(frame)),
            mapping,
            [64, 64],
        )
        .unwrap();
        assert_eq!(ordinary.image_size, rotated.image_size);
        assert_ne!(ordinary.document_to_image, rotated.document_to_image);
        assert!(
            ordinary
                .document_region(Rect::new(15.0, 0.0, 2.0, 1.0).unwrap())
                .is_err()
        );
    }
    #[test]
    fn invalid_mapping_extent_scale_and_source_binding_refuse() {
        let mapping = ImageMapping::new([0, 0, 16, 12], 256, 64).unwrap();
        let mut bad = mapping;
        bad.scale_denominator = 0;
        assert!(geometry([16, 12], None, bad, [64, 64]).is_err());
        let mut bad = mapping;
        bad.content_width += 1;
        assert!(geometry([16, 12], None, bad, [64, 64]).is_err());
        assert!(geometry([16, 12], None, mapping, [63, 64]).is_err());
        let frame = ImageOutputFrame::identity(16, 12).unwrap();
        assert!(
            geometry(
                [15, 12],
                Some(vw_raster::ImageOutputMetadata::new(frame)),
                mapping,
                [64, 64]
            )
            .is_err()
        );
        assert!(
            framed(QuarterTurn::Half)
                .document_region(Rect::new(0.0, 20.0, 11.0, 2.0).unwrap())
                .is_err()
        );
    }
}
