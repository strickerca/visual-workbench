//! Nonallocating JSON admission before serde can reserve attacker-sized vectors.
//! This is intentionally a strict preflight for this compiler's ASCII field names;
//! serde remains responsible for full JSON syntax, duplicate fields and types.
use crate::{Cancellation, Error, Limits, Result, check};

pub(crate) fn manifest(bytes: &[u8], limits: Limits, cancel: &dyn Cancellation) -> Result<()> {
    scan(bytes, limits.markers, 32 * 1024 * 1024, cancel)
}
pub(crate) fn semantic(bytes: &[u8], cancel: &dyn Cancellation) -> Result<()> {
    scan(bytes, 0, 16 * 1024 * 1024, cancel)
}
fn scan(bytes: &[u8], markers: usize, projection: usize, cancel: &dyn Cancellation) -> Result<()> {
    std::str::from_utf8(bytes).map_err(|_| Error::Integrity)?;
    // Twice the encoded bytes covers retained strings and serde's bounded string
    // scratch/geometric capacity; node charges cover vector slack and structs.
    let used = bytes
        .len()
        .checked_mul(2)
        .ok_or(Error::Limit("JSON projection"))?;
    if used > projection {
        return Err(Error::Limit("JSON projection"));
    }
    let mut scan = Scan {
        bytes,
        at: 0,
        used,
        projection,
        markers,
        cancel,
    };
    scan.value(b"", 0)?;
    scan.space();
    if scan.at != bytes.len() {
        return Err(Error::Integrity);
    }
    Ok(())
}
struct Scan<'a> {
    bytes: &'a [u8],
    at: usize,
    used: usize,
    projection: usize,
    markers: usize,
    cancel: &'a dyn Cancellation,
}
impl Scan<'_> {
    fn space(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.at += 1;
        }
    }
    fn take(&mut self, byte: u8) -> Result<()> {
        self.space();
        if self.bytes.get(self.at) != Some(&byte) {
            return Err(Error::Integrity);
        }
        self.at += 1;
        Ok(())
    }
    fn string(&mut self, key: bool) -> Result<&[u8]> {
        self.take(b'"')?;
        let start = self.at;
        let max = if key {
            64
        } else {
            6 * vw_instructions::MAX_TEXT_BYTES
        };
        loop {
            if self.at - start > max {
                return Err(Error::Limit("JSON string"));
            }
            match self.bytes.get(self.at).copied().ok_or(Error::Integrity)? {
                b'"' => {
                    let end = self.at;
                    self.at += 1;
                    return Ok(&self.bytes[start..end]);
                }
                b'\\' if !key => {
                    self.at += 1;
                    if self.bytes.get(self.at).is_none() {
                        return Err(Error::Integrity);
                    }
                    self.at += 1;
                }
                b'\\' | 0..=31 => return Err(Error::Integrity),
                byte if key && !byte.is_ascii() => return Err(Error::Integrity),
                _ => self.at += 1,
            }
        }
    }
    fn array_limit(&self, key: &[u8]) -> usize {
        match key {
            b"markers" | b"crops" => self.markers,
            b"images" => self.markers + 2,
            b"files" => self.markers + 4,
            b"constraints" | b"redactions" => 64,
            b"instructions" => vw_instructions::MAX_INSTRUCTIONS,
            b"target_ids" => vw_instructions::MAX_TARGETS,
            b"element_refs" => vw_semantics::MAX_REFERENCES,
            b"elements" => vw_semantics::MAX_ELEMENTS,
            // All other arrays emitted by this compiler are 2/4-number geometry.
            _ => 4,
        }
    }
    fn value(&mut self, key: &[u8], depth: usize) -> Result<()> {
        check(self.cancel)?;
        if depth > 32 {
            return Err(Error::Limit("JSON depth"));
        }
        self.used = self
            .used
            .checked_add(128)
            .ok_or(Error::Limit("JSON projection"))?;
        if self.used > self.projection {
            return Err(Error::Limit("JSON projection"));
        }
        self.space();
        match self.bytes.get(self.at).copied().ok_or(Error::Integrity)? {
            b'{' => {
                self.at += 1;
                self.space();
                if self.bytes.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(());
                }
                let mut fields = 0;
                loop {
                    fields += 1;
                    if fields > 64 {
                        return Err(Error::Limit("JSON fields"));
                    }
                    // Copy a bounded key to the stack; never retain/allocate input.
                    let raw = self.string(true)?;
                    let mut field = [0_u8; 64];
                    let len = raw.len();
                    field[..len].copy_from_slice(raw);
                    self.take(b':')?;
                    self.value(&field[..len], depth + 1)?;
                    self.space();
                    if self.bytes.get(self.at) == Some(&b'}') {
                        self.at += 1;
                        break;
                    }
                    self.take(b',')?;
                }
            }
            b'[' => {
                let limit = self.array_limit(key);
                self.at += 1;
                self.space();
                if self.bytes.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(());
                }
                let mut count = 0;
                loop {
                    // Check before scanning the next value, let alone retaining it.
                    if count == limit {
                        return Err(Error::Limit("JSON collection"));
                    }
                    count += 1;
                    self.value(b"", depth + 1)?;
                    self.space();
                    if self.bytes.get(self.at) == Some(&b']') {
                        self.at += 1;
                        break;
                    }
                    self.take(b',')?;
                }
            }
            b'"' => {
                self.string(false)?;
            }
            b'-' | b'0'..=b'9' | b't' | b'f' | b'n' => {
                let start = self.at;
                while self.bytes.get(self.at).is_some_and(|b| {
                    !matches!(b, b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t')
                }) {
                    self.at += 1;
                    if self.at - start > 64 {
                        return Err(Error::Limit("JSON scalar"));
                    }
                }
            }
            _ => return Err(Error::Integrity),
        }
        Ok(())
    }
}
