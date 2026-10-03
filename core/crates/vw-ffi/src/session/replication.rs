use super::{SessionError, SessionResult};
use crate::{ProjectSession, project::ProjectState};
use std::path::PathBuf;
use vw_model::{AssetId, DeviceId, Id};
use vw_proto::v1 as pb;

pub(crate) async fn call<T: Send + 'static>(
    project: &ProjectSession,
    job: impl FnOnce(&mut ProjectState) -> SessionResult<T> + Send + 'static,
) -> SessionResult<T> {
    project.check_open()?;
    project.worker.call(move |state| Ok(job(state))).await?
}
pub(crate) fn now() -> SessionResult<i64> {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| SessionError::Storage)?
            .as_millis(),
    )
    .map_err(|_| SessionError::Storage)
}
pub(crate) struct State {
    pub project: Id,
    pub host: DeviceId,
    pub local: DeviceId,
    pub revision: pb::Revision,
    pub pending: usize,
    pub blocked: usize,
}
pub(crate) async fn state(project: &ProjectSession) -> SessionResult<State> {
    call(project, |s| {
        let store = s.store()?;
        Ok(State {
            project: store.project().id.clone(),
            host: store.host_device().clone(),
            local: store.local_device().map_err(|_| SessionError::Storage)?,
            revision: store.revision().map_err(|_| SessionError::Storage)?,
            pending: s.replica.as_ref().map_or(0, |r| r.pending().len()),
            blocked: s.replica.as_ref().map_or(0, |r| {
                r.pending()
                    .iter()
                    .filter(|p| p.blocked_reason.is_some())
                    .count()
            }),
        })
    })
    .await
}
pub(crate) async fn pending(project: &ProjectSession) -> SessionResult<Vec<pb::Transaction>> {
    call(project, |s| {
        Ok(s.replica.as_ref().map_or_else(Vec::new, |r| {
            r.pending()
                .iter()
                .filter(|p| p.blocked_reason.is_none())
                .map(|p| p.transaction.clone())
                .collect()
        }))
    })
    .await
}
pub(crate) async fn gesture_closed(
    project: &ProjectSession,
    author: DeviceId,
    gesture: Id,
) -> SessionResult<bool> {
    call(project, move |state| {
        Ok(state.store()?.gesture_closed(&author, &gesture))
    })
    .await
}
pub(crate) async fn cancel_gesture(
    project: &ProjectSession,
    author: DeviceId,
    gesture: Id,
) -> SessionResult<()> {
    call(project, move |state| {
        // Acceptance wins a cross-channel cancel race. Retiring its preview must
        // never rewrite accepted history or add a conflicting cancellation.
        if !state.store()?.gesture_closed(&author, &gesture) {
            state
                .store
                .as_mut()
                .ok_or(SessionError::Closed)?
                .cancel_gesture(author, gesture, now()?)
                .map_err(|_| SessionError::Storage)?;
        }
        Ok(())
    })
    .await
}
pub(crate) async fn request(project: &ProjectSession) -> SessionResult<pb::SyncRequest> {
    call(project, |s| {
        Ok(s.remote.as_ref().ok_or(SessionError::Invalid)?.request()?)
    })
    .await
}
pub(crate) async fn respond(
    project: &ProjectSession,
    request: pb::SyncRequest,
) -> SessionResult<vw_net::SyncResponse> {
    call(project, move |s| {
        Ok(vw_net::HostSync::from_store(s.store()?)?.respond(&request)?)
    })
    .await
}
pub(crate) async fn submit(
    project: &ProjectSession,
    txn: pb::Transaction,
    peer: DeviceId,
) -> SessionResult<vw_ops::Acceptance> {
    call(project, move |s| {
        if s.replica.is_some()
            || s.store()?
                .local_device()
                .map_err(|_| SessionError::Storage)?
                != *s.store()?.host_device()
        {
            return Err(SessionError::Authentication);
        }
        for (id, size) in transaction_assets(&txn)? {
            if s.blobs.verify(&id).map_err(|_| SessionError::Storage)? != size {
                return Err(SessionError::Storage);
            }
        }
        let accepted = s
            .store
            .as_mut()
            .ok_or(SessionError::Closed)?
            .commit(&txn, &peer, now()?)
            .map_err(|_| SessionError::Invalid)?;
        s.history.accept(&txn, &accepted)?;
        let info = s.info()?;
        s.events.publish(crate::ChangeKind::Committed, info);
        s.changed.notify_one();
        Ok(accepted)
    })
    .await
}
fn persist(
    s: &mut ProjectState,
    remote: vw_net::SyncReceiver,
    snapshot: vw_ops::HostSnapshot,
) -> SessionResult<()> {
    let mut replica = s.replica.as_ref().ok_or(SessionError::Invalid)?.clone();
    replica
        .receive(snapshot)
        .map_err(|_| SessionError::Invalid)?;
    remote.persist(s.store.as_mut().ok_or(SessionError::Closed)?, now()?)?;
    // Publication follows the single SQLite transaction which installs the
    // accepted journal and removes only exact acknowledged outbox entries.
    s.history = crate::editor::History::restore(s.store()?)?;
    s.replica = Some(replica);
    s.remote = Some(remote);
    let info = s.info()?;
    s.events.publish(crate::ChangeKind::Committed, info);
    s.changed.notify_one();
    Ok(())
}
pub(crate) async fn receive(
    project: &ProjectSession,
    batch: pb::SyncBatch,
    peer: DeviceId,
) -> SessionResult<bool> {
    call(project, move |s| {
        let mut remote = s.remote.as_ref().ok_or(SessionError::Invalid)?.clone();
        let snapshot = remote.receive(batch, &peer)?;
        if let Some(snapshot) = snapshot {
            persist(s, remote, snapshot)?;
            Ok(true)
        } else {
            s.remote = Some(remote);
            Ok(false)
        }
    })
    .await
}
pub(crate) async fn checkpoint(
    project: &ProjectSession,
    bytes: Vec<u8>,
    peer: DeviceId,
) -> SessionResult<()> {
    call(project, move |s| {
        let mut remote = s.remote.as_ref().ok_or(SessionError::Invalid)?.clone();
        let snapshot = remote.install_checkpoint(&bytes, &peer)?;
        persist(s, remote, snapshot)
    })
    .await
}
pub(crate) async fn asset_path(
    project: &ProjectSession,
    id: AssetId,
) -> SessionResult<(PathBuf, u64)> {
    call(project, move |s| {
        let mut inventory = s
            .store()?
            .retained_assets()
            .map_err(|_| SessionError::Storage)?;
        // Only immutable originals explicitly named by this project's durable
        // outbox may be served before authority acceptance. Arbitrary cache or
        // private files are never addressable by a peer-provided hash.
        for txn in s.store()?.pending().map_err(|_| SessionError::Storage)? {
            for (id, size) in transaction_assets(&txn)? {
                if inventory.insert(id, size).is_some_and(|old| old != size) {
                    return Err(SessionError::Storage);
                }
            }
        }
        let expected = inventory.get(&id).copied().ok_or(SessionError::Invalid)?;
        let size = s.blobs.verify(&id).map_err(|_| SessionError::Storage)?;
        if size != expected {
            return Err(SessionError::Storage);
        }
        Ok((s.blobs.path(&id).map_err(|_| SessionError::Storage)?, size))
    })
    .await
}
fn transaction_assets(
    txn: &pb::Transaction,
) -> SessionResult<std::collections::BTreeMap<AssetId, u64>> {
    let mut assets = std::collections::BTreeMap::new();
    for op in &txn.ops {
        if let Some(pb::op::Kind::AddAsset(asset)) = &op.kind {
            let id =
                AssetId::try_from(asset.asset_id.clone()).map_err(|_| SessionError::Invalid)?;
            if asset.byte_size == 0
                || asset.byte_size > super::link::MAX_TRANSFER
                || assets
                    .insert(id, asset.byte_size)
                    .is_some_and(|old| old != asset.byte_size)
            {
                return Err(SessionError::Invalid);
            }
        }
    }
    Ok(assets)
}
/// A valid transaction authorizes acquisition of only its declared originals.
/// No journal/project mutation occurs until every hash has been persisted.
pub(crate) async fn transaction_missing(
    project: &ProjectSession,
    txn: pb::Transaction,
    peer: DeviceId,
) -> SessionResult<Vec<(AssetId, u64)>> {
    call(project, move |state| {
        if state.replica.is_some()
            || state.store()?.host_device()
                != &state
                    .store()?
                    .local_device()
                    .map_err(|_| SessionError::Storage)?
        {
            return Err(SessionError::Authentication);
        }
        state
            .store()?
            .validate_transaction(&txn, &peer, now()?)
            .map_err(|_| SessionError::Invalid)?;
        let mut missing = Vec::new();
        for (id, size) in transaction_assets(&txn)? {
            let path = state.blobs.path(&id).map_err(|_| SessionError::Storage)?;
            if !path.try_exists().map_err(|_| SessionError::Storage)? {
                missing.push((id, size));
            } else if state.blobs.verify(&id).map_err(|_| SessionError::Storage)? != size {
                return Err(SessionError::Storage);
            }
        }
        Ok(missing)
    })
    .await
}
pub(crate) async fn install_transaction_asset(
    project: &ProjectSession,
    txn: pb::Transaction,
    peer: DeviceId,
    id: AssetId,
    mut file: std::fs::File,
    size: u64,
) -> SessionResult<()> {
    call(project, move |state| {
        use std::io::{Seek, SeekFrom};
        state
            .store()?
            .validate_transaction(&txn, &peer, now()?)
            .map_err(|_| SessionError::Invalid)?;
        if transaction_assets(&txn)?.get(&id) != Some(&size)
            || file.metadata().map_err(|_| SessionError::Storage)?.len() != size
        {
            return Err(SessionError::Invalid);
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|_| SessionError::Storage)?;
        if state
            .blobs
            .put_reader(&mut file)
            .map_err(|_| SessionError::Storage)?
            != id
        {
            return Err(SessionError::Storage);
        }
        Ok(())
    })
    .await
}
pub(crate) async fn missing_assets(project: &ProjectSession) -> SessionResult<Vec<(AssetId, u64)>> {
    call(project, |s| {
        let mut missing = Vec::new();
        for (id, size) in s
            .store()?
            .retained_assets()
            .map_err(|_| SessionError::Storage)?
        {
            match s
                .blobs
                .path(&id)
                .map_err(|_| SessionError::Storage)?
                .try_exists()
            {
                Ok(false) => missing.push((id.clone(), size)),
                Ok(true) => {
                    if s.blobs.verify(&id).map_err(|_| SessionError::Storage)? != size {
                        return Err(SessionError::Storage);
                    }
                }
                Err(_) => return Err(SessionError::Storage),
            }
        }
        Ok(missing)
    })
    .await
}
pub(crate) async fn install_asset(
    project: &ProjectSession,
    id: AssetId,
    mut file: std::fs::File,
    size: u64,
) -> SessionResult<()> {
    call(project, move |s| {
        use std::io::{Seek, SeekFrom};
        if s.store()?
            .retained_assets()
            .map_err(|_| SessionError::Storage)?
            .get(&id)
            != Some(&size)
        {
            return Err(SessionError::Invalid);
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|_| SessionError::Storage)?;
        if s.blobs
            .put_reader(&mut file)
            .map_err(|_| SessionError::Storage)?
            != id
        {
            return Err(SessionError::Storage);
        }
        let info = s.info()?;
        s.events.publish(crate::ChangeKind::Committed, info);
        Ok(())
    })
    .await
}
pub(crate) async fn transfer_file(
    project: &ProjectSession,
) -> SessionResult<tempfile::NamedTempFile> {
    call(project, |s| {
        s.store()?
            .transfer_file()
            .map_err(|_| SessionError::Storage)
    })
    .await
}
