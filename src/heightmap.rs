use std::path::Path;

use anyhow::{Context, Result};
use image::{ImageBuffer, Luma, Rgb, RgbImage};
use serde_json::{json, Value};

use crate::util::oodle_decompress;

const HMAP_WIDTH_BITS: u32 = 24;
const HMAP_WIDTH_MASK: i32 = (1 << HMAP_WIDTH_BITS) - 1;

fn read_nonnegative_i32_as_usize(data: &[u8], pos: usize) -> Option<usize> {
    let value = i32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?);
    usize::try_from(value).ok()
}

#[derive(Debug, Clone)]
struct Hm2Header {
    h_min: f32,
    h_scale_raw: f32,
    width: usize,
    height: usize,
    version: i32,
    /// Metres per HM2 pixel
    cell_size: f32,
    /// World X position of HM2 origin (pixel 0,0)
    wpo_x: f32,
    /// World Z position of HM2 origin (pixel 0,0)
    wpo_y: f32,
}

fn find_hm2_block(data: &[u8]) -> Option<(usize, usize)> {
    find_all_hm2_blocks(data).into_iter().next()
}

/// Return every `\0HM2` block in the DBLD container, in file order.
fn find_all_hm2_blocks(data: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if data.len() < 12 || &data[0..4] != b"DBLD" {
        return out;
    }
    let mut pos = 0x0cusize;
    while pos + 8 <= data.len() {
        let lf = match data.get(pos..pos + 4).and_then(|s| s.try_into().ok()) {
            Some(v) => u32::from_le_bytes(v),
            None => break,
        };
        let blen = (lf & 0x3fff_ffff) as usize;
        if blen < 4 || blen > data.len().saturating_sub(pos) {
            break;
        }
        let tag = match data.get(pos + 4..pos + 8) {
            Some(v) => v,
            None => break,
        };
        if tag == b"\0HM2" {
            out.push((pos + 8, blen - 4));
        }
        if tag.contains(&b'E') && tag.contains(&b'N') && tag.contains(&b'D') {
            break;
        }
        pos += 4 + blen;
    }
    out
}

fn zstd_decompress_exact(comp: &[u8], expected_size: usize) -> Option<Vec<u8>> {
    let out = zstd::stream::decode_all(comp).ok()?;
    if out.len() < expected_size {
        return None;
    }
    Some(out)
}

fn extract_land_ray_tracer_dump(data: &[u8]) -> Option<(Vec<u8>, f32)> {
    let lndm_pos = data.windows(4).position(|w| w == b"lndm")?;
    let mut p = lndm_pos + 4;
    let version = i32::from_le_bytes(data.get(p..p + 4)?.try_into().ok()?);
    p += 4;
    if version != 4 {
        return None;
    }

    let grid_cell_size = f32::from_le_bytes(data.get(p..p + 4)?.try_into().ok()?);
    p += 4;
    p += 4; // landCellSize
    p += 4 + 4; // mapSizeX/Y
    p += 4 + 4 + 4; // originX/Y/useTile

    let base_ofs = p;
    p += 4; // meshMapOfs
    p += 4; // detailDataOfs
    p += 4; // tileDataOfs
    let ray_tracer_ofs = i32::from_le_bytes(data.get(p..p + 4)?.try_into().ok()?) as isize;
    if ray_tracer_ofs <= 0 {
        return None;
    }

    let abs_rt = (base_ofs as isize + ray_tracer_ofs) as usize;
    if abs_rt < 4 || abs_rt >= data.len() {
        return None;
    }
    let header_pos = abs_rt - 4;
    let v = u32::from_le_bytes(data.get(header_pos..header_pos + 4)?.try_into().ok()?);
    let compr_flags = (v >> 30) & 3;
    let block_len = (v & 0x3fff_ffff) as usize;
    if abs_rt + block_len > data.len() {
        return None;
    }

    let rt_dump = match compr_flags {
        2 => {
            let decomp_size = u32::from_le_bytes(data.get(abs_rt..abs_rt + 4)?.try_into().ok()?) as usize;
            let comp = data.get(abs_rt + 4..abs_rt + block_len)?;
            oodle_decompress(comp, decomp_size)?
        }
        1 => {
            let comp = data.get(abs_rt..abs_rt + block_len)?;
            zstd::stream::decode_all(comp).ok()?
        }
        0 => data.get(abs_rt..abs_rt + block_len)?.to_vec(),
        _ => return None,
    };

    Some((rt_dump, grid_cell_size))
}

