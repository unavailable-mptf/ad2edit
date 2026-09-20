//! Cross-module tests on a fixture built in memory.
//!
//! The fixture is a small ACF-shaped build: a baseplate, a gun linked to an
//! ammo crate and a turret, a wheel referenced by a tank track, a wired
//! button and lamp, a parented prop, and a weld. Every reference site the
//! registry knows appears at least once, so a change to any of them shows
//! up here rather than in someone's dupe.

use crate::bulk;
use crate::dupe::Dupe;
use crate::duplicate;
use crate::merge;
use crate::refs;
use crate::transform;
use crate::value::{Node, Value};

fn s(t: &str) -> Value {
    Value::Str(t.as_bytes().to_vec())
}

/// Adds an entity with one physics bone at `pos`, returning its table.
fn entity(d: &mut Dupe, ents: usize, index: f64, class: &str, model: &str, pos: (f64, f64, f64)) -> usize {
    let et = d.new_table();
    d.set(et, "Class", s(class));
    d.set(et, "Model", s(model));
    d.set(et, "CollisionGroup", Value::Number(0.0));
    let physics = d.new_table();
    let bone = d.new_table();
    d.set(bone, "Pos", Value::Vector(pos.0, pos.1, pos.2));
    d.set(bone, "Angle", Value::Angle(0.0, 0.0, 0.0));
    d.set(bone, "Frozen", Value::Bool(true));
    d.set(physics, "0", Value::Table(bone));
    // PhysicsObjects is keyed by bone NUMBER, not string, in a real dupe.
    if let Node::Table(e) = &mut d.arena[physics] {
        e[0].0 = Value::Number(0.0);
    }
    d.set(et, "PhysicsObjects", Value::Table(physics));
    let mods = d.new_table();
    d.set(et, "EntityMods", Value::Table(mods));
    if let Node::Table(e) = &mut d.arena[ents] {
        e.push((Value::Number(index), Value::Table(et)));
    }
    et
}

fn mods_of(d: &Dupe, et: usize) -> usize {
    d.get_table(et, "EntityMods").unwrap()
}

fn index_array(d: &mut Dupe, values: &[f64]) -> usize {
    let a = d.new_array();
    if let Node::Array(items) = &mut d.arena[a] {
        for v in values {
            items.push(Value::Number(*v));
        }
    }
    a
}

