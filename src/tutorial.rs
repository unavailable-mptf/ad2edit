//! A tutorial that watches the build instead of talking at it: each step
//! is a fact about the dupe, so it ticks itself off the moment it is true
//! and cannot be ticked by hand. The first step not yet true is the one to
//! do next.

use crate::doctor::{self, Severity};
use crate::dupe::Dupe;
use crate::graph::{self, Edge};
use crate::roles::{self, Role};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub title: &'static str,
    /// What to do, in the editor's own words for where things are.
    pub how: &'static str,
    pub done: bool,
}

/// A first tank, from a bare hull to something that pastes. With no dupe
/// open every step is still to do.
pub fn first_tank(dupe: Option<&Dupe>) -> Vec<Step> {
    let roles: Vec<Role> = dupe.map(|dupe| roles::of_dupe(dupe).into_iter().map(|(_, role)| role).collect()).unwrap_or_default();
    let count = |wanted: Role| roles.iter().filter(|role| **role == wanted).count();
    let has = |wanted: Role| count(wanted) > 0;
    let edges = dupe.map(|dupe| graph::of_dupe(dupe, &[]).edges).unwrap_or_default();
    let linked = |wanted: &str| edges.iter().any(|edge| matches!(edge, Edge::Link { modifier, .. } if modifier == wanted));
    let wired = edges.iter().any(|edge| matches!(edge, Edge::Wire { .. }));
    let errors = dupe.map(|dupe| doctor::doctor(dupe).iter().filter(|finding| finding.severity == Severity::Error).count());

    let step = |title, how, done| Step { title, how, done };
    vec![
        step("Start with a hull", "File, New with a baseplate; or the Bare hull card in newcomer mode.", has(Role::Hull)),
        step("Give it wheels", "Add, Wheels. Four or more; each hinges to the hull by itself. Move them to the corners with the move handle.", count(Role::RoadWheel) >= 4),
        step(
            "Add the drivetrain",
            "Add an engine, a fuel tank, a gearbox (a CVT) and a final drive (a 2Gear-L) for each side.",
            has(Role::Engine) && has(Role::FuelTank) && has(Role::Gearbox),
        ),
        step(
            "Link the drivetrain",
            "In the Nodes tab drag from a link socket onto another node: engine to fuel tank, engine to gearbox, gearbox to final drive, final drive to its wheels.",
            linked("ACFGearboxes") && linked("ACFFuelTanks") && linked("ACFWheels"),
        ),
        step(
            "Mount the gun",
            "Add a turret ring, a trunnion, a turret motor, a gun and an ammo crate. Link the gun to its crate and each turret to its motor.",
            has(Role::TurretRing) && has(Role::Trunnion) && has(Role::MainGun) && has(Role::AmmoCrate) && linked("ACFCrates"),
        ),
        step("Crew it", "Add a driver, a gunner, a loader and a controller. Tune any of them under Tune this part.", has(Role::Driver) && has(Role::Gunner) && has(Role::Loader) && has(Role::Controller)),
        step("Armour it", "Build a hull (its sloped nose is what makes the armour work), set a weight under armour to, and press Plan it.", count(Role::ArmourPlate) >= 3),
        step("Wire something", "Optional for a tank that drives by its controller; in the Nodes tab drag an output onto an input. A refused wire says why.", wired),
        step("Make the doctor happy", "The checklist lists what would stop it pasting. Fix what it can, do the rest by hand.", has(Role::Hull) && errors == Some(0)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(dupe: Option<&Dupe>) -> Vec<bool> {
        first_tank(dupe).into_iter().map(|step| step.done).collect()
    }

    #[test]
    fn nothing_is_done_with_nothing_open_and_a_bare_hull_is_the_first_tick() {
        assert!(done(None).iter().all(|step| !step));
        let mut d = crate::buildfile::empty_dupe();
        let item = crate::catalog::find("GroundVehicle").unwrap();
        crate::build::spawn_catalog(&mut d, item, (0.0, 0.0, 0.0), (0.0, 90.0, 0.0), &[("Width", 96.0), ("Length", 200.0), ("Thickness", 2.0)]).unwrap();
        crate::extras::repair_head(&mut d);
        let ticks = done(Some(&d));
        assert!(ticks[0], "the hull is there");
        assert!(ticks[1..7].iter().all(|step| !step), "{ticks:?}");
    }

    #[test]
    fn a_preset_tank_has_done_everything_but_the_armour_and_the_wiring() {
        let mut d = crate::buildfile::preset_dupe("light").unwrap();
        let ticks = done(Some(&d));
        assert_eq!(&ticks[..6], &[true; 6], "{ticks:?}");
        assert!(!ticks[6], "a preset has no armour plates");
        assert!(ticks[8], "and it passes the doctor");

        let hull = crate::roles::of_dupe(&d).into_iter().find(|(_, role)| *role == Role::Hull).unwrap().0;
        crate::hull::add_sloped_hull(&mut d, hull, 36.0, 60.0, 20.0).unwrap();
        assert!(done(Some(&d))[6]);
    }
}
