//! Authenticated, connection-local marker focus. CONTROL carries only a bounded
//! hint; serialized canonical validation is required before exposing a target.
use super::{LiveSession, SessionError, SessionResult};
use crate::{
    Cancellation,
    workflow::{self, WorkflowBinding, WorkflowError, WorkflowResult},
};
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};
use tokio::sync::watch;
use vw_model::{Id, StateHash};
use vw_proto::Message;
use vw_proto::v1::{self as pb, envelope::Body};

const MAX_BYTES: usize = 256;
const TTL: Duration = Duration::from_secs(2);
const SEND_INTERVAL: Duration = Duration::from_millis(50);
const RECEIVE_BURST: f64 = 40.0;

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FocusSignal {
    pub sequence: u64,
    pub connection_epoch: u64,
    pub available: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PeerMarkerFocus {
    pub binding: WorkflowBinding,
    pub marker_id: String,
    pub instruction_id: String,
    pub sequence: u64,
    pub connection_epoch: u64,
}
#[derive(Clone)]
struct Pending {
    binding: WorkflowBinding,
    marker: Option<Id>,
}
#[derive(Clone)]
struct Received {
    value: Pending,
    sequence: u64,
    observed: Instant,
}
struct State {
    epoch: u64,
    stopped: bool,
    sent: u64,
    received: u64,
    issued: u64,
    outbound: Option<Pending>,
    incoming: Option<Received>,
    next_send: Instant,
    tokens: f64,
    refilled: Instant,
    pending_signal: bool,
}
pub(super) struct FocusHub {
    project: Id,
    state: Mutex<State>,
    signal: watch::Sender<FocusSignal>,
}
pub(super) struct FocusEpoch {
    hub: Arc<FocusHub>,
    epoch: u64,
}
impl Drop for FocusEpoch {
    fn drop(&mut self) {
        self.hub.deactivate(self.epoch);
    }
}
impl FocusHub {
    pub(super) fn new(project: Id) -> Arc<Self> {
        let now = Instant::now();
        let (signal, _) = watch::channel(FocusSignal {
            sequence: 1,
            connection_epoch: 0,
            available: false,
        });
        Arc::new(Self {
            project,
            signal,
            state: Mutex::new(State {
                epoch: 0,
                stopped: false,
                sent: 0,
                received: 0,
                issued: 0,
                outbound: None,
                incoming: None,
                next_send: now,
                tokens: RECEIVE_BURST,
                refilled: now,
                pending_signal: false,
            }),
        })
    }
    fn publish(&self, epoch: u64, available: bool) {
        let next = self.signal.borrow().sequence.saturating_add(1);
        self.signal.send_replace(FocusSignal {
            sequence: next,
            connection_epoch: epoch,
            available,
        });
    }
    pub(super) fn activate(self: &Arc<Self>, epoch: u64) -> SessionResult<FocusEpoch> {
        let mut state = self.state.lock().map_err(|_| SessionError::Worker)?;
        if state.stopped || epoch & 1 == 0 || epoch <= state.epoch {
            return Err(SessionError::Closed);
        }
        let now = Instant::now();
        *state = State {
            epoch,
            stopped: false,
            sent: 0,
            received: 0,
            issued: 0,
            outbound: None,
            incoming: None,
            next_send: now,
            tokens: RECEIVE_BURST,
            refilled: now,
            pending_signal: false,
        };
        self.publish(epoch, true);
        Ok(FocusEpoch {
            hub: self.clone(),
            epoch,
        })
    }
    fn deactivate(&self, epoch: u64) {
        if let Ok(mut state) = self.state.lock()
            && state.epoch == epoch
        {
            state.outbound = None;
            state.incoming = None;
            state.pending_signal = false;
            state.epoch = epoch.saturating_add(1);
            self.publish(state.epoch, false);
        }
    }
    pub(super) fn stop(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.stopped = true;
            state.outbound = None;
            state.incoming = None;
            state.pending_signal = false;
            if state.epoch & 1 != 0 {
                state.epoch = state.epoch.saturating_add(1);
            }
            self.publish(state.epoch, false);
        }
    }
    fn current(&self, epoch: u64) -> WorkflowResult<()> {
        let state = self.state.lock().map_err(|_| WorkflowError::Closed)?;
        if state.stopped {
            return Err(WorkflowError::Closed);
        }
        if epoch & 1 == 0 || state.epoch != epoch {
            return Err(WorkflowError::Stale);
        }
        Ok(())
    }
    fn issue(&self, epoch: u64) -> WorkflowResult<u64> {
        let mut state = self.state.lock().map_err(|_| WorkflowError::Closed)?;
        if state.stopped {
            return Err(WorkflowError::Closed);
        }
        if epoch & 1 == 0 || state.epoch != epoch {
            return Err(WorkflowError::Stale);
        }
        state.issued = state.issued.checked_add(1).ok_or(WorkflowError::Limit)?;
        Ok(state.issued)
    }
    fn queue(&self, epoch: u64, request: u64, value: Pending) -> WorkflowResult<()> {
        let mut state = self.state.lock().map_err(|_| WorkflowError::Closed)?;
        if state.stopped {
            return Err(WorkflowError::Closed);
        }
        if epoch & 1 == 0 || state.epoch != epoch || request != state.issued {
            return Err(WorkflowError::Stale);
        }
        if value.binding.project_id != self.project.as_str() {
            return Err(WorkflowError::Invalid);
        }
        state.outbound = Some(value); // Exactly one latest pending selection.
        Ok(())
    }
    pub(super) fn flush(&self, sender: &vw_net::ConnectionSender) -> SessionResult<()> {
        self.flush_at(Instant::now(), |value| {
            sender.enqueue(Body::MarkerFocus(value))
        })
    }
    fn expire(&self, state: &mut State, now: Instant) {
        if state
            .incoming
            .as_ref()
            .is_some_and(|v| now.saturating_duration_since(v.observed) >= TTL)
        {
            state.incoming = None;
            state.pending_signal = false;
            // Queries can retire a slot before the network tick. Publication is
            // part of retirement, so a discarded query cannot swallow expiry.
            self.publish(state.epoch, state.epoch & 1 != 0 && !state.stopped);
        }
    }
    fn publish_pending(&self, state: &mut State, now: Instant) {
        state.tokens = (state.tokens
            + now.saturating_duration_since(state.refilled).as_secs_f64() * 20.0)
            .min(RECEIVE_BURST);
        state.refilled = now;
        if state.pending_signal && state.tokens >= 1.0 {
            state.tokens -= 1.0;
            state.pending_signal = false;
            self.publish(state.epoch, true);
        }
    }
    fn flush_at(
        &self,
        now: Instant,
        send: impl FnOnce(pb::MarkerFocus) -> Result<(), vw_net::NetError>,
    ) -> SessionResult<()> {
        let mut state = self.state.lock().map_err(|_| SessionError::Worker)?;
        self.expire(&mut state, now);
        self.publish_pending(&mut state, now);
        if state.stopped || state.epoch & 1 == 0 || now < state.next_send {
            return Ok(());
        }
        let Some(pending) = state.outbound.as_ref() else {
            return Ok(());
        };
        let sequence = state.sent.checked_add(1).ok_or(SessionError::Invalid)?;
        let value = wire(pending, sequence).map_err(|_| SessionError::Invalid)?;
        // Reserve no unbounded retry queue and never let volatile backpressure
        // tear down a healthy reliable connection. New input replaces this slot.
        state.next_send = now + SEND_INTERVAL;
        match send(value) {
            Ok(()) => {
                state.outbound = None;
                state.sent = sequence;
                Ok(())
            }
            Err(vw_net::NetError::Backpressure) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    pub(super) fn receive(&self, value: pb::MarkerFocus, epoch: u64) -> SessionResult<()> {
        self.receive_at(value, epoch, Instant::now())
    }
    fn receive_at(&self, value: pb::MarkerFocus, epoch: u64, now: Instant) -> SessionResult<()> {
        // Identity, size and shape are checked even for stale sequence numbers.
        let sequence = value.sequence;
        let pending = parse(&self.project, value)?;
        let mut state = self.state.lock().map_err(|_| SessionError::Worker)?;
        if state.stopped || epoch & 1 == 0 || state.epoch != epoch {
            return Err(SessionError::Closed);
        }
        if sequence <= state.received {
            return Ok(());
        }
        state.received = sequence;
        state.incoming = Some(Received {
            value: pending,
            sequence,
            observed: now,
        });
        // Ordered reliable traffic may arrive in a burst after carrier stalls.
        // Keep one newest hint; rate-limit observer work, not link survival.
        state.pending_signal = true;
        self.publish_pending(&mut state, now);
        Ok(())
    }
    fn snapshot(&self, epoch: u64, now: Instant) -> WorkflowResult<Option<Received>> {
        let mut state = self.state.lock().map_err(|_| WorkflowError::Closed)?;
        if state.stopped {
            return Err(WorkflowError::Closed);
        }
        if state.epoch != epoch || epoch & 1 == 0 {
            return Err(WorkflowError::Stale);
        }
        self.expire(&mut state, now);
        Ok(state.incoming.clone())
    }
    fn unchanged(&self, epoch: u64, sequence: u64, now: Instant) -> WorkflowResult<bool> {
        Ok(self
            .snapshot(epoch, now)?
            .is_some_and(|v| v.sequence == sequence))
    }
}
fn wire(value: &Pending, sequence: u64) -> WorkflowResult<pb::MarkerFocus> {
    value.binding.validate()?;
    Ok(pb::MarkerFocus {
        project_id: Some(Id::try_from(value.binding.project_id.clone())?.to_proto()),
        document_id: Some(Id::try_from(value.binding.document_id.clone())?.to_proto()),
        revision: Some(pb::Revision {
            host_seq: value.binding.host_seq,
            state_hash: StateHash::try_from(value.binding.state_hash.clone())?
                .bytes()
                .to_vec(),
        }),
        marker_id: value.marker.as_ref().map(Id::to_proto),
        sequence,
    })
}
fn parse(project: &Id, value: pb::MarkerFocus) -> SessionResult<Pending> {
    if value.encoded_len() > MAX_BYTES || value.sequence == 0 {
        return Err(SessionError::Invalid);
    }
    let actual = Id::from_proto(value.project_id.as_ref()).map_err(|_| SessionError::Invalid)?;
    if &actual != project {
        return Err(SessionError::Authentication);
    }
    let document = Id::from_proto(value.document_id.as_ref()).map_err(|_| SessionError::Invalid)?;
    let revision = value.revision.ok_or(SessionError::Invalid)?;
    if revision.state_hash.len() != 32 {
        return Err(SessionError::Invalid);
    }
    let marker = value
        .marker_id
        .as_ref()
        .map(|v| Id::from_proto(Some(v)))
        .transpose()
        .map_err(|_| SessionError::Invalid)?;
    Ok(Pending {
        binding: WorkflowBinding {
            project_id: actual.to_string(),
            document_id: document.to_string(),
            host_seq: revision.host_seq,
            state_hash: super::hex(&revision.state_hash),
        },
        marker,
    })
}
fn target(
    project: &vw_model::Project,
    binding: &WorkflowBinding,
    marker: &Id,
) -> WorkflowResult<String> {
    if project.id.as_str() != binding.project_id {
        return Err(WorkflowError::Stale);
    }
    let revision = pb::Revision {
        host_seq: binding.host_seq,
        state_hash: StateHash::try_from(binding.state_hash.clone())?
            .bytes()
            .to_vec(),
    };
    let view = vw_instructions::DocumentView::new(
        project,
        &revision,
        &Id::try_from(binding.document_id.clone())?,
    )?;
    Ok(view.marker_instruction(marker)?.to_string())
}
#[uniffi::export]
impl LiveSession {
    /// Borrowed capability: this never changes canonical state or the camera.
    pub async fn send_marker_focus(
        &self,
        binding: WorkflowBinding,
        marker_id: Option<String>,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<()> {
        let entered_epoch = self.epoch.load(Ordering::Acquire);
        if self.closed.load(Ordering::Acquire) {
            return Err(WorkflowError::Closed);
        }
        let request = self.focus.issue(entered_epoch)?;
        binding.validate()?;
        if marker_id.as_ref().is_some_and(|v| v.len() != 36) {
            return Err(WorkflowError::Invalid);
        }
        let marker = marker_id.map(Id::try_from).transpose()?;
        let project = self.project.upgrade().ok_or(WorkflowError::Closed)?;
        project.check_open()?;
        let closed = project.closed.clone();
        let hub = self.focus.clone();
        project
            .worker
            .call(move |state| {
                Ok((|| {
                    workflow::check_live(&closed, &cancellation)?;
                    hub.current(entered_epoch)?;
                    workflow::admit(
                        state,
                        workflow::MAX_MEMORY,
                        false,
                        workflow::INSTRUCTION_WORK,
                    )?;
                    workflow::check_binding(state, &binding)?;
                    if let Some(id) = &marker {
                        target(state.project()?, &binding, id)?;
                    }
                    workflow::check_live(&closed, &cancellation)?;
                    hub.queue(entered_epoch, request, Pending { binding, marker })
                })())
            })
            .await?
    }
    /// Requery after a canonical UI revision publication as well as a focus
    /// signal: a CONTROL hint may legitimately arrive before its OPS revision.
    pub async fn peer_marker_focus(
        &self,
        expected: WorkflowBinding,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Option<PeerMarkerFocus>> {
        let entered_epoch = self.epoch.load(Ordering::Acquire);
        if self.closed.load(Ordering::Acquire) {
            return Err(WorkflowError::Closed);
        }
        expected.validate()?;
        let Some(received) = self.focus.snapshot(entered_epoch, Instant::now())? else {
            return Ok(None);
        };
        if received.value.binding != expected {
            return Ok(None);
        }
        let Some(marker) = received.value.marker.clone() else {
            return Ok(None);
        };
        let project = self.project.upgrade().ok_or(WorkflowError::Closed)?;
        project.check_open()?;
        let closed = project.closed.clone();
        let hub = self.focus.clone();
        project
            .worker
            .call(move |state| {
                Ok((|| {
                    workflow::check_live(&closed, &cancellation)?;
                    if !hub.unchanged(entered_epoch, received.sequence, Instant::now())? {
                        return Ok(None);
                    }
                    workflow::admit(
                        state,
                        workflow::MAX_MEMORY,
                        false,
                        workflow::INSTRUCTION_WORK,
                    )?;
                    workflow::check_binding(state, &expected)?;
                    let instruction_id = target(state.project()?, &expected, &marker)?;
                    workflow::check_live(&closed, &cancellation)?;
                    if !hub.unchanged(entered_epoch, received.sequence, Instant::now())? {
                        return Ok(None);
                    }
                    Ok(Some(PeerMarkerFocus {
                        binding: expected,
                        marker_id: marker.to_string(),
                        instruction_id,
                        sequence: received.sequence,
                        connection_epoch: entered_epoch,
                    }))
                })())
            })
            .await?
    }
    pub fn focus_signal(&self) -> FocusSignal {
        self.focus.signal.borrow().clone()
    }
    pub async fn wait_focus(&self, after_sequence: u64) -> SessionResult<FocusSignal> {
        if self.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        let mut signal = self.focus.signal.subscribe();
        if signal.borrow().sequence > after_sequence {
            return Ok(signal.borrow().clone());
        }
        self.runtime
            .call(async move {
                match tokio::time::timeout(Duration::from_secs(30), signal.changed()).await {
                    Ok(Ok(())) | Err(_) => {}
                    Ok(Err(_)) => return Err(SessionError::Closed),
                }
                let latest = signal.borrow().clone();
                Ok(latest)
            })
            .await
    }
}

#[cfg(test)]
#[path = "focus_tests.rs"]
mod tests;
