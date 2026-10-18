//! Gemini 2.5 Flash Image generateContent protocol, verified 2026-10-18:
//! https://ai.google.dev/api/generate-content
//! https://ai.google.dev/gemini-api/docs/models/gemini-2.5-flash-image
//! https://ai.google.dev/gemini-api/docs/image-generation
use crate::{
    ApiKeyAdapter, ApiKeyHeader, ApiKeyProtocol, PlatformTrust, Result, SecretProvider,
    SendOutcome, http,
};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
use vw_ai::{
    AuthorizedRequest, Cancellation, Prepared, ProviderResponse,
    budget::{BudgetLedger, BudgetPolicy, Confirmation},
    config::ProviderKind,
};

pub(crate) const ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-image:generateContent";

/// One explicit Send, one generateContent POST, bounded inline PNG output.
/// Credentials remain in the existing protected provider abstraction.
pub struct GeminiAdapter(ApiKeyAdapter);
impl GeminiAdapter {
    pub fn new(
        trust: PlatformTrust,
        secrets: Arc<dyn SecretProvider>,
        api_key_header: ApiKeyHeader,
    ) -> Result<Self> {
        Ok(Self(ApiKeyAdapter::new(
            trust,
            secrets,
            api_key_header,
            ApiKeyProtocol::Gemini,
        )?))
    }
    pub fn send_confirmed(
        &mut self,
        prepared: &Prepared,
        ledger: &mut impl BudgetLedger,
        confirmation: Confirmation,
        policy: BudgetPolicy,
        cancel: &dyn Cancellation,
    ) -> Result<SendOutcome> {
        self.0
            .send_confirmed(prepared, ledger, confirmation, policy, cancel)
    }
    pub fn send_typed(
        &mut self,
        request: AuthorizedRequest,
        cancel: &dyn Cancellation,
    ) -> Result<ProviderResponse> {
        self.0.send_typed(request, cancel)
    }
}
impl vw_ai::ProviderAdapter for GeminiAdapter {
    fn send_once(
        &mut self,
        request: AuthorizedRequest,
        cancel: &dyn Cancellation,
    ) -> vw_ai::Result<ProviderResponse> {
        self.send_typed(request, cancel)
            .map_err(crate::core_transport_error)
    }
}
pub(crate) async fn send(
    client: reqwest::Client,
    request: AuthorizedRequest,
    secrets: Arc<dyn SecretProvider>,
    api_key_header: ApiKeyHeader,
    stop: Arc<AtomicBool>,
    deadline: Instant,
    target: &str,
) -> Result<ProviderResponse> {
    if request.provider_kind() != ProviderKind::GeminiImageGeneration
        || request.endpoint() != ENDPOINT
        || request.content_type() != "application/json"
    {
        return Err(crate::Error::Representation);
    }
    #[cfg(not(test))]
    if target != ENDPOINT {
        return Err(crate::Error::Representation);
    }
    let header = http::load_api_key(&secrets, api_key_header, &stop, deadline)?;
    let request = Arc::new(request);
    let bytes = http::post_json(
        &client,
        Arc::clone(&request),
        ("x-goog-api-key", header),
        &stop,
        deadline,
        target,
        request.max_response_bytes(),
    )
    .await?;
    http::check(&stop, deadline)?;
    let result = request.parse_response(bytes.as_slice())?;
    http::check(&stop, deadline)?;
    Ok(result)
}
