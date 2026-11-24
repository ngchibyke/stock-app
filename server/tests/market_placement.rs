mod common;

use common::TestApp;
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
    let ids = unplaced_stores(&app, 9).await;
    let base = mark(&app).await;
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
    release(&mut tx, &app.state.market, ids[2]).await.unwrap();
    tx.commit().await.unwrap();
    busy.rollback().await.unwrap();

    assert_eq!(plot_of(&app, ids[8]).await, None);
    assert_eq!(plot_of(&app, ids[9]).await, Some(freed));
}
