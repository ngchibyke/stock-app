#![allow(dead_code)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Method, Request, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use server::{
    AppState,
    auth::otp::OtpSender,
    build_app,
    crypto::FieldCipher,
    error::AppError,
    payments::{ProviderRegistry, VerifyGate, fake::FakeProvider},
    phone::normalize_ng_phone,
};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

pub const TEST_FIELD_KEY: [u8; 32] = [7u8; 32];
pub const TEST_INDEX_KEY: [u8; 32] = [9u8; 32];
/// The secret the test partner signs its webhooks with.
pub const PARTNER_WEBHOOK_SECRET: &str = "partner-secret-for-tests-1";
pub const TEST_NIN_KEY: [u8; 32] = [11u8; 32];
pub const TEST_PASSWORD: &str = "correct-horse-1";
pub const TEST_REDIRECT_URL: &str = "http://localhost:8080/checkout/return";
/// The business phone of stores made by `make_active_store_with_product`.
pub const STORE_PHONE: &str = "08031234567";
/// The address line `buyer_address` saves (personal data: tests check it is never
/// stored or shown in plaintext where it should not be).
pub const BUYER_ADDRESS_LINE: &str = "4 Quiet Close, Maitama";

/// A published product with one variant in an active store; see
/// [`TestApp::make_active_store_with_product`].
pub struct Listing {
    pub seller_id: Uuid,
    pub seller_token: String,
    pub store_id: Uuid,
    pub product_id: Uuid,
    pub variant_id: Uuid,
}

/// Captures the last OTP code sent to each (E.164) phone number.
#[derive(Default)]
pub struct RecordingOtpSender {
    codes: Mutex<HashMap<String, String>>,
    sent: Mutex<usize>,
}

impl RecordingOtpSender {
    pub fn sent_count(&self) -> usize {
        *self.sent.lock().unwrap()
    }

    pub fn last_code(&self, phone_e164: &str) -> Option<String> {
        self.codes.lock().unwrap().get(phone_e164).cloned()
    }
}

#[async_trait]
impl OtpSender for RecordingOtpSender {
    async fn send(&self, phone_e164: &str, code: &str) -> Result<(), AppError> {
        self.codes
            .lock()
            .unwrap()
            .insert(phone_e164.to_string(), code.to_string());
        *self.sent.lock().unwrap() += 1;
        Ok(())
    }
}

pub struct TestResponse {
    pub status: u16,
    pub json: Value,
    pub headers: axum::http::HeaderMap,
}

pub struct TestApp {
    router: Router,
    pub state: AppState,
    pub pool: PgPool,
    pub otp: Arc<RecordingOtpSender>,
    /// The registered payment provider (name "fake", method "mono_directpay").
    pub fake: Arc<FakeProvider>,
    /// The installed courier provider (always a fake).
    pub courier: Arc<server::couriers::fake::FakeCourier>,
    /// A fake statement source. Not installed until [`TestApp::use_statement`]: with one
    /// installed, the statement job runs on every tick.
    pub statement: Arc<server::statement::fake::FakeStatement>,
    /// The installed tone check (always a fake; answers clean by default).
    pub tone: Arc<server::moderation::fake::FakeTone>,
    /// The push double the state sends through.
    pub push: Arc<server::notify::fake::FakePush>,
    /// The identity provider double the state uses.
    pub identity: Arc<server::identity::fake::FakeIdentity>,
    /// The partner double behind the gateway in `state.partner`.
    pub partner: Arc<server::partners::fake::FakePartner>,
    /// The photo store the state uses.
    pub objects: Arc<server::uploads::fake::FakeStore>,
    /// Numbers `shipped_order`'s sellers and buyers, so it can run many times.
    shipped_seq: std::sync::atomic::AtomicUsize,
}

impl TestApp {
    pub async fn new(pool: PgPool) -> Self {
        Self::with_verify_gate(pool, VerifyGate::default()).await
    }

