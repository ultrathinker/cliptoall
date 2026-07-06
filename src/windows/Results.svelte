<script lang="ts">
  import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { saveResultsWindowSize, readImageBase64, copyImageToClipboard } from '../lib/api';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import { settings } from '../lib/stores/settings';
  import { session, startUpload, copyLink, currentImagePath } from '../lib/stores/session.svelte';
  import { onMount, onDestroy } from 'svelte';

  let { onEdit }: { onEdit?: () => void } = $props();

  const AUTOCLOSE_MS = 30000;
  let autoCloseSeconds = $state(30);
  let autoCloseEnabled = $state(false);
  let autoCloseRunning = $state(false); // reactive: is the countdown actually ticking?
  let deadline = 0;
  let intervalId: number | undefined;
  let previewDataUrl = $state('');
  let mounted = true;
  let resizeSaveTimer: number | undefined;
  let unlistenResize: (() => void) | undefined;
  let showErrorPopup = $state(false);
  let copyLinkLabel = $state('Copy link');
  let copyImageLabel = $state('Copy image');

  // ── Derived view state (single source of truth = session store) ──────
  let uploading = $derived(session.status === 'uploading');
  let hasUrl = $derived(!!session.url);
  // The link is directly usable only when it matches the current image.
  let linkUsable = $derived(session.status === 'done' && !session.stale);
  let uploadError = $derived(session.status === 'error' ? session.error : '');

  const ERROR_MAX = 150;
  let shortError = $derived(uploadError.length > ERROR_MAX ? uploadError.slice(0, ERROR_MAX) + '...' : uploadError);

  // Primary action button (label + whether it re-uploads or just copies).
  let primaryLabel = $derived(
    session.status === 'uploading' ? 'Uploading…'
    : linkUsable ? copyLinkLabel
    : session.status === 'error' ? 'Retry'
    : session.stale ? 'Update link'
    : 'Upload'
  );
  let primaryDisabled = $derived(session.status === 'uploading');

  // Start the countdown as soon as the upload reaches a terminal, ready state —
  // this covers the common case where the upload finishes AFTER mount (the
  // upload runs asynchronously in the session store), which onMount alone misses.
  // Start the countdown once the upload reaches a terminal, ready state. This
  // also fires on the FRESH Results view shown after returning from the editor
  // (the previous view's timer was stopped when Edit was pressed) — the user
  // wants autoclose to restart from 30 there, so there is deliberately no
  // "was in editor" suppression here.
  $effect(() => {
    if (autoCloseEnabled && intervalId === undefined
        && (linkUsable || session.status === 'skipped')) {
      startAutoclose();
    }
  });

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && $settings.escapeHidesResults) {
      // Minimize (not close): an upload may still be in flight and its state
      // lives in this window's session store — closing would discard it.
      minimizeWindow();
    }
  }

  onMount(async () => {
    autoCloseEnabled = $settings.autoclose;
    window.addEventListener('keydown', handleKeydown);

    const win = getCurrentWindow();
    await win.setMinSize(new LogicalSize(600, 200));

    // Save window size on resize (debounced), skip if minimized
    unlistenResize = await win.onResized(async ({ payload: size }) => {
      if (size.width === 0 || size.height === 0) return;
      if (resizeSaveTimer) clearTimeout(resizeSaveTimer);
      resizeSaveTimer = setTimeout(async () => {
        const isMin = await win.isMinimized();
        if (isMin) return;
        const factor = await win.scaleFactor();
        const logicalW = size.width / factor;
        const logicalH = size.height / factor;
        if (logicalW < 300 || logicalH < 100) return;
        await saveResultsWindowSize(logicalW, logicalH);
      }, 500);
    });

    // Load preview thumbnail of the current (possibly edited) image.
    const path = currentImagePath();
    if (path) {
      try {
        const base64 = await readImageBase64(path);
        const mime = path.toLowerCase().endsWith('.png') ? 'image/png' : 'image/jpeg';
        if (mounted) previewDataUrl = `data:${mime};base64,${base64}`;
      } catch (_) {}
    }

    // The $effect above starts the countdown once the upload reaches a terminal,
    // ready state (link current, or image copied). Nothing to do here.
  });

  onDestroy(() => {
    mounted = false;
    if (intervalId) clearInterval(intervalId);
    if (resizeSaveTimer) clearTimeout(resizeSaveTimer);
    if (unlistenResize) unlistenResize();
    window.removeEventListener('keydown', handleKeydown);
  });

  async function closeWindow() {
    // Autoclose destroys the window (it is single-use) so its WebView is freed
    // instead of leaking as a hidden window (BUGS#5).
    await getCurrentWindow().close();
  }

  async function minimizeWindow() {
    await getCurrentWindow().minimize();
  }

  function stopAutoclose() {
    if (intervalId) { clearInterval(intervalId); intervalId = undefined; }
    autoCloseRunning = false;
  }

  /** Deadline-based, idempotent. Survives WebView timer throttling because it
   *  compares wall-clock time rather than counting ticks (BUGS#2a-2c). */
  function startAutoclose() {
    if (!autoCloseEnabled) return;
    stopAutoclose();
    deadline = Date.now() + AUTOCLOSE_MS;
    autoCloseSeconds = 30;
    autoCloseRunning = true;
    intervalId = setInterval(() => {
      const remaining = Math.max(0, Math.ceil((deadline - Date.now()) / 1000));
      autoCloseSeconds = remaining;
      if (Date.now() >= deadline) {
        stopAutoclose();
        closeWindow();
      }
    }, 250);
  }

  async function primaryAction() {
    if (linkUsable) {
      // Just copy the existing, current link.
      const ok = await copyLink();
      copyLinkLabel = ok ? 'Copied' : 'Copy failed';
      setTimeout(() => { copyLinkLabel = 'Copy link'; }, 1000);
    } else {
      // skipped / stale / error → (re)upload and copy the link.
      // The $effect restarts the countdown once the upload completes.
      stopAutoclose();
      await startUpload({ copyLink: true });
    }
  }

  async function copyImage() {
    try {
      await copyImageToClipboard(currentImagePath(), session.outputScale);
      copyImageLabel = 'Copied';
      setTimeout(() => { copyImageLabel = 'Copy image'; }, 1000);
    } catch (e) {
      console.error('Copy image failed:', e);
    }
  }

  async function openInBrowser() {
    if (session.url) await openUrl(session.url);
  }

  async function searchGoogle() {
    if (session.url) await openUrl(`https://lens.google.com/uploadbyurl?url=${encodeURIComponent(session.url)}`);
  }

  async function searchTineye() {
    if (session.url) await openUrl(`https://tineye.com/search?url=${encodeURIComponent(session.url)}`);
  }

  async function searchEverywhere() {
    await searchGoogle();
    await searchTineye();
  }

  function toggleAutoclose() {
    autoCloseEnabled = !autoCloseEnabled;
    if (autoCloseEnabled) {
      // Explicit user opt-in — start now even after an editor visit (which
      // otherwise suppresses the automatic countdown).
      if (linkUsable || session.status === 'skipped') startAutoclose();
    } else {
      stopAutoclose();
    }
  }

  function handleEdit() {
    stopAutoclose();
    if (onEdit) onEdit();
  }
