//! Where a turret can point: the sweep a ring's traverse limits allow and
//! the fan a trunnion's elevation limits allow, as lines in the world for
//! the viewport to draw. ACF's own convention (`entities/acf_turret/
//! init.lua`): the limits are in the turret's own frame, a ring's positive
//! degrees turn it to its right, a trunnion's raise the gun, a trunnion is
//! held to 85 either way, and a ring at -180 to 180 has no arc at all.

use crate::dupe::Dupe;
use crate::transform::{self, Vec3};
use crate::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Arc {
    pub turret: f64,
    pub pivot: Vec3,
    /// Up and down rather than round.
    pub elevation: bool,
    /// Degrees, lowest first.
    pub limits: (f64, f64),
    /// A ring that turns all the way round.
    pub unlimited: bool,
    /// The sweep at `reach` from the pivot, lowest limit first.
    pub points: Vec<Vec3>,
}

fn text(dupe: &Dupe, table: usize, key: &str) -> Option<String> {
    match dupe.get(table, key) {
        Some(Value::Str(bytes)) => Some(String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    }
}

/// Every turret's arc, drawn `reach` units out from its pivot.
pub fn of_dupe(dupe: &Dupe, reach: f64) -> Vec<Arc> {
    let mut found = Vec::new();
    for (index, class, _) in dupe.list_entities() {
        if class.trim_matches('"') != "acf_turret" {
            continue;
        }
        let Some(et) = transform::entity_table(dupe, index) else { continue };
        let Some((pivot, ang)) = transform::entity_transform(dupe, index) else { continue };
        let ud = dupe.get_table(et, "ACF_UserData");
        let number = |key: &str| dupe.get_number(et, key).or_else(|| ud.and_then(|ud| dupe.get_number(ud, key)));
        let elevation = text(dupe, et, "Turret").or_else(|| ud.and_then(|ud| text(dupe, ud, "Turret"))).is_some_and(|kind| kind.eq_ignore_ascii_case("Turret-V"));
        let (low, high) = (number("MinDeg").unwrap_or(-180.0), number("MaxDeg").unwrap_or(180.0));
        let (low, high) = if elevation { (low.max(-85.0), high.min(85.0)) } else { (low.max(-180.0), high.min(180.0)) };
        if high < low {
            continue;
        }
        let unlimited = !elevation && low <= -180.0 && high >= 180.0;
        let steps = (((high - low) / 5.0).ceil() as usize).max(1);
        let points = (0..=steps)
            .map(|step| {
                let degrees = (low + (high - low) * step as f64 / steps as f64).to_radians();
                let own = if elevation { (degrees.cos(), 0.0, degrees.sin()) } else { (degrees.cos(), -degrees.sin(), 0.0) };
                let out = transform::rotate_vec(ang, own);
                (pivot.0 + out.0 * reach, pivot.1 + out.1 * reach, pivot.2 + out.2 * reach)
            })
            .collect();
        found.push(Arc { turret: index, pivot, elevation, limits: (low, high), unlimited, points });
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turret(kind: &str, ang: Vec3, limits: Option<(f64, f64)>) -> (Dupe, f64) {
        let mut dupe = crate::buildfile::empty_dupe();
        let item = crate::catalog::find(kind).unwrap();
        let index = crate::build::spawn_catalog(&mut dupe, item, (10.0, 20.0, 30.0), ang, &[]).unwrap();
        let et = transform::entity_table(&dupe, index).unwrap();
        if let Some((low, high)) = limits {
            dupe.set(et, "MinDeg", Value::Number(low));
            dupe.set(et, "MaxDeg", Value::Number(high));
        }
        (dupe, index)
    }

    #[test]
    fn a_trunnion_fans_from_its_depression_up_to_its_elevation() {
        let (dupe, index) = turret("Turret-V", (0.0, 0.0, 0.0), Some((-7.0, 20.0)));
        let arc = of_dupe(&dupe, 100.0).into_iter().find(|arc| arc.turret == index).unwrap();
        assert!(arc.elevation && !arc.unlimited && arc.limits == (-7.0, 20.0));
        let (first, last) = (arc.points[0], *arc.points.last().unwrap());
        assert!((first.2 - (30.0 - 100.0 * 7f64.to_radians().sin())).abs() < 1e-6, "{first:?}");
        assert!((last.2 - (30.0 + 100.0 * 20f64.to_radians().sin())).abs() < 1e-6 && last.0 > 100.0, "{last:?}");
        assert!(arc.points.iter().all(|point| (point.1 - 20.0).abs() < 1e-9), "it fans in its own upright plane");
    }

    #[test]
    fn a_trunnion_is_held_to_85_and_follows_the_way_it_is_turned() {
        let (dupe, _) = turret("Turret-V", (0.0, 90.0, 0.0), Some((-120.0, 120.0)));
        let arc = &of_dupe(&dupe, 50.0)[0];
        assert_eq!(arc.limits, (-85.0, 85.0));
        let level = arc.points[arc.points.len() / 2];
        assert!((level.1 - 70.0).abs() < 1e-6 && (level.0 - 10.0).abs() < 1e-6, "yawed 90, level points along world y: {level:?}");
    }

    #[test]
    fn a_ring_sweeps_to_its_right_for_positive_degrees_and_a_free_one_says_so() {
        let (dupe, _) = turret("Turret-H", (0.0, 0.0, 0.0), Some((-30.0, 60.0)));
        let arc = &of_dupe(&dupe, 100.0)[0];
        assert!(!arc.elevation && !arc.unlimited);
        let (first, last) = (arc.points[0], *arc.points.last().unwrap());
        assert!(first.1 > 20.0 && last.1 < 20.0, "-30 is to its left (+y), +60 to its right: {first:?} {last:?}");
        assert!(arc.points.iter().all(|point| (point.2 - 30.0).abs() < 1e-9));

        let (dupe, _) = turret("Turret-H", (0.0, 0.0, 0.0), None);
        assert!(of_dupe(&dupe, 100.0)[0].unlimited);
    }
}
