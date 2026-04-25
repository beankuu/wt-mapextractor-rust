# WT-MapExtractor

War Thunder map data extractor and interactive 3D terrain viewer, written
in Rust.

![Main view](1.MapViewer.webp)

![Example view, avg_tunisia_desert](2.Example.webp)

## Features

- Native Rust extraction for DDSx, DxP, HM2, LandRayTracer, RIGz, BLK
  landclasses
- Native terrain-paint compositing (per-tile material-weight blend) with
  parallel Rayon workers
- Three.js 3D viewer with GPU vertex displacement, optional WebGPU
  line-of-sight, normal mapping, mission overlays, and Earth mini-globe
- Batch mode (`--all`) with per-map progress, ETA, and a thumbnail
  gallery (`src/index.html`)

## Pre-requirements

Set up `config.json` (see `config.sample.json`):

1. **Datamine path** — local clone of
   <https://github.com/gszabi99/War-Thunder-Datamine>
2. **Client path** — local War Thunder installation
3. **`oo2core_9_win64.dll`** — copy into `src/` (or set `OODLE_DLL` env
   var). The DLL is not shipped by War Thunder; obtain it from another
   game such as Warframe.

## Test Environment

- AMD Ryzen 7 5800X3D · 64 GB RAM · 9060XT 8 GB · Windows 11
- 162 maps total in current War Thunder version

## Performance vs. legacy Python implementation

| Metric | Python | Rust |
|--------|-------:|-----:|
| CPU usage | 50–99 % | 20–45 % |
| Peak RAM | 35+ GB | 12+ GB |
| Per map | 150–200 s | 15–30 s |
| All maps | ~4000 s | ~400 s |
| Output total | 25+ GB | 9+ GB |

## Quick start

```powershell
# build + serve all maps
cargo run --release -- --all

# build a single map
cargo run --release -- iwo_jima

# build several maps
cargo run --release -- iwo_jima guam

# open viewer without building
cargo run --release --

# tune worker count (default = min(CPU, 4); raise on big-RAM machines)
$env:WT_WORKERS="16"; cargo run --release -- --all
```

### Useful flags

```powershell
# write to ./viewer_data/ instead of maps/<name>/
cargo run --release -- iwo_jima --local

# sun direction & strength used by the colormap shading
cargo run --release -- iwo_jima --sun-azimuth 30 --sun-elevation 30 --sun-strength 0.7

# export per-material textures to mat/
cargo run --release -- iwo_jima --mat

# generate landclass thumbnails
cargo run --release -- iwo_jima --thumbs

# skip the local HTTP server (extract only)
cargo run --release -- iwo_jima --no-serve

# fast mode: skip tile-grid and rendinst stages
cargo run --release -- iwo_jima --fast

# dump every known structure in a .bin plus hex of unknown regions
cargo run --release -- --inspect --no-serve air_israel avg_japan
```

See `cargo run --release -- --help` for the full flag list.

## Required level files

Resolved automatically from the datamine path configured in
`config.json`. The minimum needed per map is `<map>.bin`; richer
metadata uses the optional files below.

| File | Required | Description |
|------|:--------:|-------------|
| `<map>.bin` | yes | DBLD container — DDSx textures, heightmap (HM2/LRT), landclass detail data |
| `<map>.blkx` | recommended | JSON metadata — coordinates, water level, grid dimensions |
| `<map>.dxp.bin` | optional | DxP2 texture pack — terrain material overlays |
| `hq_tex_<map>.dxp.bin` | optional | High-quality DxP2 texture pack |

Also required at runtime: `src/oo2core_9_win64.dll` (or `OODLE_DLL`).

## Pipeline

| Step | Description |
|------|-------------|
| 1 | Extract DDSx textures from `.bin` |
| 2 | Extract DxP material textures from `.dxp.bin` |
| 3 | Convert overview DDSx → colormap or normal map |
| 4 | Extract heightmap (HM2 → LandRayTracer mesh → pseudo fallback) |
| 5 | Export per-cell splatting tiles + material PNGs |
| 6 | Parse landclass detail BLK (per-landclass tiling, detail textures) |
| 7 | Composite painted terrain: weight maps × tiled materials |
| 8 | Optionally extract render-instances (`RIGz` → `rendinst.bin`) |

### Heightmap source priority

1. **HM2 block** — CompressedHeightmap (CBLOCK v2). High-resolution uint16.
2. **LandRayTracer mesh** — triangle mesh rasterised into a heightmap.
3. **Pseudo-heightmap** — Gaussian-blurred overview luminance fallback.

