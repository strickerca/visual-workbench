//! One charged native tile publication. The bulk FFI record contains pixels
//! only; its retirement authority is fetched as a separate small object first.
use super::{GiantImageError, GiantImageResult, GiantTilePixels};
use crate::SessionResourceCharge;
use std::sync::{Arc, Mutex};
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget};

// UniFFI 0.32.2 writes two 64-byte strings (4-byte length each), a tile
// address (1 + 4 + 4), six u32 fields, and the RGBA count (4): 173 bytes
// besides RGBA. Lower::lower_into_rust_buffer starts with Vec::new and grows
// the buffer as fields are appended. The pinned Rust Vec growth is at most
// twice the final written length, including a partially filled edge tile.
const TILE_WIRE_OVERHEAD: u64 = 2 * (4 + 64) + (1 + 4 + 4) + 6 * 4 + 4;
// Covers the PreparedGiantTile and SessionResourceCharge Arc allocations,
// their headers, and the three MemoryCharge entries retained beside pixels.
const RETAINED_OWNER_BYTES: usize = 512;

fn abi_copy_bound(pixels: &GiantTilePixels) -> GiantImageResult<u64> {
    (pixels.rgba.len() as u64)
        .checked_add(TILE_WIRE_OVERHEAD)
        .and_then(|wire_bytes| wire_bytes.checked_mul(2))
        .ok_or(GiantImageError::Limit)
}

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
        // ABI admission can fail after the resident charge succeeds. Keep the
        // hashes and both charges in one owner so field drop order frees pixel
        // allocations before returning either reservation on that path.
        struct WithResident {
            pixels: GiantTilePixels,
            outgoing: MemoryCharge,
            resident: MemoryCharge,
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
            .and_then(|sum| sum.checked_add(RETAINED_OWNER_BYTES))
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
        // Outgoing already covers the actual RGBA Vec capacity. The foreign
        // reservation is separate. Keep two additional charges: one for the
        // serialized RustBuffer growth and one for identities/owner storage.
        let bytes = abi_copy_bound(pixels)?;
        let resident_bytes =
            u64::try_from(metadata_capacity).map_err(|_| GiantImageError::Limit)?;
        let resident = budget
            .reserve(MemoryKind::Transfer, resident_bytes)
            .map_err(|_| GiantImageError::Memory {
                estimated: budget.used_bytes().saturating_add(resident_bytes),
                budget: budget.limit_bytes(),
            })?;
        let with_resident = WithResident {
            pixels: incoming.pixels,
            outgoing: incoming.outgoing,
            resident,
        };
        let abi = budget
            .reserve(MemoryKind::CodecWorkspace, bytes)
            .map_err(|_| GiantImageError::Memory {
                estimated: budget.used_bytes().saturating_add(bytes),
                budget: budget.limit_bytes(),
            })?;
        let WithResident {
            pixels,
            outgoing,
            resident,
        } = with_resident;
        Ok(Arc::new(Self {
            state: Mutex::new(State::Ready(OwnedPixels {
                pixels,
                lease: SessionResourceCharge::from_charges(vec![outgoing, resident, abi]),
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
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::{
        OnceLock, Weak,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    };

    // SAFETY: This test allocator delegates every allocation operation to
    // System. Its deallocation observer performs only atomic and budget reads.
    struct ObservedAllocator;
    #[global_allocator]
    static OBSERVED_ALLOCATOR: ObservedAllocator = ObservedAllocator;
    static OBSERVED_BUDGET: OnceLock<SessionBudget> = OnceLock::new();
    static HASH_A_PTR: AtomicUsize = AtomicUsize::new(0);
    static HASH_B_PTR: AtomicUsize = AtomicUsize::new(0);
    static RGBA_PTR: AtomicUsize = AtomicUsize::new(0);
    static HASH_A_CHARGE_AT_FREE: AtomicU64 = AtomicU64::new(0);
    static HASH_B_CHARGE_AT_FREE: AtomicU64 = AtomicU64::new(0);
    static RGBA_CHARGE_AT_FREE: AtomicU64 = AtomicU64::new(0);

    unsafe impl GlobalAlloc for ObservedAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: Forward the unchanged valid allocation layout to System.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            let address = ptr as usize;
            let observed = [
                (&HASH_A_PTR, &HASH_A_CHARGE_AT_FREE),
                (&HASH_B_PTR, &HASH_B_CHARGE_AT_FREE),
                (&RGBA_PTR, &RGBA_CHARGE_AT_FREE),
            ];
            for (target, receipt) in observed {
                if target
                    .compare_exchange(address, 0, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    if let Some(budget) = OBSERVED_BUDGET.get() {
                        receipt.store(budget.used_bytes(), Ordering::SeqCst);
                    }
                }
            }
            // SAFETY: The pointer and layout are the exact pair supplied by
            // the allocator client; this wrapper has not changed either.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

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

    fn edge_pixels(width: u32, height: u32) -> GiantTilePixels {
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
            document_width: width,
            document_height: height,
            width,
            height,
            rgba: vec![21; (width * height * 4) as usize],
        }
    }

    #[test]
    fn actual_edge_record_lowering_fits_its_charge_and_tight_budget_refuses() {
        for (width, height) in [
            (1, 1),
            (1, 256),
            (128, 128),
            (192, 192),
            (200, 200),
            (255, 255),
            (256, 256),
        ] {
            let pixels = edge_pixels(width, height);
            let rgba_bytes = pixels.rgba.len() as u64;
            let abi_bytes = abi_copy_bound(&pixels).unwrap();
            let resident_bytes = (pixels.asset_id.capacity()
                + pixels.manifest_hash.capacity()
                + RETAINED_OWNER_BYTES) as u64;
            let budget = SessionBudget::new(rgba_bytes + resident_bytes + abi_bytes).unwrap();
            let outgoing = budget.reserve(MemoryKind::Transfer, rgba_bytes).unwrap();
            let source =
                PreparedGiantTile::admit_in_pool(pixels, outgoing, budget.clone()).unwrap();
            let prefetched = source.publication_lease().unwrap();
            let weak = Arc::downgrade(&prefetched);
            let wire = <GiantTilePixels as uniffi::Lower<crate::UniFfiTag>>::lower_into_rust_buffer(
                source.take_pixels().unwrap(),
            );
            let actual_capacity = wire.capacity() as u64;
            assert_eq!(wire.len() as u64, rgba_bytes + TILE_WIRE_OVERHEAD);
            assert!(
                actual_capacity <= abi_bytes,
                "{width}x{height} grew beyond its ABI charge"
            );
            assert_eq!(budget.used_bytes(), rgba_bytes + resident_bytes + abi_bytes);
            source.release_source().unwrap();
            assert_eq!(budget.used_bytes(), rgba_bytes + resident_bytes + abi_bytes);
            wire.destroy();
            prefetched.close().unwrap();
            drop(prefetched);
            drop(source);
            retired(weak, &budget);

            if width == 200 || width == 255 {
                // The old charge admitted this budget but could not pay for
                // the actual RustBuffer while the original RGBA stayed owned.
                let old_abi = rgba_bytes + 16 * 1024;
                assert!(old_abi < actual_capacity);
                let tight_budget =
                    SessionBudget::new(rgba_bytes + resident_bytes + actual_capacity - 1).unwrap();
                let outgoing = tight_budget
                    .reserve(MemoryKind::Transfer, rgba_bytes)
                    .unwrap();
                assert!(matches!(
                    PreparedGiantTile::admit_in_pool(
                        edge_pixels(width, height),
                        outgoing,
                        tight_budget.clone()
                    ),
                    Err(GiantImageError::Memory { .. })
                ));
                assert_eq!(tight_budget.used_bytes(), 0);
            }
        }
    }

    #[test]
    fn tiny_tile_with_large_retained_hash_buffers_needs_its_own_charge() {
        fn large_hash_buffers() -> GiantTilePixels {
            let mut pixels = edge_pixels(1, 1);
            pixels.asset_id = String::with_capacity(4096);
            pixels.asset_id.push_str(&"a".repeat(64));
            pixels.manifest_hash = String::with_capacity(4096);
            pixels.manifest_hash.push_str(&"b".repeat(64));
            pixels
        }
        let pixels = large_hash_buffers();
        let resident_bytes = (pixels.asset_id.capacity()
            + pixels.manifest_hash.capacity()
            + RETAINED_OWNER_BYTES) as u64;
        let abi_bytes = abi_copy_bound(&pixels).unwrap();
        assert!(resident_bytes >= 8192 + RETAINED_OWNER_BYTES as u64);
        assert_eq!(abi_bytes, 354);
        // This budget admits the same tiny tile with compact 64-byte hashes.
        // It also admitted the inflated hashes under the old ABI-only charge.
        let compact_budget =
            SessionBudget::new(4 + abi_bytes + 128 + RETAINED_OWNER_BYTES as u64).unwrap();
        let outgoing = compact_budget.reserve(MemoryKind::Transfer, 4).unwrap();
        assert!(matches!(
            PreparedGiantTile::admit_in_pool(pixels, outgoing, compact_budget.clone()),
            Err(GiantImageError::Memory { .. })
        ));
        assert_eq!(compact_budget.used_bytes(), 0);
        let outgoing = compact_budget.reserve(MemoryKind::Transfer, 4).unwrap();
        let compact =
            PreparedGiantTile::admit_in_pool(edge_pixels(1, 1), outgoing, compact_budget.clone())
                .unwrap();
        compact.release_source().unwrap();
        assert_eq!(compact_budget.used_bytes(), 0);

        let budget = SessionBudget::new(4 + resident_bytes + abi_bytes).unwrap();
        let outgoing = budget.reserve(MemoryKind::Transfer, 4).unwrap();
        let source =
            PreparedGiantTile::admit_in_pool(large_hash_buffers(), outgoing, budget.clone())
                .unwrap();
        let prefetched = source.publication_lease().unwrap();
        let weak = Arc::downgrade(&prefetched);
        let wire = <GiantTilePixels as uniffi::Lower<crate::UniFfiTag>>::lower_into_rust_buffer(
            source.take_pixels().unwrap(),
        );
        assert!(wire.capacity() as u64 <= abi_bytes);
        assert_eq!(budget.used_bytes(), 4 + resident_bytes + abi_bytes);
        source.release_source().unwrap();
        wire.destroy();
        prefetched.close().unwrap();
        drop(prefetched);
        drop(source);
        retired(weak, &budget);
    }

    #[test]
    fn abi_admission_failure_keeps_quota_until_actual_pixel_deallocation() {
        fn hashes(asset_capacity: usize, manifest_capacity: usize) -> GiantTilePixels {
            let mut pixels = edge_pixels(1, 1);
            pixels.asset_id = String::with_capacity(asset_capacity);
            pixels.asset_id.push_str(&"a".repeat(64));
            pixels.manifest_hash = String::with_capacity(manifest_capacity);
            pixels.manifest_hash.push_str(&"b".repeat(64));
            pixels
        }
        fn observe(pixels: &GiantTilePixels) {
            HASH_A_CHARGE_AT_FREE.store(0, Ordering::SeqCst);
            HASH_B_CHARGE_AT_FREE.store(0, Ordering::SeqCst);
            RGBA_CHARGE_AT_FREE.store(0, Ordering::SeqCst);
            HASH_A_PTR.store(pixels.asset_id.as_ptr() as usize, Ordering::SeqCst);
            HASH_B_PTR.store(pixels.manifest_hash.as_ptr() as usize, Ordering::SeqCst);
            RGBA_PTR.store(pixels.rgba.as_ptr() as usize, Ordering::SeqCst);
        }
        fn assert_observed(expected: u64) {
            assert_eq!(HASH_A_CHARGE_AT_FREE.load(Ordering::SeqCst), expected);
            assert_eq!(HASH_B_CHARGE_AT_FREE.load(Ordering::SeqCst), expected);
            assert_eq!(RGBA_CHARGE_AT_FREE.load(Ordering::SeqCst), expected);
        }
        let pixels = hashes(4096, 4096);
        let resident_bytes = (pixels.asset_id.capacity()
            + pixels.manifest_hash.capacity()
            + RETAINED_OWNER_BYTES) as u64;
        let abi_bytes = abi_copy_bound(&pixels).unwrap();
        let budget = SessionBudget::new(4 + resident_bytes + abi_bytes - 1).unwrap();
        OBSERVED_BUDGET.set(budget.clone()).unwrap();
        observe(&pixels);
        let outgoing = budget.reserve(MemoryKind::Transfer, 4).unwrap();
        assert!(matches!(
            PreparedGiantTile::admit_in_pool(pixels, outgoing, budget.clone()),
            Err(GiantImageError::Memory { .. })
        ));
        assert_observed(4 + resident_bytes);
        assert_eq!(budget.used_bytes(), 0);

        // Validation and resident-admission failures still release pixels
        // before returning the one outgoing charge they could have acquired.
        let mut invalid = hashes(4096, 4096);
        invalid.asset_id.push('a');
        observe(&invalid);
        let outgoing = budget.reserve(MemoryKind::Transfer, 4).unwrap();
        assert!(matches!(
            PreparedGiantTile::admit_in_pool(invalid, outgoing, budget.clone()),
            Err(GiantImageError::Invalid)
        ));
        assert_observed(4);
        assert_eq!(budget.used_bytes(), 0);

        let resident_refused = hashes(8192, 4096);
        observe(&resident_refused);
        let outgoing = budget.reserve(MemoryKind::Transfer, 4).unwrap();
        assert!(matches!(
            PreparedGiantTile::admit_in_pool(resident_refused, outgoing, budget.clone()),
            Err(GiantImageError::Memory { .. })
        ));
        assert_observed(4);
        assert_eq!(budget.used_bytes(), 0);

        // The successful Ready path drops its pixels before explicit release
        // credits the outgoing, resident and ABI charges together.
        let compact = edge_pixels(1, 1);
        observe(&compact);
        let outgoing = budget.reserve(MemoryKind::Transfer, 4).unwrap();
        let source = PreparedGiantTile::admit_in_pool(compact, outgoing, budget.clone()).unwrap();
        let charged = budget.used_bytes();
        source.release_source().unwrap();
        assert_observed(charged);
        assert_eq!(budget.used_bytes(), 0);
    }

    #[test]
    fn fixed_owner_allowance_covers_all_retained_structs_and_charge_entries() {
        let structural_bytes = std::mem::size_of::<PreparedGiantTile>()
            + std::mem::size_of::<SessionResourceCharge>()
            + 3 * std::mem::size_of::<MemoryCharge>()
            + 4 * std::mem::size_of::<usize>(); // Two Arc headers.
        assert!(structural_bytes <= RETAINED_OWNER_BYTES);
    }
}
