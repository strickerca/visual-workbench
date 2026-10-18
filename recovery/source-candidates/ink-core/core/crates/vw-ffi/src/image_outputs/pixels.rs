use super::*;
use crate::{CoreError,ExportOptions,ExportResult,ImageFormat,project::ProjectState};
use std::io::Read;
use vw_raster::{AssetResolver,DecodedImage,RasterError,RenderOptions};

fn raster(error:RasterError)->WorkflowError {
    match error {
        RasterError::Memory{estimated,budget}=>WorkflowError::Memory{estimated,budget},
        RasterError::Cancelled=>WorkflowError::Cancelled,
        RasterError::OriginalRequired|RasterError::MissingAsset=>WorkflowError::Missing,
        RasterError::Io=>WorkflowError::Storage,
        RasterError::Allocation|RasterError::EncodedLimit{..}|RasterError::ScratchLimit{..}=>WorkflowError::Limit,
        _=>WorkflowError::Invalid,
    }
}
fn memory(estimated:u64,budget:u64)->WorkflowResult<()> {
    if estimated>budget{Err(WorkflowError::Memory{estimated,budget})}else{Ok(())}
}
fn working(state:&ProjectState,document:&Id,budget:u64,assume:bool)->WorkflowResult<(u64,u64)> {
    // The meter runs before binding/state hash and before image allocation.
    admit(state,budget,false,OUTPUT_WORK)?;
    let canonical=canonical_workspace(state,budget-OUTPUT_WORK,false)?;
    let available=budget.checked_sub(canonical+OUTPUT_WORK).ok_or(WorkflowError::Limit)?;
    bounds(state.project()?,document)?;
    let reservation=vw_raster::document_output_reservation(state.project()?,document,
        RenderOptions{memory_budget_bytes:available,include_guides:false,assume_untagged_srgb:assume}).map_err(raster)?;
    memory(reservation,available)?;
    Ok((available,reservation))
}

fn load(blobs:&vw_store::BlobStore,asset:&AssetId,budget:u64,cancel:&Cancellation)->Result<DecodedImage,RasterError> {
    if cancel.is_cancelled(){return Err(RasterError::Cancelled);}
    let path=blobs.path(asset).map_err(|_|RasterError::MissingAsset)?;
    let mut file=std::fs::File::open(path).map_err(|_|RasterError::MissingAsset)?;
    let size=file.metadata().map_err(|_|RasterError::Io)?.len();
    if size==0||size>64*1024*1024||size>=budget {return Err(RasterError::Memory{estimated:size.saturating_add(1),budget});}
    let size=usize::try_from(size).map_err(|_|RasterError::Allocation)?;
    let mut bytes=Vec::new();bytes.try_reserve_exact(size).map_err(|_|RasterError::Allocation)?;bytes.resize(size,0);
    for chunk in bytes.chunks_mut(64*1024){if cancel.is_cancelled(){return Err(RasterError::Cancelled);}file.read_exact(chunk).map_err(|_|RasterError::Io)?;}
    if file.read(&mut[0u8;1]).map_err(|_|RasterError::Io)?!=0||AssetId::hash(&bytes)!=*asset {return Err(RasterError::MissingAsset);}
    let remaining=budget.checked_sub(bytes.capacity() as u64).ok_or(RasterError::Allocation)?;
    let image=crate::os_images::decode(&bytes,vw_raster::DecodeLimits{max_encoded_bytes:64*1024*1024,max_pixels:50_000_000,
        max_memory_bytes:remaining},&||cancel.is_cancelled())?;
    if cancel.is_cancelled(){return Err(RasterError::Cancelled);}Ok(image)
}
struct Assets<'a>{blobs:&'a vw_store::BlobStore,budget:u64,cancel:&'a Cancellation}
impl AssetResolver for Assets<'_>{fn image(&self,asset:&str)->Result<DecodedImage,RasterError>{
    let asset=AssetId::try_from(asset.to_owned()).map_err(|_|RasterError::MissingAsset)?;
    load(self.blobs,&asset,self.budget,self.cancel)
}}

