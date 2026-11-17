use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
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
        .route("/orders", post(create).get(list))
        .route("/orders/{id}", get(fetch))
        .route("/orders/{id}/deliver", post(deliver))
}
#[derive(Deserialize)]
struct ListParams {
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Serialize)]
struct OrderPage {
    items: Vec<Order>,
    next_cursor: Option<String>,
}

fn encode_cursor(o: &Order) -> String {
    format!("{}_{}", o.created_at.timestamp_micros(), o.id)
}

fn decode_cursor(raw: &str) -> Result<(chrono::DateTime<chrono::Utc>, Uuid), AppError> {
    let bad = || AppError::BadRequest("invalid cursor".into());
    let (micros, id) = raw.split_once('_').ok_or_else(bad)?;
    let micros: i64 = micros.parse().map_err(|_| bad())?;
    let at = DateTime::from_timestamp_micros(micros).ok_or_else(bad)?;
    let id = Uuid::parse_str(id).map_err(|_| bad())?;
    Ok((at, id))
}

async fn list(
    State(s): State<AppState>,
    user: AuthUser,
    Query(p): Query<ListParams>,
) -> Result<Json<OrderPage>, AppError> {
    let limit = p.limit.unwrap_or(20).clamp(1, 100);
    let after = p.cursor.as_deref().map(decode_cursor).transpose()?;
    let who = if user.role == "staff" {
        None
    } else {
        Some(user.id)
    };

    // fetch one extra row to learn whether another page exists
    let mut items = orders::list_orders(&s.db, who, limit + 1, after).await?;
    let next_cursor = if items.len() as i64 > limit {
        items.truncate(limit as usize);
        items.last().map(encode_cursor)
    } else {
        None
    };
    Ok(Json(OrderPage { items, next_cursor }))
}

async fn deliver(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Order>, AppError> {
    if user.role != "staff" {
        return Err(AppError::Forbidden);
    }
    let order = orders::mark_delivered(&s.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(order))
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
