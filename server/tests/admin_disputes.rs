//! The staff dispute screen (plan 5, task 6): the queue, and one read-only view
//! of a dispute for the Owner and Support, with the buyer's contact left out.

mod common;

use axum::http::Method;
use common::{BUYER_ADDRESS_LINE, ShippedOrder, TestApp, TestResponse};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const COMPLAINT: &str = "The zip was torn and the lining was stained when it arrived";
const REPLY: &str = "It left the shop in perfect condition and was packed in a box";
const CHAT_TEXT: &str = "Hello, the parcel has not reached me yet";
const PHONE_TEXT: &str = "call me on 08031234567";

async fn open_id(app: &TestApp, o: &ShippedOrder) -> String {
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/dispute", o.order_id),
            Some(json!({ "reason": "damaged", "body": COMPLAINT, "photo_keys": [] })),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    res.json["id"].as_str().unwrap().to_string()
}

async fn respond(app: &TestApp, o: &ShippedOrder, id: &str) {
    let res = app
        .request(
            Method::POST,
            &format!("/disputes/{id}/response"),
            Some(json!({ "body": REPLY, "photo_keys": [] })),
            Some(&o.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
}

async fn get(app: &TestApp, path: &str, token: &str) -> TestResponse {
    app.request(Method::GET, path, None, Some(token)).await
}

async fn screen(app: &TestApp, id: &str, token: &str) -> TestResponse {
    get(app, &format!("/admin/disputes/{id}"), token).await
}

fn code(res: &TestResponse) -> (u16, Option<&str>) {
    (res.status, res.json["error"]["code"].as_str())
}

async fn say(app: &TestApp, chat: &str, token: &str, body: &str) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/chats/{chat}/messages"),
        Some(json!({ "client_message_id": Uuid::new_v4(), "body": body })),
        Some(token),
    )
    .await
}

/// The order's chat, with one message from each side.
async fn chat_on(app: &TestApp, o: &ShippedOrder) -> String {
    let res = app
        .request(
            Method::POST,
            "/chats",
            Some(json!({ "order_id": o.order_id })),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let chat = res.json["id"].as_str().unwrap().to_string();
    let sent = say(app, &chat, &o.buyer_token, CHAT_TEXT).await;
    assert_eq!(sent.json["outcome"], "delivered", "{}", sent.json);
    let sent = say(app, &chat, &o.seller_token, "It is on its way today").await;
    assert_eq!(sent.json["outcome"], "delivered", "{}", sent.json);
    chat
}

async fn audit_count(app: &TestApp, action: &str, subject: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = $1 AND subject_id = $2::uuid")
        .bind(action)
        .bind(subject)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn owner_screen_has_every_section(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("self", "out_for_delivery").await;
    let owner = app.owner_token().await;

    // A wrong handover entry by the seller.
    let wrong = app
        .request(
            Method::POST,
            &format!("/orders/{}/delivered", o.order_id),
            Some(json!({ "handover_code": "000000" })),
            Some(&o.seller_token),
        )
        .await;
    assert_eq!(wrong.status, 422, "{}", wrong.json);

    let chat = chat_on(&app, &o).await;
    let blocked = say(&app, &chat, &o.buyer_token, PHONE_TEXT).await;
    assert_eq!(blocked.json["outcome"], "blocked", "{}", blocked.json);

    let id = open_id(&app, &o).await;
    respond(&app, &o, &id).await;

    let res = screen(&app, &id, &owner).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let v = &res.json;
    assert_eq!(v["dispute"]["id"], id.as_str());
    assert_eq!(v["dispute"]["status"], "awaiting_decision");
    assert_eq!(v["dispute"]["buyer_statement"]["body"], COMPLAINT);
    assert_eq!(v["dispute"]["seller_statement"]["body"], REPLY);
    assert_eq!(v["order"]["order_id"], o.order_id.to_string());
    assert_eq!(v["order"]["held"], true);
    assert_eq!(v["payment"]["checkout_status"], "paid");
    assert_eq!(v["payment"]["provider"], "fake");
    assert_eq!(v["payment"]["provider_flagged"], false);
    assert!(v["payment"]["total_kobo"].as_i64().unwrap() > 0);
    assert!(!v["ledger"].as_array().unwrap().is_empty(), "{v}");
    for row in v["ledger"].as_array().unwrap() {
        assert!(row["account"].is_string() && row["kind"].is_string());
    }
    let steps = v["timeline"].as_array().unwrap();
    assert_eq!(steps.len(), 6);
    assert_eq!(
        steps
            .iter()
            .find(|s| s["current"] == true)
            .map(|s| s["key"].clone()),
        Some(json!("out_for_delivery"))
    );
    assert!(v["delivery_events"].is_array());
    assert_eq!(v["handover"]["applies"], true);
    assert_eq!(v["handover"]["wrong_entries"], 1);
    assert_eq!(v["handover"]["delivered_with_code"], false);
    assert!(v["handover"]["issued_at"].is_string());
    assert!(!v.to_string().contains("handover_code"));

    let messages = v["chat"].as_array().unwrap();
    assert_eq!(messages.len(), 2, "blocked messages are not in the chat");
    assert_eq!(messages[0]["from"], "buyer");
    assert_eq!(messages[0]["body"], CHAT_TEXT);
    assert_eq!(messages[1]["from"], "seller");
    assert!(messages[0]["seq"].as_i64() < messages[1]["seq"].as_i64());

    let flagged = v["flagged"].as_array().unwrap();
    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0]["original"], PHONE_TEXT);
    assert_eq!(flagged[0]["from"], "buyer");
}

