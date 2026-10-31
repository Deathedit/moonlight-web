<script lang="ts">
  import type { Host } from '../api';
  import { link } from '../router.svelte';
  import Icon from './Icon.svelte';
  import MoonPhase, { type Phase } from './MoonPhase.svelte';

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
  const phases: Record<string, Phase> = { streaming: 'full', online: 'half', unpaired: 'crescent', offline: 'new' };
  const labels: Record<string, string> = {
    streaming: 'In session',
    online: 'Online',
    unpaired: 'Not paired',
    offline: 'Offline',
  };
  const resumeUrl = $derived(`/hosts/${encodeURIComponent(host.id)}/stream/${host.currentGame}`);
</script>

<svelte:window onclick={() => (menuOpen = false)} />

<li class="row" data-status={status}>
  <MoonPhase phase={phases[status]} size={48} label={labels[status]} />

  <div class="meta">
    {#if host.online}
      <button class="name" onclick={() => onopen(host)} title={host.name}>{host.name}</button>
    {:else}
      <span class="name" title={host.name}>{host.name}</span>
    {/if}
    <div class="sub mono">
      <span>{host.address}</span>
      <span class="state">{status === 'streaming' ? '● in session' : labels[status].toLowerCase()}</span>
    </div>
  </div>

  <div class="actions">
    {#if status === 'streaming'}
      <a class="btn btn-primary" href={resumeUrl} use:link>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M7 4.5v15l12-7.5z" /></svg>
        Resume
      </a>
      <button class="btn" onclick={() => onopen(host)}>Apps</button>
    {:else if status === 'online'}
      <button class="btn strong" onclick={() => onopen(host)}>
        Open <Icon name="arrow-right" size={14} />
      </button>
    {:else if status === 'unpaired'}
      <button class="btn strong" onclick={() => onopen(host)}><Icon name="lock" size={15} /> Pair</button>
    {/if}

    <div class="menu-wrap">
      <button
        class="btn btn-icon"
        aria-label="Host options"
        aria-expanded={menuOpen}
        onclick={(e) => {
          e.stopPropagation();
          menuOpen = !menuOpen;
        }}><Icon name="more" size={20} /></button
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
  </div>
</li>

<style>
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 16px 24px;
    padding: 28px 0;
    border-bottom: 1px solid var(--border);
  }

  .meta {
    flex: 999 1 280px;
    min-width: 0;
  }
  .name {
    display: block;
    max-width: 100%;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font: 600 28px/1.25 var(--font);
    letter-spacing: -0.02em;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  button.name {
    cursor: pointer;
  }
  button.name:hover {
    color: #fff;
  }
  [data-status='offline'] .name {
    color: var(--faint);
  }
  .sub {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 18px;
    margin-top: 6px;
    color: var(--muted);
    font-size: 13px;
  }
  [data-status='streaming'] .state {
    color: var(--accent);
  }
  [data-status='offline'] .sub {
    color: var(--faint);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .strong {
    border-color: var(--border-strong);
  }

  .menu-wrap {
    position: relative;
  }
  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 6px);
    z-index: 5;
    min-width: 180px;
    padding: 4px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.5);
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 38px;
    padding: 0 10px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--text);
    font: 14px var(--font);
    cursor: pointer;
  }
  .menu button:hover {
    background: var(--border);
  }
  .menu .danger {
    color: var(--danger);
  }
</style>
