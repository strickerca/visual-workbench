//! Synthetic portable copying/retirement checks, not Android codec execution.
#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use std::{
    io::{Seek, SeekFrom},
    time::Duration,
};
const MIB: u64 = 1024 * 1024;
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
struct Fixture {
    context: Context,
    android: PathBuf,
    original: Vec<u8>,
}
fn export(format: vw_raster::ExportFormat) -> Vec<u8> {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[21, 42, 84])
            .unwrap();
    }
    let image = vw_raster::decode(&png, vw_raster::DecodeLimits::default()).unwrap();
    vw_raster::export(
        &image,
        &vw_raster::ExportRequest {
            format,
            region: None,
            revision: vw_proto::v1::Revision {
                host_seq: 0,
                state_hash: vec![0; 32],
            },
            alpha: vw_raster::AlphaPolicy::Preserve,
            color: vw_raster::ColorPolicy::Preserve,
            allow_depth_reduction: false,
            memory_budget_bytes: 64 * MIB,
            capture_session: None,
            frame_id: None,

            capture_provenance: None,
        },
    )
    .unwrap()
    .bytes
}
fn lossless() -> Vec<u8> {
    let encoded = export(vw_raster::ExportFormat::WebpLossless);
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
    panic!("fixture lossless chunk")
}
impl Fixture {
    fn new() -> Self {
        let original = export(vw_raster::ExportFormat::Jpeg { quality: 90 });
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&original).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let budget = SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap();
        // CompletedOutput requires the canonical native path representation.
        // On Windows canonicalize adds the extended-length prefix; create the
        // fixture under that parent instead of weakening the production check.
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let work = tempfile::tempdir_in(parent).unwrap();
        let source = input::prepare(
            input::RootOwnedPickedOriginal { file, work },
            input::GiantInputLimits {
                max_original_bytes: MIB,
                max_work_disk_bytes: 512 * MIB,
                assume_untagged_srgb: true,
            },
            &AtomicBool::new(false),
            &Cancellation::new(),
            &budget,
            &vw_store::RealFreeSpace,
        )
        .unwrap();
        let work = source.staging.path().join("android-work");
        std::fs::create_dir(&work).unwrap();
        let marker = "01234567-89ab-4cde-8fab-0123456789ab";
        let android = work.join(format!("vw-jpeg-{marker}"));
        std::fs::create_dir(&android).unwrap();
        std::fs::write(android.join(".owner"), marker).unwrap();
        std::fs::write(android.join("preview.jpg"), &original).unwrap();
        std::fs::create_dir(android.join("L0")).unwrap();
        std::fs::write(android.join("L0/0_0.webp"), lossless()).unwrap();
        Self {
            context: Context {
                source,
                closed: Arc::new(AtomicBool::new(false)),
                cancel: Arc::new(Cancellation::new()),
                budget,
                disk_cache_bytes: 32 * MIB,
                deadline: Instant::now() + Duration::from_secs(60),
            },
            android,
            original,
        }
    }
    fn output(&self) -> CompletedOutput {
        Self::describe(&self.context, &self.android)
    }
    fn describe(context: &Context, android: &std::path::Path) -> CompletedOutput {
        let info = &context.source.info;
        CompletedOutput {
            directory: android.to_path_buf(),
            source_sha256: tiles::hex(&info.sha256),
            encoded_bytes: info.encoded_bytes,
            raw_width: info.raw_width,
            raw_height: info.raw_height,
            orientation: info.orientation,
            width: info.width,
            height: info.height,
            preview_width: 1,
            preview_height: 1,
        }
    }
}
// One input-owning test keeps the existing two-input process limit meaningful
// beside the producer lane's independently serialized fixture family.
#[tokio::test]
async fn typed_copy_refusals_independent_output_and_failed_android_retirement_preserve_owners() {
    let _serial = SERIAL.lock().await;
    let mut f = Fixture::new();
    let baseline = f.context.budget.used_bytes();
    assert_eq!(baseline, 4 * MIB);
    let android_charge = f
        .context
        .budget
        .reserve(MemoryKind::Foreign, 80 * MIB)
        .unwrap();
    let mut wrong = f.output();
    wrong.source_sha256 = "0".repeat(64);
    assert!(matches!(copy_output(wrong, &f.context), Err(Error::Stale)));
    std::fs::write(f.android.join("unknown"), b"x").unwrap();
    assert!(matches!(
        copy_output(f.output(), &f.context),
        Err(Error::Invalid)
    ));
    std::fs::remove_file(f.android.join("unknown")).unwrap();
    std::fs::write(f.android.join(".owner"), b"foreign").unwrap();
    assert!(matches!(
        copy_output(f.output(), &f.context),
        Err(Error::Invalid)
    ));
    std::fs::write(
        f.android.join(".owner"),
        b"01234567-89ab-4cde-8fab-0123456789ab",
    )
    .unwrap();
    let tile = std::fs::read(f.android.join("L0/0_0.webp")).unwrap();
    std::fs::write(f.android.join("L0/0_0.webp"), b"bad").unwrap();
    assert!(copy_output(f.output(), &f.context).is_err());
    std::fs::write(f.android.join("L0/0_0.webp"), &tile).unwrap();
    let build = copy_output(f.output(), &f.context).unwrap();
    assert_eq!(
        f.context.budget.used_bytes(),
        baseline + 80 * MIB + 25 * MIB
    );
    assert!(matches!(
        after_android_retirement(&f.context, true, 1, Some(Ok(build))),
        Err(Error::Storage)
    ));
    assert_eq!(f.context.budget.used_bytes(), baseline + 80 * MIB); // Failed Android close still owns all 80 MiB.
    assert!(f.android.exists());
    assert_eq!(
        std::fs::read(&f.context.source.info.source_path).unwrap(),
        f.original
    );
    drop(android_charge);
    let blocker = f
        .context
        .budget
        .reserve(
            MemoryKind::Foreign,
            f.context.budget.limit_bytes() - baseline - 24 * MIB,
        )
        .unwrap();
    assert!(matches!(
        copy_output(f.output(), &f.context),
        Err(Error::Memory { .. })
    ));
    assert_eq!(f.context.budget.used_bytes(), baseline + blocker.bytes());
    drop(blocker);
    let build = copy_output(f.output(), &f.context).unwrap();
    let native = build.directory();
    assert_ne!(native, f.android);
    std::fs::remove_dir_all(&f.android).unwrap();
    // Simulate successful exact Android retirement, then verify solely native
    // owned copies after the foreign directory has actually disappeared.
    let build = after_android_retirement(&f.context, true, 0, Some(Ok(build))).unwrap();
    cache::verify_output(
        &build,
        &f.context.closed,
        None,
        &f.context.cancel,
        &f.context.budget,
    )
    .unwrap();
    assert_eq!(std::fs::read(native.join("L0/0_0.webp")).unwrap(), tile);
    cache::release(Arc::new(build)).unwrap();
    assert_eq!(f.context.budget.used_bytes(), baseline);
    f.context.deadline = Instant::now();
    assert!(matches!(
        copy_output(f.output(), &f.context),
        Err(Error::Limit)
    ));
    f.context.deadline = Instant::now() + Duration::from_secs(60);
    f.context.cancel.cancel();
    assert!(matches!(
        copy_output(f.output(), &f.context),
        Err(Error::Cancelled)
    ));
    assert_eq!(f.context.budget.used_bytes(), baseline);
}

