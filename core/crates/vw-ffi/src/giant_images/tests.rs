//! Native leaf tests. These are deliberately registered by provider.rs only;
//! the root owner admits the module before the next coherent native test phase.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::CoreError;
use std::{fs::File, io::Write, path::Path};
use vw_assets::{Source, TileRecord};
use vw_model::{DeviceId, Id};

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[21, 42, 84])
            .unwrap();
    }
    bytes
}
fn fixture(root: &Path) -> CapturedSource {
    let bytes = png();
    let path = root.join("original.png");
    std::fs::write(&path, &bytes).unwrap();
    CapturedSource {
        root: root.to_path_buf(),
        path,
        source: Source {
            original: *blake3::hash(&bytes).as_bytes(),
            original_bytes: bytes.len() as u64,
            width: 1,
            height: 1,
            orientation: 1,
            bit_depth: 8,
            icc: None,
            display_conversion: false,
        },
        format: "png",
    }
}
fn manifest(source: &Source, encoded: &[u8]) -> Manifest {
    Manifest::new(
        source.clone(),
        source
            .validate()
            .unwrap()
            .keys()
            .map(|key| TileRecord {
                key,
                encoded_bytes: encoded.len() as u32,
                hash: *blake3::hash(encoded).as_bytes(),
            })
            .collect(),
    )
    .unwrap()
}
fn key0() -> TileKey {
    TileKey {
        level: 0,
        x: 0,
        y: 0,
    }
}
fn tile() -> Vec<u8> {
    tile_from_png(&png())
}
fn tile_from_png(bytes: &[u8]) -> Vec<u8> {
    let image = vw_raster::decode(bytes, vw_raster::DecodeLimits::default()).unwrap();
    let encoded = vw_raster::export(
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
        },
    )
    .unwrap()
    .bytes;
    // Derived caches contain only this exact lossless chunk, never an export
    // receipt. This extraction is a fixture builder, not a user export path.
    let mut at = 12;
    while at + 8 <= encoded.len() {
        let len = u32::from_le_bytes(encoded[at + 4..at + 8].try_into().unwrap()) as usize;
        if &encoded[at..at + 4] == b"VP8L" {
            let mut out = b"RIFF".to_vec();
            out.extend_from_slice(&((12 + len + len % 2) as u32).to_le_bytes());
            out.extend_from_slice(b"WEBP");
            out.extend_from_slice(&encoded[at..at + 8 + len + len % 2]);
            return out;
        }
        at += 8 + len + len % 2;
    }
    panic!("missing fixture lossless chunk")
}
fn inspect(bytes: &[u8]) -> crate::Result<source::Metadata> {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("header");
    std::fs::write(&path, bytes).unwrap();
    source::inspect(&mut source::regular(&path).unwrap(), &Cancellation::new())
}
fn jpeg(orientation: u16) -> Vec<u8> {
    let mut exif = b"Exif\0\0II".to_vec();
    exif.extend_from_slice(&42u16.to_le_bytes());
    exif.extend_from_slice(&8u32.to_le_bytes());
    exif.extend_from_slice(&1u16.to_le_bytes());
    exif.extend_from_slice(&0x112u16.to_le_bytes());
    exif.extend_from_slice(&3u16.to_le_bytes());
    exif.extend_from_slice(&1u32.to_le_bytes());
    exif.extend_from_slice(&orientation.to_le_bytes());
    exif.extend_from_slice(&[0; 6]);
    let mut out = vec![255, 216, 255, 225];
    out.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    out.extend(exif);
    out.extend_from_slice(&[
        255, 192, 0, 17, 8, 0, 2, 0, 3, 3, 1, 17, 0, 2, 17, 1, 3, 17, 1, 255, 218, 0, 12, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0,
    ]);
    out
}

