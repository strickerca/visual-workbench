//! Selected-window-only, read-only UIA. No Invoke/Select/Toggle/SetValue/focus calls.
#[path = "uia/controls.rs"]
mod controls;
use crate::{Error, Result, platform};
use serde::Serialize;
use std::{
    ptr,
    time::{Duration, Instant},
};
use vw_remote::{Rect, Target};
use windows::{
    Win32::{
        Foundation::HWND,
        System::{Com::*, Ole::*, Variant::VT_I4},
        UI::Accessibility::*,
    },
    core::{BSTR, Interface},
};
pub(crate) const MAX_NODES: usize = 512;
const MAX_DEPTH: usize = 24;
const TEXT_BYTES: usize = 256;
const NODE_OUTPUT_BUDGET: usize = super::MAX_OUTPUT - 64 * 1024;

#[derive(Serialize)]
struct Observed<T> {
    value: Option<T>,
    hresult: Option<u32>,
}
impl<T> Observed<T> {
    fn from(value: windows::core::Result<T>) -> Self {
        match value {
            Ok(v) => Self {
                value: Some(v),
                hresult: None,
            },
            Err(e) => Self {
                value: None,
                hresult: Some(e.code().0 as u32),
            },
        }
    }
    fn map<U>(self, convert: impl FnOnce(T) -> U) -> Observed<U> {
        Observed {
            value: self.value.map(convert),
            hresult: self.hresult,
        }
    }
}
#[derive(Serialize)]
struct Text {
    value: String,
    truncated: bool,
}
fn bounded_text(value: BSTR) -> Text {
    let units: &[u16] = &value;
    // Provider-owned BSTR is already allocated by COM; the Job memory limit and
    // parent retirement remain required. Only this bounded prefix is copied.
    let prefix = &units[..units.len().min(TEXT_BYTES)];
    let mut text = String::from_utf16_lossy(prefix);
    let mut end = text.len().min(TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = prefix.len() != units.len() || end != text.len();
    text.truncate(end);
    Text {
        value: text,
        truncated,
    }
}
#[derive(Serialize)]
struct Legacy {
    state: Observed<u32>,
    role: Observed<u32>,
    description: Observed<Text>,
    value: Observed<Text>,
    keyboard_shortcut: Observed<Text>,
}
#[derive(Serialize)]
struct RangeValue {
    value: Observed<Option<f64>>,
    minimum: Observed<Option<f64>>,
    maximum: Observed<Option<f64>>,
}
#[derive(Serialize)]
struct Node {
    index: usize,
    parent: Option<usize>,
    depth: usize,
    runtime_id_hash: Observed<String>,
    process_id: i32,
    name: Observed<Text>,
    framework_id: Observed<Text>,
    class_name: Observed<Text>,
    automation_id: Observed<Text>,
    control_type: Observed<i32>,
    accelerator_key: Observed<Text>,
    access_key: Observed<Text>,
    help_text: Observed<Text>,
    item_status: Observed<Text>,
    item_type: Observed<Text>,
    has_keyboard_focus: Observed<bool>,
    enabled: Observed<bool>,
    offscreen: Observed<bool>,
    physical_bounds: Observed<Rect>,
    invoke_pattern: Observed<bool>,
    toggle_state: Observed<Observed<i32>>,
    selection_item_selected: Observed<Observed<bool>>,
    value: Observed<Observed<Text>>,
    range_value: Observed<RangeValue>,
    legacy: Observed<Legacy>,
}
#[derive(Serialize)]
struct EdgeError {
    node: usize,
    relation: &'static str,
    hresult: u32,
}
#[derive(Serialize)]
struct FocusObservation {
    sampled_qpc_100ns: u64,
    status: &'static str,
    hresult: Option<u32>,
    selected_node_index: Option<usize>,
    selected_subtree_matches: usize,
    root_native_window_matches: Observed<bool>,
}
impl FocusObservation {
    fn matched(&mut self, index: usize) {
        self.selected_subtree_matches += 1;
        if self.selected_subtree_matches == 1 {
            self.status = "selected_subtree_match";
            self.selected_node_index = Some(index);
        } else {
            self.status = "ambiguous_selected_subtree_matches";
            self.selected_node_index = None;
        }
    }
    fn provider_error(&mut self, hresult: u32) {
        self.status = "provider_error";
        self.hresult = Some(hresult);
        self.selected_node_index = None;
    }
}
fn focus_candidate<T>(
    pid: Observed<i32>,
    selected_pid: u32,
    retain: impl FnOnce() -> T,
) -> (Option<T>, &'static str, Option<u32>) {
    match pid {
        Observed {
            value: Some(pid),
            hresult: None,
        } if pid > 0 && pid as u32 == selected_pid => {
            (Some(retain()), "selected_pid_unproved", None)
        }
        Observed {
            hresult: Some(error),
            ..
        } => (None, "provider_error", Some(error)),
        _ => (None, "foreign_or_unknown_pid_refused", None),
    }
}
#[derive(Serialize)]
pub(super) struct Observation {
    nodes: Vec<Node>,
    traversal_errors: Vec<EdgeError>,
    termination: &'static str,
    selected_pid_only: bool,
    max_nodes: usize,
    max_depth: usize,
    max_text_utf8_bytes: usize,
    cooperative_provider_budget_ms: u32,
    tree_is_atomic: bool,
    focused_element: FocusObservation,
}
struct Budget {
    start: Instant,
}
impl Budget {
    fn check(&self) -> Result<()> {
        if self.start.elapsed() >= Duration::from_secs(3) {
            Err(Error::Timeout)
        } else {
            Ok(())
        }
    }
    fn call<T>(&self, call: impl FnOnce() -> windows::core::Result<T>) -> Result<Observed<T>> {
        self.check()?;
        let value = Observed::from(call());
        self.check()?;
        Ok(value)
    }
    fn native<T>(&self, call: impl FnOnce() -> windows::core::Result<T>) -> Result<T> {
        self.check()?;
        let result = call();
        self.check()?;
        platform::api("probe UIA", result)
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: successful initialization on this same non-window-owning thread.
        unsafe {
            CoUninitialize();
        }
    }
}
struct RuntimeArray(*mut SAFEARRAY);
impl Drop for RuntimeArray {
    fn drop(&mut self) {
        // SAFETY: sole ownership of the SAFEARRAY returned by GetRuntimeId.
        unsafe {
            let _ = SafeArrayDestroy(self.0);
        }
    }
}
fn runtime_hash(element: &IUIAutomationElement) -> windows::core::Result<String> {
    // SAFETY: owned COM element and sole SAFEARRAY owner; rank/type/count checked
    // before accessing integers. No borrowed provider pointer escapes this call.
    unsafe {
        let raw = element.GetRuntimeId()?;
        if raw.is_null() {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x80004003u32 as i32,
            )));
        }
        let array = RuntimeArray(raw);
        let low = SafeArrayGetLBound(array.0, 1)?;
        let high = SafeArrayGetUBound(array.0, 1)?;
        let count = i64::from(high) - i64::from(low) + 1;
        if SafeArrayGetDim(array.0) != 1
            || SafeArrayGetVartype(array.0)? != VT_I4
            || !(1..=128).contains(&count)
        {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x80070057u32 as i32,
            )));
        }
        let mut data = ptr::null_mut();
        SafeArrayAccessData(array.0, &mut data)?;
        let bytes = if data.is_null() {
            None
        } else {
            Some(
                std::slice::from_raw_parts(data.cast::<i32>(), count as usize)
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
        };
        SafeArrayUnaccessData(array.0)?;
        let bytes = bytes.ok_or_else(|| {
            windows::core::Error::from_hresult(windows::core::HRESULT(0x80004003u32 as i32))
        })?;
        Ok(ring::digest::digest(&ring::digest::SHA256, &bytes)
            .as_ref()
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect())
    }
}
fn node(
    element: &IUIAutomationElement,
    process_id: i32,
    parent: Option<usize>,
    depth: usize,
    index: usize,
    b: &Budget,
) -> Result<Node> {
    // SAFETY: read-only getters/pattern queries on owned COM interfaces in this
    // helper MTA. Every provider call is observed before/after the local budget;
    // blocked calls remain owned by the outer hard-deadline Job until retirement.
    unsafe {
        let toggle = b.native(|| {
            Ok(element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId))
        })?;
        let toggle_state = match toggle {
            Ok(v) => Observed::from(Ok(b.call(|| v.CurrentToggleState())?.map(|s| s.0))),
            Err(e) => Observed::from(Err(e)),
        };
        let selection = b.native(|| {
            Ok(
                element.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                    UIA_SelectionItemPatternId,
                ),
            )
        })?;
        let selection_item_selected = match selection {
            Ok(v) => Observed::from(Ok(b.call(|| v.CurrentIsSelected())?.map(|s| s.as_bool()))),
            Err(e) => Observed::from(Err(e)),
        };
        let value_pattern = b.native(|| {
            Ok(element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId))
        })?;
        let value = match value_pattern {
            Ok(v) => Observed::from(Ok(
                Observed::from(b.native(|| Ok(v.CurrentValue()))?).map(bounded_text)
            )),
            Err(e) => Observed::from(Err(e)),
        };
        let range = b.native(|| {
            Ok(element
                .GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId))
        })?;
        let finite = |v: f64| if v.is_finite() { Some(v) } else { None };
        let range_value = match range {
            Ok(v) => Observed::from(Ok(RangeValue {
                value: b.call(|| v.CurrentValue())?.map(finite),
                minimum: b.call(|| v.CurrentMinimum())?.map(finite),
                maximum: b.call(|| v.CurrentMaximum())?.map(finite),
            })),
            Err(e) => Observed::from(Err(e)),
        };
        let legacy_pattern = b.native(|| {
            Ok(
                element.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
                    UIA_LegacyIAccessiblePatternId,
                ),
            )
        })?;
        let legacy = match legacy_pattern {
            Ok(v) => Observed::from(Ok(Legacy {
                state: b.call(|| v.CurrentState())?,
                role: b.call(|| v.CurrentRole())?,
                description: Observed::from(b.native(|| Ok(v.CurrentDescription()))?)
                    .map(bounded_text),
                value: Observed::from(b.native(|| Ok(v.CurrentValue()))?).map(bounded_text),
                keyboard_shortcut: Observed::from(b.native(|| Ok(v.CurrentKeyboardShortcut()))?)
                    .map(bounded_text),
            })),
            Err(e) => Observed::from(Err(e)),
        };
        let physical_bounds = b
            .call(|| element.CurrentBoundingRectangle())?
            .map(|r| Rect {
                x: r.left,
                y: r.top,
                width: u32::try_from(i64::from(r.right) - i64::from(r.left)).unwrap_or(0),
                height: u32::try_from(i64::from(r.bottom) - i64::from(r.top)).unwrap_or(0),
            });
        Ok(Node {
            index,
            parent,
            depth,
            process_id,
            runtime_id_hash: b.call(|| runtime_hash(element))?,
            name: Observed::from(b.native(|| Ok(element.CurrentName()))?).map(bounded_text),
            framework_id: Observed::from(b.native(|| Ok(element.CurrentFrameworkId()))?)
                .map(bounded_text),
            class_name: Observed::from(b.native(|| Ok(element.CurrentClassName()))?)
                .map(bounded_text),
            automation_id: Observed::from(b.native(|| Ok(element.CurrentAutomationId()))?)
                .map(bounded_text),
            control_type: b.call(|| element.CurrentControlType())?.map(|v| v.0),
            accelerator_key: Observed::from(b.native(|| Ok(element.CurrentAcceleratorKey()))?)
                .map(bounded_text),
            access_key: Observed::from(b.native(|| Ok(element.CurrentAccessKey()))?)
                .map(bounded_text),
            help_text: Observed::from(b.native(|| Ok(element.CurrentHelpText()))?)
                .map(bounded_text),
            item_status: Observed::from(b.native(|| Ok(element.CurrentItemStatus()))?)
                .map(bounded_text),
            item_type: Observed::from(b.native(|| Ok(element.CurrentItemType()))?)
                .map(bounded_text),
            has_keyboard_focus: b
                .call(|| element.CurrentHasKeyboardFocus())?
                .map(|v| v.as_bool()),
            enabled: b.call(|| element.CurrentIsEnabled())?.map(|v| v.as_bool()),
            offscreen: b
                .call(|| element.CurrentIsOffscreen())?
                .map(|v| v.as_bool()),
            invoke_pattern: Observed::from(
                b.native(|| Ok(element.GetCurrentPattern(UIA_InvokePatternId)))?,
            )
            .map(|_| true),
            physical_bounds,
            toggle_state,
            selection_item_selected,
            value,
            range_value,
            legacy,
        })
    }
}
fn edge(
    walker: &IUIAutomationTreeWalker,
    element: &IUIAutomationElement,
    sibling: bool,
) -> windows::core::Result<Option<IUIAutomationElement>> {
    // SAFETY: exact pinned interface ABI and initialized output. Unlike the
    // generated convenience method this preserves a successful null leaf as
    // None, independently of a genuine provider HRESULT error. A non-null
    // successful COM output reference is transferred to one owning interface.
    unsafe {
        let mut raw = ptr::null_mut();
        let table = Interface::vtable(walker);
        let get = if sibling {
            table.GetNextSiblingElement
        } else {
            table.GetFirstChildElement
        };
        get(
            Interface::as_raw(walker),
            Interface::as_raw(element),
            &mut raw,
        )
        .ok()?;
        Ok(if raw.is_null() {
            None
        } else {
            Some(IUIAutomationElement::from_raw(raw))
        })
    }
}
struct Walk<'a> {
    walker: &'a IUIAutomationTreeWalker,
    target: &'a Target,
    budget: &'a Budget,
    observation: &'a mut Observation,
    node_bytes: usize,
    automation: &'a IUIAutomation,
    focused: Option<&'a IUIAutomationElement>,
}
impl Walk<'_> {
    fn visit(
        &mut self,
        element: &IUIAutomationElement,
        parent: Option<usize>,
        depth: usize,
    ) -> Result<()> {
        let b = self.budget;
        b.check()?;
        if self.observation.nodes.len() >= self.observation.max_nodes {
            self.observation.termination = "node_limit";
            return Err(Error::Limit);
        }
        // SAFETY: query-only property. Foreign PID subtrees are never entered.
        let pid = b.native(|| unsafe { element.CurrentProcessId() })?;
        if pid <= 0 || pid as u32 != self.target.process_id {
            self.observation.termination = "foreign_pid_subtree_refused";
            return Err(Error::TargetChanged);
        }
        let index = self.observation.nodes.len();
        let record = node(element, pid, parent, depth, index, b)?;
        let size = serde_json::to_vec(&record)
            .map_err(|_| Error::Invalid)?
            .len();
        if self
            .node_bytes
            .checked_add(size)
            .is_none_or(|v| v > NODE_OUTPUT_BUDGET)
        {
            self.observation.termination = "output_limit";
            return Err(Error::Limit);
        }
        self.node_bytes += size;
        self.observation.nodes.push(record);
        if self.observation.focused_element.hresult.is_none()
            && let Some(focused) = self.focused
        {
            // SAFETY: both owned same-MTA interfaces; element is reached only
            // from the exact selected HWND subtree and its PID was checked.
            // The focused element's PID was checked before retaining it here.
            match b.call(|| unsafe { self.automation.CompareElements(focused, element) })? {
                Observed {
                    value: Some(equal),
                    hresult: None,
                } if equal.as_bool() => {
                    self.observation.focused_element.matched(index);
                }
                Observed {
                    hresult: Some(error),
                    ..
                } => {
                    self.observation.focused_element.provider_error(error);
                }
                _ => {}
            }
        }
        let first = b.native(|| Ok(edge(self.walker, element, false)))?;
        let mut child = match first {
            Ok(v) => v,
            Err(e) => {
                self.observation.traversal_errors.push(EdgeError {
                    node: index,
                    relation: "first_child",
                    hresult: e.code().0 as u32,
                });
                return Ok(());
            }
        };
        if depth == MAX_DEPTH && child.is_some() {
            self.observation.termination = "depth_limit";
            return Err(Error::Limit);
        }
        while let Some(current) = child {
            self.visit(&current, Some(index), depth + 1)?;
            child = match b.native(|| Ok(edge(self.walker, &current, true)))? {
                Ok(v) => v,
                Err(e) => {
                    self.observation.traversal_errors.push(EdgeError {
                        node: index,
                        relation: "next_sibling",
                        hresult: e.code().0 as u32,
                    });
                    break;
                }
            };
        }
        Ok(())
    }
}
pub(super) fn observe_controls(target: &Target) -> Result<serde_json::Value> {
    controls::observe(target)
}
pub(super) fn observe(target: &Target, max_nodes: usize) -> Result<Observation> {
    let b = Budget {
        start: Instant::now(),
    };
    // SAFETY: this dedicated helper thread owns no UI. All interfaces and their
    // releases remain on this MTA, before its paired CoUninitialize.
    let (apartment, automation) = unsafe {
        b.check()?;
        platform::api("probe MTA", CoInitializeEx(None, COINIT_MULTITHREADED).ok())?;
        let apartment = Apartment;
        b.check()?;
        let automation: IUIAutomation =
            b.native(|| CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER))?;
        (apartment, automation)
    };
    // SAFETY: only the exact selected HWND root. GetRootElement/desktop search is
    // deliberately absent; traversal cannot ascend from this subtree.
    let root = b.native(|| unsafe {
        automation.ElementFromHandle(HWND(target.window as usize as *mut std::ffi::c_void))
    })?;
    // SAFETY: selected HWND root only. Confirm PID before any additional root
    // property; the same retained-source checks still bracket the entire probe.
    let root_pid = b.native(|| unsafe { root.CurrentProcessId() })?;
    if root_pid <= 0 || root_pid as u32 != target.process_id {
        return Err(Error::TargetChanged);
    }
    let root_native_window_matches = b
        .call(|| unsafe { root.CurrentNativeWindowHandle() })?
        .map(|hwnd| hwnd.0 as usize as u64 == target.window);
    let mut focus = FocusObservation {
        sampled_qpc_100ns: platform::clock_100ns()?,
        status: "unavailable",
        hresult: None,
        selected_node_index: None,
        selected_subtree_matches: 0,
        root_native_window_matches,
    };
    // Query exactly one focused element, never its ancestors/children or the
    // desktop. A foreign PID exposes only a refusal marker; no names, values,
    // runtime IDs or other application properties are queried or serialized.
    let focused = match b.native(|| Ok(unsafe { automation.GetFocusedElement() }))? {
        Ok(element) => {
            let pid = b.call(|| unsafe { element.CurrentProcessId() })?;
            let (retained, status, hresult) = focus_candidate(pid, target.process_id, || element);
            focus.status = status;
            focus.hresult = hresult;
            retained
        }
        Err(error) => {
            focus.provider_error(error.code().0 as u32);
            None
        }
    };
    // SAFETY: owned automation object on the same MTA, read-only tree walker.
    let walker = b.native(|| unsafe { automation.RawViewWalker() })?;
    let mut observation = Observation {
        nodes: Vec::new(),
        traversal_errors: Vec::new(),
        termination: "complete",
        selected_pid_only: true,
        max_nodes,
        max_depth: MAX_DEPTH,
        max_text_utf8_bytes: TEXT_BYTES,
        cooperative_provider_budget_ms: 3000,
        tree_is_atomic: false,
        focused_element: focus,
    };
    let mut walk = Walk {
        walker: &walker,
        target,
        budget: &b,
        observation: &mut observation,
        node_bytes: 0,
        automation: &automation,
        focused: focused.as_ref(),
    };
    let result = walk.visit(&root, None, 0);
    match result {
        Ok(()) => {
            if !observation.traversal_errors.is_empty() {
                observation.termination = "provider_errors";
            }
        }
        Err(Error::Timeout) => observation.termination = "cooperative_deadline",
        Err(Error::Limit | Error::TargetChanged) => {}
        Err(e) => return Err(e),
    }
    // Explicit release order keeps every provider/interface release within this
    // helper's lifetime and the parent's deadline/actual-retirement ownership.
    drop(walker);
    drop(root);
    drop(focused);
    drop(automation);
    drop(apartment);
    Ok(observation)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreign_and_unknown_focus_never_enters_selected_property_callback() {
        let called = std::cell::Cell::new(0);
        for pid in [0, -1, 8] {
            let (element, status, hresult) = focus_candidate(Observed::from(Ok(pid)), 7, || {
                called.set(called.get() + 1);
                9
            });
            assert_eq!(element, None);
            assert_eq!(status, "foreign_or_unknown_pid_refused");
            assert_eq!(hresult, None);
        }
        assert_eq!(called.get(), 0);
        let missing = Observed::from(Err(windows::core::Error::from_hresult(
            windows::core::HRESULT(0x80040200u32 as i32),
        )));
        let (element, status, hresult) = focus_candidate(missing, 7, || {
            called.set(called.get() + 1);
            9
        });
        assert_eq!(element, None);
        assert_eq!(status, "provider_error");
        assert_eq!(hresult, Some(0x80040200));
        assert_eq!(called.get(), 0);
        let (element, status, _) = focus_candidate(Observed::from(Ok(7)), 7, || {
            called.set(called.get() + 1);
            9
        });
        assert_eq!(element, Some(9));
        assert_eq!(status, "selected_pid_unproved");
        assert_eq!(called.get(), 1);
    }
    #[test]
    fn text_bound_preserves_complete_unicode_and_marks_truncation() {
        let raw = "\u{1f58c}".repeat(200);
        let value = bounded_text(BSTR::from(raw.as_str()));
        assert!(value.truncated);
        assert!(value.value.len() <= TEXT_BYTES);
        assert!(raw.starts_with(&value.value));
        assert!(value.value.chars().all(|c| c == '\u{1f58c}'));
    }
    #[test]
    fn exact_and_empty_text_are_distinct_from_missing_property() {
        let exact = "x".repeat(TEXT_BYTES);
        let value = bounded_text(BSTR::from(exact.as_str()));
        assert_eq!(value.value, exact);
        assert!(!value.truncated);
        let empty = bounded_text(BSTR::new());
        assert_eq!(empty.value, "");
        assert!(!empty.truncated);
        let unavailable: Observed<bool> = Observed::from(Err(windows::core::Error::from_hresult(
            windows::core::HRESULT(0x80040200u32 as i32),
        )));
        assert_eq!(unavailable.value, None);
        assert_eq!(unavailable.hresult, Some(0x80040200));
    }
    #[test]
    fn expired_budget_refuses_a_provider_call_before_it_runs() {
        let mut called = false;
        let b = Budget {
            start: Instant::now() - Duration::from_secs(4),
        };
        let value = b.native(|| {
            called = true;
            Ok(())
        });
        assert_eq!(value, Err(Error::Timeout));
        assert!(!called);
    }
}
