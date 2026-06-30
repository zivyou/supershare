//! Certificate authority operations for pairing-based provisioning.
//!
//! The Server acts as its own CA: on first run it generates a CA certificate
//! and a Server device certificate (see [`ensure_server_ca`]). During pairing
//! it signs a certificate for the Client from a CSR (see [`sign_client_cert`]).
//!
//! This lives in `ss-network` (not the binary-local `certgen.rs`) because the
//! pairing server must sign client certs at runtime. The CLI `gen-cert`
//! command continues to use `certgen.rs` for manual/advanced deployments.

use anyhow::Context;
use rcgen::{
    CertificateParams, CertificateSigningRequestParams, DnType, Ia5String, KeyPair, SanType,
};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

/// Paths to the Server's CA and device certificate material.
#[derive(Debug, Clone)]
pub struct ServerCa {
    pub ca_cert_path: PathBuf,
    pub ca_key_path: PathBuf,
    pub server_cert_path: PathBuf,
    pub server_key_path: PathBuf,
}

/// Ensure a CA and Server device certificate exist in `dir`, generating them
/// if absent. Returns the paths. The CA private key is written with
/// restrictive permissions (0600 on Unix).
///
/// `server_ips` are added as IP SANs on the Server certificate so clients can
/// verify the Server by IP.
pub fn ensure_server_ca(dir: &Path, server_ips: &[IpAddr]) -> anyhow::Result<ServerCa> {
    std::fs::create_dir_all(dir)?;
    let paths = ServerCa {
        ca_cert_path: dir.join("ca.pem"),
        ca_key_path: dir.join("ca-key.pem"),
        server_cert_path: dir.join("server.pem"),
        server_key_path: dir.join("server-key.pem"),
    };

    // If the CA already exists, assume the set is present and reuse it.
    if paths.ca_cert_path.exists() && paths.ca_key_path.exists() {
        // Regenerate the server cert only if it is missing.
        if !paths.server_cert_path.exists() || !paths.server_key_path.exists() {
            let ca_cert_pem = std::fs::read_to_string(&paths.ca_cert_path)?;
            let ca_key_pem = std::fs::read_to_string(&paths.ca_key_path)?;
            let (server_cert_pem, server_key_pem) =
                generate_server_cert(&ca_cert_pem, &ca_key_pem, server_ips)?;
            std::fs::write(&paths.server_cert_path, server_cert_pem)?;
            write_private(&paths.server_key_path, &server_key_pem)?;
        }
        return Ok(paths);
    }

    // Generate a fresh CA.
    let ca_key = KeyPair::generate()?;
    let mut ca_params = CertificateParams::new(vec!["SuperShare CA".to_string()])?;
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "SuperShare Root CA");
    ca_params
        .distinguished_name
        .push(DnType::OrganizationName, "SuperShare");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key)?;
    let ca_cert_pem = ca_cert.pem();
    let ca_key_pem = ca_key.serialize_pem();

    std::fs::write(&paths.ca_cert_path, &ca_cert_pem)?;
    write_private(&paths.ca_key_path, &ca_key_pem)?;

    // Generate the Server device cert signed by the new CA.
    let (server_cert_pem, server_key_pem) =
        generate_server_cert(&ca_cert_pem, &ca_key_pem, server_ips)?;
    std::fs::write(&paths.server_cert_path, server_cert_pem)?;
    write_private(&paths.server_key_path, &server_key_pem)?;

    tracing::info!("Generated Server CA and certificate in {}", dir.display());
    Ok(paths)
}

/// Generate a Server device certificate (PEM cert, PEM key) signed by the CA.
fn generate_server_cert(
    ca_cert_pem: &str,
    ca_key_pem: &str,
    server_ips: &[IpAddr],
) -> anyhow::Result<(String, String)> {
    let (ca_cert, ca_key) = load_ca(ca_cert_pem, ca_key_pem)?;

    let mut params = CertificateParams::new(vec!["SuperShare Server".to_string()])?;
    params
        .distinguished_name
        .push(DnType::CommonName, "SuperShare Server");
    let localhost: Ia5String = "localhost".try_into()?;
    params.subject_alt_names.push(SanType::DnsName(localhost));
    params
        .subject_alt_names
        .push(SanType::IpAddress(IpAddr::from([127, 0, 0, 1])));
    for ip in server_ips {
        params.subject_alt_names.push(SanType::IpAddress(*ip));
    }

    let key = KeyPair::generate()?;
    let cert = params.signed_by(&key, &ca_cert, &ca_key)?;
    Ok((cert.pem(), key.serialize_pem()))
}

