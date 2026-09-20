//! The Primitive addon's shapes (workshop 2840295308). A `primitive_shape`
//! or `primitive_airfoil` has no model worth drawing: its mesh is generated
//! in game from the numbers in its `DT` table, by
//! `lua/primitive/core/construct.lua`. This is that file's geometry, shape
//! for shape and index for index, so the editor draws what the game builds.
//! Indices are 1-based here as they are there; winding is counter-clockwise
//! seen from outside. A shape this file does not know is drawn as the box
//! of its `PrimSIZE`, which is at least the right place and size.

use std::collections::HashMap;
use std::f64::consts::{PI, TAU};

use crate::dupe::Dupe;
use crate::models::MeshData;
use crate::transform::{self, Vec3};
use crate::value::Value;

pub const DEFAULT_MATERIAL: &str = "hunter/myplastic";

#[derive(Default, Clone)]
struct Simpleton {
    verts: Vec<Vec3>,
    index: Vec<usize>,
}

impl Simpleton {
    fn xyz(&mut self, x: f64, y: f64, z: f64) -> usize {
        self.verts.push((x, y, z));
        self.verts.len()
    }

    fn point(&mut self, at: Vec3) -> usize {
        self.xyz(at.0, at.1, at.2)
    }

    fn tri(&mut self, a: usize, b: usize, c: usize) {
        self.index.extend([a, b, c]);
    }

    fn face(&mut self, corners: &[usize]) {
        for pair in corners[1..].windows(2) {
            self.tri(corners[0], pair[0], pair[1]);
        }
    }
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    (a.0 + b.0, a.1 + b.1, a.2 + b.2)
}
fn mul(a: Vec3, by: f64) -> Vec3 {
    (a.0 * by, a.1 * by, a.2 * by)
}
fn dot(a: Vec3, b: Vec3) -> f64 {
    a.0 * b.0 + a.1 * b.1 + a.2 * b.2
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    (a.1 * b.2 - a.2 * b.1, a.2 * b.0 - a.0 * b.2, a.0 * b.1 - a.1 * b.0)
}
fn lerp(a: Vec3, b: Vec3, along: f64) -> Vec3 {
    add(mul(a, 1.0 - along), mul(b, along))
}

/// The shape's numbers, read from `DT` with the addon's defaults.
struct Numbers<'a> {
    dupe: &'a Dupe,
    table: usize,
}

impl Numbers<'_> {
    fn number(&self, key: &str, default: f64) -> f64 {
        match self.dupe.get(self.table, key) {
            Some(Value::Number(value)) => *value,
            Some(Value::Bool(value)) => *value as u8 as f64,
            _ => default,
        }
    }

    fn flag(&self, key: &str) -> bool {
        self.number(key, 0.0) != 0.0
    }

    fn vector(&self, key: &str, default: Vec3) -> Vec3 {
        match self.dupe.get(self.table, key) {
            Some(Value::Vector(x, y, z)) | Some(Value::Angle(x, y, z)) => (*x, *y, *z),
            _ => default,
        }
    }

    fn half_size(&self) -> Vec3 {
        mul(self.vector("PrimSIZE", (1.0, 1.0, 1.0)), 0.5)
    }

    fn segments(&self) -> (usize, usize) {
        let most = (self.number("PrimMAXSEG", 32.0) as i64).clamp(3, 32) as usize;
        (most, (self.number("PrimNUMSEG", 32.0) as i64).clamp(1, most as i64) as usize)
    }
}

fn segment_hit(start: Vec3, finish: Vec3, at: Vec3, normal: Vec3) -> Vec3 {
    let direction = sub(finish, start);
    let along = dot(normal, direction);
    if along == 0.0 {
        return start;
    }
    add(start, mul(direction, dot(normal, sub(at, start)) / along))
}

