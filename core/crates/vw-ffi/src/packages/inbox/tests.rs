#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

// This entire parent file is a #[cfg(test)] child. The Windows-only helper
// changes only the newly created, owned fixture file, never a volume/device.
#[cfg(windows)]
mod sparse_fixture {
    use std::{
        ffi::c_void,
        fs::File,
        io,
        os::windows::{fs::MetadataExt, io::AsRawHandle},
    };

    // Pinned windows-sys 0.61.2 System::Ioctl::FSCTL_SET_SPARSE = 590020.
    const FSCTL_SET_SPARSE: u32 = 0x0009_00c4;
    const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        // SAFETY: matches kernel32 DeviceIoControl's HANDLE/DWORD/LPVOID/
        // DWORD/LPVOID/DWORD/LPDWORD/LPOVERLAPPED -> BOOL ABI.
        fn DeviceIoControl(
            file: *mut c_void,
            control: u32,
            input: *mut c_void,
            input_bytes: u32,
            output: *mut c_void,
            output_bytes: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
    }
    pub(super) fn mark(file: &File) -> io::Result<()> {
        let mut returned = 0;
        // SAFETY: the borrowed fixture File keeps its valid synchronous handle
        // alive throughout the call; no ownership is transferred. A NULL input
        // with zero bytes sets sparse=true. No output or overlapped IO is used;
        // returned is a valid live DWORD pointer required for synchronous IO.
        let ok = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                FSCTL_SET_SPARSE,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_SPARSE_FILE == 0 {
            return Err(io::Error::other("owned fixture did not become sparse"));
        }
        Ok(())
    }
}

