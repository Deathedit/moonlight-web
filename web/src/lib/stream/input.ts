// Mouse + keyboard capture for the stream view.

import type { MouseMode } from '../settings.svelte';
import { toVirtualKey } from './keymap';
import { encode, MODIFIER } from './protocol';

export interface InputChannels {
  /** Unordered, no retransmits: mouse motion (stale positions are worthless). */
  fast: RTCDataChannel;
  /** Ordered + reliable: buttons, keys, scroll, controllers. */
  reliable: RTCDataChannel;
}

/** Moonlight-style local shortcuts: Ctrl+Alt+Shift + key. Never forwarded to the host. */
export type Shortcut = 'quit' | 'release-mouse' | 'stats' | 'fullscreen' | 'toggle-input';
const SHORTCUTS: Record<string, Shortcut> = {
  KeyQ: 'quit',
  KeyZ: 'release-mouse',
  KeyS: 'stats',
  KeyX: 'fullscreen',
  KeyI: 'toggle-input',
};

// Browser MouseEvent.button → GameStream button (1 left, 2 middle, 3 right, 4 X1, 5 X2)
const BUTTONS = [1, 2, 3, 4, 5];

export class PointerLockError extends Error {}

function lockError(e: unknown): never {
  const msg = e instanceof DOMException ? e.message : String(e);
  throw new PointerLockError(msg || 'The browser refused to capture the mouse.');
}

export class InputCapture {
  private pressedKeys = new Map<string, number>(); // code → vk
  private pressedButtons = new Set<number>();
  private dx = 0;
  private dy = 0;
  private flushScheduled = false;
  private cleanup: (() => void)[] = [];

  /** Called when the browser refuses to capture the mouse. */
  onLockError: (message: string) => void = () => {};

  private _enabled = true;

  /** When false nothing is forwarded (local shortcuts still work). */
  get enabled() {
    return this._enabled;
  }

  set enabled(on: boolean) {
    if (on === this._enabled) return;
    this._enabled = on;
    if (!on) {
      // Don't leave anything held down on the host, and give the mouse back.
      this.releaseAll();
      this.dx = this.dy = 0;
      if (this.pointerLocked) document.exitPointerLock();
    }
  }

  constructor(
    private surface: HTMLElement,
    private video: HTMLVideoElement,
    private channels: InputChannels,
    private mode: MouseMode,
    private onShortcut: (s: Shortcut) => void,
  ) {
    this.listen(surface, 'pointerdown', this.onPointerDown);
    this.listen(surface, 'pointerup', this.onPointerUp);
    this.listen(surface, 'pointermove', this.onPointerMove);
    this.listen(surface, 'wheel', this.onWheel, { passive: false });
    this.listen(surface, 'contextmenu', (e: Event) => e.preventDefault());
    this.listen(window, 'keydown', this.onKeyDown);
    this.listen(window, 'keyup', this.onKeyUp);
    this.listen(window, 'blur', this.releaseAll);
    this.listen(document, 'visibilitychange', () => document.hidden && this.releaseAll());
    this.listen(document, 'pointerlockchange', () => !this.pointerLocked && this.releaseButtons());
    // Firefox reports failures via this event rather than a rejected promise.
    this.listen(document, 'pointerlockerror', () => this.onLockError('The browser refused to capture the mouse.'));
  }

  get pointerLocked() {
    return document.pointerLockElement === this.surface;
  }

  /** In relative mode the pointer must be locked before motion is captured. */
  get needsPointerLock() {
    return this._enabled && this.mode === 'relative' && !this.pointerLocked;
  }

  /**
   * Capture the mouse. Must be called from a click/keypress handler. Prefers raw
   * (unaccelerated) motion; if the browser rejects that (e.g. Chrome on Linux), we
   * remember it and use a plain lock from then on. Retrying asynchronously inside the
   * rejection handler would be outside the user gesture and can be refused, so the
   * plain request is made synchronously here whenever raw isn't known to work.
   */
  lockPointer(): Promise<void> {
    const el = this.surface as HTMLElement & {
      requestPointerLock(opts?: { unadjustedMovement?: boolean }): Promise<void> | void;
    };
    if (InputCapture.rawMotion === false) return Promise.resolve(el.requestPointerLock()).catch(lockError);

    const p = el.requestPointerLock({ unadjustedMovement: true });
    if (!(p instanceof Promise)) return Promise.resolve(); // older Firefox: no options, no promise
    return p.then(
      () => {
        InputCapture.rawMotion = true;
      },
      (e: DOMException) => {
        if (e?.name === 'NotSupportedError') {
          InputCapture.rawMotion = false;
          // Still inside the gesture's activation window in practice; if refused, the
          // next click goes straight to the plain request above.
          return Promise.resolve(el.requestPointerLock()).catch(lockError);
        }
        lockError(e);
      },
    );
  }

  /** Whether `unadjustedMovement` works in this browser (unknown until first tried). */
  private static rawMotion: boolean | undefined;

  destroy() {
    this.releaseAll();
    if (this.pointerLocked) document.exitPointerLock();
    this.cleanup.forEach((f) => f());
    this.cleanup = [];
  }

  // ---- plumbing ----

  private listen<E extends Event>(
    target: EventTarget,
    type: string,
    handler: (e: E) => void,
    opts?: AddEventListenerOptions,
  ) {
    const bound = handler.bind(this) as EventListener;
    target.addEventListener(type, bound, opts);
    this.cleanup.push(() => target.removeEventListener(type, bound, opts));
  }

