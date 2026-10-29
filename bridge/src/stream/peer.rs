//! WebRTC side of a stream (webrtc 0.21): one peer connection per session with an
//! H.264 video track; the browser is the offerer and opens the data channels.

use std::{
    collections::VecDeque,
    sync::{Arc, OnceLock},
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
        RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
        RTCRtpHeaderExtensionCapability, RtpCodecKind,
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

/// Playout delay min = max = 0 puts Chrome into its low-latency path: render each frame as
/// soon as it decodes, ignoring RTP timing. Without it Chrome adds its own jitter estimate
/// on top of `jitterBufferTarget`, and that estimate keeps growing on a desktop, where the
/// host only sends frames when something changes. Browsers that don't offer it (Firefox)
/// simply don't negotiate it.
const PLAYOUT_DELAY_URI: &str = "http://www.webrtc.org/experiments/rtp-hdrext/playout-delay";

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
        media.register_header_extension(
            RTCRtpHeaderExtensionCapability { uri: PLAYOUT_DELAY_URI.to_owned() },
            RtpCodecKind::Video,
            None,
        )?;
        let playout_delay = Arc::new(OnceLock::new());
        // Codecs must be registered before this: it attaches NACK/PLI feedback to them.
        // Writes walk the slots high to low, so the stamper runs before the NACK responder
        // stores packets and retransmissions carry the extension too.
        let registry = register_default_interceptors(Registry::new(), &mut media)?
            .with(Slot::from(14_000), KeyframeRequestForwarder::default())
            .with(Slot::from(14_500), PlayoutDelayStamper { target: Arc::clone(&playout_delay), ..Default::default() });

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
            match video_extension_id(&sdp, PLAYOUT_DELAY_URI) {
                Some(id) => {
                    let _ = playout_delay.set((video_ssrc, id));
                }
                None => tracing::info!("browser didn't negotiate playout-delay; it may buffer video for smoothness"),
            }
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
    use rtc::{
        peer_connection::configuration::media_engine::MIME_TYPE_H264,
        rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit},
        sdp::extmap::{ABS_SEND_TIME_URI, TRANSPORT_CC_URI},
    };
    use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};

    #[test]
    fn finds_video_extension_id() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=extmap:3 urn:x\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 102\r\na=extmap:2 urn:y\r\na=extmap:7/sendonly urn:x\r\n";
        assert_eq!(video_extension_id(sdp, "urn:x"), Some(7), "video section only, direction suffix ok");
        assert_eq!(video_extension_id(sdp, "urn:z"), None);
        assert_eq!(video_extension_id("m=video 9 x 1\r\na=extmap:15 urn:x\r\n", "urn:x"), None, "two-byte id");
    }

    /// Browser stand-in: forwards the video RTP it receives.
    struct Browser {
        gathered: mpsc::Sender<()>,
        rtp: mpsc::UnboundedSender<rtc::rtp::Packet>,
    }

    #[async_trait::async_trait]
    impl PeerConnectionEventHandler for Browser {
        async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
            if state == RTCIceGatheringState::Complete {
                let _ = self.gathered.try_send(());
            }
        }

        async fn on_track(&self, track: Arc<dyn TrackRemote>) {
            let rtp = self.rtp.clone();
            tokio::spawn(async move {
                while let Some(ev) = track.poll().await {
                    if let TrackRemoteEvent::OnRtpPacket(p) = ev {
                        let _ = rtp.send(p);
                    }
                }
            });
        }
    }

    /// End to end over loopback: video packets must actually arrive, stamped with
    /// playout-delay 0/0 under the id our answer gave it.
    #[tokio::test]
    async fn video_arrives_with_zero_playout_delay() {
        let mut media = MediaEngine::default();
        let h264 = RTCRtpCodec {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_owned(),
            rtcp_feedback: vec![],
        };
        media.register_codec(RTCRtpCodecParameters { rtp_codec: h264, payload_type: 102 }, RtpCodecKind::Video).unwrap();
        media.register_codec(RTCRtpCodecParameters { rtp_codec: opus_codec(), payload_type: 111 }, RtpCodecKind::Audio).unwrap();
        // Others first, so playout-delay's id differs from the one the bridge picks before
        // negotiation (the mismatch that once dropped every packet).
        for uri in [ABS_SEND_TIME_URI, TRANSPORT_CC_URI, "urn:3gpp:video-orientation", PLAYOUT_DELAY_URI] {
            let cap = RTCRtpHeaderExtensionCapability { uri: uri.to_owned() };
            media.register_header_extension(cap, RtpCodecKind::Video, None).unwrap();
        }

        let (gathered_tx, mut gathered_rx) = mpsc::channel(1);
        let (rtp_tx, mut rtp_rx) = mpsc::unbounded_channel();
        let browser = PeerConnectionBuilder::new()
            .with_media_engine(media)
            .with_handler(Arc::new(Browser { gathered: gathered_tx, rtp: rtp_tx }))
            .with_udp_addrs(vec!["127.0.0.1:0".to_owned()])
            .build()
            .await
            .unwrap();
        for kind in [RtpCodecKind::Video, RtpCodecKind::Audio] {
            let init = RTCRtpTransceiverInit { direction: RTCRtpTransceiverDirection::Recvonly, ..Default::default() };
            browser.add_transceiver_from_kind(kind, Some(init)).await.unwrap();
        }
        let offer = browser.create_offer(None).await.unwrap();
        browser.set_local_description(offer).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), gathered_rx.recv()).await;
        let offer = browser.local_description().await.unwrap().sdp;

        let codec = crate::stream::codec::offered_video_codecs(&offer).into_iter().next().expect("H.264 offered");
        let cfg = WebRtcConfig { host_ips: vec!["127.0.0.1".into()], ..Default::default() };
        let (peer, answer, _events) = Peer::answer(offer, &codec, &cfg).await.unwrap();
        let id = video_extension_id(&answer, PLAYOUT_DELAY_URI).expect("playout-delay negotiated");
        browser.set_remote_description(RTCSessionDescription::answer(answer).unwrap()).await.unwrap();

        // Keep sending until something arrives; ICE and DTLS take a moment.
        let idr = Bytes::from_static(&[0, 0, 0, 1, 0x65, 0x88, 0x84, 0x00, 0x33, 0xff]);
        let pkt = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let _ = peer.write_video(idr.clone(), Duration::from_millis(16), Instant::now()).await;
                if let Ok(Some(p)) = tokio::time::timeout(Duration::from_millis(20), rtp_rx.recv()).await {
                    break p;
                }
            }
        })
        .await
        .expect("no video reached the browser");
        peer.close().await;
        let _ = browser.close().await;

        assert_eq!(pkt.header.get_extension(id).as_deref(), Some(&[0u8, 0, 0][..]));
    }

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