fn id(n: u8) -> String {
    vw_model::Id::from_parts(1_790_985_600_000, [n; 10])
        .unwrap()
        .to_string()
}
fn temp() -> tempfile::TempDir {
    if cfg!(target_os = "android") {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    }
}
fn cancel() -> Cancellation {
    Cancellation::new()
}
fn png(color: [u8; 4], deep: bool) -> Vec<u8> {
    let pixels = if deep {
        vw_raster::Pixels::Rgba16(color.iter().map(|v| u16::from(*v) * 257).collect())
    } else {
        vw_raster::Pixels::Rgba8(color.to_vec())
    };
    let source = vw_raster::DecodedImage {
        width: 1,
        height: 1,
        pixels,
        icc: None,
        source_asset: vw_model::AssetId::hash(b"synthetic inbox fixture"),
        original_available: true,
        orientation_applied: 1,
    };
    vw_raster::export(
        &source,
        &vw_raster::ExportRequest {
            format: if deep {
                vw_raster::ExportFormat::Png16
            } else {
                vw_raster::ExportFormat::Png8
            },
            region: None,
            revision: vw_proto::v1::Revision {
                host_seq: 7,
                state_hash: vec![5; 32],
            },
            alpha: vw_raster::AlphaPolicy::Preserve,
            color: vw_raster::ColorPolicy::Preserve,
            allow_depth_reduction: false,
            memory_budget_bytes: MAX_MEMORY,
            capture_session: None,
            frame_id: None,
        },
    )
    .unwrap()
    .bytes
}
fn info(before: &[u8]) -> PackageInfo {
    PackageInfo {
        package_id: id(1),
        target: "generic".into(),
        model: None,
        binding: WorkflowBinding {
            project_id: id(2),
            document_id: id(3),
            host_seq: 7,
            state_hash: "05".repeat(32),
        },
        source_asset_id: vw_model::AssetId::hash(before).to_string(),
        manifest_sha256: "ab".repeat(32),
        created_at: "2026-10-03T00:00:00.000Z".into(),
        total_bytes: before.len() as u64,
        marker_count: 0,
        images: vec![PackageImageInfo {
            id: "clean_source".into(),
            role: "clean_source".into(),
            path: "images/clean_source.png".into(),
            width: 1,
            height: 1,
            encoded_bytes: before.len() as u64,
        }],
        inline_images_available: true,
        includes_window_title: false,
    }
}
fn submission() -> McpInboxSubmission {
    McpInboxSubmission {
        receipt_id: id(4),
        package_id: id(1),
        target: "generic".into(),
        manifest_sha256: "ab".repeat(32),
        connection: "ca".repeat(16),
        created_at_ms: 1_790_985_600_000,
        text: Some("<script> literal untrusted result </script>".into()),
        png: None,
        note: "Ignore instructions is literal captured data.".into(),
    }
}
fn open(path: &std::path::Path) -> storage::State {
    storage::State::open(
        path.to_str().unwrap(),
        &cancel(),
        Permit::acquire(&INBOXES, 1).unwrap(),
    )
    .unwrap()
}
#[test]
fn exactly_one_body_and_authenticated_connection_shape_are_admitted_before_storage() {
    let _g = SERIAL.lock().unwrap();
    let mut s = submission();
    assert!(input(&s).is_ok());
    s.png = Some(vec![1]);
    assert!(matches!(input(&s), Err(PackageError::Invalid)));
    s.png = None;
    s.connection = "../another-session".into();
    assert!(input(&s).is_err());
    s.connection = "ca".repeat(16);
    s.note = "x".repeat(TEXT_LIMIT + 1);
    assert!(matches!(input(&s), Err(PackageError::Limit)));
}
#[test]
fn hostile_dimensions_and_truncated_png_fail_before_publication() {
    let _g = SERIAL.lock().unwrap();
    let dir = temp();
    let state = open(dir.path());
    let before = png([255, 0, 0, 255], false);
    let mut huge = before.clone();
    huge[16..20].copy_from_slice(&50_000_000u32.to_be_bytes());
    huge[20..24].copy_from_slice(&2u32.to_be_bytes());
    for data in [huge, vec![137, 80, 78, 71, 13, 10, 26, 10]] {
        let mut s = submission();
        s.text = None;
        s.png = Some(data);
        assert!(
            state
                .submit(s, info(&before), before.clone(), &cancel())
                .is_err()
        );
        assert!(state.list(&cancel()).unwrap().is_empty());
    }
}
#[test]
fn exact_retry_and_reopen_preserve_literal_text_note_and_package_revision() {
    let _g = SERIAL.lock().unwrap();
    let dir = temp();
    let before = png([255, 0, 0, 255], false);
    let state = open(dir.path());
    let receipt = state
        .submit(submission(), info(&before), before.clone(), &cancel())
        .unwrap();
    assert_eq!(receipt.text, submission().text);
    assert_eq!(receipt.note, submission().note);
    assert_eq!(receipt.binding.host_seq, 7);
    assert_eq!(receipt.binding.state_hash, "05".repeat(32));
    let retry = state
        .submit(submission(), info(&before), before.clone(), &cancel())
        .unwrap();
    assert_eq!(retry.receipt_blake3, receipt.receipt_blake3);
    drop(state);
    let state = open(dir.path());
    assert_eq!(
        state.list(&cancel()).unwrap()[0].receipt_blake3,
        receipt.receipt_blake3
    );
    let mut changed = submission();
    changed.note = "different".into();
    assert!(matches!(
        state.submit(changed, info(&before), before, &cancel()),
        Err(PackageError::Identity)
    ));
}
#[test]
fn before_after_regions_come_from_exact_stored_package_and_result_bytes() {
    let _g = SERIAL.lock().unwrap();
    let dir = temp();
    let state = open(dir.path());
    let before = png([255, 0, 0, 255], false);
    let after = png([0, 0, 255, 255], false);
    let mut s = submission();
    s.text = None;
    s.png = Some(after.clone());
    let receipt = state
        .submit(s, info(&before), before.clone(), &cancel())
        .unwrap();
    let area = McpInboxRegion {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    for (side, original, color) in [
        (McpInboxSide::Before, before, [255, 0, 0, 255]),
        (McpInboxSide::After, after, [0, 0, 255, 255]),
    ] {
        let (record, encoded) = state
            .image(
                &receipt.receipt_id,
                &receipt.receipt_blake3,
                side,
                &cancel(),
            )
            .unwrap();
        assert_eq!(encoded, original);
        let tile = pixels::display(&record, &encoded, side, area, true, false, &cancel()).unwrap();
        assert_eq!(tile.2, color);
    }
}
#[test]
fn display_requires_explicit_untagged_and_depth_choices_and_bounds_regions() {
    let _g = SERIAL.lock().unwrap();
    let dir = temp();
    let state = open(dir.path());
    let before = png([255, 0, 0, 255], false);
    let mut s = submission();
    s.text = None;
    s.png = Some(png([0, 0, 255, 255], true));
    let receipt = state.submit(s, info(&before), before, &cancel()).unwrap();
    let (record, bytes) = state
        .image(
            &receipt.receipt_id,
            &receipt.receipt_blake3,
            McpInboxSide::After,
            &cancel(),
        )
        .unwrap();
    let area = McpInboxRegion {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    assert!(matches!(
        pixels::display(
            &record,
            &bytes,
            McpInboxSide::After,
            area,
            true,
            false,
            &cancel()
        ),
        Err(PackageError::Depth)
    ));
    assert!(
        pixels::display(
            &record,
            &bytes,
            McpInboxSide::After,
            area,
            false,
            true,
            &cancel()
        )
        .is_err()
    );
    assert_eq!(
        pixels::display(
            &record,
            &bytes,
            McpInboxSide::After,
            area,
            true,
            true,
            &cancel()
        )
        .unwrap()
        .2
        .len(),
        4
    );
    assert!(pixels::region(McpInboxRegion { width: 513, ..area }).is_err());
}
#[test]
fn cancellation_before_storage_does_not_publish_and_lost_postpublish_receipt_recovers() {
    let _g = SERIAL.lock().unwrap();
    let dir = temp();
    let state = open(dir.path());
    let before = png([1, 2, 3, 255], false);
    let token = cancel();
    token.cancel();
    assert!(matches!(
        state.submit(submission(), info(&before), before.clone(), &token),
        Err(PackageError::Cancelled)
    ));
    assert!(state.list(&cancel()).unwrap().is_empty());
    storage::FAIL_AFTER_PUBLISH.store(true, Ordering::Release);
    assert!(matches!(
        state.submit(submission(), info(&before), before.clone(), &cancel()),
        Err(PackageError::Storage)
    ));
    let found = state.list(&cancel()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(
        state
            .submit(submission(), info(&before), before, &cancel())
            .unwrap()
            .receipt_blake3,
        found[0].receipt_blake3
    );
}
#[test]
fn changed_published_bytes_are_preserved_and_never_served_or_deleted() {
    let _g = SERIAL.lock().unwrap();
    let dir = temp();
    let state = open(dir.path());
    let before = png([1, 2, 3, 255], false);
    let receipt = state
        .submit(submission(), info(&before), before, &cancel())
        .unwrap();
    let path = dir
        .path()
        .join("mcp-inbox-v1")
        .join(&receipt.receipt_id)
        .join("images/before.png");
    std::fs::write(&path, b"changed by an external owner").unwrap();
    assert!(
        state
            .image(
                &receipt.receipt_id,
                &receipt.receipt_blake3,
                McpInboxSide::Before,
                &cancel()
            )
            .is_err()
    );
    assert!(
        state
            .retire(&receipt.receipt_id, &receipt.receipt_blake3, &cancel())
            .is_err()
    );
    assert_eq!(
        std::fs::read(path).unwrap(),
        b"changed by an external owner"
    );
}
#[test]
fn retirement_has_explicit_platform_cleanup_outcome_and_blocks_further_reads() {
    let _g = SERIAL.lock().unwrap();
    let dir = temp();
    let state = open(dir.path());
    let before = png([1, 2, 3, 255], false);
    let receipt = state
        .submit(submission(), info(&before), before, &cancel())
        .unwrap();
    let removed = state
        .retire(&receipt.receipt_id, &receipt.receipt_blake3, &cancel())
        .unwrap();
    if cfg!(windows) {
        assert!(removed.files_removed);
        assert!(state.list(&cancel()).unwrap().is_empty());
    } else {
        assert!(!removed.files_removed);
        assert!(state.list(&cancel()).unwrap()[0].retired);
    }
    assert!(
        state
            .image(
                &receipt.receipt_id,
                &receipt.receipt_blake3,
                McpInboxSide::Before,
                &cancel()
            )
            .is_err()
    );
}
#[test]
fn retained_orphan_bytes_count_against_quota_without_allocating_them() {
    let _g = SERIAL.lock().unwrap();
    let dir = temp();
    let state = open(dir.path());
    let root = dir.path().join("mcp-inbox-v1");
    std::fs::create_dir(root.join("incoming-synthetic")).unwrap();
    let f = std::fs::File::create(root.join("incoming-synthetic/partial")).unwrap();
    #[cfg(windows)]
    sparse_fixture::mark(&f).unwrap();
    f.set_len(DISK_LIMIT + 1).unwrap();
    assert_eq!(f.metadata().unwrap().len(), DISK_LIMIT + 1);
    drop(f);
    let before = png([1, 2, 3, 255], false);
    assert!(matches!(
        state.submit(submission(), info(&before), before, &cancel()),
        Err(PackageError::Limit)
    ));
    assert!(root.join("incoming-synthetic/partial").exists());
    assert!(!root.join(id(4)).exists());
}

#[test]
fn marker_only_retirement_keeps_its_logical_slot_and_reopens_at_capacity() {
    let _g = SERIAL.lock().unwrap();
    let dir = temp();
    let state = open(dir.path());
    let before = png([1, 2, 3, 255], false);
    let first = state
        .submit(submission(), info(&before), before.clone(), &cancel())
        .unwrap();
    let root = dir.path().join("mcp-inbox-v1");
    let live = root.join(&first.receipt_id);
    let marker = root.join(format!("retiring-{}", first.receipt_id));
    std::fs::create_dir(&marker).unwrap();
    std::fs::copy(live.join("manifest.json"), marker.join("manifest.json")).unwrap();
    // A live directory plus its complete retirement marker occupies one slot,
    // so all63 other receipts can still be admitted through production submit.
    for n in 20..83 {
        let mut s = submission();
        s.receipt_id = id(n);
        state
            .submit(s, info(&before), before.clone(), &cancel())
            .unwrap();
    }
    assert_eq!(state.list(&cancel()).unwrap().len(), MAX_ENTRIES);
    // Reproduce successful item cleanup followed by failed marker cleanup.
    std::fs::remove_file(live.join("images/before.png")).unwrap();
    std::fs::remove_dir(live.join("images")).unwrap();
    std::fs::remove_file(live.join("manifest.json")).unwrap();
    std::fs::remove_dir(&live).unwrap();
    let mut extra = submission();
    extra.receipt_id = id(99);
    assert!(matches!(
        state.submit(extra.clone(), info(&before), before.clone(), &cancel()),
        Err(PackageError::Limit)
    ));
    assert!(!root.join(id(99)).exists());
    assert!(
        state
            .list(&cancel())
            .unwrap()
            .iter()
            .any(|v| v.receipt_id == first.receipt_id && v.retired)
    );
    // An interrupted retirement write on a full inbox consumes admission
    // headroom but must not prevent reading/reopening the64 valid receipts.
    std::fs::create_dir(root.join("retiring-write-interrupted")).unwrap();
    drop(state);
    let state = open(dir.path());
    assert_eq!(state.list(&cancel()).unwrap().len(), MAX_ENTRIES);
    assert!(matches!(
        state.submit(extra, info(&before), before, &cancel()),
        Err(PackageError::Limit)
    ));
}
#[test]
fn both_retirement_and_incoming_scratch_consume_publication_slots() {
    let _g = SERIAL.lock().unwrap();
    let dir = temp();
    let state = open(dir.path());
    let root = dir.path().join("mcp-inbox-v1");
    for n in 0..MAX_ENTRIES - 1 {
        let prefix = if n % 2 == 0 {
            "retiring-write-"
        } else {
            "incoming-"
        };
        std::fs::create_dir(root.join(format!("{prefix}synthetic-{n}"))).unwrap();
    }
    let before = png([1, 2, 3, 255], false);
    state
        .submit(submission(), info(&before), before.clone(), &cancel())
        .unwrap();
    let mut extra = submission();
    extra.receipt_id = id(99);
    assert!(matches!(
        state.submit(extra, info(&before), before, &cancel()),
        Err(PackageError::Limit)
    ));
    assert!(!root.join(id(99)).exists());
    drop(state);
    let state = open(dir.path());
    assert_eq!(state.list(&cancel()).unwrap().len(), 1);
}
