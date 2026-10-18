#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
use super::*;
use super::super::{hub::Hub,api::CaptureStream,receiver_tests::{WORKERS,BUDGET,source,packet,id}};
use std::{future::poll_fn,task::Poll};

async fn step(bridge:&mut Bridge){
    let event=tokio::time::timeout(Duration::from_secs(5),bridge.next()).await.unwrap();
    bridge.handle(event,0).unwrap();
}
async fn dispatch<F:Future>(bridge:&mut Bridge,mut future:Pin<&mut F>)->F::Output{
    for _ in 0..24{
        if let Poll::Ready(value)=poll_fn(|cx|Poll::Ready(future.as_mut().poll(cx))).await{return value}
        step(bridge).await;
    }panic!("bounded command events exhausted")
}
async fn open(hub:&Arc<Hub>,bridge:&mut Bridge)->Arc<CaptureStream>{
    let mut open=Box::pin(hub.open(bridge.endpoint.epoch,BUDGET));
    for _ in 0..16{if let Poll::Ready(value)=poll_fn(|cx|Poll::Ready(open.as_mut().poll(cx))).await{return value.unwrap()}step(bridge).await;}
    panic!("open did not settle within bounded fixture events")
}
fn connection(hub:&Hub,epoch:u64)->Bridge{let(endpoint,receive)=hub.activate(epoch).unwrap();Bridge::new(endpoint,receive)}
async fn decoded(bridge:&mut Bridge,n:u64){
    bridge.media(packet(n,1)).unwrap();bridge.maintenance(0).unwrap();
    while bridge.receiver.as_ref().unwrap().has_decode(){step(bridge).await;}
}
async fn draw(view:&CaptureStream,bridge:&mut Bridge)->Arc<CaptureDraw>{
    let mut wait=Box::pin(view.begin_draw());assert!(poll_fn(|cx|Poll::Ready(wait.as_mut().poll(cx).is_pending())).await);
    step(bridge).await;wait.await.unwrap().unwrap()
}
async fn close_view(view:&CaptureStream,bridge:&mut Bridge){
    let mut close=Box::pin(view.close_stream());assert!(poll_fn(|cx|Poll::Ready(close.as_mut().poll(cx).is_pending())).await);
    bridge.maintenance(0).unwrap();while bridge.retiring.is_some(){step(bridge).await;}
    tokio::time::timeout(Duration::from_secs(5),close).await.unwrap().unwrap();
}

