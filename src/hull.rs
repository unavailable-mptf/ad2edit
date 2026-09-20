//! Hulls made of SProps, sized from the baseplate and parented to it.
//!
//! The sloped hull is the one to use: a wedge nose whose glacis lies on two
//! triangle cheeks, with sides, a deck and a rear behind it. Slope is what
//! makes armour work in ACF: a plate met at an angle is thicker by
//! 1 / cos(angle), and a round that fails to get through a well sloped
//! plate glances off it (ACF-NOTES.md, "Slope and ricochet"). A vertical
//! plate gets neither, which is why the box hull, kept here for what was
//! built with it, is a poor hull.

use crate::build;
use crate::dupe::Dupe;
use crate::transform::{self, Vec3};

/// SProps' rectangle plates are `rect_AxBx3` with A no longer than B, filed
/// under a folder named for A. Read from the workshop archive's file list
/// (173482196); `ad2read --catalogue-check` proves every name below exists.
const SHORT_SIDES: &[(u32, &str)] = &[
    (12, "size_2"),
    (18, "size_2_5"),
    (24, "size_3"),
    (30, "size_3_5"),
    (36, "size_4"),
    (42, "size_4_5"),
    (48, "size_5"),
    (54, "size_54"),
    (60, "size_60"),
    (66, "size_66"),
    (72, "size_72"),
    (96, "size_6"),
    (144, "size_7"),
];
const LONG_SIDES: &[u32] = &[12, 18, 24, 30, 36, 42, 48, 54, 60, 66, 72, 78, 84, 90, 96, 108, 120, 132, 144, 192, 240, 288, 336, 384, 432, 480];

/// SProps' right triangles are `rtri_AxB`, A the short leg and B the long,
/// filed under a folder named for A. Read from the game's files with
/// `ad2read --model-info`: the long leg lies along the model's x, the short
/// leg stands up its z, it is 3 thick along y, the right angle is at the
/// low x and low z corner, and the slope falls towards high x.
const TRIANGLE_SHORT_LEGS: &[(u32, &str)] = &[(12, "size_1"), (18, "size_1_5"), (24, "size_2"), (30, "size_2_5"), (36, "size_3"), (42, "size_3_5"), (48, "size_4")];
const TRIANGLE_LONG_LEGS: &[u32] = &[12, 18, 24, 30, 36, 42, 48, 54, 60, 66, 72, 78, 84, 90, 96, 108, 120, 132, 144];

pub fn triangle_model(short_leg: u32, long_leg: u32) -> Option<String> {
    let (_, folder) = TRIANGLE_SHORT_LEGS.iter().find(|(leg, _)| *leg == short_leg)?;
    (long_leg >= short_leg && TRIANGLE_LONG_LEGS.contains(&long_leg)).then(|| format!("models/sprops/triangles/right/{folder}/rtri_{short_leg}x{long_leg}.mdl"))
}

/// Every triangle `add_sloped_hull` can name, for checking against the game.
pub fn triangle_models() -> Vec<String> {
    TRIANGLE_SHORT_LEGS.iter().flat_map(|(short, _)| TRIANGLE_LONG_LEGS.iter().filter_map(move |long| triangle_model(*short, *long))).collect()
}

/// What a glacis is worth, by ACF's own sums.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glacis {
    /// From the vertical, which is also the angle a level shot meets it at.
    pub degrees: f64,
    /// How many times its own thickness it is to a level shot.
    pub thicker_by: f64,
    /// The chance an AP round at its usual speed glances off when it fails
    /// to get through.
    pub ap_glances: f64,
}

impl Glacis {
    pub fn at(degrees: f64) -> Glacis {
        Glacis { degrees, thicker_by: crate::containers::effective_thickness(1.0, degrees), ap_glances: crate::containers::ricochet_chance(degrees, 60.0, 800.0, 800.0) }
    }
}

/// The largest plate that fits inside `a` by `b`: its model and its real
/// short and long sides. None when even the smallest plate is too big.
pub fn plate_model(a: f64, b: f64) -> Option<(String, f64, f64)> {
    let (short, long) = if a <= b { (a, b) } else { (b, a) };
    let (side, folder) = SHORT_SIDES.iter().rev().find(|(side, _)| *side as f64 <= short)?;
    let length = LONG_SIDES.iter().rev().find(|length| **length as f64 <= long && **length >= *side)?;
    Some((format!("models/sprops/rectangles/{folder}/rect_{side}x{length}x3.mdl"), *side as f64, *length as f64))
}

