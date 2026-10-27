//! WebRTC side of a stream (webrtc 0.21): one peer connection per session with an
//! H.264 video track; the browser is the offerer and opens the data channels.

use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use rtc::{
    interceptor::{Attribute, Interceptor, Packet, Slot, StreamInfo, TaggedPacket},
    media::Sample,
    peer_connection::{
        configuration::media_engine::MIME_TYPE_OPUS,
        transport::RTCDtlsRole,
    },
    rtcp::payload_feedbacks::{full_intra_request::FullIntraRequest, picture_loss_indication::PictureLossIndication},
    rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
    },
    sansio::Protocol,
};
use tokio::sync::mpsc;

use super::codec::OfferedCodec;
use crate::config::WebRtcConfig;
use webrtc::{
    data_channel::DataChannel,
    media_stream::{
        track_local::{static_sample::TrackLocalStaticSample, TrackLocal, TrackLocalEvent},
        MediaStreamTrack,
    },
    peer_connection::{
        register_default_interceptors, MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
        RTCConfigurationBuilder, RTCIceGatheringState, RTCIceServer, RTCPeerConnectionState, RTCSessionDescription,
        Registry, SettingEngineBuilder,
    },
};

/// How long to wait for our ICE candidates before answering anyway.
const GATHER_TIMEOUT: Duration = Duration::from_secs(3);

fn opus_codec() -> RTCRtpCodec {
    RTCRtpCodec {
        mime_type: MIME_TYPE_OPUS.to_owned(),
        clock_rate: 48_000,
        channels: 2,
        sdp_fmtp_line: "minptime=5;useinbandfec=1;stereo=1;sprop-stereo=1".to_owned(),
        rtcp_feedback: vec![],
    }
}

/// A send-only sample track plus what's needed to write to it.
struct OutTrack {
    track: Arc<TrackLocalStaticSample>,
    ssrc: u32,
    pt: u8,
}

impl OutTrack {
    /// `duration` is how far the RTP timestamp advances *after* this sample.
    async fn write(&self, data: Bytes, duration: Duration, at: Instant) -> anyhow::Result<()> {
        let sample = Sample { data, duration, ..Sample::new(at) };
        self.track.sample_writer(self.ssrc, self.pt).write_sample(&sample).await?;
        Ok(())
    }
}

/// Create a track in its own media stream and add it to the connection. Audio and video
/// are deliberately *not* in one stream: browsers lip-sync tracks of the same stream by
/// delaying video to match the (larger) audio buffer, which costs input latency.
async fn add_track(
    pc: &Arc<dyn PeerConnection>,
    kind: RtpCodecKind,
    id: &str,
    codec: RTCRtpCodec,
) -> anyhow::Result<(Arc<TrackLocalStaticSample>, u32, Arc<dyn webrtc::rtp_transceiver::RtpSender>)> {
    let ssrc = random_u32();
    let track = Arc::new(TrackLocalStaticSample::new(
        Instant::now(),
        MediaStreamTrack::new(
            format!("moonlight-{id}"),
            id.into(),
            id.into(),
            kind,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters { ssrc: Some(ssrc), ..Default::default() },
                codec,
                ..Default::default()
            }],
        ),
    )?);
    let sender = pc.add_track(Arc::clone(&track) as Arc<dyn TrackLocal>).await?;
    Ok((track, ssrc, sender))
}

/// The payload type the browser accepted for a sender (only known after negotiation).
async fn negotiated_pt(sender: &Arc<dyn webrtc::rtp_transceiver::RtpSender>) -> Option<u8> {
    sender.get_parameters().await.ok()?.rtp_parameters.codecs.first().map(|c| c.payload_type)
}

/// Events from the peer connection, delivered to the session task.
pub enum PeerEvent {
    State(RTCPeerConnectionState),
    DataChannel(Arc<dyn DataChannel>),
}

/// Handlers run inline on the connection's driver task, so they only forward.
struct Handler {
    gathered: mpsc::Sender<()>,
    events: mpsc::UnboundedSender<PeerEvent>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.gathered.try_send(());
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        let _ = self.events.send(PeerEvent::State(state));
    }

    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        let _ = self.events.send(PeerEvent::DataChannel(dc));
    }
}

pub struct Peer {
    pub pc: Arc<dyn PeerConnection>,
    video: OutTrack,
    /// None if the browser didn't offer to receive audio.
    audio: Option<OutTrack>,
}

