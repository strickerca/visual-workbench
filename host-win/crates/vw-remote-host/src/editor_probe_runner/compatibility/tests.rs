use super::*;
use std::{
    io::Cursor,
    sync::atomic::{AtomicUsize, Ordering as AtomicOrdering},
};

fn id(suffix: u32) -> String {
    format!("0199a2b3-c4d5-7000-8000-{suffix:012x}")
}

#[test]
fn fixed_phase_order_has_no_arbitrary_input_or_sample_expansion() {
    assert_eq!(
        ipc::Step::ORDER.map(ipc::Step::authorize),
        [
            PaintDrawingInputStep::CanvasDot,
            PaintDrawingInputStep::Undo,
            PaintDrawingInputStep::Redo,
            PaintDrawingInputStep::Undo
        ]
    );
    assert_eq!(
        ipc::Step::ORDER.map(ipc::Step::samples_before),
        [2, 4, 6, 8]
    );
}
#[test]
fn preauthorization_pixels_refuse_nonblank_or_noncausal_phases() {
    let blank = vec![255u8; 130 * 130 * 4];
    assert_eq!(ipc::blank_white(&blank, &blank), Ok(()));
    let mut nonwhite = blank.clone();
    nonwhite[0] = 200;
    assert_eq!(
        ipc::blank_white(&nonwhite, &nonwhite),
        Err(Error::Unavailable)
    );
    assert_eq!(ipc::blank_white(&blank, &nonwhite), Err(Error::Unavailable));
    let mut dot = blank.clone();
    let center = (65 * 130 + 65) * 4;
    dot[center] = 0;
    assert_eq!(ipc::admitted_dot_delta(&blank, &dot, 130, [65, 65]), Ok(()));
    assert_eq!(
        ipc::admitted_dot_delta(&blank, &blank, 130, [65, 65]),
        Err(Error::Unavailable)
    );
    dot[0] = 0;
    assert_eq!(
        ipc::admitted_dot_delta(&blank, &dot, 130, [65, 65]),
        Err(Error::Unavailable)
    );
}
#[test]
fn bounded_lines_refuse_truncation_and_extra_capacity() {
    assert_eq!(ipc::line(Cursor::new(b"sample")), Err(Error::Io));
    assert_eq!(
        ipc::line(Cursor::new(vec![b'x'; ipc::MAX_LINE + 1])),
        Err(Error::Limit)
    );
    assert_eq!(ipc::line(Cursor::new(b"ready\n")), Ok(b"ready".to_vec()));
}
#[test]
fn actual_source_held_terminal_requires_complete_and_exact_phase() {
    let complete =
        b"{\"schema\":1,\"operation\":\"finite-retirement\",\"retirement\":\"Complete\"}\n";
    assert_eq!(
        ipc::read_source_held_terminal(&mut Cursor::new(complete)),
        Ok(())
    );
    let pending = b"{\"schema\":1,\"operation\":\"finite-retirement\",\"retirement\":{\"Pending\":{\"held_count\":1,\"uncertain\":false,\"error\":\"RetirementPending\"}}}\n";
    let mut joined = pending.to_vec();
    joined.extend_from_slice(complete);
    assert_eq!(
        ipc::read_source_held_terminal(&mut Cursor::new(joined)),
        Ok(())
    );
    assert_eq!(
        ipc::read_source_held_terminal(&mut Cursor::new(pending)),
        Err(Error::Io)
    );
    let wrong = b"{\"schema\":1,\"operation\":\"finite-retirement\",\"retirement\":\"Pending\"}\n";
    assert_eq!(
        ipc::read_source_held_terminal(&mut Cursor::new(wrong)),
        Err(Error::RetirementPending)
    );
}
#[test]
fn joined_owner_refuses_collection_while_input_is_uncertain() {
    let uncertainty = AtomicBool::new(true);
    let mut queried = false;
    let blocked: Collection<()> = collect_released(
        &uncertainty,
        || {
            queried = true;
            Ok((41, 0))
        },
        true,
        || Ok(Some(())),
    );
    assert!(matches!(
        blocked,
        Collection::Pending(Error::RetirementPending)
    ));
    assert!(!queried);
    uncertainty.store(false, Ordering::Release);
    let joined: Collection<()> =
        collect_released(&uncertainty, || Ok((41, 0)), true, || Ok(Some(())));
    assert!(matches!(
        joined,
        Collection::Joined {
            pid: 41,
            code: 0,
            ..
        }
    ));
}
#[test]
fn authorization_and_cancel_kill_are_serialized() {
    use std::sync::{Barrier, mpsc};
    let gate = Arc::new(ipc::AuthorizationGate::new());
    let uncertain = Arc::new(AtomicBool::new(false));
    let unresolved = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let worker = {
        let gate = gate.clone();
        let uncertain = uncertain.clone();
        let entered = entered.clone();
        let release = release.clone();
        std::thread::spawn(move || {
            gate.authorize(&uncertain, || {
                entered.wait();
                release.wait();
                Ok(())
            })
        })
    };
    entered.wait();
    let (tx, rx) = mpsc::channel();
    let cancel = {
        let gate = gate.clone();
        let uncertain = uncertain.clone();
        let unresolved = unresolved.clone();
        std::thread::spawn(move || tx.send(gate.close_if_safe(&uncertain, &unresolved, || {})))
    };
    release.wait();
    assert_eq!(worker.join().unwrap(), Ok(()));
    assert_eq!(rx.recv().unwrap(), false);
    cancel.join().unwrap().unwrap();
    assert!(uncertain.load(Ordering::Acquire));
    uncertain.store(false, Ordering::Release);
    assert!(gate.close_if_safe(&uncertain, &unresolved, || {}));
    assert_eq!(gate.authorize(&uncertain, || Ok(())), Err(Error::Cancelled));
}
#[test]
fn private_disposal_retains_extra_file_failure_then_retries() -> Result<()> {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    let ordinal = NEXT.fetch_add(1, AtomicOrdering::Relaxed) as u32;
    let marker = id(std::process::id().wrapping_add(ordinal));
    let sample = id(std::process::id().wrapping_add(ordinal).wrapping_add(1));
    let mut lease = PrivateTempLease::create(&marker, &sample, std::process::id())?;
    let unexpected = lease
        .root
        .join(format!("{sample}-00"))
        .join("unexpected.txt");
    fs::write(&unexpected, b"private synthetic fixture")?;
    assert_eq!(lease.dispose(), Err(Error::Io));
    assert!(lease.root.exists());
    fs::remove_file(&unexpected)?;
    assert_eq!(lease.dispose()?, 0);
    assert!(!lease.root.exists());
    Ok(())
}
#[test]
fn private_capture_cleanup_does_not_require_canvas_recovery_or_live_source() -> Result<()> {
    static NEXT: AtomicUsize = AtomicUsize::new(1000);
    let ordinal = NEXT.fetch_add(1, AtomicOrdering::Relaxed) as u32;
    let marker = id(std::process::id().wrapping_add(ordinal));
    let sample = id(std::process::id().wrapping_add(ordinal).wrapping_add(1));
    let mut lease = PrivateTempLease::create(&marker, &sample, std::process::id())?;
    fs::write(
        lease
            .root
            .join(format!("{sample}-00"))
            .join("capture-canvas.png"),
        b"private",
    )?;
    let canvas_unresolved = AtomicBool::new(true);
    assert!(canvas_unresolved.load(Ordering::Acquire));
    assert_eq!(lease.dispose()?, 1);
    assert!(!lease.root.exists());
    Ok(())
}
fn sample_request() -> Request {
    Request {
        schema: 1,
        owner_pid: std::process::id(),
        target: Target {
            token: id(101),
            window: 1,
            process_id: 2,
            thread_id: 3,
            process_created: 4,
            window_rect: vw_remote::Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            frame_rect: vw_remote::Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            client_rect: vw_remote::Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            dpi: 96,
            integrity: 0,
        },
        image: ImageIdentity {
            executable_name: "mspaint.exe".into(),
            executable_blake3: "a".repeat(64),
            executable_bytes: 1024,
            file_version: None,
            package_full_name: None,
            package_version: None,
        },
        context_sha256: "c".repeat(64),
        settings_digest: "b".repeat(64),
        capture_session_id: id(102),
        private_root: PathBuf::new(),
        sample_id: id(103),
    }
}
#[test]
fn request_refuses_uncanonical_context_before_helper_launch() {
    let mut request = sample_request();
    request.context_sha256 = "not-a-digest".into();
    assert_eq!(request.validate(), Err(Error::Invalid));
}
#[test]
fn joined_click_rejects_changed_source_and_partial_input() {
    let request = sample_request();
    let canvas = PaintFrameWitness {
        settings_digest: request.settings_digest.clone(),
        canvas_runtime_id_hash: "d".repeat(64),
        canvas_rect_host: request.target.client_rect,
        canvas_tool_name: "Using Brush tool on Canvas".into(),
        observed_start_qpc_100ns: 100,
        observed_end_qpc_100ns: 110,
    };
    let report = ipc::Click {
        step: ipc::Step::CanvasDot,
        action: Action::CanvasDot,
        target: request.target.clone(),
        image: request.image.clone(),
        context_sha256: request.context_sha256.clone(),
        before: canvas.clone(),
        after: PaintFrameWitness {
            observed_start_qpc_100ns: 120,
            observed_end_qpc_100ns: 130,
            ..canvas.clone()
        },
        destination: [50, 50],
        native_accepted_count: 4,
        expected_count: 4,
        accepted_qpc_100ns: Some(115),
        input_retirement_complete: true,
        held_count: 0,
        had_error: false,
        pointer_after: [-1, -1],
    };
    let mut observed = ipc::ObservedClick {
        report,
        receipt_sha256: "e".repeat(64),
    };
    observed.report.image.executable_blake3 = "f".repeat(64);
    assert!(matches!(
        joined_click(&observed, 55, 0, &request, &canvas),
        Err(Error::TargetChanged)
    ));
    observed.report.image = request.image.clone();
    observed.report.native_accepted_count = 2;
    let joined = joined_click(&observed, 55, 0, &request, &canvas).unwrap();
    assert_eq!(joined.native_accepted_count, 2);
    assert_ne!(joined.native_accepted_count, joined.expected_count);
}
