use crate::{FrameIdentity, OriginalFrame, Result, StreamError, TileRect};
use vw_model::{DeviceId, Id};

pub const REGION_REQUEST_TTL_MS:u64=30_000;
const REQUEST_INTERVAL_MS:u64=1_000;

/// Transport adapters construct this only from the authenticated carrier and
/// captured live connection epoch, never peer-supplied fields in a request.
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct AuthenticatedPeer { pub device:DeviceId,pub epoch:u64 }
impl AuthenticatedPeer {
    fn validate(&self)->Result<()> {if self.epoch==0 {Err(StreamError::Invalid)}else{Ok(())}}
}

#[derive(Debug,Clone,PartialEq)]
pub struct RegionOffer {
    pub request_id:Id,pub offer_id:Id,pub frame:FrameIdentity,pub expires_at_ms:u64,
}
#[derive(Debug,Clone,PartialEq)]
pub struct RegionChoice {pub request_id:Id,pub offer_id:Id,pub frame:FrameIdentity,pub rectangle:TileRect}
/// One-shot authority retains the exact owner-approved original. It cannot
/// broaden to another window, monitor, later frame or a reconnected peer.
pub struct AcceptedRegion {frame:OriginalFrame,rectangle:TileRect,request_id:Id}
impl AcceptedRegion {
    pub fn frame(&self)->&OriginalFrame {&self.frame}
    pub fn rectangle(&self)->TileRect {self.rectangle}
    pub fn request_id(&self)->&Id {&self.request_id}
}
enum Pending {
    Prompt{request:Id,deadline:u64},
    Offered{offer:RegionOffer,frame:OriginalFrame},
}
/// The incoming message creates only a prompt. Only explicit local approval
/// supplies an original and creates a capability; no catalog/desktop bytes are
/// exposed before approval. One slot, fixed lifetime, no persisted grant.
pub struct RegionConsent {
    peer:AuthenticatedPeer,pending:Option<Pending>,last_sequence:u64,last_request_ms:Option<u64>,
}
impl RegionConsent {
    pub fn new(peer:AuthenticatedPeer)->Result<Self> {peer.validate()?;Ok(Self{peer,pending:None,last_sequence:0,last_request_ms:None})}
    fn authenticated(&self,peer:&AuthenticatedPeer)->Result<()> {if peer==&self.peer {Ok(())}else{Err(StreamError::Consent)}}
    fn expire(&mut self,now_ms:u64) {
        let expired=self.pending.as_ref().is_some_and(|p|now_ms>=match p {Pending::Prompt{deadline,..}=>*deadline,Pending::Offered{offer,..}=>offer.expires_at_ms});
        if expired {self.pending=None;}
    }
    /// Returns true only for a newly admitted prompt; lawful retries/rate-limit
    /// drops are harmless and cannot replace the owner's pending selection.
    pub fn request(&mut self,peer:&AuthenticatedPeer,sequence:u64,request:Id,now_ms:u64)->Result<bool> {
        self.authenticated(peer)?;self.expire(now_ms);
        if sequence==0 {return Err(StreamError::Invalid)}
        if sequence<=self.last_sequence {return Ok(false)}
        self.last_sequence=sequence;
        if self.pending.is_some() || self.last_request_ms.is_some_and(|last|now_ms.saturating_sub(last)<REQUEST_INTERVAL_MS) {return Ok(false)}
        let deadline=now_ms.checked_add(REGION_REQUEST_TTL_MS).ok_or(StreamError::Limit)?;
        self.last_request_ms=Some(now_ms);self.pending=Some(Pending::Prompt{request,deadline});Ok(true)
    }
    /// Only the local explicit consent UI calls this method after selecting and
    /// reviewing a real source; a network message must never route here.
    pub fn approve(&mut self,request:&Id,offer_id:Id,frame:OriginalFrame,now_ms:u64)->Result<RegionOffer> {
        self.expire(now_ms);
        let Some(Pending::Prompt{request:expected,deadline})=&self.pending else{return Err(StreamError::Consent)};
        if request!=expected {return Err(StreamError::Consent)}
        let offer=RegionOffer{request_id:request.clone(),offer_id,frame:frame.identity().clone(),expires_at_ms:*deadline};
        self.pending=Some(Pending::Offered{offer:offer.clone(),frame});Ok(offer)
    }
    pub fn accept(&mut self,peer:&AuthenticatedPeer,choice:&RegionChoice,now_ms:u64)->Result<AcceptedRegion> {
        self.authenticated(peer)?;self.expire(now_ms);
        let Some(Pending::Offered{offer,frame})=&self.pending else{return Err(StreamError::Consent)};
        if choice.request_id!=offer.request_id || choice.offer_id!=offer.offer_id || choice.frame!=offer.frame {return Err(StreamError::Stale)}
        let r=choice.rectangle;let g=&frame.identity().geometry;
        if r.width==0 || r.height==0 || r.x.checked_add(r.width).is_none_or(|n|n>g.width())
            || r.y.checked_add(r.height).is_none_or(|n|n>g.height()) {return Err(StreamError::Invalid)}
        let Some(Pending::Offered{offer,frame})=self.pending.take() else{return Err(StreamError::Consent)};
        Ok(AcceptedRegion{frame,rectangle:r,request_id:offer.request_id})
    }
    pub fn deny(&mut self,request:&Id)->bool {
        let matches=self.pending.as_ref().is_some_and(|pending|match pending {Pending::Prompt{request:r,..}=>r==request,Pending::Offered{offer,..}=>&offer.request_id==request});
        if matches {self.pending=None;}matches
    }
    pub fn cancel(&mut self,peer:&AuthenticatedPeer,request:&Id)->Result<bool> {self.authenticated(peer)?;Ok(self.deny(request))}
    pub fn revoke(&mut self) {self.pending=None;}
    pub fn pending_request(&mut self,now_ms:u64)->Option<&Id> {
        self.expire(now_ms);self.pending.as_ref().map(|p|match p {Pending::Prompt{request,..}=>request,Pending::Offered{offer,..}=>&offer.request_id})
    }
}
