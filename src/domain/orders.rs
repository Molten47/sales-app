use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, Serialize, FromRow)]
pub struct Order {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub status: String,
    pub total_minor: i64,
    pub currency: String,
    pub created_at: DateTime<Utc>,
}

pub async fn create_order(
    db: &PgPool,
    customer_id: Uuid,
    total_minor: i64,
    currency: &str,
) -> Result<Order, sqlx::Error> {
    let mut tx = db.begin().await?;

    // 1. business state
    let order: Order = sqlx::query_as(
        "INSERT INTO orders (customer_id, total_minor, currency)
         VALUES ($1, $2, $3)
         RETURNING id, customer_id, status, total_minor,
                   currency::text AS currency, created_at",
    )
    .bind(customer_id)
    .bind(total_minor)
    .bind(currency)
    .fetch_one(&mut *tx)
    .await?;

    // 2. outbox event, same transaction
    let payload = serde_json::json!({
        "order_id": order.id,
        "customer_id": order.customer_id,
        "total_minor": order.total_minor,
        "currency": order.currency,
    });
    sqlx::query(
        "INSERT INTO outbox_events (aggregate_type, aggregate_id, event_type, payload)
         VALUES ('order', $1, 'order.created', $2)",
    )
    .bind(order.id)
    .bind(payload)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(order)
}

pub async fn get_order(db: &PgPool, id: Uuid) -> Result<Option<Order>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, customer_id, status, total_minor,
                currency::text AS currency, created_at
         FROM orders WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await
}
