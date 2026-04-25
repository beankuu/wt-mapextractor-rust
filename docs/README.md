# Documentation Index

Start here: [**BIN_FORMAT.md**](BIN_FORMAT.md) — the reverse-engineered
layout of a War Thunder `.bin` level container. Inspect reports referenced
in docs are static examples committed for format analysis.

## Reference

| Document | Description |
|----------|-------------|
| [BIN_FORMAT.md](BIN_FORMAT.md) | **DBLD container** — known block tags, inline structures (`lndm`, `LTdump`, cell prefix, DDSx), per-map findings |
| [PIPELINE_OVERVIEW.md](PIPELINE_OVERVIEW.md) | End-to-end view of the Rust extraction pipeline and the viewer data it produces |
| [BUILD_PIPELINE.md](BUILD_PIPELINE.md) | 8-step extraction pipeline — textures, DxP materials, colormap, heightmap, tiles, landclass, terrain compositing, rendinst |
| [HM2_REFERENCE.md](HM2_REFERENCE.md) | HM2 CompressedHeightmap — decompression, dual-output approach, viewer integration |
| [CDK_KNOWLEDGE.md](CDK_KNOWLEDGE.md) | Dagor CDK findings — tank battle zones, ground level fallback, micro-detail normals |
| [ORIENTATION.md](ORIENTATION.md) | Canonical 1:1 orientation convention for pixel/cell/world frames |
| [KNOWN_ISSUES.md](KNOWN_ISSUES.md) | Fixed and open issues |

## Tools

- `wt-map-extractor` — main pipeline: `cargo run --release -- <map_name>`.
- Standalone in-game-map renderer (separate, gitignored crate at
  `ingame_map/`). Reads an already-extracted `maps/<name>/` directory
  and writes `ingame_map.png` with a 10×10 grid and world-metre scale
  bar. Example:

  ```
  cargo run --manifest-path ingame_map/Cargo.toml --release -- avg_vietnam_hills
  ```
