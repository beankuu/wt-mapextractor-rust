# HM2 (CompressedHeightmap) Reference

Consolidated reference for HM2 heightmap architecture, decompression, and viewer integration.

---

## 1. Engine Architecture

### HM2 Replaces the Land Mesh

In the Dagor engine, HM2 (CompressedHeightmap) **replaces** the LandRayTracer mesh
in the battle area via the `excludeBounding` mechanism — it does not overlay it.

1. `LandMeshManager::loadHeightmapDump()` loads the HM2 block from the level binary
2. Stores the HM2's exclude bounding box in `cullingState.exclBox`
3. Sets `cullingState.useExclBox = true`
4. During rendering, land mesh cells inside `exclBox` are culled:
   ```cpp
   // lmeshCulling.inc.cpp:53
   if (useExclBox && borderX >= exclBox[0].x && borderX < exclBox[1].x ...) return;
   ```
5. The HM2 heightmap shader provides terrain geometry for the excluded region

### Shader Height Decoding

From `heightmapHandler.cpp`:
```
height = texel * hScale + hMin
```
Where `hScaleRaw = hScale / 65535.0` per u16 value.

### Key Source Files

| File | Role |
|------|------|
| `heightmapPhysHandler.cpp` | HM2 data loading, `getHeight()` sampling |
| `heightmapHandler.cpp` | Rendering, shader vars, GPU texture upload |
| `lmeshManager.cpp:1797` | `loadHeightmapDump()` — sets `exclBox` |
| `lmeshCulling.inc.cpp` | Cell culling using `exclBox` |
| `dag_compressedHeightmap.h` | BC4-style block compression format |

---

## 2. Our Approach — Dual Output with Float Overlay

1. **Float overlay on LR** — HM2 world heights are overlaid onto the LR float
   heightmap (before u8 quantization) with 16-pixel feather blending at edges.
   Also saved as `heightmap_float.npy` for precise water masking.

2. **Separate full-resolution HM2 output** — `heightmap_detail.png` at native
   resolution (1024–2048px). The viewer creates a second geometry plane.

3. **Viewer** — cuts a hole in the main mesh where HM2 sits and renders the
   detail plane at full resolution.

### Why Not Simple Overlay?

Downsampling HM2 into the LR output image loses resolution:
- avg_poland: 2048² HM2 → 128×128 in LR = **16× downsample**
- avg_karelia: 1024² HM2 → 256×256 = **4× downsample**

And mapping to u8 (0–255) loses precision:
- karelia LR range: 817m → u8 step = **3.2m** → only 31 unique values for 100m of HM2 variation

---

## 3. Locating & Parsing the HM2 Block

Scan for `HM2\x00` tag from offset `0x10000` in the `.bin` file.

### Header (55 bytes)

| Offset | Type | Field | Notes |
|--------|------|-------|-------|
| +0 | char[4] | tag | `HM2\x00` |
| +4 | u32 LE | decomp_size | Total decompressed bytes |
| +8 | float | heightScale | Usually `1.0` |
| +12 | float | heightMin | e.g., `28.0` |
| +16 | float | heightMax | e.g., `195.0` |
| +24 | u16 LE | numSubX/Y | Subdivision counts |
| +28 | u16 LE | gridWidth/Height | Per sub-tile |
| +51 | u24 LE | comp_total | Compressed data size (3 bytes) |
| +55 | — | **data start** | Oodle compressed data begins |

---

## 4. Decompression

Uses Oodle compression (Kraken). Handled natively by `oozextract` (pure-Rust
reimplementation of ooz; no DLL required). `oo2core_9_win64.dll` can be
supplied via `--oo2core <path>` as a fallback if needed.

### Strategy

| Path | Description | Maps |
|------|-------------|------|
| **Single-stream** | One `OodleLZ_Decompress` call | 46 maps (decomp = 4,194,304) |
| **Multi-stream** | Binary-search pages per stream, scan for next stream start | 8+ maps |
| **Partial + padding** | Pad tail with last row when multi-stream is incomplete | ~5 maps |

### Oodle Buffer Safety

`OodleLZ_Decompress` reads far beyond declared `comp_size`. All compressed data
must be copied into a single oversized ctypes buffer with 256 KB padding.

### Interpreting Output

Decompressed bytes = u8 heightmap grid (`dim × dim` where `dim = sqrt(decomp_size)`).
Height: `heightMin + (pixel / 255.0) * (heightMax - heightMin)`.

For non-square maps (e.g., hangar_field = 2048×2032), try power-of-2 widths.

---

## 5. Manifest Format

```json
{
  "heightmapDetail": {
    "file": "heightmap_detail.png",
    "width": 2048, "height": 2048,
    "cellSize": 2.0,
    "height_min_m": -17.6, "height_max_m": 43.4,
    "world_x0": 0.0, "world_z0": 0.0,
    "world_x1": 4096.0, "world_z1": 4096.0
  }
}
```

---

## 6. Viewer Integration

- **Geometry**: Separate `PlaneGeometry` at HM2 world coordinates, up to 512 segments
- **Displacement**: Aligned with LR via `displacementScale` and `displacementBias`
- **Height slider**: HM2 mesh scaled proportionally when user adjusts slider
- **Main mesh cutout**: Fragment shader discards pixels inside HM2 bounding box
