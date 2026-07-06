/**
 * Per-window capture session — the single source of truth for one screenshot's
 * upload lifecycle. It lives at module scope, so it SURVIVES the Results↔Editor
 * component swap (App.svelte only unmounts child components, not this module).
 *
 * This is what fixes BUGS#1: previously the upload URL / status / "skipped" flag
 * lived inside Results.svelte and were destroyed every time the editor opened.
 *
 * Each results window is its own WebView (own JS realm), so this singleton is
 * naturally scoped to exactly one capture.
 */
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { get } from 'svelte/store';
import { settings } from './settings';
import { uploadToS3, gdriveUploadPooled, copyImageToClipboard } from '../api';

export type UploadStatus = 'idle' | 'skipped' | 'uploading' | 'done' | 'error';

export const session = $state({
  initialized: false,
  originalPath: '',
  editedPath: null as string | null,
  copyImageMode: false,
  status: 'idle' as UploadStatus,
  url: '',
  error: '',
  /** true when `url` points to a version older than the current (edited) image */
  stale: false,
  /** true when upload succeeded but writing the link to the clipboard failed */
  clipboardWarning: false,
  /** id of the latest upload; a late GDrive pool fallback with an older id is ignored */
  callId: 0,
  /** capture-monitor DPI scale; the image is full-res, this is applied only at
   *  output (upload/clipboard) when "resize shared images" is on */
  outputScale: 1,
});

/** The image the user is currently looking at: edited version if any, else original. */
export function currentImagePath(): string {
  return session.editedPath ?? session.originalPath;
}

/** Initialize the session for a freshly captured image. Called once per window. */
export function initSession(originalPath: string, copyImageMode: boolean, outputScale = 1) {
  session.initialized = true;
  session.originalPath = originalPath;
  session.editedPath = null;
  session.copyImageMode = copyImageMode;
  session.status = 'idle';
  session.url = '';
  session.error = '';
  session.stale = false;
  session.clipboardWarning = false;
  session.outputScale = outputScale > 0 ? outputScale : 1;
}

/** Mark the session as "image already copied, upload deferred until user asks". */
export function markSkipped() {
  session.status = 'skipped';
}

/** Try to copy text to the clipboard, retrying a few times — on Windows the
 *  clipboard is often briefly locked by clipboard-manager apps (BUGS#4c). */
async function copyToClipboardSafe(text: string): Promise<boolean> {
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      await writeText(text);
      return true;
    } catch {
      await new Promise((r) => setTimeout(r, 80));
    }
  }
  return false;
}

/**
 * Upload the current image and update session state. Safe to call again for a
 * deferred/skipped upload or a Retry after an error. Never throws — failures
 * land in session.status='error'. Clipboard failures never mask a good upload.
 */
let callSeq = 0;

export async function startUpload(opts: { copyLink?: boolean } = {}): Promise<void> {
  if (session.status === 'uploading') return; // already in flight
  const s = get(settings);
  const path = currentImagePath();

  // Each upload gets a fresh id; a late GDrive pool fallback carrying an older
  // id is ignored (so it can't overwrite a newer edited-image link) (3.6).
  const myCallId = ++callSeq;
  session.callId = myCallId;
  session.status = 'uploading';
  session.error = '';

  try {
    let url: string;
    if (s.storageType === 's3') {
      url = await uploadToS3({
        imagePath: path,
        outputScale: session.outputScale,
      });
    } else {
      const result = await gdriveUploadPooled(path, s.googleDriveFolder, myCallId, session.outputScale);
      url = result.url;
    }

    session.url = url;
    session.status = 'done';
    session.stale = false;

    // Copy the link in normal (link) mode, or when the user explicitly asked.
    if (!session.copyImageMode || opts.copyLink) {
      session.clipboardWarning = !(await copyToClipboardSafe(url));
    }
  } catch (e) {
    session.status = 'error';
    session.error = String(e);
  }
}

/**
 * Replace the link with a corrected one (e.g. the GDrive pool fell back to a
 * direct upload because the placeholder couldn't be filled — BUGS#4b). Re-copy
 * it in link mode so the user's clipboard no longer holds the broken link.
 */
export async function updateUrl(callId: number, url: string): Promise<void> {
  if (!url) return;
  // Ignore a fallback for an upload the session has already moved past (3.6).
  if (callId !== session.callId) return;
  session.url = url;
  session.status = 'done';
  session.stale = false;
  if (!session.copyImageMode) {
    session.clipboardWarning = !(await copyToClipboardSafe(url));
  }
}

/** Copy the already-uploaded link to the clipboard on demand. Returns success. */
export async function copyLink(): Promise<boolean> {
  if (!session.url) return false;
  const ok = await copyToClipboardSafe(session.url);
  session.clipboardWarning = !ok;
  return ok;
}

/**
 * Register an edited image produced by the editor. If a link was already
 * uploaded, it now points to the pre-edit version → mark it stale so the UI
 * offers to re-upload. Refreshes the clipboard image in copy-image mode.
 */
export function applyEditedPath(newPath: string) {
  session.editedPath = newPath;
  if (session.status === 'done') {
    session.stale = true;
  }
  if (session.copyImageMode) {
    copyImageToClipboard(newPath, session.outputScale).catch(() => {});
  }
}
