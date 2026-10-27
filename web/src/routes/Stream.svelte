<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import Icon from '../lib/components/Icon.svelte';
  import { router } from '../lib/router.svelte';
  import { settings } from '../lib/settings.svelte';
  import { GamepadForwarder } from '../lib/stream/gamepad';
  import { InputCapture, type Shortcut } from '../lib/stream/input';
  import { StreamSession, type ControlMessage, type StreamStats } from '../lib/stream/session';

  let { hostId, appId }: { hostId: string; appId: string } = $props();

  type Phase =
    | { kind: 'connecting'; stage: string }
    | { kind: 'streaming' }
    | { kind: 'error'; message: string }
    | { kind: 'ended'; message: string | null };

  let phase = $state<Phase>({ kind: 'connecting', stage: 'Contacting bridge' });
  let appTitle = $state<string | null>(null);
  let poorConnection = $state(false);
  let stats = $state<StreamStats | null>(null);
  let showStats = $state(settings.showStats);
  let pointerLocked = $state(false);
  let soundBlocked = $state(false);
  let lockError = $state<string | null>(null);
  let inputPaused = $state(false);
  /** Shortcut reminder shown briefly when the stream starts (there's no toolbar). */
  let showHint = $state(false);

  let root: HTMLDivElement;
  let surface: HTMLDivElement;
  let video: HTMLVideoElement;
  let session: StreamSession | null = null;
  let input: InputCapture | null = null;
  let pads: GamepadForwarder | null = null;

  const backUrl = $derived(`/hosts/${encodeURIComponent(hostId)}/apps`);
  const relative = settings.mouseMode === 'relative';
  const needsCapture = $derived(phase.kind === 'streaming' && relative && !pointerLocked && !inputPaused);

  function onControl(msg: ControlMessage) {
    switch (msg.type) {
      case 'stage':
        if (phase.kind === 'connecting') phase = { kind: 'connecting', stage: msg.stage };
        break;
      case 'started':
        phase = { kind: 'streaming' };
        startInput();
        showHint = true;
        setTimeout(() => (showHint = false), 5000);
        break;
      case 'status':
        poorConnection = msg.poor;
        break;
      case 'rumble':
        pads?.rumble(msg.controller, msg.low, msg.high);
        break;
      case 'failed':
        phase = { kind: 'error', message: msg.error };
        stopInput();
        break;
      case 'ended':
        phase = { kind: 'ended', message: msg.reason };
        stopInput();
        break;
    }
  }

  function startInput() {
    if (!session || input) return;
    input = new InputCapture(surface, video, session.input, settings.mouseMode, onShortcut);
    pads = new GamepadForwarder(session.input.reliable);
    input.onLockError = (msg) => (lockError = msg);
    // Relative mode captures on the first click (browsers require a user gesture).
  }

  function capture() {
    lockError = null;
    input?.lockPointer().catch((e: Error) => (lockError = e.message));
  }

  function stopInput() {
    input?.destroy();
    pads?.destroy();
    input = null;
    pads = null;
    if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
  }

  function onShortcut(s: Shortcut) {
    if (s === 'quit') leave();
    else if (s === 'release-mouse') document.exitPointerLock();
    else if (s === 'stats') showStats = !showStats;
    else if (s === 'fullscreen') toggleFullscreen();
    else if (s === 'toggle-input') setInputPaused(!inputPaused);
  }

  function setInputPaused(paused: boolean) {
    inputPaused = paused;
    if (input) input.enabled = !paused;
    if (pads) pads.paused = paused;
  }

  function attachMedia(stream: MediaStream) {
    if (video.srcObject === stream) return;
    video.srcObject = stream;
    video.muted = false;
    video.play().catch(() => {
      // Autoplay policy: no recent user gesture. Play muted and offer a button.
      soundBlocked = true;
      video.muted = true;
      video.play().catch(() => {});
    });
  }

  function enableSound() {
    video.muted = false;
    video.play().then(() => (soundBlocked = false), () => {});
  }

  onMount(() => {
    const id = Number(appId);
    api.listApps(hostId).then(
      (apps) => (appTitle = apps.find((a) => a.id === id)?.title ?? null),
      () => {},
    );

    let cancelled = false;
    StreamSession.start(hostId, id, settings, {
      onMessage: onControl,
      onMedia: attachMedia,
      onDisconnected: (reason) => {
        if (phase.kind !== 'ended') phase = { kind: 'error', message: reason };
        stopInput();
      },
    }).then(
      (s) => {
        if (cancelled) s.stop();
        else session = s;
      },
      (e) => (phase = { kind: 'error', message: (e as Error).message }),
    );

    const statsTimer = setInterval(async () => {
      if (showStats && session) stats = await session.stats();
    }, 1000);

    const onFs = () => {
      const fullscreen = document.fullscreenElement === root;
      // In fullscreen, Chromium lets us capture Esc, Win, Alt+Tab etc. for the host.
      const kb = (navigator as Navigator & { keyboard?: { lock?: () => Promise<void>; unlock?: () => void } }).keyboard;
      if (fullscreen) kb?.lock?.().catch(() => {});
      else kb?.unlock?.();
    };
    const onLock = () => {
      pointerLocked = document.pointerLockElement === surface;
      if (pointerLocked) lockError = null;
    };
    document.addEventListener('fullscreenchange', onFs);
    document.addEventListener('pointerlockchange', onLock);

    return () => {
      cancelled = true;
      stopInput();
      session?.stop();
      clearInterval(statsTimer);
      document.removeEventListener('fullscreenchange', onFs);
      document.removeEventListener('pointerlockchange', onLock);
    };
  });

  function toggleFullscreen() {
    if (document.fullscreenElement) document.exitFullscreen();
    else root.requestFullscreen({ navigationUI: 'hide' }).catch(() => {});
  }

  function leave() {
    stopInput();
    router.go(backUrl);
  }

  const fmt = (n: number | null, digits = 0) => (n == null ? '–' : n.toFixed(digits));
