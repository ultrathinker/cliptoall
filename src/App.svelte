<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
  import { loadSettings, getPendingImage, setupEditorWindow, restoreResultsWindow } from './lib/api';
  import { applyTheme } from './lib/stores/theme';
  import { settings, defaultSettings } from './lib/stores/settings';
  import { session, initSession, markSkipped, startUpload, applyEditedPath, currentImagePath, updateUrl, markContentReady } from './lib/stores/session.svelte';
  import { showAlert } from './lib/stores/alert.svelte';
  import Settings from './windows/Settings.svelte';
  import About from './windows/About.svelte';
  import Results from './windows/Results.svelte';
  import Editor from './windows/Editor.svelte';
  import OverlayWeb from './windows/OverlayWeb.svelte';
  import AlertModal from './lib/AlertModal.svelte';

  let currentWindow = $state('loading');
  let isMainWindow = false;

  async function showWindow(name: string, width: number, height: number, minWidth?: number, minHeight?: number, resizable = true) {
    currentWindow = name;
    const win = getCurrentWindow();
    await win.setDecorations(true);
    await win.setFullscreen(false);
    await win.setAlwaysOnTop(false);
    await win.setResizable(resizable);
    await win.setMinSize(null);
    await win.setSize(new LogicalSize(width, height));
    await win.center();
    await win.show();
    await win.setFocus();
    if (minWidth && minHeight) {
      await win.setMinSize(new LogicalSize(minWidth, minHeight));
    }
  }

  /// Give the browser a chance to paint, then continue — but NEVER hang.
  ///
  /// requestAnimationFrame is suspended while a window is hidden (macOS
  /// suspends rendering for invisible windows), and every window here is
  /// hidden at exactly this point on purpose: they're revealed only once
  /// their themed UI is ready, to avoid a white flash. Waiting on a bare rAF
  /// is therefore a deadlock — the frame that would resolve it can only
  /// happen after the show() that the wait is blocking. That is precisely
  /// why a pre-warmed results window never appeared at all: warmed long
  /// before, fully idle and hidden, its rAF never fired again. (A freshly
  /// built hidden window still got its initial frames, which is why this
  /// only broke once windows started being reused.)
  ///
  /// So: race rAF against a short timer. Visible window → resolves on the
  /// real frame (~32ms). Hidden window → resolves on the timer.
  function waitForPaint(timeoutMs = 60): Promise<void> {
    return new Promise((resolve) => {
      let settled = false;
      const finish = () => { if (!settled) { settled = true; resolve(); } };
      requestAnimationFrame(() => requestAnimationFrame(finish));
      setTimeout(finish, timeoutMs);
    });
  }

  /// Wire a capture into the session store and reveal the window. Shared by
  /// the normal path (window built for this capture, image already pending)
  /// and the pre-warmed-spare path (window booted earlier, image arrives
  /// later via the "results-show" event).
  async function showResults(pending: { path: string; copyImageMode: boolean; outputScale?: number }) {
    const win = getCurrentWindow();
    initSession(pending.path, pending.copyImageMode, pending.outputScale ?? 1);
    // Decide the initial action once, here — not on every Results mount.
    if (pending.copyImageMode && $settings.skipUploadInCopyMode) {
      markSkipped(); // image already on clipboard; upload deferred until user asks
    } else {
      startUpload(); // fire-and-forget; state tracked in the session store
    }
    currentWindow = 'results';
    // The results window is created HIDDEN (main.rs / results_spare.rs) to
    // avoid a white flash; reveal it only after the themed UI has rendered.
    // tick() flushes Svelte, then waitForPaint gives the browser a chance to
    // paint — but with a timeout, because rAF alone would hang forever here
    // (see waitForPaint).
    await tick();
    await waitForPaint();
    await win.show();
    await win.setFocus();
  }

  onMount(async () => {
    const win = getCurrentWindow();
    isMainWindow = win.label === 'main';

    // Load settings + theme for ALL windows
    try {
      const loadedSettings = await loadSettings();
      settings.set({ ...defaultSettings, ...loadedSettings });
      applyTheme(loadedSettings.theme || 'crimson');
    } catch (e) {
      console.error('Failed to load settings:', e);
      applyTheme('crimson');
    }

    // Live-update settings/theme in every open window when saved elsewhere.
    // Secrets are NOT broadcast (see save_settings), so keep whatever secret
    // values this window already loaded rather than blanking them.
    listen('settings-changed', (event) => {
      const s = event.payload as any;
      settings.update((cur) => ({
        ...defaultSettings,
        ...s,
        amazonAccessKeyId: s.amazonAccessKeyId || cur.amazonAccessKeyId,
        amazonSecretAccessKey: s.amazonSecretAccessKey || cur.amazonSecretAccessKey,
      }));
      applyTheme(s.theme || 'crimson');
    });

    // GDrive pool fell back to a direct upload — adopt the corrected link
    // (only if it matches the latest upload; see updateUrl).
    listen('gdrive-url-updated', (event) => {
      const p = event.payload as { callId: number; url: string };
      updateUrl(p.callId, p.url);
    });

    // GDrive pool: the background PATCH landed, so the already-shared link now
    // serves the real image instead of the blank placeholder.
    listen('gdrive-content-ready', (event) => {
      const p = event.payload as { callId: number };
      markContentReady(p.callId);
    });

    // Screen-Recording TCC preflight — fired by `start_capture` in main.rs
    // whenever the user presses the capture hotkey without the grant. The
    // Rust side already showed + focused the main window before emitting, so
    // by the time this handler runs AlertModal is mounted in a visible
    // window. Only relevant on macOS, but the listener is harmless elsewhere
    // — the event is never emitted on Windows.
    listen<{ settings_url: string }>('screen-recording-required', (event) => {
      const url = event.payload?.settings_url;
      // Spell out BOTH the user's next step and the gotcha. "Restart the app"
      // is the single most-skipped line in any macOS TCC onboarding copy —
      // the system prompts the user, they flip the toggle, nothing happens,
      // they think the toggle is broken. Say it twice (once here, once in the
      // App Store review notes) and link the system pane so they don't have
      // to dig for it.
      showAlert(
        'Screen Recording permission is required to capture your screen.\n\n' +
        'Click "Open Settings" below to grant ClipToAll access in System Settings → Privacy & Security → Screen Recording, then quit and reopen ClipToAll for the new permission to take effect.',
        url ? { label: 'Open Settings', url } : undefined,
      );
    });

    if (isMainWindow) {
      // Main window — tray app, settings, about
      currentWindow = 'main';

      listen('show-settings', async () => {
        if (currentWindow === 'settings') {
          const win = getCurrentWindow();
          await win.unminimize();
          await win.show();
          await win.setFocus();
          return;
        }
        showWindow('settings', 680, 600, 680, 500);
      });

      listen('show-about', async () => {
        if (currentWindow === 'about') {
          const win = getCurrentWindow();
          await win.unminimize();
          await win.show();
          await win.setFocus();
          return;
        }
        showWindow('about', 540, 260, 400, 260);
      });
    } else if (win.label === 'overlay') {
      // macOS web overlay (Phase 3) — created already visible/sized/positioned
      // by overlay_web.rs, nothing to fetch or show here. Now opaque (TASK A,
      // phase 14): Tauri's `macos-private-api` is gone, so the body uses the
      // .overlay-window class to switch from the theme crimson to a neutral
      // dim background that matches the canvas wipe/empty state. The canvas
      // covers the entire viewport (see OverlayWeb.svelte's .overlay-canvas),
      // so the user only ever sees this background in the gap between
      // captures or inside the 400 ms ready-fallback.
      document.body.classList.add('overlay-window');
      currentWindow = 'overlay';
    } else {
      // Results window — fetch the pending image data and set up the session.
      // The session (upload state, URL, edited path) lives in a module store so
      // it survives the Results↔Editor component swap (BUGS#1).
      const isSpare = new URLSearchParams(location.search).get('spare') === '1';
      const pending = await getPendingImage();
      if (pending) {
        await showResults(pending);
      } else if (isSpare) {
        // A pre-warmed spare (results_spare.rs, macOS): booted early and kept
        // hidden precisely so it does NOT pay WKWebView's cold start when a
        // capture lands. It has no image yet by design — wait to be handed one
        // rather than closing (which is what a non-spare does below).
        listen('results-show', async () => {
          const p = await getPendingImage();
          if (p) await showResults(p);
        });
      } else {
        // No pending image (e.g. the window was reloaded after its capture was
        // already consumed/closed). Nothing to show — close instead of hanging
        // forever on the blank 'loading' screen (3.23 / App.svelte null-pending).
        await getCurrentWindow().close();
      }
    }
  });

  async function openEditor() {
    const win = getCurrentWindow();
    await win.setMinSize(new LogicalSize(500, 400));
    currentWindow = 'editor';
    setupEditorWindow().catch(e => console.error('Editor window error:', e));
  }

  async function handleEditorSave(newPath: string) {
    const win = getCurrentWindow();
    await win.setMinSize(new LogicalSize(600, 200));
    applyEditedPath(newPath); // updates session; marks link stale if already uploaded
    currentWindow = 'results';
    restoreResultsWindow().catch(() => {});
  }

  async function handleEditorCancel() {
    // Go back to results (image is still there, upload may be in progress)
    const win = getCurrentWindow();
    await win.setMinSize(new LogicalSize(600, 200));
    currentWindow = 'results';
    restoreResultsWindow().catch(() => {});
  }

  async function handleClose() {
    const win = getCurrentWindow();
    if (isMainWindow) {
      await win.setMinSize(null);
      await win.hide();
      currentWindow = 'main';
    } else {
      win.close();
    }
  }

</script>

<main
  class="w-full h-screen"
  style="background: {currentWindow === 'overlay' ? 'rgb(40, 38, 42)' : 'var(--bg-base)'}; color: var(--text-main);"
>
  {#if currentWindow === 'settings'}
    <Settings onClose={handleClose} />
  {:else if currentWindow === 'about'}
    <About onClose={handleClose} />
  {:else if currentWindow === 'results'}
    <Results onEdit={openEditor} />
  {:else if currentWindow === 'editor'}
    <Editor imagePath={currentImagePath()} outputScale={session.outputScale} onSave={handleEditorSave} onCancel={handleEditorCancel} />
  {:else if currentWindow === 'overlay'}
    <OverlayWeb />
  {/if}
</main>
<AlertModal />
