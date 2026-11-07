//! Opening, viewing and withdrawing a dispute (plan 5, task 2).

mod common;

use axum::http::Method;
use common::{ShippedOrder, TestApp, TestResponse};
use serde_json::{Value, json};
use server::{
    disputes::service::SELLER_RESPONSE_HOURS,
    escrow::jobs::run_due_jobs,
    limits::{MAX_DISPUTE_CHARS, MAX_DISPUTE_PHOTOS},
};
use sqlx::PgPool;
use uuid::Uuid;

const COMPLAINT: &str = "The zip was torn and the lining was stained when it arrived";

fn report(body: &str) -> Value {
    json!({ "reason": "damaged", "body": body, "photo_keys": ["evidence/1.jpg"] })
}

async fn open(app: &TestApp, o: &ShippedOrder, body: Value) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/orders/{}/dispute", o.order_id),
        Some(body),
        Some(&o.buyer_token),
    )
    .await
}

async fn withdraw(app: &TestApp, dispute: &str, token: &str, body: Value) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/disputes/{dispute}/withdraw"),
        Some(body),
        Some(token),
    )
    .await
}

async fn make_due(app: &TestApp, order: Uuid) {
    sqlx::query("UPDATE orders SET release_at = now() - interval '1 hour' WHERE id = $1")
        .bind(order)
        .execute(&app.pool)
        .await
        .unwrap();
}

