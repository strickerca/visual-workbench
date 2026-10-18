#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
use super::*;
use super::super::receiver_tests::{WORKERS,BUDGET,work,owned};
use std::{future::{Future,poll_fn},task::Poll,time::Duration};

#[tokio::test]
async fn cancelled_consumer_retains_actual_worker_and_all_bytes_until_decoder_returns(){
    let _guard=WORKERS.lock().await;let budget=ByteBudget::new(BUDGET).unwrap();let mut decoder=Decoder::new(budget.clone()).unwrap();
    let (entered,ready)=oneshot::channel();let (release,wait)=std::sync::mpsc::channel();
    let job=decoder.start_with(work(1),None,move |packet,previous,budget,cancel|{
        entered.send(()).unwrap();wait.recv_timeout(Duration::from_secs(5)).unwrap();
        vw_stream::decode_tiles(packet,previous,budget,cancel).map_err(map_stream)
    }).unwrap();
    tokio::time::timeout(Duration::from_secs(5),ready).await.unwrap().unwrap();let retained=budget.used();assert!(retained>0);
    drop(job);assert!(matches!(decoder.start(work(2),None),Err(SessionError::Backpressure)));assert_eq!(budget.used(),retained);
    let mut shutdown=Box::pin(decoder.shutdown());assert!(poll_fn(|cx|Poll::Ready(shutdown.as_mut().poll(cx).is_pending())).await);
    release.send(()).unwrap();tokio::time::timeout(Duration::from_secs(5),shutdown).await.unwrap().unwrap();assert_eq!(budget.used(),0);
}
#[tokio::test]
async fn interrupted_wait_keeps_exact_result_and_frame_credit_after_worker_shutdown(){
    let _guard=WORKERS.lock().await;let budget=ByteBudget::new(BUDGET).unwrap();let mut decoder=Decoder::new(budget.clone()).unwrap();
    let (entered,ready)=oneshot::channel();let (release,wait)=std::sync::mpsc::channel();
    let mut job=decoder.start_with(work(1),None,move |packet,previous,budget,cancel|{
        entered.send(()).unwrap();wait.recv_timeout(Duration::from_secs(5)).unwrap();
        vw_stream::decode_tiles(packet,previous,budget,cancel).map_err(map_stream)
    }).unwrap();
    tokio::time::timeout(Duration::from_secs(5),ready).await.unwrap().unwrap();
    {let mut waiting=Box::pin(job.receive());assert!(poll_fn(|cx|Poll::Ready(waiting.as_mut().poll(cx).is_pending())).await);}
    release.send(()).unwrap();let frame=tokio::time::timeout(Duration::from_secs(5),job.receive()).await.unwrap().unwrap();
    assert_eq!(frame.identity().frame_id,1);assert!(job.receive().await.is_err());decoder.shutdown().await.unwrap();
    assert_eq!(budget.used(),16*9*4+4096);drop(frame);assert_eq!(budget.used(),0);
}
#[tokio::test]
async fn foreign_budget_base_and_small_budget_refuse_without_consuming_codec_lifetime(){
    let _guard=WORKERS.lock().await;let other=ByteBudget::new(BUDGET).unwrap();let previous=owned(1,&other);
    let budget=ByteBudget::new(1024).unwrap();let mut decoder=Decoder::new(budget.clone()).unwrap();
    assert!(matches!(decoder.start(work(2),Some(previous)),Err(SessionError::Invalid)));
    assert!(matches!(decoder.start(work(1),None),Err(SessionError::Backpressure)));assert_eq!(budget.used(),0);
    decoder.shutdown().await.unwrap();
}
