//! Photo upload requests and the finalize step (plan 7a, task 5), with the
//! in-memory object store.

mod common;

use std::sync::Arc;

use axum::http::Method;
use common::{TestApp, TestResponse};
use image::{GenericImageView, ImageEncoder};
use serde_json::{Value, json};
use server::{
    escrow::jobs::run_due_jobs,
    uploads::store::{Bucket, NoStore},
};
use sqlx::PgPool;
use uuid::Uuid;

const MIB: usize = 1024 * 1024;

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x * 7 % 256) as u8, (y * 5 % 256) as u8, 128])
    });
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
        .encode_image(&img)
        .unwrap();
    out
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, 7, 255])
    });
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}

/// Puts an APP1 (EXIF) segment right after the start of a JPEG.
fn with_exif(jpeg: &[u8], payload: &[u8]) -> Vec<u8> {
    let len = (payload.len() + 2) as u16;
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1, (len >> 8) as u8, len as u8]);
    out.extend_from_slice(payload);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// EXIF that says "rotate 90 degrees clockwise to display" (orientation 6).
fn exif_orientation_6() -> Vec<u8> {
    let mut p = b"Exif\0\0".to_vec();
    p.extend_from_slice(&[
        0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00, // little-endian TIFF header
        0x01, 0x00, // one entry
        0x12, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00,
        0x00, // Orientation = 6
        0x00, 0x00, 0x00, 0x00, // no next IFD
    ]);
    p
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A tiny PNG whose header claims a gigantic canvas (a decompression bomb).
fn png_claiming(w: u32, h: u32) -> Vec<u8> {
    let mut bytes = png(1, 1);
    // IHDR data starts at byte 16: width, height; its CRC follows 13 bytes later.
    bytes[16..20].copy_from_slice(&w.to_be_bytes());
    bytes[20..24].copy_from_slice(&h.to_be_bytes());
    let crc = crc32(&bytes[12..29]);
    bytes[29..33].copy_from_slice(&crc.to_be_bytes());
    bytes
}

async fn request_upload(
    app: &TestApp,
    token: &str,
    purpose: &str,
    content_type: &str,
    bytes: usize,
) -> TestResponse {
    app.request(
        Method::POST,
        "/uploads",
        Some(json!({ "purpose": purpose, "content_type": content_type, "bytes": bytes })),
        Some(token),
    )
    .await
}

async fn finalize(app: &TestApp, token: &str, id: &str) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/uploads/{id}/finalize"),
        Some(json!({})),
        Some(token),
    )
    .await
}

