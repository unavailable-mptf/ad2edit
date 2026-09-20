//! What a contraption weighs, part by part, as the game will weigh it on
//! paste: a `mass` modifier, a plate's armour, a container's walls and
//! contents, a gun by its calibre, an engine from the catalogue, crew; and
//! where none of those apply, the weight the game itself saved with the
//! part. Whatever is left is counted as unknown, never guessed.
//! `ad2read d.txt --mass-check` holds every rule here to the saved weights.

use std::collections::BTreeMap;

use crate::containers;
use crate::dupe::Dupe;
use crate::roles::{self, Role};
use crate::transform::{self, Vec3};
use crate::value::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// `EntityMods.mass`, which the paste applies as it is.
    Modifier,
    /// `ACF_Armor` thickness over the model's surface.
    Armour,
    /// A fuel tank full, or a crate full of its rounds.
    Container,
    Gun,
    Crew,
    Catalogue,
    /// `ACF.Mass` or `_mass` as the game saved it; right until the part is changed.
    Saved,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartMass {
    pub index: f64,
    pub kg: f64,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MassReport {
    pub parts: Vec<PartMass>,
    /// Entities whose weight nothing here can tell: a plain prop keeps its
    /// model's own physics weight, which is not in a dupe.
    pub unknown: Vec<f64>,
}

impl MassReport {
    pub fn total_kg(&self) -> f64 {
        self.parts.iter().map(|part| part.kg).sum()
    }

    pub fn by_role(&self, dupe: &Dupe) -> BTreeMap<&'static str, f64> {
        let roles: BTreeMap<u64, Role> = roles::of_dupe(dupe).into_iter().map(|(index, role)| (index.to_bits(), role)).collect();
        let mut totals = BTreeMap::new();
        for part in &self.parts {
            let name = roles.get(&part.index.to_bits()).map_or("other", |role| role.name());
            *totals.entry(name).or_insert(0.0) += part.kg;
        }
        totals
    }

    /// "12.4 t", or "12.4 t + 3 parts unknown".
    pub fn headline(&self) -> String {
        let total = self.total_kg();
        let weight = if total >= 1000.0 { format!("{:.1} t", total / 1000.0) } else { format!("{total:.0} kg") };
        match self.unknown.len() {
            0 => weight,
            1 => format!("{weight} + 1 part unknown"),
            count => format!("{weight} + {count} parts unknown"),
        }
    }
}

/// (class id, kg at the base calibre, base calibre mm), from each gun class's
/// `CLASS.Mass` and `CaliberLimits.Base`; a gun weighs that times the cube
/// of its calibre over the base (`entities/acf_gun/init.lua`, GetMass).
const GUN_CLASSES: &[(&str, f64, f64)] = &[
    ("C", 2031.0, 100.0),
    ("SC", 1195.0, 100.0),
    ("AC", 1953.0, 50.0),
    ("LAC", 301.0, 40.0),
    ("RAC", 212.0, 20.0),
    ("MG", 53.0, 20.0),
    ("SA", 453.0, 45.0),
    ("HW", 860.0, 105.0),
    ("MO", 459.0, 120.0),
    ("GL", 101.0, 40.0),
    ("SL", 3.77, 40.0),
    ("FGL", 75.0, 40.0),
];

const STEEL_KG_PER_CUBIC_CM: f64 = 0.0079;
const PROPELLANT_KG_PER_CUBIC_CM: f64 = 0.00095;

