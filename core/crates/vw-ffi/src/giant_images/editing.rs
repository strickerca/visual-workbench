//! Current-revision D-space editing tiles over authenticated level-0 cache data.
//! Coarser cached JPEGs are deliberately never fed to the canonical compositor.
use super::*;
use vw_model::{AssetId, Id, Project};
use vw_proto::v1::object_state::Shape;
use vw_raster::{
    AssetResolver, DecodedImage, Pixels, RasterError, Region, RenderOptions, SourceInfo,
};

const MAX_RESULT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RESULT_PIXELS: u64 = 50_000_000;

struct Snapshot {
    project: Project,
    blobs: vw_store::BlobStore,
    document: Id,
    // The cloned project and result metadata die before their reservation.
    _charge: MemoryCharge,
}

fn raster_error(error: RasterError) -> Error {
    match error {
        RasterError::Cancelled => Error::Cancelled,
        RasterError::MissingAsset | RasterError::OriginalRequired => Error::Missing,
        RasterError::Unsupported(_) | RasterError::CodecUnavailable => Error::Unsupported,
        RasterError::Memory { estimated, budget } => Error::Memory { estimated, budget },
        RasterError::Allocation | RasterError::EncodedLimit { .. } => Error::Limit,
        RasterError::Io => Error::Storage,
        _ => Error::Invalid,
    }
}

fn visible_result_workspace(project: &Project, document: &Id) -> Result<u64> {
    let mut largest = 0u64;
    for object in project.objects.values() {
        if &object.document_id != document || object.state.hidden {
            continue;
        }
        let Some(Shape::ResultId(result)) = &object.state.shape else {
            continue;
        };
        let layer = Id::from_proto(object.state.layer_id.as_ref()).map_err(|_| Error::Invalid)?;
        if !project
            .layers
            .get(&layer)
            .is_some_and(|v| v.visible && v.opacity > 0.0)
        {
            continue;
        }
        let result = project
            .results
            .get(&Id::from_proto(Some(result)).map_err(|_| Error::Invalid)?)
            .ok_or(Error::Missing)?;
        let asset = if result.definition.composite_asset_id.is_empty() {
            &result.definition.output_asset_id
        } else {
            &result.definition.composite_asset_id
        };
        let asset = project
            .assets
            .get(&AssetId::try_from(asset.clone()).map_err(|_| Error::Invalid)?)
            .ok_or(Error::Missing)?;
        let pixels = u64::from(asset.width) * u64::from(asset.height);
        if pixels == 0 || pixels > MAX_RESULT_PIXELS || asset.byte_size > MAX_RESULT_BYTES {
            return Err(Error::Unsupported);
        }
        let bytes = pixels
            .checked_mul(40)
            .and_then(|n| n.checked_add(asset.byte_size))
            .and_then(|n| n.checked_add(64 * 1024 * 1024))
            .ok_or(Error::Limit)?;
        largest = largest.max(bytes);
    }
    Ok(largest)
}

struct ResultAssets<'a> {
    snapshot: &'a Snapshot,
    cancel: &'a Cancellation,
    budget: u64,
}
impl AssetResolver for ResultAssets<'_> {
    fn image(&self, asset: &str) -> std::result::Result<DecodedImage, RasterError> {
        if self.cancel.is_cancelled() {
            return Err(RasterError::Cancelled);
        }
        let id = AssetId::try_from(asset.to_owned()).map_err(|_| RasterError::MissingAsset)?;
        let metadata = self
            .snapshot
            .project
            .assets
            .get(&id)
            .ok_or(RasterError::MissingAsset)?;
        if metadata.byte_size == 0 || metadata.byte_size > MAX_RESULT_BYTES {
            return Err(RasterError::Unsupported("bounded result original"));
        }
        let path = self
            .snapshot
            .blobs
            .path(&id)
            .map_err(|_| RasterError::MissingAsset)?;
        let bytes = crate::streaming_ffi::read_bounded(&path, metadata.byte_size, self.cancel)
            .map_err(|error| match error {
                crate::CoreError::Cancelled => RasterError::Cancelled,
                _ => RasterError::MissingAsset,
            })?;
        if bytes.len() as u64 != metadata.byte_size || AssetId::hash(&bytes) != id {
            return Err(RasterError::MissingAsset);
        }
        let remaining = self
            .budget
            .checked_sub(bytes.capacity() as u64)
            .ok_or(RasterError::Allocation)?;
        let image = crate::format_images::decode_asset(
            &bytes,
            vw_raster::DecodeLimits {
                max_encoded_bytes: MAX_RESULT_BYTES as usize,
                max_pixels: MAX_RESULT_PIXELS,
                max_memory_bytes: remaining,
            },
            &|| self.cancel.is_cancelled(),
            metadata,
        )?;
        if self.cancel.is_cancelled() {
            return Err(RasterError::Cancelled);
        }
        Ok(image)
    }
}

