use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use flate2::read::ZlibDecoder;

use crate::util::{oodle_decompress, read_u32_le};

const DDSX_MAGIC: &[u8; 4] = b"DDSx";
const DDSX_HEADER_SIZE: usize = 32;

const FMT_DXT1: &[u8; 4] = b"DXT1";
const FMT_DXT5: &[u8; 4] = b"DXT5";
const FMT_A4R4G4B4: [u8; 4] = [0x1a, 0x00, 0x00, 0x00];
const FMT_A8R8G8B8: [u8; 4] = [0x15, 0x00, 0x00, 0x00];
const FMT_L8: [u8; 4] = [0x32, 0x00, 0x00, 0x00];
const FMT_BC7: &[u8; 4] = b"BC7 ";

#[derive(Debug, Clone)]
struct DdsxHeader {
    fmt: [u8; 4],
    flags: u32,
    w: u16,
    h: u16,
    levels: u8,
    bpp: u16,
    mem_sz: u32,
    packed_sz: u32,
}

fn parse_header(data: &[u8]) -> Option<DdsxHeader> {
    if data.len() < DDSX_HEADER_SIZE || &data[0..4] != DDSX_MAGIC {
        return None;
    }
    Some(DdsxHeader {
        fmt: [data[4], data[5], data[6], data[7]],
        flags: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
        w: u16::from_le_bytes([data[12], data[13]]),
        h: u16::from_le_bytes([data[14], data[15]]),
        levels: data[16],
        bpp: u16::from_le_bytes([data[20], data[21]]),
        mem_sz: u32::from_le_bytes([data[24], data[25], data[26], data[27]]),
        packed_sz: u32::from_le_bytes([data[28], data[29], data[30], data[31]]),
    })
}

fn is_supported_format(fmt: &[u8; 4]) -> bool {
    fmt == FMT_DXT1
        || fmt == FMT_DXT5
        || fmt == &FMT_A4R4G4B4
        || fmt == &FMT_A8R8G8B8
        || fmt == &FMT_L8
        || fmt == FMT_BC7
}

fn comp_type(raw: u8) -> &'static str {
    match raw & 0xe0 {
        0x00 => "none",
        0x20 => "zstd",
        0x40 => "lzma",
        0x60 => "oodle",
        0x80 => "zlib",
        _ => "unknown",
    }
}

fn reverse_mips(pixels: Vec<u8>, hdr: &DdsxHeader) -> Vec<u8> {
    if hdr.levels <= 1 {
        return pixels;
    }
    let mut pos = 0usize;
    let mut mips = Vec::new();
    for level in (0..hdr.levels).rev() {
        let mw = std::cmp::max(1u32, (hdr.w as u32) >> level as u32);
        let mh = std::cmp::max(1u32, (hdr.h as u32) >> level as u32);
        let sz = if &hdr.fmt == FMT_DXT1 {
            let blocks = std::cmp::max(1, mw.div_ceil(4)) * std::cmp::max(1, mh.div_ceil(4));
            (blocks * 8) as usize
        } else if &hdr.fmt == FMT_DXT5 || &hdr.fmt == FMT_BC7 {
            let blocks = std::cmp::max(1, mw.div_ceil(4)) * std::cmp::max(1, mh.div_ceil(4));
            (blocks * 16) as usize
        } else {
            (mw * mh * std::cmp::max(1, (hdr.bpp / 8) as u32)) as usize
        };
        if pos + sz > pixels.len() {
            return pixels;
        }
        mips.push(pixels[pos..pos + sz].to_vec());
        pos += sz;
    }
    mips.reverse();
    mips.into_iter().flatten().collect()
}

fn dds_header_template() -> Vec<u8> {
    let mut dds = vec![0u8; 128];
    dds[0..4].copy_from_slice(b"DDS ");
    dds[4..8].copy_from_slice(&124u32.to_le_bytes());
    dds[8..12].copy_from_slice(&0x0008_1007u32.to_le_bytes());
    dds[28..32].copy_from_slice(&1u32.to_le_bytes());
    dds[76..80].copy_from_slice(&32u32.to_le_bytes());
    dds[80..84].copy_from_slice(&4u32.to_le_bytes());
    dds[108..112].copy_from_slice(&0x1000u32.to_le_bytes());
    dds
}

