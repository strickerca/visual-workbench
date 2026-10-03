#![allow(clippy::expect_used)]

use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageEncoder, Rgba, RgbaImage};
use vw_ai_spike::{
    Error, Result,
    budget::Reservation,
    config::{ProviderConfig, Tokens},
    geometry::{self, Rect},
    pixels::{self, SourceImage},
    proof,
    request::{self, MockTransport, Prepared, Transport},
    sha256,
};

fn temporary() -> tempfile::TempDir {
    #[cfg(target_os = "android")]
    return tempfile::tempdir_in(std::env::current_dir().expect("runner directory"))
        .expect("owned fixture");
    #[cfg(not(target_os = "android"))]
    tempfile::tempdir().expect("owned fixture")
}

fn image(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        Rgba([
            (x % 251) as u8,
            (y % 241) as u8,
            ((x * 3 + y) % 239) as u8,
            255,
        ])
    })
}

fn test_config() -> ProviderConfig {
    let mut config = ProviderConfig::bundled().expect("dated provider configuration");
    // Pure offline kernel tests use an explicitly smaller mock capability grid.
    config.capabilities.min_pixels = 256;
    config.capabilities.max_pixels = 4096;
    config.capabilities.max_long_edge = 64;
    config
}

fn prepared(rect: Rect, feather: u32) -> Result<Prepared> {
    let pixels = image(96, 72);
    let source_bytes = pixels::encode_png(&pixels, None)?;
    let mask = pixels::rectangle_mask(96, 72, rect)?;
    Prepared::new(
        SourceImage {
            pixels,
            icc: None,
            orientation_applied: false,
            assumed_srgb: true,
        },
        sha256(&source_bytes),
        &[mask],
        "Change selected color",
        feather,
        test_config(),
    )
}

#[test]
fn dated_configuration_matches_documented_boundaries_and_never_invents_a_quote() -> Result<()> {
    let config = ProviderConfig::bundled()?;
    assert!(config.estimate()?.is_none());
    let caps = &config.capabilities;
    assert!(caps.valid_size(1024, 1024));
    assert!(caps.valid_size(3840, 2160));
    for (w, h) in [
        (16, 16),
        (1023, 1024),
        (3856, 1024),
        (3840, 3840),
        (2048, 512),
    ] {
        assert!(!caps.valid_size(w, h));
    }
    assert_eq!(caps.max_mask_bytes_exclusive, 4_000_000);
    assert_eq!(
        config.prices.cost(Tokens {
            text_input: 1000,
            image_input: 1000,
            image_output: 1000
        })?,
        43_000
    );
    assert!(
        config
            .prices
            .cost(Tokens {
                text_input: u64::MAX,
                image_input: 1,
                image_output: 1
            })
            .is_err()
    );
    Ok(())
}

#[test]
fn crop_expansion_rounds_outward_with_the_minimum_margin() -> Result<()> {
    let mask = pixels::rectangle_mask(
        1000,
        1000,
        Rect {
            x: 300,
            y: 300,
            width: 100,
            height: 200,
        },
    )?;
    let plan = geometry::plan(&mask, 1000, 1000, &ProviderConfig::bundled()?.capabilities)?;
    assert_eq!(
        plan.crop,
        Rect {
            x: 236,
            y: 225,
            width: 228,
            height: 350
        }
    );
    assert!(!plan.has_padding);
    Ok(())
}

#[test]
fn impossible_source_aspect_grows_with_padding_without_clipping_the_mask() -> Result<()> {
    let mask = vec![255; 4096 * 64];
    let plan = geometry::plan(&mask, 4096, 64, &ProviderConfig::bundled()?.capabilities)?;
    assert_eq!(plan.crop.width, 4096);
    assert_eq!(plan.crop.height, 1366);
    assert!(plan.has_padding && plan.crop.y < 0);
    assert!(plan.crop.y + plan.crop.height as i32 >= 64);
    let source = image(8, 2);
    let padded = pixels::crop_padded(
        &source,
        Rect {
            x: -1,
            y: -1,
            width: 10,
            height: 4,
        },
    )?;
    assert_eq!(padded.get_pixel(0, 0), source.get_pixel(0, 0));
    assert_eq!(padded.get_pixel(9, 3), source.get_pixel(7, 1));
    Ok(())
}

