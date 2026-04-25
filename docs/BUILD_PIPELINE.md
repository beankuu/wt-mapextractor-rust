# Build Pipeline — Detailed Steps

The Rust pipeline (`src/pipeline.rs`) runs a 7-step extraction flow per map. This document records each step's purpose, inputs, outputs, and known edge cases.

---

## Step 1: Extract DDSx Textures from `.bin`

**Function:** `extract_ddsx_from_bin(bin_path)`

**What it does:**
- Scans the `.bin` file for DDSx magic bytes (`DDSx`)
- Parses each 32-byte DDSx header (format, size, compression)
- Decompresses the body (Oodle, ZSTD, LZMA, ZLIB, or uncompressed)
- Reverses mip order if flagged (`flags & 0x40000`)
- Converts to standard DDS format and saves to a temporary directory

**Outputs:** `<tmp>/<map>_000.dds`, `<tmp>/<map>_001.dds`, ...

**Supported DDS formats:** DXT1 (BC1), DXT5 (BC3), A4R4G4B4

**Notes:**
- Entry `_000` is typically the overview colormap/normalmap
- Entries `_001` through `_NNN` are the per-cell splatting weight tiles
- Some maps have very few tiles (e.g., peleliu: 60, hangar_field: 16)

---

## Step 2: Extract DxP Material Textures

**Function:** `extract_dxp(dxp_path)`

**What it does:**
- Reads DxP2 texture packs (`.dxp.bin`, `hq_tex_*.dxp.bin`)
- Extracts named textures (landclass materials: rock, sand, grass, etc.)
- Also searches client `content/base/res/` DxP packs (602 packs with shared textures)

**Outputs:** Named DDS files in temporary directory (e.g., `african_savanna_dry_soil_tex_d.dds`)

**Notes:**
- Some maps have no DxP files; textures are resolved from context packs in Step 7
- Material textures are RGB diffuse maps used for terrain painting

---

## Step 3: Build Colormap / Normal Map

**Function:** `export_overview(map_name)`

**What it does:**
- Reads the first large (≥1024px) DDSx texture from the `.bin` file
- Detects DXT5nm normal maps (R≈0, B≈255, A varies) and converts them
- Saves as either `colormap.png` (RGB overview) or `normalmap.png` (converted DXT5nm)

**Outputs:** `<map-output>/colormap.png` OR `<map-output>/normalmap.png`

**Notes:**
- Many maps (especially air/avn/avg prefixed) have normal maps instead of color overview
- When only a normal map is available, the viewer applies it for 3D lighting but has no color overlay
- The terrain paint composite (Step 7) provides the main visual in these cases

---

## Step 4: Extract Heightmap

**Function:** `extract_heightmap(map_name)` → tries 3 methods in order

### 4a: HM2 Block (CompressedHeightmap)

**Function:** `_extract_hm2(data)`

**What it does:**
- Scans for `HM2\x00` magic at offsets > 0x10000
- Reads decompressed size from offset +4
- Tries Oodle decompression at header offsets 44–80 (compressed data start varies, typically offset 55)
- Validates via `std > 5` to reject garbage decompression
- **Applies Gaussian smoothing (sigma=3)** to fix spike artifacts from the CompressedHeightmap LOD structure
- Handles both square (e.g., 2048×2048) and non-square (e.g., 2048×2032) dimensions

**Outputs:**
- `<map-output>/heightmap.png` (8-bit grayscale, LR base with HM2 float-overlaid)
- `<map-output>/heightmap_float.npy` (float32, world meters — used for precise water masking)
- `<map-output>/heightmap_detail.png` (full HM2 resolution, 1024–2048px, for battle-area detail mesh)
- `<map-output>/normalmap_detail.png` (tangent-space normals from HM2 Sobel gradients)

**Decompression strategy (`_decompress_hm2_block`):**
1. **Single-stream** — one `OodleLZ_Decompress` call (works for most maps)
2. **Multi-stream paged** — binary-search pages per 256 KB stream, scan for next stream boundary
3. **Partial + padding** — pad tail rows when stream is incomplete (~5 maps)

