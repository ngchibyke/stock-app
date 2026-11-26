//! Part 7f: the art pipeline. A tiny synthetic pack goes through check, build, publish and
//! verify. No database is needed.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use image::{GrayImage, Rgb, RgbImage};
use serde_json::{Value, json};
use server::{
    art::{
        CACHE_CONTROL,
        build::{Manifest, build},
        pack::{check, levels_for},
        publish::{content_type, key, publish},
        verify::{Fetch, verify},
    },
    uploads::{
        fake::FakeStore,
        store::{Bucket, ObjectStore},
    },
};

const ROOM_JSON: &str = include_str!("../market/rooms/boutique/room.json");

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("art-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn picture(w: u32, h: u32, seed: u8) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| {
        Rgb([
            (x as u8).wrapping_mul(3).wrapping_add(seed),
            (y as u8).wrapping_mul(5),
            ((x ^ y) as u8).wrapping_add(seed),
        ])
    })
}

fn layout(max_level: u64) -> Value {
    json!({
        "version": "test-pack",
        "world": {"w": 512, "h": 512},
        "tile_size": 64,
        "max_level": max_level,
        "base_image": "base.jpg",
        "districts": [{"id": "fashion", "name": "Fashion", "store_type": "boutique"}],
        "plots": [
            {"id": "fashion-1", "district": "fashion", "x": 10, "y": 10, "w": 50, "h": 50},
            {"id": "fashion-2", "district": "fashion", "x": 100, "y": 10, "w": 50, "h": 50}
        ]
    })
}

/// A good pack: a 512 px market (levels 0 to 3 with 64 px tiles) and one room.
fn good_pack(dir: &Path) {
    fs::write(dir.join("layout.json"), layout(3).to_string()).unwrap();
    picture(512, 512, 1).save(dir.join("market.png")).unwrap();
    let room = dir.join("rooms/boutique");
    fs::create_dir_all(&room).unwrap();
    fs::write(room.join("room.json"), ROOM_JSON).unwrap();
    picture(200, 300, 2).save(room.join("room.png")).unwrap();
    GrayImage::from_fn(100, 150, |x, y| image::Luma([(x + y) as u8]))
        .save(room.join("depth.png"))
        .unwrap();
}

fn problems(dir: &Path) -> Vec<String> {
    check(dir).errors
}

fn any(errs: &[String], needle: &str) -> bool {
    errs.iter().any(|e| e.contains(needle))
}

#[test]
fn a_good_pack_passes_and_says_what_it_will_make() {
    let d = Dir::new("good");
    good_pack(&d.0);
    let r = check(&d.0);
    assert!(r.ok(), "{:?}", r.errors);
    let plan = r.plan.unwrap();
    assert_eq!((plan.tile_size, plan.max_level), (64, 3));
    assert_eq!(plan.room_types, ["boutique"]);
    // Two plots is well under a launch market: a warning, not a failure.
    assert!(r.warnings.iter().any(|w| w.contains("plots")));
}

#[test]
fn levels_never_enlarge_the_picture() {
    assert_eq!(levels_for(512, 64), Some(3));
    assert_eq!(levels_for(6000, 256), Some(4));
    assert_eq!(levels_for(256, 256), Some(0));
    assert_eq!(levels_for(100, 256), None);
}

#[test]
fn a_wrong_max_level_says_what_to_set() {
    let d = Dir::new("level");
    good_pack(&d.0);
    fs::write(d.0.join("layout.json"), layout(5).to_string()).unwrap();
    let e = problems(&d.0);
    assert!(any(&e, "set max_level to 3"), "{e:?}");
}

#[test]
fn every_picture_problem_is_reported_together() {
    let d = Dir::new("many");
    good_pack(&d.0);
    // A rectangular market, an RGB depth map of the wrong shape, a room json naming wrong files.
    picture(512, 256, 1).save(d.0.join("market.png")).unwrap();
    let room = d.0.join("rooms/boutique");
    picture(100, 100, 3).save(room.join("depth.png")).unwrap();
    let j = ROOM_JSON.replace("room.jpg", "interior.jpg");
    fs::write(room.join("room.json"), j).unwrap();
    let e = problems(&d.0);
    assert!(any(&e, "must be square"), "{e:?}");
    assert!(any(&e, "grey"), "{e:?}");
    assert!(any(&e, "not the same shape"), "{e:?}");
    assert!(any(&e, "room_image must be"), "{e:?}");
}

#[test]
fn missing_files_and_bad_settings_are_named() {
    let d = Dir::new("missing");
    good_pack(&d.0);
    fs::remove_file(d.0.join("market.png")).unwrap();
    fs::remove_file(d.0.join("rooms/boutique/depth.png")).unwrap();
    let mut l = layout(3);
    l["tile_size"] = json!(100);
    l["world"]["h"] = json!(300);
    fs::write(d.0.join("layout.json"), l.to_string()).unwrap();
    let e = problems(&d.0);
    assert!(any(&e, "market.png"), "{e:?}");
    assert!(any(&e, "depth.png is missing"), "{e:?}");
    assert!(any(&e, "power of two"), "{e:?}");
    assert!(any(&e, "world must be square"), "{e:?}");
    // And a layout the server itself would refuse.
    let d = Dir::new("overlap");
    good_pack(&d.0);
    let mut l = layout(3);
    l["plots"][1]["x"] = json!(20);
    fs::write(d.0.join("layout.json"), l.to_string()).unwrap();
    assert!(any(&problems(&d.0), "overlap"));
}

