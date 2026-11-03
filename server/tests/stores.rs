mod common;

use axum::http::Method;
use common::TestApp;
use serde_json::{Value, json};
use sqlx::PgPool;

fn store_body(name: &str) -> Value {
    json!({
        "name": name,
        "description": "Fresh goods",
        "pickup_address": "12 Unique Street, Wuse 2",
        "pickup_state": "FCT",
        "delivery_courier": true,
        "delivery_self": false,
        "pickup_area_council": "amac"
    })
}

async fn create(app: &TestApp, token: &str, name: &str) -> common::TestResponse {
    app.request(Method::POST, "/stores", Some(store_body(name)), Some(token))
        .await
}

#[sqlx::test]
async fn seller_creates_store(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.make_seller("08011111111").await;
    let res = create(&app, &token, "Mama Put").await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["status"], "pending");
    let slug = res.json["slug"].as_str().unwrap();
    let got = app
        .request(Method::GET, &format!("/stores/{slug}"), None, Some(&token))
        .await;
    assert_eq!(got.status, 200);
    assert_eq!(got.json["name"], "Mama Put");
    assert!(got.json.get("pickup_address").is_none());
    // Pending stores are hidden from the public.
    let public = app
        .request(Method::GET, &format!("/stores/{slug}"), None, None)
        .await;
    assert_eq!(public.status, 404);
    // The owner's own list includes the decrypted address.
    let mine = app
        .request(Method::GET, "/stores/mine", None, Some(&token))
        .await;
    assert_eq!(mine.status, 200);
    assert_eq!(mine.json[0]["pickup_address"], "12 Unique Street, Wuse 2");
}

#[sqlx::test]
async fn non_seller_cannot_create_store(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08011111111").await;
    let res = create(&app, &token, "Nope").await;
    assert_eq!(res.status, 403);
    assert_eq!(res.json["error"]["code"], "not_a_seller");
}

#[sqlx::test]
async fn third_store_rejected(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.make_seller("08011111111").await;
    assert_eq!(create(&app, &token, "One").await.status, 201);
    assert_eq!(create(&app, &token, "Two").await.status, 201);
    let res = create(&app, &token, "Three").await;
    assert_eq!(res.status, 409);
    assert_eq!(res.json["error"]["code"], "store_limit_reached");
}

#[sqlx::test]
async fn deleted_store_frees_slot(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.make_seller("08011111111").await;
    let first = create(&app, &token, "One").await;
    assert_eq!(create(&app, &token, "Two").await.status, 201);
    let id = first.json["id"].as_str().unwrap();
    let del = app
        .request(Method::DELETE, &format!("/stores/{id}"), None, Some(&token))
        .await;
    assert_eq!(del.status, 204);
    assert_eq!(create(&app, &token, "Three").await.status, 201);
}

#[sqlx::test]
async fn concurrent_second_and_third_one_wins(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (_, token) = app.make_seller("08011111111").await;
    assert_eq!(create(&app, &token, "One").await.status, 201);
    let (a, b) = tokio::join!(create(&app, &token, "Two"), create(&app, &token, "Three"));
    let mut statuses = [a.status, b.status];
    statuses.sort();
    assert_eq!(statuses, [201, 409]);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM stores WHERE deleted_at IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[sqlx::test]
async fn non_fct_rejected(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.make_seller("08011111111").await;
    let mut body = store_body("Lagos Shop");
    body["pickup_state"] = json!("Lagos");
    let res = app
        .request(Method::POST, "/stores", Some(body), Some(&token))
        .await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json["error"]["code"], "outside_service_area");
}

#[sqlx::test]
async fn needs_a_delivery_method(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.make_seller("08011111111").await;
    let mut body = store_body("No Delivery");
    body["delivery_courier"] = json!(false);
    body["delivery_self"] = json!(false);
    let res = app
        .request(Method::POST, "/stores", Some(body), Some(&token))
        .await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json["error"]["code"], "validation_failed");
}

