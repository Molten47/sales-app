use std::time::Duration;

use serde_json::json;
use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    domain::notifications::{self, Notification},
    outbox::OutboxEvent,
    ws::hub::Hub,
};

const PERSIST_ATTEMPTS: u32 = 4;

pub async fn route_events(mut rx: mpsc::Receiver<OutboxEvent>, hub: Hub, db: PgPool) {
    while let Some(e) = rx.recv().await {
        match e.event_type.as_str() {
            "order.created" => {
                let customer_id = e.payload["customer_id"]
                    .as_str()
                    .and_then(|s| s.parse::<Uuid>().ok());

                // 1. persist first, so offline users still get it
                let Some(rows) = persist(&db, customer_id, &e.event_type, &e.payload).await else {
                    tracing::error!(event_id = %e.id, "notification insert failed, giving up");
                    continue;
                };

                // 2. then push live to whoever is connected
                // (staff are in the hub's per-user map too, so no broadcast_staff needed)
                let mut delivered = 0;
                for n in &rows {
                    let msg = json!({
                        "type": e.event_type,
                        "event_id": e.id,
                        "notification_id": n.id,
                        "data": e.payload,
                    })
                    .to_string();
                    delivered += hub.send_to_user(n.user_id, &msg);
                }

                tracing::info!(
                    event_id = %e.id,
                    recipients = rows.len(),
                    delivered,
                    "order routed"
                );
            }
            other => tracing::debug!(event = other, "no route for event"),
        }
    }
}

/// The poller marks an event `published` as soon as it is handed to this worker,
/// so a transient DB error here would otherwise lose the notification. Retry with backoff.
async fn persist(
    db: &PgPool,
    customer_id: Option<Uuid>,
    kind: &str,
    payload: &serde_json::Value,
) -> Option<Vec<Notification>> {
    for attempt in 1..=PERSIST_ATTEMPTS {
        match notifications::insert_for_order(db, customer_id, kind, payload).await {
            Ok(rows) => return Some(rows),
            Err(e) => {
                tracing::warn!(attempt, "notification insert failed: {e}");
                tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
            }
        }
    }
    None
}
