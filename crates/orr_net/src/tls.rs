//! TLS setup for the QUIC backend.

use crate::config::NetConfig;
use crate::NetError;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;

const ALPN: &[u8] = b"orrery/1";

/// Server certificate source.
#[derive(Clone, Debug)]
pub enum QuicServerTls {
    /// Development: a self-signed certificate is generated at startup for these
    /// names (DNS names or IP literals). Read it back with
    /// `Endpoint::server_cert_der` and hand it to clients.
    SelfSigned { names: Vec<String> },
    /// Production: PEM certificate chain and PEM private key files.
    PemFiles { cert_chain: PathBuf, private_key: PathBuf },
}

impl QuicServerTls {
    /// Self-signed for `localhost`, `127.0.0.1` and `::1`.
    pub fn dev_localhost() -> Self {
        QuicServerTls::SelfSigned {
            names: vec!["localhost".into(), "127.0.0.1".into(), "::1".into()],
        }
    }
}

/// How a client decides to trust the server certificate.
#[derive(Clone, Debug)]
pub enum QuicTrust {
    /// Production: the Mozilla root set (`webpki-roots`).
    WebPki,
    /// Trust exactly this DER certificate (also checks the server name).
    CertDer(Vec<u8>),
    /// Trust the certificates in this PEM file (also checks the server name).
    PemFile(PathBuf),
    /// Trust only a certificate whose SHA-256 equals this (name is not checked).
    Sha256Fingerprint([u8; 32]),
    /// DEVELOPMENT ONLY. Accepts any certificate. Never use outside local testing:
    /// anyone on the network path can impersonate the server.
    DangerousSkipVerificationDevOnly,
}

pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub(crate) fn transport_config(cfg: &NetConfig, server: bool) -> Result<quinn::TransportConfig, NetError> {
    let mut t = quinn::TransportConfig::default();
    t.keep_alive_interval(Some(cfg.keepalive_interval));
    t.max_idle_timeout(Some(cfg.idle_timeout.try_into().map_err(NetError::new)?));
    // One bidirectional stream (the reliable channel), no unidirectional streams.
    t.max_concurrent_bidi_streams(if server { 1u32 } else { 0u32 }.into());
    t.max_concurrent_uni_streams(0u32.into());
    let dgram = cfg.max_message_size.min(u16::MAX as usize).max(1500);
    t.datagram_receive_buffer_size(Some(dgram * 256));
    t.datagram_send_buffer_size(dgram * 256);
    Ok(t)
}

pub(crate) fn build_quic_server_config(
    tls: &QuicServerTls,
    cfg: &NetConfig,
) -> Result<(quinn::ServerConfig, Option<Vec<u8>>), NetError> {
    let (chain, key, generated) = match tls {
        QuicServerTls::SelfSigned { names } => {
            let ck = rcgen::generate_simple_self_signed(names.clone()).map_err(NetError::new)?;
            let der = ck.cert.der().to_vec();
            let key = PrivateKeyDer::Pkcs8(ck.signing_key.serialize_der().into());
            (vec![CertificateDer::from(der.clone())], key, Some(der))
        }
        QuicServerTls::PemFiles { cert_chain, private_key } => {
            let chain = CertificateDer::pem_file_iter(cert_chain)
                .map_err(NetError::new)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(NetError::new)?;
            if chain.is_empty() {
                return Err(NetError("certificate file holds no certificate".into()));
            }
            let key = PrivateKeyDer::from_pem_file(private_key).map_err(NetError::new)?;
            (chain, key, None)
        }
    };
    let mut rc = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(NetError::new)?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(NetError::new)?;
    rc.alpn_protocols = vec![ALPN.to_vec()];
    let qc = QuicServerConfig::try_from(Arc::new(rc)).map_err(NetError::new)?;
    let mut sc = quinn::ServerConfig::with_crypto(Arc::new(qc));
    sc.transport_config(Arc::new(transport_config(cfg, true)?));
    Ok((sc, generated))
}

/// Builds the quinn client configuration. Public only so that tests can build
/// raw quinn clients with the same trust rules.
#[doc(hidden)]
pub fn build_quic_client_config(trust: &QuicTrust, cfg: &NetConfig) -> Result<quinn::ClientConfig, NetError> {
    let prov = provider();
    let builder = rustls::ClientConfig::builder_with_provider(prov.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(NetError::new)?;
    let mut rc = match trust {
        QuicTrust::WebPki => {
            let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        QuicTrust::CertDer(der) => {
            let mut roots = RootCertStore::empty();
            roots.add(CertificateDer::from(der.clone())).map_err(NetError::new)?;
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        QuicTrust::PemFile(path) => {
            let mut roots = RootCertStore::empty();
            for c in CertificateDer::pem_file_iter(path).map_err(NetError::new)? {
                roots.add(c.map_err(NetError::new)?).map_err(NetError::new)?;
            }
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        QuicTrust::Sha256Fingerprint(fp) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(LaxVerifier {
                pin: Some(*fp),
                algs: prov.signature_verification_algorithms,
            }))
            .with_no_client_auth(),
        QuicTrust::DangerousSkipVerificationDevOnly => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(LaxVerifier {
                pin: None,
                algs: prov.signature_verification_algorithms,
            }))
            .with_no_client_auth(),
    };
    rc.alpn_protocols = vec![ALPN.to_vec()];
    let qc = QuicClientConfig::try_from(Arc::new(rc)).map_err(NetError::new)?;
    let mut cc = quinn::ClientConfig::new(Arc::new(qc));
    cc.transport_config(Arc::new(transport_config(cfg, false)?));
    Ok(cc)
}

/// Verifier that pins a fingerprint, or accepts anything when `pin` is `None`.
/// Handshake signatures are still verified, so the server must own the key.
#[derive(Debug)]
struct LaxVerifier {
    pin: Option<[u8; 32]>,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for LaxVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.pin {
            Some(fp) if sha256(end_entity.as_ref()) != fp => {
                Err(rustls::Error::General("server certificate fingerprint mismatch".into()))
            }
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}
