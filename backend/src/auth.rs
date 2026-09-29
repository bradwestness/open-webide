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
use spin_sdk::http::HeaderMap;

use crate::error::ApiError;
use crate::state::{AppState, unix_now_checked};

/// Token lifetime: 30 days.
const TOKEN_TTL_SECS: i64 = 30 * 24 * 60 * 60;
/// The `settings` key holding the token-signing secret.
const SECRET_KEY: &str = "auth_secret";

/// Hash a password with argon2id.
pub fn hash_password(password: &str) -> Result<String, ApiError> {
    openwebide_auth::hash_password(password).map_err(|e| ApiError::internal(e.to_string()))
}

pub fn verify_password_or_dummy(password: &str, hash: Option<&str>) -> bool {
    openwebide_auth::verify_password_or_dummy(password, hash)
}

pub fn session_cookie(headers: &HeaderMap) -> Option<String> {
    for cookie in headers
        .get_all("cookie")
        .iter()
        .filter_map(|h| h.to_str().ok())
    {
        for part in cookie.split(';') {
            let part = part.trim();
            if let Some(token) = part.strip_prefix("owide_session=") {
                return Some(token.to_string());
            }
        }
    }
    None
}

pub fn csrf_header_ok(headers: &HeaderMap) -> bool {
    headers.get("x-openwebide").and_then(|h| h.to_str().ok()) == Some("1")
}

pub fn is_https(headers: &HeaderMap) -> bool {
    let url = headers
        .get("spin-full-url")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    url.starts_with("https://") || proto == "https"
}

pub fn set_cookie(token: &str, secure: bool) -> String {
    let mut c =
        format!("owide_session={token}; Path=/api; HttpOnly; SameSite=Strict; Max-Age=2592000");
    if secure {
        c.push_str("; Secure");
    }
    c
}

pub fn clear_cookie(secure: bool) -> String {
    let mut c = "owide_session=; Path=/api; HttpOnly; SameSite=Strict; Max-Age=0".to_string();
    if secure {
        c.push_str("; Secure");
    }
    c
}

/// Read the token-signing secret, creating and persisting it on first use.
async fn get_or_create_secret(state: &AppState) -> Result<String, ApiError> {
    if let Some(secret) = state.store.get_setting(SECRET_KEY).await? {
        return Ok(secret);
    }
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    state
        .store
        .insert_setting_if_absent(SECRET_KEY, &URL_SAFE_NO_PAD.encode(bytes))
        .await?;
    state
        .store
        .get_setting(SECRET_KEY)
        .await?
        .ok_or_else(|| ApiError::internal("auth secret missing after insert"))
}

/// Sign a bearer token for `user_id`, valid until `now + TOKEN_TTL_SECS`.
fn sign_token(
    secret: &str,
    user_id: i64,
    now: Option<i64>,
    epoch: i64,
) -> Result<String, ApiError> {
    let now = now.ok_or_else(|| ApiError::internal("system clock unavailable"))?;
    Ok(openwebide_auth::sign_token_expires(
        secret,
        user_id,
        now + TOKEN_TTL_SECS,
        epoch,
    ))
}

/// Verify a bearer token at `now` (unix seconds), returning the user id if
/// valid and unexpired. A `None` clock fails closed.
fn verify_token(
    secret: &str,
    token: &str,
    now: Option<i64>,
) -> Result<Option<openwebide_auth::TokenClaims>, ApiError> {
    let now = now.ok_or_else(|| ApiError::internal("system clock unavailable"))?;
    Ok(openwebide_auth::verify_token_at(secret, token, now))
}

/// Issue a signed bearer token.
pub async fn issue_token(
    state: &AppState,
    user: &openwebide_storage::store::UserRecord,
) -> Result<String, ApiError> {
    let secret = get_or_create_secret(state).await?;
    sign_token(&secret, user.id, unix_now_checked(), user.token_epoch)
}

/// Authenticate a request from its headers, returning the account.
pub async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<User, ApiError> {
    let token = session_cookie(headers).ok_or_else(|| ApiError::unauthorized("not signed in"))?;
    if !csrf_header_ok(headers) {
        return Err(ApiError::unauthorized("not signed in"));
    }

    let secret = get_or_create_secret(state).await?;
    let claims = verify_token(&secret, &token, unix_now_checked())?
        .ok_or_else(|| ApiError::unauthorized("invalid or expired token"))?;

    let user = state
        .store
        .get_user(claims.user_id)
        .await?
        .ok_or_else(|| ApiError::unauthorized("unknown user"))?;

    if user.token_epoch != claims.epoch {
        return Err(ApiError::unauthorized("invalid or expired token"));
    }

    Ok(user.public())
}

#[cfg(test)]
mod tests {
    use super::*;
    use spin_sdk::http::HeaderMap;

    #[test]
    fn verify_token_rejects_expired_token() {
        let secret = "test-secret";
        let expired = openwebide_auth::sign_token_expires(secret, 7, 1_000, 2);
        assert!(matches!(
            verify_token(secret, &expired, Some(2_000)),
            Ok(None)
        ));

        let err = verify_token(secret, &expired, None).unwrap_err();
        assert_eq!(err.into_response().status().as_u16(), 500);
    }

    #[test]
    fn test_session_cookie() {
        let mut h = HeaderMap::new();
        h.append("cookie", "a=1; owide_session=x; b=2".parse().unwrap());
        assert_eq!(session_cookie(&h).as_deref(), Some("x"));
    }

    #[test]
    fn test_csrf_header_ok() {
        let mut h = HeaderMap::new();
        assert!(!csrf_header_ok(&h));
        h.insert("x-openwebide", "1".parse().unwrap());
        assert!(csrf_header_ok(&h));
    }

    #[test]
    fn test_cookie_flags() {
        assert_eq!(
            set_cookie("t", false),
            "owide_session=t; Path=/api; HttpOnly; SameSite=Strict; Max-Age=2592000"
        );
        assert_eq!(
            set_cookie("t", true),
            "owide_session=t; Path=/api; HttpOnly; SameSite=Strict; Max-Age=2592000; Secure"
        );
    }
}
