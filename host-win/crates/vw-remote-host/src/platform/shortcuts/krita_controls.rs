//! Exact owned blank Krita finite measurements. Never production authority.
pub(crate) mod wheel;
use super::*;
use crate::editor_source::{SourceBudget, SourceLease};
use observer::NumericWitness;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Undo,
    Redo,
    Brush,
    EraserToggle,
    Rectangle,
    CanvasDot,
}
impl Action {
    fn route(self) -> Result<profile::live::Route> {
        match self {
            Self::Undo => profile::live::route_for(1),
            Self::Redo => profile::live::route_for(2),
            Self::Brush | Self::CanvasDot => profile::live::route_for(3),
            Self::EraserToggle => profile::live::route_for(4),
            Self::Rectangle => profile::live::fixture_rectangle_route(),
        }
    }
    pub(crate) fn inverse_history(self) -> Option<Self> {
        match self {
            Self::Undo => Some(Self::Redo),
            Self::Redo | Self::CanvasDot => Some(Self::Undo),
            _ => None,
        }
    }
    pub(crate) fn baseline_tool(settings: &ToolSettings) -> Result<Self> {
        match settings.selected_tool_id.as_str() {
            profile::live::BRUSH_ID if settings.freehand_selected => Ok(Self::Brush),
            profile::live::RECTANGLE_ID if !settings.freehand_selected => Ok(Self::Rectangle),
            _ => Err(Error::Unavailable),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Snapshot {
    pub settings: ToolSettings,
    pub settings_digest: String,
    pub canvas: CanvasProof,
    pub element: ElementProof,
    pub brush_numeric_witnesses: Vec<NumericWitness>,
    pub lookup_micros: u64,
}
fn same_canvas(a: &CanvasProof, b: &CanvasProof) -> bool {
    a.runtime_id_hash == b.runtime_id_hash
        && a.process_id == b.process_id
        && a.class_name == b.class_name
        && a.control_type == b.control_type
        && a.canvas_rect == b.canvas_rect
        && a.profile_digest == b.profile_digest
}
impl Snapshot {
    pub(crate) fn same_state(&self, other: &Self) -> bool {
        self.settings == other.settings
            && self.settings_digest == other.settings_digest
            && self.brush_numeric_witnesses == other.brush_numeric_witnesses
            && same_canvas(&self.canvas, &other.canvas)
    }
    pub(crate) fn same_address(&self, other: &Self) -> bool {
        self.same_state(other) && self.element == other.element
    }
    pub(crate) fn destination(&self) -> Result<(i32, i32)> {
        center(self.element.physical_rect)
    }
    pub(crate) fn compatible_change(&self, after: &Self, action: Action) -> Result<()> {
        if !same_canvas(&self.canvas, &after.canvas)
            || self.settings.preset != after.settings.preset
            || self.settings.blending_mode != after.settings.blending_mode
            || self.settings.preserve_alpha != after.settings.preserve_alpha
            || self.brush_numeric_witnesses != after.brush_numeric_witnesses
            || (!matches!(action, Action::Brush | Action::Rectangle)
                && (self.settings.selected_tool_id != after.settings.selected_tool_id
                    || self.settings.freehand_selected != after.settings.freehand_selected))
            || (action != Action::EraserToggle
                && self.settings.eraser_mode != after.settings.eraser_mode)
        {
            Err(Error::TargetChanged)
        } else {
            Ok(())
        }
    }
}
fn settings_digest(settings: &ToolSettings) -> Result<String> {
    let bytes = serde_json::to_vec(settings).map_err(|_| Error::Invalid)?;
    Ok(super::sha256(&bytes))
}
pub(crate) fn observe(target: &Target, budget: &SourceBudget, action: Action) -> Result<Snapshot> {
    budget.check()?;
    let lookup = Budget::new(budget.remaining()?.min(Duration::from_secs(2)));
    let observer = Observer::open(target, &lookup)?;
    let settings = observer.settings()?;
    let (canvas_element, canvas_rect) = observer.canvas()?;
    let canvas = CanvasProof {
        runtime_id_hash: runtime_hash(&canvas_element)?,
        process_id: target.process_id,
        sampled_qpc_100ns: clock_100ns()?,
        class_name: "KisOpenGLCanvas2".into(),
        control_type: 50026,
        canvas_rect,
        profile_digest: String::new(),
    };
    let element = if action == Action::CanvasDot {
        observer.proof(&canvas_element)?
    } else {
        observer.proof(&observer.element(&action.route()?)?)?
    };
    let brush_numeric_witnesses = observer.brush_numeric_witnesses()?;
    lookup.check()?;
    drop(canvas_element);
    drop(observer);
    budget.check()?;
    Ok(Snapshot {
        settings_digest: settings_digest(&settings)?,
        settings,
        canvas,
        element,
        brush_numeric_witnesses,
        lookup_micros: lookup.elapsed_micros(),
    })
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct BatchReceipt {
    pub action: Action,
    pub kind: &'static str,
    pub destination_runtime_hash: String,
    pub actual_destination: (i32, i32),
    pub observed_state_before: Snapshot,
    pub expected_count: u32,
    pub native_accepted_count: u32,
    pub accepted_qpc_100ns: Option<u64>,
    pub error: Option<Error>,
    pub input_retirement: super::super::finite_input::FiniteRetirement,
    pub editor_effect_proven: bool,
    pub physical_pen_proven: bool,
    pub validation_stage: &'static str,
    pub state_mismatch_mask: u32,
    pub validation_observed_state: Option<Snapshot>,
}
impl BatchReceipt {
    pub(crate) fn completed(&self) -> Result<()> {
        if !matches!(
            self.input_retirement,
            super::super::finite_input::FiniteRetirement::Complete
        ) {
            Err(Error::RetirementPending)
        } else if let Some(error) = &self.error {
            // A zero-accepted guard refusal is not a partial native injection.
            Err(error.clone())
        } else if self.native_accepted_count != self.expected_count {
            Err(Error::PartialInput)
        } else {
            Ok(())
        }
    }
}
// Bounded field classes only; timestamps and lookup durations are observations.
fn state_mismatch(expected: &Snapshot, current: &Snapshot) -> u32 {
    u32::from(expected.settings != current.settings)
        | (u32::from(expected.settings_digest != current.settings_digest) << 1)
        | (u32::from(expected.brush_numeric_witnesses != current.brush_numeric_witnesses) << 2)
        | (u32::from(!same_canvas(&expected.canvas, &current.canvas)) << 3)
        | (u32::from(expected.element != current.element) << 4)
}
pub(crate) fn inject(
    input_owner: &mut super::super::finite_input::FiniteInputOwner<SourceLease>,
    target: &Target,
    owner: u32,
    budget: &SourceBudget,
    action: Action,
    expected: &Snapshot,
) -> Result<BatchReceipt> {
    let destination = expected.destination()?;
    let batch = click(absolute(destination)?);
    let mut validation_stage = "owner_preparation";
    let mut state_mismatch_mask = 0;
    let mut validation_observed_state = None;
    let attempt = input_owner.send(&batch, destination, |source| {
        validation_stage = "source_before_observe";
        source.verify(target, owner, budget)?;
        validation_stage = "observe_current";
        let current = observe(target, budget, action)?;
        validation_stage = "compare_current_state";
        state_mismatch_mask = state_mismatch(expected, &current);
        validation_observed_state = Some(current.clone());
        if !expected.same_address(&current) {
            return Err(Error::TargetChanged);
        }
        validation_stage = "brush_precondition";
        if action == Action::CanvasDot && Action::baseline_tool(&current.settings)? != Action::Brush
        {
            return Err(Error::Unavailable);
        }
        // Hash/process/namespace and every potentially blocking source read
        // precede the final rooted current destination/canvas focus proof.
        validation_stage = "source_after_observe";
        source.verify(target, owner, budget)?;
        validation_stage = "target_before_final";
        unchanged(target, owner)?;
        let lookup = Budget::new(budget.remaining()?.min(Duration::from_secs(2)));
        validation_stage = "final_observer_open";
        let observer = Observer::open(target, &lookup)?;
        let element = if action == Action::CanvasDot {
            validation_stage = "focused_canvas";
            let (canvas, rect) = observer.focused_canvas()?;
            validation_stage = "canvas_rectangle";
            if rect != expected.canvas.canvas_rect {
                return Err(Error::TargetChanged);
            }
            canvas
        } else {
            validation_stage = "final_route";
            observer.element(&action.route()?)?
        };
        validation_stage = "final_element";
        if observer.proof(&element)? != expected.element {
            return Err(Error::TargetChanged);
        }
        validation_stage = "exact_hit";
        observer.hit_exact(&element, destination)?;
        validation_stage = "final_lookup_budget";
        lookup.check()?;
        let finished = Instant::now();
        drop(element);
        drop(observer);
        validation_stage = "provider_release_budget";
        if finished.elapsed() >= Duration::from_millis(20) {
            return Err(Error::Timeout);
        }
        validation_stage = "source_budget";
        budget.check()?;
        validation_stage = "native_owner_guard_and_send";
        Ok(())
        // Shared owner performs final cheap exact destination/foreground guard.
        // No provider follows it, or runs while an accepted down remains owned.
    });
    Ok(BatchReceipt {
        action,
        kind: "rooted_balanced_mouse_click",
        destination_runtime_hash: expected.element.runtime_id_hash.clone(),
        actual_destination: destination,
        observed_state_before: expected.clone(),
        expected_count: attempt.expected_count,
        native_accepted_count: attempt.accepted_count,
        accepted_qpc_100ns: attempt.accepted_qpc_100ns,
        error: attempt.error,
        input_retirement: attempt.retirement,
        editor_effect_proven: false,
        physical_pen_proven: false,
        validation_stage,
        state_mismatch_mask,
        validation_observed_state,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Snapshot {
        let rect = Rect {
            x: 10,
            y: 20,
            width: 100,
            height: 200,
        };
        Snapshot {
            settings: ToolSettings {
                freehand_selected: true,
                selected_tool_id: profile::live::BRUSH_ID.into(),
                eraser_mode: false,
                preset: "fixture".into(),
                blending_mode: "Normal".into(),
                preserve_alpha: false,
            },
            settings_digest: "a".repeat(64),
            canvas: CanvasProof {
                runtime_id_hash: "b".repeat(64),
                process_id: 5,
                sampled_qpc_100ns: 1,
                class_name: "KisOpenGLCanvas2".into(),
                control_type: 50026,
                canvas_rect: rect,
                profile_digest: String::new(),
            },
            element: ElementProof {
                runtime_id_hash: "c".repeat(64),
                physical_rect: rect,
                enabled: true,
                offscreen: false,
            },
            brush_numeric_witnesses: vec![NumericWitness {
                element: ElementProof {
                    runtime_id_hash: "d".repeat(64),
                    physical_rect: rect,
                    enabled: true,
                    offscreen: false,
                },
                current: 40.0,
                minimum: 0.01,
                maximum: 1000.0,
                small_change: 1.0,
                large_change: 10.0,
                read_only: false,
                value: "40.00".into(),
            }],
            lookup_micros: 5,
        }
    }
    #[test]
    fn query_time_is_observation_and_changed_numeric_or_canvas_cannot_be_restored_claim() {
        let before = fixture();
        let mut after = before.clone();
        after.canvas.sampled_qpc_100ns += 1;
        after.lookup_micros += 1;
        assert!(before.same_address(&after));
        after.brush_numeric_witnesses[0].current = 41.0;
        assert!(!before.same_state(&after));
        assert_eq!(
            before.compatible_change(&after, Action::Undo),
            Err(Error::TargetChanged)
        );
        after = before.clone();
        after.canvas.canvas_rect.x += 1;
        assert!(!before.same_state(&after));
    }
    #[test]
    fn toggle_change_is_measured_without_becoming_brush_or_eraser_state_setter() -> Result<()> {
        let before = fixture();
        let mut after = before.clone();
        after.settings.eraser_mode = true;
        before.compatible_change(&after, Action::EraserToggle)?;
        assert_eq!(
            before.compatible_change(&after, Action::Brush),
            Err(Error::TargetChanged)
        );
        after.settings.preset = "foreign".into();
        assert_eq!(
            before.compatible_change(&after, Action::EraserToggle),
            Err(Error::TargetChanged)
        );
        Ok(())
    }
    #[test]
    fn finite_routes_do_not_import_shortcuts_coordinates_or_size_semantics() -> Result<()> {
        let action: Action =
            serde_json::from_str("\"eraser_toggle\"").map_err(|_| Error::Invalid)?;
        assert_eq!(action, Action::EraserToggle);
        for value in ["\"ctrl_z\"", "\"opacity_up\"", "\"size_up\"", "{\"x\":100}"] {
            assert!(serde_json::from_str::<Action>(value).is_err());
        }
        Ok(())
    }
    #[test]
    fn history_inverse_never_guesses_toggle_or_tool_setter() {
        assert_eq!(Action::Undo.inverse_history(), Some(Action::Redo));
        assert_eq!(Action::CanvasDot.inverse_history(), Some(Action::Undo));
        assert_eq!(Action::EraserToggle.inverse_history(), None);
        assert_eq!(Action::Rectangle.inverse_history(), None);
    }
    #[test]
    fn baseline_tool_requires_actual_consistent_tool_and_toggle() {
        let mut settings = ToolSettings {
            freehand_selected: true,
            selected_tool_id: profile::live::BRUSH_ID.into(),
            eraser_mode: false,
            preset: "fixture".into(),
            blending_mode: "Normal".into(),
            preserve_alpha: false,
        };
        assert_eq!(Action::baseline_tool(&settings), Ok(Action::Brush));
        settings.freehand_selected = false;
        assert_eq!(Action::baseline_tool(&settings), Err(Error::Unavailable));
        settings.selected_tool_id = profile::live::RECTANGLE_ID.into();
        assert_eq!(Action::baseline_tool(&settings), Ok(Action::Rectangle));
    }
    #[test]
    fn bounded_mismatch_diagnostics_ignore_clocks_and_identify_actual_fields() {
        let expected = fixture();
        let mut current = expected.clone();
        current.lookup_micros += 100;
        current.canvas.sampled_qpc_100ns += 100;
        assert_eq!(state_mismatch(&expected, &current), 0);
        assert!(expected.same_address(&current));
        current.brush_numeric_witnesses[0].current += 1.0;
        assert_eq!(state_mismatch(&expected, &current), 4);
        current.element.physical_rect.x += 1;
        assert_eq!(state_mismatch(&expected, &current), 20);
        current.canvas.canvas_rect.x += 1;
        assert_eq!(state_mismatch(&expected, &current), 28);
    }
    fn refused_batch() -> BatchReceipt {
        let expected = fixture();
        BatchReceipt {
            action: Action::CanvasDot,
            kind: "rooted_balanced_mouse_click",
            destination_runtime_hash: expected.element.runtime_id_hash.clone(),
            actual_destination: (10, 20),
            observed_state_before: expected,
            expected_count: 3,
            native_accepted_count: 0,
            accepted_qpc_100ns: None,
            error: Some(Error::TargetChanged),
            input_retirement: super::super::super::finite_input::FiniteRetirement::Complete,
            editor_effect_proven: false,
            physical_pen_proven: false,
            validation_stage: "exact_hit",
            state_mismatch_mask: 0,
            validation_observed_state: None,
        }
    }
    #[test]
    fn zero_input_preserves_original_refusal_instead_of_partial_injection() {
        let mut value = refused_batch();
        assert_eq!(value.completed(), Err(Error::TargetChanged));
        value.error = Some(Error::Ungranted);
        assert_eq!(value.completed(), Err(Error::Ungranted));
        value.error = Some(Error::Timeout);
        assert_eq!(value.completed(), Err(Error::Timeout));
    }
    #[test]
    fn pending_still_dominates_and_unexplained_short_count_remains_partial() {
        let mut value = refused_batch();
        value.input_retirement = super::super::super::finite_input::FiniteRetirement::Pending {
            held_count: 1,
            uncertain: false,
            error: Error::TargetChanged,
        };
        assert_eq!(value.completed(), Err(Error::RetirementPending));
        value.input_retirement = super::super::super::finite_input::FiniteRetirement::Complete;
        value.error = None;
        assert_eq!(value.completed(), Err(Error::PartialInput));
        value.native_accepted_count = value.expected_count;
        assert_eq!(value.completed(), Ok(()));
    }
}
