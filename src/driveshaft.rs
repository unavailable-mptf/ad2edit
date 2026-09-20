//! ACF.DetermineDriveshaftAngle, from util_sh.lua, on a dupe.
//!
//! Each power connection has a "straight shaft" direction at both ends:
//! - engine Out: at the `driveshaft` attachment; direction −R·X (−R·Y for
//!   transverse engines) — the shaft leaves the engine backwards.
//! - gearbox In: at `input`; direction +R·X for T shapes, −R·Y otherwise.
//! - gearbox OutL/OutR: at `driveshaftL`/`driveshaftR`; the side whose
//!   local y matches the target's; OutL points −R·(0,∓1,0) by shape, OutR
//!   +R·Y.
//! The angle is the larger deviation of the actual shaft line from either
//! straight direction; over MaxDriveshaftAngle (85° default) the link is
//! refused. For wheels only the gearbox side is checked.
//!
//! Without a model library the attachment points fall back to the entity
//! origins, which still catches a gearbox facing the wrong way.

use crate::dupe::Dupe;
use crate::models::ModelLibrary;
use crate::rules::{gearbox_shape, GearboxShape, TRANSVERSE_ENGINES};
use crate::transform::{self, Vec3};
use crate::value::Value;

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}
fn norm(v: Vec3) -> Vec3 {
    let l = (v.0 * v.0 + v.1 * v.1 + v.2 * v.2).sqrt().max(1e-9);
    (v.0 / l, v.1 / l, v.2 / l)
}
fn dot(a: Vec3, b: Vec3) -> f64 {
    a.0 * b.0 + a.1 * b.1 + a.2 * b.2
}
fn deg_between(a: Vec3, b: Vec3) -> f64 {
    dot(norm(a), norm(b)).clamp(-1.0, 1.0).acos().to_degrees()
}
fn neg(v: Vec3) -> Vec3 {
    (-v.0, -v.1, -v.2)
}

fn str_field(dupe: &Dupe, index: f64, key: &str) -> String {
    transform::entity_table(dupe, index)
        .and_then(|et| match dupe.get(et, key) {
            Some(Value::Str(b)) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        })
        .unwrap_or_default()
}

fn model_of(dupe: &Dupe, index: f64) -> String {
    str_field(dupe, index, "Model")
}

/// An attachment's local position on a model, if the library can read it.
fn attachment(lib: Option<&mut ModelLibrary>, model: &str, name: &str) -> Option<Vec3> {
    lib?.attachment(model, name)
}

/// A world point and the straight-shaft world direction for an engine's
/// output.
fn engine_out(dupe: &Dupe, lib: &mut Option<&mut ModelLibrary>, engine: f64) -> Option<(Vec3, Vec3)> {
    let (pos, ang) = transform::entity_transform(dupe, engine)?;
    let id = str_field(dupe, engine, "Engine");
    let trans = TRANSVERSE_ENGINES.iter().any(|t| t.eq_ignore_ascii_case(&id));
    let local = attachment(lib.as_deref_mut(), &model_of(dupe, engine), "driveshaft").unwrap_or((0.0, 0.0, 0.0));
    let point = add(pos, transform::rotate_vec(ang, local));
    let dir_local = if trans { (0.0, 1.0, 0.0) } else { (1.0, 0.0, 0.0) };
    Some((point, neg(transform::rotate_vec(ang, dir_local))))
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    (a.0 + b.0, a.1 + b.1, a.2 + b.2)
}

fn gearbox_in(dupe: &Dupe, lib: &mut Option<&mut ModelLibrary>, gearbox: f64) -> Option<(Vec3, Vec3)> {
    let (pos, ang) = transform::entity_transform(dupe, gearbox)?;
    let shape = gearbox_shape(&str_field(dupe, gearbox, "Gearbox"));
    let local = attachment(lib.as_deref_mut(), &model_of(dupe, gearbox), "input").unwrap_or((0.0, 0.0, 0.0));
    let point = add(pos, transform::rotate_vec(ang, local));
    // Dir point −forward (T) or right (0,1,0); world dir = −R·Dir.
    let dir_point = match shape {
        GearboxShape::T => (-1.0, 0.0, 0.0),
        _ => (0.0, 1.0, 0.0),
    };
    Some((point, neg(transform::rotate_vec(ang, dir_point))))
}

/// The gearbox output facing `target`, chosen by the target's local y.
fn gearbox_out(dupe: &Dupe, lib: &mut Option<&mut ModelLibrary>, gearbox: f64, target_world: Vec3) -> Option<(Vec3, Vec3)> {
    let (pos, ang) = transform::entity_transform(dupe, gearbox)?;
    let shape = gearbox_shape(&str_field(dupe, gearbox, "Gearbox"));
    let local_target = transform::inverse_rotate_vec(ang, sub(target_world, pos));
    let model = model_of(dupe, gearbox);
    let (name, dir_point) = if local_target.1 < 0.0 {
        ("driveshaftL", if shape == GearboxShape::ST { (0.0, -1.0, 0.0) } else { (0.0, 1.0, 0.0) })
    } else {
        ("driveshaftR", (0.0, -1.0, 0.0))
    };
    let local = attachment(lib.as_deref_mut(), &model, name).unwrap_or((0.0, 0.0, 0.0));
    let point = add(pos, transform::rotate_vec(ang, local));
    Some((point, neg(transform::rotate_vec(ang, dir_point))))
}

