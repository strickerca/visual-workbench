#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
use super::*;
use super::super::{hub::Hub,publication_api::{CapturePublisher,CapturePublishSource},
    receiver_tests::{WORKERS,BUDGET,id,identity,packet,source,stamp},verified_tests::{fixture,root}};
use std::{future::poll_fn,io::{Read,Seek,SeekFrom},task::Poll};
use vw_proto::Message;

fn approved()->CapturePublishSource{
    let source=source(1);let geometry=source.geometry.wire();let rect=geometry.client_rect_host.as_ref().unwrap();
    CapturePublishSource{explicit_owner_action:true,capture_session_id:source.session.to_string(),source_kind:geometry.source_kind.clone(),
        window_handle:geometry.window_handle,monitor_id:geometry.monitor_id.clone(),host_x:rect.x,host_y:rect.y,width:source.geometry.width(),height:source.geometry.height(),
        dpi_scale:geometry.dpi_scale,geometry_revision:geometry.geometry_revision,observed_timestamp_ns:geometry.timestamp_ns,target_fps:source.fps,interaction:source.interaction,platform:source.platform}
}
fn encoded(n:u64)->Vec<u8>{let mut frame=packet(n,1);frame.source_generation=0;frame.delivery_sequence=0;frame.encode_to_vec()}
fn freeze(generation:u64)->pb::FreezeRequest{pb::FreezeRequest{capture_session_id:Some(id(1).to_proto()),frame_id:1,request_id:Some(id(8).to_proto()),geometry_revision:1,source_generation:generation}}
fn connection(hub:&Hub,epoch:u64)->Bridge{let(endpoint,commands)=hub.activate(epoch).unwrap();Bridge::new(endpoint,commands)}
async fn step(bridge:&mut Bridge){let event=tokio::time::timeout(Duration::from_secs(5),bridge.next()).await.unwrap();bridge.handle(event,0).unwrap();}
fn flush(bridge:&mut Bridge,output:&mut Vec<Body>){if let Some(publisher)=&mut bridge.publisher{publisher.flush(|body|{output.push(body);Ok(())}).unwrap();}}
async fn drive<F:Future>(bridge:&mut Bridge,mut future:Pin<&mut F>,output:&mut Vec<Body>)->F::Output{
    for _ in 0..24{
        if let Poll::Ready(value)=poll_fn(|cx|Poll::Ready(future.as_mut().poll(cx))).await{return value}
        step(bridge).await;flush(bridge,output);
    }panic!("publication fixture exceeded bounded events")
}
async fn open(hub:&Hub,bridge:&mut Bridge)->Arc<CapturePublisher>{
    let mut opening=Box::pin(hub.publish(bridge.endpoint.epoch,approved(),BUDGET));drive(bridge,opening.as_mut(),&mut Vec::new()).await.unwrap()
}
async fn close(publisher:&CapturePublisher,bridge:&mut Bridge){
    let mut closing=Box::pin(publisher.close_publisher());assert!(poll_fn(|cx|Poll::Ready(closing.as_mut().poll(cx).is_pending())).await);
    bridge.maintenance(0).unwrap();while bridge.publisher_retiring.is_some(){step(bridge).await;}
    tokio::time::timeout(Duration::from_secs(5),closing).await.unwrap().unwrap();
}
fn original()->(crate::CaptureImportDescriptor,Vec<u8>){
    let root=root();let(offer,mut file)=fixture(root.path());let geometry=offer.frame.geometry.wire();let rect=geometry.client_rect_host.as_ref().unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();let mut bytes=Vec::new();file.read_to_end(&mut bytes).unwrap();
    (crate::CaptureImportDescriptor{explicit_owner_action:true,capture_session_id:offer.frame.session.to_string(),frame_id:offer.frame.frame_id,
        geometry_revision:geometry.geometry_revision,platform:offer.stamp.platform,source_kind:geometry.source_kind.clone(),physical_x:rect.x,physical_y:rect.y,
        width:offer.frame.geometry.width(),height:offer.frame.geometry.height(),dpi_scale:geometry.dpi_scale,monotonic_timestamp_ns:offer.frame.timestamp_ns as u64,
        captured_at_ms:offer.stamp.captured_at_ms,window_handle:geometry.window_handle,monitor_id:geometry.monitor_id.clone(),expected_source_asset_id:Some(offer.asset.to_string())},bytes)
}

