//! Exact packaged profile + selected process + native focused canvas authority.
//! UIA and filesystem providers run only in this isolated, parent-Job-owned helper.
use super::{api, clock_100ns, unchanged};
use crate::{Error, Result};
use std::{
    ptr,
    sync::{Arc, OnceLock},
};
use vw_remote::{
    Rect, Target,
    profile::{
        self, Authority, CanvasProof, PackagedProfile, Profile,
        live::{LiveProfile, ToolSettings},
    },
};
mod catalog;
mod essential_runtime;
pub(crate) mod krita_controls;
mod observer;
pub(crate) mod paint_controls;
use observer::{Budget, ElementProof, Observer};
use windows::Win32::{
    System::{Com::*, Ole::*},
    UI::{Accessibility::*, Input::KeyboardAndMouse::*},
};
struct Installed {
    profile: Profile,
    live: Option<LiveProfile>,
    digest: String,
    // Retained target process and ancestor/final-image leases persist until the
    // actual isolated helper exits; OS retirement releases static handles.
    source: crate::editor_probe::Source,
}
static INSTALLED: OnceLock<Installed> = OnceLock::new();
pub struct Apartment;
impl Apartment {
    pub fn initialize() -> Result<Self> {
        /* SAFETY: this isolated helper owns this fresh thread's MTA. */
        unsafe {
            api(
                "input COM MTA",
                CoInitializeEx(None, COINIT_MULTITHREADED).ok(),
            )?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        /* SAFETY: paired successful CoInitializeEx on this thread. */
        unsafe {
            CoUninitialize();
        }
    }
}
pub fn sha256(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
pub fn install(
    target: &Target,
    owner_pid: u32,
    packaged: Option<PackagedProfile>,
) -> Result<Option<Authority>> {
    let Some(v) = packaged else { return Ok(None) };
    if !profile::digest(&v.digest)
        || v.json.len() > 32 * 1024
        || sha256(v.json.as_bytes()) != v.digest
    {
        return Err(Error::Invalid);
    }
    let schema: serde_json::Value = serde_json::from_str(&v.json).map_err(|_| Error::Invalid)?;
    if schema
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        == Some(4)
    {
        return essential_runtime::install(target, owner_pid, &v);
    }
    let live = if schema
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        == Some(2)
    {
        Some(LiveProfile::decode(&v.json)?)
    } else {
        None
    };
    let profile: Profile = match &live {
        Some(value) => value.metadata.clone(),
        None => serde_json::from_str(&v.json).map_err(|_| Error::Invalid)?,
    };
    profile.validate()?;
    for action in &profile.actions {
        if sha256(&profile::key_batch_bytes(&action.keys)) != action.native_batch_digest {
            return Err(Error::Invalid);
        }
    }
    let installed = Installed {
        profile,
        live,
        digest: v.digest,
        source: crate::editor_probe::Source::open_quiet(target, owner_pid)?,
    };
    identity(target, owner_pid, &installed)?;
    let budget = Budget::production();
    let (focus, verified_actions) = match live_authority(target, owner_pid, &installed, &budget) {
        Ok(v) => v,
        Err(Error::Ungranted | Error::Unavailable) => return Ok(None),
        Err(e) => return Err(e),
    };
    let authority = Authority {
        identity: installed.profile.identity(),
        profile_digest: installed.digest.clone(),
        verified_actions,
        focus,
    };
    INSTALLED.set(installed).map_err(|_| Error::Invalid)?;
    Ok(Some(authority))
}
fn live_focus(
    target: &Target,
    owner_pid: u32,
    p: &Installed,
    budget: &Budget,
) -> Result<CanvasProof> {
    p.source.verify(target, owner_pid)?;
    unchanged(target, owner_pid)?;
    let observer = Observer::open(target, budget)?;
    let (canvas, canvas_rect) = observer.focused_canvas()?;
    let runtime_id_hash = runtime_hash(&canvas)?;
    budget.check()?;
    unchanged(target, owner_pid)?;
    p.source.verify(target, owner_pid)?;
    Ok(CanvasProof {
        runtime_id_hash,
        process_id: target.process_id,
        sampled_qpc_100ns: clock_100ns()?,
        class_name: "KisOpenGLCanvas2".into(),
        control_type: 50026,
        canvas_rect,
        profile_digest: p.digest.clone(),
    })
}
fn live_settings(target: &Target, p: &Installed, budget: &Budget) -> Result<ToolSettings> {
    let live = p.live.as_ref().ok_or(Error::Unavailable)?;
    let value = Observer::open(target, budget)?.settings()?;
    live.watch(&value)?;
    if sha256(&value.settings_bytes()) != p.profile.settings_digest {
        return Err(Error::Unavailable);
    }
    budget.check()?;
    Ok(value)
}
fn live_authority(
    target: &Target,
    owner_pid: u32,
    p: &Installed,
    budget: &Budget,
) -> Result<(CanvasProof, Vec<u32>)> {
    let Some(live) = &p.live else {
        // Shape-only/imported metadata still advertises zero actions.
        return focused(target, owner_pid, p).map(|focus| (focus, Vec::new()));
    };
    budget.check()?;
    let focus = live_focus(target, owner_pid, p, budget)?;
    let settings = match live_settings(target, p, budget) {
        Ok(value) => value,
        Err(Error::Unavailable) => return Ok((focus, Vec::new())),
        Err(error) => return Err(error),
    };
    let observer = Observer::open(target, budget)?;
    let mut actions = Vec::new();
    for action in &live.actions {
        if live.permit(action.action, &settings).is_err() {
            continue;
        }
        let element = observer.element(&action.route)?;
        match observer.proof(&element) {
            Ok(_) => actions.push(action.action),
            Err(Error::Unavailable) => {}
            Err(error) => return Err(error),
        }
    }
    budget.check()?;
    unchanged(target, owner_pid)?;
    p.source.verify(target, owner_pid)?;
    budget.check()?;
    Ok((focus, actions))
}
fn identity(target: &Target, owner_pid: u32, p: &Installed) -> Result<()> {
    p.source.verify(target, owner_pid)?;
    unchanged(target, owner_pid)?;
    let t = vw_capture::windows::fixed_target(target.window, owner_pid)
        .map_err(|_| Error::TargetChanged)?;
    let v = vw_host::inspect_editor_identity_sync(
        t.into(),
        Arc::new(vw_capture::Cancellation::default()),
    )
    .map_err(|_| Error::Unavailable)?;
    let actual = profile::ImageIdentity {
        executable_name: v.executable_name,
        executable_blake3: v.executable_blake3,
        executable_bytes: v.executable_bytes,
        file_version: v
            .file_version
            .map(|v| format!("{}.{}.{}.{}", v.major, v.minor, v.build, v.revision)),
        package_full_name: v.package_full_name,
        package_version: v.package_version,
    };
    if actual != p.profile.identity() {
        return Err(Error::TargetChanged);
    }
    p.source.verify(target, owner_pid)?;
    unchanged(target, owner_pid)
}
struct RuntimeArray(*mut SAFEARRAY);
impl Drop for RuntimeArray {
    fn drop(&mut self) {
        /* SAFETY: GetRuntimeId transfers this non-null SAFEARRAY to this owner. */
        unsafe {
            let _ = SafeArrayDestroy(self.0);
        }
    }
}
fn runtime_hash(element: &IUIAutomationElement) -> Result<String> {
    // SAFETY: owned COM element, bounded validated one-dimensional integer array.
    unsafe {
        let raw = api("focused runtime id", element.GetRuntimeId())?;
        if raw.is_null() {
            return Err(Error::Invalid);
        }
        let array = RuntimeArray(raw);
        if SafeArrayGetDim(array.0) != 1
            || api("runtime id type", SafeArrayGetVartype(array.0))?
                != windows::Win32::System::Variant::VT_I4
        {
            return Err(Error::Invalid);
        }
        let low = api("runtime lower bound", SafeArrayGetLBound(array.0, 1))?;
        let high = api("runtime upper bound", SafeArrayGetUBound(array.0, 1))?;
        let count = i64::from(high) - i64::from(low) + 1;
        if !(1..=128).contains(&count) {
            return Err(Error::Invalid);
        }
        let mut data = ptr::null_mut();
        api("runtime id access", SafeArrayAccessData(array.0, &mut data))?;
        let bytes = if data.is_null() {
            None
        } else {
            Some(
                std::slice::from_raw_parts(data.cast::<i32>(), count as usize)
                    .iter()
                    .flat_map(|n| n.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
        };
        api("runtime id unaccess", SafeArrayUnaccessData(array.0))?;
        Ok(sha256(&bytes.ok_or(Error::Invalid)?))
    }
}
fn focused(target: &Target, owner_pid: u32, p: &Installed) -> Result<CanvasProof> {
    if p.live.is_some() {
        return live_focus(target, owner_pid, p, &Budget::production());
    }
    unchanged(target, owner_pid)?;
    // SAFETY: owned MTA helper; foreign providers are bounded by the parent process deadline.
    unsafe {
        let automation: IUIAutomation = api(
            "UIA canvas",
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER),
        )?;
        let element = api("focused canvas", automation.GetFocusedElement())?;
        let selector = &p.profile.canvas_selector;
        let pid = api("canvas PID", element.CurrentProcessId())?;
        if pid <= 0 || pid as u32 != target.process_id {
            return Err(Error::Ungranted);
        }
        let class = api("canvas class", element.CurrentClassName())?.to_string();
        if class != selector.class_name
            || api("canvas framework", element.CurrentFrameworkId())? != selector.framework_id
            || api("canvas automation id", element.CurrentAutomationId())? != selector.automation_id
            || api("canvas type", element.CurrentControlType())?.0 != selector.control_type
            || !api("canvas keyboard focus", element.CurrentHasKeyboardFocus())?.as_bool()
        {
            return Err(Error::Ungranted);
        }
        let r = api("canvas physical bounds", element.CurrentBoundingRectangle())?;
        let width =
            u32::try_from(i64::from(r.right) - i64::from(r.left)).map_err(|_| Error::Invalid)?;
        let height =
            u32::try_from(i64::from(r.bottom) - i64::from(r.top)).map_err(|_| Error::Invalid)?;
        let canvas_rect = Rect {
            x: r.left,
            y: r.top,
            width,
            height,
        };
        if !canvas_rect.inside(target.client_rect) {
            return Err(Error::TargetChanged);
        }
        let runtime_id_hash = runtime_hash(&element)?;
        unchanged(target, owner_pid)?;
        // This records selected-target geometry. The injector separately applies
        // the cheap current foreground/root-hit guard after all provider calls.
        Ok(CanvasProof {
            runtime_id_hash,
            process_id: pid as u32,
            sampled_qpc_100ns: clock_100ns()?,
            class_name: class,
            control_type: selector.control_type,
            canvas_rect,
            profile_digest: p.digest.clone(),
        })
    }
}
pub fn validate_canvas(
    target: &Target,
    owner_pid: u32,
    point: Option<(i32, i32)>,
) -> Result<CanvasProof> {
    let installed = INSTALLED.get().ok_or(Error::Unavailable)?;
    let proof = focused(target, owner_pid, installed)?;
    if point.is_some_and(|(x, y)| !proof.canvas_rect.contains(x, y)) {
        return Err(Error::Ungranted);
    }
    Ok(proof)
}
fn admitted(digest: &str, action: u32) -> Result<&'static profile::Action> {
    let p = INSTALLED.get().ok_or(Error::Unavailable)?;
    if digest != p.digest {
        return Err(Error::Ungranted);
    }
    let _candidate = p
        .profile
        .actions
        .iter()
        .find(|a| a.action == action)
        .ok_or(Error::Unavailable)?;
    // Root receipt metadata cannot substitute for current native tool/settings
    // proof. A later source-admitted observer must provide that proof first.
    Err(Error::Unavailable)
}
pub fn prepare(
    target: &Target,
    owner_pid: u32,
    digest: &str,
    action: u32,
    runtime: &str,
) -> Result<Vec<INPUT>> {
    let p = INSTALLED.get().ok_or(Error::Unavailable)?;
    let action = admitted(digest, action)?;
    identity(target, owner_pid, p)?;
    let proof = focused(target, owner_pid, p)?;
    if proof.runtime_id_hash != runtime {
        return Err(Error::Ungranted);
    }
    let key = |v: u16, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(v),
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    Ok(action
        .keys
        .iter()
        .map(|v| key(*v, false))
        .chain(action.keys.iter().rev().map(|v| key(*v, true)))
        .collect())
}
pub fn validate_current(
    target: &Target,
    owner_pid: u32,
    digest: &str,
    action: u32,
    runtime: &str,
) -> Result<CanvasProof> {
    admitted(digest, action)?;
    let proof = validate_canvas(target, owner_pid, None)?;
    if proof.runtime_id_hash != runtime {
        return Err(Error::Ungranted);
    }
    Ok(proof)
}

/// A complete finite batch whose coordinates came from a current, uniquely
/// scoped native element. The raw inputs cannot be mutated through this API.
pub struct PreparedBatch {
    inputs: Vec<INPUT>,
    pub destination: (i32, i32),
    pub destination_runtime_hash: String,
    pub canvas: CanvasProof,
    pub action: u32,
    pub profile_digest: String,
    settings: ToolSettings,
    element: ElementProof,
    absolute_destination: (i32, i32),
    started: std::time::Instant,
}
impl PreparedBatch {
    pub fn inputs(&self) -> &[INPUT] {
        &self.inputs
    }
}
#[derive(serde::Serialize)]
pub struct Measurement {
    pub settings: ToolSettings,
    pub canvas: CanvasProof,
    pub element: ElementProof,
    pub lookup_micros: u64,
}
/// Root-owned blank-editor fixture only. No profile is installed or advertised;
/// production helpers never call this observer-only measurement entry point.
pub fn measure(target: &Target, owner_pid: u32, action: u32) -> Result<Measurement> {
    let budget = Budget::new(std::time::Duration::from_secs(2));
    unchanged(target, owner_pid)?;
    let observer = Observer::open(target, &budget)?;
    let settings = observer.settings()?;
    let (canvas, canvas_rect) = observer.focused_canvas()?;
    let runtime_id_hash = runtime_hash(&canvas)?;
    let route = if action == 90 {
        profile::live::fixture_rectangle_route()?
    } else {
        profile::live::route_for(action)?
    };
    let element = observer.proof(&observer.element(&route)?)?;
    budget.check()?;
    unchanged(target, owner_pid)?;
    Ok(Measurement {
        settings,
        element,
        lookup_micros: budget.elapsed_micros(),
        canvas: CanvasProof {
            runtime_id_hash,
            process_id: target.process_id,
            sampled_qpc_100ns: clock_100ns()?,
            class_name: "KisOpenGLCanvas2".into(),
            control_type: 50026,
            canvas_rect,
            profile_digest: String::new(),
        },
    })
}
#[derive(serde::Serialize)]
pub struct ClickMeasurement {
    pub attempted: bool,
    pub native_accepted_count: u32,
    pub expected_count: u32,
    pub accepted_qpc_100ns: Option<u64>,
    pub error: Option<Error>,
    pub editor_effect_proven: bool,
}
pub fn measure_click(
    target: &Target,
    owner_pid: u32,
    action: u32,
    expected: &Measurement,
) -> Result<ClickMeasurement> {
    let current = measure(target, owner_pid, action)?;
    if current.settings != expected.settings
        || current.element != expected.element
        || current.canvas.runtime_id_hash != expected.canvas.runtime_id_hash
        || current.canvas.canvas_rect != expected.canvas.canvas_rect
    {
        return Err(Error::TargetChanged);
    }
    let destination = center(current.element.physical_rect)?;
    let batch = click(absolute(destination)?);
    fn native_rect(v: Rect) -> super::guard::Rect {
        super::guard::Rect {
            left: v.x,
            top: v.y,
            right: v.x + v.width as i32,
            bottom: v.y + v.height as i32,
        }
    }
    let native = super::guard::Target {
        hwnd: target.window as usize,
        pid: target.process_id,
        thread: target.thread_id,
        process_created: target.process_created,
        window: native_rect(target.window_rect),
        client: native_rect(target.client_rect),
        dpi: target.dpi,
        integrity: target.integrity,
    };
    let mut guard =
        super::native_guard::Injector::arm(native, 1, false).map_err(|_| Error::TargetChanged)?;
    unchanged(target, owner_pid)?;
    neutral_input()?;
    // This actual point/foreground guard follows all blocking provider work.
    guard
        .validate_point(destination.0, destination.1)
        .map_err(|_| Error::TargetChanged)?;
    // SAFETY: exactly three initialized source-owned balanced mouse INPUTs;
    // immutable measured destination, final native guard just succeeded. Win32
    // does not offer an atomic guard-and-inject transaction.
    let count = unsafe { SendInput(&batch, std::mem::size_of::<INPUT>() as i32) };
    // Preserve actual injection even if its subsequent clock query fails.
    // Returning early here would hide a completed native input batch.
    let (accepted_qpc_100ns, error) = if count == 3 {
        match clock_100ns() {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        }
    } else {
        (None, Some(Error::PartialInput))
    };
    let result = ClickMeasurement {
        attempted: true,
        native_accepted_count: count,
        expected_count: 3,
        accepted_qpc_100ns,
        error,
        editor_effect_proven: false,
    };
    // Drop releases only a synthetic device with no pen contacts. Partial mouse
    // failure does not claim a global button-release repair or editor effect.
    drop(guard);
    Ok(result)
}
fn center(rect: Rect) -> Result<(i32, i32)> {
    rect.validate()?;
    Ok((
        rect.x + (rect.width / 2) as i32,
        rect.y + (rect.height / 2) as i32,
    ))
}
fn absolute(point: (i32, i32)) -> Result<(i32, i32)> {
    use windows::Win32::UI::WindowsAndMessaging::*;
    // SAFETY: query-only physical virtual desktop after parent helper PMv2 init.
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    if width <= 1
        || height <= 1
        || i64::from(point.0) < i64::from(left)
        || i64::from(point.1) < i64::from(top)
        || i64::from(point.0) >= i64::from(left) + i64::from(width)
        || i64::from(point.1) >= i64::from(top) + i64::from(height)
    {
        return Err(Error::Invalid);
    }
    Ok((
        ((i64::from(point.0) - i64::from(left)) * 65535 / (i64::from(width) - 1)) as i32,
        ((i64::from(point.1) - i64::from(top)) * 65535 / (i64::from(height) - 1)) as i32,
    ))
}
fn click(point: (i32, i32)) -> Vec<INPUT> {
    let mouse = |dx, dy, dw_flags| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: dw_flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    vec![
        mouse(
            point.0,
            point.1,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        ),
        mouse(0, 0, MOUSEEVENTF_LEFTDOWN),
        mouse(0, 0, MOUSEEVENTF_LEFTUP),
    ]
}
fn neutral_input() -> Result<()> {
    // Do not turn an owner's held button/modifier into a balanced remote click
    // that releases it or changes the tested toolbar command's meaning.
    for key in [1, 2, 4, 5, 6, 0x10, 0x11, 0x12, 0x5b, 0x5c] {
        // SAFETY: query-only current asynchronous input state; no mutation.
        if unsafe { GetAsyncKeyState(key) } as u16 & 0x8000 != 0 {
            return Err(Error::Ungranted);
        }
    }
    Ok(())
}
pub fn prepare_batch(
    target: &Target,
    owner_pid: u32,
    digest: &str,
    action: u32,
    runtime: &str,
) -> Result<PreparedBatch> {
    let started = std::time::Instant::now();
    let budget = Budget::production();
    let p = INSTALLED.get().ok_or(Error::Unavailable)?;
    if digest != p.digest {
        return Err(Error::Ungranted);
    }
    let live = p.live.as_ref().ok_or(Error::Unavailable)?;
    let route = &live.action(action)?.route;
    identity(target, owner_pid, p)?;
    budget.check()?;
    let settings = live_settings(target, p, &budget)?;
    live.permit(action, &settings)?;
    let canvas = live_focus(target, owner_pid, p, &budget)?;
    if canvas.runtime_id_hash != runtime {
        return Err(Error::Ungranted);
    }
    let observer = Observer::open(target, &budget)?;
    let element = observer.proof(&observer.element(route)?)?;
    let destination = center(element.physical_rect)?;
    let absolute_destination = absolute(destination)?;
    let inputs = click(absolute_destination);
    unchanged(target, owner_pid)?;
    budget.check()?;
    Ok(PreparedBatch {
        inputs,
        destination,
        destination_runtime_hash: element.runtime_id_hash.clone(),
        canvas,
        action,
        profile_digest: digest.into(),
        settings,
        element,
        absolute_destination,
        started,
    })
}
pub fn validate_prepared(target: &Target, owner_pid: u32, prepared: &PreparedBatch) -> Result<()> {
    // This spans preparation AND revalidation and cannot extend the unchanged
    // parent finite-command deadline. Blocked providers remain Job-owned.
    if prepared.started.elapsed()
        >= std::time::Duration::from_millis(observer::PRODUCTION_BUDGET_MS)
    {
        return Err(Error::Timeout);
    }
    let budget = Budget::new(
        std::time::Duration::from_millis(observer::PRODUCTION_BUDGET_MS)
            .saturating_sub(prepared.started.elapsed()),
    );
    let p = INSTALLED.get().ok_or(Error::Unavailable)?;
    if prepared.profile_digest != p.digest {
        return Err(Error::Ungranted);
    }
    let live = p.live.as_ref().ok_or(Error::Unavailable)?;
    let action = live.action(prepared.action)?;
    identity(target, owner_pid, p)?;
    budget.check()?;
    let focus = live_focus(target, owner_pid, p, &budget)?;
    let settings = live_settings(target, p, &budget)?;
    live.permit(prepared.action, &settings)?;
    if settings != prepared.settings
        || focus.runtime_id_hash != prepared.canvas.runtime_id_hash
        || focus.process_id != prepared.canvas.process_id
        || focus.canvas_rect != prepared.canvas.canvas_rect
        || focus.class_name != prepared.canvas.class_name
        || focus.control_type != prepared.canvas.control_type
        || focus.profile_digest != prepared.canvas.profile_digest
    {
        return Err(Error::TargetChanged);
    }
    let observer = Observer::open(target, &budget)?;
    let current = observer.proof(&observer.element(&action.route)?)?;
    if current != prepared.element
        || current.runtime_id_hash != prepared.destination_runtime_hash
        || center(current.physical_rect)? != prepared.destination
        || absolute(prepared.destination)? != prepared.absolute_destination
    {
        return Err(Error::TargetChanged);
    }
    unchanged(target, owner_pid)?;
    budget.check()?;
    neutral_input()?;
    budget.check()
    // Caller performs final cheap destination/foreground guard immediately
    // before SendInput. No provider calls follow that guard in this module.
}

/// The caller retains the same actual input helper/SourceLease owner. Stage two
/// is prepared only after first Complete+no error from its common finite owner.
pub enum PreparedFiniteCommand {
    Legacy(Box<PreparedBatch>),
    Essential(Box<essential_runtime::Command>),
}
pub enum PreparedStage<'a> {
    Legacy(&'a PreparedBatch),
    Essential(&'a essential_runtime::Stage),
}
impl PreparedStage<'_> {
    pub fn inputs(&self) -> &[INPUT] {
        match self {
            Self::Legacy(v) => v.inputs(),
            Self::Essential(v) => v.inputs(),
        }
    }
    pub fn destination(&self) -> (i32, i32) {
        match self {
            Self::Legacy(v) => v.destination,
            Self::Essential(v) => v.destination(),
        }
    }
}
impl PreparedFiniteCommand {
    pub fn first_stage(&self) -> PreparedStage<'_> {
        match self {
            Self::Legacy(v) => PreparedStage::Legacy(v),
            Self::Essential(v) => PreparedStage::Essential(v.first_stage()),
        }
    }
}
pub struct PreparedSecondStage(essential_runtime::Stage);
impl PreparedSecondStage {
    pub fn stage(&self) -> PreparedStage<'_> {
        PreparedStage::Essential(&self.0)
    }
}
pub fn prepare_command(
    target: &Target,
    owner: u32,
    digest: &str,
    action: u32,
    runtime: &str,
) -> Result<PreparedFiniteCommand> {
    if essential_runtime::installed() {
        essential_runtime::prepare(target, owner, digest, action, runtime)
            .map(|value| PreparedFiniteCommand::Essential(Box::new(value)))
    } else {
        prepare_batch(target, owner, digest, action, runtime)
            .map(|value| PreparedFiniteCommand::Legacy(Box::new(value)))
    }
}
fn completed_first_stage(expected: usize, attempt: &vw_remote::wire::FiniteAttempt) -> Result<()> {
    if !matches!(
        attempt.retirement,
        vw_remote::wire::FiniteRetirement::Complete
    ) {
        return Err(Error::RetirementPending);
    }
    if let Some(error) = &attempt.error {
        return Err(error.clone());
    }
    if expected == 0
        || attempt.expected_count as usize != expected
        || attempt.accepted_count != attempt.expected_count
        || !attempt.accepted_qpc_100ns.is_some_and(|value| value > 0)
    {
        return Err(Error::PartialInput);
    }
    Ok(())
}
pub fn prepare_second(
    target: &Target,
    owner: u32,
    command: &PreparedFiniteCommand,
    first_attempt: &vw_remote::wire::FiniteAttempt,
) -> Result<Option<PreparedSecondStage>> {
    // This is the actual result from the retained finite owner for this command;
    // the input adapter must never manufacture a receipt or substitute another
    // command's result. Refusal precedes all source/UIA work.
    completed_first_stage(command.first_stage().inputs().len(), first_attempt)?;
    match command {
        PreparedFiniteCommand::Legacy(_) => Ok(None),
        PreparedFiniteCommand::Essential(v) => {
            essential_runtime::second(target, owner, v).map(|v| v.map(PreparedSecondStage))
        }
    }
}
pub fn validate_stage(
    target: &Target,
    owner: u32,
    command: &PreparedFiniteCommand,
    stage: &PreparedStage<'_>,
) -> Result<()> {
    match (command, stage) {
        (PreparedFiniteCommand::Legacy(expected), PreparedStage::Legacy(value))
            if std::ptr::eq(expected.as_ref(), *value) =>
        {
            validate_prepared(target, owner, value)
        }
        (PreparedFiniteCommand::Essential(command), PreparedStage::Essential(stage)) => {
            essential_runtime::validate(target, owner, command, stage)
        }
        _ => Err(Error::Ungranted),
    }
}