    /// Like [`new`](Self::new) with a custom payment-verify gate (e.g. no cooldown,
    /// so concurrent settle paths can race each other).
    pub async fn with_verify_gate(pool: PgPool, gate: VerifyGate) -> Self {
        let otp = Arc::new(RecordingOtpSender::default());
        let fake = Arc::new(FakeProvider::new());
        let mut providers = ProviderRegistry::new();
        providers.register(fake.clone());
        let courier = Arc::new(server::couriers::fake::FakeCourier::new());
        let tone = Arc::new(server::moderation::fake::FakeTone::new());
        let push = Arc::new(server::notify::fake::FakePush::new());
        let objects = Arc::new(server::uploads::fake::FakeStore::new());
        let statement = Arc::new(server::statement::fake::FakeStatement::new());
        let identity = Arc::new(server::identity::fake::FakeIdentity::new());
        let partner_api = Arc::new(server::partners::fake::FakePartner::new());
        let partner = Arc::new(server::partners::PartnerGateway::new(
            partner_api.clone(),
            pool.clone(),
            Some(server::payments::mono::Secret::new(PARTNER_WEBHOOK_SECRET)),
        ));
        let state = AppState {
            db: pool.clone(),
            cipher: Arc::new(FieldCipher::new(TEST_FIELD_KEY)),
            index_key: Arc::new(TEST_INDEX_KEY),
            otp: otp.clone(),
            providers: Arc::new(providers),
            payment_redirect_url: TEST_REDIRECT_URL.into(),
            verify_gate: Arc::new(gate),
            postcodes: Arc::new(server::delivery::postcode::ManualPostcode),
            courier: Some(courier.clone()),
            tone: tone.clone(),
            push: push.clone(),
            objects: objects.clone(),
            chat_hub: Arc::new(server::chat::hub::ChatHub::with_timings(
                server::chat::hub::SocketTimings {
                    ping: std::time::Duration::from_secs(30),
                    session_check: std::time::Duration::from_millis(200),
                },
            )),
            statements: None,
            app_config: Arc::new(server::app_config::AppClientConfig::default()),
            market: Arc::new(
                server::market::layout::Layout::load(
                    &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("market"),
                )
                .expect("the shipped market layout loads"),
            ),
            identity: Some(identity.clone()),
            nin_key: Some(Arc::new(TEST_NIN_KEY)),
            partner,
        };
        Self {
            router: build_app(state.clone()),
            state,
            pool,
            otp,
            fake,
            courier,
            tone,
            push,
            objects,
            statement,
            identity,
            partner: partner_api,
            shipped_seq: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Like [`new`](Self::new) but sends sign-up and reset codes through `sender`
    /// (for example an `SmsOtpSender` over a fake provider) instead of the recorder.
    pub async fn with_otp_sender(pool: PgPool, sender: Arc<dyn OtpSender>) -> Self {
        let mut app = Self::new(pool).await;
        app.state.otp = sender;
        app.router = build_app(app.state.clone());
        app
    }

    /// Replaces what `GET /app/config` answers.
    /// Replaces the payment providers (for example with one that talks to a local mock) and
    /// rebuilds the router around them.
    pub fn use_providers(&mut self, providers: server::payments::ProviderRegistry) {
        self.state.providers = Arc::new(providers);
        self.router = build_app(self.state.clone());
    }

    pub fn use_app_config(&mut self, config: server::app_config::AppClientConfig) {
        self.state.app_config = Arc::new(config);
        self.router = build_app(self.state.clone());
    }

    /// Runs the app without the identity provider and/or its key.
    pub fn disable_identity(&mut self, provider: bool, key: bool) {
        if provider {
            self.state.identity = None;
        }
        if key {
            self.state.nin_key = None;
        }
        self.router = build_app(self.state.clone());
    }

    /// Installs [`statement`](Self::statement) as the app's statement source.
    pub fn use_statement(&mut self) {
        self.state.statements = Some(self.statement.clone());
        self.router = build_app(self.state.clone());
    }

    /// Runs the app with no public photo store, so no photo has an address.
    pub fn use_no_public_photos(&mut self) {
        self.state.objects = Arc::new(server::uploads::store::NoStore);
        self.router = build_app(self.state.clone());
    }

    /// Serves the app on a real local port, for WebSocket tests.
    pub async fn spawn_server(&self) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let router = self.router.clone();
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serve");
        });
        addr
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        token: Option<&str>,
    ) -> TestResponse {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        self.send(builder, body).await
    }

    /// Like [`request`](Self::request), with extra headers.
    pub async fn request_with_headers(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        token: Option<&str>,
        headers: &[(&str, &str)],
    ) -> TestResponse {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        self.send(builder, body).await
    }

    async fn send(
        &self,
        builder: axum::http::request::Builder,
        body: Option<Value>,
    ) -> TestResponse {
        let req = match body {
            Some(json) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json.to_string())),
            None => builder.body(Body::empty()),
        }
        .expect("valid request");

        let res = self.router.clone().oneshot(req).await.expect("infallible");
        let status = res.status().as_u16();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.expect("body").to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        TestResponse {
            status,
            json,
            headers,
        }
    }

    /// Sends raw bytes with the given headers (no content type unless given).
    pub async fn send_raw(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> TestResponse {
        let mut builder = Request::builder().method(method).uri(path);
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        let req = builder.body(Body::from(body)).expect("valid request");
        let res = self.router.clone().oneshot(req).await.expect("infallible");
        let status = res.status().as_u16();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.expect("body").to_bytes();
        TestResponse {
            status,
            json: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            headers,
        }
    }

    /// Sends a raw (possibly non-JSON) body with a JSON content type.
    pub async fn request_raw(&self, method: Method, path: &str, body: &str) -> TestResponse {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("valid request");
        let res = self.router.clone().oneshot(req).await.expect("infallible");
        let status = res.status().as_u16();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.expect("body").to_bytes();
        TestResponse {
            status,
            json: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            headers,
        }
    }

    /// `mark_delivered` for tests of what happens after delivery. The service
    /// refuses self-delivery orders (the default checkout here), so the order
    /// becomes a part-2 one, with no delivery method, first.
    pub async fn mark_delivered(
        &self,
        order_id: Uuid,
        at: time::OffsetDateTime,
    ) -> Result<(), server::error::AppError> {
        sqlx::query(
            "UPDATE orders SET delivery_method = NULL \
             WHERE id = $1 AND delivery_method = 'self'",
        )
        .bind(order_id)
        .execute(&self.pool)
        .await
        .unwrap();
        server::orders::service::mark_delivered(&self.pool, order_id, at).await
    }

    /// Makes outstanding OTPs look older so the issue throttle window passes.
    pub async fn age_otps(&self, secs: i32) {
        sqlx::query(
            "UPDATE otp_codes SET created_at = created_at - make_interval(secs => $1), \
                                  sent_at = sent_at - make_interval(secs => $1)",
        )
        .bind(f64::from(secs))
        .execute(&self.pool)
        .await
        .unwrap();
    }

    /// The last OTP code sent to `phone` (any accepted format).
    pub fn last_code(&self, phone: &str) -> String {
        let e164 = normalize_ng_phone(phone).expect("valid test phone");
        self.otp.last_code(&e164).expect("an OTP was sent")
    }

    pub async fn sign_up(&self, phone: &str, password: &str) -> TestResponse {
        self.request(
            Method::POST,
            "/auth/sign-up",
            Some(json!({ "phone": phone, "password": password, "display_name": "Ada" })),
            None,
        )
        .await
    }

    pub async fn sign_in(&self, phone: &str, password: &str) -> TestResponse {
        self.request(
            Method::POST,
            "/auth/sign-in",
            Some(json!({ "phone": phone, "password": password })),
            None,
        )
        .await
    }

    /// POST /auth/verify-phone with a verification id and code.
    pub async fn verify(&self, verification_id: &Value, code: &str) -> TestResponse {
        self.request(
            Method::POST,
            "/auth/verify-phone",
            Some(json!({ "verification_id": verification_id, "code": code })),
            None,
        )
        .await
    }

    /// Signs up, verifies the phone with the emitted OTP, returns (user id, session token).
    pub async fn sign_up_verified(&self, phone: &str) -> (Uuid, String) {
        let res = self.sign_up(phone, TEST_PASSWORD).await;
        assert_eq!(res.status, 201, "sign-up failed: {}", res.json);
        let user_id: Uuid = res.json["user_id"].as_str().unwrap().parse().unwrap();
        let verify = self
            .verify(&res.json["verification_id"], &self.last_code(phone))
            .await;
        assert_eq!(verify.status, 200, "verify failed: {}", verify.json);
        (user_id, verify.json["token"].as_str().unwrap().to_string())
    }

    /// Creates a verified user, promotes it to owner, returns its session token.
    pub async fn make_owner(&self) -> String {
        let (user_id, token) = self.sign_up_verified("08099999999").await;
        sqlx::query("UPDATE users SET role = 'owner' WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .expect("promote to owner");
        token
    }

    /// Creates a verified user, promotes it to support, returns its session token.
    pub async fn make_support(&self) -> String {
        let (user_id, token) = self.sign_up_verified("08088888888").await;
        sqlx::query("UPDATE users SET role = 'support' WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .expect("promote to support");
        token
    }

    /// Owner creates an access code; returns (code id, plain code).
    pub async fn create_access_code(&self, owner_token: &str, body: Value) -> (Uuid, String) {
        let res = self
            .request(
                Method::POST,
                "/admin/access-codes",
                Some(body),
                Some(owner_token),
            )
            .await;
        assert_eq!(res.status, 201, "create code failed: {}", res.json);
        (
            res.json["id"].as_str().unwrap().parse().unwrap(),
            res.json["code"].as_str().unwrap().to_string(),
        )
    }

    pub async fn redeem(&self, token: &str, code: &str) -> TestResponse {
        self.request(
            Method::POST,
            "/seller/redeem",
            Some(json!({ "code": code })),
            Some(token),
        )
        .await
    }

    /// The owner's session token: signs in if the owner already exists, else creates it.
    pub async fn owner_token(&self) -> String {
        let res = self
            .request(
                Method::POST,
                "/auth/sign-in",
                Some(json!({ "phone": "08099999999", "password": TEST_PASSWORD })),
                None,
            )
            .await;
        if res.status == 200 {
            res.json["token"].as_str().unwrap().to_string()
        } else {
            self.make_owner().await
        }
    }

    /// Verified user who has redeemed an owner-issued code; returns (user id, token).
    pub async fn make_seller(&self, phone: &str) -> (Uuid, String) {
        let owner = self.owner_token().await;
        let (_, code) = self.create_access_code(&owner, json!({})).await;
        let (id, token) = self.sign_up_verified(phone).await;
        let res = self.redeem(&token, &code).await;
        assert_eq!(res.status, 200, "redeem failed: {}", res.json);
        // Sellers in these tests have passed the identity check, so earlier suites keep
        // their meaning. The identity suites make their own sellers.
        self.pass_identity(id).await;
        (id, token)
    }

    /// Signs up and verifies a phone, as [`sign_up_verified`](Self::sign_up_verified), and
    /// the person has passed the identity check (a team member who may act on orders).
    pub async fn sign_up_cleared(&self, phone: &str) -> (Uuid, String) {
        let (id, token) = self.sign_up_verified(phone).await;
        self.pass_identity(id).await;
        (id, token)
    }

    /// Marks `user` as having passed the identity check (a stand-in fingerprint, no NIN).
    pub async fn pass_identity(&self, user: Uuid) {
        sqlx::query(
            "INSERT INTO identity_checks (user_id, provider, status, nin_idx, full_name_enc, \
                                          birth_year, decided_at) \
             VALUES ($1, 'fake', 'passed', sha256($1::text::bytea), $2, 1990, now())",
        )
        .bind(user)
        .bind(self.state.cipher.encrypt("Ada Obi"))
        .execute(&self.pool)
        .await
        .expect("pass identity");
    }

    /// Seller (`phone`) with an active store named `store_name`, activated the real
    /// way (a provider-verified payout account), holding one published product with
    /// a single variant at `price_kobo` and `stock`.
    pub async fn make_active_store_with_product(
        &self,
        phone: &str,
        store_name: &str,
        price_kobo: i64,
        stock: i32,
    ) -> Listing {
        let (seller_id, seller_token) = self.make_seller(phone).await;
        let res = self
            .request(
                Method::POST,
                "/stores",
                Some(json!({
                    "name": store_name,
                    "pickup_address": "12 Unique Street, Wuse 2",
                    "district": "everything-else",
                    "pickup_state": "FCT",
                    // Both methods: self-delivery at fee 0 is what `checkout` picks by
                    // default, so part-2 tests keep their meaning (ruling P4).
                    "delivery_courier": true,
                    "delivery_self": true,
                    "pickup_area_council": "amac",
                    "business_phone": STORE_PHONE,
                    "self_delivery_fee_kobo": 0
                })),
                Some(&seller_token),
            )
            .await;
        assert_eq!(res.status, 201, "create store: {}", res.json);
        let store_id: Uuid = res.json["id"].as_str().unwrap().parse().unwrap();

        sqlx::query("UPDATE platform_settings SET payout_provider = 'fake'")
            .execute(&self.pool)
            .await
            .unwrap();
        // Sellers sign up as "Ada"; the resolved account name must match it.
        self.fake.set_lookup(Ok("ADA OBI".into()));
        let res = self
            .request(
                Method::PUT,
                &format!("/stores/{store_id}/payout-account"),
                Some(json!({
                    "bank_code": "058",
                    "account_number": "0123456785",
                    "password": TEST_PASSWORD
                })),
                Some(&seller_token),
            )
            .await;
        assert_eq!(res.status, 200, "payout account: {}", res.json);
        assert_eq!(res.json["status"], "verified", "{}", res.json);

        let product_id = self
            .add_product(&seller_token, store_id, price_kobo, &[(json!({}), stock)])
            .await;
        let variant_id = self.variant_ids(product_id).await[0];
        Listing {
            seller_id,
            seller_token,
            store_id,
            product_id,
            variant_id,
        }
    }

    /// A finished upload owned by `user` for `purpose` (`product`, `review`,
    /// `dispute`, ...), as if the app had uploaded and finalized a photo; returns
    /// its key. Use it wherever a request needs a photo key it is allowed to attach.
    pub async fn ready_photo(&self, user: Uuid, purpose: &str) -> String {
        let key: String = sqlx::query_scalar(
            "INSERT INTO uploads (user_id, purpose, raw_key, key, declared_type, declared_bytes,                                   status, finalized_at)              VALUES ($1, $2, 'raw/' || gen_random_uuid()::text,                      'u/' || $2 || '/' || gen_random_uuid()::text || '.jpg',                      'image/jpeg', 1000, 'ready', now()) RETURNING key",
        )
        .bind(user)
        .bind(purpose)
        .fetch_one(&self.pool)
        .await
        .expect("insert ready upload");
        let bucket = if purpose == "dispute" {
            server::uploads::store::Bucket::Private
        } else {
            server::uploads::store::Bucket::Public
        };
        self.objects
            .put_raw(bucket, &key, vec![0xFF, 0xD8, 0xFF], "image/jpeg");
        key
    }

    /// `body` with each made-up entry of `photo_keys` (anything not already a
    /// real `u/...` key, and not blank) swapped for a ready photo of `purpose`
    /// owned by the person holding `token`. Lets a fixture keep a readable
    /// placeholder such as `"photos/a.jpg"`.
    pub async fn ready_photos_in(&self, token: &str, purpose: &str, mut body: Value) -> Value {
        let count = body["photo_keys"].as_array().map_or(0, Vec::len);
        for i in 0..count {
            let key = body["photo_keys"][i].as_str().unwrap_or("").to_string();
            if !key.trim().is_empty() && !key.starts_with("u/") {
                body["photo_keys"][i] = json!(self.ready_photo_for(token, purpose).await);
            }
        }
        body
    }

    /// [`ready_photo`](Self::ready_photo) for the person holding `token`.
    pub async fn ready_photo_for(&self, token: &str, purpose: &str) -> String {
        let me = self.request(Method::GET, "/me", None, Some(token)).await;
        assert_eq!(me.status, 200, "me: {}", me.json);
        let user: Uuid = me.json["id"].as_str().unwrap().parse().unwrap();
        self.ready_photo(user, purpose).await
    }

    /// Adds a published product to `store_id` with the given (attributes, stock)
    /// variants at `price_kobo`; returns the product id.
    pub async fn add_product(
        &self,
        seller_token: &str,
        store_id: Uuid,
        price_kobo: i64,
        variants: &[(Value, i32)],
    ) -> Uuid {
        let variants: Vec<Value> = variants
            .iter()
            .map(|(attrs, stock)| json!({ "attributes": attrs, "stock": stock }))
            .collect();
        let photo = self.ready_photo_for(seller_token, "product").await;
        let res = self
            .request(
                Method::POST,
                &format!("/stores/{store_id}/products"),
                Some(json!({
                    "name": "Ankara Shirt",
                    "description": "Handmade shirt",
                    "category": "fashion",
                    "condition": "new",
                    "photo_keys": [photo],
                    "weight_grams": 300,
                    "base_price_kobo": price_kobo,
                    "variants": variants
                })),
                Some(seller_token),
            )
            .await;
        assert_eq!(res.status, 201, "create product: {}", res.json);
        let product_id: Uuid = res.json["id"].as_str().unwrap().parse().unwrap();
        let res = self
            .request(
                Method::PATCH,
                &format!("/products/{product_id}"),
                Some(json!({ "is_published": true })),
                Some(seller_token),
            )
            .await;
        assert_eq!(res.status, 200, "publish: {}", res.json);
        product_id
    }

    /// The product's variant ids, ordered by id.
    pub async fn variant_ids(&self, product_id: Uuid) -> Vec<Uuid> {
        sqlx::query_scalar("SELECT id FROM product_variants WHERE product_id = $1 ORDER BY id")
            .bind(product_id)
            .fetch_all(&self.pool)
            .await
            .unwrap()
    }

    /// Units held by pending checkouts.
    pub async fn reserved(&self, variant_id: Uuid) -> i32 {
        sqlx::query_scalar("SELECT reserved_quantity FROM product_variants WHERE id = $1")
            .bind(variant_id)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    /// Units a buyer can still buy (`stock - reserved_quantity`).
    pub async fn available(&self, variant_id: Uuid) -> i32 {
        self.stock(variant_id).await - self.reserved(variant_id).await
    }

    /// Sets a variant's on-hand and reserved counts at its store's first branch
    /// through the stock module (the only writer, which keeps the variant totals in
    /// step), as a seller's recount and pending checkouts would leave them. A fixture
    /// that inserts variants directly inserts them with stock 0, then calls this.
    pub async fn set_counts(&self, variant_id: Uuid, stock: i32, reserved: i32) {
        let branch: Uuid = sqlx::query_scalar(
            "SELECT b.id FROM product_variants v JOIN products p ON p.id = v.product_id \
             JOIN store_branches b ON b.store_id = p.store_id AND b.position = 0 \
             WHERE v.id = $1",
        )
        .bind(variant_id)
        .fetch_one(&self.pool)
        .await
        .unwrap();
        let mut tx = self.pool.begin().await.unwrap();
        let now = server::stock::lock_counts(&mut tx, branch, variant_id)
            .await
            .unwrap();
        server::stock::apply(&mut tx, branch, variant_id, 0, -now.reserved)
            .await
            .unwrap();
        server::stock::set_stock(&mut tx, branch, variant_id, stock)
            .await
            .unwrap();
        server::stock::apply(&mut tx, branch, variant_id, 0, reserved)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    /// On-hand stock.
    pub async fn stock(&self, variant_id: Uuid) -> i32 {
        sqlx::query_scalar("SELECT stock FROM product_variants WHERE id = $1")
            .bind(variant_id)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    /// PUT /cart/items.
    pub async fn set_cart(&self, token: &str, variant_id: Uuid, quantity: i64) -> TestResponse {
        self.request(
            Method::PUT,
            "/cart/items",
            Some(json!({ "variant_id": variant_id, "quantity": quantity })),
            Some(token),
        )
        .await
    }

    /// The buyer's first saved address, or a new one in AMAC; returns its id.
    pub async fn buyer_address(&self, token: &str) -> Uuid {
        let list = self
            .request(Method::GET, "/me/addresses", None, Some(token))
            .await;
        assert_eq!(list.status, 200, "list addresses: {}", list.json);
        if let Some(first) = list.json.as_array().and_then(|a| a.first()) {
            return first["id"].as_str().unwrap().parse().unwrap();
        }
        self.add_address(token, "amac").await
    }

    /// Saves a new address in `area` for the buyer; returns its id.
    pub async fn add_address(&self, token: &str, area: &str) -> Uuid {
        let res = self
            .request(
                Method::POST,
                "/me/addresses",
                Some(json!({
                    "recipient_name": "Ada Buyer",
                    "phone": "08077777777",
                    "address_line": BUYER_ADDRESS_LINE,
                    "landmark": "Opposite the blue gate",
                    "area_council": area
                })),
                Some(token),
            )
            .await;
        assert_eq!(res.status, 201, "add address: {}", res.json);
        res.json["id"].as_str().unwrap().parse().unwrap()
    }

    /// POST /checkout/delivery-options for `address_id`.
    pub async fn delivery_options(&self, token: &str, address_id: Uuid) -> TestResponse {
        self.request(
            Method::POST,
            "/checkout/delivery-options",
            Some(json!({ "address_id": address_id })),
            Some(token),
        )
        .await
    }

    /// The default checkout body (ruling P4): the buyer's address and, for every
    /// branch group of every store in the cart, its self-delivery option (fee 0 on
    /// stores made by `make_active_store_with_product`). For an empty cart `groups` is
    /// empty, so checkout answers with its own `cart_empty`; any other quote failure
    /// fails the test here, where its cause is visible.
    pub async fn checkout_body(&self, token: &str, method: &str) -> Value {
        let address_id = self.buyer_address(token).await;
        let res = self.delivery_options(token, address_id).await;
        assert!(
            res.status == 200 || res.json["error"]["code"] == "cart_empty",
            "delivery options: {} {}",
            res.status,
            res.json
        );
        let mut groups = Vec::new();
        if res.status == 200 {
            for store in res.json["stores"].as_array().unwrap() {
                for group in store["groups"].as_array().unwrap() {
                    let own = group["options"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|o| o["method"] == "self");
                    if let Some(option) = own {
                        groups.push(json!({
                            "group_id": group["group_id"],
                            "option_id": option["id"],
                        }));
                    }
                }
            }
        }
        json!({ "method": method, "address_id": address_id, "groups": groups })
    }

    /// POST /checkout with `method`, the buyer's address and self-delivery for
    /// every group (see [`checkout_body`](Self::checkout_body)).
    pub async fn checkout(&self, token: &str, method: &str) -> TestResponse {
        let body = self.checkout_body(token, method).await;
        self.request(Method::POST, "/checkout", Some(body), Some(token))
            .await
    }

    /// POST /checkout with explicit choices: `groups` is `[(group_id, option_id)]`.
    pub async fn checkout_with(
        &self,
        token: &str,
        method: &str,
        address_id: Uuid,
        groups: &[(String, Uuid)],
    ) -> TestResponse {
        let groups: Vec<Value> = groups
            .iter()
            .map(|(g, o)| json!({ "group_id": g, "option_id": o }))
            .collect();
        self.request(
            Method::POST,
            "/checkout",
            Some(json!({ "method": method, "address_id": address_id, "groups": groups })),
            Some(token),
        )
        .await
    }
}

/// The first store's first group in a delivery-options answer, and that group's
/// first option of `method`: `(group_id, option_id)`.
pub fn group_option(json: &Value, method: &str) -> (String, Uuid) {
    let group = &json["stores"][0]["groups"][0];
    let option = group["options"]
        .as_array()
        .unwrap_or_else(|| panic!("no group in {json}"))
        .iter()
        .find(|o| o["method"] == method)
        .unwrap_or_else(|| panic!("no {method} option in {json}"));
    (
        group["group_id"].as_str().unwrap().to_string(),
        option["id"].as_str().unwrap().parse().unwrap(),
    )
}

/// The first store's first group's options in a delivery-options answer.
pub fn first_group_options(json: &Value) -> &Vec<Value> {
    json["stores"][0]["groups"][0]["options"]
        .as_array()
        .unwrap_or_else(|| panic!("no group in {json}"))
}

/// A paid order moved on to `picked_up`, `out_for_delivery` or `delivered`;
/// see [`TestApp::shipped_order`].
pub struct ShippedOrder {
    pub order_id: Uuid,
    pub checkout_id: Uuid,
    pub store_id: Uuid,
    pub buyer_id: Uuid,
    pub buyer_token: String,
    pub seller_id: Uuid,
    pub seller_token: String,
}

impl TestApp {
    /// One paid order (a single ₦10,000 unit) delivered by `method` (`"self"` or
    /// `"courier"`), moved through the service to `status` (`"picked_up"`,
    /// `"out_for_delivery"` or `"delivered"`). The steps go through the order
    /// service, never the routes, so a self-delivery order reaches `delivered`
    /// without being released (the handover route releases it). The payment is
    /// 10 days old, so a release sends its payout at once. Each call creates its
    /// own seller, store and buyer (distinct phones), so it can be called any
    /// number of times in one app.
    pub async fn shipped_order(&self, method: &str, status: &str) -> ShippedOrder {
        use server::{checkout::settle_payment, orders::service, payments::PaymentStatus};
        let now = time::OffsetDateTime::now_utc();
        let n = self
            .shipped_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let listing = self
            .make_active_store_with_product(
                &format!("0801{:07}", 5_000_000 + n),
                &format!("Shipped Store {n}"),
                10_000,
                5,
            )
            .await;
        let (buyer_id, buyer_token) = self
            .sign_up_verified(&format!("0802{:07}", 5_000_000 + n))
            .await;
        assert_eq!(
            self.set_cart(&buyer_token, listing.variant_id, 1)
                .await
                .status,
            200
        );
        let res = if method == "courier" {
            let address = self.buyer_address(&buyer_token).await;
            let opts = self.delivery_options(&buyer_token, address).await;
            assert_eq!(opts.status, 200, "delivery options: {}", opts.json);
            let choice = group_option(&opts.json, "courier");
            self.checkout_with(&buyer_token, "mono_directpay", address, &[choice])
                .await
        } else {
            assert_eq!(method, "self", "unknown delivery method");
            self.checkout(&buyer_token, "mono_directpay").await
        };
        assert_eq!(res.status, 201, "checkout: {}", res.json);
        let reference = res.json["reference"].as_str().unwrap().to_string();
        let total = res.json["total_kobo"].as_i64().unwrap();
        let checkout_id: Uuid = res.json["checkout_id"].as_str().unwrap().parse().unwrap();
        self.fake.set_payment(
            &reference,
            PaymentStatus::Successful {
                amount_kobo: total,
                fee_kobo: 10,
                currency: "NGN".into(),
                held: false,
                reference: reference.clone(),
            },
        );
        settle_payment(&self.state, &reference).await.unwrap();
        let order_id: Uuid = sqlx::query_scalar("SELECT id FROM orders WHERE checkout_id = $1")
            .bind(checkout_id)
            .fetch_one(&self.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE checkouts SET paid_at = now() - interval '10 days' WHERE id = $1")
            .bind(checkout_id)
            .execute(&self.pool)
            .await
            .unwrap();

        service::mark_picked_up(&self.pool, order_id, now)
            .await
            .unwrap();
        if status != "picked_up" {
            let mut tx = self.pool.begin().await.unwrap();
            service::mark_out_for_delivery(&mut tx, &self.state.cipher, order_id, now)
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }
        match status {
            "picked_up" | "out_for_delivery" => {}
            "delivered" if method == "self" => {
                let enc: Vec<u8> =
                    sqlx::query_scalar("SELECT handover_code_enc FROM orders WHERE id = $1")
                        .bind(order_id)
                        .fetch_one(&self.pool)
                        .await
                        .unwrap();
                let code = self.state.cipher.decrypt(&enc).unwrap();
                service::deliver_with_code(
                    &self.pool,
                    &self.state.cipher,
                    listing.seller_id,
                    order_id,
                    &code,
                    now,
                )
                .await
                .unwrap();
            }
            "delivered" => service::mark_delivered(&self.pool, order_id, now)
                .await
                .unwrap(),
            other => panic!("unknown order status {other}"),
        }
        ShippedOrder {
            order_id,
            checkout_id,
            store_id: listing.store_id,
            buyer_id,
            buyer_token,
            seller_id: listing.seller_id,
            seller_token: listing.seller_token,
        }
    }
}
