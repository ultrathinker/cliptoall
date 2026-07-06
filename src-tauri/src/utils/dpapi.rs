//! Windows DPAPI encryption helpers.
//!
//! Uses CryptProtectData / CryptUnprotectData (CurrentUser scope)
//! to encrypt sensitive data. Only the same Windows user can decrypt.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
};

/// Free a buffer allocated by DPAPI. MSDN specifies `LocalFree` for the
/// CryptProtectData/CryptUnprotectData output blob (3.21).
unsafe fn free_dpapi_blob(ptr: *mut u8) {
    if !ptr.is_null() {
        let _ = LocalFree(HLOCAL(ptr as *mut _));
    }
}

/// Prefix for DPAPI-encrypted fields stored inline in JSON.
const DPAPI_PREFIX: &str = "dpapi:";

/// Encrypt a string using Windows DPAPI (CurrentUser scope), return base64.
pub fn dpapi_encrypt(plaintext: &str) -> Result<String, String> {
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
pub fn encrypt_field(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    match dpapi_encrypt(value) {
        Ok(encrypted) => Ok(format!("{}{}", DPAPI_PREFIX, encrypted)),
        Err(e) => {
            crate::log(&format!("dpapi: encrypt_field failed: {}", e));
            Err(e)
        }
    }
}

/// Decrypt a field value from JSON.
/// Detects "dpapi:" prefix → decrypt. No prefix → return as-is (plaintext migration).
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
