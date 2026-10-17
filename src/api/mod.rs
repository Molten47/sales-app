mod error;
mod orders;

use crate::AppState;
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new().nest("/api/v1", orders::router())
}
