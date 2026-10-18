#![allow(clippy::unwrap_used,clippy::expect_used)]
use vw_geom::{ImageOutputFrame,QuarterTurn};
use vw_model::AssetId;
use vw_raster::*;
const BUDGET:u64=256*1024*1024;
fn source(width:u32,height:u32,depth:u8)->DecodedImage {
    let samples:Vec<u16>=(0..width*height).flat_map(|i|[i as u16*113,(i as u16+1)*257,65535-i as u16*71,if i%3==0{0}else{32768}]).collect();
    DecodedImage{width,height,pixels:if depth==16 {Pixels::Rgba16(samples)}else{Pixels::Rgba8(samples.iter().map(|v|(v/257) as u8).collect())},
        icc:Some(standard_srgb_profile().unwrap()),source_asset:AssetId::hash(b"synthetic-original"),original_available:true,orientation_applied:6}
}
fn pixel(pixels:&Pixels,at:usize)->[u16;4] {
    match pixels {Pixels::Rgba8(v)=>[u16::from(v[at])*257,u16::from(v[at+1])*257,u16::from(v[at+2])*257,u16::from(v[at+3])*257],
        Pixels::Rgba16(v)=>[v[at],v[at+1],v[at+2],v[at+3]]}
}
#[test]
fn exact_crop_and_every_quarter_turn_preserve_depth_alpha_hidden_rgb_profile_and_identity() {
    for depth in [8,16] {for rotation in [QuarterTurn::Zero,QuarterTurn::Clockwise90,QuarterTurn::Half,QuarterTurn::Clockwise270] {
        let original=source(5,4,depth);let before=original.clone();
        let size=if matches!(rotation,QuarterTurn::Zero|QuarterTurn::Half){[3,2]}else{[2,3]};
        let frame=ImageOutputFrame::new([5,4],[1,1,3,2],rotation,size).unwrap();
        let result=render_image_output(&original,frame,BUDGET,&||false).unwrap();
        assert_eq!((result.width,result.height),(size[0],size[1]));assert_eq!(result.pixels.bit_depth(),depth);
        for y in 0..size[1] {for x in 0..size[0] {
            let [sx,sy]=frame.source_pixel(x,y).unwrap();
            assert_eq!(pixel(&result.pixels,((y*size[0]+x)*4) as usize),pixel(&original.pixels,((sy*5+sx)*4) as usize));
        }}
        assert_eq!(result.icc,original.icc);assert_eq!(result.source_asset,original.source_asset);assert_eq!(result.orientation_applied,6);
        assert_eq!(original,before);
    }}
}
#[test]
fn source_sample_bilinear_uses_premultiplied_alpha_and_ignores_hidden_color() {
    let mut original=source(2,1,8);original.pixels=Pixels::Rgba8(vec![255,0,0,0,0,0,255,255]);
    let frame=ImageOutputFrame::new([2,1],[0,0,2,1],QuarterTurn::Zero,[3,1]).unwrap();
    let result=render_image_output(&original,frame,BUDGET,&||false).unwrap();
    assert_eq!(result.pixels,Pixels::Rgba8(vec![255,0,0,0,0,0,255,128,0,0,255,255]));
    // Exact center downsampling is explicitly bilinear, not an area average.
    let one=ImageOutputFrame::new([2,1],[0,0,2,1],QuarterTurn::Zero,[1,1]).unwrap();
    assert_eq!(render_image_output(&original,one,BUDGET,&||false).unwrap().pixels,Pixels::Rgba8(vec![0,0,255,128]));
}
#[test]
fn sixteen_bit_resizing_does_not_quantize_adjacent_source_values_to_eight_bits() {
    let mut original=source(2,1,16);original.pixels=Pixels::Rgba16(vec![1000,2000,3000,65535,1002,2002,3002,65535]);
    let frame=ImageOutputFrame::new([2,1],[0,0,2,1],QuarterTurn::Zero,[1,1]).unwrap();
    assert_eq!(render_image_output(&original,frame,BUDGET,&||false).unwrap().pixels,Pixels::Rgba16(vec![1001,2001,3001,65535]));
}
#[test]
fn every_region_uses_global_output_pixel_centers_and_stitches_exactly() {
    let original=source(7,5,16);let frame=ImageOutputFrame::new([7,5],[1,1,5,3],QuarterTurn::Clockwise270,[11,13]).unwrap();
    let complete=render_image_output(&original,frame,BUDGET,&||false).unwrap();
    for y in 0..13 {for x in 0..11 {
        let part=render_image_output_region(&original,frame,Region{x,y,width:1,height:1},BUDGET,&||false).unwrap();
        assert_eq!(pixel(&part.pixels,0),pixel(&complete.pixels,((y*11+x)*4) as usize));
    }}
}
#[test]
fn retained_capacity_profile_workspace_invalid_source_and_cancellation_refuse_without_output() {
    let mut original=source(2,2,8);let frame=ImageOutputFrame::identity(2,2).unwrap();
    let region=Region{x:0,y:0,width:2,height:2};
    let before=image_output_estimate(&original,frame,region).unwrap();
    if let Pixels::Rgba8(bytes)=&mut original.pixels {bytes.reserve_exact(4096);}
    let after=image_output_estimate(&original,frame,region).unwrap();assert!(after>=before+4096);
    assert!(matches!(render_image_output(&original,frame,after-1,&||false),Err(RasterError::Memory{..})));
    assert!(matches!(render_image_output(&original,frame,BUDGET,&||true),Err(RasterError::Cancelled)));
    original.original_available=false;
    assert!(matches!(render_image_output(&original,frame,BUDGET,&||false),Err(RasterError::OriginalRequired)));
}
#[test]
fn output_metadata_roundtrips_exact_geometry_and_refuses_unknown_policy_version() {
    let frame=ImageOutputFrame::new([9,8],[2,3,4,5],QuarterTurn::Half,[7,11]).unwrap();
    let metadata=ImageOutputMetadata::new(frame);assert_eq!(metadata.frame().unwrap(),frame);
    let bytes=serde_json::to_vec(&metadata).unwrap();let copy:ImageOutputMetadata=serde_json::from_slice(&bytes).unwrap();assert_eq!(copy,metadata);
    assert!(ImageOutputMetadata{version:2,..metadata}.frame().is_err());
    assert!(ImageOutputMetadata{clockwise_degrees:360,..metadata}.frame().is_err());
}
