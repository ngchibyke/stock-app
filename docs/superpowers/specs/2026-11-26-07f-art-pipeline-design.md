# Part 7f: The art pipeline, design

Status: draft for owner approval, 2026-11-26. Nothing is planned or built until it is approved.
Builds on: 7c (the immersive market and its `AssetResolver`), 7d (layout, plots, rooms and the `assets` block in `/app/config`). Read with section 9 of `2026-11-21-immersive-market-design.md`.

## 1. Intent

The owner makes the real market art in Unreal and hands over files. This part is the tool that checks those files, cuts them into what the phone downloads, publishes them under a version, and lets the app switch to the new version without an app release. Until real art exists the app keeps using the placeholder art, and nothing about publishing changes.

Decisions made with the owner on 2026-11-26:

- The art comes from **Unreal renders** made offline by the owner: one top-down market image, and one interior image per store type.
- Publishing is a **command-line tool** run by the owner or a developer. No dashboard page.
- The launch market is **about 60 plots in 4 to 5 districts** (the placeholder's shape).
- No generated art and no video: the fly-in is the app's own camera over tiles and rooms, as approved in the market design.

## 2. Goals, non-goals, anti-goals

**Goals**

- One command checks an "art pack" and says in plain words what is wrong, before anything is cut or uploaded.
- One command cuts the pack into the exact files the app already loads (`base.jpg`, `tiles/{level}/{x}_{y}.jpg`, `rooms/{type}/room.jpg`, `depth.png`, the layout and room files) and writes a manifest with a checksum for every file.
- One command publishes a built version to the public bucket. A half-uploaded version is never visible.
- One command verifies a published version from the outside (the public address, every file, every checksum), and says if the live layout and the published art disagree.
- Old versions stay in the bucket, so going back is changing the live version, nothing else.

**Non-goals**

- No rendering. Unreal does that offline; nothing here runs on a phone.
- No dashboard upload page, no editing of art in the browser.
- No change to how the app draws the market, rooms or depth. The resolver and the screens stay as they are.
- No lossy WebP or AVIF in this version (section 8).

**Anti-goals**

- No version goes live that failed its checks.
- No personal data in the art or the tool's output. Renders must not contain real people's faces or real people's names.
- No overwriting a published version: a version name is written once.

## 3. The art pack (what the owner hands over)

```
pack/
  layout.json            world size, districts, plots (the same file the server reads)
  market.png             one top-down render of the whole market, same aspect as the world
  rooms/<type>/
    room.json            slots, light tint, shadow opacity (as today)
    room.png             the interior, same aspect for every type
    depth.png            grey depth map, same size or smaller, same aspect as room.png
```

Input formats: PNG or JPEG for pictures, 8 or 16 bit grey PNG for depth. Sizes are not fixed: the tool picks the tile levels from the market image.

## 4. The tool

Four subcommands of the server binary, `server art ...`, so the same checks the server runs at start-up (the layout rules: no overlapping plots, unknown districts, slots outside 0 to 1) are the ones the art must pass, and the existing object-store code is reused.

1. `art check <pack>`: reads everything and reports every problem, not just the first. It checks the layout rules; that each district's `store_type` has a room folder; that `room.png` and `depth.png` share an aspect ratio; that `market.png` matches the world's aspect; that no picture is larger than a cap (so a 20,000 px render fails early and says so); that the depth map is grey; that slots sit inside the picture; that there are enough plots (a warning, not an error, below the launch size).
2. `art build <pack> <out> --version <name>`: runs `check`, then writes `<out>/<name>/` with the base picture, the tile pyramid, the rooms, the depth maps, the layout and room files and `manifest.json` (version, file list, size and sha256 of each, a hash of the layout, tile size and levels). Pictures are re-encoded as JPEG, so no camera or tool metadata is carried over.
3. `art publish <out>/<name>`: uploads every file with a long immutable cache lifetime, the manifest **last**. If the version already exists, it refuses. The tool prints the two settings that make it live.
4. `art verify <name>`: fetches the manifest from the public address, then every file, and compares sizes and checksums. It also compares the manifest's layout hash with the layout the server was started with, and says plainly if plots in the published art are not the plots the server places stores on.

## 5. Going live and going back

The live version is the server's `MARKET_ASSETS_VERSION` (with `MARKET_ASSETS_BASE_URL`), already served in `/app/config`. Going live, or back to an older version, is changing that setting and restarting the server. The app puts the version in every image address, so new art never mixes with cached old art. This keeps one source of truth and no new table; a runtime switch can come later if restarts are too slow.

When the art changes the plots, the server's layout files change with it, in the same release. `art verify` is what notices if they were not.

## 6. App

No screen changes. One test builds a small pack with the tool, then checks that every address `RemoteAssetResolver` can ask for exists in the output. That is the contract that keeps the tool and the app in step.

## 7. Testing and checks

- Tool: a tiny synthetic pack builds to the expected tiles, names and checksums; each check fails for the right reason and says so; a version that already exists is refused; the manifest is uploaded last (an interrupted upload leaves no manifest); `verify` catches a missing file, a changed file and a layout mismatch.
- The placeholder art goes through the same tool, so it is the first pack that has to pass.
- All CI jobs as for every part. The existing `layout_matches_the_apps_bundled_art` test stays.

## 8. Open questions (defaults stand unless changed)

- **Picture format.** The market design prefers WebP or AVIF for size. The Rust image library only writes lossless WebP, which is larger than JPEG for photos. Default: JPEG for pictures and PNG for depth now, WebP later behind the same manifest.
- **Size caps.** Default: market image up to 16,384 px on the long side, room images up to 4,096 px. Tiles are 256 px as today.
- **Brand names on signs.** The concept stills show real signage. Renders should use invented names, or the owner needs permission. A legal question, listed in section 9.

## 9. For DPO and legal (not legal advice)

- Renders must not show real people. Real brand names or logos in signage need permission or invention.
- The bucket and CDN are processors only of public art; nothing about a person is in it.

## 10. Left to the plan

How the subcommands are wired into `main`, adding a cache-lifetime header to the object store's `put`, the exact tile-level maths, and the fixture packs.