#[sqlx::test]
async fn handover_reports_a_code_delivery(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("self", "delivered").await;
    let owner = app.owner_token().await;
    // A delivered order that is not yet released can still be reported.
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/dispute", o.order_id),
            Some(json!({ "reason": "damaged", "body": COMPLAINT, "photo_keys": [] })),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    let id = res.json["id"].as_str().unwrap();
    let v = screen(&app, id, &owner).await.json;
    assert_eq!(v["handover"]["delivered_with_code"], true);
    assert_eq!(v["handover"]["wrong_entries"], 0);
}

#[sqlx::test]
async fn support_reads_open_dispute_without_originals(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("self", "out_for_delivery").await;
    let support = app.make_support().await;
    let chat = chat_on(&app, &o).await;
    say(&app, &chat, &o.buyer_token, PHONE_TEXT).await;
    let id = open_id(&app, &o).await;

    let res = screen(&app, &id, &support).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["dispute"]["id"], id.as_str());
    assert_eq!(res.json["chat"].as_array().unwrap().len(), 2);
    assert_eq!(res.json["dispute"]["buyer_statement"]["body"], COMPLAINT);
}

#[sqlx::test]
async fn support_never_sees_originals(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("self", "out_for_delivery").await;
    let support = app.make_support().await;
    let chat = chat_on(&app, &o).await;
    let blocked = say(&app, &chat, &o.buyer_token, PHONE_TEXT).await;
    assert_eq!(blocked.json["outcome"], "blocked");
    let id = open_id(&app, &o).await;
    // Even the owner's hold note stays out of Support's view.
    sqlx::query("UPDATE orders SET hold_reason = 'private owner note' WHERE id = $1")
        .bind(o.order_id)
        .execute(&app.pool)
        .await
        .unwrap();

    let res = screen(&app, &id, &support).await;
    assert_eq!(res.status, 200);
    assert!(res.json.get("flagged").is_none(), "{}", res.json);
    let text = res.json.to_string();
    assert!(!text.contains(PHONE_TEXT) && !text.contains("08031234567"));
    assert!(!text.contains("private owner note"));
}

