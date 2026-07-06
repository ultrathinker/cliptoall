# ClipToAll Plugin Protocol v1

Plugins are standalone executables that communicate with ClipToAll via **stdin/stdout** using **line-delimited JSON** (one JSON object per line).

## Lifecycle

1. ClipToAll launches the plugin: `plugin.exe --daemon`
2. Plugin sends a **hello** message (stdout)
3. ClipToAll reads hello, registers plugin functions, default keys, and self-describing metadata
4. Plugin enters a loop reading **commands** from stdin
5. Plugin writes **responses** to stdout
6. On app exit, ClipToAll sends **shutdown** command
7. Plugin exits cleanly

## Messages

### Hello (plugin → host)

Sent once immediately after startup. Must arrive within 20 seconds or plugin is considered invalid.

```json
{
  "type": "hello",
  "name": "My Plugin",
  "version": "1.0",
  "description": "Short one-line description",
  "instruction": "Detailed multi-line description of what the plugin does,\nhow to use it, and any important notes.",
  "settings_description": "What settings are needed and why.",
  "settings_format": "{\"api_key\": \"your-key-here\"}",
  "functions": [
    {
      "id": "my_action",
      "label": "Do Something",
      "default_key": "Z"
    }
  ]
}
```

**Required fields:**
- `name` — display name in the Plugins settings tab
- `version` — semver string
- `functions` — list of callable actions (see below)

**Optional self-describing fields:**
- `description` — short one-line description shown below plugin name
- `instruction` — detailed description shown in the "show more" modal. Can be multi-line. Describe what the plugin does, how to use it, algorithm details, etc.
- `settings_description` — explains what settings the plugin needs and why
- `settings_format` — example JSON showing the expected format. **Non-empty value means the plugin requires settings to work.** The host UI will block enabling the plugin without configured settings.

**Function fields:**
- `id` — unique identifier (snake_case)
- `label` — human-readable name shown in UI
- `default_key` — suggested single key (A-Z, 0-9). User can override in settings.

### Call (host → plugin)

Sent when user presses the assigned hotkey during the capture overlay.

```json
{
  "type": "call",
  "function": "my_action",
  "context": {
    "settings": "{\"api_key\": \"actual-user-key\"}"
  }
}
```

- `function` — matches one of the `id` values from hello
- `context.settings` — JSON string with user-configured settings for this plugin. Empty string if no settings configured. **The plugin is responsible for parsing this JSON internally.**

> **Note:** If your plugin needs the foreground window (e.g., to manipulate a specific window), call `GetForegroundWindow()` directly inside your plugin after a brief delay (~100ms) to let the window system settle after the overlay closes. The host does not pass window information — each plugin is responsible for finding its own context.

### Result (plugin → host)

Sent after processing a call.

```json
{"type": "result", "status": "ok"}
```

```json
{"type": "result", "status": "ok", "message": "Ungrouped successfully"}
```

```json
{
  "type": "result",
  "status": "error",
  "message": "Access denied",
  "action": "admin_required"
}
```

- `status` — `"ok"` or `"error"`
- `message` — optional, logged by host
- `action` — optional, tells host to take special action:
  - `"admin_required"` — show "restart as admin" dialog

### Shutdown (host → plugin)

Sent when ClipToAll is exiting.

```json
{"type": "shutdown"}
```

Plugin should clean up and exit within 2 seconds. After that, host will kill the process.

## Rules

1. **One JSON per line** — no pretty-printing, no multi-line JSON
2. **Flush stdout** after every write (important!)
3. **Don't write to stderr** — host ignores it, but it may cause issues
4. **Hello must be first** — no other output before hello
5. **Respond to every call** — host expects exactly one result per call
6. **Exit on shutdown** — or on stdin EOF (host process died)
7. **No UI of your own** — plugins run headless; use `action` field to request host UI

## Settings

Plugins declare their settings requirements via `settings_format` and `settings_description` in the hello message. The host provides a generic JSON editor in Settings → Plugins where users enter settings as JSON.

- Settings are stored encrypted on disk using Windows DPAPI (per-user scope)
- Settings are passed to the plugin as a raw JSON string in `context.settings` on every call
- The plugin must parse its own settings internally (e.g., extract `password`, `api_key`, etc.)
- If `settings_format` is non-empty, the host requires settings to be configured before the plugin can be enabled

## Recommended CLI Modes

