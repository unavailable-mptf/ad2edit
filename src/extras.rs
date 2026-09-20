//! Two things that didn't fit anywhere else: welding an entity to the world,
//! and comparing two dupes.

use std::collections::BTreeMap;

use crate::dupe::Dupe;
use crate::transform;
use crate::value::{key_matches, table_index, Node, Value};

/// Adds a Weld between an entity and the world.
///
/// The table shape is copied from a real dupe — a weld carries `Entity`,
/// `LPos2`, `Type`, `forcelimit`, `nocollide` and a `BuildDupeInfo` of
/// `Ent1Ang`, `Ent1Pos`, `Ent2Ang`, `EntityPos`. The world endpoint is
/// `{ World = true, Index = 0 }`, which is exactly how AdvDupe2 writes one and
/// how its paste code tells the world apart from a missing entity.
pub fn weld_to_world(dupe: &mut Dupe, entity: f64) -> Result<(), String> {
    let (pos, ang) = transform::entity_transform(dupe, entity)
        .ok_or_else(|| format!("entity {entity} has no readable transform"))?;

    let root = dupe.root_table()?;
    let cons = match dupe.get_table(root, "Constraints") {
        Some(c) => c,
        None => {
            let c = dupe.new_table();
            dupe.set(root, "Constraints", Value::Table(c));
            c
        }
    };

    let s = |t: &str| Value::Str(t.as_bytes().to_vec());

    // Endpoints. Slot 1 is the entity, slot 2 the world — matching GMod's
    // constraint.Weld, which swaps so the world is never Ent1.
    let ep1 = dupe.new_table();
    dupe.set(ep1, "Index", Value::Number(entity));
    let ep2 = dupe.new_table();
    dupe.set(ep2, "World", Value::Bool(true));
    dupe.set(ep2, "Index", Value::Number(0.0));
    let ends = dupe.new_table();
    if let Node::Table(_) = &dupe.arena[ends] {
        dupe.arena[ends] = Node::Array(vec![Value::Table(ep1), Value::Table(ep2)]);
    }

    // The pose cache AD2 rebuilds the constraint from. Ent1Pos is a WORLD
    // position at copy time; for something we're adding in the editor the
    // entity's stored position is the only one we have, and resync on save
    // writes EntityPos/Ent1Ang/Ent2Ang the same way it does for everything.
    let bdi = dupe.new_table();
    dupe.set(bdi, "EntityPos", Value::Vector(pos.0, pos.1, pos.2));
    dupe.set(bdi, "Ent1Pos", Value::Vector(pos.0, pos.1, pos.2));
    dupe.set(bdi, "Ent1Ang", Value::Angle(ang.0, ang.1, ang.2));
    dupe.set(bdi, "Ent2Ang", Value::Angle(0.0, 0.0, 0.0));

    let weld = dupe.new_table();
    dupe.set(weld, "Type", s("Weld"));
    dupe.set(weld, "Entity", Value::Table(ends));
    dupe.set(weld, "LPos2", Value::Vector(pos.0, pos.1, pos.2));
    dupe.set(weld, "forcelimit", Value::Number(0.0));
    dupe.set(weld, "nocollide", Value::Bool(false));
    dupe.set(weld, "BuildDupeInfo", Value::Table(bdi));

    match &mut dupe.arena[cons] {
        Node::Array(items) => items.push(Value::Table(weld)),
        Node::Table(entries) => {
            let n = entries.len() as f64 + 1.0;
            entries.push((Value::Number(n), Value::Table(weld)));
        }
    }
    Ok(())
}

