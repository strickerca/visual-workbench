use super::*;
use vw_model::DeviceId;
use vw_raster::{AssetResolver, RasterError};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn referenced(actual: [u32; 2], declared: [u32; 2]) -> std::result::Result<(Project,BTreeMap<AssetId,Vec<u8>>,AssetId),Box<dyn std::error::Error>> {
    let mut encoded=Vec::new();
    {
        let mut encoder=png::Encoder::new(&mut encoded,actual[0],actual[1]);
        encoder.set_color(png::ColorType::Rgba);encoder.set_depth(png::BitDepth::Eight);
        let mut writer=encoder.write_header()?;
        writer.write_image_data(&[42u8,70,90,255].repeat(actual[0] as usize*actual[1] as usize))?;
        writer.finish()?;
    }
    let key=AssetId::hash(&encoded);
    let mut project=Project::new(Id::from_parts(1,[7;10])?,"synthetic".into(),DeviceId::from_bytes([5;16]));
    project.assets.insert(key.clone(),v1::AddAsset{asset_id:key.to_string(),format:"png".into(),
        width:declared[0],height:declared[1],bit_depth:8,orientation:1,byte_size:encoded.len() as u64,
        has_alpha:false,color_space:"sRGB".into(),source:"import".into(),captured_at_ms:1,metadata_json:"{}".into(),..Default::default()});
    Ok((project,BTreeMap::from([(key.clone(),encoded)]),key))
}

#[test]
fn production_resolver_refuses_understated_dimensions_before_pixel_decode() -> TestResult {
    let (project,originals,key)=referenced([128,128],[1,1])?;
    let resolver=Resolver{project:&project,originals:&originals,limits:DecodeLimits::default(),cancel:&crate::NeverCancel};
    // A truthful content hash cannot authorize 16,384 pixels against one declared
    // pixel. This is the real package resolver, not a replacement fake decoder.
    assert!(matches!(resolver.image(key.as_str()),Err(RasterError::Unsupported("image exceeds pixel limit; split or use tiled import"))));
    Ok(())
}
#[test]
fn production_resolver_requires_exact_shape_and_depth_even_with_equal_pixel_count() -> TestResult {
    let (mut project,originals,key)=referenced([3,2],[2,3])?;
    let resolve=|project:&Project|Resolver{project,originals:&originals,limits:DecodeLimits::default(),cancel:&crate::NeverCancel}.image(key.as_str());
    assert!(matches!(resolve(&project),Err(RasterError::MissingAsset)));
    let binding=project.assets.get_mut(&key).ok_or("asset")?;binding.width=3;binding.height=2;binding.bit_depth=16;
    assert!(matches!(resolve(&project),Err(RasterError::MissingAsset)));
    project.assets.get_mut(&key).ok_or("asset")?.bit_depth=8;
    assert_eq!((resolve(&project)?.width,resolve(&project)?.height),(3,2));
    Ok(())
}
#[test]
fn production_resolver_does_not_borrow_the_full_render_allowance() -> TestResult {
    let (project,originals,key)=referenced([1,1],[1,1])?;
    let reservation=32+32*1024*1024+originals.get(&key).ok_or("bytes")?.len() as u64;
    let resolver=Resolver{project:&project,originals:&originals,
        limits:DecodeLimits{max_memory_bytes:reservation-1,..Default::default()},cancel:&crate::NeverCancel};
    assert!(matches!(resolver.image(key.as_str()),Err(RasterError::Memory{estimated,budget}) if estimated==reservation && budget==reservation-1));
    Ok(())
}
