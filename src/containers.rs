//! What ACF makes of a fuel tank's and an ammo crate's numbers, so a tank
//! can be sized from the litres wanted and a crate from the rounds wanted.
//! Every formula is the installed build's (ACF-NOTES.md, "Containers,
//! rounds, slope") and the tests hold them to what the game saved in the
//! reference dupes. A crate stores round counts; its `Size` is worked out
//! from them on spawn, so a crate is only ever resized in rounds.

use crate::dupe::Dupe;
use crate::transform::{self, Vec3};
use crate::value::Value;

/// `ACF.ContainerArmor` (5 mm) in inches.
pub const WALL: f64 = 5.0 * 0.0393701;
const LITRES_PER_CUBIC_INCH: f64 = 0.016387064;
const CUBIC_CM_PER_CUBIC_INCH: f64 = 16.387;
const STEEL_KG_PER_CUBIC_CM: f64 = 0.0079;
const HEX_SPACING: f64 = 0.866;
const HEX_OFFSET: f64 = 0.5;
pub const TANK_SIDE: (f64, f64) = (6.0, 96.0);
pub const CRATE_MAX_LENGTH: f64 = 192.0;
pub const CRATE_MAX_WIDTH: f64 = 96.0;
pub const DRUM_MIN_PER_RING: u32 = 6;
const CRATE_MODEL: &str = "models/holograms/hq_rcube_thin.mdl";
/// `ACF.FuelRate`.
const FUEL_RATE: f64 = 15.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TankShape {
    Box,
    Cylinder,
    Sphere,
}

impl TankShape {
    pub const ALL: [TankShape; 3] = [TankShape::Box, TankShape::Cylinder, TankShape::Sphere];

