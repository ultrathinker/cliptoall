# ClipToAll example plugins

Minimal, runnable example plugins that speak the ClipToAll plugin protocol from
[`../PLUGIN-PROTOCOL.md`](../PLUGIN-PROTOCOL.md). Each one mirrors the oneshot
templates in [`../../src/lib/plugin-templates.ts`](../../src/lib/plugin-templates.ts)
and additionally supports `--daemon` mode, so it can be dropped into the
`plugins/` folder next to `ClipToAll.exe` and enabled from **Settings → Plugins**.

| File | Runtime | How ClipToAll launches it |
|------|---------|---------------------------|
| [`hello.py`](hello.py) | Python 3 | `python hello.py --daemon` / `--call @<file>` |
| [`hello.cs`](hello.cs) | .NET 10+ SDK | `dotnet run hello.cs -- --daemon` / `-- --call @<file>` |
| [`hello.ps1`](hello.ps1) | PowerShell (built into Windows) | `powershell -NoProfile -File hello.ps1 --daemon` / `--call @<file>` |

## What they do

Each script exposes one function — `hello` (key **H**) — that prints a greeting.
In oneshot mode it emits **exactly one** JSON result line and exits; in daemon
mode it emits exactly one `hello` JSON line at startup, then reads
line-delimited JSON commands from stdin and writes one result line per call.

## Manual testing

```bash
# Python — inline JSON
python hello.py --call "{\"type\":\"call\",\"function\":\"hello\",\"context\":{\"settings\":\"\"}}"

# Python — daemon handshake
python hello.py --daemon
# stdout: {"type":"hello","name":"Hello Python",...}
# stdin:  {"type":"call","function":"hello","context":{"settings":""}}
# stdout: {"type":"result","status":"ok","message":"Hello from Hello Python! (function=hello)"}

# C# — note the "--" separator before script args for `dotnet run`
dotnet run hello.cs -- --call "{\"type\":\"call\",\"function\":\"hello\",\"context\":{\"settings\":\"\"}}"

# PowerShell
powershell -NoProfile -File hello.ps1 -call '{\"type\":\"call\",\"function\":\"hello\",\"context\":{\"settings\":\"\"}}'
```

## Deploying

Copy the script(s) into the `plugins/` folder next to `ClipToAll.exe`. ClipToAll
scans that folder for `.py`, `.cs`, and `.ps1` files, parses the `# @plugin:`
/ `// @plugin:` metadata header, and registers the declared function(s).

See [`../PLUGIN-PROTOCOL.md`](../PLUGIN-PROTOCOL.md) for the full protocol spec.
