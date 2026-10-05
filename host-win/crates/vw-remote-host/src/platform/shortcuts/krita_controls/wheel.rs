//! Finite measurement of one current anonymous brush-toolbar numeric control.
//! A measured numeric change is not a size/opacity semantic or profile grant.
use super::*;
use crate::platform::finite_input::{FiniteInputOwner, FiniteRetirement};
use observer::NumericProof;
use vw_remote::profile::{
    essential::RangeShape,
    live::{Route, Selector},
};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct State {
    settings: ToolSettings,
    canvas_runtime_hash: String,
    canvas_rect: Rect,
    pub numeric: NumericProof,
    watched: Vec<NumericWitness>,
}
impl State {
    pub(crate) fn digest(&self) -> Result<String> {
        Ok(sha256(
            &serde_json::to_vec(self).map_err(|_| Error::Invalid)?,
        ))
    }
    pub(crate) fn compatible_change(&self, after: &Self) -> Result<()> {
        let mut normalized = after.clone();
        normalized.numeric.current = self.numeric.current;
        normalized
            .numeric
            .value_text
            .clone_from(&self.numeric.value_text);
        let hash = &self.numeric.element.runtime_id_hash;
        let old = self
            .watched
            .iter()
            .filter(|w| &w.element.runtime_id_hash == hash)
            .collect::<Vec<_>>();
        let changed = normalized
            .watched
            .iter_mut()
            .filter(|w| &w.element.runtime_id_hash == hash)
            .collect::<Vec<_>>();
        if old.len() != 1 || changed.len() != 1 {
            return Err(Error::Unavailable);
        }
        for value in changed {
            value.current = old[0].current;
            value.value.clone_from(&old[0].value);
        }
        if normalized != *self {
            return Err(Error::TargetChanged);
        }
        Ok(())
    }
    pub(crate) fn numeric_effect(&self, after: &Self) -> Result<bool> {
        self.compatible_change(after)?;
        let current_changed = self.numeric.current != after.numeric.current;
        let text_changed = self.numeric.value_text != after.numeric.value_text;
        if current_changed != text_changed {
            return Err(Error::Unavailable);
        }
        Ok(current_changed)
    }
}
fn route() -> Route {
    Route {
        anchor: profile::live::brushes_anchor(),
        leaf: Selector {
            class_name: "KisDoubleSliderSpinBox".into(),
            automation_id: String::new(),
            name: String::new(),
            control_type: 50016,
        },
    }
}
pub(crate) fn delta_valid(delta: i32) -> Result<()> {
    if matches!(delta, -120 | 120) {
        Ok(())
    } else {
        Err(Error::Invalid)
    }
}
pub(crate) fn observe(target: &Target, budget: &SourceBudget, shape: &RangeShape) -> Result<State> {
    shape.validate_wheel()?;
    budget.check()?;
    let lookup = Budget::new(budget.remaining()?);
    let observer = Observer::open(target, &lookup)?;
    let settings = observer.settings()?;
    let (canvas, canvas_rect) = observer.canvas()?;
    let canvas_runtime_hash = runtime_hash(&canvas)?;
    let (_, numeric) = observer.numeric_wheel(&route(), shape)?;
    if numeric.read_only
        || numeric.value_text.is_empty()
        || !numeric.element.enabled
        || numeric.element.offscreen
    {
        return Err(Error::Unavailable);
    }
    let watched = observer.brush_numeric_witnesses()?;
    let matches = watched
        .iter()
        .filter(|w| w.element.runtime_id_hash == numeric.element.runtime_id_hash)
        .collect::<Vec<_>>();
    if matches.len() != 1
        || matches[0].element != numeric.element
        || matches[0].current != numeric.current
        || matches[0].value != numeric.value_text
        || matches[0].read_only != numeric.read_only
        || matches[0].minimum != shape.minimum
        || matches[0].maximum != shape.maximum
        || matches[0].small_change != shape.small_change
        || matches[0].large_change != shape.large_change
    {
        return Err(Error::TargetChanged);
    }
    lookup.check()?;
    drop(canvas);
    drop(observer);
    budget.check()?;
    Ok(State {
        settings,
        canvas_runtime_hash,
        canvas_rect,
        numeric,
        watched,
    })
}
#[derive(Serialize)]
pub(crate) struct Batch {
    pub wheel_delta: i32,
    pub before: State,
    pub expected_count: u32,
    pub native_accepted_count: u32,
    pub accepted_qpc_100ns: Option<u64>,
    pub error: Option<Error>,
    pub input_retirement: FiniteRetirement,
    pub local_preflight_input_micros: u64,
    pub production_parent_exchange_proven: bool,
}
impl Batch {
    pub(crate) fn complete(&self) -> Result<()> {
        if self.input_retirement != FiniteRetirement::Complete {
            return Err(Error::RetirementPending);
        }
        if self.native_accepted_count != self.expected_count {
            return Err(Error::PartialInput);
        }
        self.error.clone().map_or(Ok(()), Err)
    }
}
pub(crate) fn inject(
    owner: &mut FiniteInputOwner<SourceLease>,
    target: &Target,
    owner_pid: u32,
    budget: &SourceBudget,
    delta: i32,
    expected: &State,
) -> Result<Batch> {
    delta_valid(delta)?;
    let shape = &expected.numeric.range;
    let start = Instant::now();
    let destination = center(expected.numeric.element.physical_rect)?;
    let normalized = absolute(destination)?;
    let inputs = [
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: normalized.0,
                    dy: normalized.1,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    mouseData: delta as u32,
                    dwFlags: MOUSEEVENTF_WHEEL,
                    ..Default::default()
                },
            },
        },
    ];
    let attempt = owner.send(&inputs, destination, |source| {
        source.verify(target, owner_pid, budget)?;
        if observe(target, budget, shape)? != *expected {
            return Err(Error::TargetChanged);
        }
        source.verify(target, owner_pid, budget)?;
        unchanged(target, owner_pid)?;
        let lookup = Budget::new(budget.remaining()?);
        let observer = Observer::open(target, &lookup)?;
        let (element, numeric) = observer.numeric_wheel(&route(), shape)?;
        if numeric != expected.numeric {
            return Err(Error::TargetChanged);
        }
        observer.hit_exact(&element, destination)?;
        lookup.check()?;
        let finished = Instant::now();
        drop(element);
        drop(observer);
        if finished.elapsed() >= Duration::from_millis(20) {
            return Err(Error::Timeout);
        }
        // Provider calls and COM teardown can outlive a desktop topology change.
        // Recheck the retained normalized INPUT only after both have completed.
        same_absolute(normalized, absolute(destination)?)?;
        budget.check()
    });
    Ok(Batch {
        wheel_delta: delta,
        before: expected.clone(),
        expected_count: attempt.expected_count,
        native_accepted_count: attempt.accepted_count,
        accepted_qpc_100ns: attempt.accepted_qpc_100ns,
        error: attempt.error,
        input_retirement: attempt.retirement,
        local_preflight_input_micros: start.elapsed().as_micros().try_into().unwrap_or(u64::MAX),
        production_parent_exchange_proven: false,
    })
}