/// The phone's side: asks for a slot, puts the bytes where it was told, returns the id.
async fn upload(
    app: &TestApp,
    token: &str,
    purpose: &str,
    content_type: &str,
    bytes: Vec<u8>,
) -> String {
    let res = request_upload(app, token, purpose, content_type, bytes.len()).await;
    assert_eq!(res.status, 201, "{}", res.json);
    let id = res.json["upload_id"].as_str().unwrap().to_string();
    let raw: String = sqlx::query_scalar("SELECT raw_key FROM uploads WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    app.objects
        .put_raw(Bucket::Private, &raw, bytes, content_type);
    id
}

async fn seller(app: &TestApp) -> String {
    app.make_seller("08011111111").await.1
}

async fn buyer(app: &TestApp) -> (Uuid, String) {
    app.sign_up_verified("08022222221").await
}

fn reason(res: &TestResponse) -> (u16, Option<&str>, Option<&str>) {
    (
        res.status,
        res.json["error"]["code"].as_str(),
        res.json["error"]["message"].as_str(),
    )
}

async fn status_of(app: &TestApp, id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM uploads WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn upload_round_trip(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    let res = request_upload(&app, &token, "product", "image/jpeg", 1234).await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert!(res.json["url"].as_str().unwrap().starts_with("https://"));
    assert!(res.json["expires_at"].as_str().is_some());
    let id = res.json["upload_id"].as_str().unwrap().to_string();
    assert_eq!(status_of(&app, &id).await, "pending");

    let raw_key: String = sqlx::query_scalar("SELECT raw_key FROM uploads WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(raw_key.starts_with("raw/"));
    let bytes = jpeg(300, 200);
    app.objects
        .put_raw(Bucket::Private, &raw_key, bytes.clone(), "image/jpeg");
    // The declared size has to be the real one.
    sqlx::query("UPDATE uploads SET declared_bytes = $2 WHERE id = $1::uuid")
        .bind(&id)
        .bind(bytes.len() as i32)
        .execute(&app.pool)
        .await
        .unwrap();

    let done = finalize(&app, &token, &id).await;
    assert_eq!(done.status, 200, "{}", done.json);
    let key = done.json["key"].as_str().unwrap().to_string();
    assert_eq!(key, format!("u/product/{id}.jpg"));
    assert_eq!(status_of(&app, &id).await, "ready");
    // A JPEG in the public bucket, and the raw upload gone.
    let out = app
        .objects
        .object(Bucket::Public, &key)
        .expect("final object");
    assert_eq!(&out[..2], &[0xFF, 0xD8]);
    assert_eq!(
        app.objects.content_type(Bucket::Public, &key).as_deref(),
        Some("image/jpeg")
    );
    assert!(app.objects.object(Bucket::Private, &raw_key).is_none());
    // Audited with the purpose and the sizes, nothing else.
    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT action, detail FROM audit_log WHERE action LIKE 'upload.%' ORDER BY id",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "upload.created");
    assert_eq!(rows[0].1["purpose"], "product");
    assert_eq!(rows[1].0, "upload.finalized");
    assert!(
        !rows[1].1.to_string().contains(&key),
        "no object key in the audit trail"
    );
}

#[sqlx::test]
async fn declared_length_and_type_are_what_the_phone_must_send(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    let res = request_upload(&app, &token, "product", "image/png", 4321).await;
    assert_eq!(res.json["headers"]["content-type"], "image/png");
    assert_eq!(res.json["headers"]["content-length"], "4321");
}

#[sqlx::test]
async fn finalize_rejects_oversize_and_bombs(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;

    // More bytes than declared (and over the cap): refused before any decoding.
    let id = upload(&app, &token, "product", "image/jpeg", vec![0u8; 100]).await;
    let raw: String = sqlx::query_scalar("SELECT raw_key FROM uploads WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    app.objects
        .put_raw(Bucket::Private, &raw, vec![0u8; 5 * MIB + 10], "image/jpeg");
    let res = finalize(&app, &token, &id).await;
    assert_eq!(reason(&res).1, Some("photo_rejected"), "{}", res.json);
    assert_eq!(status_of(&app, &id).await, "failed");
    assert!(
        app.objects.object(Bucket::Private, &raw).is_none(),
        "raw upload deleted"
    );

    // A tiny PNG that claims a 60000 x 60000 canvas.
    let bomb = png_claiming(60_000, 60_000);
    let id = upload(&app, &token, "product", "image/png", bomb).await;
    let res = finalize(&app, &token, &id).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "photo_rejected");
    assert!(
        res.json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("pixels"),
        "{}",
        res.json
    );
    assert_eq!(status_of(&app, &id).await, "failed");
    assert!(
        app.objects.keys(Bucket::Public).is_empty(),
        "nothing reached the public bucket"
    );

    // Just over the edge limit is refused too; just under is fine.
    let id = upload(&app, &token, "product", "image/jpeg", jpeg(4100, 8)).await;
    assert_eq!(finalize(&app, &token, &id).await.status, 422);
    let id = upload(&app, &token, "product", "image/jpeg", jpeg(4096, 8)).await;
    assert_eq!(finalize(&app, &token, &id).await.status, 200);
}

#[sqlx::test]
async fn finalize_rejects_non_images_and_mismatched_types(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    let cases: [(&str, Vec<u8>); 5] = [
        (
            "image/png",
            b"just some text, not a picture at all".to_vec(),
        ),
        ("image/png", b"#!/bin/sh\nrm -rf /\n".to_vec()),
        ("image/jpeg", png(8, 8)), // declared one thing, sent another
        ("image/jpeg", jpeg(40, 40)[..60].to_vec()), // cut off
        ("image/webp", vec![0u8; 64]),
    ];
    for (i, (content_type, bytes)) in cases.into_iter().enumerate() {
        let id = upload(&app, &token, "product", content_type, bytes).await;
        let res = finalize(&app, &token, &id).await;
        assert_eq!(res.status, 422, "case {i}: {}", res.json);
        assert_eq!(res.json["error"]["code"], "photo_rejected", "case {i}");
        assert_eq!(status_of(&app, &id).await, "failed", "case {i}");
    }
    assert!(app.objects.keys(Bucket::Public).is_empty());
    // A rejected photo's message is fixed copy, never the decoder's text.
    let id = upload(&app, &token, "product", "image/png", b"plain text".to_vec()).await;
    let res = finalize(&app, &token, &id).await;
    let msg = res.json["error"]["message"].as_str().unwrap();
    assert!(msg.contains("not a JPEG, PNG or WebP"), "{msg}");
}

#[sqlx::test]
async fn finalize_strips_exif_and_keeps_the_picture_upright(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    let sentinel = b"GPSSENTINEL-12.3456N-7.4890E";
    let mut payload = exif_orientation_6();
    payload.extend_from_slice(sentinel);
    let id = upload(
        &app,
        &token,
        "product",
        "image/jpeg",
        with_exif(&jpeg(40, 20), &payload),
    )
    .await;
    let res = finalize(&app, &token, &id).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let out = app
        .objects
        .object(Bucket::Public, res.json["key"].as_str().unwrap())
        .unwrap();
    assert!(
        !out.windows(sentinel.len()).any(|w| w == sentinel),
        "GPS data survived"
    );
    assert!(
        !out.windows(2).any(|w| w == [0xFF, 0xE1]),
        "an EXIF segment survived"
    );
    // Orientation 6 means the stored 40x20 picture is shown 20x40: the server turned it.
    let (w, h) = image::load_from_memory(&out).unwrap().dimensions();
    assert_eq!((w, h), (20, 40));
}

#[sqlx::test]
async fn finalize_shrinks_big_photos_and_flattens_transparency(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    let id = upload(&app, &token, "product", "image/jpeg", jpeg(3000, 1500)).await;
    let res = finalize(&app, &token, &id).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let out = app
        .objects
        .object(Bucket::Public, res.json["key"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        image::load_from_memory(&out).unwrap().dimensions(),
        (2048, 1024)
    );

    // A PNG with a fully transparent pixel comes out as a JPEG, white there.
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 0]));
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(img.as_raw(), 8, 8, image::ExtendedColorType::Rgba8)
        .unwrap();
    let id = upload(&app, &token, "product", "image/png", bytes).await;
    let res = finalize(&app, &token, &id).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let out = app
        .objects
        .object(Bucket::Public, res.json["key"].as_str().unwrap())
        .unwrap();
    assert_eq!(&out[..2], &[0xFF, 0xD8]);
    let px = image::load_from_memory(&out)
        .unwrap()
        .to_rgb8()
        .get_pixel(4, 4)
        .0;
    assert!(
        px.iter().all(|c| *c > 240),
        "transparent became white, got {px:?}"
    );
}

#[sqlx::test]
async fn finalize_checks_the_declared_size_and_waits_for_the_upload(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    // The phone has not uploaded yet: not an error that spoils the slot.
    let res = request_upload(&app, &token, "product", "image/jpeg", 100).await;
    let id = res.json["upload_id"].as_str().unwrap().to_string();
    let res = finalize(&app, &token, &id).await;
    assert_eq!(reason(&res).1, Some("upload_missing"), "{}", res.json);
    assert_eq!(status_of(&app, &id).await, "pending");
    // Smaller than declared is refused and spent.
    let raw: String = sqlx::query_scalar("SELECT raw_key FROM uploads WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    app.objects
        .put_raw(Bucket::Private, &raw, jpeg(10, 10), "image/jpeg");
    let res = finalize(&app, &token, &id).await;
    assert_eq!(reason(&res).1, Some("photo_rejected"), "{}", res.json);
    assert_eq!(status_of(&app, &id).await, "failed");
}

#[sqlx::test]
async fn finalize_is_idempotent_and_owner_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    let (_, stranger) = buyer(&app).await;
    let id = upload(&app, &token, "product", "image/jpeg", jpeg(50, 50)).await;
    assert_eq!(finalize(&app, &stranger, &id).await.status, 404);
    assert_eq!(
        status_of(&app, &id).await,
        "pending",
        "a stranger cannot spend it"
    );
    let first = finalize(&app, &token, &id).await;
    let second = finalize(&app, &token, &id).await;
    assert_eq!(first.status, 200);
    assert_eq!(first.json["key"], second.json["key"]);
    assert_eq!(app.objects.keys(Bucket::Public).len(), 1);
    // A failed one answers the same way each time.
    let bad = upload(
        &app,
        &token,
        "product",
        "image/png",
        b"not an image".to_vec(),
    )
    .await;
    let a = finalize(&app, &token, &bad).await;
    let b = finalize(&app, &token, &bad).await;
    assert_eq!(
        (a.status, a.json["error"]["message"].clone()),
        (b.status, b.json["error"]["message"].clone())
    );
    // An id that is not one, and one that does not exist.
    assert_eq!(finalize(&app, &token, "nope").await.status, 404);
    assert_eq!(
        finalize(&app, &token, &Uuid::new_v4().to_string())
            .await
            .status,
        404
    );
}

#[sqlx::test]
async fn concurrent_finalizes_leave_one_object(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    let id = upload(&app, &token, "product", "image/jpeg", jpeg(60, 60)).await;
    let (a, b) = tokio::join!(finalize(&app, &token, &id), finalize(&app, &token, &id));
    assert_eq!((a.status, b.status), (200, 200), "{} {}", a.json, b.json);
    assert_eq!(a.json["key"], b.json["key"]);
    assert_eq!(app.objects.keys(Bucket::Public).len(), 1);
}

#[sqlx::test]
async fn purposes_and_buckets(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    for purpose in ["product", "store_logo", "store_banner", "review"] {
        let id = upload(&app, &token, purpose, "image/jpeg", jpeg(20, 20)).await;
        let res = finalize(&app, &token, &id).await;
        assert_eq!(
            res.json["key"],
            format!("u/{purpose}/{id}.jpg"),
            "{purpose}"
        );
        assert!(
            app.objects
                .object(Bucket::Public, res.json["key"].as_str().unwrap())
                .is_some(),
            "{purpose}"
        );
    }
    let id = upload(&app, &token, "dispute", "image/jpeg", jpeg(20, 20)).await;
    let res = finalize(&app, &token, &id).await;
    let key = res.json["key"].as_str().unwrap().to_string();
    assert!(key.starts_with("u/dispute/"));
    assert!(
        app.objects.object(Bucket::Private, &key).is_some(),
        "evidence is private"
    );
    assert!(
        app.objects.object(Bucket::Public, &key).is_none(),
        "and never public"
    );
    // Raw uploads never sit in the public bucket, not even for a moment.
    assert!(
        app.objects
            .keys(Bucket::Public)
            .iter()
            .all(|k| k.starts_with("u/"))
    );
    assert!(
        app.objects
            .keys(Bucket::Private)
            .iter()
            .all(|k| k.starts_with("u/dispute/"))
    );
}

#[sqlx::test]
async fn seller_only_purposes_refuse_buyers(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = buyer(&app).await;
    for purpose in ["product", "store_logo", "store_banner"] {
        let res = request_upload(&app, &token, purpose, "image/jpeg", 100).await;
        assert_eq!(
            reason(&res).1,
            Some("not_a_seller"),
            "{purpose}: {}",
            res.json
        );
        assert_eq!(res.status, 403);
    }
    for purpose in ["review", "dispute"] {
        assert_eq!(
            request_upload(&app, &token, purpose, "image/jpeg", 100)
                .await
                .status,
            201,
            "{purpose}"
        );
    }
}

#[sqlx::test]
async fn requests_are_validated(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    let bad = [
        json!({ "purpose": "avatar", "content_type": "image/jpeg", "bytes": 100 }),
        json!({ "purpose": "product", "content_type": "image/gif", "bytes": 100 }),
        json!({ "purpose": "product", "content_type": "text/html", "bytes": 100 }),
        json!({ "purpose": "product", "content_type": "image/jpeg", "bytes": 0 }),
        json!({ "purpose": "product", "content_type": "image/jpeg", "bytes": 5 * MIB + 1 }),
        json!({ "purpose": "product", "content_type": "image/jpeg", "bytes": -4 }),
    ];
    for body in bad {
        let res = app
            .request(Method::POST, "/uploads", Some(body.clone()), Some(&token))
            .await;
        assert_eq!(res.status, 422, "{body}: {}", res.json);
    }
    let ok = request_upload(&app, &token, "product", "image/webp", 5 * MIB).await;
    assert_eq!(ok.status, 201, "{}", ok.json);
    // Signing in is required.
    let res = app
        .request(
            Method::POST,
            "/uploads",
            Some(json!({ "purpose": "review", "content_type": "image/jpeg", "bytes": 1 })),
            None,
        )
        .await;
    assert_eq!(res.status, 401);
}

#[sqlx::test]
async fn hourly_cap_is_enforced(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    for i in 0..30 {
        let res = request_upload(&app, &token, "product", "image/jpeg", 100).await;
        assert_eq!(res.status, 201, "request {i}: {}", res.json);
    }
    let res = request_upload(&app, &token, "product", "image/jpeg", 100).await;
    assert_eq!(res.status, 429, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "upload_rate_limited");
    // Another person has their own allowance, and the window moves on.
    let (_, other) = buyer(&app).await;
    assert_eq!(
        request_upload(&app, &other, "review", "image/jpeg", 100)
            .await
            .status,
        201
    );
    sqlx::query("UPDATE uploads SET created_at = now() - interval '2 hours'")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        request_upload(&app, &token, "product", "image/jpeg", 100)
            .await
            .status,
        201
    );
}

