use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{io::Write, net::TcpListener, thread::JoinHandle};
use vw_ai::{
    Instruction, InstructionRole, PrepareOptions, RevisionBinding, SourcePolicy,
    config::{Capabilities, Format, MaskSupport, Prices, ProviderConfig, ProviderKind, Tokens},
};

fn prepared_for(
    protocol: ApiKeyProtocol,
    intent: u8,
    maximum: usize,
) -> std::result::Result<Prepared, Box<dyn std::error::Error>> {
    let is_gemini = matches!(protocol, ApiKeyProtocol::Gemini);
    let source = fixtures::png(256, 256)?;
    let mut dense = vec![0; 256 * 256];
    for y in 100..156 {
        for x in 100..156 {
            dense[y * 256 + x] = 255;
        }
    }
    let mask = vw_mask::Mask::from_dense(vw_mask::Size::new(256, 256)?, &dense)?;
    Ok(Prepared::new(
        &source,
        &[mask],
        PrepareOptions {
            provider: ProviderConfig {
                schema: 1,
                kind: if is_gemini {
                    ProviderKind::GeminiImageGeneration
                } else {
                    ProviderKind::Flux1Fill {
                        image_price_microusd: 50_000,
                    }
                },
                verified_on: "2026-10-18".into(),
                endpoint: if is_gemini {
                    gemini::ENDPOINT
                } else {
                    flux::ENDPOINT
                }
                .into(),
                model: if is_gemini {
                    "gemini-2.5-flash-image"
                } else {
                    "flux-pro-1.0-fill"
                }
                .into(),
                quality: "standard".into(),
                capabilities: Capabilities {
                    mask_support: MaskSupport::BlackWhitePngGuidance,
                    max_long_edge: if is_gemini { 1024 } else { 256 },
                    min_pixels: if is_gemini { 1024 * 1024 } else { 256 },
                    max_pixels: if is_gemini { 1024 * 1024 } else { 256 * 256 },
                    size_multiple: if is_gemini { 1024 } else { 16 },
                    max_aspect_ratio: 1,
                    formats: vec![Format::Png],
                    max_image_bytes_exclusive: 7_000_000,
                    max_mask_bytes_exclusive: 7_000_000,
                    experimental_above_pixels: 1024 * 1024,
                },
                prices: if is_gemini {
                    Prices {
                        text_input_microusd_per_million: 1_000_000,
                        image_input_microusd_per_million: 2_000_000,
                        image_output_microusd_per_million: 3_000_000,
                    }
                } else {
                    Prices::ZERO
                },
                timeout_seconds: 2,
                max_response_bytes: maximum,
            },
            revision: RevisionBinding {
                project_id: vw_model::Id::from_parts(1, [0; 10])?,
                host_seq: 1,
                state_hash: "a".repeat(64),
            },
            intent_id: vw_model::Id::from_parts(1, [intent; 10])?,
            instructions: vec![Instruction {
                role: InstructionRole::Change,
                text: "Synthetic protocol fixture".into(),
            }],
            feather_px: 0,
            estimated_tokens: if is_gemini {
                Some(Tokens {
                    text_input: 10,
                    image_input: 20,
                    image_output: 30,
                })
            } else {
                None
            },
            source_policy: SourcePolicy {
                assume_untagged_srgb: true,
                allow_16bit_provider_copy: false,
            },
            limits: vw_ai::Limits::default(),
        },
        &NeverCancel,
    )?)
}
// Test-only synthetic capability. It never inspects/derives a live credential.
fn fixture_key(_: &Secret) -> crate::Result<HeaderValue> {
    let mut value = HeaderValue::from_static("synthetic-api-key-fixture-only");
    value.set_sensitive(true);
    Ok(value)
}
fn fixture_adapter(
    protocol: ApiKeyProtocol,
    target: Option<String>,
    secrets: Arc<dyn SecretProvider>,
) -> crate::Result<ApiKeyAdapter> {
    Ok(ApiKeyAdapter {
        client: http::test_client()?,
        secrets,
        api_key_header: fixture_key,
        protocol,
        fixture_target: target,
    })
}
struct WireReply {
    status: u16,
    mime: &'static str,
    body: Vec<u8>,
}
impl WireReply {
    fn json(body: serde_json::Value) -> std::result::Result<Self, serde_json::Error> {
        Ok(Self {
            status: 200,
            mime: "application/json",
            body: serde_json::to_vec(&body)?,
        })
    }
}
struct ScriptServer {
    target: String,
    thread: Option<JoinHandle<std::io::Result<Vec<Observed>>>>,
}
impl ScriptServer {
    fn start(replies: Vec<WireReply>) -> std::io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let thread = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(4);
            let mut observed = Vec::new();
            for reply in replies {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(5))
                        }
                        Err(error) => return Err(error),
                    }
                };
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                let (head, body) = fixtures::read_request(&mut stream)?;
                observed.push(Observed {
                    requests: 1,
                    head,
                    body,
                });
                write!(
                    stream,
                    "HTTP/1.1 {} Fixture\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.status,
                    reply.mime,
                    reply.body.len()
                )?;
                stream.write_all(&reply.body)?;
            }
            Ok(observed)
        });
        Ok(Self {
            target: format!("http://{address}/fixture"),
            thread: Some(thread),
        })
    }
    fn finish(mut self) -> std::result::Result<Vec<Observed>, Box<dyn std::error::Error>> {
        Ok(self
            .thread
            .take()
            .ok_or("fixture thread")?
            .join()
            .map_err(|_| "fixture panic")??)
    }
}
impl Drop for ScriptServer {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn submission() -> serde_json::Value {
    serde_json::json!({"id":"fixture-task","polling_url":"https://api.us1.bfl.ai/v1/get_result?id=fixture-task","cost":5})
}
fn ready() -> serde_json::Value {
    serde_json::json!({"id":"fixture-task","status":"Ready","result":{"sample":"https://delivery.eu1.bfl.ai/fixture.png?signature=fixture"},"cost":5})
}
#[test]
fn flux_real_async_wire_has_one_post_bound_poll_and_no_delivery_credential() -> TestResult {
    let _guard = workers()?;
    let root = temp()?;
    let prepared = prepared_for(ApiKeyProtocol::Flux, 80, 1024 * 1024)?;
    let crop = prepared.review().crop();
    let image = fixtures::png(crop.model_width, crop.model_height)?;
    let server = ScriptServer::start(vec![
        WireReply::json(submission())?,
        WireReply::json(ready())?,
        WireReply {
            status: 200,
            mime: "image/png",
            body: image.clone(),
        },
    ])?;
    let secrets = Arc::new(FakeSecret::default());
    let mut adapter = fixture_adapter(
        ApiKeyProtocol::Flux,
        Some(server.target.clone()),
        secrets.clone(),
    )?;
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    let result = adapter.send_confirmed(
        &prepared,
        &mut ledger,
        Confirmation::explicit_send(prepared.review(), 20_000, false)?,
        BudgetPolicy::default(),
        &NeverCancel,
    )?;
    assert_eq!(result.settlement, Settlement::UsageMissing);
    assert_eq!(result.response.image_png(), image);
    assert!(result.response.tokens().is_none());
    assert_eq!(
        ledger
            .entry(prepared.review().request_id())?
            .ok_or("entry")?
            .state,
        EntryState::Attempted
    );
    assert_eq!(secrets.calls.load(Ordering::Acquire), 1);
    let wire = server.finish()?;
    assert_eq!(wire.len(), 3);
    assert!(wire[0].head.starts_with("POST /fixture "));
    assert!(
        wire[1]
            .head
            .starts_with("GET /v1/get_result?id=fixture-task ")
    );
    assert!(
        wire[2]
            .head
            .starts_with("GET /fixture.png?signature=fixture ")
    );
    for request in &wire[..2] {
        assert!(
            request
                .head
                .to_ascii_lowercase()
                .contains("x-key: synthetic-api-key-fixture-only")
        );
        assert!(!request.head.to_ascii_lowercase().contains("authorization:"));
    }
    assert!(!wire[2].head.to_ascii_lowercase().contains("x-key:"));
    assert!(!wire[2].head.to_ascii_lowercase().contains("authorization:"));
    let payload: serde_json::Value = serde_json::from_slice(&wire[0].body)?;
    assert_eq!(
        payload["image"],
        STANDARD.encode(prepared.request_image_png())
    );
    assert_eq!(
        payload["mask"],
        STANDARD.encode(prepared.request_mask_png())
    );
    prepared.finish(result.response, &NeverCancel)?;
    Ok(())
}
#[test]
fn gemini_inline_wire_uses_its_key_header_and_real_usage_schema() -> TestResult {
    let _guard = workers()?;
    let root = temp()?;
    let prepared = prepared_for(ApiKeyProtocol::Gemini, 81, 1024 * 1024)?;
    let image = fixtures::png(1024, 1024)?;
    let reply = serde_json::json!({"candidates":[{"finishReason":"STOP","index":0,"content":{"parts":[{"inlineData":{"mimeType":"image/png","data":STANDARD.encode(&image)}}]}}],
        "usageMetadata":{"promptTokenCount":30,"candidatesTokenCount":30,"totalTokenCount":60,
            "promptTokensDetails":[{"modality":"TEXT","tokenCount":10},{"modality":"IMAGE","tokenCount":20}],
            "candidatesTokensDetails":[{"modality":"IMAGE","tokenCount":30}]}});
    let server = ScriptServer::start(vec![WireReply::json(reply)?])?;
    let mut adapter = fixture_adapter(
        ApiKeyProtocol::Gemini,
        Some(server.target.clone()),
        Arc::new(FakeSecret::default()),
    )?;
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    let result = adapter.send_confirmed(
        &prepared,
        &mut ledger,
        Confirmation::explicit_send(prepared.review(), 20_000, false)?,
        BudgetPolicy::default(),
        &NeverCancel,
    )?;
    assert_eq!(result.settlement, Settlement::UsagePriced { microusd: 140 });
    let wire = server.finish()?;
    assert_eq!(wire.len(), 1);
    assert!(
        wire[0]
            .head
            .to_ascii_lowercase()
            .contains("x-goog-api-key: synthetic-api-key-fixture-only")
    );
    assert!(!wire[0].head.to_ascii_lowercase().contains("authorization:"));
    assert_eq!(result.response.image_png(), image);
    prepared.finish(result.response, &NeverCancel)?;
    Ok(())
}
#[test]
fn flux_bad_task_status_and_untrusted_delivery_never_download_or_retry() -> TestResult {
    let _guard = workers()?;
    for (index,reply) in [
        serde_json::json!({"id":"wrong-task","status":"Ready","result":null}),
        serde_json::json!({"id":"fixture-task","status":"Content Moderated","result":null}),
        serde_json::json!({"id":"fixture-task","status":"Ready","result":{"sample":"https://evil.invalid/result.png"}}),
    ].into_iter().enumerate() {
        let root=temp()?;
        let prepared=prepared_for(ApiKeyProtocol::Flux,82+index as u8,1024*1024)?;
        let server=ScriptServer::start(vec![WireReply::json(submission())?,WireReply::json(reply)?])?;
        let mut adapter=fixture_adapter(ApiKeyProtocol::Flux,Some(server.target.clone()),Arc::new(FakeSecret::default()))?;
        let mut ledger=SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
        assert!(adapter.send_confirmed(&prepared,&mut ledger,
            Confirmation::explicit_send(prepared.review(),20_000,false)?,BudgetPolicy::default(),&NeverCancel).is_err());
        assert_eq!(server.finish()?.len(),2);
        assert_eq!(ledger.entry(prepared.review().request_id())?.ok_or("entry")?.state,EntryState::Attempted);
    }
    Ok(())
}
#[test]
fn provider_mismatch_and_cancel_precede_reservation_and_key_access() -> TestResult {
    let _guard = workers()?;
    let root = temp()?;
    let prepared = prepared_for(ApiKeyProtocol::Flux, 85, 1024 * 1024)?;
    let secrets = Arc::new(FakeSecret::default());
    let mut adapter = fixture_adapter(ApiKeyProtocol::Gemini, None, secrets.clone())?;
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    assert!(matches!(
        adapter.send_confirmed(
            &prepared,
            &mut ledger,
            Confirmation::explicit_send(prepared.review(), 20_000, false)?,
            BudgetPolicy::default(),
            &NeverCancel
        ),
        Err(Error::Representation)
    ));
    adapter.protocol = ApiKeyProtocol::Flux;
    assert!(matches!(
        adapter.send_confirmed(
            &prepared,
            &mut ledger,
            Confirmation::explicit_send(prepared.review(), 20_000, false)?,
            BudgetPolicy::default(),
            &AtomicBool::new(true)
        ),
        Err(Error::Cancelled)
    ));
    assert!(ledger.entry(prepared.review().request_id())?.is_none());
    assert_eq!(secrets.calls.load(Ordering::Acquire), 0);
    Ok(())
}
#[test]
fn callback_must_produce_sensitive_header_after_consumed_attempt() -> TestResult {
    let _guard = workers()?;
    fn not_sensitive(_: &Secret) -> crate::Result<HeaderValue> {
        Ok(HeaderValue::from_static("synthetic"))
    }
    let root = temp()?;
    let prepared = prepared_for(ApiKeyProtocol::Flux, 86, 1024 * 1024)?;
    let secrets = Arc::new(FakeSecret::default());
    let mut adapter = fixture_adapter(ApiKeyProtocol::Flux, None, secrets.clone())?;
    adapter.api_key_header = not_sensitive;
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    assert!(matches!(
        adapter.send_confirmed(
            &prepared,
            &mut ledger,
            Confirmation::explicit_send(prepared.review(), 20_000, false)?,
            BudgetPolicy::default(),
            &NeverCancel
        ),
        Err(Error::Credential(CredentialFailure::Invalid))
    ));
    assert_eq!(
        ledger
            .entry(prepared.review().request_id())?
            .ok_or("entry")?
            .state,
        EntryState::Attempted
    );
    assert_eq!(secrets.calls.load(Ordering::Acquire), 1);
    Ok(())
}

#[test]
fn gemini_missing_usage_details_preserve_image_without_settlement() -> TestResult {
    let _guard = workers()?;
    let root = temp()?;
    let prepared = prepared_for(ApiKeyProtocol::Gemini, 87, 1024 * 1024)?;
    let image = fixtures::png(1024, 1024)?;
    let reply = serde_json::json!({"candidates":[{"finishReason":"STOP","content":{"parts":[{"inlineData":{"mimeType":"image/png","data":STANDARD.encode(&image)}}]}}],
        "usageMetadata":{"promptTokenCount":30,"candidatesTokenCount":30,"totalTokenCount":60}});
    let server = ScriptServer::start(vec![WireReply::json(reply)?])?;
    let mut adapter = fixture_adapter(
        ApiKeyProtocol::Gemini,
        Some(server.target.clone()),
        Arc::new(FakeSecret::default()),
    )?;
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    let result = adapter.send_confirmed(
        &prepared,
        &mut ledger,
        Confirmation::explicit_send(prepared.review(), 20_000, false)?,
        BudgetPolicy::default(),
        &NeverCancel,
    )?;
    assert_eq!(result.response.image_png(), image);
    assert_eq!(result.settlement, Settlement::UsageMissing);
    assert_eq!(
        ledger
            .entry(prepared.review().request_id())?
            .ok_or("entry")?
            .state,
        EntryState::Attempted
    );
    assert_eq!(server.finish()?.len(), 1);
    Ok(())
}

#[test]
fn pending_flux_deadline_does_not_submit_another_paid_request() -> TestResult {
    let _guard = workers()?;
    let root = temp()?;
    let prepared = prepared_for(ApiKeyProtocol::Flux, 88, 1024 * 1024)?;
    let mut replies = vec![WireReply::json(submission())?];
    for _ in 0..3 {
        replies.push(WireReply::json(serde_json::json!({
            "id":"fixture-task", "status":"Pending", "result":null
        }))?);
    }
    let server = ScriptServer::start(replies)?;
    let mut adapter = fixture_adapter(
        ApiKeyProtocol::Flux,
        Some(server.target.clone()),
        Arc::new(FakeSecret::default()),
    )?;
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    let result = adapter.send_confirmed(
        &prepared,
        &mut ledger,
        Confirmation::explicit_send(prepared.review(), 20_000, false)?,
        BudgetPolicy::default(),
        &NeverCancel,
    );
    assert!(matches!(result, Err(Error::Deadline)));
    let wire = server.finish()?;
    assert_eq!(wire.len(), 4);
    assert_eq!(
        wire.iter()
            .filter(|item| item.head.starts_with("POST "))
            .count(),
        1
    );
    assert_eq!(
        ledger
            .entry(prepared.review().request_id())?
            .ok_or("entry")?
            .state,
        EntryState::Attempted
    );
    Ok(())
}

#[test]
fn json_adapter_cancellation_interrupts_stalled_io_and_keeps_attempt() -> TestResult {
    let _guard = workers()?;
    for (index, protocol) in [ApiKeyProtocol::Gemini, ApiKeyProtocol::Flux]
        .into_iter()
        .enumerate()
    {
        let root = temp()?;
        let prepared = prepared_for(protocol, 89 + index as u8, 1024 * 1024)?;
        let request_id = prepared.review().request_id().to_owned();
        let path = root.path().join("ledger.sqlite");
        let mut ledger = SqliteBudgetLedger::create(&path)?;
        let server = Server::start(Vec::new(), ReplyMode::Stall)?;
        let mut adapter = fixture_adapter(
            protocol,
            Some(server.target().to_owned()),
            Arc::new(FakeSecret::default()),
        )?;
        let cancel = Arc::new(AtomicBool::new(false));
        let caller_cancel = Arc::clone(&cancel);
        let caller = std::thread::spawn(move || {
            let confirmation = Confirmation::explicit_send(prepared.review(), 20_000, false)?;
            adapter.send_confirmed(
                &prepared,
                &mut ledger,
                confirmation,
                BudgetPolicy::default(),
                caller_cancel.as_ref(),
            )
        });
        server.wait_received()?;
        let start = Instant::now();
        cancel.store(true, Ordering::Release);
        assert!(matches!(
            caller.join().map_err(|_| "fixture caller panic")?,
            Err(Error::Cancelled)
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(server.finish()?.requests, 1);
        assert_eq!(
            SqliteBudgetLedger::open(&path)?
                .entry(&request_id)?
                .ok_or("entry")?
                .state,
            EntryState::Attempted
        );
    }
    Ok(())
}

#[test]
fn json_adapters_do_not_retry_redirect_error_malformed_or_over_limit_bodies() -> TestResult {
    let _guard = workers()?;
    for (index, protocol) in [ApiKeyProtocol::Gemini, ApiKeyProtocol::Flux]
        .into_iter()
        .enumerate()
    {
        for (case, mode) in [
            ReplyMode::Redirect,
            ReplyMode::RateLimit,
            ReplyMode::Normal,
            ReplyMode::Chunks,
        ]
        .into_iter()
        .enumerate()
        {
            let root = temp()?;
            let maximum = if case == 3 { 64 } else { 1024 * 1024 };
            let prepared = prepared_for(protocol, 91 + index as u8 * 4 + case as u8, maximum)?;
            let server = Server::start(vec![b' '; 1024], mode)?;
            let mut adapter = fixture_adapter(
                protocol,
                Some(server.target().to_owned()),
                Arc::new(FakeSecret::default()),
            )?;
            let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
            assert!(
                adapter
                    .send_confirmed(
                        &prepared,
                        &mut ledger,
                        Confirmation::explicit_send(prepared.review(), 20_000, false)?,
                        BudgetPolicy::default(),
                        &NeverCancel
                    )
                    .is_err()
            );
            assert_eq!(server.finish()?.requests, 1);
            assert_eq!(
                ledger
                    .entry(prepared.review().request_id())?
                    .ok_or("entry")?
                    .state,
                EntryState::Attempted
            );
        }
    }
    Ok(())
}
