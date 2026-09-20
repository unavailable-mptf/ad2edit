//! The pipeline pieces that are pure library: an entity explained for an
//! agent, two build files composed into one, a project's build history,
//! an armour plan applied to a target mass, and a round against the plan.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value as J};

use crate::build;
use crate::doctor;
use crate::dupe::Dupe;
use crate::refs;
use crate::transform;
use crate::value::{Node, Value};

// ---------------------------------------------------------------------------
// 9. --explain
// ---------------------------------------------------------------------------

fn str_of(dupe: &Dupe, et: usize, k: &str) -> Option<String> {
    match dupe.get(et, k) {
        Some(Value::Str(b)) => Some(String::from_utf8_lossy(b).into_owned()),
        _ => None,
    }
}

/// Everything about one entity in one JSON: identity, pose, parent chain
/// and children, links out by modifier, links in from whoever names it,
/// wires, the doctor's findings for it, and every tuning field it exposes.
pub fn explain(dupe: &Dupe, index: f64) -> Result<J, String> {
    let et = transform::entity_table(dupe, index).ok_or_else(|| format!("no entity {index}"))?;
    let class = str_of(dupe, et, "Class").unwrap_or_default();
    let model = str_of(dupe, et, "Model").unwrap_or_default();
    let (pos, ang) = transform::entity_transform(dupe, index).unwrap_or(((0.0, 0.0, 0.0), (0.0, 0.0, 0.0)));

    // Parent chain up to the root.
    let mut chain = Vec::new();
    let mut cur = index;
    for _ in 0..32 {
        let Some(e) = transform::entity_table(dupe, cur) else { break };
        let Some(bdi) = dupe.get_table(e, "BuildDupeInfo") else { break };
        match dupe.get_number(bdi, "DupeParentID") {
            Some(p) if transform::entity_table(dupe, p).is_some() => {
                chain.push(json!({ "index": p, "class": transform::entity_table(dupe, p).and_then(|pe| str_of(dupe, pe, "Class")) }));
                cur = p;
            }
            _ => break,
        }
    }
    let children: Vec<f64> = dupe
        .list_entities()
        .iter()
        .filter(|(i, _, _)| {
            transform::entity_table(dupe, *i)
                .and_then(|e| dupe.get_table(e, "BuildDupeInfo"))
                .and_then(|b| dupe.get_number(b, "DupeParentID"))
                == Some(index)
        })
        .map(|(i, _, _)| *i)
        .collect();

    // Links out: the registry's sites on this entity, grouped by modifier —
    // exact, where "any array of numbers" would count a mass modifier.
    let mut out = serde_json::Map::new();
    let prefix = format!("Entities.{}.EntityMods.", crate::value::short(&Value::Number(index)).trim_matches('"'));
    for h in refs::scan(dupe) {
        if !h.known || !h.path.starts_with(&prefix) || h.path.contains("WireDupeInfo") || h.path.contains("._links") {
            continue;
        }
        let rest = &h.path[prefix.len()..];
        let modifier = rest.split(|c| c == '[' || c == '.').next().unwrap_or(rest).to_owned();
        let entry = out.entry(modifier).or_insert_with(|| json!([]));
        if let Some(arr) = entry.as_array_mut() {
            arr.push(json!(h.value));
        }
    }
    // Links in: every scan hit whose value is this index.
    let mut incoming = Vec::new();
    for h in refs::scan(dupe) {
        if h.value == index && !h.path.contains("._links") && !h.path.contains("DupeParentID") {
            incoming.push(json!({ "path": h.path }));
        }
    }
    // Wires on this entity.
    let mut wires = Vec::new();
    if let Some(mods) = dupe.get_table(et, "EntityMods") {
        if let Some(wdi) = dupe.get_table(mods, "WireDupeInfo") {
            if let Some(ws) = dupe.get_table(wdi, "Wires") {
                if let Node::Table(e) = &dupe.arena[ws] {
                    for (k, v) in e {
                        if let Some(w) = crate::value::table_index(v) {
                            wires.push(json!({ "input": crate::value::short(k).trim_matches('"'), "src": dupe.get_number(w, "Src"), "src_port": str_of(dupe, w, "SrcId") }));
                        }
                    }
                }
            }
        }
    }
    // Tuning: top-level scalars ACF reads, and DT keys.
    let mut tuning = serde_json::Map::new();
    for k in ["Engine", "Gearbox", "GearAmount", "FinalDrive", "Weapon", "Caliber", "AmmoType", "FuelTank", "FuelType", "Turret", "RingSize", "MinDeg", "MaxDeg", "MaxSpeed", "Motor", "Teeth", "CompSize", "CrewTypeID", "VehicleName"] {
        match dupe.get(et, k) {
            Some(Value::Number(n)) => { tuning.insert(k.into(), json!(n)); }
            Some(Value::Str(b)) => { tuning.insert(k.into(), json!(String::from_utf8_lossy(b))); }
            _ => {}
        }
    }
    if let Some(dt) = dupe.get_table(et, "DT") {
        if let Node::Table(e) = &dupe.arena[dt] {
            let mut d = serde_json::Map::new();
            for (k, v) in e {
                let mut seen = vec![false; dupe.arena.len()];
                d.insert(crate::value::short(k).trim_matches('"').to_owned(), crate::json::value_to_json(dupe, v, &mut seen));
            }
            tuning.insert("DT".into(), J::Object(d));
        }
    }
    let findings: Vec<J> = doctor::doctor(dupe)
        .into_iter()
        .filter(|f| f.entity == Some(index))
        .map(|f| json!({ "severity": format!("{:?}", f.severity).to_lowercase(), "what": f.what, "fixable": f.fix.is_some() }))
        .collect();

    Ok(json!({
        "index": index,
        "class": class,
        "model": model,
        "pos": [pos.0, pos.1, pos.2],
        "ang": [ang.0, ang.1, ang.2],
        "parent_chain": chain,
        "children": children,
        "links_out": out,
        "links_in": incoming,
        "wires": wires,
        "tuning": tuning,
        "findings": findings,
    }))
}

