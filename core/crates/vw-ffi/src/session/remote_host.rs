//! Windows-only orchestration. Codec/UIA/injection execute in owned helpers;
//! cancellation retains threads and helper leases until actual retirement.
use super::*;
use std::{
    path::Path,
    sync::{OnceLock, mpsc},
    thread,
};
use vw_remote::{
    applied::Receipt,
    wire::{Command, FiniteAction, Header, PenSample, Phase},
};
use vw_remote_host::{
    platform,
    process::{self, Cancellation, Kind, LockedHelper, Process},
};
#[derive(Clone)]
pub(super) struct Runtime {
    video: Arc<LockedHelper>,
    input: Arc<LockedHelper>,
    profile: Option<vw_remote::profile::PackagedProfile>,
}
pub(super) struct Worker {
    cancel: Arc<Cancellation>,
    thread: Option<thread::JoinHandle<SessionResult<()>>>,
}
impl Worker {
    fn cancel(&self) {
        self.cancel.cancel()
    }
    fn close(&mut self) -> SessionResult<()> {
        self.cancel();
        let deadline = Instant::now() + Duration::from_millis(500);
        while self.thread.as_ref().is_some_and(|t| !t.is_finished()) {
            if Instant::now() >= deadline {
                return Err(SessionError::RemoteRetirementPending);
            }
            thread::sleep(Duration::from_millis(5));
        }
        if let Some(handle) = self.thread.take() {
            let _result = handle.join().map_err(|_| SessionError::Worker)?;
        }
        Ok(())
    }
}
fn retained() -> &'static Mutex<Vec<Worker>> {
    static RETAINED: OnceLock<Mutex<Vec<Worker>>> = OnceLock::new();
    RETAINED.get_or_init(|| Mutex::new(Vec::new()))
}
impl Drop for Worker {
    fn drop(&mut self) {
        if self.close().is_err()
            && let Some(thread) = self.thread.take()
        {
            retained()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(Worker {
                    cancel: self.cancel.clone(),
                    thread: Some(thread),
                });
        }
    }
}
pub(super) fn pending_retirement_count() -> SessionResult<usize> {
    let mut workers = retained().lock().map_err(|_| SessionError::Worker)?;
    let mut at = 0;
    while at < workers.len() {
        if workers[at]
            .thread
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished)
        {
            let mut done = workers.remove(at);
            done.close()?;
        } else {
            at += 1
        }
    }
    Ok(workers.len() + process::retry_retirements().map_err(failure)?)
}
pub(super) fn retirement_count() -> SessionResult<usize> {
    let pending = pending_retirement_count()?;
    Ok(pending + process::active_owner_count())
}
struct Starting {
    hub: Arc<RemoteHub>,
    cancel: Arc<Cancellation>,
}
impl Starting {
    fn new(hub: Arc<RemoteHub>, cancel: Arc<Cancellation>) -> SessionResult<Self> {
        {
            let mut registered = hub.cancellations.lock().map_err(|_| SessionError::Worker)?;
            registered.retain(|weak| weak.strong_count() != 0);
            if registered.len() >= 8 {
                return Err(SessionError::Backpressure);
            }
            registered.push(Arc::downgrade(&cancel));
        }
        let mut workers = hub.workers.lock().map_err(|_| SessionError::Worker)?;
        if workers.starting.is_some() {
            return Err(SessionError::Backpressure);
        }
        workers.starting = Some(cancel.clone());
        drop(workers);
        Ok(Self { hub, cancel })
    }
}
impl Drop for Starting {
    fn drop(&mut self) {
        if let Ok(mut workers) = self.hub.workers.lock()
            && workers
                .starting
                .as_ref()
                .is_some_and(|c| Arc::ptr_eq(c, &self.cancel))
        {
            workers.starting.take();
        }
    }
}
struct Pending {
    binding: Binding,
    event: pb::InputEvent,
    arrived: Instant,
}
#[derive(Default)]
pub(super) struct Workers {
    video: Option<Worker>,
    input: Option<Worker>,
    starting: Option<Arc<Cancellation>>,
    input_send: Option<mpsc::SyncSender<Pending>>,
    keyframe: Option<Arc<stream::Keyframes>>,
}
impl Workers {
    #[cfg(test)]
    pub(super) fn keyframe_fixture(&mut self, flag: Arc<stream::Keyframes>) {
        self.keyframe = Some(flag);
    }
    pub(super) fn cancel(&self) {
        if let Some(v) = &self.video {
            v.cancel()
        }
        if let Some(v) = &self.input {
            v.cancel()
        }
        if let Some(v) = &self.starting {
            v.cancel()
        }
    }
    fn cancel_input(&self) {
        if let Some(v) = &self.input {
            v.cancel()
        }
    }
    pub(super) fn close(&mut self) -> SessionResult<()> {
        self.cancel();
        if self.starting.is_some() {
            return Err(SessionError::RemoteRetirementPending);
        }
        self.input_send.take();
        if let Some(v) = &mut self.input {
            v.close()?;
        }
        if let Some(v) = &mut self.video {
            v.close()?;
        }
        self.input.take();
        self.video.take();
        self.keyframe.take();
        Ok(())
    }
    pub(super) fn request_keyframe(&self) {
        if let Some(v) = &self.keyframe {
            v.request()
        }
    }
    pub(super) fn input(&self, binding: Binding, event: pb::InputEvent) -> SessionResult<()> {
        self.input_send
            .as_ref()
            .ok_or(SessionError::Authentication)?
            .try_send(Pending {
                binding,
                event,
                arrived: Instant::now(),
            })
            .map_err(|e| match e {
                mpsc::TrySendError::Full(_) => SessionError::Backpressure,
                mpsc::TrySendError::Disconnected(_) => SessionError::Worker,
            })
    }
}
fn worker_error(hub: &RemoteHub, scope: &Scope, reason: &str) {
    if let Ok(mut s) = hub.state.lock()
        && s.selected.as_ref().is_some_and(|v| &v.scope == scope)
    {
        s.host_retiring = true;
        RemoteHub::retire(&mut s);
        let _ = hub.send_control(&mut s, "revoke", reason);
        let _ = hub.publish(&mut s, "sealed", Some(reason));
    }
    hub.cancel_workers();
}
fn publish_video(
    hub: &RemoteHub,
    selected: &Selected,
    config: &VideoConfig,
    mut frame: Frame,
) -> SessionResult<()> {
    let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
    RemoteHub::current(&s)?;
    if s.selected
        .as_ref()
        .is_none_or(|v| v.scope != selected.scope || v.target != selected.target)
    {
        return Err(SessionError::Closed);
    }
    if let Some(applied) = &s.applied {
        (frame.input_session_id, frame.last_input_seq_applied) =
            applied.for_frame(&frame.scope, frame.captured_qpc_100ns)
    }
    frame.validate(config).map_err(failure)?;
    if s.config.is_none() {
        s.config = Some(config.clone());
        let status = s.display.status.clone();
        hub.publish(&mut s, &status, None)?;
    }
    let sender = s.sender.as_ref().ok_or(SessionError::Closed)?;
    // Config repeats with every IDR, repairing independent CONTROL/MEDIA ordering.
    if frame.keyframe {
        sender.enqueue(Body::RemoteVideoConfig(config_pb(config)?))?;
        hub.stream.increment(3);
    }
    let r = selected.target.client_rect;
    let geometry = pb::CaptureGeometry {
        source_kind: "window".into(),
        window_handle: selected.target.window,
        monitor_id: String::new(),
        client_rect_host: Some(pb::RectI {
            x: r.x,
            y: r.y,
            w: r.width as i32,
            h: r.height as i32,
        }),
        dpi_scale: f64::from(selected.target.dpi) / 96.0,
        geometry_revision: selected.scope.geometry_revision,
        timestamp_ns: i64::try_from(
            frame
                .captured_qpc_100ns
                .checked_mul(100)
                .ok_or(SessionError::Invalid)?,
        )
        .map_err(|_| SessionError::Invalid)?,
    };
    sender.enqueue(Body::VideoFrame(pb::VideoFrame {
        capture_session_id: Some(native_id(&frame.scope.capture_session_id)?.to_proto()),
        frame_id: frame.frame_id,
        geometry: Some(geometry),
        codec: "hevc".into(),
        keyframe: frame.keyframe,
        pts_ns: frame
            .pts_100ns
            .checked_mul(100)
            .ok_or(SessionError::Invalid)?,
        annexb: frame.annexb,
        last_input_seq_applied: frame.last_input_seq_applied,
        remote_scope: Some(scope_pb(&frame.scope)?),
        config_generation: config.generation,
        input_session_id: frame
            .input_session_id
            .as_ref()
            .map(|v| native_id(v).map(|v| v.to_proto()))
            .transpose()?,
        coded_width: config.coded_width,
        coded_height: config.coded_height,
        visible_width: config.visible_width,
        visible_height: config.visible_height,
        captured_qpc_100ns: frame.captured_qpc_100ns,
    }))?;
    hub.stream.increment(4);
    Ok(())
}
fn video_loop(
    hub: Arc<RemoteHub>,
    runtime: Runtime,
    selected: Selected,
    cancel: Arc<Cancellation>,
    keyframe: Arc<stream::Keyframes>,
) -> SessionResult<()> {
    let result = (|| {
        let (mut owner, ready) = Process::open(
            runtime.video,
            Command::OpenVideo {
                sequence: 1,
                target: selected.target.clone(),
                scope: selected.scope.clone(),
                owner_pid: std::process::id(),
            },
            &cancel,
        )
        .map_err(failure)?;
        if !matches!(
            ready.header,
            Header::Ready {
                capabilities: Some(_),
                ..
            }
        ) {
            return Err(SessionError::Invalid);
        }
        let mut config = None;
        let mut completed_keyframe = 0;
        loop {
            cancel.check().map_err(failure)?;
            let requested_keyframe = keyframe.generation();
            hub.stream.increment(0);
            let reply = owner
                .exchange(
                    Command::Poll {
                        sequence: owner.next_sequence().map_err(failure)?,
                        force_keyframe: keyframe.pending(completed_keyframe),
                    },
                    &cancel,
                )
                .map_err(failure)?;
            match reply.header {
                Header::Idle { .. } => hub.stream.increment(1),
                Header::Video {
                    scope,
                    config_generation,
                    frame_id,
                    pts_100ns,
                    captured_qpc_100ns,
                    keyframe: idr,
                    config: incoming,
                    ..
                } => {
                    hub.stream.increment(2);
                    if let Some(c) = incoming {
                        c.validate().map_err(failure)?;
                        if config.as_ref().is_some_and(|old| old != &c) {
                            return Err(SessionError::Invalid);
                        }
                        config = Some(c)
                    }
                    let c = config.as_ref().ok_or(SessionError::Invalid)?;
                    let frame = Frame {
                        scope,
                        config_generation,
                        frame_id,
                        pts_100ns,
                        captured_qpc_100ns,
                        input_session_id: None,
                        last_input_seq_applied: 0,
                        keyframe: idr,
                        annexb: reply.payload,
                    };
                    match publish_video(&hub, &selected, c, frame) {
                        Ok(()) => {
                            if idr {
                                completed_keyframe = requested_keyframe;
                            }
                        }
                        Err(SessionError::Backpressure) => keyframe.request(),
                        Err(e) => return Err(e),
                    }
                }
                _ => return Err(SessionError::Invalid),
            }
        }
    })();
    if result.is_err() && cancel.check().is_ok() {
        worker_error(&hub, &selected.scope, "video_owner_retired")
    }
    result
}
fn to_sample(target: &Target, event: pb::InputEvent) -> SessionResult<(u64, Either)> {
    let x = |v: f64, offset: i32, length: u32| -> SessionResult<i32> {
        if !v.is_finite() || v < 0.0 || v >= f64::from(length) {
            return Err(SessionError::Invalid);
        }
        offset
            .checked_add(v.round() as i32)
            .filter(|n| i64::from(*n) < i64::from(offset) + i64::from(length))
            .ok_or(SessionError::Invalid)
    };
    let value = match event.event.ok_or(SessionError::Invalid)? {
        pb::input_event::Event::Pen(v) => {
            if !v.pressure.is_finite()
                || !(0.0..=1.0).contains(&v.pressure)
                || !v.tilt_x.is_finite()
                || !v.tilt_y.is_finite()
                || !v.rotation.is_finite()
                || v.tilt_x.abs() > 90.0
                || v.tilt_y.abs() > 90.0
                || !(0.0..=359.0).contains(&v.rotation)
            {
                return Err(SessionError::Invalid);
            }
            let phase = match pb::PenPhase::try_from(v.phase).map_err(|_| SessionError::Invalid)? {
                pb::PenPhase::Hover => Phase::Hover,
                pb::PenPhase::Down => Phase::Down,
                pb::PenPhase::Move => Phase::Move,
                pb::PenPhase::Up => Phase::Up,
                pb::PenPhase::Cancel => Phase::Leave,
                _ => return Err(SessionError::Invalid),
            };
            let flags = v.native_pen_flags.ok_or(SessionError::Invalid)?;
            if flags & !7 != 0 || v.barrel != (flags & 1 != 0) || v.eraser != (flags & 4 != 0) {
                return Err(SessionError::Invalid);
            };
            Either::Pen(PenSample {
                phase,
                x: x(v.x, target.client_rect.x, target.client_rect.width)?,
                y: x(v.y, target.client_rect.y, target.client_rect.height)?,
                pressure: (v.pressure * 1024.0).round() as u32,
                tilt_x: v.tilt_x.round() as i32,
                tilt_y: v.tilt_y.round() as i32,
                rotation: v.rotation.round() as u32,
                pen_flags: flags,
            })
        }
        pb::input_event::Event::Finite(v) => {
            let value = match v.kind.as_str() {
                "click" => FiniteAction::Click {
                    x: x(v.x, target.client_rect.x, target.client_rect.width)?,
                    y: x(v.y, target.client_rect.y, target.client_rect.height)?,
                    button: u8::try_from(v.button).map_err(|_| SessionError::Invalid)?,
                },
                "wheel" => FiniteAction::Wheel {
                    x: x(v.x, target.client_rect.x, target.client_rect.width)?,
                    y: x(v.y, target.client_rect.y, target.client_rect.height)?,
                    delta: v.wheel_delta,
                },
                "shortcut" => FiniteAction::Shortcut {
                    profile_digest: v.profile_digest,
                    action: v.editor_action,
                    focus_runtime_hash: v.focus_runtime_hash,
                },
                _ => return Err(SessionError::Invalid),
            };
            Either::Finite(value)
        }
        // No arbitrary KeyEvent or held mouse state is admitted in M4.
        _ => return Err(SessionError::Invalid),
    };
    if event.input_seq == 0 {
        return Err(SessionError::Invalid);
    }
    Ok((event.input_seq, value))
}
enum Either {
    Pen(PenSample),
    Finite(FiniteAction),
}
fn input_loop(
    hub: Arc<RemoteHub>,
    mut owner: Process,
    target: Target,
    binding: Binding,
    receive: mpsc::Receiver<Pending>,
    cancel: Arc<Cancellation>,
) -> SessionResult<()> {
    let result = (|| {
        let mut check = Instant::now();
        loop {
            cancel.check().map_err(failure)?;
            match receive.recv_timeout(Duration::from_millis(5)) {
                Ok(p) => {
                    if p.binding != binding || p.arrived.elapsed() >= Duration::from_millis(50) {
                        hub.injected_command(&binding, &p.event, &Err(vw_remote::Error::Invalid))?;
                        return Err(SessionError::Invalid);
                    }
                    {
                        let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
                        RemoteHub::current(&s)?;
                        if s.binding.as_ref() != Some(&binding) {
                            return Err(SessionError::Closed);
                        }
                    }
                    let receipt_event = p.event.clone();
                    let (seq, event) = match to_sample(&target, p.event) {
                        Ok(v) => v,
                        Err(error) => {
                            hub.injected_command(
                                &binding,
                                &receipt_event,
                                &Err(vw_remote::Error::Invalid),
                            )?;
                            return Err(error);
                        }
                    };
                    let sequence = owner.next_sequence().map_err(failure)?;
                    let request = match event {
                        Either::Pen(sample) => Command::Pen {
                            sequence,
                            input_seq: seq,
                            sample,
                        },
                        Either::Finite(action) => Command::Finite {
                            sequence,
                            input_seq: seq,
                            action,
                        },
                    };
                    let reply = match owner.exchange(request, &cancel) {
                        Ok(reply) => reply,
                        Err(error) => {
                            let _receipt =
                                hub.injected_command(&binding, &receipt_event, &Err(error.clone()));
                            return Err(failure(error));
                        }
                    };
                    let Header::Injected {
                        binding: actual,
                        input_seq,
                        accepted_qpc_100ns,
                        ..
                    } = reply.header
                    else {
                        hub.injected_command(
                            &binding,
                            &receipt_event,
                            &Err(vw_remote::Error::Invalid),
                        )?;
                        return Err(SessionError::Invalid);
                    };
                    if actual != binding || input_seq != seq || accepted_qpc_100ns == 0 {
                        hub.injected_command(
                            &binding,
                            &receipt_event,
                            &Err(vw_remote::Error::Invalid),
                        )?;
                        return Err(SessionError::Invalid);
                    }
                    let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
                    if s.binding.as_ref() != Some(&binding) {
                        return Err(SessionError::Closed);
                    }
                    s.applied
                        .as_mut()
                        .ok_or(SessionError::Authentication)?
                        .record(Receipt {
                            binding: actual,
                            input_seq,
                            accepted_qpc_100ns,
                        })
                        .map_err(failure)?;
                    drop(s);
                    hub.injected_command(&binding, &receipt_event, &Ok(accepted_qpc_100ns))?;
                    if let Some(pb::input_event::Event::Finite(finite)) = &receipt_event.event
                        && finite.kind == "shortcut"
                    {
                        // The pen barrier was acquired before phone enqueue. Applied
                        // record and injected ACK already exist; UIA cannot delay them.
                        let refreshed = owner
                            .exchange(
                                Command::RefreshAuthority {
                                    sequence: owner.next_sequence().map_err(failure)?,
                                    profile_digest: finite.profile_digest.clone(),
                                },
                                &cancel,
                            )
                            .map_err(failure)?;
                        let Header::Authority {
                            binding: actual,
                            authority,
                            ..
                        } = refreshed.header
                        else {
                            return Err(SessionError::Invalid);
                        };
                        if actual != binding {
                            return Err(SessionError::Authentication);
                        }
                        hub.publish_authority(
                            &binding,
                            seq,
                            finite.editor_action,
                            receipt_event.request_nonce,
                            authority,
                        )?;
                    }
                    check = Instant::now();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(SessionError::Closed),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if check.elapsed() >= Duration::from_millis(20) {
                owner
                    .exchange(
                        Command::Check {
                            sequence: owner.next_sequence().map_err(failure)?,
                        },
                        &cancel,
                    )
                    .map_err(failure)?;
                check = Instant::now();
            }
        }
    })();
    let retired = owner.close();
    if retired.is_ok()
        && let Some(witness) = owner.pen_release_witness()
    {
        hub.integration_pen_release(witness);
    }
    if result.is_err() && cancel.check().is_ok() {
        worker_error(&hub, &binding.scope, "input_owner_retired")
    }
    result
}
impl RemoteHub {
    pub(super) fn configure_runtime(
        self: &Arc<Self>,
        files: RemoteRuntimeFiles,
    ) -> SessionResult<()> {
        if !self.host {
            return Err(SessionError::Invalid);
        }
        let video = LockedHelper::open(
            Path::new(&files.video_path),
            &files.video_sha256,
            Kind::Video,
        )
        .map_err(failure)?;
        let input = LockedHelper::open(
            Path::new(&files.input_path),
            &files.input_sha256,
            Kind::Input,
        )
        .map_err(failure)?;
        let _operation = self.operations.lock().map_err(|_| SessionError::Worker)?;
        let profile = match (files.profile_path, files.profile_sha256) {
            (None, None) => None,
            (Some(p), Some(h)) => {
                Some(process::load_packaged_profile(Path::new(&p), &h).map_err(failure)?)
            }
            _ => return Err(SessionError::Invalid),
        };
        self.close_workers()?;
        self.state.lock().map_err(|_| SessionError::Worker)?.runtime = Some(Runtime {
            video,
            input,
            profile,
        });
        Ok(())
    }
    pub(super) fn select_window(
        self: &Arc<Self>,
        window: u64,
        process_id: u32,
        process_created: u64,
    ) -> SessionResult<()> {
        if !self.host {
            return Err(SessionError::Invalid);
        }
        let _operation = self.operations.lock().map_err(|_| SessionError::Worker)?;
        self.close_workers()?;
        let target =
            platform::query_target(window, std::process::id(), fresh_id()?).map_err(failure)?;
        if target.process_id != process_id || target.process_created != process_created {
            return Err(SessionError::Authentication);
        }
        let display_label =
            vw_remote_host::picker::describe(&target, std::process::id()).map_err(failure)?;
        let (mut selected, runtime) = {
            let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
            RemoteHub::current(&s)?;
            RemoteHub::retire(&mut s);
            let selected = Selected {
                target,
                display_label,
                scope: Scope {
                    connection_epoch: s.epoch,
                    capture_session_id: fresh_id()?,
                    source_generation: s
                        .display
                        .sequence
                        .checked_add(1)
                        .ok_or(SessionError::Backpressure)?,
                    target_token: String::new(),
                    geometry_revision: 1,
                },
            };
            let runtime = s.runtime.clone().ok_or(SessionError::Invalid)?;
            (selected, runtime)
        };
        selected.scope.target_token = selected.target.token.clone();
        let cancel = Arc::new(Cancellation::default());
        let keyframe = Arc::new(stream::Keyframes::default());
        keyframe.request();
        let hub = self.clone();
        let c = cancel.clone();
        let k = keyframe.clone();
        let _starting = Starting::new(self.clone(), cancel.clone())?;
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        RemoteHub::current(&s)?;
        // close_workers above proved actual old input/video settlement under
        // this operation lock. Finish A's notification in the SAME state lock
        // as B's adoption so a final A background cannot re-arm it in between.
        RemoteHub::finish_retirement_notice(&mut s, |s| self.send_control(s, "retired", ""))?;
        RemoteHub::replace_selected(&mut s, selected.clone())?;
        s.config = None;
        s.binding = None;
        s.last_frame = 0;
        self.send_control(&mut s, "start", "")?;
        self.publish(&mut s, "viewing", None)?;
        drop(s);
        let thread = match thread::Builder::new()
            .name("vw-remote-video-owner".into())
            .spawn(move || video_loop(hub, runtime, selected, c, k))
        {
            Ok(v) => v,
            Err(_) => {
                let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
                RemoteHub::retire(&mut s);
                self.publish(&mut s, "sealed", Some("video_worker_unavailable"))?;
                return Err(SessionError::Worker);
            }
        };
        let mut workers = self.workers.lock().map_err(|_| SessionError::Worker)?;
        workers.keyframe = Some(keyframe);
        workers.video = Some(Worker {
            cancel,
            thread: Some(thread),
        });
        Ok(())
    }
    pub(super) fn grant(self: &Arc<Self>) -> SessionResult<()> {
        if !self.host {
            return Err(SessionError::Invalid);
        }
        let _operation = self.operations.lock().map_err(|_| SessionError::Worker)?;
        let (selected, runtime, revision) = {
            let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
            RemoteHub::current(&s)?;
            RemoteHub::retire(&mut s);
            (
                s.selected.clone().ok_or(SessionError::Invalid)?,
                s.runtime.clone().ok_or(SessionError::Invalid)?,
                s.grant_revision,
            )
        };
        {
            let mut workers = self.workers.lock().map_err(|_| SessionError::Worker)?;
            workers.cancel_input();
            workers.input_send.take();
            if let Some(w) = &mut workers.input {
                w.close()?;
            }
            workers.input.take();
        }
        if let Err(e) = platform::focus_for_owner_grant(&selected.target, std::process::id()) {
            let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
            self.publish(&mut s, "pending_focus", Some("target_focus_required"))?;
            return Err(failure(e));
        }
        let binding = Binding {
            scope: selected.scope.clone(),
            input_session_id: fresh_id()?,
        };
        let cancel = Arc::new(Cancellation::default());
        let _starting = Starting::new(self.clone(), cancel.clone())?;
        let (mut process, ready) = Process::open(
            runtime.input,
            Command::OpenInput {
                sequence: 1,
                target: selected.target.clone(),
                binding: binding.clone(),
                owner_pid: std::process::id(),
                profile: runtime.profile,
            },
            &cancel,
        )
        .map_err(failure)?;
        let Header::Ready {
            input_authority: authority,
            ..
        } = ready.header
        else {
            let _ = process.close();
            return Err(SessionError::Invalid);
        };
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        if let Err(error) = RemoteHub::current(&s) {
            drop(s);
            let _ = process.close();
            return Err(error);
        }
        if s.grant_revision != revision
            || s.selected
                .as_ref()
                .is_none_or(|v| v.scope != selected.scope || v.target != selected.target)
        {
            drop(s);
            let _ = process.close();
            return Err(SessionError::Closed);
        }
        let now_ms = s.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        s.sender.as_ref().ok_or(SessionError::Closed)?.grant_input(
            native_id(&binding.input_session_id)?,
            native_id(&binding.scope.capture_session_id)?,
            binding.scope.geometry_revision,
            now_ms,
        )?;
        s.binding = Some(binding.clone());
        s.authority = authority;
        s.applied = Some(Applied::new(binding.clone()).map_err(failure)?);
        s.last_input = Instant::now();
        let (send, receive) = mpsc::sync_channel(8);
        let hub = self.clone();
        let c = cancel.clone();
        let target = selected.target;
        let thread = match thread::Builder::new()
            .name("vw-remote-input-owner".into())
            .spawn(move || input_loop(hub, process, target, binding, receive, c))
        {
            Ok(v) => v,
            Err(_) => {
                RemoteHub::retire(&mut s);
                self.publish(&mut s, "sealed", Some("input_worker_unavailable"))?;
                return Err(SessionError::Worker);
            }
        };
        {
            let mut workers = self.workers.lock().map_err(|_| SessionError::Worker)?;
            workers.input_send = Some(send);
            workers.input = Some(Worker {
                cancel,
                thread: Some(thread),
            });
        }
        if let Err(error) = self
            .send_control(&mut s, "grant", "")
            .and_then(|()| self.publish(&mut s, "controlling", None))
        {
            RemoteHub::retire(&mut s);
            drop(s);
            self.cancel_workers();
            return Err(error);
        }
        Ok(())
    }
}

pub(super) fn windows() -> SessionResult<Vec<RemoteWindow>> {
    vw_remote_host::picker::windows(std::process::id())
        .map_err(failure)
        .map(|values| {
            values
                .into_iter()
                .map(|v| RemoteWindow {
                    window: v.window,
                    process_id: v.process_id,
                    process_created: v.process_created,
                    label: v.label,
                    executable_name: v.executable_name,
                })
                .collect()
        })
}
