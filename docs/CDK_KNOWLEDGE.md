# CDK Knowledge Applied to Pipeline

Summary of Dagor CDK exploration findings and how they were applied to the WT-MapExtractor codebase.

**CDK Location:** `E:\WarThunderDev\WarThunderCDK`

---

## Applied Changes

### 1. Tank Battle Zone (`tankMapCoord0/1`)

- [x] **src/pipeline.rs**: Extract `tankMapCoord0`, `tankMapCoord1`, `tankGridSize` from blkx
- [x] **manifest.json**: New `tankZone` field with `coord0`, `coord1`, `gridSize`
- [x] **viewer.html**: Orange wireframe rectangle showing ground-battle area boundary
- [x] **viewer.html**: "Battle Zone" toggle checkbox in Layers panel

**CDK source:** `levels/flat_location_4_tests.blk` — `tankMapCoord0/1` is separate from air `mapCoord0/1`

**Example (avg_japan):** `tankMapCoord0: [0, 0]`, `tankMapCoord1: [4096, 4096]` within a 65536×65536 air map

---

### 2. Average Ground Level Fallback

- [x] **src/pipeline.rs**: Extract `average_ground_level` from blkx → `averageGroundLevel` in manifest
- [x] **viewer.html**: Use as water level fallback when `waterLevel` is 0 or missing

**CDK source:** Level .blk files contain `average_ground_level` as a reference elevation

**Example (avg_japan):** `average_ground_level: 360.0` — used instead of water_level 0.0 for ocean plane positioning

---

### 3. Micro-Detail Normal Maps

- [x] **src/pipeline.rs**: Extract `micro_details` block from blkx (texture array + UV scale)
- [x] **manifest.json**: New `microDetails` field with `textures[]` and `uvScale`
- [x] **viewer.html**: Show micro-detail count and UV scale in info panel

**CDK source:** `_assetsMicroDetails.blk` — 14 micro-detail normal map textures:
`stone`, `mud/gravel`, `sand`, `soil`, `grass`, `forest_floor`, `generic_granules`, `fabric`, `wood`, `metal`, `snow`, `concrete`

**Example (avg_mozdok):** 12 micro-detail entries, `land_micro_details_uv_scale: 0.9`

---

### 4. Custom Level Map References

- [x] **src/pipeline.rs**: Extract `customLevelMap` from blkx
- [x] **manifest.json**: New `customLevelMap` field (string or null)
- [x] **viewer.html**: Show tactical map reference in info panel

**CDK source:** `levels/location_sample.blk` — `customLevelMap:t="levels\location_sample_map.jpg"`

**Coverage:** 39 of ~166 maps have `customLevelMap` references (mostly legacy maps like avg_mozdok, avg_berlin, avg_guadalcanal). Newer maps generate tactical maps procedurally.

---

### 5. Per-Channel Detail Sizes (`detailSizes`)

- [x] **src/landclass.rs**: Scan binary data for per-channel detailSize floats (4–128m range)
- [x] **manifest.json**: New `detailSizes` array per landclass (up to 4 values for R/G/B/K channels)
- [x] **viewer.html**: Show detail sizes in landclass panel entries

**CDK source:** Landclass `.land.blk` files have `detailSizeRed/Green/Blue/Black` properties controlling per-channel micro-detail texture tiling in world meters.

**Example (avg_japan → japan_sakura_fields_a):** `detailSizes: [16.3, 15.5, 25.7, 19.2]` — each channel tiles at different scales

---

## CDK Findings (Reference Only)

These findings inform understanding but don't require code changes yet.

### Binary Chunk Tags (Validated)

CDK `application.blk` confirms `levelExpSHA1Tags`:
| Tag | Description |
|-----|-------------|
| `RqRL` | Required resources |
| `lmap` | Land mesh / lndm data |
| `HM2` | CompressedHeightmap |
| `hspl` | Height spline data |
| `spgZ` | Spline geometry |
| `stbl` | String table |
| `SCN` | Scene objects |
| `RIGz` | Render instances geometry |
| `FRT` | Face render tree |

Our parser correctly scans for all these tags.

### Clipmap Configuration

CDK `application.blk` clipmap settings:
- `sideSize: 4096` — virtual texture atlas side length
- `texelSize: 0.01` — 1cm per texel at highest zoom
- `stackCount: 8` — 8 mipmap LOD levels

This validates our 4096px detail paint resolution as appropriate.

### Landclass `.land.blk` Format

From `develop/assets/landclasses/`:
- `size:p2=W,H` — tile world size (meters). Validated ours: 256–4096 range
- `detail{}` block with `texture`, `splattingmap`, channel textures + sizes
- `colorMapSize/splattingMapSize` — texture resolution
- `obj_plant_generate{}` — vegetation placement with density maps

### Height Generation Layers (`genLayers`)

From `heightmapLand.plugin.blk`:
- Ordered landclass stacking with blend rules:
  - `ht_conv` / `ht_v0` / `ht_dv` — height-based blending
  - `ang_conv` / `ang_v0` / `ang_dv` — slope-angle blending  
  - `mask_conv` / `mask` — mask-based blending
  - `writeImportance` — layer priority
- `colorGenParams{}` — procedural LC assignment by height/angle/curvature

### Vertex Texturing (Not Implemented)

CDK shows `useVertTex:b=yes` with `vertTexAng0/vertTexAng1` for cliff rendering at steep angles. Not currently implemented in our viewer.

---

## Potential Future Improvements

| Feature | CDK Source | Complexity |
|---------|-----------|:----------:|
| Use `detailSizes` for tiling frequency | `detailSize[Color]` in `.land.blk` | Medium |
| Vegetation placement from `obj_plant_generate` | Landclass `.land.blk` | High |
| Cliff texturing at steep angles | `vertTexAng0/1` in heightmapLand | Medium |
| Procedural LC from height/angle rules | `genLayers` + `colorGenParams` | High |
| Tactical map overlay from `customLevelMap` | Level `.blk` references | Low |
| Micro-detail normal map compositing | `micro_details` + `_assetsMicroDetails.blk` | Medium |
| `randomGrass` layer visualization | Level `.blk` grass config | High |
