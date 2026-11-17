#![allow(dead_code)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use sales_app::{
    AppState, app, auth, config::EmailProvider, domain::otp, email::Mailer, ws::hub::Hub,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

pub const SECRET: &str = "test-secret";

pub fn test_state(db: &PgPool) -> AppState {
    AppState {
        db: db.clone(),
        hub: Hub::default(),
        jwt_secret: Arc::from(SECRET),
        mailer: Mailer::new(EmailProvider::Log).unwrap(),
    }
}

pub fn test_app(db: &PgPool) -> Router {
    app(test_state(db))
}

pub async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let body = match body {
        Some(b) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Inserts a user directly and returns (id, valid token).
pub async fn seed_user(db: &PgPool, email: &str, staff: bool) -> (Uuid, String) {
    let role = if staff { "staff" } else { "customer" };
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, name, role) VALUES ($1, 'Test', $2) RETURNING id",
    )
    .bind(email)
    .bind(role)
    .fetch_one(db)
    .await
    .unwrap();
    (id, auth::issue_token(SECRET, id, role))
}

/// Replaces the user's OTP with a known code.
pub async fn set_code(db: &PgPool, id: Uuid, code: &str) {
    otp::store(db, id, &auth::hash_code(SECRET, id, code))
        .await
        .unwrap();
}

/// Runs a `SELECT count(*)`-style query. Test-only: the SQL is written in the
/// tests themselves, never taken from user input, so it is safe to assert as such.
pub async fn count(db: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(db)
        .await
        .unwrap()
}

/// Creates an order as the given customer and returns its id.
pub async fn create_order(app: &Router, token: &str) -> String {
    let (status, body) = call(
        app,
        Method::POST,
        "/api/v1/orders",
        Some(token),
        Some(json!({ "total_minor": 250000 })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    body["id"].as_str().unwrap().to_string()
}

pub fn verify_body(email: &str, code: &str) -> Option<Value> {
    Some(json!({ "email": email, "code": code }))
}