#[sqlx::test]
async fn only_owner_can_edit(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.make_seller("08011111111").await;
    let (_, other) = app.make_seller("08022222222").await;
    let store = create(&app, &token, "Mine").await;
    let id = store.json["id"].as_str().unwrap();
    let patch = app
        .request(
            Method::PATCH,
            &format!("/stores/{id}"),
            Some(json!({ "name": "Stolen" })),
            Some(&other),
        )
        .await;
    assert_eq!(patch.status, 404);
    let del = app
        .request(Method::DELETE, &format!("/stores/{id}"), None, Some(&other))
        .await;
    assert_eq!(del.status, 404);
    // Owner can edit.
    let ok = app
        .request(
            Method::PATCH,
            &format!("/stores/{id}"),
            Some(json!({ "name": "Renamed" })),
            Some(&token),
        )
        .await;
    assert_eq!(ok.status, 200, "{}", ok.json);
    assert_eq!(ok.json["name"], "Renamed");
    let bad = app
        .request(
            Method::PATCH,
            &format!("/stores/{id}"),
            Some(json!({ "pickup_state": "Kano" })),
            Some(&token),
        )
        .await;
    assert_eq!(bad.status, 422);
    assert_eq!(bad.json["error"]["code"], "outside_service_area");
}

#[sqlx::test]
async fn pickup_address_encrypted(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (_, token) = app.make_seller("08011111111").await;
    assert_eq!(create(&app, &token, "Secret").await.status, 201);
    let raw: Vec<u8> = sqlx::query_scalar("SELECT pickup_address_enc FROM stores")
        .fetch_one(&pool)
        .await
        .unwrap();
    let needle = b"Unique Street";
    assert!(!raw.windows(needle.len()).any(|w| w == needle));
}

#[sqlx::test]
async fn slug_collision_suffixed(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, a) = app.make_seller("08011111111").await;
    let (_, b) = app.make_seller("08022222222").await;
    assert_eq!(create(&app, &a, "Mama Put").await.json["slug"], "mama-put");
    assert_eq!(
        create(&app, &b, "Mama  Put!").await.json["slug"],
        "mama-put-2"
    );
}

#[sqlx::test]
async fn revoked_code_keeps_existing_store(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let (code_id, code) = app.create_access_code(&owner, json!({})).await;
    let (_, token) = app.sign_up_verified("08011111111").await;
    assert_eq!(app.redeem(&token, &code).await.status, 200);
    let store = create(&app, &token, "Survivor").await;
    assert_eq!(store.status, 201);
    let revoke = app
        .request(
            Method::POST,
            &format!("/admin/access-codes/{code_id}/revoke"),
            None,
            Some(&owner),
        )
        .await;
    assert_eq!(revoke.status, 204);
    let slug = store.json["slug"].as_str().unwrap();
    let got = app
        .request(Method::GET, &format!("/stores/{slug}"), None, Some(&token))
        .await;
    assert_eq!(got.status, 200);
    assert!(got.json.get("pickup_address").is_none());
}

#[sqlx::test]
async fn store_creation_audited(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (user, token) = app.make_seller("08011111111").await;
    let store = create(&app, &token, "Audited").await;
    let id: uuid::Uuid = store.json["id"].as_str().unwrap().parse().unwrap();
    let row: (Option<uuid::Uuid>, Option<uuid::Uuid>, String) = sqlx::query_as(
        "SELECT actor_id, subject_id, detail::text FROM audit_log WHERE action = 'store.created'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.0, Some(user));
    assert_eq!(row.1, Some(id));
    assert!(!row.2.contains("Unique Street"));
}

#[sqlx::test]
async fn reserved_slug_is_skipped(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.make_seller("08011111111").await;
    let res = create(&app, &token, "Mine").await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["slug"], "mine-2");
    let got = app
        .request(Method::GET, "/stores/mine-2", None, Some(&token))
        .await;
    assert_eq!(got.status, 200);
    assert_eq!(got.json["name"], "Mine");
}

fn self_body(name: &str) -> Value {
    json!({
        "name": name,
        "pickup_address": "12 Unique Street, Wuse 2",
        "pickup_state": "FCT",
        "delivery_courier": false,
        "delivery_self": true
    })
}

