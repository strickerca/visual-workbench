//! One document composition path for output preview, export and instruction packages.
use crate::{AssetResolver, DecodedImage, ExportRequest, ExportedImage, ImageOutputMetadata,
    RasterError, RenderOptions};
use vw_model::{Id, Project};

/// Whether a document-aware transfer must use the full output compositor.
/// Raw image export has no project context and deliberately cannot infer this.
pub fn requires_document_output(project:&Project,document:&Id,marked:bool)->Result<bool,RasterError> {
    if project.active_image_output(document)?.is_some() {return Ok(true);}
    Ok(!marked&&project.objects.values().any(|object| &object.document_id==document&&!object.state.hidden
        &&matches!(object.state.shape,Some(vw_proto::v1::object_state::Shape::Adjustment(_)))
        &&object.state.layer_id.as_ref().and_then(|wire|Id::from_proto(Some(wire)).ok()).and_then(|id|project.layers.get(&id))
            .is_some_and(|layer|layer.visible&&layer.opacity>0.0)))
}

/// Project-metadata admission before reading/decoding the original. This counts
/// the encoded original and worst-case 16-bit source storage in addition to the
/// renderer and output workspace. Actual decoded capacity is checked again.
pub fn document_output_reservation(project:&Project,document:&Id,options:RenderOptions)->Result<u64,RasterError> {
    let definition=&project.documents.get(document).ok_or(RasterError::Invalid("document"))?.definition;
    let asset=project.assets.get(&vw_model::AssetId::try_from(definition.primary_asset_id.clone())?).ok_or(RasterError::MissingAsset)?;
    if !matches!(asset.bit_depth,8|16){return Err(RasterError::Unsupported("image output source depth"));}
    let [w,h]=if asset.orientation>=5 {[asset.height,asset.width]}else{[asset.width,asset.height]};
    let [ow,oh]=project.active_image_output(document)?.map_or([w,h],|v|v.output_size());
    if w==0||h==0||u64::from(w)*u64::from(h)>50_000_000||u64::from(ow)*u64::from(oh)>50_000_000 {
        return Err(RasterError::Unsupported("image output pixel limit"));
    }
    let geometry=crate::stream::geometry_reserve(project,document,options.include_guides)?;
    let profile=crate::export::icc_workspace((!asset.icc_profile.is_empty()).then_some(&asset.icc_profile))?;
    u64::from(w).checked_mul(u64::from(h)).and_then(|v|v.checked_mul(72))
        .and_then(|v|v.checked_add(u64::from(ow)*u64::from(oh)*8))
        .and_then(|v|v.checked_add(asset.byte_size)).and_then(|v|v.checked_add(32*1024*1024))
        .and_then(|v|v.checked_add(geometry)).and_then(|v|v.checked_add(profile)).ok_or(RasterError::Allocation)
}

/// A completed output image. Canonical objects and the original remain in D;
/// this owned buffer alone uses the output grid described by `frame`.
pub struct DocumentOutput {
    pub image: DecodedImage,
    pub frame: Option<ImageOutputMetadata>,
}

/// Conservative nonallocating compositor admission, including the caller's
/// retained source capacity, complete D composition, output samples and ICC
/// workspace. Resolver images are additionally admitted by the shared renderer
/// before it asks the resolver to allocate. Full-resolution regions do not
/// authorize an implicit downsample or a preview-as-original fallback.
pub fn document_output_estimate(source:&DecodedImage, project:&Project,
    document:&Id, options:RenderOptions) -> Result<u64,RasterError> {
    crate::export::image_shape(source)?;
    if !source.original_available { return Err(RasterError::OriginalRequired); }
    let frame=project.active_image_output(document)?;
    if frame.is_some_and(|v|v.source_size()!=[source.width,source.height]) {
        return Err(RasterError::Invalid("document output source binding"));
    }
    let [w,h]=frame.map_or([source.width,source.height],|v|v.output_size());
    if u64::from(w)*u64::from(h)>50_000_000 {return Err(RasterError::Invalid("image output pixel limit"));}
    let geometry=crate::stream::geometry_reserve(project,document,options.include_guides)?;
    let profile=crate::export::icc_workspace(source.icc.as_ref())?;
    let retained=crate::image_resident_bytes(source)?;
    u64::from(source.width).checked_mul(u64::from(source.height))
        .and_then(|v|v.checked_mul(64))
        .and_then(|v|v.checked_add(u64::from(w)*u64::from(h)*8))
        .and_then(|v|v.checked_add(32*1024*1024))
        .and_then(|v|v.checked_add(geometry))
        .and_then(|v|v.checked_add(profile))
        .and_then(|v|v.checked_add(retained))
        .ok_or(RasterError::Allocation)
}

