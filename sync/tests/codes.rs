use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use common::AsExpected;
use iceblink_sync::models;
use serde_json::json;
use sqlx::SqlitePool;
use tower::ServiceExt;

pub mod common;

#[sqlx::test(fixtures("users", "codes"))]
async fn list_own_codes(db: SqlitePool) {
    let app = common::testing_setup(&db).await;
    let (a1, a2) = common::get_access_tokens(&db).await;

    let u1 = common::list_codes_content(&app, a1.as_str()).await;
    assert_eq!(u1.len(), 2);
    for code in u1.iter() {
        assert!(code.is_as_expected())
    }

    let u2 = common::list_codes_content(&app, a2.as_str()).await;
    assert_eq!(u2.len(), 1);
    for code in u2.iter() {
        assert!(code.is_as_expected())
    }
}

#[sqlx::test(fixtures("users", "codes"))]
async fn add_codes(db: SqlitePool) {
    let app = common::testing_setup(&db).await;
    let (a1, a2) = common::get_access_tokens(&db).await;

    // Add code
    let added = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/v1/code")
                .header("Authorization", format!("Bearer {a1}"))
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "content": "garbage",
                        "display_name": "Permafrost",
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(added.status(), StatusCode::OK);
    let added_res: models::codes::Code =
        serde_json::from_value(common::convert_response(added).await).unwrap();
    assert_eq!(added_res.content, "garbage");
    assert_eq!(added_res.display_name, "Permafrost");
    assert_eq!(added_res.icon_url, None);
    assert_eq!(added_res.website_url, None);
    assert_eq!(added_res.owner_id, common::USER1_ID);
    assert_eq!(added_res.id.len(), 16);

    // Check that it was added to the list
    let user1_after = common::list_codes(&app, a1.as_str()).await;

    assert_eq!(user1_after.status(), StatusCode::OK);
    assert_eq!(
        common::convert_response(user1_after).await,
        json!([
            {
                "content": common::USER1_CODE1_CONTENT,
                "display_name": "Google",
                "icon_url": null,
                "id": common::USER1_CODE1_ID,
                "owner_id": common::USER1_ID,
                "website_url": "google.com"
            },
            {
                "content": common::USER1_CODE2_CONTENT,
                "display_name": "google.com",
                "icon_url": null,
                "id": common::USER1_CODE2_ID,
                "owner_id": common::USER1_ID,
                "website_url": "google.com"
            },
            {
                "content": "garbage",
                "display_name": "Permafrost",
                "icon_url": null,
                "id": added_res.id,
                "owner_id": common::USER1_ID,
                "website_url": null
            },
        ])
    );

    // User 2 should not affected by the operation
    let u2 = common::list_codes_content(&app, a2.as_str()).await;
    assert_eq!(u2.len(), 1);
    for code in u2.iter() {
        assert!(code.is_as_expected())
    }
}
