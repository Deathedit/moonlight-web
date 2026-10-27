// Browser half of a stream: WebRTC peer connection to the bridge.
//
// The browser is the offerer (recvonly video + audio, plus "control", "input" and
// "input-reliable" data channels); the bridge answers non-trickle, so one HTTP round trip
// sets everything up. Session progress and termination arrive as JSON on "control".

import { ApiError } from '../api';
import { resolveStreamMode, type StreamSettings } from '../settings.svelte';
import type { InputChannels } from './input';

export type ControlMessage =
  | { type: 'stage'; stage: string }
  | { type: 'started'; width: number; height: number; fps: number }
  | { type: 'status'; poor: boolean }
  | { type: 'failed'; error: string }
  | { type: 'ended'; reason: string | null }
  | { type: 'rumble'; controller: number; low: number; high: number };

export interface SessionHandlers {
  onMessage: (msg: ControlMessage) => void;
  /** Transport-level loss of the bridge connection. */
  onDisconnected: (reason: string) => void;
  /** A track arrived; the stream (same object each time) now contains it. */
  onMedia: (stream: MediaStream) => void;
}

export interface StreamStats {
  width: number;
  height: number;
  fps: number;
  bitrateMbps: number;
  rttMs: number | null;
  /** Average time frames spend in the jitter buffer. */
  jitterBufferMs: number | null;
  decodeMs: number | null;
  packetsLost: number;
  framesDropped: number;
  codec: string;
}

const ICE_GATHER_TIMEOUT_MS = 2000;

export class StreamSession {
  private constructor(
    readonly pc: RTCPeerConnection,
    readonly control: RTCDataChannel,
    readonly input: InputChannels,
    private handlers: SessionHandlers,
  ) {}

