use crate::OpsError;
use std::collections::{BTreeMap, BTreeSet};
use vw_model::{DeviceId, Id};
use vw_proto::v1;

#[derive(Debug, Clone)]
struct Preview {
    update: v1::GestureUpdate,
    last_ms: u64,
}

/// Session-local provisional updates. Latest sequence wins, stale packets do not
/// refresh expiry, and explicit cancel/commit suppresses late packets for that
/// session. Nothing in this store is serialized into project state or history.
#[derive(Debug, Clone, Default)]
pub struct GestureStore {
    active: BTreeMap<(DeviceId, Id), Preview>,
    finished: BTreeSet<(DeviceId, Id)>,
    watermarks: BTreeMap<(DeviceId, Id), (u32, u64, Id, i32)>,
}

impl GestureStore {
    pub fn new() -> Self {
        Self::default()
    }
    /// Supply a monotonic local receive time in milliseconds. Returns false for
    /// stale/finished updates. Caller identity comes from the authenticated peer.
    pub fn update(
        &mut self,
        device: DeviceId,
        update: v1::GestureUpdate,
        now_ms: u64,
    ) -> Result<bool, OpsError> {
        let id = Id::from_proto(update.gesture_id.as_ref())?;
        let document = Id::from_proto(update.document_id.as_ref())?;
        if !(1..=5).contains(&update.kind) || update.seq == 0 || !update.slider_value.is_finite() {
            return Err(OpsError::Invalid("gesture metadata"));
        }
        for point in &update.handle_positions {
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(OpsError::Invalid("gesture point"));
            }
        }
        if update.handle_positions.len() > 1_000_000 {
            return Err(OpsError::Invalid("gesture point count"));
        }
        if let Some(stroke) = &update.stroke_delta {
            vw_model::validate_stroke(stroke)?;
        }
        if let Some(state) = &update.preview_state {
            vw_model::validate_object_state(state)?;
        }
        let key = (device, id);
        if self.finished.contains(&key) {
            return Ok(false);
        }
        if let Some((sequence, last_ms, previous_document, kind)) = self.watermarks.get(&key) {
            if now_ms < *last_ms {
                return Err(OpsError::Invalid("monotonic gesture time"));
            }
            if previous_document != &document || kind != &update.kind {
                return Err(OpsError::Invalid("gesture binding changed"));
            }
            if update.seq <= *sequence {
                return Ok(false);
            }
        }
        if self.watermarks.len() >= 65_536 && !self.watermarks.contains_key(&key) {
            return Err(OpsError::Invalid("gesture session capacity"));
        }
        if self.active.len() >= 1024 && !self.active.contains_key(&key) {
            return Err(OpsError::Invalid("active gesture capacity"));
        }
        self.watermarks
            .insert(key.clone(), (update.seq, now_ms, document, update.kind));
        self.active.insert(
            key,
            Preview {
                update,
                last_ms: now_ms,
            },
        );
        Ok(true)
    }
    /// Remove a cancelled/committed preview immediately and suppress late updates.
    /// The caller also cancels its pending transaction or registers host cancel.
    pub fn finish(&mut self, device: DeviceId, id: Id) -> Result<bool, OpsError> {
        let key = (device, id);
        if self.finished.len() >= 65_536 && !self.finished.contains(&key) {
            return Err(OpsError::Invalid("finished gesture capacity"));
        }
        let removed = self.active.remove(&key).is_some();
        self.finished.insert(key);
        Ok(removed)
    }
    /// Expire exactly 1,000 ms after the last accepted update. Expiry clears only
    /// the preview; an eventual reliable commit may still replace it.
    pub fn expire(&mut self, now_ms: u64) -> Vec<Id> {
        let expired = self
            .active
            .iter()
            .filter(|(_, v)| now_ms.saturating_sub(v.last_ms) >= 1000)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in &expired {
            self.active.remove(key);
        }
        expired.into_iter().map(|(_, id)| id).collect()
    }
    pub fn get(&self, device: &DeviceId, id: &Id) -> Option<&v1::GestureUpdate> {
        self.active
            .get(&(device.clone(), id.clone()))
            .map(|p| &p.update)
    }
}
