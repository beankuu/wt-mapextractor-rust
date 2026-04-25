use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::hash::{Hash, Hasher};

use anyhow::Result;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageBuffer, Rgb, Rgba, RgbaImage};
use rayon::prelude::*;
use serde_json::{json, Value};

use crate::util::{decode_dds_bytes, read_u32_le};

const COLORMAP_JPEG_QUALITY: u8 = 82;

/// In-memory DDS store: stem name (no extension) → raw DDS bytes.
pub type DdsStore = HashMap<String, Vec<u8>>;

#[derive(Debug, Clone)]
pub struct TileInfo {
    pub index: usize,
    pub image: String,
}

/// In-memory collection of decoded tile images.
/// Maps export_idx → RGBA image, with cell_index mapping.
pub struct TileSet {
    pub images: HashMap<usize, Arc<RgbaImage>>,
    pub info: Vec<TileInfo>,
    /// Maps export_idx → cell_index (grid position)
    pub idx_to_cell: HashMap<usize, usize>,
}

fn read_dds_wh_bytes(data: &[u8]) -> Option<(u32, u32, String)> {
    if data.len() < 128 || &data[0..4] != b"DDS " {
        return None;
    }
    let h = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
    let w = u32::from_le_bytes([data[16], data[17], data[18], data[19]]);
    let f = String::from_utf8_lossy(&data[84..88]).replace('\0', "");
    Some((w, h, f))
}

fn is_normalmap(img: &DynamicImage) -> bool {
    let rgba = img.to_rgba8();
    if rgba.width() == 0 || rgba.height() == 0 {
        return false;
    }
    let mut rs = 0f64;
    let mut bs = 0f64;
    let mut asq = 0f64;
    let mut ac = 0f64;
    let n = (rgba.width() as u64 * rgba.height() as u64) as f64;
    for p in rgba.pixels() {
        rs += p[0] as f64;
        bs += p[2] as f64;
        ac += p[3] as f64;
    }
    let amean = ac / n;
    for p in rgba.pixels() {
        let d = p[3] as f64 - amean;
        asq += d * d;
    }
    let r_mean = rs / n;
    let b_mean = bs / n;
    let a_std = (asq / n).sqrt();
    r_mean < 10.0 && b_mean > 220.0 && a_std > 3.0
}

