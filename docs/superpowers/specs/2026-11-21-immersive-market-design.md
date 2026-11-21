# The Immersive Market: Design

Date: 2026-11-21
Status: Approved by the owner 2026-11-21 (design only; nothing in it is built yet). The prototype that settled the feel was throwaway and is not in the repository.
Part of: the Flutter app (part 7). Read with `2026-10-22-abuja-marketplace-design.md` (the product) and `docs/superpowers/plans/2026-11-20-07-mobile-app.md` (the app overview).

## 1. Intent

The owner wants buying here to feel like walking into a real market, not scrolling a catalogue: you look down on the whole market like a satellite, zoom toward a store, the camera takes you in, and you see its goods on the shelves and carry a cart between stores. The reference is the 3D seat views on stadium ticket sites, where the camera moves for you and the place looks real. It has to be attractive enough that people want to open it, and it has to be fast and seamless, because the buyers are on mid-range Android phones and mobile data in Abuja.

Why it matters: the product's promise is trust between strangers (escrow, curated sellers, honest tracking). A market that looks and feels like a real place, and shows real people's real goods, supports that. It is also the thing that makes this app different from "just another shop app".

## 2. Goals, non-goals, anti-goals

**Goals**
- A photoreal bird's-eye market that zooms smoothly to a store and enters it with one continuous camera move.
- Two ways to start the move: pinch and drag, or tap a glowing store and be flown in.
- A store room where a seller's published products appear on the shelves, and the buyer's cart stays with them in every store.
- A seller uploads goods once, in the app, and they show up in their store's room for every buyer.
- Fast and seamless: the first view appears quickly on a mid-range phone over 4G, motion stays smooth, and nothing makes the buyer wait on a spinner during the journey.

**Non-goals (not in the first version)**
- Headset VR, a free-walking 3D world, or live 3D rendering on the phone.
- 3D models of products, or sellers supplying anything but ordinary photos.
- Cut-out products standing on shelves (planned as a later upgrade, see section 6).
- A map of real Abuja streets. The market is a designed place, not a geographic map.

