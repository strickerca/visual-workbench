use crate::{Error, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Write;

pub(crate) const MAX_TEXT: usize = 16 * 1024 * 1024;
pub(crate) struct Buffer {
    pub bytes: Vec<u8>,
    limit: usize,
}
impl Buffer {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
}
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|n| *n <= self.limit)
            .ok_or_else(|| std::io::Error::other("bounded output"))?;
        if length > self.bytes.capacity() {
            let target = length
                .max(self.bytes.capacity().saturating_mul(2).max(4096))
                .min(self.limit);
            self.bytes
                .try_reserve_exact(target - self.bytes.len())
                .map_err(|_| std::io::Error::other("allocation"))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(crate) fn json(value: &impl Serialize, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Buffer::new(limit);
    serde_json::to_writer(&mut bytes, value).map_err(|_| Error::Limit("JSON"))?;
    Ok(bytes.bytes)
}
pub(crate) fn admit(value: &impl Serialize, limit: usize) -> Result<()> {
    struct Count {
        total: usize,
        limit: usize,
    }
    impl Write for Count {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.total = self
                .total
                .checked_add(b.len())
                .filter(|n| *n <= self.limit)
                .ok_or_else(|| std::io::Error::other("bounded projection"))?;
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(&mut Count { total: 0, limit }, value)
        .map_err(|_| Error::Limit("canonical projection"))
}
pub(crate) fn hash(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for b in digest {
        text.push(char::from(HEX[(b >> 4) as usize]));
        text.push(char::from(HEX[(b & 15) as usize]));
    }
    text
}
pub(crate) fn hash_valid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn push(out: &mut String, value: &str) -> Result<()> {
    if out
        .len()
        .checked_add(value.len())
        .is_none_or(|n| n > MAX_TEXT)
    {
        return Err(Error::Limit("prompt"));
    }
    out.push_str(value);
    Ok(())
}
pub(crate) fn quote(value: &str) -> Result<String> {
    if value.len() > MAX_TEXT {
        return Err(Error::Limit("quoted text"));
    }
    let json = String::from_utf8(json(&value, MAX_TEXT)?).map_err(|_| Error::Encoding)?;
    let run = json.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    if json
        .len()
        .checked_add((run + 1) * 2 + 2)
        .is_none_or(|n| n > MAX_TEXT)
    {
        return Err(Error::Limit("quoted text"));
    }
    let fence = "`".repeat(run + 1);
    Ok(format!("{fence} {json} {fence}"))
}
