mod common;
use base64::{Engine, engine::general_purpose::STANDARD};
use common::*;
use vw_ai::{budget::*, config::*, *};

fn flux_config() -> ProviderConfig {
    let mut config = provider();
    config.kind = ProviderKind::Flux1Fill {
        image_price_microusd: 50_000,
    };
    config.endpoint = "https://api.bfl.ai/v1/flux-pro-1.0-fill".into();
    config.model = "flux-pro-1.0-fill".into();
    config.quality = "standard".into();
    config.capabilities.mask_support = MaskSupport::BlackWhitePngGuidance;
    config.prices = Prices::ZERO;
    config
}
fn gemini_config() -> ProviderConfig {
    let mut config = provider();
    config.kind = ProviderKind::GeminiImageGeneration;
    config.endpoint = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-image:generateContent".into();
    config.model = "gemini-2.5-flash-image".into();
    config.quality = "standard".into();
    let caps = &mut config.capabilities;
    caps.mask_support = MaskSupport::BlackWhitePngGuidance;
    caps.max_long_edge = 1024;
    caps.min_pixels = 1024 * 1024;
    caps.max_pixels = 1024 * 1024;
    caps.size_multiple = 1024;
    caps.max_aspect_ratio = 1;
    caps.max_image_bytes_exclusive = 7_000_000;
    caps.max_mask_bytes_exclusive = 7_000_000;
    config
}
fn prepared_provider(config: ProviderConfig) -> Result<Prepared> {
    prepared_provider_with_instruction(config, None)
}
fn prepared_provider_with_instruction(
    config: ProviderConfig,
    instruction: Option<&str>,
) -> Result<Prepared> {
    let mut opt = options(1).map_err(|_| Error::Invalid("fixture id"))?;
    if let Some(text) = instruction {
        opt.instructions = vec![Instruction {
            role: InstructionRole::Change,
            text: text.into(),
        }];
    }
    opt.estimated_tokens = if matches!(config.kind, ProviderKind::Flux1Fill { .. }) {
        None
    } else {
        opt.estimated_tokens
    };
    opt.provider = config;
    let source =
        png8(256, 256, |_, _| [20, 40, 60, 255]).map_err(|_| Error::Invalid("fixture PNG"))?;
    let mask = region(256, 256, 100, 100, 156, 156).map_err(|_| Error::Invalid("fixture mask"))?;
    Prepared::new(&source, &[mask], opt, &NeverCancel)
}

