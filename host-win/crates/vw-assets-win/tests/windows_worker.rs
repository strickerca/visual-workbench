#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::{fs, io::Write, path::PathBuf};
use vw_assets_win::*;
use vw_stream::{DESKTOP_SESSION_BYTES, SessionBudget};

fn fixture() -> (Vec<u8>, Vec<u8>) {
    let mut pixels = Vec::new();
    for y in 0..257 {
        for x in 0..513 {
            pixels.extend_from_slice(&[(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8]);
        }
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, 513, 257);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
    }
    (encoded, pixels)
}
fn configured() -> (LockedHelper, LockedRuntime) {
    let helper = PathBuf::from(
        std::env::var_os("VW_PYRAMID_TEST_HELPER").expect("root must supply built owned helper"),
    );
    let hash = std::env::var("VW_PYRAMID_TEST_HELPER_SHA256")
        .expect("root must supply independently bound helper hash");
    let runtime = PathBuf::from(
        std::env::var_os("VW_PYRAMID_TEST_RUNTIME")
            .expect("root must supply flat verified DLL package"),
    );
    (
        LockedHelper::open(&helper, &hash).unwrap(),
        LockedRuntime::open(&runtime).unwrap(),
    )
}
#[test]
fn runtime_refuses_missing_or_extra_files() {
    let temp = tempfile::tempdir().unwrap();
    assert!(matches!(
        LockedRuntime::open(temp.path()),
        Err(Error::Binding)
    ));
    fs::write(temp.path().join("unlisted.dll"), b"not a dll").unwrap();
    assert!(matches!(
        LockedRuntime::open(temp.path()),
        Err(Error::Binding)
    ));
}

#[test]
#[ignore = "central Windows lane must supply packaged pinned runtime and built helper"]
fn cancellation_after_launch_keeps_worker_admission_until_job_join() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let (helper, runtime) = configured();
    let work = tempfile::tempdir().unwrap();
    let (encoded, _) = fixture();
    let input = work.path().join("input.png");
    fs::write(&input, &encoded).unwrap();
    let request = BuildRequest {
        original_hash: *blake3::hash(&encoded).as_bytes(),
        original_bytes: encoded.len() as u64,
        width: 513,
        height: 257,
        orientation: 1,
        disk_budget: 512 * 1024 * 1024,
        timeout_ms: 60_000,
    };
    let budget = SessionBudget::new(DESKTOP_SESSION_BYTES).unwrap();
    let observed = Arc::new(AtomicBool::new(false));
    let token = Cancellation::from_probe({
        let budget = budget.clone();
        let observed = observed.clone();
        Arc::new(move || {
            if budget.used_bytes() >= WORKER_MEMORY_BYTES {
                observed.store(true, Ordering::Release);
                true
            } else {
                false
            }
        })
    });
    let destination = work.path().join("cancelled");
    assert_eq!(
        build_budgeted(
            &helper,
            &runtime,
            &input,
            &destination,
            &request,
            &budget,
            &token
        )
        .err(),
        Some(Error::Cancelled)
    );
    assert!(observed.load(Ordering::Acquire));
    // The probe first fires after suspended creation/Job assignment, before
    // resume; return requires Child's actual process/Job join to have completed.
    assert_eq!(budget.used_bytes(), 0);
    assert!(!destination.exists());
    assert_eq!(fs::read(input).unwrap(), encoded);
}

#[test]
#[ignore = "central Windows lane must supply packaged pinned runtime and built helper"]
fn actual_worker_lossless_base_mixed_pyramid_no_clobber_and_cancellation() {
    let (helper, runtime) = configured();
    let temp = tempfile::tempdir().unwrap();
    let (encoded, pixels) = fixture();
    let input = temp.path().join("input.png");
    let mut file = fs::File::create(&input).unwrap();
    file.write_all(&encoded).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let request = BuildRequest {
        original_hash: *blake3::hash(&encoded).as_bytes(),
        original_bytes: encoded.len() as u64,
        width: 513,
        height: 257,
        orientation: 1,
        disk_budget: 512 * 1024 * 1024,
        timeout_ms: 60_000,
    };
    let budget = SessionBudget::new(DESKTOP_SESSION_BYTES).unwrap();
    let dest = temp.path().join("cache");
    let owner = build_budgeted(
        &helper,
        &runtime,
        &input,
        &dest,
        &request,
        &budget,
        &Cancellation::default(),
    )
    .unwrap();
    let manifest = owner.manifest();
    assert!(owner.charge().belongs_to(&budget));
    assert_eq!(budget.used_bytes(), owner.charge().bytes());
    for record in manifest.tiles() {
        let bytes = fs::read(dest.join(record.relative_path())).unwrap();
        record.verify(&bytes).unwrap();
        let image = vw_raster::decode(
            &bytes,
            vw_raster::DecodeLimits {
                max_encoded_bytes: 2 * 1024 * 1024,
                max_pixels: 256 * 256,
                max_memory_bytes: 32 * 1024 * 1024,
            },
        )
        .unwrap();
        if record.key.level == 0 {
            let region = manifest.pyramid().region(record.key).unwrap();
            let vw_raster::Pixels::Rgba8(rgba) = image.pixels else {
                panic!("8-bit cache");
            };
            for y in 0..region.height {
                for x in 0..region.width {
                    let source = ((region.y + y) * 513 + region.x + x) as usize * 3;
                    let tile = (y * region.width + x) as usize * 4;
                    assert_eq!(&rgba[tile..tile + 3], &pixels[source..source + 3]);
                    assert_eq!(rgba[tile + 3], 255);
                }
            }
        }
    }
    let before = fs::read(dest.join("pyramid.vwt")).unwrap();
    assert_eq!(
        build_budgeted(
            &helper,
            &runtime,
            &input,
            &dest,
            &request,
            &budget,
            &Cancellation::default()
        )
        .err(),
        Some(Error::Storage)
    );
    assert_eq!(fs::read(dest.join("pyramid.vwt")).unwrap(), before);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        build_budgeted(
            &helper,
            &runtime,
            &input,
            &temp.path().join("cancelled"),
            &request,
            &budget,
            &cancel
        )
        .err(),
        Some(Error::Cancelled)
    );
    assert!(!temp.path().join("cancelled").exists());
    assert_eq!(fs::read(&input).unwrap(), encoded);
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}
