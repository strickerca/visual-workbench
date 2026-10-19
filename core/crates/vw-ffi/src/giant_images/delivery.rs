//! One charged native tile publication. The bulk FFI record contains pixels
//! only; its retirement authority is fetched as a separate small object first.
use super::{GiantImageError, GiantImageResult, GiantTilePixels};
use crate::SessionResourceCharge;
use std::sync::{Arc, Mutex};
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget};

// Dropping an undelivered owner destroys its pixels before releasing charges.
struct OwnedPixels {
    pixels: GiantTilePixels,
    lease: Arc<SessionResourceCharge>,
}

enum State {
    Ready(OwnedPixels),
    Prepared(OwnedPixels),
    Taken { _lease: Arc<SessionResourceCharge> },
    Retiring(Arc<SessionResourceCharge>),
    Closed,
}

#[derive(uniffi::Object)]
pub struct PreparedGiantTile {
    state: Mutex<State>,
}

impl PreparedGiantTile {
    /// Only the verified native provider, or the current-project editing
    /// compositor, can supply an outgoing allocation from this endpoint pool.
    pub(crate) fn admit(
        pixels: GiantTilePixels,
        outgoing: MemoryCharge,
    ) -> GiantImageResult<Arc<Self>> {
        Self::admit_in_pool(pixels, outgoing, crate::session_resources::shared_budget()?)
    }

    fn admit_in_pool(
        pixels: GiantTilePixels,
        outgoing: MemoryCharge,
        budget: SessionBudget,
    ) -> GiantImageResult<Arc<Self>> {
        // Fields are declared in retirement order for every rejected input.
        struct Incoming {
            pixels: GiantTilePixels,
            outgoing: MemoryCharge,
        }
        let incoming = Incoming { pixels, outgoing };
        let pixels = &incoming.pixels;
        let canonical_hash = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        };
        let metadata_capacity = pixels
            .asset_id
            .capacity()
            .checked_add(pixels.manifest_hash.capacity())
            .and_then(|sum| sum.checked_add(512))
            .ok_or(GiantImageError::Limit)?;
        if !canonical_hash(&pixels.asset_id)
            || !canonical_hash(&pixels.manifest_hash)
            || metadata_capacity > 16 * 1024
            || pixels.width == 0
            || pixels.height == 0
            || pixels.width > 256
            || pixels.height > 256
            || pixels.rgba.len() as u64 != u64::from(pixels.width) * u64::from(pixels.height) * 4
        {
            return Err(GiantImageError::Invalid);
        }
        if !incoming.outgoing.belongs_to(&budget)
            || pixels.rgba.capacity() as u64 > incoming.outgoing.bytes()
        {
            return Err(GiantImageError::Invalid);
        }
        let bytes = (pixels.rgba.capacity() as u64)
            .checked_add(16 * 1024)
            .ok_or(GiantImageError::Limit)?;
        let abi = budget
            .reserve(MemoryKind::CodecWorkspace, bytes)
            .map_err(|_| GiantImageError::Memory {
                estimated: budget.used_bytes().saturating_add(bytes),
                budget: budget.limit_bytes(),
            })?;
        Ok(Arc::new(Self {
            state: Mutex::new(State::Ready(OwnedPixels {
                pixels: incoming.pixels,
                lease: SessionResourceCharge::from_charges(vec![incoming.outgoing, abi]),
            })),
        }))
    }

    fn release_source_with(
        &self,
        retire: impl FnOnce(&Arc<SessionResourceCharge>) -> GiantImageResult<()>,
    ) -> GiantImageResult<()> {
        let mut state = self.state.lock().map_err(|_| GiantImageError::Storage)?;
        let lease = match &*state {
            State::Closed => return Ok(()),
            State::Taken { .. } => {
                *state = State::Closed;
                return Ok(());
            }
            State::Retiring(lease) => lease.clone(),
            State::Ready(_) | State::Prepared(_) => {
                let owned = match std::mem::replace(&mut *state, State::Closed) {
                    State::Ready(owned) | State::Prepared(owned) => owned,
                    _ => return Err(GiantImageError::Invalid),
                };
                let OwnedPixels { pixels, lease } = owned;
                drop(pixels);
                *state = State::Retiring(lease.clone());
                lease
            }
        };
        // Failed retirement leaves the same Arc reachable for an exact retry.
        retire(&lease)?;
        *state = State::Closed;
        Ok(())
    }
}

