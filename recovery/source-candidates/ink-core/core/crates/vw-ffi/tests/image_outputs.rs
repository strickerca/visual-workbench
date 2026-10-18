//! Real serialized project, immutable plan, compositor and publication regressions.
#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
mod support;
use support::*;
use std::sync::Arc;
use vw_core::*;
use vw_model::DeviceId;
const BUDGET:u64=256*1024*1024;
static TESTS:tokio::sync::Mutex<()>=tokio::sync::Mutex::const_new(());
fn temp()->tempfile::TempDir {if cfg!(target_os="android"){tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()}else{tempfile::tempdir().unwrap()}}
fn frame()->ImageOutputSettings {ImageOutputSettings{crop_x:8,crop_y:4,crop_width:32,crop_height:24,clockwise_degrees:90,output_width:24,output_height:32}}
fn command()->ImageOutputCommand {ImageOutputCommand::SetFrame{layer_id:id(30),object_id:id(31),settings:frame(),enabled:true}}
async fn snapshot(p:&Arc<ProjectSession>)->ImageOutputSnapshot {p.image_output_snapshot(id(2),BUDGET,cancel()).await.unwrap()}
async fn prepare(p:&Arc<ProjectSession>,n:u8,cmd:ImageOutputCommand)->Arc<WorkflowPlan> {
    let view=snapshot(p).await;let info=p.info().await.unwrap();
    p.clone().prepare_image_output(view.binding,WorkflowMetadata{transaction_id:id(n),device_id:info.device_id,
        first_lamport:info.next_lamport,created_at_ms:TIME+i64::from(n)},cmd,BUDGET,cancel()).await.unwrap()
}
async fn apply(p:&Arc<ProjectSession>,n:u8,cmd:ImageOutputCommand)->WorkflowReceipt {
    let plan=prepare(p,n,cmd).await;let result=plan.commit(cancel()).await.unwrap();plan.dispose();result
}
#[tokio::test]
async fn output_is_one_atomic_undoable_edit_with_exact_plan_retry_and_unchanged_original() {
    let _serial=TESTS.lock().await;let root=temp();let path=root.path().join("p");let p=create_image_project(create(&path),cancel()).await.unwrap();
    let initial=p.info().await.unwrap();let plan=prepare(&p,20,command()).await;
    assert_eq!(initial,p.info().await.unwrap());assert!(snapshot(&p).await.output.is_none());
    let accepted=plan.commit(cancel()).await.unwrap();let duplicate=plan.commit(cancel()).await.unwrap();
    assert!(!accepted.duplicate);assert!(duplicate.duplicate);assert_eq!(accepted.revision,duplicate.revision);plan.dispose();
    assert_eq!(snapshot(&p).await.output.unwrap().settings,frame());
    assert!(p.render_list(id(2),None,cancel()).await.unwrap().items.is_empty());
    p.undo_redo(edit(21,accepted.revision.next_lamport),false,cancel()).await.unwrap();
    assert!(snapshot(&p).await.output.is_none());assert_eq!(p.info().await.unwrap().state_hash,initial.state_hash);
    let now=p.info().await.unwrap();p.undo_redo(edit(22,now.next_lamport),true,cancel()).await.unwrap();p.close().await.unwrap();
    let store=vw_store::ProjectStore::open(&path).unwrap();let source_id=store.project().assets.keys().next().unwrap();
    assert_eq!(vw_store::BlobStore::new(&path).unwrap().read(source_id).unwrap(),source());drop(store);
    let p=open_project(path.to_string_lossy().into(),cancel()).await.unwrap();assert_eq!(snapshot(&p).await.output.unwrap().settings,frame());p.close().await.unwrap();
}
#[tokio::test]
async fn native_preview_buffer_export_and_private_file_export_agree_without_resizing_again() {
    let _serial=TESTS.lock().await;let root=temp();let p=create_image_project(create(&root.path().join("p")),cancel()).await.unwrap();
    apply(&p,20,command()).await;let view=snapshot(&p).await;
    let preview=p.image_output_preview(view.binding.clone(),SelectionRegion{x:0,y:0,width:24,height:32},false,true,BUDGET,cancel()).await.unwrap();
    assert_eq!(preview.binding,view.binding);assert_eq!((preview.width,preview.height),(24,32));
    let mut opts=options();opts.marked=false;
    let exported=p.export_image(opts.clone(),cancel()).await.unwrap();
    let decoded=vw_raster::decode(&exported.bytes,vw_raster::DecodeLimits::default()).unwrap();
    assert_eq!((decoded.width,decoded.height),(24,32));assert_eq!(decoded.pixels.rgba8(),preview.rgba);
    let metadata:vw_raster::ExportMetadata=serde_json::from_str(&exported.metadata_json).unwrap();
    assert_eq!(metadata.image_output.unwrap().clockwise_degrees,90);assert_eq!(metadata.source_width,64);
    let work=root.path().join("work");std::fs::create_dir(&work).unwrap();let target=work.join("complete.png");
    let transfer=FileExportOptions{export:opts,work_directory:work.to_string_lossy().into(),output_path:target.to_string_lossy().into(),
        max_encoded_bytes:64*1024*1024,max_scratch_bytes:128*1024*1024};
    let preflight=p.preflight_image_file(transfer.clone(),cancel()).await.unwrap();
    assert_eq!((preflight.width,preflight.height),(24,32));assert!(preflight.buffered&&preflight.requires_render_validation);
    let receipt=p.export_transfer_file(transfer.clone(),cancel()).await.unwrap();
    assert_eq!(receipt.revision,exported.revision);let bytes=std::fs::read(&target).unwrap();
    assert_eq!(vw_raster::decode(&bytes,vw_raster::DecodeLimits::default()).unwrap().pixels.rgba8(),preview.rgba);
    assert!(p.export_transfer_file(transfer,cancel()).await.is_err());assert_eq!(std::fs::read(&target).unwrap(),bytes);
    assert_eq!(std::fs::read_dir(&work).unwrap().count(),1);p.close().await.unwrap();
}
#[tokio::test]
async fn stale_binding_cancelled_commit_and_disabled_frame_do_not_damage_prior_settings() {
    let _serial=TESTS.lock().await;let root=temp();let p=create_image_project(create(&root.path().join("p")),cancel()).await.unwrap();
    let old=prepare(&p,20,command()).await;let current=prepare(&p,21,command()).await;current.commit(cancel()).await.unwrap();current.dispose();
    assert!(matches!(old.commit(cancel()).await,Err(WorkflowError::Stale)));old.dispose();
    let cancelled=prepare(&p,22,ImageOutputCommand::RemoveFrame{layer_id:id(30),object_id:id(31)}).await;
    let token=cancel();token.cancel();assert!(matches!(cancelled.commit(token).await,Err(WorkflowError::Cancelled)));cancelled.dispose();
    apply(&p,23,ImageOutputCommand::SetFrame{layer_id:id(30),object_id:id(31),settings:frame(),enabled:false}).await;
    assert!(!snapshot(&p).await.output.unwrap().enabled);
    let image=p.export_image(options(),cancel()).await.unwrap();let decoded=vw_raster::decode(&image.bytes,vw_raster::DecodeLimits::default()).unwrap();
    assert_eq!((decoded.width,decoded.height),(64,64));p.close().await.unwrap();
}
#[tokio::test]
async fn locked_output_refuses_replacement_removal_and_malformed_frames_atomically() {
    let _serial=TESTS.lock().await;let root=temp();let p=create_image_project(create(&root.path().join("p")),cancel()).await.unwrap();apply(&p,20,command()).await;
    let info=p.info().await.unwrap();let binding=snapshot(&p).await.binding;
    let lock=p.clone().prepare_document_control(binding,WorkflowMetadata{transaction_id:id(21),device_id:info.device_id,first_lamport:info.next_lamport,created_at_ms:TIME+21},
        DocumentControlCommand::UpdateLayer{update:LayerControlUpdate{layer_id:id(30),locked:Some(true),name:None,visible:None,opacity:None,blend:None}},BUDGET,cancel()).await.unwrap();
    lock.commit(cancel()).await.unwrap();lock.dispose();let before=p.info().await.unwrap();let view=snapshot(&p).await;
    for (n,cmd) in [(22,command()),(23,ImageOutputCommand::RemoveFrame{layer_id:id(30),object_id:id(31)})] {
        assert!(matches!(p.clone().prepare_image_output(view.binding.clone(),WorkflowMetadata{transaction_id:id(n),device_id:before.device_id.clone(),first_lamport:before.next_lamport,created_at_ms:TIME+i64::from(n)},cmd,BUDGET,cancel()).await,Err(WorkflowError::Locked)));
    }
    let mut invalid=frame();invalid.clockwise_degrees=45;
    assert!(p.clone().prepare_image_output(view.binding,WorkflowMetadata{transaction_id:id(24),device_id:before.device_id.clone(),first_lamport:before.next_lamport,created_at_ms:TIME+24},
        ImageOutputCommand::SetFrame{layer_id:id(30),object_id:id(31),settings:invalid,enabled:true},BUDGET,cancel()).await.is_err());
    assert_eq!(p.info().await.unwrap(),before);p.close().await.unwrap();
}
#[tokio::test]
async fn tiny_budget_refuses_before_original_blob_access_or_canonical_hash() {
    let _serial=TESTS.lock().await;let root=temp();let path=root.path().join("p");let p=create_image_project(create(&path),cancel()).await.unwrap();apply(&p,20,command()).await;
    let view=snapshot(&p).await;let original=vw_store::BlobStore::new(&path).unwrap();let asset=vw_model::AssetId::try_from(view.source_asset_id.clone()).unwrap();
    let blob=original.path(&asset).unwrap();let preserved=std::fs::read(&blob).unwrap();std::fs::remove_file(&blob).unwrap();
    let failure=p.image_output_preview(view.binding,SelectionRegion{x:0,y:0,width:1,height:1},false,true,1,cancel()).await;
    assert!(matches!(failure,Err(WorkflowError::Memory{..})));
    // This is an owned fixture only; restore bytes before ordinary close.
    std::fs::write(blob,preserved).unwrap();p.close().await.unwrap();
}
#[tokio::test]
async fn offline_output_and_tonal_plans_use_visible_fences_and_accepted_wire_base() {
    let _serial=TESTS.lock().await;let root=temp();let host_path=root.path().join("host");let p=create_image_project(create(&host_path),cancel()).await.unwrap();p.close().await.unwrap();
    let host=vw_store::ProjectStore::open(&host_path).unwrap();let accepted=host.revision().unwrap();let checkpoint=host.checkpoint_bytes().unwrap();let host_device=host.host_device().clone();drop(host);
    let path=root.path().join("client");drop(vw_store::ProjectStore::create_authenticated_checkpoint(&path,&checkpoint,&host_device,&DeviceId::from_bytes([9;16]),TIME).unwrap());
    let p=open_project(path.to_string_lossy().into(),cancel()).await.unwrap();apply(&p,20,command()).await;
    let tone=ImageToneSettings{brightness:-0.1,contrast:0.0,input_black:0.0,input_white:1.0,gamma:1.0,output_black:0.0,output_white:1.0};
    apply(&p,21,ImageOutputCommand::SetTone{layer_id:id(32),object_id:id(33),settings:tone.clone(),enabled:true,opacity:1.0}).await;
    let visible=snapshot(&p).await;assert_eq!(visible.binding.host_seq,0);assert_eq!(visible.tones[0].settings,tone);
    let mut opts=options();opts.marked=false;let image=p.export_image(opts,cancel()).await.unwrap();let pixel=vw_raster::decode(&image.bytes,vw_raster::DecodeLimits::default()).unwrap().pixels.rgba8();
    assert!(pixel[0]<255);p.close().await.unwrap();
    let stored=vw_store::ProjectStore::open(&path).unwrap();let pending=stored.pending().unwrap();assert_eq!(pending.len(),2);
    assert!(pending.iter().all(|txn|txn.base_revision.as_ref()==Some(&accepted)));drop(stored);
    let p=open_project(path.to_string_lossy().into(),cancel()).await.unwrap();assert_eq!(snapshot(&p).await.binding,visible.binding);p.close().await.unwrap();
}
