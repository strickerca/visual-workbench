mod common;
use common::*;
use std::sync::atomic::AtomicBool;
use vw_ai::{config::*, *};
use vw_mask::{Mask, Size};
use vw_raster::Pixels;

#[test]
fn preparation_admits_retained_mask_bytes_before_starting_source_decode() -> TestResult {
    // Clones share allocations, but admission deliberately counts each supplied
    // mask conservatively. This fixture exercises the production path with only
    // one 64 KiB backing tile, not a 250 MiB test allocation.
    let mask = Mask::from_dense(Size::new(256, 256)?, &vec![255; 256 * 256])?;
    let masks = vec![mask; 128];
    let mut options = options(1)?;
    options.limits.memory_bytes = 20 * 1024 * 1024;
    assert!(matches!(
        Prepared::new(&[0xff], &masks, options, &NeverCancel),
        Err(Error::Limit(_))
    ));
    Ok(())
}

#[test]
fn configuration_is_typed_bounded_and_prices_round_once() -> TestResult {
    let p = provider();
    assert_eq!(ProviderConfig::from_json(&serde_json::to_vec(&p)?)?, p);
    let cost = Prices {
        text_input_microusd_per_million: 1,
        image_input_microusd_per_million: 1,
        image_output_microusd_per_million: 1,
    };
    assert_eq!(
        cost.cost(Tokens {
            text_input: 1,
            image_input: 1,
            image_output: 1
        })?,
        1
    );
    assert_eq!(
        cost.cost(Tokens {
            text_input: 500_000,
            image_input: 500_000,
            image_output: 1
        })?,
        2
    );
    assert!(
        Prices {
            text_input_microusd_per_million: u64::MAX,
            image_input_microusd_per_million: u64::MAX,
            image_output_microusd_per_million: u64::MAX
        }
        .cost(Tokens {
            text_input: u64::MAX,
            image_input: u64::MAX,
            image_output: u64::MAX
        })
        .is_err()
    );
    let mut invalid = p.clone();
    invalid.endpoint = "https://different.invalid/".into();
    assert_eq!(invalid.validate(), Err(Error::Unsupported));
    invalid = p.clone();
    invalid.capabilities.size_multiple = 0;
    assert!(invalid.validate().is_err());
    invalid = p.clone();
    invalid.capabilities.size_multiple = 1;
    invalid.capabilities.max_long_edge = 16384;
    assert!(invalid.validate().is_err());
    let mut json = serde_json::to_value(&p)?;
    json["unknown"] = 1.into();
    assert!(ProviderConfig::from_json(&serde_json::to_vec(&json)?).is_err());
    Ok(())
}

#[test]
fn crop_known_answer_and_extreme_aspect_never_discard_selection() -> TestResult {
    let mut caps = provider().capabilities;
    caps.max_long_edge = 4096;
    caps.max_pixels = 20_000_000;
    caps.size_multiple = 64;
    let mask = region(1000, 1000, 300, 300, 400, 500)?.to_dense()?;
    let plan = geometry::plan(&mask, 1000, 1000, &caps)?;
    assert_eq!(
        plan.crop,
        geometry::Rect {
            x: 236,
            y: 225,
            width: 228,
            height: 350
        }
    );
    assert!(!plan.has_padding);
    let plan = geometry::plan(&vec![255; 4096 * 64], 4096, 64, &caps)?;
    assert_eq!((plan.crop.width, plan.crop.height), (4096, 1366));
    assert!(plan.crop.y < 0 && plan.has_padding);
    assert!(plan.crop.y + plan.crop.height as i32 >= 64);
    assert!(caps.valid_size(plan.model_width, plan.model_height));
    Ok(())
}

#[test]
fn binding_is_stable_and_covers_every_reviewed_input() -> TestResult {
    let source = png8(64, 64, |_, _| [70, 90, 110, 255])?;
    let mask = region(64, 64, 20, 20, 44, 44)?;
    let first = Prepared::new(
        &source,
        std::slice::from_ref(&mask),
        options(1)?,
        &NeverCancel,
    )?;
    let second = Prepared::new(
        &source,
        std::slice::from_ref(&mask),
        options(1)?,
        &NeverCancel,
    )?;
    assert_eq!(first.review().request_id(), second.review().request_id());
    assert_eq!(first.request_image_png(), second.request_image_png());
    for case in 0..6 {
        let mut opt = options(1)?;
        match case {
            0 => opt.intent_id = vw_model::Id::from_parts(2, [1; 10])?,
            1 => opt.revision.host_seq += 1,
            2 => opt.provider.prices.image_output_microusd_per_million += 1,
            3 => opt.instructions[0].text.push('!'),
            4 => opt.feather_px += 1,
            _ => {
                opt.estimated_tokens = Some(Tokens {
                    text_input: 11,
                    image_input: 20,
                    image_output: 30,
                })
            }
        }
        let other = Prepared::new(&source, std::slice::from_ref(&mask), opt, &NeverCancel)?;
        assert_ne!(first.review().request_id(), other.review().request_id());
    }
    let different = Prepared::new(
        &source,
        &[region(64, 64, 21, 20, 44, 44)?],
        options(1)?,
        &NeverCancel,
    )?;
    assert_ne!(first.review().request_id(), different.review().request_id());
    Ok(())
}

