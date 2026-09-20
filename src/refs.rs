//! Finding, rewriting, and pruning entity-index references.
//!
//! Entity indices appear all over a dupe and nothing marks them as indices —
//! they're just numbers. A generic "renumber every number that looks like an
//! index" pass would happily rewrite a prop's mass or a link colour. So known
//! sites are listed explicitly; anything unrecognised is reported, not touched.

use crate::dupe::Dupe;
use crate::value::{key_matches, table_index, Node, Value};

#[derive(Debug, Clone, Copy)]
pub enum Slot {
    Key(usize),   // nth entry's key in a Node::Table
    Value(usize), // nth entry's value in a Node::Table
    Item(usize),  // nth item in a Node::Array
}

/// Every ACF modifier that holds a list of linked entity indices, with the
/// entity class that owns it. From ACF-Team/ACF-3 lua/entities/*/init.lua.
pub const ACF_LINKS: &[&str] = &[
    "ACFCrates",      // gun, rack
    "ACFTurret",      // gun
    "ACFFuelTanks",   // engine
    "ACFGearboxes",   // engine, gearbox
    "ACFWheels",      // gearbox
    "ACFEffectors",   // gearbox
    "ACFRadar",       // rack
    "ACFComputer",    // rack
    "ACFGyro",        // turret
    "ACFMotor",       // turret
    "ACFAmmoCrates",  // autoloader
    "ACFGun",         // autoloader, turret computer
    "ACFWeapons",     // computer
    // acf_controller (the AIO controller) stores its links under bare field
    // names, one modifier per RegisterControllerLink in its modules/.
    "Baseplate",
    "Gearbox",
    "Seat",
    "SteerPlates",
    "Guns",
    "Turrets",
    "TurretComputer",
    "GuidanceComputer",
    "Racks",
    "Radar",
    "Receivers",
    // acf_crew lists what it serves; acf_controller's camera parents.
    "CrewTargets",
    "CamParents",
    // acf_baseplate's own generated seat.
    "LuaSeatID",
];

#[derive(Debug, Clone)]
pub struct RefHit {
    pub path: String,
    pub value: f64,
    /// A site we recognise as holding an entity index.
    pub known: bool,
    /// Points at an entity that actually exists. A known site that isn't
    /// live is a dangling reference — a bug in whatever produced the dupe.
    pub live: bool,
    pub table: usize,
    pub slot: Slot,
}

/// Does the table at this path use entity indices as its KEYS?
fn keys_are_indices(path: &str) -> bool {
    // Entities is keyed by entity index...
    path == "Entities"
    // ...and so is CFW's per-entity _links mirror of the constraint graph.
        || path.ends_with("._links")
}

/// Is this path one we recognise as holding an entity index?
fn classify(path: &str) -> bool {
    let ends_with = |k: &str| path.ends_with(&format!(".{k}"));

    // Constraints[3].Entity[1].Index
    if path.contains(".Entity[") && ends_with("Index") {
        return true;
    }
    // Constraints[3].MyCrtl — a WireHydraulic's gmod_wire_hydraulic controller
    if path.starts_with("Constraints[") && ends_with("MyCrtl") {
        return true;
    }
    // HeadEnt.Index
    if path.starts_with("HeadEnt") && ends_with("Index") {
        return true;
    }
    // Entities.155.BuildDupeInfo.DupeParentID
    if ends_with("DupeParentID") {
        return true;
    }
    // ACF link arrays. Every entity that links to others stores the targets
    // as an EntityMods array of indices, and the full set is wider than the
    // two this first knew about — read out of every StoreEntityModifier call
    // in ACF-Team/ACF-3, not guessed.
    if let Some(rest) = path.split(".EntityMods.").nth(1) {
        for name in ACF_LINKS {
            if rest.starts_with(name) && rest[name.len()..].starts_with('[') {
                return true;
            }
        }
    }
    // Wiremod: the source entity of a wire, and each waypoint along it
    if path.contains(".WireDupeInfo.") && ends_with("Src") {
        return true;
    }
    if path.contains(".WireDupeInfo.") && path.contains(".Path[") && ends_with("Entity") {
        return true;
    }
    // CFW: Entities.155._links.152 = { indexA, indexB, color }
    if path.contains("._links.") && (ends_with("indexA") || ends_with("indexB")) {
        return true;
    }
    // Tank Track Tool: EntityMods.tanktracktool.links.<wheel name> = index,
    // read back through the same createdEntities lookup Wiremod uses. The
    // legacy ttc_dupe_info.link_ents shape is still honoured by the addon.
    if path.contains(".EntityMods.tanktracktool.links.")
        || path.contains(".EntityMods.ttc_dupe_info.link_ents.")
    {
        return true;
    }
    false
}