/// Fill NaN pixels using a pyramid pull-push approach, matching Python's
/// `_pyramid_fill_nan`.  Valid pixels are averaged into progressively coarser
/// levels; the coarsest remaining NaN cells are filled by simple dilation;
/// then we upsample back, filling only NaN pixels at each level from the
/// bilinearly-interpolated coarser level.  This gives smooth ocean-to-land
/// transitions instead of the hard cliff that a constant fill produces.
fn fill_nan_pyramid(hm: &mut Vec<f64>, dim: usize) {
    if !hm.iter().any(|v| v.is_nan()) {
        return;
    }
    if hm.iter().all(|v| v.is_nan()) {
        for v in hm.iter_mut() {
            *v = 0.0;
        }
        return;
    }

    // Build pyramid: each level is half the resolution, averaging only valid pixels.
    let mut pyramid: Vec<(Vec<f64>, usize)> = vec![(hm.clone(), dim)];
    loop {
        let (prev_data, prev_dim) = pyramid.last().unwrap();
        if *prev_dim <= 4 {
            break;
        }
        let next_dim = prev_dim / 2;
        let mut next_data = vec![f64::NAN; next_dim * next_dim];
        for nz in 0..next_dim {
            for nx in 0..next_dim {
                let mut sum = 0.0f64;
                let mut cnt = 0u32;
                for dz in 0..2usize {
                    for dx in 0..2usize {
                        let sz = nz * 2 + dz;
                        let sx = nx * 2 + dx;
                        if sz < *prev_dim && sx < *prev_dim {
                            let v = prev_data[sz * prev_dim + sx];
                            if !v.is_nan() {
                                sum += v;
                                cnt += 1;
                            }
                        }
                    }
                }
                if cnt > 0 {
                    next_data[nz * next_dim + nx] = sum / cnt as f64;
                }
            }
        }
        pyramid.push((next_data, next_dim));
    }

    // Fill coarsest level with simple iterative dilation (grid is tiny here).
    {
        let (coarse_data, coarse_dim) = pyramid.last_mut().unwrap();
        let cd = *coarse_dim;
        for _ in 0..cd * 4 {
            if !coarse_data.iter().any(|v| v.is_nan()) {
                break;
            }
            let snap = coarse_data.clone();
            for z in 0..cd {
                for x in 0..cd {
                    let i = z * cd + x;
                    if !snap[i].is_nan() {
                        continue;
                    }
                    let mut sum = 0.0f64;
                    let mut cnt = 0u32;
                    if x > 0 && !snap[i - 1].is_nan() { sum += snap[i - 1]; cnt += 1; }
                    if x + 1 < cd && !snap[i + 1].is_nan() { sum += snap[i + 1]; cnt += 1; }
                    if z > 0 && !snap[i - cd].is_nan() { sum += snap[i - cd]; cnt += 1; }
                    if z + 1 < cd && !snap[i + cd].is_nan() { sum += snap[i + cd]; cnt += 1; }
                    if cnt > 0 {
                        coarse_data[i] = sum / cnt as f64;
                    }
                }
            }
        }
        for v in coarse_data.iter_mut() {
            if v.is_nan() {
                *v = 0.0;
            }
        }
    }

    // Upsample: for each level from coarsest to finest, fill NaN in the finer
    // level using bilinear interpolation from the fully-filled coarser level.
    let num_levels = pyramid.len();
    for lvl in (0..num_levels - 1).rev() {
        let coarser_data = pyramid[lvl + 1].0.clone();
        let cd = pyramid[lvl + 1].1;
        let (fine_data, fd) = &mut pyramid[lvl];
        let fd = *fd;
        for fz in 0..fd {
            for fx in 0..fd {
                let i = fz * fd + fx;
                if !fine_data[i].is_nan() {
                    continue;
                }
                // Map fine pixel centre to coarser grid coordinate.
                let cx = (fx as f64 + 0.5) / fd as f64 * cd as f64 - 0.5;
                let cz = (fz as f64 + 0.5) / fd as f64 * cd as f64 - 0.5;
                let x0 = (cx.floor() as isize).clamp(0, cd as isize - 1) as usize;
                let z0 = (cz.floor() as isize).clamp(0, cd as isize - 1) as usize;
                let x1 = (x0 + 1).min(cd - 1);
                let z1 = (z0 + 1).min(cd - 1);
                let ffx = (cx - x0 as f64).clamp(0.0, 1.0);
                let ffz = (cz - z0 as f64).clamp(0.0, 1.0);
                let h00 = coarser_data[z0 * cd + x0];
                let h10 = coarser_data[z0 * cd + x1];
                let h01 = coarser_data[z1 * cd + x0];
                let h11 = coarser_data[z1 * cd + x1];
                fine_data[i] = h00 * (1.0 - ffx) * (1.0 - ffz)
                    + h10 * ffx * (1.0 - ffz)
                    + h01 * (1.0 - ffx) * ffz
                    + h11 * ffx * ffz;
            }
        }
    }

    hm.copy_from_slice(&pyramid[0].0);
}

