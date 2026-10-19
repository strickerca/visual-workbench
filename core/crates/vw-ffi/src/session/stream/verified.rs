//! Lossless originals stay in project-owned private transfer scratch. The BLOB
//! hash is necessary but not sufficient: this owner also validates the exact
//! requested frame and bounded PNG pixels before allowing the lossless swap.
use super::{
    decoder::map_stream,
    memory::{ByteBudget, Bytes},
    wire::OriginalOffer,
};
use crate::session::{SessionError, SessionResult};
use std::io::{Read, Seek, SeekFrom};
use vw_stream::{Cancellation, VerifiedOriginal};

pub(super) struct Original {
    pixels: std::sync::Arc<VerifiedOriginal>,
    offer: OriginalOffer,
    // Encoded source is never re-encoded. Only the existing canonical save
    // adapter may copy these exact verified bytes into immutable BlobStore.
    scratch: tempfile::NamedTempFile,
    _bytes: Bytes,
}
impl Original {
    pub fn pixels(&self) -> &std::sync::Arc<VerifiedOriginal> {
        &self.pixels
    }
    pub fn offer(&self) -> &OriginalOffer {
        &self.offer
    }
    pub fn encoded_file(&self) -> &std::fs::File {
        self.scratch.as_file()
    }
}

pub(super) fn verify(
    offer: OriginalOffer,
    mut scratch: tempfile::NamedTempFile,
    budget: ByteBudget,
    cancel: &Cancellation,
) -> SessionResult<Original> {
    cancel.check().map_err(map_stream)?;
    if super::wire::CaptureStamp::new(offer.stamp.captured_at_ms, &offer.stamp.platform)?
        != offer.stamp
        || offer.scope.generation == 0
        || !offer.scope.matches(&offer.frame, &offer.scope.request)
    {
        return Err(SessionError::Invalid);
    }
    let metadata = scratch
        .as_file()
        .metadata()
        .map_err(|_| SessionError::Storage)?;
    if !metadata.is_file()
        || metadata.len() != offer.bytes
        || offer.bytes == 0
        || offer.bytes > vw_stream::MAX_LOSSLESS_BYTES as u64
    {
        return Err(SessionError::Invalid);
    }
    let retained = budget.reserve(0)?;
    let mut encoded = vw_stream::BudgetedBytes::allocate(
        budget.pool(),
        vw_stream::MemoryKind::Transfer,
        offer.bytes as usize,
        cancel,
    )
    .map_err(map_stream)?;
    scratch
        .as_file_mut()
        .seek(SeekFrom::Start(0))
        .map_err(|_| SessionError::Storage)?;
    for chunk in encoded.bytes_mut().chunks_mut(64 * 1024) {
        cancel.check().map_err(map_stream)?;
        scratch
            .as_file_mut()
            .read_exact(chunk)
            .map_err(|_| SessionError::Storage)?;
    }
    let mut extra = [0u8; 1];
    if scratch
        .as_file_mut()
        .read(&mut extra)
        .map_err(|_| SessionError::Storage)?
        != 0
    {
        return Err(SessionError::Invalid);
    }
    let pixels = std::sync::Arc::new(
        vw_stream::verify_original_budgeted(
            &offer.frame,
            &offer.asset,
            &encoded,
            budget.pool(),
            cancel,
        )
        .map_err(map_stream)?,
    );
    drop(encoded);
    cancel.check().map_err(map_stream)?;
    scratch
        .as_file_mut()
        .seek(SeekFrom::Start(0))
        .map_err(|_| SessionError::Storage)?;
    Ok(Original {
        pixels,
        offer,
        scratch,
        _bytes: retained,
    })
}
