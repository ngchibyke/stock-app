//! Stock per branch (part 7e-1, task 5): `branch_stock` is the truth, the variant's
//! `stock` and `reserved_quantity` are totals over open branches, and a buyer sees
//! the most one branch can send.

mod common;

use std::{sync::Arc, time::Duration};

use axum::http::Method;
use common::{Listing, TestApp};
use rand::{Rng, SeedableRng, rngs::StdRng};
use serde_json::json;
use server::{error::AppError, stock};
use sqlx::PgPool;
use tokio::sync::Barrier;
use uuid::Uuid;

const SELLER: &str = "08011111111";
const BUYER: &str = "08022222221";

/// A second open branch (position 1) for the listing's store.
async fn add_branch(app: &TestApp, store_id: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO store_branches (store_id, name, pickup_address_enc, position) \
         SELECT store_id, 'Garki', pickup_address_enc, 1 FROM store_branches \
         WHERE store_id = $1 AND position = 0 RETURNING id",
    )
    .bind(store_id)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

async fn first_branch(app: &TestApp, store_id: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM store_branches WHERE store_id = $1 AND position = 0")
        .bind(store_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// (stock, reserved) at one branch; (0, 0) when it has no row.
async fn at_branch(pool: &PgPool, branch: Uuid, variant: Uuid) -> (i32, i32) {
    sqlx::query_as(
        "SELECT stock, reserved_quantity FROM branch_stock WHERE branch_id = $1 AND variant_id = $2",
    )
    .bind(branch)
    .bind(variant)
    .fetch_optional(pool)
    .await
    .unwrap()
    .unwrap_or((0, 0))
}

/// Each variant's totals equal the sum over its open branches, and nothing is negative.
async fn assert_invariant(pool: &PgPool, variants: &[Uuid]) {
    let rows: Vec<(Uuid, i32, i32, i64, i64, i32)> = sqlx::query_as(
        "SELECT v.id, v.stock, v.reserved_quantity, \
                COALESCE(sum(bs.stock) FILTER (WHERE b.closed_at IS NULL), 0)::bigint, \
                COALESCE(sum(bs.reserved_quantity) FILTER (WHERE b.closed_at IS NULL), 0)::bigint, \
                COALESCE(min(bs.stock - bs.reserved_quantity), 0) \
         FROM product_variants v \
         LEFT JOIN branch_stock bs ON bs.variant_id = v.id \
         LEFT JOIN store_branches b ON b.id = bs.branch_id \
         WHERE v.id = ANY($1) GROUP BY v.id",
    )
    .bind(variants)
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), variants.len());
    for (id, stock, reserved, sum_stock, sum_reserved, min_free) in rows {
        assert_eq!(
            (i64::from(stock), i64::from(reserved)),
            (sum_stock, sum_reserved),
            "variant {id} totals drifted from its branches"
        );
        assert!(
            reserved >= 0 && reserved <= stock,
            "variant {id}: {reserved}/{stock}"
        );
        assert!(
            min_free >= 0,
            "variant {id}: a branch holds more than it has"
        );
    }
}

fn is_conflict(e: &AppError, code: &str) -> bool {
    matches!(e, AppError::ConflictDetail(c, _) if *c == code)
}

async fn put_branch_stock(
    app: &TestApp,
    token: &str,
    branch: Uuid,
    items: &[(Uuid, i32)],
) -> common::TestResponse {
    let items: Vec<_> = items
        .iter()
        .map(|(v, s)| json!({ "variant_id": v, "stock": s }))
        .collect();
    app.request(
        Method::PUT,
        &format!("/branches/{branch}/stock"),
        Some(json!({ "items": items })),
        Some(token),
    )
    .await
}

/// One store, two open branches (Wuse first, Garki second), one variant.
async fn two_branch_listing(app: &TestApp, wuse: i32, garki: i32) -> (Listing, Uuid, Uuid) {
    let l = app
        .make_active_store_with_product(SELLER, "Mama Put", 1_000, wuse)
        .await;
    let a = first_branch(app, l.store_id).await;
    let b = add_branch(app, l.store_id).await;
    let res = put_branch_stock(app, &l.seller_token, b, &[(l.variant_id, garki)]).await;
    assert_eq!(res.status, 200, "{}", res.json);
    (l, a, b)
}

