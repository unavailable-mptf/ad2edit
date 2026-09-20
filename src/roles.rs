//! What each entity is for, worked out from the dupe alone. A role is
//! never stored: a dupe exported from the game has no role keys and never
//! will, so class, links and modifiers have to be enough. The conventions
//! are the ones every reference build follows (ACF-NOTES.md).

use crate::dupe::Dupe;
use crate::transform;
use crate::value::{Node, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Hull,
    RoadWheel,
    Engine,
    FuelTank,
    Gearbox,
    FinalDrive,
    TurretRing,
    Trunnion,
    TurretMotor,
    MainGun,
    SecondaryGun,
    AmmoCrate,
    Driver,
    Gunner,
    Loader,
    Crew,
    Controller,
    Seat,
    Chip,
    WirePart,
    ArmourPlate,
    OtherAcf,
    Prop,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Hull => "hull",
            Role::RoadWheel => "road wheel",
            Role::Engine => "engine",
            Role::FuelTank => "fuel tank",
            Role::Gearbox => "gearbox",
            Role::FinalDrive => "final drive",
            Role::TurretRing => "turret ring",
            Role::Trunnion => "trunnion",
            Role::TurretMotor => "turret motor",
            Role::MainGun => "main gun",
            Role::SecondaryGun => "secondary gun",
            Role::AmmoCrate => "ammo crate",
            Role::Driver => "driver",
            Role::Gunner => "gunner",
            Role::Loader => "loader",
            Role::Crew => "crew",
            Role::Controller => "controller",
            Role::Seat => "seat",
            Role::Chip => "chip",
            Role::WirePart => "wire part",
            Role::ArmourPlate => "armour plate",
            Role::OtherAcf => "ACF part",
            Role::Prop => "prop",
        }
    }
}