#[test]
fn source_headers_preserve_all_eight_exif_orientations_without_decode() {
    for orientation in 1..=8 {
        let metadata = inspect(&jpeg(orientation)).unwrap();
        assert_eq!(metadata.orientation, orientation as u8);
        assert_eq!(
            metadata.dimensions(),
            if orientation >= 5 { (2, 3) } else { (3, 2) }
        );
    }
    assert!(inspect(&jpeg(9)).is_err());
    assert_eq!(inspect(&png()).unwrap().dimensions(), (1, 1));
}

#[test]
fn unsupported_profile_depth_alpha_and_truncated_source_headers_refuse() {
    let mut profiled = jpeg(1);
    let mut app = vec![255, 226, 0, 16];
    app.extend_from_slice(b"ICC_PROFILE\0\x01\x01");
    profiled.splice(2..2, app);
    assert!(matches!(inspect(&profiled), Err(CoreError::Unsupported)));
    for (marker, payload) in [(0xe2, b"MPF\0".as_slice()), (0xeb, b"jumb".as_slice())] {
        let mut bytes = jpeg(1);
        let mut app = vec![255, marker];
        app.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        app.extend_from_slice(payload);
        bytes.splice(2..2, app);
        assert!(matches!(inspect(&bytes), Err(CoreError::Unsupported)));
    }
    for (depth, color) in [(16, 2), (8, 6), (8, 3)] {
        let mut bytes = png();
        bytes[24] = depth;
        bytes[25] = color;
        assert!(matches!(inspect(&bytes), Err(CoreError::Unsupported)));
    }
    for len in [0, 1, 8, 20, 32] {
        assert!(inspect(&png()[..len]).is_err());
    }
}

#[test]
fn source_stream_hash_and_length_refuse_damage_preserving_original_bytes() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let pool = SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap();
    let closed = AtomicBool::new(false);
    let cancel = Cancellation::new();
    source.verify(&closed, &cancel, &pool).unwrap();
    assert_eq!(pool.used_bytes(), 0);
    let before = std::fs::read(&source.path).unwrap();
    let mut damaged = before.clone();
    let at = damaged.len() - 1;
    damaged[at] ^= 1;
    std::fs::write(&source.path, &damaged).unwrap();
    assert!(matches!(
        source.verify(&closed, &cancel, &pool),
        Err(Error::Stale)
    ));
    assert_eq!(std::fs::read(&source.path).unwrap(), damaged);
    std::fs::write(&source.path, &before[..before.len() - 1]).unwrap();
    assert!(matches!(
        source.verify(&closed, &cancel, &pool),
        Err(Error::Stale)
    ));
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn source_cancellation_closure_and_workspace_refusal_precede_reads() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let pool = SessionBudget::new(1).unwrap();
    let cancel = Cancellation::new();
    assert!(matches!(
        source.verify(&AtomicBool::new(false), &cancel, &pool),
        Err(Error::Memory { .. })
    ));
    cancel.cancel();
    std::fs::remove_file(&source.path).unwrap();
    assert!(matches!(
        source.verify(&AtomicBool::new(false), &cancel, &pool),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        source.verify(&AtomicBool::new(true), &cancel, &pool),
        Err(Error::Closed)
    ));
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn missing_and_symlink_original_never_falls_back_to_derived_content() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let pool = SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap();
    std::fs::remove_file(&source.path).unwrap();
    assert!(matches!(
        source.verify(&AtomicBool::new(false), &Cancellation::new(), &pool),
        Err(Error::Missing)
    ));
    #[cfg(unix)]
    {
        let owner = root.path().join("owner.png");
        std::fs::write(&owner, png()).unwrap();
        std::os::unix::fs::symlink(&owner, &source.path).unwrap();
        assert!(
            source
                .verify(&AtomicBool::new(false), &Cancellation::new(), &pool)
                .is_err()
        );
        assert_eq!(std::fs::read(&owner).unwrap(), png());
    }
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn hash_copy_refuses_overrun_before_output_write_and_checks_cancellation() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let mut input = source::regular(&source.path).unwrap();
    let mut output = tempfile::tempfile().unwrap();
    assert!(matches!(
        source::hash_copy(&mut input, Some(&mut output), 1, &Cancellation::new()),
        Err(CoreError::Backpressure)
    ));
    assert_eq!(output.metadata().unwrap().len(), 0);
    let cancel = Cancellation::new();
    cancel.cancel();
    assert!(matches!(
        source::hash_copy(&mut input, None, 1024, &cancel),
        Err(CoreError::Cancelled)
    ));
}

