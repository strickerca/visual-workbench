//! Deterministic ownership/admission cases. Synthetic completion is confined
//! here; these fixtures do not establish execution of an approved OS producer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use std::sync::{Mutex, MutexGuard};
use vw_assets::{Manifest, TileRecord};
use vw_store::{DiskSpace, FreeSpaceQuery};
use vw_stream::{MemoryKind, SessionBudget};

static SERIAL: Mutex<()> = Mutex::new(());
fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|error| error.into_inner())
}
#[derive(Clone, Copy)]
struct Space(u64);
impl FreeSpaceQuery for Space {
    fn query(&self, _: &Path) -> std::result::Result<DiskSpace, vw_store::StoreError> {
        Ok(DiskSpace {
            total_bytes: 1024 * 1024 * 1024 * 1024,
            available_bytes: self.0,
        })
    }
}
fn plentiful() -> Space {
    Space(1024 * 1024 * 1024 * 1024)
}
fn png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[21, 42, 84])
            .unwrap();
    }
    out
}
fn request(bytes: &[u8]) -> (RootOwnedPickedOriginal, GiantInputLimits) {
    use std::io::{Seek, SeekFrom, Write};
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(bytes).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    (
        RootOwnedPickedOriginal {
            file,
            work: tempfile::tempdir().unwrap(),
        },
        GiantInputLimits {
            max_original_bytes: 4 * 1024 * 1024,
            max_work_disk_bytes: 512 * 1024 * 1024,
            assume_untagged_srgb: true,
        },
    )
}
struct Fixture {
    source: Arc<input::PreparedSource>,
    budget: SessionBudget,
}
fn fixture() -> Fixture {
    fixture_with_budget(vw_stream::DESKTOP_SESSION_BYTES)
}
fn fixture_with_budget(bytes: u64) -> Fixture {
    let (owned, limits) = request(&png());
    let budget = SessionBudget::new(bytes).unwrap();
    let source = input::prepare(
        owned,
        limits,
        &AtomicBool::new(false),
        &Cancellation::new(),
        &budget,
        &plentiful(),
    )
    .unwrap();
    Fixture { source, budget }
}
fn tile() -> Vec<u8> {
    let image = vw_raster::decode(&png(), vw_raster::DecodeLimits::default()).unwrap();
    let exported = vw_raster::export(
        &image,
        &vw_raster::ExportRequest {
            format: vw_raster::ExportFormat::WebpLossless,
            region: None,
            revision: vw_proto::v1::Revision {
                host_seq: 0,
                state_hash: vec![0; 32],
            },
            alpha: vw_raster::AlphaPolicy::Preserve,
            color: vw_raster::ColorPolicy::Preserve,
            allow_depth_reduction: false,
            memory_budget_bytes: 64 * 1024 * 1024,
            capture_session: None,
            frame_id: None,

            capture_provenance: None,
        },
    )
    .unwrap()
    .bytes;
    let mut at = 12;
    while at + 8 <= exported.len() {
        let len = u32::from_le_bytes(exported[at + 4..at + 8].try_into().unwrap()) as usize;
        if &exported[at..at + 4] == b"VP8L" {
            let mut result = b"RIFF".to_vec();
            result.extend_from_slice(&((12 + len + len % 2) as u32).to_le_bytes());
            result.extend_from_slice(b"WEBP");
            result.extend_from_slice(&exported[at..at + 8 + len + len % 2]);
            return result;
        }
        at += 8 + len + len % 2;
    }
    panic!("missing fixture VP8L");
}
fn synthetic(fixture: &Fixture) -> cache::ApprovedBuild {
    let _owner = source::reserve(
        &fixture.budget,
        MemoryKind::Transfer,
        super::super::tiles::MANIFEST_WORKSPACE,
    )
    .unwrap();
    let encoded = tile();
    let key = fixture
        .source
        .source
        .validate()
        .unwrap()
        .keys()
        .next()
        .unwrap();
    let manifest = Manifest::new(
        fixture.source.source.clone(),
        vec![TileRecord {
            key,
            encoded_bytes: encoded.len() as u32,
            hash: *blake3::hash(&encoded).as_bytes(),
        }],
    )
    .unwrap();
    let temporary = tempfile::Builder::new()
        .prefix("fixture-output-")
        .tempdir_in(fixture.source.staging.path())
        .unwrap();
    std::fs::create_dir(temporary.path().join("pyramid")).unwrap();
    std::fs::create_dir(temporary.path().join("pyramid/L0")).unwrap();
    std::fs::write(
        temporary
            .path()
            .join("pyramid")
            .join(manifest.tiles()[0].relative_path()),
        encoded,
    )
    .unwrap();
    std::fs::write(
        temporary.path().join("pyramid/pyramid.vwt"),
        manifest.encode(),
    )
    .unwrap();
    let directory_charge =
        source::reserve(&fixture.budget, MemoryKind::Transfer, 1024 * 1024).unwrap();
    cache::ApprovedBuild {
        manifest,
        output: cleanup::OwnedDirectory::new(temporary, (fixture.source.clone(), directory_charge)),
        source: fixture.source.clone(),
        disk_cache_bytes: 32 * 1024 * 1024,
        platform: "test-fixture-only",
        _manifest_charge: _owner,
    }
}
#[test]
fn preparation_requires_explicit_untagged_srgb_consent_before_io() {
    let _guard = serial();
    let (owned, mut limits) = request(&png());
    limits.assume_untagged_srgb = false;
    let budget = SessionBudget::new(1).unwrap();
    assert!(matches!(
        input::prepare(
            owned,
            limits,
            &AtomicBool::new(false),
            &Cancellation::new(),
            &budget,
            &plentiful()
        ),
        Err(Error::Unsupported)
    ));
    assert_eq!(budget.used_bytes(), 0);
}
#[test]
fn preparation_reserves_shared_owner_before_source_io() {
    let _guard = serial();
    let (owned, limits) = request(&png());
    let budget = SessionBudget::new(1024).unwrap();
    assert!(matches!(
        input::prepare(
            owned,
            limits,
            &AtomicBool::new(false),
            &Cancellation::new(),
            &budget,
            &plentiful()
        ),
        Err(Error::Memory { .. })
    ));
    assert_eq!(budget.used_bytes(), 0);
}
#[test]
fn streamed_private_input_preserves_original_hash_and_exact_bytes() {
    let _guard = serial();
    let fixture = fixture();
    let expected = png();
    assert_eq!(
        std::fs::read(&fixture.source.info.source_path).unwrap(),
        expected
    );
    assert_eq!(
        fixture.source.source.original,
        *blake3::hash(&expected).as_bytes()
    );
    assert_eq!(fixture.source.info.asset_id, fixture.source.source.original);
    assert_eq!(fixture.source.info.encoded_bytes, expected.len() as u64);
    assert_eq!(fixture.source.info.sha256.len(), 32);
    fixture
        .source
        .verify(
            &AtomicBool::new(false),
            &Cancellation::new(),
            &fixture.budget,
        )
        .unwrap();
}
#[test]
fn preparation_low_disk_refuses_and_releases_owner() {
    let _guard = serial();
    let (owned, limits) = request(&png());
    let path = owned.work.path().to_path_buf();
    let budget = SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap();
    assert!(matches!(
        input::prepare(
            owned,
            limits,
            &AtomicBool::new(false),
            &Cancellation::new(),
            &budget,
            &Space(0)
        ),
        Err(Error::Limit)
    ));
    assert!(!path.exists());
    assert_eq!(budget.used_bytes(), 0);
}
#[test]
fn preparation_work_cap_accounts_for_scratch_and_publication() {
    let _guard = serial();
    let (owned, mut limits) = request(&png());
    let path = owned.work.path().to_path_buf();
    limits.max_work_disk_bytes = 1024;
    let budget = SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap();
    assert!(matches!(
        input::prepare(
            owned,
            limits,
            &AtomicBool::new(false),
            &Cancellation::new(),
            &budget,
            &plentiful()
        ),
        Err(Error::Limit)
    ));
    assert!(!path.exists());
    assert_eq!(budget.used_bytes(), 0);
}
#[test]
fn closed_and_cancelled_preparation_never_creates_input() {
    let _guard = serial();
    let budget = SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap();
    let cancel = Cancellation::new();
    let (owned, limits) = request(&png());
    assert!(matches!(
        input::prepare(
            owned,
            limits,
            &AtomicBool::new(true),
            &cancel,
            &budget,
            &plentiful()
        ),
        Err(Error::Closed)
    ));
    let (owned, limits) = request(&png());
    cancel.cancel();
    assert!(matches!(
        input::prepare(
            owned,
            limits,
            &AtomicBool::new(false),
            &cancel,
            &budget,
            &plentiful()
        ),
        Err(Error::Cancelled)
    ));
    assert_eq!(budget.used_bytes(), 0);
}
#[test]
fn prepared_cleanup_waits_for_last_actual_source_owner() {
    let _guard = serial();
    let Fixture { source, budget } = fixture();
    let path = source.staging.path().to_path_buf();
    let retained = source.clone();
    input::release(source).unwrap();
    assert!(path.exists());
    assert!(budget.used_bytes() > 0);
    input::release(retained).unwrap();
    assert!(!path.exists());
    assert_eq!(budget.used_bytes(), 0);
}
#[test]
fn private_inventory_alone_never_grants_approval() {
    let _guard = serial();
    let fixture = fixture();
    let build = synthetic(&fixture);
    let root = tempfile::tempdir().unwrap();
    assert!(matches!(
        cache::approved(
            root.path(),
            &build.manifest,
            &fixture.budget,
            &Cancellation::new()
        ),
        Err(Error::Unsupported)
    ));
}
#[test]
fn approval_is_bound_to_root_manifest_original_and_same_pool() {
    let _guard = serial();
    let fixture = fixture();
    let build = synthetic(&fixture);
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let cancel = Cancellation::new();
    cache::register(root.path(), &build, &fixture.budget, &cancel).unwrap();
    cache::approved(root.path(), &build.manifest, &fixture.budget, &cancel).unwrap();
    assert!(matches!(
        cache::approved(other.path(), &build.manifest, &fixture.budget, &cancel),
        Err(Error::Unsupported)
    ));
    assert!(matches!(
        cache::approved(
            root.path(),
            &build.manifest,
            &SessionBudget::new(vw_stream::DESKTOP_SESSION_BYTES).unwrap(),
            &cancel
        ),
        Err(Error::Unsupported)
    ));
    let mut records = build.manifest.tiles().to_vec();
    records[0].hash[0] ^= 1;
    let changed = Manifest::new(build.manifest.source().clone(), records).unwrap();
    assert!(matches!(
        cache::approved(root.path(), &changed, &fixture.budget, &cancel),
        Err(Error::Unsupported)
    ));
    let mut identity = build.manifest.source().clone();
    identity.original[0] ^= 1;
    let changed = Manifest::new(identity, build.manifest.tiles().to_vec()).unwrap();
    assert!(matches!(
        cache::approved(root.path(), &changed, &fixture.budget, &cancel),
        Err(Error::Unsupported)
    ));
}
#[test]
fn output_damage_is_rejected_before_any_cache_mutation() {
    let _guard = serial();
    let fixture = fixture();
    let build = synthetic(&fixture);
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        build
            .directory()
            .join(build.manifest.tiles()[0].relative_path()),
        [0u8; 1],
    )
    .unwrap();
    assert!(
        cache::install(
            root.path(),
            &build,
            plentiful(),
            &AtomicBool::new(false),
            None,
            &Cancellation::new(),
            &fixture.budget
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
#[test]
fn output_unlisted_file_is_rejected() {
    let _guard = serial();
    let fixture = fixture();
    let build = synthetic(&fixture);
    std::fs::write(build.directory().join("L0/extra.webp"), [0u8; 4]).unwrap();
    assert!(matches!(
        cache::verify_output(
            &build,
            &AtomicBool::new(false),
            None,
            &Cancellation::new(),
            &fixture.budget
        ),
        Err(Error::Invalid)
    ));
}
#[test]
fn installed_fixture_requires_final_admission_before_scoped_open() {
    let _guard = serial();
    let fixture = fixture();
    let build = synthetic(&fixture);
    let root = tempfile::tempdir().unwrap();
    let cancel = Cancellation::new();
    cache::install(
        root.path(),
        &build,
        plentiful(),
        &AtomicBool::new(false),
        None,
        &cancel,
        &fixture.budget,
    )
    .unwrap();
    let mut disk =
        vw_store::DiskCache::with_query(root.path(), build.disk_cache_bytes, plentiful()).unwrap();
    assert!(matches!(
        super::super::tiles::manifest_scoped(
            &mut disk,
            root.path(),
            &fixture.source.source,
            &cancel,
            &fixture.budget
        ),
        Err(Error::Unsupported)
    ));
    cache::register(root.path(), &build, &fixture.budget, &cancel).unwrap();
    let admitted = super::super::tiles::manifest_scoped(
        &mut disk,
        root.path(),
        &fixture.source.source,
        &cancel,
        &fixture.budget,
    )
    .unwrap();
    assert_eq!(admitted.manifest, build.manifest);
}
#[test]
fn cancellation_or_project_close_prevents_installation() {
    let _guard = serial();
    let fixture = fixture();
    let build = synthetic(&fixture);
    let root = tempfile::tempdir().unwrap();
    let cancel = Cancellation::new();
    cancel.cancel();
    assert!(matches!(
        cache::install(
            root.path(),
            &build,
            plentiful(),
            &AtomicBool::new(false),
            None,
            &cancel,
            &fixture.budget
        ),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        cache::install(
            root.path(),
            &build,
            plentiful(),
            &AtomicBool::new(false),
            Some(&AtomicBool::new(true)),
            &Cancellation::new(),
            &fixture.budget
        ),
        Err(Error::Closed)
    ));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
#[test]
fn bounded_tile_read_refuses_before_allocating_over_limit() {
    let _guard = serial();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tile");
    std::fs::write(&path, [1u8; 33]).unwrap();
    let budget = SessionBudget::new(32).unwrap();
    assert!(matches!(
        cache::bytes(&path, 32, &budget, &Cancellation::new()),
        Err(Error::Limit)
    ));
    assert_eq!(budget.used_bytes(), 0);
    assert!(matches!(
        cache::bytes(&path, 33, &budget, &Cancellation::new()),
        Err(Error::Memory { .. })
    ));
    assert_eq!(budget.used_bytes(), 0);
}
#[test]
fn output_capability_retains_source_until_actual_release() {
    let _guard = serial();
    let fixture = fixture();
    let build = Arc::new(synthetic(&fixture));
    let path = build.output.path().to_path_buf();
    let retained = build.clone();
    cache::release(build).unwrap();
    assert!(path.exists());
    cache::release(retained).unwrap();
    assert!(!path.exists());
    assert!(fixture.source.info.source_path.exists());
}
#[cfg(unix)]
#[test]
fn source_replacement_and_symlink_inputs_are_refused() {
    let _guard = serial();
    let fixture = fixture();
    let path = fixture.source.info.source_path.clone();
    let mut changed = std::fs::read(&path).unwrap();
    changed[30] ^= 1;
    std::fs::write(&path, changed).unwrap();
    assert!(matches!(
        fixture.source.verify(
            &AtomicBool::new(false),
            &Cancellation::new(),
            &fixture.budget
        ),
        Err(Error::Stale)
    ));
    let external = tempfile::tempdir().unwrap();
    let link = external.path().join("link.png");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(source::regular(&link).is_err());
}

#[test]
fn exact_cleanup_drops_retained_payload_before_its_memory_credit() {
    let _guard = serial();
    struct Payload {
        budget: SessionBudget,
        observed: Arc<AtomicBool>,
    }
    impl Drop for Payload {
        fn drop(&mut self) {
            self.observed
                .store(self.budget.used_bytes() == 123, Ordering::Release);
        }
    }
    let budget = SessionBudget::new(1024).unwrap();
    let charge = budget.reserve(MemoryKind::Transfer, 123).unwrap();
    let observed = Arc::new(AtomicBool::new(false));
    let payload = Payload {
        budget: budget.clone(),
        observed: observed.clone(),
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_path_buf();
    let mut owned = cleanup::OwnedDirectory::new(directory, (payload, charge));
    owned.cleanup().unwrap();
    assert!(!path.exists());
    assert!(observed.load(Ordering::Acquire));
    assert_eq!(budget.used_bytes(), 0);
}
#[test]
fn failed_exact_cleanup_quarantines_its_actual_retained_credit() {
    let _guard = serial();
    let root = tempfile::tempdir().unwrap();
    let directory = tempfile::tempdir_in(root.path()).unwrap();
    let path = directory.path().to_path_buf();
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, [1u8]).unwrap();
    let budget = SessionBudget::new(1024).unwrap();
    let charge = budget.reserve(MemoryKind::Transfer, 123).unwrap();
    let mut owned = cleanup::OwnedDirectory::new(directory, charge);
    assert!(matches!(owned.cleanup(), Err(Error::Storage)));
    drop(owned);
    assert_eq!(budget.used_bytes(), 123);
    // The test's outer temp owner removes its fixture file. The quarantine
    // intentionally retains the credit: no runtime cleanup recovery is claimed.
}

#[test]
fn bounded_approval_inventory_evicts_the_oldest_witness() {
    let _guard = serial();
    let fixture = fixture();
    let build = synthetic(&fixture);
    let cancel = Cancellation::new();
    let roots = (0..65)
        .map(|_| tempfile::tempdir().unwrap())
        .collect::<Vec<_>>();
    for root in &roots {
        cache::register(root.path(), &build, &fixture.budget, &cancel).unwrap();
    }
    assert!(matches!(
        cache::approved(roots[0].path(), &build.manifest, &fixture.budget, &cancel),
        Err(Error::Unsupported)
    ));
    cache::approved(roots[64].path(), &build.manifest, &fixture.budget, &cancel).unwrap();
}
#[cfg(unix)]
#[test]
fn valid_tile_bytes_through_a_symlink_cannot_enter_the_cache() {
    let _guard = serial();
    let fixture = fixture();
    let build = synthetic(&fixture);
    let external = tempfile::tempdir().unwrap();
    let tile = build
        .directory()
        .join(build.manifest.tiles()[0].relative_path());
    let copy = external.path().join("tile.webp");
    std::fs::copy(&tile, &copy).unwrap();
    std::fs::remove_file(&tile).unwrap();
    std::os::unix::fs::symlink(copy, tile).unwrap();
    assert!(
        cache::verify_output(
            &build,
            &AtomicBool::new(false),
            None,
            &Cancellation::new(),
            &fixture.budget
        )
        .is_err()
    );
}

fn publication_options(path: &Path, disk_cache_bytes: u64) -> GiantPublishOptions {
    GiantPublishOptions {
        path: path.to_string_lossy().into_owned(),
        project_id: "018bcfe5-6800-7000-8000-000000000001".into(),
        document_id: "018bcfe5-6800-7000-8000-000000000002".into(),
        layer_id: "018bcfe5-6800-7000-8000-000000000003".into(),
        device_id: vw_model::DeviceId::from_bytes([3; 16]).to_string(),
        title: "Synthetic producer fixture".into(),
        now_ms: 1_700_000_000_000,
        disk_cache_bytes,
    }
}

/// Test-only approved producer. No public foreign factory can mint this witness.
fn public_fixture() -> (Fixture, Arc<GiantBuiltCache>) {
    let budget = crate::session_resources::shared_budget().unwrap();
    let (owned, limits) = request(&png());
    let source = input::prepare(
        owned,
        limits,
        &AtomicBool::new(false),
        &Cancellation::new(),
        &budget,
        &plentiful(),
    )
    .unwrap();
    let fixture = Fixture { source, budget };
    let build = synthetic(&fixture);
    let cap = Arc::new(GiantBuiltCache {
        worker: Worker::new(
            "test-public-approved",
            Some(BuiltState {
                build: Arc::new(build),
                publication_started: false,
            }),
            2,
        )
        .unwrap(),
        closed: Arc::new(AtomicBool::new(false)),
        budget: fixture.budget.clone(),
    });
    (fixture, cap)
}
fn public_options(path: &Path) -> crate::GiantProjectOptions {
    let options = publication_options(path, 32 * 1024 * 1024);
    crate::GiantProjectOptions {
        path: options.path,
        project_id: options.project_id,
        document_id: options.document_id,
        layer_id: options.layer_id,
        device_id: options.device_id,
        title: options.title,
        now_ms: options.now_ms,
        disk_cache_bytes: options.disk_cache_bytes,
    }
}
#[test]
fn public_publication_adopts_full_budgeted_session_and_reopens_accepted_destination() {
    let _guard = serial();
    let (fixture, cap) = public_fixture();
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("project");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let project = cap
            .publish_project(public_options(&destination), Arc::new(Cancellation::new()))
            .await
            .unwrap();
        assert!(matches!(
            cap.publish_project(
                public_options(&root.path().join("second")),
                Arc::new(Cancellation::new())
            )
            .await,
            Err(Error::Backpressure)
        ));
        let info = project.info().await.unwrap();
        let image = project
            .open_giant_image(
                super::super::GiantOpenOptions {
                    binding: WorkflowBinding {
                        project_id: info.project_id.clone(),
                        document_id: info.document_ids[0].clone(),
                        host_seq: info.host_seq,
                        state_hash: info.state_hash.clone(),
                    },
                    disk_cache_bytes: 32 * 1024 * 1024,
                    ram_cache_bytes: 1024 * 1024,
                },
                Arc::new(Cancellation::new()),
            )
            .await
            .unwrap();
        let delivery = image
            .prepare_tile(
                super::super::GiantTileAddress {
                    level: 0,
                    x: 0,
                    y: 0,
                },
                Arc::new(Cancellation::new()),
            )
            .await
            .unwrap();
        let lease = delivery.publication_lease().unwrap();
        let pixels = delivery.take_pixels().unwrap();
        assert_eq!(pixels.rgba, [21, 42, 84, 255]);
        drop(pixels);
        delivery.release_source().unwrap();
        lease.close().unwrap();
        image.close().await.unwrap();
        cap.close().await.unwrap();
        let charged = fixture.budget.used_bytes();
        assert!(charged >= 32 * 1024 * 1024);
        project.close().await.unwrap();
        assert_eq!(fixture.budget.used_bytes(), charged - 32 * 1024 * 1024);
        let reopened = crate::open_project(
            destination.to_string_lossy().into_owned(),
            Arc::new(Cancellation::new()),
        )
        .await
        .unwrap();
        assert_eq!(reopened.info().await.unwrap().state_hash, info.state_hash);
        reopened.close().await.unwrap();
    });
}
#[test]
fn public_rebuild_uses_actual_project_binding_and_closed_owner() {
    let _guard = serial();
    let (_fixture, cap) = public_fixture();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let project = cap
            .publish_project(
                public_options(&root.path().join("project")),
                Arc::new(Cancellation::new()),
            )
            .await
            .unwrap();
        let info = project.info().await.unwrap();
        let binding = WorkflowBinding {
            project_id: info.project_id,
            document_id: info.document_ids[0].clone(),
            host_seq: info.host_seq,
            state_hash: info.state_hash,
        };
        let mut stale = binding.clone();
        stale.state_hash = "f".repeat(64);
        assert!(matches!(
            project
                .rebuild_giant_image(cap.clone(), stale, Arc::new(Cancellation::new()))
                .await,
            Err(Error::Stale)
        ));
        project
            .rebuild_giant_image(cap.clone(), binding.clone(), Arc::new(Cancellation::new()))
            .await
            .unwrap();
        project.close().await.unwrap();
        assert!(matches!(
            project
                .rebuild_giant_image(cap.clone(), binding, Arc::new(Cancellation::new()))
                .await,
            Err(Error::Closed)
        ));
        cap.close().await.unwrap();
    });
}
#[test]
fn private_publication_streams_exact_original_and_retains_project_resources() {
    let _guard = serial();
    let fixture = fixture();
    let build = synthetic(&fixture);
    let parent = tempfile::tempdir().unwrap();
    let destination = parent.path().join("project");
    let before = fixture.budget.used_bytes();
    let options = publication_options(&destination, build.disk_cache_bytes);
    let published = publish::publish(
        &build,
        options.clone(),
        &AtomicBool::new(false),
        &Cancellation::new(),
        &fixture.budget,
    )
    .unwrap();
    assert!(fixture.budget.used_bytes() >= before + 32 * 1024 * 1024);
    struct Retained {
        store: vw_store::ProjectStore,
        blobs: vw_store::BlobStore,
        _permit: crate::project::SessionPermit,
        _charge: vw_stream::MemoryCharge,
    }
    let (store, blobs, permit, charge) = published.into_parts();
    let owned = Retained {
        store,
        blobs,
        _permit: permit,
        _charge: charge,
    };
    let asset =
        vw_model::AssetId::try_from(super::super::tiles::hex(&fixture.source.source.original))
            .unwrap();
    assert_eq!(
        std::fs::read(owned.blobs.path(&asset).unwrap()).unwrap(),
        png()
    );
    let metadata = &owned.store.project().assets[&asset];
    assert_eq!(metadata.bit_depth, 8);
    assert_eq!(metadata.color_space, "untagged");
    assert!(metadata.metadata_json.contains("untagged_srgb_consent"));
    assert!(metadata.metadata_json.contains("test-fixture-only"));
    cache::approved(
        &destination,
        &build.manifest,
        &fixture.budget,
        &Cancellation::new(),
    )
    .unwrap();
    assert!(matches!(
        publish::publish(
            &build,
            options,
            &AtomicBool::new(false),
            &Cancellation::new(),
            &fixture.budget
        ),
        Err(Error::Invalid)
    ));
    assert_eq!(
        std::fs::read(owned.blobs.path(&asset).unwrap()).unwrap(),
        png()
    );
    drop(owned);
    assert!(fixture.budget.used_bytes() < before + 32 * 1024 * 1024);
}
#[test]
fn cancelled_publication_leaves_no_destination_or_approved_cache() {
    let _guard = serial();
    let fixture = fixture();
    let build = synthetic(&fixture);
    let parent = tempfile::tempdir().unwrap();
    let destination = parent.path().join("project");
    let cancel = Cancellation::new();
    cancel.cancel();
    let before = fixture.budget.used_bytes();
    assert!(matches!(
        publish::publish(
            &build,
            publication_options(&destination, build.disk_cache_bytes),
            &AtomicBool::new(false),
            &cancel,
            &fixture.budget
        ),
        Err(Error::Cancelled)
    ));
    assert!(!destination.exists());
    assert_eq!(fixture.budget.used_bytes(), before);
}
#[test]
fn prepared_facade_refuses_unavailable_platform_without_directory_approval() {
    let _guard = serial();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (owned, limits) = request(&png());
        let budget = SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap();
        let prepared =
            prepare_original(owned, limits, budget.clone(), Arc::new(Cancellation::new()))
                .await
                .unwrap();
        assert_eq!(
            (prepared.describe().width, prepared.describe().height),
            (1, 1)
        );
        for platform in [
            GiantProducerPlatform::AndroidRegionJpeg,
            GiantProducerPlatform::WindowsPinnedVips,
        ] {
            let result = prepared
                .build_approved(
                    GiantProducerOptions {
                        platform,
                        disk_cache_bytes: 32 * 1024 * 1024,
                        timeout_ms: 1000,
                        assume_untagged_srgb: true,
                    },
                    Arc::new(Cancellation::new()),
                )
                .await;
            assert!(matches!(result, Err(Error::Unsupported)));
        }
        prepared.close_worker().await.unwrap();
        assert_eq!(budget.used_bytes(), 0);
    });
}
#[test]
fn joined_rebuild_fences_full_binding_and_reads_only_approved_tiles() {
    let _guard = serial();
    let fixture = fixture();
    let build = synthetic(&fixture);
    let parent = tempfile::tempdir().unwrap();
    let destination = parent.path().join("project");
    let options = publication_options(&destination, build.disk_cache_bytes);
    let document_id = options.document_id.clone();
    let published = publish::publish(
        &build,
        options,
        &AtomicBool::new(false),
        &Cancellation::new(),
        &fixture.budget,
    )
    .unwrap();
    let (store, blobs, permit, charge) = published.into_parts();
    // Test-only facade composition keeps the retained project charge through
    // actual close. Production still requires root's from_store_budgeted hook.
    struct FixtureProject {
        project: Arc<crate::ProjectSession>,
        _charge: vw_stream::MemoryCharge,
    }
    let project = crate::ProjectSession::from_store(store, blobs, permit).unwrap();
    let retained = FixtureProject {
        project,
        _charge: charge,
    };
    let cap = GiantBuiltCache {
        worker: Worker::new(
            "test-approved-build",
            Some(BuiltState {
                build: Arc::new(build),
                publication_started: false,
            }),
            2,
        )
        .unwrap(),
        closed: Arc::new(AtomicBool::new(false)),
        budget: fixture.budget.clone(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let info = retained.project.info().await.unwrap();
        let binding = WorkflowBinding {
            project_id: info.project_id,
            document_id,
            host_seq: info.host_seq,
            state_hash: info.state_hash,
        };
        let mut stale = binding.clone();
        stale.state_hash = "f".repeat(64);
        assert!(matches!(
            cap.rebuild(
                retained.project.worker.clone(),
                retained.project.closed.clone(),
                stale,
                Arc::new(Cancellation::new())
            )
            .await,
            Err(Error::Stale)
        ));
        cap.rebuild(
            retained.project.worker.clone(),
            retained.project.closed.clone(),
            binding.clone(),
            Arc::new(Cancellation::new()),
        )
        .await
        .unwrap();
        let image = super::super::provider::open_image(
            retained.project.worker.clone(),
            retained.project.closed.clone(),
            super::super::GiantOpenOptions {
                binding,
                disk_cache_bytes: 32 * 1024 * 1024,
                ram_cache_bytes: 1024 * 1024,
            },
            fixture.budget.clone(),
            Arc::new(Cancellation::new()),
        )
        .await
        .unwrap();
        let tile = image
            .read_tile(
                super::super::GiantTileAddress {
                    level: 0,
                    x: 0,
                    y: 0,
                },
                Arc::new(Cancellation::new()),
            )
            .await
            .unwrap();
        let (pixels, outgoing) = tile.into_parts();
        assert_eq!(pixels.rgba, [21, 42, 84, 255]);
        assert!(outgoing.belongs_to(&fixture.budget));
        drop(pixels);
        drop(outgoing);
        image.close_worker().await.unwrap();
        cap.close_worker().await.unwrap();
        retained.project.close().await.unwrap();
    });
    drop(retained);
}

