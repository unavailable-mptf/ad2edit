//! Reading and writing entity transforms.
//!
//! PhysicsObjects[0].Pos/.Angle is the single source of truth for where an
//! entity sits. The entity's own BuildDupeInfo copies (PosReset, AngleReset,
//! its PhysicsObjects mirror) are rebuilt at paste time and never read from
//! the file, so they don't need maintaining.
//!
//! Constraints are the catch. Before building a constraint, AD2 poses both
//! ends from the CONSTRAINT's stored BuildDupeInfo, then snaps everything
//! back to PosReset. So the rest pose comes from those fields, not from where
//! the entities are. Move an entity without updating them and the dupe
//! pastes, then fights itself apart — a failure neither the round-trip check
//! nor the dangling scan can see.
//!
//! resync_constraints recomputes them all from the current entity transforms.
//! Idempotent and order-independent: move fifty entities however you like,
//! resync once, and it's correct.

use crate::dupe::Dupe;
use crate::value::{key_matches, table_index, Node, Value};

pub type Vec3 = (f64, f64, f64);
type Mat3 = [[f64; 3]; 3];

// ---------------------------------------------------------------------------
// Source engine angle maths
// ---------------------------------------------------------------------------
//
// Angles are (pitch, yaw, roll) in degrees. The layout below mirrors Valve's
// AngleMatrix / MatrixAngles exactly, because a *different* but internally
// consistent convention would round-trip fine in our tests and then be wrong
// in game. Column 0 is forward, column 1 is left, column 2 is up.

fn angle_to_matrix(a: Vec3) -> Mat3 {
    let (p, y, r) = (a.0.to_radians(), a.1.to_radians(), a.2.to_radians());
    let (sp, cp) = (p.sin(), p.cos());
    let (sy, cy) = (y.sin(), y.cos());
    let (sr, cr) = (r.sin(), r.cos());

    [
        [cp * cy, sr * sp * cy - cr * sy, cr * sp * cy + sr * sy],
        [cp * sy, sr * sp * sy + cr * cy, cr * sp * sy - sr * cy],
        [-sp, sr * cp, cr * cp],
    ]
}

fn matrix_to_angle(m: Mat3) -> Vec3 {
    let forward = (m[0][0], m[1][0], m[2][0]);
    let xy_dist = (forward.0 * forward.0 + forward.1 * forward.1).sqrt();

    if xy_dist > 0.001 {
        (
            (-forward.2).atan2(xy_dist).to_degrees(),
            forward.1.atan2(forward.0).to_degrees(),
            m[2][1].atan2(m[2][2]).to_degrees(),
        )
    } else {
        // Straight up or down: yaw and roll act on the same axis, so roll is
        // pinned to zero and yaw absorbs it.
        (
            (-forward.2).atan2(xy_dist).to_degrees(),
            (-m[0][1]).atan2(m[1][1]).to_degrees(),
            0.0,
        )
    }
}

fn mat_mul(a: Mat3, b: Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
        }
    }
    out
}

/// A rotation matrix is orthonormal, so its transpose is its inverse.
fn mat_transpose(m: Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = m[c][r];
        }
    }
    out
}

fn mat_apply(m: Mat3, v: Vec3) -> Vec3 {
    (
        m[0][0] * v.0 + m[0][1] * v.1 + m[0][2] * v.2,
        m[1][0] * v.0 + m[1][1] * v.1 + m[1][2] * v.2,
        m[2][0] * v.0 + m[2][1] * v.1 + m[2][2] * v.2,
    )
}

// ---------------------------------------------------------------------------
// Entity lookups
// ---------------------------------------------------------------------------

/// PhysicsObjects is keyed by bone number, so its keys are numbers rather
/// than strings and the string-keyed lookups won't find them.
pub fn bone(dupe: &Dupe, physics: usize, index: f64) -> Option<usize> {
    let Node::Table(entries) = &dupe.arena[physics] else {
        return None;
    };
    entries
        .iter()
        .find(|(k, _)| matches!(k, Value::Number(n) if *n == index))
        .and_then(|(_, v)| table_index(v))
}

