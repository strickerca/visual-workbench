#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
use super::*;
use super::super::{receiver_tests::{WORKERS,BUDGET,id,identity,stamp},wire::Scope};
use std::{future::{Future,poll_fn},io::{Read,Seek,SeekFrom},task::Poll,time::Duration};
fn root()->tempfile::TempDir{if cfg!(target_os="android"){tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()}else{tempfile::tempdir().unwrap()}}
fn fixture(root:&std::path::Path)->(OriginalOffer,tempfile::NamedTempFile,pb::StreamBlob){
    let bytes=b"exact scoped original bytes".to_vec();let offer=OriginalOffer{
        scope:Scope{session:id(1),frame:1,geometry_revision:1,request:id(8),transfer:id(9),generation:1},
        frame:identity(1),asset:vw_model::AssetId::hash(&bytes),bytes:bytes.len()as u64,stamp:stamp(1)};
    let chunk=offer.scope.envelope(pb::stream_blob::Body::Chunk(pb::BlobChunk{asset_id:offer.asset.to_string(),offset:0,total_size:offer.bytes,last:true,data:bytes}));
    (offer,tempfile::NamedTempFile::new_in(root).unwrap(),chunk)
}
#[tokio::test]
async fn actual_hash_verified_file_survives_io_shutdown_only_while_result_owner_retains_it(){
    let _gate=WORKERS.lock().await;let root=root();let(offer,file,chunk)=fixture(root.path());let path=file.path().to_path_buf();let budget=ByteBudget::new(BUDGET).unwrap();
    let mut incoming=Incoming::new(offer.clone(),file,budget.clone()).unwrap();let mut job=incoming.start(chunk).unwrap();let result=job.receive().await.unwrap();
    let pb::stream_blob::Body::Ack(ack)=result.ack.body.unwrap()else{panic!("wrong receipt")};assert!(ack.verified);assert_eq!(ack.next_offset,offer.bytes);
    incoming.shutdown().await.unwrap();assert_eq!(budget.used(),0);let(_,mut file)=result.original.unwrap();assert!(path.exists());
    file.seek(SeekFrom::Start(0)).unwrap();let mut data=Vec::new();file.read_to_end(&mut data).unwrap();assert_eq!(vw_model::AssetId::hash(&data),offer.asset);
    drop(file);assert!(!path.exists());
}
#[tokio::test]
async fn cancelled_consumer_keeps_actual_writer_and_scratch_until_join(){
    let _gate=WORKERS.lock().await;let root=root();let(offer,file,chunk)=fixture(root.path());let path=file.path().to_path_buf();let budget=ByteBudget::new(BUDGET).unwrap();
    let mut incoming=Incoming::new(offer,file,budget.clone()).unwrap();let(entered,ready)=oneshot::channel();let(release,wait)=std::sync::mpsc::channel();
    let job=incoming.start_with(chunk.clone(),move||{entered.send(()).unwrap();wait.recv_timeout(Duration::from_secs(5)).unwrap();Ok(())}).unwrap();
    tokio::time::timeout(Duration::from_secs(5),ready).await.unwrap().unwrap();drop(job);let retained=budget.used();assert!(retained>8192);
    assert!(matches!(incoming.start(chunk),Err(SessionError::Backpressure)));assert!(path.exists());
    let mut close=Box::pin(incoming.shutdown());assert!(poll_fn(|cx|Poll::Ready(close.as_mut().poll(cx).is_pending())).await);assert_eq!(budget.used(),retained);
    release.send(()).unwrap();tokio::time::timeout(Duration::from_secs(5),close).await.unwrap().unwrap();assert_eq!(budget.used(),0);assert!(!path.exists());
}
#[tokio::test]
async fn interrupted_select_wait_retains_the_single_complete_file_result(){
    let _gate=WORKERS.lock().await;let root=root();let(offer,file,chunk)=fixture(root.path());let budget=ByteBudget::new(BUDGET).unwrap();let mut incoming=Incoming::new(offer,file,budget.clone()).unwrap();
    let(entered,ready)=oneshot::channel();let(release,wait)=std::sync::mpsc::channel();let mut job=incoming.start_with(chunk,move||{entered.send(()).unwrap();wait.recv_timeout(Duration::from_secs(5)).unwrap();Ok(())}).unwrap();
    ready.await.unwrap();{let mut receiving=Box::pin(job.receive());assert!(poll_fn(|cx|Poll::Ready(receiving.as_mut().poll(cx).is_pending())).await);}
    release.send(()).unwrap();let result=tokio::time::timeout(Duration::from_secs(5),job.receive()).await.unwrap().unwrap();assert!(result.original.is_some());assert!(job.receive().await.is_err());
    incoming.shutdown().await.unwrap();drop(result);assert_eq!(budget.used(),0);
}
#[tokio::test]
async fn wrong_scope_and_hidden_chunk_capacity_refuse_before_worker_admission(){
    let _gate=WORKERS.lock().await;let root=root();let(offer,file,mut chunk)=fixture(root.path());let budget=ByteBudget::new(BUDGET).unwrap();let mut incoming=Incoming::new(offer,file,budget.clone()).unwrap();
    chunk.key.as_mut().unwrap().generation=2;assert!(matches!(incoming.start(chunk.clone()),Err(SessionError::Invalid)));assert_eq!(budget.used(),8192);
    chunk.key.as_mut().unwrap().generation=1;let pb::stream_blob::Body::Chunk(value)=chunk.body.as_mut().unwrap()else{panic!("wrong body")};let mut oversized=Vec::with_capacity(1024*1024);oversized.extend_from_slice(&value.data);value.data=oversized;
    assert!(matches!(incoming.start(chunk),Err(SessionError::Backpressure)));assert_eq!(budget.used(),8192);incoming.shutdown().await.unwrap();assert_eq!(budget.used(),0);
}
