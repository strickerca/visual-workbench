#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
mod common;
use common::*;
use vw_model::DeviceId;
use vw_stream::*;
fn peer(epoch:u64)->AuthenticatedPeer{AuthenticatedPeer{device:DeviceId::from_bytes([7;16]),epoch}}
fn choice(offer:&RegionOffer)->RegionChoice{RegionChoice{request_id:offer.request_id.clone(),offer_id:offer.offer_id.clone(),frame:offer.frame.clone(),rectangle:TileRect{x:1,y:2,width:4,height:3}}}

#[test]fn incoming_request_never_grants_source_access_without_explicit_local_approval(){
    let mut c=RegionConsent::new(peer(1)).unwrap();let mut r=ring(8,8);let f=frame(&mut r,1,8,8,[1,2,3,255]);
    assert!(matches!(c.approve(&id(2),id(3),f.clone(),0),Err(StreamError::Consent)));
    assert!(c.request(&peer(1),1,id(2),0).unwrap());let offered=c.approve(&id(2),id(3),f,1).unwrap();
    let accepted=c.accept(&peer(1),&choice(&offered),2).unwrap();assert_eq!(accepted.frame().identity(),&offered.frame);assert_eq!(accepted.rectangle().width,4);
    assert!(matches!(c.accept(&peer(1),&choice(&offered),3),Err(StreamError::Consent)));
}
#[test]fn foreign_peer_and_reconnected_epoch_cannot_use_an_old_offer(){
    let mut c=RegionConsent::new(peer(1)).unwrap();let mut r=ring(8,8);let f=frame(&mut r,1,8,8,[1,2,3,255]);
    c.request(&peer(1),1,id(2),0).unwrap();let offered=c.approve(&id(2),id(3),f,1).unwrap();
    assert!(matches!(c.accept(&peer(2),&choice(&offered),2),Err(StreamError::Consent)));
    let foreign=AuthenticatedPeer{device:DeviceId::from_bytes([8;16]),epoch:1};assert!(matches!(c.accept(&foreign,&choice(&offered),2),Err(StreamError::Consent)));
    assert!(c.accept(&peer(1),&choice(&offered),2).is_ok());
}
#[test]fn exact_frame_rectangle_and_offer_binding_prevent_scope_expansion(){
    let mut c=RegionConsent::new(peer(1)).unwrap();let mut r=ring(8,8);let f=frame(&mut r,1,8,8,[1,2,3,255]);
    c.request(&peer(1),1,id(2),0).unwrap();let offered=c.approve(&id(2),id(3),f,1).unwrap();
    let mut other=choice(&offered);other.frame.frame_id=2;assert!(matches!(c.accept(&peer(1),&other,2),Err(StreamError::Stale)));
    other=choice(&offered);other.rectangle.width=u32::MAX;assert!(matches!(c.accept(&peer(1),&other,2),Err(StreamError::Invalid)));
    other=choice(&offered);other.offer_id=id(4);assert!(matches!(c.accept(&peer(1),&other,2),Err(StreamError::Stale)));
    assert!(c.accept(&peer(1),&choice(&offered),2).is_ok());
}
#[test]fn expiry_deny_and_revoke_release_authority_without_displacing_pending_request(){
    let mut c=RegionConsent::new(peer(1)).unwrap();let mut r=ring(8,8);let f=frame(&mut r,1,8,8,[1,2,3,255]);
    assert!(c.request(&peer(1),1,id(2),0).unwrap());assert!(!c.request(&peer(1),2,id(4),2000).unwrap());
    let offered=c.approve(&id(2),id(3),f.clone(),3000).unwrap();
    assert!(matches!(c.accept(&peer(1),&choice(&offered),REGION_REQUEST_TTL_MS),Err(StreamError::Consent)));
    assert!(c.request(&peer(1),3,id(4),REGION_REQUEST_TTL_MS).unwrap());assert!(!c.deny(&id(2)));assert!(c.deny(&id(4)));
    assert!(!c.request(&peer(1),4,id(5),REGION_REQUEST_TTL_MS+1).unwrap());
    assert!(c.request(&peer(1),5,id(5),REGION_REQUEST_TTL_MS+1000).unwrap());c.revoke();assert!(c.pending_request(REGION_REQUEST_TTL_MS+1000).is_none());
}
#[test]fn stale_control_sequences_cannot_reopen_a_denied_prompt(){
    let mut c=RegionConsent::new(peer(1)).unwrap();c.request(&peer(1),8,id(2),0).unwrap();c.deny(&id(2));
    assert!(!c.request(&peer(1),7,id(3),2000).unwrap());assert!(!c.request(&peer(1),8,id(2),2000).unwrap());
    assert!(c.request(&peer(1),9,id(3),2000).unwrap());
}
#[test]fn approved_region_pins_original_after_ring_eviction(){
    let mut c=RegionConsent::new(peer(1)).unwrap();let mut r=ring(8,8);let f=frame(&mut r,1,8,8,[11,2,3,255]);
    c.request(&peer(1),1,id(2),0).unwrap();let offered=c.approve(&id(2),id(3),f,1).unwrap();
    for n in 2..=10{frame(&mut r,n,8,8,[n as u8,2,3,255]);}
    assert!(matches!(r.pin(&id(1),1),Err(StreamError::Unavailable)));
    let accepted=c.accept(&peer(1),&choice(&offered),2).unwrap();assert_eq!(accepted.frame().rgba()[0],11);
}
