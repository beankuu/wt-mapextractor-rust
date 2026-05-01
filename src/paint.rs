use std::path::Path;
use std::sync::Arc;
use std::collections::HashMap;

use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::{imageops, ImageBuffer, Rgb, RgbImage, RgbaImage};
use rayon::prelude::*;
use serde_json::{json, Value};

use crate::export::{DdsStore, TileSet};
use crate::post::parse_lndm_grid;
use crate::util::{decode_dds_bytes, read_i32_le, read_u32_le};

/// JPEG quality used for all terrain paint outputs (0-100).
const JPEG_QUALITY: u8 = 84;

fn save_rgb_image(img: &RgbImage, path: &Path, compress: bool) -> Result<()> {
    if compress {
        // Save as JPEG (compressed output path).
        let f = std::fs::File::create(path)
            .with_context(|| format!("Failed to create {}", path.display()))?;
        let mut enc = JpegEncoder::new_with_quality(std::io::BufWriter::new(f), JPEG_QUALITY);
        enc.encode(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgb8)
            .with_context(|| format!("Failed to encode JPEG {}", path.display()))?;
    } else {
        // Save as PNG (uncompressed, faster)
        img.save(path)
            .with_context(|| format!("Failed to save PNG {}", path.display()))?;
    }
    Ok(())
}

/// World extent of the HM2 detail region, used to crop terrain_paint_detail output.
pub struct Hm2ExtentForPaint {
    pub world_x0: f64,
    pub world_z0: f64,
    pub world_x1: f64,
    pub world_z1: f64,
}

fn parse_grid(data: &[u8]) -> Option<(usize, usize)> {
    let lndm_pos = data.windows(4).position(|w| w == b"lndm")?;
    let mut p = lndm_pos + 4;
    let version = read_i32_le(data, p)?;
    p += 4;
    if version != 4 {
        return None;
    }
    p += 4 + 4; // gridCellSize, landCellSize
    let map_x = read_i32_le(data, p)?;
    p += 4;
    let map_y = read_i32_le(data, p)?;
    if map_x <= 0 || map_y <= 0 {
        return None;
    }
    Some((map_x as usize, map_y as usize))
}

#[derive(Debug, Clone)]
struct TileCell {
    det: [u8; 7],
    export_idx: Option<usize>,
    has_tex2: bool,
}

#[derive(Clone)]
struct LcMaterial {
    fallback_seed: u64,
    texture: Option<Arc<RgbImage>>,
    detail_textures: [Option<Arc<RgbImage>>; 4],
    detail_scales: [f32; 4],
    scale_x: f32,
    scale_y: f32,
}

#[derive(Debug, Clone, Default)]
pub struct WaterMaskParams {
    pub water_level: Option<f64>,
    pub height_min_m: Option<f64>,
    pub height_max_m: Option<f64>,
    /// If true, the heightmap is synthesized (e.g. from the colormap) and we
    /// fall back to a colormap-based ocean detector instead of using
    /// `height_{min,max}_m`.
    pub pseudo_heightmap: bool,
}

#[derive(Debug, Clone)]
struct WaterMaskProtection {
    protected_cells: Vec<bool>,
    grid_w: usize,
    grid_h: usize,
    tile_w: u32,
    tile_h: u32,
}

/// Scanline flood-fill of `water_bytes` bytes where `mask[i]==1`, starting
/// from every border pixel with `mask[i]==1`. Writes 1 to `out` where
/// reachable from a border through mask-true pixels. 4-connected.
fn flood_from_borders(mask: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; mask.len()];
    if w == 0 || h == 0 { return out; }
    let mut stack: Vec<(i32, i32)> = Vec::new();
    let mut seed = |stack: &mut Vec<(i32, i32)>, x: usize, y: usize| {
        let idx = y * w + x;
        if mask[idx] != 0 && out[idx] == 0 {
            out[idx] = 1;
            stack.push((x as i32, y as i32));
        }
    };
    for x in 0..w {
        seed(&mut stack, x, 0);
        seed(&mut stack, x, h - 1);
    }
    for y in 0..h {
        seed(&mut stack, 0, y);
        seed(&mut stack, w - 1, y);
    }
    while let Some((cx, cy)) = stack.pop() {
        // Scanline: expand left and right along the current row first.
        let mut lx = cx;
        while lx > 0 {
            let idx = cy as usize * w + (lx as usize - 1);
            if mask[idx] == 0 || out[idx] != 0 { break; }
            lx -= 1;
            out[idx] = 1;
        }
        let mut rx = cx;
        while (rx as usize) + 1 < w {
            let idx = cy as usize * w + (rx as usize + 1);
            if mask[idx] == 0 || out[idx] != 0 { break; }
            rx += 1;
            out[idx] = 1;
        }
        // Check rows above/below across [lx..=rx].
        for nx in lx..=rx {
            if cy > 0 {
                let ny = cy - 1;
                let idx = ny as usize * w + nx as usize;
                if mask[idx] != 0 && out[idx] == 0 {
                    out[idx] = 1;
                    stack.push((nx, ny));
                }
            }
            if (cy as usize) + 1 < h {
                let ny = cy + 1;
                let idx = ny as usize * w + nx as usize;
                if mask[idx] != 0 && out[idx] == 0 {
                    out[idx] = 1;
                    stack.push((nx, ny));
                }
            }
        }
    }
    out
}