/// What is left of `model` on the side of the plane its normal points to,
/// capped flat when `fill` (construct.lua, Bisect).
fn bisect(model: &Simpleton, at: Vec3, normal: Vec3, fill: bool) -> Option<Simpleton> {
    let kept: Vec<bool> = model.verts.iter().map(|vert| dot(normal, sub(*vert, at)) >= 1e-6).collect();
    if kept.iter().all(|kept| *kept) {
        return Some(model.clone());
    }
    if !kept.iter().any(|kept| *kept) {
        return None;
    }
    let mut above = Simpleton::default();
    let mut moved: HashMap<usize, usize> = HashMap::new();
    for (n, vert) in model.verts.iter().enumerate() {
        if kept[n] {
            moved.insert(n + 1, above.point(*vert));
        }
    }
    let mut rim: Vec<Vec3> = Vec::new();
    for triangle in model.index.chunks_exact(3) {
        let side = [kept[triangle[0] - 1], kept[triangle[1] - 1], kept[triangle[2] - 1]];
        if side[0] == side[1] && side[1] == side[2] {
            if side[0] {
                above.tri(moved[&triangle[0]], moved[&triangle[1]], moved[&triangle[2]]);
            }
            continue;
        }
        let lone = if side[0] != side[1] { if side[0] != side[2] { 0 } else { 1 } } else { 2 };
        let (before, after) = ((lone + 2) % 3, (lone + 1) % 3);
        let vert = |corner: usize| model.verts[triangle[corner] - 1];
        let hit_before = segment_hit(vert(lone), vert(before), at, normal);
        let hit_after = segment_hit(vert(lone), vert(after), at, normal);
        if side[lone] {
            let (a, b) = (above.point(hit_after), above.point(hit_before));
            above.tri(moved[&triangle[lone]], a, b);
            rim.push(hit_before);
        } else {
            let a = above.point(hit_before);
            above.tri(moved[&triangle[before]], a, moved[&triangle[after]]);
            let (b, c) = (above.point(hit_before), above.point(hit_after));
            above.tri(moved[&triangle[after]], b, c);
            rim.push(hit_after);
        }
    }
    if fill && !rim.is_empty() {
        let centre = mul(rim.iter().fold((0.0, 0.0, 0.0), |sum, point| add(sum, *point)), 1.0 / rim.len() as f64);
        let reference = sub(rim[0], centre);
        let sideways = cross(normal, reference);
        rim.sort_by(|a, b| {
            let angle = |point: &Vec3| -dot(sub(*point, centre), sideways).atan2(dot(sub(*point, centre), reference));
            angle(a).total_cmp(&angle(b))
        });
        let middle = above.point(centre);
        for n in 0..rim.len() {
            let (a, b) = (above.point(rim[n]), above.point(rim[(n + 1) % rim.len()]));
            above.tri(middle, a, b);
        }
    }
    Some(above)
}

fn cube(p: &Numbers) -> Simpleton {
    let (dx, dy, dz) = p.half_size();
    let (tx, ty) = (1.0 - p.number("PrimTX", 0.0), 1.0 - p.number("PrimTY", 0.0));
    let mut m = Simpleton::default();
    if tx == 0.0 && ty == 0.0 {
        for (x, y, z) in [(dx, -dy, -dz), (dx, dy, -dz), (-dx, -dy, -dz), (-dx, dy, -dz), (0.0, 0.0, dz)] {
            m.xyz(x, y, z);
        }
        for (a, b, c) in [(1, 2, 5), (2, 4, 5), (4, 3, 5), (3, 1, 5)] {
            m.tri(a, b, c);
        }
        m.face(&[3, 4, 2, 1]);
    } else {
        for (x, y, z) in [
            (dx, -dy, -dz),
            (dx, dy, -dz),
            (dx * tx, dy * ty, dz),
            (dx * tx, -dy * ty, dz),
            (-dx, -dy, -dz),
            (-dx, dy, -dz),
            (-dx * tx, dy * ty, dz),
            (-dx * tx, -dy * ty, dz),
        ] {
            m.xyz(x, y, z);
        }
        for face in [[1, 2, 3, 4], [2, 6, 7, 3], [6, 5, 8, 7], [5, 1, 4, 8], [4, 3, 7, 8], [5, 6, 2, 1]] {
            m.face(&face);
        }
    }
    m
}

fn wedge(p: &Numbers) -> Simpleton {
    let (dx, dy, dz) = p.half_size();
    let (tx, ty) = (p.number("PrimTX", 0.0) * 2.0, 1.0 - p.number("PrimTY", 0.0));
    let mut m = Simpleton::default();
    for (x, y, z) in [(dx, -dy, -dz), (dx, dy, -dz), (-dx, -dy, -dz), (-dx, dy, -dz)] {
        m.xyz(x, y, z);
    }
    if ty == 0.0 {
        m.xyz(-dx * tx, 0.0, dz);
        for (a, b, c) in [(1, 2, 5), (2, 4, 5), (4, 3, 5), (3, 1, 5)] {
            m.tri(a, b, c);
        }
    } else {
        m.xyz(-dx * tx, dy * ty, dz);
        m.xyz(-dx * tx, -dy * ty, dz);
        m.face(&[1, 2, 5, 6]);
        m.tri(2, 4, 5);
        m.face(&[4, 3, 6, 5]);
        m.tri(3, 1, 6);
    }
    m.face(&[3, 4, 2, 1]);
    m
}

fn wedge_corner(p: &Numbers) -> Simpleton {
    let (dx, dy, dz) = p.half_size();
    let (tx, ty) = (p.number("PrimTX", 0.0) * 2.0, p.number("PrimTY", 0.0) + 1.0);
    let mut m = Simpleton::default();
    for (x, y, z) in [(dx, dy, -dz), (-dx, -dy, -dz), (-dx, dy, -dz), (-dx * tx, dy * ty, dz)] {
        m.xyz(x, y, z);
    }
    for (a, b, c) in [(1, 3, 4), (2, 1, 4), (3, 2, 4), (1, 2, 3)] {
        m.tri(a, b, c);
    }
    m
}

