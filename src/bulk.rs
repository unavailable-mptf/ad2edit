//! Bulk field edits across many entities.
//!
//! The value's type is inferred from what the key already holds, not from how
//! the text looks. "0" could be a number or a string, and writing the wrong
//! Value variant changes the bytes AD2 reads — so if a key currently holds a
//! Number we parse a number, and if it holds a string we write bytes.
//!
//! A key that isn't already present is skipped rather than added. AD2 and its
//! addons test fields for nil and behave differently when they're absent, so
//! inventing one changes how the entity pastes.
//!
//! Scope is deliberately top-level entity fields only. Reaching into nested
//! tables needs a path syntax, and that can wait until there's a UI where you
//! can see what you're reaching into.

use crate::dupe::Dupe;
use crate::value::{key_matches, table_index, Node, Value};

#[derive(Debug, Default)]
pub struct BulkReport {
    pub matched: usize,
    pub changed: usize,
    /// Entities that matched the filter but didn't have the key being set.
    pub skipped_missing: usize,
    /// Matched, had the key, but the text couldn't be parsed as its type.
    pub skipped_type: usize,
}

/// Compares a stored value against text from the command line.
/// Writes into either a keyed table or an array slot, depending on what the
/// path resolved to.
fn write_at(dupe: &mut Dupe, owner: usize, key: &str, v: Value) {
    if let Ok(n) = key.parse::<usize>() {
        if let Node::Array(items) = &mut dupe.arena[owner] {
            if let Some(slot) = items.get_mut(n) {
                *slot = v;
            }
            return;
        }
    }
    dupe.set(owner, key, v);
}

fn value_matches(v: &Value, text: &str) -> bool {
    match v {
        Value::Str(b) => b.as_slice() == text.as_bytes(),
        Value::Number(n) => text.parse::<f64>().map(|t| t == *n).unwrap_or(false),
        Value::Bool(b) => text.parse::<bool>().map(|t| t == *b).unwrap_or(false),
        _ => false,
    }
}

/// Builds a replacement value of the SAME variant as the one already stored.
fn typed_like(existing: &Value, text: &str) -> Option<Value> {
    match existing {
        Value::Str(_) => Some(Value::Str(text.as_bytes().to_vec())),
        Value::Number(_) => text.parse::<f64>().ok().map(Value::Number),
        Value::Bool(_) => text.parse::<bool>().ok().map(Value::Bool),
        _ => None,
    }
}

/// Looks up a field by path. A bare name reads a top-level entity field;
/// dots descend into nested tables and `[n]` indexes an array, so
/// `EntityMods.ACF_Armor.Thickness` and `PhysicsObjects[0].Frozen` both
/// resolve. Returns the table that holds the last segment along with the
/// value, so a write can go to the right place.
fn resolve<'a>(dupe: &'a Dupe, table: usize, path: &str) -> Option<(usize, String, &'a Value)> {
    let mut here = table;
    let segs: Vec<&str> = path.split('.').filter(|s| !s.is_empty()).collect();
    let (last, parents) = segs.split_last()?;

    for seg in parents {
        here = step(dupe, here, seg)?;
    }
    let (owner, key, v) = step_value(dupe, here, last)?;
    Some((owner, key, v))
}

/// One path segment, possibly with a trailing `[n]`. Returns the table it
/// lands on.
fn step(dupe: &Dupe, table: usize, seg: &str) -> Option<usize> {
    let (_, _, v) = step_value(dupe, table, seg)?;
    table_index(v)
}

/// One path segment, returning the table that holds it, the key inside that
/// table, and the value.
fn step_value<'a>(dupe: &'a Dupe, table: usize, seg: &str) -> Option<(usize, String, &'a Value)> {
    let (name, index) = match seg.find('[') {
        Some(b) if seg.ends_with(']') => {
            let n: usize = seg[b + 1..seg.len() - 1].parse().ok()?;
            (&seg[..b], Some(n))
        }
        _ => (seg, None),
    };

    // The named part first.
    let (owner, key, v) = if name.is_empty() {
        // A bare `[n]` indexes the current table directly.
        (table, String::new(), None)
    } else {
        match &dupe.arena[table] {
            Node::Table(entries) => {
                let (k, v) = entries.iter().find(|(k, _)| key_matches(k, name))?;
                (table, name.to_owned(), Some((k, v)))
            }
            _ => return None,
        }
    };

    match (index, v) {
        (None, Some((_, v))) => Some((owner, key, v)),
        (Some(n), Some((_, v))) => {
            let arr = table_index(v)?;
            let item = match &dupe.arena[arr] {
                Node::Array(items) => items.get(n)?,
                Node::Table(entries) => {
                    // A keyed table indexed numerically: match the key `n`.
                    entries
                        .iter()
                        .find(|(k, _)| matches!(k, Value::Number(x) if *x as usize == n))
                        .map(|(_, v)| v)?
                }
            };
            Some((arr, n.to_string(), item))
        }
        (Some(n), None) => {
            let item = match &dupe.arena[table] {
                Node::Array(items) => items.get(n)?,
                _ => return None,
            };
            Some((table, n.to_string(), item))
        }
        (None, None) => None,
    }
}

fn field<'a>(dupe: &'a Dupe, table: usize, key: &str) -> Option<&'a Value> {
    resolve(dupe, table, key).map(|(_, _, v)| v)
}

/// `filter` of None means every entity. `sets` is a list of (key, text).
pub fn bulk_set(
    dupe: &mut Dupe,
    filter: Option<(&str, &str)>,
    sets: &[(String, String)],
) -> Result<BulkReport, String> {
    let mut report = BulkReport::default();

    let root = dupe.root_table()?;
    let ents = dupe
        .get_table(root, "Entities")
        .ok_or("no Entities table in this dupe")?;

    // Collect the entity tables first so the arena isn't borrowed while we
    // write into it.
    let entity_tables: Vec<usize> = match &dupe.arena[ents] {
        Node::Table(entries) => entries.iter().filter_map(|(_, v)| table_index(v)).collect(),
        _ => return Err("Entities is not a keyed table".into()),
    };

    for et in entity_tables {
        if let Some((key, want)) = filter {
            match field(dupe, et, key) {
                Some(v) if value_matches(v, want) => {}
                _ => continue,
            }
        }
        report.matched += 1;

        // Work out every write for this entity before applying any, so the
        // immutable lookups finish before the mutable ones start.
        let mut writes: Vec<(usize, String, Value)> = Vec::new();
        for (key, text) in sets {
            match resolve(dupe, et, key) {
                None => report.skipped_missing += 1,
                Some((owner, last, existing)) => match typed_like(existing, text) {
                    Some(v) => writes.push((owner, last, v)),
                    None => report.skipped_type += 1,
                },
            }
        }

        for (owner, key, v) in writes {
            write_at(dupe, owner, &key, v);
            report.changed += 1;
        }
    }

    Ok(report)
}
