mod common;

use axum::http::Method;
use common::TestApp;
use serde_json::{Value, json};
use server::delivery::addresses::load_for_checkout;
use sqlx::PgPool;
use tracing_test::traced_test;
use uuid::Uuid;

fn body() -> Value {
    json!({
        "recipient_name": "Ada Obi",
        "phone": "0803 123 4567",
        "address_line": "12 Zambezi Crescent, Maitama",
        "landmark": "Opposite the blue gate",
        "area_council": "amac",
        "postcode": "fc-02 a09db09"
    })
}

async fn add(app: &TestApp, token: &str, b: Value) -> common::TestResponse {
    app.request(Method::POST, "/me/addresses", Some(b), Some(token))
        .await
}

#[sqlx::test]
async fn create_list_delete_address(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031234567").await;

    let created = add(&app, &token, body()).await;
    assert_eq!(created.status, 201, "{}", created.json);
    assert_eq!(created.headers["cache-control"], "no-store");
    assert_eq!(created.json["phone"], "+2348031234567");
    assert_eq!(created.json["postcode"], "FC02A09DB09");
    assert_eq!(created.json["area_council"], "amac");
    let id = created.json["id"].as_str().unwrap().to_string();

    // A blank postcode and blank landmark are treated as absent.
    let mut b2 = body();
    b2["postcode"] = json!("  ");
    b2["landmark"] = json!("");
    let second = add(&app, &token, b2).await;
    assert_eq!(second.status, 201, "{}", second.json);
    assert!(second.json["postcode"].is_null());
    assert!(second.json["landmark"].is_null());
    let second_id = second.json["id"].as_str().unwrap().to_string();

    let list = app
        .request(Method::GET, "/me/addresses", None, Some(&token))
        .await;
    assert_eq!(list.status, 200);
    let items = list.json.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], second_id, "newest first");
    assert_eq!(items[1]["address_line"], "12 Zambezi Crescent, Maitama");
    assert_eq!(items[1]["landmark"], "Opposite the blue gate");

    let del = app
        .request(
            Method::DELETE,
            &format!("/me/addresses/{id}"),
            None,
            Some(&token),
        )
        .await;
    assert_eq!(del.status, 204);
    let list = app
        .request(Method::GET, "/me/addresses", None, Some(&token))
        .await;
    assert_eq!(list.json.as_array().unwrap().len(), 1);

    // Deleting again is a 404.
    let again = app
        .request(
            Method::DELETE,
            &format!("/me/addresses/{id}"),
            None,
            Some(&token),
        )
        .await;
    assert_eq!(again.status, 404);

    // Audit rows carry no personal data.
    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT action, detail FROM audit_log WHERE action LIKE 'address.%' ORDER BY id",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    let actions: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        actions,
        ["address.created", "address.created", "address.deleted"]
    );
    assert!(rows.iter().all(|r| !r.1.to_string().contains("Zambezi")));
}

#[sqlx::test]
async fn address_list_is_never_cached(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031234567").await;
    let res = app
        .request(Method::GET, "/me/addresses", None, Some(&token))
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(res.headers["cache-control"], "no-store");
}

#[sqlx::test]
async fn address_outside_fct_is_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031234567").await;
    let mut b = body();
    b["area_council"] = json!("suleja");
    let res = add(&app, &token, b).await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json["error"]["code"], "outside_service_area");
    assert_eq!(res.json["error"]["message"], "Coming to your city soon");
}

#[sqlx::test]
async fn postcode_must_be_fct_when_given(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031234567").await;
    let mut b = body();
    b["postcode"] = json!("LA02A09DB09");
    let res = add(&app, &token, b).await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json["error"]["code"], "outside_service_area");

    let mut b = body();
    b["postcode"] = json!("FC02");
    let res = add(&app, &token, b).await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json["error"]["code"], "invalid_postcode");
}

#[sqlx::test]
async fn address_input_is_validated(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031234567").await;
    for (field, value) in [
        ("recipient_name", json!("x".repeat(101))),
        ("recipient_name", json!("   ")),
        ("address_line", json!("x".repeat(301))),
        ("landmark", json!("x".repeat(201))),
    ] {
        let mut b = body();
        b[field] = value;
        let res = add(&app, &token, b).await;
        assert_eq!(res.status, 422, "{field}");
        assert_eq!(res.json["error"]["code"], "validation_failed");
    }
    let mut b = body();
    b["phone"] = json!("12345");
    let res = add(&app, &token, b).await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json["error"]["code"], "invalid_phone");
}

