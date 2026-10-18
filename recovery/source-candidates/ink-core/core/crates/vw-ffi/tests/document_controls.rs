//! Real store/ops/FFI source regressions; central owner executes them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::sync::Arc;
use support::*;
use vw_core::*;
use vw_model::DeviceId;

const BUDGET: u64 = 256 * 1024 * 1024;
static TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn temp() -> tempfile::TempDir {
    if cfg!(target_os = "android") { tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap() }
    else { tempfile::tempdir().unwrap() }
}
fn metadata(n: u8, info: &ProjectInfo) -> WorkflowMetadata {
    WorkflowMetadata { transaction_id:id(n),device_id:info.device_id.clone(),first_lamport:info.next_lamport,created_at_ms:TIME+i64::from(n) }
}
async fn snapshot(project: &Arc<ProjectSession>) -> DocumentControlSnapshot {
    project.document_controls_snapshot(id(2),BUDGET,cancel()).await.unwrap()
}
async fn prepare(project: &Arc<ProjectSession>, n: u8, command: DocumentControlCommand) -> Arc<WorkflowPlan> {
    let view = snapshot(project).await;
    project.clone().prepare_document_control(view.binding,metadata(n,&view.revision),command,BUDGET,cancel()).await.unwrap()
}
async fn apply(project: &Arc<ProjectSession>, n: u8, command: DocumentControlCommand) -> WorkflowReceipt {
    let plan = prepare(project,n,command).await;
    let result = plan.commit(cancel()).await.unwrap(); plan.dispose(); result
}
fn update(layer: u8) -> LayerControlUpdate {
    LayerControlUpdate { layer_id:id(layer),name:None,visible:None,locked:None,opacity:None,blend:None }
}
fn targets(values: &[u8]) -> DocumentControlTargets { DocumentControlTargets::Objects { object_ids:values.iter().copied().map(id).collect() } }
fn line(n: u8) -> EditCommand {
    EditCommand::Create { object_id:id(n),layer_id:id(3),shape:NewShape::Line { points:vec![Point{x:4.0,y:f64::from(n)},Point{x:24.0,y:f64::from(n)}] },
        style:ObjectStyle {rgba:0x112233ff,width:2.0,screen_constant_width:false,fill:None},
        transform:Transform{a:1.0,b:0.0,c:0.0,d:1.0,e:0.0,f:0.0} }
}
async fn seed(project: &Arc<ProjectSession>) {
    project.apply_edit(edit(10,1),vec![line(11),line(12),line(13)],cancel()).await.unwrap();
}

