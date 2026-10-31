<script lang="ts">
  import { api, type App } from '../api';

  // Box art from the host, with a monogram while it loads or when there is none.
  // Fills its (positioned) parent.
  let { hostId, app }: { hostId: string; app: App } = $props();

  let failed = $state(false);
  let loaded = $state(false);

  const initials = $derived(
    app.title
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w[0].toUpperCase())
      .join(''),
  );
</script>

{#if failed || !loaded}
  <span class="fallback mono" class:live={app.running}>{initials}</span>
{/if}
{#if !failed}
  <img
    src={api.artUrl(hostId, app.id)}
    alt=""
    loading="lazy"
    class:loaded
    onload={() => (loaded = true)}
    onerror={() => (failed = true)}
  />
{/if}

<style>
  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
    transition: opacity 250ms ease;
  }
  img.loaded {
    opacity: 1;
  }
  .fallback {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--dim);
    font-size: 32px;
    font-weight: 500;
    letter-spacing: 0.04em;
  }
  .fallback.live {
    color: var(--accent);
  }
</style>