#[test]
fn provider_mask_preamble_does_not_rewrite_quoted_user_instructions() -> TestResult {
    let quoted = "The attached transparent-alpha mask guides the change region.";
    for kind in [gemini_config(), flux_config()] {
        let prepared = prepared_provider_with_instruction(kind, Some(quoted))?;
        let request = authorize(&prepared)?;
        let body: serde_json::Value = serde_json::from_slice(request.body())?;
        let prompt = if request.provider_kind() == ProviderKind::GeminiImageGeneration {
            body["contents"][0]["parts"][0]["text"]
                .as_str()
                .ok_or("Gemini prompt")?
        } else {
            body["prompt"].as_str().ok_or("FLUX prompt")?
        };
        assert!(prompt.contains("The second image is a black/white change-region mask"));
        assert_eq!(prompt.matches(quoted).count(), 1);
        let instructions: serde_json::Value =
            serde_json::from_str(prompt.split_once('\n').ok_or("preamble")?.1)?;
        assert_eq!(instructions[0]["text"], quoted);
    }
    Ok(())
}
fn authorize(
    prepared: &Prepared,
) -> std::result::Result<AuthorizedRequest, Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    ledger.reserve(
        prepared.review(),
        Confirmation::explicit_send(prepared.review(), 20_000, false)?,
        BudgetPolicy::default(),
    )?;
    Ok(prepared.authorize(ledger.begin_attempt(prepared.review())?, &NeverCancel)?)
}
fn gemini_response(image: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "candidates":[{"index":0,"finishReason":"STOP","content":{"parts":[{
            "inlineData":{"mimeType":"image/png","data":STANDARD.encode(image)}
        }]}}],
        "usageMetadata":{
            "promptTokenCount":30,"candidatesTokenCount":40,"totalTokenCount":70,
            "promptTokensDetails":[{"modality":"IMAGE","tokenCount":20},{"modality":"TEXT","tokenCount":10}],
            "candidatesTokensDetails":[{"modality":"IMAGE","tokenCount":40}]
        }
    })
}
#[test]
fn provider_identity_geometry_and_flat_quote_are_explicit() -> TestResult {
    let flux = flux_config();
    flux.validate()?;
    assert_eq!(flux.estimated_cost(None)?, Some(50_000));
    assert!(
        flux.estimated_cost(Some(Tokens {
            text_input: 0,
            image_input: 0,
            image_output: 1
        }))
        .is_err()
    );
    let mut other = flux.clone();
    other.kind = ProviderKind::Flux1Fill {
        image_price_microusd: 50_001,
    };
    assert_ne!(flux.fingerprint()?, other.fingerprint()?);
    other.prices = provider().prices;
    assert!(other.validate().is_err());
    let gemini = gemini_config();
    gemini.validate()?;
    for case in 0..4 {
        let mut other = gemini.clone();
        match case {
            0 => other.model = "gpt-image-1".into(),
            1 => other.endpoint.push_str("?key=forbidden"),
            2 => other.capabilities.size_multiple = 32,
            _ => other.capabilities.mask_support = MaskSupport::AlphaPngGuidance,
        }
        assert!(other.validate().is_err());
    }
    assert_eq!(
        ProviderConfig::from_json(&serde_json::to_vec(&flux)?)?,
        flux
    );
    Ok(())
}
#[test]
fn flux_payload_uses_exact_reviewed_pngs_and_white_change_mask() -> TestResult {
    let prepared = prepared_provider(flux_config())?;
    let request = authorize(&prepared)?;
    let body: serde_json::Value = serde_json::from_slice(request.body())?;
    assert_eq!(request.content_type(), "application/json");
    assert_eq!(body["image"], STANDARD.encode(prepared.request_image_png()));
    assert_eq!(body["mask"], STANDARD.encode(prepared.request_mask_png()));
    assert_eq!(body["output_format"], "png");
    assert_eq!(body["prompt_upsampling"], false);
    assert!(body.get("model").is_none());
    let mask = read_rgba(prepared.request_mask_png())?;
    assert_eq!(mask.get_pixel(0, 0).0, [0, 0, 0, 255]);
    assert_eq!(
        mask.get_pixel(mask.width() / 2, mask.height() / 2).0,
        [255; 4]
    );
    assert_eq!(prepared.review().estimated_microusd(), Some(50_000));
    Ok(())
}
#[test]
fn gemini_payload_is_image_only_with_bound_inline_source_and_guidance() -> TestResult {
    let prepared = prepared_provider(gemini_config())?;
    let request = authorize(&prepared)?;
    let body: serde_json::Value = serde_json::from_slice(request.body())?;
    assert_eq!(
        body["contents"][0]["parts"][1]["inlineData"]["data"],
        STANDARD.encode(prepared.request_image_png())
    );
    assert_eq!(
        body["contents"][0]["parts"][2]["inlineData"]["data"],
        STANDARD.encode(prepared.request_mask_png())
    );
    assert_eq!(
        body["generationConfig"]["responseModalities"],
        serde_json::json!(["IMAGE"])
    );
    assert_eq!(body["generationConfig"]["candidateCount"], 1);
    assert_eq!(prepared.review().crop().model_width, 1024);
    assert_eq!(prepared.review().crop().model_height, 1024);
    Ok(())
}
#[test]
fn gemini_usage_modalities_totals_and_cardinality_are_checked() -> TestResult {
    let prepared = prepared_provider(gemini_config())?;
    let request = authorize(&prepared)?;
    let image = png8(1024, 1024, |_, _| [11, 22, 33, 255])?;
    let body = gemini_response(&image);
    let result = request.parse_response(&serde_json::to_vec(&body)?)?;
    assert_eq!(result.image_png(), image);
    assert_eq!(
        result.tokens(),
        Some(Tokens {
            text_input: 10,
            image_input: 20,
            image_output: 40
        })
    );
    for case in 0..7 {
        let mut invalid = body.clone();
        match case {
            0 => invalid["usageMetadata"]["candidatesTokensDetails"][0]["modality"] = "TEXT".into(),
            1 => invalid["usageMetadata"]["totalTokenCount"] = 71.into(),
            2 => invalid["usageMetadata"]["thoughtsTokenCount"] = 1.into(),
            3 => invalid["candidates"][0]["finishReason"] = "SAFETY".into(),
            4 => {
                invalid["candidates"][0]["content"]["parts"][0]["inlineData"]["mimeType"] =
                    "image/jpeg".into()
            }
            5 => {
                let part = invalid["candidates"][0]["content"]["parts"][0].clone();
                invalid["candidates"][0]["content"]["parts"]
                    .as_array_mut()
                    .ok_or("parts")?
                    .push(part);
            }
            _ => {
                let candidate = invalid["candidates"][0].clone();
                invalid["candidates"]
                    .as_array_mut()
                    .ok_or("candidates")?
                    .push(candidate);
            }
        }
        let result = request.parse_response(&serde_json::to_vec(&invalid)?);
        if case < 3 {
            let result = result?;
            assert_eq!(result.image_png(), image);
            assert!(result.tokens().is_none());
        } else {
            assert!(result.is_err());
        }
    }
    for detail in ["promptTokensDetails", "candidatesTokensDetails"] {
        let mut missing = body.clone();
        missing["usageMetadata"]
            .as_object_mut()
            .ok_or("usage")?
            .remove(detail);
        let result = request.parse_response(&serde_json::to_vec(&missing)?)?;
        assert_eq!(result.image_png(), image);
        assert!(result.tokens().is_none());
    }
    let mut missing = body;
    missing
        .as_object_mut()
        .ok_or("response")?
        .remove("usageMetadata");
    assert!(
        request
            .parse_response(&serde_json::to_vec(&missing)?)?
            .tokens()
            .is_none()
    );
    Ok(())
}
#[test]
fn flux_download_keeps_attempt_unsettled_and_local_exterior_proof() -> TestResult {
    let prepared = prepared_provider(flux_config())?;
    let request = authorize(&prepared)?;
    let crop = prepared.review().crop();
    let image = png8(crop.model_width, crop.model_height, |_, _| {
        [200, 10, 20, 255]
    })?;
    let response = request.parse_image_response(&image)?;
    assert!(response.tokens().is_none());
    let completed = prepared.finish(response, &NeverCancel)?;
    assert!(completed.actual_microusd().is_none());
    let encoded = completed
        .image()
        .encode_png(Limits::default(), &NeverCancel)?;
    prepared.verify_candidate(&encoded, &NeverCancel)?;
    let changed = png8(256, 256, |_, _| [200, 10, 20, 255])?;
    assert!(matches!(
        prepared.verify_candidate(&changed, &NeverCancel),
        Err(Error::Exterior(_))
    ));
    assert!(request.parse_response(b"{}").is_err());
    Ok(())
}