#[tokio::test]
async fn layer_changes_are_immutable_retryable_undoable_and_persisted() {
    let _serial=TESTS.lock().await; let root=temp(); let path=root.path().join("p");
    let p=create_image_project(create(&path),cancel()).await.unwrap();
    let initial=p.info().await.unwrap();
    let plan=prepare(&p,20,DocumentControlCommand::CreateAnnotationLayer{layer_id:id(30),name:"Ink".into()}).await;
    assert_eq!(p.info().await.unwrap(),initial);
    let first=plan.commit(cancel()).await.unwrap(); let repeated=plan.commit(cancel()).await.unwrap();
    assert!(!first.duplicate);assert!(repeated.duplicate);assert_eq!(first.revision,repeated.revision);plan.dispose();
    apply(&p,21,DocumentControlCommand::UpdateLayer{update:LayerControlUpdate{name:Some("Highlights".into()),opacity:Some(0.5),blend:Some("multiply".into()),..update(30)}}).await;
    let before=p.info().await.unwrap();
    apply(&p,22,DocumentControlCommand::PlaceLayer{layer_id:id(30),before_layer_id:Some(id(3))}).await;
    assert_eq!(snapshot(&p).await.layers.iter().map(|v|v.layer_id.clone()).collect::<Vec<_>>(),vec![id(30),id(3)]);
    let now=p.info().await.unwrap();
    p.undo_redo(edit(23,now.next_lamport),false,cancel()).await.unwrap();
    assert_eq!(p.info().await.unwrap().state_hash,before.state_hash);
    p.close().await.unwrap();
    let reopened=open_project(path.to_string_lossy().into(),cancel()).await.unwrap();
    let layers=snapshot(&reopened).await.layers;
    assert_eq!(layers[1].name,"Highlights");assert_eq!(layers[1].opacity,0.5);assert_eq!(layers[1].blend,"multiply");
    reopened.close().await.unwrap();
}
#[tokio::test]
async fn locked_layer_refuses_visibility_reorder_delete_and_group_member_edits_atomically() {
    let _serial=TESTS.lock().await;let root=temp();let p=create_image_project(create(&root.path().join("p")),cancel()).await.unwrap();seed(&p).await;
    apply(&p,20,DocumentControlCommand::UpdateLayer{update:LayerControlUpdate{locked:Some(true),..update(3)}}).await;
    let view=snapshot(&p).await;
    let commands=vec![DocumentControlCommand::UpdateLayer{update:LayerControlUpdate{visible:Some(false),..update(3)}},
        DocumentControlCommand::PlaceLayer{layer_id:id(3),before_layer_id:None},
        DocumentControlCommand::DeleteEmptyLayer{layer_id:id(3)},
        DocumentControlCommand::SetObjectLocks{targets:targets(&[11,12]),locked:false},
        DocumentControlCommand::Group{group_id:id(30),member_ids:vec![id(11),id(12)]}];
    for (i,command) in commands.into_iter().enumerate() {
        assert!(matches!(p.clone().prepare_document_control(view.binding.clone(),metadata(40+i as u8,&view.revision),command,BUDGET,cancel()).await,Err(WorkflowError::Locked)));
        assert_eq!(p.info().await.unwrap(),view.revision);
    }
    apply(&p,50,DocumentControlCommand::UpdateLayer{update:LayerControlUpdate{locked:Some(false),..update(3)}}).await;
    p.close().await.unwrap();
}
#[tokio::test]
async fn nested_group_flags_move_and_ungroup_preserve_every_object_identity() {
    let _serial=TESTS.lock().await;let root=temp();let p=create_image_project(create(&root.path().join("p")),cancel()).await.unwrap();seed(&p).await;
    apply(&p,20,DocumentControlCommand::Group{group_id:id(30),member_ids:vec![id(11),id(12)]}).await;
    apply(&p,21,DocumentControlCommand::Group{group_id:id(31),member_ids:vec![id(30),id(13)]}).await;
    let whole=DocumentControlTargets::Group{group_id:id(31)};
    apply(&p,22,DocumentControlCommand::SetObjectHidden{targets:whole.clone(),hidden:true}).await;
    assert!(p.render_list(id(2),None,cancel()).await.unwrap().items.is_empty());
    assert!(snapshot(&p).await.objects.iter().all(|o|o.hidden));
    apply(&p,23,DocumentControlCommand::SetObjectHidden{targets:whole.clone(),hidden:false}).await;
    apply(&p,24,DocumentControlCommand::CreateAnnotationLayer{layer_id:id(32),name:"Moved".into()}).await;
    apply(&p,25,DocumentControlCommand::MoveObjects{targets:whole.clone(),layer_id:id(32)}).await;
    let view=snapshot(&p).await;
    assert_eq!(view.objects.iter().map(|v|v.object_id.clone()).collect::<Vec<_>>(),vec![id(11),id(12),id(13)]);
    assert!(view.objects.iter().all(|o|o.layer_id==id(32)));
    apply(&p,26,DocumentControlCommand::SetObjectLocks{targets:whole.clone(),locked:true}).await;
    let locked=snapshot(&p).await;
    assert!(matches!(p.clone().prepare_document_control(locked.binding,metadata(27,&locked.revision),DocumentControlCommand::Ungroup{group_id:id(31)},BUDGET,cancel()).await,Err(WorkflowError::Locked)));
    apply(&p,28,DocumentControlCommand::SetObjectLocks{targets:whole,locked:false}).await;
    apply(&p,29,DocumentControlCommand::Ungroup{group_id:id(31)}).await;
    let last=snapshot(&p).await;
    assert_eq!(last.groups.len(),1);assert_eq!(last.groups[0].group_id,id(30));assert_eq!(last.groups[0].parent_id,None);
    assert_eq!(last.objects.len(),3);p.close().await.unwrap();
}
#[tokio::test]
async fn twenty_independent_documents_share_original_bytes_and_reopen_with_distinct_titles() {
    let _serial=TESTS.lock().await;let root=temp();let path=root.path().join("p");let original=source();
    let p=create_image_project(create(&path),cancel()).await.unwrap();
    for n in 0..19u8 {
        apply(&p,20+n,DocumentControlCommand::CreateDocumentFromSource{document_id:id(60+n),layer_id:id(100+n),title:format!("Sheet {}",n+2)}).await;
    }
    assert_eq!(snapshot(&p).await.documents.len(),20);
    p.close().await.unwrap();
    let store=vw_store::ProjectStore::open(&path).unwrap();
    assert_eq!(store.project().documents.len(),20);assert_eq!(store.project().assets.len(),1);assert_eq!(store.project().layers.len(),20);
    let source_id=store.project().assets.keys().next().unwrap();
    assert_eq!(vw_store::BlobStore::new(&path).unwrap().read(source_id).unwrap(),original);
    let expected=store.project().state_hash().unwrap().to_string();drop(store);
    let reopened=open_project(path.to_string_lossy().into(),cancel()).await.unwrap();
    assert_eq!(reopened.info().await.unwrap().state_hash,expected);
    let view=reopened.document_controls_snapshot(id(78),BUDGET,cancel()).await.unwrap();
    assert_eq!(view.documents.len(),20);assert_eq!(view.layers.len(),1);assert!(view.objects.is_empty());
    reopened.close().await.unwrap();
}
#[tokio::test]
async fn stale_plan_cancelled_plan_and_small_budget_leave_the_accepted_state_unchanged() {
    let _serial=TESTS.lock().await;let root=temp();let p=create_image_project(create(&root.path().join("p")),cancel()).await.unwrap();
    let one=prepare(&p,20,DocumentControlCommand::RenameDocument{title:"First".into()}).await;
    let old=prepare(&p,21,DocumentControlCommand::RenameDocument{title:"Stale".into()}).await;
    let first=one.commit(cancel()).await.unwrap();
    assert!(matches!(old.commit(cancel()).await,Err(WorkflowError::Stale)));one.dispose();old.dispose();
    let cancelled=prepare(&p,22,DocumentControlCommand::RenameDocument{title:"Cancelled".into()}).await;
    let token=cancel();token.cancel();assert!(matches!(cancelled.commit(token).await,Err(WorkflowError::Cancelled)));cancelled.dispose();
    let view=snapshot(&p).await;
    assert!(matches!(p.clone().prepare_document_control(view.binding,metadata(23,&view.revision),DocumentControlCommand::RenameDocument{title:"Budget".into()},1,cancel()).await,Err(WorkflowError::Memory{..})));
    assert_eq!(p.info().await.unwrap(),first.revision);p.close().await.unwrap();
}
#[tokio::test]
async fn offline_controls_use_visible_cas_but_keep_the_accepted_transaction_base() {
    let _serial=TESTS.lock().await;let root=temp();let host_path=root.path().join("host");
    let p=create_image_project(create(&host_path),cancel()).await.unwrap();p.close().await.unwrap();
    let host=vw_store::ProjectStore::open(&host_path).unwrap();let accepted=host.revision().unwrap();
    let checkpoint=host.checkpoint_bytes().unwrap();let host_device=host.host_device().clone();drop(host);
    let path=root.path().join("client");drop(vw_store::ProjectStore::create_authenticated_checkpoint(&path,&checkpoint,&host_device,&DeviceId::from_bytes([9;16]),TIME).unwrap());
    let p=open_project(path.to_string_lossy().into(),cancel()).await.unwrap();
    for (n,title) in [(20,"Offline first"),(21,"Offline second")] {
        let receipt=apply(&p,n,DocumentControlCommand::RenameDocument{title:title.into()}).await;assert_eq!(receipt.revision.host_seq,0);
    }
    let visible=snapshot(&p).await.binding;p.close().await.unwrap();
    let stored=vw_store::ProjectStore::open(&path).unwrap();let pending=stored.pending().unwrap();
    assert_eq!(pending.len(),2);assert!(pending.iter().all(|t|t.base_revision.as_ref()==Some(&accepted)));drop(stored);
    let p=open_project(path.to_string_lossy().into(),cancel()).await.unwrap();assert_eq!(snapshot(&p).await.binding,visible);p.close().await.unwrap();
}
#[tokio::test]
async fn cross_document_members_duplicate_ids_and_populated_layer_deletion_refuse() {
    let _serial=TESTS.lock().await;let root=temp();let p=create_image_project(create(&root.path().join("p")),cancel()).await.unwrap();seed(&p).await;
    apply(&p,20,DocumentControlCommand::CreateDocumentFromSource{document_id:id(30),layer_id:id(31),title:"Second".into()}).await;
    let view=snapshot(&p).await;
    for (n,command) in [(21,DocumentControlCommand::MoveObjects{targets:targets(&[11]),layer_id:id(31)}),
        (22,DocumentControlCommand::Group{group_id:id(32),member_ids:vec![id(11),id(11)]}),
        (23,DocumentControlCommand::DeleteEmptyLayer{layer_id:id(3)})] {
        assert!(matches!(p.clone().prepare_document_control(view.binding.clone(),metadata(n,&view.revision),command,BUDGET,cancel()).await,Err(WorkflowError::Invalid)));
        assert_eq!(p.info().await.unwrap(),view.revision);
    }
    p.close().await.unwrap();
}