/// Paths that match the "looks like a live entity index" heuristic but which
/// we've checked and know are something else. Listed so they stop cluttering
/// the discovery report.
fn known_false_positive(path: &str) -> bool {
    // CFW builds each link's colour with ColorRand(). Colour bytes run 0-255
    // and entity indices land in the same range, so they collide constantly.
    if path.contains("._links.") && path.contains(".color.") {
        return true;
    }
    // HeadEnt.Z is a height, not a reference. Usually fractional, which the
    // whole-number filter catches, but not always.
    if path == "HeadEnt.Z" {
        return true;
    }
    false
}

fn key_name(k: &Value) -> String {
    match k {
        Value::Str(b) => String::from_utf8_lossy(b).into_owned(),
        Value::Number(n) => format!("{n}"),
        other => format!("{other:?}"),
    }
}

fn is_index(v: &Value, target: f64) -> bool {
    matches!(v, Value::Number(n) if *n == target)
}

struct Scanner<'a> {
    dupe: &'a Dupe,
    live: Vec<f64>,
    seen: Vec<bool>,
    hits: Vec<RefHit>,
}

impl<'a> Scanner<'a> {
    fn check(&mut self, v: &Value, path: &str, table: usize, slot: Slot) {
        let Value::Number(n) = v else { return };
        let known = classify(path);
        let live = self.live.contains(n);

        // At a known site, always record — including dangling ones, which is
        // the whole point. At an unknown site, only bother if the number is a
        // whole number matching a real entity index, since that's the only
        // signal available that it might be a reference at all.
        if !known && (!live || n.fract() != 0.0 || known_false_positive(path)) {
            return;
        }
        self.hits.push(RefHit {
            path: path.to_string(),
            value: *n,
            known,
            live,
            table,
            slot,
        });
    }

    fn walk(&mut self, table: usize, path: &str) {
        if self.seen[table] {
            return; // shared tables are visited once; also stops cycles
        }
        self.seen[table] = true;

        // Copy the &Dupe out of self first. That detaches the arena borrow
        // from `self`, so the recursive &mut self calls below are allowed.
        let dupe = self.dupe;

        match &dupe.arena[table] {
            Node::Table(entries) => {
                let keyed_by_index = keys_are_indices(path);

                for (i, (k, v)) in entries.iter().enumerate() {
                    if keyed_by_index {
                        if let Value::Number(n) = k {
                            let live = self.live.contains(n);
                            self.hits.push(RefHit {
                                path: format!("{path}[{n}] (key)"),
                                value: *n,
                                known: true,
                                live,
                                table,
                                slot: Slot::Key(i),
                            });
                        }
                    }

                    let child = if path.is_empty() {
                        key_name(k)
                    } else {
                        format!("{path}.{}", key_name(k))
                    };
                    self.check(v, &child, table, Slot::Value(i));
                    if let Value::Table(t) = v {
                        self.walk(*t, &child);
                    }
                }
            }
            Node::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    let child = format!("{path}[{i}]");
                    self.check(v, &child, table, Slot::Item(i));
                    if let Value::Table(t) = v {
                        self.walk(*t, &child);
                    }
                }
            }
        }
    }
}