/// Every bone in an entity as (bone number, table index), bone 0 included.
fn bones(dupe: &Dupe, physics: usize) -> Vec<(f64, usize)> {
    let Node::Table(entries) = &dupe.arena[physics] else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|(k, v)| match k {
            Value::Number(n) => Some((*n, table_index(v)?)),
            _ => None,
        })
        .collect()
}

/// The table for one entity, looked up by its entity index.
pub fn entity_table(dupe: &Dupe, index: f64) -> Option<usize> {
    let root = dupe.root_table().ok()?;
    let ents = dupe.get_table(root, "Entities")?;
    let Node::Table(entries) = &dupe.arena[ents] else {
        return None;
    };
    entries
        .iter()
        .find(|(k, _)| matches!(k, Value::Number(n) if *n == index))
        .and_then(|(_, v)| table_index(v))
}

fn vector_of(v: Option<&Value>) -> Option<Vec3> {
    match v {
        Some(Value::Vector(x, y, z)) => Some((*x, *y, *z)),
        _ => None,
    }
}

fn angle_of(v: Option<&Value>) -> Option<Vec3> {
    match v {
        Some(Value::Angle(p, y, r)) => Some((*p, *y, *r)),
        _ => None,
    }
}

/// Rotation matrix for `radians` about an arbitrary unit axis (Rodrigues).
fn axis_angle_matrix(axis: Vec3, radians: f64) -> Mat3 {
    let len = (axis.0 * axis.0 + axis.1 * axis.1 + axis.2 * axis.2).sqrt();
    if len < 1e-9 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let (x, y, z) = (axis.0 / len, axis.1 / len, axis.2 / len);
    let (c, s) = (radians.cos(), radians.sin());
    let t = 1.0 - c;
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

/// The world angle of something whose angle is `inner` in the frame of a
/// parent whose world angle is `outer`: what LocalToWorld does to angles.
pub fn compose_angles(outer: Vec3, inner: Vec3) -> Vec3 {
    matrix_to_angle(mat_mul(angle_to_matrix(outer), angle_to_matrix(inner)))
}

/// Turns an entity's angle by `radians` about a world axis and gives back the
/// new Source angle. The rotate gizmo runs through here so it shares the exact
/// matrix convention the rest of the transform code uses.
pub fn rotate_angle_about(ang: Vec3, axis: Vec3, radians: f64) -> Vec3 {
    matrix_to_angle(mat_mul(axis_angle_matrix(axis, radians), angle_to_matrix(ang)))
}

/// Where something sits and how it is turned once reflected in the plane
/// through `plane_at` with unit normal `plane_normal`. A reflection alone
/// would turn the model inside out, so its own left and right (local y) are
/// swapped as well, which leaves a proper rotation: right for anything that
/// is the same on both sides of its own middle, as plates and wedges are.
pub fn mirrored_pose(at: Vec3, ang: Vec3, plane_at: Vec3, plane_normal: Vec3) -> (Vec3, Vec3) {
    let n = plane_normal;
    let reflect = |v: Vec3| {
        let along = 2.0 * (v.0 * n.0 + v.1 * n.1 + v.2 * n.2);
        (v.0 - along * n.0, v.1 - along * n.1, v.2 - along * n.2)
    };
    let from_plane = reflect((at.0 - plane_at.0, at.1 - plane_at.1, at.2 - plane_at.2));
    let m = angle_to_matrix(ang);
    let column = |c: usize| (m[0][c], m[1][c], m[2][c]);
    let (x, y, z) = (reflect(column(0)), reflect(column(1)), reflect(column(2)));
    let turned = [[x.0, -y.0, z.0], [x.1, -y.1, z.1], [x.2, -y.2, z.2]];
    ((plane_at.0 + from_plane.0, plane_at.1 + from_plane.1, plane_at.2 + from_plane.2), matrix_to_angle(turned))
}

/// The inverse: world offset into the entity's local frame. A rotation
/// matrix's inverse is its transpose, so this applies the transposed matrix
/// rather than computing an inverse.
pub fn inverse_rotate_vec(ang: Vec3, v: Vec3) -> Vec3 {
    let m = angle_to_matrix(ang);
    (
        m[0][0] * v.0 + m[1][0] * v.1 + m[2][0] * v.2,
        m[0][1] * v.0 + m[1][1] * v.1 + m[2][1] * v.2,
        m[0][2] * v.0 + m[1][2] * v.1 + m[2][2] * v.2,
    )
}

/// Rotates a vector by a Source angle. Exposed because the viewport needs to
/// place box corners around an entity's origin, and re-deriving the matrix
/// convention somewhere else is exactly how the two would drift apart.
pub fn rotate_vec(ang: Vec3, v: Vec3) -> Vec3 {
    mat_apply(angle_to_matrix(ang), v)
}

/// Position and angle of an entity, or None if it has no bone 0.
pub fn entity_transform(dupe: &Dupe, index: f64) -> Option<(Vec3, Vec3)> {
    let et = entity_table(dupe, index)?;
    let physics = dupe.get_table(et, "PhysicsObjects")?;
    let b0 = bone(dupe, physics, 0.0)?;
    Some((
        vector_of(dupe.get(b0, "Pos"))?,
        angle_of(dupe.get(b0, "Angle"))?,
    ))
}

/// Writes an entity's transform. Either component can be left alone by
/// passing None. Does NOT resync constraints — the caller does that once
/// after all its moves, not per move.
///
/// Ragdolls carry more than one physics bone. Bones above 0 store a position
/// relative to the entity and an ABSOLUTE angle of their own, and AD2 poses
/// each one independently at paste. So changing bone 0's angle alone would
/// twist the root and leave every other bone behind — a rotation has to be
/// applied to all of them. Returns how many extra bones it moved.
pub fn set_entity_transform(
    dupe: &mut Dupe,
    index: f64,
    pos: Option<Vec3>,
    ang: Option<Vec3>,
) -> Result<usize, String> {
    let et = entity_table(dupe, index).ok_or_else(|| format!("no entity {index}"))?;
    let physics = dupe
        .get_table(et, "PhysicsObjects")
        .ok_or_else(|| format!("entity {index} has no PhysicsObjects"))?;
    let b0 = bone(dupe, physics, 0.0).ok_or_else(|| format!("entity {index} has no bone 0"))?;

    let mut extra_bones = 0;

    if let Some(new_ang) = ang {
        let old_ang = angle_of(dupe.get(b0, "Angle"))
            .ok_or_else(|| format!("entity {index} bone 0 has no Angle"))?;

        // The rotation that takes the old orientation to the new one.
        let delta = mat_mul(
            angle_to_matrix(new_ang),
            mat_transpose(angle_to_matrix(old_ang)),
        );

        // Gather the other bones' poses before writing anything, so the
        // immutable reads finish before the mutable writes start.
        let others: Vec<(usize, Option<Vec3>, Option<Vec3>)> = bones(dupe, physics)
            .into_iter()
            .filter(|(n, _)| *n != 0.0)
            .map(|(_, t)| {
                (
                    t,
                    vector_of(dupe.get(t, "Pos")),
                    angle_of(dupe.get(t, "Angle")),
                )
            })
            .collect();

        for (t, bpos, bang) in others {
            // Offsets are relative to the entity origin, so they rotate about it.
            if let Some(p) = bpos {
                let r = mat_apply(delta, p);
                dupe.set(t, "Pos", Value::Vector(r.0, r.1, r.2));
            }
            // Bone angles are absolute, so compose rather than replace.
            if let Some(a) = bang {
                let r = matrix_to_angle(mat_mul(delta, angle_to_matrix(a)));
                dupe.set(t, "Angle", Value::Angle(r.0, r.1, r.2));
            }
            extra_bones += 1;
        }

        dupe.set(b0, "Angle", Value::Angle(new_ang.0, new_ang.1, new_ang.2));
    }

    if let Some((x, y, z)) = pos {
        // Only bone 0 holds the entity's position; the rest are relative to
        // it and follow a translation for free.
        dupe.set(b0, "Pos", Value::Vector(x, y, z));
    }

    Ok(extra_bones)
}

/// Moves every entity by the same vector. Safe without a constraint resync:
/// EntityPos is a difference between two positions and the stored angles are
/// absolute, so a uniform translation changes neither.
pub fn translate(dupe: &mut Dupe, delta: Vec3) -> usize {
    let Ok(root) = dupe.root_table() else { return 0 };
    let Some(ents) = dupe.get_table(root, "Entities") else {
        return 0;
    };

    let entity_tables: Vec<usize> = match &dupe.arena[ents] {
        Node::Table(entries) => entries.iter().filter_map(|(_, v)| table_index(v)).collect(),
        _ => Vec::new(),
    };

    let mut moved = 0;
    for et in entity_tables {
        let Some(physics) = dupe.get_table(et, "PhysicsObjects") else {
            continue;
        };
        let Some(b0) = bone(dupe, physics, 0.0) else {
            continue;
        };

        let current = vector_of(dupe.get(b0, "Pos"));
        if let Some((x, y, z)) = current {
            dupe.set(
                b0,
                "Pos",
                Value::Vector(x + delta.0, y + delta.1, z + delta.2),
            );
            moved += 1;
        }
    }
    moved
}

// ---------------------------------------------------------------------------
// Constraint pose resync
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ResyncReport {
    pub updated: usize,
    /// Constraints with the world at one end. Only the fields AD2 will
    /// actually read got rewritten.
    pub world_anchored: usize,
    /// Fewer than two endpoints, or no readable transform at either end.
    pub skipped: usize,
    /// Weld bone poses rewritten (ragdoll welds cache one per end).
    pub bones: usize,
}

/// Only overwrites a key that's already there. AD2 tests these fields for nil
/// and behaves differently when they're absent, so inventing one would change
/// how the constraint pastes.
fn set_if_present(dupe: &mut Dupe, table: usize, key: &str, value: Value) -> bool {
    let Node::Table(entries) = &mut dupe.arena[table] else {
        return false;
    };
    let Some(i) = entries.iter().position(|(k, _)| key_matches(k, key)) else {
        return false;
    };
    entries[i].1 = value;
    true
}

/// One physics bone's stored pose: offset from the entity, and absolute angle.
fn bone_pose(dupe: &Dupe, entity: f64, bone_num: f64) -> Option<(Vec3, Vec3)> {
    let et = entity_table(dupe, entity)?;
    let physics = dupe.get_table(et, "PhysicsObjects")?;
    let b = bone(dupe, physics, bone_num)?;
    Some((
        vector_of(dupe.get(b, "Pos"))?,
        angle_of(dupe.get(b, "Angle"))?,
    ))
}

/// Every endpoint of a constraint: entity index, and whether it's the world.
fn endpoints(dupe: &Dupe, list: usize) -> Vec<(f64, bool)> {
    let Node::Array(items) = &dupe.arena[list] else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let t = table_index(item)?;
            let index = dupe.get_number(t, "Index")?;
            let world = matches!(dupe.get(t, "World"), Some(Value::Bool(true)));
            Some((index, world))
        })
        .collect()
}

