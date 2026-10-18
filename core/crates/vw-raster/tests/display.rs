mod support;

use std::cell::Cell;
use support::*;
use vw_raster::*;

const BUDGET: u64 = 256 * 1024 * 1024;
const FIXED_WORKSPACE: u64 = 32 * 1024 * 1024;

#[test]
fn display_admission_counts_conversion_resident_capacity_and_profile() -> TestResult {
    for depth in [8, 16] {
        let mut image = source(2, 2, depth);
        let (resident, samples) = match &mut image.pixels {
            Pixels::Rgba8(values) => {
                values.reserve_exact(8192);
                (values.capacity() as u64, values.len() as u64)
            }
            Pixels::Rgba16(values) => {
                values.reserve_exact(8192);
                (values.capacity() as u64 * 2, values.len() as u64 * 2)
            }
        };
        let expected = resident + samples * 2 + FIXED_WORKSPACE;
        // The original over-capacity Vec must be admitted, not merely its length.
        assert!(matches!(
            display_rgba8(image, expected - 1, true, &|| false),
            Err(RasterError::Memory { estimated, budget })
                if estimated == expected && budget == expected - 1
        ));
    }
    Ok(())
}

#[test]
fn profile_alias_workspace_is_admitted_before_semantic_parsing() -> TestResult {
    let mut image = source(1, 1, 8);
    image.icc = Some(aliased_profile()?);
    assert!(matches!(image.validate(), Err(RasterError::Color)));
    assert!(matches!(
        display_rgba8(image.clone(), FIXED_WORKSPACE + 4096, true, &|| false),
        Err(RasterError::Memory { estimated, budget }) if estimated > budget
    ));
    assert!(matches!(
        display_rgba8(image, BUDGET, true, &|| false),
        Err(RasterError::Color)
    ));
    Ok(())
}

#[test]
fn direct_display_preserves_orientation_alpha_and_source_identity() -> TestResult {
    for orientation in 1..=8 {
        for depth in [8, 16] {
            for tagged in [false, true] {
                let mut image = source(3, 2, depth);
                image.orientation_applied = orientation;
                match &mut image.pixels {
                    Pixels::Rgba8(values) => {
                        for (pixel, alpha) in values
                            .as_chunks_mut::<4>()
                            .0
                            .iter_mut()
                            .zip([0, 1, 127, 128, 254, 255])
                        {
                            pixel[3] = alpha;
                        }
                    }
                    Pixels::Rgba16(values) => {
                        for (pixel, alpha) in values
                            .as_chunks_mut::<4>()
                            .0
                            .iter_mut()
                            .zip([0, 1, 128, 129, 32768, 65535])
                        {
                            pixel[3] = alpha;
                        }
                    }
                }
                if tagged {
                    image.icc = Some(standard_srgb_profile()?);
                }
                let expected = image.pixels.rgba8();
                let asset = image.source_asset.clone();
                let result = display_rgba8(image, BUDGET, !tagged, &|| false)?;
                assert_eq!((result.width, result.height), (3, 2));
                assert_eq!(result.orientation_applied, orientation);
                assert_eq!(result.source_bit_depth, depth);
                assert_eq!(result.source_asset, asset);
                assert_eq!(result.icc_profile, standard_srgb_profile()?);
                assert_eq!(result.rgba.len(), expected.len());
                for (actual, expected) in result
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(expected.as_chunks::<4>().0.iter())
                {
                    for channel in 0..3 {
                        assert!(actual[channel].abs_diff(expected[channel]) <= 1);
                    }
                    assert_eq!(
                        actual[3], expected[3],
                        "alpha must not be color transformed"
                    );
                }
                let expected_alpha: &[u8] = if depth == 16 {
                    &[0, 0, 0, 1, 128, 255]
                } else {
                    &[0, 1, 127, 128, 254, 255]
                };
                assert!(
                    result
                        .rgba
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(expected_alpha)
                        .all(|(rgba, alpha)| rgba[3] == *alpha)
                );
            }
        }
    }
    Ok(())
}

