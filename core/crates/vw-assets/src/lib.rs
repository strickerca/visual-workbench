//! Derived, disposable giant-image tiles. Originals remain immutable and authoritative.
//! The wire manifest is fixed-width and admitted before allocation, with no paths.
#![forbid(unsafe_code)]
mod cache;
mod manifest;
mod plan;
mod scheduler;
pub use cache::{
    BudgetCache, CacheLease, DESKTOP_GPU_BYTES, DESKTOP_RAM_BYTES, PHONE_GPU_BYTES,
    PHONE_RAM_BYTES, Reservation,
};
pub use manifest::{MAX_ENCODED_TILE_BYTES, MAX_MANIFEST_BYTES, Manifest, Source, TileRecord};
pub use plan::{
    Level, MAX_PIXELS, Pyramid, TILE_SIZE, TileKey, TileRegion, Viewport, ViewportTiles,
    maximum_physical_scale,
};
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