impl ImageState {
    fn original_region(
        &mut self,
        region: Region,
        cancel: &Cancellation,
        budget: &SessionBudget,
    ) -> Result<(Vec<u8>, MemoryCharge)> {
        let bytes = u64::from(region.width)
            .checked_mul(u64::from(region.height))
            .and_then(|n| n.checked_mul(4))
            .ok_or(Error::Limit)?;
        let charge = reserve(budget, MemoryKind::Transfer, bytes)?;
        let len = usize::try_from(bytes).map_err(|_| Error::Limit)?;
        let mut assembled = Vec::new();
        assembled.try_reserve_exact(len).map_err(|_| Error::Limit)?;
        if assembled.capacity() as u64 > charge.bytes() {
            return Err(Error::Limit);
        }
        assembled.resize(len, 0);
        let last_x = region.x + region.width - 1;
        let last_y = region.y + region.height - 1;
        for ty in region.y / 256..=last_y / 256 {
            for tx in region.x / 256..=last_x / 256 {
                cancel.check().map_err(Error::from)?;
                let (part, part_charge) = self
                    .tile(
                        GiantTileAddress {
                            level: 0,
                            x: tx,
                            y: ty,
                        },
                        cancel,
                        budget,
                    )?
                    .into_parts();
                tiles::copy_original_tile(region, &part, &mut assembled, cancel)?;
                drop(part);
                drop(part_charge);
            }
        }
        cancel.check().map_err(Error::from)?;
        Ok((assembled, charge))
    }

    fn editing_tile(
        &mut self,
        requested: GiantTileAddress,
        assume_untagged_srgb: bool,
        snapshot: &Snapshot,
        cancel: &Cancellation,
        budget: &SessionBudget,
    ) -> Result<ChargedTile> {
        // Higher JPEG pyramid levels are never compositor input. Levels 1-2
        // compose up to 1024² original pixels, then use a declared box filter.
        if requested.level > 2 {
            return Err(Error::Unsupported);
        }
        let key = key(requested);
        let tile = self.manifest.pyramid().region(key).map_err(asset_error)?;
        let (document_x, document_y, document_width, document_height) =
            document_region(&self.manifest, key)?;
        let full = SourceInfo {
            width: self.manifest.source().width,
            height: self.manifest.source().height,
            bit_depth: 8,
            icc: None,
            source_asset: AssetId::try_from(tiles::hex(&self.manifest.source().original))
                .map_err(|_| Error::Invalid)?,
            original_available: true,
            orientation_applied: self.manifest.source().orientation,
        };
        let region = Region {
            x: document_x,
            y: document_y,
            width: document_width,
            height: document_height,
        };
        let options = RenderOptions {
            memory_budget_bytes: budget.limit_bytes(),
            include_guides: false,
            assume_untagged_srgb,
        };
        let working = vw_raster::region_composition_estimate(
            &full,
            &snapshot.project,
            &snapshot.document,
            options,
            region,
        )
        .map_err(raster_error)?;
        let resolver = visible_result_workspace(&snapshot.project, &snapshot.document)?;
        let _work = reserve(
            budget,
            MemoryKind::CodecWorkspace,
            working.checked_add(resolver).ok_or(Error::Limit)?,
        )?;
        // The returned transfer charge is acquired before asking the compositor
        // to allocate its result. The raw input has its own transfer charge.
        let outgoing = reserve(budget, MemoryKind::Transfer, tiles::TILE_RGBA_BYTES)?;
        let (pixels, raw_charge) = if requested.level == 0 {
            let (raw, charge) = self.tile(requested, cancel, budget)?.into_parts();
            (raw.rgba, charge)
        } else {
            self.original_region(region, cancel, budget)?
        };
        let source = DecodedImage {
            width: region.width,
            height: region.height,
            pixels: Pixels::Rgba8(pixels),
            icc: None,
            source_asset: full.source_asset.clone(),
            original_available: true,
            orientation_applied: full.orientation_applied,
        };
        let assets = ResultAssets {
            snapshot,
            cancel,
            budget: resolver,
        };
        let result = vw_raster::compose_document_region(
            &source,
            &full,
            &snapshot.project,
            &snapshot.document,
            &assets,
            options,
            region,
            &|| cancel.is_cancelled(),
        )
        .map_err(raster_error)?;
        let rgba = if requested.level == 0 {
            match result.pixels {
                Pixels::Rgba8(value) => value,
                Pixels::Rgba16(_) => return Err(Error::Unsupported),
            }
        } else {
            vw_raster::downsample_composed_box_rgba8(
                &result,
                1u32 << requested.level,
                tile.width,
                tile.height,
                &|| cancel.is_cancelled(),
            )
            .map_err(raster_error)?
        };
        if rgba.capacity() as u64 > outgoing.bytes() {
            return Err(Error::Limit);
        }
        drop(source);
        drop(raw_charge);
        cancel.check().map_err(Error::from)?;
        Ok(ChargedTile {
            pixels: GiantTilePixels {
                asset_id: tiles::hex(&self.manifest.source().original),
                manifest_hash: self.identity.clone(),
                address: requested,
                document_x: region.x,
                document_y: region.y,
                document_width: region.width,
                document_height: region.height,
                width: tile.width,
                height: tile.height,
                rgba,
            },
            charge: outgoing,
        })
    }
}

