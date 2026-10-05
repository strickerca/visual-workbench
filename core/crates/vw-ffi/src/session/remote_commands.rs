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
fn action_allowed<'a>(
    s: &'a State,
    binding: &Binding,
    action: u32,
) -> SessionResult<&'a vw_remote::profile::Authority> {
    if s.binding.as_ref() != Some(binding) || !(1..=16).contains(&action) {
        return Err(SessionError::Authentication);
    }
    let authority = s.authority.as_ref().ok_or(SessionError::Authentication)?;
    if !authority.verified_actions.contains(&action)
        || authority.profile_digest != authority.focus.profile_digest
    {
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
            focus_runtime_hash: authority.focus.runtime_id_hash.clone(),
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
            if next.profile_digest != command.digest
                || next.focus.profile_digest != command.digest
                || next.identity != old.identity
                || next.focus.process_id != target.process_id
                || next.focus.sampled_qpc_100ns == 0
                || !vw_remote::profile::digest(&next.focus.runtime_id_hash)
                || next.verified_actions.len() > 16
                || next.verified_actions.iter().any(|a| !(1..=16).contains(a))
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
}
