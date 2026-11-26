//! Reading and checking an art pack. See the design for the layout of a pack.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use image::{ColorType, ImageReader};
use serde_json::Value;

use crate::market::layout::Layout;

/// The longest side the market picture may have, and the longest side of a room picture.
pub const MAX_MARKET_PX: u32 = 16_384;
pub const MAX_ROOM_PX: u32 = 4_096;
/// The deepest tile level the app is asked to show.
pub const MAX_LEVEL: u32 = 6;
pub const MIN_PLOTS: usize = 40;
/// The names `room.json` must give its output files.
pub const ROOM_IMAGE: &str = "room.jpg";
pub const DEPTH_IMAGE: &str = "depth.png";
pub const BASE_IMAGE: &str = "base.jpg";

/// What a check found. Every picture problem is listed; a layout problem is the first one
/// the server's own loader finds.
#[derive(Debug, Default)]
pub struct Report {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// What a build will produce, when the pack is good enough to say.
    pub plan: Option<Plan>,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub tile_size: u32,
    pub max_level: u32,
    pub market_file: PathBuf,
    pub room_types: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn print(&self) {
        for w in &self.warnings {
            println!("warning: {w}");
        }
        for e in &self.errors {
            println!("problem: {e}");
        }
        if self.ok() {
            println!("the pack is good");
        }
    }

    fn err(&mut self, msg: impl Into<String>) {
        self.errors.push(msg.into());
    }
}

/// The first of `stem.png`, `stem.jpg`, `stem.jpeg` that exists.
pub fn find_picture(dir: &Path, stem: &str) -> Option<PathBuf> {
    ["png", "jpg", "jpeg"]
        .iter()
        .map(|ext| dir.join(format!("{stem}.{ext}")))
        .find(|p| p.is_file())
}

fn dimensions(path: &Path) -> Result<(u32, u32), String> {
    ImageReader::open(path)
        .map_err(|_| format!("{} cannot be opened", name(path)))?
        .with_guessed_format()
        .map_err(|_| format!("{} is not a picture", name(path)))?
        .into_dimensions()
        .map_err(|_| {
            format!(
                "{} is not a picture the tool can read (use PNG or JPEG)",
                name(path)
            )
        })
}

