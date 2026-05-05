# Known Issues & Edge Cases

Tracking document for map extraction problems, workarounds, and format quirks.

---

## Heightmap Issues

### HM2 CompressedHeightmap Spiking (FIXED)

**Affected maps:** avn_coral_islands, avg_karantan, firing_range, and other HM2-only maps

**Problem:** The HM2 block stores a CompressedHeightmap with an interleaved LOD (Level of Detail) structure. When the Oodle-decompressed bytes are naively interpreted as raw uint8 pixel values, the interleaved LOD data produces severe high-frequency spikes in the terrain.

**Fix:** Apply Gaussian smoothing (sigma=3) via `scipy.ndimage.gaussian_filter` after decompressing. This removes the spike artifacts while preserving the overall terrain shape.

**Trade-off:** Fine terrain detail is slightly blurred. A proper decoder for Dagor's CompressedHeightmap format would produce better results but requires reverse-engineering the quad-tree LOD structure.

### HM2 Header Offset Varies

**Problem:** The Oodle compressed data within the HM2 block doesn't always start at a fixed offset. It typically starts at offset 55 from the `HM2\x00` marker, but can vary.

**Fix:** Scan range 44–80 (was previously 48–64). Accept the first offset that produces valid Oodle decompression with `std > 5`.

### HM2 Non-Square Dimensions

**Affected maps:** hangar_field (decomp_size=4161536 = 2048×2032)

**Problem:** The `_extract_hm2()` function originally only handled square heightmaps (`dim = sqrt(decomp_size)`). Non-square maps would silently fail.

**Fix:** Try power-of-2 widths (4096, 2048, 1024, 512) and accept the first that evenly divides decomp_size with a reasonable aspect ratio.

### HM2 / LandRayTracer Boundary Gap (FIXED)

**Affected maps:** All avg_* maps with HM2 detail heightmaps (avg_poland, avg_karelia_forest_a, etc.)

**Problem:** The LandRayTracer mesh has no triangles in the HM2 region (engine uses excludeBounding). When rasterized, those pixels are NaN, then filled by `_pyramid_fill_nan()` which interpolates from surrounding terrain. The interpolated heights don't match the actual HM2 heights, causing a visible gap or "sinkhole" at the boundary. For avg_poland: LR interpolated ~100m in the HM2 region, but HM2 actual max was 43.39m.

**Fix:** `_overlay_hm2_on_lr()` replaces the NaN-filled LR region with actual HM2 world heights (decoded from CompressedHeightmap, resampled to LR pixel resolution via `scipy.ndimage.zoom`). Uses 16-pixel feather blending at edges for smooth transition. Operates on float64 data — no precision loss.

### Water Masking Angular Boundaries (FIXED)

**Affected maps:** air_skyscraper_city, air_kamchatka, air_israel, and other maps with large height ranges

**Problem:** Water masking compared 8-bit heightmap pixel values against the water level. For maps with large height ranges (e.g., air_skyscraper_city: 5210m range), each u8 step represents ~20m of elevation change. This created blocky, angular water boundaries.

**Fix:** `build_painted_terrain()` now loads `heightmap_float.npy` (float32, saved during extract_heightmap) and compares `hm_world <= water_level` in world meters. Falls back to 8-bit PNG if float data unavailable.

### Authored Underwater Splat Layers vs Water Plane

**Affected maps:** Ground maps with interior lakes or underwater sand/shore landclasses.

**Problem:** Height-only water masking can miss shallow authored lake edges whose
heightmap samples sit just above `waterLevel`. A coarse follow-up attempt marked
entire cells containing an underwater landclass as water, which over-painted whole
tiles. Sampling authored underwater splat pixels directly also proved too broad:
these materials can represent shoreline or submerged sand layers that extend up
slopes and above the actual water plane.

**Current policy:** Water is painted by the height-derived mask, with per-cell
protection to avoid flooding authored land. Authored underwater landclass names
are not treated as water placement by themselves; height remains the source of
truth for avoiding mountain/shoreline false positives.

### Maps Without Any Heightmap Source

**Affected maps:** Some maps have no HM2 block, no LandRayTracer data, and no colormap for pseudo-heightmap (only a DXT5nm normal map).

**Behavior:** Falls through to `_pseudo_heightmap()`. If no colormap exists (only normalmap), pseudo-heightmap returns None and the map has no height displacement.

---

## Terrain Visibility Issues

### Terrain Not Visible in Viewer (FIXED)

**Affected maps:** peleliu, firing_range, hangar_field, arcade_tabletop_mountain, avn_coral_islands, avg_karantan, air_equatorial_island

