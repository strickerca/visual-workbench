#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
mod support;
use support::*;
use vw_core::*;

static TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn temp()->tempfile::TempDir {
    if cfg!(target_os="android"){tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()}else{tempfile::tempdir().unwrap()}
}
fn identity()->Transform{Transform{a:1.0,b:0.0,c:0.0,d:1.0,e:0.0,f:0.0}}
fn point(x:f64,y:f64)->Point{Point{x,y}}
fn rectangle(n:u8,x:f64,y:f64,width:f64,height:f64)->EditCommand {
    EditCommand::Create{object_id:id(n),layer_id:id(3),shape:NewShape::Rectangle{rectangle:QueryRect{x,y,width,height}},
        style:ObjectStyle{rgba:0x11223380,width:0.0,screen_constant_width:false,fill:Some(0x11223380)},transform:identity()}
}
fn pixels(bytes:&[u8])->vw_raster::Pixels {vw_raster::decode(bytes,vw_raster::DecodeLimits::default()).unwrap().pixels}

#[test]
fn geometry_boundary_rejects_empty_oversized_singular_and_invalid_inputs() {
    let p=point(0.0,0.0);let start=point(0.0,-1.0);let current=point(1.0,0.0);
    assert!(rotate_object_transforms(vec![],p.clone(),start.clone(),current.clone(),false).is_err());
    assert!(rotate_object_transforms(vec![identity();257],p.clone(),start.clone(),current.clone(),false).is_err());
    assert!(rotate_object_transforms(vec![Transform{d:0.0,..identity()}],p.clone(),start.clone(),current.clone(),false).is_err());
    assert!(rotate_object_transforms(vec![identity()],p.clone(),start.clone(),point(f64::NAN,0.0),false).is_err());
    assert!(rotate_object_transforms(vec![identity()],p.clone(),start,p,false).is_err());
}

#[tokio::test]
async fn a_two_object_rotation_is_one_undoable_retryable_transaction_and_matches_independent_export_geometry() {
    let _serial=TESTS.lock().await;let root=temp();let path=root.path().join("rotated");
    let p=create_image_project(create(&path),cancel()).await.unwrap();
    let q=create_image_project(create(&root.path().join("oracle")),cancel()).await.unwrap();
    let before=p.apply_edit(edit(10,1),vec![rectangle(11,4.0,8.0,12.0,6.0),rectangle(12,28.0,20.0,10.0,4.0)],cancel()).await.unwrap();
    // Independent coordinate oracle for x'=32-y, y'=x. It never calls the
    // rotation helper and catches alpha/fill/export differences over all pixels.
    q.apply_edit(edit(10,1),vec![rectangle(11,18.0,4.0,6.0,12.0),rectangle(12,8.0,28.0,4.0,10.0)],cancel()).await.unwrap();
    let transformed=rotate_object_transforms(vec![identity(),identity()],point(16.0,16.0),point(16.0,0.0),point(32.0,16.0),true).unwrap();
    assert_eq!(p.info().await.unwrap(),before);
    let commands=vec![EditCommand::Transform{object_id:id(11),transform:transformed[0].clone()},EditCommand::Transform{object_id:id(12),transform:transformed[1].clone()}];
    let expected=Some(EditPrecondition{host_seq:before.host_seq,state_hash:before.state_hash.clone()});
    let options=edit(20,before.next_lamport);
    let accepted=p.apply_edit_at(options.clone(),commands.clone(),expected.clone(),Some(id(21)),cancel()).await.unwrap();
    assert_eq!(accepted.host_seq,before.host_seq+1);
    assert_eq!(p.apply_edit_at(options,commands,expected,Some(id(21)),cancel()).await.unwrap(),accepted);
    let rotated=p.export_image(support::options(),cancel()).await.unwrap();
    let oracle=q.export_image(support::options(),cancel()).await.unwrap();
    assert_eq!(pixels(&rotated.bytes),pixels(&oracle.bytes));
    let undone=p.undo_redo(edit(30,accepted.next_lamport),false,cancel()).await.unwrap();
    assert_eq!(undone.state_hash,before.state_hash);
    let redone=p.undo_redo(edit(31,undone.next_lamport),true,cancel()).await.unwrap();
    assert_eq!(redone.state_hash,accepted.state_hash);
    p.close().await.unwrap();q.close().await.unwrap();
    let stored=vw_store::ProjectStore::open(&path).unwrap();
    assert_eq!(stored.project().assets.len(),1);
    let asset=stored.project().assets.keys().next().unwrap();
    assert_eq!(vw_store::BlobStore::new(&path).unwrap().read(asset).unwrap(),source());drop(stored);
    let p=open_project(path.to_string_lossy().into(),cancel()).await.unwrap();
    assert_eq!(pixels(&p.export_image(support::options(),cancel()).await.unwrap().bytes),pixels(&rotated.bytes));
    p.close().await.unwrap();
}

#[tokio::test]
async fn previews_do_not_mutate_and_a_changed_visible_revision_refuses_the_whole_rotation() {
    let _serial=TESTS.lock().await;let root=temp();let p=create_image_project(create(&root.path().join("p")),cancel()).await.unwrap();
    let captured=p.apply_edit(edit(10,1),vec![rectangle(11,4.0,8.0,12.0,6.0)],cancel()).await.unwrap();
    for n in 1..=100 {rotate_object_transforms(vec![identity()],point(16.0,16.0),point(16.0,0.0),point(16.0+f64::from(n),8.0),n%2==0).unwrap();}
    assert_eq!(p.info().await.unwrap(),captured);
    let changed=p.apply_edit(edit(20,captured.next_lamport),vec![rectangle(12,40.0,40.0,8.0,8.0)],cancel()).await.unwrap();
    let transform=rotate_object_transforms(vec![identity()],point(16.0,16.0),point(16.0,0.0),point(32.0,16.0),true).unwrap().remove(0);
    assert!(p.apply_edit_at(edit(30,changed.next_lamport),vec![EditCommand::Transform{object_id:id(11),transform}],
        Some(EditPrecondition{host_seq:captured.host_seq,state_hash:captured.state_hash}),Some(id(31)),cancel()).await.is_err());
    assert_eq!(p.info().await.unwrap(),changed);p.close().await.unwrap();
}
