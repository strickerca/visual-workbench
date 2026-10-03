use super::*;
use super::{
    offer::{OfferKind, PairingClock},
    pake::{Exchange, Transcript, qr_proof, verify_qr},
    tls::Wire,
};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use vw_model::DeviceId;
use vw_proto::v1;

struct Clock(AtomicU64);
impl PairingClock for Clock {
    fn now_ms(&self) -> Result<u64> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}
impl Clock {
    fn set(&self, now: u64) {
        self.0.store(now, Ordering::SeqCst);
    }
}
fn manager() -> Result<(PairingManager, Arc<Clock>)> {
    let clock = Arc::new(Clock(AtomicU64::new(1_000_000)));
    let store = Arc::new(InMemoryTrustStore::new(DeviceIdentity::generate()?)?);
    Ok((
        PairingManager {
            store,
            clock: clock.clone(),
        },
        clock,
    ))
}
fn endpoint() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 43127))
}
fn proof(identity: &DeviceIdentity, hmac: Vec<u8>) -> v1::PairingProof {
    v1::PairingProof {
        hmac,
        device_id: identity.device().to_string(),
        platform: v1::Platform::Android as i32,
        device_label: String::new(),
        certificate_der: identity.certificate().to_vec(),
    }
}
fn transcript(exporter: [u8; 32], client: &DeviceIdentity, server: &DeviceIdentity) -> Transcript {
    Transcript::new(
        &exporter,
        client.device(),
        server.device(),
        client.certificate(),
        server.certificate(),
    )
}
fn pending(host: PairingHost) -> Result<PendingPairing> {
    match host {
        PairingHost::AwaitingConfirmation(value) => Ok(*value),
        _ => Err(PairingError::Authentication),
    }
}

#[test]
fn identity_validation_rejects_a_certificate_with_another_private_key() -> Result<()> {
    let mut identity = DeviceIdentity::generate()?;
    identity.key = DeviceIdentity::generate()?.key.clone();
    assert!(identity.validate().is_err());
    assert!(identity.transport_identity().is_err());
    Ok(())
}