#[test]
fn a_build_makes_the_files_the_app_asks_for_and_is_deterministic() {
    let d = Dir::new("build");
    good_pack(&d.0.join(""));
    let out_a = Dir::new("out-a");
    let out_b = Dir::new("out-b");
    let a = build(&d.0, &out_a.0, "v1").unwrap();
    let b = build(&d.0, &out_b.0, "v1").unwrap();
    // 1 + 4 + 16 + 64 tiles, the base, the layout, the room's three files and the manifest.
    assert_eq!(a.tiles, 85);
    assert_eq!(a.files, 85 + 1 + 1 + 3 + 1);
    assert_eq!(b.files, a.files);
    let ma = fs::read(a.dir.join("manifest.json")).unwrap();
    assert_eq!(
        ma,
        fs::read(b.dir.join("manifest.json")).unwrap(),
        "same pack, same bytes"
    );
    let m: Manifest = serde_json::from_slice(&ma).unwrap();
    assert_eq!(
        (m.version.as_str(), m.tile_size, m.max_level),
        ("v1", 64, 3)
    );

    // The contract with the app: this exact list of names.
    let mut names: Vec<String> = m.files.iter().map(|f| f.path.clone()).collect();
    names.push("manifest.json".into());
    names.sort();
    let golden = include_str!("fixtures/art_files.txt");
    assert_eq!(
        names.join("\n") + "\n",
        golden,
        "the app's resolver reads these names"
    );

    // Tiles are 64 px JPEGs; the depth map is a small grey PNG; nothing carries metadata.
    let tile = image::open(a.dir.join("tiles/3/7_7.jpg")).unwrap();
    assert_eq!((tile.width(), tile.height()), (64, 64));
    let depth = image::open(a.dir.join("rooms/boutique/depth.png")).unwrap();
    assert!(depth.width() <= 512 && depth.color() == image::ColorType::L8);
    for f in &m.files {
        let bytes = fs::read(a.dir.join(&f.path)).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains("Exif"),
            "{}",
            f.path
        );
    }
}

#[test]
fn a_build_refuses_a_bad_pack_a_taken_version_and_a_bad_name() {
    let d = Dir::new("refuse");
    good_pack(&d.0);
    let out = Dir::new("out");
    build(&d.0, &out.0, "v1").unwrap();
    assert!(
        build(&d.0, &out.0, "v1")
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    for bad in ["", "a/b", "..", "has space"] {
        assert!(build(&d.0, &out.0, bad).is_err(), "{bad:?}");
    }
    fs::remove_file(d.0.join("market.png")).unwrap();
    let err = build(&d.0, &out.0, "v2").unwrap_err().to_string();
    assert!(err.contains("nothing was built"), "{err}");
    assert!(!out.0.join("v2").exists());
}

fn built(tag: &str) -> (Dir, Dir, PathBuf) {
    let pack = Dir::new(tag);
    good_pack(&pack.0);
    let out = Dir::new("out");
    let dir = build(&pack.0, &out.0, "v1").unwrap().dir;
    (pack, out, dir)
}

#[tokio::test]
async fn publishing_uploads_everything_cached_and_the_manifest_last() {
    let (_pack, _out, dir) = built("pub");
    let store = Arc::new(FakeStore::new());
    let r = publish(store.clone() as Arc<dyn ObjectStore>, &dir)
        .await
        .unwrap();
    assert_eq!((r.version.as_str(), r.files), ("v1", 91));
    let keys = store.keys(Bucket::Public);
    assert_eq!(keys.len(), 91);
    for k in &keys {
        assert!(k.starts_with("market/v1/"), "{k}");
        assert_eq!(
            store.cache_control(Bucket::Public, k).as_deref(),
            Some(CACHE_CONTROL),
            "{k}"
        );
    }
    assert_eq!(content_type("tiles/0/0_0.jpg"), "image/jpeg");
    assert_eq!(
        store
            .content_type(Bucket::Public, &key("v1", "rooms/boutique/depth.png"))
            .as_deref(),
        Some("image/png")
    );
    assert_eq!(
        store
            .content_type(Bucket::Public, &key("v1", "manifest.json"))
            .as_deref(),
        Some("application/json")
    );
    // A published version never changes.
    let err = publish(store.clone() as Arc<dyn ObjectStore>, &dir)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already published"), "{err}");
}

#[tokio::test]
async fn an_upload_cut_short_leaves_no_manifest_and_can_be_retried() {
    let (_pack, _out, dir) = built("cut");
    let store = Arc::new(FakeStore::new());
    store.fail_puts_after(20);
    let err = publish(store.clone() as Arc<dyn ObjectStore>, &dir)
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("manifest was not uploaded"),
        "{err:#}"
    );
    assert!(
        store
            .object(Bucket::Public, &key("v1", "manifest.json"))
            .is_none()
    );
    // With the store back, the same version publishes: no manifest means it never existed.
    let fresh = Arc::new(FakeStore::new());
    assert!(publish(fresh as Arc<dyn ObjectStore>, &dir).await.is_ok());
}

