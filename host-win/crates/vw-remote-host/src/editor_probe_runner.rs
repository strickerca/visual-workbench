//! Root-owned, input-free probe runner. Reuses actual suspended helper launch.
//! An unsuccessful retirement keeps every owner until joined or outer Job kill.
#[path = "editor_probe_runner/finite.rs"]
mod finite;
#[path = "editor_probe_runner/frame.rs"]
mod frame;
use crate::{
    Error, Result,
    process::{
        Cancellation,
        windows::{Child, Pins},
    },
};
use serde::Serialize;
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::Win32::{Storage::FileSystem::GetDriveTypeW, System::WindowsProgramming::DRIVE_FIXED};
const MAX_INVOCATION: usize = 16 * 1024;
const MAX_PROBE_OUTPUT: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(15);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static RETAINED: OnceLock<Mutex<Vec<PendingOwner>>> = OnceLock::new();
struct Permit;
impl Permit {
    fn acquire() -> Result<Self> {
        ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Limit)?;
        Ok(Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}
struct Pipes {
    input: Option<File>,
    output: Option<File>,
}
struct Outcome {
    _input: Option<File>,
    _output: Option<File>,
    bytes: Result<Vec<u8>>,
}
struct Owner {
    child: Child,
    pins: Arc<Pins>,
    pipes: Arc<Mutex<Pipes>>,
    io: Option<JoinHandle<Outcome>>,
    _permit: Permit,
    input_uncertain: Arc<AtomicBool>,
}
struct PendingOwner {
    owner: Owner,
    receipt: Receipt,
}
enum Collection<T> {
    Pending(Error),
    Joined {
        pid: u32,
        code: u32,
        outcome: Result<Option<T>>,
    },
}
/// Only a fresh affirmative process/Job receipt can authorize consuming the
/// finished worker's join handle. A join error is a completed failed join;
/// failed queries and unfinished workers keep the handle and every owner.
fn collect_latest<T>(
    query: impl FnOnce() -> Result<(u32, u32)>,
    worker_finished: bool,
    join: impl FnOnce() -> Result<Option<T>>,
) -> Collection<T> {
    let (pid, code) = match query() {
        Ok(receipt) => receipt,
        Err(error) => return Collection::Pending(error),
    };
    if !worker_finished {
        return Collection::Pending(Error::RetirementPending);
    }
    Collection::Joined {
        pid,
        code,
        outcome: join(),
    }
}
fn collect_released<T>(
    uncertain: &AtomicBool,
    query: impl FnOnce() -> Result<(u32, u32)>,
    worker_finished: bool,
    join: impl FnOnce() -> Result<Option<T>>,
) -> Collection<T> {
    if uncertain.load(Ordering::Acquire) {
        return Collection::Pending(Error::RetirementPending);
    }
    collect_latest(query, worker_finished, join)
}
#[derive(Clone, Serialize)]
struct Receipt {
    schema: u32,
    status: &'static str,
    probe_pid: Option<u32>,
    probe_exit_code: Option<u32>,
    probe_sha256: String,
    suspended_before_job_assignment: bool,
    exact_image_verified_before_resume: bool,
    process_and_job_retired: bool,
    io_joined: bool,
    error: Option<Error>,
    retirement_error: Option<Error>,
    io_error: Option<Error>,
    observation: Option<serde_json::Value>,
    diagnostic_sha256: Option<String>,
    diagnostic_prefix: Option<String>,
    diagnostic_truncated: bool,
    input_sent: Option<bool>,
    profile_authority: bool,
    effect_harness: bool,
    input_retirement_complete: bool,
}
fn local(path: &Path) -> Result<()> {
    crate::process::windows::namespace(path)?;
    let text = path.to_str().ok_or(Error::Invalid)?;
    let root: Vec<u16> = text[..3].encode_utf16().chain([0]).collect();
    // SAFETY: namespace validated a drive-letter root, initialized NUL terminator.
    if unsafe { GetDriveTypeW(windows::core::PCWSTR(root.as_ptr())) } != DRIVE_FIXED {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn locked(path: &Path, expected: &str) -> Result<Arc<Pins>> {
    locked_image(path, expected, "vw-editor-probe.exe")
}
fn locked_image(path: &Path, expected: &str, name: &str) -> Result<Arc<Pins>> {
    local(path)?;
    if !vw_remote::profile::digest(expected)
        || !path
            .file_name()
            .and_then(|v| v.to_str())
            .is_some_and(|v| v.eq_ignore_ascii_case(name))
    {
        return Err(Error::Invalid);
    }
    let pins = Arc::new(Pins::open(path)?);
    let mut file = pins.executable.file().try_clone()?;
    if file.metadata()?.len() > 128 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = [0u8; 65536];
    let mut read = 0usize;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        read = read.checked_add(n).ok_or(Error::Limit)?;
        if read > 128 * 1024 * 1024 {
            return Err(Error::Limit);
        }
        hash.update(&buffer[..n]);
    }
    let actual: String = hash
        .finish()
        .as_ref()
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect();
    if actual != expected {
        return Err(Error::Invalid);
    }
    pins.verify()?;
    Ok(pins)
}
fn invocation(mode: &str, request: &str) -> Result<Vec<u8>> {
    if !matches!(
        mode,
        "--bind"
            | "--observe"
            | "--bind-paint"
            | "--observe-paint"
            | "--observe-controls"
            | "--observe-paint-controls"
    ) || request.len() > MAX_INVOCATION
    {
        return Err(Error::Invalid);
    }
    let request: serde_json::Value = serde_json::from_str(request).map_err(|_| Error::Invalid)?;
    if !request.is_object() {
        return Err(Error::Invalid);
    }
    let bytes = serde_json::to_vec(&serde_json::json!({"mode":mode,"request":request}))
        .map_err(|_| Error::Invalid)?;
    if bytes.len() > MAX_INVOCATION {
        return Err(Error::Limit);
    }
    Ok(bytes)
}
fn io(
    pipes: Arc<Mutex<Pipes>>,
    bytes: Vec<u8>,
    _pins: Arc<Pins>,
    input_uncertain: Arc<AtomicBool>,
    effect: bool,
) -> Outcome {
    let (input, output) = {
        let mut p = pipes.lock().unwrap_or_else(|e| e.into_inner());
        (p.input.take(), p.output.take())
    };
    // Missing future/malformed endpoints remain in the finished worker result
    // until actual child retirement and join; no artificial parked IO thread.
    let Some(mut output) = output else {
        return Outcome {
            _input: input,
            _output: None,
            bytes: Err(Error::Io),
        };
    };
    let result = match input {
        Some(mut input) => {
            let written = input.write_all(&bytes).map_err(Error::from);
            // Finite request EOF is intentional: the probe's bounded stdin
            // reader cannot finish until this writer closes. Read endpoint,
            // Pins, Child, Job and permit remain retained through actual join.
            drop(input);
            written.and_then(|()| {
                if effect {
                    return finite::read(&mut output, &input_uncertain);
                }
                let mut bytes = Vec::new();
                Read::by_ref(&mut output)
                    .take((MAX_PROBE_OUTPUT + 1) as u64)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > MAX_PROBE_OUTPUT {
                    return Err(Error::Limit);
                }
                Ok(bytes)
            })
        }
        None => Err(Error::Io),
    };
    Outcome {
        _input: None,
        _output: Some(output),
        bytes: result,
    }
}
impl Owner {
    fn may_retire(&self) -> bool {
        !self.input_uncertain.load(Ordering::Acquire)
    }
    fn kill_if_released(&self) {
        if self.may_retire() {
            let _ = self.child.kill_tree();
        }
    }
    fn retired(&self) -> Result<bool> {
        if !self.may_retire() {
            return Ok(false);
        }
        Ok(self.child.exited()? && self.io.as_ref().is_none_or(JoinHandle::is_finished))
    }
    fn collect(&mut self) -> Collection<Outcome> {
        let child = &self.child;
        let worker = &mut self.io;
        collect_released(
            &self.input_uncertain,
            || child.retired_receipt(),
            worker.as_ref().is_none_or(JoinHandle::is_finished),
            || {
                worker
                    .take()
                    .map(|v| v.join().map_err(|_| Error::Io))
                    .transpose()
            },
        )
    }
    fn reap(&self, duration: Duration) -> Result<bool> {
        let started = Instant::now();
        while started.elapsed() < duration {
            if self.retired().unwrap_or(false) {
                return Ok(true);
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.retired()
    }
}
fn emit(receipt: &Receipt) -> Result<()> {
    let bytes = serde_json::to_vec(receipt).map_err(|_| Error::Invalid)?;
    if bytes.len() > MAX_PROBE_OUTPUT + 16 * 1024 {
        return Err(Error::Limit);
    }
    let mut output = std::io::stdout().lock();
    output.write_all(&bytes)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
fn measured_output(bytes: &[u8]) -> Result<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    let source_held = match v.get("source_namespace_policy") {
        None => {
            v.get("held_process_and_image_namespace")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
        }
        Some(policy) if policy.as_str() == Some("installed-paint-v1") => {
            v.get("held_process_and_final_image")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                && v.get("held_ancestor_namespace")
                    .and_then(serde_json::Value::as_bool)
                    == Some(false)
                && v.get("namespace_lease_equivalence_claim")
                    .and_then(serde_json::Value::as_bool)
                    == Some(false)
                && v.get("os_package_namespace_validated")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                && v.get("package_evidence")
                    .is_some_and(serde_json::Value::is_object)
                && v.get("held_process_and_image_namespace").is_none()
        }
        _ => false,
    };
    if v.get("schema").and_then(|v| v.as_u64()) != Some(1)
        || v.get("input_sent").and_then(|v| v.as_bool()) != Some(false)
        || v.get("profile_authority").and_then(|v| v.as_bool()) != Some(false)
        || v.get("parent_job_present").and_then(|v| v.as_bool()) != Some(true)
        || !source_held
        || !v.get("target").is_some_and(serde_json::Value::is_object)
        || !v.get("image").is_some_and(serde_json::Value::is_object)
    {
        return Err(Error::Invalid);
    }
    Ok(v)
}
// Typed ownership projection of the existing wheel producer's receipt. Other
// fields remain raw observations; accepting output never proves effect, successful
// restoration or settlement of the actual child/Job/IO owner.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WheelSourceProof {
    policy: crate::editor_source::SourcePolicy,
    held_process: bool,
    held_final_image: bool,
    held_ancestor_namespace: bool,
    installed_package_namespace: bool,
    namespace_lease_equivalence_claim: bool,
    input_authority: bool,
    profile_authority: bool,
}
#[derive(serde::Deserialize)]
struct WheelReceiptOwner {
    source_proof: WheelSourceProof,
    input_retirement: vw_remote::wire::FiniteRetirement,
    common_finite_owner: bool,
    provider_budget_ms: u32,
    local_preflight_input_budget_ms: u32,
    semantic_role_proven: bool,
    production_parent_exchange_proven: bool,
    physical_or_editor_pixel_effect_proven: bool,
}
fn wheel_receipt_owner(value: &serde_json::Value) -> Result<()> {
    let owner: WheelReceiptOwner =
        serde_json::from_value(value.clone()).map_err(|_| Error::Invalid)?;
    let proof = owner.source_proof;
    if proof.policy != crate::editor_source::SourcePolicy::Ordinary
        || !proof.held_process
        || !proof.held_final_image
        || !proof.held_ancestor_namespace
        || proof.installed_package_namespace
        || proof.namespace_lease_equivalence_claim
        || proof.input_authority
        || proof.profile_authority
        || !owner.common_finite_owner
        || owner.input_retirement != vw_remote::wire::FiniteRetirement::Complete
        || owner.provider_budget_ms != 180
        || owner.local_preflight_input_budget_ms != 250
        || owner.semantic_role_proven
        || owner.production_parent_exchange_proven
        || owner.physical_or_editor_pixel_effect_proven
        || value.get("source_namespace_policy").is_some()
        || value.get("held_process_and_image_namespace").is_some()
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn measured_effect_output(bytes: &[u8]) -> Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    if value.get("schema").and_then(serde_json::Value::as_u64) != Some(1)
        || value
            .get("profile_authority")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
        || value
            .get("parent_job_present")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || value
            .get("owned_blank_fixture_asserted")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || value
            .get("input_sent")
            .and_then(serde_json::Value::as_bool)
            .is_none()
        || !value
            .get("target")
            .is_some_and(serde_json::Value::is_object)
        || !value.get("image").is_some_and(serde_json::Value::is_object)
        || !value
            .get("before")
            .is_some_and(serde_json::Value::is_object)
    {
        return Err(Error::Invalid);
    }
    match value.get("operation").and_then(serde_json::Value::as_str) {
        Some("owned-krita-numeric-wheel-calibration") => wheel_receipt_owner(&value)?,
        Some("owned-blank-editor-effect-hil")
            if value
                .get("held_process_and_image_namespace")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                && value.get("source_namespace_policy").is_none() => {}
        Some("owned-blank-paint-controls-hil")
            if value
                .get("source_namespace_policy")
                .and_then(serde_json::Value::as_str)
                == Some("installed-paint-v1")
                && value
                    .get("held_process_and_final_image")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                && value
                    .get("held_ancestor_namespace")
                    .and_then(serde_json::Value::as_bool)
                    == Some(false)
                && value
                    .get("namespace_lease_equivalence_claim")
                    .and_then(serde_json::Value::as_bool)
                    == Some(false)
                && value
                    .get("held_process_and_image_namespace")
                    .and_then(serde_json::Value::as_bool)
                    == Some(false)
                && value
                    .get("os_package_namespace_validated")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                && value
                    .get("package_evidence")
                    .is_some_and(serde_json::Value::is_object) => {}
        _ => return Err(Error::Invalid),
    }
    Ok(value)
}
fn receipt(owner: &Owner, digest: &str, error: Option<Error>) -> Receipt {
    Receipt {
        schema: 1,
        status: "RetirementPending",
        probe_pid: None,
        probe_exit_code: None,
        probe_sha256: digest.into(),
        suspended_before_job_assignment: true,
        exact_image_verified_before_resume: owner.child.startup_error.is_none(),
        process_and_job_retired: false,
        io_joined: false,
        error,
        retirement_error: None,
        io_error: None,
        observation: None,
        diagnostic_sha256: None,
        diagnostic_prefix: None,
        diagnostic_truncated: false,
        input_sent: Some(false),
        profile_authority: false,
        effect_harness: false,
        input_retirement_complete: owner.may_retire(),
    }
}
fn pending(owner: Owner, mut receipt: Receipt, cause: Error) -> Result<()> {
    receipt.input_retirement_complete = owner.may_retire();
    receipt.retirement_error = Some(cause.clone());
    if receipt.error.is_none() {
        receipt.error = Some(cause);
    }
    // Register before output: a failed stdout write cannot release a live owner.
    let output = receipt.clone();
    RETAINED
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(PendingOwner { owner, receipt });
    emit(&output)?;
    Err(Error::RetirementPending)
}
fn finished(receipt: &mut Receipt, pid: u32, code: u32, outcome: Result<Option<Outcome>>) {
    receipt.probe_pid = Some(pid);
    receipt.probe_exit_code = Some(code);
    receipt.process_and_job_retired = true;
    receipt.io_joined = true;
    let outcome = match outcome {
        Ok(value) => value,
        Err(error) => {
            // join() returned: even worker panic means the join completed.
            // Retain its failure separately from any earlier timeout/query error.
            receipt.io_error = Some(error.clone());
            if receipt.error.is_none() {
                receipt.error = Some(error);
            }
            None
        }
    };
    receipt.observation = match outcome {
        Some(v) => match v.bytes {
            Ok(bytes) => match if receipt.effect_harness {
                measured_effect_output(&bytes)
            } else {
                measured_output(&bytes)
            } {
                Ok(value) => Some(value),
                Err(error) => {
                    receipt.diagnostic_sha256 = Some(
                        ring::digest::digest(&ring::digest::SHA256, &bytes)
                            .as_ref()
                            .iter()
                            .map(|v| format!("{v:02x}"))
                            .collect(),
                    );
                    let text = String::from_utf8_lossy(&bytes);
                    let mut end = text.len().min(2048);
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    receipt.diagnostic_truncated = end != text.len();
                    receipt.diagnostic_prefix = Some(text[..end].into());
                    if receipt.error.is_none() {
                        receipt.error = Some(error);
                    }
                    None
                }
            },
            Err(error) => {
                receipt.io_error = Some(error.clone());
                if receipt.error.is_none() {
                    receipt.error = Some(error);
                }
                None
            }
        },
        None => None,
    };
    if receipt.effect_harness {
        // Failed/partial output never fabricates "no input": retain unknown.
        receipt.input_sent = receipt
            .observation
            .as_ref()
            .and_then(|value| value.get("input_sent"))
            .and_then(serde_json::Value::as_bool);
    }
    if code != 0 && receipt.error.is_none() {
        receipt.error = Some(Error::Unavailable);
    }
    if receipt.observation.is_none() && receipt.error.is_none() {
        receipt.error = Some(Error::Invalid);
    }
    receipt.status = if receipt.error.is_none() {
        "Observed"
    } else {
        "Failed"
    };
}
/// Does not load UIA or target-editor code. Root must also bound this runner
/// with its own retained Job/streams (recommended outer30s, inner probe15s).
pub fn run() -> Result<()> {
    run_for(false)
}
/// Separate explicit root-owned effect entry point. The ordinary probe runner
/// cannot select this image/mode via JSON, and remains input-free.
pub fn run_editor_effect() -> Result<()> {
    run_for(true)
}
/// Separate harness-only WGC coordinator. Never launches from an effect child.
pub fn run_editor_frame() -> Result<()> {
    frame::run()
}
pub fn wait_editor_frame_retained() {
    frame::wait_retained()
}
fn run_for(effect: bool) -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        return Err(Error::Invalid);
    }
    let bytes = if effect {
        if !matches!(
            args[2].as_str(),
            "--observe"
                | "--click"
                | "--paint-observe"
                | "--paint-experiment"
                | "--krita-observe"
                | "--krita-experiment"
                | "--krita-wheel-observe"
                | "--krita-wheel-experiment"
        ) || args[3].len() > MAX_INVOCATION
        {
            return Err(Error::Invalid);
        }
        let request: serde_json::Value =
            serde_json::from_str(&args[3]).map_err(|_| Error::Invalid)?;
        if !request.is_object() {
            return Err(Error::Invalid);
        }
        let bytes = serde_json::to_vec(&serde_json::json!({"mode":args[2],"request":request}))
            .map_err(|_| Error::Invalid)?;
        if bytes.len() > MAX_INVOCATION {
            return Err(Error::Limit);
        }
        bytes
    } else {
        invocation(&args[2], &args[3])?
    };
    let permit = Permit::acquire()?;
    let path = PathBuf::from(&args[0]);
    let pins = if effect {
        locked_image(&path, &args[1], "vw-editor-effect-hil.exe")?
    } else {
        locked(&path, &args[1])?
    };
    let cancel = Cancellation::default();
    // Arm before launch/resume; no child death, timeout or stream failure can
    // clear this flag. Only the closed source-owned terminal parser can clear it.
    let input_uncertain = Arc::new(AtomicBool::new(effect));
    // Child::launch is the already source-admitted CreateProcess(CREATE_SUSPENDED)
    // -> AssignProcessToJobObject -> exact retained image/Pins verify -> Resume.
    // No new UIA/probe user code can execute before its own actual Job contains it.
    let mut child = Child::launch(&path, &pins, &cancel)?;
    let pipes = Arc::new(Mutex::new(Pipes {
        input: child.stdin.take(),
        output: child.stdout.take(),
    }));
    let mut owner = Owner {
        child,
        pins,
        pipes,
        io: None,
        _permit: permit,
        input_uncertain,
    };
    let mut error = owner.child.startup_error.clone();
    if error.is_none() {
        let p = owner.pipes.clone();
        let image = owner.pins.clone();
        let uncertain = owner.input_uncertain.clone();
        match thread::Builder::new()
            .name("editor-probe-io".into())
            .spawn(move || io(p, bytes, image, uncertain, effect))
        {
            Ok(worker) => owner.io = Some(worker),
            Err(_) => error = Some(Error::Io),
        }
    }
    if error.is_none() {
        let started = Instant::now();
        let mut last = 0;
        while !owner.retired().unwrap_or(false) && started.elapsed() < DEADLINE {
            let seconds = started.elapsed().as_secs();
            if seconds > last {
                eprintln!(
                    "editor-probe-runner: waiting for actual child exit and IO join {seconds}/15s"
                );
                last = seconds;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if !owner.retired().unwrap_or(false) {
            error = Some(Error::Timeout);
        }
    }
    if error.is_some() {
        owner.kill_if_released();
    }
    let mut receipt = receipt(&owner, &args[1], error);
    receipt.effect_harness = effect;
    if effect {
        receipt.input_sent = None;
    }
    if !owner.may_retire() {
        return pending(owner, receipt, Error::RetirementPending);
    }
    match owner.reap(Duration::from_secs(5)) {
        Ok(true) => {}
        Ok(false) => return pending(owner, receipt, Error::RetirementPending),
        Err(cause) => return pending(owner, receipt, cause),
    }
    // reap is only an observation. A subsequent failed native receipt must
    // retain ownership; it cannot authorize an early return and Child drop.
    match owner.collect() {
        Collection::Pending(cause) => return pending(owner, receipt, cause),
        Collection::Joined { pid, code, outcome } => finished(&mut receipt, pid, code, outcome),
    }
    let result = receipt.error.clone().map_or(Ok(()), Err);
    emit(&receipt)?;
    // Owner/pipes/file leases/permit release only after actual Job exit + join.
    drop(owner);
    result
}
/// Pending is a strong failure owner, not permission to exit/unload and abandon
/// a live worker. It emits progress while root's outer deadline remains active.
pub fn wait_retained() {
    loop {
        let Some(registry) = RETAINED.get() else {
            return;
        };
        let mut owners = registry.lock().unwrap_or_else(|e| e.into_inner());
        let mut index = 0;
        while index < owners.len() {
            // No earlier predicate is release authority. Query once more
            // inside collection and remove only after an actual completed join.
            match owners[index].owner.collect() {
                Collection::Pending(cause) => {
                    owners[index].receipt.retirement_error = Some(cause);
                    owners[index].owner.kill_if_released();
                    index += 1;
                }
                Collection::Joined { pid, code, outcome } => {
                    let retained = &mut owners[index];
                    retained.receipt.input_retirement_complete = retained.owner.may_retire();
                    finished(&mut retained.receipt, pid, code, outcome);
                    if let Err(error) = emit(&retained.receipt) {
                        eprintln!("editor-probe-runner: retired failure receipt output: {error}");
                    }
                    // A join returning an error is completed; its Failed
                    // receipt above preserves that outcome before release.
                    owners.remove(index);
                }
            }
        }
        if owners.is_empty() {
            return;
        }
        drop(owners);
        eprintln!(
            "editor-probe-runner: RetirementPending; actual owned process/Job/IO still retained"
        );
        thread::sleep(Duration::from_secs(1));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn wheel_output() -> serde_json::Value {
        serde_json::json!({"schema":1,"operation":"owned-krita-numeric-wheel-calibration",
            "profile_authority":false,"parent_job_present":true,
            "owned_blank_fixture_asserted":true,"input_sent":true,"target":{},"image":{},"before":{},
            "source_proof":{"policy":"ordinary","held_process":true,"held_final_image":true,
                "held_ancestor_namespace":true,"installed_package_namespace":false,
                "namespace_lease_equivalence_claim":false,"input_authority":false,"profile_authority":false},
            "common_finite_owner":true,"input_retirement":"Complete",
            "provider_budget_ms":180,"local_preflight_input_budget_ms":250,
            "semantic_role_proven":false,"production_parent_exchange_proven":false,
            "physical_or_editor_pixel_effect_proven":false,"error":null,
            "restoration_exact":true,"owned_fixture_control_cleanup_outstanding":false})
    }
    fn wheel_bytes(value: &serde_json::Value) -> Result<Vec<u8>> {
        serde_json::to_vec(value).map_err(|_| Error::Invalid)
    }
    #[test]
    fn wheel_receipt_uses_actual_source_proof_without_legacy_namespace_claim() -> Result<()> {
        let mut value = wheel_output();
        for input_sent in [false, true] {
            value["input_sent"] = serde_json::json!(input_sent);
            assert_eq!(measured_effect_output(&wheel_bytes(&value)?)?, value);
            assert_eq!(measured_output(&wheel_bytes(&value)?), Err(Error::Invalid));
        }
        Ok(())
    }
    #[test]
    fn wheel_receipt_refuses_missing_or_false_ownership_and_imported_authority() -> Result<()> {
        for field in [
            "held_process",
            "held_final_image",
            "held_ancestor_namespace",
        ] {
            let mut value = wheel_output();
            value["source_proof"][field] = serde_json::json!(false);
            assert_eq!(
                measured_effect_output(&wheel_bytes(&value)?),
                Err(Error::Invalid)
            );
        }
        for field in [
            "installed_package_namespace",
            "namespace_lease_equivalence_claim",
            "input_authority",
            "profile_authority",
        ] {
            let mut value = wheel_output();
            value["source_proof"][field] = serde_json::json!(true);
            assert_eq!(
                measured_effect_output(&wheel_bytes(&value)?),
                Err(Error::Invalid)
            );
        }
        for field in [
            "profile_authority",
            "semantic_role_proven",
            "production_parent_exchange_proven",
            "physical_or_editor_pixel_effect_proven",
        ] {
            let mut value = wheel_output();
            value[field] = serde_json::json!(true);
            assert_eq!(
                measured_effect_output(&wheel_bytes(&value)?),
                Err(Error::Invalid)
            );
        }
        for field in ["source_proof", "common_finite_owner", "input_retirement"] {
            let mut value = wheel_output();
            value.as_object_mut().ok_or(Error::Invalid)?.remove(field);
            assert_eq!(
                measured_effect_output(&wheel_bytes(&value)?),
                Err(Error::Invalid)
            );
        }
        Ok(())
    }
    #[test]
    fn wheel_receipt_refuses_pending_wrong_policy_legacy_fields_and_wrong_budgets() -> Result<()> {
        let mut value = wheel_output();
        value["input_retirement"] =
            serde_json::json!({"Pending":{"held_count":1,"uncertain":true,"error":"Timeout"}});
        assert_eq!(
            measured_effect_output(&wheel_bytes(&value)?),
            Err(Error::Invalid)
        );
        let mut value = wheel_output();
        value["source_proof"]["policy"] = serde_json::json!("installed_paint");
        assert_eq!(
            measured_effect_output(&wheel_bytes(&value)?),
            Err(Error::Invalid)
        );
        let mut value = wheel_output();
        value["source_proof"]["invented_ownership"] = serde_json::json!(true);
        assert_eq!(
            measured_effect_output(&wheel_bytes(&value)?),
            Err(Error::Invalid)
        );
        for (field, replacement) in [
            ("source_namespace_policy", serde_json::json!("ordinary")),
            ("held_process_and_image_namespace", serde_json::json!(true)),
            ("common_finite_owner", serde_json::json!(false)),
            ("provider_budget_ms", serde_json::json!(181)),
            ("local_preflight_input_budget_ms", serde_json::json!(251)),
            (
                "operation",
                serde_json::json!("owned-blank-editor-effect-hil"),
            ),
        ] {
            let mut value = wheel_output();
            value[field] = replacement;
            assert_eq!(
                measured_effect_output(&wheel_bytes(&value)?),
                Err(Error::Invalid)
            );
        }
        Ok(())
    }
    #[test]
    fn completed_wheel_owner_preserves_failed_effect_and_outstanding_restoration() -> Result<()> {
        let mut value = wheel_output();
        value["error"] = serde_json::json!("Timeout");
        value["restoration_exact"] = serde_json::json!(false);
        value["owned_fixture_control_cleanup_outstanding"] = serde_json::json!(true);
        let measured = measured_effect_output(&wheel_bytes(&value)?)?;
        assert_eq!(measured, value);
        assert_eq!(measured["error"], serde_json::json!("Timeout"));
        assert_eq!(measured["restoration_exact"], serde_json::json!(false));
        // This gate only returns the unchanged JSON. Existing actual process,
        // terminal, Job and reader settlement still controls owner collection.
        Ok(())
    }
    #[test]
    fn package_modes_remain_explicit_read_only_and_cannot_claim_ordinary_leases() -> Result<()> {
        invocation("--bind-paint", "{}")?;
        invocation("--observe-paint", "{}")?;
        invocation("--observe-paint-controls", "{}")?;
        assert_eq!(invocation("--paint-click", "{}"), Err(Error::Invalid));
        let mut value = serde_json::json!({"schema":1,"input_sent":false,"profile_authority":false,
            "parent_job_present":true,"source_namespace_policy":"installed-paint-v1",
            "held_process_and_final_image":true,"held_ancestor_namespace":false,
            "namespace_lease_equivalence_claim":false,"os_package_namespace_validated":true,
            "package_evidence":{},"target":{},"image":{}});
        measured_output(&serde_json::to_vec(&value).map_err(|_| Error::Invalid)?)?;
        value["namespace_lease_equivalence_claim"] = serde_json::json!(true);
        assert_eq!(
            measured_output(&serde_json::to_vec(&value).map_err(|_| Error::Invalid)?),
            Err(Error::Invalid)
        );
        value["namespace_lease_equivalence_claim"] = serde_json::json!(false);
        value["held_process_and_image_namespace"] = serde_json::json!(true);
        assert_eq!(
            measured_output(&serde_json::to_vec(&value).map_err(|_| Error::Invalid)?),
            Err(Error::Invalid)
        );
        value["source_namespace_policy"] = serde_json::json!(null);
        assert_eq!(
            measured_output(&serde_json::to_vec(&value).map_err(|_| Error::Invalid)?),
            Err(Error::Invalid)
        );
        Ok(())
    }
    #[test]
    fn effect_output_cannot_be_adopted_by_read_only_probe_or_as_profile_authority() -> Result<()> {
        let mut value = serde_json::json!({"schema":1,"operation":"owned-blank-editor-effect-hil",
            "profile_authority":false,"parent_job_present":true,"held_process_and_image_namespace":true,
            "owned_blank_fixture_asserted":true,"input_sent":true,"target":{},"image":{},"before":{}});
        let bytes = serde_json::to_vec(&value).map_err(|_| Error::Invalid)?;
        assert_eq!(measured_output(&bytes), Err(Error::Invalid));
        measured_effect_output(&bytes)?;
        value["profile_authority"] = serde_json::json!(true);
        let bytes = serde_json::to_vec(&value).map_err(|_| Error::Invalid)?;
        assert_eq!(measured_effect_output(&bytes), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn paint_effect_cannot_claim_ordinary_leases_or_read_only_input() -> Result<()> {
        let mut value = serde_json::json!({"schema":1,"operation":"owned-blank-paint-controls-hil",
            "profile_authority":false,"parent_job_present":true,"held_process_and_image_namespace":false,
            "owned_blank_fixture_asserted":true,"input_sent":true,"target":{},"image":{},"before":{},
            "source_namespace_policy":"installed-paint-v1","held_process_and_final_image":true,
            "held_ancestor_namespace":false,"namespace_lease_equivalence_claim":false,
            "os_package_namespace_validated":true,"package_evidence":{}});
        let bytes = serde_json::to_vec(&value).map_err(|_| Error::Invalid)?;
        measured_effect_output(&bytes)?;
        assert_eq!(measured_output(&bytes), Err(Error::Invalid));
        assert_eq!(invocation("--paint-experiment", "{}"), Err(Error::Invalid));
        value["held_ancestor_namespace"] = serde_json::json!(true);
        assert_eq!(
            measured_effect_output(&serde_json::to_vec(&value).map_err(|_| Error::Invalid)?),
            Err(Error::Invalid)
        );
        value["held_ancestor_namespace"] = serde_json::json!(false);
        value["profile_authority"] = serde_json::json!(true);
        assert_eq!(
            measured_effect_output(&serde_json::to_vec(&value).map_err(|_| Error::Invalid)?),
            Err(Error::Invalid)
        );
        Ok(())
    }
    struct FixtureOwner<'a> {
        worker: Option<u8>,
        joined: &'a Cell<u32>,
        dropped: &'a Cell<u32>,
    }
    impl FixtureOwner<'_> {
        fn collect(
            &mut self,
            query: impl FnOnce() -> Result<(u32, u32)>,
            worker_finished: bool,
        ) -> Collection<u8> {
            collect_latest(query, worker_finished, || {
                self.joined.set(self.joined.get() + 1);
                self.worker.take().map(Some).ok_or(Error::Io)
            })
        }
    }
    impl Drop for FixtureOwner<'_> {
        fn drop(&mut self) {
            self.dropped.set(self.dropped.get() + 1);
        }
    }
    #[test]
    fn only_read_only_modes_and_bounded_object_requests_pass() {
        assert!(invocation("--bind", "{}").is_ok());
        assert!(invocation("--observe", "{}").is_ok());
        assert!(invocation("--observe-controls", "{}").is_ok());
        assert_eq!(invocation("--input", "{}"), Err(Error::Invalid));
        assert_eq!(invocation("--bind", "[]"), Err(Error::Invalid));
        assert_eq!(
            invocation("--bind", &"x".repeat(MAX_INVOCATION + 1)),
            Err(Error::Invalid)
        );
    }
    #[test]
    fn successful_json_cannot_claim_input_or_skip_held_source_membership() -> Result<()> {
        let mut v = serde_json::json!({"schema":1,"input_sent":false,"profile_authority":false,
            "parent_job_present":true,"held_process_and_image_namespace":true,"target":{},"image":{}});
        let bytes = serde_json::to_vec(&v).map_err(|_| Error::Invalid)?;
        assert!(measured_output(&bytes).is_ok());
        v["input_sent"] = serde_json::json!(true);
        let bytes = serde_json::to_vec(&v).map_err(|_| Error::Invalid)?;
        assert_eq!(measured_output(&bytes), Err(Error::Invalid));
        v["input_sent"] = serde_json::json!(false);
        v["parent_job_present"] = serde_json::json!(false);
        let bytes = serde_json::to_vec(&v).map_err(|_| Error::Invalid)?;
        assert_eq!(measured_output(&bytes), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn failed_query_after_affirmative_reap_retains_unjoined_owner() -> Result<()> {
        let joined = Cell::new(0);
        let dropped = Cell::new(0);
        let calls = Cell::new(0);
        let query = || {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                Ok((19, 0))
            } else {
                Err(Error::Io)
            }
        };
        assert!(query().is_ok()); // Prior reap's affirmative native observation.
        let mut owner = FixtureOwner {
            worker: Some(7),
            joined: &joined,
            dropped: &dropped,
        };
        let mut retained = Vec::new();
        match owner.collect(query, true) {
            Collection::Pending(Error::Io) => retained.push(owner),
            _ => return Err(Error::Invalid),
        }
        assert_eq!(calls.get(), 2);
        assert_eq!(joined.get(), 0);
        assert_eq!(dropped.get(), 0);
        assert_eq!(retained.first().map(|v| v.worker), Some(Some(7)));
        let fresh = retained
            .first_mut()
            .ok_or(Error::Invalid)?
            .collect(|| Ok((19, 0)), true);
        match fresh {
            Collection::Joined {
                pid: 19,
                code: 0,
                outcome: Ok(Some(7)),
            } => {
                retained.remove(0);
            }
            _ => return Err(Error::Invalid),
        }
        assert_eq!(joined.get(), 1);
        assert_eq!(dropped.get(), 1);
        Ok(())
    }
    #[test]
    fn failed_collection_after_prior_predicate_cannot_remove_registry_owner() -> Result<()> {
        let joined = Cell::new(0);
        let dropped = Cell::new(0);
        let mut retained = vec![FixtureOwner {
            worker: Some(9),
            joined: &joined,
            dropped: &dropped,
        }];
        let prior_retirement_predicate = true;
        assert!(prior_retirement_predicate);
        let current = retained
            .first_mut()
            .ok_or(Error::Invalid)?
            .collect(|| Err(Error::RetirementPending), true);
        match current {
            Collection::Pending(Error::RetirementPending) => {}
            _ => return Err(Error::Invalid),
        }
        assert_eq!(retained.len(), 1);
        assert_eq!(retained.first().map(|v| v.worker), Some(Some(9)));
        assert_eq!(joined.get(), 0);
        assert_eq!(dropped.get(), 0);
        // Even a good native receipt cannot consume an unfinished IO owner.
        let unfinished = retained
            .first_mut()
            .ok_or(Error::Invalid)?
            .collect(|| Ok((23, 0)), false);
        assert!(matches!(
            unfinished,
            Collection::Pending(Error::RetirementPending)
        ));
        assert_eq!(joined.get(), 0);
        assert_eq!(dropped.get(), 0);
        let current = retained
            .first_mut()
            .ok_or(Error::Invalid)?
            .collect(|| Ok((23, 0)), true);
        match current {
            Collection::Joined {
                pid: 23,
                code: 0,
                outcome: Ok(Some(9)),
            } => {
                retained.remove(0);
            }
            _ => return Err(Error::Invalid),
        }
        assert_eq!(joined.get(), 1);
        assert_eq!(dropped.get(), 1);
        Ok(())
    }
    #[test]
    fn completed_failed_join_reports_failure_and_preserves_prior_error() -> Result<()> {
        let joined = Cell::new(false);
        let collection = collect_latest(
            || Ok((29, 0)),
            true,
            || {
                joined.set(true);
                Err(Error::Io)
            },
        );
        let mut receipt = Receipt {
            schema: 1,
            status: "RetirementPending",
            probe_pid: None,
            probe_exit_code: None,
            probe_sha256: "0".repeat(64),
            suspended_before_job_assignment: true,
            exact_image_verified_before_resume: true,
            process_and_job_retired: false,
            io_joined: false,
            error: Some(Error::Timeout),
            retirement_error: None,
            io_error: None,
            observation: None,
            diagnostic_sha256: None,
            diagnostic_prefix: None,
            diagnostic_truncated: false,
            input_sent: Some(false),
            profile_authority: false,
            effect_harness: false,
            input_retirement_complete: true,
        };
        match collection {
            Collection::Joined { pid, code, outcome } => finished(&mut receipt, pid, code, outcome),
            Collection::Pending(_) => return Err(Error::Invalid),
        }
        assert!(joined.get());
        assert!(receipt.io_joined && receipt.process_and_job_retired);
        assert_eq!(receipt.status, "Failed");
        assert_eq!(receipt.error, Some(Error::Timeout));
        assert_eq!(receipt.io_error, Some(Error::Io));
        assert!(receipt.observation.is_none());
        Ok(())
    }
    #[test]
    fn dead_child_and_finished_io_cannot_consume_uncertain_source_owner() -> Result<()> {
        let uncertain = AtomicBool::new(true);
        let queried = Cell::new(0);
        let joined = Cell::new(0);
        let result = collect_released(
            &uncertain,
            || {
                queried.set(queried.get() + 1);
                Ok((31, 0))
            },
            true,
            || {
                joined.set(joined.get() + 1);
                Ok(Some(9))
            },
        );
        assert!(matches!(
            result,
            Collection::Pending(Error::RetirementPending)
        ));
        assert_eq!((queried.get(), joined.get()), (0, 0));
        // Only the actual terminal consumer can clear uncertainty. The same
        // production collection helper then requires fresh Job query + join.
        let terminal =
            b"{\"schema\":1,\"operation\":\"finite-retirement\",\"retirement\":\"Complete\"}\n";
        assert!(finite::read(&mut std::io::Cursor::new(terminal), &uncertain).is_err());
        assert!(!uncertain.load(Ordering::Acquire)); // final receipt missing -> Failed, but actual UP retained.
        let result = collect_released(
            &uncertain,
            || {
                queried.set(queried.get() + 1);
                Ok((31, 0))
            },
            true,
            || {
                joined.set(joined.get() + 1);
                Ok(Some(9))
            },
        );
        assert!(matches!(
            result,
            Collection::Joined {
                pid: 31,
                code: 0,
                outcome: Ok(Some(9))
            }
        ));
        assert_eq!((queried.get(), joined.get()), (1, 1));
        Ok(())
    }
}