impl Peer {
    /// Answer the browser's offer with `video` (one of the codecs it offered, with its
    /// exact payload type and fmtp, so negotiation can't mismatch). Returns the peer, its
    /// answer SDP and the event stream.
    pub async fn answer(
        offer_sdp: String,
        video: &OfferedCodec,
        cfg: &WebRtcConfig,
    ) -> anyhow::Result<(Self, String, mpsc::UnboundedReceiver<PeerEvent>)> {
        let mut media = MediaEngine::default();
        media.register_codec(
            RTCRtpCodecParameters { rtp_codec: video.rtp_codec(), payload_type: video.payload_type },
            RtpCodecKind::Video,
        )?;
        media.register_codec(RTCRtpCodecParameters { rtp_codec: opus_codec(), payload_type: 111 }, RtpCodecKind::Audio)?;
        // Codecs must be registered before this: it attaches NACK/PLI feedback to them.
        let registry = register_default_interceptors(Registry::new(), &mut media)?
            .with(Slot::from(14_000), KeyframeRequestForwarder::default());

        let ice = if cfg.ice_servers.is_empty() {
            vec![]
        } else {
            vec![RTCIceServer { urls: cfg.ice_servers.clone(), ..Default::default() }]
        };

        let (gathered_tx, mut gathered_rx) = mpsc::channel(1);
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        // Answer as DTLS server (a=setup:passive). In rtc the DTLS server side is also the
        // SCTP server; as DTLS client we'd refuse the browser's SCTP INIT and data channels
        // would only open after retransmits (seconds in Chrome, never with some clients).
        let settings = SettingEngineBuilder::new().with_answering_dtls_role(RTCDtlsRole::Server).build();

        let pc = PeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::new().with_ice_servers(ice).build())
            .with_setting_engine(settings)
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .with_handler(Arc::new(Handler { gathered: gathered_tx, events: events_tx }))
            .with_udp_addrs(vec![format!("0.0.0.0:{}", cfg.udp_port)])
            .build()
            .await?;
        let pc: Arc<dyn PeerConnection> = Arc::new(pc);

        let result = async {
            let (video_track, video_ssrc, video_sender) =
                add_track(&pc, RtpCodecKind::Video, "video", video.rtp_codec()).await?;
            let (audio, audio_ssrc, audio_sender) = add_track(&pc, RtpCodecKind::Audio, "audio", opus_codec()).await?;

            pc.set_remote_description(RTCSessionDescription::offer(offer_sdp)?).await?;
            let answer = pc.create_answer(None).await?;
            pc.set_local_description(answer).await?;
            let _ = tokio::time::timeout(GATHER_TIMEOUT, gathered_rx.recv()).await;

            let video_pt = negotiated_pt(&video_sender)
                .await
                .ok_or_else(|| anyhow::anyhow!("browser didn't accept {} video", video.codec.label()))?;
            let video = OutTrack { track: video_track, ssrc: video_ssrc, pt: video_pt };
            let audio = negotiated_pt(&audio_sender).await.map(|pt| OutTrack { track: audio, ssrc: audio_ssrc, pt });
            if audio.is_none() {
                tracing::warn!("browser didn't negotiate Opus audio; streaming video only");
            }
            let mut sdp = pc
                .local_description()
                .await
                .ok_or_else(|| anyhow::anyhow!("no local description"))?
                .sdp;
            if !cfg.host_ips.is_empty() {
                // rtc 0.21 accepts SettingEngine nat_1to1_ips but never applies them, so
                // swap the addresses in our (non-trickle) answer ourselves.
                sdp = advertise_host_ips(&sdp, &cfg.host_ips);
            }
            anyhow::Ok((video, audio, sdp))
        }
        .await;

        match result {
            Ok((video, audio, sdp)) => Ok((Self { pc, video, audio }, sdp, events_rx)),
            Err(e) => {
                let _ = pc.close().await;
                Err(e)
            }
        }
    }

    /// Write one encoded frame. Applies back-pressure when the transport is behind.
    pub async fn write_video(&self, data: Bytes, duration: Duration, at: Instant) -> anyhow::Result<()> {
        self.video.write(data, duration, at).await
    }

    /// Write one Opus packet (no-op if audio wasn't negotiated).
    pub async fn write_audio(&self, data: Bytes, duration: Duration) -> anyhow::Result<()> {
        match &self.audio {
            Some(audio) => audio.write(data, duration, Instant::now()).await,
            None => Ok(()),
        }
    }

    /// Calls `on_request` whenever the browser asks for a keyframe (PLI/FIR).
    pub fn on_keyframe_request(&self, on_request: impl Fn() + Send + 'static) {
        let track = Arc::clone(&self.video.track);
        tokio::spawn(async move {
            while let Some(event) = track.poll().await {
                if let TrackLocalEvent::OnRtcpPacket(pkts) = event {
                    if pkts.iter().any(|p| is_keyframe_request(p.as_ref())) {
                        on_request();
                    }
                }
            }
        });
    }

    pub async fn close(&self) {
        if let Err(e) = self.pc.close().await {
            tracing::debug!("closing peer connection: {e}");
        }
    }
}

