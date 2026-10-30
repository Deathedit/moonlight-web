<script lang="ts">
  import { onMount } from 'svelte';
  import {
    measureRefreshRate,
    nativeResolution,
    recommendedBitrateKbps,
    resetSettings,
    settings,
  } from '../lib/settings.svelte';
  import {
    CODEC_LABELS,
    probeCodecs,
    probeWebCodecs,
    rankForAuto,
    webCodecsAvailable,
    type CodecSupport,
    type VideoCodec,
  } from '../lib/stream/codecs';

  const presets = [
    { label: '720p', w: 1280, h: 720 },
    { label: '1080p', w: 1920, h: 1080 },
    { label: '1440p', w: 2560, h: 1440 },
    { label: '4K', w: 3840, h: 2160 },
  ];
  const fpsPresets = [30, 60, 120, 144, 165];

  let native = $state(nativeResolution());
  let nativeHz = $state<number | null>(null);

  onMount(() => {
    measureRefreshRate().then((hz) => (nativeHz = hz));
    // The window may move to another monitor.
    const update = () => (native = nativeResolution());
    addEventListener('resize', update);
    return () => removeEventListener('resize', update);
  });

  const isPreset = $derived(
    !settings.nativeResolution && presets.some((p) => p.w === settings.width && p.h === settings.height),
  );
  let customRes = $state(false);
  const showCustomRes = $derived(customRes || (!settings.nativeResolution && !isPreset));

  const isFpsPreset = $derived(!settings.nativeFps && fpsPresets.includes(settings.fps));
  let customFps = $state(false);
  const showCustomFps = $derived(customFps || (!settings.nativeFps && !isFpsPreset));

  function pickPreset(w: number, h: number) {
    settings.nativeResolution = false;
    settings.width = w;
    settings.height = h;
    customRes = false;
  }
  function pickNativeRes() {
    settings.nativeResolution = true;
    customRes = false;
  }
  function pickCustomRes() {
    if (settings.nativeResolution) Object.assign(settings, native);
    settings.nativeResolution = false;
    customRes = true;
  }

  function pickFps(f: number) {
    settings.nativeFps = false;
    settings.fps = f;
    customFps = false;
  }
  function pickNativeFps() {
    settings.nativeFps = true;
    customFps = false;
  }
  function pickCustomFps() {
    if (settings.nativeFps && nativeHz) settings.fps = nativeHz;
    settings.nativeFps = false;
    customFps = true;
  }

  // What the next stream will actually request.
  const effective = $derived({
    width: settings.nativeResolution ? native.width : settings.width,
    height: settings.nativeResolution ? native.height : settings.height,
    fps: settings.nativeFps ? (nativeHz ?? 60) : settings.fps,
  });
  const recommended = $derived(recommendedBitrateKbps(effective.width, effective.height, effective.fps));
  const mbps = (kbps: number) => (kbps / 1000).toFixed(kbps % 1000 === 0 ? 0 : 1);

  const canWebCodecs = webCodecsAvailable();
  const webCodecs = $derived(settings.renderer === 'webcodecs' && canWebCodecs);

  // Decoder support depends on resolution/frame rate (and renderer), so re-probe when they change.
  let support = $state<CodecSupport[] | null>(null);
  $effect(() => {
    const mode = { ...effective, bitrateKbps: settings.bitrateKbps };
    const probe = webCodecs ? probeWebCodecs : probeCodecs;
    let stale = false;
    const t = setTimeout(() => probe(mode).then((s) => !stale && (support = s)), 150);
    return () => {
      stale = true;
      clearTimeout(t);
    };
  });
  const autoPick = $derived<VideoCodec | null>(support ? (rankForAuto(support)[0] ?? null) : null);
  const effectiveCodec = $derived(settings.codec === 'auto' ? autoPick : settings.codec);

  function describe(c: VideoCodec): string {
    const s = support?.find((x) => x.codec === c);
    if (!s) return '';
    if (!s.supported) return ' — not supported by this browser';
    if (s.hardware === true) return ' — hardware decode';
    if (s.hardware === false) return ' — software decode';
    return '';
  }

  // H.264 tops out at level 5.2 in most hardware decoders: ~2 million macroblocks/s.
  const h264Heavy = $derived(
    effectiveCodec === 'h264' && (Math.ceil(effective.width / 16) * Math.ceil(effective.height / 16) * effective.fps) > 2_073_600,
  );

  const clampInt = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, Math.round(v || lo)));
