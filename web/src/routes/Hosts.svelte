<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type DiscoveredHost, type Host } from '../lib/api';
  import HostRow from '../lib/components/HostRow.svelte';
  import MoonPhase from '../lib/components/MoonPhase.svelte';
  import Icon from '../lib/components/Icon.svelte';
  import Modal from '../lib/components/Modal.svelte';
  import PairDialog from '../lib/components/PairDialog.svelte';
  import { router } from '../lib/router.svelte';

  const POLL_MS = 5000;

  let hosts = $state<Host[] | null>(null);
  let loadError = $state<string | null>(null);
  let discovered = $state<DiscoveredHost[] | null>(null);
  let addingFound = $state<string | null>(null);

  const newFound = $derived((discovered ?? []).filter((d) => !d.knownId));

  let addOpen = $state(false);
  let address = $state('');
  let adding = $state(false);
  let addError = $state<string | null>(null);

  let removing = $state<Host | null>(null);
  let removeOpen = $state(false);

  async function refresh() {
    try {
      hosts = await api.listHosts();
      loadError = null;
    } catch (e) {
      loadError = (e as Error).message;
    }
    // Discovery is best-effort; failures just mean nothing extra to show.
    api.discover().then(
      (d) => (discovered = d),
      () => (discovered = []),
    );
  }

  async function addFound(d: DiscoveredHost) {
    addingFound = d.address;
    try {
      const host = await api.addHost(`${d.address}:${d.port}`);
      hosts = [...(hosts ?? []).filter((h) => h.id !== host.id), host];
      discovered = (discovered ?? []).map((x) => (x.address === d.address ? { ...x, knownId: host.id } : x));
      openHost(host); // straight into pairing
    } catch (e) {
      loadError = (e as Error).message;
    } finally {
      addingFound = null;
    }
  }

  onMount(() => {
    refresh();
    // Poll for online/busy changes, but only while the tab is visible.
    const id = setInterval(() => document.visibilityState === 'visible' && refresh(), POLL_MS);
    return () => clearInterval(id);
  });

  function openAdd() {
    address = '';
    addError = null;
    addOpen = true;
  }

  async function submitAdd(e: SubmitEvent) {
    e.preventDefault();
    adding = true;
    addError = null;
    try {
      const host = await api.addHost(address);
      hosts = [...(hosts ?? []).filter((h) => h.id !== host.id), host];
      addOpen = false;
    } catch (err) {
      addError = (err as Error).message;
    } finally {
      adding = false;
    }
  }

  let pairing = $state<Host | null>(null);
  let pairOpen = $state(false);

  const goToApps = (host: Host) => router.go(`/hosts/${encodeURIComponent(host.id)}/apps`);

  function openHost(host: Host) {
    if (host.paired) {
      goToApps(host);
    } else {
      pairing = host;
      pairOpen = true;
    }
  }

  let unpairing = $state<Host | null>(null);
  let unpairOpen = $state(false);

  function askUnpair(host: Host) {
    unpairing = host;
    unpairOpen = true;
  }

  async function confirmUnpair() {
    if (!unpairing) return;
    const id = unpairing.id;
    unpairOpen = false;
    try {
      await api.unpair(id);
    } catch (e) {
      loadError = (e as Error).message;
    }
    await refresh();
  }

  function askRemove(host: Host) {
    removing = host;
    removeOpen = true;
  }

  async function confirmRemove() {
    if (!removing) return;
    const id = removing.id;
    removeOpen = false;
    await api.deleteHost(id).catch((e) => (loadError = (e as Error).message));
    hosts = (hosts ?? []).filter((h) => h.id !== id);
  }
</script>

