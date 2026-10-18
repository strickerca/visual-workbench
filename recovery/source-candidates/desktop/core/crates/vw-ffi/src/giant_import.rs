//! File-backed giant-image admission/publication. Only approved platform producers
//! may supply the derived pyramid; arbitrary external pyramids are not an import format.
mod source;
mod tiles;
#[cfg(test)] mod tests;
use crate::{project::SessionPermit,worker::Worker,*};
use std::{fs::{File,OpenOptions},io::{Seek,SeekFrom,Write},path::{Path},sync::{Arc,atomic::{AtomicUsize,Ordering}}};
use vw_assets::{Manifest,Source,TileKey};
use vw_model::{AssetId,DeviceId,Document,Id,Layer,Project};
use vw_proto::v1 as pb;
use vw_store::{BlobStore,DiskCache,ProjectStore};
const DISK_LIMIT:u64=16*1024*1024*1024;
static INPUTS:AtomicUsize=AtomicUsize::new(0);
struct InputPermit;
impl InputPermit {fn acquire()->Result<Self>{INPUTS.try_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<2).then_some(n+1)).map_err(|_|CoreError::Backpressure)?;Ok(Self)}}
impl Drop for InputPermit{fn drop(&mut self){INPUTS.fetch_sub(1,Ordering::AcqRel);}}