fn text(dupe: &Dupe, table: usize, key: &str) -> String {
    match dupe.get(table, key) {
        Some(Value::Str(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
        _ => String::new(),
    }
}

fn linked(dupe: &Dupe, owner: f64, modifier: &str) -> Vec<f64> {
    let list = transform::entity_table(dupe, owner)
        .and_then(|et| dupe.get_table(et, "EntityMods"))
        .and_then(|mods| dupe.get_table(mods, modifier));
    let number = |value: &Value| if let Value::Number(n) = value { Some(*n) } else { None };
    match list.map(|list| &dupe.arena[list]) {
        Some(Node::Array(items)) => items.iter().filter_map(number).collect(),
        Some(Node::Table(entries)) => entries.iter().filter_map(|(_, value)| number(value)).collect(),
        None => Vec::new(),
    }
}

/// Every entity's role, in the dupe's own order.
pub fn of_dupe(dupe: &Dupe) -> Vec<(f64, Role)> {
    let entities = dupe.list_entities();
    let class_of = |index: f64| {
        transform::entity_table(dupe, index).map(|et| text(dupe, et, "Class").to_ascii_lowercase()).unwrap_or_default()
    };
    let mut driven_by_engine = BTreeSet::new();
    let mut driven_by_gearbox = BTreeSet::new();
    let mut wheels = BTreeSet::new();
    for (index, _, _) in &entities {
        match class_of(*index).as_str() {
            "acf_engine" => driven_by_engine.extend(linked(dupe, *index, "ACFGearboxes").into_iter().map(f64::to_bits)),
            "acf_gearbox" => {
                driven_by_gearbox.extend(linked(dupe, *index, "ACFGearboxes").into_iter().map(f64::to_bits));
                wheels.extend(linked(dupe, *index, "ACFWheels").into_iter().map(f64::to_bits));
            }
            _ => {}
        }
    }
    // The main gun is the biggest; a coaxial or an autocannon is secondary.
    let biggest_gun = entities
        .iter()
        .filter(|(index, _, _)| class_of(*index) == "acf_gun")
        .map(|(index, _, _)| (*index, transform::entity_table(dupe, *index).and_then(|et| dupe.get_number(et, "Caliber")).unwrap_or(0.0)))
        .fold(None, |best: Option<(f64, f64)>, gun| match best {
            Some(best) if best.1 >= gun.1 => Some(best),
            _ => Some(gun),
        })
        .map(|(index, _)| index);

    entities
        .iter()
        .map(|(index, _, _)| {
            let class = class_of(*index);
            let et = transform::entity_table(dupe, *index);
            let field = |key: &str| et.map(|et| text(dupe, et, key)).unwrap_or_default();
            let modifier = |name: &str| et.and_then(|et| dupe.get_table(et, "EntityMods")).is_some_and(|mods| dupe.get(mods, name).is_some());
            let role = match class.as_str() {
                "acf_baseplate" => Role::Hull,
                "acf_engine" => Role::Engine,
                "acf_fueltank" => Role::FuelTank,
                "acf_gearbox" if driven_by_gearbox.contains(&index.to_bits()) && !driven_by_engine.contains(&index.to_bits()) => Role::FinalDrive,
                "acf_gearbox" => Role::Gearbox,
                "acf_turret" if field("Turret").eq_ignore_ascii_case("Turret-V") => Role::Trunnion,
                "acf_turret" => Role::TurretRing,
                "acf_turret_motor" => Role::TurretMotor,
                "acf_gun" if biggest_gun == Some(*index) => Role::MainGun,
                "acf_gun" => Role::SecondaryGun,
                "acf_ammo" => Role::AmmoCrate,
                "acf_crew" => match field("CrewTypeID").to_ascii_lowercase().as_str() {
                    "driver" => Role::Driver,
                    "gunner" => Role::Gunner,
                    "loader" => Role::Loader,
                    _ => Role::Crew,
                },
                "acf_controller" => Role::Controller,
                "gmod_wire_expression2" => Role::Chip,
                _ if class.starts_with("starfall_") => Role::Chip,
                _ if class.starts_with("prop_vehicle_") => Role::Seat,
                _ if class.starts_with("gmod_wire_") => Role::WirePart,
                _ if class.starts_with("acf_") => Role::OtherAcf,
                _ if wheels.contains(&index.to_bits()) || modifier("MakeSphericalCollisions") => Role::RoadWheel,
                _ if modifier("ACF_Armor") => Role::ArmourPlate,
                _ => Role::Prop,
            };
            (*index, role)
        })
        .collect()
}

/// A role a tank needs that nothing in the build fills yet, and what to do
/// about it in the builder's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSlot {
    pub role: Role,
    pub advice: &'static str,
}

/// What a tank still lacks. Nothing is asked of a build with no ACF in it:
/// a drone or a door is not an unfinished tank.
pub fn open_slots(dupe: &Dupe) -> Vec<OpenSlot> {
    let roles: Vec<Role> = of_dupe(dupe).into_iter().map(|(_, role)| role).collect();
    let count = |role: Role| roles.iter().filter(|found| **found == role).count();
    let has = |role: Role| count(role) > 0;
    let acf = roles.iter().any(|role| !matches!(role, Role::Prop | Role::ArmourPlate | Role::RoadWheel | Role::Chip | Role::WirePart | Role::Seat));
    if !acf {
        return Vec::new();
    }
    let mut open = Vec::new();
    let mut need = |missing: bool, role: Role, advice: &'static str| {
        if missing {
            open.push(OpenSlot { role, advice });
        }
    };
    need(!has(Role::Hull), Role::Hull, "It needs a baseplate: Parts, Baseplates, or start from a preset.");
    need(count(Role::RoadWheel) < 4, Role::RoadWheel, "Fewer than four road wheels: Parts, Wheels. They hinge to the hull by themselves.");
    need(!has(Role::Engine), Role::Engine, "No engine yet: Parts, Engines. Then link it to a gearbox.");
    need(has(Role::Engine) && !has(Role::FuelTank), Role::FuelTank, "The engine has no fuel tank: Parts, Fuel tanks, then link the engine to it.");
    need(!has(Role::Gearbox), Role::Gearbox, "No gearbox: Parts, Gearboxes. A CVT into two transfer cases is the reference layout.");
    need(has(Role::Gearbox) && !has(Role::FinalDrive) && has(Role::RoadWheel), Role::FinalDrive, "Nothing carries power from the gearbox to the wheels: add a 2Gear-L per side and link gearbox, transfer, wheels.");
    need(!has(Role::MainGun), Role::MainGun, "No gun: Parts, Guns. Give it a crate with matching ammo.");
    need(has(Role::MainGun) && !has(Role::AmmoCrate), Role::AmmoCrate, "No ammo crate: Parts, Ammo types, then link the gun to it.");
    need(has(Role::TurretRing) && !has(Role::Trunnion), Role::Trunnion, "A ring with no trunnion: the gun needs a Turret-V on the ring to elevate.");
    need((has(Role::TurretRing) || has(Role::Trunnion)) && !has(Role::TurretMotor), Role::TurretMotor, "A turret with no motor turns by hand speed: Parts, Turrets, Motor, then link the ring to it.");
    need(!has(Role::Driver), Role::Driver, "No driver: Parts, Crew. The driver links the baseplate.");
    need(has(Role::MainGun) && !has(Role::Gunner), Role::Gunner, "No gunner: Parts, Crew. The gunner links the turret.");
    need(has(Role::MainGun) && !has(Role::Loader), Role::Loader, "No loader: Parts, Crew. The loader links the gun.");
    need(!has(Role::Controller), Role::Controller, "No controller: Parts, Control. It links the baseplate, gearbox, turrets and guns.");
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(d: &mut Dupe, index: f64, class: &str, fields: &[(&str, Value)]) -> usize {
        let root = d.root_table().unwrap();
        let ents = d.get_table(root, "Entities").unwrap();
        let et = d.new_table();
        d.set(et, "Class", Value::Str(class.as_bytes().to_vec()));
        for (key, value) in fields {
            d.set(et, key, value.clone());
        }
        if let Node::Table(e) = &mut d.arena[ents] {
            e.push((Value::Number(index), Value::Table(et)));
        }
        et
    }

    fn links(d: &mut Dupe, et: usize, modifier: &str, targets: &[f64]) {
        let mods = match d.get_table(et, "EntityMods") {
            Some(mods) => mods,
            None => {
                let mods = d.new_table();
                d.set(et, "EntityMods", Value::Table(mods));
                mods
            }
        };
        let list = d.new_array();
        if let Node::Array(items) = &mut d.arena[list] {
            items.extend(targets.iter().map(|target| Value::Number(*target)));
        }
        d.set(mods, modifier, Value::Table(list));
    }

    #[test]
    fn roles_come_from_class_links_and_modifiers() {
        let mut d = crate::buildfile::empty_dupe();
        entity(&mut d, 1.0, "acf_baseplate", &[]);
        let engine = entity(&mut d, 2.0, "acf_engine", &[]);
        let cvt = entity(&mut d, 3.0, "acf_gearbox", &[]);
        let transfer = entity(&mut d, 4.0, "acf_gearbox", &[]);
        entity(&mut d, 5.0, "prop_physics", &[]);
        let plate = entity(&mut d, 6.0, "prop_physics", &[]);
        entity(&mut d, 7.0, "acf_gun", &[("Caliber", Value::Number(100.0))]);
        entity(&mut d, 8.0, "acf_gun", &[("Caliber", Value::Number(20.0))]);
        entity(&mut d, 9.0, "acf_turret", &[("Turret", Value::Str(b"Turret-V".to_vec()))]);
        entity(&mut d, 10.0, "acf_crew", &[("CrewTypeID", Value::Str(b"Loader".to_vec()))]);
        entity(&mut d, 11.0, "prop_physics", &[]);
        links(&mut d, engine, "ACFGearboxes", &[3.0]);
        links(&mut d, cvt, "ACFGearboxes", &[4.0]);
        links(&mut d, transfer, "ACFWheels", &[5.0]);
        let (mods, armour) = (d.new_table(), d.new_table());
        d.set(mods, "ACF_Armor", Value::Table(armour));
        d.set(plate, "EntityMods", Value::Table(mods));

        let roles: Vec<Role> = of_dupe(&d).into_iter().map(|(_, role)| role).collect();
        assert_eq!(
            roles,
            [
                Role::Hull,
                Role::Engine,
                Role::Gearbox,
                Role::FinalDrive,
                Role::RoadWheel,
                Role::ArmourPlate,
                Role::MainGun,
                Role::SecondaryGun,
                Role::Trunnion,
                Role::Loader,
                Role::Prop,
            ]
        );
    }

    #[test]
    fn the_tank_presets_leave_no_slot_open_and_a_bare_hull_lists_what_is_missing() {
        for preset in ["light", "medium", "heavy"] {
            let spec = crate::gen::preset(preset).expect(preset).catalogue();
            let file: crate::buildfile::BuildFile = toml::from_str(&crate::gen::write_build_file(&spec)).expect(preset);
            let (dupe, _) = crate::buildfile::run(&file, std::path::Path::new(".")).expect(preset);
            let open = open_slots(&dupe);
            assert!(open.is_empty(), "{preset}: {open:?}");
        }

        let mut d = crate::buildfile::empty_dupe();
        entity(&mut d, 1.0, "acf_baseplate", &[]);
        let missing: Vec<Role> = open_slots(&d).into_iter().map(|slot| slot.role).collect();
        assert_eq!(missing, [Role::RoadWheel, Role::Engine, Role::Gearbox, Role::MainGun, Role::Driver, Role::Controller]);

        let mut drone = crate::buildfile::empty_dupe();
        entity(&mut drone, 1.0, "prop_physics", &[]);
        entity(&mut drone, 2.0, "gmod_wire_expression2", &[]);
        assert!(open_slots(&drone).is_empty(), "a drone is not an unfinished tank");
    }
}
