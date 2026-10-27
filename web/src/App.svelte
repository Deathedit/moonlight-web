<script lang="ts">
  import Icon from './lib/components/Icon.svelte';
  import Sidebar from './lib/components/Sidebar.svelte';
  import { router } from './lib/router.svelte';
  import Hosts from './routes/Hosts.svelte';
  import Apps from './routes/Apps.svelte';
  import Stream from './routes/Stream.svelte';
  import Settings from './routes/Settings.svelte';
  import NotFound from './routes/NotFound.svelte';

  const route = $derived(router.route);

  // Sidebar is folded by default; only an explicit "expanded" choice is stored, so the
  // default can change later without being pinned by a value written on first load.
  const FOLD_KEY = 'mlw.sidebar';
  let collapsed = $state(readCollapsed());
  let mobileOpen = $state(false);

  function readCollapsed() {
    try {
      return localStorage.getItem(FOLD_KEY) !== 'expanded';
    } catch {
      return true;
    }
  }

  $effect(() => {
    try {
      if (collapsed) localStorage.removeItem(FOLD_KEY);
      else localStorage.setItem(FOLD_KEY, 'expanded');
    } catch {
      // storage unavailable — just don't persist
    }
  });
</script>

{#if route.name === 'stream'}
  <!-- Stream view is fullscreen, no chrome -->
  <Stream hostId={route.hostId} appId={route.appId} />
{:else}
  <div class="shell">
    <Sidebar bind:collapsed bind:mobileOpen />

    <div class="content">
      <header class="mobile-bar">
        <button class="menu-btn" onclick={() => (mobileOpen = true)} aria-label="Open menu">
          <Icon name="menu" size={20} />
        </button>
        <span class="mobile-title">Moonlight <b>Web</b></span>
      </header>

      <main>
        {#if route.name === 'hosts'}
          <Hosts />
        {:else if route.name === 'apps'}
          <Apps hostId={route.hostId} />
        {:else if route.name === 'settings'}
          <Settings />
        {:else}
          <NotFound />
        {/if}
      </main>
    </div>
  </div>
{/if}

<style>
  .shell {
    display: flex;
    min-height: 100vh;
    min-height: 100dvh;
  }
  .content {
    flex: 1;
    min-width: 0;
  }

  .mobile-bar {
    display: none;
  }

  @media (max-width: 720px) {
    .mobile-bar {
      position: sticky;
      top: 0;
      z-index: 10;
      display: flex;
      align-items: center;
      gap: 8px;
      height: 56px;
      padding: 0 8px;
      background: rgba(11, 18, 32, 0.8);
      backdrop-filter: blur(14px);
      border-bottom: 1px solid var(--border);
    }
    .menu-btn {
      display: grid;
      place-items: center;
      width: 40px;
      height: 40px;
      border: 0;
      border-radius: var(--radius-sm);
      background: transparent;
      color: var(--text);
      cursor: pointer;
    }
    .menu-btn:hover {
      background: var(--surface-2);
    }
    .mobile-title {
      font-weight: 500;
    }
    .mobile-title b {
      color: var(--accent);
    }
  }
</style>