/// AdvDupe2's own `CheckValidDupe`, from sh_codec.lua, so a dupe the CLI
/// writes is refused here rather than by the game. Returns every problem
/// found, in the order AD2 would hit them.
pub fn validate(dupe: &Dupe) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(root) = dupe.root_table() else {
        return vec!["root is not a table".into()];
    };
    let head = dupe.get_table(root, "HeadEnt");
    let ents = dupe.get_table(root, "Entities");
    if head.is_none() {
        out.push("Missing HeadEnt table".into());
    }
    if ents.is_none() {
        out.push("Missing Entities table".into());
    }
    if dupe.get_table(root, "Constraints").is_none() {
        out.push("Missing Constraints table".into());
    }
    if let Some(h) = head {
        if dupe.get(h, "Z").is_none() {
            out.push("Missing HeadEnt.Z".into());
        }
        if !matches!(dupe.get(h, "Pos"), Some(Value::Vector(..))) {
            out.push("Missing HeadEnt.Pos".into());
        }
        match dupe.get_number(h, "Index") {
            None => out.push("Missing HeadEnt.Index".into()),
            Some(i) => {
                if transform::entity_table(dupe, i).is_none() {
                    out.push(format!("Missing HeadEnt index [{i}] from Entities table"));
                }
            }
        }
    }
    for (index, class, model) in dupe.list_entities() {
        let Some(et) = transform::entity_table(dupe, index) else { continue };
        let tag = format!("Entity [{index}][{class}][{model}]");
        let Some(physics) = dupe.get_table(et, "PhysicsObjects") else {
            out.push(format!("Missing PhysicsObject table from {tag}"));
            continue;
        };
        // Bone 0 is keyed by the NUMBER 0, as AD2 writes it.
        let bone0 = match &dupe.arena[physics] {
            Node::Table(entries) => entries
                .iter()
                .find(|(k, _)| matches!(k, Value::Number(n) if *n == 0.0) || key_matches(k, "0"))
                .and_then(|(_, v)| table_index(v)),
            Node::Array(items) => items.first().and_then(table_index),
        };
        let Some(b) = bone0 else {
            out.push(format!("Missing PhysicsObject[0] table from {tag}"));
            continue;
        };
        if !matches!(dupe.get(b, "Pos"), Some(Value::Vector(..))) {
            out.push(format!("Missing PhysicsObject[0].Pos from {tag}"));
        }
        if !matches!(dupe.get(b, "Angle"), Some(Value::Angle(..))) {
            out.push(format!("Missing PhysicsObject[0].Angle from {tag}"));
        }
    }
    out
}

/// Fills in a HeadEnt that a build left incomplete: Index from the first
/// entity if absent or dangling, Pos from that entity, Z of 0 (the height
/// above the ground the paste ghost sits at). AD2 only ever reads these; a
/// dupe written by hand has no reason to be missing them.
pub fn repair_head(dupe: &mut Dupe) -> Vec<String> {
    let mut fixed = Vec::new();
    let Ok(root) = dupe.root_table() else { return fixed };
    let ents = dupe.list_entities();
    let Some((first, _, _)) = ents.first() else { return fixed };
    let first = *first;

    let head = match dupe.get_table(root, "HeadEnt") {
        Some(h) => h,
        None => {
            let h = dupe.new_table();
            dupe.set(root, "HeadEnt", Value::Table(h));
            fixed.push("created HeadEnt".into());
            h
        }
    };
    let index = match dupe.get_number(head, "Index") {
        Some(i) if transform::entity_table(dupe, i).is_some() => i,
        _ => {
            dupe.set(head, "Index", Value::Number(first));
            fixed.push(format!("HeadEnt.Index -> {first}"));
            first
        }
    };
    if !matches!(dupe.get(head, "Pos"), Some(Value::Vector(..))) {
        let p = transform::entity_transform(dupe, index).map(|(p, _)| p).unwrap_or((0.0, 0.0, 0.0));
        dupe.set(head, "Pos", Value::Vector(p.0, p.1, p.2));
        fixed.push(format!("HeadEnt.Pos -> ({:.1}, {:.1}, {:.1})", p.0, p.1, p.2));
    }
    if dupe.get(head, "Z").is_none() {
        dupe.set(head, "Z", Value::Number(0.0));
        fixed.push("HeadEnt.Z -> 0".into());
    }
    if dupe.get_table(root, "Constraints").is_none() {
        let c = dupe.new_array();
        dupe.set(root, "Constraints", Value::Table(c));
        fixed.push("created empty Constraints".into());
    }
    fixed
}

/// One line of a diff.
#[derive(Debug, PartialEq)]
pub enum Change {
    OnlyInLeft(f64, String),
    OnlyInRight(f64, String),
    /// Same entity index in both, a top-level field differs.
    Field(f64, String, String, String),
    ConstraintCount(usize, usize),
}