#[tokio::test]
async fn a_damaged_build_is_not_published() {
    let (_pack, _out, dir) = built("damaged");
    fs::write(dir.join("tiles/0/0_0.jpg"), b"not what was built").unwrap();
    let store = Arc::new(FakeStore::new());
    let err = publish(store.clone() as Arc<dyn ObjectStore>, &dir)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("does not match the manifest"),
        "{err}"
    );
    assert!(store.keys(Bucket::Public).is_empty());
}

struct StoreFetch(Arc<FakeStore>);

#[async_trait]
impl Fetch for StoreFetch {
    async fn get(&self, version: &str, path: &str) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self.0.object(Bucket::Public, &key(version, path)))
    }
}

#[tokio::test]
async fn verify_passes_a_good_version_and_catches_each_kind_of_damage() {
    let (pack, _out, dir) = built("verify");
    let store = Arc::new(FakeStore::new());
    publish(store.clone() as Arc<dyn ObjectStore>, &dir)
        .await
        .unwrap();
    let fetch = StoreFetch(store.clone());

    // The server's layout is the pack's: all good.
    let v = verify(&fetch, "v1", Some(&pack.0)).await.unwrap();
    assert!(v.ok(), "{:?}", v.problems);
    assert_eq!(v.checked, 90);

    // Nothing at all under another name.
    let v = verify(&fetch, "v2", None).await.unwrap();
    assert!(v.problems[0].contains("no manifest"), "{:?}", v.problems);

    // A changed file and a missing one.
    store.put_raw(
        Bucket::Public,
        &key("v1", "tiles/1/0_0.jpg"),
        b"x".to_vec(),
        "image/jpeg",
    );
    let missing = key("v1", "base.jpg");
    store.delete(Bucket::Public, &missing).await.unwrap();
    let v = verify(&fetch, "v1", None).await.unwrap();
    assert!(
        v.problems.iter().any(|p| p.contains("tiles/1/0_0.jpg")),
        "{:?}",
        v.problems
    );
    assert!(
        v.problems.iter().any(|p| p.contains("base.jpg is missing")),
        "{:?}",
        v.problems
    );
}

#[tokio::test]
async fn verify_says_when_the_servers_plots_are_not_the_arts() {
    let (pack, _out, dir) = built("layout");
    let store = Arc::new(FakeStore::new());
    publish(store.clone() as Arc<dyn ObjectStore>, &dir)
        .await
        .unwrap();
    // The server was left with a layout that has one more plot.
    let other = Dir::new("server-layout");
    good_pack(&other.0);
    let mut l = layout(3);
    l["plots"].as_array_mut().unwrap().push(
        json!({"id": "fashion-3", "district": "fashion", "x": 200, "y": 10, "w": 50, "h": 50}),
    );
    fs::write(other.0.join("layout.json"), l.to_string()).unwrap();
    let v = verify(&StoreFetch(store), "v1", Some(&other.0))
        .await
        .unwrap();
    assert!(
        v.problems
            .iter()
            .any(|p| p.contains("not the plots the server places stores on")),
        "{:?}",
        v.problems
    );
    drop(pack);
}

/// The real binary, as someone would run it: no database or other settings, plain words
/// on stdout, and a failing exit code when the pack is bad.
#[test]
fn the_command_line_needs_no_database_and_fails_loudly() {
    use std::process::Command;
    let bin = env!("CARGO_BIN_EXE_server");
    let run = |args: &[&str]| {
        Command::new(bin)
            .args(args)
            .env_remove("DATABASE_URL")
            .env_remove("FIELD_KEY")
            .output()
            .unwrap()
    };
    let pack = Dir::new("cli");
    good_pack(&pack.0);
    let p = pack.0.to_str().unwrap();
    let out = Dir::new("cli-out");
    let o = out.0.to_str().unwrap();

    let ok = run(&["art", "check", p]);
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(String::from_utf8_lossy(&ok.stdout).contains("the pack is good"));

    let built = run(&["art", "build", p, o, "--version", "v1"]);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert!(String::from_utf8_lossy(&built.stdout).contains("85 tiles"));
    assert!(out.0.join("v1/manifest.json").is_file());

    fs::remove_file(pack.0.join("market.png")).unwrap();
    let bad = run(&["art", "check", p]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stdout).contains("problem: market.png"));
    // Without a store configured, publishing says what to set rather than failing obscurely.
    let publish = run(&["art", "publish", out.0.join("v1").to_str().unwrap()]);
    assert!(!publish.status.success());
    assert!(String::from_utf8_lossy(&publish.stderr).contains("S3_"));
    // And a mistyped command is refused.
    assert!(!run(&["art", "nonsense"]).status.success());
}
