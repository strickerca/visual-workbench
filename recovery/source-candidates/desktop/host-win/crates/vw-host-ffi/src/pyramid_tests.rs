use super::*;
use std::{sync::mpsc, time::Duration};

type TestResult=std::result::Result<(),Box<dyn std::error::Error>>;
struct Release(Option<mpsc::Sender<()>>);
impl Drop for Release { fn drop(&mut self){if let Some(tx)=self.0.take(){let _=tx.send(());}} }
#[tokio::test(flavor="current_thread")]
async fn dropping_receiver_and_last_builder_owner_keeps_process_wide_slot_until_worker_settles() -> TestResult {
    // These are the actual creation admission and actual retained std-worker
    // path, with a gated body instead of launching an OS codec/helper.
    let first=Lifetime::admitted()?;let second=Lifetime::admitted()?;
    let weak=Arc::downgrade(&first);
    let (entered_tx,entered_rx)=tokio::sync::oneshot::channel();
    let (tx,rx)=mpsc::channel();let release=Release(Some(tx));
    let worker_owner=first.clone();let cancel=Arc::new(assets::Cancellation::default());
    let token=cancel.clone();
    let future=tokio::spawn(async move { owned(worker_owner,token,move || {
        let _=entered_tx.send(());rx.recv_timeout(Duration::from_secs(5)).map_err(|_|refused("fixture"))?;Ok(())
    }).await });
    tokio::time::timeout(Duration::from_secs(3),entered_rx).await??;
    future.abort();let _=future.await;
    first.seal()?;drop(first); // Same final-owner seal as PyramidBuilder::drop.
    assert!(cancel.check().is_err());assert!(weak.upgrade().is_some());
    assert!(matches!(Lifetime::admitted(),Err(PyramidError::Refused{kind}) if kind=="Busy"));
    drop(release);
    tokio::time::timeout(Duration::from_secs(3),async { while weak.upgrade().is_some(){tokio::task::yield_now().await;} }).await?;
    // Weak expiry precedes the final Lifetime field destructors. Observe the
    // real capacity release instead of assuming Arc expiry credited the slot.
    let replacement=tokio::time::timeout(Duration::from_secs(3),async {
        loop { match Lifetime::admitted() {
            Ok(value)=>break Ok(value),
            Err(PyramidError::Refused{kind}) if kind=="Busy"=>tokio::task::yield_now().await,
            Err(error)=>break Err(error),
        } }
    }).await??;
    drop(replacement);drop(second);Ok(())
}
#[tokio::test(flavor="current_thread")]
async fn cancelled_receiver_does_not_release_actual_worker_capacity() -> TestResult {
    let life=Arc::new(Lifetime::default());let cancel=Arc::new(assets::Cancellation::default());
    let (entered_tx,entered_rx)=tokio::sync::oneshot::channel();let (tx,rx)=mpsc::channel();let release=Release(Some(tx));
    let owner=life.clone();let token=cancel.clone();
    let first=tokio::spawn(async move{owned(owner,token,move||{let _=entered_tx.send(());rx.recv_timeout(Duration::from_secs(5)).map_err(|_|refused("fixture"))?;Ok(())}).await});
    tokio::time::timeout(Duration::from_secs(3),entered_rx).await??;
    first.abort();let _=first.await;
    assert!(cancel.check().is_err());
    assert!(matches!(life.begin(Arc::new(assets::Cancellation::default())),Err(PyramidError::Refused{kind}) if kind=="Busy"));
    let a=life.clone();let b=life.clone();
    let one=tokio::spawn(async move{a.shutdown().await});let two=tokio::spawn(async move{b.shutdown().await});
    tokio::task::yield_now().await;assert!(!one.is_finished());assert!(!two.is_finished());
    drop(release);
    tokio::time::timeout(Duration::from_secs(3),one).await???;
    tokio::time::timeout(Duration::from_secs(3),two).await???;
    assert!(matches!(life.begin(Arc::new(assets::Cancellation::default())),Err(PyramidError::Refused{kind}) if kind=="Closed"));
    Ok(())
}
#[tokio::test(flavor="current_thread")]
async fn worker_captures_drop_before_shutdown_acknowledgment() -> TestResult {
    struct Capture(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Capture{fn drop(&mut self){self.0.store(true,Ordering::Release);}}
    let dropped=Arc::new(std::sync::atomic::AtomicBool::new(false));let capture=Capture(dropped.clone());
    let life=Arc::new(Lifetime::default());
    owned(life.clone(),Arc::new(assets::Cancellation::default()),move||{drop(capture);Ok(())}).await?;
    life.shutdown().await?;assert!(dropped.load(Ordering::Acquire));Ok(())
}
#[test]
fn request_limits_precede_worker_or_storage_admission() -> TestResult {
    let path=if cfg!(windows){"C:/private/input"}else{"/private/input"};
    let mut value=PyramidRequest{source_path:path.into(),destination:format!("{path}-pyramid"),source_asset_id:"a".repeat(64),
        original_bytes:1024,width:16320,height:12240,orientation:1,disk_budget:4*1024*1024*1024,timeout_ms:60_000};
    assert!(request(&value).is_ok());
    value.timeout_ms=600_001;assert!(request(&value).is_err());value.timeout_ms=60_000;
    value.disk_budget=1024;assert!(request(&value).is_err());value.disk_budget=4*1024*1024*1024;
    value.source_asset_id="G".repeat(64);assert!(request(&value).is_err());
    assert!(local_path("relative/input").is_err());assert!(local_path("bad\0path").is_err());
    #[cfg(windows)] assert!(local_path("\\\\remote\\share\\image.jpg").is_err());
    Ok(())
}
