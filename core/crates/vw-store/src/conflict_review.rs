//! Local review drafts over the existing immutable conflict journal and outbox.
//! No patch or authentication authority is added to the wire protocol.
use crate::{ProjectStore, StoreError, database::decode};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use vw_model::{AssetId, Id};
use vw_proto::{Message, v1 as pb};

const PREFIX: &str = "offline-review/v1/";
const MAX_REVIEW_BYTES: usize = 16 * 1024 * 1024;

/// A restore makes a new, ungrouped copy. The original tombstone and conflict
/// remain intact. The destination must match the retained object's layer.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ConflictReviewChoice {
    SwapOther,
    RestoreCopy { document: Id, object: Id },
}

/// Caller-supplied fresh identity, allocated by the existing editor counter owner.
/// Clock values never select a conflict value here; this is a new explicit edit.
pub struct ConflictReviewIdentity {
    pub transaction: Id,
    pub lamport: u64,
    pub wall_ms: i64,
}

/// This is device-local review progress, not a new replicated resolution log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictReviewStatus {
    Unreviewed,
    Kept,
    /// Existing metadata without this implementation's exact draft binding.
    PreviouslyResolved,
    Draft,
    Queued,
    Applied,
}

/// Private payloads deliberately have no Debug implementation. Display only in
/// an explicit project review surface, never diagnostics or public evidence.
pub struct ConflictReview {
    pub record: pb::ConflictRecord,
    pub revision: pb::Revision,
    pub status: ConflictReviewStatus,
    pub transaction: Option<pb::Transaction>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    version: u8,
    record_hash: String,
    choice: ConflictReviewChoice,
    transaction: pb::Transaction,
}

impl ProjectStore {
    /// Stable, bounded journal cursor. Fetch payloads individually with
    /// conflict_review; pages never clone all retained losing values.
    pub fn conflict_review_ids(
        &self,
        after_row: i64,
        limit: u32,
    ) -> Result<Vec<(i64, Id)>, StoreError> {
        if after_row < 0 || !(1..=128).contains(&limit) {
            return Err(StoreError::Invalid("review cursor"));
        }
        let mut query = self.connection.prepare(
            "SELECT rowid,conflict_id FROM conflicts WHERE rowid>?1 ORDER BY rowid LIMIT ?2",
        )?;
        let mut rows = query.query(params![after_row, limit])?;
        let mut result = Vec::new();
        while let Some(row) = rows.next()? {
            result.push((row.get(0)?, Id::try_from(row.get::<_, String>(1)?)?));
        }
        Ok(result)
    }

