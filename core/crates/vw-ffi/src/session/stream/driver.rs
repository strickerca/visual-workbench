//! Sole connected-loop owner. Codec waits and retirement are select branches;
//! neither a saturated foreign queue nor a closing decoder blocks OPS traffic.
use super::{
    api::{CaptureBackground, CaptureDraw, StreamSignal, StreamStage},
    hub::{Command, Endpoint, Lease},
    lossless::{Lossless, Offer},
    order::Source,
    publication::{self, Publication},
    receiver::StreamReceiver,
    verified::Original,
};
use crate::session::{SessionError, SessionResult};
use std::{
    future::{Future, pending},
    pin::Pin,
    sync::{Arc, Weak, atomic::Ordering},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use vw_net::ConnectionSender;
use vw_proto::v1::{self as pb, envelope::Body};
use vw_stream::{FreezePhase, FreezeQuality};

type Retirement = Pin<Box<dyn Future<Output = SessionResult<()>> + Send>>;
pub(in crate::session) struct Event(EventKind);
enum EventKind {
    Command(Option<Command>),
    Decode(SessionResult<bool>),
    Retired(SessionResult<()>),
    Lossless(super::lossless::Event),
    FlowRetired(SessionResult<()>),
    Publication(publication::Event),
    PublicationRetired(SessionResult<()>),
    Wake,
}
pub(in crate::session) struct Bridge {
    endpoint: Arc<Endpoint>,
    commands: mpsc::Receiver<Command>,
    lease: Option<Weak<Lease>>,
    receiver: Option<StreamReceiver>,
    retiring: Option<Retirement>,
    source: Option<Source>,
    highest_source: u64,
    source_available: bool,
    failure: Option<String>,
    last_repair: Option<Instant>,
    finished: bool,
    frames: u64,
    pending_control: Option<pb::StreamControl>,
    region: super::region::Mailbox,
    flow: Option<Lossless>,
    flow_retiring: Option<Retirement>,
    original: Option<Arc<Original>>,
    publisher: Option<Publication>,
    publisher_retiring: Option<Retirement>,
    pending_publication_close: Option<pb::StreamControl>,
}
impl Bridge {
    #[cfg(test)]
    pub(super) fn test_publication(&mut self) -> Option<&mut Publication> {
        self.publisher.as_mut()
    }

    pub fn new(endpoint: Arc<Endpoint>, commands: mpsc::Receiver<Command>) -> Self {
        let region = super::region::Mailbox::new(endpoint.clone());
        Self {
            region,
            endpoint,
            commands,
            lease: None,
            receiver: None,
            retiring: None,
            source: None,
            highest_source: 0,
            source_available: false,
            failure: None,
            last_repair: None,
            finished: false,
            frames: 0,
            pending_control: None,
            flow: None,
            flow_retiring: None,
            original: None,
            publisher: None,
            publisher_retiring: None,
            pending_publication_close: None,
        }
    }
    pub fn is_stopped(&self) -> bool {
        self.endpoint.stop.load(Ordering::Acquire)
    }
    /// Cancellation of this waiting future keeps both the decoder result and
    /// exact retirement future in this owner, ready for the next select poll.
    pub async fn next(&mut self) -> Event {
        let Self {
            endpoint,
            commands,
            receiver,
            retiring,
            flow,
            flow_retiring,
            publisher,
            publisher_retiring,
            ..
        } = self;
        Event(tokio::select! {
            command=commands.recv()=>EventKind::Command(command),
            result=async{match receiver.as_mut(){Some(value)if value.has_decode()=>value.complete_decode().await,_=>pending().await}}=>EventKind::Decode(result),
            result=async{match retiring.as_mut(){Some(value)=>value.as_mut().await,None=>pending().await}}=>EventKind::Retired(result),
            result=async{match flow.as_mut(){Some(value)=>value.next().await,None=>pending().await}}=>EventKind::Lossless(result),
            result=async{match flow_retiring.as_mut(){Some(value)=>value.as_mut().await,None=>pending().await}}=>EventKind::FlowRetired(result),
            result=async{match publisher.as_mut(){Some(value)if value.has_work()=>value.next().await,_=>pending().await}}=>EventKind::Publication(result),
            result=async{match publisher_retiring.as_mut(){Some(value)=>value.as_mut().await,None=>pending().await}}=>EventKind::PublicationRetired(result),
            _=endpoint.wake.notified()=>EventKind::Wake,
        })
    }
    pub fn handle(&mut self, event: Event, now_ms: u64) -> SessionResult<()> {
        match event.0 {
            EventKind::Command(Some(command)) => self.command(command, now_ms)?,
            EventKind::Command(None) => self.endpoint.stop(),
            EventKind::Decode(Ok(true)) => {
                self.frames = self
                    .frames
                    .checked_add(1)
                    .ok_or(SessionError::Backpressure)?;
            }
            EventKind::Decode(Ok(false)) => {}
            EventKind::Decode(Err(error)) => self.failure = Some(failure(&error).into()),
            EventKind::Retired(result) => {
                self.retiring = None;
                if let Err(error) = result {
                    self.failure = Some(failure(&error).into());
                }
            }
            EventKind::FlowRetired(result) => {
                self.flow_retiring = None;
                if let Err(error) = result {
                    self.failure = Some(failure(&error).into());
                }
            }
            EventKind::Publication(event) => self
                .publisher
                .as_mut()
                .ok_or(SessionError::Worker)?
                .handle(event)?,
            EventKind::PublicationRetired(result) => {
                self.publisher_retiring = None;
                if let Err(error) = result {
                    self.failure = Some(failure(&error).into());
                }
            }
            EventKind::Lossless(event) => {
                let result = self
                    .flow
                    .as_mut()
                    .ok_or(SessionError::Closed)?
                    .handle(event);
                match result {
                    Ok(Some(original)) => {
                        let request = self.flow.as_ref().ok_or(SessionError::Closed)?.request();
                        if self
                            .receiver
                            .as_mut()
                            .ok_or(SessionError::Closed)?
                            .lossless(request, original.pixels().clone())?
                        {
                            self.original = Some(Arc::new(original));
                        }
                    }
                    Ok(None) => {}
                    Err(error) => self.fail_flow(&error),
                }
            }
            EventKind::Wake => {}
        }
        self.maintenance(now_ms)?;
        self.publish();
        Ok(())
    }
    fn live_lease(&self, id: u64) -> SessionResult<Arc<Lease>> {
        let lease = self
            .lease
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or(SessionError::Closed)?;
        if lease.id != id {
            return Err(SessionError::Closed);
        }
        lease.check()?;
        Ok(lease)
    }
    fn command(&mut self, command: Command, now_ms: u64) -> SessionResult<()> {
        match command {
            Command::Publication(command) => self.publication_command(command)?,
            Command::RegionRequest { id, reply } => {
                if !reply.is_closed() {
                    let result = self.region.request(id, now_ms);
                    let _ = reply.send(result);
                }
            }
            Command::RegionResolve { id, approve, reply } => {
                if !reply.is_closed() {
                    let result = self.region.resolve(&id, approve, now_ms);
                    let _ = reply.send(result);
                }
            }
            Command::Enable {
                lease,
                budget,
                reply,
            } => {
                let Some(owner) = lease.upgrade() else {
                    return Ok(());
                };
                if reply.is_closed() || owner.check().is_err() {
                    let _ = owner.done.send_replace(Some(Ok(())));
                    return Ok(());
                }
                let result = (|| {
                    if owner.endpoint.epoch != self.endpoint.epoch
                        || !Arc::ptr_eq(&owner.endpoint, &self.endpoint)
                    {
                        return Err(SessionError::Closed);
                    }
                    if self.receiver.is_some() || self.retiring.is_some() {
                        return Err(SessionError::Backpressure);
                    }
                    self.receiver = Some(StreamReceiver::new(self.endpoint.epoch, budget)?);
                    self.lease = Some(lease);
                    if self.source_available
                        && let Some(source) = &self.source
                    {
                        self.receiver
                            .as_mut()
                            .ok_or(SessionError::Worker)?
                            .open(&source.control("start"))?;
                    }
                    self.failure = None;
                    Ok(())
                })();
                if result.is_err() {
                    if self
                        .lease
                        .as_ref()
                        .and_then(Weak::upgrade)
                        .is_some_and(|active| Arc::ptr_eq(&active, &owner))
                    {
                        self.retire();
                    } else {
                        let _ = owner.done.send_replace(Some(Ok(())));
                    }
                }
                if reply.send(result).is_err() {
                    owner.stop();
                }
            }
            Command::Draw { lease, reply } => {
                if reply.is_closed() {
                    return Ok(());
                }
                let result = (|| {
                    let owner = self.live_lease(lease)?;
                    let receiver = self.receiver.as_mut().ok_or(SessionError::Closed)?;
                    let generation = receiver.generation();
                    let Some(draw) = receiver.begin_draw()? else {
                        return Ok(None);
                    };
                    CaptureDraw::new(owner, generation, draw).map(Some)
                })();
                // If delivery loses its race, CaptureDraw::drop seals the view.
                // Never manufacture a receipt after handing a draw to the ABI.
                let _ = reply.send(result);
            }
            Command::Background { lease, reply } => {
                if reply.is_closed() {
                    return Ok(());
                }
                let result = (|| {
                    let owner = self.live_lease(lease)?;
                    let Some(frozen) = self.receiver.as_ref().and_then(StreamReceiver::frozen)
                    else {
                        return Ok(None);
                    };
                    let Some(request) = frozen.request() else {
                        return Ok(None);
                    };
                    let frame = frozen.frame().ok_or(SessionError::Worker)?.clone();
                    CaptureBackground::new(owner, request, frame, self.original.clone()).map(Some)
                })();
                // An undelivered immutable read has no external side effect.
                let _ = reply.send(result);
            }
            Command::Receipt {
                lease,
                generation,
                token,
                shown,
                settled,
                reply,
            } => {
                let result = self.live_lease(lease).and_then(|_| {
                    self.receiver
                        .as_mut()
                        .ok_or(SessionError::Closed)?
                        .draw_receipt(generation, &token, shown)
                });
                if result.is_ok() {
                    settled.store(true, Ordering::Release);
                }
                let _ = reply.send_replace(Some(result));
            }
            Command::Contact {
                lease,
                draft,
                reply,
            } => {
                if reply.is_closed() {
                    return Ok(());
                }
                let result = (|| {
                    self.live_lease(lease)?;
                    if self.flow.is_some() || self.flow_retiring.is_some() {
                        return Err(SessionError::Backpressure);
                    }
                    let receiver = self.receiver.as_mut().ok_or(SessionError::Closed)?;
                    let request = receiver.contact(draft, true, now_ms)?;
                    if let Some(request) = &request {
                        let stamp = receiver
                            .frozen()
                            .and_then(|v| v.frame())
                            .ok_or(SessionError::Worker)?
                            .stamp()
                            .clone();
                        match Lossless::new(request.clone(), stamp, receiver.budget().clone()) {
                            Ok(flow) => self.flow = Some(flow),
                            Err(error) => {
                                receiver.cancel_resume_after_draft_disposed();
                                return Err(error);
                            }
                        }
                    }
                    Ok(request)
                })();
                if result.as_ref().is_ok_and(|v| v.is_some()) {
                    self.pending_control = self.source.as_ref().map(|v| v.control("pause"));
                }
                if let Err(Ok(Some(_))) = reply.send(result) {
                    // No public receipt/gesture owner acquired this draft.
                    if let Some(receiver) = &mut self.receiver {
                        receiver.cancel_resume_after_draft_disposed();
                    }
                    self.retire_flow();
                    self.pending_control = self.source.as_ref().map(|v| v.control("resume"));
                }
            }
            Command::Resume { lease, reply } => {
                if reply.is_closed() {
                    return Ok(());
                }
                self.apply_saved()?;
                let result = self.live_lease(lease).and_then(|_| {
                    Ok(self
                        .receiver
                        .as_mut()
                        .ok_or(SessionError::Closed)?
                        .cancel_resume_after_draft_disposed())
                });
                if result.as_ref().is_ok_and(|v| v.is_some()) {
                    self.pending_control = self.source.as_ref().map(|v| v.control("resume"));
                    self.retire_flow();
                }
                let _ = reply.send(result);
            }
        }
        Ok(())
    }
    fn publication_command(&mut self, command: publication::Command) -> SessionResult<()> {
        match command {
            publication::Command::Enable {
                lease,
                source,
                budget,
                signal,
                reply,
            } => {
                let Some(owner) = lease.upgrade() else {
                    return Ok(());
                };
                if reply.is_closed() || owner.check().is_err() {
                    let _ = owner.done.send_replace(Some(Ok(())));
                    return Ok(());
                }
                let result = (|| {
                    if owner.endpoint.epoch != self.endpoint.epoch
                        || !Arc::ptr_eq(&owner.endpoint, &self.endpoint)
                    {
                        return Err(SessionError::Closed);
                    }
                    if self.publisher.is_some() || self.publisher_retiring.is_some() {
                        return Err(SessionError::Backpressure);
                    }
                    self.publisher = Some(Publication::new(lease, source, budget, signal)?);
                    Ok(())
                })();
                // new() returns only after owning a live worker. There are no
                // later fallible stages before its publication into this actor.
                if result.is_err() {
                    let _ = owner.done.send_replace(Some(Ok(())));
                }
                if reply.send(result).is_err() {
                    owner.stop();
                }
            }
            command => match &mut self.publisher {
                Some(publisher) => publisher.command(command)?,
                None => command.refuse(SessionError::Closed),
            },
        }
        Ok(())
    }
    /// Called only after authenticated capture_stream_v1 channel admission.
    /// Start is a source announcement, not local consent to allocate/display.
    pub fn control(&mut self, value: pb::StreamControl) -> SessionResult<()> {
        if value.action != "start"
            && let Some(publisher) = &mut self.publisher
            && publisher.control(&value)?
        {
            return Ok(());
        }
        if value.action == "start" {
            let source = Source::parse(&value)?;
            if source.generation < self.highest_source {
                return Ok(());
            }
            if source.generation == self.highest_source {
                if self.source.as_ref() != Some(&source) {
                    return Err(SessionError::Invalid);
                }
                return Ok(());
            }
            if let Some(receiver) = &mut self.receiver {
                if receiver
                    .frozen()
                    .is_some_and(|v| v.phase() != FreezePhase::Live)
                {
                    self.failure = Some("draft_requires_decision".into());
                    self.publish();
                    return Ok(());
                }
                receiver.open(&value)?;
            }
            self.highest_source = source.generation;
            self.source = Some(source);
            self.source_available = true;
            self.failure = None;
            self.pending_control = None;
        } else {
            let Some(source) = &self.source else {
                // Validate without admitting an unknown future close/open.
                if !matches!(
                    value.action.as_str(),
                    "pause" | "resume" | "stop" | "source_closed" | "keyframe"
                ) {
                    return Err(SessionError::Invalid);
                }
                let mut start = value;
                start.action = "start".into();
                Source::parse(&start)?;
                return Ok(());
            };
            if !source.matches_control(&value)? {
                return Ok(());
            }
            if !matches!(
                value.action.as_str(),
                "pause" | "resume" | "stop" | "source_closed"
            ) {
                return Err(SessionError::Invalid);
            }
            if let Some(receiver) = &mut self.receiver {
                receiver.control(&value)?;
            }
            if matches!(value.action.as_str(), "stop" | "source_closed") {
                self.source_available = false;
            }
        }
        self.publish();
        Ok(())
    }
    pub fn media(&mut self, value: pb::FrameTiles) -> SessionResult<()> {
        if let Some(receiver) = &mut self.receiver {
            receiver.media(value)?;
        } else {
            super::wire::frame_identity(&value)?;
        }
        Ok(())
    }
    pub fn lossless_offer(
        &mut self,
        value: pb::LosslessFrame,
        project: &Arc<crate::ProjectSession>,
    ) -> SessionResult<()> {
        let Some(flow) = &mut self.flow else {
            return super::lossless::admit_offer_shape(&value);
        };
        match flow.offer(&value)? {
            Offer::Ignored => {}
            Offer::Unavailable => {
                if let Some(receiver) = &mut self.receiver {
                    receiver.unavailable(flow.request());
                }
            }
            Offer::Create(offer) => {
                if let Err(error) = flow.prepare(&offer, project.clone()) {
                    self.fail_flow(&error);
                }
            }
        }
        self.publish();
        Ok(())
    }
    pub fn stream_blob(&mut self, value: pb::StreamBlob) -> SessionResult<()> {
        if let Some(publisher) = &mut self.publisher
            && publisher.blob(&value)?
        {
            return Ok(());
        }
        if let Some(flow) = &mut self.flow {
            if flow.receive(value)? {
                self.fail_flow(&SessionError::Cancelled);
            }
            Ok(())
        } else {
            super::wire::Scope::parse(value.key.as_ref().ok_or(SessionError::Invalid)?)?;
            if vw_proto::Message::encoded_len(&value) > 128 * 1024 - 1024 {
                return Err(SessionError::Backpressure);
            }
            Ok(())
        }
    }
    pub fn freeze_request(&mut self, value: pb::FreezeRequest, now_ms: u64) -> SessionResult<()> {
        match &mut self.publisher {
            Some(publisher) => publisher.freeze(&value, now_ms),
            None => super::publish_order::admit_freeze(&value),
        }
    }
    pub fn region_control(
        &mut self,
        value: pb::RegionCaptureControl,
        now_ms: u64,
    ) -> SessionResult<()> {
        self.region.receive(value, now_ms)
    }
    pub fn tick(&mut self, sender: &ConnectionSender, now_ms: u64) -> SessionResult<()> {
        self.region.flush(sender)?;
        self.maintenance(now_ms)?;
        // The old exact source closure is ordered before a new source opening.
        // A blocked CONTROL queue retains both; it cannot allocate a backlog.
        if let Some(control) = self.pending_publication_close.take() {
            match sender.enqueue(Body::StreamControl(control.clone())) {
                Ok(()) => {}
                Err(vw_net::NetError::Backpressure) => {
                    self.pending_publication_close = Some(control)
                }
                Err(error) => return Err(error.into()),
            }
        }
        if self.pending_publication_close.is_none()
            && let Some(publisher) = &mut self.publisher
        {
            publisher.tick(sender)?;
        }
        if let Some(control) = self.pending_control.take() {
            match sender.enqueue(Body::StreamControl(control.clone())) {
                Ok(()) => {}
                Err(vw_net::NetError::Backpressure) => self.pending_control = Some(control),
                Err(error) => return Err(error.into()),
            }
        }
        if let Some(flow) = &mut self.flow {
            flow.flush(sender)?;
        }
        if let Some(receiver) = &self.receiver
            && let Some(control) = receiver.repair_control()
            && self
                .last_repair
                .is_none_or(|last| last.elapsed() >= Duration::from_millis(250))
        {
            match sender.enqueue(Body::StreamControl(control)) {
                Ok(()) => self.last_repair = Some(Instant::now()),
                Err(vw_net::NetError::Backpressure) => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.publish();
        Ok(())
    }
    fn apply_saved(&mut self) -> SessionResult<()> {
        let Some(lease) = self.lease.as_ref().and_then(Weak::upgrade) else {
            return Ok(());
        };
        let mut notice = lease.saved.lock().map_err(|_| SessionError::Worker)?;
        let Some((draft, asset, generation)) = notice.as_ref() else {
            return Ok(());
        };
        if let Some(receiver) = &mut self.receiver
            && let Some(request) = receiver.frozen().and_then(|v| v.request()).cloned()
            && request.draft_id == *draft
            && request.epoch == *generation
            && match asset {
                Some(asset) => self
                    .original
                    .as_ref()
                    .is_some_and(|v| v.offer().asset == *asset && v.offer().frame == request.frame),
                None => receiver.frozen().is_some_and(|v| {
                    matches!(v.quality(), Some(FreezeQuality::DegradedCompressed))
                }),
            }
            && receiver
                .frozen()
                .is_none_or(|v| v.phase() != FreezePhase::Saved)
        {
            receiver.saved(&request)?;
        }
        *notice = None;
        Ok(())
    }
    fn maintenance(&mut self, now_ms: u64) -> SessionResult<()> {
        self.region.tick(now_ms)?;
        self.apply_saved()?;
        let stop = self.endpoint.stop.load(Ordering::Acquire)
            || self.lease.as_ref().is_some_and(|v| {
                v.upgrade()
                    .is_none_or(|v| v.closing.load(Ordering::Acquire))
            });
        if stop {
            self.retire();
        }
        if self.endpoint.stop.load(Ordering::Acquire)
            || self.publisher.as_ref().is_some_and(Publication::stopped)
        {
            self.retire_publication();
        }
        if let Some(receiver) = &mut self.receiver {
            receiver.tick(now_ms);
            if self.flow_retiring.is_none()
                && let Err(error) = receiver.start_decode()
            {
                self.failure = Some(failure(&error).into());
            }
            if let Some(flow) = &mut self.flow
                && let Err(error) = flow.advance(receiver)
            {
                self.fail_flow(&error);
            }
        }
        Ok(())
    }
    fn retire_publication(&mut self) {
        if let Some(mut publisher) = self.publisher.take() {
            self.pending_publication_close = Some(publisher.source_closed());
            self.publisher_retiring = Some(Box::pin(async move {
                let result = publisher.shutdown().await;
                let owner = publisher.completion();
                drop(publisher);
                if let Some(owner) = owner.upgrade() {
                    let _ = owner.done.send_replace(Some(result.clone()));
                }
                result
            }));
        }
    }
    fn retire(&mut self) {
        if let Some(mut receiver) = self.receiver.take() {
            let lease = self.lease.take();
            let mut flow = self.flow.take();
            let mut prior = self.flow_retiring.take();
            let original = self.original.take();
            self.retiring = Some(Box::pin(async move {
                let mut result = Ok(());
                if let Some(flow) = &mut flow {
                    result = flow.shutdown().await;
                }
                if let Some(prior) = &mut prior {
                    result = result.and(prior.as_mut().await);
                }
                result = result.and(receiver.shutdown().await);
                drop(flow);
                drop(prior);
                drop(original);
                drop(receiver);
                if let Some(lease) = lease.and_then(|v| v.upgrade()) {
                    let _ = lease.done.send_replace(Some(result.clone()));
                }
                result
            }));
        }
    }
    fn retire_flow(&mut self) {
        self.original = None;
        if let Some(mut flow) = self.flow.take() {
            self.flow_retiring = Some(Box::pin(async move { flow.shutdown().await }));
        }
    }
    fn fail_flow(&mut self, error: &SessionError) {
        self.failure = Some(failure(error).into());
        if let Some(flow) = &self.flow
            && let Some(receiver) = &mut self.receiver
        {
            receiver.unavailable(flow.request());
        }
        self.retire_flow();
    }
    fn publish(&self) {
        let mut value = StreamSignal::empty(self.endpoint.epoch);
        value.frame_sequence = self.frames;
        value.source_available =
            self.source_available && !self.endpoint.stop.load(Ordering::Acquire);
        if let Some(source) = &self.source {
            value.source_generation = source.generation;
            value.source_name = Some(source.name.clone());
            value.width = source.geometry.width();
            value.height = source.geometry.height();
        }
        value.stage = if self.endpoint.stop.load(Ordering::Acquire) {
            StreamStage::Closed
        } else if let Some(receiver) = &self.receiver {
            value.retained_bytes = receiver.retained_bytes();
            match receiver.frozen() {
                Some(frozen) if frozen.phase() == FreezePhase::Saved => StreamStage::Saved,
                Some(frozen) if frozen.phase() != FreezePhase::Live => match frozen.quality() {
                    Some(FreezeQuality::Lossless { .. }) => StreamStage::FrozenLossless,
                    Some(FreezeQuality::DegradedCompressed) => StreamStage::FrozenDegraded,
                    _ => StreamStage::Freezing,
                },
                _ if self.source_available => StreamStage::Live,
                _ if self.highest_source > 0 => StreamStage::SourceClosed,
                _ => StreamStage::AwaitingSource,
            }
        } else if self.highest_source > 0 && !self.source_available {
            StreamStage::SourceClosed
        } else {
            StreamStage::AwaitingSource
        };
        value.failure = self.failure.clone();
        self.endpoint.signal.send_if_modified(|prior| {
            value.sequence = prior.sequence;
            if *prior == value {
                return false;
            }
            let Some(sequence) = prior.sequence.checked_add(1) else {
                return false;
            };
            value.sequence = sequence;
            *prior = value;
            true
        });
    }
    pub async fn shutdown(&mut self) -> SessionResult<()> {
        self.endpoint.stop();
        self.region.stop();
        self.commands.close();
        // Refuse all undispatched requests and settle never-created leases.
        while let Ok(command) = self.commands.try_recv() {
            match command {
                Command::Enable { lease, .. } => {
                    if let Some(lease) = lease.upgrade() {
                        let _ = lease.done.send_replace(Some(Ok(())));
                    }
                }
                Command::Publication(command) => command.refuse(SessionError::Closed),
                _ => {}
            }
        }
        self.retire();
        self.retire_publication();
        let receive = match self.retiring.as_mut() {
            Some(value) => value.as_mut().await,
            None => Ok(()),
        };
        let publish = match self.publisher_retiring.as_mut() {
            Some(value) => value.as_mut().await,
            None => Ok(()),
        };
        let result = receive.and(publish);
        self.retiring = None;
        self.publisher_retiring = None;
        self.finished = true;
        self.publish();
        let _ = self.endpoint.done.send_replace(Some(result.clone()));
        result
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.endpoint.stop();
        if !self.finished {
            // Forced runtime destruction is not an orderly cleanup receipt.
            if let Some(lease) = self.lease.take().and_then(|v| v.upgrade()) {
                let _ = lease.done.send_replace(Some(Err(SessionError::Worker)));
            }
            if let Some(lease) = self
                .publisher
                .as_ref()
                .and_then(|v| v.completion().upgrade())
            {
                let _ = lease.done.send_replace(Some(Err(SessionError::Worker)));
            }
            let _ = self
                .endpoint
                .done
                .send_replace(Some(Err(SessionError::Worker)));
        }
    }
}
fn failure(error: &SessionError) -> &'static str {
    match error {
        SessionError::Backpressure => "stream_memory_limit",
        SessionError::Cancelled => "stream_cancelled",
        SessionError::Closed => "stream_closed",
        _ => "stream_decode_refused",
    }
}

#[cfg(test)]
#[path = "publication_tests.rs"]
mod publication_tests;
#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