**Root causes (all fixed):**

1. **`_parse_tile_cells` threshold too strict:** The tile cell parser required ≥25% of cells to have valid DDSx data. Many maps have sparse tile coverage (e.g., peleliu: 59/256, hangar_field: 15/256). Fixed by lowering threshold to 2%.

2. **Tile grid transparency:** When `build_painted_terrain()` failed, the viewer fell back to `tile_grid.png`. Raw tiles are RGBA images; when assembled into the grid and used with `transparent: true`, areas with low/zero alpha became invisible. Fixed by: (a) saving `tile_grid.png` as RGB (alpha stripped), and (b) setting `transparent: false` in the viewer when using tileGrid fallback.

3. **Overview is DXT5nm normal map, not colormap:** Many maps (especially air_*, avn_*, avg_*, arcade_*) have a normal map as the overview texture instead of a color map. The viewer's colormap overlay mesh is not created for these maps. Terrain paint is the only visible texture layer.

---

## Texture Quality

### Low Resolution When Zooming

**Problem:** Terrain paint resolution was limited by fixed `cell_px = 256`. For a 16×16 grid, this produced 4096×4096 textures.

**Fix:** Increased to adaptive sizing: `cell_px = max(128, min(512, 8192 // max(grid_w, grid_h)))`. This targets ~8192px output:
- 16×16 grids → 512px/cell = 8192×8192
- 32×32 grids → 256px/cell = 8192×8192

---

## Binary Format Notes

### lndm Header Fields

| Field | Type | Description |
|-------|------|-------------|
| `gridCellSize` | float | Cell size in world units (typically 4–32) |
| `landCellSize` | float | Land cell size (256–4096) |
| `mapSizeX/Y` | int32 | Grid dimensions (16 or 32) |
| `originCellX/Y` | int32 | Grid origin offset |
| `useTile` | int32 | Whether tiled textures are used |
| `meshMapOfs` | int32 | Offset to mesh map data (relative to baseOfs) |
| `detailDataOfs` | int32 | Offset to detail data BLK region |
| `tileDataOfs` | int32 | Offset to tile DDSx data |
| `rayTracerOfs` | int32 | Offset to LandRayTracer block |

### Tile Cell Layout (15-byte prefix)

Each cell in the tile data has:
```
Bytes 0–6:   detTexIds[7] (uint8) — landclass indices for RGBA channels + tex2 RGB
Bytes 7–10:  totalLen (uint32) — total DDSx data length for this cell
Bytes 11–14: tex2Offset (uint32) — offset to secondary texture within totalLen
```

### DDSx Compression Byte (offset 0x0B, bits 7-5)

| Value | Compression |
|-------|-------------|
| 0x00 | None |
| 0x20 | ZSTD |
| 0x40 | LZMA |
| 0x60 | Oodle |
| 0x80 | ZLIB |

### DxP2 Pack Structure

```
Header:
  0x00: "DxP2" magic
  0x08: texture_count (uint32)
  0x10: section_table_base (3 sections × 16 bytes each)

Section 0: name_directory — string offsets for texture names
Section 1: ddsx_headers — 32 bytes per texture (DDSx format)
Section 2: body_directory — offsets and sizes for texture body data
```

---

## Map Categories

Maps follow naming conventions that indicate their type:

| Prefix | Type | Typical Grid | Notes |
|--------|------|:------------:|-------|
| (none) | Historical battles | 16×16 | Usually have colormap + LandRayTracer |
| `air_` | Air battles | 16×16 or 32×32 | Usually DXT5nm normalmap only, large world size |
| `avg_` | Ground battles | 32×32 | Often have HM2 heightmaps, smaller landCellSize |
| `avn_` | Naval battles | 16×16 | DXT5nm normalmap, may have HM2 |
| `arcade_` | Arcade modes | 16×16 | Variable, some have very few tiles |
| `sector_` | Sectors | 16×16 | Sub-maps of larger battlefields |

---

## Biome System (Partial Support)

### Biome Land Classes

Some maps use a biome-type landclass (e.g., `karelia_forest_detailed_biome`) that works differently from regular LCs. The biome block in the nameMap contains:

| Resource | Size | Purpose | Status |
|----------|------|---------|--------|
| `*_tex_d` | 2048×2048 RGBA | Macro diffuse color map | **Supported** — used as biome LC material tile |
| `*_tex_b` | 4096×4096 L (grayscale) | Sub-landclass index map | **Exported** but not composited |
| `*_tex_f` | varies | Flowmap (water flow effects) | **Exported** but not used |
| `biome_detail_*_tex_d` | 16 textures | Tiling sub-LC diffuse textures | **Not extracted** — available in `hq_tex_landscape_extra.dxp.bin` |