/// Query-only current essential authority. The input protocol caller must gate
/// contacts/known-owned downs and retain exact binding/parent deadline. It must
/// not block an already accepted Up ACK or create another input grant.
pub fn refresh_authority(target: &Target, owner: u32, digest: &str) -> Result<Authority> {
    essential_runtime::refresh(target, owner, digest)
}

#[cfg(test)]
mod stage_completion_tests {
    use super::*;
    use vw_remote::wire::{FiniteAttempt, FiniteRetirement};
    fn attempt() -> FiniteAttempt {
        FiniteAttempt {
            expected_count: 3,
            accepted_count: 3,
            accepted_qpc_100ns: Some(1),
            error: None,
            retirement: FiniteRetirement::Complete,
        }
    }
    #[test]
    fn second_stage_requires_complete_and_no_error_even_after_release() {
        let mut value = attempt();
        assert_eq!(completed_first_stage(3, &value), Ok(()));
        value.error = Some(Error::PartialInput);
        assert_eq!(completed_first_stage(3, &value), Err(Error::PartialInput));
        value.error = None;
        value.retirement = FiniteRetirement::Pending {
            held_count: 1,
            uncertain: false,
            error: Error::RetirementPending,
        };
        assert_eq!(
            completed_first_stage(3, &value),
            Err(Error::RetirementPending)
        );
    }
    #[test]
    fn second_stage_rejects_partial_wrong_count_or_unknown_acceptance_clock() {
        let mut value = attempt();
        value.accepted_count = 2;
        assert_eq!(completed_first_stage(3, &value), Err(Error::PartialInput));
        value = attempt();
        assert_eq!(completed_first_stage(2, &value), Err(Error::PartialInput));
        value.accepted_qpc_100ns = None;
        assert_eq!(completed_first_stage(3, &value), Err(Error::PartialInput));
        value.accepted_qpc_100ns = Some(0);
        assert_eq!(completed_first_stage(3, &value), Err(Error::PartialInput));
    }
}
