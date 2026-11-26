//! Part 8: data rights and retention. Deletion with a cooling-off and its blockers, the
//! erasure of every table in the inventory, the export, and the retention windows.

mod common;

use axum::http::Method;
use common::{TEST_PASSWORD, TestApp, TestResponse};
use serde_json::{Value, json};
use server::{escrow::jobs::run_due_jobs, privacy::erase::INVENTORY};
use sqlx::PgPool;
use uuid::Uuid;

/// recipient name, phone, address line, landmark, postcode.
type AddressColumns = (Vec<u8>, Vec<u8>, Vec<u8>, Option<Vec<u8>>, Option<String>);

const PHONE: &str = "08031110001";

async fn ask(app: &TestApp, token: &str, method: Method, path: &str, body: Value) -> TestResponse {
    app.request(method, path, Some(body), Some(token)).await
}

async fn get(app: &TestApp, token: &str, path: &str) -> TestResponse {
    app.request(Method::GET, path, None, Some(token)).await
}

async fn request_deletion(app: &TestApp, token: &str) -> TestResponse {
    ask(
        app,
        token,
        Method::POST,
        "/me/deletion",
        json!({ "password": TEST_PASSWORD }),
    )
    .await
}

async fn make_due(app: &TestApp) {
    sqlx::query("UPDATE deletion_requests SET due_at = now() - interval '1 minute'")
        .execute(&app.pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn deletion_starts_a_cooling_off_that_gates_the_account_and_can_be_cancelled(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_id, token) = app.sign_up_verified(PHONE).await;
    // A second phone signed in.
    let second = app.sign_in(PHONE, TEST_PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();

    // A wrong password changes nothing.
    let wrong = ask(
        &app,
        &token,
        Method::POST,
        "/me/deletion",
        json!({"password": "nope-nope-1"}),
    )
    .await;
    assert_eq!(wrong.status, 401, "{}", wrong.json);
    assert_eq!(
        get(&app, &token, "/me/deletion").await.json["state"],
        "none"
    );

    let res = request_deletion(&app, &token).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["state"], "pending");
    let days: f64 = sqlx::query_scalar(
        "SELECT (extract(epoch FROM due_at - requested_at) / 86400)::float8 FROM deletion_requests",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!((days - 30.0).abs() < 0.01, "{days}");
    // Asking again is the same request.
    assert_eq!(
        request_deletion(&app, &token).await.json["due_at"],
        res.json["due_at"]
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM deletion_requests")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        1
    );

    // The other phone is signed out; this one is gated.
    assert_eq!(get(&app, &second, "/me").await.status, 401);
    let gated = get(&app, &token, "/me").await;
    assert_eq!(gated.status, 403);
    assert_eq!(gated.json["error"]["code"], "deletion_pending");
    assert_eq!(get(&app, &token, "/me/deletion").await.status, 200);
    assert_eq!(get(&app, &token, "/me/export").await.status, 200);
    assert_eq!(get(&app, &token, "/cart").await.status, 403);

    // Changing your mind restores everything.
    let res = app
        .request(Method::DELETE, "/me/deletion", None, Some(&token))
        .await;
    assert_eq!(res.status, 204);
    assert_eq!(get(&app, &token, "/me").await.status, 200);
    assert_eq!(
        get(&app, &token, "/me/deletion").await.json["state"],
        "none"
    );
    // And the purge does nothing for it.
    make_due(&app).await;
    run_due_jobs(&app.state).await;
    assert_eq!(get(&app, &token, "/me").await.status, 200);
}

#[sqlx::test]
async fn deletion_is_refused_while_something_is_open(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let order = app.shipped_order("self", "delivered").await;

    // A buyer with an unfinished order.
    let res = request_deletion(&app, &order.buyer_token).await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "deletion_blocked");
    assert_eq!(res.json["error"]["reasons"], json!(["open_order"]));
    // Nothing changed: still signed in and not pending.
    assert_eq!(get(&app, &order.buyer_token, "/me").await.status, 200);

    // A store owner: the store and its order.
    let res = request_deletion(&app, &order.seller_token).await;
    assert_eq!(res.status, 409);
    let reasons = res.json["error"]["reasons"].as_array().unwrap().clone();
    assert!(reasons.contains(&json!("owns_store")), "{reasons:?}");
    assert!(reasons.contains(&json!("open_order")), "{reasons:?}");

    // A finished order is no blocker; an open dispute is.
    sqlx::query("UPDATE orders SET status = 'completed', completed_at = now()")
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO disputes (order_id, buyer_id, store_id, reason, status, seller_respond_by) \
         VALUES ($1, $2, $3, 'not_as_described', 'awaiting_seller', now() + interval '3 days')",
    )
    .bind(order.order_id)
    .bind(order.buyer_id)
    .bind(order.store_id)
    .execute(&app.pool)
    .await
    .unwrap();
    let res = request_deletion(&app, &order.buyer_token).await;
    assert_eq!(res.json["error"]["reasons"], json!(["open_dispute"]));
    sqlx::query("UPDATE disputes SET status = 'withdrawn', withdrawn_at = now()")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(request_deletion(&app, &order.buyer_token).await.status, 200);

    // An owner who closes the store may delete, once nothing is owed.
    let res = app
        .request(
            Method::DELETE,
            &format!("/stores/{}", order.store_id),
            None,
            Some(&order.seller_token),
        )
        .await;
    assert!(res.status < 300, "{}", res.json);
    let res = request_deletion(&app, &order.seller_token).await;
    assert_eq!(res.status, 200, "{}", res.json);
}

