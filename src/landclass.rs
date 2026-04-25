use std::collections::{BTreeSet, HashSet};

use anyhow::Result;
use serde_json::{json, Value};

use crate::util::{read_i32_le, read_u32_le, read_f32_le};

#[derive(Debug, Clone)]
struct LndmHeader {
    grid_cell_size: f32,
    land_cell_size: f32,
    map_size_x: i32,
    map_size_y: i32,
    base_ofs: usize,
    detail_data_ofs: i32,
    tile_data_ofs: i32,
}

fn parse_lndm_header(data: &[u8]) -> Option<LndmHeader> {
    let lndm_pos = data.windows(4).position(|w| w == b"lndm")?;
    let mut p = lndm_pos + 4;
    let version = read_i32_le(data, p)?;
    p += 4;
    if version != 4 {
        return None;
    }

    let grid_cell_size = read_f32_le(data, p)?;
    p += 4;
    let land_cell_size = read_f32_le(data, p)?;
    p += 4;
    let map_size_x = read_i32_le(data, p)?;
    p += 4;
    let map_size_y = read_i32_le(data, p)?;
    p += 4;

    p += 4 + 4 + 4; // originX, originY, useTile
    let base_ofs = p;
    p += 4; // meshMapOfs
    let detail_data_ofs = read_i32_le(data, p)?;
    p += 4;
    let tile_data_ofs = read_i32_le(data, p)?;

    Some(LndmHeader {
        grid_cell_size,
        land_cell_size,
        map_size_x,
        map_size_y,
        base_ofs,
        detail_data_ofs,
        tile_data_ofs,
    })
}

fn is_printable_ascii(b: u8) -> bool {
    (0x20..0x7f).contains(&b)
}

fn read_cstr_ascii(data: &[u8], pos: usize) -> Option<(String, usize)> {
    if pos >= data.len() {
        return None;
    }
    let end_rel = data[pos..].iter().position(|b| *b == 0)?;
    if end_rel == 0 {
        return None;
    }
    let end = pos + end_rel;
    if !data[pos..end].iter().all(|b| is_printable_ascii(*b)) {
        return None;
    }
    let s = String::from_utf8_lossy(&data[pos..end]).to_string();
    Some((s, end + 1))
}

fn extract_block_name(nm: &[u8], sep_pos: usize) -> Option<String> {
    let mut ne = sep_pos;
    while ne > 0 && nm[ne - 1] == 0 {
        ne -= 1;
    }
    let mut ns = ne;
    while ns > 0 && is_printable_ascii(nm[ns - 1]) {
        ns -= 1;
    }
    if ns >= ne {
        return None;
    }
    let s = String::from_utf8_lossy(&nm[ns..ne]).to_string();
    if s.len() < 3 {
        return None;
    }
    Some(s)
}

