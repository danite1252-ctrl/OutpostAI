//! PINs, passwords and bearer tokens.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::{CoreError, CoreResult};

/// Prefix on every Outpost bearer token, so they are easy to spot in logs and configs.
pub const TOKEN_PREFIX: &str = "op_";

/// Check a new PIN or password against the rules:
/// a PIN is 4–12 digits; anything else is a password of 6–128 characters.
pub fn validate_secret(secret: &str) -> CoreResult<()> {
    let len = secret.chars().count();
    let is_pin = !secret.is_empty() && secret.chars().all(|c| c.is_ascii_digit());
    if is_pin {
        if !(4..=12).contains(&len) {
            return Err(CoreError::Invalid("A PIN must be 4 to 12 digits.".into()));
        }
    } else if !(6..=128).contains(&len) {
        return Err(CoreError::Invalid(
            "A password must be 6 to 128 characters (or use a 4–12 digit PIN).".into(),
        ));
    }
    Ok(())
}

/// Hash a PIN or password with Argon2id. Slow on purpose; call from a blocking task.
pub fn hash_secret(secret: &str) -> CoreResult<String> {
    let salt_bytes = random_bytes::<16>()?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| CoreError::Internal(e.to_string()))?;
    Argon2::default()
        .hash_password(secret.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| CoreError::Internal(e.to_string()))
}

/// Check a PIN or password against a stored hash. Slow on purpose; call from a blocking task.
pub fn verify_secret(secret: &str, stored_hash: &str) -> bool {
    match PasswordHash::new(stored_hash) {
        Ok(parsed) => Argon2::default().verify_password(secret.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/// A new random bearer token and its storage hash. Only the hash is ever saved.
pub fn new_token() -> CoreResult<(String, String)> {
    let token = format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(random_bytes::<32>()?));
    let hash = hash_token(&token);
    Ok((token, hash))
}

/// SHA-256 of a token, hex encoded. Tokens are long and random, so a fast hash is enough.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_bytes<const N: usize>() -> CoreResult<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf).map_err(|e| CoreError::Internal(format!("no randomness: {e}")))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_and_password_rules() {
        assert!(validate_secret("1234").is_ok());
        assert!(validate_secret("123456789012").is_ok());
        assert!(validate_secret("123").is_err());
        assert!(validate_secret("1234567890123").is_err());
        assert!(validate_secret("abc12").is_err());
        assert!(validate_secret("correct horse").is_ok());
        assert!(validate_secret("").is_err());
    }

    #[test]
    fn hash_round_trip() {
        let hash = hash_secret("4821").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_secret("4821", &hash));
        assert!(!verify_secret("4822", &hash));
        assert!(!verify_secret("4821", "not-a-hash"));
    }

    #[test]
    fn tokens_are_unique_and_hashed() {
        let (a, ha) = new_token().unwrap();
        let (b, hb) = new_token().unwrap();
        assert!(a.starts_with(TOKEN_PREFIX));
        assert_ne!(a, b);
        assert_ne!(ha, hb);
        assert_eq!(hash_token(&a), ha);
        assert_eq!(ha.len(), 64);
    }
}
