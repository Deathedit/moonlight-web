//! Video over the "video" data channel, for browsers that decode it themselves with
//! WebCodecs. That skips the browser's jitter buffer and frame scheduling entirely: each
//! frame is drawn as soon as it decodes.
//!
//! The channel is reliable and ordered, so a frame is never half-delivered and the
//! decoder never sees a gap. Frames are split into messages below the browser's
//! max-message-size (Chrome: 256 KiB). Each message starts with one flags byte; the
//! browser joins the payloads from a FIRST message up to and including a LAST one.
//! Mirrors `web/src/lib/stream/video-worker.ts`.

use std::sync::Arc;

use bytes::{BufMut, BytesMut};
use webrtc::data_channel::{DataChannel, DataChannelEvent};

pub const FIRST: u8 = 0x01;
pub const LAST: u8 = 0x02;
pub const KEYFRAME: u8 = 0x04;

/// Payload bytes per message.
const CHUNK: usize = 64 * 1024 - 1;

/// Split one encoded frame into channel messages.
pub fn messages(frame: &[u8], keyframe: bool) -> Vec<BytesMut> {
    let parts: Vec<&[u8]> = if frame.is_empty() { vec![&[][..]] } else { frame.chunks(CHUNK).collect() };
    let n = parts.len();
    parts
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            let mut flags = if keyframe { KEYFRAME } else { 0 };
            if i == 0 {
                flags |= FIRST;
            }
            if i == n - 1 {
                flags |= LAST;
            }
            let mut msg = BytesMut::with_capacity(1 + part.len());
            msg.put_u8(flags);
            msg.put_slice(part);
            msg
        })
        .collect()
}

pub struct VideoChannel {
    dc: Arc<dyn DataChannel>,
    /// Above this many unacknowledged bytes the link isn't keeping up, and queueing more
    /// would only add delay.
    max_backlog: usize,
}

pub enum Sent {
    Ok,
    /// The frame was dropped because the link is behind; the stream needs an IDR.
    Dropped,
}

impl VideoChannel {
    pub fn new(dc: Arc<dyn DataChannel>, bitrate_kbps: u32) -> Self {
        // Nothing is read from this channel, but its events (buffered-amount-low, ...)
        // must still be consumed: once a held channel's event queue is full, webrtc stops
        // reading *every* data channel on the connection, input included.
        tokio::spawn({
            let dc = Arc::clone(&dc);
            async move {
                while let Some(ev) = dc.poll().await {
                    if matches!(ev, DataChannelEvent::OnClose) {
                        break;
                    }
                }
            }
        });
        // ~100 ms at the stream's bitrate, but always room for a large keyframe.
        let max_backlog = (bitrate_kbps as usize * 1000 / 8 / 10).max(1 << 20);
        Self { dc, max_backlog }
    }

