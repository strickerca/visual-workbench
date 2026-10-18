//! Source-resolution PNG strips use only verified lossless level-zero pixels.
//! Lower JPEG/preview levels are never export inputs. The original must remain
//! available and is rehashed before output begins; no cached-preview fallback.
use super::{GiantImage,project_source,source,tiles,hex};
use crate::{Cancellation,CoreError,ProjectInfo,QueryRect,Result,WorkflowBinding};
use std::{fs::File,io::{Read,Write},path::Path,sync::Arc};
use vw_assets::{Manifest,TileKey};
use vw_model::{AssetId,Id};
use vw_raster::{AssetResolver,DecodedImage,Pixels,RasterError,RasterSource,Region,SourceInfo};
use vw_store::{BlobStore,DiskCache,FreeSpaceQuery};
const MAX_MEMORY:u64=256*1024*1024;
const MAX_OUTPUT:u64=4*1024*1024*1024;
// DiskCache retains up to 100,000 entries independent of this image. Charge
// 96 MiB for its bounded keys/filenames/BTree nodes, plus 32 MiB transient JSON
// capacity when a tile touch persists the <=16 MiB index. No image-count proxy.
const CACHE_INDEX_RESIDENT:u64=96*1024*1024;
const READ_WORKSPACE:u64=64*1024*1024; // 32 MiB tile codec + 32 MiB index serialization.

