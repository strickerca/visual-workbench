//! Real store/decoder/lease contracts. Coordinator runs after root registration.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::{io::Cursor, sync::Arc};
use support::*;
use vw_core::*;
use vw_model::{AssetId, DeviceId};
use vw_raster::{DecodedImage, Pixels};

const BUDGET: u64 = 256 * 1024 * 1024;
static TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn temp() -> tempfile::TempDir {
    if cfg!(target_os = "android") {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    }
}
fn fixture(depth: u8, tagged: bool) -> (DecodedImage, Vec<u8>) {
    let samples: Vec<u16> = (0..64u16)
        .flat_map(|y| (0..64u16).flat_map(move |x| [x * 997, y * 991, (x + y) * 509, 65535]))
        .collect();
    let image = DecodedImage {
        width: 64,
        height: 64,
        pixels: if depth == 16 {
            Pixels::Rgba16(samples)
        } else {
            Pixels::Rgba8(samples.iter().map(|v| (v / 257) as u8).collect())
        },
        icc: if tagged {
            Some(vw_raster::standard_srgb_profile().unwrap())
        } else {
            None
        },
        source_asset: AssetId::hash(b"synthetic-assisted-source"),
        original_available: true,
        orientation_applied: 1,
    };
    let bytes = vw_raster::export(
        &image,
        &vw_raster::ExportRequest {
            format: if depth == 16 {
                vw_raster::ExportFormat::Png16
            } else {
                vw_raster::ExportFormat::Png8
            },
            region: None,
            revision: vw_proto::v1::Revision {
                host_seq: 0,
                state_hash: vec![0; 32],
            },
            alpha: vw_raster::AlphaPolicy::Preserve,
            color: vw_raster::ColorPolicy::Preserve,
            allow_depth_reduction: false,
            memory_budget_bytes: BUDGET,
            capture_session: None,
            frame_id: None,
        },
    )
    .unwrap()
    .bytes;
    (image, bytes)
}
async fn project(path: &std::path::Path, bytes: Vec<u8>) -> Arc<ProjectSession> {
    let mut options = create(path);
    options.source = bytes;
    create_image_project(options, cancel()).await.unwrap()
}
async fn document(project: &Arc<ProjectSession>) -> SelectionDocument {
    project.selection_document(id(2), cancel()).await.unwrap()
}
async fn acquire(
    project: &Arc<ProjectSession>,
    doc: &SelectionDocument,
    assume: bool,
) -> Arc<AssistedSourcePixels> {
    project
        .assisted_source_pixels(
            doc.binding.clone(),
            doc.width,
            doc.height,
            doc.source_bit_depth,
            None,
            assume,
            BUDGET,
            cancel(),
        )
        .await
        .unwrap()
}
fn retire(delivery: AssistedSourcePixelsDelivery) {
    let AssistedSourcePixelsDelivery { rgba, native_lease } = delivery;
    drop(rgba); // Foreign adapter must do the equivalent before releaseCharge.
    native_lease.close().unwrap();
    native_lease.close().unwrap();
}
fn metadata(n: u8, info: &ProjectInfo) -> WorkflowMetadata {
    WorkflowMetadata {
        transaction_id: id(n),
        device_id: info.device_id.clone(),
        first_lamport: info.next_lamport,
        created_at_ms: TIME + i64::from(n),
    }
}

