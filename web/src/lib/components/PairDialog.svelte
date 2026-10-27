<script lang="ts">
  import { untrack } from 'svelte';
  import { api, type Host, type PairState } from '../api';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';

  let {
    host,
    open = $bindable(false),
    onpaired,
  }: { host: Host | null; open?: boolean; onpaired: (host: Host) => void } = $props();

  const POLL_MS = 1000;

  let pair = $state<PairState | null>(null);
  let startError = $state<string | null>(null);
  let attempt = $state(0);

  // Key on the id only: the host list refreshes in the background and replaces the
  // object, which must not restart a pairing that's in progress.
  const hostId = $derived(host?.id);

  // Start pairing whenever the dialog opens for a host; cancel if closed before it finishes.
  $effect(() => {
    void attempt;
    if (!open || !hostId) return;
    const current = untrack(() => host)!;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    pair = null;
    startError = null;

    const poll = async () => {
      if (stopped) return;
      try {
        pair = await api.pairStatus(current.id);
      } catch {
        // transient — keep polling
      }
      if (stopped) return;
      if (pair?.status === 'paired') {
        stopped = true;
        setTimeout(() => {
          open = false;
          onpaired(current);
        }, 900);
      } else if (pair?.status === 'pending') {
        timer = setTimeout(poll, POLL_MS);
      }
    };

    api.startPair(current.id).then(
      (s) => {
        pair = s;
        timer = setTimeout(poll, POLL_MS);
      },
      (e) => (startError = (e as Error).message),
    );

    return () => {
      stopped = true;
      clearTimeout(timer);
      if (pair?.status === 'pending') api.cancelPair(current.id).catch(() => {});
    };
  });

  const sunshineUrl = $derived(host ? `https://${host.address.includes(':') ? `[${host.address}]` : host.address}:47990/pin` : '');

  const retry = () => attempt++;
</script>

<Modal bind:open title="Pair with {host?.name ?? 'host'}">
  {#if startError}
    <p class="error"><Icon name="alert" size={16} /> {startError}</p>
  {:else if !pair}
    <div class="pin skeleton" aria-busy="true"></div>
  {:else if pair.status === 'paired'}
    <div class="done">
      <span class="done-icon"><Icon name="check" size={28} /></span>
      <p>Paired successfully</p>
    </div>
  {:else}
    <p class="lead">Enter this PIN in Sunshine on <b>{host?.name}</b>:</p>
    <div class="pin" class:failed={pair.status === 'failed'} aria-label="PIN {pair.pin.split('').join(' ')}">
      {#each pair.pin.split('') as digit, i (i)}<span>{digit}</span>{/each}
    </div>

    {#if pair.status === 'failed'}
      <p class="error"><Icon name="alert" size={16} /> {pair.error}</p>
    {:else}
      <a class="sunshine-link" href={sunshineUrl} target="_blank" rel="noreferrer">
        Open Sunshine's PIN page <Icon name="external-link" size={14} />
      </a>
      <div class="waiting"><span class="spinner"></span> Waiting for the PIN to be entered…</div>
    {/if}
  {/if}

  {#snippet footer()}
    <button class="btn" onclick={() => (open = false)}>
      {pair?.status === 'paired' ? 'Close' : 'Cancel'}
    </button>
    {#if pair?.status === 'failed' || startError}
      <button class="btn btn-primary" onclick={retry}><Icon name="refresh" size={15} /> Try again</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .lead {
    margin: 0 0 16px;
    color: var(--muted);
  }
  .lead b {
    color: var(--text);
  }

  .pin {
    display: flex;
    justify-content: center;
    gap: 10px;
    margin-bottom: 20px;
  }
  .pin span {
    display: grid;
    place-items: center;
    width: 64px;
    height: 76px;
    border-radius: var(--radius);
    background: var(--bg);
    border: 1px solid var(--border-strong);
    box-shadow:
      inset 0 0 0 1px rgba(79, 140, 255, 0.15),
      0 0 24px rgba(79, 140, 255, 0.12);
    color: var(--text);
    font-size: 36px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .pin.failed span {
    color: var(--muted);
    box-shadow: none;
  }
  .pin.skeleton {
    height: 76px;
    border-radius: var(--radius);
    background: var(--surface-2);
  }

  .sunshine-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 14px;
  }
  .sunshine-link:hover {
    color: var(--accent-hover);
  }

  .waiting {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 14px;
    color: var(--muted);
    font-size: 14px;
  }
  .spinner {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .done {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 12px 0;
  }
  .done p {
    margin: 0;
    font-weight: 600;
  }
  .done-icon {
    display: grid;
    place-items: center;
    width: 60px;
    height: 60px;
    border-radius: 50%;
    background: rgba(62, 207, 142, 0.12);
    color: var(--success);
    box-shadow: 0 0 24px rgba(62, 207, 142, 0.25);
  }

  .error {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 0;
    color: var(--danger);
    font-size: 14px;
  }
</style>
