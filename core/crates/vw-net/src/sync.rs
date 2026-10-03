use crate::{NetError, Result};
use std::collections::BTreeMap;
use vw_model::{AssetId, DeviceId, Id};
use vw_ops::{Acceptance, HostSequencer, HostSnapshot};
use vw_proto::{Message, v1};

const SYNC_BYTES: usize = 8 * 1024 * 1024 - 256;
pub struct SyncResponse {
    pub batch: v1::SyncBatch,
    pub checkpoint: Option<Vec<u8>>,
}
pub struct HostSync {
    host: HostSequencer,
    journal: BTreeMap<u64, v1::AckedTransaction>,
    initial: v1::Revision,
}
impl HostSync {
    pub fn new(host: HostSequencer) -> Result<Self> {
        let mut journal = BTreeMap::new();
        for (txn, ack) in host.accepted_transactions() {
            let id = Id::from_proto(txn.txn_id.as_ref())?;
            let accepted_at_ms = host
                .accepted_at(&id)
                .ok_or(NetError::Invalid("host receipt"))?;
            journal.insert(
                ack.host_seq,
                v1::AckedTransaction {
                    host_seq: ack.host_seq,
                    state_hash: ack.state_hash.clone(),
                    txn: Some(txn.clone()),
                    ack: Some(ack.clone()),
                    accepted_at_ms,
                },
            );
        }
        let initial = if let Some(first) = journal.values().next() {
            first
                .txn
                .as_ref()
                .and_then(|t| t.base_revision.clone())
                .ok_or(NetError::Invalid("initial revision"))?
        } else {
            host.revision()?
        };
        Ok(Self {
            host,
            journal,
            initial,
        })
    }
    pub fn from_store(store: &vw_store::ProjectStore) -> Result<Self> {
        Self::new(HostSequencer::from_checkpoint_bytes(
            &store.checkpoint_bytes()?,
        )?)
    }
    pub const fn host(&self) -> &HostSequencer {
        &self.host
    }
    /// In-memory engine entry used by deterministic simulation. Applications use
    /// commit_persisted, whose SQLite commit precedes exposure of the receipt.
    pub fn submit(
        &mut self,
        txn: v1::Transaction,
        peer: &DeviceId,
        now_ms: i64,
    ) -> Result<Acceptance> {
        let accepted = self.host.submit(txn.clone(), peer, now_ms)?;
        if !accepted.duplicate {
            self.journal.insert(
                accepted.ack.host_seq,
                v1::AckedTransaction {
                    host_seq: accepted.ack.host_seq,
                    state_hash: accepted.ack.state_hash.clone(),
                    txn: Some(txn),
                    ack: Some(accepted.ack.clone()),
                    accepted_at_ms: now_ms,
                },
            );
        }
        Ok(accepted)
    }
    pub fn commit_persisted(
        &mut self,
        store: &mut vw_store::ProjectStore,
        txn: v1::Transaction,
        peer: &DeviceId,
        now_ms: i64,
    ) -> Result<Acceptance> {
        if store.revision()? != self.host.revision()?
            || store.project().id != self.host.project().id
        {
            return Err(NetError::Resync);
        }
        let accepted = store.commit(&txn, peer, now_ms)?;
        // Reload preserves durable gesture cancellation and exact original retry
        // receipt times as well as project state. A crash here is safely retried.
        *self = Self::from_store(store)?;
        Ok(accepted)
    }
    pub fn respond(&self, request: &v1::SyncRequest) -> Result<SyncResponse> {
        if request.project_id.as_ref() != Some(&self.host.project().id.to_proto())
            || !(1..=256).contains(&request.max_transactions)
        {
            return Err(NetError::Invalid("sync request"));
        }
        let known = if request.since_host_seq == self.initial.host_seq {
            Some(self.initial.state_hash.as_slice())
        } else {
            self.journal
                .get(&request.since_host_seq)
                .map(|r| r.state_hash.as_slice())
        };
        let revision = self.host.revision()?;
        let mut batch = v1::SyncBatch {
            project_id: request.project_id.clone(),
            base: Some(v1::Revision {
                host_seq: request.since_host_seq,
                state_hash: request.state_hash.clone(),
            }),
            revision: Some(revision.clone()),
            ..Default::default()
        };
        if known != Some(request.state_hash.as_slice()) || request.state_hash.len() != 32 {
            return self.checkpoint(batch);
        }
        for (_, entry) in self
            .journal
            .range((
                std::ops::Bound::Excluded(request.since_host_seq),
                std::ops::Bound::Unbounded,
            ))
            .take(request.max_transactions as usize)
        {
            batch.txns.push(entry.clone());
            if batch.encoded_len() > SYNC_BYTES {
                batch.txns.pop();
                if batch.txns.is_empty() {
                    return self.checkpoint(batch);
                }
                break;
            }
        }
        batch.complete = batch
            .txns
            .last()
            .map_or(request.since_host_seq, |t| t.host_seq)
            == revision.host_seq;
        Ok(SyncResponse {
            batch,
            checkpoint: None,
        })
    }
    fn checkpoint(&self, mut batch: v1::SyncBatch) -> Result<SyncResponse> {
        let bytes = self.host.checkpoint_bytes()?;
        batch.txns.clear();
        batch.complete = true;
        batch.checkpoint_asset_id = AssetId::hash(&bytes).to_string();
        batch.checkpoint_size = bytes.len() as u64;
        Ok(SyncResponse {
            batch,
            checkpoint: Some(bytes),
        })
    }
}

