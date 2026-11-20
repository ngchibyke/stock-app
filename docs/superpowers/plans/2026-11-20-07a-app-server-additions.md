# Part 7a: Server Additions for the App Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use the subagent-driven or inline plan-execution workflow, task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Everything the mobile app needs from the server that does not exist yet (see the gap table in `2026-11-20-07-mobile-app.md`). When this plan is done, a phone can sign up with a real SMS code, recover a forgotten password, browse and search a catalog, and a seller can put photos on products, stores, reviews and disputes, with every photo checked to belong to the person who uploaded it. This plan adds:
- fixed **categories** and a public `GET /categories`;
- a public **catalog** (`GET /products` with search, filters and sorting) and a **store directory** (`GET /stores`);
- **photo upload**: presigned uploads to S3-compatible storage, a server-side finalize step that re-encodes the image (dropping EXIF and GPS), ownership checks wherever a photo key is accepted, public URLs for public photos and short-lived signed URLs for private dispute evidence;
- **password reset** by phone code;
- a real **SMS sender** behind the existing `OtpSender` trait, and a fix for the throttled-resend gap carried from plan 1;
- a small public **app config** route (minimum versions, links, maintenance notice).

**Architecture:** Two new modules. `catalog` is read-only SQL over the existing products, variants and stores. `uploads` owns the `uploads` table and an `ObjectStore` trait (an `S3Store` adapter, a `FakeStore` test double and a `NoStore`), following the `ToneChecker` and `PushSender` pattern: config builds the adapter, `AppState` carries it, tests inject the fake. A photo is accepted by a resource only after `uploads::service::claim` proves it is `ready`, uploaded by the caller, for the right purpose, and not attached elsewhere. Password reset reuses the OTP tables with a `purpose`. **Never make a network call inside a database transaction**: object-store calls happen before or after the transaction, as provider calls do elsewhere.

**Tech Stack:** Rust, axum 0.8, sqlx 0.8 on Postgres 17, tokio, reqwest 0.12 (rustls), wiremock. New: the `image` crate (decoding and re-encoding, with explicit limits), and the `pg_trgm` Postgres extension. S3 presigning is written by hand with `hmac` and `sha2` (already dependencies), so no AWS SDK is added.

**Spec and context:** `docs/superpowers/specs/2026-10-22-abuja-marketplace-design.md` §3 (accounts), §4 (stores and products), §9 (reviews), §13 (data protection); `docs/superpowers/plans/2026-11-20-07-mobile-app.md` (the gap table and decisions D3 to D5); the roadmap's carry-forwards on uploads and OTP.

## Global Constraints

- Codebase conventions apply everywhere, and every task's requirements include this section:
  - errors use the `AppError` JSON envelope;
  - bodies use `AppJson<T>`, query strings use `AppQuery<T>`;
  - path ids are `Path<String>` parsed with `parse_id`, never `Path<Uuid>`;
  - every state change writes to the append-only `audit_log`, and `detail` never holds a phone number, a code, a password, a photo's bytes or a signed URL;
  - personal data is never logged and never put in a URL.
- Lock order: users → stores → products → variants → access_codes → checkouts → orders → disputes → shipments → payouts, then conversations → reviews. `uploads` rows are locked last (after reviews, before `payout_accounts`), and only by the uploads module.
- **Public reads show only what is public:** a product appears in the catalog only when `is_published` and not deleted, in a store that is `active` and not deleted. A suspended store's products disappear from browse and search at once.
- **Categories:** a fixed list in the `categories` table (slug, name, position). Seed list (decision D5): `fashion` "Fashion", `phones-gadgets` "Phones and gadgets", `electronics` "Electronics", `home-kitchen` "Home and kitchen", `beauty-personal-care` "Beauty and personal care", `baby-kids` "Baby and kids", `sports-fitness` "Sports and fitness", `books-stationery` "Books and stationery", `food-groceries` "Food and groceries", `other` "Other". A product's `category` is a slug in this table. Existing free-text values are mapped by lower-casing and matching the slug or the name, else `other`, in the migration.
- **Browse limits:** default page 24, maximum 50; query text at most 80 characters; prices in kobo within `1..=MAX_PRICE_KOBO`; at most 200 results are reachable by relevance paging.
- **Uploads (decision D3 for the provider):**
  - types `image/jpeg`, `image/png`, `image/webp` only (the app converts HEIC to JPEG before upload);
  - raw upload at most **5 MiB**; at most **4096 × 4096 pixels** and **40 million decoded pixels**; decoding runs with explicit limits and a timeout;
  - the finalized image is re-encoded as JPEG (quality 85), longest edge at most **2048 px**, with no metadata carried over;
  - purposes and where they live: public (`product`, `store_logo`, `store_banner`, `review`) in the public bucket under `u/{purpose}/{uuid}.jpg`; **private** (`dispute`) in the private bucket, never reachable by a public URL;
  - at most 30 presigned uploads per user per hour; a presigned URL lives 10 minutes and signs the exact `content-length` and `content-type` the app declared;
  - a `pending` upload older than 24 hours is deleted, with its object, by a job.
