//! Opaque secret token encode/decode.
//!
//! An opaque token has the format `starpod:v1:<base64(nonce ++ ciphertext)>`.
//! The encrypted payload is `{"v": "<real_value>", "h": ["allowed_host", ...]}`.
//!
//! The proxy crate will scan outbound HTTP traffic for these tokens, decrypt
//! them, verify the target host is in the allow-list, and replace the token
//! with the real value before forwarding.

use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use starpod_core::{Result, StarpodError};

const TOKEN_PREFIX: &str = "starpod:v1:";

#[derive(Serialize, Deserialize)]
struct TokenPayload {
    /// The real secret value.
    v: String,
    /// Allowed hosts (empty = unrestricted).
    h: Vec<String>,
}

/// Encrypt a vault value into an opaque token.
///
/// Returns a string of the form `starpod:v1:<base64(nonce ++ ciphertext)>`.
/// Each call produces a unique token (fresh random nonce).
pub fn encode_opaque_token(
    cipher: &Aes256Gcm,
    value: &str,
    allowed_hosts: &[String],
) -> Result<String> {
    let payload = TokenPayload {
        v: value.to_string(),
        h: allowed_hosts.to_vec(),
    };
    let json = serde_json::to_vec(&payload)
        .map_err(|e| StarpodError::Vault(format!("Token payload serialization failed: {e}")))?;

    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, json.as_ref())
        .map_err(|e| StarpodError::Vault(format!("Opaque token encryption failed: {e}")))?;

    // nonce (12 bytes) ++ ciphertext
    let mut blob = Vec::with_capacity(12 + ciphertext.len());
    blob.extend_from_slice(nonce.as_slice());
    blob.extend_from_slice(&ciphertext);

    Ok(format!(
        "{TOKEN_PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(&blob)
    ))
}

/// Decode an opaque token back to `(value, allowed_hosts)`.
///
/// Returns `Err` if the token format is invalid or decryption fails.
pub fn decode_opaque_token(cipher: &Aes256Gcm, token: &str) -> Result<(String, Vec<String>)> {
    let encoded = token
        .strip_prefix(TOKEN_PREFIX)
        .ok_or_else(|| StarpodError::Vault("Invalid opaque token prefix".into()))?;

    let blob = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| StarpodError::Vault(format!("Invalid opaque token base64: {e}")))?;

    if blob.len() < 13 {
        return Err(StarpodError::Vault("Opaque token too short".into()));
    }

    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| StarpodError::Vault(format!("Opaque token decryption failed: {e}")))?;

    let payload: TokenPayload = serde_json::from_slice(&plaintext)
        .map_err(|e| StarpodError::Vault(format!("Invalid opaque token payload: {e}")))?;

    Ok((payload.v, payload.h))
}

/// Returns `true` if the string looks like an opaque token.
pub fn is_opaque_token(s: &str) -> bool {
    s.starts_with(TOKEN_PREFIX)
}

#[cfg(test)]
mod tests {
    use aes_gcm::KeyInit;

    use super::*;

    fn test_cipher() -> Aes256Gcm {
        Aes256Gcm::new_from_slice(&[0xAB; 32]).unwrap()
    }

    #[test]
    fn round_trip() {
        let cipher = test_cipher();
        let hosts = vec!["api.github.com".into(), "*.github.com".into()];
        let token = encode_opaque_token(&cipher, "ghp_secret123", &hosts).unwrap();

        assert!(is_opaque_token(&token));
        assert!(token.starts_with("starpod:v1:"));

        let (value, decoded_hosts) = decode_opaque_token(&cipher, &token).unwrap();
        assert_eq!(value, "ghp_secret123");
        assert_eq!(decoded_hosts, hosts);
    }

    #[test]
    fn empty_hosts() {
        let cipher = test_cipher();
        let token = encode_opaque_token(&cipher, "sk-key", &[]).unwrap();
        let (value, hosts) = decode_opaque_token(&cipher, &token).unwrap();
        assert_eq!(value, "sk-key");
        assert!(hosts.is_empty());
    }

    #[test]
    fn unique_tokens_per_call() {
        let cipher = test_cipher();
        let t1 = encode_opaque_token(&cipher, "same", &[]).unwrap();
        let t2 = encode_opaque_token(&cipher, "same", &[]).unwrap();
        // Different nonces → different tokens
        assert_ne!(t1, t2);
        // But both decode to the same value
        let (v1, _) = decode_opaque_token(&cipher, &t1).unwrap();
        let (v2, _) = decode_opaque_token(&cipher, &t2).unwrap();
        assert_eq!(v1, v2);
    }

    #[test]
    fn wrong_key_fails() {
        let cipher1 = test_cipher();
        let cipher2 = Aes256Gcm::new_from_slice(&[0xCD; 32]).unwrap();
        let token = encode_opaque_token(&cipher1, "secret", &[]).unwrap();
        assert!(decode_opaque_token(&cipher2, &token).is_err());
    }

    #[test]
    fn invalid_prefix() {
        let cipher = test_cipher();
        assert!(decode_opaque_token(&cipher, "not:a:token").is_err());
    }

    #[test]
    fn truncated_token() {
        let cipher = test_cipher();
        assert!(decode_opaque_token(&cipher, "starpod:v1:AAAA").is_err());
    }

