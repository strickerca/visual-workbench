use super::{hex,source};
use crate::{Cancellation,CoreError,Result};
use std::{collections::BTreeSet,io::Read,path::Path};
use vw_assets::{Manifest,Source,TileKey,TileRecord,TileRegion,MAX_ENCODED_TILE_BYTES,MAX_MANIFEST_BYTES};
use vw_store::DiskCache;
pub(super) fn manifest_key(source:&Source)->String{format!("giant-v1-{}-manifest",hex(&source.original))}
pub(super) fn tile_key(source:&Source,key:TileKey)->String{format!("giant-v1-{}-{}-{}-{}",hex(&source.original),key.level,key.x,key.y)}
fn bytes(path:&Path,max:usize)->Result<Vec<u8>>{
    let file=source::regular(path)?;let size=file.metadata()?.len();if size==0||size>max as u64{return Err(CoreError::Backpressure)}
    let mut bytes=Vec::with_capacity(size as usize);file.take(max as u64+1).read_to_end(&mut bytes)?;if bytes.len()!=size as usize{return Err(CoreError::Invalid)}Ok(bytes)
}
pub(super) fn inventory(root:&Path,source:&Source,budget:u64,cancel:&Cancellation)->Result<Manifest>{
    let pyramid=source.validate().map_err(|_|CoreError::Invalid)?;let mut expected:BTreeSet<String>=pyramid.levels().iter().map(|l|format!("L{}",l.index)).collect();
    for entry in std::fs::read_dir(root)?{cancel.check()?;let entry=entry?;let name=entry.file_name().into_string().map_err(|_|CoreError::Invalid)?;
        if expected.remove(&name){source::directory(entry.path().to_str().ok_or(CoreError::Invalid)?)?;}
        else if ["pyramid.vwt","preview.jpg",".owner"].contains(&name.as_str()){
            let file=source::regular(&entry.path())?;let limit=match name.as_str(){".owner"=>4096,"preview.jpg"=>16*1024*1024,_=>MAX_MANIFEST_BYTES};if file.metadata()?.len()>limit as u64{return Err(CoreError::Backpressure)}
        }else{return Err(CoreError::Invalid)}
    }
    if !expected.is_empty(){return Err(CoreError::Invalid)}
    for level in pyramid.levels(){let expected=u64::from(level.columns)*u64::from(level.rows);let mut count=0u64;for entry in std::fs::read_dir(root.join(format!("L{}",level.index)))?{cancel.check()?;let entry=entry?;let _file=source::regular(&entry.path())?;count+=1;if count>expected{return Err(CoreError::Invalid)}}if count!=expected{return Err(CoreError::Invalid)}}
    let mut records=Vec::with_capacity(pyramid.tile_count());let mut total=0u64;
    for key in pyramid.keys(){cancel.check()?;let mut record=TileRecord{key,encoded_bytes:0,hash:[0;32]};let encoded=bytes(&root.join(record.relative_path()),MAX_ENCODED_TILE_BYTES)?;
        total=total.checked_add(encoded.len()as u64).ok_or(CoreError::Invalid)?;if total.checked_add(MAX_MANIFEST_BYTES as u64).filter(|size|*size<=budget).is_none(){return Err(CoreError::Backpressure)}
        record.encoded_bytes=encoded.len()as u32;record.hash=*blake3::hash(&encoded).as_bytes();records.push(record);
    }
    let result=Manifest::new(source.clone(),records).map_err(|_|CoreError::Invalid)?;
    if root.join("pyramid.vwt").try_exists()?{let original=bytes(&root.join("pyramid.vwt"),MAX_MANIFEST_BYTES)?;if Manifest::decode(&original,source).map_err(|_|CoreError::Invalid)?!=result{return Err(CoreError::Invalid)}}
    Ok(result)
}
pub(super) fn install<Q:vw_store::FreeSpaceQuery>(cache:&mut DiskCache<Q>,root:&Path,manifest:&Manifest,cancel:&Cancellation)->Result<()>{
    for record in manifest.tiles(){cancel.check()?;let encoded=bytes(&root.join(record.relative_path()),MAX_ENCODED_TILE_BYTES)?;record.verify(&encoded).map_err(|_|CoreError::Invalid)?;
        // Decoder validation precedes every cache write. Decoded buffers are
        // dropped before filesystem hashing/manifest serialization.
        drop(decode(&encoded,record.key,manifest.pyramid().region(record.key).map_err(|_|CoreError::Invalid)?)?);
        cancel.check()?;cache.put(&tile_key(manifest.source(),record.key),&encoded)?;
    }
    cancel.check()?;cache.put(&manifest_key(manifest.source()),&manifest.encode())?;
    // Low-space eviction cannot masquerade as a complete published cache.
    for record in manifest.tiles(){if !cache.contains(&tile_key(manifest.source(),record.key)){return Err(CoreError::Backpressure)}}
    if !cache.contains(&manifest_key(manifest.source())){return Err(CoreError::Backpressure)}Ok(())
}
pub(super) fn decode(bytes:&[u8],key:TileKey,region:TileRegion)->Result<Vec<u8>>{
    if key.level==0{if bytes.len()<20||&bytes[..4]!=b"RIFF"||&bytes[8..16]!=b"WEBPVP8L"{return Err(CoreError::Invalid)}}else if bytes.get(..2)!=Some(&[0xff,0xd8]){return Err(CoreError::Invalid)}
    let decoded=vw_raster::decode(bytes,vw_raster::DecodeLimits{max_encoded_bytes:MAX_ENCODED_TILE_BYTES,max_pixels:256*256,max_memory_bytes:32*1024*1024})?;
    if decoded.width!=region.width||decoded.height!=region.height||decoded.icc.is_some()||decoded.orientation_applied!=1{return Err(CoreError::Invalid)}
    match decoded.pixels{vw_raster::Pixels::Rgba8(rgba) if rgba.chunks_exact(4).all(|p|p[3]==255)=>Ok(rgba),_=>Err(CoreError::Unsupported)}
}
