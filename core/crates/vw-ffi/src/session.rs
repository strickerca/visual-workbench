//! Authenticated app sessions. Each listener/connection has an isolated, bounded
//! runtime: platform trust reads during TLS cannot stall another active session.
mod bootstrap;
mod discovery;
mod integration;
mod link;
mod pairing;
mod preview;
mod replication;
mod runtime;
mod trust;
pub use bootstrap::*;
pub use discovery::*;
pub use integration::*;
pub use link::*;
pub use pairing::*;
pub use preview::{NewObjectPreview, ObjectPreview, PeerPreviews, PreviewKind};
pub use trust::*;

#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum SessionError {
    #[error("invalid session input")]
    Invalid,
    #[error("session capacity exhausted")]
    Backpressure,
    #[error("session closed")]
    Closed,
    #[error("session operation cancelled")]
    Cancelled,
    #[error("peer authentication failed or pairing was revoked")]
    Authentication,
    #[error("pairing offer expired or was consumed")]
    Expired,
    #[error("pairing temporarily locked after unsuccessful attempts")]
    LockedOut,
    #[error("fingerprint confirmation declined")]
    Declined,
    #[error("protected app storage failed")]
    Storage,
    #[error("bounded network operation timed out")]
    Timeout,
    #[error("connection interrupted")]
    Transport,
    #[error("session worker unavailable")]
    Worker,
    #[error("remote edit is unavailable on this platform or package")]
    RemoteUnavailable,
    #[error("remote edit retirement pending; sealed owner retained")]
    RemoteRetirementPending,
    #[error("remote edit partial input; held global key state is unknown")]
    RemotePartialInput,
}
pub type SessionResult<T> = std::result::Result<T, SessionError>;
impl From<vw_net::pairing::PairingError> for SessionError {
    fn from(error: vw_net::pairing::PairingError) -> Self {
        use vw_net::pairing::PairingError as P;
        match error {
            P::Invalid(_) => Self::Invalid,
            P::Authentication | P::Untrusted => Self::Authentication,
            P::Expired | P::Used => Self::Expired,
            P::LockedOut => Self::LockedOut,
            P::Capacity => Self::Backpressure,
            P::Timeout => Self::Timeout,
            P::Connection => Self::Transport,
            P::Declined => Self::Declined,
            P::ClockRollback | P::Conflict | P::Storage | P::Entropy => Self::Storage,
        }
    }
}
impl From<vw_net::NetError> for SessionError {
    fn from(error: vw_net::NetError) -> Self {
        match error {
            vw_net::NetError::Authentication => Self::Authentication,
            vw_net::NetError::Backpressure => Self::Backpressure,
            vw_net::NetError::Timeout => Self::Timeout,
            vw_net::NetError::Store(_) | vw_net::NetError::Io(_) => Self::Storage,
            vw_net::NetError::Carrier => Self::Transport,
            _ => Self::Invalid,
        }
    }
}
impl From<crate::CoreError> for SessionError {
    fn from(error: crate::CoreError) -> Self {
        match error {
            crate::CoreError::Backpressure => Self::Backpressure,
            crate::CoreError::Closed => Self::Closed,
            crate::CoreError::Cancelled => Self::Cancelled,
            crate::CoreError::Storage => Self::Storage,
            crate::CoreError::Worker => Self::Worker,
            _ => Self::Invalid,
        }
    }
}
pub(crate) fn device(value: String) -> SessionResult<vw_model::DeviceId> {
    vw_model::DeviceId::try_from(value).map_err(|_| SessionError::Invalid)
}
pub(crate) fn endpoint(value: &str, allow_zero: bool) -> SessionResult<std::net::SocketAddr> {
    if value.len() > 128 {
        return Err(SessionError::Invalid);
    }
    let address: std::net::SocketAddr = value.parse().map_err(|_| SessionError::Invalid)?;
    let ip = address.ip();
    if ip.is_unspecified()
        || ip.is_multicast()
        || (!allow_zero && address.port() == 0)
        || !match ip {
            std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
            std::net::IpAddr::V6(ip) => {
                ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
            }
        }
    {
        return Err(SessionError::Invalid);
    }
    Ok(address)
}