#[sqlx::test]
async fn purge_job_removes_stale_pending_uploads_and_objects(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    let old = upload(&app, &token, "product", "image/jpeg", jpeg(10, 10)).await;
    let young = upload(&app, &token, "product", "image/jpeg", jpeg(10, 10)).await;
    let ready = upload(&app, &token, "product", "image/jpeg", jpeg(10, 10)).await;
    assert_eq!(finalize(&app, &token, &ready).await.status, 200);
    let failed = upload(
        &app,
        &token,
        "product",
        "image/png",
        b"not an image".to_vec(),
    )
    .await;
    assert_eq!(finalize(&app, &token, &failed).await.status, 422);
    for id in [&old, &ready, &failed] {
        sqlx::query(
            "UPDATE uploads SET created_at = now() - interval '25 hours' WHERE id = $1::uuid",
        )
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    }
    let old_raw: String = sqlx::query_scalar("SELECT raw_key FROM uploads WHERE id = $1::uuid")
        .bind(&old)
        .fetch_one(&app.pool)
        .await
        .unwrap();

    let report = run_due_jobs(&app.state).await;
    assert_eq!(
        report.uploads_purged, 2,
        "the stale pending and the stale failed"
    );
    let left: Vec<String> = sqlx::query_scalar("SELECT id::text FROM uploads ORDER BY id")
        .fetch_all(&app.pool)
        .await
        .unwrap();
    let mut expect = vec![young.clone(), ready.clone()];
    expect.sort();
    assert_eq!(left, expect, "a young pending and a finished upload stay");
    assert!(app.objects.object(Bucket::Private, &old_raw).is_none());
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM job_runs WHERE job = 'purge_uploads'")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(runs, 1);
}

