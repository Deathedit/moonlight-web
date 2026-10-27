<script lang="ts">
  import { api, ApiError, type App, type Host } from '../lib/api';
  import AppTile from '../lib/components/AppTile.svelte';
  import Icon from '../lib/components/Icon.svelte';
  import Modal from '../lib/components/Modal.svelte';
  import { link, router } from '../lib/router.svelte';

  let { hostId }: { hostId: string } = $props();

  let host = $state<Host | null>(null);
  let apps = $state<App[] | null>(null);
  let error = $state<{ message: string; notPaired: boolean } | null>(null);
  let refreshing = $state(false);

  // Quit / switch confirmations
  let quitting = $state<App | null>(null);
  let quitOpen = $state(false);
  let quitBusy = $state(false);
  let quitError = $state<string | null>(null);
  let switchTo = $state<App | null>(null);

  const running = $derived(apps?.find((a) => a.running) ?? null);

  async function load() {
    refreshing = true;
    try {
      const [h, a] = await Promise.all([api.getHost(hostId), api.listApps(hostId)]);
      host = h;
      apps = a;
      error = null;
    } catch (e) {
      const notPaired = e instanceof ApiError && e.status === 409;
      error = { message: (e as Error).message, notPaired };
      if (e instanceof ApiError && e.status === 404) error.message = 'This host no longer exists.';
    } finally {
      refreshing = false;
    }
  }

  $effect(() => {
    void hostId;
    host = null;
    apps = null;
    load();
  });

  const streamUrl = (app: App) => `/hosts/${encodeURIComponent(hostId)}/stream/${app.id}`;

  function launch(app: App) {
    if (running && running.id !== app.id) {
      // Sunshine runs one app at a time: starting another means quitting this one first.
      switchTo = app;
      openQuit(running);
      return;
    }
    router.go(streamUrl(app));
  }

  function openQuit(app: App) {
    quitting = app;
    quitError = null;
    quitOpen = true;
  }

  async function confirmQuit() {
    quitBusy = true;
    quitError = null;
    try {
      await api.quitApp(hostId);
      quitOpen = false;
      const next = switchTo;
      switchTo = null;
      if (next) router.go(streamUrl(next));
      else await load();
    } catch (e) {
      quitError = (e as Error).message;
    } finally {
      quitBusy = false;
    }
  }

  $effect(() => {
    if (!quitOpen) switchTo = null;
  });
</script>

<section class="page">
  <div class="page-header">
    <div>
      <a href="/" use:link class="back"><Icon name="arrow-left" size={14} /> Hosts</a>
      <h1>{host?.name ?? 'Apps'}</h1>
      {#if host}
        <p>
          {host.address}
          {#if running}· <span class="live">Running {running.title}</span>{/if}
        </p>
      {/if}
    </div>
    <button class="btn" onclick={load} disabled={refreshing}>
      <span class:spin={refreshing}><Icon name="refresh" size={15} /></span> Refresh
    </button>
  </div>

  {#if error}
    <div class="card notice">
      <Icon name={error.notPaired ? 'lock' : 'alert'} size={22} />
      <div>
        <b>{error.notPaired ? 'Not paired' : "Couldn't load apps"}</b>
        <p>{error.notPaired ? 'Pair with this host from the Hosts page first.' : error.message}</p>
      </div>
      <a href="/" use:link class="btn">Back to hosts</a>
    </div>
  {:else if apps === null}
    <div class="grid">
      {#each [0, 1, 2, 3] as i (i)}<div class="skeleton"></div>{/each}
    </div>
  {:else if apps.length === 0}
    <div class="card notice">
      <Icon name="monitor" size={22} />
      <div>
        <b>No apps</b>
        <p>Add applications in Sunshine's web UI under <i>Applications</i>.</p>
      </div>
    </div>
  {:else}
    <div class="grid">
      {#each apps as app (app.id)}
        <AppTile {hostId} {app} onlaunch={launch} onquit={openQuit} />
      {/each}
    </div>
  {/if}
</section>

<Modal bind:open={quitOpen} title={switchTo ? `Switch to ${switchTo.title}?` : `Quit ${quitting?.title}?`}>
  <p class="confirm">
    {#if switchTo}
      <b>{quitting?.title}</b> is running on {host?.name}. It will be closed, and any unsaved progress in it will be
      lost.
    {:else}
      <b>{quitting?.title}</b> will be closed on {host?.name}. Any unsaved progress will be lost.
    {/if}
  </p>
  {#if quitError}<p class="error"><Icon name="alert" size={14} /> {quitError}</p>{/if}
  {#snippet footer()}
    <button class="btn" onclick={() => (quitOpen = false)}>Cancel</button>
    <button class="btn btn-danger" onclick={confirmQuit} disabled={quitBusy}>
      {quitBusy ? 'Quitting…' : switchTo ? 'Quit and switch' : 'Quit app'}
    </button>
  {/snippet}
</Modal>

<style>
  .back {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 8px;
    color: var(--muted);
    font-size: 13px;
  }
  .back:hover {
    color: var(--text);
  }
  .live {
    color: var(--accent);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(168px, 1fr));
    gap: 28px 20px;
  }
  @media (max-width: 600px) {
    .grid {
      grid-template-columns: repeat(2, 1fr);
      gap: 20px 14px;
    }
  }

  .skeleton {
    aspect-ratio: 3 / 4;
    border-radius: var(--radius);
    background: linear-gradient(100deg, var(--surface) 30%, var(--surface-2) 50%, var(--surface) 70%);
    background-size: 300% 100%;
    animation: shimmer 1.4s infinite linear;
  }
  @keyframes shimmer {
    from {
      background-position: 100% 0;
    }
    to {
      background-position: -50% 0;
    }
  }

  .notice {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 20px 24px;
    color: var(--muted);
    flex-wrap: wrap;
  }
  .notice > div {
    flex: 1;
    min-width: 200px;
  }
  .notice b {
    color: var(--text);
  }
  .notice p {
    margin: 2px 0 0;
    font-size: 14px;
  }

  .spin {
    display: inline-grid;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .confirm {
    margin: 0;
    color: var(--muted);
  }
  .confirm b {
    color: var(--text);
  }
  .error {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 12px 0 0;
    color: var(--danger);
    font-size: 13px;
  }
</style>
