//! Numeric part of the owned Paint dot/undo/redo/undo experiment. Only the joined
//! native producer constructs these views; no Deserialize/FFI/file admission.
//! Passing pixels alone never establishes their source or child ownership.
use crate::{Error, Result};

pub(super) const MAX_PIXELS: u64 = 1024 * 1024;
const SETTLE_QPC_100NS: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClickAction {
    CanvasDot,
    Undo,
    Redo,
}

/// Project these facts only inside the actual child/Job/IO collection boundary.
/// An ACK, a caller Boolean or a parsed external receipt cannot construct proof.
#[derive(Clone, Debug)]
pub(super) struct JoinedClick {
    pub action: ClickAction,
    pub context_sha256: String,
    pub canvas_runtime_sha256: String,
    pub settings_sha256: String,
    pub process_id: u32,
    pub process_exit_code: u32,
    pub process_and_job_retired: bool,
    pub io_joined: bool,
    pub native_accepted_count: u32,
    pub expected_count: u32,
    pub input_retirement_complete: bool,
    pub held_count: u32,
    pub had_error: bool,
    pub accepted_qpc_100ns: u64,
    pub receipt_sha256: String,
}
impl JoinedClick {
    fn complete(&self, expected: ClickAction) -> Result<()> {
        if !self.process_and_job_retired
            || !self.io_joined
            || !self.input_retirement_complete
            || self.held_count != 0
        {
            return Err(Error::RetirementPending);
        }
        // Fixed parked click: move/down/up/return-to-original outside point.
        if self.action != expected
            || self.process_id == 0
            || self.process_exit_code != 0
            || self.had_error
            || self.expected_count != 4
            || self.native_accepted_count != 4
            || self.accepted_qpc_100ns == 0
            || !digest(&self.receipt_sha256)
        {
            return Err(Error::PartialInput);
        }
        Ok(())
    }
}

/// RGBA8/explicit-sRGB canvas-only pixels borrowed from the retained verified
/// capture owner after actual process/Job retirement and IO join. Producer must
/// account allocation capacities and encoded/decode/child scratch separately.
#[derive(Clone, Copy)]
pub(super) struct Canvas<'a> {
    pub context_sha256: &'a str,
    pub canvas_runtime_sha256: &'a str,
    pub settings_sha256: &'a str,
    pub tool_id: &'a str,
    pub width: u32,
    pub height: u32,
    pub captured_qpc_100ns: u64,
    pub os_cursor_capture_disabled: bool,
    pub pointer_before: [i64; 2],
    pub pointer_after: [i64; 2],
    pub rgba: &'a [u8],
}
impl Canvas<'_> {
    fn validate(&self) -> Result<()> {
        let pixels = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .ok_or(Error::Limit)?;
        if self.width == 0
            || self.height == 0
            || pixels > MAX_PIXELS
            || u64::try_from(self.rgba.len()).map_err(|_| Error::Limit)? != pixels * 4
            || self.captured_qpc_100ns == 0
            || self.tool_id != "paint.brush"
            || !digest(self.context_sha256)
            || !digest(self.canvas_runtime_sha256)
            || !digest(self.settings_sha256)
        {
            return Err(Error::Invalid);
        }
        if self.rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 255) {
            return Err(Error::Unavailable);
        }
        if !self.os_cursor_capture_disabled
            || self.pointer_before != self.pointer_after
            || (self.pointer_before[0] >= 0
                && self.pointer_before[1] >= 0
                && self.pointer_before[0] < i64::from(self.width)
                && self.pointer_before[1] < i64::from(self.height))
        {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
    fn same_source(&self, other: &Self) -> bool {
        self.context_sha256 == other.context_sha256
            && self.canvas_runtime_sha256 == other.canvas_runtime_sha256
            && self.settings_sha256 == other.settings_sha256
            && self.tool_id == other.tool_id
            && self.width == other.width
            && self.height == other.height
    }
}