fn rasterize_land_ray_tracer(rt_dump: &[u8], grid_cell_size: f32) -> Option<LrFloatMap> {
    let lt_sig = rt_dump.windows(6).position(|w| w == b"LTdump")?;
    let mut pos = lt_sig + 6;

    let num_cx = read_nonnegative_i32_as_usize(rt_dump, pos)?;
    pos += 4;
    let num_cy = read_nonnegative_i32_as_usize(rt_dump, pos)?;
    pos += 4;
    let cell_size = f32::from_le_bytes(rt_dump.get(pos..pos + 4)?.try_into().ok()?);
    pos += 4;

    let rt_offset_x = f32::from_le_bytes(rt_dump.get(pos..pos + 4)?.try_into().ok()?);
    let _rt_offset_y = f32::from_le_bytes(rt_dump.get(pos + 4..pos + 8)?.try_into().ok()?);
    let rt_offset_z = f32::from_le_bytes(rt_dump.get(pos + 8..pos + 12)?.try_into().ok()?);
    pos += 12;

    pos += 12; // bmin
    pos += 12; // bmax

    let cells_count = read_nonnegative_i32_as_usize(rt_dump, pos)?;
    pos += 4;
    let cells_bytes = cells_count.checked_mul(64)?;
    let cells_end = pos.checked_add(cells_bytes)?;
    let cells_raw = rt_dump.get(pos..cells_end)?;
    pos = cells_end;

    let grid_count = read_nonnegative_i32_as_usize(rt_dump, pos)?;
    let grid_bytes = grid_count.checked_mul(4)?;
    pos = pos.checked_add(4)?.checked_add(grid_bytes)?;
    if pos > rt_dump.len() {
        return None;
    }

    let ght_count = read_nonnegative_i32_as_usize(rt_dump, pos)?;
    let ght_bytes = ght_count.checked_mul(4)?;
    pos = pos.checked_add(4)?.checked_add(ght_bytes)?;
    if pos > rt_dump.len() {
        return None;
    }

    let faces_count = read_nonnegative_i32_as_usize(rt_dump, pos)?;
    pos += 4;
    let faces_bytes = faces_count.checked_mul(2)?;
    let faces_end = pos.checked_add(faces_bytes)?;
    let faces_raw = rt_dump.get(pos..faces_end)?;
    pos = faces_end;

    let verts_count = read_nonnegative_i32_as_usize(rt_dump, pos)?;
    pos += 4;
    let verts_bytes = verts_count.checked_mul(8)?;
    let verts_end = pos.checked_add(verts_bytes)?;
    let verts_raw = rt_dump.get(pos..verts_end)?;
    let _ = verts_end;

    let mut faces = Vec::with_capacity(faces_count);
    for i in 0..faces_count {
        let o = i * 2;
        faces.push(u16::from_le_bytes([faces_raw[o], faces_raw[o + 1]]) as usize);
    }

    let mut verts = Vec::with_capacity(verts_count);
    for i in 0..verts_count {
        let o = i * 8;
        // LandRayTracer vertices are packed u16 values. Cell scale converts them
        // directly into world space; they are not normalized here.
        let x = u16::from_le_bytes([verts_raw[o], verts_raw[o + 1]]) as f64;
        let y = u16::from_le_bytes([verts_raw[o + 2], verts_raw[o + 3]]) as f64;
        let z = u16::from_le_bytes([verts_raw[o + 4], verts_raw[o + 5]]) as f64;
        verts.push((x, y, z));
    }

    let mut cells = Vec::with_capacity(cells_count);
    for i in 0..cells_count {
        let co = i * 64;
        let ox = f32::from_le_bytes(cells_raw.get(co..co + 4)?.try_into().ok()?);
        let oy = f32::from_le_bytes(cells_raw.get(co + 4..co + 8)?.try_into().ok()?);
        let oz = f32::from_le_bytes(cells_raw.get(co + 8..co + 12)?.try_into().ok()?);
        let sx = f32::from_le_bytes(cells_raw.get(co + 16..co + 20)?.try_into().ok()?);
        let sy = f32::from_le_bytes(cells_raw.get(co + 20..co + 24)?.try_into().ok()?);
        let sz = f32::from_le_bytes(cells_raw.get(co + 24..co + 28)?.try_into().ok()?);
        let f_start = u32::from_le_bytes(cells_raw.get(co + 48..co + 52)?.try_into().ok()?) as usize;
        let v_start = u32::from_le_bytes(cells_raw.get(co + 52..co + 56)?.try_into().ok()?) as usize;
        cells.push(((ox, oy, oz), (sx, sy, sz), f_start, v_start));
    }

    let mut dim = if grid_cell_size > 0.0 {
        // Natural rasterisation resolution = LR cells × (cell_size /
        // grid_cell_size). On large maps (e.g. avg_tunisia_desert at
        // 65 536 m wide) the LR source is 2048 px native, which forced
        // 32 m/pixel on the base heightmap and left everything outside
        // the HM2 detail box visibly coarse. We super-sample the LR mesh
        // to twice its native density so bilinear interpolation between
        // LR vertices yields smoother base terrain. The supersample
        // factor is bounded so small maps don't bloat unnecessarily.
        let natural = (num_cx.max(num_cy) as f32 * (cell_size / grid_cell_size)).round() as i32;
        let supersampled = natural.saturating_mul(2);
        supersampled.clamp(512, 4096) as usize
    } else {
        1024
    };
    if dim == 0 {
        dim = 1024;
    }

    let wx_min = rt_offset_x as f64;
    let wz_min = rt_offset_z as f64;
    let wx_max = rt_offset_x as f64 + num_cx as f64 * cell_size as f64;
    let wz_max = rt_offset_z as f64 + num_cy as f64 * cell_size as f64;
    let wx_range = wx_max - wx_min;
    let wz_range = wz_max - wz_min;
    if wx_range <= 0.0 || wz_range <= 0.0 {
        return None;
    }

    let mut hm = vec![f64::NAN; dim * dim];

    for ci in 0..cells.len() {
        let ((ox, oy, oz), (sx, sy, sz), f_start, v_start) = cells[ci];
        let next_v = if ci + 1 < cells.len() { cells[ci + 1].3 } else { verts.len() };
        let cell_vert_count = next_v.saturating_sub(v_start);
        let next_f = if ci + 1 < cells.len() {
            cells[ci + 1].2
        } else {
            faces.len()
        };

        for base in (f_start..next_f).step_by(3) {
            if base + 2 >= faces.len() {
                continue;
            }

            let li0 = faces[base];
            let li1 = faces[base + 1];
            let li2 = faces[base + 2];
            if li0 >= cell_vert_count || li1 >= cell_vert_count || li2 >= cell_vert_count {
                continue;
            }

            let i0 = v_start + li0;
            let i1 = v_start + li1;
            let i2 = v_start + li2;

            let (vx0, vy0, vz0) = verts[i0];
            let (vx1, vy1, vz1) = verts[i1];
            let (vx2, vy2, vz2) = verts[i2];

            let x0 = vx0 * sx as f64 + ox as f64;
            let y0 = vy0 * sy as f64 + oy as f64;
            let z0 = vz0 * sz as f64 + oz as f64;
            let x1 = vx1 * sx as f64 + ox as f64;
            let y1 = vy1 * sy as f64 + oy as f64;
            let z1 = vz1 * sz as f64 + oz as f64;
            let x2 = vx2 * sx as f64 + ox as f64;
            let y2 = vy2 * sy as f64 + oy as f64;
            let z2 = vz2 * sz as f64 + oz as f64;

            let minx = x0.min(x1.min(x2));
            let maxx = x0.max(x1.max(x2));
            let minz = z0.min(z1.min(z2));
            let maxz = z0.max(z1.max(z2));

            let px0 = (((minx - wx_min) / wx_range) * dim as f64).floor() as isize;
            let px1 = (((maxx - wx_min) / wx_range) * dim as f64).ceil() as isize;
            let pz0 = (((minz - wz_min) / wz_range) * dim as f64).floor() as isize;
            let pz1 = (((maxz - wz_min) / wz_range) * dim as f64).ceil() as isize;

            let bx0 = px0.max(0).min(dim as isize - 1) as usize;
            let bx1 = px1.max(0).min(dim as isize - 1) as usize;
            let bz0 = pz0.max(0).min(dim as isize - 1) as usize;
            let bz1 = pz1.max(0).min(dim as isize - 1) as usize;
            if bx1 < bx0 || bz1 < bz0 {
                continue;
            }

            let e1x = x1 - x0;
            let e1z = z1 - z0;
            let e2x = x2 - x0;
            let e2z = z2 - z0;
            let det = e1x * e2z - e1z * e2x;
            if det.abs() < 1e-12 {
                continue;
            }
            let inv = 1.0 / det;

            for pz in bz0..=bz1 {
                let wz = wz_min + (pz as f64 + 0.5) / dim as f64 * wz_range;
                for px in bx0..=bx1 {
                    let wx = wx_min + (px as f64 + 0.5) / dim as f64 * wx_range;
                    let dx = wx - x0;
                    let dz = wz - z0;
                    let u = (dx * e2z - dz * e2x) * inv;
                    let v = (e1x * dz - e1z * dx) * inv;
                    if u >= 0.0 && v >= 0.0 && (u + v) <= 1.0 {
                        let h = y0 + u * (y1 - y0) + v * (y2 - y0);
                        let idx = pz * dim + px;
                        if hm[idx].is_nan() || h > hm[idx] {
                            hm[idx] = h;
                        }
                    }
                }
            }
        }
    }

    let mut min_h = f64::INFINITY;
    let mut max_h = f64::NEG_INFINITY;
    for v in &mut hm {
        if v.is_nan() {
            continue;
        }
        min_h = min_h.min(*v);
        max_h = max_h.max(*v);
    }
    if !min_h.is_finite() || !max_h.is_finite() {
        return None;
    }

    // Fill NaN (ocean / uncovered) pixels using pyramid pull-push interpolation.
    // This matches Python's `_pyramid_fill_nan`: each empty pixel gets smoothly
    // interpolated from its nearest valid neighbours, producing gradual coastal
    // slopes instead of the hard cliff a constant fill would create.
    fill_nan_pyramid(&mut hm, dim);

    // Recompute min/max to include the pyramid-filled ocean pixels.
    min_h = f64::INFINITY;
    max_h = f64::NEG_INFINITY;
    for &v in &hm {
        min_h = min_h.min(v);
        max_h = max_h.max(v);
    }
    // If every pixel ended up identical, give a tiny range so we don't divide by zero.
    if (max_h - min_h) < 0.001 {
        max_h = min_h + 1.0;
    }

    Some(LrFloatMap {
        hm,
        dim,
        wx_min,
        wz_min,
        wx_max,
        wz_max,
        min_h,
        max_h,
    })
}

