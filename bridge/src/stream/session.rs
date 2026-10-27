//! One streaming session: browser offer → WebRTC answer → launch app on the host →
//! moonlight-common-c connection → frames into the video track, until either side ends it.

use std::{sync::Arc, time::Duration};

use serde::Serialize;
use tokio::sync::{mpsc, Mutex, Notify};
use webrtc::{
    data_channel::{DataChannel, DataChannelEvent},
    peer_connection::RTCPeerConnectionState,
};

use super::{
    codec::{self, VideoCodec},
    connection::{AudioFrame, Connection, ConnectionParams, StreamEvent, VideoFrame, AUDIO_QUEUE, VIDEO_QUEUE},
    input,
    peer::{Peer, PeerEvent},
};
use crate::{
    config::WebRtcConfig,
    hosts::Host,
    nvhttp::{LaunchParams, NvClient},
};

/// The browser must open its control channel within this time after the answer.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(15);

pub struct StreamRequest {
    pub app_id: u32,
    /// Browser's codec preference, best first.
    pub codecs: Vec<VideoCodec>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub offer_sdp: String,
}

/// moonlight-common-c supports one connection per process, so there is at most one
/// session; starting a new one stops the old (e.g. the user reloaded the page).
#[derive(Clone, Default)]
pub struct Streams(Arc<Mutex<Option<Active>>>);

struct Active {
    stop: Arc<Notify>,
    done: tokio::task::JoinHandle<()>,
}

