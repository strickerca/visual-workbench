//! Display-only MCP capture-grant telemetry. It grants no capability, mutates
//! no project and carries no selector, token, agent or user-supplied string.
use super::{LiveSession, SessionError, SessionResult};
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, watch};
use vw_proto::{
    Message,
    v1::{self as pb, envelope::Body},
};

pub(super) const CAPABILITY: &str = "agent_capture_status_v1";
const MAX_LIFETIME_MS: u32 = 600_000;
const TTL: Duration = Duration::from_secs(3);
const LOCAL_TTL: Duration = Duration::from_secs(2);
const PROBE_INTERVAL: Duration = Duration::from_secs(1);
const UPDATE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AgentCaptureDisplay {
    pub sequence: u64,
    pub connection_epoch: u64,
    /// False means unknown, never inactive. This also covers unsupported peers,
    /// stale source callbacks, carrier loss and absence of a fresh probe reply.
    pub known: bool,
    pub active_grant_count: u32,
    pub active_capture: bool,
    pub remaining_ms: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Publisher,
    Receiver,
    Unsupported,
}
#[derive(Clone, Copy)]
struct Value {
    count: u32,
    active: bool,
    remaining: u32,
    observed: Instant,
}
struct State {
    epoch: u64,
    enabled: bool,
    stopped: bool,
    sequence: u64,
    sent: u64,
    received: u64,
    nonce: u64,
    probe: Option<(u64, Instant)>,
    local: Option<Value>,
    incoming: Option<Value>,
    next_probe: Instant,
    next_send: Instant,
    next_notify: Instant,
    dirty: bool,
    notify: bool,
}
pub(super) struct CaptureStatusHub {
    role: Role,
    state: Mutex<State>,
    signal: watch::Sender<AgentCaptureDisplay>,
    waiters: Arc<Semaphore>,
}
pub(super) struct CaptureStatusEpoch {
    hub: Arc<CaptureStatusHub>,
    epoch: u64,
}
impl Drop for CaptureStatusEpoch {
    fn drop(&mut self) {
        self.hub.deactivate(self.epoch);
    }
}
impl CaptureStatusHub {
    pub(super) fn new(host: bool) -> Arc<Self> {
        Self::with_role(if host && cfg!(target_os = "windows") {
            Role::Publisher
        } else if !host && cfg!(target_os = "android") {
            Role::Receiver
        } else {
            Role::Unsupported
        })
    }
    fn with_role(role: Role) -> Arc<Self> {
        let now = Instant::now();
        let (signal, _) = watch::channel(unknown(1, 0));
        Arc::new(Self {
            role,
            state: Mutex::new(State {
                epoch: 0,
                enabled: false,
                stopped: false,
                sequence: 1,
                sent: 0,
                received: 0,
                nonce: 0,
                probe: None,
                local: None,
                incoming: None,
                next_probe: now,
                next_send: now,
                next_notify: now,
                dirty: false,
                notify: false,
            }),
            signal,
            waiters: Arc::new(Semaphore::new(4)),
        })
    }
    pub(super) fn activate(
        self: &Arc<Self>,
        epoch: u64,
        negotiated: bool,
    ) -> SessionResult<CaptureStatusEpoch> {
        let mut state = self.state.lock().map_err(|_| SessionError::Worker)?;
        if state.stopped || epoch & 1 == 0 || epoch <= state.epoch {
            return Err(SessionError::Closed);
        }
        state.epoch = epoch;
        state.enabled = negotiated && self.role != Role::Unsupported;
        state.sent = 0;
        state.received = 0;
        state.nonce = 0;
        state.probe = None;
        state.local = None;
        state.incoming = None;
        state.dirty = false;
        state.notify = false;
        let now = Instant::now();
        state.next_probe = now;
        state.next_send = now;
        state.next_notify = now;
        self.publish(&mut state, now, true);
        Ok(CaptureStatusEpoch {
            hub: self.clone(),
            epoch,
        })
    }
    fn deactivate(&self, epoch: u64) {
        if let Ok(mut state) = self.state.lock() {
            if state.epoch != epoch {
                return;
            }
            state.enabled = false;
            state.epoch = epoch.saturating_add(1);
            state.local = None;
            state.incoming = None;
            state.probe = None;
            self.publish(&mut state, Instant::now(), true);
        }
    }
    pub(super) fn stop(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.stopped = true;
            state.enabled = false;
            state.local = None;
            state.incoming = None;
            state.probe = None;
            self.publish(&mut state, Instant::now(), true);
        }
    }
    fn current(state: &State, epoch: u64) -> SessionResult<()> {
        if state.stopped || !state.enabled || state.epoch != epoch || epoch & 1 == 0 {
            Err(SessionError::Closed)
        } else {
            Ok(())
        }
    }
    fn set(
        &self,
        epoch: u64,
        count: u32,
        active: bool,
        remaining: u32,
        now: Instant,
    ) -> SessionResult<()> {
        validate_value(true, count, active, remaining)?;
        if self.role != Role::Publisher {
            return Err(SessionError::Authentication);
        }
        let mut state = self.state.lock().map_err(|_| SessionError::Worker)?;
        Self::current(&state, epoch)?;
        state.local = Some(Value {
            count,
            active,
            remaining,
            observed: now,
        });
        state.dirty = true;
        self.publish(&mut state, now, false);
        Ok(())
    }
    fn invalidate(&self, epoch: u64) -> SessionResult<()> {
        if self.role != Role::Publisher {
            return Err(SessionError::Authentication);
        }
        let mut state = self.state.lock().map_err(|_| SessionError::Worker)?;
        Self::current(&state, epoch)?;
        state.local = None;
        state.dirty = true;
        self.publish(&mut state, Instant::now(), true);
        Ok(())
    }
    pub(super) fn receive(&self, value: pb::AgentCaptureStatus, epoch: u64) -> SessionResult<()> {
        self.receive_at(value, epoch, Instant::now())
    }
    fn receive_at(
        &self,
        value: pb::AgentCaptureStatus,
        epoch: u64,
        now: Instant,
    ) -> SessionResult<()> {
        validate_wire(&value)?;
        let mut state = self.state.lock().map_err(|_| SessionError::Worker)?;
        Self::current(&state, epoch)?;
        match (self.role, value.probe) {
            (Role::Publisher, true) => {
                if value.nonce <= state.nonce {
                    return Ok(());
                }
                state.nonce = value.nonce;
                state.probe = Some((value.nonce, now + TTL));
                state.dirty = true;
            }
            (Role::Receiver, false) => {
                let Some((nonce, deadline)) = state.probe else {
                    return Ok(());
                };
                if nonce != value.nonce || now >= deadline || value.sequence <= state.received {
                    return Ok(());
                }
                state.received = value.sequence;
                state.incoming = value.known.then_some(Value {
                    count: value.active_grant_count,
                    active: value.active_capture,
                    remaining: value.remaining_ms,
                    observed: now,
                });
                self.publish(&mut state, now, false);
            }
            _ => return Err(SessionError::Authentication),
        }
        Ok(())
    }
    pub(super) fn flush(&self, sender: &vw_net::ConnectionSender) -> SessionResult<()> {
        self.flush_at(Instant::now(), |value| {
            sender.enqueue(Body::AgentCaptureStatus(value))
        })
    }
    fn flush_at(
        &self,
        now: Instant,
        send: impl FnOnce(pb::AgentCaptureStatus) -> Result<(), vw_net::NetError>,
    ) -> SessionResult<()> {
        let mut state = self.state.lock().map_err(|_| SessionError::Worker)?;
        self.expire(&mut state, now);
        if state.notify && now >= state.next_notify {
            self.publish(&mut state, now, true);
        }
        if !state.enabled || state.stopped {
            return Ok(());
        }
        let message = match self.role {
            Role::Receiver if now >= state.next_probe => {
                let nonce = state
                    .nonce
                    .checked_add(1)
                    .ok_or(SessionError::Backpressure)?;
                pb::AgentCaptureStatus {
                    probe: true,
                    nonce,
                    ..Default::default()
                }
            }
            Role::Publisher if state.dirty && now >= state.next_send => {
                let Some((nonce, deadline)) = state.probe else {
                    return Ok(());
                };
                if now >= deadline {
                    state.probe = None;
                    return Ok(());
                }
                let value = self.display(&state, now);
                pb::AgentCaptureStatus {
                    probe: false,
                    nonce,
                    sequence: state
                        .sent
                        .checked_add(1)
                        .ok_or(SessionError::Backpressure)?,
                    known: value.known,
                    active_grant_count: value.active_grant_count,
                    active_capture: value.active_capture,
                    remaining_ms: value.remaining_ms,
                }
            }
            _ => return Ok(()),
        };
        match send(message) {
            Ok(()) => {
                if message.probe {
                    state.nonce = message.nonce;
                    state.probe = Some((message.nonce, now + TTL));
                    state.next_probe = now + PROBE_INTERVAL;
                } else {
                    state.sent = message.sequence;
                    state.dirty = false;
                    state.next_send = now + UPDATE_INTERVAL;
                }
                Ok(())
            }
            Err(vw_net::NetError::Backpressure) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    fn expire(&self, state: &mut State, now: Instant) {
        let expired = |value: Value| {
            value.count != 0
                && now.saturating_duration_since(value.observed).as_millis()
                    >= u128::from(value.remaining)
        };
        let stale = if self.role == Role::Publisher {
            state.local.is_some_and(|v| {
                now.saturating_duration_since(v.observed) >= LOCAL_TTL || expired(v)
            })
        } else {
            state.incoming.is_some()
                && (state.probe.is_none_or(|(_, deadline)| now >= deadline)
                    || state.incoming.is_some_and(|v| {
                        now.saturating_duration_since(v.observed) >= TTL || expired(v)
                    }))
        };
        if stale {
            state.local = None;
            state.incoming = None;
            state.dirty = true;
            self.publish(state, now, true);
        }
    }
    fn display(&self, state: &State, now: Instant) -> AgentCaptureDisplay {
        let value = if self.role == Role::Publisher {
            state.local
        } else {
            state.incoming
        };
        let Some(value) = value.filter(|_| state.enabled && !state.stopped) else {
            return unknown(state.sequence, state.epoch);
        };
        let elapsed = now
            .saturating_duration_since(value.observed)
            .as_millis()
            .min(u128::from(u32::MAX)) as u32;
        AgentCaptureDisplay {
            sequence: state.sequence,
            connection_epoch: state.epoch,
            known: true,
            active_grant_count: value.count,
            active_capture: value.active,
            remaining_ms: value.remaining.saturating_sub(elapsed),
        }
    }
    fn publish(&self, state: &mut State, now: Instant, force: bool) {
        if !force && now < state.next_notify {
            state.notify = true;
            return;
        }
        state.sequence = state.sequence.saturating_add(1);
        state.notify = false;
        state.next_notify = now + UPDATE_INTERVAL;
        self.signal.send_replace(self.display(state, now));
    }
    fn snapshot(&self) -> SessionResult<AgentCaptureDisplay> {
        let mut state = self.state.lock().map_err(|_| SessionError::Worker)?;
        let now = Instant::now();
        self.expire(&mut state, now);
        Ok(self.display(&state, now))
    }
}
fn unknown(sequence: u64, epoch: u64) -> AgentCaptureDisplay {
    AgentCaptureDisplay {
        sequence,
        connection_epoch: epoch,
        known: false,
        active_grant_count: 0,
        active_capture: false,
        remaining_ms: 0,
    }
}
fn validate_value(known: bool, count: u32, active: bool, remaining: u32) -> SessionResult<()> {
    if count > 64
        || remaining > MAX_LIFETIME_MS
        || (!known && (count != 0 || active || remaining != 0))
        || (count == 0 && remaining != 0)
        || (count != 0 && remaining == 0)
    {
        return Err(SessionError::Invalid);
    }
    Ok(())
}
fn validate_wire(value: &pb::AgentCaptureStatus) -> SessionResult<()> {
    if value.encoded_len() > 64 || value.nonce == 0 {
        return Err(SessionError::Invalid);
    }
    if value.probe {
        if value.sequence != 0
            || value.known
            || value.active_grant_count != 0
            || value.active_capture
            || value.remaining_ms != 0
        {
            return Err(SessionError::Invalid);
        }
    } else {
        if value.sequence == 0 {
            return Err(SessionError::Invalid);
        }
        validate_value(
            value.known,
            value.active_grant_count,
            value.active_capture,
            value.remaining_ms,
        )?;
    }
    Ok(())
}
#[uniffi::export]
impl LiveSession {
    pub fn capture_grant_status(&self) -> SessionResult<AgentCaptureDisplay> {
        self.agent_capture.snapshot()
    }
    pub fn invalidate_capture_grant_status(&self) -> SessionResult<()> {
        let entered_epoch = self.epoch.load(Ordering::Acquire);
        if self.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        self.agent_capture.invalidate(entered_epoch)
    }
    pub fn publish_capture_grant_status(
        &self,
        active_grant_count: u32,
        active_capture: bool,
        remaining_ms: u32,
    ) -> SessionResult<()> {
        let entered_epoch = self.epoch.load(Ordering::Acquire);
        if self.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        self.agent_capture.set(
            entered_epoch,
            active_grant_count,
            active_capture,
            remaining_ms,
            Instant::now(),
        )
    }
    pub async fn wait_capture_grant_status(
        &self,
        after_sequence: u64,
    ) -> SessionResult<AgentCaptureDisplay> {
        let _slot = self
            .agent_capture
            .waiters
            .clone()
            .try_acquire_owned()
            .map_err(|_| SessionError::Backpressure)?;
        let mut changed = self.agent_capture.signal.subscribe();
        loop {
            let value = self.agent_capture.snapshot()?;
            if value.sequence > after_sequence {
                return Ok(value);
            }
            if self.closed.load(Ordering::Acquire) {
                return Err(SessionError::Closed);
            }
            changed.changed().await.map_err(|_| SessionError::Closed)?;
        }
    }
}
#[cfg(test)]
#[path = "agent_capture_tests.rs"]
mod tests;