#[test]
fn seeded_small_border_and_extreme_crops_always_choose_valid_provider_sizes() -> Result<()> {
    let caps = ProviderConfig::bundled()?.capabilities;
    let mut state = 0x5657_0011_2026u64;
    for _ in 0..128 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let w = ((state >> 12) % 4096 + 1) as u32;
        let h = ((state >> 33) % 2048 + 1) as u32;
        let chosen = geometry::closest_size(w, h, &caps)?;
        assert!(caps.valid_size(chosen.0, chosen.1));
        if u64::from(w) * u64::from(h) < caps.min_pixels {
            assert!(u64::from(chosen.0) * u64::from(chosen.1) >= caps.min_pixels);
        }
    }
    Ok(())
}

#[test]
fn area_mask_obeys_alpha_polarity_and_exact_cell_overlap() -> Result<()> {
    let mask = [255, 0];
    let rect = Rect {
        x: 0,
        y: 0,
        width: 2,
        height: 1,
    };
    let down = pixels::area_mask(&mask, 2, 1, rect, 1, 1)?;
    assert_eq!(down.get_pixel(0, 0)[3], 127);
    let up = pixels::area_mask(&mask, 2, 1, rect, 4, 1)?;
    assert_eq!(
        up.pixels().map(|p| p[3]).collect::<Vec<_>>(),
        vec![0, 0, 255, 255]
    );
    let padded = pixels::area_mask(
        &mask,
        2,
        1,
        Rect {
            x: -1,
            y: 0,
            width: 4,
            height: 1,
        },
        4,
        1,
    )?;
    assert_eq!(
        padded.pixels().map(|p| p[3]).collect::<Vec<_>>(),
        vec![255, 0, 255, 255]
    );
    Ok(())
}

#[test]
fn union_uses_maximum_coverage_and_rejects_empty_or_mismatched_masks() -> Result<()> {
    assert_eq!(
        pixels::union_masks(&[vec![20, 0, 90, 0], vec![0, 40, 80, 255]], 2, 2)?,
        vec![20, 40, 90, 255]
    );
    assert!(pixels::union_masks(&[], 2, 2).is_err());
    assert!(pixels::union_masks(&[vec![0; 4]], 2, 2).is_err());
    assert!(pixels::union_masks(&[vec![1; 3]], 2, 2).is_err());
    Ok(())
}

#[test]
fn circular_feather_support_is_inside_the_independent_disk_definition() -> Result<()> {
    let mask = pixels::rectangle_mask(
        31,
        29,
        Rect {
            x: 15,
            y: 14,
            width: 1,
            height: 1,
        },
    )?;
    for radius in [0, 1, 3, 8] {
        let feather = pixels::feather(&mask, 31, 29, radius)?;
        let dilated = pixels::dilate(&mask, 31, 29, radius + 1)?;
        for y in 0..29i32 {
            for x in 0..31i32 {
                let squared = (x - 15).pow(2) + (y - 14).pow(2);
                let index = (y * 31 + x) as usize;
                assert_eq!(dilated[index], squared <= (radius as i32 + 1).pow(2));
                if feather[index] > 0.0 {
                    assert!(squared <= (radius as i32).pow(2));
                }
            }
        }
    }
    Ok(())
}

#[test]
fn five_offline_shapes_prove_containment_even_when_mock_changes_every_pixel() -> Result<()> {
    let cases = [
        Rect {
            x: 30,
            y: 20,
            width: 20,
            height: 15,
        },
        Rect {
            x: 3,
            y: 4,
            width: 60,
            height: 31,
        },
        Rect {
            x: 10,
            y: 10,
            width: 70,
            height: 9,
        },
        Rect {
            x: 0,
            y: 55,
            width: 20,
            height: 17,
        },
        Rect {
            x: 90,
            y: 2,
            width: 1,
            height: 1,
        },
    ];
    for rect in cases {
        let request = prepared(rect, 8)?;
        let before = request.source.pixels.clone();
        let response = MockTransport {
            color: [7, 65, 240, 177],
        }
        .edit(&request)?;
        let result = request.finish(response)?;
        assert!(result.mock);
        assert_eq!(result.proof.changed_outside, 0);
        assert_eq!(
            result.proof.outside_sha256_before,
            result.proof.outside_sha256_after
        );
        assert_ne!(
            result.proof.source_pixels_sha256, result.proof.result_pixels_sha256,
            "selection {rect:?}"
        );
        assert_eq!(request.source.pixels, before);
        assert!(result.actual_microusd.is_none());
    }
    Ok(())
}

