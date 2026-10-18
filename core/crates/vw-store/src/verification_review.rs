//! Private manual reviews over the existing settings/transaction owner.
//!
//! No notes, settings keys or selectors enter package lineage. Callers derive
//! the pair from checked catalog/receipt owners and recheck their retirement
//! under the existing caller mutex; this store is not a capture authority.
//! Portable archive scrubbing/refusal must cover this raw-byte namespace,
//! including malformed BLOB keys; see the archive portability finding in
//! vw-ffi/src/verification/INTEGRATION.md. GLOB's BLOB matching is build-dependent.
use crate::{ProjectStore, StoreError, migrations};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{
    Deserialize, Serialize,
    de::{SeqAccess, Visitor},
};
use vw_model::Id;

pub const VERIFICATION_REVIEW_PREFIX: &str = "vw.verification-review.v1.";
pub const VERIFICATION_REVIEW_MAX_MARKERS: usize = 512;
pub const VERIFICATION_REVIEW_MAX_ROWS: usize = 64;
pub const VERIFICATION_REVIEW_MAX_BYTES: usize = 1024 * 1024;
pub const VERIFICATION_REVIEW_HISTORY_MAX_ROWS: u32 = 8;
pub const VERIFICATION_REVIEW_HISTORY_MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_NOTE: usize = 32 * 1024;
const MAX_NOTES: usize = 128 * 1024;
// Read only bounded key previews and scalar admission metadata. Byte-prefix
// matching also catches malformed BLOB keys regardless of GLOB compile options.
const HISTORY_ADMISSION_SQL: &str = "SELECT typeof(key),length(CAST(key AS BLOB)),typeof(value),length(CAST(value AS BLOB)),substr(CAST(key AS BLOB),1,?3) FROM settings WHERE substr(CAST(key AS BLOB),1,?1)=CAST(?2 AS BLOB) LIMIT ?4";

