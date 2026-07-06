use aes::Aes256;
use base64::Engine;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use sha2::{Digest, Sha256};

type Aes256CbcEnc = cbc::Encryptor<Aes256>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;

fn derive_key(password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

fn derive_iv() -> [u8; 16] {
    let mut hasher = Sha256::new();
    hasher.update(b"ClipToAll");
    let result = hasher.finalize();
    let mut iv = [0u8; 16];
    iv.copy_from_slice(&result[..16]);
    iv
}

#[tauri::command]
pub fn encrypt_text(text: String, password: String) -> Result<String, String> {
    let key = derive_key(&password);
    let iv = derive_iv();
    
    let mut buffer = text.as_bytes().to_vec();
    let pos = buffer.len();
    let block_size = 16;
    let padding = block_size - (pos % block_size);
    buffer.resize(pos + padding, padding as u8);
    
    let cipher = Aes256CbcEnc::new(&key.into(), &iv.into());
    cipher.encrypt_padded_mut::<Pkcs7>(&mut buffer, pos)
        .map_err(|e| format!("Encryption failed: {:?}", e))?;
    
    Ok(base64::engine::general_purpose::STANDARD.encode(&buffer))
}

#[tauri::command]
pub fn decrypt_text(encrypted: String, password: String) -> Result<String, String> {
    let key = derive_key(&password);
    let iv = derive_iv();
    
    let mut encrypted_bytes = base64::engine::general_purpose::STANDARD.decode(&encrypted)
        .map_err(|e| format!("Base64 decode failed: {}", e))?;
    
    let cipher = Aes256CbcDec::new(&key.into(), &iv.into());
    let decrypted = cipher.decrypt_padded_mut::<Pkcs7>(&mut encrypted_bytes)
        .map_err(|e| format!("Decryption failed: {:?}", e))?;
    
    String::from_utf8(decrypted.to_vec())
        .map_err(|e| format!("UTF-8 decode failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ascii() {
        let enc = encrypt_text("hello world".into(), "pass".into()).unwrap();
        assert_eq!(decrypt_text(enc, "pass".into()).unwrap(), "hello world");
    }

    #[test]
    fn roundtrip_unicode() {
        let s = "Привет, 世界 🌍";
        let enc = encrypt_text(s.into(), "pw".into()).unwrap();
        assert_eq!(decrypt_text(enc, "pw".into()).unwrap(), s);
    }

    #[test]
    fn wrong_password_fails() {
        let enc = encrypt_text("secret".into(), "right".into()).unwrap();
        // Wrong key almost always breaks PKCS7 padding → error (not the plaintext).
        assert_ne!(decrypt_text(enc, "wrong".into()).ok().as_deref(), Some("secret"));
    }
}
