//! Mutual certificate pinning for QUIC (ADR 0004, NET-3). Both peers present a
//! certificate. Each side accepts the other only if the fingerprint is in the local
//! trust list. There is no CA and no trust on first use.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use telescreen_core::ids::ComputerId;

use crate::network::TrustSource;
use crate::{Fingerprint, Identity};

pub(crate) const ALPN: &[u8] = b"telescreen/1";
pub(crate) const SERVER_NAME: &str = "telescreen";

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The dialer side: accept only a fingerprint that is trusted for the peer we dial.
#[derive(Debug)]
struct PinnedListener {
    peer: ComputerId,
    trust: TrustSource,
    provider: Arc<CryptoProvider>,
}

/// The listener side: accept only a fingerprint that is trusted for some computer.
/// The Hello message then must name one of these computers.
#[derive(Debug)]
struct PinnedDialer {
    trust: TrustSource,
    provider: Arc<CryptoProvider>,
}

fn refuse(fingerprint: Fingerprint) -> rustls::Error {
    rustls::Error::General(format!("fingerprint {fingerprint} is not trusted"))
}

impl ServerCertVerifier for PinnedListener {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fingerprint = Fingerprint::of(end_entity.as_ref());
        if self.trust.current().is_trusted(&self.peer, &fingerprint) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(refuse(fingerprint))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

impl ClientCertVerifier for PinnedDialer {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let fingerprint = Fingerprint::of(end_entity.as_ref());
        if self.trust.current().computers_for(&fingerprint).is_empty() {
            Err(refuse(fingerprint))
        } else {
            Ok(ClientCertVerified::assertion())
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub(crate) fn listener_config(
    identity: &Identity,
    trust: TrustSource,
) -> Result<rustls::ServerConfig, rustls::Error> {
    let provider = provider();
    let verifier = Arc::new(PinnedDialer {
        trust,
        provider: provider.clone(),
    });
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![identity.certificate.clone()], identity.key())?;
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(config)
}

pub(crate) fn dialer_config(
    identity: &Identity,
    peer: ComputerId,
    trust: TrustSource,
) -> Result<rustls::ClientConfig, rustls::Error> {
    let provider = provider();
    let verifier = Arc::new(PinnedListener {
        peer,
        trust,
        provider: provider.clone(),
    });
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(vec![identity.certificate.clone()], identity.key())?;
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(config)
}

/// The fingerprint of the certificate that the other peer presented.
pub(crate) fn peer_fingerprint(connection: &quinn::Connection) -> Option<Fingerprint> {
    let identity = connection.peer_identity()?;
    let certs = identity.downcast::<Vec<CertificateDer<'static>>>().ok()?;
    certs.first().map(|c| Fingerprint::of(c.as_ref()))
}