**How the engine renders biomes:**
1. For each pixel in the biome area, look up the sub-LC index from `_tex_b`
2. The index maps to one of ~16 sub-LC types (soil_a, grass_a, rock_cliff_a, etc.)
3. Tile the corresponding `biome_detail_*_tex_d` texture at the detail scale
4. Blend with the macro `_tex_d` for overall color

**Current behavior:** The biome area is painted with just the macro `_tex_d` texture. This produces correct color but lacks the micro-detail variation visible in-game.

**Biome property names (42 props):** `detail`, `texture`, `shader`, `size`, `landClassTextures`, `indices`, `grass_weight`, `flowmap`, `landClassParams`, `detailMul1/2`, `randomRotatePeriod`, `randomVariationPeriod`, `details` (sub-block with 16 sub-LC entries each having `albedo` and `reflectance`), `scheme`, `second`, `random_flowmap`, `height_scale`, `puddle_prob`, `invert_height`, `grass_microdetail_removal_bits`, `normal_scale`, `detail_weights_mul`, `editorId`.

### Parser prop_count Limit (FIXED)

**Affected maps:** avg_karelia_forest_a (biome has 42 properties)

**Problem:** `_parse_detail_blk` separator detection used `prop_count <= 40`, silently skipping biome blocks with more properties. This shifted all LC indices, causing out-of-range tile references.

**Fix:** Raised limit to 100.

---

## Viewer Notes

### Water Level Positioning

The viewer positions the ocean plane using `waterFraction`:
```javascript
waterFraction = (waterLevel - height_min_m) / (height_max_m - height_min_m)
waterMesh.position.y = displacementScale * waterFraction
```

When `height_min_m` / `height_max_m` are not available (HM2 heightmaps), `waterFraction` defaults to 0.5. This places the water at 50% of the terrain height range, which may not match the actual water level.

### Heightmap Displacement

The viewer uses Three.js `displacementMap` with a single-channel 8-bit heightmap PNG. The displacement maps 0→255 to 0→`displacementScale` (which is `heightPct / 100 * maxHeightScale` where `maxHeightScale` is the real-world height range in meters). The default height slider is at 100% (real-world scale), adjustable up to 400%.

### Maps Without Colormap

For maps with only a DXT5nm normal map (no colormap), the viewer:
- Does NOT create the `terrainMesh` (colormap overlay)
- Creates `tileGridMesh` using terrain paint or tile grid as the main visible texture
- Applies the normal map for 3D lighting on the terrain paint mesh

---

## Texture Pack Issues

### HQ DxP Stub Entries Shadowing Valid Textures (FIXED)

**Affected maps:** avg_kursk_villages and others using `hq_tex_*.dxp.bin` packs

**Problem:** HQ DxP packs may contain stub entries (zero format/size) for textures that exist in the base pack. When `_extract_context_texture` found the stub first, it returned empty data, leaving LCs with no material texture even though the base pack had a valid texture.

**Fix:** `_extract_context_texture` now falls back to the corresponding base pack (e.g., `kursk.dxp.bin`) when HQ extraction produces an empty/zero-size result.

---

## Tile Layout Issues

### Tile Column Reversal (~15 Maps) (AUTO-DETECTED)

**Affected maps:** ~15 maps with reversed tile column order (detected at build time)

**Problem:** Some maps store tile cells in reversed column order (right-to-left per row). Naively assembling them in scan order produces a terrain paint that is mirrored horizontally versus the heightmap.

**Fix:** `_detect_tile_col_reversal()` compares tile fill pattern (which cells have data) against the heightmap water mask. If the mirrored layout has higher correlation, the column order is reversed. Applied in both `assemble_tile_grid()` and `build_painted_terrain()`. BFS graph traversal uses walk order (no reversal) since adjacency is topologically equivalent regardless of column order.

---

## Pseudo-Heightmap Issues

### Normalmap Ocean Detection for Air Maps (AUTO-DETECTED)

**Affected maps:** air_* maps with empty LandRayTracer (0 triangles) and no HM2

**Problem:** Pseudo-heightmap for air maps uses normalmap luminance. Without water masking the result includes flat ocean areas at non-zero height, misrepresenting elevation.

**Fix:** `_pseudo_heightmap()` detects ocean from normalmap flatness (nz > 0.96) using erosion → label → area-threshold → dilation at reduced resolution (max 2048px). Large flat regions are masked as water `[20, 60, 120]`.
