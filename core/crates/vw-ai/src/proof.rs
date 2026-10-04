pub use crate::metrics::{delta_e2000, srgb_lab};
use crate::{
    Cancellation, Error, Result, check_cancel,
    pixels::{self, EditImage},
    sha256,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vw_raster::Pixels;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metrics {
    pub selected_pixels: u64,
    pub delta_e2000_mean: f64,
    pub delta_e2000_max: f64,
    pub ssim_inside_mask: f64,
    pub definition: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub schema: u32,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub dilation_px: u32,
    pub changed_outside: u64,
    pub outside_pixels: u64,
    pub outside_sha256_before: String,
    pub outside_sha256_after: String,
    pub source_pixels_sha256: String,
    pub result_pixels_sha256: String,
    pub source_file_sha256: String,
    pub normalized_icc_sha256: String,
    pub mask_sha256: String,
    pub metrics: Metrics,
    pub acceptance: Option<AcceptanceProof>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceProof {
    pub mask_sha256: String,
    pub changed_unaccepted: u64,
    pub before_sha256: String,
    pub after_sha256: String,
}
pub(crate) fn pixel_hash(image: &EditImage) -> String {
    let mut hash = Sha256::new();
    hash.update(b"vw-ai/pixels/v1\0");
    hash.update(image.width.to_le_bytes());
    hash.update(image.height.to_le_bytes());
    hash.update([image.bit_depth()]);
    match &image.pixels {
        Pixels::Rgba8(p) => hash.update(p),
        Pixels::Rgba16(p) => {
            for value in p {
                hash.update(value.to_le_bytes());
            }
        }
    }
    crate::hex(hash.finalize().as_ref())
}
fn scan(
    source: &EditImage,
    result: &EditImage,
    allowed: &[u8],
    cancel: &dyn Cancellation,
) -> Result<(u64, u64, String, String)> {
    if source.width != result.width
        || source.height != result.height
        || source.bit_depth() != result.bit_depth()
        || source.icc != result.icc
        || allowed.len() != crate::pixel_count(source.width, source.height)?
    {
        return Err(Error::Invalid("proof source/result binding"));
    }
    let mut before = Sha256::new();
    before.update(b"vw-ai/indexed-exterior/v1\0");
    before.update(source.width.to_le_bytes());
    before.update(source.height.to_le_bytes());
    before.update([source.bit_depth()]);
    before.update(sha256(&source.icc).as_bytes());
    before.update(sha256(allowed).as_bytes());
    let mut after = before.clone();
    let (mut changed, mut count) = (0, 0);
    for (i, &selected) in allowed.iter().enumerate() {
        if i % 4096 == 0 {
            check_cancel(cancel)?;
        }
        if selected != 0 {
            continue;
        }
        count += 1;
        before.update((i as u64).to_le_bytes());
        after.update((i as u64).to_le_bytes());
        match (&source.pixels, &result.pixels) {
            (Pixels::Rgba8(a), Pixels::Rgba8(b)) => {
                let a = &a[i * 4..i * 4 + 4];
                let b = &b[i * 4..i * 4 + 4];
                changed += u64::from(a != b);
                before.update(a);
                after.update(b);
            }
            (Pixels::Rgba16(a), Pixels::Rgba16(b)) => {
                let a = &a[i * 4..i * 4 + 4];
                let b = &b[i * 4..i * 4 + 4];
                changed += u64::from(a != b);
                for v in a {
                    before.update(v.to_le_bytes());
                }
                for v in b {
                    after.update(v.to_le_bytes());
                }
            }
            _ => return Err(Error::Invalid("proof depth")),
        }
    }
    Ok((
        changed,
        count,
        crate::hex(before.finalize().as_ref()),
        crate::hex(after.finalize().as_ref()),
    ))
}
pub(crate) fn compare(
    source: &EditImage,
    result: &EditImage,
    mask: &vw_mask::Mask,
    radius: u32,
    source_file: &str,
    acceptance: Option<&vw_mask::Mask>,
    cancel: &dyn Cancellation,
) -> Result<Proof> {
    if radius > crate::MAX_FEATHER || !crate::valid_hash(source_file) {
        return Err(Error::Invalid("proof binding or radius"));
    }
    let dense = mask.to_dense()?;
    let excluded = mask.expand(radius + 1)?.to_dense()?;
    let (changed_outside, outside_pixels, before, after) = scan(source, result, &excluded, cancel)?;
    if changed_outside != 0 {
        return Err(Error::Exterior(changed_outside));
    }
    let acceptance = acceptance
        .map(|a| -> Result<_> {
            let dense = a.to_dense()?;
            let (changed, _, before, after) = scan(source, result, &dense, cancel)?;
            if changed != 0 {
                return Err(Error::Exterior(changed));
            }
            Ok(AcceptanceProof {
                mask_sha256: sha256(&dense),
                changed_unaccepted: changed,
                before_sha256: before,
                after_sha256: after,
            })
        })
        .transpose()?;
    check_cancel(cancel)?;
    let source8 = source.rgba8();
    let a = pixels::transform(&source8, &source.icc, true)?;
    drop(source8);
    let result8 = result.rgba8();
    let b = pixels::transform(&result8, &result.icc, true)?;
    drop(result8);
    let metrics = crate::metrics::metrics(&a, &b, &dense, cancel)?;
    check_cancel(cancel)?;
    Ok(Proof {
        schema: 1,
        width: source.width,
        height: source.height,
        bit_depth: source.bit_depth(),
        dilation_px: radius + 1,
        changed_outside,
        outside_pixels,
        outside_sha256_before: before,
        outside_sha256_after: after,
        source_pixels_sha256: pixel_hash(source),
        result_pixels_sha256: pixel_hash(result),
        source_file_sha256: source_file.into(),
        normalized_icc_sha256: sha256(&source.icc),
        mask_sha256: sha256(&dense),
        metrics,
        acceptance,
    })
}