**Float overlay (`_overlay_hm2_on_lr`):**
Replaces NaN-filled LR region with actual HM2 world heights. Uses 16-pixel outward feather blending so the visible LR mesh transitions smoothly to HM2 heights at the boundary. Inner region = direct HM2 replacement.

**Known issues:**
- HM2 raw data has interleaved LOD structure — Gaussian smoothing (sigma=3) applied to remove spikes
- Non-square dimensions handled by trying power-of-2 widths (4096→512)

### 4b: LandRayTracer Mesh

**Function:** `_extract_land_ray_tracer(data)`

**What it does:**
- Finds the lndm block and reads the rayTracerOfs
- Decompresses the LandRayTracer block (Oodle, ZSTD, or uncompressed)
- Parses the LTdump format: cell descriptors, vertex arrays (uint16[4]), face indices
- Rasterizes triangle mesh into a 2D heightmap grid
- Fills gaps with nearest-neighbor interpolation

**Outputs:** `<map-output>/heightmap.png` (16-bit heights → 8-bit grayscale)

**Notes:**
- Produces `height_min_m` and `height_max_m` in the manifest (real-world elevation)
- Most maps have this data; it's the most reliable heightmap source
- Dimension is `max(numCX, numCY) * (cellSize / gridCellSize)`, clamped to 512–4096

### 4c: Pseudo-Heightmap (Fallback)

**Function:** `_pseudo_heightmap()`

**What it does:**
- Uses the overview colormap (from Step 3) as height proxy
- Converts to grayscale and applies Gaussian blur (radius=5)
- Produces approximate elevation from image luminance

**Outputs:** `<map-output>/heightmap.png` with `pseudo: true` flag

**Notes:**
- Only works if a colormap exists (fails if only normal map available)
- Very approximate — coastlines show as height contours

---

## Step 5: Build Tiles & Materials

### Tile Export

**Function:** `export_tiles(map_name)` → `_export_single_tile()` (parallelized)

**What it does:**
- Reads numbered DDS files (`<map>_001.dds` ... `<map>_NNN.dds`)
- Converts each to PNG, saves as `<map-output>/tile_XXX.png`

### Tile Grid Assembly

**Function:** `assemble_tile_grid(map_name, tiles)`

**What it does:**
- Reads lndm header for grid dimensions (mapSizeX × mapSizeY)
- Assembles all primary tiles into an NxN grid image
- **Saves as RGB** (alpha stripped to prevent transparency issues in viewer fallback)

**Outputs:** `<map-output>/tile_grid.png`

### Material Export

**Function:** `export_materials(map_name)` → `_export_single_material()` (parallelized)

**What it does:**
- Converts named DDS material textures to PNGs
- These appear as thumbnails in the viewer sidebar

---

## Step 6: Extract Landclass Detail Data

**Function:** `extract_detail_data(map_name)`

**What it does:**
- Parses the lndm header from the `.bin` file
- Slices the detailData region (between `detailDataOfs` and `tileDataOfs`)
- Parses the Dagor BLK binary format using signature-based nameMap scanning
- Extracts per-landclass definitions: name, base texture, detail textures (R/G/B/K channels), tiling size

**Outputs:** Landclass array in `manifest.json`

**BLK Parser details:**
- Scans for signature patterns: `detail\x00texture\x00`, `detailmap\x00`, etc.
- Finds block boundaries via `\x01` separator bytes with property counts
- Reads block name, property names, value strings (texture paths), and binary floats (tiling sizes)

**Notes:**
- The parser handles multiple Dagor BLK format variations (3-byte and 4-byte separators)
- Tiling size is a `point2` float pair in range [16, 131072] world units per tile repeat
- Some landclasses have no size parameter; they use the default `_MAT_TILE_PX = 128`

---

## Step 7: Build Painted Terrain