#[derive(Clone)]
pub struct SyncReceiver {
    mirror: HostSequencer,
    checkpoint: Option<v1::SyncBatch>,
}
impl SyncReceiver {
    /// Seed from a validated local store or previously authenticated checkpoint.
    pub fn new(mirror: HostSequencer) -> Self {
        Self {
            mirror,
            checkpoint: None,
        }
    }
    pub const fn host(&self) -> &HostSequencer {
        &self.mirror
    }
    pub fn request(&self) -> Result<v1::SyncRequest> {
        let revision = self.mirror.revision()?;
        Ok(v1::SyncRequest {
            since_host_seq: revision.host_seq,
            project_id: Some(self.mirror.project().id.to_proto()),
            state_hash: revision.state_hash,
            max_transactions: 256,
        })
    }
    /// Only pass bodies delivered by a Session authenticated as this project's
    /// host. The explicit peer binding prevents a different paired device from
    /// becoming a sequencer by sending a syntactically valid checkpoint.
    pub fn receive(
        &mut self,
        batch: v1::SyncBatch,
        peer: &DeviceId,
    ) -> Result<Option<HostSnapshot>> {
        if peer != self.mirror.host_device()
            || batch.project_id.as_ref() != Some(&self.mirror.project().id.to_proto())
            || batch.encoded_len() > SYNC_BYTES
            || batch.txns.len() > 256
        {
            return Err(NetError::Authentication);
        }
        let revision = batch
            .revision
            .as_ref()
            .ok_or(NetError::Invalid("sync revision"))?;
        if revision.state_hash.len() != 32 || revision.host_seq < self.mirror.revision()?.host_seq {
            return Err(NetError::Resync);
        }
        if !batch.checkpoint_asset_id.is_empty() {
            AssetId::try_from(batch.checkpoint_asset_id.clone())?;
            if !batch.txns.is_empty()
                || !batch.complete
                || batch.checkpoint_size == 0
                || batch.checkpoint_size > 256 * 1024 * 1024
            {
                return Err(NetError::Invalid("checkpoint manifest"));
            }
            self.checkpoint = Some(batch);
            return Ok(None);
        }
        if batch.checkpoint_size != 0 || batch.base.as_ref() != Some(&self.mirror.revision()?) {
            return Err(NetError::Resync);
        }
        let mut staged = self.mirror.clone();
        for entry in &batch.txns {
            let txn = entry
                .txn
                .as_ref()
                .ok_or(NetError::Invalid("sync transaction"))?;
            let author = DeviceId::try_from(txn.device_id.clone())?;
            if entry.host_seq
                != staged
                    .revision()?
                    .host_seq
                    .checked_add(1)
                    .ok_or(NetError::Invalid("host sequence"))?
                || entry.host_seq > revision.host_seq
            {
                return Err(NetError::Invalid("sync continuity"));
            }
            let accepted = staged.submit(txn.clone(), &author, entry.accepted_at_ms)?;
            if accepted.duplicate
                || entry.ack.as_ref() != Some(&accepted.ack)
                || entry.state_hash != accepted.ack.state_hash
                || entry.host_seq != accepted.ack.host_seq
            {
                return Err(NetError::Integrity);
            }
        }
        let current = staged.revision()?;
        if (batch.complete && &current != revision)
            || (!batch.complete && (batch.txns.is_empty() || current.host_seq >= revision.host_seq))
        {
            return Err(NetError::Invalid("sync completion"));
        }
        let snapshot = staged.snapshot()?;
        self.mirror = staged;
        self.checkpoint = None;
        Ok(Some(snapshot))
    }
    pub fn install_checkpoint(&mut self, bytes: &[u8], peer: &DeviceId) -> Result<HostSnapshot> {
        let manifest = self
            .checkpoint
            .as_ref()
            .ok_or(NetError::Invalid("unsolicited checkpoint"))?;
        if peer != self.mirror.host_device()
            || bytes.len() as u64 != manifest.checkpoint_size
            || AssetId::hash(bytes).as_str() != manifest.checkpoint_asset_id
        {
            return Err(NetError::Integrity);
        }
        let staged = HostSequencer::from_checkpoint_bytes(bytes)?;
        if staged.host_device() != peer
            || staged.project().id != self.mirror.project().id
            || manifest.revision.as_ref() != Some(&staged.revision()?)
        {
            return Err(NetError::Integrity);
        }
        let previous = self.mirror.revision()?;
        let next = staged.revision()?;
        if next.host_seq < previous.host_seq
            || (next.host_seq == previous.host_seq && next != previous)
            || !self.mirror.cancellations_are_subset_of(&staged)
            || !staged.conflicts().starts_with(self.mirror.conflicts())
        {
            return Err(NetError::Resync);
        }
        for (id, (txn, ack)) in self.mirror.snapshot()?.accepted {
            if staged.accepted_transaction(&id) != Some((&txn, &ack))
                || staged.accepted_at(&id) != self.mirror.accepted_at(&id)
            {
                return Err(NetError::Integrity);
            }
        }
        if previous.host_seq == 0 && next.host_seq > 0 {
            let candidate = staged.snapshot()?;
            let first = candidate
                .accepted
                .values()
                .find(|(_, ack)| ack.host_seq == 1)
                .and_then(|(txn, _)| txn.base_revision.as_ref());
            if first != Some(&previous) {
                return Err(NetError::Integrity);
            }
        }
        let snapshot = staged.snapshot()?;
        self.mirror = staged;
        self.checkpoint = None;
        Ok(snapshot)
    }
    /// Persist the complete verified host journal/projections and remove exact
    /// matching pending work in one SQLite transaction. A crash cannot leave
    /// an old durable state with its only pending copy already deleted.
    pub fn persist(&self, store: &mut vw_store::ProjectStore, now_ms: i64) -> Result<usize> {
        Ok(store.install_authenticated_checkpoint(
            &self.mirror.checkpoint_bytes()?,
            self.mirror.host_device(),
            now_ms,
        )?)
    }
    /// Compatibility entry point: acknowledgement now includes durable state
    /// installation. Prefer `persist` when the application supplies receipt time.
    pub fn acknowledge_pending(&self, store: &mut vw_store::ProjectStore) -> Result<usize> {
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| NetError::Invalid("checkpoint clock"))?;
        let now_ms = i64::try_from(elapsed.as_millis())
            .map_err(|_| NetError::Invalid("checkpoint clock"))?;
        self.persist(store, now_ms)
    }
}
