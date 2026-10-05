//! Explicit root-owned blank-editor effect harness. Never packaged authority.
mod krita;
mod paint;
mod restoration;
mod terminal;
use crate::{
    Error, Result,
    editor_probe::{Source, image},
    platform,
};
use restoration::{Restoration, restore};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use vw_remote::{
    Target,
    profile::{ImageIdentity, live::ToolSettings},
};
use windows::{
    Win32::System::{JobObjects::IsProcessInJob, Threading::GetCurrentProcess},
    core::BOOL,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: u32,
    owner_pid: u32,
    target: Target,
    image: ImageIdentity,
    action: u32,
    owned_blank_fixture: bool,
    consent_to_finite_click: bool,
    expected_tool_settings: Option<ToolSettings>,
    #[serde(default = "default_restore")]
    restore_before_exit: bool,
}
fn default_restore() -> bool {
    true
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    mode: String,
    request: Request,
}
#[derive(Serialize)]
struct Receipt {
    schema: u32,
    operation: &'static str,
    target: Target,
    image: ImageIdentity,
    parent_job_present: bool,
    held_process_and_image_namespace: bool,
    owned_blank_fixture_asserted: bool,
    before: platform::Measurement,
    injection: Option<platform::ClickMeasurement>,
    after: Option<platform::Measurement>,
    after_error: Option<Error>,
    observed_tool_transition: Option<bool>,
    restoration: Restoration,
    input_sent: bool,
    profile_authority: bool,
    physical_or_editor_pixel_effect_proven: bool,
}
pub fn run() -> Result<()> {
    let mut producer = terminal::Producer::default();
    let result = run_inner(&mut producer);
    if result.is_err() {
        // Closed source paths before any shared-owner send have sent no input.
        // After an attempt, only finish(actual_owner) can emit Complete.
        producer.failed_before_input()?;
    }
    result
}
fn run_inner(producer: &mut terminal::Producer) -> Result<()> {
    let mut parent = BOOL::default();
    // SAFETY: query-only current-process pseudo handle and initialized output.
    unsafe {
        platform::api(
            "effect parent Job",
            IsProcessInJob(GetCurrentProcess(), None, &mut parent),
        )?;
    }
    if !parent.as_bool() {
        return Err(Error::Ungranted);
    }
    let _job = platform::confine()?;
    let _apartment = platform::EffectApartment::initialize()?;
    platform::initialize_hil_dpi()?;
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 {
        return Err(Error::Limit);
    }
    let mode: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    if matches!(
        mode.get("mode").and_then(serde_json::Value::as_str),
        Some("--paint-observe" | "--paint-experiment")
    ) {
        return paint::run(&bytes, producer);
    }
    if matches!(
        mode.get("mode").and_then(serde_json::Value::as_str),
        Some(
            "--krita-observe"
                | "--krita-experiment"
                | "--krita-wheel-observe"
                | "--krita-wheel-experiment"
        )
    ) {
        return krita::run(&bytes, producer);
    }
    let invocation: Invocation = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    let r = invocation.request;
    // The legacy measurement click used a raw SendInput pair. It has no shared
    // accepted-down owner and cannot participate in this release rendezvous.
    // A later separately reviewed Krita finite adapter will own input here.
    if invocation.mode == "--click" {
        return Err(Error::Ungranted);
    }
    if r.schema != 1
        || r.owner_pid == 0
        || !r.owned_blank_fixture
        || !matches!(invocation.mode.as_str(), "--observe" | "--click")
        || !matches!(r.action, 1..=4 | 90)
    {
        return Err(Error::Invalid);
    }
    if invocation.mode == "--click"
        && (!r.consent_to_finite_click || r.expected_tool_settings.is_none())
    {
        return Err(Error::Ungranted);
    }
    r.target.validate()?;
    let source = Source::open(&r.target, r.owner_pid)?;
    if image(&r.target, r.owner_pid)? != r.image {
        return Err(Error::TargetChanged);
    }
    source.verify(&r.target, r.owner_pid)?;
    let before = platform::measure(&r.target, r.owner_pid, r.action)?;
    source.verify(&r.target, r.owner_pid)?;
    if image(&r.target, r.owner_pid)? != r.image {
        return Err(Error::TargetChanged);
    }
    let injection = if invocation.mode == "--click" {
        if r.expected_tool_settings.as_ref() != Some(&before.settings) {
            return Err(Error::TargetChanged);
        }
        Some(platform::measure_click(
            &r.target,
            r.owner_pid,
            r.action,
            &before,
        )?)
    } else {
        None
    };
    // No retry/replay of input. After an actual attempted batch, observation
    // errors must be retained in a receipt instead of hiding injection evidence.
    let after_result = source
        .verify(&r.target, r.owner_pid)
        .and_then(|()| {
            if image(&r.target, r.owner_pid)? != r.image {
                return Err(Error::TargetChanged);
            }
            platform::measure(&r.target, r.owner_pid, r.action)
        })
        .and_then(|value| {
            source.verify(&r.target, r.owner_pid)?;
            Ok(value)
        });
    let (after, after_error) = match after_result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    };
    let observed_tool_transition = after.as_ref().and_then(|value| match r.action {
        3 => Some(!before.settings.freehand_selected && value.settings.freehand_selected),
        4 => Some(value.settings.eraser_mode != before.settings.eraser_mode),
        90 => Some(
            value.settings.selected_tool_id == vw_remote::profile::live::RECTANGLE_ID
                && value.settings.selected_tool_id != before.settings.selected_tool_id,
        ),
        _ => None,
    });
    let restoration = if let Some(outcome) = &injection {
        if let Some(error) = &outcome.error {
            Restoration::failed(error.clone())
        } else if r.restore_before_exit {
            restore(&source, &r.target, r.owner_pid, &before.settings)
        } else {
            Restoration::inactive()
        }
    } else {
        Restoration::inactive()
    };
    let input_sent = injection
        .as_ref()
        .is_some_and(|v| v.native_accepted_count > 0)
        || restoration
            .injections
            .iter()
            .any(|v| v.native_accepted_count > 0);
    let receipt = Receipt {
        schema: 1,
        operation: "owned-blank-editor-effect-hil",
        target: r.target,
        image: r.image,
        parent_job_present: true,
        held_process_and_image_namespace: true,
        owned_blank_fixture_asserted: true,
        before,
        injection,
        after,
        after_error,
        observed_tool_transition,
        restoration,
        input_sent,
        profile_authority: false,
        physical_or_editor_pixel_effect_proven: false,
    };
    let bytes = serde_json::to_vec(&receipt).map_err(|_| Error::Invalid)?;
    if bytes.len() > 1024 * 1024 {
        return Err(Error::Limit);
    }
    producer.complete_without_input()?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&bytes)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}
