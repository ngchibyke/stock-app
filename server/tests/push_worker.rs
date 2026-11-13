//! The push loop: the outbox goes to eligible devices through `PushSender`
//! (plan 5, task 9). `FakePush` stands in for FCM.

mod common;

use std::time::Duration;

use axum::http::Method;
use common::{TEST_PASSWORD, TestApp};
use serde_json::json;
use server::notify::{
    kinds::Kind,
    push::PushError,
    service::{Notice, enqueue},
    worker::run_once,
};
use sqlx::PgPool;
use uuid::Uuid;

const ADA: &str = "08022222221";
const BOLA: &str = "08033333331";

async fn device(app: &TestApp, session: &str, push_token: &str) {
    let res = app
        .request(
            Method::PUT,
            "/me/push-device",
            Some(json!({ "token": push_token, "platform": "android" })),
            Some(session),
        )
        .await;
    assert!(res.status < 300, "{}", res.json);
}

/// A second session for the same person (another phone).
async fn second_session(app: &TestApp, phone: &str) -> String {
    let res = app.sign_in(phone, TEST_PASSWORD).await;
    assert_eq!(res.status, 200, "{}", res.json);
    res.json["token"].as_str().unwrap().to_string()
}

async fn notice(app: &TestApp, user: Uuid, kind: Kind) -> Uuid {
    let key = format!("test:{}", Uuid::new_v4());
    let mut conn = app.pool.acquire().await.unwrap();
    enqueue(&mut conn, Notice::user(user, kind, key.clone()))
        .await
        .unwrap();
    sqlx::query_scalar("SELECT id FROM notifications WHERE dedupe_key = $1")
        .bind(key)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// `(push_status, push_attempts, devices already sent to)`.
async fn state(app: &TestApp, id: Uuid) -> (String, i32, i32) {
    sqlx::query_as(
        "SELECT push_status, push_attempts, cardinality(pushed_devices) \
         FROM notifications WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

/// Makes a backed-off notice due now.
async fn make_due(app: &TestApp, id: Uuid) {
    sqlx::query(
        "UPDATE notifications SET next_push_at = now() - interval '1 second' WHERE id = $1",
    )
    .bind(id)
    .execute(&app.pool)
    .await
    .unwrap();
}

async fn tick(app: &TestApp) {
    run_once(&app.state).await.unwrap();
}

#[sqlx::test]
async fn push_payload_is_fixed_copy_and_ids(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, session) = app.sign_up_verified(ADA).await;
    device(&app, &session, "ada-phone").await;
    let id = notice(&app, ada, Kind::OrderDelivered).await;
    tick(&app).await;

    let seen = app.push.seen();
    assert_eq!(seen.len(), 1);
    let m = &seen[0];
    assert_eq!(m.token, "ada-phone");
    assert_eq!(m.title, Kind::OrderDelivered.title());
    assert_eq!(m.body, Kind::OrderDelivered.body());
    assert_eq!(m.kind, "order_delivered");
    assert_eq!(m.notification_id, id);
    assert_eq!(state(&app, id).await, ("sent".to_string(), 0, 1));

    // Sent once: a later tick sends nothing.
    tick(&app).await;
    assert_eq!(app.push.sent().len(), 1);
}

#[sqlx::test]
async fn push_outage_keeps_inbox_and_retries(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, session) = app.sign_up_verified(ADA).await;
    device(&app, &session, "ada-phone").await;
    app.push.set_all(Err(PushError::Unavailable));
    let id = notice(&app, ada, Kind::RefundStarted).await;
    tick(&app).await;

    // The inbox has it whatever push does.
    let inbox = app
        .request(Method::GET, "/me/notifications", None, Some(&session))
        .await;
    assert_eq!(inbox.json["items"][0]["kind"], "refund_started");
    assert_eq!(state(&app, id).await, ("pending".to_string(), 1, 0));
    let wait: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM next_push_at - now())::float8 FROM notifications WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(
        (20.0..=30.0).contains(&wait),
        "first backoff is 30s: {wait}"
    );

    // Backed off: not tried again until due.
    tick(&app).await;
    assert_eq!(app.push.sent().len(), 1);

    app.push.set_all(Ok(()));
    make_due(&app, id).await;
    tick(&app).await;
    assert_eq!(state(&app, id).await, ("sent".to_string(), 1, 1));
    assert_eq!(app.push.sent().len(), 2);
}

