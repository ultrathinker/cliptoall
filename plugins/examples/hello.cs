// @plugin: Hello CSharp
// @description: Minimal example plugin that prints a greeting
// @version: 1.0.0
// @mode: oneshot
// @key: H
// @label: Say Hello

// ClipToAll example plugin (C#).
//
// Mirrors the oneshot template in src/lib/plugin-templates.ts and also speaks
// the line-delimited JSON stdin/stdout protocol from plugins/PLUGIN-PROTOCOL.md
// so it can be enabled as a plugin.
//
// Usage (note the "--" separator before script arguments for `dotnet run`):
//   dotnet run hello.cs                       Print help
//   dotnet run hello.cs -- --call <json>      Oneshot: one JSON result line, exit
//   dotnet run hello.cs -- --call @file.json  Oneshot: read call JSON from a file
//   dotnet run hello.cs -- --daemon           Plugin mode: hello line, then stdin loop
//
// Manual JSON output is used to avoid depending on System.Text.Json reflection/trimming
// behavior when the plugin is later compiled/trimmed; hand-written JSON is always safe.

using System;

const string Name = "Hello CSharp";
const string Version = "1.0.0";
const string Description = "Minimal example plugin that prints a greeting";

static string EscapeJson(string s) => s.Replace("\\", "\\\\").Replace("\"", "\\\"")
    .Replace("\n", "\\n").Replace("\r", "\\r").Replace("\t", "\\t");

static void Emit(string jsonLine)
{
    Console.WriteLine(jsonLine);
    Console.Out.Flush();
}

static void Result(string message, string status = "ok")
{
    Emit($"{{\"type\":\"result\",\"status\":\"{status}\",\"message\":\"{EscapeJson(message)}\"}}");
}

static void RunDaemon()
{
    // Exactly one hello line first, then one result line per stdin command.
    Emit("{\"type\":\"hello\",\"name\":\"" + Name + "\",\"version\":\"" + Version + "\"" +
         ",\"description\":\"" + EscapeJson(Description) + "\"" +
         ",\"instruction\":\"Press the assigned shortcut key to print a hello message.\"" +
         ",\"settings_description\":\"\",\"settings_format\":\"\"" +
         ",\"functions\":[{\"id\":\"hello\",\"label\":\"Say Hello\",\"default_key\":\"H\"}]}");

    string? line;
    while ((line = Console.ReadLine()) != null)
    {
        line = line.Trim();
        if (string.IsNullOrEmpty(line)) continue;

        // Minimal hand-rolled JSON peek for the "type" and "function" fields —
        // avoids relying on System.Text.Json reflection (safe under trimming/AOT).
        var type = PeekJsonField(line, "type");
        if (type == "shutdown") break;
        if (type == "call")
        {
            var func = PeekJsonField(line, "function");
            Result($"Hello from {Name}! (function={func})");
        }
    }
}

static string PeekJsonField(string json, string field)
{
    // Tolerant reader for our own well-formed protocol lines: finds "field":"value".
    var key = "\"" + field + "\":\"";
    var at = json.IndexOf(key, StringComparison.Ordinal);
    if (at < 0) return "";
    at += key.Length;
    var end = json.IndexOf('"', at);
    return end < 0 ? "" : json.Substring(at, end - at);
}

static void PrintHelp()
{
    Console.WriteLine($"{Name} v{Version}");
    Console.WriteLine(Description);
    Console.WriteLine();
    Console.WriteLine("Usage (dotnet run <file.cs> -- <args>):");
    Console.WriteLine("  dotnet run hello.cs -- --daemon       Run as ClipToAll plugin (stdin/stdout JSON)");
    Console.WriteLine("  dotnet run hello.cs -- --call <json>  Oneshot: print a result line, exit");
    Console.WriteLine("  dotnet run hello.cs -- --call @file   Oneshot: read call JSON from a file");
    Console.WriteLine("  dotnet run hello.cs -- --help         Show this help");
}

if (args.Length >= 1 && args[0] == "--daemon")
{
    RunDaemon();
}
else if (args.Length >= 2 && args[0] == "--call")
{
    // arg is inline JSON or @<file> (ClipToAll passes @<file>).
    var arg = args[1];
    var callJson = arg.StartsWith("@") ? System.IO.File.ReadAllText(arg.Substring(1)) : arg;
    var func = PeekJsonField(callJson, "function");
    Result($"Hello from {Name}! (function={func})");
}
else
{
    PrintHelp();
}
