#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
use super::*;
use super::super::{publish_order::PublishedFrame,receiver_tests::{BUDGET,id},verified_tests::{root,fixture}};
use std::io::{Read,Seek,SeekFrom};
fn payload(budget:&ByteBudget)->OriginalPayload{
    let root=root();let(offer,mut file)=fixture(root.path());file.seek(SeekFrom::Start(0)).unwrap();let mut encoded=Vec::new();file.read_to_end(&mut encoded).unwrap();
    let request=OriginalRequest{sequence:1,draft:id(8),frame:PublishedFrame{frame:offer.frame,stamp:offer.stamp}};
    OriginalPayload::prepare(request,1,encoded,budget,&vw_stream::Cancellation::default()).unwrap()
}
#[test]
fn original_announcement_precedes_scoped_request_and_backpressure_retains_exact_chunk(){
    let budget=ByteBudget::new(BUDGET).unwrap();let mut sender=Outgoing::new(payload(&budget));let request=sender.offer().request();
    assert!(sender.receive(&request).is_err());sender.flush(|_|Err(vw_net::NetError::Backpressure)).unwrap();assert!(!sender.announced);
    let mut output=Vec::new();sender.flush(|body|{output.push(body);Ok(())}).unwrap();assert!(matches!(&output[0],Body::LosslessFrame(_)));
    assert!(!sender.receive(&request).unwrap());sender.flush(|_|Err(vw_net::NetError::Backpressure)).unwrap();
    output.clear();sender.flush(|body|{output.push(body);Ok(())}).unwrap();
    assert_eq!(output.len(),1);let Body::StreamBlob(chunk)=&output[0]else{panic!("no chunk")};
    let pb::stream_blob::Body::Chunk(bytes)=chunk.body.as_ref().unwrap()else{panic!("no bytes")};assert_eq!(bytes.offset,0);assert!(bytes.last);assert_eq!(AssetId::hash(&bytes.data),sender.offer.asset);
    let ack=sender.offer.scope.envelope(pb::stream_blob::Body::Ack(pb::BlobAck{asset_id:sender.offer.asset.to_string(),next_offset:sender.offer.bytes,verified:true}));
    sender.receive(&ack).unwrap();assert!(sender.verified());sender.receive(&ack).unwrap();
    let stale=sender.offer.scope.envelope(pb::stream_blob::Body::Ack(pb::BlobAck{asset_id:sender.offer.asset.to_string(),next_offset:0,verified:false}));sender.receive(&stale).unwrap();assert!(sender.verified());
    assert!(!sender.receive(&sender.offer.scope.envelope(pb::stream_blob::Body::Cancel(pb::StreamTransferCancel{}))).unwrap());
    drop(output);drop(sender);assert_eq!(budget.used(),0);
}
#[test]
fn arbitrary_asset_or_foreign_scope_cannot_start_or_cancel_the_approved_original(){
    let budget=ByteBudget::new(BUDGET).unwrap();let mut sender=Outgoing::new(payload(&budget));sender.flush(|_|Ok(())).unwrap();
    let mut request=sender.offer.request();request.key.as_mut().unwrap().generation+=1;assert!(!sender.receive(&request).unwrap());assert!(sender.source.is_some());
    request=sender.offer.request();let pb::stream_blob::Body::Request(body)=request.body.as_mut().unwrap()else{panic!("wrong request")};body.asset_id=AssetId::hash(b"project asset").to_string();
    assert!(sender.receive(&request).is_err());assert!(sender.source.is_some());
    let mut scope=sender.offer.scope.clone();scope.request=id(9);assert!(!sender.receive(&scope.envelope(pb::stream_blob::Body::Cancel(pb::StreamTransferCancel{}))).unwrap());
    assert!(sender.receive(&sender.offer.scope.envelope(pb::stream_blob::Body::Cancel(pb::StreamTransferCancel{}))).unwrap());
}
#[test]
fn malformed_or_cancelled_local_original_never_reaches_an_offer_and_releases_budget(){
    let budget=ByteBudget::new(BUDGET).unwrap();let valid=payload(&budget);let request=valid.request.clone();drop(valid);
    assert!(OriginalPayload::prepare(request.clone(),1,b"not a png".to_vec(),&budget,&vw_stream::Cancellation::default()).is_err());assert_eq!(budget.used(),0);
    let cancelled=vw_stream::Cancellation::default();cancelled.cancel();assert!(OriginalPayload::prepare(request,1,vec![1],&budget,&cancelled).is_err());assert_eq!(budget.used(),0);
}
