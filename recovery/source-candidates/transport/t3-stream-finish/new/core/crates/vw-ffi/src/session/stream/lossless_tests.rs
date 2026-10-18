#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
use super::*;
use super::super::{receiver_tests::{WORKERS,BUDGET,id,packet,source,stamp},verified_tests::{root,fixture}};
use std::{io::{Read,Seek,SeekFrom},time::Duration};

async fn receiver()->(StreamReceiver,Lossless){
    let mut receiver=StreamReceiver::new(1,BUDGET).unwrap();receiver.open(&source(1).control("start")).unwrap();
    receiver.media(packet(1,1)).unwrap();receiver.start_decode().unwrap();receiver.complete_decode().await.unwrap();
    let draw=receiver.begin_draw().unwrap().unwrap();receiver.draw_receipt(1,&draw.token,true).unwrap();drop(draw);
    let request=receiver.contact(id(8),true,0).unwrap().unwrap();
    let flow=Lossless::new(request,stamp(1),receiver.budget().clone()).unwrap();(receiver,flow)
}
fn chunk(offer:&OriginalOffer,bytes:Vec<u8>)->pb::StreamBlob{offer.scope.envelope(pb::stream_blob::Body::Chunk(pb::BlobChunk{
    asset_id:offer.asset.to_string(),offset:0,total_size:offer.bytes,last:true,data:bytes,
}))}
fn staged(root:&std::path::Path)->(OriginalOffer,tempfile::NamedTempFile,Vec<u8>){
    let(offer,mut file)=fixture(root);file.seek(SeekFrom::Start(0)).unwrap();let mut bytes=Vec::new();file.read_to_end(&mut bytes).unwrap();
    file.as_file_mut().set_len(0).unwrap();file.seek(SeekFrom::Start(0)).unwrap();(offer,file,bytes)
}
async fn finish(flow:&mut Lossless,receiver:&mut StreamReceiver)->SessionResult<Original>{
    for _ in 0..8{
        flow.advance(receiver)?;
        let event=tokio::time::timeout(Duration::from_secs(5),flow.next()).await.unwrap();
        if let Some(original)=flow.handle(event)?{return Ok(original)}
    }panic!("bounded transfer events exhausted")
}

#[tokio::test]
async fn blob_hash_is_not_lossless_proof_and_late_exact_png_can_upgrade_degraded_frame(){
    let _gate=WORKERS.lock().await;let root=root();let(mut receiver,mut flow)=receiver().await;
    assert!(receiver.tick(2000));assert_eq!(receiver.frozen().unwrap().quality(),Some(&vw_stream::FreezeQuality::DegradedCompressed));
    let(offer,file,bytes)=staged(root.path());let path=file.path().to_path_buf();
    assert!(matches!(flow.offer(&offer.wire()).unwrap(),Offer::Create(_)));flow.install(offer.clone(),file).unwrap();
    assert!(!flow.receive(chunk(&offer,bytes)).unwrap());flow.advance(&mut receiver).unwrap();
    let event=tokio::time::timeout(Duration::from_secs(5),flow.next()).await.unwrap();assert!(matches!(&event,Event::Chunk(Ok(_))));
    assert!(flow.handle(event).unwrap().is_none());
    let pb::stream_blob::Body::Ack(ack)=flow.ack.as_ref().unwrap().body.as_ref().unwrap()else{panic!("not an ack")};assert!(ack.verified);
    assert_eq!(receiver.frozen().unwrap().quality(),Some(&vw_stream::FreezeQuality::DegradedCompressed));
    let original=finish(&mut flow,&mut receiver).await.unwrap();assert!(receiver.lossless(flow.request(),original.pixels()).unwrap());
    assert_eq!(original.pixels().identity(),&flow.request().frame);
    assert!(!flow.receive(offer.scope.envelope(pb::stream_blob::Body::Cancel(pb::StreamTransferCancel{}))).unwrap());
    flow.shutdown().await.unwrap();receiver.shutdown().await.unwrap();assert!(path.exists());drop(original);assert!(!path.exists());
}
#[tokio::test]
async fn same_frame_provenance_hash_or_scope_cannot_change_after_first_offer(){
    let _gate=WORKERS.lock().await;let root=root();let(mut receiver,mut flow)=receiver().await;
    let(offer,file,_)=staged(root.path());let mut other=offer.wire();other.request_id=Some(id(7).to_proto());
    assert!(matches!(flow.offer(&other).unwrap(),Offer::Ignored));assert!(flow.offer.is_none());
    other=offer.wire();other.captured_at_ms+=1;assert!(matches!(flow.offer(&other),Err(SessionError::Invalid)));assert!(flow.offer.is_none());
    assert!(matches!(flow.offer(&offer.wire()).unwrap(),Offer::Create(_)));assert!(matches!(flow.offer(&offer.wire()).unwrap(),Offer::Ignored));
    other=offer.wire();other.asset_id=vw_model::AssetId::hash(b"changed").to_string();assert!(flow.offer(&other).is_err());
    other=offer.wire();other.transfer.as_mut().unwrap().generation+=1;assert!(flow.offer(&other).is_err());
    drop(file);flow.shutdown().await.unwrap();receiver.shutdown().await.unwrap();
}
#[tokio::test]
async fn correctly_hashed_non_png_never_becomes_a_lossless_original(){
    let _gate=WORKERS.lock().await;let root=root();let(mut receiver,mut flow)=receiver().await;
    let(mut offer,file,_)=staged(root.path());let path=file.path().to_path_buf();let bytes=b"valid blob hash, invalid PNG".to_vec();
    offer.asset=vw_model::AssetId::hash(&bytes);offer.bytes=bytes.len()as u64;
    assert!(matches!(flow.offer(&offer.wire()).unwrap(),Offer::Create(_)));flow.install(offer.clone(),file).unwrap();flow.receive(chunk(&offer,bytes)).unwrap();
    assert!(finish(&mut flow,&mut receiver).await.is_err());assert!(!matches!(receiver.frozen().unwrap().quality(),Some(vw_stream::FreezeQuality::Lossless{..})));
    flow.shutdown().await.unwrap();receiver.shutdown().await.unwrap();assert!(!path.exists());
}
#[tokio::test]
async fn exact_cancel_destroys_queued_file_and_cannot_publish_a_late_codec_result(){
    let _gate=WORKERS.lock().await;let root=root();let(mut receiver,mut flow)=receiver().await;
    let(offer,file,bytes)=staged(root.path());let path=file.path().to_path_buf();flow.offer(&offer.wire()).unwrap();flow.install(offer.clone(),file).unwrap();
    flow.receive(chunk(&offer,bytes)).unwrap();
    let mut foreign=offer.scope.clone();foreign.generation+=1;
    assert!(!flow.receive(foreign.envelope(pb::stream_blob::Body::Cancel(pb::StreamTransferCancel{}))).unwrap());
    assert!(flow.receive(offer.scope.envelope(pb::stream_blob::Body::Cancel(pb::StreamTransferCancel{}))).unwrap());
    flow.advance(&mut receiver).unwrap();assert!(flow.chunk.is_none());flow.shutdown().await.unwrap();assert!(!path.exists());
    assert!(!matches!(receiver.frozen().unwrap().quality(),Some(vw_stream::FreezeQuality::Lossless{..})));receiver.shutdown().await.unwrap();
}
