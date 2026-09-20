//! ACF-3's rules, read from the repo rather than remembered. Each constant
//! names where it came from. The doctor enforces these; `--doctor --fix`
//! applies the ones with a mechanical correction.

// core/globals.lua
pub const MIN_ARMOUR_MM: f64 = 0.01;
pub const MAX_ARMOUR_MM: f64 = 5000.0;
pub const MIN_DUCTILITY: f64 = -80.0;
pub const MAX_DUCTILITY: f64 = 80.0;
pub const MIN_MASS_KG: f64 = 0.1;
pub const MAX_MASS_KG: f64 = 50000.0;
pub const MIN_GEAR_RATIO: f64 = -10.0;
pub const MAX_GEAR_RATIO: f64 = 10.0;
pub const MIN_CVT_RATIO: f64 = 1.0;
pub const MAX_CVT_RATIO: f64 = 100.0;
pub const MIN_AUTOLOADER_CALIBER: f64 = 60.0;
pub const MAX_AUTOLOADER_CALIBER: f64 = 280.0;
/// ACF.LinkDistance and ACF.MobilityLinkDistance, both 650 inches.
pub const LINK_DISTANCE: f64 = 650.0;
/// ACF.MaxDriveshaftAngle default; a server setting.
pub const MAX_DRIVESHAFT_ANGLE_DEG: f64 = 85.0;

/// Official turret caps, from the T-34-85's ring and trunnion.
pub const RING_MAX_SPEED: f64 = 30.0;
pub const TRUNNION_MAX_SPEED: f64 = 10.0;

/// Server mass limit most servers run.
pub const SERVER_MASS_LIMIT_KG: f64 = 60000.0;

/// crew_types.lua: what each occupation's LinkHandlers accept. A Gunner
/// links the TURRET, not the gun; the Loader and Commander link guns.
pub fn crew_targets(crew_type: &str) -> &'static [&'static str] {
    match crew_type.to_ascii_lowercase().as_str() {
        "loader" => &["acf_gun", "acf_rack"],
        "gunner" => &["acf_baseplate", "acf_turret"],
        "driver" => &["acf_baseplate"],
        "commander" | "pilot" => &["acf_baseplate", "acf_gun", "acf_rack", "acf_turret"],
        _ => &[],
    }
}

/// acf_engine/init.lua CFW_PreParentedTo: an engine's direct parent must be
/// an acf_baseplate, or the engine reports "Parenting Issue" and won't run
/// (unless the server allows arbitrary parents).
pub const ENGINE_PARENT_CLASS: &str = "acf_baseplate";

/// acf_turret/init.lua: one motor and one gyro per turret.
pub const MAX_MOTORS_PER_TURRET: usize = 1;
pub const MAX_GYROS_PER_TURRET: usize = 1;

/// acf_controller modules: Single-field links allow exactly one target.
pub const CONTROLLER_SINGLE_FIELDS: &[&str] = &["Gearbox", "Baseplate", "Seat", "TurretComputer", "GuidanceComputer"];

/// acf_gun/init.lua: a crate must match the gun's weapon and caliber
/// ("Wrong ammo type for this weapon"). Both are legacy top-level keys.
pub fn ammo_matches(gun_weapon: &str, gun_caliber: f64, crate_weapon: &str, crate_caliber: f64) -> bool {
    gun_weapon.eq_ignore_ascii_case(crate_weapon) && (gun_caliber - crate_caliber).abs() < 0.01
}

/// acf_gun/init.lua BeltFedCheck: belted weapons (not machineguns) need the
/// crate mounted on the same propagator — same parent root — as the gun.
pub fn is_belted_weapon(weapon: &str) -> bool {
    matches!(weapon.to_ascii_uppercase().as_str(), "RAC" | "AC" | "SA")
}

/// The runtime bookkeeping keys that never need to be in a saved dupe.
pub const JUNK_ENTITY_KEYS: &[&str] = &["_links", "_contraption", "GMN_GetClassVar", "ImprovedWeight", "_mass", "__ancf_grabbed_with"];

