//! Separate owned-fixture canvas crop. Never admitted as a full client Frame.
use crate::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasCaptureRequest {
    pub capture: CaptureRequest,
    pub explicit_owned_blank_fixture: bool,
    pub canvas_rect_host: Rect,
}
impl CanvasCaptureRequest {
    pub fn validate(&self) -> Result<()> {
        if !self.explicit_owned_blank_fixture || !self.capture.explicit_owner_action {
            return Err(Error::Consent);
        }
        self.capture
            .target
            .validate(self.capture.owner_process_id)?;
        self.capture.limits.validate()?;
        // This fixture keeps the parent sampler's original resource/deadline ceiling.
        if self.capture.limits.memory_bytes > 128 * 1024 * 1024
            || self.capture.limits.png_bytes > 32 * 1024 * 1024
            || self.capture.limits.max_pixels > 4 * 1024 * 1024
            || self.capture.limits.capture_ms > 3000
        {
            return Err(Error::Limit);
        }
        self.canvas_rect_host
            .offset_inside(self.capture.target.client)?;
        self.canvas_rect_host
            .offset_inside(self.capture.target.frame)?;
        if self.canvas_rect_host.width > 4096
            || self.canvas_rect_host.height > 4096
            || self.capture.diagnostic_alpha_region.is_some()
        {
            return Err(Error::Invalid);
        }
        // Full captured allocation remains budgeted; ROI cannot admit a larger source.
        self.capture.limits.image(
            self.capture.target.frame.width,
            self.capture.target.frame.height,
        )?;
        self.capture
            .limits
            .image(self.canvas_rect_host.width, self.canvas_rect_host.height)?;
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasFrameReceipt {
    /// Metadata of the original captured window; does not label output full-client.
    pub source_identity: FrameIdentity,
    pub target: WindowTarget,
    pub canvas_rect_host: Rect,
    pub crop_in_frame: AlphaRegion,
    pub source_asset_id: String,
    pub png_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub border_visible: bool,
    pub lossless: bool,
    pub filename: String,
}
/// Select the exact physical crop from a mapped row; outside pixels are not
/// converted, masked or used to claim whole-client opacity.
#[cfg(any(windows, test))]
pub(crate) fn selected_row(row: &[u8], x: u32, width: u32) -> Result<&[u8]> {
    if width == 0 || !row.len().is_multiple_of(4) {
        return Err(Error::Invalid);
    }
    let start = usize::try_from(u64::from(x) * 4).map_err(|_| Error::Limit)?;
    let count = usize::try_from(u64::from(width) * 4).map_err(|_| Error::Limit)?;
    row.get(start..start.checked_add(count).ok_or(Error::Limit)?)
        .ok_or(Error::Invalid)
}
#[cfg(any(windows, test))]
pub(crate) fn canvas_rgba(
    row: &[u8],
    enabled: bool,
    diagnostic: &mut Option<ColorDepthDiagnostic>,
    out: &mut Vec<u8>,
) -> Result<()> {
    if !row.len().is_multiple_of(4) {
        return Err(Error::Invalid);
    }
    for pixel in row.as_chunks::<4>().0 {
        if pixel[3] != 255 {
            if enabled {
                *diagnostic = Some(ColorDepthDiagnostic::NonopaquePixel { alpha: pixel[3] })
            }
            return Err(Error::ColorDepth);
        }
        out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    Ok(())
}
/// Both sides of a potentially blocking native target query use the SAME
/// capture deadline. Cancellation/refusal never manufactures a success receipt.
#[cfg(any(windows, test))]
pub(crate) fn completion_check(
    cancel: &Cancellation,
    within: &mut impl FnMut() -> bool,
) -> Result<()> {
    cancel.check()?;
    if !within() {
        return Err(Error::Timeout);
    }
    Ok(())
}
#[cfg(any(windows, test))]
pub(crate) fn final_target_check(
    cancel: &Cancellation,
    mut within: impl FnMut() -> bool,
    verify: impl FnOnce() -> Result<()>,
) -> Result<()> {
    completion_check(cancel, &mut within)?;
    verify()?;
    completion_check(cancel, &mut within)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> CanvasCaptureRequest {
        CanvasCaptureRequest {
            capture: CaptureRequest {
                explicit_owner_action: true,
                owner_process_id: 99,
                target: WindowTarget {
                    window: 1,
                    process_id: 7,
                    process_created: 1,
                    client: Rect {
                        x: -90,
                        y: -30,
                        width: 80,
                        height: 60,
                    },
                    frame: Rect {
                        x: -100,
                        y: -50,
                        width: 100,
                        height: 90,
                    },
                    dpi: 96,
                    observed_ns: 1000,
                },
                capture_session_id: "019f7c21-5678-7123-8123-0123456789ab".into(),
                output_directory: "C:\\Temp".into(),
                limits: Limits {
                    memory_bytes: 128 * 1024 * 1024,
                    png_bytes: 32 * 1024 * 1024,
                    max_pixels: 4 * 1024 * 1024,
                    ..Limits::default()
                },
                diagnostic_color_stage: true,
                diagnostic_alpha_region: None,
            },
            explicit_owned_blank_fixture: true,
            canvas_rect_host: Rect {
                x: -80,
                y: -20,
                width: 40,
                height: 30,
            },
        }
    }
    #[test]
    fn exact_canvas_maps_inside_original_client_and_frame() -> Result<()> {
        let r = request();
        r.validate()?;
        assert_eq!(
            r.canvas_rect_host.offset_inside(r.capture.target.client)?,
            (10, 10)
        );
        assert_eq!(
            r.canvas_rect_host.offset_inside(r.capture.target.frame)?,
            (20, 30)
        );
        Ok(())
    }
    #[test]
    fn crop_never_substitutes_target_or_expands_source_budget() {
        let mut r = request();
        r.canvas_rect_host.x = -91;
        assert_eq!(r.validate(), Err(Error::Stale));
        let mut r = request();
        r.capture.limits.max_pixels = 100;
        assert_eq!(r.validate(), Err(Error::Limit));
        let mut r = request();
        r.explicit_owned_blank_fixture = false;
        assert_eq!(r.validate(), Err(Error::Consent));
        let mut r = request();
        r.capture.limits.capture_ms = 3001;
        assert_eq!(r.validate(), Err(Error::Limit));
        let mut r = request();
        r.capture.limits.memory_bytes = 128 * 1024 * 1024 + 1;
        assert_eq!(r.validate(), Err(Error::Limit));
    }
    #[test]
    fn canvas_request_is_a_distinct_operation_and_rejects_alpha_mask() -> Result<()> {
        let mut r = request();
        r.capture.diagnostic_alpha_region = Some(AlphaRegion {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        });
        assert_eq!(r.validate(), Err(Error::Invalid));
        r.capture.diagnostic_alpha_region = None;
        let bytes = serde_json::to_vec(&Request::CanvasFixture(r)).map_err(|_| Error::Invalid)?;
        assert!(
            String::from_utf8(bytes.clone())
                .map_err(|_| Error::Invalid)?
                .contains("\"operation\":\"canvas_fixture\"")
        );
        assert!(matches!(
            serde_json::from_slice::<Request>(&bytes).map_err(|_| Error::Invalid)?,
            Request::CanvasFixture(_)
        ));
        Ok(())
    }
    #[test]
    fn outside_nonopaque_pixels_do_not_change_typed_canvas_but_full_row_refuses() -> Result<()> {
        let row = [1, 2, 3, 47, 4, 5, 6, 255, 7, 8, 9, 228];
        let mut out = Vec::new();
        let mut diagnostic = None;
        canvas_rgba(selected_row(&row, 1, 1)?, true, &mut diagnostic, &mut out)?;
        assert_eq!(out, vec![6, 5, 4, 255]);
        assert!(diagnostic.is_none());
        assert_eq!(
            canvas_rgba(&row, true, &mut diagnostic, &mut Vec::new()),
            Err(Error::ColorDepth)
        );
        Ok(())
    }
    #[test]
    fn actual_inside_alpha_refuses_without_conversion_or_alpha_relief() -> Result<()> {
        let row = [1, 2, 3, 228];
        let mut diagnostic = None;
        let mut out = Vec::new();
        assert_eq!(
            canvas_rgba(selected_row(&row, 0, 1)?, true, &mut diagnostic, &mut out),
            Err(Error::ColorDepth)
        );
        assert_eq!(
            diagnostic,
            Some(ColorDepthDiagnostic::NonopaquePixel { alpha: 228 })
        );
        assert!(out.is_empty());
        Ok(())
    }
    #[test]
    fn row_crop_rejects_overflow_outside_and_zero_width() {
        assert!(selected_row(&[0; 8], u32::MAX, u32::MAX).is_err());
        assert!(selected_row(&[0; 8], 1, 2).is_err());
        assert!(selected_row(&[0; 8], 0, 0).is_err());
        assert!(selected_row(&[0; 7], 0, 1).is_err());
    }

    #[test]
    fn query_started_inside_budget_cannot_finish_outside_and_publish_success() -> Result<()> {
        let now = std::cell::Cell::new(100u64);
        let called = std::cell::Cell::new(false);
        let cancel = Cancellation::default();
        let result = final_target_check(
            &cancel,
            || now.get() <= 150,
            || {
                called.set(true);
                now.set(151);
                Ok(())
            },
        );
        assert!(called.get());
        assert_eq!(result, Err(Error::Timeout));
        called.set(false);
        assert_eq!(
            final_target_check(
                &cancel,
                || now.get() <= 150,
                || {
                    called.set(true);
                    Ok(())
                }
            ),
            Err(Error::Timeout)
        );
        assert!(!called.get());
        now.set(100);
        final_target_check(
            &cancel,
            || now.get() <= 150,
            || {
                now.set(149);
                Ok(())
            },
        )?;
        now.set(151);
        assert_eq!(
            completion_check(&cancel, &mut || now.get() <= 150),
            Err(Error::Timeout)
        );
        Ok(())
    }
    #[test]
    fn cancellation_during_final_query_remains_refusal_before_adoption() {
        let cancel = Cancellation::default();
        assert_eq!(
            final_target_check(
                &cancel,
                || true,
                || {
                    cancel.cancel();
                    Ok(())
                }
            ),
            Err(Error::Cancelled)
        );
    }
}
