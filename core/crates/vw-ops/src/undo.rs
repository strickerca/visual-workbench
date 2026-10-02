use crate::{Acceptance, OpsError};
use std::collections::{BTreeMap, BTreeSet};
use vw_model::{DeviceId, Id};
use vw_proto::{Message, v1};

#[derive(Debug, Clone, PartialEq, Eq)]
enum EventKind {
    Ordinary,
    Inverse { redo: bool, target: Id },
}

#[derive(Debug, Clone)]
struct Event {
    bytes: Vec<u8>,
    kind: EventKind,
    ack: Option<v1::TxnAck>,
    local_order: Option<u64>,
}

/// Per-device, per-project undo history. Acknowledged events follow host order,
/// regardless of delivery order. Explicitly registered pending events advance the
/// local cursor immediately, allowing an offline edit, undo and redo to be queued.
/// Register pending events only after the replica has successfully queued them.
#[derive(Debug, Clone)]
pub struct UndoManager {
    device: DeviceId,
    project: Option<Id>,
    undo: Vec<Id>,
    redo: Vec<Id>,
    events: BTreeMap<Id, Event>,
    next_local_order: u64,
}

impl UndoManager {
    pub fn new(device: DeviceId) -> Self {
        Self {
            device,
            project: None,
            undo: Vec::new(),
            redo: Vec::new(),
            events: BTreeMap::new(),
            next_local_order: 0,
        }
    }

    /// Record an ordinary accepted transaction belonging to this device. Exact
    /// retries are harmless, including a retry received before the original ack.
    pub fn record(
        &mut self,
        transaction: &v1::Transaction,
        acceptance: &Acceptance,
    ) -> Result<(), OpsError> {
        let kind = ordinary(transaction)?;
        self.register(transaction, kind, Some(acceptance))
    }

    /// Register a successfully queued ordinary transaction before its ack. A
    /// later ack replaces its provisional ordering without adding a second entry.
    pub fn record_pending(&mut self, transaction: &v1::Transaction) -> Result<(), OpsError> {
        let kind = ordinary(transaction)?;
        self.register(transaction, kind, None)
    }

    /// Register a successfully queued undo or redo and advance its cursor now.
    /// Registration is atomic and requires the target currently offered by
    /// `operation`; an exact retry never advances the cursor twice.
    pub fn record_pending_inverse(
        &mut self,
        redo: bool,
        transaction: &v1::Transaction,
    ) -> Result<(), OpsError> {
        let kind = inverse(redo, transaction)?;
        self.register(transaction, kind, None)
    }

    /// Build an inverse operation for the current cursor. The caller supplies a
    /// new transaction ID, base revision and persisted Lamport counter.
    pub fn operation(&self, redo: bool, lamport: u64) -> Result<v1::Op, OpsError> {
        if lamport == 0 {
            return Err(OpsError::Invalid("Lamport counter"));
        }
        let target = if redo {
            self.redo.last()
        } else {
            self.undo.last()
        }
        .ok_or(OpsError::NotFound)?;
        Ok(v1::Op {
            op_id: Some(v1::OpId {
                device_id: self.device.to_string(),
                lamport,
            }),
            kind: Some(v1::op::Kind::UndoTransaction(v1::UndoTransaction {
                target_txn_id: Some(target.to_proto()),
            })),
        })
    }

    /// Record an accepted inverse, even when its target's acknowledgement has
    /// not arrived. Journal replay reconciles the cursor when missing receipts
    /// arrive. Redo targets the inverse's actual host-approved effects.
    pub fn acknowledge(
        &mut self,
        redo: bool,
        transaction: &v1::Transaction,
        acceptance: &Acceptance,
    ) -> Result<(), OpsError> {
        let kind = inverse(redo, transaction)?;
        self.register(transaction, kind, Some(acceptance))
    }

    /// Discard an explicitly rejected unacknowledged transaction and pending
    /// inverses that depend on it. Return removed IDs so the caller can remove
    /// the same entries from its replica queue. Accepted history cannot be erased.
    pub fn discard_pending(&mut self, id: &Id) -> Result<Vec<Id>, OpsError> {
        let event = self.events.get(id).ok_or(OpsError::NotFound)?;
        if event.ack.is_some() {
            return Err(OpsError::Invalid("accepted undo history"));
        }
        let mut removed = BTreeSet::from([id.clone()]);
        loop {
            let previous = removed.len();
            for (candidate, event) in &self.events {
                if let EventKind::Inverse { target, .. } = &event.kind
                    && removed.contains(target)
                {
                    if event.ack.is_some() {
                        return Err(OpsError::Invalid("accepted inverse dependency"));
                    }
                    removed.insert(candidate.clone());
                }
            }
            if removed.len() == previous {
                break;
            }
        }
        let mut candidate = self.clone();
        candidate.events.retain(|id, _| !removed.contains(id));
        candidate.rebuild()?;
        *self = candidate;
        Ok(removed.into_iter().collect())
    }