#[test]
fn direct_display_cancellation_at_conversion_boundary_returns_no_image() -> TestResult {
    for (depth, cancel_at) in [(8, 1), (8, 3), (16, 4)] {
        let mut image = source(3, 2, depth);
        image.icc = Some(standard_srgb_profile()?);
        let checks = Cell::new(0usize);
        assert!(matches!(
            display_rgba8(image, BUDGET, false, &|| {
                let current = checks.get() + 1;
                checks.set(current);
                current == cancel_at
            }),
            Err(RasterError::Cancelled)
        ));
        assert_eq!(checks.get(), cancel_at);
    }
    Ok(())
}

#[test]
fn display_shape_and_original_availability_are_checked_before_parsing() -> TestResult {
    let mut unavailable = source(1, 1, 8);
    unavailable.original_available = false;
    unavailable.icc = Some(vec![0; 132]);
    assert!(matches!(
        display_rgba8(unavailable, BUDGET, true, &|| false),
        Err(RasterError::OriginalRequired)
    ));
    let mut truncated = source(1, 1, 8);
    truncated.pixels = Pixels::Rgba8(vec![0; 3]);
    truncated.icc = Some(vec![0; 132]);
    assert!(matches!(
        display_rgba8(truncated, BUDGET, true, &|| false),
        Err(RasterError::Invalid("pixel buffer length"))
    ));
    let mut orientation = source(1, 1, 8);
    orientation.orientation_applied = 0;
    assert!(matches!(
        display_rgba8(orientation, BUDGET, true, &|| false),
        Err(RasterError::Invalid("orientation"))
    ));
    assert!(matches!(
        display_rgba8(source(1, 1, 8), BUDGET, false, &|| false),
        Err(RasterError::Unsupported(
            "untagged color; explicitly assume sRGB"
        ))
    ));
    let mut invalid_profile = source(1, 1, 8);
    invalid_profile.icc = Some(vec![0; 132]);
    assert!(matches!(
        display_rgba8(invalid_profile, BUDGET, true, &|| false),
        Err(RasterError::Color)
    ));
    Ok(())
}

// A structurally bounded ~16KiB ICC with repeated text aliases, but an invalid
// signature. Memory must win over semantic parsing under a small caller budget.
fn aliased_profile() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let base = standard_srgb_profile()?;
    let records = 8usize;
    let payload = 16_384usize;
    let strings = 16 + records * 12;
    let tag_size = strings + payload;
    let mut profile = vec![0; 144 + tag_size];
    profile[..128].copy_from_slice(&base[..128]);
    let length = u32::try_from(profile.len())?;
    profile[..4].copy_from_slice(&length.to_be_bytes());
    profile[36..40].copy_from_slice(b"bad!");
    profile[128..132].copy_from_slice(&1u32.to_be_bytes());
    profile[132..136].copy_from_slice(b"desc");
    profile[136..140].copy_from_slice(&144u32.to_be_bytes());
    profile[140..144].copy_from_slice(&(tag_size as u32).to_be_bytes());
    let tag = &mut profile[144..];
    tag[..4].copy_from_slice(b"mluc");
    tag[8..12].copy_from_slice(&(records as u32).to_be_bytes());
    tag[12..16].copy_from_slice(&12u32.to_be_bytes());
    for index in 0..records {
        let start = 16 + index * 12;
        tag[start..start + 4].copy_from_slice(b"enUS");
        tag[start + 4..start + 8].copy_from_slice(&(payload as u32).to_be_bytes());
        tag[start + 8..start + 12].copy_from_slice(&(strings as u32).to_be_bytes());
    }
    for pair in tag[strings..].as_chunks_mut::<2>().0.iter_mut() {
        pair.copy_from_slice(&[0, b'A']);
    }
    Ok(profile)
}
