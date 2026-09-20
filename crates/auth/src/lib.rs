//! Password hashing (argon2id) and signed bearer tokens.
//!
//! A token is `base64url("{user_id}.{expires_at}")` + `"."` +
//! `base64url(hmac_sha256(secret, payload))`. These are pure functions with no
//! storage or clock dependency, so the crypto is unit-testable natively — the
//! backend is a wasm cdylib whose own tests never run in CI.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Error from password hashing.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid salt")]
    InvalidSalt,
    #[error("argon2 params: {0}")]
    Argon2Params(String),
    #[error("hash password: {0}")]
    HashPassword(String),
}

/// Hash a password with argon2id.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version, password_hash::SaltString};
    use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
    // 16 random bytes, base64-encoded (no padding — the PHC format) as the salt.
    let mut salt_bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut salt_bytes);
    let salt = SaltString::from_b64(&STANDARD_NO_PAD.encode(salt_bytes))
        .map_err(|_| AuthError::InvalidSalt)?;
    let params =
        Params::new(19_456, 2, 1, None).map_err(|e| AuthError::Argon2Params(e.to_string()))?;
    let hash = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AuthError::HashPassword(e.to_string()))?;
    Ok(hash.to_string())
}

/// Verify a password against a stored argon2id hash.
pub fn verify_password(password: &str, hash: &str) -> bool {
    use argon2::{Argon2, PasswordVerifier, password_hash::PasswordHash};
    PasswordHash::new(hash)
        .ok()
        .and_then(|ph| {
            Argon2::default()
                .verify_password(password.as_bytes(), &ph)
                .ok()
        })
        .is_some()
}

/// Sign a bearer token for `user_id`, valid until `expires_at` (unix seconds).
pub fn sign_token_expires(secret: &str, user_id: i64, expires_at: i64) -> String {
    let payload = format!("{user_id}.{expires_at}");
    let signature = hmac(secret, payload.as_bytes());
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(payload.as_bytes()),
        URL_SAFE_NO_PAD.encode(signature)
    )
}

/// Verify a bearer token at time `now` (unix seconds), returning the user id
/// if the signature is valid and the token is unexpired.
pub fn verify_token_at(secret: &str, token: &str, now: i64) -> Option<i64> {
    let (payload_b64, sig_b64) = token.split_once('.')?;
    let payload_bytes = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let payload = std::str::from_utf8(&payload_bytes).ok()?;
    let expected = hmac(secret, payload.as_bytes());
    let actual = URL_SAFE_NO_PAD.decode(sig_b64).ok()?;
    if !constant_time_eq(&expected, &actual) {
        return None;
    }
    let (user_id, expires_at) = payload.split_once('.')?;
    let user_id: i64 = user_id.parse().ok()?;
    let expires_at: i64 = expires_at.parse().ok()?;
    (expires_at >= now).then_some(user_id)
}

fn hmac(secret: &str, payload: &[u8]) -> Vec<u8> {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(payload);
    mac.finalize().into_bytes().to_vec()
}

/// Constant-time byte comparison to avoid a timing side-channel.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    #[test]
    fn token_roundtrip() {
        let secret = "test-secret";
        let token = sign_token_expires(secret, 7, NOW + 100);
        assert_eq!(verify_token_at(secret, &token, NOW), Some(7));
    }

    #[test]
    fn token_rejects_wrong_secret() {
        let token = sign_token_expires("secret-a", 7, NOW + 100);
        assert_eq!(verify_token_at("secret-b", &token, NOW), None);
    }

    #[test]
    fn token_rejects_expired() {
        let secret = "test-secret";
        let token = sign_token_expires(secret, 7, NOW - 1);
        assert_eq!(verify_token_at(secret, &token, NOW), None);
    }

    #[test]
    fn token_rejects_tampered_payload() {
        let secret = "test-secret";
        let token = sign_token_expires(secret, 7, NOW + 100);
        let (payload, sig) = token.split_once('.').unwrap();
        // Change the user id in the payload; the signature no longer matches.
        let mut forged = URL_SAFE_NO_PAD.decode(payload).unwrap();
        forged[0] = b'8';
        let forged = format!("{}.{}", URL_SAFE_NO_PAD.encode(forged), sig);
        assert_eq!(verify_token_at(secret, &forged, NOW), None);
    }

    #[test]
    fn password_hash_roundtrip() {
        let hash = hash_password("hunter2").unwrap();
        assert!(verify_password("hunter2", &hash));
        assert!(!verify_password("wrong", &hash));
    }
}