/// Every plate model `plate_model` can name, for checking against the game.
pub fn plate_models() -> Vec<String> {
    SHORT_SIDES
        .iter()
        .flat_map(|(side, folder)| {
            LONG_SIDES.iter().filter(move |length| **length >= *side).map(move |length| format!("models/sprops/rectangles/{folder}/rect_{side}x{length}x3.mdl"))
        })
        .collect()
}

fn place(dupe: &mut Dupe, baseplate: f64, model: &str, local_at: Vec3, local_ang: Vec3, thickness_mm: f64) -> Result<f64, String> {
    let (base_at, base_ang) = transform::entity_transform(dupe, baseplate).ok_or("the baseplate has no position")?;
    let offset = transform::rotate_vec(base_ang, local_at);
    let at = (base_at.0 + offset.0, base_at.1 + offset.1, base_at.2 + offset.2);
    let plate = build::spawn_prop(dupe, model, at, transform::compose_angles(base_ang, local_ang))?;
    build::set_armour(dupe, plate, thickness_mm, 0.0)?;
    build::set_parent(dupe, plate, baseplate)?;
    Ok(plate)
}

/// A hull with a wedge nose: two triangle cheeks whose slope is as near
/// `glacis_degrees` from the vertical as SProps' sizes allow, the glacis
/// plate lying on them, and sides, a deck and a rear behind. Worked out in
/// the baseplate's frame, where x is its length and the nose is at high x.
/// Returns the entities and what the glacis came out as.
pub fn add_sloped_hull(dupe: &mut Dupe, baseplate: f64, height: f64, glacis_degrees: f64, thickness_mm: f64) -> Result<(Vec<f64>, Glacis), String> {
    let et = transform::entity_table(dupe, baseplate).ok_or_else(|| format!("no entity {baseplate}"))?;
    let ud = dupe.get_table(et, "ACF_UserData");
    let number = |key: &str| ud.and_then(|ud| dupe.get_number(ud, key)).or_else(|| dupe.get_number(et, key));
    let (length, width) = (number("Length").unwrap_or(96.0), number("Width").unwrap_or(36.0));

    let (rise, _) = TRIANGLE_SHORT_LEGS.iter().rev().find(|(leg, _)| *leg as f64 <= height).ok_or("a sloped hull is at least 12 high")?;
    let wanted_run = *rise as f64 * glacis_degrees.clamp(45.0, 80.0).to_radians().tan();
    // The glacis plate runs across the hull, so the slope may be no longer
    // than the hull is wide; and the nose may take no more than half of it.
    let fits = |run: &&u32| **run >= *rise && (**run as f64) <= length / 2.0 && ((*rise * *rise + **run * **run) as f64).sqrt() <= width;
    let run = TRIANGLE_LONG_LEGS
        .iter()
        .filter(fits)
        .min_by(|a, b| (**a as f64 - wanted_run).abs().total_cmp(&(**b as f64 - wanted_run).abs()))
        .ok_or("the baseplate is too small for a sloped nose: it wants to be at least twice as long and as wide as the hull is high")?;
    let (rise, run) = (*rise as f64, *run as f64);
    let slope = (rise * rise + run * run).sqrt();
    let cheek = triangle_model(rise as u32, run as u32).ok_or("no such triangle")?;

    let body = length - run;
    let (side_model, side_height, side_length) = plate_model(rise, body).ok_or("the baseplate is too short for a side plate")?;
    let (rear_model, rear_short, rear_long) = plate_model(rise, width).ok_or("the baseplate is too narrow for a rear plate")?;
    let (deck_model, _, deck_length) = plate_model(width, body).ok_or("the baseplate is too small for a deck")?;
    let (glacis_model, glacis_short, glacis_long) = plate_model(slope, width).ok_or("the baseplate is too narrow for a glacis")?;
    let _ = (side_height, rear_short, glacis_short);
    let half_width = rear_long.min(glacis_long) / 2.0;
    let body_length = side_length.min(deck_length);
    // The nose ends at the front of the baseplate; the body runs back from it.
    let nose_start = length / 2.0 - run;
    let body_middle = nose_start - body_length / 2.0;

    // The glacis turns to lie across the hull (yaw 90) and then leans back
    // about that axis until its short side runs up the cheeks' slope. Which
    // way round a roll goes is settled by trying both, not by remembering.
    let lean = (rise / run).atan().to_degrees();
    let up_the_slope = (-run / slope, 0.0, rise / slope);
    let glacis_ang = [(0.0, 90.0, lean), (0.0, 90.0, -lean)]
        .into_iter()
        .find(|ang| {
            let short_side = transform::rotate_vec(*ang, (0.0, 1.0, 0.0));
            (short_side.0 * up_the_slope.0 + short_side.2 * up_the_slope.2).abs() > 0.999
        })
        .ok_or("the glacis could not be laid on its cheeks")?;

    let pieces: [(&str, Vec3, Vec3); 8] = [
        (&glacis_model, (nose_start + run / 2.0, 0.0, rise / 2.0), glacis_ang),
        (&cheek, (nose_start + run / 2.0, half_width, rise / 2.0), (0.0, 0.0, 0.0)),
        (&cheek, (nose_start + run / 2.0, -half_width, rise / 2.0), (0.0, 0.0, 0.0)),
        (&side_model, (body_middle, half_width, rise / 2.0), (0.0, 0.0, 90.0)),
        (&side_model, (body_middle, -half_width, rise / 2.0), (0.0, 0.0, 90.0)),
        (&deck_model, (body_middle, 0.0, rise), (0.0, 0.0, 0.0)),
        (&rear_model, (nose_start - body_length, 0.0, rise / 2.0), (0.0, 90.0, 90.0)),
        (&rear_model, (nose_start, 0.0, rise / 2.0), (0.0, 90.0, 90.0)),
    ];
    let mut made = Vec::new();
    // The last piece is a bulkhead behind the nose; a hull does without it.
    for (model, local_at, local_ang) in &pieces[..7] {
        made.push(place(dupe, baseplate, model, *local_at, *local_ang, thickness_mm)?);
    }
    Ok((made, Glacis::at((run / rise).atan().to_degrees())))
}

