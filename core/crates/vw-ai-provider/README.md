# GPT Image transport foundation (T2.02)

This crate consumes the reviewed `vw-ai` `AuthorizedRequest`, cost ledger and
response parser. It changes no canonical model, compositor, mask/proof, AI
configuration, app, FFI or shared API. Source is written; tests and live/provider,
OS credential-store and mobile packaging acceptance are not yet performed.

## Admission and one attempt

The app prepares an edit with `vw_ai::Prepared`, displays its exact review and
configured estimate, then creates `Confirmation::explicit_send` only from the
owner's Send action. Create `PlatformTrust::system()` and `OpenAiAdapter::new`
before offering Send. `send_confirmed` checks cancellation/worker capacity,
reserves the exact review and daily soft budget, durably begins the attempt,
consumes its permit, reads the protected credential and sends once. It implements
the existing synchronous `ProviderAdapter`; `send_typed` retains transport error
detail that the older core trait cannot represent.

A cancellation observed after reservation but before `begin_attempt` records
NotSent. After Attempted, every failure remains unresolved: key unavailable,
worker launch failure, timeout, TLS failure, HTTP error, malformed/partial bytes,
missing usage or settlement-write failure. No branch refunds, resets a ledger,
obtains a second permit or retries a POST. A fresh UI request must follow the
existing ledger's unresolved-attempt recovery policy; changing an intent ID
does not evade that guard. Error response bodies are never read or retained.

On a complete valid response, `Settlement::UsagePriced` means reported tokens
priced with the exact reviewed configuration and durably recorded. It is not
a verified provider invoice. Missing usage retains the image with UsageMissing
and leaves Attempted. A failed ledger write retains the image with
LedgerUnavailable. New output-token breakdowns containing text or inconsistent
image counts fail closed because the current cost model only prices image
output. No token usage is inferred from response size, estimates or errors.
The caller still must run `Prepared::finish` and its exterior proof before
displaying success or accepting a result layer.

## Request, bounds and cancellation

Production exposes only `https://api.openai.com/v1/images/edits`, TLS 1.3 with
the OS verifier and explicit ring. Redirects, system/environment proxies,
cookies, decompression, referer, verbose connection logs and automatic retries
are disabled. HTTP/1.1, Connection: close and no idle pool remove reuse/replay
ambiguity. The upstream retry policy is explicitly `never` (the default retries
protocol NACKs). There is no custom HTTP or TLS parser. No API idempotency claim
is made: crash safety is the local durable one-attempt gate.

`vw-ai` already owns a bounded complete multipart body. An Arc retains it while
the HTTP stream copies at most 16 KiB per upload poll; it never clones the full
request. This is streaming network I/O with a complete bounded JSON result,
not SSE partial-image generation. It leaves model/quality/size/prices exactly
as reviewed and does not append prompts, switch models or enable stream=true.

Response headers are limited to 64 and admitted at 32 KiB aggregate; HTTP/1
parser buffers are also upstream-bounded. Only a single application/json
representation, identity content encoding, and one consistent content length
are accepted. Each response chunk is checked before appending. Capacity is
reserved exactly to AuthorizedRequest::max_response_bytes, whose conservative
4x response allowance also covers retained capacity while parsing smaller
responses. The existing core repeats cardinality/base64/PNG/usage and memory
checks. The core's 16 MiB transport/parser reserve covers our 16 KiB upload
chunk and bounded HTTP buffers; this is allocation admission, not a hard OS RSS
limit, and native verifier/system allocator memory is not exactly measurable.

The entry points are blocking and belong on an application worker, never UI.
At most two owned network workers can exist across all adapter instances, with
no submission queue. Each has one current-thread Tokio runtime and at most one
blocking DNS helper. Caller cancellation is polled every 20 ms and the absolute
1–300 second configured deadline covers credential access, upload and response.
Cancellation sets a stop token and drops the caller's result receiver; it does
not claim the remote service stopped billing. Async network stages drop on
cancellation; already running OS DNS, certificate checks or credential calls
may return later. Their worker keeps the slot and request ownership until
actual runtime shutdown, so repeated cancellation cannot create unbounded
background work. Late results are dropped and cannot write the ledger or a
project. Two permanently stalled OS calls make later attempts return Busy.
Scheduling and platform calls still need real device validation.

## Protected credentials and trust integration

