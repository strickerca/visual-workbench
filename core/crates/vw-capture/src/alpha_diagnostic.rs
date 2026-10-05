//! Private owned-fixture relative alpha census. No RGB or source authority.
#[cfg(any(windows, test))]
use crate::Cancellation;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
const MAX_PIXELS: u64 = 4 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlphaRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl AlphaRegion {
    pub fn validate(self, width: u32, height: u32) -> Result<()> {
        if width == 0
            || height == 0
            || u64::from(width) * u64::from(height) > MAX_PIXELS
            || self.width == 0
            || self.height == 0
            || self.x.checked_add(self.width).is_none_or(|n| n > width)
            || self.y.checked_add(self.height).is_none_or(|n| n > height)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    #[cfg(any(windows, test))]
    fn contains(self, x: u32, y: u32) -> bool {
        x >= self.x && y >= self.y && x - self.x < self.width && y - self.y < self.height
    }
}
/// All positions are relative to the retained client crop; bounds are inclusive.
/// The edge is exactly its outermost row/column, never a guessed corner mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlphaSummary {
    pub client_width: u32,
    pub client_height: u32,
    pub bad_pixel_count: u32,
    pub first_x: u32,
    pub first_y: u32,
    pub min_x: u32,
    pub min_y: u32,
    pub max_x: u32,
    pub max_y: u32,
    pub min_alpha: u8,
    pub max_alpha: u8,
    pub edge_pixel_count: u32,
    pub interior_pixel_count: u32,
    pub min_edge_distance: u32,
    pub max_edge_distance: u32,
    pub canvas_bad_pixel_count: u32,
}
impl AlphaSummary {
    pub fn validate(self, width: u32, height: u32, canvas: AlphaRegion) -> Result<()> {
        canvas.validate(width, height)?;
        let pixels = u64::from(width) * u64::from(height);
        let max_distance = (width.min(height) - 1) / 2;
        if self.client_width != width
            || self.client_height != height
            || self.bad_pixel_count == 0
            || u64::from(self.bad_pixel_count) > pixels
            || self.min_x > self.max_x
            || self.min_y > self.max_y
            || self.max_x >= width
            || self.max_y >= height
            || self.first_x < self.min_x
            || self.first_x > self.max_x
            || self.first_y < self.min_y
            || self.first_y > self.max_y
            || self.min_alpha > self.max_alpha
            || self.max_alpha == 255
            || self.edge_pixel_count.checked_add(self.interior_pixel_count)
                != Some(self.bad_pixel_count)
            || self.canvas_bad_pixel_count > self.bad_pixel_count
            || u64::from(self.canvas_bad_pixel_count)
                > u64::from(canvas.width) * u64::from(canvas.height)
            || self.min_edge_distance > self.max_edge_distance
            || self.max_edge_distance > max_distance
            || (self.edge_pixel_count > 0) != (self.min_edge_distance == 0)
            || (self.interior_pixel_count == 0) != (self.max_edge_distance == 0)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[cfg(any(windows, test))]
pub(crate) fn retain_after_target_check<T>(
    summary: &mut Option<AlphaSummary>,
    check: impl FnOnce() -> Result<T>,
) -> Option<T> {
    summary.as_ref()?;
    match check() {
        Ok(proof) => Some(proof),
        Err(_) => {
            *summary = None;
            None
        }
    }
}
#[cfg(any(windows, test))]
pub(crate) struct AlphaScanner {
    width: u32,
    height: u32,
    canvas: AlphaRegion,
    next_row: u32,
    summary: Option<AlphaSummary>,
}
#[cfg(any(windows, test))]
impl AlphaScanner {
    pub(crate) fn new(width: u32, height: u32, canvas: AlphaRegion) -> Result<Self> {
        canvas.validate(width, height)?;
        Ok(Self {
            width,
            height,
            canvas,
            next_row: 0,
            summary: None,
        })
    }
    pub(crate) fn row(&mut self, y: u32, bgra: &[u8], cancel: &Cancellation) -> Result<()> {
        cancel.check()?;
        if y != self.next_row || y >= self.height || bgra.len() != self.width as usize * 4 {
            return Err(Error::Invalid);
        }
        for (x, pixel) in bgra.as_chunks::<4>().0.iter().enumerate() {
            let alpha = pixel[3];
            if alpha == 255 {
                continue;
            }
            let x = u32::try_from(x).map_err(|_| Error::Limit)?;
            let d = x.min(y).min(self.width - 1 - x).min(self.height - 1 - y);
            let value = self.summary.get_or_insert(AlphaSummary {
                client_width: self.width,
                client_height: self.height,
                bad_pixel_count: 0,
                first_x: x,
                first_y: y,
                min_x: x,
                min_y: y,
                max_x: x,
                max_y: y,
                min_alpha: alpha,
                max_alpha: alpha,
                edge_pixel_count: 0,
                interior_pixel_count: 0,
                min_edge_distance: d,
                max_edge_distance: d,
                canvas_bad_pixel_count: 0,
            });
            value.bad_pixel_count += 1;
            value.min_x = value.min_x.min(x);
            value.min_y = value.min_y.min(y);
            value.max_x = value.max_x.max(x);
            value.max_y = value.max_y.max(y);
            value.min_alpha = value.min_alpha.min(alpha);
            value.max_alpha = value.max_alpha.max(alpha);
            value.min_edge_distance = value.min_edge_distance.min(d);
            value.max_edge_distance = value.max_edge_distance.max(d);
            if d == 0 {
                value.edge_pixel_count += 1;
            } else {
                value.interior_pixel_count += 1;
            }
            if self.canvas.contains(x, y) {
                value.canvas_bad_pixel_count += 1;
            }
        }
        self.next_row += 1;
        Ok(())
    }
    pub(crate) fn finish(self) -> Result<Option<AlphaSummary>> {
        if self.next_row != self.height {
            return Err(Error::Invalid);
        }
        if let Some(value) = self.summary {
            value.validate(self.width, self.height, self.canvas)?;
        }
        Ok(self.summary)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn canvas() -> AlphaRegion {
        AlphaRegion {
            x: 1,
            y: 1,
            width: 3,
            height: 3,
        }
    }
    fn scan(alphas: &[[u8; 5]; 5]) -> Result<Option<AlphaSummary>> {
        let mut s = AlphaScanner::new(5, 5, canvas())?;
        let cancel = Cancellation::default();
        for (y, row) in alphas.iter().enumerate() {
            let mut bytes = Vec::new();
            for alpha in row {
                bytes.extend_from_slice(&[7, 11, 19, *alpha]);
            }
            s.row(y as u32, &bytes, &cancel)?;
        }
        s.finish()
    }
    #[test]
    fn opaque_crop_has_no_alpha_summary() -> Result<()> {
        assert_eq!(scan(&[[255; 5]; 5])?, None);
        Ok(())
    }
    #[test]
    fn first_relative_point_bounds_alpha_and_exact_canvas_intersection() -> Result<()> {
        let mut a = [[255; 5]; 5];
        a[0][4] = 228;
        a[2][2] = 1;
        a[4][0] = 254;
        let s = scan(&a)?.ok_or(Error::Invalid)?;
        assert_eq!((s.first_x, s.first_y), (4, 0));
        assert_eq!((s.min_x, s.min_y, s.max_x, s.max_y), (0, 0, 4, 4));
        assert_eq!((s.bad_pixel_count, s.min_alpha, s.max_alpha), (3, 1, 254));
        assert_eq!(
            (
                s.edge_pixel_count,
                s.interior_pixel_count,
                s.canvas_bad_pixel_count
            ),
            (2, 1, 1)
        );
        assert_eq!((s.min_edge_distance, s.max_edge_distance), (0, 2));
        Ok(())
    }
    #[test]
    fn exact_canvas_excludes_nearby_border_and_half_open_end() -> Result<()> {
        let mut a = [[255; 5]; 5];
        a[1][0] = 0;
        a[1][1] = 2;
        a[3][3] = 3;
        a[3][4] = 4;
        let s = scan(&a)?.ok_or(Error::Invalid)?;
        assert_eq!(s.canvas_bad_pixel_count, 2);
        Ok(())
    }
    #[test]
    fn interior_bad_pixels_never_classified_as_edges() -> Result<()> {
        let mut a = [[255; 5]; 5];
        a[1][2] = 0;
        let s = scan(&a)?.ok_or(Error::Invalid)?;
        assert_eq!(
            (
                s.edge_pixel_count,
                s.interior_pixel_count,
                s.min_edge_distance,
                s.max_edge_distance
            ),
            (0, 1, 1, 1)
        );
        Ok(())
    }
    #[test]
    fn incomplete_or_wrong_stride_census_cannot_publish_summary() -> Result<()> {
        let mut s = AlphaScanner::new(5, 5, canvas())?;
        assert!(matches!(
            s.row(0, &[255; 19], &Cancellation::default()),
            Err(Error::Invalid)
        ));
        assert!(matches!(s.finish(), Err(Error::Invalid)));
        Ok(())
    }
    #[test]
    fn cancelled_row_never_finishes_a_partial_census() -> Result<()> {
        let mut s = AlphaScanner::new(5, 5, canvas())?;
        let c = Cancellation::default();
        c.cancel();
        assert!(matches!(s.row(0, &[255; 20], &c), Err(Error::Cancelled)));
        assert!(matches!(s.finish(), Err(Error::Invalid)));
        Ok(())
    }
    #[test]
    fn oversized_overflow_or_unbound_region_is_refused() {
        for r in [
            AlphaRegion {
                x: u32::MAX,
                y: 0,
                width: 1,
                height: 1,
            },
            AlphaRegion {
                x: 0,
                y: 0,
                width: 0,
                height: 1,
            },
            AlphaRegion {
                x: 4,
                y: 0,
                width: 2,
                height: 1,
            },
        ] {
            assert!(r.validate(5, 5).is_err());
        }
        assert!(canvas().validate(32768, 32768).is_err());
    }
    #[test]
    fn bounded_summary_roundtrip_excludes_rgb_text_and_absolute_facts() -> Result<()> {
        let mut a = [[255; 5]; 5];
        a[0][0] = 228;
        let s = scan(&a)?.ok_or(Error::Invalid)?;
        let bytes = serde_json::to_vec(&s).map_err(|_| Error::Invalid)?;
        assert!(bytes.len() < 1024);
        assert_eq!(
            serde_json::from_slice::<AlphaSummary>(&bytes).map_err(|_| Error::Invalid)?,
            s
        );
        let mut v = serde_json::to_value(s).map_err(|_| Error::Invalid)?;
        v["rgb"] = serde_json::json!("private");
        assert!(serde_json::from_value::<AlphaSummary>(v).is_err());
        Ok(())
    }
    #[test]
    fn forged_count_bounds_alpha_and_dimensions_are_refused() -> Result<()> {
        let mut a = [[255; 5]; 5];
        a[0][0] = 228;
        let s = scan(&a)?.ok_or(Error::Invalid)?;
        for bad in [
            AlphaSummary {
                bad_pixel_count: 26,
                ..s
            },
            AlphaSummary { first_x: 5, ..s },
            AlphaSummary {
                max_alpha: 255,
                ..s
            },
            AlphaSummary {
                canvas_bad_pixel_count: 2,
                ..s
            },
            AlphaSummary {
                client_width: 6,
                ..s
            },
        ] {
            assert!(bad.validate(5, 5, canvas()).is_err());
        }
        Ok(())
    }
    fn target() -> crate::WindowTarget {
        crate::WindowTarget {
            window: 1,
            process_id: 7,
            process_created: 1,
            client: crate::Rect {
                x: 0,
                y: 0,
                width: 5,
                height: 5,
            },
            frame: crate::Rect {
                x: 0,
                y: 0,
                width: 5,
                height: 5,
            },
            dpi: 96,
            observed_ns: 100,
        }
    }
    #[test]
    fn summary_survives_only_actual_post_target_equivalence() -> Result<()> {
        let mut pixels = [[255; 5]; 5];
        pixels[0][0] = 228;
        let value = scan(&pixels)?;
        let expected = target();
        let mut current = expected.clone();
        current.observed_ns += 1;
        let mut summary = value;
        assert_eq!(
            retain_after_target_check(&mut summary, || expected.unchanged(&current, 99)),
            Some(())
        );
        assert_eq!(summary, value);
        current.client.x = 1;
        current.frame.width = 6;
        assert_eq!(
            retain_after_target_check(&mut summary, || expected.unchanged(&current, 99)),
            None
        );
        assert_eq!(summary, None);
        Ok(())
    }
    #[test]
    fn failed_target_query_suppresses_summary_without_replacing_refusal() -> Result<()> {
        let mut pixels = [[255; 5]; 5];
        pixels[0][0] = 228;
        let mut summary = scan(&pixels)?;
        let original = Error::ColorDepth;
        assert_eq!(
            retain_after_target_check(&mut summary, || Err::<(), _>(Error::Platform)),
            None
        );
        assert_eq!(summary, None);
        assert_eq!(original, Error::ColorDepth);
        Ok(())
    }
}
