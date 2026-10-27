// Binary input protocol, browser → bridge. Mirrors `bridge/src/stream/input.rs`:
// one type byte, then a fixed little-endian payload.

const T = {
  MOUSE_MOVE: 0x01,
  MOUSE_POSITION: 0x02,
  MOUSE_BUTTON: 0x03,
  SCROLL: 0x04,
  HSCROLL: 0x05,
  KEY: 0x06,
  CONTROLLER: 0x07,
  CONTROLLER_ARRIVAL: 0x08,
  TEXT: 0x09,
} as const;

const clampI16 = (v: number) => Math.max(-32768, Math.min(32767, Math.round(v)));

function msg(type: number, size: number, write: (v: DataView) => void): ArrayBuffer {
  const buf = new ArrayBuffer(1 + size);
  const view = new DataView(buf);
  view.setUint8(0, type);
  write(new DataView(buf, 1));
  return buf;
}

export const encode = {
  mouseMove: (dx: number, dy: number) =>
    msg(T.MOUSE_MOVE, 4, (v) => {
      v.setInt16(0, clampI16(dx), true);
      v.setInt16(2, clampI16(dy), true);
    }),

  mousePosition: (x: number, y: number, refWidth: number, refHeight: number) =>
    msg(T.MOUSE_POSITION, 8, (v) => {
      v.setInt16(0, clampI16(x), true);
      v.setInt16(2, clampI16(y), true);
      v.setInt16(4, clampI16(refWidth), true);
      v.setInt16(6, clampI16(refHeight), true);
    }),

  /** button: 1 left, 2 middle, 3 right, 4 back (X1), 5 forward (X2). */
  mouseButton: (pressed: boolean, button: number) =>
    msg(T.MOUSE_BUTTON, 2, (v) => {
      v.setUint8(0, pressed ? 1 : 0);
      v.setUint8(1, button);
    }),

  /** amount in Windows wheel units (120 = one notch, positive = up/away). */
  scroll: (amount: number) => msg(T.SCROLL, 2, (v) => v.setInt16(0, clampI16(amount), true)),
  hscroll: (amount: number) => msg(T.HSCROLL, 2, (v) => v.setInt16(0, clampI16(amount), true)),

  key: (pressed: boolean, vk: number, modifiers: number) =>
    msg(T.KEY, 4, (v) => {
      v.setUint8(0, pressed ? 1 : 0);
      v.setUint16(1, vk, true);
      v.setUint8(3, modifiers);
    }),

  controller: (s: ControllerState) =>
    msg(T.CONTROLLER, 17, (v) => {
      v.setUint8(0, s.index);
      v.setUint16(1, s.activeMask, true);
      v.setUint32(3, s.buttons >>> 0, true);
      v.setUint8(7, s.leftTrigger);
      v.setUint8(8, s.rightTrigger);
      v.setInt16(9, s.leftStick[0], true);
      v.setInt16(11, s.leftStick[1], true);
      v.setInt16(13, s.rightStick[0], true);
      v.setInt16(15, s.rightStick[1], true);
    }),

  controllerArrival: (index: number, activeMask: number, type: number, supportedButtons: number, capabilities: number) =>
    msg(T.CONTROLLER_ARRIVAL, 10, (v) => {
      v.setUint8(0, index);
      v.setUint16(1, activeMask, true);
      v.setUint8(3, type);
      v.setUint32(4, supportedButtons >>> 0, true);
      v.setUint16(8, capabilities, true);
    }),

  text: (text: string) => {
    const bytes = new TextEncoder().encode(text);
    const out = new Uint8Array(1 + bytes.length);
    out[0] = T.TEXT;
    out.set(bytes, 1);
    return out.buffer;
  },
};

export interface ControllerState {
  index: number;
  activeMask: number;
  buttons: number;
  leftTrigger: number;
  rightTrigger: number;
  leftStick: [number, number];
  rightStick: [number, number];
}

// Limelight.h constants
export const MODIFIER = { SHIFT: 0x01, CTRL: 0x02, ALT: 0x04, META: 0x08 } as const;
export const BUTTON_FLAG = {
  UP: 0x0001,
  DOWN: 0x0002,
  LEFT: 0x0004,
  RIGHT: 0x0008,
  PLAY: 0x0010,
  BACK: 0x0020,
  LS_CLK: 0x0040,
  RS_CLK: 0x0080,
  LB: 0x0100,
  RB: 0x0200,
  SPECIAL: 0x0400,
  A: 0x1000,
  B: 0x2000,
  X: 0x4000,
  Y: 0x8000,
} as const;
export const CONTROLLER_TYPE = { XBOX: 0x01, PS: 0x02, NINTENDO: 0x03 } as const;
export const CONTROLLER_CAP = { ANALOG_TRIGGERS: 0x01, RUMBLE: 0x02 } as const;