</script>

<div class="results-container">
  <div class="results-card">
    <div class="top-area">
      <!-- Left: URL + preview + status + search -->
      <div class="left-col">
        <input
          type="text"
          value={session.url}
          readonly
          placeholder={uploading ? '' : session.status === 'skipped' ? 'Press Upload to get a link' : 'URL will appear here...'}
          class="url-input"
        />
        <div class="preview-row">
          <div class="preview-box">
            {#if previewDataUrl}
              <img src={previewDataUrl} alt="preview" class="preview-img" />
            {/if}
          </div>
          <div class="info-col">
            <div class="row-status">
              {#if uploading}
                <div class="spinner"></div>
                <span class="status-text">Uploading to server...</span>
              {:else if uploadError}
                <span class="status-icon error">&#x2716;</span>
                <span class="status-text error-text">
                  {shortError}
                  {#if uploadError.length > ERROR_MAX}
                    <button type="button" class="more-link" onclick={() => showErrorPopup = true}>more</button>
                  {/if}
                </span>
              {:else if session.status === 'skipped'}
                <span class="status-icon ok">&#x2714;</span>
                <span class="status-text">Image copied to clipboard.</span>
              {:else if session.stale}
                <span class="status-icon" style="color: #ff9800;">&#x26A0;</span>
                <span class="status-text">Link points to the un-edited image. Press "Update link".</span>
              {:else}
                <span class="status-icon ok">&#x2714;</span>
                <span class="status-text">
                  Uploaded. {session.copyImageMode ? 'Image copied to clipboard.' : 'Link copied to clipboard.'}
                  {#if session.clipboardWarning}<span style="color: #ff9800;"> (clipboard was busy — press "Copy link")</span>{/if}
                </span>
              {/if}
            </div>
            <div class="search-buttons">
              <button class="btn-default" disabled={!linkUsable} onclick={searchGoogle}>Google</button>
              <button class="btn-default" disabled={!linkUsable} onclick={searchTineye}>Tineye</button>
              <button class="btn-default" disabled={!linkUsable} onclick={searchEverywhere}>Search both</button>
            </div>
            <label class="autoclose-label">
              <input type="checkbox" checked={autoCloseEnabled} onchange={toggleAutoclose} />
              <span>{autoCloseRunning ? `Autoclose in ${autoCloseSeconds} seconds` : 'Autoclose'}</span>
            </label>
            <span class="hint-text">{$settings.captureHotkey} = {$settings.defaultMode === 'image' ? 'Image' : 'Link'}, &nbsp; ({$settings.captureHotkey}) x2 = {$settings.defaultMode === 'image' ? 'Link' : 'Image'}</span>
          </div>
        </div>
      </div>

      <!-- Right: action buttons -->
      <div class="right-col">
        <button class="btn-accent btn-action" disabled={primaryDisabled} onclick={primaryAction}>{primaryLabel}</button>
        <button class="btn-accent btn-action" onclick={copyImage}>{copyImageLabel}</button>
        <button class="btn-default btn-action" disabled={!linkUsable} onclick={openInBrowser}>Show</button>
        <button class="btn-default btn-action" onclick={handleEdit}>Edit</button>
      </div>
    </div>
  </div>
</div>

{#if showErrorPopup}
  <div class="error-overlay" onclick={() => showErrorPopup = false} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ' || e.key === 'Escape') { e.preventDefault(); showErrorPopup = false; } }} role="button" tabindex="-1">
    <div class="error-popup" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
      <div class="error-popup-header">
        <span>Error details</span>
        <button class="error-popup-close" onclick={() => showErrorPopup = false}>&times;</button>
      </div>
      <pre class="error-popup-body">{uploadError}</pre>
      <button class="btn-accent error-popup-copy" onclick={async () => { await writeText(uploadError); }}>Copy to clipboard</button>
    </div>
  </div>
{/if}

<style>
  .results-container {
    padding: 7px;
    height: 100vh;
    min-width: 540px;
    box-sizing: border-box;
    background: var(--bg-base);
    overflow: hidden;
  }

  .results-card {
    background: var(--bg-surface);
    border-radius: 6px;
    padding: 14px;
    height: 100%;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
  }

  .top-area {
    display: flex;
    gap: 8px;
    flex: 1;
  }

  .left-col {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }

  .right-col {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    flex-shrink: 0;
  }

  .url-input {
    width: 100%;
    background: var(--bg-input);
    color: var(--text-main);
    border: 1px solid var(--border);
    padding: 5px 10px;
    border-radius: 4px;
    font-family: 'Segoe UI', sans-serif;
    font-size: 10.5pt;
    height: 30px;
    box-sizing: border-box;
  }

  .url-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .preview-row {
    display: flex;
    gap: 10px;
    align-items: flex-start;
  }

  .preview-box {
    width: 160px;
    height: 120px;
    border: 1px solid var(--border);
    border-radius: 4px;
    overflow: hidden;
    flex-shrink: 0;
    background: var(--bg-input);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .preview-img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }

  .info-col {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .row-status {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 22px;
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--accent);
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .status-icon {
    font-size: 14px;
    flex-shrink: 0;
  }

  .status-icon.ok {
    color: #4caf50;
  }

  .status-icon.error {
    color: #f44336;
  }

  .status-text {
    font-size: 9pt;
    color: var(--text-dim);
  }

  .error-text {
    color: #f44336;
  }

  .search-buttons {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .autoclose-label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 9pt;
    color: var(--text-main);
    cursor: pointer;
  }

  .autoclose-label input[type="checkbox"] {
    accent-color: var(--accent);
  }

  /* Buttons */
  .btn-accent {
    background: var(--accent);
    color: white;
    border: none;
    padding: 5px 16px;
    border-radius: 4px;
    font-family: 'Segoe UI Semibold', 'Segoe UI', sans-serif;
    font-size: 9pt;
    cursor: pointer;
    transition: background 0.15s;
    height: 30px;
    white-space: nowrap;
  }

  .btn-accent:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn-accent:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .btn-action {
    width: 130px;
    flex-shrink: 0;
  }

  .btn-default {
    background: var(--bg-input);
    color: var(--text-main);
    border: 1px solid var(--border);
    padding: 5px 14px;
    border-radius: 4px;
    font-family: 'Segoe UI', sans-serif;
    font-size: 9pt;
    cursor: pointer;
    transition: border-color 0.15s;
    height: 30px;
    white-space: nowrap;
  }

  .btn-default:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .btn-default:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .hint-text {
    font-size: 7.5pt;
    color: var(--text-dim);
    opacity: 0.5;
  }

  .more-link {
    background: none;
    border: none;
    padding: 0;
    font-family: inherit;
    color: var(--accent);
    text-decoration: underline;
    cursor: pointer;
    margin-left: 4px;
    font-size: 9pt;
  }

  .more-link:hover {
    opacity: 0.8;
  }

  .error-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }

  .error-popup {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    width: 90%;
    max-width: 500px;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .error-popup-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    font-size: 10pt;
    font-weight: 600;
    color: var(--text-main);
  }

  .error-popup-close {
    background: none;
    border: none;
    color: var(--text-dim);
    font-size: 18px;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
  }

  .error-popup-close:hover {
    color: var(--text-main);
  }

  .error-popup-body {
    padding: 14px;
    margin: 0;
    font-size: 8.5pt;
    color: #f44336;
    white-space: pre-wrap;
    word-break: break-all;
    overflow-y: auto;
    max-height: 300px;
    font-family: 'Consolas', 'Courier New', monospace;
  }

  .error-popup-copy {
    margin: 0 14px 14px;
    align-self: flex-start;
  }
</style>