async fn payouts(app: &TestApp, order: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM payouts WHERE order_id = $1")
        .bind(order)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn hold(app: &TestApp, order: Uuid) -> (bool, Option<String>) {
    sqlx::query_as("SELECT held_at IS NOT NULL, hold_reason FROM orders WHERE id = $1")
        .bind(order)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn buyer_opens_dispute_and_order_is_held(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    let res = open(&app, &o, report(COMPLAINT)).await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["status"], "awaiting_seller");
    assert_eq!(res.json["reason"], "damaged");
    assert_eq!(res.json["buyer_statement"]["body"], COMPLAINT);
    assert_eq!(
        res.json["buyer_statement"]["photo_keys"],
        json!(["evidence/1.jpg"])
    );
    assert!(res.json["seller_statement"].is_null());
    assert_eq!(hold(&app, o.order_id).await, (true, Some("dispute".into())));

    let id = res.json["id"].as_str().unwrap();
    let hours: f64 = sqlx::query_scalar(
        "SELECT (extract(epoch FROM seller_respond_by - opened_at) / 3600)::float8 \
         FROM disputes WHERE id = $1::uuid",
    )
    .bind(id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(hours, SELLER_RESPONSE_HOURS as f64);

    let audit: Value = sqlx::query_scalar(
        "SELECT detail FROM audit_log WHERE action = 'dispute.opened' AND subject_id = $1::uuid",
    )
    .bind(id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        audit,
        json!({ "reason": "damaged", "prior_status": "delivered" })
    );

    for token in [&o.buyer_token, &o.seller_token] {
        let order = app
            .request(
                Method::GET,
                &format!("/orders/{}", o.order_id),
                None,
                Some(token),
            )
            .await;
        assert_eq!(order.status, 200);
        assert_eq!(order.json["dispute"]["id"], id);
        assert_eq!(order.json["dispute"]["status"], "awaiting_seller");
    }
}

#[sqlx::test]
async fn only_shipped_unreleased_orders_can_be_disputed(pool: PgPool) {
    let app = TestApp::new(pool).await;
    for status in ["picked_up", "out_for_delivery", "delivered"] {
        let o = app.shipped_order("courier", status).await;
        assert_eq!(
            open(&app, &o, report(COMPLAINT)).await.status,
            201,
            "{status}"
        );
    }
    let o = app.shipped_order("courier", "delivered").await;
    let set = |s: &'static str| {
        let pool = app.pool.clone();
        async move {
            sqlx::query(&format!("UPDATE orders SET status = '{s}' WHERE id = $1"))
                .bind(o.order_id)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    set("paid").await;
    let res = open(&app, &o, report(COMPLAINT)).await;
    assert_eq!(
        (res.status, res.json["error"]["code"].as_str()),
        (409, Some("cancel_instead"))
    );
    set("completed").await;
    let res = open(&app, &o, report(COMPLAINT)).await;
    assert_eq!(
        (res.status, res.json["error"]["code"].as_str()),
        (409, Some("already_released"))
    );
    set("refunded").await;
    let res = open(&app, &o, report(COMPLAINT)).await;
    assert_eq!(
        (res.status, res.json["error"]["code"].as_str()),
        (409, Some("invalid_order_state"))
    );

    // A delivered order that already has a payout is released too.
    let released = app.shipped_order("courier", "delivered").await;
    make_due(&app, released.order_id).await;
    run_due_jobs(&app.state).await;
    assert_eq!(payouts(&app, released.order_id).await, 1);
    let res = open(&app, &released, report(COMPLAINT)).await;
    assert_eq!(
        (res.status, res.json["error"]["code"].as_str()),
        (409, Some("already_released"))
    );

    // The seller and a stranger get 404.
    let fresh = app.shipped_order("courier", "delivered").await;
    let (_, stranger) = app.sign_up_verified("08099999991").await;
    for token in [&fresh.seller_token, &stranger] {
        let res = app
            .request(
                Method::POST,
                &format!("/orders/{}/dispute", fresh.order_id),
                Some(report(COMPLAINT)),
                Some(token),
            )
            .await;
        assert_eq!(res.status, 404);
    }
}

#[sqlx::test]
async fn one_report_per_order(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    let first = open(&app, &o, report(COMPLAINT)).await;
    assert_eq!(first.status, 201);
    let again = open(&app, &o, report(COMPLAINT)).await;
    assert_eq!(
        (again.status, again.json["error"]["code"].as_str()),
        (409, Some("dispute_exists"))
    );

    let id = first.json["id"].as_str().unwrap();
    let w = withdraw(&app, id, &o.buyer_token, json!({ "confirm": true })).await;
    assert_eq!(w.status, 200, "{}", w.json);
    let after = open(&app, &o, report(COMPLAINT)).await;
    assert_eq!(
        (after.status, after.json["error"]["code"].as_str()),
        (409, Some("dispute_exists"))
    );
}

#[sqlx::test]
async fn statement_limits(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    let long = "a".repeat(MAX_DISPUTE_CHARS + 1);
    let photos: Vec<String> = (0..=MAX_DISPUTE_PHOTOS)
        .map(|i| format!("p{i}.jpg"))
        .collect();
    let bad = [
        report(&long),
        report("   "),
        report(""),
        json!({ "reason": "damaged", "body": COMPLAINT, "photo_keys": photos }),
        json!({ "reason": "damaged", "body": COMPLAINT, "photo_keys": [" "] }),
        json!({ "reason": "broken", "body": COMPLAINT, "photo_keys": [] }),
    ];
    for body in bad {
        let res = open(&app, &o, body.clone()).await;
        assert_eq!(res.status, 422, "{body}: {}", res.json);
    }
    assert_eq!(hold(&app, o.order_id).await, (false, None));
    // The limits themselves are fine.
    let exact = "a".repeat(MAX_DISPUTE_CHARS);
    assert_eq!(open(&app, &o, report(&exact)).await.status, 201);
}

#[sqlx::test]
async fn statement_with_phone_number_is_refused_without_strike(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    let res = open(
        &app,
        &o,
        report("Call me on 08031234567 and I will explain"),
    )
    .await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json["error"]["code"], "statement_refused");
    assert_eq!(
        res.json["error"]["message"],
        "Keep contact details and payments inside the app. Remove them and try again."
    );
    let strikes: i64 = sqlx::query_scalar("SELECT count(*) FROM moderation_strikes")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(strikes, 0);
    assert_eq!(app.tone.calls(), 0);
    assert_eq!(hold(&app, o.order_id).await, (false, None));
}

#[sqlx::test]
async fn abusive_statement_gets_the_abuse_copy(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    let res = open(&app, &o, report("You are a whore and a thief")).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "statement_refused");
    assert_eq!(
        res.json["error"]["message"],
        "This breaks the market's rules. Describe the problem without insults."
    );
    assert_eq!(app.tone.calls(), 0);
}

#[sqlx::test]
async fn statement_is_encrypted(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    let res = open(&app, &o, report(COMPLAINT)).await;
    assert_eq!(res.status, 201);
    for table in ["disputes", "dispute_statements", "audit_log", "orders"] {
        let rows: Vec<String> =
            sqlx::query_scalar(&format!("SELECT to_jsonb(t)::text FROM {table} t"))
                .fetch_all(&app.pool)
                .await
                .unwrap();
        assert!(!rows.is_empty());
        for raw in rows {
            assert!(!raw.contains("torn"), "plaintext in {table}");
        }
    }
}

#[sqlx::test]
async fn held_dispute_stops_release(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    make_due(&app, o.order_id).await;
    assert_eq!(open(&app, &o, report(COMPLAINT)).await.status, 201);
    run_due_jobs(&app.state).await;
    assert_eq!(payouts(&app, o.order_id).await, 0);
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/received", o.order_id),
            Some(json!({ "confirm": true })),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(
        (res.status, res.json["error"]["code"].as_str()),
        (409, Some("order_held"))
    );
    assert_eq!(payouts(&app, o.order_id).await, 0);
}

#[sqlx::test]
async fn dispute_races_auto_release(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (mut disputed, mut released) = (0, 0);
    for i in 0..6 {
        let o = app.shipped_order("courier", "delivered").await;
        make_due(&app, o.order_id).await;
        // Alternate who is polled first, so each side gets to win.
        let (res, _) = if i % 2 == 0 {
            tokio::join!(open(&app, &o, report(COMPLAINT)), run_due_jobs(&app.state))
        } else {
            let (job, res) =
                tokio::join!(run_due_jobs(&app.state), open(&app, &o, report(COMPLAINT)));
            (res, job)
        };
        let paid = payouts(&app, o.order_id).await;
        match res.status {
            201 => {
                disputed += 1;
                assert_eq!(paid, 0, "disputed and paid");
                assert!(hold(&app, o.order_id).await.0);
            }
            409 => {
                released += 1;
                assert_eq!(res.json["error"]["code"], "already_released");
                assert_eq!(paid, 1, "neither disputed nor paid");
            }
            other => panic!("unexpected {other}: {}", res.json),
        }
    }
    eprintln!("auto-release race: {disputed} disputed, {released} released");
}

#[sqlx::test]
async fn dispute_races_buyer_confirmation(pool: PgPool) {
    let app = TestApp::new(pool).await;
    for _ in 0..6 {
        let o = app.shipped_order("courier", "delivered").await;
        make_due(&app, o.order_id).await;
        let received_path = format!("/orders/{}/received", o.order_id);
        let confirm = app.request(
            Method::POST,
            &received_path,
            Some(json!({ "confirm": true })),
            Some(&o.buyer_token),
        );
        let (res, received) = tokio::join!(open(&app, &o, report(COMPLAINT)), confirm);
        let paid = payouts(&app, o.order_id).await;
        match res.status {
            201 => {
                assert_eq!(received.status, 409, "{}", received.json);
                assert_eq!(received.json["error"]["code"], "order_held");
                assert_eq!(paid, 0, "disputed and paid");
            }
            409 => {
                assert_eq!(res.json["error"]["code"], "already_released");
                assert_eq!(received.status, 200, "{}", received.json);
                assert_eq!(paid, 1, "neither disputed nor paid");
            }
            other => panic!("unexpected {other}: {}", res.json),
        }
    }
}

#[sqlx::test]
async fn withdraw_unholds_only_a_dispute_hold(pool: PgPool) {
    let app = TestApp::new(pool).await;

    // An owner hold placed first survives the withdrawal.
    let held = app.shipped_order("courier", "delivered").await;
    sqlx::query("UPDATE orders SET held_at = now(), hold_reason = 'owner check' WHERE id = $1")
        .bind(held.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let res = open(&app, &held, report(COMPLAINT)).await;
    assert_eq!(res.status, 201);
    assert_eq!(
        hold(&app, held.order_id).await,
        (true, Some("owner check".into()))
    );
    let w = withdraw(
        &app,
        res.json["id"].as_str().unwrap(),
        &held.buyer_token,
        json!({ "confirm": true }),
    )
    .await;
    assert_eq!(w.status, 200, "{}", w.json);
    assert_eq!(w.json["status"], "withdrawn");
    assert_eq!(
        w.json["note"],
        "You can't report this order again after withdrawing."
    );
    assert_eq!(
        hold(&app, held.order_id).await,
        (true, Some("owner check".into()))
    );

    // A dispute hold is cleared, and a due order then releases.
    let o = app.shipped_order("courier", "delivered").await;
    make_due(&app, o.order_id).await;
    let res = open(&app, &o, report(COMPLAINT)).await;
    let id = res.json["id"].as_str().unwrap().to_string();
    // Needs the exact confirmation body.
    for body in [
        json!({}),
        json!({ "confirm": false }),
        json!({ "confirm": true, "x": 1 }),
    ] {
        assert_eq!(withdraw(&app, &id, &o.buyer_token, body).await.status, 422);
    }
    assert_eq!(
        withdraw(&app, &id, &o.buyer_token, json!({ "confirm": true }))
            .await
            .status,
        200
    );
    assert_eq!(hold(&app, o.order_id).await, (false, None));
    let again = withdraw(&app, &id, &o.buyer_token, json!({ "confirm": true })).await;
    assert_eq!(
        (again.status, again.json["error"]["code"].as_str()),
        (409, Some("dispute_closed"))
    );
    run_due_jobs(&app.state).await;
    assert_eq!(payouts(&app, o.order_id).await, 1);
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'dispute.withdrawn' AND subject_id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(audited, 1);
}

#[sqlx::test]
async fn seller_sees_buyer_as_buyer(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    let res = open(&app, &o, report(COMPLAINT)).await;
    let id = res.json["id"].as_str().unwrap();
    let path = format!("/disputes/{id}");

    let seller = app
        .request(Method::GET, &path, None, Some(&o.seller_token))
        .await;
    assert_eq!(seller.status, 200, "{}", seller.json);
    assert_eq!(seller.json["buyer_statement"]["body"], COMPLAINT);
    let raw = seller.json.to_string();
    let name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
        .bind(o.buyer_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    // The first fixture buyer's number is 08025000000.
    let phone = "8025000000";
    for secret in [name.as_str(), phone, "08022", "address", "buyer_id"] {
        assert!(!raw.contains(secret), "{secret} in the seller's view");
    }
    assert!(seller.json.get("buyer_id").is_none());
    assert!(seller.json.get("note").is_none());

    let buyer = app
        .request(Method::GET, &path, None, Some(&o.buyer_token))
        .await;
    assert_eq!(buyer.status, 200);
    let (_, stranger) = app.sign_up_verified("08099999992").await;
    assert_eq!(
        app.request(Method::GET, &path, None, Some(&stranger))
            .await
            .status,
        404
    );
    assert_eq!(
        app.request(Method::GET, &path, None, None).await.status,
        401
    );
    let missing = format!("/disputes/{}", Uuid::new_v4());
    assert_eq!(
        app.request(Method::GET, &missing, None, Some(&o.buyer_token))
            .await
            .status,
        404
    );
    // A seller cannot withdraw the buyer's report.
    let res = withdraw(&app, id, &o.seller_token, json!({ "confirm": true })).await;
    assert_eq!(res.status, 404);
}

#[sqlx::test]
async fn dispute_blocked_behind_release_is_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    make_due(&app, o.order_id).await;

    // Stand in for a release in flight: it holds the order's row lock.
    let mut tx = app.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM orders WHERE id = $1 FOR UPDATE")
        .bind(o.order_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    let task = {
        let app_router = server::build_app(app.state.clone());
        let path = format!("/orders/{}/dispute", o.order_id);
        let token = o.buyer_token.clone();
        tokio::spawn(async move {
            use tower::ServiceExt;
            let req = axum::http::Request::builder()
                .method(Method::POST)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(report(COMPLAINT).to_string()))
                .unwrap();
            let res = app_router.oneshot(req).await.unwrap();
            let status = res.status().as_u16();
            let bytes = http_body_util::BodyExt::collect(res.into_body())
                .await
                .unwrap()
                .to_bytes();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap())
        })
    };

    // Wait until the dispute is queued behind our lock.
    let mut waiting = false;
    for _ in 0..200 {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE wait_event_type = 'Lock' AND datname = current_database()",
        )
        .fetch_one(&app.pool)
        .await
        .unwrap();
        if n > 0 {
            waiting = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(waiting, "the dispute never queued behind the order lock");

    // The release lands: the order is completed and the payout exists.
    sqlx::query("UPDATE orders SET status = 'completed' WHERE id = $1")
        .bind(o.order_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO payouts (order_id, reference, provider, amount_kobo, status, release_reason) \
         VALUES ($1, 'PO1TEST', 'fake', 100, 'manual', 'auto_after_delivery')",
    )
    .bind(o.order_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (status, body) = task.await.unwrap();
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "already_released");
    let disputes: i64 = sqlx::query_scalar("SELECT count(*) FROM disputes")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(disputes, 0);
    assert_eq!(hold(&app, o.order_id).await, (false, None));
}
