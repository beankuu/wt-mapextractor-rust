use std::{fs, path::Path};

use anyhow::{Context, Result};
use image::{imageops, GrayImage, ImageBuffer, Luma, Rgb, RgbImage};
use serde_json::{json, Value};

use crate::export::TileSet;
use crate::util::read_i32_le;

/// Parsed lndm header fields needed for tile_grid assembly and world extent.
#[derive(Debug, Clone)]
pub struct LndmGrid {
    pub cols: u32,
    pub rows: u32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub land_cell_size: f32,
    /// Absolute byte offset in `data` of the tile-data stream (15-byte cell prefixes + DDSx tiles).
    #[allow(dead_code)]
    pub tile_data_abs: usize,
}

pub fn parse_lndm_grid(data: &[u8]) -> Option<LndmGrid> {
    let pos = data.windows(4).position(|w| w == b"lndm")?;
    let mut p = pos + 4;
    let version = read_i32_le(&data, p)?;
    p += 4;
    if version != 4 {
        return None;
    }
    p += 4; // gridCellSize
    let land_cell_size = f32::from_le_bytes(data.get(p..p + 4)?.try_into().ok()?);
    p += 4; // landCellSize
    let map_x = read_i32_le(&data, p)?;
    p += 4;
    let map_y = read_i32_le(&data, p)?;
    p += 4;
    let origin_x = read_i32_le(&data, p)?;
    p += 4;
    let origin_y = read_i32_le(&data, p)?;
    p += 4;
    p += 4; // useTile

    if map_x <= 0 || map_y <= 0 {
        return None;
    }

    let base_ofs = p;
    let det_data_ofs = read_i32_le(&data, base_ofs + 4)?; // detailDataOfs field

    // The tile cell stream is embedded within the detail-data section.
    // Layout at det_data_abs:
    //   +0:                   u32 N = byte-size of the string name blob
    //   +4 .. +19:            4 more u32 fixed header fields (16 bytes)
    //   +20 .. +(20+cells*4): per-cell detail index table (one u32 per grid cell)
    //   +(20+cells*4)+N:      tile cell stream begins here
    let tile_data_abs = if det_data_ofs > 0 {
        let det_abs = ((base_ofs as isize) + (det_data_ofs as isize)) as usize;
        let n = read_i32_le(data, det_abs).unwrap_or(0).max(0) as usize;
        let fixed_hdr = 20 + (map_x as usize) * (map_y as usize) * 4;
        let stream = det_abs + fixed_hdr + n;
        if stream < data.len() { stream } else { 0 }
    } else {
        0
    };

    Some(LndmGrid {
        cols: map_x as u32,
        rows: map_y as u32,
        origin_x,
        origin_y,
        land_cell_size,
        tile_data_abs,
    })
}

fn infer_grid_from_tiles(n: usize) -> LndmGrid {
    if n == 0 {
        return LndmGrid { cols: 16, rows: 16, origin_x: 0, origin_y: 0, land_cell_size: 0.0, tile_data_abs: 0 };
    }
    let side = (n as f64).sqrt().ceil() as u32;
    LndmGrid { cols: side, rows: side, origin_x: 0, origin_y: 0, land_cell_size: 0.0, tile_data_abs: 0 }
}

