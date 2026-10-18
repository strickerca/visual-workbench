//! Typed nondestructive output and tonal settings over existing canonical ops.
mod plan;
mod pixels;
use crate::{Cancellation,ProjectSession,workflow::*};
use std::sync::Arc;
use vw_model::{AssetId,Id,Project};
use vw_proto::v1 as pb;
pub(crate) use pixels::legacy_export;
pub(crate) use pixels::admit_legacy_export;

const OUTPUT_WORK:u64=16*1024*1024;
const MAX_LAYERS:usize=256;
const MAX_OBJECTS:usize=4096;

#[derive(Clone,Debug,PartialEq,Eq,uniffi::Record)]
pub struct ImageOutputSettings {
    pub crop_x:u32,pub crop_y:u32,pub crop_width:u32,pub crop_height:u32,
    pub clockwise_degrees:u16,pub output_width:u32,pub output_height:u32,
}
impl ImageOutputSettings {
    fn frame(&self,source:[u32;2])->WorkflowResult<vw_geom::ImageOutputFrame> {
        if self.clockwise_degrees>=360 {return Err(WorkflowError::Invalid);}
        let result=vw_geom::ImageOutputFrame::new(source,[self.crop_x,self.crop_y,self.crop_width,self.crop_height],
            vw_geom::QuarterTurn::from_degrees(i32::from(self.clockwise_degrees))?,[self.output_width,self.output_height])?;
        if u64::from(self.output_width)*u64::from(self.output_height)>50_000_000 {return Err(WorkflowError::Limit);}
        Ok(result)
    }
    fn from_frame(frame:vw_geom::ImageOutputFrame)->Self {
        let v=vw_raster::ImageOutputMetadata::new(frame);
        Self{crop_x:v.crop.x,crop_y:v.crop.y,crop_width:v.crop.width,crop_height:v.crop.height,
            clockwise_degrees:v.clockwise_degrees,output_width:v.output_width,output_height:v.output_height}
    }
}
#[derive(Clone,Debug,PartialEq,uniffi::Record)]
pub struct ImageToneSettings {
    pub brightness:f64,pub contrast:f64,pub input_black:f64,pub input_white:f64,
    pub gamma:f64,pub output_black:f64,pub output_white:f64,
}
impl ImageToneSettings {
    fn wire(&self)->WorkflowResult<pb::Adjustment> {
        if ![self.brightness,self.contrast,self.input_black,self.input_white,self.gamma,self.output_black,self.output_white].iter().all(|v|v.is_finite())
            || !(-1.0..=1.0).contains(&self.brightness) || !(-1.0..=1.0).contains(&self.contrast)
            || ![self.input_black,self.input_white,self.output_black,self.output_white].iter().all(|v|(0.0..=1.0).contains(v))
            || self.input_black>=self.input_white || self.output_black>self.output_white || self.gamma<=0.0 {
            return Err(WorkflowError::Invalid);
        }
        Ok(pb::Adjustment{brightness:self.brightness,contrast:self.contrast,
            levels:vec![self.input_black,self.input_white,self.gamma,self.output_black,self.output_white]})
    }
    fn from_wire(v:&pb::Adjustment)->WorkflowResult<Self> {
        if v.levels.len()!=5 {return Err(WorkflowError::Invalid);}
        let result=Self{brightness:v.brightness,contrast:v.contrast,input_black:v.levels[0],input_white:v.levels[1],gamma:v.levels[2],output_black:v.levels[3],output_white:v.levels[4]};
        result.wire()?;Ok(result)
    }
}
#[derive(Clone,Debug,uniffi::Record)]
pub struct ImageOutputEntry {pub layer_id:String,pub object_id:String,pub enabled:bool,pub locked:bool,pub settings:ImageOutputSettings}
#[derive(Clone,Debug,uniffi::Record)]
pub struct ImageToneEntry {pub layer_id:String,pub object_id:String,pub enabled:bool,pub opacity:f64,pub locked:bool,pub settings:ImageToneSettings}
#[derive(Clone,Debug,uniffi::Record)]
pub struct ImageOutputSnapshot {
    pub binding:WorkflowBinding,pub source_asset_id:String,pub source_width:u32,pub source_height:u32,pub source_bit_depth:u8,
    pub output:Option<ImageOutputEntry>,pub tones:Vec<ImageToneEntry>,
}
#[derive(Clone,Debug,uniffi::Enum)]
pub enum ImageOutputCommand {
    SetFrame{layer_id:String,object_id:String,settings:ImageOutputSettings,enabled:bool},
    RemoveFrame{layer_id:String,object_id:String},
    SetTone{layer_id:String,object_id:String,settings:ImageToneSettings,enabled:bool,opacity:f64},
    RemoveTone{layer_id:String,object_id:String},
}
#[derive(Clone,Debug,uniffi::Record)]
pub struct ImageOutputPreview {
    pub binding:WorkflowBinding,pub source_asset_id:String,pub output:Option<ImageOutputSettings>,
    pub output_width:u32,pub output_height:u32,pub x:u32,pub y:u32,pub width:u32,pub height:u32,
    /// Explicit sRGB RGBA8 DISPLAY pixels. This is never an original/export asset.
    pub rgba:Vec<u8>,
}

