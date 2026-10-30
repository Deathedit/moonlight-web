// Decodes the "video" data channel with WebCodecs and draws each frame to an
// OffscreenCanvas the moment it decodes: no jitter buffer, no frame scheduling. Runs in a
// worker so main-thread work (input, UI) never delays a frame.
//
// Messages are framed by `bridge/src/stream/video_channel.rs`: one flags byte, then a
// slice of the frame; payloads from FIRST through LAST make one access unit.

import { codecString } from './bitstream';
import type { VideoCodec } from './codecs';

const FIRST = 0x01;
const LAST = 0x02;
const KEYFRAME = 0x04;

export type ToWorker =
  | { type: 'init'; canvas: OffscreenCanvas; codec: VideoCodec }
  | { type: 'chunk'; data: ArrayBuffer }
  | { type: 'close' };

export interface RendererCounters {
  /** Frames drawn. */
  frames: number;
  bytes: number;
  /** Sum of decode() → output times, ms. */
  decodeMs: number;
  /** Frames discarded while waiting for a keyframe. */
  dropped: number;
  width: number;
  height: number;
  codec: string;
}

export type FromWorker =
  | { type: 'size'; width: number; height: number }
  | { type: 'idr' }
  | { type: 'fatal'; message: string }
  | { type: 'counters'; counters: RendererCounters };

const post = (msg: FromWorker) => (self as unknown as Worker).postMessage(msg);

let canvas: OffscreenCanvas | null = null;
let ctx: OffscreenCanvasRenderingContext2D | null = null;
let codec: VideoCodec = 'h264';
let decoder: VideoDecoder | null = null;
let configured: string | null = null;
/** Set until a keyframe (re)starts decoding; deltas before it can't be decoded. */
let needKeyframe = true;
/** Whether the current decoder has produced anything (a config that never works is fatal). */
let decodedAny = false;
let failures = 0;

let parts: Uint8Array[] = [];
let partsBytes = 0;
let partsKey = false;

let nextTimestamp = 0;
const submitted = new Map<number, number>();
const counters: RendererCounters = { frames: 0, bytes: 0, decodeMs: 0, dropped: 0, width: 0, height: 0, codec: '' };

self.onmessage = (e: MessageEvent<ToWorker>) => {
  const msg = e.data;
  if (msg.type === 'init') {
    canvas = msg.canvas;
    ctx = canvas.getContext('2d', { alpha: false });
    codec = msg.codec;
  } else if (msg.type === 'chunk') {
    onChunk(new Uint8Array(msg.data));
  } else if (msg.type === 'close') {
    clearInterval(countersTimer);
    if (decoder && decoder.state !== 'closed') decoder.close();
    self.close();
  }
};

const countersTimer = setInterval(() => post({ type: 'counters', counters: { ...counters } }), 500);

function onChunk(msg: Uint8Array) {
  const flags = msg[0];
  if (flags & FIRST) {
    parts = [];
    partsBytes = 0;
    partsKey = (flags & KEYFRAME) !== 0;
  }
  const payload = msg.subarray(1);
  parts.push(payload);
  partsBytes += payload.length;
  if (!(flags & LAST)) return;

  let frame: Uint8Array;
  if (parts.length === 1) frame = parts[0];
  else {
    frame = new Uint8Array(partsBytes);
    let at = 0;
    for (const p of parts) {
      frame.set(p, at);
      at += p.length;
    }
  }
  parts = [];
  counters.bytes += partsBytes;
  decode(frame, partsKey);
}

function decode(frame: Uint8Array, key: boolean) {
  if (needKeyframe && !key) {
    counters.dropped++;
    return;
  }
  if (key) {
    const wanted = codecString(codec, frame);
    if (!decoder || decoder.state === 'closed' || wanted !== configured) configure(wanted);
  }
  if (!decoder || decoder.state !== 'configured') return;

  const timestamp = nextTimestamp++;
  submitted.set(timestamp, performance.now());
  try {
    decoder.decode(new EncodedVideoChunk({ type: key ? 'key' : 'delta', timestamp, data: frame }));
    needKeyframe = false;
  } catch (e) {
    recover(`decode: ${(e as Error).message}`);
  }
}

function configure(codecString: string) {
  if (decoder && decoder.state !== 'closed') decoder.close();
  submitted.clear();
  decodedAny = false;
  configured = codecString;
  counters.codec = codecString;
  // A replaced decoder may still report errors; only the current one's matter.
  const d: VideoDecoder = new VideoDecoder({ output: draw, error: (e) => d === decoder && recover(e.message) });
  decoder = d;
  try {
    // Annex B (no description): parameter sets travel in-band with each keyframe.
    d.configure({ codec: codecString, optimizeForLatency: true });
  } catch (e) {
    recover(`configure ${codecString}: ${(e as Error).message}`);
  }
}

function draw(frame: VideoFrame) {
  const at = submitted.get(frame.timestamp);
  if (at != null) {
    submitted.delete(frame.timestamp);
    counters.decodeMs += performance.now() - at;
  }
  decodedAny = true;
  failures = 0;
  if (canvas && ctx) {
    const w = frame.displayWidth;
    const h = frame.displayHeight;
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
      counters.width = w;
      counters.height = h;
      post({ type: 'size', width: w, height: h });
    }
    ctx.drawImage(frame, 0, 0, w, h);
  }
  frame.close();
  counters.frames++;
}

/** The decoder errored (and closed itself): start over from the next keyframe. */
function recover(reason: string) {
  console.warn('video decoder:', reason);
  if (decoder && decoder.state !== 'closed') decoder.close();
  decoder = null;
  configured = null;
  needKeyframe = true;
  // A configuration that fails before producing a single frame won't start working.
  if (!decodedAny && ++failures >= 3) {
    post({ type: 'fatal', message: `This browser can't decode the stream (${counters.codec}): ${reason}` });
    return;
  }
  post({ type: 'idr' });
}
