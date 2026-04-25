// .bin file inspector: walks a DBLD container and reports every known
// structure, plus hex-dumps anything that sits between them as "unknown"
// so we can study undocumented regions.
//
// Output is a plain-text report written next to the map output folder.
//
// References:
//   - DBLD block chain   (see src/heightmap.rs :: find_hm2_block)
//   - lndm v4 header     (see src/heightmap.rs, src/landclass.rs)
//   - LandRayTracer dump (see src/heightmap.rs :: extract_land_ray_tracer_dump)
//   - DDSx tile stream   (see src/extract.rs)
//   - RIGz render-inst   (see src/rendinst.rs)

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::util::{read_f32_le, read_i32_le, read_u32_le};

const DBLD_MAGIC: &[u8; 4] = b"DBLD";
const DDSX_MAGIC: &[u8; 4] = b"DDSx";
const DDSX_HEADER: usize = 32;
const LNDM_MAGIC: &[u8; 4] = b"lndm";
const LT_DUMP_MAGIC: &[u8; 6] = b"LTdump";
const CELL_PREFIX: usize = 15;

/// A contiguous range of the bin file that the inspector has accounted for.
/// Gaps between spans get reported as "unknown".
#[derive(Debug, Clone)]
struct Span {
    start: usize,
    end: usize,
    #[allow(dead_code)]
    label: String,
}

struct Report {
    text: String,
    spans: Vec<Span>,
    total: usize,
}

impl Report {
    fn new(total: usize) -> Self {
        Self { text: String::new(), spans: Vec::new(), total }
    }

    fn line(&mut self, s: impl AsRef<str>) {
        self.text.push_str(s.as_ref());
        self.text.push('\n');
    }

    fn section(&mut self, title: &str) {
        self.text.push('\n');
        self.line(format!("── {title} {}", "─".repeat(72usize.saturating_sub(title.len() + 4))));
    }

    fn add_span(&mut self, start: usize, end: usize, label: impl Into<String>) {
        let end = end.min(self.total);
        if end <= start {
            return;
        }
        self.spans.push(Span { start, end, label: label.into() });
    }
}

