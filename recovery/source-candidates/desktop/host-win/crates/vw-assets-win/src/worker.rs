use crate::{admission::admit_input,native::Api,ownership::{pin_ancestors,plain,read_locked},process,BuildRequest,Cancellation,Error,LockedHelper,LockedRuntime,Result,wide};
use std::{fs::{self,OpenOptions},io::{Read,Write},path::Path,sync::atomic::{AtomicBool,Ordering},time::Duration};
use vw_assets::{Manifest,TileRecord,MAX_ENCODED_TILE_BYTES,MAX_MANIFEST_BYTES};
static ACTIVE:AtomicBool=AtomicBool::new(false);
struct Permit;
impl Drop for Permit{fn drop(&mut self){ACTIVE.store(false,Ordering::Release);}}

/// Synchronous worker API: invoke on the app's retained bounded worker, never UI.
/// Original is copied/hash-verified into private sibling staging. Destination
/// publication is same-volume/no-clobber only after exact complete verification.
/// No project metadata is mutated. Parent app must atomically bind the returned
/// manifest to its immutable original before exposing the tiled document.
pub fn build(helper:&LockedHelper,runtime:&LockedRuntime,input:&Path,destination:&Path,request:&BuildRequest,cancel:&Cancellation)->Result<Manifest>{
    request.validate()?;cancel.check()?;
    ACTIVE.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).map_err(|_|Error::Busy)?;let _permit=Permit;
    if destination.file_name().is_none()||fs::symlink_metadata(destination).is_ok(){return Err(Error::Storage);}
    let parent=destination.parent().ok_or(Error::Invalid)?;let _parent_pins=pin_ancestors(parent)?;
    let _input_pins=pin_ancestors(input.parent().ok_or(Error::Invalid)?)?;
    let mut original=read_locked(input)?;
    if original.metadata().map_err(|_|Error::Storage)?.len()!=request.original_bytes{return Err(Error::Binding);}
    admit_input(&mut original,request)?;
    let staging=tempfile::Builder::new().prefix(".vw-pyramid-").tempdir_in(parent).map_err(|_|Error::Storage)?;
    let stage=staging.path();
    let mut copy=OpenOptions::new().write(true).create_new(true).open(stage.join("original.input")).map_err(|_|Error::Storage)?;
    let mut hash=blake3::Hasher::new();let mut count=0u64;let mut buffer=[0;65536];
    loop{cancel.check()?;let n=original.read(&mut buffer).map_err(|_|Error::Storage)?;if n==0{break;}count+=n as u64;if count>request.original_bytes{return Err(Error::Binding);}hash.update(&buffer[..n]);copy.write_all(&buffer[..n]).map_err(|_|Error::Storage)?;}
    if count!=request.original_bytes||hash.finalize().as_bytes()!=&request.original_hash{return Err(Error::Binding);}copy.sync_all().map_err(|_|Error::Storage)?;drop(copy);
    let json=serde_json::to_vec(request).map_err(|_|Error::Invalid)?;if json.len()>4096{return Err(Error::Limit);}
    let mut receipt=OpenOptions::new().write(true).create_new(true).open(stage.join("request.json")).map_err(|_|Error::Storage)?;receipt.write_all(&json).and_then(|()|receipt.sync_all()).map_err(|_|Error::Storage)?;drop(receipt);
    let args=[runtime.directory.to_str().ok_or(Error::Invalid)?.to_owned(),stage.to_str().ok_or(Error::Invalid)?.to_owned()];
    let outcome=process::run(&helper.path,&args,cancel,Duration::from_millis(request.timeout_ms))?;
    if outcome.code!=0{return Err(Error::Codec);}cancel.check()?;
    let result=stage.join("result");plain(&result,true)?;
    let manifest=read_manifest(&result.join("pyramid.vwt"),request)?;
    verify_output(&result,&manifest,cancel)?;cancel.check()?;
    let from=wide(result.as_os_str())?;let to=wide(destination.as_os_str())?;
    // SAFETY: exact owned sibling staging directory, pinned destination parent,
    // and zero MoveFileEx flags means existing destinations are never replaced.
    if unsafe{windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(),to.as_ptr(),0)}==0{return Err(Error::Storage);}
    // Destination now owns only complete disposable derived files. The original
    // remains untouched; TempDir cleans this operation's request/source/scratch.
    Ok(manifest)
}

