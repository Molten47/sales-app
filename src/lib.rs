use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use sqlx::PgPool;
use std::sync::Arc;

pub mod api;
pub mod auth;
pub mod config;
pub mod domain;
pub mod email;
pub mod outbox;
pub mod workers;
pub mod ws;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub hub: ws::hub::Hub,
    pub jwt_secret: Arc<str>,
    pub mailer: email::Mailer,
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/health/ready", get(ready))
        .route("/ws/connect", get(ws::handler::ws_connect))
        .route(
            "/api/v1/notifications",
            get(api::notifications::list_unread),
        )
        .route(
            "/api/v1/notifications/{id}/read",
            post(api::notifications::mark_read),
        )
        .route("/api/v1/auth/request-otp", post(api::auth::request_otp))
        .route("/api/v1/auth/verify-otp", post(api::auth::verify_otp))
        .merge(api::router())
        .with_state(state)
}

async fn ready(State(s): State<AppState>) -> StatusCode {
    match sqlx::query("SELECT 1").execute(&s.db).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}