#[test]
fn missing_malformed_oversized_and_unapproved_manifest_are_distinct_refusals() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let pool = SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap();
    let cancel = Cancellation::new();
    let mut disk = DiskCache::new(root.path(), 64 * 1024 * 1024).unwrap();
    assert!(matches!(
        tiles::manifest(&mut disk, &source.source, &cancel, &pool),
        Err(Error::Missing)
    ));
    let key = tiles::manifest_key(&source.source);
    disk.put(&key, b"bad").unwrap();
    assert!(matches!(
        tiles::manifest(&mut disk, &source.source, &cancel, &pool),
        Err(Error::Invalid)
    ));
    disk.put(&key, &vec![0; vw_assets::MAX_MANIFEST_BYTES + 1])
        .unwrap();
    assert!(matches!(
        tiles::manifest(&mut disk, &source.source, &cancel, &pool),
        Err(Error::Limit)
    ));
    let manifest = manifest(&source.source, &tile());
    disk.put(&key, &manifest.encode()).unwrap();
    assert!(matches!(
        tiles::manifest(&mut disk, &source.source, &cancel, &pool),
        Err(Error::Unsupported)
    ));
    assert_eq!(pool.used_bytes(), 0);
    assert_eq!(std::fs::read(&source.path).unwrap(), png());
}

#[test]
fn exact_encoded_length_and_hash_refuse_before_decoding() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let bytes = tile();
    let manifest = manifest(&source.source, &bytes);
    let pool = SessionBudget::new(128 * 1024 * 1024).unwrap();
    let mut disk = DiskCache::new(root.path(), 16 * 1024 * 1024).unwrap();
    let cancel = Cancellation::new();
    let key = tiles::tile_key(&source.source, key0());
    assert!(matches!(
        CachedTile::decode(&mut disk, &manifest, key0(), &cancel, &pool),
        Err(Error::Missing)
    ));
    disk.put(&key, &vec![0; bytes.len() + 1]).unwrap();
    assert!(matches!(
        CachedTile::decode(&mut disk, &manifest, key0(), &cancel, &pool),
        Err(Error::Limit)
    ));
    disk.put(&key, &vec![0; bytes.len()]).unwrap();
    assert!(matches!(
        CachedTile::decode(&mut disk, &manifest, key0(), &cancel, &pool),
        Err(Error::Invalid)
    ));
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn codec_dimensions_metadata_and_trailing_payload_refuse() {
    let bytes = tile();
    let region = vw_assets::TileRegion {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    let cancel = Cancellation::new();
    assert_eq!(
        tiles::decode(&bytes, key0(), region, &cancel).unwrap(),
        [21, 42, 84, 255]
    );
    let wrong = vw_assets::TileRegion { width: 2, ..region };
    assert!(tiles::decode(&bytes, key0(), wrong, &cancel).is_err());
    let mut trailing = bytes.clone();
    trailing.extend_from_slice(b"EXIF");
    assert!(tiles::decode(&trailing, key0(), region, &cancel).is_err());
    assert!(tiles::decode(&png(), key0(), region, &cancel).is_err());
    let mut alpha_png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut alpha_png, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[21, 42, 84, 254])
            .unwrap();
    }
    assert!(matches!(
        tiles::decode(&tile_from_png(&alpha_png), key0(), region, &cancel),
        Err(Error::Unsupported)
    ));
    let mut expansion = bytes.clone();
    // VP8L declares 16384-square output while retaining the tiny fixture's
    // compressed bytes. DecodeLimits must refuse before any expanded surface.
    expansion[21..25].copy_from_slice(&(16383u32 | (16383u32 << 14)).to_le_bytes());
    assert!(tiles::decode(&expansion, key0(), region, &cancel).is_err());
    assert!(tiles::decode(&bytes, TileKey { level: 1, ..key0() }, region, &cancel).is_err());
}