#[tokio::test]
async fn one_publication_keeps_native_abi_and_foreign_charges_after_source_and_project_release() {
    let _serial = TESTS.lock().await;
    for depth in [8, 16] {
        let root = temp();
        let path = root.path().join("p");
        let (_, original) = fixture(depth, false);
        let project = project(&path, original.clone()).await;
        let doc = document(&project).await;
        let initial = project.info().await.unwrap();
        let pool = shared_session_resources().unwrap();
        let baseline = pool.used_bytes();
        let owner = acquire(&project, &doc, true).await;
        let info = owner.describe().unwrap();
        assert_eq!(info.binding, doc.binding);
        assert_eq!(
            (info.width, info.height, info.source_bit_depth),
            (64, 64, u32::from(depth))
        );
        assert_eq!(info.rgba_bytes, 64 * 64 * 4);
        assert_eq!(info.page, None);
        let prefetched = owner.publication_lease().unwrap();
        let delivery = owner.take_pixels().unwrap();
        assert!(Arc::ptr_eq(&prefetched, &delivery.native_lease));
        assert!(matches!(owner.take_pixels(), Err(SelectionError::Invalid)));
        assert!(pool.used_bytes() > baseline);
        let decoded = vw_raster::decode(&original, vw_raster::DecodeLimits::default()).unwrap();
        let expected = vw_raster::display_rgba8(decoded, BUDGET, true, &|| false).unwrap();
        assert_eq!(delivery.rgba, expected.rgba);
        assert_eq!(project.info().await.unwrap(), initial);
        project.close().await.unwrap();
        let held = pool.used_bytes();
        owner.release_source().unwrap();
        owner.release_source().unwrap();
        drop(owner);
        assert_eq!(pool.used_bytes(), held);
        assert_eq!(delivery.rgba.len(), 64 * 64 * 4);
        retire(delivery);
        assert!(pool.used_bytes() < held);
        assert!(pool.used_bytes() <= baseline);
        let store = vw_store::ProjectStore::open(&path).unwrap();
        let source = store.project().assets.keys().next().unwrap();
        assert_eq!(
            vw_store::BlobStore::new(&path)
                .unwrap()
                .read(source)
                .unwrap(),
            original
        );
    }
}

#[tokio::test]
async fn undelivered_release_is_idempotent_and_never_exposes_pixels_again() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let (_, bytes) = fixture(8, false);
    let project = project(&root.path().join("p"), bytes).await;
    let doc = document(&project).await;
    let pool = shared_session_resources().unwrap();
    let baseline = pool.used_bytes();
    let owner = acquire(&project, &doc, true).await;
    assert!(pool.used_bytes() > baseline);
    owner.release_source().unwrap();
    owner.release_source().unwrap();
    assert!(matches!(owner.describe(), Err(SelectionError::Closed)));
    assert!(matches!(owner.take_pixels(), Err(SelectionError::Closed)));
    assert_eq!(pool.used_bytes(), baseline);
    project.close().await.unwrap();
}

#[tokio::test]
async fn original_pixels_ignore_annotations_and_canonical_tonal_parameters() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let path = root.path().join("p");
    let (_, bytes) = fixture(8, false);
    let project = project(&path, bytes.clone()).await;
    let before = acquire(&project, &document(&project).await, true).await;
    let original_binding = before.describe().unwrap().binding;
    let raw = before.take_pixels().unwrap();
    before.release_source().unwrap();
    let info = project.info().await.unwrap();
    project
        .apply_edit(
            edit(20, info.next_lamport),
            vec![EditCommand::Create {
                object_id: id(21),
                layer_id: id(3),
                shape: NewShape::Rectangle {
                    rectangle: QueryRect {
                        x: 0.0,
                        y: 0.0,
                        width: 60.0,
                        height: 60.0,
                    },
                },
                style: ObjectStyle {
                    rgba: 0xff0000ff,
                    width: 1.0,
                    screen_constant_width: false,
                    fill: Some(0xff0000ff),
                },
                transform: Transform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: 0.0,
                    f: 0.0,
                },
            }],
            cancel(),
        )
        .await
        .unwrap();
    let now = project.info().await.unwrap();
    let plan = project
        .clone()
        .prepare_tone_control(
            WorkflowBinding {
                project_id: now.project_id.clone(),
                document_id: id(2),
                host_seq: now.host_seq,
                state_hash: now.state_hash.clone(),
            },
            metadata(22, &now),
            ToneControlCommand::Set {
                layer_id: id(30),
                object_id: id(31),
                settings: ImageToneSettings {
                    brightness: -0.5,
                    contrast: 0.2,
                    input_black: 0.0,
                    input_white: 1.0,
                    gamma: 1.2,
                    output_black: 0.0,
                    output_white: 1.0,
                },
                enabled: true,
                opacity: 0.75,
            },
            BUDGET,
            cancel(),
        )
        .await
        .unwrap();
    plan.commit(cancel()).await.unwrap();
    plan.dispose();
    let doc = document(&project).await;
    let after = acquire(&project, &doc, true).await;
    let delivered = after.take_pixels().unwrap();
    after.release_source().unwrap();
    assert_eq!(delivered.rgba, raw.rgba);
    assert_ne!(doc.binding, original_binding);
    retire(delivered);
    retire(raw);
    project.close().await.unwrap();
    assert_eq!(
        vw_store::BlobStore::new(&path)
            .unwrap()
            .read(&AssetId::hash(&bytes))
            .unwrap(),
        bytes
    );
}

