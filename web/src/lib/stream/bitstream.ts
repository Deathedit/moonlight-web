// WebCodecs codec strings from the stream itself: the decoder must be configured with
// the profile/level the host actually encoded, which is in the keyframe's parameter sets.

import type { VideoCodec } from './codecs';

/** Fallbacks (high levels, so they cover any resolution/refresh) if parsing fails. */
export const GENERIC_CODEC_STRING: Record<VideoCodec, string> = {
  h264: 'avc1.640034', // High, level 5.2
  h265: 'hev1.1.6.L186.B0', // Main, level 6.2
  av1: 'av01.0.16M.08', // Main, level 6.0, 8-bit
};

/** Annex-B NAL units (start codes stripped). */
export function* nalUnits(data: Uint8Array): Generator<Uint8Array> {
  let start = -1;
  for (let i = 0; i + 2 < data.length; i++) {
    if (data[i] === 0 && data[i + 1] === 0 && data[i + 2] === 1) {
      if (start >= 0) yield trimZeros(data.subarray(start, i));
      start = i + 3;
      i += 2;
    }
  }
  if (start >= 0 && start < data.length) yield data.subarray(start);
}

// A 4-byte start code leaves a trailing 0 on the previous unit.
function trimZeros(nal: Uint8Array): Uint8Array {
  let end = nal.length;
  while (end > 0 && nal[end - 1] === 0) end--;
  return nal.subarray(0, end);
}

/** Remove emulation-prevention bytes (00 00 03 → 00 00). */
function unescape(nal: Uint8Array): Uint8Array {
  const out: number[] = [];
  let zeros = 0;
  for (const b of nal) {
    if (zeros >= 2 && b === 3) {
      zeros = 0;
      continue;
    }
    zeros = b === 0 ? zeros + 1 : 0;
    out.push(b);
  }
  return Uint8Array.from(out);
}

const hex2 = (n: number) => n.toString(16).padStart(2, '0');

function h264(data: Uint8Array): string | null {
  for (const nal of nalUnits(data)) {
    // SPS: profile_idc, constraint flags, level_idc right after the header byte.
    if ((nal[0] & 0x1f) === 7 && nal.length >= 4) return `avc1.${hex2(nal[1])}${hex2(nal[2])}${hex2(nal[3])}`;
  }
  return null;
}

function h265(data: Uint8Array): string | null {
  for (const nal of nalUnits(data)) {
    if (((nal[0] >> 1) & 0x3f) !== 33) continue; // SPS
    const sps = unescape(nal.subarray(2));
    if (sps.length < 13) return null;
    // After one byte of ids comes profile_tier_level: space/tier/profile, 32 compatibility
    // flags, 48 constraint bits, level (ISO/IEC 14496-15 E.3 codec string).
    const space = sps[1] >> 6;
    const tier = (sps[1] >> 5) & 1;
    const profile = sps[1] & 0x1f;
    let compat = 0;
    for (let i = 0; i < 4; i++) compat = (compat << 8) | sps[2 + i];
    let reversed = 0;
    for (let i = 0; i < 32; i++) reversed = (reversed << 1) | ((compat >>> i) & 1);
    const constraints = Array.from(sps.subarray(6, 12));
    while (constraints.length && constraints[constraints.length - 1] === 0) constraints.pop();
    const level = sps[12];
    const spaceTag = space ? String.fromCharCode(64 + space) : '';
    const tail = constraints.length ? '.' + constraints.map((b) => b.toString(16).toUpperCase()).join('.') : '';
    return `hev1.${spaceTag}${profile}.${(reversed >>> 0).toString(16).toUpperCase()}.${tier ? 'H' : 'L'}${level}${tail}`;
  }
  return null;
}

function av1(data: Uint8Array): string | null {
  // Walk OBUs to the sequence header; read profile and the first operating point's level.
  let i = 0;
  while (i < data.length) {
    const header = data[i];
    const type = (header >> 3) & 0xf;
    const hasExt = (header & 0x04) !== 0;
    const hasSize = (header & 0x02) !== 0;
    i += 1 + (hasExt ? 1 : 0);
    let size = data.length - i;
    if (hasSize) {
      size = 0;
      for (let shift = 0; i < data.length; shift += 7) {
        const b = data[i++];
        size += (b & 0x7f) * 2 ** shift;
        if (!(b & 0x80)) break;
      }
    }
    if (type === 1) {
      const p = data.subarray(i, i + size);
      if (p.length < 2) return null;
      const profile = p[0] >> 5;
      const reduced = (p[0] >> 3) & 1;
      const timingInfo = (p[0] >> 2) & 1;
      // Without timing info / reduced header the level sits at a fixed position:
      // timing(1)=0 initial_display_delay_present(1) operating_points_cnt_minus_1(5)
      // operating_point_idc[0](12) seq_level_idx[0](5)
      let level: number | null = null;
      if (reduced) level = ((p[0] & 0x07) << 2) | (p[1] >> 6);
      else if (!timingInfo && p.length >= 4) {
        const bits = (p[0] << 24) | (p[1] << 16) | (p[2] << 8) | p[3];
        if (((bits >>> 25) & 1) === 0) level = (bits >>> 3) & 0x1f; // no initial_display_delay
      }
      if (level == null) return null;
      return `av01.${profile}.${String(level).padStart(2, '0')}M.08`;
    }
    i += size;
  }
  return null;
}

/** The codec string for a keyframe, or a generic one if its headers can't be read. */
export function codecString(codec: VideoCodec, keyframe: Uint8Array): string {
  const parsed = codec === 'h264' ? h264(keyframe) : codec === 'h265' ? h265(keyframe) : av1(keyframe);
  return parsed ?? GENERIC_CODEC_STRING[codec];
}
