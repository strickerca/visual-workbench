//! SPAKE2 uses the pinned Ed25519 implementation, with an application transcript and
//! directional key confirmation. The eight digits are never sent or hashed into
//! an independently testable verifier. Only the two 33-byte PAKE group messages
//! precede key confirmation. See the dependency audit limitations in the ADR.
use super::{PairingError, Result};
use ring::{digest, hmac};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use vw_model::DeviceId;
use zeroize::Zeroizing;

pub(crate) const MESSAGE_LEN: usize = 33;

pub(crate) struct Transcript {
    client: Zeroizing<Vec<u8>>,
    server: Zeroizing<Vec<u8>>,
    context: Zeroizing<Vec<u8>>,
}
impl Transcript {
    pub fn new(
        exporter: &[u8; 32],
        client: &DeviceId,
        server: &DeviceId,
        client_certificate: &[u8],
        server_certificate: &[u8],
    ) -> Self {
        let mut context = b"visual-workbench-pairing/spake2/1\0".to_vec();
        context.extend_from_slice(exporter);
        let client_fingerprint = digest::digest(&digest::SHA256, client_certificate);
        let server_fingerprint = digest::digest(&digest::SHA256, server_certificate);
        for bytes in [
            client.as_str().as_bytes(),
            server.as_str().as_bytes(),
            client_fingerprint.as_ref(),
            server_fingerprint.as_ref(),
        ] {
            context.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            context.extend_from_slice(bytes);
        }
        let mut client_identity = context.clone();
        client_identity.extend_from_slice(b"client");
        let mut server_identity = context.clone();
        server_identity.extend_from_slice(b"server");
        Self {
            client: Zeroizing::new(client_identity),
            server: Zeroizing::new(server_identity),
            context: Zeroizing::new(context),
        }
    }
}

pub(crate) struct Exchange {
    state: Spake2<Ed25519Group>,
    transcript: Transcript,
    own_message: Vec<u8>,
    client: bool,
}
impl Exchange {
    pub fn start(code: &[u8], transcript: Transcript, client: bool) -> Result<(Self, Vec<u8>)> {
        if code.len() != 8 || !code.iter().all(u8::is_ascii_digit) {
            return Err(PairingError::Invalid("8-digit code"));
        }
        let password = Password::new(code);
        let client_identity = Identity::new(&transcript.client);
        let server_identity = Identity::new(&transcript.server);
        let (state, message) = if client {
            Spake2::<Ed25519Group>::start_a(&password, &client_identity, &server_identity)
        } else {
            Spake2::<Ed25519Group>::start_b(&password, &client_identity, &server_identity)
        };
        if message.len() != MESSAGE_LEN {
            return Err(PairingError::Authentication);
        }
        Ok((
            Self {
                state,
                transcript,
                own_message: message.clone(),
                client,
            },
            message,
        ))
    }
    pub fn finish(self, remote: &[u8]) -> Result<ConfirmationKey> {
        if remote.len() != MESSAGE_LEN || remote == self.own_message {
            return Err(PairingError::Authentication);
        }
        let shared = Zeroizing::new(
            self.state
                .finish(remote)
                .map_err(|_| PairingError::Authentication)?,
        );
        let mut context = self.transcript.context;
        if self.client {
            context.extend_from_slice(&self.own_message);
            context.extend_from_slice(remote);
        } else {
            context.extend_from_slice(remote);
            context.extend_from_slice(&self.own_message);
        }
        Ok(ConfirmationKey {
            key: hmac::Key::new(hmac::HMAC_SHA256, &shared),
            context,
        })
    }
}

pub(crate) struct ConfirmationKey {
    key: hmac::Key,
    context: Zeroizing<Vec<u8>>,
}
impl ConfirmationKey {
    pub fn proof(&self, role: &[u8]) -> Vec<u8> {
        let mut context = hmac::Context::with_key(&self.key);
        context.update(&self.context);
        context.update(role);
        context.sign().as_ref().to_vec()
    }
    pub fn verify(&self, role: &[u8], proof: &[u8]) -> Result<()> {
        if proof.len() != 32 {
            return Err(PairingError::Authentication);
        }
        let mut message = self.context.clone();
        message.extend_from_slice(role);
        hmac::verify(&self.key, &message, proof).map_err(|_| PairingError::Authentication)
    }
}

pub(crate) fn qr_proof(secret: &[u8], exporter: &[u8; 32]) -> Result<Vec<u8>> {
    if secret.len() != 16 {
        return Err(PairingError::Authentication);
    }
    Ok(
        hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, secret), exporter)
            .as_ref()
            .to_vec(),
    )
}
pub(crate) fn verify_qr(secret: &[u8], exporter: &[u8; 32], proof: &[u8]) -> Result<()> {
    if secret.len() != 16 || proof.len() != 32 {
        return Err(PairingError::Authentication);
    }
    hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, secret), exporter, proof)
        .map_err(|_| PairingError::Authentication)
}
