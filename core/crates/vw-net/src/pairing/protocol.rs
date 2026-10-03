use super::{
    CodeDisplay, Fingerprint, PairingChannel, PairingError, PairingListener, PairingManager,
    QrPayload, Result,
    offer::{OfferKind, Reservation},
    pake::{ConfirmationKey, Exchange, Transcript, qr_proof, verify_qr},
    tls::Wire,
};
use std::{fmt, net::SocketAddr};
use vw_model::DeviceId;
use vw_proto::{Message, v1};

const CLIENT_KEY: &[u8] = b"client-key-confirmed";
const SERVER_KEY: &[u8] = b"server-key-confirmed";
const CLIENT_ACCEPT: &[u8] = b"client-user-confirmed-fingerprint";
const SERVER_ACCEPT: &[u8] = b"server-user-confirmed-fingerprint";

/// The UI creates this only after the user explicitly compares and accepts the
/// full fingerprint displayed on BOTH devices. There is no automatic accept.
pub struct FingerprintConfirmation(Fingerprint);
impl FingerprintConfirmation {
    pub fn user_confirmed(displayed_fingerprint: &str) -> Result<Self> {
        Ok(Self(Fingerprint::parse(displayed_fingerprint)?))
    }
}
impl fmt::Debug for FingerprintConfirmation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FingerprintConfirmation([REDACTED])")
    }
}

pub struct ClientPairing;
impl ClientPairing {
    /// QR possession authorizes this one peer. The QR cert pin and HMAC are bound
    /// to this actual TLS 1.3 session; no caller-supplied exporter is accepted.
    pub async fn qr(
        manager: PairingManager,
        qr: &QrPayload,
        address: SocketAddr,
    ) -> Result<DeviceId> {
        let mut channel = PairingChannel::connect_qr(&manager, qr, address).await?;
        let identity = manager.identity()?;
        channel
            .send(Wire::Proof(announcement(
                &identity,
                qr_proof(qr.secret.expose(), channel.exporter())?,
            )))
            .await?;
        let result = match channel.receive(false).await? {
            Wire::Result(result) => result,
            _ => return Err(PairingError::Authentication),
        };
        if !result.paired {
            return Err(remote_error(&result.reason));
        }
        if !result.reason.is_empty()
            || result.certificate_der != channel.peer_certificate()
            || Fingerprint::parse(&result.pc_fingerprint_sha256)? != *qr.fingerprint()
        {
            return Err(PairingError::Authentication);
        }
        qr.validate(manager.clock.now_ms()?)?;
        manager.pin_confirmed_server(qr.device(), channel.peer_certificate())?;
        Ok(qr.device().clone())
    }
    /// Discovery only supplies an address; authentication is the exporter-bound
    /// PAKE followed by two explicit fingerprint confirmations.
    pub async fn code(
        manager: PairingManager,
        code: &CodeDisplay,
        address: SocketAddr,
    ) -> Result<PendingPairing> {
        let mut channel = PairingChannel::connect_code(&manager, address).await?;
        let identity = manager.identity()?;
        send_step(
            &mut channel,
            0,
            announcement(&identity, Vec::new()).encode_to_vec(),
        )
        .await?;
        let server = parse_announcement(
            &receive_step(&mut channel, 1, false).await?,
            channel.peer_certificate(),
        )?;
        let transcript = Transcript::new(
            channel.exporter(),
            identity.device(),
            &server,
            channel.local_certificate(),
            channel.peer_certificate(),
        );
        let (exchange, public) = Exchange::start(code.bytes(), transcript, true)?;
        send_step(&mut channel, 2, public).await?;
        let key = exchange.finish(&receive_step(&mut channel, 3, false).await?)?;
        send_step(&mut channel, 4, key.proof(CLIENT_KEY)).await?;
        key.verify(SERVER_KEY, &receive_step(&mut channel, 5, false).await?)?;
        let fingerprint = Fingerprint::certificate(channel.peer_certificate())?;
        Ok(PendingPairing {
            manager,
            channel,
            peer: server,
            fingerprint,
            key,
            reservation: None,
        })
    }
}

/// QR completes immediately. The code path produces a pending UI confirmation.
pub enum PairingHost {
    Paired(DeviceId),
    AwaitingConfirmation(Box<PendingPairing>),
}
impl PairingHost {
    pub async fn accept(manager: PairingManager, listener: &PairingListener) -> Result<Self> {
        Self::on_channel(manager, listener.accept().await?).await
    }
    pub async fn on_channel(manager: PairingManager, mut channel: PairingChannel) -> Result<Self> {
        let first = channel.receive(false).await?;
        match first {
            Wire::Proof(proof) => {
                // Persist the guess reservation before verification or response.
                let reservation = manager.reserve(OfferKind::Qr)?;
                let peer = validate_announcement(&proof, channel.peer_certificate(), false)?;
                verify_qr(reservation.secret.expose(), channel.exporter(), &proof.hmac)?;
                manager.complete(&reservation, &peer, channel.peer_certificate())?;
                let identity = manager.identity()?;
                channel
                    .send(Wire::Result(v1::PairingResult {
                        paired: true,
                        reason: String::new(),
                        pc_fingerprint_sha256: identity.fingerprint()?.display_hex(),
                        certificate_der: identity.certificate().to_vec(),
                    }))
                    .await?;
                Ok(Self::Paired(peer))
            }
            Wire::Pake(first) if first.step == 0 => {
                let reservation = manager.reserve(OfferKind::Code)?;
                let peer = parse_announcement(&first.payload, channel.peer_certificate())?;
                let identity = manager.identity()?;
                send_step(
                    &mut channel,
                    1,
                    announcement(&identity, Vec::new()).encode_to_vec(),
                )
                .await?;
                let transcript = Transcript::new(
                    channel.exporter(),
                    &peer,
                    identity.device(),
                    channel.peer_certificate(),
                    channel.local_certificate(),
                );
                let (exchange, public) =
                    Exchange::start(reservation.secret.expose(), transcript, false)?;
                let client_public = receive_step(&mut channel, 2, false).await?;
                send_step(&mut channel, 3, public).await?;
                let key = exchange.finish(&client_public)?;
                key.verify(CLIENT_KEY, &receive_step(&mut channel, 4, false).await?)?;
                send_step(&mut channel, 5, key.proof(SERVER_KEY)).await?;
                let fingerprint = identity.fingerprint()?;
                Ok(Self::AwaitingConfirmation(Box::new(PendingPairing {
                    manager,
                    channel,
                    peer,
                    fingerprint,
                    key,
                    reservation: Some(reservation),
                })))
            }
            _ => Err(PairingError::Authentication),
        }
    }
}
impl fmt::Debug for PairingHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Paired(_) => "PairingHost::Paired([REDACTED])",
            Self::AwaitingConfirmation(_) => "PairingHost::AwaitingConfirmation([REDACTED])",
        })
    }
}

