//! Undo/redo by snapshotting the whole arena.
//!
//! A decoded dupe is small — a 22KB body comes out as a few hundred tables —
//! so a full copy per edit costs almost nothing and a hundred levels of undo
//! is a couple of megabytes. That buys us out of the usual command-pattern
//! machinery entirely: no inverse operations to write, no chance of an undo
//! that doesn't quite invert its redo, and it works for any edit including
//! ones that add or remove tables.
//!
//! The CLI uses this to roll back when a post-edit check fails. The GUI will
//! use it for actual undo.

use crate::dupe::Dupe;
use crate::value::{Node, Value};

type Snapshot = (Value, Vec<Node>);

pub struct History {
    states: Vec<Snapshot>,
    /// Index of the state currently loaded. Everything after it is redo.
    cursor: usize,
    #[allow(dead_code)]
    limit: usize,
}

// The CLI only ever needs new() and undo() — one edit per invocation, rolled
// back if a check fails. push/redo/depth exist for the GUI, where edits chain.
// Tested below rather than left unexercised.

impl History {
    pub fn new(dupe: &Dupe) -> History {
        History {
            states: vec![(dupe.root.clone(), dupe.arena.clone())],
            cursor: 0,
            limit: 100,
        }
    }

    /// Records the current state as a new step. Anything that had been undone
    /// is discarded, which is what every editor does when you undo and then
    /// make a fresh edit.
    #[allow(dead_code)]
    pub fn push(&mut self, dupe: &Dupe) {
        self.states.truncate(self.cursor + 1);
        self.states.push((dupe.root.clone(), dupe.arena.clone()));
        if self.states.len() > self.limit {
            // Drop the oldest. cursor follows it down.
            self.states.remove(0);
        }
        self.cursor = self.states.len() - 1;
    }

    pub fn undo(&mut self, dupe: &mut Dupe) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        let (root, arena) = self.states[self.cursor].clone();
        dupe.root = root;
        dupe.arena = arena;
        true
    }

    #[allow(dead_code)]
    pub fn redo(&mut self, dupe: &mut Dupe) -> bool {
        if self.cursor + 1 >= self.states.len() {
            return false;
        }
        self.cursor += 1;
        let (root, arena) = self.states[self.cursor].clone();
        dupe.root = root;
        dupe.arena = arena;
        true
    }

    /// How many undo steps are available.
    #[allow(dead_code)]
    pub fn depth(&self) -> usize {
        self.cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{Node, Value};

    fn dupe_with(n: f64) -> Dupe {
        Dupe {
            root: Value::Table(0),
            arena: vec![Node::Table(vec![(
                Value::Str(b"n".to_vec()),
                Value::Number(n),
            )])],
        }
    }

    fn read(d: &Dupe) -> f64 {
        match &d.arena[0] {
            Node::Table(e) => match e[0].1 {
                Value::Number(n) => n,
                _ => panic!(),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn undo_and_redo_walk_the_stack() {
        let mut d = dupe_with(1.0);
        let mut h = History::new(&d);

        d = dupe_with(2.0);
        h.push(&d);
        d = dupe_with(3.0);
        h.push(&d);
        assert_eq!(h.depth(), 2);

        assert!(h.undo(&mut d));
        assert_eq!(read(&d), 2.0);
        assert!(h.undo(&mut d));
        assert_eq!(read(&d), 1.0);
        assert!(!h.undo(&mut d)); // nothing left

        assert!(h.redo(&mut d));
        assert_eq!(read(&d), 2.0);
    }

    #[test]
    fn a_new_edit_discards_the_redo_tail() {
        let mut d = dupe_with(1.0);
        let mut h = History::new(&d);
        d = dupe_with(2.0);
        h.push(&d);

        h.undo(&mut d);
        assert_eq!(read(&d), 1.0);

        d = dupe_with(9.0);
        h.push(&d);
        assert!(!h.redo(&mut d)); // the 2.0 branch is gone
        assert_eq!(read(&d), 9.0);
    }
}
