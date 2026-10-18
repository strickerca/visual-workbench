use crate::{
    BudgetedBytes, Cancellation, DEFAULT_RING_FRAMES, FrameIdentity, MAX_RING_BYTES, MemoryCharge,
    MemoryKind, Result, SessionBudget, StreamError, StreamGeometry, rgba_bytes,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

// Includes Arc/control block, Vec/header, fixed IDs and bounded geometry label.
const FRAME_OVERHEAD: u64 = 4096;
#[derive(Debug)]
struct Pool {
    used: AtomicU64,
    limit: u64,
}
#[derive(Debug)]
struct Reservation {
    pool: Arc<Pool>,
    bytes: u64,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.pool.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
impl Pool {
    fn reserve(self: &Arc<Self>, bytes: u64) -> Result<Reservation> {
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.limit)
            })
            .map_err(|_| StreamError::Limit)?;
        Ok(Reservation {
            pool: self.clone(),
            bytes,
        })
    }
}

struct Pixels {
    identity: FrameIdentity,
    rgba: Vec<u8>,
    _reservation: Reservation,
    charge: MemoryCharge,
}
/// Cheap immutable pin. A pin continues charging the original ring pool after
/// eviction, so abandoned freeze/export readers cannot bypass the hard cap.
#[derive(Clone)]
pub struct OriginalFrame(Arc<Pixels>);
impl std::fmt::Debug for OriginalFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OriginalFrame")
            .field("identity", self.identity())
            .field("charged_bytes", &self.0.charge.bytes())
            .finish_non_exhaustive()
    }
}
impl OriginalFrame {
    pub fn identity(&self) -> &FrameIdentity {
        &self.0.identity
    }
    pub fn rgba(&self) -> &[u8] {
        &self.0.rgba
    }
    pub fn belongs_to(&self, budget: &SessionBudget) -> bool {
        self.0.charge.belongs_to(budget)
    }
    pub fn copy_into_transfer(
        &self,
        budget: &SessionBudget,
        cancel: &Cancellation,
    ) -> Result<BudgetedBytes> {
        if !self.belongs_to(budget) {
            return Err(StreamError::Invalid);
        }
        BudgetedBytes::copy_from(budget, MemoryKind::Transfer, self.rgba(), cancel)
    }
}

/// Capture adapters fill this already-admitted allocation directly. Dropping
/// before publication releases the reservation. No caller Vec is cloned.
pub struct FrameWriter {
    identity: FrameIdentity,
    rgba: Vec<u8>,
    reservation: Reservation,
    charge: MemoryCharge,
}
impl FrameWriter {
    pub fn rgba_mut(&mut self) -> &mut [u8] {
        &mut self.rgba
    }
    pub fn finish(self, cancel: &Cancellation) -> Result<OriginalFrame> {
        crate::frame::opaque(
            &self.rgba,
            self.identity.geometry.width(),
            self.identity.geometry.height(),
            cancel,
        )?;
        Ok(OriginalFrame(Arc::new(Pixels {
            identity: self.identity,
            rgba: self.rgba,
            _reservation: self.reservation,
            charge: self.charge,
        })))
    }
}

/// Reserve before a platform CPU readback which already returns one owned Vec.
/// This avoids allocating/copying a second full frame in the capture adapter.
pub struct FrameReservation {
    identity: FrameIdentity,
    bytes: usize,
    reservation: Reservation,
    charge: MemoryCharge,
}
impl FrameReservation {
    pub fn allocate(self) -> Result<FrameWriter> {
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(self.bytes)
            .map_err(|_| StreamError::Limit)?;
        rgba.resize(self.bytes, 0);
        Ok(FrameWriter {
            identity: self.identity,
            rgba,
            reservation: self.reservation,
            charge: self.charge,
        })
    }
    pub fn adopt(self, rgba: Vec<u8>, cancel: &Cancellation) -> Result<OriginalFrame> {
        if rgba.len() != self.bytes || rgba.capacity() > self.bytes {
            return Err(StreamError::Limit);
        }
        FrameWriter {
            identity: self.identity,
            rgba,
            reservation: self.reservation,
            charge: self.charge,
        }
        .finish(cancel)
    }
}

