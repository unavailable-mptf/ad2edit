//! The node view's model: which entities are nodes, which ACF links and
//! wires join them, a first arrangement, and the sidecar that remembers
//! where the builder dragged each node. An edge always leaves the entity
//! that stores it: a link leaves the owner of the array in `refs.rs`, a
//! wire leaves its source.

use crate::dupe::Dupe;
use crate::ports::{self, Ports};
use crate::refs::ACF_LINKS;
use crate::transform;
use crate::value::{table_index, Node, Value};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct GraphNode {
    pub entity: f64,
    pub class: String,
    /// What the builder would call it: the class without its addon prefix,
    /// and the item or chip name when there is one.
    pub title: String,
    pub ports: Ports,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Edge {
    Link { owner: f64, target: f64, modifier: String },
    Wire { source: f64, output: String, target: f64, input: String },
}

impl Edge {
    pub fn ends(&self) -> (f64, f64) {
        match self {
            Edge::Link { owner, target, .. } => (*owner, *target),
            Edge::Wire { source, target, .. } => (*source, *target),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
}

fn text(dupe: &Dupe, table: usize, key: &str) -> String {
    match dupe.get(table, key) {
        Some(Value::Str(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
        _ => String::new(),
    }
}

fn numbers(dupe: &Dupe, table: usize) -> Vec<f64> {
    let number = |value: &Value| if let Value::Number(n) = value { Some(*n) } else { None };
    match &dupe.arena[table] {
        Node::Array(items) => items.iter().filter_map(number).collect(),
        Node::Table(entries) => entries.iter().filter_map(|(_, value)| number(value)).collect(),
    }
}

fn title(dupe: &Dupe, et: usize, class: &str) -> String {
    let kind = class.trim_start_matches("acf_").trim_start_matches("gmod_wire_").replace('_', " ");
    let named = ["Weapon", "Engine", "Gearbox", "Turret", "Motor", "FuelTank", "AmmoType", "CrewTypeID", "_name", "action"]
        .iter()
        .map(|key| text(dupe, et, key))
        .find(|name| !name.is_empty());
    match named {
        Some(name) => format!("{kind} · {name}"),
        None => kind,
    }
}

/// Every link and wire in the dupe, and as nodes the entities that have
/// ports, carry an edge, or are pinned. Props with nothing to connect stay
/// out, or thirty armour plates would bury the drivetrain.
pub fn of_dupe(dupe: &Dupe, pinned: &[f64]) -> Graph {
    let entities = dupe.list_entities();
    let live: BTreeSet<u64> = entities.iter().map(|(index, _, _)| index.to_bits()).collect();
    let mut edges = Vec::new();
    for (index, _, _) in &entities {
        let Some(mods) = transform::entity_table(dupe, *index).and_then(|et| dupe.get_table(et, "EntityMods")) else {
            continue;
        };
        for modifier in ACF_LINKS {
            let Some(list) = dupe.get_table(mods, modifier) else { continue };
            for target in numbers(dupe, list) {
                if live.contains(&target.to_bits()) {
                    edges.push(Edge::Link { owner: *index, target, modifier: (*modifier).to_owned() });
                }
            }
        }
        let wires = dupe.get_table(mods, "WireDupeInfo").and_then(|info| dupe.get_table(info, "Wires"));
        if let Some(Node::Table(entries)) = wires.map(|wires| &dupe.arena[wires]) {
            for (key, value) in entries {
                let (Value::Str(input), Some(wire)) = (key, table_index(value)) else { continue };
                let Some(source) = dupe.get_number(wire, "Src") else { continue };
                if live.contains(&source.to_bits()) {
                    edges.push(Edge::Wire {
                        source,
                        output: text(dupe, wire, "SrcId"),
                        target: *index,
                        input: String::from_utf8_lossy(input).into_owned(),
                    });
                }
            }
        }
    }
    let joined: BTreeSet<u64> = edges.iter().flat_map(|edge| [edge.ends().0.to_bits(), edge.ends().1.to_bits()]).collect();
    let mut nodes = Vec::new();
    for (index, _, _) in &entities {
        let Some(et) = transform::entity_table(dupe, *index) else { continue };
        let class = text(dupe, et, "Class").to_ascii_lowercase();
        let found = ports::of_entity(dupe, *index);
        let wanted = found.is_some() || class.starts_with("acf_") || joined.contains(&index.to_bits()) || pinned.contains(index);
        if wanted {
            nodes.push(GraphNode { entity: *index, title: title(dupe, et, &class), class, ports: found.unwrap_or_default() });
        }
    }
    Graph { nodes, edges }
}

/// Where each node goes the first time a dupe is opened: columns by how far
/// down the chain of edges a node sits, so power and signals read left to
/// right, and rows that leave room for each node's ports.
pub fn arrange(graph: &Graph) -> BTreeMap<u64, (f32, f32)> {
    let mut column: BTreeMap<u64, usize> = graph.nodes.iter().map(|node| (node.entity.to_bits(), 0)).collect();
    for _ in 0..graph.nodes.len().min(12) {
        let mut moved = false;
        for edge in &graph.edges {
            let (from, to) = edge.ends();
            let (Some(&a), Some(&b)) = (column.get(&from.to_bits()), column.get(&to.to_bits())) else { continue };
            if from != to && b <= a && a + 1 < 12 {
                column.insert(to.to_bits(), a + 1);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    let mut next_row: BTreeMap<usize, f32> = BTreeMap::new();
    let mut placed = BTreeMap::new();
    for node in &graph.nodes {
        let at = column[&node.entity.to_bits()];
        let top = next_row.entry(at).or_insert(0.0);
        placed.insert(node.entity.to_bits(), (at as f32 * COLUMN_WIDTH, *top));
        *top += node_height(node) + ROW_GAP;
    }
    placed
}

pub const COLUMN_WIDTH: f32 = 300.0;
pub const NODE_WIDTH: f32 = 220.0;
pub const TITLE_HEIGHT: f32 = 26.0;
pub const PORT_HEIGHT: f32 = 18.0;
const ROW_GAP: f32 = 22.0;

/// A node is its title, a row for the link socket, and a row per port on
/// its longer side.
pub fn node_height(node: &GraphNode) -> f32 {
    TITLE_HEIGHT + PORT_HEIGHT * (1 + node.ports.inputs.len().max(node.ports.outputs.len())) as f32 + 6.0
}

/// Takes the wire off an input. True when there was one.
pub fn remove_wire(dupe: &mut Dupe, target: f64, input: &str) -> bool {
    let wires = transform::entity_table(dupe, target)
        .and_then(|et| dupe.get_table(et, "EntityMods"))
        .and_then(|mods| dupe.get_table(mods, "WireDupeInfo"))
        .and_then(|info| dupe.get_table(info, "Wires"));
    let Some(Node::Table(entries)) = wires.map(|wires| &mut dupe.arena[wires]) else { return false };
    let before = entries.len();
    entries.retain(|(key, _)| !matches!(key, Value::Str(name) if name.as_slice() == input.as_bytes()));
    entries.len() != before
}

/// Takes one target out of one of the owner's link arrays. True when it
/// was there. A keyed list is renumbered so ACF still reads it as a list.
pub fn remove_link(dupe: &mut Dupe, owner: f64, target: f64, modifier: &str) -> bool {
    let list = transform::entity_table(dupe, owner)
        .and_then(|et| dupe.get_table(et, "EntityMods"))
        .and_then(|mods| dupe.get_table(mods, modifier));
    let Some(list) = list else { return false };
    let is_target = |value: &Value| matches!(value, Value::Number(n) if *n == target);
    match &mut dupe.arena[list] {
        Node::Array(items) => {
            let before = items.len();
            items.retain(|value| !is_target(value));
            items.len() != before
        }
        Node::Table(entries) => {
            let before = entries.len();
            entries.retain(|(_, value)| !is_target(value));
            for (position, (key, _)) in entries.iter_mut().enumerate() {
                if matches!(key, Value::Number(_)) {
                    *key = Value::Number(position as f64 + 1.0);
                }
            }
            entries.len() != before
        }
    }
}

/// Where the builder left each node, kept beside the dupe so a dupe with no
/// project still has a layout. Keys are entity indices.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NodeLayout {
    #[serde(default)]
    pub positions: BTreeMap<String, (f32, f32)>,
}

impl NodeLayout {
    pub fn path_for(dupe_file: &Path) -> PathBuf {
        let mut name = dupe_file.file_name().map(|name| name.to_os_string()).unwrap_or_default();
        name.push(".nodes.toml");
        dupe_file.with_file_name(name)
    }

    /// A missing or unreadable sidecar is an empty layout, never an error:
    /// the view arranges the nodes itself.
    pub fn load(dupe_file: &Path) -> NodeLayout {
        std::fs::read_to_string(Self::path_for(dupe_file)).ok().and_then(|text| toml::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, dupe_file: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(Self::path_for(dupe_file), text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build;

    fn entity(d: &mut Dupe, index: f64, class: &str, fields: &[(&str, Value)]) {
        let root = d.root_table().unwrap();
        let ents = d.get_table(root, "Entities").unwrap();
        let et = d.new_table();
        d.set(et, "Class", Value::Str(class.as_bytes().to_vec()));
        for (key, value) in fields {
            d.set(et, key, value.clone());
        }
        if let Node::Table(e) = &mut d.arena[ents] {
            e.push((Value::Number(index), Value::Table(et)));
        }
    }

    fn links(d: &mut Dupe, owner: f64, modifier: &str, targets: &[f64]) {
        let et = transform::entity_table(d, owner).unwrap();
        let mods = match d.get_table(et, "EntityMods") {
            Some(mods) => mods,
            None => {
                let mods = d.new_table();
                d.set(et, "EntityMods", Value::Table(mods));
                mods
            }
        };
        let list = d.new_array();
        if let Node::Array(items) = &mut d.arena[list] {
            items.extend(targets.iter().map(|target| Value::Number(*target)));
        }
        d.set(mods, modifier, Value::Table(list));
    }

    fn tank() -> Dupe {
        let mut d = crate::buildfile::empty_dupe();
        entity(&mut d, 1.0, "acf_engine", &[("Engine", Value::Str(b"6.5-I6".to_vec()))]);
        entity(&mut d, 2.0, "acf_gearbox", &[("Gearbox", Value::Str(b"CVT-T".to_vec()))]);
        entity(&mut d, 3.0, "prop_physics", &[]);
        entity(&mut d, 4.0, "prop_physics", &[]);
        entity(&mut d, 5.0, "gmod_wire_pod", &[]);
        links(&mut d, 1.0, "ACFGearboxes", &[2.0]);
        links(&mut d, 2.0, "ACFWheels", &[3.0]);
        build::add_wire(&mut d, 5.0, "W", 1.0, "Throttle").unwrap();
        d
    }

    #[test]
    fn nodes_are_what_can_be_connected_and_edges_leave_their_owner() {
        let graph = of_dupe(&tank(), &[]);
        let shown: Vec<f64> = graph.nodes.iter().map(|node| node.entity).collect();
        assert_eq!(shown, [1.0, 2.0, 3.0, 5.0], "the unlinked prop stays out");
        assert!(graph.edges.contains(&Edge::Link { owner: 1.0, target: 2.0, modifier: "ACFGearboxes".into() }));
        assert!(graph.edges.contains(&Edge::Link { owner: 2.0, target: 3.0, modifier: "ACFWheels".into() }));
        assert!(graph.edges.contains(&Edge::Wire { source: 5.0, output: "W".into(), target: 1.0, input: "Throttle".into() }));
        assert_eq!(graph.nodes[0].title, "engine · 6.5-I6");
        assert!(graph.nodes[1].ports.input("CVT Ratio").is_some());

        let pinned = of_dupe(&tank(), &[4.0]);
        assert!(pinned.nodes.iter().any(|node| node.entity == 4.0));
    }

    #[test]
    fn the_first_arrangement_runs_left_to_right_and_never_overlaps() {
        let graph = of_dupe(&tank(), &[]);
        let placed = arrange(&graph);
        let x = |entity: f64| placed[&entity.to_bits()].0;
        assert!(x(5.0) < x(1.0) && x(1.0) < x(2.0) && x(2.0) < x(3.0));
        for a in &graph.nodes {
            for b in &graph.nodes {
                let (pa, pb) = (placed[&a.entity.to_bits()], placed[&b.entity.to_bits()]);
                if a.entity != b.entity && pa.0 == pb.0 {
                    let (top, bottom) = if pa.1 < pb.1 { (a, pb.1) } else { (b, pa.1) };
                    assert!(pa.1.min(pb.1) + node_height(top) <= bottom);
                }
            }
        }
    }

    #[test]
    fn a_loop_of_links_still_arranges() {
        let mut d = tank();
        links(&mut d, 3.0, "ACFGearboxes", &[1.0]);
        let graph = of_dupe(&d, &[]);
        assert_eq!(arrange(&graph).len(), graph.nodes.len());
    }

    #[test]
    fn a_wire_and_a_link_can_be_taken_off() {
        let mut d = tank();
        assert!(remove_wire(&mut d, 1.0, "Throttle"));
        assert!(!remove_wire(&mut d, 1.0, "Throttle"));
        assert!(remove_link(&mut d, 2.0, 3.0, "ACFWheels"));
        assert!(!remove_link(&mut d, 2.0, 3.0, "ACFWheels"));
        let graph = of_dupe(&d, &[]);
        assert_eq!(graph.edges, [Edge::Link { owner: 1.0, target: 2.0, modifier: "ACFGearboxes".into() }]);
    }

    #[test]
    fn the_layout_sits_beside_the_dupe_and_a_missing_one_is_empty() {
        let dir = std::env::temp_dir().join(format!("ad2read-nodes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tank.txt");
        assert_eq!(NodeLayout::load(&file), NodeLayout::default());
        let mut layout = NodeLayout::default();
        layout.positions.insert("12".into(), (300.0, 44.5));
        layout.save(&file).unwrap();
        assert!(dir.join("tank.txt.nodes.toml").exists());
        assert_eq!(NodeLayout::load(&file), layout);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