</script>

<section class="page narrow">
  <div class="page-header">
    <div>
      <h1>Settings</h1>
      <p>Saved in this browser and applied to the next stream.</p>
    </div>
    <button class="btn" onclick={resetSettings}>Reset to defaults</button>
  </div>

  <div class="card group">
    <h2>Video</h2>

    <div class="field">
      <span class="label">Resolution</span>
      <div class="segmented">
        {#each presets as p (p.label)}
          <button class:active={!showCustomRes && !settings.nativeResolution && settings.width === p.w && settings.height === p.h} onclick={() => pickPreset(p.w, p.h)}>{p.label}</button>
        {/each}
        <button class:active={settings.nativeResolution} onclick={pickNativeRes} title="Match this display">
          Native <span class="dim">{native.width}×{native.height}</span>
        </button>
        <button class:active={showCustomRes} onclick={pickCustomRes}>Custom</button>
      </div>
      {#if showCustomRes}
        <div class="custom">
          <input
            class="input num"
            type="number"
            min="320"
            max="7680"
            step="2"
            aria-label="Width"
            value={settings.width}
            onchange={(e) => (settings.width = clampInt(+e.currentTarget.value, 320, 7680))}
          />
          <span>×</span>
          <input
            class="input num"
            type="number"
            min="240"
            max="4320"
            step="2"
            aria-label="Height"
            value={settings.height}
            onchange={(e) => (settings.height = clampInt(+e.currentTarget.value, 240, 4320))}
          />
        </div>
      {/if}
    </div>

    <div class="field">
      <span class="label">Frame rate</span>
      <div class="segmented">
        {#each fpsPresets as f (f)}
          <button class:active={!showCustomFps && !settings.nativeFps && settings.fps === f} onclick={() => pickFps(f)}>{f}</button>
        {/each}
        <button class:active={settings.nativeFps} onclick={pickNativeFps} title="Match this display's refresh rate">
          Native <span class="dim">{nativeHz ? `${nativeHz} Hz` : '…'}</span>
        </button>
        <button class:active={showCustomFps} onclick={pickCustomFps}>Custom</button>
      </div>
      {#if showCustomFps}
        <div class="custom">
          <input
            class="input num"
            type="number"
            min="10"
            max="240"
            aria-label="Frames per second"
            value={settings.fps}
            onchange={(e) => (settings.fps = clampInt(+e.currentTarget.value, 10, 240))}
          />
          <span>FPS</span>
        </div>
      {/if}
    </div>

    <div class="field">
      <label class="label" for="bitrate">
        Bitrate <span class="value">{mbps(settings.bitrateKbps)} Mbps</span>
      </label>
      <input id="bitrate" type="range" min="1000" max="150000" step="500" bind:value={settings.bitrateKbps} />
      <p class="hint">
        Recommended for {effective.width}×{effective.height} @ {effective.fps} FPS: <b>{mbps(recommended)} Mbps</b>
        {#if settings.bitrateKbps !== recommended}
          · <button class="link" onclick={() => (settings.bitrateKbps = recommended)}>Use</button>
        {/if}
      </p>
    </div>

    <div class="field">
      <span class="label">Video renderer</span>
      <div class="segmented">
        <button class:active={!webCodecs} onclick={() => (settings.renderer = 'webrtc')}>Browser (WebRTC)</button>
        <button
          class:active={webCodecs}
          disabled={!canWebCodecs}
          onclick={() => (settings.renderer = 'webcodecs')}
          title={canWebCodecs ? '' : 'Needs HTTPS (or localhost)'}>WebCodecs (lowest latency)</button
        >
      </div>
      <p class="hint">
        {#if !canWebCodecs}
          WebCodecs needs this page to be served over HTTPS (or from localhost).
        {:else if webCodecs}
          Each frame is drawn the moment it decodes: no jitter buffer, no frame pacing. A network hiccup shows as a
          stall rather than being smoothed over.
        {:else}
          The browser's own player. It may hold frames back briefly to smooth out network jitter.
        {/if}
      </p>
    </div>

    <div class="field">
      <label class="label" for="codec">Codec</label>
      <select id="codec" class="select" bind:value={settings.codec}>
        <option value="auto">Auto{autoPick ? ` — ${CODEC_LABELS[autoPick]}${describe(autoPick)}` : ''}</option>
        {#each Object.entries(CODEC_LABELS) as [value, label] (value)}
          <option {value} disabled={support?.find((s) => s.codec === value)?.supported === false}>
            {label}{describe(value as VideoCodec)}
          </option>
        {/each}
      </select>
      <p class="hint">
        Auto prefers codecs your GPU decodes in hardware, and falls back to whatever the host can encode.
      </p>
      {#if h264Heavy}
        <p class="hint warn">
          This exceeds what most H.264 hardware decoders handle (level 5.2). If the stream stutters, pick HEVC or AV1
          if available, or lower the frame rate.
        </p>
      {/if}
    </div>
  </div>

  <div class="card group">
    <h2>Input</h2>
    <div class="field">
      <span class="label">Mouse mode</span>
      <div class="segmented">
        <button class:active={settings.mouseMode === 'relative'} onclick={() => (settings.mouseMode = 'relative')}
          >Relative (games)</button
        >
        <button class:active={settings.mouseMode === 'absolute'} onclick={() => (settings.mouseMode = 'absolute')}
          >Absolute (desktop)</button
        >
      </div>
    </div>
  </div>

  <div class="card group">
    <h2>Interface</h2>
    <label class="toggle">
      <input type="checkbox" bind:checked={settings.showStats} />
      <span class="track"><span class="thumb"></span></span>
      Show performance stats while streaming
    </label>
  </div>
</section>

<style>
  .narrow {
    max-width: 760px;
  }
  .group {
    padding: 24px;
    margin-bottom: 16px;
  }
  .group h2 {
    font-size: 15px;
    margin-bottom: 20px;
    color: var(--text);
  }
  .field + .field {
    margin-top: 22px;
  }
  .value {
    float: right;
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }

  .segmented {
    display: inline-flex;
    flex-wrap: wrap;
    padding: 3px;
    gap: 2px;
    border-radius: var(--radius-sm);
    background: var(--bg);
    border: 1px solid var(--border);
  }
  .segmented button {
    height: 32px;
    padding: 0 14px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--muted);
    font: 500 13px var(--font);
    cursor: pointer;
    transition: background var(--ease), color var(--ease);
  }
  .segmented button:hover {
    color: var(--text);
  }
  .segmented button:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .segmented button.active {
    background: var(--accent);
    color: #fff;
    box-shadow: 0 2px 10px var(--accent-glow);
  }
  .dim {
    margin-left: 4px;
    opacity: 0.65;
    font-variant-numeric: tabular-nums;
  }

  .custom {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
    color: var(--muted);
    font-size: 13px;
  }
  .num {
    width: 104px;
    font-variant-numeric: tabular-nums;
  }

  input[type='range'] {
    width: 100%;
    accent-color: var(--accent);
  }

  .hint {
    margin: 8px 0 0;
    color: var(--faint);
    font-size: 13px;
  }
  .hint b {
    color: var(--muted);
  }
  .hint.warn {
    color: var(--warning);
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
  }
  .link:hover {
    color: var(--accent-hover);
  }

  .toggle {
    display: flex;
    align-items: center;
    gap: 12px;
    cursor: pointer;
    color: var(--text);
  }
  .toggle input {
    position: absolute;
    opacity: 0;
  }
  .track {
    flex: none;
    width: 40px;
    height: 22px;
    padding: 2px;
    border-radius: 999px;
    background: var(--surface-3);
    border: 1px solid var(--border-strong);
    transition: background var(--ease);
  }
  .thumb {
    display: block;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--muted);
    transition: transform var(--ease), background var(--ease);
  }
  .toggle input:checked + .track {
    background: var(--accent);
    border-color: transparent;
  }
  .toggle input:checked + .track .thumb {
    transform: translateX(18px);
    background: #fff;
  }
  .toggle input:focus-visible + .track {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
