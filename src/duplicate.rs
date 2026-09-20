//! Duplicating a set of entities.
//!
//! The trick here is that duplicating a set is the same problem as merging,
//! so it reuses merge wholesale. Extract the selection into a throwaway dupe,
//! then merge that back into the original. Renumbering into a free range,
//! the offset, and the arena splice all come for free and are already tested.
//!
//! Which constraints come along is decided by containment: a constraint whose
//! endpoints are ALL inside the selection is copied, one that reaches outside
//! it is dropped. Duplicate a single prop and it arrives loose; duplicate a
//! whole turret and its internal welds arrive with it. No flag needed — the
//! selection expresses the intent.

use crate::dupe::Dupe;
use crate::merge::{self, MergeReport};
use crate::refs;
use crate::transform::Vec3;
use crate::value::{table_index, Node, Value};

/// Every entity index a constraint names.
fn constraint_endpoints(dupe: &Dupe, constraint: &Value) -> Vec<f64> {
    let Some(ct) = table_index(constraint) else {
        return Vec::new();
    };
    let Some(list) = dupe.get_table(ct, "Entity") else {
        return Vec::new();
    };
    let Node::Array(items) = &dupe.arena[list] else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|it| dupe.get_number(table_index(it)?, "Index"))
        .collect()
}

/// Builds a standalone dupe containing only the selected entities and the
/// constraints internal to them.
///
/// The arena is cloned rather than deep-copied entry by entity: unreachable
/// tables cost nothing because the encoder only walks what the root can
/// reach, so a filtered root is all it takes to "extract" a subset. Mutating
/// the clone can't affect the original because Node clones its contents.
pub fn extract(dupe: &Dupe, indices: &[f64]) -> Result<Dupe, String> {
    let mut sub = Dupe {
        root: dupe.root.clone(),
        arena: dupe.arena.clone(),
    };

    let root = sub.root_table()?;
    let ents = sub
        .get_table(root, "Entities")
        .ok_or("no Entities table in this dupe")?;

    // --- the selected entities -------------------------------------------
    let kept: Vec<(Value, Value)> = match &sub.arena[ents] {
        Node::Table(entries) => entries
            .iter()
            .filter(|(k, _)| matches!(k, Value::Number(n) if indices.contains(n)))
            .cloned()
            .collect(),
        _ => return Err("Entities is not a keyed table".into()),
    };
    if kept.len() != indices.len() {
        let found: Vec<f64> = kept
            .iter()
            .filter_map(|(k, _)| match k {
                Value::Number(n) => Some(*n),
                _ => None,
            })
            .collect();
        let missing: Vec<f64> = indices
            .iter()
            .filter(|i| !found.contains(i))
            .cloned()
            .collect();
        return Err(format!("no entity with index {missing:?}"));
    }

    // --- constraints wholly inside the selection --------------------------
    let mut kept_cons: Vec<Value> = Vec::new();
    if let Some(cons) = sub.get_table(root, "Constraints") {
        let items: Vec<Value> = match &sub.arena[cons] {
            Node::Array(items) => items.clone(),
            _ => Vec::new(),
        };
        for item in items {
            let eps = constraint_endpoints(&sub, &item);
            if !eps.is_empty() && eps.iter().all(|e| indices.contains(e)) {
                kept_cons.push(item);
            }
        }
    }

    // Original HeadEnt's Pos/Z are worth carrying, but its Index has to name
    // something in the selection.
    let mut head_entries: Vec<(Value, Value)> = Vec::new();
    if let Some(head) = sub.get_table(root, "HeadEnt") {
        if let Node::Table(entries) = &sub.arena[head] {
            head_entries = entries.clone();
        }
    }

    let new_ents = sub.new_table();
    sub.arena[new_ents] = Node::Table(kept);
    let new_cons = sub.new_array();
    sub.arena[new_cons] = Node::Array(kept_cons);

    let new_head = sub.new_table();
    sub.arena[new_head] = Node::Table(head_entries);
    sub.set(new_head, "Index", Value::Number(indices[0]));

    let new_root = sub.new_table();
    sub.arena[new_root] = Node::Table(vec![
        (
            Value::Str(b"Entities".to_vec()),
            Value::Table(new_ents),
        ),
        (
            Value::Str(b"Constraints".to_vec()),
            Value::Table(new_cons),
        ),
        (Value::Str(b"HeadEnt".to_vec()), Value::Table(new_head)),
    ]);
    sub.root = Value::Table(new_root);

    // --- cut every reference that reaches outside the selection -----------
    // The copies still carry CFW _links entries, parent IDs, ACF links and
    // wire ports naming entities we didn't take. Those would claim the copy
    // is attached to things it has no constraint to. prune_entity removes
    // references to one index, so run it for everything left behind.
    let outsiders: Vec<f64> = dupe
        .list_entities()
        .into_iter()
        .map(|(i, _, _)| i)
        .filter(|i| !indices.contains(i))
        .collect();
    for outsider in outsiders {
        refs::prune_entity(&mut sub, outsider);
    }

    Ok(sub)
}

