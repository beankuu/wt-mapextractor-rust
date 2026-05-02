//! Minimal standalone tool that renders an in-game-style 2D map overview
//! from an already-extracted viewer_data directory.
//!
//! Output layout mirrors the game's tactical map screenshot:
//!   * square terrain preview with an N×N grid (default 10×10),
//!   * column numbers 1..N across the top,
//!   * row letters A..(A+N-1) down the left side,
//!   * a scale bar in the lower-right corner (world metres).
//!
//! Usage:
//!   wt-ingame-map <map_name> [--grid 10] [--size 1024] [--type main|heightmap|battle] [--mission N] [--out path]
//!
//! By default reads `maps/<map_name>/terrain_paint.jpg` (or `.png`) and
//! `manifest.json`, writes `ingame_map/<map_name>_<type>.png`.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use image::{imageops, Rgb, RgbImage};
use serde_json::Value;

struct Opts {
    map_name: String,
    grid: u32,
    size: u32,
    map_type: MapType,
    out: Option<PathBuf>,
    probe: Vec<(f64, f64)>,
    mission: MissionSelection,
    list_missions: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissionSelection {
    Interactive,
    Index(usize),
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MapType {
    Main,
    Heightmap,
    Battle,
}

impl MapType {
    fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "main" | "map" | "terrain" => Ok(Self::Main),
            "heightmap" | "height" => Ok(Self::Heightmap),
            "battle" | "battlezone" | "battle-zone" => Ok(Self::Battle),
            _ => Err(format!("unknown --type {s}; expected main, heightmap, or battle")),
        }
    }
}

fn print_usage() {
    eprintln!(
        "wt-ingame-map <map_name> [--grid N] [--size PX] [--type main|heightmap|battle] [--mission N] [--out PATH] [--probe X,Z ...]\n\
         \n\
         Renders an in-game-style tactical map from maps/<name>/ output images\n\
         with an overlaid N×N grid (default 10), ocean/object overlays, and a world-metre scale bar.\n\
         --type battle crops to tankZone when present and can draw a selected mission overlay.\n\
         --mission N    Draw mission N from the printed 1-based mission list.\n\
         --no-mission   Do not draw mission battle/capture/spawn overlays.\n\
         --list-missions Print available missions and exit.\n\
         \n\
         --probe X,Z    Print the heightmap and detail-heightmap values at\n\
                        world (X, Z) in metres; may be repeated. Skips render."
    );
}

fn parse_args() -> Result<Opts, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        return Err(String::new());
    }
    let mut grid = 10u32;
    let mut size = 1024u32;
    let mut map_type = MapType::Battle;
    let mut out: Option<PathBuf> = None;
    let mut probe: Vec<(f64, f64)> = Vec::new();
    let mut mission = MissionSelection::Interactive;
    let mut list_missions = false;
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--grid" => {
                i += 1;
                grid = args
                    .get(i)
                    .ok_or_else(|| "--grid needs a value".to_string())?
                    .parse()
                    .map_err(|e| format!("--grid: {e}"))?;
            }
            "--size" => {
                i += 1;
                size = args
                    .get(i)
                    .ok_or_else(|| "--size needs a value".to_string())?
                    .parse()
                    .map_err(|e| format!("--size: {e}"))?;
            }
            "--type" => {
                i += 1;
                map_type = MapType::parse(
                    args.get(i).ok_or_else(|| "--type needs a value".to_string())?,
                )?;
            }
            "--out" => {
                i += 1;
                out = Some(PathBuf::from(
                    args.get(i).ok_or_else(|| "--out needs a value".to_string())?,
                ));
            }
            "--mission" => {
                i += 1;
                let n: usize = args
                    .get(i)
                    .ok_or_else(|| "--mission needs a 1-based mission number".to_string())?
                    .parse()
                    .map_err(|e| format!("--mission: {e}"))?;
                if n == 0 {
                    return Err("--mission uses 1-based mission numbers".to_string());
                }
                mission = MissionSelection::Index(n - 1);
            }
            "--no-mission" => {
                mission = MissionSelection::None;
            }
            "--list-missions" => {
                list_missions = true;
            }
            "--probe" => {
                i += 1;
                let s = args
                    .get(i)
                    .ok_or_else(|| "--probe needs X,Z".to_string())?;
                let mut parts = s.split(',');
                let x: f64 = parts
                    .next()
                    .ok_or_else(|| "--probe: missing X".to_string())?
                    .trim()
                    .parse()
                    .map_err(|e| format!("--probe X: {e}"))?;
                let z: f64 = parts
                    .next()
                    .ok_or_else(|| "--probe: missing Z".to_string())?
                    .trim()
                    .parse()
                    .map_err(|e| format!("--probe Z: {e}"))?;
                probe.push((x, z));
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    let map_name = positional.into_iter().next().ok_or_else(|| {
        "missing <map_name>. Example: wt-ingame-map avg_vietnam_hills".to_string()
    })?;
    Ok(Opts { map_name, grid, size, map_type, out, probe, mission, list_missions })
}

#[derive(Clone, Copy)]
struct WorldRect {
    x0: f64,
    z0: f64,
    x1: f64,
    z1: f64,
}

impl WorldRect {
    fn new(x0: f64, z0: f64, x1: f64, z1: f64) -> Self {
        Self { x0: x0.min(x1), z0: z0.min(z1), x1: x0.max(x1), z1: z0.max(z1) }
    }

