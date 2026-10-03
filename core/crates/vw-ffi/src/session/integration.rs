//! Read-only app integration. Discovery addresses and route diagnostics never
//! grant trust, choose an interface, or change operating-system configuration.
use super::{SessionError, SessionResult, SessionService, replication};
use crate::ProjectSession;
use std::sync::Arc;

#[derive(Clone, Debug, uniffi::Record)]
pub struct ProjectSessionRole {
    pub project_id: String,
    pub local_device_id: String,
    pub host_device_id: String,
    pub is_host: bool,
    pub pending: u32,
    pub blocked: u32,
}
#[derive(Clone, uniffi::Record)]
pub struct QrDetails {
    pub peer_device_id: String,
    pub fingerprint: String,
    pub expires_at_ms: u64,
    pub endpoints: Vec<String>,
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum TetherRouteRisk {
    Preferred,
    Tied,
    OnlyDefault,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct TetherRouteWarning {
    pub family: String,
    pub interface_index: u32,
    pub risk: TetherRouteRisk,
    pub fix_command: Option<String>,
    pub revert_command: Option<String>,
}

#[uniffi::export]
impl SessionService {
    pub async fn project_role(
        &self,
        project: Arc<ProjectSession>,
    ) -> SessionResult<ProjectSessionRole> {
        let local = self.local_device().await?;
        let state = replication::state(&project).await?;
        if state.local.as_str() != local.device_id {
            return Err(SessionError::Authentication);
        }
        Ok(ProjectSessionRole {
            project_id: state.project.to_string(),
            local_device_id: state.local.to_string(),
            host_device_id: state.host.to_string(),
            is_host: state.local == state.host,
            pending: u32::try_from(state.pending).map_err(|_| SessionError::Backpressure)?,
            blocked: u32::try_from(state.blocked).map_err(|_| SessionError::Backpressure)?,
        })
    }
    /// Decodes and validates the bounded, expiring native payload. Selection is
    /// still checked against these exact endpoints by join_qr before dialing.
    pub fn inspect_qr(&self, qr: Vec<u8>) -> SessionResult<QrDetails> {
        let bytes = zeroize::Zeroizing::new(qr);
        let offer = vw_net::pairing::QrPayload::decode_qr(&bytes)?;
        let endpoints = offer
            .endpoints()
            .iter()
            .map(|address| validate_session_endpoint(address.to_string(), false))
            .collect::<SessionResult<Vec<_>>>()?;
        Ok(QrDetails {
            peer_device_id: offer.device().to_string(),
            fingerprint: offer.fingerprint().display_hex(),
            expires_at_ms: offer.expires_at_ms(),
            endpoints,
        })
    }
    /// The caller supplies the explicitly selected tether interface. The result
    /// contains reviewable administrator commands only; none are executed here.
    pub async fn tether_route_warnings(
        &self,
        interface_index: u32,
    ) -> SessionResult<Vec<TetherRouteWarning>> {
        if interface_index == 0 {
            return Err(SessionError::Invalid);
        }
        #[cfg(windows)]
        {
            crate::worker::startup(move || {
                Ok((|| -> SessionResult<_> {
                    use vw_net::route::{
                        AddressFamily, RouteProvider, RouteRisk, detect_tether_default,
                    };
                    let routes = vw_host_win::routes::WindowsRoutes.default_routes()?;
                    Ok(detect_tether_default(&routes, interface_index)?
                        .into_iter()
                        .map(|finding| {
                            let (fix_command, revert_command) =
                                finding.suggestion.map_or((None, None), |s| {
                                    (Some(s.fix_command), Some(s.revert_command))
                                });
                            TetherRouteWarning {
                                family: match finding.family {
                                    AddressFamily::Ipv4 => "IPv4",
                                    AddressFamily::Ipv6 => "IPv6",
                                }
                                .into(),
                                interface_index: finding.interface_index,
                                risk: match finding.risk {
                                    RouteRisk::Preferred => TetherRouteRisk::Preferred,
                                    RouteRisk::Tied => TetherRouteRisk::Tied,
                                    RouteRisk::OnlyDefault => TetherRouteRisk::OnlyDefault,
                                },
                                fix_command,
                                revert_command,
                            }
                        })
                        .collect())
                })())
            })
            .await?
        }
        #[cfg(not(windows))]
        {
            Err(SessionError::Invalid)
        }
    }
}

/// Numeric endpoint validation shared with platform address enumeration. This
/// does not resolve names, open sockets, or establish that an address is local.
#[uniffi::export]
pub fn validate_session_endpoint(address: String, allow_zero_port: bool) -> SessionResult<String> {
    let address = super::endpoint(&address, allow_zero_port)?;
    if let std::net::SocketAddr::V6(v6) = address
        && v6.ip().is_unicast_link_local()
        && v6.scope_id() == 0
    {
        return Err(SessionError::Invalid);
    }
    Ok(address.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_facade_rejects_public_wildcard_names_and_unscoped_link_local() {
        for address in [
            "0.0.0.0:8",
            "[::]:8",
            "example.com:8",
            "8.8.8.8:8",
            "[fe80::1]:8",
            "127.0.0.1:0",
        ] {
            assert!(validate_session_endpoint(address.into(), false).is_err());
        }
        assert_eq!(
            validate_session_endpoint("[fe80::1%7]:8".into(), false)
                .ok()
                .as_deref(),
            Some("[fe80::1%7]:8")
        );
        assert!(validate_session_endpoint("127.0.0.1:0".into(), true).is_ok());
    }
}
