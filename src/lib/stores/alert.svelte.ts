/**
 * Cross-window replacement for the browser's `alert()`.
 *
 * WHY THIS EXISTS: `alert()`/`confirm()`/`prompt()` are silent no-ops in the
 * Tauri WebView on macOS — wry's WKWebView UIDelegate doesn't implement the
 * WKUIDelegate JS-dialog-panel methods (runJavaScriptAlertPanel etc.), unlike
 * WebView2 on Windows which shows them natively. Every `alert(...)` error
 * message in this codebase was silently swallowed on macOS: the user clicks
 * something, an error IS thrown and caught, and nothing visibly happens.
 * A plain DOM-rendered modal works identically on both WebView engines.
 */
export interface AlertAction {
  /** Label shown on the modal button. */
  label: string;
  /**
   * URL the button deep-links to when clicked (system `open` via the
   * tauri-plugin-opener plugin). Used for, e.g., "Open System Settings"
   * on the Screen-Recording preflight.
   */
  url: string;
}

export const alertState = $state({
  message: null as string | null,
  action: null as AlertAction | null,
});

export function showAlert(message: string, action?: AlertAction) {
  alertState.message = message;
  alertState.action = action ?? null;
}

export function dismissAlert() {
  alertState.message = null;
  alertState.action = null;
}
