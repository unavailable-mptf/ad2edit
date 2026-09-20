//! The eight verbs a builder needs: new, add, wire, check, fix, build, send,
//! explain. Each is rewritten into the flags that already do the work, so
//! the flags stay as plumbing and nothing is implemented twice. A verb that
//! changes a dupe writes it back in place; every write still goes through
//! the same validation as `--out`.

use crate::roles::Role;

/// The catalogue item a role is filled with when none is named: what the
/// reference tanks and the presets use.
pub fn default_item(role: Role) -> Option<&'static str> {
    Some(match role {
        Role::Hull => "GroundVehicle",
        Role::RoadWheel => "Tankwheel30",
        Role::Engine => "6.5-I6",
        Role::FuelTank => "Diesel",
        Role::Gearbox => "CVT-T",
        Role::FinalDrive => "2Gear-L",
        Role::TurretRing => "Turret-H",
        Role::Trunnion => "Turret-V",
        Role::TurretMotor => "Motor-ELC",
        Role::MainGun => "C",
        Role::SecondaryGun => "RAC",
        Role::AmmoCrate => "AP",
        Role::Driver => "Driver",
        Role::Gunner => "Gunner",
        Role::Loader => "Loader",
        Role::Controller => "Controller",
        Role::Chip => "@name New chip",
        _ => return None,
    })
}

/// A role by the name `Role::name` gives it, spaces or hyphens alike.
fn role_named(word: &str) -> Option<Role> {
    use Role::*;
    let word = word.to_ascii_lowercase().replace(['-', '_'], " ");
    [Hull, RoadWheel, Engine, FuelTank, Gearbox, FinalDrive, TurretRing, Trunnion, TurretMotor, MainGun, SecondaryGun, AmmoCrate, Driver, Gunner, Loader, Controller, Chip]
        .into_iter()
        .find(|role| role.name() == word)
}

/// Where the file as it was before the last in-place verb is kept, so
/// `undo` can bring it back.
pub fn previous_path(file: &str) -> String {
    format!("{file}.prev")
}

/// The file a verb is about to change in place, if it is one that does.
pub fn changes_in_place(args: &[String]) -> Option<&str> {
    match args.get(1).map(String::as_str) {
        Some("add" | "wire" | "fix") => args.get(2).map(String::as_str),
        _ => None,
    }
}