pub fn scan(dupe: &Dupe) -> Vec<RefHit> {
    let live: Vec<f64> = dupe.list_entities().into_iter().map(|(i, _, _)| i).collect();
    let mut s = Scanner {
        dupe,
        live,
        seen: vec![false; dupe.arena.len()],
        hits: Vec::new(),
    };
    let Ok(root) = dupe.root_table() else {
        return Vec::new();
    };
    s.walk(root, "");
    s.hits
}

/// Known references pointing at entities that don't exist.
///
/// Keys are NOT exempt. An earlier version skipped anything ending in
/// "(key)" to avoid flagging the Entities table's own keys — but those can
/// never be dangling, because the live-entity list is derived from them. All
/// that filter did was hide dangling _links keys, which are real bugs.
pub fn dangling(dupe: &Dupe) -> Vec<RefHit> {
    scan(dupe)
        .into_iter()
        // Entity index 0 is worldspawn. A constraint anchored to the world
        // legitimately names it, and it will never appear in the Entities
        // table, so treating it as dangling is a false positive.
        .filter(|h| h.known && !h.live && h.value != 0.0)
        .collect()
}

/// Rewrites every known reference according to `mapping` (old, new). Unknown
/// sites are deliberately left alone.
///
/// This is the primitive that makes merging two dupes possible: both will
/// have an entity 155, so one side gets remapped into a free range first.
pub fn remap(dupe: &mut Dupe, mapping: &[(f64, f64)]) -> usize {
    // Scan completes before any mutation, so the immutable borrow is done.
    let hits = scan(dupe);
    let mut changed = 0;

    for hit in hits.iter().filter(|h| h.known) {
        let Some(&(_, to)) = mapping.iter().find(|(from, _)| *from == hit.value) else {
            continue;
        };
        match (&mut dupe.arena[hit.table], hit.slot) {
            (Node::Table(entries), Slot::Key(i)) => {
                entries[i].0 = Value::Number(to);
                changed += 1;
            }
            (Node::Table(entries), Slot::Value(i)) => {
                entries[i].1 = Value::Number(to);
                changed += 1;
            }
            (Node::Array(items), Slot::Item(i)) => {
                items[i] = Value::Number(to);
                changed += 1;
            }
            _ => {}
        }
    }
    changed
}

// ---------------------------------------------------------------------------
// Pruning
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct PruneReport {
    pub links: usize,
    pub acf: usize,
    pub parents: usize,
    pub wires: usize,
    pub wire_path: usize,
    /// Tank Track Tool wheel links removed.
    pub tracks: usize,
}

impl PruneReport {
    pub fn total(&self) -> usize {
        self.links + self.acf + self.parents + self.wires + self.wire_path + self.tracks
    }
}

/// Drops entries in a keyed table where `keep` says no. Returns how many went.
fn retain_entries(dupe: &mut Dupe, table: usize, keep: impl Fn(&Value, &Value) -> bool) -> usize {
    let Node::Table(entries) = &mut dupe.arena[table] else {
        return 0;
    };
    let before = entries.len();
    entries.retain(|(k, v)| keep(k, v));
    before - entries.len()
}

fn retain_items(dupe: &mut Dupe, table: usize, keep: impl Fn(&Value) -> bool) -> usize {
    let Node::Array(items) = &mut dupe.arena[table] else {
        return 0;
    };
    let before = items.len();
    items.retain(|v| keep(v));
    before - items.len()
}

