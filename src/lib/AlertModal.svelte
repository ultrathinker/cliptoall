<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { alertState, dismissAlert } from './stores/alert.svelte';

  async function handleAction() {
    const url = alertState.action?.url;
    // Clear the modal first so the URL click feels immediate even if
    // `openUrl` rejects (e.g. sandbox blocked an unauthorized scheme).
    dismissAlert();
    if (!url) return;
    try {
      await openUrl(url);
    } catch (e) {
      console.error('Failed to open URL from alert:', e);
    }
  }
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
      <div class="alert-popup-actions">
        {#if alertState.action}
          <button class="btn-default alert-popup-secondary" onclick={handleAction}>{alertState.action.label}</button>
        {/if}
        <button class="btn-accent alert-popup-ok" onclick={dismissAlert}>OK</button>
      </div>
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

  .alert-popup-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .alert-popup-secondary {
    /* Visual cue that the secondary button is the more interesting action;
       keeps the OK button as the default primary position on the right. */
    color: var(--accent, #3b82f6);
  }

  .alert-popup-ok {
    align-self: flex-end;
  }
</style>
