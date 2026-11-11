use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::email::Email;

pub const MAX_ATTEMPTS: i32 = 6;

pub struct Job {
    pub id: Uuid,
    pub to_email: String,
    pub payload: serde_json::Value,
    pub attempts: i32,
}

/// A repeated dedupe_key is silently ignored.
pub async fn enqueue<'e>(
    db: impl PgExecutor<'e>,
    to: &str,
    email: &Email,
    dedupe_key: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO email_jobs (kind, to_email, payload, priority, dedupe_key)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (dedupe_key) DO NOTHING",
    )
    .bind(email.kind())
    .bind(to)
    .bind(sqlx::types::Json(email))
    .bind(email.priority())
    .bind(dedupe_key)
    .execute(db)
    .await?;
    Ok(())
}

/// Claims one due job. The 2-minute lease means a crashed worker's job
/// becomes due again on its own, and SKIP LOCKED keeps instances apart.
pub async fn claim(pool: &PgPool) -> Result<Option<Job>, sqlx::Error> {
    let row = sqlx::query_as::<_, (Uuid, String, serde_json::Value, i32)>(
        "UPDATE email_jobs
         SET attempts = attempts + 1, next_attempt_at = now() + interval '2 minutes'
         WHERE id = (
             SELECT id FROM email_jobs
             WHERE status = 'pending' AND next_attempt_at <= now()
             ORDER BY priority, next_attempt_at
             LIMIT 1 FOR UPDATE SKIP LOCKED)
         RETURNING id, to_email, payload, attempts",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id, to_email, payload, attempts)| Job {
        id,
        to_email,
        payload,
        attempts,
    }))
}

pub async fn mark_sent(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE email_jobs SET status = 'sent', sent_at = now(), last_error = NULL WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Retries with exponential backoff (30s, 60s, 120s...), then marks dead.
pub async fn mark_failed(
    pool: &PgPool,
    id: Uuid,
    attempts: i32,
    err: &str,
) -> Result<(), sqlx::Error> {
    let backoff = 30.0 * 2f64.powi((attempts - 1).max(0));
    sqlx::query(
        "UPDATE email_jobs
         SET status = CASE WHEN $2 >= $3 THEN 'dead' ELSE 'pending' END,
             last_error = $4,
             next_attempt_at = now() + make_interval(secs => $5)
         WHERE id = $1",
    )
    .bind(id)
    .bind(attempts)
    .bind(MAX_ATTEMPTS)
    .bind(err)
    .bind(backoff)
    .execute(pool)
    .await?;
    Ok(())
}
pub async fn mark_dead(pool: &PgPool, id: Uuid, err: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE email_jobs SET status = 'dead', last_error = $2 WHERE id = $1")
        .bind(id)
        .bind(err)
        .execute(pool)
        .await?;
    Ok(())
}
