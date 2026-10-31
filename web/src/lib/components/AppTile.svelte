<script lang="ts">
  import type { App } from '../api';
  import AppArt from './AppArt.svelte';

  let { hostId, app, onlaunch }: { hostId: string; app: App; onlaunch: (app: App) => void } = $props();
</script>

<li class="tile" class:running={app.running}>
  <button class="poster" onclick={() => onlaunch(app)} aria-label="{app.running ? 'Resume' : 'Start'} {app.title}">
    <AppArt {hostId} {app} />

    <span class="overlay">
      <span class="play">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M7 4.5v15l12-7.5z" /></svg>
      </span>
      <span class="action">{app.running ? 'Resume' : 'Start'}</span>
    </span>

    {#if app.running}
      <span class="badge mono">Running</span>
    {/if}
  </button>

  <div class="meta">
    <span class="title" title={app.title}>{app.title}</span>
    {#if app.hdr}
      <span class="hdr mono" title="HDR supported">HDR</span>
    {/if}
  </div>
</li>

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
    background: var(--surface);
    cursor: pointer;
    transition: transform var(--ease), border-color var(--ease);
  }
  .poster:hover,
  .poster:focus-visible {
    transform: translateY(-4px);
    border-color: var(--dim);
  }
  .running .poster {
    border-color: var(--accent);
  }

  .overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    background: rgba(13, 14, 17, 0.6);
    color: var(--text);
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
    width: 52px;
    height: 52px;
    padding-left: 3px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
    transform: scale(0.9);
    transition: transform var(--ease);
  }
  .poster:hover .play {
    transform: scale(1);
  }
  .action {
    font-size: 13px;
    font-weight: 600;
  }

  .badge {
    position: absolute;
    top: 10px;
    left: 10px;
    padding: 3px 8px;
    border-radius: 4px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 24px;
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
    padding: 0 5px;
    border-radius: 3px;
    border: 1px solid var(--border-strong);
    color: var(--muted);
    font-size: 10px;
  }
</style>
