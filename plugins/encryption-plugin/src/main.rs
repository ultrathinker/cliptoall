//! ClipToAll Encryption Plugin
//!
//! Provides AES-256-CBC encryption/decryption for clipboard text.
//! Follows the ClipToAll plugin protocol (stdin/stdout JSON).

mod crypto;

use crypto::Scheme;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

// ── Plugin metadata ─────────────────────────────────────────────
const PLUGIN_NAME: &str = "Clipboard Encryption";
const PLUGIN_VERSION: &str = "1.0.0";
const PLUGIN_DESCRIPTION: &str = "AES-256 encryption/decryption for clipboard text";
const PLUGIN_INSTRUCTION: &str = "Instantly encrypt or decrypt any text in your clipboard with AES-256 — \
perfect for securely sharing passwords, keys, or sensitive data through \
any messenger or email.\n\n\
While the capture overlay is visible, press the assigned shortcut key to encrypt \
or decrypt the text currently in your clipboard. Shortcut keys are shown \
and configurable in Settings > Plugins.\n\n\
Two schemes are available (set \"scheme\" in Settings):\n\
• \"strong\" (default) — per-message random salt + PBKDF2-HMAC-SHA256 (600k rounds) + \
AES-256-GCM (authenticated). Recommended.\n\
• \"legacy\" — AES-256-CBC, key=sha256(Password), IV=sha256('ClipToAll') [first 16 \
bytes]. Opt-in interop mode: its output is decryptable by the .NET/old version of \
ClipToAll. Output is base64 of (raw ciphertext bytes) with PKCS#7 padding.\n\n\
Decryption auto-detects both schemes, so anything encrypted earlier keeps working.";
const SETTINGS_DESCRIPTION: &str = "Requires an encryption password. Optional \
\"scheme\": \"strong\" (default — PBKDF2 + AES-256-GCM, authenticated) or \
\"legacy\" (AES-256-CBC, only for interop with the .NET/old version of ClipToAll). \
Decryption auto-detects the scheme, so previously encrypted values always work.";
const SETTINGS_FORMAT: &str = r#"{"password": "your-password-here", "scheme": "strong"}"#;

/// Return the list of functions this plugin provides.
fn functions() -> Vec<Function> {
    vec![
        Function {
            id: "encrypt".into(),
            label: "Encrypt clipboard".into(),
            default_key: "E".into(),
        },
        Function {
            id: "decrypt".into(),
            label: "Decrypt clipboard".into(),
            default_key: "D".into(),
        },
    ]
}

// ── Clipboard helpers ───────────────────────────────────────────

fn read_clipboard() -> Result<String, String> {
    clipboard_win::get_clipboard_string().map_err(|e| format!("Failed to read clipboard: {}", e))
}

fn write_clipboard(text: &str) -> Result<(), String> {
    clipboard_win::set_clipboard_string(text)
        .map_err(|e| format!("Failed to write clipboard: {}", e))
}

// ── Call handler ────────────────────────────────────────────────