#[tokio::test]
async fn old_entry_epoch_cannot_enable_a_receiver_after_reconnect(){
    let _guard=WORKERS.lock().await;let hub=Hub::new();let mut first=connection(&hub,1);
    first.shutdown().await.unwrap();let mut next=connection(&hub,3);
    assert!(matches!(hub.open(1,BUDGET).await,Err(SessionError::Closed)));
    assert!(next.commands.try_recv().is_err());let view=open(&hub,&mut next).await;
    close_view(&view,&mut next).await;next.shutdown().await.unwrap();hub.close().await.unwrap();
}
#[tokio::test]
async fn cancelled_enable_before_dispatch_does_not_create_a_worker_or_seal_the_link(){
    let _guard=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);
    let mut opening=Box::pin(hub.open(1,BUDGET));assert!(poll_fn(|cx|Poll::Ready(opening.as_mut().poll(cx).is_pending())).await);
    drop(opening);while !bridge.commands.is_empty(){step(&mut bridge).await;}
    assert!(bridge.receiver.is_none());assert!(!bridge.is_stopped());
    let view=open(&hub,&mut bridge).await;close_view(&view,&mut bridge).await;bridge.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancelled_enable_after_creation_keeps_receiver_until_actual_retirement(){
    let _guard=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);
    let mut opening=Box::pin(hub.open(1,BUDGET));assert!(poll_fn(|cx|Poll::Ready(opening.as_mut().poll(cx).is_pending())).await);
    step(&mut bridge).await;assert!(bridge.receiver.is_some());drop(opening);
    bridge.maintenance(0).unwrap();assert!(bridge.receiver.is_none());assert!(bridge.retiring.is_some());
    while bridge.retiring.is_some(){step(&mut bridge).await;}
    let view=open(&hub,&mut bridge).await;close_view(&view,&mut bridge).await;bridge.shutdown().await.unwrap();
}
#[tokio::test]
async fn undelivered_draw_seals_its_view_without_claiming_it_was_not_presented(){
    let _guard=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);bridge.control(source(1).control("start")).unwrap();
    let view=open(&hub,&mut bridge).await;decoded(&mut bridge,1).await;
    let mut drawing=Box::pin(view.begin_draw());assert!(poll_fn(|cx|Poll::Ready(drawing.as_mut().poll(cx).is_pending())).await);
    step(&mut bridge).await;drop(drawing);
    assert!(view.lease.closing.load(Ordering::Acquire));
    assert!(matches!(view.first_contact(id(8).to_string()).await,Err(SessionError::Closed)));
    close_view(&view,&mut bridge).await;assert!(!bridge.is_stopped());bridge.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancelled_receipt_waiter_rejoins_exact_choice_and_contact_freezes_actual_frame(){
    let _guard=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);bridge.control(source(1).control("start")).unwrap();
    let view=open(&hub,&mut bridge).await;decoded(&mut bridge,1).await;let shown=draw(&view,&mut bridge).await;
    assert_eq!(shown.rgba_rows(0,9).unwrap().len(),16*9*4);assert!(shown.rgba_rows(8,2).is_err());
    let mut receipt=Box::pin(shown.presented(true));assert!(poll_fn(|cx|Poll::Ready(receipt.as_mut().poll(cx).is_pending())).await);drop(receipt);
    step(&mut bridge).await;shown.presented(true).await.unwrap();assert!(shown.presented(false).await.is_err());
    decoded(&mut bridge,2).await;
    let mut contact=Box::pin(view.first_contact(id(8).to_string()));assert!(poll_fn(|cx|Poll::Ready(contact.as_mut().poll(cx).is_pending())).await);
    step(&mut bridge).await;let receipt=contact.await.unwrap().unwrap();assert_eq!(receipt.frame_id,1);
    assert_eq!(bridge.pending_control.as_ref().unwrap().action,"pause");drop(shown);
    close_view(&view,&mut bridge).await;bridge.shutdown().await.unwrap();
}
#[tokio::test]
async fn equal_size_new_frames_publish_readiness_and_close_wakes_a_cancelled_ui_wait(){
    let _guard=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);bridge.control(source(1).control("start")).unwrap();
    let view=open(&hub,&mut bridge).await;decoded(&mut bridge,1).await;let first=view.signal();
    decoded(&mut bridge,2).await;let second=view.signal();assert_eq!(first.retained_bytes,second.retained_bytes);
    assert!(second.sequence>first.sequence);assert!(second.frame_sequence>first.frame_sequence);
    let mut waiting=Box::pin(view.wait_signal(second.sequence));assert!(poll_fn(|cx|Poll::Ready(waiting.as_mut().poll(cx).is_pending())).await);
    view.lease.stop();assert!(matches!(waiting.await,Err(SessionError::Closed)));
    close_view(&view,&mut bridge).await;bridge.shutdown().await.unwrap();
}
#[tokio::test]
async fn link_close_seals_immediately_and_waits_for_driver_join(){
    let _guard=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);let view=open(&hub,&mut bridge).await;
    let mut closing=Box::pin(hub.close());assert!(poll_fn(|cx|Poll::Ready(closing.as_mut().poll(cx).is_pending())).await);
    assert!(bridge.is_stopped());assert!(matches!(view.begin_draw().await,Err(SessionError::Closed)));
    bridge.shutdown().await.unwrap();closing.await.unwrap();view.close_stream().await.unwrap();
}
#[tokio::test]
async fn forced_driver_drop_reports_failure_to_waiters_instead_of_false_join_success(){
    let _guard=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);
    // No codec owner needed: test the actual completion fallback independently.
    let mut opening=Box::pin(hub.open(1,BUDGET));assert!(poll_fn(|cx|Poll::Ready(opening.as_mut().poll(cx).is_pending())).await);
    let command=bridge.commands.recv().await.unwrap();drop(command);assert!(opening.await.is_err());
    drop(bridge);assert!(matches!(hub.close().await,Err(SessionError::Worker)));
}

