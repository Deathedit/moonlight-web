// Main-thread side of the WebCodecs renderer: owns the canvas and the decode worker, and
// relays the "video" data channel to it (Chrome can't hand a data channel to a worker).

import type { VideoCodec } from './codecs';
import type { FromWorker, RendererCounters, ToWorker } from './video-worker';

export interface RendererEvents {
  /** The decoder lost its place and needs a keyframe. */
  onKeyframeNeeded: () => void;
  /** The stream can't be decoded at all. */
  onFatal: (message: string) => void;
}

export class WebCodecsRenderer {
  readonly canvas = document.createElement('canvas');
  /** Size of the decoded video (0 until the first frame). */
  width = 0;
  height = 0;
  counters: RendererCounters | null = null;
  private worker: Worker;

  constructor(channel: RTCDataChannel, codec: VideoCodec, events: RendererEvents) {
    this.worker = new Worker(new URL('./video-worker.ts', import.meta.url), { type: 'module' });
    const offscreen = this.canvas.transferControlToOffscreen();
    this.send({ type: 'init', canvas: offscreen, codec }, [offscreen]);

    this.worker.onmessage = (e: MessageEvent<FromWorker>) => {
      const msg = e.data;
      if (msg.type === 'size') {
        this.width = msg.width;
        this.height = msg.height;
      } else if (msg.type === 'counters') this.counters = msg.counters;
      else if (msg.type === 'idr') events.onKeyframeNeeded();
      else if (msg.type === 'fatal') events.onFatal(msg.message);
    };

    channel.binaryType = 'arraybuffer';
    channel.onmessage = (e: MessageEvent<ArrayBuffer>) => this.send({ type: 'chunk', data: e.data }, [e.data]);
  }

  private send(msg: ToWorker, transfer: Transferable[] = []) {
    this.worker.postMessage(msg, transfer);
  }

  destroy() {
    this.send({ type: 'close' });
    this.canvas.remove();
  }
}
