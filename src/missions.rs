use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};

fn mode_from_name(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.contains("hardcore") {
        "hardcore"
    } else {
        "arcade"
    }
}

fn team_from_name(name: &str) -> i32 {
    let n = name.to_ascii_lowercase();
    if n.contains("t1") || n.contains("team1") || n.contains("_a") {
        1
    } else if n.contains("t2") || n.contains("team2") || n.contains("_b") {
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

    let mut spawn_name_set: HashSet<String> = HashSet::new();
    let mut spawns: Vec<Value> = Vec::new();
    if let Some(area_squad) = root
        .get("units")
        .and_then(|u| u.get("area_squad"))
        .and_then(Value::as_array)
    {
        for sq in area_squad {
            let name = sq.get("name").and_then(Value::as_str).unwrap_or("");
            let pos = match tm_pos_xz(sq) {
                Some(v) => v,
                None => continue,
            };
            let mode = mode_from_name(name);
            let team = team_from_name(name);
            spawns.push(json!({
                "name": name,
                "mode": mode,
                "team": team,
                "pos": [pos[0], pos[1]],
            }));

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
            let mode = mode_from_name(name);
            if spawn_name_set.contains(name) {
                if let Some(pos) = tm_pos_xz(area) {
                    spawn_zones.push(json!({
                        "name": name,
                        "mode": mode,
                        "team": team_from_name(name),
                        "pos": [pos[0], pos[1]],
                    }));
                }
            }

            if l.contains("capture_area") || l.contains("flag_area") {
                if let (Some(pos), Some(radius)) = (tm_pos_xz(area), tm_cylinder_radius(area)) {
                    let label = if let Some(idx) = l.rfind('_') {
                        l[idx + 1..]
                            .parse::<usize>()
                            .ok()
                            .map(|n| ((b'A' + ((n.saturating_sub(1)) % 26) as u8) as char).to_string())
                    } else {
                        None
                    };
                    capture_zones.push(json!({
                        "name": name,
                        "mode": mode,
                        "pos": [pos[0], pos[1]],
                        "radius": radius,
                        "label": label,
                    }));
                }
            }

            if l.contains("battle_area") || l.contains("battlearea") {
                if let (Some(pos), Some(half)) = (tm_pos_xz(area), tm_box_half_size_xz(area)) {
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

    let mission_dir: PathBuf = datamine_root
        .join("mis.vromfs.bin_u")
        .join("gamedata")
        .join("missions")
        .join("cta")
        .join("tanks")
        .join(suffix);

    if !mission_dir.exists() || !mission_dir.is_dir() {
        return Ok(None);
    }

    let mut missions: Vec<Value> = Vec::new();
    for ent in fs::read_dir(&mission_dir)? {
        let p = ent?.path();
        if !p.is_file() {
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
        if stem.starts_with("template_") {
            continue;
        }
        if !stem.starts_with(&format!("{}_", suffix.to_ascii_lowercase())) {
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