- Photo counts stay as they are: product 1 to 10, review `MAX_REVIEW_PHOTOS`, dispute `MAX_DISPUTE_PHOTOS`.
- **Private evidence:** the dispute view returns photo keys, never URLs. `GET /disputes/{id}/photo?key=...` answers `{url, expires_at}` with a signed URL valid 60 seconds, only to the dispute's buyer, the seller of its store, and staff (Owner any dispute, Support open disputes only, as on the staff screen). Every staff read is audited as `admin.dispute_photo_viewed` with the dispute id only.
- **Password reset:** the request answers `202` with the same body whether or not the phone has an account. A code is never sent to an unverified or deleted account. Confirming sets the new password, revokes every session of that user, clears the sign-in lockout, and audits `auth.password_reset` (user id only). New passwords follow the same rules as sign-up.
- **OTP sending:** production refuses to start without a configured SMS sender. Outside production with none set, codes go to the log as today (`LogOtpSender`).
- Tests run from `server/`: first confirm the database is up with `docker ps -a`, and start the existing container with `docker start market-db` if stopped, then `DATABASE_URL=postgres://market:market@localhost:5433/market cargo test`. Run the whole suite at the end of each task, and never two `cargo test` processes at once (their test databases collide). If a build fails strangely (a paging-file error, a linker error), check free disk space first.
- Commits: conventional messages. Never add a `Co-Authored-By` line, a "Generated with" footer, a session link or any other AI attribution.

## Review Focus

1. **An uploaded file is not what it claims.** A PNG that is really a script, a file larger than it declared, a decompression bomb (a tiny file that decodes to gigapixels), a truncated or corrupt image, and a file with GPS EXIF. None may reach a public bucket in a form that keeps hostile content or location data. Tests: `finalize_rejects_oversize_and_bombs`, `finalize_strips_exif`, `finalize_rejects_non_images`, `declared_length_is_signed` (Task 5).
2. **Someone else's photo.** Attaching another user's `ready` key, a `pending` key, a key of the wrong purpose, or a key already on another resource; racing two claims of one key. Tests: `cannot_claim_foreign_pending_or_wrong_purpose_keys`, `concurrent_claims_of_one_key_attach_once` (Task 6).
3. **Private evidence becoming public.** A dispute photo reachable by the public base URL, by a stranger, or by Support on a closed dispute. Tests: `dispute_photos_are_never_public`, `support_cannot_sign_closed_dispute_photo`, `stranger_gets_404_for_dispute_photo` (Task 6).
4. **Catalog leaks and slow queries.** A suspended store, an unpublished product or a deleted one appearing; a search string that breaks the SQL or the cursor; a query that cannot use its index; unstable pages when prices tie. Tests: `hidden_products_never_appear`, `search_text_is_parameterised`, `pages_are_stable_when_prices_tie`, `trigram_index_exists` (Task 2).
5. **Password reset as an attack surface.** Telling an attacker which phones have accounts, guessing the code, using one code twice, resetting a deleted or unverified account, leaving a stolen session alive. Tests: `reset_request_looks_the_same_for_unknown_phones`, `reset_code_is_single_use_and_attempt_limited`, `reset_revokes_every_session`, `deleted_account_cannot_reset` (Task 7).

