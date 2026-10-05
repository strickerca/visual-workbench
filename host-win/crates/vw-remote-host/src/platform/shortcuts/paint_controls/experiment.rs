//! Finite blank-fixture measurements only. Never profile or production authority.
use super::*;
use crate::editor_source::SourceLease;
use serde::{Deserialize, Serialize};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Undo,
    Redo,
    Brush,
    Eraser,
    SizeUp,
    SizeDown,
    ZoomUp,
    ZoomDown,
    ZoomIn,
    ZoomOut,
    FitToWindow,
    CanvasDot,
}
impl Action {
    pub(crate) fn slider(self) -> bool {
        matches!(
            self,
            Self::SizeUp | Self::SizeDown | Self::ZoomUp | Self::ZoomDown
        )
    }
    fn size(self) -> bool {
        matches!(self, Self::SizeUp | Self::SizeDown)
    }
    fn zoom(self) -> bool {
        matches!(
            self,
            Self::ZoomUp | Self::ZoomDown | Self::ZoomIn | Self::ZoomOut | Self::FitToWindow
        )
    }
    fn tool(self) -> bool {
        matches!(self, Self::Brush | Self::Eraser)
    }
    pub(crate) fn right(self) -> bool {
        matches!(self, Self::SizeUp | Self::ZoomUp)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct State {
    brush: BrushFacts,
    toggles: Vec<ToggleFacts>,
    size: NumericControl,
    watched_opacity: NumericControl,
    zoom: NumericControl,
    canvas: CanvasFacts,
    colors: Vec<ColorFacts>,
}
fn known<T: Clone>(value: &Observed<T>) -> Result<T> {
    match value {
        Observed::Known { value } => Ok(value.clone()),
        Observed::Unknown { error } => Err(error.clone()),
    }
}
impl State {
    pub(crate) fn from_snapshot(snapshot: &Snapshot) -> Result<Self> {
        let result = Self {
            brush: known(&snapshot.brush)?,
            toggles: known(&snapshot.tool_toggles)?,
            size: known(&snapshot.size)?,
            watched_opacity: known(&snapshot.watched_opacity)?,
            zoom: known(&snapshot.zoom)?,
            canvas: known(&snapshot.canvas)?,
            colors: known(&snapshot.color_names)?,
        };
        known(&result.size.value)?;
        known(&result.size.range)?;
        known(&result.watched_opacity.value)?;
        known(&result.zoom.value)?;
        known(&result.zoom.range)?;
        Ok(result)
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        self == other
    }
    pub(crate) fn compatible_change(&self, after: &Self, action: Action) -> Result<()> {
        if self.brush.style != after.brush.style
            || self.brush.primary_button != after.brush.primary_button
            || self.colors != after.colors
            || self.watched_opacity != after.watched_opacity
            || !numeric_shape_same(&self.size, &after.size)
            || !numeric_shape_same(&self.zoom, &after.zoom)
            || self.canvas.element.runtime_id_hash != after.canvas.element.runtime_id_hash
            || (!action.tool()
                && (self.brush != after.brush
                    || self.toggles != after.toggles
                    || self.canvas.current_tool_name != after.canvas.current_tool_name))
            || (!action.size() && !action.tool() && self.size != after.size)
            || (!action.zoom() && self.zoom != after.zoom)
            || (!action.zoom() && self.canvas.element != after.canvas.element)
        {
            return Err(Error::TargetChanged);
        }
        Ok(())
    }
    pub(crate) fn baseline_tool(&self) -> Result<Action> {
        match recognize(&self.brush, &self.toggles, &self.canvas.current_tool_name)? {
            RecognizedTool::Brush => Ok(Action::Brush),
            RecognizedTool::Eraser => Ok(Action::Eraser),
        }
    }
    fn numeric(&self, action: Action) -> Result<(&ValueFacts, &RangeFacts)> {
        let numeric = if action.size() {
            &self.size
        } else {
            &self.zoom
        };
        match (&numeric.value, &numeric.range) {
            (Observed::Known { value: v }, Observed::Known { value: r }) => Ok((v, r)),
            _ => Err(Error::Unavailable),
        }
    }
    pub(crate) fn numeric_current(&self, action: Action) -> Result<f64> {
        Ok(self.numeric(action)?.1.current)
    }
    pub(crate) fn slider_preflight(&self, action: Action) -> Result<()> {
        let (value, range) = self.numeric(action)?;
        // The unique current Thumb is the focus destination. No track-center
        // jump is permitted, and the actual post-click baseline must be equal
        // before the single candidate key. No range-to-pixel mapping is used.
        thumb_preflight(value, range)
    }
}
fn numeric_shape_same(before: &NumericControl, after: &NumericControl) -> bool {
    match (&before.value, &after.value, &before.range, &after.range) {
        (
            Observed::Known { value: a },
            Observed::Known { value: b },
            Observed::Known { value: c },
            Observed::Known { value: d },
        ) => {
            before.element == after.element
                && a.read_only == b.read_only
                && c.minimum == d.minimum
                && c.maximum == d.maximum
                && c.small_change == d.small_change
                && c.large_change == d.large_change
                && c.read_only == d.read_only
        }
        _ => false,
    }
}
fn thumb_preflight(value: &ValueFacts, range: &RangeFacts) -> Result<()> {
    range_valid(range)?;
    // The actual Thumb center avoids a track jump. Every post-click baseline
    // fact must still compare equal before any single key; advertised span is
    // neither a pixel meaning nor permission for a broad restoration traversal.
    if value.text.is_empty() || range.read_only || range.small_change <= 0.0 {
        Err(Error::Unavailable)
    } else {
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Address {
    pub action: Action,
    pub element: ElementFacts,
    pub destination: (i32, i32),
    pub selected_root_only: bool,
    pub ordinal_authority: bool,
}
impl Address {
    pub(crate) fn runtime_hash(&self) -> &str {
        &self.element.runtime_id_hash
    }
}
pub(crate) fn settings_digest(snapshot: &Snapshot) -> Option<&str> {
    snapshot.current_settings_digest.as_deref()
}
struct View<'a> {
    query: Query<'a>,
    pane: IUIAutomationElement,
    toolbar: IUIAutomationElement,
}
fn view<'a>(target: &'a Target, budget: &'a SourceBudget) -> Result<View<'a>> {
    let start = Instant::now();
    let automation = budget.call(|| {
        // SAFETY: caller retains this thread's initialized MTA and actual Job.
        platform::api("Paint effect UIA", unsafe {
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        })
    })?;
    let q = Query {
        automation,
        target,
        source_budget: budget,
        deadline: start
            .checked_add(QUERY_LIMIT.min(budget.remaining()?))
            .ok_or(Error::Limit)?,
    };
    // SAFETY: exact selected HWND only, no desktop or other window traversal.
    let root = q.call("Paint effect root", || unsafe {
        q.automation
            .ElementFromHandle(HWND(target.window as usize as *mut std::ffi::c_void))
    })?;
    q.matches(
        &root,
        &Selector {
            class: "MSPaintApp",
            id: "",
            name: None,
            framework: "Win32",
            kind: 50032,
        },
    )?;
    // SAFETY: PID already matched before any selected root property.
    if q.call("Paint effect root HWND", || unsafe {
        root.CurrentNativeWindowHandle()
    })? != HWND(target.window as usize as *mut std::ffi::c_void)
    {
        return Err(Error::TargetChanged);
    }
    let bridge = q.unique(
        &root,
        &Selector {
            class: "Microsoft.UI.Content.DesktopChildSiteBridge",
            id: "",
            name: None,
            framework: "Win32",
            kind: 50033,
        },
    )?;
    let site = q.unique(
        &bridge,
        &Selector::xaml("InputSiteWindowClass", "", None, 50033),
    )?;
    let pane = q.unique(&site, &Selector::xaml("", "", Some(""), 50025))?;
    let landmark = q.unique(
        &pane,
        &Selector::xaml("LandmarkTarget", "", Some(""), 50026),
    )?;
    let toolbar = q.unique(
        &landmark,
        &Selector::xaml("ScrollViewer", "ScrollViewer", Some(""), 50033),
    )?;
    Ok(View {
        query: q,
        pane,
        toolbar,
    })
}
impl View<'_> {
    fn element(&self, action: Action) -> Result<IUIAutomationElement> {
        let q = &self.query;
        match action {
            Action::Brush => {
                let brushes = q.unique(
                    &self.toolbar,
                    &Selector::xaml("NamedContainerAutomationPeer", "", Some("Brushes"), 50026),
                )?;
                let split = q.unique(
                    &brushes,
                    &Selector::xaml(
                        "Microsoft.UI.Xaml.Controls.SplitButton",
                        "BrushesSplitButton",
                        None,
                        50013,
                    ),
                )?;
                q.raw_unique(
                    &split,
                    &Selector::xaml("Button", "PrimaryButton", None, 50000),
                )
            }
            Action::Eraser => {
                let tools = q.unique(
                    &self.toolbar,
                    &Selector::xaml("NamedContainerAutomationPeer", "", Some("Tools"), 50026),
                )?;
                q.unique(
                    &tools,
                    &Selector::xaml("ToggleButton", "EraserTool", Some("Eraser"), 50000),
                )
            }
            Action::Undo | Action::Redo => q.unique(
                &self.pane,
                &Selector::xaml(
                    "AppBarButton",
                    "",
                    Some(if action == Action::Undo {
                        "Undo"
                    } else {
                        "Redo"
                    }),
                    50000,
                ),
            ),
            Action::SizeUp | Action::SizeDown => q.unique(
                &self.pane,
                &Selector::xaml("Slider", "", Some("Size"), 50015),
            ),
            Action::ZoomUp | Action::ZoomDown => q.unique(
                &self.pane,
                &Selector::xaml("Slider", "ZoomSliderControl", Some("Zoom"), 50015),
            ),
            Action::ZoomIn | Action::ZoomOut | Action::FitToWindow => q.unique(
                &self.pane,
                &Selector::xaml(
                    "Button",
                    "",
                    Some(match action {
                        Action::ZoomIn => "Zoom in",
                        Action::ZoomOut => "Zoom out",
                        _ => "Fit to window",
                    }),
                    50000,
                ),
            ),
            Action::CanvasDot => {
                let scroll = q.unique(
                    &self.pane,
                    &Selector::xaml("ScrollViewer", "scrollViewer", Some(""), 50033),
                )?;
                q.unique(
                    &scroll,
                    &Selector::xaml("NamedContainerAutomationPeer", "image", None, 50026),
                )
            }
        }
    }
    fn thumb(&self, action: Action) -> Result<IUIAutomationElement> {
        if !action.slider() {
            return Err(Error::Invalid);
        }
        self.query.raw_unique(
            &self.element(action)?,
            &Selector::xaml(
                "Thumb",
                if action.size() {
                    "VerticalThumb"
                } else {
                    "HorizontalThumb"
                },
                Some(""),
                50027,
            ),
        )
    }
    fn focused_exact(
        &self,
        element: &IUIAutomationElement,
        thumb: &IUIAutomationElement,
    ) -> Result<()> {
        let q = &self.query;
        // SAFETY: PID-first focus query. No foreign names/values/runtime IDs.
        unsafe {
            let focused = q.call("Paint effect actual focus", || {
                q.automation.GetFocusedElement()
            })?;
            q.pid(&focused)?;
            let slider_focused = q
                .call("Paint focused slider identity", || {
                    q.automation.CompareElements(element, &focused)
                })?
                .as_bool();
            let thumb_focused = q
                .call("Paint focused thumb identity", || {
                    q.automation.CompareElements(thumb, &focused)
                })?
                .as_bool();
            // Only this freshly rooted slider or its unique measured Thumb may
            // own keyboard focus. Other selected-PID controls do not qualify.
            if !(slider_focused || thumb_focused)
                || !q
                    .call("Paint actual focused control", || {
                        focused.CurrentHasKeyboardFocus()
                    })?
                    .as_bool()
            {
                return Err(Error::Ungranted);
            }
        }
        Ok(())
    }
    fn address(&self, action: Action) -> Result<Address> {
        let element = if action.slider() {
            self.thumb(action)?
        } else {
            self.element(action)?
        };
        let facts = self.query.facts(&element)?;
        if !facts.destination_eligible {
            return Err(Error::Unavailable);
        }
        let destination = super::super::center(facts.visible_rect_host.ok_or(Error::Unavailable)?)?;
        Ok(Address {
            action,
            element: facts,
            destination,
            selected_root_only: true,
            ordinal_authority: false,
        })
    }
    fn hit_exact(&self, element: &IUIAutomationElement, destination: (i32, i32)) -> Result<()> {
        // SAFETY: one exact physical point inside the freshly measured selected
        // rooted element. PID is checked before any hit property or comparison.
        unsafe {
            let hit = self.query.call("Paint actual destination element", || {
                self.query
                    .automation
                    .ElementFromPoint(windows::Win32::Foundation::POINT {
                        x: destination.0,
                        y: destination.1,
                    })
            })?;
            self.query.pid(&hit)?;
            if !self
                .query
                .call("Paint destination element identity", || {
                    self.query.automation.CompareElements(element, &hit)
                })?
                .as_bool()
            {
                return Err(Error::Ungranted);
            }
        }
        Ok(())
    }
}
pub(crate) fn address(target: &Target, budget: &SourceBudget, action: Action) -> Result<Address> {
    view(target, budget)?.address(action)
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BatchReceipt {
    pub action: Action,
    pub kind: &'static str,
    pub address: Address,
    pub observed_state_before: State,
    pub focused_control_proven: bool,
    pub attempted: bool,
    pub native_accepted_count: u32,
    pub expected_count: u32,
    pub accepted_qpc_100ns: Option<u64>,
    pub error: Option<Error>,
    pub input_retirement: &'static str,
    pub held_count: u32,
    pub retirement_error: Option<Error>,
    pub editor_effect_proven: bool,
    pub global_release_repair_claim: bool,
}
impl BatchReceipt {
    pub(crate) fn completed(&self) -> Result<()> {
        if self.input_retirement != "complete" {
            Err(Error::RetirementPending)
        } else if self.native_accepted_count != self.expected_count {
            Err(Error::PartialInput)
        } else {
            self.error.clone().map_or(Ok(()), Err)
        }
    }
}
fn keys(action: Action, positive: bool) -> Vec<INPUT> {
    let key = candidate_key(action, positive);
    [KEYBD_EVENT_FLAGS(0), KEYEVENTF_KEYUP]
        .into_iter()
        .map(|flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        })
        .collect()
}
fn candidate_key(action: Action, positive: bool) -> VIRTUAL_KEY {
    VIRTUAL_KEY(if action.size() {
        if positive { 0x26 } else { 0x28 }
    } else if positive {
        0x27
    } else {
        0x25
    })
}
pub(crate) fn inject(
    input_owner: &mut super::super::super::finite_input::FiniteInputOwner<SourceLease>,
    target: &Target,
    owner: u32,
    budget: &SourceBudget,
    step: Step<'_>,
) -> Result<BatchReceipt> {
    let Step {
        action,
        expected,
        expected_address,
        arrow,
    } = step;
    if arrow.is_some() && !action.slider() {
        return Err(Error::Invalid);
    }
    let current = expected_address.clone();
    let normalized = if arrow.is_none() {
        Some(super::super::absolute(current.destination)?)
    } else {
        None
    };
    let batch = if let Some(positive) = arrow {
        keys(action, positive)
    } else {
        super::super::click(normalized.ok_or(Error::Invalid)?)
    };
    // The shared owner refuses any provider closure while a known down remains.
    // It retains the actual SourceLease and owns all accepted down/up accounting.
    let attempt = input_owner.send(&batch, current.destination, |source| {
        source.verify(target, owner, budget)?;
        if action == Action::CanvasDot && expected.baseline_tool()? != Action::Brush {
            return Err(Error::Unavailable);
        }
        let snapshot = observe(target, budget)?;
        if !expected.same(&State::from_snapshot(&snapshot)?) {
            return Err(Error::TargetChanged);
        }
        let v = view(target, budget)?;
        if v.address(action)? != current {
            return Err(Error::TargetChanged);
        }
        if arrow.is_some() {
            let element = v.element(action)?;
            let numeric = NumericControl {
                element: v.query.facts(&element)?,
                value: Observed::capture(v.query.value(&element)),
                range: Observed::capture(v.query.range(&element)),
            };
            if numeric
                != *if action.size() {
                    &expected.size
                } else {
                    &expected.zoom
                }
            {
                return Err(Error::TargetChanged);
            }
        }
        // All blocking source/numeric work precedes final current focus proof.
        source.verify(target, owner, budget)?;
        platform::unchanged(target, owner)?;
        let destination_element = if action.slider() {
            v.thumb(action)?
        } else {
            v.element(action)?
        };
        if arrow.is_some() {
            v.focused_exact(&v.element(action)?, &destination_element)?;
        } else {
            v.hit_exact(&destination_element, current.destination)?;
        }
        let proof_finished = Instant::now();
        drop(destination_element);
        drop(v);
        if proof_finished.elapsed() >= Duration::from_millis(20) {
            return Err(Error::Timeout);
        }
        // The prebuilt absolute INPUT must still map to the guarded point after
        // all provider work and COM teardown. Keyboard-only stages have no map.
        if let Some(expected) = normalized {
            same_absolute(expected, super::super::absolute(current.destination)?)?;
        }
        budget.check()
        // The shared owner now performs the final cheap exact native destination
        // and foreground guard, with no provider call afterward or while held.
    });
    let (input_retirement, held_count, retirement_error) = match &attempt.retirement {
        super::super::super::finite_input::FiniteRetirement::Complete => ("complete", 0, None),
        super::super::super::finite_input::FiniteRetirement::Pending {
            held_count, error, ..
        } => ("pending", *held_count, Some(error.clone())),
    };
    Ok(BatchReceipt {
        action,
        kind: if arrow.is_some() {
            "focused_thumb_arrow_pair"
        } else {
            "rooted_balanced_click"
        },
        address: current,
        observed_state_before: expected.clone(),
        focused_control_proven: arrow.is_some() && attempt.accepted_count > 0,
        attempted: true,
        native_accepted_count: attempt.accepted_count,
        expected_count: attempt.expected_count,
        accepted_qpc_100ns: attempt.accepted_qpc_100ns,
        error: attempt.error,
        input_retirement,
        held_count,
        retirement_error,
        editor_effect_proven: false,
        global_release_repair_claim: false,
    })
}
fn same_absolute(expected: (i32, i32), current: (i32, i32)) -> Result<()> {
    if expected != current {
        Err(Error::TargetChanged)
    } else {
        Ok(())
    }
}
pub(crate) struct Step<'a> {
    pub action: Action,
    pub expected: &'a State,
    pub expected_address: &'a Address,
    pub arrow: Option<bool>,
}
pub(crate) fn progress_toward(previous: f64, next: f64, wanted: f64) -> Result<()> {
    if ![previous, next, wanted].iter().all(|v| v.is_finite())
        || next == previous
        || (wanted - previous).signum() != (next - previous).signum()
        || (wanted - next).abs() >= (wanted - previous).abs()
        || (next != wanted && (wanted - next).signum() != (wanted - previous).signum())
    {
        Err(Error::TargetChanged)
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_absolute_mapping_refuses_topology_drift_for_unchanged_physical_point() {
        let retained = (32_768, 32_784);
        assert_eq!(same_absolute(retained, retained), Ok(()));
        assert_eq!(
            same_absolute(retained, (16_384, 32_784)),
            Err(Error::TargetChanged)
        );
        assert_eq!(
            same_absolute(retained, (32_768, 16_392)),
            Err(Error::TargetChanged)
        );
    }
    #[test]
    fn candidate_keys_are_one_balanced_pair_not_admitted_shortcuts() {
        let inputs = keys(Action::ZoomUp, true);
        assert_eq!(inputs.len(), 2);
        // SAFETY: source-created INPUT_KEYBOARD variants read in this fixture.
        unsafe {
            assert_eq!(inputs[0].Anonymous.ki.wVk, VIRTUAL_KEY(0x27));
            assert_eq!(inputs[0].Anonymous.ki.dwFlags, KEYBD_EVENT_FLAGS(0));
            assert_eq!(inputs[1].Anonymous.ki.dwFlags, KEYEVENTF_KEYUP);
        }
    }
    #[test]
    fn actual_size_thumb_can_address_wide_span_without_a_track_jump_or_pixel_mapping() -> Result<()>
    {
        let value = ValueFacts {
            text: "3 pixel".into(),
            read_only: true,
        };
        let mut range = RangeFacts {
            current: 2.0,
            minimum: 0.0,
            maximum: 90.0,
            small_change: 1.0,
            large_change: 4.0,
            read_only: false,
        };
        thumb_preflight(&value, &range)?;
        range.small_change = 0.0;
        assert_eq!(thumb_preflight(&value, &range), Err(Error::Unavailable));
        range.small_change = 1.0;
        range.current = f64::NAN;
        assert!(thumb_preflight(&value, &range).is_err());
        Ok(())
    }
    #[test]
    fn candidate_numeric_keys_are_finite_and_distinguish_size_from_zoom() {
        assert_eq!(candidate_key(Action::SizeUp, true), VIRTUAL_KEY(0x26));
        assert_eq!(candidate_key(Action::SizeDown, false), VIRTUAL_KEY(0x28));
        assert_eq!(candidate_key(Action::ZoomUp, true), VIRTUAL_KEY(0x27));
        assert_eq!(candidate_key(Action::ZoomDown, false), VIRTUAL_KEY(0x25));
        assert!(!Action::CanvasDot.slider());
    }
    #[test]
    fn restoration_stops_on_no_progress_crossing_wrong_direction_or_nan() -> Result<()> {
        progress_toward(2.0, 3.0, 4.0)?;
        progress_toward(3.0, 4.0, 4.0)?;
        for (a, b, c) in [
            (2.0, 2.0, 4.0),
            (2.0, 1.0, 4.0),
            (2.0, 5.0, 4.0),
            (2.0, f64::NAN, 4.0),
        ] {
            assert_eq!(progress_toward(a, b, c), Err(Error::TargetChanged));
        }
        Ok(())
    }
    #[test]
    fn a_numeric_change_cannot_replace_the_control_or_range_policy() {
        let element = ElementFacts {
            runtime_id_hash: "a".repeat(64),
            physical_rect_host: None,
            visible_rect_host: None,
            enabled: true,
            offscreen: false,
            destination_eligible: false,
        };
        let before = NumericControl {
            element,
            value: Observed::Known {
                value: ValueFacts {
                    text: "3 pixel".into(),
                    read_only: true,
                },
            },
            range: Observed::Known {
                value: RangeFacts {
                    current: 2.0,
                    minimum: 0.0,
                    maximum: 16.0,
                    small_change: 1.0,
                    large_change: 4.0,
                    read_only: false,
                },
            },
        };
        let mut after = before.clone();
        if let Observed::Known { value } = &mut after.range {
            value.current = 3.0;
        }
        if let Observed::Known { value } = &mut after.value {
            value.text = "4 pixel".into();
        }
        assert!(numeric_shape_same(&before, &after));
        after.element.runtime_id_hash = "b".repeat(64);
        assert!(!numeric_shape_same(&before, &after));
        after = before.clone();
        if let Observed::Known { value } = &mut after.range {
            value.maximum = 100.0;
        }
        assert!(!numeric_shape_same(&before, &after));
        after.range = Observed::Unknown {
            error: Error::Unavailable,
        };
        assert!(!numeric_shape_same(&before, &after));
    }
}
