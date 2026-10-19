//! Programmatic, query-only entry. No CLI arguments, stdout receipt, input or
//! capture path participates in a compatibility import.
use super::*;
use crate::editor_source::SourceLease;
use serde::Deserialize;
use vw_remote::{Target, profile::ImageIdentity};

pub struct PaintObservationHelper {
    path: PathBuf,
    digest: String,
    pins: Arc<Pins>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaintControlsObservation {
    pub tool_id: String,
    pub settings_digest: String,
}
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Observed<T> {
    Known {
        value: T,
    },
    Unknown {
        #[serde(rename = "error")]
        _error: serde::de::IgnoredAny,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Controls {
    schema: u32,
    selected_root_only: bool,
    required_settings_known: bool,
    observed_start_qpc_100ns: u64,
    observed_end_qpc_100ns: u64,
    recognized_tool: Observed<String>,
    current_settings_digest: Option<String>,
    frame_witness: Observed<FrameWitness>,
    input_sent: bool,
    profile_authority: bool,
    effect_proven: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameWitness {
    settings_digest: String,
    canvas_tool_name: String,
}
#[derive(Deserialize)]
struct Observation {
    controls: Controls,
}
#[derive(Deserialize)]
struct Output {
    schema: u32,
    target: Target,
    image: ImageIdentity,
    parent_job_present: bool,
    source_namespace_policy: String,
    held_process_and_final_image: bool,
    held_ancestor_namespace: bool,
    namespace_lease_equivalence_claim: bool,
    os_package_namespace_validated: bool,
    foreground_same_selected_window_before: bool,
    foreground_same_selected_window_after: bool,
    input_sent: bool,
    profile_authority: bool,
    observation: Observation,
}
fn project(
    bytes: &[u8],
    target: &Target,
    image: &ImageIdentity,
) -> Result<PaintControlsObservation> {
    // Project known fields directly. Unknown UIA trees are skipped by serde;
    // never allocate a full dynamic JSON tree in the application process.
    if bytes.is_empty() || bytes.len() > MAX_PROBE_OUTPUT {
        return Err(Error::Limit);
    }
    let v: Output = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    let c = v.observation.controls;
    if v.schema != 1
        || &v.target != target
        || &v.image != image
        || !v.parent_job_present
        || v.source_namespace_policy != "installed-paint-v1"
        || !v.held_process_and_final_image
        || v.held_ancestor_namespace
        || v.namespace_lease_equivalence_claim
        || !v.os_package_namespace_validated
        || !v.foreground_same_selected_window_before
        || !v.foreground_same_selected_window_after
        || v.input_sent
        || v.profile_authority
        || c.schema != 1
        || !c.selected_root_only
        || !c.required_settings_known
        || c.input_sent
        || c.profile_authority
        || c.effect_proven
        || c.observed_start_qpc_100ns == 0
        || c.observed_end_qpc_100ns < c.observed_start_qpc_100ns
    {
        return Err(Error::Unavailable);
    }
    let (Observed::Known { value: tool }, Observed::Known { value: frame }, Some(settings)) = (
        c.recognized_tool,
        c.frame_witness,
        c.current_settings_digest,
    ) else {
        return Err(Error::Unavailable);
    };
    let tool_id = match tool.as_str() {
        "brush" if frame.canvas_tool_name == "Using Brush tool on Canvas" => "paint.brush",
        "eraser" if frame.canvas_tool_name == "Using Eraser tool on Canvas" => "paint.eraser",
        _ => return Err(Error::Unavailable),
    };
    if !vw_remote::profile::digest(&settings) || frame.settings_digest != settings {
        return Err(Error::Invalid);
    }
    Ok(PaintControlsObservation {
        tool_id: tool_id.into(),
        settings_digest: settings,
    })
}
impl PaintObservationHelper {
    pub fn open(path: &Path, digest: &str) -> Result<Self> {
        Ok(Self {
            path: path.into(),
            digest: digest.into(),
            pins: locked(path, digest)?,
        })
    }
    pub fn observe(
        &self,
        target: &Target,
        source: Arc<SourceLease>,
        cancel: &Cancellation,
    ) -> Result<PaintControlsObservation> {
        cancel.check()?;
        self.pins.verify()?;
        let request = serde_json::to_string(&serde_json::json!({
            "schema":1,"owner_pid":std::process::id(),"target":target,"image":source.identity()
        }))
        .map_err(|_| Error::Invalid)?;
        let bytes = invocation("--observe-paint-controls", &request)?;
        let permit = Permit::acquire()?;
        let mut child = Child::launch(&self.path, &self.pins, cancel)?;
        let pipes = Arc::new(Mutex::new(Pipes {
            input: child.stdin.take(),
            output: child.stdout.take(),
        }));
        let mut owner = Owner {
            child,
            pins: self.pins.clone(),
            pipes,
            io: None,
            _permit: permit,
            input_uncertain: Arc::new(AtomicBool::new(false)),
            _source: Some(source.clone()),
        };
        let mut error = owner.child.startup_error.clone();
        if error.is_none() {
            let p = owner.pipes.clone();
            let pins = owner.pins.clone();
            let uncertain = owner.input_uncertain.clone();
            match thread::Builder::new()
                .name("paint-observation-io".into())
                .spawn(move || io(p, bytes, pins, uncertain, false, None))
            {
                Ok(worker) => owner.io = Some(worker),
                Err(_) => error = Some(Error::Io),
            }
        }
        let deadline = Instant::now() + DEADLINE;
        while error.is_none() && !owner.retired().unwrap_or(false) {
            if let Err(cause) = cancel.check() {
                error = Some(cause);
            } else if Instant::now() >= deadline {
                error = Some(Error::Timeout);
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
        if error.is_some() {
            owner.kill_if_released();
        }
        let retain = |owner: Owner, cause: Error| {
            let mut receipt = receipt(&owner, &self.digest, error.clone());
            receipt.retirement_error = Some(cause);
            RETAINED
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(PendingOwner {
                    owner,
                    receipt,
                    quiet: true,
                });
            Error::RetirementPending
        };
        match owner.reap(Duration::from_secs(5)) {
            Ok(true) => {}
            Ok(false) => return Err(retain(owner, Error::RetirementPending)),
            Err(cause) => return Err(retain(owner, cause)),
        }
        let outcome = match owner.collect() {
            Collection::Pending(cause) => return Err(retain(owner, cause)),
            Collection::Joined { code, outcome, .. } => {
                if let Some(error) = error {
                    return Err(error);
                }
                if code != 0 {
                    return Err(Error::Unavailable);
                }
                outcome?.ok_or(Error::Invalid)?
            }
        };
        cancel.check()?;
        project(&outcome.bytes?, target, source.identity())
    }
}

/// Retry the existing strong failure owners without emitting private probe
/// observations into application stdout. A failed join/query remains owned.
pub fn retry_paint_observation_retirements() -> Result<usize> {
    let Some(registry) = RETAINED.get() else {
        return Ok(usize::from(ACTIVE.load(Ordering::Acquire)));
    };
    let mut owners = registry.lock().map_err(|_| Error::Io)?;
    let mut at = 0;
    while at < owners.len() {
        if !owners[at].quiet {
            at += 1;
            continue;
        }
        match owners[at].owner.collect() {
            Collection::Joined { .. } => {
                owners.remove(at);
            }
            Collection::Pending(_) => {
                owners[at].owner.kill_if_released();
                at += 1;
            }
        }
    }
    Ok(owners.iter().filter(|v| v.quiet).count() + usize::from(ACTIVE.load(Ordering::Acquire)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::json!({"schema":1,"target":{"window":1,"process_id":2,"process_created":3,
            "token":"selected","thread_id":4,"client_rect":{"x":0,"y":0,"width":100,"height":100},"window_rect":{"x":0,"y":0,"width":100,"height":100},"frame_rect":{"x":0,"y":0,"width":100,"height":100},"dpi":96,"integrity":8192},
            "image":{"executableName":"mspaint.exe","executableBlake3":"a".repeat(64),"executableBytes":128,
                "fileVersion":null,"packageFullName":"Paint_1.2.3.4_x64__test","packageVersion":"1.2.3.4"},
            "parent_job_present":true,"source_namespace_policy":"installed-paint-v1",
            "held_process_and_final_image":true,"held_ancestor_namespace":false,
            "namespace_lease_equivalence_claim":false,"os_package_namespace_validated":true,
            "foreground_same_selected_window_before":true,"foreground_same_selected_window_after":true,
            "input_sent":false,"profile_authority":false,
            "observation":{"controls":{"schema":1,"selectedRootOnly":true,"requiredSettingsKnown":true,
                "observedStartQpc100ns":10,"observedEndQpc100ns":11,"recognizedTool":{"status":"known","value":"brush"},
                "currentSettingsDigest":"b".repeat(64),"inputSent":false,"profileAuthority":false,"effectProven":false,
                "frameWitness":{"status":"known","value":{"settingsDigest":"b".repeat(64),
                    "canvasToolName":"Using Brush tool on Canvas"}}}}})
    }
    fn check(value: &serde_json::Value) -> Result<PaintControlsObservation> {
        let original = fixture();
        let target =
            serde_json::from_value(original["target"].clone()).map_err(|_| Error::Invalid)?;
        let image =
            serde_json::from_value(original["image"].clone()).map_err(|_| Error::Invalid)?;
        project(
            &serde_json::to_vec(value).map_err(|_| Error::Invalid)?,
            &target,
            &image,
        )
    }
    #[test]
    fn projects_only_actual_known_tool_and_settings() -> Result<()> {
        let result = check(&fixture())?;
        assert_eq!(result.tool_id, "paint.brush");
        assert_eq!(result.settings_digest, "b".repeat(64));
        let mut v = fixture();
        v["observation"]["controls"]["recognizedTool"]["value"] = "eraser".into();
        v["observation"]["controls"]["frameWitness"]["value"]["canvasToolName"] =
            "Using Eraser tool on Canvas".into();
        assert_eq!(check(&v)?.tool_id, "paint.eraser");
        Ok(())
    }
    #[test]
    fn refuses_source_job_foreground_and_identity_changes() {
        for field in [
            "parent_job_present",
            "held_process_and_final_image",
            "os_package_namespace_validated",
            "foreground_same_selected_window_before",
            "foreground_same_selected_window_after",
        ] {
            let mut v = fixture();
            v[field] = false.into();
            assert!(check(&v).is_err(), "{field}");
        }
        let mut v = fixture();
        v["target"]["process_id"] = 99.into();
        assert!(check(&v).is_err());
        let mut v = fixture();
        v["image"]["executableBlake3"] = "c".repeat(64).into();
        assert!(check(&v).is_err());
    }
    #[test]
    fn refuses_injected_profile_effect_unknown_or_mismatched_settings() {
        for field in ["inputSent", "profileAuthority", "effectProven"] {
            let mut v = fixture();
            v["observation"]["controls"][field] = true.into();
            assert!(check(&v).is_err());
        }
        let mut v = fixture();
        v["observation"]["controls"]["recognizedTool"] =
            serde_json::json!({"status":"unknown","error":"Unavailable"});
        assert!(check(&v).is_err());
        let mut v = fixture();
        v["observation"]["controls"]["frameWitness"]["value"]["settingsDigest"] =
            "c".repeat(64).into();
        assert!(check(&v).is_err());
    }
    #[test]
    fn rejects_duplicate_and_oversized_input() -> Result<()> {
        let v = fixture();
        let raw = serde_json::to_string(&v)
            .map_err(|_| Error::Invalid)?
            .replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1);
        let parsed = serde_json::from_str::<Output>(&raw);
        assert!(parsed.is_err());
        let original = fixture();
        let target =
            serde_json::from_value(original["target"].clone()).map_err(|_| Error::Invalid)?;
        let image =
            serde_json::from_value(original["image"].clone()).map_err(|_| Error::Invalid)?;
        assert!(project(&vec![b' '; MAX_PROBE_OUTPUT + 1], &target, &image).is_err());
        Ok(())
    }
}
