//! Building a dupe up, rather than only editing one down.
//!
//! Everything here writes the exact table shapes GMod and its addons read
//! back. Constraint layouts come from garrysmod/lua/includes/modules/
//! constraint.lua, ACF link rules from ACF-3's util_sv.lua, chip storage
//! from wiremod's gmod_wire_expression2 and StarfallEx's starfall_processor.

use crate::dupe::Dupe;
use crate::duplicate;
use crate::merge;
use crate::transform::{self, Vec3};
use crate::value::{key_matches, table_index, Node, Value};

fn s(t: &str) -> Value {
    Value::Str(t.as_bytes().to_vec())
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    (a.0 + b.0, a.1 + b.1, a.2 + b.2)
}
fn dist(a: Vec3, b: Vec3) -> f64 {
    let d = sub(a, b);
    (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt()
}

/// A world point expressed in an entity's local frame — the inverse of the
/// entity's rotation applied to the offset. `Phys:WorldToLocal` in GMod.
pub fn world_to_local(point: Vec3, pos: Vec3, ang: Vec3) -> Vec3 {
    transform::inverse_rotate_vec(ang, sub(point, pos))
}

fn class_of(dupe: &Dupe, index: f64) -> Option<String> {
    let et = transform::entity_table(dupe, index)?;
    match dupe.get(et, "Class") {
        Some(Value::Str(b)) => Some(String::from_utf8_lossy(b).to_lowercase()),
        _ => None,
    }
}

fn constraints_list(dupe: &mut Dupe) -> Result<usize, String> {
    let root = dupe.root_table()?;
    Ok(match dupe.get_table(root, "Constraints") {
        Some(c) => c,
        None => {
            let c = dupe.new_array();
            dupe.set(root, "Constraints", Value::Table(c));
            c
        }
    })
}

fn push_constraint(dupe: &mut Dupe, table: usize) -> Result<(), String> {
    let cons = constraints_list(dupe)?;
    match &mut dupe.arena[cons] {
        Node::Array(items) => items.push(Value::Table(table)),
        Node::Table(entries) => {
            let n = entries.len() as f64 + 1.0;
            entries.push((Value::Number(n), Value::Table(table)));
        }
    }
    Ok(())
}

/// The `Entity` list and `BuildDupeInfo` every two-entity constraint carries.
/// BuildDupeInfo mirrors what resync_constraints writes, so a constraint
/// made here is indistinguishable from one AD2 copied.
fn two_entity_base(dupe: &mut Dupe, kind: &str, a: f64, b: f64) -> Result<usize, String> {
    let (pa, aa) = transform::entity_transform(dupe, a)
        .ok_or_else(|| format!("entity {a} has no readable transform"))?;
    let (pb, _) = transform::entity_transform(dupe, b)
        .ok_or_else(|| format!("entity {b} has no readable transform"))?;

    let ea = dupe.new_table();
    dupe.set(ea, "Index", Value::Number(a));
    let eb = dupe.new_table();
    dupe.set(eb, "Index", Value::Number(b));
    let ends = dupe.new_array();
    if let Node::Array(items) = &mut dupe.arena[ends] {
        items.push(Value::Table(ea));
        items.push(Value::Table(eb));
    }

    let bdi = dupe.new_table();
    let rel = sub(pa, pb);
    dupe.set(bdi, "EntityPos", Value::Vector(rel.0, rel.1, rel.2));
    dupe.set(bdi, "Ent1Pos", Value::Vector(pa.0, pa.1, pa.2));
    dupe.set(bdi, "Ent1Ang", Value::Angle(aa.0, aa.1, aa.2));
    let (_, ab) = transform::entity_transform(dupe, b).unwrap();
    dupe.set(bdi, "Ent2Ang", Value::Angle(ab.0, ab.1, ab.2));

    let c = dupe.new_table();
    dupe.set(c, "Type", s(kind));
    dupe.set(c, "Entity", Value::Table(ends));
    dupe.set(c, "BuildDupeInfo", Value::Table(bdi));
    Ok(c)
}

pub struct AxisSpec {
    /// World point the hinge passes through.
    pub point: Vec3,
    /// World direction of the hinge axis.
    pub axis: Vec3,
    pub friction: f64,
    pub forcelimit: f64,
    pub torquelimit: f64,
    pub nocollide: bool,
}

/// A hinge between `a` and `b`. GMod's Axis takes LPos1/LPos2 as the hinge
/// point in each entity's local frame, and — this is the part that's easy
/// to get wrong — `LocalAxis` is not a direction but a SECOND POINT in
/// Ent1's frame; the hinge runs from LPos1 to it.
pub fn add_axis(dupe: &mut Dupe, a: f64, b: f64, spec: &AxisSpec) -> Result<(), String> {
    let (pa, aa) = transform::entity_transform(dupe, a).ok_or("no transform for a")?;
    let (pb, ab) = transform::entity_transform(dupe, b).ok_or("no transform for b")?;
    let len = (spec.axis.0 * spec.axis.0 + spec.axis.1 * spec.axis.1 + spec.axis.2 * spec.axis.2).sqrt();
    if len < 1e-9 {
        return Err("axis direction is zero".into());
    }
    let unit = (spec.axis.0 / len, spec.axis.1 / len, spec.axis.2 / len);

    let lpos1 = world_to_local(spec.point, pa, aa);
    let lpos2 = world_to_local(spec.point, pb, ab);
    let local_axis = world_to_local(add(spec.point, unit), pa, aa);

    let c = two_entity_base(dupe, "Axis", a, b)?;
    dupe.set(c, "LPos1", Value::Vector(lpos1.0, lpos1.1, lpos1.2));
    dupe.set(c, "LPos2", Value::Vector(lpos2.0, lpos2.1, lpos2.2));
    dupe.set(c, "LocalAxis", Value::Vector(local_axis.0, local_axis.1, local_axis.2));
    dupe.set(c, "friction", Value::Number(spec.friction));
    dupe.set(c, "forcelimit", Value::Number(spec.forcelimit));
    dupe.set(c, "torquelimit", Value::Number(spec.torquelimit));
    // GMod tests `nocollide > 0` — a boolean here is a Lua error on paste.
    dupe.set(c, "nocollide", Value::Number(if spec.nocollide { 1.0 } else { 0.0 }));
    push_constraint(dupe, c)
}

/// A weld. Shape from a real dupe: LPos2, forcelimit, nocollide.
pub fn add_weld(dupe: &mut Dupe, a: f64, b: f64, forcelimit: f64, nocollide: bool) -> Result<(), String> {
    let (pa, _) = transform::entity_transform(dupe, a).ok_or("no transform for a")?;
    let c = two_entity_base(dupe, "Weld", a, b)?;
    dupe.set(c, "LPos2", Value::Vector(pa.0, pa.1, pa.2));
    dupe.set(c, "forcelimit", Value::Number(forcelimit));
    dupe.set(c, "nocollide", Value::Number(if nocollide { 1.0 } else { 0.0 }));
    push_constraint(dupe, c)
}

/// No-collide between two entities. Carries no BuildDupeInfo in real dupes.
pub fn add_nocollide(dupe: &mut Dupe, a: f64, b: f64) -> Result<(), String> {
    for i in [a, b] {
        if transform::entity_table(dupe, i).is_none() {
            return Err(format!("no entity {i}"));
        }
    }
    let ea = dupe.new_table();
    dupe.set(ea, "Index", Value::Number(a));
    let eb = dupe.new_table();
    dupe.set(eb, "Index", Value::Number(b));
    let ends = dupe.new_array();
    if let Node::Array(items) = &mut dupe.arena[ends] {
        items.push(Value::Table(ea));
        items.push(Value::Table(eb));
    }
    let c = dupe.new_table();
    dupe.set(c, "Type", s("NoCollide"));
    dupe.set(c, "Entity", Value::Table(ends));
    dupe.set(c, "disableOnRemove", Value::Number(0.0));
    push_constraint(dupe, c)
}

/// A ballsocket at a world point. GMod's Ballsocket resolves `LPos` in
/// ENT2's frame (`Phys2:LocalToWorld(LPos)`), not Ent1's.
pub fn add_ballsocket(
    dupe: &mut Dupe,
    a: f64,
    b: f64,
    point: Vec3,
    forcelimit: f64,
    torquelimit: f64,
    nocollide: bool,
) -> Result<(), String> {
    let (pb, ab) = transform::entity_transform(dupe, b).ok_or("no transform for b")?;
    let lpos = world_to_local(point, pb, ab);
    let c = two_entity_base(dupe, "Ballsocket", a, b)?;
    dupe.set(c, "LPos", Value::Vector(lpos.0, lpos.1, lpos.2));
    dupe.set(c, "forcelimit", Value::Number(forcelimit));
    dupe.set(c, "torquelimit", Value::Number(torquelimit));
    dupe.set(c, "nocollide", Value::Number(if nocollide { 1.0 } else { 0.0 }));
    push_constraint(dupe, c)
}

/// The wheel hinge exactly as the reference builds have it: Ent1 is the
/// WHEEL with LPos1 at its own centre and LocalAxis a unit vector along its
/// axle in its own frame; Ent2 is the base with LPos2 the wheel's centre in
/// the base's frame. Endpoints carry Bone and LPos, as AD2 writes them.
pub fn add_wheel_axis(dupe: &mut Dupe, wheel: f64, base: f64, axle_local: Vec3, nocollide: bool) -> Result<(), String> {
    let (pw, _) = transform::entity_transform(dupe, wheel).ok_or("no transform for wheel")?;
    let (pb, ab) = transform::entity_transform(dupe, base).ok_or("no transform for base")?;
    let len = (axle_local.0 * axle_local.0 + axle_local.1 * axle_local.1 + axle_local.2 * axle_local.2).sqrt();
    if len < 1e-9 {
        return Err("axle direction is zero".into());
    }
    let axis = (axle_local.0 / len, axle_local.1 / len, axle_local.2 / len);
    let lpos2 = world_to_local(pw, pb, ab);

    let e1 = dupe.new_table();
    dupe.set(e1, "Index", Value::Number(wheel));
    dupe.set(e1, "Bone", Value::Number(0.0));
    dupe.set(e1, "LPos", Value::Vector(0.0, 0.0, 0.0));
    let e2 = dupe.new_table();
    dupe.set(e2, "Index", Value::Number(base));
    dupe.set(e2, "Bone", Value::Number(0.0));
    dupe.set(e2, "LPos", Value::Vector(lpos2.0, lpos2.1, lpos2.2));
    let ends = dupe.new_array();
    if let Node::Array(items) = &mut dupe.arena[ends] {
        items.push(Value::Table(e1));
        items.push(Value::Table(e2));
    }
    let c = dupe.new_table();
    dupe.set(c, "Type", s("Axis"));
    dupe.set(c, "Entity", Value::Table(ends));
    dupe.set(c, "LPos1", Value::Vector(0.0, 0.0, 0.0));
    dupe.set(c, "LPos2", Value::Vector(lpos2.0, lpos2.1, lpos2.2));
    dupe.set(c, "LocalAxis", Value::Vector(axis.0, axis.1, axis.2));
    dupe.set(c, "friction", Value::Number(0.0));
    dupe.set(c, "forcelimit", Value::Number(0.0));
    dupe.set(c, "torquelimit", Value::Number(0.0));
    // ironlock/tangk leave the wheels colliding with the hull (0); brookster
    // doesn't (1). Both paste; it's a builder's choice.
    dupe.set(c, "nocollide", Value::Number(if nocollide { 1.0 } else { 0.0 }));
    push_constraint(dupe, c)
}

/// Which rotation axis an orientation-only socket pins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lock {
    X,
    Y,
    Z,
}

