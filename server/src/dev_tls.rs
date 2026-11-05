//! Throwaway TLS for local development: a fresh CA and server certificate on
//! every start. The private keys only ever live in memory.

use anyhow::Result;
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

pub struct DevTls {
    /// What a client must trust. Safe to write to disk.
    pub ca_pem: String,
    pub certs: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

pub fn generate() -> Result<DevTls> {
    let mut ca_params = CertificateParams::new(Vec::new())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate()?;
    let ca_cert = ca_params.self_signed(&ca_key)?;
    let ca_pem = ca_cert.pem();
    let issuer = Issuer::new(ca_params, ca_key);

    let server_key = KeyPair::generate()?;
    let server_params = CertificateParams::new(vec!["localhost".to_string()])?;
    let server_cert = server_params.signed_by(&server_key, &issuer)?;

    Ok(DevTls {
        ca_pem,
        certs: vec![server_cert.der().clone(), ca_cert.der().clone()],
        key: PrivateKeyDer::try_from(server_key.serialize_der())
            .map_err(|err| anyhow::anyhow!("server key: {err}"))?,
    })
}
