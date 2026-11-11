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

const SECRET: &str = "test-secret";

fn test_app(db: &PgPool) -> Router {
    app(AppState {
        db: db.clone(),
        hub: Hub::default(),
        jwt_secret: Arc::from(SECRET),
        mailer: Mailer::new(EmailProvider::Log).unwrap(),
    })
}

async fn call(
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
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Inserts a user directly and returns (id, valid token).
async fn seed_user(db: &PgPool, email: &str, staff: bool) -> (Uuid, String) {
    let role = if staff { "staff" } else { "customer" };
    let id: Uuid = sqlx::query_scalar(&format!(
        "INSERT INTO users (email, name, role) VALUES ($1, 'Test', '{role}') RETURNING id"
    ))
    .bind(email)
    .fetch_one(db)
    .await
    .unwrap();
    (id, auth::issue_token(SECRET, id, role))
}

/// Replaces the user's OTP with a known code.
async fn set_code(db: &PgPool, id: Uuid, code: &str) {
    otp::store(db, id, &auth::hash_code(SECRET, id, code))
        .await
        .unwrap();
}

async fn count(db: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(db).await.unwrap()
}

async fn create_order(app: &Router, token: &str) -> String {
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

fn verify_body(email: &str, code: &str) -> Option<Value> {
    Some(json!({ "email": email, "code": code }))
}

#[sqlx::test]
async fn otp_unknown_email_answers_202(db: PgPool) {
    let app = test_app(&db);
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/auth/request-otp",
        None,
        Some(json!({ "email": "nobody@example.com" })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(count(&db, "SELECT count(*) FROM otp_codes").await, 0);
}

#[sqlx::test]
async fn otp_login_is_single_use(db: PgPool) {
    let app = test_app(&db);
    let (id, _) = seed_user(&db, "a@example.com", false).await;

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/auth/request-otp",
        None,
        Some(json!({ "email": "a@example.com" })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    set_code(&db, id, "123456").await;
    let uri = "/api/v1/auth/verify-otp";

    let (status, body) =
        call(&app, Method::POST, uri, None, verify_body("a@example.com", "123456")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["role"], "customer");
    assert!(body["token"].as_str().is_some());

    let (status, _) =
        call(&app, Method::POST, uri, None, verify_body("a@example.com", "123456")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn otp_locks_after_five_wrong_attempts(db: PgPool) {
    let app = test_app(&db);
    let (id, _) = seed_user(&db, "a@example.com", false).await;
    set_code(&db, id, "123456").await;
    let uri = "/api/v1/auth/verify-otp";

    for _ in 0..5 {
        let (status, _) =
            call(&app, Method::POST, uri, None, verify_body("a@example.com", "000000")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    // the right code no longer works once locked
    let (status, _) =
        call(&app, Method::POST, uri, None, verify_body("a@example.com", "123456")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn request_otp_is_rate_limited_per_email(db: PgPool) {
    let app = test_app(&db);
    seed_user(&db, "a@example.com", false).await;

    for _ in 0..6 {
        let (status, _) = call(
            &app,
            Method::POST,
            "/api/v1/auth/request-otp",
            None,
            Some(json!({ "email": "a@example.com" })),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }
    assert_eq!(count(&db, "SELECT count(*) FROM otp_requests").await, 5);
}

#[sqlx::test]
async fn public_signup_cannot_create_staff(db: PgPool) {
    let app = test_app(&db);

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/users",
        None,
        Some(json!({ "email": "evil@example.com", "name": "Evil", "role": "staff" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/users",
        None,
        Some(json!({ "email": "ok@example.com", "name": "Ok" })),
    )
    .await;
    assert!(status.is_success());
}

#[sqlx::test]
async fn only_staff_can_create_staff(db: PgPool) {
    let app = test_app(&db);
    let (_, customer) = seed_user(&db, "c@example.com", false).await;
    let (_, staff) = seed_user(&db, "s@example.com", true).await;
    let body = || Some(json!({ "email": "new@example.com", "name": "New" }));

    let (status, _) = call(&app, Method::POST, "/api/v1/staff", Some(&customer), body()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = call(&app, Method::POST, "/api/v1/staff", None, body()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = call(&app, Method::POST, "/api/v1/staff", Some(&staff), body()).await;
    assert!(status.is_success());
}

#[sqlx::test]
async fn orders_and_notifications_need_a_token(db: PgPool) {
    let app = test_app(&db);
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/orders",
        None,
        Some(json!({ "total_minor": 1000 })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = call(&app, Method::GET, "/api/v1/notifications", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn order_rejects_non_positive_total(db: PgPool) {
    let app = test_app(&db);
    let (_, token) = seed_user(&db, "c@example.com", false).await;
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/orders",
        Some(&token),
        Some(json!({ "total_minor": 0 })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn customers_only_see_their_own_orders(db: PgPool) {
    let app = test_app(&db);
    let (_, a) = seed_user(&db, "a@example.com", false).await;
    let (_, b) = seed_user(&db, "b@example.com", false).await;
    let (_, staff) = seed_user(&db, "s@example.com", true).await;

    let id = create_order(&app, &a).await;
    let uri = format!("/api/v1/orders/{id}");

    let (status, _) = call(&app, Method::GET, &uri, Some(&a), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&app, Method::GET, &uri, Some(&staff), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&app, Method::GET, &uri, Some(&b), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn creating_an_order_queues_event_and_email_together(db: PgPool) {
    let app = test_app(&db);
    let (_, token) = seed_user(&db, "c@example.com", false).await;
    create_order(&app, &token).await;

    assert_eq!(
        count(&db, "SELECT count(*) FROM outbox_events WHERE event_type = 'order.created'").await,
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM email_jobs
             WHERE kind = 'order_created' AND to_email = 'c@example.com' AND status = 'pending'"
        )
        .await,
        1
    );
}

#[sqlx::test]
async fn deliver_is_staff_only_and_happens_once(db: PgPool) {
    let app = test_app(&db);
    let (_, customer) = seed_user(&db, "c@example.com", false).await;
    let (_, staff) = seed_user(&db, "s@example.com", true).await;

    let id = create_order(&app, &customer).await;
    let uri = format!("/api/v1/orders/{id}/deliver");

    let (status, _) = call(&app, Method::POST, &uri, Some(&customer), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = call(&app, Method::POST, &uri, Some(&staff), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "delivered");

    let (status, _) = call(&app, Method::POST, &uri, Some(&staff), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    assert_eq!(
        count(&db, "SELECT count(*) FROM email_jobs WHERE kind = 'order_delivered'").await,
        1
    );
}