#![allow(clippy::unwrap_used,clippy::expect_used)]
use std::{cell::Cell,fs::File,io::Read};
use vw_model::{DeviceId,Document,Id,Project};
use vw_proto::v1 as pb;
use vw_store::*;

fn id(n:u64)->Id{Id::from_parts(1_700_000_000_000+n,[7;10]).unwrap()}
fn device(n:u8)->DeviceId{DeviceId::from_bytes([n;16])}
fn temp()->tempfile::TempDir{if cfg!(target_os="android"){tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()}else{tempfile::tempdir().unwrap()}}
fn camera(n:f64)->LocalCamera{LocalCamera{center_x:n,center_y:-0.0,scale:1.234_567_890_123_456_7,rotation:0.314_159_265_358_979_3,viewport_width:3840.0,viewport_height:2160.0}}
fn project()->Project {
    let mut project=Project::new(id(1),"Local views".into(),device(1));
    for n in 10..34 {let doc=id(n);project.documents.insert(doc.clone(),Document{definition:pb::CreateDocument{document_id:Some(doc.to_proto()),
        kind:pb::DocumentKind::Image as i32,schema_version:1,title:format!("Document {n}"),..Default::default()},pages:vec![],created_at_ms:0});}
    project
}
fn fixture(root:&std::path::Path)->ProjectStore{ProjectStore::create(root,project(),device(1),0).unwrap()}
fn key(d:&DeviceId,doc:&Id)->String{format!("{LOCAL_VIEW_PREFIX}{}.{}",d.as_str(),doc.as_str())}
fn remember(store:&mut ProjectStore,dev:&DeviceId,doc:&Id,revision:u64,operation:u64,c:LocalCamera)->LocalViewSnapshot {
    store.change_local_view(dev,doc,revision,&id(operation),LocalViewChange::Remember(c),||false).unwrap()
}

