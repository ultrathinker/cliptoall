/**
 * Which parts of the application exist in this build.
 *
 * Unlike a Vite-time define, this is derived from the platform, because the
 * platform is what actually decides. There is one macOS build, so what you run
 * is what ships — no env var to remember at release time, and no chance of
 * shipping a store bundle whose UI still offers a feature the binary does not
 * have.
 */

import { IS_MAC } from './platform';

/**
 * True where the plugin system exists — Windows only.
 *
 * The Rust side gates the same code with `#[cfg(windows)]` (see the comment
 * above `[dependencies]` in src-tauri/Cargo.toml for why macOS cannot have it:
 * the plugins folder lives inside the signed .app bundle, and the App Sandbox
 * forbids spawning interpreters). This constant must stay in agreement with
 * that gate — a UI offering a tab whose backend commands are not registered
 * produces an "unknown command" error, not a missing tab.
 */
export const PLUGINS_ENABLED: boolean = !IS_MAC;
