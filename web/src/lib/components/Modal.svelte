<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  let {
    open = $bindable(false),
    title,
    eyebrow,
    children,
    footer,
  }: { open?: boolean; title: string; eyebrow?: string; children: Snippet; footer?: Snippet } = $props();

  let dialog: HTMLDialogElement;

  // Native <dialog> gives us focus trapping, Esc-to-close and a top layer for free.
  $effect(() => {
    if (open && !dialog.open) dialog.showModal();
    else if (!open && dialog.open) dialog.close();
  });
</script>

<dialog
  bind:this={dialog}
  onclose={() => (open = false)}
  onclick={(e) => e.target === dialog && (open = false)}
>
  <header>
    <div>
      {#if eyebrow}<div class="eyebrow">{eyebrow}</div>{/if}
      <h2>{title}</h2>
    </div>
    <button class="btn btn-icon" onclick={() => (open = false)} aria-label="Close"><Icon name="x" size={18} /></button>
  </header>
  <div class="body">{@render children()}</div>
  {#if footer}<footer>{@render footer()}</footer>{/if}
</dialog>

<style>
  dialog {
    width: min(520px, calc(100vw - 32px));
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.6);
  }
  dialog[open] {
    animation: pop 160ms cubic-bezier(0.2, 0, 0, 1);
  }
  dialog::backdrop {
    background: rgba(8, 9, 11, 0.75);
    backdrop-filter: blur(4px);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 24px 20px 0 28px;
  }
  header h2 {
    margin-top: 4px;
    font-size: 24px;
    letter-spacing: -0.02em;
    overflow-wrap: anywhere;
  }
  header .btn-icon {
    margin-top: -4px;
  }
  .body {
    padding: 24px 28px 28px;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    padding: 16px 28px;
    border-top: 1px solid var(--border);
  }
</style>