fn name(path: &Path) -> String {
    let parts: Vec<_> = path.components().rev().take(3).collect();
    parts
        .iter()
        .rev()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// The deepest level a square picture of `px` can fill without being enlarged.
pub fn levels_for(px: u32, tile: u32) -> Option<u32> {
    if tile == 0 || px < tile {
        return None;
    }
    Some(((px / tile) as f64).log2().floor() as u32)
}

pub fn check(dir: &Path) -> Report {
    let mut r = Report::default();
    let layout_path = dir.join("layout.json");
    let layout_json: Value = match std::fs::read(&layout_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
    {
        Some(v) => v,
        None => {
            r.err("layout.json is missing or is not valid JSON");
            return r;
        }
    };
    // The server's own rules: the same loader it runs at start-up.
    if let Err(e) = Layout::load(dir) {
        r.err(format!("layout: {e:#}"));
    }
    let world_w = layout_json["world"]["w"].as_f64().unwrap_or(0.0);
    let world_h = layout_json["world"]["h"].as_f64().unwrap_or(0.0);
    if world_w <= 0.0 || (world_w - world_h).abs() > f64::EPSILON {
        r.err("the world must be square (world.w equal to world.h): the app draws square tiles");
    }
    let tile = layout_json["tile_size"].as_u64().unwrap_or(0) as u32;
    if !(64..=512).contains(&tile) || !tile.is_power_of_two() {
        r.err("tile_size must be a power of two from 64 to 512 (256 is usual)");
    }
    if layout_json["base_image"].as_str() != Some(BASE_IMAGE) {
        r.err(format!("base_image must be \"{BASE_IMAGE}\""));
    }
    let plots = layout_json["plots"].as_array().map_or(0, Vec::len);
    if plots < MIN_PLOTS {
        r.warnings.push(format!(
            "the layout has {plots} plots; the launch market is about 60 (at least {MIN_PLOTS} is expected)"
        ));
    }

    // The market picture.
    let mut max_level = None;
    let market_file = find_picture(dir, "market");
    match &market_file {
        None => r.err("market.png (or market.jpg) is missing"),
        Some(p) => match dimensions(p) {
            Err(e) => r.err(e),
            Ok((w, h)) => {
                if w != h {
                    r.err(format!("market picture is {w} x {h}; it must be square"));
                } else if w > MAX_MARKET_PX {
                    r.err(format!(
                        "market picture is {w} px wide; the most is {MAX_MARKET_PX}. Render it smaller"
                    ));
                } else if let Some(level) = levels_for(w, tile.max(1)) {
                    let level = level.min(MAX_LEVEL);
                    max_level = Some(level);
                    let declared = layout_json["max_level"].as_u64();
                    if declared != Some(u64::from(level)) {
                        r.err(format!(
                            "layout.json says max_level {} but a {w} px market picture with {tile} px tiles gives {level}: set max_level to {level}",
                            declared.map_or("nothing".into(), |d| d.to_string())
                        ));
                    }
                } else {
                    r.err(format!(
                        "market picture ({w} px) is smaller than one {tile} px tile"
                    ));
                }
            }
        },
    }

    // Rooms, for every type a district names.
    let types: BTreeSet<String> = layout_json["districts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["store_type"].as_str().map(str::to_string))
        .collect();
    for t in &types {
        check_room(dir, t, &mut r);
    }

    if r.ok()
        && let (Some(level), Some(market_file)) = (max_level, market_file)
    {
        r.plan = Some(Plan {
            tile_size: tile,
            max_level: level,
            market_file,
            room_types: types.into_iter().collect(),
        });
    }
    r
}

fn check_room(dir: &Path, t: &str, r: &mut Report) {
    let room_dir = dir.join("rooms").join(t);
    let json_path = room_dir.join("room.json");
    match std::fs::read(&json_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    {
        None => r.err(format!(
            "rooms/{t}/room.json is missing or is not valid JSON"
        )),
        Some(j) => {
            if j["room_image"].as_str() != Some(ROOM_IMAGE) {
                r.err(format!(
                    "rooms/{t}/room.json: room_image must be \"{ROOM_IMAGE}\""
                ));
            }
            if j["depth_image"].as_str() != Some(DEPTH_IMAGE) {
                r.err(format!(
                    "rooms/{t}/room.json: depth_image must be \"{DEPTH_IMAGE}\""
                ));
            }
        }
    }
    let room = find_picture(&room_dir, "room");
    let depth = room_dir.join("depth.png");
    let room_dim = match &room {
        None => {
            r.err(format!("rooms/{t}/room.png (or room.jpg) is missing"));
            None
        }
        Some(p) => match dimensions(p) {
            Ok(d) if d.0.max(d.1) > MAX_ROOM_PX => {
                r.err(format!(
                    "rooms/{t}: the room picture is {} x {}; the longest side may be {MAX_ROOM_PX}",
                    d.0, d.1
                ));
                None
            }
            Ok(d) => Some(d),
            Err(e) => {
                r.err(e);
                None
            }
        },
    };
    if !depth.is_file() {
        r.err(format!("rooms/{t}/depth.png is missing"));
        return;
    }
    match image::open(&depth) {
        Err(_) => r.err(format!(
            "rooms/{t}/depth.png is not a picture the tool can read"
        )),
        Ok(img) => {
            if !is_grey(&img) {
                r.err(format!("rooms/{t}/depth.png must be a grey picture"));
            }
            if let Some((rw, rh)) = room_dim {
                let (dw, dh) = (img.width(), img.height());
                // Same shape, to a pixel's worth of rounding.
                let off = (f64::from(rw) / f64::from(rh) - f64::from(dw) / f64::from(dh)).abs();
                if off > 0.01 {
                    r.err(format!(
                        "rooms/{t}: the depth map ({dw} x {dh}) is not the same shape as the room picture ({rw} x {rh})"
                    ));
                }
            }
        }
    }
}

/// Grey, or colour with equal channels (some tools save a depth map as RGB).
fn is_grey(img: &image::DynamicImage) -> bool {
    match img.color() {
        ColorType::L8 | ColorType::L16 | ColorType::La8 | ColorType::La16 => true,
        _ => img
            .to_rgb8()
            .pixels()
            .all(|p| p.0[0] == p.0[1] && p.0[1] == p.0[2]),
    }
}
