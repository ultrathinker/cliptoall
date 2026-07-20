# @plugin: Hello Python
# @description: Minimal example plugin that prints a greeting
# @version: 1.0.0
# @mode: oneshot
# @key: H
# @label: Say Hello

"""ClipToAll example plugin (Python).

Mirrors the oneshot template in src/lib/plugin-templates.ts and also speaks
the line-delimited JSON stdin/stdout protocol from plugins/PLUGIN-PROTOCOL.md
so it can be enabled as a plugin.

Usage:
  python hello.py                       Print help
  python hello.py --call <json>         Oneshot: emit one JSON result line, exit
  python hello.py --call @file.json     Oneshot: read call JSON from a file
  python hello.py --daemon              Plugin mode: hello line, then stdin loop
"""

import json
import sys

NAME = "Hello Python"
VERSION = "1.0.0"
DESCRIPTION = "Minimal example plugin that prints a greeting"
FUNCTIONS = [{"id": "hello", "label": "Say Hello", "default_key": "H"}]


def emit(obj):
    """Write exactly one JSON line and flush (the protocol is line-delimited)."""
    print(json.dumps(obj), flush=True)


def result(message, status="ok"):
    emit({"type": "result", "status": status, "message": message})


def handle_call(arg):
    """Oneshot entry point. `arg` is inline JSON or `@<path>` to a JSON file
    (ClipToAll passes @<file> so secret settings never appear on the command line)."""
    call_json = open(arg[1:], encoding="utf-8").read() if arg.startswith("@") else arg
    data = json.loads(call_json)
    func = data.get("function", "")
    result(f"Hello from {NAME}! (function={func})")


def run_daemon():
    """Plugin mode: emit exactly one hello line, then loop over stdin commands,
    emitting one result line per call. Exits on `shutdown` or stdin EOF."""
    emit({
        "type": "hello",
        "name": NAME,
        "version": VERSION,
        "description": DESCRIPTION,
        "instruction": "Press the assigned shortcut key to print a hello message.",
        "settings_description": "",
        "settings_format": "",
        "functions": FUNCTIONS,
    })
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
            result(f"Hello from {NAME}! (function={cmd.get('function', '')})")


def print_help():
    print(f"{NAME} v{VERSION}")
    print(DESCRIPTION)
    print()
    print("Usage:")
    print("  python hello.py --daemon       Run as ClipToAll plugin (stdin/stdout JSON)")
    print("  python hello.py --call <json>  Oneshot: print a result line, exit")
    print("  python hello.py --call @file   Oneshot: read call JSON from a file")
    print("  python hello.py --help         Show this help")


if __name__ == "__main__":
    args = sys.argv[1:]
    if not args or args[0] in ("--help", "-h"):
        print_help()
    elif args[0] == "--daemon":
        run_daemon()
    elif args[0] == "--call" and len(args) >= 2:
        handle_call(args[1])
    else:
        print_help()
