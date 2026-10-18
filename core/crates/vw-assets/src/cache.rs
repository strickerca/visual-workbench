use crate::{AssetError, Result};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub const PHONE_RAM_BYTES: u64 = 256 * 1024 * 1024;
pub const DESKTOP_RAM_BYTES: u64 = 512 * 1024 * 1024;
pub const PHONE_GPU_BYTES: u64 = 192 * 1024 * 1024;
pub const DESKTOP_GPU_BYTES: u64 = 384 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

struct Entry<T> {
    payload: T,
    bytes: u64,
}
struct Resident<T> {
    value: Arc<Entry<T>>,
    touched: u64,
    retired: bool,
}
struct Pending {
    id: u64,
    bytes: u64,
    epoch: u64,
}
struct State<K, T> {
    budget: u64,
    used: u64,
    reserved: u64,
    clock: u64,
    epoch: u64,
    entries: BTreeMap<K, Resident<T>>,
    pending: BTreeMap<K, Pending>,
}

/// Owns resources, including pinned display/upload leases, rather than merely
/// decrementing bookkeeping when an external handle still owns an allocation.
/// K should include the manifest identity as well as TileKey. T must own all
/// accounted allocation, and its destructor must release the actual resource.
/// GPU owners call reserve/clear/lease-drop on their graphics context thread.
pub struct BudgetCache<K, T> {
    state: Arc<Mutex<State<K, T>>>,
}
impl<K: Ord + Clone, T> BudgetCache<K, T> {
    pub fn new(budget: u64) -> Result<Self> {
        if budget == 0 || budget > 2 * 1024 * 1024 * 1024 {
            return Err(AssetError::Limit("cache budget"));
        }
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                budget,
                used: 0,
                reserved: 0,
                clock: 0,
                epoch: 0,
                entries: BTreeMap::new(),
                pending: BTreeMap::new(),
            })),
        })
    }
    pub fn accounting(&self) -> Result<(u64, u64)> {
        let s = self.state.lock().map_err(|_| AssetError::Poisoned)?;
        Ok((s.used, s.reserved))
    }
    pub fn get(&self, key: &K) -> Result<Option<CacheLease<K, T>>> {
        let mut s = self.state.lock().map_err(|_| AssetError::Poisoned)?;
        s.clock = s
            .clock
            .checked_add(1)
            .ok_or(AssetError::Limit("cache clock"))?;
        let clock = s.clock;
        Ok(s.entries.get_mut(key).filter(|e| !e.retired).map(|entry| {
            entry.touched = clock;
            CacheLease {
                value: Some(entry.value.clone()),
                state: self.state.clone(),
                key: key.clone(),
            }
        }))
    }
    /// Admission precedes decoder/upload allocation. Pinned and in-flight bytes
    /// both count. Failure may release ordinary LRU entries but never a lease.
    pub fn reserve(&self, key: K, bytes: u64) -> Result<Reservation<K, T>> {
        let mut s = self.state.lock().map_err(|_| AssetError::Poisoned)?;
        if bytes == 0 || bytes > s.budget {
            return Err(AssetError::Limit("cache item"));
        }
        if s.pending.contains_key(&key) || s.entries.contains_key(&key) {
            return Err(AssetError::Busy);
        }
        while s.used + s.reserved + bytes > s.budget
            || s.entries.len() + s.pending.len() >= MAX_ENTRIES
        {
            let oldest = s
                .entries
                .iter()
                .filter(|(_, e)| Arc::strong_count(&e.value) == 1)
                .min_by_key(|(_, e)| (u8::from(!e.retired), e.touched))
                .map(|(k, _)| k.clone());
            let Some(oldest) = oldest else {
                return Err(AssetError::Busy);
            };
            if let Some(entry) = s.entries.remove(&oldest) {
                let released = entry.value.bytes;
                drop(s);
                drop(entry);
                s = self.state.lock().map_err(|_| AssetError::Poisoned)?;
                s.used -= released;
                if s.pending.contains_key(&key) || s.entries.contains_key(&key) {
                    return Err(AssetError::Busy);
                }
            }
        }
        s.clock = s
            .clock
            .checked_add(1)
            .ok_or(AssetError::Limit("cache clock"))?;
        let id = s.clock;
        let epoch = s.epoch;
        s.reserved += bytes;
        s.pending.insert(key.clone(), Pending { id, bytes, epoch });
        drop(s);
        Ok(Reservation {
            state: self.state.clone(),
            key,
            id,
            settled: false,
        })
    }
    /// Invalidates queued upload/decode results; held frames remain counted until
    /// their leases are dropped. A new view never sees retired resources.
    pub fn clear(&self) -> Result<()> {
        let mut s = self.state.lock().map_err(|_| AssetError::Poisoned)?;
        s.epoch = s
            .epoch
            .checked_add(1)
            .ok_or(AssetError::Limit("cache epoch"))?;
        for entry in s.entries.values_mut() {
            entry.retired = true;
        }
        let keys: Vec<K> = s
            .entries
            .iter()
            .filter(|(_, e)| Arc::strong_count(&e.value) == 1)
            .map(|(k, _)| k.clone())
            .collect();
        let mut removed = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(entry) = s.entries.remove(&key) {
                removed.push(entry);
            }
        }
        let released: u64 = removed.iter().map(|e| e.value.bytes).sum();
        drop(s);
        drop(removed);
        self.state.lock().map_err(|_| AssetError::Poisoned)?.used -= released;
        Ok(())
    }
}