fn pyramid(p: &Numbers) -> Simpleton {
    let (dx, dy, dz) = p.half_size();
    let (tx, ty) = (p.number("PrimTX", 0.0) * 2.0, p.number("PrimTY", 0.0) * 2.0);
    let mut m = Simpleton::default();
    for (x, y, z) in [(dx, -dy, -dz), (dx, dy, -dz), (-dx, -dy, -dz), (-dx, dy, -dz), (-dx * tx, dy * ty, dz)] {
        m.xyz(x, y, z);
    }
    for (a, b, c) in [(1, 2, 5), (2, 4, 5), (4, 3, 5), (3, 1, 5), (3, 4, 2), (3, 2, 1)] {
        m.tri(a, b, c);
    }
    m
}

fn cone(p: &Numbers) -> Simpleton {
    let (most, count) = p.segments();
    let (dx, dy, dz) = p.half_size();
    let (tx, ty) = (p.number("PrimTX", 0.0) * 2.0, p.number("PrimTY", 0.0) * 2.0);
    let mut m = Simpleton::default();
    for i in 0..=count {
        let a = -TAU * i as f64 / most as f64;
        m.xyz(a.sin() * dx, a.cos() * dy, -dz);
    }
    let rim = m.verts.len();
    let (base, tip) = (m.xyz(0.0, 0.0, -dz), m.xyz(-dx * tx, dy * ty, dz));
    for i in 1..rim {
        m.tri(i, i + 1, tip);
        m.tri(i, base, i + 1);
    }
    if count != most {
        m.tri(rim, base, tip);
        m.tri(base, 1, tip);
    }
    m
}

fn cylinder(p: &Numbers) -> Simpleton {
    let (most, count) = p.segments();
    let (dx, dy, dz) = p.half_size();
    let (tx, ty) = (1.0 - p.number("PrimTX", 0.0), 1.0 - p.number("PrimTY", 0.0));
    let pointed = tx == 0.0 && ty == 0.0;
    let mut m = Simpleton::default();
    for i in 0..=count {
        let a = -TAU * i as f64 / most as f64;
        m.xyz(a.sin() * dx, a.cos() * dy, -dz);
        if !pointed {
            m.xyz(a.sin() * dx * tx, a.cos() * dy * ty, dz);
        }
    }
    let rim = m.verts.len();
    let (bottom, top) = (m.xyz(0.0, 0.0, -dz), m.xyz(0.0, 0.0, dz));
    if pointed {
        for i in 1..rim {
            m.tri(i, i + 1, top);
            m.tri(i, bottom, i + 1);
        }
        if count != most {
            m.tri(rim, bottom, top);
            m.tri(bottom, 1, top);
        }
    } else {
        for i in (1..rim - 1).step_by(2) {
            m.face(&[i, i + 2, i + 3, i + 1]);
            m.tri(i, bottom, i + 2);
            m.tri(i + 1, i + 3, top);
        }
        if count != most {
            m.face(&[bottom, top, rim, rim - 1]);
            m.face(&[bottom, 1, 2, top]);
        }
    }
    m
}

fn tube(p: &Numbers) -> Simpleton {
    let (most, count) = p.segments();
    let (dx, dy, dz) = p.half_size();
    let wall = p.number("PrimDT", 1.0).min(dx).min(dy);
    if wall == dx || wall == dy {
        return cylinder(p);
    }
    let (tx, ty) = (1.0 - p.number("PrimTX", 0.0), 1.0 - p.number("PrimTY", 0.0));
    let pointed = tx == 0.0 && ty == 0.0;
    let mut m = Simpleton::default();
    for i in 0..=count {
        let a = -TAU * i as f64 / most as f64;
        let (s, c) = (a.sin(), a.cos());
        m.xyz(s * dx, c * dy, -dz);
        if !pointed {
            m.xyz(s * dx * tx, c * dy * ty, dz);
        }
        m.xyz(s * (dx - wall), c * (dy - wall), -dz);
        if !pointed {
            m.xyz(s * (dx - wall) * tx, c * (dy - wall) * ty, dz);
        }
    }
    let rim = m.verts.len();
    m.xyz(0.0, 0.0, -dz);
    let top = m.xyz(0.0, 0.0, dz);
    if pointed {
        for i in (1..rim - 1).step_by(2) {
            m.face(&[i + 3, i + 2, i, i + 1]);
            m.tri(i, i + 2, top);
            m.tri(i + 3, i + 1, top);
        }
        if count != most {
            let i = count * 2 + 1;
            m.tri(i, i + 1, top);
            m.tri(2, 1, top);
        }
    } else {
        for i in (1..rim - 3).step_by(4) {
            m.face(&[i, i + 2, i + 6, i + 4]);
            m.face(&[i + 4, i + 5, i + 1, i]);
            m.face(&[i + 2, i + 3, i + 7, i + 6]);
            m.face(&[i + 5, i + 7, i + 3, i + 1]);
        }
        if count != most {
            let i = count * 4 + 1;
            m.face(&[i + 2, i + 3, i + 1, i]);
            m.face(&[1, 2, 4, 3]);
        }
    }
    m
}

