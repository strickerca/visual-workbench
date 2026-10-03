//! Deterministic faults around the actual transaction/replica and wire engines.
mod faults;
pub use faults::{Report, SimError, run};