---

### Task 0: Branch and baseline

- [ ] **Step 1:** Create the branch from up-to-date main: `git fetch origin` then `git switch -c feat/app-server-additions origin/main`. Move this plan onto it with the first commit.
- [ ] **Step 2:** Start the database (`docker start market-db`) and run the full suite. Expected: all pass before any change (1025 passed, 3 ignored at the time of writing, after the dead-port fix). If not, stop and report.

### Task 1: Categories

**Files:**
- Create: `server/migrations/0039_categories.sql`, `server/src/catalog/mod.rs`, `server/src/catalog/categories.rs`, `server/tests/catalog.rs`
- Modify: `server/src/lib.rs`, `server/src/products/service.rs`

**Schema (`0039`):** `categories (slug text PRIMARY KEY CHECK (slug ~ '^[a-z0-9-]{2,40}$'), name text NOT NULL, position int NOT NULL, created_at timestamptz NOT NULL DEFAULT now())`, seeded with the list in Global Constraints. Then map existing `products.category` values (lower-cased, matched to a slug or a name, else `other`), then `ALTER TABLE products ADD CONSTRAINT products_category_fk FOREIGN KEY (category) REFERENCES categories (slug)`. Header comment: no personal data.

**Interfaces:**
- Produces: `GET /categories` (public, no auth) answering `[{slug, name}]` in `position` order; `catalog::categories::exists(conn, slug) -> bool`.
- `create_product` and `update_product` refuse a category not in the table with 422 `unknown_category`.

- [ ] **Step 1: Write the failing tests** in `tests/catalog.rs`:
  - `categories_are_listed_in_order_without_auth`.
  - `product_needs_a_known_category`: create and patch with an unknown slug give 422 `unknown_category`.
  - `migration_maps_existing_categories`: insert a product with the category "Phones" through SQL before the constraint (use a helper that runs the mapping statement on a scratch row), and check it becomes `phones-gadgets`; an unrecognised value becomes `other`.
- [ ] **Step 2: Run them.** `cargo test --test catalog`. Expected: FAIL.
- [ ] **Step 3: Implement the migration, the route and the validation.** Update every existing test fixture that creates a product so it passes a slug (search `"category"` in `server/tests`).
- [ ] **Step 4: Run the whole suite.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add product categories`

### Task 2: Catalog browse and search

**Files:**
- Create: `server/migrations/0040_catalog_search.sql`, `server/src/catalog/browse.rs`
- Modify: `server/src/catalog/mod.rs`, `server/src/lib.rs` (router), `server/tests/catalog.rs`

**Schema (`0040`):** `CREATE EXTENSION IF NOT EXISTS pg_trgm;` and a GIN trigram index on `products` over `lower(name || ' ' || coalesce(brand, '') || ' ' || category)`, partial on `is_published AND deleted_at IS NULL`. If the migration role cannot create the extension, the migration fails with a clear message (a go-live note: the production role needs it, or the extension is pre-created by the database owner).

**Interfaces:**
- `GET /products`, public (`MaybeAuth` only to keep the door open for later personalisation; nothing differs today). Query: `q` (≤ 80 chars), `category` (slug), `condition` (`new|used|refurbished`), `min_price_kobo`, `max_price_kobo`, `store` (slug), `sort` (`newest` default without `q`, `relevance` default with `q`, `price_asc`, `price_desc`), `cursor`, `limit`.
- Response: `{items: [ProductCard], next_cursor: Option<String>}`. `ProductCard { id, name, category, condition, price_from_kobo, photo_key, in_stock, store: { id, slug, name, rating_average, rating_count } }`. `price_from_kobo` is the cheapest of the variants' `price_kobo` (falling back to `base_price_kobo`). `in_stock` is any variant with `stock - reserved_quantity > 0`. (`photo_url` arrives in Task 6.)
- Paging: `newest`, `price_asc` and `price_desc` use keyset cursors over `(sort key, id)`, so equal prices page stably. `relevance` orders by trigram similarity, then `created_at`, then `id`, and pages by an offset inside an opaque cursor, capped at 200 results. A malformed cursor is 422 `bad_cursor`.
- Matching: a product matches `q` when any word matches by `ILIKE` on name, brand or category, or the whole string's trigram similarity passes a threshold (so "sandle" finds "sandal"). All values are bound parameters.

- [ ] **Step 1: Write the failing tests** in `tests/catalog.rs`:
  - `browse_lists_published_products_newest_first`.
  - `hidden_products_never_appear`: unpublished, deleted, and products of a suspended or pending store are absent, in browse and in search.
  - `filters_combine`: category, condition and price range together; a reversed price range is 422.
  - `search_finds_close_spellings_and_brands`.
  - `search_text_is_parameterised`: `'; DROP TABLE products; --`, `%`, `_`, a 5000-character string (422), and unicode all answer safely.
  - `pages_are_stable_when_prices_tie`: twenty products at one price, paged by 7, give every product exactly once.
  - `relevance_paging_stops_at_200`.
  - `bad_cursor_is_422`.
  - `price_from_uses_variant_overrides_and_in_stock_follows_reservations`.
  - `trigram_index_exists`: a query against `pg_indexes`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement the migration, the query and the route.**
