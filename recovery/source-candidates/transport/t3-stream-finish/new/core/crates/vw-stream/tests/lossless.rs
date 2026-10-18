#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
mod common;
use common::*;
use vw_stream::*;
use vw_model::{AssetId,DeviceId};
const WORK:u64=128*1024*1024;
#[test]fn lossless_frame_roundtrip_binds_hash_extent_geometry_and_every_rgba_sample(){
    let mut r=ring(32,16);let mut w=r.begin_frame(identity(1,32,16),&Cancellation::default()).unwrap();
    for (n,p) in w.rgba_mut().chunks_exact_mut(4).enumerate(){p.copy_from_slice(&[n as u8,(n/32) as u8,(n*13) as u8,255]);}
    let f=w.finish(&Cancellation::default()).unwrap();r.publish(f.clone()).unwrap();
    let encoded=encode_original(&f,MAX_LOSSLESS_BYTES,WORK,&Cancellation::default()).unwrap();
    let verified=verify_original(&encoded.identity,&encoded.asset_id,encoded.bytes(),WORK,&Cancellation::default()).unwrap();
    assert_eq!(verified.identity(),f.identity());assert_eq!(verified.rgba(),f.rgba());assert_eq!(AssetId::hash(encoded.bytes()),encoded.asset_id);
}
#[test]fn approved_region_crops_same_original_with_negative_physical_origin_and_unchanged_time(){
    let mut r=ring(8,8);let f=frame(&mut r,1,8,8,[21,22,23,255]);let peer=AuthenticatedPeer{device:DeviceId::from_bytes([1;16]),epoch:1};
    let mut c=RegionConsent::new(peer.clone()).unwrap();c.request(&peer,1,id(2),0).unwrap();let offer=c.approve(&id(2),id(3),f,1).unwrap();
    let chosen=c.accept(&peer,&RegionChoice{request_id:id(2),offer_id:id(3),frame:offer.frame.clone(),rectangle:TileRect{x:2,y:3,width:4,height:2}},2).unwrap();
    let encoded=encode_region(&chosen,MAX_LOSSLESS_BYTES,WORK,&Cancellation::default()).unwrap();
    let verified=verify_original(&encoded.identity,&encoded.asset_id,encoded.bytes(),WORK,&Cancellation::default()).unwrap();
    let g=verified.identity().geometry.wire();assert_eq!(g.source_kind,"region");assert_eq!(g.client_rect_host.as_ref().unwrap().x,-1918);assert_eq!(g.client_rect_host.as_ref().unwrap().y,-253);
    assert_eq!(verified.identity().timestamp_ns,offer.frame.timestamp_ns);assert_eq!(verified.rgba(),[21,22,23,255].repeat(8));
}
#[test]fn corrupt_crc_hash_dimension_metadata_and_trailing_bytes_cannot_be_verified(){
    let mut r=ring(8,8);let f=frame(&mut r,1,8,8,[21,22,23,255]);let encoded=encode_original(&f,MAX_LOSSLESS_BYTES,WORK,&Cancellation::default()).unwrap();
    assert!(matches!(verify_original(&encoded.identity,&AssetId::hash(b"wrong"),encoded.bytes(),WORK,&Cancellation::default()),Err(StreamError::Invalid)));
    let mut bad=encoded.bytes().to_vec();bad[29]^=1;assert!(verify_original(&encoded.identity,&AssetId::hash(&bad),&bad,WORK,&Cancellation::default()).is_err());
    let mut bad=encoded.bytes().to_vec();bad.extend_from_slice(b"tail");assert!(verify_original(&encoded.identity,&AssetId::hash(&bad),&bad,WORK,&Cancellation::default()).is_err());
    let wrong=identity(1,9,8);assert!(verify_original(&wrong,&encoded.asset_id,encoded.bytes(),WORK,&Cancellation::default()).is_err());
    let mut bad=encoded.bytes().to_vec();bad[37..41].copy_from_slice(b"iCCP");assert!(verify_original(&encoded.identity,&AssetId::hash(&bad),&bad,WORK,&Cancellation::default()).is_err());
}
#[test]fn complete_output_and_decode_budgets_are_admitted_before_codec(){
    let mut r=ring(8,8);let f=frame(&mut r,1,8,8,[1,2,3,255]);
    assert!(matches!(encode_original(&f,MAX_LOSSLESS_BYTES,1,&Cancellation::default()),Err(StreamError::Limit)));
    assert!(encode_original(&f,1,WORK,&Cancellation::default()).is_err());
    let encoded=encode_original(&f,MAX_LOSSLESS_BYTES,WORK,&Cancellation::default()).unwrap();
    assert!(matches!(verify_original(&encoded.identity,&encoded.asset_id,encoded.bytes(),1,&Cancellation::default()),Err(StreamError::Limit)));
    let cancel=Cancellation::default();cancel.cancel();assert!(matches!(encode_original(&f,MAX_LOSSLESS_BYTES,WORK,&cancel),Err(StreamError::Cancelled)));
}