/// `args` as the program received them, program name first. None when the
/// first word is not a verb; otherwise the flag form to run, or what was
/// missing.
pub fn expand(args: &[String]) -> Option<Result<Vec<String>, String>> {
    let verb = args.get(1)?.as_str();
    if !matches!(verb, "new" | "add" | "wire" | "check" | "fix" | "build" | "send" | "explain") {
        return None;
    }
    let program = args[0].clone();
    let rest = &args[2..];
    let file = |what: &str| rest.first().cloned().ok_or_else(|| format!("{verb} needs {what}"));
    let flags = |parts: Vec<String>| {
        let mut out = vec![program.clone()];
        out.extend(parts);
        out
    };
    let own = |words: &[&str]| words.iter().map(|word| (*word).to_owned()).collect::<Vec<_>>();
    Some((|| match verb {
        "new" => {
            let file = file("a file to create: new tank.txt [--baseplate | --preset light]")?;
            let mut parts = own(&["--new"]);
            parts.push(file);
            parts.extend(rest[1..].iter().cloned());
            Ok(flags(parts))
        }
        "build" => {
            let file = file("a build file: build tank.toml")?;
            let mut parts = own(&["--build"]);
            parts.push(file);
            parts.extend(rest[1..].iter().cloned());
            Ok(flags(parts))
        }
        "check" => {
            let mut parts = vec![file("a dupe: check tank.txt")?];
            parts.extend(own(&["--doctor"]));
            parts.extend(rest[1..].iter().cloned());
            Ok(flags(parts))
        }
        "fix" => {
            let file = file("a dupe: fix tank.txt")?;
            Ok(flags(vec![file.clone(), "--fix".into(), "--out".into(), file]))
        }
        "send" => Ok(flags(vec![file("a dupe: send tank.txt")?, "--send".into()])),
        "explain" => {
            let file = file("a dupe and an entity: explain tank.txt 12")?;
            let entity = rest.get(1).cloned().ok_or("explain needs an entity index: explain tank.txt 12")?;
            Ok(flags(vec![file, "--explain".into(), entity]))
        }
        "wire" => {
            let file = file("a dupe: wire tank.txt <source> <output> <target> <input>")?;
            let ends = rest.get(1..5).ok_or("wire needs <source> <output> <target> <input>")?;
            let mut parts = vec![file.clone(), "--wire".into()];
            parts.extend(ends.iter().cloned());
            parts.extend(["--out".to_owned(), file]);
            Ok(flags(parts))
        }
        "add" => {
            let file = file("a dupe: add tank.txt <role or item> [x,y,z] [p,y,r] [Key=value ...]")?;
            let what = rest.get(1).ok_or("add needs a role (main-gun, engine, driver, ...) or a catalogue item")?;
            // A role may be followed by the item to fill it with; otherwise
            // it gets the one the reference tanks use.
            let (item, after) = match role_named(what) {
                Some(role) => match rest.get(2).filter(|next| crate::catalog::find(next).is_some()) {
                    Some(named) => (named.clone(), 3),
                    None => (default_item(role).ok_or_else(|| format!("{what} has no usual catalogue item; name one"))?.to_owned(), 2),
                },
                None => (what.clone(), 2),
            };
            if crate::catalog::find(&item).is_none() {
                return Err(format!("{item} is neither a role nor a catalogue item; see --catalog"));
            }
            let mut parts = vec![file.clone(), "--add".into(), item];
            match rest.get(after).filter(|at| at.contains(',') && !at.contains('=')) {
                Some(at) => parts.push(at.clone()),
                None => parts.push("0,0,0".into()),
            }
            let placed = rest.get(after).is_some_and(|at| at.contains(',') && !at.contains('='));
            parts.extend(rest[(after + placed as usize).min(rest.len())..].iter().cloned());
            parts.extend(["--out".to_owned(), file]);
            Ok(flags(parts))
        }
        _ => unreachable!(),
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(words: &[&str]) -> Option<Result<Vec<String>, String>> {
        let args: Vec<String> = std::iter::once("ad2read").chain(words.iter().copied()).map(str::to_owned).collect();
        expand(&args)
    }

    fn flags(words: &[&str]) -> Vec<String> {
        run(words).expect("a verb").expect("well formed")[1..].to_vec()
    }

    #[test]
    fn a_verb_becomes_the_flags_that_already_do_the_work() {
        assert_eq!(flags(&["check", "t.txt"]), ["t.txt", "--doctor"]);
        assert_eq!(flags(&["check", "t.txt", "--json"]), ["t.txt", "--doctor", "--json"]);
        assert_eq!(flags(&["fix", "t.txt"]), ["t.txt", "--fix", "--out", "t.txt"]);
        assert_eq!(flags(&["build", "t.toml"]), ["--build", "t.toml"]);
        assert_eq!(flags(&["send", "t.txt"]), ["t.txt", "--send"]);
        assert_eq!(flags(&["explain", "t.txt", "12"]), ["t.txt", "--explain", "12"]);
        assert_eq!(flags(&["wire", "t.txt", "5", "W", "1", "Throttle"]), ["t.txt", "--wire", "5", "W", "1", "Throttle", "--out", "t.txt"]);
        assert_eq!(flags(&["new", "t.txt", "--preset", "light"]), ["--new", "t.txt", "--preset", "light"]);
    }

    #[test]
    fn add_takes_a_role_or_an_item_and_a_role_brings_its_usual_item() {
        assert_eq!(flags(&["add", "t.txt", "main-gun"]), ["t.txt", "--add", "C", "0,0,0", "--out", "t.txt"]);
        assert_eq!(flags(&["add", "t.txt", "main gun", "HW", "0,60,50", "Caliber=105"]), ["t.txt", "--add", "HW", "0,60,50", "Caliber=105", "--out", "t.txt"]);
        assert_eq!(flags(&["add", "t.txt", "CVT-T", "0,-18,31", "0,-90,-180"]), ["t.txt", "--add", "CVT-T", "0,-18,31", "0,-90,-180", "--out", "t.txt"]);
        assert!(run(&["add", "t.txt", "flux-capacitor"]).unwrap().is_err());
        assert!(run(&["add", "t.txt"]).unwrap().is_err());
    }

    #[test]
    fn every_role_with_a_usual_item_names_one_the_catalogue_has() {
        use Role::*;
        for role in [Hull, RoadWheel, Engine, FuelTank, Gearbox, FinalDrive, TurretRing, Trunnion, TurretMotor, MainGun, SecondaryGun, AmmoCrate, Driver, Gunner, Loader, Controller, Chip] {
            let item = default_item(role).unwrap_or_else(|| panic!("{role:?} has no usual item"));
            assert!(crate::catalog::find(item).is_some(), "{role:?} -> {item}");
            assert_eq!(role_named(&role.name().replace(' ', "-")), Some(role));
        }
    }

    #[test]
    fn only_the_verbs_that_write_in_place_are_kept_for_undo() {
        let words = |list: &[&str]| std::iter::once("ad2read").chain(list.iter().copied()).map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(changes_in_place(&words(&["add", "t.txt", "engine"])), Some("t.txt"));
        assert_eq!(changes_in_place(&words(&["fix", "t.txt"])), Some("t.txt"));
        assert_eq!(changes_in_place(&words(&["check", "t.txt"])), None);
        assert_eq!(changes_in_place(&words(&["t.txt", "--doctor"])), None);
        assert_eq!(previous_path("t.txt"), "t.txt.prev");
    }

    #[test]
    fn anything_else_is_left_to_the_flags() {
        assert!(run(&["t.txt", "--doctor"]).is_none());
        assert!(run(&["--build", "t.toml"]).is_none());
        assert!(run(&[]).is_none());
    }
}