fn same_absolute(expected: (i32, i32), current: (i32, i32)) -> Result<()> {
    if expected == current {
        Ok(())
    } else {
        Err(Error::TargetChanged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wheel_rechecks_retained_mapping_after_provider_work() {
        let retained = (32_784, 32_784);
        assert_eq!(same_absolute(retained, retained), Ok(()));
        // The selected window/physical point may stay fixed while width/origin change.
        assert_eq!(
            same_absolute(retained, (16_387, 32_784)),
            Err(Error::TargetChanged)
        );
        assert_eq!(
            same_absolute(retained, (49_164, 32_784)),
            Err(Error::TargetChanged)
        );
    }
    fn fixture() -> State {
        let rect = Rect {
            x: 10,
            y: 20,
            width: 100,
            height: 200,
        };
        let element = ElementProof {
            runtime_id_hash: "a".repeat(64),
            physical_rect: rect,
            enabled: true,
            offscreen: false,
        };
        let shape = RangeShape {
            minimum: 0.01,
            maximum: 1000.0,
            small_change: 1.0,
            large_change: 10.0,
        };
        let mut watched = vec![];
        for hash in ["a", "b"] {
            watched.push(NumericWitness {
                element: ElementProof {
                    runtime_id_hash: hash.repeat(64),
                    ..element.clone()
                },
                current: 40.0,
                minimum: shape.minimum,
                maximum: shape.maximum,
                small_change: shape.small_change,
                large_change: shape.large_change,
                read_only: false,
                value: "40.00".into(),
            });
        }
        State {
            settings: ToolSettings {
                freehand_selected: true,
                selected_tool_id: profile::live::BRUSH_ID.into(),
                eraser_mode: false,
                preset: "fixture".into(),
                blending_mode: "Normal".into(),
                preserve_alpha: false,
            },
            canvas_runtime_hash: "c".repeat(64),
            canvas_rect: rect,
            numeric: NumericProof {
                element,
                range: shape,
                current: 40.0,
                value_text: "40.00".into(),
                read_only: false,
            },
            watched,
        }
    }
    fn changed(s: &State) -> State {
        let mut x = s.clone();
        x.numeric.current = 41.0;
        x.numeric.value_text = "41.00".into();
        x.watched[0].current = 41.0;
        x.watched[0].value = "41.00".into();
        x
    }
    #[test]
    fn single_exact_numeric_change_is_observed_without_role_inference() -> Result<()> {
        let b = fixture();
        assert!(b.numeric_effect(&changed(&b))?);
        assert!(!b.numeric_effect(&b)?);
        Ok(())
    }
    #[test]
    fn other_anonymous_numeric_or_settings_change_refuses_restoration() -> Result<()> {
        let b = fixture();
        let mut a = changed(&b);
        a.watched[1].current += 1.0;
        assert_eq!(b.numeric_effect(&a), Err(Error::TargetChanged));
        let mut a = changed(&b);
        a.settings.preserve_alpha = true;
        assert_eq!(b.numeric_effect(&a), Err(Error::TargetChanged));
        Ok(())
    }
    #[test]
    fn changed_shape_identity_or_canvas_refuses_inverse() {
        let b = fixture();
        let mut a = changed(&b);
        a.numeric.range.maximum += 1.0;
        assert_eq!(b.numeric_effect(&a), Err(Error::TargetChanged));
        let mut a = changed(&b);
        a.numeric.element.runtime_id_hash = "d".repeat(64);
        assert_eq!(b.numeric_effect(&a), Err(Error::TargetChanged));
        let mut a = changed(&b);
        a.canvas_rect.x += 1;
        assert_eq!(b.numeric_effect(&a), Err(Error::TargetChanged));
    }
    #[test]
    fn ambiguous_or_partial_numeric_proofs_do_not_claim_effect() {
        let b = fixture();
        let mut a = b.clone();
        a.numeric.current += 1.0;
        assert_eq!(b.numeric_effect(&a), Err(Error::Unavailable));
        let mut b = fixture();
        b.watched.push(b.watched[0].clone());
        assert_eq!(b.numeric_effect(&b), Err(Error::Unavailable));
    }
    #[test]
    fn wheel_candidates_are_single_detents_only() {
        for d in [-120, 120] {
            assert_eq!(delta_valid(d), Ok(()));
        }
        for d in [0, 1, -1, 240, i32::MIN] {
            assert_eq!(delta_valid(d), Err(Error::Invalid));
        }
    }
    #[test]
    fn observation_digest_binds_numeric_state_and_other_witnesses() -> Result<()> {
        let b = fixture();
        assert_ne!(b.digest()?, changed(&b).digest()?);
        let mut a = b.clone();
        a.watched[1].value = "41".into();
        assert_ne!(b.digest()?, a.digest()?);
        Ok(())
    }
    #[test]
    fn partial_error_or_pending_batch_never_authorizes_inverse() {
        let mut batch = Batch {
            wheel_delta: 120,
            before: fixture(),
            expected_count: 2,
            native_accepted_count: 2,
            accepted_qpc_100ns: Some(1),
            error: None,
            input_retirement: FiniteRetirement::Complete,
            local_preflight_input_micros: 100,
            production_parent_exchange_proven: false,
        };
        assert_eq!(batch.complete(), Ok(()));
        batch.native_accepted_count = 1;
        assert_eq!(batch.complete(), Err(Error::PartialInput));
        batch.native_accepted_count = 2;
        batch.error = Some(Error::Timeout);
        assert_eq!(batch.complete(), Err(Error::Timeout));
        batch.error = None;
        batch.input_retirement = FiniteRetirement::Pending {
            held_count: 0,
            uncertain: true,
            error: Error::Timeout,
        };
        assert_eq!(batch.complete(), Err(Error::RetirementPending));
    }
}