/// Eight originals by default, never more than 300 MB including leased/evicted
/// originals. Call begin_frame before CPU readback; caller/GPU/codec memory is a
/// separate explicit workspace, not hidden in this retained-original budget.
pub struct FrameRing {
    pool: Arc<Pool>,
    budget: SessionBudget,
    source: StreamGeometry,
    session: vw_model::Id,
    capacity: usize,
    frames: VecDeque<OriginalFrame>,
    last_frame: u64,
    last_timestamp: i64,
}
impl FrameRing {
    /// Isolated compatibility ring with a private pool. Production capture uses
    /// new_budgeted and the existing endpoint pool.
    pub fn new(
        session: vw_model::Id,
        source: StreamGeometry,
        capacity: usize,
        limit: u64,
    ) -> Result<Self> {
        // Isolated compatibility utility. Production callers must supply the
        // endpoint's shared pool with new_budgeted/default_budgeted_for.
        Self::new_budgeted(session, source, capacity, limit, SessionBudget::new(limit)?)
    }
    pub fn new_budgeted(
        session: vw_model::Id,
        source: StreamGeometry,
        capacity: usize,
        limit: u64,
        budget: SessionBudget,
    ) -> Result<Self> {
        let charge = rgba_bytes(source.width(), source.height())? as u64 + FRAME_OVERHEAD;
        if !(DEFAULT_RING_FRAMES..=32).contains(&capacity)
            || limit > MAX_RING_BYTES
            || charge
                .checked_mul(capacity as u64)
                .is_none_or(|n| n > limit)
        {
            return Err(StreamError::Limit);
        }
        Ok(Self {
            pool: Arc::new(Pool {
                used: AtomicU64::new(0),
                limit,
            }),
            budget,
            source,
            session,
            capacity,
            frames: VecDeque::with_capacity(capacity),
            last_frame: 0,
            last_timestamp: 0,
        })
    }
    pub fn default_for(session: vw_model::Id, source: StreamGeometry) -> Result<Self> {
        Self::new(session, source, DEFAULT_RING_FRAMES, MAX_RING_BYTES)
    }
    pub fn default_budgeted_for(
        session: vw_model::Id,
        source: StreamGeometry,
        budget: SessionBudget,
    ) -> Result<Self> {
        Self::new_budgeted(session, source, DEFAULT_RING_FRAMES, MAX_RING_BYTES, budget)
    }
    pub fn used_bytes(&self) -> u64 {
        self.pool.used.load(Ordering::Acquire)
    }
    pub fn retained_frames(&self) -> usize {
        self.frames.len()
    }
    pub fn begin_frame(
        &mut self,
        identity: FrameIdentity,
        cancel: &Cancellation,
    ) -> Result<FrameWriter> {
        self.reserve_frame(identity, cancel)?.allocate()
    }
    pub fn reserve_frame(
        &mut self,
        identity: FrameIdentity,
        cancel: &Cancellation,
    ) -> Result<FrameReservation> {
        cancel.check()?;
        identity.validate()?;
        if identity.session != self.session
            || !identity.geometry.same_source(&self.source)
            || identity.frame_id <= self.last_frame
            || identity.timestamp_ns <= self.last_timestamp
        {
            return Err(StreamError::Stale);
        }
        let bytes = rgba_bytes(self.source.width(), self.source.height())?;
        let charge = bytes as u64 + FRAME_OVERHEAD;
        // Eviction is explicit and occurs before allocation; pinned frames stay
        // charged. Never purge every live ring entry in an attempt to beat pins.
        let reserve = || -> Result<(Reservation, MemoryCharge)> {
            let local = self.pool.reserve(charge)?;
            let shared = self.budget.reserve(MemoryKind::RingOriginal, charge)?;
            Ok((local, shared))
        };
        let (reservation, charge) = match reserve() {
            Ok(value) => value,
            Err(StreamError::Limit) if self.frames.len() == self.capacity => {
                self.frames.pop_front();
                reserve()?
            }
            Err(error) => return Err(error),
        };
        Ok(FrameReservation {
            identity,
            bytes,
            reservation,
            charge,
        })
    }
    pub fn publish(&mut self, frame: OriginalFrame) -> Result<()> {
        let id = frame.identity();
        if !Arc::ptr_eq(&frame.0._reservation.pool, &self.pool)
            || id.session != self.session
            || !id.geometry.same_source(&self.source)
            || id.frame_id <= self.last_frame
            || id.timestamp_ns <= self.last_timestamp
        {
            return Err(StreamError::Stale);
        }
        self.last_frame = id.frame_id;
        self.last_timestamp = id.timestamp_ns;
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
        Ok(())
    }
    pub fn pin(&self, session: &vw_model::Id, frame_id: u64) -> Result<OriginalFrame> {
        if session != &self.session {
            return Err(StreamError::Stale);
        }
        self.frames
            .iter()
            .find(|frame| frame.identity().frame_id == frame_id)
            .cloned()
            .ok_or(StreamError::Unavailable)
    }
}
