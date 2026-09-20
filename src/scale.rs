//! How big ACF draws an entity: the multiplier it applies to a model on
//! paste. Read from acf_gun, acf_turret, acf_gearbox, acf_turret_motor,
//! acf_baseplate and base_scalable in the ACF-3 source, and checked
//! against the values the game saved into the ironlock dupe.

use crate::dupe::Dupe;
use crate::rules;
use crate::transform::{self, Vec3};
use crate::value::Value;

/// The per-axis multiplier for the entity's model, given the model's
/// native half-extents. A `Scale` the game saved wins; a `Size` is the box
/// the model is stretched to; otherwise the class formula, which is what
/// a catalogue-built entity gets on paste.
pub fn model_scale(dupe: &Dupe, index: f64, native_half: Vec3) -> Vec3 {
    let raw = raw_scale(dupe, index, native_half);
    // A missing or zero number would otherwise scale the model to nothing,
    // and an entity the editor cannot show cannot be fixed in it.
    let visible = |axis: f64| if axis.is_finite() && axis > 0.0 { axis } else { 1.0 };
    (visible(raw.0), visible(raw.1), visible(raw.2))
}

fn raw_scale(dupe: &Dupe, index: f64, native_half: Vec3) -> Vec3 {
    let Some(et) = transform::entity_table(dupe, index) else {
        return (1.0, 1.0, 1.0);
    };
    // What ACF will work out on paste comes first, then the Size it will
    // stretch the model to, and only then the Scale it saved last time: a
    // dupe edited since it was saved still carries the old Scale, and
    // drawing by it made a resized crate look as if nothing had happened.
    if let Some(scale) = class_scale(dupe, et, native_half) {
        return scale;
    }
    let size = match dupe.get(et, "Size") {
        Some(Value::Vector(x, y, z)) => Some((*x, *y, *z)),
        _ => None,
    };
    // Only ACF's containers are stretched to their Size. On a gearbox or a
    // gun, Size is a measurement the game took, and stretching the model to
    // it drew every gearbox squashed; there the saved Scale is the truth.
    let stretched_to_size = matches!(class_of(dupe, et).as_str(), "acf_ammo" | "acf_fueltank" | "acf_supply");
    if let (true, Some(size)) = (stretched_to_size, size) {
        return stretch(size, native_half);
    }
    if let Some(Value::Vector(x, y, z)) = dupe.get(et, "Scale") {
        return (*x, *y, *z);
    }
    size.map_or((1.0, 1.0, 1.0), |size| stretch(size, native_half))
}

/// The scale a class's own rule gives, when the entity carries what the
/// rule needs.
fn class_scale(dupe: &Dupe, et: usize, native_half: Vec3) -> Option<Vec3> {
    let text = |key: &str| match dupe.get(et, key) {
        Some(Value::Str(b)) => String::from_utf8_lossy(b).into_owned(),
        _ => String::new(),
    };
    let uniform = |s: f64| (s, s, s);
    match text("Class").to_lowercase().as_str() {
        "acf_gun" => rules::gun_scale(&text("Weapon"), dupe.get_number(et, "Caliber")?).map(uniform),
        "acf_turret" => {
            let size = dupe.get_number(et, "RingSize").filter(|size| *size > 0.0)?;
            Some(if text("Turret").eq_ignore_ascii_case("Turret-V") {
                uniform(size / rules::TRUNNION_SCALE_BASE)
            } else {
                stretch((size, size, rules::ring_height(size)), native_half)
            })
        }
        "acf_turret_motor" => dupe.get_number(et, "CompSize").map(uniform),
        // A pasted gearbox keeps it at the top level; a built one in its user data.
        "acf_gearbox" => dupe
            .get_number(et, "GearboxScale")
            .or_else(|| dupe.get_table(et, "ACF_UserData").and_then(|ud| dupe.get_number(ud, "GearboxScale")))
            .map(uniform),
        "acf_baseplate" => {
            // acf_baseplate/cl_init.lua builds the box with its length along
            // the entity's x axis and its width along y.
            let ud = dupe.get_table(et, "ACF_UserData")?;
            let dims = (dupe.get_number(ud, "Length")?, dupe.get_number(ud, "Width")?, dupe.get_number(ud, "Thickness")?);
            Some(stretch(dims, native_half))
        }
        _ => None,
    }
}