pub fn fixture() -> Dupe {
    let mut d = Dupe {
        root: Value::Nil,
        arena: Vec::new(),
    };
    let root = d.new_table();
    d.root = Value::Table(root);
    let ents = d.new_table();
    d.set(root, "Entities", Value::Table(ents));
    let cons = d.new_array();
    d.set(root, "Constraints", Value::Table(cons));
    let head = d.new_table();
    d.set(head, "Index", Value::Number(10.0));
    d.set(head, "Pos", Value::Vector(0.0, 0.0, 0.0));
    d.set(head, "Z", Value::Number(50.0));
    d.set(root, "HeadEnt", Value::Table(head));

    let base = entity(&mut d, ents, 10.0, "acf_baseplate", "models/cube.mdl", (0.0, 0.0, 0.0));
    let gun = entity(&mut d, ents, 11.0, "acf_gun", "models/gun.mdl", (50.0, 0.0, 20.0));
    let _ammo = entity(&mut d, ents, 12.0, "acf_ammo", "models/crate.mdl", (-50.0, 0.0, 20.0));
    let _turret = entity(&mut d, ents, 13.0, "acf_turret", "models/ring.mdl", (0.0, 0.0, 10.0));
    let _wheel = entity(&mut d, ents, 14.0, "prop_physics", "models/wheel.mdl", (0.0, 60.0, 0.0));
    let track = entity(&mut d, ents, 15.0, "sent_tanktracks_auto", "models/plate.mdl", (0.0, 70.0, 0.0));
    let _button = entity(&mut d, ents, 16.0, "gmod_wire_button", "models/button.mdl", (0.0, -60.0, 0.0));
    let lamp = entity(&mut d, ents, 17.0, "gmod_wire_light", "models/lamp.mdl", (0.0, -70.0, 0.0));
    let child = entity(&mut d, ents, 18.0, "prop_physics", "models/box.mdl", (10.0, 10.0, 30.0));

    // ACF links on the gun.
    let gm = mods_of(&d, gun);
    let crates = index_array(&mut d, &[12.0]);
    d.set(gm, "ACFCrates", Value::Table(crates));
    let turrets = index_array(&mut d, &[13.0]);
    d.set(gm, "ACFTurret", Value::Table(turrets));

    // Tank track linking the wheel.
    let tm = mods_of(&d, track);
    let tt = d.new_table();
    let links = d.new_table();
    d.set(links, "wheel_a", Value::Number(14.0));
    d.set(tt, "links", Value::Table(links));
    d.set(tm, "tanktracktool", Value::Table(tt));

    // Wire from button to lamp, stored on the lamp.
    let lm = mods_of(&d, lamp);
    let wdi = d.new_table();
    let wires = d.new_table();
    let port = d.new_table();
    d.set(port, "Src", Value::Number(16.0));
    d.set(port, "SrcId", s("Out"));
    d.set(wires, "Toggle", Value::Table(port));
    d.set(wdi, "Wires", Value::Table(wires));
    d.set(lm, "WireDupeInfo", Value::Table(wdi));

    // Parented prop.
    let bdi = d.new_table();
    d.set(bdi, "DupeParentID", Value::Number(10.0));
    d.set(child, "BuildDupeInfo", Value::Table(bdi));

    // A weld from the child to the baseplate.
    let weld = d.new_table();
    d.set(weld, "Type", s("Weld"));
    let e1 = d.new_table();
    d.set(e1, "Index", Value::Number(18.0));
    let e2 = d.new_table();
    d.set(e2, "Index", Value::Number(10.0));
    let ends = d.new_array();
    if let Node::Array(items) = &mut d.arena[ends] {
        items.push(Value::Table(e1));
        items.push(Value::Table(e2));
    }
    d.set(weld, "Entity", Value::Table(ends));
    let wb = d.new_table();
    d.set(wb, "EntityPos", Value::Vector(10.0, 10.0, 30.0));
    d.set(wb, "Ent1Pos", Value::Vector(10.0, 10.0, 30.0));
    d.set(wb, "Ent1Ang", Value::Angle(0.0, 0.0, 0.0));
    d.set(wb, "Ent2Ang", Value::Angle(0.0, 0.0, 0.0));
    d.set(weld, "BuildDupeInfo", Value::Table(wb));
    if let Node::Array(items) = &mut d.arena[cons] {
        items.push(Value::Table(weld));
    }
    let _ = base;
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6 && (a.2 - b.2).abs() < 1e-6
    }

    // ------------------------------------------------------------------
    // Angle maths, checked two ways: against Source's AngleVectors closed
    // form, and by algebraic properties a rotation must have.
    // ------------------------------------------------------------------

    /// Source's AngleVectors, transcribed from mathlib.cpp.
    fn source_angle_vectors(p: f64, y: f64, r: f64) -> [(f64, f64, f64); 3] {
        let (sp, cp) = (p.to_radians().sin(), p.to_radians().cos());
        let (sy, cy) = (y.to_radians().sin(), y.to_radians().cos());
        let (sr, cr) = (r.to_radians().sin(), r.to_radians().cos());
        let forward = (cp * cy, cp * sy, -sp);
        let right = (
            -sr * sp * cy + cr * sy,
            -sr * sp * sy - cr * cy,
            -sr * cp,
        );
        let up = (cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp);
        [forward, right, up]
    }

    #[test]
    fn rotate_vec_matches_source_anglevectors() {
        for &(p, y, r) in &[
            (0.0, 0.0, 0.0),
            (0.0, 90.0, 0.0),
            (90.0, 0.0, 0.0),
            (0.0, 0.0, 90.0),
            (30.0, -120.0, 45.0),
            (-75.0, 200.0, -160.0),
        ] {
            let [fwd, right, up] = source_angle_vectors(p, y, r);
            let a = (p, y, r);
            assert!(close(transform::rotate_vec(a, (1.0, 0.0, 0.0)), fwd), "forward at {a:?}");
            // Rotating +Y gives LEFT, which is -right.
            let left = (-right.0, -right.1, -right.2);
            assert!(close(transform::rotate_vec(a, (0.0, 1.0, 0.0)), left), "left at {a:?}");
            assert!(close(transform::rotate_vec(a, (0.0, 0.0, 1.0)), up), "up at {a:?}");
        }
    }

    #[test]
    fn pitch_down_is_positive_and_yaw_is_counterclockwise() {
        // Source: +pitch looks DOWN, +yaw turns LEFT (counter-clockwise from above).
        let down = transform::rotate_vec((90.0, 0.0, 0.0), (1.0, 0.0, 0.0));
        assert!(close(down, (0.0, 0.0, -1.0)), "{down:?}");
        let left = transform::rotate_vec((0.0, 90.0, 0.0), (1.0, 0.0, 0.0));
        assert!(close(left, (0.0, 1.0, 0.0)), "{left:?}");
    }

    #[test]
    fn gimbal_lock_round_trips_by_forward_vector() {
        // At pitch ±90 yaw and roll are the same axis, so the angles need
        // not come back identical — but the orientation must.
        for &a in &[(90.0, 33.0, 0.0), (-90.0, -120.0, 0.0), (90.0, 0.0, 45.0)] {
            let m = transform::rotate_angle_about(a, (0.0, 0.0, 1.0), 0.0);
            for axis in [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)] {
                let v1 = transform::rotate_vec(a, axis);
                let v2 = transform::rotate_vec(m, axis);
                assert!(close(v1, v2), "{a:?} -> {m:?} on {axis:?}: {v1:?} vs {v2:?}");
            }
        }
    }

    #[test]
    fn rotation_is_orthonormal_with_unit_determinant() {
        for &a in &[(12.0, 34.0, 56.0), (-80.0, 170.0, -100.0), (89.9, 0.0, 0.0)] {
            let x = transform::rotate_vec(a, (1.0, 0.0, 0.0));
            let y = transform::rotate_vec(a, (0.0, 1.0, 0.0));
            let z = transform::rotate_vec(a, (0.0, 0.0, 1.0));
            let dot = |u: (f64, f64, f64), v: (f64, f64, f64)| u.0 * v.0 + u.1 * v.1 + u.2 * v.2;
            let len = |u: (f64, f64, f64)| dot(u, u).sqrt();
            assert!((len(x) - 1.0).abs() < 1e-9 && (len(y) - 1.0).abs() < 1e-9 && (len(z) - 1.0).abs() < 1e-9);
            assert!(dot(x, y).abs() < 1e-9 && dot(y, z).abs() < 1e-9 && dot(x, z).abs() < 1e-9);
            // det = x · (y × z) must be +1: a rotation, not a reflection.
            let cross = (y.1 * z.2 - y.2 * z.1, y.2 * z.0 - y.0 * z.2, y.0 * z.1 - y.1 * z.0);
            assert!((dot(x, cross) - 1.0).abs() < 1e-9, "determinant at {a:?}");
        }
    }

    #[test]
    fn a_turn_and_its_reverse_cancel_for_every_axis() {
        let a = (20.0, -40.0, 60.0);
        for axis in [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0), (0.577, 0.577, 0.577)] {
            let there = transform::rotate_angle_about(a, axis, 1.1);
            let back = transform::rotate_angle_about(there, axis, -1.1);
            let fa = transform::rotate_vec(a, (1.0, 0.0, 0.0));
            let fb = transform::rotate_vec(back, (1.0, 0.0, 0.0));
            assert!(close(fa, fb), "{axis:?}");
        }
    }

    // ------------------------------------------------------------------
    // Reference registry against the fixture.
    // ------------------------------------------------------------------

    #[test]
    fn the_fixture_passes_ad2s_own_validation_and_a_broken_head_is_repaired() {
        let d = fixture();
        assert!(crate::extras::validate(&d).is_empty(), "{:?}", crate::extras::validate(&d));

        // Strip HeadEnt entirely, as a hand-built dupe might, and confirm
        // validate catches it and repair_head makes it pass again.
        let mut broken = fixture();
        let root = broken.root_table().unwrap();
        if let Node::Table(e) = &mut broken.arena[root] {
            e.retain(|(k, _)| !matches!(k, Value::Str(b) if b == b"HeadEnt"));
        }
        let problems = crate::extras::validate(&broken);
        assert!(problems.iter().any(|p| p == "Missing HeadEnt table"), "{problems:?}");
        let fixes = crate::extras::repair_head(&mut broken);
        assert!(!fixes.is_empty());
        assert!(crate::extras::validate(&broken).is_empty(), "{:?}", crate::extras::validate(&broken));
    }

    #[test]
    fn fixture_has_no_dangling_references() {
        let d = fixture();
        let bad = refs::dangling(&d);
        assert!(bad.is_empty(), "{bad:?}");
    }

    #[test]
    fn every_reference_site_in_the_fixture_is_recognised() {
        let d = fixture();
        let hits = refs::scan(&d);
        let paths: Vec<&str> = hits.iter().map(|h| h.path.as_str()).collect();
        for needle in [
            "ACFCrates[",
            "ACFTurret[",
            "tanktracktool.links.wheel_a",
            "WireDupeInfo.Wires.Toggle.Src",
            "DupeParentID",
            "Constraints[0].Entity[0].Index",
            "HeadEnt.Index",
        ] {
            assert!(paths.iter().any(|p| p.contains(needle)), "no hit for {needle}: {paths:?}");
        }
        assert!(hits.iter().all(|h| h.known), "unrecognised: {:?}", hits.iter().filter(|h| !h.known).collect::<Vec<_>>());
    }

    #[test]
    fn deleting_the_wheel_prunes_the_track_link_and_nothing_else() {
        let mut d = fixture();
        let r = d.delete_entity(14.0).expect("delete");
        assert_eq!(r.pruned.tracks, 1, "{:?}", r.pruned);
        assert!(refs::dangling(&d).is_empty());
        // The gun's links are untouched.
        let gun = transform::entity_table(&d, 11.0).unwrap();
        let gm = d.get_table(gun, "EntityMods").unwrap();
        let crates = d.get_table(gm, "ACFCrates").unwrap();
        assert!(matches!(&d.arena[crates], Node::Array(v) if v.len() == 1));
    }

    #[test]
    fn deleting_the_ammo_prunes_the_gun_link() {
        let mut d = fixture();
        let r = d.delete_entity(12.0).expect("delete");
        assert_eq!(r.pruned.acf, 1);
        assert!(refs::dangling(&d).is_empty());
    }

    #[test]
    fn deleting_the_button_removes_the_wire_port() {
        let mut d = fixture();
        let r = d.delete_entity(16.0).expect("delete");
        assert_eq!(r.pruned.wires, 1);
        assert!(refs::dangling(&d).is_empty());
    }

    #[test]
    fn deleting_the_baseplate_clears_parent_and_weld() {
        let mut d = fixture();
        let r = d.delete_entity(10.0).expect("delete");
        assert_eq!(r.removed_constraints, 1, "weld to the base should go");
        assert_eq!(r.pruned.parents, 1);
        assert!(refs::dangling(&d).is_empty());
    }

    #[test]
    fn remapping_follows_every_site() {
        let mut d = fixture();
        let n = refs::remap(&mut d, &[(12.0, 120.0), (14.0, 140.0), (16.0, 160.0), (10.0, 100.0)]);
        assert!(n >= 6, "only {n} references rewritten");
        assert!(refs::dangling(&d).is_empty(), "{:?}", refs::dangling(&d));
        let gun = transform::entity_table(&d, 11.0).unwrap();
        let gm = d.get_table(gun, "EntityMods").unwrap();
        let crates = d.get_table(gm, "ACFCrates").unwrap();
        assert!(matches!(&d.arena[crates], Node::Array(v) if v[0] == Value::Number(120.0)));
    }

    // ------------------------------------------------------------------
    // Duplicate and merge keep references consistent.
    // ------------------------------------------------------------------

    #[test]
    fn duplicating_gun_and_ammo_keeps_their_link_inside_the_copy() {
        let mut d = fixture();
        let before = d.list_entities().len();
        let r = duplicate::duplicate(&mut d, &[11.0, 12.0], (0.0, 0.0, 200.0)).expect("dup");
        assert_eq!(r.entities_added, 2);
        assert_eq!(d.list_entities().len(), before + 2);
        assert!(refs::dangling(&d).is_empty(), "{:?}", refs::dangling(&d));
        // The copied gun links the copied crate, not the original.
        let first = r.first_new_index;
        let hits = refs::scan(&d);
        let copied_gun_links: Vec<f64> = hits
            .iter()
            .filter(|h| h.path.contains(&format!("Entities.{}.EntityMods.ACFCrates", first)))
            .map(|h| h.value)
            .collect();
        assert!(!copied_gun_links.is_empty());
        assert!(copied_gun_links.iter().all(|v| *v >= first), "{copied_gun_links:?}");
    }

    #[test]
    fn merging_the_fixture_into_itself_renumbers_cleanly() {
        let mut base = fixture();
        let other = fixture();
        let n = base.list_entities().len();
        let r = merge::merge(&mut base, other, (500.0, 0.0, 0.0)).expect("merge");
        assert_eq!(r.entities_added, n);
        assert!(refs::dangling(&base).is_empty(), "{:?}", refs::dangling(&base));
        let body = base.to_body().expect("encode");
        let (again, _) = Dupe::from_body(&body).expect("decode");
        assert_eq!(again.list_entities().len(), 2 * n);
    }

    #[test]
    fn a_dupe_survives_encode_decode_encode_identically() {
        let d = fixture();
        let a = d.to_body().unwrap();
        let (re, _) = Dupe::from_body(&a).unwrap();
        let b = re.to_body().unwrap();
        assert_eq!(a, b, "second encode differs from first");
    }

    // ------------------------------------------------------------------
    // Bulk edit with nested paths.
    // ------------------------------------------------------------------

    #[test]
    fn bulk_set_reaches_nested_and_indexed_fields() {
        let mut d = fixture();
        let r = bulk::bulk_set(
            &mut d,
            Some(("Class", "prop_physics")),
            &[("PhysicsObjects[0].Frozen".to_owned(), "false".to_owned())],
        )
        .expect("bulk");
        assert_eq!(r.matched, 2, "two prop_physics in the fixture");
        assert_eq!(r.changed, 2, "{r:?}");
        let wheel = transform::entity_table(&d, 14.0).unwrap();
        let physics = d.get_table(wheel, "PhysicsObjects").unwrap();
        let bone = match &d.arena[physics] {
            Node::Table(e) => match &e[0].1 {
                Value::Table(t) => *t,
                _ => panic!(),
            },
            _ => panic!(),
        };
        assert_eq!(d.get(bone, "Frozen"), Some(&Value::Bool(false)));
    }

    #[test]
    fn bulk_set_skips_a_key_that_does_not_exist_rather_than_adding_it() {
        let mut d = fixture();
        let r = bulk::bulk_set(&mut d, None, &[("EntityMods.mass.Mass".to_owned(), "5".to_owned())])
            .expect("bulk");
        assert_eq!(r.changed, 0);
        assert_eq!(r.skipped_missing, d.list_entities().len());
    }

    // ------------------------------------------------------------------
    // Transform edits keep the constraint pose cache in sync.
    // ------------------------------------------------------------------

    #[test]
    fn moving_the_child_then_resyncing_leaves_the_weld_consistent() {
        let mut d = fixture();
        transform::set_entity_transform(&mut d, 18.0, Some((40.0, 40.0, 40.0)), Some((0.0, 45.0, 0.0)))
            .expect("move");
        let r = transform::resync_constraints(&mut d);
        assert_eq!(r.updated, 1);
        let root = d.root_table().unwrap();
        let cons = d.get_table(root, "Constraints").unwrap();
        let weld = match &d.arena[cons] {
            Node::Array(v) => match &v[0] {
                Value::Table(t) => *t,
                _ => panic!(),
            },
            _ => panic!(),
        };
        let bdi = d.get_table(weld, "BuildDupeInfo").unwrap();
        // EntityPos = pos(child) - pos(base) = (40,40,40) - (0,0,0).
        assert_eq!(d.get(bdi, "EntityPos"), Some(&Value::Vector(40.0, 40.0, 40.0)));
        assert_eq!(d.get(bdi, "Ent1Ang"), Some(&Value::Angle(0.0, 45.0, 0.0)));
    }
}