pub fn duplicate(dupe: &mut Dupe, indices: &[f64], offset: Vec3) -> Result<MergeReport, String> {
    let sub = extract(dupe, indices)?;
    merge::merge(dupe, sub, offset)
}

/// Copies of the selection reflected in a plane: build one side, get the
/// other. Anything sitting on the plane is left out, since its mirror image
/// would land on top of it. Returns the report and how many were left out.
pub fn mirror(dupe: &mut Dupe, indices: &[f64], plane_at: Vec3, plane_normal: Vec3) -> Result<(MergeReport, usize), String> {
    let length = (plane_normal.0.powi(2) + plane_normal.1.powi(2) + plane_normal.2.powi(2)).sqrt();
    if length < 1e-9 {
        return Err("the mirror plane has no direction".into());
    }
    let normal = (plane_normal.0 / length, plane_normal.1 / length, plane_normal.2 / length);
    let off_plane = |dupe: &Dupe, index: f64| {
        crate::transform::entity_transform(dupe, index).is_some_and(|(at, _)| ((at.0 - plane_at.0) * normal.0 + (at.1 - plane_at.1) * normal.1 + (at.2 - plane_at.2) * normal.2).abs() > 0.5)
    };
    let wanted: Vec<f64> = indices.iter().copied().filter(|index| off_plane(dupe, *index)).collect();
    if wanted.is_empty() {
        return Err("everything selected sits on the centreline, so its mirror image is itself".into());
    }
    let before: std::collections::HashSet<u64> = dupe.list_entities().iter().map(|(index, _, _)| index.to_bits()).collect();
    let report = duplicate(dupe, &wanted, (0.0, 0.0, 0.0))?;
    let copies: Vec<f64> = dupe.list_entities().iter().map(|(index, _, _)| *index).filter(|index| !before.contains(&index.to_bits())).collect();
    for copy in copies {
        if let Some((at, ang)) = crate::transform::entity_transform(dupe, copy) {
            let (at, ang) = crate::transform::mirrored_pose(at, ang, plane_at, normal);
            crate::transform::set_entity_transform(dupe, copy, Some(at), Some(ang))?;
        }
    }
    Ok((report, indices.len() - wanted.len()))
}

/// `copies` more of the selection, each `spacing` further on than the last:
/// a row of road wheels, a run of plates.
pub fn array(dupe: &mut Dupe, indices: &[f64], spacing: Vec3, copies: usize) -> Result<MergeReport, String> {
    if copies == 0 {
        return Err("an array of no copies".into());
    }
    let mut total: Option<MergeReport> = None;
    for n in 1..=copies {
        let by = n as f64;
        let report = duplicate(dupe, indices, (spacing.0 * by, spacing.1 * by, spacing.2 * by))?;
        total = Some(match total {
            None => report,
            Some(so_far) => MergeReport {
                entities_added: so_far.entities_added + report.entities_added,
                constraints_added: so_far.constraints_added + report.constraints_added,
                remapped: so_far.remapped + report.remapped,
                moved: so_far.moved + report.moved,
                first_new_index: so_far.first_new_index,
            },
        });
    }
    total.ok_or_else(|| "nothing was copied".to_owned())
}

