// Gamepad API → GameStream controller events (up to 4 pads), plus host rumble.

import { BUTTON_FLAG, CONTROLLER_CAP, CONTROLLER_TYPE, encode, type ControllerState } from './protocol';

const MAX_PADS = 4;

// "standard" mapping button index → XInput-style flag
const STANDARD_BUTTONS: [number, number][] = [
  [0, BUTTON_FLAG.A],
  [1, BUTTON_FLAG.B],
  [2, BUTTON_FLAG.X],
  [3, BUTTON_FLAG.Y],
  [4, BUTTON_FLAG.LB],
  [5, BUTTON_FLAG.RB],
  [8, BUTTON_FLAG.BACK],
  [9, BUTTON_FLAG.PLAY],
  [10, BUTTON_FLAG.LS_CLK],
  [11, BUTTON_FLAG.RS_CLK],
  [12, BUTTON_FLAG.UP],
  [13, BUTTON_FLAG.DOWN],
  [14, BUTTON_FLAG.LEFT],
  [15, BUTTON_FLAG.RIGHT],
  [16, BUTTON_FLAG.SPECIAL],
];
const ALL_BUTTONS = STANDARD_BUTTONS.reduce((m, [, f]) => m | f, 0);

/** Small stick drift shouldn't wake the host or spam packets. */
const DEADZONE = 0.06;

type Haptic = { playEffect?: (type: string, params: object) => Promise<unknown>; reset?: () => Promise<unknown> };

function controllerType(id: string): number {
  const vendor = /vendor:\s*([0-9a-f]{4})/i.exec(id)?.[1] ?? /^([0-9a-f]{4})-/i.exec(id)?.[1];
  if (vendor === '054c' || /dualsense|dualshock|playstation/i.test(id)) return CONTROLLER_TYPE.PS;
  if (vendor === '057e' || /nintendo|pro controller|joy-con/i.test(id)) return CONTROLLER_TYPE.NINTENDO;
  return CONTROLLER_TYPE.XBOX;
}

function axis(v: number | undefined, invert = false): number {
  let x = v ?? 0;
  if (Math.abs(x) < DEADZONE) x = 0;
  if (invert) x = -x; // browser: up = -1; XInput: up = +32767
  return Math.max(-32768, Math.min(32767, Math.round(x * 32767)));
}

function readState(pad: Gamepad, index: number, activeMask: number): ControllerState {
  const pressed = (i: number) => pad.buttons[i]?.pressed ?? false;
  const value = (i: number) => pad.buttons[i]?.value ?? 0;
  let buttons = 0;
  for (const [i, flag] of STANDARD_BUTTONS) if (pressed(i)) buttons |= flag;
  return {
    index,
    activeMask,
    buttons,
    leftTrigger: Math.round(value(6) * 255),
    rightTrigger: Math.round(value(7) * 255),
    leftStick: [axis(pad.axes[0]), axis(pad.axes[1], true)],
    rightStick: [axis(pad.axes[2]), axis(pad.axes[3], true)],
  };
}

const same = (a: ControllerState, b: ControllerState) =>
  a.activeMask === b.activeMask &&
  a.buttons === b.buttons &&
  a.leftTrigger === b.leftTrigger &&
  a.rightTrigger === b.rightTrigger &&
  a.leftStick[0] === b.leftStick[0] &&
  a.leftStick[1] === b.leftStick[1] &&
  a.rightStick[0] === b.rightStick[0] &&
  a.rightStick[1] === b.rightStick[1];

export class GamepadForwarder {
  private last = new Map<number, ControllerState>();
  private raf = 0;
  private rumbleTimers = new Map<number, ReturnType<typeof setInterval>>();

  private _paused = false;

  /** While paused nothing is sent; pads are reset to neutral on the host when pausing. */
  set paused(on: boolean) {
    if (on === this._paused) return;
    this._paused = on;
    if (on) {
      for (const state of this.last.values()) {
        this.send(encode.controller({ ...state, buttons: 0, leftTrigger: 0, rightTrigger: 0, leftStick: [0, 0], rightStick: [0, 0] }));
      }
    }
    // Resend everything on resume.
    this.last.clear();
  }

  constructor(private channel: RTCDataChannel) {
    this.raf = requestAnimationFrame(this.poll);
  }

  destroy() {
    cancelAnimationFrame(this.raf);
    for (const t of this.rumbleTimers.values()) clearInterval(t);
    for (const pad of navigator.getGamepads()) (pad?.vibrationActuator as Haptic | undefined)?.reset?.();
  }

  private send(data: ArrayBuffer) {
    if (this.channel.readyState === 'open') this.channel.send(data);
  }

  private poll = () => {
    this.raf = requestAnimationFrame(this.poll);
    if (this._paused) return;
    const pads = navigator.getGamepads().filter((p): p is Gamepad => !!p && p.connected && p.index < MAX_PADS);
    const mask = pads.reduce((m, p) => m | (1 << p.index), 0);

    for (const pad of pads) {
      const prev = this.last.get(pad.index);
      if (!prev) {
        const caps = CONTROLLER_CAP.ANALOG_TRIGGERS | (pad.vibrationActuator ? CONTROLLER_CAP.RUMBLE : 0);
        this.send(encode.controllerArrival(pad.index, mask, controllerType(pad.id), ALL_BUTTONS, caps));
      }
      const state = readState(pad, pad.index, mask);
      if (!prev || !same(prev, state)) {
        this.send(encode.controller(state));
        this.last.set(pad.index, state);
      }
    }

    // Unplugged pads: tell the host with a neutral state and the updated mask.
    for (const index of [...this.last.keys()]) {
      if (mask & (1 << index)) continue;
      this.last.delete(index);
      this.send(
        encode.controller({
          index,
          activeMask: mask,
          buttons: 0,
          leftTrigger: 0,
          rightTrigger: 0,
          leftStick: [0, 0],
          rightStick: [0, 0],
        }),
      );
    }

  };

  /** Host rumble: magnitudes 0..65535; 0/0 stops. Effects are re-armed until changed. */
  rumble(index: number, low: number, high: number) {
    clearInterval(this.rumbleTimers.get(index));
    this.rumbleTimers.delete(index);
    const actuator = navigator.getGamepads()[index]?.vibrationActuator as Haptic | undefined;
    if (!actuator?.playEffect) return;
    if (low === 0 && high === 0) {
      actuator.reset?.();
      return;
    }
    const play = () =>
      actuator.playEffect!('dual-rumble', {
        duration: 1000,
        strongMagnitude: low / 65535,
        weakMagnitude: high / 65535,
      }).catch(() => {});
    play();
    // GameStream rumble lasts until the host changes it; browser effects time out.
    this.rumbleTimers.set(index, setInterval(play, 900));
  }
}
