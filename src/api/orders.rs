use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use uuid::Uuid;

use super::error::AppError;
use crate::{
    AppState,
    auth::AuthUser,
    domain::orders::{self, Order},
};

#[derive(Deserialize)]
pub struct CreateOrder {
    // no customer_id: the customer is whoever the token says they are
    total_minor: i64,
    currency: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/orders", post(create))
        .route("/orders/{id}", get(fetch))
}

async fn create(
    State(s): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateOrder>,
) -> Result<(StatusCode, Json<Order>), AppError> {
    if req.total_minor <= 0 {
        return Err(AppError::BadRequest(
            "total_minor must be greater than zero".into(),
        ));
    }
    let currency = req.currency.unwrap_or_else(|| "NGN".into());
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(AppError::BadRequest(
            "currency must be a 3-letter code like NGN".into(),
        ));
    }

    let order = orders::create_order(&s.db, user.id, req.total_minor, &currency).await?;
    Ok((StatusCode::CREATED, Json(order)))
}

async fn fetch(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Order>, AppError> {
    let order = orders::get_order(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;

    // Someone else's order answers 404, not 403, so ids can't be probed for existence.
    if order.customer_id != user.id && user.role != "staff" {
        return Err(AppError::NotFound);
    }
    Ok(Json(order))
}