fn torus(p: &Numbers) -> Simpleton {
    let (most, count) = p.segments();
    let rings = (p.number("PrimSUBDIV", 16.0) as i64).clamp(3, 32) as usize;
    let (dx, dy, dz) = p.half_size();
    let wall = (p.number("PrimDT", 1.0) * 0.5).min(dx).min(dy);
    let mut m = Simpleton::default();
    for j in 0..=rings {
        for i in 0..=most {
            let (u, v) = (TAU * i as f64 / most as f64, TAU * j as f64 / rings as f64);
            m.xyz((dx + wall * v.cos()) * u.cos(), (dy + wall * v.cos()) * u.sin(), dz * v.sin());
        }
    }
    let width = most + 1;
    for j in 1..=rings {
        for i in 1..=count {
            m.face(&[width * j + i, width * (j - 1) + i, width * (j - 1) + i + 1, width * j + i + 1]);
        }
    }
    if count != most {
        m.face(&(1..=rings).map(|j| width * j + 1).collect::<Vec<_>>());
        m.face(&(1..=rings).map(|j| width * (rings - j) + count + 1).collect::<Vec<_>>());
    }
    m
}

fn sphere(p: &Numbers, dome: bool) -> Simpleton {
    let steps = ((2.0 * (p.number("PrimSUBDIV", 32.0) / 2.0 + 0.5).floor()) as i64).clamp(4, 32) as usize;
    let (dx, dy, dz) = p.half_size();
    let mut m = Simpleton::default();
    for y in 0..=steps {
        let t = PI * y as f64 / steps as f64;
        for x in 0..=steps {
            let q = TAU * x as f64 / steps as f64;
            m.xyz(-dx * q.cos() * t.sin(), dy * q.sin() * t.sin(), dz * t.cos());
        }
        if y > 0 {
            let mut i = m.verts.len() - 2 * (steps + 1);
            while i + steps + 2 < m.verts.len() {
                m.face(&[i + 1, i + 2, i + steps + 3, i + steps + 2]);
                i += 1;
            }
        }
    }
    if dome {
        return bisect(&m, (0.0, 0.0, 0.0), (0.0, 0.0, 1.0), true).unwrap_or(m);
    }
    m
}

/// A NACA four-digit section of `chord`, upper then lower, 16 stations
/// each, leading edge at the offset and the chord running along -X.
fn naca(chord: f64, camber: f64, camber_at: f64, thickness: f64, open: bool, offset: Vec3) -> (Vec<Vec3>, Vec<Vec3>) {
    let (m, p, t) = (camber * 0.01, camber_at * 0.01, thickness * 0.01);
    let a4 = if open { 0.1015 } else { 0.1036 };
    let (mut upper, mut lower) = (Vec::new(), Vec::new());
    for i in 0..=15 {
        let x = 1.0 - 0.5 * ((PI * i as f64 / 15.0).cos() + 1.0);
        let half = (t / 0.2) * (0.2969 * x.sqrt() - 0.1260 * x - 0.3516 * x * x + 0.2843 * x.powi(3) - a4 * x.powi(4));
        let (k, y) = if x > p {
            let k = m / (1.0 - p).powi(2);
            (k, k * ((1.0 - 2.0 * p) + 2.0 * p * x - x * x))
        } else {
            let k = if p == 0.0 { 0.0 } else { m / (p * p) };
            (k, k * (2.0 * p * x - x * x))
        };
        let a = (k * (2.0 * p - 2.0 * x)).atan();
        upper.push((-(x - a.sin() * half) * chord + offset.0, offset.1, (y + a.cos() * half) * chord + offset.2));
        lower.push((-(x + a.sin() * half) * chord + offset.0, offset.1, (y - a.cos() * half) * chord + offset.2));
    }
    (upper, lower)
}

struct Wing {
    root: (Vec<Vec3>, Vec<Vec3>),
    tip: (Vec<Vec3>, Vec<Vec3>),
    open: bool,
}

impl Wing {
    /// One section `along` the span (0 root, 1 tip), skinned to the section
    /// pushed next when `fill`, and capped on `side` (-1 root-ward, 1 tip-ward).
    fn insert(&self, m: &mut Simpleton, along: f64, fill: bool, side: i32) {
        let stations = self.root.0.len();
        let ahead = stations * 2;
        for j in 0..stations {
            let n = m.verts.len();
            m.point(lerp(self.root.0[j], self.tip.0[j], along));
            m.point(lerp(self.root.1[j], self.tip.1[j], along));
            if j < stations - 1 {
                if along < 1.0 && fill {
                    m.tri(n + 1 + ahead, n + 3 + ahead, n + 3);
                    m.tri(n + 1 + ahead, n + 3, n + 1);
                    m.tri(n + 2, n + 4, n + 4 + ahead);
                    m.tri(n + 2, n + 4 + ahead, n + 2 + ahead);
                }
                if side == -1 {
                    m.tri(n + 1, n + 3, n + 4);
                    m.tri(n + 1, n + 4, n + 2);
                } else if side == 1 {
                    m.tri(n + 2, n + 4, n + 3);
                    m.tri(n + 2, n + 3, n + 1);
                }
            } else if self.open && along < 1.0 && fill {
                m.tri(n + 1 + ahead, n + 2 + ahead, n + 2);
                m.tri(n + 1 + ahead, n + 2, n + 1);
            }
        }
    }
}

