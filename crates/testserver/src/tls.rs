//! TLS for the test server: a throwaway certificate authority generated at
//! start-up, a server certificate it issued for `localhost`, `127.0.0.1` and
//! `::1`, and a rustls server configuration with the chosen ALPN protocols.
//!
//! Clients trust the CA certificate ([`TestCa::der`] / [`TestCa::pem`]), not
//! the server certificate itself: platform verifiers (the Windows chain engine in
//! particular) only consult extra trust anchors for a chain that ends in an
//! unknown issuer, not for a self-signed leaf.

use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, date_time_ymd,
};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// Which application protocols a TLS listener offers through ALPN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Alpn {
    /// `h2` and `http/1.1`, preferring `h2` (what most HTTPS servers do).
    #[default]
    Both,
    /// `h2` only.
    H2,
    /// `http/1.1` only (an HTTPS server without HTTP/2).
    Http1,
}

impl Alpn {
    fn protocols(self) -> Vec<Vec<u8>> {
        match self {
            Alpn::Both => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
            Alpn::H2 => vec![b"h2".to_vec()],
            Alpn::Http1 => vec![b"http/1.1".to_vec()],
        }
    }
}

impl std::str::FromStr for Alpn {
    type Err = String;

    /// `both`, `h2` or `http1`.
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "both" => Ok(Alpn::Both),
            "h2" => Ok(Alpn::H2),
            "http1" | "http/1.1" => Ok(Alpn::Http1),
            other => Err(format!("unknown ALPN choice {other:?} (both, h2, http1)")),
        }
    }
}

/// A certificate authority made for one test server run, and the server
/// certificate it issued.
pub struct TestCa {
    ca_der: CertificateDer<'static>,
    ca_pem: String,
    leaf_der: CertificateDer<'static>,
    leaf_key: Vec<u8>,
}

impl std::fmt::Debug for TestCa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestCa").finish_non_exhaustive()
    }
}

impl TestCa {
    /// Generates a new CA (ECDSA P-256) and a server certificate for
    /// `localhost`, `127.0.0.1` and `::1`.
    pub fn generate() -> Result<TestCa, rcgen::Error> {
        let ca_key = KeyPair::generate()?;
        let mut ca = CertificateParams::new(Vec::<String>::new())?;
        ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca.distinguished_name = DistinguishedName::new();
        ca.distinguished_name
            .push(DnType::CommonName, "Boltget test CA (boltget-testserver)");
        ca.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        ca.not_before = date_time_ymd(2024, 1, 1);
        ca.not_after = date_time_ymd(2099, 12, 31);
        let ca_cert = ca.self_signed(&ca_key)?;
        let issuer = Issuer::new(ca, ca_key);

        let leaf_key = KeyPair::generate()?;
        let mut leaf = CertificateParams::new(vec![
            "localhost".to_owned(),
            "127.0.0.1".to_owned(),
            "::1".to_owned(),
        ])?;
        leaf.is_ca = IsCa::ExplicitNoCa;
        leaf.distinguished_name = DistinguishedName::new();
        leaf.distinguished_name
            .push(DnType::CommonName, "boltget-testserver");
        leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        leaf.use_authority_key_identifier_extension = true;
        leaf.not_before = date_time_ymd(2024, 1, 1);
        leaf.not_after = date_time_ymd(2099, 12, 31);
        let leaf_cert = leaf.signed_by(&leaf_key, &issuer)?;

        Ok(TestCa {
            ca_der: ca_cert.der().clone(),
            ca_pem: ca_cert.pem(),
            leaf_der: leaf_cert.der().clone(),
            leaf_key: leaf_key.serialize_der(),
        })
    }

    /// The CA certificate (DER), to add to a client's trusted roots.
    pub fn der(&self) -> &[u8] {
        &self.ca_der
    }

    /// The CA certificate (PEM), as `boltget --ca-cert` reads it.
    pub fn pem(&self) -> &str {
        &self.ca_pem
    }

    /// A rustls server configuration (`ring` provider) presenting the server
    /// certificate and offering `alpn`.
    pub(crate) fn server_config(&self, alpn: Alpn) -> Result<Arc<ServerConfig>, rustls::Error> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.leaf_key.clone()));
        let mut cfg = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(vec![self.leaf_der.clone()], key)?;
        cfg.alpn_protocols = alpn.protocols();
        Ok(Arc::new(cfg))
    }
}
