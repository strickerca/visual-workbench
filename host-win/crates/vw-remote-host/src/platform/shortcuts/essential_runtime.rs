//! Actual selected-source control consumer. Catalog bytes alone cannot satisfy
//! current tool/settings/canvas/destination proof. The caller owns finite input,
//! process/Job/IO retirement and the unchanged parent exchange deadline.
use super::{
    absolute, catalog, center, click, clock_100ns, observer, paint_controls, runtime_hash, sha256,
};
use crate::{
    Error, Result,
    editor_source::{SourceBudget, SourceLease},
};
use observer::{Budget, ElementProof, NumericProof, NumericWitness, Observer};
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use vw_remote::{
    Target,
    profile::{
        self, Authority, CanvasProof, PackagedProfile,
        catalog::Entry,
        essential::{
            ControlRoute, Editor, EssentialAction, EssentialProfile, NumericKey, RangeShape,
        },
        live::{self, ToolSettings},
    },
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

struct Installed {
    profile: EssentialProfile,
    source: SourceLease,
    digest: String,
    cancel: Arc<vw_capture::Cancellation>,
}
static INSTALLED: OnceLock<Installed> = OnceLock::new();
impl Installed {
    fn budget(&self, deadline: Instant) -> Result<SourceBudget> {
        SourceBudget::until(self.cancel.clone(), deadline)
    }
    fn verify(&self, target: &Target, owner: u32, budget: &SourceBudget) -> Result<()> {
        self.source.verify(target, owner, budget)?;
        if self.source.identity() != &self.profile.metadata.identity() {
            return Err(Error::TargetChanged);
        }
        budget.check()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct KritaProof {
    settings: ToolSettings,
    canvas: CanvasProof,
    element: ElementProof,
    numeric: Option<NumericProof>,
    watched: Vec<NumericWitness>,
    watched_digest: String,
}
#[derive(Clone, Debug, PartialEq)]
enum Proof {
    Krita(Box<KritaProof>),
    Paint(Box<paint_controls::production::Proof>),
}
fn same_canvas(a: &CanvasProof, b: &CanvasProof) -> bool {
    a.runtime_id_hash == b.runtime_id_hash
        && a.process_id == b.process_id
        && a.class_name == b.class_name
        && a.control_type == b.control_type
        && a.canvas_rect == b.canvas_rect
        && a.profile_digest == b.profile_digest
}
impl Proof {
    fn canvas(&self, target: &Target, digest: &str) -> Result<CanvasProof> {
        match self {
            Self::Krita(v) => Ok(v.canvas.clone()),
            Self::Paint(v) => v.canvas_proof(target, digest),
        }
    }
    fn destination(&self) -> Result<(i32, i32)> {
        match self {
            Self::Krita(v) => center(v.element.physical_rect),
            Self::Paint(v) => v.destination_point(),
        }
    }
    fn stable_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Krita(a), Self::Krita(b)) => {
                a.settings == b.settings
                    && same_canvas(&a.canvas, &b.canvas)
                    && a.element == b.element
                    && a.numeric == b.numeric
                    && a.watched == b.watched
                    && a.watched_digest == b.watched_digest
            }
            (Self::Paint(a), Self::Paint(b)) => a == b,
            _ => false,
        }
    }
}

