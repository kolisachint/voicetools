//! TLS trust configuration for outbound downloads.
//!
//! Trust is layered the same way as `webtools`: OS trust store first (so
//! TLS-intercepting corporate proxies work out of the box), then the bundled
//! Mozilla root program as a fallback, then any certs added via
//! `SSL_CERT_FILE` or `--ca-cert`. `--insecure` disables verification
//! entirely as a last resort. Proxy selection (`HTTPS_PROXY`/`NO_PROXY`) is
//! handled by ureq's `proxy-from-env` feature, not here.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

/// User-controlled trust settings, wired up from CLI flags.
#[derive(Default, Clone)]
pub struct TlsOptions {
    /// Extra PEM CA files from `--ca-cert` (repeatable). Unlike
    /// `SSL_CERT_FILE`, a bad path here is a hard error — the user asked for
    /// it explicitly.
    pub extra_ca_certs: Vec<PathBuf>,
    /// Disable certificate verification entirely. Loudly opt-in only.
    pub insecure: bool,
}

/// Build a ureq agent with the layered trust store (and proxy-from-env, via
/// the ureq feature flag) applied.
pub fn build_agent(opts: &TlsOptions) -> Result<ureq::Agent> {
    let mut roots = RootCertStore::empty();

    let native_added = match rustls_native_certs::load_native_certs() {
        Ok(certs) => roots.add_parsable_certificates(certs).0,
        Err(e) => {
            eprintln!("  warning: could not load OS root certificates: {e}");
            0
        }
    };
    if native_added == 0 {
        eprintln!("  (no usable OS root certificates, using bundled Mozilla roots)");
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }

    if let Ok(path) = std::env::var("SSL_CERT_FILE") {
        match load_pem_certs(Path::new(&path)) {
            Ok(certs) => {
                let (added, skipped) = roots.add_parsable_certificates(certs);
                if skipped > 0 {
                    eprintln!(
                        "  warning: SSL_CERT_FILE={path}: {skipped} cert(s) could not be parsed"
                    );
                }
                let _ = added;
            }
            Err(e) => {
                eprintln!("  warning: SSL_CERT_FILE={path}: {e:#} — skipping");
            }
        }
    }

    for path in &opts.extra_ca_certs {
        let certs = load_pem_certs(path)
            .with_context(|| format!("reading --ca-cert {}", path.display()))?;
        roots.add_parsable_certificates(certs);
    }

    let builder = ClientConfig::builder();
    let tls_config = if opts.insecure {
        eprintln!(
            "  \u{26A0} WARNING: --insecure disables TLS certificate verification. \
             Do not use this on an untrusted network."
        );
        let mut cfg = builder.with_root_certificates(roots).with_no_client_auth();
        cfg.dangerous()
            .set_certificate_verifier(Arc::new(AcceptAnyCert));
        cfg
    } else {
        builder.with_root_certificates(roots).with_no_client_auth()
    };

    Ok(ureq::AgentBuilder::new()
        .tls_config(Arc::new(tls_config))
        .user_agent("voicetools/0.1")
        .build())
}

fn load_pem_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let bytes = fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    let mut reader = std::io::Cursor::new(bytes);
    rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("could not parse PEM certificates in {}", path.display()))
}

/// Accepts any certificate. Only reachable via `--insecure`.
#[derive(Debug)]
struct AcceptAnyCert;

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
