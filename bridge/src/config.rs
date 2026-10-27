use std::{net::SocketAddr, path::PathBuf};

/// Runtime configuration, read from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address the HTTP/WebSocket server binds to.
    pub bind: SocketAddr,
    /// Where host list, client certificate and pairing data are stored.
    pub data_dir: PathBuf,
    /// Built web UI (`web/dist`).
    pub web_dir: PathBuf,
    pub webrtc: WebRtcConfig,
}

#[derive(Debug, Clone, Default)]
pub struct WebRtcConfig {
    /// Optional STUN/TURN servers for non-LAN use; empty = host candidates only.
    pub ice_servers: Vec<String>,
    /// Fixed UDP port for media (0 = random). Only one stream runs at a time, so one
    /// port is enough; fixing it lets a firewall (or Docker port mapping) allow it.
    pub udp_port: u16,
    /// IPs to advertise to the browser instead of our interface addresses. Needed in a
    /// Docker bridge network, where our own address is a container IP the browser can't reach.
    pub host_ips: Vec<String>,
}

fn list_env(name: &str) -> Vec<String> {
    std::env::var(name)
        .map(|s| s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind = std::env::var("MLW_BIND")
            .unwrap_or_else(|_| "0.0.0.0:8080".into())
            .parse()?;
        let data_dir = std::env::var("MLW_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data"));
        let web_dir = std::env::var("MLW_WEB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("web/dist"));
        let udp_port = match std::env::var("MLW_WEBRTC_UDP_PORT") {
            Ok(p) => p.parse().map_err(|_| anyhow::anyhow!("MLW_WEBRTC_UDP_PORT must be a port number"))?,
            Err(_) => 0,
        };
        let host_ips = list_env("MLW_WEBRTC_HOST_IPS");
        for ip in &host_ips {
            ip.parse::<std::net::IpAddr>()
                .map_err(|_| anyhow::anyhow!("MLW_WEBRTC_HOST_IPS: {ip:?} is not an IP address"))?;
        }

        let webrtc = WebRtcConfig { ice_servers: list_env("MLW_ICE_SERVERS"), udp_port, host_ips };
        Ok(Self { bind, data_dir, web_dir, webrtc })
    }
}
