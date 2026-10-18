//! Explicit output metadata. Ordinary Crop objects retain guide semantics.
use crate::{AssetId,Id,ModelError,Project};
use vw_geom::{Affine,ImageOutputFrame,Rect};
use vw_proto::v1;

/// Older model validators reject this layer kind instead of ignoring output
/// settings. It is not inferred from a mutable display name or object title.
pub const IMAGE_OUTPUT_LAYER_KIND:&str="output";
/// Required application session capability before syncing this convention.
pub const IMAGE_OUTPUT_CAPABILITY:&str="image_output_frame_v1";

/// Borrowed canonical identity and the independently validated output grid.
#[derive(Debug,Clone,Copy)]
pub struct DocumentImageOutput<'a> {
    pub layer_id:&'a Id,
    pub object_id:&'a Id,
    pub frame:ImageOutputFrame,
    /// Only layer visibility enables/disables the output. Hidden objects,
    /// fractional opacity, blend modes and group transforms are ambiguous.
    pub enabled:bool,
}
impl Project {
    /// Read the single explicit image-output frame, including disabled settings.
    /// Every other Crop object remains an editor guide. Multiple/malformed
    /// frames refuse even when one of their layers is currently invisible.
    pub fn image_output(&self, document:&Id)->Result<Option<DocumentImageOutput<'_>>,ModelError> {
        let wire=document.to_proto();
        let mut found=None;
        for (layer_id,layer) in &self.layers {
            if layer.definition.kind!=IMAGE_OUTPUT_LAYER_KIND || layer.definition.document_id.as_ref()!=Some(&wire) {continue;}
            if found.is_some() {return Err(ModelError::Invalid("multiple image output frames"));}
            if layer.opacity!=1.0 || layer.blend!="normal" || layer.definition.page_index!=-1 {
                return Err(ModelError::Invalid("image output layer"));
            }
            let definition=&self.documents.get(document).ok_or(ModelError::Missing("image output document"))?.definition;
            if !matches!(v1::DocumentKind::try_from(definition.kind),Ok(v1::DocumentKind::Image|v1::DocumentKind::Capture)) {
                return Err(ModelError::Invalid("image output document kind"));
            }
            let source=self.assets.get(&AssetId::try_from(definition.primary_asset_id.clone())?).ok_or(ModelError::Missing("image output source"))?;
            let dimensions=if source.orientation>=5 {[source.height,source.width]}else{[source.width,source.height]};
            let layer_wire=layer_id.to_proto();let mut object=None;
            for (id,value) in &self.objects {
                if value.state.layer_id.as_ref()!=Some(&layer_wire) {continue;}
                if object.is_some() || &value.document_id!=document {return Err(ModelError::Invalid("image output object count"));}
                object=Some((id,value));
            }
            let (id,object)=object.ok_or(ModelError::Missing("image output object"))?;
            let state=&object.state;
            if state.hidden || state.group_id.is_some() || state.role!=v1::Role::None as i32 {
                return Err(ModelError::Invalid("image output authority"));
            }
            let Some(v1::object_state::Shape::Crop(crop))=&state.shape else{return Err(ModelError::Invalid("image output crop"));};
            let affine=state.transform.as_ref().ok_or(ModelError::Missing("image output transform"))?;
            let frame=ImageOutputFrame::from_crop_affine(dimensions,Rect::new(crop.x,crop.y,crop.w,crop.h)?,
                Affine::new(affine.a,affine.b,affine.c,affine.d,affine.e,affine.f)?)?;
            found=Some(DocumentImageOutput{layer_id,object_id:id,frame,enabled:layer.visible});
        }
        Ok(found)
    }
    /// Whether the canonical document has active global output geometry.
    pub fn active_image_output(&self,document:&Id)->Result<Option<ImageOutputFrame>,ModelError> {
        Ok(self.image_output(document)?.filter(|v|v.enabled).map(|v|v.frame))
    }
    /// Validate only explicit output layers. Legacy documents do not acquire a
    /// new source-size restriction merely because the software was upgraded.
    pub(crate) fn validate_image_outputs(&self)->Result<(),ModelError> {
        for layer in self.layers.values().filter(|v|v.definition.kind==IMAGE_OUTPUT_LAYER_KIND) {
            let document=Id::from_proto(layer.definition.document_id.as_ref())?;
            self.image_output(&document)?;
        }
        Ok(())
    }
}
