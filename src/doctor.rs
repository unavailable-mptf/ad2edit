//! The ACF doctor: what eight reference builds said a working tank looks
//! like, as checks. Findings are advice, not refusals — a stationary
//! phalanx has no wheels on purpose.

use crate::build::{ACF_LINK_TABLE, ACF_MAX_LINK_DISTANCE};
use crate::dupe::Dupe;
use crate::extras;
use crate::refs;
use crate::transform;
use crate::value::{table_index, Node, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum Severity {
    /// Will not paste, or pastes broken.
    Error,
    /// Pastes, but behaves in a way every reference avoids.
    Warn,
    /// Worth knowing.
    Note,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub entity: Option<f64>,
    pub what: String,
    /// A mechanical correction, when there is one. `--doctor --fix` applies it.
    pub fix: Option<Fix>,
}

/// Corrections the doctor knows how to make. Each is one bounded edit.
#[derive(Debug, Clone, PartialEq)]
pub enum Fix {
    /// Set a numeric field by dotted path on an entity.
    Put { entity: f64, key: String, value: f64 },
    /// Remove an EntityMods modifier.
    Unmod { entity: f64, modifier: String },
    /// Make a hidden entity visible.
    Show { entity: f64 },
    /// Drop one target from a link array on the owner.
    Unlink { owner: f64, modifier: String, target: f64 },
    /// Move an entity by a world delta.
    Move { entity: f64, delta: (f64, f64, f64) },
    /// Clamp a modifier's numeric field into a range.
    Clamp { entity: f64, modifier: String, key: String, value: f64 },
    /// Remove a top-level entity key.
    Strip { entity: f64, key: String },
    /// Remove a whole entity (a stale wire source etc.) — never generated automatically.
    Nothing,
}

fn class_of(dupe: &Dupe, index: f64) -> String {
    transform::entity_table(dupe, index)
        .and_then(|et| match dupe.get(et, "Class") {
            Some(Value::Str(b)) => Some(String::from_utf8_lossy(b).to_lowercase()),
            _ => None,
        })
        .unwrap_or_default()
}

fn dist(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt()
}

fn number_in(dupe: &Dupe, table: usize, path: &[&str]) -> Option<f64> {
    let mut here = table;
    for seg in &path[..path.len() - 1] {
        here = dupe.get_table(here, seg)?;
    }
    dupe.get_number(here, path[path.len() - 1])
}

fn link_targets(dupe: &Dupe, mods: usize, name: &str) -> Vec<f64> {
    let Some(arr) = dupe.get_table(mods, name) else { return Vec::new() };
    match &dupe.arena[arr] {
        Node::Array(items) => items.iter().filter_map(|v| if let Value::Number(n) = v { Some(*n) } else { None }).collect(),
        Node::Table(e) => e.iter().filter_map(|(_, v)| if let Value::Number(n) = v { Some(*n) } else { None }).collect(),
    }
}

pub fn doctor(dupe: &Dupe) -> Vec<Finding> {
    doctor_with(dupe, None)
}

/// With a model library the driveshaft check uses real attachment points;
/// without one it uses entity origins, which still catches gross errors.
pub fn doctor_with(dupe: &Dupe, mut lib: Option<&mut crate::models::ModelLibrary>) -> Vec<Finding> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<Finding>, severity: Severity, entity: Option<f64>, what: String| {
        out.push(Finding { severity, entity, what, fix: None });
    };
    let push_fix = |out: &mut Vec<Finding>, severity: Severity, entity: Option<f64>, what: String, fix: Fix| {
        out.push(Finding { severity, entity, what, fix: Some(fix) });
    };

    // --- structure AD2 itself refuses -----------------------------------------
    for p in extras::validate(dupe) {
        push(&mut out, Severity::Error, None, format!("AD2 would refuse: {p}"));
    }
    // CFW's _links are runtime bookkeeping every reference dupe carries
    // stale copies of; they resolve to nothing on paste and hurt nothing.
    let mut stale_cfw = 0;
    for h in refs::dangling(dupe) {
        if h.path.contains("._links") {
            stale_cfw += 1;
            continue;
        }
        push(&mut out, Severity::Error, None, format!("dangling reference {} = {}", h.path, h.value));
    }
    if stale_cfw > 0 {
        push(&mut out, Severity::Note, None, format!("{stale_cfw} stale CFW _links entries (inert; every reference dupe has them)"));
    }

    let ents = dupe.list_entities();
    let by_class = |c: &str| -> Vec<f64> { ents.iter().filter(|(i, _, _)| class_of(dupe, *i) == c).map(|(i, _, _)| *i).collect() };
    let pos = |i: f64| transform::entity_transform(dupe, i).map(|(p, _)| p);

    // --- constraints ------------------------------------------------------------
    if let Ok(root) = dupe.root_table() {
        if let Some(cons) = dupe.get_table(root, "Constraints") {
            let items: Vec<usize> = match &dupe.arena[cons] {
                Node::Array(v) => v.iter().filter_map(table_index).collect(),
                Node::Table(e) => e.iter().filter_map(|(_, v)| table_index(v)).collect(),
            };
            let mut welds = 0;
            for c in items {
                let kind = match dupe.get(c, "Type") {
                    Some(Value::Str(b)) => String::from_utf8_lossy(b).into_owned(),
                    _ => String::new(),
                };
                if kind == "Weld" {
                    welds += 1;
                }
                for k in ["nocollide", "forcelimit", "torquelimit", "friction"] {
                    if let Some(Value::Bool(_)) = dupe.get(c, k) {
                        if kind == "Axis" || kind == "AdvBallsocket" || kind == "Ballsocket" {
                            push(&mut out, Severity::Error, None, format!("{kind}.{k} is a boolean; GMod compares it with > 0 and errors on paste"));
                        }
                    }
                }
            }
            if welds > 2 {
                push(&mut out, Severity::Warn, None, format!("{welds} welds; every reference build parents instead (welds only on chips)"));
            }
        }
    }

    // --- the plate and the controller -----------------------------------------
    let plates = by_class("acf_baseplate");
    if plates.is_empty() {
        push(&mut out, Severity::Warn, None, "no acf_baseplate".into());
    }
    let controllers = by_class("acf_controller");
    if controllers.is_empty() && !plates.is_empty() {
        push(&mut out, Severity::Note, None, "no acf_controller; nothing drives this".into());
    }
    for c in &controllers {
        let et = transform::entity_table(dupe, *c).unwrap();
        match number_in(dupe, et, &["DT", "BrakeEngagement"]) {
            Some(v) if v >= 1.0 => {}
            _ => push_fix(&mut out, Severity::Warn, Some(*c), "DT.BrakeEngagement is 0: brakes only on Space, coasts when the throttle is released (VDC sets 1)".into(), Fix::Put { entity: *c, key: "DT.BrakeEngagement".into(), value: 1.0 }),
        }
        // Single-target fields with more than one target: ACF keeps one.
        if let Some(mods) = dupe.get_table(et, "EntityMods") {
            for field in crate::rules::CONTROLLER_SINGLE_FIELDS {
                let t = link_targets(dupe, mods, field);
                if t.len() > 1 {
                    for extra in &t[1..] {
                        push_fix(&mut out, Severity::Error, Some(*c), format!("controller {field} has {} targets; ACF allows one; dropping {extra}", t.len()), Fix::Unlink { owner: *c, modifier: field.to_string(), target: *extra });
                    }
                }
            }
        }
        let mods = dupe.get_table(et, "EntityMods");
        let has = |n: &str| mods.map(|m| !link_targets(dupe, m, n).is_empty()).unwrap_or(false);
        if !has("Baseplate") {
            push(&mut out, Severity::Warn, Some(*c), "controller not linked to a baseplate".into());
        }
        if !has("Gearbox") && !by_class("acf_gearbox").is_empty() {
            push(&mut out, Severity::Warn, Some(*c), "controller not linked to a gearbox".into());
        }
    }

    // --- turrets ------------------------------------------------------------------
    for t in by_class("acf_turret") {
        let et = transform::entity_table(dupe, t).unwrap();
        let kind = match dupe.get(et, "Turret") {
            Some(Value::Str(b)) => String::from_utf8_lossy(b).into_owned(),
            _ => "?".into(),
        };
        match dupe.get_number(et, "MaxSpeed") {
            Some(v) if v > 0.0 => {}
            _ => {
                let cap = if kind == "Turret-V" { crate::rules::TRUNNION_MAX_SPEED } else { crate::rules::RING_MAX_SPEED };
                push_fix(&mut out, Severity::Warn, Some(t), format!("{kind}: no MaxSpeed cap; an uncapped ring runs at the motor rate (ironlock: 73.8°/s). Official T-34: ring 30, trunnion 10"), Fix::Put { entity: t, key: "MaxSpeed".into(), value: cap });
            }
        }
        if let Some(mods) = dupe.get_table(et, "EntityMods") {
            let motors = link_targets(dupe, mods, "ACFMotor");
            if motors.len() > crate::rules::MAX_MOTORS_PER_TURRET {
                for extra in &motors[1..] {
                    push_fix(&mut out, Severity::Error, Some(t), format!("{kind} has {} motors; ACF allows one", motors.len()), Fix::Unlink { owner: t, modifier: "ACFMotor".into(), target: *extra });
                }
            }
            let gyros = link_targets(dupe, mods, "ACFGyro");
            if gyros.len() > crate::rules::MAX_GYROS_PER_TURRET {
                for extra in &gyros[1..] {
                    push_fix(&mut out, Severity::Error, Some(t), format!("{kind} has {} gyros; ACF allows one", gyros.len()), Fix::Unlink { owner: t, modifier: "ACFGyro".into(), target: *extra });
                }
            }
        }
        if kind == "Turret-V" {
            let lo = dupe.get_number(et, "MinDeg").unwrap_or(-180.0);
            let hi = dupe.get_number(et, "MaxDeg").unwrap_or(180.0);
            if lo <= -90.0 || hi >= 90.0 {
                push(&mut out, Severity::Note, Some(t), format!("Turret-V elevation {lo}..{hi}; a real gun is about -5..+15"));
            }
        }
        let mods = dupe.get_table(et, "EntityMods");
        if !mods.map(|m| !link_targets(dupe, m, "ACFMotor").is_empty()).unwrap_or(false) {
            push(&mut out, Severity::Note, Some(t), format!("{kind} has no motor linked (hand-crank speed; the official T-34 is built this way)"));
        }
    }

    // --- crew, guns, engines, gearboxes ------------------------------------------
    for c in by_class("acf_crew") {
        let et = transform::entity_table(dupe, c).unwrap();
        let mods = dupe.get_table(et, "EntityMods");
        let targets = mods.map(|m| link_targets(dupe, m, "CrewTargets")).unwrap_or_default();
        if targets.is_empty() {
            push(&mut out, Severity::Warn, Some(c), "crew with no CrewTargets (should list what it serves)".into());
        }
        // crew_types.lua LinkHandlers: a Gunner serves the turret, a Loader the
        // gun, a Driver the baseplate. The wrong pairing is refused in game.
        let occupation = match dupe.get(et, "CrewTypeID") {
            Some(Value::Str(b)) => String::from_utf8_lossy(b).into_owned(),
            _ => String::new(),
        };
        let allowed = crate::rules::crew_targets(&occupation);
        if !allowed.is_empty() {
            for t in &targets {
                let tc = class_of(dupe, *t);
                if !allowed.contains(&tc.as_str()) {
                    push_fix(&mut out, Severity::Error, Some(c), format!("{occupation} crew linked to {tc} {t}; a {occupation} serves {}; dropping it", allowed.join("/")), Fix::Unlink { owner: c, modifier: "CrewTargets".into(), target: *t });
                }
            }
        }
    }
    let guns = by_class("acf_gun");
    let str_of = |et: usize, k: &str| match dupe.get(et, k) { Some(Value::Str(b)) => String::from_utf8_lossy(b).into_owned(), _ => String::new() };
    for g in &guns {
        let et = transform::entity_table(dupe, *g).unwrap();
        let mods = dupe.get_table(et, "EntityMods");
        let crates = mods.map(|m| link_targets(dupe, m, "ACFCrates")).unwrap_or_default();
        if crates.is_empty() {
            push(&mut out, Severity::Warn, Some(*g), "gun with no ammo crate linked".into());
        }
        // "Wrong ammo type for this weapon": weapon and caliber must match.
        let (gw, gc) = (str_of(et, "Weapon"), dupe.get_number(et, "Caliber").unwrap_or(0.0));
        if !gw.is_empty() {
            for c in &crates {
                let Some(cet) = transform::entity_table(dupe, *c) else { continue };
                let (cw, cc) = (str_of(cet, "Weapon"), dupe.get_number(cet, "Caliber").unwrap_or(0.0));
                if !cw.is_empty() && !crate::rules::ammo_matches(&gw, gc, &cw, cc) {
                    push_fix(&mut out, Severity::Error, Some(*g), format!("gun {gw} {gc}mm linked to crate {c} of {cw} {cc}mm: \"Wrong ammo type for this weapon\"; dropping the link"), Fix::Unlink { owner: *g, modifier: "ACFCrates".into(), target: *c });
                }
                // Belt-fed: crate on the same parent root as the gun.
                if crate::rules::is_belted_weapon(&gw) {
                    let root = |i: f64| {
                        let mut cur = i;
                        for _ in 0..32 {
                            let Some(e) = transform::entity_table(dupe, cur) else { break };
                            let Some(bdi) = dupe.get_table(e, "BuildDupeInfo") else { break };
                            match dupe.get_number(bdi, "DupeParentID") {
                                Some(p) if transform::entity_table(dupe, p).is_some() => cur = p,
                                _ => break,
                            }
                        }
                        cur
                    };
                    if root(*g) != root(*c) {
                        push(&mut out, Severity::Warn, Some(*g), format!("belt-fed {gw}: crate {c} is not on the same parent root as the gun (BeltFedCheck refuses the link in game)"));
                    }
                }
            }
        }
    }
    // Engines: the direct parent must be the baseplate or the engine reports
    // "Parenting Issue" and stays off.
    for e in by_class("acf_engine") {
        let et = transform::entity_table(dupe, e).unwrap();
        let parent = dupe.get_table(et, "BuildDupeInfo").and_then(|b| dupe.get_number(b, "DupeParentID"));
        match parent {
            Some(p) if class_of(dupe, p) == crate::rules::ENGINE_PARENT_CLASS => {}
            Some(p) => push(&mut out, Severity::Error, Some(e), format!("engine parented to {} {p}; ACF requires the direct parent to be an acf_baseplate (\"Parenting Issue\")", class_of(dupe, p))),
            None => push(&mut out, Severity::Error, Some(e), "engine has no parent; ACF requires it parented to an acf_baseplate".into()),
        }
    }
    // Driveshaft angle: engine→gearbox, gearbox→gearbox, gearbox→wheel.
    for src in by_class("acf_engine").into_iter().chain(by_class("acf_gearbox")) {
        let Some(et) = transform::entity_table(dupe, src) else { continue };
        let Some(mods) = dupe.get_table(et, "EntityMods") else { continue };
        let targets: Vec<f64> = link_targets(dupe, mods, "ACFGearboxes").into_iter().chain(link_targets(dupe, mods, "ACFWheels")).collect();
        for t in targets {
            if let Some(a) = crate::driveshaft::link_angle(dupe, &mut lib, src, t) {
                if a > crate::rules::MAX_DRIVESHAFT_ANGLE_DEG {
                    push(&mut out, Severity::Error, Some(src), format!("driveshaft {src} -> {t} bends {a:.0}°; ACF refuses over {:.0}° (\"excessive driveshaft angle\")", crate::rules::MAX_DRIVESHAFT_ANGLE_DEG));
                }
            }
        }
    }
    // Fuel compatibility: engine item → fuel types it burns.
    for e in by_class("acf_engine") {
        let et = transform::entity_table(dupe, e).unwrap();
        let id = str_of(et, "Engine");
        let Some(fuels) = crate::rules::engine_fuels(&id) else { continue };
        let Some(mods) = dupe.get_table(et, "EntityMods") else { continue };
        for t in link_targets(dupe, mods, "ACFFuelTanks") {
            let Some(tet) = transform::entity_table(dupe, t) else { continue };
            let ft = str_of(tet, "FuelType");
            if !ft.is_empty() && !fuels.iter().any(|f| f.eq_ignore_ascii_case(&ft)) {
                push_fix(&mut out, Severity::Error, Some(e), format!("{id} burns {} but tank {t} holds {ft}: \"fuel type is incompatible\"; dropping the link", fuels.join("/")), Fix::Unlink { owner: e, modifier: "ACFFuelTanks".into(), target: t });
            }
        }
    }
    // Gun calibre within its class's CaliberLimits.
    for g in &guns {
        let et = transform::entity_table(dupe, *g).unwrap();
        let (w, c) = (str_of(et, "Weapon"), dupe.get_number(et, "Caliber").unwrap_or(0.0));
        if let Some((lo, hi)) = crate::rules::gun_calibre_range(&w) {
            if c > 0.0 && !(lo..=hi).contains(&c) {
                push_fix(&mut out, Severity::Error, Some(*g), format!("{w} at {c}mm; that class allows {lo}..{hi}; clamping"), Fix::Put { entity: *g, key: "Caliber".into(), value: c.clamp(lo, hi) });
            }
        }
    }
    // Baseplate, fuel tank and ammo sizes.
    for b in &plates {
        let et = transform::entity_table(dupe, *b).unwrap();
        if let Some(ud) = dupe.get_table(et, "ACF_UserData") {
            for (k, (lo, hi)) in [("Width", crate::rules::BASEPLATE_WIDTH), ("Length", crate::rules::BASEPLATE_LENGTH), ("Thickness", crate::rules::BASEPLATE_THICKNESS)] {
                if let Some(v) = dupe.get_number(ud, k) {
                    if !(lo..=hi).contains(&v) {
                        push_fix(&mut out, Severity::Error, Some(*b), format!("baseplate {k} {v} outside {lo}..{hi}; clamping"), Fix::Put { entity: *b, key: format!("ACF_UserData.{k}"), value: v.clamp(lo, hi) });
                    }
                }
            }
        }
    }
    // Size rules apply to legacy-format containers only: AutoRegister v2
    // entities (ACF_UserData present) store counts, and their Size is a
    // leftover the game never reads.
    let legacy = |et: usize| dupe.get_table(et, "ACF_UserData").is_none();
    for f in by_class("acf_fueltank") {
        let et = transform::entity_table(dupe, f).unwrap();
        if !legacy(et) { continue; }
        if let Some(Value::Vector(x, y, z)) = dupe.get(et, "Size") {
            let (lo, hi) = crate::rules::CONTAINER_SIZE;
            if [*x, *y, *z].iter().any(|v| !(lo..=hi).contains(v)) {
                push(&mut out, Severity::Error, Some(f), format!("fuel tank size ({x}, {y}, {z}) outside {lo}..{hi} per axis"));
            }
        }
    }
    // AmmoMinSize (6) is in globals.lua but the official T-34's MG crates
    // are 4.8 units on a side and paste fine: not a rule a dupe can carry.
    for t in by_class("acf_turret") {
        let et = transform::entity_table(dupe, t).unwrap();
        let kind = str_of(et, "Turret");
        let (lo, hi) = if kind == "Turret-V" { crate::rules::RING_SIZE_V } else { crate::rules::RING_SIZE_H };
        if let Some(r) = dupe.get_number(et, "RingSize") {
            if !(lo..=hi).contains(&r) {
                push_fix(&mut out, Severity::Error, Some(t), format!("{kind} RingSize {r} outside {lo}..{hi}; clamping"), Fix::Put { entity: t, key: "RingSize".into(), value: r.clamp(lo, hi) });
            }
        }
    }
    // Legality: Make Spherical or any visclip on an ACF entity disables it.
    for (i, _, _) in &ents {
        let cls = class_of(dupe, *i);
        if !cls.starts_with("acf_") { continue; }
        let Some(et) = transform::entity_table(dupe, *i) else { continue };
        let Some(mods) = dupe.get_table(et, "EntityMods") else { continue };
        for m in crate::rules::ILLEGAL_ON_ACF {
            if dupe.get_table(mods, m).is_some() {
                let why = if *m == "MakeSphericalCollisions" { "Invalid Physics: custom physics objects cannot be applied to ACF entities" } else { "Visual Clip: visclips cannot be applied to ACF entities" };
                push_fix(&mut out, Severity::Error, Some(*i), format!("{cls} has {m}: {why}; removing it"), Fix::Unmod { entity: *i, modifier: m.to_string() });
            }
        }
    }

    // Gearbox loops: "You cannot link gearboxes in a loop!"
    {
        let gbs = by_class("acf_gearbox");
        let next = |i: f64| -> Vec<f64> {
            transform::entity_table(dupe, i)
                .and_then(|et| dupe.get_table(et, "EntityMods"))
                .map(|m| link_targets(dupe, m, "ACFGearboxes"))
                .unwrap_or_default()
        };
        for start in &gbs {
            let mut stack = vec![(*start, 0usize)];
            let mut seen: Vec<f64> = Vec::new();
            while let Some((cur, depth)) = stack.pop() {
                if depth > 16 { break; }
                for n in next(cur) {
                    if n == *start {
                        push_fix(&mut out, Severity::Error, Some(cur), format!("gearbox loop: {cur} -> {start} closes a cycle; dropping that link"), Fix::Unlink { owner: cur, modifier: "ACFGearboxes".into(), target: *start });
                    } else if !seen.contains(&n) {
                        seen.push(n);
                        stack.push((n, depth + 1));
                    }
                }
            }
        }
    }
    for e in by_class("acf_engine") {
        let et = transform::entity_table(dupe, e).unwrap();
        let mods = dupe.get_table(et, "EntityMods");
        let has = |n: &str| mods.map(|m| !link_targets(dupe, m, n).is_empty()).unwrap_or(false);
        if !has("ACFFuelTanks") {
            push(&mut out, Severity::Warn, Some(e), "engine with no fuel tank linked".into());
        }
        if !has("ACFGearboxes") {
            push(&mut out, Severity::Warn, Some(e), "engine with no gearbox linked".into());
        }
    }
    for gb in by_class("acf_gearbox") {
        let et = transform::entity_table(dupe, gb).unwrap();
        let mods = dupe.get_table(et, "EntityMods");
        let has = |n: &str| mods.map(|m| !link_targets(dupe, m, n).is_empty()).unwrap_or(false);
        if !has("ACFWheels") && !has("ACFGearboxes") {
            push(&mut out, Severity::Warn, Some(gb), "gearbox drives nothing (no wheels or gearboxes linked)".into());
        }
        // Gear ratios: ±10, CVT 1..100 (globals.lua).
        let is_cvt = matches!(dupe.get(et, "Gearbox"), Some(Value::Str(b)) if String::from_utf8_lossy(b).starts_with("CVT"));
        // A CVT's Gears table holds its fixed slots (0.01, -1 in every
        // reference), not ratios; only manual/auto gears are ratios.
        if let Some(gears) = dupe.get_table(et, "Gears").filter(|_| !is_cvt) {
            let vals: Vec<f64> = match &dupe.arena[gears] {
                Node::Table(e) => e.iter().filter_map(|(_, v)| if let Value::Number(n) = v { Some(*n) } else { None }).collect(),
                Node::Array(v) => v.iter().filter_map(|x| if let Value::Number(n) = x { Some(*n) } else { None }).collect(),
            };
            for g in vals {
                let (lo, hi) = (crate::rules::MIN_GEAR_RATIO, crate::rules::MAX_GEAR_RATIO);
                if g != 0.0 && !(lo..=hi).contains(&g) {
                    push(&mut out, Severity::Warn, Some(gb), format!("gear ratio {g} outside ACF's {lo}..{hi}"));
                }
            }
        }
    }

    // --- link distances and pairs, by the registry ------------------------------
    for (i, _, _) in &ents {
        let Some(et) = transform::entity_table(dupe, *i) else { continue };
        let Some(mods) = dupe.get_table(et, "EntityMods") else { continue };
        let ca = class_of(dupe, *i);
        for (src, _dst, modifier) in ACF_LINK_TABLE {
            if *src != ca && !(*src == "prop_physics") {
                continue;
            }
            for t in link_targets(dupe, mods, modifier) {
                if t == 0.0 {
                    continue;
                }
                if let (Some(a), Some(b)) = (pos(*i), pos(t)) {
                    let d = dist(a, b);
                    if d > ACF_MAX_LINK_DISTANCE {
                        push(&mut out, Severity::Error, Some(*i), format!("{modifier} -> {t} is {d:.0} units away; ACF refuses over {ACF_MAX_LINK_DISTANCE:.0}"));
                    }
                }
            }
        }
    }

    // --- per-entity modifiers -----------------------------------------------------
    let mut hidden = 0;
    // A build with a prop2mesh hides its real parts on purpose; showing them
    // would draw everything twice in game.
    let has_prop2mesh = ents.iter().any(|(i, _, _)| class_of(dupe, *i) == "sent_prop2mesh");
    for (i, _, model) in &ents {
        let Some(et) = transform::entity_table(dupe, *i) else { continue };
        let Some(mods) = dupe.get_table(et, "EntityMods") else { continue };
        let has_mass = dupe.get_table(mods, "mass").is_some();
        let has_armour = dupe.get_table(mods, "ACF_Armor").is_some();
        if has_mass && has_armour {
            push_fix(&mut out, Severity::Warn, Some(*i), "both mass and ACF_Armor: ACF ignores the thickness and uses the mass; dropping the mass modifier".into(), Fix::Unmod { entity: *i, modifier: "mass".into() });
        }
        if let Some(a) = dupe.get_table(mods, "ACF_Armor") {
            if let Some(t) = dupe.get_number(a, "Thickness") {
                if t > crate::rules::SERVER_MAX_THICKNESS_DEFAULT && t <= crate::rules::MAX_ARMOUR_MM {
                    push(&mut out, Severity::Note, Some(*i), format!("armour {t}mm is over the default server MaxThickness ({}); the server clamps it", crate::rules::SERVER_MAX_THICKNESS_DEFAULT));
                }
                if !(crate::rules::MIN_ARMOUR_MM..=crate::rules::MAX_ARMOUR_MM).contains(&t) {
                    let v = t.clamp(crate::rules::MIN_ARMOUR_MM, crate::rules::MAX_ARMOUR_MM);
                    push_fix(&mut out, Severity::Error, Some(*i), format!("armour {t}mm outside ACF's 0.01..5000; clamping to {v}"), Fix::Clamp { entity: *i, modifier: "ACF_Armor".into(), key: "Thickness".into(), value: v });
                }
            }
            if let Some(d) = dupe.get_number(a, "Ductility") {
                if !(crate::rules::MIN_DUCTILITY..=crate::rules::MAX_DUCTILITY).contains(&d) {
                    let v = d.clamp(crate::rules::MIN_DUCTILITY, crate::rules::MAX_DUCTILITY);
                    push_fix(&mut out, Severity::Error, Some(*i), format!("ductility {d} outside ACF's -80..80; clamping to {v}"), Fix::Clamp { entity: *i, modifier: "ACF_Armor".into(), key: "Ductility".into(), value: v });
                }
            }
        }
        if let Some(m) = dupe.get_table(mods, "mass") {
            if let Some(kg) = dupe.get_number(m, "Mass") {
                if !(crate::rules::MIN_MASS_KG..=crate::rules::MAX_MASS_KG).contains(&kg) {
                    let v = kg.clamp(crate::rules::MIN_MASS_KG, crate::rules::MAX_MASS_KG);
                    push_fix(&mut out, Severity::Error, Some(*i), format!("mass {kg}kg outside ACF's 0.1..50000; clamping to {v}"), Fix::Clamp { entity: *i, modifier: "mass".into(), key: "Mass".into(), value: v });
                }
            }
        }
        if let Some(colour) = dupe.get_table(mods, "colour") {
            if number_in(dupe, colour, &["Color", "a"]).map(|a| a <= 0.0).unwrap_or(false) {
                hidden += 1;
                if has_prop2mesh {
                    push(&mut out, Severity::Note, Some(*i), "invisible (colour alpha 0); a prop2mesh draws it; left alone".into());
                } else {
                    push_fix(&mut out, Severity::Note, Some(*i), "invisible (colour alpha 0)".into(), Fix::Show { entity: *i });
                }
            }
        }
        for k in crate::rules::JUNK_ENTITY_KEYS {
            if dupe.get(et, k).is_some() {
                push_fix(&mut out, Severity::Note, Some(*i), format!("runtime key {k} (never read on paste)"), Fix::Strip { entity: *i, key: k.to_string() });
            }
        }
        let cls = class_of(dupe, *i);
        let looks_like_wheel = cls == "prop_physics" && (model.contains("wheel") || model.contains("tank") || model.contains("tire"));
        if looks_like_wheel && dupe.get_table(mods, "MakeSphericalCollisions").is_none() {
            push(&mut out, Severity::Note, Some(*i), "wheel without MakeSphericalCollisions (every reference wheel has it)".into());
        }
    }
    let _ = hidden;

    // --- crates near a gun's breech -------------------------------------------
    for crate_i in by_class("acf_ammo") {
        let Some(cp) = pos(crate_i) else { continue };
        for g in &guns {
            let Some((gp, ga)) = transform::entity_transform(dupe, *g) else { continue };
            // Breech sits behind the muzzle: 30 units back along the gun's forward.
            let fwd = transform::rotate_vec(ga, (1.0, 0.0, 0.0));
            let breech = (gp.0 - fwd.0 * 30.0, gp.1 - fwd.1 * 30.0, gp.2 - fwd.2 * 30.0);
            if dist(cp, breech) < 28.0 {
                // Back, away from the breech, along the gun's axis.
                let delta = (-fwd.0 * 40.0, -fwd.1 * 40.0, -fwd.2 * 40.0);
                push_fix(&mut out, Severity::Warn, Some(crate_i), format!("ammo crate within 28 units of gun {g}'s breech zone; moving it 40 back along the gun"), Fix::Move { entity: crate_i, delta });
            }
        }
    }

    // --- wires: Wiremod drops a wire to a port that is not there --------------
    for fault in crate::ports::wire_faults(dupe) {
        use crate::ports::WireProblem;
        let wire = format!("wire {} \"{}\" → {} \"{}\"", fault.source, fault.output, fault.entity, fault.input);
        let what = match fault.problem {
            WireProblem::UnknownInput => format!("{wire}: this entity has no input \"{}\", so the wire is lost on paste", fault.input),
            WireProblem::UnknownOutput => format!("{wire}: entity {} has no output \"{}\", so the wire is lost on paste", fault.source, fault.output),
            WireProblem::TypeMismatch { output, input } => format!("{wire}: a {output:?} output cannot drive a {input:?} input"),
        };
        push(&mut out, Severity::Warn, Some(fault.entity), what);
    }

    out
}

