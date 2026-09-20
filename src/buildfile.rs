//! A build file: the whole tank as data. Names instead of indices, so a
//! person or an agent writes it once and `ad2read --build tank.toml` does
//! what the shell recipe did by hand.
//!
//! ```toml
//! output = "tank.txt"
//! template = "mptf_ironlock.txt"      # default source for `from`
//! parts = "parts/"                    # optional folder for `part = "name"`
//!
//! [[spawn]]
//! name = "hull"
//! index = 337                         # entity in `template` (or `from`)
//! at = [0, 0, 0]
//! ang = [0, 90, 0]
//!
//! [[spawn]]
//! name = "eng1"
//! part = "engine"                     # parts/engine.txt, first entity
//! at = [-17.4, -72, -0.7]
//! ang = [0, -90, 0]
//! parent = "hull"
//!
//! [[spawn]]
//! name = "gun"
//! item = "C"                          # a catalogue item instead (--catalog)
//! set = { Caliber = 125 }             # the numbers the item takes
//! at = [0, 64, 53]
//! ang = [0, 90, 0]
//! parent = "trun"
//!
//! [[wheel]]                           # spawn + axis + spherical in one
//! name = "fr"
//! index = 287
//! at = [51, 57, -11]
//! ang = [0, 90, 0]
//! base = "hull"
//! axle = [0, 1, 0]
//!
//! [[link]]    a = "eng1"  b = "cvt"
//! [[lock_pair]] a = "fr"  b = "rr"
//! [[sprung]]  wheel = "w1"  base = "hull"
//! [[hydraulic]] wheel = "w1"  base = "hull"  controller = "hyd1"
//! [[wire]]    src = "ctrl"  out = "Active"  dst = "eng1"  in = "Load"
//! [[ammo]]    crate = "c1"  type = "APFSDS"  size = [30, 24, 22]
//! [[put]]     entity = "ctrl"  key = "DT.BrakeEngagement"  value = 1
//! ```
//!
//! Sections run in this order regardless of file order: spawn/wheel →
//! parent → wheel axes → lock_pair → sprung/hydraulic → link → wire → ammo
//! → spherical → put → track. Then AD2's own validation, then the write.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::build;
use crate::dupe::Dupe;
use crate::dupefile;
use crate::transform::Vec3;
use crate::value::Value;

