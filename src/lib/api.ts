/**
 * Typed wrappers around Tauri `invoke` — the single IPC boundary for the
 * frontend. Centralizing the command names, argument shapes and return types
 * here removes the scattered `as` casts and ad-hoc inline interfaces, so a
 * renamed command or a camelCase/snake_case slip is caught by the compiler.
 *
 * Argument keys must match the Rust command parameters (Tauri serializes them
 * camelCase). Return types mirror the Rust return values.
 */
import { invoke } from '@tauri-apps/api/core';
import type { AppSettings } from './stores/settings';
import type { DiscoveredPlugin, PluginConfig } from './plugin-types';

// ── Shapes crossing the IPC boundary ────────────────────────────

/** Pending capture handed to a freshly-created results window (main.rs). */
export interface PendingImage {
  path: string;
  copyImageMode: boolean;
  /** capture-monitor DPI scale; applied only at output (upload/clipboard) */
  outputScale: number;
}

/** Result of a pooled Google Drive upload (upload_gdrive.rs). */
export interface GdriveUploadResult {
  url: string;
  instant: boolean;
}

// ── Settings ────────────────────────────────────────────────────

export const loadSettings = () => invoke<AppSettings>('load_settings');
export const saveSettings = (settings: AppSettings) => invoke<void>('save_settings', { settings });
export const saveResultsWindowSize = (width: number, height: number) =>
  invoke<void>('save_results_window_size', { width, height });

// ── Capture / image ─────────────────────────────────────────────

export const readImageBase64 = (path: string) => invoke<string>('read_image_base64', { path });
export const saveImageBase64 = (base64Data: string) => invoke<string>('save_image_base64', { base64Data });
export const saveImageToFile = (sourcePath: string, outputScale: number) =>
  invoke<string | null>('save_image_to_file', { sourcePath, outputScale });
export const getPendingImage = () => invoke<PendingImage | null>('get_pending_image');
export const copyImageToClipboard = (path: string, outputScale: number) =>
  invoke<void>('copy_image_to_clipboard', { path, outputScale });

// ── Window lifecycle (Rust-driven sizing) ───────────────────────

export const setupEditorWindow = () => invoke<void>('setup_editor_window');
export const restoreResultsWindow = () => invoke<void>('restore_results_window');
export const updateHotkey = (hotkey: string) => invoke<void>('update_hotkey', { hotkey });

// ── Upload ──────────────────────────────────────────────────────

export interface S3UploadArgs {
  imagePath: string;
  outputScale: number;
}
export const uploadToS3 = (args: S3UploadArgs) => invoke<string>('upload_to_s3', { ...args });

export const gdriveUploadPooled = (imagePath: string, folderName: string, callId: number, outputScale: number) =>
  invoke<GdriveUploadResult>('gdrive_upload_pooled', { imagePath, folderName, callId, outputScale });
export const gdriveAuthorize = () => invoke<string>('gdrive_authorize');
export const gdriveHasToken = () => invoke<boolean>('gdrive_has_token');
export const gdriveDisconnect = () => invoke<void>('gdrive_disconnect');

// ── Encryption (clipboard plugin passthrough commands) ──────────

export const encryptText = (text: string, password: string) =>
  invoke<string>('encrypt_text', { text, password });
export const decryptText = (text: string, password: string) =>
  invoke<string>('decrypt_text', { text, password });

// ── Plugins ─────────────────────────────────────────────────────

export const discoverPlugins = () => invoke<DiscoveredPlugin[]>('discover_plugins');
export const loadPluginConfigs = () => invoke<PluginConfig[]>('load_plugin_configs');
export const applyPluginConfig = (configs: PluginConfig[]) =>
  invoke<void>('apply_plugin_config', { configs });
export const runScript = (path: string) => invoke<string>('run_script', { path });
export const runScriptInTerminal = (path: string) => invoke<void>('run_script_in_terminal', { path });
export const saveScript = (name: string, language: string, content: string, overwrite: boolean) =>
  invoke<string>('save_script', { name, language, content, overwrite });
export const deleteScript = (path: string) => invoke<void>('delete_script', { path });
export const checkRuntime = (language: string) => invoke<string>('check_runtime', { language });
export const readScript = (path: string) => invoke<string>('read_script', { path });
export const precompileScript = (path: string) => invoke<string>('precompile_script', { path });
