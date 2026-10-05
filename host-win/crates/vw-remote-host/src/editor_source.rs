//! Explicit retained source trust. This adapter grants no profile/input authority.
use crate::{Error, Result, editor_probe::Source, platform};
use serde::{Deserialize, Serialize};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use vw_host::editor_paint_package::{
    PAINT_FULL_NAME, PAINT_VERSION, PaintPackageError, PaintPackageEvidence, PaintPackageLease,
};
use vw_remote::{Target, profile::ImageIdentity};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePolicy {
    Ordinary,
    InstalledPaint,
}
fn policy_image(policy: SourcePolicy, image: &ImageIdentity) -> Result<()> {
    if image.executable_bytes == 0
        || image.executable_bytes > 256 * 1024 * 1024
        || !vw_remote::profile::digest(&image.executable_blake3)
    {
        return Err(Error::Invalid);
    }
    match policy {
        SourcePolicy::Ordinary
            if image.package_full_name.is_none() && image.package_version.is_none() =>
        {
            Ok(())
        }
        SourcePolicy::InstalledPaint
            if image.executable_name == "mspaint.exe"
                && image.package_full_name.as_deref() == Some(PAINT_FULL_NAME)
                && image.package_version.as_deref() == Some(PAINT_VERSION) =>
        {
            Ok(())
        }
        _ => Err(Error::Unavailable),
    }
}
/// Same actual caller cancellation owner and absolute invocation deadline.
/// Create it once for all source/UIA work, never after a slow call returns.
pub struct SourceBudget {
    cancel: Arc<vw_capture::Cancellation>,
    deadline: Instant,
}
fn checked_call<T>(check: impl Fn() -> Result<()>, call: impl FnOnce() -> Result<T>) -> Result<T> {
    check()?;
    let value = call();
    check()?;
    value
}
impl SourceBudget {
    pub fn until(cancel: Arc<vw_capture::Cancellation>, deadline: Instant) -> Result<Self> {
        let budget = Self { cancel, deadline };
        if budget.remaining()? > Duration::from_secs(5) {
            return Err(Error::Limit);
        }
        Ok(budget)
    }
    pub fn new(cancel: Arc<vw_capture::Cancellation>, remaining: Duration) -> Result<Self> {
        if remaining.is_zero() {
            return Err(Error::Timeout);
        }
        if remaining > Duration::from_secs(5) {
            return Err(Error::Limit);
        }
        Self::until(
            cancel,
            Instant::now().checked_add(remaining).ok_or(Error::Limit)?,
        )
    }
    pub fn remaining(&self) -> Result<Duration> {
        self.cancel.check().map_err(|_| Error::Cancelled)?;
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|v| !v.is_zero())
            .ok_or(Error::Timeout)
    }
    pub fn check(&self) -> Result<()> {
        self.remaining().map(|_| ())
    }
    /// A caller may bracket its other source/provider calls with the exact same
    /// deadline. A late completion refuses; it cannot reset or renew this budget.
    pub fn call<T>(&self, call: impl FnOnce() -> Result<T>) -> Result<T> {
        checked_call(|| self.check(), call)
    }
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct SourceProof {
    pub policy: SourcePolicy,
    pub held_process: bool,
    pub held_final_image: bool,
    pub held_ancestor_namespace: bool,
    pub installed_package_namespace: bool,
    pub namespace_lease_equivalence_claim: bool,
    pub input_authority: bool,
    pub profile_authority: bool,
}
enum Held {
    Ordinary(Source),
    InstalledPaint(Box<PaintPackageLease>),
}
/// Caller must retain this lease, actual worker/Job/IO/permit through affirmative
/// process retirement and completed joins, including drop/cancel/blocked calls.
/// No COM/UIA provider or target code is loaded by this adapter.
pub struct SourceLease {
    held: Held,
    target: Target,
    owner: u32,
    identity: ImageIdentity,
    cancel: Arc<vw_capture::Cancellation>,
    policy: SourcePolicy,
}
fn package_error(error: PaintPackageError) -> Error {
    match error {
        PaintPackageError::Policy => Error::Unavailable,
        PaintPackageError::TargetChanged => Error::TargetChanged,
        PaintPackageError::Cancelled => Error::Cancelled,
        PaintPackageError::Timeout => Error::Timeout,
        PaintPackageError::Limit => Error::Limit,
        PaintPackageError::Platform { stage, code } => Error::Platform {
            phase: format!("paint-package/{stage}"),
            code: code as u32,
        },
    }
}
fn target(target: &Target, owner: u32, budget: &SourceBudget) -> Result<vw_host::CaptureTarget> {
    budget.call(|| {
        platform::unchanged(target, owner)?;
        vw_capture::windows::fixed_target(target.window, owner)
            .map(Into::into)
            .map_err(|_| Error::TargetChanged)
    })
}
fn identity(value: &vw_host::RemoteEditorImageIdentity) -> ImageIdentity {
    ImageIdentity {
        executable_name: value.executable_name.clone(),
        executable_blake3: value.executable_blake3.clone(),
        executable_bytes: value.executable_bytes,
        file_version: value
            .file_version
            .map(|v| format!("{}.{}.{}.{}", v.major, v.minor, v.build, v.revision)),
        package_full_name: value.package_full_name.clone(),
        package_version: value.package_version.clone(),
    }
}
impl SourceLease {
    /// Policy/expected identity must come from the admitted exact target/profile
    /// evidence. No filename/title or failed ordinary open chooses InstalledPaint.
    pub fn open(
        target_value: &Target,
        owner: u32,
        expected: &ImageIdentity,
        policy: SourcePolicy,
        budget: &SourceBudget,
    ) -> Result<Self> {
        policy_image(policy, expected)?;
        budget.check()?;
        target_value.validate()?;
        let (held, actual) = match policy {
            SourcePolicy::Ordinary => {
                let source = budget.call(|| Source::open_quiet(target_value, owner))?;
                let value = vw_host::inspect_editor_identity_sync(
                    target(target_value, owner, budget)?,
                    budget.cancel.clone(),
                );
                budget.check()?;
                let actual = identity(&value.map_err(|_| Error::Unavailable)?);
                budget.call(|| source.verify(target_value, owner))?;
                (Held::Ordinary(source), actual)
            }
            SourcePolicy::InstalledPaint => {
                let lease = PaintPackageLease::open_with_budget(
                    target(target_value, owner, budget)?,
                    owner,
                    budget.cancel.clone(),
                    budget.remaining()?,
                )
                .map_err(package_error)?;
                budget.check()?;
                let actual = identity(lease.identity());
                (Held::InstalledPaint(Box::new(lease)), actual)
            }
        };
        policy_image(policy, &actual)?;
        if &actual != expected {
            return Err(Error::TargetChanged);
        }
        let value = Self {
            held,
            target: target_value.clone(),
            owner,
            identity: actual,
            cancel: budget.cancel.clone(),
            policy,
        };
        value.verify(target_value, owner, budget)?;
        Ok(value)
    }
    pub fn identity(&self) -> &ImageIdentity {
        &self.identity
    }
    /// Borrowed actual installed-package facts; no new query, lease or authority.
    /// Valid only while this same retained SourceLease remains owned and freshly verified.
    pub fn installed_package_evidence(&self) -> Option<&PaintPackageEvidence> {
        match &self.held {
            Held::Ordinary(_) => None,
            Held::InstalledPaint(lease) => Some(lease.evidence()),
        }
    }
    pub fn proof(&self) -> SourceProof {
        SourceProof {
            policy: self.policy,
            held_process: true,
            held_final_image: true,
            held_ancestor_namespace: self.policy == SourcePolicy::Ordinary,
            installed_package_namespace: self.policy == SourcePolicy::InstalledPaint,
            namespace_lease_equivalence_claim: false,
            input_authority: false,
            profile_authority: false,
        }
    }
    pub fn verify(&self, current: &Target, owner: u32, budget: &SourceBudget) -> Result<()> {
        budget.check()?;
        if owner != self.owner
            || current != &self.target
            || !Arc::ptr_eq(&self.cancel, &budget.cancel)
        {
            return Err(Error::TargetChanged);
        }
        budget.call(|| platform::unchanged(current, owner))?;
        match &self.held {
            Held::Ordinary(source) => budget.call(|| source.verify(current, owner))?,
            Held::InstalledPaint(lease) => {
                lease
                    .verify_with_budget(target(current, owner, budget)?, owner, budget.remaining()?)
                    .map_err(package_error)?;
                budget.check()?;
                if identity(lease.identity()) != self.identity {
                    return Err(Error::TargetChanged);
                }
            }
        }
        budget.call(|| platform::unchanged(current, owner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image() -> ImageIdentity {
        ImageIdentity {
            executable_name: "mspaint.exe".into(),
            executable_blake3: "a".repeat(64),
            executable_bytes: 17,
            file_version: Some("11.2605.81.0".into()),
            package_full_name: Some(PAINT_FULL_NAME.into()),
            package_version: Some(PAINT_VERSION.into()),
        }
    }
    #[test]
    fn explicit_policy_does_not_fallback_or_accept_unknown_packages() -> Result<()> {
        let mut value = image();
        policy_image(SourcePolicy::InstalledPaint, &value)?;
        assert_eq!(
            policy_image(SourcePolicy::Ordinary, &value),
            Err(Error::Unavailable)
        );
        value.package_version = Some("11.2605.82.0".into());
        assert_eq!(
            policy_image(SourcePolicy::InstalledPaint, &value),
            Err(Error::Unavailable)
        );
        value.package_full_name = None;
        value.package_version = None;
        assert_eq!(
            policy_image(SourcePolicy::InstalledPaint, &value),
            Err(Error::Unavailable)
        );
        policy_image(SourcePolicy::Ordinary, &value)?;
        Ok(())
    }
    #[test]
    fn expired_or_cancelled_shared_budget_never_enters_call() {
        let cancel = Arc::new(vw_capture::Cancellation::default());
        let budget = SourceBudget {
            cancel: cancel.clone(),
            deadline: Instant::now() - Duration::from_millis(1),
        };
        let entered = std::cell::Cell::new(false);
        assert_eq!(
            budget.call(|| {
                entered.set(true);
                Ok(())
            }),
            Err(Error::Timeout)
        );
        assert!(!entered.get());
        cancel.cancel();
        assert_eq!(
            budget.call(|| {
                entered.set(true);
                Ok(())
            }),
            Err(Error::Cancelled)
        );
        assert!(!entered.get());
    }
    #[test]
    fn post_call_deadline_is_not_reset_and_same_cancel_owner_is_required() -> Result<()> {
        let cancel = Arc::new(vw_capture::Cancellation::default());
        let budget = SourceBudget::new(cancel.clone(), Duration::from_secs(1))?;
        let checks = std::cell::Cell::new(0);
        let completed = std::cell::Cell::new(false);
        let outcome = checked_call(
            || {
                checks.set(checks.get() + 1);
                if checks.get() == 1 {
                    Ok(())
                } else {
                    Err(Error::Timeout)
                }
            },
            || {
                completed.set(true);
                Ok(7)
            },
        );
        assert_eq!(outcome, Err(Error::Timeout));
        assert!(completed.get());
        assert_eq!(checks.get(), 2);
        assert!(Arc::ptr_eq(&cancel, &budget.cancel));
        assert!(!Arc::ptr_eq(
            &Arc::new(vw_capture::Cancellation::default()),
            &budget.cancel
        ));
        Ok(())
    }
}
