mod common;

use axum::http::{Method, StatusCode};
use common::*;
use sales_app::domain::orders;
use sqlx::PgPool;

const URI: &str = "/api/v1/analytics/summary";

#[sqlx::test]
async fn summary_is_staff_only(db: PgPool) {
    let app = test_app(&db);
    let (_, customer) = seed_user(&db, "c@example.com", false).await;
    let (_, staff) = seed_user(&db, "s@example.com", true).await;

    let (status, _) = call(&app, Method::GET, URI, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&app, Method::GET, URI, Some(&customer), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&app, Method::GET, URI, Some(&staff), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test]
async fn summary_groups_by_currency_and_skips_cancelled(db: PgPool) {
    let app = test_app(&db);
    let (customer, _) = seed_user(&db, "c@example.com", false).await;
    let (_, staff) = seed_user(&db, "s@example.com", true).await;

    for (amount, currency) in [(100_000, "NGN"), (200_000, "NGN"), (5_000, "USD")] {
        orders::create_order(&db, customer, amount, currency)
            .await
            .unwrap();
    }
    let cancelled = orders::create_order(&db, customer, 900_000, "NGN")
        .await
        .unwrap();
    sqlx::query("UPDATE orders SET status = 'cancelled' WHERE id = $1")
        .bind(cancelled.id)
        .execute(&db)
        .await
        .unwrap();

    let (_, body) = call(&app, Method::GET, URI, Some(&staff), None).await;

    let today = body["today"].as_array().unwrap();
    assert_eq!(today.len(), 2);
    let ngn = today.iter().find(|t| t["currency"] == "NGN").unwrap();
    assert_eq!(ngn["orders"], 2);
    assert_eq!(ngn["revenue_minor"], 300_000);

    assert_eq!(body["last_7_days"].as_array().unwrap().len(), 2);

    let statuses = body["by_status"].as_array().unwrap();
    let pending = statuses.iter().find(|s| s["status"] == "pending").unwrap();
    assert_eq!(pending["orders"], 3);
}
