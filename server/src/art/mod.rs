//! The art pipeline (part 7f): `server art check | build | publish | verify`.
//!
//! The owner makes the market art offline and hands over an *art pack*. These commands
//! check it, cut it into the files the app downloads, publish them under a version and
//! verify a published version from the outside. Nothing here touches the database, and
//! nothing about a person is in the art or in this tool's output.
//!
//! The file names are a contract with the app's `RemoteAssetResolver`:
//! `base.jpg`, `tiles/{level}/{x}_{y}.jpg`, `rooms/{type}/room.jpg` and `depth.png`.

pub mod build;
pub mod pack;
pub mod publish;
pub mod verify;

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::{
    config::s3_from_env,
    uploads::{s3::S3Store, store::ObjectStore},
};

/// Where a version's files live in the public bucket: `market/<version>/...`.
pub const KEY_ROOT: &str = "market";

/// The long cache lifetime for art: a version's files never change under their name.
pub const CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A hash of the layout that ignores `store_type`, which only the server reads, so the
/// published art can be compared with the server's copy of the layout.
pub fn layout_hash(layout: &serde_json::Value) -> String {
    let mut v = layout.clone();
    if let Some(districts) = v.get_mut("districts").and_then(|d| d.as_array_mut()) {
        for d in districts {
            if let Some(o) = d.as_object_mut() {
                o.remove("store_type");
            }
        }
    }
    sha256_hex(v.to_string().as_bytes())
}

fn flag(args: &[String], name: &str) -> Option<String> {
    let at = args.iter().position(|a| a == name)?;
    args.get(at + 1).cloned()
}

fn positional(args: &[String]) -> Vec<&String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a.starts_with("--") {
            skip = true;
        } else {
            out.push(a);
        }
    }
    out
}

fn store() -> Result<S3Store> {
    let cfg = s3_from_env()?.context(
        "set the S3_* variables and UPLOADS_PUBLIC_BASE_URL to publish (see .env.example)",
    )?;
    S3Store::new(cfg)
}

/// The command line after `art`. Prints plain words; exits non-zero (an error) on a failed
/// check, so a script can stop.
pub async fn run(args: &[String]) -> Result<()> {
    let Some((cmd, rest)) = args.split_first() else {
        bail!(
            "usage: server art check <pack> | build <pack> <out> --version <name> | publish <built> | verify <version>"
        );
    };
    let pos = positional(rest);
    match cmd.as_str() {
        "check" => {
            let [pack_dir] = pos[..] else {
                bail!("usage: server art check <pack>");
            };
            let report = pack::check(&PathBuf::from(pack_dir));
            report.print();
            if !report.ok() {
                bail!("the pack has {} problem(s)", report.errors.len());
            }
            Ok(())
        }
        "build" => {
            let [pack_dir, out] = pos[..] else {
                bail!("usage: server art build <pack> <out> --version <name>");
            };
            let version = flag(rest, "--version").context("--version <name> is required")?;
            let built = build::build(&PathBuf::from(pack_dir), &PathBuf::from(out), &version)?;
            println!(
                "built {} files ({} tiles) in {}",
                built.files,
                built.tiles,
                built.dir.display()
            );
            Ok(())
        }
        "publish" => {
            let [built] = pos[..] else {
                bail!("usage: server art publish <built folder>");
            };
            let store = Arc::new(store()?);
            let published = publish::publish(store.clone(), &PathBuf::from(built)).await?;
            let base = store
                .public_url(KEY_ROOT)
                .context("the store has no public address")?;
            println!(
                "published {} files as version {}",
                published.files, published.version
            );
            println!("to make it live, set these and restart the server:");
            println!("  MARKET_ASSETS_VERSION={}", published.version);
            println!("  MARKET_ASSETS_BASE_URL={base}");
            Ok(())
        }
        "verify" => {
            let [version] = pos[..] else {
                bail!("usage: server art verify <version> [--layout-dir <dir>]");
            };
            let base = match std::env::var("MARKET_ASSETS_BASE_URL").ok() {
                Some(b) if !b.trim().is_empty() => b,
                _ => store()?
                    .public_url(KEY_ROOT)
                    .context("set MARKET_ASSETS_BASE_URL")?,
            };
            let layout_dir = flag(rest, "--layout-dir")
                .or_else(|| std::env::var("MARKET_LAYOUT_DIR").ok())
                .unwrap_or_else(|| "market".into());
            let fetcher = verify::HttpFetcher::new(&base)?;
            let report =
                verify::verify(&fetcher, version, Some(&PathBuf::from(layout_dir))).await?;
            report.print();
            if !report.ok() {
                bail!("version {version} has {} problem(s)", report.problems.len());
            }
            Ok(())
        }
        _ => bail!("unknown art command (expected: check, build, publish, verify)"),
    }
}