fn apply_water_mask(
    canvas: &mut RgbImage,
    viewer_dir: &Path,
    params: &WaterMaskParams,
    protection: Option<&WaterMaskProtection>,
) {
    // Build a boolean "candidate water pixel" mask in canvas resolution.
    // Policy differs between real and pseudo heightmaps.
    let (cw, ch) = (canvas.width(), canvas.height());
    let npx = (cw as usize) * (ch as usize);
    let mut cand = vec![0u8; npx];
    let mut any_candidate = false;

    if params.pseudo_heightmap {
        // For pseudo heightmaps (e.g. built from colormap), height thresholds
        // are meaningless. Derive ocean from the colormap blue-dominance
        // (typical satellite-imagery ocean detector).
        let colormap = {
            let jpg = viewer_dir.join("colormap.jpg");
            if jpg.exists() { jpg } else { viewer_dir.join("colormap.png") }
        };
        if !colormap.exists() || params.water_level.is_none() {
            return;
        }
        let img = match image::open(&colormap) {
            Ok(v) => v.to_rgb8(),
            Err(_) => return,
        };
        let resized = if img.width() != cw || img.height() != ch {
            image::imageops::resize(&img, cw, ch, image::imageops::FilterType::Triangle)
        } else {
            img
        };
        for (i, p) in resized.pixels().enumerate() {
            let r = p[0] as i32;
            let g = p[1] as i32;
            let b = p[2] as i32;
            // Ocean heuristic: blue dominates, greenish-blue allowed, dark enough.
            let blue_dom = b > r + 12 && b >= g - 6;
            let dark_enough = (r + g + b) < 360; // avg < 120
            if blue_dom && dark_enough {
                cand[i] = 1;
                any_candidate = true;
            }
        }
    } else {
        let (water_level, h_min, h_max) =
            match (params.water_level, params.height_min_m, params.height_max_m) {
                (Some(wl), Some(hmin), Some(hmax)) if hmax > hmin => (wl, hmin, hmax),
                _ => return,
            };

        // Skip entirely for landlocked maps whose minimum elevation is above
        // the stated water level — the authored water plane sits below the
        // terrain and cannot flood anything (avg_krymsk: wl=10, hmin=135).
        if h_min > water_level {
            return;
        }

        let h_range = h_max - h_min;

        let hm_path = viewer_dir.join("heightmap.png");
        if !hm_path.exists() {
            return;
        }
        let hm = match image::open(&hm_path) {
            Ok(v) => v.to_luma8(),
            Err(_) => return,
        };
        let hm_resized = if hm.width() != cw || hm.height() != ch {
            image::imageops::resize(
                &hm,
                cw,
                ch,
                image::imageops::FilterType::Triangle,
            )
        } else {
            hm
        };
        // Slightly conservative: pull threshold down half a byte to avoid
        // over-expanding at coastlines where the 8-bit quantization edge
        // would otherwise flood terrain straddling the waterline.
        let water_threshold = water_level - (h_range / 255.0);
        let inv255 = 1.0 / 255.0;
        for (i, &b) in hm_resized.as_raw().iter().enumerate() {
            let h_m = h_min + (b as f64 * inv255) * h_range;
            if h_m <= water_threshold {
                cand[i] = 1;
                any_candidate = true;
            }
        }
    }

    if !any_candidate {
        return;
    }

    // Flood-fill from image borders through the candidate mask. Only pixels
    // connected to the border stay marked → inland plains (Berlin) and
    // lakes without an authored water feature are excluded, while real
    // coastlines and archipelagos still fill correctly.
    let reachable = flood_from_borders(&cand, cw as usize, ch as usize);

    // If the border flood didn't reach any candidate (e.g. the map is
    // fully enclosed by land and the threshold picked up only small
    // interior lakes), fall back to the raw candidate mask but only when
    // candidate coverage is small. This avoids accidentally painting huge
    // inland regions like Berlin as ocean.
    let mut reached_count = 0usize;
    for &b in &reachable { reached_count += b as usize; }
    let use_mask: &[u8] = if reached_count > 0 {
        &reachable
    } else {
        let cand_ratio = {
            let mut c = 0usize;
            for &b in &cand { c += b as usize; }
            c as f64 / npx as f64
        };
        if cand_ratio < 0.05 {
            &cand
        } else {
            // Large candidate region with no border contact → likely a
            // landlocked river/plain triggered the threshold. Skip.
            return;
        }
    };

    let out_raw = canvas.as_mut();
    out_raw
        .par_chunks_mut(3)
        .zip(use_mask.par_iter())
        .enumerate()
        .for_each(|(i, (o, &m))| {
            if m != 0 {
                if let Some(p) = protection {
                    let x = (i as u32) % cw;
                    let y = (i as u32) / cw;
                    // Canvas is 1:1 with the §5 float frame (no rotate): pixel
                    // (x, y) sits in cell (x/tile_w, y/tile_h) directly.
                    let cx = (x / p.tile_w) as usize;
                    let cz = (y / p.tile_h) as usize;
                    if cx < p.grid_w && cz < p.grid_h {
                        let ci = cz * p.grid_w + cx;
                        if p.protected_cells.get(ci).copied().unwrap_or(false) {
                            return;
                        }
                    }
                }
                o[0] = 20;
                o[1] = 60;
                o[2] = 120;
            }
        });
}