/// Sign a client CSR (PEM) with the CA, returning the signed certificate PEM.
/// The CA is reconstructed in-memory from its PEM material — no temp files.
pub fn sign_client_cert(
    ca_cert_pem: &str,
    ca_key_pem: &str,
    csr_pem: &str,
) -> anyhow::Result<String> {
    let (ca_cert, ca_key) = load_ca(ca_cert_pem, ca_key_pem)?;
    let csr = CertificateSigningRequestParams::from_pem(csr_pem)
        .context("failed to parse client CSR")?;
    let signed = csr
        .signed_by(&ca_cert, &ca_key)
        .context("failed to sign client CSR")?;
    Ok(signed.pem())
}

/// Generate a fresh client keypair and a CSR (PEM) for `device_name`.
/// Returns `(csr_pem, key_pem)`; the key never leaves the client.
pub fn generate_client_csr(device_name: &str) -> anyhow::Result<(String, String)> {
    let key = KeyPair::generate()?;
    let mut params = CertificateParams::new(vec![device_name.to_string()])?;
    params
        .distinguished_name
        .push(DnType::CommonName, device_name);
    let dns: Ia5String = sanitize_dns(device_name).try_into()?;
    params.subject_alt_names.push(SanType::DnsName(dns));
    let csr = params.serialize_request(&key)?;
    Ok((csr.pem()?, key.serialize_pem()))
}

/// Compute a hex-encoded SHA-256 fingerprint of the first certificate in a PEM.
pub fn cert_fingerprint(cert_pem: &str) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let der = pem_to_first_der(cert_pem)?;
    let digest = Sha256::digest(&der);
    Ok(hex_encode(&digest))
}

/// Reconstruct a CA certificate + keypair from PEM material.
fn load_ca(ca_cert_pem: &str, ca_key_pem: &str) -> anyhow::Result<(rcgen::Certificate, KeyPair)> {
    let ca_key = KeyPair::from_pem(ca_key_pem).context("failed to parse CA key")?;
    let ca_params =
        CertificateParams::from_ca_cert_pem(ca_cert_pem).context("failed to parse CA cert")?;
    let ca_cert = ca_params.self_signed(&ca_key)?;
    Ok((ca_cert, ca_key))
}

/// Write a private-key file with restrictive permissions where supported.
fn write_private(path: &Path, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}

/// Replace characters not valid in a DNS SAN with '-'.
fn sanitize_dns(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '-' })
        .collect();
    if s.is_empty() {
        "device".to_string()
    } else {
        s
    }
}

/// Extract the DER bytes of the first certificate in a PEM string.
fn pem_to_first_der(pem: &str) -> anyhow::Result<Vec<u8>> {
    let mut reader = std::io::BufReader::new(pem.as_bytes());
    let cert = rustls_pemfile::certs(&mut reader)
        .next()
        .ok_or_else(|| anyhow::anyhow!("no certificate found in PEM"))??;
    Ok(cert.to_vec())
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_creates_then_reuses() {
        let tmp = std::env::temp_dir().join(format!("ss-ca-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let ca1 = ensure_server_ca(&tmp, &[]).unwrap();
        assert!(ca1.ca_cert_path.exists());
        assert!(ca1.server_cert_path.exists());
        let ca_pem_1 = std::fs::read_to_string(&ca1.ca_cert_path).unwrap();

        // Second call reuses the existing CA.
        let _ca2 = ensure_server_ca(&tmp, &[]).unwrap();
        let ca_pem_2 = std::fs::read_to_string(&ca1.ca_cert_path).unwrap();
        assert_eq!(ca_pem_1, ca_pem_2, "CA must be reused, not regenerated");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn sign_client_csr_round_trip() {
        let tmp = std::env::temp_dir().join(format!("ss-ca-sign-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let ca = ensure_server_ca(&tmp, &[]).unwrap();
        let ca_cert_pem = std::fs::read_to_string(&ca.ca_cert_path).unwrap();
        let ca_key_pem = std::fs::read_to_string(&ca.ca_key_path).unwrap();

        let (csr_pem, _key_pem) = generate_client_csr("laptop").unwrap();
        let signed = sign_client_cert(&ca_cert_pem, &ca_key_pem, &csr_pem).unwrap();
        assert!(signed.contains("BEGIN CERTIFICATE"));

        let fp = cert_fingerprint(&signed).unwrap();
        assert_eq!(fp.len(), 64, "sha256 hex fingerprint is 64 chars");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