fn airfoil(p: &Numbers) -> Simpleton {
    let root_chord = p.number("PrimCHORDR", 1.0).clamp(1.0, 2000.0);
    let tip_chord = p.number("PrimCHORDT", 1.0).clamp(1.0, 2000.0);
    let span = p.number("PrimSPAN", 1.0);
    let open = p.flag("PrimAFOPEN");
    let section = |chord: f64, offset: Vec3| naca(chord, p.number("PrimAFM", 0.0).clamp(0.0, 9.5), p.number("PrimAFP", 0.0).clamp(0.0, 90.0), p.number("PrimAFT", 0.0).clamp(1.0, 40.0), open, offset);
    let lean = |degrees: f64| degrees.to_radians().sin() * (span + root_chord);
    let wing = Wing {
        root: section(root_chord, (0.0, 0.0, 0.0)),
        tip: section(tip_chord, (lean(p.number("PrimSWEEP", 0.0)), span, lean(p.number("PrimDIHEDRAL", 0.0)))),
        open,
    };
    let options = p.number("PrimCSOPT", 0.0) as i64;
    let (from, length, depth) = (p.number("PrimCSYPOS", 0.0), p.number("PrimCSYLEN", 0.0), p.number("PrimCSXLEN", 0.0));

    let mut parts: Option<Vec<Simpleton>> = None;
    if options & 1 == 1 && length > 0.0 && depth > 0.0 {
        let (near, far) = (from, (from + length).min(1.0));
        let trailing = |side: &(Vec<Vec3>, Vec<Vec3>)| mul(add(side.0[15], side.1[15]), 0.5);
        let (a, b) = (trailing(&wing.root), trailing(&wing.tip));
        let chord_here = root_chord + (tip_chord - root_chord) * (from + if root_chord > tip_chord { length } else { 0.0 }).min(1.0);
        let cut_at = add(lerp(a, b, from), (chord_here * depth, 0.0, 0.0));
        let along = sub(a, b);
        let aft = cross(mul(along, 1.0 / dot(along, along).sqrt().max(1e-9)), (0.0, 0.0, 1.0));
        if options & 2 == 2 {
            if from < 1.0 {
                let mut surface = Simpleton::default();
                wing.insert(&mut surface, near, true, -1);
                wing.insert(&mut surface, far, false, 1);
                parts = bisect(&surface, cut_at, aft, true).map(|cut| vec![cut]);
            }
        } else {
            let (mut front, mut rear) = (Simpleton::default(), Simpleton::default());
            wing.insert(&mut front, 0.0, true, -1);
            wing.insert(&mut front, 1.0, true, 1);
            wing.insert(&mut rear, 0.0, near > 0.0, if near <= 0.0 { 0 } else { -1 });
            if near > 0.0 && near < 1.0 {
                wing.insert(&mut rear, near, false, 1);
            }
            if far < 1.0 {
                wing.insert(&mut rear, far, true, -1);
            }
            wing.insert(&mut rear, 1.0, true, if far >= 1.0 { 0 } else { 1 });
            if let (Some(front), Some(rear)) = (bisect(&front, cut_at, mul(aft, -1.0), true), bisect(&rear, cut_at, aft, false)) {
                parts = Some(vec![front, rear]);
            }
        }
    }
    let parts = parts.unwrap_or_else(|| {
        let mut whole = Simpleton::default();
        wing.insert(&mut whole, 0.0, true, -1);
        wing.insert(&mut whole, 1.0, true, 1);
        vec![whole]
    });

    let mut model = Simpleton::default();
    for part in parts {
        let base = model.verts.len();
        model.verts.extend(part.verts);
        model.index.extend(part.index.iter().map(|n| n + base));
    }
    if p.flag("PrimAFFLIP") {
        for vert in &mut model.verts {
            vert.1 = -vert.1;
        }
        for triangle in model.index.chunks_exact_mut(3) {
            triangle.swap(0, 2);
        }
    }
    model
}