#[sqlx::test]
async fn self_delivery_store_requires_fee_and_phone(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.make_seller("08011111111").await;
    let res = app
        .request(
            Method::POST,
            "/stores",
            Some(self_body("Own")),
            Some(&token),
        )
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "delivery_details_required");
    assert_eq!(
        res.json["error"]["message"],
        "Self-delivery needs a fee and a business phone"
    );
    let mut body = self_body("Own");
    body["self_delivery_fee_kobo"] = json!(150_000);
    let res = app
        .request(Method::POST, "/stores", Some(body.clone()), Some(&token))
        .await;
    assert_eq!(res.json["error"]["code"], "delivery_details_required");
    body["business_phone"] = json!("08031234567");
    let res = app
        .request(Method::POST, "/stores", Some(body), Some(&token))
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    // Patch: switching self-delivery on for a store without the details is rejected.
    let created = create(&app, &token, "Courier Only").await;
    let id = created.json["id"].as_str().unwrap();
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{id}"),
            Some(json!({ "delivery_self": true })),
            Some(&token),
        )
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "delivery_details_required");
}

#[sqlx::test]
async fn courier_store_requires_pickup_area(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.make_seller("08011111111").await;
    let mut body = store_body("Courier");
    body.as_object_mut().unwrap().remove("pickup_area_council");
    let res = app
        .request(Method::POST, "/stores", Some(body.clone()), Some(&token))
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "delivery_details_required");
    body["pickup_area_council"] = json!("Kano");
    let res = app
        .request(Method::POST, "/stores", Some(body.clone()), Some(&token))
        .await;
    assert_eq!(res.json["error"]["code"], "outside_service_area");
    body["pickup_area_council"] = json!("bwari");
    body["pickup_postcode"] = json!("fc 01-ab12cde");
    let res = app
        .request(Method::POST, "/stores", Some(body), Some(&token))
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["pickup_area_council"], "bwari");
    assert_eq!(res.json["pickup_postcode"], "FC01AB12CDE");
}

#[sqlx::test]
async fn legacy_store_without_delivery_fields_can_still_be_renamed(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (_, token) = app.make_seller("08011111111").await;
    let created = create(&app, &token, "Legacy").await;
    let id = created.json["id"].as_str().unwrap();
    sqlx::query("UPDATE stores SET pickup_area_council = NULL")
        .execute(&pool)
        .await
        .unwrap();
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{id}"),
            Some(json!({ "name": "Legacy Renamed" })),
            Some(&token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    // Touching a delivery field validates the merged state.
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{id}"),
            Some(json!({ "pickup_postcode": "" })),
            Some(&token),
        )
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
}