pub(super) struct DrawingCheck<'a> {
    pub before: [Canvas<'a>; 2],
    pub after: [Canvas<'a>; 2],
    pub restored: [Canvas<'a>; 2],
    pub redone: [Canvas<'a>; 2],
    pub final_restored: [Canvas<'a>; 2],
    pub dot: &'a JoinedClick,
    pub undo: &'a JoinedClick,
    pub redo: JoinedClick,
    pub final_undo: JoinedClick,
    /// Actual admitted CanvasDot destination, translated from host physical
    /// pixels through the checked canvas rectangle; never caller coordinates.
    pub dot_in_canvas: [u32; 2],
    /// Conservative support bound for this experiment; no size/zoom inference.
    pub radius_pixels: u32,
}

/// This proves only Mouse / EditorObserved / Drawing. Undo/redo verify canvas
/// pixels, not restoration of the prior undo/redo history or physical gestures.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct DrawingEvidence {
    pub changed_pixels: u64,
    pub changed_bounds: [u32; 4],
    pub evidence_sha256: String,
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
fn sha(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}

pub(super) fn evaluate(check: DrawingCheck<'_>) -> Result<DrawingEvidence> {
    check.dot.complete(ClickAction::CanvasDot)?;
    check.undo.complete(ClickAction::Undo)?;
    check.redo.complete(ClickAction::Redo)?;
    check.final_undo.complete(ClickAction::Undo)?;
    let frames = [
        check.before[0],
        check.before[1],
        check.after[0],
        check.after[1],
        check.restored[0],
        check.restored[1],
        check.redone[0],
        check.redone[1],
        check.final_restored[0],
        check.final_restored[1],
    ];
    let original = frames[0];
    for input in [check.dot, check.undo, &check.redo, &check.final_undo] {
        if input.process_id != check.dot.process_id
            || input.context_sha256 != original.context_sha256
            || input.canvas_runtime_sha256 != original.canvas_runtime_sha256
            || input.settings_sha256 != original.settings_sha256
        {
            return Err(Error::TargetChanged);
        }
    }
    let mut previous = 0;
    for frame in &frames {
        frame.validate()?;
        if !original.same_source(frame)
            || frame.captured_qpc_100ns <= previous
            || frame.pointer_before != original.pointer_before
        {
            return Err(Error::TargetChanged);
        }
        previous = frame.captured_qpc_100ns;
    }
    if check.before[1].captured_qpc_100ns >= check.dot.accepted_qpc_100ns
        || check.dot.accepted_qpc_100ns >= check.after[0].captured_qpc_100ns
        || check.after[1].captured_qpc_100ns >= check.undo.accepted_qpc_100ns
        || check.undo.accepted_qpc_100ns >= check.restored[0].captured_qpc_100ns
        || check.restored[1].captured_qpc_100ns >= check.redo.accepted_qpc_100ns
        || check.redo.accepted_qpc_100ns >= check.redone[0].captured_qpc_100ns
        || check.redone[1].captured_qpc_100ns >= check.final_undo.accepted_qpc_100ns
        || check.final_undo.accepted_qpc_100ns >= check.final_restored[0].captured_qpc_100ns
    {
        return Err(Error::TargetChanged);
    }
    for (input, pair) in [
        (check.dot, check.after),
        (check.undo, check.restored),
        (&check.redo, check.redone),
        (&check.final_undo, check.final_restored),
    ] {
        if pair[0]
            .captured_qpc_100ns
            .saturating_sub(input.accepted_qpc_100ns)
            < SETTLE_QPC_100NS
            || pair[1]
                .captured_qpc_100ns
                .saturating_sub(pair[0].captured_qpc_100ns)
                < SETTLE_QPC_100NS
        {
            return Err(Error::Unavailable);
        }
    }
    if check.before[0].rgba != check.before[1].rgba
        || check.after[0].rgba != check.after[1].rgba
        || check
            .restored
            .iter()
            .any(|frame| frame.rgba != original.rgba)
        || check
            .final_restored
            .iter()
            .any(|frame| frame.rgba != original.rgba)
        || check
            .redone
            .iter()
            .any(|frame| frame.rgba != check.after[0].rgba)
        || original
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| *pixel != [255, 255, 255, 255])
    {
        return Err(Error::Unavailable);
    }
    let [cx, cy] = check.dot_in_canvas;
    let r = check.radius_pixels;
    if !(1..=64).contains(&r)
        || cx < r
        || cy < r
        || cx.checked_add(r).is_none_or(|x| x >= original.width)
        || cy.checked_add(r).is_none_or(|y| y >= original.height)
    {
        return Err(Error::Invalid);
    }
    let mut changed = 0u64;
    let (mut left, mut top, mut right, mut bottom) = (original.width, original.height, 0, 0);
    let mut hits_center = false;
    for (index, (before, after)) in original
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(check.after[0].rgba.as_chunks::<4>().0.iter())
        .enumerate()
    {
        if before == after {
            continue;
        }
        let index = u32::try_from(index).map_err(|_| Error::Limit)?;
        let (x, y) = (index % original.width, index / original.width);
        let (dx, dy) = (x.abs_diff(cx), y.abs_diff(cy));
        if dx > r || dy > r {
            return Err(Error::Unavailable);
        }
        hits_center |= dx <= 1 && dy <= 1;
        changed += 1;
        left = left.min(x);
        top = top.min(y);
        right = right.max(x);
        bottom = bottom.max(y);
    }
    if changed == 0 || !hits_center {
        return Err(Error::Unavailable);
    }
    let bounds = [left, top, right - left + 1, bottom - top + 1];
    // Hash actual pixels plus all source, input, timing and support predicates.
    // The summary is private evidence, never an input permission or profile.
    let descriptor = serde_json::json!({"schema":1,"kind":"owned-paint-canvas-dot-pixels",
        "contextSha256":original.context_sha256,"canvasRuntimeSha256":original.canvas_runtime_sha256,
        "settingsSha256":original.settings_sha256,"toolId":original.tool_id,
        "width":original.width,"height":original.height,"frameQpc100ns":frames.map(|f|f.captured_qpc_100ns),
        "beforeRgbaSha256":sha(original.rgba),"afterRgbaSha256":sha(check.after[0].rgba),
        "restoredRgbaSha256":sha(check.restored[0].rgba),"dotReceiptSha256":check.dot.receipt_sha256,
        "undoReceiptSha256":check.undo.receipt_sha256,"dotProcessId":check.dot.process_id,
        "undoProcessId":check.undo.process_id,"dotAcceptedQpc100ns":check.dot.accepted_qpc_100ns,
        "undoAcceptedQpc100ns":check.undo.accepted_qpc_100ns,"dotInCanvas":[cx,cy],"radiusPixels":r,
        "redoReceiptSha256":check.redo.receipt_sha256,"finalUndoReceiptSha256":check.final_undo.receipt_sha256,
        "redoAcceptedQpc100ns":check.redo.accepted_qpc_100ns,"finalUndoAcceptedQpc100ns":check.final_undo.accepted_qpc_100ns,
        "redoRgbaSha256":sha(check.redone[0].rgba),"finalRestoredRgbaSha256":sha(check.final_restored[0].rgba),
        "pointerOutsideCanvas":original.pointer_before,"osCursorCaptureDisabled":true,
        "changedPixels":changed,"changedBounds":bounds,"inputApi":"Mouse","method":"EditorObserved",
        "aspect":"Drawing","result":"Supported","canvasPixelsRestored":true,
        "historyRestoredProven":false,"physicalOwnerGestureProven":false});
    let bytes = serde_json::to_vec(&descriptor).map_err(|_| Error::Invalid)?;
    Ok(DrawingEvidence {
        changed_pixels: changed,
        changed_bounds: bounds,
        evidence_sha256: sha(&bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn click(time: u64) -> JoinedClick {
        JoinedClick {
            action: if time == 20 {
                ClickAction::CanvasDot
            } else {
                ClickAction::Undo
            },
            context_sha256: "a".repeat(64),
            canvas_runtime_sha256: "b".repeat(64),
            settings_sha256: "c".repeat(64),
            process_id: 42,
            process_exit_code: 0,
            process_and_job_retired: true,
            io_joined: true,
            native_accepted_count: 4,
            expected_count: 4,
            input_retirement_complete: true,
            held_count: 0,
            had_error: false,
            accepted_qpc_100ns: time * SETTLE_QPC_100NS,
            receipt_sha256: "d".repeat(64),
        }
    }
    fn canvas<'a>(rgba: &'a [u8], time: u64) -> Canvas<'a> {
        Canvas {
            context_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            canvas_runtime_sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            settings_sha256: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            tool_id: "paint.brush",
            width: 8,
            height: 8,
            captured_qpc_100ns: time * SETTLE_QPC_100NS,
            os_cursor_capture_disabled: true,
            pointer_before: [-5, -5],
            pointer_after: [-5, -5],
            rgba,
        }
    }
    fn check<'a>(
        before: &'a [u8],
        after: &'a [u8],
        restored: &'a [u8],
        dot: &'a JoinedClick,
        undo: &'a JoinedClick,
    ) -> DrawingCheck<'a> {
        DrawingCheck {
            before: [canvas(before, 10), canvas(before, 11)],
            after: [canvas(after, 21), canvas(after, 22)],
            restored: [canvas(restored, 31), canvas(restored, 32)],
            redone: [canvas(after, 41), canvas(after, 42)],
            final_restored: [canvas(restored, 51), canvas(restored, 52)],
            dot,
            undo,
            redo: JoinedClick {
                action: ClickAction::Redo,
                ..click(40)
            },
            final_undo: click(50),
            dot_in_canvas: [4, 4],
            radius_pixels: 2,
        }
    }
    fn pixels() -> (Vec<u8>, Vec<u8>) {
        let before = vec![255; 8 * 8 * 4];
        let mut after = before.clone();
        after[(4 * 8 + 4) * 4..(4 * 8 + 4) * 4 + 3].fill(0);
        (before, after)
    }
    #[test]
    fn admits_stable_dot_only_after_joined_input_and_verified_undo() -> Result<()> {
        let (b, a) = pixels();
        let (dot, undo) = (click(20), click(30));
        let result = evaluate(check(&b, &a, &b, &dot, &undo))?;
        assert_eq!(result.changed_pixels, 1);
        assert_eq!(result.changed_bounds, [4, 4, 1, 1]);
        assert!(digest(&result.evidence_sha256));
        Ok(())
    }
    #[test]
    fn missing_process_io_or_input_retirement_cannot_prove_drawing() {
        let (b, a) = pixels();
        let undo = click(30);
        for kind in 0..7 {
            let mut dot = click(20);
            match kind {
                0 => dot.process_and_job_retired = false,
                1 => dot.io_joined = false,
                2 => dot.input_retirement_complete = false,
                3 => dot.held_count = 1,
                4 => dot.native_accepted_count = 2,
                5 => dot.had_error = true,
                _ => dot.process_exit_code = 1,
            };
            assert!(evaluate(check(&b, &a, &b, &dot, &undo)).is_err());
        }
    }
    #[test]
    fn unchanged_or_not_restored_pixels_stay_unproved() {
        let (b, a) = pixels();
        let (dot, undo) = (click(20), click(30));
        assert!(evaluate(check(&b, &b, &b, &dot, &undo)).is_err());
        assert!(evaluate(check(&b, &a, &a, &dot, &undo)).is_err());
    }
    #[test]
    fn off_dot_or_unstable_changes_cannot_prove_a_canvas_dot() {
        let (b, mut a) = pixels();
        let (dot, undo) = (click(20), click(30));
        a[..3].fill(0);
        assert!(evaluate(check(&b, &a, &b, &dot, &undo)).is_err());
        let (_, a) = pixels();
        let mut c = check(&b, &a, &b, &dot, &undo);
        c.after[1].rgba = &b;
        assert!(evaluate(c).is_err());
    }
    #[test]
    fn replayed_frames_or_changed_context_canvas_settings_tool_are_refused() {
        let (b, a) = pixels();
        let (dot, undo) = (click(20), click(30));
        for field in 0..5 {
            let mut c = check(&b, &a, &b, &dot, &undo);
            match field {
                0 => c.after[0].captured_qpc_100ns = 19,
                1 => c.after[1].context_sha256 = c.before[0].settings_sha256,
                2 => c.after[1].canvas_runtime_sha256 = c.before[0].settings_sha256,
                3 => c.after[1].settings_sha256 = c.before[0].context_sha256,
                _ => c.after[1].tool_id = "paint.eraser",
            };
            assert!(evaluate(c).is_err());
        }
    }
    #[test]
    fn opaque_blank_canvas_and_checked_size_are_required() {
        let (mut b, a) = pixels();
        let (dot, undo) = (click(20), click(30));
        b[3] = 0;
        assert!(evaluate(check(&b, &a, &b, &dot, &undo)).is_err());
        let (b, a) = pixels();
        let mut c = check(&b, &a, &b, &dot, &undo);
        c.before[0].width = u32::MAX;
        assert!(evaluate(c).is_err());
        let mut b = b;
        b[..3].fill(0);
        assert!(evaluate(check(&b, &a, &b, &dot, &undo)).is_err());
    }
    #[test]
    fn wrong_input_action_or_input_source_cannot_be_relabelled_as_drawing() {
        let (b, a) = pixels();
        let undo = click(30);
        for field in 0..5 {
            let mut dot = click(20);
            match field {
                0 => dot.action = ClickAction::Undo,
                1 => dot.context_sha256 = "b".repeat(64),
                2 => dot.canvas_runtime_sha256 = "c".repeat(64),
                3 => dot.settings_sha256 = "a".repeat(64),
                _ => dot.process_id = 99,
            };
            assert!(evaluate(check(&b, &a, &b, &dot, &undo)).is_err());
        }
    }
    #[test]
    fn cursor_or_transient_preview_is_not_reversible_ink() {
        let (b, a) = pixels();
        let (dot, undo) = (click(20), click(30));
        let mut c = check(&b, &a, &b, &dot, &undo);
        c.redone[0].rgba = &b;
        c.redone[1].rgba = &b;
        assert!(evaluate(c).is_err());
        for field in 0..3 {
            let mut c = check(&b, &a, &b, &dot, &undo);
            match field {
                0 => c.after[0].os_cursor_capture_disabled = false,
                1 => {
                    c.after[0].pointer_before = [4, 4];
                    c.after[0].pointer_after = [4, 4]
                }
                _ => c.after[0].pointer_after = [-4, -5],
            }
            assert!(evaluate(c).is_err());
        }
    }
    #[test]
    fn parked_complete_redo_and_final_undo_and_settled_frames_are_required() {
        let (b, a) = pixels();
        let (dot, undo) = (click(20), click(30));
        for field in 0..5 {
            let mut c = check(&b, &a, &b, &dot, &undo);
            match field {
                0 => c.redo.had_error = true,
                1 => c.final_undo.io_joined = false,
                2 => c.final_restored[1].rgba = &a,
                3 => c.after[0].captured_qpc_100ns = dot.accepted_qpc_100ns + 1,
                _ => c.redo.native_accepted_count = 3,
            }
            assert!(evaluate(c).is_err());
        }
    }
}