// ---------------------------------------------------------------------------
// 8. --compose
// ---------------------------------------------------------------------------

/// Two build files into one: every name in `b` gets `prefix_`, `a`'s
/// header wins, sections concatenate. Cross-links are added afterwards by
/// hand — which is the point of composing.
pub fn compose(a_text: &str, b_text: &str, prefix: &str) -> Result<String, String> {
    let a: toml::Value = toml::from_str(a_text).map_err(|e| format!("a: {e}"))?;
    let mut b: toml::Value = toml::from_str(b_text).map_err(|e| format!("b: {e}"))?;
    let name_keys = ["name", "a", "b", "wheel", "base", "controller", "src", "dst", "crate", "entity", "parent", "track", "chassis"];
    fn rename(v: &mut toml::Value, keys: &[&str], prefix: &str) {
        match v {
            toml::Value::Table(t) => {
                for (k, val) in t.iter_mut() {
                    if keys.contains(&k.as_str()) {
                        if let toml::Value::String(s) = val {
                            *s = format!("{prefix}_{s}");
                        }
                    }
                    if k == "wheels" {
                        if let toml::Value::Array(arr) = val {
                            for w in arr.iter_mut() {
                                if let toml::Value::String(s) = w { *s = format!("{prefix}_{s}"); }
                            }
                        }
                    }
                    rename(val, keys, prefix);
                }
            }
            toml::Value::Array(arr) => {
                for x in arr.iter_mut() { rename(x, keys, prefix); }
            }
            _ => {}
        }
    }
    rename(&mut b, &name_keys, prefix);
    let mut out = a;
    let (Some(ot), Some(bt)) = (out.as_table_mut(), b.as_table()) else { return Err("not tables".into()) };
    for (k, v) in bt {
        match (ot.get_mut(k), v) {
            (Some(toml::Value::Array(dst)), toml::Value::Array(src)) => dst.extend(src.iter().cloned()),
            (None, toml::Value::Array(src)) => { ot.insert(k.clone(), toml::Value::Array(src.clone())); }
            _ => {} // scalar header keys: a's win
        }
    }
    toml::to_string(&out).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 3. projects
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Project {
    /// The build file this project is about.
    pub build: String,
    /// Parts bin folder (for --gen-tank and [[spawn]] part = …).
    #[serde(default)]
    pub parts: Option<String>,
    /// Reference dupes the parts came from, for the record.
    #[serde(default)]
    pub references: Vec<String>,
    /// garrysmod folder, for --models.
    #[serde(default)]
    pub gmod: Option<String>,
    /// Map to preview on.
    #[serde(default)]
    pub map: Option<String>,
    /// AD2 data folder to copy builds into.
    #[serde(default)]
    pub into: Option<String>,
    /// Keep every build with its doctor report under builds/.
    #[serde(default = "yes")]
    pub history: bool,
}
fn yes() -> bool { true }

pub const PROJECT_FILE: &str = "project.toml";

pub fn default_project(build: &str) -> String {
    let p = Project { build: build.into(), parts: Some("parts".into()), history: true, ..Default::default() };
    toml::to_string_pretty(&p).unwrap_or_default()
}

/// The project a build file belongs to: `project.toml` in its folder.
pub fn project_for(build_file: &Path) -> Option<(PathBuf, Project)> {
    let dir = build_file.parent()?.to_path_buf();
    let text = std::fs::read_to_string(dir.join(PROJECT_FILE)).ok()?;
    toml::from_str::<Project>(&text).ok().map(|p| (dir, p))
}

/// Records a build: copies the output into `builds/<stamp>.txt` beside a
/// `<stamp>.doctor.json`, and appends a line to `builds/history.log`.
pub fn record_build(dir: &Path, output: &Path, findings: &[doctor::Finding]) -> Result<PathBuf, String> {
    let builds = dir.join("builds");
    std::fs::create_dir_all(&builds).map_err(|e| e.to_string())?;
    let stamp = {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        format!("{now}")
    };
    let stem = output.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "build".into());
    let dest = builds.join(format!("{stem}.{stamp}.txt"));
    std::fs::copy(output, &dest).map_err(|e| e.to_string())?;
    let report: Vec<J> = findings.iter().map(|f| json!({ "severity": format!("{:?}", f.severity).to_lowercase(), "entity": f.entity, "what": f.what })).collect();
    std::fs::write(builds.join(format!("{stem}.{stamp}.doctor.json")), serde_json::to_string_pretty(&report).unwrap_or_default()).map_err(|e| e.to_string())?;
    let errors = findings.iter().filter(|f| f.severity == doctor::Severity::Error).count();
    let warns = findings.iter().filter(|f| f.severity == doctor::Severity::Warn).count();
    let line = format!("{stamp} {stem} errors={errors} warnings={warns}\n");
    use std::io::Write;
    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(builds.join("history.log")).map_err(|e| e.to_string())?;
    log.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    Ok(dest)
}