#[sqlx::test]
async fn money_still_moving_blocks_both_a_seller_and_a_buyer(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let order = app.shipped_order("self", "delivered").await;
    sqlx::query("UPDATE orders SET status = 'completed'")
        .execute(&app.pool)
        .await
        .unwrap();
    // A payout on its way to the seller: blocks the seller once the store is closed.
    sqlx::query(
        "INSERT INTO payouts (order_id, reference, provider, amount_kobo, status) \
         VALUES ($1, 'PO1', 'fake', 900000, 'scheduled')",
    )
    .bind(order.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    let closed = app
        .request(
            Method::DELETE,
            &format!("/stores/{}", order.store_id),
            None,
            Some(&order.seller_token),
        )
        .await;
    assert!(closed.status < 300, "{}", closed.json);
    let res = request_deletion(&app, &order.seller_token).await;
    assert_eq!(
        res.json["error"]["reasons"],
        json!(["money_in_flight"]),
        "{}",
        res.json
    );
    sqlx::query("UPDATE payouts SET status = 'failed'")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        request_deletion(&app, &order.seller_token).await.status,
        200
    );

    // A refund still processing blocks the buyer.
    sqlx::query(
        "INSERT INTO refunds (order_id, reference, provider, amount_kobo, reason, status) \
         VALUES ($1, 'RF1', 'fake', 100000, 'test', 'processing')",
    )
    .bind(order.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    let res = request_deletion(&app, &order.buyer_token).await;
    assert_eq!(
        res.json["error"]["reasons"],
        json!(["money_in_flight"]),
        "{}",
        res.json
    );
    sqlx::query("UPDATE refunds SET status = 'failed'")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(request_deletion(&app, &order.buyer_token).await.status, 200);
}

/// A buyer with data in every table the purge touches.
async fn full_buyer(app: &TestApp) -> (common::ShippedOrder, Uuid) {
    let order = app.shipped_order("courier", "delivered").await;
    sqlx::query("UPDATE orders SET status = 'completed', completed_at = now()")
        .execute(&app.pool)
        .await
        .unwrap();
    let t = &order.buyer_token;
    let device = ask(
        app,
        t,
        Method::PUT,
        "/me/push-device",
        json!({"token": "device-token-1", "platform": "android"}),
    )
    .await;
    assert!(device.status < 300, "{}", device.json);
    // A conversation and a message from the buyer.
    let product: Uuid = sqlx::query_scalar("SELECT id FROM products WHERE store_id = $1")
        .bind(order.store_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let chat = ask(
        app,
        t,
        Method::POST,
        "/chats",
        json!({ "product_id": product }),
    )
    .await;
    assert_eq!(chat.status, 200, "{}", chat.json);
    let chat_id: Uuid = chat.json["id"].as_str().unwrap().parse().unwrap();
    let sent = ask(
        app,
        t,
        Method::POST,
        &format!("/chats/{chat_id}/messages"),
        json!({"client_message_id": Uuid::new_v4(), "body": "Is it still available?"}),
    )
    .await;
    assert_eq!(sent.status, 200, "{}", sent.json);
    // A photo, a notice, a cart line and a review with a photo.
    let key = app.ready_photo(order.buyer_id, "review").await;
    sqlx::query(
        "INSERT INTO reviews (order_id, store_id, buyer_id, stars, body, photo_keys) \
         VALUES ($1, $2, $3, 5, 'Lovely', ARRAY[$4])",
    )
    .bind(order.order_id)
    .bind(order.store_id)
    .bind(order.buyer_id)
    .bind(&key)
    .execute(&app.pool)
    .await
    .unwrap();
    let mut conn = app.pool.acquire().await.unwrap();
    server::notify::service::enqueue(
        &mut conn,
        server::notify::service::Notice::user(
            order.buyer_id,
            server::notify::kinds::Kind::OrderDelivered,
            format!("test:{}", Uuid::new_v4()),
        ),
    )
    .await
    .unwrap();
    (order, chat_id)
}

#[sqlx::test]
async fn erasure_clears_or_unlinks_everything_and_frees_the_number(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (order, chat_id) = full_buyer(&app).await;
    let phone: String = {
        let enc: Vec<u8> = sqlx::query_scalar("SELECT phone_enc FROM users WHERE id = $1")
            .bind(order.buyer_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
        app.state.cipher.decrypt(&enc).unwrap()
    };
    assert_eq!(request_deletion(&app, &order.buyer_token).await.status, 200);

    // Not before the date.
    run_due_jobs(&app.state).await;
    assert!(
        sqlx::query_scalar::<_, Option<time::OffsetDateTime>>(
            "SELECT erased_at FROM users WHERE id = $1"
        )
        .bind(order.buyer_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
        .is_none()
    );

    make_due(&app).await;
    let outcome = server::privacy::erase::erase_user(&app.state, order.buyer_id).await;
    assert_eq!(outcome.unwrap(), server::privacy::erase::Outcome::Erased);

    let count = |sql: &'static str| {
        let pool = app.pool.clone();
        let id = order.buyer_id;
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    for sql in [
        "SELECT count(*) FROM sessions WHERE user_id = $1",
        "SELECT count(*) FROM push_devices WHERE user_id = $1",
        "SELECT count(*) FROM notifications WHERE user_id = $1",
        "SELECT count(*) FROM cart_items WHERE user_id = $1",
        "SELECT count(*) FROM uploads WHERE user_id = $1",
        "SELECT count(*) FROM otp_codes WHERE user_id = $1",
    ] {
        assert_eq!(count(sql).await, 0, "{sql}");
    }
    // Addresses stay as empty rows, because an order's delivery option points at one.
    let kept: Vec<AddressColumns> = sqlx::query_as(
        "SELECT recipient_name_enc, phone_enc, address_line_enc, landmark_enc, postcode \
         FROM buyer_addresses WHERE user_id = $1",
    )
    .bind(order.buyer_id)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert!(!kept.is_empty());
    for (n, p, a, l, post) in kept {
        assert!(n.is_empty() && p.is_empty() && a.is_empty() && l.is_none() && post.is_none());
    }
    // The shell.
    let (name, phone_enc, hash, deleted, erased): (String, Vec<u8>, String, bool, bool) =
        sqlx::query_as(
            "SELECT display_name, phone_enc, password_hash, deleted_at IS NOT NULL, \
                    erased_at IS NOT NULL FROM users WHERE id = $1",
        )
        .bind(order.buyer_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(name, "Deleted user");
    assert!(phone_enc.is_empty() && hash.is_empty() && deleted && erased);
    // Orders stay for the record, without the address or the handover code.
    let (status, address, code, purged): (String, Option<Vec<u8>>, Option<Vec<u8>>, bool) =
        sqlx::query_as(
            "SELECT status::text, delivery_address_enc, handover_code_enc, \
                    delivery_address_purged_at IS NOT NULL FROM orders WHERE id = $1",
        )
        .bind(order.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(status, "completed");
    assert!(address.is_none() && code.is_none() && purged);
    // The review stays without its photo; the chat stays for the other side.
    let photos: Vec<String> =
        sqlx::query_scalar("SELECT unnest(photo_keys) FROM reviews WHERE buyer_id = $1")
            .bind(order.buyer_id)
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert!(photos.is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM chat_messages WHERE conversation_id = $1"
        )
        .bind(chat_id)
        .fetch_one(&app.pool)
        .await
        .unwrap(),
        1
    );
    // The seller still sees the conversation, and the author is "Deleted user".
    let list = get(&app, &order.seller_token, "/chats").await;
    assert_eq!(list.status, 200, "{}", list.json);
    assert!(!list.json.to_string().contains("Wire"), "{}", list.json);
    // Photos were deleted from the store.
    assert!(
        app.objects
            .keys(server::uploads::store::Bucket::Public)
            .iter()
            .all(|k| !k.contains("/review/"))
    );

    // Signing in no longer works, and the number can sign up again.
    assert_eq!(app.sign_in(&phone, TEST_PASSWORD).await.status, 401);
    let again = app.sign_up(&phone, TEST_PASSWORD).await;
    assert_eq!(again.status, 201, "{}", again.json);

    // The audit trail names ids and codes only.
    let rows: Vec<Value> =
        sqlx::query_scalar("SELECT detail FROM audit_log WHERE action LIKE 'privacy.%'")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert!(rows.len() >= 2);
    for d in rows {
        assert_eq!(d, json!({}));
    }
    // Running it again is a no-op.
    run_due_jobs(&app.state).await;
}

#[sqlx::test]
async fn an_erasure_waits_while_a_blocker_has_appeared(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (order, _) = full_buyer(&app).await;
    assert_eq!(request_deletion(&app, &order.buyer_token).await.status, 200);
    // During the cooling-off something opens (here, by the other side's action).
    sqlx::query("UPDATE orders SET status = 'paid'")
        .execute(&app.pool)
        .await
        .unwrap();
    make_due(&app).await;
    run_due_jobs(&app.state).await;
    let (erased, deferred): (bool, i32) = sqlx::query_as(
        "SELECT u.erased_at IS NOT NULL, d.deferred_count FROM users u \
         JOIN deletion_requests d ON d.user_id = u.id WHERE u.id = $1",
    )
    .bind(order.buyer_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(!erased);
    assert_eq!(deferred, 1);
    // Once it closes, the next run erases.
    sqlx::query("UPDATE orders SET status = 'completed'")
        .execute(&app.pool)
        .await
        .unwrap();
    run_due_jobs(&app.state).await;
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT erased_at IS NOT NULL FROM users WHERE id = $1")
            .bind(order.buyer_id)
            .fetch_one(&app.pool)
            .await
            .unwrap()
    );
}

/// Every table that points at a user, or holds a column that looks personal, must have a
/// decided fate in `INVENTORY`. Adding one without deciding breaks this test.
#[sqlx::test]
async fn every_table_with_personal_data_has_a_fate(pool: PgPool) {
    let referencing: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT c.conrelid::regclass::text FROM pg_constraint c \
         WHERE c.contype = 'f' AND c.confrelid = 'users'::regclass",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let personal: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT table_name::text FROM information_schema.columns \
         WHERE table_schema = 'public' \
           AND (column_name ~ '_enc$' OR column_name ~ 'phone' OR column_name = 'display_name' \
                OR column_name = 'body')",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let mut need: Vec<String> = referencing.into_iter().chain(personal).collect();
    need.sort();
    need.dedup();
    // Tables that hold nothing about a person despite a matching column name.
    let exempt = ["platform_settings"];
    let missing: Vec<&String> = need
        .iter()
        .filter(|t| !exempt.contains(&t.as_str()) && !INVENTORY.iter().any(|(n, _)| n == t))
        .collect();
    assert!(
        missing.is_empty(),
        "decide what erasure does with these tables in privacy::erase::INVENTORY: {missing:?}"
    );
    // And the inventory names no table that is gone.
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = 'public'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for (name, _) in INVENTORY {
        assert!(tables.iter().any(|t| t == name), "{name} is not a table");
    }
}

#[sqlx::test]
async fn an_export_holds_the_persons_own_data_and_only_they_can_take_it(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (order, _chat) = full_buyer(&app).await;
    let other = app.sign_up_verified("08031110002").await;
    let t = &order.buyer_token;

    let wrong = ask(
        &app,
        t,
        Method::POST,
        "/me/export",
        json!({"password": "nope-nope-1"}),
    )
    .await;
    assert_eq!(wrong.status, 401);
    let res = ask(
        &app,
        t,
        Method::POST,
        "/me/export",
        json!({"password": TEST_PASSWORD}),
    )
    .await;
    assert_eq!(res.status, 202, "{}", res.json);
    assert_eq!(res.json["state"], "pending");
    assert_eq!(get(&app, t, "/me/export/file").await.status, 404);
    // Asking again while pending is the same request; a new one needs a day.
    let again = ask(
        &app,
        t,
        Method::POST,
        "/me/export",
        json!({"password": TEST_PASSWORD}),
    )
    .await;
    assert_eq!(again.status, 202);

    run_due_jobs(&app.state).await;
    let status = get(&app, t, "/me/export").await;
    assert_eq!(status.json["state"], "ready", "{}", status.json);
    assert!(status.json["expires_at"].is_string());

    let file = get(&app, t, "/me/export/file").await;
    assert_eq!(file.status, 200);
    assert_eq!(file.headers["cache-control"], "no-store");
    let doc: Value = file.json.clone();
    assert_eq!(doc["profile"]["is_seller"], false);
    assert_eq!(doc["orders"].as_array().unwrap().len(), 1);
    assert_eq!(doc["chats"][0]["text"], "Is it still available?");
    assert_eq!(doc["reviews"][0]["text"], "Lovely");
    // Only their own: nothing from anybody else, and no NIN or password.
    let text = doc.to_string();
    assert!(!text.contains("password"), "{text}");
    // The file is encrypted at rest.
    let stored: Vec<u8> = sqlx::query_scalar("SELECT body_enc FROM data_exports")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&stored).contains("Is it still available?"));
    // Another person gets nothing.
    assert_eq!(get(&app, &other.1, "/me/export/file").await.status, 404);
    assert_eq!(
        get(&app, &other.1, "/me/export").await.json["state"],
        "none"
    );

    // After its week it is gone, and one request a day holds.
    sqlx::query("UPDATE data_exports SET expires_at = now() - interval '1 minute'")
        .execute(&app.pool)
        .await
        .unwrap();
    run_due_jobs(&app.state).await;
    assert_eq!(get(&app, t, "/me/export").await.json["state"], "expired");
    assert_eq!(get(&app, t, "/me/export/file").await.status, 404);
    let limited = ask(
        &app,
        t,
        Method::POST,
        "/me/export",
        json!({"password": TEST_PASSWORD}),
    )
    .await;
    assert_eq!(limited.status, 429, "{}", limited.json);
    sqlx::query("UPDATE data_exports SET requested_at = now() - interval '25 hours'")
        .execute(&app.pool)
        .await
        .unwrap();
    let ok = ask(
        &app,
        t,
        Method::POST,
        "/me/export",
        json!({"password": TEST_PASSWORD}),
    )
    .await;
    assert_eq!(ok.status, 202, "{}", ok.json);
}

#[sqlx::test]
async fn retention_windows_remove_old_data_and_spare_the_young_and_the_disputed(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (order, chat_id) = full_buyer(&app).await;
    // An old message (13 months) and a young one.
    sqlx::query(
        "UPDATE chat_messages SET created_at = now() - interval '13 months' WHERE conversation_id = $1",
    )
    .bind(chat_id)
    .execute(&app.pool)
    .await
    .unwrap();
    let young = ask(
        &app,
        &order.buyer_token,
        Method::POST,
        &format!("/chats/{chat_id}/messages"),
        json!({"client_message_id": Uuid::new_v4(), "body": "Hello again"}),
    )
    .await;
    assert_eq!(young.status, 200);
    sqlx::query("UPDATE notifications SET created_at = now() - interval '7 months'")
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE push_devices SET registered_at = now() - interval '100 days'")
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE orders SET created_at = now() - interval '14 months', completed_at = now() - interval '13 months'")
        .execute(&app.pool)
        .await
        .unwrap();
    run_due_jobs(&app.state).await;

    let bodies: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chat_messages WHERE conversation_id = $1")
            .bind(chat_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(bodies, 1, "only the young message stays");
    for sql in [
        "SELECT count(*) FROM notifications WHERE created_at < now() - interval '6 months'",
        "SELECT count(*) FROM push_devices",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(&app.pool)
                .await
                .unwrap(),
            0,
            "{sql}"
        );
    }
    let purged: bool = sqlx::query_scalar(
        "SELECT delivery_address_enc IS NULL AND delivery_address_purged_at IS NOT NULL FROM orders WHERE id = $1",
    )
    .bind(order.order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(purged, "a finished order's address goes after its window");

    // An open dispute on the order keeps its (order) chat past the window.
    let order_chat = ask(
        &app,
        &order.buyer_token,
        Method::POST,
        "/chats",
        json!({ "order_id": order.order_id }),
    )
    .await;
    assert_eq!(order_chat.status, 200, "{}", order_chat.json);
    let oc: Uuid = order_chat.json["id"].as_str().unwrap().parse().unwrap();
    ask(
        &app,
        &order.buyer_token,
        Method::POST,
        &format!("/chats/{oc}/messages"),
        json!({"client_message_id": Uuid::new_v4(), "body": "About my order"}),
    )
    .await;
    sqlx::query("UPDATE chat_messages SET created_at = now() - interval '13 months' WHERE conversation_id = $1")
        .bind(oc)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO disputes (order_id, buyer_id, store_id, reason, status, seller_respond_by) \
         VALUES ($1, $2, $3, 'not_as_described', 'awaiting_decision', now() + interval '3 days')",
    )
    .bind(order.order_id)
    .bind(order.buyer_id)
    .bind(order.store_id)
    .execute(&app.pool)
    .await
    .unwrap();
    run_due_jobs(&app.state).await;
    let kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chat_messages WHERE conversation_id = $1")
            .bind(oc)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(kept, 1, "an open dispute keeps the chat");
    sqlx::query("UPDATE disputes SET status = 'withdrawn', withdrawn_at = now()")
        .execute(&app.pool)
        .await
        .unwrap();
    run_due_jobs(&app.state).await;
    let kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chat_messages WHERE conversation_id = $1")
            .bind(oc)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(kept, 0, "and once it closes the window applies");
}

#[sqlx::test]
async fn the_owner_sets_the_windows_within_their_bounds(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let (_id, user) = app.sign_up_verified(PHONE).await;
    assert_eq!(get(&app, &user, "/admin/retention").await.status, 403);
    let now = get(&app, &owner, "/admin/retention").await;
    assert_eq!(now.status, 200, "{}", now.json);
    assert_eq!(now.json["chat_retention_months"], 12);
    assert_eq!(now.json["records_retention_years"], 6);
    assert_eq!(now.json["deletion_cooling_off_days"], 30);
    let res = ask(
        &app,
        &owner,
        Method::PATCH,
        "/admin/retention",
        json!({"chat_retention_months": 6}),
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["chat_retention_months"], 6);
    for bad in [
        json!({"chat_retention_months": 0}),
        json!({"deletion_cooling_off_days": 3}),
        json!({"push_device_stale_days": 9999}),
        json!({"nonsense": 1}),
    ] {
        let res = ask(&app, &owner, Method::PATCH, "/admin/retention", bad).await;
        assert_eq!(res.status, 422, "{}", res.json);
    }
    // A new window applies to the next request.
    ask(
        &app,
        &owner,
        Method::PATCH,
        "/admin/retention",
        json!({"deletion_cooling_off_days": 7}),
    )
    .await;
    let res = request_deletion(&app, &user).await;
    assert_eq!(res.status, 200);
    let days: f64 = sqlx::query_scalar(
        "SELECT (extract(epoch FROM due_at - requested_at) / 86400)::float8 FROM deletion_requests",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!((days - 7.0).abs() < 0.01);
}
