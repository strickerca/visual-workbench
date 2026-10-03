use crate::{NetError, Result};
use std::io::{Read, Seek, SeekFrom, Write};
use vw_model::AssetId;
use vw_proto::v1;
pub const BLOB_CHUNK_BYTES: usize = 64 * 1024;
pub const BLOB_WINDOW_BYTES: u64 = 256 * 1024;

/// A bounded resumable transfer. A caller publishes the staging sink only after
/// `finish`; bytes are hashed while streaming, never accumulated in memory here.
pub struct BlobReceiver<W> {
    sink: W,
    asset: AssetId,
    size: u64,
    next: u64,
    hash: blake3::Hasher,
}
impl<W: Read + Write + Seek> BlobReceiver<W> {
    pub fn resume(mut sink: W, asset: AssetId, size: u64, next: u64, maximum: u64) -> Result<Self> {
        if size > maximum || next > size || sink.seek(SeekFrom::End(0))? != next {
            return Err(NetError::Invalid("blob resume bounds"));
        }
        sink.seek(SeekFrom::Start(0))?;
        let mut hash = blake3::Hasher::new();
        let mut remaining = next;
        let mut buffer = [0_u8; BLOB_CHUNK_BYTES];
        while remaining > 0 {
            let n = remaining.min(buffer.len() as u64) as usize;
            sink.read_exact(&mut buffer[..n])?;
            hash.update(&buffer[..n]);
            remaining -= n as u64;
        }
        Ok(Self {
            sink,
            asset,
            size,
            next,
            hash,
        })
    }
    pub const fn offset(&self) -> u64 {
        self.next
    }
    pub fn receive(&mut self, chunk: &v1::BlobChunk) -> Result<v1::BlobAck> {
        let end = chunk
            .offset
            .checked_add(chunk.data.len() as u64)
            .ok_or(NetError::Invalid("blob offset"))?;
        if chunk.asset_id != self.asset.as_str()
            || chunk.total_size != self.size
            || chunk.data.len() > BLOB_CHUNK_BYTES
            || end > self.size
            || chunk.last != (end == self.size)
            || (chunk.data.is_empty() && self.size != 0)
        {
            return Err(NetError::Invalid("blob chunk"));
        }
        if chunk.offset < self.next {
            if end > self.next {
                return Err(NetError::Invalid("overlapping blob chunk"));
            }
            let mut prior = vec![0; chunk.data.len()];
            self.sink.seek(SeekFrom::Start(chunk.offset))?;
            self.sink.read_exact(&mut prior)?;
            self.sink.seek(SeekFrom::Start(self.next))?;
            if prior != chunk.data {
                return Err(NetError::Integrity);
            }
        } else {
            if chunk.offset != self.next {
                return Err(NetError::Invalid("blob gap"));
            }
            self.sink.write_all(&chunk.data)?;
            self.hash.update(&chunk.data);
            self.next = end;
        }
        let verified =
            self.next == self.size && self.hash.finalize().to_hex().as_str() == self.asset.as_str();
        if self.next == self.size && !verified {
            return Err(NetError::Integrity);
        }
        Ok(v1::BlobAck {
            asset_id: self.asset.to_string(),
            next_offset: self.next,
            verified,
        })
    }
    pub fn finish(mut self) -> Result<W> {
        if self.next != self.size
            || self.hash.finalize().to_hex().as_str() != self.asset.as_str()
            || self.sink.seek(SeekFrom::End(0))? != self.size
        {
            return Err(NetError::Integrity);
        }
        self.sink.flush()?;
        Ok(self.sink)
    }
}
pub struct BlobSender<R> {
    source: R,
    asset: AssetId,
    size: u64,
    sent: u64,
    acked: u64,
    empty_sent: bool,
    verified: bool,
}
impl<R: Read + Seek> BlobSender<R> {
    pub fn resume(mut source: R, asset: AssetId, size: u64, offset: u64) -> Result<Self> {
        if offset > size || source.seek(SeekFrom::End(0))? != size {
            return Err(NetError::Invalid("blob source bounds"));
        }
        source.seek(SeekFrom::Start(offset))?;
        Ok(Self {
            source,
            asset,
            size,
            sent: offset,
            acked: offset,
            empty_sent: false,
            verified: false,
        })
    }
    pub fn next_chunk(&mut self) -> Result<Option<v1::BlobChunk>> {
        if (self.sent == self.size && (self.size != 0 || self.empty_sent))
            || self.sent - self.acked >= BLOB_WINDOW_BYTES
        {
            return Ok(None);
        }
        let length = (self.size - self.sent)
            .min(BLOB_CHUNK_BYTES as u64)
            .min(BLOB_WINDOW_BYTES - (self.sent - self.acked)) as usize;
        let mut data = vec![0; length];
        self.source.read_exact(&mut data)?;
        let offset = self.sent;
        self.sent += length as u64;
        self.empty_sent = true;
        Ok(Some(v1::BlobChunk {
            asset_id: self.asset.to_string(),
            offset,
            data,
            total_size: self.size,
            last: self.sent == self.size,
        }))
    }
    pub fn acknowledge(&mut self, ack: &v1::BlobAck) -> Result<()> {
        if ack.asset_id != self.asset.as_str()
            || ack.next_offset > self.sent
            || (ack.verified && ack.next_offset != self.size)
        {
            return Err(NetError::Invalid("blob acknowledgement"));
        }
        // Cumulative acknowledgements can arrive on different QUIC streams.
        // Validate identity/range/verification first, then ignore older progress
        // without rewinding the window or clearing a verified final receipt.
        if ack.next_offset < self.acked {
            return Ok(());
        }
        self.acked = ack.next_offset;
        self.verified |= ack.verified;
        Ok(())
    }
    pub const fn is_verified(&self) -> bool {
        self.verified
    }
}
