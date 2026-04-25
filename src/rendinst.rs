use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use lzma_rs::lzma_decompress;
use serde_json::{json, Value};

use crate::util::{oodle_decompress, read_u32_le, read_i32_le, read_f32_le};

const RIGZ_SUBCELL_DIV: usize = 8;
const RIGZ_CELL_STRUCT_SIZE: usize = 552;
const RIGZ_POOL_STRUCT_SIZE: usize = 32;
const RIGZ_LANDCLS_STRUCT_SIZE: usize = 32;

fn decompress_rigz_block(block_data: &[u8], flags: u32) -> Option<Vec<u8>> {
    match flags {
        2 => {
            let raw_size = read_u32_le(block_data, 0)? as usize;
            oodle_decompress(block_data.get(4..)?, raw_size)
        }
        1 => zstd::stream::decode_all(block_data).ok(),
        0 => {
            let mut src = std::io::Cursor::new(block_data);
            let mut out = Vec::new();
            if lzma_decompress(&mut src, &mut out).is_ok() {
                Some(out)
            } else {
                None
            }
        }
        3 => Some(block_data.to_vec()),
        _ => None,
    }
}

fn classify_pool(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if ["tree", "palm"].iter().any(|k| n.contains(k)) {
        return "tree";
    }
    if ["bush", "shrub", "hedge"].iter().any(|k| n.contains(k)) {
        return "bush";
    }
    if ["grass", "weed", "flower", "plant", "fern", "reed"].iter().any(|k| n.contains(k)) {
        return "vegetation";
    }
    if [
        "house", "building", "build", "barn", "church", "mosque", "palace", "castle", "tower",
        "factory", "warehouse", "hangar", "bunker", "shelter", "pillbox", "barrack", "hospital",
        "school", "shop", "hotel", "station", "industrial", "admin", "fortress", "courtyard",
    ]
    .iter()
    .any(|k| n.contains(k))
    {
        return "building";
    }
    if [
        "fence", "wall", "gate", "column", "lattice", "embankment", "lamppost", "pole", "sign",
        "rail", "bridge", "road", "pipe", "wire", "cable",
    ]
    .iter()
    .any(|k| n.contains(k))
    {
        return "infrastructure";
    }
    if ["rock", "stone", "boulder", "cliff"].iter().any(|k| n.contains(k)) {
        return "rock";
    }
    if ["debris", "wreck", "ruin", "rubble", "crater", "sandbag", "hedgehog", "barbed", "trench"]
        .iter()
        .any(|k| n.contains(k))
    {
        return "debris";
    }
    "other"
}