/// An orientation-only AdvBallsocket that locks ONE axis with friction on it
/// and leaves the other two free — the reference builds use three of these
/// per same-side wheel pair, one per axis, so the pair turns as a solid
/// axle. Shape copied field for field from a working base.
pub fn add_rotation_lock(dupe: &mut Dupe, a: f64, b: f64, lock: Lock) -> Result<(), String> {
    for i in [a, b] {
        if transform::entity_table(dupe, i).is_none() {
            return Err(format!("no entity {i}"));
        }
    }
    let e1 = dupe.new_table();
    dupe.set(e1, "Index", Value::Number(a));
    dupe.set(e1, "Bone", Value::Number(0.0));
    dupe.set(e1, "LPos", Value::Vector(0.0, 0.0, 0.0));
    let e2 = dupe.new_table();
    dupe.set(e2, "Index", Value::Number(b));
    dupe.set(e2, "Bone", Value::Number(0.0));
    dupe.set(e2, "LPos", Value::Vector(0.0, 0.0, 0.0));
    let ends = dupe.new_array();
    if let Node::Array(items) = &mut dupe.arena[ends] {
        items.push(Value::Table(e1));
        items.push(Value::Table(e2));
    }
    let free = (-180.0, 180.0);
    let held = (0.0, 0.0);
    let (x, y, z) = match lock {
        Lock::X => (held, free, free),
        Lock::Y => (free, held, free),
        Lock::Z => (free, free, held),
    };
    let fric = |l: Lock| if l == lock { 50.0 } else { 0.0 };
    let c = dupe.new_table();
    dupe.set(c, "Type", s("AdvBallsocket"));
    dupe.set(c, "Entity", Value::Table(ends));
    dupe.set(c, "LPos1", Value::Vector(0.0, 0.0, 0.0));
    dupe.set(c, "LPos2", Value::Vector(0.0, 0.0, 0.0));
    dupe.set(c, "xmin", Value::Number(x.0));
    dupe.set(c, "xmax", Value::Number(x.1));
    dupe.set(c, "ymin", Value::Number(y.0));
    dupe.set(c, "ymax", Value::Number(y.1));
    dupe.set(c, "zmin", Value::Number(z.0));
    dupe.set(c, "zmax", Value::Number(z.1));
    dupe.set(c, "xfric", Value::Number(fric(Lock::X)));
    dupe.set(c, "yfric", Value::Number(fric(Lock::Y)));
    dupe.set(c, "zfric", Value::Number(fric(Lock::Z)));
    dupe.set(c, "onlyrotation", Value::Number(1.0));
    dupe.set(c, "nocollide", Value::Number(1.0));
    dupe.set(c, "forcelimit", Value::Number(0.0));
    dupe.set(c, "torquelimit", Value::Number(0.0));
    push_constraint(dupe, c)
}

/// Endpoint list `{Index, Bone 0, LPos}` × 2, as AD2 writes it.
fn endpoints(dupe: &mut Dupe, a: f64, la: Vec3, b: f64, lb: Vec3) -> usize {
    let e1 = dupe.new_table();
    dupe.set(e1, "Index", Value::Number(a));
    dupe.set(e1, "Bone", Value::Number(0.0));
    dupe.set(e1, "LPos", Value::Vector(la.0, la.1, la.2));
    let e2 = dupe.new_table();
    dupe.set(e2, "Index", Value::Number(b));
    dupe.set(e2, "Bone", Value::Number(0.0));
    dupe.set(e2, "LPos", Value::Vector(lb.0, lb.1, lb.2));
    let ends = dupe.new_array();
    if let Node::Array(items) = &mut dupe.arena[ends] {
        items.push(Value::Table(e1));
        items.push(Value::Table(e2));
    }
    ends
}

/// The same-side pair lock: three orientation-only sockets, one per
/// axis, so two wheels on a side turn as one solid axle.
pub fn add_pair_lock(dupe: &mut Dupe, a: f64, b: f64) -> Result<(), String> {
    for lock in [Lock::X, Lock::Y, Lock::Z] {
        add_rotation_lock(dupe, a, b, lock)?;
    }
    Ok(())
}

/// VDC's sprung road wheel, copied field for field from vdc_tank.txt. With
/// `r` the wheel's rest position in the base's frame:
/// - Elastic to r + 17 up, constant 100000, damping 500 (the spring)
/// - two Ropes to r + (-100, ±100, 0), length 141.42 (positioning triangle)
/// - one Rope to r itself, length 0, addlength 14 (travel limiter)
/// - orientation socket to the base, X free, Y and Z pinned
/// - nocollide to the base
/// Adjacent-wheel sockets and wheel-to-wheel nocollides are the caller's, as
/// they need the neighbour.
pub fn add_sprung_wheel(dupe: &mut Dupe, wheel: f64, base: f64) -> Result<(), String> {
    let (pw, _) = transform::entity_transform(dupe, wheel).ok_or("no transform for wheel")?;
    let (pb, ab) = transform::entity_transform(dupe, base).ok_or("no transform for base")?;
    let r = world_to_local(pw, pb, ab);
    let zero = (0.0, 0.0, 0.0);

    // Elastic
    let anchor = (r.0, r.1, r.2 + 17.0);
    let ends = endpoints(dupe, wheel, zero, base, anchor);
    let c = dupe.new_table();
    dupe.set(c, "Type", s("Elastic"));
    dupe.set(c, "Entity", Value::Table(ends));
    dupe.set(c, "LPos1", Value::Vector(0.0, 0.0, 0.0));
    dupe.set(c, "LPos2", Value::Vector(anchor.0, anchor.1, anchor.2));
    dupe.set(c, "constant", Value::Number(100000.0));
    dupe.set(c, "damping", Value::Number(500.0));
    dupe.set(c, "rdamping", Value::Number(0.0));
    dupe.set(c, "width", Value::Number(0.0));
    dupe.set(c, "material", s(""));
    dupe.set(c, "stretchonly", Value::Number(0.0));
    push_constraint(dupe, c)?;

    // Ropes: two positioning, one travel limit
    for (dx, dy, length, addlength) in [
        (-100.0, 100.0, 141.42135620117188, 0.0),
        (-100.0, -100.0, 141.42135620117188, 0.0),
        (0.0, 0.0, 0.0, 14.0),
    ] {
        let at = (r.0 + dx, r.1 + dy, r.2);
        let ends = endpoints(dupe, wheel, zero, base, at);
        let c = dupe.new_table();
        dupe.set(c, "Type", s("Rope"));
        dupe.set(c, "Entity", Value::Table(ends));
        dupe.set(c, "LPos1", Value::Vector(0.0, 0.0, 0.0));
        dupe.set(c, "LPos2", Value::Vector(at.0, at.1, at.2));
        dupe.set(c, "length", Value::Number(length));
        dupe.set(c, "addlength", Value::Number(addlength));
        dupe.set(c, "forcelimit", Value::Number(0.0));
        dupe.set(c, "width", Value::Number(0.0));
        dupe.set(c, "material", s(""));
        dupe.set(c, "rigid", Value::Number(0.0));
        push_constraint(dupe, c)?;
    }

    // Orientation socket: X free, Y/Z pinned (±0.001 as the tool writes it)
    let ends = endpoints(dupe, wheel, zero, base, zero);
    let c = dupe.new_table();
    dupe.set(c, "Type", s("AdvBallsocket"));
    dupe.set(c, "Entity", Value::Table(ends));
    dupe.set(c, "LPos1", Value::Vector(0.0, 0.0, 0.0));
    dupe.set(c, "LPos2", Value::Vector(0.0, 0.0, 0.0));
    for (k, v) in [
        ("xmin", -180.0), ("xmax", 180.0),
        ("ymin", -0.001), ("ymax", 0.001),
        ("zmin", -0.001), ("zmax", 0.001),
        ("xfric", 0.0), ("yfric", 0.0), ("zfric", 0.0),
        ("onlyrotation", 1.0), ("nocollide", 0.0),
        ("forcelimit", 0.0), ("torquelimit", 0.0),
    ] {
        dupe.set(c, k, Value::Number(v));
    }
    push_constraint(dupe, c)?;

    add_nocollide(dupe, wheel, base)
}

/// The official T-34-85's sprung road wheel, copied from t-34-85.txt. With
/// `r` the wheel's rest position in the base's frame:
/// - WireHydraulic to r + 48 up, speed 4, `MyCrtl` = a gmod_wire_hydraulic
///   controller entity you've already spawned (its Length/Constant/Damping
///   inputs are wired from a chip in the original — that's `--wire`)
/// - two Ropes to r + (48, ±48, 0), length 67.88 (= 48√2)
/// - one Rope to r, length 0, addlength 25 (travel limiter)
/// - orientation socket to the base, X free, Y and Z pinned
pub fn add_hydraulic_wheel(dupe: &mut Dupe, wheel: f64, base: f64, controller: f64) -> Result<(), String> {
    let (pw, _) = transform::entity_transform(dupe, wheel).ok_or("no transform for wheel")?;
    let (pb, ab) = transform::entity_transform(dupe, base).ok_or("no transform for base")?;
    match class_of(dupe, controller) {
        Some(c) if c == "gmod_wire_hydraulic" => {}
        Some(c) => return Err(format!("{controller} is {c}, not a gmod_wire_hydraulic")),
        None => return Err(format!("no entity {controller}")),
    }
    let r = world_to_local(pw, pb, ab);
    let zero = (0.0, 0.0, 0.0);

    let anchor = (r.0, r.1, r.2 + 48.0);
    let ends = endpoints(dupe, wheel, zero, base, anchor);
    let c = dupe.new_table();
    dupe.set(c, "Type", s("WireHydraulic"));
    dupe.set(c, "Entity", Value::Table(ends));
    dupe.set(c, "width", Value::Number(0.0));
    dupe.set(c, "material", s(""));
    dupe.set(c, "speed", Value::Number(4.0));
    dupe.set(c, "fixed", Value::Number(0.0));
    dupe.set(c, "stretchonly", Value::Number(0.0));
    dupe.set(c, "MyCrtl", Value::Number(controller));
    push_constraint(dupe, c)?;

    for (dx, dy, length, addlength) in [
        (48.0, 48.0, 67.88225099390856, 0.0),
        (48.0, -48.0, 67.88225099390856, 0.0),
        (0.0, 0.0, 0.0, 25.0),
    ] {
        let at = (r.0 + dx, r.1 + dy, r.2);
        let ends = endpoints(dupe, wheel, zero, base, at);
        let c = dupe.new_table();
        dupe.set(c, "Type", s("Rope"));
        dupe.set(c, "Entity", Value::Table(ends));
        dupe.set(c, "length", Value::Number(length));
        dupe.set(c, "addlength", Value::Number(addlength));
        dupe.set(c, "forcelimit", Value::Number(0.0));
        dupe.set(c, "width", Value::Number(0.0));
        dupe.set(c, "material", s(""));
        dupe.set(c, "rigid", Value::Number(0.0));
        push_constraint(dupe, c)?;
    }

    let ends = endpoints(dupe, wheel, zero, base, zero);
    let c = dupe.new_table();
    dupe.set(c, "Type", s("AdvBallsocket"));
    dupe.set(c, "Entity", Value::Table(ends));
    dupe.set(c, "LPos1", Value::Vector(0.0, 0.0, 0.0));
    dupe.set(c, "LPos2", Value::Vector(0.0, 0.0, 0.0));
    for (k, v) in [
        ("xmin", -180.0), ("xmax", 180.0),
        ("ymin", -0.001), ("ymax", 0.001),
        ("zmin", -0.001), ("zmax", 0.001),
        ("xfric", 0.0), ("yfric", 0.0), ("zfric", 0.0),
        ("onlyrotation", 1.0), ("nocollide", 0.0),
        ("forcelimit", 0.0), ("torquelimit", 0.0),
    ] {
        dupe.set(c, k, Value::Number(v));
    }
    push_constraint(dupe, c)
}

