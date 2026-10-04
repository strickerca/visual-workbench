use super::*;
use crate::worker::{Worker, startup};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use vw_ai::budget::{BudgetLedger, SqliteBudgetLedger};
use vw_ai_provider::{OpenAiAdapter, PlatformTrust, SecretProvider};

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
                    let configured = credential_ready(&cancellation)?;
                    Ok(AiReadiness {
                        configured,
                        trust_ready: PlatformTrust::system().is_ok(),
                        configuration_current: state.configuration.current(today)?,
                        spent_today_microusd: state.ledger.spent_on(today)?,
                        daily_soft_budget_microusd: state.configuration.daily_soft_budget_microusd,
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
        self.worker
            .call(move |_| {
                Ok((|| {
                    check(&stop, &cancellation)?;
                    #[cfg(windows)]
                    {
                        vw_ai_platform::windows::WindowsCredentials
                            .save(
                                bytes,
                                cancellation.as_ref(),
                                Instant::now() + Duration::from_secs(5),
                            )
                            .map_err(|_| AiEditError::Credentials)
                    }
                    #[cfg(not(windows))]
                    {
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
        self.worker
            .call(move |_| {
                Ok((|| {
                    check(&stop, &cancellation)?;
                    #[cfg(windows)]
                    {
                        vw_ai_platform::windows::WindowsCredentials
                            .remove(
                                cancellation.as_ref(),
                                Instant::now() + Duration::from_secs(5),
                            )
                            .map_err(|_| AiEditError::Credentials)
                    }
                    #[cfg(not(windows))]
                    {
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
pub(super) fn adapter(cancel: &crate::Cancellation) -> AiEditResult<OpenAiAdapter> {
    if !credential_ready(cancel)? {
        return Err(AiEditError::Credentials);
    }
    #[cfg(windows)]
    let secrets: Arc<dyn SecretProvider> = Arc::new(vw_ai_platform::windows::WindowsCredentials);
    #[cfg(target_os = "android")]
    let secrets: Arc<dyn SecretProvider> = Arc::new(
        vw_ai_platform::android::AndroidCredentials::ready()
            .map_err(|_| AiEditError::Credentials)?,
    );
    #[cfg(not(any(windows, target_os = "android")))]
    {
        return Err(AiEditError::Unsupported);
    }
    #[cfg(any(windows, target_os = "android"))]
    OpenAiAdapter::new(
        PlatformTrust::system().map_err(|_| AiEditError::Credentials)?,
        secrets,
    )
    .map_err(AiEditError::from)
}
fn credential_ready(cancel: &crate::Cancellation) -> AiEditResult<bool> {
    #[cfg(windows)]
    {
        vw_ai_platform::windows::WindowsCredentials
            .is_configured(cancel, Instant::now() + Duration::from_secs(5))
            .map_err(|_| AiEditError::Credentials)
    }
    #[cfg(target_os = "android")]
    {
        vw_ai_platform::android::AndroidCredentials::ready()
            .and_then(|store| store.is_configured(cancel, Instant::now() + Duration::from_secs(5)))
            .map_err(|_| AiEditError::Credentials)
    }
    #[cfg(not(any(windows, target_os = "android")))]
    {
        let _ = cancel;
        Err(AiEditError::Unsupported)
    }
}
