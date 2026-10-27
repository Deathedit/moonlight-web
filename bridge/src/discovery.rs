//! Finds GameStream hosts on the LAN via mDNS (`_nvstream._tcp`, which Sunshine and
//! GeForce Experience advertise).

use std::{
    collections::HashMap,
    net::Ipv4Addr,
    sync::{Arc, RwLock},
};

use mdns_sd::{ServiceDaemon, ServiceEvent};

const SERVICE: &str = "_nvstream._tcp.local.";

#[derive(Debug, Clone)]
pub struct Advertised {
    /// mDNS instance name, usually the PC's hostname.
    pub name: String,
    pub address: Ipv4Addr,
    pub port: u16,
}

#[derive(Clone, Default)]
pub struct Discovery {
    found: Arc<RwLock<HashMap<String, Advertised>>>,
}

impl Discovery {
    /// Start browsing in the background. On failure (e.g. no multicast-capable
    /// interface) the bridge works as before, just without discovery.
    pub fn start() -> Self {
        let discovery = Self::default();
        match ServiceDaemon::new().and_then(|d| d.browse(SERVICE).map(|rx| (d, rx))) {
            Ok((daemon, events)) => {
                let found = Arc::clone(&discovery.found);
                std::thread::Builder::new()
                    .name("mdns".into())
                    .spawn(move || {
                        // Keep the daemon alive for as long as we browse.
                        let _daemon = daemon;
                        while let Ok(event) = events.recv() {
                            handle(&found, event);
                        }
                    })
                    .expect("spawn mdns thread");
                tracing::info!("browsing for GameStream hosts via mDNS");
            }
            Err(e) => tracing::warn!("mDNS discovery unavailable: {e}"),
        }
        discovery
    }

    pub fn list(&self) -> Vec<Advertised> {
        let mut hosts: Vec<_> = self.found.read().unwrap().values().cloned().collect();
        hosts.sort_by(|a, b| a.name.cmp(&b.name));
        hosts
    }
}

fn handle(found: &RwLock<HashMap<String, Advertised>>, event: ServiceEvent) {
    match event {
        ServiceEvent::ServiceResolved(info) => {
            // Prefer IPv4: it's what users recognise and what every host listens on.
            let Some(address) = info.get_addresses_v4().into_iter().min() else { return };
            let name = instance_name(info.get_fullname());
            tracing::debug!(%name, %address, port = info.get_port(), "host advertised");
            found
                .write()
                .unwrap()
                .insert(info.get_fullname().to_string(), Advertised { name, address, port: info.get_port() });
        }
        ServiceEvent::ServiceRemoved(_, fullname) => {
            found.write().unwrap().remove(&fullname);
        }
        _ => {}
    }
}

/// "DESKTOP-KFP8A2P._nvstream._tcp.local." → "DESKTOP-KFP8A2P"
fn instance_name(fullname: &str) -> String {
    fullname.strip_suffix(SERVICE).unwrap_or(fullname).trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_service_suffix() {
        assert_eq!(instance_name("DESKTOP-KFP8A2P._nvstream._tcp.local."), "DESKTOP-KFP8A2P");
        assert_eq!(instance_name("weird"), "weird");
    }
}