#[test]
fn proof_detects_a_single_exterior_color_or_alpha_change() -> Result<()> {
    let source = image(32, 32);
    let mask = pixels::rectangle_mask(
        32,
        32,
        Rect {
            x: 16,
            y: 16,
            width: 2,
            height: 2,
        },
    )?;
    for channel in [0, 3] {
        let mut changed = source.clone();
        changed.get_pixel_mut(0, 0)[channel] ^= 1;
        let proof = proof::compare(&source, &changed, &mask, 3, &"1".repeat(64), None)?;
        assert_eq!(proof.changed_outside, 1);
        assert!(matches!(
            proof.require_unchanged_exterior(),
            Err(Error::Exterior(1))
        ));
    }
    Ok(())
}

#[test]
fn identity_has_zero_delta_and_unit_ssim_and_ciede2000_matches_reference_pairs() -> Result<()> {
    let source = image(15, 17);
    let report = proof::compare(
        &source,
        &source,
        &vec![255; 15 * 17],
        0,
        &"a".repeat(64),
        None,
    )?;
    assert_eq!(report.metrics.delta_e2000_mean, 0.0);
    assert!((report.metrics.ssim_inside_mask - 1.0).abs() < 1e-12);
    for (first, second, expected) in [
        ([50., 2.6772, -79.7751], [50., 0., -82.7485], 2.0425),
        ([50., 3.1571, -77.2803], [50., 0., -82.7485], 2.8615),
        ([50., 2.8361, -74.0200], [50., 0., -82.7485], 3.4412),
        ([50., 0., 0.], [50., -1., 2.], 2.3669),
    ] {
        assert!((proof::delta_e2000(first, second) - expected).abs() < 0.0001);
        assert!(
            (proof::delta_e2000(first, second) - proof::delta_e2000(second, first)).abs() < 1e-12
        );
    }
    Ok(())
}

#[test]
fn profiles_roundtrip_without_reencoding_original_pixels_and_untagged_is_explicit() -> Result<()> {
    let source = image(19, 17);
    let profile = moxcms::ColorProfile::new_display_p3()
        .encode()
        .map_err(|_| Error::Color)?;
    let png = pixels::encode_png(&source, Some(&profile))?;
    let decoded = pixels::decode(&png, false)?;
    assert_eq!(decoded.pixels, source);
    assert_eq!(decoded.icc.as_deref(), Some(profile.as_slice()));
    let converted = pixels::transform(&source, Some(&profile), true)?;
    assert_ne!(converted, source);
    let mut untagged = Vec::new();
    image::codecs::png::PngEncoder::new(&mut untagged).write_image(
        source.as_raw(),
        19,
        17,
        image::ExtendedColorType::Rgba8,
    )?;
    assert!(pixels::decode(&untagged, false).is_err());
    assert!(pixels::decode(&untagged, true)?.assumed_srgb);
    assert!(pixels::transform(&source, Some(b"not an ICC profile"), true).is_err());
    Ok(())
}

