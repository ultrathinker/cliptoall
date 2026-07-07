//! Clipboard text encryption for the ClipToAll encryption plugin.
//!
//! Two on-the-wire schemes coexist:
//!
//! * **Legacy** (default, unmarked): key = SHA-256(password), fixed IV =
//!   SHA-256("ClipToAll")[..16], AES-256-CBC + PKCS7, base64. Byte-for-byte
//!   compatible with the old .NET ClipToAll and with every value the user has
//!   already encrypted. Intentionally left unchanged.
//!
//! * **Strong v2** (opt-in, marker-prefixed `CTA2:`): per-message random salt,
//!   PBKDF2-HMAC-SHA256 key derivation, random 96-bit nonce, AES-256-GCM
//!   (authenticated). Envelope: `CTA2:` + base64(salt(16) || nonce(12) || ct+tag).
//!
//! CAVEAT: v2 ciphertext is NOT decryptable by the old .NET ClipToAll — only
//! legacy is. Choosing "Strong" deliberately breaks .NET interop. Decryption
//! auto-detects the scheme from the marker, so legacy values keep working.
//!
//! NOTE: this module is intentionally a behavioural twin of
//! `src-tauri/src/commands/encryption.rs` (a separate crate). Ciphertext
//! produced by one must decrypt in the other; keep them in sync.

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use sha2::{Digest, Sha256};

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

/// ASCII marker that prefixes strong (v2) ciphertext. Standard base64 never
/// contains ':', so this marker unambiguously distinguishes a v2 envelope from
/// a legacy (unmarked) base64 blob.
pub const MARKER_V2: &str = "CTA2:";

/// PBKDF2 iteration count — 600k HMAC-SHA256 rounds (OWASP 2023 guidance).
const PBKDF2_ITERATIONS: u32 = 600_000;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12; // AES-GCM standard 96-bit nonce
const KEY_LEN: usize = 32;

/// Which scheme to use when encrypting. Decryption always auto-detects.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scheme {
    /// Legacy AES-256-CBC (default; compatible with the .NET version).
    Legacy,
    /// Strong v2: PBKDF2 + AES-256-GCM (breaks .NET interop).
    Strong,
}

impl Scheme {
    /// Parse the `scheme` settings field. Anything other than an explicit
    /// "strong"/"v2" (case-insensitive) falls back to Legacy — the safe default.
    pub fn from_setting(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "strong" | "v2" => Scheme::Strong,
            _ => Scheme::Legacy,
        }
    }
}

// ── Legacy scheme (unchanged, .NET-compatible) ──────────────────────────────

fn derive_key_legacy(password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hasher.finalize().into()
}

fn derive_iv_legacy() -> [u8; 16] {
    let mut hasher = Sha256::new();
    hasher.update(b"ClipToAll");
    let hash: [u8; 32] = hasher.finalize().into();
    let mut iv = [0u8; 16];
    iv.copy_from_slice(&hash[..16]);
    iv
}

fn encrypt_legacy(plaintext: &str, password: &str) -> Result<String, String> {
    let key = derive_key_legacy(password);
    let iv = derive_iv_legacy();

    let plaintext_bytes = plaintext.as_bytes();

    // Buffer must fit plaintext + padding (up to 16 extra bytes).
    let mut buf = vec![0u8; plaintext_bytes.len() + 16];
    buf[..plaintext_bytes.len()].copy_from_slice(plaintext_bytes);

    let ciphertext = Aes256CbcEnc::new(&key.into(), &iv.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plaintext_bytes.len())
        .map_err(|e| format!("Encryption failed: {}", e))?;

    Ok(BASE64.encode(ciphertext))
}

fn decrypt_legacy(base64_input: &str, password: &str) -> Result<String, String> {
    let key = derive_key_legacy(password);
    let iv = derive_iv_legacy();

    let mut ciphertext = BASE64
        .decode(base64_input.trim())
        .map_err(|e| format!("Invalid base64: {}", e))?;

    let plaintext_bytes = Aes256CbcDec::new(&key.into(), &iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut ciphertext)
        .map_err(|_| "Decryption failed: wrong password or corrupted data".to_string())?;

    String::from_utf8(plaintext_bytes.to_vec())
        .map_err(|e| format!("Decrypted data is not valid UTF-8: {}", e))
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

fn encrypt_v2(plaintext: &str, password: &str) -> Result<String, String> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    fill_random(&mut salt)?;
    fill_random(&mut nonce_bytes)?;

    let key = derive_key_pbkdf2(password, &salt);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("Encryption failed: {:?}", e))?;

    // Envelope: salt || nonce || (ciphertext + 16-byte GCM tag)
    let mut envelope = Vec::with_capacity(SALT_LEN + NONCE_LEN + ciphertext.len());
    envelope.extend_from_slice(&salt);
    envelope.extend_from_slice(&nonce_bytes);
    envelope.extend_from_slice(&ciphertext);

    Ok(format!("{}{}", MARKER_V2, BASE64.encode(&envelope)))
}

