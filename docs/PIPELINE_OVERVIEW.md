# WT-MapExtractor: Complete Pipeline Overview

## System Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                       WAR THUNDER MAP EXTRACTION PIPELINE                    │
└─────────────────────────────────────────────────────────────────────────────┘

┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ PHASE 1: RUST BACKEND - MAP EXTRACTION & PROCESSING                         ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

INPUT FILES
═══════════
     ↓
     ├─ from_client/levels/*.bin           [DBLD3x64 container with textures]
     ├─ from_client/content/base/res/*.dxp.bin  [Material texture packs]
     ├─ from_client/content.hq/hq_tex/res/*.dxp.bin  [HQ texture packs]
     └─ from_datamine/aces.vromfs.bin_u/levels/*.blkx  [Metadata: water level, coords]

RUST PROCESSING PIPELINE (pipeline.rs)
══════════════════════════════════════

   [1] Extract DBLD container
        ↓
        ├─ Read bin file header
        ├─ Scan for DDSx texture blocks
        └─ Parse lndm (terrain grid) metadata

   [2] Build DxP Global Index (dxp_index.rs)
        ├─ Scan all .dxp.bin packs
        ├─ Extract material definitions
        └─ Cache 57,577 textures (HQ packs preferred)
        
   [3] Extract Heightmap (heightmap.rs)
        │
        ├─ If HM2 block exists (detailed battle zones)
        │  ├─ Decompress Oodle + BC4 blocks
        │  ├─ Apply water_level ocean cutout
        │  └─ Save heightmap_detail.png (8192px @ 1px/m)
        │
        ├─ Else if LandRayTracer exists (terrain mesh)
        │  ├─ Decompress raytracer dump
        │  ├─ Rasterize triangle mesh to grid
        │  └─ Fill NaN regions (pyramid algorithm)
        │
        └─ Else pseudo-heightmap
           └─ Use normalmap blue channel or flat grey

   [4] Extract Overview Textures (export.rs)
        ├─ DDSx[0] → colormap.png (2048×2048) or normalmap.png
        ├─ Normalmap: detect if R≈0, B≈255 (DXT5nm format)
        ├─ If normalmap: convert tangent-space to RGB
        └─ Color overlay with water mask

   [5] Build Landclass Grid (landclass.rs)
        ├─ Parse lndm detailData BLK
        ├─ Extract material textures from DxP packs
        ├─ Build per-cell landclass weight maps
        └─ Detect sparse vs fully-populated grids

   [6] Build Terrain Paint (paint.rs) ⚙ GPU-accelerated
        ├─ If HM2 exists:
        │  ├─ Build detail terrain paint (cropped, high freq)
        │  └─ Save terrain_paint_detail.png (8192px)
        │
        ├─ Native splatmap render:
        │  ├─ For each cell: blend detTexIds[0..6] by weight
        │  ├─ Apply water mask (pixels below water_level → blue)
        │  ├─ GPU compositing via wgpu compute shader
        │  ├─ Gaussian blur (sigma = cell_px/40.0)
        │  └─ Save terrain_paint.png (4096-8192px max)
        │
        └─ Fallback (if native failed):
           ├─ Try tile_grid.png (per-cell landclass index)
           └─ Try colormap (pseudo-heightmap maps)

   [7] Generate Tile Grid (post.rs)
        ├─ Cell ID index map → RGB visualization
        ├─ Each RGB = (cell_x, cell_y, 0)
        └─ Save tile_grid.png (world-aligned)

   [8] Extract Render Instances (rendinst.rs)
        ├─ Parse RIGz block (vegetation, buildings, debris)
        ├─ Decompress cell-local pregenerated instances
        ├─ Group by landclass + style (destroyed, winter, etc)
        └─ Save rendinst.json (style + count per cell)

   [9] Generate Thumbnails & Manifest (pipeline.rs)
        ├─ Resize all PNGs → 256-512px thumbnails
        ├─ Collect metadata:
        │  ├─ mapSize (world extent)
        │  ├─ waterLevel
        │  ├─ heightmap bounds (height_min_m, height_max_m)
        │  ├─ textures (heightmap, normalmap, colormap, terrainPaint, etc.)
        │  ├─ landclasses (count, names, properties)
        │  ├─ materials (texture specs)
        │  └─ missions (CTA spawns, capture zones, if datamine available)
        │
        └─ Save manifest.json (2-5 KB metadata)


OUTPUTS (maps/<name>/)
════════════════════

   Stored Images:
   ├─ heightmap.png               [RGBA, float32 displacements, 1px/m resolution]
   ├─ heightmap_detail.png        [Cropped detail heightmap (HM2 battle area)]
   ├─ normalmap.png               [Normal map (tangent-space XYZ)]
   ├─ normalmap_detail.png        [Detail normal from HM2 gradients]
   ├─ colormap.png                [Overview RGB texture (2048×2048)]
   ├─ terrain_paint.png           [Native blended landclass composite (8192px max)]
   ├─ terrain_paint_detail.png    [Cropped detail paint (HM2 area)]
   ├─ tile_grid.png               [Per-cell landclass index (for fallback)]
   ├─ terrain_paint_thumb.png     [512px preview for gallery]
   ├─ colormap_thumb.png          [512px colormap thumbnail]
   ├─ heightmap_thumb.png         [512px heightmap thumbnail]
   └─ tile_grid_thumb.png         [512px tile grid thumbnail]

   Metadata:
   ├─ manifest.json               [Master index with all asset references]
   ├─ missions.json               [CTA spawns, capture zones (if datamine available)]
   └─ materials.json              [Diffuse + detail texture specs (if --export-mat)]


┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ PHASE 2: WEB VIEWER - BROWSER RENDERING & INTERACTION                       ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

VIEWER INITIALIZATION (viewer.html + terrain-init.js)
═════════════════════════════════════════════════════

   [1] Load Manifest
        ├─ fetch manifest.json
        └─ Extract map size, heightmap bounds, asset list

   [2] Initialize 3D Scene (Three.js + WebGL)
        ├─ Create camera, lighting, renderer
        ├─ Create PlaneGeometry (1024 segments when WebGPU available, else 512)
        └─ Setup orbit controls

   [3] WebGPU Detection & GPU Init
        ├─ Try to initialize WebGPU compute
        ├─ Set GPU badge (WebGPU / CPU)
        └─ Cache gpuAvailable flag

   [4] Load Primary Textures (PARALLEL via Promise.all)
        │
        │   ┌─────────────────────────────────────┐
        │   │   ALL of these load in PARALLEL:    │
        │   ├─────────────────────────────────────┤
        │   │ • heightmap.png                     │
        │   │ • normalmap.png                     │
        │   │ • colormap.png                      │
        │   │ • terrain_paint.png                 │
        │   │ • tile_grid.png                     │
        │   └─────────────────────────────────────┘
        │
        ├─ Apply minFilters, mipmaps, anisotropy
        └─ Compute UV offset/repeat for mapCoord alignment

   [5] Build Terrain Meshes
        ├─ Colormap Mesh (if colormap exists)
        │  ├─ Displaced by heightmap
        │  ├─ Normal-mapped
        │  └─ Semi-transparent overlay
        │
        ├─ Terrain Paint Mesh (if painted terrain exists)
        │  ├─ Main splatmap layer
        │  └─ Optional tile_grid overlay
        │
        ├─ Heightmap Mesh (for debugging)
        │  └─ Grayscale displacement view
        │
        └─ Splatmap Mesh (landclass index visualization)
           └─ Raw RGB cell IDs

   [6] HM2 Detail Heightmap (if available)
        │   ⚠ LAZY LOAD: This is deferred if detail heightmaps are optional
        │
        ├─ Load detail heightmap + detail paint texture
        ├─ Build 1024-segment detail geometry (cropped)
        ├─ Position at HM2 bounding box
        ├─ Compute detail height scale/bias for proper alignment
        └─ Add hidden by default (enabled via "Detail" toggle)

   [7] Material Textures & Render Instances (LAZY LOAD)
        │   ⚠ Deferred until user needs them
        │
        ├─ Load individual material tiles on-demand
        ├─ Load render instance meshes (vegetation, buildings)
        ├─ Fetch mission data (spawns, capture zones)
        └─ Cache loaded assets for reuse

   [8] UI Panel & Interactive Controls
        ├─ Height scale slider (0-100%)
        ├─ Layer toggles (colormap, terrain paint, heightmap, detail, etc)
        ├─ Sun light controls (azimuth, elevation, strength)
        ├─ Tool selection (ruler, line-of-sight, missions, etc)
        └─ Coordinate readout on hover

   [9] Continuous Rendering Loop
        ├─ Orbit camera via mouse/touch
        ├─ Update hover coordinates on mousemove
        ├─ Re-render on height scale change
        └─ WebGPU LoS compute async (batch async, throttled to 1 concurrent)


VIEWER FEATURES
═══════════════

   🗺 Terrain Visualization:
      ├─ Heightmap-displaced terrain (PlaneGeometry with vertex displacement)
      ├─ Multi-layer compositing (colormap + painted terrain + detail)
      ├─ Real-time sun lighting (directional + ambient)
      └─ Normal mapping + anisotropic filtering

   📐 Grid Analysis:
      ├─ Cell index overlay (RGB per-cell visualization)
      ├─ World coordinate readout on hover
      ├─ Landclass identification
      └─ Height sampling at cursor position

   🎯 Line of Sight Computation:
      ├─ GPU-accelerated (720 rays × 400 steps on WebGPU)
      ├─ CPU fallback (144 rays × 15 steps, no blocking UI)
      ├─ Draws from observer position to targets
      ├─ Occlusion testing against HM2 detail when available
      └─ Includes foliage collision via render instances

   🚁 Mission Overlays:
      ├─ CTA mission selection (dom, conq, bttl, ctf, etc)
      ├─ Team 1/2 spawn markers (blue/red)
      ├─ Capture zone cylinders (A/B/C labels)
      ├─ Battle area wireframe boxes (orange)
      └─ Mode toggle (Arcade/Realistic)

   🎮 Interactive Tools:
      ├─ Ruler: click → drag → measure distance
      ├─ Vector cross-hair overlay
      └─ Coordinate display in world meters + game units

   ⚙ Settings:
      ├─ Sun direction & intensity
      ├─ Height scale multiplier
      ├─ Camera near/far planes
      ├─ Detail level toggle
      └─ Layer opacity per mesh


┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ LAZY LOADING STRATEGY FOR WEB VIEWER                                        ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

PRIORITY TIERS (by importance to initial render):
═════════════════════════════════════════════════

   TIER 1 - CRITICAL (load immediately, block render):
   ├─ Manifest JSON
   ├─ Heightmap (geometry displacement)
   └─ Colormap OR terrain paint (surface color)

   TIER 2 - IMPORTANT (load in parallel, visible on startup):
   ├─ Normalmap (lighting quality)
   ├─ Tile grid (visual reference)
   └─ Sun/lighting setup

   TIER 3 - DETAIL (lazy load on first visibility):
   ├─ heightmap_detail.png (HM2 battle area)
   ├─ normalmap_detail.png
   ├─ terrain_paint_detail.png
   ├─ Material tiles (if needed)
   └─ Render instances (vegetation, buildings)

   TIER 4 - OPTIONAL (load only on demand):
   ├─ Mission data (JSON)
   ├─ Ancillary thumbnails
   └─ Mobile textures (reduced quality)


IMPLEMENTATION PLAN:
════════════════════

   Load Phase 1 (0-10%):
      └─ Manifest.json

   Load Phase 2 (10-25%) [PARALLEL]:
      ├─ heightmap.png
      ├─ colormap.png
      ├─ terrain_paint.png
      └─ normalmap.png

   Load Phase 3 (25-40%) [STAGGERED]:
      ├─ tile_grid.png
      └─ [Scene render begins here if Tier 1 complete]

   Load Phase 4 (background):
      ├─ heightmap_detail.png [on tab visibility + user hover over HM2 area]
      ├─ normalmap_detail.png
      └─ Material tiles [on-demand when shader samples them]

   Load Phase 5 (user interaction):
      ├─ missions.json [when user clicks "Missions" toggle]
      ├─ Render instances [when user enables "Foliage" layer]
      └─ Mobile-optimized texture variants [if mobile device detected]


BENEFITS:
═════════
✓ Faster initial page load (critical textures first)
✓ Scene renders with basic terrain while details load
✓ Reduced initial bandwidth for low-priority assets
✓ Better mobile performance (can skip detail tier on mobile)
✓ Graceful degradation (detail features optional)
✓ User can interact while background loading continues


┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ DATA STRUCTURES & COORDINATE SYSTEMS                                        ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

manifest.json Structure
═══════════════════════

{
  "mapName": "avg_krymsk",
  "mapSize": [81920, 65536],
  "mapCoord0": [-40960, -32768],
  "mapCoord1": [40960, 32768],
  "waterLevel": 10.0,

  "heightmap": {
    "file": "heightmap.png",
    "width": 4096, "height": 4096,
    "world_extent": [-40960, -32768, 40960, 32768],
    "height_min_m": 135.7, "height_max_m": 245.3,
    "pseudo": false, "source": "hm2"
  },

  "heightmapDetail": {
    "file": "heightmap_detail.png",
    "width": 8192, "height": 8192,
    "world_x0": -8192, "world_x1": 8192,
    "world_z0": -8192, "world_z1": 8192,
    "height_min_m": 150.2, "height_max_m": 240.5
  },

  "normalmap": {
    "file": "normalmap.png",
    "width": 2048, "height": 2048,
    "source": "overview"
  },

  "colormap": {
    "file": "colormap.png",
    "width": 2048, "height": 2048,
    "source": "overview"
  },

  "terrainPaint": {
    "file": "terrain_paint.png",
    "width": 8192, "height": 8192,
    "source": "native-material-weight-blend",
    "detail": {
      "file": "terrain_paint_detail.png",
      "width": 8192, "height": 8192
    }
  },

  "tileGrid": {
    "file": "tile_grid.png",
    "width": 4096, "height": 4096,
    "world_extent": [-40960, -32768, 40960, 32768]
  },

  "landclasses": [
    { "name": "krymsk_grass_a", "size": 128, ... },
    ...
  ],

  "materials": [
    { "name": "krymsk_grass_a_tex_m", ... },
    ...
  ],

  "missions": [
    { "type": "dom", "spawns": [...], "captureZones": [...] },
    ...
  ]
}


World Coordinate System
═══════════════════════

   Game World (War Thunder):
   ┌─────────────────────────────┐
   │  X-axis (East-West)         │  Z-axis (North-South)
   │  Positive = East            │  Positive = North
   │  mapCoord0/1 = game bounds  │
   └─────────────────────────────┘

   Heightmap PNG Layout (FLIP_LR applied):
   ┌─────────────────────────────┐
   │ PNG Row 0 = North           │  PNG (0,0) = NE corner
   │ PNG Col 0 = East            │
   │ Row increases downward       │  Pixel layout:
   │ Col increases rightward      │  (px, py) = (1 - u, 1 - v) * (W, H)
   └─────────────────────────────┘

   Three.js Scene:
   ┌─────────────────────────────┐
   │ sceneX = -(worldX - cx) * sx │
   │ sceneZ = +(worldZ - cz) * sz │
   │ Camera Y (height) = +up      │
   │ Displacement: vertex.y += heightmap(uv) │
   └─────────────────────────────┘


┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ FILE SIZE REFERENCE                                                         ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

Typical output sizes per map:
  ├─ heightmap.png               2-5 MB (depending on resolution)
  ├─ colormap.png                2-3 MB
  ├─ terrain_paint.png           4-15 MB (highly compressed PNG)
  ├─ heightmap_detail.png        6-12 MB (if HM2 present)
  ├─ normalmap.png               1-2 MB
  ├─ tile_grid.png               3-8 MB
  ├─ manifest.json               2-5 KB
  ├─ missions.json               50-200 KB (if available)
  └─ materials/ (optional)        100+ MB (per --export-mat flag)

   Total per large map:          ~20-50 MB (TIER 1-2 only)
                                 ~60-120 MB (with detail + materials)


┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ BROWSER PERFORMANCE NOTES                                                   ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

Texture Upload:
  ├─ 8192×8192 PNG → GPU upload: ~200-400ms per texture (on modern GPU)
  ├─ WebGL limit: 2-4 GB VRAM typical (enough for all terrain textures)
  ├─ Mipmap generation: automatic but can stall render for 50-100ms
  └─ Anisotropic filtering: improves quality at oblique angles (2-4x perf cost)

LoS Computation:
  ├─ GPU path: 720 rays × 400 steps = 2 frame stalls (~32ms @ 60fps)
  ├─ CPU path: 144 rays × 15 steps = negligible (< 1ms on modern CPU)
  ├─ Detail mesh LoS: Adds 50-100ms per frame if HM2 available
  └─ Async: GPU operations don't block UI, CPU does

Rendering:
  ├─ 1024-segment mesh: ~60fps on modern GPU (shader overhead)
  ├─ Multi-layer rendering: 3-4 meshes per frame, ~5-10% overhead per layer
  ├─ Shadows/reflections: Not currently used (would add 20-50% overhead)
  └─ Mobile target: Use 512-segment mesh, skip detail, reduce texture resolution

Memory:
  ├─ Typical viewer session: 500-800 MB for all TIER 1-2 textures
  ├─ With detail + material tiles: 1-2 GB
  ├─ Mobile target: <300 MB (use TIER 1-2 only, reduce resolution)
  └─ Cache: Browser disk cache keeps downloaded PNGs for repeat visits


