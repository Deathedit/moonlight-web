//! Video codec negotiation between the browser (what it can decode, from its SDP offer)
//! and the host (what it can encode, from serverinfo's ServerCodecModeSupport).

use moonlight_common_sys as ml;
use rtc::{
    peer_connection::configuration::media_engine::{MIME_TYPE_AV1, MIME_TYPE_H264, MIME_TYPE_HEVC},
    rtp_transceiver::rtp_sender::RTCRtpCodec,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    H265,
    Av1,
}

impl VideoCodec {
    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::H265 => "HEVC",
            Self::Av1 => "AV1",
        }
    }

    fn sdp_name(self) -> &'static str {
        match self {
            Self::H264 => "H264",
            Self::H265 => "H265",
            Self::Av1 => "AV1",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Self::H264 => MIME_TYPE_H264,
            Self::H265 => MIME_TYPE_HEVC,
            Self::Av1 => MIME_TYPE_AV1,
        }
    }

    /// VIDEO_FORMAT_* to request from moonlight-common-c (8-bit 4:2:0 variants).
    pub fn video_format(self) -> i32 {
        (match self {
            Self::H264 => ml::VIDEO_FORMAT_H264,
            Self::H265 => ml::VIDEO_FORMAT_H265,
            Self::Av1 => ml::VIDEO_FORMAT_AV1_MAIN8,
        }) as i32
    }

    /// Whether the host's ServerCodecModeSupport (SCM_* bits) includes this codec.
    pub fn host_supports(self, scm: u32) -> bool {
        let bit = match self {
            Self::H264 => ml::SCM_H264,
            Self::H265 => ml::SCM_HEVC,
            Self::Av1 => ml::SCM_AV1_MAIN8,
        };
        scm & bit != 0
    }
}

/// A video codec the browser offered: its payload type and exact fmtp.
#[derive(Debug, Clone, PartialEq)]
pub struct OfferedCodec {
    pub codec: VideoCodec,
    pub payload_type: u8,
    pub fmtp: String,
}

impl OfferedCodec {
    pub fn rtp_codec(&self) -> RTCRtpCodec {
        RTCRtpCodec {
            mime_type: self.codec.mime().to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: self.fmtp.clone(),
            rtcp_feedback: vec![],
        }
    }
}

/// Video codecs in the offer's first video m-section, in offer order.
pub fn offered_video_codecs(sdp: &str) -> Vec<OfferedCodec> {
    let mut in_video = false;
    let mut seen_video = false;
    let mut rtpmap: Vec<(u8, VideoCodec)> = Vec::new();
    let mut fmtp: Vec<(u8, String)> = Vec::new();

    for line in sdp.lines().map(str::trim) {
        if let Some(m) = line.strip_prefix("m=") {
            if seen_video && in_video {
                break; // only the first video section
            }
            in_video = m.starts_with("video ");
            seen_video |= in_video;
            continue;
        }
        if !in_video {
            continue;
        }
        if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            let Some((pt, name)) = rest.split_once(' ') else { continue };
            let Ok(pt) = pt.parse() else { continue };
            let name = name.split('/').next().unwrap_or_default();
            let codec = [VideoCodec::H264, VideoCodec::H265, VideoCodec::Av1]
                .into_iter()
                .find(|c| c.sdp_name().eq_ignore_ascii_case(name));
            if let Some(codec) = codec {
                rtpmap.push((pt, codec));
            }
        } else if let Some(rest) = line.strip_prefix("a=fmtp:") {
            if let Some((pt, params)) = rest.split_once(' ') {
                if let Ok(pt) = pt.parse() {
                    fmtp.push((pt, params.to_string()));
                }
            }
        }
    }

    rtpmap
        .into_iter()
        .map(|(payload_type, codec)| OfferedCodec {
            codec,
            payload_type,
            fmtp: fmtp.iter().find(|(pt, _)| *pt == payload_type).map(|(_, f)| f.clone()).unwrap_or_default(),
        })
        // H.264 needs packetization-mode=1 (we send NAL units split across packets).
        .filter(|c| c.codec != VideoCodec::H264 || c.fmtp.contains("packetization-mode=1"))
        .collect()
}

