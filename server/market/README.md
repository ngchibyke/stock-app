# Market layout

The map the server places stores on, and the rooms they open into. Loaded at start-up from
`MARKET_LAYOUT_DIR` (default `market`, relative to the working directory). A layout that is
missing or unusable (no plots, duplicate ids, plots outside the world or overlapping, a plot
naming an unknown district, a district naming an unknown or slotless room type, a slot outside
0 to 1) stops the server, for every command.

## How the files are made

They are copies of the app's placeholder art layout in `app/assets/market/` (`layout.json` and
`rooms/<type>/room.json`), with one addition: each district carries `store_type`, the room type
its stores get. The app ignores that key. The test `layout_matches_the_apps_bundled_art`
(`server/tests/market_layout.rs`) compares the two, minus `store_type`, and fails if they drift.
It reads `../app/assets/market`, so it needs the whole repository checked out.

If you change one side, change the other the same way. Real art (7f) will replace both.

## Adding plots or a district

- A plot: add an entry to `plots` in `layout.json` (unique `id`, an existing `district`, inside
  the world, not overlapping another plot). Do the same in the app's copy.
- A district: add it to `districts` with a `store_type` that has a folder under `rooms/`, then
  give it plots. The app's copy gets the district without `store_type`.
- Existing stores keep their plots. A new plot is picked up by waiting stores at the next
  start-up, or when a plot in that district is released.

## Placement

A store gets a plot when it first becomes active: the first free plot in its district, in the
order the plots appear in `layout.json`. Reordering plots changes who gets which plot next, not
who already has one.

When a district is full the store stays active and listed everywhere except the map, and
`market.district_full` is written to the audit log. It takes the next plot that frees up
(oldest waiting store first), or one at the next start-up.

## Real art (part 7f)

The owner makes the art in Unreal and hands over an *art pack*. `server art ...` (no database
needed) checks it, cuts it, publishes it and verifies it. Design:
`docs/superpowers/specs/2026-11-26-07f-art-pipeline-design.md`.

```
pack/
  layout.json            the server's layout file (districts carry store_type), with the right max_level
  market.png             one square top-down render of the whole market (at most 16384 px)
  rooms/<type>/
    room.json            slots and light; room_image must be "room.jpg", depth_image "depth.png"
    room.png             the interior (at most 4096 px)
    depth.png            a grey depth map, the same shape as the room picture
```

The world in `layout.json` must be square, `tile_size` a power of two from 64 to 512 and
`base_image` `base.jpg`. `max_level` must be what the market picture supports without being
enlarged: `floor(log2(pixels / tile_size))`, at most 6. `art check` computes it and says which
value to set.

```
server art check <pack>                          # every picture problem at once, then the layout's first
server art build <pack> <out> --version <name>   # writes <out>/<name>/ with a manifest of checksums
server art publish <out>/<name>                  # needs the S3_* variables; prints the two settings
server art verify <name>                         # fetches from the public address and compares
```

`publish` uploads every file with a long immutable cache lifetime and the manifest last, and
refuses a version that already has one. It prints:

```
MARKET_ASSETS_VERSION=<name>
MARKET_ASSETS_BASE_URL=<public base>/market
```

Setting those and restarting the server makes the version live; setting the old name back
restarts into the old art. The app puts the version in every picture address, so versions never
mix in its cache.

**The layout and the art travel together.** Copy the pack's `layout.json` and `rooms/*/room.json`
into this folder (and the app's bundled copy while it still ships placeholders), then restart.
`art verify <name>` compares the published art's layout hash with the layout this server loads
(`--layout-dir`, default `MARKET_LAYOUT_DIR`) and says plainly when they differ.

Pictures are JPEG (depth maps PNG) for now: the image library only writes lossless WebP, which
is larger than JPEG for photographs.
