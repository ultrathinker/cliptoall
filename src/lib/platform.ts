/**
 * Single source of truth for "are we on the macOS build" in the frontend.
 *
 * The WKWebView user agent always contains "Macintosh" (WKWebView is built on
 * WebKit, which only ships on macOS). The Windows WebView2 UA does not — so
 * testing the UA for "Macintosh" distinguishes the two builds cleanly. This is
 * the only platform branch the frontend needs right now; every branch you add
 * is a maintenance cost, so keep the count small and check here before adding
 * a new `IS_MAC ?` anywhere.
 */
export const IS_MAC = typeof navigator !== 'undefined'
  && /Macintosh/.test(navigator.userAgent);

/**
 * Translate a stored hotkey modifier token to the form a Mac user expects to
 * see. The Rust parser (`parse_hotkey` in `src-tauri/src/main.rs:518`) accepts
 * `super | win | meta | cmd` interchangeably and stores them all as the same
 * SUPER modifier, so a stored value may legitimately contain any of those
 * spellings regardless of platform. On macOS we display `Cmd` (the key
 * labelling on the keyboard) and `Option` (the Alt equivalent); on every
 * other platform we display the stored token verbatim.
 */
export function displayHotkey(hk: string): string {
  if (!IS_MAC) return hk;
  // Split on '+', but keep empty segments (none here) and trim segments. A
  // trailing/leading '+' would survive — accept that, it cannot happen on
  // data shaped by the Settings UI.
  return hk.split('+').map(part => {
    const lower = part.trim().toLowerCase();
    if (lower === 'super' || lower === 'win' || lower === 'meta' || lower === 'cmd') return 'Cmd';
    if (lower === 'alt') return 'Option';
    return part.trim();
  }).join('+');
}

/**
 * Emit a modifier label that the *parser* will accept on macOS. The Rust
 * parser accepts `super|win|meta|cmd` for the Cmd modifier; we choose `Cmd`
 * because that is what gets shown back to the user via `displayHotkey`, so a
 * round-trip in the Settings UI produces a stable string. On non-macOS we
 * keep the historical `Super` label — this matches what `e.metaKey` has always
 * produced on Windows/Linux browsers.
 */
export function modifierLabelForCmd(): string {
  return IS_MAC ? 'Cmd' : 'Super';
}