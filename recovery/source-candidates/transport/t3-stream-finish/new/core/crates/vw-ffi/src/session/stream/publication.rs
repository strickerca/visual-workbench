//! Connected-loop publisher owner. Worker results, CONTROL hints and bounded
//! MEDIA/BLOB queue feedback never run an OS capture or select a remote target.
use super::{hub::Lease,memory::{ByteBudget,Bytes},outgoing::Outgoing,
    publication_api::{CaptureOriginalRequest,CapturePublicationSignal,CapturePublishSource},
    publish_order::{FreezeAdmission,OriginalRequest,PreparedFrame,PublishOrder},publish_worker::{Job,Prepared,PublishWorker}};
use crate::session::{SessionError,SessionResult};
use std::sync::{Arc,Weak,atomic::Ordering};
use tokio::sync::{oneshot,watch,OwnedSemaphorePermit};
use vw_model::AssetId;
use vw_net::ConnectionSender;
use vw_proto::{Message,v1::{self as pb,envelope::Body}};

pub(in crate::session) enum Command {
    Enable{lease:Weak<Lease>,source:CapturePublishSource,budget:u64,signal:watch::Sender<CapturePublicationSignal>,reply:oneshot::Sender<SessionResult<()>>},
    Frame{lease:u64,encoded:Vec<u8>,permit:OwnedSemaphorePermit,reply:oneshot::Sender<SessionResult<bool>>},
    Original{lease:u64,request_sequence:u64,identity:crate::CaptureImportDescriptor,encoded:Vec<u8>,permit:OwnedSemaphorePermit,reply:oneshot::Sender<SessionResult<()>>},
    Unavailable{lease:u64,request_sequence:u64,reply:oneshot::Sender<SessionResult<()>>},
}
impl Command {
    /// Settles only admission that never acquired an actual worker. An active
    /// publication's completion must come from shutdown after worker join.
    pub(super) fn refuse(self,error:SessionError){match self{
        Self::Enable{lease,reply,..}=>{
            if let Some(owner)=lease.upgrade(){owner.stop();let _=owner.done.send_replace(Some(Ok(())));}
            let _=reply.send(Err(error));
        },
        Self::Frame{reply,..}=>{let _=reply.send(Err(error));},
        Self::Original{reply,..}|Self::Unavailable{reply,..}=>{let _=reply.send(Err(error));},
    }}
}
enum Reply {Frame(oneshot::Sender<SessionResult<bool>>),Original{request:OriginalRequest,asset:Option<AssetId>,reply:oneshot::Sender<SessionResult<()>>}}
struct Active {job:Job,reply:Reply,_permit:OwnedSemaphorePermit}
pub(super) struct Event(SessionResult<Prepared>);
pub(super) struct Publication {
    lease:Weak<Lease>,order:PublishOrder,worker:PublishWorker,signal:watch::Sender<CapturePublicationSignal>,
    active:Option<Active>,ready:Option<(PreparedFrame,oneshot::Sender<SessionResult<bool>>,OwnedSemaphorePermit)>,
    outgoing:Option<Outgoing>,unavailable:Option<pb::LosslessFrame>,failure:Option<String>,
    // One API input permit persists through the actual job + queue result. Its
    // complete largest incoming Vec is reserved independently from conversion.
    _input:Bytes,
}
impl Publication {
    pub fn new(lease:Weak<Lease>,source:CapturePublishSource,budget:u64,signal:watch::Sender<CapturePublicationSignal>)->SessionResult<Self>{
        let owner=lease.upgrade().ok_or(SessionError::Closed)?;owner.check()?;
        let source=source.source(owner.id)?;let budget=ByteBudget::new(budget)?;
        let input=budget.reserve(vw_stream::MAX_LOSSLESS_BYTES as u64+64*1024)?;
        let order=PublishOrder::new(source,&budget)?;let worker=PublishWorker::new(budget)?;
        Ok(Self{lease,order,worker,signal,active:None,ready:None,outgoing:None,unavailable:None,failure:None,_input:input})
    }
    pub fn stopped(&self)->bool{self.lease.upgrade().is_none_or(|v|v.closing.load(Ordering::Acquire)||v.endpoint.stop.load(Ordering::Acquire))}
    pub fn source_closed(&self)->pb::StreamControl{self.order.source().control("source_closed")}
    fn check(&self,id:u64)->SessionResult<()>{let owner=self.lease.upgrade().ok_or(SessionError::Closed)?;if owner.id!=id{return Err(SessionError::Closed)}owner.check()}
    pub fn has_work(&self)->bool{self.active.is_some()}
    pub async fn next(&mut self)->Event{Event(match &mut self.active{Some(active)=>active.job.receive().await,None=>Err(SessionError::Closed)})}
    pub fn command(&mut self,command:Command)->SessionResult<()>{
        match command{
            command@Command::Enable{..}=>command.refuse(SessionError::Backpressure),
            Command::Frame{lease,encoded,permit,reply}=>{
                if reply.is_closed(){return Ok(())}
                let result=(||{self.check(lease)?;if self.active.is_some()||self.ready.is_some(){return Err(SessionError::Backpressure)}self.worker.frame(encoded)})();
                match result{Ok(job)=>self.active=Some(Active{job,reply:Reply::Frame(reply),_permit:permit}),Err(error)=>{drop(permit);let _=reply.send(Err(error));}}
            },
            Command::Original{lease,request_sequence,identity,encoded,permit,reply}=>{
                if reply.is_closed(){return Ok(())}
                let result=(||{
                    self.check(lease)?;if self.active.is_some()||self.ready.is_some()||self.outgoing.is_some(){return Err(SessionError::Backpressure)}
                    let request=self.order.request().filter(|v|v.sequence==request_sequence).ok_or(SessionError::Closed)?.clone();
                    let asset=check_original(&request,&identity)?;
                    let job=self.worker.original(request.clone(),self.order.source().generation,encoded)?;Ok((job,request,asset))
                })();
                match result{Ok((job,request,asset))=>self.active=Some(Active{job,reply:Reply::Original{request,asset,reply},_permit:permit}),Err(error)=>{drop(permit);let _=reply.send(Err(error));}}
            },
            Command::Unavailable{lease,request_sequence,reply}=>{
                if reply.is_closed(){return Ok(())}
                let result=self.check(lease).and_then(|()|{
                    let request=self.order.request().filter(|v|v.sequence==request_sequence).ok_or(SessionError::Closed)?;
                    if self.outgoing.is_some()||self.active.as_ref().is_some_and(|v|matches!(&v.reply,Reply::Original{..})){return Err(SessionError::Backpressure)}
                    self.unavailable=Some(super::wire::unavailable(&request.frame.frame,&request.draft));Ok(())
                });let _=reply.send(result);
            },
        }Ok(())
    }
    pub fn handle(&mut self,event:Event)->SessionResult<()>{
        let Active{reply,_permit,..}=self.active.take().ok_or(SessionError::Worker)?;
        match reply{
            Reply::Frame(reply)=>{
                if reply.is_closed(){return Ok(())}
                match event.0{
                    Ok(Prepared::Frame(frame))if !self.stopped()=>self.ready=Some((frame,reply,_permit)),
                    Err(error)=>{drop(_permit);let _=reply.send(Err(error));},
                    _=>{drop(_permit);let _=reply.send(Err(SessionError::Closed));},
                }
            },
            Reply::Original{request,asset,reply}=>{
                let result=match event.0{
                    Ok(Prepared::Original(payload))if !reply.is_closed()&&!self.stopped()&&self.order.current_request(&request)
                        &&payload.request==request&&asset.as_ref().is_none_or(|id|id==&payload.offer.asset)=>{self.unavailable=None;self.outgoing=Some(Outgoing::new(payload));Ok(())},
                    Err(error)=>Err(error),_=>Err(SessionError::Closed),
                };drop(_permit);let _=reply.send(result);
            },
        }self.publish();Ok(())
    }
    pub fn control(&mut self,value:&pb::StreamControl)->SessionResult<bool>{
        if !self.order.source().matches_control(value)?{return Ok(false)}
        self.order.control(value)?;
        if matches!(value.action.as_str(),"resume"|"stop"){
            self.outgoing=None;self.unavailable=None;
            if let Some(active)=&self.active&&matches!(&active.reply,Reply::Original{..}){active.job.cancel();}
        }
        if self.order.closed()&&let Some(lease)=self.lease.upgrade(){lease.stop();}
        self.publish();Ok(true)
    }
    pub fn freeze(&mut self,value:&pb::FreezeRequest,now_ms:u64)->SessionResult<()>{
        match self.order.freeze(value,now_ms)?{FreezeAdmission::Ignored=>{},FreezeAdmission::Unavailable(value)=>self.unavailable=Some(value),FreezeAdmission::Request(_)=>{}}
        self.publish();Ok(())
    }
    pub fn blob(&mut self,value:&pb::StreamBlob)->SessionResult<bool>{
        let scope=super::wire::Scope::parse(value.key.as_ref().ok_or(SessionError::Invalid)?)?;
        if value.encoded_len()>128*1024-1024{return Err(SessionError::Invalid)}
        let Some(outgoing)=&mut self.outgoing else{return Ok(false)};
        if outgoing.offer().scope!=scope{return Ok(false)}
        if outgoing.receive(value)?{self.outgoing=None;}
        Ok(true)
    }
    pub fn tick(&mut self,sender:&ConnectionSender)->SessionResult<()>{
        self.flush(|body|sender.enqueue(body))
    }
    pub(super) fn flush(&mut self,mut enqueue:impl FnMut(Body)->vw_net::Result<()>)->SessionResult<()>{
        if self.stopped(){return Ok(())}
        if !self.order.announce(&mut enqueue)?{return Ok(())}
        if let Some((frame,reply,permit))=self.ready.take(){
            let result=if reply.is_closed(){Ok(false)}else{self.order.publish(frame,&mut enqueue)};
            drop(permit);let _=reply.send(result);
        }
        if let Some(outgoing)=&mut self.outgoing{outgoing.flush(&mut enqueue)?;}
        if let Some(value)=self.unavailable.take(){match enqueue(Body::LosslessFrame(value.clone())){
            Ok(())=>{},Err(vw_net::NetError::Backpressure)=>self.unavailable=Some(value),Err(error)=>return Err(error.into()),
        }}self.publish();Ok(())
    }
    fn publish(&self){
        let mut value=self.signal.borrow().clone();value.paused=self.order.paused();value.closed=self.order.closed()||self.stopped();
        value.repair_sequence=self.order.repair();value.failure=self.failure.clone();
        value.original=self.order.request().map(|request|CaptureOriginalRequest{sequence:request.sequence,draft_id:request.draft.to_string(),frame:frame_info(request,self.order.source().generation)});
        self.signal.send_if_modified(|prior|{value.sequence=prior.sequence;if *prior==value{return false}let Some(next)=prior.sequence.checked_add(1)else{return false};value.sequence=next;*prior=value;true});
    }
    pub async fn shutdown(&mut self)->SessionResult<()>{
        self.order.stop();if let Some(lease)=self.lease.upgrade(){lease.stop();}self.publish();
        if let Some(active)=&self.active{active.job.cancel();}
        let result=self.worker.shutdown().await;self.active=None;self.ready=None;self.outgoing=None;self.unavailable=None;result
    }
    pub fn completion(&self)->Weak<Lease>{self.lease.clone()}
}
fn frame_info(request:&OriginalRequest,generation:u64)->super::api::StreamFrameInfo{
    let frame=&request.frame.frame;let wire=frame.geometry.wire();let rect=wire.client_rect_host.as_ref();
    super::api::StreamFrameInfo{capture_session_id:frame.session.to_string(),frame_id:frame.frame_id,source_generation:generation,width:frame.geometry.width(),height:frame.geometry.height(),
        host_x:rect.map_or(0,|v|v.x),host_y:rect.map_or(0,|v|v.y),dpi_scale:wire.dpi_scale,geometry_revision:wire.geometry_revision,timestamp_ns:frame.timestamp_ns,
        captured_at_ms:request.frame.stamp.captured_at_ms,platform:request.frame.stamp.platform.clone()}
}
fn check_original(request:&OriginalRequest,identity:&crate::CaptureImportDescriptor)->SessionResult<Option<AssetId>>{
    let frame=&request.frame.frame;let geometry=frame.geometry.wire();let rect=geometry.client_rect_host.as_ref().ok_or(SessionError::Invalid)?;
    if !identity.explicit_owner_action||identity.capture_session_id!=frame.session.as_str()||identity.frame_id!=frame.frame_id||identity.geometry_revision!=geometry.geometry_revision
        ||identity.platform!=request.frame.stamp.platform||identity.source_kind!=geometry.source_kind||identity.physical_x!=rect.x||identity.physical_y!=rect.y
        ||identity.width!=frame.geometry.width()||identity.height!=frame.geometry.height()||identity.dpi_scale!=geometry.dpi_scale
        ||i64::try_from(identity.monotonic_timestamp_ns).ok()!=Some(frame.timestamp_ns)||identity.captured_at_ms!=request.frame.stamp.captured_at_ms
        ||identity.window_handle!=geometry.window_handle||identity.monitor_id!=geometry.monitor_id{return Err(SessionError::Invalid)}
    identity.expected_source_asset_id.clone().map(|v|AssetId::try_from(v).map_err(|_|SessionError::Invalid)).transpose()
}
