#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::{CreateImageProject, ProjectSession, WorkflowBinding, create_image_project};
use vw_model::{DeviceId, Id};
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn id(n: u8) -> String {
    Id::from_parts(1, [n; 10]).unwrap().to_string()
}
fn cancel() -> Arc<Cancellation> {
    Arc::new(Cancellation::new())
}
fn temp() -> tempfile::TempDir {
    tempfile::tempdir_in(if cfg!(target_os = "android") {
        std::env::current_dir().unwrap()
    } else {
        std::env::temp_dir()
    })
    .unwrap()
}
async fn fixture(root: &Path) -> (Arc<ProjectSession>, PackageCompileOptions) {
    let mut bytes = vec![];
    {
        let mut e = png::Encoder::new(&mut bytes, 8, 8);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut w = e.write_header().unwrap();
        w.write_image_data(&[31, 63, 127, 255].repeat(64)).unwrap();
        w.finish().unwrap();
    }
    let p = create_image_project(
        CreateImageProject {
            path: root.join("project").to_string_lossy().into(),
            project_id: id(1),
            document_id: id(2),
            layer_id: id(3),
            device_id: DeviceId::from_bytes([1; 16]).to_string(),
            title: "Package fixture".into(),
            source: bytes,
            now_ms: 1,
        },
        cancel(),
    )
    .await
    .unwrap();
    let info = p.info().await.unwrap();
    let options = PackageCompileOptions {
        binding: WorkflowBinding {
            project_id: info.project_id,
            document_id: id(2),
            host_seq: info.host_seq,
            state_hash: info.state_hash,
        },
        package_id: id(4),
        created_at_ms: 10,
        target: PackageTarget::Generic { max_long_edge: 512 },
        semantic_snapshot_id: None,
        include_window_title: false,
        assume_untagged_srgb: false,
        allow_depth_reduction: false,
        memory_budget_bytes: MAX_MEMORY,
    };
    (p, options)
}
#[tokio::test]
async fn compile_publish_retry_reopen_read_and_retire_are_exact() {
    let _serial = SERIAL.lock().await;
    let root = temp();
    let (p, o) = fixture(root.path()).await;
    let before = p.info().await.unwrap();
    let compiled = p.compile_package(o, cancel()).await.unwrap();
    let info = compiled.describe().unwrap();
    assert_eq!(info.binding.state_hash, before.state_hash);
    assert_eq!(info.images.len(), 2);
    assert_eq!(info.marker_count, 0);
    let catalog = open_package_catalog(root.path().to_str().unwrap().into(), cancel())
        .await
        .unwrap();
    let published = catalog.publish(compiled.clone(), cancel()).await.unwrap();
    assert_eq!(
        catalog.publish(compiled.clone(), cancel()).await.unwrap(),
        published
    );
    let json = catalog
        .read_file(
            info.package_id.clone(),
            info.target.clone(),
            info.manifest_sha256.clone(),
            "manifest.json".into(),
            4 * 1024 * 1024,
            cancel(),
        )
        .await
        .unwrap();
    let m: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(m["extensions"]["state_hash"], before.state_hash);
    assert_eq!(p.info().await.unwrap(), before);
    compiled.dispose();
    drop(compiled);
    catalog.shutdown().await.unwrap();
    drop(catalog);
    let catalog = open_package_catalog(root.path().to_str().unwrap().into(), cancel())
        .await
        .unwrap();
    assert_eq!(catalog.list(cancel()).await.unwrap(), vec![published]);
    let retired = catalog
        .retire(info.package_id, info.target, info.manifest_sha256, cancel())
        .await
        .unwrap();
    assert_eq!(retired.files_removed, cfg!(windows));
    assert!(catalog.list(cancel()).await.unwrap().is_empty());
    assert_eq!(
        catalog
            .pending_retirement(cancel())
            .await
            .unwrap()
            .is_none(),
        cfg!(windows)
    );
    catalog.shutdown().await.unwrap();
    drop(catalog);
    p.close().await.unwrap();
}
#[tokio::test]
async fn a_package_id_cannot_rebind_published_bytes_or_bypass_manifest_hash() {
    let _serial = SERIAL.lock().await;
    let root = temp();
    let (p, o) = fixture(root.path()).await;
    let a = p.compile_package(o.clone(), cancel()).await.unwrap();
    let catalog = open_package_catalog(root.path().to_str().unwrap().into(), cancel())
        .await
        .unwrap();
    let published = catalog.publish(a.clone(), cancel()).await.unwrap();
    let mut changed = o;
    changed.created_at_ms += 1;
    let b = p.compile_package(changed, cancel()).await.unwrap();
    assert!(matches!(
        catalog.publish(b.clone(), cancel()).await,
        Err(PackageError::Identity)
    ));
    assert!(matches!(
        catalog
            .lookup(
                published.info.package_id.clone(),
                published.info.target.clone(),
                "0".repeat(64),
                cancel()
            )
            .await,
        Err(PackageError::Identity)
    ));
    assert!(matches!(
        catalog
            .read_file(
                published.info.package_id,
                published.info.target,
                published.info.manifest_sha256,
                "../owner-v1".into(),
                4096,
                cancel()
            )
            .await,
        Err(PackageError::Integrity)
    ));
    b.dispose();
    a.dispose();
    catalog.shutdown().await.unwrap();
    p.close().await.unwrap();
}
#[tokio::test]
async fn cancelled_or_stale_compilation_never_publishes_and_does_not_exhaust_handles() {
    let _serial = SERIAL.lock().await;
    let root = temp();
    let (p, o) = fixture(root.path()).await;
    let token = cancel();
    token.cancel();
    assert!(matches!(
        p.compile_package(o.clone(), token).await,
        Err(PackageError::Cancelled)
    ));
    let mut stale = o.clone();
    stale.binding.state_hash = "a".repeat(64);
    assert!(matches!(
        p.compile_package(stale, cancel()).await,
        Err(PackageError::Stale)
    ));
    let a = p.compile_package(o.clone(), cancel()).await.unwrap();
    let b = p.compile_package(o.clone(), cancel()).await.unwrap();
    assert!(matches!(
        p.compile_package(o.clone(), cancel()).await,
        Err(PackageError::Busy)
    ));
    a.dispose();
    b.dispose();
    let c = p.compile_package(o, cancel()).await.unwrap();
    c.dispose();
    assert!(!root.path().join("packages-v1").exists());
    p.close().await.unwrap();
}
#[tokio::test]
async fn source_or_published_file_tampering_is_refused() {
    let _serial = SERIAL.lock().await;
    let root = temp();
    let (p, o) = fixture(root.path()).await;
    let compiled = p.compile_package(o.clone(), cancel()).await.unwrap();
    let catalog = open_package_catalog(root.path().to_str().unwrap().into(), cancel())
        .await
        .unwrap();
    let published = catalog.publish(compiled.clone(), cancel()).await.unwrap();
    fs::write(
        Path::new(&published.directory).join("prompt.md"),
        b"replaced",
    )
    .unwrap();
    assert!(matches!(
        catalog
            .lookup(
                published.info.package_id,
                published.info.target,
                published.info.manifest_sha256,
                cancel()
            )
            .await,
        Err(PackageError::Integrity) | Err(PackageError::Invalid)
    ));
    p.worker
        .call(|state| {
            let project = state.project()?;
            let a = project.assets.values().next().unwrap();
            let asset = vw_model::AssetId::try_from(a.asset_id.clone())?;
            let path = state.blobs.path(&asset)?;
            let n = fs::metadata(&path).unwrap().len() as usize;
            fs::write(path, vec![0u8; n]).unwrap();
            Ok(())
        })
        .await
        .unwrap();
    assert!(matches!(
        p.compile_package(o, cancel()).await,
        Err(PackageError::Original)
    ));
    compiled.dispose();
    catalog.shutdown().await.unwrap();
    p.close().await.unwrap();
}
#[tokio::test]
async fn catalog_lock_and_missing_index_fail_closed() {
    let _serial = SERIAL.lock().await;
    let root = temp();
    let catalog = open_package_catalog(root.path().to_str().unwrap().into(), cancel())
        .await
        .unwrap();
    assert!(matches!(
        open_package_catalog(root.path().to_str().unwrap().into(), cancel()).await,
        Err(PackageError::Busy)
    ));
    catalog.shutdown().await.unwrap();
    drop(catalog);
    fs::remove_file(root.path().join("packages-v1/index-v1.json")).unwrap();
    assert!(matches!(
        open_package_catalog(root.path().to_str().unwrap().into(), cancel()).await,
        Err(PackageError::Integrity)
    ));
    assert!(!root.path().join("packages-v1/index-v1.json").exists());
}
fn interrupted_retirement(state: &mut State, prepared: &compile::Prepared) -> (String, String) {
    let info = dto::describe(&prepared.package).unwrap();
    let k = key(&info.package_id, &info.target).unwrap();
    let entry = state.entries[&k].clone();
    let retiring = Retiring {
        entry,
        files: prepared
            .package
            .files()
            .map(|(name, bytes)| RetiredFile {
                name: name.into(),
                bytes: bytes.len() as u64,
                blake3: blake3::hash(bytes).to_hex().to_string(),
            })
            .collect(),
    };
    state
        .persist(&BTreeMap::new(), Some(retiring.clone()))
        .unwrap();
    state.entries.clear();
    state.retiring = Some(retiring);
    fs::remove_file(state.root.join(&k).join("prompt.md")).unwrap();
    (k, info.manifest_sha256)
}
#[tokio::test]
async fn partial_retirement_reopens_with_exact_inventory_and_can_finish() {
    let _serial = SERIAL.lock().await;
    let root = temp();
    let (p, o) = fixture(root.path()).await;
    let compiled = p.compile_package(o, cancel()).await.unwrap();
    let prepared = compiled.get().unwrap();
    let mut state = State::open(
        root.path().to_str().unwrap(),
        &Cancellation::new(),
        Permit::acquire().unwrap(),
    )
    .unwrap();
    state.publish(&prepared, &Cancellation::new()).unwrap();
    let (k, hash) = interrupted_retirement(&mut state, &prepared);
    drop(state);
    let mut state = State::open(
        root.path().to_str().unwrap(),
        &Cancellation::new(),
        Permit::acquire().unwrap(),
    )
    .unwrap();
    assert!(state.entries.is_empty());
    assert!(state.retiring.is_some());
    assert_eq!(
        state
            .retire(id(4), "generic".into(), hash, &Cancellation::new())
            .unwrap()
            .files_removed,
        cfg!(windows)
    );
    assert_eq!(state.root.join(k).exists(), !cfg!(windows));
    drop(state);
    drop(prepared);
    compiled.dispose();
    p.close().await.unwrap();
}
#[tokio::test]
async fn retirement_preserves_unrecognized_or_changed_remaining_bytes() {
    let _serial = SERIAL.lock().await;
    let root = temp();
    let (p, o) = fixture(root.path()).await;
    let compiled = p.compile_package(o, cancel()).await.unwrap();
    let prepared = compiled.get().unwrap();
    let mut state = State::open(
        root.path().to_str().unwrap(),
        &Cancellation::new(),
        Permit::acquire().unwrap(),
    )
    .unwrap();
    state.publish(&prepared, &Cancellation::new()).unwrap();
    let (k, hash) = interrupted_retirement(&mut state, &prepared);
    let unowned = state.root.join(&k).join("keep.txt");
    fs::write(&unowned, b"preserve").unwrap();
    assert!(
        !state
            .retire(id(4), "generic".into(), hash.clone(), &Cancellation::new())
            .unwrap()
            .files_removed
    );
    assert_eq!(fs::read(&unowned).unwrap(), b"preserve");
    assert!(state.root.join(&k).join("manifest.json").exists());
    fs::remove_file(unowned).unwrap();
    let changed = state.root.join(&k).join("semantic.json");
    fs::write(&changed, b"changed").unwrap();
    assert!(
        !state
            .retire(id(4), "generic".into(), hash, &Cancellation::new())
            .unwrap()
            .files_removed
    );
    assert_eq!(fs::read(changed).unwrap(), b"changed");
    assert!(state.retiring.is_some());
    drop(state);
    drop(prepared);
    compiled.dispose();
    p.close().await.unwrap();
}

