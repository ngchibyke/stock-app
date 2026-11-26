//! Publishing a built version to the public bucket. Every file goes up with a long
//! immutable cache lifetime and the manifest goes last, so a half-uploaded version has no
//! manifest and is never taken for a real one. A version that already has a manifest is
//! refused: a published version never changes.

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result, bail};
use tokio::{sync::Semaphore, task::JoinSet};

use super::{CACHE_CONTROL, KEY_ROOT, build::Manifest, sha256_hex};
use crate::uploads::store::{Bucket, ObjectStore, StoreError};

const AT_ONCE: usize = 8;

#[derive(Debug)]
pub struct Published {
    pub version: String,
    pub files: usize,
}

pub fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        _ => "application/json",
    }
}

pub fn key(version: &str, path: &str) -> String {
    format!("{KEY_ROOT}/{version}/{path}")
}

/// Uploads `dir` (a folder made by `build`), which is named for its version.
pub async fn publish(store: Arc<dyn ObjectStore>, dir: &Path) -> Result<Published> {
    let manifest_bytes = std::fs::read(dir.join("manifest.json"))
        .context("manifest.json is missing: publish a folder made by `art build`")?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).context("manifest.json")?;
    let version = manifest.version.clone();

    // Every file must be what the manifest says, so a damaged build is not published.
    let mut loaded = Vec::new();
    for f in &manifest.files {
        let bytes = std::fs::read(dir.join(&f.path))
            .with_context(|| format!("{} is missing from the build", f.path))?;
        if bytes.len() as u64 != f.bytes || sha256_hex(&bytes) != f.sha256 {
            bail!("{} does not match the manifest; build again", f.path);
        }
        loaded.push((f.path.clone(), bytes));
    }

    match store
        .get(Bucket::Public, &key(&version, "manifest.json"), 1 << 20)
        .await
    {
        Ok(_) => bail!("version {version} is already published; choose a new version name"),
        Err(StoreError::NotFound) => {}
        Err(_) => bail!("the object store could not be reached"),
    }

    let slots = Arc::new(Semaphore::new(AT_ONCE));
    let mut tasks: JoinSet<Result<()>> = JoinSet::new();
    let count = loaded.len() + 1;
    for (path, bytes) in loaded {
        let slots = slots.clone();
        let version = version.clone();
        let store = store.clone();
        tasks.spawn(async move {
            let _permit = slots
                .acquire()
                .await
                .map_err(|_| anyhow::anyhow!("stopped"))?;
            store
                .put_cached(
                    Bucket::Public,
                    &key(&version, &path),
                    bytes,
                    content_type(&path),
                    CACHE_CONTROL,
                )
                .await
                .map_err(|_| anyhow::anyhow!("upload of {path} failed"))
        });
    }
    let mut failure = None;
    while let Some(done) = tasks.join_next().await {
        if let Err(e) = done.context("upload task").and_then(|r| r) {
            failure.get_or_insert(e);
            tasks.abort_all();
        }
    }
    if let Some(e) = failure {
        return Err(e.context("nothing was made live: the manifest was not uploaded"));
    }
    store
        .put_cached(
            Bucket::Public,
            &key(&version, "manifest.json"),
            manifest_bytes,
            "application/json",
            CACHE_CONTROL,
        )
        .await
        .map_err(|_| anyhow::anyhow!("upload of the manifest failed"))?;
    Ok(Published {
        version,
        files: count,
    })
}
