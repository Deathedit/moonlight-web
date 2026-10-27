//! GameStream ("nvhttp") HTTP API client — the part of the protocol that runs over
//! plain HTTP(S) before streaming: serverinfo, pairing, app list, launch.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    DigitallySignedStruct, SignatureScheme,
};
use serde::Serialize;

use crate::identity::Identity;

pub const DEFAULT_HTTP_PORT: u16 = 47989;
pub const DEFAULT_HTTPS_PORT: u16 = 47984;
/// Name shown for this client in Sunshine.
const DEVICE_NAME: &str = "MoonlightWeb";

#[derive(Debug, thiserror::Error)]
pub enum NvError {
    #[error("host unreachable: {}", error_chain(.0))]
    Unreachable(#[from] reqwest::Error),
    #[error("invalid response from host: {0}")]
    BadResponse(String),
    #[error("host returned status {code}: {message}")]
    Status { code: u32, message: String },
    #[error("TLS setup failed: {0}")]
    Tls(String),
}

/// reqwest's Display stops at "error sending request for url (...)"; append the causes
/// (connection reset, TLS failure, ...) and shorten the URL to its path — the query
/// carries kilobytes of hex-encoded pairing material.
fn error_chain(e: &reqwest::Error) -> String {
    let mut msg = e.to_string();
    if let Some(url) = e.url() {
        msg = msg.replace(url.as_str(), url.path());
    }
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        msg.push_str(": ");
        msg.push_str(&s.to_string());
        source = s.source();
    }
    msg
}

/// Parsed `/serverinfo` response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub hostname: String,
    pub unique_id: String,
    pub app_version: String,
    pub gfe_version: String,
    pub https_port: u16,
    pub external_port: u16,
    pub local_ip: String,
    pub mac: String,
    pub paired: bool,
    /// 0 when idle, otherwise the running app's id.
    pub current_game: u32,
    pub busy: bool,
    pub codec_mode_support: u32,
}

pub struct LaunchParams {
    pub app_id: u32,
    pub resume: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub audio_configuration: i32,
    /// AES key/IV for the input stream; must match STREAM_CONFIGURATION.
    pub ri_key: [u8; 16],
    /// First 4 bytes of the IV, big-endian, as Moonlight sends it.
    pub ri_key_id: i32,
}

/// Parse `LiGetLaunchUrlQueryParameters()` ("&a=1&b=2") into pairs.
fn launch_url_query_parameters() -> Vec<(String, String)> {
    // SAFETY: returns a pointer to a static, NUL-terminated string.
    let raw = unsafe { std::ffi::CStr::from_ptr(moonlight_common_sys::LiGetLaunchUrlQueryParameters()) };
    raw.to_string_lossy()
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// One entry of `/applist`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub id: u32,
    pub title: String,
    pub hdr: bool,
}

impl ServerInfo {
    /// Major component of `appversion` (GFE 7.x+ / Sunshine use SHA-256 for pairing).
    pub fn app_version_major(&self) -> u32 {
        self.app_version.split('.').next().and_then(|v| v.parse().ok()).unwrap_or(0)
    }
}

/// Where and how to reach a host.
#[derive(Debug, Clone)]
pub struct Endpoint<'a> {
    pub address: &'a str,
    pub http_port: u16,
    pub https_port: u16,
}

#[derive(Clone)]
pub struct NvClient {
    http: reqwest::Client,
    identity: Arc<Identity>,
    /// HTTPS clients keyed by pinned server cert (PEM), built on demand.
    https: Arc<Mutex<HashMap<String, reqwest::Client>>>,
}