fn show(dupe: &Dupe, v: &Value) -> String {
    match v {
        Value::Str(b) => String::from_utf8_lossy(b).into_owned(),
        Value::Number(n) => format!("{n}"),
        Value::Bool(b) => format!("{b}"),
        Value::Vector(x, y, z) => format!("({x:.3}, {y:.3}, {z:.3})"),
        Value::Angle(x, y, z) => format!("<{x:.3}, {y:.3}, {z:.3}>"),
        Value::Nil => "nil".into(),
        Value::Table(t) => match &dupe.arena[*t] {
            Node::Table(e) => format!("table[{}]", e.len()),
            Node::Array(a) => format!("array[{}]", a.len()),
        },
    }
}

/// Compares two dupes by entity index: which entities exist on only one side,
/// and for shared indices which scalar top-level fields differ. Nested tables
/// are compared by size only, which is enough to flag "this changed" without
/// producing a wall of text.
pub fn diff(left: &Dupe, right: &Dupe) -> Vec<Change> {
    let mut out = Vec::new();

    let index = |d: &Dupe| -> BTreeMap<u64, (String, usize)> {
        let mut m = BTreeMap::new();
        for (i, model, _) in d.list_entities() {
            if let Some(t) = transform::entity_table(d, i) {
                m.insert(i.to_bits(), (model, t));
            }
        }
        m
    };
    let l = index(left);
    let r = index(right);

    for (k, (model, _)) in &l {
        if !r.contains_key(k) {
            out.push(Change::OnlyInLeft(f64::from_bits(*k), model.clone()));
        }
    }
    for (k, (model, _)) in &r {
        if !l.contains_key(k) {
            out.push(Change::OnlyInRight(f64::from_bits(*k), model.clone()));
        }
    }

    for (k, (_, lt)) in &l {
        let Some((_, rt)) = r.get(k) else { continue };
        let (Node::Table(le), Node::Table(re)) = (&left.arena[*lt], &right.arena[*rt]) else {
            continue;
        };
        for (lk, lv) in le {
            let name = match lk {
                Value::Str(b) => String::from_utf8_lossy(b).into_owned(),
                other => show(left, other),
            };
            let rv = re.iter().find(|(rk, _)| key_matches(rk, &name)).map(|(_, v)| v);
            let differs = match (lv, rv) {
                (Value::Table(a), Some(Value::Table(b))) => {
                    let size = |d: &Dupe, t: usize| match &d.arena[t] {
                        Node::Table(e) => e.len(),
                        Node::Array(a) => a.len(),
                    };
                    size(left, *a) != size(right, *b)
                }
                (a, Some(b)) => show(left, a) != show(right, b),
                (_, None) => true,
            };
            if differs {
                out.push(Change::Field(
                    f64::from_bits(*k),
                    name,
                    show(left, lv),
                    rv.map(|v| show(right, v)).unwrap_or_else(|| "(absent)".into()),
                ));
            }
        }
    }

    let count = |d: &Dupe| -> usize {
        d.root_table()
            .ok()
            .and_then(|r| d.get_table(r, "Constraints"))
            .map(|c| match &d.arena[c] {
                Node::Array(a) => a.len(),
                Node::Table(e) => e.len(),
            })
            .unwrap_or(0)
    };
    let (lc, rc) = (count(left), count(right));
    if lc != rc {
        out.push(Change::ConstraintCount(lc, rc));
    }
    // Keep the borrow checker's view of table_index honest for callers that
    // want it; not needed here but the import keeps this module self-contained.
    let _ = table_index;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_world_weld_round_trips_and_counts() {
        // In-memory fixture: a test that returns early when a file is
        // missing passes without testing anything.
        let mut d = crate::fixture::fixture();
        let first = d.list_entities()[0].0;
        let before = diff(&d, &d);
        assert!(before.is_empty(), "a dupe must diff clean against itself");

        let original = Dupe {
            root: d.root.clone(),
            arena: d.arena.clone(),
        };
        weld_to_world(&mut d, first).expect("weld");
        let after = diff(&original, &d);
        assert!(
            after.iter().any(|c| matches!(c, Change::ConstraintCount(a, b) if b == &(a + 1))),
            "{after:?}"
        );

        // And the scanner sees it as a world endpoint, not a dangling one.
        let dangling = crate::refs::dangling(&d);
        assert!(dangling.is_empty(), "{dangling:?}");

        // Encode/decode still works with the new table in place.
        let body = d.to_body().expect("encode");
        let (again, _) = Dupe::from_body(&body).expect("re-decode");
        assert!(diff(&d, &again).is_empty());
    }
}