fn id(value:&str)->WorkflowResult<Id> {if value.len()!=36 {return Err(WorkflowError::Invalid);}Ok(Id::try_from(value.to_owned())?)}
fn bounds<'a>(project:&'a Project,document:&Id)->WorkflowResult<&'a pb::AddAsset> {
    let definition=&project.documents.get(document).ok_or(WorkflowError::Missing)?.definition;
    if !matches!(pb::DocumentKind::try_from(definition.kind),Ok(pb::DocumentKind::Image|pb::DocumentKind::Capture)) {return Err(WorkflowError::Invalid);}
    if project.layers.values().filter(|v|v.definition.document_id.as_ref()==Some(&document.to_proto())).count()>MAX_LAYERS
        || project.objects.values().filter(|v|&v.document_id==document).count()>MAX_OBJECTS {return Err(WorkflowError::Limit);}
    let asset=project.assets.get(&AssetId::try_from(definition.primary_asset_id.clone())?).ok_or(WorkflowError::Missing)?;
    if !matches!(asset.bit_depth,8|16) {return Err(WorkflowError::Invalid);}
    Ok(asset)
}
fn dimensions(asset:&pb::AddAsset)->[u32;2] {if asset.orientation>=5 {[asset.height,asset.width]}else{[asset.width,asset.height]}}
fn snapshot(state:&crate::project::ProjectState,document:&Id,cancel:&Cancellation)->WorkflowResult<ImageOutputSnapshot> {
    let project=state.project()?;let asset=bounds(project,document)?;let [source_width,source_height]=dimensions(asset);
    let output=project.image_output(document)?.map(|v| ImageOutputEntry{layer_id:v.layer_id.to_string(),object_id:v.object_id.to_string(),
        enabled:v.enabled,locked:project.layers[v.layer_id].locked||project.objects[v.object_id].state.locked,settings:ImageOutputSettings::from_frame(v.frame)});
    let mut tones=Vec::new();
    let mut ordered=project.layers.iter().filter(|(_,v)|v.definition.document_id.as_ref()==Some(&document.to_proto())).collect::<Vec<_>>();
    ordered.sort_by(|(a,x),(b,y)|x.definition.order_key.cmp(&y.definition.order_key).then(a.cmp(b)));
    for (layer_id,layer) in ordered {
        cancel.check()?;
        for (object_id,object) in project.objects.iter().filter(|(_,v)|v.state.layer_id.as_ref()==Some(&layer_id.to_proto())) {
            if let Some(pb::object_state::Shape::Adjustment(value))=&object.state.shape {
                plan::tone(project,document,layer_id,object_id,false)?;
                tones.push(ImageToneEntry{layer_id:layer_id.to_string(),object_id:object_id.to_string(),enabled:layer.visible,
                    opacity:layer.opacity,locked:layer.locked||object.state.locked,settings:ImageToneSettings::from_wire(value)?});
            }
        }
    }
    Ok(ImageOutputSnapshot{binding:current(state,document.as_str())?,source_asset_id:asset.asset_id.clone(),source_width,source_height,
        source_bit_depth:asset.bit_depth as u8,output,tones})
}

#[uniffi::export]
impl ProjectSession {
    pub async fn image_output_snapshot(&self,document_id:String,memory_budget_bytes:u64,cancellation:Arc<Cancellation>)->WorkflowResult<ImageOutputSnapshot> {
        self.check_open()?;let document=id(&document_id)?;let closed=self.closed.clone();
        self.worker.call(move |state|Ok((||{check_live(&closed,&cancellation)?;admit(state,memory_budget_bytes,false,OUTPUT_WORK)?;
            snapshot(state,&document,&cancellation)})())).await?
    }
    pub async fn prepare_image_output(self:Arc<Self>,binding:WorkflowBinding,metadata:WorkflowMetadata,command:ImageOutputCommand,
        memory_budget_bytes:u64,cancellation:Arc<Cancellation>)->WorkflowResult<Arc<WorkflowPlan>> {
        self.check_open()?;binding.validate()?;metadata.validate()?;plan::validate(&command)?;cancellation.check()?;
        let permit=HandlePermit::acquire()?;let owner=self.clone();
        self.worker.call(move|state|Ok((||{
            check_live(&owner.closed,&cancellation)?;admit(state,memory_budget_bytes,true,OUTPUT_WORK)?;
            check_binding(state,&binding)?;check_metadata(state,&metadata)?;
            let document=id(&binding.document_id)?;bounds(state.project()?,&document)?;
            let (txn,next)=plan::transaction(state,&document,&metadata,command)?;
            admit_transaction(state,&txn,memory_budget_bytes,OUTPUT_WORK)?;
            if let Some(replica)=&state.replica {let mut staged=replica.clone();staged.queue(txn.clone()).map_err(plan::operation_error)?;}
            else {state.store()?.validate_transaction(&txn,&vw_model::DeviceId::try_from(metadata.device_id.clone())?,metadata.created_at_ms)
                .map_err(|e|match e {vw_store::StoreError::Operation(e)=>plan::operation_error(e),_=>WorkflowError::Storage})?;}
            check_live(&owner.closed,&cancellation)?;
            WorkflowPlan::new(&owner,state,binding,txn,next,memory_budget_bytes,OUTPUT_WORK,permit)
        })())).await?
    }
    pub async fn image_output_preview(&self,binding:WorkflowBinding,region:crate::SelectionRegion,marked:bool,
        assume_untagged_srgb:bool,memory_budget_bytes:u64,cancellation:Arc<Cancellation>)->WorkflowResult<ImageOutputPreview> {
        self.check_open()?;binding.validate()?;let closed=self.closed.clone();
        let crate::SelectionRegion{x,y,width,height}=region;
        if width==0||height==0||width>512||height>512 {return Err(WorkflowError::Limit);}
        self.worker.call(move|state|Ok((||{
            check_live(&closed,&cancellation)?;
            pixels::preview(state,binding,marked,vw_raster::Region{x,y,width,height},assume_untagged_srgb,memory_budget_bytes,&cancellation)
        })())).await?
    }
}
