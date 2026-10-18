use super::{hub::{Command,Lease},memory::OwnedFrame,presentation::Draw,verified::Original};
use crate::session::{SessionError,SessionResult};
use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
use tokio::sync::{oneshot,watch,Mutex};
use vw_model::Id;

#[derive(Clone,Debug,PartialEq,Eq,uniffi::Enum)]
pub enum StreamStage { AwaitingSource,Live,Freezing,FrozenLossless,FrozenDegraded,Saved,SourceClosed,Closed }
#[derive(Clone,Debug,PartialEq,uniffi::Record)]
pub struct StreamSignal {
    pub sequence:u64,pub connection_epoch:u64,pub source_generation:u64,
    pub frame_sequence:u64,
    pub stage:StreamStage,pub width:u32,pub height:u32,pub retained_bytes:u64,
    pub failure:Option<String>,
}
impl StreamSignal {
    pub(super) fn empty(epoch:u64)->Self{Self{sequence:1,connection_epoch:epoch,source_generation:0,frame_sequence:0,
        stage:StreamStage::AwaitingSource,width:0,height:0,retained_bytes:0,failure:None}}
}
#[derive(Clone,Debug,PartialEq,uniffi::Record)]
pub struct StreamFrameInfo {
    pub capture_session_id:String,pub frame_id:u64,pub source_generation:u64,
    pub width:u32,pub height:u32,pub host_x:i32,pub host_y:i32,pub dpi_scale:f64,
    pub geometry_revision:u32,pub timestamp_ns:i64,pub captured_at_ms:i64,pub platform:String,
}
impl StreamFrameInfo {
    pub(super) fn from_frame(frame:&OwnedFrame,generation:u64)->SessionResult<Self>{
        let id=frame.identity();let g=&id.geometry;let rect=g.wire().client_rect_host.as_ref().ok_or(SessionError::Invalid)?;
        Ok(Self{capture_session_id:id.session.to_string(),frame_id:id.frame_id,source_generation:generation,
            width:g.width(),height:g.height(),host_x:rect.x,host_y:rect.y,dpi_scale:g.wire().dpi_scale,
            geometry_revision:g.revision(),timestamp_ns:id.timestamp_ns,captured_at_ms:frame.stamp().captured_at_ms,
            platform:frame.stamp().platform.clone()})
    }
}
#[derive(Clone,uniffi::Record)]
pub struct StreamFreezeReceipt {pub draft_id:String,pub frame_id:u64,pub capture_session_id:String,pub source_generation:u64}

#[derive(uniffi::Object)]
pub struct CaptureStream {pub(in crate::session) lease:Arc<Lease>}
impl CaptureStream {pub(super) fn new(lease:Arc<Lease>)->Self{Self{lease}}}
#[uniffi::export]
impl CaptureStream {
    pub fn signal(&self)->StreamSignal{let mut value=self.lease.endpoint.signal.borrow().clone();if self.lease.check().is_err(){value.stage=StreamStage::Closed;}value}
    /// Latest-value wait, cancelled by the shared owner's coroutine. No callback
    /// backlog and no repeated network-runtime admission is created per frame.
    pub async fn wait_signal(&self,after_sequence:u64)->SessionResult<StreamSignal>{
        self.lease.check()?;let _slot=self.lease.slots.clone().try_acquire_owned().map_err(|_|SessionError::Backpressure)?;
        let mut signal=self.lease.endpoint.signal.subscribe();let mut stopped=self.lease.stopped.subscribe();let mut endpoint=self.lease.endpoint.done.subscribe();
        loop{self.lease.check()?;let value=signal.borrow().clone();if value.sequence>after_sequence{return Ok(value)}
            tokio::select!{
                result=signal.changed()=>result.map_err(|_|SessionError::Closed)?,
                _=stopped.changed()=>return Err(SessionError::Closed),
                _=endpoint.changed()=>return Err(SessionError::Closed),
            }}
    }
    /// The returned handle is the sole permission to draw. Offers/decode signals
    /// are not permission. Serialize actual drawing, receipt, and accepted DOWN.
    pub async fn begin_draw(&self)->SessionResult<Option<Arc<CaptureDraw>>>{
        self.lease.check()?;let _slot=self.lease.slots.clone().try_acquire_owned().map_err(|_|SessionError::Backpressure)?;
        let(reply,receive)=oneshot::channel();self.lease.endpoint.send(Command::Draw{lease:self.lease.id,reply})?;
        receive.await.map_err(|_|SessionError::Closed)?
    }
    /// Immutable exact frozen background, never the newest live frame. The
    /// caller replaces its bitmap only after all bounded rows have arrived.
    /// A later verified lossless upgrade returns a new owner with the same
    /// capture identity; an older compressed owner remains honest and usable.
    pub async fn frozen_background(&self)->SessionResult<Option<Arc<CaptureBackground>>>{
        self.lease.check()?;let _slot=self.lease.slots.clone().try_acquire_owned().map_err(|_|SessionError::Backpressure)?;
        let(reply,receive)=oneshot::channel();self.lease.endpoint.send(Command::Background{lease:self.lease.id,reply})?;
        receive.await.map_err(|_|SessionError::Closed)?
    }
    /// Call only for an actual accepted pen DOWN, after the draw receipt settles.
    /// The driver supplies its monotonic clock. Hover/prediction/palm/tool refusal
    /// must not call this API. Remote-edit mode returns None.
    pub async fn first_contact(&self,draft_id:String)->SessionResult<Option<StreamFreezeReceipt>>{
        self.lease.check()?;let _slot=self.lease.slots.clone().try_acquire_owned().map_err(|_|SessionError::Backpressure)?;
        if draft_id.len()!=36{return Err(SessionError::Invalid)}
        let draft=Id::try_from(draft_id).map_err(|_|SessionError::Invalid)?;let(reply,receive)=oneshot::channel();
        self.lease.endpoint.send(Command::Contact{lease:self.lease.id,draft,reply})?;
        Ok(receive.await.map_err(|_|SessionError::Closed)??.map(|v|StreamFreezeReceipt{
            draft_id:v.draft_id.to_string(),frame_id:v.frame.frame_id,capture_session_id:v.frame.session.to_string(),source_generation:v.epoch}))
    }
    /// The real gesture/project draft must already be disposed. This does not
    /// erase edits or save an image; it only resumes the same approved source.
    pub async fn resume_after_draft_disposed(&self)->SessionResult<Option<String>>{
        self.lease.check()?;let _slot=self.lease.slots.clone().try_acquire_owned().map_err(|_|SessionError::Backpressure)?;
        let(reply,receive)=oneshot::channel();self.lease.endpoint.send(Command::Resume{lease:self.lease.id,reply})?;
        Ok(receive.await.map_err(|_|SessionError::Closed)??.map(|v|v.to_string()))
    }
    /// Must settle before closing the link/project. Cancellation seals admission
    /// immediately; repeated calls join the same actual decoder destruction.
    pub async fn close_stream(&self)->SessionResult<()>{self.lease.close().await}
}
impl Drop for CaptureStream{fn drop(&mut self){self.lease.stop();}}

