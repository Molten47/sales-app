use axum::{Router, extract::State, http::StatusCode, routing::get};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{net::SocketAddr, time::Duration};

mod api;
mod config;
mod domain;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
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
    tracing::info!("migration appplied");

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/health/ready", get(ready))
        .merge(api::router())
        .with_state(AppState { db });

    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.port));
    tracing::info!("listening on addr");
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
