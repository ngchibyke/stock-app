//! Photo upload and key ownership (plan 7, S2).

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse, tiny_png};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use serde_json::{Value, json};
use server::uploads::{ObjectStore, S3Config, S3Store};
use sqlx::PgPool;
use std::io::Cursor;
use uuid::Uuid;

async fn upload(
    app: &TestApp,
    token: Option<&str>,
    content_type: &str,
    body: Vec<u8>,
) -> TestResponse {
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut headers = vec![("content-type", content_type)];
    if let Some(a) = auth.as_deref() {
        headers.push(("authorization", a));
    }
    app.send_raw(Method::POST, "/uploads", &headers, body).await
}

fn encode(img: DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), format).unwrap();
    out
}

fn solid(w: u32, h: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, Rgb([10, 120, 200])))
}

/// A JPEG with an EXIF block carrying `orientation` and some "GPS" text.
fn jpeg_with_exif(w: u32, h: u32, orientation: u8) -> Vec<u8> {
    let jpeg = encode(solid(w, h), ImageFormat::Jpeg);
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"MM\x00\x2a\x00\x00\x00\x08"); // big-endian, IFD at 8
    tiff.extend_from_slice(&[0x00, 0x01]); // one entry
    tiff.extend_from_slice(&[
        0x01,
        0x12,
        0x00,
        0x03,
        0x00,
        0x00,
        0x00,
        0x01,
        0x00,
        orientation,
        0x00,
        0x00,
    ]);
    tiff.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // no next IFD
    let mut app1 = b"Exif\x00\x00".to_vec();
    app1.extend_from_slice(&tiff);
    app1.extend_from_slice(b"GPS-SECRET-6.5244N-3.3792E");
    let len = (app1.len() + 2) as u16;
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[sqlx::test]
async fn a_signed_in_user_uploads_a_photo_and_gets_a_key(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (user, token) = app.sign_up_verified("08031110001").await;
    let res = upload(&app, Some(&token), "image/png", tiny_png()).await;
    assert_eq!(res.status, 201, "{}", res.json);
    let key = res.json["key"].as_str().unwrap().to_string();
    assert!(key.starts_with("photos/") && key.ends_with(".png"), "{key}");
    assert_eq!(res.json["content_type"], "image/png");
    assert_eq!(
        (res.json["width"].as_i64(), res.json["height"].as_i64()),
        (Some(2), Some(2))
    );

    let (bytes, content_type) = app.photos.get(&key).expect("stored");
    assert_eq!(content_type, "image/png");
    assert_eq!(res.json["bytes"].as_i64().unwrap() as usize, bytes.len());
    assert!(
        image::load_from_memory(&bytes).is_ok(),
        "what is stored is a real image"
    );

    let owner: Uuid = sqlx::query_scalar("SELECT uploader_id FROM uploads WHERE key = $1")
        .bind(&key)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner, user);
    // Two uploads never share a key.
    let again = upload(&app, Some(&token), "image/png", tiny_png()).await;
    assert_ne!(again.json["key"], res.json["key"]);
}

#[sqlx::test]
async fn jpeg_and_webp_are_accepted_too(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031110001").await;
    let jpeg = upload(
        &app,
        Some(&token),
        "image/jpeg",
        encode(solid(8, 6), ImageFormat::Jpeg),
    )
    .await;
    assert_eq!(jpeg.status, 201, "{}", jpeg.json);
    assert!(jpeg.json["key"].as_str().unwrap().ends_with(".jpg"));
    let webp = upload(
        &app,
        Some(&token),
        "image/webp",
        encode(solid(8, 6), ImageFormat::WebP),
    )
    .await;
    assert_eq!(webp.status, 201, "{}", webp.json);
    assert!(webp.json["key"].as_str().unwrap().ends_with(".webp"));
}

#[sqlx::test]
async fn signing_in_is_required(pool: PgPool) {
    let app = TestApp::new(pool).await;
    assert_eq!(
        upload(&app, None, "image/png", tiny_png()).await.status,
        401
    );
    assert!(app.photos.is_empty());
}