Every plugin should support these command-line modes:

| Command | Behavior |
|---------|----------|
| `plugin.exe` | Print help (name, version, functions, settings, usage) |
| `plugin.exe --help` | Same as above |
| `plugin.exe --daemon` | Run as ClipToAll plugin (stdin/stdout JSON protocol) |
| `plugin.exe --call <json>` | Execute one function call, print result, exit |
| `plugin.exe --call @file.json` | Read call JSON from a file, execute, exit |

### `--help` / no arguments

Print human-readable information:

```
Clipboard Encryption v1.0.0
AES-256 encryption/decryption for clipboard text

Instantly encrypt or decrypt any text in your clipboard with AES-256...

Functions:
  encrypt      [E]  Encrypt clipboard
  decrypt      [D]  Decrypt clipboard

Settings: Requires an encryption password...
Format:   {"password": "your-password-here"}

Usage:
  plugin.exe --daemon            Run as ClipToAll plugin
  plugin.exe --call <json>       Test a function
  plugin.exe --call @file.json   Read call JSON from file
  plugin.exe --help              Show this help
```

**Important:** Running without arguments should **never hang** waiting for stdin. Always show help and exit.

### `--call` for testing

The `--call` mode executes a single function call using the same JSON format as the daemon protocol, then exits. This is the primary testing tool during development.

Inline JSON:
```bash
plugin.exe --call "{\"type\":\"call\",\"function\":\"encrypt\",\"context\":{\"settings\":\"{\\\"password\\\":\\\"test\\\"}\"}}"
```

From file (recommended — avoids quoting hell):
```bash
plugin.exe --call @test.json
```

Where `test.json`:
```json
{
  "type": "call",
  "function": "encrypt",
  "context": {
    "settings": "{\"password\": \"mypass\"}"
  }
}
```

The output is pretty-printed JSON (unlike daemon mode which uses compact one-line JSON):
```json
{
  "type": "result",
  "status": "ok",
  "message": "Clipboard encrypted"
}
```

## Building a Plugin

Any language that can read stdin and write stdout works. The existing plugins are written in Rust.

### Rust

Use the existing plugins as a starting point:

```bash
cd plugins/encryption-plugin  # or aumid-plugin
cargo build --release
```

### Any Other Language

The protocol is simple line-delimited JSON over stdin/stdout. You can write plugins in Python, Go, C#, Node.js, or any language:

1. Parse command-line arguments (`--help`, `--daemon`, `--call`)
2. `--daemon`: Print hello JSON to stdout, flush, then loop: read stdin → parse JSON → execute → print result → flush
3. `--call <json>`: Parse the JSON, execute the function, print result, exit
4. No args / `--help`: Print human-readable info and exit
5. On `"shutdown"` or stdin EOF → exit

### Deployment

Copy plugin files to the `plugins/` folder next to `ClipToAll.exe`. The host scans this folder for `.exe`, `.py`, `.cs`, and `.ps1` files.

Plugins are sorted by **file modification time** (most recently modified first), not alphabetically.

### C# Pre-compilation

C# scripts are pre-compiled when saved or enabled, eliminating the first-run delay. The dotnet SDK caches the compilation automatically.

C# scripts are compiled with `--property:WarningLevel=0` to suppress all compiler warnings that would otherwise clutter stdout and break JSON parsing.

---

## Script Plugins (Python, C# & PowerShell)

In addition to compiled `.exe` plugins, ClipToAll supports **Python** (`.py`), **C#** (`.cs`), and **PowerShell** (`.ps1`) script plugins. These are single-file scripts that follow the same protocol but are interpreted at runtime.

### Requirements

- **Python scripts**: Python must be installed and in PATH
- **C# scripts**: .NET 10+ SDK must be installed and in PATH (uses `dotnet run file.cs` single-file execution)
- **PowerShell scripts**: PowerShell must be available (built into Windows)

### Plugin Types Summary

| Type | Extension | Metadata Format | Runtime Command |
|------|-----------|-----------------|-----------------|
| Executable | `.exe` | JSON-RPC hello | Native |
| Python | `.py` | `# @key: value` | `python` |
| C# | `.cs` | `// @key: value` | `dotnet run` |
| PowerShell | `.ps1` | `# @key: value` | `powershell -NoProfile -File` |

**Note:** PowerShell runtime check uses `$PSVersionTable.PSVersion.ToString()` instead of `--version`.

