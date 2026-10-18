use crate::WorkflowBinding;

#[derive(Clone, Debug, uniffi::Record)]
pub struct GiantOpenOptions {
    pub binding: WorkflowBinding,
    pub disk_cache_bytes: u64,
    pub ram_cache_bytes: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct GiantViewport {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GiantTileAddress {
    pub level: u8,
    pub x: u32,
    pub y: u32,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct GiantViewportPlan {
    pub asset_id: String,
    pub manifest_hash: String,
    pub level: u8,
    pub visible: Vec<GiantTileAddress>,
    pub prefetch: Vec<GiantTileAddress>,
}
// Pixel-bearing records intentionally do not derive Debug.
#[derive(Clone, uniffi::Record)]
pub struct GiantTilePixels {
    pub asset_id: String,
    pub manifest_hash: String,
    pub address: GiantTileAddress,
    pub document_x: u32,
    pub document_y: u32,
    pub document_width: u32,
    pub document_height: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum GiantImageError {
    #[error("invalid giant-image request")]
    Invalid,
    #[error("giant-image resource limit exceeded")]
    Limit,
    #[error("giant-image workspace exceeds the budget")]
    Memory { estimated: u64, budget: u64 },
    #[error("giant-image project, source or visible revision changed")]
    Stale,
    #[error("verified derived content is unavailable; rebuild from the original")]
    Missing,
    #[error("unsupported giant-image source or producer")]
    Unsupported,
    #[error("giant-image owner or project closed")]
    Closed,
    #[error("giant-image operation cancelled")]
    Cancelled,
    #[error("giant-image worker capacity exhausted")]
    Backpressure,
    #[error("giant-image storage failed")]
    Storage,
}
pub type GiantImageResult<T> = std::result::Result<T, GiantImageError>;
impl From<crate::CoreError> for GiantImageError {
    fn from(value: crate::CoreError) -> Self {
        use crate::CoreError as E;
        match value {
            E::Closed => Self::Closed,
            E::Cancelled => Self::Cancelled,
            E::Backpressure | E::Worker => Self::Backpressure,
            E::Storage => Self::Storage,
            E::Unsupported => Self::Unsupported,
            _ => Self::Invalid,
        }
    }
}
