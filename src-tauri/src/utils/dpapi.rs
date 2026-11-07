//! Encrypted secret storage.
//!
//! Windows: DPAPI (CryptProtectData/CryptUnprotectData, CurrentUser scope)
//! encrypts an arbitrary blob with a system key and returns self-contained
//! ciphertext — no external state, so callers pass no identifier.
//!
//! macOS: the Keychain is a service+account key-value store, not a blob
//! cipher, so each secret needs a STABLE account name. "Encrypt" stores the
//! plaintext under that account and returns a small opaque reference
//! ("keychain:<account>") to embed inline where DPAPI ciphertext used to go;
//! "decrypt" looks the account back up. The account must be stable across
//! saves (not a fresh UUID per call) or every settings save would leak a new
//! orphaned Keychain entry (PLAN.md Phase 1.2).

#[cfg(windows)]
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
#[cfg(windows)]
use windows::Win32::Foundation::{HLOCAL, LocalFree};
#[cfg(windows)]
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
};

/// Free a buffer allocated by DPAPI. MSDN specifies `LocalFree` for the
/// CryptProtectData/CryptUnprotectData output blob (3.21).
#[cfg(windows)]
unsafe fn free_dpapi_blob(ptr: *mut u8) {
    if !ptr.is_null() {
        let _ = LocalFree(HLOCAL(ptr as *mut _));
    }
}

/// Prefix for DPAPI-encrypted fields stored inline in JSON.
#[cfg(windows)]
const DPAPI_PREFIX: &str = "dpapi:";

/// Encrypt a string using Windows DPAPI (CurrentUser scope), return base64.
/// `_account` is unused on Windows — DPAPI ciphertext is self-contained.
#[cfg(windows)]
pub fn dpapi_encrypt(_account: &str, plaintext: &str) -> Result<String, String> {
    unsafe {
        let input_bytes = plaintext.as_bytes();
        let input_blob = CRYPT_INTEGER_BLOB {
            cbData: input_bytes.len() as u32,
            pbData: input_bytes.as_ptr() as *mut u8,
        };
        let mut output_blob = CRYPT_INTEGER_BLOB::default();

        CryptProtectData(
            &input_blob,
            None,     // description
            None,     // entropy
            None,     // reserved
            None,     // prompt
            0,        // flags (CurrentUser is default)
            &mut output_blob,
        ).map_err(|e| format!("DPAPI encrypt failed: {}", e))?;

        let encrypted = std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize).to_vec();
        free_dpapi_blob(output_blob.pbData);

        Ok(BASE64.encode(&encrypted))
    }
}

/// Decrypt a base64+DPAPI string back to plaintext.
#[cfg(windows)]
pub fn dpapi_decrypt(encrypted_b64: &str) -> Result<String, String> {
    let encrypted = BASE64.decode(encrypted_b64).map_err(|e| format!("base64 decode: {}", e))?;

    unsafe {
        let input_blob = CRYPT_INTEGER_BLOB {
            cbData: encrypted.len() as u32,
            pbData: encrypted.as_ptr() as *mut u8,
        };
        let mut output_blob = CRYPT_INTEGER_BLOB::default();

        CryptUnprotectData(
            &input_blob,
            None,     // description
            None,     // entropy
            None,     // reserved
            None,     // prompt
            0,        // flags
            &mut output_blob,
        ).map_err(|e| format!("DPAPI decrypt failed: {}", e))?;

        let decrypted = std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize).to_vec();
        free_dpapi_blob(output_blob.pbData);

        String::from_utf8(decrypted).map_err(|e| format!("UTF-8 decode: {}", e))
    }
}

/// Encrypt a field value for inline storage in JSON.
/// Prepends "dpapi:" prefix so the value can be identified as encrypted.
/// Returns empty string unchanged (no point encrypting nothing).
#[cfg(windows)]
pub fn encrypt_field(account: &str, value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    match dpapi_encrypt(account, value) {
        Ok(encrypted) => Ok(format!("{}{}", DPAPI_PREFIX, encrypted)),
        Err(e) => {
            crate::log(&format!("dpapi: encrypt_field failed: {}", e));
            Err(e)
        }
    }
}

/// Decrypt a field value from JSON.
/// Detects "dpapi:" prefix → decrypt. No prefix → return as-is (plaintext migration).
#[cfg(windows)]
pub fn decrypt_field(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if let Some(encrypted) = value.strip_prefix(DPAPI_PREFIX) {
        match dpapi_decrypt(encrypted) {
            Ok(decrypted) => decrypted,
            Err(e) => {
                crate::log(&format!("dpapi: decrypt_field failed: {}", e));
                String::new() // corrupted — return empty, user will need to re-enter
            }
        }
    } else {
        // No prefix — plaintext (pre-encryption migration), return as-is
        value.to_string()
    }
}

// ── macOS: Keychain-backed store ────────────────────────────────

/// Service name all ClipToAll Keychain entries are filed under (visible in
/// Keychain Access.app as the entries' "Where" column).
#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "ClipToAll";

/// Prefix for Keychain-backed fields stored inline in JSON — analogous to
/// Windows' "dpapi:" prefix, but the payload is an account name, not ciphertext.
#[cfg(target_os = "macos")]
const KEYCHAIN_PREFIX: &str = "keychain:";

/// Store `plaintext` in the Keychain under `account` (stable — a re-save
/// overwrites the same entry instead of leaking a new one), return the
/// opaque reference to embed inline in JSON.
#[cfg(target_os = "macos")]
pub fn dpapi_encrypt(account: &str, plaintext: &str) -> Result<String, String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, account)
        .map_err(|e| format!("Keychain entry failed: {}", e))?;
    entry.set_password(plaintext)
        .map_err(|e| format!("Keychain set_password failed: {}", e))?;
    Ok(format!("{}{}", KEYCHAIN_PREFIX, account))
}

/// Look up a Keychain reference ("keychain:<account>") and return its secret.
#[cfg(target_os = "macos")]
pub fn dpapi_decrypt(reference: &str) -> Result<String, String> {
    let account = reference.strip_prefix(KEYCHAIN_PREFIX)
        .ok_or_else(|| "not a keychain reference".to_string())?;
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, account)
        .map_err(|e| format!("Keychain entry failed: {}", e))?;
    entry.get_password().map_err(|e| format!("Keychain get_password failed: {}", e))
}

/// Encrypt a field value for inline storage in JSON (see module docs for why
/// `account` must be a stable identifier, e.g. the field's own name).
/// Returns empty string unchanged (no point storing nothing).
#[cfg(target_os = "macos")]
pub fn encrypt_field(account: &str, value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    match dpapi_encrypt(account, value) {
        Ok(reference) => Ok(reference),
        Err(e) => {
            crate::log(&format!("keychain: encrypt_field failed: {}", e));
            Err(e)
        }
    }
}

/// Decrypt a field value from JSON.
/// Detects "keychain:" prefix → look up. No prefix → return as-is (plaintext migration).
#[cfg(target_os = "macos")]
pub fn decrypt_field(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if value.starts_with(KEYCHAIN_PREFIX) {
        match dpapi_decrypt(value) {
            Ok(decrypted) => decrypted,
            Err(e) => {
                crate::log(&format!("keychain: decrypt_field failed: {}", e));
                String::new() // corrupted/missing — return empty, user will need to re-enter
            }
        }
    } else {
        // No prefix — plaintext (pre-encryption migration), return as-is
        value.to_string()
    }
}
