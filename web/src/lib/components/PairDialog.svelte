<script lang="ts">
  import { untrack } from 'svelte';
  import { api, type Host, type PairState } from '../api';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';
  import MoonPhase from './MoonPhase.svelte';

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

<Modal bind:open eyebrow="Pairing" title={host?.name ?? 'host'}>
  {#if startError}
    <p class="error"><Icon name="alert" size={16} /> {startError}</p>
  {:else if !pair}
    <div class="pin skeleton" aria-busy="true"></div>
  {:else if pair.status === 'paired'}
    <div class="done">
      <MoonPhase phase="full" size={56} />
      <p>Paired</p>
    </div>
  {:else}
    <p class="lead">Enter this PIN in Sunshine on the host.</p>
    <div class="pin" class:failed={pair.status === 'failed'} aria-label="PIN {pair.pin.split('').join(' ')}">
      {#each pair.pin.split('') as digit, i (i)}<span class="mono">{digit}</span>{/each}
    </div>

    {#if pair.status === 'failed'}
      <p class="error"><Icon name="alert" size={16} /> {pair.error}</p>
    {:else}
      <a class="sunshine-link" href={sunshineUrl} target="_blank" rel="noreferrer">
        Open Sunshine's PIN page <Icon name="external-link" size={14} />
      </a>
      <div class="waiting">
        <div class="bar"><span></span></div>
        <span class="mono">waiting for PIN…</span>
      </div>
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
    margin: 0 0 20px;
    color: var(--muted);
  }

  .pin {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 10px;
  }
  .pin span {
    display: grid;
    place-items: center;
    height: 96px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--text);
    font-size: 48px;
    font-weight: 500;
  }
  .pin.failed span {
    color: var(--faint);
  }
  .pin.skeleton {
    display: block;
    height: 96px;
  }

  .sunshine-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 20px;
    font-size: 14px;
    font-weight: 500;
  }
  .sunshine-link:hover {
    color: var(--accent-hover);
  }

  .waiting {
    margin-top: 24px;
    color: var(--muted);
    font-size: 12px;
  }
  .bar {
    position: relative;
    height: 2px;
    margin-bottom: 10px;
    overflow: hidden;
    border-radius: 1px;
    background: var(--border);
  }
  .bar span {
    position: absolute;
    inset: 0;
    width: 40%;
    background: var(--accent);
    animation: wax 1.6s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }
  @keyframes wax {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(250%);
    }
  }

  .done {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    padding: 16px 0;
  }
  .done p {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }

  .error {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 16px 0 0;
    color: var(--danger);
    font-size: 14px;
  }
</style>
