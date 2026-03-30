//! Token scanning and replacement in byte buffers.
//!
//! Finds `starpod:v1:` opaque tokens in arbitrary data (headers, bodies),
//! decrypts them, verifies host binding, and replaces with real values.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use tracing::warn;

use crate::host_match::host_matches;

const TOKEN_PREFIX: &[u8] = b"starpod:v1:";
const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";

/// Result of scanning a buffer for opaque tokens.
#[derive(Debug)]
pub struct ScanResult {
    /// The buffer with tokens replaced (or stripped on host mismatch).
    pub data: Vec<u8>,
    /// Number of tokens successfully replaced with real values.
    pub replaced: usize,
    /// Number of tokens stripped due to host mismatch.
    pub stripped: usize,
}

/// Decode an opaque token payload. Returns `(value, allowed_hosts)`.
fn decode_token(cipher: &Aes256Gcm, token: &str) -> Option<(String, Vec<String>)> {
    let encoded = token.strip_prefix("starpod:v1:")?;

    let blob = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;

    if blob.len() < 13 {
        return None;
    }

    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext).ok()?;

    #[derive(serde::Deserialize)]
    struct Payload {
        v: String,
        h: Vec<String>,
    }

    let payload: Payload = serde_json::from_slice(&plaintext).ok()?;
    Some((payload.v, payload.h))
}

/// Scan `data` for `starpod:v1:` tokens and replace/strip them.
///
/// For each token found:
/// - Decrypt and check host binding against `target_host`
/// - Host match → replace token with real value
/// - Host mismatch → strip token (replace with empty)
/// - Decode failure → leave token as-is
pub fn scan_and_replace(cipher: &Aes256Gcm, data: &[u8], target_host: &str) -> ScanResult {
    let mut result = Vec::with_capacity(data.len());
    let mut replaced = 0usize;
    let mut stripped = 0usize;
    let mut i = 0;

    while i < data.len() {
        // Look for the token prefix
        if data[i..].starts_with(TOKEN_PREFIX) {
            let token_start = i;
            i += TOKEN_PREFIX.len();

            // Consume base64 characters
            while i < data.len() && BASE64_CHARS.contains(&data[i]) {
                i += 1;
            }

            let token_bytes = &data[token_start..i];
            let token_str = match std::str::from_utf8(token_bytes) {
                Ok(s) => s,
                Err(_) => {
                    // Not valid UTF-8 — leave as-is
                    result.extend_from_slice(token_bytes);
                    continue;
                }
            };

            match decode_token(cipher, token_str) {
                Some((value, allowed_hosts)) => {
                    if host_matches(target_host, &allowed_hosts) {
                        result.extend_from_slice(value.as_bytes());
                        replaced += 1;
                    } else {
                        warn!(
                            target_host = %target_host,
                            allowed_hosts = ?allowed_hosts,
                            "Token host mismatch — stripped"
                        );
                        // Strip: don't write anything (token removed)
                        stripped += 1;
                    }
                }
                None => {
                    // Decode failed — leave token as-is
                    result.extend_from_slice(token_bytes);
                }
            }
        } else {
            result.push(data[i]);
            i += 1;
        }
    }

    ScanResult {
        data: result,
        replaced,
        stripped,
    }
}

/// Convenience wrapper for string data.
pub fn scan_and_replace_str(cipher: &Aes256Gcm, data: &str, target_host: &str) -> ScanResult {
    scan_and_replace(cipher, data.as_bytes(), target_host)
}

/// Create a cipher from a 32-byte master key.
pub fn cipher_from_key(master_key: &[u8; 32]) -> Aes256Gcm {
    Aes256Gcm::new_from_slice(master_key).expect("32-byte key is always valid for AES-256")
}

#[cfg(test)]
mod tests {
    use aes_gcm::aead::OsRng;
    use aes_gcm::AeadCore;

    use super::*;

    fn test_cipher() -> Aes256Gcm {
        cipher_from_key(&[0xAB; 32])
    }

    fn encode_token(cipher: &Aes256Gcm, value: &str, hosts: &[String]) -> String {
        #[derive(serde::Serialize)]
        struct Payload {
            v: String,
            h: Vec<String>,
        }
        let payload = Payload {
            v: value.to_string(),
            h: hosts.to_vec(),
        };
        let json = serde_json::to_vec(&payload).unwrap();
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher.encrypt(&nonce, json.as_ref()).unwrap();
        let mut blob = Vec::with_capacity(12 + ciphertext.len());
        blob.extend_from_slice(nonce.as_slice());
        blob.extend_from_slice(&ciphertext);
        format!(
            "starpod:v1:{}",
            base64::engine::general_purpose::STANDARD.encode(&blob)
        )
    }