#[tokio::test]
async fn real_publication_adopts_all_four_parts_and_keeps_only_its_project_charge_until_close() {
    let _serial = SERIAL.lock().await;
    let f = Fixture::new();
    let budget = f.context.budget.clone();
    let original = f.original.clone();
    let source_id =
        vw_model::AssetId::try_from(tiles::hex(&f.context.source.info.asset_id)).unwrap();
    let build = copy_output(f.output(), &f.context).unwrap();
    std::fs::remove_dir_all(&f.android).unwrap();
    let build = after_android_retirement(&f.context, true, 0, Some(Ok(build))).unwrap();
    let destination = tempfile::tempdir().unwrap();
    let path = destination.path().join("published");
    let id = |n| {
        vw_model::Id::from_parts(1_700_000_000_000, [n; 10])
            .unwrap()
            .to_string()
    };
    let options = super::super::publish::GiantPublishOptions {
        path: path.to_str().unwrap().into(),
        project_id: id(11),
        document_id: id(12),
        layer_id: id(13),
        device_id: vw_model::DeviceId::from_bytes([14; 16]).to_string(),
        title: "root publication fixture".into(),
        now_ms: 0,
        disk_cache_bytes: f.context.disk_cache_bytes,
    };
    let published = super::super::publish::publish(
        &build,
        options,
        &f.context.closed,
        &f.context.cancel,
        &budget,
    )
    .unwrap();
    let project = crate::giant_images::adopt_published_project(published).unwrap();
    assert_eq!(project.info().await.unwrap().project_id, id(11));
    cache::release(Arc::new(build)).unwrap();
    input::release(f.context.source).unwrap();
    // Registry entries are bounded and may be evicted concurrently by their own
    // census test. No prepared/build owner is credited while it is still live.
    assert!((32 * MIB..=32 * MIB + 64 * 1024).contains(&budget.used_bytes()));
    project.close().await.unwrap();
    assert!(budget.used_bytes() <= 64 * 1024);
    let blobs = vw_store::BlobStore::new(&path).unwrap();
    assert_eq!(
        std::fs::read(blobs.path(&source_id).unwrap()).unwrap(),
        original
    );
    vw_store::ProjectStore::open(&path)
        .unwrap()
        .integrity_check()
        .unwrap();
}

