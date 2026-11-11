use sqlx::PgPool;
use uuid::Uuid;

pub const MAX_ATTEMPTS: i32 = 5;
pub const MAX_REQUESTS_PER_HOUR: i64 = 5;
const TTL_SECS: f64 = 300.0;

pub async fn find_user_by_email(
    pool: &PgPool,
    email: &str,
) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as::<_, (Uuid, String)>("SELECT id, role::text FROM users WHERE email = $1")
        .bind(email)
        .fetch_optional(pool)
        .await
}

/// Replaces any existing code for this user and resets the attempt counter.
pub async fn store(pool: &PgPool, user_id: Uuid, code_hash: &[u8]) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO otp_codes (user_id, code_hash, expires_at)
         VALUES ($1, $2, now() + make_interval(secs => $3))
         ON CONFLICT (user_id) DO UPDATE
         SET code_hash = EXCLUDED.code_hash,
             attempts = 0,
             expires_at = EXCLUDED.expires_at,
             created_at = now()",
    )
    .bind(user_id)
    .bind(code_hash)
    .bind(TTL_SECS)
    .execute(pool)
    .await?;
    Ok(())
}

/// Counts the attempt *before* the code is compared, in one atomic statement,
/// so parallel guesses can't squeeze extra tries past the limit.
/// Returns None if there is no live code (missing, expired or locked out).
pub async fn begin_attempt(pool: &PgPool, user_id: Uuid) -> Result<Option<Vec<u8>>, sqlx::Error> {
    sqlx::query_scalar::<_, Vec<u8>>(
        "UPDATE otp_codes SET attempts = attempts + 1
         WHERE user_id = $1 AND expires_at > now() AND attempts < $2
         RETURNING code_hash",
    )
    .bind(user_id)
    .bind(MAX_ATTEMPTS)
    .fetch_optional(pool)
    .await
}

// False means someone's already used the code.
pub async fn consume(pool: &PgPool, user_id: Uuid) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM otp_codes WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

// Record new request and return false if the email is over its hourly limit.
pub async fn allow_request(pool: &PgPool, email: &str) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM otp_requests
         WHERE email = $1 AND created_at > now() - interval '1 hour'",
    )
    .bind(email)
    .fetch_one(pool)
    .await?;

    if count >= MAX_REQUESTS_PER_HOUR {
        return Ok(false);
    }

    sqlx::query("INSERT INTO otp_requests (email) VALUES ($1)")
        .bind(email)
        .execute(pool)
        .await?;
    Ok(true)
}