#[sqlx::test]
async fn only_real_jpeg_png_and_webp_are_accepted_whatever_the_name_or_header_says(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (_, token) = app.sign_up_verified("08031110001").await;
    // The start of a GIF: enough for the type to be recognised and then refused.
    let gif = b"GIF89a\x02\x00\x02\x00\x80\x00\x00\xff\xff\xff\x00\x00\x00".to_vec();
    for (what, body) in [
        ("a gif", gif),
        (
            "plain text called .png",
            b"hello, this is not an image".to_vec(),
        ),
        (
            "an html page",
            b"<html><script>alert(1)</script></html>".to_vec(),
        ),
        ("nothing", Vec::new()),
    ] {
        let res = upload(&app, Some(&token), "image/png", body).await;
        assert_eq!(res.status, 422, "{what}: {}", res.json);
        assert_eq!(res.json["error"]["code"], "unsupported_image", "{what}");
    }
    // A PNG header followed by rubbish is a PNG that cannot be read.
    let mut broken = tiny_png();
    broken.truncate(30);
    let res = upload(&app, Some(&token), "image/png", broken).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "invalid_image");
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM uploads")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert!(app.photos.is_empty());
}

#[sqlx::test]
async fn a_photo_over_five_megabytes_is_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031110001").await;
    let mut big = tiny_png();
    big.resize(5 * 1024 * 1024 + 1, 0);
    let res = upload(&app, Some(&token), "image/png", big).await;
    assert_eq!(res.status, 413, "{}", res.json);
    assert!(app.photos.is_empty());
}

#[sqlx::test]
async fn a_photo_too_big_in_pixels_is_refused_before_it_is_decoded(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031110001").await;
    // 4097 x 1 is a few kilobytes as a file; 4096 x 4096 would be a 50 MB bitmap.
    let wide = encode(solid(4097, 1), ImageFormat::Png);
    let res = upload(&app, Some(&token), "image/png", wide).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "image_too_large");
    let edge = encode(solid(4096, 1), ImageFormat::Png);
    assert_eq!(
        upload(&app, Some(&token), "image/png", edge).await.status,
        201
    );
}

#[sqlx::test]
async fn location_and_hidden_data_do_not_survive(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031110001").await;

    // EXIF with a GPS-like string inside a JPEG.
    let res = upload(&app, Some(&token), "image/jpeg", jpeg_with_exif(8, 4, 1)).await;
    assert_eq!(res.status, 201, "{}", res.json);
    let (stored, _) = app.photos.get(res.json["key"].as_str().unwrap()).unwrap();
    assert!(!contains(&stored, b"GPS-SECRET"), "EXIF survived");
    assert!(!contains(&stored, b"Exif"), "an EXIF block survived");

    // Bytes smuggled after the end of a PNG.
    let mut png = tiny_png();
    png.extend_from_slice(b"HIDDEN-PAYLOAD-AFTER-THE-IMAGE");
    let res = upload(&app, Some(&token), "image/png", png).await;
    assert_eq!(res.status, 201, "{}", res.json);
    let (stored, _) = app.photos.get(res.json["key"].as_str().unwrap()).unwrap();
    assert!(!contains(&stored, b"HIDDEN-PAYLOAD"));
}

#[sqlx::test]
async fn a_photo_taken_sideways_is_turned_upright_before_the_orientation_is_dropped(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.sign_up_verified("08031110001").await;
    // 8 wide by 4 tall, marked "rotate 90": it is really 4 wide by 8 tall.
    let res = upload(&app, Some(&token), "image/jpeg", jpeg_with_exif(8, 4, 6)).await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(
        (res.json["width"].as_i64(), res.json["height"].as_i64()),
        (Some(4), Some(8))
    );
    let (stored, _) = app.photos.get(res.json["key"].as_str().unwrap()).unwrap();
    let img = image::load_from_memory(&stored).unwrap();
    assert_eq!((img.width(), img.height()), (4, 8));
}

#[sqlx::test]
async fn no_object_store_means_503_and_a_failed_store_leaves_nothing_behind(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (_, token) = app.sign_up_verified("08031110001").await;

    app.photos.fail(true);
    let res = upload(&app, Some(&token), "image/png", tiny_png()).await;
    assert_eq!(res.status, 503, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "uploads_unavailable");
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM uploads")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "a photo that was not stored must not be recorded");
    app.photos.fail(false);
    assert_eq!(
        upload(&app, Some(&token), "image/png", tiny_png())
            .await
            .status,
        201
    );

    // No store configured at all.
    let mut state = app.state.clone();
    state.photos = None;
    let router = server::build_app(state);
    use tower::ServiceExt;
    let req = axum::http::Request::builder()
        .method(Method::POST)
        .uri("/uploads")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "image/png")
        .body(axum::body::Body::from(tiny_png()))
        .unwrap();
    assert_eq!(router.oneshot(req).await.unwrap().status().as_u16(), 503);
}

