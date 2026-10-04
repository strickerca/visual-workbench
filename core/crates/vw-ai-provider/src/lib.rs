//! One-attempt GPT Image HTTP transport. No built-in key, key-file or environment
//! fallback; no logging. Call blocking entry points on an application worker.
#![forbid(unsafe_code)]

mod http;
mod secret;
mod trust;
mod worker;

pub use secret::{CredentialFailure, Secret, SecretProvider};
pub use trust::PlatformTrust;
#[cfg(target_os = "android")]
pub use trust::{AndroidRuntime, initialize_android};

use std::sync::Arc;
use vw_ai::budget::{BudgetLedger, BudgetPolicy, Confirmation};
use vw_ai::{AuthorizedRequest, Cancellation, Prepared, ProviderResponse};

pub const MAX_ACTIVE_ATTEMPTS: usize = 2;
pub const UPLOAD_CHUNK_BYTES: usize = 16 * 1024;
pub const MAX_SECRET_BYTES: usize = 4096;

/// Errors intentionally exclude HTTP bodies, URLs, prompts, credentials and
/// underlying errors (which may carry headers or OS-private paths).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("platform certificate verification is not initialized or unavailable")]
    TrustUnavailable,
    #[error("provider worker capacity is occupied; do not reset uncertain attempts")]
    Busy,
    #[error("provider worker could not start or terminated")]
    Worker,
    #[error("protected credential retrieval failed: {0}")]
    Credential(CredentialFailure),
    #[error("provider request was cancelled; an attempted charge remains unresolved")]
    Cancelled,
    #[error("provider deadline expired; an attempted charge remains unresolved")]
    Deadline,
    #[error("provider connection or TLS verification failed")]
    Connection,
    #[error("provider HTTP transfer failed")]
    Transfer,
    #[error("provider response exceeds the admitted byte limit")]
    ResponseLimit,
    #[error("provider response headers or representation are invalid")]
    Representation,
    #[error("provider usage contains an unsupported output-token category")]
    UsageUnsupported,
    #[error("provider returned HTTP {status}; no automatic retry")]
    Http {
        status: u16,
        retry_after_seconds: Option<u32>,
    },
    #[error("AI request admission or accounting failed: {0}")]
    Core(#[from] vw_ai::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

/// A locally priced, structurally validated provider usage report is distinct
/// from a bill verified against the owner's provider account.
#[derive(Debug, PartialEq, Eq)]
pub enum Settlement {
    UsagePriced { microusd: u64 },
    UsageMissing,
    LedgerUnavailable,
}

pub struct SendOutcome {
    pub response: ProviderResponse,
    pub settlement: Settlement,
}

/// The only public constructor installs the OS verifier and fixed HTTPS origin.
/// No alternate origin, injected HTTP client, certificate or insecure flag is
/// available in production. The secret source is an application-owned adapter
/// to Credential Manager / Keystore, not a project-supplied implementation.
pub struct OpenAiAdapter {
    client: reqwest::Client,
    secrets: Arc<dyn SecretProvider>,
    #[cfg(test)]
    fixture_target: Option<String>,
}
impl OpenAiAdapter {
    pub fn new(trust: PlatformTrust, secrets: Arc<dyn SecretProvider>) -> Result<Self> {
        Ok(Self {
            client: http::client(trust)?,
            secrets,
            #[cfg(test)]
            fixture_target: None,
        })
    }

    /// Readiness and worker admission precede reservation; credentials are read
    /// only after a durable Attempted permit has been consumed. Once attempted,
    /// cancellation/error/timeout never refunds, retries or resets the ledger.
    /// Run this on a blocking application worker: SQLite and protected storage
    /// are intentionally outside a UI dispatcher.
    pub fn send_confirmed(
        &mut self,
        prepared: &Prepared,
        ledger: &mut impl BudgetLedger,
        confirmation: Confirmation,
        policy: BudgetPolicy,
        cancel: &dyn Cancellation,
    ) -> Result<SendOutcome> {
        check_cancel(cancel)?;
        let slot = worker::Slot::acquire()?;
        ledger.reserve(prepared.review(), confirmation, policy)?;
        if cancel.is_cancelled() {
            ledger.cancel_unattempted(prepared.review().request_id())?;
            return Err(Error::Cancelled);
        }
        let permit = ledger.begin_attempt(prepared.review())?;
        let request = prepared.authorize(permit, cancel)?;
        let response = self.send_admitted(request, slot, cancel)?;
        // Never synthesize usage from estimate, response length, HTTP status,
        // partial response or errors. Keep a useful image if settlement fails.
        let settlement = match response.tokens() {
            None => Settlement::UsageMissing,
            Some(tokens) => match ledger.settle(response.request_id(), prepared.provider(), tokens)
            {
                Ok(microusd) => Settlement::UsagePriced { microusd },
                Err(_) => Settlement::LedgerUnavailable,
            },
        };
        Ok(SendOutcome {
            response,
            settlement,
        })
    }

    /// Low-level typed bridge for a caller already holding the consumed permit.
    /// The caller owns subsequent usage settlement and result compositing/proof.
    pub fn send_typed(
        &mut self,
        request: AuthorizedRequest,
        cancel: &dyn Cancellation,
    ) -> Result<ProviderResponse> {
        check_cancel(cancel)?;
        let slot = worker::Slot::acquire()?;
        self.send_admitted(request, slot, cancel)
    }

    fn send_admitted(
        &self,
        request: AuthorizedRequest,
        slot: worker::Slot,
        cancel: &dyn Cancellation,
    ) -> Result<ProviderResponse> {
        let duration = std::time::Duration::from_secs(u64::from(request.timeout_seconds()));
        let client = self.client.clone();
        let secrets = Arc::clone(&self.secrets);
        #[cfg(test)]
        let fixture_target = self.fixture_target.clone();
        worker::run(slot, duration, cancel, move |stop, deadline| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .max_blocking_threads(1)
                .build()
                .map_err(|_| Error::Worker)?;
            #[cfg(not(test))]
            let result = runtime.block_on(http::send(client, request, secrets, stop, deadline));
            #[cfg(test)]
            let result = match fixture_target {
                Some(target) => runtime.block_on(http::test_send_with_client(
                    client, request, secrets, stop, deadline, &target,
                )),
                None => runtime.block_on(http::send(client, request, secrets, stop, deadline)),
            };
            // Deliberately wait for DNS/blocking OS tasks on THIS owned worker.
            // Caller cancellation is independent; the global slot is retained
            // until shutdown finishes, so blocked OS calls cannot grow threads.
            drop(runtime);
            result
        })
    }
}
impl vw_ai::ProviderAdapter for OpenAiAdapter {
    fn send_once(
        &mut self,
        request: AuthorizedRequest,
        cancel: &dyn Cancellation,
    ) -> vw_ai::Result<ProviderResponse> {
        self.send_typed(request, cancel)
            .map_err(|error| match error {
                Error::Core(error) => error,
                Error::Cancelled => vw_ai::Error::Cancelled,
                Error::ResponseLimit | Error::Busy => {
                    vw_ai::Error::Limit("provider transport admission")
                }
                // Typed callers should use send_typed/send_confirmed. The original
                // synchronous core trait has no network-specific error variants.
                _ => vw_ai::Error::Unsupported,
            })
    }
}
fn check_cancel(cancel: &dyn Cancellation) -> Result<()> {
    if cancel.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
