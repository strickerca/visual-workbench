//! Input-free semantic candidates. Numeric ranges never become action authority.
use super::*;
use windows::Win32::System::Variant::VARIANT;

const MAX_CANDIDATES: i32 = 8;
const MAX_MEMBERSHIP: i32 = 512;

#[derive(Serialize)]
struct Label<T> {
    status: &'static str,
    hresult: Option<u32>,
    selected_subtree_matches: usize,
    facts: Option<T>,
}
fn prove_label<T>(
    pid: Observed<i32>,
    selected_pid: u32,
    membership: impl FnOnce() -> Result<usize>,
    facts: impl FnOnce() -> Result<T>,
) -> Result<Label<T>> {
    let (same_pid, status, hresult) = focus_candidate(pid, selected_pid, || ());
    if same_pid.is_none() {
        return Ok(Label {
            status,
            hresult,
            selected_subtree_matches: 0,
            facts: None,
        });
    }
    let matches = membership()?;
    if matches != 1 {
        return Ok(Label {
            status: "outside_or_ambiguous_selected_subtree",
            hresult: None,
            selected_subtree_matches: matches,
            facts: None,
        });
    }
    Ok(Label {
        status: "selected_subtree_match",
        hresult: None,
        selected_subtree_matches: matches,
        facts: Some(facts()?),
    })
}
fn bounded_count(count: i32, limit: i32) -> Result<i32> {
    if (0..=limit).contains(&count) {
        Ok(count)
    } else {
        Err(Error::Limit)
    }
}
fn label_element(
    element: &IUIAutomationElement,
) -> windows::core::Result<Option<IUIAutomationElement>> {
    // SAFETY: exact pinned COM ABI; initialized null output and sole ownership
    // of a successful non-null result. Preserve null (no label) vs HRESULT error.
    unsafe {
        let mut raw = ptr::null_mut();
        (Interface::vtable(element).CurrentLabeledBy)(Interface::as_raw(element), &mut raw).ok()?;
        Ok(if raw.is_null() {
            None
        } else {
            Some(IUIAutomationElement::from_raw(raw))
        })
    }
}
struct Query<'a> {
    automation: &'a IUIAutomation,
    root: &'a IUIAutomationElement,
    target: &'a Target,
    budget: &'a Budget,
    members: Option<IUIAutomationElementArray>,
}
impl Query<'_> {
    fn pid(&self, element: &IUIAutomationElement) -> Result<i32> {
        // SAFETY: PID is the first element property; no foreign facts are read.
        let pid = self
            .budget
            .native(|| unsafe { element.CurrentProcessId() })?;
        if pid > 0 && pid as u32 == self.target.process_id {
            Ok(pid)
        } else {
            Err(Error::TargetChanged)
        }
    }
    fn condition(
        &self,
        class: &str,
        id: Option<&str>,
        control_type: Option<i32>,
    ) -> Result<IUIAutomationCondition> {
        let mut properties = vec![
            (
                UIA_ProcessIdPropertyId,
                VARIANT::from(self.target.process_id as i32),
            ),
            (UIA_FrameworkIdPropertyId, VARIANT::from("Qt")),
            (UIA_ClassNamePropertyId, VARIANT::from(class)),
        ];
        if let Some(id) = id {
            properties.push((UIA_AutomationIdPropertyId, VARIANT::from(id)));
        }
        if let Some(kind) = control_type {
            properties.push((UIA_ControlTypePropertyId, VARIANT::from(kind)));
        }
        // SAFETY: UIA copies owned initialized VARIANTs; every search is limited
        // to the exact selected native root or its proved direct-child anchor.
        unsafe {
            let mut condition = self.budget.native(|| {
                self.automation
                    .CreatePropertyCondition(properties[0].0, &properties[0].1)
            })?;
            for (property, value) in &properties[1..] {
                let next = self
                    .budget
                    .native(|| self.automation.CreatePropertyCondition(*property, value))?;
                condition = self
                    .budget
                    .native(|| self.automation.CreateAndCondition(&condition, &next))?;
            }
            Ok(condition)
        }
    }
    fn find(
        &self,
        parent: &IUIAutomationElement,
        scope: TreeScope,
        class: &str,
        id: Option<&str>,
        kind: Option<i32>,
        limit: i32,
    ) -> Result<(IUIAutomationElementArray, i32)> {
        self.pid(parent)?;
        let condition = self.condition(class, id, kind)?;
        // SAFETY: selected-root/anchor scoped query only; blocked allocation and
        // calls remain contained by the hard-deadline Job and retained owner.
        let array = self
            .budget
            .native(|| unsafe { parent.FindAll(scope, &condition) })?;
        let count = bounded_count(self.budget.native(|| unsafe { array.Length() })?, limit)?;
        Ok((array, count))
    }
    fn anchor(&self, class: &str, id: &str, kind: i32) -> Result<IUIAutomationElement> {
        let (found, count) = self.find(
            self.root,
            TreeScope_Children,
            class,
            Some(id),
            Some(kind),
            8,
        )?;
        if count != 1 {
            return Err(Error::Unavailable);
        }
        // SAFETY: exactly one proved direct-child match, no ordinal selection
        // among candidates. This index obtains the sole element only.
        let element = self.budget.native(|| unsafe { found.GetElement(0) })?;
        self.pid(&element)?;
        Ok(element)
    }
    fn membership(&mut self, label: &IUIAutomationElement) -> Result<usize> {
        if self.members.is_none() {
            // SAFETY: selected-PID condition and selected HWND root only. Never
            // GetRootElement, ancestor walking or another application's tree.
            let condition = self.budget.native(|| unsafe {
                self.automation.CreatePropertyCondition(
                    UIA_ProcessIdPropertyId,
                    &VARIANT::from(self.target.process_id as i32),
                )
            })?;
            let found = self
                .budget
                .native(|| unsafe { self.root.FindAll(TreeScope_Subtree, &condition) })?;
            bounded_count(
                self.budget.native(|| unsafe { found.Length() })?,
                MAX_MEMBERSHIP,
            )?;
            self.members = Some(found);
        }
        let members = self.members.as_ref().ok_or(Error::Invalid)?;
        let count = bounded_count(
            self.budget.native(|| unsafe { members.Length() })?,
            MAX_MEMBERSHIP,
        )?;
        let mut matches = 0usize;
        for at in 0..count {
            // SAFETY: every compared object came from the exact selected subtree;
            // no candidate facts are queried during membership comparison.
            let element = self.budget.native(|| unsafe { members.GetElement(at) })?;
            if self
                .budget
                .native(|| unsafe { self.automation.CompareElements(&element, label) })?
                .as_bool()
            {
                matches += 1;
            }
        }
        Ok(matches)
    }
    fn label(&mut self, candidate: &IUIAutomationElement) -> Result<Label<Node>> {
        let label = match self.budget.call(|| label_element(candidate))? {
            Observed {
                value: Some(Some(label)),
                hresult: None,
            } => label,
            Observed {
                value: Some(None),
                hresult: None,
            } => {
                return Ok(Label {
                    status: "no_label",
                    hresult: None,
                    selected_subtree_matches: 0,
                    facts: None,
                });
            }
            Observed { hresult, .. } => {
                return Ok(Label {
                    status: "provider_error",
                    hresult,
                    selected_subtree_matches: 0,
                    facts: None,
                });
            }
        };
        // SAFETY: PID is queried before label names/values/runtime/bounds. Facts
        // additionally require one actual selected-root membership match.
        let pid = self.budget.call(|| unsafe { label.CurrentProcessId() })?;
        let target = self.target;
        let budget = self.budget;
        prove_label(
            pid,
            target.process_id,
            || self.membership(&label),
            || node(&label, target.process_id as i32, None, 0, 0, budget),
        )
    }
    fn group(&mut self, anchor: &IUIAutomationElement, class: &'static str) -> CandidateSet {
        let mut set = CandidateSet {
            candidate_class: class,
            matched_count: None,
            records: Vec::new(),
            status: "complete",
            error: None,
        };
        let result = (|| {
            let (found, count) = self.find(
                anchor,
                TreeScope_Descendants,
                class,
                None,
                None,
                MAX_CANDIDATES,
            )?;
            set.matched_count = Some(count);
            for at in 0..count {
                // SAFETY: return every bounded candidate; array index is bookkeeping
                // only. Runtime/anchor facts identify each record; no action selects it.
                let element = self.budget.native(|| unsafe { found.GetElement(at) })?;
                let pid = self.pid(&element)?;
                let facts = node(&element, pid, None, 0, at as usize, self.budget)?;
                let label = self.label(&element)?;
                set.records.push(Candidate { facts, label });
            }
            Ok(())
        })();
        if let Err(error) = result {
            set.status = "incomplete_or_provider_error";
            set.error = Some(error);
        }
        set
    }
}
#[derive(Serialize)]
struct Candidate {
    facts: Node,
    label: Label<Node>,
}
#[derive(Serialize)]
struct CandidateSet {
    candidate_class: &'static str,
    matched_count: Option<i32>,
    records: Vec<Candidate>,
    status: &'static str,
    error: Option<Error>,
}
#[derive(Serialize)]
struct Group {
    anchor_class: &'static str,
    anchor: Option<Node>,
    candidates: Vec<CandidateSet>,
    status: &'static str,
    error: Option<Error>,
}
#[derive(Serialize)]
struct Controls {
    groups: Vec<Group>,
    expected_groups: usize,
    termination: &'static str,
    selected_pid_only: bool,
    tree_is_atomic: bool,
    candidate_indices_authority: bool,
    numeric_semantics_proved: bool,
    profile_authority: bool,
    label_membership_max: i32,
    candidates_per_class_max: i32,
    cooperative_provider_budget_ms: u32,
}