/// Rebuilds every constraint's cached rest pose from the current entity
/// transforms. Run once after any set of per-entity moves.
pub fn resync_constraints(dupe: &mut Dupe) -> ResyncReport {
    let mut report = ResyncReport::default();

    let Ok(root) = dupe.root_table() else {
        return report;
    };
    let Some(cons) = dupe.get_table(root, "Constraints") else {
        return report;
    };

    let constraint_tables: Vec<usize> = match &dupe.arena[cons] {
        Node::Array(items) => items.iter().filter_map(table_index).collect(),
        _ => Vec::new(),
    };

    for ct in constraint_tables {
        let (Some(bdi), Some(list)) = (
            dupe.get_table(ct, "BuildDupeInfo"),
            dupe.get_table(ct, "Entity"),
        ) else {
            report.skipped += 1;
            continue;
        };

        let eps = endpoints(dupe, list);
        if eps.len() < 2 {
            report.skipped += 1;
            continue;
        }

        // At paste, AD2 picks the two entities it poses like this:
        //
        //     if (gtSetupTable.ENT1[Key]) then first = Val else second = Val end
        //
        // with ENT1 = { Ent, Ent1 }. So `first` is always the Ent1 slot, and
        // `second` is whichever entity argument is assigned last — Ent2 on a
        // pair, Ent4 on a hydraulic or muscle, which is exactly why AD2 also
        // stores an Ent4Ang. Constraint.Entity is indexed by the same number
        // as Ent<n>, so that's slot 0 and the last slot.
        let (i1, w1) = eps[0];
        let (i2, w2) = eps[eps.len() - 1];

        // An endpoint that IS the world has no transform to read, and AD2
        // guards every field that would use one.
        let t1 = if w1 { None } else { entity_transform(dupe, i1) };
        let t2 = if w2 { None } else { entity_transform(dupe, i2) };

        if t1.is_none() && t2.is_none() {
            report.skipped += 1;
            continue;
        }

        let mut wrote = false;

        // second:SetPos(first:GetPos() - EntityPos), guarded by
        // `not second:IsWorld()`. Needs both positions.
        if let (Some((p1, _)), Some((p2, _))) = (t1, t2) {
            let entity_pos = Value::Vector(p1.0 - p2.0, p1.1 - p2.1, p1.2 - p2.2);
            wrote |= set_if_present(dupe, bdi, "EntityPos", entity_pos);
        }

        // first:SetAngles(Ent1Ang), guarded by `not first:IsWorld()`.
        //
        // Ent1Pos is deliberately NOT touched. AD2 writes it as the entity's
        // WORLD position at copy time, whereas PhysicsObjects[0].Pos is
        // relative to the dupe origin — so writing one into the other puts a
        // value in the wrong coordinate space. Nothing reads Ent1Pos at
        // paste, so leaving the original alone is strictly better than
        // replacing it with a number that means something else.
        if let Some((_, a1)) = t1 {
            wrote |= set_if_present(dupe, bdi, "Ent1Ang", Value::Angle(a1.0, a1.1, a1.2));
        }

        // second:SetAngles(Ent2Ang), falling back to Ent4Ang, guarded by
        // `not second:IsWorld()`. Only one of the names will be present.
        if let Some((_, a2)) = t2 {
            let second = Value::Angle(a2.0, a2.1, a2.2);
            wrote |= set_if_present(dupe, bdi, "Ent2Ang", second.clone());
            wrote |= set_if_present(dupe, bdi, "Ent4Ang", second);
        }

        // A weld onto a specific ragdoll bone caches that bone's pose too.
        // AD2 builds the constraint at Bone<n>Pos (an offset from the entity)
        // and Bone<n>Angle (absolute), then snaps the bone back to
        // PhysicsObjects[n]. Those are the same two quantities PhysicsObjects
        // already holds, so they copy straight across — but if they are left
        // stale, rotating a ragdoll builds its weld at the old bone pose and
        // the model tears itself apart on paste.
        let mut bone_writes: Vec<(String, Vec3, Vec3)> = Vec::new();
        for (slot, ent, is_world) in [(1usize, i1, w1), (2usize, i2, w2)] {
            if is_world {
                continue;
            }
            let Some(n) = dupe.get_number(bdi, &format!("Bone{slot}")) else {
                continue;
            };
            // Bone 0 is the entity origin itself, and its stored Pos is
            // relative to the DUPE origin rather than the entity, so the
            // offset is zero by definition. AD2 guards bone 0 the same way.
            let pose = if n == 0.0 {
                entity_transform(dupe, ent).map(|(_, a)| ((0.0, 0.0, 0.0), a))
            } else {
                bone_pose(dupe, ent, n)
            };
            if let Some((bp, ba)) = pose {
                bone_writes.push((format!("Bone{slot}"), bp, ba));
            }
        }
        for (name, bp, ba) in bone_writes {
            let a = set_if_present(dupe, bdi, &format!("{name}Pos"), Value::Vector(bp.0, bp.1, bp.2));
            let b = set_if_present(
                dupe,
                bdi,
                &format!("{name}Angle"),
                Value::Angle(ba.0, ba.1, ba.2),
            );
            if a || b {
                report.bones += 1;
            }
        }

        if !wrote {
            report.skipped += 1;
        } else if w1 || w2 {
            report.world_anchored += 1;
        } else {
            report.updated += 1;
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        // Compared modulo 360, because the matrix conversion normalises.
        let diff = |x: f64, y: f64| {
            let d = ((x - y) % 360.0 + 540.0) % 360.0 - 180.0;
            d.abs() < 1e-6
        };
        diff(a.0, b.0) && diff(a.1, b.1) && diff(a.2, b.2)
    }

    #[test]
    fn angle_matrix_round_trips() {
        for a in [
            (0.0, 0.0, 0.0),
            (30.0, 45.0, 60.0),
            (-12.048, -82.107, 0.457),
            (0.0, 180.0, 0.0),
            (-89.0, 12.0, 3.0),
        ] {
            let back = matrix_to_angle(angle_to_matrix(a));
            assert!(close(a, back), "{a:?} -> {back:?}");
        }
    }

    #[test]
    fn identity_delta_changes_nothing() {
        let a = (11.0, -47.0, 3.0);
        let m = angle_to_matrix(a);
        let delta = mat_mul(m, mat_transpose(m));
        let v = (3.0, -7.0, 11.0);
        let moved = mat_apply(delta, v);
        assert!((moved.0 - v.0).abs() < 1e-9);
        assert!((moved.1 - v.1).abs() < 1e-9);
        assert!((moved.2 - v.2).abs() < 1e-9);
    }

    #[test]
    fn delta_takes_old_orientation_to_new() {
        let old = (10.0, 20.0, 30.0);
        let new = (-5.0, 100.0, 12.0);
        let delta = mat_mul(angle_to_matrix(new), mat_transpose(angle_to_matrix(old)));
        let composed = matrix_to_angle(mat_mul(delta, angle_to_matrix(old)));
        assert!(close(composed, new), "{composed:?} != {new:?}");
    }

    #[test]
    fn turning_by_nothing_changes_nothing() {
        let a = (12.0, -34.0, 56.0);
        for axis in [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)] {
            assert!(close(rotate_angle_about(a, axis, 0.0), a));
        }
    }

    #[test]
    fn a_full_turn_comes_back() {
        let a = (12.0, -34.0, 56.0);
        let full = std::f64::consts::TAU;
        assert!(close(rotate_angle_about(a, (0.0, 0.0, 1.0), full), a));
        assert!(close(rotate_angle_about(a, (0.0, 1.0, 0.0), full), a));
    }

    #[test]
    fn a_quarter_turn_about_z_is_ninety_degrees_of_yaw() {
        let turned = rotate_angle_about(
            (0.0, 0.0, 0.0),
            (0.0, 0.0, 1.0),
            std::f64::consts::FRAC_PI_2,
        );
        assert!(close(turned, (0.0, 90.0, 0.0)), "{turned:?}");
    }

    #[test]
    fn opposite_turns_cancel() {
        let a = (5.0, 40.0, -20.0);
        let axis = (0.0, 1.0, 0.0);
        let there = rotate_angle_about(a, axis, 0.6);
        let back = rotate_angle_about(there, axis, -0.6);
        assert!(close(back, a), "{back:?} != {a:?}");
    }

    #[test]
    fn rotation_preserves_length() {
        let delta = mat_mul(
            angle_to_matrix((5.0, 77.0, -13.0)),
            mat_transpose(angle_to_matrix((0.0, 0.0, 0.0))),
        );
        let v = (3.0, -7.0, 11.0);
        let r = mat_apply(delta, v);
        let len = |t: Vec3| (t.0 * t.0 + t.1 * t.1 + t.2 * t.2).sqrt();
        assert!((len(r) - len(v)).abs() < 1e-9);
    }
}