### Terrain painting

1. **Splatting tiles** (DXT1 128×128 per cell) encode RGBA weights mapped
   to landclass indices via `detTexIds`.
2. **Landclass definitions** parsed from Dagor BLK binary — base
   texture, detail textures (R/G/B/K), tiling sizes.
3. **Native weight blend** in Rust produces `terrain_paint.png`.
4. **Parallel tile processing** uses Rayon to scale across cores.

### Batch mode (`--all`)

```
  ========== WT-MapExtractor: Batch Build All Maps ==========

  120 maps | 8 workers

  [ 15%] [18/120] avg_berlin    - 28.5s - OK   ETA: 364s
  [ 14%] [17/120] avg_normandy  - 24.3s - OK
  [ 13%] [16/120] avg_poland    - 31.2s - OK

  ===========================================================
  Build complete in 120.5s
  120 succeeded | 0 failed | 120 total
  ===========================================================
```

Failed maps display the full anyhow context for troubleshooting.

## 3D viewer

Open `src/viewer.html` (or omit `--no-serve`). Highlights:

- GPU vertex displacement + CPU raycast mesh for accurate hover read-out
- Height-scale slider (0–400 %)
- Real-time world-space coordinates + elevation in metres
- Layer toggles (texture, heightmap, ocean, battle zone, tile grid)
- DXT5nm normal maps auto-detected
- Earth mini-globe with the map location marked (Robinson projection,
  shipped locally as `src/World_Map.svg`, sourced from
  <https://commons.wikimedia.org/wiki/File:BlankMap-World.svg>)
- Mission overlay — spawn points, capture zones, battle areas
- Optional WebGPU line-of-sight (720 rays × 400 steps) with CPU fallback
- HM2 detail mesh on top of the base heightmap
- Batch gallery (`src/index.html`) for `--all` builds

## Standalone in-game-map tool

A separate gitignored crate at `ingame_map/` renders a 2D tactical-map
PNG (grid + scale bar) from an already-extracted `maps/<name>/`
directory. It is intentionally outside the mainstream pipeline.

```powershell
cargo run --manifest-path ingame_map/Cargo.toml --release -- avg_vietnam_hills
```

See `ingame_map/README.md` for full options.

## Project structure

```
src/
  main.rs             - CLI entrypoint
  cli.rs              - clap argument parser
  config.rs           - config.json loader
  pipeline.rs         - end-to-end extraction/export pipeline
  extract.rs          - DDSx + DxP extraction & decompression
  dxp_index.rs        - DxP material index
  export.rs           - overview / tile / material export
  heightmap.rs        - HM2 + LandRayTracer native heightmap
  landclass.rs        - Dagor BLK landclass parser
  paint.rs            - terrain paint compositing
  rendinst.rs         - RIGz render-instance extraction
  inspect.rs          - `--inspect` structural dumper
  missions.rs         - mission BLK parser (spawns / zones)
  post.rs             - manifest + post-processing
  progress.rs         - batch progress / ETA rendering
  server.rs           - local static server for the viewer
  util.rs             - shared helpers
  viewer.html         - Three.js 3D viewer
  index.html          - batch gallery
  web/                - viewer JS modules (scene, hover, globe, tools, ...)
  World_Map.svg       - Robinson world map used by the Earth mini-globe
  oo2core_9_win64.dll - Oodle decoder (gitignored)
config.json           - datamine + client paths (gitignored)
config.sample.json    - example configuration
maps/                 - output (default): per-map directories (gitignored)
viewer_data/          - output (--local mode, gitignored)
docs/                 - technical documentation (formats, pipeline, issues)
ingame_map/           - standalone tactical-map renderer (gitignored)
```

## Supported texture formats

| Format | Description |
|--------|-------------|
| DXT1 (BC1) | Compressed, 4 bpp |
| DXT5 (BC3) | Compressed, 8 bpp |
| DXT5nm | Normal map (swizzled AG channels) — auto-detected |
| BC7 (DXGI 98) | Compressed, DX10 header — newer maps |
| A4R4G4B4 | Uncompressed, 16 bpp |
| A8R8G8B8 | Uncompressed 32-bit BGRA — B↔R swapped on read |

Compression containers handled: Oodle, zstd, zlib, lzma.

## Documentation

See [docs/README.md](docs/README.md) for an index of the technical
references (`BIN_FORMAT`, `BUILD_PIPELINE`, `PIPELINE_OVERVIEW`,
`HM2_REFERENCE`, `CDK_KNOWLEDGE`, `ORIENTATION`, `KNOWN_ISSUES`).
