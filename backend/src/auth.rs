//! Local account authentication: password hashing and signed bearer tokens.
//!
//! The pure crypto (argon2id hashing, HMAC token sign/verify) lives in the
//! `openwebide-auth` crate so it is unit-testable natively; this module wires
//! it to the app: the signing secret is a random 32-byte value persisted in
//! the `settings` table, so tokens stay valid across requests (Spin
//! components are otherwise stateless).

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use openwebide_core::User;
use rand::Rng;

use crate::error::ApiError;
use crate::state::{AppState, now};

/// Token lifetime: 30 days.
const TOKEN_TTL_SECS: i64 = 30 * 24 * 60 * 60;
/// The `settings` key holding the token-signing secret.
const SECRET_KEY: &str = "auth_secret";

/// Hash a password with argon2id.
pub fn hash_password(password: &str) -> Result<String, ApiError> {
    openwebide_auth::hash_password(password).map_err(|e| ApiError::internal(e.to_string()))
}

/// Verify a password against a stored argon2id hash.
pub fn verify_password(password: &str, hash: &str) -> bool {
    openwebide_auth::verify_password(password, hash)
}

/// Read the token-signing secret, creating and persisting it on first use.
async fn get_or_create_secret(state: &AppState) -> Result<String, ApiError> {
    if let Some(secret) = state.store.get_setting(SECRET_KEY).await? {
        return Ok(secret);
    }
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let secret = URL_SAFE_NO_PAD.encode(bytes);
    state.store.set_setting(SECRET_KEY, &secret).await?;
    Ok(secret)
}

/// Sign a bearer token for `user_id`, valid until `now + TOKEN_TTL_SECS`.
fn sign_token(secret: &str, user_id: i64) -> String {
    openwebide_auth::sign_token_expires(secret, user_id, now() + TOKEN_TTL_SECS)
}

/// Verify a bearer token, returning the user id if valid and unexpired.
fn verify_token(secret: &str, token: &str) -> Option<i64> {
    openwebide_auth::verify_token_at(secret, token, now())
}

/// Issue a signed bearer token for `user_id`.
pub async fn issue_token(state: &AppState, user_id: i64) -> Result<String, ApiError> {
    let secret = get_or_create_secret(state).await?;
    Ok(sign_token(&secret, user_id))
}

/// Authenticate a request from its bearer token, returning the account.
pub async fn authenticate(state: &AppState, token: Option<&str>) -> Result<User, ApiError> {
    let token = token.ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
    let secret = get_or_create_secret(state).await?;
    let user_id = verify_token(&secret, token)
        .ok_or_else(|| ApiError::unauthorized("invalid or expired token"))?;
    state
        .store
        .get_user(user_id)
        .await?
        .map(|u| u.public())
        .ok_or_else(|| ApiError::unauthorized("unknown user"))
}