fn ddsx_to_dds(raw: &[u8]) -> Option<Vec<u8>> {
    let hdr = parse_header(raw)?;
    if !is_supported_format(&hdr.fmt) {
        return None;
    }

    let body_sz = if hdr.packed_sz != 0 {
        hdr.packed_sz as usize
    } else {
        hdr.mem_sz as usize
    };
    if raw.len() < DDSX_HEADER_SIZE + body_sz {
        return None;
    }
    let body = &raw[DDSX_HEADER_SIZE..DDSX_HEADER_SIZE + body_sz];

    let compression = comp_type(raw.get(0x0b).copied().unwrap_or(0));
    let mut pixels = match compression {
        "none" => body.to_vec(),
        "zstd" => zstd::stream::decode_all(body).ok()?,
        "zlib" => {
            let mut d = ZlibDecoder::new(body);
            let mut out = Vec::new();
            if d.read_to_end(&mut out).is_err() {
                return None;
            }
            out
        }
        "oodle" => oodle_decompress(body, hdr.mem_sz as usize)?,
        _ => return None,
    };

    if (hdr.flags & 0x40_000) != 0 {
        pixels = reverse_mips(pixels, &hdr);
    }

    let mut dds = dds_header_template();
    dds[0x0c..0x10].copy_from_slice(&(hdr.h as u32).to_le_bytes());
    dds[0x10..0x14].copy_from_slice(&(hdr.w as u32).to_le_bytes());
    dds[0x14..0x18].copy_from_slice(&hdr.mem_sz.to_le_bytes());
    dds[0x1c] = hdr.levels;

    if &hdr.fmt == FMT_DXT1 || &hdr.fmt == FMT_DXT5 {
        dds[0x54..0x58].copy_from_slice(&hdr.fmt);
        dds.extend_from_slice(&pixels);
        return Some(dds);
    }

    if hdr.fmt == FMT_A4R4G4B4 {
        dds[0x08..0x0c].copy_from_slice(&0x0000_100Fu32.to_le_bytes());
        dds[0x14..0x18].copy_from_slice(&((hdr.w as u32) * 2).to_le_bytes());
        dds[0x50..0x54].copy_from_slice(&0x41u32.to_le_bytes());
        dds[0x58..0x5c].copy_from_slice(&16u32.to_le_bytes());
        dds[0x5c..0x60].copy_from_slice(&0x0F00u32.to_le_bytes());
        dds[0x60..0x64].copy_from_slice(&0x00F0u32.to_le_bytes());
        dds[0x64..0x68].copy_from_slice(&0x000Fu32.to_le_bytes());
        dds[0x68..0x6c].copy_from_slice(&0xF000u32.to_le_bytes());
        dds.extend_from_slice(&pixels);
        return Some(dds);
    }

    if hdr.fmt == FMT_A8R8G8B8 {
        dds[0x08..0x0c].copy_from_slice(&0x0000_100Fu32.to_le_bytes());
        dds[0x14..0x18].copy_from_slice(&((hdr.w as u32) * 4).to_le_bytes());
        dds[0x50..0x54].copy_from_slice(&0x41u32.to_le_bytes());
        dds[0x58..0x5c].copy_from_slice(&32u32.to_le_bytes());
        dds[0x5c..0x60].copy_from_slice(&0x00FF_0000u32.to_le_bytes());
        dds[0x60..0x64].copy_from_slice(&0x0000_FF00u32.to_le_bytes());
        dds[0x64..0x68].copy_from_slice(&0x0000_00FFu32.to_le_bytes());
        dds[0x68..0x6c].copy_from_slice(&0xFF00_0000u32.to_le_bytes());
        dds.extend_from_slice(&pixels);
        return Some(dds);
    }

    if hdr.fmt == FMT_L8 {
        dds[0x08..0x0c].copy_from_slice(&0x0000_100Fu32.to_le_bytes());
        dds[0x14..0x18].copy_from_slice(&(hdr.w as u32).to_le_bytes());
        dds[0x50..0x54].copy_from_slice(&0x0002_0000u32.to_le_bytes());
        dds[0x58..0x5c].copy_from_slice(&8u32.to_le_bytes());
        dds[0x5c..0x60].copy_from_slice(&0xFFu32.to_le_bytes());
        dds.extend_from_slice(&pixels);
        return Some(dds);
    }

    if &hdr.fmt == FMT_BC7 {
        dds[0x54..0x58].copy_from_slice(b"DX10");
        let mut out = dds;
        out.extend_from_slice(&98u32.to_le_bytes());
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&pixels);
        return Some(out);
    }

    None
}

fn sanitize_name(s: &str) -> String {
    s.trim_end_matches('*')
        .replace('*', "")
        .replace('?', "_")
        .split('$')
        .next()
        .unwrap_or(s)
        .to_string()
}

