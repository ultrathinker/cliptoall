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
export const alertState = $state({ message: null as string | null });

export function showAlert(message: string) {
  alertState.message = message;
}

export function dismissAlert() {
  alertState.message = null;
}
