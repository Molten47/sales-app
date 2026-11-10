use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::Deserialize;

use super::error::AppError;
use crate::{
    AppState,
    auth::StaffUser,
    domain::users::{self, User},
};

#[derive(Deserialize)]
pub struct CreateUser {
    email: String,
    name: String,
    role: Option<String>,
}

// no role field: this endpoint can only ever create staff
#[derive(Deserialize)]
pub struct CreateStaff {
    email: String,
    name: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users", post(signup))
        .route("/staff", post(create_staff))
}

/// Shared validation: returns the normalized (email, name).
fn validate(email: &str, name: &str) -> Result<(String, String), AppError> {
    let email = email.trim().to_lowercase();
    if !email.contains('@') || email.len() > 254 {
        return Err(AppError::BadRequest("invalid email".into()));
    }
    let name = name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::BadRequest("name must be 1-100 characters".into()));
    }
    Ok((email, name.to_owned()))
}

/// Public signup. Always creates a customer; asking for staff is refused.
async fn signup(
    State(s): State<AppState>,
    Json(req): Json<CreateUser>,
) -> Result<(StatusCode, Json<User>), AppError> {
    match req.role.as_deref() {
        None | Some("customer") => {}
        Some("staff") => return Err(AppError::Forbidden),
        Some(_) => {
            return Err(AppError::BadRequest(
                "role must be customer or staff".into(),
            ));
        }
    }
    let (email, name) = validate(&req.email, &req.name)?;

    let user = users::create_user(&s.db, &email, &name, "customer").await?;
    Ok((StatusCode::CREATED, Json(user)))
}

/// Staff-only: creates a staff account.
async fn create_staff(
    State(s): State<AppState>,
    staff: StaffUser,
    Json(req): Json<CreateStaff>,
) -> Result<(StatusCode, Json<User>), AppError> {
    let (email, name) = validate(&req.email, &req.name)?;

    let user = users::create_user(&s.db, &email, &name, "staff").await?;
    tracing::info!(created_by = %staff.id, new_staff = %user.id, "staff account created");
    Ok((StatusCode::CREATED, Json(user)))
}
