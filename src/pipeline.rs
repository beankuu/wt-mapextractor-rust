use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use image::imageops::FilterType;
use rayon::prelude::*;
use rayon::ThreadPoolBuilder;
use serde_json::{json, Value};

use crate::config::AppConfig;
use crate::dxp_index::DxpIndex;
use crate::export::{export_materials, export_overview, export_tiles, DdsStore};
use crate::extract::{extract_ddsx_from_data, extract_dxp};
use crate::heightmap::build_heightmap_native;
use crate::landclass::extract_landclass_native;
use crate::missions::extract_missions_for_map;
use crate::paint::{build_terrain_paint_native, Hm2ExtentForPaint, WaterMaskParams};
use crate::post::{build_heightmap_fallback, build_terrain_paint_fallback, build_tile_grid, parse_lndm_grid, LndmGrid};
use crate::progress::Progress;
use crate::rendinst::extract_rendinst_native;

fn make_map_summary(name: &str, manifest: &Value) -> Value {
    let landclasses = manifest
        .get("landclasses")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    let materials = manifest
        .get("materials")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    json!({
        "name": name,
        "mapSize": manifest.get("mapSize"),
        "waterLevel": manifest.get("waterLevel"),
        "heightmap": manifest.get("heightmap"),
        "colormap": manifest.get("colormap"),
        "terrainPaint": manifest.get("terrainPaint"),
        "landclasses": landclasses,
        "materials": materials,
    })
}

fn upsert_maps_index(path: &Path, name: &str, manifest: &Value) -> Result<()> {
    let mut entries: Vec<Value> = if path.exists() {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
            .unwrap_or_default()
    } else {
        vec![]
    };
    let summary = make_map_summary(name, manifest);
    match entries.iter().position(|e| e.get("name").and_then(Value::as_str) == Some(name)) {
        Some(pos) => entries[pos] = summary,
        None => entries.push(summary),
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(&entries)?)?;
    Ok(())
}

