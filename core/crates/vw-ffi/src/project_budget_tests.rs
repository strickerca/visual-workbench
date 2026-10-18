//! Publication adoption regressions. These exercise the real project worker and
//! SQLite writer rather than treating a dropped foreign handle as retirement.
#![allow(clippy::unwrap_used)]
use crate::project::*;
use std::{
    future::{Future, poll_fn},
    task::Poll,
};
use vw_stream::{MemoryKind, SessionBudget};
const BYTES: u64 = 32 * 1024 * 1024;
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn input(path: &Path, budget: &SessionBudget, bytes: u64) -> StoreInput {
    let device = DeviceId::from_bytes([73; 16]);
    let model = Project::new(
        Id::from_parts(1_700_000_000_000, [74; 10]).unwrap(),
        "published budget fixture".into(),
        device.clone(),
    );
    StoreInput {
        store: ProjectStore::create(path, model, device, 0).unwrap(),
        blobs: BlobStore::new(path).unwrap(),
        permit: SessionPermit::acquire().unwrap(),
        charge: Some(budget.reserve(MemoryKind::CodecWorkspace, bytes).unwrap()),
    }
}
fn adopt(input: StoreInput) -> Result<Arc<ProjectSession>> {
    let StoreInput {
        store,
        blobs,
        permit,
        charge,
    } = input;
    ProjectSession::from_store_budgeted(store, blobs, permit, charge.unwrap())
}
#[tokio::test]
async fn rejected_constructor_returns_charge_but_preserves_the_published_destination() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("published");
    let budget = SessionBudget::new(BYTES).unwrap();
    assert!(matches!(
        adopt(input(&path, &budget, BYTES - 1)),
        Err(CoreError::Invalid)
    ));
    assert_eq!(budget.used_bytes(), 0);
    assert!(path.is_dir());
    ProjectStore::open(&path)
        .unwrap()
        .integrity_check()
        .unwrap();
}
#[tokio::test]
async fn cancelled_close_keeps_publication_charge_until_actual_project_state_destruction() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("published");
    let budget = SessionBudget::new(BYTES).unwrap();
    let project = adopt(input(&path, &budget, BYTES)).unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    project
        .worker
        .enqueue(Box::new(move |_| {
            let _ = started.send(());
            wait.recv().unwrap();
        }))
        .unwrap();
    ready.await.unwrap();
    let mut close = Box::pin(project.close());
    assert!(poll_fn(|cx| Poll::Ready(close.as_mut().poll(cx).is_pending())).await);
    drop(close);
    assert_eq!(budget.used_bytes(), BYTES);
    release.send(()).unwrap();
    project.close().await.unwrap();
    assert_eq!(budget.used_bytes(), 0);
    // The facade remains alive, but its actual state and writer have retired.
    ProjectStore::open(&path)
        .unwrap()
        .integrity_check()
        .unwrap();
}
#[tokio::test]
async fn abandoned_startup_reply_never_drops_charge_before_its_owned_adoption_settles() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("published");
    let budget = SessionBudget::new(BYTES).unwrap();
    let input = input(&path, &budget, BYTES);
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let mut startup = Box::pin(crate::worker::startup(move || {
        let _ = started.send(());
        wait.recv().unwrap();
        adopt(input)
    }));
    assert!(poll_fn(|cx| Poll::Ready(startup.as_mut().poll(cx).is_pending())).await);
    ready.await.unwrap();
    drop(startup);
    assert_eq!(budget.used_bytes(), BYTES);
    release.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while budget.used_bytes() != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    ProjectStore::open(&path)
        .unwrap()
        .integrity_check()
        .unwrap();
}