fn shape(class: &str, p: &Numbers) -> Simpleton {
    if class == "primitive_airfoil" {
        return airfoil(p);
    }
    let kind = match p.dupe.get(p.table, "PrimTYPE") {
        Some(Value::Str(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
        _ => String::new(),
    };
    let mut made = match kind.as_str() {
        "wedge" => wedge(p),
        "wedge_corner" => wedge_corner(p),
        "pyramid" => pyramid(p),
        "cone" => cone(p),
        "cylinder" => cylinder(p),
        "tube" => tube(p),
        "torus" => torus(p),
        "sphere" => sphere(p, false),
        "dome" => sphere(p, true),
        _ => cube(p),
    };
    let (turn, shift) = (p.vector("PrimMESHROT", (0.0, 0.0, 0.0)), p.vector("PrimMESHPOS", (0.0, 0.0, 0.0)));
    for vert in &mut made.verts {
        if turn != (0.0, 0.0, 0.0) {
            *vert = transform::rotate_vec(turn, *vert);
        }
        *vert = add(*vert, shift);
    }
    made
}

/// Flat triangles turned into a mesh: a corner's normal is the average of
/// the normals of every corner at the same point that lie within `smooth`
/// degrees of its own, which is how the addon smooths; UVs are the
/// position projected along the normal's main axis, `uv_scale` units a tile.
fn mesh_from(model: &Simpleton, smooth: f64, uv_scale: f64) -> MeshData {
    let mut corners: Vec<(Vec3, Vec3)> = Vec::new();
    for triangle in model.index.chunks_exact(3) {
        let at = [model.verts[triangle[0] - 1], model.verts[triangle[1] - 1], model.verts[triangle[2] - 1]];
        let normal = cross(sub(at[1], at[0]), sub(at[2], at[0]));
        let length = dot(normal, normal).sqrt();
        if length < 1e-12 {
            continue;
        }
        corners.extend(at.map(|point| (point, mul(normal, 1.0 / length))));
    }
    let place = |point: Vec3| ((point.0 * 64.0).round() as i64, (point.1 * 64.0).round() as i64, (point.2 * 64.0).round() as i64);
    let mut sharing: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (n, (point, _)) in corners.iter().enumerate() {
        sharing.entry(place(*point)).or_default().push(n);
    }
    let within = smooth.to_radians().cos();
    let mut mesh = MeshData { positions: Vec::new(), normals: Vec::new(), uvs: Vec::new(), indices: Vec::new(), sections: Vec::new(), section_slots: vec![0] };
    for (n, (point, flat)) in corners.iter().enumerate() {
        let mut normal = (0.0, 0.0, 0.0);
        for other in &sharing[&place(*point)] {
            if dot(corners[*other].1, *flat) >= within {
                normal = add(normal, corners[*other].1);
            }
        }
        let length = dot(normal, normal).sqrt().max(1e-9);
        let normal = mul(normal, 1.0 / length);
        let main = [flat.0.abs(), flat.1.abs(), flat.2.abs()];
        let (u, v) = if main[0] >= main[1] && main[0] >= main[2] { (point.1, point.2) } else if main[1] >= main[2] { (point.0, point.2) } else { (point.0, point.1) };
        mesh.positions.push([point.0 as f32, point.1 as f32, point.2 as f32]);
        mesh.normals.push([normal.0 as f32, normal.1 as f32, normal.2 as f32]);
        mesh.uvs.push([(u / uv_scale) as f32, (v / uv_scale) as f32]);
        mesh.indices.push(n as u32);
    }
    // The engine draws clockwise; the addon swaps its triangles on the way
    // out and so does this.
    for triangle in mesh.indices.chunks_exact_mut(3) {
        triangle.swap(1, 2);
    }
    mesh.sections.push((0, mesh.indices.len() as u32, DEFAULT_MATERIAL.to_owned()));
    mesh
}

/// The generated mesh of the primitive at `index`, with a name that is the
/// same for the same numbers, so equal shapes share one upload. None when
/// the entity is not a primitive or stores no numbers.
pub fn of_entity(dupe: &Dupe, index: f64) -> Option<(String, MeshData)> {
    let et = transform::entity_table(dupe, index)?;
    let class = match dupe.get(et, "Class") {
        Some(Value::Str(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
        _ => return None,
    };
    if !matches!(class.as_str(), "primitive_shape" | "primitive_airfoil") {
        return None;
    }
    let numbers = Numbers { dupe, table: dupe.get_table(et, "DT")? };
    let model = shape(&class, &numbers);
    if model.index.is_empty() {
        return None;
    }
    let mesh = mesh_from(&model, numbers.number("PrimMESHSMOOTH", 0.0), numbers.number("PrimMESHUV", 48.0).max(1.0));
    let mut hash = 0xcbf29ce484222325u64;
    for position in &mesh.positions {
        for value in position {
            hash = (hash ^ value.to_bits() as u64).wrapping_mul(0x100000001b3);
        }
    }
    Some((format!("generated/{class}_{hash:016x}.mdl"), mesh))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn primitive(class: &str, numbers: &[(&str, Value)]) -> (Dupe, f64) {
        let mut dupe = crate::buildfile::empty_dupe();
        let index = crate::build::spawn_prop(&mut dupe, "models/combine_helicopter/helicopter_bomb01.mdl", (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)).unwrap();
        let et = transform::entity_table(&dupe, index).unwrap();
        dupe.set(et, "Class", Value::Str(class.as_bytes().to_vec()));
        let dt = dupe.new_table();
        for (key, value) in numbers {
            dupe.set(dt, key, value.clone());
        }
        dupe.set(et, "DT", Value::Table(dt));
        (dupe, index)
    }

    fn text(value: &str) -> Value {
        Value::Str(value.as_bytes().to_vec())
    }

    /// Volume by the divergence theorem, which is only right for a closed
    /// mesh wound outwards; and the box the mesh fits in.
    fn volume_and_extent(mesh: &MeshData) -> (f64, Vec3, Vec3) {
        let at = |n: u32| {
            let p = mesh.positions[n as usize];
            (p[0] as f64, p[1] as f64, p[2] as f64)
        };
        let mut volume = 0.0;
        for triangle in mesh.indices.chunks_exact(3) {
            // Stored clockwise for the engine, so the sign is turned back.
            volume -= dot(at(triangle[0]), cross(at(triangle[1]), at(triangle[2]))) / 6.0;
        }
        let (mut low, mut high) = ((f64::MAX, f64::MAX, f64::MAX), (f64::MIN, f64::MIN, f64::MIN));
        for n in 0..mesh.positions.len() as u32 {
            let p = at(n);
            low = (low.0.min(p.0), low.1.min(p.1), low.2.min(p.2));
            high = (high.0.max(p.0), high.1.max(p.1), high.2.max(p.2));
        }
        (volume, low, high)
    }

    #[test]
    fn a_cube_is_its_size_and_a_wedge_half_of_it() {
        let size = Value::Vector(40.0, 20.0, 10.0);
        let (dupe, index) = primitive("primitive_shape", &[("PrimTYPE", text("cube")), ("PrimSIZE", size.clone())]);
        let (name, mesh) = of_entity(&dupe, index).unwrap();
        assert!(name.starts_with("generated/primitive_shape_") && name.ends_with(".mdl"));
        let (volume, low, high) = volume_and_extent(&mesh);
        assert!((volume - 8000.0).abs() < 1e-6, "{volume}");
        assert_eq!((low, high), ((-20.0, -10.0, -5.0), (20.0, 10.0, 5.0)));
        assert_eq!(mesh.triangles(), 12);

        let (dupe, index) = primitive("primitive_shape", &[("PrimTYPE", text("wedge")), ("PrimSIZE", size)]);
        let (volume, _, _) = volume_and_extent(&of_entity(&dupe, index).unwrap().1);
        assert!((volume - 4000.0).abs() < 1e-6, "a wedge with its ridge over one edge: {volume}");
    }

    #[test]
    fn the_round_shapes_are_closed_wound_outwards_and_about_the_right_volume() {
        let size = Value::Vector(60.0, 60.0, 80.0);
        let expect = [
            ("cylinder", PI * 30.0 * 30.0 * 80.0),
            ("cone", PI * 30.0 * 30.0 * 80.0 / 3.0),
            ("sphere", 4.0 / 3.0 * PI * 30.0 * 30.0 * 40.0),
            ("dome", 2.0 / 3.0 * PI * 30.0 * 30.0 * 40.0),
            ("pyramid", 60.0 * 60.0 * 80.0 / 3.0),
            ("tube", PI * (30.0 * 30.0 - 26.0 * 26.0) * 80.0),
        ];
        for (kind, volume) in expect {
            let (dupe, index) = primitive("primitive_shape", &[("PrimTYPE", text(kind)), ("PrimSIZE", size.clone()), ("PrimDT", Value::Number(4.0))]);
            let (found, low, high) = volume_and_extent(&of_entity(&dupe, index).unwrap().1);
            assert!((found / volume - 1.0).abs() < 0.03, "{kind}: {found} against {volume}");
            assert!(high.0 <= 30.0 + 1e-6 && low.0 >= -30.0 - 1e-6 && high.2 <= 40.0 + 1e-6, "{kind}: {low:?} {high:?}");
        }
        // The p62's half tube: sixteen of thirty-two segments.
        let (dupe, index) = primitive(
            "primitive_shape",
            &[("PrimTYPE", text("tube")), ("PrimSIZE", size), ("PrimDT", Value::Number(4.0)), ("PrimMAXSEG", Value::Number(32.0)), ("PrimNUMSEG", Value::Number(16.0))],
        );
        let (half, _, _) = volume_and_extent(&of_entity(&dupe, index).unwrap().1);
        assert!((half / (PI * (900.0 - 676.0) * 40.0) - 1.0).abs() < 0.03, "{half}");
    }

    #[test]
    fn a_torus_is_as_wide_as_its_centre_line_plus_its_tube() {
        let (dupe, index) = primitive("primitive_shape", &[("PrimTYPE", text("torus")), ("PrimSIZE", Value::Vector(60.0, 60.0, 10.0)), ("PrimDT", Value::Number(8.0)), ("PrimSUBDIV", Value::Number(32.0))]);
        let (volume, low, high) = volume_and_extent(&of_entity(&dupe, index).unwrap().1);
        assert!((high.0 - 34.0).abs() < 1e-6 && (low.2 + 5.0).abs() < 1e-6, "{low:?} {high:?}");
        assert!((volume / (TAU * 30.0 * PI * 4.0 * 5.0) - 1.0).abs() < 0.03, "{volume}");
    }

    #[test]
    fn the_p62_wing_is_its_span_long_its_root_chord_deep_and_mirrors_when_flipped() {
        let wing = [
            ("PrimAFM", Value::Number(2.0)),
            ("PrimAFP", Value::Number(40.0)),
            ("PrimAFT", Value::Number(3.8826)),
            ("PrimCHORDR", Value::Number(165.0)),
            ("PrimCHORDT", Value::Number(111.0)),
            ("PrimSPAN", Value::Number(315.0)),
            ("PrimSWEEP", Value::Number(-2.7592)),
        ];
        let (dupe, index) = primitive("primitive_airfoil", &wing);
        let (volume, low, high) = volume_and_extent(&of_entity(&dupe, index).unwrap().1);
        assert!(volume > 0.0, "closed and wound outwards: {volume}");
        assert!((high.1 - 315.0).abs() < 1e-6 && low.1.abs() < 1e-6, "span along +Y: {low:?} {high:?}");
        assert!((low.0 + 165.0).abs() < 25.0 && high.0.abs() < 1e-6, "chord along -X from the leading edge: {low:?} {high:?}");
        let low_x = low.0;

        let mut flipped = wing.to_vec();
        flipped.push(("PrimAFFLIP", Value::Bool(true)));
        let (dupe, index) = primitive("primitive_airfoil", &flipped);
        let (mirrored, low, _) = volume_and_extent(&of_entity(&dupe, index).unwrap().1);
        assert!((mirrored - volume).abs() < 1e-3 && (low.1 + 315.0).abs() < 1e-6, "{mirrored} {low:?}");

        // With a control surface cut out the wing keeps its outline (its rear
        // piece is left open against the front piece's cap, as in game, so
        // only its extent can be measured); the surface on its own is a
        // closed piece that sits inside the notch.
        let cut = [("PrimCSYPOS", Value::Number(0.65)), ("PrimCSYLEN", Value::Number(0.33)), ("PrimCSXLEN", Value::Number(0.25))];
        let mut notched = wing.to_vec();
        notched.extend(cut.iter().cloned());
        notched.push(("PrimCSOPT", Value::Number(1.0)));
        let (dupe, index) = primitive("primitive_airfoil", &notched);
        let whole_wing = of_entity(&dupe, index).unwrap().1;
        let (_, notched_low, notched_high) = volume_and_extent(&whole_wing);
        assert!((notched_high.1 - 315.0).abs() < 1e-6 && (notched_low.0 - low_x).abs() < 1e-6, "{notched_low:?} {notched_high:?}");

        let mut surface = wing.to_vec();
        surface.extend(cut.iter().cloned());
        surface.push(("PrimCSOPT", Value::Number(3.0)));
        let (dupe, index) = primitive("primitive_airfoil", &surface);
        let (piece, piece_low, piece_high) = volume_and_extent(&of_entity(&dupe, index).unwrap().1);
        assert!(piece > 0.0 && piece < volume * 0.2, "{piece} of {volume}");
        assert!(piece_low.1 >= 315.0 * 0.65 - 1e-3 && piece_high.1 <= 315.0 * 0.98 + 1e-3, "{piece_low:?} {piece_high:?}");
        assert!(piece_high.0 < -60.0, "behind the hinge line, at the trailing edge: {piece_high:?}");
    }

    #[test]
    fn an_unknown_shape_is_the_box_of_its_size_and_a_prop_is_not_a_primitive() {
        let (dupe, index) = primitive("primitive_shape", &[("PrimTYPE", text("cube_magic")), ("PrimSIZE", Value::Vector(10.0, 10.0, 10.0))]);
        assert!((volume_and_extent(&of_entity(&dupe, index).unwrap().1).0 - 1000.0).abs() < 1e-6);
        let (dupe, index) = primitive("prop_physics", &[("PrimTYPE", text("cube"))]);
        assert!(of_entity(&dupe, index).is_none());
    }

    #[test]
    fn a_mesh_offset_moves_the_shape_and_equal_numbers_share_a_name() {
        let numbers = [("PrimTYPE", text("dome")), ("PrimSIZE", Value::Vector(20.0, 20.0, 20.0)), ("PrimMESHPOS", Value::Vector(0.0, 11.5, 0.0))];
        let (dupe, index) = primitive("primitive_shape", &numbers);
        let (name, mesh) = of_entity(&dupe, index).unwrap();
        let (_, low, high) = volume_and_extent(&mesh);
        assert!((low.1 - 1.5).abs() < 1e-6 && (high.1 - 21.5).abs() < 1e-6 && low.2.abs() < 1e-6, "{low:?} {high:?}");
        let (again, other) = primitive("primitive_shape", &numbers);
        assert_eq!(of_entity(&again, other).unwrap().0, name);
    }
}
