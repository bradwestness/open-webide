use crate::ServerConfig;
use crate::secret::constant_time_eq;
use openwebide_auth::verify_token_at;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    User { user_id: i64 },
    Paired,
}

pub fn authenticate(token: &str, cfg: &ServerConfig, now: i64) -> Result<Principal, String> {
    if let Some(ref pairing_token) = cfg.pairing_token
        && constant_time_eq(token.as_bytes(), pairing_token.as_bytes())
    {
        return Ok(Principal::Paired);
    }

    match verify_token_at(&cfg.secret, token, now) {
        Some(claims) => Ok(Principal::User {
            user_id: claims.user_id,
        }),
        None => Err("sign-in rejected: bridge token invalid or expired".to_string()),
    }
}
