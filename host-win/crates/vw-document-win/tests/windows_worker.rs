#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use vw_document_protocol::{JobLimits, Operation, Request, Tile};
use vw_document_win::{
    ArtifactEntry, Cancellation, PinnedRuntime, RuntimeManifest, WorkerLimits, cleanup_pending, run,
};

// The production host deliberately admits one active owner per process. All
// fixtures that enter run share that admission rather than racing for Busy.
static RUN_FIXTURE: Mutex<()> = Mutex::new(());
fn serial() -> MutexGuard<'static, ()> {
    RUN_FIXTURE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

fn physical(path: &Path) -> PathBuf {
    let value = fs::canonicalize(path)
        .unwrap()
        .into_os_string()
        .into_string()
        .unwrap();
    PathBuf::from(value.strip_prefix("\\\\?\\").unwrap_or(&value))
}
fn entry(name: &str, bytes: &[u8]) -> ArtifactEntry {
    ArtifactEntry {
        file_name: name.to_owned(),
        bytes: bytes.len() as u64,
        sha256: Sha256::digest(bytes).into(),
    }
}

#[test]
fn real_runtime_pins_reject_write_rename_extra_files_and_wrong_hashes() {
    let temp = tempfile::tempdir().unwrap();
    let root = physical(temp.path());
    fs::write(root.join("vw-document-worker.exe"), b"synthetic-executable").unwrap();
    fs::write(root.join("pdfium.dll"), b"synthetic-library").unwrap();
    let manifest = RuntimeManifest {
        entries: vec![
            entry("vw-document-worker.exe", b"synthetic-executable"),
            entry("pdfium.dll", b"synthetic-library"),
        ],
    };
    let pinned = PinnedRuntime::pin(&root, &manifest).unwrap();
    assert!(fs::write(root.join("pdfium.dll"), b"changed").is_err());
    assert!(fs::rename(root.join("pdfium.dll"), root.join("moved.dll")).is_err());
    let moved = root.with_extension("moved");
    let renamed = fs::rename(&root, &moved).is_ok();
    if renamed {
        fs::rename(&moved, &root).unwrap();
    }
    assert!(!renamed);
    drop(pinned);
    fs::write(root.join("extra.dll"), b"extra").unwrap();
    assert!(PinnedRuntime::pin(&root, &manifest).is_err());
    fs::remove_file(root.join("extra.dll")).unwrap();
    fs::write(root.join("pdfium.dll"), b"tampered--library").unwrap();
    assert!(PinnedRuntime::pin(&root, &manifest).is_err());
}

#[test]
fn existing_crash_marker_refuses_before_profile_or_dummy_executable_startup() {
    let _serial = serial();
    let temp = tempfile::tempdir().unwrap();
    let root = physical(temp.path());
    let runtime = root.join("runtime");
    let scratch = root.join("document-worker-private");
    fs::create_dir(&runtime).unwrap();
    fs::create_dir(&scratch).unwrap();
    fs::write(runtime.join("vw-document-worker.exe"), b"never-executable").unwrap();
    fs::write(runtime.join("pdfium.dll"), b"never-library").unwrap();
    let manifest = RuntimeManifest {
        entries: vec![
            entry("vw-document-worker.exe", b"never-executable"),
            entry("pdfium.dll", b"never-library"),
        ],
    };
    let pinned = Arc::new(PinnedRuntime::pin(&runtime, &manifest).unwrap());
    let marker = scratch.join("vw-document-active-v1.json");
    fs::write(&marker, b"preserve prior owner").unwrap();
    let request = request();
    let result = run(
        pinned.clone(),
        &scratch,
        &request,
        b"synthetic",
        WorkerLimits {
            wall_time: Duration::from_secs(2),
            process_memory_bytes: request.limits.memory_bytes,
        },
        &Cancellation::default(),
        &mut |_| Ok(()),
        &mut |_| {},
    );
    assert!(matches!(
        result,
        Err(vw_document_win::Error::CleanupPending)
    ));
    assert_eq!(fs::read(&marker).unwrap(), b"preserve prior owner");
    assert!(pinned.workspace_cleanup_pending().unwrap());
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 1);
    drop(pinned);
}

