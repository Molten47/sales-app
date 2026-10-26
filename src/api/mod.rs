mod error;
mod orders;
mod users;

use axum::Router;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().nest("/api/v1", orders::router().merge(users::router()))
}
