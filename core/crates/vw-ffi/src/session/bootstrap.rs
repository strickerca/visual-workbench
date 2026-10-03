use super::{
    SessionEndpoint, SessionError, SessionResult, SessionService, device,
    link::{self, MAX_TRANSFER},
    runtime::NetworkRuntime,
};
use crate::{ProjectSession, project::SessionPermit};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use vw_model::{AssetId, Id};
use vw_net::{ConnectionReceiver, ConnectionSender, Receive};
use vw_proto::v1::{self as pb, envelope::Body};

const PROGRESS_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, uniffi::Record)]
pub struct ReceiveProjectOptions {
    pub path: String,
    pub peer_device_id: String,
    pub endpoints: Vec<SessionEndpoint>,
    pub expected_project_id: Option<String>,
    pub max_total_blob_bytes: u64,
}
#[uniffi::export]
impl SessionService {
    /// Download into an owned sibling staging directory. A project is published
    /// only after its authenticated checkpoint and every required original pass
    /// length/hash verification and SQLite recovery/integrity checks.
    pub async fn receive_project(
        &self,
        options: ReceiveProjectOptions,
        cancellation: Arc<crate::Cancellation>,
    ) -> SessionResult<Arc<ProjectSession>> {
        if options.path.len() > 32767
            || options.path.contains('\0')
            || !Path::new(&options.path).is_absolute()
            || options.max_total_blob_bytes == 0
            || options.max_total_blob_bytes > 16 * 1024 * 1024 * 1024
        {
            return Err(SessionError::Invalid);
        }
        let peer = device(options.peer_device_id)?;
        let addresses = link::routes(options.endpoints, false)?;
        let expected = options
            .expected_project_id
            .map(Id::try_from)
            .transpose()
            .map_err(|_| SessionError::Invalid)?;
        let manager = self.manager.clone();
        let permit = crate::worker::startup(SessionPermit::acquire).await?;
        let runtime = crate::worker::startup(|| Ok(NetworkRuntime::new())).await??;
        let result = runtime
            .cancellable(cancellation.clone(), async move {
                cancellation.check()?;
                let destination = Path::new(&options.path);
                let parent = destination
                    .parent()
                    .ok_or(SessionError::Invalid)?
                    .canonicalize()
                    .map_err(|_| SessionError::Storage)?;
                let destination =
                    parent.join(destination.file_name().ok_or(SessionError::Invalid)?);
                match std::fs::symlink_metadata(&destination) {
                    Ok(_) => return Err(SessionError::Invalid),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(SessionError::Storage),
                }
                let staging = tempfile::Builder::new()
                    .prefix(".vw-receive-")
                    .tempdir_in(&parent)
                    .map_err(|_| SessionError::Storage)?;
                let local = manager.identity()?.device().clone();
                let (_, connection) = link::establish(
                    &link::Route::Client(addresses),
                    &manager,
                    &peer,
                    &link::hello(local.clone()),
                )
                .await?;
                if !connection.session().capabilities().contains("app_sync_v1") {
                    return Err(SessionError::Invalid);
                }
                let (sender, mut receiver) = connection.into_duplex()?;
                let _closing = Close(sender.clone());
                let start = Instant::now();
                let deadline = tokio::time::Instant::now() + PROGRESS_TIMEOUT;
                let state = loop {
                    if let Body::SessionState(state) =
                        next(&sender, &mut receiver, start, deadline).await?
                    {
                        break state;
                    }
                };
                let project = Id::from_proto(state.open_project_id.as_ref())
                    .map_err(|_| SessionError::Invalid)?;
                if expected.as_ref().is_some_and(|id| id != &project) {
                    return Err(SessionError::Authentication);
                }
                sender.enqueue(Body::SyncRequest(pb::SyncRequest {
                    since_host_seq: 0,
                    project_id: Some(project.to_proto()),
                    state_hash: vec![],
                    max_transactions: 256,
                }))?;
                let deadline = tokio::time::Instant::now() + PROGRESS_TIMEOUT;
                let batch = loop {
                    match next(&sender, &mut receiver, start, deadline).await? {
                        Body::SyncBatch(batch) => break batch,
                        Body::SessionState(_) => {}
                        _ => return Err(SessionError::Invalid),
                    }
                };
                if batch.project_id.as_ref() != Some(&project.to_proto())
                    || !batch.txns.is_empty()
                    || !batch.complete
                    || batch.checkpoint_size == 0
                    || batch.checkpoint_size > MAX_TRANSFER
                {
                    return Err(SessionError::Invalid);
                }
                let checkpoint_id = AssetId::try_from(batch.checkpoint_asset_id)
                    .map_err(|_| SessionError::Invalid)?;
                let mut checkpoint = download(
                    &sender,
                    &mut receiver,
                    start,
                    checkpoint_id,
                    batch.checkpoint_size,
                    staging.path(),
                )
                .await?;
                let mut bytes = Vec::new();
                bytes
                    .try_reserve_exact(batch.checkpoint_size as usize)
                    .map_err(|_| SessionError::Backpressure)?;
                checkpoint
                    .as_file_mut()
                    .seek(SeekFrom::Start(0))
                    .map_err(|_| SessionError::Storage)?;
                checkpoint
                    .as_file_mut()
                    .take(batch.checkpoint_size + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| SessionError::Storage)?;
                let host = vw_ops::HostSequencer::from_checkpoint_bytes(&bytes)
                    .map_err(|_| SessionError::Invalid)?;
                if host.host_device() != &peer
                    || host.project().id != project
                    || batch.revision.as_ref()
                        != Some(&host.revision().map_err(|_| SessionError::Invalid)?)
                {
                    return Err(SessionError::Authentication);
                }
                drop(host);
                cancellation.check()?;
                let candidate = staging.path().join("project");
                let store = vw_store::ProjectStore::create_authenticated_checkpoint(
                    &candidate,
                    &bytes,
                    &peer,
                    &local,
                    super::replication::now()?,
                )
                .map_err(|_| SessionError::Storage)?;
                drop(bytes);
                drop(checkpoint);
                // Undo hides assets from the visible model without releasing their
                // immutable originals. Bootstrap the complete retained inventory.
                let assets = store.retained_assets().map_err(|_| SessionError::Storage)?;
                let mut total = 0u64;
                for size in assets.values() {
                    if *size == 0 || *size > MAX_TRANSFER {
                        return Err(SessionError::Backpressure);
                    }
                    total = total.checked_add(*size).ok_or(SessionError::Backpressure)?;
                }
                if total > options.max_total_blob_bytes {
                    return Err(SessionError::Backpressure);
                }
                let blobs =
                    vw_store::BlobStore::new(&candidate).map_err(|_| SessionError::Storage)?;
                for (id, size) in assets {
                    cancellation.check()?;
                    let mut asset = download(
                        &sender,
                        &mut receiver,
                        start,
                        id.clone(),
                        size,
                        staging.path(),
                    )
                    .await?;
                    cancellation.check()?;
                    asset
                        .as_file_mut()
                        .seek(SeekFrom::Start(0))
                        .map_err(|_| SessionError::Storage)?;
                    if blobs
                        .put_reader(asset.as_file_mut())
                        .map_err(|_| SessionError::Storage)?
                        != id
                        || blobs.verify(&id).map_err(|_| SessionError::Storage)? != size
                    {
                        return Err(SessionError::Storage);
                    }
                }
                cancellation.check()?;
                store.integrity_check().map_err(|_| SessionError::Storage)?;
                drop(store);
                drop(blobs);
                // The core's normal open verifies the complete retained journal,
                // snapshot, materialized projections and device-owned outbox.
                let verified =
                    vw_store::ProjectStore::open(&candidate).map_err(|_| SessionError::Storage)?;
                drop(verified);
                publish_received(&candidate, &destination, &parent, &cancellation)?;
                ProjectSession::from_store(
                    vw_store::ProjectStore::open(&destination)
                        .map_err(|_| SessionError::Storage)?,
                    vw_store::BlobStore::new(&destination).map_err(|_| SessionError::Storage)?,
                    permit,
                )
                .map_err(SessionError::from)
            })
            .await;
        runtime.close().await?;
        result
    }
}
fn publish_received(
    candidate: &Path,
    destination: &Path,
    parent: &Path,
    cancellation: &crate::Cancellation,
) -> SessionResult<()> {
    crate::creation::sync_directory(candidate)?;
    // The asynchronous cancellation wrapper cannot interrupt synchronous
    // verification/fsync. Observe the token at the actual publication boundary.
    cancellation.check()?;
    crate::creation::publish(candidate, destination)?;
    crate::creation::sync_directory(parent)?;
    Ok(())
}
struct Close(ConnectionSender);
impl Drop for Close {
    fn drop(&mut self) {
        self.0.close();
    }
}
async fn next(
    sender: &ConnectionSender,
    receiver: &mut ConnectionReceiver,
    start: Instant,
    deadline: tokio::time::Instant,
) -> SessionResult<Body> {
    before_deadline(deadline, async {
        loop {
            let received = receiver
                .receive(start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
                .await?;
            match received {
                Receive::Discard => {}
                Receive::Deliver(Body::Ping(ping)) => {
                    let now = i64::try_from(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_err(|_| SessionError::Storage)?
                            .as_nanos(),
                    )
                    .map_err(|_| SessionError::Storage)?;
                    sender.enqueue(Body::Pong(pb::Pong {
                        nonce: ping.nonce,
                        t_sent_ns: ping.t_sent_ns,
                        t_recv_ns: now,
                        t_reply_ns: now,
                    }))?;
                }
                Receive::Deliver(Body::Pong(_)) => {}
                Receive::Deliver(body) => return Ok(body),
            }
        }
    })
    .await
}

/// Keepalive and repeated state frames prove connectivity, not download progress.
/// The caller advances this deadline only after the expected control response or
/// additional verified blob bytes, so an otherwise live peer cannot stall a
/// bootstrap forever. A progressing large transfer has no fixed total duration.
async fn before_deadline<T>(
    deadline: tokio::time::Instant,
    work: impl std::future::Future<Output = SessionResult<T>>,
) -> SessionResult<T> {
    // timeout_at may let an immediately ready future win at its deadline. Check
    // explicitly so a stream of ready but irrelevant messages cannot evade it.
    if tokio::time::Instant::now() >= deadline {
        return Err(SessionError::Timeout);
    }
    tokio::time::timeout_at(deadline, work)
        .await
        .map_err(|_| SessionError::Timeout)?
}
async fn download(
    sender: &ConnectionSender,
    receiver: &mut ConnectionReceiver,
    start: Instant,
    asset: AssetId,
    size: u64,
    directory: &Path,
) -> SessionResult<tempfile::NamedTempFile> {
    if size == 0 || size > MAX_TRANSFER {
        return Err(SessionError::Backpressure);
    }
    let temporary =
        tempfile::NamedTempFile::new_in(directory).map_err(|_| SessionError::Storage)?;
    let mut blob = vw_net::BlobReceiver::resume(
        temporary.reopen().map_err(|_| SessionError::Storage)?,
        asset.clone(),
        size,
        0,
        MAX_TRANSFER,
    )?;
    sender.enqueue(Body::BlobRequest(pb::BlobRequest {
        asset_id: asset.to_string(),
        offset: 0,
        length: size,
        priority: 0,
    }))?;
    let mut deadline = tokio::time::Instant::now() + PROGRESS_TIMEOUT;
    let mut received_bytes = 0;
    loop {
        match next(sender, receiver, start, deadline).await? {
            Body::BlobChunk(chunk) => {
                let ack = blob.receive(&chunk)?;
                if ack.next_offset > received_bytes {
                    received_bytes = ack.next_offset;
                    deadline = tokio::time::Instant::now() + PROGRESS_TIMEOUT;
                }
                if ack.verified {
                    let file = blob.finish()?;
                    file.sync_all().map_err(|_| SessionError::Storage)?;
                    drop(file);
                    sender.enqueue(Body::BlobAck(ack))?;
                    return Ok(temporary);
                }
                sender.enqueue(Body::BlobAck(ack))?;
            }
            Body::SessionState(_) => {}
            _ => return Err(SessionError::Invalid),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn an_expired_progress_deadline_refuses_even_immediately_ready_work() {
        let result = before_deadline(tokio::time::Instant::now(), async { Ok(()) }).await;
        assert!(matches!(result, Err(SessionError::Timeout)));
    }
    #[tokio::test]
    async fn keepalive_activity_cannot_extend_the_progress_deadline() {
        let mut keepalives = 0;
        let result: SessionResult<()> = before_deadline(
            tokio::time::Instant::now() + Duration::from_millis(60),
            async {
                loop {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                    keepalives += 1;
                    // Ping/Pong and repeated SessionState do not complete the
                    // awaited control response or advance the blob offset.
                }
            },
        )
        .await;
        assert!(matches!(result, Err(SessionError::Timeout)));
        assert!(keepalives > 0);
    }
    #[test]
    fn cancellation_after_staging_prevents_publication_and_preserves_other_content() {
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("received");
        std::fs::write(parent.path().join("sentinel"), b"owner content").unwrap();
        {
            let staging = tempfile::Builder::new()
                .prefix(".vw-receive-")
                .tempdir_in(parent.path())
                .unwrap();
            let candidate = staging.path().join("project");
            std::fs::create_dir(&candidate).unwrap();
            std::fs::write(candidate.join("verified-candidate"), b"complete staging").unwrap();
            let cancellation = crate::Cancellation::new();
            cancellation.cancel();
            assert!(matches!(
                publish_received(&candidate, &destination, parent.path(), &cancellation),
                Err(SessionError::Cancelled)
            ));
            assert!(candidate.exists());
            assert!(!destination.exists());
        }
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 1);
        assert_eq!(
            std::fs::read(parent.path().join("sentinel")).unwrap(),
            b"owner content"
        );
    }
}
