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
    domain::orders::{self, Order},
};

#[derive(Deserialize)]
pub struct CreateOrder {
    customer_id: Uuid,
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
    Json(req): Json<CreateOrder>,
) -> Result<(StatusCode, Json<Order>), AppError> {
    if req.total_minor <= 0 {
        return Err(AppError::BadRequest("total minor cannot be zero".into()));
    }
    let currency = req.currency.unwrap_or_else(|| "NGN".into());
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(AppError::BadRequest(
            "currency must be a 3- ltter code like NGN".into(),
        ));
    }

    let order = orders::create_order(&s.db, req.customer_id, req.total_minor, &currency).await?;
    Ok((StatusCode::CREATED, Json(order)))
}

async fn fetch(State(s): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Order>, AppError> {
    orders::get_order(&s.db, id)
        .await?
        .map(Json)
        .ok_or(AppError::NotFound)
}