/// The calibrated size shape is used only to exclude its intentional current
/// value from the preset watcher. Every shape and the other control's actual
/// value stay watched. The signed packaged semantic receipt supplies the role;
/// a range, position or enumeration ordinal never supplies size meaning.
fn size_shape(profile: &EssentialProfile) -> Result<Option<RangeShape>> {
    let mut result: Option<RangeShape> = None;
    for action in profile
        .actions
        .iter()
        .filter(|action| matches!(action.action, 6 | 7))
    {
        let shape = match &action.route {
            ControlRoute::FocusedNumeric { numeric } => &numeric.range,
            ControlRoute::ElementWheel { numeric } => &numeric.range,
            _ => continue,
        };
        if result.as_ref().is_some_and(|old| old != shape) {
            return Err(Error::Invalid);
        }
        result = Some(shape.clone());
    }
    Ok(result)
}
fn witness_shape(value: &NumericWitness) -> RangeShape {
    RangeShape {
        minimum: value.minimum,
        maximum: value.maximum,
        small_change: value.small_change,
        large_change: value.large_change,
    }
}
#[derive(serde::Serialize)]
struct PortableNumeric {
    shape: RangeShape,
    read_only: bool,
    current: Option<f64>,
    value: Option<String>,
}
fn watched_digest(
    settings: &ToolSettings,
    watched: &[NumericWitness],
    mutable_size: Option<&RangeShape>,
) -> Result<String> {
    if watched.len() != 2 {
        return Err(Error::Unavailable);
    }
    let mut mutable_matches = 0usize;
    let mut values = Vec::new();
    for value in watched {
        value.validate()?;
        let shape = witness_shape(value);
        shape.validate()?;
        let mutable = mutable_size == Some(&shape);
        mutable_matches += usize::from(mutable);
        values.push(PortableNumeric {
            shape,
            read_only: value.read_only,
            current: (!mutable).then_some(value.current),
            value: (!mutable).then(|| value.value.clone()),
        });
    }
    if mutable_size.is_some() && mutable_matches != 1 {
        return Err(Error::Unavailable);
    }
    values.sort_by(|a, b| {
        a.shape
            .minimum
            .total_cmp(&b.shape.minimum)
            .then(a.shape.maximum.total_cmp(&b.shape.maximum))
            .then(a.shape.small_change.total_cmp(&b.shape.small_change))
            .then(a.shape.large_change.total_cmp(&b.shape.large_change))
    });
    if values[0].shape == values[1].shape {
        return Err(Error::Unavailable);
    }
    let bytes = serde_json::to_vec(&(
        "M4-Krita-Essential-Watched-v1",
        settings.settings_bytes(),
        values,
    ))
    .map_err(|_| Error::Invalid)?;
    Ok(sha256(&bytes))
}
fn known_tool(settings: &ToolSettings) -> Result<()> {
    if !matches!(
        settings.selected_tool_id.as_str(),
        live::BRUSH_ID | live::RECTANGLE_ID
    ) || settings.freehand_selected != (settings.selected_tool_id == live::BRUSH_ID)
        || settings.preset.is_empty()
        || settings.blending_mode.is_empty()
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn permit(action: u32, settings: &ToolSettings) -> Result<()> {
    known_tool(settings)?;
    if action == 4 && !settings.freehand_selected {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn krita_base(
    p: &Installed,
    target: &Target,
    observer: &Observer<'_>,
    require_focus: bool,
) -> Result<KritaProof> {
    let settings = observer.settings()?;
    known_tool(&settings)?;
    let watched = observer.brush_numeric_witnesses()?;
    let digest = watched_digest(&settings, &watched, size_shape(&p.profile)?.as_ref())?;
    if digest != p.profile.watched_settings_digest {
        return Err(Error::Unavailable);
    }
    let (canvas, canvas_rect) = if require_focus {
        observer.focused_canvas()?
    } else {
        observer.canvas()?
    };
    let canvas = CanvasProof {
        runtime_id_hash: runtime_hash(&canvas)?,
        process_id: target.process_id,
        sampled_qpc_100ns: clock_100ns()?,
        class_name: "KisOpenGLCanvas2".into(),
        control_type: 50026,
        canvas_rect,
        profile_digest: p.digest.clone(),
    };
    let element = ElementProof {
        runtime_id_hash: canvas.runtime_id_hash.clone(),
        physical_rect: canvas_rect,
        enabled: true,
        offscreen: false,
    };
    Ok(KritaProof {
        settings,
        canvas,
        element,
        numeric: None,
        watched,
        watched_digest: digest,
    })
}
fn krita_destination(
    observer: &Observer<'_>,
    action: &EssentialAction,
    proof: &mut KritaProof,
) -> Result<()> {
    permit(action.action, &proof.settings)?;
    let (element, numeric) = match &action.route {
        ControlRoute::Toolbar { route } => (observer.proof(&observer.element(route)?)?, None),
        ControlRoute::FocusedNumeric { numeric } => {
            let (_, value) = observer.numeric(&numeric.route, &numeric.range)?;
            if value.read_only || value.value_text.is_empty() {
                return Err(Error::Unavailable);
            }
            (value.element.clone(), Some(value))
        }
        ControlRoute::ElementWheel { numeric } => {
            let (_, value) = observer.numeric_wheel(&numeric.route, &numeric.range)?;
            if value.read_only || value.value_text.is_empty() {
                return Err(Error::Unavailable);
            }
            (value.element.clone(), Some(value))
        }
        _ => return Err(Error::Unavailable),
    };
    proof.element = element;
    proof.numeric = numeric;
    Ok(())
}
fn read(
    p: &Installed,
    target: &Target,
    owner: u32,
    action: &EssentialAction,
    budget: &SourceBudget,
) -> Result<Proof> {
    p.verify(target, owner, budget)?;
    let proof = match p.profile.editor {
        Editor::Paint => Proof::Paint(Box::new(paint_controls::production::observe_action(
            target,
            budget,
            action,
            &p.profile.watched_settings_digest,
            false,
        )?)),
        Editor::Krita => {
            let observer_budget = Budget::new(budget.remaining()?);
            let observer = Observer::open(target, &observer_budget)?;
            let mut proof = krita_base(p, target, &observer, false)?;
            krita_destination(&observer, action, &mut proof)?;
            observer_budget.check()?;
            drop(observer);
            budget.check()?;
            Proof::Krita(Box::new(proof))
        }
    };
    p.verify(target, owner, budget)?;
    Ok(proof)
}
fn authority(
    p: &Installed,
    target: &Target,
    owner: u32,
    budget: &SourceBudget,
    require_focus: bool,
) -> Result<Authority> {
    p.verify(target, owner, budget)?;
    let (proof, verified_actions) = match p.profile.editor {
        Editor::Paint => {
            let (proof, actions) = paint_controls::production::observe_authority(
                target,
                budget,
                &p.profile.actions,
                &p.profile.watched_settings_digest,
                require_focus,
            )?;
            (Proof::Paint(Box::new(proof)), actions)
        }
        Editor::Krita => {
            let observer_budget = Budget::new(budget.remaining()?);
            let observer = Observer::open(target, &observer_budget)?;
            let proof = krita_base(p, target, &observer, require_focus)?;
            let mut actions = Vec::new();
            for action in &p.profile.actions {
                let mut current = proof.clone();
                match krita_destination(&observer, action, &mut current) {
                    Ok(()) => actions.push(action.action),
                    Err(Error::Unavailable) => {}
                    Err(error) => return Err(error),
                }
            }
            observer_budget.check()?;
            drop(observer);
            budget.check()?;
            (Proof::Krita(Box::new(proof)), actions)
        }
    };
    p.verify(target, owner, budget)?;
    let sampled = budget.call(clock_100ns)?;
    let mut focus = proof.canvas(target, &p.digest)?;
    focus.sampled_qpc_100ns = sampled;
    budget.check()?;
    Ok(Authority {
        identity: p.profile.metadata.identity(),
        profile_digest: p.digest.clone(),
        verified_actions,
        focus,
    })
}
pub(super) fn install(
    target: &Target,
    owner: u32,
    packaged: &PackagedProfile,
) -> Result<Option<Authority>> {
    let cancel = Arc::new(vw_capture::Cancellation::default());
    let budget = SourceBudget::new(
        cancel.clone(),
        Duration::from_millis(observer::PRODUCTION_BUDGET_MS),
    )?;
    let Some(selected) = catalog::select(target, owner, packaged, &budget)? else {
        return Ok(None);
    };
    let Entry::Essential { profile } = selected.entry;
    let installed = Installed {
        profile,
        source: selected.source,
        digest: selected.digest,
        cancel,
    };
    let result = match authority(&installed, target, owner, &budget, true) {
        Ok(result) => result,
        Err(Error::Unavailable | Error::Ungranted) => return Ok(None),
        Err(error) => return Err(error),
    };
    INSTALLED.set(installed).map_err(|_| Error::Invalid)?;
    Ok(Some(result))
}
pub(super) fn installed() -> bool {
    INSTALLED.get().is_some()
}
/// Caller gates contact/known-held state and the full authenticated binding.
/// Refresh cannot block an accepted Up ACK or adopt a new source/grant.
pub(super) fn refresh(target: &Target, owner: u32, digest: &str) -> Result<Authority> {
    let p = INSTALLED.get().ok_or(Error::Unavailable)?;
    if p.digest != digest {
        return Err(Error::Ungranted);
    }
    let started = Instant::now();
    // The current DTO names this a focus proof. A toolbar-owned keyboard focus
    // is a typed refusal, never a selected-canvas descriptor mislabeled as focus.
    authority(p, target, owner, &p.budget(deadline(started)?)?, true)
}

/// Raw inputs and stage association are immutable outside this source module.
pub struct Stage {
    inputs: Vec<INPUT>,
    destination: (i32, i32),
    absolute_destination: Option<(i32, i32)>,
    second: bool,
    token: Arc<()>,
}
impl Stage {
    pub fn inputs(&self) -> &[INPUT] {
        &self.inputs
    }
    pub fn destination(&self) -> (i32, i32) {
        self.destination
    }
}
pub struct Command {
    proof: Proof,
    action: EssentialAction,
    started: Instant,
    digest: String,
    runtime: String,
    token: Arc<()>,
    first: Stage,
}
impl Command {
    pub fn first_stage(&self) -> &Stage {
        &self.first
    }
}
fn paired(key: NumericKey) -> Vec<INPUT> {
    let input = |up| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(key.virtual_key()),
                wScan: 0,
                dwFlags: KEYEVENTF_EXTENDEDKEY
                    | if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    vec![input(false), input(true)]
}
fn wheel(point: (i32, i32), delta: i32) -> Result<Vec<INPUT>> {
    if !matches!(delta, -120 | 120) {
        return Err(Error::Invalid);
    }
    // The caller computed and retained these exact normalized coordinates.
    Ok(vec![
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: point.0,
                    dy: point.1,
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
    ])
}
fn right_click(point: (i32, i32)) -> Vec<INPUT> {
    let button = |flag| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flag,
                ..Default::default()
            },
        },
    };
    vec![
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: point.0,
                    dy: point.1,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    ..Default::default()
                },
            },
        },
        button(MOUSEEVENTF_RIGHTDOWN),
        button(MOUSEEVENTF_RIGHTUP),
    ]
}
fn deadline(started: Instant) -> Result<Instant> {
    started
        .checked_add(Duration::from_millis(observer::PRODUCTION_BUDGET_MS))
        .ok_or(Error::Limit)
}
// The final relookup must prove actual numeric value/text, range, mutability,
// runtime and bounds, not merely return another element matching the selector.
fn same_numeric(expected: Option<&NumericProof>, current: &NumericProof) -> Result<()> {
    if expected == Some(current) {
        Ok(())
    } else {
        Err(Error::TargetChanged)
    }
}
fn stage_matches(command: &Command, stage: &Stage) -> Result<()> {
    same_owner(&command.token, &stage.token)
}
fn same_owner(expected: &Arc<()>, actual: &Arc<()>) -> Result<()> {
    if Arc::ptr_eq(expected, actual) {
        Ok(())
    } else {
        Err(Error::Ungranted)
    }
}
pub(super) fn prepare(
    target: &Target,
    owner: u32,
    digest: &str,
    action: u32,
    runtime: &str,
) -> Result<Command> {
    let started = Instant::now();
    let p = INSTALLED.get().ok_or(Error::Unavailable)?;
    if p.digest != digest || !profile::digest(runtime) {
        return Err(Error::Ungranted);
    }
    let budget = p.budget(deadline(started)?)?;
    let action = p.profile.action(action)?.clone();
    let proof = read(p, target, owner, &action, &budget)?;
    if proof.canvas(target, digest)?.runtime_id_hash != runtime {
        return Err(Error::Ungranted);
    }
    let destination = proof.destination()?;
    let absolute_destination = absolute(destination)?;
    let inputs = match &action.route {
        ControlRoute::ElementWheel { numeric } => wheel(absolute_destination, numeric.wheel_delta)?,
        // Right-click is an explicit measured entry route, not a claim that a
        // center left-click harmlessly focuses Qt. Exact final focus still gates
        // stage two; only admitted causal/restoration receipts enable this route.
        ControlRoute::FocusedNumeric { .. } => right_click(absolute_destination),
        _ => click(absolute_destination),
    };
    budget.check()?;
    let token = Arc::new(());
    Ok(Command {
        proof,
        action,
        started,
        digest: digest.into(),
        runtime: runtime.into(),
        token: token.clone(),
        first: Stage {
            inputs,
            destination,
            absolute_destination: Some(absolute_destination),
            second: false,
            token,
        },
    })
}
pub(super) fn second(target: &Target, owner: u32, command: &Command) -> Result<Option<Stage>> {
    let key = match &command.action.route {
        ControlRoute::FocusedNumeric { numeric } => numeric.key,
        ControlRoute::PaintSize { numeric } => numeric.key,
        _ => return Ok(None),
    };
    let p = INSTALLED.get().ok_or(Error::Unavailable)?;
    if p.digest != command.digest {
        return Err(Error::Ungranted);
    }
    let budget = p.budget(deadline(command.started)?)?;
    let proof = read(p, target, owner, &command.action, &budget)?;
    if !command.proof.stable_eq(&proof) {
        return Err(Error::TargetChanged);
    }
    budget.check()?;
    Ok(Some(Stage {
        inputs: paired(key),
        destination: proof.destination()?,
        absolute_destination: None,
        second: true,
        token: command.token.clone(),
    }))
}
pub(super) fn validate(
    target: &Target,
    owner: u32,
    command: &Command,
    stage: &Stage,
) -> Result<()> {
    stage_matches(command, stage)?;
    let p = INSTALLED.get().ok_or(Error::Unavailable)?;
    if p.digest != command.digest {
        return Err(Error::Ungranted);
    }
    let budget = p.budget(deadline(command.started)?)?;
    let current = read(p, target, owner, &command.action, &budget)?;
    if !command.proof.stable_eq(&current)
        || current.destination()? != stage.destination
        || current.canvas(target, &p.digest)?.runtime_id_hash != command.runtime
    {
        return Err(Error::TargetChanged);
    }
    p.verify(target, owner, &budget)?;
    // Every potentially blocking source/package operation precedes this final
    // provider proof. After it, only bounded COM release, clock/deadline checks
    // and the common finite owner's cheap exact native destination guard run.
    match &current {
        Proof::Paint(value) => paint_controls::production::validate_final(
            target,
            &budget,
            &command.action,
            value,
            stage.second,
        )?,
        Proof::Krita(expected) => {
            let observer_budget = Budget::new(budget.remaining()?);
            let observer = Observer::open(target, &observer_budget)?;
            let mut value = krita_base(p, target, &observer, false)?;
            krita_destination(&observer, &command.action, &mut value)?;
            if !Proof::Krita(Box::new(value)).stable_eq(&current) {
                return Err(Error::TargetChanged);
            }
            let element = match &command.action.route {
                ControlRoute::Toolbar { route } => observer.element(route)?,
                ControlRoute::FocusedNumeric { numeric } => {
                    let (element, last) = observer.numeric(&numeric.route, &numeric.range)?;
                    same_numeric(expected.numeric.as_ref(), &last)?;
                    element
                }
                ControlRoute::ElementWheel { numeric } => {
                    let (element, last) = observer.numeric_wheel(&numeric.route, &numeric.range)?;
                    same_numeric(expected.numeric.as_ref(), &last)?;
                    element
                }
                _ => return Err(Error::Invalid),
            };
            if observer.proof(&element)? != expected.element {
                return Err(Error::TargetChanged);
            }
            if stage.second {
                observer.focused_numeric(&element)?;
            } else {
                observer.hit_exact(&element, stage.destination)?;
            }
            observer_budget.check()?;
            let finished = Instant::now();
            drop(element);
            drop(observer);
            if finished.elapsed() >= Duration::from_millis(20) {
                return Err(Error::Timeout);
            }
        }
    }
    // Topology may change without moving this selected HWND. Revalidate the
    // exact normalized mouse mapping after all provider work; otherwise the
    // retained INPUT can land elsewhere despite a correct physical-point guard.
    if let Some(expected) = stage.absolute_destination {
        same_absolute(expected, absolute(stage.destination)?)?;
    }
    budget.check()
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
    fn same_window_point_cannot_reuse_mapping_after_virtual_desktop_change() {
        let original = (32_784, 32_784);
        assert_eq!(same_absolute(original, original), Ok(()));
        // Same physical target point after doubling the virtual desktop width.
        assert_eq!(
            same_absolute(original, (16_387, 32_784)),
            Err(Error::TargetChanged)
        );
        // A changed virtual origin also changes normalization with fixed HWND.
        assert_eq!(
            same_absolute(original, (49_164, 32_784)),
            Err(Error::TargetChanged)
        );
    }
    fn numeric() -> NumericProof {
        NumericProof {
            element: witnesses().remove(0).element,
            range: RangeShape {
                minimum: 0.0,
                maximum: 100.0,
                small_change: 1.0,
                large_change: 10.0,
            },
            current: 40.0,
            value_text: "40".into(),
            read_only: false,
        }
    }
    #[test]
    fn final_numeric_relookup_refuses_value_text_range_and_readonly_races() {
        let expected = numeric();
        assert_eq!(same_numeric(Some(&expected), &expected), Ok(()));
        let mut changed = expected.clone();
        changed.current = 41.0;
        assert_eq!(
            same_numeric(Some(&expected), &changed),
            Err(Error::TargetChanged)
        );
        changed = expected.clone();
        changed.value_text = "41".into();
        assert_eq!(
            same_numeric(Some(&expected), &changed),
            Err(Error::TargetChanged)
        );
        changed = expected.clone();
        changed.range.maximum = 200.0;
        assert_eq!(
            same_numeric(Some(&expected), &changed),
            Err(Error::TargetChanged)
        );
        changed = expected.clone();
        changed.read_only = true;
        assert_eq!(
            same_numeric(Some(&expected), &changed),
            Err(Error::TargetChanged)
        );
    }
    #[test]
    fn final_numeric_relookup_refuses_missing_or_replaced_element_proof() {
        let expected = numeric();
        assert_eq!(same_numeric(None, &expected), Err(Error::TargetChanged));
        let mut changed = expected.clone();
        changed.element.runtime_id_hash = "f".repeat(64);
        assert_eq!(
            same_numeric(Some(&expected), &changed),
            Err(Error::TargetChanged)
        );
        changed = expected.clone();
        changed.element.physical_rect.x += 1;
        assert_eq!(
            same_numeric(Some(&expected), &changed),
            Err(Error::TargetChanged)
        );
    }
    fn settings() -> ToolSettings {
        ToolSettings {
            freehand_selected: true,
            selected_tool_id: live::BRUSH_ID.into(),
            eraser_mode: false,
            preset: "fixture preset".into(),
            blending_mode: "Normal".into(),
            preserve_alpha: false,
        }
    }
    fn witnesses() -> Vec<NumericWitness> {
        [(0.0, 100.0, 100.0), (0.01, 1000.0, 40.0)]
            .into_iter()
            .enumerate()
            .map(|(index, (minimum, maximum, current))| NumericWitness {
                element: ElementProof {
                    runtime_id_hash: format!("{index:064x}"),
                    physical_rect: vw_remote::Rect {
                        x: 10,
                        y: 10,
                        width: 80,
                        height: 20,
                    },
                    enabled: true,
                    offscreen: false,
                },
                current,
                minimum,
                maximum,
                small_change: 1.0,
                large_change: 10.0,
                read_only: false,
                value: current.to_string(),
            })
            .collect()
    }
    #[test]
    fn calibrated_size_mutation_keeps_opacity_and_preset_watched() -> Result<()> {
        let mut values = witnesses();
        let shape = witness_shape(&values[1]);
        let original = watched_digest(&settings(), &values, Some(&shape))?;
        values[1].current = 41.0;
        values[1].value = "41".into();
        assert_eq!(
            original,
            watched_digest(&settings(), &values, Some(&shape))?
        );
        values[0].current = 99.0;
        values[0].value = "99".into();
        assert_ne!(
            original,
            watched_digest(&settings(), &values, Some(&shape))?
        );
        let mut changed = settings();
        changed.preset = "other".into();
        assert_ne!(
            original,
            watched_digest(&changed, &witnesses(), Some(&shape))?
        );
        Ok(())
    }
    #[test]
    fn runtime_order_cannot_select_numeric_semantics() -> Result<()> {
        let mut values = witnesses();
        let shape = witness_shape(&values[1]);
        let original = watched_digest(&settings(), &values, Some(&shape))?;
        values.reverse();
        values[0].element.runtime_id_hash = "f".repeat(64);
        assert_eq!(
            original,
            watched_digest(&settings(), &values, Some(&shape))?
        );
        values[1].minimum = shape.minimum;
        values[1].maximum = shape.maximum;
        assert_eq!(
            watched_digest(&settings(), &values, Some(&shape)),
            Err(Error::Unavailable)
        );
        Ok(())
    }
    #[test]
    fn without_calibrated_size_every_actual_numeric_value_remains_watched() -> Result<()> {
        let mut values = witnesses();
        let original = watched_digest(&settings(), &values, None)?;
        values[1].current += 1.0;
        assert_ne!(original, watched_digest(&settings(), &values, None)?);
        Ok(())
    }
    #[test]
    fn unknown_or_inconsistent_tool_does_not_supply_action_authority() {
        let mut value = settings();
        value.selected_tool_id = "other tool".into();
        assert_eq!(known_tool(&value), Err(Error::Unavailable));
        value.selected_tool_id = live::RECTANGLE_ID.into();
        assert_eq!(known_tool(&value), Err(Error::Unavailable));
        value.freehand_selected = false;
        assert_eq!(permit(3, &value), Ok(()));
        assert_eq!(permit(4, &value), Err(Error::Unavailable));
    }
    #[test]
    fn one_absolute_command_budget_cannot_be_renewed_for_stage_two() -> Result<()> {
        let cancel = Arc::new(vw_capture::Cancellation::default());
        let started = Instant::now() - Duration::from_millis(observer::PRODUCTION_BUDGET_MS + 1);
        assert!(matches!(
            SourceBudget::until(cancel, deadline(started)?),
            Err(Error::Timeout)
        ));
        Ok(())
    }
    #[test]
    fn another_command_stage_cannot_substitute_an_equal_sized_owner_token() {
        let expected = Arc::new(());
        let cloned = expected.clone();
        let foreign = Arc::new(());
        assert_eq!(same_owner(&expected, &cloned), Ok(()));
        assert_eq!(same_owner(&expected, &foreign), Err(Error::Ungranted));
    }
    #[test]
    fn unknown_wheel_step_is_preserved_and_cannot_change_before_execution() {
        let mut expected = numeric();
        expected.range.small_change = 0.0;
        expected.range.large_change = 0.0;
        assert_eq!(same_numeric(Some(&expected), &expected), Ok(()));
        let mut changed = expected.clone();
        changed.range.small_change = 1.0;
        assert_eq!(
            same_numeric(Some(&expected), &changed),
            Err(Error::TargetChanged)
        );
        changed = expected.clone();
        changed.current += 1.0;
        assert_eq!(
            same_numeric(Some(&expected), &changed),
            Err(Error::TargetChanged)
        );
    }
}