    fn width(self) -> f64 { (self.x1 - self.x0).abs().max(1e-6) }
    fn height(self) -> f64 { (self.z1 - self.z0).abs().max(1e-6) }
}

fn load_terrain_paint(viewer_dir: &Path) -> Result<RgbImage, String> {
    for ext in ["jpg", "png"] {
        let p = viewer_dir.join(format!("terrain_paint.{ext}"));
        if p.exists() {
            return image::open(&p)
                .map(|i| i.to_rgb8())
                .map_err(|e| format!("failed to decode {}: {e}", p.display()));
        }
    }
    Err(format!(
        "no terrain_paint.{{jpg,png}} under {}",
        viewer_dir.display()
    ))
}

fn load_main_map(viewer_dir: &Path) -> Result<RgbImage, String> {
    for name in ["terrain_paint.jpg", "terrain_paint.png", "colormap.jpg", "colormap.png", "tile_grid.png"] {
        let p = viewer_dir.join(name);
        if p.exists() {
            return image::open(&p)
                .map(|i| i.to_rgb8())
                .map_err(|e| format!("failed to decode {}: {e}", p.display()));
        }
    }
    load_terrain_paint(viewer_dir)
}

fn load_heightmap_map(viewer_dir: &Path, manifest: Option<&Value>) -> Result<RgbImage, String> {
    let file = manifest
        .and_then(|m| m.get("heightmap"))
        .and_then(|h| h.get("file"))
        .and_then(Value::as_str)
        .unwrap_or("heightmap.png");
    let p = viewer_dir.join(file);
    image::open(&p)
        .map(|i| i.to_rgb8())
        .map_err(|e| format!("failed to decode {}: {e}", p.display()))
}

fn world_extent_m(manifest: Option<&Value>) -> f64 {
    manifest
        .and_then(|m| m.get("heightmap"))
        .and_then(|h| h.get("world_extent"))
        .and_then(|v| v.as_array())
        .and_then(|a| {
            if a.len() == 4 {
                let x0 = a[0].as_f64()?;
                let x1 = a[2].as_f64()?;
                Some((x1 - x0).abs())
            } else {
                None
            }
        })
        .unwrap_or(0.0)
}

/// Map world extent (x0,z0,x1,z1) using manifest.heightmap.world_extent.
/// Returns None if not available.
fn world_extent_rect(manifest: Option<&Value>) -> Option<(f64, f64, f64, f64)> {
    let a = manifest?
        .get("heightmap")?
        .get("world_extent")?
        .as_array()?;
    if a.len() != 4 {
        return None;
    }
    Some((a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?, a[3].as_f64()?))
}

fn tile_grid_rect(manifest: Option<&Value>) -> Option<(f64, f64, f64, f64)> {
    let a = manifest?
        .get("tileGrid")?
        .get("world_extent")?
        .as_array()?;
    if a.len() != 4 {
        return None;
    }
    Some((a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?, a[3].as_f64()?))
}

fn source_extent_rect(manifest: Option<&Value>, map_type: MapType) -> Option<(f64, f64, f64, f64)> {
    match map_type {
        MapType::Heightmap => world_extent_rect(manifest),
        MapType::Main | MapType::Battle => tile_grid_rect(manifest).or_else(|| world_extent_rect(manifest)),
    }
}

fn tank_zone_rect(manifest: Option<&Value>) -> Option<(f64, f64, f64, f64)> {
    let tz = manifest?.get("tankZone")?;
    let c0 = tz.get("coord0")?.as_array()?;
    let c1 = tz.get("coord1")?.as_array()?;
    if c0.len() < 2 || c1.len() < 2 {
        return None;
    }
    Some((c0[0].as_f64()?, c0[1].as_f64()?, c1[0].as_f64()?, c1[1].as_f64()?))
}

fn norm_rect((x0, z0, x1, z1): (f64, f64, f64, f64)) -> WorldRect {
    WorldRect::new(x0, z0, x1, z1)
}

fn world_to_img(view: WorldRect, margin: u32, thumb: u32, wx: f64, wz: f64) -> (i32, i32) {
    let x = margin as f64 + ((wx - view.x0) / view.width()) * thumb as f64;
    let y = margin as f64 + thumb as f64 - ((wz - view.z0) / view.height()) * thumb as f64;
    (x.round() as i32, y.round() as i32)
}

fn put_pixel_safe(img: &mut RgbImage, x: i32, y: i32, c: Rgb<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, c);
    }
}