#[sqlx::test]
async fn uploads_are_unavailable_without_a_store(pool: PgPool) {
    use tower::ServiceExt;
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    let mut state = app.state.clone();
    state.objects = Arc::new(NoStore);
    let router = server::build_app(state);
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/uploads")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(axum::body::Body::from(
            json!({ "purpose": "product", "content_type": "image/jpeg", "bytes": 100 }).to_string(),
        ))
        .unwrap();
    let res = router.oneshot(request).await.unwrap();
    assert_eq!(res.status().as_u16(), 503);
    let body = http_body_util::BodyExt::collect(res.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "uploads_unavailable");
}

#[sqlx::test]
async fn a_store_outage_keeps_the_slot_for_a_retry(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let token = seller(&app).await;
    let id = upload(&app, &token, "product", "image/jpeg", jpeg(30, 30)).await;
    app.objects.set_unavailable(true);
    let res = finalize(&app, &token, &id).await;
    assert_eq!(res.status, 503, "{}", res.json);
    assert_eq!(status_of(&app, &id).await, "pending");
    app.objects.set_unavailable(false);
    assert_eq!(finalize(&app, &token, &id).await.status, 200);
}

// ---- Task 6: ownership of photo keys, public URLs, private evidence ----

fn product_with(photos: Value) -> Value {
    json!({
        "name": "Ankara Shirt",
        "description": "Handmade shirt",
        "category": "fashion",
        "condition": "new",
        "photo_keys": photos,
        "weight_grams": 300,
        "base_price_kobo": 1_200_000,
        "variants": [{ "attributes": { "size": "M" }, "stock": 4 }]
    })
}