impl NvClient {
    pub fn new(identity: Arc<Identity>) -> Self {
        let http = reqwest::Client::builder()
            // Sunshine closes the connection after every response without sending
            // `Connection: close`, so a pooled connection is always dead on reuse.
            .pool_max_idle_per_host(0)
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()
            .expect("reqwest client");
        Self { http, identity, https: Default::default() }
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// Unauthenticated `/serverinfo` over plain HTTP.
    pub async fn server_info(&self, address: &str, port: u16) -> Result<ServerInfo, NvError> {
        let ep = Endpoint { address, http_port: port, https_port: 0 };
        parse_server_info(&self.get_http(&ep, "serverinfo", &[], None).await?)
    }

    /// `/serverinfo` over HTTPS with our client cert — only here does `PairStatus` reflect reality.
    pub async fn server_info_https(&self, ep: &Endpoint<'_>, server_cert: &str) -> Result<ServerInfo, NvError> {
        parse_server_info(&self.get_https(ep, server_cert, "serverinfo", &[]).await?)
    }

    /// Best-effort `/unpair` (used on failed/aborted pairing and host removal).
    pub async fn unpair(&self, ep: &Endpoint<'_>) {
        if let Err(e) = self.get_http(ep, "unpair", &[], None).await {
            tracing::debug!("unpair failed: {e}");
        }
    }

    pub async fn get_http(
        &self,
        ep: &Endpoint<'_>,
        path: &str,
        params: &[(&str, &str)],
        timeout: Option<Duration>,
    ) -> Result<String, NvError> {
        let url = format!("http://{}/{path}", host_port(ep.address, ep.http_port));
        let mut req = self.http.get(url).query(&self.common_params()).query(params);
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        Ok(req.send().await?.text().await?)
    }

    pub async fn get_https(
        &self,
        ep: &Endpoint<'_>,
        server_cert: &str,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<String, NvError> {
        Ok(self.send_https(ep, server_cert, path, params).await?.text().await?)
    }

    async fn send_https(
        &self,
        ep: &Endpoint<'_>,
        server_cert: &str,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<reqwest::Response, NvError> {
        let client = self.https_client(server_cert)?;
        let url = format!("https://{}/{path}", host_port(ep.address, ep.https_port));
        Ok(client.get(url).query(&self.common_params()).query(params).send().await?)
    }

    pub async fn app_list(&self, ep: &Endpoint<'_>, server_cert: &str) -> Result<Vec<AppInfo>, NvError> {
        parse_app_list(&self.get_https(ep, server_cert, "applist", &[]).await?)
    }

    /// Box art (Sunshine serves PNG; unknown ids get a generic placeholder image).
    pub async fn app_asset(&self, ep: &Endpoint<'_>, server_cert: &str, app_id: u32) -> Result<Vec<u8>, NvError> {
        let id = app_id.to_string();
        let res = self
            .send_https(ep, server_cert, "appasset", &[("appid", &id), ("AssetType", "2"), ("AssetIdx", "0")])
            .await?;
        if !res.status().is_success() {
            return Err(NvError::Status { code: res.status().as_u16().into(), message: "appasset".into() });
        }
        Ok(res.bytes().await?.to_vec())
    }

    /// `/launch` (or `/resume` if the app is already running). Returns the RTSP session URL
    /// to hand to moonlight-common-c, if the host provided one.
    pub async fn launch(&self, ep: &Endpoint<'_>, server_cert: &str, p: &LaunchParams) -> Result<Option<String>, NvError> {
        let rikey = hex::encode(p.ri_key);
        let rikeyid = p.ri_key_id.to_string();
        let surround = moonlight_common_sys::surround_audio_info(p.audio_configuration).to_string();
        let mode = format!("{}x{}x{}", p.width, p.height, p.fps);
        let app_id = p.app_id.to_string();

        let mut params: Vec<(&str, &str)> = vec![("rikey", &rikey), ("rikeyid", &rikeyid), ("surroundAudioInfo", &surround)];
        if !p.resume {
            params.extend([
                ("appid", app_id.as_str()),
                ("mode", &mode),
                ("additionalStates", "1"),
                ("sops", "0"),
                ("localAudioPlayMode", "0"),
                ("remoteControllersBitmap", "0"),
                ("gcmap", "0"),
                ("gcpersist", "0"),
            ]);
        }
        // Protocol extensions moonlight-common-c wants advertised (e.g. "&corever=1").
        let extra = launch_url_query_parameters();
        params.extend(extra.iter().map(|(k, v)| (k.as_str(), v.as_str())));

        let path = if p.resume { "resume" } else { "launch" };
        let body = self.get_https(ep, server_cert, path, &params).await?;
        let doc = parse_root(&body)?;
        let ok_tag = if p.resume { "resume" } else { "gamesession" };
        if matches!(child_text(&doc, ok_tag), None | Some("0")) {
            return Err(NvError::BadResponse(format!("host refused to {path} the app")));
        }
        Ok(child_text(&doc, "sessionUrl0").filter(|s| !s.is_empty()).map(String::from))
    }

    /// Quit the running app.
    pub async fn cancel(&self, ep: &Endpoint<'_>, server_cert: &str) -> Result<(), NvError> {
        let body = self.get_https(ep, server_cert, "cancel", &[]).await?;
        let doc = parse_root(&body)?;
        match child_text(&doc, "cancel") {
            Some("1") => Ok(()),
            _ => Err(NvError::BadResponse("host refused to quit the app".into())),
        }
    }

    fn common_params(&self) -> [(&'static str, String); 3] {
        [
            ("uniqueid", self.identity.unique_id.clone()),
            ("uuid", uuid::Uuid::new_v4().simple().to_string()),
            ("devicename", DEVICE_NAME.to_string()),
        ]
    }

    fn https_client(&self, server_cert_pem: &str) -> Result<reqwest::Client, NvError> {
        if let Some(c) = self.https.lock().unwrap().get(server_cert_pem) {
            return Ok(c.clone());
        }
        let tls = |e: &dyn std::fmt::Display| NvError::Tls(e.to_string());

        let server_der = openssl::x509::X509::from_pem(server_cert_pem.as_bytes())
            .and_then(|c| c.to_der())
            .map_err(|e| tls(&e))?;
        let client_cert = CertificateDer::from(self.identity.cert.to_der().map_err(|e| tls(&e))?);
        let client_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            self.identity.key.private_key_to_pkcs8().map_err(|e| tls(&e))?,
        ));

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| tls(&e))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedCert { der: server_der, provider }))
            .with_client_auth_cert(vec![client_cert], client_key)
            .map_err(|e| tls(&e))?;
        // Sunshine identifies the client by its certificate on every connection; a resumed
        // session carries no peer cert and Sunshine aborts the handshake (alert InternalError).
        config.resumption = rustls::client::Resumption::disabled();