#[uniffi::export]
impl PreparedGiantTile {
    /// The foreign adapter must keep this handle before calling take_pixels.
    pub fn publication_lease(&self) -> GiantImageResult<Arc<SessionResourceCharge>> {
        let mut state = self.state.lock().map_err(|_| GiantImageError::Storage)?;
        if matches!(*state, State::Ready(_)) {
            let State::Ready(pixels) = std::mem::replace(&mut *state, State::Closed) else {
                return Err(GiantImageError::Invalid);
            };
            *state = State::Prepared(pixels);
        }
        match &*state {
            State::Prepared(pixels) => Ok(pixels.lease.clone()),
            State::Ready(_) | State::Taken { .. } => Err(GiantImageError::Invalid),
            State::Retiring(_) | State::Closed => Err(GiantImageError::Closed),
        }
    }

    /// The only bulk reply contains plain metadata and RGBA bytes. The source
    /// and the prefetched foreign lease retain the charge through FFI lifting.
    pub fn take_pixels(&self) -> GiantImageResult<GiantTilePixels> {
        let mut state = self.state.lock().map_err(|_| GiantImageError::Storage)?;
        let retained = match &*state {
            State::Ready(_) | State::Taken { .. } => return Err(GiantImageError::Invalid),
            State::Retiring(_) | State::Closed => return Err(GiantImageError::Closed),
            State::Prepared(pixels) => pixels.lease.clone(),
        };
        let State::Prepared(pixels) =
            std::mem::replace(&mut *state, State::Taken { _lease: retained })
        else {
            return Err(GiantImageError::Invalid);
        };
        Ok(pixels.pixels)
    }