</script>

<div class="stream" bind:this={root}>
  <div
    class="surface"
    class:hide-cursor={phase.kind === 'streaming' && !relative}
    bind:this={surface}
    role="application"
    aria-label="Game stream — input is sent to the host"
  >
    <!-- svelte-ignore a11y_media_has_caption -->
    <video bind:this={video} autoplay playsinline class:visible={phase.kind === 'streaming'}></video>
  </div>

  {#if phase.kind === 'streaming' && (poorConnection || soundBlocked || inputPaused)}
    <div class="notices">
      {#if inputPaused}
        <button class="chip action" onclick={() => setInputPaused(false)} title="Ctrl+Alt+Shift+I">
          Input paused — click or <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> to resume
        </button>
      {/if}
      {#if poorConnection}<span class="chip warn"><Icon name="alert" size={13} /> Poor connection</span>{/if}
      {#if soundBlocked}
        <button class="chip action" onclick={enableSound} title="Your browser blocked autoplay with sound">
          <Icon name="alert" size={13} /> Enable sound
        </button>
      {/if}
    </div>
  {/if}

  {#if showHint && !needsCapture}
    <div class="hint" aria-live="polite">
      <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Shift</kbd> + <kbd>Q</kbd> disconnect · <kbd>I</kbd> pause input · <kbd>X</kbd> fullscreen · <kbd>S</kbd> stats
    </div>
  {/if}

  {#if needsCapture}
    <button class="capture" onclick={capture}>
      <b>Click to control</b>
      {#if lockError}<span class="lock-err">{lockError}</span>{/if}
      <span><kbd>Esc</kbd> releases the mouse</span>
      <span><kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Shift</kbd> + <kbd>Q</kbd> disconnect · <kbd>I</kbd> pause input · <kbd>X</kbd> fullscreen · <kbd>S</kbd> stats</span>
    </button>
  {/if}

  {#if showStats && stats && phase.kind === 'streaming'}
    <dl class="stats">
      <dt>Video</dt><dd>{stats.width}×{stats.height} {stats.codec}</dd>
      <dt>FPS</dt><dd>{fmt(stats.fps)}</dd>
      <dt>Bitrate</dt><dd>{fmt(stats.bitrateMbps, 1)} Mbps</dd>
      <dt>RTT</dt><dd>{fmt(stats.rttMs)} ms</dd>
      <dt>Jitter buffer</dt><dd>{fmt(stats.jitterBufferMs, 1)} ms</dd>
      <dt>Decode</dt><dd>{fmt(stats.decodeMs, 1)} ms</dd>
      <dt>Lost / dropped</dt><dd>{stats.packetsLost} / {stats.framesDropped}</dd>
    </dl>
  {/if}

  {#if phase.kind !== 'streaming'}
    <div class="center">
      {#if phase.kind === 'connecting'}
        <div class="spinner"></div>
        <h2>Starting {appTitle ?? 'stream'}…</h2>
        <p>{phase.stage}</p>
      {:else}
        <div class="state-icon" class:err={phase.kind === 'error'}>
          <Icon name={phase.kind === 'error' ? 'alert' : 'check'} size={26} />
        </div>
        <h2>{phase.kind === 'error' ? "Couldn't stream" : 'Stream ended'}</h2>
        {#if phase.message}<p>{phase.message}</p>{/if}
        <button class="btn btn-primary" onclick={leave}><Icon name="arrow-left" size={15} /> Back to apps</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .stream {
    position: fixed;
    inset: 0;
    background: #000;
    color: var(--text);
    overflow: hidden;
  }
  .surface {
    position: absolute;
    inset: 0;
  }
  /* Absolute mode: the host draws its own cursor into the video. */
  .hide-cursor {
    cursor: none;
  }

  video {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    opacity: 0;
    transition: opacity 300ms ease;
  }
  video.visible {
    opacity: 1;
  }

  .notices {
    position: absolute;
    top: 12px;
    right: 12px;
    z-index: 2;
    display: flex;
    gap: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 12px;
    font-weight: 500;
  }
  .chip.warn {
    background: rgba(245, 184, 74, 0.15);
    color: var(--warning);
  }
  .chip.action {
    border: 1px solid rgba(245, 184, 74, 0.4);
    background: rgba(8, 14, 28, 0.8);
    color: var(--warning);
    font-family: var(--font);
    cursor: pointer;
  }

  .hint {
    position: absolute;
    left: 50%;
    bottom: 32px;
    z-index: 2;
    transform: translateX(-50%);
    padding: 10px 16px;
    border-radius: var(--radius);
    background: rgba(8, 14, 28, 0.78);
    border: 1px solid var(--border);
    backdrop-filter: blur(10px);
    color: var(--muted);
    font-size: 12px;
    white-space: nowrap;
    pointer-events: none;
    animation: hint 5s ease forwards;
  }
  @keyframes hint {
    0% {
      opacity: 0;
      transform: translate(-50%, 8px);
    }
    6%,
    85% {
      opacity: 1;
      transform: translate(-50%, 0);
    }
    100% {
      opacity: 0;
    }
  }

  .capture {
    position: absolute;
    left: 50%;
    bottom: 32px;
    z-index: 2;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 14px 22px;
    border: 1px solid rgba(79, 140, 255, 0.45);
    border-radius: var(--radius);
    background: rgba(8, 14, 28, 0.82);
    backdrop-filter: blur(10px);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5), 0 0 24px var(--accent-glow);
    color: var(--text);
    font: 14px var(--font);
    cursor: pointer;
    animation: rise 200ms ease;
  }
  .capture span {
    color: var(--muted);
    font-size: 12px;
  }
  .capture .lock-err {
    color: var(--warning);
  }
  kbd {
    padding: 1px 5px;
    border: 1px solid var(--border-strong);
    border-bottom-width: 2px;
    border-radius: 4px;
    background: var(--surface-2);
    color: var(--text);
    font: 11px var(--font);
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translate(-50%, 8px);
    }
  }

  .stats {
    position: absolute;
    top: 12px;
    left: 12px;
    z-index: 2;
    display: grid;
    grid-template-columns: auto auto;
    gap: 3px 14px;
    margin: 0;
    padding: 12px 14px;
    border-radius: var(--radius-sm);
    background: rgba(8, 14, 28, 0.78);
    border: 1px solid var(--border);
    backdrop-filter: blur(8px);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    pointer-events: none;
  }
  .stats dt {
    color: var(--muted);
  }
  .stats dd {
    margin: 0;
    text-align: right;
  }

  .center {
    position: absolute;
    inset: 0;
    z-index: 2;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 24px;
    text-align: center;
    background: radial-gradient(60% 50% at 50% 45%, rgba(79, 140, 255, 0.12), transparent 70%);
  }
  .center h2 {
    font-size: 18px;
  }
  .center p {
    max-width: 520px;
    margin: 0 0 8px;
    color: var(--muted);
    font-size: 14px;
  }
  .spinner {
    width: 40px;
    height: 40px;
    margin-bottom: 8px;
    border-radius: 50%;
    border: 3px solid var(--border-strong);
    border-top-color: var(--accent);
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .state-icon {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    margin-bottom: 4px;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .state-icon.err {
    background: rgba(255, 92, 122, 0.12);
    color: var(--danger);
  }
</style>