#[tokio::test]
async fn uncertain_post_replace_failure_poison_closes_stale_cache_until_reopen() {
    let _serial = SERIAL.lock().await;
    let root = temp();
    let (p, o) = fixture(root.path()).await;
    let compiled = p.compile_package(o, cancel()).await.unwrap();
    let prepared = compiled.get().unwrap();
    let mut state = State::open(
        root.path().to_str().unwrap(),
        &Cancellation::new(),
        Permit::acquire().unwrap(),
    )
    .unwrap();
    state.publish(&prepared, &Cancellation::new()).unwrap();
    let info = compiled.describe().unwrap();
    FAIL_AFTER_REPLACE.store(true, Ordering::Release);
    assert!(matches!(
        state.retire(
            info.package_id.clone(),
            info.target.clone(),
            info.manifest_sha256.clone(),
            &Cancellation::new()
        ),
        Err(PackageError::Storage)
    ));
    assert!(state.poisoned);
    assert!(matches!(
        state.publish(&prepared, &Cancellation::new()),
        Err(PackageError::Storage)
    ));
    assert!(matches!(state.usage(), Err(PackageError::Storage)));
    drop(state);
    let state = State::open(
        root.path().to_str().unwrap(),
        &Cancellation::new(),
        Permit::acquire().unwrap(),
    )
    .unwrap();
    assert!(state.entries.is_empty());
    assert_eq!(
        state.retiring.as_ref().unwrap().entry.manifest_sha256,
        info.manifest_sha256
    );
    drop(state);
    drop(prepared);
    compiled.dispose();
    p.close().await.unwrap();
}
#[cfg(windows)]
#[tokio::test]
async fn verified_files_and_ancestor_identity_remain_pinned_through_deletion() {
    let _serial = SERIAL.lock().await;
    let root = temp();
    let (p, o) = fixture(root.path()).await;
    let compiled = p.compile_package(o, cancel()).await.unwrap();
    let prepared = compiled.get().unwrap();
    let mut state = State::open(
        root.path().to_str().unwrap(),
        &Cancellation::new(),
        Permit::acquire().unwrap(),
    )
    .unwrap();
    state.publish(&prepared, &Cancellation::new()).unwrap();
    *RETIRE_HOOK.lock().unwrap() = Some(Box::new(|package| {
        assert!(fs::write(package.join("prompt.md"), b"late replacement").is_err());
        assert!(fs::rename(package.join("prompt.md"), package.join("moved.md")).is_err());
        assert!(fs::rename(package, package.with_extension("moved")).is_err());
        let root = package.parent().unwrap();
        assert!(fs::rename(root, root.with_extension("moved")).is_err());
    }));
    let info = compiled.describe().unwrap();
    assert!(
        state
            .retire(
                info.package_id,
                info.target,
                info.manifest_sha256,
                &Cancellation::new()
            )
            .unwrap()
            .files_removed
    );
    assert!(RETIRE_HOOK.lock().unwrap().is_none());
    drop(state);
    drop(prepared);
    compiled.dispose();
    p.close().await.unwrap();
}