/// A wire from `src`'s output `src_port` into `dst`'s input `dst_port`,
/// in the shape WireLib.BuildDupeInfo writes and WireLib.ApplyDupeInfo
/// reads: EntityMods.WireDupeInfo.Wires.<input> = {StartPos, Material,
/// Color, Width, Src, SrcId, SrcPos, Path}. Positions are entity-local
/// and only decorate the rendered cable.
pub fn add_wire(dupe: &mut Dupe, src: f64, src_port: &str, dst: f64, dst_port: &str) -> Result<(), String> {
    if transform::entity_table(dupe, src).is_none() {
        return Err(format!("no entity {src}"));
    }
    let et = transform::entity_table(dupe, dst).ok_or_else(|| format!("no entity {dst}"))?;
    let mods = match dupe.get_table(et, "EntityMods") {
        Some(m) => m,
        None => {
            let m = dupe.new_table();
            dupe.set(et, "EntityMods", Value::Table(m));
            m
        }
    };
    let wdi = match dupe.get_table(mods, "WireDupeInfo") {
        Some(w) => w,
        None => {
            let w = dupe.new_table();
            dupe.set(mods, "WireDupeInfo", Value::Table(w));
            w
        }
    };
    let wires = match dupe.get_table(wdi, "Wires") {
        Some(w) => w,
        None => {
            let w = dupe.new_table();
            dupe.set(wdi, "Wires", Value::Table(w));
            w
        }
    };
    let wire = dupe.new_table();
    dupe.set(wire, "StartPos", Value::Vector(0.0, 0.0, 0.0));
    dupe.set(wire, "Material", s("cable/rope"));
    let color = dupe.new_table();
    for k in ["r", "g", "b", "a"] {
        dupe.set(color, k, Value::Number(255.0));
    }
    dupe.set(wire, "Color", Value::Table(color));
    dupe.set(wire, "Width", Value::Number(1.0));
    dupe.set(wire, "Src", Value::Number(src));
    dupe.set(wire, "SrcId", s(src_port));
    dupe.set(wire, "SrcPos", Value::Vector(0.0, 0.0, 0.0));
    let path = dupe.new_array();
    dupe.set(wire, "Path", Value::Table(path));
    dupe.set(wires, dst_port, Value::Table(wire));
    Ok(())
}

/// ACF-3 ammunition class IDs, from lua/acf/entities/ammo_types/.
pub const AMMO_TYPES: &[&str] = &[
    "AP", "APCR", "APDS", "APFSDS", "APHE", "FL", "FLR", "GLATGM", "HE", "HEAT", "HEATFS", "HP", "SM",
];

/// Sets an ammo crate's type (validated against ACF's registry) and,
/// optionally, its size. Writes the legacy top-level keys, which current
/// ACF still reads when ACF_UserData is absent — the path every reference
/// crate uses.
pub fn set_ammo(dupe: &mut Dupe, crate_index: f64, ammo_type: &str, size: Option<Vec3>) -> Result<(), String> {
    match class_of(dupe, crate_index) {
        Some(c) if c == "acf_ammo" => {}
        Some(c) => return Err(format!("{crate_index} is {c}, not acf_ammo")),
        None => return Err(format!("no entity {crate_index}")),
    }
    let id = AMMO_TYPES
        .iter()
        .find(|t| t.eq_ignore_ascii_case(ammo_type))
        .ok_or_else(|| format!("unknown ammo type {ammo_type}; ACF has {}", AMMO_TYPES.join(", ")))?;
    let et = transform::entity_table(dupe, crate_index).unwrap();
    dupe.set(et, "AmmoType", s(id));
    if let Some(sz) = size {
        dupe.set(et, "Size", Value::Vector(sz.0, sz.1, sz.2));
    }
    Ok(())
}

/// ACF's armour mass for a plate: UpdateArea × UpdateThickness from
/// validation_sv.lua. `surface_in2` is the physics surface area in square
/// inches; for a plate the OBB surface is within a percent of it.
pub fn armour_mass(surface_in2: f64, thickness_mm: f64, ductility_pct: f64) -> f64 {
    let area_cm2 = surface_in2 * 6.45 * 0.52505066107;
    area_cm2 * (1.0 + ductility_pct / 100.0).max(0.0).sqrt() * thickness_mm * 0.00078
}

/// Surface area of a box from its half-extents, square inches.
pub fn box_surface(half: Vec3) -> f64 {
    let (l, w, h) = (half.0 * 2.0, half.1 * 2.0, half.2 * 2.0);
    2.0 * (l * w + l * h + w * h)
}

/// Best-effort half-extents from an SProps model name — rectangles are
/// WxL from the file name, thickness from the folder — when no model
/// library is available. None for anything else.
pub fn sprops_half_extents(model: &str) -> Option<Vec3> {
    let m = model.to_lowercase();
    if !m.contains("sprops/rectangles") {
        return None;
    }
    let name = m.rsplit('/').next()?;
    let dims = name.strip_prefix("rect_")?.strip_suffix(".mdl")?;
    let mut it = dims.split('x');
    let w: f64 = it.next()?.parse().ok()?;
    let l: f64 = it.next()?.parse().ok()?;
    let t = if m.contains("superthin") { 0.5 } else if m.contains("thin") { 1.0 } else { 1.5 };
    Some((w / 2.0, l / 2.0, t / 2.0))
}

/// Tank Track Tool links: `EntityMods.tanktracktool.links` keyed 1..N,
/// slot 1 the chassis and the rest the wheels IN TRACK ORDER — the auto
/// track walks them with SortedPairs to lay the path.
pub fn set_track_links(dupe: &mut Dupe, track: f64, chassis: f64, wheels: &[f64]) -> Result<(), String> {
    match class_of(dupe, track) {
        Some(c) if c.starts_with("sent_tanktracks") => {}
        Some(c) => return Err(format!("{track} is {c}, not a tank track entity")),
        None => return Err(format!("no entity {track}")),
    }
    for i in std::iter::once(&chassis).chain(wheels) {
        if transform::entity_table(dupe, *i).is_none() {
            return Err(format!("no entity {i}"));
        }
    }
    let et = transform::entity_table(dupe, track).unwrap();
    let mods = match dupe.get_table(et, "EntityMods") {
        Some(m) => m,
        None => {
            let m = dupe.new_table();
            dupe.set(et, "EntityMods", Value::Table(m));
            m
        }
    };
    let tt = match dupe.get_table(mods, "tanktracktool") {
        Some(t) => t,
        None => {
            let t = dupe.new_table();
            dupe.set(mods, "tanktracktool", Value::Table(t));
            t
        }
    };
    let links = dupe.new_table();
    let mut all = vec![chassis];
    all.extend_from_slice(wheels);
    if let Node::Table(e) = &mut dupe.arena[links] {
        for (k, idx) in all.iter().enumerate() {
            e.push((Value::Number((k + 1) as f64), Value::Number(*idx)));
        }
    }
    dupe.set(tt, "links", Value::Table(links));
    Ok(())
}

fn modifier_table(dupe: &mut Dupe, entity: f64, name: &str) -> Result<usize, String> {
    let et = transform::entity_table(dupe, entity).ok_or_else(|| format!("no entity {entity}"))?;
    let mods = match dupe.get_table(et, "EntityMods") {
        Some(m) => m,
        None => {
            let m = dupe.new_table();
            dupe.set(et, "EntityMods", Value::Table(m));
            m
        }
    };
    Ok(match dupe.get_table(mods, name) {
        Some(t) => t,
        None => {
            let t = dupe.new_table();
            dupe.set(mods, name, Value::Table(t));
            t
        }
    })
}

/// Fading Door tool. The modifier is literally named "Fading Door". `key`
/// is a numpad key number; a plain keypad opens the door by pressing that
/// key (its KeyGranted), a wire keypad by wiring the door's Fade input.
pub fn set_fading_door(dupe: &mut Dupe, entity: f64, key: f64, material: &str, toggle: bool) -> Result<(), String> {
    let t = modifier_table(dupe, entity, "Fading Door")?;
    dupe.set(t, "key", Value::Number(key));
    dupe.set(t, "toggle", Value::Bool(toggle));
    dupe.set(t, "reversed", Value::Bool(false));
    dupe.set(t, "CanDisableMotion", Value::Bool(true));
    dupe.set(t, "DoorMaterial", s(material));
    for k in ["DoorOpenSound", "DoorCloseSound", "DoorLoopSound"] {
        if dupe.get(t, k).is_none() {
            dupe.set(t, k, Value::Number(0.0));
        }
    }
    Ok(())
}

/// Submaterial tool: one override per model material slot.
pub fn set_submaterial(dupe: &mut Dupe, entity: f64, slot: u32, material: &str) -> Result<(), String> {
    let t = modifier_table(dupe, entity, "submaterial")?;
    dupe.set(t, &format!("SubMaterialOverride_{slot}"), s(material));
    Ok(())
}

/// Keypad tool, plain or wire. `password` None writes `false` (no
/// password). `key_granted` only means something on the plain keypad; the
/// wire one drives by OutputOn/OutputOff.
pub fn set_keypad(dupe: &mut Dupe, entity: f64, password: Option<f64>, secure: bool, key_granted: f64) -> Result<(), String> {
    let class = class_of(dupe, entity).unwrap_or_default();
    let (name, wire) = match class.as_str() {
        "keypad" => ("keypad_password_passthrough", false),
        "keypad_wire" => ("keypad_wire_password_passthrough", true),
        other => return Err(format!("{entity} is {other}, not a keypad or keypad_wire")),
    };
    let t = modifier_table(dupe, entity, name)?;
    dupe.set(t, "Password", match password { Some(p) => Value::Number(p), None => Value::Bool(false) });
    dupe.set(t, "Secure", Value::Bool(secure));
    for k in ["RepeatsGranted", "RepeatsDenied", "DelayGranted", "DelayDenied", "InitDelayGranted", "InitDelayDenied", "LengthGranted", "LengthDenied"] {
        if dupe.get(t, k).is_none() {
            dupe.set(t, k, Value::Number(0.0));
        }
    }
    if wire {
        if dupe.get(t, "OutputOn").is_none() {
            dupe.set(t, "OutputOn", Value::Number(1.0));
        }
        if dupe.get(t, "OutputOff").is_none() {
            dupe.set(t, "OutputOff", Value::Number(0.0));
        }
    } else {
        dupe.set(t, "KeyGranted", Value::Number(key_granted));
        if dupe.get(t, "KeyDenied").is_none() {
            dupe.set(t, "KeyDenied", Value::Number(0.0));
        }
    }
    // The entity itself carries Secure too.
    let et = transform::entity_table(dupe, entity).unwrap();
    dupe.set(et, "Secure", Value::Bool(secure));
    Ok(())
}

/// The Resizer tool. It has no modifier of its own: it scales bones with
/// ManipulateBoneScale, and GMod's duplicator saves that as
/// `BoneManip[bone] = { s = Vector }` and restores it with
/// DoBoneManipulator. `bones` is how many bones to scale (the reference
/// scaled 0 and 1 on a two-bone prop); scaling more than the model has is
/// ignored on paste.
pub fn set_resize(dupe: &mut Dupe, entity: f64, scale: Vec3, bones: u32) -> Result<(), String> {
    let et = transform::entity_table(dupe, entity).ok_or_else(|| format!("no entity {entity}"))?;
    let bm = match dupe.get_table(et, "BoneManip") {
        Some(t) => t,
        None => {
            let t = dupe.new_table();
            dupe.set(et, "BoneManip", Value::Table(t));
            t
        }
    };
    for bone in 0..bones.max(1) {
        let key = format!("{bone}");
        let b = match dupe.get_table(bm, &key) {
            Some(b) => b,
            None => {
                let b = dupe.new_table();
                dupe.set(bm, &key, Value::Table(b));
                // Bone keys are numbers in a real dupe.
                if let Node::Table(e) = &mut dupe.arena[bm] {
                    if let Some(last) = e.last_mut() {
                        last.0 = Value::Number(bone as f64);
                    }
                }
                b
            }
        };
        dupe.set(b, "s", Value::Vector(scale.0, scale.1, scale.2));
    }
    Ok(())
}

/// Armour on a prop: ACF_Armor {Thickness, Ductility}, clamped to ACF's
/// ranges, and the mass modifier removed so the thickness is what ACF
/// reads (with both present it ignores the thickness).
pub fn set_armour(dupe: &mut Dupe, entity: f64, thickness_mm: f64, ductility: f64) -> Result<(), String> {
    let class = class_of(dupe, entity).unwrap_or_default();
    if class != "prop_physics" {
        return Err(format!("{entity} is {class}; armour goes on prop_physics plates"));
    }
    let t = modifier_table(dupe, entity, "ACF_Armor")?;
    dupe.set(t, "Thickness", Value::Number(thickness_mm.clamp(crate::rules::MIN_ARMOUR_MM, crate::rules::MAX_ARMOUR_MM)));
    dupe.set(t, "Ductility", Value::Number(ductility.clamp(crate::rules::MIN_DUCTILITY, crate::rules::MAX_DUCTILITY)));
    let et = transform::entity_table(dupe, entity).unwrap();
    if let Some(mods) = dupe.get_table(et, "EntityMods") {
        if let Node::Table(e) = &mut dupe.arena[mods] {
            e.retain(|(k, _)| !key_matches(k, "mass"));
        }
    }
    Ok(())
}

