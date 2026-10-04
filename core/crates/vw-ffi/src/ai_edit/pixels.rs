use super::*;
use crate::{ProjectSession, project::ProjectState};
use std::sync::Arc;
use vw_model::{AssetId, Id};
pub(super) fn region(r: AiRegion, w: u32, h: u32) -> AiEditResult<vw_raster::Region> {
    if r.width > 512 || r.height > 512 {
        return Err(AiEditError::Limit);
    }
    let out = vw_raster::Region {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    };
    out.validate(w, h)?;
    Ok(out)
}
pub(super) fn compare(
    s: &request::RequestState,
    mode: AiCompareMode,
    r: AiRegion,
    stop: &request::Stop,
) -> AiEditResult<AiPixels> {
    let done = s.current()?;
    let w = done.image().width();
    let h = done.image().height();
    let canvas = if matches!(mode, AiCompareMode::Split) {
        w.checked_mul(2).ok_or(AiEditError::Limit)?
    } else {
        w
    };
    region(r, canvas, h)?;
    memory(
        s.resident() + u64::from(w) * u64::from(h) * 64 + 64 * 1024 * 1024,
        s.options.memory_budget_bytes,
    )?;
    let mode = match mode {
        AiCompareMode::Before => vw_ai::CompareMode::Blink { show_result: false },
        AiCompareMode::After => vw_ai::CompareMode::Blink { show_result: true },
        AiCompareMode::Wipe {
            vertical,
            cut,
            result_before,
        } => vw_ai::CompareMode::Wipe {
            axis: if vertical {
                vw_ai::Axis::Horizontal
            } else {
                vw_ai::Axis::Vertical
            },
            cut,
            result_before,
        },
        AiCompareMode::Split => vw_ai::CompareMode::Split,
        AiCompareMode::Difference { gain } => vw_ai::CompareMode::Difference { gain },
    };
    let image = done.compare(mode, stop)?;
    let length = r.width as usize * r.height as usize * 4;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(length)
        .map_err(|_| AiEditError::Limit)?;
    for y in r.y..r.y + r.height {
        stop.check()?;
        let start = (y as usize * image.width as usize + r.x as usize) * 4;
        pixels.extend_from_slice(
            image
                .rgba_srgb
                .get(start..start + r.width as usize * 4)
                .ok_or(AiEditError::Proof)?,
        );
    }
    Ok(AiPixels {
        candidate_id: s.candidate_id.clone(),
        region: r,
        canvas_width: image.width,
        canvas_height: image.height,
        rgba_srgb: pixels,
    })
}
#[uniffi::export]
impl ProjectSession {
    pub async fn ai_result_pixels(
        &self,
        binding: crate::WorkflowBinding,
        result_id: String,
        area: AiRegion,
        memory_budget_bytes: u64,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<AiResultPixels> {
        self.check_open()?;
        binding.validate()?;
        let id = Id::try_from(result_id.clone())?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    crate::workflow::canonical_workspace(
                        state,
                        memory_budget_bytes.min(crate::workflow::MAX_MEMORY),
                        false,
                    )?;
                    crate::workflow::check_binding(state, &binding)?;
                    let revision = vw_proto::v1::Revision {
                        host_seq: binding.host_seq,
                        state_hash: vw_model::StateHash::try_from(binding.state_hash.clone())?
                            .bytes()
                            .to_vec(),
                    };
                    let (original, composite) = result_binding(state, &binding, &id)?;
                    let project = state.project()?;
                    let asset = project.assets.get(&composite).ok_or(AiEditError::Proof)?;
                    let w = asset.width;
                    let h = asset.height;
                    let region = region(area, w, h)?;
                    memory(
                        u64::from(w) * u64::from(h) * 40 + asset.byte_size + 64 * 1024 * 1024,
                        memory_budget_bytes,
                    )?;
                    let bytes = crate::masks::ai_bridge::blob(
                        state,
                        &composite,
                        asset.byte_size,
                        vw_ai::MAX_ENCODED as u64,
                        &cancellation,
                    )?;
                    let decoded = vw_raster::decode(
                        &bytes,
                        vw_raster::DecodeLimits {
                            max_encoded_bytes: vw_ai::MAX_ENCODED,
                            max_pixels: vw_ai::MAX_PIXELS,
                            max_memory_bytes: memory_budget_bytes - bytes.len() as u64,
                        },
                    )?;
                    if decoded.width != w
                        || decoded.height != h
                        || decoded.pixels.bit_depth() as u32 != asset.bit_depth
                        || decoded.orientation_applied != 1
                        || decoded.icc.as_deref().unwrap_or(&[]) != asset.icc_profile
                    {
                        return Err(AiEditError::Proof);
                    }
                    let encoded = vw_raster::export(
                        &decoded,
                        &vw_raster::ExportRequest {
                            format: vw_raster::ExportFormat::Png8,
                            region: Some(region),
                            revision,
                            alpha: vw_raster::AlphaPolicy::Preserve,
                            color: vw_raster::ColorPolicy::ConvertToSrgb {
                                assume_untagged_srgb: false,
                            },
                            allow_depth_reduction: true,
                            memory_budget_bytes: memory_budget_bytes - bytes.len() as u64,
                            capture_session: None,
                            frame_id: None,
                        },
                    )?;
                    drop(decoded);
                    drop(bytes);
                    check(&closed, &cancellation)?;
                    let pixels = vw_raster::decode(
                        &encoded.bytes,
                        vw_raster::DecodeLimits {
                            max_encoded_bytes: vw_ai::MAX_ENCODED,
                            max_pixels: 512 * 512,
                            max_memory_bytes: memory_budget_bytes - encoded.bytes.len() as u64,
                        },
                    )?;
                    let vw_raster::Pixels::Rgba8(rgba_srgb) = pixels.pixels else {
                        return Err(AiEditError::Proof);
                    };
                    Ok(AiResultPixels {
                        binding,
                        result_id,
                        source_asset_id: original.to_string(),
                        composite_asset_id: composite.to_string(),
                        region: area,
                        canvas_width: w,
                        canvas_height: h,
                        rgba_srgb,
                    })
                })())
            })
            .await?
    }
}
pub(super) fn result_binding(
    state: &ProjectState,
    binding: &crate::WorkflowBinding,
    id: &Id,
) -> AiEditResult<(AssetId, AssetId)> {
    let project = state.project()?;
    let candidate = project.results.get(id).ok_or(AiEditError::Invalid)?;
    if candidate.definition.document_id
        != Some(Id::try_from(binding.document_id.clone())?.to_proto())
        || !matches!(candidate.status.as_str(), "ready" | "accepted" | "partial")
    {
        return Err(AiEditError::Invalid);
    }
    let original = AssetId::try_from(
        project
            .documents
            .get(&Id::try_from(binding.document_id.clone())?)
            .ok_or(AiEditError::Invalid)?
            .definition
            .primary_asset_id
            .clone(),
    )?;
    let composite = AssetId::try_from(candidate.definition.composite_asset_id.clone())?;
    result::validate_materialized(
        &candidate.definition.proof_json,
        &original,
        &composite,
        candidate
            .acceptance_mask_asset_id
            .as_ref()
            .map(|id| id.as_str()),
    )?;
    Ok((original, composite))
}