/// Applies every finding's fix. Returns what was done, in order.
pub fn apply_fixes(dupe: &mut Dupe, findings: &[Finding]) -> Vec<String> {
    let mut done = Vec::new();
    for f in findings {
        let Some(fix) = &f.fix else { continue };
        let ok = match fix {
            Fix::Put { entity, key, value } => crate::build::put_path(dupe, *entity, key, Value::Number(*value)).is_ok(),
            Fix::Unmod { entity, modifier } => remove_modifier(dupe, *entity, modifier),
            Fix::Show { entity } => show_entity(dupe, *entity),
            Fix::Unlink { owner, modifier, target } => unlink(dupe, *owner, modifier, *target),
            Fix::Move { entity, delta } => transform::set_entity_transform(dupe, *entity, transform::entity_transform(dupe, *entity).map(|(p, _)| (p.0 + delta.0, p.1 + delta.1, p.2 + delta.2)), None).is_ok(),
            Fix::Clamp { entity, modifier, key, value } => {
                transform::entity_table(dupe, *entity)
                    .and_then(|et| dupe.get_table(et, "EntityMods"))
                    .and_then(|m| dupe.get_table(m, modifier))
                    .map(|t| { dupe.set(t, key, Value::Number(*value)); true })
                    .unwrap_or(false)
            }
            Fix::Strip { entity, key } => {
                transform::entity_table(dupe, *entity)
                    .map(|et| {
                        if let Node::Table(e) = &mut dupe.arena[et] {
                            let before = e.len();
                            e.retain(|(k, _)| !crate::value::key_matches(k, key));
                            e.len() < before
                        } else { false }
                    })
                    .unwrap_or(false)
            }
            Fix::Nothing => false,
        };
        if ok {
            done.push(f.what.clone());
        }
    }
    if !done.is_empty() {
        transform::resync_constraints(dupe);
    }
    done
}