#[test]
fn codec_workspace_admission_precedes_decode_and_cancel_drops_actual_payload() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let bytes = tile();
    let manifest = manifest(&source.source, &bytes);
    let mut disk = DiskCache::new(root.path(), 16 * 1024 * 1024).unwrap();
    disk.put(&tiles::tile_key(&source.source, key0()), &bytes)
        .unwrap();
    let pool = SessionBudget::new(tiles::DECODE_WORKSPACE).unwrap();
    assert!(matches!(
        CachedTile::decode(&mut disk, &manifest, key0(), &Cancellation::new(), &pool),
        Err(Error::Memory { .. })
    ));
    assert_eq!(pool.used_bytes(), 0);
    let cancel = Cancellation::new();
    cancel.cancel();
    assert!(matches!(
        CachedTile::decode(&mut disk, &manifest, key0(), &cancel, &pool),
        Err(Error::Cancelled)
    ));
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn cleared_pinned_cache_payload_keeps_shared_charge_until_actual_lease_drop() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let bytes = tile();
    let manifest = manifest(&source.source, &bytes);
    let pool = SessionBudget::new(128 * 1024 * 1024).unwrap();
    let mut disk = DiskCache::new(root.path(), 16 * 1024 * 1024).unwrap();
    disk.put(&tiles::tile_key(&source.source, key0()), &bytes)
        .unwrap();
    let cache = BudgetCache::new(tiles::TILE_RGBA_BYTES).unwrap();
    let slot = cache.reserve(key0(), tiles::TILE_RGBA_BYTES).unwrap();
    let payload =
        CachedTile::decode(&mut disk, &manifest, key0(), &Cancellation::new(), &pool).unwrap();
    let lease = slot.commit(payload, tiles::TILE_RGBA_BYTES).unwrap();
    cache.clear().unwrap();
    assert!(cache.get(&key0()).unwrap().is_none());
    assert_eq!(pool.used_bytes(), tiles::TILE_RGBA_BYTES);
    assert_eq!(lease.payload().unwrap().rgba, [21, 42, 84, 255]);
    drop(cache);
    assert_eq!(pool.used_bytes(), tiles::TILE_RGBA_BYTES);
    drop(lease);
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn native_delivery_copy_keeps_its_charge_after_cache_and_worker_state_drop() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let bytes = tile();
    let manifest = manifest(&source.source, &bytes);
    let pool = SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap();
    // The platform's required surface allowance shares this same pool. Holding
    // a 128 MiB index peak permanently would leave no room for tile decoding.
    let surface = reserve(&pool, MemoryKind::Foreign, 96 * 1024 * 1024).unwrap();
    let store_charge = reserve(&pool, MemoryKind::CodecWorkspace, tiles::STORE_RESIDENT).unwrap();
    let manifest_charge = reserve(&pool, MemoryKind::Transfer, tiles::MANIFEST_WORKSPACE).unwrap();
    let mut disk = DiskCache::new(root.path(), 16 * 1024 * 1024).unwrap();
    disk.put(&tiles::tile_key(&source.source, key0()), &bytes)
        .unwrap();
    let mut state = ImageState {
        disk,
        identity: tiles::hex(&manifest.identity()),
        manifest,
        memory: BudgetCache::new(tiles::TILE_RGBA_BYTES).unwrap(),
        _manifest_charge: manifest_charge,
        _store_charge: store_charge,
    };
    let delivery = state
        .tile(address(key0()), &Cancellation::new(), &pool)
        .unwrap();
    assert_eq!(delivery.pixels().rgba, [21, 42, 84, 255]);
    let (pixels, outgoing) = delivery.into_parts();
    assert!(outgoing.belongs_to(&pool));
    let abi = reserve(&pool, MemoryKind::CodecWorkspace, 4 + 16 * 1024).unwrap();
    let native_lease = crate::SessionResourceCharge::from_charges(vec![outgoing, abi]);
    let delivery = super::super::GiantTileDelivery {
        pixels,
        native_lease,
    };
    drop(state);
    assert_eq!(pool.used_bytes(), surface.bytes() + 8 + 16 * 1024);
    let lease = delivery.native_lease.clone();
    drop(delivery);
    assert_eq!(pool.used_bytes(), surface.bytes() + 8 + 16 * 1024);
    lease.close().unwrap();
    assert_eq!(pool.used_bytes(), surface.bytes());
    drop(surface);
    assert_eq!(pool.used_bytes(), 0);
    assert_eq!(std::fs::read(&source.path).unwrap(), png());
}

