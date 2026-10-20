//! Dedicated one-process UIA/WGC/finite-input Paint drawing experiment.
//! Only the parent runner can grant each fixed action over its ready pipe.
use super::*;
use crate::platform::{finite_input::FiniteInputOwner, paint_controls::experiment};
use crate::{
    editor_effect::terminal::Producer,
    editor_source::{SourceBudget, SourceLease, SourcePolicy},
    platform,
};
use std::{io::Read, sync::Arc, thread, time::Duration};
use windows::Win32::{
    Foundation::POINT,
    System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
    UI::WindowsAndMessaging::GetCursorPos,
};

fn pointer() -> Result<[i64; 2]> {
    let mut value = POINT::default();
    // SAFETY: query-only current OS pointer; no caller coordinate participates.
    unsafe { GetCursorPos(&mut value) }.map_err(|_| Error::TargetChanged)?;
    Ok([i64::from(value.x), i64::from(value.y)])
}
pub(super) fn outside(point: [i64; 2], rect: vw_remote::Rect) -> bool {
    point[0] < i64::from(rect.x)
        || point[1] < i64::from(rect.y)
        || point[0] >= i64::from(rect.x) + i64::from(rect.width)
        || point[1] >= i64::from(rect.y) + i64::from(rect.height)
}
fn witness(
    source: &SourceLease,
    r: &Request,
    cancel: &Arc<vw_capture::Cancellation>,
) -> Result<(experiment::State, PaintFrameWitness)> {
    let b = SourceBudget::new(cancel.clone(), Duration::from_secs(5))?;
    source.verify(&r.target, r.owner_pid, &b)?;
    let snapshot = platform::paint_controls::observe(&r.target, &b)?;
    let state = experiment::State::from_snapshot(&snapshot)?;
    if state.baseline_tool()? != Action::Brush {
        return Err(Error::Unavailable);
    }
    let frame = snapshot.frame_witness()?;
    if frame.canvas_tool_name != "Using Brush tool on Canvas"
        || frame.settings_digest != r.settings_digest
        || !frame.canvas_rect_host.inside(r.target.client_rect)
        || u64::from(frame.canvas_rect_host.width) * u64::from(frame.canvas_rect_host.height)
            > evidence::MAX_PIXELS
    {
        return Err(Error::TargetChanged);
    }
    source.verify(&r.target, r.owner_pid, &b)?;
    Ok((state, frame))
}
fn same_frame(before: &PaintFrameWitness, after: &PaintFrameWitness) -> Result<()> {
    if before.settings_digest != after.settings_digest
        || before.canvas_runtime_id_hash != after.canvas_runtime_id_hash
        || before.canvas_rect_host != after.canvas_rect_host
        || before.canvas_tool_name != after.canvas_tool_name
        || after.observed_start_qpc_100ns < before.observed_end_qpc_100ns
    {
        Err(Error::TargetChanged)
    } else {
        Ok(())
    }
}
fn capture(
    source: &SourceLease,
    r: &Request,
    cancel: &Arc<vw_capture::Cancellation>,
    baseline: &PaintFrameWitness,
    parked: [i64; 2],
    index: usize,
) -> Result<()> {
    let before_pointer = pointer()?;
    if before_pointer != parked || !outside(parked, baseline.canvas_rect_host) {
        return Err(Error::TargetChanged);
    }
    let (_, before) = witness(source, r, cancel)?;
    same_frame(baseline, &before)?;
    let target = vw_capture::windows::fixed_target(r.target.window, r.owner_pid)
        .map_err(|_| Error::TargetChanged)?;
    if target.client
        != (vw_capture::Rect {
            x: r.target.client_rect.x,
            y: r.target.client_rect.y,
            width: r.target.client_rect.width,
            height: r.target.client_rect.height,
        })
        || target.frame
            != (vw_capture::Rect {
                x: r.target.frame_rect.x,
                y: r.target.frame_rect.y,
                width: r.target.frame_rect.width,
                height: r.target.frame_rect.height,
            })
    {
        return Err(Error::TargetChanged);
    }
    let directory = r.sample_directory(index)?;
    vw_capture::plain(&directory, true).map_err(|_| Error::Invalid)?;
    let limits = vw_capture::Limits {
        memory_bytes: 128 * 1024 * 1024,
        png_bytes: 32 * 1024 * 1024,
        max_pixels: 4 * 1024 * 1024,
        capture_ms: 3000,
        ..vw_capture::Limits::default()
    };
    limits
        .image(target.frame.width, target.frame.height)
        .map_err(|_| Error::Limit)?;
    limits
        .image(
            baseline.canvas_rect_host.width,
            baseline.canvas_rect_host.height,
        )
        .map_err(|_| Error::Limit)?;
    let request = vw_capture::CanvasCaptureRequest {
        capture: vw_capture::CaptureRequest {
            explicit_owner_action: true,
            owner_process_id: r.owner_pid,
            target,
            capture_session_id: r.capture_session_id.clone(),
            output_directory: directory.to_str().ok_or(Error::Invalid)?.to_owned(),
            limits,
            diagnostic_color_stage: true,
            diagnostic_alpha_region: None,
        },
        explicit_owned_blank_fixture: true,
        canvas_rect_host: vw_capture::Rect {
            x: baseline.canvas_rect_host.x,
            y: baseline.canvas_rect_host.y,
            width: baseline.canvas_rect_host.width,
            height: baseline.canvas_rect_host.height,
        },
    };
    let mut color = None;
    // This WGC call runs in this same helper process, never a nested process.
    // The capture API requires and checks SetIsCursorCaptureEnabled(false).
    let frame = vw_capture::windows::capture_canvas_fixture(request, cancel, &mut color)
        .map_err(|_| Error::Unavailable)?;
    if color.is_some() {
        return Err(Error::Unavailable);
    }
    let after_pointer = pointer()?;
    let (_, after) = witness(source, r, cancel)?;
    same_frame(&before, &after)?;
    if after_pointer != parked
        || frame.source_identity.monotonic_timestamp_ns
            < before
                .observed_end_qpc_100ns
                .checked_mul(100)
                .ok_or(Error::Limit)?
        || frame.width != baseline.canvas_rect_host.width
        || frame.height != baseline.canvas_rect_host.height
    {
        return Err(Error::TargetChanged);
    }
    ipc::send(
        std::io::stdout().lock(),
        &ipc::Line::Sample {
            value: ipc::Sample {
                index,
                target: r.target.clone(),
                image: r.image.clone(),
                context_sha256: r.context_sha256.clone(),
                before_witness: before,
                witness: after,
                pointer_before: before_pointer,
                pointer_after: after_pointer,
                os_cursor_capture_disabled: true,
                frame,
            },
        },
    )
}
fn input_step(
    input: &mut impl Read,
    source: &SourceLease,
    r: &Request,
    cancel: &Arc<vw_capture::Cancellation>,
    baseline: &PaintFrameWitness,
    parked: [i64; 2],
    step: ipc::Step,
) -> Result<ipc::Click> {
    let owner_budget = SourceBudget::new(cancel.clone(), Duration::from_secs(5))?;
    let step_source = SourceLease::open(
        &r.target,
        r.owner_pid,
        &r.image,
        SourcePolicy::InstalledPaint,
        &owner_budget,
    )?;
    let mut input_owner = FiniteInputOwner::new(r.target.clone(), r.owner_pid, step_source)?;
    let (state, before) = witness(source, r, cancel)?;
    same_frame(baseline, &before)?;
    if pointer()? != parked {
        return Err(Error::TargetChanged);
    }
    let budget = SourceBudget::new(cancel.clone(), Duration::from_secs(5))?;
    source.verify(&r.target, r.owner_pid, &budget)?;
    let route = experiment::address(&r.target, &budget, step.action())?;
    if step == ipc::Step::CanvasDot {
        let rect = baseline.canvas_rect_host;
        let x = i64::from(route.destination.0) - i64::from(rect.x);
        let y = i64::from(route.destination.1) - i64::from(rect.y);
        if x < 64 || y < 64 || x + 64 >= i64::from(rect.width) || y + 64 >= i64::from(rect.height) {
            return Err(Error::Unavailable);
        }
    }
    let parked_point = (
        i32::try_from(parked[0]).map_err(|_| Error::Invalid)?,
        i32::try_from(parked[1]).map_err(|_| Error::Invalid)?,
    );
    ipc::send(
        std::io::stdout().lock(),
        &ipc::Line::Ready {
            step,
            target: r.target.clone(),
            image: r.image.clone(),
            context_sha256: r.context_sha256.clone(),
            witness: before.clone(),
        },
    )?;
    let mut decision = [0u8; 1];
    input.read_exact(&mut decision)?;
    let mut terminal = Producer::default();
    if decision[0] == ipc::DENY {
        terminal.finish(&mut input_owner)?;
        let (_, after) = witness(source, r, cancel)?;
        let report = ipc::Click {
            step,
            action: step.action(),
            target: r.target.clone(),
            image: r.image.clone(),
            context_sha256: r.context_sha256.clone(),
            before,
            after,
            destination: [0, 0],
            native_accepted_count: 0,
            expected_count: 0,
            accepted_qpc_100ns: None,
            input_retirement_complete: true,
            held_count: 0,
            had_error: false,
            pointer_after: pointer()?,
        };
        ipc::send(
            std::io::stdout().lock(),
            &ipc::Line::Terminal {
                value: report.clone(),
            },
        )?;
        return Err(Error::Ungranted);
    }
    if decision[0] != ipc::GRANT {
        return Err(Error::Ungranted);
    }
    terminal.arm_attempt()?;
    let attempted = experiment::inject_parked(
        &mut input_owner,
        &r.target,
        r.owner_pid,
        &budget,
        experiment::Step {
            action: step.action(),
            expected: &state,
            expected_address: &route,
            arrow: None,
        },
        parked_point,
    );
    // This writes the existing source-held finite-retirement record, waiting
    // for actual zero held input even if the parked batch refused or failed.
    let retired = terminal.finish(&mut input_owner);
    let receipt = attempted?;
    retired?;
    let after_pointer = pointer()?;
    let (_, after) = witness(source, r, cancel)?;
    same_frame(&before, &after)?;
    let report = ipc::Click {
        step,
        action: receipt.action,
        target: r.target.clone(),
        image: r.image.clone(),
        context_sha256: r.context_sha256.clone(),
        before,
        after,
        destination: [route.destination.0, route.destination.1],
        native_accepted_count: receipt.native_accepted_count,
        expected_count: receipt.expected_count,
        accepted_qpc_100ns: receipt.accepted_qpc_100ns,
        input_retirement_complete: receipt.input_retirement == "complete",
        held_count: receipt.held_count,
        had_error: receipt.error.is_some(),
        pointer_after: after_pointer,
    };
    ipc::send(
        std::io::stdout().lock(),
        &ipc::Line::Terminal {
            value: report.clone(),
        },
    )?;
    receipt.completed()?;
    if after_pointer != parked {
        return Err(Error::TargetChanged);
    }
    Ok(report)
}

