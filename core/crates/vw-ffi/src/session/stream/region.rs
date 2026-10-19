use super::hub::Endpoint;
use crate::session::{SessionError, SessionResult};
use std::sync::Arc;
use vw_model::Id;
use vw_proto::v1::{self as pb, envelope::Body, region_capture_control::Body as RegionBody};
const TTL: u64 = 30_000;
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct CaptureRegionSignal {
    pub sequence: u64,
    pub connection_epoch: u64,
    pub request_id: Option<String>,
    pub outgoing_request_id: Option<String>,
    pub status: String,
}
impl CaptureRegionSignal {
    pub(super) fn empty(epoch: u64) -> Self {
        Self {
            sequence: 1,
            connection_epoch: epoch,
            request_id: None,
            outgoing_request_id: None,
            status: String::new(),
        }
    }
}
pub(super) struct Mailbox {
    endpoint: Arc<Endpoint>,
    prompt: Option<(Id, u64)>,
    outgoing: Option<(Id, u64)>,
    last_sequence: u64,
    last_request: Option<u64>,
    next_sequence: u64,
    pending: Option<pb::RegionCaptureControl>,
}
impl Mailbox {
    pub fn new(endpoint: Arc<Endpoint>) -> Self {
        Self {
            endpoint,
            prompt: None,
            outgoing: None,
            last_sequence: 0,
            last_request: None,
            next_sequence: 1,
            pending: None,
        }
    }
    fn queue(&mut self, id: &Id, body: RegionBody) -> SessionResult<()> {
        if self.pending.is_some() {
            return Err(SessionError::Backpressure);
        }
        let sequence = self.next_sequence;
        self.next_sequence = sequence.checked_add(1).ok_or(SessionError::Backpressure)?;
        self.pending = Some(pb::RegionCaptureControl {
            request_id: Some(id.to_proto()),
            sequence,
            body: Some(body),
        });
        Ok(())
    }
    fn end(reason: &str) -> RegionBody {
        RegionBody::End(pb::RegionCaptureEnd {
            reason: reason.into(),
        })
    }
    fn publish(&self, status: &str) {
        let request_id = self.prompt.as_ref().map(|(id, _)| id.to_string());
        let outgoing_request_id = self.outgoing.as_ref().map(|(id, _)| id.to_string());
        self.endpoint.region.send_if_modified(|value| {
            if value.request_id == request_id
                && value.outgoing_request_id == outgoing_request_id
                && value.status == status
            {
                return false;
            }
            let Some(sequence) = value.sequence.checked_add(1) else {
                return false;
            };
            value.sequence = sequence;
            value.request_id = request_id;
            value.outgoing_request_id = outgoing_request_id;
            value.status = status.into();
            true
        });
    }
    pub fn request(&mut self, id: Id, now: u64) -> SessionResult<()> {
        self.tick(now)?;
        if self.outgoing.is_some() {
            return Err(SessionError::Backpressure);
        }
        let deadline = now.checked_add(TTL).ok_or(SessionError::Invalid)?;
        self.queue(&id, RegionBody::Request(pb::RegionCaptureRequest {}))?;
        self.outgoing = Some((id, deadline));
        self.publish("waiting_for_computer");
        Ok(())
    }
    pub fn resolve(&mut self, id: &Id, approve: bool, now: u64) -> SessionResult<()> {
        self.tick(now)?;
        if self
            .prompt
            .as_ref()
            .is_none_or(|(expected, _)| expected != id)
        {
            return Err(SessionError::Invalid);
        }
        self.queue(
            id,
            Self::end(if approve { "picker_opened" } else { "declined" }),
        )?;
        self.prompt = None;
        self.publish(if approve { "picker_opened" } else { "declined" });
        Ok(())
    }
    // Caller is the sole connected loop after channel/capability and envelope
    // admission. Endpoint/epoch are captured from that authenticated carrier.
    pub fn receive(&mut self, value: pb::RegionCaptureControl, now: u64) -> SessionResult<()> {
        self.tick(now)?;
        let id = Id::from_proto(value.request_id.as_ref()).map_err(|_| SessionError::Invalid)?;
        if value.sequence == 0 {
            return Err(SessionError::Invalid);
        }
        if value.sequence <= self.last_sequence {
            return Ok(());
        }
        self.last_sequence = value.sequence;
        match value.body.ok_or(SessionError::Invalid)? {
            RegionBody::Request(_) => {
                if self.prompt.is_some()
                    || self
                        .last_request
                        .is_some_and(|last| now.saturating_sub(last) < 1000)
                {
                    return Ok(());
                }
                self.prompt = Some((id, now.checked_add(TTL).ok_or(SessionError::Invalid)?));
                self.last_request = Some(now);
                self.publish("computer_consent_required");
            }
            RegionBody::End(end) => {
                if !matches!(
                    end.reason.as_str(),
                    "declined"
                        | "cancelled"
                        | "expired"
                        | "unavailable"
                        | "invalid"
                        | "picker_opened"
                ) {
                    return Err(SessionError::Invalid);
                }
                if self
                    .outgoing
                    .as_ref()
                    .is_some_and(|(expected, _)| expected == &id)
                {
                    self.outgoing = None;
                    self.publish(&end.reason)
                }
                if self
                    .prompt
                    .as_ref()
                    .is_some_and(|(expected, _)| expected == &id)
                {
                    self.prompt = None;
                    self.publish(&end.reason)
                }
            }
            // This product route asks the computer to choose the region. Remote
            // offer/choice/result authority is not admitted by this UI/API.
            _ => {
                if self.pending.is_none() {
                    self.queue(&id, Self::end("unavailable"))?;
                }
            }
        }
        Ok(())
    }
    pub fn tick(&mut self, now: u64) -> SessionResult<()> {
        if self
            .prompt
            .as_ref()
            .is_some_and(|(_, deadline)| now >= *deadline)
        {
            if self.pending.is_none() {
                let id = self.prompt.as_ref().ok_or(SessionError::Worker)?.0.clone();
                self.queue(&id, Self::end("expired"))?;
            }
            self.prompt = None;
            self.publish("expired");
        }
        if self
            .outgoing
            .as_ref()
            .is_some_and(|(_, deadline)| now >= *deadline)
        {
            self.outgoing = None;
            self.publish("expired");
        }
        Ok(())
    }
    pub fn flush(&mut self, sender: &vw_net::ConnectionSender) -> SessionResult<()> {
        if let Some(value) = self.pending.take() {
            match sender.enqueue(Body::RegionCaptureControl(value.clone())) {
                Ok(()) => {}
                Err(vw_net::NetError::Backpressure) => self.pending = Some(value),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
    pub fn stop(&mut self) {
        self.prompt = None;
        self.outgoing = None;
        self.pending = None;
        self.publish("closed");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    fn id(v: u8) -> Id {
        Id::from_parts(1_700_000_000_000, [v; 10]).unwrap()
    }
    fn request(value: u8, seq: u64) -> pb::RegionCaptureControl {
        pb::RegionCaptureControl {
            request_id: Some(id(value).to_proto()),
            sequence: seq,
            body: Some(RegionBody::Request(pb::RegionCaptureRequest {})),
        }
    }
    #[test]
    fn phone_request_is_only_a_prompt_and_requires_exact_local_resolution() {
        let hub = super::super::hub::Hub::new();
        let (endpoint, mut commands) = hub.activate(1).unwrap();
        let mut box_ = Mailbox::new(endpoint.clone());
        box_.receive(request(1, 1), 100).unwrap();
        assert_eq!(endpoint.region.borrow().request_id, Some(id(1).to_string()));
        assert!(commands.try_recv().is_err());
        assert!(box_.pending.is_none());
        assert!(box_.resolve(&id(2), true, 110).is_err());
        box_.resolve(&id(1), true, 110).unwrap();
        assert!(endpoint.region.borrow().request_id.is_none());
        let RegionBody::End(end) = box_.pending.as_ref().unwrap().body.as_ref().unwrap() else {
            panic!("end")
        };
        assert_eq!(end.reason, "picker_opened");
    }
    #[test]
    fn pending_prompt_cannot_be_replaced_by_replay_or_rate_limited_request() {
        let hub = super::super::hub::Hub::new();
        let (endpoint, _) = hub.activate(1).unwrap();
        let mut box_ = Mailbox::new(endpoint.clone());
        box_.receive(request(1, 1), 0).unwrap();
        box_.receive(request(2, 2), 5).unwrap();
        assert_eq!(endpoint.region.borrow().request_id, Some(id(1).to_string()));
        box_.resolve(&id(1), false, 10).unwrap();
        box_.pending = None;
        box_.receive(request(2, 2), 1200).unwrap();
        assert!(endpoint.region.borrow().request_id.is_none());
        box_.receive(request(2, 3), 1200).unwrap();
        assert_eq!(endpoint.region.borrow().request_id, Some(id(2).to_string()));
    }
    #[test]
    fn consent_expires_at_fixed_thirty_seconds_and_never_opens_a_source() {
        let hub = super::super::hub::Hub::new();
        let (endpoint, mut commands) = hub.activate(1).unwrap();
        let mut box_ = Mailbox::new(endpoint.clone());
        box_.receive(request(1, 1), 10).unwrap();
        box_.tick(30009).unwrap();
        assert!(endpoint.region.borrow().request_id.is_some());
        box_.tick(30010).unwrap();
        assert!(endpoint.region.borrow().request_id.is_none());
        assert!(box_.resolve(&id(1), true, 30010).is_err());
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn stop_releases_only_prompt_and_pending_wire_and_new_epoch_starts_empty() {
        let hub = super::super::hub::Hub::new();
        let (endpoint, _) = hub.activate(1).unwrap();
        let mut box_ = Mailbox::new(endpoint.clone());
        box_.request(id(1), 0).unwrap();
        box_.receive(request(2, 1), 0).unwrap();
        box_.stop();
        let state = endpoint.region.borrow();
        assert!(state.request_id.is_none());
        assert!(state.outgoing_request_id.is_none());
        assert_eq!(state.status, "closed");
        assert!(box_.pending.is_none());
        drop(state);
        endpoint.done.send_replace(Some(Ok(())));
        let (next, _) = hub.activate(3).unwrap();
        assert!(next.region.borrow().request_id.is_none());
        assert_eq!(next.region.borrow().connection_epoch, 3);
    }
    #[test]
    fn remote_rectangle_choice_has_no_source_authority() {
        let hub = super::super::hub::Hub::new();
        let (endpoint, mut commands) = hub.activate(1).unwrap();
        let mut box_ = Mailbox::new(endpoint.clone());
        box_.receive(
            pb::RegionCaptureControl {
                request_id: Some(id(1).to_proto()),
                sequence: 1,
                body: Some(RegionBody::Choose(pb::RegionCaptureChoice::default())),
            },
            0,
        )
        .unwrap();
        assert!(commands.try_recv().is_err());
        assert!(endpoint.region.borrow().request_id.is_none());
        let RegionBody::End(end) = box_.pending.as_ref().unwrap().body.as_ref().unwrap() else {
            panic!("end")
        };
        assert_eq!(end.reason, "unavailable");
    }
}
