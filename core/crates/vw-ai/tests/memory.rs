// Include the exact production arithmetic without allocating a quarter-gigabyte
// fixture. The library's remaining integration tests exercise actual masks.
mod common;
#[path = "../src/memory.rs"]
mod memory;
use vw_ai::{Error, Result};

#[test]
fn all_retained_mask_tiles_are_charged_before_decode() -> Result<()> {
    let source_bytes = 4096;
    let per_mask = 3_000_000;
    let retained = memory::retained_inputs(source_bytes, 83, std::iter::repeat_n(per_mask, 83))?;
    assert!(retained >= 249_000_000 + source_bytes);
    let peak =
        memory::preparation_peak(3_000_000, 3_000_000 + 64 * 64, retained, 64 * 1024 * 1024)?;
    assert!(peak > 300 * 1024 * 1024);
    // This was the previous admission value, missing 83 caller-owned masks.
    let old = memory::preparation_peak(
        3_000_000,
        3_000_000 + 64 * 64,
        source_bytes,
        64 * 1024 * 1024,
    )?;
    assert!(old < 300 * 1024 * 1024);
    Ok(())
}

#[test]
fn sparse_tiles_and_shared_clones_are_conservatively_accounted() -> Result<()> {
    assert_eq!(
        memory::retained_inputs(200, 2, [4, 6])?,
        200 + 256 + 4 + 6 + 256
    );
    let one = memory::retained_inputs(0, 1, [65_536])?;
    let clones = memory::retained_inputs(0, 3, [65_536; 3])?;
    assert_eq!(clones, one * 3);
    assert!(memory::retained_inputs(u64::MAX, 1, []).is_err());
    assert!(memory::retained_inputs(0, 1, [u64::MAX]).is_err());
    assert!(memory::preparation_peak(u64::MAX, 1, 0, 0).is_err());
    assert!(memory::preparation_peak(1, u64::MAX, 0, 0).is_err());
    assert!(memory::preparation_peak(1, 1, u64::MAX, 0).is_err());
    Ok(())
}

#[test]
fn preparation_must_admit_the_larger_completion_and_response_peak() -> Result<()> {
    let n = 10_000_000;
    let plan = 128 * 128 + 128 * 128;
    let retained = memory::retained_prepared(n, 4096, 1024, 1024)?;
    let prepare = memory::preparation_peak(n, plan, 8192, 64 * 1024 * 1024)?;
    let completion = memory::completion_peak(n, plan, retained, vw_ai::MAX_ENCODED as u64)?;
    let default_budget = vw_ai::Limits::default().memory_bytes;
    assert!(prepare < default_budget);
    assert!(completion > default_budget);
    assert_eq!(
        memory::completion_peak(n, plan, retained, 0)? + vw_ai::MAX_ENCODED as u64,
        completion
    );
    assert_eq!(
        memory::retained_prepared(123, 456, 789, 12)?,
        1230 + 456 + 789 + 12
    );
    for args in [
        (u64::MAX, 0, 0, 0),
        (0, u64::MAX, 1, 0),
        (0, 0, u64::MAX, 1),
        (1, 0, 0, u64::MAX),
    ] {
        assert!(memory::retained_prepared(args.0, args.1, args.2, args.3).is_err());
        assert!(memory::completion_peak(args.0, args.1, args.2, args.3).is_err());
    }
    Ok(())
}

#[test]
fn exact_retained_encodings_bind_the_preparation_boundary_and_finish_succeeds() -> common::TestResult
{
    use vw_ai::{NeverCancel, Prepared};
    let source = common::png8(64, 64, |x, y| [(x * 3) as u8, (y * 3) as u8, 70, 180])?;
    let mask = common::region(64, 64, 24, 24, 40, 40)?;
    let baseline = Prepared::new(
        &source,
        std::slice::from_ref(&mask),
        common::options(1)?,
        &NeverCancel,
    )?;
    let c = baseline.review().crop();
    let plan = u64::from(c.crop.width) * u64::from(c.crop.height)
        + u64::from(c.model_width) * u64::from(c.model_height);
    let retained = memory::retained_prepared(
        64 * 64,
        baseline.request_image_png().len() as u64,
        baseline.request_mask_png().len() as u64,
        baseline.source().icc().len() as u64,
    )?;
    let exact = memory::completion_peak(64 * 64, plan, retained, vw_ai::MAX_ENCODED as u64)?;
    let minimum = memory::completion_peak(
        64 * 64,
        plan,
        memory::retained_prepared(64 * 64, 0, 0, baseline.source().icc().len() as u64)?,
        vw_ai::MAX_ENCODED as u64,
    )?;
    assert!(minimum < exact - 1);
    drop(baseline);
    let mut too_small = common::options(1)?;
    too_small.limits.memory_bytes = exact - 1;
    assert!(matches!(
        Prepared::new(
            &source,
            std::slice::from_ref(&mask),
            too_small,
            &NeverCancel
        ),
        Err(Error::Limit(
            "working memory; use a smaller region or a tiled adapter"
        ))
    ));
    let mut admitted = common::options(1)?;
    admitted.limits.memory_bytes = exact;
    let prepared = Prepared::new(&source, &[mask], admitted, &NeverCancel)?;
    let response = prepared.mock_response([200, 30, 40, 255])?;
    let result = prepared.finish(response, &NeverCancel)?;
    assert_eq!(result.proof().changed_outside, 0);
    Ok(())
}