#[derive(Clone,uniffi::Record)]
pub struct GiantPngExport{pub binding:WorkflowBinding,pub output_path:String,pub marked:bool,pub region:Option<QueryRect>,pub assume_untagged_srgb:bool,pub memory_budget_bytes:u64,pub max_encoded_bytes:u64}
#[derive(Clone,uniffi::Record)]
pub struct GiantPngReceipt{pub revision:ProjectInfo,pub encoded_bytes:u64,pub blake3:String,pub metadata_json:String,pub estimated_peak_bytes:u64}
#[uniffi::export]
impl GiantImage{
    pub async fn export_png(&self,options:GiantPngExport,cancellation:Arc<Cancellation>)->Result<GiantPngReceipt>{
        cancellation.check()?;
        let guard=self.admit_operation()?;
        let guard=self.check_binding_owned(guard).await?;
        if options.binding.document_id!=self.document{return Err(CoreError::Invalid)}
        // The owned guard follows queued jobs even if this Rust future is dropped.
        // No new tile decode may repopulate RAM while the snapshot is retained.
        let guard=self.worker.call(move|state|{state.memory.clear().map_err(|_|CoreError::Worker)?;Ok(guard)}).await?;
        if options.output_path.is_empty()||options.output_path.len()>8192||!Path::new(&options.output_path).is_absolute()||!(64*1024*1024..=MAX_MEMORY).contains(&options.memory_budget_bytes)||options.max_encoded_bytes==0||options.max_encoded_bytes>MAX_OUTPUT{return Err(CoreError::Invalid)}
        let expected=self.source.clone();let document=self.document.clone();let binding=options.binding.clone();let budget=options.memory_budget_bytes;let cancel=cancellation.clone();
        let(snapshot,revision,info,blobs,canonical)=self.project.worker.call(move|state|{
            cancel.check()?;
            // Borrowed admission precedes canonical hash and project clone. All
            // retained snapshot bytes stay charged during later strip encoding.
            let canonical=crate::workflow::canonical_workspace(state,budget.saturating_sub(CACHE_INDEX_RESIDENT+READ_WORKSPACE+2*vw_assets::MAX_MANIFEST_BYTES as u64),false).map_err(|_|CoreError::Backpressure)?;
            crate::workflow::check_binding(state,&binding).map_err(|_|CoreError::Invalid)?;
            if project_source(state,&document)?!=expected{return Err(CoreError::Invalid)}
            // Construct ownership before crossing the actor/oneshot boundary.
            // A dropped result or skipped next job destroys the clone first.
            let snapshot=HeldSnapshot{project:state.project()?.clone(),_guard:guard};
            Ok((snapshot,state.view_revision()?,state.info()?,state.blobs.clone(),canonical))
        }).await?;
        let document=self.document.clone();self.worker.call(move|state|{
            let project=snapshot.project();
            cancellation.check()?;
            let id=AssetId::try_from(hex(&state.manifest.source().original))?;
            let mut original=source::regular(&blobs.path(&id)?)?;
            let(hash,_,size)=source::hash_copy(&mut original,None,state.manifest.source().original_bytes,&cancellation)?;
            if hash!=state.manifest.source().original||size!=state.manifest.source().original_bytes{return Err(CoreError::Invalid)}
            // Retain the verified original handle through publication. Windows
            // denies concurrent write/delete; portable stores still rehash bytes.
            let document_id=Id::try_from(document)?;let definition=&project.documents.get(&document_id).ok_or(CoreError::Invalid)?.definition;
            if definition.capture.is_some(){return Err(CoreError::Unsupported)}
            let region=options.region.map(|r|region(r,state.manifest.source().width,state.manifest.source().height)).transpose()?;
            let request=vw_raster::ExportRequest{format:vw_raster::ExportFormat::Png8,region,revision,alpha:vw_raster::AlphaPolicy::Preserve,color:vw_raster::ColorPolicy::Preserve,allow_depth_reduction:false,memory_budget_bytes:budget,capture_session:None,frame_id:None};
            let limits=vw_raster::StreamLimits{max_encoded_bytes:options.max_encoded_bytes,max_strip_rows:128,max_pixels:vw_assets::MAX_PIXELS};
            let target=Path::new(&options.output_path);let parent=target.parent().ok_or(CoreError::Invalid)?.canonicalize()?;let destination=parent.join(target.file_name().ok_or(CoreError::Invalid)?);
            if std::fs::symlink_metadata(&destination).is_ok(){return Err(CoreError::Invalid)}
            let mut raster=TileSource::new(&mut state.cache,&state.manifest,canonical)?;
            // Clean preflight before a file is created; marked preflight performs
            // its additional shape/resource admission before touching its sink.
            vw_raster::plan_png(&raster,&request,limits)?;
            let mut output=tempfile::Builder::new().prefix(".vw-giant-export-").tempfile_in(&parent)?;let mut hash=blake3::Hasher::new();
            let report={let mut sink=HashWriter{file:output.as_file_mut(),hash:&mut hash};
                if options.marked{let assets=ExportAssets{project,blobs:&blobs,budget:budget.saturating_sub(canonical),cancel:&cancellation};vw_raster::export_document_png_to(&mut raster,vw_raster::MarkedDocument{project,document:&document_id,assets:&assets,options:vw_raster::RenderOptions{memory_budget_bytes:budget,include_guides:false,assume_untagged_srgb:options.assume_untagged_srgb}},&request,limits,&mut sink,&||cancellation.is_cancelled())?}
                else{vw_raster::export_png_to(&mut raster,&request,limits,&mut sink,&||cancellation.is_cancelled())?}
            };
            output.as_file_mut().flush()?;output.as_file().sync_all()?;if output.as_file().metadata()?.len()!=report.encoded_bytes{return Err(CoreError::Storage)}
            let metadata_json=serde_json::to_string(&report.metadata).map_err(|_|CoreError::Invalid)?;if metadata_json.len()>64*1024{return Err(CoreError::Backpressure)}
            cancellation.check()?;
            let published=output.persist_noclobber(&destination).map_err(|error|CoreError::from(error.error))?;published.sync_all()?;crate::creation::sync_directory(&parent)?;
            drop(original);
            Ok(GiantPngReceipt{revision:info,encoded_bytes:report.encoded_bytes,blake3:hash.finalize().to_hex().to_string(),metadata_json,estimated_peak_bytes:report.plan.estimated_peak_bytes})
        }).await
    }
}
// Rust drops fields in declaration order: release the clone before permitting
// another preview decoder, including any failed/cancelled worker completion.
struct HeldSnapshot{project:vw_model::Project,_guard:tokio::sync::OwnedMutexGuard<()>}
impl HeldSnapshot{fn project(&self)->&vw_model::Project{&self.project}}
fn region(r:QueryRect,width:u32,height:u32)->Result<Region>{
    if ![r.x,r.y,r.width,r.height].iter().all(|n|n.is_finite()&&*n>=0.&&n.fract()==0.)||r.x>u32::MAX as f64||r.y>u32::MAX as f64||r.width>u32::MAX as f64||r.height>u32::MAX as f64{return Err(CoreError::Invalid)}
    let result=Region{x:r.x as u32,y:r.y as u32,width:r.width as u32,height:r.height as u32};result.validate(width,height)?;Ok(result)
}
struct TileSource<'a,Q:FreeSpaceQuery>{cache:&'a mut DiskCache<Q>,manifest:&'a Manifest,info:SourceInfo,resident:u64}
impl<'a,Q:FreeSpaceQuery> TileSource<'a,Q>{fn new(cache:&'a mut DiskCache<Q>,manifest:&'a Manifest,canonical:u64)->Result<Self>{
    let source=manifest.source();if source.bit_depth!=8||source.icc.is_some()||source.display_conversion{return Err(CoreError::Unsupported)}
    let resident=canonical.checked_add((manifest.tiles().len()as u64)*64).and_then(|v|v.checked_add(CACHE_INDEX_RESIDENT+1024*1024)).ok_or(CoreError::Backpressure)?;
    Ok(Self{cache,manifest,resident,info:SourceInfo{width:source.width,height:source.height,bit_depth:8,icc:None,source_asset:AssetId::try_from(hex(&source.original))?,original_available:true,orientation_applied:source.orientation}})
}}
impl<Q:FreeSpaceQuery> RasterSource for TileSource<'_,Q>{
    fn info(&self)->&SourceInfo{&self.info}
    fn resident_bytes(&self)->u64{self.resident}
    fn read_workspace_bytes(&self)->u64{READ_WORKSPACE}
    fn read_region(&mut self,region:Region,budget:u64,cancelled:&dyn Fn()->bool)->std::result::Result<DecodedImage,RasterError>{
        region.validate(self.info.width,self.info.height)?;
        let size=u64::from(region.width).checked_mul(u64::from(region.height)).and_then(|v|v.checked_mul(4)).ok_or(RasterError::Allocation)?;
        let estimated=self.resident.checked_add(READ_WORKSPACE).and_then(|v|v.checked_add(size)).ok_or(RasterError::Allocation)?;if estimated>budget{return Err(RasterError::Memory{estimated,budget})}
        if cancelled(){return Err(RasterError::Cancelled)}let length=usize::try_from(size).map_err(|_|RasterError::Allocation)?;let mut out=Vec::new();out.try_reserve_exact(length).map_err(|_|RasterError::Allocation)?;out.resize(length,0);
        let right=region.x+region.width;let bottom=region.y+region.height;
        for y in region.y/256..bottom.div_ceil(256){for x in region.x/256..right.div_ceil(256){
            if cancelled(){return Err(RasterError::Cancelled)}let key=TileKey{level:0,x,y};let record=self.manifest.tile(key).map_err(|_|RasterError::MissingAsset)?;let tile_region=self.manifest.pyramid().region(key).map_err(|_|RasterError::MissingAsset)?;
            let encoded=self.cache.get_bounded(&tiles::tile_key(self.manifest.source(),key),vw_assets::MAX_ENCODED_TILE_BYTES as u64).map_err(|_|RasterError::Io)?.ok_or(RasterError::MissingAsset)?;record.verify(&encoded).map_err(|_|RasterError::MissingAsset)?;
            let tile=tiles::decode(&encoded,key,tile_region).map_err(|_|RasterError::Codec)?;drop(encoded);
            let left=region.x.max(tile_region.x);let top=region.y.max(tile_region.y);let tr=right.min(tile_region.x+tile_region.width);let tb=bottom.min(tile_region.y+tile_region.height);let count=(tr-left)as usize*4;
            for row in top..tb{if cancelled(){return Err(RasterError::Cancelled)}let from=((row-tile_region.y)as usize*tile_region.width as usize+(left-tile_region.x)as usize)*4;let to=((row-region.y)as usize*region.width as usize+(left-region.x)as usize)*4;out[to..to+count].copy_from_slice(&tile[from..from+count]);}
        }}
        Ok(DecodedImage{width:region.width,height:region.height,pixels:Pixels::Rgba8(out),icc:None,source_asset:self.info.source_asset.clone(),original_available:true,orientation_applied:self.info.orientation_applied})
    }
}
struct ExportAssets<'a>{project:&'a vw_model::Project,blobs:&'a BlobStore,budget:u64,cancel:&'a Cancellation}
impl AssetResolver for ExportAssets<'_>{fn image(&self,id:&str)->std::result::Result<DecodedImage,RasterError>{
    if self.cancel.is_cancelled(){return Err(RasterError::Cancelled)}
    let asset=AssetId::try_from(id.to_owned()).map_err(|_|RasterError::MissingAsset)?;
    let binding=self.project.assets.get(&asset).ok_or(RasterError::MissingAsset)?;
    let pixels=u64::from(binding.width).checked_mul(u64::from(binding.height)).ok_or(RasterError::Allocation)?;
    if pixels==0||pixels>50_000_000{return Err(RasterError::Unsupported("referenced image pixel limit"))}
    // This allowance is no greater than raster's per-asset reserve, even if a
    // dishonest metadata record understates the real image. ICC bytes are not
    // borrowed from other strip/encoder reservations; unusually large profiles
    // can conservatively refuse this subset rather than exceed the allowance.
    let decode_budget=pixels.checked_mul(32).and_then(|n|n.checked_add(32*1024*1024)).ok_or(RasterError::Allocation)?;
    let total=decode_budget.checked_add(binding.byte_size).ok_or(RasterError::Allocation)?;
    if total>self.budget{return Err(RasterError::Memory{estimated:total,budget:self.budget})}
    let path=self.blobs.path(&asset).map_err(|_|RasterError::MissingAsset)?;
    let file=source::regular(&path).map_err(|_|RasterError::MissingAsset)?;
    let len=file.metadata().map_err(|_|RasterError::Io)?.len();
    if len==0||len!=binding.byte_size{return Err(RasterError::MissingAsset)}
    if len>64*1024*1024{return Err(RasterError::EncodedLimit{limit:64*1024*1024})}
    let mut encoded=Vec::new();encoded.try_reserve_exact(len as usize).map_err(|_|RasterError::Allocation)?;
    let mut input=file.take(len+1);let mut buffer=[0;65536];
    loop{if self.cancel.is_cancelled(){return Err(RasterError::Cancelled)}let n=input.read(&mut buffer).map_err(|_|RasterError::Io)?;if n==0{break}if encoded.len()as u64+n as u64>len{return Err(RasterError::MissingAsset)}encoded.extend_from_slice(&buffer[..n]);}
    if encoded.len()as u64!=len||AssetId::hash(&encoded)!=asset{return Err(RasterError::MissingAsset)}
    let image=crate::os_images::decode(&encoded,vw_raster::DecodeLimits{max_encoded_bytes:len as usize,max_pixels:pixels,max_memory_bytes:decode_budget},&||self.cancel.is_cancelled())?;
    let (width,height)=if binding.orientation>=5{(binding.height,binding.width)}else{(binding.width,binding.height)};
    if image.width!=width||image.height!=height||u32::from(image.orientation_applied)!=binding.orientation||u32::from(image.pixels.bit_depth())!=binding.bit_depth||image.icc.as_deref().unwrap_or_default()!=binding.icc_profile.as_slice(){return Err(RasterError::MissingAsset)}
    Ok(image)
}}
struct HashWriter<'a>{file:&'a mut File,hash:&'a mut blake3::Hasher}
impl Write for HashWriter<'_>{fn write(&mut self,bytes:&[u8])->std::io::Result<usize>{let n=self.file.write(bytes)?;self.hash.update(&bytes[..n]);Ok(n)}fn flush(&mut self)->std::io::Result<()>{self.file.flush()}}
#[cfg(test)] mod tests;