fn remove_modifier(dupe: &mut Dupe, entity: f64, name: &str) -> bool {
    let Some(et) = transform::entity_table(dupe, entity) else { return false };
    let Some(mods) = dupe.get_table(et, "EntityMods") else { return false };
    if let Node::Table(e) = &mut dupe.arena[mods] {
        let before = e.len();
        e.retain(|(k, _)| !crate::value::key_matches(k, name));
        return e.len() < before;
    }
    false
}

fn show_entity(dupe: &mut Dupe, entity: f64) -> bool {
    let Some(et) = transform::entity_table(dupe, entity) else { return false };
    let Some(mods) = dupe.get_table(et, "EntityMods") else { return false };
    let Some(colour) = dupe.get_table(mods, "colour") else { return false };
    if let Some(c) = dupe.get_table(colour, "Color") {
        dupe.set(c, "a", Value::Number(255.0));
    }
    dupe.set(colour, "RenderMode", Value::Number(0.0));
    true
}

fn unlink(dupe: &mut Dupe, owner: f64, modifier: &str, target: f64) -> bool {
    let Some(et) = transform::entity_table(dupe, owner) else { return false };
    let Some(mods) = dupe.get_table(et, "EntityMods") else { return false };
    let Some(arr) = dupe.get_table(mods, modifier) else { return false };
    match &mut dupe.arena[arr] {
        Node::Array(items) => { let b = items.len(); items.retain(|v| !matches!(v, Value::Number(n) if *n == target)); items.len() < b }
        Node::Table(e) => { let b = e.len(); e.retain(|(_, v)| !matches!(v, Value::Number(n) if *n == target)); e.len() < b }
    }
}

