#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
use super::*;
use super::super::receiver_tests::{WORKERS,BUDGET,packet};
use std::{future::{Future,poll_fn},task::Poll,time::Duration};
use vw_proto::Message;
fn encoded()->Vec<u8>{let mut frame=packet(1,1);frame.source_generation=0;frame.delivery_sequence=0;frame.encode_to_vec()}

#[tokio::test]
async fn dropped_waiter_retains_actual_input_and_worker_until_held_preparation_returns(){
    let _gate=WORKERS.lock().await;let budget=ByteBudget::new(BUDGET).unwrap();let mut worker=PublishWorker::new(budget.clone()).unwrap();
    let(entered,ready)=oneshot::channel();let(release,wait)=std::sync::mpsc::channel();
    let job=worker.start(encoded(),vw_proto::capture_admission::MAX_MEDIA_BYTES,move|bytes,budget,cancel|{
        entered.send(()).unwrap();wait.recv_timeout(Duration::from_secs(5)).unwrap();cancel.check().map_err(super::super::decoder::map_stream)?;
        PreparedFrame::decode(bytes,budget).map(Prepared::Frame)
    }).unwrap();
    tokio::time::timeout(Duration::from_secs(5),ready).await.unwrap().unwrap();let used=budget.used();assert!(used>0);drop(job);
    assert!(matches!(worker.frame(encoded()),Err(SessionError::Backpressure)));
    let mut shutdown=Box::pin(worker.shutdown());assert!(poll_fn(|cx|Poll::Ready(shutdown.as_mut().poll(cx).is_pending())).await);assert_eq!(budget.used(),used);
    release.send(()).unwrap();tokio::time::timeout(Duration::from_secs(5),shutdown).await.unwrap().unwrap();assert_eq!(budget.used(),0);
}
#[tokio::test]
async fn interrupted_select_can_rejoin_exact_preparation_and_escaped_result_keeps_its_charge(){
    let _gate=WORKERS.lock().await;let budget=ByteBudget::new(BUDGET).unwrap();let mut worker=PublishWorker::new(budget.clone()).unwrap();
    let(entered,ready)=oneshot::channel();let(release,wait)=std::sync::mpsc::channel();
    let mut job=worker.start(encoded(),vw_proto::capture_admission::MAX_MEDIA_BYTES,move|bytes,budget,_|{
        entered.send(()).unwrap();wait.recv_timeout(Duration::from_secs(5)).unwrap();PreparedFrame::decode(bytes,budget).map(Prepared::Frame)
    }).unwrap();
    ready.await.unwrap();{let mut waiting=Box::pin(job.receive());assert!(poll_fn(|cx|Poll::Ready(waiting.as_mut().poll(cx).is_pending())).await);}
    release.send(()).unwrap();let result=tokio::time::timeout(Duration::from_secs(5),job.receive()).await.unwrap().unwrap();assert!(matches!(&result,Prepared::Frame(_)));
    worker.shutdown().await.unwrap();assert!(budget.used()>0);assert!(job.receive().await.is_err());drop(result);assert_eq!(budget.used(),0);
}
