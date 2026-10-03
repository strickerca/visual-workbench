use super::{SessionError, SessionResult, device};
use std::sync::Arc;
use vw_net::pairing::{
    CallbackTrustStore, PairingError, PairingManager, TrustState, TrustStore, TrustStoreCallback,
};

/// App-private bytes, protected before any filesystem write. Successful CAS
/// means replacement and revision are durable. No plaintext fallback is allowed.
#[uniffi::export(callback_interface)]
pub trait ProtectedTrustCallback: Send + Sync {
    fn load(&self) -> SessionResult<Option<Vec<u8>>>;
    fn clear_loaded_plaintext(&self);
    fn compare_exchange(
        &self,
        expected_revision: u64,
        replacement_revision: u64,
        plaintext: Vec<u8>,
    ) -> SessionResult<bool>;
}
struct Callback {
    callback: Box<dyn ProtectedTrustCallback>,
    gate: std::sync::Mutex<()>,
}
impl TrustStoreCallback for Callback {
    fn load_app_state(&self) -> vw_net::pairing::Result<Option<Vec<u8>>> {
        let _gate = self.gate.lock().map_err(|_| PairingError::Storage)?;
        let result = self.callback.load().map_err(|_| PairingError::Storage);
        // UniFFI has copied the callback's byte array on return. It is now safe
        // to clear the platform copy before allowing another load/CAS callback.
        self.callback.clear_loaded_plaintext();
        result
    }
    fn compare_exchange_app_state(
        &self,
        expected: u64,
        replacement: Vec<u8>,
    ) -> vw_net::pairing::Result<bool> {
        let bytes = zeroize::Zeroizing::new(replacement);
        let state = TrustState::decode_unprotected(&bytes)?;
        if expected.checked_add(1) != Some(state.revision()) {
            return Err(PairingError::Storage);
        }
        let _gate = self.gate.lock().map_err(|_| PairingError::Storage)?;
        self.callback
            .compare_exchange(expected, state.revision(), bytes.to_vec())
            .map_err(|_| PairingError::Storage)
    }
}
#[derive(Clone, uniffi::Record)]
pub struct LocalDevice {
    pub device_id: String,
    pub fingerprint: String,
}
#[derive(Clone, uniffi::Record)]
pub struct PairedDevice {
    pub device_id: String,
    pub fingerprint: String,
    pub paired_at_ms: u64,
    pub revoked_at_ms: Option<u64>,
}
#[derive(uniffi::Object)]
pub struct SessionService {
    pub(crate) store: Arc<dyn TrustStore>,
    pub(crate) manager: PairingManager,
}
impl SessionService {
    pub(crate) fn from_store(store: Arc<dyn TrustStore>) -> SessionResult<Arc<Self>> {
        let manager = PairingManager::new(store.clone())?;
        Ok(Arc::new(Self { store, manager }))
    }
}
#[uniffi::export]
pub async fn open_callback_session_service(
    installation_device_id: String,
    callback: Box<dyn ProtectedTrustCallback>,
) -> SessionResult<Arc<SessionService>> {
    let id = device(installation_device_id)?;
    crate::worker::startup(move || {
        Ok((|| -> SessionResult<_> {
            SessionService::from_store(Arc::new(CallbackTrustStore::open_for(
                Arc::new(Callback {
                    callback,
                    gate: std::sync::Mutex::new(()),
                }),
                Some(id),
            )?))
        })())
    })
    .await?
}
#[uniffi::export]
pub async fn open_windows_session_service(
    installation_device_id: String,
) -> SessionResult<Arc<SessionService>> {
    let id = device(installation_device_id)?;
    #[cfg(windows)]
    {
        crate::worker::startup(move || {
            Ok((|| -> SessionResult<_> {
                SessionService::from_store(Arc::new(
                    vw_host_win::trust::DpapiTrustStore::open_current_user_for(Some(id))?,
                ))
            })())
        })
        .await?
    }
    #[cfg(not(windows))]
    {
        let _ = id;
        Err(SessionError::Invalid)
    }
}
#[uniffi::export]
impl SessionService {
    pub async fn local_device(&self) -> SessionResult<LocalDevice> {
        let manager = self.manager.clone();
        crate::worker::startup(move || {
            Ok((|| -> SessionResult<_> {
                let identity = manager.identity()?;
                Ok(LocalDevice {
                    device_id: identity.device().to_string(),
                    fingerprint: identity.fingerprint()?.display_hex(),
                })
            })())
        })
        .await?
    }
    pub async fn paired_devices(&self) -> SessionResult<Vec<PairedDevice>> {
        let store = self.store.clone();
        crate::worker::startup(move || {
            Ok((|| -> SessionResult<_> {
                store
                    .load()?
                    .peers()
                    .map(|peer| {
                        Ok(PairedDevice {
                            device_id: peer.device().to_string(),
                            fingerprint: peer.fingerprint()?.display_hex(),
                            paired_at_ms: peer.paired_at_ms(),
                            revoked_at_ms: peer.revoked_at_ms(),
                        })
                    })
                    .collect::<SessionResult<Vec<_>>>()
            })())
        })
        .await?
    }
    pub async fn revoke_device(&self, device_id: String) -> SessionResult<()> {
        let id = device(device_id)?;
        let manager = self.manager.clone();
        crate::worker::startup(move || Ok(manager.revoke(&id).map_err(SessionError::from))).await?
    }
}