pub struct Reservation<K: Ord + Clone, T> {
    state: Arc<Mutex<State<K, T>>>,
    key: K,
    id: u64,
    settled: bool,
}
impl<K: Ord + Clone, T> Reservation<K, T> {
    /// bytes is the actual retained allocation, including platform row stride;
    /// it cannot exceed the reservation. Scratch needs its own reservation.
    pub fn commit(mut self, payload: T, bytes: u64) -> Result<CacheLease<K, T>> {
        let mut s = self.state.lock().map_err(|_| AssetError::Poisoned)?;
        let admission = match s.pending.get(&self.key) {
            Some(p) if p.id == self.id && p.epoch == s.epoch => {
                if bytes == 0 || bytes > p.bytes {
                    Err(AssetError::Limit("actual allocation"))
                } else {
                    Ok(p.bytes)
                }
            }
            _ => Err(AssetError::Stale),
        };
        let allocated = match admission {
            Ok(bytes) => bytes,
            Err(error) => {
                drop(s);
                drop(payload);
                return Err(error);
            }
        };
        s.pending.remove(&self.key);
        s.reserved -= allocated;
        s.used += bytes;
        let value = Arc::new(Entry { payload, bytes });
        let touched = s.clock;
        s.entries.insert(
            self.key.clone(),
            Resident {
                value: value.clone(),
                touched,
                retired: false,
            },
        );
        self.settled = true;
        Ok(CacheLease {
            value: Some(value),
            state: self.state.clone(),
            key: self.key.clone(),
        })
    }
}
impl<K: Ord + Clone, T> Drop for Reservation<K, T> {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let mut s = match self.state.lock() {
            Ok(v) => v,
            Err(e) => e.into_inner(),
        };
        if s.pending.get(&self.key).is_some_and(|p| p.id == self.id) {
            if let Some(p) = s.pending.remove(&self.key) {
                s.reserved -= p.bytes;
            }
        }
    }
}
pub struct CacheLease<K: Ord + Clone, T> {
    value: Option<Arc<Entry<T>>>,
    state: Arc<Mutex<State<K, T>>>,
    key: K,
}
impl<K: Ord + Clone, T> CacheLease<K, T> {
    pub fn payload(&self) -> Result<&T> {
        self.value
            .as_ref()
            .map(|v| &v.payload)
            .ok_or(AssetError::Stale)
    }
    pub fn bytes(&self) -> u64 {
        self.value.as_ref().map_or(0, |v| v.bytes)
    }
}
impl<K: Ord + Clone, T> Drop for CacheLease<K, T> {
    fn drop(&mut self) {
        let value = self.value.take();
        let mut s = match self.state.lock() {
            Ok(v) => v,
            Err(e) => e.into_inner(),
        };
        // The cache retains an Arc, so decrementing this pin under the lock
        // cannot run T's destructor. Concurrent last leases cannot miss retirement.
        drop(value);
        let remove = s
            .entries
            .get(&self.key)
            .is_some_and(|e| e.retired && Arc::strong_count(&e.value) == 1);
        let removed = if remove {
            s.entries.remove(&self.key)
        } else {
            None
        };
        let released = removed.as_ref().map_or(0, |e| e.value.bytes);
        drop(s);
        drop(removed);
        let mut s = match self.state.lock() {
            Ok(v) => v,
            Err(e) => e.into_inner(),
        };
        s.used -= released;
    }
}
