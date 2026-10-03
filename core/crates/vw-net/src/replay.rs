//! Bounded identity history for one connection. Retiring an identity advances
//! only its channel's replay floor; identities still tracked retain their own
//! watermark so a delayed update to a live identity is not discarded.
use crate::{NetError, Result};
use std::collections::BTreeMap;

pub(crate) type Key = (i32, String);
#[derive(Clone, Copy)]
pub(crate) struct Mark {
    pub sequence: u64,
    pub value: u64,
    used: u64,
}
pub(crate) struct ReplayWindow {
    entries: BTreeMap<Key, Mark>,
    order: BTreeMap<u64, Key>,
    retired: [u64; 7],
    clock: u64,
    limit: usize,
}
impl ReplayWindow {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            order: BTreeMap::new(),
            retired: [0; 7],
            clock: 0,
            limit,
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.retired = [0; 7];
        self.clock = 0;
    }
    pub fn get(&self, key: &Key) -> Option<Mark> {
        self.entries.get(key).copied()
    }
    pub fn accepts(&self, key: &Key, sequence: u64) -> bool {
        let watermark = self
            .get(key)
            .map_or(self.retired[key.0 as usize], |mark| mark.sequence);
        sequence > watermark
    }
    /// Protect all queued identities. With every slot live, overflow is real
    /// backpressure; consuming frames makes their history eligible to retire.
    pub fn make_room(
        &mut self,
        key: &Key,
        protected: impl Fn(&Key) -> bool,
    ) -> Result<Option<Key>> {
        if self.entries.contains_key(key) || self.entries.len() < self.limit {
            return Ok(None);
        }
        let victim = self
            .order
            .values()
            .find(|candidate| !protected(candidate))
            .cloned()
            .ok_or(NetError::Backpressure)?;
        let mark = self
            .entries
            .remove(&victim)
            .ok_or(NetError::Invalid("replay history invariant"))?;
        self.order.remove(&mark.used);
        let floor = &mut self.retired[victim.0 as usize];
        *floor = (*floor).max(mark.sequence);
        Ok(Some(victim))
    }
    pub fn record(&mut self, key: Key, sequence: u64, value: u64) -> Result<()> {
        if !self.entries.contains_key(&key) && self.entries.len() >= self.limit {
            return Err(NetError::Backpressure);
        }
        let used = self
            .clock
            .checked_add(1)
            .ok_or(NetError::Invalid("replay history exhausted"))?;
        self.clock = used;
        if let Some(previous) = self.entries.insert(
            key.clone(),
            Mark {
                sequence,
                value,
                used,
            },
        ) {
            self.order.remove(&previous.used);
        }
        self.order.insert(used, key);
        Ok(())
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}