type ReceiptWatch=watch::Receiver<Option<SessionResult<()>>>;
#[derive(uniffi::Object)]
pub struct CaptureDraw {
    lease:Arc<Lease>,frame:Arc<OwnedFrame>,token:vw_stream::PresentationToken,info:StreamFrameInfo,
    settled:Arc<AtomicBool>,receipt:Mutex<Option<(bool,ReceiptWatch)>>,
    // Native row-copy calls are serialized. Returned ABI/platform copies belong
    // to the caller's separately bounded bitmap owner, never this byte ledger.
    copying:std::sync::Mutex<()>,
}
impl CaptureDraw {
    pub(super) fn new(lease:Arc<Lease>,generation:u64,draw:Draw)->SessionResult<Arc<Self>>{
        let info=StreamFrameInfo::from_frame(&draw.frame,generation)?;
        Ok(Arc::new(Self{lease,frame:draw.frame,token:draw.token,info,settled:Arc::new(AtomicBool::new(false)),receipt:Mutex::new(None),copying:std::sync::Mutex::new(())}))
    }
}
#[uniffi::export]
impl CaptureDraw {
    pub fn info(&self)->StreamFrameInfo{self.info.clone()}
    /// At most 256 KiB per call, exact opaque RGBA8 rows. The platform must admit
    /// its complete destination bitmap/GPU copy before beginning any row reads.
    /// Native retained-frame accounting does not claim to own foreign copies.
    pub fn rgba_rows(&self,first_row:u32,row_count:u32)->SessionResult<Vec<u8>>{
        self.lease.check()?;let _copy=self.copying.try_lock().map_err(|_|SessionError::Backpressure)?;
        rows(self.frame.pixels(),&self.info,first_row,row_count)
    }
    /// Idempotent exact choice. A cancelled waiter does not cancel the queued
    /// receipt; a retry joins it. A conflicting shown/not-shown choice refuses.
    pub async fn presented(&self,actually_shown:bool)->SessionResult<()>{
        let mut receive={let mut receipt=self.receipt.lock().await;
            if let Some((choice,receive))=&*receipt{if *choice!=actually_shown{return Err(SessionError::Invalid)}receive.clone()}
            else{self.lease.check()?;let(reply,receive)=watch::channel(None);
                self.lease.endpoint.send(Command::Receipt{lease:self.lease.id,generation:self.info.source_generation,
                    token:self.token.clone(),shown:actually_shown,settled:self.settled.clone(),reply})?;
                *receipt=Some((actually_shown,receive.clone()));receive}
        };
        loop{if let Some(result)=receive.borrow().clone(){return result}receive.changed().await.map_err(|_|SessionError::Closed)?;}
    }
}
impl Drop for CaptureDraw {
    fn drop(&mut self){if !self.settled.load(Ordering::Acquire){self.lease.stop();}}
}