/// Removes every reference to `target` outside the root Constraints list,
/// which delete_entity handles itself. Each site gets the removal that keeps
/// the dupe coherent, which is not the same thing everywhere: a dead ACF
/// crate link drops one array item, a dead wire source drops the whole port.
pub fn prune_entity(dupe: &mut Dupe, target: f64) -> PruneReport {
    let mut report = PruneReport::default();

    let Ok(root) = dupe.root_table() else {
        return report;
    };
    let Some(ents) = dupe.get_table(root, "Entities") else {
        return report;
    };

    // Collect the entity tables up front so the arena isn't borrowed while
    // we mutate it below.
    let entity_tables: Vec<usize> = match &dupe.arena[ents] {
        Node::Table(entries) => entries.iter().filter_map(|(_, v)| table_index(v)).collect(),
        _ => Vec::new(),
    };

    for et in entity_tables {
        // _links is keyed by the partner's entity index, so the whole entry
        // goes. indexA/indexB inside a surviving entry can't name the target,
        // because they're always {this entity, the key}.
        if let Some(links) = dupe.get_table(et, "_links") {
            report.links += retain_entries(dupe, links, |k, _| !is_index(k, target));
        }

        // Parenting: an entity whose parent was deleted becomes unparented.
        if let Some(bdi) = dupe.get_table(et, "BuildDupeInfo") {
            report.parents += retain_entries(dupe, bdi, |k, v| {
                !(key_matches(k, "DupeParentID") && is_index(v, target))
            });
        }

        let Some(mods) = dupe.get_table(et, "EntityMods") else {
            continue;
        };

        // ACF link arrays: drop the single dead index, keep the rest.
        for name in ACF_LINKS {
            if let Some(arr) = dupe.get_table(mods, name) {
                report.acf += retain_items(dupe, arr, |v| !is_index(v, target));
            }
        }

        // Tank tracks: a link to a deleted wheel is removed by name. The track
        // rebuilds from whatever links survive; a dangling one would resolve
        // to the wrong entity after paste renumbering.
        for (modifier, list) in [("tanktracktool", "links"), ("ttc_dupe_info", "link_ents")] {
            if let Some(m) = dupe.get_table(mods, modifier) {
                if let Some(links) = dupe.get_table(m, list) {
                    if let Node::Table(entries) = &mut dupe.arena[links] {
                        let before = entries.len();
                        entries.retain(|(_, v)| !is_index(v, target));
                        report.tracks += before - entries.len();
                    }
                }
            }
        }

        // Wiremod. A port whose source entity is gone has nothing left to
        // mean, so the port goes. A waypoint on a surviving wire is just a
        // routing point, so only that point goes.
        let Some(wdi) = dupe.get_table(mods, "WireDupeInfo") else {
            continue;
        };
        let Some(wires) = dupe.get_table(wdi, "Wires") else {
            continue;
        };

        let ports: Vec<(String, usize)> = match &dupe.arena[wires] {
            Node::Table(entries) => entries
                .iter()
                .filter_map(|(k, v)| Some((key_name(k), table_index(v)?)))
                .collect(),
            _ => Vec::new(),
        };

        let mut dead_ports: Vec<String> = Vec::new();
        let mut paths_to_clean: Vec<usize> = Vec::new();
        for (name, port) in &ports {
            if dupe
                .get(*port, "Src")
                .map(|v| is_index(v, target))
                .unwrap_or(false)
            {
                dead_ports.push(name.clone());
            } else if let Some(path) = dupe.get_table(*port, "Path") {
                paths_to_clean.push(path);
            }
        }

        if !dead_ports.is_empty() {
            report.wires +=
                retain_entries(dupe, wires, |k, _| !dead_ports.contains(&key_name(k)));
        }

        for path in paths_to_clean {
            let doomed: Vec<usize> = match &dupe.arena[path] {
                Node::Array(items) => items
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| {
                        table_index(v)
                            .and_then(|t| dupe.get(t, "Entity"))
                            .map(|e| is_index(e, target))
                            .unwrap_or(false)
                    })
                    .map(|(i, _)| i)
                    .collect(),
                _ => Vec::new(),
            };
            if doomed.is_empty() {
                continue;
            }
            report.wire_path += doomed.len();
            if let Node::Array(items) = &mut dupe.arena[path] {
                for &i in doomed.iter().rev() {
                    items.remove(i);
                }
            }
        }
    }

    report
}