  static async start(
    hostId: string,
    appId: number,
    settings: StreamSettings,
    handlers: SessionHandlers,
  ): Promise<StreamSession> {
    const pc = new RTCPeerConnection({ bundlePolicy: 'max-bundle' });
    preferH264(pc.addTransceiver('video', { direction: 'recvonly' }));
    pc.addTransceiver('audio', { direction: 'recvonly' });

    const control = pc.createDataChannel('control', { ordered: true });
    const input: InputChannels = {
      fast: pc.createDataChannel('input', { ordered: false, maxRetransmits: 0 }),
      reliable: pc.createDataChannel('input-reliable', { ordered: true }),
    };
    input.fast.binaryType = input.reliable.binaryType = 'arraybuffer';
    const session = new StreamSession(pc, control, input, handlers);

    control.onmessage = (e) => {
      try {
        handlers.onMessage(JSON.parse(e.data));
      } catch {
        console.warn('bad control message', e.data);
      }
    };

    // The bridge sends audio and video in separate streams so the browser doesn't delay
    // video for lip sync; combine them locally for one <video> element.
    const media = new MediaStream();
    pc.ontrack = (e) => {
      // Latency over smoothness: play frames as soon as they decode.
      const receiver = e.receiver as RTCRtpReceiver & { jitterBufferTarget?: number; playoutDelayHint?: number };
      if ('jitterBufferTarget' in receiver) receiver.jitterBufferTarget = 0;
      if ('playoutDelayHint' in receiver) receiver.playoutDelayHint = 0;
      media.addTrack(e.track);
      handlers.onMedia(media);
    };

    pc.onconnectionstatechange = () => {
      if (pc.connectionState === 'failed') handlers.onDisconnected('Lost connection to the bridge.');
    };

    try {
      await pc.setLocalDescription(await pc.createOffer());
      await waitForIceGathering(pc);

      const mode = await resolveStreamMode(settings);
      const res = await fetch(`/api/hosts/${encodeURIComponent(hostId)}/stream`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          appId,
          ...mode,
          bitrateKbps: settings.bitrateKbps,
          sdp: pc.localDescription!.sdp,
        }),
      });
      if (!res.ok) {
        const body = await res.json().catch(() => null);
        throw new ApiError(res.status, body?.error ?? res.statusText);
      }
      const { sdp } = await res.json();
      await pc.setRemoteDescription({ type: 'answer', sdp });
    } catch (e) {
      pc.close();
      throw e;
    }
    return session;
  }

  /** End the stream. The bridge stops the connection when the control channel says so or the PC closes. */
  stop() {
    this.handlers = { onMessage: () => {}, onDisconnected: () => {}, onMedia: () => {} };
    if (this.control.readyState === 'open') this.control.send(JSON.stringify({ type: 'stop' }));
    // Give the stop message a moment to flush before tearing down.
    setTimeout(() => this.pc.close(), 50);
  }

  private prev: { at: number; bytes: number; jbDelay: number; jbCount: number } | null = null;

  async stats(): Promise<StreamStats | null> {
    const report = await this.pc.getStats();
    let inbound: any = null;
    let pair: any = null;
    const codecs = new Map<string, any>();
    report.forEach((s: any) => {
      if (s.type === 'inbound-rtp' && s.kind === 'video') inbound = s;
      if (s.type === 'candidate-pair' && s.nominated && s.state === 'succeeded') pair = s;
      if (s.type === 'codec') codecs.set(s.id, s);
    });
    if (!inbound) return null;

    const now = inbound.timestamp as number;
    const bytes = inbound.bytesReceived as number;
    const jbDelay = (inbound.jitterBufferDelay as number) ?? 0;
    const jbCount = (inbound.jitterBufferEmittedCount as number) ?? 0;
    let bitrateMbps = 0;
    // The jitter buffer counters are cumulative since the start; report the last interval
    // so the number reflects what's happening now.
    let jitterBufferMs: number | null = null;
    if (this.prev && now > this.prev.at) {
      bitrateMbps = ((bytes - this.prev.bytes) * 8) / ((now - this.prev.at) * 1000);
      const frames = jbCount - this.prev.jbCount;
      if (frames > 0) jitterBufferMs = ((jbDelay - this.prev.jbDelay) / frames) * 1000;
    }
    this.prev = { at: now, bytes, jbDelay, jbCount };

    const per = (total?: number, count?: number) => (total != null && count ? (total / count) * 1000 : null);
    return {
      width: inbound.frameWidth ?? 0,
      height: inbound.frameHeight ?? 0,
      fps: inbound.framesPerSecond ?? 0,
      bitrateMbps,
      rttMs: pair?.currentRoundTripTime != null ? pair.currentRoundTripTime * 1000 : null,
      jitterBufferMs,
      decodeMs: per(inbound.totalDecodeTime, inbound.framesDecoded),
      packetsLost: inbound.packetsLost ?? 0,
      framesDropped: inbound.framesDropped ?? 0,
      codec: codecs.get(inbound.codecId)?.mimeType?.replace('video/', '') ?? '?',
    };
  }
}

/** Put H.264 (packetization-mode=1) first so it's what gets negotiated. */
function preferH264(transceiver: RTCRtpTransceiver) {
  const caps = RTCRtpReceiver.getCapabilities?.('video')?.codecs;
  if (!caps || !transceiver.setCodecPreferences) return;
  const h264 = caps.filter((c) => c.mimeType.toLowerCase() === 'video/h264');
  if (h264.length === 0) return;
  const mode1 = (c: RTCRtpCodec) => (c.sdpFmtpLine ?? '').includes('packetization-mode=1');
  h264.sort((a, b) => Number(mode1(b)) - Number(mode1(a)));
  // Keep RTX/RED/FEC entries so retransmissions still work.
  const extras = caps.filter((c) => /rtx|red|ulpfec|flexfec/i.test(c.mimeType));
  transceiver.setCodecPreferences([...h264, ...extras]);
}

function waitForIceGathering(pc: RTCPeerConnection): Promise<void> {
  if (pc.iceGatheringState === 'complete') return Promise.resolve();
  return new Promise((resolve) => {
    const done = () => {
      pc.removeEventListener('icegatheringstatechange', check);
      clearTimeout(timer);
      resolve();
    };
    const check = () => pc.iceGatheringState === 'complete' && done();
    // LAN host candidates arrive almost instantly; don't hang on unreachable STUN.
    const timer = setTimeout(done, ICE_GATHER_TIMEOUT_MS);
    pc.addEventListener('icegatheringstatechange', check);
  });
}
