<script lang="ts">
  import TopBar from './lib/components/TopBar.svelte';
  import { router } from './lib/router.svelte';
  import Hosts from './routes/Hosts.svelte';
  import Apps from './routes/Apps.svelte';
  import Stream from './routes/Stream.svelte';
  import Settings from './routes/Settings.svelte';
  import NotFound from './routes/NotFound.svelte';

  const route = $derived(router.route);
</script>

{#if route.name === 'stream'}
  <!-- Stream view is fullscreen, no chrome -->
  <Stream hostId={route.hostId} appId={route.appId} />
{:else}
  <TopBar />
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
{/if}
