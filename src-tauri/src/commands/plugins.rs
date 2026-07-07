use crate::plugins::{DiscoveredPlugin, PluginConfig, PluginManagerState, PluginManager, PluginType};
use tauri::State;

/// Scan the plugins/ folder for exe, .py, .cs, and .ps1 files.
#[tauri::command]
pub fn discover_plugins(window: tauri::Window) -> Vec<DiscoveredPlugin> {
    // NOT read-only: probing exe plugins SPAWNS every .exe in the plugins/ folder
    // (even disabled ones). Only the Settings UI legitimately enumerates plugins,
    // so refuse other windows — a compromised Results overlay must not be able to
    // trigger execution of those binaries. Return empty rather than erroring since
    // the return type is a plain list.
    if crate::commands::require_main_window(&window).is_err() {
        return Vec::new();
    }
    // Probe exe plugins in PARALLEL — each probe can take up to the hello
    // timeout, so a couple of uncooperative exes would otherwise serialize into
    // a long UI freeze when opening the Plugins tab (3.13).
    let handles: Vec<_> = PluginManager::discover_exe_files()
        .into_iter()
        .map(|p| {
            std::thread::spawn(move || {
                let path_str = p.to_string_lossy().to_string();
                crate::log(&format!("plugins: probing exe {}", path_str));
                PluginManager::probe_plugin(&path_str)
            })
        })
        .collect();
    let mut plugins: Vec<DiscoveredPlugin> = handles
        .into_iter()
        .filter_map(|h| h.join().ok())
        .collect();

    // Discover script plugins (metadata from file, no spawning)
    let scripts = crate::plugins::discover_script_files();
    plugins.extend(scripts);

    // Sort all plugins by file modification time (oldest first = chronological order)
    plugins.sort_by(|a, b| {
        let mtime_a = std::fs::metadata(&a.path).and_then(|m| m.modified()).ok();
        let mtime_b = std::fs::metadata(&b.path).and_then(|m| m.modified()).ok();
        mtime_a.cmp(&mtime_b)
    });

    plugins
}

/// Save plugin configurations and restart plugins accordingly.
#[tauri::command]
pub fn apply_plugin_config(
    window: tauri::Window,
    state: State<PluginManagerState>,
    configs: Vec<PluginConfig>,
) -> Result<(), String> {
    crate::commands::require_main_window(&window)?;
    for cfg in &configs {
        ensure_in_plugins_dir(std::path::Path::new(&cfg.path))
            .map_err(|e| format!("Invalid plugin path '{}': {}", cfg.path, e))?;
    }

    let mut mgr = state.0.lock();

    // Stop all current plugins
    mgr.stop_all();

    // Start enabled ones — detect type from file extension
    for cfg in &configs {
        if !cfg.enabled { continue; }

        let (ptype, mode) = crate::plugins::detect_plugin_type(&cfg.path);
        match ptype {
            PluginType::Exe => {
                if let Err(e) = mgr.start_plugin(&cfg.path, &cfg.key_bindings) {
                    crate::log(&format!("plugins: failed to start {}: {}", cfg.path, e));
                }
            }
            _ => {
                // Script: read metadata to get hello info
                if let Ok(content) = std::fs::read_to_string(&cfg.path) {
                    if let Some((hello, _)) = crate::plugins::parse_script_metadata(&content, ptype) {
                        if let Err(e) = mgr.start_plugin_ext(&cfg.path, ptype, mode, &hello, &cfg.key_bindings) {
                            crate::log(&format!("plugins: failed to start script {}: {}", cfg.path, e));
                        }
                    }
                }
            }
        }
    }

    // Save configs to settings file
    save_plugin_configs(configs)?;

    Ok(())
}

/// Load saved plugin configs from the plugin config file.
#[tauri::command]
pub fn load_plugin_configs(window: tauri::Window) -> Vec<PluginConfig> {
    let mut configs = load_plugin_configs_sync();
    // Only the Settings UI (the "main" window) may receive decrypted plugin
    // settings — they can contain secrets such as the encryption plugin's
    // password. Every other window (Results/Editor) never needs the plaintext
    // settings, so blank them to keep a compromised WebView from reading
    // secrets over the IPC boundary (mirrors load_settings in settings.rs).
    if window.label() != "main" {
        for cfg in &mut configs {
            cfg.settings = String::new();
        }
    }
    configs
}

/// Open a script in a PowerShell terminal for interactive debugging.
#[tauri::command]
pub fn run_script_in_terminal(window: tauri::Window, path: String) -> Result<(), String> {
    crate::commands::require_main_window(&window)?;
    let p = std::path::Path::new(&path);
    ensure_in_plugins_dir(p)?;

    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    let script_cmd = match ext {
        "py" => format!("python '{}'", path.replace('\'', "''")),
        "cs" => format!("dotnet run '{}' '--property:WarningLevel=0' --", path.replace('\'', "''")),
        "ps1" => format!("& '{}'", path.replace('\'', "''")),
        _ => return Err(format!("Unsupported extension: {}", ext)),
    };

    std::process::Command::new(crate::plugins::powershell_path())
        .args(["-NoExit", "-Command", &script_cmd])
        .spawn()
        .map_err(|e| format!("Failed to open PowerShell: {}", e))?;

    Ok(())
}

