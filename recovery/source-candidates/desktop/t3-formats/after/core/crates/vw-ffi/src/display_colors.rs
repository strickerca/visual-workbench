//! Revision-bound original display. No persistent mutation or export conversion.
use crate::{Cancellation, CoreError, ProjectSession, WorkflowBinding, WorkflowError};
use std::{io::Read, sync::Arc};
use vw_model::{AssetId, Id};
use vw_raster::{DisplayOptions, DisplaySpace, Region};
const MIB: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum DisplayColorSpace { Srgb, DisplayP3 }
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct DisplayRegion { pub x: u32, pub y: u32, pub width: u32, pub height: u32 }
#[derive(Clone, Debug, uniffi::Record)]
pub struct DisplayColorRequest {
    pub binding: WorkflowBinding,
    pub region: Option<DisplayRegion>,
    pub space: DisplayColorSpace,
    pub assume_untagged_srgb: bool,
    pub allow_display_depth_reduction: bool,
    pub memory_budget_bytes: u64,
}
#[derive(Clone, uniffi::Record)]
pub struct DisplayColorPixels {
    pub binding: WorkflowBinding, pub source_asset_id: String,
    pub region: DisplayRegion, pub space: DisplayColorSpace,
    pub rgba: Vec<u8>, pub icc_profile: Vec<u8>, pub source_bit_depth: u8,
    pub assumed_srgb: bool, pub depth_reduced: bool, pub estimated_peak_bytes: u64,
}
#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum DisplayColorError {
    #[error("invalid display request")] Invalid,
    #[error("original or revision changed")] Stale,
    #[error("untagged source requires an explicit sRGB assumption")] Untagged,
    #[error("profile or platform decoder unsupported for bounded display")] Unsupported,
    #[error("display depth reduction requires explicit permission")] Depth,
    #[error("display workspace exceeds budget")] Memory { estimated: u64, budget: u64 },
    #[error("display operation cancelled")] Cancelled,
    #[error("project closed")] Closed,
    #[error("display queue is full")] Backpressure,
    #[error("original source unavailable or changed")] Storage,
}
type Result<T> = std::result::Result<T, DisplayColorError>;
impl From<CoreError> for DisplayColorError { fn from(e: CoreError) -> Self { match e { CoreError::Cancelled=>Self::Cancelled,CoreError::Closed=>Self::Closed,CoreError::Backpressure|CoreError::Worker=>Self::Backpressure,CoreError::Storage=>Self::Storage,_=>Self::Invalid } } }
impl From<WorkflowError> for DisplayColorError { fn from(e: WorkflowError) -> Self { match e { WorkflowError::Stale=>Self::Stale,WorkflowError::Memory{estimated,budget}=>Self::Memory{estimated,budget},WorkflowError::Closed=>Self::Closed,WorkflowError::Cancelled=>Self::Cancelled,_=>Self::Invalid } } }
impl From<vw_raster::RasterError> for DisplayColorError { fn from(e: vw_raster::RasterError) -> Self { match e { vw_raster::RasterError::Memory{estimated,budget}=>Self::Memory{estimated,budget},vw_raster::RasterError::Depth=>Self::Depth,vw_raster::RasterError::Cancelled=>Self::Cancelled,_=>Self::Unsupported } } }
fn checked_add(a:u64,b:u64)->Result<u64>{a.checked_add(b).ok_or(DisplayColorError::Invalid)}
fn memory(peak:u64,budget:u64)->Result<()> { if peak>budget {Err(DisplayColorError::Memory{estimated:peak,budget})}else{Ok(())} }