  private send(ch: RTCDataChannel, data: ArrayBuffer) {
    if (ch.readyState === 'open') ch.send(data);
  }

  // ---- mouse ----

  private onPointerDown(e: PointerEvent) {
    if (e.pointerType !== 'mouse' || !this._enabled) return;
    if (this.needsPointerLock) {
      // First click only captures the mouse; don't forward it.
      this.lockPointer().catch((err: Error) => this.onLockError(err.message));
      return;
    }
    if (this.mode === 'absolute') this.sendPosition(e);
    const button = BUTTONS[e.button];
    if (!button) return;
    e.preventDefault();
    this.pressedButtons.add(button);
    this.send(this.channels.reliable, encode.mouseButton(true, button));
  }

  private onPointerUp(e: PointerEvent) {
    if (e.pointerType !== 'mouse') return;
    const button = BUTTONS[e.button];
    if (!button || !this.pressedButtons.delete(button)) return;
    e.preventDefault();
    this.send(this.channels.reliable, encode.mouseButton(false, button));
  }

  private onPointerMove(e: PointerEvent) {
    if (e.pointerType !== 'mouse' || !this._enabled) return;
    if (this.mode === 'absolute') {
      this.sendPosition(e);
      return;
    }
    if (!this.pointerLocked) return;
    // High-polling-rate mice fire far more often than frames are drawn: coalesce.
    this.dx += e.movementX;
    this.dy += e.movementY;
    if (!this.flushScheduled) {
      this.flushScheduled = true;
      requestAnimationFrame(() => this.flushMotion());
    }
  }

  private flushMotion() {
    this.flushScheduled = false;
    while (this.dx !== 0 || this.dy !== 0) {
      const sx = Math.max(-32768, Math.min(32767, this.dx));
      const sy = Math.max(-32768, Math.min(32767, this.dy));
      this.dx -= sx;
      this.dy -= sy;
      this.send(this.channels.fast, encode.mouseMove(sx, sy));
    }
  }

  /** Map the pointer into the video's content box (object-fit: contain letterboxes it). */
  private sendPosition(e: PointerEvent) {
    const vw = this.video.videoWidth;
    const vh = this.video.videoHeight;
    if (!vw || !vh) return;
    const rect = this.video.getBoundingClientRect();
    const scale = Math.min(rect.width / vw, rect.height / vh);
    const left = rect.left + (rect.width - vw * scale) / 2;
    const top = rect.top + (rect.height - vh * scale) / 2;
    const x = Math.round((e.clientX - left) / scale);
    const y = Math.round((e.clientY - top) / scale);
    if (x < 0 || y < 0 || x >= vw || y >= vh) return;
    this.send(this.channels.fast, encode.mousePosition(x, y, vw, vh));
  }

  private onWheel(e: WheelEvent) {
    if (!this._enabled || this.needsPointerLock) return;
    e.preventDefault();
    // Windows wheel units: 120 per notch. Browsers report ~100px per notch in pixel mode.
    const unit = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? 40 : e.deltaMode === WheelEvent.DOM_DELTA_PAGE ? 360 : 1.2;
    if (e.deltaY) this.send(this.channels.reliable, encode.scroll(-e.deltaY * unit));
    if (e.deltaX) this.send(this.channels.reliable, encode.hscroll(e.deltaX * unit));
  }

  // ---- keyboard ----

  private modifiers(e: KeyboardEvent) {
    return (
      (e.shiftKey ? MODIFIER.SHIFT : 0) |
      (e.ctrlKey ? MODIFIER.CTRL : 0) |
      (e.altKey ? MODIFIER.ALT : 0) |
      (e.metaKey ? MODIFIER.META : 0)
    );
  }

  private onKeyDown(e: KeyboardEvent) {
    if (e.ctrlKey && e.altKey && e.shiftKey && SHORTCUTS[e.code]) {
      e.preventDefault();
      // The host saw Ctrl/Alt/Shift go down; lift them so they don't stick there.
      this.releaseKeys();
      this.onShortcut(SHORTCUTS[e.code]);
      return;
    }
    if (!this._enabled) return;
    const vk = toVirtualKey(e.code);
    if (vk === undefined) return;
    e.preventDefault();
    // Hosts generate their own key repeat (Sunshine's key_repeat_delay).
    if (e.repeat) return;
    this.pressedKeys.set(e.code, vk);
    this.send(this.channels.reliable, encode.key(true, vk, this.modifiers(e)));
  }

  private onKeyUp(e: KeyboardEvent) {
    const vk = this.pressedKeys.get(e.code);
    if (vk === undefined) return;
    e.preventDefault();
    this.pressedKeys.delete(e.code);
    this.send(this.channels.reliable, encode.key(false, vk, this.modifiers(e)));
  }

  // ---- stuck-input protection ----

  private releaseKeys() {
    for (const vk of this.pressedKeys.values()) this.send(this.channels.reliable, encode.key(false, vk, 0));
    this.pressedKeys.clear();
  }

  private releaseButtons() {
    for (const b of this.pressedButtons) this.send(this.channels.reliable, encode.mouseButton(false, b));
    this.pressedButtons.clear();
  }

  private releaseAll() {
    this.releaseKeys();
    this.releaseButtons();
  }
}