/// Consuming this value without confirmation drops the channel without trust.
/// Only the PC fingerprint is displayed on both sides. Trust is persisted before
/// reporting success. A lost final acknowledgement can leave one side paired;
/// a fresh, explicitly authorized offer is then required for a retry.
pub struct PendingPairing {
    manager: PairingManager,
    channel: PairingChannel,
    peer: DeviceId,
    fingerprint: Fingerprint,
    key: ConfirmationKey,
    reservation: Option<Reservation>,
}
impl PendingPairing {
    pub const fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }
    pub async fn confirm(mut self, confirmation: FingerprintConfirmation) -> Result<DeviceId> {
        if confirmation.0 != self.fingerprint {
            return Err(PairingError::Declined);
        }
        if let Some(reservation) = &self.reservation {
            self.key.verify(
                CLIENT_ACCEPT,
                &receive_step(&mut self.channel, 6, true).await?,
            )?;
            self.manager
                .complete(reservation, &self.peer, self.channel.peer_certificate())?;
            send_step(&mut self.channel, 7, self.key.proof(SERVER_ACCEPT)).await?;
        } else {
            send_step(&mut self.channel, 6, self.key.proof(CLIENT_ACCEPT)).await?;
            self.key.verify(
                SERVER_ACCEPT,
                &receive_step(&mut self.channel, 7, true).await?,
            )?;
            self.manager
                .pin_confirmed_server(&self.peer, self.channel.peer_certificate())?;
        }
        Ok(self.peer)
    }
    pub async fn decline(mut self) -> Result<()> {
        self.channel
            .send(Wire::Result(v1::PairingResult {
                paired: false,
                reason: "bad_proof".into(),
                pc_fingerprint_sha256: String::new(),
                certificate_der: Vec::new(),
            }))
            .await?;
        self.channel.close().await
    }
}
impl fmt::Debug for PendingPairing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingPairing([REDACTED])")
    }
}
fn announcement(identity: &super::DeviceIdentity, hmac: Vec<u8>) -> v1::PairingProof {
    v1::PairingProof {
        hmac,
        device_id: identity.device().to_string(),
        platform: if cfg!(target_os = "android") {
            v1::Platform::Android as i32
        } else {
            v1::Platform::Windows as i32
        },
        device_label: String::new(),
        certificate_der: identity.certificate().to_vec(),
    }
}
fn parse_announcement(bytes: &[u8], actual_certificate: &[u8]) -> Result<DeviceId> {
    if bytes.len() > 17 * 1024 {
        return Err(PairingError::Invalid("pairing identity size"));
    }
    let proof = v1::PairingProof::decode(bytes).map_err(|_| PairingError::Authentication)?;
    validate_announcement(&proof, actual_certificate, true)
}
fn validate_announcement(
    proof: &v1::PairingProof,
    actual_certificate: &[u8],
    no_mac: bool,
) -> Result<DeviceId> {
    if proof.certificate_der != actual_certificate
        || !proof.device_label.is_empty()
        || !matches!(
            v1::Platform::try_from(proof.platform),
            Ok(v1::Platform::Android | v1::Platform::Windows)
        )
        || (no_mac && !proof.hmac.is_empty())
        || (!no_mac && proof.hmac.len() != 32)
    {
        return Err(PairingError::Authentication);
    }
    DeviceId::try_from(proof.device_id.clone()).map_err(|_| PairingError::Authentication)
}
async fn send_step(channel: &mut PairingChannel, step: u32, payload: Vec<u8>) -> Result<()> {
    channel
        .send(Wire::Pake(v1::PakeMessage { step, payload }))
        .await
}
async fn receive_step(
    channel: &mut PairingChannel,
    step: u32,
    confirmation: bool,
) -> Result<Vec<u8>> {
    match channel.receive(confirmation).await? {
        Wire::Pake(message) if message.step == step => Ok(message.payload),
        Wire::Result(result) if !result.paired => Err(remote_error(&result.reason)),
        _ => Err(PairingError::Authentication),
    }
}
fn remote_error(reason: &str) -> PairingError {
    match reason {
        "expired" => PairingError::Expired,
        "used" => PairingError::Used,
        "locked_out" => PairingError::LockedOut,
        _ => PairingError::Authentication,
    }
}