/// Feather the regular cell-grid seams on the composited canvas. Only pixels
/// within `border` of any horizontal or vertical cell boundary are touched;
/// each such pixel is alpha-blended with a small box-filtered sample whose
/// footprint crosses the seam. Interior pixels (further than `border` from
/// any seam) are unchanged, so material detail is preserved.
fn blend_cell_seams(canvas: &mut RgbImage, tile_w: u32, tile_h: u32, border: u32) {
    if border == 0 || tile_w <= 2 * border || tile_h <= 2 * border {
        return;
    }
    let (w, h) = canvas.dimensions();
    if w == 0 || h == 0 { return; }
    let src = canvas.clone();
    let src_raw: &[u8] = src.as_raw();
    let inv_border = 1.0f32 / border as f32;
    let radius: i32 = border as i32;

    let mut affected: Vec<usize> = Vec::new();
    let seam_band = border.saturating_sub(1) as i32;
    let mut seam_x = tile_w;
    while seam_x < w {
        let x0 = (seam_x as i32 - seam_band).max(0) as u32;
        let x1 = (seam_x + border - 1).min(w - 1);
        for y in 0..h {
            let row = y as usize * w as usize;
            for x in x0..=x1 {
                affected.push(row + x as usize);
            }
        }
        seam_x += tile_w;
    }
    let mut seam_y = tile_h;
    while seam_y < h {
        let y0 = (seam_y as i32 - seam_band).max(0) as u32;
        let y1 = (seam_y + border - 1).min(h - 1);
        for y in y0..=y1 {
            let row = y as usize * w as usize;
            for x in 0..w {
                affected.push(row + x as usize);
            }
        }
        seam_y += tile_h;
    }
    affected.sort_unstable();
    affected.dedup();

    let updates: Vec<(usize, u8, u8, u8)> = affected
        .par_iter()
        .filter_map(|&idx| {
            let x = (idx as u32) % w;
            let y = (idx as u32) / w;
            let dx_mod = x % tile_w;
            let dy_mod = y % tile_h;
            // distance from the nearest vertical / horizontal seam
            let dx_dist = dx_mod.min(tile_w - dx_mod);
            let dy_dist = dy_mod.min(tile_h - dy_mod);
            let d = dx_dist.min(dy_dist);
            if d >= border {
                return None;
            }
            // Box-filter sample crossing the seam. Keep the kernel small so
            // total cost is O(border^2) per seam pixel, ~6% of canvas area.
            let (mut sr, mut sg, mut sb, mut n) = (0u32, 0u32, 0u32, 0u32);
            let x0 = (x as i32 - radius).max(0) as u32;
            let y0 = (y as i32 - radius).max(0) as u32;
            let x1 = ((x as i32 + radius) as u32).min(w - 1);
            let y1 = ((y as i32 + radius) as u32).min(h - 1);
            for yy in y0..=y1 {
                let row = (yy as usize) * (w as usize) * 3;
                for xx in x0..=x1 {
                    let p = row + (xx as usize) * 3;
                    sr += src_raw[p] as u32;
                    sg += src_raw[p + 1] as u32;
                    sb += src_raw[p + 2] as u32;
                    n += 1;
                }
            }
            if n == 0 {
                return None;
            }
            let ar = (sr / n) as f32;
            let ag = (sg / n) as f32;
            let ab = (sb / n) as f32;
            // Linear fade: seam center (d=0) = fully blurred, seam edge (d=border) = original.
            let alpha = 1.0 - (d as f32) * inv_border;
            let p = idx * 3;
            let orig_r = src_raw[p] as f32;
            let orig_g = src_raw[p + 1] as f32;
            let orig_b = src_raw[p + 2] as f32;
            Some((
                idx,
                (orig_r + (ar - orig_r) * alpha).round().clamp(0.0, 255.0) as u8,
                (orig_g + (ag - orig_g) * alpha).round().clamp(0.0, 255.0) as u8,
                (orig_b + (ab - orig_b) * alpha).round().clamp(0.0, 255.0) as u8,
            ))
        })
        .collect();

    let out = canvas.as_mut();
    for (idx, r, g, b) in updates {
        let p = idx * 3;
        out[p] = r;
        out[p + 1] = g;
        out[p + 2] = b;
    }
}

fn apply_normalmap_lighting(
    canvas: &mut RgbImage,
    viewer_dir: &Path,
    sun_azimuth: f64,
    sun_elevation: f64,
    sun_strength: f64,
) {
    let nmap_path = viewer_dir.join("normalmap.png");
    if !nmap_path.exists() {
        return;
    }

    let nmap = match image::open(&nmap_path) {
        Ok(v) => v.to_rgb8(),
        Err(_) => return,
    };
    // Canvas and normalmap.png are both in display space (ROTATE_180 applied).
    // The channel inversion for X/Y is already baked into the -(v*2-1) formula
    // below, matching Python's approach.

    let nmap_resized = if nmap.width() != canvas.width() || nmap.height() != canvas.height() {
        image::imageops::resize(
            &nmap,
            canvas.width(),
            canvas.height(),
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        nmap
    };

    let el_rad = sun_elevation.clamp(0.0, 90.0).to_radians();
    let az_rad = sun_azimuth.to_radians();
    let lx = el_rad.cos() * az_rad.sin();
    let ly = el_rad.cos() * az_rad.cos();
    let lz = el_rad.sin();
    let li = sun_strength.clamp(0.0, 1.0) as f32;
    let ambient = 1.0f32 - li;

    let n_raw = nmap_resized.as_raw();
    let out_raw = canvas.as_mut();
    let lx_f = lx as f32;
    let ly_f = ly as f32;
    let lz_f = lz as f32;
    // Parallel row-wise shading: canvas and normalmap share identical dims.
    out_raw
        .par_chunks_mut(3)
        .zip(n_raw.par_chunks(3))
        .for_each(|(o, n)| {
            let nr = n[0] as f32 * (1.0 / 255.0);
            let ng = n[1] as f32 * (1.0 / 255.0);
            let nb = n[2] as f32 * (1.0 / 255.0);

            let mut nx = -(nr * 2.0 - 1.0);
            let mut ny = -(ng * 2.0 - 1.0);
            let mut nz = nb * 2.0 - 1.0;
            let mag = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
            let inv = 1.0 / mag;
            nx *= inv;
            ny *= inv;
            nz *= inv;

            let diffuse = (nx * lx_f + ny * ly_f + nz * lz_f).clamp(0.0, 1.0);
            let shading = ambient + li * diffuse;

            o[0] = (o[0] as f32 * shading).clamp(0.0, 255.0) as u8;
            o[1] = (o[1] as f32 * shading).clamp(0.0, 255.0) as u8;
            o[2] = (o[2] as f32 * shading).clamp(0.0, 255.0) as u8;
        });
}

fn parse_tile_cells(data: &[u8], num_cells: usize) -> Option<Vec<TileCell>> {
    const DDSX_MAGIC: &[u8; 4] = b"DDSx";
    const CELL_PREFIX: usize = 15;
    let grid = parse_lndm_grid(data)?;
    let det_start = grid.tile_data_abs;
    if det_start == 0 || det_start + CELL_PREFIX > data.len() {
        return None;
    }

    // Walk tile cells directly from lndm tile stream (v4 layout):
    // [det:7][totalLen:4][tex2Ofs:4][DDSx...totalLen]
    let mut walk_pos = det_start;
    let mut export_idx = 1usize;
    let mut cells = Vec::with_capacity(num_cells);
    let mut non_empty = 0usize;

    for _ci in 0..num_cells {
        if walk_pos + CELL_PREFIX > data.len() {
            break;
        }
        let mut det = [0u8; 7];
        det.copy_from_slice(&data[walk_pos..walk_pos + 7]);
        let total_len = read_u32_le(data, walk_pos + 7)? as usize;
        let tex2_off = read_u32_le(data, walk_pos + 11)? as usize;
        let ddsx_start = walk_pos + CELL_PREFIX;

        if total_len == 0 {
            cells.push(TileCell { det, export_idx: None, has_tex2: false });
            walk_pos = ddsx_start;
        } else {
            // Validate expected DDSx boundary before trusting this cell.
            if ddsx_start + 4 > data.len() || &data[ddsx_start..ddsx_start + 4] != DDSX_MAGIC {
                break;
            }

            // tex2Offset belongs to the 15-byte cell prefix. Validate the
            // nested DDSx header to avoid false positives that shift export_idx.
            let has_tex2 = tex2_off > 0
                && tex2_off + 4 <= total_len
                && ddsx_start + tex2_off + 4 <= data.len()
                && &data[ddsx_start + tex2_off..ddsx_start + tex2_off + 4] == DDSX_MAGIC;

            cells.push(TileCell {
                det,
                export_idx: Some(export_idx),
                has_tex2,
            });
            non_empty += 1;
            export_idx += 1;
            if has_tex2 {
                export_idx += 1;  // Skip tex2 index
            }
            walk_pos = ddsx_start + total_len;
        }
    }

    if cells.is_empty() || cells.len() < num_cells {
        return None;
    }
    // Guard against false-positive scans: terrain paint is unusable if almost
    // all cells parse as empty.
    if non_empty.saturating_mul(100) < num_cells.saturating_mul(2) {
        return None;
    }
    Some(cells)
}

fn hash_seed(name: &str) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn read_lc_scale(lc: &Value) -> (f32, f32) {
    if let Some(arr) = lc.get("size").and_then(Value::as_array) {
        if arr.len() >= 2 {
            let sx = arr[0].as_f64().unwrap_or(256.0) as f32;
            let sy = arr[1].as_f64().unwrap_or(256.0) as f32;
            return (sx.max(8.0), sy.max(8.0));
        }
    }
    (256.0, 256.0)
}

fn is_ocean_like_lc_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("ocean") || lower.contains("_ocean")
}

