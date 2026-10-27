//! Persistent list of known GameStream hosts (`<data_dir>/hosts.json`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::nvhttp::{Endpoint, DEFAULT_HTTPS_PORT, DEFAULT_HTTP_PORT};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    /// The host's GameStream `uniqueid` — stable across IP changes.
    pub id: String,
    pub name: String,
    pub address: String,
    pub http_port: u16,
    #[serde(default = "default_https_port")]
    pub https_port: u16,
    /// Pinned server certificate (PEM), set once pairing succeeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_cert: Option<String>,
}

fn default_https_port() -> u16 {
    DEFAULT_HTTPS_PORT
}

impl Host {
    pub fn endpoint(&self) -> Endpoint<'_> {
        Endpoint { address: &self.address, http_port: self.http_port, https_port: self.https_port }
    }
}

pub struct HostStore {
    path: PathBuf,
    hosts: RwLock<Vec<Host>>,
}

impl HostStore {
    pub async fn load(data_dir: &std::path::Path) -> anyhow::Result<Self> {
        let path = data_dir.join("hosts.json");
        let hosts = match tokio::fs::read(&path).await {
            Ok(raw) => serde_json::from_slice(&raw)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, hosts: RwLock::new(hosts) })
    }

    pub async fn list(&self) -> Vec<Host> {
        self.hosts.read().await.clone()
    }

    pub async fn get(&self, id: &str) -> Option<Host> {
        self.hosts.read().await.iter().find(|h| h.id == id).cloned()
    }

    /// Insert, or update the address/name of an already-known host (keeps pairing).
    pub async fn upsert(&self, host: Host) -> anyhow::Result<Host> {
        let mut hosts = self.hosts.write().await;
        let stored = match hosts.iter_mut().find(|h| h.id == host.id) {
            Some(existing) => {
                existing.name = host.name;
                existing.address = host.address;
                existing.http_port = host.http_port;
                existing.https_port = host.https_port;
                existing.clone()
            }
            None => {
                hosts.push(host.clone());
                host
            }
        };
        self.save(&hosts).await?;
        Ok(stored)
    }

    /// Pin (or with `None`, forget) the host's certificate.
    pub async fn set_server_cert(&self, id: &str, cert: Option<String>) -> anyhow::Result<()> {
        let mut hosts = self.hosts.write().await;
        if let Some(h) = hosts.iter_mut().find(|h| h.id == id) {
            h.server_cert = cert;
            self.save(&hosts).await?;
        }
        Ok(())
    }

    pub async fn remove(&self, id: &str) -> anyhow::Result<bool> {
        let mut hosts = self.hosts.write().await;
        let before = hosts.len();
        hosts.retain(|h| h.id != id);
        let removed = hosts.len() != before;
        if removed {
            self.save(&hosts).await?;
        }
        Ok(removed)
    }

    async fn save(&self, hosts: &[Host]) -> anyhow::Result<()> {
        crate::util::write_atomic(&self.path, &serde_json::to_vec_pretty(hosts)?).await?;
        Ok(())
    }
}

/// Parse user input like `192.168.1.85`, `pc.lan:47989`, `fe80::1` or `[fe80::1]:47989`.
pub fn parse_address(input: &str) -> Option<(String, u16)> {
    let input = input.trim();
    if input.is_empty() || input.contains('/') || input.contains(char::is_whitespace) {
        return None;
    }
    if let Some(rest) = input.strip_prefix('[') {
        let (addr, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None if tail.is_empty() => DEFAULT_HTTP_PORT,
            None => return None,
        };
        return Some((addr.to_string(), port));
    }
    match input.matches(':').count() {
        0 => Some((input.to_string(), DEFAULT_HTTP_PORT)),
        1 => {
            let (addr, port) = input.split_once(':')?;
            Some((addr.to_string(), port.parse().ok()?))
        }
        // Bare IPv6 literal.
        _ => Some((input.to_string(), DEFAULT_HTTP_PORT)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_addresses() {
        assert_eq!(parse_address("192.168.1.85"), Some(("192.168.1.85".into(), 47989)));
        assert_eq!(parse_address(" pc.lan:1234 "), Some(("pc.lan".into(), 1234)));
        assert_eq!(parse_address("fe80::1"), Some(("fe80::1".into(), 47989)));
        assert_eq!(parse_address("[fe80::1]:50000"), Some(("fe80::1".into(), 50000)));
        assert_eq!(parse_address("[fe80::1]"), Some(("fe80::1".into(), 47989)));
        assert_eq!(parse_address("pc:notaport"), None);
        assert_eq!(parse_address("http://pc"), None);
        assert_eq!(parse_address(""), None);
    }
}