#[tokio::test]
async fn stale_identity_dimensions_depth_page_color_budget_and_cancellation_refuse_without_leaks() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let (_, bytes) = fixture(8, false);
    let project = project(&root.path().join("p"), bytes).await;
    let doc = document(&project).await;
    let pool = shared_session_resources().unwrap();
    let baseline = pool.used_bytes();
    let mut stale = doc.binding.clone();
    stale.state_hash = "f".repeat(64);
    assert!(matches!(
        project
            .assisted_source_pixels(stale, 64, 64, 8, None, true, BUDGET, cancel())
            .await,
        Err(SelectionError::Conflict)
    ));
    let mut wrong_source = doc.binding.clone();
    wrong_source.source_asset_id = AssetId::hash(b"different original").to_string();
    assert!(matches!(
        project
            .assisted_source_pixels(wrong_source, 64, 64, 8, None, true, BUDGET, cancel())
            .await,
        Err(SelectionError::Conflict)
    ));
    assert!(matches!(
        project
            .assisted_source_pixels(doc.binding.clone(), 63, 64, 8, None, true, BUDGET, cancel())
            .await,
        Err(SelectionError::Conflict)
    ));
    assert!(matches!(
        project
            .assisted_source_pixels(
                doc.binding.clone(),
                64,
                64,
                16,
                None,
                true,
                BUDGET,
                cancel()
            )
            .await,
        Err(SelectionError::Conflict)
    ));
    assert!(matches!(
        project
            .assisted_source_pixels(
                doc.binding.clone(),
                64,
                64,
                32,
                None,
                true,
                BUDGET,
                cancel()
            )
            .await,
        Err(SelectionError::Unsupported)
    ));
    assert!(matches!(
        project
            .assisted_source_pixels(
                doc.binding.clone(),
                64,
                64,
                8,
                Some(0),
                true,
                BUDGET,
                cancel()
            )
            .await,
        Err(SelectionError::Unsupported)
    ));
    assert!(matches!(
        project
            .assisted_source_pixels(
                doc.binding.clone(),
                64,
                64,
                8,
                None,
                false,
                BUDGET,
                cancel()
            )
            .await,
        Err(SelectionError::Unsupported)
    ));
    assert!(matches!(
        project
            .assisted_source_pixels(doc.binding.clone(), 64, 64, 8, None, true, 1, cancel())
            .await,
        Err(SelectionError::Memory { .. })
    ));
    let token = cancel();
    token.cancel();
    assert!(matches!(
        project
            .assisted_source_pixels(doc.binding.clone(), 64, 64, 8, None, true, BUDGET, token)
            .await,
        Err(SelectionError::Cancelled)
    ));
    assert_eq!(pool.used_bytes(), baseline);
    let now = project.info().await.unwrap();
    project
        .apply_edit(
            edit(23, now.next_lamport),
            vec![EditCommand::Create {
                object_id: id(24),
                layer_id: id(3),
                shape: NewShape::Line {
                    points: vec![Point { x: 1.0, y: 1.0 }, Point { x: 10.0, y: 10.0 }],
                },
                style: ObjectStyle {
                    rgba: 0x123456ff,
                    width: 1.0,
                    screen_constant_width: false,
                    fill: None,
                },
                transform: Transform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: 0.0,
                    f: 0.0,
                },
            }],
            cancel(),
        )
        .await
        .unwrap();
    assert!(matches!(
        project
            .assisted_source_pixels(doc.binding.clone(), 64, 64, 8, None, true, BUDGET, cancel())
            .await,
        Err(SelectionError::Conflict)
    ));
    project.close().await.unwrap();
    assert!(matches!(
        project
            .assisted_source_pixels(doc.binding, 64, 64, 8, None, true, BUDGET, cancel())
            .await,
        Err(SelectionError::Closed)
    ));
}