/// In-memory LandRayTracer rasterization result (world-space float heights).
struct LrFloatMap {
    hm: Vec<f64>,
    dim: usize,
    wx_min: f64,
    wz_min: f64,
    wx_max: f64,
    wz_max: f64,
    min_h: f64,
    max_h: f64,
}

impl LrFloatMap {
    /// Raise any pixel below `water_level` up to `water_level` so the
    /// saved heightmap has a flat ocean surface at sea level. Does NOT
    /// touch pixels above water_level (mountains keep real altitude).
    /// After clamping, `min_h` is set to `water_level` and `max_h` is
    /// recomputed so the PNG normalisation uses the new range.
    #[allow(dead_code)]
    fn clamp_to_water_level(&mut self, water_level: f64) {
        let mut max_h = f64::NEG_INFINITY;
        for v in self.hm.iter_mut() {
            if *v < water_level {
                *v = water_level;
            }
            if *v > max_h {
                max_h = *v;
            }
        }
        self.min_h = water_level;
        if max_h.is_finite() {
            self.max_h = max_h.max(water_level + 1.0);
        }
    }

    /// Save the float heightmap to a 16-bit grayscale PNG (1:1 with the §5
    /// float buffer — pixel (0,0) = world (x_min, z_min); see
    /// docs/ORIENTATION.md §6). 16-bit precision is required: with typical
    /// WT map height ranges of ~2500–3000 m, an 8-bit PNG quantises to
    /// ~10–12 m per step, which is visible as terrain appearing below sea
    /// level when the true stored height is slightly positive.
    fn save_png(&self, path: &Path) -> Option<(u16, u16, f64)> {
        let dim = self.dim;
        let range = (self.max_h - self.min_h).max(0.0001);
        let mut img: ImageBuffer<Luma<u16>, Vec<u16>> =
            ImageBuffer::new(dim as u32, dim as u32);
        let mut sum = 0u64;
        let mut min_v = u16::MAX;
        let mut max_v = u16::MIN;
        for y in 0..dim {
            for x in 0..dim {
                let i = y * dim + x;
                let n = (((self.hm[i] - self.min_h) / range) * 65535.0)
                    .clamp(0.0, 65535.0) as u16;
                min_v = min_v.min(n);
                max_v = max_v.max(n);
                sum += n as u64;
                img.put_pixel(x as u32, y as u32, Luma([n]));
            }
        }
        img.save(path).ok()?;
        let mean = sum as f64 / (dim as f64 * dim as f64);
        Some((min_v, max_v, (mean * 10.0).round() / 10.0))
    }