/// Replace the addresses of our host candidates with `ips` (one candidate per IP and
/// port), e.g. a Docker host's LAN address in place of the container's. ICE checks are
/// authenticated by ufrag/password, so traffic forwarded to the container still matches.
fn advertise_host_ips(sdp: &str, ips: &[String]) -> String {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in sdp.split("\r\n") {
        let Some(rest) = line.strip_prefix("a=candidate:") else {
            out.push(line.to_string());
            continue;
        };
        // foundation component transport priority address port "typ" type ...
        let fields: Vec<&str> = rest.split(' ').collect();
        if fields.len() < 8 || fields[6] != "typ" || fields[7] != "host" {
            out.push(line.to_string());
            continue;
        }
        for ip in ips {
            // Several interfaces yield several candidates on the same port: emit each
            // advertised address once per component/transport/port.
            if !seen.insert((fields[1].to_string(), fields[2].to_lowercase(), ip.clone(), fields[5].to_string())) {
                continue;
            }
            let mut f = fields.clone();
            f[4] = ip;
            out.push(format!("a=candidate:{}", f.join(" ")));
        }
    }
    out.join("\r\n")
}

fn random_u32() -> u32 {
    let mut b = [0u8; 4];
    openssl::rand::rand_bytes(&mut b).expect("rng");
    u32::from_le_bytes(b)
}

fn is_keyframe_request(p: &dyn rtc::rtcp::Packet) -> bool {
    let any = p.as_any();
    any.is::<PictureLossIndication>() || any.is::<FullIntraRequest>()
}

/// The default interceptor chain ends inbound RTCP before it reaches the application.
/// This passes PLI/FIR through (tagged DeliverToApplication) so we can request IDR frames.
#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 102\r\nc=IN IP4 0.0.0.0\r\n\
a=candidate:1 1 udp 2130706431 172.18.0.5 8080 typ host\r\n\
a=candidate:2 1 udp 2130706431 10.0.0.9 8080 typ host\r\n\
a=candidate:3 1 udp 1694498815 198.51.100.1 40000 typ srflx raddr 0.0.0.0 rport 8080\r\n\
a=end-of-candidates\r\n";

    #[test]
    fn rewrites_host_candidates() {
        let out = advertise_host_ips(ANSWER, &["192.168.1.10".into()]);
        let cands: Vec<_> = out.lines().filter(|l| l.starts_with("a=candidate:")).collect();
        assert_eq!(
            cands,
            vec![
                "a=candidate:1 1 udp 2130706431 192.168.1.10 8080 typ host",
                // Non-host candidates are left alone.
                "a=candidate:3 1 udp 1694498815 198.51.100.1 40000 typ srflx raddr 0.0.0.0 rport 8080",
            ]
        );
        assert!(out.contains("a=end-of-candidates"));
        assert!(!out.contains("172.18.0.5") && !out.contains("10.0.0.9"));
    }

    #[test]
    fn one_candidate_per_advertised_ip() {
        let out = advertise_host_ips(ANSWER, &["192.168.1.10".into(), "fd00::10".into()]);
        assert_eq!(out.matches("typ host").count(), 2);
        assert!(out.contains(" fd00::10 8080 typ host"));
    }
}

#[derive(Default)]
struct KeyframeRequestForwarder {
    read: VecDeque<TaggedPacket>,
    write: VecDeque<TaggedPacket>,
}

impl Protocol<TaggedPacket, TaggedPacket, ()> for KeyframeRequestForwarder {
    type Rout = TaggedPacket;
    type Wout = TaggedPacket;
    type Eout = ();
    type Error = rtc::shared::error::Error;
    type Time = Instant;

    fn handle_read(&mut self, mut msg: TaggedPacket) -> Result<(), Self::Error> {
        if let Packet::Rtcp(pkts) = &msg.message.packet {
            let keep: Vec<_> = pkts.iter().filter(|p| is_keyframe_request(p.as_ref())).cloned().collect();
            if keep.is_empty() {
                return Ok(());
            }
            msg.message.packet = Packet::Rtcp(keep);
            msg.message.add(Attribute::DeliverToApplication);
        }
        self.read.push_back(msg);
        Ok(())
    }

    fn poll_read(&mut self) -> Option<TaggedPacket> {
        self.read.pop_front()
    }

    fn handle_write(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> {
        self.write.push_back(msg);
        Ok(())
    }

    fn poll_write(&mut self) -> Option<TaggedPacket> {
        self.write.pop_front()
    }
}

impl Interceptor for KeyframeRequestForwarder {
    fn bind_local_stream(&mut self, _: &StreamInfo) {}
    fn unbind_local_stream(&mut self, _: &StreamInfo) {}
    fn bind_remote_stream(&mut self, _: &StreamInfo) {}
    fn unbind_remote_stream(&mut self, _: &StreamInfo) {}
}
