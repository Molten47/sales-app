use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use uuid::Uuid;

use crate::{
    AppState,
    api::error::AppError,
    auth::AuthUser,
    domain::notifications::{self, Notification},
};

pub async fn list_unread(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<Notification>>, AppError> {
    Ok(Json(notifications::list_unread(&state.db, user.id).await?))
}

pub async fn mark_read(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if notifications::mark_read(&state.db, id, user.id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound)
    }
}