/// The id `uri` got in the first video section of `sdp` (our answer: what the browser
/// will parse). Only one-byte header ids (1–14), which is all we write.
fn video_extension_id(sdp: &str, uri: &str) -> Option<u8> {
    let mut in_video = false;
    for line in sdp.lines().map(str::trim) {
        if let Some(m) = line.strip_prefix("m=") {
            if in_video {
                break;
            }
            in_video = m.starts_with("video ");
        } else if let Some(map) = line.strip_prefix("a=extmap:").filter(|_| in_video) {
            // a=extmap:<id>[/<direction>] <uri> [<attributes>]
            let mut parts = map.split_whitespace();
            let id = parts.next()?.split('/').next()?.parse().ok()?;
            if parts.next() == Some(uri) && (1..=14).contains(&id) {
                return Some(id);
            }
        }
    }
    None
}

/// Adds playout-delay 0/0 to outgoing video packets once the answer has fixed its id.
///
/// Not via the sample writer: webrtc 0.21 binds tracks before the offer is applied, so the
/// writer would use a pre-negotiation id that the sender then rejects ("extension not
/// found"), dropping every packet. Interceptors run after that check.
#[derive(Default)]
struct PlayoutDelayStamper {
    /// (video SSRC, extension id), set after negotiation.
    target: Arc<OnceLock<(u32, u8)>>,
    read: VecDeque<TaggedPacket>,
    write: VecDeque<TaggedPacket>,
}

impl Protocol<TaggedPacket, TaggedPacket, ()> for PlayoutDelayStamper {
    type Rout = TaggedPacket;
    type Wout = TaggedPacket;
    type Eout = ();
    type Error = rtc::shared::error::Error;
    type Time = Instant;

    fn handle_read(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> {
        self.read.push_back(msg);
        Ok(())
    }

    fn poll_read(&mut self) -> Option<TaggedPacket> {
        self.read.pop_front()
    }

    fn handle_write(&mut self, mut msg: TaggedPacket) -> Result<(), Self::Error> {
        if let (Some(&(ssrc, id)), Packet::Rtp(pkt)) = (self.target.get(), &mut msg.message.packet) {
            if pkt.header.ssrc == ssrc {
                // min = max = 0: both 12-bit fields zero.
                pkt.header.set_extension(id, Bytes::from_static(&[0, 0, 0]))?;
            }
        }
        self.write.push_back(msg);
        Ok(())
    }

    fn poll_write(&mut self) -> Option<TaggedPacket> {
        self.write.pop_front()
    }
}

impl Interceptor for PlayoutDelayStamper {
    fn bind_local_stream(&mut self, _: &StreamInfo) {}
    fn unbind_local_stream(&mut self, _: &StreamInfo) {}
    fn bind_remote_stream(&mut self, _: &StreamInfo) {}
    fn unbind_remote_stream(&mut self, _: &StreamInfo) {}
}