/// Handle a function call from ClipToAll.
fn handle_call(function: &str, context: &CallContext) -> ResultMsg {
    // Parse settings JSON to extract password and (optional) scheme.
    #[derive(Deserialize)]
    struct Settings {
        #[serde(default)]
        password: String,
        #[serde(default)]
        scheme: String,
    }

    let (password, scheme) = if context.settings.is_empty() {
        (String::new(), Scheme::Strong)
    } else {
        match serde_json::from_str::<Settings>(&context.settings) {
            Ok(s) => (s.password, Scheme::from_setting(&s.scheme)),
            Err(e) => return ResultMsg::error(
                format!("Invalid settings JSON: {}", e), None,
            ),
        }
    };

    if password.is_empty() {
        return ResultMsg::error(
            "No encryption password configured. Set it in Settings > Plugins.".into(),
            None,
        );
    }

    match function {
        "encrypt" => {
            let text = match read_clipboard() {
                Ok(t) => t,
                Err(e) => return ResultMsg::error(e, None),
            };

            if text.is_empty() {
                return ResultMsg::error("Clipboard is empty".into(), None);
            }

            match crypto::encrypt_text(&text, &password, scheme) {
                Ok(encrypted) => match write_clipboard(&encrypted) {
                    Ok(()) => ResultMsg::ok(Some("Clipboard encrypted".into())),
                    Err(e) => ResultMsg::error(e, None),
                },
                Err(e) => ResultMsg::error(e, None),
            }
        }
        "decrypt" => {
            let text = match read_clipboard() {
                Ok(t) => t,
                Err(e) => return ResultMsg::error(e, None),
            };

            if text.is_empty() {
                return ResultMsg::error("Clipboard is empty".into(), None);
            }

            match crypto::decrypt_text(&text, &password) {
                Ok(decrypted) => match write_clipboard(&decrypted) {
                    Ok(()) => ResultMsg::ok(Some("Clipboard decrypted".into())),
                    Err(e) => ResultMsg::error(e, None),
                },
                Err(e) => ResultMsg::error(e, None),
            }
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
    instruction: String,
    settings_description: String,
    settings_format: String,
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

// ── CLI ─────────────────────────────────────────────────────────

fn print_help() {
    let exe = std::env::current_exe().ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "plugin.exe".into());

    println!("{} v{}", PLUGIN_NAME, PLUGIN_VERSION);
    println!("{}", PLUGIN_DESCRIPTION);
    println!();
    for line in PLUGIN_INSTRUCTION.lines() {
        println!("{}", line.trim());
    }
    println!();
    println!("Functions:");
    for f in functions() {
        println!("  {:12} [{}]  {}", f.id, f.default_key, f.label);
    }
    println!();
    if !SETTINGS_DESCRIPTION.is_empty() {
        println!("Settings: {}", SETTINGS_DESCRIPTION);
        println!("Format:   {}", SETTINGS_FORMAT);
    } else {
        println!("Settings: not required");
    }
    println!();
    println!("Usage:");
    println!("  {} --daemon            Run as ClipToAll plugin (stdin/stdout JSON)", exe);
    println!("  {} --call <json>       Test a function with a JSON call command", exe);
    println!("  {} --call @file.json   Read call JSON from a file", exe);
    println!("  {} --help              Show this help", exe);
    println!();
    println!("Example:");
    let first_fn = &functions()[0];
    println!(r#"  {exe} --call "{{\""type\"":\""call\"",\""function\"":\""{}\"",\""context\"":{{\"settings\"":\""{{\\\"password\\\":\\\"test\\\"}}\""}}}}""#, first_fn.id);
}

fn run_call(json_str: &str) {
    let cmd: Command = match serde_json::from_str(json_str) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Invalid JSON: {}", e);
            eprintln!("Expected format: {{\"type\":\"call\",\"function\":\"<id>\",\"context\":{{...}}}}");
            std::process::exit(1);
        }
    };

    if cmd.msg_type != "call" {
        eprintln!("Expected type \"call\", got \"{}\"", cmd.msg_type);
        std::process::exit(1);
    }

    let result = handle_call(&cmd.function, &cmd.context);
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}

fn run_daemon() {
    let hello = HelloMsg {
        msg_type: "hello".into(),
        name: PLUGIN_NAME.into(),
        version: PLUGIN_VERSION.into(),
        description: PLUGIN_DESCRIPTION.into(),
        instruction: PLUGIN_INSTRUCTION.into(),
        settings_description: SETTINGS_DESCRIPTION.into(),
        settings_format: SETTINGS_FORMAT.into(),
        functions: functions(),
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &hello).ok();
    stdout.write_all(b"\n").ok();
    stdout.flush().ok();

    let stdin = io::stdin().lock();
    for line in stdin.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
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
            "shutdown" => break,
            _ => {
                let r = ResultMsg::error(format!("Unknown command: {}", cmd.msg_type), None);
                serde_json::to_writer(&mut stdout, &r).ok();
                stdout.write_all(b"\n").ok();
                stdout.flush().ok();
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || args[1] == "--help" || args[1] == "-h" {
        print_help();
        return;
    }

    match args[1].as_str() {
        "--daemon" => run_daemon(),
        "--call" => {
            if args.len() < 3 {
                eprintln!("Missing argument. Usage: {} --call <json>  or  {} --call @file.json", args[0], args[0]);
                std::process::exit(1);
            }
            let json_str = if args[2].starts_with('@') {
                let path = &args[2][1..];
                std::fs::read_to_string(path).unwrap_or_else(|e| {
                    eprintln!("Failed to read {}: {}", path, e);
                    std::process::exit(1);
                })
            } else {
                args[2..].join(" ")
            };
            run_call(json_str.trim());
        }
        other => {
            eprintln!("Unknown option: {}", other);
            eprintln!("Run with --help for usage information.");
            std::process::exit(1);
        }
    }
}
