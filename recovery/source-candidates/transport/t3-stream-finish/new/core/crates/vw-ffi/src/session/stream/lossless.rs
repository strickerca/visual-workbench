//! One freeze request and one transient original per admitted receiver. Exact
//! immutable scope survives late delivery; cancellation joins file work before
//! disposing scratch. No code here publishes an asset or project transaction.
use super::{decoder::OriginalJob,incoming::{ChunkJob,ChunkResult,Incoming,QueuedChunk},
    memory::ByteBudget,receiver::StreamReceiver,verified::Original,wire::{CaptureStamp,OriginalOffer,Scope}};
use crate::session::{SessionError,SessionResult};
use std::{collections::VecDeque,future::{Future,pending},pin::Pin,sync::Arc};
use vw_net::ConnectionSender;
use vw_proto::{Message,v1::{self as pb,envelope::Body}};
use vw_stream::FreezeRequest;

type IoRetirement=Pin<Box<dyn Future<Output=SessionResult<()>>+Send>>;
type Scratch=Pin<Box<dyn Future<Output=SessionResult<tempfile::NamedTempFile>>+Send>>;
pub(super) enum Event {Prepared(SessionResult<tempfile::NamedTempFile>),Chunk(SessionResult<ChunkResult>),Original(SessionResult<Original>),Retired(SessionResult<()>)}
pub(super) struct Lossless {
    request:FreezeRequest,stamp:CaptureStamp,budget:ByteBudget,
    offer:Option<OriginalOffer>,preparing:Option<Scratch>,incoming:Option<Incoming>,chunk:Option<ChunkJob>,
    queued:VecDeque<QueuedChunk>,ready:Option<(OriginalOffer,tempfile::NamedTempFile)>,
    original:Option<OriginalJob>,retiring:Option<IoRetirement>,
    request_pending:bool,blob_pending:bool,ack:Option<pb::StreamBlob>,finished:bool,
}
impl Lossless {
    pub fn new(request:FreezeRequest,stamp:CaptureStamp,budget:ByteBudget)->SessionResult<Self>{
        request.frame.validate().map_err(super::decoder::map_stream)?;
        CaptureStamp::new(stamp.captured_at_ms,&stamp.platform)?;
        if request.epoch==0{return Err(SessionError::Invalid)}
        Ok(Self{request,stamp,budget,offer:None,preparing:None,incoming:None,chunk:None,queued:VecDeque::new(),ready:None,
            original:None,retiring:None,request_pending:true,blob_pending:false,ack:None,finished:false})
    }
    pub fn request(&self)->&FreezeRequest{&self.request}
    /// Only after local request matching may the caller create private scratch.
    /// Duplicate offers never reset receiver offsets or replace the first hash.
    pub fn offer(&mut self,value:&pb::LosslessFrame)->SessionResult<Offer>{
        admit_offer_shape(value)?;
        if value.request_id.as_ref()!=Some(&self.request.draft_id.to_proto())
            ||value.capture_session_id.as_ref()!=Some(&self.request.frame.session.to_proto())
            ||value.frame_id!=self.request.frame.frame_id{return Ok(Offer::Ignored)}
        let Some(offer)=OriginalOffer::parse(value,&self.request.frame,&self.stamp,&self.request.draft_id,self.request.epoch)?else{
            return Ok(if self.offer.is_none(){Offer::Unavailable}else{Offer::Ignored})
        };
        if let Some(prior)=&self.offer{
            if prior!=&offer{return Err(SessionError::Invalid)}
            return Ok(Offer::Ignored)
        }
        if self.finished{return Ok(Offer::Ignored)}
        // Publish the binding before the caller awaits owned scratch creation.
        self.offer=Some(offer.clone());Ok(Offer::Create(offer))
    }
    pub fn install(&mut self,offer:OriginalOffer,scratch:tempfile::NamedTempFile)->SessionResult<()>{
        if self.finished||self.offer.as_ref()!=Some(&offer)||self.incoming.is_some(){return Err(SessionError::Closed)}
        self.incoming=Some(Incoming::new(offer,scratch,self.budget.clone())?);self.blob_pending=true;Ok(())
    }
    pub fn prepare(&mut self,offer:&OriginalOffer,project:Arc<crate::ProjectSession>)->SessionResult<()>{
        if self.finished||self.offer.as_ref()!=Some(offer)||self.preparing.is_some()||self.incoming.is_some(){return Err(SessionError::Closed)}
        // Keep this future across driver select cancellation. Filesystem work
        // belongs to the project actor; it never blocks CONTROL/OPS reception.
        self.preparing=Some(Box::pin(async move{crate::session::replication::transfer_file(&project).await}));Ok(())
    }
    /// True requests joined cancellation of a still-active exact transfer.
    /// A late Cancel cannot revoke a fully verified retained original.
    pub fn receive(&mut self,value:pb::StreamBlob)->SessionResult<bool>{
        let scope=Scope::parse(value.key.as_ref().ok_or(SessionError::Invalid)?)?;
        if value.encoded_len()>128*1024-1024{return Err(SessionError::Backpressure)}
        let Some(offer)=&self.offer else{return Ok(false)};
        if scope!=offer.scope{return Ok(false)}
        if matches!(offer.scope.check(&value)?,pb::stream_blob::Body::Cancel(_)){
            if self.finished{return Ok(false)}
            if let Some(incoming)=&self.incoming{incoming.cancel();}
            if let Some(job)=&self.original{job.cancel();}
            self.finished=true;return Ok(true)
        }
        // Structural admission also applies to ignored late chunks. They cannot
        // revive a finished transfer or alter its already verified original.
        super::incoming::admit_chunk(offer,&value)?;
        if self.finished||self.ready.is_some()||self.original.is_some()||self.incoming.is_none(){return Ok(false)}
        if self.queued.len()>=8{return Err(SessionError::Backpressure)}
        self.queued.push_back(QueuedChunk::new(offer,value,&self.budget)?);Ok(false)
    }
    pub async fn next(&mut self)->Event{
        let Self{preparing,chunk,original,retiring,..}=self;
        tokio::select!{
            result=async{match preparing.as_mut(){Some(value)=>value.as_mut().await,None=>pending().await}}=>Event::Prepared(result),
            result=async{match chunk.as_mut(){Some(value)=>value.receive().await,None=>pending().await}}=>Event::Chunk(result),
            result=async{match original.as_mut(){Some(value)=>value.receive().await,None=>pending().await}}=>Event::Original(result),
            result=async{match retiring.as_mut(){Some(value)=>value.as_mut().await,None=>pending().await}}=>Event::Retired(result),
        }
    }
    /// Return a PNG-verified original only. A verified BLOB ACK alone cannot
    /// authorize lossless quality or durable Save.
    pub fn handle(&mut self,event:Event)->SessionResult<Option<Original>>{
        match event{
            Event::Prepared(result)=>{
                self.preparing=None;
                self.install(self.offer.as_ref().ok_or(SessionError::Worker)?.clone(),result?)?;
            },
            Event::Chunk(result)=>{
                self.chunk=None;let result=result?;self.ack=Some(result.ack);
                if let Some(value)=result.original{
                    self.ready=Some(value);self.queued.clear();
                    if let Some(mut incoming)=self.incoming.take(){self.retiring=Some(Box::pin(async move{incoming.shutdown().await}));}
                }
            },
            Event::Original(result)=>{self.original=None;self.finished=true;return result.map(Some)},
            Event::Retired(result)=>{self.retiring=None;result?;},
        }Ok(None)
    }
    pub fn advance(&mut self,receiver:&mut StreamReceiver)->SessionResult<()>{
        if self.finished{return Ok(())}
        if self.chunk.is_none() && let Some(incoming)=&self.incoming && let Some(value)=self.queued.pop_front(){
            self.chunk=Some(incoming.start_queued(value)?);
        }
        if self.original.is_none()&&!receiver.has_decode() && let Some((offer,scratch))=self.ready.take(){
            self.original=Some(receiver.verify_original(offer,scratch)?);
        }
        Ok(())
    }
    pub fn flush(&mut self,sender:&ConnectionSender)->SessionResult<()>{
        if self.request_pending{
            let value=pb::FreezeRequest{capture_session_id:Some(self.request.frame.session.to_proto()),frame_id:self.request.frame.frame_id,
                request_id:Some(self.request.draft_id.to_proto()),geometry_revision:self.request.frame.geometry.revision(),source_generation:self.request.epoch};
            if !enqueue(sender,Body::FreezeRequest(value))?{return Ok(())}self.request_pending=false;
        }
        if self.blob_pending{
            if let Some(offer)=&self.offer{if !enqueue(sender,Body::StreamBlob(offer.request()))?{return Ok(())}}
            self.blob_pending=false;
        }
        if let Some(ack)=self.ack.as_ref(){if enqueue(sender,Body::StreamBlob(ack.clone()))?{self.ack=None;}}
        Ok(())
    }
    pub async fn shutdown(&mut self)->SessionResult<()>{
        self.finished=true;self.request_pending=false;self.blob_pending=false;
        if let Some(job)=&self.original{job.cancel();}
        let mut result=Ok(());
        if let Some(preparing)=&mut self.preparing{
            match preparing.as_mut().await{Ok(file)=>drop(file),Err(SessionError::Closed)=>{},Err(error)=>result=Err(error),}
        }
        if let Some(incoming)=&mut self.incoming{result=incoming.shutdown().await;}
        if let Some(retiring)=&mut self.retiring{let completed=retiring.as_mut().await;result=result.and(completed);}
        if let Some(job)=&mut self.original{
            // Cancelled or already-completed results are discarded only after
            // the real codec releases its busy admission and owned scratch.
            match job.receive().await{Ok(value)=>drop(value),Err(SessionError::Cancelled)=>{},Err(error)=>result=result.and(Err(error)),}
        }
        self.preparing=None;self.chunk=None;self.incoming=None;self.queued.clear();self.ready=None;self.retiring=None;self.original=None;self.ack=None;
        // The containing receiver still joins the decoder thread on full close.
        result
    }
}
pub(super) enum Offer {Ignored,Unavailable,Create(OriginalOffer)}
pub(super) fn admit_offer_shape(value:&pb::LosslessFrame)->SessionResult<()>{
    if value.encoded_len()>1024||!value.data.is_empty()||value.frame_id==0{return Err(SessionError::Invalid)}
    super::wire::id(value.capture_session_id.as_ref())?;super::wire::id(value.request_id.as_ref())?;
    if value.available{
        Scope::parse(value.transfer.as_ref().ok_or(SessionError::Invalid)?)?;
        if value.format!="png"||value.total_size==0||value.total_size>vw_stream::MAX_LOSSLESS_BYTES as u64
            ||!(1..=super::wire::MAX_CAPTURED_AT_MS).contains(&value.captured_at_ms){return Err(SessionError::Invalid)}
        vw_model::AssetId::try_from(value.asset_id.clone()).map_err(|_|SessionError::Invalid)?;
    }else if value.geometry.is_some()||!value.format.is_empty()||!value.asset_id.is_empty()||value.transfer.is_some()||value.total_size!=0||value.captured_at_ms!=0{
        return Err(SessionError::Invalid)
    }Ok(())
}
fn enqueue(sender:&ConnectionSender,value:Body)->SessionResult<bool>{match sender.enqueue(value){
    Ok(())=>Ok(true),Err(vw_net::NetError::Backpressure)=>Ok(false),Err(error)=>Err(error.into()),
}}

#[cfg(test)]
#[path="lossless_tests.rs"]
mod tests;