    pub async fn send(&self, frame: &[u8], keyframe: bool) -> anyhow::Result<Sent> {
        if self.dc.outstanding_bytes().await? > self.max_backlog {
            return Ok(Sent::Dropped);
        }
        for msg in messages(frame, keyframe) {
            self.dc.send(msg).await?;
        }
        Ok(Sent::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_flags_frames() {
        let small = messages(&[1, 2, 3], true);
        assert_eq!(small.len(), 1);
        assert_eq!(&small[0][..], &[FIRST | LAST | KEYFRAME, 1, 2, 3]);

        let frame: Vec<u8> = (0..CHUNK * 2 + 10).map(|i| i as u8).collect();
        let msgs = messages(&frame, false);
        let flags: Vec<u8> = msgs.iter().map(|m| m[0]).collect();
        assert_eq!(flags, vec![FIRST, 0, LAST]);
        assert!(msgs.iter().all(|m| m.len() <= 64 * 1024));
        let joined: Vec<u8> = msgs.iter().flat_map(|m| m[1..].to_vec()).collect();
        assert_eq!(joined, frame);
    }
}

/// End to end over loopback: a bridge peer answering a WebCodecs-style offer (audio and
/// data channels, no video m-line) delivers frames over "video" that reassemble intact,
/// and keeps reading the browser's other channels however long video flows.
#[cfg(test)]
mod loopback {
    use std::time::Duration;

    use rtc::{
        peer_connection::configuration::media_engine::MIME_TYPE_OPUS,
        rtp_transceiver::{
            rtp_sender::{RTCRtpCodec, RTCRtpCodecParameters, RtpCodecKind},
            RTCRtpTransceiverDirection, RTCRtpTransceiverInit,
        },
    };
    use tokio::sync::mpsc;
    use webrtc::{
        data_channel::DataChannel,
        peer_connection::{
            MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
            RTCSessionDescription,
        },
    };

    use super::*;
    use crate::{
        config::WebRtcConfig,
        stream::peer::{Peer, PeerEvent},
    };

    struct Browser {
        gathered: mpsc::Sender<()>,
    }

    #[async_trait::async_trait]
    impl PeerConnectionEventHandler for Browser {
        async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
            if state == RTCIceGatheringState::Complete {
                let _ = self.gathered.try_send(());
            }
        }
    }

    #[tokio::test]
    async fn frames_arrive_whole_over_the_video_channel() {
        let mut media = MediaEngine::default();
        let opus = RTCRtpCodec {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: 48_000,
            channels: 2,
            sdp_fmtp_line: String::new(),
            rtcp_feedback: vec![],
        };
        media.register_codec(RTCRtpCodecParameters { rtp_codec: opus, payload_type: 111 }, RtpCodecKind::Audio).unwrap();
        let (gathered_tx, mut gathered_rx) = mpsc::channel(1);
        let browser = PeerConnectionBuilder::new()
            .with_media_engine(media)
            .with_handler(Arc::new(Browser { gathered: gathered_tx }))
            .with_udp_addrs(vec!["127.0.0.1:0".to_owned()])
            .build()
            .await
            .unwrap();
        let init = RTCRtpTransceiverInit { direction: RTCRtpTransceiverDirection::Recvonly, ..Default::default() };
        browser.add_transceiver_from_kind(RtpCodecKind::Audio, Some(init)).await.unwrap();
        let rx = browser.create_data_channel("video", None).await.unwrap();
        let input_tx = browser.create_data_channel("input", None).await.unwrap();
        let offer = browser.create_offer(None).await.unwrap();
        browser.set_local_description(offer).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), gathered_rx.recv()).await;
        let offer = browser.local_description().await.unwrap().sdp;

        let cfg = WebRtcConfig { host_ips: vec!["127.0.0.1".into()], ..Default::default() };
        let (peer, answer, mut events) = Peer::answer(offer, None, &cfg).await.unwrap();
        assert!(!answer.contains("m=video"), "no video track without a video m-line");
        browser.set_remote_description(RTCSessionDescription::answer(answer).unwrap()).await.unwrap();

        let mut opened: Vec<Arc<dyn DataChannel>> = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while opened.len() < 2 {
                if let Some(PeerEvent::DataChannel(dc)) = events.recv().await {
                    opened.push(dc);
                }
            }
        })
        .await
        .expect("data channels never opened");
        let mut video = None;
        let mut input = None;
        for dc in opened {
            match dc.label().await.unwrap().as_str() {
                "video" => video = Some(dc),
                _ => input = Some(dc),
            }
        }
        let (video, input) = (video.unwrap(), input.unwrap());
        let channel = VideoChannel::new(video, 20_000);

        // A large keyframe (several messages) and a small delta frame.
        let key: Vec<u8> = (0..300_000u32).map(|i| (i * 7) as u8).collect();
        let delta = vec![9u8; 1234];
        assert!(matches!(channel.send(&key, true).await.unwrap(), Sent::Ok));
        assert!(matches!(channel.send(&delta, false).await.unwrap(), Sent::Ok));

        let mut frames: Vec<(bool, Vec<u8>)> = Vec::new();
        let mut current = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while frames.len() < 2 {
                let Some(DataChannelEvent::OnMessage(m)) = rx.poll().await else { continue };
                let flags = m.data[0];
                if flags & FIRST != 0 {
                    current.clear();
                }
                current.extend_from_slice(&m.data[1..]);
                if flags & LAST != 0 {
                    frames.push((flags & KEYFRAME != 0, std::mem::take(&mut current)));
                }
            }
        })
        .await
        .expect("frames didn't arrive");

        // Each acknowledged frame raises events on the video channel. Well past the
        // driver's event queue (256), the browser's input must still get through.
        for _ in 0..600 {
            channel.send(&delta, false).await.unwrap();
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        input_tx.send(bytes::BytesMut::from(&b"click"[..])).await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(DataChannelEvent::OnMessage(m)) = input.poll().await {
                    break m.data;
                }
            }
        })
        .await
        .expect("input stalled behind unread video channel events");
        assert_eq!(&got[..], b"click");
        peer.close().await;
        let _ = browser.close().await;

        assert_eq!(frames, vec![(true, key), (false, delta)]);
    }
}