<section class="page">
  <div class="page-header">
    <div>
      <div class="eyebrow">01 — Hosts</div>
      <h1>Your machines</h1>
      <p>Computers running Sunshine that this bridge can stream from.</p>
    </div>
    <button class="btn btn-primary" onclick={openAdd}><Icon name="plus" size={16} /> Add host</button>
  </div>

  {#if loadError}
    <div class="notice-banner"><Icon name="alert" size={16} /> {loadError}</div>
  {/if}

  {#if hosts === null}
    <div class="list-skeleton">
      {#each [0, 1, 2] as i (i)}<div class="skeleton"></div>{/each}
    </div>
  {:else if hosts.length === 0}
    <div class="empty">
      <MoonPhase phase="new" size={56} />
      <h2>No hosts yet</h2>
      <p>
        Add a PC running <a href="https://github.com/LizardByte/Sunshine" target="_blank" rel="noreferrer">Sunshine</a>
        by its IP address or hostname{newFound.length > 0 ? ', or pick one found on your network below' : ''}.
      </p>
      <button class="btn btn-primary" onclick={openAdd}><Icon name="plus" size={16} /> Add your first host</button>
    </div>
  {:else}
    <div class="legend eyebrow" aria-hidden="true">
      <span><MoonPhase phase="full" size={12} />In session</span>
      <span><MoonPhase phase="half" size={12} />Online</span>
      <span><MoonPhase phase="crescent" size={12} />Not paired</span>
      <span><MoonPhase phase="new" size={12} />Offline</span>
    </div>
    <ul class="list">
      {#each hosts as host (host.id)}
        <HostRow {host} onopen={openHost} onremove={askRemove} onunpair={askUnpair} />
      {/each}
    </ul>
  {/if}

  {#if newFound.length > 0}
    <section class="found">
      <h2 class="eyebrow">Found on your network · mDNS</h2>
      <div class="found-list">
        {#each newFound as d (d.address)}
          <div class="found-item">
            <svg class="found-icon" width="28" height="28" viewBox="0 0 24 24" aria-hidden="true">
              <circle cx="12" cy="12" r="9.5" /><circle class="core" cx="12" cy="12" r="2.5" />
            </svg>
            <div class="found-meta">
              <b>{d.name}</b>
              <span class="mono">{d.address}</span>
            </div>
            <button class="btn strong" onclick={() => addFound(d)} disabled={addingFound !== null}>
              {addingFound === d.address ? 'Adding…' : 'Add & pair'}
            </button>
          </div>
        {/each}
      </div>
    </section>
  {:else if discovered !== null && discovered.length === 0}
    <p class="discover-hint">
      PCs running Sunshine are found automatically via mDNS. If yours doesn't show up, allow mDNS (UDP port 5353)
      through the firewall on the machine running this bridge, or add it by address.
    </p>
  {/if}
</section>

<Modal bind:open={addOpen} title="Add host">
  <form id="add-host" onsubmit={submitAdd}>
    <label class="label" for="address">IP address or hostname</label>
    <!-- svelte-ignore a11y_autofocus -->
    <input
      id="address"
      class="input mono"
      placeholder="192.168.1.85"
      bind:value={address}
      autocomplete="off"
      spellcheck="false"
      autofocus
      required
    />
    <p class="hint">Add <code>:port</code> if Sunshine doesn't use the default port 47989.</p>
    {#if addError}<p class="error"><Icon name="alert" size={14} /> {addError}</p>{/if}
  </form>
  {#snippet footer()}
    <button class="btn" onclick={() => (addOpen = false)}>Cancel</button>
    <button class="btn btn-primary" type="submit" form="add-host" disabled={adding || !address.trim()}>
      {adding ? 'Connecting…' : 'Add host'}
    </button>
  {/snippet}
</Modal>

<PairDialog host={pairing} bind:open={pairOpen} onpaired={goToApps} />

<Modal bind:open={unpairOpen} title="Unpair host?">
  <p class="confirm">
    <b>{unpairing?.name}</b> will forget this bridge. You can pair again any time with a new PIN.
  </p>
  {#snippet footer()}
    <button class="btn" onclick={() => (unpairOpen = false)}>Cancel</button>
    <button class="btn btn-danger" onclick={confirmUnpair}><Icon name="lock" size={15} /> Unpair</button>
  {/snippet}
</Modal>

<Modal bind:open={removeOpen} title="Remove host?">
  <p class="confirm">
    <b>{removing?.name}</b> will be removed from this bridge and unpaired on the host. You'll need to pair again
    to stream from it.
  </p>
  {#snippet footer()}
    <button class="btn" onclick={() => (removeOpen = false)}>Cancel</button>
    <button class="btn btn-danger" onclick={confirmRemove}><Icon name="trash" size={15} /> Remove</button>
  {/snippet}
</Modal>

<style>
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 20px;
    padding-bottom: 14px;
    border-bottom: 1px solid var(--border);
    font-size: 11px;
    letter-spacing: 0.06em;
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .list {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .list-skeleton {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .list-skeleton .skeleton {
    height: 104px;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    padding: 64px 24px;
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-lg);
  }
  .empty h2 {
    margin-top: 20px;
    font-size: 22px;
  }
  .empty p {
    max-width: 440px;
    margin: 8px 0 24px;
    color: var(--muted);
  }

  .found {
    margin-top: 56px;
  }
  .found h2 {
    margin-bottom: 16px;
  }
  .found-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .found-item {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 16px;
    padding: 18px 20px;
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius);
    animation: found-in 250ms ease;
  }
  @keyframes found-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
  .found-icon {
    flex: none;
    fill: none;
    stroke: var(--muted);
    stroke-width: 1.5;
    stroke-dasharray: 2.5 2.5;
  }
  .found-icon .core {
    fill: var(--muted);
    stroke: none;
  }
  .found-meta {
    flex: 999 1 200px;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .found-meta b {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .found-meta span {
    color: var(--muted);
    font-size: 13px;
  }
  .strong {
    border-color: var(--border-strong);
  }
  .discover-hint {
    max-width: 640px;
    margin: 40px 0 0;
    color: var(--faint);
    font-size: 13px;
  }

  .hint {
    margin: 8px 0 0;
    color: var(--faint);
    font-size: 13px;
  }
  .hint code {
    color: var(--muted);
  }
  .error {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 12px 0 0;
    color: var(--danger);
    font-size: 13px;
  }
  .confirm {
    margin: 0;
    color: var(--muted);
  }
  .confirm b {
    color: var(--text);
  }
</style>
