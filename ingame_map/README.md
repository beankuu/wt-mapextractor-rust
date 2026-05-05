# wt-ingame-map (standalone)

A small standalone tool that renders an in-game-style 2D tactical map
from an already-extracted `maps/<name>/` directory produced by the main
`wt-map-extractor` pipeline.

This tool is **not** part of the mainstream pipeline. It is built independently
so that it can evolve without touching the main extractor.

## Build and run

From the project root:

```powershell
cargo run --manifest-path ingame_map/Cargo.toml --release -- <map_name>
```

Examples:

```powershell
# Default 10×10 grid, 1024 px output under minimaps/
cargo run --manifest-path ingame_map/Cargo.toml --release -- avg_vietnam_hills

# Custom grid / size / output path
cargo run --manifest-path ingame_map/Cargo.toml --release -- avg_vietnam_hills --grid 12 --size 2048 --out custom.jpg

# Choose minimap source: main terrain, heightmap, or battle-zone crop
cargo run --manifest-path ingame_map/Cargo.toml --release -- avg_berlin --type battle
cargo run --manifest-path ingame_map/Cargo.toml --release -- air_afghan --type heightmap

# List battle/capture/spawn missions, then render one non-interactively
cargo run --manifest-path ingame_map/Cargo.toml --release -- avg_container_port --list-missions
cargo run --manifest-path ingame_map/Cargo.toml --release -- avg_container_port --mission 1

# Mission 0 disables mission overlays (same as --no-mission)
cargo run --manifest-path ingame_map/Cargo.toml --release -- avg_container_port --mission 0

# Render every mission battle zoom for one map (one output per mission)
cargo run --manifest-path ingame_map/Cargo.toml --release -- avg_container_port --all-battle

# Batch render all extracted maps
cargo run --manifest-path ingame_map/Cargo.toml --release -- --all --type main

# Probe heightmap values at specific world (X,Z) metres
cargo run --manifest-path ingame_map/Cargo.toml --release -- avg_vietnam_hills --probe 0,0 --probe 1500,-2000
```

## Inputs

Reads from `maps/<map_name>/`:

- `terrain_paint.{png,jpg}` — preferred terrain colour source for main/battle views
- `colormap.{png,jpg}` — next fallback
- `tile_grid.png` — final fallback when painted terrain and colormap are missing
- `manifest.json` — for `heightmap.world_extent`, `tankZone`, and optional
  `heightmapDetail` (HM2) sub-region cropping, `waterLevel`, and render
  instance metadata
- `missions.json` — optional mission battle areas, capture zones, and spawn
  points for `--type battle`
- `rendinst.bin` — optional object positions drawn directly onto the tactical
  map when present
- `heightmap.png` and optional `heightmap_detail.png` — only needed for
  ocean masking and `--probe`

## Output

By default writes compressed `minimaps/<map_name>_<type>.jpg`. Override with
`--out`; `.jpg`/`.jpeg` paths are JPEG-encoded, other extensions use the image
crate's matching encoder.

For battle maps with `missions.json`, the tool prints a numbered mission list
and asks which mission to draw. Use `--mission N` for scripts.

Use `--mission 0` or `--no-mission` to disable mission overlays. In
`--type battle`, this now renders the full map extent (not a forced tank-zone
crop), which is useful when you want a full-size minimap.

Use `--all-battle` to render every mission as a separate battle-zone-zoom
output file (for example `minimaps/<map>_battle_m01.jpg`,
`minimaps/<map>_battle_m02.jpg`, ...).
