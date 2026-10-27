<script lang="ts">
  import { api, type App } from '../api';
  import Icon from './Icon.svelte';

  let {
    hostId,
    app,
    onlaunch,
    onquit,
  }: { hostId: string; app: App; onlaunch: (app: App) => void; onquit: (app: App) => void } = $props();

  let artFailed = $state(false);
  let artLoaded = $state(false);

  const initials = $derived(
    app.title
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w[0].toUpperCase())
      .join(''),
  );
</script>

<article class="tile" class:running={app.running}>
  <button class="poster" onclick={() => onlaunch(app)} aria-label="{app.running ? 'Resume' : 'Start'} {app.title}">
    {#if !artFailed}
      <img
        src={api.artUrl(hostId, app.id)}
        alt=""
        loading="lazy"
        class:loaded={artLoaded}
        onload={() => (artLoaded = true)}
        onerror={() => (artFailed = true)}
      />
    {/if}
    {#if artFailed || !artLoaded}
      <span class="fallback">{initials}</span>
    {/if}

    <span class="overlay">
      <span class="play"><Icon name="play" size={22} /></span>
      <span class="action">{app.running ? 'Resume' : 'Start'}</span>
    </span>

    {#if app.running}
      <span class="badge"><span class="pulse"></span>Running</span>
    {/if}
  </button>

  <div class="meta">
    <span class="title" title={app.title}>{app.title}</span>
    {#if app.running}
      <button class="quit" onclick={() => onquit(app)} title="Quit {app.title} on the host">
        <Icon name="x" size={14} /> Quit
      </button>
    {:else if app.hdr}
      <span class="hdr" title="HDR supported">HDR</span>
    {/if}
  </div>
</article>

<style>
  .tile {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }

  .poster {
    position: relative;
    display: block;
    aspect-ratio: 3 / 4;
    padding: 0;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background:
      radial-gradient(120% 80% at 50% 0%, rgba(79, 140, 255, 0.25), transparent 70%),
      linear-gradient(180deg, var(--surface-2), var(--surface));
    box-shadow: var(--shadow);
    cursor: pointer;
    transition: transform var(--ease), box-shadow var(--ease), border-color var(--ease);
  }
  .poster:hover,
  .poster:focus-visible {
    transform: translateY(-4px);
    box-shadow: var(--shadow-lift);
    border-color: rgba(79, 140, 255, 0.5);
  }
  .running .poster {
    border-color: rgba(79, 140, 255, 0.6);
    box-shadow:
      0 0 0 1px rgba(79, 140, 255, 0.35),
      0 8px 32px rgba(79, 140, 255, 0.25);
  }

  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
    transition: opacity 250ms ease;
  }
  img.loaded {
    opacity: 1;
  }

  .fallback {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--accent);
    font-size: 32px;
    font-weight: 700;
    letter-spacing: 0.04em;
  }

  .overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    background: linear-gradient(180deg, rgba(8, 14, 28, 0.15), rgba(8, 14, 28, 0.75));
    color: #fff;
    opacity: 0;
    transition: opacity var(--ease);
  }
  .poster:hover .overlay,
  .poster:focus-visible .overlay {
    opacity: 1;
  }
  .play {
    display: grid;
    place-items: center;
    width: 54px;
    height: 54px;
    padding-left: 3px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 6px 24px var(--accent-glow);
    transform: scale(0.9);
    transition: transform var(--ease);
  }
  .play :global(svg) {
    fill: currentColor;
  }
  .poster:hover .play {
    transform: scale(1);
  }
  .action {
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.02em;
  }

  .badge {
    position: absolute;
    top: 10px;
    left: 10px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 9px;
    border-radius: 999px;
    background: rgba(8, 14, 28, 0.8);
    backdrop-filter: blur(6px);
    border: 1px solid rgba(79, 140, 255, 0.45);
    color: var(--text);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .pulse {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--success);
    box-shadow: 0 0 0 0 rgba(62, 207, 142, 0.6);
    animation: pulse 1.8s infinite;
  }
  @keyframes pulse {
    70% {
      box-shadow: 0 0 0 7px rgba(62, 207, 142, 0);
    }
    100% {
      box-shadow: 0 0 0 0 rgba(62, 207, 142, 0);
    }
  }

  .meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 28px;
    padding: 0 2px;
  }
  .title {
    font-size: 14px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .hdr {
    flex: none;
    padding: 2px 6px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    color: var(--faint);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
  }
  .quit {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 28px;
    padding: 0 10px;
    border: 1px solid rgba(255, 92, 122, 0.35);
    border-radius: 7px;
    background: rgba(255, 92, 122, 0.08);
    color: var(--danger);
    font: 500 12px var(--font);
    cursor: pointer;
    transition: background var(--ease);
  }
  .quit:hover {
    background: rgba(255, 92, 122, 0.16);
  }
</style>
