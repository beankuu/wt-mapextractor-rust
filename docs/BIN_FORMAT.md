# War Thunder `.bin` Level Container — Format Reference

This document describes the on-disk layout of a War Thunder map `.bin` file
(e.g. `levels/air_israel.bin`) as observed by this project's inspector.
Everything here is reverse-engineered; field names come from War Thunder's
CDK source where available (see `docs/CDK_KNOWLEDGE.md`).

> The two reference inspect reports used to build this document are:
>
> - [`maps/air_israel/inspect.txt`](../maps/air_israel/inspect.txt)
> - [`maps/avg_japan/inspect.txt`](../maps/avg_japan/inspect.txt)

---

## Table of contents

1. [Container: `DBLD`](#1-container-dbld)
2. [Block chain](#2-block-chain)
3. [Block tag catalog](#3-block-tag-catalog)
   - [`eVER` — editor version stamp](#ever--editor-version-stamp)
   - [`RqRL` — required-resource list](#rqrl--required-resource-list)
   - [`DxP2` — DDSx texture-pack v2](#dxp2--ddsx-texture-pack-version-2)
   - [`.TEX` / `TEX.` — lightmap texture envelope](#tex--tex--lightmap-texture-envelope)
   - [`lmap` — landmesh + tile stream](#lmap--landmesh--tile-stream)
   - [`.HM2` — compressed detail heightmap](#hm2--compressed-detail-heightmap)
   - [`rivM` — river shader-mesh](#rivm--river-geometry-shader-mesh)
   - [`LMp2` — landmesh physmap v2](#lmp2--landmesh-physmap-v2-phys-materials--decal-meshes)
   - [`hspl` — heightmap splines](#hspl--heightmapland-splines-roads-rivers-fences-)
   - [`spgz` / `spgZ` / `splg` — spline spatial grid](#spgz--spgz--splg--spline-spatial-grid)
   - [`Lnav` / `Lnv2` / `Lnv3` / `Lnvs` / `CVRS` — nav meshes](#lnav--lnv2--lnv3--lnvs--cvrs--navigation-meshes)
   - [`stbl` / `tabl` / `ncon` — spline intersection tables](#stbl--tabl--ncon--spline-intersection-tables)
   - [`wt3d` — water-3D bbox](#wt3d--water-3d-bbox-conditional)
   - [`WBBX` — world bbox](#wbbx--world-bbox)
   - [`OCCL` — occluders](#occl--occluders-occlusion-system)
   - [`.SCN` / `ENVI` — render scenes](#scn--envi--render-scenes)
   - [`RIGz` — render-instance dump](#rigz--render-instance-dump)
   - [`.FRT` — land ray-tracer dump](#frt--land-ray-tracer-dump)
   - [`.END` — terminator](#end--terminator)
4. [Inline structures](#4-inline-structures)
   - [`lndm` v4 header](#lndm-v4-header)
   - [`LTdump` / `RTdumpX` — ray-tracer dumps](#ltdump--rtdumpx--ray-tracer-dumps)
   - [Terrain-paint cell prefix (15 bytes)](#terrain-paint-cell-prefix-15-bytes)
   - [`DDSx` texture stream](#ddsx-texture-stream)
5. [Per-map findings](#5-per-map-findings)
   - [`air_israel.bin`](#air_israelbin)
   - [`avg_japan.bin`](#avg_japanbin)
6. [What is still unknown](#6-what-is-still-unknown)

---

## 1. Container: `DBLD`

Confirmed via `DagorEngine/prog/tools/sceneTools/daEditorX/de_exportlevel.cpp:1691`:

```
offset  size  field
------  ----  -------------------------------------------------
0x00    4     magic           = '_MAKE4C('DBLD')'   \u2192 ASCII "DBLD"
0x04    4     DBLD_Version    = version FourCC (on PC builds: "3x64";
                                the literal is a version token, not a
                                platform tag)
0x08    4     textureCount    = number of `.TEX` texture blobs in the
                                file (i.e. trh.getTexturesCount())
0x0c    ...   first block header (`eVER`)
```

The dispatcher in `dbldUtil/dbldDeps.cpp` checks `'DBLD'` and the
version FourCC, reads the texture count, then enters the standard
`beginTaggedBlock` loop.

## 2. Block chain

Every block is a tagged, length-prefixed record:

```
offset  size  field
------  ----  --------------------------------------------------
+0      4     lengthflags (u32 LE)
                bits [0..30)  = length  (size of block body in bytes)
                bits [30..32) = flags   (block-specific, often 0)
+4      4     tag (4 ASCII bytes; leading '.' is common, e.g. '.TEX')
+8      N     body (length bytes, including the tag? no — see note)
```

**Important:** the length field already includes the 4-byte tag, so the
next block starts at `pos + 4 + length`. A body therefore has
`length - 4` payload bytes after the tag.

The walker stops when it sees a tag containing all three of the letters
`E`, `N`, `D` (typically `.END`, length `4`, zero-byte body).

## 3. Block tag catalog

All tags observed across the two reference maps, in DBLD order.

### `eVER` — editor version stamp

Body layout (confirmed via `DagorEngine/prog/tools/sceneTools/daEditorX/de_exportlevel.cpp:1701`):

```
+0   dwString  buildVersion    (dword-prefixed, padded-to-4 ASCII, e.g. "DaEditorX ver 5.0")
+..  bytes     zeroes((sha1TagCount + 1) * 24)   ← SHA-1 hash placeholder slots
                 per slot: 4 B tag-id + 20 B SHA-1 digest
                 patched in post-export after each tagged block is hashed
+..  align8
```

The "unknown" tail bytes `0x20..0x110` are simply zeroed SHA-1 digest slots
(one per internal tag the editor tracks). Useful only as provenance; not
consumed by the extractor.

### `RqRL` — required-resource list

Serialised `RoNameMap` listing **all asset names required by the level**
(rendinst landclasses, props, FX, textures, etc.), used by the streaming
loader to prefetch resources before the level renders. Writer:
`de_exportlevel.cpp:1710` (`RoNameMapBuilder::prepare/writeHdr/writeMap`
+ `StrCollector::writeStrings`). Body layout:

```
setOrigin
+0   RoNameMap header  (mkbindump::RoNameMapBuilder::writeHdr)
                        — 2 × PatchablePtr32 + u32 count + u32 pad (16 B)
                        — pointers are on-disk *origin-relative* offsets
                          (patched at load time by dagor_ro_api)
+16  Strings block     (StrCollector::writeStrings) — packed null-terminated
                        ASCII, dword-aligned
+N   Map table         (RoNameMapBuilder::writeMap) — array of u32 indices
                        into the string block, sorted for binary search
popOrigin + align8
```

The two words visible at `+0x00` / `+0x04` in a hex dump are the two
patchable pointers (origin-relative offsets, not real counts — which is
why they look like magic numbers like `0x0001` or `0x0f10`). The true
name count lives at `+0x08`.

Example (`air_israel`, 11 names): `air_israel_fields_b`, `alps_snow`,
`euraspen_fall_03_rendinst`, `israel_bushes_a`, `israel_city_a`,
`near_east_courtyard_c_k`, `near_east_town_building_3_floor_[b|c|d]`,
`ruhr_forest`, `tr_snow_a`.

The `avg_japan` map has 181 entries (`0xb5`) covering buildings, foliage,
fences and props. The extractor already parses these from the decompressed
`RIGz` body (see [`src/rendinst.rs`](../src/rendinst.rs)); `RqRL` is a
quick-lookup table for the runtime resource manager.

### `DxP2` — DDSx texture-pack, version 2

Used when `get_gameres_sys_ver() == 2` (gameres V2 pipeline, always on
for current War Thunder builds). Header precedes one or more `TEX`
sub-blocks carrying DDSx payloads. Writer:
`de_exportlevel.cpp:219` (`TextureRemapHelper::saveTextures`). Body layout:

```
+0   dwString  packName      = literal "~"  (gameres sys-V2 marker)
+..  u32       texCount
+..  dwString[texCount]       each entry is "<assetName>*" (the '*' suffix
                              is Dagor's texture-asset-lookup convention)
+..  align8
```

Followed immediately (as siblings in the DBLD chain, not nested) by
`texCount` × `TEX` blocks, each holding one DDSx blob:

```
TEX block body:
+0   dwString  assetName  (no '*' suffix here)
+..  u32       ddsxLen
+..  u8[ddsxLen]  raw DDSx bytes (see §4 DDSx)
+..  align8
```

Observed names in `avg_japan`: `japan_pagoda_foundation_a_b_overlay*`,
`wallpaper_b_tex_n*`, `brick_large_blocks_a_floor_d*`,
`brick_large_blocks_a_n*`, `stucco_new_d*`, `stucco_new_n*`,
`stucco_white_b_n*`. Not currently consumed by the extractor.

### `.TEX` / `TEX.` — lightmap texture envelope

`.TEX` is a single large block (22 MiB in `air_israel`, 4.2 MiB in
`avg_japan`) whose body is:

```
+0   u32      name length
+4   [name]   "<map>_lightmap"
+??  [pad]    zero / small fixed header
+??  DDSx     one big lightmap texture (often BC1/BC3)
```

`TEX.` is a 4-byte "closing bracket" that simply marks the end of the
lightmap envelope; it has no payload beyond the tag.

### `lmap` — landmesh + tile stream

The heart of the file. Body contains:

```
[0]     'lndm' v4 header        (see §4 lndm v4 header)
[..]    mesh grid table         (at base + meshMapOfs)
[..]    detail section          (at base + detailDataOfs)
           — NOT a flat tile stream; internal layout:
             +0     u32 N        byte-length of the string-name blob
             +4     u32[4]       4 fixed header words (16 bytes)
             +20    u32[cols×rows]  per-cell detail index table
             +20+cols×rows×4    string name blob (N bytes)
             +20+cols×rows×4+N  TILE CELL STREAM starts here
                  sequence of 15-byte cell prefixes (§ terrain-paint prefix)
                  each followed by its terrain DDSx tile
[..]    'LTdump' ray tracer     (at base + rayTracerOfs; see §4 LTdump)
```

**Note:** the `tileDataOfs` field in the lndm header points to an ASCII
text region (BLK name list), NOT the tile cell stream. The tile cell
stream is always embedded inside the `detailDataOfs` section per the
layout above.

In `air_israel` the `.HM2` heightmap is *embedded inside* the `lmap`
block (this is a common layout). In `avg_japan` the heightmap gets its
own top-level `.HM2` block.

### `.HM2` — compressed detail heightmap

Writer: `hmlIGenEditorPlugin.cpp:2086`. Detail heightmap (higher
resolution than the coarse `hmap` counterpart), compressed with one of
the Dagor packers. Body layout:

```
+0    f32   cellSize        (= gridCellSize / detDivisor, world units per pixel)
+4    f32   hmin            (world-space min height)
+8    f32   hdelta          (world-space range; h = hmin + raw*hdelta/65535)
+12   f32   worldOriginX    (= detRect[0].x)
+16   f32   worldOriginZ    (= detRect[0].y)
+20   u32   widthAndVersion = width | (version << HMAP_WIDTH_BITS)
                             HMAP_WIDTH_BITS = 24 → version in high 8 bits
+24   u32   height
+28   i32   exclBBoxMinX    (cell-space excluded BBox; terrain holes)
+32   i32   exclBBoxMinY
+36   i32   exclBBoxMaxX
+40   i32   exclBBoxMaxY
// only for HMAP_CBLOCK_DELTAC_VER (compressed-block variant):
+44   u32   packedConfig = (exportChunkSz)
                          | (hrb_subsz_bits << 8)
                          | (block_width_shift << 0)
+..   compressed chunks    (one or more sub-blocks emitted via
                            BinDumpSaveCB::beginBlock/endBlock — each
                            sub-block is oodle / zstd / lzma packed
                            depending on export flags)
```

Internal raw format (after decompression) is `CompressedHeightmap` — a
fixed-size block pyramid where each block stores a `blockVariance` byte
followed by `(blockSize-1)` delta-encoded i8 per-pixel height deltas.

See [`docs/HM2_REFERENCE.md`](HM2_REFERENCE.md) for the extractor's
consumption path.

The sibling `hmap` block (same plugin, line 2294) carries the **coarse**
heightmap in an older uncompressed format; `hset` (line 2314) is a
12-byte terrain-LOD params triple: `u32 gridStep, u32 radiusElems,
u32 ringElems`.

### `rivM` — river geometry shader-mesh

Writer: `hmlIGenEditorPlugin.cpp:2388` (via
`hmlService->exportGeomToShaderMesh`). Static mesh of all polygonal
water surfaces (rivers, lakes) baked into a single ShaderMesh dump, with
the source DataBlock saved out to `<plugin>/.work/rivers.dat`. Opaque to
the extractor.

### `LMp2` — landmesh physmap v2 (phys-materials + decal meshes)

Writer: `hmlIGenEditorPlugin.cpp:1752`. Body layout:

```
+0    u32       version = 1
+4    u32       pmNameCount
+..   dwString[pmNameCount]    phys-material names ("default", "soil",
                               "roadSoil", "roadSand", "concrete",
                               "rocksSlippery", "rocks", ...)
+..   u32       physMap_w
+..   u32       physMap_h
+..   f32       worldOriginX   (= detRect[0].x, or 0 if no detail hmap)
+..   f32       worldOriginZ   (= detRect[0].y)
+..   f32       cellSize       (= gridCellSize/detDivisor)
+..   <TreeBitmap>              phys-material id per cell, compressed as a
                                 2-D hierarchical bitmap
                                 (see mkbindump::save_tree_bitmap)
+..   i16       decalNodeCount
+..   per decal node:
         i16           vertexCount
         Point2[vc]    xz-projected vertices
         i16           tvertCount
         Point2[tvc]   UV coordinates
         // then per-material face groups until terminator:
         repeat {
           i8    physMatId               (0 → terminator if paired with i16(0))
           i16   faceCount
           i16   bitmapId                (omitted on first sentinel)
           i16[3][fc]  tface UVs
           i16[3][fc]  face vertex indices
         } until (i8 physMatId == 0 && i16 faceCount == 0)
+..   i16       textureCount
+..   per texture:
         PackedDecalBitmap (binary alpha-threshold raster for decal UV atlas)
```

Runtime loader: `lmeshManager.cpp:loadPhysMap()` dispatches `LMpm`/`LMp2`
to `load_phys_map_with_decals()` in
`DagorEngine/prog/gameLibs/physMap/physMapLoad.cpp`. Not consumed by this
extractor.

### `hspl` — HeightmapLand splines (roads, rivers, fences, ...)

Writer: `hmlExportSplines.cpp`. Companion blocks `stbl`, `tabl`, `ncon`,
`spgz`/`spgZ`/`splg` carry auxiliary intersection/grid data. Body layout:

```
+0   u32   splineCount
for each spline:
  +0   u32       splineBlkSize     (size of following asset-blk dump)
  +..  DataBlock dump              binary DataBlock: spline class name
                                   (e.g. "country_road_asphalt_a_167"),
                                   texturing params, asset overrides
  +..  u32       pointCount
  +..  per point:
          Point3   pos                    (12 B)
          Point3   relIn                  (12 B, inbound tangent)
          Point3   relOut                 (12 B, outbound tangent)
          u32      classAndFlags          (layer / corner / attach flags)
          u32      scaleTcAlong / width   (packed spline-class fields)
          ...      additional per-point fields depending on spline class
```

Seen in `air_israel` (420 KiB, many `country_road_asphalt_a_*` entries).
Absent from `avg_japan` (no road network in that map).

### `spgz` / `spgZ` / `splg` — spline spatial grid

Part of the HeightmapLand spline subsystem (sibling of `hspl`). Provides
an xz-aligned lookup grid mapping world-space cells to the splines that
pass through them (used by runtime queries such as "which road am I on?").
Writer: `hmlExportSplines.cpp` (`SplineGrid::write`). Body layout:

```
+0   u32   width                (grid columns)
+4   u32   height               (grid rows)
+..  i32[width*height]          per-cell record index into the per-spline
                                dense array — packed arrangement varies
                                by variant:
                                  splg = raw uncompressed
                                  spgZ = oodle-compressed
                                  spgz = zstd-compressed  (zstd magic
                                          28 b5 2f fd at +8)
```

### `Lnav` / `Lnv2` / `Lnv3` / `Lnvs` / `CVRS` — navigation meshes

Recast/Detour AI navigation meshes. Writers: `recastNavMesh.cpp:2661` and
`:2707`. Layout is identical: each block body is a blob produced by
`BinDumpSaveCB::copyDataTo` wrapping a `dtNavMesh` serialized tile set
(Detour native format). Variants:

- `Lnav` — single-tile (non-tiled) detour nav mesh (legacy)
- `Lnv2` — multi-tile detour nav mesh without tile cache
- `Lnv3` — multi-tile detour nav mesh **with** `dtTileCache`
  (dynamic obstacles — preferred for modern maps)
- `Lnvs` — top-level container wrapping **multiple** nav meshes per map:
  ```
  +0  u32 fmt = 1
  +4  u32 exportMask          (bitfield over MAX_NAVMESHES)
  per set bit:
    dwString  kind            (e.g. "main", "vehicle")
    nested block (Lnav/Lnv2/Lnv3)
  ```
- `CVRS` — cover points blob (recast cover builder output)

All payloads are opaque to the extractor (Detour-internal format).

### `stbl` / `tabl` / `ncon` — spline intersection tables

Companion blocks to `hspl` produced by the HeightmapLand spline
subsystem. Writer: `hmlExportSplines.cpp`. **Not a string table** — the
name is misleading; these carry spline-intersection graph data.

- **`stbl`** — shared-point table (intersections of 2+ splines):
  ```
  +0  u32 intersectionCount
  per intersection:
    u32 pointCount
    per point:
      f32 pos                     (xz-projected position component)
      i16 splineId
      i16 pad/flags
  ```
- **`tabl`** — per-intersection distance table:
  ```
  per entry: i32 dist + i16 nodeId + i8 nodePointId + i8 pad
  ```
- **`ncon`** — intersection node connectivity graph (adjacency list).

Only present in maps that have a road/spline network (`air_israel` has
`stbl`; `avg_japan` does not).

### `wt3d` — water-3D bbox (conditional)

Writer: `hmlIGenEditorPlugin.cpp:2352`. Written only when the level
contains at least one spline/poly object with `polyGeom.altGeom` and a
positive `bboxAlignStep` (i.e. has a water volume). Body:

```
+0   u32       version      = 0x20150909   (date stamp: 2015-09-09)
+4   u32       hasWater     (0 or 1)
if hasWater:
  +8   BBox2   water_bb     (min.x, min.y, max.x, max.y — xz projection, 16 B)
```

The tag is `wt3d` (ASCII), despite the content being a 2-D xz bbox;
`3d` refers to the water-surface subsystem, not the bbox dimension.

### `WBBX` — world bbox

Writer: `hmlIGenEditorPlugin.cpp:2373`. Body is a single `BBox3`:

```
+0   f32  min.x
+4   f32  min.y
+8   f32  min.z
+12  f32  max.x
+16  f32  max.y
+20  f32  max.z
```

Used by the render-inst streamer to cull the entire map. Byte dump
(`air_israel`):

```
00 00 80 c7  5c ef bc c3  00 00 80 c7
a0 14 80 47  66 ee 2f 45  01 e0 7f 47
```

decodes to `min = (-65536, -377.87, -65536)`, `max = (65664, 2814.89, 65504)`.

### `OCCL` — occluders (occlusion system)

Writer:
`DagorEngine/prog/tools/sceneTools/daEditorX/Occluders/plugin_occ.cpp`
(and mirrored by `de_appwnd.cpp`). Body layout:

```
+0   u32     version   = 0x20080514     (date stamp: 2008-05-14)
+4   u32     occluderCount              (= boxCount + quadCount)
+..  OcclusionMap::Occluder[occluderCount]
```

`OcclusionMap::Occluder` is a union (defined in
`DagorEngine/prog/engine/sharedInclude/scene/dag_occlusionMap.h`):

- **box variant**: 8 × `Point3` corners (96 B) + 4×3 `TMatrix` box-space
  (48 B) = 144 B
- **quad variant**: 4 × `Point3` corners (48 B) + 1 × `Plane3` (16 B) = 64 B

Each record is written as `sizeof(Occluder)` = max of both = **144 bytes**
regardless of kind; the loader discriminates by the plane/TMatrix field
being zeroed. Used by the runtime for portal/occlusion culling.

### `.SCN` / `ENVI` — render scenes

Decoded by `RenderScene::loadBinary` in
`DagorEngine/prog/engine/scene/renderScene.cpp`. The DBLD dispatcher
(`dbldUtil/dbldDeps.cpp`) treats `SCN` as the main static-scene graph
and `ENVI` as the environment scene (skybox, distant terrain billboards,
etc.) — both share the same binary format.

Body begins with the `scn2` sub-magic (visible as `scn2$ ` because the
following `$` is a field delimiter in the loader's tokenizer):

```
+0   ASCII   "scn2"
+4   u32     subVersion flags (`0x20130524` — date code 2013-05-24)
+8   u32     nodeCount
+..  nodes   RenderScene tree:
               per node: TMatrix (48 B) + asset-name dwString + child count
                        + material/shader references
```

In both maps the `.SCN` contains references to generic assets such as
`water_aces` (air_israel). Not consumed by the viewer.

### `RIGz` — render-instance dump

Wraps two compressed sub-blocks. Header at block body offset 0:

```
+0   u32  sub1_lengthflags
           flags = 0 → lzma,   1 → zstd,   2 → oodle,   3 → raw
+4   N    sub1 body (compressed; decompresses to the RIGz dump)
+4+N u32  sub2_lengthflags
+8+N M    sub2 body (compressed; rendinst per-instance transform stream)
```

Both reference maps use `flags=2` (oodle) for sub1 and `flags=0` for sub2
(which is small and may be raw despite the flag). See
[`src/rendinst.rs`](../src/rendinst.rs) for the decompressed dump layout
(cells × 552 B, pools × 32 B, landclasses × 32 B).

### `.FRT` — land ray-tracer dump

Payload starts with `RTdumpX` and wraps a single zstd stream (zstd magic
`28 b5 2f fd` at +16 inside the block body). Decompresses to an `LTdump`
style mesh used for ballistic ray tracing against the terrain and
buildings. Not consumed.

### `.END` — terminator

4-byte block with zero-length body (just the tag). The walker stops on
any tag containing `E`, `N`, and `D`.

## 4. Inline structures

### `lndm` v4 header

Located inside `lmap`. Signature `lndm` + 44 bytes of header:

```
offset  size  field
------  ----  --------------------------------
+0      4     magic          = 'lndm'
+4      4     version        = 4
+8      4     gridCellSize   (heightmap units per grid cell)
+12     4     landCellSize   (world metres per land cell)
+16     4     mapSizeX       (cells)
+20     4     mapSizeY       (cells)
+24     4     originCellX    (cell offset of map origin; usually -mapSizeX/2)
+28     4     originCellY
+32     4     useTile        (bool)
+36     4     meshMapOfs     ⎫
+40     4     detailDataOfs  ⎬ all relative to the header address of `meshMapOfs` (= magic + 36)
+44     4     tileDataOfs    ⎪ NOTE: despite its name, points to a BLK ASCII text region,
+48     4     rayTracerOfs   ⎭        NOT the tile cell stream. Tile stream is inside detailDataOfs section.
```

### `LTdump` / `RTdumpX` — ray-tracer dumps

`LTdump` (in `lmap`, terrain rays) and `RTdumpX` (in `.FRT`, prop rays)
share a prefix:

```
+0   [6|7] ASCII  "LTdump" / "RTdumpX"
+..  4     numCX
+..  4     numCY
+..  4     cellSize (f32)
+..  12    offset   (vec3 f32)
+..  12    bmin     (vec3 f32)
+..  12    bmax     (vec3 f32)
+..  4     cellsCount
+..  [64 B per cell]  cell entries
```

### Terrain-paint cell prefix (15 bytes)

Prefixes every terrain DDSx tile inside `lmap`. Confirmed by both reference
reports:

```
offset  size  field
------  ----  --------------------------------------
+0      7     det[7]       up to 7 detail-index bytes; 0xff = unused slot
+7      4     totalLen     byte length of this cell's payload (0 = empty)
+11     4     tex2Offset   byte offset of the 2nd texture inside the payload,
                           or == totalLen if the cell has only one texture
+15     …     payload      one or two DDSx blobs packed back-to-back
```

Walk rule: if `totalLen == 0`, advance 15 bytes (empty cell). Otherwise
advance `15 + totalLen` bytes. The walker must also rewind past up to
`mapSizeX × mapSizeY` empty prefixes at the start (both maps showed
trailing-empty runs of 0–7 cells).

### `DDSx` texture stream

Each DDSx blob is a standard War Thunder DDSx header + compressed body:

```
+0   4    magic         = 'DDSx'
+4   4    format_code   (e.g. 'DXT1', 'DXT5')
+8   16   reserved/flags
+24  4    mem_sz        decompressed size
+28  4    packed_sz     compressed size (0 ⇒ uncompressed)
+32  N    body          size = packed_sz if > 0 else mem_sz
```

Extractor uses `mem_sz` / `packed_sz` to walk the stream without parsing
the inner texture format (see [`src/extract.rs`](../src/extract.rs)).

## 5. Per-map findings

### `air_israel.bin`

| # | Offset       | Tag   | Length      | Notes                              |
|---|-------------:|-------|------------:|------------------------------------|
| 0 |   `0x00000c` | eVER  |         272 | editor version stamp               |
| 1 |   `0x000120` | RqRL  |         348 | 11 rendinst landclasses            |
| 2 |   `0x000280` | DxP2  |          20 | short decal pointer                |
| 3 |   `0x000298` | .TEX  |  22,369,724 | lightmap (single big DDSx)         |
| 4 |   `0x155858` | TEX.  |           4 | envelope closer                    |
| 5 |   `0x155860` | lmap  |  14,521,116 | landmesh + HM2 + tile stream       |
| 6 |   `0x232eb80`| LMp2  |     935,932 | land phys-mats                     |
| 7 |   `0x2413380`| hspl  |     419,836 | **road splines**                   |
| 8 |   `0x2479b80`| spgz  |      32,316 | zstd blob (sparse grid?)           |
| 9 |   `0x24819c0`| stbl  |      97,572 | shared string table                |
|10 |   `0x24996e8`| wt3d  |          28 | 3D bbox                            |
|11 |   `0x2499708`| WBBX  |          28 | world bbox (see §3 WBBX)           |
|12 |   `0x2499728`| .SCN  |         684 | scene graph                        |
|13 |   `0x24999d8`| RIGz  |   1,662,236 | rendinst (oodle + zstd)            |
|14 |   `0x262f6f8`| .FRT  |   1,689,824 | RTdumpX for props                  |
|15 |   `0x27cbfdc`| .END  |           4 | terminator                         |

Landmesh: `32×32` cells × 4096 m = **131 km × 131 km logical domain**
(flight-sim map with 2 km × 2 km engagement tiles). 563 DDSx tiles total
(1 lightmap + 562 terrain tiles). 562 of 1024 terrain cells are
populated — the rest are open water/sky.

### `avg_japan.bin`

| # | Offset       | Tag   | Length      | Notes                              |
|---|-------------:|-------|------------:|------------------------------------|
| 0 |   `0x00000c` | eVER  |         272 | editor version stamp               |
| 1 |   `0x000120` | RqRL  |       5,308 | **181** rendinst landclasses       |
| 2 |   `0x0015e0` | DxP2  |         212 | 7 decal texture refs               |
| 3 |   `0x0016b8` | .TEX  |   4,388,716 | lightmap                           |
| 4 |   `0x430e28` | TEX.  |           4 | envelope closer                    |
| 5 |   `0x430e30` | lmap  |   9,859,284 | landmesh + tile stream             |
| 6 |   `0xd97f08` | .HM2  |   2,225,724 | **standalone** heightmap           |
| 7 |   `0xfb7548` | LMp2  |     459,148 | 6 phys-mats                        |
| 8 |   `0x10276d8`| WBBX  |          28 | world bbox                         |
| 9 |   `0x10276f8`| Lnav  |     211,740 | **navigation mesh** (zstd)         |
|10 |   `0x105b218`| OCCL  |         188 | 6 occlusion boxes                  |
|11 |   `0x105b2d8`| .SCN  |      74,948 | scene graph (large)                |
|12 |   `0x106d7a0`| RIGz  |   2,357,636 | rendinst (oodle + raw)             |
|13 |   `0x12ad128`| .FRT  |      34,872 | RTdumpX                            |
|14 |   `0x12b5964`| .END  |           4 | terminator                         |

Landmesh: `32×32` cells × 2048 m = **65 km × 65 km** (ground-battle map
with 2 km engagement tiles). 1021 DDSx tiles (1 lightmap + 1020 terrain).
1020 of 1024 terrain cells are populated — the map is nearly fully
covered by terrain, consistent with a dense AVG battle zone.

## 6. What is still unknown

After cataloging every named block, the inspector reports these bytes as
"unknown" (i.e. residing inside a known container but with an
unparsed internal layout):

| Map          | File size  | Cataloged | Unknown          |
|--------------|-----------:|----------:|-----------------:|
| `air_israel` |  39.80 MiB |  54.69 %  |  45.31 % (18 MiB)|
| `avg_japan`  |  18.71 MiB |  27.94 %  |  72.06 % (13 MiB)|

The large unknown bytes are **not mystery regions** — they are the
bodies of known blocks (compressed `.HM2`, lightmap DDSx, compressed
`RIGz`/`Lnav`/`.FRT` payloads, `LMp2` property tuples, `hspl` spline
data, `stbl` intersection pool, scene-graph node bodies inside `.SCN`).
They are flagged "unknown" only because the inspector does not re-descend
into them after claiming the outer block header; the actual parsers
for `.HM2`, terrain DDSx, `RIGz` and the `lndm`/`LTdump` subsystems
*do* handle their respective regions.

All previously-listed "genuine unknowns" have been **resolved** against
the Dagor Engine editor source (`D:\DagorEngine\prog\tools\sceneTools\`)
and runtime (`D:\DagorEngine\prog\gameLibs\`). Summary of resolutions:

| Unknown (old)              | Resolution (authoritative source)                                  |
|----------------------------|--------------------------------------------------------------------|
| `eVER` bytes 0x20..0x110   | SHA-1 digest slots, one per hashed sub-block (post-export patch)   |
| `RqRL` header              | `RoNameMap`: 2 PatchablePtr32 + u32 count + strings + index table  |
| `DxP2` fixed fields        | `dwString "~"` + `u32 texCount` + N × `dwString "<asset>*"`         |
| `LMp2` per-material record | tree-bitmap + i16 decal nodes + per-node face groups (above)       |
| `hspl` control points      | per-spline DataBlock dump + u32 pointCount + Point3×3 + flags       |
| `spgz` payload             | zstd-compressed `SplineGrid` (width × height × i32 record array)    |
| `Lnav` payload             | Detour `dtNavMesh` serialized tile set (opaque Detour format)      |
| `.FRT` payload             | `RTdumpX` wrapped in a zstd stream (LTdump-family)                 |
| `OCCL` per-box record      | `u32 date + u32 count` + N × 144-byte `OcclusionMap::Occluder`      |
| `.SCN` scene-node record   | `RenderScene::loadBinary`: "scn2" + u32 ver + u32 count + nodes    |

Remaining nuances (refinement targets, not blockers):

- **`HM2` compressed-chunk layout** (sub-blocks with per-chunk packer
  choice) — understood structurally but not implemented by the inspector.
- **`LMp2` TreeBitmap** exact encoding — see
  `DagorEngine/prog/tools/libTools/util/makeBindump.cpp`
  (`save_tree_bitmap`).
- **`RenderScene` node record tail** — per-node shader-mesh references
  and LOD-distance table layout.
- **Detour nav-mesh internals** — opaque by design; would require
  linking/implementing Detour to decode.

When the extractor/parser changes, refresh and diff the reference inspect
reports under `maps/air_israel/` and `maps/avg_japan/` to confirm coverage
does not regress.
