//! Reuse the verified ordinary BLOB window/hash implementation while retaining
//! an independent exact capture scope. The driver supplies owned private scratch
//! and never routes these messages through project asset installation.
use super::wire::OriginalOffer;
use crate::session::{SessionError, SessionResult};
use std::io::{Read, Seek, Write};
use vw_proto::v1 as pb;

pub(super) struct Sender<R> {
    offer: OriginalOffer,
    sender: vw_net::BlobSender<R>,
    held: Option<pb::StreamBlob>,
}
impl<R: Read + Seek> Sender<R> {
    pub fn new(offer: OriginalOffer, request: &pb::StreamBlob, source: R) -> SessionResult<Self> {
        let pb::stream_blob::Body::Request(request) = offer.scope.check(request)? else { return Err(SessionError::Invalid); };
        // Capture transfer resume is connection-local. New/reconnected requests
        // cannot acquire bytes using an arbitrary project content hash.
        if request.asset_id != offer.asset.as_str() || request.offset != 0 || request.length != offer.bytes
            || request.priority != 0 { return Err(SessionError::Invalid); }
        let sender = vw_net::BlobSender::resume(source, offer.asset.clone(), offer.bytes, 0)?;
        Ok(Self { offer, sender, held: None })
    }
    /// On queue backpressure put this exact message back with hold(); never
    /// advance again and lose bytes which the cumulative ACK cannot cover.
    pub fn next(&mut self) -> SessionResult<Option<pb::StreamBlob>> {
        if let Some(value) = self.held.take() { return Ok(Some(value)); }
        Ok(self.sender.next_chunk()?.map(|chunk| self.offer.scope.envelope(pb::stream_blob::Body::Chunk(chunk))))
    }
    pub fn hold(&mut self, value: pb::StreamBlob) -> SessionResult<()> {
        if self.held.is_some() || !matches!(self.offer.scope.check(&value)?, pb::stream_blob::Body::Chunk(_)) {
            return Err(SessionError::Invalid);
        }
        self.held = Some(value); Ok(())
    }
    pub fn acknowledge(&mut self, value: &pb::StreamBlob) -> SessionResult<()> {
        let pb::stream_blob::Body::Ack(ack) = self.offer.scope.check(value)? else { return Err(SessionError::Invalid); };
        self.sender.acknowledge(ack)?; Ok(())
    }
    pub fn verified(&self) -> bool { self.sender.is_verified() }
}

pub(super) struct Receiver<W> {
    offer: OriginalOffer,
    receiver: Option<vw_net::BlobReceiver<W>>,
    complete: bool,
}
impl<W: Read + Write + Seek> Receiver<W> {
    pub fn new(offer: OriginalOffer, scratch: W) -> SessionResult<Self> {
        let receiver = vw_net::BlobReceiver::resume(scratch, offer.asset.clone(), offer.bytes, 0,
            vw_stream::MAX_LOSSLESS_BYTES as u64)?;
        Ok(Self { offer, receiver: Some(receiver), complete: false })
    }
    pub fn receive(&mut self, value: &pb::StreamBlob) -> SessionResult<pb::StreamBlob> {
        let pb::stream_blob::Body::Chunk(chunk) = self.offer.scope.check(value)? else { return Err(SessionError::Invalid); };
        let result = self.receiver.as_mut().ok_or(SessionError::Closed)?.receive(chunk);
        let ack = match result { Ok(value) => value, Err(error) => {
            // A failed write/hash may have modified the staging file. Consume
            // receiver authority now; only owned scratch cleanup may use it.
            self.receiver = None; return Err(error.into());
        }};
        self.complete |= ack.verified;
        Ok(self.offer.scope.envelope(pb::stream_blob::Body::Ack(ack)))
    }
    pub fn complete(&self) -> bool { self.complete }
    /// BLOB verification is not pixel/PNG verification. The driver must invoke
    /// vw_stream::verify_original before a lossless swap or durable Save.
    pub fn finish(mut self) -> SessionResult<(OriginalOffer, W)> {
        if !self.complete { return Err(SessionError::Invalid); }
        let sink = self.receiver.take().ok_or(SessionError::Closed)?.finish()?;
        Ok((self.offer, sink))
    }
}