**Anti-goals (a failure even if it works)**
- A look that is flat, cartoonish or "templated" (the product spec's rule). The approved look is photoreal.
- The journey getting in the way: the buyer must always be able to search, use a plain list and check out quickly. The immersive layer is how you discover and enter stores, not how you pay.
- Anything that needs a powerful phone, a huge download, or lots of data to browse.
- Any path that pushes a buyer to pay outside the app, or shows personal data (the product spec's rules still apply in full).

## 3. The experience

```
 from above               zoomed to a block          inside a store
 [market, districts,  ->  [signs, ratings,       ->  [room with shelves,
  glowing open dots]       the one you head for]       product cards, your cart]
  pinch / drag / tap       camera locks on             drag to look around
```

1. **The market from above.** One photoreal top-down view of the whole market, in districts by category. Glowing dots mark stores that are open now; grey dots mark plots where no store has opened ("coming soon"). Names, ratings and the dots are drawn by the app on top of the picture.
2. **Getting in.** Either zoom onto a glowing store until the camera commits, or tap it and be flown in. Same camera path both ways: sky, block, shopfront, door, room, with a warm light fading up as you enter.
3. **The store room.** The store's type decides the room (boutique, phone shop, grocery and so on). The seller's products sit on the shelves as cards. Drag or tilt to look around; the view has real depth. Tap a card to see the product (photos, price, sizes or variants) and add it to the cart.
4. **The cart is carried.** It stays visible in every room and on the street, grouped by store at checkout as today. Checkout and payment are the plain, fast screens already planned, not part of the scene.
5. **Back out.** "Back to market" reverses the move. A **list view** is always one tap away for people who want to search or are on a weak phone; it opens a store directly with no flight.

## 4. The look

Approved by the owner on 2026-11-21 from four photoreal concept stills of an Abuja market: golden-hour aerial of a dense market district with real signage, umbrellas, motorbikes and tiled and corrugated roofs; a block of shopfronts; a boutique entrance with Ankara outfits; and a first-person view inside a warm, well-lit boutique with a woven basket. Those stills were generated at 1k, low quality, only to agree the look. The final art must be sharp at the deepest zoom and keep that realism: believable materials, light and people, set in Abuja.

## 5. Approaches considered

| | What it is | Trade-off |
|---|---|---|
| 1 | Photoreal map tiles plus pre-made fly-in clips | Looks the most real and runs everywhere, but each store's room is its type's room, and there are no real depth cues in the room. |
| 2 | Live 3D in the app (Unreal, Unity or Flutter 3D) | A free camera, but hundreds of megabytes, hot and slow on the minimum phones (Android 8, iOS 15), and every store needs real 3D assets. Rejected. |
| 3 | **Chosen.** Tiles for the market, and depth-aware interiors | The market works as in 1. Each interior is a photoreal image with a depth map, so the camera genuinely shifts perspective as you drag or tilt, with products pinned in place. Light on phones, and better art can be swapped in later. Costs more engineering, and depth maps can show small glitches at edges. |

Chosen by the owner: approach 3, with speed and a seamless experience as the stated priority. Unreal Engine was offered by the owner for making the art; it is a good fit for rendering the tiles and interiors offline, and not for running on the phone.

## 6. The design

Everything in this section was decided in the conversation unless marked **(my call)**.

**Market view.** A zoomable, pannable picture streamed as tiles at several zoom levels, like a satellite map, so zooming stays sharp and only what is on screen is downloaded. Stores are not baked into the picture. A fixed set of **plots** is defined on top of it (position, size, district); the app draws the sign, rating and glow for a plot that has a store, and a grey "coming soon" marker otherwise. So the images never change when a store opens or closes. **(my call)**

**Entering.** One camera path used by both the zoom and the tap. Zooming onto an open store past a threshold, with the store near the centre of the screen, commits the move. **(my call: thresholds, timings of about 1.2 to 1.5 seconds, and a short pause after leaving so it does not re-enter.)** The prototype used a threshold of about 3.4 times zoom with the store near the middle.

**Store rooms.** A small set of **store types**, each one a photoreal interior picture with a depth map, a list of **shelf slots** (positions and sizes where a product can sit), and lighting values for styling the cards. A store has one type, chosen when it is opened. **(my call: start with boutique, phone shop and grocery, and add types as demand shows.)** The first room used in the prototype is the boutique.

**Products on shelves: cards first.** A product is shown as its seller's own photo in a small card styled to belong there: matched light, perspective for its shelf slot, a soft shadow, a price tag. Cards drift slightly against the room as the view moves, for depth. **Cut-outs** (the product standing on the shelf with its background removed) are a later upgrade, used only where background removal works cleanly, because it adds cost and can cut photos wrongly. Decided by the owner.

**Which product takes which slot.** The server assigns products to shelf slots by their position in the store's product order; the seller can reorder to choose what is front and centre. Products beyond the slots are reached through a "see all" list inside the room. **(my call)**

**Speed and smoothness (my call; targets to validate on the minimum phones).**
- First view of the market on a mid-range Android phone over 4G in about 1.5 seconds, by showing a small low-resolution base picture at once and refining it as tiles arrive.
- The journey holds a smooth frame rate (aim for 60, accept no worse than about 45 on the minimum phones). If a phone cannot, it quietly drops to simpler effects, and in the worst case to a short fade.
- The room for a store is fetched as the buyer approaches it (once they zoom past a level, or hover or press on a dot), so it is ready when the camera arrives. Nothing waits on a spinner mid-journey.
- Tiles and rooms are cached on the phone and use a CDN. A normal browsing session aims to stay within a few megabytes.
- Image formats chosen for size (WebP or AVIF); depth maps kept small and low resolution.

**Cart.** One cart across stores, as the server already has it. The cart control shows on the street and in every room, and an item flies into it when added.

**Accessibility and fallbacks (the plan's rules, restated for this feature).** Every card, dot and control has a label and a touch target of at least 48 dp. The list view gives the same stores and products with no motion. With the system's reduced-motion setting on, the journey is a short fade and the room does not move. Text scales to 200 percent. Offline: the app shows what it last had and says so; nothing queues a money action.

## 7. Sellers and their goods

A seller already uploads photos and creates products with variants, a price and a stock level through the server (part 7a). What this adds is how that looks:
- A published product appears in the room as a card, automatically, with no extra step.
- The in-app camera coaches the seller to frame a clean, well-lit shot on a plain surface. It improves how every card looks and is what makes later cut-outs work.
- Unpublishing or running out of stock removes or marks the card. The room never shows what cannot be bought.
- Opening a store (after an access code, as today) also means choosing its **store type**, which decides its room. A seller with two stores holds two plots.
- A seller can walk into their own store at any time, including before it is public, to see how their goods look and to reorder them. **(my call; the screens are planned with the selling piece.)**

## 8. What it needs from the server (new)

These are additions to the server, planned as their own piece (section 10):
- A **market layout**: the plots (id, district, position and size), which plot a store holds, and a store's type. A new store is given a free plot in its category's district. Who chooses the plot, and what happens when a district is full, is open (section 11).
- A **shelf layout** per store type (the slots) and a store-room call that returns the products placed in them, with the existing photo URLs.
- **Asset manifest and version** (extend the app-config route): where the tile sets and room assets live and which version the app should use, so art can be updated without an app release.
- Tiles and rooms served from storage behind a CDN, like the existing public photos. Nothing about a person is in them, and none of this logs or sends personal data.

## 9. The art

Deliverables, all produced offline and shipped as assets, never rendered on the phone:
- The market: one photoreal top-down render of the whole market, cut into a tile pyramid (several zoom levels), plus a plot map (where each plot is, in picture coordinates).
- Per store type: an interior picture (sharp at phone resolution), its depth map, the shelf-slot positions and sizes, and the lighting values.
- A short list of approved sizes and formats so the budgets in section 6 can be met.

Where it comes from is open (section 11): Unreal Engine (offered by the owner) is well suited to rendering the market and rooms offline with consistent light; an illustrator or generated art are the alternatives. Placeholder art built from code or from the concept stills is enough to build and test the experience before final art exists.

## 10. How the work is split (agreed order)

1. **App foundation** (what plan 7b was): the Flutter project and tooling, design system, API client and session, navigation, sign-up and recovery, address book. It can start now.
2. **The immersive market** (this design): tiles, the camera journey, depth rooms, cards, the cart, the list view.
3. **Server additions for it** (section 8): plots, store types, shelf slots, the room call, the asset manifest.
4. **Selling, buying and chat** (what plans 7c and 7d were): checkout, orders and tracking, store setup and product upload with the camera guide, disputes, chat, inbox and push.
5. **The art pipeline** (section 9). Runs in parallel from the moment piece 2 can use placeholders.
6. Then release readiness (plan 7e), and the partner-services seam (part 9) after.

Each piece gets its own plan, written when the one before it has landed, as the roadmap's convention says. This design replaces the "home, search, store and product pages" part of plan 7b: those become the immersive market and its list view. The overview (`2026-11-20-07-mobile-app.md`) and the roadmap need updating to match when the plans are written.

## 11. Open decisions for the owner

- **Where the art comes from** (section 9): Unreal Engine, an illustrator, or generated art. It decides cost and the final look, and is needed before launch, not before the build starts.
- **Who chooses a store's plot** and how the market grows: the owner assigns plots by hand, or the server places stores automatically in their district; and what happens when a district fills (a new district, a larger market).
- **How big the market is at launch**: how many plots and districts to draw and fill. It sets the size of the tile set and how much art is needed. Until it is decided, the builder can assume a market of about 60 plots in four or five districts, which the art can grow into. **(my call)**
- **The first store types** beyond the boutique, and whether sellers pick one or the owner assigns it.
- **When cut-outs are worth adding**, once real seller photos show how often the background removal works.

## 12. Risks

- **Depth maps look wrong at the edges** on some pictures. Mitigation: keep movement small, test on real pictures early, and fall back to a flat picture with card drift.
- **Sharpness versus size.** A market that is sharp at the deepest zoom is a lot of pixels. Mitigation: tiles, a CDN, careful formats, and capping the deepest zoom to what the phone's screen can show.
- **Cards that do not look like they belong** if seller photos are dark or cluttered. Mitigation: the camera guide, consistent card styling, and cut-outs later.
- **The journey becoming a gimmick** that slows buyers who know what they want. Mitigation: the list view, search, and skipping the flight, always one tap away.
- **No Flutter toolchain on this machine yet** and no way to test on an iPhone here (the app overview already says so). The first task of the foundation piece sets that up.
- **Photos can show faces or other people**, and are processed by the object store (a processor under the NDPA). This is already on the DPO list for part 7a and applies here too.

## 13. Left to the builder

- Which Flutter technique draws the journey and the depth rooms (custom painting and transforms, or fragment shaders), as long as it meets the budgets and has the fallbacks above.
- The tile scheme (sizes, number of zoom levels, formats) and how the base picture is refined.
- Exact camera paths, easing and timings, within the speed targets.
- The shelf-slot data format and the card styling values per store type.
- How the depth map is produced and how its edge glitches are hidden.
- Test strategy: the plan's four states for lists, golden tests for the key frames of the journey, and an integration flow against a real server. The journey itself needs checking on real low-end devices, not only emulators.
