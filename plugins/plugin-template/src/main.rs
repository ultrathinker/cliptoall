/// ClipToAll Plugin Template
///
/// Copy this project to create your own plugin.
/// See PLUGIN-PROTOCOL.md for the full specification.
///
/// Usage:
///   1. Copy the entire `plugin-template` folder
///   2. Rename it and update Cargo.toml
///   3. Edit the PLUGIN_NAME, PLUGIN_VERSION, DESCRIPTION constants
///   4. Add your functions to the `functions()` list
///   5. Implement your logic in `handle_call()`
///   6. Build with `cargo build --release`
///   7. Place the .exe next to ClipToAll.exe

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

// ── Plugin metadata ─────────────────────────────────────────────
const PLUGIN_NAME: &str = "Hello World";
const PLUGIN_VERSION: &str = "1.0.0";
const PLUGIN_DESCRIPTION: &str = "A template plugin that demonstrates the protocol";

/// Return the list of functions this plugin provides.
fn functions() -> Vec<Function> {
    vec![
        Function {
            id: "greet".into(),
            label: "Say Hello".into(),
            default_key: "H".into(),
        },
    ]
}

/// Handle a function call from ClipToAll.
/// `function` is the function id (e.g. "greet").
/// `context.settings` is a JSON string with the user-configured settings for
/// this plugin (empty if none) — parse it yourself. The host does NOT pass
/// window info; if you need the foreground window, call GetForegroundWindow()
/// yourself after a brief delay (see PLUGIN-PROTOCOL.md).
/// Return a Result message to send back.
fn handle_call(function: &str, context: &CallContext) -> ResultMsg {
    match function {
        "greet" => {
            // Example: settings arrive as a JSON string in context.settings.
            eprintln!("Hello from plugin! settings = {}", context.settings);
            ResultMsg::ok(Some("Hello World!".into()))
        }
        _ => ResultMsg::error(format!("Unknown function: {}", function), None),
    }
}

// ── Protocol types (don't modify) ───────────────────────────────

#[derive(Serialize)]
struct HelloMsg {
    #[serde(rename = "type")]
    msg_type: String,
    name: String,
    version: String,
    description: String,
    functions: Vec<Function>,
}

#[derive(Serialize, Clone)]
struct Function {
    id: String,
    label: String,
    default_key: String,
}

#[derive(Deserialize)]
struct Command {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    function: String,
    #[serde(default)]
    context: CallContext,
}

#[derive(Deserialize, Default)]
struct CallContext {
    /// User-configured settings for this plugin, as a JSON string (empty if none).
    /// The plugin parses this itself. See PLUGIN-PROTOCOL.md.
    #[serde(default)]
    settings: String,
}

#[derive(Serialize)]
struct ResultMsg {
    #[serde(rename = "type")]
    msg_type: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<String>,
}

impl ResultMsg {
    fn ok(message: Option<String>) -> Self {
        Self { msg_type: "result".into(), status: "ok".into(), message, action: None }
    }
    fn error(message: String, action: Option<String>) -> Self {
        Self { msg_type: "result".into(), status: "error".into(), message: Some(message), action }
    }
}

// ── Main loop (don't modify) ────────────────────────────────────

fn main() {
    // Only run in daemon mode
    let args: Vec<String> = std::env::args().collect();
    if !args.iter().any(|a| a == "--daemon") {
        eprintln!("ClipToAll plugin: {}", PLUGIN_NAME);
        eprintln!("Run with --daemon to start in plugin mode.");
        eprintln!("Or place this .exe next to ClipToAll.exe and enable in Settings > Plugins.");
        return;
    }

    // Send hello
    let hello = HelloMsg {
        msg_type: "hello".into(),
        name: PLUGIN_NAME.into(),
        version: PLUGIN_VERSION.into(),
        description: PLUGIN_DESCRIPTION.into(),
        functions: functions(),
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &hello).ok();
    stdout.write_all(b"\n").ok();
    stdout.flush().ok();

    // Read commands from stdin
    let stdin = io::stdin().lock();
    for line in stdin.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break, // stdin closed — host exited
        };

        if line.trim().is_empty() {
            continue;
        }

        let cmd: Command = match serde_json::from_str(&line) {
            Ok(c) => c,
            Err(e) => {
                let r = ResultMsg::error(format!("Invalid JSON: {}", e), None);
                serde_json::to_writer(&mut stdout, &r).ok();
                stdout.write_all(b"\n").ok();
                stdout.flush().ok();
                continue;
            }
        };

        match cmd.msg_type.as_str() {
            "call" => {
                let result = handle_call(&cmd.function, &cmd.context);
                serde_json::to_writer(&mut stdout, &result).ok();
                stdout.write_all(b"\n").ok();
                stdout.flush().ok();
            }
            "shutdown" => {
                break;
            }
            _ => {
                let r = ResultMsg::error(format!("Unknown command: {}", cmd.msg_type), None);
                serde_json::to_writer(&mut stdout, &r).ok();
                stdout.write_all(b"\n").ok();
                stdout.flush().ok();
            }
        }
    }
}
