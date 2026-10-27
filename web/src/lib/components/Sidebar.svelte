<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../api';
  import { link, router } from '../router.svelte';
  import Icon, { type IconName } from './Icon.svelte';

  let {
    collapsed = $bindable(false),
    mobileOpen = $bindable(false),
  }: { collapsed?: boolean; mobileOpen?: boolean } = $props();

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

  const items: { href: string; label: string; icon: IconName; section: string }[] = [
    { href: '/', label: 'Hosts', icon: 'monitor', section: 'hosts' },
    { href: '/settings', label: 'Settings', icon: 'settings', section: 'settings' },
  ];

  const section = $derived(router.route.name === 'settings' ? 'settings' : 'hosts');

  // Close the mobile drawer after navigating.
  $effect(() => {
    void router.path;
    mobileOpen = false;
  });

  const statusText = $derived(
    bridgeOnline === null ? 'Connecting…' : bridgeOnline ? 'Bridge connected' : 'Bridge offline',
  );
</script>

<aside class="sidebar" class:collapsed class:mobile-open={mobileOpen} aria-label="Main navigation">
  <div class="brand-row">
    <a href="/" use:link class="brand" title="Moonlight Web">
      <span class="logo"><Icon name="moon" size={18} /></span>
      <span class="label">Moonlight <b>Web</b></span>
    </a>
  </div>

  <nav>
    {#each items as item (item.href)}
      <a
        href={item.href}
        use:link
        class:active={section === item.section}
        aria-current={section === item.section ? 'page' : undefined}
        title={collapsed ? item.label : undefined}
      >
        <Icon name={item.icon} size={18} />
        <span class="label">{item.label}</span>
      </a>
    {/each}
  </nav>

  <div class="bottom">
    <div
      class="status"
      title={statusText}
      data-state={bridgeOnline === null ? 'pending' : bridgeOnline ? 'ok' : 'down'}
    >
      <span class="dot"></span>
      <span class="label">{statusText}</span>
    </div>

    <button
      class="fold"
      onclick={() => (collapsed = !collapsed)}
      aria-expanded={!collapsed}
      title={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
    >
      <span class="fold-icon"><Icon name="chevrons-left" size={16} /></span>
      <span class="label">Collapse</span>
    </button>
  </div>
</aside>

<!-- Scrim behind the drawer on small screens -->
{#if mobileOpen}
  <button class="scrim" aria-label="Close menu" onclick={() => (mobileOpen = false)}></button>
{/if}

<style>
  .sidebar {
    position: sticky;
    top: 0;
    z-index: 20;
    display: flex;
    flex-direction: column;
    width: var(--sidebar-w);
    height: 100vh;
    height: 100dvh;
    padding: 14px 12px;
    background: rgba(13, 21, 38, 0.82);
    backdrop-filter: blur(14px) saturate(140%);
    border-right: 1px solid var(--border);
    transition: width 200ms cubic-bezier(0.2, 0, 0, 1);
    overflow: hidden;
  }
  .sidebar.collapsed {
    width: var(--sidebar-w-collapsed);
  }

  /* Labels fade/clip rather than reflow while the width animates. */
  .label {
    white-space: nowrap;
    opacity: 1;
    transition: opacity 150ms ease;
  }
  .collapsed .label {
    opacity: 0;
    pointer-events: none;
  }

  .brand-row {
    padding: 2px 4px 18px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 12px;
    color: var(--text);
    font-weight: 500;
  }
  .brand b {
    color: var(--accent);
    font-weight: 700;
  }
  .logo {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 11px;
    background: linear-gradient(135deg, var(--accent), #2d5fd6);
    color: #fff;
    box-shadow: 0 4px 14px var(--accent-glow);
  }

  nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  nav a {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    height: 40px;
    padding: 0 11px;
    border-radius: var(--radius-sm);
    color: var(--muted);
    font-size: 14px;
    font-weight: 500;
    transition: color var(--ease), background var(--ease);
  }
  nav a :global(svg) {
    flex: none;
  }
  nav a:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  nav a.active {
    color: var(--text);
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px rgba(79, 140, 255, 0.22);
  }
  nav a.active::before {
    content: '';
    position: absolute;
    left: -12px;
    top: 9px;
    bottom: 9px;
    width: 3px;
    border-radius: 0 3px 3px 0;
    background: var(--accent);
    box-shadow: 0 0 10px var(--accent-glow);
  }
  nav a.active :global(svg) {
    color: var(--accent);
  }

  .bottom {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }

  .status {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 36px;
    padding: 0 15px;
    color: var(--muted);
    font-size: 13px;
  }
  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--faint);
  }
  [data-state='ok'] .dot {
    background: var(--success);
    box-shadow: 0 0 8px var(--success);
  }
  [data-state='down'] .dot {
    background: var(--danger);
    box-shadow: 0 0 8px var(--danger);
  }
  [data-state='down'] {
    color: var(--danger);
  }

  .fold {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 38px;
    padding: 0 11px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--muted);
    font: 500 13px var(--font);
    cursor: pointer;
    transition: color var(--ease), background var(--ease);
  }
  .fold:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  .fold-icon {
    flex: none;
    display: grid;
    place-items: center;
    width: 18px;
    transition: transform 200ms cubic-bezier(0.2, 0, 0, 1);
  }
  .collapsed .fold-icon {
    transform: rotate(180deg);
  }

  .scrim {
    display: none;
  }

  /* Small screens: off-canvas drawer, always full width when open. */
  @media (max-width: 720px) {
    .sidebar,
    .sidebar.collapsed {
      position: fixed;
      left: 0;
      width: min(272px, 82vw);
      transform: translateX(-100%);
      transition: transform 220ms cubic-bezier(0.2, 0, 0, 1);
      box-shadow: 0 0 48px rgba(0, 0, 0, 0.5);
    }
    .sidebar.mobile-open {
      transform: none;
    }
    .collapsed .label {
      opacity: 1;
      pointer-events: auto;
    }
    .fold {
      display: none;
    }
    .scrim {
      display: block;
      position: fixed;
      inset: 0;
      z-index: 15;
      border: 0;
      background: rgba(5, 9, 18, 0.6);
      backdrop-filter: blur(2px);
    }
  }
</style>
