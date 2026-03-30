//! Local Certificate Authority for MITM HTTPS inspection.
//!
//! Generates a self-signed CA cert on first use and persists it in the data
//! directory. Ephemeral per-host certificates are issued on the fly, signed
//! by this CA, for TLS interception of outbound HTTPS traffic.

use std::path::{Path, PathBuf};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tracing::{debug, info};

use starpod_core::{Result, StarpodError};

/// A local Certificate Authority that can issue ephemeral per-host certificates.
pub struct CertAuthority {
    ca_cert_der: CertificateDer<'static>,
    ca_key: KeyPair,
    ca_params: CertificateParams,
    /// Path to the CA cert PEM file.
    pub ca_cert_path: PathBuf,
    /// Path to the combined CA bundle (system roots + local CA).
    pub ca_bundle_path: PathBuf,
}

impl CertAuthority {
    /// Load an existing CA or generate a new one.
    ///
    /// CA files are stored at:
    /// - `{data_dir}/proxy-ca.pem` — CA certificate
    /// - `{data_dir}/proxy-ca-key.pem` — CA private key
    /// - `{data_dir}/proxy-ca-bundle.pem` — System roots + CA cert
    pub fn load_or_generate(data_dir: &Path) -> Result<Self> {
        let ca_cert_path = data_dir.join("proxy-ca.pem");
        let ca_key_path = data_dir.join("proxy-ca-key.pem");
        let ca_bundle_path = data_dir.join("proxy-ca-bundle.pem");

        let (ca_cert_der, ca_key, ca_params) = if ca_cert_path.exists() && ca_key_path.exists() {
            debug!("Loading existing proxy CA from {}", ca_cert_path.display());
            let cert_pem = std::fs::read_to_string(&ca_cert_path)
                .map_err(|e| StarpodError::Proxy(format!("Read CA cert: {e}")))?;
            let key_pem = std::fs::read_to_string(&ca_key_path)
                .map_err(|e| StarpodError::Proxy(format!("Read CA key: {e}")))?;

            let key = KeyPair::from_pem(&key_pem)
                .map_err(|e| StarpodError::Proxy(format!("Parse CA key: {e}")))?;

            // Rebuild CA params (must match the original generation)
            let params = build_ca_params();

            let cert_der = pem_to_der(&cert_pem)?;
            (cert_der, key, params)
        } else {
            info!("Generating new proxy CA at {}", ca_cert_path.display());
            let (cert_der, key, params) = generate_ca()?;

            let cert_pem = pem_encode("CERTIFICATE", cert_der.as_ref());
            let key_pem = key.serialize_pem();

            std::fs::create_dir_all(data_dir)
                .map_err(|e| StarpodError::Proxy(format!("Create data dir: {e}")))?;
            std::fs::write(&ca_cert_path, &cert_pem)
                .map_err(|e| StarpodError::Proxy(format!("Write CA cert: {e}")))?;
            std::fs::write(&ca_key_path, &key_pem)
                .map_err(|e| StarpodError::Proxy(format!("Write CA key: {e}")))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&ca_key_path, std::fs::Permissions::from_mode(0o600));
            }

            (cert_der, key, params)
        };

        // Build the combined CA bundle
        build_ca_bundle(&ca_cert_path, &ca_bundle_path)?;

        Ok(Self {
            ca_cert_der,
            ca_key,
            ca_params,
            ca_cert_path,
            ca_bundle_path,
        })
    }

    /// Issue an ephemeral TLS certificate for `hostname`, signed by this CA.
    ///
    /// Returns `(cert_chain, private_key)` suitable for `rustls::ServerConfig`.
    pub fn issue_cert(
        &self,
        hostname: &str,
    ) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
        let mut params = CertificateParams::new(vec![hostname.to_string()])
            .map_err(|e| StarpodError::Proxy(format!("Cert params for {hostname}: {e}")))?;

        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, hostname);
        params.distinguished_name = dn;

        let leaf_key = KeyPair::generate()
            .map_err(|e| StarpodError::Proxy(format!("Generate key for {hostname}: {e}")))?;

        let issuer = Issuer::from_params(&self.ca_params, &self.ca_key);
        let cert = params
            .signed_by(&leaf_key, &issuer)
            .map_err(|e| StarpodError::Proxy(format!("Sign cert for {hostname}: {e}")))?;

        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));

        Ok((vec![cert_der, self.ca_cert_der.clone()], key_der))
    }
}

/// Build the CA certificate parameters (used for both generation and reload).
fn build_ca_params() -> CertificateParams {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Starpod Secret Proxy CA");
    dn.push(DnType::OrganizationName, "Starpod");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
}

/// Generate a new self-signed CA certificate.
fn generate_ca() -> Result<(CertificateDer<'static>, KeyPair, CertificateParams)> {
    let params = build_ca_params();

    let key =
        KeyPair::generate().map_err(|e| StarpodError::Proxy(format!("Generate CA key: {e}")))?;

    let cert = params
        .self_signed(&key)
        .map_err(|e| StarpodError::Proxy(format!("Self-sign CA: {e}")))?;

    let cert_der = CertificateDer::from(cert.der().to_vec());
    Ok((cert_der, key, params))
}

