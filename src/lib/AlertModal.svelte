<script lang="ts">
  import { alertState, dismissAlert } from './stores/alert.svelte';
</script>

{#if alertState.message}
  <div
    class="alert-overlay"
    onclick={dismissAlert}
    onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ' || e.key === 'Escape') { e.preventDefault(); dismissAlert(); } }}
    role="button"
    tabindex="-1"
  >
    <div class="alert-popup" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
      <pre class="alert-popup-body">{alertState.message}</pre>
      <button class="btn-accent alert-popup-ok" onclick={dismissAlert}>OK</button>
    </div>
  </div>
{/if}

<style>
  .alert-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .alert-popup {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    width: 90%;
    max-width: 420px;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .alert-popup-body {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: inherit;
    font-size: 9pt;
    color: var(--text-main);
    overflow-y: auto;
  }

  .alert-popup-ok {
    align-self: flex-end;
  }
</style>
