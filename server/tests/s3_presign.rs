//! The object store layer (plan 7a, task 4): SigV4 signing checked against the
//! examples in AWS's documentation, and `S3Store` against a local mock server.
//! No real network.

use std::time::Duration;

mod dead_port;

use dead_port::DeadPort;
use server::{
    payments::mono::Secret,
    uploads::{
        s3::{S3Config, S3Store},
        sigv4::{Signer, sha256_hex},
        store::{Bucket, ObjectStore, StoreError},
    },
};
use time::macros::datetime;
use tracing_test::traced_test;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const KEY_ID: &str = "TESTKEYID0123456789";
const SECRET: &str = "TESTSECRETaccessKEY/never/log/this";

/// The example credentials AWS uses throughout its signing documentation.
fn aws_example_signer() -> Signer {
    Signer::new(
        "AKIAIOSFODNN7EXAMPLE",
        Secret::new("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
        "us-east-1",
    )
}

#[test]
fn presigned_get_matches_the_documented_example() {
    // "Authenticating Requests: Using Query Parameters" in the S3 documentation:
    // GET https://examplebucket.s3.amazonaws.com/test.txt, valid 86400 seconds.
    let query = aws_example_signer().presign(
        "GET",
        "examplebucket.s3.amazonaws.com",
        "/test.txt",
        &[],
        86_400,
        datetime!(2013-05-24 00:00:00 UTC),
    );
    assert_eq!(
        query,
        "X-Amz-Algorithm=AWS4-HMAC-SHA256\
         &X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request\
         &X-Amz-Date=20130524T000000Z\
         &X-Amz-Expires=86400\
         &X-Amz-SignedHeaders=host\
         &X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
    );
}

#[test]
fn header_signature_matches_the_documented_example() {
    // "Signature Calculations for the Authorization Header", GET Object with a Range.
    let empty = sha256_hex(b"");
    assert_eq!(
        empty,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    let auth = aws_example_signer().authorize(
        "GET",
        "examplebucket.s3.amazonaws.com",
        "/test.txt",
        "",
        &[
            ("range", "bytes=0-9"),
            ("x-amz-content-sha256", &empty),
            ("x-amz-date", "20130524T000000Z"),
        ],
        &empty,
        datetime!(2013-05-24 00:00:00 UTC),
    );
    assert_eq!(
        auth,
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,\
         SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,\
         Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
    );
}

fn config(server: &MockServer) -> S3Config {
    S3Config {
        endpoint: server.uri(),
        region: "af-south-1".into(),
        bucket_public: "pub".into(),
        bucket_private: "priv".into(),
        access_key_id: KEY_ID.into(),
        secret_access_key: Secret::new(SECRET),
        public_base_url: "https://cdn.example.test".into(),
        path_style: true,
        connect_timeout: Duration::from_millis(500),
        timeout: Duration::from_millis(300),
        insecure_http_for_tests: true,
    }
}

fn store(server: &MockServer) -> S3Store {
    S3Store::new(config(server)).unwrap()
}

fn query_param(url: &str, name: &str) -> String {
    let url = reqwest::Url::parse(url).unwrap();
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| panic!("no {name} in {url}"))
}

#[tokio::test]
async fn presigned_put_signs_the_length_and_the_type() {
    let server = MockServer::start().await;
    let s = store(&server);
    let at = datetime!(2026-11-20 10:00:00 UTC);
    let ttl = Duration::from_secs(600);
    let base = s
        .presign_put_at(Bucket::Public, "raw/abc", "image/jpeg", 1000, ttl, at)
        .unwrap();
    let longer = s
        .presign_put_at(Bucket::Public, "raw/abc", "image/jpeg", 1001, ttl, at)
        .unwrap();
    let other_type = s
        .presign_put_at(Bucket::Public, "raw/abc", "image/png", 1000, ttl, at)
        .unwrap();
    let other_bucket = s
        .presign_put_at(Bucket::Private, "raw/abc", "image/jpeg", 1000, ttl, at)
        .unwrap();
    let sig = |p: &server::uploads::store::Presigned| query_param(&p.url, "X-Amz-Signature");
    assert_ne!(sig(&base), sig(&longer), "the declared length is signed");
    assert_ne!(sig(&base), sig(&other_type), "the declared type is signed");
    assert_ne!(sig(&base), sig(&other_bucket), "the bucket is signed");
    assert_eq!(
        query_param(&base.url, "X-Amz-SignedHeaders"),
        "content-length;content-type;host"
    );
    assert_eq!(query_param(&base.url, "X-Amz-Expires"), "600");
    assert!(
        base.url
            .starts_with(&format!("{}/pub/raw/abc?", server.uri()))
    );
    // What the phone must send, exactly.
    assert!(
        base.headers
            .contains(&("content-type".into(), "image/jpeg".into()))
    );
    assert!(
        base.headers
            .contains(&("content-length".into(), "1000".into()))
    );
    // The same inputs sign the same way, and the secret is nowhere in the URL.
    let again = s
        .presign_put_at(Bucket::Public, "raw/abc", "image/jpeg", 1000, ttl, at)
        .unwrap();
    assert_eq!(base.url, again.url);
    assert!(!base.url.contains(SECRET));
}

#[tokio::test]
async fn presigned_get_is_short_lived_and_signs_only_the_host() {
    let server = MockServer::start().await;
    let url = store(&server)
        .presign_get_at(
            Bucket::Private,
            "u/dispute/a.jpg",
            Duration::from_secs(60),
            datetime!(2026-11-20 10:00:00 UTC),
        )
        .unwrap();
    assert_eq!(query_param(&url, "X-Amz-Expires"), "60");
    assert_eq!(query_param(&url, "X-Amz-SignedHeaders"), "host");
    assert!(url.contains("/priv/u/dispute/a.jpg?"));
}

#[tokio::test]
async fn public_url_is_the_cdn_base_plus_the_key() {
    let server = MockServer::start().await;
    assert_eq!(
        store(&server).public_url("u/product/a.jpg").as_deref(),
        Some("https://cdn.example.test/u/product/a.jpg")
    );
}

#[tokio::test]
async fn server_calls_sign_their_headers() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/pub/u/product/a.jpg"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/pub/u/product/a.jpg"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello".to_vec()))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/pub/u/product/a.jpg"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let s = store(&server);
    s.put(
        Bucket::Public,
        "u/product/a.jpg",
        b"hello".to_vec(),
        "image/jpeg",
    )
    .await
    .unwrap();
    assert_eq!(
        s.get(Bucket::Public, "u/product/a.jpg", 1024)
            .await
            .unwrap(),
        b"hello"
    );
    s.delete(Bucket::Public, "u/product/a.jpg").await.unwrap();

    let calls = server.received_requests().await.unwrap();
    assert_eq!(calls.len(), 3);
    for call in &calls {
        let auth = call.headers.get("authorization").unwrap().to_str().unwrap();
        assert!(
            auth.starts_with(&format!("AWS4-HMAC-SHA256 Credential={KEY_ID}/")),
            "{auth}"
        );
        assert!(auth.contains("/af-south-1/s3/aws4_request"), "{auth}");
        assert!(
            auth.contains("SignedHeaders=") && auth.contains("host"),
            "{auth}"
        );
        assert!(call.headers.get("x-amz-date").is_some());
        assert!(!auth.contains(SECRET));
    }
    let put = &calls[0];
    assert_eq!(
        put.headers
            .get("x-amz-content-sha256")
            .unwrap()
            .to_str()
            .unwrap(),
        sha256_hex(b"hello")
    );
    assert_eq!(put.headers.get("content-type").unwrap(), "image/jpeg");
}