fn blend_pixel_safe(img: &mut RgbImage, x: i32, y: i32, c: Rgb<u8>, alpha: f32) {
    if x < 0 || y < 0 || (x as u32) >= img.width() || (y as u32) >= img.height() {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    let a = alpha.clamp(0.0, 1.0);
    for i in 0..3 {
        p[i] = ((p[i] as f32 * (1.0 - a)) + (c[i] as f32 * a)).round().clamp(0.0, 255.0) as u8;
    }
}

fn fill_circle(img: &mut RgbImage, cx: i32, cy: i32, r: i32, c: Rgb<u8>, alpha: f32) {
    let rr = r.max(1);
    let r2 = rr * rr;
    for y in (cy - rr)..=(cy + rr) {
        for x in (cx - rr)..=(cx + rr) {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy <= r2 {
                blend_pixel_safe(img, x, y, c, alpha);
            }
        }
    }
}

fn stroke_circle(img: &mut RgbImage, cx: i32, cy: i32, r: i32, c: Rgb<u8>) {
    let rr = r.max(1);
    let inner = (rr - 2).max(0);
    let r2 = rr * rr;
    let i2 = inner * inner;
    for y in (cy - rr)..=(cy + rr) {
        for x in (cx - rr)..=(cx + rr) {
            let dx = x - cx;
            let dy = y - cy;
            let d2 = dx * dx + dy * dy;
            if d2 <= r2 && d2 >= i2 {
                put_pixel_safe(img, x, y, c);
            }
        }
    }
}

fn load_json(path: &Path) -> Option<Value> {
    fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok())
}

fn missions<'a>(missions_json: &'a Value) -> &'a [Value] {
    missions_json
        .get("missions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn mission_label(mission: &Value, idx: usize) -> String {
    mission
        .get("label")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("Mission {}", idx + 1))
}

fn mode_for(items: &[Value]) -> &'static str {
    if items.iter().any(|x| x.get("mode").and_then(Value::as_str) == Some("arcade")) {
        "arcade"
    } else if items.iter().any(|x| x.get("mode").and_then(Value::as_str) == Some("hardcore")) {
        "hardcore"
    } else {
        "arcade"
    }
}

fn array_items<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn count_mode(items: &[Value]) -> usize {
    let mode = mode_for(items);
    items.iter().filter(|x| x.get("mode").and_then(Value::as_str) == Some(mode)).count()
}

fn print_mission_list(missions_json: &Value) {
    let list = missions(missions_json);
    if list.is_empty() {
        println!("No missions found.");
        return;
    }
    println!("Available missions:");
    for (i, mission) in list.iter().enumerate() {
        let caps = count_mode(array_items(mission, "captureZones"));
        let spawns = count_mode(array_items(mission, "spawns")) + count_mode(array_items(mission, "spawnZones"));
        let battles = count_mode(array_items(mission, "battleAreas"));
        println!("  {}. {}  ({} battle, {} cap, {} spawn)", i + 1, mission_label(mission, i), battles, caps, spawns);
    }
}

fn choose_mission(opts: &Opts, missions_json: Option<&Value>) -> Result<Option<usize>, String> {
    let Some(missions_json) = missions_json else { return Ok(None); };
    let list = missions(missions_json);
    if list.is_empty() || opts.map_type != MapType::Battle {
        return Ok(None);
    }
    match opts.mission {
        MissionSelection::None => Ok(None),
        MissionSelection::Index(idx) => {
            if idx < list.len() { Ok(Some(idx)) } else { Err(format!("mission {} is out of range; found {} missions", idx + 1, list.len())) }
        }
        MissionSelection::Interactive => {
            print_mission_list(missions_json);
            print!("Pick mission number to draw, or press Enter for none: ");
            io::stdout().flush().map_err(|e| format!("failed to flush stdout: {e}"))?;
            let mut line = String::new();
            io::stdin().read_line(&mut line).map_err(|e| format!("failed to read mission selection: {e}"))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let n: usize = trimmed.parse().map_err(|e| format!("mission selection: {e}"))?;
            if n == 0 || n > list.len() {
                return Err(format!("mission {n} is out of range; found {} missions", list.len()));
            }
            Ok(Some(n - 1))
        }
    }
}

fn selected_mission_battle_extent(missions_json: Option<&Value>, mission_idx: Option<usize>) -> Option<WorldRect> {
    let idx = mission_idx?;
    let mission = missions_json.and_then(|m| missions(m).get(idx))?;

    let mut min_x = f64::INFINITY;
    let mut min_z = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_z = f64::NEG_INFINITY;
    let mut push = |x0: f64, z0: f64, x1: f64, z1: f64| {
        min_x = min_x.min(x0.min(x1));
        min_z = min_z.min(z0.min(z1));
        max_x = max_x.max(x0.max(x1));
        max_z = max_z.max(z0.max(z1));
    };

    let battle_mode = mode_for(array_items(mission, "battleAreas"));
    for ba in array_items(mission, "battleAreas") {
        if ba.get("mode").and_then(Value::as_str) != Some(battle_mode) {
            continue;
        }
        let Some((x, z)) = pos2(ba) else { continue; };
        let Some(half) = ba.get("halfSize").and_then(Value::as_array) else { continue; };
        if half.len() < 2 {
            continue;
        }
        let hx = half[0].as_f64().unwrap_or(0.0).abs();
        let hz = half[1].as_f64().unwrap_or(0.0).abs();
        push(x - hx, z - hz, x + hx, z + hz);
    }

    if min_x.is_finite() && min_z.is_finite() && max_x.is_finite() && max_z.is_finite() {
        Some(WorldRect::new(min_x, min_z, max_x, max_z))
    } else {
        None
    }
}