async fn create_product(app: &TestApp, token: &str, store: Uuid, photos: Value) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/stores/{store}/products"),
        Some(product_with(photos)),
        Some(token),
    )
    .await
}

async fn shop(app: &TestApp) -> common::Listing {
    app.make_active_store_with_product("08033333331", "Kano Fabrics", 10_000, 5)
        .await
}

async fn attached(app: &TestApp, key: &str) -> (Option<String>, Option<Uuid>) {
    sqlx::query_as("SELECT attached_kind, attached_id FROM uploads WHERE key = $1")
        .bind(key)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn cannot_claim_foreign_pending_or_wrong_purpose_keys(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let s = shop(&app).await;
    let (other, _) = app.make_seller("08033333332").await;

    let foreign = app.ready_photo(other, "product").await;
    let wrong_purpose = app.ready_photo(s.seller_id, "review").await;
    let unknown = format!("u/product/{}.jpg", Uuid::new_v4());
    // A slot that was asked for but never finalized has no key at all.
    let pending = upload(&app, &s.seller_token, "product", "image/jpeg", jpeg(30, 30)).await;
    let pending_raw: String = sqlx::query_scalar("SELECT raw_key FROM uploads WHERE id = $1::uuid")
        .bind(&pending)
        .fetch_one(&app.pool)
        .await
        .unwrap();

    let mut bodies = Vec::new();
    for key in [foreign, wrong_purpose, unknown, pending_raw] {
        let res = create_product(&app, &s.seller_token, s.store_id, json!([key])).await;
        assert_eq!(res.status, 422, "{}", res.json);
        assert_eq!(res.json["error"]["code"], "photo_not_yours");
        bodies.push(res.json.to_string());
    }
    assert!(bodies.windows(2).all(|w| w[0] == w[1]), "{bodies:?}");
}

#[sqlx::test]
async fn a_failed_claim_leaves_good_keys_unattached(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let s = shop(&app).await;
    let (other, _) = app.make_seller("08033333332").await;
    let mine = app.ready_photo(s.seller_id, "product").await;
    let theirs = app.ready_photo(other, "product").await;
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM products")
        .fetch_one(&app.pool)
        .await
        .unwrap();

    let res = create_product(&app, &s.seller_token, s.store_id, json!([mine, theirs])).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(attached(&app, &mine).await, (None, None));
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM products")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(before, after, "the product rolled back with its claim");

    // The same key works once the request is right.
    let res = create_product(&app, &s.seller_token, s.store_id, json!([mine])).await;
    assert_eq!(res.status, 201, "{}", res.json);
    let id: Uuid = res.json["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        attached(&app, &mine).await,
        (Some("product".into()), Some(id))
    );
}

#[sqlx::test]
async fn concurrent_claims_of_one_key_attach_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let s = shop(&app).await;
    let key = app.ready_photo(s.seller_id, "product").await;
    let (a, b) = tokio::join!(
        create_product(&app, &s.seller_token, s.store_id, json!([key])),
        create_product(&app, &s.seller_token, s.store_id, json!([key])),
    );
    let mut statuses = [a.status, b.status];
    statuses.sort();
    assert_eq!(statuses, [201, 409], "{} / {}", a.json, b.json);
    let loser = if a.status == 409 { &a } else { &b };
    assert_eq!(loser.json["error"]["code"], "photo_in_use");
    let products: i64 =
        sqlx::query_scalar("SELECT count(*) FROM products WHERE $1 = ANY(photo_keys)")
            .bind(&key)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(products, 1);
}

