// Plugin script templates and AI-instruction texts for the Settings script
// editor. Extracted verbatim from Settings.svelte (pure data/functions, no
// component reactivity) to shrink that component and keep one source of truth.

export function pythonTemplate(name: string): string {
    return `# @plugin: ${name}
# @description: Short description of what this script does
# @version: 1.0.0
# @mode: oneshot
# @key: R
# @label: Run ${name}

import sys
import json

def main():
    """Called when user presses the shortcut key during overlay."""
    print("Hello from ${name}!")

def handle_call(arg: str):
    """Handle --call invocation. arg is inline JSON or @<path> to a JSON file
    (ClipToAll passes @<file> so secret settings never appear on the command line)."""
    call_json = open(arg[1:], encoding="utf-8").read() if arg.startswith("@") else arg
    data = json.loads(call_json)
    func = data.get("function", "")
    settings = data.get("context", {}).get("settings", "")
    # Your logic here
    result = {"type": "result", "status": "ok", "message": f"Executed {func}"}
    print(json.dumps(result))

if __name__ == "__main__":
    if len(sys.argv) >= 3 and sys.argv[1] == "--call":
        handle_call(sys.argv[2])
    else:
        main()
`;
  }

export function csharpTemplate(name: string): string {
    return `// @plugin: ${name}
// @description: Short description of what this script does
// @version: 1.0.0
// @mode: oneshot
// @key: R
// @label: Run ${name}

using System;
using System.Text.Json;

if (args.Length >= 2 && args[0] == "--call")
{
    // arg is inline JSON, or @<path> to a JSON file (ClipToAll passes @<file>
    // so secret settings never appear on the command line).
    var arg = args[1];
    var callJson = arg.StartsWith("@") ? System.IO.File.ReadAllText(arg.Substring(1)) : arg;
    var data = JsonDocument.Parse(callJson);
    var func = data.RootElement.GetProperty("function").GetString();
    var result = new { type = "result", status = "ok", message = $"Executed {func}" };
    Console.WriteLine(JsonSerializer.Serialize(result));
}
else
{
    Console.WriteLine("Hello from ${name}!");
}
`;
  }

export function powershellTemplate(name: string): string {
    return `# @plugin: ${name}
# @description: Short description of what this script does
# @version: 1.0.0
# @mode: oneshot
# @key: R
# @label: Run ${name}

param([string]$call, [string]$daemon)

if ($call) {
    # $call is inline JSON, or @<path> to a JSON file (ClipToAll passes @<file>
    # so secret settings never appear on the command line).
    $callJson = if ($call.StartsWith("@")) { Get-Content -Raw -LiteralPath $call.Substring(1) } else { $call }
    $data = $callJson | ConvertFrom-Json
    $func = $data.function
    @{ type = "result"; status = "ok"; message = "Executed $func" } | ConvertTo-Json
}
else {
    Write-Host "Hello from ${name}!"
}
`;
  }

  // ── AI Instructions for script plugins ────────────────────────

