//! Giant-image readers and owned-input facades. Only actual platform producers
//! mint build capabilities; source paths and runtime admission stay private.
mod delivery;
mod dto;
mod intake;
mod producer;
mod provider;
mod source;
mod tiles;
use crate::ProjectSession;
pub use delivery::PreparedGiantTile;
pub use intake::*;
pub use producer::{GiantBuiltCache, PreparedOriginal};
pub use provider::GiantImage;
use std::sync::Arc;

/// Consume the complete private publication owner without uncharged intermediate
/// sessions. An undelivered Arc closes only after its worker state is destroyed;
/// the accepted destination is never removed by facade construction failure.
pub(crate) fn adopt_published_project(
    published: producer::PublishedProject,
) -> crate::Result<Arc<ProjectSession>> {
    let (store, blobs, permit, charge) = published.into_parts();
    ProjectSession::from_store_budgeted(store, blobs, permit, charge)
}

#[uniffi::export]
impl ProjectSession {
    pub async fn open_giant_image(
        &self,
        options: GiantOpenOptions,
        cancellation: Arc<crate::Cancellation>,
    ) -> GiantImageResult<Arc<GiantImage>> {
        self.check_open().map_err(GiantImageError::from)?;
        provider::open_image(
            self.worker.clone(),
            self.closed.clone(),
            options,
            crate::session_resources::shared_budget()?,
            cancellation,
        )
        .await
    }
}
#[uniffi::export]
impl GiantImage {
    pub async fn viewport(
        &self,
        view: GiantViewport,
        cancellation: Arc<crate::Cancellation>,
    ) -> GiantImageResult<GiantViewportPlan> {
        self.read_viewport(view, cancellation).await
    }
    pub async fn prepare_tile(
        &self,
        address: GiantTileAddress,
        cancellation: Arc<crate::Cancellation>,
    ) -> GiantImageResult<Arc<PreparedGiantTile>> {
        let delivery = self.read_tile(address, cancellation).await?;
        let (pixels, outgoing) = delivery.into_parts();
        marshal_delivery(pixels, outgoing)
    }
    pub async fn prepare_editing_tile(
        &self,
        address: GiantTileAddress,
        assume_untagged_srgb: bool,
        cancellation: Arc<crate::Cancellation>,
    ) -> GiantImageResult<Arc<PreparedGiantTile>> {
        self.read_editing_tile(address, assume_untagged_srgb, cancellation)
            .await
    }
    pub async fn close(&self) -> GiantImageResult<()> {
        self.close_worker().await
    }
}
pub use dto::*;

/// Admit the provider's already charged original pixels into a small owner
/// before any bulk FFI lowering. Foreign pixels have their own prior reservation.
pub(crate) fn marshal_delivery(
    pixels: GiantTilePixels,
    outgoing: vw_stream::MemoryCharge,
) -> GiantImageResult<Arc<PreparedGiantTile>> {
    PreparedGiantTile::admit(pixels, outgoing)
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use vw_stream::{MemoryKind, SessionBudget};
    fn pixel(rgba: Vec<u8>) -> GiantTilePixels {
        GiantTilePixels {
            asset_id: "a".repeat(64),
            manifest_hash: "b".repeat(64),
            address: GiantTileAddress {
                level: 0,
                x: 0,
                y: 0,
            },
            document_x: 0,
            document_y: 0,
            document_width: 1,
            document_height: 1,
            width: 1,
            height: 1,
            rgba,
        }
    }
    #[test]
    fn reply_rejects_an_outgoing_charge_from_another_pool() -> GiantImageResult<()> {
        let pool = SessionBudget::new(1024).map_err(|_| GiantImageError::Limit)?;
        let outgoing = pool
            .reserve(MemoryKind::Transfer, 4)
            .map_err(|_| GiantImageError::Limit)?;
        assert!(matches!(
            marshal_delivery(pixel(vec![0, 0, 0, 255]), outgoing),
            Err(GiantImageError::Invalid)
        ));
        assert_eq!(pool.used_bytes(), 0);
        Ok(())
    }
    #[test]
    fn reply_capacity_requires_prior_admission_and_valid_reply_keeps_both_leases()
    -> GiantImageResult<()> {
        let pool = crate::session_resources::shared_budget()?;
        let outgoing = pool
            .reserve(MemoryKind::Transfer, 4)
            .map_err(|_| GiantImageError::Limit)?;
        let mut bytes = Vec::with_capacity(1024);
        bytes.extend_from_slice(&[0, 0, 0, 255]);
        assert!(matches!(
            marshal_delivery(pixel(bytes), outgoing),
            Err(GiantImageError::Invalid)
        ));
        let outgoing = pool
            .reserve(MemoryKind::Transfer, 4)
            .map_err(|_| GiantImageError::Limit)?;
        let reply = marshal_delivery(pixel(vec![0, 0, 0, 255]), outgoing)?;
        let lease = reply.publication_lease()?;
        assert_eq!(reply.take_pixels()?.rgba.len(), 4);
        reply.release_source()?;
        lease.close()?;
        lease.close()?;
        Ok(())
    }
}
