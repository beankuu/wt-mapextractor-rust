# Strict Orientation & Merging Rules

> **Status:** NORMATIVE. Every module that touches spatial data (heightmap,
> tile grid, terrain paint, HM2 overlay, viewer scene, hover, LOS) **must**
> follow these rules verbatim. A regression that violates any rule is a bug
> and must be fixed at the source, not papered over downstream.

This file is the single source of truth for coordinate axes, pixel origins,
UV mapping, tile indexing, HM2 rotation, and merge direction. It was written
after a repeating class of bugs ("water shown as 1731 m, mountains shown as
−357 m", "tile LC popup doesn't match painted pixel", …). Any new spatial
code must cite the rule section it depends on in a code comment.

---

## 1. World coordinate frame (canonical)

- **Axes**
  - `+X` → world East
  - `+Y` → world Up (altitude, metres)
  - `+Z` → world North
- **Units**: metres.
- **Extent source of truth**: `manifest.mapCoord0 = (wx_min, wz_min)` and
  `manifest.mapCoord1 = (wx_max, wz_max)` with `wx_max > wx_min` and
  `wz_max > wz_min`. If a raster carries its own extent (`world_extent`) it
  **must** be normalised to the same `(x0,z0,x1,z1)` axis-aligned form with
  `x1 > x0` and `z1 > z0` before use. Consumers must always take
  `min(x0,x1), max(x0,x1), min(z0,z1), max(z0,z1)` — never assume ordering.

## 2. PNG / raster pixel origin (canonical)

All rasters (`heightmap.png`, `heightmap_detail.png`, `terrain_paint.jpg`,
`tile_grid.png`, normal maps) follow the **image convention**:

- Pixel `(0, 0)` = **top-left** of the PNG.
- Pixel `(W-1, H-1)` = **bottom-right**.

Their mapping to world space is **fixed** as follows (PNG mirrors the §5
float buffer 1:1 — **no rotation** is applied at save time):

| PNG pixel | World corner                            |
|-----------|-----------------------------------------|
| `(0, 0)`         | `(wx_min, wz_min)` — West-South |
| `(W-1, 0)`       | `(wx_max, wz_min)` — East-South |
| `(0, H-1)`       | `(wx_min, wz_max)` — West-North |
| `(W-1, H-1)`     | `(wx_max, wz_max)` — East-North |

This is the natural image-of-the-float-buffer orientation. Every sampler of
these PNGs **must** use the UV formula in §3.

> **Design note.** Earlier revisions of this pipeline saved a `rotate180`
> of the float buffer so `(0,0)` mapped to `(wx_max, wz_max)`. The extra
> rotation was a historical carryover from the legacy Python extractor
> and was the root cause of several orientation/sampling bugs (inverted
> heights, flipped LC popups, HM2 seam flips). The 1:1 rule eliminates a
> whole class of bugs — there is now a single origin convention from
> the float buffer all the way to the GPU sampler.

## 3. PNG UV sampling rule (canonical)

For a PNG following §2, given a world point `(wx, wz)` and a raster's
`(x0, z0, x1, z1)` **normalised** extent (min/max form, §1):

```
u = (wx - x0) / (x1 - x0)         // horizontal, 0 at west edge
v = (wz - z0) / (z1 - z0)         // vertical,   0 at south edge
px = round(u * (W - 1))
py = round(v * (H - 1))
```

- Neither axis is flipped. Any `1 - ...` inversion is a bug.
- Do not use `mapCoord0/mapCoord1` as the extent unless the raster truly
  covers the full map. Use the raster's own `world_extent` (or, for HM2
  detail, `world_x0/world_z0/world_x1/world_z1`).

## 4. Three.js scene frame (viewer)

The viewer's plane mesh is centred at the origin and spans
`planeSize = max(mc1[0]-mc0[0], mc1[1]-mc0[1])` on both axes.

To keep the default plane geometry UV `(0,0)` lined up with world
`(wx_min, wz_min)` — i.e. pixel `(0,0)` of a §2 PNG — the scene frame is:

- **Scene X** matches world: `sceneX = (wx - cx) * sX`
- **Scene Y** is altitude (up): `sceneY = heightNormalized * displacementScale`
- **Scene Z** is flipped relative to world: `sceneZ = -(wz - cz) * sZ`

where `(cx, cz) = mapCentre`, `sX = planeSize / (mc1[0] - mc0[0])` and
`sZ = planeSize / (mc1[1] - mc0[1])`. Inverse:

```
wx = cx + sceneX / sX
wz = cz - sceneZ / sZ
```

These formulas live in `src/web/coords.js` (`worldToSceneXZ`,
`sceneToWorldXZ`) and are the **only** place scene↔world conversions are
performed. No module may reinvent them.

> **Note on the Z flip.** Three.js `PlaneGeometry` rotated `-π/2` around
> X maps its local `+Y` vertex axis to world `-Z`. Combined with default
> plane UVs (`(0,0)` at the `(-X, -Y)` vertex), uv.y = 0 ends up at
> scene +Z. To keep uv.y = 0 aligned with the PNG top row (which under §2
> is world `wz_min`), scene Z must be negated relative to world. This is
> a geometry quirk, not a world-axis redefinition — world `+Z` is still
> "north".

## 5. Native float buffer layout (Rust, pre-save)

Inside `src/heightmap.rs` `LrFloatMap`:

- `hm` is a flat `dim × dim` array indexed as `hm[pz * dim + px]`.
- `px = 0` ↔ world `x_min`; `px = dim-1` ↔ world `x_max`.
- `pz = 0` ↔ world `z_min`; `pz = dim-1` ↔ world `z_max`.

