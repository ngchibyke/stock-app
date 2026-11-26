//! Test double for [`ObjectStore`]: objects live in memory.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;

use super::store::{Bucket, ObjectStore, Presigned, StoreError};

/// An object's bytes and content type.
type Object = (Vec<u8>, String);

#[derive(Default)]
pub struct FakeStore {
    objects: Mutex<HashMap<(Bucket, String), Object>>,
    down: AtomicBool,
    cache: Mutex<HashMap<(Bucket, String), String>>,
    fail_after: Mutex<Option<usize>>,
    puts: std::sync::atomic::AtomicUsize,
}

fn name(bucket: Bucket) -> &'static str {
    match bucket {
        Bucket::Public => "public",
        Bucket::Private => "private",
    }
}

impl FakeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// While set, every call fails as unavailable.
    pub fn set_unavailable(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
    }

    /// Puts an object the way a phone's signed upload would.
    pub fn put_raw(&self, bucket: Bucket, key: &str, bytes: Vec<u8>, content_type: &str) {
        self.objects
            .lock()
            .unwrap()
            .insert((bucket, key.to_string()), (bytes, content_type.to_string()));
    }

    /// The `Cache-Control` an object was stored with, if it was stored with one.
    pub fn cache_control(&self, bucket: Bucket, key: &str) -> Option<String> {
        self.cache
            .lock()
            .unwrap()
            .get(&(bucket, key.to_string()))
            .cloned()
    }

    /// After this many more successful puts, every put fails (an upload cut short).
    pub fn fail_puts_after(&self, n: usize) {
        *self.fail_after.lock().unwrap() = Some(n);
        self.puts.store(0, Ordering::SeqCst);
    }

    pub fn object(&self, bucket: Bucket, key: &str) -> Option<Vec<u8>> {
        self.objects
            .lock()
            .unwrap()
            .get(&(bucket, key.to_string()))
            .map(|(b, _)| b.clone())
    }

    pub fn content_type(&self, bucket: Bucket, key: &str) -> Option<String> {
        self.objects
            .lock()
            .unwrap()
            .get(&(bucket, key.to_string()))
            .map(|(_, t)| t.clone())
    }

    /// Every key in a bucket, sorted.
    pub fn keys(&self, bucket: Bucket) -> Vec<String> {
        let mut keys: Vec<String> = self
            .objects
            .lock()
            .unwrap()
            .keys()
            .filter(|(b, _)| *b == bucket)
            .map(|(_, k)| k.clone())
            .collect();
        keys.sort();
        keys
    }

    fn check(&self) -> Result<(), StoreError> {
        if self.down.load(Ordering::SeqCst) {
            Err(StoreError::Unavailable)
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl ObjectStore for FakeStore {
    fn public_url(&self, key: &str) -> Option<String> {
        Some(format!("https://cdn.fake.invalid/{key}"))
    }

    fn presign_put(
        &self,
        bucket: Bucket,
        key: &str,
        content_type: &str,
        content_length: u64,
        ttl: Duration,
    ) -> Result<Presigned, StoreError> {
        self.check()?;
        Ok(Presigned {
            url: format!(
                "https://fake.invalid/{}/{key}?signed=put&ttl={}",
                name(bucket),
                ttl.as_secs()
            ),
            headers: vec![
                ("content-type".into(), content_type.into()),
                ("content-length".into(), content_length.to_string()),
            ],
        })
    }

    fn presign_get(&self, bucket: Bucket, key: &str, ttl: Duration) -> Result<String, StoreError> {
        self.check()?;
        Ok(format!(
            "https://fake.invalid/{}/{key}?signed=get&ttl={}",
            name(bucket),
            ttl.as_secs()
        ))
    }

    async fn get(
        &self,
        bucket: Bucket,
        key: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, StoreError> {
        self.check()?;
        let bytes = self.object(bucket, key).ok_or(StoreError::NotFound)?;
        if bytes.len() > max_bytes {
            return Err(StoreError::TooLarge);
        }
        Ok(bytes)
    }

    async fn put(
        &self,
        bucket: Bucket,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StoreError> {
        self.check()?;
        if let Some(limit) = *self.fail_after.lock().unwrap()
            && self.puts.fetch_add(1, Ordering::SeqCst) >= limit
        {
            return Err(StoreError::Unavailable);
        }
        self.put_raw(bucket, key, bytes, content_type);
        Ok(())
    }

    async fn put_cached(
        &self,
        bucket: Bucket,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
        cache_control: &str,
    ) -> Result<(), StoreError> {
        self.put(bucket, key, bytes, content_type).await?;
        self.cache
            .lock()
            .unwrap()
            .insert((bucket, key.to_string()), cache_control.to_string());
        Ok(())
    }

    async fn delete(&self, bucket: Bucket, key: &str) -> Result<(), StoreError> {
        self.check()?;
        self.objects
            .lock()
            .unwrap()
            .remove(&(bucket, key.to_string()));
        Ok(())
    }
}
