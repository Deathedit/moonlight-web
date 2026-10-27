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
    /// Optional STUN/TURN servers for non-LAN use; empty = host candidates only.
    pub ice_servers: Vec<String>,
    /// Fixed UDP port for WebRTC media (0 = random). Only one stream runs at a time,
    /// so one port is enough; fixing it lets a firewall allow it.
    pub webrtc_udp_port: u16,
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
        let ice_servers = std::env::var("MLW_ICE_SERVERS")
            .map(|s| s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect())
            .unwrap_or_default();

        let webrtc_udp_port = match std::env::var("MLW_WEBRTC_UDP_PORT") {
            Ok(p) => p.parse().map_err(|_| anyhow::anyhow!("MLW_WEBRTC_UDP_PORT must be a port number"))?,
            Err(_) => 0,
        };

        Ok(Self { bind, data_dir, web_dir, ice_servers, webrtc_udp_port })
    }
}