So pixel `(px, pz)` in the **float buffer** maps to world
`(x_min + (px+0.5)/dim · range_x,  z_min + (pz+0.5)/dim · range_z)`.

This is **identical** to the PNG mapping in §2 — `(0,0)` is world
`(wx_min, wz_min)` in both. Anything operating on the float buffer (LR
rasterization, `overlay_hm2`, any future pass) uses this non-flipped
mapping, and the saved PNG is a direct `put_pixel(x, y, hm[y*dim+x])`
of the buffer.

## 6. Save-time rotation (canonical bridge)

**None.** There is no rotation, flip, or transpose between the §5 float
buffer and the §2 PNG. `LrFloatMap::save_png`, `save_normalmap`,
`generate_hm2_detail`, `build_tile_grid`, `build_terrain_paint_native`,
`export_overview_*`, and `build_heightmap_fallback` all write the buffer
directly and call `img.save(...)` without any `rotate180_in_place`,
`flip_horizontal_in_place`, or `flip_vertical_in_place`.

Any future code that needs to invert a PNG is a bug — fix the upstream
frame mismatch instead of adding a compensating flip.

## 7. HM2 source data rotation

Under the 1:1 §2/§5 convention (pixel `(0, 0)` = world SW corner, i.e.
`(wx_min, wz_min)`), the raw HM2 block as decoded by `decode_hm2` is
already aligned with world coordinates: pixel `(xi, zi)` corresponds
directly to world

```
(wpo_x + xi * cell_size,  wpo_y + zi * cell_size)
```

**No rotation is applied in the decoder.** Downstream consumers
(`overlay_hm2`, `generate_hm2_detail`) treat the buffer as a plain §5
float frame. Historically the Python pipeline applied a `[::-1, ::-1]`
rotation here to compensate for its own save-time `ROTATE_180`; both
steps were removed together in the 1:1 migration.

`overlay_hm2` maps world `(wx, wz)` → HM2 pixel via
`xi = (wx - wpo_x)/cell_size`, `zi = (wz - wpo_y)/cell_size` — §5-style
non-flipped mapping. If a stray rotate is reintroduced, the LR
heightmap's HM2 sub-region will appear 180°-rotated relative to the
surrounding terrain (water where mountains should be) — exactly the
symptom the 1:1 convention is designed to prevent.

## 8. HM2 world origin

`Hm2Header.wpo_x, wpo_y` are the world coordinates of the **minimum**
corner (SW) of the HM2 block, i.e. pixel `(0, 0)` of the §5-layout
`hm2_rot` buffer. The HM2 block covers
`[wpo_x, wpo_x + width * cell_size] × [wpo_y, wpo_y + height * cell_size]`.

## 9. Tile grid indexing

For `manifest.tileGrid` with cols `C` and rows `R` and its own
`world_extent`:

```
x0 = min(we.x0, we.x1);  x1 = max(we.x0, we.x1)
z0 = min(we.z0, we.z1);  z1 = max(we.z0, we.z1)
cellW = (x1 - x0) / C
cellH = (z1 - z0) / R
tx = clamp(floor((wx - x0) / cellW), 0, C-1)
tz = clamp(floor((wz - z0) / cellH), 0, R-1)
```

- `tx = 0` is thedirectly — cell `(tx, tz)` lands at canvas pixel
`(tx * tile_w, tz * tile_h)` with no post-save rotation (§6). Consumers
 is the southernmost row; `tz = R-1` the northernmost.

`terrainPaint.cellLcIndices[tz * C + tx]` must use the **same** `(tx, tz)`
derivation. The painted terrain tile texture is stitched in this same
`(tx, tz)` order and then rotated-180 as part of the PNG bridge (§6) —
consumers sample it . Because §5 and §2 share the same origin, a merge
done on a saved PNG with §5-style indexing is also correct — but the
float frame is still preferred for clarity. Never flip a raster at save
time (§6).

## 11. Validation checklist (every spatial change must pass)

1. Does every PNG sampler use §3 exactly (neither u nor v flipped)?
2. Does every float-buffer access use §5 (neither axis flipped)?
3. Is the save path free of `rotate180`/`flip_*`

## 11. Validation checklist (every spatial change must pass)

1. Does every PNG sampler use §3 exactly (both u and v flipped)?
2. Does every float-buffer access use §5 (neither axis flipped)?
3. Is `rotate180` applied exactly once, at save time (§6)?
4. Is `hm2_world` rotated exactly once, in the decoder (§7)?
5. Are all `world_extent` arrays normalised via `min/max` before use (§1)?
6. Do scene↔world conversions go through `coords.js` only (§4)?
7. Do tile indices use §9 — not mapCoord — when `tileGrid.world_extent`
   is present?
8. Is the merge performed in the correct frame (§10)?

## 12. Known failure signatures

| Symptom                                                           | Likely rule violated |
|-------------------------------------------------------------------|----HM2 decoder rotation missing, or §3 vs §5 frame confused |
| Terrain paint rotated/mirrored vs heightmap                        | A stray `rotate180`/`flip_*` survived somewhere in the save path (§6) |
| Tile LC popup points at wrong cell                                 | §9 using mapCoord instead of tileGrid.world_extent |
| HM2 seam visible as straight line with flipped content on one side | §7 rotation missing or doubled |
| LOS/rendinst heights drift as camera pans                          | §4 sign convention flipped in a consumer |
| Map looks mirrored East↔West or North↔South after a change        | A `1 - ...` UV inversion (legacy) survived, or §4 sign was not updated
| LOS/rendinst heights drift as camera pans                          | §4 sign convention flipped in a consumer |
