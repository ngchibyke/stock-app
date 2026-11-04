mod common;

use axum::http::Method;
use common::{Listing, TestApp};
use serde_json::{Value, json};
use server::{
    couriers::{CourierError, Rate},
    delivery::promise::{load_holidays, promised_date},
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

const SELLER: &str = "08011111111";
const SELLER2: &str = "08011111112";
const BUYER: &str = "08022222221";

fn rate(id: &str, charged_kobo: i64) -> Rate {
    Rate {
        courier_id: id.into(),
        service_code: format!("{id}-standard"),
        courier_name: format!("Courier {id}"),
        amount_kobo: charged_kobo - 1_000,
        charged_kobo,
        delivery_eta: Some(format!("{id} by tomorrow")),
    }
}

async fn patch_store(app: &TestApp, l: &Listing, body: Value) {
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{}", l.store_id),
            Some(body),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
}

/// The promised date for an order paid right now, under the default 18:00 cut-off.
async fn promised_now(app: &TestApp) -> String {
    let holidays = load_holidays(&app.pool).await.unwrap();
    promised_date(OffsetDateTime::now_utc(), 1080, &holidays).to_string()
}

/// Repeat calls add no rows while the option can still be used.
#[sqlx::test]
async fn self_option_is_reused_until_used_or_repriced(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = app
        .make_active_store_with_product("08011111111", "Mama Put", 10_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified("08022222221").await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let self_id = |json: &serde_json::Value| {
        json["stores"][0]["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["method"] == "self")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let first = self_id(&app.delivery_options(&buyer, address).await.json);
    for _ in 0..3 {
        assert_eq!(
            self_id(&app.delivery_options(&buyer, address).await.json),
            first
        );
    }
    let rows = || {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_options WHERE method = 'self'")
            .fetch_one(&app.pool)
    };
    assert_eq!(rows().await.unwrap(), 1);

    // A new fee is a new option: the old price is never reused.
    let res = app
        .request(
            axum::http::Method::PATCH,
            &format!("/stores/{}", l.store_id),
            Some(serde_json::json!({ "self_delivery_fee_kobo": 70_000 })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let res = app.delivery_options(&buyer, address).await;
    assert_ne!(self_id(&res.json), first);
    assert_eq!(rows().await.unwrap(), 2);
}

#[sqlx::test]
async fn options_offer_self_and_courier_with_promised_date(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = app
        .make_active_store_with_product(SELLER, "Mama Put", 100_000, 5)
        .await;
    patch_store(&app, &l, json!({ "self_delivery_fee_kobo": 70_000 })).await;
    // Four rates: only the three cheapest (by what the courier charges) are offered.
    app.courier.set_rates(vec![
        rate("a", 300_000),
        rate("b", 150_000),
        rate("c", 200_000),
        rate("d", 250_000),
    ]);
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 2).await;
    let address = app.buyer_address(&buyer).await;

    let before = promised_now(&app).await;
    let res = app.delivery_options(&buyer, address).await;
    let after = promised_now(&app).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let stores = res.json["stores"].as_array().unwrap();
    assert_eq!(stores.len(), 1);
    let store = &stores[0];
    assert_eq!(store["store_id"], json!(l.store_id));
    assert_eq!(store["store_name"], "Mama Put");
    assert!(store.get("unavailable").is_none(), "{store}");
    let options = store["options"].as_array().unwrap();
    let summary: Vec<(String, i64, Option<String>)> = options
        .iter()
        .map(|o| {
            (
                o["method"].as_str().unwrap().to_string(),
                o["fee_kobo"].as_i64().unwrap(),
                o["courier_name"].as_str().map(str::to_string),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            ("self".into(), 70_000, None),
            ("courier".into(), 150_000, Some("Courier b".into())),
            ("courier".into(), 200_000, Some("Courier c".into())),
            ("courier".into(), 250_000, Some("Courier d".into())),
        ]
    );
    assert_eq!(options[1]["delivery_eta"], "b by tomorrow");
    assert!(options[0].get("delivery_eta").is_none());
    for o in options {
        let shown = o["promised_date"].as_str().unwrap();
        assert!(
            shown == before || shown == after,
            "{shown} vs {before}/{after}"
        );
    }

    // Stored server-side, tied to this buyer, store and address, expiring with the TTL.
    #[allow(clippy::type_complexity)]
    let rows: Vec<(Uuid, Uuid, Uuid, String, i64, String, Option<String>, Option<i64>, bool)> =
        sqlx::query_as(
            "SELECT buyer_id, store_id, address_id, method::text, fee_kobo, cart_fingerprint, \
                    request_token, charged_kobo, \
                    expires_at BETWEEN now() + interval '29 minutes' AND now() + interval '31 minutes' \
             FROM delivery_options ORDER BY fee_kobo",
        )
        .fetch_all(&app.pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    for r in &rows {
        assert_eq!((r.0, r.1, r.2), (buyer_id, l.store_id, address));
        assert_eq!(r.5.len(), 64, "sha256 hex");
        assert!(r.8, "expires after the quote TTL");
    }
    assert_eq!(rows[0].3, "self");
    assert_eq!((rows[0].6.as_deref(), rows[0].7), (None, None));
    assert_eq!(
        (rows[1].3.as_str(), rows[1].6.as_deref(), rows[1].7),
        ("courier", Some("fake-token"), Some(150_000))
    );
    // Every option of one quote shares the cart fingerprint.
    assert!(rows.iter().all(|r| r.5 == rows[0].5));

    // Both address codes are fetched once and cached on their rows.
    let codes = || async {
        let buyer_code: Option<i64> =
            sqlx::query_scalar("SELECT courier_address_code FROM buyer_addresses")
                .fetch_one(&app.pool)
                .await
                .unwrap();
        let store_code: Option<i64> =
            sqlx::query_scalar("SELECT pickup_courier_address_code FROM stores")
                .fetch_one(&app.pool)
                .await
                .unwrap();
        (buyer_code, store_code)
    };
    let first = codes().await;
    assert!(first.0.is_some() && first.1.is_some(), "{first:?}");
    let res = app.delivery_options(&buyer, address).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(codes().await, first, "cached codes are reused");
    // The unused, unexpired quote is reused: no second provider call.
    assert_eq!(app.courier.quote_calls(), 1);
    let sender: i64 = sqlx::query_scalar(
        "SELECT DISTINCT courier_sender_code FROM delivery_options WHERE method = 'courier'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(Some(sender), first.1, "the quote records its sender code");
}

#[sqlx::test]
async fn options_reuse_a_valid_courier_quote(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = app
        .make_active_store_with_product(SELLER, "Mama Put", 100_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let ids = |res: &common::TestResponse, method: &str| -> Vec<String> {
        res.json["stores"][0]["options"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o["method"] == method)
            .map(|o| o["id"].as_str().unwrap().to_string())
            .collect()
    };

    let first = app.delivery_options(&buyer, address).await;
    assert_eq!(first.status, 200, "{}", first.json);
    let again = app.delivery_options(&buyer, address).await;
    assert_eq!(again.status, 200, "{}", again.json);
    assert_eq!(app.courier.quote_calls(), 1);
    assert_eq!(ids(&again, "courier"), ids(&first, "courier"));
    assert_eq!(ids(&again, "self").len(), 1);

    // A changed cart is a new quote.
    app.set_cart(&buyer, l.variant_id, 2).await;
    let changed = app.delivery_options(&buyer, address).await;
    assert_eq!(changed.status, 200, "{}", changed.json);
    assert_eq!(app.courier.quote_calls(), 2);
    assert_ne!(ids(&changed, "courier"), ids(&first, "courier"));

    // A quote used by a checkout is never offered again.
    let courier: Uuid = ids(&changed, "courier")[0].parse().unwrap();
    let res = app
        .checkout_with(&buyer, "mono_directpay", address, &[(l.store_id, courier)])
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    let after = app.delivery_options(&buyer, address).await;
    assert_eq!(after.status, 200, "{}", after.json);
    assert_eq!(app.courier.quote_calls(), 3);
    assert!(!ids(&after, "courier").contains(&courier.to_string()));
}

#[sqlx::test]
async fn fresh_quotes_are_capped_per_hour(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = app
        .make_active_store_with_product(SELLER, "Mama Put", 100_000, 5)
        .await;
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    assert_eq!(app.delivery_options(&buyer, address).await.status, 200);
    // 19 more fresh quotes in the last hour (20 in all); older ones do not count.
    sqlx::query(
        "INSERT INTO audit_log (actor_id, action, subject_type, subject_id, detail, at) \
         SELECT $1, 'delivery.options_quoted', 'buyer_address', $2, \
                '{\"courier_quotes\": 1}'::jsonb, now() - make_interval(mins => g) \
         FROM generate_series(1, 19) g \
         UNION ALL \
         SELECT $1, 'delivery.options_quoted', 'buyer_address', $2, \
                '{\"courier_quotes\": 1}'::jsonb, now() - interval '2 hours' \
         FROM generate_series(1, 5)",
    )
    .bind(buyer_id)
    .bind(address)
    .execute(&app.pool)
    .await
    .unwrap();

    // Reusing the valid quote costs nothing and is not capped.
    assert_eq!(app.delivery_options(&buyer, address).await.status, 200);
    // A fresh quote is refused before any provider call.
    app.set_cart(&buyer, l.variant_id, 2).await;
    let res = app.delivery_options(&buyer, address).await;
    assert_eq!(res.status, 429, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "too_many_attempts");
    assert_eq!(app.courier.quote_calls(), 1);
    // A self-delivery-only store needs no provider call and is not capped.
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{}", l.store_id),
            Some(json!({ "delivery_courier": false })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(app.delivery_options(&buyer, address).await.status, 200);
    assert_eq!(app.courier.quote_calls(), 1);
}

#[sqlx::test]
async fn courier_failure_leaves_self_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let both = app
        .make_active_store_with_product(SELLER, "Mama Put", 100_000, 5)
        .await;
    let courier_only = app
        .make_active_store_with_product(SELLER2, "Bukka Hut", 50_000, 5)
        .await;
    // Self-delivery switched off: the fee kept on the row must not be offered.
    patch_store(&app, &courier_only, json!({ "delivery_self": false })).await;
    app.courier.fail_quote_with(CourierError::Unavailable);
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, both.variant_id, 1).await;
    app.set_cart(&buyer, courier_only.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;

    let res = app.delivery_options(&buyer, address).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let stores = res.json["stores"].as_array().unwrap();
    let find = |id: Uuid| {
        stores
            .iter()
            .find(|s| s["store_id"] == json!(id))
            .unwrap()
            .clone()
    };
    let a = find(both.store_id);
    let methods: Vec<&str> = a["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["method"].as_str().unwrap())
        .collect();
    assert_eq!(methods, vec!["self"]);
    assert!(a.get("unavailable").is_none());
    let b = find(courier_only.store_id);
    assert_eq!(b["options"], json!([]));
    assert_eq!(b["unavailable"], "no_delivery_option");
    assert_eq!(app.courier.quote_calls(), 2);

    // The store with no option cannot be checked out.
    let option = a["options"][0]["id"].as_str().unwrap().parse().unwrap();
    let res = app
        .checkout_with(
            &buyer,
            "mono_directpay",
            address,
            &[(both.store_id, option)],
        )
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "delivery_option_required");
}

#[sqlx::test]
async fn courier_store_without_phone_gets_no_courier_option(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = app
        .make_active_store_with_product(SELLER, "Mama Put", 100_000, 5)
        .await;
    // A legacy courier store from before the phone was required (ruling T6d).
    sqlx::query("UPDATE stores SET business_phone_enc = NULL")
        .execute(&app.pool)
        .await
        .unwrap();
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let res = app.delivery_options(&buyer, address).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let options = res.json["stores"][0]["options"].as_array().unwrap();
    assert_eq!(options.len(), 1, "{}", res.json);
    assert_eq!(options[0]["method"], "self");
    assert_eq!(app.courier.quote_calls(), 0);
}

#[sqlx::test]
async fn options_need_the_buyers_own_address_and_a_cart(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = app
        .make_active_store_with_product(SELLER, "Mama Put", 100_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    let (_, other) = app.sign_up_verified("08022222222").await;
    let address = app.buyer_address(&buyer).await;
    let res = app.delivery_options(&buyer, address).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "cart_empty");

    app.set_cart(&other, l.variant_id, 1).await;
    let res = app.delivery_options(&other, address).await;
    assert_eq!(res.status, 404, "{}", res.json);
    let res = app
        .request(
            Method::POST,
            "/checkout/delivery-options",
            Some(json!({ "address_id": address })),
            None,
        )
        .await;
    assert_eq!(res.status, 401);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_options")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        0
    );
}