#[sqlx::test]
async fn backoff_doubles_and_is_capped(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, session) = app.sign_up_verified(ADA).await;
    device(&app, &session, "ada-phone").await;
    app.push.set_all(Err(PushError::Unavailable));
    let id = notice(&app, ada, Kind::RefundStarted).await;
    let mut waits = Vec::new();
    for _ in 0..3 {
        make_due(&app, id).await;
        tick(&app).await;
        let wait: f64 = sqlx::query_scalar(
            "SELECT extract(epoch FROM next_push_at - now())::float8 FROM notifications \
             WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
        waits.push(wait.round() as i64);
    }
    for (wait, want) in waits.iter().zip([30, 60, 120]) {
        assert!((want - 10..=want).contains(wait), "{waits:?}");
    }
}

#[sqlx::test]
async fn revoked_session_gets_no_push(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, session) = app.sign_up_verified(ADA).await;
    device(&app, &session, "ada-phone").await;
    let out = app
        .request(Method::POST, "/auth/sign-out", None, Some(&session))
        .await;
    assert_eq!(out.status, 204);
    let id = notice(&app, ada, Kind::OrderDelivered).await;
    tick(&app).await;
    assert!(app.push.sent().is_empty());
    assert_eq!(state(&app, id).await, ("skipped".to_string(), 0, 0));
}

#[sqlx::test]
async fn expired_session_deleted_user_and_other_users_get_no_push(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, ada_session) = app.sign_up_verified(ADA).await;
    let (bola, bola_session) = app.sign_up_verified(BOLA).await;
    device(&app, &ada_session, "ada-phone").await;
    device(&app, &bola_session, "bola-phone").await;

    // Ada's notice goes to Ada's phone only.
    notice(&app, ada, Kind::OrderDelivered).await;
    tick(&app).await;
    assert_eq!(
        app.push
            .sent()
            .iter()
            .map(|s| s.0.as_str())
            .collect::<Vec<_>>(),
        ["ada-phone"]
    );

    sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 minute' WHERE user_id = $1")
        .bind(ada)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(bola)
        .execute(&app.pool)
        .await
        .unwrap();
    let a = notice(&app, ada, Kind::OrderDelivered).await;
    let b = notice(&app, bola, Kind::OrderDelivered).await;
    tick(&app).await;
    assert_eq!(app.push.sent().len(), 1, "nothing more was sent");
    assert_eq!(state(&app, a).await.0, "skipped");
    assert_eq!(state(&app, b).await.0, "skipped");
}

#[sqlx::test]
async fn retry_skips_devices_already_sent(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, first) = app.sign_up_verified(ADA).await;
    let second = second_session(&app, ADA).await;
    device(&app, &first, "ada-phone").await;
    device(&app, &second, "ada-tablet").await;
    app.push.set_unavailable("ada-tablet", true);
    let id = notice(&app, ada, Kind::OrderDelivered).await;
    tick(&app).await;
    assert_eq!(state(&app, id).await, ("pending".to_string(), 1, 1));

    app.push.set_unavailable("ada-tablet", false);
    make_due(&app, id).await;
    tick(&app).await;
    assert_eq!(state(&app, id).await, ("sent".to_string(), 1, 2));
    let mut tokens: Vec<String> = app.push.sent().into_iter().map(|s| s.0).collect();
    tokens.sort();
    assert_eq!(
        tokens,
        ["ada-phone", "ada-tablet", "ada-tablet"],
        "the phone got it once"
    );
}