fn mission_extent(missions_json: Option<&Value>, mission_idx: Option<usize>) -> Option<WorldRect> {
    let idx = mission_idx?;
    let mission = missions_json.and_then(|m| missions(m).get(idx))?;

    let mut min_x = f64::INFINITY;
    let mut min_z = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_z = f64::NEG_INFINITY;
    let mut push = |x0: f64, z0: f64, x1: f64, z1: f64| {
        min_x = min_x.min(x0.min(x1));
        min_z = min_z.min(z0.min(z1));
        max_x = max_x.max(x0.max(x1));
        max_z = max_z.max(z0.max(z1));
    };

    if let Some(rect) = selected_mission_battle_extent(missions_json, mission_idx) {
        push(rect.x0, rect.z0, rect.x1, rect.z1);
    }

    let cap_mode = mode_for(array_items(mission, "captureZones"));
    for cap in array_items(mission, "captureZones") {
        if cap.get("mode").and_then(Value::as_str) != Some(cap_mode) {
            continue;
        }
        let Some((x, z)) = pos2(cap) else { continue; };
        let r = cap.get("radius").and_then(Value::as_f64).unwrap_or(30.0).abs().max(30.0);
        push(x - r, z - r, x + r, z + r);
    }

    let spawn_mode = mode_for(array_items(mission, "spawns"));
    for sp in array_items(mission, "spawns") {
        if sp.get("mode").and_then(Value::as_str) != Some(spawn_mode) {
            continue;
        }
        let Some((x, z)) = pos2(sp) else { continue; };
        push(x, z, x, z);
    }

    let spawn_zone_mode = mode_for(array_items(mission, "spawnZones"));
    for sp in array_items(mission, "spawnZones") {
        if sp.get("mode").and_then(Value::as_str) != Some(spawn_zone_mode) {
            continue;
        }
        let Some((x, z)) = pos2(sp) else { continue; };
        push(x, z, x, z);
    }

    if min_x.is_finite() && min_z.is_finite() && max_x.is_finite() && max_z.is_finite() {
        Some(WorldRect::new(min_x, min_z, max_x, max_z))
    } else {
        None
    }
}

fn format_grid_meters(cell_m: f64) -> String {
    if !cell_m.is_finite() || cell_m <= 0.0 {
        return "-- M".to_string();
    }
    if cell_m >= 1000.0 {
        format!("{:.2} KM", cell_m / 1000.0)
    } else if cell_m >= 100.0 {
        format!("{} M", cell_m.round() as i64)
    } else {
        format!("{:.1} M", cell_m)
    }
}

fn pos2(v: &Value) -> Option<(f64, f64)> {
    let a = v.get("pos")?.as_array()?;
    if a.len() < 2 { return None; }
    Some((a[0].as_f64()?, a[1].as_f64()?))
}

fn draw_line(img: &mut RgbImage, x0: u32, y0: u32, x1: u32, y1: u32, c: Rgb<u8>) {
    let (w, h) = img.dimensions();
    if x0 == x1 {
        let x = x0.min(w - 1);
        let (ya, yb) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        for y in ya..=yb.min(h - 1) {
            img.put_pixel(x, y, c);
        }
    } else if y0 == y1 {
        let y = y0.min(h - 1);
        let (xa, xb) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        for x in xa..=xb.min(w - 1) {
            img.put_pixel(x, y, c);
        }
    }
}

fn draw_rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Rgb<u8>, fill: bool) {
    let (iw, ih) = img.dimensions();
    let x1 = (x + w).min(iw);
    let y1 = (y + h).min(ih);
    if fill {
        for yy in y..y1 {
            for xx in x..x1 {
                img.put_pixel(xx, yy, c);
            }
        }
    } else {
        draw_line(img, x, y, x1.saturating_sub(1), y, c);
        draw_line(img, x, y1.saturating_sub(1), x1.saturating_sub(1), y1.saturating_sub(1), c);
        draw_line(img, x, y, x, y1.saturating_sub(1), c);
        draw_line(img, x1.saturating_sub(1), y, x1.saturating_sub(1), y1.saturating_sub(1), c);
    }
}

/// Extremely small 5×7 bitmap font covering digits 0..9 and uppercase A..Z.
/// One byte per row, high bit = leftmost column. Only what we need on the grid
/// labels is declared.
fn glyph(ch: char) -> Option<[u8; 7]> {
    Some(match ch.to_ascii_uppercase() {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0E],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        _ => return None,
    })
}

fn draw_text(img: &mut RgbImage, x: u32, y: u32, text: &str, scale: u32, c: Rgb<u8>) {
    let mut cx = x;
    for ch in text.chars() {
        if ch == ' ' {
            cx += 4 * scale;
            continue;
        }
        if let Some(g) = glyph(ch) {
            for (row, bits) in g.iter().enumerate() {
                for col in 0..5u32 {
                    if (bits >> (4 - col)) & 1 == 1 {
                        let px = cx + col * scale;
                        let py = y + row as u32 * scale;
                        for dy in 0..scale {
                            for dx in 0..scale {
                                if px + dx < img.width() && py + dy < img.height() {
                                    img.put_pixel(px + dx, py + dy, c);
                                }
                            }
                        }
                    }
                }
            }
        }
        cx += 6 * scale;
    }
}

fn row_label(row: u32) -> String {
    // A, B, ... Z, AA, AB ... — only up to ZZ matters here.
    if row < 26 {
        char::from(b'A' + row as u8).to_string()
    } else {
        let a = (row / 26) - 1;
        let b = row % 26;
        format!(
            "{}{}",
            char::from(b'A' + a as u8),
            char::from(b'A' + b as u8)
        )
    }
}