fn sanitize_det_indices(mut det: [u8; 7], ocean_like_lc: &[bool]) -> [u8; 7] {
    let base_idx = det[0] as usize;
    let base_is_ocean = base_idx < ocean_like_lc.len() && ocean_like_lc[base_idx];
    if !base_is_ocean {
        for idx in det.iter_mut().skip(1) {
            let lc_idx = *idx as usize;
            if *idx != 0xFF && lc_idx < ocean_like_lc.len() && ocean_like_lc[lc_idx] {
                *idx = 0xFF;
            }
        }
    }
    det
}

pub fn collect_lc_texture_candidates(lc: &Value) -> Vec<String> {
    // Mirror Python's load_material_tiles() candidate order:
    //   1. lc.texture (with trailing '*' stripped — authored tex reference)
    //   2. lc.name
    //   3. "{lc.name}_tex_d"
    //   4. each details.{R,G,B,K} plus '_' prefix stripped short form
    let mut cands: Vec<String> = Vec::new();
    let push_unique = |cands: &mut Vec<String>, s: String| {
        if !s.is_empty() && !cands.iter().any(|c| c == &s) {
            cands.push(s);
        }
    };

    let name = lc
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();
    if let Some(tex_raw) = lc
        .get("texture")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let tex = tex_raw.replace('*', "");
        if !tex.is_empty() && tex != name {
            push_unique(&mut cands, tex);
        }
    }
    if !name.is_empty() {
        push_unique(&mut cands, name.clone());
        push_unique(&mut cands, format!("{name}_tex_d"));
    }
    if let Some(obj) = lc.get("details").and_then(Value::as_object) {
        // Only diffuse channels (R/G/B/K) — *_r entries are roughness maps
        // whose RGB channels render as sky-blue if mistakenly used as diffuse.
        for key in ["R", "G", "B", "K"] {
            if let Some(v) = obj.get(key).and_then(Value::as_str) {
                let v = v.to_string();
                if v.is_empty() {
                    continue;
                }
                push_unique(&mut cands, v.clone());
                if let Some(short) = v.strip_prefix("detail_") {
                    push_unique(&mut cands, short.to_string());
                }
            }
        }
    }
    cands
}

fn load_texture_png(name: &str, viewer_dir: &Path, dds_store: &DdsStore) -> Option<RgbImage> {
    let mat_png = viewer_dir.join("mat").join(format!("{name}.png"));
    if mat_png.exists() {
        return image::open(&mat_png).ok().map(|v| v.to_rgb8());
    }
    let mat_png_na16 = viewer_dir.join("mat").join(format!("{name}_na16.png"));
    if mat_png_na16.exists() {
        return image::open(&mat_png_na16).ok().map(|v| v.to_rgb8());
    }

    if let Some(maps_root) = viewer_dir.parent() {
        let shared_png = maps_root.join("shared").join("mat").join(format!("{name}.png"));
        if shared_png.exists() {
            return image::open(&shared_png).ok().map(|v| v.to_rgb8());
        }
        let shared_png_na16 = maps_root.join("shared").join("mat").join(format!("{name}_na16.png"));
        if shared_png_na16.exists() {
            return image::open(&shared_png_na16).ok().map(|v| v.to_rgb8());
        }
    }

    if let Some(dds_bytes) = dds_store.get(name) {
        return decode_dds_bytes(dds_bytes).ok().map(|v| v.to_rgb8());
    }
    None
}

fn load_cached_texture(
    name: &str,
    texture_cache: &mut HashMap<String, Option<Arc<RgbImage>>>,
    viewer_dir: &Path,
    dds_store: &DdsStore,
) -> Option<Arc<RgbImage>> {
    if let Some(cached) = texture_cache.get(name) {
        return cached.clone();
    }

    let loaded = load_texture_png(name, viewer_dir, dds_store).map(Arc::new);
    texture_cache.insert(name.to_string(), loaded.clone());
    loaded
}

