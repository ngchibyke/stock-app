//! Dashboard audit log view (plan 6, task 6).

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn get(app: &TestApp, path: &str, token: &str) -> TestResponse {
    app.request(Method::GET, path, None, Some(token)).await
}

fn ids(v: &Value) -> Vec<i64> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect()
}

async fn row(pool: &PgPool, actor: Option<Uuid>, action: &str, days_ago: i32) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO audit_log (at, actor_id, action, subject_type, detail) \
         VALUES (now() - make_interval(days => $3), $1, $2, 'thing', '{\"k\": 1}') RETURNING id",
    )
    .bind(actor)
    .bind(action)
    .bind(days_ago)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn only_the_owner_reads_the_audit_log(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let support = app.make_support().await;
    let (_, user) = app.sign_up_verified("08031112222").await;
    assert_eq!(get(&app, "/admin/audit", &support).await.status, 403);
    assert_eq!(get(&app, "/admin/audit", &user).await.status, 403);
}

#[sqlx::test]
async fn filters_by_actor_action_and_time(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let actor = Uuid::new_v4();
    let a = row(&pool, Some(actor), "test.alpha", 5).await;
    let b = row(&pool, Some(actor), "test.beta", 1).await;
    let c = row(&pool, None, "test.alpha", 10).await;

    let by_actor = get(&app, &format!("/admin/audit?actor_id={actor}"), &owner).await;
    assert_eq!(by_actor.status, 200, "{}", by_actor.json);
    assert_eq!(ids(&by_actor.json), vec![b, a]);

    let by_action = get(&app, "/admin/audit?action=test.alpha", &owner).await;
    assert_eq!(ids(&by_action.json), vec![c, a]); // id order

    // Newest first, with the stored detail.
    assert_eq!(by_actor.json["items"][0]["detail"], json!({"k": 1}));
    assert_eq!(by_actor.json["items"][0]["action"], "test.beta");

    let recent = get(
        &app,
        "/admin/audit?action=test.alpha&from=2000-01-01T00:00:00Z&to=2999-01-01T00:00:00Z",
        &owner,
    )
    .await;
    assert_eq!(ids(&recent.json), vec![c, a]);
    let future = get(&app, "/admin/audit?from=2999-01-01T00:00:00Z", &owner).await;
    assert!(ids(&future.json).is_empty());
}

#[sqlx::test]
async fn pages_without_overlap_and_rejects_bad_input(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    for i in 0..5 {
        row(&pool, None, "test.page", i).await;
    }
    let first = get(&app, "/admin/audit?action=test.page&limit=2", &owner).await;
    assert_eq!(ids(&first.json).len(), 2);
    let c1 = first.json["next_cursor"].as_str().unwrap().to_string();
    let second = get(
        &app,
        &format!("/admin/audit?action=test.page&limit=2&before={c1}"),
        &owner,
    )
    .await;
    let c2 = second.json["next_cursor"].as_str().unwrap().to_string();
    let third = get(
        &app,
        &format!("/admin/audit?action=test.page&limit=2&before={c2}"),
        &owner,
    )
    .await;
    assert!(third.json["next_cursor"].is_null());
    let all: Vec<i64> = [first, second, third]
        .iter()
        .flat_map(|r| ids(&r.json))
        .collect();
    assert_eq!(all.len(), 5);
    let mut sorted = all.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    sorted.dedup();
    assert_eq!(all, sorted, "newest first, no overlap");

    for bad in ["before=x", "actor_id=x", "from=x", "limit=0", "limit=101"] {
        assert_eq!(
            get(&app, &format!("/admin/audit?{bad}"), &owner)
                .await
                .status,
            422,
            "{bad}"
        );
    }
}

#[sqlx::test]
async fn nothing_can_write_or_delete_audit_rows(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let id = row(&pool, None, "test.keep", 0).await;
    for m in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        let res = app
            .request(m.clone(), "/admin/audit", Some(json!({})), Some(&owner))
            .await;
        assert_eq!(res.status, 405, "{m}");
        let res = app
            .request(
                m.clone(),
                &format!("/admin/audit/{id}"),
                Some(json!({})),
                Some(&owner),
            )
            .await;
        assert!(res.status == 404 || res.status == 405, "{m} {}", res.status);
    }
    assert!(
        sqlx::query("DELETE FROM audit_log WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE audit_log SET action = 'x' WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .is_err()
    );
}

#[sqlx::test]
async fn reading_the_log_is_itself_audited(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    get(&app, "/admin/audit", &owner).await;
    let (n,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM audit_log WHERE action = 'admin.viewed' AND detail->>'view' = 'audit'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
}
