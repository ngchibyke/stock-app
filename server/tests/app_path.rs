//! The whole path a phone takes through the app server additions (plan 7a, task 10):
//! a seller lists a product with photos, anyone finds it, a buyer recovers their password,
//! a dispute carries private evidence, and the app reads its config. Everything goes through
//! the routes, with the in-memory object store standing in for S3.

mod common;

use axum::http::Method;
use common::{STORE_PHONE, TEST_PASSWORD, TestApp, TestResponse};
use image::ImageEncoder;
use serde_json::json;
use server::{app_config::AppClientConfig, uploads::store::Bucket};
use sqlx::PgPool;
use uuid::Uuid;

const NEW_PASSWORD: &str = "a-fresh-passphrase-42";

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x * 7 % 256) as u8, (y * 5 % 256) as u8, 128])
    });
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
        .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .unwrap();
    out
}

/// The phone's side of a photo: ask for a slot, put the bytes where told, finalize.
/// Returns the finished key.
async fn upload_photo(app: &TestApp, token: &str, purpose: &str) -> String {
    let bytes = jpeg(640, 480);
    let slot = app
        .request(
            Method::POST,
            "/uploads",
            Some(json!({ "purpose": purpose, "content_type": "image/jpeg", "bytes": bytes.len() })),
            Some(token),
        )
        .await;
    assert_eq!(slot.status, 201, "request slot: {}", slot.json);
    let id = slot.json["upload_id"].as_str().unwrap().to_string();
    let raw_key: String = sqlx::query_scalar("SELECT raw_key FROM uploads WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    app.objects
        .put_raw(Bucket::Private, &raw_key, bytes, "image/jpeg");
    let done = app
        .request(
            Method::POST,
            &format!("/uploads/{id}/finalize"),
            Some(json!({})),
            Some(token),
        )
        .await;
    assert_eq!(done.status, 200, "finalize: {}", done.json);
    done.json["key"].as_str().unwrap().to_string()
}

async fn dispute_photo(app: &TestApp, token: &str, id: &str, key: &str) -> TestResponse {
    app.request(
        Method::GET,
        &format!("/disputes/{id}/photo?key={key}"),
        None,
        Some(token),
    )
    .await
}

#[sqlx::test]
async fn app_path_from_signup_to_search(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.use_app_config(
        AppClientConfig::from_lookup(&|k: &str| match k {
            "APP_MIN_VERSION_ANDROID" => Some("1.0.0".to_string()),
            "APP_TERMS_URL" => Some("https://shop.example/terms".to_string()),
            _ => None,
        })
        .unwrap(),
    );

    // 1. A seller redeems a code, opens a store, and uploads two photos.
    let (_, seller) = app.make_seller("08011112222").await;
    let created = app
        .request(
            Method::POST,
            "/stores",
            Some(json!({
                "name": "Path Store",
                "pickup_address": "12 Unique Street, Wuse 2",
                "pickup_state": "FCT",
                "delivery_courier": true,
                "delivery_self": true,
                "pickup_area_council": "amac",
                "business_phone": STORE_PHONE,
                "self_delivery_fee_kobo": 0
            })),
            Some(&seller),
        )
        .await;
    assert_eq!(created.status, 201, "create store: {}", created.json);
    let store_id: Uuid = created.json["id"].as_str().unwrap().parse().unwrap();
    // A store is public once a payout account is verified.
    sqlx::query("UPDATE platform_settings SET payout_provider = 'fake'")
        .execute(&app.pool)
        .await
        .unwrap();
    app.fake.set_lookup(Ok("ADA OBI".into()));
    let payout = app
        .request(
            Method::PUT,
            &format!("/stores/{store_id}/payout-account"),
            Some(json!({
                "bank_code": "058",
                "account_number": "0123456785",
                "password": TEST_PASSWORD
            })),
            Some(&seller),
        )
        .await;
    assert_eq!(payout.json["status"], "verified", "{}", payout.json);

    let first = upload_photo(&app, &seller, "product").await;
    let second = upload_photo(&app, &seller, "product").await;
    assert_ne!(first, second);
    for key in [&first, &second] {
        assert!(app.objects.object(Bucket::Public, key).is_some());
    }

    // 2. The product carries those keys and is published.
    let product = app
        .request(
            Method::POST,
            &format!("/stores/{store_id}/products"),
            Some(json!({
                "name": "Ankara Shirt",
                "description": "Handmade shirt",
                "category": "fashion",
                "condition": "new",
                "photo_keys": [first, second],
                "weight_grams": 300,
                "base_price_kobo": 1_200_000,
                "variants": [{ "attributes": { "size": "M" }, "stock": 4 }]
            })),
            Some(&seller),
        )
        .await;
    assert_eq!(product.status, 201, "create product: {}", product.json);
    let product_id = product.json["id"].as_str().unwrap().to_string();
    let published = app
        .request(
            Method::PATCH,
            &format!("/products/{product_id}"),
            Some(json!({ "is_published": true })),
            Some(&seller),
        )
        .await;
    assert_eq!(published.status, 200, "publish: {}", published.json);

    // 3. Someone with no account lists categories, searches, and sees the card.
    let categories = app.request(Method::GET, "/categories", None, None).await;
    assert_eq!(categories.status, 200);
    assert_eq!(categories.json[0]["slug"], "fashion");
    let found = app
        .request(Method::GET, "/products?q=shirt", None, None)
        .await;
    assert_eq!(found.status, 200, "search: {}", found.json);
    let card = &found.json["items"][0];
    assert_eq!(card["id"], product_id.as_str());
    assert_eq!(card["photo_key"], first.as_str());
    assert_eq!(
        card["photo_url"],
        format!("https://cdn.fake.invalid/{first}")
    );
    assert_eq!(card["store"]["name"], "Path Store");
    assert!(card["store"]["rating_average"].is_null());
    assert_eq!(card["store"]["rating_count"], 0);
    assert_eq!(card["price_from_kobo"], 1_200_000);

    // 4. A buyer forgets their password and gets back in; the old session is dead.
    let order = app.shipped_order("courier", "delivered").await;
    let me = app
        .request(Method::GET, "/me", None, Some(&order.buyer_token))
        .await;
    let phone = me.json["phone"].as_str().unwrap().to_string();
    let reset = app
        .request(
            Method::POST,
            "/auth/password-reset",
            Some(json!({ "phone": phone })),
            None,
        )
        .await;
    assert_eq!(reset.status, 202, "reset request: {}", reset.json);
    let code = app.otp.last_code(&phone).expect("a reset code was sent");
    let confirmed = app
        .request(
            Method::POST,
            "/auth/password-reset/confirm",
            Some(json!({
                "verification_id": reset.json["verification_id"],
                "code": code,
                "new_password": NEW_PASSWORD,
            })),
            None,
        )
        .await;
    assert_eq!(confirmed.status, 204, "confirm: {}", confirmed.json);
    assert_eq!(
        app.request(Method::GET, "/me", None, Some(&order.buyer_token))
            .await
            .status,
        401,
        "the old session must be dead"
    );
    let signed_in = app.sign_in(&phone, NEW_PASSWORD).await;
    assert_eq!(signed_in.status, 200, "sign in: {}", signed_in.json);
    let buyer = signed_in.json["token"].as_str().unwrap().to_string();

    // 5. The buyer opens a dispute with an evidence photo.
    let evidence = upload_photo(&app, &buyer, "dispute").await;
    assert!(app.objects.object(Bucket::Private, &evidence).is_some());
    assert!(
        app.objects.object(Bucket::Public, &evidence).is_none(),
        "evidence never reaches the public bucket"
    );
    let dispute = app
        .request(
            Method::POST,
            &format!("/orders/{}/dispute", order.order_id),
            Some(json!({
                "reason": "damaged",
                "body": "The zip was torn and the lining was stained when it arrived",
                "photo_keys": [evidence]
            })),
            Some(&buyer),
        )
        .await;
    assert_eq!(dispute.status, 201, "open dispute: {}", dispute.json);
    let dispute_id = dispute.json["id"].as_str().unwrap().to_string();

    // The dispute view holds keys, never links.
    let view = app
        .request(
            Method::GET,
            &format!("/disputes/{dispute_id}"),
            None,
            Some(&buyer),
        )
        .await;
    assert!(!view.json.to_string().contains("http"), "{}", view.json);

    // The seller can read it through the signed route; it expires in a minute.
    let signed = dispute_photo(&app, &order.seller_token, &dispute_id, &evidence).await;
    assert_eq!(signed.status, 200, "seller read: {}", signed.json);
    let url = signed.json["url"].as_str().unwrap();
    assert!(
        url.contains("/private/") && url.ends_with("ttl=60"),
        "{url}"
    );
    // A third user (the other seller) cannot, and learns nothing.
    let stranger = dispute_photo(&app, &seller, &dispute_id, &evidence).await;
    assert_eq!(stranger.status, 404, "stranger: {}", stranger.json);
    // And the public address does not serve it.
    assert!(!url.contains("cdn.fake.invalid"));
    assert!(app.objects.object(Bucket::Public, &evidence).is_none());

    // 6. The app asks for its config.
    let config: TestResponse = app.request(Method::GET, "/app/config", None, None).await;
    assert_eq!(config.status, 200, "config: {}", config.json);
    assert_eq!(config.json["min_version"]["android"], "1.0.0");
    assert_eq!(config.json["terms_url"], "https://shop.example/terms");
    assert_eq!(
        config.json["maintenance"],
        json!({ "active": false, "message": null })
    );
}
