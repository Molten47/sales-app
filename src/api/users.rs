use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::Deserialize;

use super::error::AppError;
use crate::{
    AppState,
    domain::users::{self, User},
};

#[derive(Deserialize)]
pub struct CreateUser {
    email: String,
    name: String,
    role: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/users", post(create))
}

// DEV SCAFFOLDING: open endpoint, anyone can create a staff user.
// Gets locked behind auth when OTP login lands.
async fn create(
    State(s): State<AppState>,
    Json(req): Json<CreateUser>,
) -> Result<(StatusCode, Json<User>), AppError> {
    let email = req.email.trim().to_lowercase();
    if !email.contains('@') || email.len() > 254 {
        return Err(AppError::BadRequest("invalid email".into()));
    }
    let name = req.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::BadRequest("name must be 1-100 characters".into()));
    }
    let role = req.role.unwrap_or_else(|| "customer".into());
    if role != "customer" && role != "staff" {
        return Err(AppError::BadRequest(
            "role must be customer or staff".into(),
        ));
    }

    let user = users::create_user(&s.db, &email, name, &role).await?;
    Ok((StatusCode::CREATED, Json(user)))
}