    fn register(
        &mut self,
        transaction: &v1::Transaction,
        kind: EventKind,
        acceptance: Option<&Acceptance>,
    ) -> Result<(), OpsError> {
        let (id, project) = self.check(transaction, acceptance)?;
        let bytes = transaction.encode_to_vec();
        if let Some(previous) = self.events.get(&id) {
            if previous.bytes != bytes || previous.kind != kind {
                return Err(OpsError::IdCollision);
            }
            match (previous.ack.as_ref(), acceptance) {
                (Some(ack), Some(acceptance)) if ack != &acceptance.ack => {
                    return Err(OpsError::IdCollision);
                }
                (Some(_), _) | (None, None) => return Ok(()),
                (None, Some(_)) => {}
            }
        } else if acceptance.is_none()
            && let EventKind::Inverse { redo, target } = &kind
        {
            let source = if *redo { &self.redo } else { &self.undo };
            if source.last() != Some(target) {
                return Err(OpsError::Invalid("undo cursor changed"));
            }
        }

        let mut candidate = self.clone();
        candidate.project = Some(project);
        if let Some(event) = candidate.events.get_mut(&id) {
            event.ack = acceptance.map(|value| value.ack.clone());
        } else {
            let local_order = if acceptance.is_none() {
                candidate.next_local_order = candidate
                    .next_local_order
                    .checked_add(1)
                    .ok_or(OpsError::CounterExhausted)?;
                Some(candidate.next_local_order)
            } else {
                None
            };
            candidate.events.insert(
                id,
                Event {
                    bytes,
                    kind,
                    ack: acceptance.map(|value| value.ack.clone()),
                    local_order,
                },
            );
        }
        candidate.rebuild()?;
        *self = candidate;
        Ok(())
    }

    fn check(
        &self,
        transaction: &v1::Transaction,
        acceptance: Option<&Acceptance>,
    ) -> Result<(Id, Id), OpsError> {
        if transaction.device_id != self.device.as_str() {
            return Err(OpsError::UndoOwnership);
        }
        let id = Id::from_proto(transaction.txn_id.as_ref())?;
        let project = Id::from_proto(transaction.project_id.as_ref())?;
        if self
            .project
            .as_ref()
            .is_some_and(|previous| previous != &project)
        {
            return Err(OpsError::Invalid("undo project"));
        }
        if transaction.ops.is_empty() || transaction.ops.len() > 4096 {
            return Err(OpsError::Invalid("operation count"));
        }
        let mut lamports = BTreeSet::new();
        for operation in &transaction.ops {
            let identity = operation
                .op_id
                .as_ref()
                .ok_or(OpsError::Invalid("operation identity"))?;
            if identity.device_id != self.device.as_str()
                || identity.lamport == 0
                || !lamports.insert(identity.lamport)
                || operation.kind.is_none()
            {
                return Err(OpsError::Invalid("operation identity"));
            }
        }
        if let Some(acceptance) = acceptance {
            if transaction.txn_id != acceptance.ack.txn_id
                || acceptance.ack.host_seq == 0
                || acceptance.ack.state_hash.len() != 32
            {
                return Err(OpsError::Invalid("acknowledgement identity"));
            }
            if self.events.iter().any(|(previous_id, event)| {
                previous_id != &id
                    && event
                        .ack
                        .as_ref()
                        .is_some_and(|ack| ack.host_seq == acceptance.ack.host_seq)
            }) {
                return Err(OpsError::Invalid("duplicate host sequence"));
            }
        }
        Ok((id, project))
    }

    fn rebuild(&mut self) -> Result<(), OpsError> {
        self.validate_dependencies()?;
        // Preserve known local queue order while receipts are incomplete. If the
        // host explicitly sequences independent local requests differently,
        // authoritative order wins over provisional local ordering.
        let order = self
            .ordered_events(true)
            .or_else(|| self.ordered_events(false))
            .ok_or(OpsError::Invalid("inverse history cycle"))?;
        let mut undo = Vec::new();
        let mut redo = Vec::new();
        for id in order {
            let event = self.events.get(&id).ok_or(OpsError::NotFound)?;
            match &event.kind {
                EventKind::Ordinary => {
                    undo.push(id);
                    redo.clear();
                }
                EventKind::Inverse {
                    redo: is_redo,
                    target,
                } => {
                    let source = if *is_redo { &mut redo } else { &mut undo };
                    // A receipt can arrive before an earlier undo or its target.
                    // Removing the named known target keeps unrelated receipts
                    // usable; replay fills gaps deterministically.
                    if let Some(position) = source.iter().position(|value| value == target) {
                        source.remove(position);
                        if *is_redo {
                            undo.push(id);
                        } else {
                            redo.push(id);
                        }
                    }
                }
            }
        }
        self.undo = undo;
        self.redo = redo;
        Ok(())
    }

