//! Small selected-root queries. No desktop walk, UIA mutation or cached authority.
use super::{api, runtime_hash};
use crate::{Error, Result};
use std::time::{Duration, Instant};
use vw_remote::{
    Rect, Target,
    profile::{
        essential::RangeShape,
        live::{self, Route, Selector, ToolSettings},
    },
};
use windows::{
    Win32::{
        Foundation::{HWND, POINT},
        System::{Com::*, Variant::VARIANT},
        UI::Accessibility::*,
    },
    core::BSTR,
};

pub const PRODUCTION_BUDGET_MS: u64 = 180;
pub struct Budget {
    start: Instant,
    limit: Duration,
}
impl Budget {
    pub fn production() -> Self {
        Self::new(Duration::from_millis(PRODUCTION_BUDGET_MS))
    }
    pub fn new(limit: Duration) -> Self {
        Self {
            start: Instant::now(),
            limit,
        }
    }
    pub fn check(&self) -> Result<()> {
        if self.start.elapsed() >= self.limit {
            Err(Error::Timeout)
        } else {
            Ok(())
        }
    }
    fn call<T>(&self, phase: &str, call: impl FnOnce() -> windows::core::Result<T>) -> Result<T> {
        self.check()?;
        let result = api(phase, call());
        self.check()?;
        result
    }
    pub fn elapsed_micros(&self) -> u64 {
        self.start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
    }
}
fn text(value: BSTR) -> Result<String> {
    let units: &[u16] = &value;
    if units.len() > 256 {
        return Err(Error::Limit);
    }
    let value = String::from_utf16(units).map_err(|_| Error::Invalid)?;
    if value.len() > 256 || value.chars().any(char::is_control) {
        return Err(Error::Limit);
    }
    Ok(value)
}
fn one(count: i32) -> Result<()> {
    if count == 1 {
        Ok(())
    } else {
        Err(Error::Unavailable)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ElementProof {
    pub runtime_id_hash: String,
    pub physical_rect: Rect,
    pub enabled: bool,
    pub offscreen: bool,
}
/// Complete watched numeric facts, with no size/opacity or ordinal authority.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub(crate) struct NumericWitness {
    pub element: ElementProof,
    pub current: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub small_change: f64,
    pub large_change: f64,
    pub read_only: bool,
    pub value: String,
}
impl NumericWitness {
    pub(crate) fn validate(&self) -> Result<()> {
        if ![
            self.current,
            self.minimum,
            self.maximum,
            self.small_change,
            self.large_change,
        ]
        .into_iter()
        .all(f64::is_finite)
            || self.minimum > self.maximum
            || self.current < self.minimum
            || self.current > self.maximum
            || self.small_change < 0.0
            || self.large_change < 0.0
            || self.value.is_empty()
        {
            Err(Error::Unavailable)
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct NumericProof {
    pub element: ElementProof,
    pub range: RangeShape,
    pub current: f64,
    pub value_text: String,
    pub read_only: bool,
}
#[derive(Clone, Copy)]
enum NumericInputPolicy {
    Arrow,
    Wheel,
}
impl NumericInputPolicy {
    fn validate(self, shape: &RangeShape) -> Result<()> {
        match self {
            Self::Arrow => shape.validate(),
            Self::Wheel => shape.validate_wheel(),
        }
    }
}
pub struct Observer<'a> {
    automation: IUIAutomation,
    root: IUIAutomationElement,
    target: &'a Target,
    budget: &'a Budget,
}
impl<'a> Observer<'a> {
    pub fn open(target: &'a Target, budget: &'a Budget) -> Result<Self> {
        // SAFETY: caller owns the isolated helper's MTA; query-only live COM
        // objects remain owned on this thread until release/helper retirement.
        unsafe {
            let automation: IUIAutomation = budget.call("tool observer UIA", || {
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            })?;
            let root = budget.call("tool selected root", || {
                automation.ElementFromHandle(HWND(target.window as usize as *mut std::ffi::c_void))
            })?;
            let value = Self {
                automation,
                root,
                target,
                budget,
            };
            value.pid(&value.root)?;
            let expected = live::root_selector();
            value.matches(&value.root, &expected, false)?;
            if budget.call("tool selected root HWND", || {
                value.root.CurrentNativeWindowHandle()
            })? != HWND(target.window as usize as *mut std::ffi::c_void)
            {
                return Err(Error::TargetChanged);
            }
            Ok(value)
        }
    }
    fn pid(&self, element: &IUIAutomationElement) -> Result<()> {
        // SAFETY: query only. PID precedes any name/value/runtime properties.
        let pid = self
            .budget
            .call("tool element PID", || unsafe { element.CurrentProcessId() })?;
        if pid > 0 && pid as u32 == self.target.process_id {
            Ok(())
        } else {
            Err(Error::TargetChanged)
        }
    }
    fn matches(
        &self,
        element: &IUIAutomationElement,
        expected: &Selector,
        name: bool,
    ) -> Result<()> {
        self.pid(element)?;
        // SAFETY: owned same-selected-PID COM element; bounded property copies.
        unsafe {
            if text(
                self.budget
                    .call("tool framework", || element.CurrentFrameworkId())?,
            )? != "Qt"
                || text(
                    self.budget
                        .call("tool class", || element.CurrentClassName())?,
                )? != expected.class_name
                || text(
                    self.budget
                        .call("tool automation ID", || element.CurrentAutomationId())?,
                )? != expected.automation_id
                || self
                    .budget
                    .call("tool control type", || element.CurrentControlType())?
                    .0
                    != expected.control_type
                || (name
                    && !expected.name.is_empty()
                    && text(
                        self.budget
                            .call("tool semantic name", || element.CurrentName())?,
                    )? != expected.name)
            {
                return Err(Error::Unavailable);
            }
            Ok(())
        }
    }
    fn condition(&self, selector: &Selector) -> Result<IUIAutomationCondition> {
        let properties = [
            (
                UIA_ProcessIdPropertyId,
                VARIANT::from(self.target.process_id as i32),
            ),
            (UIA_FrameworkIdPropertyId, VARIANT::from("Qt")),
            (
                UIA_ClassNamePropertyId,
                VARIANT::from(selector.class_name.as_str()),
            ),
            (
                UIA_AutomationIdPropertyId,
                VARIANT::from(selector.automation_id.as_str()),
            ),
            (
                UIA_ControlTypePropertyId,
                VARIANT::from(selector.control_type),
            ),
        ];
        // SAFETY: initialized owned VARIANTs are copied by UIA into owned
        // conditions; all search scopes are the retained selected root/anchor.
        unsafe {
            let mut condition = self.budget.call("tool condition", || {
                self.automation
                    .CreatePropertyCondition(properties[0].0, &properties[0].1)
            })?;
            for (property, value) in &properties[1..] {
                let next = self.budget.call("tool condition", || {
                    self.automation.CreatePropertyCondition(*property, value)
                })?;
                condition = self.budget.call("tool combined condition", || {
                    self.automation.CreateAndCondition(&condition, &next)
                })?;
            }
            if !selector.name.is_empty() {
                let name = self.budget.call("tool name condition", || {
                    self.automation.CreatePropertyCondition(
                        UIA_NamePropertyId,
                        &VARIANT::from(selector.name.as_str()),
                    )
                })?;
                condition = self.budget.call("tool named condition", || {
                    self.automation.CreateAndCondition(&condition, &name)
                })?;
            }
            Ok(condition)
        }
    }
    fn unique(
        &self,
        parent: &IUIAutomationElement,
        scope: TreeScope,
        selector: &Selector,
    ) -> Result<IUIAutomationElement> {
        self.pid(parent)?;
        let condition = self.condition(selector)?;
        // SAFETY: selected native-root scope or an already uniquely proved direct
        // child anchor only. Provider allocation/blocked calls are Job-contained;
        // cooperative deadline refuses after return, never claims interruption.
        unsafe {
            let found = self
                .budget
                .call("tool scoped lookup", || parent.FindAll(scope, &condition))?;
            one(self.budget.call("tool unique result", || found.Length())?)?;
            let element = self
                .budget
                .call("tool unique element", || found.GetElement(0))?;
            self.matches(&element, selector, true)?;
            Ok(element)
        }
    }
    pub fn element(&self, route: &Route) -> Result<IUIAutomationElement> {
        let anchor = self.unique(&self.root, TreeScope_Children, &route.anchor)?;
        self.unique(&anchor, TreeScope_Descendants, &route.leaf)
    }
    fn numeric_proof(
        &self,
        element: &IUIAutomationElement,
        policy: NumericInputPolicy,
    ) -> Result<NumericProof> {
        self.pid(element)?;
        // SAFETY: read-only patterns on an actual selected-root element.
        unsafe {
            let range: IUIAutomationRangeValuePattern =
                self.budget.call("numeric range pattern", || {
                    element.GetCurrentPatternAs(UIA_RangeValuePatternId)
                })?;
            let value: IUIAutomationLegacyIAccessiblePattern =
                self.budget.call("numeric value pattern", || {
                    element.GetCurrentPatternAs(UIA_LegacyIAccessiblePatternId)
                })?;
            let shape = RangeShape {
                minimum: self
                    .budget
                    .call("numeric minimum", || range.CurrentMinimum())?,
                maximum: self
                    .budget
                    .call("numeric maximum", || range.CurrentMaximum())?,
                small_change: self
                    .budget
                    .call("numeric small step", || range.CurrentSmallChange())?,
                large_change: self
                    .budget
                    .call("numeric large step", || range.CurrentLargeChange())?,
            };
            policy.validate(&shape)?;
            let current = self
                .budget
                .call("numeric current", || range.CurrentValue())?;
            if !current.is_finite() || current < shape.minimum || current > shape.maximum {
                return Err(Error::Invalid);
            }
            Ok(NumericProof {
                element: self.proof(element)?,
                range: shape,
                current,
                value_text: text(
                    self.budget
                        .call("numeric actual value", || value.CurrentValue())?,
                )?,
                read_only: self
                    .budget
                    .call("numeric range readonly", || range.CurrentIsReadOnly())?
                    .as_bool(),
            })
        }
    }
    /// Actual shape filtering follows root/ancestor/class proof. The trusted
    /// profile must bind a semantic calibration receipt; range syntax supplies
    /// no brush-size/zoom authority. Multiple matches always refuse.
    pub fn numeric(
        &self,
        route: &Route,
        shape: &RangeShape,
    ) -> Result<(IUIAutomationElement, NumericProof)> {
        self.numeric_matching(route, shape, NumericInputPolicy::Arrow)
    }
    pub fn numeric_wheel(
        &self,
        route: &Route,
        shape: &RangeShape,
    ) -> Result<(IUIAutomationElement, NumericProof)> {
        self.numeric_matching(route, shape, NumericInputPolicy::Wheel)
    }
    fn numeric_matching(
        &self,
        route: &Route,
        shape: &RangeShape,
        policy: NumericInputPolicy,
    ) -> Result<(IUIAutomationElement, NumericProof)> {
        policy.validate(shape)?;
        let anchor = self.unique(&self.root, TreeScope_Children, &route.anchor)?;
        let condition = self.condition(&route.leaf)?;
        // SAFETY: bounded read-only descendants of one exact named ancestor.
        unsafe {
            let found = self.budget.call("numeric bounded candidates", || {
                anchor.FindAll(TreeScope_Descendants, &condition)
            })?;
            let count = self
                .budget
                .call("numeric candidate count", || found.Length())?;
            if !(1..=8).contains(&count) {
                return Err(Error::Unavailable);
            }
            let mut selected = None;
            for index in 0..count {
                let element = self
                    .budget
                    .call("numeric candidate", || found.GetElement(index))?;
                self.matches(&element, &route.leaf, true)?;
                let proof = self.numeric_proof(&element, policy)?;
                if proof.range == *shape {
                    if selected.is_some() {
                        return Err(Error::Unavailable);
                    }
                    selected = Some((element, proof));
                }
            }
            selected.ok_or(Error::Unavailable)
        }
    }
    pub fn focused_numeric(&self, element: &IUIAutomationElement) -> Result<()> {
        // SAFETY: read-only current focus, selected PID first. No same-title,
        // same-process sibling or presumed focus-after-click may authorize keys.
        unsafe {
            let focused = self.budget.call("numeric actual focus", || {
                self.automation.GetFocusedElement()
            })?;
            self.pid(&focused)?;
            if !self
                .budget
                .call("numeric exact focused element", || {
                    self.automation.CompareElements(element, &focused)
                })?
                .as_bool()
                || !self
                    .budget
                    .call("numeric actual keyboard focus", || {
                        focused.CurrentHasKeyboardFocus()
                    })?
                    .as_bool()
            {
                return Err(Error::Ungranted);
            }
            self.budget.check()
        }
    }
    fn toggle(&self, element: &IUIAutomationElement) -> Result<bool> {
        self.pid(element)?;
        // SAFETY: read-only current toggle pattern; never Toggle or Invoke.
        unsafe {
            let pattern: IUIAutomationTogglePattern =
                self.budget.call("tool toggle pattern", || {
                    element.GetCurrentPatternAs(UIA_TogglePatternId)
                })?;
            match self
                .budget
                .call("tool actual toggle state", || pattern.CurrentToggleState())?
            {
                state if state == ToggleState_On => Ok(true),
                state if state == ToggleState_Off => Ok(false),
                _ => Err(Error::Unavailable),
            }
        }
    }
    fn selected_tool_id(&self) -> Result<String> {
        let route = live::route_for(3)?;
        let anchor = self.unique(&self.root, TreeScope_Children, &route.anchor)?;
        // SAFETY: get-only current selection inside the exact selected toolbox.
        // The current result must be unique; unknown IDs never become authority.
        unsafe {
            let class = self.budget.call("tool selected class condition", || {
                self.automation.CreatePropertyCondition(
                    UIA_ClassNamePropertyId,
                    &VARIANT::from("KoToolBoxButton"),
                )
            })?;
            let on = self.budget.call("tool selected state condition", || {
                self.automation
                    .CreatePropertyCondition(UIA_ToggleToggleStatePropertyId, &VARIANT::from(1i32))
            })?;
            let condition = self.budget.call("tool selected condition", || {
                self.automation.CreateAndCondition(&class, &on)
            })?;
            let found = self.budget.call("tool selected scoped lookup", || {
                anchor.FindAll(TreeScope_Descendants, &condition)
            })?;
            one(self
                .budget
                .call("tool selected unique", || found.Length())?)?;
            let element = self
                .budget
                .call("tool actual selected", || found.GetElement(0))?;
            self.pid(&element)?;
            if !self.toggle(&element)? {
                return Err(Error::Unavailable);
            }
            text(
                self.budget
                    .call("tool selected actual ID", || element.CurrentAutomationId())?,
            )
        }
    }
    pub fn settings(&self) -> Result<ToolSettings> {
        let brush = self.element(&live::route_for(3)?)?;
        let eraser = self.element(&live::route_for(4)?)?;
        let preset = self.element(&live::preset_route())?;
        let blend = self.element(&live::blend_route())?;
        let alpha = self.element(&live::alpha_route())?;
        // SAFETY: uniquely scoped same-PID live elements; get-only patterns.
        unsafe {
            let blend_value: IUIAutomationValuePattern =
                self.budget.call("tool blend value pattern", || {
                    blend.GetCurrentPatternAs(UIA_ValuePatternId)
                })?;
            let value = ToolSettings {
                freehand_selected: self.toggle(&brush)?,
                eraser_mode: self.toggle(&eraser)?,
                selected_tool_id: self.selected_tool_id()?,
                preset: text(
                    self.budget
                        .call("tool preset label", || preset.CurrentName())?,
                )?,
                blending_mode: text(
                    self.budget
                        .call("tool actual blend value", || blend_value.CurrentValue())?,
                )?,
                preserve_alpha: self.toggle(&alpha)?,
            };
            self.budget.check()?;
            Ok(value)
        }
    }
    pub fn canvas(&self) -> Result<(IUIAutomationElement, Rect)> {
        let route = Route {
            anchor: Selector {
                class_name: "QStackedWidget".into(),
                automation_id: "".into(),
                name: "".into(),
                control_type: 50025,
            },
            leaf: Selector {
                class_name: "KisOpenGLCanvas2".into(),
                automation_id: "".into(),
                name: "".into(),
                control_type: 50026,
            },
        };
        let canvas = self.element(&route)?;
        let rect = self.proof(&canvas)?.physical_rect;
        Ok((canvas, rect))
    }
    pub fn focused_canvas(&self) -> Result<(IUIAutomationElement, Rect)> {
        let (canvas, rect) = self.canvas()?;
        // SAFETY: one focus query; foreign PID rejected before any extra property.
        unsafe {
            let focus = self.budget.call("tool current focused element", || {
                self.automation.GetFocusedElement()
            })?;
            self.pid(&focus)?;
            if !self
                .budget
                .call("tool focused selected canvas", || {
                    self.automation.CompareElements(&canvas, &focus)
                })?
                .as_bool()
                || !self
                    .budget
                    .call("tool canvas keyboard focus", || {
                        canvas.CurrentHasKeyboardFocus()
                    })?
                    .as_bool()
            {
                return Err(Error::Ungranted);
            }
        }
        Ok((canvas, rect))
    }
    /// Query-only destination witness. No provider follows this final hit proof
    /// in the finite consumer; the shared owner still performs native guarding.
    pub(crate) fn hit_exact(
        &self,
        element: &IUIAutomationElement,
        point: (i32, i32),
    ) -> Result<()> {
        self.pid(element)?;
        // SAFETY: bounded physical point in an independently rooted live element.
        let hit = self.budget.call("Krita actual destination", || unsafe {
            self.automation.ElementFromPoint(POINT {
                x: point.0,
                y: point.1,
            })
        })?;
        self.pid(&hit)?;
        // SAFETY: query-only actual same-PID element comparison. An anonymous
        // other control or matching class is never a destination identity.
        if !self
            .budget
            .call("Krita exact hit element", || unsafe {
                self.automation.CompareElements(element, &hit)
            })?
            .as_bool()
        {
            return Err(Error::TargetChanged);
        }
        Ok(())
    }
    pub(crate) fn brush_numeric_witnesses(&self) -> Result<Vec<NumericWitness>> {
        let anchor = self.unique(&self.root, TreeScope_Children, &live::brushes_anchor())?;
        let selector = Selector {
            class_name: "KisDoubleSliderSpinBox".into(),
            automation_id: String::new(),
            name: String::new(),
            control_type: 50016,
        };
        let condition = self.condition(&selector)?;
        // SAFETY: only measured brush-toolbar descendants, never desktop scope.
        let found = self.budget.call("Krita numeric witnesses", || unsafe {
            anchor.FindAll(TreeScope_Descendants, &condition)
        })?;
        if self
            .budget
            .call("Krita bounded numeric count", || unsafe { found.Length() })?
            != 2
        {
            return Err(Error::Unavailable);
        }
        let mut result = Vec::new();
        for index in 0..2 {
            // Index enumerates every candidate; it never selects a semantic role.
            let element = self.budget.call("Krita numeric witness", || unsafe {
                found.GetElement(index)
            })?;
            self.matches(&element, &selector, false)?;
            let proof = self.proof(&element)?;
            // SAFETY: PID was proved before read-only patterns and values.
            let range: IUIAutomationRangeValuePattern =
                self.budget.call("Krita watched range", || unsafe {
                    element.GetCurrentPatternAs(UIA_RangeValuePatternId)
                })?;
            let legacy: IUIAutomationLegacyIAccessiblePattern =
                self.budget.call("Krita watched legacy value", || unsafe {
                    element.GetCurrentPatternAs(UIA_LegacyIAccessiblePatternId)
                })?;
            let current = self
                .budget
                .call("Krita watched current", || unsafe { range.CurrentValue() })?;
            let minimum = self.budget.call("Krita watched minimum", || unsafe {
                range.CurrentMinimum()
            })?;
            let maximum = self.budget.call("Krita watched maximum", || unsafe {
                range.CurrentMaximum()
            })?;
            let small_change = self.budget.call("Krita watched small change", || unsafe {
                range.CurrentSmallChange()
            })?;
            let large_change = self.budget.call("Krita watched large change", || unsafe {
                range.CurrentLargeChange()
            })?;
            let read_only = self
                .budget
                .call("Krita watched read-only", || unsafe {
                    range.CurrentIsReadOnly()
                })?
                .as_bool();
            let value = text(
                self.budget
                    .call("Krita watched value", || unsafe { legacy.CurrentValue() })?,
            )?;
            let witness = NumericWitness {
                element: proof,
                current,
                minimum,
                maximum,
                small_change,
                large_change,
                read_only,
                value,
            };
            witness.validate()?;
            result.push(witness);
        }
        result.sort_by(|a, b| a.element.runtime_id_hash.cmp(&b.element.runtime_id_hash));
        let [first, second] = result.as_slice() else {
            return Err(Error::Unavailable);
        };
        if first.element.runtime_id_hash == second.element.runtime_id_hash {
            return Err(Error::Unavailable);
        }
        self.budget.check()?;
        Ok(result)
    }
    pub fn proof(&self, element: &IUIAutomationElement) -> Result<ElementProof> {
        self.pid(element)?;
        // SAFETY: retained current native element, typed initialized property return.
        unsafe {
            let bounds = self
                .budget
                .call("tool actual bounds", || element.CurrentBoundingRectangle())?;
            let rect = Rect {
                x: bounds.left,
                y: bounds.top,
                width: u32::try_from(i64::from(bounds.right) - i64::from(bounds.left))
                    .map_err(|_| Error::Invalid)?,
                height: u32::try_from(i64::from(bounds.bottom) - i64::from(bounds.top))
                    .map_err(|_| Error::Invalid)?,
            };
            if !rect.inside(self.target.client_rect) {
                return Err(Error::TargetChanged);
            }
            let enabled = self
                .budget
                .call("tool actual enabled", || element.CurrentIsEnabled())?
                .as_bool();
            let offscreen = self
                .budget
                .call("tool actual offscreen", || element.CurrentIsOffscreen())?
                .as_bool();
            self.budget.check()?;
            let runtime_id_hash = runtime_hash(element)?;
            self.budget.check()?;
            if !enabled || offscreen {
                return Err(Error::Unavailable);
            }
            Ok(ElementProof {
                runtime_id_hash,
                physical_rect: rect,
                enabled,
                offscreen,
            })
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_missing_negative_provider_count_never_selects_an_ordinal() {
        for count in [-1, 0, 2, 500] {
            assert_eq!(one(count), Err(Error::Unavailable));
        }
        assert_eq!(one(1), Ok(()));
    }
    #[test]
    fn expired_budget_does_not_enter_provider() {
        let budget = Budget::new(Duration::ZERO);
        let entered = std::cell::Cell::new(false);
        let value = budget.call("fixture", || {
            entered.set(true);
            Ok(())
        });
        assert_eq!(value, Err(Error::Timeout));
        assert!(!entered.get());
    }
    #[test]
    fn oversized_or_control_text_is_refused_without_lossy_identity() {
        assert_eq!(
            text(BSTR::from("x".repeat(257).as_str())),
            Err(Error::Limit)
        );
        assert_eq!(text(BSTR::from("a\nb")), Err(Error::Limit));
    }
    #[test]
    fn watched_numeric_witness_never_hides_nan_or_unknown_current_bounds() -> Result<()> {
        let mut witness = NumericWitness {
            element: ElementProof {
                runtime_id_hash: "a".repeat(64),
                physical_rect: Rect {
                    x: 1,
                    y: 2,
                    width: 30,
                    height: 20,
                },
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
        };
        witness.validate()?;
        witness.current = f64::NAN;
        assert_eq!(witness.validate(), Err(Error::Unavailable));
        witness.current = 1001.0;
        assert_eq!(witness.validate(), Err(Error::Unavailable));
        witness.current = 40.0;
        witness.value.clear();
        assert_eq!(witness.validate(), Err(Error::Unavailable));
        Ok(())
    }
    #[test]
    fn observed_unknown_step_requires_explicit_wheel_policy() -> Result<()> {
        let actual = vw_remote::profile::essential::RangeShape {
            minimum: 0.01,
            maximum: 1000.0,
            small_change: 0.0,
            large_change: 0.0,
        };
        NumericInputPolicy::Wheel.validate(&actual)?;
        assert_eq!(
            NumericInputPolicy::Arrow.validate(&actual),
            Err(Error::Invalid)
        );
        let invalid = vw_remote::profile::essential::RangeShape {
            small_change: -1.0,
            ..actual
        };
        assert_eq!(
            NumericInputPolicy::Wheel.validate(&invalid),
            Err(Error::Invalid)
        );
        Ok(())
    }
}