/// The box a builder sizes by hand: a baseplate's length, width and
/// thickness, or the Size of a crate, a fuel tank or another ACF container.
/// None for anything sized some other way, a gun by its calibre say.
pub fn dimensions(dupe: &Dupe, index: f64) -> Option<Vec3> {
    let et = transform::entity_table(dupe, index)?;
    if is_class(dupe, et, "acf_baseplate") {
        let ud = dupe.get_table(et, "ACF_UserData")?;
        return Some((dupe.get_number(ud, "Length")?, dupe.get_number(ud, "Width")?, dupe.get_number(ud, "Thickness")?));
    }
    match dupe.get(et, "Size") {
        Some(Value::Vector(x, y, z)) if matches!(class_of(dupe, et).as_str(), "acf_ammo" | "acf_fueltank" | "acf_supply") => Some((*x, *y, *z)),
        _ => None,
    }
}

fn class_of(dupe: &Dupe, et: usize) -> String {
    match dupe.get(et, "Class") {
        Some(Value::Str(b)) => String::from_utf8_lossy(b).to_lowercase(),
        _ => String::new(),
    }
}

fn is_class(dupe: &Dupe, et: usize, class: &str) -> bool {
    class_of(dupe, et) == class
}

/// Sets those dimensions, held inside what ACF allows for the class, in
/// every place the dupe keeps them, and keeps a saved Scale in step so the
/// file never disagrees with itself. Returns the size that was written.
pub fn resize(dupe: &mut Dupe, index: f64, wanted: Vec3) -> Result<Vec3, String> {
    let et = transform::entity_table(dupe, index).ok_or_else(|| format!("no entity {index}"))?;
    let before = dimensions(dupe, index).ok_or("this part is not sized by hand")?;
    let class = class_of(dupe, et);
    // A crate is rounds and a tank whole units; containers.rs writes the
    // keys the game reads for both.
    if class == "acf_ammo" {
        let mut made = crate::containers::ammo_crate(dupe, index).ok_or("this crate could not be read")?;
        let round = made.round();
        let count = |side: f64, each: f64| if side.is_finite() && each > 0.0 { (side / each).round().clamp(1.0, 10_000.0) as u32 } else { 1 };
        made.counts = (count(wanted.0, round.length), count(wanted.1, round.diameter), count(wanted.2, round.diameter));
        return crate::containers::set_ammo_crate(dupe, index, &made).map(|made| made.size());
    }
    if class == "acf_fueltank" {
        let tank = crate::containers::tank(dupe, index).ok_or("this tank could not be read")?;
        return crate::containers::set_tank(dupe, index, &crate::containers::Tank { size: wanted, ..tank }).map(|made| made.size);
    }
    let limits: [(f64, f64); 3] = match class.as_str() {
        "acf_baseplate" => [rules::BASEPLATE_LENGTH, rules::BASEPLATE_WIDTH, rules::BASEPLATE_THICKNESS],
        _ => [rules::CONTAINER_SIZE; 3],
    };
    let held = |value: f64, (low, high): (f64, f64)| if value.is_finite() { value.clamp(low, high) } else { low };
    let size = (held(wanted.0, limits[0]), held(wanted.1, limits[1]), held(wanted.2, limits[2]));

    if class == "acf_baseplate" {
        let ud = dupe.get_table(et, "ACF_UserData").ok_or("the baseplate has no ACF_UserData")?;
        for (key, value) in [("Length", size.0), ("Width", size.1), ("Thickness", size.2)] {
            dupe.set(ud, key, Value::Number(value));
            if dupe.get_number(et, key).is_some() {
                dupe.set(et, key, Value::Number(value));
            }
        }
    }
    if matches!(dupe.get(et, "Size"), Some(Value::Vector(..))) {
        dupe.set(et, "Size", Value::Vector(size.0, size.1, size.2));
    }
    if let Some(ud) = dupe.get_table(et, "ACF_UserData") {
        if matches!(dupe.get(ud, "Size"), Some(Value::Vector(..))) {
            dupe.set(ud, "Size", Value::Vector(size.0, size.1, size.2));
        }
    }
    if let Some(Value::Vector(x, y, z)) = dupe.get(et, "Scale").cloned() {
        let grown = |scale: f64, new: f64, old: f64| if old.abs() > 1e-9 { scale * new / old } else { scale };
        dupe.set(et, "Scale", Value::Vector(grown(x, size.0, before.0), grown(y, size.1, before.1), grown(z, size.2, before.2)));
    }
    Ok(size)
}