#[test]
fn source_and_manifest_geometry_clip_odd_edges_without_scope_expansion() {
    let root = tempfile::tempdir().unwrap();
    let mut source = fixture(root.path()).source;
    source.width = 513;
    source.height = 257;
    let manifest = manifest(&source, &[1]);
    assert_eq!(
        document_region(
            &manifest,
            TileKey {
                level: 0,
                x: 2,
                y: 1
            }
        )
        .unwrap(),
        (512, 256, 1, 1)
    );
    assert_eq!(
        document_region(
            &manifest,
            TileKey {
                level: 1,
                x: 1,
                y: 0
            }
        )
        .unwrap(),
        (512, 0, 1, 257)
    );
    assert!(
        document_region(
            &manifest,
            TileKey {
                level: 0,
                x: 3,
                y: 0
            }
        )
        .is_err()
    );
    assert!(
        document_region(
            &manifest,
            TileKey {
                level: u8::MAX,
                x: 0,
                y: 0
            }
        )
        .is_err()
    );
}

#[test]
fn nonuniform_scale_preserves_density_and_singular_transform_refuses() {
    let view = GiantViewport {
        left: 0.,
        top: 0.,
        right: 1.,
        bottom: 1.,
        a: 2.,
        b: 0.,
        c: 0.,
        d: 0.25,
    };
    assert_eq!(physical_scale(&view).unwrap(), 2.);
    assert!(
        physical_scale(&GiantViewport {
            a: 1.,
            b: 2.,
            c: 2.,
            d: 4.,
            ..view.clone()
        })
        .is_err()
    );
    assert!(
        physical_scale(&GiantViewport {
            a: f64::NAN,
            ..view
        })
        .is_err()
    );
}

#[test]
fn staged_plan_is_bounded_but_neither_cache_nor_caller_paths_are_producer_approval() {
    let root = tempfile::tempdir().unwrap();
    let mut source = fixture(root.path());
    let plan = super::super::producer::stage(&source, 1024).unwrap();
    assert_eq!(plan.asset_id, tiles::hex(&source.source.original));
    assert_eq!(
        (plan.width, plan.height, plan.orientation, plan.format),
        (1, 1, 1, "png")
    );
    assert_eq!(plan.original_bytes, source.source.original_bytes);
    assert_eq!(
        (plan.tile_size, plan.tile_count, plan.disk_cache_bytes),
        (256, 1, 1024)
    );
    assert_eq!(plan.maximum_tile_bytes, vw_assets::MAX_ENCODED_TILE_BYTES);
    assert_eq!(plan.maximum_manifest_bytes, vw_assets::MAX_MANIFEST_BYTES);
    assert_eq!(plan.maximum_band_workspace_bytes, 32 * 1024 * 1024);
    assert!(plan.android_band_fits);
    assert!(super::super::producer::stage(&source, 0).is_err());
    assert!(
        super::super::producer::stage(&source, super::super::producer::MAX_DISK_BYTES + 1).is_err()
    );
    source.source.display_conversion = true;
    assert!(matches!(
        super::super::producer::stage(&source, 1024),
        Err(Error::Unsupported)
    ));
}

