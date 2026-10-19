use super::*;

#[uniffi::export]
impl ProjectSession {
    /// Borrow current canonical metadata only. No source decode, artifact,
    /// transaction, cache or native child owner is created by this query.
    pub async fn image_output_export_geometry(
        &self,
        binding: WorkflowBinding,
        source_region: Option<crate::SelectionRegion>,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<ImageOutputExportGeometry> {
        self.check_open()?;
        binding.validate()?;
        cancellation.check()?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, FRAME_WORK)?;
                    check_binding(state, &binding)?;
                    let document = id(&binding.document_id)?;
                    let project = state.project()?;
                    let asset = bounds(project, &document)?;
                    let source = dimensions(asset);
                    let frame = project
                        .image_output(&document)?
                        .filter(|value| value.enabled)
                        .map(|value| value.frame);
                    let output = frame.map_or(source, |value| value.output_size());
                    let region = project_region(source, frame, source_region)?;
                    let encoded = region
                        .as_ref()
                        .map_or(output, |value| [value.width, value.height]);
                    check_live(&closed, &cancellation)?;
                    Ok(ImageOutputExportGeometry {
                        binding,
                        source_asset_id: asset.asset_id.clone(),
                        source_width: source[0],
                        source_height: source[1],
                        output_width: output[0],
                        output_height: output[1],
                        region,
                        encoded_width: encoded[0],
                        encoded_height: encoded[1],
                    })
                })())
            })
            .await?
    }
}

fn project_region(
    source: [u32; 2],
    frame: Option<vw_geom::ImageOutputFrame>,
    region: Option<crate::SelectionRegion>,
) -> WorkflowResult<Option<crate::SelectionRegion>> {
    let Some(region) = region else {
        return Ok(None);
    };
    if region.width == 0
        || region.height == 0
        || region
            .x
            .checked_add(region.width)
            .is_none_or(|right| right > source[0])
        || region
            .y
            .checked_add(region.height)
            .is_none_or(|bottom| bottom > source[1])
    {
        return Err(WorkflowError::Invalid);
    }
    // No enabled frame preserves legacy extents; do not impose the frame's
    // 32768-side bound on an ordinary giant image merely to copy its region.
    let Some(frame) = frame else {
        return Ok(Some(region));
    };
    let [x, y, width, height] =
        frame.source_region([region.x, region.y, region.width, region.height])?;
    Ok(Some(crate::SelectionRegion {
        x,
        y,
        width,
        height,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn unframed_giant_geometry_preserves_large_extents_without_frame_admission() {
        let region = crate::SelectionRegion {
            x: 90_000,
            y: 1,
            width: 10_000,
            height: 2,
        };
        let result = project_region([100_000, 200_000], None, Some(region)).unwrap();
        let result = result.unwrap();
        assert_eq!(
            [result.x, result.y, result.width, result.height],
            [90_000, 1, 10_000, 2]
        );
        assert!(
            project_region([100_000, 200_000], None, None)
                .unwrap()
                .is_none()
        );
    }
}
