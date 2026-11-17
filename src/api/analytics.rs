use axum::{Json, Router, extract::State, routing::get};

use super::error::AppError;
use crate::{
    AppState,
    auth::AuthUser,
    domain::analytics::{self, Summary},
};

pub fn router() -> Router<AppState> {
    Router::new().route("/analytics/summary", get(summary))
}

async fn summary(State(s): State<AppState>, user: AuthUser) -> Result<Json<Summary>, AppError> {
    if user.role != "staff" {
        return Err(AppError::Forbidden);
    }
    Ok(Json(analytics::summary(&s.db).await?))
}
