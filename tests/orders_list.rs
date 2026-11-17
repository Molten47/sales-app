mod common;

use axum::http::{Method, StatusCode};
use common::*;
use sqlx::PgPool;

#[sqlx::test]
async fn customers_see_own_orders_and_staff_see_all(db: PgPool) {
    let app = test_app(&db);
    let (_, alice) = seed_user(&db, "alice@example.com", false).await;
    let (_, bob) = seed_user(&db, "bob@example.com", false).await;
    let (_, staff) = seed_user(&db, "staff@example.com", true).await;

    create_order(&app, &alice).await;
    create_order(&app, &alice).await;
    create_order(&app, &bob).await;

    let (st, body) = call(&app, Method::GET, "/api/v1/orders", Some(&alice), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 2);

    let (_, body) = call(&app, Method::GET, "/api/v1/orders", Some(&staff), None).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 3);

    let (st, _) = call(&app, Method::GET, "/api/v1/orders", None, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn paging_walks_newest_first_without_overlap(db: PgPool) {
    let app = test_app(&db);
    let (_, alice) = seed_user(&db, "alice@example.com", false).await;

    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(create_order(&app, &alice).await);
    }

    let (_, p1) = call(
        &app,
        Method::GET,
        "/api/v1/orders?limit=2",
        Some(&alice),
        None,
    )
    .await;
    let got: Vec<&str> = p1["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    assert_eq!(got, vec![ids[2].as_str(), ids[1].as_str()]);
    let cursor = p1["next_cursor"].as_str().expect("a second page exists");

    let uri = format!("/api/v1/orders?limit=2&cursor={cursor}");
    let (_, p2) = call(&app, Method::GET, &uri, Some(&alice), None).await;
    let got: Vec<&str> = p2["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    assert_eq!(got, vec![ids[0].as_str()]);
    assert!(p2["next_cursor"].is_null());
}

#[sqlx::test]
async fn garbage_cursor_is_a_400(db: PgPool) {
    let app = test_app(&db);
    let (_, alice) = seed_user(&db, "alice@example.com", false).await;
    let (st, _) = call(
        &app,
        Method::GET,
        "/api/v1/orders?cursor=nonsense",
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}
