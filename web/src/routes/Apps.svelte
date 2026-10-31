<script lang="ts">
  import { api, ApiError, type App, type Host } from '../lib/api';
  import AppArt from '../lib/components/AppArt.svelte';
  import AppTile from '../lib/components/AppTile.svelte';
  import Icon from '../lib/components/Icon.svelte';
  import MoonPhase from '../lib/components/MoonPhase.svelte';
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
  <a href="/" use:link class="back eyebrow"><Icon name="arrow-left" size={13} /> Hosts</a>

  <div class="page-header">
    <div class="title">
      {#if host}<MoonPhase phase={running ? 'full' : 'half'} size={56} label={running ? 'In session' : 'Online'} />{/if}
      <div>
        <h1>{host?.name ?? 'Apps'}</h1>
        {#if host}<div class="addr mono">{host.address} · {host.paired ? 'paired' : 'not paired'}</div>{/if}
      </div>
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
      {#each [0, 1, 2, 3, 4, 5] as i (i)}<div class="skeleton poster-skel"></div>{/each}
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
    {#if running}
      <section class="now card" aria-label="Now running">
        <div class="now-art">
          <AppArt {hostId} app={running} />
        </div>
        <div class="now-body">
          <div>
            <div class="eyebrow live"><MoonPhase phase="full" size={10} /> Now running</div>
            <h2>{running.title}</h2>
            <p>Pick up where you left off. Your current stream settings apply on resume.</p>
          </div>
          <div class="now-actions">
            <button class="btn btn-primary" onclick={() => launch(running)}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M7 4.5v15l12-7.5z" /></svg>
              Resume
            </button>
            <button class="btn btn-danger" onclick={() => openQuit(running)}><Icon name="x" size={14} /> Quit app</button>
          </div>
        </div>
      </section>
    {/if}

    <div class="lib-head">
      <h2 class="eyebrow">02 — Library</h2>
      <span class="eyebrow count">{apps.length} {apps.length === 1 ? 'app' : 'apps'}</span>
    </div>
    <ul class="grid">
      {#each apps as app (app.id)}
        <AppTile {hostId} {app} onlaunch={launch} />
      {/each}
    </ul>
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
    margin-bottom: 16px;
    color: var(--muted);
  }
  .back:hover {
    color: var(--text);
  }
  .title {
    display: flex;
    align-items: center;
    gap: 20px;
    min-width: 0;
  }
  .title h1 {
    margin-top: 0;
    overflow-wrap: anywhere;
  }
  .addr {
    margin-top: 10px;
    color: var(--muted);
    font-size: 13px;
  }

  .now {
    display: flex;
    flex-wrap: wrap;
    overflow: hidden;
    margin-bottom: 56px;
  }
  .now-art {
    position: relative;
    flex: 0 0 210px;
    aspect-ratio: 3 / 4;
    background: var(--surface-2);
    border-right: 1px solid var(--border);
  }
  .now-body {
    flex: 999 1 380px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 24px;
    padding: 32px;
  }
  .live {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: var(--accent);
  }
  .now h2 {
    margin-top: 10px;
    font-size: 36px;
    line-height: 1.1;
    letter-spacing: -0.025em;
  }
  .now p {
    margin: 8px 0 0;
    color: var(--muted);
  }
  .now-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }

  .lib-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    padding-bottom: 14px;
    margin-bottom: 24px;
    border-bottom: 1px solid var(--border);
  }
  .count {
    letter-spacing: 0.02em;
    text-transform: none;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 28px 20px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  @media (max-width: 600px) {
    .grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 20px 14px;
    }
    /* The library below shows the same poster; skip the cropped copy on phones. */
    .now-art {
      display: none;
    }
    .now-body {
      padding: 24px;
    }
  }
  .poster-skel {
    aspect-ratio: 3 / 4;
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
