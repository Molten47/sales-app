use sqlx::PgPool;
use std::time::Duration;

use super::OutboxEvent;
use tokio::sync::mpsc;

const BATCH_SIZE: i64 = 50;
const MAX_ATTEMPS: i32 = 8;
const MIN_IDLE: Duration = Duration::from_millis(200);
const MAX_IDLE: Duration = Duration::from_secs(5);

pub async fn run(db: PgPool, out: mpsc::Sender<OutboxEvent>) {
    tracing::info!("outbox poller started");
    let mut idle = MAX_IDLE;

    loop {
        match process_batch(&db, &out).await {
            Ok(n) if n > 0 => {
                idle = MIN_IDLE;
                continue;
            }
            Ok(_) => {}
            Err(e) => tracing::error!("outbox poll failed {e:#}"),
        }
        tokio::time::sleep(idle).await;
        idle = (idle * 2).min(MAX_IDLE);
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
                let delay_secs = (2f64.powi(attempts + 1)).min(300.0);
                tracing::warn!(%id, "publish failed attemps{}): {e}", attempts + 1);

                sqlx::query(
                    "UPDATE outbox_events
                    SET attemps = attempte = 1,
                    last error = $2,
                    available_at = now() + make_interval(secs => $3),
                    status = CASE WHEN attemps + 1 >= THEN 'failed' ELSE 'pending' END
                    WHERE id =$1",
                )
                .bind(id)
                .bind(e.to_string())
                .bind(delay_secs)
                .bind(MAX_ATTEMPS)
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(count)
}
