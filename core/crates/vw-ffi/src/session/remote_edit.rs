//! Borrowed authenticated remote-edit capability. Native producer/grant/frame
//! ownership is retired before carrier or library close can complete.
use super::{LiveSession, SessionError, SessionResult};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};
use tokio::sync::watch;
use vw_model::Id;
use vw_net::ConnectionSender;
use vw_proto::v1::{self as pb, envelope::Body};
use vw_remote::{Binding, Frame, Scope, Target, VideoConfig, applied::Applied};
#[path = "remote_admission.rs"]
mod admission;
#[path = "remote_commands.rs"]
mod commands;
#[cfg(windows)]
#[path = "remote_host.rs"]
mod host;
#[path = "remote_integration_fault.rs"]
mod integration_fault;
#[path = "remote_retired.rs"]
mod retired;
#[path = "remote_stream.rs"]
mod stream;
use commands::PendingCommand;
pub use commands::RemoteCommandResult;

#[derive(Clone, Debug, uniffi::Record)]
pub struct RemoteRuntimeFiles {
    pub video_path: String,
    pub video_sha256: String,
    pub input_path: String,
    pub input_sha256: String,
    pub profile_path: Option<String>,
    pub profile_sha256: Option<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct RemoteDisplay {
    pub sequence: u64,
    pub local_epoch: u64,
    pub status: String,
    pub scope_json: Option<String>,
    pub target_json: Option<String>,
    pub binding_json: Option<String>,
    pub authority_json: Option<String>,
    pub reason: Option<String>,
    pub encoder_description: Option<String>,
    pub destination_label: Option<String>,
    pub command_busy: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct RemoteFrame {
    pub ticket: u64,
    pub config_json: String,
    pub frame_id: u64,
    pub pts_us: i64,
    pub keyframe: bool,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct RemoteAcknowledgment {
    pub binding_json: String,
    pub frame_id: u64,
    pub ticket: u64,
    pub last_input_seq_applied: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct RemoteInputAdmitted {
    pub binding_json: String,
    pub input_sequence: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct RemoteWindow {
    pub window: u64,
    pub process_id: u32,
    pub process_created: u64,
    pub label: String,
    pub executable_name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Selected {
    pub target: Target,
    pub scope: Scope,
    pub display_label: String,
}
pub(super) struct State {
    epoch: u64,
    // Host-origin source scope epoch is distinct from this endpoint's local
    // carrier lifetime. Peer adoption occurs only on a fully valid Start.
    source_epoch: Option<u64>,
    enabled: bool,
    closed: bool,
    host_retiring: bool,
    peer_retiring: bool,
    sender: Option<ConnectionSender>,
    started: Instant,
    grant_revision: u64,
    sent_control: u64,
    received_control: u64,
    selected: Option<Selected>,
    retired: retired::History,
    binding: Option<Binding>,
    authority: Option<vw_remote::profile::Authority>,
    config: Option<VideoConfig>,
    last_frame: u64,
    next_ticket: u64,
    held: BTreeMap<u64, Frame>,
    ready: VecDeque<u64>,
    awaiting_idr: bool,
    // MEDIA has no authority to introduce a source. Only remember bounded loss;
    // a later valid CONTROL Start/config requests a real replacement IDR.
    prestart_media_lost: bool,
    start_recovery_pending: bool,
    input_seq: u64,
    last_input: Instant,
    applied: Option<Applied>,
    display: RemoteDisplay,
    commands: BTreeMap<u64, PendingCommand>,
    next_command: u64,
    admission: admission::Admission,
    #[cfg(windows)]
    runtime: Option<host::Runtime>,
}
pub(super) struct RemoteHub {
    integration_fault: Mutex<integration_fault::FaultGate>,
    integration_release: Mutex<integration_fault::ReleaseGate>,
    host: bool,
    stream: stream::Diagnostics,
    state: Mutex<State>,
    signal: watch::Sender<RemoteDisplay>,
    #[cfg(windows)]
    workers: Mutex<host::Workers>,
    #[cfg(windows)]
    operations: Mutex<()>,
    // Separate short local registry: cancellation never waits behind a helper
    // retirement observation while its synthetic contact refreshes autonomously.
    #[cfg(windows)]
    cancellations: Mutex<Vec<std::sync::Weak<vw_remote_host::process::Cancellation>>>,
}
pub(super) struct RemoteEpoch {
    hub: Arc<RemoteHub>,
    epoch: u64,
}
impl Drop for RemoteEpoch {
    fn drop(&mut self) {
        self.hub.deactivate(self.epoch);
    }
}
fn failure(error: vw_remote::Error) -> SessionError {
    match error {
        vw_remote::Error::Invalid => SessionError::Invalid,
        vw_remote::Error::Limit => SessionError::Backpressure,
        vw_remote::Error::TargetChanged | vw_remote::Error::Ungranted => {
            SessionError::Authentication
        }
        vw_remote::Error::Unavailable => SessionError::RemoteUnavailable,
        vw_remote::Error::Timeout => SessionError::Timeout,
        vw_remote::Error::Cancelled => SessionError::Cancelled,
        vw_remote::Error::RetirementPending => SessionError::RemoteRetirementPending,
        vw_remote::Error::PartialInput => SessionError::RemotePartialInput,
        _ => SessionError::Worker,
    }
}
fn json<T: Serialize>(value: &T) -> SessionResult<String> {
    serde_json::to_string(value).map_err(|_| SessionError::Invalid)
}
fn parse<T: serde::de::DeserializeOwned>(value: &str, limit: usize) -> SessionResult<T> {
    if value.len() > limit {
        return Err(SessionError::Invalid);
    }
    serde_json::from_str(value).map_err(|_| SessionError::Invalid)
}
fn native_id(v: &str) -> SessionResult<Id> {
    Id::try_from(v.to_owned()).map_err(|_| SessionError::Invalid)
}
#[cfg(windows)]
fn fresh_id() -> SessionResult<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| SessionError::Invalid)?
        .as_millis();
    let now = u64::try_from(now).map_err(|_| SessionError::Invalid)?;
    Id::generate(now)
        .map(|v| v.to_string())
        .map_err(|_| SessionError::Worker)
}
pub(super) fn scope_pb(v: &Scope) -> SessionResult<pb::RemoteScope> {
    Ok(pb::RemoteScope {
        connection_epoch: v.connection_epoch,
        capture_session_id: Some(native_id(&v.capture_session_id)?.to_proto()),
        source_generation: v.source_generation,
        target_token: Some(native_id(&v.target_token)?.to_proto()),
        geometry_revision: v.geometry_revision,
    })
}
fn scope_from(v: Option<pb::RemoteScope>) -> SessionResult<Scope> {
    let v = v.ok_or(SessionError::Invalid)?;
    let s = Scope {
        connection_epoch: v.connection_epoch,
        capture_session_id: Id::from_proto(v.capture_session_id.as_ref())
            .map_err(|_| SessionError::Invalid)?
            .to_string(),
        source_generation: v.source_generation,
        target_token: Id::from_proto(v.target_token.as_ref())
            .map_err(|_| SessionError::Invalid)?
            .to_string(),
        geometry_revision: v.geometry_revision,
    };
    s.validate().map_err(failure)?;
    Ok(s)
}
#[cfg(windows)]
fn config_pb(v: &VideoConfig) -> SessionResult<pb::RemoteVideoConfig> {
    Ok(pb::RemoteVideoConfig {
        scope: Some(scope_pb(&v.scope)?),
        generation: v.generation,
        visible_width: v.visible_width,
        visible_height: v.visible_height,
        coded_width: v.coded_width,
        coded_height: v.coded_height,
        vps: v.vps.clone(),
        sps: v.sps.clone(),
        pps: v.pps.clone(),
        encoder_capabilities_json: json(&v.encoder)?.into_bytes(),
    })
}
fn config_from(v: pb::RemoteVideoConfig) -> SessionResult<VideoConfig> {
    if v.encoder_capabilities_json.len() > 4096 {
        return Err(SessionError::Invalid);
    }
    let c = VideoConfig {
        scope: scope_from(v.scope)?,
        generation: v.generation,
        visible_width: v.visible_width,
        visible_height: v.visible_height,
        coded_width: v.coded_width,
        coded_height: v.coded_height,
        vps: v.vps,
        sps: v.sps,
        pps: v.pps,
        encoder: serde_json::from_slice(&v.encoder_capabilities_json)
            .map_err(|_| SessionError::Invalid)?,
    };
    c.validate().map_err(failure)?;
    Ok(c)
}
impl RemoteHub {
    pub(super) fn new(host: bool) -> Arc<Self> {
        let now = Instant::now();
        let display = RemoteDisplay {
            sequence: 1,
            local_epoch: 0,
            status: "disconnected".into(),
            scope_json: None,
            target_json: None,
            binding_json: None,
            authority_json: None,
            reason: None,
            encoder_description: None,
            destination_label: None,
            command_busy: false,
        };
        let (signal, _) = watch::channel(display.clone());
        Arc::new(Self {
            integration_fault: Mutex::new(integration_fault::FaultGate::default()),
            integration_release: Mutex::new(integration_fault::ReleaseGate::default()),
            host,
            stream: stream::Diagnostics::default(),
            state: Mutex::new(State {
                epoch: 0,
                source_epoch: None,
                enabled: false,
                closed: false,
                host_retiring: false,
                peer_retiring: false,
                sender: None,
                started: now,
                grant_revision: 0,
                sent_control: 0,
                received_control: 0,
                selected: None,
                retired: retired::History::default(),
                binding: None,
                authority: None,
                config: None,
                last_frame: 0,
                next_ticket: 0,
                held: BTreeMap::new(),
                ready: VecDeque::new(),
                awaiting_idr: false,
                prestart_media_lost: false,
                start_recovery_pending: false,
                input_seq: 0,
                last_input: now,
                applied: None,
                display,
                commands: BTreeMap::new(),
                next_command: 0,
                admission: admission::Admission::default(),
                #[cfg(windows)]
                runtime: None,
            }),
            signal,
            #[cfg(windows)]
            workers: Mutex::new(host::Workers::default()),
            #[cfg(windows)]
            operations: Mutex::new(()),
            #[cfg(windows)]
            cancellations: Mutex::new(Vec::new()),
        })
    }
    fn publish(&self, s: &mut State, status: &str, reason: Option<&str>) -> SessionResult<()> {
        s.display = RemoteDisplay {
            sequence: s
                .display
                .sequence
                .checked_add(1)
                .ok_or(SessionError::Backpressure)?,
            local_epoch: s.epoch,
            status: status.into(),
            scope_json: s.selected.as_ref().map(|v| json(&v.scope)).transpose()?,
            target_json: s.selected.as_ref().map(|v| json(&v.target)).transpose()?,
            binding_json: s.binding.as_ref().map(json).transpose()?,
            authority_json: s.authority.as_ref().map(json).transpose()?,
            reason: reason.map(str::to_owned),
            encoder_description: s.config.as_ref().map(|v| v.encoder.name.clone()),
            destination_label: s.selected.as_ref().map(|v| v.display_label.clone()),
            command_busy: s.admission.busy(),
        };
        self.signal.send_replace(s.display.clone());
        Ok(())
    }
    pub(super) fn activate(
        self: &Arc<Self>,
        epoch: u64,
        negotiated: bool,
        sender: ConnectionSender,
        started: Instant,
    ) -> SessionResult<RemoteEpoch> {
        self.close_workers()?;
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        if s.closed || epoch & 1 == 0 || epoch <= s.epoch {
            return Err(SessionError::Closed);
        }
        s.epoch = epoch;
        Self::reset_source(&mut s, self.host.then_some(epoch));
        s.enabled = negotiated;
        s.sender = Some(sender);
        s.started = started;
        s.sent_control = 0;
        s.received_control = 0;
        s.host_retiring = false;
        s.peer_retiring = false;
        s.binding = None;
        s.admission = admission::Admission::default();
        s.authority = None;
        s.config = None;
        s.held.clear();
        s.ready.clear();
        s.awaiting_idr = false;
        s.last_frame = 0;
        s.input_seq = 0;
        s.applied = None;
        s.commands.clear();
        self.publish(
            &mut s,
            if negotiated {
                "selecting"
            } else {
                "unavailable"
            },
            None,
        )?;
        Ok(RemoteEpoch {
            hub: self.clone(),
            epoch,
        })
    }
    fn deactivate(&self, epoch: u64) {
        if let Ok(mut s) = self.state.lock() {
            if s.epoch != epoch {
                return;
            }
            self.integration_release_cause(&s, "connection_retired", None);
            Self::retire(&mut s);
            s.enabled = false;
            s.sender = None;
            s.peer_retiring = false;
            s.epoch = epoch.saturating_add(1);
            let _ = self.publish(&mut s, "disconnected", Some("connection_retired"));
        }
        self.cancel_workers();
    }
    fn retire(s: &mut State) {
        Self::retire_authority(s);
        s.held.clear();
        s.ready.clear();
        s.awaiting_idr = false;
    }
    // An input grant/revocation within the same video scope must not revoke a
    // valid decoder ticket. Its actual callback may drain as video only; input
    // acknowledgment still requires the exact current input-session binding.
    fn retire_authority(s: &mut State) {
        if let Some(sender) = &s.sender {
            sender.revoke_input()
        }
        if let Some(a) = &mut s.applied {
            a.retire()
        }
        s.grant_revision = s.grant_revision.saturating_add(1);
        if s.grant_revision == u64::MAX {
            s.closed = true
        }
        for (ticket, c) in &mut s.commands {
            c.refresh_done = true;
            if c.result.is_none() {
                c.result = Some(RemoteCommandResult {
                    binding_json: json(&c.binding).unwrap_or_default(),
                    request_ticket: *ticket,
                    input_sequence: 0,
                    action: c.action,
                    accepted_qpc_100ns: 0,
                    status: "refused".into(),
                    reason: Some("host_revoked".into()),
                });
            }
        }
        s.binding = None;
        s.admission = admission::Admission::default();
        s.authority = None;
        s.applied = None;
    }
    fn current(s: &State) -> SessionResult<()> {
        if s.closed || !s.enabled || s.epoch & 1 == 0 || s.sender.is_none() {
            return Err(SessionError::Closed);
        }
        Ok(())
    }
    fn reset_source(s: &mut State, source_epoch: Option<u64>) {
        s.source_epoch = source_epoch;
        s.selected = None;
        s.retired.clear();
        s.prestart_media_lost = false;
        s.start_recovery_pending = false;
    }
    fn validate_selected(s: &State, selected: &Selected) -> SessionResult<()> {
        selected.scope.validate().map_err(failure)?;
        selected.target.validate().map_err(failure)?;
        if s.source_epoch
            .is_some_and(|epoch| selected.scope.connection_epoch != epoch)
            || selected.scope.target_token != selected.target.token
            || s.selected
                .as_ref()
                .is_some_and(|old| selected.scope.source_generation <= old.scope.source_generation)
            || selected.scope.source_generation <= s.retired.latest_generation()
        {
            return Err(SessionError::Invalid);
        }
        Ok(())
    }
    // Keep exact disposal identity across a view close on the same carrier.
    // Old CONTROL/MEDIA may arrive after the decoder actually retired; neither
    // those packets nor a stale Start may restore this selected source.
    fn retire_selected(s: &mut State) {
        if let Some(selected) = s.selected.take() {
            s.retired.remember(selected, s.config.take());
        } else {
            s.config = None;
        }
    }
    fn replace_selected(s: &mut State, selected: Selected) -> SessionResult<()> {
        Self::validate_selected(s, &selected)?;
        Self::retire_selected(s);
        s.source_epoch = Some(selected.scope.connection_epoch);
        s.selected = Some(selected);
        s.config = None;
        s.last_frame = 0;
        Ok(())
    }
    fn send_control(&self, s: &mut State, action: &str, reason: &str) -> SessionResult<()> {
        Self::current(s)?;
        let body = Self::control_body(s, action, reason)?;
        s.sender
            .as_ref()
            .ok_or(SessionError::Closed)?
            .enqueue(body)?;
        Ok(())
    }
    fn control_body(s: &mut State, action: &str, reason: &str) -> SessionResult<Body> {
        if !vw_net::remote_reason_allowed(reason) {
            return Err(SessionError::Invalid);
        }
        let selected = s.selected.as_ref().ok_or(SessionError::Invalid)?;
        s.sent_control = s
            .sent_control
            .checked_add(1)
            .ok_or(SessionError::Backpressure)?;
        let body = pb::RemoteControl {
            scope: Some(scope_pb(&selected.scope)?),
            sequence: s.sent_control,
            action: action.into(),
            selected_target_json: if action == "start" {
                json(&selected.target)?.into_bytes()
            } else {
                Vec::new()
            },
            input_session_id: s
                .binding
                .as_ref()
                .map(|b| native_id(&b.input_session_id).map(|i| i.to_proto()))
                .transpose()?,
            reason: reason.into(),
            shortcut_state_json: if action == "grant" {
                s.authority
                    .as_ref()
                    .map(json)
                    .transpose()?
                    .unwrap_or_default()
                    .into_bytes()
            } else {
                Vec::new()
            },
            request_nonce: 0,
            command_input_seq: 0,
            editor_action: 0,
            selected_destination_label: if action == "start" {
                selected.display_label.clone()
            } else {
                String::new()
            },
        };
        Ok(Body::RemoteControl(body))
    }
    // Exact bounded recovery/enqueue transaction used by real flush. Tests may
    // observe/refuse this enqueue without manufacturing a carrier or source.
    fn flush_start_recovery(
        s: &mut State,
        enqueue: impl FnOnce(Body) -> SessionResult<()>,
    ) -> SessionResult<()> {
        if s.start_recovery_pending && s.selected.is_some() && s.config.is_some() {
            let body = Self::control_body(s, "keyframe", "")?;
            enqueue(body)?;
            s.start_recovery_pending = false;
        }
        Ok(())
    }
    pub(super) fn stop(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.closed = true;
            Self::retire(&mut s);
            s.enabled = false;
            s.sender = None;
            let _ = self.publish(&mut s, "closing", None);
        }
        self.cancel_workers();
    }
    fn cancel_workers(&self) {
        #[cfg(windows)]
        {
            // Registry operations are bounded local pointer/atomic work only. No
            // provider, OS, exchange, join or slow observation runs under this lock.
            let mut registered = self.cancellations.lock().unwrap_or_else(|e| e.into_inner());
            registered.retain(|weak| {
                if let Some(cancel) = weak.upgrade() {
                    cancel.cancel();
                    true
                } else {
                    false
                }
            });
        }
    }
    fn close_workers(&self) -> SessionResult<()> {
        #[cfg(windows)]
        {
            self.workers
                .lock()
                .map_err(|_| SessionError::Worker)?
                .close()?;
            if host::pending_retirement_count()? != 0 {
                return Err(SessionError::RemoteRetirementPending);
            }
        }
        Ok(())
    }
    pub(super) fn close(&self) -> SessionResult<()> {
        self.stop();
        #[cfg(windows)]
        let _operation = self
            .operations
            .try_lock()
            .map_err(|_| SessionError::RemoteRetirementPending)?;
        self.close_workers()?;
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        self.publish(&mut s, "closed", None)
    }
    fn close_view(&self) -> SessionResult<()> {
        self.pause("owner_pause")?;
        #[cfg(windows)]
        let _operation = self
            .operations
            .try_lock()
            .map_err(|_| SessionError::RemoteRetirementPending)?;
        self.finish_close_view(|| self.close_workers())
    }
    fn finish_close_view(
        &self,
        settle_workers: impl FnOnce() -> SessionResult<()>,
    ) -> SessionResult<()> {
        // The real call retains the operation guard and settles actual worker,
        // process, Job and IO owners first. A lost carrier alone proves no UP.
        settle_workers()?;
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        if !s.enabled && s.sender.is_none() && (s.closed || s.epoch & 1 == 0) {
            // Exact carrier deactivation (even epoch), or explicit terminal stop,
            // makes notification impossible. This is not a catch for Closed:
            // active/ambiguous carriers still require their normal handshake.
            s.host_retiring = false;
            s.peer_retiring = false;
        }
        if s.peer_retiring {
            return Err(SessionError::RemoteRetirementPending);
        }
        if s.host_retiring {
            self.send_control(&mut s, "retired", "")?;
            s.host_retiring = false;
        }
        Self::retire_selected(&mut s);
        let status = if s.enabled {
            "selecting"
        } else {
            "disconnected"
        };
        self.publish(&mut s, status, None)
    }
    pub(super) fn flush(&self) -> SessionResult<()> {
        if !self.host {
            let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
            if s.start_recovery_pending && s.selected.is_some() && s.config.is_some() {
                // Scope and config were admitted on ordered CONTROL. The lost
                // pre-Start MEDIA never supplied either, nor a decoder ticket.
                Self::current(&s)?;
                let sender = s.sender.clone().ok_or(SessionError::Closed)?;
                Self::flush_start_recovery(&mut s, |body| {
                    sender.enqueue(body)?;
                    Ok(())
                })?;
            }
        }
        #[cfg(windows)]
        {
            let retiring = self
                .state
                .lock()
                .map_err(|_| SessionError::Worker)?
                .host_retiring;
            if retiring && let Ok(_operation) = self.operations.try_lock() {
                match self.flush_retiring_workers(|| self.close_workers()) {
                    Ok(()) => {}
                    Err(SessionError::RemoteRetirementPending) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        let expired = {
            let s = self.state.lock().map_err(|_| SessionError::Worker)?;
            s.binding.is_some() && s.last_input.elapsed() >= Duration::from_secs(600)
        };
        if expired {
            self.pause("input_expired")?;
        }
        Ok(())
    }
    #[cfg(any(windows, test))]
    fn finish_retirement_notice(
        s: &mut State,
        notify: impl FnOnce(&mut State) -> SessionResult<()>,
    ) -> SessionResult<()> {
        if s.host_retiring {
            // Caller holds operations and has already settled the actual old
            // workers. A failed enqueue retains this exact source obligation.
            notify(s)?;
            s.host_retiring = false;
        }
        Ok(())
    }
    #[cfg(any(windows, test))]
    fn flush_retiring_workers(
        &self,
        settle_workers: impl FnOnce() -> SessionResult<()>,
    ) -> SessionResult<()> {
        // Caller holds operations. The earlier unlocked flag was only a hint:
        // selection may have settled A and started B before this lock was won.
        // Never let that old hint close B's actual worker.
        if !self
            .state
            .lock()
            .map_err(|_| SessionError::Worker)?
            .host_retiring
        {
            return Ok(());
        }
        settle_workers()?;
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        if s.host_retiring && s.enabled {
            Self::finish_retirement_notice(&mut s, |s| self.send_control(s, "retired", ""))?;
            self.publish(&mut s, "paused", None)?;
        }
        Ok(())
    }
    fn pause(&self, reason: &str) -> SessionResult<()> {
        if !vw_net::remote_reason_allowed(reason) {
            return Err(SessionError::Invalid);
        }
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        if s.selected.is_some() && s.enabled {
            let _ = self.send_control(
                &mut s,
                if self.host { "revoke" } else { "background" },
                reason,
            );
        }
        if self.host {
            s.host_retiring = s.selected.is_some();
        } else if s.binding.is_some() {
            s.peer_retiring = true;
        }
        if self.host {
            Self::retire(&mut s);
        } else {
            Self::retire_authority(&mut s);
        }
        self.publish(&mut s, "paused", Some(reason))?;
        drop(s);
        self.cancel_workers();
        Ok(())
    }
    pub(super) fn recover(&self) -> SessionResult<()> {
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        if s.selected.is_none() {
            return Ok(());
        } // Keyframe recovery may drop an unissued ready frame. Issued decoder
        // tickets remain owned until explicit discard after actual retirement.
        discard_ready(&mut s);
        self.send_control(&mut s, "keyframe", "")?;
        #[cfg(windows)]
        if self.host {
            self.workers
                .lock()
                .map_err(|_| SessionError::Worker)?
                .request_keyframe();
        }
        Ok(())
    }
    pub(super) fn receive(self: &Arc<Self>, body: Body, epoch: u64) -> SessionResult<()> {
        let s = self.state.lock().map_err(|_| SessionError::Worker)?;
        Self::current(&s)?;
        if s.epoch != epoch {
            return Err(SessionError::Closed);
        }
        self.receive_locked(body, s)
    }
    // Only receive() enters this path after actual carrier/connection admission.
    // Pure tests drive this exact handler without manufacturing a live sender.
    fn receive_locked(
        self: &Arc<Self>,
        body: Body,
        mut s: std::sync::MutexGuard<'_, State>,
    ) -> SessionResult<()> {
        match body {
            Body::RemoteControl(v) => {
                if v.sequence == 0
                    || v.sequence <= s.received_control
                    || !vw_net::remote_reason_allowed(&v.reason)
                    || v.selected_destination_label.len() > 2048
                    || v.selected_destination_label.chars().any(char::is_control)
                    || (v.action != "start" && !v.selected_destination_label.is_empty())
                    || v.selected_target_json.len() > 8192
                    || v.shortcut_state_json.len() > 16384
                {
                    return Err(SessionError::Invalid);
                }
                let scope = scope_from(v.scope.clone())?;
                if v.action == "start" && self.host {
                    return Err(SessionError::Authentication);
                }
                if v.action != "start"
                    && let retired::Disposition::Retired(_) = retired::disposition(&s, &scope)?
                {
                    retired::control_shape(&v, self.host)?;
                    s.received_control = v.sequence;
                    return Ok(());
                }
                s.received_control = v.sequence;
                if self.host {
                    let current = s.selected.as_ref().ok_or(SessionError::Invalid)?;
                    if scope != current.scope
                        || !v.selected_target_json.is_empty()
                        || !v.shortcut_state_json.is_empty()
                    {
                        return Err(SessionError::Invalid);
                    }
                    match v.action.as_str() {
                        "command_refused" => self.command_refused(&mut s, &v)?,
                        "request_control" => self.publish(&mut s, "pending_grant", None)?,
                        "keyframe" => {
                            #[cfg(windows)]
                            self.workers
                                .lock()
                                .map_err(|_| SessionError::Worker)?
                                .request_keyframe();
                        }
                        "background" | "pause" | "stop" => {
                            self.integration_release_cause(
                                &s,
                                &v.reason,
                                v.input_session_id.as_ref(),
                            );
                            s.host_retiring = true;
                            Self::retire(&mut s);
                            self.publish(&mut s, "paused", Some("peer_background"))?;
                            drop(s);
                            self.cancel_workers();
                            return Ok(());
                        }
                        _ => return Err(SessionError::Authentication),
                    }
                } else {
                    match v.action.as_str() {
                        "retired" => {
                            if s.selected.as_ref().is_none_or(|v| v.scope != scope) {
                                return Err(SessionError::Invalid);
                            }
                            s.peer_retiring = false;
                            self.publish(&mut s, "paused", None)?;
                        }
                        "authority" => self.receive_authority(&mut s, &v)?,
                        "command_request" => {
                            self.peer_command(
                                &mut s,
                                scope,
                                v.input_session_id,
                                v.request_nonce,
                                v.editor_action,
                            )?;
                        }
                        "start" => {
                            if v.selected_target_json.is_empty() {
                                return Err(SessionError::Invalid);
                            }
                            let target: Target = serde_json::from_slice(&v.selected_target_json)
                                .map_err(|_| SessionError::Invalid)?;
                            target.validate().map_err(failure)?;
                            if target.token != scope.target_token {
                                return Err(SessionError::Invalid);
                            }
                            let selected = Selected {
                                target,
                                scope,
                                display_label: v.selected_destination_label,
                            };
                            // Refused Start must not revoke the previous grant or
                            // adopt a source epoch before complete validation.
                            Self::validate_selected(&s, &selected)?;
                            Self::retire(&mut s);
                            Self::replace_selected(&mut s, selected)?;
                            s.start_recovery_pending = std::mem::take(&mut s.prestart_media_lost);
                            if s.start_recovery_pending {
                                s.awaiting_idr = true;
                            }
                            self.publish(&mut s, "viewing", None)?;
                        }
                        "grant" => {
                            if s.selected.as_ref().is_none_or(|v| v.scope != scope) {
                                return Err(SessionError::Invalid);
                            }
                            let binding = Binding {
                                scope,
                                input_session_id: Id::from_proto(v.input_session_id.as_ref())
                                    .map_err(|_| SessionError::Invalid)?
                                    .to_string(),
                            };
                            binding.validate().map_err(failure)?;
                            Self::retire_authority(&mut s);
                            s.binding = Some(binding);
                            s.input_seq = 0;
                            s.last_input = Instant::now();
                            s.authority = if v.shortcut_state_json.is_empty() {
                                None
                            } else {
                                Some(
                                    serde_json::from_slice(&v.shortcut_state_json)
                                        .map_err(|_| SessionError::Invalid)?,
                                )
                            };
                            self.publish(&mut s, "controlling", None)?;
                        }
                        "pause" | "revoke" | "stop" => {
                            if s.selected.as_ref().is_none_or(|v| v.scope != scope) {
                                return Err(SessionError::Invalid);
                            }
                            if s.binding.is_some() {
                                s.peer_retiring = true;
                            }
                            Self::retire_authority(&mut s);
                            self.publish(&mut s, "paused", Some("host_revoked"))?;
                        }
                        _ => return Err(SessionError::Invalid),
                    }
                }
            }
            Body::RemoteVideoConfig(v) => {
                self.stream.increment(5);
                if self.host {
                    return Err(SessionError::Authentication);
                }
                let config = config_from(v)?;
                if let retired::Disposition::Retired(index) =
                    retired::disposition(&s, &config.scope)?
                {
                    s.retired.config(index, config)?;
                    return Ok(());
                }
                if s.selected.as_ref().is_none_or(|v| {
                    v.scope != config.scope
                        || v.target.client_rect.width != config.visible_width
                        || v.target.client_rect.height != config.visible_height
                }) {
                    return Err(SessionError::Invalid);
                }
                if s.config.as_ref().is_some_and(|old| old != &config) {
                    return Err(SessionError::Invalid);
                }
                s.config = Some(config);
                let status = if s.binding.is_some() {
                    "controlling"
                } else {
                    "viewing"
                };
                self.publish(&mut s, status, None)?;
            }
            Body::VideoFrame(v) => {
                self.stream.increment(6);
                if self.host {
                    return Err(SessionError::Authentication);
                }
                let scope = scope_from(v.remote_scope.clone())?;
                let disposition = match retired::disposition(&s, &scope) {
                    Ok(value) => value,
                    Err(error) => {
                        if !retired::may_precede_start(&s, &scope)? {
                            return Err(error);
                        }
                        // Network ingress already bounded/validated this envelope.
                        // Independent MEDIA may overtake its CONTROL Start. Drop
                        // it without adopting its epoch, scope, payload or input.
                        s.prestart_media_lost = true;
                        self.stream.increment(7);
                        return Ok(());
                    }
                };
                let (selected, config) = match disposition {
                    retired::Disposition::Retired(index) => {
                        let (selected, config) = s.retired.frame_context(index)?;
                        if let Some(config) = config {
                            retired::frame(v, &selected, &config)?;
                        } else {
                            // Exact retired CONTROL scope supplies disposal identity
                            // even when its late config has not arrived. Never adopt
                            // this payload or infer config/input authority from it.
                            retired::unconfigured_frame(v, &selected)?;
                        }
                        self.stream.increment(8);
                        return Ok(());
                    }
                    retired::Disposition::Current => {
                        let Some(config) = &s.config else {
                            self.stream.increment(7);
                            drop(s);
                            self.recover()?;
                            return Ok(());
                        };
                        (
                            s.selected.clone().ok_or(SessionError::Invalid)?,
                            config.clone(),
                        )
                    }
                };
                let frame = retired::frame(v, &selected, &config)?;
                if frame.frame_id <= s.last_frame {
                    return Err(SessionError::Invalid);
                }
                if frame.input_session_id.as_ref().is_some_and(|id| {
                    s.binding
                        .as_ref()
                        .is_some_and(|b| &b.input_session_id == id)
                }) && frame.last_input_seq_applied > s.input_seq
                {
                    return Err(SessionError::Invalid);
                }
                match retain_frame(&mut s, frame)? {
                    FrameRetention::Accepted => self.stream.increment(9),
                    FrameRetention::AwaitingIdr => {}
                    FrameRetention::Overflow => {
                        drop(s);
                        self.recover()?;
                        return Ok(());
                    }
                }
            }
            Body::InputStatus(v) => {
                if self.host {
                    return Err(SessionError::Authentication);
                }
                self.command_result(&mut s, &v)?;
            }
            Body::InputEvent(v) => {
                if !self.host {
                    return Err(SessionError::Authentication);
                }
                let binding = s.binding.clone().ok_or(SessionError::Authentication)?;
                if scope_from(v.remote_scope.clone())? != binding.scope
                    || v.input_session_id != Some(native_id(&binding.input_session_id)?.to_proto())
                    || v.capture_session_id
                        != Some(native_id(&binding.scope.capture_session_id)?.to_proto())
                    || v.geometry_revision != binding.scope.geometry_revision
                {
                    return Err(SessionError::Authentication);
                }
                #[cfg(windows)]
                {
                    if s.admission.busy() {
                        return Err(SessionError::Authentication);
                    }
                    self.validate_command_request(&mut s, &binding, &v)?;
                    if let Some(pb::input_event::Event::Finite(f)) = &v.event
                        && f.kind == "shortcut"
                    {
                        s.admission.reserve(
                            v.input_seq,
                            f.editor_action,
                            v.request_nonce,
                            f.profile_digest.clone(),
                        )?;
                        self.publish(&mut s, "controlling", None)?;
                    }
                    self.workers
                        .lock()
                        .map_err(|_| SessionError::Worker)?
                        .input(binding, v)?;
                    s.last_input = Instant::now();
                }
                #[cfg(not(windows))]
                return Err(SessionError::Invalid);
            }
            _ => return Err(SessionError::Invalid),
        }
        Ok(())
    }
    fn take_frame(&self) -> SessionResult<Option<RemoteFrame>> {
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        Self::current(&s)?;
        let Some(ticket) = take_ready(&mut s) else {
            return Ok(None);
        };
        let config = s.config.as_ref().ok_or(SessionError::Invalid)?;
        let frame = s.held.get(&ticket).ok_or(SessionError::Invalid)?;
        self.stream.increment(10);
        Ok(Some(RemoteFrame {
            ticket,
            config_json: json(config)?,
            frame_id: frame.frame_id,
            pts_us: frame.pts_100ns / 10,
            keyframe: frame.keyframe,
            bytes: frame.annexb.clone(),
        }))
    }
    fn rendered(&self, ticket: u64, pts_us: i64) -> SessionResult<Option<RemoteAcknowledgment>> {
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        Self::current(&s)?;
        let scope = s
            .selected
            .as_ref()
            .ok_or(SessionError::Invalid)?
            .scope
            .clone();
        let binding = s.binding.clone();
        let admitted = s.input_seq;
        let acknowledgment =
            acknowledge_issued(&mut s, ticket, pts_us, &scope, binding.as_ref(), admitted)?;
        if let Some(ack) = &acknowledgment {
            s.admission.cover(ack.last_input_seq_applied);
        }
        Ok(acknowledgment)
    }
}
#[derive(Debug, PartialEq, Eq)]
enum FrameRetention {
    Accepted,
    AwaitingIdr,
    Overflow,
}
// Called only after exact current scope/config/frame and input-coverage checks.
// Total retained ownership remains four; one issued ticket plus ordered ready AUs.
fn retain_frame(s: &mut State, frame: Frame) -> SessionResult<FrameRetention> {
    s.last_frame = frame.frame_id;
    if s.awaiting_idr && !frame.keyframe {
        return Ok(FrameRetention::AwaitingIdr);
    }
    if s.held.len() >= 4 {
        discard_ready(s);
        return Ok(FrameRetention::Overflow);
    }
    s.next_ticket = s
        .next_ticket
        .checked_add(1)
        .ok_or(SessionError::Backpressure)?;
    let ticket = s.next_ticket;
    if frame.keyframe {
        s.awaiting_idr = false;
    }
    s.held.insert(ticket, frame);
    s.ready.push_back(ticket);
    Ok(FrameRetention::Accepted)
}
fn take_ready(s: &mut State) -> Option<u64> {
    // Every held ticket outside ready is already issued to the consumer. It
    // stays owned until actual render validation or explicit decoder retirement.
    if s.held.len() > s.ready.len() {
        return None;
    }
    s.ready.pop_front()
}
fn discard_ready(s: &mut State) {
    for ready in s.ready.drain(..) {
        s.held.remove(&ready);
    }
    // Losing any unissued AU breaks the reference chain. Issued tickets remain
    // owned, and future dependent AUs cannot be handed to the decoder as intact.
    s.awaiting_idr = true;
}
fn discard_issued(s: &mut State, ticket: u64) -> bool {
    if s.held.remove(&ticket).is_none() {
        return false;
    }
    s.ready.retain(|v| *v != ticket);
    discard_ready(s);
    true
}
fn acknowledge_issued(
    s: &mut State,
    ticket: u64,
    pts_us: i64,
    scope: &Scope,
    binding: Option<&Binding>,
    admitted: u64,
) -> SessionResult<Option<RemoteAcknowledgment>> {
    if s.ready.contains(&ticket) {
        return Err(SessionError::Invalid);
    }
    acknowledge_retained(&mut s.held, ticket, pts_us, scope, binding, admitted)
}
// Frame intake establishes codec/config/geometry provenance. This adoption
// gate consumes exactly one retained native ticket only after lookup validation.
fn acknowledge_retained(
    held: &mut BTreeMap<u64, Frame>,
    ticket: u64,
    pts_us: i64,
    scope: &Scope,
    binding: Option<&Binding>,
    admitted: u64,
) -> SessionResult<Option<RemoteAcknowledgment>> {
    let frame = held.get(&ticket).ok_or(SessionError::Invalid)?;
    if frame.pts_100ns / 10 != pts_us || &frame.scope != scope {
        return Err(SessionError::Invalid);
    }
    let result = if let Some(binding) = binding {
        if frame.input_session_id.as_ref() == Some(&binding.input_session_id) {
            if frame.last_input_seq_applied > admitted {
                return Err(SessionError::Invalid);
            }
            Some(RemoteAcknowledgment {
                binding_json: json(binding)?,
                frame_id: frame.frame_id,
                ticket,
                last_input_seq_applied: frame.last_input_seq_applied,
            })
        } else {
            None
        }
    } else {
        None
    };
    held.remove(&ticket);
    Ok(result)
}
#[uniffi::export]
impl LiveSession {
    pub fn remote_display(&self) -> SessionResult<RemoteDisplay> {
        Ok(self.remote.signal.borrow().clone())
    }
    pub async fn wait_remote_display(&self, after: u64) -> SessionResult<RemoteDisplay> {
        let mut signal = self.remote.signal.subscribe();
        if signal.borrow().sequence > after {
            return Ok(signal.borrow().clone());
        }
        self.runtime
            .call(async move {
                let _ = tokio::time::timeout(Duration::from_secs(1), signal.changed()).await;
                Ok(signal.borrow().clone())
            })
            .await
    }
    pub fn remote_stream_diagnostics(&self) -> SessionResult<String> {
        json(&self.remote.stream.snapshot())
    }
    pub fn remote_frame(&self) -> SessionResult<Option<RemoteFrame>> {
        self.remote.take_frame()
    }
    pub fn remote_discard_frame(&self, ticket: u64) -> SessionResult<()> {
        let mut s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
        if !discard_issued(&mut s, ticket) {
            return Ok(());
        }
        // A real discard releases backpressure but invalidates the codec chain.
        // Closed/stale cleanup does not require a live carrier or manufacture ACK.
        if RemoteHub::current(&s).is_ok() && s.selected.is_some() {
            self.remote.send_control(&mut s, "keyframe", "")?;
        }
        Ok(())
    }
    pub fn remote_rendered(
        &self,
        ticket: u64,
        pts_us: i64,
    ) -> SessionResult<Option<RemoteAcknowledgment>> {
        self.remote.rendered(ticket, pts_us)
    }
    pub fn remote_request_control(&self) -> SessionResult<()> {
        if self.remote.host {
            return Err(SessionError::Invalid);
        }
        let mut s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
        self.remote.send_control(&mut s, "request_control", "")
    }
    pub fn remote_keyframe(&self) -> SessionResult<()> {
        self.remote.recover()
    }
    pub fn remote_pause(&self, reason: String) -> SessionResult<()> {
        if !matches!(
            reason.as_str(),
            "owner_pause"
                | "background"
                | "surface_lost"
                | "decoder_failure"
                | "input_expired"
                | "connection_retired"
                | "current_state_unknown"
        ) {
            return Err(SessionError::Invalid);
        }
        self.remote.pause(&reason)
    }
    pub fn remote_retirement_ready(&self) -> SessionResult<bool> {
        let s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
        Ok(!s.host_retiring && !s.peer_retiring)
    }
    pub fn remote_close(&self) -> SessionResult<()> {
        self.remote.close_view()
    }
    pub fn remote_configure_runtime(&self, files: RemoteRuntimeFiles) -> SessionResult<()> {
        #[cfg(windows)]
        {
            self.remote.configure_runtime(files)
        }
        #[cfg(not(windows))]
        {
            let _ = files;
            Err(SessionError::RemoteUnavailable)
        }
    }
    pub fn remote_windows(&self) -> SessionResult<Vec<RemoteWindow>> {
        #[cfg(windows)]
        {
            host::windows()
        }
        #[cfg(not(windows))]
        {
            Err(SessionError::RemoteUnavailable)
        }
    }
    pub fn remote_select_window(
        &self,
        window: u64,
        process_id: u32,
        process_created: u64,
    ) -> SessionResult<()> {
        #[cfg(windows)]
        {
            self.remote
                .select_window(window, process_id, process_created)
        }
        #[cfg(not(windows))]
        {
            let _ = (window, process_id, process_created);
            Err(SessionError::RemoteUnavailable)
        }
    }
    pub fn remote_grant(&self) -> SessionResult<()> {
        #[cfg(windows)]
        {
            self.remote.grant()
        }
        #[cfg(not(windows))]
        {
            Err(SessionError::RemoteUnavailable)
        }
    }
    /// Reserve a bounded UI sample batch before it enters the phone queue.
    /// False means command refresh is busy; it does not revoke the pen grant.
    pub fn remote_reserve_pen(&self, binding_json: String, samples: u32) -> SessionResult<bool> {
        if self.remote.host || self.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        let binding: Binding = parse(&binding_json, 2048)?;
        binding.validate().map_err(failure)?;
        let mut s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
        RemoteHub::current(&s)?;
        if s.binding.as_ref() != Some(&binding) {
            return Err(SessionError::Authentication);
        }
        s.admission.queue(samples)
    }
    pub fn remote_pen(
        &self,
        binding_json: String,
        sample_json: String,
    ) -> SessionResult<RemoteInputAdmitted> {
        if self.remote.host || self.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        let binding: Binding = parse(&binding_json, 2048)?;
        let sample: vw_remote::wire::PenSample = parse(&sample_json, 1024)?;
        sample.validate().map_err(failure)?;
        let mut s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
        RemoteHub::current(&s)?;
        if s.binding.as_ref() != Some(&binding) {
            return Err(SessionError::Authentication);
        }
        s.admission.check_pen(sample.phase)?;
        let selected = s.selected.as_ref().ok_or(SessionError::Invalid)?;
        if !selected.target.client_rect.contains(sample.x, sample.y) {
            return Err(SessionError::Invalid);
        }
        let sequence = s
            .input_seq
            .checked_add(1)
            .ok_or(SessionError::Backpressure)?;
        let phase = match sample.phase {
            vw_remote::wire::Phase::Hover => pb::PenPhase::Hover,
            vw_remote::wire::Phase::Down => pb::PenPhase::Down,
            vw_remote::wire::Phase::Move => pb::PenPhase::Move,
            vw_remote::wire::Phase::Up => pb::PenPhase::Up,
            vw_remote::wire::Phase::Leave => pb::PenPhase::Cancel,
        };
        let event = pb::InputEvent {
            input_session_id: Some(native_id(&binding.input_session_id)?.to_proto()),
            capture_session_id: Some(native_id(&binding.scope.capture_session_id)?.to_proto()),
            geometry_revision: binding.scope.geometry_revision,
            input_seq: sequence,
            remote_scope: Some(scope_pb(&binding.scope)?),
            request_nonce: 0,
            event: Some(pb::input_event::Event::Pen(pb::PenEvent {
                phase: phase as i32,
                x: f64::from(sample.x - selected.target.client_rect.x),
                y: f64::from(sample.y - selected.target.client_rect.y),
                pressure: sample.pressure as f32 / 1024.0,
                tilt_x: sample.tilt_x as f32,
                tilt_y: sample.tilt_y as f32,
                rotation: sample.rotation as f32,
                barrel: sample.pen_flags & 1 != 0,
                eraser: sample.pen_flags & 4 != 0,
                t_ns: 0,
                native_pen_flags: Some(sample.pen_flags),
            })),
        };
        s.sender
            .as_ref()
            .ok_or(SessionError::Closed)?
            .enqueue(Body::InputEvent(event))?;
        s.admission.admit_pen(sample.phase, sequence);
        s.input_seq = sequence;
        s.last_input = Instant::now();
        Ok(RemoteInputAdmitted {
            binding_json: json(&binding)?,
            input_sequence: sequence,
        })
    }
}
#[uniffi::export]
pub fn remote_helpers_retirement_count() -> SessionResult<u32> {
    #[cfg(windows)]
    {
        u32::try_from(host::retirement_count()?).map_err(|_| SessionError::Backpressure)
    }
    #[cfg(not(windows))]
    {
        Ok(0)
    }
}

#[cfg(test)]
mod retained_tests {
    use super::*;
    fn scope() -> Scope {
        Scope {
            connection_epoch: 1,
            capture_session_id: "018bcfe5-6800-7000-8000-000000000001".into(),
            source_generation: 1,
            target_token: "018bcfe5-6800-7000-8000-000000000002".into(),
            geometry_revision: 1,
        }
    }
    fn binding() -> Binding {
        Binding {
            scope: scope(),
            input_session_id: "018bcfe5-6800-7000-8000-000000000003".into(),
        }
    }
    fn frame(id: u64) -> Frame {
        Frame {
            scope: scope(),
            config_generation: 1,
            frame_id: id,
            pts_100ns: 101,
            captured_qpc_100ns: 101,
            input_session_id: Some(binding().input_session_id),
            last_input_seq_applied: 7,
            keyframe: true,
            annexb: vec![0, 0, 1, 38, 1],
        }
    }
    fn close_fixture(host: bool) -> SessionResult<Arc<RemoteHub>> {
        let hub = RemoteHub::new(host);
        let rect = vw_remote::Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };
        {
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            s.epoch = 1;
            s.enabled = true;
            s.source_epoch = Some(1);
            s.selected = Some(Selected {
                target: Target {
                    token: scope().target_token,
                    window: 1,
                    process_id: 7,
                    thread_id: 8,
                    process_created: 9,
                    window_rect: rect,
                    frame_rect: rect,
                    client_rect: rect,
                    dpi: 96,
                    integrity: 0,
                },
                scope: scope(),
                display_label: "owned close fixture".into(),
            });
            s.binding = Some(binding());
        }
        Ok(hub)
    }
    #[test]
    fn deactivated_carrier_close_settles_before_discarding_impossible_notification()
    -> SessionResult<()> {
        let hub = close_fixture(true)?;
        hub.deactivate(1);
        hub.pause("owner_pause")?;
        {
            let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            assert!(s.host_retiring && s.selected.is_some());
            assert_eq!(s.epoch, 2);
            assert!(s.binding.is_none());
        }
        hub.finish_close_view(|| {
            let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            assert!(s.host_retiring && s.selected.is_some());
            Ok(())
        })?;
        let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        assert!(!s.host_retiring && !s.peer_retiring);
        assert!(s.selected.is_none() && s.binding.is_none() && s.authority.is_none());
        assert_eq!(s.retired.latest_generation(), 1);
        assert_eq!(s.display.status, "disconnected");
        assert_eq!(s.sent_control, 0);
        Ok(())
    }
    #[test]
    fn deactivated_carrier_cannot_clear_pending_or_failed_actual_workers() -> SessionResult<()> {
        let hub = close_fixture(true)?;
        hub.deactivate(1);
        hub.pause("owner_pause")?;
        for error in [
            SessionError::RemoteRetirementPending,
            SessionError::Worker,
            SessionError::Closed,
        ] {
            assert!(
                matches!(hub.finish_close_view(|| Err(error.clone())), Err(actual) if std::mem::discriminant(&actual) == std::mem::discriminant(&error))
            );
            let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            assert!(s.host_retiring && s.selected.is_some());
            assert_eq!(s.retired.latest_generation(), 0);
        }
        // Only the later actual successful settlement can release the selection.
        hub.finish_close_view(|| Ok(()))?;
        assert!(
            hub.state
                .lock()
                .map_err(|_| SessionError::Worker)?
                .selected
                .is_none()
        );
        Ok(())
    }
    #[test]
    fn closed_error_or_missing_sender_is_not_carrier_retirement_proof() -> SessionResult<()> {
        for (enabled, epoch) in [(true, 1), (false, 1), (true, 2)] {
            let hub = close_fixture(true)?;
            {
                let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
                s.enabled = enabled;
                s.epoch = epoch;
                s.host_retiring = true;
            }
            assert!(matches!(
                hub.finish_close_view(|| Ok(())),
                Err(SessionError::Closed)
            ));
            let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            assert!(s.host_retiring && s.selected.is_some());
        }
        Ok(())
    }
    #[test]
    fn active_peer_handshake_survives_settlement_and_stale_deactivation() -> SessionResult<()> {
        let hub = close_fixture(false)?;
        hub.state
            .lock()
            .map_err(|_| SessionError::Worker)?
            .peer_retiring = true;
        hub.deactivate(3); // A different lifetime cannot retire the current peer.
        assert!(matches!(
            hub.finish_close_view(|| Ok(())),
            Err(SessionError::RemoteRetirementPending)
        ));
        assert!(
            hub.state
                .lock()
                .map_err(|_| SessionError::Worker)?
                .selected
                .is_some()
        );
        hub.deactivate(1);
        hub.finish_close_view(|| Ok(()))?;
        assert!(
            hub.state
                .lock()
                .map_err(|_| SessionError::Worker)?
                .selected
                .is_none()
        );
        Ok(())
    }
    #[test]
    fn explicit_stop_close_is_retryable_only_after_actual_settlement() -> SessionResult<()> {
        let hub = close_fixture(true)?;
        hub.stop();
        hub.pause("owner_pause")?;
        assert!(matches!(
            hub.finish_close_view(|| Err(SessionError::RemoteRetirementPending)),
            Err(SessionError::RemoteRetirementPending)
        ));
        hub.finish_close_view(|| Ok(()))?;
        hub.pause("owner_pause")?;
        hub.finish_close_view(|| Ok(()))?;
        let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        assert!(s.closed && !s.enabled && s.sender.is_none());
        assert!(s.selected.is_none() && s.binding.is_none());
        assert_eq!(s.sent_control, 0);
        Ok(())
    }
    fn next_close_selection(s: &State) -> SessionResult<Selected> {
        let mut next = s.selected.clone().ok_or(SessionError::Invalid)?;
        next.scope.source_generation += 1;
        next.scope.capture_session_id = "018bcfe5-6800-7000-8000-000000000005".into();
        Ok(next)
    }
    #[test]
    fn stale_flush_hint_cannot_close_replacement_after_old_retirement_notice() -> SessionResult<()>
    {
        let hub = close_fixture(true)?;
        let mut notified = None;
        {
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            s.host_retiring = true;
            let next = next_close_selection(&s)?;
            // Model select_window's operation-locked state step only after its
            // real close_workers returned. The notification is still for A.
            RemoteHub::finish_retirement_notice(&mut s, |s| {
                notified = Some(RemoteHub::control_body(s, "retired", "")?);
                Ok(())
            })?;
            RemoteHub::retire(&mut s);
            RemoteHub::replace_selected(&mut s, next)?;
        }
        let Some(Body::RemoteControl(sent)) = notified else {
            return Err(SessionError::Invalid);
        };
        assert_eq!(sent.action, "retired");
        assert_eq!(scope_from(sent.scope)?, scope());
        // This is the exact production check performed AFTER acquiring the
        // operation lock, despite flush's earlier true hint for source A.
        let closes = std::cell::Cell::new(0);
        hub.flush_retiring_workers(|| {
            closes.set(closes.get() + 1);
            Ok(())
        })?;
        assert_eq!(closes.get(), 0);
        let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(
            s.selected.as_ref().map(|v| v.scope.source_generation),
            Some(2)
        );
        assert!(!s.host_retiring);
        Ok(())
    }
    #[test]
    fn failed_retirement_notice_cannot_clear_old_source_obligation() -> SessionResult<()> {
        let hub = close_fixture(true)?;
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        s.host_retiring = true;
        for error in [SessionError::Backpressure, SessionError::Closed] {
            assert!(matches!(
                RemoteHub::finish_retirement_notice(&mut s, |_| Err(error.clone())),
                Err(actual) if std::mem::discriminant(&actual) == std::mem::discriminant(&error)
            ));
            assert!(s.host_retiring);
            assert_eq!(s.selected.as_ref().map(|v| v.scope.clone()), Some(scope()));
        }
        Ok(())
    }
    #[test]
    fn flush_pending_workers_keep_exact_retirement_flag_and_source() -> SessionResult<()> {
        let hub = close_fixture(true)?;
        hub.state
            .lock()
            .map_err(|_| SessionError::Worker)?
            .host_retiring = true;
        let closes = std::cell::Cell::new(0);
        assert!(matches!(
            hub.flush_retiring_workers(|| {
                closes.set(closes.get() + 1);
                Err(SessionError::RemoteRetirementPending)
            }),
            Err(SessionError::RemoteRetirementPending)
        ));
        assert_eq!(closes.get(), 1);
        let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        assert!(s.host_retiring);
        assert_eq!(s.selected.as_ref().map(|v| v.scope.clone()), Some(scope()));
        Ok(())
    }
    #[test]
    fn late_old_background_is_disposed_without_rearming_new_video_retirement() -> SessionResult<()>
    {
        let hub = close_fixture(true)?;
        let old = {
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            let body = RemoteHub::control_body(&mut s, "background", "owner_pause")?;
            let next = next_close_selection(&s)?;
            RemoteHub::retire(&mut s);
            RemoteHub::replace_selected(&mut s, next)?;
            body
        };
        hub.receive_locked(old, hub.state.lock().map_err(|_| SessionError::Worker)?)?;
        let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        assert!(!s.host_retiring);
        assert_eq!(
            s.selected.as_ref().map(|v| v.scope.source_generation),
            Some(2)
        );
        assert!(s.binding.is_none());
        Ok(())
    }
    #[test]
    fn unknown_ticket_and_ns_vs_us_mismatch_leave_native_ledger_intact() -> SessionResult<()> {
        let mut held = BTreeMap::from([(8, frame(1)), (9, frame(2))]);
        assert!(acknowledge_retained(&mut held, 99, 10, &scope(), Some(&binding()), 7).is_err());
        assert!(acknowledge_retained(&mut held, 8, 101, &scope(), Some(&binding()), 7).is_err());
        assert_eq!(held.len(), 2);
        assert_eq!(held.get(&8).map(|v| v.frame_id), Some(1));
        Ok(())
    }
    #[test]
    fn exact_ticket_disambiguates_identical_rounded_microsecond_pts() -> SessionResult<()> {
        let mut second = frame(2);
        second.pts_100ns = 109;
        let mut held = BTreeMap::from([(8, frame(1)), (9, second)]);
        let ack = acknowledge_retained(&mut held, 8, 10, &scope(), Some(&binding()), 7)?
            .ok_or(SessionError::Invalid)?;
        assert_eq!(
            (ack.ticket, ack.frame_id, ack.last_input_seq_applied),
            (8, 1, 7)
        );
        assert!(held.contains_key(&9));
        assert!(acknowledge_retained(&mut held, 8, 10, &scope(), Some(&binding()), 7).is_err());
        Ok(())
    }
    #[test]
    fn scope_change_and_future_input_ack_cannot_consume_a_ticket() -> SessionResult<()> {
        let mut held = BTreeMap::from([(8, frame(1))]);
        let mut wrong = scope();
        wrong.geometry_revision += 1;
        assert!(acknowledge_retained(&mut held, 8, 10, &wrong, Some(&binding()), 7).is_err());
        assert!(acknowledge_retained(&mut held, 8, 10, &scope(), Some(&binding()), 6).is_err());
        assert_eq!(held.len(), 1);
        Ok(())
    }
    #[test]
    fn a_new_input_session_can_view_without_acknowledging_old_injection() -> SessionResult<()> {
        let mut held = BTreeMap::from([(8, frame(1))]);
        let mut current = binding();
        current.input_session_id = "018bcfe5-6800-7000-8000-000000000004".into();
        assert!(acknowledge_retained(&mut held, 8, 10, &scope(), Some(&current), 0)?.is_none());
        assert!(held.is_empty());
        Ok(())
    }
    #[test]
    fn source_recovery_retains_issued_tickets_and_removes_only_unissued_ready() -> SessionResult<()>
    {
        let hub = RemoteHub::new(false);
        let mut state = hub.state.lock().map_err(|_| SessionError::Worker)?;
        state.held.insert(8, frame(1));
        state.held.insert(9, frame(2));
        state.ready.push_back(9);
        // Same local operation used by recover(), without manufacturing a sender.
        discard_ready(&mut state);
        assert!(state.held.contains_key(&8));
        assert!(!state.held.contains_key(&9));
        assert!(state.ready.is_empty());
        Ok(())
    }
    #[test]
    fn queued_frames_wait_in_order_until_exact_render_callback() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        for id in 1..=3 {
            assert_eq!(retain_frame(&mut s, frame(id))?, FrameRetention::Accepted);
        }
        assert_eq!(take_ready(&mut s), Some(1));
        assert_eq!(take_ready(&mut s), None);
        acknowledge_issued(&mut s, 1, 10, &scope(), Some(&binding()), 7)?;
        assert_eq!(take_ready(&mut s), Some(2));
        assert_eq!(take_ready(&mut s), None);
        acknowledge_issued(&mut s, 2, 10, &scope(), Some(&binding()), 7)?;
        assert_eq!(take_ready(&mut s), Some(3));
        Ok(())
    }
    #[test]
    fn overflow_keeps_issued_ticket_and_requires_idr_after_real_loss() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        retain_frame(&mut s, frame(1))?;
        assert_eq!(take_ready(&mut s), Some(1));
        for id in 2..=4 {
            retain_frame(&mut s, frame(id))?;
        }
        assert_eq!(s.held.len(), 4);
        assert_eq!(retain_frame(&mut s, frame(5))?, FrameRetention::Overflow);
        assert_eq!(s.held.keys().copied().collect::<Vec<_>>(), vec![1]);
        assert!(s.ready.is_empty());
        assert!(s.awaiting_idr);
        let mut dependent = frame(6);
        dependent.keyframe = false;
        assert_eq!(
            retain_frame(&mut s, dependent)?,
            FrameRetention::AwaitingIdr
        );
        assert_eq!(retain_frame(&mut s, frame(7))?, FrameRetention::Accepted);
        assert!(!s.awaiting_idr);
        assert_eq!(take_ready(&mut s), None);
        acknowledge_issued(&mut s, 1, 10, &scope(), Some(&binding()), 7)?;
        assert_eq!(take_ready(&mut s), Some(5));
        Ok(())
    }
    #[test]
    fn unissued_or_stale_callback_cannot_release_backpressure() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        retain_frame(&mut s, frame(1))?;
        retain_frame(&mut s, frame(2))?;
        assert_eq!(take_ready(&mut s), Some(1));
        assert!(acknowledge_issued(&mut s, 2, 10, &scope(), Some(&binding()), 7).is_err());
        let mut stale = scope();
        stale.geometry_revision += 1;
        assert!(acknowledge_issued(&mut s, 1, 10, &stale, Some(&binding()), 7).is_err());
        assert_eq!(s.held.len(), 2);
        assert_eq!(take_ready(&mut s), None);
        Ok(())
    }
    #[test]
    fn actual_decoder_discard_releases_owner_but_not_reference_chain() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        retain_frame(&mut s, frame(1))?;
        retain_frame(&mut s, frame(2))?;
        assert_eq!(take_ready(&mut s), Some(1));
        assert!(!discard_issued(&mut s, 99));
        assert_eq!(s.held.len(), 2);
        assert!(discard_issued(&mut s, 1));
        assert!(s.held.is_empty());
        assert!(s.awaiting_idr);
        let mut dependent = frame(3);
        dependent.keyframe = false;
        assert_eq!(
            retain_frame(&mut s, dependent)?,
            FrameRetention::AwaitingIdr
        );
        retain_frame(&mut s, frame(4))?;
        assert_eq!(take_ready(&mut s), Some(3));
        Ok(())
    }
    #[test]
    fn recovery_without_callback_keeps_exact_issued_owner_until_discard() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        retain_frame(&mut s, frame(1))?;
        assert_eq!(take_ready(&mut s), Some(1));
        discard_ready(&mut s);
        retain_frame(&mut s, frame(2))?;
        assert_eq!(take_ready(&mut s), None);
        assert!(s.held.contains_key(&1));
        assert!(discard_issued(&mut s, 1));
        assert!(s.held.is_empty());
        retain_frame(&mut s, frame(3))?;
        assert_eq!(take_ready(&mut s), Some(3));
        Ok(())
    }
    #[test]
    fn same_scope_new_grant_drains_prior_video_without_old_input_ack() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        s.binding = Some(binding());
        retain_frame(&mut s, frame(1))?;
        retain_frame(&mut s, frame(2))?;
        assert_eq!(take_ready(&mut s), Some(1));
        let revision = s.grant_revision;
        RemoteHub::retire_authority(&mut s);
        assert!(s.binding.is_none());
        assert!(s.authority.is_none());
        assert!(s.applied.is_none());
        assert_eq!(s.grant_revision, revision + 1);
        let mut next = binding();
        next.input_session_id = "018bcfe5-6800-7000-8000-000000000004".into();
        s.binding = Some(next.clone());
        s.input_seq = 0;
        assert_eq!(take_ready(&mut s), None);
        assert!(acknowledge_issued(&mut s, 1, 10, &scope(), Some(&next), 0)?.is_none());
        assert_eq!(s.binding.as_ref(), Some(&next));
        assert_eq!(take_ready(&mut s), Some(2));
        Ok(())
    }
    #[test]
    fn same_scope_revoke_preserves_video_owner_but_cannot_regrant_input() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        s.binding = Some(binding());
        retain_frame(&mut s, frame(1))?;
        assert_eq!(take_ready(&mut s), Some(1));
        RemoteHub::retire_authority(&mut s);
        assert!(s.held.contains_key(&1));
        assert!(s.binding.is_none());
        assert!(acknowledge_issued(&mut s, 1, 10, &scope(), None, 0)?.is_none());
        assert!(s.held.is_empty());
        assert!(s.binding.is_none());
        assert!(s.authority.is_none());
        Ok(())
    }
    #[test]
    fn authority_retirement_preserves_codec_recovery_requirement() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        retain_frame(&mut s, frame(1))?;
        assert_eq!(take_ready(&mut s), Some(1));
        discard_ready(&mut s);
        RemoteHub::retire_authority(&mut s);
        assert!(s.awaiting_idr);
        assert!(s.held.contains_key(&1));
        let mut dependent = frame(2);
        dependent.keyframe = false;
        assert_eq!(
            retain_frame(&mut s, dependent)?,
            FrameRetention::AwaitingIdr
        );
        Ok(())
    }
    #[test]
    fn source_retirement_still_refuses_prior_scope_ticket() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        retain_frame(&mut s, frame(1))?;
        assert_eq!(take_ready(&mut s), Some(1));
        RemoteHub::retire(&mut s);
        assert!(s.held.is_empty());
        assert!(s.ready.is_empty());
        assert!(acknowledge_issued(&mut s, 1, 10, &scope(), None, 0).is_err());
        Ok(())
    }
    #[test]
    fn partial_and_refusal_results_survive_terminal_grant_retirement() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        let b = binding();
        s.binding = Some(b.clone());
        for (ticket, status) in [(1, "sealed_partial"), (2, "refused")] {
            s.commands.insert(
                ticket,
                PendingCommand {
                    binding: b.clone(),
                    action: 1,
                    input_seq: 7,
                    request_nonce: ticket,
                    refresh_done: false,
                    result: Some(RemoteCommandResult {
                        binding_json: json(&b)?,
                        request_ticket: ticket,
                        input_sequence: 0,
                        action: 1,
                        accepted_qpc_100ns: 0,
                        status: status.into(),
                        reason: Some("partial_input".into()),
                    }),
                },
            );
        }
        RemoteHub::retire(&mut s);
        assert_eq!(
            s.commands
                .get(&1)
                .and_then(|v| v.result.as_ref())
                .map(|r| r.status.as_str()),
            Some("sealed_partial")
        );
        assert_eq!(
            s.commands
                .get(&2)
                .and_then(|v| v.result.as_ref())
                .map(|r| r.status.as_str()),
            Some("refused")
        );
        assert!(s.binding.is_none());
        Ok(())
    }

    #[test]
    fn command_receipt_acceptance_is_separate_from_exact_issued_correlation() -> SessionResult<()> {
        for (status, qpc, reason) in [
            ("injected", 101, ""),
            ("refused", 0, "native_refused"),
            ("sealed_partial", 0, "partial_input"),
        ] {
            let hub = RemoteHub::new(false);
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            let b = binding();
            s.binding = Some(b.clone());
            s.admission.reserve(7, 1, 0, "a".repeat(64))?;
            s.commands.insert(
                1,
                PendingCommand {
                    binding: b.clone(),
                    action: 1,
                    input_seq: 7,
                    request_nonce: 0,
                    result: None,
                    refresh_done: false,
                },
            );
            let mut receipt = pb::InputStatus {
                input_session_id: Some(native_id(&b.input_session_id)?.to_proto()),
                state: status.into(),
                reason: reason.into(),
                remote_scope: Some(scope_pb(&b.scope)?),
                input_seq: 7,
                accepted_qpc_100ns: qpc,
                editor_action: 1,
                request_nonce: 0,
            };
            receipt.input_seq = 8;
            // A wrong issued sequence cannot find/complete the retained pending request.
            if status == "injected" {
                assert!(hub.command_result(&mut s, &receipt).is_err());
            } else {
                hub.command_result(&mut s, &receipt)?;
            }
            assert!(s.commands.get(&1).is_some_and(|v| v.result.is_none()));
            receipt.input_seq = 7;
            hub.command_result(&mut s, &receipt)?;
            let result = s
                .commands
                .get(&1)
                .and_then(|v| v.result.as_ref())
                .ok_or(SessionError::Invalid)?;
            assert_eq!(
                result.input_sequence,
                if status == "injected" { 7 } else { 0 }
            );
            assert_eq!(result.accepted_qpc_100ns, qpc);
            assert_eq!(result.request_ticket, 1);
            assert_eq!(result.status, status);
        }
        Ok(())
    }
}