#[test]
fn qr_hmac_is_exact_and_rejects_cross_exporter_or_secret() -> Result<()> {
    let secret = [0x24; 16];
    let exporter = [0x38; 32];
    let proof = qr_proof(&secret, &exporter)?;
    verify_qr(&secret, &exporter, &proof)?;
    assert!(verify_qr(&secret, &[0x39; 32], &proof).is_err());
    assert!(verify_qr(&[0x25; 16], &exporter, &proof).is_err());
    assert!(verify_qr(&secret, &exporter, &proof[..31]).is_err());
    assert_eq!(
        proof,
        ring::hmac::sign(
            &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &secret),
            &exporter
        )
        .as_ref()
    );
    Ok(())
}
#[test]
fn pake_confirms_only_matching_code_exporter_identity_and_direction() -> Result<()> {
    let a = DeviceIdentity::generate()?;
    let b = DeviceIdentity::generate()?;
    for mismatch in 0..4 {
        let (left, left_public) = Exchange::start(b"12345678", transcript([1; 32], &a, &b), true)?;
        let context = if mismatch == 2 {
            transcript([2; 32], &a, &b)
        } else if mismatch == 3 {
            transcript([1; 32], &b, &a)
        } else {
            transcript([1; 32], &a, &b)
        };
        let (right, right_public) = Exchange::start(
            if mismatch == 1 {
                b"12345679"
            } else {
                b"12345678"
            },
            context,
            false,
        )?;
        assert_eq!(left_public.len(), 33);
        assert_eq!(right_public.len(), 33);
        let left = left.finish(&right_public)?;
        let right = right.finish(&left_public)?;
        if mismatch == 0 {
            right.verify(b"client", &left.proof(b"client"))?;
            assert!(right.verify(b"server", &left.proof(b"client")).is_err());
        } else {
            assert!(right.verify(b"client", &left.proof(b"client")).is_err());
        }
    }
    Ok(())
}
#[test]
fn pake_rejects_reflection_truncation_and_invalid_code() -> Result<()> {
    let a = DeviceIdentity::generate()?;
    let b = DeviceIdentity::generate()?;
    let (exchange, message) = Exchange::start(b"00000000", transcript([1; 32], &a, &b), true)?;
    assert!(exchange.finish(&message).is_err());
    let (exchange, _) = Exchange::start(b"00000000", transcript([1; 32], &a, &b), true)?;
    assert!(exchange.finish(&[0; 32]).is_err());
    assert!(Exchange::start(b"secret", transcript([1; 32], &a, &b), true).is_err());
    Ok(())
}
#[test]
fn qr_expiry_and_single_use_are_durable() -> Result<()> {
    let (host, clock) = manager()?;
    let peer = DeviceIdentity::generate()?;
    let qr = host.issue_qr(vec![endpoint()])?;
    let reservation = host.reserve(OfferKind::Qr)?;
    host.complete(&reservation, peer.device(), peer.certificate())?;
    assert!(matches!(
        host.reserve(OfferKind::Qr),
        Err(PairingError::Used)
    ));
    assert!(
        host.complete(&reservation, peer.device(), peer.certificate())
            .is_err()
    );
    let encoded = host.store.load()?.encode_for_protection()?;
    let restarted = PairingManager {
        store: Arc::new(InMemoryTrustStore::from_state(
            TrustState::decode_unprotected(encoded.expose())?,
        )?),
        clock: clock.clone(),
    };
    assert!(matches!(
        restarted.reserve(OfferKind::Qr),
        Err(PairingError::Used)
    ));
    let _ = host.issue_qr(vec![endpoint()])?;
    clock.set(qr.expires_at_ms());
    assert!(matches!(
        host.reserve(OfferKind::Qr),
        Err(PairingError::Expired)
    ));
    assert!(qr.validate(qr.expires_at_ms()).is_err());
    Ok(())
}
#[test]
fn fifth_incomplete_attempt_locks_through_restart_and_new_offers() -> Result<()> {
    let (host, clock) = manager()?;
    let _ = host.issue_code()?;
    for _ in 0..4 {
        let _ = host.reserve(OfferKind::Code)?;
        let _ = host.issue_code()?;
    }
    let _ = host.reserve(OfferKind::Code)?;
    assert!(host.is_locked()?);
    assert!(matches!(host.issue_code(), Err(PairingError::LockedOut)));
    let state = host.store.load()?;
    let restarted = PairingManager {
        store: Arc::new(InMemoryTrustStore::from_state(state.clone())?),
        clock: clock.clone(),
    };
    assert!(matches!(
        restarted.reserve(OfferKind::Code),
        Err(PairingError::LockedOut)
    ));
    clock.set(state.locked_until_ms() - 1);
    assert!(restarted.is_locked()?);
    clock.set(state.locked_until_ms());
    assert!(!restarted.is_locked()?);
    // The old five-minute offer expires during the ten-minute lock.
    assert!(matches!(
        restarted.reserve(OfferKind::Code),
        Err(PairingError::Expired)
    ));
    let _ = restarted.issue_code()?;
    assert!(restarted.reserve(OfferKind::Code).is_ok());
    Ok(())
}
#[test]
fn successful_fifth_attempt_resets_lockout_but_stale_reservation_cannot_pin() -> Result<()> {
    let (host, _) = manager()?;
    let peer = DeviceIdentity::generate()?;
    let _ = host.issue_code()?;
    let first = host.reserve(OfferKind::Code)?;
    for _ in 0..3 {
        let _ = host.reserve(OfferKind::Code)?;
    }
    let fifth = host.reserve(OfferKind::Code)?;
    assert!(matches!(
        host.complete(&first, peer.device(), peer.certificate()),
        Err(PairingError::Used)
    ));
    host.complete(&fifth, peer.device(), peer.certificate())?;
    assert!(!host.is_locked()?);
    assert_eq!(host.store.load()?.peers().count(), 1);
    Ok(())
}
#[test]
fn clock_rollback_fails_closed_and_expiry_boundary_is_exclusive() -> Result<()> {
    let (host, clock) = manager()?;
    let _ = host.issue_code()?;
    clock.set(999_999);
    assert!(matches!(
        host.reserve(OfferKind::Code),
        Err(PairingError::ClockRollback)
    ));
    assert!(matches!(
        host.issue_code(),
        Err(PairingError::ClockRollback)
    ));
    assert!(matches!(host.is_locked(), Err(PairingError::ClockRollback)));
    clock.set(1_300_000);
    assert!(matches!(
        host.reserve(OfferKind::Code),
        Err(PairingError::Expired)
    ));
    Ok(())
}
#[test]
fn trust_rejects_identity_aliases_revoked_peers_and_capacity_eviction() -> Result<()> {
    let identity = DeviceIdentity::generate()?;
    let mut state = TrustState::new(identity.clone())?;
    assert!(
        state
            .pin(identity.device().clone(), identity.certificate(), 1)
            .is_err()
    );
    let peer = DeviceIdentity::generate()?;
    state.pin(peer.device().clone(), peer.certificate(), 1)?;
    assert!(
        state
            .pin(DeviceId::from_bytes([0x77; 16]), peer.certificate(), 1)
            .is_err()
    );
    state
        .peers
        .get_mut(peer.device())
        .ok_or(PairingError::Storage)?
        .revoked_at_ms = Some(2);
    assert!(
        state
            .pin(peer.device().clone(), peer.certificate(), 3)
            .is_err()
    );
    assert!(state.trusted_peer(peer.device()).is_err());
    assert!(state.authorize(peer.device(), peer.certificate()).is_err());
    for _ in 1..store::MAX_PEERS {
        let other = DeviceIdentity::generate()?;
        state.pin(other.device().clone(), other.certificate(), 3)?;
    }
    let overflow = DeviceIdentity::generate()?;
    assert!(matches!(
        state.pin(overflow.device().clone(), overflow.certificate(), 4),
        Err(PairingError::Capacity)
    ));
    assert_eq!(state.peers().filter(|peer| peer.is_revoked()).count(), 1);
    Ok(())
}
#[test]
fn credential_debug_output_is_redacted_and_decoders_are_bounded() -> Result<()> {
    let (host, _) = manager()?;
    let identity = host.identity()?;
    let qr = host.issue_qr(vec![endpoint()])?;
    let code = host.issue_code()?;
    let state = host.store.load()?;
    let output = format!(
        "{host:?} {identity:?} {qr:?} {code:?} {state:?} {:?}",
        identity.fingerprint()?
    );
    assert!(!output.contains(identity.device().as_str()));
    assert!(!output.contains(code.digits_for_display()?));
    assert!(!output.contains(&identity.fingerprint()?.display_hex()));
    assert!(!output.contains("127.0.0.1"));
    assert!(QrPayload::decode_qr(&vec![0; 4097]).is_err());
    assert!(TrustState::decode_unprotected(&vec![0; MAX_TRUST_STATE_BYTES + 1]).is_err());
    assert!(CodeDisplay::parse_for_entry("1234567").is_err());
    assert!(CodeDisplay::parse_for_entry("1234567x").is_err());
    assert!(
        host.issue_qr(vec![SocketAddr::from(([0, 0, 0, 0], 5))])
            .is_err()
    );
    Ok(())
}