/// Angle of a powered link. `source` drives `target`. Returns degrees.
pub fn link_angle(dupe: &Dupe, lib: &mut Option<&mut ModelLibrary>, source: f64, target: f64) -> Option<f64> {
    let src_class = str_field(dupe, source, "Class").to_lowercase();
    let dst_class = str_field(dupe, target, "Class").to_lowercase();
    let (tp, _) = transform::entity_transform(dupe, target)?;
    let (op, out_dir) = match src_class.as_str() {
        "acf_engine" => engine_out(dupe, lib, source)?,
        "acf_gearbox" => gearbox_out(dupe, lib, source, tp)?,
        _ => return None,
    };
    if dst_class == "acf_gearbox" {
        let (ip, in_dir) = gearbox_in(dupe, lib, target)?;
        let out_to_in = deg_between(sub(op, ip), in_dir);
        let in_to_out = deg_between(sub(ip, op), out_dir);
        Some(out_to_in.max(in_to_out))
    } else {
        // Wheel (or anything without an input): one direction only.
        Some(deg_between(sub(tp, op), out_dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::fixture;
    use crate::value::{Node, Value};

    fn add_ent(d: &mut Dupe, idx: f64, class: &str, pos: Vec3, ang: Vec3, fields: &[(&str, &str)]) {
        let root = d.root_table().unwrap();
        let ents = d.get_table(root, "Entities").unwrap();
        let et = d.new_table();
        d.set(et, "Class", Value::Str(class.as_bytes().to_vec()));
        d.set(et, "Model", Value::Str(b"models/x.mdl".to_vec()));
        for (k, v) in fields {
            d.set(et, k, Value::Str(v.as_bytes().to_vec()));
        }
        let ph = d.new_table();
        let b = d.new_table();
        d.set(b, "Pos", Value::Vector(pos.0, pos.1, pos.2));
        d.set(b, "Angle", Value::Angle(ang.0, ang.1, ang.2));
        d.set(ph, "0", Value::Table(b));
        if let Node::Table(e) = &mut d.arena[ph] {
            e[0].0 = Value::Number(0.0);
        }
        d.set(et, "PhysicsObjects", Value::Table(ph));
        let m = d.new_table();
        d.set(et, "EntityMods", Value::Table(m));
        if let Node::Table(e) = &mut d.arena[ents] {
            e.push((Value::Number(idx), Value::Table(et)));
        }
    }

    #[test]
    fn a_straight_engine_to_transaxial_is_near_zero_and_a_reversed_one_is_180() {
        let mut d = fixture();
        // Engine at the origin facing +X; its shaft leaves backwards (−X).
        // A T gearbox behind it at −X, also facing +X: its input dir is +X,
        // i.e. it points at the engine. Straight.
        add_ent(&mut d, 50.0, "acf_engine", (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), &[("Engine", "6.5-I6")]);
        add_ent(&mut d, 51.0, "acf_gearbox", (-60.0, 0.0, 0.0), (0.0, 0.0, 0.0), &[("Gearbox", "CVT-T")]);
        let mut lib: Option<&mut ModelLibrary> = None;
        let a = link_angle(&d, &mut lib, 50.0, 51.0).unwrap();
        assert!(a < 1.0, "{a}");
        // Turn the gearbox around: its input now faces away — 180.
        transform::set_entity_transform(&mut d, 51.0, None, Some((0.0, 180.0, 0.0))).unwrap();
        let b = link_angle(&d, &mut lib, 50.0, 51.0).unwrap();
        assert!(b > 179.0, "{b}");
        // Put the gearbox beside the engine instead of behind: 90.
        transform::set_entity_transform(&mut d, 51.0, Some((0.0, 60.0, 0.0)), Some((0.0, 0.0, 0.0))).unwrap();
        let c = link_angle(&d, &mut lib, 50.0, 51.0).unwrap();
        assert!((c - 90.0).abs() < 1.0, "{c}");
    }

    #[test]
    fn a_transaxial_drives_a_wheel_on_its_side_straight() {
        let mut d = fixture();
        add_ent(&mut d, 52.0, "acf_gearbox", (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), &[("Gearbox", "Manual-T")]);
        // Wheel at −Y: OutL, dir point (0,1,0) → world dir −Y: straight.
        let mut lib: Option<&mut ModelLibrary> = None;
        let a = link_angle(&d, &mut lib, 52.0, 14.0).unwrap(); // fixture wheel 14 is at +Y 60
        assert!(a < 1.0, "{a}");
    }
}
