<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../api';
  import { link, router } from '../router.svelte';

  let bridgeOnline = $state<boolean | null>(null);

  onMount(() => {
    const check = () =>
      api.health().then(
        () => (bridgeOnline = true),
        () => (bridgeOnline = false),
      );
    check();
    const id = setInterval(check, 10_000);
    return () => clearInterval(id);
  });

  const items = [
    { href: '/', label: 'Hosts', section: 'hosts' },
    { href: '/settings', label: 'Settings', section: 'settings' },
  ];

  const section = $derived(router.route.name === 'settings' ? 'settings' : 'hosts');

  const statusText = $derived(
    bridgeOnline === null ? 'connecting…' : bridgeOnline ? 'bridge connected' : 'bridge offline',
  );
</script>

<header class="topbar">
  <div class="left">
    <a href="/" use:link class="brand" title="Moonlight Web">
      <svg width="22" height="22" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M15 3a9 9 0 1 0 6 15.5A7.5 7.5 0 0 1 15 3z" />
      </svg>
      <span class="word">moonlight</span>
      <span class="tag mono">web</span>
    </a>

    <nav aria-label="Main">
      {#each items as item (item.href)}
        <a
          href={item.href}
          use:link
          class:active={section === item.section}
          aria-current={section === item.section ? 'page' : undefined}>{item.label}</a
        >
      {/each}
    </nav>
  </div>

  <div class="status mono" data-state={bridgeOnline === null ? 'pending' : bridgeOnline ? 'ok' : 'down'}>
    <span class="dot"></span>
    <span class="status-text">{statusText}</span>
  </div>
</header>

<style>
  .topbar {
    position: sticky;
    top: 0;
    z-index: 20;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    height: var(--topbar-h);
    padding: 0 32px;
    background: rgba(13, 14, 17, 0.88);
    backdrop-filter: blur(12px);
    border-bottom: 1px solid var(--border);
  }
  .left {
    display: flex;
    align-items: center;
    gap: 40px;
    height: 100%;
    min-width: 0;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--text);
  }
  .brand svg {
    fill: var(--accent);
  }
  .word {
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .tag {
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--faint);
    font-size: 11px;
  }

  nav {
    display: flex;
    gap: 28px;
    height: 100%;
  }
  nav a {
    display: flex;
    align-items: center;
    height: 100%;
    border-bottom: 2px solid transparent;
    color: var(--muted);
    font-size: 14px;
    font-weight: 500;
    transition: color var(--ease);
  }
  nav a:hover {
    color: var(--text);
  }
  nav a.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }

  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--muted);
    font-size: 12px;
    white-space: nowrap;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--dim);
  }
  [data-state='ok'] .dot {
    background: var(--success);
  }
  [data-state='down'] {
    color: var(--danger);
  }
  [data-state='down'] .dot {
    background: var(--danger);
  }

  @media (max-width: 600px) {
    .topbar {
      padding: 0 16px;
    }
    .left {
      gap: 20px;
    }
    nav {
      gap: 18px;
    }
    .tag,
    .status-text {
      display: none;
    }
  }
</style>
