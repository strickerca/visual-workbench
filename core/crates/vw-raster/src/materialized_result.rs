//! Versioned full-frame AI composites already contain the original. Applying
//! them with source-over changes exterior alpha. These layers replace covered
//! samples and use premultiplied interpolation solely for explicit layer opacity.
use crate::{DecodedImage, RasterError};
use vw_model::{AssetId, ResultCandidate};
use vw_proto::v1::Affine;
fn value(json: &str) -> Result<serde_json::Value, RasterError> {
    if json.len() > 16 * 1024 {
        return Err(RasterError::Invalid("result proof size"));
    }
    serde_json::from_str(json).map_err(|_| RasterError::Invalid("result proof"))
}
pub(crate) fn identifies(candidate: &ResultCandidate) -> Result<bool, RasterError> {
    Ok(value(&candidate.definition.proof_json)?
        .get("materialization")
        .and_then(serde_json::Value::as_str)
        == Some("vw-ai-composite-v1"))
}
pub(crate) fn validate(candidate: &ResultCandidate, source: &AssetId) -> Result<(), RasterError> {
    let v = value(&candidate.definition.proof_json)?;
    let proof = &v["proof"];
    if v["schema"].as_u64() != Some(1)
        || v["materialization"].as_str() != Some("vw-ai-composite-v1")
        || v["source_asset_id"].as_str() != Some(source.as_str())
        || v["composite_asset_id"].as_str()
            != Some(candidate.definition.composite_asset_id.as_str())
        || v["acceptance_mask_asset_id"].as_str()
            != candidate
                .acceptance_mask_asset_id
                .as_ref()
                .map(AssetId::as_str)
        || v["changed_outside"].as_u64() != Some(0)
        || proof["changed_outside"].as_u64() != Some(0)
        || proof["outside_sha256_before"].as_str().is_none()
        || proof["outside_sha256_before"] != proof["outside_sha256_after"]
    {
        return Err(RasterError::Invalid("materialized result binding"));
    }
    let acceptance = &proof["acceptance"];
    if candidate.acceptance_mask_asset_id.is_some() {
        if acceptance["changed_unaccepted"].as_u64() != Some(0)
            || acceptance["before_sha256"].as_str().is_none()
            || acceptance["before_sha256"] != acceptance["after_sha256"]
        {
            return Err(RasterError::Invalid("partial result proof"));
        }
    } else if !acceptance.is_null() {
        return Err(RasterError::Invalid("unexpected acceptance proof"));
    }
    Ok(())
}
pub(crate) fn replace(
    dst: &mut DecodedImage,
    src: &DecodedImage,
    t: &Affine,
    origin: (u32, u32),
    opacity: f64,
    cancel: &dyn Fn() -> bool,
) -> Result<(), RasterError> {
    let det = t.a * t.d - t.b * t.c;
    if ![t.a, t.b, t.c, t.d, t.e, t.f, det, opacity]
        .iter()
        .all(|v| v.is_finite())
        || det.abs() < 1e-12
        || !(0.0..=1.0).contains(&opacity)
        || dst.pixels.bit_depth() != src.pixels.bit_depth()
    {
        return Err(RasterError::Invalid("materialized result transform/depth"));
    }
    for y in 0..dst.height {
        crate::source::check_cancel(cancel)?;
        for x in 0..dst.width {
            let dx = f64::from(x) + f64::from(origin.0) + 0.5 - t.e;
            let dy = f64::from(y) + f64::from(origin.1) + 0.5 - t.f;
            let sx = (t.d * dx - t.c * dy) / det;
            let sy = (-t.b * dx + t.a * dy) / det;
            if sx < 0.0 || sy < 0.0 || sx >= f64::from(src.width) || sy >= f64::from(src.height) {
                continue;
            }
            let i = (y as usize * dst.width as usize + x as usize) * 4;
            let j = (sy as usize * src.width as usize + sx as usize) * 4;
            if opacity == 1.0 {
                for c in 0..4 {
                    dst.pixels.set_sample(i + c, src.pixels.sample(j + c));
                }
                continue;
            }
            if opacity == 0.0 {
                continue;
            }
            let a = f64::from(dst.pixels.sample(i + 3)) / 65535.0;
            let b = f64::from(src.pixels.sample(j + 3)) / 65535.0;
            let alpha = a * (1.0 - opacity) + b * opacity;
            for c in 0..3 {
                let old = f64::from(dst.pixels.sample(i + c)) / 65535.0;
                let new = f64::from(src.pixels.sample(j + c)) / 65535.0;
                let color = if alpha == 0.0 {
                    old * (1.0 - opacity) + new * opacity
                } else {
                    (old * a * (1.0 - opacity) + new * b * opacity) / alpha
                };
                dst.pixels
                    .set_sample(i + c, libm::round(color.clamp(0.0, 1.0) * 65535.0) as u16);
            }
            dst.pixels
                .set_sample(i + 3, libm::round(alpha.clamp(0.0, 1.0) * 65535.0) as u16);
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn image(values: Vec<u8>) -> DecodedImage {
        DecodedImage {
            width: 2,
            height: 1,
            pixels: crate::Pixels::Rgba8(values),
            icc: None,
            source_asset: AssetId::hash(b"fixture"),
            original_available: true,
            orientation_applied: 1,
        }
    }
    #[test]
    fn replacement_preserves_8bit_hidden_rgb_and_does_not_source_over_alpha() {
        let mut dst = image(vec![11, 22, 33, 128, 44, 55, 66, 200]);
        let src = image(vec![199, 99, 49, 0, 20, 30, 40, 128]);
        replace(
            &mut dst,
            &src,
            &Affine {
                a: 1.0,
                d: 1.0,
                ..Default::default()
            },
            (0, 0),
            1.0,
            &|| false,
        )
        .unwrap();
        assert_eq!(dst.pixels, src.pixels);
        let old = dst.pixels.clone();
        replace(
            &mut dst,
            &src,
            &Affine {
                a: 1.0,
                d: 1.0,
                ..Default::default()
            },
            (0, 0),
            0.0,
            &|| false,
        )
        .unwrap();
        assert_eq!(dst.pixels, old);
    }
    #[test]
    fn invalid_or_cancelled_replacement_never_starts_pixel_mutation() {
        let mut dst = image(vec![1; 8]);
        let before = dst.pixels.clone();
        let src = image(vec![2; 8]);
        assert!(
            replace(
                &mut dst,
                &src,
                &Affine {
                    a: f64::NAN,
                    d: 1.0,
                    ..Default::default()
                },
                (0, 0),
                1.0,
                &|| false
            )
            .is_err()
        );
        assert_eq!(dst.pixels, before);
        assert!(
            replace(
                &mut dst,
                &src,
                &Affine {
                    a: 1.0,
                    d: 1.0,
                    ..Default::default()
                },
                (0, 0),
                1.0,
                &|| true
            )
            .is_err()
        );
        assert_eq!(dst.pixels, before);
    }
}