        // Sunshine closes the connection after every response without sending
        // `Connection: close`, so a pooled connection is always dead on reuse.
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(config)
            .pool_max_idle_per_host(0)
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()?;
        self.https.lock().unwrap().insert(server_cert_pem.to_string(), client.clone());
        Ok(client)
    }
}

/// Accepts exactly the server certificate pinned at pairing time. GameStream hosts use
/// self-signed certs with no usable hostname, so normal WebPKI validation can't apply.
#[derive(Debug)]
struct PinnedCert {
    der: Vec<u8>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedCert {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end_entity.as_ref() == self.der.as_slice() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Bracket IPv6 literals so they can be used in a URL authority.
fn host_port(address: &str, port: u16) -> String {
    if address.contains(':') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

/// Parse an nvhttp XML document and check its `<root status_code>`.
pub fn parse_root(body: &str) -> Result<roxmltree::Document<'_>, NvError> {
    let doc = roxmltree::Document::parse(body).map_err(|e| NvError::BadResponse(e.to_string()))?;
    let root = doc.root_element();
    let code: u32 = root
        .attribute("status_code")
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| NvError::BadResponse("missing status_code".into()))?;
    if code != 200 {
        return Err(NvError::Status {
            code,
            message: root.attribute("status_message").unwrap_or_default().to_string(),
        });
    }
    Ok(doc)
}

pub fn child_text<'a>(doc: &'a roxmltree::Document, tag: &str) -> Option<&'a str> {
    doc.root_element()
        .children()
        .find(|n| n.has_tag_name(tag))
        .map(|n| n.text().unwrap_or_default().trim())
}

