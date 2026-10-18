use serde_json::json;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{outbox::OutboxEvent, ws::hub::Hub};

pub async fn route_events(mut rx: mpsc::Receiver<OutboxEvent>, hub: Hub) {
    while let Some(e) = rx.recv().await {
        match e.event_type.as_str() {
            "order.created" => {
                let customer_id = e.payload["customer_id"]
                    .as_str()
                    .and_then(|s| s.parse::<Uuid>().ok());

                let msg = json!({
                    "type": e.event_type,
                    "event_id": e.id,
                    "data": e.payload,
                })
                .to_string();

                let mut delivered = 0;
                if let Some(c) = customer_id {
                    delivered += hub.send_to_user(c, &msg);
                }
                delivered += hub.broadcast_staff(&msg);

                tracing::info!(event_id = %e.id, delivered, "order.created routed over ws");
            }
            other => tracing::debug!(event = other, "no route for event"),
        }
    }
}