fn build_lc_materials(
    landclasses: &[Value],
    viewer_dir: &Path,
    dds_store: &DdsStore,
    world_per_px_x: f32,
    world_per_px_z: f32,
) -> Vec<LcMaterial> {
    // Reuse already-loaded textures across landclasses to trade RAM for speed.
    let mut texture_cache: HashMap<String, Option<Arc<RgbImage>>> = HashMap::new();

    let mut mats = Vec::with_capacity(landclasses.len());
    for lc in landclasses {
        let name = lc
            .get("texture")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                lc.get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("lc");
        let fallback_seed = hash_seed(name);
        let (scale_x_m, scale_y_m) = read_lc_scale(lc);
        // LC `size` is a world-metre period (texture repeats every size_m
        // world-metres). sample_texture_scaled consumes the value in OUTPUT
        // PIXELS (gx/scale_x is the repeat fraction), so convert here using
        // the paint canvas's world-per-pixel ratio. Ratios ≤ 0 fall back to
        // 1:1 which matches the pre-fix behaviour.
        let scale_x = (scale_x_m / world_per_px_x.max(1e-6)).max(8.0);
        let scale_y = (scale_y_m / world_per_px_z.max(1e-6)).max(8.0);

        let mut texture: Option<Arc<RgbImage>> = None;
        for cand in collect_lc_texture_candidates(lc) {
            if let Some(img) = load_cached_texture(&cand, &mut texture_cache, viewer_dir, dds_store) {
                texture = Some(img.clone());
                break;
            }
        }

        let mut detail_textures: [Option<Arc<RgbImage>>; 4] = [None, None, None, None];
        let mut detail_scales = [scale_x, scale_x, scale_x, scale_x];
        if let Some(sizes) = lc.get("detailSizes").and_then(Value::as_array) {
            for (i, size) in sizes.iter().take(4).enumerate() {
                if let Some(v) = size.as_f64() {
                    // detailSizes are in world metres — convert to output pixels
                    // using the same world/pixel ratio as scale_x.
                    let scale_m = (v as f32).max(1.0);
                    detail_scales[i] = (scale_m / world_per_px_x.max(1e-6)).max(1.0);
                }
            }
        }
        if let Some(obj) = lc.get("details").and_then(Value::as_object) {
            for (detail_idx, key) in [(0usize, "R"), (1usize, "G"), (2usize, "B"), (3usize, "K")] {
                if let Some(name) = obj.get(key).and_then(Value::as_str) {
                    if let Some(img) = load_cached_texture(name, &mut texture_cache, viewer_dir, dds_store) {
                        detail_textures[detail_idx] = Some(img);
                    }
                }
            }
        }

        mats.push(LcMaterial {
            fallback_seed,
            texture,
            detail_textures,
            detail_scales,
            scale_x,
            scale_y,
        });
    }
    mats
}

fn fallback_noise_byte(seed: u64, gx: u32, gy: u32, chan: u64) -> u8 {
    let mut h = seed
        ^ ((gx as u64) << 32)
        ^ (gy as u64)
        ^ (chan << 56)
        ^ 0x9e37_79b9_7f4a_7c15u64;
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;

    let centered = ((h & 0xff) as i32) - 128;
    let val = 128 + (centered * 24 / 128);
    val.clamp(0, 255) as u8
}

fn sample_texture_scaled(tex: &RgbImage, gx: u32, gy: u32, scale_x: f32, scale_y: f32) -> [u8; 3] {
        let tw = tex.width().max(1);
        let th = tex.height().max(1);
        let sx = ((gx as f32 / scale_x.max(1.0)) * tw as f32) as u32 % tw;
        let sy = ((gy as f32 / scale_y.max(1.0)) * th as f32) as u32 % th;
        let raw = tex.as_raw();
        let o = ((sy * tw + sx) * 3) as usize;
        [raw[o], raw[o + 1], raw[o + 2]]
}

fn sample_lc(m: &LcMaterial, gx: u32, gy: u32) -> [u8; 3] {
    if let Some(tex) = &m.texture {
        return sample_texture_scaled(tex, gx, gy, m.scale_x, m.scale_y);
    }
    [
        fallback_noise_byte(m.fallback_seed, gx, gy, 0),
        fallback_noise_byte(m.fallback_seed, gx, gy, 1),
        fallback_noise_byte(m.fallback_seed, gx, gy, 2),
    ]
}

fn sample_lc_detail(m: &LcMaterial, gx: u32, gy: u32, detail_idx: usize) -> [u8; 3] {
    if detail_idx < m.detail_textures.len() {
        if let Some(tex) = &m.detail_textures[detail_idx] {
            return sample_texture_scaled(tex, gx, gy, m.detail_scales[detail_idx], m.detail_scales[detail_idx]);
        }
    }
    sample_lc(m, gx, gy)
}

