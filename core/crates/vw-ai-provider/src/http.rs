use crate::{Error, PlatformTrust, Result, SecretProvider, UPLOAD_CHUNK_BYTES};
use bytes::Bytes;
use futures_util::stream;
use reqwest::header::{self, HeaderMap};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use vw_ai::{AuthorizedRequest, ProviderResponse};

pub(crate) const ENDPOINT: &str = "https://api.openai.com/v1/images/edits";
const MAX_HEADERS: usize = 64;
const MAX_HEADER_BYTES: usize = 32 * 1024;

pub(crate) fn client(trust: PlatformTrust) -> Result<reqwest::Client> {
    builder()
        .https_only(true)
        .tls_backend_preconfigured(trust.0)
        .build()
        .map_err(|_| Error::TrustUnavailable)
}
fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .no_hickory_dns()
        .referer(false)
        .connection_verbose(false)
        .http1_only()
        .http1_max_headers(MAX_HEADERS)
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(15))
}

pub(crate) async fn send(
    client: reqwest::Client,
    request: AuthorizedRequest,
    secrets: Arc<dyn SecretProvider>,
    stop: Arc<AtomicBool>,
    deadline: Instant,
) -> Result<ProviderResponse> {
    send_to(client, request, secrets, stop, deadline, ENDPOINT).await
}

// Origin substitution is private and used by unit tests only. Production has
// one caller above and checks the authorized request's fixed original endpoint.
async fn send_to(
    client: reqwest::Client,
    request: AuthorizedRequest,
    secrets: Arc<dyn SecretProvider>,
    stop: Arc<AtomicBool>,
    deadline: Instant,
    target: &str,
) -> Result<ProviderResponse> {
    if request.endpoint() != ENDPOINT || request.max_response_bytes() == 0 {
        return Err(Error::Representation);
    }
    check(&stop, deadline)?;
    let secret = secrets
        .load(stop.as_ref(), deadline)
        .map_err(Error::Credential)?;
    check(&stop, deadline)?;
    let authorization = secret.authorization()?;
    drop(secret); // our protected input and temporary Bearer bytes are zeroized
    let request = Arc::new(request);
    let body_request = Arc::clone(&request);
    let body_stop = Arc::clone(&stop);
    let body = stream::unfold(
        (body_request, body_stop, 0usize),
        move |(request, stop, offset)| async move {
            if offset >= request.body().len() {
                return None;
            }
            if check(&stop, deadline).is_err() {
                return Some((
                    Err(std::io::Error::from(std::io::ErrorKind::Interrupted)),
                    (request, stop, usize::MAX),
                ));
            }
            let end = (offset + UPLOAD_CHUNK_BYTES).min(request.body().len());
            let bytes = Bytes::copy_from_slice(&request.body()[offset..end]);
            Some((Ok(bytes), (request, stop, end)))
        },
    );
    let future = async {
        check(&stop, deadline)?;
        let mut response = client
            .post(target)
            .header(header::AUTHORIZATION, authorization)
            .header(header::CONTENT_TYPE, request.content_type())
            .header(header::CONTENT_LENGTH, request.body().len())
            .header(header::ACCEPT, "application/json")
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::CONNECTION, "close")
            .body(reqwest::Body::wrap_stream(body))
            .timeout(deadline.saturating_duration_since(Instant::now()))
            .send()
            .await
            .map_err(network_error)?;
        let maximum = request.max_response_bytes();
        let declared = admit_headers(response.status().as_u16(), response.headers(), maximum)?;
        let mut bytes = BoundedBytes::new(maximum)?;
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            check(&stop, deadline)?;
            bytes.append(&chunk)?;
        }
        if declared.is_some_and(|size| size != bytes.len()) {
            return Err(Error::Representation);
        }
        check(&stop, deadline)?;
        drop(response);
        drop(client);
        admit_usage(bytes.as_slice())?;
        // vw-ai repeats its own allocation/cardinality/base64/PNG/usage checks.
        // JSON/codec parsing is bounded but not interruptible mid-call; caller
        // cancellation remains prompt because it waits on the owned worker.
        let result = request.parse_response(bytes.as_slice())?;
        check(&stop, deadline)?;
        Ok(result)
    };
    tokio::pin!(future);
    loop {
        tokio::select! {
            biased;
            _ = tokio::time::sleep(crate::worker::POLL) => check(&stop, deadline)?,
            result = &mut future => return result,
        }
    }
}

