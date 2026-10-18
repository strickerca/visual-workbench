//! Local producer packet/original admission on one owned worker. Cancellation
//! retains input allocation and the worker slot until actual work settles.
use super::{memory::ByteBudget,publish_order::{OriginalRequest,PreparedFrame},outgoing::OriginalPayload};
use crate::{session::{SessionError,SessionResult},worker::Worker};
use std::sync::{Arc,atomic::{AtomicBool,AtomicUsize,Ordering}};
use tokio::sync::oneshot;
static ACTIVE:AtomicUsize=AtomicUsize::new(0);
struct Permit;
impl Drop for Permit{fn drop(&mut self){ACTIVE.fetch_sub(1,Ordering::AcqRel);}}
struct Busy(Arc<AtomicBool>);
impl Drop for Busy{fn drop(&mut self){self.0.store(false,Ordering::Release);}}

pub(super) enum Prepared {Frame(PreparedFrame),Original(OriginalPayload)}
pub(super) struct Job {receive:Option<oneshot::Receiver<SessionResult<Prepared>>>,cancel:Arc<vw_stream::Cancellation>}
impl Job {
    pub fn cancel(&self){self.cancel.cancel();}
    pub async fn receive(&mut self)->SessionResult<Prepared>{
        let result=self.receive.as_mut().ok_or(SessionError::Closed)?.await.map_err(|_|SessionError::Worker);self.receive=None;result?
    }
}
impl Drop for Job{fn drop(&mut self){self.cancel.cancel();}}
pub(super) struct PublishWorker {
    worker:Worker<Permit>,budget:ByteBudget,busy:Arc<AtomicBool>,active:Option<Arc<vw_stream::Cancellation>>,closed:bool,
}
impl PublishWorker {
    pub fn new(budget:ByteBudget)->SessionResult<Self>{
        ACTIVE.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<2).then_some(n+1)).map_err(|_|SessionError::Backpressure)?;
        let worker=Worker::new("vw-stream-publish",Permit,1).map_err(SessionError::from)?;
        Ok(Self{worker,budget,busy:Arc::new(AtomicBool::new(false)),active:None,closed:false})
    }
    pub fn frame(&mut self,encoded:Vec<u8>)->SessionResult<Job>{
        self.start(encoded,vw_proto::capture_admission::MAX_MEDIA_BYTES,|bytes,budget,cancel|{
            cancel.check().map_err(super::decoder::map_stream)?;let frame=PreparedFrame::decode(bytes,budget)?;
            cancel.check().map_err(super::decoder::map_stream)?;Ok(Prepared::Frame(frame))
        })
    }
    pub fn original(&mut self,request:OriginalRequest,generation:u64,encoded:Vec<u8>)->SessionResult<Job>{
        self.start(encoded,vw_stream::MAX_LOSSLESS_BYTES,move|bytes,budget,cancel|
            OriginalPayload::prepare(request,generation,bytes,budget,cancel).map(Prepared::Original))
    }
    fn start(&mut self,encoded:Vec<u8>,limit:usize,prepare:impl FnOnce(Vec<u8>,&ByteBudget,&vw_stream::Cancellation)->SessionResult<Prepared>+Send+'static)->SessionResult<Job>{
        if self.closed{return Err(SessionError::Closed)}
        if encoded.is_empty()||encoded.len()>limit||encoded.capacity()>limit{return Err(SessionError::Backpressure)}
        self.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).map_err(|_|SessionError::Backpressure)?;
        let busy=Busy(self.busy.clone());let queued=self.budget.reserve(encoded.capacity()as u64)?;
        let budget=self.budget.clone();let cancel=Arc::new(vw_stream::Cancellation::default());let signal=cancel.clone();
        let(send,receive)=oneshot::channel();self.worker.enqueue(Box::new(move|_|{
            // The queued reservation remains through conversion, covering ABI
            // input/parsed/decoded overlap conservatively. No allocation credit
            // returns while a cancelled producer still owns the input bytes.
            let result=signal.check().map_err(super::decoder::map_stream).and_then(|()|prepare(encoded,&budget,&signal));
            drop(queued);drop(busy);let _=send.send(result);
        })).map_err(SessionError::from)?;
        self.active=Some(cancel.clone());Ok(Job{receive:Some(receive),cancel})
    }
    pub async fn shutdown(&mut self)->SessionResult<()>{
        self.closed=true;if let Some(cancel)=&self.active{cancel.cancel();}
        self.worker.shutdown(|_|Ok(())).await.map_err(SessionError::from)
    }
}
impl Drop for PublishWorker{fn drop(&mut self){if let Some(cancel)=&self.active{cancel.cancel();}}}

#[cfg(test)]
#[path="publish_worker_tests.rs"]
mod tests;
