//! Giant-image contracts. Project/native provider registration is composed only
//! after the dedicated owner returns reviewed source and bounded tests.
mod dto;
pub use dto::*;

/// Preserve private provider delivery ownership through ABI serialization. The
/// provider supplied the outgoing pixel charge; this additional copy is admitted
/// before the returned record can create a RustBuffer. Kotlin already owns its
/// independent Foreign reservation for the eventual byte array/bitmap/texture.
pub(crate) fn marshal_delivery(
    pixels: GiantTilePixels,
    outgoing: vw_stream::MemoryCharge,
) -> GiantImageResult<GiantTileDelivery> {
    if pixels.width == 0
        || pixels.height == 0
        || pixels.width > 256
        || pixels.height > 256
        || pixels.rgba.len() as u64 != u64::from(pixels.width) * u64::from(pixels.height) * 4
    {
        return Err(GiantImageError::Invalid);
    }
    let budget = crate::session_resources::shared_budget()?;
    let bytes = (pixels.rgba.len() as u64)
        .checked_add(16 * 1024)
        .ok_or(GiantImageError::Limit)?;
    let abi = budget
        .reserve(vw_stream::MemoryKind::CodecWorkspace, bytes)
        .map_err(|_| GiantImageError::Memory {
            estimated: budget.used_bytes().saturating_add(bytes),
            budget: budget.limit_bytes(),
        })?;
    Ok(GiantTileDelivery {
        pixels,
        native_lease: crate::SessionResourceCharge::from_charges(vec![outgoing, abi]),
    })
}