    #[test]
    fn is_opaque_token_check() {
        assert!(is_opaque_token("starpod:v1:abc123"));
        assert!(!is_opaque_token("ghp_abc123"));
        assert!(!is_opaque_token(""));
    }

    // ── Stress tests ─────────────────────────────────────────────

    #[test]
    fn empty_value() {
        let cipher = test_cipher();
        let token = encode_opaque_token(&cipher, "", &[]).unwrap();
        let (value, hosts) = decode_opaque_token(&cipher, &token).unwrap();
        assert_eq!(value, "");
        assert!(hosts.is_empty());
    }

    #[test]
    fn large_value() {
        let cipher = test_cipher();
        // 1 MB secret value
        let big = "x".repeat(1_000_000);
        let token = encode_opaque_token(&cipher, &big, &[]).unwrap();
        let (value, _) = decode_opaque_token(&cipher, &token).unwrap();
        assert_eq!(value.len(), 1_000_000);
    }

    #[test]
    fn unicode_value_and_hosts() {
        let cipher = test_cipher();
        let value = "sk-ключ-🔐-密钥-مفتاح";
        let hosts = vec!["api.例え.jp".into(), "api.مثال.com".into()];
        let token = encode_opaque_token(&cipher, value, &hosts).unwrap();
        let (decoded_value, decoded_hosts) = decode_opaque_token(&cipher, &token).unwrap();
        assert_eq!(decoded_value, value);
        assert_eq!(decoded_hosts, hosts);
    }

    #[test]
    fn value_with_special_json_chars() {
        let cipher = test_cipher();
        // Value containing JSON special characters that could break serialization
        let value = r#"key"with\back/slash{braces}[brackets]null:true,false"#;
        let token = encode_opaque_token(&cipher, value, &[]).unwrap();
        let (decoded, _) = decode_opaque_token(&cipher, &token).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn value_with_newlines_and_control_chars() {
        let cipher = test_cipher();
        let value = "line1\nline2\ttab\r\n\0null";
        let token = encode_opaque_token(&cipher, value, &[]).unwrap();
        let (decoded, _) = decode_opaque_token(&cipher, &token).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn many_hosts() {
        let cipher = test_cipher();
        let hosts: Vec<String> = (0..500).map(|i| format!("host-{i}.example.com")).collect();
        let token = encode_opaque_token(&cipher, "val", &hosts).unwrap();
        let (_, decoded_hosts) = decode_opaque_token(&cipher, &token).unwrap();
        assert_eq!(decoded_hosts.len(), 500);
    }

    #[test]
    fn token_with_base64_padding_edge() {
        let cipher = test_cipher();
        // Different value lengths to exercise base64 padding (0, 1, 2 pad chars)
        for len in 0..=20 {
            let value = "a".repeat(len);
            let token = encode_opaque_token(&cipher, &value, &[]).unwrap();
            let (decoded, _) = decode_opaque_token(&cipher, &token).unwrap();
            assert_eq!(decoded, value, "failed at len={len}");
        }
    }

    #[test]
    fn tampered_token_fails() {
        let cipher = test_cipher();
        let token = encode_opaque_token(&cipher, "secret", &["api.x.com".into()]).unwrap();

        // Flip a byte in the middle of the base64 payload
        let mut chars: Vec<char> = token.chars().collect();
        let mid = chars.len() / 2;
        chars[mid] = if chars[mid] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();

        // Should fail — either base64 decode or AEAD auth tag
        assert!(decode_opaque_token(&cipher, &tampered).is_err());
    }

    #[test]
    fn truncated_ciphertext_fails() {
        let cipher = test_cipher();
        let token = encode_opaque_token(&cipher, "secret", &[]).unwrap();
        // Keep prefix + first 20 chars of base64 (truncated ciphertext)
        let truncated = &token[..TOKEN_PREFIX.len() + 20];
        assert!(decode_opaque_token(&cipher, truncated).is_err());
    }

    #[test]
    fn concurrent_encode_decode() {
        use std::sync::Arc;
        use std::thread;

        let cipher = Arc::new(test_cipher());
        let mut handles = vec![];

        for i in 0..100 {
            let cipher = Arc::clone(&cipher);
            handles.push(thread::spawn(move || {
                let value = format!("secret-{i}");
                let hosts = vec![format!("host-{i}.com")];
                let token = encode_opaque_token(&cipher, &value, &hosts).unwrap();
                let (decoded_value, decoded_hosts) = decode_opaque_token(&cipher, &token).unwrap();
                assert_eq!(decoded_value, value);
                assert_eq!(decoded_hosts, hosts);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn prefix_only_is_invalid() {
        let cipher = test_cipher();
        assert!(decode_opaque_token(&cipher, "starpod:v1:").is_err());
    }

    #[test]
    fn token_not_reusable_across_keys() {
        // Verify a token encrypted with key A cannot be decrypted with key B
        let cipher_a = Aes256Gcm::new_from_slice(&[0x01; 32]).unwrap();
        let cipher_b = Aes256Gcm::new_from_slice(&[0x02; 32]).unwrap();

        let token = encode_opaque_token(&cipher_a, "secret", &["host.com".into()]).unwrap();

        // Key B cannot decrypt key A's token
        assert!(decode_opaque_token(&cipher_b, &token).is_err());
        // Key A can
        let (val, hosts) = decode_opaque_token(&cipher_a, &token).unwrap();
        assert_eq!(val, "secret");
        assert_eq!(hosts, vec!["host.com"]);
    }
}
