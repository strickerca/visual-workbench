mod common;
use common::*;
use vw_geom::{ImageOutputFrame,QuarterTurn};
use vw_model::{Layer,Object};
use vw_ops::HostSequencer;
use vw_package::*;
use vw_proto::v1::{self as pb,object_state::Shape};

fn output(f:&mut Fixture,crop:[u32;4],size:[u32;2])->TestResult {
    let mut project=f.host.project().clone();let source=project.assets.values().next().ok_or("asset")?;
    let frame=ImageOutputFrame::new([source.width,source.height],crop,QuarterTurn::Clockwise90,size)?;
    let layer=id(60)?;let obj=id(61)?;let [a,b,c,d,e,z]=frame.matrix()?.coefficients();
    project.layers.insert(layer.clone(),Layer{definition:pb::CreateLayer{layer_id:Some(layer.to_proto()),
        document_id:Some(f.document.to_proto()),page_index:-1,name:"Output".into(),kind:"output".into(),order_key:"z".into()},
        visible:true,locked:false,opacity:1.0,blend:"normal".into()});
    project.objects.insert(obj.clone(),Object{document_id:f.document.clone(),state:pb::ObjectState{object_id:Some(obj.to_proto()),
        layer_id:Some(layer.to_proto()),order_key:"V".into(),transform:Some(pb::Affine{a,b,c,d,e,f:z}),
        style:Some(pb::Style{stroke:Some(pb::Color{rgba:0}),width:0.0,..Default::default()}),role:pb::Role::None as i32,
        created_by:device().to_string(),created_at_ms:1,shape:Some(Shape::Crop(pb::RectD{x:crop[0].into(),y:crop[1].into(),w:crop[2].into(),h:crop[3].into()})),..Default::default()}});
    f.host=HostSequencer::new(project,device())?;Ok(())
}
#[test]
fn marker_box_point_crop_and_images_share_the_explicit_output_mapping()->TestResult {
    let mut f=fixture(128,96,8)?;let originals=f.originals.clone();
    output(&mut f,[16,8,80,64],[128,160])?;
    marker(&mut f,Some([26.0,18.0,20.0,10.0]),[36.0,23.0],"Change the bounded target",vec![])?;
    let package=compiled(&f,options()?)?;let manifest=package.manifest();
    assert_eq!(manifest.extensions.compiler,"vw-package-1-output-bilinear-v1");
    let map=&manifest.extensions.overview_mapping;
    assert_eq!((map.content_width,map.content_height),(128,160));
    // D -> crop local: [10,10,20,10]. CW90 -> [44,10,10,20].
    // Independent 2× axes -> [88,20,20,40], point [98,40].
    near(&manifest.markers[0].bbox_compiled,&[88.0,20.0,20.0,40.0]);
    near(&manifest.markers[0].point_compiled,&[98.0,40.0]);
    near(&manifest.markers[0].bbox_document,&[26.0,18.0,20.0,10.0]);
    assert_eq!(manifest.extensions.crops[0].mapping.output_frame,map.output_frame);
    for image in &manifest.images {
        let bytes=package.files().find(|(path,_)|*path==image.path).ok_or("image")?.1;
        let pixels=vw_raster::decode(bytes,vw_raster::DecodeLimits::default())?;
        assert_eq!((pixels.width,pixels.height),(image.width,image.height));
    }
    let prompt=package.files().find(|(path,_)|*path=="prompt.md").ok_or("prompt")?.1;
    assert!(std::str::from_utf8(prompt)?.contains("per O (output-grid) pixel"));
    Package::from_files(files(&package),Limits::default(),&NeverCancel)?;
    assert_eq!(f.originals,originals);Ok(())
}
#[test]
fn output_crop_refuses_instruction_targets_cut_by_the_frame()->TestResult {
    let mut f=fixture(128,96,8)?;output(&mut f,[16,8,80,64],[64,80])?;
    marker(&mut f,Some([10.0,18.0,20.0,10.0]),[25.0,23.0],"Not wholly inside",vec![])?;
    assert!(matches!(compile(CompileInput{project:f.host.project(),revision:&f.host.revision()?,document:&f.document,originals:&f.originals},options()?,&NeverCancel),Err(Error::Unsupported(_))));Ok(())
}
#[test]
fn cropped_package_refuses_unbounded_region_targets_instead_of_losing_the_instruction()->TestResult {
    let mut f=fixture(128,96,8)?;output(&mut f,[16,8,80,64],[64,80])?;
    marker(&mut f,Some([26.0,18.0,20.0,10.0]),[36.0,23.0],"Also references a region",vec![])?;
    let mut project=f.host.project().clone();let target=id(62)?;
    let mut region=project.objects.get(&id(10)?).ok_or("marker")?.clone();
    region.state.object_id=Some(target.to_proto());region.state.shape=Some(Shape::Rect(pb::RectD{x:0.0,y:0.0,w:8.0,h:8.0}));
    project.objects.insert(target.clone(),region);
    project.instructions.get_mut(&id(20)?).ok_or("instruction")?.definition.target_object_ids.push(target.to_proto());
    f.host=HostSequencer::new(project,device())?;
    assert!(matches!(compile(CompileInput{project:f.host.project(),revision:&f.host.revision()?,document:&f.document,originals:&f.originals},options()?,&NeverCancel),Err(Error::Unsupported(_))));Ok(())
}
#[test]
fn importer_refuses_output_metadata_tampering_even_with_well_formed_geometry()->TestResult {
    let mut f=fixture(128,96,8)?;output(&mut f,[16,8,80,64],[64,80])?;
    let package=compiled(&f,options()?)?;
    for field in ["source_width","output_width","clockwise_degrees"] {
        let mut entries=files(&package);let bytes=&mut entries.iter_mut().find(|(p,_)|p=="manifest.json").ok_or("manifest")?.1;
        let mut value:serde_json::Value=serde_json::from_slice(bytes)?;
        value["extensions"]["overview_mapping"]["output_frame"][field]=serde_json::json!(17);
        *bytes=serde_json::to_vec(&value)?;
        assert!(Package::from_files(entries,Limits::default(),&NeverCancel).is_err());
    }Ok(())
}
#[test]
fn legacy_packages_keep_old_mapping_and_depth_permission_stays_explicit()->TestResult {
    let legacy=compiled(&fixture(8,8,8)?,options()?)?;
    assert!(legacy.manifest().extensions.overview_mapping.output_frame.is_none());
    let json=serde_json::to_string(&legacy.manifest().extensions.overview_mapping)?;
    assert!(!json.contains("output_frame"));assert_eq!(legacy.manifest().extensions.compiler,"vw-package-1-area-integer");
    let mut high=fixture(32,24,16)?;output(&mut high,[0,0,32,24],[24,32])?;
    assert!(compiled(&high,options()?).is_err());let mut permitted=options()?;permitted.allow_depth_reduction=true;
    let result=compiled(&high,permitted)?;assert_eq!(result.manifest().source.width,32);
    assert_eq!(result.manifest().extensions.overview_mapping.content_width,24);Ok(())
}