fn write_debug_thumbs(viewer_dir: &Path) -> Result<()> {
    let thumbs_dir = viewer_dir.join("thumbs");
    fs::create_dir_all(&thumbs_dir)?;

    let make_thumb = |src_name: &str, dst_name: &str| {
        let src = viewer_dir.join(src_name);
        if !src.exists() {
            return;
        }
        if let Ok(img) = image::open(&src) {
            let tw = 256u32.min(img.width().max(1));
            let th = 256u32.min(img.height().max(1));
            let thumb = image::imageops::resize(&img.to_rgba8(), tw, th, FilterType::Lanczos3);
            let _ = thumb.save(thumbs_dir.join(dst_name));
        }
    };

    let terrain_paint_src = if viewer_dir.join("terrain_paint.jpg").exists() {
        "terrain_paint.jpg"
    } else {
        "terrain_paint.png"
    };
    make_thumb(terrain_paint_src, "terrain_paint_thumb.png");
    make_thumb("heightmap.png", "heightmap_thumb.png");
    if viewer_dir.join("colormap.jpg").exists() {
        make_thumb("colormap.jpg", "colormap_thumb.png");
    } else {
        make_thumb("colormap.png", "colormap_thumb.png");
    }
    make_thumb("tile_grid.png", "tile_grid_thumb.png");

    let mat_dir = viewer_dir.join("mat");
    if mat_dir.exists() {
        let mat_thumb_dir = thumbs_dir.join("mat");
        let _ = fs::create_dir_all(&mat_thumb_dir);
        if let Ok(rd) = fs::read_dir(&mat_dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or_default();
                if ext != "png" && ext != "webp" {
                    continue;
                }
                if let Ok(img) = image::open(&p) {
                    let tw = 96u32.min(img.width().max(1));
                    let th = 96u32.min(img.height().max(1));
                    let thumb = image::imageops::resize(&img.to_rgba8(), tw, th, FilterType::Triangle);
                    if let Some(name) = p.file_name() {
                        let _ = thumb.save(mat_thumb_dir.join(name));
                    }
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub export_mat: bool,
    pub export_thumbs: bool,
    pub clean_maps_on_all: bool,
    pub skip_tile_grid: bool,
    pub skip_rendinst: bool,
    pub compress_maps: bool,
    pub sun_azimuth: f64,
    pub sun_elevation: f64,
    pub sun_strength: f64,
}

#[derive(Debug)]
pub struct Pipeline {
    cfg: AppConfig,
    dxp_index: OnceLock<Arc<DxpIndex>>,
}

impl Pipeline {
    pub fn new(cfg: AppConfig) -> Self {
        Self { cfg, dxp_index: OnceLock::new() }
    }

    /// Lazily-built global DxP2 texture-name index covering every `*.dxp.bin`
    /// under the configured content roots. Scans name tables only; individual
    /// pack bodies are decoded on demand and cached inside the index.
    fn dxp_index(&self) -> &Arc<DxpIndex> {
        self.dxp_index.get_or_init(|| {
            let t0 = Instant::now();
            let roots: [&Path; 3] = [
                &self.cfg.hq_context_dir,
                &self.cfg.context_dir,
                &self.cfg.all_levels,
            ];
            let idx = DxpIndex::build(&roots);
            eprintln!(
                "  Built global DxP name index: {} textures ({} ms)",
                idx.len(),
                t0.elapsed().as_millis()
            );
            Arc::new(idx)
        })
    }

    fn cleanup_maps_dir(&self) -> Result<()> {
        let maps_root = self.cfg.project_root.join("maps");
        if !maps_root.exists() {
            fs::create_dir_all(&maps_root)?;
            return Ok(());
        }

        for entry in fs::read_dir(&maps_root)
            .with_context(|| format!("Failed to read {}", maps_root.display()))?
        {
            let path = entry?.path();
            if path.is_dir() {
                fs::remove_dir_all(&path)
                    .with_context(|| format!("Failed to remove {}", path.display()))?;
            } else {
                fs::remove_file(&path)
                    .with_context(|| format!("Failed to remove {}", path.display()))?;
            }
        }
        Ok(())
    }

    pub fn build_all(&self, opts: &BuildOptions) -> Result<()> {
        if opts.clean_maps_on_all {
            eprintln!("  Cleaning ./maps before --all build...");
            self.cleanup_maps_dir()?;
        }

        let mut maps = Vec::new();
        if self.cfg.all_levels.exists() {
            for entry in fs::read_dir(&self.cfg.all_levels)
                .with_context(|| format!("Failed to read {}", self.cfg.all_levels.display()))?
            {
                let path = entry?.path();
                if !path.is_file() {
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("bin") {
                    continue;
                }
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                if name.ends_with(".dxp.bin") {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    maps.push(stem.to_string());
                }
            }
        }

        maps.sort();
        if maps.is_empty() {
            return Err(anyhow!(
                "No .bin maps found in {}",
                self.cfg.all_levels.display()
            ));
        }

        let auto_workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .max(1);
        // Each worker can transiently hold ~1.5 GB and heavy image passes are
        // memory-bandwidth bound. Use about one worker per physical core on
        // common SMT CPUs; override with `WT_WORKERS=N` for throughput tests.
        let default_workers = (auto_workers / 2).max(1).min(8);
        let workers = std::env::var("WT_WORKERS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(default_workers);

        eprintln!();
        eprintln!("  {} WT-MapExtractor: Batch Build All Maps {}", "=".repeat(26), "=".repeat(27));
        eprintln!();
        let progress = Progress::new(maps.len());
        progress.print_header(workers);

        let pool = ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .context("Failed to build worker pool")?;

        let results: Vec<(String, Result<()>)> = pool.install(|| {
            maps.par_iter()
                .enumerate()
                .map(|(idx, map)| {
                    let start = Instant::now();
                    // Read the .bin lazily inside the worker. The OS page
                    // cache coalesces repeated access; we no longer preload
                    // every map binary up-front (that used to consume
                    // ~30 GB of RAM on --all).
                    let result = self.build_one_internal(Some(map), opts, false, None);
                    let duration = start.elapsed();
                    let success = result.is_ok();

                    let status = if success { "✓ OK" } else { "✗ FAIL" };
                    progress.print_progress(map, idx + 1, status, duration.as_secs_f64());
                    progress.increment_completed(success);

                    (map.clone(), result)
                })
                .collect()
        });

        let mut failed_maps = Vec::new();
        for (map, res) in &results {
            if let Err(e) = res {
                let error_msg = format!("{:#}", e);
                failed_maps.push((map.clone(), error_msg));
            }
        }

        progress.print_summary(&failed_maps);

        if !failed_maps.is_empty() {
            eprintln!();
            return Err(anyhow!("  {} map(s) failed. See details above.", failed_maps.len()));
        }

        let maps_index = self.cfg.project_root.join("maps").join("maps_index.json");
        let mut all_entries: Vec<Value> = Vec::new();
        for map in &maps {
            let mpath = self.cfg.project_root.join("maps").join(map).join("manifest.json");
            if let Ok(s) = fs::read_to_string(&mpath) {
                if let Ok(m) = serde_json::from_str::<Value>(&s) {
                    all_entries.push(make_map_summary(map, &m));
                }
            }
        }
        if let Some(parent) = maps_index.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&maps_index, serde_json::to_string_pretty(&all_entries)?)
            .with_context(|| format!("Failed to write {}", maps_index.display()))?;

        eprintln!("  Output: ./maps/");
        eprintln!();

        Ok(())
    }

    pub fn build_one(&self, map_name: Option<&str>, opts: &BuildOptions) -> Result<()> {
        self.build_one_internal(map_name, opts, true, None)
    }

    fn build_one_internal(&self, map_name: Option<&str>, opts: &BuildOptions, verbose: bool, preloaded_bin: Option<Arc<Vec<u8>>>) -> Result<()> {
        let mut levels = self.cfg.for_test_levels.clone();

        let map = match map_name {
            Some(m) => m.to_string(),
            None => self
                .detect_map_name(&levels)
                .ok_or_else(|| anyhow!("No map files found in {}", levels.display()))?,
        };

        let local_bin = levels.join(format!("{map}.bin"));
        if !local_bin.exists() {
            let global_bin = self.cfg.all_levels.join(format!("{map}.bin"));
            if global_bin.exists() {
                levels = self.cfg.all_levels.clone();
            }
        }

        let viewer_dir = self
            .cfg
            .project_root
            .join("maps")
            .join(&map);

        if viewer_dir.exists() {
            // Don't delete the entire map folder, just clean known output files
            let _ = fs::remove_file(viewer_dir.join("heightmap.png"));
            let _ = fs::remove_file(viewer_dir.join("heightmap_detail.png"));
            let _ = fs::remove_file(viewer_dir.join("normalmap_detail.png"));
            let _ = fs::remove_file(viewer_dir.join("colormap.jpg"));
            let _ = fs::remove_file(viewer_dir.join("colormap.png"));
            let _ = fs::remove_file(viewer_dir.join("tile_grid.json"));
            let _ = fs::remove_file(viewer_dir.join("terrain_paint.png"));
            let _ = fs::remove_file(viewer_dir.join("terrain_paint.jpg"));
            let _ = fs::remove_file(viewer_dir.join("terrain_paint_thumb.png"));
            let _ = fs::remove_file(viewer_dir.join("terrain_paint_thumb.jpg"));
            let _ = fs::remove_file(viewer_dir.join("terrain_paint_detail.png"));
            let _ = fs::remove_file(viewer_dir.join("terrain_paint_detail.jpg"));
            let _ = fs::remove_file(viewer_dir.join("missions.json"));
            let _ = fs::remove_file(viewer_dir.join("manifest.json"));
            let _ = fs::remove_dir_all(viewer_dir.join("mat"));
            let _ = fs::remove_dir_all(viewer_dir.join("thumbs"));
        } else {
            fs::create_dir_all(&viewer_dir)
                .with_context(|| format!("Failed to create {}", viewer_dir.display()))?;
        }
        if opts.export_mat {
            fs::create_dir_all(viewer_dir.join("mat"))?;
        }
        if opts.export_thumbs {
            fs::create_dir_all(viewer_dir.join("thumbs"))?;
        }

        if verbose {
            println!("=== WT-MapExtractor (Rust) ===");
            println!("Map: {map}");
            println!("[1/8] Extract DDSx from map container...");
        }
        let bin_path = levels.join(format!("{map}.bin"));
        let bin_data: Option<Arc<Vec<u8>>> = if let Some(pre) = preloaded_bin {
            Some(pre)
        } else if bin_path.exists() {
            Some(Arc::new(
                fs::read(&bin_path)
                    .with_context(|| format!("Failed to read {}", bin_path.display()))?,
            ))
        } else {
            None
        };
        // Convenience slice view for APIs expecting Option<&[u8]>
        let bin_slice: Option<&[u8]> = bin_data.as_ref().map(|a| a.as_slice());
        let mut dds_store: DdsStore = HashMap::new();
        let mut dds_count = 0usize;
        if let Some(data) = bin_slice {
            let store = extract_ddsx_from_data(data, &map);
            dds_count = store.len();
            dds_store.extend(store);
            if verbose {
                println!("  Extracted {dds_count} DDS files");
            }
        } else if verbose {
            println!("  No .bin found, skipping DDSx extraction");
        }

        if verbose {
            println!("[2/8] Extract DxP material packs...");
        }
        let mut dxp_count = 0usize;
        let mut dxp_tex_count = 0usize;
        let map_suffix = map.split_once('_').map(|(_, s)| s).unwrap_or(map.as_str());
        for p in [
            levels.join(format!("{map}.dxp.bin")),
            levels.join(format!("hq_tex_{map}.dxp.bin")),
            levels.join(format!("{map}-hq.dxp.bin")),
            self.cfg.hq_context_dir.join(format!("{map}.dxp.bin")),
            self.cfg.hq_context_dir.join(format!("hq_tex_{map}.dxp.bin")),
            self.cfg.context_dir.join(format!("{map}.dxp.bin")),
            // Shared terrain packs used across many maps.
            self.cfg.hq_context_dir.join("hq_tex_landscape_extra.dxp.bin"),
            self.cfg.context_dir.join("landscape_extra.dxp.bin"),
            // Cross-map alias packs (e.g., air_israel often references avg_israel textures).
            self.cfg.hq_context_dir.join(format!("hq_tex_avg_{map_suffix}.dxp.bin")),
            self.cfg.hq_context_dir.join(format!("hq_tex_{map_suffix}.dxp.bin")),
        ] {
            if p.exists() {
                dxp_count += 1;
                match extract_dxp(&p) {
                    Ok(store) => {
                        dxp_tex_count += store.len();
                        // HQ packs should not overwrite already-loaded textures
                        for (k, v) in store {
                            dds_store.entry(k).or_insert(v);
                        }
                    }
                    Err(e) => {
                        if verbose {
                            eprintln!("  Warning: failed to read {}: {e}", p.display());
                        }
                    }
                }
            }
        }
        if verbose {
            println!("  DxP packs: {dxp_count}, extracted textures: {dxp_tex_count}");
        }

        if verbose {
            println!("[3/8] Build colormap/normalmap overview...");
        }
        let overview_info = export_overview(&map, &dds_store, &viewer_dir)?;

        if verbose {
            println!("[4/8] Build heightmap...");
        }
        // Parse blkx early so water_level can feed into heightmap ocean-cutout handling.
        let blkx_path = self.find_blkx(&map, &levels);
        let blkx = match blkx_path {
            Some(ref p) => Self::parse_blkx(p).unwrap_or_else(|_| json!({})),
            None => json!({}),
        };
        let water_level_opt = read_num_or_first(&blkx, "water_level");
        // Always pass Some(wl) to the heightmap builder so that ocean areas
        // (LR NaN cells) are filled with the world water level rather than
        // min_h.  Maps without an explicit water_level key default to 0.0.
        let water_level_for_hm = water_level_opt.or(Some(0.0));
        let mut heightmap_info = if let Some(ref data) = bin_slice {
            match build_heightmap_native(data, &viewer_dir, water_level_for_hm)? {
                Some(v) => v,
                None => build_heightmap_fallback(&viewer_dir)?,
            }
        } else {
            build_heightmap_fallback(&viewer_dir)?
        };
        // Hoist the embedded _heightmapDetail key to the top-level manifest
        let heightmap_detail_info = heightmap_info
            .as_object_mut()
            .and_then(|obj| obj.remove("_heightmapDetail"))
            .unwrap_or(Value::Null);

        if verbose {
            if opts.skip_tile_grid {
                println!("[5/8] Build tile metadata (tile grid skipped)...");
            } else {
                println!("[5/8] Build tile metadata + tile grid...");
            }
        }
        let tile_set = export_tiles(&map, &dds_store, bin_slice)?;
        let map_grid: Option<LndmGrid> = bin_slice.and_then(parse_lndm_grid);
        let tile_grid_info: Option<Value> = if opts.skip_tile_grid {
            None
        } else {
            build_tile_grid(&viewer_dir, &tile_set, map_grid)?
        };

        if verbose {
            println!("[6/8] Parse landclass metadata...");
        }
        let detail_info = if let Some(ref data) = bin_slice {
            match extract_landclass_native(data)? {
                Some(v) => v,
                None => json!({ "landclasses": [] }),
            }
        } else {
            json!({ "landclasses": [] })
        };

        let height_min_m = heightmap_info.get("height_min_m").and_then(Value::as_f64);
        let height_max_m = heightmap_info.get("height_max_m").and_then(Value::as_f64);
        let pseudo_heightmap = heightmap_info
            .get("pseudo")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let water_mask = WaterMaskParams {
            water_level: water_level_opt,
            height_min_m,
            height_max_m,
            pseudo_heightmap,
        };

        if verbose {
            println!("[7/8] Build terrain paint...");
        }
        let landclass_list: Vec<Value> = detail_info
            .get("landclasses")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        // Resolve any landclass-referenced textures that aren't already in the
        // per-map DDS store by pulling them from the global DxP index. This
        // picks up shared detail_* textures and cross-map aliases that the
        // per-map DxP discovery heuristic cannot cover (e.g. air_race_phiphi_*
        // needing phi_phi_* textures, detail_forest_tex_d, etc.).
        let mut material_names: BTreeSet<String> = BTreeSet::new();
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        for lc in &landclass_list {
            for name in crate::paint::collect_lc_texture_candidates(lc) {
                material_names.insert(name.clone());
                if !dds_store.contains_key(&name) {
                    wanted.insert(name);
                }
            }
        }
        let mut index_resolved = 0usize;
        if !wanted.is_empty() {
            let idx = self.dxp_index();
            let resolved = idx.resolve_batch(wanted.iter());
            index_resolved = resolved.len();
            for (k, v) in resolved.iter() {
                dds_store.entry(k.clone()).or_insert_with(|| v.clone());
            }
            if verbose {
                println!(
                    "  Resolved {index_resolved}/{} speculative LC texture candidate names via global DxP index",
                    wanted.len()
                );
                // The unresolved list is benign noise: `wanted` contains
                // every candidate name from `collect_lc_texture_candidates`
                // (real `lc.texture`, plus speculative `{name}_tex_d`
                // suffixes and detail R/G/B/K aliases). Real per-LC
                // texture resolution is reported by `lcTextureResolved /
                // lcTextureMissing` in the terrainPaint manifest block.
                let mut unresolved: Vec<&String> =
                    wanted.iter().filter(|n| !resolved.contains_key(*n)).collect();
                unresolved.sort();
                if !unresolved.is_empty() {
                    println!(
                        "  Unresolved candidate names ({}, most are speculative aliases):",
                        unresolved.len()
                    );
                    for n in &unresolved {
                        println!("    - {n}");
                    }
                }
            }
        }

        let paint_info = if let Some(ref data) = bin_slice {
            let hm2_for_detail = heightmap_detail_info.as_object().and_then(|hmd| {
                let x0 = hmd.get("world_x0").and_then(Value::as_f64)?;
                let z0 = hmd.get("world_z0").and_then(Value::as_f64)?;
                let x1 = hmd.get("world_x1").and_then(Value::as_f64)?;
                let z1 = hmd.get("world_z1").and_then(Value::as_f64)?;
                Some(Hm2ExtentForPaint { world_x0: x0, world_z0: z0, world_x1: x1, world_z1: z1 })
            });
            match build_terrain_paint_native(
                &map,
                data,
                &dds_store,
                &viewer_dir,
                &landclass_list,
                &tile_set,
                &water_mask,
                opts.sun_azimuth,
                opts.sun_elevation,
                opts.sun_strength,
                hm2_for_detail,
                opts.compress_maps,
            )? {
                Some(v) => v,
                None => build_terrain_paint_fallback(&viewer_dir)?.unwrap_or_else(|| {
                    json!({
                        "file": Value::Null,
                        "notes": "native compositor in progress"
                    })
                }),
            }
        } else {
            build_terrain_paint_fallback(&viewer_dir)?.unwrap_or_else(|| {
                json!({
                    "file": Value::Null,
                    "notes": "native compositor in progress"
                })
            })
        };

        if verbose {
            if opts.skip_rendinst {
                println!("[8/8] Parse render instances metadata (skipped)...");
            } else {
                println!("[8/8] Parse render instances metadata...");
            }
        }
        let rendinst_info = if opts.skip_rendinst {
            Value::Null
        } else {
            if let Some(ref data) = bin_slice {
                match extract_rendinst_native(data, &viewer_dir)? {
                    Some(v) => v,
                    None => Value::Null,
                }
            } else {
                Value::Null
            }
        };

        let datamine_root = self
            .cfg
            .aces_levels
            .parent()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.cfg.aces_levels.clone());
        let missions_json = extract_missions_for_map(&datamine_root, &map)?;
        if let Some(ref mjson) = missions_json {
            let missions_path = viewer_dir.join("missions.json");
            fs::write(&missions_path, serde_json::to_string_pretty(mjson)?)
                .with_context(|| format!("Failed to write {}", missions_path.display()))?;
        }

        let materials = if opts.export_mat {
            export_materials(&map, &dds_store, &viewer_dir, Some(&material_names))?
        } else {
            vec![]
        };

        let map_coord0 = read_vec2(&blkx, "mapCoord0").unwrap_or([-32768.0, -32768.0]);
        let map_coord1 = read_vec2(&blkx, "mapCoord1").unwrap_or([32768.0, 32768.0]);
        let map_size = [map_coord1[0] - map_coord0[0], map_coord1[1] - map_coord0[1]];
        let average_ground_level = read_num_or_first(&blkx, "average_ground_level");
        let custom_level_map = read_str(&blkx, "customLevelMap");
        let tank_zone = match (read_vec2(&blkx, "tankMapCoord0"), read_vec2(&blkx, "tankMapCoord1")) {
            (Some(coord0), Some(coord1)) => {
                json!({
                    "coord0": coord0,
                    "coord1": coord1,
                    "gridSize": read_num_or_first(&blkx, "tankGridSize"),
                })
            }
            _ => Value::Null,
        };
        // Use null when BLK has no location keys so the viewer only shows the
        // globe for maps where geographic coordinates are actually known.
        let lat_opt: Option<f64> = read_num(&blkx, "latitude")
            .or_else(|| blkx.get("stars").and_then(|s| s.get("latitude")).and_then(Value::as_f64));
        let lon_opt: Option<f64> = read_num(&blkx, "longitude")
            .or_else(|| blkx.get("stars").and_then(|s| s.get("longitude")).and_then(Value::as_f64));

        let mut notes = BTreeMap::new();
        notes.insert("rustPortStatus", Value::String("in_progress".to_string()));
        notes.insert(
            "message",
            Value::String("Rust extraction/export path active. Native heightmap and terrain paint are enabled.".to_string()),
        );

        let normalmap_info: Value = if viewer_dir.join("normalmap.png").exists() {
            json!({ "file": "normalmap.png" })
        } else {
            Value::Null
        };

        let mut landclasses_manifest = landclass_list.clone();
        if let Some(paint_obj) = paint_info.as_object() {
            if let Some(lc_names) = paint_obj.get("lcNames").and_then(Value::as_array) {
                if lc_names.len() > landclasses_manifest.len() {
                    for i in landclasses_manifest.len()..lc_names.len() {
                        let name = lc_names
                            .get(i)
                            .and_then(Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("lc_{i}"));
                        landclasses_manifest.push(json!({
                            "name": name,
                            "texture": Value::Null,
                            "details": {},
                            "size": Value::Null,
                            "detailSizes": Value::Null
                        }));
                    }
                }
            }
        }

        let manifest = json!({
            "mapName": map,
            "mapCoord0": map_coord0,
            "mapCoord1": map_coord1,
            "mapSize": map_size,
            // Serialize as null when the BLK has no explicit water_level key so
            // the viewer can distinguish landlocked maps from true ocean maps.
            "waterLevel": water_level_opt,
            "averageGroundLevel": average_ground_level,
            "customLevelMap": custom_level_map,
            "tankZone": tank_zone,
            "location": {
                "latitude": lat_opt,
                "longitude": lon_opt,
            },
            "sun": {
                "azimuth": opts.sun_azimuth,
                "elevation": opts.sun_elevation,
                "strength": opts.sun_strength,
            },
            "pipeline": {
                "binFound": bin_slice.is_some(),
                "ddsExtracted": dds_count,
                "dxpCandidateCount": dxp_count,
                "dxpTexturesExtracted": dxp_tex_count,
                "dxpIndexResolved": index_resolved,
                "materialsRequested": opts.export_mat,
                "materialsPreconverted": false,
                "thumbnailsRequested": opts.export_thumbs,
                "tileGridSkipped": opts.skip_tile_grid,
                "rendinstSkipped": opts.skip_rendinst,
            },
            "colormap": overview_info,
            "normalmap": normalmap_info,
            "heightmap": heightmap_info,
            "heightmapDetail": heightmap_detail_info,
            "tileCount": tile_set.info.len(),
            "tiles": tile_set.info.iter().map(|t| json!({"index": t.index, "image": t.image})).collect::<Vec<_>>(),
            "tileGrid": tile_grid_info,
            "terrainPaint": paint_info,
            "materials": materials,
            "landclasses": landclasses_manifest,
            "lndm": detail_info.get("lndm").cloned().unwrap_or(Value::Null),
            "rendinst": rendinst_info,
            "hasMissions": missions_json.is_some(),
            "notes": notes,
        });

        let manifest_path = viewer_dir.join("manifest.json");
        fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)
            .with_context(|| format!("Failed to write {}", manifest_path.display()))?;

        if opts.export_thumbs {
            let _ = write_debug_thumbs(&viewer_dir);
        }

        if verbose {
            println!("Wrote {}", manifest_path.display());
            // Single-map build: update the gallery index immediately
            let maps_index_path = self.cfg.project_root.join("maps").join("maps_index.json");
            let _ = upsert_maps_index(&maps_index_path, &map, &manifest);
        }
        Ok(())
    }

    fn detect_map_name(&self, levels: &Path) -> Option<String> {
        if levels.exists() {
            if let Ok(read_dir) = fs::read_dir(levels) {
                for entry in read_dir.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("blkx") {
                        return path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(ToOwned::to_owned);
                    }
                }
            }

            if let Ok(read_dir) = fs::read_dir(levels) {
                for entry in read_dir.flatten() {
                    let path = entry.path();
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                    if path.extension().and_then(|e| e.to_str()) == Some("bin") && !name.ends_with(".dxp.bin") {
                        return path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(ToOwned::to_owned);
                    }
                }
            }
        }
        None
    }

    fn find_blkx(&self, map: &str, levels: &Path) -> Option<PathBuf> {
        let candidates = [
            levels.join(format!("{map}.blkx")),
            self.cfg.aces_levels.join(format!("{map}.blkx")),
            self.cfg.all_levels.join(format!("{map}.blkx")),
        ];
        candidates.into_iter().find(|p| p.exists())
    }

    fn parse_blkx(path: &Path) -> Result<Value> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("Failed reading {}", path.display()))?;
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("Invalid JSON in {}", path.display()))?;
        Ok(value)
    }

}

fn read_num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn read_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(ToOwned::to_owned)
}

fn read_num_or_first(v: &Value, key: &str) -> Option<f64> {
    if let Some(n) = read_num(v, key) {
        return Some(n);
    }
    let arr = v.get(key)?.as_array()?;
    arr.first()?.as_f64()
}

fn read_vec2(v: &Value, key: &str) -> Option<[f64; 2]> {
    let arr = v.get(key)?.as_array()?;
    if arr.len() < 2 {
        return None;
    }
    Some([arr[0].as_f64()?, arr[1].as_f64()?])
}
