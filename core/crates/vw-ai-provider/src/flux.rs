//! FLUX.1 Fill protocol, verified 2026-10-18 against official BFL docs:
//! https://docs.bfl.ai/flux_1_fill
//! https://docs.bfl.ai/api_integration/integration_guidelines
//! https://docs.bfl.ai/api-reference/utility/get-result
use crate::{
    ApiKeyAdapter, ApiKeyHeader, ApiKeyProtocol, Error, PlatformTrust, Result, SecretProvider,
    SendOutcome, http,
};
use serde::Deserialize;
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
use vw_ai::{
    AuthorizedRequest, Cancellation, FluxCostReceipt, Prepared, ProviderResponse,
    budget::{BudgetLedger, BudgetPolicy, Confirmation},
    config::ProviderKind,
};

pub(crate) const ENDPOINT: &str = "https://api.bfl.ai/v1/flux-pro-1.0-fill";
const MAX_RECEIPT: usize = 65_536;
const MAX_POLLS: usize = 600;
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// One paid POST followed by bounded status polling and an unauthenticated
/// signed-image download. Errors/cancellation never resubmit a paid request.
pub struct FluxFillAdapter(ApiKeyAdapter);
impl FluxFillAdapter {
    pub fn new(
        trust: PlatformTrust,
        secrets: Arc<dyn SecretProvider>,
        api_key_header: ApiKeyHeader,
    ) -> Result<Self> {
        Ok(Self(ApiKeyAdapter::new(
            trust,
            secrets,
            api_key_header,
            ApiKeyProtocol::Flux,
        )?))
    }
    /// Only the task-bound settled cost from BFL's Ready response can settle
    /// this attempt. An absent or inexact cost leaves the image unresolved.
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
impl vw_ai::ProviderAdapter for FluxFillAdapter {
    fn send_once(
        &mut self,
        request: AuthorizedRequest,
        cancel: &dyn Cancellation,
    ) -> vw_ai::Result<ProviderResponse> {
        self.send_typed(request, cancel)
            .map_err(crate::core_transport_error)
    }
}
#[derive(Deserialize)]
struct Submission {
    id: String,
    polling_url: String,
}
#[derive(Deserialize)]
struct PollResult {
    id: String,
    status: String,
    result: Option<Sample>,
    cost: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct Sample {
    sample: String,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}
fn https_url(raw: &str) -> Result<reqwest::Url> {
    if raw.len() > 4096 {
        return Err(Error::Representation);
    }
    let url = reqwest::Url::parse(raw).map_err(|_| Error::Representation)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err(Error::Representation);
    }
    Ok(url)
}
fn polling_url(raw: &str, id: &str) -> Result<reqwest::Url> {
    let url = https_url(raw)?;
    let domain = url.domain().ok_or(Error::Representation)?;
    if !(domain == "api.bfl.ai" || (domain.starts_with("api.") && domain.ends_with(".bfl.ai")))
        || url.path() != "/v1/get_result"
    {
        return Err(Error::Representation);
    }
    let mut query = url.query_pairs();
    match (query.next(), query.next()) {
        (Some((key, value)), None) if key == "id" && value == id => {}
        _ => return Err(Error::Representation),
    }
    Ok(url)
}
fn sample_url(raw: &str) -> Result<reqwest::Url> {
    let url = https_url(raw)?;
    let domain = url.domain().ok_or(Error::Representation)?;
    // Officially documented delivery.*.bfl.ai regions may change. No arbitrary
    // storage origin, IP literal, redirect, userinfo or API credential delivery.
    if !domain.starts_with("delivery.")
        || !domain.ends_with(".bfl.ai")
        || domain == "delivery.bfl.ai"
        || url.path() == "/"
    {
        return Err(Error::Representation);
    }
    Ok(url)
}
fn transport_url(url: reqwest::Url, _target: &str) -> Result<reqwest::Url> {
    #[cfg(test)]
    if _target != ENDPOINT {
        // Tests keep real validated wire URLs and route only network IO to a
        // private loopback listener. This substitution is absent in production.
        let mut fixture = reqwest::Url::parse(_target).map_err(|_| Error::Representation)?;
        fixture.set_path(url.path());
        fixture.set_query(url.query());
        return Ok(fixture);
    }
    Ok(url)
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
    if !matches!(request.provider_kind(), ProviderKind::Flux1Fill { .. })
        || request.endpoint() != ENDPOINT
        || request.content_type() != "application/json"
    {
        return Err(Error::Representation);
    }
    #[cfg(not(test))]
    if target != ENDPOINT {
        return Err(Error::Representation);
    }
    let key = http::load_api_key(&secrets, api_key_header, &stop, deadline)?;
    let request = Arc::new(request);
    let maximum = request.max_response_bytes();
    let bytes = http::post_json(
        &client,
        Arc::clone(&request),
        ("x-key", key.clone()),
        &stop,
        deadline,
        target,
        maximum.min(MAX_RECEIPT),
    )
    .await?;
    let submission: Submission =
        serde_json::from_slice(bytes.as_slice()).map_err(|_| Error::Representation)?;
    drop(bytes);
    if !valid_id(&submission.id) {
        return Err(Error::Representation);
    }
    let poll = polling_url(&submission.polling_url, &submission.id)?;
    let poll = transport_url(poll, target)?;
    for _ in 0..MAX_POLLS {
        http::checked(&stop, deadline, async {
            tokio::time::sleep(POLL_INTERVAL).await;
            Ok(())
        })
        .await?;
        let bytes = http::get_bytes(
            &client,
            poll.clone(),
            Some(key.clone()),
            &stop,
            deadline,
            maximum.min(MAX_RECEIPT),
            "application/json",
        )
        .await?;
        let response: PollResult =
            serde_json::from_slice(bytes.as_slice()).map_err(|_| Error::Representation)?;
        drop(bytes);
        if response.id != submission.id {
            return Err(Error::Representation);
        }
        match response.status.as_str() {
            "Pending" | "Reasoning" | "Generating" if response.result.is_none() => continue,
            "Ready" => {
                let cost_receipt = response
                    .cost
                    .as_ref()
                    .and_then(serde_json::Value::as_number)
                    .and_then(|credits| FluxCostReceipt::from_provider(&submission.id, credits));
                let sample = response.result.ok_or(Error::Representation)?;
                let sample = transport_url(sample_url(&sample.sample)?, target)?;
                drop(key);
                // A delivery URL never receives x-key or Authorization headers.
                let image = http::get_bytes(
                    &client,
                    sample,
                    None,
                    &stop,
                    deadline,
                    maximum.min(vw_ai::MAX_ENCODED),
                    "image/png",
                )
                .await?;
                let result =
                    request.parse_image_response_with_receipt(image.as_slice(), cost_receipt)?;
                http::check(&stop, deadline)?;
                return Ok(result);
            }
            // Terminal failures, moderation and unknown statuses retain the
            // attempted entry. No provider-body text is included in the error.
            _ => return Err(Error::Representation),
        }
    }
    Err(Error::Deadline)
}

#[cfg(test)]
#[path = "tests/flux_urls.rs"]
mod url_tests;