#[allow(clippy::too_many_arguments)] // One immutable worker invocation, with all admission inputs explicit.
pub(super) fn preview(state:&ProjectState,binding:WorkflowBinding,marked:bool,region:vw_raster::Region,assume:bool,budget:u64,
    cancel:&Cancellation)->WorkflowResult<ImageOutputPreview> {
    let document=id(&binding.document_id)?;
    let (available,reservation)=working(state,&document,budget,assume)?;
    check_binding(state,&binding)?;
    let project=state.project()?;let asset=bounds(project,&document)?;
    let frame=project.active_image_output(&document)?;
    let [output_width,output_height]=frame.map_or(dimensions(asset),|v|v.output_size());
    region.validate(output_width,output_height).map_err(raster)?;
    let source=load(&state.blobs,&AssetId::try_from(asset.asset_id.clone())?,available,cancel).map_err(raster)?;
    let assets=Assets{blobs:&state.blobs,budget:available-reservation,cancel};
    let output=vw_raster::prepare_document_output(&source,project,&document,&assets,
        RenderOptions{memory_budget_bytes:available,include_guides:false,assume_untagged_srgb:assume},marked,&||cancel.is_cancelled()).map_err(raster)?;
    drop(source);
    // Crop DISPLAY pixels only after reserving the full retained output and
    // worst-case profile conversion workspace. Full output remains unchanged.
    let tile_reserve=u64::from(region.width)*u64::from(region.height)*64+16*1024*1024
        +if output.image.icc.is_some(){72*1024*1024}else{0};
    memory(vw_raster::image_resident_bytes(&output.image).map_err(raster)?.checked_add(tile_reserve).ok_or(WorkflowError::Limit)?,available)?;
    cancel.check()?;let mut tile=output.image.crop(region).map_err(raster)?;drop(output);
    tile.convert_profile(None,assume).map_err(raster)?;cancel.check()?;
    let rgba=tile.pixels.rgba8();
    Ok(ImageOutputPreview{binding,source_asset_id:asset.asset_id.clone(),output:frame.map(ImageOutputSettings::from_frame),
        output_width,output_height,x:region.x,y:region.y,width:region.width,height:region.height,rgba})
}

/// The original legacy in-memory export API routes configured output through
/// the same compositor. It cannot silently export an untransformed document.
pub(crate) fn legacy_export(state:&ProjectState,options:&ExportOptions,cancel:&Cancellation)->crate::Result<ExportResult> {
    let result=(||->WorkflowResult<ExportResult>{
        let document=id(&options.document_id)?;
        let (available,reservation)=working(state,&document,options.memory_budget_bytes,options.assume_untagged_srgb)?;
        let project=state.project()?;let source_id=AssetId::try_from(bounds(project,&document)?.asset_id.clone())?;
        let definition=&project.documents.get(&document).ok_or(WorkflowError::Missing)?.definition;
        let source=load(&state.blobs,&source_id,available,cancel).map_err(raster)?;
        let region=options.region.map(|v|{
            if [v.x,v.y,v.width,v.height].iter().any(|v|!v.is_finite()||*v<0.0||*v>f64::from(u32::MAX)||v.fract()!=0.0){return Err(WorkflowError::Invalid);}
            Ok(vw_raster::Region{x:v.x as u32,y:v.y as u32,width:v.width as u32,height:v.height as u32})
        }).transpose()?;
        let request=vw_raster::ExportRequest{format:match options.format{
            ImageFormat::Png8=>vw_raster::ExportFormat::Png8,ImageFormat::Png16=>vw_raster::ExportFormat::Png16,
            ImageFormat::Jpeg{quality}=>vw_raster::ExportFormat::Jpeg{quality},ImageFormat::WebpLossless=>vw_raster::ExportFormat::WebpLossless,
            ImageFormat::WebpLossy{quality}=>vw_raster::ExportFormat::WebpLossy{quality}},region,revision:state.view_revision()?,
            alpha:options.matte_rgb.map_or(vw_raster::AlphaPolicy::Preserve,|v|vw_raster::AlphaPolicy::Matte([(v>>16)as u8,(v>>8)as u8,v as u8])),
            color:if options.convert_to_srgb{vw_raster::ColorPolicy::ConvertToSrgb{assume_untagged_srgb:options.assume_untagged_srgb}}else{vw_raster::ColorPolicy::Preserve},
            allow_depth_reduction:options.allow_depth_reduction,memory_budget_bytes:available,
            capture_session:definition.capture.as_ref().map(|v|Id::from_proto(v.capture_session_id.as_ref())).transpose()?,frame_id:definition.capture.as_ref().map(|v|v.frame_id)};
        let assets=Assets{blobs:&state.blobs,budget:available-reservation,cancel};
        let result=vw_raster::export_document_output(&source,project,&document,&assets,RenderOptions{memory_budget_bytes:available,include_guides:false,
            assume_untagged_srgb:options.assume_untagged_srgb},options.marked,&request,&||cancel.is_cancelled()).map_err(raster)?;
        cancel.check()?;if result.bytes.len()>64*1024*1024{return Err(WorkflowError::Limit);}
        Ok(ExportResult{blake3:AssetId::hash(&result.bytes).to_string(),bytes:result.bytes,
            metadata_json:serde_json::to_string(&result.metadata).map_err(|_|WorkflowError::Invalid)?,revision:state.info()?})
    })();
    result.map_err(core_error)
}

pub(crate) fn admit_legacy_export(state:&ProjectState,options:&ExportOptions)->crate::Result<()> {
    let document=Id::try_from(options.document_id.clone())?;
    if vw_raster::requires_document_output(state.project()?,&document,options.marked)? {
        working(state,&document,options.memory_budget_bytes,options.assume_untagged_srgb).map_err(core_error)?;
    }
    Ok(())
}
fn core_error(e:WorkflowError)->CoreError {match e {WorkflowError::Cancelled=>CoreError::Cancelled,WorkflowError::Closed=>CoreError::Closed,
    WorkflowError::Storage=>CoreError::Storage,WorkflowError::Memory{..}|WorkflowError::Limit=>CoreError::Backpressure,_=>CoreError::Raster}}
