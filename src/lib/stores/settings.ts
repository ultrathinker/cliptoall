import { writable } from 'svelte/store';
import { IS_MAC } from '../platform';

export interface AppSettings {
  imagePrefix: string;
  autorun: boolean;
  autoclose: boolean;
  amazonAccessKeyId: string;
  amazonSecretAccessKey: string;
  amazonBucket: string;
  amazonS3folder: string;
  amazonRegion: string;
  loggingOn: boolean;
  storageType: 'gdrive' | 's3';
  googleDriveFolder: string;
  /** legacy boolean, kept for round-trip; canonical setting is outputMode */
  downscaleForDpi: boolean;
  /** how the shared image is produced: full-res / downscaled / full-res+EXIF density */
  outputMode: 'off' | 'resize' | 'exif';
  theme: string;
  resultsWidth: number;
  resultsHeight: number;
  skipUploadInCopyMode: boolean;
  captureHotkey: string;
  escapeHidesResults: boolean;
  defaultMode: 'link' | 'image';
  jpegQuality: number;
}

// Frontend pre-load placeholder. The real defaults come from the Rust side
// (AppSettings::default in src-tauri/src/commands/settings.rs) via the
// load_settings IPC, so this only flashes for the moment between store
// creation and the first IPC round-trip — but that flash is visible, so the
// hotkey default at least has to match the platform to avoid showing
// "Alt+X" briefly on a Mac.
export const defaultSettings: AppSettings = {
  imagePrefix: 'cta_',
  // Must match AppSettings::default() in commands/settings.rs, which is off on
  // macOS: a login item should be something the user asked for.
  autorun: !IS_MAC,
  autoclose: true,
  amazonAccessKeyId: '',
  amazonSecretAccessKey: '',
  amazonBucket: 'cliptoall',
  amazonS3folder: '',
  amazonRegion: 'us-west-2',
  loggingOn: false,
  storageType: 'gdrive',
  googleDriveFolder: 'public-images',
  downscaleForDpi: !IS_MAC,
  outputMode: IS_MAC ? 'exif' : 'resize',
  theme: 'crimson',
  resultsWidth: 850,
  // Matches RESULTS_MIN_HEIGHT in src-tauri/src/main.rs — below that, the
  // fixed-size preview box / action buttons no longer fit and the window's
  // own min_inner_size would clamp it back up anyway; kept in sync here so
  // this pre-load fallback doesn't visually differ from the enforced floor.
  resultsHeight: 210,
  skipUploadInCopyMode: true,
  captureHotkey: IS_MAC ? 'Ctrl+Cmd+X' : 'Alt+X',
  escapeHidesResults: true,
  defaultMode: 'image',
  jpegQuality: 85,
};

export const settings = writable<AppSettings>(defaultSettings);
