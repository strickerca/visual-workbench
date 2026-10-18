//! Shared byte admission for a receiver's actual retained frame owners.
use crate::session::{SessionError, SessionResult};
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

pub(super) const MAX_RECEIVER_BYTES:u64=512*1024*1024;
static PROCESS_BYTES:AtomicU64=AtomicU64::new(0);

#[derive(Clone)]
pub(super) struct ByteBudget(Arc<Budget>);
struct Budget { limit:u64,used:AtomicU64 }
pub(super) struct Bytes { budget:ByteBudget,bytes:u64 }
impl ByteBudget {
    pub fn new(limit:u64)->SessionResult<Self>{
        if limit==0 || limit>MAX_RECEIVER_BYTES{return Err(SessionError::Invalid)}
        Ok(Self(Arc::new(Budget{limit,used:AtomicU64::new(0)})))
    }
    pub fn reserve(&self,bytes:u64)->SessionResult<Bytes>{
        self.0.used.fetch_update(Ordering::AcqRel,Ordering::Acquire,|used|
            used.checked_add(bytes).filter(|next|*next<=self.0.limit))
            .map_err(|_|SessionError::Backpressure)?;
        if PROCESS_BYTES.fetch_update(Ordering::AcqRel,Ordering::Acquire,|used|
            used.checked_add(bytes).filter(|next|*next<=MAX_RECEIVER_BYTES)).is_err(){
            self.0.used.fetch_sub(bytes,Ordering::AcqRel);return Err(SessionError::Backpressure)
        }
        Ok(Bytes{budget:self.clone(),bytes})
    }
    pub fn used(&self)->u64{self.0.used.load(Ordering::Acquire)}
    pub fn limit(&self)->u64{self.0.limit}
}
impl Drop for Bytes {
    fn drop(&mut self){
        self.budget.0.used.fetch_sub(self.bytes,Ordering::AcqRel);
        PROCESS_BYTES.fetch_sub(self.bytes,Ordering::AcqRel);
    }
}

/// Only this module's owner travels across worker/UI boundaries. The frame is
/// destroyed before its byte credit, including when the last owner is dropped
/// after connection shutdown. No bare Arc<DisplayFrame> escapes the adapter.
pub(super) struct OwnedFrame {
    pub(super) frame:Arc<vw_stream::DisplayFrame>,
    stamp:super::wire::CaptureStamp,
    _bytes:Bytes,
}
impl OwnedFrame {
    pub fn new(frame:vw_stream::DisplayFrame,stamp:super::wire::CaptureStamp,bytes:Bytes)->SessionResult<Arc<Self>>{
        if frame.rgba().len() as u64>bytes.bytes{return Err(SessionError::Backpressure)}
        Ok(Arc::new(Self{frame:Arc::new(frame),stamp,_bytes:bytes}))
    }
    pub fn pixels(&self)->&[u8]{self.frame.rgba()}
    pub fn identity(&self)->&vw_stream::FrameIdentity{self.frame.identity()}
    pub fn stamp(&self)->&super::wire::CaptureStamp{&self.stamp}
    pub fn belongs_to(&self,budget:&ByteBudget)->bool{Arc::ptr_eq(&self._bytes.budget.0,&budget.0)}
}
