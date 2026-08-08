#[cfg(windows)]
use winreg::enums::*;
#[cfg(windows)]
use winreg::RegKey;

#[cfg(windows)]
pub fn set_autorun(enable: bool) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let key = hkcu.open_subkey_with_flags(path, KEY_WRITE)
        .map_err(|e| format!("Failed to open registry key: {}", e))?;

    if enable {
        let exe_path = std::env::current_exe()
            .map_err(|e| format!("Failed to get exe path: {}", e))?;

        // Quote the path so spaces (e.g. C:\Program Files\...) parse unambiguously.
        let value = format!("\"{}\"", exe_path.to_string_lossy());
        key.set_value("ClipToAll", &value)
            .map_err(|e| format!("Failed to set registry value: {}", e))?;
    } else {
        key.delete_value("ClipToAll").ok();
    }

    Ok(())
}

/// LaunchAgent label — also its plist filename. Matches the app's bundle
/// identifier (tauri.conf.json) since this is the only LaunchAgent ClipToAll
/// installs.
#[cfg(not(windows))]
const LAUNCH_AGENT_LABEL: &str = "net.appshub.cliptoall";

/// Hand-rolled LaunchAgent plist rather than tauri-plugin-autostart: that
/// plugin's enable/disable live on `AppHandle` (via `ManagerExt`), but
/// `set_autorun` is called from `save_settings_to_disk_locked`, deep in the
/// settings-save path with no `AppHandle` in scope — threading one through
/// just for this would touch call sites that have nothing to do with
/// autostart. A plist write + `launchctl` needs no handle at all.
#[cfg(not(windows))]
fn launch_agent_path() -> Result<std::path::PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Could not resolve home directory".to_string())?;
    Ok(home.join("Library/LaunchAgents").join(format!("{}.plist", LAUNCH_AGENT_LABEL)))
}

#[cfg(not(windows))]
pub fn set_autorun(enable: bool) -> Result<(), String> {
    let path = launch_agent_path()?;

    if enable {
        let exe_path = std::env::current_exe()
            .map_err(|e| format!("Failed to get exe path: {}", e))?;

        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
            label = LAUNCH_AGENT_LABEL,
            exe = exe_path.to_string_lossy(),
        );

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create LaunchAgents dir: {}", e))?;
        }
        std::fs::write(&path, plist)
            .map_err(|e| format!("Failed to write LaunchAgent plist: {}", e))?;

        // Load immediately so the change takes effect without a logout/login.
        // Best-effort: RunAtLoad in the plist still covers next login even if
        // `launchctl load` fails here (e.g. a stale load from a previous run).
        let _ = std::process::Command::new("launchctl").args(["load", "-w"]).arg(&path).output();
    } else {
        let _ = std::process::Command::new("launchctl").args(["unload", "-w"]).arg(&path).output();
        let _ = std::fs::remove_file(&path);
    }

    Ok(())
}
