use super::{SessionError, SessionResult, SessionService, endpoint, runtime::NetworkRuntime};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use vw_net::pairing::{
    ClientPairing, CodeDisplay, FingerprintConfirmation, PairingHost, PairingListener,
    PendingPairing, QrPayload,
};

#[derive(Clone, uniffi::Record)]
pub struct PairingOffer {
    pub qr: Vec<u8>,
    pub code: String,
    pub expires_at_ms: u64,
    pub endpoint: String,
}
#[derive(uniffi::Record)]
pub struct PairingOutcome {
    pub paired_device_id: Option<String>,
    pub confirmation: Option<Arc<PairingConfirmation>>,
}
#[derive(uniffi::Object)]
pub struct PairingConfirmation {
    runtime: NetworkRuntime,
    pending: Arc<tokio::sync::Mutex<Option<PendingPairing>>>,
    fingerprint: String,
}
impl PairingConfirmation {
    fn new(runtime: NetworkRuntime, pending: PendingPairing) -> Arc<Self> {
        let fingerprint = pending.fingerprint().display_hex();
        Arc::new(Self {
            runtime,
            pending: Arc::new(tokio::sync::Mutex::new(Some(pending))),
            fingerprint,
        })
    }
}
#[uniffi::export]
impl PairingConfirmation {
    pub fn fingerprint(&self) -> String {
        self.fingerprint.clone()
    }
    /// Call only after the person compares the complete fingerprint on both
    /// devices. The protocol binds it to this actual TLS session as well.
    pub async fn confirm(&self, displayed_fingerprint: String) -> SessionResult<String> {
        let confirmation = FingerprintConfirmation::user_confirmed(&displayed_fingerprint)?;
        if displayed_fingerprint != self.fingerprint {
            return Err(SessionError::Authentication);
        }
        let pending = self.pending.clone();
        self.runtime
            .call(async move {
                let state = pending.lock().await.take().ok_or(SessionError::Closed)?;
                Ok(state.confirm(confirmation).await?.to_string())
            })
            .await
    }
    pub async fn decline(&self) -> SessionResult<()> {
        let pending = self.pending.clone();
        self.runtime
            .call(async move {
                let state = pending.lock().await.take().ok_or(SessionError::Closed)?;
                state.decline().await?;
                Ok(())
            })
            .await
    }
}
#[derive(uniffi::Object)]
pub struct PairingServer {
    runtime: NetworkRuntime,
    listener: Arc<PairingListener>,
    manager: vw_net::pairing::PairingManager,
    accepting: Arc<AtomicBool>,
    address: String,
}
impl Drop for PairingServer {
    fn drop(&mut self) {
        self.runtime.stop();
    }
}
struct AcceptPermit(Arc<AtomicBool>);
impl Drop for AcceptPermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
#[uniffi::export]
impl PairingServer {
    pub fn local_endpoint(&self) -> String {
        self.address.clone()
    }
    /// Issuing another offer durably replaces the previous credential.
    pub async fn offer(&self, use_code: bool) -> SessionResult<PairingOffer> {
        let manager = self.manager.clone();
        let address = self.address.clone();
        self.runtime
            .call(async move {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| SessionError::Storage)?
                    .as_millis();
                if use_code {
                    let code = manager.issue_code()?;
                    Ok(PairingOffer {
                        qr: vec![],
                        code: code.digits_for_display()?.into(),
                        expires_at_ms: u64::try_from(now)
                            .map_err(|_| SessionError::Storage)?
                            .checked_add(vw_net::pairing::PAIRING_LIFETIME_MS)
                            .ok_or(SessionError::Storage)?,
                        endpoint: address,
                    })
                } else {
                    let qr = manager.issue_qr(vec![endpoint(&address, false)?])?;
                    Ok(PairingOffer {
                        qr: qr.encode_for_qr()?.expose().to_vec(),
                        code: String::new(),
                        expires_at_ms: qr.expires_at_ms(),
                        endpoint: address,
                    })
                }
            })
            .await
    }
    pub async fn accept(
        &self,
        cancellation: Arc<crate::Cancellation>,
    ) -> SessionResult<PairingOutcome> {
        self.accepting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| SessionError::Backpressure)?;
        let permit = AcceptPermit(self.accepting.clone());
        let manager = self.manager.clone();
        let listener = self.listener.clone();
        let runtime = self.runtime.clone();
        self.runtime
            .cancellable(cancellation, async move {
                let _permit = permit;
                match PairingHost::accept(manager, &listener).await? {
                    PairingHost::Paired(id) => Ok(PairingOutcome {
                        paired_device_id: Some(id.to_string()),
                        confirmation: None,
                    }),
                    PairingHost::AwaitingConfirmation(pending) => Ok(PairingOutcome {
                        paired_device_id: None,
                        confirmation: Some(PairingConfirmation::new(runtime, *pending)),
                    }),
                }
            })
            .await
    }
    pub async fn close(&self) -> SessionResult<()> {
        self.runtime.close().await
    }
}
#[uniffi::export]
impl SessionService {
    pub async fn listen_pairing(&self, bind_endpoint: String) -> SessionResult<Arc<PairingServer>> {
        let address = endpoint(&bind_endpoint, true)?;
        let manager = self.manager.clone();
        let runtime = crate::worker::startup(|| Ok(NetworkRuntime::new())).await??;
        let owned = runtime.clone();
        runtime
            .call(async move {
                let listener =
                    Arc::new(PairingListener::bind(address, &manager.identity()?).await?);
                let address = listener.local_addr()?.to_string();
                Ok(Arc::new(PairingServer {
                    runtime: owned,
                    listener,
                    manager,
                    accepting: Arc::new(AtomicBool::new(false)),
                    address,
                }))
            })
            .await
    }
    pub async fn join_qr(
        &self,
        qr_bytes: Vec<u8>,
        selected_endpoint: String,
    ) -> SessionResult<String> {
        let qr_bytes = zeroize::Zeroizing::new(qr_bytes);
        let qr = QrPayload::decode_qr(&qr_bytes)?;
        let address = endpoint(&selected_endpoint, false)?;
        let manager = self.manager.clone();
        if !qr.endpoints().contains(&address) {
            return Err(SessionError::Authentication);
        }
        let runtime = crate::worker::startup(|| Ok(NetworkRuntime::new())).await??;
        let result = runtime
            .call(async move { Ok(ClientPairing::qr(manager, &qr, address).await?.to_string()) })
            .await;
        runtime.close().await?;
        result
    }
    pub async fn join_code(
        &self,
        code: String,
        selected_endpoint: String,
        cancellation: Arc<crate::Cancellation>,
    ) -> SessionResult<Arc<PairingConfirmation>> {
        let code_text = zeroize::Zeroizing::new(code);
        let code = CodeDisplay::parse_for_entry(&code_text)?;
        let address = endpoint(&selected_endpoint, false)?;
        let manager = self.manager.clone();
        let runtime = crate::worker::startup(|| Ok(NetworkRuntime::new())).await??;
        let owned = runtime.clone();
        runtime
            .cancellable(cancellation, async move {
                Ok(PairingConfirmation::new(
                    owned,
                    ClientPairing::code(manager, &code, address).await?,
                ))
            })
            .await
    }
}