#[sqlx::test]
async fn at_most_twenty_live_addresses(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031234567").await;
    let mut first = String::new();
    for i in 0..20 {
        let res = add(&app, &token, body()).await;
        assert_eq!(res.status, 201);
        if i == 0 {
            first = res.json["id"].as_str().unwrap().to_string();
        }
    }
    let res = add(&app, &token, body()).await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json["error"]["code"], "too_many_addresses");

    // Deleting one frees a slot.
    app.request(
        Method::DELETE,
        &format!("/me/addresses/{first}"),
        None,
        Some(&token),
    )
    .await;
    assert_eq!(add(&app, &token, body()).await.status, 201);
}

#[sqlx::test]
async fn addresses_are_private_to_their_owner(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, alice) = app.sign_up_verified("08031234567").await;
    let (_, bob) = app.sign_up_verified("08037654321").await;
    let id = add(&app, &alice, body()).await.json["id"]
        .as_str()
        .unwrap()
        .to_string();

    let list = app
        .request(Method::GET, "/me/addresses", None, Some(&bob))
        .await;
    assert_eq!(list.status, 200);
    assert!(list.json.as_array().unwrap().is_empty());

    let del = app
        .request(
            Method::DELETE,
            &format!("/me/addresses/{id}"),
            None,
            Some(&bob),
        )
        .await;
    assert_eq!(del.status, 404);

    // Still alive for Alice.
    let list = app
        .request(Method::GET, "/me/addresses", None, Some(&alice))
        .await;
    assert_eq!(list.json.as_array().unwrap().len(), 1);

    let anon = app.request(Method::GET, "/me/addresses", None, None).await;
    assert_eq!(anon.status, 401);
}

#[sqlx::test]
async fn address_fields_encrypted_at_rest(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031234567").await;
    assert_eq!(add(&app, &token, body()).await.status, 201);

    let (name, phone, line, landmark): (Vec<u8>, Vec<u8>, Vec<u8>, Option<Vec<u8>>) =
        sqlx::query_as(
            "SELECT recipient_name_enc, phone_enc, address_line_enc, landmark_enc \
             FROM buyer_addresses",
        )
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let contains =
        |blob: &[u8], needle: &str| blob.windows(needle.len()).any(|w| w == needle.as_bytes());
    assert!(!contains(&name, "Ada"));
    assert!(!contains(&phone, "8031234567"));
    assert!(!contains(&line, "Zambezi"));
    assert!(!contains(&landmark.unwrap(), "blue gate"));
}

#[traced_test]
#[sqlx::test]
async fn address_logs_have_no_personal_data(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031234567").await;
    let created = add(&app, &token, body()).await;
    let id = created.json["id"].as_str().unwrap().to_string();
    app.request(Method::GET, "/me/addresses", None, Some(&token))
        .await;
    app.request(
        Method::DELETE,
        &format!("/me/addresses/{id}"),
        None,
        Some(&token),
    )
    .await;

    assert!(logs_contain("response"), "expected request logs");
    assert!(!logs_contain("Zambezi"));
    assert!(!logs_contain("Ada Obi"));
    assert!(!logs_contain("8031234567"));
    assert!(!logs_contain("blue gate"));
}

#[sqlx::test]
async fn deleted_address_cannot_be_used(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (alice_id, alice) = app.sign_up_verified("08031234567").await;
    let (bob_id, _) = app.sign_up_verified("08037654321").await;
    let id: Uuid = add(&app, &alice, body()).await.json["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let loaded = load_for_checkout(&app.pool, &app.state.cipher, alice_id, id)
        .await
        .unwrap();
    assert_eq!(loaded.address_line, "12 Zambezi Crescent, Maitama");
    assert_eq!(loaded.phone, "+2348031234567");
    let debug = format!("{loaded:?}");
    assert!(!debug.contains("Zambezi") && !debug.contains("Ada") && !debug.contains("8031234567"));

    // Another user's address is not found.
    assert!(
        load_for_checkout(&app.pool, &app.state.cipher, bob_id, id)
            .await
            .is_err()
    );

    app.request(
        Method::DELETE,
        &format!("/me/addresses/{id}"),
        None,
        Some(&alice),
    )
    .await;
    let err = load_for_checkout(&app.pool, &app.state.cipher, alice_id, id)
        .await
        .unwrap_err();
    assert!(matches!(err, server::error::AppError::NotFound));
}