    fn validate_dependencies(&self) -> Result<(), OpsError> {
        for (id, event) in &self.events {
            if let EventKind::Inverse { redo, target } = &event.kind {
                if id == target {
                    return Err(OpsError::Invalid("inverse self reference"));
                }
                if let Some(previous) = self.events.get(target) {
                    let valid_kind = matches!(
                        (redo, &previous.kind),
                        (false, EventKind::Ordinary)
                            | (false, EventKind::Inverse { redo: true, .. })
                            | (true, EventKind::Inverse { redo: false, .. })
                    );
                    if !valid_kind {
                        return Err(OpsError::Invalid("inverse direction"));
                    }
                    if let (Some(before), Some(after)) = (&previous.ack, &event.ack)
                        && before.host_seq >= after.host_seq
                    {
                        return Err(OpsError::Invalid("inverse host order"));
                    }
                }
            }
        }
        Ok(())
    }

    fn ordered_events(&self, local_edges: bool) -> Option<Vec<Id>> {
        let mut outgoing = BTreeMap::<Id, BTreeSet<Id>>::new();
        let mut incoming: BTreeMap<Id, usize> =
            self.events.keys().cloned().map(|id| (id, 0)).collect();
        let mut edge = |before: &Id, after: &Id| {
            if outgoing
                .entry(before.clone())
                .or_default()
                .insert(after.clone())
                && let Some(count) = incoming.get_mut(after)
            {
                *count += 1;
            }
        };
        let mut acknowledged: Vec<_> = self
            .events
            .iter()
            .filter_map(|(id, event)| event.ack.as_ref().map(|ack| (ack.host_seq, id)))
            .collect();
        acknowledged.sort_unstable();
        for adjacent in acknowledged.windows(2) {
            edge(adjacent[0].1, adjacent[1].1);
        }
        for (id, event) in &self.events {
            if let EventKind::Inverse { target, .. } = &event.kind
                && self.events.contains_key(target)
            {
                edge(target, id);
            }
        }
        if local_edges {
            let mut local: Vec<_> = self
                .events
                .iter()
                .filter_map(|(id, event)| event.local_order.map(|order| (order, id)))
                .collect();
            local.sort_unstable();
            for adjacent in local.windows(2) {
                if self.events.get(adjacent[0].1)?.ack.is_none()
                    || self.events.get(adjacent[1].1)?.ack.is_none()
                {
                    edge(adjacent[0].1, adjacent[1].1);
                }
            }
        }
        let priority = |id: &Id| -> Option<(bool, u64, Id)> {
            let event = self.events.get(id)?;
            Some(match &event.ack {
                Some(ack) => (false, ack.host_seq, id.clone()),
                None => (true, event.local_order?, id.clone()),
            })
        };
        let mut ready = BTreeSet::new();
        for (id, count) in &incoming {
            if *count == 0 {
                ready.insert(priority(id)?);
            }
        }
        let mut ordered = Vec::with_capacity(self.events.len());
        while let Some((_, _, id)) = ready.pop_first() {
            if let Some(successors) = outgoing.get(&id) {
                for next in successors {
                    let count = incoming.get_mut(next)?;
                    *count = count.checked_sub(1)?;
                    if *count == 0 {
                        ready.insert(priority(next)?);
                    }
                }
            }
            ordered.push(id);
        }
        (ordered.len() == self.events.len()).then_some(ordered)
    }
}

fn ordinary(transaction: &v1::Transaction) -> Result<EventKind, OpsError> {
    if transaction
        .ops
        .iter()
        .any(|op| matches!(op.kind, Some(v1::op::Kind::UndoTransaction(_))))
    {
        return Err(OpsError::Invalid("inverse history registration"));
    }
    Ok(EventKind::Ordinary)
}

fn inverse(redo: bool, transaction: &v1::Transaction) -> Result<EventKind, OpsError> {
    let [
        v1::Op {
            kind: Some(v1::op::Kind::UndoTransaction(value)),
            ..
        },
    ] = transaction.ops.as_slice()
    else {
        return Err(OpsError::Invalid("inverse transaction"));
    };
    Ok(EventKind::Inverse {
        redo,
        target: Id::from_proto(value.target_txn_id.as_ref())?,
    })
}