#[derive(Clone,uniffi::Record)]
pub struct GiantInputOptions {pub source_path:String,pub work_directory:String,pub max_original_bytes:u64}
#[derive(Clone,uniffi::Record)]
pub struct GiantInputInfo {pub source_path:String,pub asset_id:String,pub sha256:String,pub encoded_bytes:u64,pub raw_width:u32,pub raw_height:u32,pub width:u32,pub height:u32,pub orientation:u8,pub format:String}
#[derive(Clone,uniffi::Record)]
pub struct GiantProjectOptions {pub path:String,pub project_id:String,pub document_id:String,pub layer_id:String,pub device_id:String,pub title:String,pub now_ms:i64,pub disk_cache_bytes:u64}
#[derive(Clone,Debug,uniffi::Record)]
pub struct GiantViewport {pub left:f64,pub top:f64,pub right:f64,pub bottom:f64,pub a:f64,pub b:f64,pub c:f64,pub d:f64}
#[derive(Clone,Debug,uniffi::Record)]
pub struct GiantTileAddress {pub level:u8,pub x:u32,pub y:u32}
#[derive(Clone,Debug,uniffi::Record)]
pub struct GiantViewportPlan {pub asset_id:String,pub manifest_hash:String,pub level:u8,pub visible:Vec<GiantTileAddress>,pub prefetch:Vec<GiantTileAddress>}
#[derive(Clone,uniffi::Record)]
pub struct GiantTilePixels {pub asset_id:String,pub manifest_hash:String,pub address:GiantTileAddress,pub document_x:u32,pub document_y:u32,pub document_width:u32,pub document_height:u32,pub width:u32,pub height:u32,pub rgba:Vec<u8>}
struct InputState{input:Option<File>,info:GiantInputInfo,source:Source,metadata:source::Metadata,_staging:Option<tempfile::TempDir>,_permit:InputPermit,published:bool}
#[derive(uniffi::Object)]
pub struct GiantInput {worker:Worker<InputState>}
#[uniffi::export]
pub async fn prepare_giant_image(options:GiantInputOptions,cancellation:Arc<Cancellation>)->Result<Arc<GiantInput>>{
    crate::worker::startup(move||{
        cancellation.check()?;let permit=InputPermit::acquire()?;
        if options.max_original_bytes==0||options.max_original_bytes>source::MAX_ORIGINAL||options.source_path.len()>8192{return Err(CoreError::Invalid)}
        let mut external=source::regular(Path::new(&options.source_path))?;
        if external.metadata()?.len()>options.max_original_bytes{return Err(CoreError::Backpressure)}
        let work=source::directory(&options.work_directory)?;let staging=tempfile::Builder::new().prefix("vw-giant-input-").tempdir_in(&work)?;
        let path=staging.path().join("original");let mut output=OpenOptions::new().write(true).create_new(true).open(&path)?;
        let(hash,sha256,bytes)=source::hash_copy(&mut external,Some(&mut output),options.max_original_bytes,&cancellation)?;output.flush()?;output.sync_all()?;drop(output);
        let mut input=source::regular(&path)?;let metadata=source::inspect(&mut input,&cancellation)?;let(width,height)=metadata.dimensions();
        let source=Source{original:hash,original_bytes:bytes,width,height,orientation:metadata.orientation,bit_depth:8,icc:None,display_conversion:false};source.validate().map_err(|_|CoreError::Invalid)?;
        let info=GiantInputInfo{source_path:path.to_str().ok_or(CoreError::Invalid)?.into(),asset_id:hex(&hash),sha256,encoded_bytes:bytes,raw_width:metadata.width,raw_height:metadata.height,width,height,orientation:metadata.orientation,format:metadata.format.into()};
        let state=InputState{input:Some(input),info,source,metadata,_staging:Some(staging),_permit:permit,published:false};
        Ok(Arc::new(GiantInput{worker:Worker::new("vw-giant-input",state,4)?}))
    }).await
}
#[uniffi::export]
impl GiantInput {
    pub async fn describe(&self)->Result<GiantInputInfo>{self.worker.call(|state|Ok(state.info.clone())).await}
    /// This boundary trusts only the bundled hash-pinned Windows worker or the
    /// in-process Android region producer. It verifies inventory/codec/hash, not
    /// a cryptographic proof that arbitrary foreign pixels derive from a JPEG.
    pub async fn publish_project(&self,options:GiantProjectOptions,approved_producer_directory:String,cancellation:Arc<Cancellation>)->Result<Arc<ProjectSession>>{
        self.worker.call(move|state|publish(state,options,approved_producer_directory,&cancellation,vw_store::RealFreeSpace)).await
    }
    pub async fn rebuild_cache(&self,project:Arc<ProjectSession>,document_id:String,approved_producer_directory:String,disk_cache_bytes:u64,cancellation:Arc<Cancellation>)->Result<()> {
        project.check_open()?;if disk_cache_bytes==0||disk_cache_bytes>DISK_LIMIT{return Err(CoreError::Invalid)}
        let(root,expected)=project.worker.call(move|state|Ok((state.blobs.project_root().to_path_buf(),project_source(state,&document_id)?))).await?;
        self.worker.call(move|state|{
            cancellation.check()?;if state.source!=expected{return Err(CoreError::Invalid)}
            let producer=source::directory(&approved_producer_directory)?;let manifest=tiles::inventory(&producer,&expected,disk_cache_bytes,&cancellation)?;
            let(hash,_,size)=source::hash_copy(state.input.as_mut().ok_or(CoreError::Closed)?,None,expected.original_bytes,&cancellation)?;if hash!=expected.original||size!=expected.original_bytes{return Err(CoreError::Invalid)}
            let mut cache=DiskCache::new(&root,disk_cache_bytes)?;
            cache.remove(&tiles::manifest_key(&expected))?;
            tiles::install(&mut cache,&producer,&manifest,&cancellation)
        }).await
    }
    pub async fn shutdown(&self)->Result<()>{self.worker.shutdown(|state|{drop(state.input.take());if let Some(staging)=state._staging.take(){staging.close()?;}Ok(())}).await}
}
fn publish<Q:vw_store::FreeSpaceQuery>(state:&mut InputState,options:GiantProjectOptions,approved_producer_directory:String,cancellation:&Cancellation,query:Q)->Result<Arc<ProjectSession>>{
            if state.published{return Err(CoreError::Closed)}
            cancellation.check()?;let permit=SessionPermit::acquire()?;
            let (project,device)=initial_project(&options,state)?;
            let producer=source::directory(&approved_producer_directory)?;
            let manifest=tiles::inventory(&producer,&state.source,options.disk_cache_bytes,cancellation)?;
            let(original,_,bytes)=source::hash_copy(state.input.as_mut().ok_or(CoreError::Closed)?,None,state.source.original_bytes,cancellation)?;
            if original!=state.source.original||bytes!=state.source.original_bytes{return Err(CoreError::Invalid)}
            let destination=Path::new(&options.path);let parent=destination.parent().ok_or(CoreError::Invalid)?.canonicalize()?;
            let destination=parent.join(destination.file_name().ok_or(CoreError::Invalid)?);
            if destination.try_exists()?{return Err(CoreError::Invalid)}
            let staging=tempfile::Builder::new().prefix(".vw-giant-create-").tempdir_in(&parent)?;let candidate=staging.path().join("project");
            {
                let store=ProjectStore::create(&candidate,project,device,options.now_ms)?;let blobs=BlobStore::new(&candidate)?;
                state.input.as_mut().ok_or(CoreError::Closed)?.seek(SeekFrom::Start(0))?;
                let mut reader=CancelledRead{input:state.input.as_mut().ok_or(CoreError::Closed)?,cancel:cancellation,remaining:state.source.original_bytes};
                let asset=blobs.put_reader(&mut reader)?;
                if asset.to_string()!=state.info.asset_id||blobs.verify(&asset)?!=state.source.original_bytes{return Err(CoreError::Invalid)}
                let mut cache=DiskCache::with_query(&candidate,options.disk_cache_bytes,query)?;
                tiles::install(&mut cache,&producer,&manifest,cancellation)?;
                store.integrity_check()?;
            }
            cancellation.check()?;crate::creation::sync_directory(&candidate)?;cancellation.check()?;
            crate::creation::publish(&candidate,&destination)?;
            // The original is accepted at this no-clobber publication boundary;
            // failed sync/reopen or cancelled foreign delivery never deletes it.
            state.published=true;crate::creation::sync_directory(&parent)?;
            ProjectSession::from_store(ProjectStore::open(&destination)?,BlobStore::new(&destination)?,permit)
}
fn initial_project(options:&GiantProjectOptions,state:&InputState)->Result<(Project,DeviceId)>{
    if options.path.is_empty()||options.path.len()>8192||!Path::new(&options.path).is_absolute()||options.title.len()>4096||options.now_ms<0||options.disk_cache_bytes==0||options.disk_cache_bytes>DISK_LIMIT{return Err(CoreError::Invalid)}
    let project_id=Id::try_from(options.project_id.clone())?;let document=Id::try_from(options.document_id.clone())?;let layer=Id::try_from(options.layer_id.clone())?;let device=DeviceId::try_from(options.device_id.clone())?;
    let mut project=Project::new(project_id,options.title.clone(),device.clone());let asset=AssetId::try_from(state.info.asset_id.clone())?;
    project.assets.insert(asset,pb::AddAsset{asset_id:state.info.asset_id.clone(),format:state.metadata.format.into(),width:state.metadata.width,height:state.metadata.height,orientation:u32::from(state.metadata.orientation),bit_depth:8,has_alpha:false,color_space:"untagged".into(),icc_profile:Vec::new(),byte_size:state.source.original_bytes,source:"import".into(),captured_at_ms:0,metadata_json:"{}".into()});
    project.documents.insert(document.clone(),Document{definition:pb::CreateDocument{document_id:Some(document.to_proto()),kind:pb::DocumentKind::Image as i32,capture:None,schema_version:1,title:options.title.clone(),primary_asset_id:state.info.asset_id.clone()},pages:Vec::new(),created_at_ms:options.now_ms});
    project.layers.insert(layer.clone(),Layer{definition:pb::CreateLayer{layer_id:Some(layer.to_proto()),document_id:Some(document.to_proto()),page_index:-1,name:"Annotations".into(),kind:"annotation".into(),order_key:"V".into()},visible:true,locked:false,opacity:1.0,blend:"normal".into()});
    project.validate()?;Ok((project,device))
}
struct CancelledRead<'a>{input:&'a mut File,cancel:&'a Cancellation,remaining:u64}
impl std::io::Read for CancelledRead<'_>{fn read(&mut self,bytes:&mut[u8])->std::io::Result<usize>{if self.cancel.is_cancelled(){return Err(std::io::Error::other("cancelled"))}if self.remaining==0{return Ok(0)}let cap=bytes.len().min(self.remaining as usize);let count=std::io::Read::read(self.input,&mut bytes[..cap])?;self.remaining-=count as u64;Ok(count)}}
fn hex(bytes:&[u8])->String{bytes.iter().map(|b|format!("{b:02x}")).collect()}
fn address(key:TileKey)->GiantTileAddress{GiantTileAddress{level:key.level,x:key.x,y:key.y}}