pub fn build_tile_grid(
    viewer_dir: &Path,
    tile_set: &TileSet,
    map_grid: Option<LndmGrid>,
) -> Result<Option<Value>> {
    if tile_set.info.is_empty() || tile_set.images.is_empty() {
        return Ok(None);
    }

    let first = tile_set.images.values().next().unwrap();
    let tile_w = first.width();
    let tile_h = first.height();

    let grid = map_grid.unwrap_or_else(|| infer_grid_from_tiles(tile_set.info.len()));
    let (cols, rows) = (grid.cols, grid.rows);
    let mut canvas: RgbImage = ImageBuffer::from_pixel(cols * tile_w, rows * tile_h, Rgb([0, 0, 0]));

    for t in &tile_set.info {
        let idx = t.index;
        if idx == 0 {
            continue;
        }
        let cell = match tile_set.idx_to_cell.get(&idx) {
            Some(&c) => c,
            None => idx.saturating_sub(1),
        };
        let gx = (cell as u32) % cols;
        let gy = (cell as u32) / cols;
        if gy >= rows {
            continue;
        }

        if let Some(img) = tile_set.images.get(&idx) {
            // Copy RGB rows directly from the Rgba8 source into the RgbImage
            // canvas — avoids cloning the tile and allocating an RGB copy.
            let src = img.as_raw();
            let src_stride = (tile_w * 4) as usize;
            let dst_stride = (canvas.width() * 3) as usize;
            let base_x = (gx * tile_w) as usize * 3;
            let base_y = (gy * tile_h) as usize;
            let dst = canvas.as_mut();
            for row in 0..(tile_h as usize) {
                let src_off = row * src_stride;
                let dst_off = (base_y + row) * dst_stride + base_x;
                for px in 0..(tile_w as usize) {
                    let s = src_off + px * 4;
                    let d = dst_off + px * 3;
                    dst[d] = src[s];
                    dst[d + 1] = src[s + 1];
                    dst[d + 2] = src[s + 2];
                }
            }
        }
    }

    // Per docs/ORIENTATION.md §6: saved 1:1 with the §5 float frame.
    let out = viewer_dir.join("tile_grid.png");
    canvas.save(&out)
        .with_context(|| format!("Failed to save {}", out.display()))?;

    let (canvas_w, canvas_h) = (canvas.width(), canvas.height());

    // World extent from lndm origin + grid dimensions
    let world_extent: Option<[f64; 4]> = if grid.land_cell_size > 0.0 {
        let lcs = grid.land_cell_size as f64;
        let x0 = grid.origin_x as f64 * lcs;
        let z0 = grid.origin_y as f64 * lcs;
        let x1 = x0 + cols as f64 * lcs;
        let z1 = z0 + rows as f64 * lcs;
        Some([x0, z0, x1, z1])
    } else {
        None
    };

    Ok(Some(json!({
        "file": "tile_grid.png",
        "cols": cols,
        "rows": rows,
        "tileWidth": tile_w,
        "tileHeight": tile_h,
        "tileCount": tile_set.info.len(),
        "width": canvas_w,
        "height": canvas_h,
        "world_extent": world_extent,
    })))
}

pub fn build_heightmap_fallback(viewer_dir: &Path) -> Result<Value> {
    let colormap = {
        let jpg = viewer_dir.join("colormap.jpg");
        if jpg.exists() { jpg } else { viewer_dir.join("colormap.png") }
    };
    let normalmap = viewer_dir.join("normalmap.png");
    let out = viewer_dir.join("heightmap.png");

    if colormap.exists() {
        let img = image::open(&colormap)
            .with_context(|| format!("Failed to open {}", colormap.display()))?
            .to_luma8();
        let blur = imageops::blur(&img, 5.0);
        blur.save(&out)
            .with_context(|| format!("Failed to save {}", out.display()))?;
        return Ok(json!({
            "file": "heightmap.png",
            "width": blur.width(),
            "height": blur.height(),
            "pseudo": true,
            "source": "colormap"
        }));
    }

    if normalmap.exists() {
        let img = image::open(&normalmap)
            .with_context(|| format!("Failed to open {}", normalmap.display()))?
            .to_rgb8();
        let (w, h) = img.dimensions();
        let mut g: GrayImage = ImageBuffer::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let b = img.get_pixel(x, y)[2];
                g.put_pixel(x, y, Luma([b]));
            }
        }
        let blur = imageops::blur(&g, 5.0);
        blur.save(&out)
            .with_context(|| format!("Failed to save {}", out.display()))?;
        return Ok(json!({
            "file": "heightmap.png",
            "width": blur.width(),
            "height": blur.height(),
            "pseudo": true,
            "source": "normalmap-blue"
        }));
    }

    let flat: GrayImage = ImageBuffer::from_pixel(512, 512, Luma([128]));
    flat.save(&out)
        .with_context(|| format!("Failed to save {}", out.display()))?;
    Ok(json!({
        "file": "heightmap.png",
        "width": 512,
        "height": 512,
        "pseudo": true,
        "source": "flat"
    }))
}

pub fn build_terrain_paint_fallback(viewer_dir: &Path) -> Result<Option<Value>> {
    build_terrain_paint_fallback_impl(viewer_dir, false)
}

