//! Giant-image contracts. Project/native provider registration is composed only
//! after the dedicated owner returns reviewed source and bounded tests.
mod dto;
mod producer;
mod provider;
mod source;
mod tiles;
use crate::ProjectSession;
pub use provider::GiantImage;
use std::sync::Arc;

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
    pub async fn tile(
        &self,
        address: GiantTileAddress,
        cancellation: Arc<crate::Cancellation>,
    ) -> GiantImageResult<GiantTileDelivery> {
        let delivery = self.read_tile(address, cancellation).await?;
        let (pixels, outgoing) = delivery.into_parts();
        marshal_delivery(pixels, outgoing)
    }
    pub async fn close(&self) -> GiantImageResult<()> {
        self.close_worker().await
    }
}
pub use dto::*;

/// Preserve private provider delivery ownership through ABI serialization. The
/// provider supplied the outgoing pixel charge; this additional copy is admitted
/// before the returned record can create a RustBuffer. Kotlin already owns its
/// independent Foreign reservation for the eventual byte array/bitmap/texture.
pub(crate) fn marshal_delivery(
    pixels: GiantTilePixels,
    outgoing: vw_stream::MemoryCharge,
) -> GiantImageResult<GiantTileDelivery> {
    // Fields drop in declaration order: destroy pixel storage before crediting
    // its reservation on every early-return path.
    struct Owned {
        pixels: GiantTilePixels,
        outgoing: vw_stream::MemoryCharge,
    }
    let owned = Owned { pixels, outgoing };
    let pixels = &owned.pixels;
    if pixels.width == 0
        || pixels.height == 0
        || pixels.width > 256
        || pixels.height > 256
        || pixels.rgba.len() as u64 != u64::from(pixels.width) * u64::from(pixels.height) * 4
    {
        return Err(GiantImageError::Invalid);
    }
    let budget = crate::session_resources::shared_budget()?;
    if !owned.outgoing.belongs_to(&budget) || pixels.rgba.capacity() as u64 > owned.outgoing.bytes()
    {
        return Err(GiantImageError::Invalid);
    }
    let bytes = (pixels.rgba.capacity() as u64)
        .checked_add(16 * 1024)
        .ok_or(GiantImageError::Limit)?;
    let abi = budget
        .reserve(vw_stream::MemoryKind::CodecWorkspace, bytes)
        .map_err(|_| GiantImageError::Memory {
            estimated: budget.used_bytes().saturating_add(bytes),
            budget: budget.limit_bytes(),
        })?;
    Ok(GiantTileDelivery {
        pixels: owned.pixels,
        native_lease: crate::SessionResourceCharge::from_charges(vec![owned.outgoing, abi]),
    })
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
        assert_eq!(reply.pixels.rgba.len(), 4);
        reply.native_lease.close()?;
        reply.native_lease.close()?;
        Ok(())
    }
}