/// entities/engines/*.lua: every engine item and the fuel types it burns.
/// "Cannot link because fuel type is incompatible." otherwise.
pub const ENGINE_FUEL: &[(&str, &[&str])] = &[
    ("0.25-I1", &["Petrol"]),
    ("0.5-I1", &["Petrol"]),
    ("0.6-V2", &["Petrol"]),
    ("0.8L-I2", &["Diesel"]),
    ("0.9L-I2", &["Petrol"]),
    ("1.0L-I4", &["Petrol"]),
    ("1.1-I3", &["Diesel"]),
    ("1.2-I3", &["Petrol"]),
    ("1.2-V2", &["Petrol"]),
    ("1.3-I1", &["Petrol"]),
    ("1.3L-R", &["Petrol"]),
    ("1.4-B4", &["Petrol"]),
    ("1.5-I4", &["Petrol"]),
    ("1.6-I4", &["Diesel"]),
    ("1.8L-V4", &["Petrol"]),
    ("1.9L-I4", &["Petrol"]),
    ("1.9L-V4", &["Diesel"]),
    ("10.0-I2", &["Diesel"]),
    ("11.0-I3", &["Diesel"]),
    ("11.0-R7", &["Petrol"]),
    ("12.0-V6", &["Petrol"]),
    ("13.0-V12", &["Petrol"]),
    ("13.5-I3", &["Petrol"]),
    ("14.5-B6", &["Diesel"]),
    ("15.0-I4", &["Diesel"]),
    ("15.0-V6", &["Diesel"]),
    ("15.8-B6", &["Petrol"]),
    ("16.0-I4", &["Petrol"]),
    ("17.2-I6", &["Petrol"]),
    ("18.0-V8", &["Petrol"]),
    ("19.0-V8", &["Diesel"]),
    ("2.0L-R", &["Petrol"]),
    ("2.1-B4", &["Petrol"]),
    ("2.2-I6", &["Petrol"]),
    ("2.3-I5", &["Petrol"]),
    ("2.4-B4", &["Diesel", "Petrol"]),
    ("2.4-V2", &["Petrol"]),
    ("2.4L-V6", &["Petrol"]),
    ("2.6L-Wankel", &["Petrol"]),
    ("2.8-B6", &["Petrol"]),
    ("2.8-I3", &["Diesel"]),
    ("2.9-I5", &["Diesel"]),
    ("2.9-V8", &["Petrol"]),
    ("20.0-I6", &["Diesel"]),
    ("21.0-V12", &["Diesel"]),
    ("22.0-V10", &["Diesel", "Petrol"]),
    ("23.0-V12", &["Petrol"]),
    ("24.0-R7", &["Petrol"]),
    ("3.0-I6", &["Diesel"]),
    ("3.0-V12", &["Petrol"]),
    ("3.1-I4", &["Diesel"]),
    ("3.2-B4", &["Petrol"]),
    ("3.3L-V4", &["Diesel"]),
    ("3.4-I3", &["Petrol"]),
    ("3.6-V6", &["Petrol"]),
    ("3.7-I4", &["Petrol"]),
    ("3.8-I6", &["Petrol"]),
    ("3.8-R7", &["Petrol"]),
    ("3.9-I5", &["Petrol"]),
    ("4.0-V12", &["Diesel"]),
    ("4.1-I5", &["Diesel"]),
    ("4.3-V10", &["Petrol"]),
    ("4.5-V8", &["Diesel"]),
    ("4.6-V12", &["Petrol"]),
    ("4.8-I6", &["Petrol"]),
    ("5.0-B6", &["Petrol"]),
    ("5.2-V6", &["Diesel"]),
    ("5.3-V10", &["Petrol"]),
    ("5.7-V8", &["Petrol"]),
    ("6.2-V6", &["Petrol"]),
    ("6.5-I6", &["Diesel"]),
    ("7.0-V12", &["Petrol"]),
    ("7.2-V8", &["Petrol"]),
    ("7.8-V8", &["Diesel"]),
    ("8.0-R7", &["Diesel", "Petrol"]),
    ("8.0-V10", &["Petrol"]),
    ("8.3-B6", &["Diesel", "Petrol"]),
    ("9.0-V8", &["Petrol"]),
    ("9.2-V12", &["Diesel"]),
    ("900cc-R", &["Petrol"]),
    ("Electric-Large", &["Electric"]),
    ("Electric-Large-NoBatt", &["Electric"]),
    ("Electric-Medium", &["Electric"]),
    ("Electric-Medium-NoBatt", &["Electric"]),
    ("Electric-Small", &["Electric"]),
    ("Electric-Small-NoBatt", &["Electric"]),
    ("Electric-Tiny-NoBatt", &["Electric"]),
    ("Turbine-Ground-Large", &["Diesel", "Petrol"]),
    ("Turbine-Ground-Medium", &["Diesel", "Petrol"]),
    ("Turbine-Ground-Small", &["Diesel", "Petrol"]),
    ("Turbine-Large", &["Diesel", "Petrol"]),
    ("Turbine-Large-Ground-Trans", &["Diesel", "Petrol"]),
    ("Turbine-Large-Trans", &["Diesel", "Petrol"]),
    ("Turbine-Medium", &["Diesel", "Petrol"]),
    ("Turbine-Medium-Ground-Trans", &["Diesel", "Petrol"]),
    ("Turbine-Medium-Trans", &["Diesel", "Petrol"]),
    ("Turbine-Small", &["Diesel", "Petrol"]),
    ("Turbine-Small-Ground-Trans", &["Diesel", "Petrol"]),
    ("Turbine-Small-Trans", &["Diesel", "Petrol"]),
];