fn decrypt_v2(marked: &str, password: &str) -> Result<String, String> {
    let b64 = marked
        .strip_prefix(MARKER_V2)
        .ok_or_else(|| "Not a v2 ciphertext".to_string())?;

    let envelope = BASE64
        .decode(b64.trim())
        .map_err(|e| format!("Invalid base64: {}", e))?;

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

    String::from_utf8(plaintext).map_err(|e| format!("Decrypted data is not valid UTF-8: {}", e))
}

// ── Public dispatch ─────────────────────────────────────────────────────────

/// Encrypt `plaintext` using the chosen `scheme`.
pub fn encrypt_text(plaintext: &str, password: &str, scheme: Scheme) -> Result<String, String> {
    match scheme {
        Scheme::Legacy => encrypt_legacy(plaintext, password),
        Scheme::Strong => encrypt_v2(plaintext, password),
    }
}

/// Decrypt `input`, auto-detecting the scheme from the `CTA2:` marker so both
/// legacy and v2 ciphertexts decrypt regardless of the configured scheme.
pub fn decrypt_text(input: &str, password: &str) -> Result<String, String> {
    if input.starts_with(MARKER_V2) {
        decrypt_v2(input, password)
    } else {
        decrypt_legacy(input, password)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_roundtrip_ascii() {
        let enc = encrypt_text("hello world", "pass", Scheme::Legacy).unwrap();
        assert!(!enc.starts_with(MARKER_V2));
        assert_eq!(decrypt_text(&enc, "pass").unwrap(), "hello world");
    }

    #[test]
    fn legacy_roundtrip_unicode() {
        let s = "Привет, 世界 🌍";
        let enc = encrypt_text(s, "pw", Scheme::Legacy).unwrap();
        assert_eq!(decrypt_text(&enc, "pw").unwrap(), s);
    }

    /// Hardcoded ciphertext from the pre-upgrade legacy scheme for
    /// "hello world" / "pass" (verified via openssl aes-256-cbc). Legacy uses a
    /// fixed IV, so it is deterministic and must still reproduce this exact blob.
    /// This is also the interop anchor with the .NET version.
    #[test]
    fn legacy_known_sample_still_decrypts() {
        const KNOWN_LEGACY: &str = "JV7UtYjhiYtyvQmTdzoikw==";
        assert_eq!(encrypt_text("hello world", "pass", Scheme::Legacy).unwrap(), KNOWN_LEGACY);
        assert_eq!(decrypt_text(KNOWN_LEGACY, "pass").unwrap(), "hello world");
    }

    #[test]
    fn v2_roundtrip() {
        let s = "top secret Привет 🌍";
        let enc = encrypt_text(s, "correct horse", Scheme::Strong).unwrap();
        assert!(enc.starts_with(MARKER_V2));
        assert_eq!(decrypt_text(&enc, "correct horse").unwrap(), s);
    }

    #[test]
    fn v2_uses_random_salt_nonce() {
        let a = encrypt_text("same", "pw", Scheme::Strong).unwrap();
        let b = encrypt_text("same", "pw", Scheme::Strong).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn v2_rejects_tampered_ciphertext() {
        let enc = encrypt_text("secret", "pw", Scheme::Strong).unwrap();
        let mut chars: Vec<char> = enc.chars().collect();
        let last = chars.len() - 1;
        chars[last] = if chars[last] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        assert!(decrypt_text(&tampered, "pw").is_err());
    }

    #[test]
    fn v2_wrong_password_fails_cleanly() {
        let enc = encrypt_text("secret", "right", Scheme::Strong).unwrap();
        assert!(decrypt_text(&enc, "wrong").is_err());
    }

    #[test]
    fn legacy_wrong_password_does_not_return_plaintext() {
        let enc = encrypt_text("secret", "right", Scheme::Legacy).unwrap();
        assert_ne!(decrypt_text(&enc, "wrong").ok().as_deref(), Some("secret"));
    }

    #[test]
    fn scheme_parsing() {
        assert_eq!(Scheme::from_setting("strong"), Scheme::Strong);
        assert_eq!(Scheme::from_setting("V2"), Scheme::Strong);
        assert_eq!(Scheme::from_setting("legacy"), Scheme::Legacy);
        assert_eq!(Scheme::from_setting(""), Scheme::Legacy);
        assert_eq!(Scheme::from_setting("garbage"), Scheme::Legacy);
    }
}
