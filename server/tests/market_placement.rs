mod common;

use axum::http::Method;
use common::{Listing, TEST_PASSWORD, TestApp};
use serde_json::json;
use server::market::placement::{place, place_missing, release};
use sqlx::PgPool;
use uuid::Uuid;

/// `n` active stores, each with its own seller, oldest first, all in the default
/// district and with no plot yet (placement is cleared after creation so the tests
/// build the "active but unplaced" state themselves).
async fn unplaced_stores(app: &TestApp, n: usize) -> Vec<Uuid> {
    let mut ids = Vec::new();
    for i in 0..n {
        let phone = format!("0803100{i:04}");
        let l = app
            .make_active_store_with_product(&phone, &format!("Shop {i}"), 1000, 5)
            .await;
        ids.push(l.store_id);
    }
    sqlx::query("DELETE FROM store_plots")
        .execute(&app.pool)
        .await
        .unwrap();
    ids
}

async fn set_district(app: &TestApp, store: Uuid, district: &str) {
    sqlx::query("UPDATE stores SET district = $2 WHERE id = $1")
        .bind(store)
        .bind(district)
        .execute(&app.pool)
        .await
        .unwrap();
}

async fn plot_of(app: &TestApp, store: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT plot_id FROM store_plots WHERE store_id = $1")
        .bind(store)
        .fetch_optional(&app.pool)
        .await
        .unwrap()
}

async fn place_now(app: &TestApp, store: Uuid) -> Option<String> {
    let mut tx = app.pool.begin().await.unwrap();
    let plot = place(&mut tx, &app.state.market, store).await.unwrap();
    tx.commit().await.unwrap();
    plot
}