#[test]
fn exact_flux_credit_receipt_settles_the_original_attempt_only() -> TestResult {
    let root = tempfile::tempdir()?;
    let prepared = prepared_provider(flux_config())?;
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    ledger.reserve(
        prepared.review(),
        Confirmation::explicit_send(prepared.review(), 20_000, false)?,
        BudgetPolicy::default(),
    )?;
    let _permit = ledger.begin_attempt(prepared.review())?;
    let credits: serde_json::Number = serde_json::from_str("0.25")?;
    let receipt = FluxCostReceipt::from_provider("fixture-task", &credits).ok_or("receipt")?;
    assert_eq!(receipt.microusd(), Some(2_500));
    assert_eq!(
        ledger.settle_flux(
            prepared.review().request_id(),
            prepared.provider(),
            &receipt
        )?,
        2_500
    );
    assert_eq!(ledger.spent_on(20_000)?, 2_500);
    assert_eq!(
        ledger
            .entry(prepared.review().request_id())?
            .ok_or("entry")?
            .state,
        EntryState::Settled
    );
    assert_eq!(
        ledger.settle_flux(
            prepared.review().request_id(),
            prepared.provider(),
            &receipt
        )?,
        2_500
    );
    let other = prepared_provider(gemini_config())?;
    assert!(
        ledger
            .settle_flux(prepared.review().request_id(), other.provider(), &receipt)
            .is_err()
    );
    Ok(())
}