#[sqlx::test]
async fn totals_equal_branch_sum_after_any_sequence(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = app
        .make_active_store_with_product(SELLER, "Mama Put", 1_000, 4)
        .await;
    let product = app
        .add_product(
            &l.seller_token,
            l.store_id,
            1_000,
            &[(json!({ "size": "M" }), 3), (json!({ "size": "L" }), 0)],
        )
        .await;
    let variants = app.variant_ids(product).await;
    let branches = [
        first_branch(&app, l.store_id).await,
        add_branch(&app, l.store_id).await,
    ];

    // The model: per (branch, variant), on hand, and the open reservations and sales.
    #[derive(Default, Clone)]
    struct Cell {
        stock: i32,
        pending: Vec<i32>,
        sold: Vec<i32>,
    }
    let cell = |b: usize, v: usize| b * 2 + v;
    let mut model = vec![Cell::default(); 4];
    for (b, &branch) in branches.iter().enumerate() {
        for (v, &variant) in variants.iter().enumerate() {
            model[cell(b, v)].stock = at_branch(&app.pool, branch, variant).await.0;
        }
    }

    let mut rng = StdRng::seed_from_u64(0x7e15);
    for step in 0..200 {
        let (b, v) = (rng.gen_range(0..2), rng.gen_range(0..2));
        let (branch, variant) = (branches[b], variants[v]);
        let c = &mut model[cell(b, v)];
        let reserved: i32 = c.pending.iter().sum();
        let mut tx = app.pool.begin().await.unwrap();
        let op = rng.gen_range(0..6);
        let label = match op {
            0 => {
                let q = rng.gen_range(1..=3);
                let r = stock::apply(&mut tx, branch, variant, 0, q).await;
                if c.stock - reserved >= q {
                    r.unwrap();
                    c.pending.push(q);
                } else {
                    let e = r.unwrap_err();
                    assert!(is_conflict(&e, "out_of_stock"), "step {step}: {e:?}");
                }
                "reserve"
            }
            1 | 2 => match c.pending.pop() {
                // commit: the sale takes the units off hand; expire: they are free again.
                Some(q) if op == 1 => {
                    stock::apply(&mut tx, branch, variant, -q, -q)
                        .await
                        .unwrap();
                    c.stock -= q;
                    c.sold.push(q);
                    "commit"
                }
                Some(q) => {
                    stock::apply(&mut tx, branch, variant, 0, -q).await.unwrap();
                    "expire"
                }
                None => "nothing pending",
            },
            // refund and cancel of a paid order: the units go back on hand.
            3 | 4 => match c.sold.pop() {
                Some(q) => {
                    stock::apply(&mut tx, branch, variant, q, 0).await.unwrap();
                    c.stock += q;
                    if op == 3 { "refund" } else { "cancel" }
                }
                None => "nothing sold",
            },
            _ => {
                let n = rng.gen_range(0..=8);
                let r = stock::set_stock(&mut tx, branch, variant, n).await;
                if n >= reserved {
                    r.unwrap();
                    c.stock = n;
                } else {
                    let e = r.unwrap_err();
                    assert!(
                        is_conflict(&e, "stock_below_reserved"),
                        "step {step}: {e:?}"
                    );
                }
                "set"
            }
        };
        tx.commit().await.unwrap();
        assert_invariant(&app.pool, &variants).await;
        let c = &model[cell(b, v)];
        assert_eq!(
            at_branch(&app.pool, branch, variant).await,
            (c.stock, c.pending.iter().sum()),
            "step {step} ({label})"
        );
    }
}