struct ImageState{cache:DiskCache,manifest:Manifest,identity:String,memory:vw_assets::BudgetCache<TileKey,Vec<u8>>}
#[derive(uniffi::Object)]
pub struct GiantImage{worker:Worker<ImageState>,project:Arc<ProjectSession>,document:String,source:Source}
#[uniffi::export]
impl ProjectSession{
    /// Reconstruct disposable tiles from the immutable original after eviction.
    pub async fn prepare_giant_rebuild(self:Arc<Self>,document_id:String,work_directory:String,cancellation:Arc<Cancellation>)->Result<Arc<GiantInput>>{
        self.check_open()?;let cancel=cancellation.clone();
        let(path,expected)=self.worker.call(move|state|{cancel.check()?;let source=project_source(state,&document_id)?;let path=state.blobs.path(&AssetId::try_from(hex(&source.original))?)?;Ok((path,source))}).await?;
        let prepared=prepare_giant_image(GiantInputOptions{source_path:path.to_str().ok_or(CoreError::Invalid)?.into(),work_directory,max_original_bytes:expected.original_bytes},cancellation).await?;
        if prepared.worker.call(move|state|{if state.source!=expected{return Err(CoreError::Invalid)}Ok(())}).await.is_err(){prepared.shutdown().await?;return Err(CoreError::Invalid)}
        Ok(prepared)
    }
    /// One image-cache owner per project at a time (DiskCache's exclusive lock).
    pub async fn open_giant_image(self:Arc<Self>,document_id:String,disk_cache_bytes:u64,ram_cache_bytes:u64,cancellation:Arc<Cancellation>)->Result<Arc<GiantImage>>{
        self.check_open()?;if disk_cache_bytes==0||disk_cache_bytes>DISK_LIMIT||!(32*1024*1024..=vw_assets::DESKTOP_RAM_BYTES).contains(&ram_cache_bytes){return Err(CoreError::Invalid)}
        let document=document_id.clone();let cancel=cancellation.clone();let(root,source)=self.worker.call(move|state|{cancel.check()?;let source=project_source(state,&document)?;Ok((state.blobs.project_root().to_path_buf(),source))}).await?;
        let source_for_worker=source.clone();let(worker,source)=crate::worker::startup(move||{cancellation.check()?;let mut cache=DiskCache::new(&root,disk_cache_bytes)?;
            let raw=cache.get_bounded(&tiles::manifest_key(&source_for_worker),vw_assets::MAX_MANIFEST_BYTES as u64)?.ok_or(CoreError::Unsupported)?;
            let manifest=Manifest::decode(&raw,&source_for_worker).map_err(|_|CoreError::Invalid)?;let identity=hex(&manifest.identity());
            cancellation.check()?;let memory=vw_assets::BudgetCache::new(ram_cache_bytes).map_err(|_|CoreError::Invalid)?;
            Ok((Worker::new("vw-giant-image",ImageState{cache,manifest,identity,memory},8)?,source_for_worker))}).await?;
        Ok(Arc::new(GiantImage{worker,project:self,document:document_id,source}))
    }
}
fn project_source(state:&project::ProjectState,document:&str)->Result<Source>{
    let project=state.project()?;let doc=project.documents.get(&Id::try_from(document.to_owned())?).ok_or(CoreError::Invalid)?;let asset=project.assets.get(&AssetId::try_from(doc.definition.primary_asset_id.clone())?).ok_or(CoreError::Invalid)?;
    if asset.has_alpha||asset.bit_depth!=8||!asset.icc_profile.is_empty()||asset.color_space!="untagged"||!matches!(asset.format.as_str(),"png"|"jpeg"){return Err(CoreError::Unsupported)}
    let id=AssetId::try_from(asset.asset_id.clone())?;let hash=id.bytes();
    let(width,height)=if asset.orientation>=5{(asset.height,asset.width)}else{(asset.width,asset.height)};
    let source=Source{original:hash,original_bytes:asset.byte_size,width,height,orientation:asset.orientation.try_into().map_err(|_|CoreError::Invalid)?,bit_depth:8,icc:None,display_conversion:false};source.validate().map_err(|_|CoreError::Invalid)?;Ok(source)
}
impl GiantImage{async fn check_binding(&self)->Result<()>{self.project.check_open()?;let source=self.source.clone();let document=self.document.clone();self.project.worker.call(move|state|{if project_source(state,&document)?!=source{return Err(CoreError::Invalid)}Ok(())}).await}}
#[uniffi::export]
impl GiantImage{
    pub async fn viewport(&self,view:GiantViewport)->Result<GiantViewportPlan>{self.check_binding().await?;self.worker.call(move|state|{
        let physical_scale=vw_assets::maximum_physical_scale(view.a,view.b,view.c,view.d).map_err(|_|CoreError::Invalid)?;
        let selected=state.manifest.pyramid().viewport(vw_assets::Viewport{left:view.left,top:view.top,right:view.right,bottom:view.bottom,physical_scale}).map_err(|_|CoreError::Invalid)?;
        Ok(GiantViewportPlan{asset_id:hex(&state.manifest.source().original),manifest_hash:state.identity.clone(),level:selected.level,visible:selected.visible.into_iter().map(address).collect(),prefetch:selected.prefetch.into_iter().map(address).collect()})
    }).await}
    pub async fn tile(&self,requested:GiantTileAddress,cancellation:Arc<Cancellation>)->Result<GiantTilePixels>{self.check_binding().await?;self.worker.call(move|state|{
        cancellation.check()?;let key=TileKey{level:requested.level,x:requested.x,y:requested.y};let record=state.manifest.tile(key).map_err(|_|CoreError::Invalid)?;let region=state.manifest.pyramid().region(key).map_err(|_|CoreError::Invalid)?;
        let lease=if let Some(lease)=state.memory.get(&key).map_err(|_|CoreError::Worker)?{lease}else{
            let reservation=state.memory.reserve(key,32*1024*1024).map_err(|_|CoreError::Backpressure)?;
            let encoded=state.cache.get_bounded(&tiles::tile_key(state.manifest.source(),key),vw_assets::MAX_ENCODED_TILE_BYTES as u64)?.ok_or(CoreError::Unsupported)?;record.verify(&encoded).map_err(|_|CoreError::Invalid)?;
            let rgba=tiles::decode(&encoded,key,region)?;drop(encoded);cancellation.check()?;let size=rgba.len()as u64;reservation.commit(rgba,size).map_err(|_|CoreError::Worker)?
        };
        cancellation.check()?;let factor=1u32.checked_shl(u32::from(key.level)).ok_or(CoreError::Invalid)?;let x=region.x.checked_mul(factor).ok_or(CoreError::Invalid)?;let y=region.y.checked_mul(factor).ok_or(CoreError::Invalid)?;
        Ok(GiantTilePixels{asset_id:hex(&state.manifest.source().original),manifest_hash:state.identity.clone(),address:requested,document_x:x,document_y:y,document_width:region.width.saturating_mul(factor).min(state.manifest.source().width-x),document_height:region.height.saturating_mul(factor).min(state.manifest.source().height-y),width:region.width,height:region.height,rgba:lease.payload().map_err(|_|CoreError::Worker)?.clone()})
    }).await}
    pub async fn shutdown(&self)->Result<()>{self.worker.shutdown(|_|Ok(())).await}
}
