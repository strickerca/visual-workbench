//! Caller tests await coordinator registration and the shared build slot.
#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use crate::project::SessionPermit;
use vw_model::{DeviceId, Document, Project};
use vw_store::{BlobStore, ProjectStore};

fn uid(n: u64) -> Id {
    Id::from_parts(1_700_000_000_000 + n, [0x42; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn title(base: pb::Revision, n: u64, who: u8, wall: i64, text: &str) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(uid(n).to_proto()),
        project_id: Some(uid(1).to_proto()),
        device_id: device(who).to_string(),
        base_revision: Some(base),
        created_at_wall_ms: wall,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(who).to_string(),
                lamport: n,
            }),
            kind: Some(pb::op::Kind::UpdateDocument(pb::UpdateDocument {
                document_id: Some(uid(2).to_proto()),
                title: text.into(),
            })),
        }],
        ..Default::default()
    }
}
fn fixture() -> (tempfile::TempDir, Arc<ProjectSession>, Id) {
    let temp = tempfile::tempdir().unwrap();
    let mut project = Project::new(uid(1), "offline review caller".into(), device(1));
    project.documents.insert(
        uid(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(uid(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "initial".into(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 0,
        },
    );
    let path = temp.path().join("host");
    let mut store = ProjectStore::create(&path, project, device(1), 0).unwrap();
    let base = store.revision().unwrap();
    store
        .commit(&title(base.clone(), 10, 1, 100, "host"), &device(1), 110)
        .unwrap();
    store
        .commit(&title(base, 11, 2, 90, "offline"), &device(2), 111)
        .unwrap();
    let conflict = Id::from_proto(store.conflicts().unwrap()[0].conflict_id.as_ref()).unwrap();
    let project = ProjectSession::from_store(
        store,
        BlobStore::new(&path).unwrap(),
        SessionPermit::acquire().unwrap(),
    )
    .unwrap();
    (temp, project, conflict)
}
fn token() -> Arc<Cancellation> {
    Arc::new(Cancellation::new())
}
fn meta() -> WorkflowMetadata {
    WorkflowMetadata {
        transaction_id: uid(100).to_string(),
        device_id: device(1).to_string(),
        first_lamport: 100,
        created_at_ms: 1_000,
    }
}
const BUDGET: u64 = 256 * 1024 * 1024;

#[tokio::test]
async fn host_submit_publishes_normal_history_and_exact_duplicate_only_once() {
    let (_temp, project, conflict) = fixture();
    let before = project.info().await.unwrap();
    let page = project
        .offline_review_page(0, 32, BUDGET, token())
        .await
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].property, "documents.definition.title");
    let detail = project
        .offline_review_details(conflict.to_string(), BUDGET, token())
        .await
        .unwrap();
    assert_eq!(detail.current.preview, "\"host\"");
    assert_eq!(detail.other.preview, "\"offline\"");
    let staged = project
        .stage_offline_review(
            conflict.to_string(),
            detail.binding,
            meta(),
            OfflineReviewChoice::SwapOther,
            BUDGET,
            token(),
        )
        .await
        .unwrap();
    assert!(staged.details.progress == OfflineReviewProgress::Draft);
    assert_eq!(staged.project.host_seq, before.host_seq);
    assert_eq!(staged.project.next_lamport, 101);
    let submitted = project
        .submit_offline_review(
            conflict.to_string(),
            staged.details.binding,
            BUDGET,
            token(),
        )
        .await
        .unwrap();
    assert!(submitted.progress == OfflineReviewProgress::Applied);
    assert!(submitted.receipt.revision.can_undo);
    assert!(!submitted.receipt.duplicate);
    assert_eq!(submitted.receipt.revision.host_seq, before.host_seq + 1);
    let current = project
        .offline_review_details(conflict.to_string(), BUDGET, token())
        .await
        .unwrap();
    let retry = project
        .submit_offline_review(conflict.to_string(), current.binding, BUDGET, token())
        .await
        .unwrap();
    assert!(retry.receipt.duplicate);
    assert_eq!(
        retry.receipt.revision.host_seq,
        submitted.receipt.revision.host_seq
    );
    project.close().await.unwrap();
}

#[tokio::test]
async fn mirror_submit_adopts_existing_replica_history_and_wakes_existing_ops_notify() {
    let (temp, host, conflict) = fixture();
    let checkpoint = host
        .worker
        .call(|state| Ok(state.store()?.checkpoint_bytes()?))
        .await
        .unwrap();
    let path = temp.path().join("mirror");
    let store = ProjectStore::create_authenticated_checkpoint(
        &path,
        &checkpoint,
        &device(1),
        &device(2),
        200,
    )
    .unwrap();
    let mirror = ProjectSession::from_store(
        store,
        BlobStore::new(&path).unwrap(),
        SessionPermit::acquire().unwrap(),
    )
    .unwrap();
    let detail = mirror
        .offline_review_details(conflict.to_string(), BUDGET, token())
        .await
        .unwrap();
    let mut metadata = meta();
    metadata.device_id = device(2).to_string();
    let staged = mirror
        .stage_offline_review(
            conflict.to_string(),
            detail.binding,
            metadata,
            OfflineReviewChoice::SwapOther,
            BUDGET,
            token(),
        )
        .await
        .unwrap();
    let notified = mirror.network_changed.notified();
    let result = mirror
        .submit_offline_review(
            conflict.to_string(),
            staged.details.binding,
            BUDGET,
            token(),
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_millis(100), notified)
        .await
        .unwrap();
    assert!(result.progress == OfflineReviewProgress::Queued);
    assert_eq!(result.receipt.revision.host_seq, 2);
    assert!(result.receipt.revision.can_undo);
    mirror
        .worker
        .call(|state| {
            assert_eq!(state.store()?.pending()?.len(), 1);
            assert_eq!(state.replica.as_ref().unwrap().pending().len(), 1);
            assert_eq!(
                state.project()?.documents[&uid(2)].definition.title,
                "offline"
            );
            Ok(())
        })
        .await
        .unwrap();
    mirror.close().await.unwrap();
    host.close().await.unwrap();
}

#[tokio::test]
async fn cancellation_and_live_gesture_refuse_before_staging_without_losing_the_conflict() {
    let (_temp, project, conflict) = fixture();
    let detail = project
        .offline_review_details(conflict.to_string(), BUDGET, token())
        .await
        .unwrap();
    let cancelled = token();
    cancelled.cancel();
    assert!(matches!(
        project
            .stage_offline_review(
                conflict.to_string(),
                detail.binding.clone(),
                meta(),
                OfflineReviewChoice::SwapOther,
                BUDGET,
                cancelled
            )
            .await,
        Err(WorkflowError::Cancelled)
    ));
    project.active_gestures.store(1, Ordering::Release);
    assert!(matches!(
        project
            .stage_offline_review(
                conflict.to_string(),
                detail.binding,
                meta(),
                OfflineReviewChoice::SwapOther,
                BUDGET,
                token()
            )
            .await,
        Err(WorkflowError::Backpressure)
    ));
    project.active_gestures.store(0, Ordering::Release);
    let detail = project
        .offline_review_details(conflict.to_string(), BUDGET, token())
        .await
        .unwrap();
    assert!(detail.progress == OfflineReviewProgress::Unreviewed);
    assert!(detail.transaction_id.is_none());
    project.close().await.unwrap();
}

#[tokio::test]
async fn stale_visible_hash_is_refused_and_a_staged_review_is_recovered_without_new_identity() {
    let (temp, project, conflict) = fixture();
    let detail = project
        .offline_review_details(conflict.to_string(), BUDGET, token())
        .await
        .unwrap();
    let mut stale = detail.binding.clone();
    stale.state_hash = "0".repeat(64);
    assert!(matches!(
        project
            .stage_offline_review(
                conflict.to_string(),
                stale,
                meta(),
                OfflineReviewChoice::SwapOther,
                BUDGET,
                token()
            )
            .await,
        Err(WorkflowError::Stale)
    ));
    project
        .stage_offline_review(
            conflict.to_string(),
            detail.binding,
            meta(),
            OfflineReviewChoice::SwapOther,
            BUDGET,
            token(),
        )
        .await
        .unwrap();
    project.close().await.unwrap();
    drop(project);
    let path = temp.path().join("host");
    let reopened = ProjectSession::from_store(
        ProjectStore::open(&path).unwrap(),
        BlobStore::new(&path).unwrap(),
        SessionPermit::acquire().unwrap(),
    )
    .unwrap();
    // The root History::restore floor hook is separately required before other
    // ordinary edits. This read also repairs the same floor on its own worker.
    let draft = reopened
        .offline_review_details(conflict.to_string(), BUDGET, token())
        .await
        .unwrap();
    assert!(draft.progress == OfflineReviewProgress::Draft);
    assert_eq!(draft.transaction_id.unwrap(), uid(100).to_string());
    assert_eq!(reopened.info().await.unwrap().next_lamport, 101);
    reopened
        .submit_offline_review(conflict.to_string(), draft.binding, BUDGET, token())
        .await
        .unwrap();
    reopened.close().await.unwrap();
}

#[test]
fn private_preview_is_utf8_bounded_hashed_and_explicitly_incomplete() {
    let text = "€".repeat(PREVIEW_BYTES);
    let value = preview(text.as_bytes()).unwrap();
    assert!(value.truncated);
    assert!(value.preview.len() <= PREVIEW_BYTES);
    assert_eq!(value.utf8_bytes, text.len() as u64);
    assert_eq!(
        value.blake3.unwrap(),
        AssetId::hash(text.as_bytes()).to_string()
    );
}