#[sqlx::test]
async fn buyer_sees_max_at_one_branch(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (l, a, b) = two_branch_listing(&app, 3, 2).await;
    let path = format!("/products/{}", l.product_id);

    // The buyer sees what one branch can send, not the total.
    let public = app.request(Method::GET, &path, None, None).await;
    assert_eq!(
        public.json["variants"][0]["available_stock"], 3,
        "{}",
        public.json
    );
    assert!(public.json["variants"][0].get("branches").is_none());

    // The seller sees each branch and the total.
    let own = app
        .request(Method::GET, &path, None, Some(&l.seller_token))
        .await;
    let v = &own.json["variants"][0];
    assert_eq!(v["stock"], 5, "{v}");
    let mut branches: Vec<(String, i64)> = v["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["branch_id"].as_str().unwrap().to_string(),
                x["stock"].as_i64().unwrap(),
            )
        })
        .collect();
    branches.sort();
    let mut expected = vec![(a.to_string(), 3), (b.to_string(), 2)];
    expected.sort();
    assert_eq!(branches, expected);

    // The cart caps the line at 3.
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    let res = app.set_cart(&buyer, l.variant_id, 4).await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "out_of_stock");
    let res = app.set_cart(&buyer, l.variant_id, 3).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["items"][0]["max_quantity"], 3);

    // Catalog and room still list it as in stock.
    let browse = app.request(Method::GET, "/products", None, None).await;
    assert_eq!(browse.json["items"][0]["in_stock"], true, "{}", browse.json);
    let slug: String = sqlx::query_scalar("SELECT slug FROM stores WHERE id = $1")
        .bind(l.store_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let room = app
        .request(Method::GET, &format!("/stores/{slug}/room"), None, None)
        .await;
    assert_eq!(room.status, 200, "{}", room.json);
    assert!(
        room.json.to_string().contains(&l.product_id.to_string()),
        "{}",
        room.json
    );
}

#[sqlx::test]
async fn plain_stock_refused_with_two_branches(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (l, a, b) = two_branch_listing(&app, 3, 2).await;
    let path = format!("/products/{}/variants", l.product_id);

    let res = app
        .request(
            Method::PUT,
            &path,
            Some(json!([{ "attributes": {}, "stock": 9 }])),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "branch_required");
    assert_eq!(at_branch(&app.pool, a, l.variant_id).await, (3, 0));

    // Per branch: the named branch changes, the other keeps its count.
    let res = app
        .request(
            Method::PUT,
            &path,
            Some(json!([{ "attributes": {}, "stock": { b.to_string(): 7 } }])),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["variants"][0]["stock"], 10);
    assert_eq!(at_branch(&app.pool, a, l.variant_id).await, (3, 0));
    assert_eq!(at_branch(&app.pool, b, l.variant_id).await, (7, 0));

    // A branch of another store is refused.
    let res = app
        .request(
            Method::PUT,
            &path,
            Some(json!([{ "attributes": {}, "stock": { Uuid::new_v4().to_string(): 1 } }])),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "unknown_branch");
    assert_invariant(&app.pool, &[l.variant_id]).await;
}

