//! Shared preview/export/package compositor regressions; source only until the
//! central validation lane runs them.
mod support;
use support::*;
use vw_geom::{ImageOutputFrame, QuarterTurn};
use vw_model::{Id, Layer, Project};
use vw_proto::v1::{self as pb, object_state::Shape};
use vw_raster::*;

fn output(project:&mut Project,document:&Id,frame:ImageOutputFrame)->TestResult {
    let layer=id(70)?;
    project.layers.insert(layer.clone(),Layer{definition:pb::CreateLayer{layer_id:Some(layer.to_proto()),
        document_id:Some(document.to_proto()),page_index:-1,name:"Explicit output".into(),kind:"output".into(),order_key:"z".into()},
        visible:true,locked:false,opacity:1.0,blend:"normal".into()});
    let [x,y,w,h]=frame.crop();let [a,b,c,d,e,f]=frame.matrix()?.coefficients();
    let (key,mut value)=object(document,&layer,71,Shape::Crop(rect(x.into(),y.into(),w.into(),h.into())))?;
    value.state.transform=Some(pb::Affine{a,b,c,d,e,f});project.objects.insert(key,value);project.validate()?;Ok(())
}
fn tone(project:&mut Project,document:&Id)->TestResult {
    let layer=id(72)?;
    project.layers.insert(layer.clone(),Layer{definition:pb::CreateLayer{layer_id:Some(layer.to_proto()),
        document_id:Some(document.to_proto()),page_index:-1,name:"Levels".into(),kind:"adjustment".into(),order_key:"Y".into()},
        visible:true,locked:false,opacity:1.0,blend:"normal".into()});
    let (key,value)=object(document,&layer,73,Shape::Adjustment(pb::Adjustment{brightness:0.1,contrast:0.0,levels:vec![0.0,1.0,1.0,0.0,1.0]}))?;
    project.objects.insert(key,value);project.validate()?;Ok(())
}
#[test]
fn sixteen_bit_alpha_output_preview_and_encoded_export_use_exactly_one_mapping()->TestResult {
    let original=source(8,6,16);let original_hash=original.source_asset.clone();
    let (mut project,doc,_)=project(&original)?;
    let frame=ImageOutputFrame::new([8,6],[2,1,4,3],QuarterTurn::Clockwise90,[3,4])?;output(&mut project,&doc,frame)?;
    let direct=render_image_output(&original,frame,256*1024*1024,&||false)?;
    let mut opts=options();opts.include_guides=true;
    let actual=prepare_document_output(&original,&project,&doc,&NoAssets,opts,true,&||false)?;
    assert_pixels_equal(&actual.image.pixels,&direct.pixels,"output layer is not painted as a crop guide")?;
    let mut request=request(ExportFormat::Png16);request.memory_budget_bytes=opts.memory_budget_bytes;
    let encoded=export_document_output(&original,&project,&doc,&NoAssets,opts,true,&request,&||false)?;
    let decoded=decode(&encoded.bytes,DecodeLimits::default())?;
    assert_pixels_equal(&decoded.pixels,&direct.pixels,"encoded output uses the same exact quarter turn")?;
    assert_eq!((decoded.width,decoded.height),(3,4));assert_eq!(encoded.metadata.source_width,8);assert_eq!(encoded.metadata.source_height,6);
    assert_eq!(encoded.metadata.image_output,Some(ImageOutputMetadata::new(frame)));
    assert_eq!(original.source_asset,original_hash);assert_pixels_equal(&original.pixels,&source(8,6,16).pixels,"immutable original")?;
    Ok(())
}
#[test]
fn region_coordinates_are_in_output_grid_and_metadata_keeps_original_grid()->TestResult {
    let original=source(8,6,8);let (mut project,doc,_)=project(&original)?;
    let frame=ImageOutputFrame::new([8,6],[2,1,4,3],QuarterTurn::Clockwise90,[6,8])?;output(&mut project,&doc,frame)?;
    let region=Region{x:2,y:3,width:3,height:4};
    let expected=render_image_output_region(&original,frame,region,256*1024*1024,&||false)?;
    let opts=options();let mut request=request(ExportFormat::Png8);request.memory_budget_bytes=opts.memory_budget_bytes;request.region=Some(region);
    let actual=export_document_output(&original,&project,&doc,&NoAssets,opts,false,&request,&||false)?;
    assert_pixels_equal(&decode(&actual.bytes,DecodeLimits::default())?.pixels,&expected.pixels,"O-grid region")?;
    assert_eq!(actual.metadata.settings.region,Some(region));assert_eq!(actual.metadata.image_output,Some(ImageOutputMetadata::new(frame)));Ok(())
}
#[test]
fn clean_tonal_output_is_adjusted_and_annotation_free()->TestResult {
    let original=source(32,24,8);let (mut project,doc,layer)=project(&original)?;tone(&mut project,&doc)?;
    assert!(requires_document_output(&project,&doc,false)?);
    let clean=prepare_document_output(&original,&project,&doc,&NoAssets,options(),false,&||false)?;
    let expected=render_document(&original,&project,&doc,&NoAssets,options())?;
    assert_pixels_equal(&clean.image.pixels,&expected.pixels,"tone is included in clean output")?;
    let (key,value)=object(&doc,&layer,20,Shape::Rect(rect(1.0,1.0,15.0,15.0)))?;project.objects.insert(key,value);
    let clean_with_marks=prepare_document_output(&original,&project,&doc,&NoAssets,options(),false,&||false)?;
    assert_pixels_equal(&clean_with_marks.image.pixels,&clean.image.pixels,"annotation excluded")?;
    let marked=prepare_document_output(&original,&project,&doc,&NoAssets,options(),true,&||false)?;
    assert_ne!(marked.image.pixels.rgba8(),clean.image.pixels.rgba8());Ok(())
}
#[test]
fn ordinary_crop_guides_and_disabled_frames_keep_document_dimensions()->TestResult {
    let original=source(24,16,8);let (mut project,doc,layer)=project(&original)?;
    let (key,value)=object(&doc,&layer,20,Shape::Crop(rect(2.0,2.0,12.0,8.0)))?;project.objects.insert(key,value);
    assert!(!requires_document_output(&project,&doc,false)?);
    let ordinary=prepare_document_output(&original,&project,&doc,&NoAssets,options(),false,&||false)?;
    assert_eq!((ordinary.image.width,ordinary.image.height),(24,16));assert!(ordinary.frame.is_none());
    let frame=ImageOutputFrame::new([24,16],[2,2,12,8],QuarterTurn::Half,[6,4])?;output(&mut project,&doc,frame)?;
    project.layers.get_mut(&id(70)?).ok_or("layer")?.visible=false;
    let disabled=prepare_document_output(&original,&project,&doc,&NoAssets,options(),false,&||false)?;
    assert_eq!((disabled.image.width,disabled.image.height),(24,16));assert!(disabled.frame.is_none());
    assert_pixels_equal(&disabled.image.pixels,&ordinary.image.pixels,"disabled output")?;Ok(())
}
#[test]
fn admission_and_cancellation_precede_full_composition_and_keep_source()->TestResult {
    let original=source(8,6,16);let (mut project,doc,_)=project(&original)?;
    output(&mut project,&doc,ImageOutputFrame::new([8,6],[0,0,8,6],QuarterTurn::Zero,[1000,1000])?)?;
    assert!(matches!(prepare_document_output(&original,&project,&doc,&NoAssets,RenderOptions{memory_budget_bytes:1024,..options()},false,&||false),Err(RasterError::Memory{..})));
    assert!(matches!(prepare_document_output(&original,&project,&doc,&NoAssets,options(),false,&||true),Err(RasterError::Cancelled)));
    let mut request=request(ExportFormat::Png8);request.memory_budget_bytes=options().memory_budget_bytes;
    assert!(matches!(export_document_output(&original,&project,&doc,&NoAssets,options(),false,&request,&||false),Err(RasterError::Depth)));
    assert_pixels_equal(&original.pixels,&source(8,6,16).pixels,"refusal original")?;Ok(())
}