/// Called only by our dedicated binary under the parent-owned bounded Job.
/// # Safety
/// Caller must be the isolated single-threaded helper process before codec init;
/// this function sets its process-local scratch environment before threads exist.
pub unsafe fn helper(runtime:&Path,stage:&Path)->Result<()> {
    use windows_sys::Win32::System::{Threading::GetCurrentProcess,JobObjects::IsProcessInJob};
    let mut in_job=0;
    // SAFETY: valid current-process pseudo handle and writable BOOL.
    if unsafe{IsProcessInJob(GetCurrentProcess(),std::ptr::null_mut(),&mut in_job)}==0||in_job==0{return Err(Error::WorkerUnavailable);}
    let _stage_pins=pin_ancestors(stage)?;let runtime=LockedRuntime::open(runtime)?;
    let input=read_locked(&stage.join("request.json"))?;let len=input.metadata().map_err(|_|Error::Storage)?.len();if len>4096{return Err(Error::Limit);}let mut bytes=Vec::with_capacity(len as usize);input.take(4097).read_to_end(&mut bytes).map_err(|_|Error::Storage)?;
    if bytes.len()>4096{return Err(Error::Limit);}let request:BuildRequest=serde_json::from_slice(&bytes).map_err(|_|Error::Invalid)?;request.validate()?;
    let input_path=stage.join("original.input");let mut original=read_locked(&input_path)?;admit_input(&mut original,&request)?;
    let mut hasher=blake3::Hasher::new();let mut total=0u64;let mut chunk=[0;65536];loop{let n=original.read(&mut chunk).map_err(|_|Error::Storage)?;if n==0{break;}total+=n as u64;if total>request.original_bytes{return Err(Error::Binding);}hasher.update(&chunk[..n]);}
    if total!=request.original_bytes||hasher.finalize().as_bytes()!=&request.original_hash{return Err(Error::Binding);}
    // SAFETY: this isolated process is still single-threaded before libvips init.
    // It inherited an empty environment. Scratch belongs exclusively to staging.
    unsafe{std::env::set_var("TMP",stage);std::env::set_var("TEMP",stage);std::env::set_var("VIPS_DISC_THRESHOLD","64m");}
    let api=Api::new(&runtime)?;api.pyramid(&input_path,&stage.join("pyramid"),&request)?;
    let result=stage.join("result");fs::create_dir(&result).map_err(|_|Error::Storage)?;
    let pyramid=request.source().validate().map_err(|_|Error::Limit)?;let top=pyramid.levels().len()-1;
    let mut records=Vec::with_capacity(pyramid.tile_count());
    for level in pyramid.levels(){fs::create_dir(result.join(format!("L{}",level.index))).map_err(|_|Error::Storage)?;}
    for key in pyramid.keys(){
        let input=stage.join("pyramid_files").join((top-key.level as usize).to_string()).join(format!("{}_{}.webp",key.x,key.y));plain(&input,false)?;
        let path=result.join(format!("L{}",key.level)).join(format!("{}_{}.{}",key.x,key.y,if key.level==0{"webp"}else{"jpg"}));
        if key.level==0 {fs::rename(&input,&path).map_err(|_|Error::Storage)?;}else{api.jpeg(&input,&path)?;fs::remove_file(&input).map_err(|_|Error::Storage)?;}
        let bytes=tile_bytes(&path)?;validate_tile(&bytes,key.level,pyramid.region(key).map_err(|_|Error::Invalid)?)?;
        let mut file=OpenOptions::new().write(true).open(&path).map_err(|_|Error::Storage)?;file.flush().and_then(|()|file.sync_all()).map_err(|_|Error::Storage)?;
        records.push(TileRecord{key,encoded_bytes:bytes.len()as u32,hash:*blake3::hash(&bytes).as_bytes()});
    }
    drop(api);
    let manifest=Manifest::new(request.source(),records).map_err(|_|Error::Invalid)?;
    if manifest.encoded_bytes()>request.disk_budget{return Err(Error::Limit);}
    let mut file=OpenOptions::new().write(true).create_new(true).open(result.join("pyramid.vwt")).map_err(|_|Error::Storage)?;
    file.write_all(&manifest.encode()).and_then(|()|file.sync_all()).map_err(|_|Error::Storage)?;Ok(())
}
fn read_manifest(path:&Path,r:&BuildRequest)->Result<Manifest>{
    let file=read_locked(path)?;let size=file.metadata().map_err(|_|Error::Storage)?.len();if size>MAX_MANIFEST_BYTES as u64{return Err(Error::Limit);}let mut bytes=Vec::with_capacity(size as usize);file.take(MAX_MANIFEST_BYTES as u64+1).read_to_end(&mut bytes).map_err(|_|Error::Storage)?;Manifest::decode(&bytes,&r.source()).map_err(|_|Error::Binding)
}
fn tile_bytes(path:&Path)->Result<Vec<u8>>{let file=read_locked(path)?;let size=file.metadata().map_err(|_|Error::Storage)?.len();if size==0||size>MAX_ENCODED_TILE_BYTES as u64{return Err(Error::Limit);}let mut bytes=Vec::with_capacity(size as usize);file.take(MAX_ENCODED_TILE_BYTES as u64+1).read_to_end(&mut bytes).map_err(|_|Error::Storage)?;if bytes.len()>MAX_ENCODED_TILE_BYTES{return Err(Error::Limit);}Ok(bytes)}
fn validate_tile(bytes:&[u8],level:u8,region:vw_assets::TileRegion)->Result<()> {
    if level==0 {
        // The worker strips ancillary metadata, yielding one VP8L chunk. Refuse
        // lossy VP8 even when renamed .webp and perfectly hash-bound.
        if bytes.len()<20 || &bytes[..4]!=b"RIFF" || &bytes[8..16]!=b"WEBPVP8L" {return Err(Error::Binding);}
    } else if bytes.get(..2)!=Some(&[0xff,0xd8]){return Err(Error::Binding);}
    let decoded=vw_raster::decode(bytes,vw_raster::DecodeLimits{max_encoded_bytes:MAX_ENCODED_TILE_BYTES,max_pixels:256*256,max_memory_bytes:32*1024*1024}).map_err(|_|Error::Codec)?;
    if decoded.width!=region.width||decoded.height!=region.height||decoded.pixels.bit_depth()!=8||decoded.icc.is_some()||decoded.orientation_applied!=1{return Err(Error::Binding);}
    if let vw_raster::Pixels::Rgba8(p)=&decoded.pixels {if p.chunks_exact(4).any(|rgba|rgba[3]!=255){return Err(Error::Unsupported);}}
    Ok(())
}
fn verify_output(root:&Path,manifest:&Manifest,cancel:&Cancellation)->Result<()> {
    let mut entries=0usize;
    for entry in fs::read_dir(root).map_err(|_|Error::Storage)?{let e=entry.map_err(|_|Error::Storage)?;let n=e.file_name().into_string().map_err(|_|Error::Binding)?;if n=="pyramid.vwt"{plain(&e.path(),false)?;}else{let level=n.strip_prefix('L').and_then(|n|n.parse::<u8>().ok()).ok_or(Error::Binding)?;if n!=format!("L{level}"){return Err(Error::Binding);}manifest.pyramid().level(level).map_err(|_|Error::Binding)?;plain(&e.path(),true)?;}entries+=1;if entries>manifest.pyramid().levels().len()+1{return Err(Error::Binding);}}
    if entries!=manifest.pyramid().levels().len()+1{return Err(Error::Binding);}
    for level in manifest.pyramid().levels(){let dir=root.join(format!("L{}",level.index));let expected=level.columns as usize*level.rows as usize;let mut count=0;for entry in fs::read_dir(&dir).map_err(|_|Error::Storage)?{plain(&entry.map_err(|_|Error::Storage)?.path(),false)?;count+=1;if count>expected{return Err(Error::Binding);}}if count!=expected{return Err(Error::Binding);}}
    for tile in manifest.tiles(){cancel.check()?;let bytes=tile_bytes(&root.join(tile.relative_path()))?;tile.verify(&bytes).map_err(|_|Error::Binding)?;validate_tile(&bytes,tile.key.level,manifest.pyramid().region(tile.key).map_err(|_|Error::Binding)?)?;}Ok(())
}