/// `--clean`: the junk keys off every entity, empty EntityMods tables, and
/// wire ports whose source is gone. Nothing that pastes differently.
pub fn clean(dupe: &mut Dupe) -> Vec<String> {
    let mut done = Vec::new();
    let ents = dupe.list_entities();
    for (i, _, _) in &ents {
        let Some(et) = transform::entity_table(dupe, *i) else { continue };
        for k in crate::rules::JUNK_ENTITY_KEYS {
            if let Node::Table(e) = &mut dupe.arena[et] {
                let b = e.len();
                e.retain(|(kk, _)| !crate::value::key_matches(kk, k));
                if e.len() < b { done.push(format!("{i}: removed {k}")); }
            }
        }
    }
    // Dangling references outside CFW: prune by target.
    let mut targets: Vec<f64> = refs::dangling(dupe).iter().filter(|h| !h.path.contains("._links")).map(|h| h.value).collect();
    targets.sort_by(|a, b| a.partial_cmp(b).unwrap());
    targets.dedup();
    for t in targets {
        let r = refs::prune_entity(dupe, t);
        if r.total() > 0 { done.push(format!("pruned {} dangling reference(s) to {t}", r.total())); }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::fixture;

    #[test]
    fn the_fixture_gets_the_warnings_it_deserves_and_no_errors() {
        let d = fixture();
        let f = doctor(&d);
        assert!(f.iter().all(|x| x.severity != Severity::Error), "{f:?}");
        // The fixture turret has no motor and no cap; the gun has a crate.
        assert!(f.iter().any(|x| x.what.contains("MaxSpeed")));
        assert!(f.iter().any(|x| x.what.contains("no motor")));
        assert!(!f.iter().any(|x| x.what.contains("no ammo crate")));
    }

    #[test]
    fn fixes_apply_and_the_dupe_comes_back_clean() {
        let mut d = fixture();
        // A controller with brakes off and two baseplates linked, a crew of
        // the wrong occupation, a gun linked to a crate of another calibre.
        let root = d.root_table().unwrap();
        let ents = d.get_table(root, "Entities").unwrap();
        let mk = |d: &mut Dupe, idx: f64, class: &str| -> usize {
            let et = d.new_table();
            d.set(et, "Class", Value::Str(class.as_bytes().to_vec()));
            d.set(et, "Model", Value::Str(b"models/x.mdl".to_vec()));
            let ph = d.new_table(); let b = d.new_table();
            d.set(b, "Pos", Value::Vector(0.0, 0.0, 0.0)); d.set(b, "Angle", Value::Angle(0.0, 0.0, 0.0));
            d.set(ph, "0", Value::Table(b));
            if let Node::Table(e) = &mut d.arena[ph] { e[0].0 = Value::Number(0.0); }
            d.set(et, "PhysicsObjects", Value::Table(ph));
            let m = d.new_table(); d.set(et, "EntityMods", Value::Table(m));
            if let Node::Table(e) = &mut d.arena[ents] { e.push((Value::Number(idx), Value::Table(et))); }
            et
        };
        let ctrl = mk(&mut d, 40.0, "acf_controller");
        crate::build::add_link(&mut d, 40.0, 10.0).unwrap();
        let mods = d.get_table(ctrl, "EntityMods").unwrap();
        let bp = d.get_table(mods, "Baseplate").unwrap();
        if let Node::Array(v) = &mut d.arena[bp] { v.push(Value::Number(10.0)); } // duplicate target
        let crew = mk(&mut d, 41.0, "acf_crew");
        d.set(crew, "CrewTypeID", Value::Str(b"Gunner".to_vec()));
        crate::build::add_link(&mut d, 41.0, 11.0).unwrap(); // gunner -> gun: invalid
        let gun = transform::entity_table(&d, 11.0).unwrap();
        d.set(gun, "Weapon", Value::Str(b"C".to_vec()));
        d.set(gun, "Caliber", Value::Number(100.0));
        let crate_ = transform::entity_table(&d, 12.0).unwrap();
        d.set(crate_, "Weapon", Value::Str(b"C".to_vec()));
        d.set(crate_, "Caliber", Value::Number(75.0));

        let f = doctor(&d);
        assert!(f.iter().any(|x| x.what.contains("Gunner crew linked to acf_gun")));
        assert!(f.iter().any(|x| x.what.contains("Wrong ammo type")));
        assert!(f.iter().any(|x| x.what.contains("has 2 targets")));
        let done = apply_fixes(&mut d, &f);
        assert!(done.len() >= 4, "{done:?}");
        let f2 = doctor(&d);
        assert!(!f2.iter().any(|x| x.severity == Severity::Error), "{f2:?}");
        assert!(refs::dangling(&d).is_empty());
    }

    #[test]
    fn a_boolean_axis_flag_is_an_error_and_a_far_link_is_an_error() {
        let mut d = fixture();
        crate::build::add_axis(&mut d, 10.0, 14.0, &crate::build::AxisSpec { point: (0.0, 60.0, 0.0), axis: (0.0, 1.0, 0.0), friction: 0.0, forcelimit: 0.0, torquelimit: 0.0, nocollide: true }).unwrap();
        // Corrupt it the way the first car was.
        let root = d.root_table().unwrap();
        let cons = d.get_table(root, "Constraints").unwrap();
        let last = match &d.arena[cons] { Node::Array(v) => table_index(v.last().unwrap()).unwrap(), _ => panic!() };
        d.set(last, "nocollide", Value::Bool(true));
        transform::set_entity_transform(&mut d, 12.0, Some((3000.0, 0.0, 0.0)), None).unwrap();
        let f = doctor(&d);
        assert!(f.iter().any(|x| x.severity == Severity::Error && x.what.contains("boolean")));
        assert!(f.iter().any(|x| x.severity == Severity::Error && x.what.contains("units away")));
    }
}
