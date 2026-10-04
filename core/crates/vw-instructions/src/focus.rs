use crate::{DocumentView, Error, MAX_FOCUS_BYTES, Result, SourceBinding};
use serde::{Deserialize, Serialize};
use vw_model::{DeviceId, Id};

pub const FOCUS_TARGET_MS: u64 = 200;
pub const FOCUS_PENDING_TTL_MS: u64 = 1000;

/// Ephemeral application payload; this is not a new protocol message or a
/// grant. The carrier adapter supplies the authenticated peer/connection epoch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusEvent {
    schema: u32,
    source: SourceBinding,
    peer: DeviceId,
    epoch: Id,
    sequence: u64,
    marker_id: Id,
    instruction_id: Id,
}
impl FocusEvent {
    pub fn new(
        view: &DocumentView<'_>,
        peer: DeviceId,
        epoch: Id,
        sequence: u64,
        marker: &Id,
    ) -> Result<Self> {
        Ok(Self {
            schema: 1,
            source: view.binding().clone(),
            peer,
            epoch,
            sequence,
            marker_id: marker.clone(),
            instruction_id: view.marker_instruction(marker)?.clone(),
        })
    }
    pub fn to_json(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self).map_err(|_| Error::Invalid("focus serialization"))?;
        if bytes.len() > MAX_FOCUS_BYTES {
            return Err(Error::Limit("focus payload"));
        }
        Ok(bytes)
    }
}
#[derive(Clone, Debug)]
pub struct FocusTicket {
    event: FocusEvent,
    admission: u64,
    received_at_ms: u64,
    pub delivery_elapsed_ms: u64,
}
impl FocusTicket {
    pub fn marker_id(&self) -> &Id {
        &self.event.marker_id
    }
    pub fn instruction_id(&self) -> &Id {
        &self.event.instruction_id
    }
    pub fn source(&self) -> &SourceBinding {
        &self.event.source
    }
    /// Local receive-to-ready timing only. It does not measure the required
    /// phone-placement-to-PC-field latency or include UI dispatch/network time.
    pub fn within_local_target(&self) -> bool {
        self.delivery_elapsed_ms <= FOCUS_TARGET_MS
    }
}
#[derive(Clone, Debug)]
pub enum FocusOutcome {
    Apply(Box<FocusTicket>),
    Deferred,
    Ignored,
    Expired,
}

/// One latest pending focus per authenticated peer/connection; no unbounded
/// history or cross-reconnect queue. A new carrier epoch requires a new follower.
pub struct FocusFollower {
    peer: DeviceId,
    epoch: Id,
    project: Id,
    document: Id,
    enabled: bool,
    last_event: Option<FocusEvent>,
    pending: Option<(FocusEvent, u64)>,
    admission: u64,
    last_clock: Option<u64>,
}
impl FocusFollower {
    pub fn new(view: &DocumentView<'_>, peer: DeviceId, epoch: Id) -> Self {
        Self {
            peer,
            epoch,
            project: view.binding().project_id.clone(),
            document: view.binding().document_id.clone(),
            enabled: true,
            last_event: None,
            pending: None,
            admission: 0,
            last_clock: None,
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<()> {
        self.invalidate()?;
        self.enabled = enabled;
        Ok(())
    }
    /// Call on local selection/field ownership changes and on project disposal.
    pub fn invalidate(&mut self) -> Result<()> {
        self.admission = self.admission.checked_add(1).ok_or(Error::Exhausted)?;
        self.pending = None;
        Ok(())
    }
    pub fn receive(
        &mut self,
        bytes: &[u8],
        authenticated: &DeviceId,
        view: &DocumentView<'_>,
        now_ms: u64,
    ) -> Result<FocusOutcome> {
        if bytes.len() > MAX_FOCUS_BYTES {
            return Err(Error::Limit("focus payload"));
        }
        let event: FocusEvent =
            serde_json::from_slice(bytes).map_err(|_| Error::Invalid("focus payload"))?;
        if event.schema != 1
            || authenticated != &self.peer
            || event.peer != self.peer
            || event.epoch != self.epoch
            || event.source.project_id != self.project
            || event.source.document_id != self.document
        {
            return Err(Error::Unauthorized);
        }
        self.view_identity(view)?;
        self.clock(now_ms)?;
        if let Some(last) = &self.last_event {
            if event.sequence < last.sequence {
                return Ok(FocusOutcome::Ignored);
            }
            if event.sequence == last.sequence {
                return if &event == last {
                    Ok(FocusOutcome::Ignored)
                } else {
                    Err(Error::Invalid("focus sequence reused"))
                };
            }
        }
        if event.source == *view.binding() {
            validate_target(&event, view)?;
        }
        self.invalidate()?;
        self.last_event = Some(event.clone());
        if !self.enabled || event.source.host_seq < view.binding().host_seq {
            return Ok(FocusOutcome::Ignored);
        }
        self.pending = Some((event, now_ms));
        self.poll(view, now_ms)
    }
    pub fn poll(&mut self, view: &DocumentView<'_>, now_ms: u64) -> Result<FocusOutcome> {
        self.view_identity(view)?;
        self.clock(now_ms)?;
        let Some((event, received)) = &self.pending else {
            return Ok(FocusOutcome::Ignored);
        };
        let elapsed = now_ms
            .checked_sub(*received)
            .ok_or(Error::Invalid("monotonic clock"))?;
        if elapsed > FOCUS_PENDING_TTL_MS {
            self.pending = None;
            return Ok(FocusOutcome::Expired);
        }
        if event.source.host_seq < view.binding().host_seq {
            self.pending = None;
            return Ok(FocusOutcome::Ignored);
        }
        if event.source != *view.binding() {
            return Ok(FocusOutcome::Deferred);
        }
        if let Err(error) = validate_target(event, view) {
            self.pending = None;
            return Err(error);
        }
        let ticket = FocusTicket {
            event: event.clone(),
            admission: self.admission,
            received_at_ms: *received,
            delivery_elapsed_ms: elapsed,
        };
        self.pending = None;
        Ok(FocusOutcome::Apply(Box::new(ticket)))
    }
    /// Recheck after asynchronous UI dispatch; newer focus, local selection,
    /// revision change or reconnect must invalidate a delayed callback.
    pub fn is_current(&self, ticket: &FocusTicket, view: &DocumentView<'_>, now_ms: u64) -> bool {
        self.enabled
            && ticket.admission == self.admission
            && ticket.event.epoch == self.epoch
            && ticket.event.peer == self.peer
            && self.last_event.as_ref() == Some(&ticket.event)
            && ticket.event.source == *view.binding()
            && validate_target(&ticket.event, view).is_ok()
            && now_ms
                .checked_sub(ticket.received_at_ms)
                .is_some_and(|age| age <= FOCUS_PENDING_TTL_MS)
    }
    fn view_identity(&self, view: &DocumentView<'_>) -> Result<()> {
        if view.binding().project_id != self.project || view.binding().document_id != self.document
        {
            return Err(Error::Stale);
        }
        Ok(())
    }
    fn clock(&mut self, now: u64) -> Result<()> {
        if self.last_clock.is_some_and(|last| now < last) {
            return Err(Error::Invalid("monotonic clock"));
        }
        self.last_clock = Some(now);
        Ok(())
    }
}
fn validate_target(event: &FocusEvent, view: &DocumentView<'_>) -> Result<()> {
    if view.marker_instruction(&event.marker_id)? != &event.instruction_id {
        return Err(Error::Reconciliation);
    }
    Ok(())
}
