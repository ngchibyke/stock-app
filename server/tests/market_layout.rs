//! The market layout files: loaded once at startup, checked strictly.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use server::market::layout::Layout;
use uuid::Uuid;

const ROOMS: [&str; 3] = ["boutique", "phone_shop", "grocery"];

fn shipped() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("market")
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn write(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

/// A fresh copy of the shipped files in a temp dir.
fn copy_shipped() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("market-layout-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(shipped().join("layout.json"), dir.join("layout.json")).unwrap();
    for room in ROOMS {
        let to = dir.join("rooms").join(room);
        std::fs::create_dir_all(&to).unwrap();
        std::fs::copy(
            shipped().join("rooms").join(room).join("room.json"),
            to.join("room.json"),
        )
        .unwrap();
    }
    dir
}

/// Loads a copy of the shipped layout after `edit` changed `layout.json`.
fn load_with_layout(edit: impl FnOnce(&mut Value)) -> anyhow::Result<Layout> {
    let dir = copy_shipped();
    let path = dir.join("layout.json");
    let mut v = read(&path);
    edit(&mut v);
    write(&path, &v);
    Layout::load(&dir)
}

/// Loads a copy of the shipped layout after `edit` changed the boutique `room.json`.
fn load_with_boutique(edit: impl FnOnce(&mut Value)) -> anyhow::Result<Layout> {
    let dir = copy_shipped();
    let path = dir.join("rooms/boutique/room.json");
    let mut v = read(&path);
    edit(&mut v);
    write(&path, &v);
    Layout::load(&dir)
}

fn message(result: anyhow::Result<Layout>) -> String {
    format!("{:#}", result.expect_err("load should have failed"))
}

#[test]
fn shipped_layout_loads_with_60_plots_5_districts_3_room_types() {
    let layout = Layout::load(&shipped()).unwrap();
    assert_eq!(layout.district_ids().count(), 5);
    let plots: usize = layout
        .district_ids()
        .map(|d| layout.plots_in(d).count())
        .sum();
    assert_eq!(plots, 60);
    for room in ROOMS {
        assert_eq!(layout.room_type(room).unwrap().slots.len(), 9, "{room}");
    }
    assert!(layout.room_type("nope").is_none());
    assert_eq!(layout.district("fashion").unwrap().store_type, "boutique");
    assert_eq!(
        layout
            .room_type_for_district("phones-electronics")
            .unwrap()
            .id,
        "phone_shop"
    );
    assert_eq!(
        layout.room_type_for_district("food-groceries").unwrap().id,
        "grocery"
    );
    assert!(layout.room_type_for_district("nope").is_none());
    let first = layout.plots_in("fashion").next().unwrap();
    assert_eq!(first.id, "fashion-1");
    assert_eq!(layout.plot("fashion-1").unwrap().district, "fashion");
    assert!(layout.plot("nope").is_none());
    assert_eq!(
        layout.layout_json()["districts"][0]["store_type"],
        "boutique"
    );
    let slot = &layout.room_type("boutique").unwrap().slots[0];
    assert_eq!(slot.id, "s1");
    assert!(slot.w > 0.0 && slot.h > 0.0 && slot.depth >= 0.0 && slot.tilt_y.abs() <= 1.0);
}

#[test]
fn no_plots() {
    let err = message(load_with_layout(|v| v["plots"] = json!([])));
    assert!(err.contains("no plots"), "{err}");
}

#[test]
fn duplicate_plot_ids() {
    let err = message(load_with_layout(|v| {
        v["plots"][1]["id"] = v["plots"][0]["id"].clone();
    }));
    assert!(err.contains("duplicate plot id"), "{err}");
}

#[test]
fn duplicate_district_ids() {
    let err = message(load_with_layout(|v| {
        v["districts"][1]["id"] = v["districts"][0]["id"].clone();
    }));
    assert!(err.contains("duplicate district id"), "{err}");
}

#[test]
fn plot_outside_the_world() {
    let err = message(load_with_layout(|v| v["plots"][0]["x"] = json!(2000)));
    assert!(err.contains("outside the world"), "{err}");
}

#[test]
fn overlapping_plots() {
    let err = message(load_with_layout(|v| {
        v["plots"][1]["x"] = v["plots"][0]["x"].clone();
        v["plots"][1]["y"] = v["plots"][0]["y"].clone();
    }));
    assert!(err.contains("overlap"), "{err}");
}

#[test]
fn plot_names_unknown_district() {
    let err = message(load_with_layout(|v| {
        v["plots"][0]["district"] = json!("atlantis");
    }));
    assert!(err.contains("unknown district"), "{err}");
}

#[test]
fn district_names_unknown_room_type() {
    let err = message(load_with_layout(|v| {
        v["districts"][0]["store_type"] = json!("castle");
    }));
    assert!(err.contains("unknown room type"), "{err}");
}

#[test]
fn room_type_with_no_slots() {
    let err = message(load_with_boutique(|v| v["slots"] = json!([])));
    assert!(err.contains("no slots"), "{err}");
}

#[test]
fn slot_outside_0_to_1() {
    let err = message(load_with_boutique(|v| v["slots"][0]["x"] = json!(1.5)));
    assert!(err.contains("outside 0 to 1"), "{err}");
}

#[test]
fn missing_file() {
    let dir = copy_shipped();
    std::fs::remove_file(dir.join("rooms/grocery/room.json")).unwrap();
    let err = message(Layout::load(&dir));
    assert!(err.contains("room.json"), "{err}");
    let err = message(Layout::load(&dir.join("elsewhere")));
    assert!(err.contains("layout.json"), "{err}");
}

#[test]
fn not_json() {
    let dir = copy_shipped();
    std::fs::write(dir.join("layout.json"), "{ not json").unwrap();
    let err = message(Layout::load(&dir));
    assert!(err.contains("layout.json"), "{err}");
}

/// The server's copy must be the app's bundled art, apart from the server-only `store_type`.
#[test]
fn layout_matches_the_apps_bundled_art() {
    let app = Path::new(env!("CARGO_MANIFEST_DIR")).join("../app/assets/market");
    let mut ours = read(&shipped().join("layout.json"));
    for d in ours["districts"].as_array_mut().unwrap() {
        d.as_object_mut().unwrap().remove("store_type");
    }
    assert_eq!(ours, read(&app.join("layout.json")));
    for room in ROOMS {
        let rel = Path::new("rooms").join(room).join("room.json");
        assert_eq!(read(&shipped().join(&rel)), read(&app.join(&rel)), "{room}");
    }
}