#[sqlx::test]
async fn staff_sets_stock_only_at_own_branch(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (l, a, b) = two_branch_listing(&app, 3, 2).await;
    let (staff, staff_token) = app.sign_up_verified("08033333331").await;
    sqlx::query(
        "INSERT INTO store_members (store_id, user_id, role, branch_id, added_by) \
         VALUES ($1, $2, 'staff', $3, $4)",
    )
    .bind(l.store_id)
    .bind(staff)
    .bind(b)
    .bind(l.seller_id)
    .execute(&app.pool)
    .await
    .unwrap();
    let (_, stranger) = app.make_seller("08033333332").await;

    let res = put_branch_stock(&app, &staff_token, b, &[(l.variant_id, 6)]).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(at_branch(&app.pool, b, l.variant_id).await, (6, 0));
    let res = put_branch_stock(&app, &staff_token, a, &[(l.variant_id, 9)]).await;
    assert_eq!(res.status, 403, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "not_allowed");
    let res = put_branch_stock(&app, &stranger, b, &[(l.variant_id, 9)]).await;
    assert_eq!(res.status, 404, "{}", res.json);

    // The product's variants route applies the same rule per branch touched.
    let path = format!("/products/{}/variants", l.product_id);
    let set =
        |branch: Uuid, n: i32| json!([{ "attributes": {}, "stock": { branch.to_string(): n } }]);
    let res = app
        .request(Method::PUT, &path, Some(set(a, 1)), Some(&staff_token))
        .await;
    assert_eq!(res.status, 403, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "not_allowed");
    let res = app
        .request(Method::PUT, &path, Some(set(b, 4)), Some(&staff_token))
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    // Restating the other branch's count is no change.
    let res = app
        .request(Method::PUT, &path, Some(set(a, 3)), Some(&staff_token))
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(at_branch(&app.pool, a, l.variant_id).await, (3, 0));
    assert_eq!(at_branch(&app.pool, b, l.variant_id).await, (4, 0));

    // A variant of another store is not this branch's to set.
    let other = app
        .make_active_store_with_product("08011111119", "Other", 1_000, 5)
        .await;
    let res = put_branch_stock(&app, &l.seller_token, a, &[(other.variant_id, 1)]).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "unknown_variant");
    assert_invariant(&app.pool, &[l.variant_id, other.variant_id]).await;
}

#[sqlx::test]
async fn never_negative_under_race(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (l, a, b) = two_branch_listing(&app, 1, 4).await;
    let v = l.variant_id;

    // Two buyers' reservations of Wuse's last unit, in two real transactions: the
    // second waits on the first's lock and then finds nothing left.
    let barrier = Arc::new(Barrier::new(2));
    let reserve = |pool: PgPool, barrier: Arc<Barrier>| async move {
        let mut tx = pool.begin().await.unwrap();
        barrier.wait().await;
        let r = stock::apply(&mut tx, a, v, 0, 1).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        tx.commit().await.unwrap();
        r
    };
    let (r1, r2) = tokio::join!(
        reserve(app.pool.clone(), barrier.clone()),
        reserve(app.pool.clone(), barrier.clone())
    );
    let errors: Vec<AppError> = [r1, r2].into_iter().filter_map(Result::err).collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(is_conflict(&errors[0], "out_of_stock"), "{errors:?}");
    assert_eq!(at_branch(&app.pool, a, v).await, (1, 1));
    // Garki's stock was never touched.
    assert_eq!(at_branch(&app.pool, b, v).await, (4, 0));
    assert_invariant(&app.pool, &[v]).await;

    // The seller recounts Garki to 0 while a buyer reserves its last unit (made 1 first).
    let mut tx = app.pool.begin().await.unwrap();
    stock::set_stock(&mut tx, b, v, 1).await.unwrap();
    tx.commit().await.unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let (pool1, pool2) = (app.pool.clone(), app.pool.clone());
    let (bar1, bar2) = (barrier.clone(), barrier.clone());
    let (reserved, set) = tokio::join!(
        async move {
            let mut tx = pool1.begin().await.unwrap();
            bar1.wait().await;
            let r = stock::apply(&mut tx, b, v, 0, 1).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            tx.commit().await.unwrap();
            r
        },
        async move {
            let mut tx = pool2.begin().await.unwrap();
            bar2.wait().await;
            let r = stock::set_stock(&mut tx, b, v, 0).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            tx.commit().await.unwrap();
            r
        }
    );
    match (&reserved, &set) {
        (Ok(()), Err(e)) => {
            assert!(is_conflict(e, "stock_below_reserved"), "{e:?}");
            assert_eq!(at_branch(&app.pool, b, v).await, (1, 1));
        }
        (Err(e), Ok(())) => {
            assert!(is_conflict(e, "out_of_stock"), "{e:?}");
            assert_eq!(at_branch(&app.pool, b, v).await, (0, 0));
        }
        other => panic!("exactly one must win: {other:?}"),
    }
    assert_invariant(&app.pool, &[v]).await;
}
