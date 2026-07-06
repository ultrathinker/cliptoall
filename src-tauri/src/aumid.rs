use windows::core::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONWARNING, MB_YESNO, SW_SHOWNORMAL,
};

/// Show a MessageBox explaining that the target window is elevated,
/// and offer to restart as Administrator. Returns true if user chose "Yes".
/// Public so other modules can reuse when they need admin elevation.
pub fn show_admin_dialog() -> bool {
    let text = "The target window belongs to an application running as Administrator.\n\n\
         To perform this action, ClipToAll must also run as Administrator.\n\n\
         Restart ClipToAll as Administrator now?".to_string();
    unsafe {
        let result = MessageBoxW(
            None,
            &HSTRING::from(text),
            w!("ClipToAll"),
            MB_YESNO | MB_ICONWARNING,
        );
        result == IDYES
    }
}

/// Restart the current exe elevated via ShellExecuteW "runas", then exit.
/// Public so other modules can reuse when they need admin elevation.
pub fn restart_as_admin() {
    let exe = std::env::current_exe().unwrap_or_default();
    let exe_str = exe.to_string_lossy();
    crate::log(&format!("aumid: restarting as admin: {}", exe_str));
    unsafe {
        ShellExecuteW(
            None,
            w!("runas"),
            &HSTRING::from(exe_str.as_ref()),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
    std::process::exit(0);
}
