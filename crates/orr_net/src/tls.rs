//! TLS setup shared by QUIC and secure WebSocket backends.

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

pub(crate) const ALPN: &[u8] = b"orrery/1";
/// ALPN of HTTP/3, which WebTransport runs on.
pub(crate) const ALPN_H3: &[u8] = b"h3";

/// Validity of a [`QuicServerTls::SelfSignedWebTransport`] certificate. Browsers
/// accept `serverCertificateHashes` only for certificates valid for at most 14 days.
const WT_VALIDITY_DAYS: i64 = 13;

/// Server certificate source.
#[derive(Clone, Debug)]
pub enum QuicServerTls {
    /// Development: a self-signed certificate is generated at startup for these
    /// names (DNS names or IP literals). Read it back with
    /// `Endpoint::server_cert_der` and hand it to clients.
    SelfSigned { names: Vec<String> },
    /// Development, for browsers over WebTransport: like `SelfSigned`, but ECDSA
    /// P-256, valid for 13 days from one hour ago, as `serverCertificateHashes`
    /// requires (Chrome, Firefox). Hand [`crate::Endpoint::server_cert_sha256`]
    /// to the page. Safari does not implement `serverCertificateHashes`.
    SelfSignedWebTransport { names: Vec<String> },
    /// Production: PEM certificate chain and PEM private key files.
    PemFiles { cert_chain: PathBuf, private_key: PathBuf },
}

impl QuicServerTls {
    /// Short-lived P-256 certificate for browsers, for `localhost`, `127.0.0.1` and `::1`.
    pub fn dev_localhost_webtransport() -> Self {
        QuicServerTls::SelfSignedWebTransport {
            names: vec!["localhost".into(), "127.0.0.1".into(), "::1".into()],
        }
    }

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

pub(crate) fn transport_config(
    cfg: &NetConfig,
    server: bool,
    webtransport: bool,
) -> Result<quinn::TransportConfig, NetError> {
    let mut t = quinn::TransportConfig::default();
    t.keep_alive_interval(Some(cfg.keepalive_interval));
    t.max_idle_timeout(Some(cfg.idle_timeout.try_into().map_err(NetError::new)?));
    if webtransport {
        // HTTP/3 needs three unidirectional streams per side (control, QPACK) and
        // one bidirectional stream per request (the CONNECT stream and our
        // reliable stream). The limits are per connection and cannot depend on
        // the ALPN, so a shared port allows these for `orrery/1` clients too.
        t.max_concurrent_bidi_streams(8u32.into());
        t.max_concurrent_uni_streams(8u32.into());
    } else {
        // One bidirectional stream (the reliable channel), no unidirectional streams.
        t.max_concurrent_bidi_streams(if server { 1u32 } else { 0u32 }.into());
        t.max_concurrent_uni_streams(0u32.into());
    }
    let dgram = cfg.max_message_size.min(u16::MAX as usize).max(1500);
    t.datagram_receive_buffer_size(Some(dgram * 256));
    t.datagram_send_buffer_size(dgram * 256);
    Ok(t)
}

/// Certificate chain and key of a server, shared by QUIC/WebTransport and `wss`.
pub(crate) struct Identity {
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
    /// DER of the generated certificate (self-signed variants).
    pub generated: Option<Vec<u8>>,
}

impl Identity {
    pub fn load(tls: &QuicServerTls) -> Result<Identity, NetError> {
        let (chain, key, generated) = match tls {
            QuicServerTls::SelfSignedWebTransport { names } => {
                let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(NetError::new)?;
                let mut params = rcgen::CertificateParams::new(names.clone()).map_err(NetError::new)?;
                let now = time::OffsetDateTime::now_utc();
                params.not_before = now - time::Duration::hours(1);
                params.not_after = now + time::Duration::days(WT_VALIDITY_DAYS);
                let cert = params.self_signed(&key).map_err(NetError::new)?;
                let der = cert.der().to_vec();
                let pkcs8 = PrivateKeyDer::Pkcs8(key.serialize_der().into());
                (vec![CertificateDer::from(der.clone())], pkcs8, Some(der))
            }
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
        Ok(Identity { chain, key, generated })
    }

    fn rustls_config(&self, alpn: Vec<Vec<u8>>) -> Result<rustls::ServerConfig, NetError> {
        let mut rc = rustls::ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
            .map_err(NetError::new)?
            .with_no_client_auth()
            .with_single_cert(self.chain.clone(), self.key.clone_key())
            .map_err(NetError::new)?;
        rc.alpn_protocols = alpn;
        Ok(rc)
    }

    /// TLS for `wss://` (HTTP/1.1 upgrade).
    pub fn wss_config(&self) -> Result<Arc<rustls::ServerConfig>, NetError> {
        Ok(Arc::new(self.rustls_config(vec![b"http/1.1".to_vec()])?))
    }
}

pub(crate) fn build_quic_server_config(
    id: &Identity,
    cfg: &NetConfig,
    webtransport: bool,
) -> Result<quinn::ServerConfig, NetError> {
    // QUIC requires TLS 1.3.
    let mut rc = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(NetError::new)?
        .with_no_client_auth()
        .with_single_cert(id.chain.clone(), id.key.clone_key())
        .map_err(NetError::new)?;
    rc.alpn_protocols = if webtransport { vec![ALPN.to_vec(), ALPN_H3.to_vec()] } else { vec![ALPN.to_vec()] };
    let qc = QuicServerConfig::try_from(Arc::new(rc)).map_err(NetError::new)?;
    let mut sc = quinn::ServerConfig::with_crypto(Arc::new(qc));
    sc.transport_config(Arc::new(transport_config(cfg, true, webtransport)?));
    Ok(sc)
}

/// Builds the quinn client configuration. Public only so that tests can build
/// raw quinn clients with the same trust rules.
#[doc(hidden)]
pub fn build_quic_client_config(trust: &QuicTrust, cfg: &NetConfig) -> Result<quinn::ClientConfig, NetError> {
    let rc = build_client_rustls(trust, false)?;
    let qc = QuicClientConfig::try_from(Arc::new(rc)).map_err(NetError::new)?;
    let mut cc = quinn::ClientConfig::new(Arc::new(qc));
    cc.transport_config(Arc::new(transport_config(cfg, false, false)?));
    Ok(cc)
}

pub(crate) fn build_client_rustls(trust: &QuicTrust, wss: bool) -> Result<rustls::ClientConfig, NetError> {
    if wss && matches!(trust, QuicTrust::Sha256Fingerprint(_) | QuicTrust::DangerousSkipVerificationDevOnly) {
        return Err(NetError("WSS requires WebPki or certificate CA trust; verification bypass is unsupported".into()));
    }
    let prov = provider();
    let versions: &[&rustls::SupportedProtocolVersion] = if wss { &[&rustls::version::TLS13, &rustls::version::TLS12] } else { &[&rustls::version::TLS13] };
    let builder = rustls::ClientConfig::builder_with_provider(prov.clone())
        .with_protocol_versions(versions)
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
            if wss && roots.is_empty() {
                return Err(NetError("WSS CA file holds no certificate".into()));
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
    rc.alpn_protocols = vec![if wss { b"http/1.1".to_vec() } else { ALPN.to_vec() }];
    Ok(rc)
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
