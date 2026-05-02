use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};

fn modes_for_name(name: &str) -> &'static [&'static str] {
    let n = name.to_ascii_lowercase();
    if n.contains("hardcore") {
        &["hardcore"]
    } else {
        // Many mission assets are tagged as arcade but are used by realistic too.
        &["arcade", "hardcore"]
    }
}

fn mission_kind_from_stem(stem: &str) -> &'static str {
    let s = stem.to_ascii_lowercase();
    if s.contains("_bttl") {
        "bttl"
    } else {
        "other"
    }
}

fn is_bttl_briefing_respawn(area_name_lower: &str) -> bool {
    area_name_lower.starts_with("briefing_t") && area_name_lower.contains("_resp")
}

fn is_bttl_capture_area(area_name_lower: &str) -> bool {
    area_name_lower.starts_with("bttl_t1_capture_area") || area_name_lower.starts_with("bttl_t2_capture_area")
}

fn team_from_name(name: &str) -> i32 {
    let n = name.to_ascii_lowercase();
    let tokens: Vec<&str> = n
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();

    if tokens.iter().any(|t| *t == "t1" || *t == "team1") {
        1
    } else if tokens.iter().any(|t| *t == "t2" || *t == "team2") {
        2
    } else {
        0
    }
}

fn tm_pos_xz(v: &Value) -> Option<[f64; 2]> {
    let tm = v.get("tm")?.as_array()?;
    if tm.len() < 4 {
        return None;
    }
    let pos = tm[3].as_array()?;
    if pos.len() < 3 {
        return None;
    }
    Some([pos[0].as_f64()?, pos[2].as_f64()?])
}

fn tm_box_half_size_xz(v: &Value) -> Option<[f64; 2]> {
    let tm = v.get("tm")?.as_array()?;
    if tm.len() < 3 {
        return None;
    }
    let row0 = tm[0].as_array()?;
    let row2 = tm[2].as_array()?;
    if row0.len() < 3 || row2.len() < 3 {
        return None;
    }
    let hx = (row0[0].as_f64()?.powi(2) + row0[2].as_f64()?.powi(2)).sqrt();
    let hz = (row2[0].as_f64()?.powi(2) + row2[2].as_f64()?.powi(2)).sqrt();
    Some([hx, hz])
}

fn tm_cylinder_radius(v: &Value) -> Option<f64> {
    let tm = v.get("tm")?.as_array()?;
    if tm.is_empty() {
        return None;
    }
    let row0 = tm[0].as_array()?;
    if row0.len() < 3 {
        return None;
    }
    Some((row0[0].as_f64()?.powi(2) + row0[2].as_f64()?.powi(2)).sqrt())
}

/// Scans `name` for the first `_<digits>` segment and converts the number to a
/// letter label (1→"A", 2→"B", …).  Returns `None` when no numeric segment exists.
fn numeric_label_from_name(name: &str) -> Option<String> {
    let b = name.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'_' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
            let start = i + 1;
            let mut end = start;
            while end < b.len() && b[end].is_ascii_digit() {
                end += 1;
            }
            if let Ok(n) = name[start..end].parse::<usize>() {
                let ch = (b'A' + ((n.saturating_sub(1)) % 26) as u8) as char;
                return Some(ch.to_string());
            }
        }
        i += 1;
    }
    None
}

fn mission_label_from_stem(stem: &str, suffix: &str) -> String {
    let lower = stem.to_ascii_lowercase();
    let pref = format!("{}_", suffix.to_ascii_lowercase());
    let short = lower.strip_prefix(&pref).unwrap_or(&lower);
    short.to_string()
}