// Newer provider schemas can split image/text output tokens. The reviewed
// core cost model prices image output only; do not silently price text as image.
// Ignored image strings are scanned without allocating another image copy.
pub(crate) fn admit_usage(bytes: &[u8]) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        usage: Option<Usage>,
    }
    #[derive(serde::Deserialize)]
    struct Usage {
        output_tokens: u64,
        output_tokens_details: Option<Details>,
    }
    #[derive(serde::Deserialize)]
    struct Details {
        image_tokens: u64,
        text_tokens: u64,
    }
    let value: Envelope = serde_json::from_slice(bytes).map_err(|_| Error::Representation)?;
    if value.usage.is_some_and(|usage| {
        usage.output_tokens_details.is_some_and(|detail| {
            detail.text_tokens != 0 || detail.image_tokens != usage.output_tokens
        })
    }) {
        return Err(Error::UsageUnsupported);
    }
    Ok(())
}

fn check(stop: &AtomicBool, deadline: Instant) -> Result<()> {
    if stop.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else if Instant::now() >= deadline {
        Err(Error::Deadline)
    } else {
        Ok(())
    }
}
fn network_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Deadline
    } else if error.is_connect() {
        Error::Connection
    } else {
        Error::Transfer
    }
}

fn single(headers: &HeaderMap, name: header::HeaderName) -> Result<Option<&str>> {
    let mut values = headers.get_all(name).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(Error::Representation);
    }
    value
        .map(|value| value.to_str().map_err(|_| Error::Representation))
        .transpose()
}
pub(crate) fn admit_headers(
    status: u16,
    headers: &HeaderMap,
    maximum: usize,
) -> Result<Option<usize>> {
    if headers.len() > MAX_HEADERS
        || headers
            .iter()
            .try_fold(0usize, |size, (name, value)| {
                size.checked_add(name.as_str().len())?
                    .checked_add(value.as_bytes().len())
            })
            .is_none_or(|size| size > MAX_HEADER_BYTES)
    {
        return Err(Error::Representation);
    }
    if status != 200 {
        let retry_after_seconds = single(headers, header::RETRY_AFTER)?.and_then(|value| {
            if value.len() <= 10 && value.bytes().all(|b| b.is_ascii_digit()) {
                value.parse::<u32>().ok().filter(|&n| n <= 3600)
            } else {
                None
            }
        });
        return Err(Error::Http {
            status,
            retry_after_seconds,
        });
    }
    let kind = single(headers, header::CONTENT_TYPE)?.ok_or(Error::Representation)?;
    let mut parts = kind.split(';');
    if !parts
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
        || parts.any(|value| !value.trim().eq_ignore_ascii_case("charset=utf-8"))
    {
        return Err(Error::Representation);
    }
    if single(headers, header::CONTENT_ENCODING)?
        .is_some_and(|value| !value.eq_ignore_ascii_case("identity"))
    {
        return Err(Error::Representation);
    }
    let declared = single(headers, header::CONTENT_LENGTH)?
        .map(|value| {
            if value.is_empty() || value.len() > 20 || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Error::Representation);
            }
            let size = value.parse::<u64>().map_err(|_| Error::ResponseLimit)?;
            let size = usize::try_from(size).map_err(|_| Error::ResponseLimit)?;
            if size > maximum {
                return Err(Error::ResponseLimit);
            }
            Ok(size)
        })
        .transpose()?;
    Ok(declared)
}

/// Fixed exact capacity prevents Vec geometric growth from exceeding the
/// response admission. Never buffers an unbounded error response.
pub(crate) struct BoundedBytes {
    bytes: Vec<u8>,
    maximum: usize,
}
impl BoundedBytes {
    pub(crate) fn new(maximum: usize) -> Result<Self> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(maximum)
            .map_err(|_| Error::ResponseLimit)?;
        Ok(Self { bytes, maximum })
    }
    pub(crate) fn append(&mut self, chunk: &[u8]) -> Result<()> {
        if chunk.len() > self.maximum.saturating_sub(self.bytes.len()) {
            return Err(Error::ResponseLimit);
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
pub(crate) fn test_client() -> Result<reqwest::Client> {
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|_| Error::TrustUnavailable)?
    .with_root_certificates(rustls::RootCertStore::empty())
    .with_no_client_auth();
    builder()
        .tls_backend_preconfigured(tls)
        .build()
        .map_err(|_| Error::TrustUnavailable)
}
#[cfg(test)]
pub(crate) async fn test_send_with_client(
    client: reqwest::Client,
    request: AuthorizedRequest,
    secret: Arc<dyn SecretProvider>,
    stop: Arc<AtomicBool>,
    deadline: Instant,
    target: &str,
) -> Result<ProviderResponse> {
    // Tests own loopback listeners. This function is absent from production.
    send_to(client, request, secret, stop, deadline, target).await
}