#[test]
fn oversized_declared_header_does_not_allocate_source_sized_memory() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("sparse.png");
    let bytes = png();
    let mut file = File::create(&path).unwrap();
    file.write_all(&bytes[..33]).unwrap();
    file.write_all(&(17 * 1024 * 1024u32).to_be_bytes())
        .unwrap();
    file.write_all(b"tEXt").unwrap();
    file.set_len(33 + 12 + 17 * 1024 * 1024).unwrap();
    drop(file);
    assert!(matches!(
        source::inspect(&mut source::regular(&path).unwrap(), &Cancellation::new()),
        Err(CoreError::Backpressure)
    ));
}

fn id(value: u8) -> String {
    Id::from_parts(1_700_000_000_000, [value; 10])
        .unwrap()
        .to_string()
}
async fn project(root: &Path) -> Arc<crate::ProjectSession> {
    crate::create_image_project(
        crate::CreateImageProject {
            path: root.join("project").to_string_lossy().into_owned(),
            project_id: id(1),
            document_id: id(2),
            layer_id: id(3),
            device_id: DeviceId::from_bytes([4; 16]).to_string(),
            title: "giant provider fixture".into(),
            source: png(),
            now_ms: 1_700_000_000_000,
        },
        Arc::new(Cancellation::new()),
    )
    .await
    .unwrap()
}
async fn binding(project: &crate::ProjectSession) -> WorkflowBinding {
    let info = project.info().await.unwrap();
    WorkflowBinding {
        project_id: info.project_id,
        document_id: id(2),
        host_seq: info.host_seq,
        state_hash: info.state_hash,
    }
}

#[tokio::test]
async fn full_state_hash_refuses_even_when_project_document_and_host_sequence_match() {
    let root = tempfile::tempdir().unwrap();
    let project = project(root.path()).await;
    let mut binding = binding(&project).await;
    binding.state_hash = "f".repeat(64);
    let pool = SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap();
    let closed = project.closed.clone();
    let observed = project
        .worker
        .call(move |state| {
            Ok(source::capture(
                state,
                &binding,
                &closed,
                &Cancellation::new(),
                &pool,
            ))
        })
        .await
        .unwrap();
    assert!(matches!(observed, Err(Error::Stale)));
    project.close().await.unwrap();
}

#[tokio::test]
async fn closed_or_cancelled_open_refuses_and_absent_cache_preserves_original() {
    let root = tempfile::tempdir().unwrap();
    let project = project(root.path()).await;
    let binding = binding(&project).await;
    let pool = SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap();
    let options = GiantOpenOptions {
        binding,
        disk_cache_bytes: 16 * 1024 * 1024,
        ram_cache_bytes: 1024 * 1024,
    };
    let cancel = Arc::new(Cancellation::new());
    cancel.cancel();
    assert!(matches!(
        open_image(
            project.worker.clone(),
            project.closed.clone(),
            options.clone(),
            pool.clone(),
            cancel
        )
        .await,
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        open_image(
            project.worker.clone(),
            project.closed.clone(),
            options.clone(),
            pool.clone(),
            Arc::new(Cancellation::new())
        )
        .await,
        Err(Error::Missing)
    ));
    assert_eq!(pool.used_bytes(), 0);
    project.close().await.unwrap();
    assert!(matches!(
        open_image(
            project.worker.clone(),
            project.closed.clone(),
            options,
            pool.clone(),
            Arc::new(Cancellation::new())
        )
        .await,
        Err(Error::Closed)
    ));
    assert_eq!(pool.used_bytes(), 0);
}