/// One plate's armour mass from its bounds (or SProps name), thickness and
/// ductility — the same formula the mass report uses.
pub fn plate_mass(half: Vec3, thickness_mm: f64, ductility: f64) -> f64 {
    armour_mass(box_surface(half), thickness_mm, ductility)
}

/// Make Spherical's modifier, as the tool writes it. Radius defaults to the
/// wheel's largest half-extent; `mass` mirrors the entity's mass modifier
/// since the tool re-applies it after swapping the collision.
pub fn make_spherical(dupe: &mut Dupe, entity: f64, radius: f64) -> Result<(), String> {
    let et = transform::entity_table(dupe, entity).ok_or_else(|| format!("no entity {entity}"))?;
    let mods = match dupe.get_table(et, "EntityMods") {
        Some(m) => m,
        None => {
            let m = dupe.new_table();
            dupe.set(et, "EntityMods", Value::Table(m));
            m
        }
    };
    let mass = dupe
        .get_table(mods, "mass")
        .and_then(|m| dupe.get_number(m, "Mass"))
        .unwrap_or(100.0);
    let t = match dupe.get_table(mods, "MakeSphericalCollisions") {
        Some(t) => t,
        None => {
            let t = dupe.new_table();
            dupe.set(mods, "MakeSphericalCollisions", Value::Table(t));
            t
        }
    };
    dupe.set(t, "enabled", Value::Bool(true));
    dupe.set(t, "radius", Value::Number(radius));
    dupe.set(t, "noradius", Value::Number(radius));
    dupe.set(t, "isrenderoffset", Value::Number(0.0));
    dupe.set(t, "renderoffset", Value::Vector(0.0, 0.0, 0.0));
    dupe.set(t, "obbcenter", Value::Vector(0.0, 0.0, 0.0));
    dupe.set(t, "mass", Value::Number(mass));
    Ok(())
}

/// Sets BuildDupeInfo.DupeParentID — what AD2 reads to SetParent on paste.
pub fn set_parent(dupe: &mut Dupe, child: f64, parent: f64) -> Result<(), String> {
    if transform::entity_table(dupe, parent).is_none() {
        return Err(format!("no entity {parent}"));
    }
    let et = transform::entity_table(dupe, child).ok_or_else(|| format!("no entity {child}"))?;
    let bdi = match dupe.get_table(et, "BuildDupeInfo") {
        Some(b) => b,
        None => {
            let b = dupe.new_table();
            dupe.set(et, "BuildDupeInfo", Value::Table(b));
            b
        }
    };
    dupe.set(bdi, "DupeParentID", Value::Number(parent));
    Ok(())
}

// ---------------------------------------------------------------------------
// ACF links
// ---------------------------------------------------------------------------

/// From ACF-3 util_sv.lua's RegisterClassLink calls, paired with the
/// EntityMods key the LINKING entity stores the target in. Direction matters:
/// the modifier lives on the first class.
pub const ACF_LINK_TABLE: &[(&str, &str, &str)] = &[
    ("acf_gun", "acf_ammo", "ACFCrates"),
    ("acf_gun", "acf_turret", "ACFTurret"),
    ("acf_engine", "acf_fueltank", "ACFFuelTanks"),
    ("acf_engine", "acf_gearbox", "ACFGearboxes"),
    ("acf_gearbox", "acf_gearbox", "ACFGearboxes"),
    ("acf_gearbox", "prop_physics", "ACFWheels"),
    ("acf_gearbox", "acf_effector", "ACFEffectors"),
    ("acf_rack", "acf_ammo", "ACFCrates"),
    ("acf_rack", "acf_radar", "ACFRadar"),
    ("acf_rack", "acf_computer", "ACFComputer"),
    ("acf_turret", "acf_turret_gyro", "ACFGyro"),
    ("acf_turret", "acf_turret_motor", "ACFMotor"),
    ("acf_autoloader", "acf_ammo", "ACFAmmoCrates"),
    ("acf_autoloader", "acf_gun", "ACFGun"),
    ("acf_computer", "acf_gun", "ACFWeapons"),
    ("acf_computer", "acf_rack", "ACFWeapons"),
    ("acf_turret_computer", "acf_gun", "ACFGun"),
    // acf_controller, from its modules/*.lua RegisterControllerLink calls.
    ("acf_controller", "acf_gearbox", "Gearbox"),
    ("acf_controller", "acf_baseplate", "Baseplate"),
    ("acf_controller", "prop_vehicle_prisoner_pod", "Seat"),
    ("acf_controller", "prop_physics", "SteerPlates"),
    ("acf_controller", "acf_gun", "Guns"),
    ("acf_controller", "acf_turret", "Turrets"),
    ("acf_controller", "acf_turret_computer", "TurretComputer"),
    ("acf_controller", "acf_computer", "GuidanceComputer"),
    ("acf_controller", "acf_rack", "Racks"),
    ("acf_controller", "acf_radar", "Radar"),
    // acf_crew stores what it serves in CrewTargets: a driver -> baseplate,
    // a gunner/loader -> gun. Seen in the reference base.
    ("acf_crew", "acf_baseplate", "CrewTargets"),
    ("acf_crew", "acf_gun", "CrewTargets"),
    ("acf_crew", "acf_engine", "CrewTargets"),
    ("acf_crew", "acf_turret", "CrewTargets"),
    // The baseplate's own generated seat.
    ("acf_baseplate", "prop_vehicle_prisoner_pod", "LuaSeatID"),
];

/// ACF refuses links further apart than this, in Source units.
pub const ACF_MAX_LINK_DISTANCE: f64 = 650.0;

/// The modifier a (src, dst) class pair writes to, if the pair is valid in
/// either direction. Returns (owner index, target index, modifier).
fn link_slot(dupe: &Dupe, a: f64, b: f64) -> Result<(f64, f64, &'static str), String> {
    let ca = class_of(dupe, a).ok_or_else(|| format!("entity {a} has no Class"))?;
    let cb = class_of(dupe, b).ok_or_else(|| format!("entity {b} has no Class"))?;
    let is_wheel = |c: &str| c == "prop_physics" || c.contains("wheel") || c.contains("tire");
    let matches = |want: &str, have: &str| {
        if want == "prop_physics" {
            is_wheel(have)
        } else {
            want == have
        }
    };
    for (src, dst, modifier) in ACF_LINK_TABLE {
        if matches(src, &ca) && matches(dst, &cb) {
            return Ok((a, b, modifier));
        }
        if matches(src, &cb) && matches(dst, &ca) {
            return Ok((b, a, modifier));
        }
    }
    Err(format!("ACF has no link between {ca} and {cb}"))
}

#[derive(Debug)]
pub struct LinkReport {
    pub owner: f64,
    pub target: f64,
    pub modifier: &'static str,
    pub distance: f64,
}

/// Links two ACF entities the way the in-game link tool would, refusing
/// pairs ACF refuses and distances it refuses.
pub fn add_link(dupe: &mut Dupe, a: f64, b: f64) -> Result<LinkReport, String> {
    let (owner, target, modifier) = link_slot(dupe, a, b)?;
    let (pa, _) = transform::entity_transform(dupe, owner).ok_or("no transform for owner")?;
    let (pb, _) = transform::entity_transform(dupe, target).ok_or("no transform for target")?;
    let distance = dist(pa, pb);
    if distance > ACF_MAX_LINK_DISTANCE {
        return Err(format!(
            "{owner} and {target} are {distance:.0} units apart; ACF's limit is {ACF_MAX_LINK_DISTANCE:.0}"
        ));
    }

    let et = transform::entity_table(dupe, owner).unwrap();
    let mods = match dupe.get_table(et, "EntityMods") {
        Some(m) => m,
        None => {
            let m = dupe.new_table();
            dupe.set(et, "EntityMods", Value::Table(m));
            m
        }
    };
    let arr = match dupe.get_table(mods, modifier) {
        Some(a) => a,
        None => {
            let a = dupe.new_array();
            dupe.set(mods, modifier, Value::Table(a));
            a
        }
    };
    let already = match &dupe.arena[arr] {
        Node::Array(items) => items.iter().any(|v| matches!(v, Value::Number(n) if *n == target)),
        Node::Table(e) => e.iter().any(|(_, v)| matches!(v, Value::Number(n) if *n == target)),
    };
    if !already {
        match &mut dupe.arena[arr] {
            Node::Array(items) => items.push(Value::Number(target)),
            Node::Table(e) => {
                let n = e.len() as f64 + 1.0;
                e.push((Value::Number(n), Value::Number(target)));
            }
        }
    }
    Ok(LinkReport {
        owner,
        target,
        modifier,
        distance,
    })
}

// ---------------------------------------------------------------------------
// Spawning from the catalogue
// ---------------------------------------------------------------------------

/// A plain prop from a model path, frozen, with the tables every entity a
/// tool made carries. For parts the catalogue cannot list one by one, like
/// the plates a hull is generated from.
pub fn spawn_prop(dupe: &mut Dupe, model: &str, at: Vec3, ang: Vec3) -> Result<f64, String> {
    let root = dupe.root_table()?;
    let ents = dupe.get_table(root, "Entities").ok_or("no Entities table")?;
    let next = dupe.list_entities().iter().map(|(i, _, _)| *i).fold(0.0f64, f64::max) + 1.0;
    let et = dupe.new_table();
    dupe.set(et, "Class", s("prop_physics"));
    dupe.set(et, "Model", s(model));
    dupe.set(et, "CollisionGroup", Value::Number(0.0));
    let physics = dupe.new_table();
    let bone = dupe.new_table();
    dupe.set(bone, "Pos", Value::Vector(at.0, at.1, at.2));
    dupe.set(bone, "Angle", Value::Angle(ang.0, ang.1, ang.2));
    dupe.set(bone, "Frozen", Value::Bool(true));
    dupe.set(physics, "0", Value::Table(bone));
    if let Node::Table(e) = &mut dupe.arena[physics] {
        e[0].0 = Value::Number(0.0);
    }
    dupe.set(et, "PhysicsObjects", Value::Table(physics));
    let mods = dupe.new_table();
    dupe.set(et, "EntityMods", Value::Table(mods));
    if let Node::Table(e) = &mut dupe.arena[ents] {
        e.push((Value::Number(next), Value::Table(et)));
    }
    Ok(next)
}