#[sqlx::test]
async fn support_cannot_read_closed_dispute(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let support = app.make_support().await;

    let decided = app.shipped_order("self", "out_for_delivery").await;
    let d = open_id(&app, &decided).await;
    respond(&app, &decided, &d).await;
    let res = app
        .request(
            Method::POST,
            &format!("/admin/disputes/{d}/decide"),
            Some(json!({ "decision": "full_refund", "note": "Photos confirm the damage" })),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);

    let withdrawn = app.shipped_order("self", "out_for_delivery").await;
    let w = open_id(&app, &withdrawn).await;
    let res = app
        .request(
            Method::POST,
            &format!("/disputes/{w}/withdraw"),
            Some(json!({ "confirm": true })),
            Some(&withdrawn.buyer_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);

    for id in [&d, &w] {
        let res = screen(&app, id, &support).await;
        assert_eq!(code(&res), (403, Some("dispute_closed")), "{}", res.json);
        let res = screen(&app, id, &owner).await;
        assert_eq!(res.status, 200, "{}", res.json);
    }
}

#[sqlx::test]
async fn screen_has_no_buyer_contact(pool: PgPool) {
    let app = TestApp::new(pool).await;
    // The first order of an app is buyer 0802 5000000; its address is the
    // helper's: phone 08077777777, BUYER_ADDRESS_LINE and a landmark, in AMAC.
    let o = app.shipped_order("courier", "out_for_delivery").await;
    let owner = app.owner_token().await;
    let support = app.make_support().await;
    let id = open_id(&app, &o).await;

    for token in [&owner, &support] {
        let res = screen(&app, &id, token).await;
        assert_eq!(res.status, 200, "{}", res.json);
        let text = res.json.to_string();
        for phone in [
            "08025000000",
            "+2348025000000",
            "2348025000000",
            "8025000000",
            "08077777777",
            "+2348077777777",
            "2348077777777",
            "8077777777",
        ] {
            assert!(!text.contains(phone), "{phone} leaked: {text}");
        }
        assert!(!text.contains(BUYER_ADDRESS_LINE), "{text}");
        assert!(!text.contains("Opposite the blue gate"), "{text}");
        assert!(!text.contains("Ada Buyer"), "{text}");
        assert_eq!(res.json["delivery_area"], "amac");
    }
}

#[sqlx::test]
async fn every_read_is_audited(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("self", "out_for_delivery").await;
    let owner = app.owner_token().await;
    let support = app.make_support().await;
    let id = open_id(&app, &o).await;

    assert_eq!(screen(&app, &id, &owner).await.status, 200);
    assert_eq!(screen(&app, &id, &support).await.status, 200);
    assert_eq!(audit_count(&app, "admin.dispute_viewed", &id).await, 2);

    let rows: Vec<(Value,)> = sqlx::query_as(
        "SELECT detail FROM audit_log WHERE action = 'admin.dispute_viewed' ORDER BY id",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(rows[0].0, json!({ "dispute_id": id, "originals": true }));
    assert_eq!(rows[1].0, json!({ "dispute_id": id, "originals": false }));

    // A refused read writes nothing.
    sqlx::query(
        "UPDATE disputes SET status = 'withdrawn', withdrawn_at = now() WHERE id = $1::uuid",
    )
    .bind(&id)
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(screen(&app, &id, &support).await.status, 403);
    assert_eq!(audit_count(&app, "admin.dispute_viewed", &id).await, 2);

    let res = get(&app, "/admin/disputes", &owner).await;
    assert_eq!(res.status, 200);
    let viewed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'admin.viewed' \
         AND detail->>'view' = 'disputes'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(viewed, 1);
}

#[sqlx::test]
async fn queue_is_oldest_first_and_filters(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let support = app.make_support().await;
    let first = app.shipped_order("self", "out_for_delivery").await;
    let second = app.shipped_order("self", "out_for_delivery").await;
    let third = app.shipped_order("self", "out_for_delivery").await;
    let a = open_id(&app, &first).await;
    let b = open_id(&app, &second).await;
    let c = open_id(&app, &third).await;
    respond(&app, &second, &b).await;
    // Opened order is the age order, whatever the ids are.
    sqlx::query("UPDATE disputes SET opened_at = now() - interval '3 hours' WHERE id = $1::uuid")
        .bind(&b)
        .execute(&app.pool)
        .await
        .unwrap();

    let ids = |res: &TestResponse| -> Vec<String> {
        res.json["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect()
    };
    let res = get(&app, "/admin/disputes", &support).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(ids(&res), vec![b.clone(), a.clone(), c.clone()]);
    let row = &res.json["items"][0];
    assert_eq!(row["order_id"], second.order_id.to_string());
    assert_eq!(row["status"], "awaiting_decision");
    assert_eq!(row["reason"], "damaged");
    assert_eq!(row["items_total_kobo"], 10_000);
    assert!(row["opened_at"].is_string() && row["seller_respond_by"].is_string());

    let res = get(&app, "/admin/disputes?status=awaiting_seller", &owner).await;
    assert_eq!(ids(&res), vec![a, c]);
    let res = get(
        &app,
        "/admin/disputes?status=awaiting_decision&limit=1",
        &owner,
    )
    .await;
    assert_eq!(ids(&res), vec![b]);
    let res = get(&app, "/admin/disputes?status=decided", &owner).await;
    assert!(ids(&res).is_empty());
    let res = get(&app, "/admin/disputes?status=bogus", &owner).await;
    assert_eq!(res.status, 422, "{}", res.json);
    let res = get(&app, "/admin/disputes?limit=0", &owner).await;
    assert_eq!(res.status, 422, "{}", res.json);
}

#[sqlx::test]
async fn buyers_and_sellers_get_403(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("self", "out_for_delivery").await;
    let id = open_id(&app, &o).await;
    for token in [&o.buyer_token, &o.seller_token] {
        let res = get(&app, "/admin/disputes", token).await;
        assert_eq!(res.status, 403, "{}", res.json);
        let res = screen(&app, &id, token).await;
        assert_eq!(res.status, 403, "{}", res.json);
    }
    let res = get(&app, "/admin/disputes", "not-a-token").await;
    assert_eq!(res.status, 401);
    assert_eq!(audit_count(&app, "admin.dispute_viewed", &id).await, 0);

    let owner = app.owner_token().await;
    let res = screen(&app, &Uuid::new_v4().to_string(), &owner).await;
    assert_eq!(res.status, 404);
    let res = screen(&app, "not-an-id", &owner).await;
    assert_eq!(res.status, 404);
}
