//! Separate OS-installed Paint trust basis. Never a weaker ordinary path lease.
//! Only query APIs and retained final-image data are used. No UIA/input authority.
use serde::Serialize;
use vw_capture as capture;
#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::PaintPackageLease;

pub const PAINT_FAMILY: &str = "Microsoft.Paint_8wekyb3d8bbwe";
pub const PAINT_FULL_NAME: &str = "Microsoft.Paint_11.2605.81.0_x64__8wekyb3d8bbwe";
pub const PAINT_VERSION: &str = "11.2605.81.0";
pub const PAINT_PUBLISHER: &str =
    "CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US";
pub const PAINT_RELATIVE_IMAGE: &str = "PaintApp\\mspaint.exe";
// First admission is restricted to root's measured immutable installation.
// A different volume/location requires a separately reviewed policy update.
pub const PAINT_ORIGINAL_PATH: &str =
    "C:\\Program Files\\WindowsApps\\Microsoft.Paint_11.2605.81.0_x64__8wekyb3d8bbwe";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaintPackageError {
    #[error("Paint package policy refused")]
    Policy,
    #[error("Paint selected source changed")]
    TargetChanged,
    #[error("Paint package read cancelled")]
    Cancelled,
    #[error("Paint package read exceeded cooperative deadline")]
    Timeout,
    #[error("Paint package query exceeded bounds")]
    Limit,
    #[error("Paint package query failed at {stage}: {code}")]
    Platform { stage: &'static str, code: i32 },
}
type Result<T> = std::result::Result<T, PaintPackageError>;

/// Private evidence may contain the installed path. Public evidence must
/// whitelist facts; this record never grants profile or native input authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PaintPackageEvidence {
    pub full_name: String,
    pub family: String,
    pub publisher: String,
    pub version: String,
    pub signature_kind: i32,
    pub development: bool,
    pub framework_or_resource_or_bundle_or_optional: bool,
    pub status_ok: bool,
    pub status_flags: u32,
    pub original_path: String,
    pub winrt_installed_path: String,
    pub effective_path: String,
    pub winrt_effective_path: String,
    pub mutable_path: String,
    pub machine_external_path: String,
    pub user_external_path: String,
    pub effective_external_path: String,
    pub process_image_path: String,
}
fn canonical_dos(value: &str) -> Result<()> {
    let b = value.as_bytes();
    if b.len() < 4
        || b.len() > 32767
        || !b[0].is_ascii_alphabetic()
        || b[1] != b':'
        || b[2] != b'\\'
        || value.contains('/')
        || value.chars().any(char::is_control)
        || value[3..].contains(':')
    {
        return Err(PaintPackageError::Policy);
    }
    if value[3..].split('\\').any(|part| {
        part.is_empty() || part == "." || part == ".." || part.ends_with('.') || part.ends_with(' ')
    }) {
        return Err(PaintPackageError::Policy);
    }
    Ok(())
}
impl PaintPackageEvidence {
    fn validate(&self) -> Result<()> {
        if self.full_name != PAINT_FULL_NAME
            || self.family != PAINT_FAMILY
            || self.publisher != PAINT_PUBLISHER
            || self.version != PAINT_VERSION
            || self.signature_kind != 3
            || self.development
            || self.framework_or_resource_or_bundle_or_optional
            || !self.status_ok
            || self.status_flags != 0
            || self.original_path != PAINT_ORIGINAL_PATH
            || !self.mutable_path.is_empty()
            || !self.machine_external_path.is_empty()
            || !self.user_external_path.is_empty()
            || !self.effective_external_path.is_empty()
            || self.original_path != self.winrt_installed_path
            || self.original_path != self.effective_path
            || self.original_path != self.winrt_effective_path
        {
            return Err(PaintPackageError::Policy);
        }
        canonical_dos(&self.original_path)?;
        canonical_dos(&self.process_image_path)?;
        if self.original_path.rsplit('\\').next() != Some(PAINT_FULL_NAME)
            || self.process_image_path
                != format!("{}\\{}", self.original_path, PAINT_RELATIVE_IMAGE)
        {
            return Err(PaintPackageError::Policy);
        }
        Ok(())
    }
    fn unchanged(&self, fresh: &Self) -> Result<()> {
        fresh.validate()?;
        if self != fresh {
            return Err(PaintPackageError::TargetChanged);
        }
        Ok(())
    }
}
fn capture_error(error: capture::Error) -> PaintPackageError {
    match error {
        capture::Error::Cancelled => PaintPackageError::Cancelled,
        capture::Error::Timeout => PaintPackageError::Timeout,
        capture::Error::Limit => PaintPackageError::Limit,
        _ => PaintPackageError::TargetChanged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> PaintPackageEvidence {
        let path = format!("C:\\Program Files\\WindowsApps\\{PAINT_FULL_NAME}");
        PaintPackageEvidence {
            full_name: PAINT_FULL_NAME.into(),
            family: PAINT_FAMILY.into(),
            publisher: PAINT_PUBLISHER.into(),
            version: PAINT_VERSION.into(),
            signature_kind: 3,
            development: false,
            framework_or_resource_or_bundle_or_optional: false,
            status_ok: true,
            status_flags: 0,
            process_image_path: format!("{path}\\{PAINT_RELATIVE_IMAGE}"),
            original_path: path.clone(),
            winrt_installed_path: path.clone(),
            effective_path: path.clone(),
            winrt_effective_path: path,
            mutable_path: String::new(),
            machine_external_path: String::new(),
            user_external_path: String::new(),
            effective_external_path: String::new(),
        }
    }
    #[test]
    fn exact_os_metadata_is_shape_only_and_store_publisher_both_required() -> Result<()> {
        let baseline = fixture();
        baseline.validate()?;
        for signature in [0, 1, 2, 4, 99] {
            let mut v = baseline.clone();
            v.signature_kind = signature;
            assert_eq!(v.validate(), Err(PaintPackageError::Policy));
        }
        let mut v = baseline.clone();
        v.publisher = "CN=Other Publisher".into();
        assert_eq!(v.validate(), Err(PaintPackageError::Policy));
        Ok(())
    }
    #[test]
    fn mutable_external_effective_and_wrong_relative_image_refuse() {
        let baseline = fixture();
        for field in 0..7 {
            let mut v = baseline.clone();
            match field {
                0 => v.mutable_path = "D:\\Mods".into(),
                1 => v.machine_external_path = "D:\\External".into(),
                2 => v.user_external_path = "D:\\External".into(),
                3 => v.effective_external_path = "D:\\External".into(),
                4 => v.effective_path = "D:\\Mods".into(),
                5 => v.winrt_effective_path = "D:\\Mods".into(),
                _ => {
                    v.process_image_path =
                        format!("{}-alias\\{PAINT_RELATIVE_IMAGE}", v.original_path)
                }
            };
            assert_eq!(v.validate(), Err(PaintPackageError::Policy));
        }
    }
    #[test]
    fn all_status_changes_and_development_or_wrong_version_refuse() {
        let baseline = fixture();
        for bit in 0..13 {
            let mut v = baseline.clone();
            v.status_flags = 1 << bit;
            assert_eq!(v.validate(), Err(PaintPackageError::Policy));
        }
        let mut v = baseline.clone();
        v.status_ok = false;
        assert_eq!(v.validate(), Err(PaintPackageError::Policy));
        v = baseline.clone();
        v.development = true;
        assert_eq!(v.validate(), Err(PaintPackageError::Policy));
        v = baseline;
        v.version = "11.2605.82.0".into();
        assert_eq!(v.validate(), Err(PaintPackageError::Policy));
    }
    #[test]
    fn ambiguous_dos_paths_never_enter_file_provider() {
        for path in [
            "\\\\server\\share\\paint.exe",
            "\\\\?\\C:\\WindowsApps\\paint.exe",
            "C:\\a\\..\\paint.exe",
            "C:\\a\\paint.exe:stream",
            "C:\\a.\\paint.exe",
            "C:\\a\\\\paint.exe",
            "C:/a/paint.exe",
        ] {
            assert_eq!(canonical_dos(path), Err(PaintPackageError::Policy));
        }
    }
    #[test]
    fn fresh_os_registration_comparison_does_not_adopt_an_update() -> Result<()> {
        let baseline = fixture();
        baseline.unchanged(&baseline)?;
        let mut fresh = baseline.clone();
        fresh.full_name.push_str("-new");
        assert_eq!(baseline.unchanged(&fresh), Err(PaintPackageError::Policy));
        fresh = baseline.clone();
        fresh.winrt_installed_path = "D:\\Changed".into();
        assert_eq!(baseline.unchanged(&fresh), Err(PaintPackageError::Policy));
        Ok(())
    }
    #[test]
    fn cancellation_and_timeout_remain_distinct() {
        assert_eq!(
            capture_error(capture::Error::Cancelled),
            PaintPackageError::Cancelled
        );
        assert_eq!(
            capture_error(capture::Error::Timeout),
            PaintPackageError::Timeout
        );
    }
}
