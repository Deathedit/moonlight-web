use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::{
    boxart::{self, BoxArtCache},
    config::Config,
    discovery::Discovery,
    hosts::{parse_address, Host, HostStore},
    nvhttp::{NvClient, NvError, ServerInfo, DEFAULT_HTTPS_PORT},
    pairing::{self, PairSession, PairSessions, PairState},
    stream::{
        codec::VideoCodec,
        session::{StreamRequest, Streams, VideoTransport},
    },
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub hosts: Arc<HostStore>,
    pub nv: NvClient,
    pub pairing: PairSessions,
    pub boxart: Arc<BoxArtCache>,
    pub streams: Streams,
    pub discovery: Discovery,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/hosts", get(list_hosts).post(add_host))
        .route("/discover", get(discover))
        .route("/hosts/{id}", get(get_host).delete(delete_host))
        .route("/hosts/{id}/pair", get(pair_status).post(start_pair).delete(cancel_pair))
        .route("/hosts/{id}/unpair", post(unpair_host))
        .route("/hosts/{id}/apps", get(list_apps))
        .route("/hosts/{id}/apps/{app_id}/art", get(app_art))
        .route("/hosts/{id}/quit", post(quit_app))
        .route("/hosts/{id}/stream", post(start_stream))
        // Don't let unknown API paths fall through to the SPA's index.html.
        .fallback(|| async { ApiError(StatusCode::NOT_FOUND, "no such endpoint".into()) })
        .with_state(state)
}

/// JSON error body: `{ "error": "..." }`.
pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!("{e:#}");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
    }
}

impl From<NvError> for ApiError {
    fn from(e: NvError) -> Self {
        Self(StatusCode::BAD_GATEWAY, e.to_string())
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

fn not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "unknown host".into())
}

/// Turn a host error into something a user can act on.
fn host_error(host: &Host, what: &str, e: NvError) -> ApiError {
    if e.is_pairing_rejected() {
        return ApiError(
            StatusCode::CONFLICT,
            format!("{} no longer accepts this bridge (it was unpaired on the host). Pair again from the Hosts page.", host.name),
        );
    }
    match e {
        NvError::Unreachable(_) => ApiError(
            StatusCode::BAD_GATEWAY,
            format!("couldn't reach {} at {} — is it on and is Sunshine running?", host.name, host.address),
        ),
        other => ApiError(StatusCode::BAD_GATEWAY, format!("{what}: {other}")),
    }
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
    /// Number of connection stages known to moonlight-common-c — proves the FFI link works.
    stages: u32,
}

async fn health() -> Json<Health> {
    // STAGE_MAX is a #define from Limelight.h, exported by bindgen.
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        stages: moonlight_common_sys::STAGE_MAX,
    })
}

/// A stored host merged with its live status.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HostView {
    id: String,
    name: String,
    address: String,
    port: u16,
    paired: bool,
    online: bool,
    busy: bool,
    current_game: u32,
    app_version: Option<String>,
}

impl HostView {
    fn new(host: Host, info: Option<&ServerInfo>, paired: bool) -> Self {
        Self {
            paired,
            online: info.is_some(),
            busy: info.is_some_and(|i| i.busy),
            current_game: info.map_or(0, |i| i.current_game),
            app_version: info.map(|i| i.app_version.clone()),
            id: host.id,
            name: host.name,
            address: host.address,
            port: host.http_port,
        }
    }
}

/// Query live status. For hosts we've paired with, ask over HTTPS so we notice if the
/// pairing was revoked on the host side (the TLS handshake or PairStatus fails then).
async fn probe(nv: &NvClient, host: Host) -> HostView {
    if let Some(cert) = &host.server_cert {
        match nv.server_info_https(&host.endpoint(), cert).await {
            Ok(info) => {
                let paired = info.paired;
                return HostView::new(host, Some(&info), paired);
            }
            Err(e) => tracing::debug!(host = %host.name, "https serverinfo failed: {e}"),
        }
    }
    match nv.server_info(&host.address, host.http_port).await {
        Ok(info) => HostView::new(host, Some(&info), false),
        Err(e) => {
            tracing::debug!(host = %host.name, "offline: {e}");
            // Can't tell while offline; report what we have on record.
            let paired = host.server_cert.is_some();
            HostView::new(host, None, paired)
        }
    }
}

async fn list_hosts(State(s): State<AppState>) -> ApiResult<Vec<HostView>> {
    let probes = s.hosts.list().await.into_iter().map(|h| probe(&s.nv, h));
    Ok(Json(futures::future::join_all(probes).await))
}