/// Portable foreign-retirement witness, not Android/JNI execution. The real
/// PreparedSource/OwnedDirectory and 4 MiB admission are closed after each
/// failed exit, while the simulated Java owner still holds its exact 80 MiB.
#[tokio::test]
async fn closing_native_input_after_uncertain_foreign_exit_preserves_marker_until_retry() {
    let _serial = SERIAL.lock().await;
    for exit in [
        "cleanup",
        "jni",
        "cancel",
        "timeout",
        "closed",
        "unwind",
        "false-success",
    ] {
        let Fixture {
            mut context,
            android,
            original,
        } = Fixture::new();
        let budget = context.budget.clone();
        let native_root = context.source.staging.path().to_path_buf();
        let original_path = context.source.info.source_path.clone();
        let marker_path = android.join(".owner");
        let marker = std::fs::read(&marker_path).unwrap();
        let pins = foreign_retirement::ForeignSources::new();
        let retired = pins.retain(19, context.source.clone()).unwrap();
        let foreign_charge = budget.reserve(MemoryKind::Foreign, 80 * MIB).unwrap();
        match exit {
            "cleanup" => {
                assert!(matches!(
                    after_android_retirement(
                        &context,
                        true,
                        1,
                        Some(Ok(copy_output(
                            Fixture::describe(&context, &android),
                            &context
                        )
                        .unwrap()))
                    ),
                    Err(Error::Storage)
                ));
            }
            "cancel" => {
                context.cancel.cancel();
                assert!(matches!(
                    after_android_retirement(&context, false, 2, None),
                    Err(Error::Cancelled)
                ));
            }
            "timeout" => {
                context.deadline = Instant::now();
                assert!(matches!(
                    after_android_retirement(&context, false, 7, None),
                    Err(Error::Limit)
                ));
            }
            "closed" => {
                context
                    .closed
                    .store(true, std::sync::atomic::Ordering::Release);
                assert!(matches!(
                    after_android_retirement(&context, false, 8, None),
                    Err(Error::Closed)
                ));
            }
            "unwind" => {
                assert!(
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _scoped_source = context.source.clone();
                        std::panic::resume_unwind(Box::new("fixture JNI unwind"));
                    }))
                    .is_err()
                );
            }
            // No Java retirement callback on failed attachment/JNI or a bogus
            // success status. Exiting ACTIVE must not remove the source pin.
            "jni" | "false-success" => {}
            _ => unreachable!(),
        }
        let Context { source, .. } = context;
        input::release(source).unwrap();
        assert!(
            matches!(retired.require_confirmed(), Err(Error::Storage)),
            "{exit}"
        );
        assert_eq!(std::fs::read(&original_path).unwrap(), original, "{exit}");
        assert_eq!(std::fs::read(&marker_path).unwrap(), marker, "{exit}");
        assert_eq!(budget.used_bytes(), 84 * MIB, "{exit}");
        // Simulate the exact Java retry: the preserved identity permits actual
        // output deletion, then its 80 MiB release, then native confirmation.
        std::fs::remove_dir_all(&android).unwrap();
        drop(foreign_charge);
        pins.confirm_retired(19).unwrap();
        retired.require_confirmed().unwrap();
        assert!(!native_root.exists(), "{exit}");
        assert_eq!(budget.used_bytes(), 0, "{exit}");
        pins.confirm_retired(19).unwrap(); // Repeated callback credits nothing.
    }
}

#[tokio::test]
async fn foreign_source_pins_are_bounded_and_confirmation_is_exact() {
    let _serial = SERIAL.lock().await;
    let f = Fixture::new();
    let pins = foreign_retirement::ForeignSources::new();
    assert!(matches!(
        pins.retain(0, f.context.source.clone()),
        Err(Error::Invalid)
    ));
    let first = pins.retain(1, f.context.source.clone()).unwrap();
    assert!(matches!(
        pins.retain(1, f.context.source.clone()),
        Err(Error::Invalid)
    ));
    let second = pins.retain(2, f.context.source.clone()).unwrap();
    assert!(matches!(
        pins.retain(3, f.context.source.clone()),
        Err(Error::Backpressure)
    ));
    pins.confirm_retired(999).unwrap();
    assert!(first.require_confirmed().is_err());
    assert!(second.require_confirmed().is_err());
    pins.confirm_retired(1).unwrap();
    first.require_confirmed().unwrap();
    assert!(second.require_confirmed().is_err());
    pins.confirm_retired(2).unwrap();
    second.require_confirmed().unwrap();
}
