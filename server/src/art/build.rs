//! Cutting a checked pack into the files the app downloads, deterministically: the same
//! pack always gives the same bytes. Pictures are re-encoded, so no tool or camera
//! metadata is carried over.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageEncoder, codecs::jpeg::JpegEncoder, imageops::FilterType};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    layout_hash,
    pack::{self, BASE_IMAGE, find_picture},
    sha256_hex,
};
use crate::app_config::assets_version;

const TILE_QUALITY: u8 = 82;
const BASE_QUALITY: u8 = 80;
const ROOM_QUALITY: u8 = 85;
const BASE_PX: u32 = 1024;
const ROOM_PX: u32 = 2048;
const DEPTH_PX: u32 = 512;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct FileEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// `manifest.json`: everything a verifier needs to check a version from the outside.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub version: String,
    pub tile_size: u32,
    pub max_level: u32,
    pub layout_sha256: String,
    pub files: Vec<FileEntry>,
}

#[derive(Debug)]
pub struct Built {
    pub dir: PathBuf,
    pub files: usize,
    pub tiles: usize,
}

fn jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality)
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .context("encode jpeg")?;
    Ok(out)
}

fn png_grey(img: &DynamicImage) -> Result<Vec<u8>> {
    let grey = img.to_luma8();
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            grey.as_raw(),
            grey.width(),
            grey.height(),
            image::ExtendedColorType::L8,
        )
        .context("encode png")?;
    Ok(out)
}

/// `img` scaled down so its longest side is at most `px`; never enlarged.
fn fit(img: &DynamicImage, px: u32) -> DynamicImage {
    let long = img.width().max(img.height());
    if long <= px {
        return img.clone();
    }
    let scale = f64::from(px) / f64::from(long);
    let w = ((f64::from(img.width()) * scale).round() as u32).max(1);
    let h = ((f64::from(img.height()) * scale).round() as u32).max(1);
    img.resize_exact(w, h, FilterType::Lanczos3)
}

struct Out {
    root: PathBuf,
    files: Vec<FileEntry>,
}

impl Out {
    fn write(&mut self, rel: &str, bytes: Vec<u8>) -> Result<()> {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().context("no parent")?).context("make folder")?;
        fs::write(&path, &bytes).with_context(|| format!("write {rel}"))?;
        self.files.push(FileEntry {
            path: rel.to_string(),
            bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
        Ok(())
    }
}

/// Builds `<out>/<version>/`. Refuses a pack that fails its checks and a folder that
/// already holds that version.
pub fn build(pack_dir: &Path, out_dir: &Path, version: &str) -> Result<Built> {
    assets_version("--version", version.to_string())?;
    let report = pack::check(pack_dir);
    if !report.ok() {
        report.print();
        bail!(
            "the pack has {} problem(s); nothing was built",
            report.errors.len()
        );
    }
    let plan = report.plan.context("a good pack has a plan")?;
    let dir = out_dir.join(version);
    if dir.exists() {
        bail!(
            "{} already exists; choose a new version name",
            dir.display()
        );
    }
    let mut out = Out {
        root: dir.clone(),
        files: Vec::new(),
    };

    // The market: the deepest level first, then each coarser level from the one below.
    let market = image::open(&plan.market_file).context("open the market picture")?;
    let deepest_px = plan.tile_size << plan.max_level;
    let mut level_img = if market.width() == deepest_px {
        market.clone()
    } else {
        market.resize_exact(deepest_px, deepest_px, FilterType::Lanczos3)
    };
    let mut tiles = 0;
    for level in (0..=plan.max_level).rev() {
        let n = 1u32 << level;
        for ty in 0..n {
            for tx in 0..n {
                let tile = level_img.crop_imm(
                    tx * plan.tile_size,
                    ty * plan.tile_size,
                    plan.tile_size,
                    plan.tile_size,
                );
                out.write(
                    &format!("tiles/{level}/{tx}_{ty}.jpg"),
                    jpeg(&tile, TILE_QUALITY)?,
                )?;
                tiles += 1;
            }
        }
        if level > 0 {
            let half = plan.tile_size << (level - 1);
            level_img = level_img.resize_exact(half, half, FilterType::Lanczos3);
        }
    }
    out.write(BASE_IMAGE, jpeg(&fit(&market, BASE_PX), BASE_QUALITY)?)?;

    // Rooms.
    for t in &plan.room_types {
        let room_dir = pack_dir.join("rooms").join(t);
        let room = image::open(find_picture(&room_dir, "room").context("room picture")?)
            .with_context(|| format!("open the {t} room"))?;
        out.write(
            &format!("rooms/{t}/{}", pack::ROOM_IMAGE),
            jpeg(&fit(&room, ROOM_PX), ROOM_QUALITY)?,
        )?;
        let depth = image::open(room_dir.join("depth.png"))
            .with_context(|| format!("open the {t} depth map"))?;
        out.write(
            &format!("rooms/{t}/{}", pack::DEPTH_IMAGE),
            png_grey(&fit(&depth, DEPTH_PX))?,
        )?;
        out.write(
            &format!("rooms/{t}/room.json"),
            fs::read(room_dir.join("room.json")).context("read room.json")?,
        )?;
    }
    let layout_bytes = fs::read(pack_dir.join("layout.json")).context("read layout.json")?;
    let layout: Value = serde_json::from_slice(&layout_bytes).context("layout.json")?;
    out.write("layout.json", layout_bytes)?;

    out.files.sort_by(|a, b| a.path.cmp(&b.path));
    let manifest = Manifest {
        version: version.to_string(),
        tile_size: plan.tile_size,
        max_level: plan.max_level,
        layout_sha256: layout_hash(&layout),
        files: out.files.clone(),
    };
    let count = out.files.len() + 1;
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).context("manifest")?,
    )
    .context("write manifest.json")?;
    Ok(Built {
        dir,
        files: count,
        tiles,
    })
}