- [ ] **Step 4: Run the whole suite.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add public catalog browse and search`

### Task 3: Store directory

**Files:**
- Create: `server/src/catalog/stores.rs`
- Modify: `server/src/catalog/mod.rs`, `server/tests/catalog.rs`

**Interfaces:** `GET /stores`, public. Query: `q`, `sort` (`rating`, `newest` default), `cursor`, `limit` (same limits). Response `{items: [StoreCard], next_cursor}` with `StoreCard { id, slug, name, logo_key, rating_average, rating_count, product_count }`, where `product_count` counts published products. Only `active`, non-deleted stores. Register it so it does not shadow `GET /stores/mine` or `GET /stores/{key}`.

- [ ] **Step 1: Write the failing tests:** `directory_lists_only_active_stores`, `directory_search_and_rating_sort`, `directory_does_not_shadow_mine_or_slug_routes`, `product_count_ignores_unpublished`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the whole suite.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add the public store directory`

### Task 4: Object storage layer

**Files:**
- Create: `server/src/uploads/mod.rs`, `server/src/uploads/store.rs` (trait, `NoStore`), `server/src/uploads/s3.rs`, `server/src/uploads/fake.rs`, `server/tests/s3_presign.rs`
- Modify: `server/src/config.rs`, `server/src/lib.rs` (`AppState.objects: Arc<dyn ObjectStore>`), `server/src/main.rs`, `server/tests/common/mod.rs` (`app.objects: Arc<FakeStore>`), `server/.env.example`

**Interfaces:**
- `ObjectStore` trait: `presign_put(bucket, key, content_type, content_length, ttl) -> Presigned { url, headers }`, `presign_get(bucket, key, ttl) -> String`, `get(bucket, key, max_bytes) -> Result<Vec<u8>, StoreError>`, `put(bucket, key, bytes, content_type)`, `delete(bucket, key)`. `StoreError` is `NotFound | TooLarge | Unavailable`, with no provider text.
- `Bucket::{Public, Private}` selects the configured bucket name.
- `S3Store`: SigV4 query-string presigning written by hand (`hmac`, `sha2`), path-style or virtual-host addressing per config, the signed headers include `content-length` and `content-type` for `presign_put`. Server-side `get`, `put` and `delete` use header-signed SigV4 requests over `reqwest` with the same HTTP rules as the other adapters (rustls, `https_only`, no redirects, connect timeout 5s, total 10s, response capped at the stated maximum, no provider text or key material in errors or logs). `insecure_http_for_tests` allows a local MinIO or wiremock.
- `FakeStore`: in-memory, with `objects()`, `put_raw` and settable failures.
- Config: `S3_ENDPOINT`, `S3_REGION`, `S3_BUCKET_PUBLIC`, `S3_BUCKET_PRIVATE`, `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY`, `UPLOADS_PUBLIC_BASE_URL` (all required together; the endpoint and the base URL must be https outside tests). Production refuses to start without them (`require_uploads`, mirroring `require_moderation`). Outside production with none set, uploads answer 503 `uploads_unavailable` and a warning is logged at startup.