/// A new entity from a catalogue item: class, model, the item's paste key,
/// physics bone, and the numbers the item needs (calibre for a gun, size
/// for a tank or crate, the crew's model and pose). ACF's VerifyData fills
/// every other field with its default on paste — the same forgiving path
/// the reference dupes rely on.
pub fn spawn_catalog(dupe: &mut Dupe, item: &crate::catalog::Item, at: Vec3, ang: Vec3, numbers: &[(&str, f64)]) -> Result<f64, String> {
    let root = dupe.root_table()?;
    let ents = dupe.get_table(root, "Entities").ok_or("no Entities table")?;
    let next = dupe.list_entities().iter().map(|(i, _, _)| *i).fold(0.0f64, f64::max) + 1.0;
    let et = dupe.new_table();
    dupe.set(et, "Class", s(item.entity));
    dupe.set(et, "Model", s(item.model));
    dupe.set(et, "CollisionGroup", Value::Number(0.0));
    if !item.key.is_empty() {
        dupe.set(et, item.key, s(item.value));
    }
    for (k, v) in numbers {
        dupe.set(et, k, Value::Number(*v));
    }
    // Per-class essentials that VerifyData does not default sensibly.
    match item.entity {
        "acf_gun" => {
            let cal = numbers.iter().find(|(k, _)| *k == "Caliber").map(|(_, v)| *v);
            let (lo, hi) = item.range.unwrap_or((20.0, 170.0));
            let cal = cal.unwrap_or(lo).clamp(lo, hi);
            dupe.set(et, "Caliber", Value::Number(cal));
        }
        "acf_ammo" => {
            if dupe.get(et, "Size").is_none() {
                dupe.set(et, "Size", Value::Vector(24.0, 24.0, 24.0));
            }
        }
        "acf_fueltank" => {
            dupe.set(et, "FuelTank", s("Scalable"));
            if dupe.get(et, "Size").is_none() {
                dupe.set(et, "Size", Value::Vector(24.0, 24.0, 24.0));
            }
        }
        "acf_crew" => {
            dupe.set(et, "CrewModelID", s("Sitting"));
            dupe.set(et, "CrewPoseID", s("SitCamera"));
        }
        "acf_baseplate" => {
            let ud = dupe.new_table();
            for (k, d) in [("Width", 48.0), ("Length", 96.0), ("Thickness", 2.0)] {
                let v = numbers.iter().find(|(kk, _)| *kk == k).map(|(_, v)| *v).unwrap_or(d);
                dupe.set(ud, k, Value::Number(v));
            }
            dupe.set(et, "ACF_UserData", Value::Table(ud));
        }
        "acf_turret" => {
            if dupe.get(et, "RingSize").is_none() {
                dupe.set(et, "RingSize", Value::Number(if item.value == "Turret-V" { 12.0 } else { 60.0 }));
            }
            if dupe.get(et, "MaxSpeed").is_none() {
                dupe.set(et, "MaxSpeed", Value::Number(if item.value == "Turret-V" { crate::rules::TRUNNION_MAX_SPEED } else { crate::rules::RING_MAX_SPEED }));
            }
            if item.value == "Turret-V" {
                if dupe.get(et, "MinDeg").is_none() { dupe.set(et, "MinDeg", Value::Number(-7.0)); }
                if dupe.get(et, "MaxDeg").is_none() { dupe.set(et, "MaxDeg", Value::Number(20.0)); }
            }
        }
        "acf_turret_motor" => {
            if dupe.get(et, "Teeth").is_none() { dupe.set(et, "Teeth", Value::Number(12.0)); }
            if dupe.get(et, "CompSize").is_none() { dupe.set(et, "CompSize", Value::Number(1.0)); }
        }
        "acf_gearbox" => {
            if dupe.get(et, "FinalDrive").is_none() { dupe.set(et, "FinalDrive", Value::Number(1.0)); }
            if dupe.get(et, "GearAmount").is_none() { dupe.set(et, "GearAmount", Value::Number(3.0)); }
        }
        "acf_controller" => {
            let dt = dupe.new_table();
            dupe.set(dt, "BrakeEngagement", Value::Number(1.0));
            dupe.set(dt, "BrakeStrength", Value::Number(300.0));
            dupe.set(et, "DT", Value::Table(dt));
        }
        _ => {}
    }
    // A Wiremod part gets the options its tool would give it by default; the
    // game rebuilds the entity from these fields on paste.
    if let Some(part) = crate::wire_items::PARTS.iter().find(|part| part.entity == item.entity && part.name == item.name) {
        for (field, setting) in part.settings {
            let value = match setting {
                crate::wire_items::Setting::Number(n) => Value::Number(*n),
                crate::wire_items::Setting::Flag(on) => Value::Bool(*on),
                crate::wire_items::Setting::Text(text) => s(text),
            };
            if dupe.get(et, field).is_none() {
                dupe.set(et, field, value);
            }
        }
    }
    if item.entity == "gmod_wire_gate" {
        // The gate tool's own default: a gate does not collide.
        dupe.set(et, "noclip", Value::Bool(true));
    }
    if item.entity == "gmod_wire_expression2" {
        write_e2_port_lists(dupe, et, item.value);
        dupe.set(et, "_name", s(item.value.trim_start_matches("@name ").trim()));
    }
    let physics = dupe.new_table();
    let bone = dupe.new_table();
    dupe.set(bone, "Pos", Value::Vector(at.0, at.1, at.2));
    dupe.set(bone, "Angle", Value::Angle(ang.0, ang.1, ang.2));
    dupe.set(bone, "Frozen", Value::Bool(true));
    dupe.set(physics, "0", Value::Table(bone));
    if let Node::Table(e) = &mut dupe.arena[physics] {
        e[0].0 = Value::Number(0.0);
    }
    dupe.set(et, "PhysicsObjects", Value::Table(physics));
    let mods = dupe.new_table();
    dupe.set(et, "EntityMods", Value::Table(mods));
    // A wheel from the catalogue is a prop with a mass modifier and a
    // spherical collision at the model's radius, as every reference has.
    if item.category == "Wheels" {
        let m = dupe.new_table();
        dupe.set(m, "Mass", Value::Number(item.mass.unwrap_or(100.0)));
        dupe.set(mods, "mass", Value::Table(m));
    }
    if let Node::Table(e) = &mut dupe.arena[ents] {
        e.push((Value::Number(next), Value::Table(et)));
    }
    if item.category == "Wheels" {
        if let Some((r, _)) = item.range {
            let _ = make_spherical(dupe, next, r);
        }
    }
    Ok(next)
}

// ---------------------------------------------------------------------------
// Spawning from a template
// ---------------------------------------------------------------------------

/// Copies one entity out of `template` into `dupe`, placed at `at` with the
/// given angle. Returns the new entity's index. A real entity from a real
/// dupe carries every class field its addon expects, which is why this
/// takes a template rather than inventing defaults for an acf_engine.
pub fn spawn_from(
    dupe: &mut Dupe,
    template: &Dupe,
    index: f64,
    at: Vec3,
    ang: Option<Vec3>,
) -> Result<f64, String> {
    let mut part = duplicate::extract(template, &[index])?;
    // A real part carries links, a parent, CFW _links and wires to the rest
    // of the build it came from. None of that comes along, so every
    // reference to an entity outside the part is pruned before merging —
    // otherwise they'd renumber onto whatever happens to occupy those
    // indices in the destination.
    let outside: Vec<f64> = crate::refs::dangling(&part).iter().map(|h| h.value).collect();
    let mut seen = std::collections::HashSet::new();
    for t in outside {
        if seen.insert(t.to_bits()) {
            crate::refs::prune_entity(&mut part, t);
        }
    }
    // A part lifted out of a build that draws itself through prop2mesh is
    // usually hidden — colour alpha 0, RenderMode 1 or 4. It's being spawned
    // to be seen, so make it visible; the colour itself is kept.
    if let Some(et) = transform::entity_table(&part, index) {
        if let Some(mods) = part.get_table(et, "EntityMods") {
            if let Some(colour) = part.get_table(mods, "colour") {
                let hidden = part
                    .get_table(colour, "Color")
                    .and_then(|c| part.get_number(c, "a"))
                    .map(|a| a <= 0.0)
                    .unwrap_or(false);
                if hidden {
                    if let Some(c) = part.get_table(colour, "Color") {
                        part.set(c, "a", Value::Number(255.0));
                    }
                    part.set(colour, "RenderMode", Value::Number(0.0));
                }
            }
        }
    }
    // The parent was outside too; a spawned part starts unparented.
    if let Some(et) = transform::entity_table(&part, index) {
        if let Some(bdi) = part.get_table(et, "BuildDupeInfo") {
            if let Node::Table(e) = &mut part.arena[bdi] {
                e.retain(|(k, _)| !key_matches(k, "DupeParentID"));
            }
        }
    }
    let (pos, _) = transform::entity_transform(&part, index)
        .ok_or_else(|| format!("template entity {index} has no transform"))?;
    // Bring the part to the origin, then merge lands it at `at`.
    transform::translate(&mut part, (-pos.0, -pos.1, -pos.2));
    if let Some(a) = ang {
        transform::set_entity_transform(&mut part, index, None, Some(a))?;
    }
    let report = merge::merge(dupe, part, at)?;
    Ok(report.first_new_index)
}

// ---------------------------------------------------------------------------
// Chips: E2 and Starfall code in and out
// ---------------------------------------------------------------------------

pub enum Chip {
    /// Expression 2. `_original` holds the code with `"` as byte 163 and
    /// newline as byte 128; decoded here.
    E2 { name: String, code: String },
    /// Starfall. Files are LZMA-packed with a small header; the main file
    /// name is stored alongside.
    Starfall { mainfile: String, files: Vec<(String, String)> },
}

fn e2_decode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| match *b {
            163 => '"',
            128 => '\n',
            b => b as char,
        })
        .collect()
}

fn e2_encode(text: &str) -> Vec<u8> {
    text.bytes()
        .map(|b| match b {
            b'"' => 163,
            b'\n' => 128,
            b => b,
        })
        .collect()
}

fn read_i32(d: &[u8], pos: &mut usize) -> Option<u32> {
    let v = u32::from_le_bytes(d.get(*pos..*pos + 4)?.try_into().ok()?);
    *pos += 4;
    Some(v)
}

/// SF.DecompressFiles: util.Decompress (GMod's LZMA layout, the same one
/// AD2 bodies use), then int32 legacy, int32 count, then per file an int32
/// name length, the name, and an int32 code length; all code follows.
fn sf_unpack(packed: &[u8]) -> Result<Vec<(String, String)>, String> {
    let raw = crate::dupefile::decompress(packed)?;
    let mut pos = 4; // legacy int32
    let count = read_i32(&raw, &mut pos).ok_or("short starfall header")? as usize;
    let mut headers = Vec::with_capacity(count);
    for _ in 0..count {
        let n = read_i32(&raw, &mut pos).ok_or("short starfall header")? as usize;
        let name = String::from_utf8_lossy(raw.get(pos..pos + n).ok_or("short name")?).into_owned();
        pos += n;
        let size = read_i32(&raw, &mut pos).ok_or("short starfall header")? as usize;
        headers.push((name, size));
    }
    let mut files = Vec::new();
    for (name, size) in headers {
        let code = String::from_utf8_lossy(raw.get(pos..pos + size).ok_or("short code")?).into_owned();
        pos += size;
        files.push((name, code));
    }
    Ok(files)
}

fn sf_pack(files: &[(String, String)]) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    raw.extend_from_slice(&0u32.to_le_bytes());
    raw.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (name, code) in files {
        raw.extend_from_slice(&(name.len() as u32).to_le_bytes());
        raw.extend_from_slice(name.as_bytes());
        raw.extend_from_slice(&(code.len() as u32).to_le_bytes());
    }
    for (_, code) in files {
        raw.extend_from_slice(code.as_bytes());
    }
    // GMod's util.Compress parameters: the same LZMA setup AD2 bodies use.
    crate::dupefile::compress(&raw, 3, 0, 2, 65536, true)
}

pub fn chip_export(dupe: &Dupe, index: f64) -> Result<Chip, String> {
    let et = transform::entity_table(dupe, index).ok_or_else(|| format!("no entity {index}"))?;
    let class = class_of(dupe, index).unwrap_or_default();

    if class == "gmod_wire_expression2" {
        let code = match dupe.get(et, "_original") {
            Some(Value::Str(b)) => e2_decode(b),
            _ => return Err("E2 has no _original code".into()),
        };
        let name = match dupe.get(et, "_name") {
            Some(Value::Str(b)) => String::from_utf8_lossy(b).into_owned(),
            _ => String::new(),
        };
        return Ok(Chip::E2 { name, code });
    }

    if class.starts_with("starfall_") {
        let mods = dupe.get_table(et, "EntityMods").ok_or("starfall chip has no EntityMods")?;
        let info = dupe.get_table(mods, "SFDupeInfo").ok_or("no SFDupeInfo")?;
        let sf = dupe.get_table(info, "starfall").ok_or("no starfall table in SFDupeInfo")?;
        let mainfile = match dupe.get(sf, "mainfile") {
            Some(Value::Str(b)) => String::from_utf8_lossy(b).into_owned(),
            _ => String::new(),
        };
        let files = match dupe.get(sf, "files") {
            Some(Value::Str(b)) => sf_unpack(b)?,
            _ => return Err("starfall files are not a packed string (old format)".into()),
        };
        return Ok(Chip::Starfall { mainfile, files });
    }

    Err(format!("entity {index} is {class}, not a chip"))
}

