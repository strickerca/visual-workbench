//! Opt-in HIL carrier fault. Default production binaries refuse both exports.
use super::*;

#[derive(Default)]
pub(super) struct FaultGate {
    armed: Option<(String, u64, Binding)>,
    used: bool,
}
impl FaultGate {
    fn arm(&mut self, run: &str, epoch: u64, binding: &Binding) -> SessionResult<()> {
        if self.used
            || self.armed.is_some()
            || run.len() != 32
            || !run
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || epoch == 0
            || epoch & 1 == 0
        {
            return Err(SessionError::Invalid);
        }
        self.armed = Some((run.into(), epoch, binding.clone()));
        Ok(())
    }
    fn consume(&mut self, run: &str, epoch: u64, binding: &Binding) -> SessionResult<()> {
        if self.used
            || self
                .armed
                .as_ref()
                .is_none_or(|(r, e, b)| r != run || *e != epoch || b != binding)
        {
            return Err(SessionError::Authentication);
        }
        self.used = true;
        self.armed = None;
        Ok(())
    }
}

#[uniffi::export]
impl LiveSession {
    /// Integration fixture feature only; binds a one-shot fault to this exact
    /// link, run and freshly granted input session before any pen admission.
    pub fn remote_integration_arm_carrier_fault(
        &self,
        run: String,
        binding_json: String,
    ) -> SessionResult<()> {
        if !cfg!(feature = "integration-carrier-fault") {
            return Err(SessionError::RemoteUnavailable);
        }
        let binding: Binding = parse(&binding_json, 4096)?;
        let s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
        if self.closed.load(Ordering::Acquire)
            || s.closed
            || !s.enabled
            || s.sender.is_none()
            || s.binding.as_ref() != Some(&binding)
            || s.input_seq != 0
        {
            return Err(SessionError::Authentication);
        }
        self.remote
            .integration_fault
            .lock()
            .map_err(|_| SessionError::Worker)?
            .arm(&run, s.epoch, &binding)
    }
    /// Close only the active duplex carrier. Never pause/close the RemoteHub,
    /// runtime, listener, input helper or foreign owner here. Normal connection
    /// receive failure and RemoteEpoch retirement must prove actual release.
    pub fn remote_integration_abort_carrier(
        &self,
        run: String,
        binding_json: String,
    ) -> SessionResult<()> {
        if !cfg!(feature = "integration-carrier-fault") {
            return Err(SessionError::RemoteUnavailable);
        }
        let binding: Binding = parse(&binding_json, 4096)?;
        let s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
        if self.closed.load(Ordering::Acquire)
            || s.closed
            || !s.enabled
            || s.binding.as_ref() != Some(&binding)
            || s.input_seq != 2
        {
            return Err(SessionError::Authentication);
        }
        let sender = s.sender.as_ref().ok_or(SessionError::Closed)?;
        self.remote
            .integration_fault
            .lock()
            .map_err(|_| SessionError::Worker)?
            .consume(&run, s.epoch, &binding)?;
        sender.close();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> SessionResult<Binding> {
        serde_json::from_str(r#"{"scope":{"connection_epoch":1,"capture_session_id":"01a10700-0000-7000-8000-000000000001","source_generation":1,"target_token":"01a10700-0000-7000-8000-000000000002","geometry_revision":1},"input_session_id":"01a10700-0000-7000-8000-000000000003"}"#).map_err(|_| SessionError::Invalid)
    }
    #[test]
    fn exact_run_epoch_and_binding_consumes_once() -> SessionResult<()> {
        let b = binding()?;
        let mut g = FaultGate::default();
        g.arm(&"a".repeat(32), 1, &b)?;
        assert!(g.consume(&"b".repeat(32), 1, &b).is_err());
        assert!(g.consume(&"a".repeat(32), 3, &b).is_err());
        let mut foreign = b.clone();
        foreign.input_session_id = "01a10700-0000-7000-8000-000000000004".into();
        assert!(g.consume(&"a".repeat(32), 1, &foreign).is_err());
        g.consume(&"a".repeat(32), 1, &b)?;
        assert!(g.consume(&"a".repeat(32), 1, &b).is_err());
        assert!(g.arm(&"c".repeat(32), 3, &b).is_err());
        Ok(())
    }
    #[test]
    fn unarmed_or_even_epoch_never_interrupts() -> SessionResult<()> {
        let b = binding()?;
        let mut g = FaultGate::default();
        assert!(g.consume(&"a".repeat(32), 1, &b).is_err());
        assert!(g.arm(&"a".repeat(32), 2, &b).is_err());
        assert!(g.arm("bad", 1, &b).is_err());
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct ReleaseGate {
    armed: Option<(String, String, Binding)>,
    witness: Option<(&'static str, u64, u64)>,
    pen_release: Option<vw_remote::wire::PenReleaseWitness>,
}
impl ReleaseGate {
    fn arm(&mut self, run: String, scenario: String, binding: Binding) -> SessionResult<()> {
        if self.armed.is_some()
            || run.len() != 32
            || !run
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !matches!(
                scenario.as_str(),
                "pause-held" | "background-held" | "disconnect-held"
            )
        {
            return Err(SessionError::Invalid);
        }
        self.armed = Some((run, scenario, binding));
        Ok(())
    }
    #[cfg(any(windows, test))]
    fn accept_release(&mut self, witness: vw_remote::wire::PenReleaseWitness) {
        if self.pen_release.is_none()
            && self
                .armed
                .as_ref()
                .is_some_and(|(_, _, b)| b == &witness.binding)
        {
            self.pen_release = Some(witness);
        }
    }
    fn report(&self, run: &str) -> SessionResult<Option<String>> {
        if self.armed.as_ref().is_none_or(|(r, _, _)| r != run) {
            return Err(SessionError::Authentication);
        }
        let (run_id, scenario, binding) =
            self.armed.as_ref().ok_or(SessionError::Authentication)?;
        Ok(match(self.witness,self.pen_release.as_ref()){
            (Some((cause,counter,frequency)),Some(pen_release))=>Some(serde_json::json!({"schema":1,"run_id":run_id,"scenario":scenario,"binding":binding,"cause":cause,"counter":counter,"frequency":frequency,"pen_release":pen_release}).to_string()),
            _=>None,
        })
    }
    #[cfg(any(windows, test))]
    fn observe(&mut self, binding: Option<&Binding>, cause: &str, counter: u64, frequency: u64) {
        if self.witness.is_some() || counter == 0 || frequency == 0 {
            return;
        }
        let Some((_, scenario, expected)) = self.armed.as_ref() else {
            return;
        };
        let expected_cause = match scenario.as_str() {
            "pause-held" => "owner_pause",
            "background-held" => "background",
            "disconnect-held" => "connection_retired",
            _ => return,
        };
        if binding != Some(expected) || cause != expected_cause {
            return;
        }
        self.witness = Some((expected_cause, counter, frequency));
    }
}
// Compare the already validated binding without constructing Id/String/Vec on
// the cancellation path. Wire bytes must match canonical UUIDv7 text exactly.
fn exact_wire_input_id(value: Option<&pb::Uuid>, expected: &str) -> bool {
    let Some(value) = value else { return false };
    let wire = value.value.as_slice();
    let text = expected.as_bytes();
    if wire.len() != 16
        || text.len() != 36
        || wire[6] >> 4 != 7
        || wire[8] >> 6 != 2
        || [8, 13, 18, 23].iter().any(|&i| text[i] != b'-')
    {
        return false;
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    wire.iter().enumerate().all(|(i, byte)| {
        let offset = 2 * i
            + usize::from(i >= 4)
            + usize::from(i >= 6)
            + usize::from(i >= 8)
            + usize::from(i >= 10);
        text[offset] == HEX[(byte >> 4) as usize] && text[offset + 1] == HEX[(byte & 15) as usize]
    })
}
impl RemoteHub {
    // Called only after exact helper terminal settlement and actual process/IO
    // retirement. The release hook itself remains fixed-field/nonblocking.
    #[cfg(windows)]
    pub(super) fn integration_pen_release(&self, witness: vw_remote::wire::PenReleaseWitness) {
        if !cfg!(feature = "integration-carrier-fault") || !self.host {
            return;
        }
        let Ok(mut gate) = self.integration_release.lock() else {
            return;
        };
        gate.accept_release(witness);
    }
    // Observation only: no blocking query, no error propagation and no changes
    // to the existing cancellation or release order. Failed observation means
    // absent evidence; it never delays release or permits lifecycle acceptance.
    pub(super) fn integration_release_cause(
        &self,
        s: &State,
        cause: &str,
        input_id: Option<&pb::Uuid>,
    ) {
        if !cfg!(feature = "integration-carrier-fault") || !self.host {
            return;
        }
        if cause != "connection_retired"
            && s.binding
                .as_ref()
                .is_none_or(|b| !exact_wire_input_id(input_id, &b.input_session_id))
        {
            return;
        }
        let Ok(mut gate) = self.integration_release.try_lock() else {
            return;
        };
        #[cfg(windows)]
        if let Some((counter, frequency)) = actual_host_clock() {
            gate.observe(s.binding.as_ref(), cause, counter, frequency);
        }
        #[cfg(not(windows))]
        let _ = (&mut gate, s, cause);
    }
}
#[cfg(windows)]
fn actual_host_clock() -> Option<(u64, u64)> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryPerformanceCounter(value: *mut i64) -> i32;
        fn QueryPerformanceFrequency(value: *mut i64) -> i32;
    }
    let mut counter = 0i64;
    let mut frequency = 0i64;
    // SAFETY: valid initialized local output buffers for read-only Win32 calls.
    let available = unsafe {
        QueryPerformanceCounter(&mut counter) != 0 && QueryPerformanceFrequency(&mut frequency) != 0
    };
    if !available || counter <= 0 || frequency <= 0 {
        return None;
    }
    Some((counter as u64, frequency as u64))
}
#[uniffi::export]
impl LiveSession {
    pub fn remote_integration_arm_release_witness(
        &self,
        run: String,
        binding_json: String,
        scenario: String,
    ) -> SessionResult<()> {
        if !cfg!(feature = "integration-carrier-fault") || !self.remote.host {
            return Err(SessionError::RemoteUnavailable);
        }
        let binding: Binding = parse(&binding_json, 4096)?;
        let s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
        if self.closed.load(Ordering::Acquire)
            || s.closed
            || !s.enabled
            || s.binding.as_ref() != Some(&binding)
        {
            return Err(SessionError::Authentication);
        }
        self.remote
            .integration_release
            .lock()
            .map_err(|_| SessionError::Worker)?
            .arm(run, scenario, binding)
    }
    pub fn remote_integration_release_witness(&self, run: String) -> SessionResult<Option<String>> {
        if !cfg!(feature = "integration-carrier-fault") || !self.remote.host {
            return Err(SessionError::RemoteUnavailable);
        }
        let gate = self
            .remote
            .integration_release
            .lock()
            .map_err(|_| SessionError::Worker)?;
        gate.report(&run)
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;
    fn binding() -> Binding {
        Binding {
            scope: Scope {
                connection_epoch: 1,
                capture_session_id: "01a10700-0000-7000-8000-000000000001".into(),
                source_generation: 1,
                target_token: "01a10700-0000-7000-8000-000000000002".into(),
                geometry_revision: 1,
            },
            input_session_id: "01a10700-0000-7000-8000-000000000003".into(),
        }
    }
    #[test]
    fn only_current_binding_and_requested_cause_record_actual_clock() -> SessionResult<()> {
        let mut gate = ReleaseGate::default();
        let b = binding();
        gate.arm("a".repeat(32), "pause-held".into(), b.clone())?;
        gate.observe(Some(&b), "input_expired", 100, 10_000_000);
        assert!(gate.witness.is_none());
        gate.observe(None, "owner_pause", 101, 10_000_000);
        assert!(gate.witness.is_none());
        let mut other = b.clone();
        other.input_session_id = "01a10700-0000-7000-8000-000000000004".into();
        gate.observe(Some(&other), "owner_pause", 102, 10_000_000);
        assert!(gate.witness.is_none());
        gate.observe(Some(&b), "owner_pause", 103, 10_000_000);
        assert_eq!(gate.witness, Some(("owner_pause", 103, 10_000_000)));
        gate.observe(Some(&b), "owner_pause", 104, 10_000_000);
        assert_eq!(gate.witness, Some(("owner_pause", 103, 10_000_000)));
        Ok(())
    }
    #[test]
    fn retired_binding_and_missing_clock_cannot_manufacture_witness() -> SessionResult<()> {
        let mut gate = ReleaseGate::default();
        let b = binding();
        gate.arm("a".repeat(32), "disconnect-held".into(), b.clone())?;
        gate.observe(None, "connection_retired", 100, 10_000_000);
        assert!(gate.witness.is_none());
        gate.observe(Some(&b), "connection_retired", 0, 10_000_000);
        assert!(gate.witness.is_none());
        gate.observe(Some(&b), "connection_retired", 101, 0);
        assert!(gate.witness.is_none());
        gate.observe(Some(&b), "connection_retired", 102, 10_000_000);
        assert_eq!(gate.witness, Some(("connection_retired", 102, 10_000_000)));
        Ok(())
    }
    #[test]
    fn witness_arm_is_once_per_exact_integration_owner() -> SessionResult<()> {
        let mut gate = ReleaseGate::default();
        let b = binding();
        assert!(
            gate.arm("a".repeat(32), "balanced".into(), b.clone())
                .is_err()
        );
        gate.arm("a".repeat(32), "background-held".into(), b.clone())?;
        assert!(
            gate.arm("b".repeat(32), "background-held".into(), b)
                .is_err()
        );
        Ok(())
    }
    #[test]
    fn cause_and_exact_terminal_release_are_both_required_in_either_order() -> SessionResult<()> {
        for release_first in [false, true] {
            let b = binding();
            let mut gate = ReleaseGate::default();
            let run = "a".repeat(32);
            gate.arm(run.clone(), "pause-held".into(), b.clone())?;
            let proof = vw_remote::wire::PenReleaseWitness {
                binding: b.clone(),
                held_before_release: true,
                release_started_qpc: 104,
                release_completed_qpc: 106,
                qpc_frequency: 10_000_000,
            };
            if release_first {
                gate.accept_release(proof);
            } else {
                gate.observe(Some(&b), "owner_pause", 103, 10_000_000);
            }
            assert!(gate.report(&run)?.is_none());
            if release_first {
                gate.observe(Some(&b), "owner_pause", 103, 10_000_000);
            } else {
                gate.accept_release(vw_remote::wire::PenReleaseWitness {
                    binding: b,
                    held_before_release: true,
                    release_started_qpc: 104,
                    release_completed_qpc: 106,
                    qpc_frequency: 10_000_000,
                });
            }
            let report = gate.report(&run)?.ok_or(SessionError::Invalid)?;
            assert!(report.contains("pen_release"));
            assert!(gate.report(&"b".repeat(32)).is_err());
        }
        Ok(())
    }
}

#[cfg(test)]
mod input_id_tests {
    use super::*;
    const ID: &str = "01a10700-0000-7000-8000-000000000003";
    fn wire() -> pb::Uuid {
        pb::Uuid {
            value: vec![1, 161, 7, 0, 0, 0, 112, 0, 128, 0, 0, 0, 0, 0, 0, 3],
        }
    }
    #[test]
    fn exact_uuid_wire_matches_without_id_construction() {
        assert!(exact_wire_input_id(Some(&wire()), ID));
        assert!(!exact_wire_input_id(
            Some(&wire()),
            "01a10700-0000-7000-8000-000000000004"
        ));
        assert!(!exact_wire_input_id(None, ID));
    }
    #[test]
    fn malformed_wire_and_non_v7_variant_are_refused() {
        let mut value = wire();
        value.value.pop();
        assert!(!exact_wire_input_id(Some(&value), ID));
        let mut value = wire();
        value.value[6] = 0x40;
        assert!(!exact_wire_input_id(Some(&value), ID));
        let mut value = wire();
        value.value[8] = 0;
        assert!(!exact_wire_input_id(Some(&value), ID));
    }
    #[test]
    fn noncanonical_text_never_matches() {
        assert!(!exact_wire_input_id(
            Some(&wire()),
            "01A10700-0000-7000-8000-000000000003"
        ));
        assert!(!exact_wire_input_id(
            Some(&wire()),
            "01a10700_0000-7000-8000-000000000003"
        ));
        assert!(!exact_wire_input_id(Some(&wire()), "short"));
    }
}
