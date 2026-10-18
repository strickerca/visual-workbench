//! Lossless originals stay in project-owned private transfer scratch. The BLOB
//! hash is necessary but not sufficient: this owner also validates the exact
//! requested frame and bounded PNG pixels before allowing the lossless swap.
use super::{decoder::map_stream,memory::{ByteBudget,Bytes},wire::OriginalOffer};
use crate::session::{SessionError,SessionResult};
use std::io::{Read,Seek,SeekFrom};
use vw_stream::{Cancellation,VerifiedOriginal};

pub(super) struct Original {
    pixels:VerifiedOriginal,
    offer:OriginalOffer,
    // Encoded source is never re-encoded. Only the existing canonical save
    // adapter may copy these exact verified bytes into immutable BlobStore.
    scratch:tempfile::NamedTempFile,
    _bytes:Bytes,
}
impl Original {
    pub fn pixels(&self)->&VerifiedOriginal{&self.pixels}
    pub fn offer(&self)->&OriginalOffer{&self.offer}
    pub fn encoded_file(&self)->&std::fs::File{self.scratch.as_file()}
}

pub(super) fn verify(offer:OriginalOffer,mut scratch:tempfile::NamedTempFile,budget:ByteBudget,cancel:&Cancellation)->SessionResult<Original>{
    cancel.check().map_err(map_stream)?;
    if super::wire::CaptureStamp::new(offer.stamp.captured_at_ms,&offer.stamp.platform)?!=offer.stamp
        ||offer.scope.generation==0||!offer.scope.matches(&offer.frame,&offer.scope.request){return Err(SessionError::Invalid)}
    let metadata=scratch.as_file().metadata().map_err(|_|SessionError::Storage)?;
    if !metadata.is_file() || metadata.len()!=offer.bytes || offer.bytes==0 || offer.bytes>vw_stream::MAX_LOSSLESS_BYTES as u64{
        return Err(SessionError::Invalid)
    }
    let raw=vw_stream::rgba_bytes(offer.frame.geometry.width(),offer.frame.geometry.height()).map_err(map_stream)? as u64;
    let retained=budget.reserve(raw.checked_add(4096).ok_or(SessionError::Backpressure)?)?;
    let scratch_bytes=offer.bytes.checked_add(16*1024*1024).ok_or(SessionError::Backpressure)?;
    let temporary=budget.reserve(scratch_bytes)?;
    let mut encoded=Vec::new();encoded.try_reserve_exact(offer.bytes as usize).map_err(|_|SessionError::Backpressure)?;
    if encoded.capacity()>offer.bytes as usize{return Err(SessionError::Backpressure)}
    encoded.resize(offer.bytes as usize,0);scratch.as_file_mut().seek(SeekFrom::Start(0)).map_err(|_|SessionError::Storage)?;
    for chunk in encoded.chunks_mut(64*1024){
        cancel.check().map_err(map_stream)?;scratch.as_file_mut().read_exact(chunk).map_err(|_|SessionError::Storage)?;
    }
    let mut extra=[0u8;1];if scratch.as_file_mut().read(&mut extra).map_err(|_|SessionError::Storage)?!=0{return Err(SessionError::Invalid)}
    let pixels=vw_stream::verify_original(&offer.frame,&offer.asset,&encoded,raw+scratch_bytes,cancel).map_err(map_stream)?;
    drop(encoded);drop(temporary);
    cancel.check().map_err(map_stream)?;
    scratch.as_file_mut().seek(SeekFrom::Start(0)).map_err(|_|SessionError::Storage)?;
    Ok(Original{pixels,offer,scratch,_bytes:retained})
}