- [ ] **Step 1: Write the failing tests** in `tests/s3_presign.rs`:
  - `presigned_put_matches_the_documented_signature`: a fixed key, time and region against a known SigV4 example copied from AWS's documentation into the test, so the canonical request, string to sign and signature are checked exactly.
  - `presigned_put_signs_length_and_type`: changing either breaks the signature.
  - `server_calls_sign_headers_and_hide_secrets`: against wiremock, `get`, `put` and `delete` send the expected method, path and `Authorization` shape, and the secret key and signature appear in no error or log (`tracing_test`).
  - `get_rejects_oversize_bodies`, `errors_map_to_not_found_too_large_unavailable`.
  - Config tests: `production_requires_uploads`, `endpoint_and_base_url_must_be_https`, `partial_config_is_refused`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement the trait, the adapters, the config and the state wiring.** Every `AppState` construction gets `objects`.
- [ ] **Step 4: Run the whole suite.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add the object store layer`

### Task 5: Upload requests and the finalize step

**Files:**
- Create: `server/migrations/0041_uploads.sql`, `server/src/uploads/service.rs`, `server/src/uploads/routes.rs`, `server/src/uploads/image_check.rs`, `server/src/uploads/jobs.rs`, `server/tests/uploads.rs`
- Modify: `server/Cargo.toml` (the `image` crate), `server/src/lib.rs`, `server/src/limits.rs`, `server/src/escrow/jobs.rs` (job 14)

**Schema (`0041`):** `uploads`: `id uuid PK`, `user_id uuid NOT NULL REFERENCES users`, `purpose text` CHECK in (`product`, `store_logo`, `store_banner`, `review`, `dispute`), `raw_key text NOT NULL`, `key text NULL` (the final key, set at finalize), `declared_type text`, `declared_bytes int`, `status text` CHECK in (`pending`, `ready`, `failed`), `attached_kind text NULL`, `attached_id uuid NULL`, `created_at`, `finalized_at NULL`. Unique on `key` where not null. Indexes: `(user_id, created_at)` and a partial index on `(created_at) WHERE status = 'pending'`. Header comment: the object itself is personal data (a photo); retention goes to the DPO's list, and plan 8 must purge uploads with their owners.

**Interfaces:**
- `POST /uploads` body `{purpose, content_type, bytes}` returns 201 `{upload_id, url, headers, expires_at}`. Checks the type, `1 <= bytes <= 5 MiB`, the per-user hourly cap (429 `upload_rate_limited`), and that a seller-only purpose (`product`, `store_logo`, `store_banner`) is requested by a seller (403 `not_a_seller`). Inserts the row (`raw_key = raw/{uuid}`), audits `upload.created` with `{purpose, bytes}`, and presigns the PUT after the insert.
- `POST /uploads/{id}/finalize` (owner of the upload only, 404 otherwise): reads the raw object with a hard byte cap (before any decoding), refuses if its size differs from the declared size, decodes with explicit limits (`image_check`), re-encodes to JPEG, writes the final object to the purpose's bucket under `u/{purpose}/{uuid}.jpg`, deletes the raw object, sets the row `ready`, audits `upload.finalized`, and returns `{upload_id, key}`. A failure marks the row `failed`, deletes the raw object, and answers 422 `photo_rejected` with a fixed reason (`not_an_image`, `too_large`, `too_many_pixels`, `corrupt`); never the decoder's text. Finalizing twice returns the same key. Object-store calls happen outside any transaction; the row update is fenced on `status = 'pending'`.
- `uploads::jobs::run` (job 14, `purge_uploads`): deletes `pending` and `failed` uploads older than 24 hours with their raw objects, in batches, like the other jobs.

- [ ] **Step 1: Write the failing tests** in `tests/uploads.rs` (with `FakeStore`):
  - `upload_round_trip`: request, `put_raw` the bytes the way the phone would, finalize, and the final object exists as a JPEG in the right bucket under the right prefix.
  - `declared_length_is_signed`: the presigned headers carry the declared type and length.
  - `finalize_rejects_oversize_and_bombs`: a body over 5 MiB, and a small PNG that declares a huge canvas, give 422 `photo_rejected` and leave no object behind.
  - `finalize_rejects_non_images`: a text file and a script renamed `.png`.
  - `finalize_strips_exif`: a JPEG with a GPS tag goes in; the output has no EXIF segment.
  - `finalize_checks_declared_size`.
  - `finalize_is_idempotent_and_owner_only`.
  - `purposes_and_buckets`: dispute photos land in the private bucket, product photos in the public one.
  - `seller_only_purposes_refuse_buyers`.
  - `hourly_cap_is_enforced`.
  - `purge_job_removes_stale_pending_uploads_and_objects`, which also checks `job_runs`.
  - `uploads_unavailable_without_a_store`: 503 `uploads_unavailable` with `NoStore`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement the migration, the module and the job.** Add the new `JobReport` field and module-doc entry as the other jobs do, and widen the `job_runs` test that lists job names.
- [ ] **Step 4: Run the whole suite.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add photo uploads with a server-side re-encode`