// ---------------------------------------------------------------------------
// 5. armour plan, applied
// ---------------------------------------------------------------------------

/// One armoured plate as the planner sees it.
pub struct Plate {
    pub index: f64,
    pub thickness: f64,
    pub ductility: f64,
    pub half: (f64, f64, f64),
    /// Position in the baseplate's frame; +x is forward.
    pub local: (f64, f64, f64),
}

pub fn plates_of(dupe: &Dupe, half_of: &mut dyn FnMut(&str) -> Option<(f64, f64, f64)>) -> Vec<Plate> {
    let base = dupe.list_entities().into_iter().find(|(_, c, _)| c.contains("acf_baseplate")).map(|(i, _, _)| i);
    let frame = base.and_then(|b| transform::entity_transform(dupe, b));
    let mut out = Vec::new();
    for (index, class, model) in dupe.list_entities() {
        if !class.contains("prop_physics") { continue; }
        let Some(et) = transform::entity_table(dupe, index) else { continue };
        let Some(mods) = dupe.get_table(et, "EntityMods") else { continue };
        let Some(a) = dupe.get_table(mods, "ACF_Armor") else { continue };
        let Some(half) = half_of(model.trim_matches('"')) else { continue };
        let (p, _) = transform::entity_transform(dupe, index).unwrap_or(((0.0, 0.0, 0.0), (0.0, 0.0, 0.0)));
        let local = match frame { Some((bp, ba)) => build::world_to_local(p, bp, ba), None => p };
        out.push(Plate { index, thickness: dupe.get_number(a, "Thickness").unwrap_or(0.0), ductility: dupe.get_number(a, "Ductility").unwrap_or(0.0), half, local });
    }
    out
}