struct Callback {
    data: Mutex<Option<SecretBytes>>,
    fail: AtomicBool,
}
impl TrustStoreCallback for Callback {
    fn load_app_state(&self) -> Result<Option<Vec<u8>>> {
        Ok(self
            .data
            .lock()
            .map_err(|_| PairingError::Storage)?
            .as_ref()
            .map(|bytes| bytes.expose().to_vec()))
    }
    fn compare_exchange_app_state(&self, expected: u64, replacement: Vec<u8>) -> Result<bool> {
        let replacement = SecretBytes::new(replacement);
        if self.fail.load(Ordering::SeqCst) {
            return Err(PairingError::Storage);
        }
        let mut data = self.data.lock().map_err(|_| PairingError::Storage)?;
        let current = data
            .as_ref()
            .map(|bytes| TrustState::decode_unprotected(bytes.expose()).map(|s| s.revision()))
            .transpose()?
            .unwrap_or(0);
        if current != expected {
            return Ok(false);
        }
        *data = Some(replacement);
        Ok(true)
    }
}
#[test]
fn platform_callback_reopens_identity_and_storage_failure_cannot_reset_attempts() -> Result<()> {
    let callback = Arc::new(Callback {
        data: Mutex::new(None),
        fail: AtomicBool::new(false),
    });
    let store = Arc::new(CallbackTrustStore::open(callback.clone())?);
    let first = store.load()?.identity();
    let manager = PairingManager::new(store)?;
    let _ = manager.issue_code()?;
    let _ = manager.reserve(OfferKind::Code)?;
    callback.fail.store(true, Ordering::SeqCst);
    assert!(matches!(
        manager.reserve(OfferKind::Code),
        Err(PairingError::Storage)
    ));
    callback.fail.store(false, Ordering::SeqCst);
    let reopened = CallbackTrustStore::open(callback)?;
    assert_eq!(reopened.load()?.identity().device(), first.device());
    assert_eq!(reopened.load()?.failures, 1);
    Ok(())
}
#[test]
fn concurrent_guesses_reserve_at_most_five_durable_slots() -> Result<()> {
    let (host, _) = manager()?;
    let _ = host.issue_code()?;
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..12)
            .map(|_| {
                let host = host.clone();
                scope.spawn(move || host.reserve(OfferKind::Code).is_ok())
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or(false))
            .collect::<Vec<_>>()
    });
    assert_eq!(results.into_iter().filter(|ok| *ok).count(), 5);
    assert_eq!(host.store.load()?.failures, 5);
    assert!(host.is_locked()?);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn real_tls_qr_pins_both_peers_and_rejects_reuse() -> Result<()> {
    let (host, _) = manager()?;
    let (client, _) = manager()?;
    let listener =
        PairingListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)), &host.identity()?).await?;
    let address = listener.local_addr()?;
    let qr = host.issue_qr(vec![address])?;
    let (a, b) = tokio::join!(
        PairingHost::accept(host.clone(), &listener),
        ClientPairing::qr(client.clone(), &qr, address)
    );
    assert!(matches!(a?, PairingHost::Paired(_)));
    assert_eq!(b?, *host.identity()?.device());
    assert!(host.pinned_tls(client.identity()?.device()).is_ok());
    assert!(client.pinned_tls(host.identity()?.device()).is_ok());
    let (a, b) = tokio::join!(
        PairingHost::accept(host.clone(), &listener),
        ClientPairing::qr(client.clone(), &qr, address)
    );
    assert!(matches!(a, Err(PairingError::Used)));
    assert!(b.is_err());
    Ok(())
}
#[tokio::test(flavor = "current_thread")]
async fn actual_tls_exporter_differs_per_connection_and_blocks_captured_qr_proof() -> Result<()> {
    let (host, _) = manager()?;
    let (client, _) = manager()?;
    let listener =
        PairingListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)), &host.identity()?).await?;
    let address = listener.local_addr()?;
    let qr = host.issue_qr(vec![address])?;
    let (client_one, server_one) = tokio::join!(
        PairingChannel::connect_qr(&client, &qr, address),
        listener.accept()
    );
    let client_one = client_one?;
    let server_one = server_one?;
    assert_eq!(client_one.exporter(), server_one.exporter());
    let captured = qr_proof(qr.secret.expose(), client_one.exporter())?;
    let old_exporter = *client_one.exporter();
    drop(client_one);
    drop(server_one);
    let (client_two, server_two) = tokio::join!(
        PairingChannel::connect_qr(&client, &qr, address),
        listener.accept()
    );
    let mut client_two = client_two?;
    assert_ne!(&old_exporter, client_two.exporter());
    client_two
        .send(Wire::Proof(proof(&client.identity()?, captured)))
        .await?;
    assert!(matches!(
        PairingHost::on_channel(host.clone(), server_two?).await,
        Err(PairingError::Authentication)
    ));
    assert_eq!(host.store.load()?.peers().count(), 0);
    Ok(())
}
#[tokio::test(flavor = "current_thread")]
async fn qr_rejects_wrong_pin_and_certificate_substitution() -> Result<()> {
    let (host, _) = manager()?;
    let (client, _) = manager()?;
    let listener =
        PairingListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)), &host.identity()?).await?;
    let address = listener.local_addr()?;
    let mut qr = host.issue_qr(vec![address])?;
    let correct = qr.certificate_sha256;
    qr.certificate_sha256 = DeviceIdentity::generate()?.fingerprint()?;
    let (a, b) = tokio::join!(
        PairingChannel::connect_qr(&client, &qr, address),
        listener.accept()
    );
    assert!(a.is_err());
    assert!(b.is_err());
    qr.certificate_sha256 = correct;
    let (a, b) = tokio::join!(
        PairingChannel::connect_qr(&client, &qr, address),
        listener.accept()
    );
    let mut channel = a?;
    let impostor = DeviceIdentity::generate()?;
    let message = proof(&impostor, qr_proof(qr.secret.expose(), channel.exporter())?);
    channel.send(Wire::Proof(message)).await?;
    assert!(matches!(
        PairingHost::on_channel(host.clone(), b?).await,
        Err(PairingError::Authentication)
    ));
    assert_eq!(host.store.load()?.peers().count(), 0);
    Ok(())
}
#[tokio::test(flavor = "current_thread")]
async fn real_code_flow_requires_both_explicit_confirmations() -> Result<()> {
    let (host, _) = manager()?;
    let (client, _) = manager()?;
    let code = host.issue_code()?;
    let listener =
        PairingListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)), &host.identity()?).await?;
    let address = listener.local_addr()?;
    let (a, b) = tokio::join!(
        PairingHost::accept(host.clone(), &listener),
        ClientPairing::code(client.clone(), &code, address)
    );
    let host_pending = pending(a?)?;
    let client_pending = b?;
    assert_eq!(host_pending.fingerprint(), client_pending.fingerprint());
    assert_eq!(host.store.load()?.peers().count(), 0);
    assert_eq!(client.store.load()?.peers().count(), 0);
    let displayed = host_pending.fingerprint().display_hex();
    let (a, b) = tokio::join!(
        host_pending.confirm(FingerprintConfirmation::user_confirmed(&displayed)?),
        client_pending.confirm(FingerprintConfirmation::user_confirmed(&displayed)?)
    );
    assert_eq!(a?, *client.identity()?.device());
    assert_eq!(b?, *host.identity()?.device());
    assert!(matches!(
        host.reserve(OfferKind::Code),
        Err(PairingError::Used)
    ));
    Ok(())
}
#[tokio::test(flavor = "current_thread")]
async fn decline_never_pins_and_wrong_code_spends_a_durable_guess() -> Result<()> {
    let (host, _) = manager()?;
    let (client, _) = manager()?;
    let code = host.issue_code()?;
    let listener =
        PairingListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)), &host.identity()?).await?;
    let address = listener.local_addr()?;
    let wrong = CodeDisplay::parse_for_entry(if code.digits_for_display()? == "00000000" {
        "00000001"
    } else {
        "00000000"
    })?;
    let (a, b) = tokio::join!(
        PairingHost::accept(host.clone(), &listener),
        ClientPairing::code(client.clone(), &wrong, address)
    );
    assert!(a.is_err());
    assert!(b.is_err());
    assert_eq!(host.store.load()?.failures, 1);
    let (a, b) = tokio::join!(
        PairingHost::accept(host.clone(), &listener),
        ClientPairing::code(client.clone(), &code, address)
    );
    let host_pending = pending(a?)?;
    let client_pending = b?;
    let displayed = client_pending.fingerprint().display_hex();
    let (a, b) = tokio::join!(
        host_pending.decline(),
        client_pending.confirm(FingerprintConfirmation::user_confirmed(&displayed)?)
    );
    let _ = a;
    assert!(b.is_err());
    assert_eq!(host.store.load()?.peers().count(), 0);
    assert_eq!(client.store.load()?.peers().count(), 0);
    Ok(())
}
#[tokio::test(flavor = "current_thread")]
async fn five_actual_wrong_code_exchanges_lock_out_the_sixth() -> Result<()> {
    let (host, _) = manager()?;
    let (client, _) = manager()?;
    let code = host.issue_code()?;
    let listener =
        PairingListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)), &host.identity()?).await?;
    let address = listener.local_addr()?;
    let wrong = CodeDisplay::parse_for_entry(if code.digits_for_display()? == "00000000" {
        "00000001"
    } else {
        "00000000"
    })?;
    for expected in 1..=5 {
        let (a, b) = tokio::join!(
            PairingHost::accept(host.clone(), &listener),
            ClientPairing::code(client.clone(), &wrong, address)
        );
        assert!(matches!(a, Err(PairingError::Authentication)));
        assert!(b.is_err());
        assert_eq!(host.store.load()?.failures, expected);
    }
    let (a, b) = tokio::join!(
        PairingHost::accept(host.clone(), &listener),
        ClientPairing::code(client.clone(), &code, address)
    );
    assert!(matches!(a, Err(PairingError::LockedOut)));
    assert!(b.is_err());
    assert!(host.is_locked()?);
    assert_eq!(host.store.load()?.peers().count(), 0);
    Ok(())
}
#[tokio::test(flavor = "current_thread")]
async fn expired_code_cannot_be_confirmed_after_pake() -> Result<()> {
    let (host, clock) = manager()?;
    let (client, _) = manager()?;
    let code = host.issue_code()?;
    let listener =
        PairingListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)), &host.identity()?).await?;
    let address = listener.local_addr()?;
    let (a, b) = tokio::join!(
        PairingHost::accept(host.clone(), &listener),
        ClientPairing::code(client.clone(), &code, address)
    );
    let a = pending(a?)?;
    let b = b?;
    let displayed = a.fingerprint().display_hex();
    clock.set(1_300_000);
    let (a, b) = tokio::join!(
        a.confirm(FingerprintConfirmation::user_confirmed(&displayed)?),
        b.confirm(FingerprintConfirmation::user_confirmed(&displayed)?)
    );
    assert!(matches!(a, Err(PairingError::Expired)));
    assert!(b.is_err());
    assert_eq!(host.store.load()?.peers().count(), 0);
    assert_eq!(client.store.load()?.peers().count(), 0);
    Ok(())
}
#[tokio::test(flavor = "current_thread")]
async fn ordinary_tls_config_rechecks_revocation_after_it_was_constructed() -> Result<()> {
    let (host, _) = manager()?;
    let (client, _) = manager()?;
    let host_identity = host.identity()?;
    let client_identity = client.identity()?;
    host.pin_confirmed_server(client_identity.device(), client_identity.certificate())?;
    client.pin_confirmed_server(host_identity.device(), host_identity.certificate())?;
    let host_tls = host.pinned_tls(client_identity.device())?;
    let client_tls = client.pinned_tls(host_identity.device())?;
    host.revoke(client_identity.device())?;
    assert!(host.pinned_tls(client_identity.device()).is_err());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .map_err(|_| PairingError::Connection)?;
    let address = listener
        .local_addr()
        .map_err(|_| PairingError::Connection)?;
    let (a, b) = tokio::join!(
        crate::carrier::TcpCarrier::accept(&listener, &host_tls),
        crate::carrier::TcpCarrier::connect(address, &client_tls)
    );
    assert!(a.is_err());
    // TLS 1.3 client may finish its local flight before receiving the server's
    // alert; no server carrier and thus no authenticated application session exists.
    drop(b);
    Ok(())
}
