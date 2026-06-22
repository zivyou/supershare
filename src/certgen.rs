use rcgen::{CertificateParams, DnType, Ia5String, KeyPair, SanType};
use std::fs;
use std::path::Path;

/// Generate a self-signed CA certificate and key
pub fn generate_ca(output_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(output_dir)?;

    let mut params = CertificateParams::new(vec!["SuperShare CA".to_string()])?;
    params
        .distinguished_name
        .push(DnType::CommonName, "SuperShare Root CA");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "SuperShare");

    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    let cert_path = output_dir.join("ca.pem");
    let key_path = output_dir.join("ca-key.pem");

    fs::write(&cert_path, &cert_pem)?;
    fs::write(&key_path, &key_pem)?;

    tracing::info!("CA certificate written to {}", cert_path.display());
    tracing::info!("CA key written to {}", key_path.display());

    Ok(())
}

/// Generate a self-signed device certificate
/// Note: For simplicity, this generates a self-signed cert rather than CA-signed.
/// In production, you would use the CA to sign device certs.
pub fn generate_device_cert(
    output_dir: &Path,
    device_name: &str,
) -> anyhow::Result<()> {
    fs::create_dir_all(output_dir)?;

    let mut params = CertificateParams::new(vec![device_name.to_string()])?;
    params
        .distinguished_name
        .push(DnType::CommonName, device_name);
    let device_dns: Ia5String = device_name.try_into()?;
    params
        .subject_alt_names
        .push(SanType::DnsName(device_dns));
    let localhost_dns: Ia5String = "localhost".try_into()?;
    params
        .subject_alt_names
        .push(SanType::DnsName(localhost_dns));

    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    let cert_path = output_dir.join(format!("{device_name}.pem"));
    let key_path = output_dir.join(format!("{device_name}-key.pem"));

    fs::write(&cert_path, &cert_pem)?;
    fs::write(&key_path, &key_pem)?;

    tracing::info!("Device certificate written to {}", cert_path.display());
    tracing::info!("Device key written to {}", key_path.display());

    Ok(())
}
