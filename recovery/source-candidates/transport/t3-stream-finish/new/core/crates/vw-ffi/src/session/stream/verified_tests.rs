#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
use super::{memory::ByteBudget,wire::{OriginalOffer,Scope},verified::verify};
use super::receiver_tests::{id,identity,stamp,BUDGET};
use std::io::Write;
use vw_stream::{Cancellation,FrameRing};

pub(super) fn root()->tempfile::TempDir{
    let mut builder=tempfile::Builder::new();builder.prefix("vw-stream-original-test-");
    #[cfg(target_os="android")]
    {builder.tempdir_in(std::env::current_dir().unwrap()).unwrap()}
    #[cfg(not(target_os="android"))]
    {builder.tempdir().unwrap()}
}
pub(super) fn fixture(root:&std::path::Path)->(OriginalOffer,tempfile::NamedTempFile){
    let frame=identity(1);let mut ring=FrameRing::default_for(frame.session.clone(),frame.geometry.clone()).unwrap();
    let mut writer=ring.begin_frame(frame.clone(),&Cancellation::default()).unwrap();for p in writer.rgba_mut().chunks_exact_mut(4){p.copy_from_slice(&[40,80,120,255]);}
    let original=writer.finish(&Cancellation::default()).unwrap();let encoded=vw_stream::encode_original(&original,vw_stream::MAX_LOSSLESS_BYTES,BUDGET,&Cancellation::default()).unwrap();
    let offer=OriginalOffer{scope:Scope{session:id(1),frame:1,geometry_revision:1,request:id(8),transfer:id(9),generation:1},
        frame,asset:encoded.asset_id.clone(),bytes:encoded.bytes().len()as u64,stamp:stamp(1)};
    let mut file=tempfile::NamedTempFile::new_in(root).unwrap();file.write_all(encoded.bytes()).unwrap();(offer,file)
}
#[test]
fn verified_original_retains_real_provenance_exact_source_and_charge_until_disposal(){
    let root=root();let(offer,file)=fixture(root.path());let path=file.path().to_path_buf();let budget=ByteBudget::new(BUDGET).unwrap();
    let value=verify(offer.clone(),file,budget.clone(),&Cancellation::default()).unwrap();assert_eq!(value.pixels().identity(),&offer.frame);
    assert_eq!(value.offer().stamp,stamp(1));assert_eq!(value.pixels().asset_id(),&offer.asset);assert_eq!(value.encoded_file().metadata().unwrap().len(),offer.bytes);
    assert_eq!(budget.used(),16*9*4+4096);assert!(path.exists());drop(value);assert_eq!(budget.used(),0);assert!(!path.exists());
}
#[test]
fn corrupt_hash_and_size_refuse_without_retained_bytes_or_scratch(){
    let root=root();let budget=ByteBudget::new(BUDGET).unwrap();
    let(mut offer,file)=fixture(root.path());let path=file.path().to_path_buf();offer.asset=vw_model::AssetId::hash(b"wrong");
    assert!(verify(offer,file,budget.clone(),&Cancellation::default()).is_err());assert_eq!(budget.used(),0);assert!(!path.exists());
    let(mut offer,file)=fixture(root.path());let path=file.path().to_path_buf();offer.bytes+=1;
    assert!(verify(offer,file,budget.clone(),&Cancellation::default()).is_err());assert_eq!(budget.used(),0);assert!(!path.exists());
}
#[test]
fn cancelled_and_insufficient_budget_verification_never_exposes_lossless_success(){
    let root=root();let(offer,file)=fixture(root.path());let path=file.path().to_path_buf();let budget=ByteBudget::new(BUDGET).unwrap();let cancel=Cancellation::default();cancel.cancel();
    assert!(verify(offer,file,budget.clone(),&cancel).is_err());assert_eq!(budget.used(),0);assert!(!path.exists());
    let(offer,file)=fixture(root.path());let path=file.path().to_path_buf();let small=ByteBudget::new(4096).unwrap();
    assert!(verify(offer,file,small.clone(),&Cancellation::default()).is_err());assert_eq!(small.used(),0);assert!(!path.exists());
}
