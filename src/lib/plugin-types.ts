/**
 * Shared plugin types — mirror the Rust structs in src-tauri/src/plugins.rs.
 * Kept in one place so the frontend doesn't re-declare them per component and
 * drift on field names/casing.
 *
 * NOTE on casing: the Rust `PluginFunction`/`DiscoveredPlugin` are serialized
 * as-is (snake_case: default_key, settings_format, plugin_type), while
 * `PluginConfig` uses `#[serde(rename_all = "camelCase")]` (keyBindings). These
 * types reflect exactly what crosses the IPC boundary.
 */

export interface PluginFunction {
  id: string;
  label: string;
  default_key: string;
}

export interface DiscoveredPlugin {
  path: string;
  valid: boolean;
  name: string;
  version: string;
  description: string;
  instruction: string;
  settings_description: string;
  settings_format: string;
  functions: PluginFunction[];
  error: string;
  plugin_type: 'exe' | 'python' | 'csharp' | 'powershell';
  mode: 'daemon' | 'oneshot';
}

export interface PluginConfig {
  path: string;
  enabled: boolean;
  keyBindings: Record<string, string>;
  settings: string;
}
