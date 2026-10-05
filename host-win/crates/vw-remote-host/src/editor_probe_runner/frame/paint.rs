//! Explicit installed Paint frame witness. Input-free; no ordinary fallback.
use super::*;
use platform::paint_controls::PaintFrameWitness;

pub(super) fn policy(
    policy: SourcePolicy,
    image: &ImageIdentity,
    has_tool: bool,
    paint: Option<&PaintFrameWitness>,
) -> Result<()> {
    match policy {
        SourcePolicy::Ordinary
            if image.executable_name == "krita.exe" && has_tool && paint.is_none() =>
        {
            Ok(())
        }
        SourcePolicy::InstalledPaint
            if image.executable_name == "mspaint.exe"
                && !has_tool
                && paint.is_some()
                && image.package_full_name.as_deref()
                    == Some(vw_host::editor_paint_package::PAINT_FULL_NAME)
                && image.package_version.as_deref()
                    == Some(vw_host::editor_paint_package::PAINT_VERSION) =>
        {
            paint.ok_or(Error::Invalid)?.validate()
        }
        _ => Err(Error::Invalid),
    }
}
fn stable(before: &PaintFrameWitness, after: &PaintFrameWitness) -> bool {
    before.settings_digest == after.settings_digest
        && before.canvas_runtime_id_hash == after.canvas_runtime_id_hash
        && before.canvas_rect_host == after.canvas_rect_host
        && before.canvas_tool_name == after.canvas_tool_name
}
pub(super) fn admit(
    expected: &PaintFrameWitness,
    actual: &PaintFrameWitness,
    canvas: Rect,
    client: Rect,
) -> Result<()> {
    expected.validate()?;
    actual.validate()?;
    expected
        .observed_end_qpc_100ns
        .checked_mul(100)
        .ok_or(Error::Limit)?;
    actual
        .observed_end_qpc_100ns
        .checked_mul(100)
        .ok_or(Error::Limit)?;
    if !stable(expected, actual)
        || actual.canvas_rect_host != canvas
        || !canvas.inside(client)
        || actual.observed_start_qpc_100ns < expected.observed_end_qpc_100ns
    {
        return Err(Error::TargetChanged);
    }
    Ok(())
}
pub(super) fn unchanged(before: &PaintFrameWitness, after: &PaintFrameWitness) -> Result<()> {
    before.validate()?;
    after.validate()?;
    if !stable(before, after) || after.observed_start_qpc_100ns < before.observed_end_qpc_100ns {
        return Err(Error::TargetChanged);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn proof(start: u64, end: u64) -> PaintFrameWitness {
        PaintFrameWitness {
            settings_digest: "a".repeat(64),
            canvas_runtime_id_hash: "b".repeat(64),
            canvas_rect_host: Rect {
                x: 5,
                y: 6,
                width: 40,
                height: 30,
            },
            canvas_tool_name: "Using Brush tool on Canvas".into(),
            observed_start_qpc_100ns: start,
            observed_end_qpc_100ns: end,
        }
    }
    fn image() -> ImageIdentity {
        ImageIdentity {
            executable_name: "mspaint.exe".into(),
            executable_blake3: "c".repeat(64),
            executable_bytes: 1,
            file_version: None,
            package_full_name: Some(vw_host::editor_paint_package::PAINT_FULL_NAME.into()),
            package_version: Some(vw_host::editor_paint_package::PAINT_VERSION.into()),
        }
    }
    #[test]
    fn installed_paint_is_explicit_and_never_an_ordinary_error_fallback() -> Result<()> {
        let mut image = image();
        let witness = proof(1, 2);
        policy(SourcePolicy::InstalledPaint, &image, false, Some(&witness))?;
        assert_eq!(
            policy(SourcePolicy::Ordinary, &image, false, Some(&witness)),
            Err(Error::Invalid)
        );
        image.package_version = Some("unknown".into());
        assert_eq!(
            policy(SourcePolicy::InstalledPaint, &image, false, Some(&witness)),
            Err(Error::Invalid)
        );
        Ok(())
    }
    #[test]
    fn policies_require_exclusive_native_witness_shapes() {
        let image = image();
        let witness = proof(1, 2);
        assert_eq!(
            policy(SourcePolicy::InstalledPaint, &image, true, Some(&witness)),
            Err(Error::Invalid)
        );
        assert_eq!(
            policy(SourcePolicy::InstalledPaint, &image, false, None),
            Err(Error::Invalid)
        );
        let mut krita = image;
        krita.executable_name = "krita.exe".into();
        krita.package_full_name = None;
        krita.package_version = None;
        assert!(policy(SourcePolicy::Ordinary, &krita, true, None).is_ok());
        assert_eq!(
            policy(SourcePolicy::Ordinary, &krita, true, Some(&witness)),
            Err(Error::Invalid)
        );
    }
    #[test]
    fn actual_visible_canvas_and_settings_bracket_can_keep_new_query_clocks() -> Result<()> {
        let expected = proof(1, 2);
        let before = proof(3, 4);
        let after = proof(5, 6);
        admit(
            &expected,
            &before,
            before.canvas_rect_host,
            Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
        )?;
        unchanged(&before, &after)?;
        assert_ne!(before, after);
        Ok(())
    }
    #[test]
    fn changed_tool_digest_canvas_runtime_or_crop_refuses_a_frame_witness() {
        let before = proof(3, 4);
        for field in 0..4 {
            let mut after = proof(5, 6);
            match field {
                0 => after.settings_digest = "c".repeat(64),
                1 => after.canvas_runtime_id_hash = "d".repeat(64),
                2 => after.canvas_tool_name = "Using Eraser tool on Canvas".into(),
                _ => after.canvas_rect_host.width += 1,
            }
            assert_eq!(unchanged(&before, &after), Err(Error::TargetChanged));
        }
    }
    #[test]
    fn stale_overlapping_or_overflow_clock_and_outside_crop_cannot_authorize_capture() {
        let expected = proof(1, 2);
        let before = proof(3, 4);
        let overlapping = proof(3, 5);
        assert_eq!(unchanged(&before, &overlapping), Err(Error::TargetChanged));
        assert_eq!(
            admit(
                &expected,
                &before,
                before.canvas_rect_host,
                Rect {
                    x: 0,
                    y: 0,
                    width: 10,
                    height: 10
                }
            ),
            Err(Error::TargetChanged)
        );
        let overflow = proof(u64::MAX - 1, u64::MAX);
        assert_eq!(
            admit(
                &overflow,
                &overflow,
                overflow.canvas_rect_host,
                Rect {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100
                }
            ),
            Err(Error::Limit)
        );
    }
}