    pub fn name(self) -> &'static str {
        match self {
            TankShape::Box => "Box",
            TankShape::Cylinder => "Cylinder",
            TankShape::Sphere => "Sphere",
        }
    }

    /// By `FuelShape`, or the older `FuelTank` group where "Drum" is a cylinder.
    pub fn parse(text: &str) -> Option<TankShape> {
        match text {
            "Box" | "Scalable" => Some(TankShape::Box),
            "Cylinder" | "Drum" => Some(TankShape::Cylinder),
            "Sphere" => Some(TankShape::Sphere),
            _ => None,
        }
    }

    pub fn model(self) -> &'static str {
        match self {
            TankShape::Box => "models/acf/core/s_fuel.mdl",
            TankShape::Cylinder => "models/acf/core/s_fuel_cyl.mdl",
            TankShape::Sphere => "models/acf/core/s_sphere.mdl",
        }
    }

    /// Inside volume and outside area, cubic and square inches.
    fn volume_and_area(self, size: Vec3) -> (f64, f64) {
        let inside = |side: f64, walls: f64| (side - walls * WALL).max(0.0);
        let (a, b, c) = (size.0 / 2.0, size.1 / 2.0, size.2 / 2.0);
        match self {
            TankShape::Box => (
                inside(size.0, 2.0) * inside(size.1, 2.0) * inside(size.2, 2.0),
                2.0 * (size.0 * size.1 + size.0 * size.2 + size.1 * size.2),
            ),
            TankShape::Cylinder => {
                let squash = ((a - b) / (a + b)).powi(2);
                let perimeter = std::f64::consts::PI * (a + b) * (1.0 + 3.0 * squash / (10.0 + (4.0 - 3.0 * squash).sqrt()));
                (
                    std::f64::consts::PI * inside(a, 1.0) * inside(b, 1.0) * inside(size.2, 2.0),
                    perimeter * size.2 + 2.0 * std::f64::consts::PI * a * b,
                )
            }
            TankShape::Sphere => {
                let p = 1.6075;
                let mean = (((a * b).powf(p) + (a * c).powf(p) + (b * c).powf(p)) / 3.0).powf(1.0 / p);
                (4.0 / 3.0 * std::f64::consts::PI * inside(a, 1.0) * inside(b, 1.0) * inside(c, 1.0), 4.0 * std::f64::consts::PI * mean)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fuel {
    Petrol,
    Diesel,
    Electric,
}

impl Fuel {
    pub const ALL: [Fuel; 3] = [Fuel::Petrol, Fuel::Diesel, Fuel::Electric];

    pub fn name(self) -> &'static str {
        match self {
            Fuel::Petrol => "Petrol",
            Fuel::Diesel => "Diesel",
            Fuel::Electric => "Electric",
        }
    }

    pub fn parse(text: &str) -> Option<Fuel> {
        Fuel::ALL.into_iter().find(|fuel| fuel.name() == text)
    }

    /// kg per litre.
    pub fn density(self) -> f64 {
        match self {
            Fuel::Petrol => 0.832,
            Fuel::Diesel => 0.745,
            Fuel::Electric => 3.89,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tank {
    pub shape: TankShape,
    pub fuel: Fuel,
    /// Whole source units, each side 6 to 96.
    pub size: Vec3,
}

impl Tank {
    pub fn litres(&self) -> f64 {
        self.shape.volume_and_area(self.size).0 * LITRES_PER_CUBIC_INCH
    }

    pub fn empty_mass(&self) -> f64 {
        self.shape.volume_and_area(self.size).1 * WALL * CUBIC_CM_PER_CUBIC_INCH * STEEL_KG_PER_CUBIC_CM
    }

    /// Full, in kg. A battery's litres weigh its density too.
    pub fn full_mass(&self) -> f64 {
        self.empty_mass() + self.litres() * self.fuel.density()
    }

    /// kWh for a battery: `ACF.LiIonED` per litre.
    pub fn kilowatt_hours(&self) -> f64 {
        self.litres() * 0.458
    }

    /// Minutes at full power for an engine of `peak_kw` and `efficiency`,
    /// the figure ACF's own menu shows.
    pub fn minutes_at_full_power(&self, peak_kw: f64, efficiency: f64) -> f64 {
        let per_minute = FUEL_RATE * efficiency * peak_kw / (60.0 * self.fuel.density());
        if per_minute > 0.0 { self.litres() / per_minute } else { f64::INFINITY }
    }

    /// This tank grown or shrunk in proportion until it first holds
    /// `litres`, in the whole units ACF keeps. A side that reaches a limit
    /// stays there and the others carry on.
    pub fn holding(&self, litres: f64) -> Tank {
        let clamp = |side: f64| side.round().clamp(TANK_SIDE.0, TANK_SIDE.1);
        let scaled = |factor: f64| Tank { size: (clamp(self.size.0 * factor), clamp(self.size.1 * factor), clamp(self.size.2 * factor)), ..*self };
        let (mut low, mut high) = (0.01, 32.0);
        for _ in 0..60 {
            let middle = (low + high) / 2.0;
            if scaled(middle).litres() < litres { low = middle } else { high = middle }
        }
        scaled(high)
    }
}

fn text(dupe: &Dupe, et: usize, key: &str) -> Option<String> {
    match dupe.get(et, key) {
        Some(Value::Str(bytes)) => Some(String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    }
}

fn set_text(dupe: &mut Dupe, et: usize, key: &str, value: &str) {
    dupe.set(et, key, Value::Str(value.as_bytes().to_vec()));
}

/// The fuel tank at `index` as ACF will read it, or None if it is not one.
pub fn tank(dupe: &Dupe, index: f64) -> Option<Tank> {
    let et = transform::entity_table(dupe, index)?;
    if text(dupe, et, "Class")? != "acf_fueltank" {
        return None;
    }
    let sides = (dupe.get_number(et, "FuelSizeX"), dupe.get_number(et, "FuelSizeY"), dupe.get_number(et, "FuelSizeZ"));
    let (size, from_sides) = match (sides, dupe.get(et, "Size")) {
        ((Some(x), Some(y), Some(z)), _) => ((x, y, z), true),
        (_, Some(Value::Vector(x, y, z))) => ((*x, *y, *z), false),
        _ => ((24.0, 24.0, 24.0), true),
    };
    let named = if from_sides { text(dupe, et, "FuelShape") } else { None };
    let shape = named.or_else(|| text(dupe, et, "FuelTank")).and_then(|name| TankShape::parse(&name)).unwrap_or(TankShape::Box);
    let fuel = text(dupe, et, "FuelType").and_then(|name| Fuel::parse(&name)).unwrap_or(Fuel::Petrol);
    Some(Tank { shape, fuel, size })
}

/// Writes every key the installed build reads for a tank, sides held to
/// ACF's whole numbers and limits, and the model and saved Scale to match.
pub fn set_tank(dupe: &mut Dupe, index: f64, wanted: &Tank) -> Result<Tank, String> {
    let et = transform::entity_table(dupe, index).ok_or_else(|| format!("no entity {index}"))?;
    if text(dupe, et, "Class").as_deref() != Some("acf_fueltank") {
        return Err(format!("{index} is not a fuel tank"));
    }
    let held = |side: f64| if side.is_finite() { side.round().clamp(TANK_SIDE.0, TANK_SIDE.1) } else { 24.0 };
    let made = Tank { size: (held(wanted.size.0), held(wanted.size.1), held(wanted.size.2)), ..*wanted };
    let before = tank(dupe, index).map(|tank| tank.size);
    set_text(dupe, et, "FuelType", made.fuel.name());
    set_text(dupe, et, "FuelShape", made.shape.name());
    set_text(dupe, et, "FuelTank", if made.shape == TankShape::Cylinder { "Drum" } else { "Scalable" });
    set_text(dupe, et, "Model", made.shape.model());
    for (key, side) in [("FuelSizeX", made.size.0), ("FuelSizeY", made.size.1), ("FuelSizeZ", made.size.2)] {
        dupe.set(et, key, Value::Number(side));
    }
    dupe.set(et, "Size", Value::Vector(made.size.0, made.size.1, made.size.2));
    rescale(dupe, et, before, made.size);
    Ok(made)
}

/// Keeps a saved `Scale` in step with a new size, so the part is drawn at
/// the size it will have, not the one the game saved last time.
fn rescale(dupe: &mut Dupe, et: usize, before: Option<Vec3>, after: Vec3) {
    let (Some(before), Some(Value::Vector(x, y, z))) = (before, dupe.get(et, "Scale").cloned()) else { return };
    let grown = |scale: f64, new: f64, old: f64| if old.abs() > 1e-9 { scale * new / old } else { scale };
    dupe.set(et, "Scale", Value::Vector(grown(x, after.0, before.0), grown(y, after.1, before.1), grown(z, after.2, before.2)));
}

/// A gun class's round: the longest cartridge and the most propellant at
/// its base calibre, both scaling with calibre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponRound {
    pub id: &'static str,
    pub name: &'static str,
    pub base_caliber: f64,
    pub max_length: f64,
    pub propellant_length: f64,
}

pub const WEAPON_ROUNDS: &[WeaponRound] = &[
    WeaponRound { id: "C", name: "Cannon", base_caliber: 100.0, max_length: 80.0, propellant_length: 65.0 },
    WeaponRound { id: "SC", name: "Short cannon", base_caliber: 100.0, max_length: 80.0, propellant_length: 65.0 },
    WeaponRound { id: "AC", name: "Autocannon", base_caliber: 50.0, max_length: 40.0, propellant_length: 32.5 },
    WeaponRound { id: "LAC", name: "Light autocannon", base_caliber: 40.0, max_length: 32.0, propellant_length: 26.0 },
    WeaponRound { id: "RAC", name: "Rotary autocannon", base_caliber: 20.0, max_length: 16.0, propellant_length: 13.0 },
    WeaponRound { id: "MG", name: "Machine gun", base_caliber: 20.0, max_length: 16.0, propellant_length: 13.0 },
    WeaponRound { id: "SA", name: "Semi-automatic", base_caliber: 45.0, max_length: 36.0, propellant_length: 29.25 },
    WeaponRound { id: "HW", name: "Howitzer", base_caliber: 105.0, max_length: 90.0, propellant_length: 90.0 },
    WeaponRound { id: "MO", name: "Mortar", base_caliber: 120.0, max_length: 40.0, propellant_length: 3.0 },
    WeaponRound { id: "GL", name: "Grenade launcher", base_caliber: 40.0, max_length: 10.0, propellant_length: 1.0 },
    WeaponRound { id: "SL", name: "Smoke launcher", base_caliber: 40.0, max_length: 17.5, propellant_length: 0.05 },
];

pub fn weapon_round(id: &str) -> Option<&'static WeaponRound> {
    WEAPON_ROUNDS.iter().find(|round| round.id == id)
}

pub const AMMO_TYPES: &[&str] = &["AP", "APHE", "HE", "HEAT", "HEATFS", "APCR", "APDS", "APFSDS", "FL", "SM", "HP", "GLATGM", "FLR"];

/// Whether a gun class fires an ammo type (each type's blacklist, or for
/// the specialised ones its whitelist).
pub fn fires(weapon: &str, ammo_type: &str) -> bool {
    let none_of = |banned: &[&str]| !banned.contains(&weapon);
    let one_of = |allowed: &[&str]| allowed.contains(&weapon);
    match ammo_type {
        "AP" => none_of(&["GL", "MO", "SL"]),
        "APHE" => none_of(&["GL", "MG", "MO", "SL", "RAC"]),
        "HE" => none_of(&["MG", "RAC"]),
        "HEAT" => none_of(&["AC", "MG", "SL", "LAC", "RAC"]),
        "FL" => none_of(&["AC", "GL", "MG", "MO", "SA", "SL", "LAC", "RAC"]),
        "SM" => none_of(&["AC", "GL", "MG", "SA", "LAC", "RAC"]),
        "APCR" => one_of(&["C", "AC", "SA", "SC", "LAC", "RAC"]),
        "APDS" => one_of(&["C", "AC", "SA", "RAC"]),
        "APFSDS" => one_of(&["C", "AC", "SA", "SC"]),
        "HEATFS" => one_of(&["C", "MO", "HW", "SC"]),
        "GLATGM" => one_of(&["C", "HW", "SC"]),
        "FLR" => one_of(&["SL"]),
        "HP" => one_of(&["MG"]),
        _ => false,
    }
}

/// How long a round's two halves may be, in cm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoundLimits {
    pub max_length: f64,
    pub min_projectile: f64,
    pub max_projectile: f64,
    pub min_propellant: f64,
    pub max_propellant: f64,
}

pub fn round_limits(weapon: &WeaponRound, caliber_mm: f64, ammo_type: &str) -> RoundLimits {
    let round2 = |value: f64| (value * 100.0).round() / 100.0;
    let scale = caliber_mm / weapon.base_caliber;
    let max_length = round2(weapon.max_length * scale * if ammo_type == "FL" { 0.5 } else { 1.0 });
    let min_projectile = round2(caliber_mm / 10.0 * 1.5);
    let min_propellant = 0.01;
    RoundLimits {
        max_length,
        min_projectile,
        max_projectile: (max_length - min_propellant).max(min_projectile),
        min_propellant,
        max_propellant: (weapon.propellant_length * scale).min(max_length - min_projectile).max(min_propellant),
    }
}

impl RoundLimits {
    /// The projectile and propellant ACF would settle on for what was asked.
    pub fn held(&self, projectile: f64, propellant: f64) -> (f64, f64) {
        let projectile = projectile.clamp(self.min_projectile, self.max_projectile);
        let propellant = propellant.clamp(self.min_propellant, self.max_propellant).min((self.max_length - projectile).max(self.min_propellant));
        (projectile, propellant)
    }
}

/// A cartridge as the crate packs it: length along the crate's X, inches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoundSize {
    pub length: f64,
    pub diameter: f64,
}

pub fn round_size(caliber_mm: f64, projectile_cm: f64, propellant_cm: f64) -> RoundSize {
    RoundSize { length: (projectile_cm + propellant_cm) / 2.54, diameter: caliber_mm / 10.0 / 2.54 }
}

fn packs_hex(y: u32, z: u32) -> bool {
    y > 1 && z > 1 && ((y as f64 - 1.0) * HEX_SPACING + 1.0) * (z as f64 + HEX_OFFSET) < (y * z) as f64
}

impl RoundSize {
    /// The crate ACF builds round `counts` of these.
    pub fn crate_size(&self, counts: (u32, u32, u32)) -> Vec3 {
        let (x, y, z) = (counts.0 as f64, counts.1 as f64, counts.2 as f64);
        if packs_hex(counts.1, counts.2) {
            (x * self.length, (y - 1.0) * self.diameter * HEX_SPACING + self.diameter, z * self.diameter + self.diameter * HEX_OFFSET)
        } else {
            (x * self.length, y * self.diameter, z * self.diameter)
        }
    }

    /// The most rounds each way, given the other two counts (`ACF.GetMaxCounts`).
    pub fn max_counts(&self, y: u32, z: u32) -> (u32, u32, u32) {
        let whole = |value: f64| value.floor().max(0.0) as u32;
        let square = whole(CRATE_MAX_WIDTH / self.diameter);
        let hex_y = whole((CRATE_MAX_WIDTH - self.diameter) / (self.diameter * HEX_SPACING) + 1.0);
        let hex_z = whole((CRATE_MAX_WIDTH - self.diameter * HEX_OFFSET) / self.diameter);
        let max_y = if packs_hex(hex_y, z) { hex_y } else { square };
        let max_z = if packs_hex(y, hex_z) { hex_z } else { square };
        (whole(CRATE_MAX_LENGTH / self.length).max(1), max_y.max(1), max_z.max(1))
    }

    /// A drum: `per_ring` rounds stood round a core, `layers` rings high
    /// (`ACF.GetDrumDimensions`). The rounds point outwards, so the drum is
    /// as wide as the ring plus a round's length each side.
    pub fn drum_size(&self, per_ring: u32, layers: u32) -> Vec3 {
        let across = (per_ring as f64 * self.diameter / (2.0 * std::f64::consts::PI) + self.length) * 2.0;
        let high = self.diameter + (layers.max(1) as f64 - 1.0) * self.diameter * HEX_SPACING;
        (across, across, high)
    }

    /// The most rounds in a ring and the most layers a drum may have.
    pub fn max_drum(&self) -> (u32, u32) {
        let room = CRATE_MAX_WIDTH - 2.0 * self.length;
        let per_ring = if room > 0.0 { ((room * std::f64::consts::PI / self.diameter).floor() as u32).max(DRUM_MIN_PER_RING) } else { DRUM_MIN_PER_RING };
        let layers = if CRATE_MAX_LENGTH < self.diameter { 1 } else { (((CRATE_MAX_LENGTH - self.diameter) / (self.diameter * HEX_SPACING) + 1.0).floor() as u32).max(1) };
        (per_ring, layers)
    }

    /// The drum that holds at least `rounds` in the least space.
    pub fn drum_for(&self, rounds: u32) -> (u32, u32) {
        let (most_per_ring, most_layers) = self.max_drum();
        let mut best: Option<((u32, u32), (u32, f64))> = None;
        for per_ring in DRUM_MIN_PER_RING..=most_per_ring {
            let layers = rounds.max(1).div_ceil(per_ring).min(most_layers);
            let size = self.drum_size(per_ring, layers);
            let score = ((per_ring * layers).min(rounds.max(1)), size.0 * size.1 * size.2);
            if best.as_ref().is_none_or(|(_, (held, volume))| score.0 > *held || (score.0 == *held && score.1 < *volume)) {
                best = Some(((per_ring, layers), score));
            }
        }
        best.map_or((DRUM_MIN_PER_RING, 1), |(drum, _)| drum)
    }

    fn allows(&self, counts: (u32, u32, u32)) -> bool {
        let most = self.max_counts(counts.1, counts.2);
        counts.0 >= 1 && counts.1 >= 1 && counts.2 >= 1 && counts.0 <= most.0 && counts.1 <= most.1 && counts.2 <= most.2
    }

    /// The counts that hold at least `rounds` in the least space, as near a
    /// cube as that allows; the fullest crate ACF permits when even that
    /// holds fewer.
    pub fn counts_for(&self, rounds: u32) -> (u32, u32, u32) {
        let rounds = rounds.max(1);
        let limit = self.max_counts(1, 1);
        let side = (CRATE_MAX_WIDTH / self.diameter).floor().max(1.0) as u32 + 8;
        let mut best: Option<((u32, u32, u32), (u32, f64, f64))> = None;
        let mut fullest = ((1, 1, 1), 1u32);
        for x in 1..=limit.0 {
            for y in 1..=side {
                for z in 1..=side {
                    let counts = (x, y, z);
                    if !self.allows(counts) {
                        continue;
                    }
                    let held = x * y * z;
                    if held > fullest.1 {
                        fullest = (counts, held);
                    }
                    if held < rounds {
                        continue;
                    }
                    let size = self.crate_size(counts);
                    let longest = size.0.max(size.1).max(size.2);
                    let score = (held, size.0 * size.1 * size.2, longest);
                    if best.as_ref().is_none_or(|(_, least)| score.0 < least.0 || (score.0 == least.0 && (score.1, score.2) < (least.1, least.2))) {
                        best = Some((counts, score));
                    }
                    break;
                }
            }
        }
        best.map_or(fullest.0, |(counts, _)| counts)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AmmoCrate {
    pub weapon: String,
    pub caliber: f64,
    pub ammo_type: String,
    pub projectile: f64,
    pub propellant: f64,
    pub tracer: bool,
    /// Long, wide, high for a box; rounds per ring, 1, layers for a drum.
    pub counts: (u32, u32, u32),
    /// `AmmoShape` "Cylinder": a drum.
    pub drum: bool,
}

impl AmmoCrate {
    pub fn round(&self) -> RoundSize {
        round_size(self.caliber, self.projectile, self.propellant)
    }

    pub fn rounds(&self) -> u32 {
        self.counts.0 * self.counts.1 * self.counts.2
    }

    pub fn size(&self) -> Vec3 {
        if self.drum { self.round().drum_size(self.counts.0, self.counts.2) } else { self.round().crate_size(self.counts) }
    }

    /// This crate sized to hold at least `rounds`.
    pub fn holding(&self, rounds: u32) -> AmmoCrate {
        let counts = if self.drum {
            let (per_ring, layers) = self.round().drum_for(rounds);
            (per_ring, 1, layers)
        } else {
            self.round().counts_for(rounds)
        };
        AmmoCrate { counts, ..self.clone() }
    }

    pub fn empty_mass(&self) -> f64 {
        let size = self.size();
        let inside = |side: f64| (side - 2.0 * WALL).max(0.0);
        (size.0 * size.1 * size.2 - inside(size.0) * inside(size.1) * inside(size.2)) * 0.13
    }

    /// The same crate with its round and counts brought inside what the gun
    /// class and ACF's crate limits allow.
    pub fn held(&self) -> AmmoCrate {
        let mut made = self.clone();
        if let Some(weapon) = weapon_round(&self.weapon) {
            (made.projectile, made.propellant) = round_limits(weapon, self.caliber, &self.ammo_type).held(self.projectile, self.propellant);
        }
        made.counts = if made.drum {
            let (per_ring, layers) = made.round().max_drum();
            (made.counts.0.clamp(DRUM_MIN_PER_RING, per_ring), 1, made.counts.2.clamp(1, layers))
        } else {
            let most = made.round().max_counts(made.counts.1.max(1), made.counts.2.max(1));
            (made.counts.0.clamp(1, most.0), made.counts.1.clamp(1, most.1), made.counts.2.clamp(1, most.2))
        };
        made
    }
}

/// The ammo crate at `index`, or None if it is not one a gun feeds from: a
/// crate of missiles or bombs packs by its munition's model, which is not
/// read here, so it is left alone rather than guessed at. A crate that
/// names no weapon yet reads as a 100 mm cannon's, ACF's own default.
pub fn ammo_crate(dupe: &Dupe, index: f64) -> Option<AmmoCrate> {
    let et = transform::entity_table(dupe, index)?;
    if text(dupe, et, "Class")? != "acf_ammo" {
        return None;
    }
    let weapon = match text(dupe, et, "Weapon") {
        Some(id) if weapon_round(&id).is_some() => id,
        Some(_) => return None,
        None => "C".to_owned(),
    };
    let base = weapon_round(&weapon).map_or(100.0, |round| round.base_caliber);
    let caliber = dupe.get_number(et, "Caliber").filter(|caliber| *caliber > 0.0).unwrap_or(base);
    let ammo_type = text(dupe, et, "AmmoType").unwrap_or_else(|| "AP".to_owned());
    let limits = weapon_round(&weapon).map(|round| round_limits(round, caliber, &ammo_type));
    let projectile = dupe.get_number(et, "Projectile").or(limits.map(|limits| limits.min_projectile * 2.0)).unwrap_or(15.0);
    let propellant = dupe.get_number(et, "Propellant").or(limits.map(|limits| limits.max_propellant)).unwrap_or(30.0);
    let (projectile, propellant) = limits.map_or((projectile, propellant), |limits| limits.held(projectile, propellant));
    let round = round_size(caliber, projectile, propellant);
    let count = |key: &str| dupe.get_number(et, key).filter(|count| *count >= 1.0).map(|count| count as u32);
    let counts = match (count("CrateProjectilesX"), count("CrateProjectilesY"), count("CrateProjectilesZ"), dupe.get(et, "Size")) {
        (Some(x), Some(y), Some(z), _) => (x, y, z),
        (_, _, _, Some(Value::Vector(x, y, z))) => {
            let fits = |side: f64, each: f64| ((side / each).floor() as u32).max(1);
            (fits(*x, round.length), fits(*y, round.diameter), fits(*z, round.diameter))
        }
        _ => (3, 3, 3),
    };
    let tracer = matches!(dupe.get(et, "Tracer"), Some(Value::Bool(true)));
    let drum = text(dupe, et, "AmmoShape").as_deref() == Some("Cylinder");
    Some(AmmoCrate { weapon, caliber, ammo_type, projectile, propellant, tracer, counts, drum })
}

/// Writes the crate's round and counts, and the `Size` ACF will work out
/// from them, so the editor draws what the game will build.
pub fn set_ammo_crate(dupe: &mut Dupe, index: f64, wanted: &AmmoCrate) -> Result<AmmoCrate, String> {
    let et = transform::entity_table(dupe, index).ok_or_else(|| format!("no entity {index}"))?;
    if text(dupe, et, "Class").as_deref() != Some("acf_ammo") {
        return Err(format!("{index} is not an ammo crate"));
    }
    if !fires(&wanted.weapon, &wanted.ammo_type) {
        return Err(format!("a {} does not fire {}", wanted.weapon, wanted.ammo_type));
    }
    let made = wanted.held();
    let before = match dupe.get(et, "Size") {
        Some(Value::Vector(x, y, z)) => Some((*x, *y, *z)),
        _ => None,
    };
    set_text(dupe, et, "Weapon", &made.weapon);
    set_text(dupe, et, "AmmoType", &made.ammo_type);
    dupe.set(et, "Caliber", Value::Number(made.caliber));
    dupe.set(et, "Projectile", Value::Number(made.projectile));
    dupe.set(et, "Propellant", Value::Number(made.propellant));
    dupe.set(et, "Tracer", Value::Bool(made.tracer));
    set_text(dupe, et, "AmmoShape", if made.drum { "Cylinder" } else { "Box" });
    set_text(dupe, et, "Model", if made.drum { TankShape::Cylinder.model() } else { CRATE_MODEL });
    for (key, count) in [("CrateProjectilesX", made.counts.0), ("CrateProjectilesY", made.counts.1), ("CrateProjectilesZ", made.counts.2)] {
        dupe.set(et, key, Value::Number(count as f64));
    }
    let size = made.size();
    dupe.set(et, "Size", Value::Vector(size.0, size.1, size.2));
    rescale(dupe, et, before, size);
    Ok(made)
}

/// Slope: how much thicker a plate is to a round arriving `degrees` from
/// its normal, and the chance an AP-like round that fails to penetrate
/// glances off (`ricochet` and `limit_velocity` are the ammo type's).
pub fn effective_thickness(thickness_mm: f64, degrees: f64) -> f64 {
    thickness_mm / degrees.to_radians().cos().abs().max(1e-6)
}

pub fn ricochet_chance(degrees: f64, ricochet: f64, limit_velocity: f64, speed_mps: f64) -> f64 {
    let centre = ricochet - (speed_mps - limit_velocity).abs() / 100.0;
    1.0 / (1.0 + (-(degrees - centre) / 4.0).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64, within: f64) -> bool {
        (a - b).abs() <= within
    }

    #[test]
    fn ironlocks_diesel_tank_holds_and_weighs_what_the_game_saved() {
        let tank = Tank { shape: TankShape::Box, fuel: Fuel::Diesel, size: (11.0, 18.0, 27.0) };
        assert!(near(tank.litres(), 81.41746127832866, 1e-3), "{}", tank.litres());
        assert!(near(tank.empty_mass(), 49.9990865664213, 1e-2), "{}", tank.empty_mass());
        assert!(near(tank.full_mass(), 110.65509521877615, 2e-2), "{}", tank.full_mass());
    }

    #[test]
    fn a_tank_sized_for_litres_holds_them_and_keeps_its_proportions() {
        let tank = Tank { shape: TankShape::Box, fuel: Fuel::Petrol, size: (24.0, 24.0, 24.0) };
        for wanted in [20.0, 100.0, 215.0, 900.0, 5000.0] {
            let made = tank.holding(wanted);
            assert!(made.litres() >= wanted, "{wanted}: {made:?} holds {}", made.litres());
            assert!(made.size.0 == made.size.1 && made.size.1 == made.size.2, "{made:?}");
            let smaller = Tank { size: (made.size.0 - 1.0, made.size.1 - 1.0, made.size.2 - 1.0), ..made };
            assert!(made.size.0 == TANK_SIDE.0 || smaller.litres() < wanted, "{wanted}: one unit smaller would also do");
        }
        let flat = Tank { shape: TankShape::Box, fuel: Fuel::Diesel, size: (49.0, 71.0, 6.0) }.holding(600.0);
        assert!(flat.litres() >= 600.0 && flat.size.2 < flat.size.0 && flat.size.0 < flat.size.1, "{flat:?}");
        let full = tank.holding(1e9);
        assert_eq!(full.size, (96.0, 96.0, 96.0), "as big as ACF allows when nothing holds that much");
    }

    #[test]
    fn a_cylinder_and_a_sphere_hold_less_than_the_box_round_them() {
        let size = (30.0, 30.0, 40.0);
        let litres = |shape| Tank { shape, fuel: Fuel::Petrol, size }.litres();
        assert!(litres(TankShape::Box) > litres(TankShape::Cylinder) && litres(TankShape::Cylinder) > litres(TankShape::Sphere));
        assert!(near(litres(TankShape::Cylinder) / litres(TankShape::Box), std::f64::consts::FRAC_PI_4, 0.01));
    }

    #[test]
    fn the_reference_crates_come_out_at_the_size_the_game_saved() {
        let lac = AmmoCrate { weapon: "LAC".into(), caliber: 40.0, ammo_type: "APHE".into(), projectile: 6.0, propellant: 19.46, tracer: false, counts: (2, 32, 5), drum: false };
        let size = lac.size();
        assert!(near(size.0, 20.047243118286133, 1e-3) && near(size.1, 43.851993560791016, 1e-3) && near(size.2, 8.661422729492188, 1e-3), "{size:?}");
        assert_eq!(lac.rounds(), 320);
        assert_eq!(lac.held(), lac, "a crate the game saved is already inside the limits");

        let rac = AmmoCrate { weapon: "RAC".into(), caliber: 20.0, ammo_type: "AP".into(), projectile: 4.0, propellant: 12.0, tracer: true, counts: (2, 50, 25), drum: false };
        let size = rac.size();
        assert!(near(size.0, 12.598424911499023, 1e-3) && near(size.1, 34.20001983642578, 1e-3) && near(size.2, 20.078752517700195, 1e-3), "{size:?}");

        let sc = AmmoCrate { weapon: "SC".into(), caliber: 127.0, ammo_type: "APFSDS".into(), projectile: 19.05, propellant: 82.55, tracer: true, counts: (1, 2, 6), drum: false };
        let size = sc.size();
        assert!(near(size.0, 40.0, 1e-3) && near(size.1, 10.0, 1e-3) && near(size.2, 30.0, 1e-3), "{size:?}");
        assert_eq!(sc.held(), sc);
    }

    #[test]
    fn the_phalanx_drum_comes_out_at_the_size_the_game_saved_and_a_drum_is_sized_by_rounds() {
        let drum = AmmoCrate { weapon: "RAC".into(), caliber: 20.0, ammo_type: "APDS".into(), projectile: 5.5, propellant: 10.5, tracer: false, counts: (45, 1, 100), drum: true };
        let size = drum.size();
        assert!(near(size.0, 23.87712860107422, 1e-3) && near(size.1, size.0, 1e-9) && near(size.2, 68.29452514648438, 1e-3), "{size:?}");
        assert_eq!(drum.rounds(), 4500);
        assert!(near(drum.empty_mass(), 193.76858845042182, 0.5), "{}", drum.empty_mass());
        assert_eq!(drum.held(), drum);

        for wanted in [10, 500, 4500] {
            let made = drum.holding(wanted);
            assert!(made.drum && made.counts.1 == 1 && made.rounds() >= wanted && made.rounds() < wanted + made.counts.0, "{wanted}: {:?}", made.counts);
            assert_eq!(made.held(), made);
        }
        let too_few = AmmoCrate { counts: (2, 9, 0), ..drum.clone() }.held();
        assert_eq!(too_few.counts, (DRUM_MIN_PER_RING, 1, 1));
    }

    #[test]
    fn a_crate_of_missiles_is_left_alone() {
        let mut dupe = crate::buildfile::empty_dupe();
        let index = crate::build::spawn_catalog(&mut dupe, crate::catalog::find("AP").unwrap(), (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), &[]).unwrap();
        assert!(ammo_crate(&dupe, index).is_some(), "no weapon named yet: a cannon's");
        let et = transform::entity_table(&dupe, index).unwrap();
        set_text(&mut dupe, et, "Weapon", "Strela-1 SAM");
        assert_eq!(ammo_crate(&dupe, index), None);
    }

    #[test]
    fn counts_for_a_number_of_rounds_hold_them_in_a_crate_acf_allows() {
        let round = round_size(40.0, 6.0, 19.46);
        for wanted in [1, 7, 40, 100, 320, 1000] {
            let counts = round.counts_for(wanted);
            let held = counts.0 * counts.1 * counts.2;
            assert!(held >= wanted, "{wanted}: {counts:?}");
            assert!(held < wanted + wanted / 4 + 4, "{wanted}: {counts:?} holds {held}, far more than asked");
            let size = round.crate_size(counts);
            assert!(size.0 <= CRATE_MAX_LENGTH && size.1 <= CRATE_MAX_WIDTH && size.2 <= CRATE_MAX_WIDTH, "{wanted}: {size:?}");
        }
        let big = round_size(140.0, 40.0, 72.0);
        let counts = big.counts_for(100_000);
        assert_eq!(counts, big.max_counts(counts.1, counts.2), "the fullest crate when nothing holds that many");
    }

    #[test]
    fn a_round_is_held_inside_its_guns_limits() {
        let cannon = weapon_round("C").unwrap();
        let limits = round_limits(cannon, 125.0, "AP");
        assert!(near(limits.max_length, 100.0, 1e-9) && near(limits.min_projectile, 18.75, 1e-9) && near(limits.max_propellant, 81.25, 1e-9), "{limits:?}");
        assert_eq!(limits.held(5.0, 500.0), (18.75, 81.25));
        assert_eq!(limits.held(60.0, 81.25), (60.0, 40.0));
        assert!(near(round_limits(cannon, 125.0, "FL").max_length, 50.0, 1e-9));
    }

    #[test]
    fn guns_fire_what_acf_lets_them() {
        assert!(fires("C", "APFSDS") && fires("SC", "APFSDS") && !fires("RAC", "APFSDS"));
        assert!(fires("LAC", "APHE") && !fires("RAC", "APHE") && !fires("MG", "HE"));
        assert!(fires("MG", "HP") && !fires("C", "HP"));
        assert!(fires("SL", "FLR") && fires("SL", "SM") && !fires("SL", "AP"));
        for weapon in WEAPON_ROUNDS {
            assert!(AMMO_TYPES.iter().any(|ammo_type| fires(weapon.id, ammo_type)), "{} fires nothing", weapon.id);
        }
    }

    #[test]
    fn a_crate_and_a_tank_round_trip_through_a_dupe() {
        let mut dupe = crate::buildfile::empty_dupe();
        let ap = crate::catalog::find("AP").unwrap();
        let diesel = crate::catalog::find("Diesel").unwrap();
        let crate_index = crate::build::spawn_catalog(&mut dupe, ap, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), &[]).unwrap();
        let tank_index = crate::build::spawn_catalog(&mut dupe, diesel, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), &[]).unwrap();

        let mut wanted = ammo_crate(&dupe, crate_index).unwrap();
        wanted.weapon = "LAC".into();
        wanted.caliber = 40.0;
        wanted.ammo_type = "APHE".into();
        (wanted.projectile, wanted.propellant) = (6.0, 19.46);
        wanted.counts = wanted.round().counts_for(300);
        let made = set_ammo_crate(&mut dupe, crate_index, &wanted).unwrap();
        assert_eq!(ammo_crate(&dupe, crate_index).unwrap(), made);
        assert!(made.rounds() >= 300);
        assert_eq!(crate::scale::dimensions(&dupe, crate_index), Some(made.size()));
        wanted.ammo_type = "HP".into();
        assert!(set_ammo_crate(&mut dupe, crate_index, &wanted).is_err());

        let tank_now = tank(&dupe, tank_index).unwrap();
        assert_eq!(tank_now.fuel, Fuel::Diesel);
        let made = set_tank(&mut dupe, tank_index, &Tank { shape: TankShape::Cylinder, ..tank_now }.holding(150.0)).unwrap();
        assert_eq!(tank(&dupe, tank_index).unwrap(), made);
        assert!(made.litres() >= 150.0);
        assert!(set_tank(&mut dupe, crate_index, &made).is_err());
        crate::extras::repair_head(&mut dupe);
        assert_eq!(crate::extras::validate(&dupe), Vec::<String>::new());
    }

    #[test]
    fn slope_is_what_makes_armour_work() {
        assert!(near(effective_thickness(50.0, 0.0), 50.0, 1e-9));
        assert!(near(effective_thickness(50.0, 60.0), 100.0, 1e-6));
        assert!(near(ricochet_chance(60.0, 60.0, 800.0, 800.0), 0.5, 1e-9));
        assert!(ricochet_chance(0.0, 60.0, 800.0, 800.0) < 1e-6);
        assert!(near(ricochet_chance(70.0, 60.0, 800.0, 800.0), 0.924, 0.005));
    }
}
