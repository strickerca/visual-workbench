//! Additional authorization layered over rustls WebPKI and exact leaf pinning.
//! These wrappers never turn failed cryptographic verification into success.
use crate::{
    NetError, Result,
    carrier::{PeerAuthorization, TrustedPeer},
};
use rustls::{
    DigitallySignedStruct, DistinguishedName, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use std::{fmt, sync::Arc};

pub(crate) struct Authorization {
    pub peer: TrustedPeer,
    pub policy: Option<Arc<dyn PeerAuthorization>>,
}
impl Authorization {
    pub fn check(&self, certificate: &[u8]) -> Result<()> {
        if certificate != self.peer.certificate.as_ref() {
            return Err(NetError::Authentication);
        }
        if let Some(policy) = &self.policy {
            policy.authorize(&self.peer.device, certificate)?;
        }
        Ok(())
    }
    fn tls_check(
        &self,
        certificate: &CertificateDer<'_>,
    ) -> std::result::Result<(), rustls::Error> {
        self.check(certificate.as_ref())
            .map_err(|_| rustls::Error::General("peer is not authorized".into()))
    }
}
pub(crate) struct ServerVerifier {
    pub cryptographic: Arc<dyn ServerCertVerifier>,
    pub authorization: Arc<Authorization>,
}
impl fmt::Debug for ServerVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AuthorizedServerVerifier")
    }
}
impl ServerCertVerifier for ServerVerifier {
    fn verify_server_cert(
        &self,
        leaf: &CertificateDer<'_>,
        chain: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let result = self
            .cryptographic
            .verify_server_cert(leaf, chain, name, ocsp, now)?;
        self.authorization.tls_check(leaf)?;
        Ok(result)
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.cryptographic
            .verify_tls12_signature(message, cert, dss)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.cryptographic
            .verify_tls13_signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.cryptographic.supported_verify_schemes()
    }
}
pub(crate) struct ClientVerifier {
    pub cryptographic: Arc<dyn ClientCertVerifier>,
    pub authorization: Arc<Authorization>,
}
impl fmt::Debug for ClientVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AuthorizedClientVerifier")
    }
}
impl ClientCertVerifier for ClientVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        self.cryptographic.root_hint_subjects()
    }
    fn verify_client_cert(
        &self,
        leaf: &CertificateDer<'_>,
        chain: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        let result = self.cryptographic.verify_client_cert(leaf, chain, now)?;
        self.authorization.tls_check(leaf)?;
        Ok(result)
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.cryptographic
            .verify_tls12_signature(message, cert, dss)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.cryptographic
            .verify_tls13_signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.cryptographic.supported_verify_schemes()
    }
}