### Task 6: Ownership checks, photo URLs and private evidence

**Files:**
- Create: `server/src/uploads/claim.rs`
- Modify: `server/src/products/service.rs`, `server/src/stores/service.rs`, `server/src/reviews/service.rs`, `server/src/disputes/service.rs`, `server/src/disputes/routes.rs`, `server/src/disputes/staff.rs`, `server/src/catalog/browse.rs`, `server/tests/uploads.rs`, `server/tests/disputes.rs`

**Interfaces:**
- `uploads::claim::claim(conn, user, purpose, keys: &[String], attach: (Kind, Uuid)) -> Result<(), AppError>`: locks the matching `uploads` rows (ordered by id), requires each to be `ready`, `user_id = user`, with `purpose` matching, and `attached_*` null or equal to `attach`; sets `attached_*`. Errors: 422 `photo_not_yours` (unknown, pending, foreign or wrong purpose, with one message for all, so keys cannot be probed), 409 `photo_in_use`.
- It is called in the caller's transaction, after the resource's own lock, from: product create and patch (`photo_keys`), store patch (`logo_key`, `banner_key`), review create (`photo_keys`), dispute open and respond (`photo_keys`). A key already stored on the same resource is not rechecked (existing rows are grandfathered).
- Views gain URLs next to the keys: `photo_urls` on products and reviews, `logo_url` and `banner_url` on stores, `photo_url` on `ProductCard` and `logo_url` on `StoreCard`, each `{UPLOADS_PUBLIC_BASE_URL}/{key}`. Dispute views keep keys only.
- `GET /disputes/{id}/photo?key=...` (the key is a query value, not a path segment, because it contains slashes; it holds no personal data): authorisation as in Global Constraints; the key must belong to that dispute's statements; answers `{url, expires_at}`; staff reads write `admin.dispute_photo_viewed`.

- [ ] **Step 1: Write the failing tests:**
  - `cannot_claim_foreign_pending_or_wrong_purpose_keys`, with one error body for all three.
  - `concurrent_claims_of_one_key_attach_once`: two requests race, one wins, the other gets `photo_in_use`.
  - `existing_keys_are_not_rechecked_on_patch`.
  - `claims_roll_back_with_the_transaction`: a product create that fails later leaves its keys unattached.
  - `product_store_review_and_dispute_photos_all_require_ownership`, one case per resource.
  - `public_views_carry_urls`, including cards in browse and the directory.
  - `dispute_photos_are_never_public`: the dispute view has no URL and the bucket for the key is the private one.
  - `dispute_photo_url_is_short_lived_and_scoped`: buyer and seller get one; the expiry is 60 seconds.
  - `stranger_gets_404_for_dispute_photo`, `support_cannot_sign_closed_dispute_photo` (403 `dispute_closed`), `owner_reads_are_audited`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement the claim, wire it into the five places, add the URLs and the evidence route.** Update the test fixtures that pass made-up photo keys (they will now need a real upload through `FakeStore`: add a `TestApp::ready_photo(user, purpose) -> String` helper).
