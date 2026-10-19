//! Bounded reconnect selection over caller-approved addresses. Every candidate
//! performs the ordinary pinned-TLS and application handshake; a winning socket
//! is not authority for a different project, peer or control grant.
use super::{AppCarrier, SessionError, SessionResult};
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use tokio::task::JoinSet;
use vw_model::DeviceId;
use vw_net::{
    CarrierIo, LocalHello, SecureConnection,
    carrier::{QuicCarrier, TcpCarrier},
};

// Selection is only the connect/Hello interval, not the later checkpoint or
// project sync interval. It does not claim the Phase 3 two-second carrier gate.
const DIAL_WINDOW: Duration = Duration::from_millis(1_800);

pub(super) async fn connect(
    addresses: &[(AppCarrier, SocketAddr)],
    manager: &vw_net::pairing::PairingManager,
    peer: &DeviceId,
    local: &LocalHello,
) -> SessionResult<(AppCarrier, SecureConnection)> {
    let candidates = addresses.iter().map(|&(carrier, address)| {
        let manager = manager.clone();
        let peer = peer.clone();
        let local = local.clone();
        (carrier, async move {
            // Preserve the configured tether/Wi-Fi/ADB preference when healthy
            // routes answer at similar speed, while probing fallbacks promptly.
            tokio::time::sleep(Duration::from_millis(u64::from(carrier.rank()) * 100)).await;
            // Re-evaluate live trust for every attempt. Dropping a losing task
            // never promotes it to a session or alters durable OPS/outbox data.
            let tls = manager.pinned_tls(&peer)?;
            let io = if carrier == AppCarrier::TcpAdb {
                TcpCarrier::connect(address, &tls).await.map(CarrierIo::Tcp)
            } else {
                let bind = SocketAddr::new(
                    if address.is_ipv4() {
                        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
                    } else {
                        IpAddr::V6(Ipv6Addr::UNSPECIFIED)
                    },
                    0,
                );
                QuicCarrier::connect(bind, address, &tls)
                    .await
                    .map(CarrierIo::Quic)
            }?;
            SecureConnection::client(io, &local)
                .await
                .map_err(SessionError::from)
        })
    });
    first_authenticated(candidates).await
}

async fn first_authenticated<T, F>(
    candidates: impl IntoIterator<Item = (AppCarrier, F)>,
) -> SessionResult<(AppCarrier, T)>
where
    T: Send + 'static,
    F: Future<Output = SessionResult<T>> + Send + 'static,
{
    let mut attempts = JoinSet::new();
    for (carrier, attempt) in candidates {
        attempts.spawn(async move { (carrier, attempt.await) });
    }
    if attempts.is_empty() {
        return Err(SessionError::Invalid);
    }
    let deadline = tokio::time::Instant::now() + DIAL_WINDOW;
    let mut failure = SessionError::Transport;
    let result = loop {
        tokio::select! {
            biased;
            joined = attempts.join_next() => match joined {
                None => break Err(failure),
                Some(Ok((carrier, Ok(connection)))) => break Ok((carrier, connection)),
                Some(Ok((_, Err(error)))) => {
                    // Preserve security and protected-storage failures when all
                    // routes fail; an unrelated transport failure cannot mask them.
                    if matches!(
                        error,
                        SessionError::Authentication | SessionError::Storage | SessionError::Closed
                    ) || !matches!(
                        failure,
                        SessionError::Authentication | SessionError::Storage | SessionError::Closed
                    ) {
                        failure = error;
                    }
                }
                Some(Err(_)) => failure = SessionError::Worker,
            },
            _ = tokio::time::sleep_until(deadline) => {
                break Err(if matches!(failure, SessionError::Authentication | SessionError::Storage | SessionError::Closed) {
                    failure
                } else { SessionError::Timeout });
            },
        }
    };
    attempts.abort_all();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn ready_authenticated_fallback_does_not_wait_for_stalled_preferred_route() {
        let (hold, stalled) = oneshot::channel::<()>();
        let preferred = async move {
            let _hold = hold;
            let _ = stalled.await;
            Err::<u8, _>(SessionError::Transport)
        };
        let fallback = async { Ok::<_, SessionError>(7_u8) };
        // Boxed futures allow the two deliberately different test schedules.
        let attempts: Vec<(
            _,
            std::pin::Pin<Box<dyn Future<Output = SessionResult<u8>> + Send>>,
        )> = vec![
            (AppCarrier::QuicTether, Box::pin(preferred)),
            (AppCarrier::QuicWifi, Box::pin(fallback)),
        ];
        assert!(matches!(
            first_authenticated(attempts).await,
            Ok((AppCarrier::QuicWifi, 7))
        ));
    }

    #[tokio::test]
    async fn failures_never_become_a_session_and_authentication_is_visible() {
        let attempts: Vec<(
            _,
            std::pin::Pin<Box<dyn Future<Output = SessionResult<u8>> + Send>>,
        )> = vec![
            (
                AppCarrier::QuicTether,
                Box::pin(async { Err(SessionError::Authentication) }),
            ),
            (
                AppCarrier::TcpAdb,
                Box::pin(async { Err(SessionError::Transport) }),
            ),
        ];
        assert!(matches!(
            first_authenticated(attempts).await,
            Err(SessionError::Authentication)
        ));
    }

    #[tokio::test]
    async fn all_stalled_candidates_end_in_a_bounded_retry_without_a_session() {
        let attempts = [(
            AppCarrier::QuicTether,
            std::future::pending::<SessionResult<u8>>(),
        )];
        assert!(matches!(
            first_authenticated(attempts).await,
            Err(SessionError::Timeout)
        ));
    }
}
