//! Finite command receipts originate in actual guarded helper injection. Both
//! palettes share the phone-native admission sequence; PC requests carry no seq.
use super::*;
#[derive(Clone, Debug, uniffi::Record)]
pub struct RemoteCommandResult {
    pub binding_json: String,
    pub request_ticket: u64,
    /// Actual accepted input only; issued correlation stays in PendingCommand/InputStatus.
    pub input_sequence: u64,
    pub action: u32,
    pub accepted_qpc_100ns: u64,
    pub status: String,
    pub reason: Option<String>,
}
pub(super) struct PendingCommand {
    pub binding: Binding,
    pub action: u32,
    pub input_seq: u64,
    pub request_nonce: u64,
    pub result: Option<RemoteCommandResult>,
    pub refresh_done: bool,
}
// Distinct from the finite command barrier. A newer admitted stroke or finite
// action invalidates pending read-only results without refusing pen samples.
#[derive(Default)]
pub(super) struct StrokeObservation {
    contact: bool,
    completed_up: u64,
    published_up: u64,
    pending: Option<(u64, vw_remote::profile::Authority)>,
}
impl StrokeObservation {
    pub(super) fn invalidate(&mut self) {
        self.completed_up = 0;
        self.pending = None;
    }
    pub(super) fn pen(&mut self, phase: vw_remote::wire::Phase, sequence: u64) {
        use vw_remote::wire::Phase;
        match phase {
            Phase::Down => {
                self.invalidate();
                self.contact = true;
            }
            Phase::Up => {
                if self.contact {
                    self.completed_up = sequence;
                }
                self.contact = false;
            }
            Phase::Leave => {
                self.invalidate();
                self.contact = false;
            }
            Phase::Move | Phase::Hover => {}
        }
    }
    #[cfg(windows)]
    pub(super) fn event(&mut self, event: &pb::InputEvent) -> SessionResult<()> {
        if let Some(pb::input_event::Event::Pen(pen)) = &event.event {
            let phase =
                match pb::PenPhase::try_from(pen.phase).map_err(|_| SessionError::Invalid)? {
                    pb::PenPhase::Down => vw_remote::wire::Phase::Down,
                    pb::PenPhase::Move => vw_remote::wire::Phase::Move,
                    pb::PenPhase::Up => vw_remote::wire::Phase::Up,
                    pb::PenPhase::Hover => vw_remote::wire::Phase::Hover,
                    pb::PenPhase::Cancel => vw_remote::wire::Phase::Leave,
                    _ => return Err(SessionError::Invalid),
                };
            self.pen(phase, event.input_seq);
        } else {
            self.invalidate();
        }
        Ok(())
    }
    fn current(&self, sequence: u64) -> bool {
        sequence != 0
            && sequence == self.completed_up
            && sequence > self.published_up
            && !self.contact
    }
}
fn action_allowed<'a>(
    s: &'a State,
    binding: &Binding,
    action: u32,
) -> SessionResult<&'a vw_remote::profile::Authority> {
    if s.binding.as_ref() != Some(binding) || !(1..=16).contains(&action) {
        return Err(SessionError::Authentication);
    }
    let authority = s.authority.as_ref().ok_or(SessionError::Authentication)?;
    let selected = s.selected.as_ref().ok_or(SessionError::Authentication)?;
    authority
        .canvas_for_target(&selected.target)
        .map_err(failure)?;
    if selected.scope != binding.scope || !authority.verified_actions.contains(&action) {
        return Err(SessionError::Authentication);
    }
    Ok(authority)
}
impl RemoteHub {
    fn enqueue_command(
        &self,
        s: &mut State,
        binding: &Binding,
        action: u32,
        request_nonce: u64,
    ) -> SessionResult<u64> {
        Self::current(s)?;
        if !s.admission.can_reserve() {
            return Err(SessionError::Backpressure);
        }
        let authority = action_allowed(s, binding, action)?;
        let finite = pb::RemoteFiniteInput {
            kind: "shortcut".into(),
            editor_action: action,
            profile_digest: authority.profile_digest.clone(),
            focus_runtime_hash: authority.canvas().map_err(failure)?.runtime_id_hash.clone(),
            ..Default::default()
        };
        let sequence = s
            .input_seq
            .checked_add(1)
            .ok_or(SessionError::Backpressure)?;
        let digest = finite.profile_digest.clone();
        let event = pb::InputEvent {
            input_session_id: Some(native_id(&binding.input_session_id)?.to_proto()),
            capture_session_id: Some(native_id(&binding.scope.capture_session_id)?.to_proto()),
            geometry_revision: binding.scope.geometry_revision,
            input_seq: sequence,
            remote_scope: Some(scope_pb(&binding.scope)?),
            request_nonce,
            event: Some(pb::input_event::Event::Finite(finite)),
        };
        s.sender
            .as_ref()
            .ok_or(SessionError::Closed)?
            .enqueue(Body::InputEvent(event))?;
        s.admission
            .reserve(sequence, action, request_nonce, digest)?;
        s.stroke.invalidate();
        self.publish(s, "controlling", None)?;
        s.input_seq = sequence;
        s.last_input = Instant::now();
        Ok(sequence)
    }
    pub(super) fn peer_command(
        &self,
        s: &mut State,
        scope: Scope,
        input_id: Option<pb::Uuid>,
        nonce: u64,
        action: u32,
    ) -> SessionResult<()> {
        if self.host || nonce == 0 {
            return Err(SessionError::Invalid);
        }
        let binding = s.binding.clone().ok_or(SessionError::Authentication)?;
        if binding.scope != scope
            || input_id != Some(native_id(&binding.input_session_id)?.to_proto())
        {
            return Err(SessionError::Authentication);
        }
        if !s.admission.can_reserve() {
            return self.send_command_control(
                s,
                &binding,
                "command_refused",
                (0, action, nonce),
                &None,
                "input_busy",
            );
        }
        self.enqueue_command(s, &binding, action, nonce)?;
        Ok(())
    }
    #[cfg(windows)]
    pub(super) fn validate_command_request(
        &self,
        s: &mut State,
        binding: &Binding,
        event: &pb::InputEvent,
    ) -> SessionResult<()> {
        if event.request_nonce == 0 {
            return Ok(());
        }
        let Some(pb::input_event::Event::Finite(finite)) = &event.event else {
            return Err(SessionError::Invalid);
        };
        let pending = s
            .commands
            .get_mut(&event.request_nonce)
            .ok_or(SessionError::Authentication)?;
        if pending.binding != *binding
            || pending.action != finite.editor_action
            || pending.input_seq != 0
            || pending.result.is_some()
        {
            return Err(SessionError::Authentication);
        }
        pending.input_seq = event.input_seq;
        Ok(())
    }
    pub(super) fn command_result(&self, s: &mut State, v: &pb::InputStatus) -> SessionResult<()> {
        let binding = Binding {
            scope: scope_from(v.remote_scope.clone())?,
            input_session_id: Id::from_proto(v.input_session_id.as_ref())
                .map_err(|_| SessionError::Invalid)?
                .to_string(),
        };
        binding.validate().map_err(failure)?;
        if v.input_seq == 0
            || !(1..=16).contains(&v.editor_action)
            || !vw_net::remote_reason_allowed(&v.reason)
            || !matches!(v.state.as_str(), "injected" | "refused" | "sealed_partial")
            || (v.state == "injected") != (v.accepted_qpc_100ns > 0)
        {
            return Err(SessionError::Invalid);
        }
        if s.binding.as_ref() != Some(&binding) {
            return Err(SessionError::Authentication);
        }
        if v.state == "injected" {
            let command = s
                .admission
                .exact(v.input_seq, v.editor_action, v.request_nonce)?;
            if command.injected {
                return Err(SessionError::Invalid);
            }
            command.injected = true;
        }
        let key = if self.host {
            (v.request_nonce != 0).then_some(v.request_nonce)
        } else if v.request_nonce == 0 {
            s.commands.iter().find_map(|(key, c)| {
                (c.input_seq == v.input_seq && c.request_nonce == 0).then_some(*key)
            })
        } else {
            None
        };
        if let Some(key) = key {
            let pending = s
                .commands
                .get_mut(&key)
                .ok_or(SessionError::Authentication)?;
            if pending.binding != binding
                || pending.action != v.editor_action
                || pending.input_seq != v.input_seq
                || pending.request_nonce != v.request_nonce
                || pending.result.is_some()
            {
                return Err(SessionError::Authentication);
            }
            pending.result = Some(RemoteCommandResult {
                binding_json: json(&binding)?,
                request_ticket: key,
                input_sequence: if v.state == "injected" {
                    v.input_seq
                } else {
                    0
                },
                action: v.editor_action,
                accepted_qpc_100ns: v.accepted_qpc_100ns,
                status: v.state.clone(),
                reason: (!v.reason.is_empty()).then(|| v.reason.clone()),
            });
        }
        if s.admission.finish() {
            self.publish(s, "controlling", None)?;
        }
        Ok(())
    }
    #[cfg(windows)]
    pub(super) fn injected_command(
        &self,
        binding: &Binding,
        event: &pb::InputEvent,
        result: &std::result::Result<u64, vw_remote::Error>,
    ) -> SessionResult<()> {
        let Some(pb::input_event::Event::Finite(finite)) = &event.event else {
            return Ok(());
        };
        if finite.kind != "shortcut" {
            return Ok(());
        }
        let (state, qpc, reason) = match result {
            Ok(qpc) => ("injected", *qpc, ""),
            Err(vw_remote::Error::PartialInput) => ("sealed_partial", 0, "partial_input"),
            Err(_) => ("refused", 0, "native_refused"),
        };
        let v = pb::InputStatus {
            input_session_id: Some(native_id(&binding.input_session_id)?.to_proto()),
            state: state.into(),
            reason: reason.into(),
            remote_scope: Some(scope_pb(&binding.scope)?),
            input_seq: event.input_seq,
            accepted_qpc_100ns: qpc,
            editor_action: finite.editor_action,
            request_nonce: event.request_nonce,
        };
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        if s.binding.as_ref() != Some(binding) {
            return Err(SessionError::Closed);
        }
        self.command_result(&mut s, &v)?;
        s.sender
            .as_ref()
            .ok_or(SessionError::Closed)?
            .enqueue(Body::InputStatus(v))?;
        Ok(())
    }
}
impl RemoteHub {
    fn send_command_control(
        &self,
        s: &mut State,
        binding: &Binding,
        action: &str,
        correlation: (u64, u32, u64),
        authority: &Option<vw_remote::profile::Authority>,
        reason: &str,
    ) -> SessionResult<()> {
        let (input_seq, editor_action, nonce) = correlation;
        Self::current(s)?;
        if s.binding.as_ref() != Some(binding) {
            return Err(SessionError::Authentication);
        }
        s.sent_control = s
            .sent_control
            .checked_add(1)
            .ok_or(SessionError::Backpressure)?;
        let v = pb::RemoteControl {
            scope: Some(scope_pb(&binding.scope)?),
            sequence: s.sent_control,
            action: action.into(),
            input_session_id: Some(native_id(&binding.input_session_id)?.to_proto()),
            editor_action,
            request_nonce: nonce,
            command_input_seq: input_seq,
            reason: reason.into(),
            shortcut_state_json: authority
                .as_ref()
                .map(json)
                .transpose()?
                .unwrap_or_default()
                .into_bytes(),
            ..Default::default()
        };
        s.sender
            .as_ref()
            .ok_or(SessionError::Closed)?
            .enqueue(Body::RemoteControl(v))?;
        Ok(())
    }
    pub(super) fn command_refused(
        &self,
        s: &mut State,
        v: &pb::RemoteControl,
    ) -> SessionResult<()> {
        if !self.host || v.reason != "input_busy" || v.command_input_seq != 0 {
            return Err(SessionError::Invalid);
        }
        let pending = s
            .commands
            .get_mut(&v.request_nonce)
            .ok_or(SessionError::Authentication)?;
        if pending.binding.scope != scope_from(v.scope.clone())?
            || v.input_session_id != Some(native_id(&pending.binding.input_session_id)?.to_proto())
            || pending.action != v.editor_action
            || pending.input_seq != 0
            || pending.result.is_some()
        {
            return Err(SessionError::Authentication);
        }
        pending.result = Some(RemoteCommandResult {
            binding_json: json(&pending.binding)?,
            request_ticket: v.request_nonce,
            input_sequence: 0,
            action: pending.action,
            accepted_qpc_100ns: 0,
            status: "refused".into(),
            reason: Some("input_busy".into()),
        });
        pending.refresh_done = true;
        Ok(())
    }
    fn validate_stroke_authority(
        s: &State,
        binding: &Binding,
        next: &vw_remote::profile::Authority,
    ) -> SessionResult<()> {
        if s.binding.as_ref() != Some(binding) {
            return Err(SessionError::Authentication);
        }
        let old = s.authority.as_ref().ok_or(SessionError::Authentication)?;
        let selected = s.selected.as_ref().ok_or(SessionError::Authentication)?;
        let canvas = next.canvas_for_target(&selected.target).map_err(failure)?;
        let previous = old.canvas_for_target(&selected.target).map_err(failure)?;
        if canvas.runtime_id_hash != previous.runtime_id_hash
            || canvas.process_id != previous.process_id
            || canvas.class_name != previous.class_name
            || canvas.control_type != previous.control_type
            || canvas.canvas_rect != previous.canvas_rect
            || selected.scope != binding.scope
            || !vw_remote::profile::essential::Editor::Paint.matches_identity(&old.identity)
            || old.routes != vw_remote::profile::AuthorityRoutes::Essential
            || next.profile_digest != old.profile_digest
            || canvas.profile_digest != old.profile_digest
            || next.identity != old.identity
            || next.routes != old.routes
        {
            return Err(SessionError::Authentication);
        }
        Ok(())
    }
    pub(super) fn apply_pending_stroke_authority(&self, s: &mut State) -> SessionResult<()> {
        if !s.admission.authority_idle() {
            return Ok(());
        }
        if let Some((sequence, authority)) = s.stroke.pending.take() {
            if !s.stroke.current(sequence) || s.authority.is_none() {
                return Ok(());
            }
            let binding = s.binding.clone().ok_or(SessionError::Authentication)?;
            Self::validate_stroke_authority(s, &binding, &authority)?;
            s.authority = Some(authority);
            s.stroke.published_up = sequence;
            // No command receipt, barrier completion, grant or input ACK is made.
            self.publish(s, "controlling", None)?;
        }
        Ok(())
    }
    pub(super) fn receive_stroke_authority(
        &self,
        s: &mut State,
        v: &pb::RemoteControl,
    ) -> SessionResult<()> {
        if self.host
            || v.command_input_seq == 0
            || v.editor_action != 0
            || v.request_nonce != 0
            || !v.reason.is_empty()
            || !v.selected_target_json.is_empty()
            || !v.selected_destination_label.is_empty()
            || v.shortcut_state_json.is_empty()
        {
            return Err(SessionError::Invalid);
        }
        let binding = Binding {
            scope: scope_from(v.scope.clone())?,
            input_session_id: Id::from_proto(v.input_session_id.as_ref())
                .map_err(|_| SessionError::Invalid)?
                .to_string(),
        };
        // Authenticate the envelope before disposal, but do not validate an old
        // result against authority removed by a newer finite refresh/revocation.
        if s.binding.as_ref() != Some(&binding) || v.command_input_seq > s.input_seq {
            return Err(SessionError::Authentication);
        }
        let authority =
            serde_json::from_slice(&v.shortcut_state_json).map_err(|_| SessionError::Invalid)?;
        if !s.stroke.current(v.command_input_seq) || s.authority.is_none() {
            return Ok(());
        }
        Self::validate_stroke_authority(s, &binding, &authority)?;
        s.stroke.pending = Some((v.command_input_seq, authority));
        self.apply_pending_stroke_authority(s)
    }
    #[cfg(windows)]
    pub(super) fn publish_stroke_authority(
        &self,
        binding: &Binding,
        sequence: u64,
        authority: vw_remote::profile::Authority,
    ) -> SessionResult<()> {
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        // This is an asynchronous local observation, not a peer command. A
        // replaced/retired binding or newer input silently owns the state now.
        if s.binding.as_ref() != Some(binding)
            || !s.stroke.current(sequence)
            || s.admission.busy()
            || s.authority.is_none()
        {
            return Ok(());
        }
        Self::current(&s)?;
        Self::validate_stroke_authority(&s, binding, &authority)?;
        self.send_command_control(
            &mut s,
            binding,
            "stroke_authority",
            (sequence, 0, 0),
            &Some(authority.clone()),
            "",
        )?;
        s.authority = Some(authority);
        s.stroke.published_up = sequence;
        self.publish(&mut s, "controlling", None)
    }
    fn adopt_authority(
        &self,
        s: &mut State,
        binding: &Binding,
        correlation: (u64, u32, u64),
        authority: Option<vw_remote::profile::Authority>,
    ) -> SessionResult<()> {
        let (input_seq, action, nonce) = correlation;
        if s.binding.as_ref() != Some(binding) {
            return Err(SessionError::Authentication);
        }
        let command = s.admission.exact(input_seq, action, nonce)?.clone();
        if command.refreshed {
            return Err(SessionError::Invalid);
        }
        if let Some(next) = &authority {
            let old = s.authority.as_ref().ok_or(SessionError::Authentication)?;
            let target = &s
                .selected
                .as_ref()
                .ok_or(SessionError::Authentication)?
                .target;
            let canvas = next.canvas_for_target(target).map_err(failure)?;
            if next.profile_digest != command.digest
                || canvas.profile_digest != command.digest
                || next.identity != old.identity
                || next.routes != old.routes
            {
                return Err(SessionError::Authentication);
            }
        }
        s.authority = authority;
        s.admission.exact(input_seq, action, nonce)?.refreshed = true;
        for pending in s.commands.values_mut() {
            if pending.binding == *binding
                && pending.input_seq == input_seq
                && pending.action == action
                && pending.request_nonce == nonce
            {
                pending.refresh_done = true;
            }
        }
        s.admission.finish();
        self.publish(s, "controlling", None)
    }
    pub(super) fn receive_authority(
        &self,
        s: &mut State,
        v: &pb::RemoteControl,
    ) -> SessionResult<()> {
        if self.host
            || v.command_input_seq == 0
            || !v.reason.is_empty()
            || !v.selected_target_json.is_empty()
        {
            return Err(SessionError::Invalid);
        }
        let binding = Binding {
            scope: scope_from(v.scope.clone())?,
            input_session_id: Id::from_proto(v.input_session_id.as_ref())
                .map_err(|_| SessionError::Invalid)?
                .to_string(),
        };
        let authority = if v.shortcut_state_json.is_empty() {
            None
        } else {
            Some(
                serde_json::from_slice(&v.shortcut_state_json)
                    .map_err(|_| SessionError::Invalid)?,
            )
        };
        self.adopt_authority(
            s,
            &binding,
            (v.command_input_seq, v.editor_action, v.request_nonce),
            authority,
        )
    }
    #[cfg(windows)]
    pub(super) fn publish_authority(
        &self,
        binding: &Binding,
        input_seq: u64,
        action: u32,
        nonce: u64,
        authority: Option<vw_remote::profile::Authority>,
    ) -> SessionResult<()> {
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        Self::current(&s)?;
        let command = s.admission.exact(input_seq, action, nonce)?;
        if !command.injected {
            return Err(SessionError::Authentication);
        }
        // Send exact-current authority on CONTROL; peer keeps reservation until
        // it also sees the INPUT-channel injection receipt, in either order.
        self.send_command_control(
            &mut s,
            binding,
            "authority",
            (input_seq, action, nonce),
            &authority,
            "",
        )?;
        self.adopt_authority(&mut s, binding, (input_seq, action, nonce), authority)
    }
}
#[uniffi::export]
impl LiveSession {
    /// Returns a retained request ticket. Only take_remote_command may return an
    /// actual injection receipt; successful admission alone is never Injected.
    pub fn remote_begin_command(&self, binding_json: String, action: u32) -> SessionResult<u64> {
        let binding: Binding = parse(&binding_json, 2048)?;
        binding.validate().map_err(failure)?;
        let mut s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
        RemoteHub::current(&s)?;
        action_allowed(&s, &binding, action)?;
        if s.commands.len() >= 4 {
            return Err(SessionError::Backpressure);
        }
        s.next_command = s
            .next_command
            .checked_add(1)
            .ok_or(SessionError::Backpressure)?;
        let ticket = s.next_command;
        let (input_seq, nonce) = if self.remote.host {
            s.sent_control = s
                .sent_control
                .checked_add(1)
                .ok_or(SessionError::Backpressure)?;
            let body = pb::RemoteControl {
                scope: Some(scope_pb(&binding.scope)?),
                sequence: s.sent_control,
                action: "command_request".into(),
                input_session_id: Some(native_id(&binding.input_session_id)?.to_proto()),
                request_nonce: ticket,
                editor_action: action,
                selected_destination_label: String::new(),
                ..Default::default()
            };
            s.sender
                .as_ref()
                .ok_or(SessionError::Closed)?
                .enqueue(Body::RemoteControl(body))?;
            (0, ticket)
        } else {
            (self.remote.enqueue_command(&mut s, &binding, action, 0)?, 0)
        };
        s.commands.insert(
            ticket,
            PendingCommand {
                binding,
                action,
                input_seq,
                request_nonce: nonce,
                result: None,
                refresh_done: false,
            },
        );
        Ok(ticket)
    }
    pub fn remote_take_command(&self, ticket: u64) -> SessionResult<Option<RemoteCommandResult>> {
        let mut s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
        let pending = s.commands.get_mut(&ticket).ok_or(SessionError::Invalid)?;
        let result = if pending.refresh_done {
            pending.result.take()
        } else {
            None
        };
        if result.is_some() {
            s.commands.remove(&ticket);
        }
        Ok(result)
    }
    pub fn remote_cancel_command(&self, ticket: u64) -> SessionResult<()> {
        {
            let mut s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
            s.commands.remove(&ticket).ok_or(SessionError::Invalid)?;
        }
        // A blocking OS batch is not hard-interruptible. Seal the whole grant;
        // helper cancellation/retirement still retains its actual owner.
        self.remote.pause("command_timeout")
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    fn authority(selected: bool) -> vw_remote::profile::Authority {
        use vw_remote::profile::{Authority, AuthorityRoutes, CanvasProof, ImageIdentity};
        let canvas = CanvasProof {
            runtime_id_hash: "b".repeat(64),
            process_id: 7,
            sampled_qpc_100ns: 101,
            class_name: "KisOpenGLCanvas2".into(),
            control_type: 50026,
            canvas_rect: vw_remote::Rect {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            profile_digest: "a".repeat(64),
        };
        Authority {
            identity: ImageIdentity {
                executable_name: "krita.exe".into(),
                executable_blake3: "c".repeat(64),
                executable_bytes: 1024,
                file_version: Some("5.3.4.0".into()),
                package_full_name: None,
                package_version: None,
            },
            profile_digest: "a".repeat(64),
            verified_actions: vec![1, 2, 3, 6, 7, 8, 9],
            routes: AuthorityRoutes::Essential,
            focus: (!selected).then(|| canvas.clone()),
            selected_canvas: selected.then_some(canvas),
        }
    }
    fn observation_fixture() -> SessionResult<(Arc<RemoteHub>, Binding)> {
        let hub = RemoteHub::new(false);
        let b = binding();
        {
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            let rect = vw_remote::Rect {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            };
            s.selected = Some(Selected {
                target: Target {
                    token: b.scope.target_token.clone(),
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
                scope: b.scope.clone(),
                display_label: "owned authority fixture".into(),
            });
            s.binding = Some(b.clone());
            s.authority = Some(authority(false));
            s.admission.reserve(7, 1, 5, "a".repeat(64))?;
        }
        Ok((hub, b))
    }
    #[test]
    fn selected_canvas_refresh_keeps_next_essential_command_in_both_channel_orders()
    -> SessionResult<()> {
        for authority_first in [false, true] {
            let (hub, b) = observation_fixture()?;
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            if authority_first {
                hub.adopt_authority(&mut s, &b, (7, 1, 5), Some(authority(true)))?;
            } else {
                hub.command_result(&mut s, &injected(&b)?)?;
            }
            assert!(s.admission.busy());
            if authority_first {
                hub.command_result(&mut s, &injected(&b)?)?;
            } else {
                hub.adopt_authority(&mut s, &b, (7, 1, 5), Some(authority(true)))?;
            }
            assert!(!s.admission.busy());
            assert_eq!(s.binding.as_ref(), Some(&b));
            let next = action_allowed(&s, &b, 2)?;
            assert!(next.focus.is_none() && next.selected_canvas.is_some());
            assert!(s.admission.can_reserve());
        }
        Ok(())
    }
    #[test]
    fn invalid_selected_refresh_preserves_current_authority_and_reservation() -> SessionResult<()> {
        for mode in 0..7 {
            let (hub, b) = observation_fixture()?;
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            let mut next = authority(true);
            match mode {
                0 => next.focus = next.selected_canvas.clone(),
                1 => next.selected_canvas = None,
                2 => next.identity.executable_blake3 = "d".repeat(64),
                3 => {
                    next.selected_canvas
                        .as_mut()
                        .ok_or(SessionError::Invalid)?
                        .process_id = 8
                }
                4 => {
                    next.selected_canvas
                        .as_mut()
                        .ok_or(SessionError::Invalid)?
                        .canvas_rect
                        .x = 1
                }
                5 => next.profile_digest = "d".repeat(64),
                _ => next.routes = vw_remote::profile::AuthorityRoutes::Legacy,
            }
            assert!(
                hub.adopt_authority(&mut s, &b, (7, 1, 5), Some(next))
                    .is_err()
            );
            assert!(s.admission.busy());
            assert!(s.authority.as_ref().is_some_and(|a| a.focus.is_some()));
            assert_eq!(s.binding.as_ref(), Some(&b));
        }
        Ok(())
    }
    #[test]
    fn focused_refresh_cannot_change_installed_route_family() -> SessionResult<()> {
        let (hub, b) = observation_fixture()?;
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        let mut next = authority(false);
        next.routes = vw_remote::profile::AuthorityRoutes::Legacy;
        assert!(
            hub.adopt_authority(&mut s, &b, (7, 1, 5), Some(next))
                .is_err()
        );
        assert!(s.admission.busy());
        Ok(())
    }
    fn binding() -> Binding {
        Binding {
            scope: Scope {
                connection_epoch: 1,
                capture_session_id: "018bcfe5-6800-7000-8000-000000000001".into(),
                source_generation: 1,
                target_token: "018bcfe5-6800-7000-8000-000000000002".into(),
                geometry_revision: 1,
            },
            input_session_id: "018bcfe5-6800-7000-8000-000000000003".into(),
        }
    }
    fn injected(b: &Binding) -> SessionResult<pb::InputStatus> {
        Ok(pb::InputStatus {
            input_session_id: Some(native_id(&b.input_session_id)?.to_proto()),
            state: "injected".into(),
            reason: String::new(),
            remote_scope: Some(scope_pb(&b.scope)?),
            input_seq: 7,
            accepted_qpc_100ns: 101,
            editor_action: 1,
            request_nonce: 5,
        })
    }
    #[test]
    fn absent_focused_authority_keeps_pen_grant_in_both_channel_orders() -> SessionResult<()> {
        for authority_first in [false, true] {
            let hub = RemoteHub::new(false);
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            let b = binding();
            s.binding = Some(b.clone());
            s.input_seq = 7;
            s.admission.reserve(7, 1, 5, "a".repeat(64))?;
            if authority_first {
                hub.adopt_authority(&mut s, &b, (7, 1, 5), None)?;
            } else {
                hub.command_result(&mut s, &injected(&b)?)?;
            }
            assert!(s.admission.busy());
            assert_eq!(s.binding.as_ref(), Some(&b));
            if authority_first {
                hub.command_result(&mut s, &injected(&b)?)?;
            } else {
                hub.adopt_authority(&mut s, &b, (7, 1, 5), None)?;
            }
            assert!(!s.admission.busy());
            assert_eq!(s.binding.as_ref(), Some(&b));
            assert!(s.authority.is_none());
            assert_eq!(s.input_seq, 7);
            assert_eq!(s.display.status, "controlling");
        }
        Ok(())
    }
    #[test]
    fn stale_binding_or_wrong_command_cannot_release_current_reservation() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        let b = binding();
        s.binding = Some(b.clone());
        s.admission.reserve(7, 1, 5, "a".repeat(64))?;
        let mut stale = b.clone();
        stale.scope.geometry_revision = 2;
        assert!(
            hub.adopt_authority(&mut s, &stale, (7, 1, 5), None)
                .is_err()
        );
        assert!(hub.adopt_authority(&mut s, &b, (8, 1, 5), None).is_err());
        assert!(s.admission.busy());
        Ok(())
    }
    #[test]
    fn cancelled_queued_up_is_retired_not_silently_decremented_or_regranted() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        let b = binding();
        s.binding = Some(b.clone());
        s.admission.admit_pen(vw_remote::wire::Phase::Down, 1);
        assert!(s.admission.queue(1)?); // an Up awaiting the phone worker
        assert!(!s.admission.can_reserve());
        RemoteHub::retire(&mut s); // deliberate recovery seal, not a queue decrement
        assert!(s.binding.is_none());
        assert!(!s.admission.busy());
        assert!(hub.adopt_authority(&mut s, &b, (7, 1, 5), None).is_err());
        Ok(())
    }
    #[test]
    fn retirement_invalidates_binding_and_barrier_without_regrant() -> SessionResult<()> {
        let hub = RemoteHub::new(false);
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        let b = binding();
        s.binding = Some(b.clone());
        s.admission.reserve(7, 1, 5, "a".repeat(64))?;
        RemoteHub::retire(&mut s);
        assert!(s.binding.is_none());
        assert!(!s.admission.busy());
        assert!(hub.adopt_authority(&mut s, &b, (7, 1, 5), None).is_err());
        Ok(())
    }

    fn stroke_fixture() -> SessionResult<(Arc<RemoteHub>, Binding)> {
        let (hub, b) = observation_fixture()?;
        {
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            s.admission = admission::Admission::default();
            let mut a = authority(false);
            a.identity = vw_remote::profile::ImageIdentity {
                executable_name: "mspaint.exe".into(),
                executable_blake3:
                    "b2a7f30ffa5f4864fbe63b49c64153f1d75a278f5771f14e59ffb6be4f65792f".into(),
                executable_bytes: 4_272_128,
                file_version: Some("11.2605.81.0".into()),
                package_full_name: Some(vw_remote::profile::essential::PAINT_PACKAGE.into()),
                package_version: Some("11.2605.81.0".into()),
            };
            a.verified_actions = vec![3, 6, 7, 8, 9];
            s.authority = Some(a);
            s.stroke.pen(vw_remote::wire::Phase::Down, 1);
            s.stroke.pen(vw_remote::wire::Phase::Up, 2);
            s.input_seq = 2;
        }
        Ok((hub, b))
    }
    fn stroke_update(s: &State, b: &Binding) -> SessionResult<pb::RemoteControl> {
        let mut a = s.authority.clone().ok_or(SessionError::Invalid)?;
        a.verified_actions.insert(0, 1); // actual fixture observation of Undo
        a.selected_canvas = a.focus.take();
        Ok(pb::RemoteControl {
            scope: Some(scope_pb(&b.scope)?),
            sequence: 1,
            action: "stroke_authority".into(),
            input_session_id: Some(native_id(&b.input_session_id)?.to_proto()),
            command_input_seq: 2,
            shortcut_state_json: json(&a)?.into_bytes(),
            ..Default::default()
        })
    }
    #[test]
    fn stroke_observation_updates_availability_without_finite_receipt_or_coverage()
    -> SessionResult<()> {
        let (hub, b) = stroke_fixture()?;
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        s.admission.admit_pen(vw_remote::wire::Phase::Down, 1);
        s.admission.admit_pen(vw_remote::wire::Phase::Up, 2);
        let update = stroke_update(&s, &b)?;
        hub.receive_stroke_authority(&mut s, &update)?;
        assert!(action_allowed(&s, &b, 1).is_ok());
        assert!(!s.admission.busy() && s.commands.is_empty());
        assert!(!s.admission.can_reserve()); // observation is not a covering ACK
        assert_eq!(s.input_seq, 2);
        assert_eq!(s.stroke.published_up, 2);
        let published = s.display.sequence;
        hub.receive_stroke_authority(&mut s, &update)?;
        assert_eq!(s.display.sequence, published); // duplicate disposed
        Ok(())
    }
    #[test]
    fn queued_hover_defers_and_queued_new_down_discards_old_stroke() -> SessionResult<()> {
        use vw_remote::wire::Phase;
        for phase in [Phase::Hover, Phase::Down] {
            let (hub, b) = stroke_fixture()?;
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            let update = stroke_update(&s, &b)?;
            assert!(s.admission.queue(1)?);
            hub.receive_stroke_authority(&mut s, &update)?;
            assert!(s.stroke.pending.is_some());
            assert!(action_allowed(&s, &b, 1).is_err());
            s.admission.check_pen(phase)?; // observer creates no input barrier
            s.admission.admit_pen(phase, 3);
            s.stroke.pen(phase, 3);
            s.input_seq = 3;
            hub.apply_pending_stroke_authority(&mut s)?;
            assert_eq!(action_allowed(&s, &b, 1).is_ok(), phase == Phase::Hover);
            assert!(s.stroke.pending.is_none());
        }
        Ok(())
    }
    #[test]
    fn stale_stroke_after_finite_or_new_down_is_disposed_before_absent_authority_validation()
    -> SessionResult<()> {
        for finite in [false, true] {
            let (hub, b) = stroke_fixture()?;
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            let update = stroke_update(&s, &b)?;
            if finite {
                s.stroke.invalidate();
            } else {
                s.stroke.pen(vw_remote::wire::Phase::Down, 3);
            }
            s.input_seq = 3;
            s.authority = None;
            hub.receive_stroke_authority(&mut s, &update)?;
            assert!(s.authority.is_none() && s.stroke.pending.is_none());
            assert_eq!(s.stroke.published_up, 0);
        }
        Ok(())
    }
    #[test]
    fn stroke_requires_exact_binding_profile_identity_routes_and_canvas() -> SessionResult<()> {
        for mode in 0..10 {
            let (hub, b) = stroke_fixture()?;
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            let mut update = stroke_update(&s, &b)?;
            let mut a: vw_remote::profile::Authority =
                serde_json::from_slice(&update.shortcut_state_json)
                    .map_err(|_| SessionError::Invalid)?;
            match mode {
                0 => {
                    update.input_session_id =
                        Some(native_id("018bcfe5-6800-7000-8000-000000000004")?.to_proto())
                }
                1 => {
                    update
                        .scope
                        .as_mut()
                        .ok_or(SessionError::Invalid)?
                        .source_generation += 1
                }
                2 => a.profile_digest = "d".repeat(64),
                3 => a.identity.executable_blake3 = "d".repeat(64),
                4 => a.routes = vw_remote::profile::AuthorityRoutes::Legacy,
                5 => {
                    a.selected_canvas
                        .as_mut()
                        .ok_or(SessionError::Invalid)?
                        .runtime_id_hash = "d".repeat(64)
                }
                6 => {
                    a.selected_canvas
                        .as_mut()
                        .ok_or(SessionError::Invalid)?
                        .canvas_rect
                        .width -= 1
                }
                7 => {
                    a.selected_canvas
                        .as_mut()
                        .ok_or(SessionError::Invalid)?
                        .process_id += 1
                }
                8 => update.command_input_seq = 3,
                _ => {
                    update
                        .scope
                        .as_mut()
                        .ok_or(SessionError::Invalid)?
                        .geometry_revision += 1
                }
            }
            update.shortcut_state_json = json(&a)?.into_bytes();
            assert!(hub.receive_stroke_authority(&mut s, &update).is_err());
            assert!(action_allowed(&s, &b, 1).is_err());
            assert!(s.commands.is_empty() && !s.admission.busy());
        }
        Ok(())
    }
    #[test]
    fn stroke_control_has_closed_query_shape_and_retired_disposal_only() -> SessionResult<()> {
        let (hub, b) = stroke_fixture()?;
        let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        let update = stroke_update(&s, &b)?;
        retired::control_shape(&update, false)?;
        assert!(retired::control_shape(&update, true).is_err());
        for mode in 0..7 {
            let mut bad = update.clone();
            match mode {
                0 => bad.command_input_seq = 0,
                1 => bad.editor_action = 1,
                2 => bad.request_nonce = 1,
                3 => bad.reason = "input_busy".into(),
                4 => bad.shortcut_state_json.clear(),
                5 => bad.selected_target_json = vec![1],
                _ => bad.selected_destination_label = "other".into(),
            }
            assert!(hub.receive_stroke_authority(&mut s, &bad).is_err());
            assert!(retired::control_shape(&bad, false).is_err());
        }
        RemoteHub::retire(&mut s);
        retired::control_shape(&update, false)?; // structural disposal, no grant
        assert!(hub.receive_stroke_authority(&mut s, &update).is_err());
        assert!(s.binding.is_none() && s.authority.is_none() && s.commands.is_empty());
        assert!(!s.stroke.current(2));
        Ok(())
    }
    #[test]
    fn only_completed_contact_and_latest_up_can_publish() {
        use vw_remote::wire::Phase;
        let mut s = StrokeObservation::default();
        for (phase, seq) in [(Phase::Hover, 1), (Phase::Up, 2), (Phase::Move, 3)] {
            s.pen(phase, seq);
            assert!(!s.current(seq));
        }
        s.pen(Phase::Down, 4);
        s.pen(Phase::Move, 5);
        assert!(!s.current(5));
        s.pen(Phase::Up, 6);
        assert!(s.current(6));
        s.pen(Phase::Down, 7);
        assert!(!s.current(6));
        s.pen(Phase::Up, 8);
        assert!(s.current(8) && !s.current(6));
        s.pen(Phase::Leave, 9);
        assert!(!s.current(8));
    }

    #[cfg(windows)]
    #[test]
    fn late_local_observation_after_focus_revoke_reconnect_or_finite_is_nonfatal_disposal()
    -> SessionResult<()> {
        for mode in 0..5 {
            let (hub, b) = stroke_fixture()?;
            let actual;
            {
                let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
                actual = s.authority.clone().ok_or(SessionError::Invalid)?;
                match mode {
                    0 => RemoteHub::retire_authority(&mut s), // same path as focus revoke
                    1 => RemoteHub::retire(&mut s),
                    2 => RemoteHub::reset_source(&mut s, Some(3)),
                    3 => {
                        s.stroke.invalidate();
                        s.authority = None; // finite observation unavailable
                    }
                    _ => {
                        let mut replacement = b.clone();
                        replacement.input_session_id =
                            "018bcfe5-6800-7000-8000-000000000004".into();
                        s.binding = Some(replacement);
                    }
                }
            }
            // Fixture has no live sender: an obsolete result must be disposed
            // before either current-carrier or absent-authority validation.
            hub.publish_stroke_authority(&b, 2, actual)?;
            let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            assert_eq!(s.stroke.published_up, 0);
            assert!(s.stroke.pending.is_none() && s.commands.is_empty());
        }
        Ok(())
    }
}