- [ ] **Step 4: Run the whole suite.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: check photo ownership and serve photos by URL`

### Task 7: Password reset *(built)*

> **As built:** the migration is `0041_otp_purpose.sql` (0041 was free). Two deviations: there is **no per-address limit** on the request route, because the server has no trusted client-address setting yet (the per-account OTP limits still cap SMS sent to any one phone); and a reset requested within a minute of a sign-up code gets no new code (the app reads `resend_after_secs`). All three wrong-code outcomes (wrong, expired, locked) answer the one `code_invalid`.

**Files:**
- Create: `server/migrations/0042_otp_purpose.sql`, `server/src/auth/reset.rs`, `server/tests/password_reset.rs`
- Modify: `server/src/auth/routes.rs`, `server/src/auth/otp.rs`

**Schema (`0042`):** `otp_codes` gains `purpose text NOT NULL DEFAULT 'verify_phone' CHECK (purpose IN ('verify_phone', 'password_reset'))`. Existing rows keep the default. A code for one purpose never verifies for another.

**Interfaces:**
- `POST /auth/password-reset` body `{phone}`: validates and normalises the phone as sign-up does, then always answers `202 {verification_id, resend_after_secs}`. If the phone belongs to a verified, non-deleted account and the OTP limits allow, a code is issued and sent; otherwise nothing is sent, and the answer is still well-formed. The limits are the existing OTP limits (`OTP_MIN_INTERVAL_SECS`, `OTP_MAX_PER_HOUR`), plus a per-address limit on this route.
- `POST /auth/password-reset/confirm` body `{verification_id, code, new_password}`: verifies the code with the attempt limit (`OTP_MAX_ATTEMPTS`), single use; sets the password (Argon2); revokes every session of the user; clears the sign-in lockout counters; audits `auth.password_reset`; answers 204. Wrong or expired code, or an unknown `verification_id`, answer one 422 `code_invalid`.

- [ ] **Step 1: Write the failing tests** in `tests/password_reset.rs`:
  - `reset_request_looks_the_same_for_unknown_phones`: status, body shape and field set match.
  - `reset_code_is_single_use_and_attempt_limited`.
  - `reset_code_does_not_verify_a_phone_and_the_reverse`.
  - `reset_revokes_every_session`: two sessions, both dead afterwards; the old password fails and the new one signs in.
  - `reset_clears_the_lockout`.
  - `deleted_account_cannot_reset` and `unverified_account_gets_no_code`.
  - `weak_new_password_is_refused_and_the_code_survives`.
  - `reset_is_rate_limited`.
  - `reset_is_audited_without_phone_or_code`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement the migration, the service and the routes.**
- [ ] **Step 4: Run the whole suite.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: let people reset a forgotten password`

### Task 8: The SMS sender and the throttled-resend fix

**Blocked on decision D4** (which SMS provider). Do Steps 1 and 2 and the trait work now; the provider adapter waits for the choice.

**Files:**
- Create: `server/src/auth/sms.rs` (the `SmsProvider` trait and `SmsOtpSender`), `server/src/auth/sms_<provider>.rs` (once chosen), `server/tests/sms_otp.rs`
- Modify: `server/src/config.rs`, `server/src/main.rs`, `server/src/auth/routes.rs`, `server/.env.example`

**Interfaces:**
- `SmsOtpSender` implements `OtpSender` over an `SmsProvider { send(to_e164, text) }`, so the provider is one small adapter. The message text is fixed copy ("Your code is 123456. It expires in 10 minutes. Never share it.") with the code the only variable, and no name.
- HTTP rules as for the other adapters. A provider failure is `AppError::Unavailable`-style (503 `otp_unavailable`), never the provider's text; the phone and the code are never logged.
- Production refuses to start without a sender (`require_sms`); elsewhere `LogOtpSender` remains.
- **The carry-forward:** sign-up resend and password-reset responses include `resend_after_secs`, and when a request was throttled they say so: `429 otp_throttled` with `retry_after_secs` for resend, and for reset the always-202 body is unchanged (so it cannot reveal anything) while the app simply uses `resend_after_secs`. Never return a `verification_id` for a code that was not sent from the sign-up routes.

