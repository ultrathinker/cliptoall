/// ClipToAll Plugin — AUMID Ungrouper
///
/// Provides two functions:
///   ungroup — sets a unique AUMID on the foreground window so it stands alone on the taskbar
///   regroup — clears the AUMID (VT_EMPTY) to restore default taskbar grouping
///
/// Build:
///   cargo build --release
/// Place the resulting .exe next to ClipToAll.exe and enable in Settings > Plugins.

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};

// ── Plugin metadata ─────────────────────────────────────────────
const PLUGIN_NAME: &str = "AUMID Ungrouper";
const PLUGIN_VERSION: &str = "1.0.0";
const PLUGIN_DESCRIPTION: &str = "Ungroup/regroup windows on the Windows taskbar";
const PLUGIN_INSTRUCTION: &str = "Separate any window into its own taskbar icon so it's no longer grouped \
with other windows of the same app — or merge it back into the group.\n\n\
How it works: sets a unique Application User Model ID (AUMID) on the foreground window, \
which tells Windows to treat it as a separate application in the taskbar.\n\n\
While the capture overlay is visible, press the assigned shortcut key to ungroup \
or regroup the foreground window. Shortcut keys are shown and configurable \
in Settings > Plugins.\n\n\
Note: If the target window is running elevated (as admin) and ClipToAll is not, \
you will be prompted to restart as admin.";
const SETTINGS_DESCRIPTION: &str = "";
const SETTINGS_FORMAT: &str = "";

// System.AppUserModel.ID property key
// fmtid: {9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}, pid: 5
const PKEY_APPUSERMODEL_ID: windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY =
    windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
        fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
        pid: 5,
    };

/// Return the list of functions this plugin provides.
fn functions() -> Vec<Function> {
    vec![
        Function {
            id: "ungroup".into(),
            label: "Ungroup window".into(),
            default_key: "A".into(),
        },
        Function {
            id: "regroup".into(),
            label: "Regroup window".into(),
            default_key: "G".into(),
        },
    ]
}

/// Get the current foreground window handle and title.
/// Sleeps briefly to let the window system settle after overlay closes.
fn get_foreground_hwnd_and_title() -> (HWND, String) {
    std::thread::sleep(std::time::Duration::from_millis(100));
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut buf);
        let title = if len > 0 {
            String::from_utf16_lossy(&buf[..len as usize])
        } else {
            "(no title)".into()
        };
        (hwnd, title)
    }
}

/// Handle a function call from ClipToAll.
fn handle_call(function: &str, _context: &CallContext) -> ResultMsg {
    let (hwnd, title) = get_foreground_hwnd_and_title();
    let hwnd_raw = hwnd.0 as i64;

    match function {
        "ungroup" => {
            if is_window_elevated_and_we_are_not(hwnd) {
                return ResultMsg::error(
                    "Access denied - target is elevated".into(),
                    Some("admin_required".into()),
                );
            }
            let unique_id = format!("ClipToAll.Alone.{}", hwnd_raw);
            match set_aumid(hwnd, Some(&unique_id)) {
                Ok(()) => ResultMsg::ok(Some(format!(
                    "Ungrouped HWND {} (\"{}\")",
                    hwnd_raw, title
                ))),
                Err(e) => {
                    let msg = format!("{}", e);
                    if msg.contains("0x80070005") {
                        ResultMsg::error(
                            "Access denied - target is elevated".into(),
                            Some("admin_required".into()),
                        )
                    } else {
                        ResultMsg::error(
                            format!("Failed to set AUMID: {}", msg),
                            None,
                        )
                    }
                }
            }
        }
        "regroup" => {
            if is_window_elevated_and_we_are_not(hwnd) {
                return ResultMsg::error(
                    "Access denied - target is elevated".into(),
                    Some("admin_required".into()),
                );
            }
            match set_aumid(hwnd, None) {
                Ok(()) => ResultMsg::ok(Some(format!(
                    "Regrouped HWND {} (\"{}\")",
                    hwnd_raw, title
                ))),
                Err(e) => {
                    let msg = format!("{}", e);
                    if msg.contains("0x80070005") {
                        ResultMsg::error(
                            "Access denied - target is elevated".into(),
                            Some("admin_required".into()),
                        )
                    } else {
                        ResultMsg::error(
                            format!("Failed to clear AUMID: {}", msg),
                            None,
                        )
                    }
                }
            }
        }
        _ => ResultMsg::error(format!("Unknown function: {}", function), None),
    }
}

// ── AUMID implementation ─────────────────────────────────────────

/// Set or clear the AUMID property on a window.
/// `aumid = Some(id)` sets a unique ID; `aumid = None` clears it (VT_EMPTY, restores grouping).
fn set_aumid(hwnd: HWND, aumid: Option<&str>) -> windows::core::Result<()> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd)?;
        let pv: PROPVARIANT = match aumid {
            Some(id) => PROPVARIANT::from(id),
            None => PROPVARIANT::default(), // VT_EMPTY
        };
        store.SetValue(&PKEY_APPUSERMODEL_ID, &pv)?;
        Ok(())
    }
}

/// Check if a process token is elevated.
fn is_token_elevated(token: HANDLE) -> bool {
    unsafe {
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        ok.is_ok() && elevation.TokenIsElevated != 0
    }
}

/// Check if the current process is running elevated (admin).
fn is_self_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_ok() {
            let result = is_token_elevated(token);
            let _ = CloseHandle(token);
            result
        } else {
            false
        }
    }
}

/// Check if the window's owning process is elevated AND we are not.
/// Returns true only when target is elevated and plugin is not — meaning the
/// operation would fail with Access Denied.
fn is_window_elevated_and_we_are_not(hwnd: HWND) -> bool {
    if is_self_elevated() {
        return false; // we are admin too — no problem
    }
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return false;
        }
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
        match proc {
            Ok(proc_handle) => {
                let mut token = HANDLE::default();
                let result = if OpenProcessToken(proc_handle, TOKEN_QUERY, &mut token).is_ok() {
                    let elevated = is_token_elevated(token);
                    let _ = CloseHandle(token);
                    elevated
                } else {
                    // Cannot open token — assume elevated
                    true
                };
                let _ = CloseHandle(proc_handle);
                result
            }
            Err(_) => {
                // Cannot open process — assume elevated
                true
            }
        }
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
    #[allow(dead_code)]
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
    // Print instruction with actual newlines
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
    println!(r#"  {exe} --call "{{\""type\"":\""call\"",\""function\"":\""{}\"",\""context\"":{{}}}}""#, first_fn.id);
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
    // Send hello
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

    // Read commands from stdin
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