#[sqlx::test]
async fn a_key_in_use_elsewhere_is_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let s = shop(&app).await;
    let key = app.ready_photo(s.seller_id, "product").await;
    assert_eq!(
        create_product(&app, &s.seller_token, s.store_id, json!([key]))
            .await
            .status,
        201
    );
    let again = create_product(&app, &s.seller_token, s.store_id, json!([key])).await;
    assert_eq!(again.status, 409, "{}", again.json);
    assert_eq!(again.json["error"]["code"], "photo_in_use");
}

#[sqlx::test]
async fn existing_keys_are_not_rechecked_on_patch(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let s = shop(&app).await;
    // A product written before ownership checks existed holds a made-up key.
    sqlx::query("UPDATE products SET photo_keys = ARRAY['photos/old.jpg'] WHERE id = $1")
        .bind(s.product_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let patch = |body: Value| {
        let (app, token, id) = (&app, s.seller_token.clone(), s.product_id);
        async move {
            app.request(
                Method::PATCH,
                &format!("/products/{id}"),
                Some(body),
                Some(&token),
            )
            .await
        }
    };
    let res = patch(json!({ "name": "Renamed" })).await;
    assert_eq!(res.status, 200, "{}", res.json);

    let fresh = app.ready_photo(s.seller_id, "product").await;
    let res = patch(json!({ "photo_keys": ["photos/old.jpg", fresh] })).await;
    assert_eq!(
        res.status, 200,
        "the old key is grandfathered: {}",
        res.json
    );
    assert_eq!(res.json["photo_keys"][0], "photos/old.jpg");

    let res = patch(json!({ "photo_keys": ["photos/old.jpg", "photos/new.jpg"] })).await;
    assert_eq!(res.status, 422, "a new made-up key is checked");
    assert_eq!(res.json["error"]["code"], "photo_not_yours");
}

#[sqlx::test]
async fn store_and_dispute_photos_require_ownership(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let s = shop(&app).await;
    let (other, _) = app.make_seller("08033333332").await;

    // Store logo and banner.
    for (field, purpose) in [("logo_key", "store_logo"), ("banner_key", "store_banner")] {
        let theirs = app.ready_photo(other, purpose).await;
        let res = app
            .request(
                Method::PATCH,
                &format!("/stores/{}", s.store_id),
                Some(json!({ field: theirs })),
                Some(&s.seller_token),
            )
            .await;
        assert_eq!(res.status, 422, "{field}: {}", res.json);
        assert_eq!(res.json["error"]["code"], "photo_not_yours");
        let mine = app.ready_photo(s.seller_id, purpose).await;
        let res = app
            .request(
                Method::PATCH,
                &format!("/stores/{}", s.store_id),
                Some(json!({ field: mine })),
                Some(&s.seller_token),
            )
            .await;
        assert_eq!(res.status, 200, "{field}: {}", res.json);
        assert_eq!(res.json[field], json!(mine));
    }

    // A dispute: someone else's evidence is refused, the buyer's own is taken.
    let o = app.shipped_order("courier", "delivered").await;
    let theirs = app.ready_photo(other, "dispute").await;
    let open = |key: String| {
        let (app, o) = (&app, &o);
        async move {
            app.request(
                Method::POST,
                &format!("/orders/{}/dispute", o.order_id),
                Some(json!({
                    "reason": "damaged",
                    "body": "The zip was torn and the lining was stained when it arrived",
                    "photo_keys": [key]
                })),
                Some(&o.buyer_token),
            )
            .await
        }
    };
    let res = open(theirs).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "photo_not_yours");
    let mine = app.ready_photo(o.buyer_id, "dispute").await;
    let res = open(mine).await;
    assert_eq!(res.status, 201, "{}", res.json);
}

