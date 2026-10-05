//! Exact current process package selects a trusted catalog policy only. No
//! basename/title/failed ordinary open fallback and no action authority here.
use crate::{
    Error, Result,
    editor_source::{SourceBudget, SourceLease, SourcePolicy},
    platform,
};
use vw_remote::{
    Target,
    profile::{
        PackagedProfile,
        catalog::{Catalog, Entry},
        essential,
    },
};
use windows::{
    Win32::{
        Foundation::{
            APPMODEL_ERROR_NO_PACKAGE, CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS,
            FILETIME, HANDLE,
        },
        Storage::Packaging::Appx::GetPackageFullName,
        System::Threading::{
            GetExitCodeProcess, GetProcessId, GetProcessTimes, OpenProcess,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
    core::PWSTR,
};

struct Process(HANDLE);
impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: uniquely owned query-only process handle, including errors.
        let _ = unsafe { CloseHandle(self.0) };
    }
}
fn created(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}
fn handle_target(
    process: &Process,
    target: &Target,
    owner: u32,
    budget: &SourceBudget,
) -> Result<()> {
    budget.call(|| {
        platform::unchanged(target, owner)?;
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let mut exit_code = 0;
        // SAFETY: retained actual selected-process handle and initialized outputs.
        unsafe {
            platform::api(
                "catalog process times",
                GetProcessTimes(process.0, &mut creation, &mut exit, &mut kernel, &mut user),
            )?;
            platform::api(
                "catalog process lifetime",
                GetExitCodeProcess(process.0, &mut exit_code),
            )?;
            if GetProcessId(process.0) != target.process_id
                || created(creation) != target.process_created
                || exit_code != 259
            {
                return Err(Error::TargetChanged);
            }
        }
        platform::unchanged(target, owner)
    })
}
fn package_result(code: u32, units: Option<&[u16]>) -> Result<Option<String>> {
    if code == APPMODEL_ERROR_NO_PACKAGE.0 && units.is_none() {
        return Ok(None);
    }
    if code != ERROR_SUCCESS.0 {
        return Err(Error::Unavailable);
    }
    let units = units.ok_or(Error::Invalid)?;
    if !(2..=2048).contains(&units.len())
        || units.last() != Some(&0)
        || units[..units.len() - 1].contains(&0)
    {
        return Err(Error::Invalid);
    }
    let name = String::from_utf16(&units[..units.len() - 1]).map_err(|_| Error::Invalid)?;
    if name.len() > 2048 || name.chars().any(char::is_control) {
        return Err(Error::Limit);
    }
    Ok(Some(name))
}
fn running_package(
    process: &Process,
    target: &Target,
    owner: u32,
    budget: &SourceBudget,
) -> Result<Option<String>> {
    handle_target(process, target, owner, budget)?;
    let mut length = 0;
    // SAFETY: null-buffer size query on the retained exact selected process.
    let first = budget.call(|| Ok(unsafe { GetPackageFullName(process.0, &mut length, None) }))?;
    let result = if first == APPMODEL_ERROR_NO_PACKAGE {
        package_result(first.0, None)?
    } else {
        if first != ERROR_INSUFFICIENT_BUFFER || !(2..=2048).contains(&length) {
            return Err(Error::Unavailable);
        }
        let mut units = vec![0u16; length as usize];
        let capacity = units.len();
        // SAFETY: owned initialized bounded UTF-16 output buffer; exact count
        // remains checked before any slice or copy. No fixed path/name inference.
        let code = budget.call(|| {
            Ok(unsafe {
                GetPackageFullName(process.0, &mut length, Some(PWSTR(units.as_mut_ptr())))
            })
        })?;
        if length as usize > capacity {
            return Err(Error::Invalid);
        }
        package_result(code.0, Some(&units[..length as usize]))?
    };
    handle_target(process, target, owner, budget)?;
    Ok(result)
}
pub(super) struct Selected {
    pub entry: Entry,
    pub source: SourceLease,
    pub digest: String,
}
/// Caller already owns the isolated helper/Job/worker/IO/permit. The shared
/// absolute budget covers selection AND actual image/package lease adoption.
pub(super) fn select(
    target: &Target,
    owner: u32,
    packaged: &PackagedProfile,
    budget: &SourceBudget,
) -> Result<Option<Selected>> {
    if !vw_remote::profile::digest(&packaged.digest)
        || packaged.json.len() > 32 * 1024
        || super::sha256(packaged.json.as_bytes()) != packaged.digest
    {
        return Err(Error::Invalid);
    }
    let catalog = Catalog::decode(&packaged.json)?;
    target.validate()?;
    budget.call(|| platform::unchanged(target, owner))?;
    // SAFETY: read-only process handle; PID/creation/lifetime checked on this
    // actual handle before package query and again after source adoption.
    let process = Process(budget.call(|| {
        platform::api("catalog process open", unsafe {
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, target.process_id)
        })
    })?);
    let package = running_package(&process, target, owner, budget)?;
    // Only this exact validated selection miss is absence. Package/provider
    // failures above and source/identity races below remain typed refusals.
    let entry = match catalog.applicable_to_running_package(package.as_deref())? {
        Some(entry) => entry.clone(),
        None => {
            if running_package(&process, target, owner, budget)? != package {
                return Err(Error::TargetChanged);
            }
            budget.check()?;
            return Ok(None);
        }
    };
    let policy = match entry.policy() {
        essential::SourcePolicy::Ordinary => SourcePolicy::Ordinary,
        essential::SourcePolicy::InstalledPaint => SourcePolicy::InstalledPaint,
    };
    let source = SourceLease::open(target, owner, &entry.identity(), policy, budget)?;
    if running_package(&process, target, owner, budget)? != package {
        return Err(Error::TargetChanged);
    }
    source.verify(target, owner, budget)?;
    Ok(Some(Selected {
        entry,
        source,
        digest: packaged.digest.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_actual_no_package_code_selects_ordinary_not_error_or_empty_success() {
        assert_eq!(package_result(APPMODEL_ERROR_NO_PACKAGE.0, None), Ok(None));
        assert_eq!(package_result(5, None), Err(Error::Unavailable));
        assert_eq!(
            package_result(ERROR_SUCCESS.0, Some(&[0])),
            Err(Error::Invalid)
        );
        assert_eq!(
            package_result(APPMODEL_ERROR_NO_PACKAGE.0, Some(&[0])),
            Err(Error::Unavailable)
        );
    }
    #[test]
    fn package_output_rejects_missing_terminal_null_interior_null_and_oversize() {
        assert_eq!(package_result(0, Some(&[65, 66])), Err(Error::Invalid));
        assert_eq!(
            package_result(0, Some(&[65, 0, 66, 0])),
            Err(Error::Invalid)
        );
        assert_eq!(
            package_result(0, Some(&vec![65; 2049])),
            Err(Error::Invalid)
        );
        assert_eq!(package_result(0, Some(&[0xd800, 0])), Err(Error::Invalid));
    }
}