fn draw_ocean_overlay(img: &mut RgbImage, map_dir: &Path, manifest: Option<&Value>, view: WorldRect, margin: u32, thumb: u32) {
    let Some(manifest) = manifest else { return; };
    let Some(water_level) = manifest.get("waterLevel").and_then(Value::as_f64) else { return; };
    let Some(hm) = manifest.get("heightmap") else { return; };
    let Some(hm_rect) = world_extent_rect(Some(manifest)).map(norm_rect) else { return; };
    let h_min = hm.get("height_min_m").and_then(Value::as_f64).unwrap_or(0.0);
    let h_max = hm.get("height_max_m").and_then(Value::as_f64).unwrap_or(0.0);
    if h_max <= h_min {
        return;
    }
    let file = hm.get("file").and_then(Value::as_str).unwrap_or("heightmap.png");
    let Ok(height_img) = image::open(map_dir.join(file)).map(|i| i.to_luma16()) else { return; };
    let (hw, hh) = height_img.dimensions();
    let threshold = water_level - ((h_max - h_min) / 65535.0);
    let water_color = Rgb([18, 86, 145]);

    for y in 0..thumb {
        let wz = view.z1 - ((y as f64 + 0.5) / thumb as f64) * view.height();
        if wz < hm_rect.z0 || wz > hm_rect.z1 {
            continue;
        }
        let py = (((wz - hm_rect.z0) / hm_rect.height()) * (hh - 1) as f64)
            .round()
            .clamp(0.0, (hh - 1) as f64) as u32;
        for x in 0..thumb {
            let wx = view.x0 + ((x as f64 + 0.5) / thumb as f64) * view.width();
            if wx < hm_rect.x0 || wx > hm_rect.x1 {
                continue;
            }
            let px = (((wx - hm_rect.x0) / hm_rect.width()) * (hw - 1) as f64)
                .round()
                .clamp(0.0, (hw - 1) as f64) as u32;
            let v = height_img.get_pixel(px, py)[0] as f64;
            let h = h_min + (v / 65535.0) * (h_max - h_min);
            if h <= threshold {
                blend_pixel_safe(img, margin as i32 + x as i32, margin as i32 + y as i32, water_color, 0.45);
            }
        }
    }
}

fn rendinst_style(pool: Option<&Value>) -> String {
    let cat = pool.and_then(|p| p.get("category")).and_then(Value::as_str).unwrap_or("other");
    let name = pool.and_then(|p| p.get("name")).and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
    let building_terms = ["container", "warehouse", "hangar", "building", "house", "shed", "garage", "barrack", "port_crane", "crane", "terminal", "tower", "bunker", "fort", "wall", "bridge", "pillbox", "blockhouse", "castle", "church", "mosque", "factory", "plant", "station", "depot", "yard", "dock", "pier"];
    if building_terms.iter().any(|term| name.contains(term)) {
        return "building".to_string();
    }
    let road_terms = ["road", "street", "highway", "asphalt", "pavement", "runway", "taxiway", "track", "path"];
    if road_terms.iter().any(|term| name.contains(term)) {
        return "road".to_string();
    }
    if cat == "debris" {
        if ["trench", "ditch", "foxhole", "earthwork", "embankment"].iter().any(|term| name.contains(term)) {
            return "earthwork".to_string();
        }
        if ["wreck", "ruin", "rubble", "debris", "hedgehog", "barbed"].iter().any(|term| name.contains(term)) {
            return "debris".to_string();
        }
    }
    cat.to_string()
}

fn object_color(style: &str) -> Rgb<u8> {
    match style {
        "tree" => Rgb([35, 130, 55]),
        "bush" | "vegetation" => Rgb([100, 150, 58]),
        "building" => Rgb([185, 188, 192]),
        "road" => Rgb([54, 54, 54]),
        "rock" => Rgb([126, 134, 142]),
        "debris" => Rgb([150, 58, 48]),
        "earthwork" => Rgb([130, 85, 50]),
        _ => Rgb([220, 220, 220]),
    }
}

fn draw_objects_overlay(img: &mut RgbImage, map_dir: &Path, manifest: Option<&Value>, view: WorldRect, margin: u32, thumb: u32) {
    let Some(ri) = manifest.and_then(|m| m.get("rendinst")) else { return; };
    let file = ri.get("file").and_then(Value::as_str).unwrap_or("rendinst.bin");
    let Ok(bytes) = fs::read(map_dir.join(file)) else { return; };
    let stride = if ri.get("stride").and_then(Value::as_u64) == Some(14) { 14usize } else { 10usize };
    let count = ri.get("instanceCount").and_then(Value::as_u64).unwrap_or((bytes.len() / stride) as u64) as usize;
    let pools = ri.get("pools").and_then(Value::as_array);
    let dot = (thumb / 900).max(1) as i32;

    for i in 0..count.min(bytes.len() / stride) {
        let off = i * stride;
        let pool_idx = u16::from_le_bytes([bytes[off], bytes[off + 1]]) as usize;
        let wx = f32::from_le_bytes([bytes[off + 2], bytes[off + 3], bytes[off + 4], bytes[off + 5]]) as f64;
        let wz_off = if stride == 14 { off + 10 } else { off + 6 };
        let wz = f32::from_le_bytes([bytes[wz_off], bytes[wz_off + 1], bytes[wz_off + 2], bytes[wz_off + 3]]) as f64;
        if wx < view.x0 || wx > view.x1 || wz < view.z0 || wz > view.z1 {
            continue;
        }
        let pool = pools.and_then(|p| p.get(pool_idx));
        let color = object_color(&rendinst_style(pool));
        let (x, y) = world_to_img(view, margin, thumb, wx, wz);
        for dy in 0..dot {
            for dx in 0..dot {
                blend_pixel_safe(img, x + dx, y + dy, color, 0.65);
            }
        }
    }
}