// Private unit fixture deliberately constructs ImageState after bounded helper
// validation. It exercises provider ownership/fences, not producer approval;
// the public factory's unapproved-cache refusal is tested independently above.
async fn reader_fixture(
    root: &Path,
) -> (Arc<crate::ProjectSession>, Arc<GiantImage>, SessionBudget) {
    let project = project(root).await;
    let binding = binding(&project).await;
    let budget = SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap();
    let capture_binding = binding.clone();
    let capture_budget = budget.clone();
    let capture_closed = project.closed.clone();
    let captured = project
        .worker
        .call(move |state| {
            Ok(source::capture(
                state,
                &capture_binding,
                &capture_closed,
                &Cancellation::new(),
                &capture_budget,
            ))
        })
        .await
        .unwrap()
        .unwrap();
    let store_charge = reserve(&budget, MemoryKind::CodecWorkspace, tiles::STORE_RESIDENT).unwrap();
    let manifest_charge =
        reserve(&budget, MemoryKind::Transfer, tiles::MANIFEST_WORKSPACE).unwrap();
    let mut disk = DiskCache::new(&captured.root, 16 * 1024 * 1024).unwrap();
    let bytes = tile();
    let manifest = manifest(&captured.source, &bytes);
    disk.put(&tiles::tile_key(&captured.source, key0()), &bytes)
        .unwrap();
    let state = ImageState {
        disk,
        identity: tiles::hex(&manifest.identity()),
        manifest,
        memory: BudgetCache::new(tiles::TILE_RGBA_BYTES).unwrap(),
        _manifest_charge: manifest_charge,
        _store_charge: store_charge,
    };
    let image = Arc::new(GiantImage {
        project: project.worker.clone(),
        project_closed: project.closed.clone(),
        closed: Arc::new(AtomicBool::new(false)),
        binding,
        source: captured,
        budget: budget.clone(),
        worker: Worker::new("vw-giant-unit-reader", Some(state), 2).unwrap(),
    });
    (project, image, budget)
}

#[tokio::test]
async fn provider_fences_actual_original_damage_before_native_result_adoption() {
    let root = tempfile::tempdir().unwrap();
    let (project, image, pool) = reader_fixture(root.path()).await;
    let view = GiantViewport {
        left: 0.,
        top: 0.,
        right: 1.,
        bottom: 1.,
        a: 1.,
        b: 0.,
        c: 0.,
        d: 1.,
    };
    let plan = image
        .read_viewport(view, Arc::new(Cancellation::new()))
        .await
        .unwrap();
    assert_eq!(plan.visible, [address(key0())]);
    let delivered = image
        .read_tile(plan.visible[0], Arc::new(Cancellation::new()))
        .await
        .unwrap();
    assert_eq!(delivered.pixels().asset_id, plan.asset_id);
    assert_eq!(delivered.pixels().manifest_hash, plan.manifest_hash);
    drop(delivered);
    let mut damaged = png();
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    std::fs::write(&image.source.path, &damaged).unwrap();
    assert!(matches!(
        image
            .read_tile(address(key0()), Arc::new(Cancellation::new()))
            .await,
        Err(Error::Stale)
    ));
    assert_eq!(std::fs::read(&image.source.path).unwrap(), damaged);
    image.close_worker().await.unwrap();
    assert_eq!(pool.used_bytes(), 0);
    project.close().await.unwrap();
}

#[tokio::test]
async fn closure_joins_actual_worker_and_cancelled_late_read_never_returns_pixels() {
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };
    let root = tempfile::tempdir().unwrap();
    let (project, image, pool) = reader_fixture(root.path()).await;
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let worker = image.worker.clone();
    let held = tokio::spawn(async move {
        worker
            .call(move |_| {
                let _ = started.send(());
                wait.recv().unwrap();
                Ok(())
            })
            .await
    });
    ready.await.unwrap();
    let cancel = Arc::new(Cancellation::new());
    let reader = image.clone();
    let reader_cancel = cancel.clone();
    let reading =
        tokio::spawn(async move { reader.read_tile(address(key0()), reader_cancel).await });
    let mut closing = Box::pin(image.close_worker());
    poll_fn(|cx| {
        assert!(closing.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(pool.used_bytes() >= tiles::STORE_RESIDENT + tiles::MANIFEST_WORKSPACE);
    cancel.cancel();
    release.send(()).unwrap();
    closing.await.unwrap();
    held.await.unwrap().unwrap();
    assert!(matches!(
        reading.await.unwrap(),
        Err(Error::Closed | Error::Cancelled)
    ));
    assert_eq!(pool.used_bytes(), 0);
    image.close_worker().await.unwrap();
    project.close().await.unwrap();
}
