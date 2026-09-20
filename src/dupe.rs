//! A decoded dupe plus the operations that edit it.
//!
//! Every edit works on the arena. Table numbering is regenerated at encode
//! time, so nothing here has to think about back-reference indices.

use crate::codec;
use crate::refs::{self, PruneReport};
use crate::value::{key_matches, short, table_index, Node, Value};

pub struct Dupe {
    pub root: Value,
    pub arena: Vec<Node>,
}

#[derive(Debug)]
pub struct DeleteReport {
    pub removed_constraints: usize,
    pub head_repointed: Option<f64>,
    pub pruned: PruneReport,
}

impl Dupe {
    pub fn from_body(body: &[u8]) -> Result<(Dupe, usize), String> {
        let d = codec::decode(body)?;
        Ok((
            Dupe {
                root: d.root,
                arena: d.arena,
            },
            d.consumed,
        ))
    }

    pub fn to_body(&self) -> Result<Vec<u8>, String> {
        codec::encode(&self.root, &self.arena)
    }

    // --- reading --------------------------------------------------------

    pub fn get(&self, table: usize, key: &str) -> Option<&Value> {
        match &self.arena[table] {
            Node::Table(entries) => entries
                .iter()
                .find(|(k, _)| key_matches(k, key))
                .map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_table(&self, table: usize, key: &str) -> Option<usize> {
        self.get(table, key).and_then(table_index)
    }

    pub fn get_number(&self, table: usize, key: &str) -> Option<f64> {
        match self.get(table, key) {
            Some(Value::Number(n)) => Some(*n),
            _ => None,
        }
    }

    pub fn root_table(&self) -> Result<usize, String> {
        table_index(&self.root).ok_or_else(|| "root is not a table".to_string())
    }

    // --- writing --------------------------------------------------------

    pub fn set(&mut self, table: usize, key: &str, value: Value) {
        let Node::Table(entries) = &mut self.arena[table] else {
            return;
        };
        // Find the slot first, then assign. Assigning inside the search loop
        // would move `value` in a loop body, which the borrow checker
        // rejects even though we'd return immediately after.
        let pos = entries.iter().position(|(k, _)| key_matches(k, key));
        match pos {
            Some(i) => entries[i].1 = value,
            None => entries.push((Value::Str(key.as_bytes().to_vec()), value)),
        }
    }

    // These are the building blocks for duplicate-entity and merge-of-a-set,
    // which aren't wired up yet. Kept because the next operations need them.
    #[allow(dead_code)]
    pub fn new_table(&mut self) -> usize {
        self.arena.push(Node::Table(Vec::new()));
        self.arena.len() - 1
    }

    #[allow(dead_code)]
    pub fn new_array(&mut self) -> usize {
        self.arena.push(Node::Array(Vec::new()));
        self.arena.len() - 1
    }

    /// Deep-copies a table and everything under it. The memo means a table
    /// shared by two branches of the source stays shared in the copy, rather
    /// than becoming two independent tables.
    #[allow(dead_code)]
    pub fn deep_copy(&mut self, src: usize) -> usize {
        let mut memo: Vec<Option<usize>> = vec![None; self.arena.len()];
        self.deep_copy_inner(src, &mut memo)
    }

    #[allow(dead_code)]
    fn deep_copy_inner(&mut self, src: usize, memo: &mut Vec<Option<usize>>) -> usize {
        if let Some(existing) = memo.get(src).copied().flatten() {
            return existing;
        }
        let dst = match &self.arena[src] {
            Node::Table(_) => self.new_table(),
            Node::Array(_) => self.new_array(),
        };
        if src < memo.len() {
            memo[src] = Some(dst);
        }

        // Clone the source contents out first so the arena isn't borrowed
        // while we recurse into it.
        let source = self.arena[src].clone();
        let copied = match source {
            Node::Table(entries) => {
                let mut out = Vec::with_capacity(entries.len());
                for (k, v) in entries {
                    let k = self.copy_value(k, memo);
                    let v = self.copy_value(v, memo);
                    out.push((k, v));
                }
                Node::Table(out)
            }
            Node::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for v in items {
                    out.push(self.copy_value(v, memo));
                }
                Node::Array(out)
            }
        };
        self.arena[dst] = copied;
        dst
    }

    #[allow(dead_code)]
    fn copy_value(&mut self, v: Value, memo: &mut Vec<Option<usize>>) -> Value {
        match v {
            Value::Table(i) => Value::Table(self.deep_copy_inner(i, memo)),
            other => other,
        }
    }

    // --- entity operations ----------------------------------------------

    pub fn list_entities(&self) -> Vec<(f64, String, String)> {
        let Ok(root) = self.root_table() else {
            return Vec::new();
        };
        let Some(ents) = self.get_table(root, "Entities") else {
            return Vec::new();
        };
        let Node::Table(entries) = &self.arena[ents] else {
            return Vec::new();
        };

        entries
            .iter()
            .filter_map(|(k, v)| {
                let Value::Number(n) = k else { return None };
                let t = table_index(v)?;
                let class = self.get(t, "Class").map(short).unwrap_or_else(|| "?".into());
                let model = self.get(t, "Model").map(short).unwrap_or_else(|| "?".into());
                Some((*n, class, model))
            })
            .collect()
    }

    /// Does this constraint name the given entity index in its Entity list?
    fn constraint_touches(&self, constraint: &Value, target: f64) -> bool {
        let Some(ct) = table_index(constraint) else {
            return false;
        };
        // A WireHydraulic without its controller is dead too.
        if self.get_number(ct, "MyCrtl") == Some(target) {
            return true;
        }
        let Some(list) = self.get_table(ct, "Entity") else {
            return false;
        };
        let Node::Array(items) = &self.arena[list] else {
            return false;
        };
        items.iter().any(|item| {
            table_index(item)
                .and_then(|t| self.get_number(t, "Index"))
                .map(|n| n == target)
                .unwrap_or(false)
        })
    }

    pub fn delete_entity(&mut self, target: f64) -> Result<DeleteReport, String> {
        let root = self.root_table()?;
        let ents = self
            .get_table(root, "Entities")
            .ok_or("no Entities table in this dupe")?;

        // 1. Drop the entity itself.
        let pos = match &self.arena[ents] {
            Node::Table(entries) => entries
                .iter()
                .position(|(k, _)| matches!(k, Value::Number(n) if *n == target)),
            _ => return Err("Entities is not a keyed table".into()),
        };
        let pos = pos.ok_or_else(|| format!("no entity with index {target}"))?;
        if let Node::Table(entries) = &mut self.arena[ents] {
            entries.remove(pos);
        }

        // 2. Drop constraints that reference it. A constraint pointing at a
        //    missing entity is what actually breaks a paste.
        let mut removed_constraints = 0;
        if let Some(cons) = self.get_table(root, "Constraints") {
            let doomed: Vec<usize> = match &self.arena[cons] {
                Node::Array(items) => items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| self.constraint_touches(item, target))
                    .map(|(i, _)| i)
                    .collect(),
                _ => Vec::new(),
            };
            removed_constraints = doomed.len();
            if let Node::Array(items) = &mut self.arena[cons] {
                // Back to front, so earlier positions stay valid.
                for &i in doomed.iter().rev() {
                    items.remove(i);
                }
            }
        }

        // 3. Everything else that names it: _links, ACF link arrays, wire
        //    ports and waypoints, parenting.
        let pruned = refs::prune_entity(self, target);

        // 4. HeadEnt is the paste anchor. If it named the entity we just
        //    deleted, point it at a survivor.
        let mut head_repointed = None;
        if let Some(head) = self.get_table(root, "HeadEnt") {
            if self.get_number(head, "Index") == Some(target) {
                let survivor = match &self.arena[ents] {
                    Node::Table(entries) => entries.iter().find_map(|(k, _)| match k {
                        Value::Number(n) => Some(*n),
                        _ => None,
                    }),
                    _ => None,
                };
                let survivor =
                    survivor.ok_or("that was the last entity; nothing left to anchor HeadEnt to")?;
                self.set(head, "Index", Value::Number(survivor));
                head_repointed = Some(survivor);
            }
        }

        Ok(DeleteReport {
            removed_constraints,
            head_repointed,
            pruned,
        })
    }
}