async fn plot_rows(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM store_plots")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// The newest audit id: counts below only look at rows written after it, so they hold
/// whether or not activation already placed the store.
async fn mark(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT COALESCE(max(id), 0) FROM audit_log")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn audit_count(app: &TestApp, base: i64, action: &str, store: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE id > $3 AND action = $1 AND subject_id = $2",
    )
    .bind(action)
    .bind(store)
    .bind(base)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn an_active_store_gets_the_first_free_plot_of_its_district(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let ids = unplaced_stores(&app, 2).await;
    let base = mark(&app).await;
    set_district(&app, ids[0], "fashion").await;
    set_district(&app, ids[1], "fashion").await;

    assert_eq!(place_now(&app, ids[0]).await.as_deref(), Some("fashion-1"));
    assert_eq!(place_now(&app, ids[1]).await.as_deref(), Some("fashion-2"));
    assert_eq!(plot_of(&app, ids[0]).await.as_deref(), Some("fashion-1"));
    assert_eq!(
        audit_count(&app, base, "market.store_placed", ids[0]).await,
        1
    );
    let detail: serde_json::Value = sqlx::query_scalar(
        "SELECT detail FROM audit_log WHERE id > $2 AND action = 'market.store_placed' AND subject_id = $1",
    )
    .bind(ids[0])
    .bind(base)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(detail["plot"], "fashion-1");
    assert_eq!(detail["district"], "fashion");
}

#[sqlx::test]
async fn pending_and_deleted_stores_are_not_placed(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let ids = unplaced_stores(&app, 2).await;
    sqlx::query("UPDATE stores SET status = 'pending' WHERE id = $1")
        .bind(ids[0])
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE stores SET deleted_at = now() WHERE id = $1")
        .bind(ids[1])
        .execute(&app.pool)
        .await
        .unwrap();

    assert_eq!(place_now(&app, ids[0]).await, None);
    assert_eq!(place_now(&app, ids[1]).await, None);
    assert_eq!(plot_rows(&app).await, 0);
}

#[sqlx::test]
async fn placing_twice_changes_nothing(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let ids = unplaced_stores(&app, 1).await;
    let base = mark(&app).await;

    let first = place_now(&app, ids[0]).await;
    let second = place_now(&app, ids[0]).await;
    assert!(first.is_some());
    assert_eq!(first, second);
    assert_eq!(plot_rows(&app).await, 1);
    assert_eq!(
        audit_count(&app, base, "market.store_placed", ids[0]).await,
        1
    );
}

#[sqlx::test]
async fn a_full_district_leaves_the_store_active_and_unplaced(pool: PgPool) {
    let app = TestApp::new(pool).await;
    // From before the stores exist: activating the ninth already audits the full district.
    let base = mark(&app).await;
    let ids = unplaced_stores(&app, 9).await;
    for id in &ids[..8] {
        assert!(place_now(&app, *id).await.is_some());
    }

    assert_eq!(place_now(&app, ids[8]).await, None);
    assert_eq!(plot_of(&app, ids[8]).await, None);
    let status: String = sqlx::query_scalar("SELECT status::text FROM stores WHERE id = $1")
        .bind(ids[8])
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(status, "active");
    assert_eq!(
        audit_count(&app, base, "market.district_full", ids[8]).await,
        1
    );
}

#[sqlx::test]
async fn two_stores_racing_for_the_last_plot_one_wins(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let ids = unplaced_stores(&app, 9).await;
    for id in &ids[..7] {
        assert!(place_now(&app, *id).await.is_some());
    }

    let (a, b) = tokio::join!(place_now(&app, ids[7]), place_now(&app, ids[8]));
    assert_eq!(
        [a.is_some(), b.is_some()].iter().filter(|w| **w).count(),
        1,
        "exactly one store wins the last plot"
    );
    let distinct: i64 = sqlx::query_scalar("SELECT count(DISTINCT plot_id) FROM store_plots")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!((plot_rows(&app).await, distinct), (8, 8));
}

#[sqlx::test]
async fn releasing_a_plot_hands_it_to_the_oldest_waiting_store(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let ids = unplaced_stores(&app, 10).await;
    let base = mark(&app).await;
    for id in &ids[..8] {
        assert!(place_now(&app, *id).await.is_some());
    }
    // ids[8] and ids[9] wait; ids[8] is the older.
    assert_eq!(place_now(&app, ids[8]).await, None);
    assert_eq!(place_now(&app, ids[9]).await, None);
    let freed = plot_of(&app, ids[2]).await.unwrap();

    let mut tx = app.pool.begin().await.unwrap();
    release(&mut tx, &app.state.market, ids[2]).await.unwrap();
    tx.commit().await.unwrap();

    assert_eq!(plot_of(&app, ids[2]).await, None);
    assert_eq!(plot_of(&app, ids[8]).await, Some(freed));
    assert_eq!(plot_of(&app, ids[9]).await, None);
    assert_eq!(
        audit_count(&app, base, "market.store_unplaced", ids[2]).await,
        1
    );
    assert_eq!(
        audit_count(&app, base, "market.store_placed", ids[8]).await,
        1
    );
}

#[sqlx::test]
async fn place_missing_places_existing_stores_oldest_first_and_is_idempotent(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let ids = unplaced_stores(&app, 3).await;

    let placed = place_missing(&app.pool, &app.state.market).await.unwrap();
    assert_eq!(placed, 3);
    for (i, id) in ids.iter().enumerate() {
        let want = format!("everything-else-{}", i + 1);
        assert_eq!(plot_of(&app, *id).await, Some(want));
    }
    assert_eq!(
        place_missing(&app.pool, &app.state.market).await.unwrap(),
        0
    );
}

#[sqlx::test]
async fn unique_plot_is_enforced_by_the_database(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let ids = unplaced_stores(&app, 2).await;
    sqlx::query("INSERT INTO store_plots (store_id, plot_id) VALUES ($1, 'fashion-1')")
        .bind(ids[0])
        .execute(&app.pool)
        .await
        .unwrap();
    let second =
        sqlx::query("INSERT INTO store_plots (store_id, plot_id) VALUES ($1, 'fashion-1')")
            .bind(ids[1])
            .execute(&app.pool)
            .await;
    assert!(second.is_err());
}

#[sqlx::test]
async fn a_store_deleted_while_another_transaction_holds_it_is_not_placed(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let ids = unplaced_stores(&app, 1).await;

    // One transaction deletes the store and keeps its row locked; a placement of
    // the same store must wait for it and then see it deleted.
    let mut deleter = app.pool.begin().await.unwrap();
    sqlx::query("UPDATE stores SET deleted_at = now() WHERE id = $1")
        .bind(ids[0])
        .execute(&mut *deleter)
        .await
        .unwrap();
    let (pool, layout) = (app.pool.clone(), app.state.market.clone());
    let placing = tokio::spawn(async move {
        let mut tx = pool.begin().await.unwrap();
        let plot = place(&mut tx, &layout, ids[0]).await.unwrap();
        tx.commit().await.unwrap();
        plot
    });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    deleter.commit().await.unwrap();

    assert_eq!(placing.await.unwrap(), None);
    assert_eq!(plot_rows(&app).await, 0);
}

#[sqlx::test]
async fn releasing_skips_a_waiting_store_that_another_transaction_holds(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let ids = unplaced_stores(&app, 10).await;
    for id in &ids[..8] {
        assert!(place_now(&app, *id).await.is_some());
    }
    let freed = plot_of(&app, ids[2]).await.unwrap();

    // The oldest waiting store is busy elsewhere (say, being deleted): the release
    // must not wait for it, and hands the plot to the next one.
    let mut busy = app.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM stores WHERE id = $1 FOR UPDATE")
        .bind(ids[8])
        .execute(&mut *busy)
        .await
        .unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    // A regression that waits on the busy store fails fast instead of hanging.
    sqlx::query("SET LOCAL lock_timeout = '2s'")
        .execute(&mut *tx)
        .await
        .unwrap();
    release(&mut tx, &app.state.market, ids[2]).await.unwrap();
    tx.commit().await.unwrap();
    busy.rollback().await.unwrap();

    assert_eq!(plot_of(&app, ids[8]).await, None);
    assert_eq!(plot_of(&app, ids[9]).await, Some(freed));
}

/// `n` active stores created and activated the real way, oldest first; each is
/// placed in the default district's plots in order (8 plots, so later ones wait).
async fn live_stores(app: &TestApp, n: usize) -> Vec<Listing> {
    let mut out = Vec::new();
    for i in 0..n {
        let phone = format!("0803200{i:04}");
        out.push(
            app.make_active_store_with_product(&phone, &format!("Live {i}"), 1000, 5)
                .await,
        );
    }
    out
}

async fn patch_district(app: &TestApp, l: &Listing, district: &str) -> common::TestResponse {
    app.request(
        Method::PATCH,
        &format!("/stores/{}", l.store_id),
        Some(json!({ "district": district })),
        Some(&l.seller_token),
    )
    .await
}

async fn store_status(app: &TestApp, store: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM stores WHERE id = $1")
        .bind(store)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn verifying_the_payout_account_places_the_store(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token) = app.make_seller("08031110001").await;
    let created = app
        .request(
            Method::POST,
            "/stores",
            Some(json!({
                "name": "Fresh Fashion",
                "pickup_address": "12 Unique Street, Wuse 2",
                "district": "fashion",
                "pickup_state": "FCT",
                "delivery_courier": true,
                "delivery_self": false,
                "pickup_area_council": "amac",
                "business_phone": common::STORE_PHONE
            })),
            Some(&token),
        )
        .await;
    assert_eq!(created.status, 201, "{}", created.json);
    let store: Uuid = created.json["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        plot_of(&app, store).await,
        None,
        "pending stores have no plot"
    );
    let base = mark(&app).await;

    sqlx::query("UPDATE platform_settings SET payout_provider = 'fake'")
        .execute(&app.pool)
        .await
        .unwrap();
    app.fake.set_lookup(Ok("ADA OBI".into()));
    let res = app
        .request(
            Method::PUT,
            &format!("/stores/{store}/payout-account"),
            Some(json!({
                "bank_code": "058",
                "account_number": "0123456785",
                "password": TEST_PASSWORD
            })),
            Some(&token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(store_status(&app, store).await, "active");
    assert_eq!(plot_of(&app, store).await.as_deref(), Some("fashion-1"));
    assert_eq!(
        audit_count(&app, base, "market.store_placed", store).await,
        1
    );
}

#[sqlx::test]
async fn a_store_going_back_to_pending_keeps_its_plot(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = app
        .make_active_store_with_product("08031110002", "Steady", 1000, 5)
        .await;
    let plot = plot_of(&app, l.store_id).await;
    assert!(plot.is_some());
    let base = mark(&app).await;

    // A new account whose holder's name does not match leaves the account pending
    // and the store with it.
    app.fake.set_lookup(Ok("SOMEONE ELSE".into()));
    let res = app
        .request(
            Method::PUT,
            &format!("/stores/{}/payout-account", l.store_id),
            Some(json!({
                "bank_code": "058",
                "account_number": "0123456785",
                "password": TEST_PASSWORD
            })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(store_status(&app, l.store_id).await, "pending");
    assert_eq!(plot_of(&app, l.store_id).await, plot);
    assert_eq!(
        audit_count(&app, base, "market.store_unplaced", l.store_id).await,
        0
    );
}

#[sqlx::test]
async fn suspending_keeps_the_plot(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.make_owner().await;
    let l = app
        .make_active_store_with_product("08031110003", "Paused", 1000, 5)
        .await;
    let plot = plot_of(&app, l.store_id).await;
    assert!(plot.is_some());

    let res = app
        .request(
            Method::POST,
            &format!("/admin/stores/{}/suspend", l.store_id),
            Some(json!({ "reason": "Checking a complaint" })),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(store_status(&app, l.store_id).await, "suspended");
    assert_eq!(plot_of(&app, l.store_id).await, plot);
}

#[sqlx::test]
async fn unsuspend_places_a_store_that_waited_for_room(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.make_owner().await;
    let stores = live_stores(&app, 9).await;
    // The ninth store found the district full and waits.
    assert_eq!(plot_of(&app, stores[8].store_id).await, None);
    let suspend = app
        .request(
            Method::POST,
            &format!("/admin/stores/{}/suspend", stores[8].store_id),
            Some(json!({ "reason": "Checking a complaint" })),
            Some(&owner),
        )
        .await;
    assert_eq!(suspend.status, 200, "{}", suspend.json);

    // A plot frees up: the suspended store is not eligible for it.
    let freed = plot_of(&app, stores[0].store_id).await.unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    release(&mut tx, &app.state.market, stores[0].store_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(plot_of(&app, stores[8].store_id).await, None);
    let base = mark(&app).await;

    let res = app
        .request(
            Method::POST,
            &format!("/admin/stores/{}/unsuspend", stores[8].store_id),
            None,
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(store_status(&app, stores[8].store_id).await, "active");
    assert_eq!(plot_of(&app, stores[8].store_id).await, Some(freed));
    assert_eq!(
        audit_count(&app, base, "market.store_placed", stores[8].store_id).await,
        1
    );
}

#[sqlx::test]
async fn changing_district_moves_the_store_to_the_new_district(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let stores = live_stores(&app, 9).await;
    let old_plot = plot_of(&app, stores[0].store_id).await.unwrap();
    assert_eq!(plot_of(&app, stores[8].store_id).await, None);
    let base = mark(&app).await;

    let res = patch_district(&app, &stores[0], "fashion").await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["district"], "fashion");

    assert_eq!(
        plot_of(&app, stores[0].store_id).await.as_deref(),
        Some("fashion-1")
    );
    // The plot it left goes to the oldest waiting store of the old district.
    assert_eq!(plot_of(&app, stores[8].store_id).await, Some(old_plot));
    for (action, store) in [
        ("market.store_unplaced", stores[0].store_id),
        ("market.store_placed", stores[0].store_id),
        ("market.store_placed", stores[8].store_id),
    ] {
        assert_eq!(audit_count(&app, base, action, store).await, 1, "{action}");
    }
}

#[sqlx::test]
async fn changing_district_to_a_full_one_leaves_it_unplaced(pool: PgPool) {
    let app = TestApp::new(pool).await;
    // Eight stores fill the default district (8 plots); the ninth waits.
    let stores = live_stores(&app, 9).await;
    let res = patch_district(&app, &stores[8], "fashion").await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(
        plot_of(&app, stores[8].store_id).await.as_deref(),
        Some("fashion-1")
    );
    let base = mark(&app).await;

    // Moving back to the full district frees the fashion plot and finds no room.
    let res = patch_district(&app, &stores[8], "everything-else").await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["district"], "everything-else");
    assert_eq!(plot_of(&app, stores[8].store_id).await, None);
    assert_eq!(store_status(&app, stores[8].store_id).await, "active");
    assert_eq!(
        audit_count(&app, base, "market.district_full", stores[8].store_id).await,
        1
    );
    assert_eq!(
        audit_count(&app, base, "market.store_unplaced", stores[8].store_id).await,
        1
    );
}

#[sqlx::test]
async fn deleting_a_store_frees_its_plot(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let stores = live_stores(&app, 9).await;
    let freed = plot_of(&app, stores[0].store_id).await.unwrap();
    assert_eq!(plot_of(&app, stores[8].store_id).await, None);
    let base = mark(&app).await;

    let res = app
        .request(
            Method::DELETE,
            &format!("/stores/{}", stores[0].store_id),
            None,
            Some(&stores[0].seller_token),
        )
        .await;
    assert_eq!(res.status, 204);
    assert_eq!(plot_of(&app, stores[0].store_id).await, None);
    assert_eq!(plot_of(&app, stores[8].store_id).await, Some(freed));
    assert_eq!(
        audit_count(&app, base, "market.store_unplaced", stores[0].store_id).await,
        1
    );
}

#[sqlx::test]
async fn sellers_changing_district_in_opposite_directions_do_not_deadlock(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let stores = live_stores(&app, 2).await;
    let first = patch_district(&app, &stores[1], "fashion").await;
    assert_eq!(first.status, 200, "{}", first.json);

    // Each round the two stores swap districts at the same moment. Locking the old
    // district then the new one would let the pair wait on each other.
    for round in 0..10 {
        let (a_to, b_to) = if round % 2 == 0 {
            ("fashion", "everything-else")
        } else {
            ("everything-else", "fashion")
        };
        let (a, b) = tokio::join!(
            patch_district(&app, &stores[0], a_to),
            patch_district(&app, &stores[1], b_to)
        );
        assert_eq!(a.status, 200, "round {round}: {}", a.json);
        assert_eq!(b.status, 200, "round {round}: {}", b.json);
        assert!(plot_of(&app, stores[0].store_id).await.is_some());
        assert!(plot_of(&app, stores[1].store_id).await.is_some());
    }
}

#[sqlx::test]
async fn releasing_hands_the_plot_to_a_waiting_store_that_is_only_read_locked(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let ids = unplaced_stores(&app, 10).await;
    for id in &ids[..8] {
        assert!(place_now(&app, *id).await.is_some());
    }
    let freed = plot_of(&app, ids[2]).await.unwrap();

    // Creating a product takes a share lock on its store: the oldest waiting store
    // being read-locked must not lose its turn.
    let mut reader = app.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM stores WHERE id = $1 FOR SHARE")
        .bind(ids[8])
        .execute(&mut *reader)
        .await
        .unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout = '2s'")
        .execute(&mut *tx)
        .await
        .unwrap();
    release(&mut tx, &app.state.market, ids[2]).await.unwrap();
    tx.commit().await.unwrap();
    reader.rollback().await.unwrap();

    assert_eq!(plot_of(&app, ids[8]).await, Some(freed));
}

async fn market_audit_rows_since(app: &TestApp, base: i64) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE id > $1 AND action LIKE 'market.%'")
        .bind(base)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn a_plot_the_layout_no_longer_has_is_released_and_replaced(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let ids = unplaced_stores(&app, 3).await;
    assert_eq!(
        place_missing(&app.pool, &app.state.market).await.unwrap(),
        3
    );
    let healthy = plot_of(&app, ids[2]).await;
    // A layout swap: one store's plot id is gone, another's plot now sits in a
    // different district than the store's.
    sqlx::query("UPDATE store_plots SET plot_id = 'gone-1' WHERE store_id = $1")
        .bind(ids[0])
        .execute(&app.pool)
        .await
        .unwrap();
    set_district(&app, ids[1], "fashion").await;
    let base = mark(&app).await;

    let placed = place_missing(&app.pool, &app.state.market).await.unwrap();

    assert_eq!(placed, 2);
    assert_eq!(
        plot_of(&app, ids[0]).await.as_deref(),
        Some("everything-else-1")
    );
    assert_eq!(plot_of(&app, ids[1]).await.as_deref(), Some("fashion-1"));
    assert_eq!(plot_of(&app, ids[2]).await, healthy);
    for id in &ids[..2] {
        assert_eq!(
            audit_count(&app, base, "market.store_unplaced", *id).await,
            1
        );
        assert_eq!(audit_count(&app, base, "market.store_placed", *id).await, 1);
    }
    assert_eq!(
        audit_count(&app, base, "market.store_unplaced", ids[2]).await,
        0
    );
    assert_eq!(
        audit_count(&app, base, "market.store_placed", ids[2]).await,
        0
    );

    let base = mark(&app).await;
    assert_eq!(
        place_missing(&app.pool, &app.state.market).await.unwrap(),
        0
    );
    assert_eq!(market_audit_rows_since(&app, base).await, 0);
}

#[sqlx::test]
async fn a_full_district_is_audited_once_not_on_every_pass(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let base = mark(&app).await;
    let ids = unplaced_stores(&app, 9).await;
    for id in &ids[..8] {
        assert!(place_now(&app, *id).await.is_some());
    }

    for _ in 0..3 {
        assert_eq!(
            place_missing(&app.pool, &app.state.market).await.unwrap(),
            0
        );
    }

    assert_eq!(
        audit_count(&app, base, "market.district_full", ids[8]).await,
        1
    );
}