#[tokio::test]
async fn actual_scoped_transfer_upgrades_only_the_exact_frozen_background_and_old_reads_remain_honest(){
    use std::io::{Read,Seek,SeekFrom};
    use super::super::verified_tests::{root,fixture};
    let _guard=WORKERS.lock().await;let root=root();let hub=Hub::new();let mut bridge=connection(&hub,1);
    bridge.control(source(1).control("start")).unwrap();let view=open(&hub,&mut bridge).await;
    decoded(&mut bridge,1).await;let shown=draw(&view,&mut bridge).await;
    let mut receipt=Box::pin(shown.presented(true));assert!(poll_fn(|cx|Poll::Ready(receipt.as_mut().poll(cx).is_pending())).await);
    step(&mut bridge).await;receipt.await.unwrap();drop(shown);
    let mut contact=Box::pin(view.first_contact(id(8).to_string()));assert!(poll_fn(|cx|Poll::Ready(contact.as_mut().poll(cx).is_pending())).await);
    step(&mut bridge).await;contact.await.unwrap().unwrap();
    let mut background=Box::pin(view.frozen_background());let compressed=dispatch(&mut bridge,background.as_mut()).await.unwrap().unwrap();assert!(!compressed.is_lossless());assert!(compressed.original_asset_id().is_none());
    let compressed_bytes=compressed.rgba_rows(0,9).unwrap();
    let(offer,mut file)=fixture(root.path());let path=file.path().to_path_buf();file.seek(SeekFrom::Start(0)).unwrap();let mut bytes=Vec::new();file.read_to_end(&mut bytes).unwrap();
    file.as_file_mut().set_len(0).unwrap();file.seek(SeekFrom::Start(0)).unwrap();
    let flow=bridge.flow.as_mut().unwrap();assert!(matches!(flow.offer(&offer.wire()).unwrap(),Offer::Create(_)));flow.install(offer.clone(),file).unwrap();
    bridge.stream_blob(offer.scope.envelope(pb::stream_blob::Body::Chunk(pb::BlobChunk{asset_id:offer.asset.to_string(),offset:0,total_size:offer.bytes,last:true,data:bytes}))).unwrap();
    bridge.maintenance(2000).unwrap();bridge.publish();assert_eq!(view.signal().stage,StreamStage::FrozenDegraded);
    for _ in 0..8{if bridge.original.is_some(){break}step(&mut bridge).await;}
    assert_eq!(view.signal().stage,StreamStage::FrozenLossless);
    let mut background=Box::pin(view.frozen_background());let lossless=dispatch(&mut bridge,background.as_mut()).await.unwrap().unwrap();
    assert!(lossless.is_lossless());assert_eq!(lossless.info(),compressed.info());assert_eq!(lossless.draft_id(),id(8).to_string());
    assert_eq!(lossless.original_asset_id(),Some(offer.asset.to_string()));assert_eq!(&lossless.rgba_rows(0,1).unwrap()[..4],&[40,80,120,255]);
    assert!(!compressed.is_lossless());assert_eq!(compressed.rgba_rows(0,9).unwrap(),compressed_bytes);
    bridge.stream_blob(offer.scope.envelope(pb::stream_blob::Body::Cancel(pb::StreamTransferCancel{}))).unwrap();assert_eq!(view.signal().stage,StreamStage::FrozenLossless);
    let mut resume=Box::pin(view.resume_after_draft_disposed());assert_eq!(dispatch(&mut bridge,resume.as_mut()).await.unwrap(),Some(id(8).to_string()));assert!(path.exists());
    close_view(&view,&mut bridge).await;assert!(matches!(lossless.rgba_rows(0,1),Err(SessionError::Closed)));
    assert!(path.exists());drop(lossless);assert!(!path.exists());drop(compressed);bridge.shutdown().await.unwrap();
}
