use winreg::enums::*;
use winreg::RegKey;

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