/// Writes the lists Wiremod rebuilds an E2's ports from on paste. It does so
/// before it applies the dupe's wires and indexes them without checking
/// (MakeWireExpression2 in gmod_wire_expression2/init.lua), so a chip with
/// no lists is a Lua error, and one whose lists lag behind its @inputs
/// loses every wire to an input that is not listed yet.
fn write_e2_port_lists(dupe: &mut Dupe, et: usize, code: &str) {
    let declared = crate::ports::of_chip(code);
    for (key, side) in [("_inputs", &declared.inputs), ("_outputs", &declared.outputs)] {
        let names = dupe.new_array();
        let kinds = dupe.new_array();
        if let Node::Array(items) = &mut dupe.arena[names] {
            items.extend(side.iter().map(|port| s(&port.name)));
        }
        if let Node::Array(items) = &mut dupe.arena[kinds] {
            items.extend(side.iter().map(|port| s(port.kind.name())));
        }
        let both = dupe.new_array();
        if let Node::Array(items) = &mut dupe.arena[both] {
            items.extend([Value::Table(names), Value::Table(kinds)]);
        }
        dupe.set(et, key, Value::Table(both));
    }
    if dupe.get_table(et, "inc_files").is_none() {
        let none = dupe.new_array();
        dupe.set(et, "inc_files", Value::Table(none));
    }
    if dupe.get_table(et, "_vars").is_none() {
        let none = dupe.new_table();
        dupe.set(et, "_vars", Value::Table(none));
    }
}

pub fn chip_import(dupe: &mut Dupe, index: f64, chip: Chip) -> Result<(), String> {
    let et = transform::entity_table(dupe, index).ok_or_else(|| format!("no entity {index}"))?;
    match chip {
        Chip::E2 { name, code } => {
            dupe.set(et, "_original", Value::Str(e2_encode(&code)));
            write_e2_port_lists(dupe, et, &code);
            // The name is whatever the @name directive says; keep it in step.
            let directive = code
                .lines()
                .find_map(|l| l.trim().strip_prefix("@name ").map(|n| n.trim().to_owned()));
            let final_name = directive.unwrap_or(name);
            if !final_name.is_empty() {
                dupe.set(et, "_name", s(&final_name));
            }
            Ok(())
        }
        Chip::Starfall { mainfile, files } => {
            let mods = dupe.get_table(et, "EntityMods").ok_or("starfall chip has no EntityMods")?;
            let info = dupe.get_table(mods, "SFDupeInfo").ok_or("no SFDupeInfo")?;
            let sf = dupe.get_table(info, "starfall").ok_or("no starfall table")?;
            dupe.set(sf, "files", Value::Str(sf_pack(&files)?));
            dupe.set(sf, "mainfile", s(&mainfile));
            Ok(())
        }
    }
}

/// Which of a model's local axes is its axle: the thinnest extent. For a
/// tire that is unambiguous. Returns a unit local axis.
pub fn thinnest_axis(half_extents: Vec3) -> Vec3 {
    let (x, y, z) = half_extents;
    if x <= y && x <= z {
        (1.0, 0.0, 0.0)
    } else if y <= x && y <= z {
        (0.0, 1.0, 0.0)
    } else {
        (0.0, 0.0, 1.0)
    }
}

/// The angle that takes local axis `from` onto world direction `to`, with
/// no roll about it. Built as a rotation about the cross product.
pub fn angle_aligning(from: Vec3, to: Vec3) -> Vec3 {
    let n = |v: Vec3| {
        let l = (v.0 * v.0 + v.1 * v.1 + v.2 * v.2).sqrt().max(1e-12);
        (v.0 / l, v.1 / l, v.2 / l)
    };
    let (f, t) = (n(from), n(to));
    let dot = (f.0 * t.0 + f.1 * t.1 + f.2 * t.2).clamp(-1.0, 1.0);
    let cross = (f.1 * t.2 - f.2 * t.1, f.2 * t.0 - f.0 * t.2, f.0 * t.1 - f.1 * t.0);
    let sin = (cross.0 * cross.0 + cross.1 * cross.1 + cross.2 * cross.2).sqrt();
    if sin < 1e-9 {
        // Parallel or anti-parallel: pick any perpendicular axis for a 180.
        if dot > 0.0 {
            return (0.0, 0.0, 0.0);
        }
        let axis = if f.2.abs() < 0.9 { (0.0, 0.0, 1.0) } else { (1.0, 0.0, 0.0) };
        return transform::rotate_angle_about((0.0, 0.0, 0.0), axis, std::f64::consts::PI);
    }
    transform::rotate_angle_about((0.0, 0.0, 0.0), cross, sin.atan2(dot))
}

/// Sets a field by dotted path on an entity, creating intermediate tables.
/// `DT.BrakeEngagement` reaches the networked-var table that the duplicator
/// restores with RestoreNetworkVars — where the AIO controller's settings
/// actually live. A bare key sets a top-level field, which is right for
/// Register args like a turret's MaxSpeed.
pub fn put_path(dupe: &mut Dupe, entity: f64, path: &str, value: Value) -> Result<(), String> {
    let mut here = transform::entity_table(dupe, entity).ok_or_else(|| format!("no entity {entity}"))?;
    let segs: Vec<&str> = path.split('.').filter(|x| !x.is_empty()).collect();
    let (last, parents) = segs.split_last().ok_or("empty path")?;
    for seg in parents {
        here = match dupe.get_table(here, seg) {
            Some(t) => t,
            None => {
                let t = dupe.new_table();
                dupe.set(here, seg, Value::Table(t));
                t
            }
        };
    }
    dupe.set(here, last, value);
    Ok(())
}

/// Whether an entity index names something in the dupe. Used by the CLI
/// before every operation so a typo fails with a clear message.
pub fn exists(dupe: &Dupe, index: f64) -> bool {
    transform::entity_table(dupe, index).is_some()
}

