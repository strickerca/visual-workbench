use crate::{Cancellation, Error, MAX_COMPRESSED_BYTES, MAX_JSON_BYTES, Result, check};
use serde::Serialize;
use std::io::{Cursor, Read, Write};

pub(crate) fn admission(value: &impl Serialize, limit: usize) -> Result<()> {
    let mut counter = Counter { used: 0, limit };
    serde_json::to_writer(&mut counter, value).map_err(|_| Error::Limit("serialized data"))
}
struct Counter {
    used: usize,
    limit: usize,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.used = self
            .used
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("limit"))?;
        if self.used > self.limit {
            return Err(std::io::Error::other("limit"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct Bounded<'a> {
    bytes: Vec<u8>,
    cancel: &'a dyn Cancellation,
}
impl Write for Bounded<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(std::io::Error::other("cancelled"));
        }
        if bytes.len() > MAX_COMPRESSED_BYTES.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("limit"));
        }
        self.bytes
            .try_reserve_exact(bytes.len())
            .map_err(|_| std::io::Error::other("allocation"))?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(crate) fn encode(value: &impl Serialize, cancel: &dyn Cancellation) -> Result<Vec<u8>> {
    check(cancel)?;
    admission(value, MAX_JSON_BYTES)?;
    let sink = Bounded {
        bytes: Vec::new(),
        cancel,
    };
    let mut encoder = zstd::stream::write::Encoder::new(sink, 3).map_err(|_| Error::Codec)?;
    encoder.window_log(20).map_err(|_| Error::Codec)?;
    encoder.include_checksum(true).map_err(|_| Error::Codec)?;
    let written = serde_json::to_writer(&mut encoder, value);
    check(cancel)?;
    written.map_err(|_| Error::Codec)?;
    let result = encoder.finish();
    check(cancel)?;
    Ok(result.map_err(|_| Error::Codec)?.bytes)
}
pub(crate) fn decode(bytes: &[u8], cancel: &dyn Cancellation) -> Result<Vec<u8>> {
    check(cancel)?;
    if bytes.len() > MAX_COMPRESSED_BYTES {
        return Err(Error::Limit("compressed data"));
    }
    // No skippable/legacy frames, dictionaries, trailing frames or hidden suffix.
    if !bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return Err(Error::Codec);
    }
    let mut decoder = zstd::stream::read::Decoder::with_buffer(Cursor::new(bytes))
        .map_err(|_| Error::Codec)?
        .single_frame();
    decoder.window_log_max(20).map_err(|_| Error::Codec)?;
    let mut decoded = Vec::new();
    let mut block = [0u8; 16 * 1024];
    loop {
        check(cancel)?;
        let count = decoder.read(&mut block).map_err(|_| Error::Codec)?;
        if count == 0 {
            break;
        }
        if count > MAX_JSON_BYTES.saturating_sub(decoded.len()) {
            return Err(Error::Limit("decompressed data"));
        }
        decoded
            .try_reserve_exact(count)
            .map_err(|_| Error::Limit("allocation"))?;
        decoded.extend_from_slice(&block[..count]);
    }
    if decoder.finish().position() != bytes.len() as u64 {
        return Err(Error::Codec);
    }
    check(cancel)?;
    Ok(decoded)
}
