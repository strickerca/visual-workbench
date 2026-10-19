//! Explicit peer reservation followed by actual host input-helper retirement.
//! Reservation traffic is CONTROL only: it never records an injection/ACK.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
pub(super) const CAPABILITY: &str = "paint_drawing_check_v1";
pub(super) const ACTIONS: [&str; 4] = [
    "drawing_reserve",
    "drawing_reserved",
    "drawing_refused",
    "drawing_release",
];
pub(super) struct Gate {
    pub nonce: u64,
    pub binding: Binding,
    pub peer_ready: AtomicBool,
    pub refused: AtomicBool,
    #[cfg(windows)]
    pub parked: AtomicBool,
    #[cfg(windows)]
    pub release: AtomicBool,
    #[cfg(windows)]
    pub resumed: AtomicBool,
    pub cancelled: AtomicBool,
}
impl Gate {
    #[cfg(windows)]
    pub fn new(nonce: u64, binding: Binding) -> Self {
        Self {
            nonce,
            binding,
            peer_ready: AtomicBool::new(false),
            refused: AtomicBool::new(false),
            parked: AtomicBool::new(false),
            release: AtomicBool::new(false),
            resumed: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        }
    }
}
pub(super) fn shape(v: &pb::RemoteControl) -> SessionResult<()> {
    if !ACTIONS.contains(&v.action.as_str())
        || v.request_nonce == 0
        || v.editor_action != 0
        || !v.selected_target_json.is_empty()
        || !v.selected_destination_label.is_empty()
        || (v.action != "drawing_reserved" && v.command_input_seq != 0)
        || (v.action != "drawing_release" && !v.shortcut_state_json.is_empty())
        || (v.action == "drawing_refused" && v.reason != "input_busy")
        || (v.action != "drawing_refused" && !v.reason.is_empty())
    {
        return Err(SessionError::Invalid);
    }
    let _ = Id::from_proto(v.input_session_id.as_ref()).map_err(|_| SessionError::Invalid)?;
    Ok(())
}
impl RemoteHub {
    pub(crate) fn enable_drawing_capability(&self, epoch: u64, enabled: bool) -> SessionResult<()> {
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        if s.epoch != epoch {
            return Err(SessionError::Closed);
        }
        s.drawing_enabled = s.enabled && enabled;
        Ok(())
    }
    pub(super) fn drawing_control(
        &self,
        s: &mut State,
        binding: &Binding,
        action: &str,
        nonce: u64,
        covered: u64,
        authority: Option<&vw_remote::profile::Authority>,
        reason: &str,
    ) -> SessionResult<()> {
        Self::current(s)?;
        if !s.drawing_enabled || s.binding.as_ref() != Some(binding) {
            return Err(SessionError::Closed);
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
            request_nonce: nonce,
            command_input_seq: covered,
            shortcut_state_json: authority
                .map(json)
                .transpose()?
                .unwrap_or_default()
                .into_bytes(),
            reason: reason.into(),
            ..Default::default()
        };
        shape(&v)?;
        s.sender
            .as_ref()
            .ok_or(SessionError::Closed)?
            .enqueue(Body::RemoteControl(v))?;
        Ok(())
    }
    pub(super) fn receive_drawing(
        &self,
        s: &mut State,
        v: &pb::RemoteControl,
    ) -> SessionResult<()> {
        shape(v)?;
        let binding = s.binding.clone().ok_or(SessionError::Closed)?;
        if !s.drawing_enabled
            || scope_from(v.scope.clone())? != binding.scope
            || v.input_session_id != Some(native_id(&binding.input_session_id)?.to_proto())
        {
            return Err(SessionError::Authentication);
        }
        if self.host {
            let gate = s
                .drawing
                .as_ref()
                .filter(|g| g.nonce == v.request_nonce && g.binding == binding)
                .cloned()
                .ok_or(SessionError::Authentication)?;
            match v.action.as_str() {
                "drawing_refused" => {
                    if gate.peer_ready.load(Ordering::Acquire) {
                        return Err(SessionError::Invalid);
                    }
                    gate.refused.store(true, Ordering::Release);
                }
                "drawing_reserved" => {
                    if gate.peer_ready.swap(true, Ordering::AcqRel)
                        || gate.refused.load(Ordering::Acquire)
                    {
                        return Err(SessionError::Invalid);
                    }
                    // Peer waited for rendered coverage of its final pen and
                    // every earlier finite command's actual injected+refresh.
                    if s.applied
                        .as_ref()
                        .is_none_or(|a| a.last_sequence() != v.command_input_seq)
                    {
                        return Err(SessionError::Authentication);
                    }
                    s.admission.reserve_drawing(gate.nonce)?;
                    #[cfg(windows)]
                    self.workers
                        .lock()
                        .map_err(|_| SessionError::Worker)?
                        .park_drawing(gate)?;
                    #[cfg(not(windows))]
                    return Err(SessionError::RemoteUnavailable);
                }
                _ => return Err(SessionError::Authentication),
            }
        } else {
            match v.action.as_str() {
                "drawing_reserve" => {
                    if !s.admission.can_reserve() {
                        return self.drawing_control(
                            s,
                            &binding,
                            "drawing_refused",
                            v.request_nonce,
                            0,
                            None,
                            "input_busy",
                        );
                    }
                    s.admission.reserve_drawing(v.request_nonce)?;
                    self.drawing_control(
                        s,
                        &binding,
                        "drawing_reserved",
                        v.request_nonce,
                        s.input_seq,
                        None,
                        "",
                    )?;
                    self.publish(s, "controlling", None)?;
                }
                "drawing_release" => {
                    if s.admission.drawing != Some(v.request_nonce) {
                        return Err(SessionError::Authentication);
                    }
                    let actual: vw_remote::profile::Authority =
                        serde_json::from_slice(&v.shortcut_state_json)
                            .map_err(|_| SessionError::Invalid)?;
                    let previous = s.authority.as_ref().ok_or(SessionError::Closed)?;
                    if actual.identity != previous.identity
                        || actual.profile_digest != previous.profile_digest
                    {
                        return Err(SessionError::Authentication);
                    }
                    actual
                        .canvas_for_target(&s.selected.as_ref().ok_or(SessionError::Closed)?.target)
                        .map_err(failure)?;
                    s.authority = Some(actual);
                    s.admission.release_drawing(v.request_nonce)?;
                    self.publish(s, "controlling", None)?;
                }
                _ => return Err(SessionError::Authentication),
            }
        }
        Ok(())
    }
}