const pythonAiInstructions = `TASK: Write a Python plugin script for ClipToAll desktop application.

WHAT IS A CLIPTOALL PLUGIN SCRIPT
- A single .py file placed in the plugins/ folder next to ClipToAll.exe
- ClipToAll launches it as a subprocess when the user presses a hotkey
- The script communicates via command-line arguments and stdout JSON
- Two modes: "oneshot" (spawned fresh each time) and "daemon" (long-running background process)

HOW THE SCRIPT IS LAUNCHED
  Oneshot:  python script.py --call @C:\\path\\call.json   (ClipToAll writes the call
            JSON — which may contain secret settings — to a temp file and passes @<file>
            so it never appears on the process command line; inline JSON also works for testing)
  Daemon:   python script.py --daemon

═══════════════════════════════════════════════════════════════
METADATA TAGS — comment headers at the top of the file
═══════════════════════════════════════════════════════════════

Every tag uses the format:  # @tagname: value
Tags MUST be at the very top of the file, before any code.
Parsing stops at the first non-comment, non-empty line.

Required:
  # @plugin: My Plugin Name

Optional:
  # @description: One-line description of what the plugin does
  # @version: 1.0.0
  # @mode: oneshot              (or "daemon" — default is "oneshot")
  # @key: R                     (single uppercase letter — hotkey to trigger)
  # @label: Run My Plugin       (human-readable label shown in UI)
  # @instruction: Detailed description shown in plugin info panel
  # @settings_description: What the user should enter in the settings field
  # @settings_format: { "api_key": "your-key-here", "timeout": 30 }

MULTIPLE FUNCTIONS (use instead of @key/@label when plugin has several actions):
  # @function: encrypt, Encrypt Text, E
  # @function: decrypt, Decrypt Text, D
  Format: # @function: function_id, Label, Key
  When @function: tags are present, @key and @label are ignored.

═══════════════════════════════════════════════════════════════
ONESHOT MODE — the simple and recommended mode
═══════════════════════════════════════════════════════════════

Your script receives:
  sys.argv[1] = "--call"
  sys.argv[2] = the call command — either inline JSON, or "@<path>" pointing to a
                JSON file. Resolve it like:
                  arg = sys.argv[2]
                  text = open(arg[1:], encoding="utf-8").read() if arg.startswith("@") else arg
                  data = json.loads(text)

Call JSON structure:
  {
    "type": "call",
    "function": "function_id",
    "context": {
      "settings": ""    ← JSON string with user settings, or empty string
    }
  }

Your script MUST print exactly one JSON line to stdout:
  Success: {"type": "result", "status": "ok", "message": "Done!"}
  Error:   {"type": "result", "status": "error", "message": "What went wrong"}

The "message" is shown to the user in the UI notification.

IMPORTANT: context.settings is a JSON STRING, not an object.
If settings_format is {"api_key": "xxx"}, then context.settings will be:
  '{"api_key": "xxx"}'    ← a string that you need to json.loads() again

═══════════════════════════════════════════════════════════════
DAEMON MODE — for plugins that need to stay running
═══════════════════════════════════════════════════════════════

On startup, print a hello JSON line to stdout:
  {
    "name": "My Plugin",
    "version": "1.0.0",
    "description": "What it does",
    "functions": [
      {"id": "run", "label": "Run", "default_key": "R"}
    ]
  }

Then loop reading lines from stdin. Each line is a JSON command:
  Call:     {"type": "call", "function": "run", "context": {"settings": ""}}
  Shutdown: {"type": "shutdown"}

For each call, print a result JSON line to stdout:
  {"type": "result", "status": "ok", "message": "Done!"}

On shutdown command, exit cleanly (sys.exit(0)).

ClipToAll reads the hello message with a 3-second timeout.
If no hello arrives in 20 seconds, the plugin is killed and marked as failed.

═══════════════════════════════════════════════════════════════
RULES — IMPORTANT
═══════════════════════════════════════════════════════════════

DO:
  - Keep stdout absolutely clean — only JSON result lines
  - Use print(..., file=sys.stderr) or sys.stderr.write() for debug output
  - Always return a result JSON, even if nothing happened
  - Handle missing or empty settings gracefully (settings can be "")
  - Add time.sleep(0.1) before interacting with foreground windows
    (ClipToAll overlay needs 100ms to close)
  - Flush stdout after printing (print() does this by default)
  - Use sys.exit(0) for clean exit in daemon mode

DON'T:
  - Don't use input() or sys.stdin.readline() in oneshot mode
  - Don't print anything before the hello message in daemon mode
  - Don't crash without printing a result — ClipToAll will hang waiting
  - Don't print multiple JSON lines per call (only one result per call)
  - Don't use logging module with default config (it writes to stderr, which is fine,
    but StreamHandler to stdout will corrupt the protocol)

═══════════════════════════════════════════════════════════════
COMPLETE ONESHOT EXAMPLE
═══════════════════════════════════════════════════════════════

# @plugin: URL Shortener
# @description: Shortens the URL from clipboard
# @version: 1.0.0
# @mode: oneshot
# @key: U
# @label: Shorten URL
# @settings_description: API key for the URL shortening service
# @settings_format: { "api_key": "your-api-key" }

import sys
import json

def handle_call(arg: str):
    # arg is inline JSON or "@<path>" to a JSON file (ClipToAll uses @<file>)
    call_json = open(arg[1:], encoding="utf-8").read() if arg.startswith("@") else arg
    data = json.loads(call_json)
    func = data.get("function", "")
    settings_str = data.get("context", {}).get("settings", "")

    # Parse settings (double JSON parsing!)
    api_key = ""
    if settings_str:
        try:
            settings = json.loads(settings_str)
            api_key = settings.get("api_key", "")
        except json.JSONDecodeError:
            pass

    if not api_key:
        result = {"type": "result", "status": "error", "message": "No API key configured"}
        print(json.dumps(result))
        return

    # Your logic here...
    result = {"type": "result", "status": "ok", "message": "URL shortened!"}
    print(json.dumps(result))

if __name__ == "__main__":
    if len(sys.argv) >= 3 and sys.argv[1] == "--call":
        handle_call(sys.argv[2])
    else:
        print("This script is a ClipToAll plugin. Run via ClipToAll.", file=sys.stderr)

═══════════════════════════════════════════════════════════════
COMPLETE DAEMON EXAMPLE
═══════════════════════════════════════════════════════════════

# @plugin: Clipboard Monitor
# @description: Monitors clipboard changes
# @version: 1.0.0
# @mode: daemon
# @function: start, Start Monitoring, S
# @function: stop, Stop Monitoring, T

import sys
import json

def main_daemon():
    # Print hello message (MUST be first line of stdout)
    hello = {
        "name": "Clipboard Monitor",
        "version": "1.0.0",
        "description": "Monitors clipboard changes",
        "functions": [
            {"id": "start", "label": "Start Monitoring", "default_key": "S"},
            {"id": "stop", "label": "Stop Monitoring", "default_key": "T"}
        ]
    }
    print(json.dumps(hello), flush=True)

    # Main loop: read commands from stdin
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            cmd = json.loads(line)
        except json.JSONDecodeError:
            continue

        if cmd.get("type") == "shutdown":
            break

        if cmd.get("type") == "call":
            func = cmd.get("function", "")
            # Handle each function
            if func == "start":
                message = "Monitoring started"
            elif func == "stop":
                message = "Monitoring stopped"
            else:
                message = f"Unknown function: {func}"

            result = {"type": "result", "status": "ok", "message": message}
            print(json.dumps(result), flush=True)

    sys.exit(0)

if __name__ == "__main__":
    if "--daemon" in sys.argv:
        main_daemon()
    elif len(sys.argv) >= 3 and sys.argv[1] == "--call":
        # Fallback oneshot handler (arg is inline JSON or @<file>)
        arg = sys.argv[2]
        text = open(arg[1:], encoding="utf-8").read() if arg.startswith("@") else arg
        data = json.loads(text)
        result = {"type": "result", "status": "ok", "message": "Done"}
        print(json.dumps(result))
    else:
        print("ClipToAll plugin.", file=sys.stderr)`;