#[uniffi::export]
impl ProjectSession {
    pub async fn display_original(&self, request: DisplayColorRequest, cancellation: Arc<Cancellation>) -> Result<DisplayColorPixels> {
        self.check_open()?; request.binding.validate()?;
        if request.memory_budget_bytes == 0 || request.memory_budget_bytes > 256*MIB {return Err(DisplayColorError::Invalid);}
        let closed = self.closed.clone();
        self.worker.call(move |state| Ok((|| {
            cancellation.check()?;
            if closed.load(std::sync::atomic::Ordering::Acquire) {return Err(DisplayColorError::Closed);}
            let budget=request.memory_budget_bytes;
            let canonical=crate::workflow::canonical_workspace(state,budget,false)?;
            crate::workflow::check_binding(state,&request.binding)?;
            let document=Id::try_from(request.binding.document_id.clone()).map_err(|_|DisplayColorError::Invalid)?;
            let project=state.project()?;
            let doc=&project.documents.get(&document).ok_or(DisplayColorError::Stale)?.definition;
            let asset_id=AssetId::try_from(doc.primary_asset_id.clone()).map_err(|_|DisplayColorError::Invalid)?;
            let asset=project.assets.get(&asset_id).ok_or(DisplayColorError::Storage)?;
            let (width,height)=if asset.orientation>=5 {(asset.height,asset.width)}else{(asset.width,asset.height)};
            let region=request.region.unwrap_or(DisplayRegion{x:0,y:0,width,height});
            let area=Region{x:region.x,y:region.y,width:region.width,height:region.height}; area.validate(width,height)?;
            let pixels=u64::from(width)*u64::from(height); let count=u64::from(region.width)*u64::from(region.height);
            if pixels==0 || pixels>50_000_000 || count>vw_raster::MAX_DISPLAY_PIXELS || asset.byte_size==0 || asset.byte_size>64*MIB || !(1..=8).contains(&asset.orientation) || asset.icc_profile.len()>4*MIB as usize {return Err(DisplayColorError::Invalid);}
            if asset.icc_profile.is_empty()&&!request.assume_untagged_srgb {return Err(DisplayColorError::Untagged);}
            if asset.bit_depth>8&&!request.allow_display_depth_reduction {return Err(DisplayColorError::Depth);}
            // Admission before encoded read or codec. Full actual decoder allowance
            // is disjoint from canonical/encoded storage. Display rechecks actual
            // capacities and profiles before its first semantic parse/copy.
            let retained=checked_add(canonical,asset.byte_size)?;
            let display=checked_add(checked_add(pixels*8,count*48)?,vw_raster::DISPLAY_COLOR_WORK+20*MIB)?;
            let decode = match asset.format.as_str() {
                "bmp" | "tiff" => checked_add(checked_add(pixels*32,asset.byte_size*2)?,checked_add(asset.icc_profile.len() as u64*4,64*MIB)?)?,
                "avif" => checked_add(checked_add(u64::from(width.div_ceil(128)*128)*u64::from(height.div_ceil(128)*128)*64,asset.byte_size*4)?,32*MIB)?,
                "heic" | "heif" => checked_add(checked_add(u64::from(width.div_ceil(64)*64)*u64::from(height.div_ceil(64)*64)*64,asset.byte_size*4)?,32*MIB)?,
                _ => checked_add(pixels*32,32*MIB)?,
            };
            memory(checked_add(retained,display.max(decode))?,budget)?;
            let path=state.blobs.path(&asset_id).map_err(|_|DisplayColorError::Storage)?;
            let mut file=std::fs::File::open(path).map_err(|_|DisplayColorError::Storage)?;
            if file.metadata().map_err(|_|DisplayColorError::Storage)?.len()!=asset.byte_size {return Err(DisplayColorError::Storage);}
            let len=usize::try_from(asset.byte_size).map_err(|_|DisplayColorError::Invalid)?;
            let mut encoded=Vec::new();encoded.try_reserve_exact(len).map_err(|_|DisplayColorError::Memory{estimated:asset.byte_size,budget})?;encoded.resize(len,0);
            for chunk in encoded.chunks_mut(65536) {cancellation.check()?;file.read_exact(chunk).map_err(|_|DisplayColorError::Storage)?;}
            if file.read(&mut[0u8;1]).map_err(|_|DisplayColorError::Storage)?!=0||AssetId::hash(&encoded)!=asset_id {return Err(DisplayColorError::Storage);}
            let decoded=crate::format_images::decode(&encoded,vw_raster::DecodeLimits{max_encoded_bytes:len,max_pixels:pixels,max_memory_bytes:budget-retained},&||cancellation.is_cancelled())?;
            if decoded.width!=width||decoded.height!=height||u32::from(decoded.pixels.bit_depth())!=asset.bit_depth||u32::from(decoded.orientation_applied)!=asset.orientation||decoded.pixels.has_alpha()!=asset.has_alpha||decoded.icc.as_deref().unwrap_or(&[])!=asset.icc_profile||decoded.source_asset!=asset_id {return Err(DisplayColorError::Storage);}
            let space=match request.space {DisplayColorSpace::Srgb=>DisplaySpace::Srgb,DisplayColorSpace::DisplayP3=>DisplaySpace::DisplayP3};
            let frame=vw_raster::display_region(&decoded,area,DisplayOptions{space,assume_untagged_srgb:request.assume_untagged_srgb,allow_depth_reduction:request.allow_display_depth_reduction,memory_budget_bytes:budget,retained_bytes:checked_add(canonical,encoded.capacity() as u64)?},&||cancellation.is_cancelled())?;
            cancellation.check()?;
            if closed.load(std::sync::atomic::Ordering::Acquire) {return Err(DisplayColorError::Closed);}
            Ok(DisplayColorPixels{binding:request.binding,source_asset_id:asset_id.to_string(),region,space:request.space,rgba:frame.rgba,icc_profile:frame.icc,source_bit_depth:frame.source_bit_depth,assumed_srgb:frame.assumed_srgb,depth_reduced:frame.depth_reduced,estimated_peak_bytes:frame.estimated_peak_bytes})
        })())).await?
    }
}