    /// Generate a tangent-space normalmap PNG from the LR height data and save it.
    /// Uses the same Sobel-style gradient approach as normalmap_detail.png.
    /// Skips if the file already exists (DDS normalmap has priority).
    fn save_normalmap(&self, path: &Path) {
        if path.exists() {
            return;
        }
        let dim = self.dim;
        let w = dim as u32;
        let h = dim as u32;
        let mut norm_img: RgbImage = ImageBuffer::new(w, h);
        for py in 1..(h - 1) {
            for px in 1..(w - 1) {
                let i_l = (py * w + (px - 1)) as usize;
                let i_r = (py * w + (px + 1)) as usize;
                let i_u = ((py - 1) * w + px) as usize;
                let i_d = ((py + 1) * w + px) as usize;
                let dzdx = (self.hm[i_r] - self.hm[i_l]) / 2.0;
                let dzdy = (self.hm[i_d] - self.hm[i_u]) / 2.0;
                let nx = -dzdx as f32;
                let ny = -dzdy as f32;
                let nz = 1.0f32;
                let mag = (nx * nx + ny * ny + nz * nz).sqrt();
                let nx_n = (nx / mag * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0;
                let ny_n = (ny / mag * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0;
                let nz_n = (nz / mag * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0;
                norm_img.put_pixel(px, py, Rgb([nx_n as u8, ny_n as u8, nz_n as u8]));
            }
        }
        let _ = norm_img.save(path);
    }

    /// Overlay HM2 world heights into this float map, replacing the sub-region
    /// that the HM2 covers (nearest-neighbour resample, no feathering).
    fn overlay_hm2(&mut self, hm2_world: &[f32], hdr: &Hm2Header) {
        let dim = self.dim;
        let wx_range = self.wx_max - self.wx_min;
        let wz_range = self.wz_max - self.wz_min;
        if wx_range <= 0.0 || wz_range <= 0.0 {
            return;
        }

        let hm2_x0 = hdr.wpo_x as f64;
        let hm2_z0 = hdr.wpo_y as f64;
        let hm2_x1 = hm2_x0 + hdr.width as f64 * hdr.cell_size as f64;
        let hm2_z1 = hm2_z0 + hdr.height as f64 * hdr.cell_size as f64;

        let px_x0 = (((hm2_x0 - self.wx_min) / wx_range * dim as f64).round() as isize).max(0) as usize;
        let px_x1 = (((hm2_x1 - self.wx_min) / wx_range * dim as f64).round() as isize).min(dim as isize) as usize;
        let px_z0 = (((hm2_z0 - self.wz_min) / wz_range * dim as f64).round() as isize).max(0) as usize;
        let px_z1 = (((hm2_z1 - self.wz_min) / wz_range * dim as f64).round() as isize).min(dim as isize) as usize;

        let crop_w = px_x1.saturating_sub(px_x0);
        let crop_h = px_z1.saturating_sub(px_z0);
        if crop_w == 0 || crop_h == 0 {
            return;
        }

        // Feather width in LR pixels: blend LR→HM2 over up to 16px at each edge
        // to avoid a hard seam at the HM2 region boundary.
        let feather_px = 16usize.min(crop_w / 4).min(crop_h / 4);
        for pz in px_z0..px_z1 {
            for px in px_x0..px_x1 {
                // Map this LR pixel back to HM2 pixel (fractional for bilinear)
                let hm2_x_frac = (px - px_x0) as f64 / crop_w as f64 * hdr.width as f64;
                let hm2_z_frac = (pz - px_z0) as f64 / crop_h as f64 * hdr.height as f64;

                // Bilinear interpolation
                let xi0 = (hm2_x_frac.floor() as usize).min(hdr.width - 1);
                let xi1 = (xi0 + 1).min(hdr.width - 1);
                let zi0 = (hm2_z_frac.floor() as usize).min(hdr.height - 1);
                let zi1 = (zi0 + 1).min(hdr.height - 1);
                let fx = hm2_x_frac - xi0 as f64;
                let fz = hm2_z_frac - zi0 as f64;
                let h00 = hm2_world[zi0 * hdr.width + xi0] as f64;
                let h10 = hm2_world[zi0 * hdr.width + xi1] as f64;
                let h01 = hm2_world[zi1 * hdr.width + xi0] as f64;
                let h11 = hm2_world[zi1 * hdr.width + xi1] as f64;
                let h = h00 * (1.0 - fx) * (1.0 - fz)
                      + h10 * fx            * (1.0 - fz)
                      + h01 * (1.0 - fx)   * fz
                      + h11 * fx            * fz;

                // Feather at HM2 boundary: blend existing LR value → HM2 value
                // over `feather_px` pixels on each edge so the transition is
                // continuous. Interior HM2 pixels overwrite LR directly.
                let dx = (px - px_x0).min(px_x1.saturating_sub(1).saturating_sub(px));
                let dz = (pz - px_z0).min(px_z1.saturating_sub(1).saturating_sub(pz));
                let dist = dx.min(dz);
                if feather_px > 1 && dist < feather_px {
                    let alpha = (dist + 1) as f64 / (feather_px + 1) as f64;
                    let lr_h = self.hm[pz * dim + px];
                    self.hm[pz * dim + px] = lr_h * (1.0 - alpha) + h * alpha;
                } else {
                    self.hm[pz * dim + px] = h;
                }
            }
        }

        // Recompute min/max after overlay
        let mut min_h = f64::INFINITY;
        let mut max_h = f64::NEG_INFINITY;
        for &v in &self.hm {
            min_h = min_h.min(v);
            max_h = max_h.max(v);
        }
        if min_h.is_finite() {
            self.min_h = min_h;
        }
        if max_h.is_finite() {
            self.max_h = max_h;
        }
    }
}

fn decode_hm2(data: &[u8]) -> Option<(Vec<u16>, Hm2Header)> {
    let (ds, _dl) = find_hm2_block(data)?;

    let cell_size = f32::from_le_bytes(data.get(ds..ds + 4)?.try_into().ok()?);
    let h_min = f32::from_le_bytes(data.get(ds + 4..ds + 8)?.try_into().ok()?);
    let h_scale = f32::from_le_bytes(data.get(ds + 8..ds + 12)?.try_into().ok()?);
    let wpo_x = f32::from_le_bytes(data.get(ds + 12..ds + 16)?.try_into().ok()?);
    let wpo_y = f32::from_le_bytes(data.get(ds + 16..ds + 20)?.try_into().ok()?);
    let wv = i32::from_le_bytes(data.get(ds + 20..ds + 24)?.try_into().ok()?);
    let version = (wv as u32 >> HMAP_WIDTH_BITS) as i32;
    let w = (wv & HMAP_WIDTH_MASK) as usize;
    let h = usize::try_from(i32::from_le_bytes(data.get(ds + 24..ds + 28)?.try_into().ok()?)).ok()?;

    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }

    let mut header_size = 44usize;
    let mut block_shift = 3u32;
    let mut hrb_sub_sz = 0usize;
    let mut chunk_sz = 0u32;

    if version == 2 {
        chunk_sz = u32::from_le_bytes(data.get(ds + 44..ds + 48)?.try_into().ok()?);
        block_shift = chunk_sz & 0xff;
        hrb_sub_sz = if (chunk_sz & 0x0f00) != 0 {
            1usize << ((chunk_sz >> 8) & 0x0f)
        } else {
            0
        };
        header_size = 48;
    }

    let inner_pos = ds + header_size;

    if version == 2 {
        let block_width = 1usize.checked_shl(block_shift)?;
        let block_size = block_width.checked_mul(block_width)?;
        let block_size_shift = (block_shift * 2) as usize;
        let bw = w.checked_shr(block_shift)?;
        let bh = h.checked_shr(block_shift)?;
        let total_blocks = bw.checked_mul(bh)?;
        let blockinfo_bytes = total_blocks.checked_mul(4)?;
        let variance_bytes = w.checked_mul(h)?;

        let hrb_levels = if hrb_sub_sz > 0 && w >= hrb_sub_sz * 2 {
            (w / hrb_sub_sz).trailing_zeros() as usize
        } else {
            0
        };
        let shier_grid_offsets = [
            0usize, 1, 5, 21, 85, 341, 1365, 5461, 21845, 87381, 349525, 1398101, 5592405,
            22369621, 89478485,
        ];
        let hrb_bytes = if hrb_levels < shier_grid_offsets.len() {
            shier_grid_offsets[hrb_levels].checked_mul(16)?
        } else {
            0
        };

        let chunk_data_sz = (chunk_sz & !0x0fffu32) as usize;
        let blocks_per_chunk = if chunk_data_sz > 0 {
            chunk_data_sz.checked_shr(block_size_shift as u32).unwrap_or(0)
        } else {
            0
        };
        let chunk_cnt = if blocks_per_chunk > 0 {
            total_blocks.div_ceil(blocks_per_chunk)
        } else {
            0
        };

        let (bi_raw, var_raw) = if chunk_cnt == 0 {
            let lf = u32::from_le_bytes(data.get(inner_pos..inner_pos + 4)?.try_into().ok()?);
            let inner_len = (lf & 0x3fff_ffff) as usize;
            let comp = data.get(inner_pos + 4..inner_pos + 4 + inner_len)?;
            let total_uncomp = blockinfo_bytes.checked_add(variance_bytes)?.checked_add(hrb_bytes)?;
            let raw = oodle_decompress(comp, total_uncomp)?;
            if raw.len() < blockinfo_bytes + variance_bytes {
                return None;
            }
            (
                raw[0..blockinfo_bytes].to_vec(),
                raw[blockinfo_bytes..blockinfo_bytes + variance_bytes].to_vec(),
            )
        } else {
            let mut pos = inner_pos;
            let lf0 = u32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?);
            let c0_len = (lf0 & 0x3fff_ffff) as usize;
            let c0_data = data.get(pos + 4..pos + 4 + c0_len)?;
            let c0_uncomp = blockinfo_bytes.checked_add(hrb_bytes)?;
            let c0_raw = oodle_decompress(c0_data, c0_uncomp)?;
            let bi_raw = c0_raw.get(0..blockinfo_bytes)?.to_vec();
            pos = pos.checked_add(4)?.checked_add(c0_len)?;

            let mut var_parts: Vec<u8> = Vec::with_capacity(variance_bytes);
            for ci in 1..=chunk_cnt {
                let b0 = (ci - 1) * blocks_per_chunk;
                let b1 = std::cmp::min(total_blocks, ci * blocks_per_chunk);
                let n_blocks = b1 - b0;
                let chunk_var_sz = n_blocks.checked_mul(block_size)?;

                let lf = u32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?);
                let cl = (lf & 0x3fff_ffff) as usize;
                let cd = data.get(pos + 4..pos + 4 + cl)?;
                let cr = oodle_decompress(cd, chunk_var_sz)?;
                var_parts.extend_from_slice(cr.get(0..chunk_var_sz)?);
                pos = pos.checked_add(4)?.checked_add(cl)?;
            }

            (bi_raw, var_parts)
        };

        let mut mins = vec![0u16; total_blocks];
        let mut deltas = vec![0u16; total_blocks];
        for i in 0..total_blocks {
            let o = i * 4;
            mins[i] = u16::from_le_bytes([bi_raw[o], bi_raw[o + 1]]);
            deltas[i] = u16::from_le_bytes([bi_raw[o + 2], bi_raw[o + 3]]);
        }

        let mut hmap = vec![0u16; w * h];
        for by in 0..bh {
            for bx in 0..bw {
                let bi = by * bw + bx;
                let mn = mins[bi] as u32;
                let delta = deltas[bi] as u32;
                let mut run = 0u32;
                let boff = bi * block_size;
                for ly in 0..block_width {
                    for lx in 0..block_width {
                        let vi = boff + ly * block_width + lx;
                        run = (run + var_raw[vi] as u32) & 0xff;
                        let val = mn + ((run * delta + 127) / 255);
                        // HM2 v2 disk layout: X is the inner iterator (pixel
                        // column), Y is the outer iterator (row). This
                        // matches the downstream convention where
                        // `hm2_world[zi * w + xi]` samples with world_x
                        // along the row and world_z down the column.
                        // Verified against in-game screenshot info.blk
                        // ground positions (screenshots/*.info.blk).
                        let y = by * block_width + ly;
                        let x = bx * block_width + lx;
                        if y < h && x < w {
                            hmap[y * w + x] = val as u16;
                        }
                    }
                }
            }
        }

        let hdr = Hm2Header {
            h_min,
            h_scale_raw: if h_scale != 0.0 { h_scale / 65535.0 } else { 0.0 },
            width: w,
            height: h,
            version,
            cell_size,
            wpo_x,
            wpo_y,
        };
        return Some((hmap, hdr));
    }