#[tokio::test]
async fn shared_pool_pressure_refuses_before_opening_an_unavailable_original() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let path = root.path().join("p");
    let (_, bytes) = fixture(8, false);
    let project = project(&path, bytes.clone()).await;
    let doc = document(&project).await;
    let pool = shared_session_resources().unwrap();
    let baseline = pool.used_bytes();
    let pressure = pool
        .reserve_foreign(pool.limit_bytes() - baseline - 1)
        .unwrap();
    let source_path = vw_store::BlobStore::new(&path)
        .unwrap()
        .path(&AssetId::hash(&bytes))
        .unwrap();
    std::fs::remove_file(&source_path).unwrap();
    assert!(matches!(
        project
            .assisted_source_pixels(doc.binding.clone(), 64, 64, 8, None, true, BUDGET, cancel())
            .await,
        Err(SelectionError::Memory { .. })
    ));
    pressure.close().unwrap();
    assert_eq!(pool.used_bytes(), baseline);
    std::fs::write(source_path, bytes).unwrap();
    let owner = acquire(&project, &doc, true).await;
    owner.release_source().unwrap();
    assert_eq!(pool.used_bytes(), baseline);
    project.close().await.unwrap();
}

#[tokio::test]
async fn original_hash_corruption_refuses_and_failed_acquisition_returns_every_reservation() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let path = root.path().join("p");
    let (_, bytes) = fixture(8, false);
    let project = project(&path, bytes.clone()).await;
    let doc = document(&project).await;
    let initial = project.info().await.unwrap();
    let pool = shared_session_resources().unwrap();
    let baseline = pool.used_bytes();
    let source_path = vw_store::BlobStore::new(&path)
        .unwrap()
        .path(&AssetId::hash(&bytes))
        .unwrap();
    let mut damaged = bytes.clone();
    damaged[0] ^= 1;
    std::fs::write(&source_path, damaged).unwrap();
    assert!(matches!(
        project
            .assisted_source_pixels(doc.binding, 64, 64, 8, None, true, BUDGET, cancel())
            .await,
        Err(SelectionError::Corrupt)
    ));
    assert_eq!(pool.used_bytes(), baseline);
    assert_eq!(project.info().await.unwrap(), initial);
    std::fs::write(source_path, bytes).unwrap();
    project.close().await.unwrap();
}

#[tokio::test]
async fn tagged_oriented_sixteen_bit_tiff_uses_the_canonical_format_decoder() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let image = DecodedImage {
        width: 2,
        height: 3,
        pixels: Pixels::Rgba16(
            (0..6)
                .flat_map(|n| [256 + n, 32768 + n, 65535 - n, 65535])
                .collect(),
        ),
        icc: Some(vw_raster::standard_srgb_profile().unwrap()),
        source_asset: AssetId::hash(b"oriented-assisted-source"),
        original_available: true,
        orientation_applied: 1,
    };
    let mut encoded = Cursor::new(Vec::new());
    vw_formats::encode_to(
        &image,
        vw_formats::ExportFormat::Tiff16,
        vw_formats::Limits::default(),
        &mut encoded,
        &|| false,
    )
    .unwrap();
    let mut bytes = encoded.into_inner();
    let ifd = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes(bytes[ifd..ifd + 2].try_into().unwrap()) as usize;
    let orientation = (0..count)
        .map(|n| ifd + 2 + n * 12)
        .find(|&at| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()) == 274)
        .unwrap();
    bytes[orientation + 8..orientation + 12].copy_from_slice(&6u32.to_le_bytes());
    let project = project(&root.path().join("p"), bytes.clone()).await;
    let doc = document(&project).await;
    assert_eq!((doc.width, doc.height, doc.source_bit_depth), (3, 2, 16));
    let owner = acquire(&project, &doc, false).await;
    let delivery = owner.take_pixels().unwrap();
    owner.release_source().unwrap();
    let decoded = vw_formats::decode(&bytes, vw_formats::Limits::default(), &|| false)
        .unwrap()
        .image;
    let expected = vw_raster::display_rgba8(decoded, BUDGET, false, &|| false).unwrap();
    assert_eq!(delivery.rgba, expected.rgba);
    retire(delivery);
    project.close().await.unwrap();
}