#[tokio::test]
async fn get_rejects_oversize_bodies() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 6]))
        .mount(&server)
        .await;
    let err = store(&server)
        .get(Bucket::Public, "k", 5)
        .await
        .unwrap_err();
    assert_eq!(err, StoreError::TooLarge);
    let ok = store(&server).get(Bucket::Public, "k", 6).await.unwrap();
    assert_eq!(ok.len(), 6);
}

#[tokio::test]
async fn errors_map_to_not_found_too_large_and_unavailable() {
    let cases = [
        (404, StoreError::NotFound),
        (403, StoreError::Unavailable),
        (500, StoreError::Unavailable),
        (503, StoreError::Unavailable),
    ];
    for (status, want) in cases {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(status).set_body_string("provider-text"))
            .mount(&server)
            .await;
        assert_eq!(
            store(&server).get(Bucket::Public, "k", 100).await,
            Err(want),
            "{status}"
        );
    }
    // Too slow.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(900)))
        .mount(&server)
        .await;
    assert_eq!(
        store(&server).get(Bucket::Public, "k", 100).await,
        Err(StoreError::Unavailable)
    );
    // Nothing listening.
    let dead = DeadPort::new();
    let mut cfg = config(&server);
    cfg.endpoint = format!("http://{}", dead.addr);
    assert_eq!(
        S3Store::new(cfg)
            .unwrap()
            .get(Bucket::Public, "k", 100)
            .await,
        Err(StoreError::Unavailable)
    );
    // Deleting what is already gone is fine.
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    store(&server).delete(Bucket::Public, "k").await.unwrap();
}

#[tokio::test]
#[traced_test]
async fn secrets_and_provider_text_never_reach_logs_or_debug() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500).set_body_string("provider-text-do-not-log"))
        .mount(&server)
        .await;
    let s = store(&server);
    let err = s
        .get(Bucket::Public, "secret-key-name/a.jpg", 100)
        .await
        .unwrap_err();
    let debug = format!("{err:?} {:?}", config(&server));
    for needle in [SECRET, "provider-text-do-not-log"] {
        assert!(!debug.contains(needle), "Debug shows {needle}: {debug}");
        assert!(!logs_contain(needle), "logs show {needle}");
    }
    assert!(!logs_contain("Signature="), "a signature reached the logs");
    assert!(
        logs_contain("object store failed"),
        "the failure is still logged"
    );
}

#[test]
fn http_endpoints_are_refused_outside_tests() {
    let cfg = S3Config {
        endpoint: "http://insecure.example".into(),
        region: "r".into(),
        bucket_public: "a".into(),
        bucket_private: "b".into(),
        access_key_id: "k".into(),
        secret_access_key: Secret::new("s"),
        public_base_url: "https://cdn.example.test".into(),
        path_style: true,
        connect_timeout: Duration::from_secs(1),
        timeout: Duration::from_secs(1),
        insecure_http_for_tests: false,
    };
    assert!(S3Store::new(cfg).is_err());
}
