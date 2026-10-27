use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Notification {
    pub id: Uuid,
    pub user_id: Uuid,
    pub kind: String,
    pub payload: serde_json::Value,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// One row per recipient: every staff user plus the customer.
/// A single statement, so it is atomic, and a customer who is also
/// staff gets one row, not two.
pub async fn insert_for_order(
    pool: &PgPool,
    customer_id: Option<Uuid>,
    kind: &str,
    payload: &serde_json::Value,
) -> Result<Vec<Notification>, sqlx::Error> {
    sqlx::query_as::<_, Notification>(
        r#"
        INSERT INTO notifications (user_id, kind, payload)
        SELECT id, $1, $2 FROM users
        WHERE role = 'staff' OR id = $3
        RETURNING id, user_id, kind, payload, read_at, created_at
        "#,
    )
    .bind(kind)
    .bind(payload)
    .bind(customer_id)
    .fetch_all(pool)
    .await
}

pub async fn list_unread(pool: &PgPool, user_id: Uuid) -> Result<Vec<Notification>, sqlx::Error> {
    sqlx::query_as::<_, Notification>(
        r#"
        SELECT id, user_id, kind, payload, read_at, created_at
        FROM notifications
        WHERE user_id = $1 AND read_at IS NULL
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Idempotent: marking an already-read item keeps its original read_at.
/// Returns false if the id doesn't exist or belongs to someone else.
pub async fn mark_read(pool: &PgPool, id: Uuid, user_id: Uuid) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE notifications SET read_at = COALESCE(read_at, now())
         WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}