pub(super) fn observe(target: &Target) -> Result<serde_json::Value> {
    let budget = Budget {
        start: Instant::now(),
    };
    // SAFETY: dedicated query-only MTA. Apartment owner is created before the
    // post-call check, and every COM interface drops before paired uninitialization.
    unsafe {
        budget.check()?;
        platform::api(
            "controls MTA",
            CoInitializeEx(None, COINIT_MULTITHREADED).ok(),
        )?;
    }
    let _apartment = Apartment;
    budget.check()?;
    // SAFETY: same-thread owned COM objects; exact selected HWND only.
    let automation: IUIAutomation = budget
        .native(|| unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) })?;
    let root = budget.native(|| unsafe {
        automation.ElementFromHandle(HWND(target.window as usize as *mut std::ffi::c_void))
    })?;
    let mut query = Query {
        automation: &automation,
        root: &root,
        target,
        budget: &budget,
        members: None,
    };
    query.pid(&root)?;
    // SAFETY: selected root PID precedes its native HWND/class checks.
    if budget
        .native(|| unsafe { root.CurrentNativeWindowHandle() })?
        .0 as usize as u64
        != target.window
        || bounded_text(budget.native(|| unsafe { root.CurrentClassName() })?).value
            != "KisMainWindow"
    {
        return Err(Error::TargetChanged);
    }
    let mut controls = Controls {
        groups: Vec::new(),
        expected_groups: 2,
        termination: "complete",
        selected_pid_only: true,
        tree_is_atomic: false,
        candidate_indices_authority: false,
        numeric_semantics_proved: false,
        profile_authority: false,
        label_membership_max: MAX_MEMBERSHIP,
        candidates_per_class_max: MAX_CANDIDATES,
        cooperative_provider_budget_ms: 3000,
    };
    for (class, id, kind, candidates) in [
        (
            "KisToolBar",
            "MainWindow#1.BrushesAndStuff",
            50021,
            &["KisDoubleSliderSpinBox"][..],
        ),
        (
            "QStatusBar",
            "",
            50017,
            &["KoZoomWidget", "KoZoomInput", "QSlider"][..],
        ),
    ] {
        let mut group = Group {
            anchor_class: class,
            anchor: None,
            candidates: Vec::new(),
            status: "complete",
            error: None,
        };
        let result = (|| {
            let anchor = query.anchor(class, id, kind)?;
            group.anchor = Some(node(&anchor, query.pid(&anchor)?, None, 0, 0, &budget)?);
            for candidate in candidates {
                let set = query.group(&anchor, candidate);
                if set.error.is_some() {
                    group.status = "incomplete_or_provider_error";
                }
                group.candidates.push(set);
            }
            Ok(())
        })();
        if let Err(error) = result {
            group.status = "incomplete_or_provider_error";
            group.error = Some(error);
        }
        if group.status != "complete" {
            controls.termination = "incomplete_or_provider_error";
        }
        controls.groups.push(group);
        if budget.check().is_err() {
            controls.termination = "cooperative_deadline";
            break;
        }
    }
    serde_json::to_value(controls).map_err(|_| Error::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreign_label_never_queries_membership_or_facts() -> Result<()> {
        let calls = std::cell::Cell::new(0);
        for pid in [-1, 0, 8] {
            let record = prove_label(
                Observed::from(Ok(pid)),
                7,
                || {
                    calls.set(calls.get() + 1);
                    Ok(1)
                },
                || {
                    calls.set(calls.get() + 1);
                    Ok(9)
                },
            )?;
            assert_eq!(record.status, "foreign_or_unknown_pid_refused");
            assert_eq!(record.facts, None);
        }
        assert_eq!(calls.get(), 0);
        Ok(())
    }
    #[test]
    fn same_pid_label_requires_one_rooted_match_before_facts() -> Result<()> {
        let calls = std::cell::Cell::new(0);
        for matches in [0, 2] {
            let record = prove_label(
                Observed::from(Ok(7)),
                7,
                || Ok(matches),
                || {
                    calls.set(calls.get() + 1);
                    Ok(9)
                },
            )?;
            assert_eq!(record.facts, None);
        }
        let record = prove_label(
            Observed::from(Ok(7)),
            7,
            || Ok(1),
            || {
                calls.set(calls.get() + 1);
                Ok(9)
            },
        )?;
        assert_eq!(record.facts, Some(9));
        assert_eq!(calls.get(), 1);
        Ok(())
    }
    #[test]
    fn candidate_and_membership_bounds_refuse_overflow_without_ordinal_choice() -> Result<()> {
        assert_eq!(bounded_count(-1, MAX_CANDIDATES), Err(Error::Limit));
        assert_eq!(bounded_count(9, MAX_CANDIDATES), Err(Error::Limit));
        assert_eq!(bounded_count(513, MAX_MEMBERSHIP), Err(Error::Limit));
        assert_eq!(bounded_count(0, MAX_CANDIDATES)?, 0);
        Ok(())
    }
}