#[sqlx::test]
async fn sixty_uploads_an_hour_is_the_limit(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (user, token) = app.sign_up_verified("08031110001").await;
    for i in 0..60 {
        sqlx::query(
            "INSERT INTO uploads (key, uploader_id, content_type, bytes, width, height) \
             VALUES ($1, $2, 'image/png', 10, 1, 1)",
        )
        .bind(format!("photos/{}.png", Uuid::from_u128(i + 1)))
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    }
    let res = upload(&app, Some(&token), "image/png", tiny_png()).await;
    assert_eq!(res.status, 429, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "too_many_uploads");
    // Someone else is not affected, and an old upload stops counting.
    let (_, other) = app.sign_up_verified("08031110002").await;
    assert_eq!(
        upload(&app, Some(&other), "image/png", tiny_png())
            .await
            .status,
        201
    );
    sqlx::query(
        "UPDATE uploads SET created_at = now() - interval '2 hours' WHERE uploader_id = $1",
    )
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        upload(&app, Some(&token), "image/png", tiny_png())
            .await
            .status,
        201
    );
}

// ------------------------------------------------------------------ ownership

fn product(photos: Value) -> Value {
    json!({
        "name": "Ankara Shirt", "description": "Handmade shirt", "category": "fashion",
        "condition": "new", "photo_keys": photos, "weight_grams": 300, "base_price_kobo": 1_200_000,
        "variants": [{ "attributes": {}, "stock": 3 }]
    })
}

#[sqlx::test]
async fn only_the_uploader_can_attach_a_photo_to_a_product(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let mine = app
        .make_active_store_with_product("08031110001", "Alpha", 500_000, 3)
        .await;
    let theirs = app
        .make_active_store_with_product("08031110002", "Beta", 500_000, 3)
        .await;
    let their_photo = app.upload_photo(&theirs.seller_token).await;
    let my_photo = app.upload_photo(&mine.seller_token).await;
    let path = format!("/stores/{}/products", mine.store_id);

    let steal = app
        .request(
            Method::POST,
            &path,
            Some(product(json!([their_photo]))),
            Some(&mine.seller_token),
        )
        .await;
    assert_eq!(steal.status, 422, "{}", steal.json);
    assert_eq!(steal.json["error"]["code"], "photo_not_yours");
    let invented = app
        .request(
            Method::POST,
            &path,
            Some(product(json!(["photos/never-uploaded.jpg"]))),
            Some(&mine.seller_token),
        )
        .await;
    assert_eq!(invented.json["error"]["code"], "photo_not_yours");
    // One foreign key among good ones still fails.
    let mixed = app
        .request(
            Method::POST,
            &path,
            Some(product(json!([my_photo, their_photo]))),
            Some(&mine.seller_token),
        )
        .await;
    assert_eq!(mixed.status, 422);
    let ok = app
        .request(
            Method::POST,
            &path,
            Some(product(json!([my_photo]))),
            Some(&mine.seller_token),
        )
        .await;
    assert_eq!(ok.status, 201, "{}", ok.json);
    // The same photo can be reused on another of your products.
    let again = app
        .request(
            Method::POST,
            &path,
            Some(product(json!([my_photo]))),
            Some(&mine.seller_token),
        )
        .await;
    assert_eq!(again.status, 201);

    // Updating: a new foreign key is refused; photos already on the product are kept.
    let id = ok.json["id"].as_str().unwrap();
    let patch = |body: Value| {
        let app = &app;
        let token = mine.seller_token.clone();
        let path = format!("/products/{id}");
        async move {
            app.request(Method::PATCH, &path, Some(body), Some(&token))
                .await
        }
    };
    assert_eq!(
        patch(json!({ "photo_keys": [their_photo] })).await.status,
        422
    );
    assert_eq!(patch(json!({ "photo_keys": [my_photo] })).await.status, 200);
    // A photo attached before uploads were tracked stays valid while it is kept.
    sqlx::query("UPDATE products SET photo_keys = ARRAY['photos/legacy.jpg'] WHERE id = $1::uuid")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(patch(json!({ "name": "Renamed" })).await.status, 200);
    assert_eq!(
        patch(json!({ "photo_keys": ["photos/legacy.jpg"] }))
            .await
            .status,
        200
    );
    assert_eq!(
        patch(json!({ "photo_keys": ["photos/legacy.jpg", their_photo] }))
            .await
            .status,
        422
    );
}