/// Engines whose driveshaft leaves sideways (IsTrans): output direction is
/// local Y instead of X.
pub const TRANSVERSE_ENGINES: &[&str] = &["Turbine-Small-Trans", "Turbine-Medium-Trans", "Turbine-Large-Trans", "Turbine-Small-Ground-Trans", "Turbine-Medium-Ground-Trans", "Turbine-Large-Ground-Trans"];

pub fn engine_fuels(engine_id: &str) -> Option<&'static [&'static str]> {
    ENGINE_FUEL.iter().find(|(k, _)| k.eq_ignore_ascii_case(engine_id)).map(|(_, v)| *v)
}

/// entities/guns/*.lua CaliberLimits per weapon ID, in mm.
pub const GUN_CALIBRE: &[(&str, f64, f64)] = &[
    ("AC", 20.0, 60.0), ("C", 20.0, 170.0), ("FGL", 40.0, 40.0), ("GL", 25.0, 40.0), ("HW", 75.0, 203.0),
    ("LAC", 20.0, 40.0), ("MG", 5.56, 20.0), ("MO", 37.0, 280.0), ("RAC", 7.62, 37.0), ("SA", 20.0, 76.0),
    ("SC", 20.0, 170.0), ("SL", 40.0, 81.0),
];
pub fn gun_calibre_range(weapon: &str) -> Option<(f64, f64)> {
    GUN_CALIBRE.iter().find(|(w, _, _)| w.eq_ignore_ascii_case(weapon)).map(|(_, lo, hi)| (*lo, *hi))
}

/// The material ACF puts on a class when it spawns, which a dupe does not
/// carry: acf_ammo/modules/spawning.lua and acf_supply (Future_vents),
/// acf_fueltank/modules/spawning.lua (TANK_MATERIAL), and
/// acf_baseplate/modules/spawning.lua. Workshop build 3248769144.
pub fn spawn_material(class: &str) -> Option<&'static str> {
    match class {
        "acf_ammo" | "acf_supply" => Some("phoenix_storms/Future_vents"),
        "acf_fueltank" => Some("models/props_canal/metalcrate001d"),
        "acf_baseplate" => Some("hunter/myplastic"),
        // The Primitive addon's own default (primitive/entities/base.lua).
        "primitive_shape" | "primitive_airfoil" => Some(crate::primitive::DEFAULT_MATERIAL),
        _ => None,
    }
}

