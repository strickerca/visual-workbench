#![allow(clippy::unwrap_used,clippy::expect_used,clippy::panic)]
mod support;
use support::*;
use vw_core::*;

static TESTS:tokio::sync::Mutex<()>=tokio::sync::Mutex::const_new(());
fn temp()->tempfile::TempDir{if cfg!(target_os="android"){tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()}else{tempfile::tempdir().unwrap()}}
fn binding(device:String)->LocalViewsBinding{LocalViewsBinding{project_id:id(1),document_id:id(2),device_id:device}}
fn camera()->Camera{Camera{center:Point{x:12.125,y:-0.0},scale:1.234_567_890_123_456_7,rotation:0.314_159_265_358_979_3,viewport_width:400.0,viewport_height:700.0}}
const MEMORY:u64=1024*1024;

#[tokio::test]
async fn local_camera_and_bookmarks_survive_reopen_without_a_canonical_receipt_or_hash_change() {
    let _guard=TESTS.lock().await;let root=temp();let path=root.path().join("p");let project=create_image_project(create(&path),cancel()).await.unwrap();
    let before=project.info().await.unwrap();let selected=binding(before.device_id.clone());
    assert_eq!(project.local_views_snapshot(selected.clone(),MEMORY,cancel()).await.unwrap().revision,0);
    let saved=project.change_local_views(selected.clone(),0,id(50),LocalViewsMutation::Remember{camera:camera()},MEMORY,cancel()).await.unwrap();
    let bookmarked=project.change_local_views(selected.clone(),saved.revision,id(51),LocalViewsMutation::AddBookmark{id:id(52),name:"Corner".into(),camera:camera()},MEMORY,cancel()).await.unwrap();
    assert_eq!(project.info().await.unwrap(),before);project.close().await.unwrap();
    let project=open_project(path.to_string_lossy().into(),cancel()).await.unwrap();
    let snapshot=project.local_views_snapshot(selected,MEMORY,cancel()).await.unwrap();
    assert_eq!(snapshot.revision,bookmarked.revision);assert_eq!(snapshot.bookmarks.len(),1);
    let c=snapshot.camera.unwrap();assert_eq!(c.center.y.to_bits(),(-0.0_f64).to_bits());assert_eq!(c.scale.to_bits(),camera().scale.to_bits());
    assert_eq!(project.info().await.unwrap(),before);project.close().await.unwrap();
}
#[tokio::test]
async fn explicit_installation_identity_isolated_from_the_project_author_and_other_documents() {
    let _guard=TESTS.lock().await;let root=temp();let project=create_image_project(create(&root.path().join("p")),cancel()).await.unwrap();
    let a=binding(vw_model::DeviceId::from_bytes([21;16]).to_string());let b=binding(vw_model::DeviceId::from_bytes([22;16]).to_string());
    project.change_local_views(a.clone(),0,id(50),LocalViewsMutation::Remember{camera:camera()},MEMORY,cancel()).await.unwrap();
    assert_eq!(project.local_views_snapshot(b,MEMORY,cancel()).await.unwrap().revision,0);
    let mut wrong=a.clone();wrong.document_id=id(99);
    assert!(matches!(project.local_views_snapshot(wrong,MEMORY,cancel()).await,Err(WorkflowError::Missing)));
    let mut wrong=a;wrong.project_id=id(99);
    assert!(matches!(project.local_views_snapshot(wrong,MEMORY,cancel()).await,Err(WorkflowError::Detached)));
    project.close().await.unwrap();
}
#[tokio::test]
async fn budget_cancel_and_invalid_camera_admission_leave_the_settings_absent() {
    let _guard=TESTS.lock().await;let root=temp();let project=create_image_project(create(&root.path().join("p")),cancel()).await.unwrap();
    let selected=binding(project.info().await.unwrap().device_id);
    assert!(matches!(project.local_views_snapshot(selected.clone(),MEMORY-1,cancel()).await,Err(WorkflowError::Memory{..})));
    let cancelled=cancel();cancelled.cancel();
    assert!(matches!(project.change_local_views(selected.clone(),0,id(50),LocalViewsMutation::Remember{camera:camera()},MEMORY,cancelled).await,Err(WorkflowError::Cancelled)));
    let mut invalid=camera();invalid.scale=f64::NAN;
    assert!(matches!(project.change_local_views(selected.clone(),0,id(51),LocalViewsMutation::Remember{camera:invalid},MEMORY,cancel()).await,Err(WorkflowError::Invalid)));
    assert_eq!(project.local_views_snapshot(selected,MEMORY,cancel()).await.unwrap().revision,0);
    project.close().await.unwrap();
}
#[tokio::test]
async fn exact_ambiguous_retry_and_closed_borrowed_capability_have_explicit_outcomes() {
    let _guard=TESTS.lock().await;let root=temp();let project=create_image_project(create(&root.path().join("p")),cancel()).await.unwrap();
    let selected=binding(project.info().await.unwrap().device_id);
    let mutation=LocalViewsMutation::AddBookmark{id:id(51),name:"Saved".into(),camera:camera()};
    let first=project.change_local_views(selected.clone(),0,id(50),mutation.clone(),MEMORY,cancel()).await.unwrap();
    let retry=project.change_local_views(selected.clone(),0,id(50),mutation,MEMORY,cancel()).await.unwrap();
    assert_eq!(first.revision,retry.revision);assert_eq!(retry.bookmarks.len(),1);
    assert!(matches!(project.change_local_views(selected.clone(),0,id(50),LocalViewsMutation::DeleteBookmark{id:id(51)},MEMORY,cancel()).await,Err(WorkflowError::Stale)));
    project.close().await.unwrap();
    assert!(matches!(project.local_views_snapshot(selected,MEMORY,cancel()).await,Err(WorkflowError::Closed)));
}
