//! Merging one dupe into another.
//!
//! Three problems have to be solved in order:
//!
//! 1. Arena collision. Both dupes number their tables from zero, so B's
//!    Value::Table(3) and A's Value::Table(3) mean different things. B's
//!    indices get shifted by A's arena length before splicing.
//! 2. Entity index collision. Both dupes will have an entity 155. B's
//!    entities get remapped into a range that's free in A, using the same
//!    machinery that powers --remap.
//! 3. Overlap. Both builds sit around their own dupe origin, so without an
//!    offset they land inside each other.
//!
//! The offset is a uniform translation, which needs no constraint resync —
//! see the note on transform::translate.

use crate::dupe::Dupe;
use crate::refs;
use crate::transform::{self, Vec3};
use crate::value::{table_index, Node, Value};

#[derive(Debug)]
pub struct MergeReport {
    pub entities_added: usize,
    pub constraints_added: usize,
    pub remapped: usize,
    pub moved: usize,
    pub first_new_index: f64,
}

fn shift_value(v: &mut Value, by: usize) {
    if let Value::Table(i) = v {
        *i += by;
    }
}

fn shift_node(node: &mut Node, by: usize) {
    match node {
        Node::Table(entries) => {
            for (k, v) in entries.iter_mut() {
                shift_value(k, by);
                shift_value(v, by);
            }
        }
        Node::Array(items) => {
            for v in items.iter_mut() {
                shift_value(v, by);
            }
        }
    }
}

pub fn merge(base: &mut Dupe, mut other: Dupe, offset: Vec3) -> Result<MergeReport, String> {
    let base_indices: Vec<f64> = base.list_entities().into_iter().map(|(i, _, _)| i).collect();
    let other_indices: Vec<f64> = other
        .list_entities()
        .into_iter()
        .map(|(i, _, _)| i)
        .collect();
    if other_indices.is_empty() {
        return Err("the incoming dupe has no entities".into());
    }

    // --- 1. renumber the incoming entities into a free range -------------
    // remap() scans first and then rewrites, so every hit carries its
    // ORIGINAL value. The whole mapping is applied as one simultaneous
    // permutation, so overlapping ranges can't corrupt it.
    let mut next = base_indices.iter().cloned().fold(0.0_f64, f64::max) + 1.0;
    let first_new_index = next;
    let mapping: Vec<(f64, f64)> = other_indices
        .iter()
        .map(|&old| {
            let pair = (old, next);
            next += 1.0;
            pair
        })
        .collect();
    let remapped = refs::remap(&mut other, &mapping);

    // --- 2. move it out of the way ---------------------------------------
    let moved = transform::translate(&mut other, offset);

    // --- 3. splice the arenas --------------------------------------------
    let shift = base.arena.len();
    for node in other.arena.iter_mut() {
        shift_node(node, shift);
    }
    let mut other_root_value = other.root.clone();
    shift_value(&mut other_root_value, shift);
    base.arena.extend(other.arena);

    let base_root = base.root_table()?;
    let other_root = table_index(&other_root_value)
        .ok_or_else(|| "the incoming dupe's root is not a table".to_string())?;

    // --- 4. concatenate Entities -----------------------------------------
    let base_ents = base
        .get_table(base_root, "Entities")
        .ok_or("base dupe has no Entities table")?;
    let other_ents = base
        .get_table(other_root, "Entities")
        .ok_or("incoming dupe has no Entities table")?;

    let incoming: Vec<(Value, Value)> = match &base.arena[other_ents] {
        Node::Table(entries) => entries.clone(),
        _ => return Err("incoming Entities is not a keyed table".into()),
    };
    let entities_added = incoming.len();
    if let Node::Table(entries) = &mut base.arena[base_ents] {
        entries.extend(incoming);
    }

    // --- 5. concatenate Constraints --------------------------------------
    let mut constraints_added = 0;
    if let (Some(bc), Some(oc)) = (
        base.get_table(base_root, "Constraints"),
        base.get_table(other_root, "Constraints"),
    ) {
        let incoming: Vec<Value> = match &base.arena[oc] {
            Node::Array(items) => items.clone(),
            _ => Vec::new(),
        };
        constraints_added = incoming.len();
        if let Node::Array(items) = &mut base.arena[bc] {
            items.extend(incoming);
        }
    }

    // The incoming dupe's own root table and HeadEnt are now unreachable from
    // base's root. They cost nothing: the encoder only walks what it can
    // reach, so they're simply never written. Another consequence of deriving
    // table numbers at encode time instead of storing them.

    Ok(MergeReport {
        entities_added,
        constraints_added,
        remapped,
        moved,
        first_new_index,
    })
}