### Script Metadata

Every script must have metadata in comment headers at the top of the file:

**Python** (`# @` prefix):
```python
# @plugin: My Script
# @description: What this script does
# @version: 1.0.0
# @mode: oneshot
# @key: R
# @label: Run My Script
# @instruction: Detailed usage text...
# @settings_description: What settings are needed
# @settings_format: {"api_key": "your-key-here"}
```

**C#** (`// @` prefix):
```csharp
// @plugin: My Script
// @description: What this script does
// @version: 1.0.0
// @mode: oneshot
// @key: R
// @label: Run My Script
```

**PowerShell** (`# @` prefix, same as Python):
```powershell
# @plugin: My Script
# @description: What this script does
# @version: 1.0.0
# @mode: oneshot
# @key: R
# @label: Run My Script
```

**PowerShell Template Example:**
```powershell
# @plugin: My PowerShell Plugin
# @description: Example PowerShell script plugin
# @version: 1.0.0
# @mode: oneshot
# @key: P
# @label: Run PowerShell Script

param(
    [string]$CallJson
)

# Parse the call JSON if provided
if ($CallJson) {
    $call = $CallJson | ConvertFrom-Json
    $settingsJson = $call.context.settings
    if ($settingsJson) {
        $settings = $settingsJson | ConvertFrom-Json
        # Use settings.$yourSetting
    }
}

# Your script logic here
Write-Output "Script executed successfully"

# Return result as JSON
$result = @{
    type = "result"
    status = "ok"
    message = "Operation completed"
} | ConvertTo-Json -Compress

Write-Output $result
```

**Required:** `@plugin` (name). All other fields are optional.

### Multiple Functions

By default, a script declares a single function via `@key` and `@label`. To declare multiple functions, use `@function:` tags instead:

```python
# @plugin: My Encryption
# @description: AES encryption/decryption
# @version: 1.0.0
# @function: encrypt, Encrypt Text, E
# @function: decrypt, Decrypt Text, D
```

Format: `@function: id, label, default_key`

When `@function` tags are present, `@key` and `@label` are ignored. Each function gets its own shortcut key in the overlay. The `function` field in the call JSON identifies which function was invoked.

### Modes

- **oneshot** (default) — script runs on demand, captures stdout, exits
- **daemon** — script runs as a long-lived process following the JSON-RPC protocol (same as exe plugins)

### Oneshot Execution

When invoked via shortcut key during the overlay, ClipToAll writes the call JSON
to a temporary file and passes it as **`--call @<file>`** — NOT inline. The call
context may contain **secret settings** (e.g. an encryption password), and a
process command line is readable by any of the user's other processes
(`Win32_Process.CommandLine`, Process Explorer), which would defeat the on-disk
DPAPI encryption. The temp file lives in the user's private temp directory and is
deleted as soon as the call returns.

```bash
# Python  (call.json holds {"type":"call","function":"run","context":{"settings":"..."}})
python script.py --call @C:\Users\you\AppData\Local\Temp\cta_call_xxxx.json

# C#
dotnet run script.cs -- --call @C:\...\cta_call_xxxx.json

# PowerShell
powershell -NoProfile -File script.ps1 --call @C:\...\cta_call_xxxx.json
```

Your script must accept **both** forms of the `--call` argument:
- `@<path>` — read the JSON from the file (this is what ClipToAll uses at runtime)
- inline JSON — convenient for manual testing

Resolve it with a one-liner, e.g. Python
`text = open(arg[1:]).read() if arg.startswith("@") else arg`, then parse.

The script should print either:
- A JSON result line: `{"type":"result","status":"ok","message":"Done"}`
- Or plain text output (treated as a message)

> **Backward compatibility:** oneshot scripts generated before this change parse
> only inline JSON and will not understand `@<file>`. Regenerate them from the
> current templates (or add the `@`-prefix handling above). Daemon/exe plugins are
> unaffected — they receive calls over stdin, never on the command line.

### Daemon Execution

Same as exe plugins but launched with:

```bash
# Python
python script.py --daemon

# C#
dotnet run script.cs -- --daemon

# PowerShell
powershell -NoProfile -File script.ps1 --daemon
```

### C# Pre-compilation

C# scripts are pre-compiled when saved or enabled, eliminating the first-run delay. The dotnet SDK caches the compilation automatically.