fn draw_mission_overlay(img: &mut RgbImage, missions_json: Option<&Value>, mission_idx: Option<usize>, view: WorldRect, margin: u32, thumb: u32) {
    let Some(idx) = mission_idx else { return; };
    let Some(mission) = missions_json.and_then(|m| missions(m).get(idx)) else { return; };
    let capture_mode = mode_for(array_items(mission, "captureZones"));
    let spawn_mode = mode_for(array_items(mission, "spawns"));
    let spawn_zone_mode = mode_for(array_items(mission, "spawnZones"));
    let cap_color = Rgb([255, 196, 0]);
    let team1 = Rgb([0, 162, 255]);
    let team2 = Rgb([255, 43, 43]);
    let neutral = Rgb([136, 255, 136]);

    for cap in array_items(mission, "captureZones") {
        if cap.get("mode").and_then(Value::as_str) != Some(capture_mode) {
            continue;
        }
        let Some((x, z)) = pos2(cap) else { continue; };
        let (px, py) = world_to_img(view, margin, thumb, x, z);
        let radius_m = cap.get("radius").and_then(Value::as_f64).unwrap_or(1.0).max(12.0);
        let radius_px = ((radius_m / view.width()) * thumb as f64).round().max(4.0) as i32;
        fill_circle(img, px, py, radius_px, cap_color, 0.32);
        stroke_circle(img, px, py, radius_px, cap_color);
        if let Some(label) = cap.get("label").and_then(Value::as_str) {
            draw_text(img, (px - 3).max(0) as u32, (py - 4).max(0) as u32, label, (thumb / 400).max(1), cap_color);
        }
    }

    let mut draw_spawn = |item: &Value, large: bool| {
        let Some((x, z)) = pos2(item) else { return; };
        let (px, py) = world_to_img(view, margin, thumb, x, z);
        let color = match item.get("team").and_then(Value::as_i64) {
            Some(1) => team1,
            Some(2) => team2,
            _ => neutral,
        };
        let r = if large { (thumb / 120).max(4) } else { (thumb / 180).max(3) } as i32;
        fill_circle(img, px, py, r, color, 0.95);
        stroke_circle(img, px, py, r + 1, Rgb([20, 20, 20]));
    };
    for sp in array_items(mission, "spawns") {
        if sp.get("mode").and_then(Value::as_str) == Some(spawn_mode) {
            draw_spawn(sp, true);
        }
    }
    for sp in array_items(mission, "spawnZones") {
        if sp.get("mode").and_then(Value::as_str) == Some(spawn_zone_mode) {
            draw_spawn(sp, false);
        }
    }
}

