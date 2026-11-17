mod common;

use std::{net::SocketAddr, time::Duration};

use axum::{
    Router,
    http::{Method, StatusCode},
};
use common::*;
use futures_util::{Stream, StreamExt};
use sales_app::{AppState, app, outbox, workers};
use serde_json::Value;
use sqlx::PgPool;
use tokio::task::AbortHandle;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error as WsError, Message},
};
use uuid::Uuid;

/// Aborts the background tasks when the test ends.
struct Pipeline(Vec<AbortHandle>);

impl Drop for Pipeline {
    fn drop(&mut self) {
        for h in &self.0 {
            h.abort();
        }
    }
}

fn start_pipeline(state: &AppState) -> Pipeline {
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let router = tokio::spawn(workers::route_events(
        rx,
        state.hub.clone(),
        state.db.clone(),
    ));
    let poller = tokio::spawn(outbox::poller::run(state.db.clone(), tx));
    Pipeline(vec![router.abort_handle(), poller.abort_handle()])
}

async fn wait_for_count(db: &PgPool, sql: &str, want: i64) {
    for _ in 0..50 {
        if count(db, sql).await == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "timed out waiting for `{sql}` to reach {want}, got {}",
        count(db, sql).await
    );
}

async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

fn ws_url(addr: SocketAddr, token: &str) -> String {
    format!("ws://{addr}/ws/connect?token={token}")
}

async fn next_json<S>(ws: &mut S, wait: Duration) -> Option<Value>
where
    S: Stream<Item = Result<Message, WsError>> + Unpin,
{
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => return serde_json::from_str(t.as_str()).ok(),
            Ok(Some(Ok(_))) => {} // ping, pong or binary: not an application message
            _ => return None,     // timed out, closed or errored
        }
    }
}

#[sqlx::test]
async fn order_notifies_customer_and_all_staff_and_can_be_read(db: PgPool) {
    let state = test_state(&db);
    let _pipeline = start_pipeline(&state);
    let app = app(state);

    let (customer_id, customer) = seed_user(&db, "c@example.com", false).await;
    let (_, other) = seed_user(&db, "o@example.com", false).await;
    seed_user(&db, "s1@example.com", true).await;
    seed_user(&db, "s2@example.com", true).await;

    create_order(&app, &customer).await;
    // customer + two staff, nobody connected: persisted for later
    wait_for_count(&db, "SELECT count(*) FROM notifications", 3).await;

    let (_, list) = call(
        &app,
        Method::GET,
        "/api/v1/notifications",
        Some(&customer),
        None,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (_, list) = call(
        &app,
        Method::GET,
        "/api/v1/notifications",
        Some(&other),
        None,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 0);

    let nid: Uuid = sqlx::query_scalar("SELECT id FROM notifications WHERE user_id = $1")
        .bind(customer_id)
        .fetch_one(&db)
        .await
        .unwrap();
    let uri = format!("/api/v1/notifications/{nid}/read");

    // someone else's notification: 404
    let (status, _) = call(&app, Method::POST, &uri, Some(&other), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = call(&app, Method::POST, &uri, Some(&customer), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, Method::POST, &uri, Some(&customer), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT); // idempotent

    let (_, list) = call(
        &app,
        Method::GET,
        "/api/v1/notifications",
        Some(&customer),
        None,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 0);
}

#[sqlx::test]
async fn delivering_an_order_notifies_the_customer(db: PgPool) {
    let state = test_state(&db);
    let _pipeline = start_pipeline(&state);
    let app = app(state);

    let (customer_id, customer) = seed_user(&db, "c@example.com", false).await;
    let (_, staff) = seed_user(&db, "s@example.com", true).await;

    let id = create_order(&app, &customer).await;
    wait_for_count(&db, "SELECT count(*) FROM notifications", 2).await;

    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/v1/orders/{id}/deliver"),
        Some(&staff),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // one for "created", one for "delivered"
    wait_for_count(
        &db,
        &format!("SELECT count(*) FROM notifications WHERE user_id = '{customer_id}'"),
        2,
    )
    .await;

    assert_eq!(
        count(
            &db,
            &format!(
                "SELECT count(*) FROM notifications
                 WHERE user_id = '{customer_id}' AND kind = 'order.delivered'"
            )
        )
        .await,
        1
    );
}

#[sqlx::test]
async fn websocket_pushes_live_to_the_right_people_only(db: PgPool) {
    let state = test_state(&db);
    let _pipeline = start_pipeline(&state);
    let app = app(state);

    let (_, customer) = seed_user(&db, "c@example.com", false).await;
    let (_, staff) = seed_user(&db, "s@example.com", true).await;
    let (_, bystander) = seed_user(&db, "o@example.com", false).await;

    let addr = serve(app.clone()).await;
    let (mut customer_ws, _) = connect_async(ws_url(addr, &customer)).await.unwrap();
    let (mut staff_ws, _) = connect_async(ws_url(addr, &staff)).await.unwrap();
    let (mut bystander_ws, _) = connect_async(ws_url(addr, &bystander)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await; // let the hub register them

    let id = create_order(&app, &customer).await;

    for ws in [&mut customer_ws, &mut staff_ws] {
        let msg = next_json(ws, Duration::from_secs(5))
            .await
            .expect("no live message");
        assert_eq!(msg["type"], "order.created");
        assert_eq!(msg["data"]["order_id"], id.as_str());
        assert!(msg["notification_id"].is_string());
    }

    // staff also get a nudge to refresh their dashboard numbers
    let msg = next_json(&mut staff_ws, Duration::from_secs(5))
        .await
        .expect("no analytics message");
    assert_eq!(msg["type"], "analytics.updated");

    // a customer who isn't involved hears nothing
    assert!(
        next_json(&mut bystander_ws, Duration::from_millis(700))
            .await
            .is_none()
    );
}

#[sqlx::test]
async fn websocket_rejects_a_bad_token(db: PgPool) {
    let addr = serve(test_app(&db)).await;
    match connect_async(ws_url(addr, "garbage")).await {
        Err(WsError::Http(resp)) => assert_eq!(resp.status(), 401),
        other => panic!("expected a 401 handshake error, got {other:?}"),
    }
}
