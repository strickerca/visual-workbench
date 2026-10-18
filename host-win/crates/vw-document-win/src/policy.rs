use crate::{Error, Result};
use std::{collections::BTreeSet, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactEntry {
    pub file_name: String,
    pub bytes: u64,
    pub sha256: [u8; 32],
}
/// Supply the source-bound packaging manifest, never values read from the
/// untrusted document or an unverified manifest adjacent to the executable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeManifest {
    pub entries: Vec<ArtifactEntry>,
}
impl RuntimeManifest {
    pub fn validate(&self) -> Result<()> {
        if !(1..=32).contains(&self.entries.len()) {
            return Err(Error::Limit);
        }
        let mut names = BTreeSet::new();
        let mut total = 0_u64;
        for entry in &self.entries {
            let name = &entry.file_name;
            if name.len() > 96
                || name.is_empty()
                || name.starts_with('.')
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
                || !(name.ends_with(".dll") || name == "vw-document-worker.exe")
                || name.contains("..")
                || !names.insert(name.as_str())
                || entry.bytes == 0
                || entry.bytes > 256 * 1024 * 1024
                || entry.sha256 == [0; 32]
            {
                return Err(Error::Invalid);
            }
            let base = name.split('.').next().ok_or(Error::Invalid)?;
            if matches!(base, "con" | "prn" | "aux" | "nul")
                || (base.len() == 4
                    && (base.starts_with("com") || base.starts_with("lpt"))
                    && base.as_bytes()[3].is_ascii_digit())
            {
                return Err(Error::Invalid);
            }
            total = total.checked_add(entry.bytes).ok_or(Error::Limit)?;
            if total > 512 * 1024 * 1024 {
                return Err(Error::Limit);
            }
        }
        if !names.contains("vw-document-worker.exe") {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerLimits {
    pub wall_time: Duration,
    pub process_memory_bytes: u64,
}
impl WorkerLimits {
    pub fn validate(self, request: &vw_document_protocol::Request) -> Result<()> {
        request.validate()?;
        if !(Duration::from_secs(1)..=Duration::from_secs(120)).contains(&self.wall_time)
            || self.process_memory_bytes != request.limits.memory_bytes
            || !(64 * 1024 * 1024..=2 * 1024 * 1024 * 1024).contains(&self.process_memory_bytes)
        {
            return Err(Error::Limit);
        }
        Ok(())
    }
}