/// Build a combined PEM bundle: our local CA cert + system roots aren't needed
/// since we set SSL_CERT_FILE to this file and tools will trust our CA.
/// For simplicity, the bundle is just our CA cert — tools that need system
/// roots can still use the system store for non-proxied connections.
fn build_ca_bundle(ca_cert_path: &Path, bundle_path: &Path) -> Result<()> {
    // Read system CA bundle if available, otherwise just use our CA
    let mut bundle = String::new();

    // Try to read system CA certs
    for sys_path in &[
        "/etc/ssl/certs/ca-certificates.crt", // Debian/Ubuntu
        "/etc/pki/tls/certs/ca-bundle.crt",   // RHEL/Fedora
        "/etc/ssl/cert.pem",                  // macOS
    ] {
        if let Ok(system_certs) = std::fs::read_to_string(sys_path) {
            bundle.push_str(&system_certs);
            if !bundle.ends_with('\n') {
                bundle.push('\n');
            }
            break;
        }
    }

    // Append our local CA
    let ca_pem = std::fs::read_to_string(ca_cert_path)
        .map_err(|e| StarpodError::Proxy(format!("Read CA for bundle: {e}")))?;
    bundle.push_str(&ca_pem);

    std::fs::write(bundle_path, &bundle)
        .map_err(|e| StarpodError::Proxy(format!("Write CA bundle: {e}")))?;

    debug!("CA bundle written to {}", bundle_path.display());
    Ok(())
}

/// PEM-encode a DER block.
fn pem_encode(label: &str, der: &[u8]) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut pem = format!("-----BEGIN {label}-----\n");
    for chunk in b64.as_bytes().chunks(76) {
        pem.push_str(std::str::from_utf8(chunk).unwrap());
        pem.push('\n');
    }
    pem.push_str(&format!("-----END {label}-----\n"));
    pem
}

/// Parse a PEM certificate to DER.
fn pem_to_der(pem: &str) -> Result<CertificateDer<'static>> {
    let mut reader = std::io::BufReader::new(pem.as_bytes());
    let certs = rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| StarpodError::Proxy(format!("Parse PEM: {e}")))?;
    certs
        .into_iter()
        .next()
        .ok_or_else(|| StarpodError::Proxy("No certificate found in PEM".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_ca_and_issue_cert() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ca = CertAuthority::load_or_generate(tmp.path()).unwrap();

        assert!(ca.ca_cert_path.exists());
        assert!(ca.ca_bundle_path.exists());

        let (chain, _key) = ca.issue_cert("api.github.com").unwrap();
        assert_eq!(chain.len(), 2); // leaf + CA
    }

    #[test]
    fn load_existing_ca() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ca1 = CertAuthority::load_or_generate(tmp.path()).unwrap();
        let cert1_pem = std::fs::read_to_string(&ca1.ca_cert_path).unwrap();

        let ca2 = CertAuthority::load_or_generate(tmp.path()).unwrap();
        let cert2_pem = std::fs::read_to_string(&ca2.ca_cert_path).unwrap();

        assert_eq!(cert1_pem, cert2_pem);
    }

    #[test]
    fn issue_different_hostnames() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ca = CertAuthority::load_or_generate(tmp.path()).unwrap();

        let (chain1, _) = ca.issue_cert("api.github.com").unwrap();
        let (chain2, _) = ca.issue_cert("api.stripe.com").unwrap();

        assert_ne!(chain1[0].as_ref(), chain2[0].as_ref());
        assert_eq!(chain1[1].as_ref(), chain2[1].as_ref());
    }

    #[test]
    fn ca_bundle_contains_local_cert() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ca = CertAuthority::load_or_generate(tmp.path()).unwrap();

        let bundle = std::fs::read_to_string(&ca.ca_bundle_path).unwrap();
        let ca_pem = std::fs::read_to_string(&ca.ca_cert_path).unwrap();

        // Bundle should contain our CA cert
        assert!(
            bundle.contains(ca_pem.trim()),
            "Bundle should contain the local CA cert"
        );
    }

    #[test]
    fn ca_key_file_permissions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _ca = CertAuthority::load_or_generate(tmp.path()).unwrap();

        let key_path = tmp.path().join("proxy-ca-key.pem");
        assert!(key_path.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&key_path).unwrap().permissions();
            assert_eq!(
                perms.mode() & 0o777,
                0o600,
                "CA key should have 0600 permissions"
            );
        }
    }

    #[test]
    fn concurrent_cert_issuance() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ca = CertAuthority::load_or_generate(tmp.path()).unwrap();
        let ca = std::sync::Arc::new(ca);

        let mut handles = vec![];
        for i in 0..20 {
            let ca = std::sync::Arc::clone(&ca);
            handles.push(std::thread::spawn(move || {
                let hostname = format!("host-{i}.example.com");
                let (chain, _key) = ca.issue_cert(&hostname).unwrap();
                assert_eq!(chain.len(), 2);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn pem_encode_roundtrip() {
        let data = b"test certificate data";
        let pem = pem_encode("CERTIFICATE", data);
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(pem.contains("-----END CERTIFICATE-----"));
    }
}
