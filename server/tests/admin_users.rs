//! Dashboard user search and view (plan 6, task 1).

mod common;

use axum::http::Method;
use common::TestApp;
use serde_json::Value;
use sqlx::PgPool;

async fn get(app: &TestApp, path: &str, token: &str) -> common::TestResponse {
    app.request(Method::GET, path, None, Some(token)).await
}

fn ids(v: &Value) -> Vec<String> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

#[sqlx::test]
async fn owner_finds_a_user_by_phone_and_by_name_prefix(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.make_owner().await;
    let (user_id, _) = app.sign_up_verified("08031112222").await;
    sqlx::query("UPDATE users SET display_name = 'Amaka Obi' WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();

    let by_phone = get(&app, "/admin/users?q=0803%20111%202222", &owner).await;
    assert_eq!(by_phone.status, 200, "{}", by_phone.json);
    assert_eq!(ids(&by_phone.json), vec![user_id.to_string()]);

    let by_name = get(&app, "/admin/users?q=amaka", &owner).await;
    assert_eq!(by_name.status, 200, "{}", by_name.json);
    assert_eq!(ids(&by_name.json), vec![user_id.to_string()]);

    let none = get(&app, "/admin/users?q=zzz", &owner).await;
    assert!(ids(&none.json).is_empty());
}

#[sqlx::test]
async fn the_list_never_shows_a_phone(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.make_owner().await;
    app.sign_up_verified("08031112222").await;
    let res = get(&app, "/admin/users", &owner).await;
    assert_eq!(res.status, 200);
    let text = res.json.to_string();
    assert!(!text.contains("0803"), "{text}");
    assert!(!text.contains("+234"), "{text}");
    assert!(!text.contains("phone"), "{text}");
}

#[sqlx::test]
async fn support_sees_only_dispute_parties(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let support = app.make_support().await;
    let (stranger, _) = app.sign_up_verified("08031112222").await;

    let o = app.shipped_order("self", "delivered").await;
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/dispute", o.order_id),
            Some(serde_json::json!({
                "reason": "damaged",
                "body": "The zip was torn and the lining was stained when it arrived",
                "photo_keys": []
            })),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);

    let list = get(&app, "/admin/users", &support).await;
    assert_eq!(list.status, 200, "{}", list.json);
    let seen = ids(&list.json);
    assert!(!seen.contains(&stranger.to_string()));
    assert!(seen.contains(&o.buyer_id.to_string()), "{seen:?}");
    assert!(seen.contains(&o.seller_id.to_string()), "{seen:?}");

    // Support cannot open the single view (it shows a phone).
    let one = get(&app, &format!("/admin/users/{}", o.buyer_id), &support).await;
    assert_eq!(one.status, 403);
}

#[sqlx::test]
async fn single_view_decrypts_the_phone_and_is_audited(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.make_owner().await;
    let (user_id, _) = app.sign_up_verified("08031112222").await;

    let res = get(&app, &format!("/admin/users/{user_id}"), &owner).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["phone"], "+2348031112222");
    assert_eq!(res.json["role"], "user");
    assert_eq!(res.json["banned"], false);

    let (n,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM audit_log WHERE action = 'admin.user_read' AND subject_id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);

    let missing = get(&app, "/admin/users/not-an-id", &owner).await;
    assert_eq!(missing.status, 404);
}

#[sqlx::test]
async fn paging_and_bad_cursor(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.make_owner().await;
    app.sign_up_verified("08031112221").await;
    app.sign_up_verified("08031112222").await;
    app.sign_up_verified("08031112223").await;

    let first = get(&app, "/admin/users?limit=2", &owner).await;
    assert_eq!(ids(&first.json).len(), 2);
    let cursor = first.json["next_cursor"].as_str().unwrap().to_string();
    let second = get(
        &app,
        &format!("/admin/users?limit=2&before={cursor}"),
        &owner,
    )
    .await;
    assert_eq!(second.status, 200, "{}", second.json);
    let all: Vec<_> = ids(&first.json)
        .into_iter()
        .chain(ids(&second.json))
        .collect();
    let mut dedup = all.clone();
    dedup.sort();
    dedup.dedup();
    assert_eq!(all.len(), dedup.len(), "pages overlap");
    assert_eq!(all.len(), 4); // owner + three

    let bad = get(&app, "/admin/users?before=nope", &owner).await;
    assert_eq!(bad.status, 422);
}

#[sqlx::test]
async fn filters_by_role_and_ban_state(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.make_owner().await;
    let (banned, _) = app.sign_up_verified("08031112222").await;
    sqlx::query("INSERT INTO user_moderation (user_id, banned_at) VALUES ($1, now())")
        .bind(banned)
        .execute(&pool)
        .await
        .unwrap();

    let res = get(&app, "/admin/users?state=banned", &owner).await;
    assert_eq!(ids(&res.json), vec![banned.to_string()]);
    let res = get(&app, "/admin/users?role=owner", &owner).await;
    assert_eq!(ids(&res.json).len(), 1);
    let bad = get(&app, "/admin/users?role=king", &owner).await;
    assert_eq!(bad.status, 422);
}
