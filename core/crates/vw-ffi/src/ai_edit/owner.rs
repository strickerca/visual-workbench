use super::*;
use crate::worker::{Worker, startup};
#[cfg(any(windows, target_os = "android"))]
use std::time::{Duration, Instant};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use vw_ai::budget::{BudgetLedger, SqliteBudgetLedger};
use vw_ai::budget::{BudgetPolicy, Confirmation};
use vw_ai::config::ProviderKind;
#[cfg(any(windows, target_os = "android"))]
use vw_ai_provider::SecretProvider;
use vw_ai_provider::{FluxFillAdapter, GeminiAdapter, OpenAiAdapter, PlatformTrust, SendOutcome};

static OWNERS: AtomicUsize = AtomicUsize::new(0);
pub(super) struct OwnerPermit;
impl OwnerPermit {
    fn acquire() -> AiEditResult<Self> {
        OWNERS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n == 0).then_some(1)
            })
            .map_err(|_| AiEditError::Busy)?;
        Ok(Self)
    }
}
impl Drop for OwnerPermit {
    fn drop(&mut self) {
        OWNERS.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(super) struct OwnerState {
    pub directory: PathBuf,
    pub ledger: SqliteBudgetLedger,
    pub configuration: configuration::Configuration,
    _permit: OwnerPermit,
    _directory_lock: private::DirectoryLock,
}
#[derive(uniffi::Object)]
pub struct AiService {
    pub(super) state: Arc<Mutex<OwnerState>>,
    pub(super) closed: Arc<AtomicBool>,
    worker: Worker<()>,
}
#[uniffi::export]
pub async fn create_ai_service(
    application_private_directory: String,
    provision_first_install: bool,
    cancellation: Arc<crate::Cancellation>,
) -> AiEditResult<Arc<AiService>> {
    let permit = OwnerPermit::acquire()?;
    startup(move || {
        Ok((|| {
            cancellation.check()?;
            let (directory, ledger, configuration, directory_lock) =
                private::open(&application_private_directory, provision_first_install)?;
            cancellation.check()?;
            Ok(Arc::new(AiService {
                state: Arc::new(Mutex::new(OwnerState {
                    directory,
                    ledger,
                    configuration,
                    _permit: permit,
                    _directory_lock: directory_lock,
                })),
                closed: Arc::new(AtomicBool::new(false)),
                worker: Worker::new("vw-ai-settings", (), 2)?,
            }))
        })())
    })
    .await?
}
#[uniffi::export]
impl AiService {
    pub async fn configuration(
        &self,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<AiConfiguration> {
        let shared = self.state.clone();
        let stop = self.closed.clone();
        self.worker
            .call(move |_| {
                Ok((|| {
                    check(&stop, &cancellation)?;
                    shared
                        .try_lock()
                        .map_err(|_| AiEditError::Busy)?
                        .configuration
                        .describe()
                })())
            })
            .await?
    }
    /// Explicit settings action; no request or protected credential is involved.
    pub async fn configure(
        &self,
        json: String,
        expected_fingerprint: String,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<AiConfiguration> {
        let config = configuration::Configuration::parse(json.as_bytes())?;
        let shared = self.state.clone();
        let stop = self.closed.clone();
        self.worker
            .call(move |_| {
                Ok((|| {
                    check(&stop, &cancellation)?;
                    let mut state = shared.try_lock().map_err(|_| AiEditError::Busy)?;
                    if state.configuration.fingerprint()? != expected_fingerprint {
                        return Err(AiEditError::Stale);
                    }
                    private::write_config(&state.directory, &config)?;
                    state.configuration = config;
                    state.configuration.describe()
                })())
            })
            .await?
    }
    pub async fn configure_daily_budget(
        &self,
        expected_fingerprint: String,
        daily_soft_budget_microusd: u64,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<AiConfiguration> {
        if daily_soft_budget_microusd == 0 || daily_soft_budget_microusd > 1_000_000_000 {
            return Err(AiEditError::Invalid);
        }
        let shared = self.state.clone();
        let stop = self.closed.clone();
        self.worker
            .call(move |_| {
                Ok((|| {
                    check(&stop, &cancellation)?;
                    let mut state = shared.try_lock().map_err(|_| AiEditError::Busy)?;
                    if state.configuration.fingerprint()? != expected_fingerprint {
                        return Err(AiEditError::Stale);
                    }
                    let mut config = state.configuration.clone();
                    config.daily_soft_budget_microusd = daily_soft_budget_microusd;
                    private::write_config(&state.directory, &config)?;
                    state.configuration = config;
                    state.configuration.describe()
                })())
            })
            .await?
    }
    /// Explicit settings/Send-readiness check. It never exposes saved key bytes.
    pub async fn readiness(
        &self,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<AiReadiness> {
        let shared = self.state.clone();
        let stop = self.closed.clone();
        self.worker
            .call(move |_| {
                Ok((|| {
                    check(&stop, &cancellation)?;
                    let state = shared.try_lock().map_err(|_| AiEditError::Busy)?;
                    let today = configuration::today()?;
                    let provider_ready = cfg!(any(windows, target_os = "android"));
                    let configured = if provider_ready {
                        credential_ready(state.configuration.provider.kind, &cancellation)?
                    } else {
                        false
                    };
                    Ok(AiReadiness {
                        configured,
                        trust_ready: PlatformTrust::system().is_ok(),
                        configuration_current: state.configuration.current(today)?,
                        spent_today_microusd: state.ledger.spent_on(today)?,
                        daily_soft_budget_microusd: state.configuration.daily_soft_budget_microusd,
                        provider_ready,
                    })
                })())
            })
            .await?
    }
    /// Windows explicit settings only. Android uses the existing secure Keystore
    /// dialog; no fallback stores its key through this method.
    pub async fn save_windows_key(
        &self,
        bytes: Vec<u8>,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<()> {
        let bytes = zeroize::Zeroizing::new(bytes);
        let stop = self.closed.clone();
        let shared = self.state.clone();
        self.worker
            .call(move |_| {
                Ok((|| {
                    check(&stop, &cancellation)?;
                    #[cfg(windows)]
                    {
                        let kind = shared
                            .try_lock()
                            .map_err(|_| AiEditError::Busy)?
                            .configuration
                            .provider
                            .kind;
                        vw_ai_platform::windows::WindowsCredentials::for_provider(kind)
                            .save(
                                bytes,
                                cancellation.as_ref(),
                                Instant::now() + Duration::from_secs(5),
                            )
                            .map_err(|_| AiEditError::Credentials)
                    }
                    #[cfg(not(windows))]
                    {
                        let _ = shared;
                        let _ = bytes;
                        Err(AiEditError::Unsupported)
                    }
                })())
            })
            .await?
    }
    pub async fn remove_windows_key(
        &self,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<()> {
        let stop = self.closed.clone();
        let shared = self.state.clone();
        self.worker
            .call(move |_| {
                Ok((|| {
                    check(&stop, &cancellation)?;
                    #[cfg(windows)]
                    {
                        let kind = shared
                            .try_lock()
                            .map_err(|_| AiEditError::Busy)?
                            .configuration
                            .provider
                            .kind;
                        vw_ai_platform::windows::WindowsCredentials::for_provider(kind)
                            .remove(
                                cancellation.as_ref(),
                                Instant::now() + Duration::from_secs(5),
                            )
                            .map_err(|_| AiEditError::Credentials)
                    }
                    #[cfg(not(windows))]
                    {
                        let _ = shared;
                        Err(AiEditError::Unsupported)
                    }
                })())
            })
            .await?
    }
    pub async fn shutdown(&self) -> AiEditResult<()> {
        self.closed.store(true, Ordering::Release);
        self.worker.shutdown(|_| Ok(())).await?;
        Ok(())
    }
}
impl Drop for AiService {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
}
// Only Windows/Android construct real credential-backed adapters.
#[cfg_attr(not(any(windows, target_os = "android")), allow(dead_code))]
pub(super) enum ProviderAdapter {
    OpenAi(OpenAiAdapter),
    Gemini(GeminiAdapter),
    Flux(FluxFillAdapter),
}
impl ProviderAdapter {
    pub fn send_confirmed(
        &mut self,
        prepared: &vw_ai::Prepared,
        ledger: &mut impl BudgetLedger,
        confirmation: Confirmation,
        policy: BudgetPolicy,
        cancel: &dyn vw_ai::Cancellation,
    ) -> vw_ai_provider::Result<SendOutcome> {
        match self {
            Self::OpenAi(adapter) => {
                adapter.send_confirmed(prepared, ledger, confirmation, policy, cancel)
            }
            Self::Gemini(adapter) => {
                adapter.send_confirmed(prepared, ledger, confirmation, policy, cancel)
            }
            Self::Flux(adapter) => {
                adapter.send_confirmed(prepared, ledger, confirmation, policy, cancel)
            }
        }
    }
}
pub(super) fn adapter(
    kind: ProviderKind,
    cancel: &crate::Cancellation,
) -> AiEditResult<ProviderAdapter> {
    if !credential_ready(kind, cancel)? {
        return Err(AiEditError::Credentials);
    }
    #[cfg(windows)]
    let secrets: Arc<dyn SecretProvider> = Arc::new(
        vw_ai_platform::windows::WindowsCredentials::for_provider(kind),
    );
    #[cfg(target_os = "android")]
    let secrets: Arc<dyn SecretProvider> = Arc::new(
        vw_ai_platform::android::AndroidCredentials::for_provider(kind)
            .map_err(|_| AiEditError::Credentials)?,
    );
    #[cfg(not(any(windows, target_os = "android")))]
    {
        Err(AiEditError::Unsupported)
    }
    #[cfg(any(windows, target_os = "android"))]
    {
        let trust = PlatformTrust::system().map_err(|_| AiEditError::Credentials)?;
        match kind {
            ProviderKind::OpenAiImageEdits => {
                OpenAiAdapter::new(trust, secrets).map(ProviderAdapter::OpenAi)
            }
            ProviderKind::GeminiImageGeneration => {
                GeminiAdapter::new(trust, secrets, vw_ai_provider::Secret::api_key_header)
                    .map(ProviderAdapter::Gemini)
            }
            ProviderKind::Flux1Fill { .. } => {
                FluxFillAdapter::new(trust, secrets, vw_ai_provider::Secret::api_key_header)
                    .map(ProviderAdapter::Flux)
            }
        }
        .map_err(AiEditError::from)
    }
}
fn credential_ready(kind: ProviderKind, cancel: &crate::Cancellation) -> AiEditResult<bool> {
    #[cfg(windows)]
    {
        vw_ai_platform::windows::WindowsCredentials::for_provider(kind)
            .is_configured(cancel, Instant::now() + Duration::from_secs(5))
            .map_err(|_| AiEditError::Credentials)
    }
    #[cfg(target_os = "android")]
    {
        vw_ai_platform::android::AndroidCredentials::for_provider(kind)
            .and_then(|store| store.is_configured(cancel, Instant::now() + Duration::from_secs(5)))
            .map_err(|_| AiEditError::Credentials)
    }
    #[cfg(not(any(windows, target_os = "android")))]
    {
        let _ = (kind, cancel);
        Err(AiEditError::Unsupported)
    }
}
