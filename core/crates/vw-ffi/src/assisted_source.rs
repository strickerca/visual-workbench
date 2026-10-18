//! Original-D pixels for an explicitly admitted assisted-selection backend.
//! No editor compositor, output frame, provider, cache or new memory pool.
mod read;
use crate::{Cancellation, ProjectSession, SelectionBinding, SelectionError, SelectionResult};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, uniffi::Record)]
pub struct AssistedSourcePixelsInfo {
    pub binding: SelectionBinding,
    pub width: u32,
    pub height: u32,
    pub source_bit_depth: u32,
    /// Image/capture originals have no document page. Other kinds refuse.
    pub page: Option<u32>,
    pub rgba_bytes: u64,
}

/// Exactly one full, oriented, sRGB RGBA8 publication. The native, ABI and
/// foreign reservations move together and outlive the acquisition handle.
/// The foreign owner must retire its ByteArray before closing native_lease.
#[derive(uniffi::Record)]
pub struct AssistedSourcePixelsDelivery {
    pub rgba: Vec<u8>,
    pub native_lease: Arc<crate::SessionResourceCharge>,
}

// Fields drop in declaration order: retire the actual native pixel allocation
// before returning its reservations on undelivered/failed acquisition paths.
pub(super) struct OwnedPixels {
    pub rgba: Vec<u8>,
    pub native_lease: Arc<crate::SessionResourceCharge>,
}
enum State {
    Ready(OwnedPixels),
    Taken,
    Retiring(Arc<crate::SessionResourceCharge>),
    Closed,
}

#[derive(uniffi::Object)]
pub struct AssistedSourcePixels {
    info: AssistedSourcePixelsInfo,
    state: Mutex<State>,
}
impl AssistedSourcePixels {
    fn new(info: AssistedSourcePixelsInfo, pixels: OwnedPixels) -> Arc<Self> {
        Arc::new(Self {
            info,
            state: Mutex::new(State::Ready(pixels)),
        })
    }
}
#[uniffi::export]
impl AssistedSourcePixels {
    pub fn describe(&self) -> SelectionResult<AssistedSourcePixelsInfo> {
        let state = self.state.lock().map_err(|_| SelectionError::Storage)?;
        if matches!(*state, State::Retiring(_) | State::Closed) {
            return Err(SelectionError::Closed);
        }
        Ok(self.info.clone())
    }
    /// Fetch and retain this small opaque lease before take_pixels can allocate
    /// a foreign ByteArray. A failed bulk ABI lift then has an explicit known
    /// retirement owner. This is the same underlying Arc returned in delivery.
    pub fn publication_lease(&self) -> SelectionResult<Arc<crate::SessionResourceCharge>> {
        let state = self.state.lock().map_err(|_| SelectionError::Storage)?;
        match &*state {
            State::Ready(pixels) => Ok(pixels.native_lease.clone()),
            State::Taken => Err(SelectionError::Invalid),
            State::Retiring(_) | State::Closed => Err(SelectionError::Closed),
        }
    }
    /// Move the pre-admitted native Vec once. No pixel clone or fresh pool is
    /// created here. The returned lease protects serialization and the actual
    /// foreign bytes, even if this source handle or project closes meanwhile.
    pub fn take_pixels(&self) -> SelectionResult<AssistedSourcePixelsDelivery> {
        let mut state = self.state.lock().map_err(|_| SelectionError::Storage)?;
        match &*state {
            State::Taken => return Err(SelectionError::Invalid),
            State::Retiring(_) | State::Closed => return Err(SelectionError::Closed),
            State::Ready(_) => {}
        }
        let State::Ready(pixels) = std::mem::replace(&mut *state, State::Taken) else {
            return Err(SelectionError::Invalid);
        };
        Ok(AssistedSourcePixelsDelivery {
            rgba: pixels.rgba,
            native_lease: pixels.native_lease,
        })
    }
    /// Idempotent native-handle retirement. After take_pixels the delivery owns
    /// the reservation independently; close cannot credit delivered bytes.
    /// A poisoned lock returns Storage while retaining the payload and charges.
    pub fn release_source(&self) -> SelectionResult<()> {
        let mut state = self.state.lock().map_err(|_| SelectionError::Storage)?;
        let lease = match &*state {
            State::Closed => return Ok(()),
            State::Taken => {
                *state = State::Closed;
                return Ok(());
            }
            State::Retiring(lease) => lease.clone(),
            State::Ready(_) => {
                let State::Ready(pixels) = std::mem::replace(&mut *state, State::Taken) else {
                    return Err(SelectionError::Invalid);
                };
                let OwnedPixels { rgba, native_lease } = pixels;
                // Even a failed small opaque-lease lift is recoverable through
                // this native owner. Retire actual pixels before crediting it.
                drop(rgba);
                *state = State::Retiring(native_lease.clone());
                native_lease
            }
        };
        // Keep Retiring reachable on failure; a retry owns this exact lease.
        lease.close().map_err(read::giant)?;
        *state = State::Closed;
        Ok(())
    }
}

