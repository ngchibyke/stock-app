//! What the server needs from object storage, behind a trait so tests use an
//! in-memory fake and production uses any S3-compatible store.

use std::time::Duration;

use async_trait::async_trait;

/// Public photos (products, stores, reviews) are served from a public bucket;
/// private ones (dispute evidence) are only ever reached by a short-lived
/// signed link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bucket {
    Public,
    Private,
}

/// Why a storage call failed. No provider text, key or URL is carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreError {
    NotFound,
    TooLarge,
    Unavailable,
}

/// A signed upload: the phone `PUT`s the file to `url` with exactly `headers`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presigned {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// `false` when uploads are not configured: the routes answer 503.
    fn available(&self) -> bool {
        true
    }

    /// The address a public object is served from, or `None` when there is none.
    fn public_url(&self, key: &str) -> Option<String>;

    /// A signed `PUT` that accepts only this content type and length.
    fn presign_put(
        &self,
        bucket: Bucket,
        key: &str,
        content_type: &str,
        content_length: u64,
        ttl: Duration,
    ) -> Result<Presigned, StoreError>;

    /// A signed `GET`, for private objects.
    fn presign_get(&self, bucket: Bucket, key: &str, ttl: Duration) -> Result<String, StoreError>;

    /// Reads an object, refusing one larger than `max_bytes`.
    async fn get(&self, bucket: Bucket, key: &str, max_bytes: usize)
    -> Result<Vec<u8>, StoreError>;

    async fn put(
        &self,
        bucket: Bucket,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StoreError>;

    /// [`put`](Self::put) with a `Cache-Control` header, for art that never changes under
    /// its name. A store that cannot set one stores the object without it.
    async fn put_cached(
        &self,
        bucket: Bucket,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
        _cache_control: &str,
    ) -> Result<(), StoreError> {
        self.put(bucket, key, bytes, content_type).await
    }

    /// Deleting what is already gone succeeds.
    async fn delete(&self, bucket: Bucket, key: &str) -> Result<(), StoreError>;
}

/// Used when no store is configured (development without one): every call says
/// it is unavailable.
pub struct NoStore;

#[async_trait]
impl ObjectStore for NoStore {
    fn available(&self) -> bool {
        false
    }
    fn public_url(&self, _key: &str) -> Option<String> {
        None
    }
    fn presign_put(
        &self,
        _bucket: Bucket,
        _key: &str,
        _content_type: &str,
        _content_length: u64,
        _ttl: Duration,
    ) -> Result<Presigned, StoreError> {
        Err(StoreError::Unavailable)
    }
    fn presign_get(
        &self,
        _bucket: Bucket,
        _key: &str,
        _ttl: Duration,
    ) -> Result<String, StoreError> {
        Err(StoreError::Unavailable)
    }
    async fn get(&self, _b: Bucket, _k: &str, _max: usize) -> Result<Vec<u8>, StoreError> {
        Err(StoreError::Unavailable)
    }
    async fn put(
        &self,
        _b: Bucket,
        _k: &str,
        _bytes: Vec<u8>,
        _ct: &str,
    ) -> Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
    async fn delete(&self, _b: Bucket, _k: &str) -> Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
}