/// Spawn a command and wait for it with a hard timeout, killing it if it hangs.
/// stdout and stderr are drained on dedicated threads for the whole wait: without
/// active readers, a script that prints more than the OS pipe buffer (~64KB)
/// before exiting would block on the full pipe, never exit, and only be killed at
/// the timeout with its output lost. The reader threads finish when the pipes
/// close (on the child's exit or kill), so they never leak.
fn output_with_timeout(mut cmd: std::process::Command, secs: u64) -> Result<std::process::Output, String> {
    use std::io::Read;
    let mut child = cmd.spawn().map_err(|e| format!("Failed to run: {}", e))?;

    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        pipe.map(|mut r| std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            buf
        }))
    };
    let out_reader = drain(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let err_reader = drain(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let status = loop {
        match child.try_wait().map_err(|e| format!("wait: {}", e))? {
            Some(status) => break status,
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    // Join the readers so their threads don't outlive this call.
                    let _ = out_reader.map(|h| h.join());
                    let _ = err_reader.map(|h| h.join());
                    return Err(format!("script timed out after {}s", secs));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    };

    let stdout = out_reader.and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr = err_reader.and_then(|h| h.join().ok()).unwrap_or_default();
    Ok(std::process::Output { status, stdout, stderr })
}

/// Run a script on demand and return its stdout.
#[tauri::command]
pub fn run_script(window: tauri::Window, path: String) -> Result<String, String> {
    crate::commands::require_main_window(&window)?;
    // Only scripts inside the plugins/ dir may be executed (BUGS#11).
    ensure_in_plugins_dir(std::path::Path::new(&path))?;

    let ext = std::path::Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let plugin_type = match ext {
        "py" => PluginType::Python,
        "cs" => PluginType::CSharp,
        "ps1" => PluginType::PowerShell,
        _ => return Err(format!("Unsupported extension: {}", ext)),
    };

    let mut cmd = match plugin_type {
        PluginType::Python => {
            let mut c = std::process::Command::new("python");
            c.arg(&path);
            c
        }
        PluginType::CSharp => {
            let mut c = std::process::Command::new("dotnet");
            c.arg("run").arg(&path).arg("--property:WarningLevel=0");
            c
        }
        PluginType::PowerShell => {
            let mut c = std::process::Command::new(crate::plugins::powershell_path());
            c.args(["-NoProfile", "-File"]).arg(&path);
            c
        }
        _ => unreachable!(),
    };

    cmd.stdout(std::process::Stdio::piped())
       .stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    // Bounded so a hung "Run" never leaves the button stuck (dotnet run can be
    // slow on first compile, hence 60s).
    let output = output_with_timeout(cmd, 60)?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(if stdout.is_empty() { "(no output)".into() } else { stdout })
    } else {
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

/// Sanitize a script name to a safe filename (alphanumeric, hyphens only).
fn sanitize_script_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>()
        .to_lowercase()
}

/// Check that a path is inside the plugins/ directory.
pub(crate) fn ensure_in_plugins_dir(path: &std::path::Path) -> Result<(), String> {
    let dir = PluginManager::plugins_dir()
        .ok_or("Cannot determine plugins directory")?;
    let canonical = if path.exists() {
        path.canonicalize()
            .map_err(|e| format!("Invalid plugin path: {}", e))?
    } else {
        let parent = path.parent()
            .ok_or_else(|| "Invalid plugin path".to_string())?;
        let filename = path.file_name()
            .ok_or_else(|| "Invalid plugin path".to_string())?;
        parent
            .canonicalize()
            .map_err(|e| format!("Invalid plugin path: {}", e))?
            .join(filename)
    };
    let canonical_dir = dir.canonicalize()
        .or_else(|_| Ok::<_, String>(dir.clone()))
        .unwrap();
    if !canonical.starts_with(&canonical_dir) {
        return Err("Path must be inside the plugins/ directory".into());
    }
    Ok(())
}

/// Save a script file to the plugins/ directory.
/// `overwrite` is true when editing an existing script; for a brand-new script
/// a name collision is rejected instead of silently clobbering (BUGS#11).
#[tauri::command]
pub fn save_script(window: tauri::Window, name: String, language: String, content: String, overwrite: Option<bool>) -> Result<String, String> {
    crate::commands::require_main_window(&window)?;
    let dir = PluginManager::plugins_dir()
        .ok_or("Cannot determine plugins directory")?;

    let ext = match language.as_str() {
        "python" => "py",
        "csharp" => "cs",
        "powershell" => "ps1",
        _ => return Err(format!("Unknown language: {}", language)),
    };

    let safe_name = sanitize_script_name(&name);
    if safe_name.is_empty() {
        return Err("Name must contain at least one alphanumeric character".into());
    }
    let filename = format!("{}.{}", safe_name, ext);
    let path = dir.join(&filename);

    // Reject silent overwrite of an existing script unless the caller is editing it.
    if path.exists() && !overwrite.unwrap_or(false) {
        return Err(format!("A script named \"{}\" already exists. Choose a different name.", filename));
    }

    std::fs::write(&path, &content)
        .map_err(|e| format!("Failed to write: {}", e))?;

    Ok(path.to_string_lossy().to_string())
}

/// Delete a script file (must be inside plugins/ directory).
#[tauri::command]
pub fn delete_script(window: tauri::Window, path: String) -> Result<(), String> {
    crate::commands::require_main_window(&window)?;
    let p = std::path::Path::new(&path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext != "py" && ext != "cs" && ext != "ps1" {
        return Err("Can only delete .py, .cs, and .ps1 scripts".into());
    }
    ensure_in_plugins_dir(p)?;
    std::fs::remove_file(p)
        .map_err(|e| format!("Failed to delete: {}", e))
}

/// Check if a runtime (python/dotnet) is available.
#[tauri::command]
pub fn check_runtime(window: tauri::Window, language: String) -> Result<String, String> {
    // Spawns an external interpreter to probe availability; only the Settings UI
    // (which manages plugins) needs it, so gate it like the other plugin commands.
    crate::commands::require_main_window(&window)?;
    crate::plugins::check_runtime(&language)
}

/// Read a script file's content (must be inside plugins/ directory).
#[tauri::command]
pub fn read_script(window: tauri::Window, path: String) -> Result<String, String> {
    crate::commands::require_main_window(&window)?;
    let p = std::path::Path::new(&path);
    ensure_in_plugins_dir(p)?;
    std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read: {}", e))
}

/// Pre-compile a C# script (triggers dotnet build cache).
#[tauri::command]
pub fn precompile_script(window: tauri::Window, path: String) -> Result<String, String> {
    crate::commands::require_main_window(&window)?;
    ensure_in_plugins_dir(std::path::Path::new(&path))?;

    let mut cmd = std::process::Command::new("dotnet");
    cmd.arg("run").arg(&path).arg("--property:WarningLevel=0").args(["--", "--help"]);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    cmd.stdout(std::process::Stdio::null())
       .stderr(std::process::Stdio::piped());

    let output = output_with_timeout(cmd, 60)
        .map_err(|e| format!("Pre-compilation failed: {}", e))?;

    if output.status.success() {
        Ok("Compiled successfully".into())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Compilation error:\n{}", stderr))
    }
}

// ── Persistence helpers ─────────────────────────────────────────

fn plugin_config_path() -> std::path::PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    path.push("ClipToAll");
    std::fs::create_dir_all(&path).ok();
    path.push("plugins.json");
    path
}

fn save_plugin_configs(configs: Vec<PluginConfig>) -> Result<(), String> {
    // Encrypt settings before writing to disk
    let encrypted_configs: Vec<PluginConfig> = configs.into_iter().map(|mut cfg| {
        if !cfg.settings.is_empty() {
            cfg.settings = crate::utils::dpapi::dpapi_encrypt(&cfg.settings)
                .map_err(|e| {
                    format!("Failed to encrypt plugin settings for '{}': {}", cfg.path, e)
                })?;
        }
        Ok(cfg)
    })
    .collect::<Result<_, String>>()?;

    let path = plugin_config_path();
    let json = serde_json::to_string_pretty(&encrypted_configs)
        .map_err(|e| format!("serialize: {}", e))?;
    // Atomic write so a crash mid-write can't corrupt plugins.json (which would
    // drop the user's plugin config, incl. the encryption password binding).
    crate::utils::fs::atomic_write(&path, json.as_bytes())?;
    Ok(())
}

pub fn load_plugin_configs_sync() -> Vec<PluginConfig> {
    let path = plugin_config_path();
    if !path.exists() {
        return vec![];
    }
    let mut configs: Vec<PluginConfig> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    // Decrypt settings after reading from disk
    for cfg in &mut configs {
        if !cfg.settings.is_empty() {
            if let Ok(decrypted) = crate::utils::dpapi::dpapi_decrypt(&cfg.settings) {
                cfg.settings = decrypted;
            }
        }
    }

    // Migrate old paths: if path doesn't exist, try plugins/ subfolder
    for cfg in &mut configs {
        if !std::path::Path::new(&cfg.path).exists() {
            let filename = std::path::Path::new(&cfg.path)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            if let Some(dir) = crate::plugins::PluginManager::plugins_dir() {
                let new_path = dir.join(&filename);
                if new_path.exists() {
                    crate::log(&format!("plugins: migrated path {} → {}", cfg.path, new_path.display()));
                    cfg.path = new_path.to_string_lossy().to_string();
                }
            }
        }
    }

    configs
}
