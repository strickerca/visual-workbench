use crate::{NetError, Result};
use vw_model::{AssetId, DeviceId};
use vw_ops::{Acceptance, HostSequencer, HostSnapshot};
use vw_proto::{Message, v1};

const SYNC_BYTES: usize = 8 * 1024 * 1024 - 256;
pub struct SyncResponse {
    pub batch: v1::SyncBatch,
    pub checkpoint: Option<Vec<u8>>,
}
pub struct HostSync {
    host: HostSequencer,
}
impl HostSync {
    pub fn new(host: HostSequencer) -> Result<Self> {
        Ok(Self { host })
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
        Ok(self.host.submit(txn, peer, now_ms)?)
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
        respond(&self.host, request)
    }
}

/// What the host answers a device's sync request with, read from its
/// sequencing state as it is: the transactions after the base the device
/// names, up to the request's count and the batch size, or a checkpoint
/// when the host does not know that base. A host that keeps its state in a
/// store answers from the store's own (`ProjectStore::host`), so a request
/// costs what it sends, not a copy of the history.
pub fn respond(host: &HostSequencer, request: &v1::SyncRequest) -> Result<SyncResponse> {
    if request.project_id.as_ref() != Some(&host.project().id.to_proto())
        || !(1..=256).contains(&request.max_transactions)
    {
        return Err(NetError::Invalid("sync request"));
    }
    // Every revision the host reached, revision zero included, is a base it
    // knows.
    let known = host
        .revision_hash(request.since_host_seq)
        .map(|hash| hash.bytes());
    let revision = host.revision()?;
    let mut batch = v1::SyncBatch {
        project_id: request.project_id.clone(),
        base: Some(v1::Revision {
            host_seq: request.since_host_seq,
            state_hash: request.state_hash.clone(),
        }),
        revision: Some(revision.clone()),
        ..Default::default()
    };
    if known.as_ref().map(<[u8; 32]>::as_slice) != Some(request.state_hash.as_slice())
        || request.state_hash.len() != 32
    {
        return checkpoint(host, batch);
    }
    for (txn, ack, accepted_at_ms) in host
        .accepted_after(request.since_host_seq)
        .take(request.max_transactions as usize)
    {
        batch.txns.push(v1::AckedTransaction {
            host_seq: ack.host_seq,
            state_hash: ack.state_hash.clone(),
            txn: Some(txn.clone()),
            ack: Some(ack.clone()),
            accepted_at_ms,
        });
        if batch.encoded_len() > SYNC_BYTES {
            batch.txns.pop();
            if batch.txns.is_empty() {
                return checkpoint(host, batch);
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
fn checkpoint(host: &HostSequencer, mut batch: v1::SyncBatch) -> Result<SyncResponse> {
    let bytes = host.checkpoint_bytes()?;
    batch.txns.clear();
    batch.complete = true;
    batch.checkpoint_asset_id = AssetId::hash(&bytes).to_string();
    batch.checkpoint_size = bytes.len() as u64;
    Ok(SyncResponse {
        batch,
        checkpoint: Some(bytes),
    })
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
            // What the host decided is the transaction's revision and state
            // hash, and this device has to decide the same. The notices in
            // its acknowledgement are text for a person: another build words
            // them differently, or gives more or fewer, and that must not
            // break the session. They are kept as the host sent them, which
            // is what a retry is answered with and what gets stored. This is
            // the rule storage follows when it decides a log again (see the
            // replay rule at `evaluate` in vw-ops).
            let ack = entry.ack.as_ref().ok_or(NetError::Integrity)?;
            if ack.txn_id != txn.txn_id
                || ack.host_seq != entry.host_seq
                || ack.state_hash != entry.state_hash
            {
                return Err(NetError::Integrity);
            }
            if staged
                .replay(txn.clone(), entry.accepted_at_ms, ack)?
                .is_none()
            {
                // This engine refuses the transaction or decides another state.
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
