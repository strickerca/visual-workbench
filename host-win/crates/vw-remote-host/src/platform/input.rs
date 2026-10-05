//! Separate input helper: bounded actual samples, guarded contact refresh and
//! finite SendInput batches. Partial batches seal; no global release claim.
use super::{clock_100ns, confine, native_guard, unchanged};
mod stages;
use crate::{Error, Result};
use std::{
    io,
    sync::mpsc,
    time::{Duration, Instant},
};
use vw_remote::{
    Binding, Target,
    wire::{self, Command, FiniteAction, Header, Packet, PenSample, Phase},
};
use windows::Win32::UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*};
// A Pending receipt is a promise to remain alive for an exact Stop/Stopped
// rendezvous. Later local release completion cannot silently revoke that promise.
trait RetirementPort {
    fn retire(&mut self) -> vw_remote::wire::FiniteRetirement;
    fn wait_retired(&mut self);
}
impl<S: 'static> RetirementPort for super::finite_input::FiniteInputOwner<S> {
    fn retire(&mut self) -> vw_remote::wire::FiniteRetirement {
        super::finite_input::FiniteInputOwner::retire(self)
    }
    fn wait_retired(&mut self) {
        super::finite_input::FiniteInputOwner::wait_retired(self)
    }
}
fn failure_returns_to_loop(owner: &mut impl RetirementPort, reported_pending: bool) -> bool {
    let retirement = owner.retire();
    if reported_pending || !matches!(retirement, vw_remote::wire::FiniteRetirement::Complete) {
        owner.wait_retired();
        true
    } else {
        false
    }
}
fn stopped_after_retirement(owner: &mut impl RetirementPort, sequence: u64) -> Packet {
    owner.wait_retired();
    Packet {
        header: Header::Stopped {
            sequence,
            pen_release: None,
        },
        payload: Vec::new(),
    }
}
fn probe(target: &Target) -> super::guard::Target {
    fn r(v: vw_remote::Rect) -> super::guard::Rect {
        super::guard::Rect {
            left: v.x,
            top: v.y,
            right: v.x + v.width as i32,
            bottom: v.y + v.height as i32,
        }
    }
    super::guard::Target {
        hwnd: target.window as usize,
        pid: target.process_id,
        thread: target.thread_id,
        process_created: target.process_created,
        window: r(target.window_rect),
        client: r(target.client_rect),
        dpi: target.dpi,
        integrity: target.integrity,
    }
}
struct PreparedStages<'a> {
    target: &'a Target,
    owner_pid: u32,
    owner: &'a mut super::finite_input::FiniteInputOwner<()>,
    command: super::shortcuts::PreparedFiniteCommand,
}
impl stages::Stages for PreparedStages<'_> {
    fn first(&mut self) -> super::finite_input::FiniteAttempt {
        let stage = self.command.first_stage();
        self.owner.send(stage.inputs(), stage.destination(), |_| {
            super::shortcuts::validate_stage(self.target, self.owner_pid, &self.command, &stage)
        })
    }
    fn second(
        &mut self,
        first: &super::finite_input::FiniteAttempt,
    ) -> Result<Option<super::finite_input::FiniteAttempt>> {
        // This exact first receipt came from this same command and live owner.
        // prepare_second refuses Pending/error before any provider query, and
        // reuses the original 180ms command deadline rather than starting anew.
        let Some(second) =
            super::shortcuts::prepare_second(self.target, self.owner_pid, &self.command, first)?
        else {
            return Ok(None);
        };
        let stage = second.stage();
        if stage
            .inputs()
            .len()
            .checked_add(first.expected_count as usize)
            .is_none_or(|n| n > 16)
        {
            return Err(Error::Limit);
        }
        Ok(Some(self.owner.send(
            stage.inputs(),
            stage.destination(),
            |_| {
                super::shortcuts::validate_stage(self.target, self.owner_pid, &self.command, &stage)
            },
        )))
    }
}
struct Input {
    target: Target,
    binding: Binding,
    owner_pid: u32,
    device: Option<native_guard::Injector>,
    last_seq: u64,
    last_real: Instant,
    last_refresh: Instant,
    last_sample: Option<PenSample>,
    contact: bool,
    sealed: Option<Error>,
    finite_owner: super::finite_input::FiniteInputOwner<()>,
    finite_attempt: Option<super::finite_input::FiniteAttempt>,
    #[cfg(feature = "integration-carrier-fault")]
    release_record: std::sync::Arc<native_guard::ReleaseRecord>,
}
impl Input {
    fn open(target: Target, binding: Binding, owner_pid: u32) -> Result<Self> {
        target.validate()?;
        binding.validate()?;
        if target.token != binding.scope.target_token {
            return Err(Error::Invalid);
        }
        unchanged(&target, owner_pid)?;
        // This process owns one immutable UUID binding for its entire lifetime.
        // The probe guard's local integer is not exposed as transport authority.
        let device = native_guard::Injector::arm(probe(&target), 1, false)
            .map_err(|_| Error::TargetChanged)?;
        #[cfg(feature = "integration-carrier-fault")]
        let release_record = std::sync::Arc::new(native_guard::ReleaseRecord::new());
        #[cfg(feature = "integration-carrier-fault")]
        let device = device.with_release_record(release_record.clone());
        let finite_owner =
            super::finite_input::FiniteInputOwner::new(target.clone(), owner_pid, ())?;
        Ok(Self {
            target,
            binding,
            owner_pid,
            device: Some(device),
            last_seq: 0,
            last_real: Instant::now(),
            last_refresh: Instant::now(),
            last_sample: None,
            contact: false,
            sealed: None,
            finite_owner,
            finite_attempt: None,
            #[cfg(feature = "integration-carrier-fault")]
            release_record,
        })
    }
    fn seal(&mut self, error: Error) {
        self.sealed = Some(error);
        self.contact = false;
        self.device.take();
    }
    fn validate(&mut self) -> Result<()> {
        if let Some(e) = &self.sealed {
            return Err(e.clone());
        }
        if self.last_real.elapsed() >= Duration::from_secs(600) {
            self.seal(Error::Ungranted);
            return Err(Error::Ungranted);
        }
        let result = unchanged(&self.target, self.owner_pid).and_then(|()| {
            let point = self.last_sample.map(|s| (s.x, s.y)).unwrap_or((
                self.target.client_rect.x + self.target.client_rect.width as i32 / 2,
                self.target.client_rect.y + self.target.client_rect.height as i32 / 2,
            ));
            self.device
                .as_mut()
                .ok_or(Error::Ungranted)?
                .validate_point(point.0, point.1)
                .map_err(|_| Error::TargetChanged)
        });
        if let Err(e) = &result {
            self.seal(e.clone())
        }
        result
    }
    fn refresh(&mut self) -> Result<()> {
        self.validate()?;
        if self.contact && self.last_refresh.elapsed() >= Duration::from_millis(20) {
            let mut sample = self.last_sample.ok_or(Error::Invalid)?;
            sample.phase = Phase::Move;
            self.device
                .as_mut()
                .ok_or(Error::Ungranted)?
                .send(sample)
                .map_err(|_| Error::TargetChanged)?;
            self.last_refresh = self
                .device
                .as_ref()
                .and_then(|v| v.contact_started())
                .unwrap_or_else(Instant::now);
        }
        Ok(())
    }
    fn sequence(&mut self, value: u64) -> Result<()> {
        if value == 0 || value <= self.last_seq {
            self.seal(Error::Invalid);
            return Err(Error::Invalid);
        }
        self.validate()
    }
    fn pen(&mut self, seq: u64, sample: PenSample) -> Result<u64> {
        self.sequence(seq)?;
        sample.validate()?;
        let transition = match sample.phase {
            Phase::Down => !self.contact,
            Phase::Move | Phase::Up => self.contact,
            Phase::Hover | Phase::Leave => !self.contact,
        };
        if !transition {
            return Err(Error::Invalid);
        }
        self.device
            .as_mut()
            .ok_or(Error::Ungranted)?
            .send(sample)
            .map_err(|_| Error::TargetChanged)?;
        // Stamp only after actual guarded OS injection returned success.
        let qpc = clock_100ns()?;
        self.contact = matches!(sample.phase, Phase::Down | Phase::Move);
        self.last_sample = Some(sample);
        self.last_seq = seq;
        self.last_real = Instant::now();
        self.last_refresh = self
            .device
            .as_ref()
            .and_then(|v| v.contact_started())
            .unwrap_or_else(Instant::now);
        Ok(qpc)
    }
    fn finite(&mut self, seq: u64, action: FiniteAction) -> Result<u64> {
        // Do not erase a retained Pending attempt merely because another
        // request arrived. No source/UIA work runs while contact/downs are held.
        stages::provider_ready(false, self.finite_attempt.as_ref())?;
        self.finite_attempt = None;
        stages::provider_ready(self.contact, None)?;
        self.sequence(seq)?;
        match action {
            FiniteAction::Click { x, y, button } => {
                if !matches!(button, 1 | 2) || !self.target.client_rect.contains(x, y) {
                    return Err(Error::Invalid);
                }
                self.device
                    .as_mut()
                    .ok_or(Error::Ungranted)?
                    .validate_point(x, y)
                    .map_err(|_| Error::TargetChanged)?;
                let (down, up) = if button == 1 {
                    (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
                } else {
                    (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
                };
                let point = absolute(x, y)?;
                let batch = [
                    mouse(
                        point.0,
                        point.1,
                        0,
                        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    ),
                    mouse(0, 0, 0, down),
                    mouse(0, 0, 0, up),
                ];
                self.send_finite(&batch, Some((x, y)))?;
            }
            FiniteAction::Wheel { x, y, delta } => {
                if delta == 0
                    || delta.unsigned_abs() > 1200
                    || delta % 120 != 0
                    || !self.target.client_rect.contains(x, y)
                {
                    return Err(Error::Invalid);
                }
                self.device
                    .as_mut()
                    .ok_or(Error::Ungranted)?
                    .validate_point(x, y)
                    .map_err(|_| Error::TargetChanged)?;
                let point = absolute(x, y)?;
                let batch = [
                    mouse(
                        point.0,
                        point.1,
                        0,
                        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    ),
                    mouse(0, 0, delta as u32, MOUSEEVENTF_WHEEL),
                ];
                self.send_finite(&batch, Some((x, y)))?;
            }
            FiniteAction::Shortcut {
                profile_digest,
                action,
                focus_runtime_hash,
            } => {
                // Contact was refused before any provider lookup. Only a current
                // uniquely scoped native element may supply this finite batch.
                let command = super::shortcuts::prepare_command(
                    &self.target,
                    self.owner_pid,
                    &profile_digest,
                    action,
                    &focus_runtime_hash,
                )?;
                let mut prepared = PreparedStages {
                    target: &self.target,
                    owner_pid: self.owner_pid,
                    owner: &mut self.finite_owner,
                    command,
                };
                stages::execute(&mut prepared, &mut self.finite_attempt)?;
            }
        }
        let qpc =
            clock_100ns().map_err(|error| stages::failure(&mut self.finite_attempt, error))?;
        self.last_seq = seq;
        self.last_real = Instant::now();
        Ok(qpc)
    }
    // Separate query-only command, scheduled by the parent after its actual
    // input receipt/covering-ACK publication. Never invoke from pen()/Check.
    fn refresh_authority(
        &mut self,
        profile_digest: &str,
    ) -> Result<Option<vw_remote::profile::Authority>> {
        // A defensively refused refresh must not drop an active pen contact.
        // The parent reservation normally prevents this query during contact.
        stages::provider_ready(false, self.finite_attempt.as_ref())?;
        if self.contact {
            return Ok(None);
        }
        self.validate()?;
        match super::shortcuts::refresh_authority(&self.target, self.owner_pid, profile_digest) {
            Ok(authority) => Ok(Some(authority)),
            Err(Error::Unavailable | Error::Ungranted) => Ok(None),
            Err(error) => Err(error),
        }
    }
    fn send_finite(&mut self, batch: &[INPUT], destination: Option<(i32, i32)>) -> Result<()> {
        self.validate()?;
        if batch.is_empty() || batch.len() > 16 {
            return Err(Error::Limit);
        }
        let point = destination.ok_or(Error::Invalid)?;
        // Raw pointer actions retain the common owner's neutral-state and
        // final foreground/root/point fence. Only its actual accepted prefix
        // can create release ownership.
        let attempt = self.finite_owner.send(batch, point, |_| Ok(()));
        let error = attempt.error.clone();
        self.finite_attempt = Some(attempt);
        if let Some(error) = error {
            self.seal(error.clone());
            return Err(stages::failure(&mut self.finite_attempt, error));
        }
        Ok(())
    }
}
fn mouse(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
fn absolute(x: i32, y: i32) -> Result<(i32, i32)> {
    // SAFETY: query-only physical virtual-desktop metrics after helper PMv2 init.
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    if width <= 1
        || height <= 1
        || i64::from(x) < i64::from(left)
        || i64::from(y) < i64::from(top)
        || i64::from(x) >= i64::from(left) + i64::from(width)
        || i64::from(y) >= i64::from(top) + i64::from(height)
    {
        return Err(Error::Invalid);
    }
    Ok((
        ((i64::from(x) - i64::from(left)) * 65535 / (i64::from(width) - 1)) as i32,
        ((i64::from(y) - i64::from(top)) * 65535 / (i64::from(height) - 1)) as i32,
    ))
}
pub fn run() -> Result<()> {
    let _job = confine()?;
    let _apartment = super::shortcuts::Apartment::initialize()?;
    native_guard::initialize_dpi().map_err(|_| Error::Unavailable)?;
    let _priority = native_guard::input_priority().map_err(|_| Error::Unavailable)?;
    let (requests, receive) = mpsc::sync_channel::<Result<Command>>(1);
    // The pipe reader belongs to this isolated helper process. Parent retirement
    // observes whole-process exit and joins its own IO worker before releasing.
    std::thread::Builder::new()
        .name("vw-input-reader".into())
        .spawn(move || {
            let mut stdin = io::stdin().lock();
            loop {
                let value = wire::read_command(&mut stdin);
                let failed = value.is_err();
                if requests.send(value).is_err() || failed {
                    break;
                }
            }
        })
        .map_err(|_| Error::Io)?;
    let command = receive.recv().map_err(|_| Error::Io)??;
    let Command::OpenInput {
        sequence: 1,
        target,
        binding,
        owner_pid,
        profile,
    } = command
    else {
        return Err(Error::Invalid);
    };
    let authority = super::shortcuts::install(&target, owner_pid, profile)?;
    let mut output = io::stdout().lock();
    let mut input = match Input::open(target, binding, owner_pid) {
        Ok(v) => v,
        Err(error) => {
            wire::write_packet(
                &mut output,
                &Packet {
                    header: Header::Refused {
                        sequence: 1,
                        error: error.clone(),
                        finite_attempt: None,
                    },
                    payload: Vec::new(),
                },
            )?;
            return Err(error);
        }
    };
    wire::write_packet(
        &mut output,
        &Packet {
            header: Header::Ready {
                sequence: 1,
                capabilities: None,
                input_authority: authority,
            },
            payload: Vec::new(),
        },
    )?;
    let mut last = 1u64;
    loop {
        if input.sealed.is_none()
            && let Err(e) = input.refresh()
        {
            input.seal(e)
        }
        let command = match receive.recv_timeout(Duration::from_millis(2)) {
            Ok(v) => v?,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Error::Io),
        };
        let sequence = command.sequence();
        if sequence != last.checked_add(1).ok_or(Error::Limit)? {
            return Err(Error::Invalid);
        }
        last = sequence;
        if matches!(command, Command::Stop { .. }) {
            let terminal = stopped_after_retirement(&mut input.finite_owner, sequence);
            #[cfg(feature = "integration-carrier-fault")]
            let release = (input.release_record.clone(), input.binding.clone());
            drop(input);
            #[cfg(feature = "integration-carrier-fault")]
            let terminal = {
                let mut terminal = terminal;
                if let Header::Stopped { pen_release, .. } = &mut terminal.header {
                    *pen_release = release.0.witness(release.1);
                }
                terminal
            };
            wire::write_packet(&mut output, &terminal)?;
            return Ok(());
        }
        let finite_command = matches!(command, Command::Finite { .. });
        let result = match command {
            Command::Pen {
                input_seq, sample, ..
            } => input.pen(input_seq, sample).map(|qpc| Header::Injected {
                sequence,
                binding: input.binding.clone(),
                input_seq,
                accepted_qpc_100ns: qpc,
            }),
            Command::Finite {
                input_seq, action, ..
            } => input.finite(input_seq, action).map(|qpc| Header::Injected {
                sequence,
                binding: input.binding.clone(),
                input_seq,
                accepted_qpc_100ns: qpc,
            }),
            Command::RefreshAuthority { profile_digest, .. } => input
                .refresh_authority(&profile_digest)
                .map(|authority| Header::Authority {
                    sequence,
                    binding: input.binding.clone(),
                    authority,
                }),
            Command::Check { .. } => input.validate().map(|()| Header::Idle { sequence }),
            _ => Err(Error::Invalid),
        };
        match result {
            Ok(header) => wire::write_packet(
                &mut output,
                &Packet {
                    header,
                    payload: Vec::new(),
                },
            )?,
            Err(error) => {
                input.seal(error.clone());
                let finite_attempt = if finite_command {
                    let mut attempt = input.finite_attempt.clone().unwrap_or(
                        super::finite_input::FiniteAttempt {
                            expected_count: 0,
                            accepted_count: 0,
                            accepted_qpc_100ns: None,
                            error: None,
                            retirement: vw_remote::wire::FiniteRetirement::Complete,
                        },
                    );
                    // The overall command may be PartialInput while its actual
                    // failing stage records a more precise timeout/guard cause.
                    if attempt.error.is_none() {
                        attempt.error = Some(error.clone());
                    }
                    Some(attempt)
                } else {
                    None
                };
                // A prior Pending reply promises a Stop/Stopped rendezvous for
                // this helper lifetime. Non-finite refusals (including refresh)
                // do not carry finite_attempt, but cannot revoke that promise.
                let reported_pending = input.finite_attempt.as_ref().is_some_and(|attempt| {
                    !matches!(
                        attempt.retirement,
                        vw_remote::wire::FiniteRetirement::Complete
                    )
                });
                wire::write_packet(
                    &mut output,
                    &Packet {
                        header: Header::Refused {
                            sequence,
                            error: error.clone(),
                            finite_attempt,
                        },
                        payload: Vec::new(),
                    },
                )?;
                if failure_returns_to_loop(&mut input.finite_owner, reported_pending) {
                    // Keep the real helper/source owner through exact Stop/Stopped,
                    // even if a second release attempt already completed. Parent
                    // cannot clear a previously reported Pending without a terminal.
                    continue;
                }
                return Err(error);
            }
        }
    }
}

#[cfg(test)]
mod retirement_loop_tests {
    use super::*;
    use vw_remote::wire::{FiniteAttempt, FiniteRetirement};
    struct Owner {
        next: FiniteRetirement,
        retire_calls: usize,
        wait_calls: usize,
    }
    impl RetirementPort for Owner {
        fn retire(&mut self) -> FiniteRetirement {
            self.retire_calls += 1;
            self.next.clone()
        }
        fn wait_retired(&mut self) {
            self.wait_calls += 1;
            self.next = FiniteRetirement::Complete;
        }
    }
    fn pending() -> FiniteRetirement {
        FiniteRetirement::Pending {
            held_count: 1,
            uncertain: false,
            error: Error::TargetChanged,
        }
    }
    #[test]
    fn reported_pending_then_second_complete_stays_for_exact_stop_terminal() -> Result<()> {
        let first = Packet {
            header: Header::Refused {
                sequence: 2,
                error: Error::PartialInput,
                finite_attempt: Some(FiniteAttempt {
                    expected_count: 3,
                    accepted_count: 2,
                    accepted_qpc_100ns: Some(99),
                    error: Some(Error::PartialInput),
                    retirement: pending(),
                }),
            },
            payload: vec![],
        };
        let mut bytes = vec![];
        wire::write_packet(&mut bytes, &first)?;
        let read = wire::read_packet(&mut std::io::Cursor::new(bytes))?;
        let Header::Refused {
            finite_attempt: Some(attempt),
            sequence: 2,
            ..
        } = read.header
        else {
            return Err(Error::Invalid);
        };
        let reported = !matches!(attempt.retirement, FiniteRetirement::Complete);
        let mut owner = Owner {
            next: FiniteRetirement::Complete,
            retire_calls: 0,
            wait_calls: 0,
        };
        assert!(failure_returns_to_loop(&mut owner, reported));
        assert_eq!((owner.retire_calls, owner.wait_calls), (1, 1));
        let mut stop_bytes = vec![];
        wire::write_command(&mut stop_bytes, &Command::Stop { sequence: 3 })?;
        let Command::Stop { sequence } = wire::read_command(&mut std::io::Cursor::new(stop_bytes))?
        else {
            return Err(Error::Invalid);
        };
        let terminal = stopped_after_retirement(&mut owner, sequence);
        let mut bytes = vec![];
        wire::write_packet(&mut bytes, &terminal)?;
        let read = wire::read_packet(&mut std::io::Cursor::new(bytes))?;
        assert!(matches!(read.header, Header::Stopped { sequence: 3, .. }));
        assert_eq!(owner.wait_calls, 2);
        Ok(())
    }
    #[test]
    fn pending_then_actual_complete_and_refresh_refusal_still_waits_for_stop() {
        let retained_attempt = FiniteAttempt {
            expected_count: 3,
            accepted_count: 2,
            accepted_qpc_100ns: Some(99),
            error: Some(Error::PartialInput),
            retirement: pending(),
        };
        let mut owner = Owner {
            next: FiniteRetirement::Complete,
            retire_calls: 0,
            wait_calls: 0,
        };
        assert!(failure_returns_to_loop(&mut owner, true));
        // Refresh refusal has no new finite receipt, but the retained prior
        // Pending promise must survive actual background release completion.
        let refresh_refusal = Header::Refused {
            sequence: 3,
            error: Error::RetirementPending,
            finite_attempt: None,
        };
        assert!(matches!(
            refresh_refusal,
            Header::Refused {
                finite_attempt: None,
                ..
            }
        ));
        let promised = !matches!(retained_attempt.retirement, FiniteRetirement::Complete);
        assert!(failure_returns_to_loop(&mut owner, promised));
        let terminal = stopped_after_retirement(&mut owner, 4);
        assert!(matches!(
            terminal.header,
            Header::Stopped { sequence: 4, .. }
        ));
        assert_eq!((owner.retire_calls, owner.wait_calls), (2, 3));
    }
    #[test]
    fn still_pending_release_waits_and_keeps_loop_alive() {
        let mut owner = Owner {
            next: pending(),
            retire_calls: 0,
            wait_calls: 0,
        };
        assert!(failure_returns_to_loop(&mut owner, true));
        assert_eq!((owner.retire_calls, owner.wait_calls), (1, 1));
    }
    #[test]
    fn unreported_complete_refusal_keeps_original_error_exit() {
        let mut owner = Owner {
            next: FiniteRetirement::Complete,
            retire_calls: 0,
            wait_calls: 0,
        };
        assert!(!failure_returns_to_loop(&mut owner, false));
        assert_eq!((owner.retire_calls, owner.wait_calls), (1, 0));
    }
}