    if version == 0 || version == 1 {
        let lf = u32::from_le_bytes(data.get(inner_pos..inner_pos + 4)?.try_into().ok()?);
        let inner_flags = (lf >> 30) & 3;
        let inner_len = (lf & 0x3fff_ffff) as usize;
        let comp = data.get(inner_pos + 4..inner_pos + 4 + inner_len)?;
        let uncomp = w * h * 2;
        let raw = match inner_flags {
            2 => oodle_decompress(comp, uncomp),
            1 => zstd_decompress_exact(comp, uncomp + 4096),
            0 => oodle_decompress(comp, uncomp).or_else(|| {
                if comp.len() >= uncomp {
                    Some(comp[0..uncomp].to_vec())
                } else {
                    None
                }
            }),
            _ => None,
        }?;

        if raw.len() < uncomp {
            return None;
        }
        let mut hmap = vec![0u16; w * h];
        for i in 0..(w * h) {
            let o = i * 2;
            hmap[i] = u16::from_le_bytes([raw[o], raw[o + 1]]);
        }
        if version == 1 {
            for y in 0..h {
                let mut acc = 0u32;
                for x in 0..w {
                    let i = y * w + x;
                    acc = (acc + hmap[i] as u32) & 0xffff;
                    hmap[i] = acc as u16;
                }
            }
        }

        let hdr = Hm2Header {
            h_min,
            h_scale_raw: if h_scale != 0.0 { h_scale / 65535.0 } else { 0.0 },
            width: w,
            height: h,
            version,
            cell_size,
            wpo_x,
            wpo_y,
        };
        return Some((hmap, hdr));
    }