fn parse_server_info(body: &str) -> Result<ServerInfo, NvError> {
    let doc = parse_root(body)?;
    let text = |tag| child_text(&doc, tag).unwrap_or_default().to_string();
    let num = |tag, default| child_text(&doc, tag).and_then(|v| v.parse().ok()).unwrap_or(default);

    let unique_id = text("uniqueid");
    if unique_id.is_empty() {
        return Err(NvError::BadResponse("missing uniqueid".into()));
    }
    Ok(ServerInfo {
        hostname: text("hostname"),
        unique_id,
        app_version: text("appversion"),
        gfe_version: text("GfeVersion"),
        https_port: num("HttpsPort", DEFAULT_HTTPS_PORT as u32) as u16,
        external_port: num("ExternalPort", DEFAULT_HTTP_PORT as u32) as u16,
        local_ip: text("LocalIP"),
        mac: text("mac"),
        paired: text("PairStatus") == "1",
        current_game: num("currentgame", 0),
        busy: text("state").ends_with("_SERVER_BUSY"),
        codec_mode_support: num("ServerCodecModeSupport", 0),
    })
}

fn parse_app_list(body: &str) -> Result<Vec<AppInfo>, NvError> {
    let doc = parse_root(body)?;
    let apps = doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("App"))
        .filter_map(|app| {
            let field = |tag| app.children().find(|n| n.has_tag_name(tag)).and_then(|n| n.text()).map(str::trim);
            Some(AppInfo {
                id: field("ID")?.parse().ok()?,
                title: field("AppTitle").unwrap_or_default().to_string(),
                hdr: field("IsHdrSupported") == Some("1"),
            })
        })
        .collect();
    Ok(apps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sunshine_applist() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<root status_code="200"><App><IsHdrSupported>1</IsHdrSupported><AppTitle>Desktop</AppTitle><UUID>87FACC02-D79B-DE09-7D1E-631C4496BAA9</UUID><IDX>0</IDX><ID>881448767</ID></App><App><IsHdrSupported>0</IsHdrSupported><AppTitle>Steam Big Picture</AppTitle><UUID>0CBCD9C2</UUID><IDX>1</IDX><ID>1093255277</ID></App><App><AppTitle>No id</AppTitle></App></root>"#;
        let apps = parse_app_list(xml).unwrap();
        assert_eq!(apps.len(), 2, "entries without a numeric ID are skipped");
        assert_eq!(apps[0].id, 881448767);
        assert_eq!(apps[0].title, "Desktop");
        assert!(apps[0].hdr);
        assert!(!apps[1].hdr);
    }

    // Captured from Sunshine 2025.x (appversion 7.1.431.-1).
    const SUNSHINE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<root status_code="200"><hostname>DESKTOP-KFP8A2P</hostname><appversion>7.1.431.-1</appversion><GfeVersion>3.23.0.74</GfeVersion><uniqueid>C18D8A40-06AD-CA0D-DD18-DA58B543DBAB</uniqueid><HttpsPort>47984</HttpsPort><ExternalPort>47989</ExternalPort><MaxLumaPixelsHEVC>1869449984</MaxLumaPixelsHEVC><mac>00:00:00:00:00:00</mac><Permission>0</Permission><LocalIP>192.168.1.85</LocalIP><ServerCodecModeSupport>2032385</ServerCodecModeSupport><PairStatus>0</PairStatus><currentgame>0</currentgame><currentgameuuid/><state>SUNSHINE_SERVER_FREE</state></root>"#;

    #[test]
    fn parses_sunshine_serverinfo() {
        let info = parse_server_info(SUNSHINE).unwrap();
        assert_eq!(info.hostname, "DESKTOP-KFP8A2P");
        assert_eq!(info.unique_id, "C18D8A40-06AD-CA0D-DD18-DA58B543DBAB");
        assert_eq!(info.https_port, 47984);
        assert_eq!(info.app_version_major(), 7);
        assert!(!info.paired);
        assert!(!info.busy);
        assert_eq!(info.current_game, 0);
    }

    #[test]
    fn surfaces_error_status() {
        let err = parse_server_info(r#"<root status_code="401" status_message="The client is not authorized"/>"#);
        assert!(matches!(err, Err(NvError::Status { code: 401, .. })));
    }

    #[test]
    fn brackets_ipv6() {
        assert_eq!(host_port("fe80::1", 47989), "[fe80::1]:47989");
        assert_eq!(host_port("192.168.1.85", 47989), "192.168.1.85:47989");
    }
}