async fn get_host(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<HostView> {
    let host = s.hosts.get(&id).await.ok_or_else(not_found)?;
    Ok(Json(probe(&s.nv, host).await))
}

#[derive(Deserialize)]
struct AddHost {
    address: String,
}

async fn add_host(State(s): State<AppState>, Json(req): Json<AddHost>) -> ApiResult<HostView> {
    let (address, port) = parse_address(&req.address)
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "enter an IP address or hostname, optionally with :port".into()))?;
    let info = s.nv.server_info(&address, port).await.map_err(|e| match e {
        NvError::Unreachable(_) => ApiError(
            StatusCode::BAD_GATEWAY,
            format!("couldn't reach {address}:{port} — is Sunshine running and the port open?"),
        ),
        other => other.into(),
    })?;

    let host = s
        .hosts
        .upsert(Host {
            id: info.unique_id.clone(),
            name: info.hostname.clone(),
            address,
            http_port: port,
            https_port: info.https_port,
            server_cert: None,
        })
        .await?;
    tracing::info!(name = %host.name, id = %host.id, "host added");
    // Re-adding a host we're already paired with keeps the pairing.
    Ok(Json(probe(&s.nv, host).await))
}

async fn delete_host(State(s): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let host = s.hosts.get(&id).await.ok_or_else(not_found)?;
    s.pairing.cancel(&id);
    if host.server_cert.is_some() {
        // Best effort: also drop our cert from the host's paired-clients list.
        s.nv.unpair(&host.endpoint()).await;
    }
    s.hosts.remove(&id).await?;
    s.boxart.remove_host(&id).await;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Pairing ----

#[derive(Serialize)]
struct PairView {
    pin: String,
    #[serde(flatten)]
    state: PairState,
}

/// Start pairing: returns a PIN immediately; the handshake runs in the background
/// (it blocks until the PIN is typed into Sunshine). Poll `GET .../pair` for the result.
async fn start_pair(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<PairView> {
    if let Some((pin, state @ PairState::Pending)) = s.pairing.get(&id) {
        return Ok(Json(PairView { pin, state }));
    }
    let mut host = s.hosts.get(&id).await.ok_or_else(not_found)?;
    let info = s.nv.server_info(&host.address, host.http_port).await?;
    if info.https_port != host.https_port {
        host.https_port = if info.https_port == 0 { DEFAULT_HTTPS_PORT } else { info.https_port };
        host = s.hosts.upsert(host).await?;
    }

    let pin = pairing::generate_pin();
    // Don't start until the session is registered, so the task can always record its result.
    let (go, ready) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn({
        let s = s.clone();
        let id = id.clone();
        let pin = pin.clone();
        async move {
            if ready.await.is_err() {
                return;
            }
            let state = match pairing::pair(&s.nv, &host.endpoint(), info.app_version_major(), &pin).await {
                Ok(cert) => match s.hosts.set_server_cert(&id, Some(cert)).await {
                    Ok(()) => {
                        tracing::info!(host = %host.name, "paired");
                        PairState::Paired
                    }
                    Err(e) => PairState::Failed { error: format!("paired, but saving failed: {e}") },
                },
                Err(e) => {
                    tracing::warn!(host = %host.name, "pairing failed: {e}");
                    PairState::Failed { error: e.to_string() }
                }
            };
            s.pairing.set_state(&id, state);
        }
    });
    s.pairing.insert(id, PairSession { pin: pin.clone(), state: PairState::Pending, task: task.abort_handle() });
    let _ = go.send(());

    Ok(Json(PairView { pin, state: PairState::Pending }))
}

async fn pair_status(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<PairView> {
    let (pin, state) = s
        .pairing
        .get(&id)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no pairing in progress".into()))?;
    Ok(Json(PairView { pin, state }))
}

async fn cancel_pair(State(s): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if s.pairing.cancel(&id) {
        if let Some(host) = s.hosts.get(&id).await {
            s.nv.unpair(&host.endpoint()).await;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---- Apps ----

/// Look up a host we hold a pinned certificate for.
async fn paired_host(s: &AppState, id: &str) -> Result<(Host, String), ApiError> {
    let host = s.hosts.get(id).await.ok_or_else(not_found)?;
    let cert = host
        .server_cert
        .clone()
        .ok_or_else(|| ApiError(StatusCode::CONFLICT, "host is not paired".into()))?;
    Ok((host, cert))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppView {
    id: u32,
    title: String,
    hdr: bool,
    /// This app is the one currently running on the host.
    running: bool,
}

async fn list_apps(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Vec<AppView>> {
    let (host, cert) = paired_host(&s, &id).await?;
    let ep = host.endpoint();
    let (apps, info) = tokio::join!(s.nv.app_list(&ep, &cert), s.nv.server_info_https(&ep, &cert));
    let apps = apps.map_err(|e| host_error(&host, "couldn't load apps", e))?;
    let current = info.map_or(0, |i| i.current_game);
    Ok(Json(
        apps.into_iter()
            .map(|a| AppView { running: a.id == current, id: a.id, title: a.title, hdr: a.hdr })
            .collect(),
    ))
}

async fn app_art(State(s): State<AppState>, Path((id, app_id)): Path<(String, u32)>) -> Result<Response, ApiError> {
    let data = match s.boxart.get(&id, app_id).await {
        Some(data) => data,
        None => {
            let (host, cert) = paired_host(&s, &id).await?;
            let data = s.nv.app_asset(&host.endpoint(), &cert, app_id).await.map_err(|e| host_error(&host, "box art", e))?;
            if let Err(e) = s.boxart.put(&id, app_id, &data).await {
                tracing::warn!("couldn't cache box art: {e}");
            }
            data
        }
    };
    Ok((
        [
            (header::CONTENT_TYPE, boxart::content_type(&data)),
            (header::CACHE_CONTROL, "private, max-age=86400"),
        ],
        data,
    )
        .into_response())
}

async fn quit_app(State(s): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let (host, cert) = paired_host(&s, &id).await?;
    s.nv.cancel(&host.endpoint(), &cert).await.map_err(|e| host_error(&host, "couldn't quit the app", e))?;
    tracing::info!(host = %host.name, "quit running app");
    Ok(StatusCode::NO_CONTENT)
}

// ---- Streaming ----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartStream {
    app_id: u32,
    width: u32,
    height: u32,
    fps: u32,
    bitrate_kbps: u32,
    /// Video codecs the browser wants, best first; defaults to H.264 only.
    #[serde(default = "default_codecs")]
    codecs: Vec<VideoCodec>,
    #[serde(default)]
    video_transport: VideoTransport,
    /// The browser's WebRTC offer (non-trickle).
    sdp: String,
}

fn default_codecs() -> Vec<VideoCodec> {
    vec![VideoCodec::H264]
}

#[derive(Serialize)]
struct StreamAnswer {
    sdp: String,
    /// The codec the bridge picked.
    codec: VideoCodec,
}

/// Answer the browser's offer; the session (launch + connect) continues in the background
/// and reports progress over the WebRTC "control" data channel.
async fn start_stream(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<StartStream>,
) -> ApiResult<StreamAnswer> {
    let bad = |m: &str| ApiError(StatusCode::BAD_REQUEST, m.into());
    if !(320..=7680).contains(&req.width) || !(240..=4320).contains(&req.height) {
        return Err(bad("unsupported resolution"));
    }
    if !(10..=240).contains(&req.fps) {
        return Err(bad("unsupported frame rate"));
    }
    if !(500..=500_000).contains(&req.bitrate_kbps) {
        return Err(bad("bitrate must be between 0.5 and 500 Mbps"));
    }

    let (host, cert) = paired_host(&s, &id).await?;
    if req.codecs.is_empty() {
        return Err(bad("no video codecs requested"));
    }
    let (sdp, codec) = s
        .streams
        .start(
            s.nv.clone(),
            host,
            cert,
            &s.config.webrtc,
            StreamRequest {
                app_id: req.app_id,
                codecs: req.codecs,
                video_transport: req.video_transport,
                width: req.width,
                height: req.height,
                fps: req.fps,
                bitrate_kbps: req.bitrate_kbps,
                offer_sdp: req.sdp,
            },
        )
        .await
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("WebRTC setup failed: {e:#}")))?;
    Ok(Json(StreamAnswer { sdp, codec }))
}

// ---- Discovery & unpairing ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveredHost {
    name: String,
    address: String,
    port: u16,
    /// Set when this host is already in the host list.
    known_id: Option<String>,
}

/// Hosts advertising on the LAN that answer serverinfo, flagged if already added.
async fn discover(State(s): State<AppState>) -> ApiResult<Vec<DiscoveredHost>> {
    let known = s.hosts.list().await;
    let probes = s.discovery.list().into_iter().map(|adv| {
        let nv = s.nv.clone();
        let known = &known;
        async move {
            let address = adv.address.to_string();
            let info = nv.server_info(&address, adv.port).await.ok()?;
            Some(DiscoveredHost {
                name: if info.hostname.is_empty() { adv.name } else { info.hostname.clone() },
                known_id: known.iter().find(|h| h.id == info.unique_id).map(|h| h.id.clone()),
                address,
                port: adv.port,
            })
        }
    });
    Ok(Json(futures::future::join_all(probes).await.into_iter().flatten().collect()))
}

/// Forget the pairing on both sides (best effort on the host).
async fn unpair_host(State(s): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let host = s.hosts.get(&id).await.ok_or_else(not_found)?;
    s.nv.unpair(&host.endpoint()).await;
    s.hosts.set_server_cert(&id, None).await?;
    tracing::info!(host = %host.name, "unpaired");
    Ok(StatusCode::NO_CONTENT)
}