fn parse_detail_blk(raw: &[u8]) -> Vec<Value> {
    if raw.len() < 16 {
        return vec![];
    }

    let name_map_sz = read_u32_le(raw, 0).unwrap_or(0) as usize;
    if raw.len() < 16 + name_map_sz {
        return vec![];
    }
    let nm = &raw[16..16 + name_map_sz];

    let signatures: [&[u8]; 6] = [
        b"detail\0texture\0",
        b"detail\0editable\0",
        b"detail\0",
        b"detailmap\0",
        b"shader\0",
        b"landClassTextures\0",
    ];

    let keywords: HashSet<&str> = [
        "detail",
        "texture",
        "splattingmap",
        "size",
        "offset",
        "editorId",
        "detailRed",
        "detail2Red",
        "detailSizeRed",
        "detailGreen",
        "detail2Green",
        "detailSizeGreen",
        "detailBlue",
        "detail2Blue",
        "detailSizeBlue",
        "detailBlack",
        "detail2Black",
        "detailSizeBlack",
        "detailSize",
        "physMatRed",
        "physMatGreen",
        "physMatBlue",
        "physMatBlack",
        "detailmap",
        "detailmap_r",
        "detailmap_m",
        "detailmap_g",
        "physMat",
        "displacementMin",
        "displacementMax",
        "displacementMaxRed",
        "displacementMaxGreen",
        "displacementMaxBlue",
        "displacementMaxBlack",
        "displacementMinRed",
        "displacementMinGreen",
        "displacementMinBlue",
        "displacementMinBlack",
        "bumpScale",
        "shader",
        "landClassTextures",
        "editable",
        "colorMapSize",
        "splattingMapSize",
        "flowmapTex",
    ]
    .into_iter()
    .collect();

    let mut blocks: BTreeSet<(usize, String, usize)> = BTreeSet::new();

    for sig in signatures {
        let mut search_pos = 0usize;
        while search_pos < nm.len() {
            let rel = match nm[search_pos..].windows(sig.len()).position(|w| w == sig) {
                Some(v) => v,
                None => break,
            };
            let idx = search_pos + rel;
            search_pos = idx + sig.len();

            let prop_start = idx;
            let mut sep_pos = None;
            let mut bname = None;
            for back in 3..32 {
                if prop_start < back {
                    break;
                }
                let check = prop_start - back;
                if nm.get(check) != Some(&0x01) {
                    continue;
                }
                let pc = *nm.get(check + 1).unwrap_or(&0);
                if pc < 3 {
                    continue;
                }

                let mut keyword_ok = false;
                for off in [4usize, 3usize] {
                    if let Some((s, _)) = read_cstr_ascii(nm, check + off) {
                        if keywords.contains(s.as_str()) {
                            keyword_ok = true;
                            break;
                        }
                    }
                }
                if !keyword_ok {
                    if let Some((s, _)) = read_cstr_ascii(nm, prop_start) {
                        if keywords.contains(s.as_str()) {
                            keyword_ok = true;
                        }
                    }
                }
                if !keyword_ok {
                    continue;
                }

                if let Some(n) = extract_block_name(nm, check) {
                    sep_pos = Some(check);
                    bname = Some(n);
                    break;
                }
            }

            if let (Some(sp), Some(name)) = (sep_pos, bname) {
                blocks.insert((sp, name, prop_start));
            }
        }
    }

    if blocks.is_empty() {
        return vec![];
    }

    let mut block_vec: Vec<(usize, String, usize)> = blocks.into_iter().collect();
    block_vec.sort_by_key(|v| v.0);

    let mut out = Vec::new();

    for bi in 0..block_vec.len() {
        let (sep_pos, bname, _prop_start) = &block_vec[bi];
        let block_end = if bi + 1 < block_vec.len() {
            block_vec[bi + 1].0
        } else {
            nm.len()
        };

        let mut value_strings = Vec::new();
        let mut vpos = *sep_pos;
        while vpos < block_end {
            if !is_printable_ascii(nm[vpos]) {
                vpos += 1;
                continue;
            }
            if let Some((s, np)) = read_cstr_ascii(nm, vpos) {
                value_strings.push(s);
                vpos = np;
            } else {
                vpos += 1;
            }
        }

        let mut texture: Option<String> = None;
        for v in &value_strings {
            if v.ends_with('*') {
                let mut t = v.trim_end_matches('*').replace('*', "");
                if let Some(ci) = t.chars().position(|c| c.is_ascii_lowercase()) {
                    t = t.chars().skip(ci).collect();
                }
                texture = Some(t);
                break;
            }
        }

        let mut details = serde_json::Map::new();
        let mut d_tex: Vec<String> = value_strings
            .iter()
            .filter(|v| v.contains("detail_") && v.ends_with("_tex_d"))
            .cloned()
            .collect();
        let mut r_tex: Vec<String> = value_strings
            .iter()
            .filter(|v| v.contains("detail_") && v.ends_with("_tex_r"))
            .cloned()
            .collect();
        d_tex.truncate(4);
        r_tex.truncate(4);
        for (i, key) in ["R", "G", "B", "K"].iter().enumerate() {
            if let Some(v) = d_tex.get(i) {
                details.insert((*key).to_string(), Value::String(v.clone()));
            }
            if let Some(v) = r_tex.get(i) {
                details.insert(format!("{}_r", key), Value::String(v.clone()));
            }
        }

        let mut size_val: Option<(f32, f32)> = None;
        for i in *sep_pos..block_end.saturating_sub(7) {
            if let (Some(f1), Some(f2)) = (read_f32_le(nm, i), read_f32_le(nm, i + 4)) {
                if (16.0..=131072.0).contains(&f1) && (16.0..=131072.0).contains(&f2) {
                    size_val = Some((f1, f2));
                    break;
                }
            }
        }

        let mut detail_sizes = Vec::new();
        for i in *sep_pos..block_end.saturating_sub(3) {
            if let Some(fv) = read_f32_le(nm, i) {
                if (4.0..=128.0).contains(&fv) {
                    if let Some((sx, _)) = size_val {
                        if (fv - sx).abs() < 0.01 {
                            continue;
                        }
                    }
                    let rv = (fv * 100.0).round() / 100.0;
                    if !detail_sizes.iter().any(|v: &f32| (*v - rv).abs() < 0.001) {
                        detail_sizes.push(rv);
                    }
                    if detail_sizes.len() >= 4 {
                        break;
                    }
                }
            }
        }

        let lc = json!({
            "name": bname,
            "texture": texture,
            "details": Value::Object(details),
            "size": size_val.map(|(x,y)| vec![x, y]),
            "detailSizes": if detail_sizes.is_empty() { Value::Null } else { json!(detail_sizes) }
        });
        out.push(lc);
    }

    // Some maps contain duplicate landclass blocks with the same name where
    // one entry is sparse/empty. Merge duplicates and keep the richer data.
    let mut merged: Vec<Value> = Vec::new();
    for lc in out {
        let name = lc
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }

        let existing_idx = merged
            .iter()
            .position(|v| v.get("name").and_then(Value::as_str) == Some(name.as_str()));

        if let Some(idx) = existing_idx {
            if let (Some(dst), Some(src)) = (merged[idx].as_object_mut(), lc.as_object()) {
                // Prefer non-empty texture.
                let dst_tex_empty = dst
                    .get("texture")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true);
                if dst_tex_empty {
                    if let Some(src_tex) = src.get("texture").and_then(Value::as_str) {
                        if !src_tex.trim().is_empty() {
                            dst.insert("texture".to_string(), Value::String(src_tex.to_string()));
                        }
                    }
                }

                // Merge details keys (R/G/B/K and *_r).
                let mut merged_details = serde_json::Map::new();
                if let Some(d) = dst.get("details").and_then(Value::as_object) {
                    for (k, v) in d {
                        merged_details.insert(k.clone(), v.clone());
                    }
                }
                if let Some(d) = src.get("details").and_then(Value::as_object) {
                    for (k, v) in d {
                        if !merged_details.contains_key(k) {
                            merged_details.insert(k.clone(), v.clone());
                        }
                    }
                }
                dst.insert("details".to_string(), Value::Object(merged_details));

                // Prefer non-null size/detailSizes.
                if dst.get("size").map(Value::is_null).unwrap_or(true) {
                    if let Some(v) = src.get("size") {
                        if !v.is_null() {
                            dst.insert("size".to_string(), v.clone());
                        }
                    }
                }
                if dst.get("detailSizes").map(Value::is_null).unwrap_or(true) {
                    if let Some(v) = src.get("detailSizes") {
                        if !v.is_null() {
                            dst.insert("detailSizes".to_string(), v.clone());
                        }
                    }
                }
            }
        } else {
            merged.push(lc);
        }
    }

    merged
}

pub fn extract_landclass_native(data: &[u8]) -> Result<Option<Value>> {

    let hdr = match parse_lndm_header(&data) {
        Some(v) => v,
        None => return Ok(None),
    };

    if hdr.detail_data_ofs <= 0 || hdr.tile_data_ofs <= hdr.detail_data_ofs {
        return Ok(None);
    }

    let a = hdr.base_ofs + hdr.detail_data_ofs as usize;
    let b = (hdr.base_ofs + hdr.tile_data_ofs as usize).min(data.len());
    if a >= b || b - a < 16 {
        return Ok(None);
    }

    let dd = &data[a..b];
    let landclasses = parse_detail_blk(dd);

    Ok(Some(json!({
        "landclasses": landclasses,
        "lndm": {
            "gridCellSize": hdr.grid_cell_size,
            "landCellSize": hdr.land_cell_size,
            "mapSizeX": hdr.map_size_x,
            "mapSizeY": hdr.map_size_y
        }
    })))
}

