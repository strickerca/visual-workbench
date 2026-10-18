#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
mod common;
use common::*;
use vw_stream::*;

#[test]fn default_ring_retains_exact_last_eight_and_never_substitutes(){
    let mut r=ring(8,8);for n in 1..=12{frame(&mut r,n,8,8,[n as u8,2,3,255]);}
    assert_eq!(r.retained_frames(),8);assert!(matches!(r.pin(&id(1),4),Err(StreamError::Unavailable)));
    for n in 5..=12{let f=r.pin(&id(1),n).unwrap();assert_eq!(f.identity().frame_id,n);assert_eq!(f.rgba()[0],n as u8);}
    assert!(matches!(r.pin(&id(2),12),Err(StreamError::Stale)));
}
#[test]fn pinned_evicted_original_stays_charged_and_pressure_refuses_before_allocating(){
    let charge=8*8*4+4096;let mut r=FrameRing::new(id(1),geometry(8,8,1000),8,8*charge).unwrap();
    let mut pins=Vec::new();for n in 1..=8{pins.push(frame(&mut r,n,8,8,[n as u8,0,0,255]));}
    assert_eq!(r.used_bytes(),8*charge);
    assert!(matches!(r.begin_frame(identity(9,8,8),&Cancellation::default()),Err(StreamError::Limit)));
    assert_eq!(r.used_bytes(),8*charge);assert_eq!(pins[0].rgba()[0],1);
    pins.remove(0);frame(&mut r,9,8,8,[9,0,0,255]);assert_eq!(r.used_bytes(),8*charge);
}
#[test]fn dropped_writer_and_failed_opaque_validation_release_their_reservation(){
    let mut r=ring(8,8);{let _w=r.begin_frame(identity(1,8,8),&Cancellation::default()).unwrap();assert!(r.used_bytes()>0);}
    assert_eq!(r.used_bytes(),0);
    let writer=r.begin_frame(identity(1,8,8),&Cancellation::default()).unwrap();
    assert!(matches!(writer.finish(&Cancellation::default()),Err(StreamError::Invalid)));assert_eq!(r.used_bytes(),0);
}
#[test]fn geometry_epoch_and_duplicate_frames_cannot_replace_history(){
    let mut r=ring(8,8);frame(&mut r,1,8,8,[1,2,3,255]);let before=r.used_bytes();
    assert!(matches!(r.begin_frame(identity(1,8,8),&Cancellation::default()),Err(StreamError::Stale)));
    let mut foreign=identity(2,8,8);foreign.session=id(2);
    assert!(matches!(r.begin_frame(foreign,&Cancellation::default()),Err(StreamError::Stale)));
    let mut changed=identity(2,8,8);let mut wire=changed.geometry.wire().clone();wire.geometry_revision=2;
    changed.geometry=StreamGeometry::new(&wire,8,8).unwrap();
    assert!(matches!(r.begin_frame(changed,&Cancellation::default()),Err(StreamError::Stale)));assert_eq!(r.used_bytes(),before);
}
#[test]fn eight_4k_frames_fit_hard_300mb_but_large_sources_cannot_relax_capacity(){
    // Arithmetic-only admission: no giant frame allocation in this fixture.
    assert!(FrameRing::default_for(id(1),geometry(3840,2160,1)).is_ok());
    assert!(matches!(FrameRing::default_for(id(1),geometry(7680,4320,1)),Err(StreamError::Limit)));
    assert!(matches!(FrameRing::new(id(1),geometry(8,8,1),7,MAX_RING_BYTES),Err(StreamError::Limit)));
    assert!(matches!(FrameRing::new(id(1),geometry(8,8,1),8,MAX_RING_BYTES+1),Err(StreamError::Limit)));
}
#[test]fn cancellation_precedes_eviction_and_allocation(){
    let mut r=ring(8,8);for n in 1..=8{frame(&mut r,n,8,8,[1,2,3,255]);}
    let token=Cancellation::default();token.cancel();let before=r.used_bytes();
    assert!(matches!(r.begin_frame(identity(9,8,8),&token),Err(StreamError::Cancelled)));
    assert_eq!(r.retained_frames(),8);assert_eq!(r.used_bytes(),before);assert!(r.pin(&id(1),1).is_ok());
}
#[test]fn platform_readback_can_adopt_one_pre_admitted_vec_without_a_second_frame_copy(){
    let mut r=ring(8,8);let reservation=r.reserve_frame(identity(1,8,8),&Cancellation::default()).unwrap();
    let bytes=[1,2,3,255].repeat(64);let pointer=bytes.as_ptr();
    let f=reservation.adopt(bytes,&Cancellation::default()).unwrap();assert_eq!(f.rgba().as_ptr(),pointer);r.publish(f).unwrap();assert_eq!(r.retained_frames(),1);
}
#[test]fn excess_readback_capacity_cannot_hide_unaccounted_retained_bytes(){
    let mut r=ring(8,8);let reservation=r.reserve_frame(identity(1,8,8),&Cancellation::default()).unwrap();
    let mut bytes=Vec::with_capacity(8*8*4+1024);bytes.extend_from_slice(&[1,2,3,255].repeat(64));
    assert!(matches!(reservation.adopt(bytes,&Cancellation::default()),Err(StreamError::Limit)));assert_eq!(r.used_bytes(),0);
}