impl Streams {
    pub async fn start(
        &self,
        nv: NvClient,
        host: Host,
        server_cert: String,
        webrtc: &WebRtcConfig,
        mut req: StreamRequest,
    ) -> anyhow::Result<(String, VideoCodec)> {
        let mut active = self.0.lock().await;
        if let Some(old) = active.take() {
            tracing::info!("replacing the active stream");
            old.stop.notify_one();
            let _ = old.done.await;
        }

        // Pick the codec before answering: the answer can only carry one.
        let info = nv.server_info_https(&host.endpoint(), &server_cert).await?;
        let offered = codec::offered_video_codecs(&req.offer_sdp);
        let video = codec::choose(&req.codecs, &offered, info.codec_mode_support).ok_or_else(|| {
            let want: Vec<_> = req.codecs.iter().map(|c| c.label()).collect();
            anyhow::anyhow!(
                "no common video codec: you asked for {}, but {} can't encode {} or this browser can't decode {}",
                want.join("/"),
                host.name,
                if want.len() == 1 { "it" } else { "them" },
                if want.len() == 1 { "it" } else { "them" },
            )
        })?;
        tracing::info!(codec = video.codec.label(), "negotiated video codec");

        let offer = std::mem::take(&mut req.offer_sdp);
        let (peer, answer, peer_events) = Peer::answer(offer, &video, webrtc).await?;
        let stop = Arc::new(Notify::new());
        let done =
            tokio::spawn(run(nv, host, server_cert, req, video.codec, Arc::new(peer), peer_events, stop.clone()));
        *active = Some(Active { stop, done });
        Ok((answer, video.codec))
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ControlOut<'a> {
    Stage { stage: &'a str },
    Started { width: u32, height: u32, fps: u32 },
    Status { poor: bool },
    Failed { error: &'a str, retryable: bool },
    /// `retryable`: the stream dropped for reasons a reconnect may fix (vs. the app
    /// being closed, another tab taking over, ...).
    Ended { reason: Option<&'a str>, retryable: bool },
    Rumble { controller: u16, low: u16, high: u16 },
}

async fn send(control: &Arc<dyn DataChannel>, msg: ControlOut<'_>) {
    let json = serde_json::to_string(&msg).expect("serialize control message");
    if let Err(e) = control.send_text(&json).await {
        tracing::debug!("control send failed: {e}");
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    nv: NvClient,
    host: Host,
    server_cert: String,
    req: StreamRequest,
    video_codec: VideoCodec,
    peer: Arc<Peer>,
    mut peer_events: mpsc::UnboundedReceiver<PeerEvent>,
    stop: Arc<Notify>,
) {
    // 1. Wait for the browser's control channel. Input channels open alongside it; keep
    //    them until the host connection is up.
    let mut input_channels = Vec::new();
    let wait_for_control = tokio::time::timeout(CONTROL_TIMEOUT, async {
        while let Some(ev) = peer_events.recv().await {
            if let PeerEvent::DataChannel(dc) = ev {
                match dc.label().await.as_deref() {
                    Ok("control") => return Some(dc),
                    Ok(label) if label.starts_with("input") => input_channels.push(dc),
                    Ok(label) => tracing::debug!(label, "ignoring unknown data channel"),
                    Err(_) => {}
                }
            }
        }
        None
    });
    // A newer session may replace this one before the browser even connects.
    let control = tokio::select! {
        res = wait_for_control => res.ok().flatten(),
        _ = stop.notified() => None,
    };
    let Some(control) = control else {
        tracing::warn!("browser never opened the control channel");
        peer.close().await;
        return;
    };

    // Browser → bridge control messages. Currently only {"type":"stop"}.
    let browser_stop = Arc::new(Notify::new());
    tokio::spawn({
        let control = Arc::clone(&control);
        let browser_stop = Arc::clone(&browser_stop);
        async move {
            while let Some(ev) = control.poll().await {
                match ev {
                    DataChannelEvent::OnMessage(m) if m.is_string && m.data.as_ref() == br#"{"type":"stop"}"# => {
                        browser_stop.notify_one();
                    }
                    DataChannelEvent::OnClose => break,
                    _ => {}
                }
            }
            // Channel closed: the page went away.
            browser_stop.notify_one();
        }
    });

    let outcome = stream(
        &nv,
        &host,
        &server_cert,
        &req,
        video_codec,
        &peer,
        &mut peer_events,
        input_channels,
        &control,
        &stop,
        &browser_stop,
    )
    .await;
    match &outcome {
        Ok(end) => {
            tracing::info!(host = %host.name, reason = end.reason.unwrap_or("stopped"), end.retryable, "stream ended");
            send(&control, ControlOut::Ended { reason: end.reason, retryable: end.retryable }).await;
        }
        Err(e) => {
            tracing::warn!(host = %host.name, "stream failed: {e:#}");
            send(&control, ControlOut::Failed { error: &format!("{e:#}"), retryable: false }).await;
        }
    }
    // Let the final message flush before tearing down the transport.
    tokio::time::sleep(Duration::from_millis(100)).await;
    peer.close().await;
}

/// How a stream that got going ended.
struct End {
    /// Shown to the user; None when the user ended it themselves.
    reason: Option<&'static str>,
    retryable: bool,
}

impl End {
    const STOPPED: Self = Self { reason: None, retryable: false };
}

/// Runs the stream until it ends. `Err` only for failures while setting it up.
#[allow(clippy::too_many_arguments)]
async fn stream(
    nv: &NvClient,
    host: &Host,
    server_cert: &str,
    req: &StreamRequest,
    video_codec: VideoCodec,
    peer: &Arc<Peer>,
    peer_events: &mut mpsc::UnboundedReceiver<PeerEvent>,
    input_channels: Vec<Arc<dyn DataChannel>>,
    control: &Arc<dyn DataChannel>,
    stop: &Notify,
    browser_stop: &Notify,
) -> anyhow::Result<End> {
    let ep = host.endpoint();

    send(control, ControlOut::Stage { stage: "Launching app" }).await;
    let info = nv.server_info_https(&ep, server_cert).await?;
    if info.current_game != 0 && info.current_game != req.app_id {
        anyhow::bail!("another app is running on {}; quit it first", host.name);
    }

    let mut ri_key = [0u8; 16];
    let mut ri_iv = [0u8; 16];
    openssl::rand::rand_bytes(&mut ri_key)?;
    openssl::rand::rand_bytes(&mut ri_iv)?;
    let audio_configuration = moonlight_common_sys::AUDIO_CONFIGURATION_STEREO;

    let rtsp_url = nv
        .launch(
            &ep,
            server_cert,
            &LaunchParams {
                app_id: req.app_id,
                resume: info.current_game == req.app_id,
                width: req.width,
                height: req.height,
                fps: req.fps,
                audio_configuration,
                ri_key,
                ri_key_id: i32::from_be_bytes(ri_iv[..4].try_into().unwrap()),
            },
        )
        .await?;

    // 2. Connect. Stage events stream to the browser while the blocking handshake runs.
    let (video_tx, mut video_rx) = mpsc::channel::<VideoFrame>(VIDEO_QUEUE);
    let (audio_tx, mut audio_rx) = mpsc::channel::<AudioFrame>(AUDIO_QUEUE);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<StreamEvent>();
    let params = ConnectionParams {
        address: host.address.clone(),
        app_version: info.app_version.clone(),
        gfe_version: info.gfe_version.clone(),
        rtsp_url,
        server_codec_mode_support: info.codec_mode_support as i32,
        width: req.width,
        height: req.height,
        fps: req.fps,
        bitrate_kbps: req.bitrate_kbps,
        video_formats: video_codec.video_format(),
        audio_configuration,
        ri_key,
        ri_iv,
    };
    let mut connecting = tokio::task::spawn_blocking(move || Connection::start(params, video_tx, audio_tx, events_tx));
    let connection = loop {
        tokio::select! {
            res = &mut connecting => break res??,
            Some(ev) = events_rx.recv() => match ev {
                StreamEvent::StageStarting(stage) => send(control, ControlOut::Stage { stage: &stage }).await,
                StreamEvent::StageFailed { stage, code } => tracing::warn!(%stage, code, "connection stage failed"),
                _ => {}
            },
        }
    };
    // Make sure the connection is torn down (on a blocking thread) however we leave.
    let connection = ConnectionGuard(Some(connection));

    send(control, ControlOut::Started { width: req.width, height: req.height, fps: req.fps }).await;
    peer.on_keyframe_request(Connection::request_idr);
    Connection::request_idr();

    // Audio gets its own task so a back-pressured video write never delays it.
    let audio_pump = tokio::spawn({
        let peer = Arc::clone(peer);
        async move {
            while let Some(frame) = audio_rx.recv().await {
                if let Err(e) = peer.write_audio(frame.data, frame.duration).await {
                    tracing::debug!("audio write failed: {e}");
                }
            }
        }
    });
    for dc in input_channels {
        spawn_input_reader(dc);
    }

    // 3. Pump frames until something ends the session.
    //
    // RTP timestamps are the running sum of sample durations. A fixed 1/fps would drift
    // behind real time whenever the host sends fewer frames (it only encodes on change),
    // and the browser would grow its jitter buffer to absorb the "lateness". Instead each
    // frame advances the clock by the real gap since the previous frame (applied one
    // frame late, since the packetizer adds a sample's duration after it), so timestamps
    // track arrival time and the browser has nothing to buffer for.
    let nominal = Duration::from_secs_f64(1.0 / req.fps.max(1) as f64);
    let mut last_frame: Option<std::time::Instant> = None;
    // P-frames are useless to the browser until it has decoded a keyframe.
    let mut waiting_for_idr = true;
    let end = loop {
        tokio::select! {
            frame = video_rx.recv() => {
                let Some(frame) = frame else {
                    break End { reason: Some("The connection to the host closed."), retryable: true };
                };
                if waiting_for_idr && !frame.idr {
                    continue;
                }
                waiting_for_idr = false;
                let gap = last_frame.map_or(nominal, |prev| {
                    frame.received.saturating_duration_since(prev).clamp(Duration::from_millis(1), Duration::from_secs(1))
                });
                last_frame = Some(frame.received);
                if let Err(e) = peer.write_video(frame.data, gap, frame.received).await {
                    tracing::debug!("video write failed: {e}");
                }
            }
            Some(ev) = events_rx.recv() => match ev {
                StreamEvent::Terminated { code } => break termination(code),
                StreamEvent::ConnectionPoor(poor) => send(control, ControlOut::Status { poor }).await,
                StreamEvent::Rumble { controller, low, high } => {
                    send(control, ControlOut::Rumble { controller, low, high }).await;
                }
                _ => {}
            },
            Some(ev) = peer_events.recv() => match ev {
                PeerEvent::State(RTCPeerConnectionState::Connected) => {
                    waiting_for_idr = true;
                    Connection::request_idr();
                }
                PeerEvent::State(RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed) => break End::STOPPED,
                PeerEvent::DataChannel(dc) if dc.label().await.is_ok_and(|l| l.starts_with("input")) => {
                    spawn_input_reader(dc);
                }
                _ => {}
            },
            _ = browser_stop.notified() => break End::STOPPED,
            _ = stop.notified() => {
                break End { reason: Some("The stream was opened in another tab."), retryable: false };
            }
        }
    };
    audio_pump.abort();
    connection.stop().await;
    Ok(end)
}

/// Decode input messages from one data channel and forward them to the host.
/// Ends by itself when the channel (or the whole peer connection) closes.
fn spawn_input_reader(dc: Arc<dyn DataChannel>) {
    tokio::spawn(async move {
        while let Some(ev) = dc.poll().await {
            match ev {
                DataChannelEvent::OnMessage(m) if !m.is_string => match input::decode(&m.data) {
                    Some(event) => input::apply(&event),
                    None => tracing::debug!(len = m.data.len(), "malformed input message"),
                },
                DataChannelEvent::OnClose => break,
                _ => {}
            }
        }
    });
}

/// Stops the moonlight-common-c connection off the async runtime (LiStopConnection blocks).
struct ConnectionGuard(Option<Connection>);

impl ConnectionGuard {
    async fn stop(mut self) {
        if let Some(c) = self.0.take() {
            let _ = tokio::task::spawn_blocking(move || c.stop()).await;
        }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        if let Some(c) = self.0.take() {
            tokio::task::spawn_blocking(move || c.stop());
        }
    }
}

/// Map moonlight-common-c's termination code to a message and whether reconnecting helps.
fn termination(code: i32) -> End {
    use moonlight_common_sys as ml;
    // bindgen types the 0-valued #define as u32 and the negative ones as i32.
    const GRACEFUL: i32 = ml::ML_ERROR_GRACEFUL_TERMINATION as i32;
    let (reason, retryable) = match code {
        GRACEFUL => ("The app was closed on the host.", false),
        // A firewall problem: retrying would just fail the same way.
        ml::ML_ERROR_NO_VIDEO_TRAFFIC => (
            "No video was received from the host. Check that UDP ports 47998-48000 aren't blocked by its firewall.",
            false,
        ),
        ml::ML_ERROR_PROTECTED_CONTENT => ("The host is showing protected content that can't be streamed.", false),
        ml::ML_ERROR_NO_VIDEO_FRAME => ("The host stopped sending video.", true),
        ml::ML_ERROR_UNEXPECTED_EARLY_TERMINATION => ("The host ended the stream unexpectedly.", true),
        ml::ML_ERROR_FRAME_CONVERSION => ("The host failed to encode video.", true),
        _ => ("The connection to the host was lost.", true),
    };
    End { reason: Some(reason), retryable }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonlight_common_sys as ml;

    #[test]
    fn classifies_terminations() {
        assert!(!termination(0).retryable, "app closed on host");
        assert!(!termination(ml::ML_ERROR_NO_VIDEO_TRAFFIC).retryable, "firewall");
        assert!(termination(ml::ML_ERROR_UNEXPECTED_EARLY_TERMINATION).retryable);
        assert!(termination(-12345).retryable, "unknown codes: network loss");
    }
}