const csharpAiInstructions = `TASK: Write a C# plugin script for ClipToAll desktop application.

WHAT IS A CLIPTOALL C# SCRIPT
- A single .cs file using top-level statements (.NET 8+)
- Placed in the plugins/ folder next to ClipToAll.exe
- ClipToAll pre-compiles it on save for faster execution
- No class declarations, no Main method — just top-level code
- The built-in "args" variable is available automatically

HOW THE SCRIPT IS LAUNCHED
  Oneshot:  dotnet run script.cs -- --call @C:\\path\\call.json   (ClipToAll writes the
            call JSON — which may contain secret settings — to a temp file and passes
            @<file> so it never appears on the process command line; inline JSON also works)
  Daemon:   dotnet run script.cs -- --daemon
  Note the "--" separator before script arguments!

═══════════════════════════════════════════════════════════════
METADATA TAGS — comment headers at the top of the file
═══════════════════════════════════════════════════════════════

Every tag uses the format:  // @tagname: value
Tags MUST be at the very top of the file, before any code.
Parsing stops at the first non-comment, non-empty line.

Required:
  // @plugin: My Plugin Name

Optional:
  // @description: One-line description of what the plugin does
  // @version: 1.0.0
  // @mode: oneshot              (or "daemon" — default is "oneshot")
  // @key: R                     (single uppercase letter — hotkey to trigger)
  // @label: Run My Plugin       (human-readable label shown in UI)
  // @instruction: Detailed description shown in plugin info panel
  // @settings_description: What the user should enter in the settings field
  // @settings_format: { "api_key": "your-key-here", "timeout": 30 }

MULTIPLE FUNCTIONS (use instead of @key/@label when plugin has several actions):
  // @function: encrypt, Encrypt Text, E
  // @function: decrypt, Decrypt Text, D
  Format: // @function: function_id, Label, Key
  When @function: tags are present, @key and @label are ignored.

═══════════════════════════════════════════════════════════════
ONESHOT MODE — the simple and recommended mode
═══════════════════════════════════════════════════════════════

Your script receives:
  args[0] = "--call"
  args[1] = the call command — inline JSON, or "@<path>" to a JSON file. Resolve like:
              var a = args[1];
              var text = a.StartsWith("@") ? System.IO.File.ReadAllText(a.Substring(1)) : a;
              var data = JsonDocument.Parse(text);

Call JSON structure:
  {
    "type": "call",
    "function": "function_id",
    "context": {
      "settings": ""    ← JSON string with user settings, or empty string
    }
  }

Your script MUST print exactly one JSON line to stdout:
  Success: {"type":"result","status":"ok","message":"Done!"}
  Error:   {"type":"result","status":"error","message":"What went wrong"}

IMPORTANT: context.settings is a JSON STRING, not an object.
You need to parse it separately if you need settings values.

═══════════════════════════════════════════════════════════════
DAEMON MODE — for plugins that need to stay running
═══════════════════════════════════════════════════════════════

On startup, print a hello JSON line to stdout:
  {
    "name": "My Plugin",
    "version": "1.0.0",
    "description": "What it does",
    "functions": [
      {"id": "run", "label": "Run", "default_key": "R"}
    ]
  }

Then loop reading lines from stdin. Each line is a JSON command:
  Call:     {"type":"call","function":"run","context":{"settings":""}}
  Shutdown: {"type":"shutdown"}

For each call, print a result JSON line to stdout.
On shutdown, exit cleanly.

ClipToAll reads the hello with a 20-second timeout.

═══════════════════════════════════════════════════════════════
CRITICAL: dotnet run AOT LIMITATIONS
═══════════════════════════════════════════════════════════════

"dotnet run <file.cs>" runs in AOT mode with reflection DISABLED.
This means:
  ✗ JsonSerializer.Serialize() — CRASHES at runtime (InvalidOperationException)
  ✗ JsonSerializer.Deserialize<T>() — CRASHES at runtime
  ✓ JsonDocument.Parse() — WORKS (read-only DOM, no reflection needed)
  ✓ Manual JSON strings — ALWAYS WORK

For OUTPUT (writing JSON): use string interpolation:
  Console.WriteLine($"{{\"type\":\"result\",\"status\":\"ok\",\"message\":\"{EscapeJson(msg)}\"}}");

For INPUT (reading JSON): use JsonDocument.Parse():
  var doc = JsonDocument.Parse(jsonString);
  var value = doc.RootElement.GetProperty("function").GetString();

Helper for escaping strings in manual JSON:
  string EscapeJson(string s) => s.Replace("\\\\", "\\\\\\\\").Replace("\"", "\\\\\"")
    .Replace("\\n", "\\\\n").Replace("\\r", "\\\\r").Replace("\\t", "\\\\t");

═══════════════════════════════════════════════════════════════
RULES — IMPORTANT
═══════════════════════════════════════════════════════════════

DO:
  - Keep stdout absolutely clean — only JSON result lines
  - Use Console.Error.WriteLine() for debug output
  - Always return a result JSON, even if nothing happened
  - Handle missing or empty settings gracefully
  - Add Thread.Sleep(100) before interacting with foreground windows
  - Use Console.Out.Flush() after writing in daemon mode
  - Add "using System.Text.Json;" for JsonDocument.Parse (reading JSON)
  - Add "#pragma warning disable" right after metadata comments (before any using statements)
    to suppress ALL compiler warnings — they pollute stdout and break JSON parsing
  - In daemon mode: call Environment.Exit(0) when Console.ReadLine() returns null
    (means host closed stdin — without this, process stays as zombie)
  - In daemon mode: send hello JSON BEFORE any heavy initialization

DON'T:
  - NEVER use JsonSerializer.Serialize() — crashes in AOT mode
  - NEVER use JsonSerializer.Deserialize<T>() — crashes in AOT mode
  - Don't use Console.ReadLine() in oneshot mode
  - Don't print anything before the hello message in daemon mode
  - Don't crash without printing a result — ClipToAll will hang
  - Don't wrap code in class/namespace — use top-level statements only
  - Don't use Console.WriteLine() for debug (it goes to stdout!)

═══════════════════════════════════════════════════════════════
COMPLETE ONESHOT EXAMPLE
═══════════════════════════════════════════════════════════════

// @plugin: Text Transform
// @description: Transforms clipboard text to uppercase
// @version: 1.0.0
// @mode: oneshot
// @key: U
// @label: Uppercase Text
// @settings_description: Optional prefix to add before the text
// @settings_format: { "prefix": "" }
#pragma warning disable

using System;
using System.Text.Json;

// Helper: escape strings for manual JSON output
string EscapeJson(string s) => s.Replace("\\\\", "\\\\\\\\").Replace("\"", "\\\\\\\"")
    .Replace("\\n", "\\\\n").Replace("\\r", "\\\\r").Replace("\\t", "\\\\t");

if (args.Length >= 2 && args[0] == "--call")
{
    // arg is inline JSON or @<file> (ClipToAll passes @<file>); JsonDocument.Parse
    // works in AOT mode (read-only DOM)
    var callArg = args[1];
    var callText = callArg.StartsWith("@") ? System.IO.File.ReadAllText(callArg.Substring(1)) : callArg;
    var doc = JsonDocument.Parse(callText);
    var root = doc.RootElement;
    var func = root.GetProperty("function").GetString() ?? "";
    var settingsStr = root.GetProperty("context").GetProperty("settings").GetString() ?? "";

    // Parse settings (double JSON parsing!)
    var prefix = "";
    if (!string.IsNullOrEmpty(settingsStr))
    {
        try
        {
            var settings = JsonDocument.Parse(settingsStr);
            prefix = settings.RootElement.TryGetProperty("prefix", out var p) ? p.GetString() ?? "" : "";
        }
        catch { }
    }

    // Your logic here...
    var msg = $"Text transformed with prefix '{prefix}'";
    // Manual JSON output — JsonSerializer.Serialize() crashes in AOT mode!
    Console.WriteLine($"{{\"type\":\"result\",\"status\":\"ok\",\"message\":\"{EscapeJson(msg)}\"}}");
}
else
{
    Console.Error.WriteLine("This is a ClipToAll plugin. Run via ClipToAll.");
}

═══════════════════════════════════════════════════════════════
COMPLETE DAEMON EXAMPLE
═══════════════════════════════════════════════════════════════

// @plugin: Counter Service
// @description: Counts how many times each function is called
// @version: 1.0.0
// @mode: daemon
// @function: increment, Increment Counter, I
// @function: reset, Reset Counter, R
#pragma warning disable

using System;
using System.Text.Json;

// Helper: escape strings for manual JSON output
string EscapeJson(string s) => s.Replace("\\\\", "\\\\\\\\").Replace("\"", "\\\\\\\"")
    .Replace("\\n", "\\\\n").Replace("\\r", "\\\\r").Replace("\\t", "\\\\t");

// Print hello IMMEDIATELY — before any heavy initialization!
// Manual JSON because JsonSerializer.Serialize() crashes in AOT mode.
Console.WriteLine("{\"name\":\"Counter Service\",\"version\":\"1.0.0\",\"description\":\"Counts function calls\",\"functions\":[{\"id\":\"increment\",\"label\":\"Increment Counter\",\"default_key\":\"I\"},{\"id\":\"reset\",\"label\":\"Reset Counter\",\"default_key\":\"R\"}]}");
Console.Out.Flush();

var counter = 0;

// Main loop: read commands from stdin
string? line;
while ((line = Console.ReadLine()) != null)
{
    line = line.Trim();
    if (string.IsNullOrEmpty(line)) continue;

    try
    {
        // JsonDocument.Parse works in AOT mode (read-only DOM)
        var cmd = JsonDocument.Parse(line).RootElement;
        var type = cmd.GetProperty("type").GetString();

        if (type == "shutdown") break;

        if (type == "call")
        {
            var func = cmd.GetProperty("function").GetString() ?? "";
            string msg;

            if (func == "increment")
            {
                counter++;
                msg = $"Counter: {counter}";
            }
            else if (func == "reset")
            {
                counter = 0;
                msg = "Counter reset to 0";
            }
            else
            {
                msg = $"Unknown function: {func}";
            }

            // Manual JSON output
            Console.WriteLine($"{{\"type\":\"result\",\"status\":\"ok\",\"message\":\"{EscapeJson(msg)}\"}}");
            Console.Out.Flush();
        }
    }
    catch (Exception ex)
    {
        Console.Error.WriteLine($"Error: {ex.Message}");
    }
}

// Host closed stdin or sent shutdown — exit immediately to avoid zombie process
Environment.Exit(0);`;

