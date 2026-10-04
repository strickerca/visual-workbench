//! Private untrusted MCP results. Never edits a project or asserts an AI proof.
mod pixels;
mod storage;
#[cfg(test)]
mod tests;
use super::*;
use crate::{
    WorkflowBinding,
    worker::{Worker, startup},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, atomic::AtomicUsize};
const IMAGE_LIMIT: usize = 4 * 1024 * 1024;
const TEXT_LIMIT: usize = 32 * 1024;
const RECORD_LIMIT: usize = 256 * 1024;
const DISK_LIMIT: u64 = 256 * 1024 * 1024;
static INBOXES: AtomicUsize = AtomicUsize::new(0);
static INPUTS: AtomicUsize = AtomicUsize::new(0);
struct Permit(&'static AtomicUsize);
impl Permit {
    fn acquire(counter: &'static AtomicUsize, limit: usize) -> PackageResult<Self> {
        counter
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < limit).then_some(n + 1)
            })
            .map_err(|_| PackageError::Busy)?;
        Ok(Self(counter))
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct McpInboxSubmission {
    pub receipt_id: String,
    pub package_id: String,
    pub target: String,
    pub manifest_sha256: String,
    pub connection: String,
    pub created_at_ms: i64,
    pub text: Option<String>,
    pub png: Option<Vec<u8>>,
    pub note: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct McpInboxImage {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub encoded_bytes: u64,
    pub blake3: String,
    pub icc_blake3: Option<String>,
    pub orientation_applied: u8,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct McpInboxReceipt {
    pub receipt_id: String,
    pub receipt_blake3: String,
    pub package_id: String,
    pub target: String,
    pub manifest_sha256: String,
    pub binding: WorkflowBinding,
    pub source_asset_id: String,
    pub connection: String,
    pub created_at_ms: i64,
    pub text: Option<String>,
    pub note: String,
    pub before: McpInboxImage,
    pub after: Option<McpInboxImage>,
    pub retired: bool,
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum McpInboxSide {
    Before,
    After,
}
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct McpInboxRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, uniffi::Record)]
pub struct McpInboxPixels {
    pub receipt_id: String,
    pub receipt_blake3: String,
    pub side: McpInboxSide,
    pub region: McpInboxRegion,
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub rgba_srgb: Vec<u8>,
}
#[derive(Debug, uniffi::Record)]
pub struct McpInboxRetirement {
    pub receipt_id: String,
    pub receipt_blake3: String,
    pub files_removed: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    id: String,
    package_id: String,
    target: String,
    manifest_sha256: String,
    project_id: String,
    document_id: String,
    host_seq: u64,
    state_hash: String,
    source_asset_id: String,
    connection: String,
    created_at_ms: i64,
    text: Option<String>,
    note: String,
    before: McpInboxImage,
    after: Option<McpInboxImage>,
}
fn id(v: &str) -> PackageResult<()> {
    if v.len() != 36 {
        return Err(PackageError::Invalid);
    }
    vw_model::Id::try_from(v.to_owned())?;
    Ok(())
}
fn hash(v: &str) -> PackageResult<()> {
    if v.len() != 64
        || !v
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(PackageError::Invalid);
    }
    Ok(())
}
fn text(v: &str) -> PackageResult<()> {
    if v.len() > TEXT_LIMIT || v.contains('\0') {
        return Err(PackageError::Limit);
    }
    Ok(())
}
fn input(v: &McpInboxSubmission) -> PackageResult<()> {
    id(&v.receipt_id)?;
    id(&v.package_id)?;
    hash(&v.manifest_sha256)?;
    if !matches!(
        v.target.as_str(),
        "claude" | "openai" | "gemini" | "generic"
    ) || v.connection.len() != 32
        || !v
            .connection
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        || v.created_at_ms < 0
        || v.text.is_some() == v.png.is_some()
    {
        return Err(PackageError::Invalid);
    }
    text(&v.note)?;
    if let Some(t) = &v.text {
        text(t)?;
        if t.is_empty() {
            return Err(PackageError::Invalid);
        }
    }
    if v.png
        .as_ref()
        .is_some_and(|b| b.is_empty() || b.len() > IMAGE_LIMIT)
    {
        return Err(PackageError::Limit);
    }
    Ok(())
}
impl Record {
    fn validate(&self) -> PackageResult<()> {
        id(&self.id)?;
        id(&self.package_id)?;
        id(&self.project_id)?;
        id(&self.document_id)?;
        hash(&self.manifest_sha256)?;
        hash(&self.state_hash)?;
        vw_model::AssetId::try_from(self.source_asset_id.clone())?;
        if self.schema != 1
            || !matches!(
                self.target.as_str(),
                "claude" | "openai" | "gemini" | "generic"
            )
            || self.connection.len() != 32
            || !self
                .connection
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            || self.created_at_ms < 0
            || self.text.is_some() == self.after.is_some()
        {
            return Err(PackageError::Integrity);
        }
        text(&self.note)?;
        if let Some(t) = &self.text {
            text(t)?;
            if t.is_empty() {
                return Err(PackageError::Integrity);
            }
        }
        for image in std::iter::once(&self.before).chain(self.after.iter()) {
            pixels::image_info(image)?;
        }
        Ok(())
    }
    fn receipt(&self, bytes: &[u8], retired: bool) -> McpInboxReceipt {
        McpInboxReceipt {
            receipt_id: self.id.clone(),
            receipt_blake3: blake3::hash(bytes).to_hex().to_string(),
            package_id: self.package_id.clone(),
            target: self.target.clone(),
            manifest_sha256: self.manifest_sha256.clone(),
            binding: WorkflowBinding {
                project_id: self.project_id.clone(),
                document_id: self.document_id.clone(),
                host_seq: self.host_seq,
                state_hash: self.state_hash.clone(),
            },
            source_asset_id: self.source_asset_id.clone(),
            connection: self.connection.clone(),
            created_at_ms: self.created_at_ms,
            text: self.text.clone(),
            note: self.note.clone(),
            before: self.before.clone(),
            after: self.after.clone(),
            retired,
        }
    }
}
#[derive(uniffi::Object)]
pub struct McpResultInbox {
    worker: Worker<storage::State>,
    closed: Arc<AtomicBool>,
}
#[uniffi::export]
pub async fn open_mcp_result_inbox(
    application_private_directory: String,
    cancellation: Arc<Cancellation>,
) -> PackageResult<Arc<McpResultInbox>> {
    let permit = Permit::acquire(&INBOXES, 1)?;
    startup(move || {
        Ok((|| {
            let state =
                storage::State::open(&application_private_directory, &cancellation, permit)?;
            Ok(Arc::new(McpResultInbox {
                worker: Worker::new("vw-mcp-inbox", state, 2)?,
                closed: Arc::new(AtomicBool::new(false)),
            }))
        })())
    })
    .await?
}
#[uniffi::export]
impl McpResultInbox {
    pub async fn submit(
        &self,
        catalog: Arc<PackageCatalog>,
        submission: McpInboxSubmission,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<McpInboxReceipt> {
        input(&submission)?;
        check(&self.closed, &cancellation)?;
        let permit = Permit::acquire(&INPUTS, 2)?;
        let published = catalog
            .lookup(
                submission.package_id.clone(),
                submission.target.clone(),
                submission.manifest_sha256.clone(),
                cancellation.clone(),
            )
            .await?;
        let mut clean = published
            .info
            .images
            .iter()
            .filter(|i| i.role == "clean_source");
        let first = clean.next().ok_or(PackageError::Integrity)?;
        if clean.next().is_some() || first.encoded_bytes > IMAGE_LIMIT as u64 {
            return Err(PackageError::Limit);
        }
        let before = catalog
            .read_file(
                submission.package_id.clone(),
                submission.target.clone(),
                submission.manifest_sha256.clone(),
                first.path.clone(),
                IMAGE_LIMIT as u32,
                cancellation.clone(),
            )
            .await?;
        let closed = self.closed.clone();
        self.worker
            .call(move |s| {
                let _permit = permit;
                Ok((|| {
                    check(&closed, &cancellation)?;
                    s.submit(submission, published.info, before, &cancellation)
                })())
            })
            .await?
    }
    pub async fn list(
        &self,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<Vec<McpInboxReceipt>> {
        let closed = self.closed.clone();
        self.worker
            .call(move |s| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    s.list(&cancellation)
                })())
            })
            .await?
    }
    pub async fn read_png(
        &self,
        receipt_id: String,
        receipt_blake3: String,
        side: McpInboxSide,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<Vec<u8>> {
        id(&receipt_id)?;
        hash(&receipt_blake3)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |s| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    s.image(&receipt_id, &receipt_blake3, side, &cancellation)
                        .map(|v| v.1)
                })())
            })
            .await?
    }
    /// Display conversion only. Originals remain byte-exact. Untagged sRGB and
    /// 16-to-8-bit display conversion each require their explicit owner choice.
    // Keep the exported ABI explicit: identity, geometry and two independent owner choices.
    #[allow(clippy::too_many_arguments)]
    pub async fn pixels(
        &self,
        receipt_id: String,
        receipt_blake3: String,
        side: McpInboxSide,
        region: McpInboxRegion,
        assume_untagged_srgb: bool,
        allow_depth_reduction: bool,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<McpInboxPixels> {
        id(&receipt_id)?;
        hash(&receipt_blake3)?;
        pixels::region(region)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |s| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    let (record, bytes) =
                        s.image(&receipt_id, &receipt_blake3, side, &cancellation)?;
                    let value = pixels::display(
                        &record,
                        &bytes,
                        side,
                        region,
                        assume_untagged_srgb,
                        allow_depth_reduction,
                        &cancellation,
                    )?;
                    Ok(McpInboxPixels {
                        receipt_id,
                        receipt_blake3,
                        side,
                        region,
                        canvas_width: value.0,
                        canvas_height: value.1,
                        rgba_srgb: value.2,
                    })
                })())
            })
            .await?
    }
    /// Local inbox removal only; callers settle their read/export tasks first.
    pub async fn retire(
        &self,
        receipt_id: String,
        receipt_blake3: String,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<McpInboxRetirement> {
        id(&receipt_id)?;
        hash(&receipt_blake3)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |s| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    s.retire(&receipt_id, &receipt_blake3, &cancellation)
                })())
            })
            .await?
    }
    pub async fn shutdown(&self) -> PackageResult<()> {
        self.closed.store(true, Ordering::Release);
        self.worker.shutdown(|_| Ok(())).await?;
        Ok(())
    }
}
