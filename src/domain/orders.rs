use crate::{domain::email_jobs, email::Email};
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

    let to: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(customer_id)
        .fetch_one(&mut *tx)
        .await?;

    email_jobs::enqueue(
        &mut *tx,
        &to,
        &Email::OrderCreated {
            order_id: order.id,
            total_minor: order.total_minor,
            currency: order.currency.clone(),
        },
        Some(&format!("order_created:{}", order.id)),
    )
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
/// Marks an order delivered and queues the email atomically.
/// Returns None if the order doesn't exist or was already delivered.
pub async fn mark_delivered(db: &PgPool, id: Uuid) -> Result<Option<Order>, sqlx::Error> {
    let mut tx = db.begin().await?;

    let order: Option<Order> = sqlx::query_as(
        "UPDATE orders SET status = 'delivered'
         WHERE id = $1 AND status <> 'delivered'
         RETURNING id, customer_id, status, total_minor,
                   currency::text AS currency, created_at",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(order) = order else {
        return Ok(None);
    };

    sqlx::query(
        "INSERT INTO outbox_events (aggregate_type, aggregate_id, event_type, payload)
         VALUES ('order', $1, 'order.delivered', $2)",
    )
    .bind(order.id)
    .bind(serde_json::json!({ "order_id": order.id, "customer_id": order.customer_id }))
    .execute(&mut *tx)
    .await?;

    let to: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(order.customer_id)
        .fetch_one(&mut *tx)
        .await?;

    email_jobs::enqueue(
        &mut *tx,
        &to,
        &Email::OrderDelivered { order_id: order.id },
        Some(&format!("order_delivered:{}", order.id)),
    )
    .await?;

    tx.commit().await?;
    Ok(Some(order))
}