pub(super) fn run() -> Result<()> {
    let mut parent = windows::core::BOOL::default();
    // SAFETY: query-only actual current process Job receipt.
    unsafe {
        platform::api(
            "drawing parent Job",
            windows::Win32::System::JobObjects::IsProcessInJob(
                windows::Win32::System::Threading::GetCurrentProcess(),
                None,
                &mut parent,
            ),
        )?;
    }
    if !parent.as_bool() {
        return Err(Error::Ungranted);
    }
    let _job = platform::confine()?;
    let _apartment = platform::EffectApartment::initialize()?;
    platform::initialize_hil_dpi()?;
    // SAFETY: this helper's initialized MTA owns all synchronous WGC calls.
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED).map_err(|_| Error::Unavailable)?;
    }
    let result = run_inner();
    // SAFETY: paired successful RoInitialize on this thread.
    unsafe {
        RoUninitialize();
    }
    result
}
fn run_inner() -> Result<()> {
    let mut input = std::io::stdin().lock();
    let raw = ipc::line(&mut input)?;
    let r: Request = serde_json::from_slice(&raw).map_err(|_| Error::Invalid)?;
    r.validate()?;
    let cancel = Arc::new(vw_capture::Cancellation::default());
    let budget = SourceBudget::new(cancel.clone(), Duration::from_secs(5))?;
    let source = SourceLease::open(
        &r.target,
        r.owner_pid,
        &r.image,
        SourcePolicy::InstalledPaint,
        &budget,
    )?;
    let (_, baseline) = witness(&source, &r, &cancel)?;
    let fixed = vw_capture::windows::fixed_target(r.target.window, r.owner_pid)
        .map_err(|_| Error::TargetChanged)?;
    frame::drawing_capacity(
        u64::from(baseline.canvas_rect_host.width) * u64::from(baseline.canvas_rect_host.height),
        u64::from(fixed.frame.width) * u64::from(fixed.frame.height),
    )?;
    let parked = pointer()?;
    if !outside(parked, baseline.canvas_rect_host) {
        return Err(Error::Unavailable);
    }
    for index in 0..2 {
        if index > 0 {
            thread::sleep(Duration::from_millis(110));
        }
        capture(&source, &r, &cancel, &baseline, parked, index)?;
    }
    for step in ipc::Step::ORDER {
        input_step(&mut input, &source, &r, &cancel, &baseline, parked, step)?;
        for offset in 0..2 {
            thread::sleep(Duration::from_millis(110));
            capture(
                &source,
                &r,
                &cancel,
                &baseline,
                parked,
                step.samples_before() + offset,
            )?;
        }
    }
    ipc::send(
        std::io::stdout().lock(),
        &ipc::Line::Finished {
            schema: 1,
            prior_history_restoration_unverified: true,
        },
    )?;
    Ok(())
}