fn text(dupe: &Dupe, table: usize, key: &str) -> Option<String> {
    match dupe.get(table, key) {
        Some(Value::Str(bytes)) => Some(String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    }
}

/// One cartridge. The game's own figure when the crate still holds the
/// round it was saved with; otherwise propellant plus a steel projectile,
/// narrowed for the sub-calibre kinds. That is exact for AP and up to a
/// fifth heavy for a round that is half filler.
fn cartridge_mass(dupe: &Dupe, et: usize, ammo: &containers::AmmoCrate) -> f64 {
    let same = |a: f64, b: f64| (a - b).abs() < 1e-3 * b.abs().max(1.0);
    let saved = dupe.get_table(et, "BulletData").and_then(|bullet| {
        let unchanged = text(dupe, bullet, "Type").as_deref() == Some(ammo.ammo_type.as_str())
            && same(dupe.get_number(bullet, "ProjLength")?, ammo.projectile)
            && same(dupe.get_number(bullet, "PropLength")?, ammo.propellant)
            && same(dupe.get_number(bullet, "Caliber")? * 10.0, ammo.caliber);
        dupe.get_number(bullet, "CartMass").filter(|_| unchanged)
    });
    saved.unwrap_or_else(|| {
        let area = std::f64::consts::PI * (ammo.caliber / 20.0).powi(2);
        let narrowed: f64 = match ammo.ammo_type.as_str() {
            "APCR" => 0.75,
            "APDS" => 0.45,
            "APFSDS" => 0.35,
            _ => 1.0,
        };
        area * (ammo.projectile * narrowed.powi(2) * STEEL_KG_PER_CUBIC_CM + ammo.propellant * PROPELLANT_KG_PER_CUBIC_CM)
    })
}

/// The weight the game saved with the part: ACF's own figure, or the one
/// the contraption framework keeps as `_mass`.
pub fn saved(dupe: &Dupe, index: f64) -> Option<f64> {
    let et = transform::entity_table(dupe, index)?;
    let acf = dupe.get_table(et, "ACF").and_then(|acf| dupe.get_number(acf, "Mass"));
    acf.or_else(|| dupe.get_number(et, "_mass")).filter(|kg| *kg > 0.0)
}

/// The weight worked out from the part's own numbers, or None when it has
/// none that say. `half_extents` gives a model's half size for a plate.
pub fn worked_out(dupe: &Dupe, index: f64, half_extents: &mut dyn FnMut(&str) -> Option<Vec3>) -> Option<(f64, Source)> {
    let et = transform::entity_table(dupe, index)?;
    let class = text(dupe, et, "Class").unwrap_or_default();
    let mods = dupe.get_table(et, "EntityMods");
    // An ACF part weighs what ACF says whatever modifiers it carries: the
    // reference gearboxes all carry a `mass` of 1 and weigh hundreds.
    if !class.starts_with("acf_") {
        let armour = mods.and_then(|mods| dupe.get_table(mods, "ACF_Armor")).and_then(|armour| Some((dupe.get_number(armour, "Thickness")?, dupe.get_number(armour, "Ductility").unwrap_or(0.0))));
        if let Some((thickness, ductility)) = armour {
            // The surface the game measured on the physics mesh, when it
            // saved one; a model's bounding box runs a few per cent over
            // and far over for anything that is not a box.
            let measured = dupe.get_table(et, "ACF").and_then(|acf| dupe.get_number(acf, "Area")).filter(|area| *area > 0.0);
            let kg = match measured {
                Some(area) => area * (1.0 + ductility / 100.0).max(0.0).sqrt() * thickness * 0.00078,
                None => {
                    let model = text(dupe, et, "Model").unwrap_or_default();
                    let half = half_extents(&model).or_else(|| crate::build::sprops_half_extents(&model))?;
                    crate::build::armour_mass(crate::build::box_surface(half), thickness, ductility)
                }
            };
            return Some((kg, Source::Armour));
        }
        if let Some(kg) = mods.and_then(|mods| dupe.get_table(mods, "mass")).and_then(|mass| dupe.get_number(mass, "Mass")) {
            return Some((kg, Source::Modifier));
        }
    }
    match class.as_str() {
        "acf_fueltank" => containers::tank(dupe, index).map(|tank| (tank.full_mass(), Source::Container)),
        "acf_ammo" => containers::ammo_crate(dupe, index).map(|ammo| (ammo.empty_mass() + ammo.rounds() as f64 * cartridge_mass(dupe, et, &ammo), Source::Container)),
        "acf_gun" => {
            let weapon = text(dupe, et, "Weapon")?;
            let (_, at_base, base) = GUN_CLASSES.iter().find(|(id, _, _)| *id == weapon)?;
            let caliber = dupe.get_number(et, "Caliber").filter(|caliber| *caliber > 0.0).unwrap_or(*base);
            Some(((at_base * (caliber / base).powi(3)).round(), Source::Gun))
        }
        "acf_crew" => Some((if text(dupe, et, "CrewTypeID").as_deref() == Some("Pilot") { 200.0 } else { 80.0 }, Source::Crew)),
        "acf_engine" => {
            let id = text(dupe, et, "Engine")?;
            let item = crate::catalog::find(&id).filter(|item| item.entity == "acf_engine")?;
            Some((item.mass?, Source::Catalogue))
        }
        _ => None,
    }
}

pub fn of_dupe(dupe: &Dupe, half_extents: &mut dyn FnMut(&str) -> Option<Vec3>) -> MassReport {
    let mut report = MassReport::default();
    for (index, _, _) in dupe.list_entities() {
        match worked_out(dupe, index, half_extents).or_else(|| saved(dupe, index).map(|kg| (kg, Source::Saved))) {
            Some((kg, source)) => report.parts.push(PartMass { index, kg, source }),
            None => report.unknown.push(index),
        }
    }
    report
}

/// Every part whose saved weight disagrees with the one worked out by more
/// than `tolerance` of it, as lines for the CLI; and how many agreed.
pub fn disagreements(dupe: &Dupe, half_extents: &mut dyn FnMut(&str) -> Option<Vec3>, tolerance: f64) -> (usize, Vec<String>) {
    let mut agreed = 0;
    let mut lines = Vec::new();
    for (index, class, _) in dupe.list_entities() {
        let (Some(was), Some((mut now, source))) = (saved(dupe, index), worked_out(dupe, index, half_extents)) else { continue };
        // A crate is weighed full; the game saved it with however many
        // rounds it held at that moment, usually one short.
        if let (Some(et), Some(ammo)) = (transform::entity_table(dupe, index), containers::ammo_crate(dupe, index)) {
            if let Some(held) = dupe.get_number(et, "Ammo") {
                now -= (ammo.rounds() as f64 - held) * cartridge_mass(dupe, et, &ammo);
            }
        }
        if (was - now).abs() <= tolerance * was.max(1.0) {
            agreed += 1;
        } else {
            lines.push(format!("{index} {}: saved {was:.1} kg, worked out {now:.1} kg ({source:?})", class.trim_matches('"')));
        }
    }
    (agreed, lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_models(_: &str) -> Option<Vec3> {
        None
    }

    #[test]
    fn a_preset_tank_is_weighed_part_by_part_and_says_what_it_cannot_tell() {
        let dupe = crate::buildfile::preset_dupe("light").unwrap();
        let report = of_dupe(&dupe, &mut no_models);
        let sources: Vec<Source> = report.parts.iter().map(|part| part.source).collect();
        for wanted in [Source::Gun, Source::Container, Source::Crew, Source::Catalogue, Source::Modifier] {
            assert!(sources.contains(&wanted), "{wanted:?} missing from {sources:?}");
        }
        assert_eq!(report.parts.len() + report.unknown.len(), dupe.list_entities().len());
        assert!(report.total_kg() > 3000.0, "a 125 mm cannon alone is nearly four tonnes: {}", report.total_kg());
        assert!(report.headline().contains(" t"));
        let roles = report.by_role(&dupe);
        assert!(roles["main gun"] > 3900.0 && roles["main gun"] < 4000.0, "{roles:?}");
    }

    #[test]
    fn a_gun_weighs_its_class_by_the_cube_of_its_calibre() {
        let mut dupe = crate::buildfile::empty_dupe();
        let cannon = crate::catalog::find("C").unwrap();
        let index = crate::build::spawn_catalog(&mut dupe, cannon, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), &[("Caliber", 125.0)]).unwrap();
        assert_eq!(worked_out(&dupe, index, &mut no_models), Some((3967.0, Source::Gun)));
    }

    #[test]
    fn armour_beats_a_mass_modifier_as_it_does_on_paste_and_the_saved_weight_is_the_last_resort() {
        let mut dupe = crate::buildfile::empty_dupe();
        let plate = crate::build::spawn_prop(&mut dupe, "models/sprops/rectangles/size_3/rect_24x48x3.mdl", (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)).unwrap();
        assert_eq!(of_dupe(&dupe, &mut no_models).unknown, vec![plate], "a bare prop's weight is not in a dupe");

        let et = transform::entity_table(&dupe, plate).unwrap();
        let acf = dupe.new_table();
        dupe.set(acf, "Mass", Value::Number(61.5));
        dupe.set(et, "ACF", Value::Table(acf));
        assert_eq!(of_dupe(&dupe, &mut no_models).parts, vec![PartMass { index: plate, kg: 61.5, source: Source::Saved }]);

        let mods = dupe.get_table(et, "EntityMods").unwrap();
        let mass = dupe.new_table();
        dupe.set(mass, "Mass", Value::Number(40.0));
        dupe.set(mods, "mass", Value::Table(mass));
        assert_eq!(worked_out(&dupe, plate, &mut no_models), Some((40.0, Source::Modifier)));

        let armour = dupe.new_table();
        dupe.set(armour, "Thickness", Value::Number(50.0));
        dupe.set(armour, "Ductility", Value::Number(0.0));
        dupe.set(mods, "ACF_Armor", Value::Table(armour));
        let (kg, source) = worked_out(&dupe, plate, &mut no_models).unwrap();
        assert_eq!(source, Source::Armour);
        assert!(kg > 300.0 && kg < 400.0, "a 24x48 plate at 50 mm: {kg}");
    }

    #[test]
    fn a_crate_weighs_its_walls_and_its_rounds_with_the_games_own_cartridge_when_it_still_fits() {
        let mut dupe = crate::buildfile::empty_dupe();
        let index = crate::build::spawn_catalog(&mut dupe, crate::catalog::find("AP").unwrap(), (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), &[]).unwrap();
        let lac = containers::AmmoCrate { weapon: "LAC".into(), caliber: 40.0, ammo_type: "APHE".into(), projectile: 6.0, propellant: 19.46, tracer: false, counts: (2, 32, 5), drum: false };
        containers::set_ammo_crate(&mut dupe, index, &lac).unwrap();
        let (estimated, _) = worked_out(&dupe, index, &mut no_models).unwrap();
        assert!((lac.empty_mass() - 71.866).abs() < 0.05, "{}", lac.empty_mass());
        assert!(estimated > 295.0 && estimated < 295.0 * 1.2, "solid steel where half was filler: {estimated}");

        // ironlock's crate as the game saved it: 0.7005 kg a round.
        let et = transform::entity_table(&dupe, index).unwrap();
        let bullet = dupe.new_table();
        for (key, value) in [("ProjLength", 6.0), ("PropLength", 19.46), ("Caliber", 4.0), ("CartMass", 0.7004833026634127)] {
            dupe.set(bullet, key, Value::Number(value));
        }
        dupe.set(bullet, "Type", Value::Str(b"APHE".to_vec()));
        dupe.set(et, "BulletData", Value::Table(bullet));
        let (exact, _) = worked_out(&dupe, index, &mut no_models).unwrap();
        assert!((exact - (71.866 + 320.0 * 0.7004833)).abs() < 0.1, "{exact}");

        // Change the round and the saved cartridge no longer describes it.
        containers::set_ammo_crate(&mut dupe, index, &containers::AmmoCrate { propellant: 10.0, ..lac }).unwrap();
        let (changed, _) = worked_out(&dupe, index, &mut no_models).unwrap();
        assert!(changed < estimated && (changed - exact).abs() > 1.0, "{changed}");
    }
}
