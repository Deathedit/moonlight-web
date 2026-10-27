// Which video codecs this browser can receive over WebRTC, and whether it decodes them in
// hardware. Used to rank codecs for "Auto" and to annotate the Settings page.

export type VideoCodec = 'h264' | 'h265' | 'av1';

export const CODEC_LABELS: Record<VideoCodec, string> = { h264: 'H.264', h265: 'HEVC (H.265)', av1: 'AV1' };

const MIME: Record<VideoCodec, string> = { h264: 'video/H264', h265: 'video/H265', av1: 'video/AV1' };

export interface CodecSupport {
  codec: VideoCodec;
  /** Can be negotiated over WebRTC at all. */
  supported: boolean;
  /** Decoded in hardware (MediaCapabilities' powerEfficient), if known. */
  hardware: boolean | null;
}

export interface Mode {
  width: number;
  height: number;
  fps: number;
  bitrateKbps: number;
}

function receivable(): Set<string> {
  const caps = RTCRtpReceiver.getCapabilities?.('video')?.codecs ?? [];
  return new Set(caps.map((c) => c.mimeType.toLowerCase()));
}

export async function probeCodecs(mode: Mode): Promise<CodecSupport[]> {
  const mimes = receivable();
  return Promise.all(
    (Object.keys(MIME) as VideoCodec[]).map(async (codec) => {
      const supported = mimes.has(MIME[codec].toLowerCase());
      let hardware: boolean | null = null;
      if (supported && navigator.mediaCapabilities?.decodingInfo) {
        try {
          const info = await navigator.mediaCapabilities.decodingInfo({
            type: 'webrtc',
            video: {
              contentType: MIME[codec],
              width: mode.width,
              height: mode.height,
              framerate: mode.fps,
              bitrate: mode.bitrateKbps * 1000,
            },
          });
          hardware = info.supported ? info.powerEfficient : false;
        } catch {
          // Some browsers don't accept type "webrtc" yet.
        }
      }
      return { codec, supported, hardware };
    }),
  );
}

/**
 * "Auto": hardware decoders first, best compression first (AV1 > HEVC > H.264); then
 * software decoders, cheapest first — software AV1 at high resolution/refresh would cost
 * far more CPU (and latency) than software H.264.
 */
export function rankForAuto(support: CodecSupport[]): VideoCodec[] {
  const ok = support.filter((s) => s.supported);
  const hw = (['av1', 'h265', 'h264'] as VideoCodec[]).filter((c) => ok.find((s) => s.codec === c)?.hardware === true);
  const sw = (['h264', 'h265', 'av1'] as VideoCodec[]).filter((c) => ok.some((s) => s.codec === c) && !hw.includes(c));
  return [...hw, ...sw];
}

/** Put the preferred codecs first in the offer (keeping the rest, and RTX/FEC, after them). */
export function applyCodecPreferences(transceiver: RTCRtpTransceiver, order: VideoCodec[]) {
  const caps = RTCRtpReceiver.getCapabilities?.('video')?.codecs;
  if (!caps || !transceiver.setCodecPreferences) return;
  const rank = (c: RTCRtpCodec) => {
    const i = order.findIndex((v) => MIME[v].toLowerCase() === c.mimeType.toLowerCase());
    return i === -1 ? order.length : i;
  };
  const mode1 = (c: RTCRtpCodec) => ((c.sdpFmtpLine ?? '').includes('packetization-mode=1') ? 0 : 1);
  const sorted = [...caps].sort((a, b) => rank(a) - rank(b) || mode1(a) - mode1(b));
  transceiver.setCodecPreferences(sorted);
}