#[test]
fn twenty_four_documents_and_two_devices_reopen_with_exact_camera_bits_without_canonical_changes() {
    let root=temp();let path=root.path().join("p");let mut store=fixture(&path);
    let revision=store.revision().unwrap();let checkpoint=store.checkpoint_bytes().unwrap();
    for n in 10..34 {for dev in [device(1),device(2)]{remember(&mut store,&dev,&id(n),0,100+n,camera(n as f64+if dev==device(1){0.0}else{0.5}));}}
    assert_eq!(store.revision().unwrap(),revision);assert_eq!(store.checkpoint_bytes().unwrap(),checkpoint);drop(store);
    let mut store=ProjectStore::open(&path).unwrap();
    for n in 10..34 {for dev in [device(1),device(2)] {
        let value=store.local_view(&dev,&id(n)).unwrap().camera.unwrap();let expected=camera(n as f64+if dev==device(1){0.0}else{0.5});
        assert_eq!(value,expected);assert_eq!(value.center_y.to_bits(),(-0.0_f64).to_bits());assert_eq!(value.scale.to_bits(),expected.scale.to_bits());
    }}
    assert_eq!(store.revision().unwrap(),revision);
}
#[test]
fn bookmarks_have_independent_names_and_cameras_and_local_edits_do_not_create_undo_history() {
    let root=temp();let mut store=fixture(&root.path().join("p"));let revision=store.revision().unwrap();
    let added=store.change_local_view(&device(1),&id(10),0,&id(100),LocalViewChange::AddBookmark{id:id(200),name:"Detail".into(),camera:camera(9.0)},||false).unwrap();
    let moved=remember(&mut store,&device(1),&id(10),added.revision,101,camera(11.0));
    assert_eq!(moved.bookmarks[0].camera,camera(9.0));
    let renamed=store.change_local_view(&device(1),&id(10),moved.revision,&id(102),LocalViewChange::RenameBookmark{id:id(200),name:"First detail".into()},||false).unwrap();
    assert_eq!(renamed.bookmarks[0].name,"First detail");
    let removed=store.change_local_view(&device(1),&id(10),renamed.revision,&id(103),LocalViewChange::DeleteBookmark{id:id(200)},||false).unwrap();
    assert!(removed.bookmarks.is_empty());assert_eq!(removed.camera,Some(camera(11.0)));assert_eq!(store.revision().unwrap(),revision);
    assert_eq!(store.accepted_transactions().count(),0);
}
#[test]
fn exact_latest_retry_is_idempotent_but_changed_or_intervening_retries_refuse() {
    let root=temp();let mut store=fixture(&root.path().join("p"));let expected=remember(&mut store,&device(1),&id(10),0,100,camera(1.0));
    assert_eq!(remember(&mut store,&device(1),&id(10),0,100,camera(1.0)),expected);
    assert!(matches!(store.change_local_view(&device(1),&id(10),0,&id(100),LocalViewChange::Remember(camera(2.0)),||false),Err(LocalViewError::Conflict)));
    remember(&mut store,&device(1),&id(10),1,101,camera(3.0));
    assert!(matches!(store.change_local_view(&device(1),&id(10),0,&id(100),LocalViewChange::Remember(camera(1.0)),||false),Err(LocalViewError::Conflict)));
    assert_eq!(store.local_view(&device(1),&id(10)).unwrap().camera,Some(camera(3.0)));
}
#[test]
fn retry_with_changed_expected_revision_refuses_without_altering_the_saved_request() {
    let root=temp();let path=root.path().join("p");let mut store=fixture(&path);
    let saved=remember(&mut store,&device(1),&id(10),0,100,camera(1.0));
    let revision=store.revision().unwrap();drop(store);
    let mut store=ProjectStore::open(&path).unwrap();
    for changed in [1,2,999] {
        assert!(matches!(store.change_local_view(&device(1),&id(10),changed,&id(100),
            LocalViewChange::Remember(camera(1.0)),||false),Err(LocalViewError::Conflict)));
        assert_eq!(store.local_view(&device(1),&id(10)).unwrap(),saved);
        assert_eq!(store.revision().unwrap(),revision);
    }
    assert_eq!(remember(&mut store,&device(1),&id(10),0,100,camera(1.0)),saved);
}
#[test]
fn cancellation_before_commit_rolls_back_and_retry_can_publish_once() {
    let root=temp();let mut store=fixture(&root.path().join("p"));let calls=Cell::new(0);
    let result=store.change_local_view(&device(1),&id(10),0,&id(100),LocalViewChange::Remember(camera(1.0)),||{let n=calls.get();calls.set(n+1);n>=2});
    assert!(matches!(result,Err(LocalViewError::Cancelled)));assert_eq!(store.local_view(&device(1),&id(10)).unwrap().revision,0);
    assert_eq!(remember(&mut store,&device(1),&id(10),0,100,camera(1.0)).revision,1);
}
#[test]
fn bounded_names_counts_and_invalid_camera_refuse_without_overwriting_prior_values() {
    let root=temp();let mut store=fixture(&root.path().join("p"));let mut revision=0;
    for n in 0..32 {revision=store.change_local_view(&device(1),&id(10),revision,&id(100+n),LocalViewChange::AddBookmark{id:id(200+n),name:format!("View {n}"),camera:camera(n as f64)},||false).unwrap().revision;}
    assert!(matches!(store.change_local_view(&device(1),&id(10),revision,&id(300),LocalViewChange::AddBookmark{id:id(400),name:"Extra".into(),camera:camera(0.0)},||false),Err(LocalViewError::Limit)));
    for name in [" leading".to_string(),"line\nbreak".to_string(),"é".repeat(129)] {
        assert!(matches!(store.change_local_view(&device(1),&id(10),revision,&id(301),LocalViewChange::RenameBookmark{id:id(200),name},||false),Err(LocalViewError::Invalid)));
    }
    assert!(matches!(store.change_local_view(&device(1),&id(10),revision,&id(302),LocalViewChange::Remember(LocalCamera{scale:f64::NAN,..camera(0.0)}),||false),Err(LocalViewError::Invalid)));
    assert_eq!(store.local_view(&device(1),&id(10)).unwrap().revision,revision);
}
#[test]
fn per_device_document_limit_preserves_existing_documents_and_other_devices() {
    let root=temp();let mut store=fixture(&root.path().join("p"));
    for n in 0..128 {remember(&mut store,&device(1),&id(1000+n),0,2000+n,camera(n as f64));}
    assert!(matches!(store.change_local_view(&device(1),&id(5000),0,&id(5001),LocalViewChange::Remember(camera(0.0)),||false),Err(LocalViewError::Limit)));
    assert_eq!(remember(&mut store,&device(1),&id(1000),1,5002,camera(2.0)).revision,2);
    assert_eq!(remember(&mut store,&device(2),&id(5000),0,5003,camera(2.0)).revision,1);
}
#[test]
fn overlong_nontext_or_malformed_settings_are_refused_and_retained() {
    let root=temp();let path=root.path().join("p");let mut store=fixture(&path);
    let sql=rusqlite::Connection::open(path.join("project.sqlite")).unwrap();let key=key(&device(1),&id(10));
    for value in ["{".to_string()," ".repeat(LOCAL_VIEW_MAX_VALUE_BYTES+1)] {
        sql.execute("INSERT OR REPLACE INTO settings(key,value) VALUES (?1,?2)",rusqlite::params![key,value]).unwrap();
        assert!(matches!(store.local_view(&device(1),&id(10)),Err(LocalViewError::Corrupt)));
        let actual:String=sql.query_row("SELECT value FROM settings WHERE key=?1",[&key],|r|r.get(0)).unwrap();assert_eq!(actual,value);
    }
    sql.execute("UPDATE settings SET value=zeroblob(16) WHERE key=?1",[&key]).unwrap();
    assert!(matches!(store.local_view(&device(1),&id(10)),Err(LocalViewError::Corrupt)));
}
#[test]
fn unexpected_live_trigger_refuses_before_mutation() {
    let root=temp();let path=root.path().join("p");let mut store=fixture(&path);
    let before=remember(&mut store,&device(1),&id(10),0,100,camera(1.0));
    let sql=rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    sql.execute_batch("CREATE TRIGGER sqlitex BEFORE UPDATE ON settings BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(matches!(store.change_local_view(&device(1),&id(10),1,&id(101),LocalViewChange::Remember(camera(2.0)),||false),Err(LocalViewError::Storage(_))));
    assert_eq!(store.local_view(&device(1),&id(10)).unwrap(),before);
    sql.execute_batch("DROP TRIGGER sqlitex").unwrap();
}
#[test]
fn oversized_live_schema_refuses_before_materializing_its_sql_in_the_local_workspace() {
    let root=temp();let path=root.path().join("p");let mut store=fixture(&path);
    let sql=rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    sql.execute_batch(&format!("CREATE TRIGGER sqlitex AFTER INSERT ON settings BEGIN SELECT '{}'; END;","a".repeat(70_000))).unwrap();
    assert!(matches!(store.change_local_view(&device(1),&id(10),0,&id(100),LocalViewChange::Remember(camera(1.0)),||false),Err(LocalViewError::Limit)));
    assert_eq!(store.local_view(&device(1),&id(10)).unwrap().revision,0);
    sql.execute_batch("DROP TRIGGER sqlitex").unwrap();
}
#[test]
fn archive_omits_local_camera_and_bookmark_bytes_but_preserves_unrelated_settings_and_source() {
    const PRIVATE:&str="SYNTHETIC_LOCAL_VIEW_NOT_FOR_ARCHIVE_06DB1461";
    let root=temp();let path=root.path().join("p");let mut store=fixture(&path);
    let saved=store.change_local_view(&device(1),&id(10),0,&id(100),LocalViewChange::AddBookmark{id:id(200),name:PRIVATE.into(),camera:camera(1.0)},||false).unwrap();
    let sql=rusqlite::Connection::open(path.join("project.sqlite")).unwrap();sql.execute("INSERT INTO settings(key,value) VALUES ('unrelated-fixture','retained')",[]).unwrap();drop(sql);
    let archive=root.path().join("export.vwbz");export_vwbz(&mut store,&archive).unwrap();
    assert_eq!(store.local_view(&device(1),&id(10)).unwrap(),saved);
    let mut zip=zip::ZipArchive::new(File::open(&archive).unwrap()).unwrap();
    let mut bytes=Vec::new();zip.by_name("project.sqlite").unwrap().read_to_end(&mut bytes).unwrap();
    assert!(!bytes.windows(PRIVATE.len()).any(|v|v==PRIVATE.as_bytes()));
    let copy=root.path().join("export.sqlite");std::fs::write(&copy,bytes).unwrap();
    let sql=rusqlite::Connection::open(copy).unwrap();
    assert_eq!(sql.query_row("SELECT COUNT(*) FROM settings WHERE key GLOB 'vw.local-view.v1.*'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(sql.query_row("SELECT value FROM settings WHERE key='unrelated-fixture'",[],|r|r.get::<_,String>(0)).unwrap(),"retained");
}