#[sqlx::test]
async fn disputes_and_their_answers_take_only_the_senders_own_photos(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("self", "delivered").await;
    let seller_photo = app.upload_photo(&o.seller_token).await;
    let buyer_photo = app.upload_photo(&o.buyer_token).await;
    let report = |photo: &str| {
        json!({ "reason": "damaged", "photo_keys": [photo],
                "body": "The zip was torn and the lining was stained when it arrived" })
    };
    let path = format!("/orders/{}/dispute", o.order_id);

    let wrong = app
        .request(
            Method::POST,
            &path,
            Some(report(&seller_photo)),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(wrong.status, 422, "{}", wrong.json);
    assert_eq!(wrong.json["error"]["code"], "photo_not_yours");
    // A stranger still gets 404, not a hint about the order.
    let (_, stranger) = app.sign_up_verified("08031119999").await;
    assert_eq!(
        app.request(
            Method::POST,
            &path,
            Some(report(&buyer_photo)),
            Some(&stranger)
        )
        .await
        .status,
        404
    );
    let opened = app
        .request(
            Method::POST,
            &path,
            Some(report(&buyer_photo)),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(opened.status, 201, "{}", opened.json);
    let id = opened.json["id"].as_str().unwrap();

    let answer = |photo: &str| json!({ "body": "It left the shop in perfect condition", "photo_keys": [photo] });
    let wrong = app
        .request(
            Method::POST,
            &format!("/disputes/{id}/response"),
            Some(answer(&buyer_photo)),
            Some(&o.seller_token),
        )
        .await;
    assert_eq!(wrong.json["error"]["code"], "photo_not_yours");
    let ok = app
        .request(
            Method::POST,
            &format!("/disputes/{id}/response"),
            Some(answer(&seller_photo)),
            Some(&o.seller_token),
        )
        .await;
    assert_eq!(ok.status, 200, "{}", ok.json);
}

// ------------------------------------------------------------------- S3 store

#[tokio::test]
async fn the_s3_store_puts_a_signed_object_at_the_bucket_path() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header_exists, method, path},
    };
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/photos-bucket/photos/abc.png"))
        .and(header_exists("authorization"))
        .and(header_exists("x-amz-date"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let store = S3Store::new(S3Config {
        endpoint: server.uri(),
        bucket: "photos-bucket".into(),
        region: "auto".into(),
        access_key_id: "AKIATESTKEY".into(),
        secret_access_key: server::payments::mono::Secret::new("secret-value"),
        photo_base_url: "https://cdn.example/".into(),
        insecure_http_for_tests: true,
    })
    .unwrap();
    store
        .put("photos/abc.png", b"imagebytes".to_vec(), "image/png")
        .await
        .unwrap();

    let req = &server.received_requests().await.unwrap()[0];
    let auth = req.headers.get("authorization").unwrap().to_str().unwrap();
    assert!(
        auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIATESTKEY/"),
        "{auth}"
    );
    assert!(auth.contains("/auto/s3/aws4_request"), "{auth}");
    assert!(
        auth.contains("SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date"),
        "{auth}"
    );
    assert!(!auth.contains("secret-value"));
    // The body hash it signed is the hash of the body it sent.
    let hash = req
        .headers
        .get("x-amz-content-sha256")
        .unwrap()
        .to_str()
        .unwrap();
    use sha2::{Digest, Sha256};
    let want: String = Sha256::digest(b"imagebytes")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(hash, want);
    assert_eq!(req.headers.get("content-type").unwrap(), "image/png");
    assert_eq!(req.body, b"imagebytes");
}

#[tokio::test]
async fn the_s3_store_reports_a_refusal_without_leaking_anything() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_string("<Error>SignatureDoesNotMatch secret-bucket</Error>"),
        )
        .mount(&server)
        .await;
    let store = S3Store::new(S3Config {
        endpoint: server.uri(),
        bucket: "b".into(),
        region: "auto".into(),
        access_key_id: "k".into(),
        secret_access_key: server::payments::mono::Secret::new("s"),
        photo_base_url: "https://cdn.example/".into(),
        insecure_http_for_tests: true,
    })
    .unwrap();
    let err = store
        .put("photos/x.png", vec![1], "image/png")
        .await
        .unwrap_err();
    let text = format!("{err} {err:?}");
    assert!(
        !text.contains("secret-bucket") && !text.contains("SignatureDoesNotMatch"),
        "{text}"
    );
}
