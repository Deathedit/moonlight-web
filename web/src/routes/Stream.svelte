<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import Icon from '../lib/components/Icon.svelte';
  import MoonPhase from '../lib/components/MoonPhase.svelte';
  import { router } from '../lib/router.svelte';
  import { settings } from '../lib/settings.svelte';
  import { GamepadForwarder } from '../lib/stream/gamepad';
  import { InputCapture, type Shortcut, type VideoView } from '../lib/stream/input';
  import { StreamSession, type ControlMessage, type StreamStats } from '../lib/stream/session';

  let { hostId, appId }: { hostId: string; appId: string } = $props();

  type Phase =
    | { kind: 'connecting'; stage: string }
    | { kind: 'reconnecting'; attempt: number; reason: string }
    | { kind: 'streaming' }
    | { kind: 'error'; message: string }
    | { kind: 'ended'; message: string | null };

  let phase = $state<Phase>({ kind: 'connecting', stage: 'Contacting bridge' });
  /** Stages already passed while connecting, shown as a checklist above the current one. */
  let pastStages = $state<string[]>([]);
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
  /** Plays audio, and video unless the WebCodecs renderer's canvas shows it. */
  let video: HTMLVideoElement;
  let canvasHost: HTMLDivElement;
  let session: StreamSession | null = null;
  let input: InputCapture | null = null;
  let pads: GamepadForwarder | null = null;

  const backUrl = $derived(`/hosts/${encodeURIComponent(hostId)}/apps`);
  const relative = settings.mouseMode === 'relative';
  const needsCapture = $derived(phase.kind === 'streaming' && relative && !pointerLocked && !inputPaused);

  function onControl(msg: ControlMessage) {
    switch (msg.type) {
      case 'stage':
        if (phase.kind === 'connecting' && msg.stage !== phase.stage) {
          pastStages = [...pastStages, phase.stage];
          phase = { kind: 'connecting', stage: msg.stage };
        }
        break;
      case 'started':
        phase = { kind: 'streaming' };
        attempts = 0;
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
        if (msg.retryable && retry(msg.error)) break;
        phase = { kind: 'error', message: msg.error };
        stopInput();
        break;
      case 'ended':
        if (msg.retryable && retry(msg.reason ?? 'The connection was lost.')) break;
        phase = { kind: 'ended', message: msg.reason };
        stopInput();
        break;
    }
  }

  function startInput() {
    if (!session || input) return;
    input = new InputCapture(surface, currentView, session.input, settings.mouseMode, onShortcut);
    pads = new GamepadForwarder(session.input.reliable);
    input.onLockError = (msg) => (lockError = msg);
    // Relative mode captures on the first click (browsers require a user gesture).
  }

  function currentView(): VideoView {
    const r = session?.renderer;
    if (r) return { el: r.canvas, width: r.width, height: r.height };
    return { el: video, width: video.videoWidth, height: video.videoHeight };
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

  // ---- (Re)connecting ----

  const MAX_RETRIES = 3;
  let attempts = 0;
  let unmounted = false;
  let retryTimer: ReturnType<typeof setTimeout> | undefined;

  /** Start a session. Each call gets its own handlers, so a stale session can't touch state. */
  function connect() {
    const id = Number(appId);
    let current: StreamSession | null = null;
    const isCurrent = () => !unmounted && session === current;

    StreamSession.start(hostId, id, settings, {
      onMessage: (msg) => isCurrent() && onControl(msg),
      onMedia: (stream) => isCurrent() && attachMedia(stream),
      onDisconnected: (reason) => {
        if (!isCurrent() || phase.kind === 'ended') return;
        if (!retry(reason)) {
          phase = { kind: 'error', message: reason };
          stopInput();
        }
      },
    }).then(
      (s) => {
        if (unmounted) return s.stop();
        current = s;
        session = s;
        canvasHost.replaceChildren(...(s.renderer ? [s.renderer.canvas] : []));
      },
      (e) => {
        if (unmounted) return;
        const message = (e as Error).message;
        // The bridge itself unreachable (fetch failed) is worth retrying; an explicit
        // refusal (bad settings, unpaired...) is not.
        if (!(e instanceof TypeError && retry(message))) phase = { kind: 'error', message };
      },
    );
  }

  /** Schedule a reconnect if attempts remain. Returns false when giving up. */
  function retry(reason: string): boolean {
    if (unmounted || attempts >= MAX_RETRIES) return false;
    attempts++;
    stopInput();
    session?.stop();
    session = null;
    phase = { kind: 'reconnecting', attempt: attempts, reason };
    retryTimer = setTimeout(connect, 1000 * 2 ** (attempts - 1)); // 1s, 2s, 4s
    return true;
  }

  function reconnectNow() {
    clearTimeout(retryTimer);
    attempts = 0;
    session?.stop();
    session = null;
    pastStages = [];
    phase = { kind: 'connecting', stage: 'Reconnecting' };
    connect();
  }

  onMount(() => {
    const id = Number(appId);
    api.listApps(hostId).then(
      (apps) => (appTitle = apps.find((a) => a.id === id)?.title ?? null),
      () => {},
    );

    connect();

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
      unmounted = true;
      clearTimeout(retryTimer);
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
    <div class="view" class:visible={phase.kind === 'streaming'}>
      <!-- svelte-ignore a11y_media_has_caption -->
      <video bind:this={video} autoplay playsinline></video>
      <div class="canvas-host" bind:this={canvasHost}></div>
    </div>
  </div>

  {#if phase.kind === 'streaming'}
    <div class="top">
      {#if showStats && stats}
        <dl class="stats mono">
          <div><dt>Video</dt><dd>{stats.codec} {stats.width}×{stats.height}</dd></div>
          <div><dt>Renderer</dt><dd>{stats.renderer}</dd></div>
          <div><dt>FPS</dt><dd class="accent">{fmt(stats.fps)}</dd></div>
          <div><dt>Bitrate</dt><dd>{fmt(stats.bitrateMbps, 1)} Mbps</dd></div>
          <div><dt>RTT</dt><dd>{fmt(stats.rttMs)} ms</dd></div>
          <div><dt>Decode</dt><dd>{fmt(stats.decodeMs, 1)} ms</dd></div>
          {#if stats.renderer !== 'WebCodecs'}
            <div><dt>Jitter buf</dt><dd>{fmt(stats.jitterBufferMs, 1)} ms</dd></div>
          {/if}
          <div><dt>Lost / drop</dt><dd>{stats.packetsLost} / {stats.framesDropped}</dd></div>
        </dl>
      {/if}

      {#if poorConnection || soundBlocked || inputPaused}
        <div class="notices">
          {#if inputPaused}
            <button class="chip" onclick={() => setInputPaused(false)} title="Ctrl+Alt+Shift+I">
              Input paused — click or <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> to resume
            </button>
          {/if}
          {#if poorConnection}<span class="chip warn"><Icon name="alert" size={14} /> Poor connection</span>{/if}
          {#if soundBlocked}
            <button class="chip" onclick={enableSound} title="Your browser blocked autoplay with sound">
              <Icon name="volume" size={14} /> Enable sound
            </button>
          {/if}
        </div>
      {/if}
    </div>
  {/if}

  {#if showHint && !needsCapture}
    <div class="hint" aria-live="polite">
      <span><kbd>Ctrl</kbd> <kbd>Alt</kbd> <kbd>Shift</kbd> +</span>
      <span><kbd>Q</kbd> disconnect</span>
      <span><kbd>I</kbd> pause input</span>
      <span><kbd>X</kbd> fullscreen</span>
      <span><kbd>S</kbd> stats</span>
    </div>
  {/if}

  {#if needsCapture}
    <button class="capture" onclick={capture}>
      <b>Click to control</b>
      {#if lockError}<span class="lock-err">{lockError}</span>{/if}
      <span><kbd>Esc</kbd> releases the mouse</span>
      <span>
        <kbd>Ctrl</kbd> <kbd>Alt</kbd> <kbd>Shift</kbd> + <kbd>Q</kbd> disconnect · <kbd>I</kbd> pause input ·
        <kbd>X</kbd> fullscreen · <kbd>S</kbd> stats
      </span>
    </button>
  {/if}

  {#if phase.kind !== 'streaming'}
    <div class="center">
      {#if phase.kind === 'connecting' || phase.kind === 'reconnecting'}
        <svg class="loader" width="88" height="88" viewBox="0 0 44 44" role="img" aria-label="Connecting">
          <defs><clipPath id="moon-disc"><circle cx="22" cy="22" r="20" /></clipPath></defs>
          <circle class="lit" cx="22" cy="22" r="20" />
          <g clip-path="url(#moon-disc)"><circle class="shade" cx="22" cy="22" r="20" /></g>
          <circle class="rim" cx="22" cy="22" r="20" />
        </svg>
      {/if}

      {#if phase.kind === 'connecting'}
        <div class="eyebrow">Connecting</div>
        <h2>Starting {appTitle ?? 'stream'}</h2>
        <ol class="stages mono">
          {#each pastStages as s, i (i)}
            <li class="done"><MoonPhase phase="full" size={12} />{s}</li>
          {/each}
          <li class="current"><MoonPhase phase="half" size={12} />{phase.stage}</li>
        </ol>
        <button class="btn" onclick={leave}><Icon name="arrow-left" size={15} /> Cancel</button>
      {:else if phase.kind === 'reconnecting'}
        <div class="eyebrow">Attempt {phase.attempt} of {MAX_RETRIES}</div>
        <h2>Reconnecting</h2>
        <p>{phase.reason}</p>
        <button class="btn" onclick={leave}><Icon name="arrow-left" size={15} /> Back to apps</button>
      {:else}
        {#if phase.kind === 'error'}
          <span class="state-icon err"><Icon name="alert" size={28} /></span>
        {:else}
          <MoonPhase phase="new" size={72} />
        {/if}
        <div class="eyebrow">{phase.kind === 'error' ? 'Error' : 'Session over'}</div>
        <h2>{phase.kind === 'error' ? "Couldn't stream" : 'Stream ended'}</h2>
        {#if phase.message}<p>{phase.message}</p>{/if}
        <div class="actions">
          <button class="btn" onclick={leave}><Icon name="arrow-left" size={15} /> Back to apps</button>
          <button class="btn btn-primary" onclick={reconnectNow}><Icon name="refresh" size={15} /> Reconnect</button>
        </div>
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

  .view {
    position: absolute;
    inset: 0;
    opacity: 0;
    transition: opacity 300ms ease;
  }
  .view.visible {
    opacity: 1;
  }
  video,
  .canvas-host :global(canvas) {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  /* Overlays sit over the video: translucent ground, hairline border, mono values. */
  .top {
    position: absolute;
    top: 16px;
    left: 50%;
    z-index: 2;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    width: max-content;
    max-width: calc(100vw - 24px);
    pointer-events: none;
  }
  .stats {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    margin: 0;
    overflow: hidden;
    border: 1px solid rgba(236, 233, 225, 0.12);
    border-radius: var(--radius);
    background: rgba(13, 14, 17, 0.82);
    backdrop-filter: blur(10px);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .stats div {
    padding: 8px 14px;
    border-right: 1px solid rgba(236, 233, 225, 0.1);
  }
  .stats div:last-child {
    border-right: 0;
  }
  .stats dt {
    color: var(--faint);
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .stats dd {
    margin: 0;
  }
  .stats .accent {
    color: var(--accent);
  }

  .notices {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-height: 36px;
    padding: 0 14px;
    border: 1px solid rgba(235, 217, 160, 0.4);
    border-radius: 18px;
    background: rgba(13, 14, 17, 0.85);
    color: var(--text);
    font: 500 13px var(--font);
    pointer-events: auto;
  }
  .chip :global(svg) {
    color: var(--accent);
  }
  button.chip {
    cursor: pointer;
  }
  .chip.warn {
    border-color: rgba(235, 194, 122, 0.45);
    color: var(--warning);
  }
  .chip.warn :global(svg) {
    color: var(--warning);
  }

  .hint {
    position: absolute;
    left: 50%;
    bottom: 24px;
    z-index: 2;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 16px;
    border: 1px solid rgba(236, 233, 225, 0.12);
    border-radius: var(--radius);
    background: rgba(13, 14, 17, 0.82);
    backdrop-filter: blur(10px);
    color: var(--muted);
    font-size: 13px;
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
    padding: 16px 24px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-lg);
    background: rgba(13, 14, 17, 0.88);
    backdrop-filter: blur(10px);
    color: var(--text);
    font: 15px var(--font);
    cursor: pointer;
    animation: rise 200ms ease;
  }
  .capture b {
    font-weight: 600;
  }
  .capture span {
    color: var(--muted);
    font-size: 12px;
  }
  .capture .lock-err {
    color: var(--warning);
  }
  kbd {
    padding: 1px 6px;
    border: 1px solid var(--border-strong);
    border-bottom-width: 2px;
    border-radius: 4px;
    background: var(--surface);
    color: var(--text);
    font-size: 11px;
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translate(-50%, 8px);
    }
  }

  .center {
    position: absolute;
    inset: 0;
    z-index: 2;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 24px;
    text-align: center;
    background: var(--bg);
  }
  .center .eyebrow {
    margin-top: 36px;
  }
  .center h2 {
    margin: 10px 0 0;
    font-size: 40px;
    letter-spacing: -0.03em;
  }
  .center p {
    max-width: 520px;
    margin: 12px 0 0;
    color: var(--muted);
  }
  .center > .btn,
  .actions {
    margin-top: 40px;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 10px;
  }

  .loader .lit {
    fill: var(--accent);
  }
  .loader .shade {
    fill: var(--bg);
    animation: phase 2.4s cubic-bezier(0.45, 0, 0.55, 1) infinite alternate;
  }
  .loader .rim {
    fill: none;
    stroke: var(--border-strong);
    stroke-width: 1;
  }
  @keyframes phase {
    from {
      transform: translateX(-22px);
    }
    to {
      transform: translateX(22px);
    }
  }

  .stages {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 32px 0 0;
    padding: 0;
    list-style: none;
    font-size: 13px;
    text-align: left;
  }
  .stages li {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .stages .done {
    color: var(--muted);
  }
  .stages .done :global(svg) {
    fill: var(--muted);
  }
  .stages .current {
    color: var(--text);
  }

  .state-icon {
    display: grid;
    place-items: center;
    width: 72px;
    height: 72px;
    border-radius: 50%;
    border: 1px solid var(--danger-border);
    color: var(--danger);
  }
</style>