/// Extract all DDSx entries from a binary blob into an in-memory DDS store.
/// Keys are `"{stem}_{idx:03}"` (no extension), values are raw DDS bytes.
/// Nothing is written to disk.
pub fn extract_ddsx_from_data(data: &[u8], stem: &str) -> HashMap<String, Vec<u8>> {
    let mut store: HashMap<String, Vec<u8>> = HashMap::new();
    let mut start = 0usize;
    let mut idx = 0usize;

    while let Some(rel) = data[start..].windows(4).position(|w| w == DDSX_MAGIC) {
        let pos = start + rel;
        if pos + DDSX_HEADER_SIZE > data.len() {
            break;
        }

        let mem_sz = match read_u32_le(data, pos + 24) {
            Some(v) => v as usize,
            None => break,
        };
        let packed_sz = match read_u32_le(data, pos + 28) {
            Some(v) => v as usize,
            None => break,
        };

        let body_sz = if packed_sz == 0 { mem_sz } else { packed_sz };
        let end = pos + DDSX_HEADER_SIZE + body_sz;
        if end > data.len() {
            start = pos + 1;
            continue;
        }

        if let Some(dds) = ddsx_to_dds(&data[pos..end]) {
            store.insert(format!("{stem}_{idx:03}"), dds);
            idx += 1;
        }

        start = end;
    }

    store
}

/// Extract all textures from a DxP2 pack into an in-memory DDS store.
/// Keys are texture names (no extension), values are raw DDS bytes.
/// Nothing is written to disk.
pub fn extract_dxp(dxp_path: &Path) -> Result<HashMap<String, Vec<u8>>> {
    let data = fs::read(dxp_path)
        .with_context(|| format!("Failed to read {}", dxp_path.display()))?;
    if data.len() < 0x20 || &data[0..4] != b"DxP2" {
        return Ok(HashMap::new());
    }

    let tex_count = read_u32_le(&data, 8).unwrap_or(0) as usize;
    if tex_count == 0 {
        return Ok(HashMap::new());
    }

    let base = 0x10usize;
    let section = |idx: usize| -> Option<usize> {
        let off = base + idx * 16;
        Some(base + read_u32_le(&data, off)? as usize)
    };

    let name_off = match section(0) {
        Some(v) => v,
        None => return Ok(HashMap::new()),
    };
    let ddsx_off = match section(1) {
        Some(v) => v,
        None => return Ok(HashMap::new()),
    };
    let body_off = match section(2) {
        Some(v) => v,
        None => return Ok(HashMap::new()),
    };

    let mut store: HashMap<String, Vec<u8>> = HashMap::new();
    let mut name_counts: HashMap<String, usize> = HashMap::new();

    for i in 0..tex_count {
        let str_rel = match read_u32_le(&data, name_off + i * 8) {
            Some(v) => v as usize,
            None => continue,
        };
        let str_abs = base + str_rel;
        if str_abs >= data.len() {
            continue;
        }
        let end = data[str_abs..]
            .iter()
            .position(|b| *b == 0)
            .map(|p| str_abs + p)
            .unwrap_or(str_abs);
        if end <= str_abs || end > data.len() {
            continue;
        }

        let raw_name = std::str::from_utf8(&data[str_abs..end]).unwrap_or("tex");
        let mut name = sanitize_name(raw_name);
        if name.is_empty() {
            name = format!("tex_{i:04}");
        }

        let hdr_start = ddsx_off + i * 32;
        if hdr_start + 32 > data.len() {
            continue;
        }
        let hdr = &data[hdr_start..hdr_start + 32];

        let bo = match read_u32_le(&data, body_off + i * 24 + 12) {
            Some(v) => v as usize,
            None => continue,
        };
        let bs = match read_u32_le(&data, body_off + i * 24 + 16) {
            Some(v) => v as usize,
            None => continue,
        };
        if bo + bs > data.len() {
            continue;
        }

        let mut packed = Vec::with_capacity(32 + bs);
        packed.extend_from_slice(hdr);
        packed.extend_from_slice(&data[bo..bo + bs]);

        if let Some(dds) = ddsx_to_dds(&packed) {
            let n = name_counts.entry(name.clone()).or_insert(0);
            let final_name = if *n == 0 {
                name.clone()
            } else {
                format!("{}_{}", name, *n)
            };
            *n += 1;
            // Only insert if not already present: HQ packs loaded first take priority.
            store.entry(final_name).or_insert(dds);
        }
    }

    Ok(store)
}
