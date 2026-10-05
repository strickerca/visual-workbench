//! Selected action only. No complete-tree query, UI mutation, cached authority,
//! default shortcut or inferred RangeValue-to-pixel mapping.
use super::*;
use vw_remote::profile::essential::{ControlRoute, EssentialAction, PaintToolbar, RangeShape};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Proof {
    pub(crate) canvas: ElementFacts,
    pub(crate) canvas_tool: String,
    pub(crate) destination: ElementFacts,
    pub(crate) watched_settings_digest: String,
    pub(crate) brush_selected: bool,
    pub(crate) eraser_selected: bool,
    pub(crate) numeric: Option<NumericControl>,
    pub(crate) brush_style: String,
}
fn exact<T>(value: Observed<T>) -> Result<T> {
    match value {
        Observed::Known { value } => Ok(value),
        Observed::Unknown { error } => Err(error),
    }
}
fn range_shape(value: &RangeFacts) -> RangeShape {
    RangeShape {
        minimum: value.minimum,
        maximum: value.maximum,
        small_change: value.small_change,
        large_change: value.large_change,
    }
}
struct View<'a> {
    query: Query<'a>,
    pane: IUIAutomationElement,
    toolbar: IUIAutomationElement,
    tools: IUIAutomationElement,
    brushes: IUIAutomationElement,
    canvas: IUIAutomationElement,
}
fn view<'a>(target: &'a Target, budget: &'a SourceBudget) -> Result<View<'a>> {
    let deadline = Instant::now()
        .checked_add(budget.remaining()?.min(Duration::from_millis(180)))
        .ok_or(Error::Limit)?;
    // SAFETY: caller owns the retained isolated MTA worker; interfaces remain on
    // this invocation's thread and never escape in the owned Proof.
    let automation: IUIAutomation = budget.call(|| {
        platform::api("Paint action UIA", unsafe {
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        })
    })?;
    let q = Query {
        automation,
        target,
        source_budget: budget,
        deadline,
    };
    let root = q.call("Paint action selected root", || unsafe {
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
    if q.call("Paint action exact HWND", || unsafe {
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
    let tools = q.unique(
        &toolbar,
        &Selector::xaml("NamedContainerAutomationPeer", "", Some("Tools"), 50026),
    )?;
    let brushes = q.unique(
        &toolbar,
        &Selector::xaml("NamedContainerAutomationPeer", "", Some("Brushes"), 50026),
    )?;
    let scroll = q.unique(
        &pane,
        &Selector::xaml("ScrollViewer", "scrollViewer", Some(""), 50033),
    )?;
    let canvas = q.unique(
        &scroll,
        &Selector::xaml("NamedContainerAutomationPeer", "image", None, 50026),
    )?;
    Ok(View {
        query: q,
        pane,
        toolbar,
        tools,
        brushes,
        canvas,
    })
}
impl View<'_> {
    fn brush(&self) -> Result<BrushFacts> {
        let q = &self.query;
        let split = q.unique(
            &self.brushes,
            &Selector::xaml(
                "Microsoft.UI.Xaml.Controls.SplitButton",
                "BrushesSplitButton",
                None,
                50013,
            ),
        )?;
        let primary = q.raw_unique(
            &split,
            &Selector::xaml("Button", "PrimaryButton", None, 50000),
        )?;
        let style = q.value_text(&split)?;
        if style.is_empty() {
            return Err(Error::Unavailable);
        }
        Ok(BrushFacts {
            selected: q.selected(&split)?,
            style,
            primary_button: q.facts(&primary)?,
        })
    }
    fn element(&self, control: PaintToolbar) -> Result<IUIAutomationElement> {
        let q = &self.query;
        match control {
            PaintToolbar::Brush => {
                let split = q.unique(
                    &self.brushes,
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
            PaintToolbar::Eraser => q.unique(
                &self.tools,
                &Selector::xaml("ToggleButton", "EraserTool", Some("Eraser"), 50000),
            ),
            PaintToolbar::Undo => q.unique(
                &self.pane,
                &Selector::xaml("AppBarButton", "", Some("Undo"), 50000),
            ),
            PaintToolbar::Redo => q.unique(
                &self.pane,
                &Selector::xaml("AppBarButton", "", Some("Redo"), 50000),
            ),
            PaintToolbar::ZoomIn => q.unique(
                &self.pane,
                &Selector::xaml("Button", "", Some("Zoom in"), 50000),
            ),
            PaintToolbar::ZoomOut => q.unique(
                &self.pane,
                &Selector::xaml("Button", "", Some("Zoom out"), 50000),
            ),
            PaintToolbar::FitCanvas => q.unique(
                &self.pane,
                &Selector::xaml("Button", "", Some("Fit to window"), 50000),
            ),
        }
    }
}
fn base(v: &View<'_>, expected_watched_digest: &str, require_canvas_focus: bool) -> Result<Proof> {
    if !vw_remote::profile::digest(expected_watched_digest) {
        return Err(Error::Invalid);
    }
    let q = &v.query;
    let brush = v.brush()?;
    let toggles = q.toggles(&v.tools)?;
    let canvas_tool = q.name(&v.canvas)?;
    let tool = recognize(&brush, &toggles, &canvas_tool)?;
    let canvas = q.facts(&v.canvas)?;
    if !canvas.destination_eligible {
        return Err(Error::Unavailable);
    }
    if require_canvas_focus && q.focus(&v.canvas)? != FocusStatus::SelectedCanvas {
        return Err(Error::Ungranted);
    }
    let opacity = q.numeric(
        &v.pane,
        &Selector::xaml("Slider", "", Some("Opacity"), 50015),
    )?;
    let opacity_value = exact(opacity.value)?;
    let colors = q.colors(&v.toolbar)?;
    // Intentional size/zoom/tool changes do not invalidate the watched preset.
    // Current tool remains separately proved from both actual toggles + canvas.
    // No runtime ID or transient focus becomes portable settings identity.
    let watched_settings_digest = watched_digest(&brush.style, &opacity_value.text, &colors)?;
    if watched_settings_digest != expected_watched_digest {
        return Err(Error::Unavailable);
    }
    Ok(Proof {
        destination: canvas.clone(),
        canvas,
        canvas_tool,
        watched_settings_digest,
        brush_selected: matches!(tool, RecognizedTool::Brush),
        eraser_selected: matches!(tool, RecognizedTool::Eraser),
        numeric: None,
        brush_style: brush.style,
    })
}
fn destination(
    v: &View<'_>,
    action: &EssentialAction,
    _canvas: &ElementFacts,
) -> Result<(ElementFacts, Option<NumericControl>)> {
    let q = &v.query;
    let (destination, numeric) = match &action.route {
        ControlRoute::PaintToolbar { control } if action.action == control.action() => {
            (q.facts(&v.element(*control)?)?, None)
        }
        ControlRoute::PaintSize { numeric } if matches!(action.action, 6 | 7) => {
            let slider = q.unique(&v.pane, &Selector::xaml("Slider", "", Some("Size"), 50015))?;
            let current = NumericControl {
                element: q.facts(&slider)?,
                value: Observed::capture(q.value(&slider)),
                range: Observed::capture(q.range(&slider)),
            };
            let range = exact(current.range.clone())?;
            let value = exact(current.value.clone())?;
            if range_shape(&range) != numeric.range
                || range.read_only
                || value.read_only
                || value.text.is_empty()
            {
                return Err(Error::Unavailable);
            }
            let thumb = q.raw_unique(
                &slider,
                &Selector::xaml("Thumb", "VerticalThumb", Some(""), 50027),
            )?;
            (q.facts(&thumb)?, Some(current))
        }
        _ => return Err(Error::Unavailable),
    };
    if !destination.destination_eligible {
        return Err(Error::Unavailable);
    }
    Ok((destination, numeric))
}
pub(crate) fn observe_action(
    target: &Target,
    budget: &SourceBudget,
    action: &EssentialAction,
    expected_watched_digest: &str,
    require_canvas_focus: bool,
) -> Result<Proof> {
    let v = view(target, budget)?;
    let mut proof = base(&v, expected_watched_digest, require_canvas_focus)?;
    let (destination, numeric) = destination(&v, action, &proof.canvas)?;
    proof.destination = destination;
    proof.numeric = numeric;
    budget.check()?;
    drop(v);
    budget.check()?;
    Ok(proof)
}
pub(crate) fn observe_authority(
    target: &Target,
    budget: &SourceBudget,
    actions: &[EssentialAction],
    expected_watched_digest: &str,
    require_canvas_focus: bool,
) -> Result<(Proof, Vec<u32>)> {
    let v = view(target, budget)?;
    let proof = base(&v, expected_watched_digest, require_canvas_focus)?;
    let mut verified = Vec::new();
    for action in actions {
        match destination(&v, action, &proof.canvas) {
            Ok(_) => verified.push(action.action),
            Err(Error::Unavailable) => {}
            Err(error) => return Err(error),
        }
    }
    budget.check()?;
    drop(v);
    budget.check()?;
    Ok((proof, verified))
}
fn watched_digest(style: &str, opacity: &str, colors: &[ColorFacts]) -> Result<String> {
    if style.is_empty() || opacity.is_empty() || colors.is_empty() {
        return Err(Error::Unavailable);
    }
    Ok(sha256(
        &serde_json::to_vec(&("M4-Paint-Watched-Settings-v1", style, opacity, colors))
            .map_err(|_| Error::Invalid)?,
    ))
}
pub(super) fn snapshot_watched_digest(snapshot: &Snapshot) -> Result<String> {
    let brush = exact(snapshot.brush.clone())?;
    let opacity = exact(exact(snapshot.watched_opacity.clone())?.value)?;
    let colors = exact(snapshot.color_names.clone())?;
    exact(snapshot.recognized_tool.clone())?;
    watched_digest(&brush.style, &opacity.text, &colors)
}
impl Proof {
    pub(crate) fn canvas_proof(
        &self,
        target: &Target,
        digest: &str,
    ) -> Result<vw_remote::profile::CanvasProof> {
        Ok(vw_remote::profile::CanvasProof {
            runtime_id_hash: self.canvas.runtime_id_hash.clone(),
            process_id: target.process_id,
            sampled_qpc_100ns: platform::clock_100ns()?,
            class_name: "NamedContainerAutomationPeer".into(),
            control_type: 50026,
            canvas_rect: self.canvas.visible_rect_host.ok_or(Error::Unavailable)?,
            profile_digest: digest.into(),
        })
    }
    pub(crate) fn destination_point(&self) -> Result<(i32, i32)> {
        super::super::center(
            self.destination
                .visible_rect_host
                .ok_or(Error::Unavailable)?,
        )
    }
}
fn same_numeric(expected: Option<&NumericControl>, current: &NumericControl) -> Result<()> {
    // Unknown cannot compare equal to an earlier failure and become authority.
    exact(current.value.clone())?;
    exact(current.range.clone())?;
    if expected == Some(current) {
        Ok(())
    } else {
        Err(Error::TargetChanged)
    }
}
/// Final provider work follows every source/package query. No other provider
/// runs after the focus/hit proof and bounded COM release; common owner guards.
pub(crate) fn validate_final(
    target: &Target,
    budget: &SourceBudget,
    action: &EssentialAction,
    expected: &Proof,
    second: bool,
) -> Result<()> {
    let v = view(target, budget)?;
    let current = base(&v, &expected.watched_settings_digest, false)?;
    let (destination, numeric) = destination(&v, action, &current.canvas)?;
    if destination != expected.destination
        || numeric != expected.numeric
        || current.canvas != expected.canvas
        || current.canvas_tool != expected.canvas_tool
        || current.brush_selected != expected.brush_selected
        || current.eraser_selected != expected.eraser_selected
        || current.brush_style != expected.brush_style
    {
        return Err(Error::TargetChanged);
    }
    let (element, slider) = match &action.route {
        ControlRoute::PaintToolbar { control } => (v.element(*control)?, None),
        ControlRoute::PaintSize { .. } => {
            let slider = v
                .query
                .unique(&v.pane, &Selector::xaml("Slider", "", Some("Size"), 50015))?;
            let numeric = NumericControl {
                element: v.query.facts(&slider)?,
                value: Observed::capture(v.query.value(&slider)),
                range: Observed::capture(v.query.range(&slider)),
            };
            same_numeric(expected.numeric.as_ref(), &numeric)?;
            let thumb = v.query.raw_unique(
                &slider,
                &Selector::xaml("Thumb", "VerticalThumb", Some(""), 50027),
            )?;
            (thumb, Some(slider))
        }
        _ => return Err(Error::Invalid),
    };
    if v.query.facts(&element)? != expected.destination {
        return Err(Error::TargetChanged);
    }
    if second {
        let slider = slider.as_ref().ok_or(Error::Invalid)?;
        // SAFETY: PID checked before focus facts; only exact rooted Slider/Thumb.
        unsafe {
            let focused = v.query.call("Paint final focused element", || {
                v.query.automation.GetFocusedElement()
            })?;
            v.query.pid(&focused)?;
            if !(v
                .query
                .call("Paint final focused slider", || {
                    v.query.automation.CompareElements(slider, &focused)
                })?
                .as_bool()
                || v.query
                    .call("Paint final focused thumb", || {
                        v.query.automation.CompareElements(&element, &focused)
                    })?
                    .as_bool())
                || !v
                    .query
                    .call("Paint final keyboard focus", || {
                        focused.CurrentHasKeyboardFocus()
                    })?
                    .as_bool()
            {
                return Err(Error::Ungranted);
            }
        }
    } else {
        // SAFETY: exact physical point from current rooted eligible element.
        let point = expected.destination_point()?;
        let hit = v
            .query
            .call("Paint final physical destination", || unsafe {
                v.query
                    .automation
                    .ElementFromPoint(windows::Win32::Foundation::POINT {
                        x: point.0,
                        y: point.1,
                    })
            })?;
        v.query.pid(&hit)?;
        if !v
            .query
            .call("Paint final same element", || unsafe {
                v.query.automation.CompareElements(&element, &hit)
            })?
            .as_bool()
        {
            return Err(Error::Ungranted);
        }
    }
    let finished = Instant::now();
    drop(element);
    drop(slider);
    drop(v);
    if finished.elapsed() >= Duration::from_millis(20) {
        return Err(Error::Timeout);
    }
    budget.check()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn numeric() -> NumericControl {
        NumericControl {
            element: ElementFacts {
                runtime_id_hash: "a".repeat(64),
                physical_rect_host: None,
                visible_rect_host: None,
                enabled: true,
                offscreen: false,
                destination_eligible: true,
            },
            value: Observed::Known {
                value: ValueFacts {
                    text: "40".into(),
                    read_only: false,
                },
            },
            range: Observed::Known {
                value: RangeFacts {
                    current: 40.0,
                    minimum: 1.0,
                    maximum: 100.0,
                    small_change: 1.0,
                    large_change: 10.0,
                    read_only: false,
                },
            },
        }
    }
    #[test]
    fn final_paint_numeric_relookup_checks_last_value_range_and_element() {
        let expected = numeric();
        assert_eq!(same_numeric(Some(&expected), &expected), Ok(()));
        let mut changed = expected.clone();
        if let Observed::Known { value } = &mut changed.range {
            value.current = 41.0;
        }
        assert_eq!(
            same_numeric(Some(&expected), &changed),
            Err(Error::TargetChanged)
        );
        changed = expected.clone();
        if let Observed::Known { value } = &mut changed.range {
            value.maximum = 200.0;
        }
        assert_eq!(
            same_numeric(Some(&expected), &changed),
            Err(Error::TargetChanged)
        );
        changed = expected.clone();
        changed.element.runtime_id_hash = "f".repeat(64);
        assert_eq!(
            same_numeric(Some(&expected), &changed),
            Err(Error::TargetChanged)
        );
    }
    #[test]
    fn equal_unknown_paint_numeric_proofs_never_authorize_keys() {
        let mut value = numeric();
        value.value = Observed::Unknown {
            error: Error::Unavailable,
        };
        assert_eq!(same_numeric(Some(&value), &value), Err(Error::Unavailable));
        value = numeric();
        value.range = Observed::Unknown {
            error: Error::Timeout,
        };
        assert_eq!(same_numeric(Some(&value), &value), Err(Error::Timeout));
        assert_eq!(same_numeric(None, &numeric()), Err(Error::TargetChanged));
    }
}
