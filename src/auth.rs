use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use hmac::{Hmac, Mac};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

use crate::{AppState, api::error::AppError};

pub const TOKEN_TTL_SECS: u64 = 8 * 60 * 60;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub role: String,
    pub exp: u64,
}

pub fn issue_token(secret: &str, user_id: Uuid, role: &str) -> String {
    let claims = Claims {
        sub: user_id,
        role: role.to_owned(),
        exp: chrono::Utc::now().timestamp() as u64 + TOKEN_TTL_SECS,
    };
    // HS256 with serializable claims cannot fail
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("signing a JWT with an HMAC key cannot fail")
}

/// Checks signature and expiry (HS256 only).
pub fn verify_token(secret: &str, token: &str) -> Result<Claims, AppError> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map(|d| d.claims)
    .map_err(|_| AppError::Unauthorized)
}

// ---- OTP helpers ----

pub fn generate_code() -> String {
    format!("{:06}", rand::thread_rng().gen_range(0..1_000_000u32))
}

fn otp_mac(secret: &str, user_id: Uuid, code: &str) -> Hmac<Sha256> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(b"otp:");
    mac.update(user_id.as_bytes());
    mac.update(code.as_bytes());
    mac
}

pub fn hash_code(secret: &str, user_id: Uuid, code: &str) -> Vec<u8> {
    otp_mac(secret, user_id, code)
        .finalize()
        .into_bytes()
        .to_vec()
}

/// Constant-time comparison.
pub fn code_matches(secret: &str, user_id: Uuid, code: &str, expected: &[u8]) -> bool {
    otp_mac(secret, user_id, code)
        .verify_slice(expected)
        .is_ok()
}

// ---- extractor: add `user: AuthUser` to any handler to require a login ----

pub struct AuthUser {
    pub id: Uuid,
    pub role: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(AppError::Unauthorized)?;
        let claims = verify_token(&state.jwt_secret, token)?;
        Ok(AuthUser {
            id: claims.sub,
            role: claims.role,
        })
    }
}