#[sqlx::test]
async fn public_views_carry_urls(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let s = shop(&app).await;
    let cdn = |key: &str| json!(format!("https://cdn.fake.invalid/{key}"));

    // Product detail.
    let res = app
        .request(
            Method::GET,
            &format!("/products/{}", s.product_id),
            None,
            None,
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let key = res.json["photo_keys"][0].as_str().unwrap().to_string();
    assert_eq!(res.json["photo_urls"], json!([cdn(&key)]));

    // Store logo and banner.
    let logo = app.ready_photo(s.seller_id, "store_logo").await;
    let banner = app.ready_photo(s.seller_id, "store_banner").await;
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{}", s.store_id),
            Some(json!({ "logo_key": logo, "banner_key": banner })),
            Some(&s.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let slug = res.json["slug"].as_str().unwrap().to_string();
    let public = app
        .request(Method::GET, &format!("/stores/{slug}"), None, None)
        .await;
    assert_eq!(public.json["logo_url"], cdn(&logo));
    assert_eq!(public.json["banner_url"], cdn(&banner));

    // Cards in browse and in the directory.
    let browse = app.request(Method::GET, "/products", None, None).await;
    assert_eq!(browse.status, 200, "{}", browse.json);
    let card = &browse.json["items"][0];
    assert_eq!(card["photo_url"], cdn(card["photo_key"].as_str().unwrap()));
    let directory = app.request(Method::GET, "/stores", None, None).await;
    assert_eq!(directory.status, 200, "{}", directory.json);
    assert_eq!(directory.json["items"][0]["logo_url"], cdn(&logo));
}

/// An open dispute with a buyer photo; returns (order, dispute id, photo key).
async fn dispute_with_photo(app: &TestApp) -> (common::ShippedOrder, String, String) {
    let o = app.shipped_order("courier", "delivered").await;
    let key = app.ready_photo(o.buyer_id, "dispute").await;
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/dispute", o.order_id),
            Some(json!({
                "reason": "damaged",
                "body": "The zip was torn and the lining was stained when it arrived",
                "photo_keys": [key]
            })),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    let id = res.json["id"].as_str().unwrap().to_string();
    (o, id, key)
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
async fn dispute_photos_are_never_public(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (o, id, key) = dispute_with_photo(&app).await;
    let view = app
        .request(
            Method::GET,
            &format!("/disputes/{id}"),
            None,
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(view.status, 200, "{}", view.json);
    let text = view.json.to_string();
    assert!(
        !text.contains("photo_urls") && !text.contains("http"),
        "{text}"
    );
    assert_eq!(view.json["buyer_statement"]["photo_keys"], json!([key]));
    assert!(app.objects.object(Bucket::Private, &key).is_some());
    assert!(app.objects.object(Bucket::Public, &key).is_none());
}

#[sqlx::test]
async fn dispute_photo_url_is_short_lived_and_scoped(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (o, id, key) = dispute_with_photo(&app).await;
    for token in [&o.buyer_token, &o.seller_token] {
        let res = dispute_photo(&app, token, &id, &key).await;
        assert_eq!(res.status, 200, "{}", res.json);
        let url = res.json["url"].as_str().unwrap();
        assert!(url.contains("/private/"), "{url}");
        assert!(url.ends_with("ttl=60"), "{url}");
        let at = time::OffsetDateTime::parse(
            res.json["expires_at"].as_str().unwrap(),
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
        let left = (at - time::OffsetDateTime::now_utc()).whole_seconds();
        assert!((50..=60).contains(&left), "{left}");
    }
    // A key that is not one of this dispute's photos is not found, even a real one.
    let elsewhere = app.ready_photo(o.buyer_id, "dispute").await;
    assert_eq!(
        dispute_photo(&app, &o.buyer_token, &id, &elsewhere)
            .await
            .status,
        404
    );
    // Neither a buyer's nor a seller's read is an admin read.
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'admin.dispute_photo_viewed'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(audited, 0);
}

#[sqlx::test]
async fn stranger_gets_404_for_dispute_photo(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, id, key) = dispute_with_photo(&app).await;
    let (_, stranger) = app.sign_up_verified("08044444441").await;
    assert_eq!(dispute_photo(&app, &stranger, &id, &key).await.status, 404);
    let no_such = Uuid::new_v4().to_string();
    assert_eq!(
        dispute_photo(&app, &stranger, &no_such, &key).await.status,
        404
    );
}

#[sqlx::test]
async fn support_cannot_sign_closed_dispute_photo(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (o, id, key) = dispute_with_photo(&app).await;
    let support = app.make_support().await;
    let owner = app.owner_token().await;

    // Open: both staff roles may.
    assert_eq!(dispute_photo(&app, &support, &id, &key).await.status, 200);
    // Withdrawn (closed): Support is refused, the Owner is not.
    let res = app
        .request(
            Method::POST,
            &format!("/disputes/{id}/withdraw"),
            Some(json!({ "confirm": true })),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let res = dispute_photo(&app, &support, &id, &key).await;
    assert_eq!(res.status, 403, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "dispute_closed");
    assert_eq!(dispute_photo(&app, &owner, &id, &key).await.status, 200);
}

#[sqlx::test]
async fn staff_reads_are_audited_with_the_dispute_id_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, id, key) = dispute_with_photo(&app).await;
    let owner = app.owner_token().await;
    assert_eq!(dispute_photo(&app, &owner, &id, &key).await.status, 200);
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT detail FROM audit_log WHERE action = 'admin.dispute_photo_viewed' \
         AND subject_id = $1::uuid",
    )
    .bind(&id)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0], json!({ "dispute_id": id }));
    assert!(!rows[0].to_string().contains(&key));
}
