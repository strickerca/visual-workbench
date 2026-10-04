//! Child of capture.rs: shares its bounded worker/cancellation owner.
use super::*;
#[uniffi::export]
impl CaptureService {
    /// Read-only refresh of a previously selected exact HWND/process identity.
    /// It does not activate a window, register a hook or capture a frame.
    pub async fn refresh_target(&self, expected: CaptureTarget) -> Result<CaptureTarget> {
        owned(Arc::new(capture::Cancellation::default()), move || {
            let expected: capture::WindowTarget = expected.into();
            expected.validate(owner())?;
            #[cfg(windows)]
            {
                let current = capture::windows::fixed_target(expected.window, owner())?;
                capture::refreshed_target(&expected, current, owner()).map(Into::into)
            }
            #[cfg(not(windows))]
            {
                Err(capture::Error::Unsupported)
            }
        })
        .await
    }
}
