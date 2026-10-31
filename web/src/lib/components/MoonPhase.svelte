<script lang="ts" module>
  /** full = in session, half = online, crescent = not paired, new = offline. */
  export type Phase = 'full' | 'half' | 'crescent' | 'new';
</script>

<script lang="ts">
  let { phase, size = 24, label }: { phase: Phase; size?: number; label?: string } = $props();
</script>

<svg
  class="moon {phase}"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  role={label ? 'img' : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : 'true'}
>
  {#if phase === 'full'}
    <circle cx="12" cy="12" r="10" />
  {:else if phase === 'half'}
    <circle class="rim" cx="12" cy="12" r="9.5" />
    <path d="M12 2.5a9.5 9.5 0 0 1 0 19z" />
  {:else if phase === 'crescent'}
    <circle class="rim" cx="12" cy="12" r="9.5" />
    <path d="M12 2.5a9.5 9.5 0 0 1 0 19a5 9.5 0 0 0 0-19z" />
  {:else}
    <circle class="rim dashed" cx="12" cy="12" r="9.5" />
  {/if}
</svg>

<style>
  .moon {
    flex: none;
    fill: var(--accent);
  }
  .rim {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
  }
  .full,
  .half {
    color: var(--accent);
  }
  .crescent {
    color: var(--muted);
    fill: var(--muted);
  }
  .new {
    color: var(--dim);
  }
  .dashed {
    stroke-width: 1.2;
    stroke-dasharray: 2.5 2.5;
  }
</style>