fn parse_one_mission_file(path: &Path, suffix: &str) -> Option<Value> {
    let txt = fs::read_to_string(path).ok()?;
    let root: Value = serde_json::from_str(&txt).ok()?;

    let stem = path.file_stem()?.to_string_lossy().to_string();
    let label = mission_label_from_stem(&stem, suffix);
    let mission_kind = mission_kind_from_stem(&stem);

    let mut spawn_name_set: HashSet<String> = HashSet::new();
    let mut spawns: Vec<Value> = Vec::new();
    if let Some(area_squad) = root
        .get("units")
        .and_then(|u| u.get("area_squad"))
        .and_then(Value::as_array)
    {
        for sq in area_squad {
            let name = sq.get("name").and_then(Value::as_str).unwrap_or("");
            let name_lower = name.to_ascii_lowercase();
            // Keep only real spawn-like entities; area_squad often includes kill areas.
            if name_lower.contains("killarea")
                || !(name_lower.contains("spawn") || name_lower.contains("resp"))
            {
                continue;
            }
            let pos = match tm_pos_xz(sq) {
                Some(v) => v,
                None => continue,
            };
            let team = team_from_name(name);
            for mode in modes_for_name(name) {
                spawns.push(json!({
                    "name": name,
                    "mode": mode,
                    "team": team,
                    "pos": [pos[0], pos[1]],
                }));
            }

            if let Some(members) = sq
                .get("props")
                .and_then(|p| p.get("squad_members"))
                .and_then(Value::as_array)
            {
                for m in members {
                    if let Some(s) = m.as_str() {
                        spawn_name_set.insert(s.to_string());
                    }
                }
            }
        }
    }

    let mut spawn_zones: Vec<Value> = Vec::new();
    let mut capture_zones: Vec<Value> = Vec::new();
    let mut battle_areas: Vec<Value> = Vec::new();
    if let Some(areas) = root.get("areas").and_then(Value::as_object) {
        for (name, area) in areas {
            let l = name.to_ascii_lowercase();
            let include_spawn = if mission_kind == "bttl" {
                is_bttl_briefing_respawn(&l)
            } else {
                spawn_name_set.contains(name)
                    || (!l.contains("killarea") && (l.contains("spawn") || l.contains("resp")))
            };
            if include_spawn {
                if let Some(pos) = tm_pos_xz(area) {
                    for mode in modes_for_name(name) {
                        spawn_zones.push(json!({
                            "name": name,
                            "mode": mode,
                            "team": team_from_name(name),
                            "pos": [pos[0], pos[1]],
                        }));
                    }
                }
            }

            let include_capture = if mission_kind == "bttl" {
                is_bttl_capture_area(&l)
            } else {
                l.contains("capture_area") || l.contains("flag_area") || l.contains("capture_zone")
            };
            if include_capture {
                if let (Some(pos), Some(radius)) = (tm_pos_xz(area), tm_cylinder_radius(area)) {
                    let label = numeric_label_from_name(&l);
                    for mode in modes_for_name(name) {
                        capture_zones.push(json!({
                            "name": name,
                            "mode": mode,
                            "pos": [pos[0], pos[1]],
                            "radius": radius,
                            "label": label,
                        }));
                    }
                }
            }

            if l.contains("battle_area") || l.contains("battlearea") {
                if let (Some(pos), Some(half)) = (tm_pos_xz(area), tm_box_half_size_xz(area)) {
                    for mode in modes_for_name(name) {
                        battle_areas.push(json!({
                            "name": name,
                            "mode": mode,
                            "pos": [pos[0], pos[1]],
                            "halfSize": [half[0], half[1]],
                        }));
                    }
                }
            }
        }
    }

    if mission_kind == "bttl" {
        // For battle missions, briefing areas are the reliable ground-truth spawn markers.
        spawns = spawn_zones.clone();
    }

    Some(json!({
        "label": label,
        "spawns": spawns,
        "spawnZones": spawn_zones,
        "captureZones": capture_zones,
        "battleAreas": battle_areas,
    }))
}

pub fn extract_missions_for_map(datamine_root: &Path, map_name: &str) -> Result<Option<Value>> {
    let suffix = map_name
        .split_once('_')
        .map(|(_, s)| s)
        .unwrap_or(map_name);
    let map_lower = map_name.to_ascii_lowercase();
    let suffix_lower = suffix.to_ascii_lowercase();
    let is_air_map = map_lower.starts_with("air_");

    let mission_dir: PathBuf = datamine_root
        .join("mis.vromfs.bin_u")
        .join("gamedata")
        .join("missions")
        .join("cta")
        .join("tanks")
        .join(suffix);

    let mission_root = datamine_root
        .join("mis.vromfs.bin_u")
        .join("gamedata")
        .join("missions");

    let mut candidate_files: Vec<PathBuf> = Vec::new();
    if !is_air_map && mission_dir.exists() && mission_dir.is_dir() {
        for ent in fs::read_dir(&mission_dir)? {
            let p = ent?.path();
            if p.is_file() {
                candidate_files.push(p);
            }
        }
    }

    if is_air_map {
        let air_roots = [
            mission_root.join("bridges"),
            mission_root.join("cta").join("planes"),
            mission_root.join("cta").join("helicopters"),
        ];
        for root in air_roots {
            collect_air_mission_files(&root, &map_lower, &suffix_lower, &mut candidate_files)?;
        }
    }

    let mut missions: Vec<Value> = Vec::new();
    let tank_stem_prefix = format!("{suffix_lower}_");
    for p in candidate_files {
        if p.extension().and_then(|e| e.to_str()) != Some("blkx") {
            continue;
        }
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if stem.starts_with("template_") {
            continue;
        }
        if is_air_map {
            if !(stem.contains(&map_lower) || stem.contains(&suffix_lower)) {
                continue;
            }
        } else if !stem.starts_with(&tank_stem_prefix) {
            continue;
        }

        if let Some(m) = parse_one_mission_file(&p, suffix) {
            missions.push(m);
        }
    }

    if missions.is_empty() {
        return Ok(None);
    }

    missions.sort_by(|a, b| {
        let la = a.get("label").and_then(Value::as_str).unwrap_or_default();
        let lb = b.get("label").and_then(Value::as_str).unwrap_or_default();
        la.cmp(lb)
    });

    Ok(Some(json!({ "missions": missions })))
}

fn collect_air_mission_files(root: &Path, map_lower: &str, suffix_lower: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() || !root.is_dir() {
        return Ok(());
    }
    for ent in fs::read_dir(root)? {
        let p = ent?.path();
        if p.is_dir() {
            collect_air_mission_files(&p, map_lower, suffix_lower, out)?;
            continue;
        }
        if p.extension().and_then(|e| e.to_str()) != Some("blkx") {
            continue;
        }
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if stem.contains(map_lower) || stem.contains(suffix_lower) {
            out.push(p);
        }
    }
    Ok(())
}