// Exercises the real local publication/cache/provider path with a test-only
// producer witness. It does not execute Android/JNI or measure codec/GPU RSS.
#[test]
fn phone_provider_reads_beside_surface_after_actual_producer_retirement() {
    let _guard = serial();
    let fixture = fixture_with_budget(vw_stream::PHONE_SESSION_BYTES);
    let budget = fixture.budget.clone();
    let original = png();
    let original_id = fixture.source.source.original;
    let build = synthetic(&fixture);
    let parent = tempfile::tempdir().unwrap();
    let destination = parent.path().join("project");
    let options = publication_options(&destination, build.disk_cache_bytes);
    let document_id = options.document_id.clone();
    let published = publish::publish(
        &build,
        options,
        &AtomicBool::new(false),
        &Cancellation::new(),
        &budget,
    )
    .unwrap();
    // Retire actual output/manifest and private input owners; do not merely
    // credit their envelopes while their directories or payloads remain alive.
    drop(build);
    drop(fixture);
    struct FixtureProject {
        project: Arc<crate::ProjectSession>,
        _charge: vw_stream::MemoryCharge,
    }
    let (store, blobs, permit, charge) = published.into_parts();
    let asset = vw_model::AssetId::try_from(super::super::tiles::hex(&original_id)).unwrap();
    let original_path = blobs.path(&asset).unwrap();
    let retained = FixtureProject {
        project: crate::ProjectSession::from_store(store, blobs, permit).unwrap(),
        _charge: charge,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let info = retained.project.info().await.unwrap();
        let binding = WorkflowBinding {
            project_id: info.project_id,
            document_id,
            host_seq: info.host_seq,
            state_hash: info.state_hash,
        };
        let options = super::super::GiantOpenOptions {
            binding,
            disk_cache_bytes: 32 * 1024 * 1024,
            ram_cache_bytes: 1024 * 1024,
        };
        // Constructor JSON parsing still requires64MiB. An old96MiB surface
        // must really retire before a new provider opens in the same phone pool.
        let old_surface = budget
            .reserve(MemoryKind::DisplayFrame, 96 * 1024 * 1024)
            .unwrap();
        assert!(matches!(
            super::super::provider::open_image(
                retained.project.worker.clone(),
                retained.project.closed.clone(),
                options.clone(),
                budget.clone(),
                Arc::new(Cancellation::new()),
            )
            .await,
            Err(Error::Memory { .. })
        ));
        drop(old_surface);
        let image = super::super::provider::open_image(
            retained.project.worker.clone(),
            retained.project.closed.clone(),
            options,
            budget.clone(),
            Arc::new(Cancellation::new()),
        )
        .await
        .unwrap();
        let surface = budget
            .reserve(MemoryKind::DisplayFrame, 96 * 1024 * 1024)
            .unwrap();
        // Matches Android's pre-call 3RGBA +16KiB per-tile owner envelope.
        let foreign = budget
            .reserve(MemoryKind::Foreign, 256 * 256 * 12 + 16 * 1024)
            .unwrap();
        let before = budget.used_bytes();
        // One live approval entry retains64KiB after the actual producer/input
        // owners retire. The floor includes real project/provider/surface owners.
        assert_eq!(before, 216 * 1024 * 1024 + foreign.bytes() + 64 * 1024);
        // The former per-read index envelope cannot fit this actual live state.
        assert!(
            budget
                .reserve(
                    MemoryKind::CodecWorkspace,
                    super::super::tiles::STORE_WORKSPACE,
                )
                .is_err()
        );
        assert_eq!(budget.used_bytes(), before);
        // Read/index scratch fits, but a live16MiB foreign owner leaves less
        // than the32MiB codec workspace. The real miss must refuse, release its
        // transient encoded/index owners, and keep that foreign owner charged.
        let pressure = budget
            .reserve(MemoryKind::Foreign, 16 * 1024 * 1024)
            .unwrap();
        let pressured = budget.used_bytes();
        assert!(matches!(
            image
                .read_tile(
                    super::super::GiantTileAddress {
                        level: 0,
                        x: 0,
                        y: 0,
                    },
                    Arc::new(Cancellation::new()),
                )
                .await,
            Err(Error::Memory { .. })
        ));
        assert_eq!(budget.used_bytes(), pressured);
        assert_eq!(pressure.bytes(), 16 * 1024 * 1024);
        drop(pressure);
        assert_eq!(budget.used_bytes(), before);
        let tile = image
            .read_tile(
                super::super::GiantTileAddress {
                    level: 0,
                    x: 0,
                    y: 0,
                },
                Arc::new(Cancellation::new()),
            )
            .await
            .unwrap();
        let (pixels, outgoing) = tile.into_parts();
        assert_eq!(pixels.rgba, [21, 42, 84, 255]);
        assert!(outgoing.belongs_to(&budget));
        assert_eq!(
            budget.used_bytes(),
            before + super::super::tiles::TILE_RGBA_BYTES + outgoing.bytes()
        );
        assert!(budget.used_bytes() <= vw_stream::PHONE_SESSION_BYTES);
        let cancelled = Arc::new(Cancellation::new());
        cancelled.cancel();
        let before_cancel = budget.used_bytes();
        assert!(matches!(
            image
                .read_tile(
                    super::super::GiantTileAddress {
                        level: 0,
                        x: 0,
                        y: 0,
                    },
                    cancelled,
                )
                .await,
            Err(Error::Cancelled)
        ));
        assert_eq!(budget.used_bytes(), before_cancel);
        image.close_worker().await.unwrap();
        // Provider close may retire its cached payload; it cannot credit the
        // outgoing payload or renderer's independent foreign/surface owners.
        let after_close = budget.used_bytes();
        assert_eq!(
            after_close,
            before - super::super::tiles::STORE_RESIDENT - super::super::tiles::MANIFEST_WORKSPACE
                + outgoing.bytes()
        );
        assert!(after_close >= foreign.bytes() + surface.bytes() + outgoing.bytes());
        drop(pixels);
        let outgoing_bytes = outgoing.bytes();
        drop(outgoing);
        assert_eq!(budget.used_bytes(), after_close - outgoing_bytes);
        let foreign_bytes = foreign.bytes();
        let before_foreign = budget.used_bytes();
        drop(foreign);
        assert_eq!(budget.used_bytes(), before_foreign - foreign_bytes);
        drop(surface);
        retained.project.close().await.unwrap();
    });
    assert_eq!(std::fs::read(original_path).unwrap(), original);
    drop(retained);
}
