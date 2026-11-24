mod common;

use axum::http::Method;
use common::TestApp;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

async fn store(app: &TestApp, n: usize) -> Uuid {
    let phone = format!("0803200{n:04}");
    app.make_active_store_with_product(&phone, &format!("Shop {n}"), 1000, 5)
        .await
        .store_id
}

async fn get_market(app: &TestApp) -> common::TestResponse {
    app.request(Method::GET, "/market", None, None).await
}

fn placements(res: &common::TestResponse) -> &Vec<Value> {
    res.json["placements"].as_array().expect("placements array")
}

async fn set_district(app: &TestApp, store: Uuid, district: &str) {
    sqlx::query("UPDATE stores SET district = $2 WHERE id = $1")
        .bind(store)
        .bind(district)
        .execute(&app.pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn market_answers_layout_and_placements_in_the_contract_shape(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let id = store(&app, 1).await;

    let res = get_market(&app).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let top = res.json.as_object().unwrap();
    for key in [
        "version",
        "world",
        "tile_size",
        "max_level",
        "base_image",
        "districts",
        "plots",
        "placements",
    ] {
        assert!(top.contains_key(key), "missing top-level key {key}");
    }
    assert!(!res.json["districts"].as_array().unwrap().is_empty());
    assert!(!res.json["plots"].as_array().unwrap().is_empty());

    let list = placements(&res);
    assert_eq!(list.len(), 1);
    let p = &list[0];
    let plot: String = sqlx::query_scalar("SELECT plot_id FROM store_plots WHERE store_id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(p["plot"], plot.as_str());
    assert_eq!(p["store_type"], "boutique");
    let card = p["store"].as_object().unwrap();
    for key in [
        "id",
        "slug",
        "name",
        "logo_url",
        "rating_average",
        "rating_count",
        "product_count",
    ] {
        assert!(card.contains_key(key), "card is missing {key}");
    }
    assert_eq!(card["id"], id.to_string().as_str());
    assert_eq!(card["name"], "Shop 1");
    assert_eq!(card["product_count"], 1);
    assert_eq!(card["rating_count"], 0);
    assert!(card["rating_average"].is_null());
}

#[sqlx::test]
async fn suspended_pending_and_deleted_stores_are_not_on_the_map(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let keep = store(&app, 1).await;
    let suspended = store(&app, 2).await;
    let pending = store(&app, 3).await;
    let deleted = store(&app, 4).await;
    sqlx::query("UPDATE stores SET status = 'suspended' WHERE id = $1")
        .bind(suspended)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE stores SET status = 'pending' WHERE id = $1")
        .bind(pending)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE stores SET deleted_at = now() WHERE id = $1")
        .bind(deleted)
        .execute(&app.pool)
        .await
        .unwrap();

    let res = get_market(&app).await;
    let ids: Vec<&str> = placements(&res)
        .iter()
        .map(|p| p["store"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![keep.to_string().as_str()]);
}

#[sqlx::test]
async fn an_unplaced_active_store_is_not_on_the_map_but_is_in_the_directory(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let id = store(&app, 1).await;
    sqlx::query("DELETE FROM store_plots WHERE store_id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();

    let res = get_market(&app).await;
    assert!(placements(&res).is_empty());

    let dir = app.request(Method::GET, "/stores", None, None).await;
    let items = dir.json["items"].as_array().unwrap();
    assert!(items.iter().any(|s| s["id"] == id.to_string().as_str()));
}

#[sqlx::test]
async fn market_with_no_stores_has_an_empty_placements_list(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let res = get_market(&app).await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json["placements"], serde_json::json!([]));
    assert!(res.json["plots"].is_array());
}

#[sqlx::test]
async fn store_type_follows_the_district(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let phones = app
        .make_active_store_with_product("08032110001", "Phones", 1000, 5)
        .await
        .store_id;
    // The helper's default district is everything-else; move it, then re-place it.
    set_district(&app, phones, "phones-electronics").await;
    sqlx::query("DELETE FROM store_plots WHERE store_id = $1")
        .bind(phones)
        .execute(&app.pool)
        .await
        .unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    let plot = server::market::placement::place(&mut tx, &app.state.market, phones)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(plot.unwrap().starts_with("phones-electronics"));

    let res = get_market(&app).await;
    let p = placements(&res)
        .iter()
        .find(|p| p["store"]["id"] == phones.to_string().as_str())
        .expect("placed");
    assert_eq!(p["store_type"], "phone_shop");
}

#[sqlx::test]
async fn placements_follow_the_plot_file_order(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let a = store(&app, 1).await;
    let b = store(&app, 2).await;
    // Give them plots so insertion order disagrees with file order.
    sqlx::query("UPDATE store_plots SET plot_id = plot_id || '-x'")
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE store_plots SET plot_id = 'everything-else-2' WHERE store_id = $1")
        .bind(a)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE store_plots SET plot_id = 'everything-else-1' WHERE store_id = $1")
        .bind(b)
        .execute(&app.pool)
        .await
        .unwrap();
    let res = get_market(&app).await;
    let plots: Vec<&str> = placements(&res)
        .iter()
        .map(|p| p["plot"].as_str().unwrap())
        .collect();
    assert_eq!(plots, vec!["everything-else-1", "everything-else-2"]);
}

#[sqlx::test]
async fn a_placement_on_a_plot_no_longer_in_the_layout_is_skipped(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let id = store(&app, 1).await;
    sqlx::query("UPDATE store_plots SET plot_id = 'gone-9' WHERE store_id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    let res = get_market(&app).await;
    assert_eq!(res.status, 200);
    assert!(placements(&res).is_empty());
}

#[sqlx::test]
async fn etag_round_trip_gives_304(pool: PgPool) {
    let app = TestApp::new(pool).await;
    store(&app, 1).await;
    let first = get_market(&app).await;
    assert_eq!(first.status, 200);
    assert_eq!(
        first.headers.get("cache-control").unwrap(),
        "public, max-age=60"
    );
    let etag = first.headers.get("etag").expect("etag").to_str().unwrap();

    let again = app
        .request_with_headers(
            Method::GET,
            "/market",
            None,
            None,
            &[("if-none-match", etag)],
        )
        .await;
    assert_eq!(again.status, 304);
    assert!(again.json.is_null());
    assert_eq!(again.headers.get("etag").unwrap(), etag);

    let stale = app
        .request_with_headers(
            Method::GET,
            "/market",
            None,
            None,
            &[("if-none-match", "\"other\"")],
        )
        .await;
    assert_eq!(stale.status, 200);

    // A change to the map changes the tag.
    store(&app, 2).await;
    let changed = get_market(&app).await;
    assert_ne!(changed.headers.get("etag").unwrap(), etag);
}

#[sqlx::test]
async fn market_needs_no_login(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let res = app.request(Method::GET, "/market", None, None).await;
    assert_eq!(res.status, 200);
}

fn keys_of(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                out.push(k.to_lowercase());
                keys_of(v, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|v| keys_of(v, out)),
        _ => {}
    }
}

#[sqlx::test]
async fn market_holds_nothing_about_the_owner(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let listing = app
        .make_active_store_with_product("08032120001", "Shop O", 1000, 5)
        .await;
    let res = get_market(&app).await;
    let text = res.json.to_string();
    assert!(!text.contains(&listing.seller_id.to_string()));
    assert!(!text.contains("08032120001"));
    assert!(!text.contains("Unique Street"));
    let mut keys = Vec::new();
    keys_of(&res.json, &mut keys);
    for k in &keys {
        assert!(
            !k.contains("owner") && !k.contains("phone") && !k.contains("address"),
            "unexpected key {k}"
        );
    }
}

async fn count(app: &TestApp, table: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn market_read_writes_nothing(pool: PgPool) {
    let app = TestApp::new(pool).await;
    store(&app, 1).await;
    let (audit, plots) = (
        count(&app, "audit_log").await,
        count(&app, "store_plots").await,
    );
    assert_eq!(get_market(&app).await.status, 200);
    assert_eq!(get_market(&app).await.status, 200);
    assert_eq!(count(&app, "audit_log").await, audit);
    assert_eq!(count(&app, "store_plots").await, plots);
}
