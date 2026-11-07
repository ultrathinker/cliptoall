/**
 * Which parts of the application exist in this build.
 *
 * Asked of the Rust side at runtime (`app_capabilities`) rather than decided
 * by a Vite define, because the binary is what actually decides: there are
 * two macOS builds from one frontend bundle — the default one (no plugin
 * system compiled in) and the personal one (`--features personal`, plugins
 * on) — and the UI must never offer a tab whose backend commands are not
 * registered (that produces an "unknown command" error, not a missing tab).
 *
 * Windows always has the plugin system (the Rust side gates it with
 * `cfg(plugins)`, which is always set there), so it is known up front and no
 * IPC is made: Windows behaves exactly as it did before this was dynamic.
 */

import { invoke } from '@tauri-apps/api/core';
import { writable } from 'svelte/store';
import { IS_MAC } from './platform';

export interface AppCapabilities {
  /** The plugin system is compiled in. */
  plugins: boolean;
  /** Where plugins are discovered; null when unknown or not applicable. */
  pluginsDir: string | null;
  /** Name of this build's settings/data directory ("ClipToAll" / "ClipToAll Personal"). */
  appDirName: string;
}

/**
 * Starts as "no plugins" on macOS until the backend answers, so a default
 * build never flashes a Plugins tab. Read it with `$capabilities` in
 * components.
 */
export const capabilities = writable<AppCapabilities>({
  plugins: !IS_MAC,
  pluginsDir: null,
  appDirName: 'ClipToAll',
});

let loading: Promise<void> | null = null;

/** Ask the backend once (macOS only); later calls reuse the same answer. */
export function loadCapabilities(): Promise<void> {
  if (!IS_MAC) return Promise.resolve();
  loading ??= invoke<AppCapabilities>('app_capabilities')
    .then((c) => capabilities.set(c))
    .catch((e) => {
      // Keep the safe default (no plugins); allow a retry next time.
      console.error('app_capabilities failed:', e);
      loading = null;
    });
  return loading;
}
