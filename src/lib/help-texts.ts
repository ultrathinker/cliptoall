// Help-panel and guide texts for the Settings window. Extracted verbatim from
// Settings.svelte (pure static data) to shrink that component.
//
// Platform vocabulary: this file is shared between the Windows and macOS
// builds. Strings that read identically on both (e.g. "log in") need no
// branch. Where the wording genuinely differs (menu bar vs system tray,
// Login Items vs Registry), we branch on IS_MAC — kept inline rather than
// pulled out into per-platform modules so the help text stays in one place
// for translation/review. Add a branch only when the platforms actually
// diverge; do not branch for the sake of it.
//
// Build features: this file also branches on PLUGINS_ENABLED (TASK B /
// Phase 4a). The Mac App Store edition strips out the entire plugin system,
// so any sentence that mentions plugins in the user guide, and the entire
// `guidePlugins` developer reference, is omitted. PLUGINS_ENABLED is a
// Vite-time define (see vite.config.ts and lib/features.ts), so the
// `false` branch is dead-stripped at build time and never ships in the
// store bundle.

import { IS_MAC } from './platform';
import { PLUGINS_ENABLED } from './features';

// Help texts common to every build. The plugin-only help text (developer
// reference for writing plugins) lives in `pluginHelpTexts` below, gated by
// PLUGINS_ENABLED so the long developer manual is dead-stripped from the
// store build's bundle.
const baseHelpTexts: Record<string, { title: string; text: string }> = {
    autoclose: {
      title: 'Autoclose in 30 seconds',
      text: 'When enabled, the Results window (that shows after a screenshot) will automatically close after 30 seconds. This keeps your desktop clean — once the link is copied to your clipboard, you probably don\'t need the window anymore. If you disable this, the Results window stays open until you close it manually.',
    },
    escapeHides: {
      title: 'Escape hides Results window',
      text: 'When enabled, pressing the Escape key will hide the Results window instead of closing it completely. The upload continues in the background and the link will still be copied to your clipboard. If disabled, Escape does nothing and you must close the window with the X button.',
    },
    autorun: {
      title: IS_MAC ? 'Open at Login' : 'Add to Autorun',
      text: IS_MAC
        ? 'When enabled, ClipToAll will start automatically when you log in. It appears in the menu bar so you can take screenshots anytime with the hotkey. The app registers itself as a Login Item for your user account.'
        : 'When enabled, ClipToAll will start automatically when you log in to Windows. It runs in the system tray so you can take screenshots anytime with the hotkey. The app registers itself in the Windows Registry under HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run.',
    },
    logging: {
      title: 'Write to Log File',
      // The "next to the executable" wording was never true: log_file_path()
      // in main.rs writes under the OS config dir on every platform.
      text: `When enabled, ClipToAll writes detailed timing and debug information to a log file. This is useful for troubleshooting if something isn't working right — you can see exactly what the app is doing step by step. Keep it off for normal use to avoid unnecessary disk writes.

The log lives at ${IS_MAC ? '~/Library/Application Support/ClipToAll/logs/cliptoall.log' : '%APPDATA%\\ClipToAll\\logs\\cliptoall.log'}, and rotates once it reaches 25 MB (one previous generation is kept as cliptoall.log.old).`,
    },
    outputMode: {
      title: 'Shared image size on HiDPI displays',
      text: `Affects only the SHARED image (upload / clipboard / "Save as file") — never what you edit; the editor always shows the capture pixel-for-pixel.

THE PROBLEM (it's subtle):
${IS_MAC
  ? 'On a Retina display (2× backing scale), a screenshot has more pixels than it visually occupies — a region that looks 800 px wide is really 1600 px. The editor looks perfect because it keeps all 1600 pixels and paints them onto exactly 1600 physical screen pixels. A plain image FILE opened in a browser can\'t do that — a browser draws 1 image pixel per CSS pixel, then stretches by the device pixel ratio. So a shared file is EITHER ~2× too big (full pixels) OR resized-and-soft (fewer pixels). There is no way to make a plain file both correct-size AND perfectly crisp everywhere.'
  : 'On a Windows display scaled above 100% (e.g. 150%), a screenshot has more pixels than it visually occupies — a region that looks 800 px wide is really 1200 px. The editor looks perfect because it\'s a DPI-aware app: it keeps all 1200 pixels and paints them onto exactly 1200 physical screen pixels. A plain image FILE opened in a browser can\'t do that — a browser draws 1 image pixel per CSS pixel, then Windows stretches that by your scale factor. So a shared file is EITHER ~1.5× too big (full pixels) OR resized-and-slightly-soft (fewer pixels). There is no way to make a plain file both correct-size AND perfectly crisp everywhere.'}

THE THREE MODES:
• Full resolution — every physical pixel. Sharpest data, but in a browser it renders ${IS_MAC ? '~2×' : '~1.5×'} larger than you saw on screen. Best for archiving / OCR / recipients on HiDPI screens.
• Resize to logical size — shrinks the shared image to its on-screen size. Smaller files, correct size in any viewer, but slightly softer (some detail is dropped). Good default for sharing into chats/docs.
• Full-res + EXIF density — keeps ALL pixels and stamps a density tag in the JPEG. Modern browsers (Chrome, Safari, Firefox 90+) then show it at the correct logical size using every pixel — as crisp as the editor. Caveat: non-browser viewers (${IS_MAC ? 'Preview, Slack, Telegram' : 'Windows Photos, Slack, Telegram'}) ignore the tag and show it at full (large) size, and this only works for JPEG (browsers ignore DPI in PNG).

Refs: MDN devicePixelRatio (https://developer.mozilla.org/en-US/docs/Web/API/Window/devicePixelRatio); EXIF density explainer (https://github.com/eeeps/exif-intrinsic-sizing-explainer).`,
    },
    skipUpload: {
      title: 'Skip upload in Copy Image mode',
      text: 'In "Copy Image" mode (green), the screenshot is copied directly to your clipboard. When this option is enabled, the app won\'t upload the image to cloud storage at all — it just copies it and you\'re done. When disabled, the image is both copied to clipboard AND uploaded, giving you a link as well.',
    },
    theme: {
      title: 'Theme',
      text: 'Choose the visual appearance of ClipToAll. Available themes include light options like Classic and Mac, and dark options like Crimson Night, Ocean Night, and Forest Night. The theme changes immediately when you select it, so you can preview before saving.',
    },
    hotkey: {
      title: 'Capture Hotkey',
      text: IS_MAC
        ? 'The global keyboard shortcut that triggers a screenshot capture. Click the button and press your desired key combination (e.g., Ctrl+Cmd+X, Ctrl+Cmd+S); press Escape to cancel and keep the current one. The hotkey works system-wide, even when ClipToAll is in the background. Must include at least one modifier key (Cmd, Ctrl, Option, or Shift) unless using a function key.\n\nPick a combination macOS does not already use. A global hotkey takes it away from every application — Cmd+X, for instance, would stop Cut from working everywhere.'
        : 'The global keyboard shortcut that triggers a screenshot capture. Click the button and press your desired key combination (e.g., Alt+X, Ctrl+Shift+S). The hotkey works system-wide, even when ClipToAll is in the background. Must include at least one modifier key (Alt, Ctrl, or Shift) unless using a function key.',
    },
    defaultMode: {
      title: 'Default Mode',
      text: 'Controls what happens after you capture a screenshot.\n\n"Green: Copy Image" — the screenshot is copied to your clipboard as an image. Quick and simple, no upload needed.\n\n"Pink: Copy Link" — the screenshot is uploaded to your cloud storage (Google Drive or S3) and the public link is copied to your clipboard.\n\nYou can temporarily switch to the other mode by pressing the hotkey twice quickly (double-press) during capture.',
    },
    imagePrefix: {
      title: 'File name prefix',
      text: 'The prefix used for saved screenshot file names (and for the temp-file cleanup that removes old captures). The default is "cta_", producing names like cta_2026_07_05_09_03_01_abc.jpg. Leave it as "cta_" unless you have a reason to change it — an empty value falls back to "cta_".',
    },
    storage: {
      title: 'Image Storage',
      text: 'Choose where your screenshots are uploaded when using "Copy Link" mode.\n\nGoogle Drive — uploads to a shared folder in your Google Drive. Easy to set up: just click "Connect" and authorize. Images get a public sharing link automatically.\n\nAmazon S3 — uploads to an S3 bucket. Requires an AWS access key, secret key, bucket name, and region. Good for high-volume use or when you need full control over storage.',
    },
    guide: {
      title: 'ClipToAll — User Guide',
      text: `WHAT IS CLIPTOALL?
ClipToAll is a lightweight screenshot tool. Press a hotkey, select a region on screen, and instantly get either the image in your clipboard or a public link to it. No bloat, no delays — capture, copy, done.

HOW TO TAKE A SCREENSHOT
${IS_MAC
  ? 'Press Cmd+X (or your custom hotkey) anywhere. The screen dims with a subtle color tint and a crosshair cursor appears. Click and drag to select the area you want to capture. Release the mouse button — done!'
  : 'Press Alt+X (or your custom hotkey) anywhere in Windows. The screen dims with a subtle color tint and a crosshair cursor appears. Click and drag to select the area you want to capture. Release the mouse button — done!'}
\nYou can also ${IS_MAC ? 'left-click the menu bar icon' : 'left-click the tray icon'} to start a capture.
\nTo cancel a capture, press Escape, right-click, or middle-click.

TWO CAPTURE MODES
ClipToAll has two modes, indicated by the overlay tint color:
\nGreen tint = "Copy Image" mode
The screenshot is copied directly to your clipboard as an image. You can immediately paste it into any app (Slack, Telegram, Word, etc.). Fast and simple — no internet needed.
\nPink tint = "Copy Link" mode
The screenshot is uploaded to your cloud storage (Google Drive or Amazon S3) and a public sharing link is automatically copied to your clipboard. Perfect for sharing in chats or emails.

SWITCHING MODES
Your default mode is set in Settings (the "Default Mode" dropdown). Every time you press the hotkey, it starts in your default mode.
\nTo temporarily switch to the OTHER mode during a capture: just press the hotkey again (double-press). You'll see the overlay tint change color instantly — green becomes pink and vice versa. This toggle only affects the current capture; the next one will use your default again.

RESULTS WINDOW
After a capture, a Results window appears showing your screenshot. In "Copy Link" mode, it shows the upload progress and the resulting URL. In "Copy Image" mode, it confirms the image was copied.
\nThe Results window auto-closes after 30 seconds (configurable). You can also press Escape to hide it.

IMAGE EDITOR
From the Results window, click the Edit button to open the built-in image editor. Tools available:
\n• Pencil — freehand drawing
• Rectangle — draw rectangles (outlines)
• Arrow — draw arrows pointing in any direction
• Text — click anywhere to type text on the image
\nYou get 7 colors, 3 brush sizes, and full undo/redo support (up to 50 steps). When done, save the edited image back.

CLIPBOARD ENCRYPTION
${PLUGINS_ENABLED
  ? 'ClipToAll includes a handy clipboard encryption feature via the Clipboard Encryption plugin. While the capture overlay is visible, press the assigned encrypt or decrypt shortcut key to transform clipboard text using AES-256.\n\nShortcut keys are shown and configurable in Settings > Plugins. The encryption password is set in the plugin\'s Settings. This is useful for quickly encrypting sensitive text before pasting it somewhere.'
  : 'ClipToAll\'s core capture and sharing features work without any plugins. Plugins (custom scripts that run during capture, like the Clipboard Encryption plugin) are a Developer-ID-edition extension and are not included in this build.'}

TRAY ICON
${IS_MAC
  ? "ClipToAll lives in your menu bar (top-right of the screen).\n\\nLeft-click the menu bar icon → starts a capture (same as pressing the hotkey)\nRight-click the menu bar icon → opens a menu with Settings, About, and Exit"
  : 'ClipToAll lives in your system tray (bottom-right corner of the taskbar).\n\\nLeft-click the tray icon → starts a capture (same as pressing the hotkey)\nRight-click the tray icon → opens a menu with Settings, About, and Exit'}

KEYBOARD SHORTCUTS SUMMARY
\n${IS_MAC ? 'Cmd+X' : 'Alt+X'} — start capture (or your custom hotkey)
${IS_MAC ? 'Cmd+X, Cmd+X' : 'Alt+X, X'} — double-press to toggle mode during capture
Escape — cancel capture / hide Results window
${PLUGINS_ENABLED ? 'Plugin shortcut keys — shown in Settings > Plugins (e.g. encrypt, decrypt, ungroup)' : '(plugin shortcut keys are not available in this build)'}

FIRST-TIME SETUP
1. Launch ClipToAll — it appears in the ${IS_MAC ? 'menu bar' : 'system tray'}
2. Right-click ${IS_MAC ? 'menu bar icon' : 'tray'} → Settings
3. Choose your preferred Default Mode (Green: Copy Image is recommended to start)
4. If you want link sharing: select Google Drive or S3 under Image Storage and configure credentials
5. Optionally enable "${IS_MAC ? 'Open at Login' : 'Add to Autorun'}" so it ${IS_MAC ? 'starts when you log in' : 'starts with Windows'}
6. Click Save — you're ready to go!`,
    },
};

