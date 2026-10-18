//! Whole-output fallback behind the existing private file publication owner.
//! Ordinary documents retain the streaming PNG path. Configured geometry or a
//! clean tonal export must never silently use a D-space strip instead.
use super::*;

pub(super) fn required(state:&ProjectState,options:&ExportOptions)->TransferResult<bool>{
    vw_raster::requires_document_output(state.project()?,&Id::try_from(options.document_id.clone())?,options.marked).map_err(transfer_raster_error)
}
pub(super) fn source_budget(state:&ProjectState,options:&ExportOptions)->TransferResult<u64>{
    if required(state,options)? {working(state,options.memory_budget_bytes)} else {Ok(options.memory_budget_bytes)}
}
fn working(state:&ProjectState,budget:u64)->TransferResult<u64>{
    const FOREIGN:u64=16*1024*1024;
    if budget<=FOREIGN{return Err(TransferError::Memory{estimated:FOREIGN+1,budget});}
    let canonical=crate::workflow::canonical_workspace(state,budget-FOREIGN,false).map_err(|e|match e{
        crate::workflow::WorkflowError::Memory{estimated,..}=>TransferError::Memory{estimated:estimated.saturating_add(FOREIGN),budget},
        crate::workflow::WorkflowError::Closed=>TransferError::Closed,crate::workflow::WorkflowError::Storage=>TransferError::Storage,
        _=>TransferError::Backpressure})?;
    budget.checked_sub(canonical+FOREIGN).ok_or(TransferError::Backpressure)
}
pub(super) fn preflight(state:&ProjectState,options:&FileExportOptions)->TransferResult<FilePreflight>{
    let budget=options.export.memory_budget_bytes;let available=working(state,budget)?;
    let project=state.project()?;let document=Id::try_from(options.export.document_id.clone())?;
    let definition=&project.documents.get(&document).ok_or(TransferError::Invalid)?.definition;
    let asset=project.assets.get(&AssetId::try_from(definition.primary_asset_id.clone())?).ok_or(TransferError::Metadata)?;
    if asset.byte_size>INPUT_LIMIT{return Err(TransferError::EncodedLimit);}
    let [sw,sh]=if asset.orientation>=5{[asset.height,asset.width]}else{[asset.width,asset.height]};
    let [width,height]=project.active_image_output(&document)?.map_or([sw,sh],|v|v.output_size());
    let reservation=vw_raster::document_output_reservation(project,&document,vw_raster::RenderOptions{
        memory_budget_bytes:available,include_guides:false,assume_untagged_srgb:options.export.assume_untagged_srgb}).map_err(transfer_raster_error)?;
    memory(reservation,available)?;
    let mut request=request(&options.export,definition,state.view_revision()?)?;request.memory_budget_bytes=u64::MAX;
    let region=request.region.unwrap_or(vw_raster::Region{x:0,y:0,width,height});
    let codec=vw_raster::preflight(width,height,asset.bit_depth as u8,false,&request).map_err(|error|match error{
        vw_raster::RasterError::Dimensions{limit,..}=>TransferError::Dimensions{width:region.width,height:region.height,limit},
        other=>transfer_raster_error(other)})?;
    let encode=asset.byte_size.checked_add(u64::from(sw)*u64::from(sh)*8)
        .and_then(|v|v.checked_add(u64::from(width)*u64::from(height)*8))
        .and_then(|v|v.checked_add(codec)).and_then(|v|v.checked_add(profile_reserve(!asset.icc_profile.is_empty())))
        .ok_or(TransferError::Backpressure)?;
    let decode=if matches!(asset.format.as_str(),"heic"|"heif") {
        u64::from(sw.div_ceil(64)*64).checked_mul(u64::from(sh.div_ceil(64)*64))
            .and_then(|n|n.checked_mul(64)).and_then(|n|n.checked_add(asset.byte_size.checked_mul(5)?))
            .and_then(|n|n.checked_add(32*1024*1024)).ok_or(TransferError::Backpressure)?
    } else {0};
    let estimate=memory(reservation.max(encode).max(decode),available)?;
    Ok(FilePreflight{width:region.width,height:region.height,source_bit_depth:asset.bit_depth as u8,
        output_bit_depth:if matches!(request.format,vw_raster::ExportFormat::Png16){16}else{8},
        estimated_peak_bytes:estimate+(budget-available),buffered:true,requires_render_validation:true,revision:state.info()?})
}
pub(super) fn export(source:&mut dyn RasterSource,state:&ProjectState,document:&Id,options:&FileExportOptions,
    request:&vw_raster::ExportRequest,cancel:&Cancellation,sink:&mut impl Write)->TransferResult<vw_raster::StreamReport>{
    let available=working(state,request.memory_budget_bytes)?;
    let retained=source.resident_bytes().checked_add(source.read_workspace_bytes())
        .and_then(|v|v.checked_add(profile_reserve(source.info().icc.is_some()))).ok_or(TransferError::Backpressure)?;
    memory(retained,available)?;
    let remaining=available-retained;
    let reservation=vw_raster::document_output_reservation(state.project()?,document,vw_raster::RenderOptions{
        memory_budget_bytes:remaining,include_guides:false,assume_untagged_srgb:options.export.assume_untagged_srgb}).map_err(transfer_raster_error)?;
    memory(reservation,remaining)?;
    let full=vw_raster::Region{x:0,y:0,width:source.info().width,height:source.info().height};
    cancel.check()?;
    let original=source.read_region(full,available,&||cancel.is_cancelled()).map_err(transfer_raster_error)?;
    let assets=ResultAssets{blobs:&state.blobs,budget:remaining-reservation,cancel};
    let mut request=request.clone();request.memory_budget_bytes=remaining;
    let encoded=vw_raster::export_document_output(&original,state.project()?,document,&assets,
        vw_raster::RenderOptions{memory_budget_bytes:remaining,include_guides:false,assume_untagged_srgb:options.export.assume_untagged_srgb},
        options.export.marked,&request,&||cancel.is_cancelled()).map_err(transfer_raster_error)?;
    cancel.check()?;
    if encoded.bytes.len() as u64>options.max_encoded_bytes{return Err(TransferError::EncodedLimit);}
    for chunk in encoded.bytes.chunks(64*1024){cancel.check()?;sink.write_all(chunk)?;}
    let [width,height]=encoded.metadata.image_output.map_or([full.width,full.height],|v|[v.output_width,v.output_height]);
    let region=request.region.unwrap_or(vw_raster::Region{x:0,y:0,width,height});
    Ok(vw_raster::StreamReport{metadata:encoded.metadata,plan:vw_raster::StreamPlan{region,strip_rows:region.height,
        estimated_peak_bytes:options.export.memory_budget_bytes},encoded_bytes:encoded.bytes.len() as u64,strips:1})
}
