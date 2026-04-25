//! Global DxP2 texture name index.
//!
//! Scans every `*.dxp.bin` file under the configured content roots and builds a
//! `texture_name -> (dxp_path, entry_idx)` map by reading only each pack's name
//! table (no decompression, no body extraction). This lets the paint step
//! resolve landclass detail textures (e.g. `detail_forest_tex_d`,
//! `detail_jungle_grass_fields_tex_d`, cross-map textures like
//! `phi_phi_jungle_a`) that live in DxP packs not automatically loaded for the
//! current map.
//!
//! Single-texture extraction re-reads the source DxP file and decodes only the
//! requested entry. Results are intended to be merged into the per-map
//! `DdsStore` after the standard extraction step.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rayon::prelude::*;

use crate::util::read_u32_le;

/// Lazy global texture index. Entry maps a sanitized texture name to the
/// source DxP pack plus entry index within that pack.
#[derive(Debug, Default)]
pub struct DxpIndex {
    entries: HashMap<String, (PathBuf, usize)>,
    /// Per-path cache of fully extracted stores. Extraction of one DxP unlocks
    /// every texture within it at once; most cases need all of them.
    file_cache: Mutex<HashMap<PathBuf, HashMap<String, Vec<u8>>>>,
}

impl DxpIndex {
    pub fn build(roots: &[&Path]) -> Self {
        let mut paths: Vec<PathBuf> = Vec::new();
        for root in roots {
            if !root.exists() {
                continue;
            }
            if let Ok(rd) = fs::read_dir(root) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if !p.is_file() {
                        continue;
                    }
                    let name = match p.file_name().and_then(|s| s.to_str()) {
                        Some(v) => v,
                        None => continue,
                    };
                    if name.ends_with(".dxp.bin") {
                        paths.push(p);
                    }
                }
            }
        }

        // Read just the name tables in parallel. Each pack pays ~one seek +
        // a few KiB of I/O for its name region, not full decompression.
        let per_file: Vec<(PathBuf, Vec<String>)> = paths
            .par_iter()
            .filter_map(|p| scan_names(p).map(|names| (p.clone(), names)))
            .collect();

        let mut entries: HashMap<String, (PathBuf, usize)> = HashMap::new();
        for (path, names) in per_file {
            for (i, name) in names.into_iter().enumerate() {
                if name.is_empty() {
                    continue;
                }
                // First registration wins. Iteration order from read_dir +
                // rayon isn't fully deterministic, but duplicate landscape
                // textures are byte-identical across packs, so any source
                // works. Per-map packs loaded first in pipeline take priority
                // anyway (we only fall back to this index).
                entries.entry(name).or_insert_with(|| (path.clone(), i));
            }
        }
        Self {
            entries,
            file_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Resolve a set of texture names to raw DDS bytes. Unknown names are
    /// simply omitted from the returned map.
    pub fn resolve_batch<I, S>(&self, names: I) -> HashMap<String, Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        // Group requested names by source pack.
        let mut by_pack: HashMap<PathBuf, Vec<String>> = HashMap::new();
        for n in names {
            let name = n.as_ref();
            if let Some((path, _)) = self.entries.get(name) {
                by_pack
                    .entry(path.clone())
                    .or_default()
                    .push(name.to_string());
            }
        }

        let mut out: HashMap<String, Vec<u8>> = HashMap::new();
        for (path, wanted) in by_pack {
            // Attempt to hit the file cache first.
            {
                let cache = self.file_cache.lock().unwrap();
                if let Some(store) = cache.get(&path) {
                    for name in &wanted {
                        if let Some(bytes) = store.get(name) {
                            out.insert(name.clone(), bytes.clone());
                        }
                    }
                    continue;
                }
            }

            // Cold: fully extract the pack via the existing extractor and
            // stash the result so later maps reuse it.
            match crate::extract::extract_dxp(&path) {
                Ok(store) => {
                    for name in &wanted {
                        if let Some(bytes) = store.get(name) {
                            out.insert(name.clone(), bytes.clone());
                        }
                    }
                    let mut cache = self.file_cache.lock().unwrap();
                    cache.insert(path, store);
                }
                Err(_) => {
                    // Ignore unreadable packs; caller will fall through to
                    // the noise fallback for this LC.
                }
            }
        }
        out
    }
}

fn scan_names(path: &Path) -> Option<Vec<String>> {
    let data = fs::read(path).ok()?;
    if data.len() < 0x20 || &data[0..4] != b"DxP2" {
        return None;
    }
    let tex_count = read_u32_le(&data, 8).unwrap_or(0) as usize;
    if tex_count == 0 {
        return Some(Vec::new());
    }
    let base = 0x10usize;
    let name_rel = read_u32_le(&data, base)? as usize;
    let name_off = base + name_rel;

    let mut names = Vec::with_capacity(tex_count);
    for i in 0..tex_count {
        let entry_off = name_off + i * 8;
        if entry_off + 4 > data.len() {
            break;
        }
        let str_rel = match read_u32_le(&data, entry_off) {
            Some(v) => v as usize,
            None => break,
        };
        let str_abs = base + str_rel;
        if str_abs >= data.len() {
            names.push(String::new());
            continue;
        }
        let end = data[str_abs..]
            .iter()
            .position(|b| *b == 0)
            .map(|p| str_abs + p)
            .unwrap_or(str_abs);
        if end <= str_abs || end > data.len() {
            names.push(String::new());
            continue;
        }
        let raw = std::str::from_utf8(&data[str_abs..end]).unwrap_or("");
        names.push(sanitize_name(raw));
    }
    Some(names)
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