    /// Retire undelivered pixels before crediting quota. Once taken, this only
    /// drops the source Arc; the prefetched owner protects foreign bytes.
    pub fn release_source(&self) -> GiantImageResult<()> {
        self.release_source_with(|lease| lease.close())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Weak;

    fn fixture(budget: &SessionBudget) -> Arc<PreparedGiantTile> {
        let outgoing = budget.reserve(MemoryKind::Transfer, 4).unwrap();
        PreparedGiantTile::admit_in_pool(
            GiantTilePixels {
                asset_id: "a".repeat(64),
                manifest_hash: "b".repeat(64),
                address: super::super::GiantTileAddress {
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
                rgba: vec![21, 42, 84, 255],
            },
            outgoing,
            budget.clone(),
        )
        .unwrap()
    }

    fn retired(weak: Weak<SessionResourceCharge>, budget: &SessionBudget) {
        assert_eq!(budget.used_bytes(), 0);
        assert!(weak.upgrade().is_none(), "unread serialized Arc leaked");
    }

    #[test]
    fn failed_actual_bulk_lift_destroys_final_charge() {
        let budget = SessionBudget::new(1 << 20).unwrap();
        let source = fixture(&budget);
        assert!(matches!(
            source.take_pixels(),
            Err(GiantImageError::Invalid)
        ));
        let prefetched = source.publication_lease().unwrap();
        let weak = Arc::downgrade(&prefetched);
        // This uses the production Record derive, including nested metadata and
        // the byte array. Simulate failure while Kotlin reads the first field.
        let wire = <GiantTilePixels as uniffi::Lower<crate::UniFfiTag>>::lower_into_rust_buffer(
            source.take_pixels().unwrap(),
        );
        wire.destroy();
        source.release_source().unwrap();
        assert!(budget.used_bytes() > 0);
        prefetched.close().unwrap();
        drop(prefetched);
        drop(source);
        retired(weak, &budget);
    }

    #[test]
    fn concurrent_source_close_cannot_credit_delivered_bytes() {
        let budget = SessionBudget::new(1 << 20).unwrap();
        let source = fixture(&budget);
        let prefetched = source.publication_lease().unwrap();
        let weak = Arc::downgrade(&prefetched);
        let pixels = source.take_pixels().unwrap();
        assert_eq!(Arc::strong_count(&prefetched), 2);
        let concurrent = source.clone();
        std::thread::spawn(move || concurrent.release_source().unwrap())
            .join()
            .unwrap();
        assert_eq!(pixels.rgba, [21, 42, 84, 255]);
        assert!(budget.used_bytes() > 0);
        drop(pixels);
        prefetched.close().unwrap();
        drop(prefetched);
        drop(source);
        retired(weak, &budget);
    }

    #[test]
    fn undelivered_close_drops_pixels_and_survives_failed_retirement() {
        let budget = SessionBudget::new(1 << 20).unwrap();
        let source = fixture(&budget);
        let prefetched = source.publication_lease().unwrap();
        let weak = Arc::downgrade(&prefetched);
        assert!(matches!(
            source.release_source_with(|_| Err(GiantImageError::Storage)),
            Err(GiantImageError::Storage)
        ));
        assert!(budget.used_bytes() > 0);
        assert!(matches!(source.take_pixels(), Err(GiantImageError::Closed)));
        source.release_source().unwrap();
        prefetched.close().unwrap();
        drop(prefetched);
        drop(source);
        retired(weak, &budget);
    }

    #[test]
    fn failed_source_lock_retains_undelivered_allocation_until_owner_drop() {
        let budget = SessionBudget::new(1 << 20).unwrap();
        let source = fixture(&budget);
        let poison = source.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison.state.lock().unwrap();
            panic!("synthetic poisoned source lock");
        })
        .join();
        assert!(matches!(
            source.release_source(),
            Err(GiantImageError::Storage)
        ));
        assert!(budget.used_bytes() > 0);
        drop(source);
        assert_eq!(budget.used_bytes(), 0);
    }

    #[test]
    fn oversized_identity_cannot_escape_the_bounded_abi_copy() {
        let budget = SessionBudget::new(1 << 20).unwrap();
        let outgoing = budget.reserve(MemoryKind::Transfer, 4).unwrap();
        let mut pixels = GiantTilePixels {
            asset_id: "a".repeat(65),
            manifest_hash: "b".repeat(64),
            address: super::super::GiantTileAddress {
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
            rgba: vec![21, 42, 84, 255],
        };
        assert!(matches!(
            PreparedGiantTile::admit_in_pool(pixels.clone(), outgoing, budget.clone()),
            Err(GiantImageError::Invalid)
        ));
        assert_eq!(budget.used_bytes(), 0);
        let mut overprovisioned = pixels.clone();
        overprovisioned.asset_id = String::with_capacity(16 * 1024);
        overprovisioned.asset_id.push_str(&"a".repeat(64));
        let outgoing = budget.reserve(MemoryKind::Transfer, 4).unwrap();
        assert!(matches!(
            PreparedGiantTile::admit_in_pool(overprovisioned, outgoing, budget.clone()),
            Err(GiantImageError::Invalid)
        ));
        assert_eq!(budget.used_bytes(), 0);
        pixels.asset_id = "A".repeat(64);
        let outgoing = budget.reserve(MemoryKind::Transfer, 4).unwrap();
        assert!(matches!(
            PreparedGiantTile::admit_in_pool(pixels, outgoing, budget.clone()),
            Err(GiantImageError::Invalid)
        ));
        assert_eq!(budget.used_bytes(), 0);
    }
}
