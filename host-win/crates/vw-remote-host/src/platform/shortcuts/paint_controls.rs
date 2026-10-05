//! Query-only measured Paint routes. No action, profile, coordinate or ordinal authority.
pub(crate) mod experiment;
pub(crate) mod production;
use super::{runtime_hash, sha256};
use crate::{Error, Result, editor_source::SourceBudget, platform};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use vw_remote::{Rect, Target};
use windows::{
    Win32::{
        Foundation::HWND,
        System::{Com::*, Variant::VARIANT},
        UI::Accessibility::*,
    },
    core::BSTR,
};

const MAX_MATCHES: i32 = 8;
const QUERY_LIMIT: Duration = Duration::from_secs(3);
const MAX_RAW_PRIMITIVE_CHILDREN: usize = 16;
fn raw_child_budget(visited: usize) -> Result<()> {
    if visited >= MAX_RAW_PRIMITIVE_CHILDREN {
        Err(Error::Limit)
    } else {
        Ok(())
    }
}
fn single_primitive<T>(mut values: Vec<T>) -> Result<T> {
    if values.len() != 1 {
        return Err(Error::Unavailable);
    }
    values.pop().ok_or(Error::Unavailable)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Observed<T> {
    Known { value: T },
    Unknown { error: Error },
}
impl<T> Observed<T> {
    fn capture(value: Result<T>) -> Self {
        match value {
            Ok(value) => Self::Known { value },
            Err(error) => Self::Unknown { error },
        }
    }
    fn is_known(&self) -> bool {
        matches!(self, Self::Known { .. })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ElementFacts {
    runtime_id_hash: String,
    physical_rect_host: Option<Rect>,
    visible_rect_host: Option<Rect>,
    enabled: bool,
    offscreen: bool,
    destination_eligible: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct RangeFacts {
    current: f64,
    minimum: f64,
    maximum: f64,
    small_change: f64,
    large_change: f64,
    read_only: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValueFacts {
    text: String,
    read_only: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NumericControl {
    element: ElementFacts,
    value: Observed<ValueFacts>,
    range: Observed<RangeFacts>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToggleFacts {
    automation_id: String,
    semantic_name: String,
    on: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrushFacts {
    selected: bool,
    style: String,
    primary_button: ElementFacts,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrushRead {
    split_name: Observed<String>,
    selected: Observed<bool>,
    style: Observed<String>,
    primary_name: Observed<String>,
    primary_button: Observed<ElementFacts>,
}
fn brush_facts(read: &BrushRead) -> Result<BrushFacts> {
    // Every semantic component is an actual independent read, not a Name filter
    // or a fallback from a canvas string alone. Unknown stays explicit.
    match (
        &read.split_name,
        &read.selected,
        &read.style,
        &read.primary_name,
        &read.primary_button,
    ) {
        (
            Observed::Known { .. },
            Observed::Known { value: selected },
            Observed::Known { value: style },
            Observed::Known { .. },
            Observed::Known {
                value: primary_button,
            },
        ) if !style.is_empty() => Ok(BrushFacts {
            selected: *selected,
            style: style.clone(),
            primary_button: primary_button.clone(),
        }),
        _ => Err(Error::Unavailable),
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct PaintFrameWitness {
    pub(crate) settings_digest: String,
    pub(crate) canvas_runtime_id_hash: String,
    pub(crate) canvas_rect_host: Rect,
    pub(crate) canvas_tool_name: String,
    pub(crate) observed_start_qpc_100ns: u64,
    pub(crate) observed_end_qpc_100ns: u64,
}
impl PaintFrameWitness {
    pub(crate) fn validate(&self) -> Result<()> {
        for hash in [&self.settings_digest, &self.canvas_runtime_id_hash] {
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
            {
                return Err(Error::Invalid);
            }
        }
        self.canvas_rect_host
            .validate()
            .map_err(|_| Error::Invalid)?;
        if self.observed_start_qpc_100ns == 0
            || self.observed_end_qpc_100ns < self.observed_start_qpc_100ns
            || !matches!(
                self.canvas_tool_name.as_str(),
                "Using Brush tool on Canvas" | "Using Eraser tool on Canvas"
            )
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RecognizedTool {
    Brush,
    Eraser,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ColorFacts {
    name: String,
    selected: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CanvasFacts {
    element: ElementFacts,
    current_tool_name: String,
    // This is a current semantic witness, never a stable selector or RGB proof.
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FocusStatus {
    SelectedCanvas,
    SelectedOtherOrUnproved,
    ForeignOrUnknownPid,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    schema: u32,
    measured_editor_package: &'static str,
    selected_root_only: bool,
    tree_is_atomic: bool,
    max_matches_per_query: i32,
    cooperative_query_limit_ms: u32,
    elapsed_micros: u64,
    observed_start_qpc_100ns: u64,
    observed_end_qpc_100ns: u64,
    complete: bool,
    required_settings_known: bool,
    brush: Observed<BrushFacts>,
    brush_reads: Observed<BrushRead>,
    tool_toggles: Observed<Vec<ToggleFacts>>,
    recognized_tool: Observed<RecognizedTool>,
    size: Observed<NumericControl>,
    size_thumb: Observed<ElementFacts>,
    zoom_thumb: Observed<ElementFacts>,
    // Watched only to disambiguate settings; no opacity action is added.
    watched_opacity: Observed<NumericControl>,
    zoom: Observed<NumericControl>,
    zoom_edit: Observed<NumericControl>,
    canvas: Observed<CanvasFacts>,
    focus: Observed<FocusStatus>,
    undo: Observed<ElementFacts>,
    redo: Observed<ElementFacts>,
    zoom_in: Observed<ElementFacts>,
    zoom_out: Observed<ElementFacts>,
    fit_to_window: Observed<ElementFacts>,
    color_names: Observed<Vec<ColorFacts>>,
    current_settings_digest: Option<String>,
    production_watched_settings_digest: Observed<String>,
    input_sent: bool,
    profile_authority: bool,
    effect_proven: bool,
    frame_witness: Observed<PaintFrameWitness>,
}
impl Snapshot {
    pub(crate) fn frame_witness(&self) -> Result<PaintFrameWitness> {
        if !self.required_settings_known
            || !self.selected_root_only
            || self.input_sent
            || self.profile_authority
            || self.effect_proven
            || self.observed_start_qpc_100ns == 0
            || self.observed_end_qpc_100ns < self.observed_start_qpc_100ns
        {
            return Err(Error::Unavailable);
        }
        let digest = self
            .current_settings_digest
            .as_ref()
            .ok_or(Error::Unavailable)?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        {
            return Err(Error::Invalid);
        }
        let (Observed::Known { value: tool }, Observed::Known { value: canvas }) =
            (&self.recognized_tool, &self.canvas)
        else {
            return Err(Error::Unavailable);
        };
        if !canvas.element.destination_eligible || canvas.element.runtime_id_hash.len() != 64 {
            return Err(Error::Unavailable);
        }
        let expected = match tool {
            RecognizedTool::Brush => "Using Brush tool on Canvas",
            RecognizedTool::Eraser => "Using Eraser tool on Canvas",
        };
        if canvas.current_tool_name != expected {
            return Err(Error::Unavailable);
        }
        let rect = canvas.element.visible_rect_host.ok_or(Error::Unavailable)?;
        rect.validate().map_err(|_| Error::Invalid)?;
        let witness = PaintFrameWitness {
            settings_digest: digest.clone(),
            canvas_runtime_id_hash: canvas.element.runtime_id_hash.clone(),
            canvas_rect_host: rect,
            canvas_tool_name: canvas.current_tool_name.clone(),
            observed_start_qpc_100ns: self.observed_start_qpc_100ns,
            observed_end_qpc_100ns: self.observed_end_qpc_100ns,
        };
        witness.validate()?;
        Ok(witness)
    }
}

fn text(value: BSTR) -> Result<String> {
    bounded_text(&value)
}
fn bounded_text(units: &[u16]) -> Result<String> {
    if units.len() > 256 {
        return Err(Error::Limit);
    }
    let value = String::from_utf16(units).map_err(|_| Error::Invalid)?;
    if value.len() > 256 || value.chars().any(char::is_control) {
        return Err(Error::Limit);
    }
    Ok(value)
}
fn bounded_count(count: i32) -> Result<i32> {
    if (0..=MAX_MATCHES).contains(&count) {
        Ok(count)
    } else {
        Err(Error::Limit)
    }
}
fn intersection(rect: Rect, client: Rect) -> Option<Rect> {
    rect.validate().ok()?;
    client.validate().ok()?;
    let left = rect.x.max(client.x);
    let top = rect.y.max(client.y);
    let right = (i64::from(rect.x) + i64::from(rect.width))
        .min(i64::from(client.x) + i64::from(client.width));
    let bottom = (i64::from(rect.y) + i64::from(rect.height))
        .min(i64::from(client.y) + i64::from(client.height));
    let value = Rect {
        x: left,
        y: top,
        width: u32::try_from(right - i64::from(left)).ok()?,
        height: u32::try_from(bottom - i64::from(top)).ok()?,
    };
    value.validate().ok()?;
    Some(value)
}
fn range_valid(value: &RangeFacts) -> Result<()> {
    if ![
        value.current,
        value.minimum,
        value.maximum,
        value.small_change,
        value.large_change,
    ]
    .iter()
    .all(|v| v.is_finite() && v.abs() <= 1_000_000.0)
        || value.minimum > value.maximum
        || value.current < value.minimum
        || value.current > value.maximum
        || value.small_change < 0.0
        || value.large_change < 0.0
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn numeric_known(value: &Observed<NumericControl>, range_required: bool) -> bool {
    match value {
        Observed::Known { value } => {
            value.value.is_known() && (!range_required || value.range.is_known())
        }
        Observed::Unknown { .. } => false,
    }
}
fn recognize(
    brush: &BrushFacts,
    toggles: &[ToggleFacts],
    canvas_name: &str,
) -> Result<RecognizedTool> {
    let selected: Vec<&str> = toggles
        .iter()
        .filter(|v| v.on)
        .map(|v| v.semantic_name.as_str())
        .collect();
    if selected.len() > 1 || (brush.selected && !selected.is_empty()) {
        return Err(Error::Unavailable);
    }
    if brush.selected && !brush.style.is_empty() && canvas_name == "Using Brush tool on Canvas" {
        Ok(RecognizedTool::Brush)
    } else if selected == ["Eraser"] && canvas_name == "Using Eraser tool on Canvas" {
        Ok(RecognizedTool::Eraser)
    } else {
        Err(Error::Unavailable)
    }
}
fn focus_status(
    pid: i32,
    selected: u32,
    compare: impl FnOnce() -> Result<bool>,
) -> Result<FocusStatus> {
    if pid <= 0 || pid as u32 != selected {
        // No names, values, bounds, runtime IDs or any other foreign facts.
        Ok(FocusStatus::ForeignOrUnknownPid)
    } else if compare()? {
        Ok(FocusStatus::SelectedCanvas)
    } else {
        Ok(FocusStatus::SelectedOtherOrUnproved)
    }
}

struct Selector<'a> {
    class: &'a str,
    id: &'a str,
    name: Option<&'a str>,
    framework: &'a str,
    kind: i32,
}
impl<'a> Selector<'a> {
    fn xaml(class: &'a str, id: &'a str, name: Option<&'a str>, kind: i32) -> Self {
        Self {
            class,
            id,
            name,
            framework: "XAML",
            kind,
        }
    }
}
struct Query<'a> {
    automation: IUIAutomation,
    target: &'a Target,
    source_budget: &'a SourceBudget,
    deadline: Instant,
}
impl Query<'_> {
    fn check(&self) -> Result<()> {
        self.source_budget.check()?;
        if Instant::now() >= self.deadline {
            Err(Error::Timeout)
        } else {
            Ok(())
        }
    }
    fn call<T>(&self, phase: &str, call: impl FnOnce() -> windows::core::Result<T>) -> Result<T> {
        self.check()?;
        let value = self.source_budget.call(|| platform::api(phase, call()));
        self.check()?;
        value
    }
    fn pid(&self, element: &IUIAutomationElement) -> Result<()> {
        // SAFETY: query-only PID first, before names/values/runtime properties.
        let pid = self.call("Paint selected PID", || unsafe {
            element.CurrentProcessId()
        })?;
        if pid > 0 && pid as u32 == self.target.process_id {
            Ok(())
        } else {
            Err(Error::TargetChanged)
        }
    }
    fn matches(&self, element: &IUIAutomationElement, selector: &Selector<'_>) -> Result<()> {
        self.pid(element)?;
        // SAFETY: only an owned same-selected-PID element; bounded copied facts.
        unsafe {
            if text(self.call("Paint framework", || element.CurrentFrameworkId())?)?
                != selector.framework
                || text(self.call("Paint class", || element.CurrentClassName())?)? != selector.class
                || text(self.call("Paint ID", || element.CurrentAutomationId())?)? != selector.id
                || self.call("Paint type", || element.CurrentControlType())?.0 != selector.kind
                || (selector.name.is_some()
                    && Some(
                        text(self.call("Paint semantic name", || element.CurrentName())?)?.as_str(),
                    ) != selector.name)
            {
                return Err(Error::Unavailable);
            }
        }
        Ok(())
    }
    fn condition(&self, selector: &Selector<'_>) -> Result<IUIAutomationCondition> {
        let mut properties = vec![
            (
                UIA_ProcessIdPropertyId,
                VARIANT::from(self.target.process_id as i32),
            ),
            (UIA_FrameworkIdPropertyId, VARIANT::from(selector.framework)),
            (UIA_ClassNamePropertyId, VARIANT::from(selector.class)),
            (UIA_AutomationIdPropertyId, VARIANT::from(selector.id)),
            (UIA_ControlTypePropertyId, VARIANT::from(selector.kind)),
        ];
        if let Some(name) = selector.name {
            properties.push((UIA_NamePropertyId, VARIANT::from(name)));
        }
        // SAFETY: owned initialized VARIANTs copied into owned UIA conditions.
        unsafe {
            let mut combined = self.call("Paint condition", || {
                self.automation
                    .CreatePropertyCondition(properties[0].0, &properties[0].1)
            })?;
            for (property, value) in &properties[1..] {
                let next = self.call("Paint condition", || {
                    self.automation.CreatePropertyCondition(*property, value)
                })?;
                combined = self.call("Paint combined condition", || {
                    self.automation.CreateAndCondition(&combined, &next)
                })?;
            }
            Ok(combined)
        }
    }
    fn find(
        &self,
        parent: &IUIAutomationElement,
        selector: &Selector<'_>,
    ) -> Result<Vec<IUIAutomationElement>> {
        self.pid(parent)?;
        let condition = self.condition(selector)?;
        // SAFETY: only direct children of the selected native root or a uniquely
        // proved child anchor. Provider allocations remain hard-Job-contained.
        unsafe {
            let found = self.call("Paint child lookup", || {
                parent.FindAll(TreeScope_Children, &condition)
            })?;
            let count = bounded_count(self.call("Paint match count", || found.Length())?)?;
            let mut result = Vec::with_capacity(count as usize);
            for at in 0..count {
                let element = self.call("Paint child result", || found.GetElement(at))?;
                self.matches(&element, selector)?;
                result.push(element);
            }
            Ok(result)
        }
    }
    fn unique(
        &self,
        parent: &IUIAutomationElement,
        selector: &Selector<'_>,
    ) -> Result<IUIAutomationElement> {
        let mut found = self.find(parent, selector)?;
        if found.len() != 1 {
            return Err(Error::Unavailable);
        }
        // Pop obtains the sole actual match; never an ordinal among candidates.
        found.pop().ok_or(Error::Unavailable)
    }
    fn facts(&self, element: &IUIAutomationElement) -> Result<ElementFacts> {
        self.pid(element)?;
        // SAFETY: same-PID selected-root member, get-only typed properties.
        unsafe {
            let bounds = self.call("Paint bounds", || element.CurrentBoundingRectangle())?;
            let rect = Rect {
                x: bounds.left,
                y: bounds.top,
                width: u32::try_from(i64::from(bounds.right) - i64::from(bounds.left))
                    .map_err(|_| Error::Invalid)?,
                height: u32::try_from(i64::from(bounds.bottom) - i64::from(bounds.top))
                    .map_err(|_| Error::Invalid)?,
            };
            let physical = rect.validate().ok().map(|()| rect);
            let visible = physical.and_then(|rect| intersection(rect, self.target.client_rect));
            let enabled = self
                .call("Paint enabled", || element.CurrentIsEnabled())?
                .as_bool();
            let offscreen = self
                .call("Paint offscreen", || element.CurrentIsOffscreen())?
                .as_bool();
            self.check()?;
            let runtime_id_hash = self.source_budget.call(|| runtime_hash(element))?;
            self.check()?;
            Ok(ElementFacts {
                runtime_id_hash,
                physical_rect_host: physical,
                visible_rect_host: visible,
                enabled,
                offscreen,
                destination_eligible: enabled && !offscreen && visible.is_some(),
            })
        }
    }
    /// Only direct raw children of an already unique SplitButton or Slider.
    /// Historical source-bound RawViewWalker exposed these primitive IDs;
    /// current FindAll omitted them. No fallback, descendants or ordinal route.
    fn raw_unique(
        &self,
        parent: &IUIAutomationElement,
        selector: &Selector<'_>,
    ) -> Result<IUIAutomationElement> {
        self.pid(parent)?;
        // SAFETY: exact same-selected-PID parent, bounded direct raw siblings.
        let walker = self.call("Paint primitive raw walker", || unsafe {
            self.automation.RawViewWalker()
        })?;
        let next=|value:windows::core::Result<IUIAutomationElement>|->Result<Option<IUIAutomationElement>>{
            match value {
                Ok(value)=>Ok(Some(value)),
                Err(error) if error.code().0==0 => Ok(None),
                Err(error)=>Err(Error::Platform{phase:"Paint primitive raw child".into(),code:error.code().0 as u32}),
            }
        };
        let mut current = self.source_budget.call(|| {
            self.check()?;
            // SAFETY: exact rooted parent; returned owned COM pointer is
            // inspected PID-first before any descriptive property.
            next(unsafe { walker.GetFirstChildElement(parent) })
        })?;
        let mut visited = 0;
        let mut found = Vec::new();
        while let Some(element) = current {
            self.check()?;
            raw_child_budget(visited)?;
            visited += 1;
            // matches() reads PID first; no foreign names, values or runtime IDs.
            match self.matches(&element, selector) {
                Ok(()) => {
                    found.push(element.clone());
                    if found.len() > 1 {
                        return Err(Error::Unavailable);
                    }
                }
                Err(Error::Unavailable) => {}
                Err(error) => return Err(error),
            }
            current = self.source_budget.call(|| {
                self.check()?;
                // SAFETY: current direct child in this bounded selected root;
                // no parent/desktop walk and no foreign property is read.
                next(unsafe { walker.GetNextSiblingElement(&element) })
            })?;
        }
        self.check()?;
        single_primitive(found)
    }
    fn value_text(&self, element: &IUIAutomationElement) -> Result<String> {
        self.pid(element)?;
        // SAFETY: bounded current style read only. Brush style does not require
        // a writable ValuePattern; numeric controls retain independent RO facts.
        unsafe {
            let pattern: IUIAutomationValuePattern = self.call("Paint style pattern", || {
                element.GetCurrentPatternAs(UIA_ValuePatternId)
            })?;
            text(self.call("Paint actual style", || pattern.CurrentValue())?)
        }
    }
    fn name(&self, element: &IUIAutomationElement) -> Result<String> {
        self.pid(element)?;
        // SAFETY: selected-PID rooted element; bounded observed Name only.
        text(self.call("Paint observed name", || unsafe { element.CurrentName() })?)
    }
    fn thumb(
        &self,
        parent: &IUIAutomationElement,
        slider: &Selector<'_>,
        thumb: &str,
    ) -> Result<ElementFacts> {
        let slider = self.unique(parent, slider)?;
        self.facts(&self.raw_unique(&slider, &Selector::xaml("Thumb", thumb, Some(""), 50027))?)
    }
    fn value(&self, element: &IUIAutomationElement) -> Result<ValueFacts> {
        self.pid(element)?;
        // SAFETY: current ValuePattern reads only; never SetValue or focus.
        unsafe {
            let pattern: IUIAutomationValuePattern = self.call("Paint value pattern", || {
                element.GetCurrentPatternAs(UIA_ValuePatternId)
            })?;
            Ok(ValueFacts {
                text: text(self.call("Paint value", || pattern.CurrentValue())?)?,
                read_only: self
                    .call("Paint value readonly", || pattern.CurrentIsReadOnly())?
                    .as_bool(),
            })
        }
    }
    fn range(&self, element: &IUIAutomationElement) -> Result<RangeFacts> {
        self.pid(element)?;
        // SAFETY: RangeValue reads only; no mutation or inferred pixel mapping.
        unsafe {
            let pattern: IUIAutomationRangeValuePattern = self
                .call("Paint range pattern", || {
                    element.GetCurrentPatternAs(UIA_RangeValuePatternId)
                })?;
            let value = RangeFacts {
                current: self.call("Paint current range", || pattern.CurrentValue())?,
                minimum: self.call("Paint minimum", || pattern.CurrentMinimum())?,
                maximum: self.call("Paint maximum", || pattern.CurrentMaximum())?,
                small_change: self.call("Paint small change", || pattern.CurrentSmallChange())?,
                large_change: self.call("Paint large change", || pattern.CurrentLargeChange())?,
                read_only: self
                    .call("Paint range readonly", || pattern.CurrentIsReadOnly())?
                    .as_bool(),
            };
            range_valid(&value)?;
            Ok(value)
        }
    }
    fn numeric(
        &self,
        parent: &IUIAutomationElement,
        selector: &Selector<'_>,
    ) -> Result<NumericControl> {
        let element = self.unique(parent, selector)?;
        Ok(NumericControl {
            element: self.facts(&element)?,
            value: Observed::capture(self.value(&element)),
            range: Observed::capture(self.range(&element)),
        })
    }
    fn selected(&self, element: &IUIAutomationElement) -> Result<bool> {
        self.pid(element)?;
        // SAFETY: SelectionItem read only; never Select/AddToSelection.
        unsafe {
            let pattern: IUIAutomationSelectionItemPattern = self
                .call("Paint selection pattern", || {
                    element.GetCurrentPatternAs(UIA_SelectionItemPatternId)
                })?;
            Ok(self
                .call("Paint actual selection", || pattern.CurrentIsSelected())?
                .as_bool())
        }
    }
    fn toggles(&self, tools: &IUIAutomationElement) -> Result<Vec<ToggleFacts>> {
        // The measured tool group contains distinct IDs, so match type/class only
        // in this one parent, then whitelist each actual ID before retaining it.
        self.pid(tools)?;
        let selector = Selector::xaml("ToggleButton", "", None, 50000);
        let properties = [
            (
                UIA_ProcessIdPropertyId,
                VARIANT::from(self.target.process_id as i32),
            ),
            (UIA_ClassNamePropertyId, VARIANT::from(selector.class)),
            (UIA_FrameworkIdPropertyId, VARIANT::from("XAML")),
            (UIA_ControlTypePropertyId, VARIANT::from(selector.kind)),
        ];
        // SAFETY: direct children of the exact named Tools anchor, bounded and
        // same-PID first. Toggle pattern is queried only; no Toggle call exists.
        unsafe {
            let mut condition = self.call("Paint tools condition", || {
                self.automation
                    .CreatePropertyCondition(properties[0].0, &properties[0].1)
            })?;
            for (property, value) in &properties[1..] {
                let next = self.call("Paint tools condition", || {
                    self.automation.CreatePropertyCondition(*property, value)
                })?;
                condition = self.call("Paint tools combined", || {
                    self.automation.CreateAndCondition(&condition, &next)
                })?;
            }
            let found = self.call("Paint tools lookup", || {
                tools.FindAll(TreeScope_Children, &condition)
            })?;
            let count = bounded_count(self.call("Paint tools count", || found.Length())?)?;
            if count != 6 {
                return Err(Error::Unavailable);
            }
            let mut result = Vec::with_capacity(count as usize);
            for at in 0..count {
                let element = self.call("Paint actual tool", || found.GetElement(at))?;
                self.pid(&element)?;
                let id = text(self.call("Paint tool ID", || element.CurrentAutomationId())?)?;
                let name = text(self.call("Paint tool name", || element.CurrentName())?)?;
                if !matches!(
                    (id.as_str(), name.as_str()),
                    ("PencilTool", "Pencil")
                        | ("", "Fill")
                        | ("", "Text")
                        | ("EraserTool", "Eraser")
                        | ("", "Color picker")
                        | ("", "Magnifier")
                ) || result.iter().any(|v: &ToggleFacts| v.semantic_name == name)
                {
                    return Err(Error::Unavailable);
                }
                let pattern: IUIAutomationTogglePattern = self
                    .call("Paint toggle pattern", || {
                        element.GetCurrentPatternAs(UIA_TogglePatternId)
                    })?;
                let state = self.call("Paint current toggle", || pattern.CurrentToggleState())?;
                let on = if state == ToggleState_On {
                    true
                } else if state == ToggleState_Off {
                    false
                } else {
                    return Err(Error::Unavailable);
                };
                result.push(ToggleFacts {
                    automation_id: id,
                    semantic_name: name,
                    on,
                });
            }
            if !result.iter().any(|v| v.automation_id == "EraserTool") {
                return Err(Error::Unavailable);
            }
            result.sort_by(|a, b| a.semantic_name.cmp(&b.semantic_name));
            Ok(result)
        }
    }
    fn button(&self, pane: &IUIAutomationElement, class: &str, name: &str) -> Result<ElementFacts> {
        self.facts(&self.unique(pane, &Selector::xaml(class, "", Some(name), 50000))?)
    }
    fn colors(&self, toolbar: &IUIAutomationElement) -> Result<Vec<ColorFacts>> {
        let group = self.unique(
            toolbar,
            &Selector::xaml("NamedContainerAutomationPeer", "", Some("Colors"), 50026),
        )?;
        let found = self.find(&group, &Selector::xaml("RadioButton", "", None, 50013))?;
        if found.len() != 2 {
            return Err(Error::Unavailable);
        }
        let mut result = Vec::new();
        for element in found {
            // SAFETY: same-PID selected-root result; bounded current color name.
            let name = text(self.call("Paint color name", || unsafe { element.CurrentName() })?)?;
            if !(name.starts_with("Color 1: ") || name.starts_with("Color 2: "))
                || result.iter().any(|v: &ColorFacts| v.name[..9] == name[..9])
            {
                return Err(Error::Unavailable);
            }
            result.push(ColorFacts {
                name,
                selected: self.selected(&element)?,
            });
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }
    fn focus(&self, canvas: &IUIAutomationElement) -> Result<FocusStatus> {
        // SAFETY: query-only focus; foreign PID is rejected before other facts.
        unsafe {
            let focused = self.call("Paint actual focused element", || {
                self.automation.GetFocusedElement()
            })?;
            let pid = self.call("Paint focused PID", || focused.CurrentProcessId())?;
            focus_status(pid, self.target.process_id, || {
                Ok(self
                    .call("Paint focused canvas membership", || {
                        self.automation.CompareElements(canvas, &focused)
                    })?
                    .as_bool()
                    && self
                        .call("Paint actual canvas focus", || {
                            canvas.CurrentHasKeyboardFocus()
                        })?
                        .as_bool())
            })
        }
    }
}

/// Caller retains SourceLease/Job/worker/IO and owns this thread's MTA before
/// entry. No COM object escapes. Release may block and requires actual Job join.
pub(crate) fn observe(target: &Target, budget: &SourceBudget) -> Result<Snapshot> {
    let start = Instant::now();
    let qpc_start = budget.call(platform::clock_100ns)?;
    let deadline = start
        .checked_add(QUERY_LIMIT.min(budget.remaining()?))
        .ok_or(Error::Limit)?;
    // SAFETY: caller-owned initialized MTA; read-only owned COM interfaces.
    let automation: IUIAutomation = budget.call(|| {
        platform::api("Paint UIA", unsafe {
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        })
    })?;
    let q = Query {
        automation,
        target,
        source_budget: budget,
        deadline,
    };
    // SAFETY: exact revalidated selected native HWND only, never desktop root.
    let root = q.call("Paint selected native root", || unsafe {
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
    if q.call("Paint root HWND", || unsafe {
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
    let canvas_facts = Observed::capture((|| {
        q.pid(&canvas)?;
        // SAFETY: named current tool witness is confined to the rooted canvas.
        Ok(CanvasFacts {
            element: q.facts(&canvas)?,
            current_tool_name: text(
                q.call("Paint canvas tool name", || unsafe { canvas.CurrentName() })?,
            )?,
        })
    })());
    let brush_reads = Observed::capture((|| {
        let split = q.unique(
            &brushes,
            &Selector::xaml(
                "Microsoft.UI.Xaml.Controls.SplitButton",
                "BrushesSplitButton",
                None,
                50013,
            ),
        )?;
        // Name is observed separately: it may describe the currently selected
        // style. Class/ID/type/framework and unique rooted parent bind identity.
        let primary = q.raw_unique(
            &split,
            &Selector::xaml("Button", "PrimaryButton", None, 50000),
        );
        let primary_name = Observed::capture(
            primary
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|v| q.name(v)),
        );
        let primary_button = Observed::capture(
            primary
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|v| q.facts(v)),
        );
        Ok(BrushRead {
            split_name: Observed::capture(q.name(&split)),
            selected: Observed::capture(q.selected(&split)),
            style: Observed::capture(q.value_text(&split)),
            primary_name,
            primary_button,
        })
    })());
    let brush = Observed::capture(match &brush_reads {
        Observed::Known { value } => brush_facts(value),
        Observed::Unknown { error } => Err(error.clone()),
    });
    let size_thumb = Observed::capture(q.thumb(
        &pane,
        &Selector::xaml("Slider", "", Some("Size"), 50015),
        "VerticalThumb",
    ));
    let zoom_thumb = Observed::capture(q.thumb(
        &pane,
        &Selector::xaml("Slider", "ZoomSliderControl", Some("Zoom"), 50015),
        "HorizontalThumb",
    ));
    let toggles = Observed::capture(q.toggles(&tools));
    let recognized_tool = Observed::capture(match (&brush, &toggles, &canvas_facts) {
        (
            Observed::Known { value: b },
            Observed::Known { value: t },
            Observed::Known { value: c },
        ) => recognize(b, t, &c.current_tool_name),
        _ => Err(Error::Unavailable),
    });
    let size =
        Observed::capture(q.numeric(&pane, &Selector::xaml("Slider", "", Some("Size"), 50015)));
    let watched_opacity =
        Observed::capture(q.numeric(&pane, &Selector::xaml("Slider", "", Some("Opacity"), 50015)));
    let zoom = Observed::capture(q.numeric(
        &pane,
        &Selector::xaml("Slider", "ZoomSliderControl", Some("Zoom"), 50015),
    ));
    let zoom_edit = Observed::capture((|| {
        let combo = q.unique(
            &pane,
            &Selector::xaml("ComboBox", "ZoomValuesComboBox", Some("Zoom"), 50003),
        )?;
        q.numeric(
            &combo,
            &Selector::xaml("TextBox", "EditableText", Some("Zoom"), 50004),
        )
    })());
    let focus = Observed::capture(q.focus(&canvas));
    let undo = Observed::capture(q.button(&pane, "AppBarButton", "Undo"));
    let redo = Observed::capture(q.button(&pane, "AppBarButton", "Redo"));
    let zoom_in = Observed::capture(q.button(&pane, "Button", "Zoom in"));
    let zoom_out = Observed::capture(q.button(&pane, "Button", "Zoom out"));
    let fit_to_window = Observed::capture(q.button(&pane, "Button", "Fit to window"));
    let color_names = Observed::capture(q.colors(&toolbar));
    let settings_known = [
        brush.is_known(),
        toggles.is_known(),
        recognized_tool.is_known(),
        numeric_known(&size, true),
        numeric_known(&watched_opacity, false),
        numeric_known(&zoom, true),
        canvas_facts.is_known(),
        color_names.is_known(),
    ]
    .iter()
    .all(|v| *v);
    let complete = settings_known
        && [
            numeric_known(&watched_opacity, true),
            numeric_known(&zoom_edit, true),
            focus.is_known(),
            undo.is_known(),
            redo.is_known(),
            zoom_in.is_known(),
            zoom_out.is_known(),
            fit_to_window.is_known(),
        ]
        .iter()
        .all(|v| *v);
    // Digest binds the observed values for later experiment request equality;
    // it is neither profile provenance nor an effect receipt. Unknown stays None.
    let digest = if settings_known {
        Some(sha256(
            &serde_json::to_vec(&(
                &brush,
                &toggles,
                &size,
                &watched_opacity,
                &zoom,
                &canvas_facts,
                &color_names,
            ))
            .map_err(|_| Error::Invalid)?,
        ))
    } else {
        None
    };
    budget.check()?;
    let end = budget.call(platform::clock_100ns)?;
    let mut snapshot = Snapshot {
        schema: 1,
        measured_editor_package: vw_host::editor_paint_package::PAINT_FULL_NAME,
        selected_root_only: true,
        tree_is_atomic: false,
        max_matches_per_query: MAX_MATCHES,
        cooperative_query_limit_ms: 3000,
        elapsed_micros: start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
        observed_start_qpc_100ns: qpc_start,
        observed_end_qpc_100ns: end,
        complete,
        required_settings_known: settings_known,
        brush,
        brush_reads,
        size_thumb,
        zoom_thumb,
        tool_toggles: toggles,
        recognized_tool,
        size,
        watched_opacity,
        zoom,
        zoom_edit,
        canvas: canvas_facts,
        focus,
        undo,
        redo,
        zoom_in,
        zoom_out,
        fit_to_window,
        color_names,
        current_settings_digest: digest,
        production_watched_settings_digest: Observed::Unknown {
            error: Error::Unavailable,
        },
        input_sent: false,
        profile_authority: false,
        effect_proven: false,
        frame_witness: Observed::Unknown {
            error: Error::Unavailable,
        },
    };
    snapshot.production_watched_settings_digest =
        Observed::capture(production::snapshot_watched_digest(&snapshot));
    budget.check()?;
    snapshot.frame_witness = Observed::capture(snapshot.frame_witness());
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn primitive_raw_walk_is_bounded_and_cannot_select_an_ordinal() -> Result<()> {
        raw_child_budget(MAX_RAW_PRIMITIVE_CHILDREN - 1)?;
        assert_eq!(
            raw_child_budget(MAX_RAW_PRIMITIVE_CHILDREN),
            Err(Error::Limit)
        );
        assert_eq!(single_primitive(vec![7]), Ok(7));
        assert_eq!(single_primitive(Vec::<u8>::new()), Err(Error::Unavailable));
        assert_eq!(single_primitive(vec![7, 8]), Err(Error::Unavailable));
        Ok(())
    }
    #[test]
    fn dynamic_names_do_not_replace_independent_brush_style_or_selection() -> Result<()> {
        let facts = ElementFacts {
            runtime_id_hash: "a".repeat(64),
            physical_rect_host: None,
            visible_rect_host: None,
            enabled: true,
            offscreen: false,
            destination_eligible: false,
        };
        let mut reads = BrushRead {
            split_name: Observed::Known {
                value: "Brushes, Brush".into(),
            },
            selected: Observed::Known { value: true },
            style: Observed::Known {
                value: "Brush".into(),
            },
            primary_name: Observed::Known {
                value: "Brush".into(),
            },
            primary_button: Observed::Known { value: facts },
        };
        assert_eq!(brush_facts(&reads)?.style, "Brush");
        reads.style = Observed::Unknown {
            error: Error::Unavailable,
        };
        assert_eq!(brush_facts(&reads), Err(Error::Unavailable));
        reads.style = Observed::Known {
            value: "Brush".into(),
        };
        reads.selected = Observed::Unknown {
            error: Error::Unavailable,
        };
        assert_eq!(brush_facts(&reads), Err(Error::Unavailable));
        Ok(())
    }
    #[test]
    fn frame_witness_rejects_unknown_properties_and_accepts_no_authority_fields() -> Result<()> {
        let raw = serde_json::json!({"settingsDigest":"a".repeat(64),"canvasRuntimeIdHash":"b".repeat(64),
            "canvasRectHost":{"x":0,"y":0,"width":40,"height":30},"canvasToolName":"Using Brush tool on Canvas",
            "observedStartQpc100ns":1,"observedEndQpc100ns":2});
        let value: PaintFrameWitness =
            serde_json::from_value(raw.clone()).map_err(|_| Error::Invalid)?;
        assert_eq!(value.canvas_rect_host.width, 40);
        value.validate()?;
        let mut invalid = value.clone();
        invalid.observed_start_qpc_100ns = 3;
        assert_eq!(invalid.validate(), Err(Error::Invalid));
        invalid = value.clone();
        invalid.canvas_runtime_id_hash = "z".repeat(64);
        assert_eq!(invalid.validate(), Err(Error::Invalid));
        invalid = value.clone();
        invalid.canvas_tool_name = "Using Brush tool on Canvas, guessed".into();
        assert_eq!(invalid.validate(), Err(Error::Invalid));
        let mut injected = raw;
        injected["profileAuthority"] = serde_json::json!(true);
        assert!(serde_json::from_value::<PaintFrameWitness>(injected).is_err());
        Ok(())
    }
    #[test]
    fn text_and_cardinality_refuse_truncation_aliases_and_ordinals() -> Result<()> {
        assert_eq!(bounded_text(&[0xD800]), Err(Error::Invalid));
        assert_eq!(bounded_text(&[0u16]), Err(Error::Limit));
        assert_eq!(bounded_text(&[0x61; 257]), Err(Error::Limit));
        assert_eq!(
            bounded_text(&"Size".encode_utf16().collect::<Vec<_>>())?,
            "Size"
        );
        for count in [-1, 9, 10000] {
            assert_eq!(bounded_count(count), Err(Error::Limit));
        }
        assert_eq!(bounded_count(8)?, 8);
        Ok(())
    }
    #[test]
    fn foreign_focus_cannot_query_names_values_or_runtime() -> Result<()> {
        let entered = std::cell::Cell::new(false);
        assert_eq!(
            focus_status(99, 42, || {
                entered.set(true);
                Ok(true)
            })?,
            FocusStatus::ForeignOrUnknownPid
        );
        assert!(!entered.get());
        assert_eq!(
            focus_status(42, 42, || Ok(false))?,
            FocusStatus::SelectedOtherOrUnproved
        );
        assert_eq!(
            focus_status(42, 42, || Ok(true))?,
            FocusStatus::SelectedCanvas
        );
        Ok(())
    }
    #[test]
    fn anonymous_ranges_do_not_replace_tool_witness_and_conflict_refuses() -> Result<()> {
        let mut brush = BrushFacts {
            selected: true,
            style: "Brush".into(),
            primary_button: ElementFacts {
                runtime_id_hash: "a".repeat(64),
                physical_rect_host: None,
                visible_rect_host: None,
                enabled: true,
                offscreen: true,
                destination_eligible: false,
            },
        };
        let mut toggles = vec![ToggleFacts {
            automation_id: "EraserTool".into(),
            semantic_name: "Eraser".into(),
            on: false,
        }];
        assert_eq!(
            recognize(&brush, &toggles, "Using Brush tool on Canvas")?,
            RecognizedTool::Brush
        );
        toggles[0].on = true;
        assert_eq!(
            recognize(&brush, &toggles, "Using Brush tool on Canvas"),
            Err(Error::Unavailable)
        );
        brush.selected = false;
        assert_eq!(
            recognize(&brush, &toggles, "Using Eraser tool on Canvas")?,
            RecognizedTool::Eraser
        );
        assert_eq!(
            recognize(&brush, &toggles, "Using Brush tool on Canvas"),
            Err(Error::Unavailable)
        );
        Ok(())
    }
    #[test]
    fn visible_crop_preserves_negative_origin_and_empty_edit_is_not_destination() {
        let client = Rect {
            x: -100,
            y: -50,
            width: 200,
            height: 100,
        };
        assert_eq!(
            intersection(
                Rect {
                    x: -120,
                    y: -40,
                    width: 100,
                    height: 120
                },
                client
            ),
            Some(Rect {
                x: -100,
                y: -40,
                width: 80,
                height: 90
            })
        );
        assert_eq!(
            intersection(
                Rect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0
                },
                client
            ),
            None
        );
        let range = RangeFacts {
            current: f64::NAN,
            minimum: 0.0,
            maximum: 100.0,
            small_change: 1.0,
            large_change: 5.0,
            read_only: false,
        };
        assert_eq!(range_valid(&range), Err(Error::Invalid));
    }
    #[test]
    fn unknown_nested_numeric_state_cannot_supply_required_settings() {
        let element = ElementFacts {
            runtime_id_hash: "a".repeat(64),
            physical_rect_host: None,
            visible_rect_host: None,
            enabled: true,
            offscreen: true,
            destination_eligible: false,
        };
        let mut value = Observed::Known {
            value: NumericControl {
                element,
                value: Observed::Unknown {
                    error: Error::Unavailable,
                },
                range: Observed::Unknown {
                    error: Error::Unavailable,
                },
            },
        };
        assert!(!numeric_known(&value, false));
        if let Observed::Known { value: control } = &mut value {
            control.value = Observed::Known {
                value: ValueFacts {
                    text: "100%".into(),
                    read_only: false,
                },
            };
        }
        assert!(numeric_known(&value, false));
        assert!(!numeric_known(&value, true));
    }
}