fn blend_tile_into(
    tile: &RgbaImage,
    tex2: Option<&RgbaImage>,
    det: [u8; 7],
    lc_mats: &[LcMaterial],
    use_single_lc_detail_mix: bool,
    global_x0: u32,
    global_y0: u32,
    out_w: u32,
    out_h: u32,
    dest: &mut [u8],
    dest_stride: usize,
    dest_x0: usize,
) {
    let det = det;
    let (src_w, src_h) = tile.dimensions();
    let w = out_w.max(1);
    let h = out_h.max(1);

    // Python: has_alpha = tile_arr[:,:,3].min() < 255
    // Only use the alpha channel as a splatmap weight when it actually carries
    // weight data (at least one pixel < 255).
    let tile_raw: &[u8] = tile.as_raw();
    let tile_stride = (src_w * 4) as usize;
    let has_alpha = tile_raw.iter().skip(3).step_by(4).any(|&a| a < 255);
    let (tex2_raw, tex2_w, tex2_h, tex2_stride, has_alpha2): (&[u8], u32, u32, usize, bool) = match tex2 {
        Some(t) => {
            let r: &[u8] = t.as_raw();
            let has = r.iter().skip(3).step_by(4).any(|&a| a < 255);
            (r, t.width(), t.height(), (t.width() * 4) as usize, has)
        }
        None => (&[], 0, 0, 0, false),
    };

    let src_w_max = src_w.saturating_sub(1);
    let src_h_max = src_h.saturating_sub(1);
    let tex2_w_max = tex2_w.saturating_sub(1);
    let tex2_h_max = tex2_h.saturating_sub(1);
    let sx_scale = src_w as f32 / w as f32;
    let sy_scale = src_h as f32 / h as f32;
    let t2x_scale = if tex2_w > 0 { tex2_w as f32 / w as f32 } else { 0.0 };
    let t2y_scale = if tex2_h > 0 { tex2_h as f32 / h as f32 } else { 0.0 };
    let lc_len = lc_mats.len();
    let primary_ids = [det[0] as usize, det[1] as usize, det[2] as usize, det[3] as usize];
    let primary_single_lc = if use_single_lc_detail_mix {
        primary_ids
            .iter()
            .copied()
            .filter(|idx| *idx < lc_len)
            .try_fold(None, |acc: Option<usize>, idx| match acc {
                Some(existing) if existing != idx => None,
                Some(existing) => Some(Some(existing)),
                None => Some(Some(idx)),
            })
            .flatten()
            .filter(|idx| lc_mats[*idx].detail_textures.iter().any(Option::is_some))
    } else {
        None
    };
    let primary_detail_slots = [3usize, 1usize, 0usize, 2usize];

    for y in 0..h {
        let sy = (((y as f32 + 0.5) * sy_scale) as u32).min(src_h_max) as usize;
        let row_off = sy * tile_stride;
        let t2_row_off = if !tex2_raw.is_empty() {
            let t2y = (((y as f32 + 0.5) * t2y_scale) as u32).min(tex2_h_max) as usize;
            t2y * tex2_stride
        } else {
            0
        };
        let gy = global_y0 + y;
        let out_row_off = (y as usize) * dest_stride + dest_x0 * 3;
        for x in 0..w {
            let sx = (((x as f32 + 0.5) * sx_scale) as u32).min(src_w_max) as usize;
            let p_off = row_off + sx * 4;
            let pr = tile_raw[p_off] as i32;
            let pg = tile_raw[p_off + 1] as i32;
            let pa = if has_alpha { tile_raw[p_off + 3] as i32 } else { 0i32 };

            let w0 = (255 - pr - pg - pa).clamp(0, 255);
            let w1 = pg;
            let w2 = pr;
            let w3 = pa;
            let ids = primary_ids;
            let ws = [w0, w1, w2, w3];

            let gx = global_x0 + x;
            let mut cr = 0i32;
            let mut cg = 0i32;
            let mut cb = 0i32;
            let mut tw = 0i32;
            for i in 0..4 {
                let wv = ws[i];
                let idx = ids[i];
                if wv <= 0 || idx >= lc_len {
                    continue;
                }
                let c = if let Some(detail_idx) = primary_single_lc {
                    sample_lc_detail(&lc_mats[detail_idx], gx, gy, primary_detail_slots[i])
                } else {
                    sample_lc(&lc_mats[idx], gx, gy)
                };
                cr += c[0] as i32 * wv;
                cg += c[1] as i32 * wv;
                cb += c[2] as i32 * wv;
                tw += wv;
            }

            // Secondary splat texture (when present) contributes channels G/R/A
            // to det[4]/det[5]/det[6], matching Dagor's paint layer order.
            if !tex2_raw.is_empty() {
                let t2x = (((x as f32 + 0.5) * t2x_scale) as u32).min(tex2_w_max) as usize;
                let p2_off = t2_row_off + t2x * 4;
                let p2r = tex2_raw[p2_off] as i32;
                let p2g = tex2_raw[p2_off + 1] as i32;
                let p2a = if has_alpha2 { tex2_raw[p2_off + 3] as i32 } else { 0i32 };
                let t2_weights = [p2g, p2r, p2a];
                let t2_ids = [det[4] as usize, det[5] as usize, det[6] as usize];
                for i in 0..3 {
                    let wv = t2_weights[i];
                    let idx = t2_ids[i];
                    if wv <= 0 || idx >= lc_len {
                        continue;
                    }
                    let c = sample_lc(&lc_mats[idx], gx, gy);
                    cr += c[0] as i32 * wv;
                    cg += c[1] as i32 * wv;
                    cb += c[2] as i32 * wv;
                    tw += wv;
                }
            }

            let o_off = out_row_off + (x as usize) * 3;
            if tw <= 0 {
                let base_idx = det[0] as usize;
                if base_idx < lc_len {
                    let c = sample_lc(&lc_mats[base_idx], gx, gy);
                    dest[o_off] = c[0];
                    dest[o_off + 1] = c[1];
                    dest[o_off + 2] = c[2];
                } else {
                    dest[o_off] = 20;
                    dest[o_off + 1] = 60;
                    dest[o_off + 2] = 120;
                }
            } else {
                dest[o_off] = (cr / tw).clamp(0, 255) as u8;
                dest[o_off + 1] = (cg / tw).clamp(0, 255) as u8;
                dest[o_off + 2] = (cb / tw).clamp(0, 255) as u8;
            }
        }
    }
}

fn synth_base_lc_tile_into(
    base_idx: usize,
    lc_mats: &[LcMaterial],
    global_x0: u32,
    global_y0: u32,
    out_w: u32,
    out_h: u32,
    dest: &mut [u8],
    dest_stride: usize,
    dest_x0: usize,
) -> bool {
    if base_idx >= lc_mats.len() {
        return false;
    }
    let w = out_w.max(1);
    let h = out_h.max(1);
    for y in 0..h {
        let out_row_off = (y as usize) * dest_stride + dest_x0 * 3;
        for x in 0..w {
            let c = sample_lc(&lc_mats[base_idx], global_x0 + x, global_y0 + y);
            let o_off = out_row_off + (x as usize) * 3;
            dest[o_off] = c[0];
            dest[o_off + 1] = c[1];
            dest[o_off + 2] = c[2];
        }
    }
    true
}

