//! Clipboard text encryption commands.
//!
//! Two on-the-wire schemes coexist:
//!
//! * **Legacy** (default, unmarked): key = SHA-256(password), fixed IV =
//!   SHA-256("ClipToAll")[..16], AES-256-CBC + PKCS7, base64. This is
//!   byte-for-byte compatible with the old .NET ClipToAll and with the
//!   hundreds of already-encrypted values the user has. It is intentionally
//!   left unchanged.
//!
//! * **Strong v2** (opt-in, marker-prefixed `CTA2:`): per-message random salt,
//!   PBKDF2-HMAC-SHA256 key derivation, random 96-bit nonce, AES-256-GCM
//!   (authenticated). Envelope: `CTA2:` + base64(salt(16) || nonce(12) || ct+tag).
//!
//! CAVEAT: v2 ciphertext is NOT decryptable by the old .NET ClipToAll — only
//! legacy is. Choosing "Strong" deliberately breaks .NET interop; that trade-off
//! must be surfaced in the UI/label. Decryption auto-detects the scheme from the
//! marker, so legacy values keep working forever.
//!
//! NOTE: this logic is intentionally duplicated in
//! `plugins/encryption-plugin/src/crypto.rs` (a separate crate). Both must stay
//! behaviourally identical so ciphertext produced by one decrypts in the other.

use aes::Aes256;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use sha2::{Digest, Sha256};

type Aes256CbcEnc = cbc::Encryptor<Aes256>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;

/// ASCII marker that prefixes strong (v2) ciphertext. Standard base64 never
/// contains ':', so the presence of this marker unambiguously distinguishes a
/// v2 envelope from a legacy (unmarked) base64 blob.
const MARKER_V2: &str = "CTA2:";

/// PBKDF2 iteration count. 600k HMAC-SHA256 rounds matches OWASP's 2023
/// guidance for PBKDF2-HMAC-SHA256 and is a good CPU-cost/UX balance for a
/// one-shot clipboard operation.
const PBKDF2_ITERATIONS: u32 = 600_000;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12; // AES-GCM standard 96-bit nonce
const KEY_LEN: usize = 32;

// ── Legacy scheme (unchanged, .NET-compatible) ──────────────────────────────

fn derive_key_legacy(password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

fn derive_iv_legacy() -> [u8; 16] {
    let mut hasher = Sha256::new();
    hasher.update(b"ClipToAll");
    let result = hasher.finalize();
    let mut iv = [0u8; 16];
    iv.copy_from_slice(&result[..16]);
    iv
}

fn encrypt_legacy(text: &str, password: &str) -> Result<String, String> {
    let key = derive_key_legacy(password);
    let iv = derive_iv_legacy();

    let mut buffer = text.as_bytes().to_vec();
    let pos = buffer.len();
    let block_size = 16;
    let padding = block_size - (pos % block_size);
    buffer.resize(pos + padding, padding as u8);

    let cipher = Aes256CbcEnc::new(&key.into(), &iv.into());
    cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, pos)
        .map_err(|e| format!("Encryption failed: {:?}", e))?;

    Ok(base64::engine::general_purpose::STANDARD.encode(&buffer))
}

fn decrypt_legacy(encrypted: &str, password: &str) -> Result<String, String> {
    let key = derive_key_legacy(password);
    let iv = derive_iv_legacy();

    let mut encrypted_bytes = base64::engine::general_purpose::STANDARD
        .decode(encrypted.trim())
        .map_err(|e| format!("Base64 decode failed: {}", e))?;

    let cipher = Aes256CbcDec::new(&key.into(), &iv.into());
    let decrypted = cipher
        .decrypt_padded_mut::<Pkcs7>(&mut encrypted_bytes)
        .map_err(|_| "Decryption failed: wrong password or corrupted data".to_string())?;

    String::from_utf8(decrypted.to_vec()).map_err(|e| format!("UTF-8 decode failed: {}", e))
}

// ── Strong scheme v2 (PBKDF2 + AES-256-GCM, marker-prefixed) ─────────────────

fn derive_key_pbkdf2(password: &str, salt: &[u8]) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, PBKDF2_ITERATIONS, &mut key);
    key
}

fn fill_random(buf: &mut [u8]) -> Result<(), String> {
    getrandom::getrandom(buf).map_err(|e| format!("Failed to gather randomness: {}", e))
}

fn encrypt_v2(text: &str, password: &str) -> Result<String, String> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    fill_random(&mut salt)?;
    fill_random(&mut nonce_bytes)?;

    let key = derive_key_pbkdf2(password, &salt);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, text.as_bytes())
        .map_err(|e| format!("Encryption failed: {:?}", e))?;

    // Envelope: salt || nonce || (ciphertext + 16-byte GCM tag)
    let mut envelope = Vec::with_capacity(SALT_LEN + NONCE_LEN + ciphertext.len());
    envelope.extend_from_slice(&salt);
    envelope.extend_from_slice(&nonce_bytes);
    envelope.extend_from_slice(&ciphertext);

    Ok(format!(
        "{}{}",
        MARKER_V2,
        base64::engine::general_purpose::STANDARD.encode(&envelope)
    ))
}

