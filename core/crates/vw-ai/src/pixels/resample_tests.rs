use super::resize;
use crate::{Cancellation, Error, NeverCancel, Result};
use image::{Rgba, RgbaImage};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn symmetric_reduction_keeps_premultiplied_color_and_half_coverage() -> Result<()> {
    for (first, expected) in [
        ([255, 0, 0, 0], [0, 0, 255, 128]),
        ([255, 0, 0, 255], [128, 0, 128, 255]),
    ] {
        let source = RgbaImage::from_fn(2, 1, |x, _| {
            Rgba(if x == 0 { first } else { [0, 0, 255, 255] })
        });
        assert_eq!(
            resize(&source, 1, 1, &NeverCancel)?.get_pixel(0, 0).0,
            expected
        );
    }
    Ok(())
}

#[test]
fn invisible_colors_do_not_affect_resized_pixels_but_identity_preserves_exact_bytes() -> Result<()>
{
    let source = RgbaImage::from_fn(7, 5, |x, y| Rgba([x as u8, y as u8, 211, 0]));
    assert_eq!(
        resize(&source, 7, 5, &NeverCancel)?.as_raw(),
        source.as_raw()
    );
    assert!(
        resize(&source, 17, 11, &NeverCancel)?
            .pixels()
            .all(|p| p.0 == [0; 4])
    );
    Ok(())
}

#[test]
fn constant_low_alpha_and_mixed_axis_extremes_keep_color() -> Result<()> {
    for alpha in [1, 254] {
        let color = [41, 129, 230, alpha];
        for (width, height) in [(16384, 1), (1, 16384)] {
            let source = RgbaImage::from_pixel(width, height, Rgba(color));
            let output = resize(&source, height, width, &NeverCancel)?;
            assert!(output.pixels().all(|p| p.0 == color));
        }
    }
    Ok(())
}

#[test]
fn cancellation_during_filtering_retires_intermediate_before_finishing() {
    struct AfterChecks(AtomicUsize);
    impl Cancellation for AfterChecks {
        fn is_cancelled(&self) -> bool {
            self.0.fetch_add(1, Ordering::Relaxed) >= 4
        }
    }
    let cancellation = AfterChecks(AtomicUsize::new(0));
    let source = RgbaImage::from_pixel(32, 32, Rgba([55, 66, 77, 255]));
    assert!(matches!(
        resize(&source, 64, 64, &cancellation),
        Err(Error::Cancelled)
    ));
    assert_eq!(cancellation.0.load(Ordering::Relaxed), 5);
}