fn fixture() -> (tempfile::TempDir, Arc<PinnedRuntime>, PathBuf, PathBuf) {
    let probe = std::env::var_os("VW_DOCUMENT_SANDBOX_PROBE")
        .expect("central source-bound probe build path required");
    let expected = std::env::var("VW_DOCUMENT_SANDBOX_PROBE_SHA256")
        .expect("central exact probe hash required");
    assert_eq!(expected.len(), 64);
    let file = fs::File::open(probe).unwrap();
    assert!(file.metadata().unwrap().len() < 256 * 1024 * 1024);
    let mut bytes = Vec::new();
    file.take(256 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .unwrap();
    assert!(bytes.len() < 256 * 1024 * 1024);
    let hash: [u8; 32] = Sha256::digest(&bytes).into();
    let actual: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(actual, expected);
    let temp = tempfile::tempdir().unwrap();
    let root = physical(temp.path());
    let runtime = root.join("runtime");
    let scratch = root.join("document-worker-private");
    fs::create_dir(&runtime).unwrap();
    fs::create_dir(&scratch).unwrap();
    fs::write(runtime.join("vw-document-worker.exe"), &bytes).unwrap();
    // This fixture tests the OS boundary, not PDFium loading. The actual native
    // library contract uses the separately verified real PDFium worker case.
    fs::write(runtime.join("pdfium.dll"), b"fixture-not-a-dll").unwrap();
    let manifest = RuntimeManifest {
        entries: vec![
            entry("vw-document-worker.exe", &bytes),
            entry("pdfium.dll", b"fixture-not-a-dll"),
        ],
    };
    let pinned = Arc::new(PinnedRuntime::pin(&runtime, &manifest).unwrap());
    (temp, pinned, scratch, root)
}
fn request() -> Request {
    Request {
        limits: JobLimits {
            input_bytes: 1024 * 1024,
            output_bytes: 1024 * 1024,
            memory_bytes: 128 * 1024 * 1024,
        },
        password: None,
        operation: Operation::SvgTile {
            tile: Tile {
                x: 0.,
                y: 0.,
                scale: 1.,
                width: 8,
                height: 1,
            },
        },
    }
}

#[test]
#[ignore = "explicit central Windows AppContainer/ACL/network/process acceptance window and hash-bound fixture required"]
fn actual_lpac_refuses_owned_outside_files_loopback_and_child_creation() {
    let _serial = serial();
    let (_temp, pinned, scratch, root) = fixture();
    let sentinel = root.join("outside.txt");
    fs::write(&sentinel, b"synthetic host-only sentinel").unwrap();
    let forbidden_write = root.join("unexpected-write.txt");
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let source = serde_json::to_vec(&serde_json::json!({"mode":"isolation", "read_path":sentinel, "write_path":forbidden_write, "loopback_port":listener.local_addr().unwrap().port()})).unwrap();
    let request = request();
    let mut reply = run(
        pinned.clone(),
        &scratch,
        &request,
        &source,
        WorkerLimits {
            wall_time: Duration::from_secs(10),
            process_memory_bytes: request.limits.memory_bytes,
        },
        &Cancellation::default(),
        &mut |_| Ok(()),
        &mut |_| {},
    )
    .unwrap();
    let mut flags = Vec::new();
    reply.private_file().read_to_end(&mut flags).unwrap();
    assert_eq!(flags.len(), 32);
    for index in 0..4 {
        assert_eq!(
            flags[index * 4],
            0,
            "outside access or child unexpectedly permitted"
        );
    }
    assert_eq!(flags[16], 255);
    assert!(!forbidden_write.exists());
    assert!(!cleanup_pending());
    drop(reply);
    drop(pinned);
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
}

#[test]
#[ignore = "explicit central Windows AppContainer/ACL/process acceptance window and hash-bound fixture required"]
fn hanging_and_noisy_workers_are_refused_and_owned_resources_join_before_return() {
    let _serial = serial();
    let (_temp, pinned, scratch, _root) = fixture();
    let request = request();
    for mode in ["timeout", "stderr_flood"] {
        let source = serde_json::to_vec(&serde_json::json!({"mode":mode,"read_path":"unused","write_path":"unused","loopback_port":1})).unwrap();
        let result = run(
            pinned.clone(),
            &scratch,
            &request,
            &source,
            WorkerLimits {
                wall_time: Duration::from_secs(2),
                process_memory_bytes: request.limits.memory_bytes,
            },
            &Cancellation::default(),
            &mut |_| Ok(()),
            &mut |_| {},
        );
        assert!(result.is_err());
        assert!(!cleanup_pending());
        assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    }
    drop(pinned);
}

#[test]
#[ignore = "explicit central Windows AppContainer/ACL/network/process acceptance window and hash-bound fixture required"]
fn cancellation_during_metadata_delivery_cannot_publish_and_settles_the_worker() {
    let _serial = serial();
    let (_temp, pinned, scratch, root) = fixture();
    let request = request();
    let sentinel = root.join("outside.txt");
    fs::write(&sentinel, b"synthetic").unwrap();
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let source = serde_json::to_vec(&serde_json::json!({"mode":"isolation","read_path":sentinel,"write_path":root.join("forbidden.txt"),"loopback_port":listener.local_addr().unwrap().port()})).unwrap();
    let cancellation = Cancellation::default();
    let result = run(
        pinned.clone(),
        &scratch,
        &request,
        &source,
        WorkerLimits {
            wall_time: Duration::from_secs(10),
            process_memory_bytes: request.limits.memory_bytes,
        },
        &cancellation,
        &mut |_| {
            cancellation.cancel();
            Ok(())
        },
        &mut |_| {},
    );
    assert!(matches!(result, Err(vw_document_win::Error::Cancelled)));
    assert!(!cleanup_pending());
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    drop(pinned);
}
