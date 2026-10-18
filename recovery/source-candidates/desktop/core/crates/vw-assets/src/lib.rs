//! Derived, disposable giant-image tiles. Originals remain immutable and authoritative.
//! The wire manifest is fixed-width and admitted before allocation, with no paths.
mod cache;
mod manifest;
mod plan;
mod scheduler;
pub use cache::{BudgetCache, CacheLease, Reservation, DESKTOP_GPU_BYTES, DESKTOP_RAM_BYTES, PHONE_GPU_BYTES, PHONE_RAM_BYTES};
pub use manifest::{Manifest, Source, TileRecord, MAX_ENCODED_TILE_BYTES, MAX_MANIFEST_BYTES};
pub use plan::{maximum_physical_scale, Level, Pyramid, TileKey, TileRegion, Viewport, ViewportTiles, MAX_PIXELS, TILE_SIZE};
pub use scheduler::{Request, TileScheduler};

pub type Result<T> = std::result::Result<T, AssetError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssetError {
    #[error("invalid tile input: {0}")]
    Invalid(&'static str),
    #[error("tile resource limit exceeded: {0}")]
    Limit(&'static str),
    #[error("tile source or content binding does not match")]
    Binding,
    #[error("tile cache is fully pinned or reserved")]
    Busy,
    #[error("tile operation belongs to a stale viewport")]
    Stale,
    #[error("tile operation was cancelled")]
    Cancelled,
    #[error("tile storage failed")]
    Storage,
    #[error("tile state lock is poisoned")]
    Poisoned,
}
