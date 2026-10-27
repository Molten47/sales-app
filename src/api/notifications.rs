use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::AppState;
use crate::api::error::AppError;
use crate::domain::notifications::{self, Notification};

// use the same AppState / AppError / Result alias as src/api/users.rs

#[derive(Deserialize)]
pub struct UserQuery {
    pub user_id: Uuid,
}

pub async fn list_unread(
    State(state): State<AppState>,
    Query(q): Query<UserQuery>,
) -> Result<Json<Vec<Notification>>, AppError> {
    Ok(Json(
        notifications::list_unread(&state.db, q.user_id).await?,
    ))
}

pub async fn mark_read(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<UserQuery>,
) -> Result<axum::http::StatusCode, AppError> {
    if notifications::mark_read(&state.db, id, q.user_id).await? {
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound) // add this variant to error.rs if it doesn't exist -> 404
    }
}