fn fmt_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    if n >= MB {
        format!("{:.2} MiB ({n} B)", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.2} KiB ({n} B)", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}

fn hex_preview(data: &[u8], pos: usize, max_rows: usize) -> String {
    const ROW: usize = 16;
    let mut out = String::new();
    let end = (pos + max_rows * ROW).min(data.len());
    let mut p = pos;
    while p < end {
        let row_end = (p + ROW).min(end);
        let _ = write!(out, "    {:08x}  ", p);
        for i in 0..ROW {
            if p + i < row_end {
                let _ = write!(out, "{:02x} ", data[p + i]);
            } else {
                out.push_str("   ");
            }
        }
        out.push(' ');
        for i in 0..ROW {
            if p + i < row_end {
                let b = data[p + i];
                out.push(if (0x20..0x7f).contains(&b) { b as char } else { '.' });
            }
        }
        out.push('\n');
        p = row_end;
    }
    out
}

fn printable_strings(data: &[u8], pos: usize, len: usize, min_len: usize, max_count: usize) -> Vec<(usize, String)> {
    let end = (pos + len).min(data.len());
    let mut out = Vec::new();
    let mut i = pos;
    while i < end && out.len() < max_count {
        if (0x20..0x7f).contains(&data[i]) {
            let mut j = i;
            while j < end && (0x20..0x7f).contains(&data[j]) {
                j += 1;
            }
            if j - i >= min_len {
                let s = String::from_utf8_lossy(&data[i..j]).to_string();
                out.push((i, s));
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

fn tag_preview(tag: &[u8]) -> String {
    let mut s = String::new();
    for &b in tag {
        s.push(if (0x20..0x7f).contains(&b) { b as char } else { '.' });
    }
    format!("'{s}' ({})", tag.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "))
}

// ── DBLD block chain ──────────────────────────────────────────────────────

fn report_dbld_chain(r: &mut Report, data: &[u8]) {
    r.section("DBLD container block chain");
    if data.len() < 12 || &data[0..4] != DBLD_MAGIC {
        r.line("  ! File does not start with 'DBLD' magic — not a DBLD container?");
        return;
    }

    r.line(format!("  Magic      : 'DBLD' at 0x00000000"));
    r.line(format!("  Header[0x04..0x0c] (8 bytes, unverified meaning):"));
    r.text.push_str(&hex_preview(data, 4, 1));
    r.add_span(0, 0x0c, String::from("DBLD header"));

    let mut pos = 0x0cusize;
    let mut idx = 0;
    while pos + 8 <= data.len() {
        let lf = match read_u32_le(data, pos) {
            Some(v) => v,
            None => break,
        };
        let blen = (lf & 0x3fff_ffff) as usize;
        let flags = lf >> 30;
        if blen < 4 || blen > data.len().saturating_sub(pos) {
            r.line(format!("  [{idx}] 0x{pos:08x}  <invalid length {blen}>"));
            break;
        }
        let tag = &data[pos + 4..pos + 8];
        let tag_str = tag_preview(tag);
        r.line(format!(
            "  [{idx}] 0x{pos:08x}  len={blen:>10}  flags={flags}  tag={tag_str}",
        ));
        // Only the 8-byte block header is "known". The payload is left as an
        // unknown region unless a specialized pass claims it.
        r.add_span(pos, pos + 8, format!("DBLD block[{idx}] header tag={tag_str}"));

        // END-like terminator: stop to avoid walking into padding.
        if tag.contains(&b'E') && tag.contains(&b'N') && tag.contains(&b'D') {
            r.line(format!("       (END-like tag — stopping block walk)"));
            break;
        }

        pos += 4 + blen;
        idx += 1;
    }
}

// ── lndm v4 header ────────────────────────────────────────────────────────

fn report_lndm(r: &mut Report, data: &[u8]) -> Option<(usize, i32, i32, usize)> {
    let pos = data.windows(4).position(|w| w == LNDM_MAGIC)?;
    r.section("lndm (landmesh) header");
    r.line(format!("  Location   : 0x{pos:08x}"));

    let mut p = pos + 4;
    let version = read_i32_le(data, p)?;
    p += 4;
    r.line(format!("  version        : {version}"));
    if version != 4 {
        r.line(format!("  ! unsupported lndm version — extractor assumes v4"));
        return None;
    }

    let grid_cell_size = read_f32_le(data, p)?;
    p += 4;
    let land_cell_size = read_f32_le(data, p)?;
    p += 4;
    let map_x = read_i32_le(data, p)?;
    p += 4;
    let map_y = read_i32_le(data, p)?;
    p += 4;
    let origin_x = read_i32_le(data, p)?;
    p += 4;
    let origin_y = read_i32_le(data, p)?;
    p += 4;
    let use_tile = read_i32_le(data, p)?;
    p += 4;

    let base_ofs = p;
    let mesh_map_ofs = read_i32_le(data, p)?;
    p += 4;
    let detail_data_ofs = read_i32_le(data, p)?;
    p += 4;
    let tile_data_ofs = read_i32_le(data, p)?;
    p += 4;
    let ray_tracer_ofs = read_i32_le(data, p)?;
    p += 4;

    r.line(format!("  gridCellSize   : {grid_cell_size}"));
    r.line(format!("  landCellSize   : {land_cell_size}"));
    r.line(format!("  mapSize        : {map_x} x {map_y}  → {} cells", map_x.saturating_mul(map_y)));
    r.line(format!("  origin         : ({origin_x}, {origin_y})"));
    r.line(format!("  useTile        : {use_tile}"));
    r.line(format!("  base_ofs       : 0x{base_ofs:08x}  (relative offsets are added to this)"));
    r.line(format!("  meshMapOfs     : {mesh_map_ofs}  → abs 0x{:08x}", (base_ofs as isize + mesh_map_ofs as isize) as usize));
    r.line(format!("  detailDataOfs  : {detail_data_ofs}  → abs 0x{:08x}", (base_ofs as isize + detail_data_ofs as isize) as usize));
    r.line(format!("  tileDataOfs    : {tile_data_ofs}  → abs 0x{:08x}", (base_ofs as isize + tile_data_ofs as isize) as usize));
    r.line(format!("  rayTracerOfs   : {ray_tracer_ofs}  → abs 0x{:08x}", (base_ofs as isize + ray_tracer_ofs as isize) as usize));

    let det_abs = (base_ofs as isize + detail_data_ofs as isize) as usize;
    r.line(format!("  Trailer after declared offsets (next 32 bytes, often undocumented):"));
    r.text.push_str(&hex_preview(data, p, 2));
    r.add_span(pos, p + 32, String::from("lndm v4 header"));

    Some((det_abs, map_x, map_y, base_ofs))
}

// ── LandRayTracer (LTdump) ────────────────────────────────────────────────

fn report_ltdump(r: &mut Report, data: &[u8]) {
    let Some(pos) = data.windows(6).position(|w| w == LT_DUMP_MAGIC) else {
        return;
    };
    r.section("LTdump (LandRayTracer)");
    r.line(format!("  Location   : 0x{pos:08x}"));

    let mut p = pos + 6;
    let num_cx = read_i32_le(data, p).unwrap_or(0);
    p += 4;
    let num_cy = read_i32_le(data, p).unwrap_or(0);
    p += 4;
    let cell_size = read_f32_le(data, p).unwrap_or(0.0);
    p += 4;
    let off_x = read_f32_le(data, p).unwrap_or(0.0);
    let off_y = read_f32_le(data, p + 4).unwrap_or(0.0);
    let off_z = read_f32_le(data, p + 8).unwrap_or(0.0);
    p += 12;
    // bmin / bmax (12 bytes each, 3xf32)
    p += 12 + 12;
    let cells_count = read_i32_le(data, p).unwrap_or(0);

    r.line(format!("  numCX / numCY  : {num_cx} x {num_cy}"));
    r.line(format!("  cellSize       : {cell_size}"));
    r.line(format!("  offset         : ({off_x}, {off_y}, {off_z})"));
    r.line(format!("  cells_count    : {cells_count}  (each cell entry is 64 bytes)"));
    r.add_span(pos, p + 4, String::from("LTdump header"));
}

// ── HM2 block ─────────────────────────────────────────────────────────────

fn report_hm2(r: &mut Report, data: &[u8]) {
    if data.len() < 12 || &data[0..4] != DBLD_MAGIC {
        return;
    }
    let mut pos = 0x0cusize;
    while pos + 8 <= data.len() {
        let lf = match read_u32_le(data, pos) {
            Some(v) => v,
            None => break,
        };
        let blen = (lf & 0x3fff_ffff) as usize;
        if blen < 4 || blen > data.len().saturating_sub(pos) {
            break;
        }
        let tag = &data[pos + 4..pos + 8];
        if tag == b"\0HM2" {
            r.section("HM2 (compressed heightmap)");
            r.line(format!("  Block start   : 0x{pos:08x}"));
            r.line(format!("  Tag           : {}", tag_preview(tag)));
            r.line(format!("  Block length  : {blen}"));
            r.line(format!("  Payload start : 0x{:08x}  payload length : {}", pos + 8, blen - 4));
            r.line(format!("  First 32 bytes of payload:"));
            r.text.push_str(&hex_preview(data, pos + 8, 2));
            return;
        }
        if tag.contains(&b'E') && tag.contains(&b'N') && tag.contains(&b'D') {
            break;
        }
        pos += 4 + blen;
    }
}

// ── RIGz block (render instances) ─────────────────────────────────────────

fn report_rigz(r: &mut Report, data: &[u8]) {
    if data.len() < 12 || &data[0..4] != DBLD_MAGIC {
        return;
    }
    let mut pos = 0x0cusize;
    while pos + 8 <= data.len() {
        let lf = match read_u32_le(data, pos) {
            Some(v) => v,
            None => break,
        };
        let blen = (lf & 0x3fff_ffff) as usize;
        if blen < 4 || blen > data.len().saturating_sub(pos) {
            break;
        }
        let tag = &data[pos + 4..pos + 8];
        if tag == b"RIGz" {
            r.section("RIGz (render instances)");
            r.line(format!("  Block start   : 0x{pos:08x}"));
            r.line(format!("  Block length  : {blen}"));

            let payload = &data[pos + 8..pos + 4 + blen];
            if payload.len() >= 8 {
                let sb1_lf = read_u32_le(payload, 0).unwrap_or(0);
                let sb1_flags = sb1_lf >> 30;
                let sb1_len = (sb1_lf & 0x3fff_ffff) as usize;
                r.line(format!("  sub1: flags={sb1_flags} (0=lzma, 1=zstd, 2=oodle, 3=raw), len={sb1_len}"));
                let sb2_off = 4 + sb1_len;
                if sb2_off + 4 <= payload.len() {
                    let sb2_lf = read_u32_le(payload, sb2_off).unwrap_or(0);
                    let sb2_flags = sb2_lf >> 30;
                    let sb2_len = (sb2_lf & 0x3fff_ffff) as usize;
                    r.line(format!("  sub2: flags={sb2_flags}, len={sb2_len}"));
                }
            }
            return;
        }
        if tag.contains(&b'E') && tag.contains(&b'N') && tag.contains(&b'D') {
            break;
        }
        pos += 4 + blen;
    }
}

// ── DDSx tile stream ──────────────────────────────────────────────────────

fn report_ddsx_stream(r: &mut Report, data: &[u8]) -> Vec<usize> {
    r.section("DDSx texture blocks");
    let mut positions = Vec::new();
    let mut s = 0usize;
    while s + DDSX_HEADER <= data.len() {
        let rel = match data[s..].windows(4).position(|w| w == DDSX_MAGIC) {
            Some(v) => v,
            None => break,
        };
        let p = s + rel;
        if p + DDSX_HEADER > data.len() {
            break;
        }
        let mem_sz = read_u32_le(data, p + 24).unwrap_or(0) as usize;
        let packed_sz = read_u32_le(data, p + 28).unwrap_or(0) as usize;
        let body = if packed_sz == 0 { mem_sz } else { packed_sz };
        let end = p + DDSX_HEADER + body;
        if end > data.len() {
            s = p + 1;
            continue;
        }
        positions.push(p);
        r.add_span(p, end, format!("DDSx[{}]", positions.len() - 1));
        s = end;
    }
    r.line(format!("  Count         : {}", positions.len()));
    if !positions.is_empty() {
        r.line(format!("  First         : 0x{:08x}", positions[0]));
        r.line(format!("  Last          : 0x{:08x}", positions[positions.len() - 1]));
    }
    // First 10 entries detailed
    for (i, p) in positions.iter().take(10).enumerate() {
        let mem_sz = read_u32_le(data, p + 24).unwrap_or(0);
        let packed_sz = read_u32_le(data, p + 28).unwrap_or(0);
        r.line(format!(
            "    [{i}] 0x{p:08x}  mem_sz={mem_sz:>10}  packed_sz={packed_sz:>10}",
        ));
    }
    if positions.len() > 10 {
        r.line(format!("    … {} more", positions.len() - 10));
    }
    positions
}

// ── Terrain-paint cell prefixes ───────────────────────────────────────────

fn report_cells(r: &mut Report, data: &[u8], ddsx_positions: &[usize], num_cells: usize) {
    if ddsx_positions.len() < 2 || num_cells == 0 {
        return;
    }
    r.section("Terrain-paint cell walk (15-byte prefixes before each DDSx tile)");

    let first_pop_pos = ddsx_positions[1].saturating_sub(CELL_PREFIX);
    let mut n_pre_empty = 0usize;
    // Same backward scan that paint.rs performs.
    let mut check = first_pop_pos.saturating_sub(CELL_PREFIX);
    while check + 15 <= data.len() && n_pre_empty < num_cells {
        let total_len = read_u32_le(data, check + 7).unwrap_or(0);
        if total_len != 0 {
            break;
        }
        n_pre_empty += 1;
        if check < CELL_PREFIX {
            break;
        }
        check -= CELL_PREFIX;
    }
    r.line(format!("  first populated cell prefix @ 0x{first_pop_pos:08x}"));
    r.line(format!("  leading empty cells (back-scan) : {n_pre_empty}"));

    let walk_start = first_pop_pos.saturating_sub(n_pre_empty * CELL_PREFIX);
    r.line(format!("  walk start (cell 0)             : 0x{walk_start:08x}"));

    let mut walk = walk_start;
    let mut idx = 0usize;
    let mut non_empty = 0usize;
    let mut sample: Vec<String> = Vec::new();
    while idx < num_cells && walk + CELL_PREFIX <= data.len() {
        let det: [u8; 7] = data[walk..walk + 7].try_into().unwrap_or([0; 7]);
        let total_len = read_u32_le(data, walk + 7).unwrap_or(0) as usize;
        let tex2_off = read_u32_le(data, walk + 11).unwrap_or(0) as usize;
        if total_len == 0 {
            r.add_span(walk, walk + CELL_PREFIX, format!("cell[{idx}] prefix (empty)"));
            walk += CELL_PREFIX;
        } else {
            non_empty += 1;
            if sample.len() < 8 {
                sample.push(format!(
                    "    cell[{idx}] @ 0x{walk:08x}  det={:?}  total_len={total_len}  tex2_off={tex2_off}",
                    det
                ));
            }
            r.add_span(walk, walk + CELL_PREFIX, format!("cell[{idx}] prefix"));
            walk += CELL_PREFIX + total_len;
        }
        idx += 1;
    }
    for s in &sample {
        r.line(s);
    }
    r.line(format!("  cells visited : {idx} / {num_cells}"));
    r.line(format!("  populated     : {non_empty}"));
}

// ── Unknown-region reporter ───────────────────────────────────────────────

fn report_unknown_regions(r: &mut Report, data: &[u8]) {
    r.section("Unknown regions (gaps between cataloged structures)");
    // Sort spans by start, merge overlaps
    let mut spans = r.spans.clone();
    spans.sort_by_key(|s| s.start);
    let mut merged: Vec<Span> = Vec::new();
    for s in spans {
        if let Some(last) = merged.last_mut() {
            if s.start <= last.end {
                if s.end > last.end {
                    last.end = s.end;
                }
                continue;
            }
        }
        merged.push(s);
    }

    let mut cursor = 0usize;
    let mut gap_idx = 0usize;
    let total = data.len();
    let mut total_unknown: u64 = 0;

    let emit_gap = |r: &mut Report, gap_idx: &mut usize, total_unknown: &mut u64, start: usize, end: usize| {
        if end <= start {
            return;
        }
        let len = end - start;
        *total_unknown += len as u64;
        let strings = printable_strings(data, start, len, 6, 12);
        r.line(format!(
            "  [gap #{}] 0x{start:08x} .. 0x{end:08x}  ({} bytes)",
            gap_idx,
            len
        ));
        if !strings.is_empty() {
            r.line(format!("    printable strings (len>=6, up to 12):"));
            for (off, s) in strings {
                // Trim very long strings
                let trimmed = if s.len() > 96 { format!("{}…", &s[..96]) } else { s };
                r.line(format!("      0x{off:08x}  {trimmed:?}"));
            }
        }
        // First two rows of hex for context
        r.text.push_str(&hex_preview(data, start, 2));
        *gap_idx += 1;
    };

    for s in &merged {
        if s.start > cursor {
            emit_gap(r, &mut gap_idx, &mut total_unknown, cursor, s.start);
        }
        if s.end > cursor {
            cursor = s.end;
        }
    }
    if cursor < total {
        emit_gap(r, &mut gap_idx, &mut total_unknown, cursor, total);
    }

    r.line(format!(
        "  Total unknown bytes : {} ({:.2}% of file)",
        fmt_bytes(total_unknown),
        (total_unknown as f64 / total as f64) * 100.0
    ));
}

// ── Public entry point ────────────────────────────────────────────────────

pub fn inspect_bin_file(bin_path: &Path, out_path: &Path) -> Result<()> {
    let data = fs::read(bin_path)
        .with_context(|| format!("Failed to read {}", bin_path.display()))?;
    let mut r = Report::new(data.len());

    r.line(format!("WT-MapExtractor bin inspector"));
    r.line(format!("============================="));
    r.line(format!("File        : {}", bin_path.display()));
    r.line(format!("File size   : {}", fmt_bytes(data.len() as u64)));

    report_dbld_chain(&mut r, &data);
    report_hm2(&mut r, &data);
    let lndm_info = report_lndm(&mut r, &data);
    report_ltdump(&mut r, &data);
    report_rigz(&mut r, &data);
    let ddsx_positions = report_ddsx_stream(&mut r, &data);

    if let Some((_det_abs, mx, my, _base)) = lndm_info {
        let num_cells = (mx.max(0) as usize) * (my.max(0) as usize);
        report_cells(&mut r, &data, &ddsx_positions, num_cells);
    }

    report_unknown_regions(&mut r, &data);

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(out_path, &r.text)
        .with_context(|| format!("Failed to write {}", out_path.display()))?;
    Ok(())
}