#[allow(dead_code)]
fn _unused(_: fn(&Value, &str) -> bool, _: fn(&Value) -> Option<usize>) {
    let _ = (key_matches, table_index);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::fixture;
    use crate::refs;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6 && (a.2 - b.2).abs() < 1e-6
    }

    #[test]
    fn world_to_local_inverts_local_to_world() {
        let pos = (10.0, 20.0, 30.0);
        let ang = (15.0, 60.0, -20.0);
        let local = (3.0, -4.0, 5.0);
        let world = add(pos, transform::rotate_vec(ang, local));
        assert!(close(world_to_local(world, pos, ang), local));
    }

    #[test]
    fn an_axis_stores_the_hinge_in_each_local_frame_and_a_second_point() {
        let mut d = fixture();
        // Wheel 14 at (0,60,0), base 10 at origin; hinge at the wheel centre
        // along world Y.
        add_axis(
            &mut d,
            10.0,
            14.0,
            &AxisSpec {
                point: (0.0, 60.0, 0.0),
                axis: (0.0, 1.0, 0.0),
                friction: 0.0,
                forcelimit: 0.0,
                torquelimit: 0.0,
                nocollide: true,
            },
        )
        .unwrap();
        let root = d.root_table().unwrap();
        let cons = d.get_table(root, "Constraints").unwrap();
        let last = match &d.arena[cons] {
            Node::Array(v) => table_index(v.last().unwrap()).unwrap(),
            _ => panic!(),
        };
        assert_eq!(d.get(last, "LPos1"), Some(&Value::Vector(0.0, 60.0, 0.0)));
        assert_eq!(d.get(last, "LPos2"), Some(&Value::Vector(0.0, 0.0, 0.0)));
        // Second point one unit further along +Y in the base's frame.
        assert_eq!(d.get(last, "LocalAxis"), Some(&Value::Vector(0.0, 61.0, 0.0)));
        assert!(refs::dangling(&d).is_empty());
        // And it survives the codec.
        let body = d.to_body().unwrap();
        let (again, _) = Dupe::from_body(&body).unwrap();
        assert!(refs::dangling(&again).is_empty());
    }

    /// GMod's constraint functions do `flag > 0` on nocollide, forcelimit,
    /// torquelimit and friction. A Lua boolean there errors on paste — which
    /// is exactly what four axis constraints did in the first ACF car.
    #[test]
    fn constraint_flags_are_numbers_never_booleans() {
        let mut d = fixture();
        add_axis(&mut d, 10.0, 14.0, &AxisSpec { point: (0.0, 60.0, 0.0), axis: (0.0, 1.0, 0.0), friction: 0.0, forcelimit: 0.0, torquelimit: 0.0, nocollide: true }).unwrap();
        add_weld(&mut d, 18.0, 10.0, 0.0, true).unwrap();
        add_ballsocket(&mut d, 10.0, 14.0, (0.0, 60.0, 0.0), 0.0, 0.0, true).unwrap();
        add_nocollide(&mut d, 10.0, 14.0).unwrap();
        let root = d.root_table().unwrap();
        let cons = d.get_table(root, "Constraints").unwrap();
        let Node::Array(items) = &d.arena[cons] else { panic!() };
        for it in items {
            let t = table_index(it).unwrap();
            for key in ["nocollide", "forcelimit", "torquelimit", "friction", "disableOnRemove"] {
                if let Some(v) = d.get(t, key) {
                    assert!(matches!(v, Value::Number(_)), "{key} is {v:?}");
                }
            }
        }
    }

    #[test]
    fn wheel_axis_and_rotation_lock_match_the_reference_shape() {
        let mut d = fixture();
        add_wheel_axis(&mut d, 14.0, 10.0, (0.0, 1.0, 0.0), true).unwrap();
        add_rotation_lock(&mut d, 14.0, 18.0, Lock::Y).unwrap();
        let root = d.root_table().unwrap();
        let cons = d.get_table(root, "Constraints").unwrap();
        let Node::Array(items) = &d.arena[cons] else { panic!() };
        let axis = table_index(&items[items.len() - 2]).unwrap();
        let lock = table_index(&items[items.len() - 1]).unwrap();
        // Axis: Ent1 is the wheel at its own centre, LPos2 is the wheel in
        // the base's frame, LocalAxis a unit axle.
        let ends = d.get_table(axis, "Entity").unwrap();
        let Node::Array(e) = &d.arena[ends] else { panic!() };
        let e1 = table_index(&e[0]).unwrap();
        assert_eq!(d.get(e1, "Index"), Some(&Value::Number(14.0)));
        assert_eq!(d.get(e1, "Bone"), Some(&Value::Number(0.0)));
        assert_eq!(d.get(axis, "LPos1"), Some(&Value::Vector(0.0, 0.0, 0.0)));
        assert_eq!(d.get(axis, "LPos2"), Some(&Value::Vector(0.0, 60.0, 0.0)));
        assert_eq!(d.get(axis, "LocalAxis"), Some(&Value::Vector(0.0, 1.0, 0.0)));
        // Lock: Y pinned with friction 50, X and Z free, rotation only.
        assert_eq!(d.get(lock, "ymin"), Some(&Value::Number(0.0)));
        assert_eq!(d.get(lock, "ymax"), Some(&Value::Number(0.0)));
        assert_eq!(d.get(lock, "xmin"), Some(&Value::Number(-180.0)));
        assert_eq!(d.get(lock, "yfric"), Some(&Value::Number(50.0)));
        assert_eq!(d.get(lock, "xfric"), Some(&Value::Number(0.0)));
        assert_eq!(d.get(lock, "onlyrotation"), Some(&Value::Number(1.0)));
        assert!(refs::dangling(&d).is_empty());
    }

    #[test]
    fn a_spawned_part_sheds_links_to_what_it_left_behind() {
        let mut d = fixture();
        let template = fixture();
        // The gun in the template links crate 12 and turret 13. Spawned alone
        // it must not carry those indices into the destination.
        let new = spawn_from(&mut d, &template, 11.0, (500.0, 0.0, 0.0), None).unwrap();
        let et = transform::entity_table(&d, new).unwrap();
        let mods = d.get_table(et, "EntityMods").unwrap();
        for name in ["ACFCrates", "ACFTurret"] {
            if let Some(arr) = d.get_table(mods, name) {
                let Node::Array(v) = &d.arena[arr] else { continue };
                assert!(v.is_empty(), "{name} still has {v:?}");
            }
        }
        assert!(refs::dangling(&d).is_empty(), "{:?}", refs::dangling(&d));
    }

    #[test]
    fn a_spawned_part_that_was_hidden_becomes_visible() {
        let mut template = fixture();
        let et = transform::entity_table(&template, 14.0).unwrap();
        let mods = template.get_table(et, "EntityMods").unwrap();
        let colour = template.new_table();
        let c = template.new_table();
        for (k, v) in [("r", 10.0), ("g", 20.0), ("b", 30.0), ("a", 0.0)] {
            template.set(c, k, Value::Number(v));
        }
        template.set(colour, "Color", Value::Table(c));
        template.set(colour, "RenderMode", Value::Number(1.0));
        template.set(mods, "colour", Value::Table(colour));

        let mut d = fixture();
        let new = spawn_from(&mut d, &template, 14.0, (0.0, 0.0, 0.0), None).unwrap();
        let et = transform::entity_table(&d, new).unwrap();
        let mods = d.get_table(et, "EntityMods").unwrap();
        let colour = d.get_table(mods, "colour").unwrap();
        let c = d.get_table(colour, "Color").unwrap();
        assert_eq!(d.get_number(c, "a"), Some(255.0));
        assert_eq!(d.get_number(c, "r"), Some(10.0), "colour itself kept");
        assert_eq!(d.get_number(colour, "RenderMode"), Some(0.0));
    }

    #[test]
    fn put_path_reaches_into_dt_and_creates_it_when_missing() {
        let mut d = fixture();
        put_path(&mut d, 10.0, "DT.BrakeEngagement", Value::Number(1.0)).unwrap();
        put_path(&mut d, 10.0, "MaxSpeed", Value::Number(30.0)).unwrap();
        let et = transform::entity_table(&d, 10.0).unwrap();
        let dt = d.get_table(et, "DT").expect("DT created");
        assert_eq!(d.get_number(dt, "BrakeEngagement"), Some(1.0));
        assert_eq!(d.get_number(et, "MaxSpeed"), Some(30.0));
        // Second write to the same DT updates in place, no duplicate table.
        put_path(&mut d, 10.0, "DT.BrakeEngagement", Value::Number(0.0)).unwrap();
        assert_eq!(d.get_number(dt, "BrakeEngagement"), Some(0.0));
    }

    #[test]
    fn a_sprung_wheel_writes_the_vdc_set() {
        let mut d = fixture();
        let before = {
            let root = d.root_table().unwrap();
            let cons = d.get_table(root, "Constraints").unwrap();
            match &d.arena[cons] { Node::Array(v) => v.len(), _ => 0 }
        };
        add_sprung_wheel(&mut d, 14.0, 10.0).unwrap();
        let root = d.root_table().unwrap();
        let cons = d.get_table(root, "Constraints").unwrap();
        let Node::Array(v) = &d.arena[cons] else { panic!() };
        let new: Vec<usize> = v[before..].iter().map(|x| table_index(x).unwrap()).collect();
        let types: Vec<String> = new.iter().map(|t| match d.get(*t, "Type") { Some(Value::Str(b)) => String::from_utf8_lossy(b).into_owned(), _ => "?".into() }).collect();
        assert_eq!(types, ["Elastic", "Rope", "Rope", "Rope", "AdvBallsocket", "NoCollide"]);
        // Wheel 14 at (0,60,0), base at origin: spring anchor 17 up.
        assert_eq!(d.get(new[0], "LPos2"), Some(&Value::Vector(0.0, 60.0, 17.0)));
        assert_eq!(d.get_number(new[3], "addlength"), Some(14.0));
        assert!(refs::dangling(&d).is_empty());
        let body = d.to_body().unwrap();
        assert!(Dupe::from_body(&body).is_ok());
    }

    #[test]
    fn pair_lock_is_three_sockets_and_spherical_mirrors_mass() {
        let mut d = fixture();
        add_pair_lock(&mut d, 14.0, 18.0).unwrap();
        let root = d.root_table().unwrap();
        let cons = d.get_table(root, "Constraints").unwrap();
        let Node::Array(v) = &d.arena[cons] else { panic!() };
        let last3: Vec<usize> = v[v.len() - 3..].iter().map(|x| table_index(x).unwrap()).collect();
        let pinned: Vec<f64> = last3.iter().map(|t| {
            [("xfric", "x"), ("yfric", "y"), ("zfric", "z")].iter().filter(|(k, _)| d.get_number(*t, k) == Some(50.0)).count() as f64
        }).collect();
        assert_eq!(pinned, [1.0, 1.0, 1.0], "each socket pins exactly one axis");

        make_spherical(&mut d, 14.0, 30.25).unwrap();
        let et = transform::entity_table(&d, 14.0).unwrap();
        let mods = d.get_table(et, "EntityMods").unwrap();
        let t = d.get_table(mods, "MakeSphericalCollisions").unwrap();
        assert_eq!(d.get_number(t, "radius"), Some(30.25));
        assert_eq!(d.get_number(t, "mass"), Some(100.0), "no mass modifier → 100");
    }

    #[test]
    fn a_hydraulic_wheel_needs_a_controller_and_dies_with_it() {
        let mut d = fixture();
        assert!(add_hydraulic_wheel(&mut d, 14.0, 10.0, 16.0).is_err(), "button is not a controller");
        // Turn the button into a hydraulic controller for the test.
        let et = transform::entity_table(&d, 16.0).unwrap();
        d.set(et, "Class", s("gmod_wire_hydraulic"));
        add_hydraulic_wheel(&mut d, 14.0, 10.0, 16.0).unwrap();
        let hits = refs::scan(&d);
        assert!(hits.iter().any(|h| h.path.ends_with(".MyCrtl") && h.known && h.value == 16.0));
        assert!(refs::dangling(&d).is_empty());
        let r = d.delete_entity(16.0).unwrap();
        assert!(r.removed_constraints >= 1, "hydraulic should go with its controller");
        assert!(refs::dangling(&d).is_empty());
    }

    #[test]
    fn a_turret_links_a_gyro() {
        let mut d = fixture();
        let root = d.root_table().unwrap();
        let ents = d.get_table(root, "Entities").unwrap();
        let et = d.new_table();
        d.set(et, "Class", s("acf_turret_gyro"));
        d.set(et, "Model", s("models/acf/core/t_gyro.mdl"));
        let physics = d.new_table();
        let bone = d.new_table();
        d.set(bone, "Pos", Value::Vector(0.0, 0.0, 20.0));
        d.set(bone, "Angle", Value::Angle(0.0, 0.0, 0.0));
        d.set(physics, "0", Value::Table(bone));
        if let Node::Table(e) = &mut d.arena[physics] { e[0].0 = Value::Number(0.0); }
        d.set(et, "PhysicsObjects", Value::Table(physics));
        if let Node::Table(e) = &mut d.arena[ents] { e.push((Value::Number(30.0), Value::Table(et))); }
        let r = add_link(&mut d, 13.0, 30.0).unwrap();
        assert_eq!((r.owner, r.modifier), (13.0, "ACFGyro"));
    }

    #[test]
    fn a_wire_lands_on_the_target_and_the_registry_sees_it() {
        let mut d = fixture();
        add_wire(&mut d, 16.0, "Out", 17.0, "Length").unwrap();
        let hits = refs::scan(&d);
        assert!(hits.iter().any(|h| h.path.ends_with("Wires.Length.Src") && h.value == 16.0 && h.known));
        assert!(refs::dangling(&d).is_empty());
        let r = d.delete_entity(16.0).unwrap();
        assert!(r.pruned.wires >= 2, "both ports on the lamp fed by 16 should go");
    }

    #[test]
    fn ammo_type_is_validated_and_armour_mass_matches_the_hand_calculation() {
        let mut d = fixture();
        assert!(set_ammo(&mut d, 12.0, "apfsds", Some((24.0, 24.0, 24.0))).is_ok());
        assert!(set_ammo(&mut d, 12.0, "nope", None).is_err());
        assert!(set_ammo(&mut d, 11.0, "AP", None).is_err(), "gun is not a crate");
        let et = transform::entity_table(&d, 12.0).unwrap();
        assert_eq!(d.get(et, "AmmoType"), Some(&Value::Str(b"APFSDS".to_vec())));
        // The 42x336 superthin side plate at 159.83mm / -17.59 came to 10,962 kg.
        let half = sprops_half_extents("models/sprops/rectangles_superthin/size_4_5/rect_42x336.mdl").unwrap();
        let m = armour_mass(box_surface(half), 159.83, -17.59);
        assert!((m - 10962.0).abs() < 15.0, "{m}");
    }

    #[test]
    fn track_links_are_positional_and_registered() {
        let mut d = fixture();
        set_track_links(&mut d, 15.0, 10.0, &[14.0, 18.0]).unwrap();
        let hits: Vec<_> = refs::scan(&d).into_iter().filter(|h| h.path.contains("tanktracktool.links")).collect();
        assert_eq!(hits.len(), 3);
        assert!(hits.iter().all(|h| h.known));
        assert!(refs::dangling(&d).is_empty());
        let r = d.delete_entity(18.0).unwrap();
        assert_eq!(r.pruned.tracks, 1);
        assert!(set_track_links(&mut d, 11.0, 10.0, &[]).is_err(), "gun is not a track");
    }

    #[test]
    fn door_submaterial_and_keypad_write_the_gift_shapes() {
        let mut d = fixture();
        set_fading_door(&mut d, 18.0, 111.0, "sprites/heatwave", true).unwrap();
        set_submaterial(&mut d, 18.0, 2, "models/effects/comball_tape").unwrap();
        let et = transform::entity_table(&d, 18.0).unwrap();
        let mods = d.get_table(et, "EntityMods").unwrap();
        let door = d.get_table(mods, "Fading Door").expect("modifier named with a space");
        assert_eq!(d.get_number(door, "key"), Some(111.0));
        assert_eq!(d.get(door, "toggle"), Some(&Value::Bool(true)));
        let sub = d.get_table(mods, "submaterial").unwrap();
        assert!(matches!(d.get(sub, "SubMaterialOverride_2"), Some(Value::Str(_))));
        // A keypad needs the right class.
        assert!(set_keypad(&mut d, 18.0, Some(1337.0), false, 111.0).is_err());
        let et16 = transform::entity_table(&d, 16.0).unwrap();
        d.set(et16, "Class", s("keypad"));
        set_keypad(&mut d, 16.0, Some(1337.0), false, 111.0).unwrap();
        let mods = d.get_table(et16, "EntityMods").unwrap();
        let kp = d.get_table(mods, "keypad_password_passthrough").unwrap();
        assert_eq!(d.get_number(kp, "Password"), Some(1337.0));
        assert_eq!(d.get_number(kp, "KeyGranted"), Some(111.0), "matches the door's key");
        assert!(d.get(kp, "OutputOn").is_none(), "plain keypad has no wire outputs");
        // No entity references anywhere in these three.
        assert!(refs::dangling(&d).is_empty());
    }

    #[test]
    fn resize_writes_bonemanip_with_numeric_bone_keys() {
        let mut d = fixture();
        set_resize(&mut d, 18.0, (2.0, 2.0, 2.0), 2).unwrap();
        let et = transform::entity_table(&d, 18.0).unwrap();
        let bm = d.get_table(et, "BoneManip").unwrap();
        let Node::Table(e) = &d.arena[bm] else { panic!() };
        assert_eq!(e.len(), 2);
        assert!(matches!(e[0].0, Value::Number(n) if n == 0.0));
        let b0 = table_index(&e[0].1).unwrap();
        assert_eq!(d.get(b0, "s"), Some(&Value::Vector(2.0, 2.0, 2.0)));
        assert!(refs::dangling(&d).is_empty());
    }

    #[test]
    fn armour_clamps_and_drops_a_conflicting_mass() {
        let mut d = fixture();
        let et = transform::entity_table(&d, 14.0).unwrap();
        let mods = d.get_table(et, "EntityMods").unwrap();
        let m = d.new_table();
        d.set(m, "Mass", Value::Number(500.0));
        d.set(mods, "mass", Value::Table(m));
        set_armour(&mut d, 14.0, 9000.0, 200.0).unwrap();
        let a = d.get_table(mods, "ACF_Armor").unwrap();
        assert_eq!(d.get_number(a, "Thickness"), Some(5000.0));
        assert_eq!(d.get_number(a, "Ductility"), Some(80.0));
        assert!(d.get_table(mods, "mass").is_none(), "mass removed so thickness applies");
        assert!(set_armour(&mut d, 11.0, 10.0, 0.0).is_err(), "not on a gun");
    }

    #[test]
    fn catalogue_spawns_carry_their_class_key_and_clamp_a_calibre() {
        let mut d = fixture();
        let gun = crate::catalog::find("C").unwrap();
        let g = spawn_catalog(&mut d, gun, (0.0, 0.0, 50.0), (0.0, 90.0, 0.0), &[("Caliber", 999.0)]).unwrap();
        let et = transform::entity_table(&d, g).unwrap();
        assert_eq!(d.get(et, "Weapon"), Some(&Value::Str(b"C".to_vec())));
        assert_eq!(d.get_number(et, "Caliber"), Some(170.0), "clamped to the class range");
        let eng = crate::catalog::find("6.5-I6").unwrap();
        let e = spawn_catalog(&mut d, eng, (0.0, -50.0, 0.0), (0.0, -90.0, 0.0), &[]).unwrap();
        let eet = transform::entity_table(&d, e).unwrap();
        assert_eq!(d.get(eet, "Engine"), Some(&Value::Str(b"6.5-I6".to_vec())));
        // The result validates and links by the registry.
        assert!(crate::extras::validate(&d).is_empty());
        let plate = crate::catalog::in_category("Baseplates")[0];
        let b = spawn_catalog(&mut d, plate, (0.0, 0.0, 0.0), (0.0, 90.0, 0.0), &[("Width", 60.0)]).unwrap();
        assert!(add_link(&mut d, g, 12.0).is_ok());
        let bet = transform::entity_table(&d, b).unwrap();
        let ud = d.get_table(bet, "ACF_UserData").unwrap();
        assert_eq!(d.get_number(ud, "Width"), Some(60.0));
    }

    #[test]
    fn a_ballsocket_uses_ent2s_frame() {
        let mut d = fixture();
        // Move the wheel so its frame differs, then socket at a world point.
        transform::set_entity_transform(&mut d, 14.0, Some((0.0, 60.0, 0.0)), Some((0.0, 90.0, 0.0))).unwrap();
        add_ballsocket(&mut d, 10.0, 14.0, (10.0, 60.0, 0.0), 0.0, 0.0, false).unwrap();
        let root = d.root_table().unwrap();
        let cons = d.get_table(root, "Constraints").unwrap();
        let last = match &d.arena[cons] {
            Node::Array(v) => table_index(v.last().unwrap()).unwrap(),
            _ => panic!(),
        };
        // World (10,60,0) relative to the wheel at (0,60,0) yawed 90° is
        // local (0,-10,0): +X world is -Y in a frame turned left.
        match d.get(last, "LPos") {
            Some(Value::Vector(x, y, z)) => assert!(close((*x, *y, *z), (0.0, -10.0, 0.0)), "{x} {y} {z}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn links_follow_acf_rules_in_both_directions() {
        let mut d = fixture();
        // gearbox doesn't exist in the fixture; use gun<->turret which does.
        // Gun 11 already links crate 12; link it to turret 13 (already there
        // too) and confirm no duplicate, then reverse-order a new pair.
        let r = add_link(&mut d, 13.0, 11.0).unwrap();
        assert_eq!((r.owner, r.target, r.modifier), (11.0, 13.0, "ACFTurret"));
        let gun = transform::entity_table(&d, 11.0).unwrap();
        let gm = d.get_table(gun, "EntityMods").unwrap();
        let turrets = d.get_table(gm, "ACFTurret").unwrap();
        assert!(matches!(&d.arena[turrets], Node::Array(v) if v.len() == 1), "duplicated link");
        // A pair ACF doesn't allow.
        assert!(add_link(&mut d, 11.0, 14.0).is_err(), "gun to wheel should be refused");
        // Too far.
        transform::set_entity_transform(&mut d, 12.0, Some((2000.0, 0.0, 0.0)), None).unwrap();
        assert!(add_link(&mut d, 11.0, 12.0).unwrap_err().contains("650"));
    }

    #[test]
    fn spawning_from_a_template_lands_at_the_requested_pose() {
        let mut d = fixture();
        let template = fixture();
        let before = d.list_entities().len();
        let new = spawn_from(&mut d, &template, 14.0, (100.0, 200.0, 300.0), Some((0.0, 45.0, 0.0))).unwrap();
        assert_eq!(d.list_entities().len(), before + 1);
        let (p, a) = transform::entity_transform(&d, new).unwrap();
        assert!(close(p, (100.0, 200.0, 300.0)), "{p:?}");
        assert!(close(a, (0.0, 45.0, 0.0)), "{a:?}");
        assert!(refs::dangling(&d).is_empty());
    }

    #[test]
    fn aligning_an_axle_puts_the_thin_axis_where_asked() {
        // A tire thin along its local X.
        let axle = thinnest_axis((4.0, 20.0, 20.0));
        assert_eq!(axle, (1.0, 0.0, 0.0));
        let ang = angle_aligning(axle, (0.0, 1.0, 0.0));
        let got = transform::rotate_vec(ang, axle);
        assert!(close(got, (0.0, 1.0, 0.0)), "{got:?}");
        // And one already aligned stays put.
        let none = angle_aligning((0.0, 1.0, 0.0), (0.0, 1.0, 0.0));
        assert!(close(none, (0.0, 0.0, 0.0)));
        // Anti-parallel still lands on the target line.
        let flip = angle_aligning((0.0, 1.0, 0.0), (0.0, -1.0, 0.0));
        let got = transform::rotate_vec(flip, (0.0, 1.0, 0.0));
        assert!(close(got, (0.0, -1.0, 0.0)), "{got:?}");
    }

    #[test]
    fn e2_code_round_trips_through_the_byte_substitution() {
        let code = "@name Test\nprint(\"hello\")\n";
        let enc = e2_encode(code);
        assert!(!enc.contains(&b'\n') && !enc.contains(&b'"'));
        assert_eq!(e2_decode(&enc), code);
    }

    #[test]
    fn starfall_files_round_trip_through_the_pack() {
        let files = vec![
            ("main.txt".to_owned(), "print(1)\n".to_owned()),
            ("lib/util.txt".to_owned(), "return {}".to_owned()),
        ];
        let packed = sf_pack(&files).unwrap();
        let back = sf_unpack(&packed).unwrap();
        assert_eq!(back, files);
    }

    #[test]
    fn e2_export_import_on_a_synthetic_chip() {
        let mut d = fixture();
        let root = d.root_table().unwrap();
        let ents = d.get_table(root, "Entities").unwrap();
        // A minimal E2 entity.
        let et = d.new_table();
        d.set(et, "Class", s("gmod_wire_expression2"));
        d.set(et, "Model", s("models/beer/wiremod/gate_e2.mdl"));
        d.set(et, "_original", Value::Str(e2_encode("@name Old\n")));
        d.set(et, "_name", s("Old"));
        let physics = d.new_table();
        let bone = d.new_table();
        d.set(bone, "Pos", Value::Vector(0.0, 0.0, 0.0));
        d.set(bone, "Angle", Value::Angle(0.0, 0.0, 0.0));
        d.set(physics, "0", Value::Table(bone));
        if let Node::Table(e) = &mut d.arena[physics] {
            e[0].0 = Value::Number(0.0);
        }
        d.set(et, "PhysicsObjects", Value::Table(physics));
        if let Node::Table(e) = &mut d.arena[ents] {
            e.push((Value::Number(99.0), Value::Table(et)));
        }

        match chip_export(&d, 99.0).unwrap() {
            Chip::E2 { name, code } => {
                assert_eq!(name, "Old");
                assert_eq!(code, "@name Old\n");
            }
            _ => panic!(),
        }
        chip_import(&mut d, 99.0, Chip::E2 { name: String::new(), code: "@name New\nA = 1\n".into() }).unwrap();
        match chip_export(&d, 99.0).unwrap() {
            Chip::E2 { name, code } => {
                assert_eq!(name, "New", "name should follow the @name directive");
                assert_eq!(code, "@name New\nA = 1\n");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn a_new_dupe_with_a_baseplate_encodes_and_is_one_ad2_accepts() {
        let mut d = crate::buildfile::empty_dupe();
        let item = crate::catalog::find("GroundVehicle").expect("the ground vehicle baseplate");
        let size = [("Width", 96.0), ("Length", 200.0), ("Thickness", 2.0)];
        let index = spawn_catalog(&mut d, item, (0.0, 0.0, 0.0), (0.0, 90.0, 0.0), &size).unwrap();
        crate::extras::repair_head(&mut d);
        let problems = crate::extras::validate(&d);
        assert!(problems.is_empty(), "{problems:?}");
        let body = d.to_body().unwrap();
        let (back, used) = Dupe::from_body(&body).unwrap();
        assert_eq!(used, body.len());
        assert_eq!(back.list_entities().len(), 1);
        assert_eq!(crate::scale::model_scale(&d, index, (6.0, 6.0, 6.0)).0, 200.0 / 12.0);

        let empty = crate::buildfile::empty_dupe();
        let body = empty.to_body().unwrap();
        assert!(Dupe::from_body(&body).is_ok(), "an empty dupe still opens");
    }

    #[test]
    fn an_e2_always_carries_the_port_lists_wiremod_reads_before_it_applies_wires() {
        let lists = |d: &Dupe, et: usize, key: &str| -> Vec<Vec<String>> {
            let outer = d.get_table(et, key).unwrap_or_else(|| panic!("{key} is missing"));
            let Node::Array(halves) = &d.arena[outer] else { panic!("{key} is not a list") };
            halves
                .iter()
                .map(|half| match &d.arena[crate::value::table_index(half).unwrap()] {
                    Node::Array(items) => items.iter().map(|v| crate::value::short(v).trim_matches('"').to_owned()).collect(),
                    Node::Table(_) => panic!("{key} holds a keyed table"),
                })
                .collect()
        };
        // A chip from the catalogue: MakeWireExpression2 indexes _inputs[1]
        // without checking, so a chip with no lists is a Lua error on paste.
        let mut d = crate::buildfile::empty_dupe();
        let chip = spawn_catalog(&mut d, crate::catalog::find("@name New chip").unwrap(), (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), &[]).unwrap();
        let et = transform::entity_table(&d, chip).unwrap();
        assert_eq!(lists(&d, et, "_inputs"), [Vec::<String>::new(), Vec::new()]);
        assert_eq!(lists(&d, et, "_outputs"), [Vec::<String>::new(), Vec::new()]);
        assert!(d.get_table(et, "inc_files").is_some() && d.get_table(et, "_vars").is_some());

        // Editing the code: the lists follow the directives, or a wire to a
        // new input would find nothing to attach to when the dupe is pasted.
        let code = "@name Aim\n@inputs Pod:wirelink [Target Home]:vector\n@outputs Fire Aim:angle\nFire = 1\n";
        chip_import(&mut d, chip, Chip::E2 { name: String::new(), code: code.into() }).unwrap();
        assert_eq!(lists(&d, et, "_inputs"), [vec!["Pod", "Target", "Home"], vec!["WIRELINK", "VECTOR", "VECTOR"]]);
        assert_eq!(lists(&d, et, "_outputs"), [vec!["Fire", "Aim"], vec!["NORMAL", "ANGLE"]]);
    }

    #[test]
    fn a_wiremod_part_spawns_with_its_tools_default_options_and_a_gate_with_its_action() {
        let mut d = crate::buildfile::empty_dupe();
        let ranger = crate::catalog::ITEMS.iter().find(|item| item.entity == "gmod_wire_ranger").expect("the ranger is in the catalogue");
        let index = spawn_catalog(&mut d, ranger, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), &[]).unwrap();
        let et = transform::entity_table(&d, index).unwrap();
        assert!(d.get_number(et, "range").is_some_and(|range| range > 0.0), "the tool's default range");
        assert!(matches!(d.get(et, "out_dist"), Some(Value::Bool(_))), "its output switches are there to be flipped");
        let ports = crate::ports::of_entity(&d, index).unwrap();
        assert!(ports.complete && ports.output("RangerData").is_some());

        let adder = crate::catalog::ITEMS.iter().find(|item| item.entity == "gmod_wire_gate" && item.value == "+").expect("the add gate");
        let index = spawn_catalog(&mut d, adder, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), &[]).unwrap();
        let et = transform::entity_table(&d, index).unwrap();
        assert_eq!(d.get(et, "noclip"), Some(&Value::Bool(true)));
        let ports = crate::ports::of_entity(&d, index).unwrap();
        assert!(ports.complete && ports.input("A").is_some() && ports.output("Out").is_some());
        assert!(crate::extras::validate(&d).is_empty() || crate::extras::validate(&d).iter().all(|problem| problem.contains("HeadEnt")));
    }
}
