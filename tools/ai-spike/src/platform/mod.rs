//! Platform-only credential and HTTPS boundary. Offline commands never call it.

#[cfg(windows)]
mod windows;

use crate::{
    Result,
    request::{Prepared, ProviderResponse},
};

/// The caller must reserve cost and confirm this exact request before entering
/// this function. Credentials are never accepted through arguments or files.
pub fn send_confirmed(
    request: &Prepared,
    confirmation: &str,
    reservation: &mut crate::budget::Reservation,
) -> Result<ProviderResponse> {
    reservation.ensure_unused()?;
    if confirmation != request.request_id
        || reservation.request_id() != request.request_id
        || request.description.provider.estimate()?.is_none()
    {
        return Err(crate::Error::Confirmation);
    }
    request.validate_binding()?;
    reservation.begin_attempt(&request.request_id, &request.description.provider)?;
    #[cfg(windows)]
    {
        windows::send(request)
    }
    #[cfg(not(windows))]
    {
        Err(crate::Error::UnsupportedPlatform)
    }
}