enum BackgroundPixels {Compressed(Arc<OwnedFrame>),Lossless(Arc<Original>)}
#[derive(uniffi::Object)]
pub struct CaptureBackground {
    pub(super) lease:Arc<Lease>,pixels:BackgroundPixels,info:StreamFrameInfo,draft_id:String,
    copying:std::sync::Mutex<()>,
}
impl CaptureBackground {
    pub(super) fn borrowed_pixels(&self)->&[u8]{match &self.pixels{
        BackgroundPixels::Compressed(value)=>value.pixels(),BackgroundPixels::Lossless(value)=>value.pixels().rgba(),
    }}
    pub(super) fn verified_original(&self)->Option<Arc<Original>>{match &self.pixels{
        BackgroundPixels::Compressed(_)=>None,BackgroundPixels::Lossless(value)=>Some(value.clone()),
    }}
    pub(super) fn capture_info(&self,lossless:bool)->crate::workflow::WorkflowResult<vw_proto::v1::CaptureInfo>{
        use crate::workflow::WorkflowError;
        if lossless&&!self.is_lossless(){return Err(WorkflowError::Missing)}
        let identity=match &self.pixels{BackgroundPixels::Compressed(value)=>value.identity(),BackgroundPixels::Lossless(value)=>&value.offer().frame};
        let result=vw_proto::v1::CaptureInfo{capture_session_id:Some(identity.session.to_proto()),frame_id:identity.frame_id,
            geometry:Some(identity.geometry.wire().clone()),platform:self.info.platform.clone(),app_name:String::new(),window_title:String::new(),
            lossless,degraded:!lossless,captured_at_ms:self.info.captured_at_ms,
            stream_save: None,
};
        vw_model::validate_capture(&result)?;Ok(result)
    }
    pub(super) fn new(lease:Arc<Lease>,request:&vw_stream::FreezeRequest,frame:Arc<OwnedFrame>,original:Option<Arc<Original>>)->SessionResult<Arc<Self>>{
        if frame.identity()!=&request.frame{return Err(SessionError::Invalid)}
        let info=StreamFrameInfo::from_frame(&frame,request.epoch)?;
        let pixels=if let Some(original)=original{
            if original.offer().frame!=request.frame||original.offer().scope.request!=request.draft_id
                ||original.offer().scope.generation!=request.epoch||original.offer().stamp!=*frame.stamp(){return Err(SessionError::Invalid)}
            BackgroundPixels::Lossless(original)
        }else{BackgroundPixels::Compressed(frame)};
        Ok(Arc::new(Self{lease,pixels,info,draft_id:request.draft_id.to_string(),copying:std::sync::Mutex::new(())}))
    }
}
#[uniffi::export]
impl CaptureBackground {
    pub fn info(&self)->StreamFrameInfo{self.info.clone()}
    pub fn draft_id(&self)->String{self.draft_id.clone()}
    pub fn is_lossless(&self)->bool{matches!(&self.pixels,BackgroundPixels::Lossless(_))}
    /// Present only after full BLOB hash, PNG dimensions/pixels and capture
    /// provenance verification. A compressed fallback has no original hash.
    pub fn original_asset_id(&self)->Option<String>{match &self.pixels{
        BackgroundPixels::Compressed(_)=>None,BackgroundPixels::Lossless(value)=>Some(value.offer().asset.to_string()),
    }}
    pub fn rgba_rows(&self,first_row:u32,row_count:u32)->SessionResult<Vec<u8>>{
        self.lease.check()?;let _copy=self.copying.try_lock().map_err(|_|SessionError::Backpressure)?;
        let pixels=match &self.pixels{BackgroundPixels::Compressed(value)=>value.pixels(),BackgroundPixels::Lossless(value)=>value.pixels().rgba()};
        rows(pixels,&self.info,first_row,row_count)
    }
}
fn rows(pixels:&[u8],info:&StreamFrameInfo,first_row:u32,row_count:u32)->SessionResult<Vec<u8>>{
    let end=first_row.checked_add(row_count).ok_or(SessionError::Invalid)?;
    let stride=usize::try_from(info.width).map_err(|_|SessionError::Invalid)?.checked_mul(4).ok_or(SessionError::Invalid)?;
    let length=(row_count as usize).checked_mul(stride).ok_or(SessionError::Invalid)?;
    if row_count==0||end>info.height||length>256*1024{return Err(SessionError::Invalid)}
    let offset=(first_row as usize).checked_mul(stride).ok_or(SessionError::Invalid)?;
    let bytes=pixels.get(offset..offset.checked_add(length).ok_or(SessionError::Invalid)?).ok_or(SessionError::Invalid)?;
    let mut result=Vec::new();result.try_reserve_exact(length).map_err(|_|SessionError::Backpressure)?;result.extend_from_slice(bytes);Ok(result)
}
