//! One endpoint-process admission pool, shared by all new stream/giant owners.
//! A retired session cannot reset its outstanding resources by opening a new pool.
use crate::{GiantImageError, GiantImageResult};
use std::sync::{Arc, Mutex, OnceLock};
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget, StreamError};

static BUDGET: OnceLock<Result<SessionBudget, StreamError>> = OnceLock::new();

pub(crate) fn shared_budget() -> GiantImageResult<SessionBudget> {
    #[cfg(target_os = "android")]
    let limit = vw_stream::PHONE_SESSION_BYTES;
    #[cfg(not(target_os = "android"))]
    let limit = vw_stream::DESKTOP_SESSION_BYTES;
    match BUDGET.get_or_init(|| SessionBudget::new(limit)) {
        Ok(budget) => Ok(budget.clone()),
        Err(_) => Err(GiantImageError::Limit),
    }
}

#[derive(uniffi::Object)]
pub struct SessionResourcePool {
    budget: SessionBudget,
}
#[uniffi::export]
pub fn shared_session_resources() -> GiantImageResult<Arc<SessionResourcePool>> {
    Ok(Arc::new(SessionResourcePool {
        budget: shared_budget()?,
    }))
}
#[uniffi::export]
impl SessionResourcePool {
    pub fn reserve_foreign(&self, bytes: u64) -> GiantImageResult<Arc<SessionResourceCharge>> {
        if bytes == 0 {
            return Err(GiantImageError::Invalid);
        }
        let charge =
            self.budget
                .reserve(MemoryKind::Foreign, bytes)
                .map_err(|error| match error {
                    StreamError::Limit => GiantImageError::Memory {
                        estimated: self.budget.used_bytes().saturating_add(bytes),
                        budget: self.budget.limit_bytes(),
                    },
                    _ => GiantImageError::Invalid,
                })?;
        Ok(Arc::new(SessionResourceCharge {
            charge: Mutex::new(Some(vec![charge])),
        }))
    }
    pub fn used_bytes(&self) -> u64 {
        self.budget.used_bytes()
    }
    pub fn limit_bytes(&self) -> u64 {
        self.budget.limit_bytes()
    }
}

/// Foreign code retains this object beside its actual rendering resource. The
/// reservation is returned only at explicit retirement or the final owned drop.
#[derive(uniffi::Object)]
pub struct SessionResourceCharge {
    charge: Mutex<Option<Vec<MemoryCharge>>>,
}
impl SessionResourceCharge {
    pub(crate) fn from_charges(charges: Vec<MemoryCharge>) -> Arc<Self> {
        Arc::new(Self {
            charge: Mutex::new(Some(charges)),
        })
    }
}
#[uniffi::export]
impl SessionResourceCharge {
    pub fn close(&self) -> GiantImageResult<()> {
        let charge = self
            .charge
            .lock()
            .map_err(|_| GiantImageError::Storage)?
            .take();
        drop(charge);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreign_object_retains_capacity_and_explicit_close_is_idempotent() -> GiantImageResult<()> {
        let budget = SessionBudget::new(128).map_err(|_| GiantImageError::Limit)?;
        let pool = SessionResourcePool {
            budget: budget.clone(),
        };
        let first = pool.reserve_foreign(80)?;
        assert_eq!(budget.used_bytes(), 80);
        assert!(matches!(
            pool.reserve_foreign(49),
            Err(GiantImageError::Memory { .. })
        ));
        first.close()?;
        first.close()?;
        assert_eq!(budget.used_bytes(), 0);
        let second = pool.reserve_foreign(128)?;
        drop(first);
        assert_eq!(budget.used_bytes(), 128);
        drop(second);
        assert_eq!(budget.used_bytes(), 0);
        Ok(())
    }
}
