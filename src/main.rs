use axum::http::{HeaderValue, Method, header};
use sales_app::{AppState, app, config, email, outbox, workers, ws};
use sqlx::postgres::PgPoolOptions;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tower_http::cors::CorsLayer;

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
    let mailer = email::Mailer::new(cfg.email.clone())?;

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(workers::route_events(event_rx, hub.clone(), db.clone()));
    tokio::spawn(outbox::poller::run(db.clone(), event_tx));
    tokio::spawn(workers::email::run(db.clone(), mailer.clone()));
    tokio::spawn(workers::email::daily_digest(db.clone()));

    let origins = cfg
        .cors_origins
        .iter()
        .map(|o| o.parse::<HeaderValue>())
        .collect::<Result<Vec<_>, _>>()?;

    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .max_age(Duration::from_secs(3600));

    let app = app(AppState {
        db,
        hub,
        jwt_secret: Arc::from(cfg.jwt_secret),
        mailer,
    })
    .layer(cors);

    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.port));
    tracing::info!("migrations applied");
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