fn render(opts: &Opts) -> Result<PathBuf, String> {
    let map_dir = PathBuf::from("maps").join(&opts.map_name);
    if !map_dir.is_dir() {
        return Err(format!(
            "map directory not found: {}. Run the main extractor first.",
            map_dir.display()
        ));
    }

    let manifest = load_json(&map_dir.join("manifest.json"));
    let missions_json = load_json(&map_dir.join("missions.json"));
    if opts.list_missions {
        if let Some(missions_json) = missions_json.as_ref() {
            print_mission_list(missions_json);
        } else {
            println!("No missions.json found for {}", opts.map_name);
        }
        return Ok(PathBuf::new());
    }
    let mission_idx = choose_mission(opts, missions_json.as_ref())?;

    let terrain_full = match opts.map_type {
        MapType::Heightmap => load_heightmap_map(&map_dir, manifest.as_ref())?,
        MapType::Main | MapType::Battle => load_main_map(&map_dir)?,
    };

    // Type-specific crop: battle areas are authoritative; tankZone is only a fallback.
    let crop_rect = match opts.map_type {
        MapType::Battle => selected_mission_battle_extent(missions_json.as_ref(), mission_idx)
            .or_else(|| tank_zone_rect(manifest.as_ref()).map(norm_rect))
            .or_else(|| mission_extent(missions_json.as_ref(), mission_idx)),
        MapType::Main | MapType::Heightmap => None,
    };
    let source_extent = source_extent_rect(manifest.as_ref(), opts.map_type).map(norm_rect);
    let (terrain, view_rect) = match (source_extent, crop_rect) {
        (Some(source_rect), Some(detail_rect)) => {
            let tw = terrain_full.width() as f64;
            let th = terrain_full.height() as f64;
            let world_w = source_rect.width();
            let world_h = source_rect.height();
            let view = WorldRect::new(
                detail_rect.x0.max(source_rect.x0),
                detail_rect.z0.max(source_rect.z0),
                detail_rect.x1.min(source_rect.x1),
                detail_rect.z1.min(source_rect.z1),
            );
            // terrain_paint.png follows ORIENTATION.md §2: pixel (0,0) = world
            // SW corner, (W-1, H-1) = world NE. Map world extent to pixels.
            let px0 = (((view.x0 - source_rect.x0) / world_w) * tw).clamp(0.0, tw) as u32;
            let px1 = (((view.x1 - source_rect.x0) / world_w) * tw).clamp(0.0, tw) as u32;
            let pz0 = (((view.z0 - source_rect.z0) / world_h) * th).clamp(0.0, th) as u32;
            let pz1 = (((view.z1 - source_rect.z0) / world_h) * th).clamp(0.0, th) as u32;
            let cw = px1.saturating_sub(px0);
            let ch = pz1.saturating_sub(pz0);
            if cw > 32 && ch > 32 {
                let cropped = imageops::crop_imm(&terrain_full, px0, pz0, cw, ch).to_image();
                (cropped, view)
            } else {
                (terrain_full, source_rect)
            }
        }
        _ => {
            let view = source_extent.unwrap_or_else(|| {
                let c0 = manifest.as_ref().and_then(|m| m.get("mapCoord0")).and_then(Value::as_array);
                let c1 = manifest.as_ref().and_then(|m| m.get("mapCoord1")).and_then(Value::as_array);
                match (c0, c1) {
                    (Some(a), Some(b)) if a.len() >= 2 && b.len() >= 2 => WorldRect::new(
                        a[0].as_f64().unwrap_or(0.0),
                        a[1].as_f64().unwrap_or(0.0),
                        b[0].as_f64().unwrap_or(1.0),
                        b[1].as_f64().unwrap_or(1.0),
                    ),
                    _ => WorldRect::new(0.0, 0.0, world_extent_m(manifest.as_ref()).max(1.0), world_extent_m(manifest.as_ref()).max(1.0)),
                }
            });
            (terrain_full, view)
        }
    };
    let world_m = view_rect.width();

    let grid = opts.grid.max(1);
    let size = opts.size.max(128);
    let margin = (size / 28).max(18);
    let inner = size - margin;

    // Canvas initialised with the in-game tactical-map charcoal background.
    let mut img: RgbImage = RgbImage::from_pixel(size, size, Rgb([11, 18, 28]));

    // Terrain thumbnail, flipped so row 0 = world north (matches §ORIENTATION
    // exception for 2D displays).
    let thumb_w = inner - margin;
    let thumb_h = thumb_w;
    let mut thumb = imageops::resize(&terrain, thumb_w, thumb_h, imageops::FilterType::Lanczos3);
    imageops::flip_vertical_in_place(&mut thumb);
    image::imageops::replace(&mut img, &thumb, margin as i64, margin as i64);

    draw_ocean_overlay(&mut img, &map_dir, manifest.as_ref(), view_rect, margin, thumb_w);
    draw_objects_overlay(&mut img, &map_dir, manifest.as_ref(), view_rect, margin, thumb_w);
    draw_mission_overlay(&mut img, missions_json.as_ref(), mission_idx, view_rect, margin, thumb_w);

    // Grid overlay (medium grey, less harsh than near-white).
    let grid_color = Rgb([130, 135, 145]);
    let label_color = Rgb([210, 215, 225]);
    for i in 0..=grid {
        let t = margin as f32 + (thumb_w as f32 * i as f32 / grid as f32);
        let xi = t.round() as u32;
        draw_line(&mut img, xi, margin, xi, margin + thumb_h, grid_color);
        let yi = (margin as f32 + thumb_h as f32 * i as f32 / grid as f32).round() as u32;
        draw_line(&mut img, margin, yi, margin + thumb_w, yi, grid_color);
    }

    // Column numbers across the top, row letters down the left side.
    let scale_font = (size / 220).max(1);
    let cell = thumb_w as f32 / grid as f32;
    for c in 0..grid {
        let lbl = format!("{}", c + 1);
        let cx = margin as f32 + cell * (c as f32 + 0.5) - 2.5 * scale_font as f32;
        draw_text(
            &mut img,
            cx.round() as u32,
            (margin as f32 / 2.0 - 3.5 * scale_font as f32)
                .max(2.0)
                .round() as u32,
            &lbl,
            scale_font,
            label_color,
        );
    }
    for r in 0..grid {
        let lbl = row_label(r);
        let cy = margin as f32 + cell * (r as f32 + 0.5) - 3.5 * scale_font as f32;
        draw_text(
            &mut img,
            (margin as f32 / 2.0 - 2.5 * scale_font as f32)
                .max(2.0)
                .round() as u32,
            cy.round() as u32,
            &lbl,
            scale_font,
            label_color,
        );
    }

    // Scale bar in the lower-right corner: one grid-cell worth of world metres.
    if world_m > 0.0 {
        let cell_m = world_m / grid as f64;
        let bar_px = (thumb_w as f64 / grid as f64).round().max(1.0) as u32;
        let txt = format_grid_meters(cell_m);
        let txt_w = (txt.chars().count() as u32) * 6 * scale_font;
        let bar_x = margin + thumb_w - bar_px - 8;
        let bar_y = margin + thumb_h + 6;
        if bar_y + 4 < size {
            draw_rect(&mut img, bar_x, bar_y, bar_px, 3, label_color, true);
            let text_x = (margin + thumb_w).saturating_sub(txt_w + 8);
            draw_text(&mut img, text_x, bar_y + 6, &txt, scale_font, label_color);
        }
    }

    let out_path = opts
        .out
        .clone()
        .unwrap_or_else(|| {
            let suffix = match opts.map_type {
                MapType::Main => "main",
                MapType::Heightmap => "heightmap",
                MapType::Battle => "battle",
            };
            PathBuf::from("ingame_map").join(format!("{}_{}.png", opts.map_name, suffix))
        });
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
        }
    }
    img.save(&out_path)
        .map_err(|e| format!("failed to save {}: {e}", out_path.display()))?;
    Ok(out_path)
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(v) => v,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("error: {msg}");
                return ExitCode::from(2);
            }
            return ExitCode::from(0);
        }
    };
    if !opts.probe.is_empty() {
        return match probe_heights(&opts) {
            Ok(()) => ExitCode::from(0),
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        };
    }
    match render(&opts) {
        Ok(p) => {
            if !p.as_os_str().is_empty() {
                println!("wrote {}", p.display());
            }
            ExitCode::from(0)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

fn sample_height(
    img: &image::ImageBuffer<image::Luma<u16>, Vec<u16>>,
    world_x: f64,
    world_z: f64,
    x0: f64,
    z0: f64,
    x1: f64,
    z1: f64,
    h_min: f64,
    h_max: f64,
) -> Option<(u32, u32, u16, f64)> {
    if world_x < x0 || world_x > x1 || world_z < z0 || world_z > z1 {
        return None;
    }
    let (w, h) = img.dimensions();
    let nx = (world_x - x0) / (x1 - x0);
    let nz = (world_z - z0) / (z1 - z0);
    let px = (nx * w as f64).clamp(0.0, (w - 1) as f64) as u32;
    let py = (nz * h as f64).clamp(0.0, (h - 1) as f64) as u32;
    let v = img.get_pixel(px, py)[0];
    let h_m = h_min + (v as f64 / 65535.0) * (h_max - h_min);
    Some((px, py, v, h_m))
}

fn probe_heights(opts: &Opts) -> Result<(), String> {
    let map_dir = PathBuf::from("maps").join(&opts.map_name);
    if !map_dir.is_dir() {
        return Err(format!("map directory not found: {}", map_dir.display()));
    }
    let manifest_raw = std::fs::read_to_string(map_dir.join("manifest.json"))
        .map_err(|e| format!("failed to read manifest.json: {e}"))?;
    let manifest: Value = serde_json::from_str(&manifest_raw)
        .map_err(|e| format!("invalid manifest.json: {e}"))?;

    let hm_meta = manifest
        .get("heightmap")
        .ok_or_else(|| "manifest missing heightmap block".to_string())?;
    let hm_path = map_dir.join(
        hm_meta
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or("heightmap.png"),
    );
    let ext = hm_meta
        .get("world_extent")
        .and_then(Value::as_array)
        .ok_or_else(|| "manifest heightmap missing world_extent".to_string())?;
    let (x0, z0, x1, z1) = (
        ext[0].as_f64().ok_or("world_extent[0]")?,
        ext[1].as_f64().ok_or("world_extent[1]")?,
        ext[2].as_f64().ok_or("world_extent[2]")?,
        ext[3].as_f64().ok_or("world_extent[3]")?,
    );
    let h_min = hm_meta.get("height_min_m").and_then(Value::as_f64).unwrap_or(0.0);
    let h_max = hm_meta.get("height_max_m").and_then(Value::as_f64).unwrap_or(0.0);
    let hm_img = image::open(&hm_path)
        .map_err(|e| format!("failed to decode {}: {e}", hm_path.display()))?
        .to_luma16();

    let detail = manifest.get("heightmapDetail").and_then(|d| {
        let file = d.get("file").and_then(Value::as_str)?;
        let dx0 = d.get("world_x0").and_then(Value::as_f64)?;
        let dx1 = d.get("world_x1").and_then(Value::as_f64)?;
        let dz0 = d.get("world_z0").and_then(Value::as_f64)?;
        let dz1 = d.get("world_z1").and_then(Value::as_f64)?;
        let dhmin = d.get("height_min_m").and_then(Value::as_f64)?;
        let dhmax = d.get("height_max_m").and_then(Value::as_f64)?;
        let img = image::open(map_dir.join(file)).ok()?.to_luma16();
        Some((img, dx0, dz0, dx1, dz1, dhmin, dhmax))
    });

    for &(wx, wz) in &opts.probe {
        println!("probe (x={wx}, z={wz}):");
        if let Some((px, py, v, hm)) =
            sample_height(&hm_img, wx, wz, x0, z0, x1, z1, h_min, h_max)
        {
            println!(
                "  heightmap.png      pixel=({px},{py}) u16={v}  height={hm:.2} m",
            );
        } else {
            println!("  heightmap.png      outside world_extent");
        }
        if let Some((img, dx0, dz0, dx1, dz1, dhmin, dhmax)) = detail.as_ref() {
            if let Some((px, py, v, hm)) =
                sample_height(img, wx, wz, *dx0, *dz0, *dx1, *dz1, *dhmin, *dhmax)
            {
                println!(
                    "  heightmap_detail   pixel=({px},{py}) u16={v}  height={hm:.2} m",
                );
            } else {
                println!("  heightmap_detail   outside detail extent");
            }
        }
    }
    Ok(())
}
