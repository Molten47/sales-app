use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AppState, api::error::AppError, auth, domain::otp};

#[derive(Deserialize)]
pub struct RequestOtp {
    pub email: String,
}

/// Always answers 202, whether or not the email exists, so the endpoint
/// can't be used to discover which emails are registered.
pub async fn request_otp(
    State(state): State<AppState>,
    Json(body): Json<RequestOtp>,
) -> Result<StatusCode, AppError> {
    let email = body.email.trim().to_lowercase();

    if let Some((user_id, _)) = otp::find_user_by_email(&state.db, &email).await? {
        let code = auth::generate_code();
        let hash = auth::hash_code(&state.jwt_secret, user_id, &code);
        otp::store(&state.db, user_id, &hash).await?;
        // DEMO ONLY: stands in for an email/SMS provider
        tracing::info!(%email, %code, "OTP issued (demo: delivered via log)");
    }
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
pub struct VerifyOtp {
    pub email: String,
    pub code: String,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub user_id: Uuid,
    pub role: String,
}

pub async fn verify_otp(
    State(state): State<AppState>,
    Json(body): Json<VerifyOtp>,
) -> Result<Json<TokenResponse>, AppError> {
    let email = body.email.trim().to_lowercase();

    // every failure below is the same 401, so the response reveals nothing
    let (user_id, role) = otp::find_user_by_email(&state.db, &email)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let expected = otp::begin_attempt(&state.db, user_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    if !auth::code_matches(&state.jwt_secret, user_id, body.code.trim(), &expected) {
        return Err(AppError::Unauthorized);
    }
    if !otp::consume(&state.db, user_id).await? {
        return Err(AppError::Unauthorized);
    }

    Ok(Json(TokenResponse {
        token: auth::issue_token(&state.jwt_secret, user_id, &role),
        token_type: "Bearer",
        expires_in: auth::TOKEN_TTL_SECS,
        user_id,
        role,
    }))
}