    /// Small existing projection fields for a bounded review list. No retained
    /// value or draft body is loaded merely to label an entry.
    pub fn conflict_review_label(&self, conflict: &Id) -> Result<(String, bool), StoreError> {
        let length: i64 = self.connection.query_row(
            "SELECT length(CAST(property AS BLOB)) FROM conflicts WHERE conflict_id=?1",
            [conflict.as_str()],
            |r| r.get(0),
        )?;
        if !(1..=1024).contains(&length) {
            return Err(StoreError::Corrupt("review property size"));
        }
        Ok(self.connection.query_row(
            "SELECT property,delete_vs_edit FROM conflicts WHERE conflict_id=?1",
            [conflict.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    pub fn conflict_review(&self, conflict: &Id) -> Result<ConflictReview, StoreError> {
        let (record, resolution) = self.review_record(conflict)?;
        let draft = self.review_draft(conflict)?;
        let status = if let Some(draft) = &draft {
            self.validate_review_binding(&record, draft)?;
            self.review_delivery(&draft.transaction)?
        } else {
            match resolution.as_deref() {
                None => ConflictReviewStatus::Unreviewed,
                Some("kept") => ConflictReviewStatus::Kept,
                Some("swapped" | "restored") => ConflictReviewStatus::PreviouslyResolved,
                _ => return Err(StoreError::Corrupt("review resolution")),
            }
        };
        Ok(ConflictReview {
            record,
            revision: self.revision()?,
            status,
            transaction: draft.map(|d| d.transaction),
        })
    }

    /// Save an exact immutable review draft before any commit or enqueue. An
    /// interruption leaves it available for explicit resume or cancellation.
    /// Existing drafts cannot be replaced, even by a value-equal fresh txn ID.
    pub fn stage_conflict_review(
        &mut self,
        conflict: &Id,
        expected: &pb::Revision,
        choice: ConflictReviewChoice,
        identity: ConflictReviewIdentity,
    ) -> Result<ConflictReview, StoreError> {
        self.review_fence(expected)?;
        let (record, resolution) = self.review_record(conflict)?;
        if resolution.is_some() || self.review_draft(conflict)?.is_some() {
            return Err(StoreError::Invalid("review already started"));
        }
        if identity.wall_ms < 0
            || identity.lamport < self.review_identity_floor(Some(&identity.transaction))?
        {
            return Err(StoreError::Invalid("review identity"));
        }
        let author = self.local_device()?;
        let kind = review_operation(
            self,
            &record,
            &choice,
            &author.to_string(),
            identity.wall_ms,
        )?;
        let transaction = pb::Transaction {
            txn_id: Some(identity.transaction.to_proto()),
            project_id: Some(self.project().id.to_proto()),
            device_id: author.to_string(),
            base_revision: Some(expected.clone()),
            created_at_wall_ms: identity.wall_ms,
            ops: vec![pb::Op {
                op_id: Some(pb::OpId {
                    device_id: author.to_string(),
                    lamport: identity.lamport,
                }),
                kind: Some(kind),
            }],
            ..Default::default()
        };
        let acceptance = self.validate_transaction(&transaction, &author, identity.wall_ms)?;
        if acceptance.duplicate || !acceptance.conflicts.is_empty() {
            return Err(StoreError::Invalid("review identity reuse"));
        }
        let draft = Draft {
            version: 1,
            record_hash: record_hash(&record),
            choice,
            transaction,
        };
        let encoded = serde_json::to_string(&draft)?;
        if encoded.len() > MAX_REVIEW_BYTES {
            return Err(StoreError::Invalid("review draft size"));
        }
        self.connection.execute(
            "INSERT INTO settings(key,value) VALUES (?1,?2)",
            params![review_key(conflict), encoded],
        )?;
        self.conflict_review(conflict)
    }

    /// Resume only this saved transaction. Exact accepted/queued retries succeed
    /// after reconnect even if authority advanced; a never-submitted stale draft
    /// is refused and preserved for a fresh explicit review.
    ///
    /// Call on the existing single project worker. On a mirror, rebuild its
    /// optimistic Replica with queue_recovered before publishing the new view.
    pub fn submit_conflict_review(
        &mut self,
        conflict: &Id,
        now_ms: i64,
    ) -> Result<ConflictReviewStatus, StoreError> {
        if now_ms < 0 {
            return Err(StoreError::Invalid("review time"));
        }
        let transaction = self.conflict_review_submission(conflict, now_ms)?;
        let draft = self
            .review_draft(conflict)?
            .ok_or(StoreError::Invalid("review not staged"))?;
        match self.review_delivery(&transaction)? {
            ConflictReviewStatus::Applied => {
                self.finish_review(conflict, &draft.choice, now_ms)?;
                return Ok(ConflictReviewStatus::Applied);
            }
            ConflictReviewStatus::Queued => return Ok(ConflictReviewStatus::Queued),
            _ => {}
        }
        let local = self.local_device()?;
        if &local == self.host_device() {
            self.commit(&transaction, &local, now_ms)?;
            // If this metadata write fails, the journal still proves Applied.
            // The next exact retry settles metadata without another acceptance.
            self.finish_review(conflict, &draft.choice, now_ms)?;
            Ok(ConflictReviewStatus::Applied)
        } else {
            self.enqueue_pending(&transaction)?;
            Ok(ConflictReviewStatus::Queued)
        }
    }

    /// Validate a saved submission without crossing its durable commit point.
    /// The existing application ProjectState::commit may then stage optimistic
    /// replica/history owners before persistence, on the same single worker.
    /// After commit call submit_conflict_review to settle local metadata only.
    pub fn conflict_review_submission(
        &self,
        conflict: &Id,
        now_ms: i64,
    ) -> Result<pb::Transaction, StoreError> {
        if now_ms < 0 {
            return Err(StoreError::Invalid("review time"));
        }
        let (record, _) = self.review_record(conflict)?;
        let draft = self
            .review_draft(conflict)?
            .ok_or(StoreError::Invalid("review not staged"))?;
        self.validate_review_binding(&record, &draft)?;
        if matches!(
            self.review_delivery(&draft.transaction)?,
            ConflictReviewStatus::Applied | ConflictReviewStatus::Queued
        ) {
            return Ok(draft.transaction);
        }
        self.review_fence(
            draft
                .transaction
                .base_revision
                .as_ref()
                .ok_or(StoreError::Corrupt("review base"))?,
        )?;
        let local = self.local_device()?;
        let intended = review_operation(
            self,
            &record,
            &draft.choice,
            local.as_str(),
            draft.transaction.created_at_wall_ms,
        )?;
        if draft.transaction.ops[0].kind.as_ref() != Some(&intended) {
            return Err(StoreError::Corrupt("review operation binding"));
        }
        self.validate_transaction(&draft.transaction, &local, now_ms)?;
        Ok(draft.transaction)
    }

    /// Explicitly dismiss a conflict at the exact reviewed state. This changes
    /// only local review metadata; the losing value remains in the journal.
    pub fn keep_conflict_current(
        &mut self,
        conflict: &Id,
        expected: &pb::Revision,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        self.review_fence(expected)?;
        if now_ms < 0 || self.review_draft(conflict)?.is_some() {
            return Err(StoreError::Invalid("review keep state"));
        }
        let (_, resolution) = self.review_record(conflict)?;
        if resolution.as_deref() == Some("kept") {
            return Ok(());
        }
        if resolution.is_some() {
            return Err(StoreError::Invalid("review already resolved"));
        }
        self.connection.execute(
            "UPDATE conflicts SET resolution='kept',resolved_at=?1 WHERE conflict_id=?2",
            params![now_ms, conflict.as_str()],
        )?;
        Ok(())
    }

    /// Explicit cancellation is allowed only before submission. Queued or
    /// accepted edits use the existing inverse-history workflow, never deletion.
    pub fn cancel_conflict_review(&mut self, conflict: &Id) -> Result<(), StoreError> {
        let review = self.conflict_review(conflict)?;
        if review.status != ConflictReviewStatus::Draft {
            return Err(StoreError::Invalid("review already submitted"));
        }
        self.connection
            .execute("DELETE FROM settings WHERE key=?1", [review_key(conflict)])?;
        Ok(())
    }

    /// Merge this floor into the existing editor counter on open and after
    /// staging. Saved review intents reserve counters before they enter outbox
    /// or accepted history; they must not be registered as undo entries yet.
    pub fn conflict_review_lamport_floor(&self) -> Result<u64, StoreError> {
        self.review_identity_floor(None)
    }

    fn review_identity_floor(&self, candidate: Option<&Id>) -> Result<u64, StoreError> {
        let mut floor = 1;
        let local = self.local_device()?;
        let mut query = self.connection.prepare(
            "SELECT key,length(CAST(key AS BLOB)) FROM settings WHERE key GLOB 'offline-review/v1/*'",
        )?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            if row.get::<_, usize>(1)? != PREFIX.len() + 36 {
                return Err(StoreError::Corrupt("review key"));
            }
            let key: String = row.get(0)?;
            let conflict = Id::try_from(key[PREFIX.len()..].to_owned())?;
            let draft = self
                .review_draft(&conflict)?
                .ok_or(StoreError::Corrupt("review draft"))?;
            if draft.transaction.device_id != local.as_str() || draft.transaction.ops.len() != 1 {
                return Err(StoreError::Corrupt("review identity"));
            }
            let id = Id::from_proto(draft.transaction.txn_id.as_ref())?;
            if candidate == Some(&id) {
                return Err(StoreError::Invalid("review identity reuse"));
            }
            let op = draft.transaction.ops[0]
                .op_id
                .as_ref()
                .ok_or(StoreError::Corrupt("review counter"))?;
            if op.device_id != local.as_str() || op.lamport == 0 {
                return Err(StoreError::Corrupt("review counter"));
            }
            floor = floor.max(
                op.lamport
                    .checked_add(1)
                    .ok_or(StoreError::Invalid("review counter exhausted"))?,
            );
        }
        Ok(floor)
    }

    fn review_fence(&self, expected: &pb::Revision) -> Result<(), StoreError> {
        if self.revision()? != *expected {
            return Err(StoreError::Invalid("review changed or pending edits"));
        }
        let pending: bool =
            self.connection
                .query_row("SELECT EXISTS(SELECT 1 FROM pending_txns)", [], |r| {
                    r.get(0)
                })?;
        if pending {
            return Err(StoreError::Invalid("review changed or pending edits"));
        }
        Ok(())
    }

    fn review_record(
        &self,
        conflict: &Id,
    ) -> Result<(pb::ConflictRecord, Option<String>), StoreError> {
        let length: i64 = self.connection.query_row(
            "SELECT length(record) FROM conflicts WHERE conflict_id=?1",
            [conflict.as_str()],
            |r| r.get(0),
        )?;
        if !(1..=MAX_REVIEW_BYTES as i64).contains(&length) {
            return Err(StoreError::Corrupt("review record size"));
        }
        let (bytes, resolution): (Vec<u8>, Option<String>) = self.connection.query_row(
            "SELECT record,resolution FROM conflicts WHERE conflict_id=?1",
            [conflict.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let record: pb::ConflictRecord = decode(&bytes)?;
        if Id::from_proto(record.conflict_id.as_ref())? != *conflict {
            return Err(StoreError::Corrupt("review conflict identity"));
        }
        Ok((record, resolution))
    }

    fn review_draft(&self, conflict: &Id) -> Result<Option<Draft>, StoreError> {
        let key = review_key(conflict);
        let length: Option<i64> = self
            .connection
            .query_row(
                "SELECT length(CAST(value AS BLOB)) FROM settings WHERE key=?1",
                [&key],
                |r| r.get(0),
            )
            .optional()?;
        let Some(length) = length else {
            return Ok(None);
        };
        if !(1..=MAX_REVIEW_BYTES as i64).contains(&length) {
            return Err(StoreError::Corrupt("review draft size"));
        }
        let encoded: String =
            self.connection
                .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                    r.get(0)
                })?;
        Ok(Some(serde_json::from_str(&encoded)?))
    }

    fn validate_review_binding(
        &self,
        record: &pb::ConflictRecord,
        draft: &Draft,
    ) -> Result<(), StoreError> {
        if draft.version != 1
            || draft.record_hash != record_hash(record)
            || draft.transaction.device_id != self.local_device()?.as_str()
            || Id::from_proto(draft.transaction.project_id.as_ref())? != self.project().id
            || draft.transaction.ops.len() != 1
        {
            return Err(StoreError::Corrupt("review draft binding"));
        }
        Ok(())
    }

    fn review_delivery(
        &self,
        transaction: &pb::Transaction,
    ) -> Result<ConflictReviewStatus, StoreError> {
        let id = Id::from_proto(transaction.txn_id.as_ref())?;
        let bytes = transaction.encode_to_vec();
        // Byte comparison in SQL avoids loading another private payload.
        // Equality is exact protobuf bytes, never just a transaction identifier.
        for (table, status) in [
            ("op_log", ConflictReviewStatus::Applied),
            ("pending_txns", ConflictReviewStatus::Queued),
        ] {
            let query = format!("SELECT txn=?1 FROM {table} WHERE txn_id=?2");
            let matched: Option<bool> = self
                .connection
                .query_row(&query, params![&bytes, id.as_str()], |r| r.get(0))
                .optional()?;
            match matched {
                Some(true) => return Ok(status),
                Some(false) => return Err(StoreError::Corrupt("review transaction collision")),
                None => {}
            }
        }
        Ok(ConflictReviewStatus::Draft)
    }

    fn finish_review(
        &self,
        conflict: &Id,
        choice: &ConflictReviewChoice,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        let resolution = match choice {
            ConflictReviewChoice::SwapOther => "swapped",
            ConflictReviewChoice::RestoreCopy { .. } => "restored",
        };
        self.connection.execute("UPDATE conflicts SET resolution=?1,resolved_at=COALESCE(resolved_at,?2) WHERE conflict_id=?3",
            params![resolution, now_ms, conflict.as_str()])?;
        Ok(())
    }
}

fn review_key(id: &Id) -> String {
    format!("{PREFIX}{id}")
}
fn record_hash(record: &pb::ConflictRecord) -> String {
    AssetId::hash(&record.encode_to_vec()).to_string()
}

fn review_operation(
    store: &ProjectStore,
    record: &pb::ConflictRecord,
    choice: &ConflictReviewChoice,
    author: &str,
    wall_ms: i64,
) -> Result<pb::op::Kind, StoreError> {
    use pb::{op::Kind, property_value::Value as Wire};
    let id = Id::from_proto(record.object_id.as_ref())?;
    if let ConflictReviewChoice::RestoreCopy { document, object } = choice {
        if !record.delete_vs_edit || object == &id {
            return Err(StoreError::Invalid("review restore identity"));
        }
        let mut state = record
            .edited_object
            .clone()
            .ok_or(StoreError::Invalid("review restore payload"))?;
        state.object_id = Some(object.to_proto());
        state.group_id = None;
        state.created_by = author.into();
        state.created_at_ms = wall_ms;
        return Ok(Kind::CreateObject(pb::CreateObject {
            document_id: Some(document.to_proto()),
            state: Some(state),
        }));
    }
    if record.delete_vs_edit {
        return Err(StoreError::Invalid("review requires restore copy"));
    }
    let raw = match record.other_value.as_ref().and_then(|v| v.value.as_ref()) {
        Some(Wire::Raw(raw)) => raw,
        _ => return Err(StoreError::Corrupt("review losing value")),
    };
    let other: Value = serde_json::from_slice(raw)?;
    let (collection, path) = record
        .property
        .split_once('.')
        .ok_or(StoreError::Corrupt("review property"))?;
    match collection {
        "documents" if path == "definition.title" => Ok(Kind::UpdateDocument(pb::UpdateDocument {
            document_id: Some(id.to_proto()),
            title: serde_json::from_value(other)?,
        })),
        "objects" => {
            let object = store
                .project()
                .objects
                .get(&id)
                .ok_or(StoreError::Invalid("review object removed"))?;
            let path = path
                .strip_prefix("state.")
                .ok_or(StoreError::Invalid("review unsupported property"))?;
            let mut encoded = serde_json::to_value(&object.state)?;
            replace_leaf(&mut encoded, path, other)?;
            let state: pb::ObjectState = serde_json::from_value(encoded)?;
            if let Some((property, value)) = precise_object_value(path, &state)? {
                return Ok(Kind::SetProperty(pb::SetProperty {
                    object_id: Some(id.to_proto()),
                    property: property.into(),
                    value: Some(pb::PropertyValue { value: Some(value) }),
                }));
            }
            let property = path
                .split('.')
                .next()
                .ok_or(StoreError::Invalid("review property"))?;
            let value = match property {
                "transform" => Wire::Transform(
                    state
                        .transform
                        .ok_or(StoreError::Invalid("review transform"))?,
                ),
                "style" => Wire::Style(state.style.ok_or(StoreError::Invalid("review style"))?),
                "role" => Wire::Role(state.role),
                "locked" => Wire::Flag(state.locked),
                "hidden" => Wire::Flag(state.hidden),
                "order_key" => Wire::Text(state.order_key),
                "shape" => Wire::Raw(serde_json::to_vec(
                    &state.shape.ok_or(StoreError::Invalid("review shape"))?,
                )?),
                _ => return Err(StoreError::Invalid("review unsupported property")),
            };
            Ok(Kind::SetProperty(pb::SetProperty {
                object_id: Some(id.to_proto()),
                property: property.into(),
                value: Some(pb::PropertyValue { value: Some(value) }),
            }))
        }
        "layers" => {
            let mut layer = pb::UpdateLayer {
                layer_id: Some(id.to_proto()),
                ..Default::default()
            };
            match path {
                "definition.name" => layer.name = Some(serde_json::from_value(other)?),
                "definition.order_key" => layer.order_key = Some(serde_json::from_value(other)?),
                "visible" => layer.visible = Some(serde_json::from_value(other)?),
                "locked" => layer.locked = Some(serde_json::from_value(other)?),
                "opacity" => layer.opacity = Some(serde_json::from_value(other)?),
                "blend" => layer.blend = Some(serde_json::from_value(other)?),
                _ => return Err(StoreError::Invalid("review unsupported property")),
            }
            Ok(Kind::UpdateLayer(layer))
        }
        _ => Err(StoreError::Invalid("review unsupported property")),
    }
}

fn replace_leaf(root: &mut Value, path: &str, other: Value) -> Result<(), StoreError> {
    let mut segments = path.split('.').peekable();
    let mut cursor = root;
    while let Some(segment) = segments.next() {
        cursor = cursor
            .get_mut(segment)
            .ok_or(StoreError::Invalid("review property absent"))?;
        if segments.peek().is_none() {
            *cursor = other;
            return Ok(());
        }
    }
    Err(StoreError::Invalid("review property"))
}

// Keep independent style and text fields independent during a later rebase.
// Aggregate shape/style conflicts use their existing coherent typed operation.
fn precise_object_value<'a>(
    path: &'a str,
    state: &pb::ObjectState,
) -> Result<Option<(&'a str, pb::property_value::Value)>, StoreError> {
    use pb::property_value::Value as Wire;
    let style = || {
        state
            .style
            .as_ref()
            .ok_or(StoreError::Invalid("review style"))
    };
    let result = match path {
        "style.width" => (path, Wire::Number(style()?.width)),
        "style.has_fill" => (path, Wire::Flag(style()?.has_fill)),
        "style.screen_constant_width" => (path, Wire::Flag(style()?.screen_constant_width)),
        "style.stroke" | "style.stroke.rgba" => (
            "style.stroke",
            Wire::Color(
                style()?
                    .stroke
                    .ok_or(StoreError::Invalid("review stroke"))?,
            ),
        ),
        "style.fill" | "style.fill.rgba" => {
            // The current setter also enables fill. Even if enabled now, a
            // queued review could re-enable an independently disabled fill on
            // reconnect. Keep this color-only conflict for explicit review.
            return Err(StoreError::Invalid("review fill requires aggregate review"));
        }
        "shape.Text.text" | "shape.Text.font_family" | "shape.Text.font_size" => {
            let Some(pb::object_state::Shape::Text(text)) = &state.shape else {
                return Err(StoreError::Invalid("review text shape"));
            };
            match path {
                "shape.Text.text" => ("text.text", Wire::Text(text.text.clone())),
                "shape.Text.font_family" => {
                    ("text.font_family", Wire::Text(text.font_family.clone()))
                }
                _ => ("text.font_size", Wire::Number(text.font_size)),
            }
        }
        "shape.Marker.number" => {
            let Some(pb::object_state::Shape::Marker(marker)) = &state.shape else {
                return Err(StoreError::Invalid("review marker shape"));
            };
            ("marker.number", Wire::Integer(i64::from(marker.number)))
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}
