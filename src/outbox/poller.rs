use std::time::Duration;

use sqlx::{PgPool, postgres::PgListener};
use tokio::sync::mpsc;

use super::OutboxEvent;

const BATCH_SIZE: i64 = 50;
const MAX_ATTEMPTS: i32 = 8;
const CHANNEL: &str = "outbox_new";
/// Safety net: even with no notification we re-check at least this often.
const MAX_WAIT: Duration = Duration::from_secs(30);

pub async fn run(db: PgPool, out: mpsc::Sender<OutboxEvent>) {
    // LISTEN before the first drain, so an event inserted in between can't slip through
    let mut listener = connect_listener(&db).await;
    tracing::info!(listening = listener.is_some(), "outbox poller started");

    loop {
        drain(&db, &out).await;

        let wait = next_wait(&db).await;
        match listener.as_mut() {
            Some(l) => match tokio::time::timeout(wait, l.recv()).await {
                Ok(Ok(_)) | Err(_) => {} // notified, or the wait elapsed: loop and drain
                Ok(Err(e)) => {
                    tracing::warn!("listener error: {e}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            },
            None => {
                tokio::time::sleep(wait.min(Duration::from_secs(5))).await;
                listener = connect_listener(&db).await;
            }
        }
    }
}

async fn connect_listener(db: &PgPool) -> Option<PgListener> {
    let mut listener = match PgListener::connect_with(db).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("outbox listener connect failed, polling only: {e}");
            return None;
        }
    };
    if let Err(e) = listener.listen(CHANNEL).await {
        tracing::warn!("outbox LISTEN failed, polling only: {e}");
        return None;
    }
    Some(listener)
}

/// Process batches back to back until nothing is due.
async fn drain(db: &PgPool, out: &mpsc::Sender<OutboxEvent>) {
    loop {
        match process_batch(db, out).await {
            Ok(n) if n > 0 => continue,
            Ok(_) => break,
            Err(e) => {
                tracing::error!("outbox poll failed: {e:#}");
                break;
            }
        }
    }
}

/// Sleep until the next pending event is due (retries use backoff timestamps), capped at MAX_WAIT.
async fn next_wait(db: &PgPool) -> Duration {
    let secs = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT EXTRACT(EPOCH FROM (min(available_at) - now()))::float8
         FROM outbox_events WHERE status = 'pending'",
    )
    .fetch_one(db)
    .await;

    match secs {
        Ok(Some(s)) => Duration::from_secs_f64(s.clamp(0.05, MAX_WAIT.as_secs_f64())),
        Ok(None) => MAX_WAIT,
        Err(e) => {
            tracing::warn!("next_wait query failed: {e}");
            Duration::from_secs(2)
        }
    }
}

async fn process_batch(db: &PgPool, out: &mpsc::Sender<OutboxEvent>) -> Result<usize, sqlx::Error> {
    let mut tx = db.begin().await?;

    let events: Vec<OutboxEvent> = sqlx::query_as(
        "SELECT id, aggregate_type, aggregate_id, event_type, payload, attempts
         FROM outbox_events
         WHERE status = 'pending' AND available_at <= now()
         ORDER BY available_at, created_at
         LIMIT $1
         FOR UPDATE SKIP LOCKED",
    )
    .bind(BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await?;

    let count = events.len();

    for event in events {
        let id = event.id;
        let attempts = event.attempts;

        match out.send(event).await {
            Ok(()) => {
                sqlx::query(
                    "UPDATE outbox_events
                     SET status = 'published', published_at = now()
                     WHERE id = $1",
                )
                .bind(id)
                .execute(&mut *tx)
                .await?;
            }
            Err(e) => {
                // exponential backoff: 2s, 4s, 8s ... capped at 5 min
                let delay_secs = (2f64.powi(attempts + 1)).min(300.0);
                tracing::warn!(%id, "publish failed (attempt {}): {e}", attempts + 1);
                sqlx::query(
                    "UPDATE outbox_events
                     SET attempts = attempts + 1,
                         last_error = $2,
                         available_at = now() + make_interval(secs => $3),
                         status = CASE WHEN attempts + 1 >= $4 THEN 'failed' ELSE 'pending' END
                     WHERE id = $1",
                )
                .bind(id)
                .bind(e.to_string())
                .bind(delay_secs)
                .bind(MAX_ATTEMPTS)
                .execute(&mut *tx)
                .await?;
            }
        }
    }

    tx.commit().await?;
    Ok(count)
}