pub fn extract_rendinst_native(data: &[u8], viewer_dir: &Path) -> Result<Option<Value>> {
    if data.len() < 12 || &data[0..4] != b"DBLD" {
        return Ok(None);
    }

    let mut pos = 0x0cusize;
    let mut rigz_payload: Option<&[u8]> = None;
    let mut tm32_mode = false;
    while pos + 8 <= data.len() {
        let lf = match read_u32_le(&data, pos) {
            Some(v) => v,
            None => break,
        };
        let blen = (lf & 0x3fff_ffff) as usize;
        if blen < 4 || blen > data.len().saturating_sub(pos) {
            break;
        }
        let tag = &data[pos + 4..pos + 8];
        if tag == b"tm24" {
            tm32_mode = true;
        } else if tag == b"RIGz" {
            rigz_payload = data.get(pos + 8..pos + 4 + blen);
            break;
        }
        if tag.contains(&b'E') && tag.contains(&b'N') && tag.contains(&b'D') {
            break;
        }
        pos += 4 + blen;
    }

    let rigz = match rigz_payload {
        Some(v) if v.len() >= 8 => v,
        _ => return Ok(None),
    };

    let sb1_lf = read_u32_le(rigz, 0).unwrap_or(0);
    let sb1_flags = sb1_lf >> 30;
    let sb1_len = (sb1_lf & 0x3fff_ffff) as usize;
    if 4 + sb1_len > rigz.len() {
        return Ok(None);
    }
    let sb1_data = &rigz[4..4 + sb1_len];

    let sb2_off = 4 + sb1_len;
    let sb2_lf = match read_u32_le(rigz, sb2_off) {
        Some(v) => v,
        None => return Ok(None),
    };
    let sb2_len = (sb2_lf & 0x3fff_ffff) as usize;
    if sb2_off + 4 + sb2_len > rigz.len() {
        return Ok(None);
    }
    let sb2_data = &rigz[sb2_off + 4..sb2_off + 4 + sb2_len];

    let decompressed = match decompress_rigz_block(sb1_data, sb1_flags) {
        Some(v) if v.len() >= 4 => v,
        _ => return Ok(None),
    };

    let dump_sz = read_u32_le(&decompressed, 0).unwrap_or(0) as usize;
    if dump_sz < 152 || 4 + dump_sz > decompressed.len() {
        return Ok(None);
    }
    let dump = &decompressed[4..4 + dump_sz];

    let cells_offset = read_u32_le(dump, 8).unwrap_or(0) as usize;
    let cells_count = read_u32_le(dump, 12).unwrap_or(0) as usize;
    let cell_num_w = read_i32_le(dump, 24).unwrap_or(0).max(0) as usize;
    let cell_num_h = read_i32_le(dump, 28).unwrap_or(0).max(0) as usize;
    let cell_sz = i16::from_le_bytes(dump.get(32..34).unwrap_or(&[0, 0]).try_into().unwrap_or([0, 0])) as f32;
    let data_flags = *dump.get(34).unwrap_or(&0);
    let per_inst_data_dwords = *dump.get(35).unwrap_or(&0) as usize;
    let landcls_offset = read_u32_le(dump, 48).unwrap_or(0) as usize;
    let landcls_count = read_u32_le(dump, 52).unwrap_or(0) as usize;
    let world0x = read_f32_le(dump, 64).unwrap_or(0.0);
    let world0z = read_f32_le(dump, 68).unwrap_or(0.0);
    let grid2world = read_f32_le(dump, 112).unwrap_or(0.0);
    let pregen_offset = read_u32_le(dump, 128).unwrap_or(0) as usize;
    let pregen_count = read_u32_le(dump, 132).unwrap_or(0) as usize;

    let ent32 = (data_flags & 0x01) != 0;
    let cell_world_sz = grid2world * cell_sz;

    let mut pools: Vec<(String, bool, bool)> = Vec::new();
    for i in 0..pregen_count {
        let po = pregen_offset + i * RIGZ_POOL_STRUCT_SIZE;
        if po + RIGZ_POOL_STRUCT_SIZE > dump.len() {
            break;
        }
        let name_ptr = read_i32_le(dump, po + 8).unwrap_or(-1);
        let mut name = String::new();
        if name_ptr >= 0 {
            let np = name_ptr as usize;
            if np < dump.len() {
                let end = dump[np..].iter().position(|b| *b == 0).map(|v| np + v).unwrap_or((np + 128).min(dump.len()));
                name = String::from_utf8_lossy(&dump[np..end]).to_string();
            }
        }
        let flags_word = read_u32_le(dump, po + 24).unwrap_or(0);
        pools.push((name, (flags_word & 1) != 0, (flags_word & 4) != 0));
    }

    let mut ri_landclasses = Vec::new();
    for i in 0..landcls_count {
        let lo = landcls_offset + i * RIGZ_LANDCLS_STRUCT_SIZE;
        if lo + RIGZ_LANDCLS_STRUCT_SIZE > dump.len() {
            break;
        }
        let name_ptr = read_i32_le(dump, lo).unwrap_or(-1);
        if name_ptr < 0 {
            continue;
        }
        let np = name_ptr as usize;
        if np >= dump.len() {
            continue;
        }
        let end = dump[np..].iter().position(|b| *b == 0).map(|v| np + v).unwrap_or((np + 128).min(dump.len()));
        ri_landclasses.push(String::from_utf8_lossy(&dump[np..end]).to_string());
    }

    let mut all_instances: Vec<(usize, f32, f32, f32)> = Vec::new();

    for ci in 0..cells_count {
        let co = cells_offset + ci * RIGZ_CELL_STRUCT_SIZE;
        if co + RIGZ_CELL_STRUCT_SIZE > dump.len() {
            break;
        }
        let ht_min = i16::from_le_bytes([dump[co + 24], dump[co + 25]]) as f32;
        let ht_delta = i16::from_le_bytes([dump[co + 26], dump[co + 27]]) as f32;
        let ri_data_rel_ofs = read_i32_le(dump, co + 28).unwrap_or(-1);
        if ri_data_rel_ofs < 0 {
            continue;
        }

        let col = ci % cell_num_w.max(1);
        let row = ci / cell_num_w.max(1);
        let cell_x0 = world0x + col as f32 * cell_world_sz;
        let cell_z0 = world0z + row as f32 * cell_world_sz;
        let cell_y0 = ht_min;
        let cell_y_sz = if ht_delta != 0.0 { ht_delta } else { 8192.0 };

        let ecb = co + 32;
        let mut subcell_pools: Vec<Vec<(usize, usize)>> = Vec::with_capacity(RIGZ_SUBCELL_DIV * RIGZ_SUBCELL_DIV);
        for s in 0..(RIGZ_SUBCELL_DIV * RIGZ_SUBCELL_DIV) {
            let p0 = read_i32_le(dump, ecb + s * 8).unwrap_or(-1);
            let p1 = read_i32_le(dump, ecb + (s + 1) * 8).unwrap_or(-1);
            if p0 < 0 || p1 < 0 || p0 >= p1 {
                subcell_pools.push(vec![]);
                continue;
            }
            let p0u = p0 as usize;
            let p1u = p1 as usize;
            let mut entries = Vec::new();
            if ent32 {
                for e in 0..((p1u - p0u) / 8) {
                    let ec = p0u + e * 8;
                    if ec + 8 > dump.len() {
                        break;
                    }
                    let idx = read_u32_le(dump, ec).unwrap_or(0) as usize;
                    let cnt = read_u32_le(dump, ec + 4).unwrap_or(0) as usize;
                    entries.push((idx, cnt));
                }
            } else {
                for e in 0..((p1u - p0u) / 4) {
                    let ec = p0u + e * 4;
                    if ec + 4 > dump.len() {
                        break;
                    }
                    let val = read_u32_le(dump, ec).unwrap_or(0);
                    let idx = (((val >> 30) << 10) | (val & 0x3ff)) as usize;
                    let cnt = ((val >> 10) & 0x000f_ffff) as usize;
                    entries.push((idx, cnt));
                }
            }
            subcell_pools.push(entries);
        }

        let rel = ri_data_rel_ofs as usize;
        if rel + 4 > sb2_data.len() {
            continue;
        }
        let cell_lf = read_u32_le(sb2_data, rel).unwrap_or(0);
        let cell_flags = cell_lf >> 30;
        let cell_len = (cell_lf & 0x3fff_ffff) as usize;
        if rel + 4 + cell_len > sb2_data.len() {
            continue;
        }
        let cell_comp = &sb2_data[rel + 4..rel + 4 + cell_len];
        let cell_raw = match decompress_rigz_block(cell_comp, cell_flags) {
            Some(v) => v,
            None => continue,
        };

        let mut dp = 0usize;
        for entries in subcell_pools {
            for (pool_idx, count) in entries {
                if pool_idx >= pools.len() {
                    continue;
                }
                let (_name, pos_inst, zero_inst_seeds) = &pools[pool_idx];
                let base_stride = if *pos_inst {
                    8usize
                } else if tm32_mode {
                    48usize
                } else {
                    24usize
                };
                let extra = if *zero_inst_seeds { 0usize } else { 4 * per_inst_data_dwords };
                let stride = base_stride + extra;
                if stride == 0 {
                    continue;
                }

                let avail = cell_raw.len().saturating_sub(dp) / stride;
                let batch = count.min(avail);
                if batch == 0 {
                    dp = dp.saturating_add(count.saturating_mul(stride));
                    continue;
                }

                if *pos_inst {
                    for bi in 0..batch {
                        let o = dp + bi * stride;
                        if o + 8 > cell_raw.len() {
                            break;
                        }
                        let px = i16::from_le_bytes([cell_raw[o], cell_raw[o + 1]]) as f32;
                        let py = i16::from_le_bytes([cell_raw[o + 2], cell_raw[o + 3]]) as f32;
                        let pz = i16::from_le_bytes([cell_raw[o + 4], cell_raw[o + 5]]) as f32;
                        let alive = i16::from_le_bytes([cell_raw[o + 6], cell_raw[o + 7]]) != 0;
                        if !alive {
                            continue;
                        }
                        let wx = px * (cell_world_sz / 32767.0) + cell_x0;
                        let wy = py * (cell_y_sz / 32767.0) + cell_y0;
                        let wz = pz * (cell_world_sz / 32767.0) + cell_z0;
                        all_instances.push((pool_idx, (wx * 10.0).round() / 10.0, (wy * 10.0).round() / 10.0, (wz * 10.0).round() / 10.0));
                    }
                    dp += batch * stride;
                } else if tm32_mode {
                    for _ in 0..batch {
                        if dp + stride > cell_raw.len() {
                            break;
                        }
                        let mut vals = [0i32; 12];
                        for (k, v) in vals.iter_mut().enumerate() {
                            let oo = dp + k * 4;
                            *v = i32::from_le_bytes([cell_raw[oo], cell_raw[oo + 1], cell_raw[oo + 2], cell_raw[oo + 3]]);
                        }
                        if vals[0] == 0 && vals[1] == 0 {
                            dp += stride;
                            continue;
                        }
                        let wx = vals[3] as f32 * cell_world_sz / 65536.0 / 32767.0 + cell_x0;
                        let wy = vals[7] as f32 * cell_y_sz / 65536.0 / 32767.0 + cell_y0;
                        let wz = vals[11] as f32 * cell_world_sz / 65536.0 / 32767.0 + cell_z0;
                        all_instances.push((pool_idx, (wx * 10.0).round() / 10.0, (wy * 10.0).round() / 10.0, (wz * 10.0).round() / 10.0));
                        dp += stride;
                    }
                } else {
                    for _ in 0..batch {
                        if dp + stride > cell_raw.len() {
                            break;
                        }
                        let mut vals = [0i16; 12];
                        for (k, v) in vals.iter_mut().enumerate() {
                            let oo = dp + k * 2;
                            *v = i16::from_le_bytes([cell_raw[oo], cell_raw[oo + 1]]);
                        }
                        if vals[0] == 0 && vals[1] == 0 && vals[2] == 0 && vals[3] == 0 {
                            dp += stride;
                            continue;
                        }
                        let wx = vals[3] as f32 * cell_world_sz / 32767.0 + cell_x0;
                        let wy = vals[7] as f32 * cell_y_sz / 32767.0 + cell_y0;
                        let wz = vals[11] as f32 * cell_world_sz / 32767.0 + cell_z0;
                        all_instances.push((pool_idx, (wx * 10.0).round() / 10.0, (wy * 10.0).round() / 10.0, (wz * 10.0).round() / 10.0));
                        dp += stride;
                    }
                }
            }
        }
    }

    if all_instances.is_empty() {
        return Ok(None);
    }

    let mut pool_counts: HashMap<usize, usize> = HashMap::new();
    for (idx, _, _, _) in &all_instances {
        *pool_counts.entry(*idx).or_insert(0) += 1;
    }

    let mut pool_list = Vec::new();
    for (i, (name, _, _)) in pools.iter().enumerate() {
        let cat = classify_pool(name);
        pool_list.push(json!({
            "name": name,
            "category": cat,
            "count": pool_counts.get(&i).copied().unwrap_or(0)
        }));
    }

    let ri_path = viewer_dir.join("rendinst.bin");
    let mut f = fs::File::create(&ri_path)
        .with_context(|| format!("Failed to create {}", ri_path.display()))?;
    // Binary format: pool_idx(u16) + wx(f32) + wy(f32) + wz(f32) = 14 bytes/instance
    for (pool_idx, wx, wy, wz) in &all_instances {
        let p = (*pool_idx as u16).to_le_bytes();
        let x = wx.to_le_bytes();
        let y = wy.to_le_bytes();
        let z = wz.to_le_bytes();
        f.write_all(&p)?;
        f.write_all(&x)?;
        f.write_all(&y)?;
        f.write_all(&z)?;
    }

    Ok(Some(json!({
        "file": "rendinst.bin",
        "stride": 14u32,
        "instanceCount": all_instances.len(),
        "poolCount": pool_list.len(),
        "pools": pool_list,
        "grid": {
            "cellNumW": cell_num_w,
            "cellNumH": cell_num_h,
            "cellWorldSize": cell_world_sz,
            "world0": [world0x, world0z],
            "grid2world": grid2world
        },
        "riLandclasses": ri_landclasses
    })))
}
