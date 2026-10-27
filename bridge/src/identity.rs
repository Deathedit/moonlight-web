//! This bridge's identity towards GameStream hosts: a unique id plus the
//! self-signed client certificate that gets pinned by the host during pairing.

use std::path::Path;

use openssl::{
    asn1::{Asn1Integer, Asn1Time},
    bn::BigNum,
    hash::MessageDigest,
    pkey::{PKey, Private},
    rsa::Rsa,
    x509::{X509NameBuilder, X509},
};
use serde::{Deserialize, Serialize};

pub struct Identity {
    /// 16 hex chars, sent as `uniqueid` on every nvhttp request.
    pub unique_id: String,
    pub cert: X509,
    pub key: PKey<Private>,
}

#[derive(Serialize, Deserialize)]
struct IdentityFile {
    unique_id: String,
}

impl Identity {
    pub async fn load_or_create(data_dir: &Path) -> anyhow::Result<Self> {
        let id_path = data_dir.join("identity.json");
        let cert_path = data_dir.join("client.pem");
        let key_path = data_dir.join("client.key");

        let unique_id = match tokio::fs::read(&id_path).await {
            Ok(raw) => serde_json::from_slice::<IdentityFile>(&raw)?.unique_id,
            Err(_) => {
                let unique_id = format!("{:016X}", uuid::Uuid::new_v4().as_u64_pair().0);
                let file = IdentityFile { unique_id: unique_id.clone() };
                crate::util::write_atomic(&id_path, &serde_json::to_vec_pretty(&file)?).await?;
                tracing::info!(%unique_id, "generated new client id");
                unique_id
            }
        };

        let (cert, key) = match (tokio::fs::read(&cert_path).await, tokio::fs::read(&key_path).await) {
            (Ok(cert), Ok(key)) => (X509::from_pem(&cert)?, PKey::private_key_from_pem(&key)?),
            _ => {
                let (cert, key) = generate_cert()?;
                crate::util::write_atomic(&cert_path, &cert.to_pem()?).await?;
                crate::util::write_private(&key_path, &key.private_key_to_pem_pkcs8()?).await?;
                tracing::info!("generated new client certificate");
                (cert, key)
            }
        };

        Ok(Self { unique_id, cert, key })
    }

    /// In-memory identity for tests.
    #[cfg(test)]
    pub fn ephemeral() -> anyhow::Result<Self> {
        let (cert, key) = generate_cert()?;
        Ok(Self { unique_id: "0123456789ABCDEF".into(), cert, key })
    }

    pub fn cert_pem(&self) -> Vec<u8> {
        self.cert.to_pem().expect("encode client cert")
    }
}

/// RSA-2048 self-signed cert, matching what official Moonlight clients generate.
fn generate_cert() -> anyhow::Result<(X509, PKey<Private>)> {
    let key = PKey::from_rsa(Rsa::generate(2048)?)?;

    let mut name = X509NameBuilder::new()?;
    name.append_entry_by_text("CN", "NVIDIA GameStream Client")?;
    let name = name.build();

    let mut serial = BigNum::new()?;
    serial.rand(63, openssl::bn::MsbOption::MAYBE_ZERO, false)?;

    let serial = Asn1Integer::from_bn(&serial)?;
    let not_before = Asn1Time::days_from_now(0)?;
    let not_after = Asn1Time::days_from_now(365 * 20)?;

    let mut b = X509::builder()?;
    b.set_version(2)?;
    b.set_serial_number(&serial)?;
    b.set_subject_name(&name)?;
    b.set_issuer_name(&name)?;
    b.set_pubkey(&key)?;
    b.set_not_before(&not_before)?;
    b.set_not_after(&not_after)?;
    b.sign(&key, MessageDigest::sha256())?;
    Ok((b.build(), key))
}