fn build_terrain_paint_fallback_impl(viewer_dir: &Path, prefer_colormap: bool) -> Result<Option<Value>> {
    let tile_grid = viewer_dir.join("tile_grid.png");
    let colormap = {
        let jpg = viewer_dir.join("colormap.jpg");
        if jpg.exists() { jpg } else { viewer_dir.join("colormap.png") }
    };
    let out = viewer_dir.join("terrain_paint.png");
    let thumb_path = viewer_dir.join("terrain_paint_thumb.png");

    let manifest_path = viewer_dir.join("manifest.json");
    let manifest_json: Option<Value> = fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let wl_opt = manifest_json
        .as_ref()
        .and_then(|m| m.get("waterLevel"))
        .and_then(Value::as_f64);
    let hm_min_opt = manifest_json
        .as_ref()
        .and_then(|m| m.get("heightmap"))
        .and_then(|h| h.get("height_min_m"))
        .and_then(Value::as_f64);
    let hm_max_opt = manifest_json
        .as_ref()
        .and_then(|m| m.get("heightmap"))
        .and_then(|h| h.get("height_max_m"))
        .and_then(Value::as_f64);

    let save_thumb = |img: &image::RgbImage| -> Result<()> {
        let thumb_size = 512u32;
        let (cw, ch) = (img.width(), img.height());
        let mut thumb = image::imageops::resize(
            img,
            thumb_size.min(cw.max(1)),
            thumb_size.min(ch.max(1)),
            image::imageops::FilterType::Lanczos3,
        );

        // Keep fallback thumbs consistent with full terrain-paint output:
        // tint likely ocean pixels using heightmap + water metadata.
        if let (Some(wl), Some(hmin), Some(hmax)) = (wl_opt, hm_min_opt, hm_max_opt) {
            let hrange = (hmax - hmin).max(0.0);
            let below_terrain = wl < hmin;
            if !below_terrain {
                let hm_path = viewer_dir.join("heightmap.png");
                if hm_path.exists() {
                    if let Ok(hm) = image::open(&hm_path).map(|i| i.to_luma8()) {
                        let (tw, th) = (thumb.width(), thumb.height());
                        let (hw, hh) = (hm.width(), hm.height());
                        let water_threshold = if hrange > 0.0 {
                            wl - (hrange / 255.0)
                        } else {
                            wl
                        };
                        for y in 0..th {
                            for x in 0..tw {
                                let hx = (x as u64 * hw as u64 / tw as u64) as u32;
                                let hy = (y as u64 * hh as u64 / th as u64) as u32;
                                let hv = hm.get_pixel(hx.min(hw.saturating_sub(1)), hy.min(hh.saturating_sub(1)))[0] as f64 / 255.0;
                                let hm_m = hmin + hv * hrange;
                                if hm_m <= water_threshold {
                                    let p = thumb.get_pixel_mut(x, y);
                                    let r = ((p[0] as u16 * 30 + 13 * 70) / 100) as u8;
                                    let g = ((p[1] as u16 * 30 + 79 * 70) / 100) as u8;
                                    let b = ((p[2] as u16 * 30 + 139 * 70) / 100) as u8;
                                    *p = Rgb([r, g, b]);
                                }
                            }
                        }
                    }
                }
            }
        }

        thumb.save(&thumb_path)
            .with_context(|| format!("Failed to save {}", thumb_path.display()))?;
        Ok(())
    };

    let try_colormap = |out: &Path| -> Result<Option<Value>> {
        if !colormap.exists() { return Ok(None); }
        let img = image::open(&colormap)
            .with_context(|| format!("Failed to open {}", colormap.display()))?
            .to_rgb8();
        img.save(out)
            .with_context(|| format!("Failed to save {}", out.display()))?;
        save_thumb(&img)?;
        Ok(Some(json!({
            "file": "terrain_paint.png",
            "thumb": "terrain_paint_thumb.png",
            "width": img.width(),
            "height": img.height(),
            "source": "colormap"
        })))
    };
    let try_tilegrid = |out: &Path| -> Result<Option<Value>> {
        if !tile_grid.exists() { return Ok(None); }
        let img = image::open(&tile_grid)
            .with_context(|| format!("Failed to open {}", tile_grid.display()))?
            .to_rgb8();
        img.save(out)
            .with_context(|| format!("Failed to save {}", out.display()))?;
        save_thumb(&img)?;
        Ok(Some(json!({
            "file": "terrain_paint.png",
            "thumb": "terrain_paint_thumb.png",
            "width": img.width(),
            "height": img.height(),
            "source": "tile_grid"
        })))
    };

    if prefer_colormap {
        if let Some(v) = try_colormap(&out)? { return Ok(Some(v)); }
        if let Some(v) = try_tilegrid(&out)? { return Ok(Some(v)); }
    } else {
        if let Some(v) = try_tilegrid(&out)? { return Ok(Some(v)); }
        if let Some(v) = try_colormap(&out)? { return Ok(Some(v)); }
    }

    Ok(None)
}
