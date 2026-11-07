//! Names that keep the public app and the personal build (`--features
//! personal`) from sharing state when both are installed on one Mac.
//!
//! The bundle identifier, and with it the TCC (Screen Recording) grant, the
//! SMAppService login item, the WKWebView data store and the single-instance
//! socket, comes from the Tauri config: tauri.personal.conf.json overrides it
//! for the personal build. Everything the app names by itself lives here:
//! the settings/data directory under the OS config dir (settings, logs,
//! Drive token, plugin config, macOS plugins dir) and the Keychain service.
//!
//! Without the feature the value is exactly what it has always been, so the
//! public build reads and writes the same paths and Keychain items as before.

/// Directory name under the OS config dir (`%APPDATA%` on Windows,
/// `~/Library/Application Support` on macOS), the Keychain service name, and
/// the tray tooltip.
#[cfg(not(feature = "personal"))]
pub const APP_NAME: &str = "ClipToAll";
#[cfg(feature = "personal")]
pub const APP_NAME: &str = "ClipToAll Personal";

/// The bundle identifier the personal build must run under (the one in
/// tauri.personal.conf.json; a test keeps the two in sync). main.rs refuses
/// to start a personal binary built WITHOUT that overlay: it would carry the
/// public identifier and so share the public app's single-instance lock,
/// Screen Recording grant and WebView data while using its own settings.
#[cfg(feature = "personal")]
pub const PERSONAL_IDENTIFIER: &str = "com.ultrathinker.cliptoall.personal";

#[cfg(test)]
mod tests {
    use super::APP_NAME;

    #[cfg(not(feature = "personal"))]
    #[test]
    fn public_build_keeps_its_historical_name() {
        // Changing this would move every existing user's settings, logs and
        // Keychain items out from under them.
        assert_eq!(APP_NAME, "ClipToAll");
    }

    /// The personal build's name must match the productName in its Tauri
    /// overlay (so the data dir is named like the .app) and its identifier
    /// must differ from the public one (so macOS treats it as another app).
    #[cfg(feature = "personal")]
    #[test]
    fn personal_build_matches_its_tauri_overlay() {
        let base: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        let overlay: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.personal.conf.json")).unwrap();
        assert_eq!(overlay["productName"].as_str(), Some(APP_NAME));
        assert_eq!(overlay["identifier"].as_str(), Some(super::PERSONAL_IDENTIFIER));
        let public_id = base["identifier"].as_str().unwrap();
        let personal_id = overlay["identifier"].as_str().unwrap();
        assert_ne!(public_id, personal_id);
        assert!(!personal_id.starts_with(&format!("{}.", public_id)));
        assert_ne!(APP_NAME, base["productName"].as_str().unwrap());
    }
}