/// Pick the first codec in `preference` that the browser offered and the host can encode.
/// For H.264, prefer the constrained-baseline entry every browser decodes.
pub fn choose(preference: &[VideoCodec], offered: &[OfferedCodec], host_scm: u32) -> Option<OfferedCodec> {
    preference.iter().filter(|c| c.host_supports(host_scm)).find_map(|&codec| {
        let mut matching = offered.iter().filter(|o| o.codec == codec);
        if codec == VideoCodec::H264 {
            let all: Vec<_> = matching.collect();
            all.iter()
                .find(|o| o.fmtp.contains("profile-level-id=42e01f"))
                .or_else(|| all.first())
                .map(|o| (*o).clone())
        } else {
            matching.next().cloned()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from a Chrome offer.
    const OFFER: &str = "v=0\r
m=video 9 UDP/TLS/RTP/SAVPF 96 102 103 45 49\r
a=rtpmap:96 VP8/90000\r
a=rtpmap:102 H264/90000\r
a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f\r
a=rtpmap:103 H264/90000\r
a=fmtp:103 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r
a=rtpmap:104 H264/90000\r
a=fmtp:104 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f\r
a=rtpmap:45 AV1/90000\r
a=fmtp:45 level-idx=5;profile=0;tier=0\r
a=rtpmap:49 H265/90000\r
a=fmtp:49 level-id=180;profile-id=1;tier-flag=0;tx-mode=SRST\r
m=audio 9 UDP/TLS/RTP/SAVPF 111\r
a=rtpmap:111 opus/48000/2\r
";

    // Sunshine on the user's host: 0x1F0301.
    const SCM_ALL: u32 = 2_032_385;

    #[test]
    fn parses_offer() {
        let offered = offered_video_codecs(OFFER);
        let summary: Vec<_> = offered.iter().map(|o| (o.codec, o.payload_type)).collect();
        // VP8 and the packetization-mode=0 H.264 entry are dropped.
        assert_eq!(
            summary,
            vec![(VideoCodec::H264, 102), (VideoCodec::H264, 103), (VideoCodec::Av1, 45), (VideoCodec::H265, 49)]
        );
        assert_eq!(offered[3].fmtp, "level-id=180;profile-id=1;tier-flag=0;tx-mode=SRST");
    }

    #[test]
    fn chooses_by_preference_and_host_support() {
        let offered = offered_video_codecs(OFFER);
        let pick = |pref: &[VideoCodec], scm| choose(pref, &offered, scm).map(|o| (o.codec, o.payload_type));

        assert_eq!(pick(&[VideoCodec::Av1, VideoCodec::H264], SCM_ALL), Some((VideoCodec::Av1, 45)));
        assert_eq!(pick(&[VideoCodec::H265], SCM_ALL), Some((VideoCodec::H265, 49)));
        // H.264: the 42e01f entry wins over the first one.
        assert_eq!(pick(&[VideoCodec::H264], SCM_ALL), Some((VideoCodec::H264, 103)));
        // Host without AV1/HEVC falls through to H.264.
        assert_eq!(pick(&[VideoCodec::Av1, VideoCodec::H265, VideoCodec::H264], 0x1), Some((VideoCodec::H264, 103)));
        assert_eq!(pick(&[VideoCodec::Av1], 0x1), None);
    }

    #[test]
    fn host_support_bits() {
        assert!(VideoCodec::H264.host_supports(SCM_ALL));
        assert!(VideoCodec::H265.host_supports(SCM_ALL));
        assert!(VideoCodec::Av1.host_supports(SCM_ALL));
        assert!(!VideoCodec::Av1.host_supports(0x0101));
    }
}