**Function:** `build_painted_terrain(map_name)`

**What it does:**

### Phase 1: Parse tile cells
- Calls `_parse_tile_cells()` to walk the tile layout in the `.bin` file
- Each cell has a 15-byte prefix: 7 detTexIds (landclass indices for RGBA + tex2 RGB channels), totalLen, tex2Offset
- Cells with `totalLen=0` are empty (no splatting data)
- Minimum 2% of cells must be non-empty to accept

### Phase 2: Build weight maps
- Creates per-landclass weight arrays of size `(out_h, out_w)` = `gridW × cell_px` where cell_px adapts to target ~8192px output
- Loads tile PNGs and scatters RGBA channel weights into per-landclass weight arrays
- Secondary tex2 channels (for detTexIds[4..6]) loaded when available

### Phase 3: Gaussian blur
- Applies cross-cell Gaussian blur (sigma=6) to ALL weight maps
- This eliminates visible grid seams at cell boundaries
- Parallelized via ThreadPoolExecutor (scipy's gaussian_filter releases GIL)

### Phase 4: Composite
- For each active landclass, tiles its material texture across the output canvas
- Tile size computed from BLK `size` parameter: `px = out_px * (worldSize / mapWorldSize)`
- Blends each landclass by its blurred weight value
- Unpainted areas (ocean) filled with `[20, 60, 120]` (dark blue)

**GPU compute path:** If wgpu is available, compositing runs via a WGSL compute shader (weight map blending in parallel). Falls back to CPU NumPy path.

**Outputs:**
- `<map-output>/terrain_paint.png` (RGB, up to ~8192×8192)
- `<map-output>/terrain_paint_detail.png` (RGB, 8192px at HM2 resolution — separate for battle-area detail mesh)
- `<map-output>/heightmap_float.npy` (float32 heights for water masking)

---

## Manifest Generation

After all 7 steps, the build creates `<map-output>/manifest.json` containing:

```json
{
  "mapName": "iwo_jima",
  "mapCoord0": [-32768, -32768],
  "mapCoord1": [32768, 32768],
  "mapSize": [65536, 65536],
  "waterLevel": 0.0,
  "location": { "latitude": 0, "longitude": 0 },
  "colormap": { "file": "colormap.png", "width": 2048, "height": 2048 },
  "normalmap": null,
  "heightmap": { "file": "heightmap.png", "width": 2048, "height": 2048, "height_min_m": -10, "height_max_m": 200 },
  "tileGrid": { "file": "tile_grid.png", "cols": 16, "rows": 16 },
  "terrainPaint": { "file": "terrain_paint.png", "width": 8192, "height": 8192 },
  "heightmapDetail": { "file": "heightmap_detail.png", "world_x0": 0, "world_z0": 0, "world_x1": 4096, "world_z1": 4096, "height_min_m": 28.0, "height_max_m": 195.0 },
  "tankZone": { "coord0": [0, 0], "coord1": [4096, 4096], "gridSize": 128 },
  "averageGroundLevel": 360.0,
  "microDetails": { "textures": ["stone", "grass", ...], "uvScale": 0.9 },
  "sun": { "azimuth": 30.0, "elevation": 50.0, "strength": 0.5 },
  "materials": [...],
  "landclasses": [...]
}
```

---

## Batch Mode (`--all`)

**Implementation:** Rust batch worker path in `src/pipeline.rs`

- Scans client `levels/` for all `.bin` files (excluding `.dxp.bin`)
- Creates per-map directories under `maps/<name>/`
- Uses native Rust worker threads to process maps in parallel
- Each worker runs the full 7-step pipeline
- Worker stdout is suppressed; main process shows a refreshing progress bar with:
  - Visual progress bar with percentage and count
  - OK/fail tallies, ETA, and elapsed time
  - Latest completed map with worker name
  - Active worker assignments
- Uses a temporary directory for intermediate files, cleaned up automatically after each map
- Writes `maps/maps_index.json` with build results for the gallery page
