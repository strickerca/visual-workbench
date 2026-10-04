//! Explicit owner-reviewed AI edits. Provider IO never occupies the project worker.
mod admission;
#[cfg(target_os = "android")]
mod android;
mod configuration;
mod context;
mod dto;
mod owner;
mod pixels;
mod prepare;
mod private;
mod request;
mod result;
use crate::Cancellation;
pub use configuration::{default_ai_configuration, validate_ai_configuration};
pub use dto::*;
pub use owner::{AiService, create_ai_service};
pub use request::AiRequest;
use std::sync::atomic::{AtomicBool, Ordering};
const MAX_MEMORY: u64 = 512 * 1024 * 1024;
const RAW_RESERVE: u64 = vw_ai::MAX_ENCODED as u64;
fn check(closed: &AtomicBool, cancellation: &Cancellation) -> AiEditResult<()> {
    if closed.load(Ordering::Acquire) {
        return Err(AiEditError::Closed);
    }
    if cancellation.is_cancelled() {
        return Err(AiEditError::Cancelled);
    }
    Ok(())
}
fn memory(estimate: u64, budget: u64) -> AiEditResult<()> {
    if budget == 0 || budget > MAX_MEMORY || estimate > budget {
        Err(AiEditError::Limit)
    } else {
        Ok(())
    }
}
impl vw_ai::Cancellation for Cancellation {
    fn is_cancelled(&self) -> bool {
        Cancellation::is_cancelled(self)
    }
}
impl From<crate::CoreError> for AiEditError {
    fn from(value: crate::CoreError) -> Self {
        use crate::CoreError as E;
        match value {
            E::Closed => Self::Closed,
            E::Cancelled => Self::Cancelled,
            E::Backpressure | E::Worker => Self::Busy,
            E::Storage => Self::Storage,
            E::Unsupported => Self::Unsupported,
            _ => Self::Invalid,
        }
    }
}
impl From<crate::WorkflowError> for AiEditError {
    fn from(value: crate::WorkflowError) -> Self {
        use crate::WorkflowError as E;
        match value {
            E::Stale | E::ReusedTransaction => Self::Stale,
            E::Closed => Self::Closed,
            E::Cancelled => Self::Cancelled,
            E::Memory { .. } | E::Limit => Self::Limit,
            E::Backpressure => Self::Busy,
            E::Storage => Self::Storage,
            _ => Self::Invalid,
        }
    }
}
impl From<crate::SelectionError> for AiEditError {
    fn from(value: crate::SelectionError) -> Self {
        use crate::SelectionError as E;
        match value {
            E::Conflict | E::ReusedTransaction => Self::Stale,
            E::Closed => Self::Closed,
            E::Cancelled => Self::Cancelled,
            E::Memory { .. } | E::Limit | E::EncodedLimit => Self::Limit,
            E::Backpressure => Self::Busy,
            E::Storage => Self::Storage,
            E::Corrupt => Self::Proof,
            E::Unsupported => Self::Unsupported,
            _ => Self::Invalid,
        }
    }
}
impl From<vw_ai::Error> for AiEditError {
    fn from(value: vw_ai::Error) -> Self {
        use vw_ai::Error as E;
        match value {
            E::Limit(_) => Self::Limit,
            E::Color => Self::Color,
            E::Depth => Self::Depth,
            E::Cancelled => Self::Cancelled,
            E::Exterior(_) => Self::Proof,
            E::Confirmation => Self::Estimate,
            E::SoftBudget => Self::SoftBudget,
            E::Unresolved => Self::Unresolved,
            E::AttemptConsumed => Self::AttemptConsumed,
            E::Storage | E::ClockRegression => Self::Storage,
            E::Unsupported => Self::Unsupported,
            _ => Self::Invalid,
        }
    }
}
impl From<vw_ai_provider::Error> for AiEditError {
    fn from(value: vw_ai_provider::Error) -> Self {
        use vw_ai_provider::Error as E;
        match value {
            E::Core(e) => e.into(),
            E::Busy | E::Worker => Self::Busy,
            E::Cancelled => Self::Cancelled,
            E::Credential(_) | E::TrustUnavailable => Self::Credentials,
            _ => Self::Provider,
        }
    }
}
impl From<vw_model::ModelError> for AiEditError {
    fn from(_: vw_model::ModelError) -> Self {
        Self::Invalid
    }
}
impl From<vw_store::StoreError> for AiEditError {
    fn from(_: vw_store::StoreError) -> Self {
        Self::Storage
    }
}
impl From<vw_mask::MaskError> for AiEditError {
    fn from(_: vw_mask::MaskError) -> Self {
        Self::Limit
    }
}
impl From<vw_raster::RasterError> for AiEditError {
    fn from(e: vw_raster::RasterError) -> Self {
        match e {
            vw_raster::RasterError::Memory { .. } | vw_raster::RasterError::Allocation => {
                Self::Limit
            }
            vw_raster::RasterError::Cancelled => Self::Cancelled,
            _ => Self::Proof,
        }
    }
}

#[cfg(test)]
mod tests;