fn decrypt_v2(marked: &str, password: &str) -> Result<String, String> {
    let b64 = marked
        .strip_prefix(MARKER_V2)
        .ok_or_else(|| "Not a v2 ciphertext".to_string())?;

    let envelope = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("Base64 decode failed: {}", e))?;

    if envelope.len() < SALT_LEN + NONCE_LEN {
        return Err("Corrupted data: v2 envelope too short".to_string());
    }

    let salt = &envelope[..SALT_LEN];
    let nonce_bytes = &envelope[SALT_LEN..SALT_LEN + NONCE_LEN];
    let ciphertext = &envelope[SALT_LEN + NONCE_LEN..];

    let key = derive_key_pbkdf2(password, salt);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "Decryption failed: wrong password or tampered data".to_string())?;

    String::from_utf8(plaintext).map_err(|e| format!("UTF-8 decode failed: {}", e))
}

// ── Public dispatch ─────────────────────────────────────────────────────────

/// Encrypt clipboard text.
///
/// `strong` selects the scheme: `Some(true)` → v2 (strong, marker-prefixed),
/// anything else (including the default when the frontend omits the argument)
/// → legacy, so existing behaviour and .NET interop are preserved unless the
/// user explicitly opts in.
#[tauri::command]
pub fn encrypt_text(text: String, password: String, strong: Option<bool>) -> Result<String, String> {
    if strong.unwrap_or(false) {
        encrypt_v2(&text, &password)
    } else {
        encrypt_legacy(&text, &password)
    }
}

/// Decrypt clipboard text. Auto-detects the scheme from the `CTA2:` marker, so
/// both legacy and v2 ciphertexts decrypt regardless of the current toggle.
#[tauri::command]
pub fn decrypt_text(encrypted: String, password: String) -> Result<String, String> {
    if encrypted.starts_with(MARKER_V2) {
        decrypt_v2(&encrypted, &password)
    } else {
        decrypt_legacy(&encrypted, &password)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_roundtrip_ascii() {
        let enc = encrypt_text("hello world".into(), "pass".into(), None).unwrap();
        // Legacy output carries no marker.
        assert!(!enc.starts_with(MARKER_V2));
        assert_eq!(decrypt_text(enc, "pass".into()).unwrap(), "hello world");
    }

    #[test]
    fn legacy_roundtrip_unicode() {
        let s = "Привет, 世界 🌍";
        let enc = encrypt_text(s.into(), "pw".into(), Some(false)).unwrap();
        assert_eq!(decrypt_text(enc, "pw".into()).unwrap(), s);
    }

    /// Known legacy ciphertext generated with the pre-upgrade scheme
    /// (key=SHA256("pass"), IV=SHA256("ClipToAll")[..16], AES-256-CBC/PKCS7).
    /// Guards against any accidental change to the legacy algorithm.
    #[test]
    fn legacy_known_sample_still_decrypts() {
        // Hardcoded ciphertext produced by the pre-upgrade legacy scheme for
        // plaintext "hello world" / password "pass" (verified independently via
        // openssl aes-256-cbc). Because legacy uses a fixed IV it is
        // deterministic, so our encryptor must still reproduce this exact blob.
        const KNOWN_LEGACY: &str = "JV7UtYjhiYtyvQmTdzoikw==";
        assert_eq!(encrypt_legacy("hello world", "pass").unwrap(), KNOWN_LEGACY);
        // And it decrypts through the public auto-detecting path.
        assert_eq!(
            decrypt_text(KNOWN_LEGACY.into(), "pass".into()).unwrap(),
            "hello world"
        );
    }

    #[test]
    fn v2_roundtrip() {
        let s = "top secret Привет 🌍";
        let enc = encrypt_text(s.into(), "correct horse".into(), Some(true)).unwrap();
        assert!(enc.starts_with(MARKER_V2));
        assert_eq!(decrypt_text(enc, "correct horse".into()).unwrap(), s);
    }

    #[test]
    fn v2_uses_random_salt_nonce() {
        // Two encryptions of the same input must differ (random salt + nonce).
        let a = encrypt_text("same".into(), "pw".into(), Some(true)).unwrap();
        let b = encrypt_text("same".into(), "pw".into(), Some(true)).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn v2_rejects_tampered_ciphertext() {
        let enc = encrypt_text("secret".into(), "pw".into(), Some(true)).unwrap();
        // Flip a character in the base64 body (after the marker).
        let mut chars: Vec<char> = enc.chars().collect();
        let last = chars.len() - 1;
        chars[last] = if chars[last] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        assert!(decrypt_text(tampered, "pw".into()).is_err());
    }

    #[test]
    fn v2_wrong_password_fails_cleanly() {
        let enc = encrypt_text("secret".into(), "right".into(), Some(true)).unwrap();
        assert!(decrypt_text(enc, "wrong".into()).is_err());
    }

    #[test]
    fn legacy_wrong_password_does_not_return_plaintext() {
        let enc = encrypt_text("secret".into(), "right".into(), None).unwrap();
        assert_ne!(
            decrypt_text(enc, "wrong".into()).ok().as_deref(),
            Some("secret")
        );
    }
}
