//! API key generation, hashing, and verification.
//!
//! Keys follow the format `sp_live_` + 40 random hex characters (48 chars total).
//! The first 8 hex chars after the prefix are stored as a lookup index so that
//! authentication requires only 1–2 argon2id verifications instead of a full
//! table scan.
//!
//! ## Security properties
//!
//! - Keys are generated from 20 bytes of cryptographic randomness (160 bits).
//! - Hashing uses argon2id with default parameters (memory-hard, resistant to
//!   GPU/ASIC attacks).
//! - Each hash uses a unique random salt, so identical keys produce different hashes.
//! - Verification is constant-time (provided by the argon2 library).

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::password_hash::SaltString;
use rand::Rng;

/// API key prefix: `sp_live_`
const KEY_PREFIX: &str = "sp_live_";

/// Length of the stored prefix for DB lookup (first 8 hex chars of random part).
const STORED_PREFIX_LEN: usize = 8;

/// Generate a new API key: `sp_live_` + 40 random hex chars (48 chars total).
pub fn generate_key() -> String {
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 20];
    rng.fill(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    format!("{}{}", KEY_PREFIX, hex)
}

/// Extract the lookup prefix (first 8 hex chars after `sp_live_`) from a full key.
///
/// Returns `None` if the key doesn't start with `sp_live_` or is too short.
/// Used to narrow candidate rows before expensive argon2 verification.
pub fn extract_prefix(key: &str) -> Option<&str> {
    let rest = key.strip_prefix(KEY_PREFIX)?;
    if rest.len() < STORED_PREFIX_LEN {
        return None;
    }
    Some(&rest[..STORED_PREFIX_LEN])
}

/// Hash a full API key with argon2id using a random salt.
///
/// The returned string is in PHC format and includes the algorithm, salt,
/// and hash — everything needed for verification.
pub fn hash_key(key: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut rand::thread_rng());
    let argon2 = Argon2::default();
    let hash = argon2.hash_password(key.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// Verify a full API key against a stored argon2id hash.
///
/// Returns `false` for malformed hashes or mismatches.
pub fn verify_key(key: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(key.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_format() {
        let key = generate_key();
        assert!(key.starts_with("sp_live_"));
        assert_eq!(key.len(), KEY_PREFIX.len() + 40);
        // All chars after prefix should be hex
        assert!(key[KEY_PREFIX.len()..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn extract_prefix_valid() {
        let key = "sp_live_abcdef0123456789abcdef0123456789abcdef01";
        let prefix = extract_prefix(key).unwrap();
        assert_eq!(prefix, "abcdef01");
    }

    #[test]
    fn extract_prefix_invalid_prefix() {
        assert!(extract_prefix("sk_live_abc").is_none());
    }

    #[test]
    fn extract_prefix_too_short() {
        assert!(extract_prefix("sp_live_abc").is_none());
    }

    #[test]
    fn extract_prefix_empty_string() {
        assert!(extract_prefix("").is_none());
    }

    #[test]
    fn extract_prefix_exact_prefix_only() {
        assert!(extract_prefix("sp_live_").is_none());
    }

    #[test]
    fn hash_and_verify() {
        let key = generate_key();
        let hash = hash_key(&key).unwrap();
        assert!(verify_key(&key, &hash));
        assert!(!verify_key("sp_live_wrong_key_here_0000000000000000", &hash));
    }

    #[test]
    fn different_salts_produce_different_hashes() {
        let key = generate_key();
        let h1 = hash_key(&key).unwrap();
        let h2 = hash_key(&key).unwrap();
        assert_ne!(h1, h2, "Same key should produce different hashes (random salt)");
        // Both should verify
        assert!(verify_key(&key, &h1));
        assert!(verify_key(&key, &h2));
    }

    #[test]
    fn verify_wrong_key_fails() {
        let key1 = generate_key();
        let key2 = generate_key();
        let hash = hash_key(&key1).unwrap();
        assert!(!verify_key(&key2, &hash));
    }

    #[test]
    fn verify_invalid_hash_fails() {
        let key = generate_key();
        assert!(!verify_key(&key, "not_a_valid_hash"));
    }

    #[test]
    fn verify_empty_hash_fails() {
        let key = generate_key();
        assert!(!verify_key(&key, ""));
    }

    #[test]
    fn generated_keys_are_unique() {
        let k1 = generate_key();
        let k2 = generate_key();
        assert_ne!(k1, k2);
    }

    #[test]
    fn single_bit_change_fails_verify() {
        let key = generate_key();
        let hash = hash_key(&key).unwrap();

        // Flip last character
        let mut tampered = key.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == '0' { '1' } else { '0' });

        assert!(!verify_key(&tampered, &hash));
    }
}
