<script lang="ts">
  import type { Host } from '../api';
  import Icon from './Icon.svelte';

  let {
    host,
    onopen,
    onremove,
    onunpair,
  }: {
    host: Host;
    onopen: (host: Host) => void;
    onremove: (host: Host) => void;
    onunpair: (host: Host) => void;
  } = $props();

  let menuOpen = $state(false);

  const status = $derived(
    !host.online ? 'offline' : !host.paired ? 'unpaired' : host.currentGame !== 0 ? 'streaming' : 'online',
  );
  const statusLabel: Record<string, string> = {
    offline: 'Offline',
    unpaired: 'Not paired',
    streaming: 'In session',
    online: 'Online',
  };
</script>

<svelte:window onclick={() => (menuOpen = false)} />

<article class="card host" data-status={status} class:disabled={!host.online}>
  <button class="hit" onclick={() => onopen(host)} disabled={!host.online} aria-label="Open {host.name}"></button>

  <div class="art">
    <div class="screen">
      <Icon name={host.paired ? 'monitor' : 'lock'} size={30} />
    </div>
  </div>

  <div class="meta">
    <div class="name" title={host.name}>{host.name}</div>
    <div class="sub">
      <span class="badge"><span class="dot"></span>{statusLabel[status]}</span>
      <span class="addr">{host.address}</span>
    </div>
  </div>

  <div class="menu-wrap">
    <button
      class="menu-btn"
      aria-label="Host options"
      onclick={(e) => {
        e.stopPropagation();
        menuOpen = !menuOpen;
      }}><Icon name="more" size={18} /></button
    >
    {#if menuOpen}
      <div class="menu" role="menu">
        {#if host.paired}
          <button role="menuitem" onclick={() => onunpair(host)}>
            <Icon name="lock" size={15} /> Unpair
          </button>
        {/if}
        <button role="menuitem" class="danger" onclick={() => onremove(host)}>
          <Icon name="trash" size={15} /> Remove host
        </button>
      </div>
    {/if}
  </div>
</article>

<style>
  .host {
    position: relative;
    overflow: visible;
    transition: transform var(--ease), box-shadow var(--ease), border-color var(--ease);
  }
  .host:not(.disabled):hover {
    transform: translateY(-3px);
    box-shadow: var(--shadow-lift);
    border-color: rgba(79, 140, 255, 0.45);
  }
  .hit {
    position: absolute;
    inset: 0;
    z-index: 1;
    border: 0;
    border-radius: inherit;
    background: transparent;
    cursor: pointer;
  }
  .hit:disabled {
    cursor: default;
  }

  .art {
    display: grid;
    place-items: center;
    aspect-ratio: 16 / 9;
    border-radius: var(--radius-lg) var(--radius-lg) 0 0;
    background:
      radial-gradient(120% 90% at 50% 0%, rgba(79, 140, 255, 0.22), transparent 70%),
      var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .screen {
    display: grid;
    place-items: center;
    width: 64px;
    height: 64px;
    border-radius: 18px;
    background: var(--accent-soft);
    color: var(--accent);
    box-shadow: inset 0 0 0 1px rgba(79, 140, 255, 0.3);
  }
  [data-status='offline'] .art {
    background: var(--surface);
  }
  [data-status='offline'] .screen {
    background: var(--surface-2);
    color: var(--faint);
    box-shadow: inset 0 0 0 1px var(--border);
  }
  [data-status='unpaired'] .screen {
    color: var(--warning);
    background: rgba(245, 184, 74, 0.1);
    box-shadow: inset 0 0 0 1px rgba(245, 184, 74, 0.3);
  }

  .meta {
    padding: 14px 48px 16px 16px;
  }
  .name {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 4px;
    font-size: 13px;
    color: var(--muted);
  }
  .addr {
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--faint);
  }
  [data-status='online'] .dot,
  [data-status='streaming'] .dot {
    background: var(--success);
    box-shadow: 0 0 6px var(--success);
  }
  [data-status='streaming'] .badge {
    color: var(--accent);
  }
  [data-status='unpaired'] .dot {
    background: var(--warning);
  }

  .menu-wrap {
    position: absolute;
    right: 8px;
    bottom: 14px;
    z-index: 2;
  }
  .menu-btn {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--muted);
    cursor: pointer;
  }
  .menu-btn:hover {
    background: var(--surface-3);
    color: var(--text);
  }
  .menu {
    position: absolute;
    right: 0;
    bottom: 38px;
    min-width: 170px;
    padding: 4px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    box-shadow: var(--shadow);
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 34px;
    padding: 0 10px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--text);
    font: 13px var(--font);
    cursor: pointer;
  }
  .menu button:hover {
    background: var(--surface-3);
  }
  .menu .danger {
    color: var(--danger);
  }
</style>