#[test]
fn lanczos_identity_keeps_hidden_alpha_pixels_and_resizing_is_bounded() -> Result<()> {
    let source = RgbaImage::from_pixel(3, 2, Rgba([123, 87, 62, 0]));
    assert_eq!(pixels::resize(&source, 3, 2)?, source);
    assert!(pixels::resize(&source, 0, 2).is_err());
    assert!(pixels::resize(&source, 16384, 16384).is_err());
    assert!(
        pixels::crop_padded(
            &source,
            Rect {
                x: i32::MAX,
                y: 0,
                width: 1,
                height: 1
            }
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn lanczos_resampling_preserves_constant_colors_and_excludes_hidden_transparent_rgb() -> Result<()>
{
    for color in [[7, 65, 240, 177], [255, 255, 255, 255], [123, 87, 62, 17]] {
        let source = RgbaImage::from_pixel(7, 5, Rgba(color));
        for (width, height) in [(19, 13), (2, 3), (1, 1)] {
            let resized = pixels::resize(&source, width, height)?;
            assert!(
                resized.pixels().all(|pixel| pixel.0 == color),
                "constant color {color:?}"
            );
        }
    }
    let mut source = RgbaImage::from_pixel(2, 1, Rgba([255, 0, 0, 255]));
    source.put_pixel(1, 0, Rgba([0, 255, 255, 0]));
    let resized = pixels::resize(&source, 1, 1)?;
    let sample = resized.get_pixel(0, 0).0;
    assert_eq!(&sample[..3], &[255, 0, 0]);
    assert!((127..=128).contains(&sample[3]));
    Ok(())
}

#[test]
fn prepared_wire_request_is_bound_and_uses_current_supported_fields_only() -> Result<()> {
    let mut prepared = prepared(
        Rect {
            x: 8,
            y: 8,
            width: 2,
            height: 2,
        },
        0,
    )?;
    let multipart = prepared.multipart()?;
    let body = String::from_utf8_lossy(&multipart.body);
    assert!(body.contains("name=\"image\"; filename=\"image.png\""));
    assert!(body.contains("name=\"mask\"; filename=\"mask.png\""));
    assert!(body.contains("name=\"output_format\"\r\n\r\npng"));
    assert!(
        !body.contains("response_format")
            && !body.contains("input_fidelity")
            && !body.contains("Authorization")
    );
    prepared.description.prompt.push('x');
    assert!(prepared.multipart().is_err());
    Ok(())
}

#[test]
fn changing_prepared_source_mask_or_profile_invalidates_the_request_binding() -> Result<()> {
    for change in 0..3 {
        let mut request = prepared(
            Rect {
                x: 8,
                y: 8,
                width: 2,
                height: 2,
            },
            0,
        )?;
        match change {
            0 => request.source.pixels.get_pixel_mut(0, 0)[0] ^= 1,
            1 => request.mask[0] ^= 1,
            _ => request.source.icc = Some(vec![1, 2, 3]),
        }
        assert!(request.multipart().is_err());
    }
    Ok(())
}

#[test]
fn mask_alpha_dimensions_and_sixteen_bit_inputs_fail_explicitly() -> Result<()> {
    let mut rgb = Vec::new();
    image::codecs::png::PngEncoder::new(&mut rgb).write_image(
        &[12, 34, 56],
        1,
        1,
        image::ExtendedColorType::Rgb8,
    )?;
    assert!(pixels::decode_mask(&rgb, 1, 1).is_err());
    let rgba = pixels::encode_png(&image(2, 2), None)?;
    assert!(pixels::decode_mask(&rgba, 3, 2).is_err());
    let mut sixteen = Vec::new();
    image::codecs::png::PngEncoder::new(&mut sixteen).write_image(
        &[0, 0, 0, 0, 0, 0, 255, 255],
        1,
        1,
        image::ExtendedColorType::Rgba16,
    )?;
    assert!(pixels::decode(&sixteen, true).is_err());
    Ok(())
}

#[test]
fn malformed_provider_responses_and_wrong_dimensions_cannot_be_successful() -> Result<()> {
    for bytes in [
        b"{}".as_slice(),
        b"{\"data\":[]}",
        b"{\"data\":[{\"url\":\"https://example.invalid/private\"}]}",
        b"{\"data\":[{\"b64_json\":\"%%%\"}]}",
    ] {
        assert!(request::parse_response(bytes, 10000, 0).is_err());
    }
    let png = pixels::encode_png(&image(2, 2), None)?;
    let response = serde_json::to_vec(
        &serde_json::json!({"data":[{"b64_json":STANDARD.encode(&png)}],
        "usage":{"input_tokens":3,"input_tokens_details":{"text_tokens":1,"image_tokens":2},"output_tokens":4,"total_tokens":7}}),
    )?;
    let parsed = request::parse_response(&response, 10000, 1)?;
    assert_eq!(
        parsed.tokens,
        Some(Tokens {
            text_input: 1,
            image_input: 2,
            image_output: 4
        })
    );
    assert!(request::parse_response(&response, response.len() - 1, 1).is_err());
    assert!(
        prepared(
            Rect {
                x: 8,
                y: 8,
                width: 4,
                height: 4
            },
            8
        )?
        .finish(parsed)
        .is_err()
    );
    Ok(())
}

#[test]
fn no_estimate_or_mismatched_confirmation_cannot_create_a_budget_reservation() -> Result<()> {
    let temporary = temporary();
    let path = temporary.path().join("budget.json");
    assert!(matches!(
        Reservation::begin(&path, &test_config(), &"a".repeat(64), &"a".repeat(64), 1),
        Err(Error::Confirmation)
    ));
    assert!(!path.exists());
    let mut config = test_config();
    config.token_estimate = Some(Tokens {
        text_input: 1,
        image_input: 1,
        image_output: 1,
    });
    assert!(matches!(
        Reservation::begin(&path, &config, &"a".repeat(64), &"b".repeat(64), 1),
        Err(Error::Confirmation)
    ));
    assert!(!path.exists());
    temporary.close()?;
    Ok(())
}

#[test]
fn budget_lock_unresolved_charges_duplicates_and_limit_all_fail_closed() -> Result<()> {
    let temporary = temporary();
    let path = temporary.path().join("budget.json");
    let mut config = test_config();
    config.token_estimate = Some(Tokens {
        text_input: 10_000,
        image_input: 10_000,
        image_output: 10_000,
    });
    config.spike_budget_microusd = 600_000;
    let mut first = Reservation::begin(&path, &config, &"a".repeat(64), &"a".repeat(64), 1)?;
    assert!(Reservation::begin(&path, &config, &"b".repeat(64), &"b".repeat(64), 1).is_err());
    first.begin_attempt(&"a".repeat(64), &config)?;
    first.settle(Some(300_000))?;
    assert!(Reservation::begin(&path, &config, &"a".repeat(64), &"a".repeat(64), 1).is_err());
    assert!(matches!(
        Reservation::begin(&path, &config, &"b".repeat(64), &"b".repeat(64), 1),
        Err(Error::Budget)
    ));
    config.spike_budget_microusd = 2_000_000;
    drop(Reservation::begin(
        &path,
        &config,
        &"b".repeat(64),
        &"b".repeat(64),
        1,
    )?);
    assert!(Reservation::begin(&path, &config, &"c".repeat(64), &"c".repeat(64), 2).is_err());
    temporary.close()?;
    Ok(())
}

#[test]
fn known_not_sent_can_retry_and_invalid_confirmation_never_reaches_credentials() -> Result<()> {
    let temporary = temporary();
    let path = temporary.path().join("budget.json");
    let mut request = prepared(
        Rect {
            x: 8,
            y: 8,
            width: 4,
            height: 4,
        },
        0,
    )?;
    request.description.provider.token_estimate = Some(Tokens {
        text_input: 1,
        image_input: 1,
        image_output: 1,
    });
    let id = "a".repeat(64);
    Reservation::begin(&path, &request.description.provider, &id, &id, 1)?.not_sent()?;
    let mut reservation = Reservation::begin(&path, &request.description.provider, &id, &id, 1)?;
    assert!(matches!(
        vw_ai_spike::platform::send_confirmed(&request, "wrong", &mut reservation),
        Err(Error::Confirmation)
    ));
    reservation.not_sent()?;
    temporary.close()?;
    Ok(())
}

#[test]
fn corrupt_budget_and_oversized_input_reject_without_network_or_allocation() -> Result<()> {
    let temporary = temporary();
    let path = temporary.path().join("budget.json");
    std::fs::write(&path, b"not JSON")?;
    let mut config = test_config();
    config.token_estimate = Some(Tokens {
        text_input: 1,
        image_input: 1,
        image_output: 1,
    });
    assert!(Reservation::begin(&path, &config, &"a".repeat(64), &"a".repeat(64), 1).is_err());
    assert!(
        pixels::rectangle_mask(
            16384,
            16384,
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }
        )
        .is_err()
    );
    assert!(pixels::decode(b"not an image", true).is_err());
    config.endpoint = "https://example.invalid/v1/images/edits".into();
    assert!(config.validate().is_err());
    temporary.close()?;
    Ok(())
}

#[test]
fn reservation_attempt_is_single_use_and_survives_an_interrupted_process() -> Result<()> {
    let temporary = temporary();
    let path = temporary.path().join("budget.json");
    let mut config = test_config();
    config.token_estimate = Some(Tokens {
        text_input: 1,
        image_input: 1,
        image_output: 1,
    });
    let id = "a".repeat(64);
    let mut reservation = Reservation::begin(&path, &config, &id, &id, 1)?;
    reservation.begin_attempt(&id, &config)?;
    assert!(matches!(
        reservation.begin_attempt(&id, &config),
        Err(Error::AttemptConsumed)
    ));
    // A repeated public adapter call is rejected before request validation,
    // credentials or platform HTTP, even if its confirmation is also invalid.
    let request = prepared(
        Rect {
            x: 8,
            y: 8,
            width: 2,
            height: 2,
        },
        0,
    )?;
    assert!(matches!(
        vw_ai_spike::platform::send_confirmed(&request, "wrong", &mut reservation),
        Err(Error::AttemptConsumed)
    ));
    drop(reservation); // Same durable state as process loss after the attempt barrier.
    let ledger: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(ledger["entries"][0]["state"], "attempted");
    assert!(Reservation::begin(&path, &config, &id, &id, 1).is_err());
    assert!(Reservation::begin(&path, &config, &"b".repeat(64), &"b".repeat(64), 2).is_err());
    temporary.close()?;
    Ok(())
}

#[test]
fn reservation_binds_configuration_and_retains_ambiguous_outcomes() -> Result<()> {
    let temporary = temporary();
    let path = temporary.path().join("budget.json");
    let mut config = test_config();
    config.token_estimate = Some(Tokens {
        text_input: 1,
        image_input: 1,
        image_output: 1,
    });
    let id = "a".repeat(64);
    let mut reservation = Reservation::begin(&path, &config, &id, &id, 1)?;
    let mut other = config.clone();
    other.token_estimate = Some(Tokens {
        text_input: 2,
        image_input: 1,
        image_output: 1,
    });
    assert!(matches!(
        reservation.begin_attempt(&id, &other),
        Err(Error::Confirmation)
    ));
    assert!(matches!(
        reservation.begin_attempt(&"b".repeat(64), &config),
        Err(Error::Confirmation)
    ));
    reservation.begin_attempt(&id, &config)?;
    reservation.settle(None)?;
    assert!(Reservation::begin(&path, &config, &"b".repeat(64), &"b".repeat(64), 2).is_err());
    temporary.close()?;
    Ok(())
}

#[test]
fn confirmed_pre_send_failure_can_release_an_attempt_without_hiding_ambiguous_calls() -> Result<()>
{
    let temporary = temporary();
    let path = temporary.path().join("budget.json");
    let mut config = test_config();
    config.token_estimate = Some(Tokens {
        text_input: 1,
        image_input: 1,
        image_output: 1,
    });
    let id = "a".repeat(64);
    let mut reservation = Reservation::begin(&path, &config, &id, &id, 1)?;
    reservation.begin_attempt(&id, &config)?;
    reservation.not_sent()?;
    let mut retry = Reservation::begin(&path, &config, &id, &id, 1)?;
    retry.begin_attempt(&id, &config)?;
    retry.settle(Some(43))?;
    assert!(Reservation::begin(&path, &config, &id, &id, 2).is_err());
    temporary.close()?;
    Ok(())
}

#[test]
fn normalized_profile_binding_covers_tagged_and_explicitly_assumed_sources() -> Result<()> {
    let mut request = prepared(
        Rect {
            x: 8,
            y: 8,
            width: 2,
            height: 2,
        },
        0,
    )?;
    assert!(request.description.source_icc_sha256.is_none());
    let source_png = pixels::encode_png(&request.source.pixels, None)?;
    let source = pixels::decode(&source_png, false)?;
    assert_eq!(
        source.icc.as_deref().map(sha256),
        Some(request.description.normalized_source_icc_sha256.clone())
    );
    request.description.normalized_source_icc_sha256 = "0".repeat(64);
    assert!(request.multipart().is_err());
    Ok(())
}

#[test]
fn generated_srgb_profile_and_request_binding_have_no_wall_clock_metadata() -> Result<()> {
    let profile = pixels::output_profile(None)?;
    assert_eq!(
        &profile[24..36],
        &[0x07, 0xd0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(&profile[84..100], &[0; 16]);
    assert_eq!(profile, pixels::output_profile(None)?);
    assert!(moxcms::ColorProfile::new_from_slice(&profile).is_ok());
    // Supplied ICC bytes, including their original date, remain unchanged.
    let mut supplied = profile.clone();
    supplied[25] = 0xe0;
    assert_eq!(pixels::output_profile(Some(&supplied))?, supplied);
    let region = Rect {
        x: 8,
        y: 8,
        width: 2,
        height: 2,
    };
    let first = prepared(region, 0)?;
    let second = prepared(region, 0)?;
    assert_eq!(first.request_id, second.request_id);
    assert_eq!(first.image_png, second.image_png);
    assert_eq!(first.mask_png, second.mask_png);
    Ok(())
}
