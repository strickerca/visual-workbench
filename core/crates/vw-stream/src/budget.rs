use crate::{Cancellation, Result, StreamError};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// Admission caps, not measurements of process RSS or platform codec peaks.
pub const PHONE_SESSION_BYTES: u64 = 256 * 1024 * 1024;
pub const DESKTOP_SESSION_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    RingOriginal,
    CodecWorkspace,
    EncodedTiles,
    DisplayFrame,
    VerifiedOriginal,
    Transfer,
    Foreign,
}

#[derive(Debug)]
struct Pool {
    used: AtomicU64,
    limit: u64,
    parent: Option<SessionBudget>,
}

/// All clones share one hard admission pool. The active endpoint supplies this
/// same value to capture, codecs, transport, rendering and foreign adapters.
#[derive(Clone, Debug)]
pub struct SessionBudget(Arc<Pool>);
impl SessionBudget {
    pub fn new(limit_bytes: u64) -> Result<Self> {
        if !(1..=DESKTOP_SESSION_BYTES).contains(&limit_bytes) {
            return Err(StreamError::Limit);
        }
        Ok(Self(Arc::new(Pool {
            used: AtomicU64::new(0),
            limit: limit_bytes,
            parent: None,
        })))
    }
    /// Independent endpoint identity with allocations also charged to a shared
    /// process pool. Retained pixels keep both charges after endpoint shutdown.
    pub fn child(&self, limit_bytes: u64) -> Result<Self> {
        if limit_bytes == 0 || limit_bytes > self.limit_bytes() {
            return Err(StreamError::Limit);
        }
        Ok(Self(Arc::new(Pool {
            used: AtomicU64::new(0),
            limit: limit_bytes,
            parent: Some(self.clone()),
        })))
    }
    /// Reserve before allocating or asking a platform codec/readback to allocate.
    /// A zero-byte charge is permitted for an empty payload.
    pub fn reserve(&self, kind: MemoryKind, bytes: u64) -> Result<MemoryCharge> {
        self.0
            .used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.0.limit)
            })
            .map_err(|_| StreamError::Limit)?;
        let parent = match &self.0.parent {
            Some(parent) => match parent.reserve(kind, bytes) {
                Ok(charge) => Some(Box::new(charge)),
                Err(error) => {
                    self.0.used.fetch_sub(bytes, Ordering::AcqRel);
                    return Err(error);
                }
            },
            None => None,
        };
        Ok(MemoryCharge {
            budget: self.clone(),
            bytes,
            kind,
            _parent: parent,
        })
    }
    pub fn used_bytes(&self) -> u64 {
        self.0.used.load(Ordering::Acquire)
    }
    pub fn limit_bytes(&self) -> u64 {
        self.0.limit
    }
    pub fn same_pool(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Linear reservation. Keep this inside the actual payload owner (inside the
/// shared Arc for shared pixels); drop it only after payload retirement. It has
/// no manual credit operation and cannot be cloned.
#[derive(Debug)]
pub struct MemoryCharge {
    budget: SessionBudget,
    bytes: u64,
    kind: MemoryKind,
    _parent: Option<Box<MemoryCharge>>,
}
impl MemoryCharge {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn kind(&self) -> MemoryKind {
        self.kind
    }
    pub fn belongs_to(&self, budget: &SessionBudget) -> bool {
        self.budget.same_pool(budget)
    }
    /// Move the same physical allocation to another owner without releasing any
    /// admission credit. A physical copy requires a second reservation instead.
    pub fn reclassify(mut self, kind: MemoryKind) -> Self {
        self.kind = kind;
        self
    }
}
impl Drop for MemoryCharge {
    fn drop(&mut self) {
        self.budget.0.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// A pre-admitted byte allocation for wire/BLOB or foreign CPU copies. Ownership
/// moves together with its charge; sharing this value in Arc keeps the charge
/// until the final reader/late completion has retired.
pub struct BudgetedBytes {
    bytes: Vec<u8>,
    charge: MemoryCharge,
}
impl BudgetedBytes {
    pub fn allocate(
        budget: &SessionBudget,
        kind: MemoryKind,
        len: usize,
        cancel: &Cancellation,
    ) -> Result<Self> {
        cancel.check()?;
        let charge = budget.reserve(kind, len as u64)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(len)
            .map_err(|_| StreamError::Limit)?;
        bytes.resize(len, 0);
        cancel.check()?;
        Ok(Self { bytes, charge })
    }
    pub fn copy_from(
        budget: &SessionBudget,
        kind: MemoryKind,
        source: &[u8],
        cancel: &Cancellation,
    ) -> Result<Self> {
        let mut owner = Self::allocate(budget, kind, source.len(), cancel)?;
        owner.bytes.copy_from_slice(source);
        cancel.check()?;
        Ok(owner)
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Filling an already admitted allocation cannot change its size/capacity.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
    pub fn charge(&self) -> &MemoryCharge {
        &self.charge
    }
    pub fn into_transfer(self) -> Self {
        Self {
            bytes: self.bytes,
            charge: self.charge.reclassify(MemoryKind::Transfer),
        }
    }
    /// Adopt an allocation whose capacity was reserved before its producer ran.
    /// A mismatched capacity fails while retaining the charge through Vec drop.
    pub fn from_parts(bytes: Vec<u8>, charge: MemoryCharge) -> Result<Self> {
        if bytes.capacity() as u64 > charge.bytes() {
            drop(bytes);
            drop(charge);
            return Err(StreamError::Limit);
        }
        Ok(Self { bytes, charge })
    }
}

impl AsRef<[u8]> for BudgetedBytes {
    fn as_ref(&self) -> &[u8] {
        self.bytes()
    }
}