#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct BuildFile {
    pub output: String,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub parts: Option<String>,
    #[serde(default)]
    pub spawn: Vec<Spawn>,
    #[serde(default)]
    pub wheel: Vec<Wheel>,
    #[serde(default)]
    pub link: Vec<Pair>,
    #[serde(default)]
    pub lock_pair: Vec<Pair>,
    #[serde(default)]
    pub sprung: Vec<WheelOnBase>,
    #[serde(default)]
    pub hydraulic: Vec<Hydraulic>,
    #[serde(default)]
    pub wire: Vec<Wire>,
    #[serde(default)]
    pub ammo: Vec<Ammo>,
    #[serde(default)]
    pub put: Vec<Put>,
    #[serde(default)]
    pub track: Vec<Track>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    pub name: String,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub index: Option<f64>,
    #[serde(default)]
    pub part: Option<String>,
    /// A catalogue item (`--catalog`) instead of a part on disk.
    #[serde(default)]
    pub item: Option<String>,
    /// Numbers the item takes: Caliber for a gun, Width, Length and
    /// Thickness for a baseplate.
    #[serde(default)]
    pub set: BTreeMap<String, f64>,
    pub at: [f64; 3],
    #[serde(default)]
    pub ang: Option<[f64; 3]>,
    #[serde(default)]
    pub parent: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Wheel {
    pub name: String,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub index: Option<f64>,
    #[serde(default)]
    pub part: Option<String>,
    /// A catalogue wheel, which brings its own mass and radius.
    #[serde(default)]
    pub item: Option<String>,
    #[serde(default)]
    pub set: BTreeMap<String, f64>,
    pub at: [f64; 3],
    #[serde(default)]
    pub ang: Option<[f64; 3]>,
    pub base: String,
    #[serde(default = "default_axle")]
    pub axle: [f64; 3],
    #[serde(default)]
    pub nocollide: bool,
    /// Make Spherical radius; omit to skip (or keep what the template had).
    #[serde(default)]
    pub radius: Option<f64>,
}
fn default_axle() -> [f64; 3] {
    [0.0, 1.0, 0.0]
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Pair {
    pub a: String,
    pub b: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct WheelOnBase {
    pub wheel: String,
    pub base: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Hydraulic {
    pub wheel: String,
    pub base: String,
    pub controller: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Wire {
    pub src: String,
    pub out: String,
    pub dst: String,
    #[serde(rename = "in")]
    pub input: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Ammo {
    #[serde(rename = "crate")]
    pub crate_: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub size: Option<[f64; 3]>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Put {
    pub entity: String,
    pub key: String,
    pub value: toml::Value,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub track: String,
    pub chassis: String,
    pub wheels: Vec<String>,
}

fn v3(a: [f64; 3]) -> Vec3 {
    (a[0], a[1], a[2])
}

/// An empty, valid dupe to build into. HeadEnt is repaired at save.
/// A dupe as the bytes of an AdvDupe2 file, for one that has never been a
/// file: revision 5, the LZMA settings AD2 itself writes, and the info
/// block it reads back.
pub fn file_bytes(dupe: &Dupe, name: &str) -> Result<Vec<u8>, String> {
    let body = dupe.to_body().map_err(|e| format!("encode: {e}"))?;
    let compressed = crate::dupefile::compress(&body, 3, 0, 2, 65536, true).map_err(|e| format!("compress: {e}"))?;
    let size = body.len().to_string();
    let info: Vec<(Vec<u8>, Vec<u8>)> = [("name", name), ("date", "built"), ("time", ""), ("timezone", ""), ("size", size.as_str()), ("check", "\r\n\t\n")]
        .iter()
        .map(|(key, value)| (key.as_bytes().to_vec(), value.as_bytes().to_vec()))
        .collect();
    Ok(crate::dupefile::build(5, &info, &compressed))
}

/// A finished tank from a preset's name, built from the catalogue alone the
/// way `--build` would and left in the state every write requires:
/// repaired, cleaned, fixed, valid, nothing dangling.
pub fn preset_dupe(name: &str) -> Result<Dupe, String> {
    let spec = crate::gen::preset(name).ok_or_else(|| format!("no preset called {name}"))?;
    let file: BuildFile = toml::from_str(&crate::gen::write_build_file(&spec.catalogue())).map_err(|e| e.to_string())?;
    let (mut dupe, _) = run(&file, std::path::Path::new("."))?;
    crate::extras::repair_head(&mut dupe);
    crate::doctor::clean(&mut dupe);
    let findings = crate::doctor::doctor(&dupe);
    crate::doctor::apply_fixes(&mut dupe, &findings);
    let problems = crate::extras::validate(&dupe);
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    if !crate::refs::dangling(&dupe).is_empty() {
        return Err("dangling references".into());
    }
    Ok(dupe)
}

pub fn empty_dupe() -> Dupe {
    let mut d = Dupe { root: Value::Nil, arena: Vec::new() };
    let root = d.new_table();
    d.root = Value::Table(root);
    let ents = d.new_table();
    d.set(root, "Entities", Value::Table(ents));
    let cons = d.new_array();
    d.set(root, "Constraints", Value::Table(cons));
    d
}

pub struct Report {
    pub names: HashMap<String, f64>,
    pub log: Vec<String>,
}

struct Templates {
    base_dir: PathBuf,
    cache: HashMap<PathBuf, Dupe>,
}

impl Templates {
    fn load(&mut self, rel: &str) -> Result<&Dupe, String> {
        let p = if Path::new(rel).is_absolute() { PathBuf::from(rel) } else { self.base_dir.join(rel) };
        if !self.cache.contains_key(&p) {
            let bytes = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            let f = dupefile::parse(&bytes)?;
            let raw = dupefile::decompress(&f.compressed)?;
            let (d, _) = Dupe::from_body(&raw)?;
            self.cache.insert(p.clone(), d);
        }
        Ok(&self.cache[&p])
    }
}

/// Executes a build file. `base_dir` is where relative paths resolve —
/// the file's own folder.
pub fn run(file: &BuildFile, base_dir: &Path) -> Result<(Dupe, Report), String> {
    let mut dupe = empty_dupe();
    let mut names: HashMap<String, f64> = HashMap::new();
    let mut log = Vec::new();
    let mut templates = Templates { base_dir: base_dir.to_path_buf(), cache: HashMap::new() };

    let resolve = |names: &HashMap<String, f64>, n: &str| -> Result<f64, String> {
        names.get(n).copied().ok_or_else(|| format!("no part named {n:?} (spawned so far: {})", {
            let mut v: Vec<&String> = names.keys().collect();
            v.sort();
            v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        }))
    };

    // Spawns: plain and wheel, in file order within each list; wheels after.
    let mut spawn_one = |dupe: &mut Dupe, names: &mut HashMap<String, f64>, log: &mut Vec<String>, name: &str, from: &Option<String>, index: Option<f64>, part: &Option<String>, item: &Option<String>, set: &BTreeMap<String, f64>, at: [f64; 3], ang: Option<[f64; 3]>| -> Result<f64, String> {
        if names.contains_key(name) {
            return Err(format!("duplicate part name {name:?}"));
        }
        if let Some(id) = item {
            let it = crate::catalog::find(id)
                .ok_or_else(|| format!("{name}: no catalogue item called {id:?}; see --catalog"))?;
            let numbers: Vec<(&str, f64)> = set.iter().map(|(k, v)| (k.as_str(), *v)).collect();
            let new = build::spawn_catalog(dupe, it, v3(at), ang.map(v3).unwrap_or((0.0, 0.0, 0.0)), &numbers)?;
            names.insert(name.to_owned(), new);
            log.push(format!("spawned {name} = {new} from the catalogue ({})", it.name));
            return Ok(new);
        }
        let (path, idx) = match (part, index) {
            (Some(p), _) => {
                let dir = file.parts.as_deref().ok_or("`part = ...` needs `parts = \"dir\"` at the top")?;
                (format!("{}/{}.txt", dir.trim_end_matches('/'), p), None)
            }
            (None, Some(i)) => {
                let t = from.clone().or_else(|| file.template.clone()).ok_or_else(|| format!("{name}: `index` needs `from` or a top-level `template`"))?;
                (t, Some(i))
            }
            (None, None) => return Err(format!("{name}: give `index` (with template/from), `part`, or `item` from the catalogue")),
        };
        let template = templates.load(&path)?;
        let idx = match idx {
            Some(i) => i,
            None => template.list_entities().first().map(|(i, _, _)| *i).ok_or_else(|| format!("{path} has no entities"))?,
        };
        let new = build::spawn_from(dupe, template, idx, v3(at), ang.map(v3))?;
        names.insert(name.to_owned(), new);
        log.push(format!("spawned {name} = {new} from {path}#{idx}"));
        Ok(new)
    };

    for s in &file.spawn {
        spawn_one(&mut dupe, &mut names, &mut log, &s.name, &s.from, s.index, &s.part, &s.item, &s.set, s.at, s.ang)?;
    }
    for w in &file.wheel {
        spawn_one(&mut dupe, &mut names, &mut log, &w.name, &w.from, w.index, &w.part, &w.item, &w.set, w.at, w.ang)?;
    }

    // Parents
    for s in &file.spawn {
        if let Some(p) = &s.parent {
            build::set_parent(&mut dupe, resolve(&names, &s.name)?, resolve(&names, p)?)?;
        }
    }
    // Wheels: axis (+ spherical)
    for w in &file.wheel {
        let wi = resolve(&names, &w.name)?;
        let bi = resolve(&names, &w.base)?;
        build::add_wheel_axis(&mut dupe, wi, bi, v3(w.axle), w.nocollide)?;
        if let Some(r) = w.radius {
            build::make_spherical(&mut dupe, wi, r)?;
        }
    }
    for p in &file.lock_pair {
        build::add_pair_lock(&mut dupe, resolve(&names, &p.a)?, resolve(&names, &p.b)?)?;
    }
    for s in &file.sprung {
        build::add_sprung_wheel(&mut dupe, resolve(&names, &s.wheel)?, resolve(&names, &s.base)?)?;
    }
    for h in &file.hydraulic {
        build::add_hydraulic_wheel(&mut dupe, resolve(&names, &h.wheel)?, resolve(&names, &h.base)?, resolve(&names, &h.controller)?)?;
    }
    for l in &file.link {
        let r = build::add_link(&mut dupe, resolve(&names, &l.a)?, resolve(&names, &l.b)?)?;
        log.push(format!("link {} -> {} via {}", l.a, l.b, r.modifier));
    }
    for w in &file.wire {
        build::add_wire(&mut dupe, resolve(&names, &w.src)?, &w.out, resolve(&names, &w.dst)?, &w.input)?;
    }
    for a in &file.ammo {
        build::set_ammo(&mut dupe, resolve(&names, &a.crate_)?, &a.type_, a.size.map(v3))?;
    }
    for p in &file.put {
        let v = match &p.value {
            toml::Value::Integer(i) => Value::Number(*i as f64),
            toml::Value::Float(f) => Value::Number(*f),
            toml::Value::Boolean(b) => Value::Bool(*b),
            toml::Value::String(s) => Value::Str(s.as_bytes().to_vec()),
            toml::Value::Array(a) if a.len() == 3 => {
                let n = |v: &toml::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)).unwrap_or(0.0);
                Value::Vector(n(&a[0]), n(&a[1]), n(&a[2]))
            }
            other => return Err(format!("put {}.{}: unsupported value {other}", p.entity, p.key)),
        };
        build::put_path(&mut dupe, resolve(&names, &p.entity)?, &p.key, v)?;
    }
    for t in &file.track {
        let wheels: Result<Vec<f64>, String> = t.wheels.iter().map(|w| resolve(&names, w)).collect();
        build::set_track_links(&mut dupe, resolve(&names, &t.track)?, resolve(&names, &t.chassis)?, &wheels?)?;
    }

    Ok((dupe, Report { names, log }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_file_parses_and_rejects_unknown_keys() {
        let ok: BuildFile = toml::from_str(r#"
            output = "x.txt"
            template = "t.txt"
            [[spawn]]
            name = "hull"
            index = 1
            at = [0, 0, 0]
            [[wheel]]
            name = "fl"
            index = 2
            at = [1, 2, 3]
            base = "hull"
            [[put]]
            entity = "hull"
            key = "DT.BrakeEngagement"
            value = 1
        "#).unwrap();
        assert_eq!(ok.spawn.len(), 1);
        assert_eq!(ok.wheel[0].axle, [0.0, 1.0, 0.0]);
        let bad: Result<BuildFile, _> = toml::from_str("output = \"x\"\n[[spawn]]\nname = \"a\"\nat = [0,0,0]\nbogus = 1\n");
        assert!(bad.is_err());
    }

    #[test]
    fn a_spawn_from_the_catalogue_needs_no_parts_dir() {
        let file: BuildFile = toml::from_str(r#"
            output = "out.txt"
            [[spawn]]
            name = "hull"
            item = "GroundVehicle"
            at = [0, 0, 0]
            ang = [0, 90, 0]
            set = { Width = 96, Length = 200 }
            [[wheel]]
            name = "w"
            item = "Tankwheel30"
            at = [51, 0, -11]
            ang = [0, 90, 0]
            base = "hull"
        "#).unwrap();
        let (dupe, report) = run(&file, std::path::Path::new(".")).unwrap();
        assert_eq!(dupe.list_entities().len(), 2);
        let hull = report.names["hull"];
        let et = crate::transform::entity_table(&dupe, hull).unwrap();
        let ud = dupe.get_table(et, "ACF_UserData").unwrap();
        assert_eq!(dupe.get_number(ud, "Width"), Some(96.0));
        assert_eq!(dupe.get_number(ud, "Length"), Some(200.0));
        let wheel = report.names["w"];
        let wt = crate::transform::entity_table(&dupe, wheel).unwrap();
        let mods = dupe.get_table(wt, "EntityMods").unwrap();
        assert!(dupe.get_table(mods, "mass").is_some(), "a catalogue wheel gets its mass");
        assert!(dupe.get_table(mods, "MakeSphericalCollisions").is_some(), "and spherical collision");

        let bad: BuildFile = toml::from_str(r#"
            output = "x"
            [[spawn]]
            name = "a"
            item = "nope"
            at = [0, 0, 0]
        "#).unwrap();
        let err = run(&bad, std::path::Path::new(".")).err().expect("an unknown item must fail");
        assert!(err.contains("catalogue"), "{err}");
    }

    #[test]
    fn a_build_runs_against_a_template_on_disk() {
        // Write the fixture as a template file, build a two-part dupe from it.
        let dir = std::env::temp_dir().join(format!("ad2build-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = crate::fixture::fixture();
        let body = t.to_body().unwrap();
        let comp = dupefile::compress(&body, 3, 0, 2, 65536, true).unwrap();
        let size = body.len().to_string();
        let info: Vec<(Vec<u8>, Vec<u8>)> = [("name", "t"), ("size", size.as_str()), ("check", "\r\n\t\n")]
            .iter().map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec())).collect();
        std::fs::write(dir.join("t.txt"), dupefile::build(5, &info, &comp)).unwrap();

        let file: BuildFile = toml::from_str(r#"
            output = "out.txt"
            template = "t.txt"
            [[spawn]]
            name = "plate"
            index = 10
            at = [0, 0, 20]
            ang = [0, 90, 0]
            [[spawn]]
            name = "gun"
            index = 11
            at = [0, 40, 40]
            parent = "plate"
            [[spawn]]
            name = "crate"
            index = 12
            at = [0, -40, 30]
            parent = "plate"
            [[wheel]]
            name = "fl"
            index = 14
            at = [51, 57, 9]
            ang = [0, 90, 0]
            base = "plate"
            radius = 30
            [[link]]
            a = "gun"
            b = "crate"
            [[put]]
            entity = "plate"
            key = "DT.Test"
            value = 7
        "#).unwrap();
        let (d, r) = run(&file, &dir).unwrap();
        assert_eq!(d.list_entities().len(), 4);
        assert!(crate::refs::dangling(&d).is_empty());
        assert!(r.log.iter().any(|l| l.contains("via ACFCrates")));
        let plate = r.names["plate"];
        let et = crate::transform::entity_table(&d, plate).unwrap();
        let dt = d.get_table(et, "DT").unwrap();
        assert_eq!(d.get_number(dt, "Test"), Some(7.0));
        // A bad name fails with the list of what exists.
        let mut bad = file;
        bad.link.push(Pair { a: "gun".into(), b: "nope".into() });
        let e = match run(&bad, &dir) { Err(e) => e, Ok(_) => panic!("should fail") };
        assert!(e.contains("nope") && e.contains("crate"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