#[uniffi::export]
impl ProjectSession {
    #[allow(clippy::too_many_arguments)]
    pub async fn assisted_source_pixels(
        &self,
        binding: SelectionBinding,
        expected_width: u32,
        expected_height: u32,
        expected_bit_depth: u32,
        page: Option<u32>,
        assume_untagged_srgb: bool,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> SelectionResult<Arc<AssistedSourcePixels>> {
        self.check_open()?;
        read::validate(
            &binding,
            expected_width,
            expected_height,
            expected_bit_depth,
            page,
            memory_budget_bytes,
        )?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    read::live(&closed, &cancellation)?;
                    let value = read::read(
                        state,
                        binding,
                        expected_width,
                        expected_height,
                        expected_bit_depth,
                        assume_untagged_srgb,
                        memory_budget_bytes,
                        &closed,
                        &cancellation,
                    )?;
                    read::live(&closed, &cancellation)?;
                    Ok(value)
                })())
            })
            .await?
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use vw_stream::{MemoryKind, SessionBudget};
    fn fixture(budget: &SessionBudget) -> Arc<AssistedSourcePixels> {
        let charges = (0..3)
            .map(|_| budget.reserve(MemoryKind::Transfer, 4).unwrap())
            .collect();
        AssistedSourcePixels::new(
            AssistedSourcePixelsInfo {
                binding: SelectionBinding {
                    project_id: "p".into(),
                    document_id: "d".into(),
                    source_asset_id: "s".into(),
                    host_seq: 0,
                    state_hash: "h".into(),
                },
                width: 1,
                height: 1,
                source_bit_depth: 8,
                page: None,
                rgba_bytes: 4,
            },
            OwnedPixels {
                rgba: vec![21, 42, 84, 255],
                native_lease: crate::SessionResourceCharge::from_charges(charges),
            },
        )
    }
    #[test]
    fn concurrent_handle_retirement_cannot_credit_the_independent_delivery() {
        let budget = SessionBudget::new(128).unwrap();
        let source = fixture(&budget);
        let prefetched = source.publication_lease().unwrap();
        let delivered = source.take_pixels().unwrap();
        assert!(Arc::ptr_eq(&prefetched, &delivered.native_lease));
        let close = source.clone();
        std::thread::spawn(move || close.release_source().unwrap())
            .join()
            .unwrap();
        assert_eq!(delivered.rgba, [21, 42, 84, 255]);
        assert_eq!(budget.used_bytes(), 12);
        assert!(matches!(source.take_pixels(), Err(SelectionError::Closed)));
        // Test foreign retirement explicitly, before returning its credit.
        let AssistedSourcePixelsDelivery { rgba, native_lease } = delivered;
        drop(rgba);
        native_lease.close().unwrap();
        native_lease.close().unwrap();
        assert_eq!(budget.used_bytes(), 0);
    }
    #[test]
    fn ready_release_can_retire_a_prefetched_but_undelivered_opaque_lease() {
        let budget = SessionBudget::new(128).unwrap();
        let source = fixture(&budget);
        let prefetched = source.publication_lease().unwrap();
        assert_eq!(budget.used_bytes(), 12);
        source.release_source().unwrap();
        // The small foreign object may still exist after a failed lift, but
        // release_source already dropped its native pixels before this credit.
        assert_eq!(budget.used_bytes(), 0);
        assert!(matches!(
            source.publication_lease(),
            Err(SelectionError::Closed)
        ));
        prefetched.close().unwrap();
    }
    #[test]
    fn prefetched_lease_can_retire_a_failed_bulk_lift_after_take() {
        let budget = SessionBudget::new(128).unwrap();
        let source = fixture(&budget);
        let prefetched = source.publication_lease().unwrap();
        let AssistedSourcePixelsDelivery { rgba, native_lease } = source.take_pixels().unwrap();
        // Simulate bulk lift failing: no record reaches the caller. Its known
        // prefetch still owns explicit retirement independently of both handles.
        drop(rgba);
        source.release_source().unwrap();
        assert_eq!(budget.used_bytes(), 12);
        prefetched.close().unwrap();
        assert_eq!(budget.used_bytes(), 0);
        drop(native_lease);
    }
    #[test]
    fn failed_close_keeps_the_undelivered_owner_and_all_its_charges() {
        let budget = SessionBudget::new(128).unwrap();
        let source = fixture(&budget);
        let poison = source.clone();
        let _failed = std::thread::spawn(move || {
            let _state = poison.state.lock().unwrap();
            panic!("synthetic close-state poison");
        })
        .join();
        assert!(matches!(
            source.release_source(),
            Err(SelectionError::Storage)
        ));
        assert_eq!(budget.used_bytes(), 12);
        drop(source); // Native allocation and reservation finally retire together.
        assert_eq!(budget.used_bytes(), 0);
    }
}