/// Thicknesses that land the armour on `target_kg`, front-weighted:
/// plates ahead of the hull centre get `front_bias`× the scale of the rest.
/// Returns (index, new thickness).
pub fn plan_thicknesses(plates: &[Plate], target_kg: f64, front_bias: f64) -> Vec<(f64, f64)> {
    if plates.is_empty() { return Vec::new(); }
    let mass = |t: f64, p: &Plate| build::plate_mass(p.half, t, p.ductility);
    let front = |p: &Plate| p.local.0 > 0.0;
    // Mass is linear in thickness, so solve one scale s with front plates at s·bias.
    let base: f64 = plates.iter().map(|p| mass(p.thickness, p) * if front(p) { front_bias } else { 1.0 }).sum();
    if base <= 0.0 { return Vec::new(); }
    let s = target_kg / base;
    plates.iter().map(|p| (p.index, (p.thickness * s * if front(p) { front_bias } else { 1.0 }).clamp(crate::rules::MIN_ARMOUR_MM, crate::rules::MAX_ARMOUR_MM))).collect()
}

pub fn apply_thicknesses(dupe: &mut Dupe, plan: &[(f64, f64)]) -> usize {
    let mut n = 0;
    for (index, t) in plan {
        let Some(et) = transform::entity_table(dupe, *index) else { continue };
        let Some(mods) = dupe.get_table(et, "EntityMods") else { continue };
        let Some(a) = dupe.get_table(mods, "ACF_Armor") else { continue };
        dupe.set(a, "Thickness", Value::Number(*t));
        n += 1;
    }
    n
}

// ---------------------------------------------------------------------------
// 6. versus: a round's penetration against each plate
// ---------------------------------------------------------------------------