/// Every entity whose game-saved Scale is not what these rules give, as
/// text. On a dupe nobody has edited the list should be empty; an entry
/// means a rule here is wrong, or the part was resized since it was saved.
pub fn disagreements(dupe: &Dupe, lib: &mut crate::models::ModelLibrary) -> Vec<String> {
    let mut found = Vec::new();
    for (index, class, model) in dupe.list_entities() {
        let Some(et) = transform::entity_table(dupe, index) else { continue };
        let Some(Value::Vector(x, y, z)) = dupe.get(et, "Scale") else { continue };
        let Some((bounds, _)) = lib.bounds(model.trim_matches('"')) else { continue };
        let ruled = model_scale(dupe, index, bounds.half_extents());
        let off = [(ruled.0, *x), (ruled.1, *y), (ruled.2, *z)].iter().map(|(a, b)| ((a - b) / b.abs().max(1e-6)).abs()).fold(0.0, f64::max);
        if off > 0.05 {
            found.push(format!(
                "{index} {}: saved ({x:.2}, {y:.2}, {z:.2}), ruled ({:.2}, {:.2}, {:.2})",
                class.trim_matches('"'),
                ruled.0,
                ruled.1,
                ruled.2
            ));
        }
    }
    found
}

fn stretch(size: Vec3, half: Vec3) -> Vec3 {
    let axis = |s: f64, h: f64| if h > 1e-6 { s / (h * 2.0) } else { 1.0 };
    (axis(size.0, half.0), axis(size.1, half.1), axis(size.2, half.2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dupe::Dupe;
    use crate::value::{Node, Value};

    fn dupe_with(class: &str, fields: &[(&str, Value)]) -> (Dupe, f64) {
        let mut d = crate::buildfile::empty_dupe();
        let root = d.root_table().unwrap();
        let ents = d.get_table(root, "Entities").unwrap();
        let et = d.new_table();
        d.set(et, "Class", Value::Str(class.as_bytes().to_vec()));
        for (k, v) in fields {
            d.set(et, k, v.clone());
        }
        if let Node::Table(e) = &mut d.arena[ents] {
            e.push((Value::Number(7.0), Value::Table(et)));
        }
        (d, 7.0)
    }

    fn close(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01 && (a.2 - b.2).abs() < 0.01
    }

    #[test]
    fn what_acf_will_work_out_on_paste_beats_the_scale_it_saved_last_time() {
        // A gun whose calibre was edited after the game saved Scale 1.05.
        let (d, i) = dupe_with("acf_gun", &[
            ("Scale", Value::Vector(1.05, 1.05, 1.05)),
            ("Caliber", Value::Number(50.0)),
            ("Weapon", Value::Str(b"C".to_vec())),
        ]);
        assert!(close(model_scale(&d, i, (108.0, 9.0, 7.0)), (0.42, 0.42, 0.42)));
        // A crate resized after the game saved its Scale.
        let (d, i) = dupe_with("acf_ammo", &[("Scale", Value::Vector(1.0, 1.0, 1.0)), ("Size", Value::Vector(24.0, 36.0, 12.0))]);
        assert!(close(model_scale(&d, i, (6.0, 6.0, 6.0)), (2.0, 3.0, 1.0)));
        // A gearbox's Size is a measurement, not what its model is stretched to.
        let (d, i) = dupe_with("acf_gearbox", &[("Scale", Value::Vector(1.7, 1.7, 1.7)), ("Size", Value::Vector(16.3, 27.2, 10.8)), ("GearboxScale", Value::Number(1.7))]);
        assert!(close(model_scale(&d, i, (6.0, 11.6, 4.1)), (1.7, 1.7, 1.7)));
        let (d, i) = dupe_with("acf_gearbox", &[("Scale", Value::Vector(1.0, 1.0, 1.0)), ("Size", Value::Vector(6.4, 14.4, 14.8))]);
        assert!(close(model_scale(&d, i, (4.0, 11.6, 8.2)), (1.0, 1.0, 1.0)));
        // With nothing else to go on, the saved Scale is all there is.
        let (d, i) = dupe_with("prop_physics", &[("Scale", Value::Vector(1.5, 1.5, 2.0))]);
        assert!(close(model_scale(&d, i, (6.0, 6.0, 6.0)), (1.5, 1.5, 2.0)));
    }

    #[test]
    fn resizing_writes_every_place_a_size_is_kept_and_stays_inside_acf_limits() {
        // A crate is whole rounds: with no gun named it packs ACF's default
        // 100 mm round, 80 cm long, and lands on the nearest count each way.
        let (mut d, i) = dupe_with("acf_ammo", &[("Scale", Value::Vector(2.0, 2.0, 2.0)), ("Size", Value::Vector(24.0, 24.0, 24.0))]);
        assert_eq!(dimensions(&d, i), Some((24.0, 24.0, 24.0)));
        let (long, across) = (80.0 / 2.54, 10.0 / 2.54);
        let made = resize(&mut d, i, (48.0, 500.0, 1.0)).unwrap();
        assert!(close(made, (2.0 * long, 24.0 * across, across)), "{made:?}: two rounds long, the 24 that fit in 96 wide, one high");
        assert_eq!(dimensions(&d, i), Some(made));
        let et = transform::entity_table(&d, i).unwrap();
        assert_eq!((d.get_number(et, "CrateProjectilesX"), d.get_number(et, "CrateProjectilesY"), d.get_number(et, "CrateProjectilesZ")), (Some(2.0), Some(24.0), Some(1.0)));
        assert!(close(model_scale(&d, i, (6.0, 6.0, 6.0)), (made.0 / 12.0, made.1 / 12.0, made.2 / 12.0)), "the saved scale is kept in step");

        let (mut d, i) = dupe_with("acf_fueltank", &[("Size", Value::Vector(24.0, 24.0, 24.0))]);
        assert_eq!(resize(&mut d, i, (200.0, 30.0, 30.0)), Ok((96.0, 30.0, 30.0)), "a fuel tank is a container: 96 at most");

        let mut d = crate::buildfile::empty_dupe();
        let plate = crate::build::spawn_catalog(&mut d, crate::catalog::find("GroundVehicle").unwrap(), (0.0, 0.0, 0.0), (0.0, 90.0, 0.0), &[]).unwrap();
        assert_eq!(dimensions(&d, plate), Some((96.0, 48.0, 2.0)), "length, width, thickness");
        assert_eq!(resize(&mut d, plate, (240.0, 120.0, 9.0)), Ok((240.0, 120.0, 3.0)));
        assert!(close(model_scale(&d, plate, (6.0, 6.0, 6.0)), (20.0, 10.0, 0.25)));

        let (mut d, i) = dupe_with("acf_engine", &[]);
        assert_eq!(dimensions(&d, i), None);
        assert!(resize(&mut d, i, (1.0, 1.0, 1.0)).is_err());
    }

    #[test]
    fn a_size_scales_the_model_to_it() {
        let (d, i) = dupe_with("acf_ammo", &[("Size", Value::Vector(24.0, 36.0, 12.0))]);
        assert!(close(model_scale(&d, i, (6.0, 6.0, 6.0)), (2.0, 3.0, 1.0)));
    }

    #[test]
    fn a_gun_scales_by_calibre_over_its_class_base() {
        // ironlock: the 125 mm cannon pastes at 1.05, the 37 mm RAC at 1.85.
        let (d, i) = dupe_with("acf_gun", &[("Caliber", Value::Number(125.0)), ("Weapon", Value::Str(b"C".to_vec()))]);
        assert!(close(model_scale(&d, i, (108.0, 9.0, 7.0)), (1.05, 1.05, 1.05)));
        let (d, i) = dupe_with("acf_gun", &[("Caliber", Value::Number(37.0)), ("Weapon", Value::Str(b"RAC".to_vec()))]);
        assert!(close(model_scale(&d, i, (30.0, 4.0, 4.0)), (1.85, 1.85, 1.85)));
    }

    #[test]
    fn turrets_scale_by_ring_size_and_baseplates_by_their_user_data() {
        // ironlock: a 60 ring on the 96 x 96 x 10 model pastes at 0.625, 0.625, 0.6.
        let (d, i) = dupe_with("acf_turret", &[("RingSize", Value::Number(60.0)), ("Turret", Value::Str(b"Turret-H".to_vec()))]);
        assert!(close(model_scale(&d, i, (48.0, 48.0, 5.0)), (0.625, 0.625, 0.6)));
        let (d, i) = dupe_with("acf_turret", &[("RingSize", Value::Number(30.0)), ("Turret", Value::Str(b"Turret-V".to_vec()))]);
        assert!(close(model_scale(&d, i, (5.0, 11.0, 10.0)), (1.5, 1.5, 1.5)));

        let mut d = crate::buildfile::empty_dupe();
        let root = d.root_table().unwrap();
        let ents = d.get_table(root, "Entities").unwrap();
        let et = d.new_table();
        d.set(et, "Class", Value::Str(b"acf_baseplate".to_vec()));
        let ud = d.new_table();
        d.set(ud, "Width", Value::Number(96.0));
        d.set(ud, "Length", Value::Number(240.0));
        d.set(ud, "Thickness", Value::Number(3.0));
        d.set(et, "ACF_UserData", Value::Table(ud));
        if let Node::Table(e) = &mut d.arena[ents] {
            e.push((Value::Number(7.0), Value::Table(et)));
        }
        assert!(close(model_scale(&d, 7.0, (6.0, 6.0, 6.0)), (20.0, 8.0, 0.25)));
    }

    #[test]
    fn a_missing_or_zero_number_never_makes_a_model_vanish() {
        let (d, i) = dupe_with("acf_gun", &[("Weapon", Value::Str(b"C".to_vec()))]);
        assert_eq!(model_scale(&d, i, (108.0, 9.0, 7.0)), (1.0, 1.0, 1.0));
        let (d, i) = dupe_with("acf_gun", &[("Weapon", Value::Str(b"C".to_vec())), ("Caliber", Value::Number(0.0))]);
        assert_eq!(model_scale(&d, i, (108.0, 9.0, 7.0)), (1.0, 1.0, 1.0));
        let (d, i) = dupe_with("acf_turret_motor", &[("CompSize", Value::Number(0.0))]);
        assert_eq!(model_scale(&d, i, (4.0, 4.0, 7.0)), (1.0, 1.0, 1.0));
        let (d, i) = dupe_with("acf_ammo", &[("Size", Value::Vector(24.0, 0.0, 12.0))]);
        assert!(close(model_scale(&d, i, (6.0, 6.0, 6.0)), (2.0, 1.0, 1.0)));
        let (d, i) = dupe_with("acf_gun", &[("Scale", Value::Vector(0.0, 1.0, f64::NAN))]);
        assert_eq!(model_scale(&d, i, (1.0, 1.0, 1.0)), (1.0, 1.0, 1.0));
    }

    #[test]
    fn anything_else_is_drawn_as_is() {
        let (d, i) = dupe_with("acf_engine", &[]);
        assert_eq!(model_scale(&d, i, (24.0, 8.0, 15.0)), (1.0, 1.0, 1.0));
        assert_eq!(model_scale(&d, 99.0, (1.0, 1.0, 1.0)), (1.0, 1.0, 1.0));
    }
}
