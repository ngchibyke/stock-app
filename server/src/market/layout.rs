//! The market's fixed layout: districts, plots on the world map, and the shop
//! rooms with their display slots. Read once at startup from `layout.json` and
//! `rooms/<type>/room.json`, checked strictly (the app's own parser is
//! forgiving; the server is the authority), then held immutably in `AppState`.
//! Holds no personal data.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct District {
    pub id: String,
    pub name: String,
    /// The room type a shop in this district gets.
    pub store_type: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Plot {
    pub id: String,
    pub district: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A display spot inside a room. Position, size and depth are fractions of the room
/// image (0 to 1); `tilt_y` is a signed lean.
#[derive(Debug, Clone, Deserialize)]
pub struct Slot {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub depth: f64,
    pub tilt_y: f64,
}

#[derive(Debug, Clone)]
pub struct RoomType {
    pub id: String,
    /// The `room.json`, verbatim.
    pub json: Value,
    pub slots: Vec<Slot>,
}

#[derive(Deserialize)]
struct World {
    w: f64,
    h: f64,
}

#[derive(Deserialize)]
struct LayoutFile {
    world: World,
    districts: Vec<District>,
    plots: Vec<Plot>,
}

#[derive(Deserialize)]
struct RoomFile {
    id: String,
    slots: Vec<Slot>,
}

#[derive(Debug)]
pub struct Layout {
    json: Value,
    districts: Vec<District>,
    plots: Vec<Plot>,
    plot_index: HashMap<String, usize>,
    rooms: HashMap<String, RoomType>,
}

fn read_json(path: &Path) -> Result<Value> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("{} is not valid JSON", path.display()))
}

fn parse<T: serde::de::DeserializeOwned>(value: &Value, path: &Path) -> Result<T> {
    serde_json::from_value(value.clone())
        .with_context(|| format!("{} has the wrong shape", path.display()))
}

fn in_unit(v: f64) -> bool {
    v.is_finite() && (0.0..=1.0).contains(&v)
}

/// A room type name is a folder name: no separators or dots, so a layout file can
/// never point outside `rooms/`.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn load_room(dir: &Path, id: &str) -> Result<RoomType> {
    let path = dir.join("rooms").join(id).join("room.json");
    let json = read_json(&path)?;
    let file: RoomFile = parse(&json, &path)?;
    if file.id != id {
        bail!(
            "{} names room type {:?}, expected {id:?}",
            path.display(),
            file.id
        );
    }
    if file.slots.is_empty() {
        bail!("room type {id} has no slots");
    }
    let mut seen = HashSet::new();
    for s in &file.slots {
        if !seen.insert(s.id.as_str()) {
            bail!("room type {id} has a duplicate slot id {:?}", s.id);
        }
        let fractions = [s.x, s.y, s.w, s.h, s.depth];
        if !fractions.into_iter().all(in_unit)
            || s.w <= 0.0
            || s.h <= 0.0
            || s.x + s.w > 1.0
            || s.y + s.h > 1.0
        {
            bail!("slot {:?} of room type {id} is outside 0 to 1", s.id);
        }
        if !s.tilt_y.is_finite() {
            bail!(
                "slot {:?} of room type {id} has a tilt that is not a number",
                s.id
            );
        }
    }
    Ok(RoomType {
        id: id.to_string(),
        json,
        slots: file.slots,
    })
}

impl Layout {
    /// Reads and checks `dir/layout.json` and the `rooms/<type>/room.json` of every
    /// room type a district names. Any problem is an error naming it.
    pub fn load(dir: &Path) -> Result<Layout> {
        let path = dir.join("layout.json");
        let json = read_json(&path)?;
        let file: LayoutFile = parse(&json, &path)?;

        if file.plots.is_empty() {
            bail!("layout has no plots");
        }
        let mut district_ids = HashSet::new();
        for d in &file.districts {
            if !district_ids.insert(d.id.as_str()) {
                bail!("duplicate district id {:?}", d.id);
            }
        }
        let mut plot_index = HashMap::new();
        for (i, p) in file.plots.iter().enumerate() {
            if plot_index.insert(p.id.clone(), i).is_some() {
                bail!("duplicate plot id {:?}", p.id);
            }
            if !district_ids.contains(p.district.as_str()) {
                bail!("plot {:?} names an unknown district {:?}", p.id, p.district);
            }
            let finite = [p.x, p.y, p.w, p.h].into_iter().all(f64::is_finite);
            if !finite
                || p.w <= 0.0
                || p.h <= 0.0
                || p.x < 0.0
                || p.y < 0.0
                || p.x + p.w > file.world.w
                || p.y + p.h > file.world.h
            {
                bail!("plot {:?} is outside the world", p.id);
            }
        }
        for (i, a) in file.plots.iter().enumerate() {
            for b in &file.plots[i + 1..] {
                // Touching edges are fine; sharing area is not.
                if a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h {
                    bail!("plots {:?} and {:?} overlap", a.id, b.id);
                }
            }
        }

        let mut rooms = HashMap::new();
        for d in &file.districts {
            if rooms.contains_key(&d.store_type) {
                continue;
            }
            if !is_plain_name(&d.store_type) || !dir.join("rooms").join(&d.store_type).is_dir() {
                bail!(
                    "district {:?} names an unknown room type {:?}",
                    d.id,
                    d.store_type
                );
            }
            rooms.insert(d.store_type.clone(), load_room(dir, &d.store_type)?);
        }

        Ok(Layout {
            json,
            districts: file.districts,
            plots: file.plots,
            plot_index,
            rooms,
        })
    }

    /// `layout.json` verbatim, `store_type` included.
    pub fn layout_json(&self) -> &Value {
        &self.json
    }

    pub fn district(&self, id: &str) -> Option<&District> {
        self.districts.iter().find(|d| d.id == id)
    }

    /// The district's plots, in file order.
    pub fn plots_in<'a>(&'a self, district: &str) -> impl Iterator<Item = &'a Plot> {
        let district = district.to_string();
        self.plots.iter().filter(move |p| p.district == district)
    }

    pub fn plot(&self, id: &str) -> Option<&Plot> {
        self.plot_index.get(id).map(|&i| &self.plots[i])
    }

    pub fn room_type(&self, id: &str) -> Option<&RoomType> {
        self.rooms.get(id)
    }

    pub fn room_type_for_district(&self, district: &str) -> Option<&RoomType> {
        self.room_type(&self.district(district)?.store_type)
    }

    pub fn district_ids(&self) -> impl Iterator<Item = &str> {
        self.districts.iter().map(|d| d.id.as_str())
    }
}
