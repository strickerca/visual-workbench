//! Read-only projection of canonical checked-package image coordinates.
//! It does not open capture sources or turn package metadata into alignment.
use super::*;

fn content_bounds(geometry: vw_package::VerificationImageGeometry) -> PackageResult<Vec<f64>> {
    let content = geometry.document_region(geometry.crop_document)?;
    Ok(vec![
        content.left(),
        content.top(),
        content.width(),
        content.height(),
    ])
}

fn coordinates(
    key: &VerificationPackageKey,
    package: &vw_package::Package,
) -> PackageResult<VerificationImageCoordinates> {
    if !key.matches(package) {
        return Err(PackageError::Stale);
    }
    let geometry = vw_package::verification_image_geometry(package)?;
    let crop = geometry.crop_document;
    Ok(VerificationImageCoordinates {
        key: key.clone(),
        binding: checked::binding(package),
        source_asset_id: package.manifest().source.asset_hash.clone(),
        source_width: geometry.source_size[0],
        source_height: geometry.source_size[1],
        output_width: geometry.output_size[0],
        output_height: geometry.output_size[1],
        image_width: geometry.image_size[0],
        image_height: geometry.image_size[1],
        content_width: geometry.content_size[0],
        content_height: geometry.content_size[1],
        content_bounds: content_bounds(geometry)?,
        crop_document: vec![crop.left(), crop.top(), crop.width(), crop.height()],
        document_to_image: geometry.document_to_image.coefficients().to_vec(),
        image_to_document: geometry.image_to_document.coefficients().to_vec(),
        has_output_frame: geometry.has_output_frame,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn fractional_content_edge_does_not_admit_the_rounded_padding_pixel() {
        let document_to_image = vw_geom::Affine::scale(0.5, 0.5).unwrap();
        let geometry = vw_package::VerificationImageGeometry {
            source_size: [5, 7],
            output_size: [5, 7],
            image_size: [64, 64],
            content_size: [3, 4],
            crop_document: vw_geom::Rect::new(0.0, 0.0, 5.0, 7.0).unwrap(),
            document_to_image,
            image_to_document: document_to_image.inverse().unwrap(),
            has_output_frame: false,
        };
        assert_eq!(content_bounds(geometry).unwrap(), [0.0, 0.0, 2.5, 3.5]);
        assert_eq!(geometry.content_size, [3, 4]);
    }
}

#[uniffi::export]
impl PackageCatalog {
    /// Read both exact checked catalog packages and return their canonical
    /// D→O→PNG mappings. This cannot admit a verification review by itself;
    /// the receipt, source and capture-owner checks remain on that path.
    pub async fn verification_coordinate_pair(
        &self,
        pair: VerificationPair,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<VerificationCoordinatePair> {
        pair.validate()?;
        let original = self
            .verification_package(
                pair.original.package_id.clone(),
                pair.original.target.clone(),
                pair.original.manifest_sha256.clone(),
                cancellation.clone(),
            )
            .await?;
        let after = self
            .verification_package(
                pair.after.package_id.clone(),
                pair.after.target.clone(),
                pair.after.manifest_sha256.clone(),
                cancellation.clone(),
            )
            .await?;
        if original.manifest().extensions.target_profile
            != after.manifest().extensions.target_profile
        {
            return Err(PackageError::Stale);
        }
        let result = VerificationCoordinatePair {
            original: coordinates(&pair.original, &original)?,
            after: coordinates(&pair.after, &after)?,
            pair: pair.clone(),
        };
        checked::catalog_current(self, &pair, cancellation).await?;
        Ok(result)
    }
}
