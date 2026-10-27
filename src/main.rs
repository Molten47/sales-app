use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{net::SocketAddr, time::Duration};

mod api;
mod config;
mod domain;
mod outbox;
mod workers;
mod ws;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub hub: ws::hub::Hub,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let cfg = config::Config::from_env()?;

    let db = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&cfg.database_url)
        .await?;

    sqlx::migrate!().run(&db).await?;

    let hub = ws::hub::Hub::default();

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(workers::route_events(event_rx, hub.clone(), db.clone()));
    tokio::spawn(outbox::poller::run(db.clone(), event_tx));

    let app = Router::new()
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
        .merge(api::router())
        .with_state(AppState { db, hub });

    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.port));
    tracing::info!("migrations applied");
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ready(State(s): State<AppState>) -> StatusCode {
    match sqlx::query("SELECT 1").execute(&s.db).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}
