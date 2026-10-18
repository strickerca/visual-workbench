//! The local producer's one verified original and canonical BLOB sender. PNG
//! validation runs on its worker; the network driver performs only bounded
//! in-memory 64 KiB chunk copies, never file reads or provider/OS capture calls.
use super::{memory::{ByteBudget,Bytes},publish_order::OriginalRequest,transfer,wire::{OriginalOffer,Scope}};
use crate::session::{SessionError,SessionResult};
use std::io::Cursor;
use vw_model::{AssetId,Id};
use vw_proto::v1::{self as pb,envelope::Body};

pub(super) struct OriginalPayload {
    pub request:OriginalRequest,pub offer:OriginalOffer,
    encoded:Vec<u8>,_bytes:Bytes,
}
impl OriginalPayload {
    /// The local helper returns exact original bytes. Its identity must be
    /// compared with the requested capture before calling this worker function.
    /// A wrong hash/PNG/dimensions/opaque-pixel contract never reaches the wire.
    pub fn prepare(request:OriginalRequest,generation:u64,encoded:Vec<u8>,budget:&ByteBudget,cancel:&vw_stream::Cancellation)->SessionResult<Self>{
        cancel.check().map_err(super::decoder::map_stream)?;
        if generation==0||encoded.is_empty()||encoded.len()>vw_stream::MAX_LOSSLESS_BYTES||encoded.capacity()>vw_stream::MAX_LOSSLESS_BYTES{return Err(SessionError::Invalid)}
        let bytes=budget.reserve(encoded.capacity()as u64+1024*1024)?;
        let raw=vw_stream::rgba_bytes(request.frame.frame.geometry.width(),request.frame.frame.geometry.height()).map_err(super::decoder::map_stream)? as u64;
        let temporary=budget.reserve(raw.checked_add(16*1024*1024).ok_or(SessionError::Backpressure)?)?;
        let asset=AssetId::hash(&encoded);
        let verified=vw_stream::verify_original(&request.frame.frame,&asset,&encoded,raw+encoded.len()as u64+16*1024*1024,cancel).map_err(super::decoder::map_stream)?;
        drop(verified);drop(temporary);cancel.check().map_err(super::decoder::map_stream)?;
        let transfer=Id::generate(u64::try_from(request.frame.stamp.captured_at_ms).map_err(|_|SessionError::Invalid)?).map_err(|_|SessionError::Worker)?;
        let offer=OriginalOffer{scope:Scope{session:request.frame.frame.session.clone(),frame:request.frame.frame.frame_id,
            geometry_revision:request.frame.frame.geometry.revision(),request:request.draft.clone(),transfer,generation},
            frame:request.frame.frame.clone(),asset,bytes:encoded.len()as u64,stamp:request.frame.stamp.clone()};
        Ok(Self{request,offer,encoded,_bytes:bytes})
    }
}
pub(super) struct Outgoing {
    offer:OriginalOffer,source:Option<Vec<u8>>,
    sender:Option<transfer::Sender<Cursor<Vec<u8>>>>,announced:bool,verified:bool,
    // Cursor and any held encoded chunk are destroyed before their reservation.
    _bytes:Bytes,
}
impl Outgoing {
    pub fn new(payload:OriginalPayload)->Self{
        let OriginalPayload{offer,encoded,_bytes,..}=payload;
        Self{offer,source:Some(encoded),sender:None,announced:false,verified:false,_bytes}
    }
    pub fn offer(&self)->&OriginalOffer{&self.offer}
    #[cfg(test)]
    pub fn verified(&self)->bool{self.verified}
    pub fn receive(&mut self,value:&pb::StreamBlob)->SessionResult<bool>{
        if Scope::parse(value.key.as_ref().ok_or(SessionError::Invalid)?)?!=self.offer.scope{return Ok(false)}
        match self.offer.scope.check(value)?{
            pb::stream_blob::Body::Request(request)=>{
                if request.asset_id!=self.offer.asset.as_str()||request.offset!=0||request.length!=self.offer.bytes||request.priority!=0{return Err(SessionError::Invalid)}
                // A duplicate request cannot rewind an active or completed flow.
                if self.sender.is_none()&&!self.verified{
                    if !self.announced{return Err(SessionError::Invalid)}
                    let bytes=self.source.take().ok_or(SessionError::Closed)?;
                    self.sender=Some(transfer::Sender::new(self.offer.clone(),value,Cursor::new(bytes))?);
                }Ok(false)
            },
            pb::stream_blob::Body::Ack(_)=>{
                let sender=self.sender.as_mut().ok_or(SessionError::Invalid)?;
                sender.acknowledge(value)?;self.verified=sender.verified();Ok(false)
            },
            pb::stream_blob::Body::Cancel(_)=>Ok(!self.verified),
            _=>Err(SessionError::Invalid),
        }
    }
    pub fn flush(&mut self,mut enqueue:impl FnMut(Body)->vw_net::Result<()>)->SessionResult<()>{
        if !self.announced{
            match enqueue(Body::LosslessFrame(self.offer.wire())){
                Ok(())=>self.announced=true,Err(vw_net::NetError::Backpressure)=>return Ok(()),Err(error)=>return Err(error.into()),
            }
        }
        let Some(sender)=&mut self.sender else{return Ok(())};
        // Same canonical four-chunk window. Work per connected tick is fixed.
        for _ in 0..4{
            let Some(chunk)=sender.next()?else{break};
            match enqueue(Body::StreamBlob(chunk.clone())){
                Ok(())=>{},Err(vw_net::NetError::Backpressure)=>{sender.hold(chunk)?;break},Err(error)=>return Err(error.into()),
            }
        }Ok(())
    }
}

#[cfg(test)]
#[path="outgoing_tests.rs"]
mod tests;
