//! The parametric tank: a `TankSpec` in, a build file out. It writes the
//! conventions the reference builds share — hull along +Y at yaw 90, wheels
//! wheel-first on their own Y, engines in a row at the rear into a CVT-T and
//! two vertical linears, turret ring → trunnion → gun, crew in the basket
//! and hull, controller with brakes engaged and turrets capped — and leaves
//! the result as TOML the builder can read and edit before `--build`.
//!
//! Parts are named as `--extract-all` names them, so the spec only needs to
//! know which bin it draws from; `catalogue()` swaps every part for the
//! matching catalogue item, and then no bin is needed at all.

use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Suspension {
    /// One axis per wheel, nothing else (ironlock, tangk).
    Simple,
    /// Axes plus three orientation locks per same-side pair.
    Locked,
    /// Elastic + ropes + socket per wheel, sprockets on axes (VDC).
    Sprung,
}

impl Suspension {
    pub fn parse(s: &str) -> Option<Suspension> {
        match s.to_lowercase().as_str() {
            "simple" | "rigid" | "ironlock" => Some(Suspension::Simple),
            "locked" | "brookster" => Some(Suspension::Locked),
            "sprung" | "vdc" | "elastic" => Some(Suspension::Sprung),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TankSpec {
    pub output: String,
    /// Folder of single-entity dupes from `--extract-all`.
    pub parts: String,
    /// Part names, as the bin names them.
    pub baseplate: String,
    pub wheel_right: String,
    pub wheel_left: String,
    pub engine: String,
    pub cvt: String,
    pub linear: String,
    pub fuel: String,
    pub ring: String,
    pub trunnion: String,
    pub gun: String,
    pub motor: String,
    pub ammo: String,
    pub crew_driver: String,
    pub crew_gunner: String,
    pub crew_loader: String,
    pub controller: String,
    pub pod: String,
    /// Layout.
    pub wheels_per_side: usize,
    pub wheel_spacing: f64,
    pub track_width: f64,
    pub wheel_drop: f64,
    pub engines: usize,
    pub suspension: Suspension,
    pub ring_height: f64,
    pub turret_forward: f64,
    /// Turret caps and elevation, from the official T-34.
    pub ring_speed: f64,
    pub trunnion_speed: f64,
    pub elevation: (f64, f64),
    /// Spawn catalogue items instead of parts from a bin.
    pub catalogue: bool,
    /// The gun's calibre in mm; only a catalogue gun needs telling.
    pub gun_caliber: f64,
    /// The crate's ammo type; only a catalogue crate needs telling.
    pub ammo_type: String,
}

impl Default for TankSpec {
    fn default() -> Self {
        TankSpec {
            output: "tank.txt".into(),
            parts: "parts".into(),
            baseplate: "baseplate_cube".into(),
            wheel_right: "prop_physics_tank30".into(),
            wheel_left: "prop_physics_tank30_2".into(),
            engine: "engine_6_5-i6".into(),
            cvt: "gearbox_cvt-t".into(),
            linear: "gearbox_2gear-l".into(),
            fuel: "fueltank_box".into(),
            ring: "turret_turret-h".into(),
            trunnion: "turret_turret-v".into(),
            gun: "gun_c125".into(),
            motor: "turret_motor_motor-elc".into(),
            ammo: "ammo_c125".into(),
            crew_driver: "crew_driver".into(),
            crew_gunner: "crew_gunner".into(),
            crew_loader: "crew_loader".into(),
            controller: "controller_plate025x025".into(),
            pod: "prop_vehicle_prisoner_pod_cube".into(),
            wheels_per_side: 4,
            wheel_spacing: 38.0,
            track_width: 102.0,
            wheel_drop: 11.0,
            engines: 4,
            suspension: Suspension::Simple,
            ring_height: 42.8,
            turret_forward: 24.3,
            ring_speed: 30.0,
            trunnion_speed: 10.0,
            elevation: (-7.0, 20.0),
            catalogue: false,
            gun_caliber: 125.0,
            ammo_type: "AP".into(),
        }
    }
}

impl TankSpec {
    /// The same layout drawn from the catalogue: every part becomes the
    /// item the reference bins were extracted from, the gun keeps its
    /// calibre, the crate matches the gun, and there is no pod, because a
    /// pasted baseplate makes its own seat in game.
    pub fn catalogue(mut self) -> Self {
        let rac = self.gun.to_lowercase().contains("rac");
        self.catalogue = true;
        self.parts = String::new();
        self.baseplate = "GroundVehicle".into();
        self.wheel_right = "Tankwheel30".into();
        self.wheel_left = "Tankwheel30".into();
        self.engine = "6.5-I6".into();
        self.cvt = "CVT-T".into();
        self.linear = "2Gear-L".into();
        self.fuel = "Diesel".into();
        self.ring = "Turret-H".into();
        self.trunnion = "Turret-V".into();
        self.gun = if rac { "RAC".into() } else { "C".into() };
        self.motor = "Motor-ELC".into();
        self.ammo = self.ammo_type.clone();
        self.crew_driver = "Driver".into();
        self.crew_gunner = "Gunner".into();
        self.crew_loader = "Loader".into();
        self.controller = "Controller".into();
        self.pod = String::new();
        self
    }
}

/// Named specs. A preset is a starting point, not a rule.
pub fn preset(name: &str) -> Option<TankSpec> {
    let base = TankSpec::default();
    Some(match name.to_lowercase().as_str() {
        "light" => TankSpec { wheels_per_side: 3, engines: 2, track_width: 90.0, suspension: Suspension::Simple, ring_speed: 40.0, ..base },
        "medium" => TankSpec { wheels_per_side: 4, engines: 3, suspension: Suspension::Simple, ..base },
        "heavy" => TankSpec { wheels_per_side: 6, wheel_spacing: 34.0, track_width: 118.0, engines: 5, suspension: Suspension::Sprung, ring_speed: 22.0, trunnion_speed: 8.0, ..base },
        "spaa" => TankSpec { wheels_per_side: 4, engines: 2, gun: "gun_rac37".into(), ammo: "ammo_rac37".into(), gun_caliber: 37.0, suspension: Suspension::Simple, ring_speed: 90.0, trunnion_speed: 60.0, elevation: (-10.0, 85.0), ..base },
        "stationary" => TankSpec { wheels_per_side: 0, engines: 0, suspension: Suspension::Simple, ..base },
        _ => return None,
    })
}

fn v(x: f64, y: f64, z: f64) -> String {
    format!("[{x}, {y}, {z}]")
}

/// Part name with the `_N` suffix the extractor gives duplicates; the first
/// of a kind has none.
fn nth(base: &str, i: usize) -> String {
    if i == 0 {
        base.to_owned()
    } else {
        format!("{base}_{}", i + 1)
    }
}

/// Writes the build file. Every section the file format has is used where
/// the spec calls for it; sections run in the fixed order `buildfile` uses.
pub fn write_build_file(spec: &TankSpec) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "# generated by ad2read --gen-tank; edit freely, then: ad2read --build {}", "this file");
    let _ = writeln!(o, "output = \"{}\"", spec.output);
    if !spec.catalogue {
        let _ = writeln!(o, "parts = \"{}\"", spec.parts);
    }
    let _ = writeln!(o);

    let source = if spec.catalogue { "item" } else { "part" };
    let spawn = |o: &mut String, name: &str, part: &str, at: (f64, f64, f64), ang: (f64, f64, f64), parent: Option<&str>, set: &[(&str, f64)]| {
        let _ = writeln!(o, "[[spawn]]\nname = \"{name}\"\n{source} = \"{part}\"\nat = {}\nang = {}", v(at.0, at.1, at.2), v(ang.0, ang.1, ang.2));
        if let Some(p) = parent {
            let _ = writeln!(o, "parent = \"{p}\"");
        }
        if !set.is_empty() {
            let pairs: Vec<String> = set.iter().map(|(k, val)| format!("{k} = {val}")).collect();
            let _ = writeln!(o, "set = {{ {} }}", pairs.join(", "));
        }
        let _ = writeln!(o);
    };
    // A bin numbers repeats (engine, engine_2); the catalogue has one item
    // per kind.
    let nth_of = |base: &str, i: usize| if spec.catalogue { base.to_owned() } else { nth(base, i) };

    // Hull at the origin, forward +Y. A catalogue baseplate is sized to the
    // layout: wheels just outside its width, engines and loader on it.
    let n = spec.wheels_per_side;
    let half_span = if n > 0 { (n as f64 - 1.0) * spec.wheel_spacing / 2.0 } else { 0.0 };
    let hull_set: Vec<(&str, f64)> = if spec.catalogue {
        vec![("Width", spec.track_width - 10.0), ("Length", (2.0 * half_span + 130.0).max(96.0)), ("Thickness", 2.0)]
    } else {
        Vec::new()
    };
    spawn(&mut o, "hull", &spec.baseplate, (0.0, 0.0, 0.0), (0.0, 90.0, 0.0), None, &hull_set);

    // Wheels: N per side, centred on y = 0, right side yaw 90, left yaw -90.
    let x = spec.track_width / 2.0;
    let mut right = Vec::new();
    let mut left = Vec::new();
    for i in 0..n {
        let y = -half_span + i as f64 * spec.wheel_spacing;
        let r = format!("fr{}", i + 1);
        let l = format!("fl{}", i + 1);
        for (name, part, xx, yaw) in [(&r, &spec.wheel_right, x, 90.0), (&l, &spec.wheel_left, -x, -90.0)] {
            let _ = writeln!(o, "[[wheel]]\nname = \"{name}\"\n{source} = \"{part}\"\nat = {}\nang = {}\nbase = \"hull\"\naxle = [0, 1, 0]\n", v(xx, y, -spec.wheel_drop), v(0.0, yaw, 0.0));
        }
        right.push(r);
        left.push(l);
    }

    // Engines in a row behind the rear axle, then the drivetrain.
    let rear = if n == 0 { -70.0 } else { -half_span - spec.wheel_spacing * 0.4 - 30.0 };
    let e = spec.engines;
    let e_span = (e as f64 - 1.0) * 11.6 / 2.0;
    for i in 0..e {
        spawn(&mut o, &format!("eng{}", i + 1), &nth_of(&spec.engine, i), (-e_span + i as f64 * 11.6, rear, -0.7), (0.0, -90.0, 0.0), Some("hull"), &[]);
    }
    let drivetrain = e > 0;
    if drivetrain {
        spawn(&mut o, "cvt", &spec.cvt, (0.0, rear + 54.0, 31.0), (0.0, -90.0, -180.0), Some("hull"), &[]);
        spawn(&mut o, "linl", &nth_of(&spec.linear, 0), (-23.6, rear + 70.0, 23.4), (-90.0, 90.0, 0.0), Some("hull"), &[]);
        spawn(&mut o, "linr", &nth_of(&spec.linear, 1), (24.7, rear + 70.0, 23.4), (90.0, -90.0, 0.0), Some("hull"), &[]);
        spawn(&mut o, "fuel1", &nth_of(&spec.fuel, 0), (19.0, rear + 30.6, 15.1), (0.0, 180.0, 0.0), Some("hull"), &[]);
        spawn(&mut o, "fuel2", &nth_of(&spec.fuel, 1), (-17.7, rear + 30.6, 15.1), (0.0, 180.0, 0.0), Some("hull"), &[]);
    }

    // Turret stack.
    let ty = spec.turret_forward;
    let rz = spec.ring_height;
    let gun_set: Vec<(&str, f64)> = if spec.catalogue { vec![("Caliber", spec.gun_caliber)] } else { Vec::new() };
    spawn(&mut o, "ring", &spec.ring, (0.0, ty, rz), (0.0, 90.0, 0.0), Some("hull"), &[]);
    spawn(&mut o, "trun", &spec.trunnion, (0.0, ty + 9.2, rz + 10.4), (0.0, 90.0, 0.0), Some("ring"), &[]);
    spawn(&mut o, "gun", &spec.gun, (0.0, ty + 39.5, rz + 9.9), (0.0, 90.0, 0.0), Some("trun"), &gun_set);
    spawn(&mut o, "motorh", &nth_of(&spec.motor, 0), (-20.3, ty - 2.0, rz + 5.0), (0.0, 180.0, 0.0), Some("ring"), &[]);
    spawn(&mut o, "motorv", &nth_of(&spec.motor, 1), (-0.3, ty + 1.5, rz + 9.4), (0.0, -180.0, 0.0), Some("trun"), &[]);
    spawn(&mut o, "ammo", &spec.ammo, (21.0, ty + 47.0, 11.5), (0.0, 0.0, 0.0), Some("hull"), &[]);
    spawn(&mut o, "driver", &spec.crew_driver, (0.7, -10.6, -0.1), (0.0, 0.0, 30.0), Some("hull"), &[]);
    spawn(&mut o, "gunner", &spec.crew_gunner, (12.9, ty + 12.5, 6.6), (0.0, 0.0, 15.0), Some("ring"), &[]);
    spawn(&mut o, "loader", &spec.crew_loader, (-22.4, ty + 72.0, -0.1), (0.0, 0.0, 30.0), Some("hull"), &[]);
    spawn(&mut o, "ctrl", &spec.controller, (0.0, 0.0, 2.0), (0.0, -180.0, 0.0), Some("hull"), &[]);
    if !spec.catalogue {
        spawn(&mut o, "pod", &spec.pod, (0.0, 0.0, 0.0), (0.0, 90.0, 0.0), Some("hull"), &[]);
    }

    // Suspension extras.
    match spec.suspension {
        Suspension::Simple => {}
        Suspension::Locked => {
            for side in [&right, &left] {
                for w in side.windows(2) {
                    let _ = writeln!(o, "[[lock_pair]]\na = \"{}\"\nb = \"{}\"\n", w[0], w[1]);
                }
            }
        }
        Suspension::Sprung => {
            for w in right.iter().chain(left.iter()) {
                let _ = writeln!(o, "[[sprung]]\nwheel = \"{w}\"\nbase = \"hull\"\n");
            }
        }
    }

    // Links: the reference graph.
    let link = |o: &mut String, a: &str, b: &str| {
        let _ = writeln!(o, "[[link]]\na = \"{a}\"\nb = \"{b}\"\n");
    };
    for i in 0..e {
        let eng = format!("eng{}", i + 1);
        link(&mut o, &eng, "cvt");
        link(&mut o, &eng, "fuel1");
        link(&mut o, &eng, "fuel2");
    }
    if drivetrain {
        link(&mut o, "cvt", "linl");
        link(&mut o, "cvt", "linr");
    }
    let drive: Vec<&String> = if !drivetrain { Vec::new() } else { match spec.suspension {
        // Locked pairs drive one wheel per side; the locks carry the rest.
        Suspension::Locked if !right.is_empty() => vec![right.last().unwrap(), left.last().unwrap()],
        Suspension::Locked => Vec::new(),
        _ => right.iter().chain(left.iter()).collect(),
    } };
    for w in drive {
        let lin = if right.contains(w) { "linr" } else { "linl" };
        link(&mut o, lin, w);
    }
    link(&mut o, "gun", "ammo");
    link(&mut o, "ring", "motorh");
    link(&mut o, "trun", "motorv");
    for t in ["hull", "cvt", "pod", "ring", "trun", "gun"] {
        if t == "cvt" && !drivetrain { continue; }
        if t == "pod" && spec.catalogue { continue; }
        link(&mut o, "ctrl", t);
    }
    if !spec.catalogue {
        link(&mut o, "hull", "pod");
    }
    // crew_types.lua LinkHandlers: Driver → baseplate, Gunner → turret,
    // Loader → gun. A gunner on the gun is refused in game.
    link(&mut o, "driver", "hull");
    link(&mut o, "gunner", "ring");
    link(&mut o, "loader", "gun");

    // Tuning the references got wrong or left unset.
    let put = |o: &mut String, e: &str, k: &str, val: f64| {
        let _ = writeln!(o, "[[put]]\nentity = \"{e}\"\nkey = \"{k}\"\nvalue = {val}\n");
    };
    put(&mut o, "ctrl", "DT.BrakeEngagement", 1.0);
    put(&mut o, "ring", "MaxSpeed", spec.ring_speed);
    put(&mut o, "trun", "MaxSpeed", spec.trunnion_speed);
    put(&mut o, "trun", "MinDeg", spec.elevation.0);
    put(&mut o, "trun", "MaxDeg", spec.elevation.1);
    // A catalogue crate is told what it feeds; a bin crate came out of a
    // reference already matched to its gun.
    if spec.catalogue {
        let _ = writeln!(o, "[[ammo]]\ncrate = \"ammo\"\ntype = \"{}\"\n", spec.ammo_type);
        let _ = writeln!(o, "[[put]]\nentity = \"ammo\"\nkey = \"Weapon\"\nvalue = \"{}\"\n", spec.gun);
        put(&mut o, "ammo", "Caliber", spec.gun_caliber);
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_spec_is_a_valid_build_file_with_the_reference_graph() {
        let spec = TankSpec::default();
        let text = write_build_file(&spec);
        let file: crate::buildfile::BuildFile = toml::from_str(&text).expect("parses");
        assert_eq!(file.wheel.len(), 8);
        assert_eq!(file.spawn.iter().filter(|s| s.name.starts_with("eng")).count(), 4);
        // Every link names a part that exists.
        let names: std::collections::HashSet<&str> = file.spawn.iter().map(|s| s.name.as_str()).chain(file.wheel.iter().map(|w| w.name.as_str())).collect();
        for l in &file.link {
            assert!(names.contains(l.a.as_str()) && names.contains(l.b.as_str()), "{:?}", l);
        }
        assert!(file.put.iter().any(|p| p.key == "DT.BrakeEngagement"));
        assert!(file.lock_pair.is_empty() && file.sprung.is_empty());
    }

    #[test]
    fn every_preset_builds_from_the_catalogue_alone_and_passes_the_doctor() {
        for name in ["light", "medium", "heavy", "spaa", "stationary"] {
            let spec = preset(name).unwrap().catalogue();
            let text = write_build_file(&spec);
            assert!(!text.contains("part = "), "{name}: no parts bin in a catalogue build");
            let file: crate::buildfile::BuildFile = toml::from_str(&text).expect(name);
            let (mut dupe, _) = crate::buildfile::run(&file, std::path::Path::new(".")).expect(name);
            crate::extras::repair_head(&mut dupe);
            crate::doctor::clean(&mut dupe);
            let findings = crate::doctor::doctor(&dupe);
            crate::doctor::apply_fixes(&mut dupe, &findings);
            let problems = crate::extras::validate(&dupe);
            assert!(problems.is_empty(), "{name}: {problems:?}");
            let errors: Vec<String> = crate::doctor::doctor(&dupe)
                .into_iter()
                .filter(|f| matches!(f.severity, crate::doctor::Severity::Error))
                .map(|f| f.what)
                .collect();
            assert!(errors.is_empty(), "{name}: {errors:?}");
        }
    }

    #[test]
    fn presets_parse_including_a_stationary_one_with_no_drivetrain() {
        for name in ["light", "medium", "heavy", "spaa", "stationary"] {
            let spec = preset(name).unwrap();
            let f: crate::buildfile::BuildFile = toml::from_str(&write_build_file(&spec)).expect(name);
            if name == "stationary" {
                assert!(f.wheel.is_empty());
                assert!(!f.spawn.iter().any(|s| s.name == "cvt"));
                assert!(!f.link.iter().any(|l| l.b == "cvt"));
            } else {
                assert!(!f.wheel.is_empty());
            }
        }
        assert!(preset("nope").is_none());
    }

    #[test]
    fn suspension_styles_change_the_sections_and_the_drive_links() {
        let mut spec = TankSpec { wheels_per_side: 3, ..TankSpec::default() };
        spec.suspension = Suspension::Locked;
        let f: crate::buildfile::BuildFile = toml::from_str(&write_build_file(&spec)).unwrap();
        assert_eq!(f.lock_pair.len(), 4, "two pairs per side of three");
        let wheel_links = f.link.iter().filter(|l| l.a.starts_with("lin")).count();
        assert_eq!(wheel_links, 2, "locked pairs drive one wheel per side");
        spec.suspension = Suspension::Sprung;
        let f: crate::buildfile::BuildFile = toml::from_str(&write_build_file(&spec)).unwrap();
        assert_eq!(f.sprung.len(), 6);
        assert_eq!(f.link.iter().filter(|l| l.a.starts_with("lin")).count(), 6);
        assert_eq!(Suspension::parse("VDC"), Some(Suspension::Sprung));
        assert_eq!(Suspension::parse("nope"), None);
    }
}