/// Compose complete D pixels first, then apply the one canonical output frame.
/// Clean output includes visible tonal adjustments but excludes annotation and
/// Result layers; marked output follows the normal layer compositor. No layer
/// named by display text receives special behavior. Ordinary Crop guides retain
/// their previous marked/guide behavior.
pub fn prepare_document_output(source:&DecodedImage, project:&Project, document:&Id,
    assets:&impl AssetResolver, options:RenderOptions, marked:bool,
    cancelled:&dyn Fn()->bool) -> Result<DocumentOutput,RasterError> {
    crate::source::check_cancel(cancelled)?;
    let estimated=document_output_estimate(source,project,document,options)?;
    crate::source::check_memory(estimated,options.memory_budget_bytes)?;
    // The admission above precedes all allocating semantic validation, full
    // image clones and ICC parsing in the shared compositor.
    let frame=project.active_image_output(document)?;
    let external=crate::image_resident_bytes(source)?;
    let available=options.memory_budget_bytes.checked_sub(external).ok_or(RasterError::Allocation)?;
    let mut render_options=options;
    render_options.memory_budget_bytes=available;
    let rendered=crate::render::render_document_mode(source,project,document,assets,
        render_options,marked,cancelled).map_err(|e|with_external(e,external,options.memory_budget_bytes))?;
    crate::source::check_cancel(cancelled)?;
    let image=if let Some(frame)=frame.filter(|v|!v.is_identity()) {
        crate::render_image_output(&rendered,frame,available,cancelled)
            .map_err(|e|with_external(e,external,options.memory_budget_bytes))?
    } else { rendered };
    crate::source::check_cancel(cancelled)?;
    Ok(DocumentOutput{image,frame:frame.map(ImageOutputMetadata::new)})
}

/// Export the canonical output grid. The request's optional region is in O
/// pixels, explicitly recorded alongside the D-to-O transform in the encoded
/// metadata. For an unconfigured document O is D, preserving legacy behavior.
/// Codec/alpha/depth/color permissions are exactly the ordinary export choices.
#[allow(clippy::too_many_arguments)] // Mirrors the compositor plus explicit encoding and cancellation policies.
pub fn export_document_output(source:&DecodedImage, project:&Project, document:&Id,
    assets:&impl AssetResolver, options:RenderOptions, marked:bool,
    request:&ExportRequest, cancelled:&dyn Fn()->bool) -> Result<ExportedImage,RasterError> {
    crate::source::check_cancel(cancelled)?;
    if request.memory_budget_bytes!=options.memory_budget_bytes {
        return Err(RasterError::Invalid("output compositor budget binding"));
    }
    let budget=request.memory_budget_bytes;
    let frame=project.active_image_output(document)?;
    let [w,h]=frame.map_or([source.width,source.height],|v|v.output_size());
    let external=crate::image_resident_bytes(source)?;
    let profile=crate::export::icc_workspace(source.icc.as_ref())?;
    let mut probe=request.clone();probe.memory_budget_bytes=u64::MAX;
    let codec=crate::preflight(w,h,source.pixels.bit_depth(),false,&probe)?;
    // A tiny selected export region still retains the complete O-grid buffer.
    let output_bytes=u64::from(w)*u64::from(h)*8;
    let export_peak=external.checked_add(output_bytes).and_then(|v|v.checked_add(codec)).and_then(|v|v.checked_add(profile))
        .ok_or(RasterError::Allocation)?;
    crate::source::check_memory(export_peak.max(document_output_estimate(source,project,document,options)?),budget)?;
    let output=prepare_document_output(source,project,document,assets,options,marked,cancelled)?;
    let external=external.checked_add(crate::image_resident_bytes(&output.image)?).ok_or(RasterError::Allocation)?;
    let mut encode_request=request.clone();
    encode_request.memory_budget_bytes=budget.checked_sub(external).ok_or(RasterError::Allocation)?;
    crate::source::check_cancel(cancelled)?;
    let encoded=crate::export::export_with_output(&output.image,&encode_request,output.frame)
        .map_err(|e|with_external(e,external,budget))?;
    // Keep the actual encoding settings intact. The recorded budget is the
    // encoder's residual budget; it must not falsely claim an RSS measurement.
    crate::source::check_cancel(cancelled)?;
    Ok(encoded)
}

fn with_external(error:RasterError,external:u64,budget:u64)->RasterError {
    match error {
        RasterError::Memory{estimated,..}=>RasterError::Memory{
            estimated:estimated.saturating_add(external),budget},
        other=>other,
    }
}
