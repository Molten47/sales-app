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

pub mod email;

pub async fn route_events(mut rx: mpsc::Receiver<OutboxEvent>, hub: Hub, db: PgPool) {
    while let Some(e) = rx.recv().await {
        match e.event_type.as_str() {
            "order.created" | "order.delivered" => {
                let customer_id = e.payload["customer_id"]
                    .as_str()
                    .and_then(|s| s.parse::<Uuid>().ok());

                // 1. persist first, so offline users still get it
                match persist(&db, customer_id, &e.event_type, e.id, &e.payload).await {
                    Some(rows) => {
                        // 2. then push live to whoever is connected
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
                            event = %e.event_type,
                            event_id = %e.id,
                            recipients = rows.len(),
                            delivered,
                            "order routed"
                        );
                    }
                    None => {
                        tracing::error!(event_id = %e.id, "notification insert failed, giving up")
                    }
                }

                // dashboards refetch their numbers; the order itself is already committed
                hub.broadcast_staff(
                    &json!({ "type": "analytics.updated", "event_id": e.id }).to_string(),
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
    event_id: Uuid,
    payload: &serde_json::Value,
) -> Option<Vec<Notification>> {
    for attempt in 1..=PERSIST_ATTEMPTS {
        match notifications::insert_for_order(db, event_id, customer_id, kind, payload).await {
            Ok(rows) => return Some(rows),
            Err(e) => {
                tracing::warn!(attempt, "notification insert failed: {e}");
                tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
            }
        }
    }
    None
}
