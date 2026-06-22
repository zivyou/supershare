use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use std::path::Path;
use std::sync::Arc;

/// Load certificates from a PEM file
pub fn load_certs(path: &Path) -> anyhow::Result<Vec<CertificateDer<'static>>> {
    let file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("Failed to open cert file {}: {e}", path.display()))?;
    let mut reader = std::io::BufReader::new(file);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to parse certs from {}: {e}", path.display()))?;
    if certs.is_empty() {
        anyhow::bail!("No certificates found in {}", path.display());
    }
    Ok(certs)
}

/// Load a private key from a PEM file
pub fn load_key(path: &Path) -> anyhow::Result<PrivateKeyDer<'static>> {
    let file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("Failed to open key file {}: {e}", path.display()))?;
    let mut reader = std::io::BufReader::new(file);

    // Try PKCS8 first, then RSA, then EC
    if let Some(key) = rustls_pemfile::pkcs8_private_keys(&mut reader).next() {
        return key
            .map(PrivateKeyDer::Pkcs8)
            .map_err(|e| anyhow::anyhow!("Failed to parse PKCS8 key: {e}"));
    }

    // Reset reader
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    if let Some(key) = rustls_pemfile::rsa_private_keys(&mut reader).next() {
        return key
            .map(PrivateKeyDer::Pkcs1)
            .map_err(|e| anyhow::anyhow!("Failed to parse RSA key: {e}"));
    }

    anyhow::bail!("No private key found in {}", path.display())
}

/// Build a rustls server config with mTLS (require client certificates)
pub fn build_server_config(
    cert_path: &Path,
    key_path: &Path,
    ca_path: &Path,
) -> anyhow::Result<Arc<rustls::ServerConfig>> {
    let certs = load_certs(cert_path)?;
    let key = load_key(key_path)?;
    let ca_certs = load_certs(ca_path)?;

    // Build CA store for client verification
    let mut root_store = rustls::RootCertStore::empty();
    for cert in ca_certs {
        root_store.add(cert)?;
    }

    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(root_store)).build()?;

    let config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)?;

    Ok(Arc::new(config))
}

/// Build a rustls client config with mTLS (present client cert, verify server)
pub fn build_client_config(
    cert_path: &Path,
    key_path: &Path,
    ca_path: &Path,
) -> anyhow::Result<Arc<rustls::ClientConfig>> {
    let certs = load_certs(cert_path)?;
    let key = load_key(key_path)?;
    let ca_certs = load_certs(ca_path)?;

    let mut root_store = rustls::RootCertStore::empty();
    for cert in ca_certs {
        root_store.add(cert)?;
    }

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_client_auth_cert(certs, key)?;

    Ok(Arc::new(config))
}

/// Parse a server address string into ServerName for TLS
pub fn parse_server_name(host: &str) -> anyhow::Result<ServerName<'static>> {
    // Strip port if present
    let hostname = host.split(':').next().unwrap_or(host);
    ServerName::try_from(hostname.to_string())
        .map_err(|e| anyhow::anyhow!("Invalid server name '{}': {e}", hostname))
}