fn dxt5nm_to_normalmap(img: &DynamicImage) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut out = ImageBuffer::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p: &Rgba<u8> = rgba.get_pixel(x, y);
            let nx = p[3] as f32 / 127.5 - 1.0;
            let ny = p[1] as f32 / 127.5 - 1.0;
            let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
            let r = ((nx * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            let g = ((ny * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            let b = ((nz * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            out.put_pixel(x, y, Rgb([r, g, b]));
        }
    }
    out
}

pub fn export_overview(map_name: &str, dds_store: &DdsStore, viewer_dir: &Path) -> Result<Option<Value>> {
    let prefix = format!("{map_name}_");
    let mut candidates: Vec<(&str, u32, u32, String)> = dds_store
        .iter()
        .filter(|(stem, _)| stem.starts_with(&prefix))
        .filter_map(|(stem, bytes)| {
            let (w, h, fmt) = read_dds_wh_bytes(bytes)?;
            Some((stem.as_str(), w, h, fmt))
        })
        .collect();
    candidates.sort_by_key(|(stem, _, _, _)| *stem);

    let mut normal_candidate: Option<(u32, u32, ImageBuffer<Rgb<u8>, Vec<u8>>)> = None;

    for (stem, w, h, fmt) in &candidates {
        if *w < 1024 || *h < 1024 {
            continue;
        }

        let bytes = match dds_store.get(*stem) {
            Some(b) => b,
            None => continue,
        };
        let img = match decode_dds_bytes(bytes) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if fmt == "DXT5" && is_normalmap(&img) {
            if normal_candidate.is_none() {
                normal_candidate = Some((*w, *h, dxt5nm_to_normalmap(&img)));
            }
            continue;
        }

        let rgb = img.to_rgb8();
        let out_path = viewer_dir.join("colormap.jpg");
        let f = std::fs::File::create(&out_path)?;
        let mut enc = JpegEncoder::new_with_quality(std::io::BufWriter::new(f), COLORMAP_JPEG_QUALITY);
        enc.encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )?;
        return Ok(Some(json!({
            "file": "colormap.jpg",
            "width": w,
            "height": h,
            "format": fmt
        })));
    }

    if let Some((_w, _h, mut n)) = normal_candidate {
        for p in n.pixels_mut() {
            p[0] = 255u8.saturating_sub(p[0]);
            p[1] = 255u8.saturating_sub(p[1]);
        }
        let _ = n.save(viewer_dir.join("normalmap.png"));
    }

    Ok(None)
}

pub fn export_tiles(map_name: &str, dds_store: &DdsStore, bin_data: Option<&[u8]>) -> Result<TileSet> {
    let prefix = format!("{map_name}_");
    let mut jobs: Vec<(&str, usize)> = dds_store
        .keys()
        .filter_map(|stem| {
            if !stem.starts_with(&prefix) {
                return None;
            }
            let idx = stem.rsplit('_').next()?.parse::<usize>().ok()?;
            if idx == 0 {
                return None;
            }
            Some((stem.as_str(), idx))
        })
        .collect();
    jobs.sort_by_key(|(_, idx)| *idx);

    let results: Vec<Option<(usize, Arc<RgbaImage>)>> = jobs
        .par_iter()
        .map(|(stem, idx)| {
            let bytes = dds_store.get(*stem)?;
            let img = decode_dds_bytes(bytes).ok()?.to_rgba8();
            Some((*idx, Arc::new(img)))
        })
        .collect();

    let mut images = HashMap::new();
    let mut info = Vec::new();
    for r in results.into_iter().flatten() {
        let (idx, img) = r;
        images.insert(idx, img);
        info.push(TileInfo {
            index: idx,
            image: format!("tile_{idx:03}.dds"),
        });
    }
    info.sort_by_key(|t| t.index);

    let idx_to_cell = if let Some(bin) = bin_data {
        parse_tile_cells_mapping(bin, info.len())
            .unwrap_or_else(|| build_naive_mapping(info.len()))
    } else {
        build_naive_mapping(info.len())
    };

    Ok(TileSet {
        images,
        info,
        idx_to_cell,
    })
}

pub fn export_materials(
    map_name: &str,
    dds_store: &DdsStore,
    viewer_dir: &Path,
    allowed_names: Option<&BTreeSet<String>>,
) -> Result<Vec<Value>> {
    let shared_mat_dir = viewer_dir
        .parent()
        .map(|p| p.join("shared").join("mat"))
        .unwrap_or_else(|| viewer_dir.join("shared").join("mat"));
    let _ = fs::create_dir_all(&shared_mat_dir);

    let prefix = format!("{map_name}_");
    let mut jobs: Vec<(&str, &Vec<u8>)> = dds_store
        .iter()
        .filter(|(stem, _)| {
            let is_tile = stem.starts_with(&prefix)
                && stem.rsplit('_').next().and_then(|n| n.parse::<usize>().ok()).is_some();
            let is_na16 = stem.to_ascii_lowercase().ends_with("_na16");
            let allowed = allowed_names.map(|set| set.contains(*stem)).unwrap_or(true);
            allowed && !is_tile && !stem.starts_with("tile_") && !is_na16
        })
        .map(|(stem, bytes)| (stem.as_str(), bytes))
        .collect();
    jobs.sort_by_key(|(stem, _)| *stem);

    let out: Vec<Value> = jobs
        .par_iter()
        .filter_map(|(stem, dds_bytes)| {
        let img = decode_dds_bytes(dds_bytes).ok()?;
        let rgba = img.to_rgba8();
        let bytes = rgba.as_raw();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        let sig = hasher.finish();

        let base_name = format!("{stem}.webp");
        let mut shared_name = base_name.clone();
        let mut shared_path = shared_mat_dir.join(&shared_name);
        if shared_path.exists() {
            let same = fs::metadata(&shared_path).ok().map(|m| m.len()).unwrap_or(0) == bytes.len() as u64;
            if !same {
                shared_name = format!("{}_{:016x}.webp", stem, sig);
                shared_path = shared_mat_dir.join(&shared_name);
            }
        }

        if !shared_path.exists() {
            img.save(&shared_path).ok()?;
        }

        let file_ref = format!("../shared/mat/{shared_name}");
        Some(json!({
            "name": stem,
            "file": file_ref,
            "width": img.width(),
            "height": img.height()
        }))
        })
        .collect();

    Ok(out)
}

// ── Tile cell mapping helpers ──────────────────────────────────────

/// Parse lndm header to extract the absolute byte offset of the detail-data
/// section and the total grid cell count (cols × rows).
fn parse_lndm_for_tile_cells(data: &[u8]) -> Option<(usize, usize)> {
    let pos = data.windows(4).position(|w| w == b"lndm")?;
    let mut p = pos + 4;
    let version = i32::from_le_bytes(data.get(p..p + 4)?.try_into().ok()?);
    p += 4;
    if version != 4 {
        return None;
    }
    p += 4; // gridCellSize
    p += 4; // landCellSize
    let map_x = i32::from_le_bytes(data.get(p..p + 4)?.try_into().ok()?);
    p += 4;
    let map_y = i32::from_le_bytes(data.get(p..p + 4)?.try_into().ok()?);
    p += 4;
    p += 4; // originCellX
    p += 4; // originCellY
    p += 4; // useTile

    let base_ofs = p;
    let det_data_ofs = i32::from_le_bytes(data.get(base_ofs + 4..base_ofs + 8)?.try_into().ok()?) as isize;
    if det_data_ofs <= 0 || map_x <= 0 || map_y <= 0 {
        return None;
    }
    let det_abs = (base_ofs as isize + det_data_ofs) as usize;
    // Tile cell stream starts after the detail blob's fixed header + string name blob.
    // Layout: [u32 N][16 bytes hdr][map_x*map_y*4 cell table][N bytes strings][tile stream]
    let n = u32::from_le_bytes(data.get(det_abs..det_abs + 4)?.try_into().ok()?) as usize;
    let fixed_hdr = 20 + (map_x as usize) * (map_y as usize) * 4;
    let det_start = det_abs + fixed_hdr + n;
    if det_start >= data.len() {
        return None;
    }
    Some((det_start, (map_x * map_y) as usize))
}

/// Parse binary tile cell structure to build export_idx → cell_index mapping.
/// Correctly handles:
///  - leading empty cells (left-gap / top-gap offset) by scanning backwards
///    from the first populated cell's prefix up to the detail-data section start.
///  - tex2 dual-texture cells (a populated cell with a tex2 block exports two
///    DDSx entries, so the export_idx must advance by 2 for that cell).
fn parse_tile_cells_mapping(data: &[u8], _num_tiles: usize) -> Option<HashMap<usize, usize>> {
    const DDSX_MAGIC: &[u8; 4] = b"DDSx";
    const CELL_PREFIX: usize = 15;

    let (det_start, num_cells) = parse_lndm_for_tile_cells(data).unwrap_or((0, 0));
    if det_start == 0 || num_cells == 0 || det_start + CELL_PREFIX > data.len() {
        return None;
    }

    // Walk tile cells directly from lndm detailData section (v4 layout):
    // [det:7][totalLen:4][tex2Ofs:4][DDSx...totalLen]
    let mut walk_pos = det_start;
    let mut export_idx = 1usize;
    let mut cell_idx = 0usize;
    let mut mapping = HashMap::new();

    loop {
        if cell_idx >= num_cells || walk_pos + CELL_PREFIX > data.len() {
            break;
        }
        let total_len = match read_u32_le(data, walk_pos + 7) {
            Some(v) => v as usize,
            None => break,
        };
        let tex2_off = match read_u32_le(data, walk_pos + 11) {
            Some(v) => v as usize,
            None => break,
        };
        let ddsx_start = walk_pos + CELL_PREFIX;

        if total_len == 0 {
            // Empty cell
            walk_pos = ddsx_start;
        } else {
            // Populated cell — verify magic
            if ddsx_start + 4 > data.len() || &data[ddsx_start..ddsx_start + 4] != DDSX_MAGIC {
                break;
            }
            mapping.insert(export_idx, cell_idx);
            export_idx += 1;
            // If the cell has a tex2 block it exports a second DDSx image.
            // Validate nested DDSx header to avoid false positives.
            let has_tex2 = tex2_off > 0
                && tex2_off + 4 <= total_len
                && ddsx_start + tex2_off + 4 <= data.len()
                && &data[ddsx_start + tex2_off..ddsx_start + tex2_off + 4] == DDSX_MAGIC;
            if has_tex2 {
                export_idx += 1;
            }
            walk_pos = ddsx_start + total_len;
        }
        cell_idx += 1;
    }

    if mapping.is_empty() {
        return None;
    }

    Some(mapping)
}

/// Fallback mapping: simple idx-1 for maps without proper binary data.
fn build_naive_mapping(num_tiles: usize) -> HashMap<usize, usize> {
    (1..=num_tiles).map(|idx| (idx, idx - 1)).collect()
}
