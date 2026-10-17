use tokio::sync::mpsc;

use crate::outbox::OutboxEvent;

pub async fn log_consumer(mut rx: mpsc::Receiver<OutboxEvent>) {
    while let Some(e) = rx.recv().await {
        tracing::info! (
            id = %&e.id,
            aggregate = %e.aggregate_type,
            aggregate_id = %e.aggregate_id,
            event = %e.event_type,
            attempts = e.attempts,
            payload = %e.payload,
            "event received"
        );
    }
}
