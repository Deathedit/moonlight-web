<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type DiscoveredHost, type Host } from '../lib/api';
  import HostCard from '../lib/components/HostCard.svelte';
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
      <h1>Hosts</h1>
      <p>Computers running Sunshine that you can stream from.</p>
    </div>
    <button class="btn btn-primary" onclick={openAdd}><Icon name="plus" size={16} /> Add host</button>
  </div>

  {#if loadError}
    <div class="banner"><Icon name="alert" size={16} /> {loadError}</div>
  {/if}

  {#if newFound.length > 0}
    <div class="found">
      <h2>Found on your network</h2>
      <div class="found-list">
        {#each newFound as d (d.address)}
          <div class="found-item card">
            <span class="found-icon"><Icon name="monitor" size={18} /></span>
            <div class="found-meta">
              <b>{d.name}</b>
              <span>{d.address}</span>
            </div>
            <button class="btn btn-primary" onclick={() => addFound(d)} disabled={addingFound !== null}>
              {addingFound === d.address ? 'Adding…' : 'Add & pair'}
            </button>
          </div>
        {/each}
      </div>
    </div>
  {/if}

  {#if hosts === null}
    <div class="grid">
      {#each [0, 1, 2] as i (i)}<div class="card skeleton"></div>{/each}
    </div>
  {:else if hosts.length === 0}
    <div class="empty card">
      <div class="empty-icon"><Icon name="monitor" size={28} /></div>
      <h2>No hosts yet</h2>
      <p>
        Add a PC running <a href="https://github.com/LizardByte/Sunshine" target="_blank" rel="noreferrer">Sunshine</a>
        by its IP address or hostname.
      </p>
      <button class="btn btn-primary" onclick={openAdd}><Icon name="plus" size={16} /> Add your first host</button>
    </div>
  {:else}
    <div class="grid">
      {#each hosts as host (host.id)}
        <HostCard {host} onopen={openHost} onremove={askRemove} onunpair={askUnpair} />
      {/each}
    </div>
  {/if}

  {#if discovered !== null && discovered.length === 0}
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
      class="input"
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
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 20px;
  }

  .skeleton {
    aspect-ratio: 16 / 13;
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

  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 20px;
    padding: 10px 14px;
    border-radius: var(--radius-sm);
    border: 1px solid rgba(255, 92, 122, 0.35);
    background: rgba(255, 92, 122, 0.08);
    color: var(--danger);
    font-size: 14px;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    padding: 56px 24px;
    border-style: dashed;
  }
  .empty-icon {
    display: grid;
    place-items: center;
    width: 64px;
    height: 64px;
    margin-bottom: 16px;
    border-radius: 18px;
    background: var(--accent-soft);
    color: var(--accent);
    box-shadow: inset 0 0 0 1px rgba(79, 140, 255, 0.25);
  }
  .empty h2 {
    font-size: 18px;
  }
  .empty p {
    max-width: 440px;
    margin: 8px 0 20px;
    color: var(--muted);
  }

  .found {
    margin-bottom: 28px;
  }
  .found h2 {
    margin-bottom: 12px;
    color: var(--muted);
    font-size: 13px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .found-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 12px;
  }
  .found-item {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px 14px;
    border-color: rgba(79, 140, 255, 0.35);
    animation: found-in 250ms ease;
  }
  @keyframes found-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
  .found-icon {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 12px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .found-meta {
    flex: 1;
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
    color: var(--faint);
    font-size: 13px;
  }
  .discover-hint {
    margin: 28px 0 0;
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
