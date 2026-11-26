# Part 7f: The art pipeline, plan

Design: `docs/superpowers/specs/2026-11-26-07f-art-pipeline-design.md` (approved 2026-11-26).
Branch: `feat/art-pipeline`. Every task ends with its tests green and a commit in the repo's style.

## Facts found in the code

- The app asks for `{base}/{version}/base.jpg`, `tiles/{level}/{x}_{y}.jpg`, `rooms/{type}/room.jpg` and `depth.png` (`RemoteAssetResolver`). Level `L` is an `n x n` grid with `n = 2^L`, so the world and the market picture must be **square**.
- The app gets `tile_size` and `max_level` from the server's `layout.json` (`GET /market`), so the pack's `layout.json` must declare the values the build produces. `art check` computes them and says which to set.
- Rooms: `room.json` names the output files (`room.jpg`, `depth.png`), so the check requires exactly those names.
- The live version is `MARKET_ASSETS_VERSION` with `MARKET_ASSETS_BASE_URL`. Public keys are `market/<version>/...`, so the base address to set is `<public base>/market`.
- `main.rs` loads the whole server config and connects to the database before any command. The `art` commands run before that and read only the `S3_*` variables, so no database is needed.
- The object store's `put` has no cache header. A defaulted `put_cached` is added to the trait; the S3 store signs `cache-control`, the in-memory fake records it.
- `Layout::load` stops at the first layout problem, so `check` reports layout problems one at a time and every picture problem together.

## Differences from the design

- The placeholder art does not go through the tool: it is generated tile by tile by `app/tool/placeholder_art.dart`. The tool is tested on a small synthetic pack instead.
- The contract between the tool and the app is a golden file of file names (`server/tests/fixtures/art_files.txt`) that both a Rust test and a Dart test compare against.

## Tasks

1. **Module and commands.** `server/src/art/` (`pack`, `build`, `publish`, `verify`) and `server art check|build|publish|verify` handled at the top of `main`. S3 settings from the environment without the full config.
2. **Check.** Layout via `Layout::load`; world and market square; tile size a power of two from 64 to 512; the `max_level` the market picture supports (never upsampled, at most 6) must equal the layout's; the long side of the market picture at most 16384 and room pictures at most 4096; `room.json` names `room.jpg` and `depth.png`; room and depth aspect agree; depth is grey; slots inside the picture; a warning below 40 plots.
3. **Build.** Deterministic output: base picture (1024 px long side), the tile pyramid, rooms (long side at most 2048, JPEG), depth (grey PNG, at most 512), `layout.json`, each `room.json`, and `manifest.json` (version, tile size, levels, layout hash without `store_type`, size and sha256 per file). Metadata is dropped by re-encoding.
4. **Publish.** `put_cached` for the object store. Upload with a long immutable cache header, a few files at a time, the manifest last; refuse a version that already has a manifest; print the two settings.
5. **Verify.** Fetch the manifest and every file over https from the public address, compare sizes and sha256, compare the layout hash with the layout the server would load (`MARKET_LAYOUT_DIR`).
6. **Tests.** A tiny synthetic pack: expected files, determinism, each check failing for its reason, a refused existing version, the manifest uploaded last (an interrupted upload leaves none), `verify` catching a missing file, a changed file and a layout mismatch. The golden file test in Rust and Dart.
7. **Docs.** `server/market/README.md` (the real-art workflow), the app README, the roadmap's go-live items (renders without real people, invented signage), plan status.

## Checks

From `server/`: `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, the whole suite with `DATABASE_URL` set. From `app/`: `build_runner`, `dart format`, `flutter analyze`, `flutter test`. Nothing here needs a phone.