/// ACF damage_result.lua: effective = thickness / |cos(angle)|; a round
/// with `pen` mm goes through when pen ≥ effective, with `pen − effective`
/// overkill. Angles are the impact obliquities to report at.
pub fn versus(plates: &[Plate], pen_mm: f64, angles_deg: &[f64]) -> Vec<J> {
    plates
        .iter()
        .map(|p| {
            let rows: Vec<J> = angles_deg
                .iter()
                .map(|a| {
                    let eff = p.thickness / a.to_radians().cos().abs().max(1e-6);
                    json!({ "angle": a, "effective_mm": eff, "penetrates": pen_mm >= eff, "overkill_mm": (pen_mm - eff).max(0.0) })
                })
                .collect();
            json!({ "plate": p.index, "thickness_mm": p.thickness, "front": p.local.0 > 0.0, "at": rows })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 7. fit a spec to a real baseplate
// ---------------------------------------------------------------------------

/// Reads Width/Length from a baseplate part (its ACF_UserData) and sizes the
/// spec to it: wheels just outside the plate's edges, spaced along its
/// length, engines behind the rear axle, turret centred.
pub fn fit_spec_to_plate(spec: &mut crate::gen::TankSpec, plate: &Dupe) -> Result<(f64, f64), String> {
    let (index, _, _) = plate.list_entities().into_iter().find(|(_, c, _)| c.contains("acf_baseplate")).ok_or("part is not a baseplate")?;
    let et = transform::entity_table(plate, index).unwrap();
    let ud = plate.get_table(et, "ACF_UserData").ok_or("baseplate has no ACF_UserData")?;
    let width = plate.get_number(ud, "Width").ok_or("no Width")?;
    let length = plate.get_number(ud, "Length").ok_or("no Length")?;
    // wheel60/tank30 are ~60 across; centre them 6 outside the edge.
    spec.track_width = width + 12.0 + 30.0;
    let n = spec.wheels_per_side.max(1) as f64;
    spec.wheel_spacing = if n > 1.0 { ((length - 40.0) / (n - 1.0)).max(34.0) } else { 0.0 };
    spec.turret_forward = length * 0.12;
    Ok((width, length))
}

/// One number on an entity that a builder tunes, where it lives as a dotted
/// path for `build::put_path`, and what to call it.
#[derive(Debug, Clone, PartialEq)]
pub struct Tunable {
    pub path: String,
    pub label: String,
    pub value: f64,
}

/// The numbers worth tuning on one entity: ACF's paste-time fields, its
/// gear ratios, everything numeric under `DT`, and its armour. Only fields
/// the entity already carries are offered, so nothing is invented.
pub fn tunables(dupe: &Dupe, index: f64) -> Vec<Tunable> {
    const NAMED: &[(&str, &str)] = &[
        ("Caliber", "calibre (mm)"),
        ("RingSize", "ring size"),
        ("MinDeg", "lowest angle (deg)"),
        ("MaxDeg", "highest angle (deg)"),
        ("MaxSpeed", "top slew speed (deg/s)"),
        ("Teeth", "motor teeth"),
        ("CompSize", "motor size"),
        ("FinalDrive", "final drive"),
        ("GearAmount", "number of gears"),
        ("GearboxScale", "gearbox scale"),
    ];
    let Some(et) = transform::entity_table(dupe, index) else { return Vec::new() };
    let mut found = Vec::new();
    for (key, label) in NAMED {
        if let Some(value) = dupe.get_number(et, key) {
            found.push(Tunable { path: (*key).to_owned(), label: (*label).to_owned(), value });
        }
    }
    let numbers_under = |table: usize| -> Vec<(String, f64)> {
        match &dupe.arena[table] {
            Node::Table(entries) => entries
                .iter()
                .filter_map(|(key, value)| match (key, value) {
                    (Value::Str(key), Value::Number(n)) => Some((String::from_utf8_lossy(key).into_owned(), *n)),
                    _ => None,
                })
                .collect(),
            Node::Array(_) => Vec::new(),
        }
    };
    for (key, value) in numbers_under(et) {
        let gear = key.strip_prefix("Gear").is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()));
        if gear {
            found.push(Tunable { label: format!("gear {} ratio", &key[4..]), path: key, value });
        }
    }
    // A gearbox built from the catalogue keeps its scale in its user data.
    if let Some(value) = dupe.get_table(et, "ACF_UserData").and_then(|ud| dupe.get_number(ud, "GearboxScale")) {
        if !found.iter().any(|tunable| tunable.path == "GearboxScale") {
            found.push(Tunable { path: "ACF_UserData.GearboxScale".to_owned(), label: "gearbox scale".to_owned(), value });
        }
    }
    if let Some(dt) = dupe.get_table(et, "DT") {
        for (key, value) in numbers_under(dt) {
            found.push(Tunable { path: format!("DT.{key}"), label: key, value });
        }
    }
    if let Some(armour) = dupe.get_table(et, "EntityMods").and_then(|mods| dupe.get_table(mods, "ACF_Armor")) {
        for (key, label) in [("Thickness", "armour (mm)"), ("Ductility", "ductility")] {
            if let Some(value) = dupe.get_number(armour, key) {
                found.push(Tunable { path: format!("EntityMods.ACF_Armor.{key}"), label: label.to_owned(), value });
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::fixture;

    #[test]
    fn explain_names_links_both_ways_and_the_parent_chain() {
        let d = fixture();
        let j = explain(&d, 11.0).unwrap();
        assert_eq!(j["class"], "acf_gun");
        assert_eq!(j["links_out"]["ACFCrates"], json!([12.0]));
        assert!(j["links_out"].get("ACFTurret").is_some());
        let j2 = explain(&d, 18.0).unwrap();
        assert_eq!(j2["parent_chain"][0]["index"], 10.0);
        let j3 = explain(&d, 12.0).unwrap();
        assert!(j3["links_in"].as_array().unwrap().iter().any(|h| h["path"].as_str().unwrap().contains("ACFCrates")));
        assert!(explain(&d, 999.0).is_err());
    }

    #[test]
    fn compose_prefixes_every_name_in_b_and_keeps_as_header() {
        let a = "output = \"a.txt\"\nparts = \"p\"\n[[spawn]]\nname = \"hull\"\nindex = 1\nat = [0,0,0]\n[[link]]\na = \"hull\"\nb = \"hull\"\n";
        let b = "output = \"b.txt\"\nparts = \"q\"\n[[spawn]]\nname = \"hull\"\nindex = 2\nat = [0,0,0]\nparent = \"hull\"\n[[wheel]]\nname = \"w\"\nindex = 3\nat = [0,0,0]\nbase = \"hull\"\n[[track]]\ntrack = \"t\"\nchassis = \"hull\"\nwheels = [\"w\"]\n";
        let out = compose(a, b, "turret").unwrap();
        let f: crate::buildfile::BuildFile = toml::from_str(&out).unwrap();
        assert_eq!(f.output, "a.txt");
        assert_eq!(f.parts.as_deref(), Some("p"));
        assert_eq!(f.spawn.len(), 2);
        assert_eq!(f.spawn[1].name, "turret_hull");
        assert_eq!(f.spawn[1].parent.as_deref(), Some("turret_hull"));
        assert_eq!(f.wheel[0].base, "turret_hull");
        assert_eq!(f.track[0].wheels[0], "turret_w");
    }

    #[test]
    fn a_plan_lands_on_the_target_and_versus_uses_the_cosine() {
        let plates = vec![
            Plate { index: 1.0, thickness: 100.0, ductility: 0.0, half: (40.0, 40.0, 0.5), local: (50.0, 0.0, 0.0) },
            Plate { index: 2.0, thickness: 100.0, ductility: 0.0, half: (40.0, 40.0, 0.5), local: (-50.0, 0.0, 0.0) },
        ];
        let now: f64 = plates.iter().map(|p| build::plate_mass(p.half, p.thickness, p.ductility)).sum();
        let plan = plan_thicknesses(&plates, now * 2.0, 1.5);
        let after: f64 = plan.iter().map(|(i, t)| { let p = plates.iter().find(|p| p.index == *i).unwrap(); build::plate_mass(p.half, *t, p.ductility) }).sum();
        assert!((after - now * 2.0).abs() < 1.0, "{after} vs {}", now * 2.0);
        assert!(plan[0].1 > plan[1].1, "front plate thicker");
        let v = versus(&plates, 150.0, &[0.0, 60.0]);
        assert_eq!(v[0]["at"][0]["penetrates"], true);
        assert_eq!(v[0]["at"][1]["penetrates"], false, "100 at 60° is 200 effective");
    }

    #[test]
    fn a_project_records_a_build() {
        let dir = std::env::temp_dir().join(format!("ad2proj-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(PROJECT_FILE), default_project("tank.toml")).unwrap();
        std::fs::write(dir.join("tank.txt"), b"x").unwrap();
        let (pdir, p) = project_for(&dir.join("tank.toml")).unwrap();
        assert_eq!(p.build, "tank.toml");
        assert!(p.history);
        let dest = record_build(&pdir, &dir.join("tank.txt"), &[]).unwrap();
        assert!(dest.exists());
        assert!(std::fs::read_to_string(dir.join("builds/history.log")).unwrap().contains("errors=0"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fit_to_plate_sizes_the_spec_from_the_user_data() {
        let d = fixture();
        let mut spec = crate::gen::TankSpec::default();
        // Fixture plate has no ACF_UserData → error; add one.
        assert!(fit_spec_to_plate(&mut spec, &d).is_err());
        let mut d2 = fixture();
        let et = transform::entity_table(&d2, 10.0).unwrap();
        let ud = d2.new_table();
        d2.set(ud, "Width", Value::Number(96.0));
        d2.set(ud, "Length", Value::Number(200.0));
        d2.set(et, "ACF_UserData", Value::Table(ud));
        let (w, l) = fit_spec_to_plate(&mut spec, &d2).unwrap();
        assert_eq!((w, l), (96.0, 200.0));
        assert!(spec.track_width > 96.0);
        assert!(spec.wheel_spacing > 34.0);
    }

    #[test]
    fn tunables_offer_only_what_the_entity_carries_and_write_back_through_put_path() {
        let mut d = crate::buildfile::empty_dupe();
        let root = d.root_table().unwrap();
        let ents = d.get_table(root, "Entities").unwrap();
        let et = d.new_table();
        d.set(et, "Class", Value::Str(b"acf_turret".to_vec()));
        d.set(et, "MaxSpeed", Value::Number(30.0));
        d.set(et, "Gear2", Value::Number(0.5));
        d.set(et, "Turret", Value::Str(b"Turret-H".to_vec()));
        let dt = d.new_table();
        d.set(dt, "BrakeEngagement", Value::Number(0.0));
        d.set(et, "DT", Value::Table(dt));
        if let Node::Table(e) = &mut d.arena[ents] {
            e.push((Value::Number(7.0), Value::Table(et)));
        }
        let found = tunables(&d, 7.0);
        let paths: Vec<&str> = found.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(paths, ["MaxSpeed", "Gear2", "DT.BrakeEngagement"]);
        assert_eq!(found[1].label, "gear 2 ratio");

        build::put_path(&mut d, 7.0, &found[2].path, Value::Number(1.0)).unwrap();
        assert_eq!(tunables(&d, 7.0)[2].value, 1.0);
        assert!(tunables(&d, 99.0).is_empty());
    }
}
