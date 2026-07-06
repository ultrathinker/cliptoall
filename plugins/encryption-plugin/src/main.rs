/// ClipToAll Encryption Plugin
///
/// Provides AES-256-CBC encryption/decryption for clipboard text.
/// Follows the ClipToAll plugin protocol (stdin/stdout JSON).

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{self, BufRead, Write};

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

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
Algorithm: AES Encrypt(CBC) key 256, IV 128, key=sha256(Password), \
IV=sha256('ClipToAll') [first 16 bytes].\n\n\
Try online: https://the-x.cn/en-us/cryptography/Aes.aspx";
const SETTINGS_DESCRIPTION: &str = "Requires an encryption password. \
The password is hashed with SHA-256 to derive the AES-256 key.";
const SETTINGS_FORMAT: &str = r#"{"password": "your-password-here"}"#;

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

// ── Crypto helpers ──────────────────────────────────────────────

/// Derive AES-256 key from password: SHA-256(password).
fn derive_key(password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hasher.finalize().into()
}

/// Derive IV: first 16 bytes of SHA-256("ClipToAll").
fn derive_iv() -> [u8; 16] {
    let mut hasher = Sha256::new();
    hasher.update(b"ClipToAll");
    let hash: [u8; 32] = hasher.finalize().into();
    let mut iv = [0u8; 16];
    iv.copy_from_slice(&hash[..16]);
    iv
}

/// Encrypt plaintext with AES-256-CBC, return base64.
fn encrypt_text(plaintext: &str, password: &str) -> Result<String, String> {
    let key = derive_key(password);
    let iv = derive_iv();

    let plaintext_bytes = plaintext.as_bytes();

    // Buffer must be large enough for plaintext + padding (up to 16 extra bytes)
    let mut buf = vec![0u8; plaintext_bytes.len() + 16];
    buf[..plaintext_bytes.len()].copy_from_slice(plaintext_bytes);

    let ciphertext = Aes256CbcEnc::new(&key.into(), &iv.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plaintext_bytes.len())
        .map_err(|e| format!("Encryption failed: {}", e))?;

    Ok(BASE64.encode(ciphertext))
}

/// Decrypt base64 ciphertext with AES-256-CBC, return plaintext.
fn decrypt_text(base64_input: &str, password: &str) -> Result<String, String> {
    let key = derive_key(password);
    let iv = derive_iv();

    let mut ciphertext = BASE64
        .decode(base64_input.trim())
        .map_err(|e| format!("Invalid base64: {}", e))?;

    let plaintext_bytes = Aes256CbcDec::new(&key.into(), &iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut ciphertext)
        .map_err(|_| "Decryption failed: wrong password or corrupted data".to_string())?;

    String::from_utf8(plaintext_bytes.to_vec())
        .map_err(|e| format!("Decrypted data is not valid UTF-8: {}", e))
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
    // Parse settings JSON to extract password
    #[derive(Deserialize)]
    struct Settings {
        #[serde(default)]
        password: String,
    }

    let password = if context.settings.is_empty() {
        String::new()
    } else {
        match serde_json::from_str::<Settings>(&context.settings) {
            Ok(s) => s.password,
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

            match encrypt_text(&text, &password) {
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

            match decrypt_text(&text, &password) {
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
    for line in PLUGIN_INSTRUCTION.split("\\n") {
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