#[test]
fn change_masks_union_and_provider_alpha_inversion_are_exact() -> TestResult {
    let source = png8(64, 64, |_, _| [20, 40, 60, 255])?;
    let a = region(64, 64, 0, 0, 10, 10)?;
    let b = region(64, 64, 30, 30, 40, 40)?;
    let p = Prepared::new(&source, &[a, b], options(1)?, &NeverCancel)?;
    let mask = read_rgba(p.request_mask_png())?;
    assert_eq!(mask.dimensions(), (64, 64));
    assert_eq!(mask.get_pixel(5, 5)[3], 0);
    assert_eq!(mask.get_pixel(35, 35)[3], 0);
    assert_eq!(mask.get_pixel(20, 20)[3], 255);
    Ok(())
}

#[test]
fn normalized_premultiplied_lanczos_preserves_constant_color_on_up_and_down_scale() -> TestResult {
    for source_size in [24, 192] {
        for alpha in [255, 127] {
            let color = [70, 140, 210, alpha];
            let source = png8(source_size, source_size, |_, _| color)?;
            let mut opt = options(1)?;
            opt.provider.capabilities.max_long_edge = 64;
            opt.provider.capabilities.min_pixels = 4096;
            opt.provider.capabilities.max_pixels = 4096;
            opt.feather_px = 0;
            let p = Prepared::new(
                &source,
                &[region(
                    source_size,
                    source_size,
                    0,
                    0,
                    source_size,
                    source_size,
                )?],
                opt,
                &NeverCancel,
            )?;
            let request = read_rgba(p.request_image_png())?;
            assert_eq!(request.dimensions(), (64, 64));
            for pixel in request.pixels() {
                for c in 0..4 {
                    assert!(
                        pixel[c].abs_diff(color[c]) <= 1,
                        "channel {c}: {} != {}",
                        pixel[c],
                        color[c]
                    );
                }
            }
            let completed = p.finish(p.mock_response(color)?, &NeverCancel)?;
            let Pixels::Rgba8(pixels) = completed.image().pixels() else {
                return Err("expected 8-bit".into());
            };
            for pixel in pixels.as_chunks::<4>().0.iter() {
                for c in 0..4 {
                    assert!(pixel[c].abs_diff(color[c]) <= 1);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn hidden_transparent_red_cannot_bleed_into_blue_resize() -> TestResult {
    for source_size in [24, 192] {
        for direction in [0, 1, 2] {
            for color in [[0, 0, 200, 255], [33, 77, 255, 127]] {
                let source = png8(source_size, source_size, |x, y| {
                    let transparent = match direction {
                        0 => x < source_size / 2,
                        1 => y < source_size / 2,
                        _ => x + y < source_size,
                    };
                    if transparent { [255, 0, 0, 0] } else { color }
                })?;
                let mut opt = options(1)?;
                opt.provider.capabilities.min_pixels = 4096;
                opt.provider.capabilities.max_pixels = 4096;
                opt.provider.capabilities.max_long_edge = 64;
                let p = Prepared::new(
                    &source,
                    &[region(
                        source_size,
                        source_size,
                        0,
                        0,
                        source_size,
                        source_size,
                    )?],
                    opt,
                    &NeverCancel,
                )?;
                let image = read_rgba(p.request_image_png())?;
                for pixel in image.pixels().filter(|pixel| pixel[3] > 0) {
                    for channel in 0..3 {
                        assert!(
                            pixel[channel].abs_diff(color[channel]) <= 1,
                            "size {source_size}, direction {direction}, channel {channel}: {pixel:?} != {color:?}"
                        );
                    }
                }
                assert!(image.pixels().any(|pixel| pixel[3] > 0));
            }
        }
    }
    Ok(())
}

#[test]
fn malicious_full_frame_model_changes_cannot_escape_a_naively_checked_disk() -> TestResult {
    let source = png8(128, 128, |x, y| [x as u8, y as u8, 11, (90 + x % 30) as u8])?;
    let mask = region(128, 128, 1, 1, 6, 6)?;
    let mut opt = options(2)?;
    opt.feather_px = 3;
    let p = Prepared::new(&source, &[mask], opt, &NeverCancel)?;
    let completed = p.finish(p.mock_response([230, 160, 90, 250])?, &NeverCancel)?;
    assert_eq!(completed.proof().changed_outside, 0);
    assert_eq!(
        completed.proof().outside_sha256_before,
        completed.proof().outside_sha256_after
    );
    let Pixels::Rgba8(a) = p.source().pixels() else {
        return Err("8-bit required".into());
    };
    let Pixels::Rgba8(b) = completed.image().pixels() else {
        return Err("8-bit required".into());
    };
    let mut changed = 0;
    for y in 0i32..128 {
        for x in 0i32..128 {
            let inside = (1i32..6)
                .any(|my| (1i32..6).any(|mx| (x - mx) * (x - mx) + (y - my) * (y - my) <= 16));
            let i = (y * 128 + x) as usize * 4;
            if !inside {
                assert_eq!(&a[i..i + 4], &b[i..i + 4]);
            }
            if a[i..i + 4] != b[i..i + 4] {
                changed += 1;
            }
        }
    }
    assert!(changed > 0);
    Ok(())
}

#[test]
fn sixteen_bit_original_low_bits_and_alpha_survive_exterior_and_export() -> TestResult {
    let source = png16(64, 64)?;
    let mask = region(64, 64, 20, 20, 40, 40)?;
    assert!(matches!(
        Prepared::new(
            &source,
            std::slice::from_ref(&mask),
            options(1)?,
            &NeverCancel
        ),
        Err(Error::Depth)
    ));
    let mut opt = options(1)?;
    opt.source_policy.allow_16bit_provider_copy = true;
    opt.feather_px = 0;
    let p = Prepared::new(&source, &[mask], opt, &NeverCancel)?;
    let done = p.finish(p.mock_response([200, 20, 40, 110])?, &NeverCancel)?;
    assert_eq!(done.image().bit_depth(), 16);
    let (Pixels::Rgba16(a), Pixels::Rgba16(b)) = (p.source().pixels(), done.image().pixels())
    else {
        return Err("16-bit required".into());
    };
    assert_eq!(&a[..80], &b[..80]);
    assert_ne!(
        &a[26 * 64 * 4 + 26 * 4..26 * 64 * 4 + 26 * 4 + 4],
        &b[26 * 64 * 4 + 26 * 4..26 * 64 * 4 + 26 * 4 + 4]
    );
    let encoded = done.image().encode_png(Limits::default(), &NeverCancel)?;
    let proof = p.verify_candidate(&encoded, &NeverCancel)?;
    assert_eq!(proof.changed_outside, 0);
    assert_eq!(
        proof.result_pixels_sha256,
        done.proof().result_pixels_sha256
    );
    Ok(())
}

#[test]
fn partial_accept_keeps_all_zero_selection_pixels_exact_and_supports_empty() -> TestResult {
    let p = prepared(1)?;
    let completed = p.finish(p.mock_response([180, 70, 210, 240])?, &NeverCancel)?;
    let accept = region(64, 64, 28, 28, 33, 33)?;
    let partial = completed.accept_part(&accept, &NeverCancel)?;
    let (Pixels::Rgba8(a), Pixels::Rgba8(b)) = (p.source().pixels(), partial.image().pixels())
    else {
        return Err("8-bit required".into());
    };
    for y in 0..64 {
        for x in 0..64 {
            if accept.coverage(x, y) == 0 {
                let i = (y * 64 + x) as usize * 4;
                assert_eq!(&a[i..i + 4], &b[i..i + 4]);
            }
        }
    }
    let acceptance = partial
        .proof()
        .acceptance
        .as_ref()
        .ok_or("acceptance proof")?;
    assert_eq!(acceptance.changed_unaccepted, 0);
    assert_eq!(acceptance.before_sha256, acceptance.after_sha256);
    let empty = completed.accept_part(&Mask::empty(Size::new(64, 64)?), &NeverCancel)?;
    assert_eq!(empty.image().pixels(), p.source().pixels());
    assert!(
        completed
            .accept_part(&Mask::empty(Size::new(1, 1)?), &NeverCancel)
            .is_err()
    );
    Ok(())
}

#[test]
fn compare_views_use_source_result_and_make_alpha_differences_visible() -> TestResult {
    let p = prepared(1)?;
    let done = p.finish(p.mock_response([250, 10, 80, 0])?, &NeverCancel)?;
    let source = done.compare(CompareMode::Blink { show_result: false }, &NeverCancel)?;
    let result = done.compare(CompareMode::Blink { show_result: true }, &NeverCancel)?;
    let wipe = done.compare(
        CompareMode::Wipe {
            axis: Axis::Horizontal,
            cut: 32,
            result_before: true,
        },
        &NeverCancel,
    )?;
    for y in 0..64 {
        for x in 0..64 {
            let i = (y * 64 + x) * 4;
            assert_eq!(
                &wipe.rgba_srgb[i..i + 4],
                if x < 32 {
                    &result.rgba_srgb[i..i + 4]
                } else {
                    &source.rgba_srgb[i..i + 4]
                }
            );
        }
    }
    let split = done.compare(CompareMode::Split, &NeverCancel)?;
    assert_eq!((split.width, split.height), (128, 64));
    let difference = done.compare(CompareMode::Difference { gain: 2 }, &NeverCancel)?;
    assert!(
        difference
            .rgba_srgb
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[..3] != [0, 0, 0])
    );
    assert!(
        done.compare(
            CompareMode::Wipe {
                axis: Axis::Vertical,
                cut: 65,
                result_before: false
            },
            &NeverCancel
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn proof_audit_detects_one_exterior_alpha_bit_without_accepting_stale_metadata() -> TestResult {
    let p = prepared(1)?;
    let done = p.finish(p.mock_response([180, 70, 210, 240])?, &NeverCancel)?;
    let encoded = done.image().encode_png(Limits::default(), &NeverCancel)?;
    assert_eq!(
        p.verify_candidate(&encoded, &NeverCancel)?.changed_outside,
        0
    );
    let mut decoded = vw_raster::decode(&encoded, vw_raster::DecodeLimits::default())?;
    if let Pixels::Rgba8(data) = &mut decoded.pixels {
        data[3] ^= 1;
    }
    // Re-encode with exactly the candidate ICC bytes, changing one outside bit.
    let Pixels::Rgba8(data) = decoded.pixels else {
        return Err("8-bit required".into());
    };
    let mut changed = Vec::new();
    {
        let mut info = png::Info::with_size(64, 64);
        info.color_type = png::ColorType::Rgba;
        info.bit_depth = png::BitDepth::Eight;
        info.icc_profile = decoded.icc.map(std::borrow::Cow::Owned);
        let mut writer = png::Encoder::with_info(&mut changed, info)?.write_header()?;
        writer.write_image_data(&data)?;
        writer.finish()?;
    }
    assert!(matches!(
        p.verify_candidate(&changed, &NeverCancel),
        Err(Error::Exterior(1))
    ));
    Ok(())
}

#[test]
fn known_color_difference_pairs_and_identical_metrics_are_stable() -> TestResult {
    for (a, b, expected) in [
        ([50., 2.6772, -79.7751], [50., 0., -82.7485], 2.0425),
        ([50., 3.1571, -77.2803], [50., 0., -82.7485], 2.8615),
        ([50., 2.8361, -74.0200], [50., 0., -82.7485], 3.4412),
    ] {
        assert!((proof::delta_e2000(a, b) - expected).abs() < 0.0001);
        assert!((proof::delta_e2000(a, b) - proof::delta_e2000(b, a)).abs() < 1e-12);
    }
    let p = prepared(1)?;
    let source = p.source().encode_png(Limits::default(), &NeverCancel)?;
    let report = p.verify_candidate(&source, &NeverCancel)?;
    assert_eq!(report.metrics.delta_e2000_mean, 0.0);
    assert!((report.metrics.ssim_inside_mask - 1.0).abs() < 1e-12);
    Ok(())
}

#[test]
fn cancellation_empty_masks_small_budgets_and_wrong_sizes_fail_before_mutation() -> TestResult {
    let p = prepared(1)?;
    let original = p.source().pixels().clone();
    let stop = AtomicBool::new(true);
    assert!(matches!(
        p.finish(p.mock_response([1, 2, 3, 4])?, &stop),
        Err(Error::Cancelled)
    ));
    assert_eq!(p.source().pixels(), &original);
    let source = png8(64, 64, |_, _| [1, 2, 3, 255])?;
    assert!(
        Prepared::new(
            &source,
            &[Mask::empty(Size::new(64, 64)?)],
            options(1)?,
            &NeverCancel
        )
        .is_err()
    );
    let mut small = options(1)?;
    small.limits.memory_bytes = 1024;
    assert!(matches!(
        Prepared::new(&source, &[region(64, 64, 2, 2, 3, 3)?], small, &NeverCancel),
        Err(Error::Limit(_))
    ));
    assert!(
        Prepared::new(
            &source,
            &[region(32, 32, 2, 2, 3, 3)?],
            options(1)?,
            &NeverCancel
        )
        .is_err()
    );
    assert!(matches!(
        Prepared::new(&source, &[region(64, 64, 2, 2, 3, 3)?], options(1)?, &stop),
        Err(Error::Cancelled)
    ));
    Ok(())
}