/// The smallest calibre, in millimetres, whose gun gets a Fuze wire input
/// (`ACF.MinFuzeCaliber`, core/globals.lua, workshop build 3248769144).
pub const MIN_FUZE_CALIBER: f64 = 25.0;

/// acf_baseplate/shared.lua ACF_UserVars.
pub const BASEPLATE_WIDTH: (f64, f64) = (36.0, 240.0);
pub const BASEPLATE_LENGTH: (f64, f64) = (36.0, 480.0);
pub const BASEPLATE_THICKNESS: (f64, f64) = (0.5, 3.0);
/// globals.lua, workshop build 3248769144: containers (fuel tanks) 6..96
/// per axis; ammo crates ≥ 6.
pub const CONTAINER_SIZE: (f64, f64) = (6.0, 96.0);
pub const AMMO_MIN_SIZE: f64 = 6.0;
/// globals.lua, workshop build 3248769144: an ammo crate is at most 192
/// long (x) and 96 wide and high.
pub const AMMO_MAX_LENGTH: f64 = 192.0;
pub const AMMO_MAX_WIDTH: f64 = 96.0;
/// turrets.lua, workshop build 3248769144: ring size ranges, H and V.
pub const RING_SIZE_H: (f64, f64) = (2.0, 512.0);
pub const RING_SIZE_V: (f64, f64) = (4.0, 48.0);
/// entities/guns/*.lua, workshop build 3248769144: the calibre each class's
/// model was made at and its ScaleFactor; a gun pastes at Caliber / Base *
/// factor (acf_gun/init.lua).
pub const GUN_SCALE: &[(&str, f64, f64)] = &[
    ("AC", 50.0, 0.86), ("C", 100.0, 0.84), ("FGL", 40.0, 1.0), ("GL", 40.0, 0.96), ("HW", 105.0, 0.84),
    ("LAC", 40.0, 0.81), ("MG", 20.0, 1.0), ("MO", 120.0, 0.84), ("RAC", 20.0, 1.0), ("SA", 45.0, 1.0),
    ("SC", 100.0, 1.0), ("SL", 40.0, 0.96),
];
pub fn gun_scale(weapon: &str, caliber_mm: f64) -> Option<f64> {
    GUN_SCALE.iter().find(|(w, _, _)| w.eq_ignore_ascii_case(weapon)).map(|(_, base, factor)| caliber_mm / base * factor)
}
/// turrets.lua: a ring is RingSize wide and max(RingSize * 0.1, 4) tall,
/// 12 when it is small enough to use the small model; a trunnion scales
/// uniformly against a base of 20 (acf_turret/init.lua).
pub const RING_HEIGHT_RATIO: f64 = 0.1;
pub const RING_MIN_HEIGHT: f64 = 4.0;
pub const RING_SMALL_MODEL_MAX: f64 = 12.5;
pub const TRUNNION_SCALE_BASE: f64 = 20.0;
pub fn ring_height(size: f64) -> f64 {
    if size <= RING_SMALL_MODEL_MAX { 12.0 } else { (size * RING_HEIGHT_RATIO).max(RING_MIN_HEIGHT) }
}
/// The server's default MaxThickness setting; thicker armour is clamped in game.
pub const SERVER_MAX_THICKNESS_DEFAULT: f64 = 300.0;

/// validation_sv.lua IsLegal: an ACF entity with a custom physics object
/// (Make Spherical) or any visclip is disabled — "Invalid Physics",
/// "Visual Clip".
pub const ILLEGAL_ON_ACF: &[&str] = &["MakeSphericalCollisions", "proper_clipping", "clips"];

/// Gearbox shape from its ID suffix: T (transaxial), L (linear), ST
/// (straight-through). Sets the input direction and which output faces
/// where (acf_gearbox/init.lua lines 180-182).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GearboxShape { T, L, ST }
pub fn gearbox_shape(gearbox_id: &str) -> GearboxShape {
    let up = gearbox_id.to_ascii_uppercase();
    if up.ends_with("-ST") { GearboxShape::ST } else if up.ends_with("-T") { GearboxShape::T } else { GearboxShape::L }
}
