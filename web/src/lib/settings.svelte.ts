// Stream settings, persisted per-browser in localStorage.

import type { VideoCodec } from './stream/codecs';

/** "auto" picks the best codec this browser decodes in hardware and the host encodes. */
export type Codec = 'auto' | VideoCodec;
export type MouseMode = 'relative' | 'absolute';

export interface StreamSettings {
  /** Use this display's resolution instead of width/height. */
  nativeResolution: boolean;
  width: number;
  height: number;
  /** Use this display's measured refresh rate instead of fps. */
  nativeFps: boolean;
  fps: number;
  bitrateKbps: number;
  codec: Codec;
  mouseMode: MouseMode;
  showStats: boolean;
}

const KEY = 'mlw.settings';

export const defaults: StreamSettings = {
  nativeResolution: false,
  width: 1920,
  height: 1080,
  nativeFps: false,
  fps: 60,
  bitrateKbps: 20_000,
  codec: 'auto',
  mouseMode: 'relative',
  showStats: false,
};

function load(): StreamSettings {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) return { ...defaults, ...JSON.parse(raw) };
  } catch {
    // storage unavailable or corrupt — fall back to defaults
  }
  return { ...defaults };
}

export const settings: StreamSettings = $state(load());

$effect.root(() => {
  $effect(() => {
    const snapshot = JSON.stringify(settings);
    try {
      localStorage.setItem(KEY, snapshot);
    } catch {
      // ignore (private mode etc.)
    }
  });
});

export function resetSettings() {
  Object.assign(settings, defaults);
}

// ---- Native display detection ----

/** Physical pixel size of the screen this window is on (even numbers, as encoders want). */
export function nativeResolution(): { width: number; height: number } {
  const dpr = window.devicePixelRatio || 1;
  const even = (n: number) => Math.round(n / 2) * 2;
  return { width: even(screen.width * dpr), height: even(screen.height * dpr) };
}

let refreshCache: number | null = null;

/**
 * Measure the display refresh rate from requestAnimationFrame intervals (browsers don't
 * expose it directly). Uses the median interval so a dropped frame doesn't skew it.
 */
export function measureRefreshRate(samples = 90): Promise<number> {
  if (refreshCache) return Promise.resolve(refreshCache);
  return new Promise((resolve) => {
    const deltas: number[] = [];
    let last: number | null = null;
    const tick = (t: number) => {
      if (last !== null) deltas.push(t - last);
      last = t;
      if (deltas.length < samples) {
        requestAnimationFrame(tick);
        return;
      }
      deltas.sort((a, b) => a - b);
      const median = deltas[Math.floor(deltas.length / 2)];
      refreshCache = snapRefreshRate(1000 / median);
      resolve(refreshCache);
    };
    requestAnimationFrame(tick);
  });
}

/** 59.94 → 60, 174.6 → 175, 143.9 → 144; otherwise nearest integer. */
function snapRefreshRate(hz: number): number {
  const common = [24, 30, 48, 50, 60, 72, 75, 90, 100, 120, 144, 160, 165, 170, 175, 180, 200, 240, 280, 360];
  const near = common.find((c) => Math.abs(c - hz) / c < 0.015);
  return near ?? Math.round(hz);
}

/** Width/height/fps actually requested for a stream, with "native" resolved. */
export async function resolveStreamMode(s: StreamSettings): Promise<{ width: number; height: number; fps: number }> {
  const { width, height } = s.nativeResolution ? nativeResolution() : s;
  const fps = s.nativeFps ? await measureRefreshRate() : s.fps;
  return { width, height, fps };
}

/**
 * Bitrate suggestion: Moonlight's 20 Mbps for 1080p60, scaled by pixel rate with
 * diminishing returns (higher resolutions/frame rates compress better per pixel).
 */
export function recommendedBitrateKbps(width: number, height: number, fps: number): number {
  const ratio = (width * height * fps) / (1920 * 1080 * 60);
  const kbps = 20_000 * Math.pow(ratio, 0.8);
  return Math.min(150_000, Math.max(2_000, Math.round(kbps / 500) * 500));
}