- [ ] **Step 1: Write the failing tests** in `tests/sms_otp.rs` with a recording fake provider: `code_text_is_fixed_copy`, `provider_failure_is_503_without_provider_text`, `phone_and_code_never_logged` (`tracing_test`), `throttled_resend_says_so_and_sends_nothing`, `production_requires_a_sender`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement the trait, the sender, the config and the resend behaviour.**
- [ ] **Step 4: Run the whole suite.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: send sign-up codes by SMS`
- [ ] **Step 6 (after D4):** add the chosen provider's adapter, with wiremock tests of its exact request and error mapping, and commit `feat: add the <provider> SMS adapter`.

### Task 9: App config

**Files:**
- Create: `server/src/app_config.rs`, `server/tests/app_config.rs`
- Modify: `server/src/lib.rs`, `server/src/config.rs`, `server/.env.example`

**Interfaces:** `GET /app/config`, public, cacheable (`Cache-Control: public, max-age=300`). Answers `{min_version: {android, ios}, terms_url, privacy_url, support_email, maintenance: {active, message}}`, all read from config (`APP_MIN_VERSION_ANDROID`, `APP_MIN_VERSION_IOS`, `APP_TERMS_URL`, `APP_PRIVACY_URL`, `APP_SUPPORT_EMAIL`, `APP_MAINTENANCE_MESSAGE`). Versions are semantic versions, validated at startup; links must be https; nothing about the caller. Empty values are `null`.

- [ ] **Step 1: Write the failing tests:** `config_route_reads_settings`, `bad_version_or_http_link_refuses_to_start`, `defaults_are_null_not_empty`, `response_is_cacheable_and_has_no_user_data`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the whole suite.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add a public app config route`

### Task 10: Local services, end-to-end path and docs

**Files:**
- Modify: `docker-compose.yml`, `server/tests/uploads.rs` (or a new `server/tests/app_path.rs`), `README.md`, `docs/superpowers/plans/2026-10-23-00-roadmap.md`, `server/.env.example`

- [ ] **Step 1: Add MinIO to `docker-compose.yml`** as an optional service for local work (an S3-compatible store with a public and a private bucket created at start by a one-shot helper container). Keep it out of the default `up -d db` path.
- [ ] **Step 2: Write the end-to-end test** `app_path_from_signup_to_search` with `FakeStore` and the fake SMS provider:
  1. A seller redeems a code, creates a store, uploads two photos and finalizes them.
  2. The seller creates a product with those keys and publishes it.
  3. An anonymous caller lists `GET /categories`, searches `GET /products?q=...`, and sees the card with a photo URL and the store's rating fields.
  4. A buyer forgets their password, resets it by code, signs in again, and the old session is dead.
  5. The buyer opens a dispute with an evidence photo; the seller can read it through the signed route; a third user cannot; the public base URL does not serve it.
  6. `GET /app/config` answers.
- [ ] **Step 3: Update the docs.**
  - **README:** add "App support (part 7a)" under "What works today", with the upload flow in one paragraph (request, put, finalize, attach), and add the new settings to the go-live list.
  - **Roadmap:** mark 7a built with its test count; keep 7b to 7e "not written".
  - **`.env.example`:** every new variable, placeholders only, a note that the S3 keys and SMS credentials are secrets kept outside the repository.
- [ ] **Step 4: Add these go-live gates** to the roadmap and README (for review, not conclusions):
  - DPO and legal: the object store as a processor of photos, its region and any cross-border transfer; retention of uploads, and a purge on account erasure (plan 8); photos that may show faces or other people; whether evidence photos need access logging beyond staff reads.
  - The SMS provider as a processor of phone numbers, its sender-ID registration, and delivery to do-not-disturb numbers.
  - The production database role needs permission to create the `pg_trgm` extension, or the database owner creates it before the first deploy.
  - Photos are re-encoded but not content-checked: nothing detects a phone number written on a product photo or an inappropriate image. Decide before launch whether to add a moderation step.
  - Owner decisions D3, D4 and D5 recorded in the roadmap.
- [ ] **Step 5: Run everything.** From `server/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, then the full `cargo test`. Expected: all pass, with no suite skipped.
- [ ] **Step 6: Commit.** `test: add the app support end-to-end path` and `docs: document the app server additions`
