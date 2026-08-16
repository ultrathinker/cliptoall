<script lang="ts">
  import { openSystemSettings } from './api';
  import { alertState, dismissAlert } from './stores/alert.svelte';

  // Escape closes, from anywhere. The handler on the backdrop below only fires
  // when the backdrop itself has focus, and the popup stops key events from
  // propagating to it — so with a single action button and no OK, that alone
  // would leave no keyboard way out.
  $effect(() => {
    if (!alertState.message) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); dismissAlert(); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  async function handleAction() {
    const url = alertState.action?.url;
    // Routed through Rust (NSWorkspace), not tauri-plugin-opener: the main
    // window's opener capability allows only http/https, and the plugin opens
    // URLs by spawning `open`, which the sandbox forbids. See
    // `open_system_settings` in main.rs.
    dismissAlert();
    if (!url) return;
    try {
      await openSystemSettings(url);
    } catch (e) {
      console.error('Failed to open System Settings from alert:', e);
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
      <!-- One button. When the alert carries an action there is exactly one
           sensible thing to do, and an extra OK next to it only asks the user
           to decide which button is the real one. Dismissing without acting is
           still possible: click outside, or press Escape. -->
      <div class="alert-popup-actions">
        {#if alertState.action}
          <button class="btn-accent alert-popup-ok" onclick={handleAction}>{alertState.action.label}</button>
        {:else}
          <button class="btn-accent alert-popup-ok" onclick={dismissAlert}>OK</button>
        {/if}
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

  .alert-popup-ok {
    align-self: flex-end;
  }
</style>