Implement `SecretProvider` only in the trusted application platform layer with
Windows Credential Manager or Android Keystore protected storage. The interface
receives cancellation/deadline and returns a non-cloneable `Secret` from
Zeroizing bytes, at most 4096 visible ASCII bytes. This crate has no key settings,
key-file/environment fallback, command-line secret, raw-error forwarding,
request logging or serializer. Secret Debug is redacted; the authorization
header is sensitive. Our input/Bearer temporary buffers zeroize. Reqwest/HTTP,
TLS, OS and allocator copies do not provide the same guarantee; do not claim
all process memory is scrubbed or that OS key storage has been integrated.

Windows uses the system verifier without extra initialization. On Android the
app must bundle the exact non-test Kotlin component supplied by
rustls-platform-verifier-android, retain its `org.rustls.platformverifier.**`
classes in shrinking, and supply a process-lived JVM/Application/class-loader
`AndroidRuntime`. `initialize_android` verifies the actual loader can load
CertificateVerifier and VerificationResult, then registers that runtime.
`PlatformTrust::system` refuses until this succeeds; this prevents an unbound
native shell binary from consuming a paid permit. Class loading is not a live
TLS/trust-manager acceptance test. The Android initialization, JNI ownership,
Gradle packaging and actual OS credential adapters remain downstream app/FFI
work. No custom root, trust bypass, alternate URL or client injection exists in
the production API; loopback substitution is compiled only into unit tests.

## Primary provenance (checked 2026-10-03)

- [OpenAI image-edit reference](https://developers.openai.com/api/reference/resources/images/methods/edit)
  and [image guide](https://developers.openai.com/api/docs/guides/image-generation):
  multipart image[] + PNG mask, n=1, PNG base64 response, explicit model/quality
  and custom size remain supported. Masks are guidance; local compositing/proof
  remains essential. Newer schemas expose optional output token categories;
  unsupported costs are refused. No default model or current price is embedded.
- [reqwest 0.13.5 published manifest](https://docs.rs/crate/reqwest/0.13.5/source/Cargo.toml.orig):
  exact new pin, MIT OR Apache-2.0, default-features=false, only stream and
  rustls-no-provider. Uses established HTTP/1 implementation and cancellable
  futures. [Retry policy](https://docs.rs/reqwest/0.13.5/reqwest/retry/fn.never.html)
  and [builder API](https://docs.rs/reqwest/0.13.5/reqwest/struct.ClientBuilder.html)
  support the explicit no-retry/no-redirect/no-proxy settings.
- [rustls-platform-verifier 0.7.1 manifest](https://docs.rs/crate/rustls-platform-verifier/0.7.1/source/Cargo.toml)
  and [upstream Android setup](https://github.com/rustls/rustls-platform-verifier#android):
  exact new pin, MIT OR Apache-2.0; Windows system verification and Android JVM
  verification. Android native dependency is rustls-platform-verifier-android
  0.2.0 and JNI 0.22; the matching packaged component must pass the central
  license/lock/inventory gates before app use. No cert-logging, ffi-testing,
  aws-lc, native-tls or insecure features are selected.
- Existing project pins reused: rustls0.23.45/ring, tokio1.53.1, bytes1.12.1,
  futures-util0.3.34 (existing lock), zeroize1.9.0, serde1.0.229,
  serde_json1.0.151 and thiserror2.0.21. Test-only pins are already used locally.
  Global manifest/lock and fetched-artifact hashes are owned by root integration;
  they have not been changed or fabricated here. No broad upgrades requested.

## Written checks and remaining acceptance

Unit source includes an owned loopback HTTP server, real chunked/truncated
responses, exact uploaded multipart comparison, redirect/429 one-request counts,
declared/streamed byte refusal, malformed representation, unsupported usage,
durable settlement/reopen refusal, confirmation/soft-budget-before-key ordering,
redacted header injection checks, real stalled-response cancellation, and
uncooperative worker deadline/late-result/capacity ownership. All fixtures are
synthetic; Android temp roots use the runner's current directory. These tests
have not been built or executed by the author.

Pending: central dependency/license resolution, compilation/lint/tests, Windows
and Android real TLS/OS-store/FFI/app integration, live owner-authorized provider
round trip, actual usage/pricing reconciliation and W1/target-device acceptance.
This source foundation does not close T2.02 or any hardware/product gate.