#[derive(Debug, thiserror::Error)]
pub enum VerificationReviewError {
    #[error("invalid verification review")]
    Invalid,
    #[error("verification review capacity exceeded")]
    Limit,
    #[error("verification review changed")]
    Conflict,
    #[error("verification review awaits owner decisions")]
    Pending,
    #[error("verification review cancelled before commit")]
    Cancelled,
    #[error("verification review data is corrupt")]
    Corrupt,
    #[error("verification review storage failed")]
    Storage(#[from] StoreError),
}
impl From<rusqlite::Error> for VerificationReviewError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Storage(StoreError::Database(value))
    }
}
pub type VerificationReviewResult<T> = Result<T, VerificationReviewError>;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationPackageKey {
    pub package_id: Id,
    pub target: String,
    pub manifest_sha256: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReviewPair {
    pub project_id: Id,
    pub original: VerificationPackageKey,
    pub after: VerificationPackageKey,
    pub comparison_receipt_id: Id,
    pub comparison_receipt_blake3: String,
    pub marker_count: u32,
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn target(value: &str) -> bool {
    matches!(value, "claude" | "openai" | "gemini" | "generic")
}
impl VerificationReviewPair {
    pub fn validate(&self) -> VerificationReviewResult<()> {
        if self.marker_count > VERIFICATION_REVIEW_MAX_MARKERS as u32
            || self.original.package_id == self.after.package_id
            || !target(&self.original.target)
            || !target(&self.after.target)
            || !hash(&self.original.manifest_sha256)
            || !hash(&self.after.manifest_sha256)
            || !hash(&self.comparison_receipt_blake3)
        {
            return Err(VerificationReviewError::Invalid);
        }
        Ok(())
    }
    fn key(&self) -> VerificationReviewResult<String> {
        self.validate()?;
        let encoded = serde_json::to_vec(self).map_err(|_| VerificationReviewError::Invalid)?;
        Ok(format!(
            "{VERIFICATION_REVIEW_PREFIX}{}",
            blake3::hash(&encoded).to_hex()
        ))
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationVerdict {
    Pending,
    Pass,
    Fail,
}
// No Debug: owner notes never become diagnostic/evidence strings by accident.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationMarkerReview {
    pub original_number: u32,
    pub verdict: VerificationVerdict,
    pub note: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReviewSnapshot {
    pub pair: VerificationReviewPair,
    pub version: u64,
    #[serde(deserialize_with = "bounded_markers")]
    pub markers: Vec<VerificationMarkerReview>,
}
/// Private installation-local history only. The cursor is exactly the settings
/// key: VERIFICATION_REVIEW_PREFIX followed by 64 lowercase ASCII hex digits.
/// It is an exclusive binary lexicographic boundary, never a capture selector.
pub struct VerificationReviewHistoryPage {
    pub snapshots: Vec<VerificationReviewSnapshot>,
    pub next_cursor: Option<String>,
}
fn bounded_markers<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Vec<VerificationMarkerReview>, D::Error> {
    struct Markers;
    impl<'de> Visitor<'de> for Markers {
        type Value = Vec<VerificationMarkerReview>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most 512 marker decisions")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(marker) = seq.next_element()? {
                if out.len() == VERIFICATION_REVIEW_MAX_MARKERS {
                    return Err(serde::de::Error::custom("marker limit"));
                }
                out.push(marker);
            }
            Ok(out)
        }
    }
    decoder.deserialize_seq(Markers)
}
impl VerificationReviewSnapshot {
    fn empty(pair: &VerificationReviewPair) -> Self {
        Self {
            pair: pair.clone(),
            version: 0,
            markers: (1..=pair.marker_count)
                .map(|n| VerificationMarkerReview {
                    original_number: n,
                    verdict: VerificationVerdict::Pending,
                    note: String::new(),
                })
                .collect(),
        }
    }
    pub fn validate(&self) -> VerificationReviewResult<()> {
        self.pair.validate()?;
        if self.version > i64::MAX as u64 || self.markers.len() != self.pair.marker_count as usize {
            return Err(VerificationReviewError::Invalid);
        }
        let mut notes = 0;
        for (index, marker) in self.markers.iter().enumerate() {
            if marker.original_number != index as u32 + 1
                || marker.note.len() > MAX_NOTE
                || marker.note.contains('\0')
            {
                return Err(VerificationReviewError::Invalid);
            }
            notes += marker.note.len();
        }
        if notes > MAX_NOTES {
            return Err(VerificationReviewError::Limit);
        }
        if self.version == 0
            && self
                .markers
                .iter()
                .any(|m| m.verdict != VerificationVerdict::Pending || !m.note.is_empty())
        {
            return Err(VerificationReviewError::Invalid);
        }
        Ok(())
    }
    pub fn failed_markers(&self) -> VerificationReviewResult<Option<Vec<u32>>> {
        self.validate()?;
        if self
            .markers
            .iter()
            .any(|m| m.verdict == VerificationVerdict::Pending)
        {
            return Err(VerificationReviewError::Pending);
        }
        let failed: Vec<_> = self
            .markers
            .iter()
            .filter(|m| m.verdict == VerificationVerdict::Fail)
            .map(|m| m.original_number)
            .collect();
        Ok((!failed.is_empty()).then_some(failed))
    }
}
pub struct VerificationReviewChange {
    pub original_number: u32,
    pub verdict: VerificationVerdict,
    pub note: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReview {
    schema: u32,
    snapshot: VerificationReviewSnapshot,
    operation: Id,
    digest: String,
}
// Inspect the identity envelope before deserializing any marker/note vector.
// IgnoredAny traverses bounded encoded markers without retaining their strings.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReviewAdmission {
    schema: u32,
    snapshot: SnapshotAdmission,
    #[serde(rename = "operation")]
    _operation: Id,
    digest: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotAdmission {
    pair: VerificationReviewPair,
    version: u64,
    #[serde(rename = "markers")]
    _markers: serde::de::IgnoredAny,
}

fn admit_schema(connection: &Connection) -> VerificationReviewResult<()> {
    let (count, bytes): (i64, i64) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(length(CAST(type AS BLOB))+length(CAST(name AS BLOB))+length(CAST(tbl_name AS BLOB))+length(CAST(COALESCE(sql,'') AS BLOB))),0) FROM (SELECT type,name,tbl_name,sql FROM sqlite_master LIMIT 257)",
        [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    if count > 256 || !(0..=65_536).contains(&bytes) {
        return Err(VerificationReviewError::Limit);
    }
    migrations::verify_schema(connection, 2)?;
    Ok(())
}
fn load(
    connection: &Connection,
    pair: Option<&VerificationReviewPair>,
    project: &Id,
    key: &str,
) -> VerificationReviewResult<Option<(StoredReview, String)>> {
    if !history_key(key) {
        return Err(VerificationReviewError::Corrupt);
    }
    let admission: Option<(String, i64)> = connection
        .query_row(
            "SELECT typeof(value),length(CAST(value AS BLOB)) FROM settings WHERE key=?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((kind, size)) = admission else {
        return Ok(None);
    };
    if kind != "text" || !(1..=VERIFICATION_REVIEW_MAX_BYTES as i64).contains(&size) {
        return Err(VerificationReviewError::Corrupt);
    }
    let bytes: Vec<u8> = connection.query_row(
        "SELECT CAST(value AS BLOB) FROM settings WHERE key=?1",
        [key],
        |r| r.get(0),
    )?;
    if bytes.len() != size as usize || bytes.len() > VERIFICATION_REVIEW_MAX_BYTES {
        return Err(VerificationReviewError::Corrupt);
    }
    let raw = String::from_utf8(bytes).map_err(|_| VerificationReviewError::Corrupt)?;
    let admission: StoredReviewAdmission =
        serde_json::from_str(&raw).map_err(|_| VerificationReviewError::Corrupt)?;
    if admission.schema != 1
        || !(1..=i64::MAX as u64).contains(&admission.snapshot.version)
        || pair.is_some_and(|expected| admission.snapshot.pair != *expected)
        || &admission.snapshot.pair.project_id != project
        || admission
            .snapshot
            .pair
            .key()
            .map_err(|_| VerificationReviewError::Corrupt)?
            != key
        || !hash(&admission.digest)
    {
        return Err(VerificationReviewError::Corrupt);
    }
    drop(admission);
    let value: StoredReview =
        serde_json::from_str(&raw).map_err(|_| VerificationReviewError::Corrupt)?;
    value
        .snapshot
        .validate()
        .map_err(|_| VerificationReviewError::Corrupt)?;
    if value.schema != 1
        || value.snapshot.version == 0
        || pair.is_some_and(|expected| value.snapshot.pair != *expected)
        || &value.snapshot.pair.project_id != project
        || value
            .snapshot
            .pair
            .key()
            .map_err(|_| VerificationReviewError::Corrupt)?
            != key
        || !hash(&value.digest)
    {
        return Err(VerificationReviewError::Corrupt);
    }
    Ok(Some((value, raw)))
}
fn history_key(key: &str) -> bool {
    key.strip_prefix(VERIFICATION_REVIEW_PREFIX)
        .is_some_and(hash)
}
struct HistoryRow {
    key: String,
    bytes: usize,
}
fn history_rows(connection: &Connection) -> VerificationReviewResult<Vec<HistoryRow>> {
    let mut statement = connection.prepare(HISTORY_ADMISSION_SQL)?;
    let mut query = statement.query(params![
        VERIFICATION_REVIEW_PREFIX.len() as i64,
        VERIFICATION_REVIEW_PREFIX,
        (VERIFICATION_REVIEW_PREFIX.len() + 64) as i64,
        (VERIFICATION_REVIEW_MAX_ROWS + 1) as i64,
    ])?;
    let mut rows = Vec::new();
    while let Some(row) = query.next()? {
        if rows.len() == VERIFICATION_REVIEW_MAX_ROWS {
            return Err(VerificationReviewError::Limit);
        }
        let key_type: String = row.get(0)?;
        let key_bytes: i64 = row.get(1)?;
        let value_type: String = row.get(2)?;
        let value_bytes: i64 = row.get(3)?;
        if key_type != "text"
            || key_bytes != (VERIFICATION_REVIEW_PREFIX.len() + 64) as i64
            || value_type != "text"
            || !(1..=VERIFICATION_REVIEW_MAX_BYTES as i64).contains(&value_bytes)
        {
            return Err(VerificationReviewError::Corrupt);
        }
        let preview: Vec<u8> = row.get(4)?;
        let key = String::from_utf8(preview).map_err(|_| VerificationReviewError::Corrupt)?;
        if !history_key(&key) {
            return Err(VerificationReviewError::Corrupt);
        }
        rows.push(HistoryRow {
            key,
            bytes: value_bytes as usize,
        });
    }
    rows.sort_unstable_by(|a, b| a.key.cmp(&b.key));
    Ok(rows)
}
impl ProjectStore {
    /// Enumerate existing durable reviews without creating version-zero rows.
    /// Persisted exact pairs are history authority, never restored OS capture
    /// authority. The caller revalidates selected packages/source/grants before
    /// any later mutation or compilation, through the existing joined owners.
    pub fn verification_review_history(
        &mut self,
        after: Option<String>,
        limit: u32,
    ) -> VerificationReviewResult<VerificationReviewHistoryPage> {
        if !(1..=VERIFICATION_REVIEW_HISTORY_MAX_ROWS).contains(&limit)
            || after.as_deref().is_some_and(|cursor| !history_key(cursor))
        {
            return Err(VerificationReviewError::Invalid);
        }
        let project = self.project().id.clone();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        admit_schema(&tx)?;
        // Validate the complete bounded namespace metadata before any raw JSON
        // read, including malformed rows before/at the supplied cursor.
        let rows = history_rows(&tx)?;
        let mut candidates = rows
            .into_iter()
            .filter(|row| after.as_ref().is_none_or(|cursor| row.key > *cursor))
            .peekable();
        let mut snapshots = Vec::new();
        let mut admitted = 0_usize;
        let mut last_returned = None;
        while snapshots.len() < limit as usize {
            let Some(row) = candidates.peek() else {
                break;
            };
            let next_bytes = admitted
                .checked_add(row.bytes)
                .ok_or(VerificationReviewError::Limit)?;
            if next_bytes > VERIFICATION_REVIEW_HISTORY_MAX_BYTES {
                // Each individual row fits <=1 MiB, so at least one row always
                // fits an empty page. Defer this row without reading its JSON;
                // returning the last included key makes the next page advance.
                break;
            }
            let row = candidates.next().ok_or(VerificationReviewError::Corrupt)?;
            let (record, raw) =
                load(&tx, None, &project, &row.key)?.ok_or(VerificationReviewError::Corrupt)?;
            if raw.len() != row.bytes {
                return Err(VerificationReviewError::Corrupt);
            }
            admitted = next_bytes;
            snapshots.push(record.snapshot);
            last_returned = Some(row.key);
        }
        let next_cursor = if candidates.peek().is_some() {
            last_returned
        } else {
            None
        };
        tx.commit()?;
        Ok(VerificationReviewHistoryPage {
            snapshots,
            next_cursor,
        })
    }
    pub fn verification_review(
        &mut self,
        pair: &VerificationReviewPair,
    ) -> VerificationReviewResult<VerificationReviewSnapshot> {
        pair.validate()?;
        if pair.project_id != self.project().id {
            return Err(VerificationReviewError::Conflict);
        }
        let key = pair.key()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        admit_schema(&tx)?;
        let snapshot = load(&tx, Some(pair), &pair.project_id, &key)?.map_or_else(
            || VerificationReviewSnapshot::empty(pair),
            |(value, _)| value.snapshot,
        );
        tx.commit()?;
        Ok(snapshot)
    }
    /// Manual owner action only. The current pair/catalog retirement fence is a
    /// caller responsibility, in the same existing serial owner as publication.
    /// Exact lost-reply retries recover the latest durable operation; a reused
    /// operation with changed precondition/content refuses. No post-commit cancel.
    pub fn change_verification_review(
        &mut self,
        pair: &VerificationReviewPair,
        expected_version: u64,
        operation: &Id,
        change: VerificationReviewChange,
        cancelled: impl Fn() -> bool,
    ) -> VerificationReviewResult<VerificationReviewSnapshot> {
        pair.validate()?;
        if pair.project_id != self.project().id || expected_version > i64::MAX as u64 {
            return Err(VerificationReviewError::Conflict);
        }
        if !(1..=pair.marker_count).contains(&change.original_number)
            || change.note.len() > MAX_NOTE
            || change.note.contains('\0')
        {
            return Err(VerificationReviewError::Invalid);
        }
        if cancelled() {
            return Err(VerificationReviewError::Cancelled);
        }
        let request = serde_json::to_vec(&(
            expected_version,
            change.original_number,
            change.verdict,
            &change.note,
        ))
        .map_err(|_| VerificationReviewError::Invalid)?;
        let digest = blake3::hash(&request).to_hex().to_string();
        let key = pair.key()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        admit_schema(&tx)?;
        let loaded = load(&tx, Some(pair), &pair.project_id, &key)?;
        if let Some((value, _)) = &loaded
            && value.operation == *operation
        {
            if value.digest != digest {
                return Err(VerificationReviewError::Conflict);
            }
            let snapshot = value.snapshot.clone();
            tx.commit()?;
            return Ok(snapshot);
        }
        let mut snapshot = loaded.as_ref().map_or_else(
            || VerificationReviewSnapshot::empty(pair),
            |(value, _)| value.snapshot.clone(),
        );
        if snapshot.version != expected_version {
            return Err(VerificationReviewError::Conflict);
        }
        if loaded.is_none() {
            let count: i64 = tx.query_row("SELECT COUNT(*) FROM (SELECT 1 FROM settings WHERE key GLOB 'vw.verification-review.v1.*' LIMIT 65)", [], |r| r.get(0))?;
            if count >= VERIFICATION_REVIEW_MAX_ROWS as i64 {
                return Err(VerificationReviewError::Limit);
            }
        }
        let marker = snapshot
            .markers
            .get_mut(change.original_number as usize - 1)
            .ok_or(VerificationReviewError::Invalid)?;
        marker.verdict = change.verdict;
        marker.note = change.note;
        snapshot.version = snapshot
            .version
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(VerificationReviewError::Limit)?;
        snapshot.validate()?;
        let value = StoredReview {
            schema: 1,
            snapshot,
            operation: operation.clone(),
            digest,
        };
        let encoded =
            serde_json::to_string(&value).map_err(|_| VerificationReviewError::Invalid)?;
        if encoded.len() > VERIFICATION_REVIEW_MAX_BYTES {
            return Err(VerificationReviewError::Limit);
        }
        if cancelled() {
            return Err(VerificationReviewError::Cancelled);
        }
        let changed = if let Some((_, raw)) = loaded {
            tx.execute(
                "UPDATE settings SET value=?1 WHERE key=?2 AND value=?3",
                params![encoded, key, raw],
            )?
        } else {
            tx.execute(
                "INSERT INTO settings(key,value) VALUES (?1,?2)",
                params![key, encoded],
            )?
        };
        if changed != 1 {
            return Err(VerificationReviewError::Conflict);
        }
        let (persisted, raw) = load(&tx, Some(pair), &pair.project_id, &key)?
            .ok_or(VerificationReviewError::Corrupt)?;
        if raw != encoded {
            return Err(VerificationReviewError::Corrupt);
        }
        if cancelled() {
            return Err(VerificationReviewError::Cancelled);
        }
        tx.commit()?;
        Ok(persisted.snapshot)
    }
    /// Recheck this immediately before preparing/publishing a failure package.
    /// This cannot replace the caller's checked catalog/receipt retirement fence.
    pub fn check_verification_review(
        &mut self,
        expected: &VerificationReviewSnapshot,
    ) -> VerificationReviewResult<()> {
        expected.validate()?;
        if self.verification_review(&expected.pair)? != *expected {
            return Err(VerificationReviewError::Conflict);
        }
        Ok(())
    }
}