impl GiantImage {
    /// The exported E-owned PreparedGiantTile wraps the returned charged pixels.
    /// Root registers the additive UniFFI method after the E delivery repair.
    pub(crate) async fn read_editing_tile(
        &self,
        address: GiantTileAddress,
        assume_untagged_srgb: bool,
        cancellation: Arc<Cancellation>,
    ) -> Result<Arc<super::super::PreparedGiantTile>> {
        self.check(cancellation.clone()).await?;
        let binding = self.binding.clone();
        let expected = self.source.clone();
        let closed = self.project_closed.clone();
        let local_closed = self.closed.clone();
        let cancel = cancellation.clone();
        let budget = self.budget.clone();
        let snapshot = self
            .project
            .call(move |state| {
                Ok(
                    source::capture(state, &binding, &closed, &cancel, &budget).and_then(
                        |current| {
                            if !current.same_source(&expected) {
                                return Err(Error::Stale);
                            }
                            live(&local_closed, &cancel)?;
                            let estimate = crate::workflow::canonical_workspace(
                                state,
                                budget.limit_bytes().min(crate::workflow::MAX_MEMORY),
                                false,
                            )
                            .map_err(|_| Error::Limit)?;
                            let charge = reserve(&budget, MemoryKind::CodecWorkspace, estimate)?;
                            let document = Id::try_from(binding.document_id.clone())
                                .map_err(|_| Error::Invalid)?;
                            let project = state.project().map_err(Error::from)?.clone();
                            Ok(Snapshot {
                                project,
                                blobs: state.blobs.clone(),
                                document,
                                _charge: charge,
                            })
                        },
                    ),
                )
            })
            .await
            .map_err(Error::from)??;
        self.check(cancellation.clone()).await?;
        let source = self.source.clone();
        let closed = self.project_closed.clone();
        let local_closed = self.closed.clone();
        let budget = self.budget.clone();
        let cancel = cancellation.clone();
        let tile = self
            .worker
            .call(move |state| {
                Ok((|| {
                    live(&local_closed, &cancel)?;
                    source.verify_owned(&closed, &local_closed, &cancel, &budget)?;
                    let tile = state.as_mut().ok_or(Error::Closed)?.editing_tile(
                        address,
                        assume_untagged_srgb,
                        &snapshot,
                        &cancel,
                        &budget,
                    )?;
                    source.verify_owned(&closed, &local_closed, &cancel, &budget)?;
                    live(&local_closed, &cancel)?;
                    Ok::<_, Error>(tile)
                })())
            })
            .await
            .map_err(Error::from)??;
        self.check(cancellation).await?;
        let (pixels, outgoing) = tile.into_parts();
        super::super::PreparedGiantTile::admit(pixels, outgoing)
    }
}

#[cfg(test)]
#[path = "editing_tests.rs"]
mod tests;