    None
}

// ── HM2 detail generation ──────────────────────────────────────────

fn generate_hm2_detail(
    _hmap16: &[u16],
    world: &[f32],
    hdr: &Hm2Header,
    viewer_dir: &Path,
) -> Result<Option<Value>> {
    let w = hdr.width as u32;
    let h = hdr.height as u32;

    // Find min/max for normalization
    let mut min_h = f32::INFINITY;
    let mut max_h = f32::NEG_INFINITY;
    for &v in world {
        min_h = min_h.min(v);
        max_h = max_h.max(v);
    }

    let range = (max_h - min_h).max(0.0001);

    // Generate heightmap_detail.png.
    // `world` is the world-frame float buffer (per docs/ORIENTATION.md §7,
    // rotated once in the decoder). It follows §5: pixel (xi, zi) ↔ world
    // (wpo_x + xi*cs, wpo_y + zi*cs). Per §6 the PNG is saved 1:1 with the
    // float buffer — no rotate180.
    let mut detail_img: ImageBuffer<Luma<u16>, Vec<u16>> = ImageBuffer::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let n = ((world[i] - min_h) / range * 65535.0).clamp(0.0, 65535.0) as u16;
            detail_img.put_pixel(x, y, Luma([n]));
        }
    }
    let detail_path = viewer_dir.join("heightmap_detail.png");
    detail_img
        .save(&detail_path)
        .with_context(|| format!("Failed to save {}", detail_path.display()))?;

    // Generate normalmap_detail.png from gradients.
    // `world` is the world-frame float buffer (docs/ORIENTATION.md §5/§7), so
    // gradients use plain neighbour indexing. Per §6 the PNG is saved 1:1
    // with the float buffer — no rotate180.
    //   dzdx = ∂h/∂x (world East)
    //   dzdy = ∂h/∂z (world North)
    //   nx = -dzdx;  ny = -dzdy;  nz = 1
    let cs = hdr.cell_size as f64;
    let mut norm_img: RgbImage = ImageBuffer::new(w, h);

    for oy in 1..(h - 1) {
        for ox in 1..(w - 1) {
            let i_xp1 = (oy * w + (ox + 1)) as usize;
            let i_xm1 = (oy * w + (ox - 1)) as usize;
            let i_yp1 = ((oy + 1) * w + ox) as usize;
            let i_ym1 = ((oy - 1) * w + ox) as usize;

            let dzdx = (world[i_xp1] - world[i_xm1]) as f64 / (2.0 * cs);
            let dzdy = (world[i_yp1] - world[i_ym1]) as f64 / (2.0 * cs);

            let nx = -dzdx;
            let ny = -dzdy;
            let nz = 1.0_f64;

            let mag = (nx * nx + ny * ny + nz * nz).sqrt();
            let r = ((nx / mag * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            let g = ((ny / mag * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            let b = ((nz / mag * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;

            norm_img.put_pixel(ox as u32, oy as u32, Rgb([r, g, b]));
        }
    }
    let norm_path = viewer_dir.join("normalmap_detail.png");
    norm_img
        .save(&norm_path)
        .with_context(|| format!("Failed to save {}", norm_path.display()))?;

    Ok(Some(json!({
        "heightmap_detail": "heightmap_detail.png",
        "normalmap_detail": "normalmap_detail.png",
        "detail_width": w,
        "detail_height": h,
        // Use non-conflicting key names so these do NOT overwrite
        // the main LR height_min_m/height_max_m during the merge.
        "_detail_height_min_m": (min_h * 10.0).round() / 10.0,
        "_detail_height_max_m": (max_h * 10.0).round() / 10.0
    })))
}

pub fn build_heightmap_native(data: &[u8], viewer_dir: &Path, water_level: Option<f64>) -> Result<Option<Value>> {
    // Priority (matches Python pipeline):
    //   1. LandRayTracer as the wide-coverage base heightmap.
    //   2. HM2 overlaid into the LR array for its battle-area sub-region.
    //   3. HM2 saved separately as heightmap_detail.png.
    //   4. If no LR but HM2 exists, use HM2 as the primary heightmap.
    //   5. If neither, return None (caller uses pseudo fallback).

    // ── Step 1: rasterize LR into float ──
    let lr_opt: Option<LrFloatMap> = extract_land_ray_tracer_dump(data)
        .and_then(|(rt_dump, grid_cell_size)| rasterize_land_ray_tracer(&rt_dump, grid_cell_size));

    // ── Step 2: decode HM2 ──
    // Per docs/ORIENTATION.md §7: under the 1:1 §2/§5 convention (pixel
    // (0,0) = world SW), the raw HM2 buffer is already aligned so no
    // rotation is applied. Downstream consumers (overlay_hm2,
    // generate_hm2_detail) treat it as a plain §5 float buffer where
    // pixel (xi, zi) maps to world (wpo_x + xi*cell, wpo_y + zi*cell).
    let hm2_opt = decode_hm2(data).map(|(hmap16, hdr)| {
        let world: Vec<f32> = hmap16
            .iter()
            .map(|&v| v as f32 * hdr.h_scale_raw + hdr.h_min)
            .collect();
        (world, hdr)
    });

    // ── Step 3: generate heightmap_detail.png from HM2 ──
    let detail_info: Option<Value> = if let Some((ref world, ref hdr)) = hm2_opt {
        generate_hm2_detail(&[], world, hdr, viewer_dir)?
    } else {
        None
    };

    // ── Step 4: build the primary heightmap ──
    let out_path = viewer_dir.join("heightmap.png");

    match (lr_opt, hm2_opt) {
        (Some(mut lr), Some((hm2_world, ref hdr))) => {
            // Overlay HM2 into LR then save.
            lr.overlay_hm2(&hm2_world, hdr);
            // NOTE: do NOT clamp to water_level. The Python reference pipeline
            // keeps the true unclamped min (e.g. −379 m for avg_vietnam_hills)
            // so the LR→PNG normalisation matches the HM2 detail's absolute
            // altitude scale. Clamping compresses `height_max_m - height_min_m`,
            // which then makes the HM2 detail displacement appear ~15 % too
            // steep relative to the main LR mesh. The viewer's water mesh
            // handles the visual sea-floor hiding.
            let _ = water_level;
            lr.save_normalmap(&viewer_dir.join("normalmap.png"));
            let (min_u8, max_u8, mean) = lr.save_png(&out_path).ok_or_else(|| {
                anyhow::anyhow!("Failed to save {}", out_path.display())
            })?;
            let mut result = json!({
                "file": "heightmap.png",
                "width": lr.dim,
                "height": lr.dim,
                "min": min_u8,
                "max": max_u8,
                "mean": mean,
                "height_min_m": (lr.min_h * 10.0).round() / 10.0,
                "height_max_m": (lr.max_h * 10.0).round() / 10.0,
                "world_extent": [lr.wx_min, lr.wz_min, lr.wx_max, lr.wz_max],
                "source": "LandRayTracer+HM2",
                "hm2Version": hdr.version,
            });
            if let Some(d) = detail_info {
                let dmin = d["_detail_height_min_m"].as_f64().unwrap_or(0.0);
                let dmax = d["_detail_height_max_m"].as_f64().unwrap_or(0.0);
                let hm2_x1 = hdr.wpo_x + hdr.width as f32 * hdr.cell_size;
                let hm2_z1 = hdr.wpo_y + hdr.height as f32 * hdr.cell_size;
                // Build the heightmapDetail sub-object consumed by the viewer
                let hmd = json!({
                    "file": "heightmap_detail.png",
                    "width": d["detail_width"],
                    "height": d["detail_height"],
                    "normalmap_detail": "normalmap_detail.png",
                    "world_x0": hdr.wpo_x,
                    "world_z0": hdr.wpo_y,
                    "world_x1": hm2_x1,
                    "world_z1": hm2_z1,
                    "height_min_m": dmin,
                    "height_max_m": dmax,
                    "cell_size": hdr.cell_size,
                });
                if let Some(obj) = result.as_object_mut() {
                    if let Some(dobj) = d.as_object() {
                        for (k, v) in dobj {
                            // Don't let detail height range or private/size keys
                            // overwrite the main LR values.
                            if k.starts_with('_') || k == "detail_width" || k == "detail_height" {
                                continue;
                            }
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                    // Embed heightmapDetail so pipeline.rs can hoist it to top-level
                    obj.insert("_heightmapDetail".to_string(), hmd);
                }
            }
            Ok(Some(result))
        }
        (Some(lr), None) => {
            // LR only. See note above — no water_level clamping.
            let lr = lr;
            let _ = water_level;
            lr.save_normalmap(&viewer_dir.join("normalmap.png"));
            let (min_u8, max_u8, mean) = lr.save_png(&out_path).ok_or_else(|| {
                anyhow::anyhow!("Failed to save {}", out_path.display())
            })?;
            Ok(Some(json!({
                "file": "heightmap.png",
                "width": lr.dim,
                "height": lr.dim,
                "min": min_u8,
                "max": max_u8,
                "mean": mean,
                "height_min_m": (lr.min_h * 10.0).round() / 10.0,
                "height_max_m": (lr.max_h * 10.0).round() / 10.0,
                "world_extent": [lr.wx_min, lr.wz_min, lr.wx_max, lr.wz_max],
                "source": "LandRayTracer",
            })))
        }
        (None, Some((hm2_world, ref hdr))) => {
            // HM2 only — use as primary heightmap
            let w = hdr.width as u32;
            let h = hdr.height as u32;
            let mut min_h = f32::INFINITY;
            let mut max_h = f32::NEG_INFINITY;
            for &v in &hm2_world {
                min_h = min_h.min(v);
                max_h = max_h.max(v);
            }
            let range = (max_h - min_h).max(0.0001);
            let mut img: ImageBuffer<Luma<u16>, Vec<u16>> = ImageBuffer::new(w, h);
            let mut sum = 0u64;
            let mut min_u8 = u16::MAX;
            let mut max_u8 = u16::MIN;
            for y in 0..h {
                for x in 0..w {
                    let i = (y * w + x) as usize;
                    let n = ((hm2_world[i] - min_h) / range * 65535.0).clamp(0.0, 65535.0) as u16;
                    min_u8 = min_u8.min(n);
                    max_u8 = max_u8.max(n);
                    sum += n as u64;
                    img.put_pixel(x, y, Luma([n]));
                }
            }
            img.save(&out_path)
                .with_context(|| format!("Failed to save {}", out_path.display()))?;
            let mean = sum as f64 / (w as f64 * h as f64);
            let hm2_x1 = hdr.wpo_x as f64 + hdr.width as f64 * hdr.cell_size as f64;
            let hm2_z1 = hdr.wpo_y as f64 + hdr.height as f64 * hdr.cell_size as f64;
            let mut result = json!({
                "file": "heightmap.png",
                "width": w,
                "height": h,
                "min": min_u8,
                "max": max_u8,
                "mean": (mean * 10.0).round() / 10.0,
                "height_min_m": (min_h as f64 * 10.0).round() / 10.0,
                "height_max_m": (max_h as f64 * 10.0).round() / 10.0,
                "world_extent": [hdr.wpo_x as f64, hdr.wpo_y as f64, hm2_x1, hm2_z1],
                "source": "HM2",
                "hm2Version": hdr.version,
            });
            if let Some(d) = detail_info {
                if let Some(obj) = result.as_object_mut() {
                    if let Some(dobj) = d.as_object() {
                        for (k, v) in dobj {
                            if k.starts_with('_') || k == "detail_width" || k == "detail_height" {
                                continue;
                            }
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            Ok(Some(result))
        }
        (None, None) => Ok(None),
    }
}
