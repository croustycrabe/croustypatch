//! TLS 1.3-only server configuration. Older protocol versions are not negotiable.
use anyhow::{bail, Context, Result};
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct TlsFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
    /// True when we made a self-signed certificate (agents must then pin it).
    pub generated: bool,
}

/// Use the configured certificate, or create a self-signed one in `<data_dir>/tls`.
pub fn ensure_files(cert: Option<&Path>, key: Option<&Path>, data_dir: &Path, san: &[String]) -> Result<TlsFiles> {
    match (cert, key) {
        (Some(c), Some(k)) => Ok(TlsFiles { cert: c.into(), key: k.into(), generated: false }),
        (None, None) => {
            let dir = data_dir.join("tls");
            let (c, k) = (dir.join("server.crt"), dir.join("server.key"));
            if !c.exists() || !k.exists() {
                std::fs::create_dir_all(&dir)?;
                let ck = rcgen::generate_simple_self_signed(san.to_vec()).context("generating self-signed certificate")?;
                std::fs::write(&c, ck.cert.pem())?;
                crate::crypto::write_private(&k, ck.signing_key.serialize_pem().as_bytes())?;
                tracing::warn!(cert = %c.display(), "generated a self-signed TLS certificate; provide CROUSTYPATCH_TLS_CERT/KEY for production");
            }
            Ok(TlsFiles { cert: c, key: k, generated: true })
        }
        _ => bail!("set both CROUSTYPATCH_TLS_CERT and CROUSTYPATCH_TLS_KEY, or neither"),
    }
}

pub fn server_config(files: &TlsFiles) -> Result<rustls::ServerConfig> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&files.cert)
        .with_context(|| format!("reading {}", files.cert.display()))?
        .collect::<Result<_, _>>()
        .context("parsing certificate chain")?;
    if certs.is_empty() {
        bail!("no certificate found in {}", files.cert.display());
    }
    let key = PrivateKeyDer::from_pem_file(&files.key).with_context(|| format!("reading {}", files.key.display()))?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("TLS 1.3 unavailable")?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("certificate and key do not match")?;
    Ok(cfg)
}
