//! Selected action only. No complete-tree query, UI mutation, cached authority,
//! default shortcut or inferred RangeValue-to-pixel mapping.
use super::super::essential_runtime::action::{ActionSpec, RouteSpec};
use super::*;
use vw_remote::profile::essential::{EssentialAction, PaintToolbar, RangeShape};

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
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
        .checked_add(budget.remaining()?.min(Duration::from_millis(
            super::super::essential_runtime::PAINT_PRODUCTION_BUDGET_MS,
        )))
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
fn base(
    v: &View<'_>,
    expected_watched_digest: Option<&str>,
    require_canvas_focus: bool,
) -> Result<Proof> {
    if expected_watched_digest.is_some_and(|expected| !vw_remote::profile::digest(expected)) {
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
    if expected_watched_digest.is_some_and(|expected| watched_settings_digest != expected) {
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
fn validate_size_numeric(current: &NumericControl, expected_range: &RangeShape) -> Result<()> {
    let range = exact(current.range.clone())?;
    let value = exact(current.value.clone())?;
    // Paint exposes display-only Value text on its keyboard-adjustable Size
    // slider. Writability belongs to RangeValue; this route never sets Value.
    // Keep the current text for exact revalidation after the thumb click.
    if range_shape(&range) != *expected_range || range.read_only || value.text.is_empty() {
        return Err(Error::Unavailable);
    }
    Ok(())
}
struct Destination {
    element: IUIAutomationElement,
    slider: Option<IUIAutomationElement>,
    facts: ElementFacts,
    numeric: Option<NumericControl>,
}
fn destination(v: &View<'_>, action: &ActionSpec, _canvas: &ElementFacts) -> Result<Destination> {
    let q = &v.query;
    let (element, slider, numeric) = match &action.route {
        RouteSpec::PaintToolbar { control } if action.action == control.action() => {
            (v.element(*control)?, None, None)
        }
        RouteSpec::PaintSize { numeric } if matches!(action.action, 6 | 7) => {
            let slider = q.unique(&v.pane, &Selector::xaml("Slider", "", Some("Size"), 50015))?;
            let current = NumericControl {
                element: q.facts(&slider)?,
                value: Observed::capture(q.value(&slider)),
                range: Observed::capture(q.range(&slider)),
            };
            validate_size_numeric(&current, &numeric.range)?;
            let thumb = q.raw_unique(
                &slider,
                &Selector::xaml("Thumb", "VerticalThumb", Some(""), 50027),
            )?;
            (thumb, Some(slider), Some(current))
        }
        _ => return Err(Error::Unavailable),
    };
    let facts = q.facts(&element)?;
    if !facts.destination_eligible {
        return Err(Error::Unavailable);
    }
    Ok(Destination {
        element,
        slider,
        facts,
        numeric,
    })
}
pub(crate) fn observe_action(
    target: &Target,
    budget: &SourceBudget,
    action: &ActionSpec,
    expected_watched_digest: Option<&str>,
    require_canvas_focus: bool,
) -> Result<Proof> {
    let v = view(target, budget)?;
    let mut proof = base(&v, expected_watched_digest, require_canvas_focus)?;
    let destination = destination(&v, action, &proof.canvas)?;
    proof.destination = destination.facts;
    proof.numeric = destination.numeric;
    drop(destination.element);
    drop(destination.slider);
    budget.check()?;
    drop(v);
    budget.check()?;
    Ok(proof)
}
pub(crate) fn observe_authority(
    target: &Target,
    budget: &SourceBudget,
    actions: &[EssentialAction],
    expected_watched_digest: Option<&str>,
    require_canvas_focus: bool,
) -> Result<(Proof, Vec<u32>)> {
    let v = view(target, budget)?;
    let proof = base(&v, expected_watched_digest, require_canvas_focus)?;
    let mut verified = Vec::new();
    // Both Size actions resolve the same slider/thumb in this one authority
    // observation. Reuse only its fresh facts here, validating each admitted
    // action's own range; no cached element crosses a command or stage.
    let mut size: Option<Destination> = None;
    for action in actions {
        let action = ActionSpec::from(action);
        let result = if let RouteSpec::PaintSize { numeric } = &action.route {
            if !matches!(action.action, 6 | 7) {
                Err(Error::Unavailable)
            } else {
                match size.as_ref() {
                    Some(value) => validate_size_numeric(
                        value.numeric.as_ref().ok_or(Error::Invalid)?,
                        &numeric.range,
                    ),
                    None => match destination(&v, &action, &proof.canvas) {
                        Ok(value) => {
                            size = Some(value);
                            Ok(())
                        }
                        Err(error) => Err(error),
                    },
                }
            }
        } else {
            destination(&v, &action, &proof.canvas).map(|_| ())
        };
        match result {
            Ok(()) => verified.push(action.action),
            Err(Error::Unavailable) => {}
            Err(error) => return Err(error),
        }
    }
    drop(size);
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
    action: &ActionSpec,
    expected: &Proof,
    second: bool,
) -> Result<()> {
    let v = view(target, budget)?;
    let mut current = base(&v, Some(&expected.watched_settings_digest), false)?;
    let destination = destination(&v, action, &current.canvas)?;
    current.destination = destination.facts;
    current.numeric = destination.numeric;
    if expected != &current {
        return Err(Error::TargetChanged);
    }
    // Reuse the exact rooted objects from the fresh complete proof above.
    // Resolving them again supplied no additional freshness: final facts and
    // actual hit/focus are still read below, after all source/settings work.
    let element = destination.element;
    let slider = destination.slider;
    if let Some(numeric) = current.numeric.as_ref() {
        same_numeric(expected.numeric.as_ref(), numeric)?;
    }
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
        let policy = match &action.route {
            RouteSpec::PaintToolbar {
                control: PaintToolbar::Brush,
            } => DestinationHit::PassiveImage(PassiveImageButton::Brush),
            RouteSpec::PaintToolbar {
                control: PaintToolbar::Eraser,
            } => DestinationHit::PassiveImage(PassiveImageButton::Eraser),
            RouteSpec::PaintSize { .. } => DestinationHit::SizeSlider(
                slider.as_ref().ok_or(Error::Invalid)?,
                &expected.numeric.as_ref().ok_or(Error::Invalid)?.element,
            ),
            _ => DestinationHit::Exact,
        };
        v.query.hit_destination_with_policy(
            &element,
            &expected.destination,
            expected.destination_point()?,
            policy,
        )?;
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
    fn discovered_size() -> NumericControl {
        let mut value = numeric();
        value.value = Observed::Known {
            value: ValueFacts {
                text: "3 pixel".into(),
                read_only: true,
            },
        };
        value.range = Observed::Known {
            value: RangeFacts {
                current: 2.0,
                minimum: 0.0,
                maximum: 90.0,
                small_change: 1.0,
                large_change: 10.0,
                read_only: false,
            },
        };
        value
    }
    fn discovered_size_range() -> RangeShape {
        RangeShape {
            minimum: 0.0,
            maximum: 90.0,
            small_change: 1.0,
            large_change: 10.0,
        }
    }
    #[test]
    fn paint_size_accepts_observed_display_only_text_with_writable_range() -> Result<()> {
        let value = discovered_size();
        validate_size_numeric(&value, &discovered_size_range())?;
        same_numeric(Some(&value), &value)
    }
    #[test]
    fn paint_size_still_refuses_readonly_or_changed_range_and_unknown_or_empty_text() {
        let original = discovered_size();
        let expected = discovered_size_range();
        for case in 0..5 {
            let mut changed = original.clone();
            match case {
                0 => {
                    if let Observed::Known { value } = &mut changed.range {
                        value.read_only = true;
                    }
                }
                1 => {
                    if let Observed::Known { value } = &mut changed.range {
                        value.maximum = 91.0;
                    }
                }
                2 => {
                    changed.range = Observed::Unknown {
                        error: Error::Unavailable,
                    }
                }
                3 => {
                    changed.value = Observed::Unknown {
                        error: Error::Unavailable,
                    }
                }
                _ => {
                    if let Observed::Known { value } = &mut changed.value {
                        value.text.clear();
                    }
                }
            }
            assert_eq!(
                validate_size_numeric(&changed, &expected),
                Err(Error::Unavailable)
            );
        }
    }
    #[test]
    fn paint_size_display_only_text_is_still_freshly_bound_after_thumb_click() {
        let original = discovered_size();
        let mut changed = original.clone();
        if let Observed::Known { value } = &mut changed.value {
            value.text = "4 pixel".into();
        }
        assert_eq!(
            same_numeric(Some(&original), &changed),
            Err(Error::TargetChanged)
        );
        let mut changed = original.clone();
        if let Observed::Known { value } = &mut changed.value {
            value.read_only = false;
        }
        assert_eq!(
            same_numeric(Some(&original), &changed),
            Err(Error::TargetChanged)
        );
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