#[tokio::test]
async fn large_source_metadata_is_admitted_before_any_original_file_allocation() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let seed_path = root.path().join("seed");
    let (_, bytes) = fixture(8, false);
    let seed = project(&seed_path, bytes).await;
    seed.close().await.unwrap();
    let seed_store = vw_store::ProjectStore::open(&seed_path).unwrap();
    let mut model = seed_store.project().clone();
    drop(seed_store);
    let asset = model.assets.values_mut().next().unwrap();
    asset.width = 4096;
    asset.height = 4096;
    let path = root.path().join("large");
    drop(
        vw_store::ProjectStore::create(&path, model, DeviceId::try_from(device()).unwrap(), TIME)
            .unwrap(),
    );
    // No blob exists. Memory must win before source path/read/decoder work.
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let doc = document(&project).await;
    let pool = shared_session_resources().unwrap();
    let baseline = pool.used_bytes();
    assert!(matches!(
        project
            .assisted_source_pixels(doc.binding, 4096, 4096, 8, None, true, BUDGET, cancel())
            .await,
        Err(SelectionError::Memory { .. })
    ));
    assert_eq!(pool.used_bytes(), baseline);
    project.close().await.unwrap();
}

#[tokio::test]
async fn capture_original_uses_the_same_page_free_exact_source_contract() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let (_, bytes) = fixture(8, false);
    let input = root.path().join("capture.png");
    std::fs::write(&input, &bytes).unwrap();
    let project = create_capture_project(
        CreateFileImageProject {
            path: root.path().join("p").to_string_lossy().into(),
            project_id: id(1),
            document_id: id(2),
            layer_id: id(3),
            device_id: device(),
            title: "Synthetic assisted capture".into(),
            source_path: input.to_string_lossy().into(),
            work_directory: root.path().to_string_lossy().into(),
            now_ms: TIME,
            memory_budget_bytes: BUDGET,
            max_encoded_bytes: 64 * 1024 * 1024,
            max_scratch_bytes: 400_000_000,
        },
        CaptureImportDescriptor {
            explicit_owner_action: true,
            capture_session_id: id(4),
            frame_id: 1,
            geometry_revision: 7,
            platform: "windows".into(),
            source_kind: "window".into(),
            physical_x: -100,
            physical_y: -100,
            width: 64,
            height: 64,
            dpi_scale: 1.5,
            monotonic_timestamp_ns: 1001,
            captured_at_ms: TIME,
            window_handle: 777,
            monitor_id: String::new(),
            expected_source_asset_id: None,
        },
        cancel(),
    )
    .await
    .unwrap();
    let doc = document(&project).await;
    let owner = acquire(&project, &doc, true).await;
    assert_eq!(owner.describe().unwrap().page, None);
    let pixels = owner.take_pixels().unwrap();
    owner.release_source().unwrap();
    let expected = vw_raster::display_rgba8(
        vw_raster::decode(&bytes, vw_raster::DecodeLimits::default()).unwrap(),
        BUDGET,
        true,
        &|| false,
    )
    .unwrap();
    assert_eq!(pixels.rgba, expected.rgba);
    retire(pixels);
    project.close().await.unwrap();
}

#[tokio::test]
async fn page_document_kinds_cannot_be_read_as_image_or_capture_originals() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let seed_path = root.path().join("seed");
    let (_, bytes) = fixture(8, false);
    let seed = project(&seed_path, bytes.clone()).await;
    seed.close().await.unwrap();
    let store = vw_store::ProjectStore::open(&seed_path).unwrap();
    let model = store.project().clone();
    drop(store);
    for (index, kind) in [
        vw_proto::v1::DocumentKind::Pdf,
        vw_proto::v1::DocumentKind::Svg,
    ]
    .into_iter()
    .enumerate()
    {
        let mut model = model.clone();
        model.documents.values_mut().next().unwrap().definition.kind = kind as i32;
        let path = root.path().join(format!("page-{index}"));
        drop(
            vw_store::ProjectStore::create(
                &path,
                model,
                DeviceId::try_from(device()).unwrap(),
                TIME,
            )
            .unwrap(),
        );
        vw_store::BlobStore::new(&path)
            .unwrap()
            .put(&bytes)
            .unwrap();
        let project = open_project(path.to_string_lossy().into(), cancel())
            .await
            .unwrap();
        let info = project.info().await.unwrap();
        let binding = SelectionBinding {
            project_id: info.project_id,
            document_id: id(2),
            source_asset_id: AssetId::hash(&bytes).to_string(),
            host_seq: info.host_seq,
            state_hash: info.state_hash,
        };
        assert!(matches!(
            project
                .assisted_source_pixels(binding, 64, 64, 8, None, true, BUDGET, cancel())
                .await,
            Err(SelectionError::Unsupported)
        ));
        project.close().await.unwrap();
    }
}