pub fn build_terrain_paint_native(
    map_name: &str,
    bin_data: &[u8],
    dds_store: &DdsStore,
    viewer_dir: &Path,
    landclasses: &[Value],
    tile_set: &TileSet,
    water_mask: &WaterMaskParams,
    sun_azimuth: f64,
    sun_elevation: f64,
    sun_strength: f64,
    hm2_for_detail: Option<Hm2ExtentForPaint>,
    compress_maps: bool,
) -> Result<Option<Value>> {
    let (grid_w, grid_h) = match parse_grid(bin_data) {
        Some(v) => v,
        None => return Ok(None),
    };

    let cells = match parse_tile_cells(bin_data, grid_w * grid_h) {
        Some(v) => v,
        None => return Ok(None),
    };

    if tile_set.images.is_empty() {
        return Ok(None);
    }
    let first = tile_set.images.values().next()
        .context("Tile set unexpectedly empty")?;
    let native_tile_w = first.width();
    let native_tile_h = first.height();

    // Match Python-style quality target: cap the largest output dimension to 8192,
    // with per-cell size in [64, 512] to avoid extreme memory usage.
    let max_dim = (grid_w.max(grid_h) as u32).max(1);
    let target_cell_px = (8192u32 / max_dim).clamp(64, 512);
    let tile_w = target_cell_px;
    let tile_h = target_cell_px;

    // World meters per output pixel (X and Z). Used to convert LC `size`
    // (stored in world metres) into the output-pixel period used by
    // sample_texture_scaled. Falls back to 1:1 if the land cell size is
    // unknown, which preserves legacy behaviour on maps where lndm didn't
    // parse cleanly.
    let (world_per_px_x, world_per_px_z) = parse_lndm_grid(bin_data)
        .map(|g| {
            let cs = g.land_cell_size.max(0.001) as f32;
            (cs / tile_w as f32, cs / tile_h as f32)
        })
        .unwrap_or((1.0, 1.0));

    let mut canvas: RgbImage =
        ImageBuffer::from_pixel((grid_w as u32) * tile_w, (grid_h as u32) * tile_h, Rgb([20, 60, 120]));

    let mut lc_names: Vec<String> = landclasses
        .iter()
        .enumerate()
        .map(|(i, lc)| {
            lc.get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    lc.get("texture")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                })
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("lc_{i}"))
        })
        .collect();
    let mut ocean_like_lc: Vec<bool> = lc_names.iter().map(|name| is_ocean_like_lc_name(name)).collect();
    let mut lc_mats = build_lc_materials(
        landclasses,
        viewer_dir,
        dds_store,
        world_per_px_x,
        world_per_px_z,
    );

    // Some maps reference det indices that exceed extracted landclass blocks
    // (e.g. det contains idx=4 while only 0..3 were decoded). Keep these
    // indices addressable so blending and tile metadata remain stable.
    let max_det_idx = cells
        .iter()
        .flat_map(|c| c.det)
        .filter(|idx| *idx != 0xFF)
        .map(|idx| idx as usize)
        .max();
    if let Some(max_idx) = max_det_idx {
        if max_idx >= lc_mats.len() {
            let base_mat = lc_mats.first().cloned();
            for i in lc_mats.len()..=max_idx {
                lc_names.push(format!("lc_{i}"));
                ocean_like_lc.push(false);
                if let Some(base) = &base_mat {
                    lc_mats.push(LcMaterial {
                        fallback_seed: hash_seed(&format!("lc_{i}")),
                        texture: base.texture.clone(),
                        detail_textures: [None, None, None, None],
                        detail_scales: [base.scale_x, base.scale_x, base.scale_x, base.scale_x],
                        scale_x: base.scale_x,
                        scale_y: base.scale_y,
                    });
                } else {
                    lc_mats.push(LcMaterial {
                        fallback_seed: hash_seed(&format!("lc_{i}")),
                        texture: None,
                        detail_textures: [None, None, None, None],
                        detail_scales: [256.0, 256.0, 256.0, 256.0],
                        scale_x: 256.0,
                        scale_y: 256.0,
                    });
                }
            }
        }
    }
    let cell_lc_indices: Vec<Vec<u8>> = cells
        .iter()
        .map(|c| sanitize_det_indices(c.det, &ocean_like_lc).to_vec())
        .collect();
    let cell_exports: Vec<i64> = cells
        .iter()
        .map(|c| c.export_idx.map(|i| i as i64).unwrap_or(-1))
        .collect();
    let _ = map_name;

    // Per-landclass texture resolution stats. An all-missing profile results
    // in the grey FNV-noise fallback being painted map-wide, which is the
    // visible symptom on maps like air_race_phiphi_islands before a DxP
    // index lookup is wired in.
    let mut lc_texture_resolved = 0usize;
    let mut lc_texture_missing = 0usize;
    let mut lc_missing_names: Vec<String> = Vec::new();
    for (mat, lc) in lc_mats.iter().zip(landclasses.iter()) {
        if mat.texture.is_some() {
            lc_texture_resolved += 1;
        } else {
            lc_texture_missing += 1;
            if lc_missing_names.len() < 16 {
                let n = lc
                    .get("texture")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        lc.get("name")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                    })
                    .unwrap_or("lc")
                    .to_string();
                if !lc_missing_names.contains(&n) {
                    lc_missing_names.push(n);
                }
            }
        }
    }

    // Diagnostic counters for LC index sanity: large invalid counts can hint
    // at mismatched det[] ordering or incomplete landclass extraction.
    let mut lc_ref_total = 0usize;
    let mut lc_ref_invalid = 0usize;
    for cell in &cells {
        for &idx in &cell.det {
            if idx == 0xFF {
                continue;
            }
            lc_ref_total += 1;
            if (idx as usize) >= lc_mats.len() {
                lc_ref_invalid += 1;
            }
        }
    }

    let cell_count = grid_w * grid_h;
    let canvas_stride = canvas.width() as usize * 3;
    let band_stride = canvas_stride * tile_h as usize;
    canvas
        .as_mut()
        .par_chunks_mut(band_stride)
        .enumerate()
        .for_each(|(gy, row_band)| {
            if gy >= grid_h {
                return;
            }
            for gx in 0..grid_w {
                let ci = gy * grid_w + gx;
                if ci >= cell_count {
                    continue;
                }
                let cell = match cells.get(ci) {
                    Some(v) => v,
                    None => continue,
                };
                let det_s = sanitize_det_indices(cell.det, &ocean_like_lc);
                let dest_x0 = gx * tile_w as usize;
                let global_x0 = gx as u32 * tile_w;
                let global_y0 = gy as u32 * tile_h;

                if let Some(tile_idx) = cell.export_idx {
                    if let Some(tile_img) = tile_set.images.get(&tile_idx) {
                        let tex2_img = if cell.has_tex2 {
                            tile_set.images.get(&(tile_idx + 1))
                        } else {
                            None
                        };
                        blend_tile_into(
                            tile_img,
                            tex2_img.map(|v| v.as_ref()),
                            det_s,
                            &lc_mats,
                            true,
                            global_x0,
                            global_y0,
                            tile_w,
                            tile_h,
                            row_band,
                            canvas_stride,
                            dest_x0,
                        );
                        continue;
                    }
                }

                let base_idx = det_s[0] as usize;
                if base_idx != 0xFFusize {
                    let _ = synth_base_lc_tile_into(
                        base_idx,
                        &lc_mats,
                        global_x0,
                        global_y0,
                        tile_w,
                        tile_h,
                        row_band,
                        canvas_stride,
                        dest_x0,
                    );
                }
            }
        });

    // Per docs/ORIENTATION.md §6 the canvas is saved 1:1 with the §5 float
    // frame: cell (tx, tz) sits at canvas pixel (tx*tile_w, tz*tile_h); no
    // rotate180 is applied. All downstream passes operate in this same frame.

    // Feather cell-grid seams to hide the regular cell boundaries produced
    // by per-cell LC splatmap compositing. Operates only on a narrow band
    // around each seam to preserve interior detail.
    blend_cell_seams(&mut canvas, tile_w, tile_h, 6);

    // Apply overview normalmap lighting like Python before water masking.
    apply_normalmap_lighting(
        &mut canvas,
        viewer_dir,
        sun_azimuth,
        sun_elevation,
        sun_strength,
    );

    // Water-mask protection: any cell that is either authored or has a valid
    // base LC (det[0] != 0xFF) is protected from being flooded by the mask.
    // This is a mechanical union — no sparse-grid heuristics.
    let protected_cells: Vec<bool> = cells
        .iter()
        .map(|c| c.export_idx.is_some() || c.det[0] != 0xFF)
        .collect();
    let protection = Some(WaterMaskProtection {
        protected_cells,
        grid_w,
        grid_h,
        tile_w,
        tile_h,
    });
    apply_water_mask(&mut canvas, viewer_dir, water_mask, protection.as_ref());

    let out = if compress_maps {
        viewer_dir.join("terrain_paint.jpg")
    } else {
        viewer_dir.join("terrain_paint.png")
    };
    save_rgb_image(&canvas, &out, compress_maps)
        .with_context(|| format!("Failed to save {}", out.display()))?;

    // Thumbnail: flip then resize for the index card
    let thumb_size = 512u32;
    let (cw, ch) = (canvas.width(), canvas.height());
    let mut thumb = imageops::resize(
        &canvas,
        thumb_size.min(cw),
        thumb_size.min(ch),
        imageops::FilterType::Lanczos3,
    );
    // Apply water masking on thumbnail too so index cards reflect ocean level
    // even when main-canvas masking used sparse-grid protection heuristics.
    apply_water_mask(&mut thumb, viewer_dir, water_mask, None);
    // Display-only exception to the §2 rule: the terrain_paint thumbnail is
    // shown in 2D map cards where users expect north-up (row 0 of the image
    // = world north). Our internal float/PNG frame has pixel (0,0) = world
    // SW, so flip the thumbnail vertically at save-time (east stays on the
    // right). This is isolated to the thumb; full terrain_paint output stays 1:1.
    imageops::flip_vertical_in_place(&mut thumb);
    let thumb_out = if compress_maps {
        viewer_dir.join("terrain_paint_thumb.jpg")
    } else {
        viewer_dir.join("terrain_paint_thumb.png")
    };
    save_rgb_image(&thumb, &thumb_out, compress_maps)
        .with_context(|| format!("Failed to save {}", thumb_out.display()))?;

    // terrain_paint_detail output — HM2-region crop upsampled to 4096×4096
    let mut detail_info: Value = Value::Null;
    if let Some(hm2) = hm2_for_detail {
        // Get canvas world extent from the lndm grid header
        if let Some(grid) = parse_lndm_grid(bin_data) {
            let cell = grid.land_cell_size as f64;
            if cell > 0.0 {
                let x_min = grid.origin_x as f64 * cell;
                let z_min = grid.origin_y as f64 * cell;
                let x_max = x_min + grid.cols as f64 * cell;
                let z_max = z_min + grid.rows as f64 * cell;
                let cw_f = canvas.width() as f64;
                let ch_f = canvas.height() as f64;
                // Per docs/ORIENTATION.md §2 the canvas is 1:1 with the §5
                // float frame: pixel (0,0) = world (x_min, z_min); pixel
                // (cw-1, ch-1) = world (x_max, z_max). Crop HM2 extent directly.
                let px0 = (cw_f * (hm2.world_x0 - x_min) / (x_max - x_min)).clamp(0.0, cw_f) as u32;
                let px1 = (cw_f * (hm2.world_x1 - x_min) / (x_max - x_min)).clamp(0.0, cw_f) as u32;
                let pz0 = (ch_f * (hm2.world_z0 - z_min) / (z_max - z_min)).clamp(0.0, ch_f) as u32;
                let pz1 = (ch_f * (hm2.world_z1 - z_min) / (z_max - z_min)).clamp(0.0, ch_f) as u32;
                let crop_w = px1.saturating_sub(px0);
                let crop_h = pz1.saturating_sub(pz0);
                if crop_w > 0 && crop_h > 0 {
                    const DETAIL_SZ: u32 = 4096;
                    let cropped = imageops::crop_imm(&canvas, px0, pz0, crop_w, crop_h).to_image();
                    let detail_img = imageops::resize(&cropped, DETAIL_SZ, DETAIL_SZ, imageops::FilterType::Lanczos3);
                    let det_path = if compress_maps {
                        viewer_dir.join("terrain_paint_detail.jpg")
                    } else {
                        viewer_dir.join("terrain_paint_detail.png")
                    };
                    save_rgb_image(&detail_img, &det_path, compress_maps)
                        .with_context(|| format!("Failed to save {}", det_path.display()))?;
                    detail_info = json!({
                        "file": if compress_maps { "terrain_paint_detail.jpg" } else { "terrain_paint_detail.png" },
                        "width": DETAIL_SZ,
                        "height": DETAIL_SZ,
                        "hasAlpha": false,
                    });
                }
            }
        }
    }

    let paint_ext = if compress_maps { "jpg" } else { "png" };
    Ok(Some(json!({
        "file": format!("terrain_paint.{}", paint_ext),
        "thumb": format!("terrain_paint_thumb.{}", paint_ext),
        "width": canvas.width(),
        "height": canvas.height(),
        "gridW": grid_w,
        "gridH": grid_h,
        "tileSize": tile_w.max(tile_h),
        "nativeTileSize": native_tile_w.max(native_tile_h),
        "lcNames": lc_names,
        "cellLcIndices": cell_lc_indices,
        "cellExports": cell_exports,
        "lcRefTotal": lc_ref_total,
        "lcRefInvalid": lc_ref_invalid,
        "lcTextureResolved": lc_texture_resolved,
        "lcTextureMissing": lc_texture_missing,
        "lcMissingTextureNames": lc_missing_names,
        "detail": detail_info,
        "source": "native-material-weight-blend"
    })))
}
