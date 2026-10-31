<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../lib/components/Icon.svelte';
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

<section class="page">
  <div class="page-header">
    <div>
      <div class="eyebrow">03 — Settings</div>
      <h1>Stream profile</h1>
      <p>Saved in this browser and applied to the next stream.</p>
    </div>
    <button class="btn" onclick={resetSettings}>Reset to defaults</button>
  </div>

  <dl class="readout card">
    <div>
      <dt>Resolution{settings.nativeResolution ? ' · native' : ''}</dt>
      <dd>{effective.width}×{effective.height}</dd>
    </div>
    <div>
      <dt>FPS{settings.nativeFps ? ' · native' : ''}</dt>
      <dd>{effective.fps}</dd>
    </div>
    <div>
      <dt>Bitrate</dt>
      <dd class="accent">{mbps(settings.bitrateKbps)}<span class="unit">Mbps</span></dd>
    </div>
    <div>
      <dt>Codec{settings.codec === 'auto' ? ' · auto' : ''}</dt>
      <dd>{effectiveCodec ? CODEC_LABELS[effectiveCodec].replace(' (H.265)', '') : '…'}</dd>
    </div>
    <div>
      <dt>Renderer</dt>
      <dd>{webCodecs ? 'WebCodecs' : 'WebRTC'}</dd>
    </div>
  </dl>

  <div class="layout">
    <nav class="toc mono" aria-label="Settings sections">
      <a href="#video">01 Video</a>
      <a href="#input">02 Input</a>
      <a href="#interface">03 Interface</a>
    </nav>

    <div class="sections">
      <section id="video">
        <h2>Video</h2>

        <div class="field">
          <span class="label">Resolution</span>
          <div class="segmented" role="group" aria-label="Resolution">
            {#each presets as p (p.label)}
              <button
                aria-pressed={!showCustomRes && !settings.nativeResolution && settings.width === p.w && settings.height === p.h}
                onclick={() => pickPreset(p.w, p.h)}>{p.label}</button
              >
            {/each}
            <button aria-pressed={settings.nativeResolution} onclick={pickNativeRes} title="Match this display">
              Native <span class="dim mono">{native.width}×{native.height}</span>
            </button>
            <button aria-pressed={showCustomRes} onclick={pickCustomRes}>Custom</button>
          </div>
          {#if showCustomRes}
            <div class="custom">
              <input
                class="input num mono"
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
                class="input num mono"
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
          <div class="segmented" role="group" aria-label="Frame rate">
            {#each fpsPresets as f (f)}
              <button aria-pressed={!showCustomFps && !settings.nativeFps && settings.fps === f} onclick={() => pickFps(f)}
                >{f}</button
              >
            {/each}
            <button aria-pressed={settings.nativeFps} onclick={pickNativeFps} title="Match this display's refresh rate">
              Native <span class="dim mono">{nativeHz ? `${nativeHz} Hz` : '…'}</span>
            </button>
            <button aria-pressed={showCustomFps} onclick={pickCustomFps}>Custom</button>
          </div>
          {#if showCustomFps}
            <div class="custom">
              <input
                class="input num mono"
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
          <div class="bitrate-head">
            <label class="label" for="bitrate">Bitrate</label>
            <span class="bitrate-value mono">{mbps(settings.bitrateKbps)} <span class="unit">Mbps</span></span>
          </div>
          <input id="bitrate" type="range" min="1000" max="150000" step="500" bind:value={settings.bitrateKbps} />
          <div class="scale mono" aria-hidden="true"><span>1</span><span>50</span><span>100</span><span>150</span></div>
          <p class="hint">
            Recommended for {effective.width}×{effective.height} @ {effective.fps} FPS:
            <b class="mono">{mbps(recommended)} Mbps</b>
            {#if settings.bitrateKbps !== recommended}
              · <button class="link" onclick={() => (settings.bitrateKbps = recommended)}>Use</button>
            {/if}
          </p>
        </div>

        <div class="field">
          <span class="label">Video renderer</span>
          <div class="choices" role="group" aria-label="Video renderer">
            <button class="choice" aria-pressed={!webCodecs} onclick={() => (settings.renderer = 'webrtc')}>
              <span class="choice-title">Browser <Icon name="check" size={16} /></span>
              <span class="eyebrow">WebRTC</span>
              <span class="choice-desc">The browser's own player. It may hold frames briefly to smooth out network jitter.</span>
            </button>
            <button
              class="choice"
              aria-pressed={webCodecs}
              disabled={!canWebCodecs}
              onclick={() => (settings.renderer = 'webcodecs')}
              title={canWebCodecs ? '' : 'Needs HTTPS (or localhost)'}
            >
              <span class="choice-title">Lowest latency <Icon name="check" size={16} /></span>
              <span class="eyebrow">WebCodecs</span>
              <span class="choice-desc">
                {#if canWebCodecs}
                  Each frame is drawn the moment it decodes: no jitter buffer, no frame pacing. A network hiccup shows
                  as a stall rather than being smoothed over.
                {:else}
                  Needs this page to be served over HTTPS (or from localhost).
                {/if}
              </span>
            </button>
          </div>
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
              This exceeds what most H.264 hardware decoders handle (level 5.2). If the stream stutters, pick HEVC or
              AV1 if available, or lower the frame rate.
            </p>
          {/if}
        </div>
      </section>

      <section id="input">
        <h2>Input</h2>
        <div class="field">
          <span class="label">Mouse mode</span>
          <div class="segmented" role="group" aria-label="Mouse mode">
            <button aria-pressed={settings.mouseMode === 'relative'} onclick={() => (settings.mouseMode = 'relative')}
              >Relative · games</button
            >
            <button aria-pressed={settings.mouseMode === 'absolute'} onclick={() => (settings.mouseMode = 'absolute')}
              >Absolute · desktop</button
            >
          </div>
        </div>
      </section>

      <section id="interface">
        <h2>Interface</h2>
        <label class="toggle">
          <span>Show performance stats while streaming</span>
          <input type="checkbox" bind:checked={settings.showStats} />
          <span class="track" aria-hidden="true"><span class="thumb"></span></span>
        </label>
      </section>
    </div>
  </div>
</section>

<style>
  .readout {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    margin: 0 0 56px;
    overflow: hidden;
  }
  .readout > div {
    display: flex;
    flex-direction: column-reverse;
    justify-content: flex-end;
    gap: 4px;
    padding: 20px 22px;
    border-right: 1px solid var(--border);
    margin-right: -1px;
  }
  .readout dd {
    margin: 0;
    font-family: var(--mono);
    font-size: 26px;
    font-weight: 500;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .readout dt {
    color: var(--faint);
    font-family: var(--mono);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .accent {
    color: var(--accent);
  }
  .unit {
    margin-left: 4px;
    color: var(--muted);
    font-size: 14px;
  }

  .layout {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: 24px 48px;
  }
  .toc {
    flex: 1 1 160px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
  }
  .toc a {
    padding: 8px 0;
    color: var(--muted);
  }
  .toc a:hover {
    color: var(--text);
  }
  .sections {
    flex: 999 1 560px;
    min-width: 0;
  }
  @media (max-width: 760px) {
    .toc {
      display: none;
    }
  }

  section[id] {
    padding: 40px 0;
    border-bottom: 1px solid var(--border);
    scroll-margin-top: calc(var(--topbar-h) + 16px);
  }
  section[id]:first-child {
    padding-top: 0;
  }
  section[id]:last-child {
    border-bottom: 0;
  }
  section[id] h2 {
    margin-bottom: 28px;
    font-size: 22px;
  }
  .field + .field {
    margin-top: 32px;
  }

  .segmented {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 2px;
    padding: 3px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }
  .segmented button {
    height: 40px;
    padding: 0 14px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--muted);
    font: 500 13px var(--font);
    white-space: nowrap;
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
  .segmented button[aria-pressed='true'] {
    background: var(--text);
    color: var(--bg);
  }
  .dim {
    margin-left: 4px;
    opacity: 0.65;
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
    width: 110px;
  }

  .bitrate-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
  }
  .bitrate-head .label {
    margin-bottom: 6px;
  }
  .bitrate-value {
    font-size: 20px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }
  .bitrate-value .unit {
    font-size: 13px;
  }
  input[type='range'] {
    width: 100%;
    height: 28px;
    margin: 0;
    accent-color: var(--accent);
  }
  .scale {
    display: flex;
    justify-content: space-between;
    color: var(--faint);
    font-size: 11px;
  }

  .choices {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 12px;
  }
  .choice {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 16px 18px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: transparent;
    color: var(--text);
    font: 15px var(--font);
    text-align: left;
    cursor: pointer;
    transition: border-color var(--ease), background var(--ease);
  }
  .choice:hover:not(:disabled) {
    border-color: var(--border-strong);
  }
  .choice:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .choice[aria-pressed='true'] {
    border-color: var(--accent);
    background: var(--surface);
  }
  .choice-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-weight: 600;
  }
  .choice-title :global(svg) {
    color: var(--accent);
    opacity: 0;
  }
  .choice[aria-pressed='true'] .choice-title :global(svg) {
    opacity: 1;
  }
  .choice .eyebrow {
    margin-bottom: 6px;
    font-size: 11px;
    letter-spacing: 0.06em;
  }
  .choice-desc {
    color: var(--muted);
    font-size: 13px;
  }

  .hint {
    margin: 10px 0 0;
    color: var(--muted);
    font-size: 13px;
  }
  .hint b {
    color: var(--text);
    font-weight: 500;
  }
  .hint.warn {
    color: var(--warning);
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: 500 13px var(--font);
    text-decoration: underline;
    text-underline-offset: 3px;
    cursor: pointer;
  }
  .link:hover {
    color: var(--accent-hover);
  }

  .toggle {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-height: 44px;
    cursor: pointer;
  }
  .toggle input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .track {
    flex: none;
    width: 44px;
    height: 26px;
    padding: 3px;
    border-radius: 13px;
    background: var(--border);
    transition: background var(--ease);
  }
  .thumb {
    display: block;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--muted);
    transition: transform var(--ease), background var(--ease);
  }
  .toggle input:checked + .track {
    background: var(--accent);
  }
  .toggle input:checked + .track .thumb {
    transform: translateX(18px);
    background: var(--bg);
  }
  .toggle input:focus-visible + .track {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