/// Builds the five plates round a baseplate and returns their entities.
/// A plate's long side lies along its own x, its short side along y, and it
/// is 3 thick along z. Positions and angles are worked out in the
/// baseplate's frame, where x is its length, and then carried into the
/// world, so a hull comes out square to a baseplate at any angle.
pub fn add_box_hull(dupe: &mut Dupe, baseplate: f64, height: f64, thickness_mm: f64) -> Result<Vec<f64>, String> {
    let et = transform::entity_table(dupe, baseplate).ok_or_else(|| format!("no entity {baseplate}"))?;
    let ud = dupe.get_table(et, "ACF_UserData");
    let number = |key: &str| ud.and_then(|ud| dupe.get_number(ud, key)).or_else(|| dupe.get_number(et, key));
    let (length, width) = (number("Length").unwrap_or(96.0), number("Width").unwrap_or(36.0));
    let (base_at, base_ang) = transform::entity_transform(dupe, baseplate).ok_or("the baseplate has no position")?;

    let (side_model, side_height, side_length) = plate_model(height, length).ok_or("the baseplate is too short for a side plate")?;
    let (end_model, end_short, end_long) = plate_model(height, width).ok_or("the baseplate is too narrow for a front plate")?;
    let (deck_model, deck_width, deck_length) = plate_model(width, length).ok_or("the baseplate is too small for a deck")?;
    // The ends stand between the sides, so the box closes on the plates'
    // real sizes rather than on the numbers that were asked for.
    let (end_width, end_height) = if end_long >= end_short && width >= height { (end_long, end_short) } else { (end_short, end_long) };
    let half_width = end_width / 2.0;
    let half_length = side_length.min(deck_length) / 2.0;
    let _ = deck_width;

    let plates: [(&str, Vec3, Vec3); 5] = [
        (&side_model, (0.0, half_width, side_height / 2.0), (0.0, 0.0, 90.0)),
        (&side_model, (0.0, -half_width, side_height / 2.0), (0.0, 0.0, 90.0)),
        (&end_model, (half_length, 0.0, end_height / 2.0), (0.0, 90.0, 90.0)),
        (&end_model, (-half_length, 0.0, end_height / 2.0), (0.0, 90.0, 90.0)),
        (&deck_model, (0.0, 0.0, side_height), (0.0, 0.0, 0.0)),
    ];
    let mut made = Vec::new();
    for (model, local_at, local_ang) in plates {
        let offset = transform::rotate_vec(base_ang, local_at);
        let at = (base_at.0 + offset.0, base_at.1 + offset.1, base_at.2 + offset.2);
        let plate = build::spawn_prop(dupe, model, at, transform::compose_angles(base_ang, local_ang))?;
        build::set_armour(dupe, plate, thickness_mm, 0.0)?;
        build::set_parent(dupe, plate, baseplate)?;
        made.push(plate);
    }
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6 && (a.2 - b.2).abs() < 1e-6
    }

    #[test]
    fn a_plate_is_the_largest_that_fits_and_is_named_the_way_sprops_names_it() {
        assert_eq!(plate_model(36.0, 200.0), Some(("models/sprops/rectangles/size_4/rect_36x192x3.mdl".into(), 36.0, 192.0)));
        assert_eq!(plate_model(200.0, 36.0), plate_model(36.0, 200.0));
        assert_eq!(plate_model(96.0, 200.0), Some(("models/sprops/rectangles/size_6/rect_96x192x3.mdl".into(), 96.0, 192.0)));
        assert_eq!(plate_model(90.0, 96.0), Some(("models/sprops/rectangles/size_72/rect_72x96x3.mdl".into(), 72.0, 96.0)));
        assert_eq!(plate_model(40.0, 40.0), Some(("models/sprops/rectangles/size_4/rect_36x36x3.mdl".into(), 36.0, 36.0)));
        assert_eq!(plate_model(5.0, 200.0), None);
        assert!(plate_models().contains(&"models/sprops/rectangles/size_4_5/rect_42x84x3.mdl".to_owned()));
    }

    #[test]
    fn a_sloped_hull_lays_its_glacis_on_two_cheeks_and_says_what_the_slope_is_worth() {
        let mut d = crate::buildfile::empty_dupe();
        let item = crate::catalog::find("GroundVehicle").unwrap();
        let size = [("Width", 96.0), ("Length", 200.0), ("Thickness", 2.0)];
        let base = build::spawn_catalog(&mut d, item, (0.0, 0.0, 0.0), (0.0, 90.0, 0.0), &size).unwrap();
        crate::extras::repair_head(&mut d);
        let (made, glacis) = add_sloped_hull(&mut d, base, 36.0, 60.0, 25.0).unwrap();
        assert_eq!(made.len(), 7);
        // 36 up and 60 along is the nearest SProps has to 60 degrees.
        assert!((glacis.degrees - (60.0f64 / 36.0).atan().to_degrees()).abs() < 1e-9 && glacis.degrees > 58.0 && glacis.degrees < 60.0, "{glacis:?}");
        assert!(glacis.thicker_by > 1.9 && glacis.thicker_by < 2.0 && glacis.ap_glances > 0.35 && glacis.ap_glances < 0.5, "{glacis:?}");

        let model = |plate: f64| match d.get(transform::entity_table(&d, plate).unwrap(), "Model") {
            Some(Value::Str(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
            _ => String::new(),
        };
        assert_eq!(model(made[1]), "models/sprops/triangles/right/size_3/rtri_36x60.mdl");
        assert_eq!(model(made[0]), "models/sprops/rectangles/size_66/rect_66x96x3.mdl", "the largest plate inside the 70 long slope");

        // The baseplate is at yaw 90, so the nose is at high world y. The
        // glacis faces forwards and upwards, square to the cheeks' slope.
        let (at, ang) = transform::entity_transform(&d, made[0]).unwrap();
        assert!(at.1 > 60.0 && at.0.abs() < 1e-6, "{at:?}");
        let normal = transform::rotate_vec(ang, (0.0, 0.0, 1.0));
        let facing = if normal.2 < 0.0 { (-normal.0, -normal.1, -normal.2) } else { normal };
        let expected = (0.0, 36.0 / 70.0_f64.hypot(0.0).max(1.0), 60.0 / 70.0);
        let slope = (36.0f64 * 36.0 + 60.0 * 60.0).sqrt();
        assert!(close(facing, (0.0, 36.0 / slope, 60.0 / slope)), "{facing:?} {expected:?}");
        // The cheeks stand either side of it with their slope falling forwards.
        let (left, right) = (transform::entity_transform(&d, made[1]).unwrap(), transform::entity_transform(&d, made[2]).unwrap());
        assert!((left.0 .0 + right.0 .0).abs() < 1e-6 && (left.0 .0 - right.0 .0).abs() > 90.0 && (left.0 .1 - at.1).abs() < 1e-6);
        assert!(transform::rotate_vec(left.1, (1.0, 0.0, 0.0)).1 > 0.999, "a cheek's long leg runs forwards");
        assert_eq!(crate::extras::validate(&d), Vec::<String>::new());
        assert_eq!(crate::doctor::doctor(&d).iter().filter(|finding| finding.severity == crate::doctor::Severity::Error).count(), 0);
    }

    #[test]
    fn a_steeper_glacis_is_asked_for_in_degrees_and_a_small_plate_says_why_it_cannot_have_one() {
        let hull = |width: f64, length: f64, degrees: f64| {
            let mut d = crate::buildfile::empty_dupe();
            let item = crate::catalog::find("GroundVehicle").unwrap();
            let base = build::spawn_catalog(&mut d, item, (0.0, 0.0, 0.0), (0.0, 90.0, 0.0), &[("Width", width), ("Length", length), ("Thickness", 2.0)]).unwrap();
            add_sloped_hull(&mut d, base, 36.0, degrees, 20.0).map(|(_, glacis)| glacis)
        };
        let steep = hull(120.0, 240.0, 68.0).unwrap();
        assert!(steep.degrees > 66.0 && steep.degrees < 70.0 && steep.ap_glances > 0.8, "{steep:?}");
        assert!(hull(36.0, 48.0, 60.0).is_err());
        assert!(triangle_models().len() > 100 && triangle_model(36, 24).is_none());
    }

    #[test]
    fn the_plate_angles_stand_the_sides_and_ends_upright() {
        // A side: long side along the hull, short side up, face outwards.
        assert!(close(transform::rotate_vec((0.0, 0.0, 90.0), (1.0, 0.0, 0.0)), (1.0, 0.0, 0.0)));
        assert!(transform::rotate_vec((0.0, 0.0, 90.0), (0.0, 1.0, 0.0)).2.abs() > 0.999);
        // An end: long side across the hull, short side up.
        assert!(transform::rotate_vec((0.0, 90.0, 90.0), (1.0, 0.0, 0.0)).1.abs() > 0.999);
        assert!(transform::rotate_vec((0.0, 90.0, 90.0), (0.0, 1.0, 0.0)).2.abs() > 0.999);
    }

    #[test]
    fn a_box_hull_is_five_armoured_plates_parented_to_the_baseplate_and_still_valid() {
        let mut d = crate::buildfile::empty_dupe();
        let item = crate::catalog::find("GroundVehicle").unwrap();
        let size = [("Width", 96.0), ("Length", 200.0), ("Thickness", 2.0)];
        let base = build::spawn_catalog(&mut d, item, (0.0, 0.0, 0.0), (0.0, 90.0, 0.0), &size).unwrap();
        crate::extras::repair_head(&mut d);
        let plates = add_box_hull(&mut d, base, 36.0, 25.0).unwrap();
        assert_eq!(plates.len(), 5);
        for plate in &plates {
            let et = transform::entity_table(&d, *plate).unwrap();
            let mods = d.get_table(et, "EntityMods").unwrap();
            let armour = d.get_table(mods, "ACF_Armor").expect("armoured");
            assert_eq!(d.get_number(armour, "Thickness"), Some(25.0));
            assert!(d.get(mods, "mass").is_none(), "a mass modifier would make ACF ignore the thickness");
            assert!(matches!(d.get(et, "Class"), Some(Value::Str(class)) if class.as_slice() == b"prop_physics"));
        }
        // The baseplate is at yaw 90, so its length runs along world y: the
        // two ends sit fore and aft on y, the two sides left and right on x.
        let at = |plate: f64| transform::entity_transform(&d, plate).unwrap().0;
        assert!(at(plates[0]).0.abs() > 30.0 && at(plates[0]).1.abs() < 1e-6);
        assert!(at(plates[2]).1.abs() > 90.0 && at(plates[2]).0.abs() < 1e-6);
        assert!(close(at(plates[4]), (0.0, 0.0, 36.0)));
        assert!(crate::extras::validate(&d).is_empty());
        assert!(crate::refs::dangling(&d).is_empty());
        assert!(crate::roles::of_dupe(&d).iter().filter(|(_, role)| *role == crate::roles::Role::ArmourPlate).count() == 5);
    }
}
