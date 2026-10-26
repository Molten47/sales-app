use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, Serialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

pub async fn create_user(
    db: &PgPool,
    email: &str,
    name: &str,
    role: &str,
) -> Result<User, sqlx::Error> {
    sqlx::query_as(
        "INSERT INTO users (email, name, role)
         VALUES ($1, $2, $3)
         RETURNING id, email, name, role, created_at",
    )
    .bind(email)
    .bind(name)
    .bind(role)
    .fetch_one(db)
    .await
}