#[cfg(test)]
mod mirror_tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6 && (a.2 - b.2).abs() < 1e-6
    }

    fn hull() -> (Dupe, f64) {
        let mut dupe = crate::buildfile::empty_dupe();
        let item = crate::catalog::find("GroundVehicle").unwrap();
        let base = crate::build::spawn_catalog(&mut dupe, item, (0.0, 0.0, 0.0), (0.0, 90.0, 0.0), &[("Width", 96.0), ("Length", 200.0), ("Thickness", 2.0)]).unwrap();
        crate::extras::repair_head(&mut dupe);
        (dupe, base)
    }

    #[test]
    fn a_side_plate_mirrors_to_the_other_side_facing_the_other_way() {
        let (mut dupe, _) = hull();
        let plate = crate::build::spawn_prop(&mut dupe, "models/sprops/rectangles/size_4/rect_36x144x3.mdl", (48.0, 10.0, 18.0), (0.0, 90.0, 80.0)).unwrap();
        let (report, skipped) = mirror(&mut dupe, &[plate], (0.0, 0.0, 0.0), (1.0, 0.0, 0.0)).unwrap();
        assert_eq!((report.entities_added, skipped), (1, 0));
        let copy = report.first_new_index;
        let (at, ang) = crate::transform::entity_transform(&dupe, copy).unwrap();
        assert!(close(at, (-48.0, 10.0, 18.0)), "{at:?}");
        // The original leans its top outwards to +x; the copy leans to -x.
        let lean = |ang| crate::transform::rotate_vec(ang, (0.0, 0.0, 1.0));
        let (was, now) = (lean((0.0, 90.0, 80.0)), lean(ang));
        assert!(close(now, (-was.0, was.1, was.2)), "{was:?} {now:?}");
        // And it is still a rotation, not a model turned inside out.
        let (x, y, z) = (crate::transform::rotate_vec(ang, (1.0, 0.0, 0.0)), crate::transform::rotate_vec(ang, (0.0, 1.0, 0.0)), lean(ang));
        let handed = x.0 * (y.1 * z.2 - y.2 * z.1) - x.1 * (y.0 * z.2 - y.2 * z.0) + x.2 * (y.0 * z.1 - y.1 * z.0);
        assert!((handed - 1.0).abs() < 1e-6, "{handed}");
        assert_eq!(crate::extras::validate(&dupe), Vec::<String>::new());
    }

    #[test]
    fn what_sits_on_the_centreline_is_not_mirrored_onto_itself() {
        let (mut dupe, _) = hull();
        let deck = crate::build::spawn_prop(&mut dupe, "models/sprops/rectangles/size_6/rect_96x192x3.mdl", (0.0, 0.0, 36.0), (0.0, 90.0, 0.0)).unwrap();
        let side = crate::build::spawn_prop(&mut dupe, "models/sprops/rectangles/size_4/rect_36x144x3.mdl", (48.0, 0.0, 18.0), (0.0, 90.0, 90.0)).unwrap();
        let count = dupe.list_entities().len();
        let (report, skipped) = mirror(&mut dupe, &[deck, side], (0.0, 0.0, 0.0), (1.0, 0.0, 0.0)).unwrap();
        assert_eq!((report.entities_added, skipped, dupe.list_entities().len()), (1, 1, count + 1));
        assert!(mirror(&mut dupe, &[deck], (0.0, 0.0, 0.0), (1.0, 0.0, 0.0)).is_err());
    }

    #[test]
    fn an_array_is_evenly_spaced_copies() {
        let (mut dupe, _) = hull();
        let wheel = crate::build::spawn_prop(&mut dupe, "models/sprops/trans/miscwheels/tank30.mdl", (50.0, -60.0, 0.0), (0.0, 0.0, 0.0)).unwrap();
        let report = array(&mut dupe, &[wheel], (0.0, 40.0, 0.0), 3).unwrap();
        assert_eq!(report.entities_added, 3);
        let mut along: Vec<f64> = dupe.list_entities().iter().filter(|(_, _, model)| model.contains("tank30")).filter_map(|(index, _, _)| crate::transform::entity_transform(&dupe, *index)).map(|(at, _)| at.1).collect();
        along.sort_by(f64::total_cmp);
        assert_eq!(along, [-60.0, -20.0, 20.0, 60.0]);
        assert!(array(&mut dupe, &[wheel], (0.0, 40.0, 0.0), 0).is_err());
    }
}
