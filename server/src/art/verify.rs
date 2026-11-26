//! Verifying a published version from the outside: the manifest, every file with its size
//! and checksum, and whether the layout the server would load is the layout the art was
//! made for.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;

use super::{build::Manifest, layout_hash, sha256_hex};
use crate::market::layout::Layout;

const MAX_FILE: usize = 64 << 20;

/// Where a verifier reads from: the public address in production, a store in tests.
#[async_trait]
pub trait Fetch: Send + Sync {
    /// The file at `<version>/<path>`; `None` when it is not there.
    async fn get(&self, version: &str, path: &str) -> Result<Option<Vec<u8>>>;
}

/// Reads over https from the address the app uses (`MARKET_ASSETS_BASE_URL`).
pub struct HttpFetcher {
    base: String,
    client: reqwest::Client,
}

impl HttpFetcher {
    pub fn new(base: &str) -> Result<Self> {
        if !base.starts_with("https://") {
            bail!("the public address must be an https URL");
        }
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| anyhow!(e.without_url()))?;
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            client,
        })
    }
}

#[async_trait]
impl Fetch for HttpFetcher {
    async fn get(&self, version: &str, path: &str) -> Result<Option<Vec<u8>>> {
        let url = format!("{}/{version}/{path}", self.base);
        let res = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("could not reach the public address: {}", e.without_url()))?;
        if res.status().as_u16() == 404 || res.status().as_u16() == 403 {
            return Ok(None);
        }
        if !res.status().is_success() {
            bail!("the public address answered {}", res.status().as_u16());
        }
        let bytes = res.bytes().await.map_err(|e| anyhow!(e.without_url()))?;
        if bytes.len() > MAX_FILE {
            bail!("{path} is larger than expected");
        }
        Ok(Some(bytes.to_vec()))
    }
}

#[derive(Debug, Default)]
pub struct Verified {
    pub problems: Vec<String>,
    pub checked: usize,
}

impl Verified {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }

    pub fn print(&self) {
        for p in &self.problems {
            println!("problem: {p}");
        }
        if self.ok() {
            println!("version is good: {} files match the manifest", self.checked);
        }
    }
}

/// Checks `version`. With `layout_dir`, also that the server's layout there is the one the
/// art was made for.
pub async fn verify(
    fetch: &dyn Fetch,
    version: &str,
    layout_dir: Option<&Path>,
) -> Result<Verified> {
    let mut v = Verified::default();
    let Some(raw) = fetch.get(version, "manifest.json").await? else {
        v.problems.push(format!(
            "version {version} has no manifest: it was never published, or the upload was cut short"
        ));
        return Ok(v);
    };
    let manifest: Manifest = serde_json::from_slice(&raw).context("the manifest is not valid")?;
    if manifest.version != version {
        v.problems.push(format!(
            "the manifest says version {} but was fetched as {version}",
            manifest.version
        ));
    }
    for f in &manifest.files {
        match fetch.get(version, &f.path).await? {
            None => v.problems.push(format!("{} is missing", f.path)),
            Some(bytes) => {
                v.checked += 1;
                if bytes.len() as u64 != f.bytes {
                    v.problems.push(format!(
                        "{} is {} bytes, the manifest says {}",
                        f.path,
                        bytes.len(),
                        f.bytes
                    ));
                } else if sha256_hex(&bytes) != f.sha256 {
                    v.problems
                        .push(format!("{} has changed since it was built", f.path));
                }
            }
        }
    }
    if let Some(dir) = layout_dir {
        match Layout::load(dir) {
            Err(e) => v
                .problems
                .push(format!("the server's layout cannot be loaded: {e:#}")),
            Ok(layout) => {
                if layout_hash(layout.layout_json()) != manifest.layout_sha256 {
                    v.problems.push(
                        "the plots in this art are not the plots the server places stores on: \
                         copy the pack's layout.json and rooms into the server's market folder and restart"
                            .to_string(),
                    );
                }
                let wanted = layout.layout_json();
                if wanted["tile_size"].as_u64() != Some(u64::from(manifest.tile_size))
                    || wanted["max_level"].as_u64() != Some(u64::from(manifest.max_level))
                {
                    v.problems.push(
                        "the server's tile_size or max_level differ from the published tiles"
                            .to_string(),
                    );
                }
            }
        }
    }
    Ok(v)
}