// Plugin-only developer reference for writing plugins. Whole entry is
// dead-stripped from the store build — see the file header for why.
const pluginHelpTexts: Record<string, { title: string; text: string }> = PLUGINS_ENABLED
  ? {
    guidePlugins: {
      title: 'Plugins — Developer Guide',
      text: `WHAT ARE PLUGINS?
Plugins extend ClipToAll with new skills that activate during the capture overlay. ${IS_MAC
  ? 'Each plugin is a standalone executable file (no extension; just the executable bit set) that communicates with ClipToAll via a simple JSON protocol over stdin/stdout. Plugins can do anything — encrypt clipboard, manipulate windows, call APIs, transform text — the possibilities are endless.'
  : 'Each plugin is a standalone .exe file that communicates with ClipToAll via a simple JSON protocol over stdin/stdout. Plugins can do anything — encrypt clipboard, manipulate windows, call APIs, transform text — the possibilities are endless.'}

HOW PLUGINS WORK
When ClipToAll starts, it launches each enabled plugin as a background process. The plugin sends a "hello" message describing its name, version, and available functions with keyboard shortcuts. When the user presses a plugin's shortcut key during the capture overlay, ClipToAll sends a "call" message and the plugin executes its function.
\nThe entire communication is line-delimited JSON — one JSON object per line, over stdin (commands to plugin) and stdout (responses from plugin).

PLUGIN LIFECYCLE
1. ClipToAll starts the plugin with --daemon flag
2. Plugin prints a hello JSON on stdout (must arrive within 20 seconds)
3. Plugin stays running, waiting for commands on stdin
4. On each hotkey press, ClipToAll sends a call command
5. Plugin executes the function and prints a result JSON
6. On shutdown, ClipToAll sends a shutdown command

THE HELLO MESSAGE
When started, your plugin must print exactly one JSON line:
\n{
  "name": "My Plugin",
  "version": "1.0.0",
  "description": "Short one-liner for catalogs",
  "instruction": "Detailed usage text shown in Settings UI",
  "settings_description": "What the user should configure",
  "settings_format": "{ \\"key\\": \\"example\\" }",
  "functions": [
    { "id": "do_thing", "label": "Do the thing", "default_key": "T" }
  ]
}
\n• name, version — displayed in the UI
• description — short one-liner summary
• instruction — full description shown in Settings (supports multi-line with \\n)
• settings_description / settings_format — optional; if present, a Settings button appears in the UI where users can enter JSON config. The format field is shown as a template
• functions — array of skills the plugin provides. Each has an id (internal), label (displayed), and default_key (single uppercase letter)

THE CALL COMMAND
When the user presses a plugin's shortcut key, your plugin receives:
\n{
  "type": "call",
  "function": "do_thing",
  "context": {
    "settings": "{ \\"key\\": \\"value\\" }"
  }
}
\n• function — the function id from your hello message
• settings — the JSON string the user entered in Settings (or empty if not configured)
\nNote: If your plugin needs the foreground window (e.g. to manipulate a specific window), ${IS_MAC
  ? 'on macOS use your preferred Cocoa/AppKit window-listing API inside your plugin after a brief delay (~100ms) to let the window system settle after the overlay closes.'
  : 'call GetForegroundWindow() directly inside your plugin after a brief delay (~100ms) to let the window system settle after the overlay closes.'}

THE RESULT RESPONSE
After executing, print one JSON line:
\n{ "type": "result", "status": "ok", "message": "Done!" }
\n• status — "ok" or "error"
• message — optional text (logged by the host)
• action — optional; recognized value: "admin_required" (prompts user to restart as Administrator)

THE SHUTDOWN COMMAND
ClipToAll sends this when exiting or when the plugin is disabled:
\n{ "type": "shutdown" }
\nYour plugin should clean up and exit. If it doesn't exit within 2 seconds, it will be forcefully terminated.

LANGUAGE CHOICE
You can write plugins in any language that can read stdin and write stdout:
\n• Rust — best performance, ${IS_MAC ? 'native macOS API access' : 'native Windows API access'}
• ${IS_MAC ? 'Swift — easy Cocoa interop, familiar for Mac devs' : 'C# / .NET — easy Win32 interop, familiar for Windows devs'}
• Python — quick prototyping ${IS_MAC ? '(bundle with PyInstaller or ship as a standalone binary)' : '(bundle with PyInstaller into .exe)'}
• Go, C++, Node.js — all work fine
\nThe only requirement: ${IS_MAC ? 'produce a standalone executable that the app can launch' : 'compile to a standalone .exe on Windows'}.

DEPLOYMENT
Place your plugin file in the plugins/ folder next to ClipToAll${IS_MAC ? '.app/Contents/MacOS/' : '.exe'}. On next launch (or when the user clicks the Plugins tab), ClipToAll discovers it automatically.

PLUGIN SETTINGS & SECURITY
If your plugin needs configuration (passwords, API keys, etc.), declare settings_description and settings_format in the hello message. The user enters a JSON string in the Settings UI. This string is encrypted ${IS_MAC ? 'in the macOS Keychain' : 'with Windows DPAPI'} before being saved to disk and is passed to your plugin in the call context.

RECOMMENDED CLI MODES
We recommend every plugin supports these command-line modes for developer ergonomics:
\n${IS_MAC ? 'plugin' : 'plugin.exe'}              Show help (name, version, functions, usage)
${IS_MAC ? 'plugin' : 'plugin.exe'} --help       Same as above
${IS_MAC ? 'plugin' : 'plugin.exe'} --daemon     Run as ClipToAll plugin (stdin/stdout JSON)
${IS_MAC ? 'plugin' : 'plugin.exe'} --call <json>   Execute one function and exit
${IS_MAC ? 'plugin' : 'plugin.exe'} --call @file.json   Read call JSON from a file
\nThe --call mode lets you test functions without running ClipToAll:
\n${IS_MAC ? 'plugin' : 'plugin.exe'} --call "{\\"type\\":\\"call\\",\\"function\\":\\"encrypt\\",\\"context\\":{\\"settings\\":\\"{\\\\\\"password\\\\\\":\\\\\\"test\\\\\\"}\\"}}"
\nOr create a test.json file and run:
\n${IS_MAC ? 'plugin' : 'plugin.exe'} --call @test.json

TIPS
• Keep stdout clean — only print hello and result JSON lines. Debug output should go to stderr or a log file
• Handle errors gracefully — if a function fails, return status "error" with a message
• Running without arguments should print help, not hang
• A plugin can expose multiple functions — each gets its own keyboard shortcut

SCRIPT PLUGINS (PYTHON & C#)
In addition to compiled ${IS_MAC ? 'native' : '.exe'} plugins, ClipToAll supports Python (.py) and C# (.cs) script plugins. These are single-file scripts placed in the plugins/ folder.
\nRequirements:
• Python scripts need Python installed and in PATH
• C# scripts need the .NET SDK installed and in PATH (uses "dotnet run")
\nScript Metadata:
Every script must have metadata in comment headers at the top of the file:
\nPython (single function):
# @plugin: My Script
# @description: What this script does
# @version: 1.0.0
# @mode: oneshot
# @key: R
# @label: Run My Script
\nPython (multiple functions):
# @plugin: My Encryption
# @function: encrypt, Encrypt Text, E
# @function: decrypt, Decrypt Text, D
\nFormat: @function: id, label, key
When @function tags are present, @key/@label are ignored.
\nC# uses // @ prefix instead of # @.
\nTwo Modes:
• oneshot (default) — script is run on demand when you click Run or press its shortcut key. Output is shown in the console panel below the plugin card.
• daemon — script runs as a long-lived process, same as exe plugins. Use --daemon flag. Follows the same JSON-RPC protocol.
\nOneshot Call Format:
When called via shortcut key during overlay, the script receives:
script.py --call '{"type":"call","function":"run","context":{"settings":"..."}}'
\nC# scripts use the dotnet separator:
dotnet run script.cs -- --call '{"type":"call","function":"run","context":{"settings":"..."}}'
\nThe script should print a JSON result line or plain text output.
\nCreating Scripts:
Use the "Add Script" button in the Plugins tab, or manually create a .py/.cs file in the plugins/ folder with proper metadata headers. C# scripts are pre-compiled on save for faster execution.`,
    },
  }
  : {};

// Combined help text map: base + plugin-specific entries. The plugin
// entries are dead-stripped from the store build (above ternary).
export const helpTexts: Record<string, { title: string; text: string }> = {
  ...baseHelpTexts,
  ...pluginHelpTexts,
};