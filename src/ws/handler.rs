use std::time::Duration;

use axum::{
    body::Bytes,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use uuid::Uuid;

use super::hub::{Hub, Role};
use crate::AppState;

const PING_EVERY: Duration = Duration::from_secs(25);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

// DEV SCAFFOLDING: anyone can claim any user_id or role.
// Replaced by a verified JWT when the OTP login lands.
#[derive(Deserialize)]
pub struct ConnectParams {
    user_id: Uuid,
    role: Option<String>,
}

pub async fn ws_connect(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(p): Query<ConnectParams>,
) -> Response {
    let role = match p.role.as_deref() {
        Some("staff") => Role::Staff,
        _ => Role::Customer,
    };
    ws.on_upgrade(move |socket| handle_socket(socket, state.hub, p.user_id, role))
}

async fn handle_socket(socket: WebSocket, hub: Hub, user_id: Uuid, role: Role) {
    let (conn_id, mut rx) = hub.register(user_id, role);
    tracing::info!(%user_id, %conn_id, ?role, "ws connected");

    let (mut sink, mut stream) = socket.split();

    // writer: hub messages out, plus heartbeat pings
    let mut writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_EVERY);
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(text) => {
                        if sink.send(Message::Text(text.into())).await.is_err() { break; }
                    }
                    None => break, // hub dropped us
                },
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Bytes::new())).await.is_err() { break; }
                }
            }
        }
    });

    // reader: any frame (including pongs) resets the idle timer
    let mut reader = tokio::spawn(async move {
        loop {
            match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
                Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
                Ok(Some(Ok(_))) => {} // chat messages will be handled here later
            }
        }
    });

    tokio::select! {
        _ = &mut writer => reader.abort(),
        _ = &mut reader => writer.abort(),
    }

    hub.unregister(user_id, conn_id);
    tracing::info!(%user_id, %conn_id, "ws disconnected");
}