const powershellAiInstructions = `TASK: Write a PowerShell plugin script for ClipToAll desktop application.

WHAT IS A CLIPTOALL POWERSHELL SCRIPT
- A single .ps1 file placed in the plugins/ folder next to ClipToAll.exe
- ClipToAll launches it as a subprocess when the user presses a hotkey
- Uses param() block to receive command-line arguments
- Two modes: "oneshot" (spawned fresh each time) and "daemon" (long-running)

HOW THE SCRIPT IS LAUNCHED
  Oneshot:  powershell -NoProfile -File script.ps1 --call @C:\\path\\call.json   (ClipToAll
            writes the call JSON — which may contain secret settings — to a temp file and
            passes @<file> so it never appears on the process command line; inline JSON also works)
  Daemon:   powershell -NoProfile -File script.ps1 --daemon
  Note: -NoProfile flag is always used (no profile scripts loaded).

═══════════════════════════════════════════════════════════════
METADATA TAGS — comment headers at the top of the file
═══════════════════════════════════════════════════════════════

Every tag uses the format:  # @tagname: value
Tags MUST be at the very top of the file, BEFORE the param() block.
Parsing stops at the first non-comment, non-empty line.

Required:
  # @plugin: My Plugin Name

Optional:
  # @description: One-line description of what the plugin does
  # @version: 1.0.0
  # @mode: oneshot              (or "daemon" — default is "oneshot")
  # @key: R                     (single uppercase letter — hotkey to trigger)
  # @label: Run My Plugin       (human-readable label shown in UI)
  # @instruction: Detailed description shown in plugin info panel
  # @settings_description: What the user should enter in the settings field
  # @settings_format: { "api_key": "your-key-here", "timeout": 30 }

MULTIPLE FUNCTIONS (use instead of @key/@label when plugin has several actions):
  # @function: encrypt, Encrypt Text, E
  # @function: decrypt, Decrypt Text, D
  Format: # @function: function_id, Label, Key
  When @function: tags are present, @key and @label are ignored.

═══════════════════════════════════════════════════════════════
PARAM BLOCK — always declare these parameters
═══════════════════════════════════════════════════════════════

param(
    [string]$call,       # Receives the JSON call command in oneshot mode
    [switch]$daemon      # Set when running in daemon mode
)

IMPORTANT: The param() block MUST be the first non-comment code in the file.
Put metadata tags ABOVE the param() block.

═══════════════════════════════════════════════════════════════
ONESHOT MODE — the simple and recommended mode
═══════════════════════════════════════════════════════════════

When called, $call contains the JSON string:
  {
    "type": "call",
    "function": "function_id",
    "context": {
      "settings": ""    ← JSON string with user settings, or empty string
    }
  }

$call is inline JSON, or "@<path>" to a JSON file (ClipToAll passes @<file>).
Resolve then parse with ConvertFrom-Json:
  $callJson = if ($call.StartsWith("@")) { Get-Content -Raw -LiteralPath $call.Substring(1) } else { $call }
  $data = $callJson | ConvertFrom-Json
  $func = $data.function
  $settingsStr = $data.context.settings

Your script MUST output exactly one JSON line:
  @{ type = "result"; status = "ok"; message = "Done!" } | ConvertTo-Json -Compress

CRITICAL: Always use -Compress flag with ConvertTo-Json!
Without -Compress, PowerShell outputs multi-line JSON which breaks the protocol.

IMPORTANT: context.settings is a JSON STRING, not an object.
You need to parse it again: $settings = $settingsStr | ConvertFrom-Json

═══════════════════════════════════════════════════════════════
DAEMON MODE — for plugins that need to stay running
═══════════════════════════════════════════════════════════════

On startup, output a hello JSON line:
  @{
      name = "My Plugin"
      version = "1.0.0"
      description = "What it does"
      functions = @(
          @{ id = "run"; label = "Run"; default_key = "R" }
      )
  } | ConvertTo-Json -Compress -Depth 3

IMPORTANT: Use -Depth 3 (or higher) for nested objects!
Default depth is 2, which truncates the functions array content.

Then loop reading lines from stdin:
  while ($line = [Console]::In.ReadLine()) {
      $cmd = $line | ConvertFrom-Json
      if ($cmd.type -eq "shutdown") { break }
      # handle call...
      @{ type = "result"; status = "ok"; message = "Done" } | ConvertTo-Json -Compress
  }

ClipToAll reads the hello with a 20-second timeout.

═══════════════════════════════════════════════════════════════
RULES — IMPORTANT
═══════════════════════════════════════════════════════════════

DO:
  - Always use ConvertTo-Json -Compress for single-line output
  - Use -Depth 3 or more when serializing nested objects
  - Use Write-Error or $host.ui.WriteErrorLine() for debug output
  - Handle missing or empty settings ($data.context.settings can be "")
  - Add Start-Sleep -Milliseconds 100 before interacting with windows
  - Use [Console]::In.ReadLine() for stdin in daemon mode (not Read-Host)

DON'T:
  - Don't use Write-Host for result output (it goes to stdout and corrupts JSON)
    Instead, pipe to ConvertTo-Json or use Write-Output
  - Don't forget -Compress — multi-line JSON breaks the protocol!
  - Don't forget -Depth 3 — nested objects get truncated!
  - Don't use Read-Host in oneshot mode
  - Don't crash without outputting a result

═══════════════════════════════════════════════════════════════
COMPLETE ONESHOT EXAMPLE
═══════════════════════════════════════════════════════════════

# @plugin: File Launcher
# @description: Opens a configured file or URL
# @version: 1.0.0
# @mode: oneshot
# @key: L
# @label: Launch File
# @settings_description: Path to file or URL to open
# @settings_format: { "path": "C:\\\\path\\\\to\\\\file.txt" }

param([string]$call, [switch]$daemon)

if ($call) {
    # $call is inline JSON or @<file> (ClipToAll passes @<file>)
    $callJson = if ($call.StartsWith("@")) { Get-Content -Raw -LiteralPath $call.Substring(1) } else { $call }
    $data = $callJson | ConvertFrom-Json
    $func = $data.function
    $settingsStr = $data.context.settings

    # Parse settings (double JSON parsing!)
    $path = ""
    if ($settingsStr) {
        try {
            $settings = $settingsStr | ConvertFrom-Json
            $path = $settings.path
        } catch { }
    }

    if (-not $path) {
        @{ type = "result"; status = "error"; message = "No path configured" } | ConvertTo-Json -Compress
        return
    }

    try {
        Start-Process $path
        @{ type = "result"; status = "ok"; message = "Launched: $path" } | ConvertTo-Json -Compress
    } catch {
        @{ type = "result"; status = "error"; message = "Failed: $_" } | ConvertTo-Json -Compress
    }
}
else {
    Write-Error "This is a ClipToAll plugin. Run via ClipToAll."
}

═══════════════════════════════════════════════════════════════
COMPLETE DAEMON EXAMPLE
═══════════════════════════════════════════════════════════════

# @plugin: Notifier
# @description: Shows Windows toast notifications
# @version: 1.0.0
# @mode: daemon
# @function: notify, Show Notification, N
# @function: clear, Clear All, C

param([string]$call, [switch]$daemon)

if ($daemon) {
    # Print hello message (MUST be first output)
    @{
        name = "Notifier"
        version = "1.0.0"
        description = "Shows Windows toast notifications"
        functions = @(
            @{ id = "notify"; label = "Show Notification"; default_key = "N" }
            @{ id = "clear"; label = "Clear All"; default_key = "C" }
        )
    } | ConvertTo-Json -Compress -Depth 3

    # Main loop: read commands from stdin
    while ($line = [Console]::In.ReadLine()) {
        if (-not $line.Trim()) { continue }

        try {
            $cmd = $line | ConvertFrom-Json

            if ($cmd.type -eq "shutdown") { break }

            if ($cmd.type -eq "call") {
                $message = switch ($cmd.function) {
                    "notify" { "Notification shown!" }
                    "clear"  { "All notifications cleared" }
                    default  { "Unknown function: $($cmd.function)" }
                }
                @{ type = "result"; status = "ok"; message = $message } | ConvertTo-Json -Compress
            }
        } catch {
            $host.ui.WriteErrorLine("Error: $_")
        }
    }
}
elseif ($call) {
    $callJson = if ($call.StartsWith("@")) { Get-Content -Raw -LiteralPath $call.Substring(1) } else { $call }
    $data = $callJson | ConvertFrom-Json
    @{ type = "result"; status = "ok"; message = "Done" } | ConvertTo-Json -Compress
}
else {
    Write-Error "ClipToAll plugin."
}`;

export function getAiInstructions(lang: string): string {
    if (lang === 'python') return pythonAiInstructions;
    if (lang === 'csharp') return csharpAiInstructions;
    return powershellAiInstructions;
  }