#[sqlx::test]
async fn unregistered_token_is_disabled(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, session) = app.sign_up_verified(ADA).await;
    device(&app, &session, "ada-phone").await;
    app.push.unregister("ada-phone");
    let id = notice(&app, ada, Kind::OrderDelivered).await;
    tick(&app).await;
    let (disabled, reason): (bool, Option<String>) =
        sqlx::query_as("SELECT disabled_at IS NOT NULL, disabled_reason FROM push_devices")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(disabled);
    assert_eq!(reason.as_deref(), Some("unregistered"));
    assert_eq!(state(&app, id).await.0, "sent", "nothing is left to try");

    // The dead device is never tried again.
    let next = notice(&app, ada, Kind::OrderDelivered).await;
    tick(&app).await;
    assert_eq!(app.push.sent().len(), 1);
    assert_eq!(state(&app, next).await.0, "skipped");
}

#[sqlx::test]
async fn rejected_device_is_done_not_retried(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, session) = app.sign_up_verified(ADA).await;
    device(&app, &session, "ada-phone").await;
    app.push.set_next(Err(PushError::Rejected));
    let id = notice(&app, ada, Kind::OrderDelivered).await;
    tick(&app).await;
    assert_eq!(state(&app, id).await, ("sent".to_string(), 0, 1));
    let disabled: bool = sqlx::query_scalar("SELECT disabled_at IS NOT NULL FROM push_devices")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(!disabled, "a refusal does not disable the device");
}

#[sqlx::test]
async fn stale_notice_is_skipped(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, session) = app.sign_up_verified(ADA).await;
    device(&app, &session, "ada-phone").await;
    let id = notice(&app, ada, Kind::OrderDelivered).await;
    sqlx::query("UPDATE notifications SET created_at = now() - interval '25 hours' WHERE id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    tick(&app).await;
    assert!(app.push.sent().is_empty());
    assert_eq!(state(&app, id).await.0, "skipped");
}

#[sqlx::test]
async fn gives_up_after_six_attempts(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, session) = app.sign_up_verified(ADA).await;
    device(&app, &session, "ada-phone").await;
    app.push.set_all(Err(PushError::Unavailable));
    let id = notice(&app, ada, Kind::OrderDelivered).await;
    for attempt in 1..=5 {
        make_due(&app, id).await;
        tick(&app).await;
        assert_eq!(state(&app, id).await, ("pending".to_string(), attempt, 0));
    }
    make_due(&app, id).await;
    tick(&app).await;
    assert_eq!(state(&app, id).await, ("failed".to_string(), 6, 0));
    make_due(&app, id).await;
    tick(&app).await;
    assert_eq!(app.push.sent().len(), 6, "a failed notice is left alone");
}

#[sqlx::test]
async fn late_answer_after_lease_is_not_recorded(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, session) = app.sign_up_verified(ADA).await;
    device(&app, &session, "ada-phone").await;
    app.push.set_delay(Duration::from_millis(1500));
    let id = notice(&app, ada, Kind::OrderDelivered).await;

    let state_clone = app.state.clone();
    let running = tokio::spawn(async move { run_once(&state_clone).await });
    // While the send is in flight, the lease passes to another worker.
    let mut moved = 0;
    for _ in 0..100 {
        moved = sqlx::query(
            "UPDATE notifications SET push_until = now() + interval '5 minutes' \
             WHERE id = $1 AND push_status = 'sending'",
        )
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap()
        .rows_affected();
        if moved == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(moved, 1, "the send was in flight");
    running.await.unwrap().unwrap();
    assert_eq!(app.push.sent().len(), 1);

    assert_eq!(
        state(&app, id).await,
        ("sending".to_string(), 0, 0),
        "the late answer changed nothing"
    );
}

#[sqlx::test]
async fn expired_lease_is_claimed_again(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, session) = app.sign_up_verified(ADA).await;
    device(&app, &session, "ada-phone").await;
    let id = notice(&app, ada, Kind::OrderDelivered).await;
    // A worker claimed it and died.
    sqlx::query(
        "UPDATE notifications SET push_status = 'sending', \
                push_until = now() - interval '1 second' WHERE id = $1",
    )
    .bind(id)
    .execute(&app.pool)
    .await
    .unwrap();
    tick(&app).await;
    assert_eq!(state(&app, id).await, ("sent".to_string(), 0, 1));
}