#[sqlx::test]
async fn business_phone_encrypted_and_owner_only(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (_, token) = app.make_seller("08011111111").await;
    let mut body = self_body("Phone Shop");
    body["self_delivery_fee_kobo"] = json!(150_000);
    body["business_phone"] = json!("0803 123 4567");
    body["pickup_area_council"] = json!("amac");
    body["pickup_postcode"] = json!("FC01AB12CDE");
    let res = app
        .request(Method::POST, "/stores", Some(body), Some(&token))
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["business_phone"], "+2348031234567");
    assert_eq!(res.json["self_delivery_fee_kobo"], 150_000);
    let slug = res.json["slug"].as_str().unwrap().to_string();
    let id: uuid::Uuid = res.json["id"].as_str().unwrap().parse().unwrap();

    let raw: Vec<u8> = sqlx::query_scalar("SELECT business_phone_enc FROM stores")
        .fetch_one(&pool)
        .await
        .unwrap();
    let needle = b"8031234567";
    assert!(!raw.windows(needle.len()).any(|w| w == needle));

    // Make the store public, then look at it anonymously and as another signed-in user.
    sqlx::query("UPDATE stores SET status = 'active', pickup_courier_address_code = 4242")
        .execute(&pool)
        .await
        .unwrap();
    let (_, other) = app.sign_up_verified("08022222222").await;
    for viewer in [None, Some(other.as_str())] {
        let public = app
            .request(Method::GET, &format!("/stores/{slug}"), None, viewer)
            .await;
        assert_eq!(public.status, 200, "{}", public.json);
        for key in [
            "business_phone",
            "self_delivery_fee_kobo",
            "pickup_postcode",
            "pickup_area_council",
            "pickup_courier_address_code",
            "pickup_address",
        ] {
            assert!(
                public.json.get(key).is_none(),
                "{key} leaked: {}",
                public.json
            );
        }
        let text = public.json.to_string();
        assert!(!text.contains("8031234567") && !text.contains("FC01AB12CDE"));
    }
    let mine = app
        .request(Method::GET, "/stores/mine", None, Some(&token))
        .await;
    assert_eq!(mine.json[0]["business_phone"], "+2348031234567");
    assert_eq!(mine.json[0]["self_delivery_fee_kobo"], 150_000);

    // delivery_profile decrypts.
    let profile = server::stores::service::delivery_profile(&pool, &app.state.cipher, id)
        .await
        .unwrap();
    assert_eq!(profile.business_phone.as_deref(), Some("+2348031234567"));
    assert_eq!(profile.self_fee_kobo, Some(150_000));
    assert_eq!(profile.pickup_address, "12 Unique Street, Wuse 2");
    assert_eq!(profile.pickup_courier_address_code, Some(4242));
    assert!(!format!("{profile:?}").contains("8031234567"));

    // The audit row names changed fields, never values.
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{id}"),
            Some(json!({ "business_phone": "08099887766", "self_delivery_fee_kobo": 0 })),
            Some(&token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let detail: String =
        sqlx::query_scalar("SELECT detail::text FROM audit_log WHERE action = 'store.updated'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(detail.contains("business_phone") && detail.contains("self_delivery_fee_kobo"));
    assert!(!detail.contains("8099887766") && !detail.contains("150000"));
}

#[sqlx::test]
async fn self_delivery_fee_capped(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.make_seller("08011111111").await;
    let mut body = self_body("Pricey");
    body["business_phone"] = json!("08031234567");
    body["self_delivery_fee_kobo"] = json!(5_000_001);
    let res = app
        .request(Method::POST, "/stores", Some(body.clone()), Some(&token))
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    body["self_delivery_fee_kobo"] = json!(-1);
    let res = app
        .request(Method::POST, "/stores", Some(body.clone()), Some(&token))
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    body["self_delivery_fee_kobo"] = json!(5_000_000);
    let res = app
        .request(Method::POST, "/stores", Some(body), Some(&token))
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
}

#[sqlx::test]
async fn moving_the_pickup_resets_the_courier_address_code(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (_, token) = app.make_seller("08011111111").await;
    let created = create(&app, &token, "Moves").await;
    let id = created.json["id"].as_str().unwrap().to_string();
    let patches = [
        json!({ "pickup_address": "7 New Road, Garki" }),
        json!({ "pickup_area_council": "kuje" }),
        json!({ "pickup_postcode": "FC01AB12CDE" }),
    ];
    for patch in patches {
        sqlx::query("UPDATE stores SET pickup_courier_address_code = 4242")
            .execute(&pool)
            .await
            .unwrap();
        let res = app
            .request(
                Method::PATCH,
                &format!("/stores/{id}"),
                Some(patch.clone()),
                Some(&token),
            )
            .await;
        assert_eq!(res.status, 200, "{patch}: {}", res.json);
        let code: Option<i64> =
            sqlx::query_scalar("SELECT pickup_courier_address_code FROM stores")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(code, None, "{patch}");
    }
    // An unrelated patch keeps it.
    sqlx::query("UPDATE stores SET pickup_courier_address_code = 4242")
        .execute(&pool)
        .await
        .unwrap();
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{id}"),
            Some(json!({ "name": "Still Here" })),
            Some(&token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let code: Option<i64> = sqlx::query_scalar("SELECT pickup_courier_address_code FROM stores")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(code, Some(4242));
}