    #[test]
    fn replace_token_in_header() {
        let cipher = test_cipher();
        let token = encode_token(&cipher, "ghp_real", &["api.github.com".into()]);
        let header = format!("Bearer {token}");

        let result = scan_and_replace_str(&cipher, &header, "api.github.com");
        assert_eq!(result.replaced, 1);
        assert_eq!(result.stripped, 0);
        assert_eq!(String::from_utf8(result.data).unwrap(), "Bearer ghp_real");
    }

    #[test]
    fn strip_token_on_host_mismatch() {
        let cipher = test_cipher();
        let token = encode_token(&cipher, "ghp_real", &["api.github.com".into()]);
        let header = format!("Bearer {token}");

        let result = scan_and_replace_str(&cipher, &header, "evil.com");
        assert_eq!(result.replaced, 0);
        assert_eq!(result.stripped, 1);
        assert_eq!(String::from_utf8(result.data).unwrap(), "Bearer ");
    }

    #[test]
    fn unrestricted_token_matches_any_host() {
        let cipher = test_cipher();
        let token = encode_token(&cipher, "secret", &[]);
        let data = format!("key={token}");

        let result = scan_and_replace_str(&cipher, &data, "any-host.com");
        assert_eq!(result.replaced, 1);
        assert_eq!(String::from_utf8(result.data).unwrap(), "key=secret");
    }

    #[test]
    fn multiple_tokens_in_one_buffer() {
        let cipher = test_cipher();
        let t1 = encode_token(&cipher, "val1", &[]);
        let t2 = encode_token(&cipher, "val2", &[]);
        let data = format!("a={t1}&b={t2}");

        let result = scan_and_replace_str(&cipher, &data, "host.com");
        assert_eq!(result.replaced, 2);
        assert_eq!(String::from_utf8(result.data).unwrap(), "a=val1&b=val2");
    }

    #[test]
    fn no_tokens_passes_through() {
        let cipher = test_cipher();
        let data = "just normal data with no tokens";
        let result = scan_and_replace_str(&cipher, data, "host.com");
        assert_eq!(result.replaced, 0);
        assert_eq!(result.stripped, 0);
        assert_eq!(String::from_utf8(result.data).unwrap(), data);
    }

    #[test]
    fn wrong_key_leaves_token_as_is() {
        let cipher1 = test_cipher();
        let cipher2 = cipher_from_key(&[0xCD; 32]);
        let token = encode_token(&cipher1, "secret", &[]);
        let data = format!("key={token}");

        // Try to scan with wrong cipher — token left as-is
        let result = scan_and_replace_str(&cipher2, &data, "host.com");
        assert_eq!(result.replaced, 0);
        assert_eq!(result.stripped, 0);
        assert_eq!(String::from_utf8(result.data).unwrap(), data);
    }

    #[test]
    fn token_at_end_of_buffer() {
        let cipher = test_cipher();
        let token = encode_token(&cipher, "val", &[]);
        let data = format!("Authorization: {token}");

        let result = scan_and_replace_str(&cipher, &data, "x.com");
        assert_eq!(result.replaced, 1);
        assert_eq!(
            String::from_utf8(result.data).unwrap(),
            "Authorization: val"
        );
    }

    #[test]
    fn token_at_start_of_buffer() {
        let cipher = test_cipher();
        let token = encode_token(&cipher, "val", &[]);

        let result = scan_and_replace_str(&cipher, &token, "x.com");
        assert_eq!(result.replaced, 1);
        assert_eq!(String::from_utf8(result.data).unwrap(), "val");
    }

    #[test]
    fn large_body_with_embedded_token() {
        let cipher = test_cipher();
        let token = encode_token(&cipher, "secret", &[]);
        // Use non-base64 chars for padding to avoid ambiguity
        let padding = "-".repeat(100_000);
        let data = format!("{padding}token={token}{padding}");

        let result = scan_and_replace_str(&cipher, &data, "host.com");
        assert_eq!(result.replaced, 1);
        let expected = format!("{padding}token=secret{padding}");
        assert_eq!(String::from_utf8(result.data).unwrap(), expected);
    }

    #[test]
    fn mixed_match_and_mismatch() {
        let cipher = test_cipher();
        let good = encode_token(&cipher, "good", &["ok.com".into()]);
        let bad = encode_token(&cipher, "bad", &["other.com".into()]);
        let data = format!("a={good}&b={bad}");

        let result = scan_and_replace_str(&cipher, &data, "ok.com");
        assert_eq!(result.replaced, 1);
        assert_eq!(result.stripped, 1);
        assert_eq!(String::from_utf8(result.data).unwrap(), "a=good&b=");
    }
}