#[tokio::test]
async fn remote_source_or_freeze_never_creates_a_local_publisher_without_explicit_admission(){
    let _gate=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);
    bridge.control(source(1).control("start")).unwrap();bridge.freeze_request(freeze(1),0).unwrap();assert!(bridge.publisher.is_none());
    let mut value=approved();value.explicit_owner_action=false;assert!(matches!(hub.publish(1,value,BUDGET).await,Err(SessionError::Declined)));
    assert!(bridge.commands.is_empty());let mut opening=Box::pin(hub.publish(1,approved(),1024));
    assert!(matches!(drive(&mut bridge,opening.as_mut(),&mut Vec::new()).await,Err(SessionError::Backpressure)));
    assert!(bridge.publisher.is_none());assert!(!bridge.is_stopped());let publisher=open(&hub,&mut bridge).await;close(&publisher,&mut bridge).await;bridge.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancelled_publisher_acquisition_before_or_after_dispatch_is_joined_and_old_epoch_refused(){
    let _gate=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);
    let mut opening=Box::pin(hub.publish(1,approved(),BUDGET));assert!(poll_fn(|cx|Poll::Ready(opening.as_mut().poll(cx).is_pending())).await);drop(opening);
    step(&mut bridge).await;assert!(bridge.publisher.is_none());
    let mut opening=Box::pin(hub.publish(1,approved(),BUDGET));assert!(poll_fn(|cx|Poll::Ready(opening.as_mut().poll(cx).is_pending())).await);
    while bridge.publisher.is_none(){step(&mut bridge).await;}drop(opening);bridge.maintenance(0).unwrap();assert!(bridge.publisher_retiring.is_some());
    while bridge.publisher_retiring.is_some(){step(&mut bridge).await;}assert!(!bridge.is_stopped());bridge.shutdown().await.unwrap();
    let mut next=connection(&hub,3);assert!(matches!(hub.publish(1,approved(),BUDGET).await,Err(SessionError::Closed)));assert!(next.commands.is_empty());
    let publisher=open(&hub,&mut next).await;close(&publisher,&mut next).await;next.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancelled_frame_waiter_cannot_release_actual_input_admission_or_publish_late_pixels(){
    let _gate=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);let publisher=open(&hub,&mut bridge).await;
    let mut waiting=Box::pin(publisher.publish_frame(encoded(1)));assert!(poll_fn(|cx|Poll::Ready(waiting.as_mut().poll(cx).is_pending())).await);
    while !bridge.publisher.as_ref().unwrap().has_work(){step(&mut bridge).await;}drop(waiting);
    assert!(matches!(publisher.publish_frame(encoded(2)).await,Err(SessionError::Backpressure)));
    while bridge.publisher.as_ref().unwrap().has_work(){step(&mut bridge).await;}
    let mut output=Vec::new();flush(&mut bridge,&mut output);assert!(output.iter().all(|v|!matches!(v,Body::FrameTiles(_))));
    let mut valid=Box::pin(publisher.publish_frame(encoded(2)));assert!(drive(&mut bridge,valid.as_mut(),&mut output).await.unwrap());
    assert!(output.iter().any(|v|matches!(v,Body::FrameTiles(value)if value.frame_id==2)));
    close(&publisher,&mut bridge).await;bridge.shutdown().await.unwrap();
}
#[tokio::test]
async fn public_frame_feedback_binds_generation_and_exact_original_descriptor_before_any_offer(){
    let _gate=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);let publisher=open(&hub,&mut bridge).await;let mut output=Vec::new();
    let mut invalid=Box::pin(publisher.publish_frame(packet(1,1).encode_to_vec()));assert!(matches!(drive(&mut bridge,invalid.as_mut(),&mut output).await,Err(SessionError::Invalid)));
    let mut frame=Box::pin(publisher.publish_frame(encoded(1)));assert!(drive(&mut bridge,frame.as_mut(),&mut output).await.unwrap());
    let generation=publisher.signal().source_generation;assert!(output.iter().any(|v|matches!(v,Body::FrameTiles(v)if v.source_generation==generation&&v.frame_id==1)));
    bridge.freeze_request(freeze(generation),0).unwrap();let ticket=publisher.signal().original.unwrap();assert_eq!(ticket.frame.captured_at_ms,stamp(1).captured_at_ms);
    let(descriptor,bytes)=original();let mut altered=descriptor.clone();altered.captured_at_ms+=1;
    let mut wrong=Box::pin(publisher.provide_original(ticket.sequence,altered,bytes.clone()));assert!(matches!(drive(&mut bridge,wrong.as_mut(),&mut output).await,Err(SessionError::Invalid)));
    assert!(!output.iter().any(|v|matches!(v,Body::LosslessFrame(v)if v.available)));
    let mut exact=Box::pin(publisher.provide_original(ticket.sequence,descriptor,bytes.clone()));drive(&mut bridge,exact.as_mut(),&mut output).await.unwrap();flush(&mut bridge,&mut output);
    let offer=output.iter().find_map(|v|match v{Body::LosslessFrame(v)if v.available=>super::super::wire::OriginalOffer::parse(v,&identity(1),&stamp(1),&id(8),generation).unwrap(),_=>None}).unwrap();
    assert_eq!(offer.asset,vw_model::AssetId::hash(&bytes));assert_eq!(offer.scope.generation,generation);bridge.stream_blob(offer.request()).unwrap();output.clear();flush(&mut bridge,&mut output);
    let chunk=output.iter().find_map(|v|match v{Body::StreamBlob(v)=>Some(v),_=>None}).unwrap();
    let pb::stream_blob::Body::Chunk(chunk)=chunk.body.as_ref().unwrap()else{panic!("missing original chunk")};assert_eq!(chunk.data,bytes);
    let ack=offer.scope.envelope(pb::stream_blob::Body::Ack(pb::BlobAck{asset_id:offer.asset.to_string(),next_offset:offer.bytes,verified:true}));bridge.stream_blob(ack.clone()).unwrap();bridge.stream_blob(ack).unwrap();
    bridge.stream_blob(offer.scope.envelope(pb::stream_blob::Body::Ack(pb::BlobAck{asset_id:offer.asset.to_string(),next_offset:0,verified:false}))).unwrap();
    bridge.stream_blob(offer.scope.envelope(pb::stream_blob::Body::Cancel(pb::StreamTransferCancel{}))).unwrap();assert_eq!(publisher.signal().original.unwrap().sequence,ticket.sequence);
    close(&publisher,&mut bridge).await;bridge.shutdown().await.unwrap();
}
#[tokio::test]
async fn original_completion_after_resume_cannot_offer_a_retired_ticket(){
    let _gate=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);let publisher=open(&hub,&mut bridge).await;let mut output=Vec::new();
    let mut frame=Box::pin(publisher.publish_frame(encoded(1)));drive(&mut bridge,frame.as_mut(),&mut output).await.unwrap();
    bridge.freeze_request(freeze(publisher.signal().source_generation),0).unwrap();let ticket=publisher.signal().original.unwrap();let(identity,bytes)=original();
    let mut providing=Box::pin(publisher.provide_original(ticket.sequence,identity,bytes));assert!(poll_fn(|cx|Poll::Ready(providing.as_mut().poll(cx).is_pending())).await);
    while !bridge.publisher.as_ref().unwrap().has_work(){step(&mut bridge).await;}
    // Complete the real worker while deliberately holding its driver delivery.
    let event=tokio::time::timeout(Duration::from_secs(5),async{loop{let event=bridge.next().await;if matches!(&event.0,EventKind::Publication(_)){break event}bridge.handle(event,0).unwrap();}}).await.unwrap();
    let control=approved().source(publisher.signal().source_generation).unwrap().control("resume");bridge.control(control).unwrap();bridge.handle(event,1).unwrap();
    assert!(matches!(providing.await,Err(SessionError::Closed)));assert!(publisher.signal().original.is_none());flush(&mut bridge,&mut output);
    assert!(!output.iter().any(|v|matches!(v,Body::LosslessFrame(v)if v.available)));close(&publisher,&mut bridge).await;bridge.shutdown().await.unwrap();
}
#[tokio::test]
async fn publication_close_does_not_close_receiver_and_link_close_joins_both_owners(){
    let _gate=WORKERS.lock().await;let hub=Hub::new();let mut bridge=connection(&hub,1);let publisher=open(&hub,&mut bridge).await;
    let mut viewing=Box::pin(hub.open(1,BUDGET));let view=drive(&mut bridge,viewing.as_mut(),&mut Vec::new()).await.unwrap();
    close(&publisher,&mut bridge).await;assert!(!bridge.is_stopped());assert!(view.lease.check().is_ok());assert_eq!(bridge.pending_publication_close.as_ref().unwrap().action,"source_closed");
    let next=open(&hub,&mut bridge).await;let mut closing=Box::pin(hub.close());assert!(poll_fn(|cx|Poll::Ready(closing.as_mut().poll(cx).is_pending())).await);
    assert!(next.signal().closed);bridge.shutdown().await.unwrap();closing.await.unwrap();next.close_publisher().await.unwrap();view.close_stream().await.unwrap();
}
