use std::env;
use std::fs;
use std::path::PathBuf;

use ad2read::dupe::Dupe;
use ad2read::dupefile::{self, AloneHeader};
use ad2read::history::History;
use ad2read::transform::{self, Vec3};
use ad2read::value::{self, dump, escape, hexline};
use ad2read::{build, bulk, duplicate, merge, refs};

/// Everything needed to write a file back out in the same shape it came in.
struct Loaded {
    file: dupefile::DupeFile,
    raw: Vec<u8>,
    dupe: Dupe,
    lzma: (u32, u32, u32, u32, bool),
    lzma_text: String,
}

/// A file that doesn't exist yet, for `ad2read new.txt --add …`: an empty
/// dupe with a fresh info block. HeadEnt is repaired at save from the first
/// entity added.
fn empty_loaded(path: &str) -> Loaded {
    let dupe = ad2read::buildfile::empty_dupe();
    let name = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "new".into());
    let info: Vec<(Vec<u8>, Vec<u8>)> = [("name", name.as_str()), ("date", "new"), ("time", ""), ("timezone", ""), ("size", "0"), ("check", "\r\n\t\n")]
        .iter().map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec())).collect();
    Loaded {
        file: dupefile::DupeFile { revision: 5, info, compressed: Vec::new() },
        raw: Vec::new(),
        dupe,
        lzma: (3, 0, 2, 65536, true),
        lzma_text: "new file".into(),
    }
}

fn load(path: &str) -> Result<Loaded, String> {
    if !std::path::Path::new(path).exists() {
        // Only for building: an inspect of a missing file is still an error.
        let building = std::env::args().any(|a| matches!(a.as_str(), "--add" | "--spawn" | "--spawn-part" | "--parts"));
        if building {
            return Ok(empty_loaded(path));
        }
    }
    let bytes = fs::read(path).map_err(|e| format!("could not read {path}: {e}"))?;
    let file = dupefile::parse(&bytes)?;
    if file.revision != 5 {
        return Err(format!(
            "{path} is revision {}, only 5 is handled",
            file.revision
        ));
    }
    let header = AloneHeader::parse(&file.compressed)
        .ok_or("compressed body too short to hold an LZMA header")?;
    let lzma = (
        header.lc,
        header.lp,
        header.pb,
        header.dict,
        header.size != u64::MAX,
    );
    let lzma_text = header.describe();
    let raw = dupefile::decompress(&file.compressed)?;
    let (dupe, consumed) = Dupe::from_body(&raw)?;
    if consumed != raw.len() {
        return Err(format!(
            "{} bytes left over after decoding {path}",
            raw.len() - consumed
        ));
    }
    Ok(Loaded { file, raw, dupe, lzma, lzma_text })
}

fn parse_triple(s: &str) -> Option<Vec3> {
    let p: Vec<f64> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    match p.as_slice() {
        [a, b, c] => Some((*a, *b, *c)),
        _ => None,
    }
}

fn parse_list(s: &str) -> Option<Vec<f64>> {
    let p: Vec<f64> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    if p.is_empty() {
        None
    } else {
        Some(p)
    }
}

/// Splits "Key=Value" once, so values containing '=' survive.
fn parse_pair(s: &str) -> Option<(String, String)> {
    let (k, v) = s.split_once('=')?;
    if k.is_empty() {
        None
    } else {
        Some((k.to_string(), v.to_string()))
    }
}

fn usage() {
    eprintln!("the eight verbs:");
    eprintln!("  ad2read new <dupe.txt> [--baseplate | --preset light|medium|heavy|spaa|stationary]");
    eprintln!("  ad2read add <dupe.txt> <role or item> [x,y,z] [p,y,r] [Key=value ...]   roles: hull road-wheel engine fuel-tank gearbox final-drive turret-ring trunnion turret-motor main-gun secondary-gun ammo-crate driver gunner loader controller chip");
    eprintln!("  ad2read wire <dupe.txt> <source> <output> <target> <input>");
    eprintln!("  ad2read check <dupe.txt>        the doctor");
    eprintln!("  ad2read fix <dupe.txt>          the doctor's mechanical fixes, written back");
    eprintln!("  ad2read build <tank.toml>");
    eprintln!("  ad2read send <dupe.txt>         validate, then copy into the AdvDupe2 folder");
    eprintln!("  ad2read explain <dupe.txt> <n>");
    eprintln!("  ad2read undo <dupe.txt>         take back the last add, wire or fix; again to redo");
    eprintln!("everything else, as flags:");
    eprintln!("  ad2read <dupe.txt>                          inspect");
    eprintln!("  ad2read <dupe.txt> --full                   inspect, every key, no truncation");
    eprintln!("  ad2read <dupe.txt> --list-entities          list entity indices");
    eprintln!("  ad2read <dupe.txt> --scan-refs              find entity-index references");
    eprintln!("  ad2read <dupe.txt> --dump-mods              list every EntityMods modifier");
    eprintln!("  ad2read <dupe.txt> --dump-constraints       list constraint types and fields");
    eprintln!("  ad2read <dupe.txt> --dump-models <garrysmod>  resolve each model's real size");
    eprintln!("  ad2read <dupe.txt> --diff <other.txt>       what differs between two dupes");
    eprintln!("  ad2read <dupe.txt> --weld-world <n>         weld one entity to the world");
    eprintln!("  building (any number, in order, then --out <file> or the _edit default):");
    eprintln!("  --spawn <template.txt> <n> <x,y,z> [p,y,r]  copy entity n from a template");
    eprintln!("  --axis <a> <b> <x,y,z> <dx,dy,dz> [friction] [forcelimit] [torquelimit] [nocollide]");
    eprintln!("  --weld <a> <b> [forcelimit] [nocollide]");
    eprintln!("  --nocollide <a> <b>");
    eprintln!("  --ballsocket <a> <b> <x,y,z>");
    eprintln!("  --link <a> <b>                              ACF link, validated");
    eprintln!("  --chip-export <n> <file>                    E2/Starfall code out to a file");
    eprintln!("  --chip-import <n> <file>                    and back in");
    eprintln!("  --models <garrysmod> --align-axle <n> <dx,dy,dz>  turn a wheel so its axle points that way");
    eprintln!("  --wheel-axis <wheel> <base> <ax,ay,az> [nocollide]  hinge a wheel to the base about its own axle");
    eprintln!("  --show <n>                                  one entity, every key");
    eprintln!("  ad2read --model-info <model> [--models <gmod>]   a model's box and which corners of it are solid");
    eprintln!("  --hull [height] [glacis degrees] [mm]          a sloped SProps hull on the baseplate, armoured and parented");
    eprintln!("  --mass | --mass-check [--models <gmod>]         total weight by role; or every part not weighing what the game saved");
    eprintln!("  --lock-pair <a> <b>                         same-side lock (3 sockets)");
    eprintln!("  --sprung-wheel <wheel> <base>               VDC sprung road wheel (elastic, ropes, socket)");
    eprintln!("  --hydraulic-wheel <wheel> <base> <ctrl>     T-34 hydraulic wheel; ctrl is a gmod_wire_hydraulic");
    eprintln!("  --spherical <n> [radius]                    Make Spherical; radius from --models bounds");
    eprintln!("  --wire <src> <output> <dst> <input>          a wire, in WireDupeInfo");
    eprintln!("  --ammo <crate> <AmmoType> [x,y,z]          type validated against ACF's registry");
    eprintln!("  --parts <dir> --spawn-part <name> <x,y,z> [p,y,r]   spawn <dir>/<name>.txt's entity");
    eprintln!("  --extract <n> <file>                        one entity as its own template dupe");
    eprintln!("  --doctor                                    check against what working ACF builds do");
    eprintln!("  --json                                      whole dupe as JSON (typed $vec/$ang)");
    eprintln!("  --extract-all <dir>                         every ACF part as its own template dupe");
    eprintln!("  --track-link <track> <chassis> <w1,w2,..>   tank track wheel list, in track order");
    eprintln!("  --fading-door <n> <key> [material]         Fading Door; key is a numpad key number");
    eprintln!("  --submaterial <n> <slot> <material>        Submaterial override for one slot");
    eprintln!("  --keypad <n> <password|none> [key]         keypad or keypad_wire; key = numpad it presses");
    eprintln!("  --resize <n> <sx,sy,sz> [bones]            Resizer (BoneManip scale, restored by GMod)");
    eprintln!("  ad2read --build <file.toml>                  build a dupe from a build file (see RECIPE.md)");
    eprintln!("  ad2read --gen-tank <out.toml> [--preset light|medium|heavy|spaa|stationary] [--wheels n --engines n --suspension ...]");
    eprintln!("  ad2read --watch <file.toml> [--into <advdupe2 folder>]   rebuild+fix+clean on every save, copy in");
    eprintln!("  (--build fixes and cleans by itself; --no-fix to skip; --models <gmod> gives the doctor real driveshaft points)");
    eprintln!("  --lock <a> <b> <x|y|z>                      orientation lock on one axis (3 per wheel pair)");
    eprintln!("  --parent <child> <parent>                   set DupeParentID");
    eprintln!("  --put <n> <key|a.b.c> <value>              create-or-set a field; DT.x reaches networked vars");
    eprintln!("  --unmod <n> <modifier>                      remove an EntityMods modifier");
    eprintln!("  ad2read <dupe.txt> --delete-entity <n>      delete one entity");
    eprintln!("  ad2read <dupe.txt> --remap <old> <new>      renumber an entity");
    eprintln!("  ad2read <dupe.txt> --merge <other.txt> [--offset x,y,z]");
    eprintln!("  ad2read <dupe.txt> --duplicate n,n,n [--offset x,y,z]");
    eprintln!("  ad2read <dupe.txt> --translate x,y,z        move the whole build");
    eprintln!("  ad2read <dupe.txt> --move <n> x,y,z         move one entity");
    eprintln!("  ad2read <dupe.txt> --rotate <n> p,y,r       set one entity's angle");
    eprintln!("  ad2read <dupe.txt> [--where K=V] --set K=V [--set K=V ...]");
}

/// A folder from the editor's config, when it is set: the same file
/// `ad2edit` writes, so `--models` and the build copy need no flag on a
/// machine that has been set up once.
fn configured_folder(pick: impl Fn(&ad2read::config::Config) -> String) -> Option<String> {
    match ad2read::config::load() {
        ad2read::config::LoadResult::Ok(c) => Some(pick(&c)).filter(|s| !s.trim().is_empty()),
        _ => None,
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    // `ad2read undo <file>`: swap the file with what it was before the last
    // verb that changed it, so a second undo is a redo.
    if args.get(1).map(|a| a == "undo").unwrap_or(false) {
        let Some(file) = args.get(2) else {
            eprintln!("undo needs a dupe: undo tank.txt");
            return;
        };
        let previous = ad2read::verbs::previous_path(file);
        let swapped = fs::read(&previous).and_then(|before| {
            let now = fs::read(file)?;
            fs::write(file, before)?;
            fs::write(&previous, now)
        });
        match swapped {
            Ok(()) => println!("{file} is back to what it was; undo again to redo"),
            Err(_) => eprintln!("nothing to undo for {file}: add, wire and fix are what it can take back"),
        }
        return;
    }
    // A verb that writes in place keeps what was there, for undo.
    if let Some(file) = ad2read::verbs::changes_in_place(&args) {
        if let Ok(before) = fs::read(file) {
            let _ = fs::write(ad2read::verbs::previous_path(file), before);
        }
    }
    // The eight verbs are the flags below under plainer names.
    let args = match ad2read::verbs::expand(&args) {
        Some(Ok(flags)) => flags,
        Some(Err(missing)) => {
            eprintln!("{missing}");
            return;
        }
        None => args,
    };

    // `ad2read new <file> [--baseplate | --preset <name>]`: a dupe from nothing.
    if args.get(1).map(|a| a == "--new").unwrap_or(false) {
        let Some(out) = args.get(2) else {
            eprintln!("new needs a file to create: new tank.txt [--baseplate | --preset light]");
            return;
        };
        if std::path::Path::new(out).exists() {
            eprintln!("{out} already exists; new never overwrites");
            return;
        }
        let preset = args.iter().position(|a| a == "--preset").and_then(|at| args.get(at + 1));
        let made = match preset {
            Some(name) => ad2read::buildfile::preset_dupe(name),
            None => {
                let mut dupe = ad2read::buildfile::empty_dupe();
                if args.iter().any(|a| a == "--baseplate") {
                    let size = [("Width", 96.0), ("Length", 200.0), ("Thickness", 2.0)];
                    match ad2read::catalog::find("GroundVehicle") {
                        Some(plate) => ad2read::build::spawn_catalog(&mut dupe, plate, (0.0, 0.0, 0.0), (0.0, 90.0, 0.0), &size).map(|_| ()),
                        None => Err("the catalogue has no ground vehicle baseplate".into()),
                    }
                    .map(|()| dupe)
                } else {
                    Ok(dupe)
                }
            }
        };
        let name = std::path::Path::new(out).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        // Like every other write: repair the head, then refuse what AdvDupe2
        // would refuse. A dupe with nothing in it yet has no head to check;
        // the first `add` repairs and validates it.
        let checked = made.and_then(|mut dupe| {
            if dupe.list_entities().is_empty() {
                return Ok(dupe);
            }
            for fix in ad2read::extras::repair_head(&mut dupe) {
                println!("repaired: {fix}");
            }
            let problems = ad2read::extras::validate(&dupe);
            if problems.is_empty() { Ok(dupe) } else { Err(format!("AdvDupe2 would refuse this file: {}", problems.join("; "))) }
        });
        match checked.and_then(|dupe| ad2read::buildfile::file_bytes(&dupe, &name).map(|bytes| (dupe, bytes))) {
            Ok((dupe, bytes)) => match fs::write(out, bytes) {
                Ok(()) => println!("wrote {out}: {} entities. Next: ad2read add {out} <role or item>", dupe.list_entities().len()),
                Err(e) => eprintln!("could not write {out}: {e}"),
            },
            Err(e) => eprintln!("new: {e}"),
        }
        return;
    }

    // `ad2read --gen-tank out.toml [...]`: write a parametric tank build file.
    if args.get(1).map(|a| a == "--gen-tank").unwrap_or(false) {
        let Some(out) = args.get(2) else {
            eprintln!("--gen-tank needs <out.toml> [--catalogue] [--parts dir] [--wheels n] [--engines n] [--suspension simple|locked|sprung] [--gun part] [--ammo part] [--width w] [--spacing s] [--output tank.txt]");
            return;
        };
        let mut spec = ad2read::gen::TankSpec::default();
        if let Some(pi) = args.iter().position(|a| a == "--preset") {
            match args.get(pi + 1).map(|s| s.as_str()) {
                Some(name) => match ad2read::gen::preset(name) {
                    Some(p) => spec = p,
                    None => {
                        eprintln!("unknown preset {name}; have: light, medium, heavy, spaa, stationary");
                        return;
                    }
                },
                None => {
                    eprintln!("--preset needs a name");
                    return;
                }
            }
        }
        let mut catalogue = false;
        let mut i = 3;
        while i < args.len() {
            let next = args.get(i + 1).cloned();
            match args[i].as_str() {
                "--catalogue" | "--catalog" => {
                    catalogue = true;
                    i += 1;
                    continue;
                }
                "--parts" => spec.parts = next.clone().unwrap_or(spec.parts.clone()),
                "--wheels" => spec.wheels_per_side = next.as_deref().and_then(|x| x.parse().ok()).unwrap_or(spec.wheels_per_side),
                "--engines" => spec.engines = next.as_deref().and_then(|x| x.parse().ok()).unwrap_or(spec.engines),
                "--suspension" => match next.as_deref().and_then(ad2read::gen::Suspension::parse) {
                    Some(su) => spec.suspension = su,
                    None => {
                        eprintln!("--suspension must be simple, locked or sprung");
                        return;
                    }
                },
                "--gun" => spec.gun = next.clone().unwrap_or(spec.gun.clone()),
                "--ammo" => spec.ammo = next.clone().unwrap_or(spec.ammo.clone()),
                "--engine" => spec.engine = next.clone().unwrap_or(spec.engine.clone()),
                "--width" => spec.track_width = next.as_deref().and_then(|x| x.parse().ok()).unwrap_or(spec.track_width),
                "--spacing" => spec.wheel_spacing = next.as_deref().and_then(|x| x.parse().ok()).unwrap_or(spec.wheel_spacing),
                "--output" => spec.output = next.clone().unwrap_or(spec.output.clone()),
                "--preset" | "--fit" => {}
                other => {
                    eprintln!("unknown --gen-tank option {other}");
                    return;
                }
            }
            i += 2;
        }
        if catalogue {
            spec = spec.catalogue();
        }
        if let Some(fi) = args.iter().position(|a| a == "--fit") {
            let Some(part) = args.get(fi + 1) else {
                eprintln!("--fit needs a baseplate part file");
                return;
            };
            let Some(plate) = open_dupe(part) else { return };
            match ad2read::pipeline::fit_spec_to_plate(&mut spec, &plate) {
                Ok((w, l)) => {
                    println!("fitted to a {w} × {l} plate: track {:.0}, spacing {:.0}", spec.track_width, spec.wheel_spacing);
                    if w <= 36.0 && l <= 36.0 {
                        println!("  that is a default-size plate; some builds anchor on a tiny plate and make the hull from armour; use --width/--spacing for those");
                    }
                }
                Err(e) => {
                    eprintln!("fit: {e}");
                    return;
                }
            }
        }
        let text = ad2read::gen::write_build_file(&spec);
        match fs::write(out, &text) {
            Ok(()) => println!("wrote {out}: {} wheels, {} engines, {:?} suspension → ad2read --build {out}", spec.wheels_per_side * 2, spec.engines, spec.suspension),
            Err(e) => eprintln!("could not write {out}: {e}"),
        }
        return;
    }

    // `ad2read --model-info <model> [--models <garrysmod>]`: the model's real
    // box, and which of the box's corners the mesh reaches. A plate reaches
    // all eight; a triangle six, and the two it misses say which way it points.
    if args.get(1).map(|a| a == "--model-info").unwrap_or(false) {
        let Some(model) = args.get(2) else {
            eprintln!("--model-info needs a model path");
            return;
        };
        let root = args.iter().position(|a| a == "--models").and_then(|at| args.get(at + 1).cloned()).or_else(|| configured_folder(|c| c.garrysmod.clone()));
        let Some(root) = root else {
            eprintln!("--model-info needs --models <garrysmod> (or the game folder set in ad2edit's Settings)");
            return;
        };
        let mut lib = ad2read::models::ModelLibrary::new(std::path::Path::new(&root));
        let (Some((bounds, _)), Some(mesh)) = (lib.bounds(model), lib.mesh(model)) else {
            println!("{model}: {}", lib.mesh_reason(model));
            return;
        };
        let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &mesh.positions {
            for axis in 0..3 {
                low[axis] = low[axis].min(p[axis]);
                high[axis] = high[axis].max(p[axis]);
            }
        }
        println!("{model}: {} triangles", mesh.triangles());
        println!("  bounds {:?} to {:?}", bounds.min, bounds.max);
        println!("  mesh   {low:?} to {high:?}");
        for corner in 0..8 {
            let at = [if corner & 1 == 0 { low[0] } else { high[0] }, if corner & 2 == 0 { low[1] } else { high[1] }, if corner & 4 == 0 { low[2] } else { high[2] }];
            let reached = mesh.positions.iter().any(|p| (0..3).all(|axis| (p[axis] - at[axis]).abs() < 0.26));
            println!("  corner x{} y{} z{}: {}", if corner & 1 == 0 { '-' } else { '+' }, if corner & 2 == 0 { '-' } else { '+' }, if corner & 4 == 0 { '-' } else { '+' }, if reached { "solid" } else { "empty" });
        }
        return;
    }

    // `ad2read --init <dir> [tank.toml]`: a project folder with project.toml.
    if args.get(1).map(|a| a == "--init").unwrap_or(false) {
        let Some(dir) = args.get(2) else {
            eprintln!("--init needs a folder");
            return;
        };
        let build = args.get(3).cloned().unwrap_or_else(|| "tank.toml".into());
        if let Err(e) = fs::create_dir_all(dir) {
            eprintln!("{dir}: {e}");
            return;
        }
        let path = std::path::Path::new(dir).join(ad2read::pipeline::PROJECT_FILE);
        if path.exists() {
            eprintln!("{} already exists", path.display());
            return;
        }
        match fs::write(&path, ad2read::pipeline::default_project(&build)) {
            Ok(()) => println!("wrote {}. Set parts/gmod/map/into, put {build} beside it; every --build there is recorded under builds/", path.display()),
            Err(e) => eprintln!("{}: {e}", path.display()),
        }
        return;
    }

    // `ad2read --compose out.toml a.toml b.toml [--prefix name]`
    if args.get(1).map(|a| a == "--compose").unwrap_or(false) {
        let (Some(out), Some(a), Some(b)) = (args.get(2), args.get(3), args.get(4)) else {
            eprintln!("--compose needs <out.toml> <a.toml> <b.toml> [--prefix name]");
            return;
        };
        let prefix = args.iter().position(|x| x == "--prefix").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "b".into());
        let (Ok(at), Ok(bt)) = (fs::read_to_string(a), fs::read_to_string(b)) else {
            eprintln!("could not read the inputs");
            return;
        };
        match ad2read::pipeline::compose(&at, &bt, &prefix) {
            Ok(text) => match fs::write(out, &text) {
                Ok(()) => println!("wrote {out}: {a} + {b} with {b}'s names prefixed {prefix}_; add the cross-links, then --build"),
                Err(e) => eprintln!("{out}: {e}"),
            },
            Err(e) => eprintln!("compose: {e}"),
        }
        return;
    }

    // `ad2read --watch tank.toml [--into <advdupe2 folder>]`: rebuild on
    // every change, doctor it, drop the dupe where AD2 reads it.
    if args.get(1).map(|a| a == "--watch").unwrap_or(false) {
        let Some(path) = args.get(2).cloned() else {
            eprintln!("--watch needs a .toml file [--into <folder>]");
            return;
        };
        let into = args.iter().position(|a| a == "--into").and_then(|i| args.get(i + 1)).cloned();
        let mut last: Option<std::time::SystemTime> = None;
        println!("watching {path}{}. Ctrl-C to stop", into.as_ref().map(|d| format!(", output copied to {d}")).unwrap_or_default());
        loop {
            let modified = fs::metadata(&path).and_then(|m| m.modified()).ok();
            if modified != last && modified.is_some() {
                last = modified;
                std::thread::sleep(std::time::Duration::from_millis(150));
                let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ad2read"));
                let out = std::process::Command::new(exe).args(["--build", &path]).output();
                match out {
                    Ok(o) => {
                        let text = String::from_utf8_lossy(&o.stdout);
                        let err = String::from_utf8_lossy(&o.stderr);
                        for l in text.lines().filter(|l| l.starts_with("wrote") || l.starts_with("fixed") || l.starts_with("clean")) {
                            println!("  {l}");
                        }
                        if !err.trim().is_empty() {
                            eprintln!("  {}", err.trim());
                        }
                        if let (Some(dir), Some(wrote)) = (&into, text.lines().find(|l| l.starts_with("wrote "))) {
                            let built = wrote.trim_start_matches("wrote ").split(" (").next().unwrap_or("");
                            let name = std::path::Path::new(built).file_name().map(|f| f.to_os_string()).unwrap_or_default();
                            let dest = std::path::Path::new(dir).join(name);
                            match fs::copy(built, &dest) {
                                Ok(_) => println!("  → {}", dest.display()),
                                Err(e) => eprintln!("  copy to {}: {e}", dest.display()),
                            }
                        }
                    }
                    Err(e) => eprintln!("build: {e}"),
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }

    // `ad2read --build tank.toml`: builds a dupe from nothing; no input file.
    if args.get(1).map(|a| a == "--build").unwrap_or(false) {
        let Some(path) = args.get(2) else {
            eprintln!("--build needs a .toml file");
            return;
        };
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{path}: {e}");
                return;
            }
        };
        let file: ad2read::buildfile::BuildFile = match toml::from_str(&text) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{path}: {e}");
                return;
            }
        };
        let base_dir = std::path::Path::new(path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let (mut dupe, report) = match ad2read::buildfile::run(&file, &base_dir) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("build failed: {e}");
                return;
            }
        };
        for l in &report.log {
            println!("{l}");
        }
        for fix in ad2read::extras::repair_head(&mut dupe) {
            println!("repaired: {fix}");
        }
        // A build is fixed and cleaned before it's written, unless asked not
        // to: the doctor is a stage, not a report.
        if !args.iter().any(|a| a == "--no-fix") {
            for line in ad2read::doctor::clean(&mut dupe) {
                println!("clean: {line}");
            }
            let findings = ad2read::doctor::doctor(&dupe);
            for d in ad2read::doctor::apply_fixes(&mut dupe, &findings) {
                println!("fixed: {d}");
            }
        }
        let problems = ad2read::extras::validate(&dupe);
        if !problems.is_empty() {
            eprintln!("refusing to write; AdvDupe2 would reject this file:");
            for p in problems {
                eprintln!("  {p}");
            }
            return;
        }
        let dangling = refs::dangling(&dupe);
        if !dangling.is_empty() {
            eprintln!("refusing to write; {} dangling reference(s)", dangling.len());
            return;
        }
        let body = match dupe.to_body() {
            Ok(b) => b,
            Err(e) => {
                eprintln!("encode: {e}");
                return;
            }
        };
        let compressed = match dupefile::compress(&body, 3, 0, 2, 65536, true) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("compress: {e}");
                return;
            }
        };
        let name = std::path::Path::new(&file.output).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "build".into());
        let size = body.len().to_string();
        let info: Vec<(Vec<u8>, Vec<u8>)> = [("name", name.as_str()), ("date", "built"), ("time", ""), ("timezone", ""), ("size", size.as_str()), ("check", "\r\n\t\n")]
            .iter().map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec())).collect();
        let out = if std::path::Path::new(&file.output).is_absolute() { PathBuf::from(&file.output) } else { base_dir.join(&file.output) };
        match fs::write(&out, dupefile::build(5, &info, &compressed)) {
            Ok(()) => {
                let findings = ad2read::doctor::doctor(&dupe);
                let warns = findings.iter().filter(|f| f.severity != ad2read::doctor::Severity::Note).count();
                println!("wrote {} ({} entities, {} names; doctor: {} warning(s), run --doctor for the list)", out.display(), dupe.list_entities().len(), report.names.len(), warns);
                // A project beside the build file records it and copies it
                // in; without one, the editor's config says where AdvDupe2
                // looks.
                let project = ad2read::pipeline::project_for(std::path::Path::new(path));
                if let Some((pdir, project)) = &project {
                    if project.history {
                        match ad2read::pipeline::record_build(pdir, &out, &findings) {
                            Ok(dest) => println!("recorded {}", dest.display()),
                            Err(e) => eprintln!("history: {e}"),
                        }
                    }
                }
                let into = project
                    .and_then(|(_, p)| p.into)
                    .or_else(|| configured_folder(|c| c.advdupe2.clone()));
                if let Some(into) = into {
                    let dest = std::path::Path::new(&into).join(out.file_name().unwrap_or_default());
                    match fs::copy(&out, &dest) {
                        Ok(_) => println!("→ {}", dest.display()),
                        Err(e) => eprintln!("copy to {}: {e}", dest.display()),
                    }
                }
            }
            Err(e) => eprintln!("could not write {}: {e}", out.display()),
        }
        return;
    }
    let Some(path) = args.get(1) else {
        usage();
        return;
    };

    let mut list_entities = false;
    let mut scan_refs = false;
    let mut dump_mods = false;
    let mut dump_cons = false;
    let mut dump_models: Option<String> = None;
    let mut diff_with: Option<String> = None;
    let mut weld_world: Option<f64> = None;
    // Building commands. Several may be given in one invocation and they run
    // in the order listed here, so a script can spawn, then constrain, then
    // link, then save once.
    let mut spawns: Vec<(String, f64, Vec3, Option<Vec3>)> = Vec::new();
    let mut axes: Vec<(f64, f64, Vec3, Vec3, f64, f64, f64, bool)> = Vec::new();
    let mut welds: Vec<(f64, f64, f64, bool)> = Vec::new();
    let mut nocollides: Vec<(f64, f64)> = Vec::new();
    let mut ballsockets: Vec<(f64, f64, Vec3)> = Vec::new();
    let mut links: Vec<(f64, f64)> = Vec::new();
    let mut chip_export: Option<(f64, String)> = None;
    let mut chip_import: Option<(f64, String)> = None;
    let mut save_to: Option<String> = None;
    let mut full = false;
    let mut models_root: Option<String> = configured_folder(|c| c.garrysmod.clone());
    let mut align_axles: Vec<(f64, Vec3)> = Vec::new();
    let mut wheel_axes: Vec<(f64, f64, Vec3, bool)> = Vec::new();
    let mut show: Option<f64> = None;
    let mut pair_locks: Vec<(f64, f64)> = Vec::new();
    let mut sprung: Vec<(f64, f64)> = Vec::new();
    let mut hydraulic: Vec<(f64, f64, f64)> = Vec::new();
    let mut wires: Vec<(f64, String, f64, String)> = Vec::new();
    let mut ammo: Vec<(f64, String, Option<Vec3>)> = Vec::new();
    let mut parts_dir: Option<String> = None;
    let mut spawn_parts: Vec<(String, Vec3, Option<Vec3>)> = Vec::new();
    let mut extract_to: Option<(f64, String)> = None;
    let mut json_out = false;
    let mut doctor = false;
    let mut fix = false;
    let mut clean = false;
    let mut armours: Vec<(Vec<f64>, f64, f64)> = Vec::new();
    let mut armour_plan: Option<f64> = None;
    let mut armour_apply: Option<(f64, f64)> = None;
    let mut explain: Option<f64> = None;
    let mut versus: Option<(f64, Vec<f64>)> = None;
    let mut catalog: Option<String> = None;
    let mut model_check = false;
    let mut render_to: Option<(String, f64, f64)> = None;
    let mut catalogue_check = false;
    let mut send = false;
    let mut scale_check = false;
    let mut mass_report: Option<bool> = None;
    let mut adds: Vec<(String, Vec3, Option<Vec3>, Vec<(String, f64)>)> = Vec::new();
    let mut sloped_hull: Option<(f64, f64, f64)> = None;
    let mut extract_all: Option<String> = None;
    let mut track_links: Vec<(f64, f64, Vec<f64>)> = Vec::new();
    let mut doors: Vec<(f64, f64, String)> = Vec::new();
    let mut submats: Vec<(f64, u32, String)> = Vec::new();
    let mut keypads: Vec<(f64, Option<f64>, f64)> = Vec::new();
    let mut resizes: Vec<(f64, Vec3, u32)> = Vec::new();
    let mut sphericals: Vec<(f64, Option<f64>)> = Vec::new();
    let mut rot_locks: Vec<(f64, f64, build::Lock)> = Vec::new();
    let mut parents: Vec<(f64, f64)> = Vec::new();
    let mut puts: Vec<(f64, String, String)> = Vec::new();
    let mut unmods: Vec<(f64, String)> = Vec::new();
    let mut delete: Option<f64> = None;
    let mut remap: Option<(f64, f64)> = None;
    let mut merge_with: Option<String> = None;
    let mut duplicate_set: Option<Vec<f64>> = None;
    let mut offset: Vec3 = (0.0, 0.0, 0.0);
    let mut translate: Option<Vec3> = None;
    let mut move_ent: Option<(f64, Vec3)> = None;
    let mut rotate_ent: Option<(f64, Vec3)> = None;
    let mut filter: Option<(String, String)> = None;
    let mut sets: Vec<(String, String)> = Vec::new();

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--list-entities" => list_entities = true,
            "--scan-refs" => scan_refs = true,
            "--dump-mods" => dump_mods = true,
            "--dump-constraints" => dump_cons = true,
            "--diff" => {
                let Some(p) = args.get(i + 1) else {
                    eprintln!("--diff needs another dupe to compare against");
                    return;
                };
                diff_with = Some(p.clone());
                i += 1;
            }
            "--weld-world" => {
                let Some(n) = args.get(i + 1).and_then(|x| x.parse().ok()) else {
                    eprintln!("--weld-world needs an entity index");
                    return;
                };
                weld_world = Some(n);
                i += 1;
            }
            "--spawn" => {
                // --spawn <template.txt> <index> <x,y,z> [p,y,r]
                let (Some(t), Some(n), Some(at)) = (
                    args.get(i + 1),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 3).and_then(|x| parse_triple(x)),
                ) else {
                    eprintln!("--spawn needs <template.txt> <index> <x,y,z> [p,y,r]");
                    return;
                };
                let ang = args.get(i + 4).and_then(|x| parse_triple(x));
                spawns.push((t.clone(), n, at, ang));
                i += if ang.is_some() { 4 } else { 3 };
            }
            "--axis" => {
                // --axis <a> <b> <x,y,z> <dx,dy,dz> [friction] [forcelimit] [torquelimit] [nocollide]
                let (Some(a), Some(b), Some(at), Some(dir)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 3).and_then(|x| parse_triple(x)),
                    args.get(i + 4).and_then(|x| parse_triple(x)),
                ) else {
                    eprintln!("--axis needs <a> <b> <x,y,z> <dx,dy,dz> [friction] [forcelimit] [torquelimit] [nocollide]");
                    return;
                };
                let mut used = 4;
                let mut nums = Vec::new();
                let mut nocollide = false;
                for k in 5..=8 {
                    match args.get(i + k) {
                        Some(x) if x == "nocollide" => {
                            nocollide = true;
                            used = k;
                        }
                        Some(x) => match x.parse::<f64>() {
                            Ok(v) if !x.starts_with("--") => {
                                nums.push(v);
                                used = k;
                            }
                            _ => break,
                        },
                        None => break,
                    }
                }
                let g = |k: usize| nums.get(k).copied().unwrap_or(0.0);
                axes.push((a, b, at, dir, g(0), g(1), g(2), nocollide));
                i += used;
            }
            "--weld" => {
                let (Some(a), Some(b)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                ) else {
                    eprintln!("--weld needs <a> <b> [forcelimit] [nocollide]");
                    return;
                };
                let mut used = 2;
                let mut forcelimit = 0.0;
                let mut nocollide = false;
                for k in 3..=4 {
                    match args.get(i + k) {
                        Some(x) if x == "nocollide" => {
                            nocollide = true;
                            used = k;
                        }
                        Some(x) if !x.starts_with("--") => {
                            if let Ok(v) = x.parse::<f64>() {
                                forcelimit = v;
                                used = k;
                            }
                        }
                        _ => break,
                    }
                }
                welds.push((a, b, forcelimit, nocollide));
                i += used;
            }
            "--nocollide" => {
                let (Some(a), Some(b)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                ) else {
                    eprintln!("--nocollide needs <a> <b>");
                    return;
                };
                nocollides.push((a, b));
                i += 2;
            }
            "--ballsocket" => {
                let (Some(a), Some(b), Some(at)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 3).and_then(|x| parse_triple(x)),
                ) else {
                    eprintln!("--ballsocket needs <a> <b> <x,y,z>");
                    return;
                };
                ballsockets.push((a, b, at));
                i += 3;
            }
            "--link" => {
                let (Some(a), Some(b)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                ) else {
                    eprintln!("--link needs <a> <b>");
                    return;
                };
                links.push((a, b));
                i += 2;
            }
            "--chip-export" => {
                let (Some(n), Some(f)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2),
                ) else {
                    eprintln!("--chip-export needs <index> <file>");
                    return;
                };
                chip_export = Some((n, f.clone()));
                i += 2;
            }
            "--chip-import" => {
                let (Some(n), Some(f)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2),
                ) else {
                    eprintln!("--chip-import needs <index> <file>");
                    return;
                };
                chip_import = Some((n, f.clone()));
                i += 2;
            }
            "--wheel-axis" => {
                // --wheel-axis <wheel> <base> <ax,ay,az in the wheel's frame>
                let (Some(w), Some(b), Some(ax)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 3).and_then(|x| parse_triple(x)),
                ) else {
                    eprintln!("--wheel-axis needs <wheel> <base> <ax,ay,az>");
                    return;
                };
                // Optional trailing "nocollide"; the default collides (ironlock).
                let nc = args.get(i + 4).map(|x| x == "nocollide").unwrap_or(false);
                wheel_axes.push((w, b, ax, nc));
                i += if nc { 4 } else { 3 };
            }
            "--lock-pair" => {
                let (Some(a), Some(b)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                ) else {
                    eprintln!("--lock-pair needs <a> <b>");
                    return;
                };
                pair_locks.push((a, b));
                i += 2;
            }
            "--sprung-wheel" => {
                let (Some(w), Some(b)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                ) else {
                    eprintln!("--sprung-wheel needs <wheel> <base>");
                    return;
                };
                sprung.push((w, b));
                i += 2;
            }
            "--hydraulic-wheel" => {
                let (Some(w), Some(b), Some(c)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 3).and_then(|x| x.parse::<f64>().ok()),
                ) else {
                    eprintln!("--hydraulic-wheel needs <wheel> <base> <controller>");
                    return;
                };
                hydraulic.push((w, b, c));
                i += 3;
            }
            "--wire" => {
                let (Some(src), Some(sp), Some(dst), Some(dp)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2),
                    args.get(i + 3).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 4),
                ) else {
                    eprintln!("--wire needs <src> <output> <dst> <input>");
                    return;
                };
                wires.push((src, sp.clone(), dst, dp.clone()));
                i += 4;
            }
            "--ammo" => {
                let (Some(n), Some(t)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2),
                ) else {
                    eprintln!("--ammo needs <crate> <AmmoType> [x,y,z]");
                    return;
                };
                let size = args.get(i + 3).and_then(|x| parse_triple(x));
                ammo.push((n, t.clone(), size));
                i += if size.is_some() { 3 } else { 2 };
            }
            "--parts" => {
                let Some(dir) = args.get(i + 1) else {
                    eprintln!("--parts needs a folder of single-entity dupes");
                    return;
                };
                parts_dir = Some(dir.clone());
                i += 1;
            }
            "--spawn-part" => {
                // --spawn-part <name> <x,y,z> [p,y,r]: <parts>/<name>.txt, its first entity
                let (Some(name), Some(at)) = (args.get(i + 1), args.get(i + 2).and_then(|x| parse_triple(x))) else {
                    eprintln!("--spawn-part needs <name> <x,y,z> [p,y,r]");
                    return;
                };
                let ang = args.get(i + 3).and_then(|x| parse_triple(x));
                spawn_parts.push((name.clone(), at, ang));
                i += if ang.is_some() { 3 } else { 2 };
            }
            "--extract" => {
                // --extract <n> <file>: one entity as its own dupe, links shed
                let (Some(n), Some(f)) = (args.get(i + 1).and_then(|x| x.parse::<f64>().ok()), args.get(i + 2)) else {
                    eprintln!("--extract needs <n> <file>");
                    return;
                };
                extract_to = Some((n, f.clone()));
                i += 2;
            }
            "--json" => json_out = true,
            "--doctor" => doctor = true,
            "--fix" => { doctor = true; fix = true; }
            "--clean" => clean = true,
            "--armour" | "--armor" => {
                // --armour <mm> [ductility] <n,n,..|props>
                let Some(mm) = args.get(i + 1).and_then(|x| x.parse::<f64>().ok()) else {
                    eprintln!("--armour needs <mm> [ductility] <n,n,..|props>");
                    return;
                };
                let mut used = 1;
                let mut duct = 0.0;
                let mut list: Option<String> = None;
                for k in 2..=3 {
                    match args.get(i + k) {
                        Some(x) if x == "props" || x.contains(',') || x.parse::<f64>().is_ok() && k == 3 => { list = Some(x.clone()); used = k; break; }
                        Some(x) if k == 2 => match x.parse::<f64>() { Ok(v) => { duct = v; used = k; } Err(_) => { list = Some(x.clone()); used = k; break; } },
                        _ => break,
                    }
                }
                let Some(list) = list else {
                    eprintln!("--armour needs a target: entity indices (n,n,..) or props");
                    return;
                };
                let targets: Vec<f64> = if list == "props" { Vec::new() } else { list.split(',').filter_map(|x| x.trim().parse::<f64>().ok()).collect() };
                armours.push((targets, mm, duct));
                i += used;
            }
            "--model-check" => model_check = true,
            "--catalogue-check" | "--catalog-check" => catalogue_check = true,
            "--send" => send = true,
            "--scale-check" => scale_check = true,
            "--mass" | "--mass-report" => mass_report = Some(false),
            "--mass-check" => mass_report = Some(true),
            "--render" => {
                // --render <out.png> [yaw pitch]: degrees round the dupe's middle.
                let Some(out) = args.get(i + 1).cloned() else {
                    eprintln!("--render needs an output .png");
                    return;
                };
                let angle = |n: usize| args.get(i + n).and_then(|a| a.parse::<f64>().ok());
                let (yaw, pitch) = match (angle(2), angle(3)) {
                    (Some(yaw), Some(pitch)) => {
                        i += 2;
                        (yaw, pitch)
                    }
                    _ => (-35.0, 25.0),
                };
                render_to = Some((out, yaw, pitch));
                i += 1;
            }
            "--catalog" | "--catalogue" => {
                // --catalog [category prefix]
                let filt = args.get(i + 1).filter(|x| !x.starts_with("--")).cloned();
                if filt.is_some() { i += 1; }
                catalog = Some(filt.unwrap_or_default());
            }
            "--hull" => {
                // --hull [height] [glacis degrees] [mm]: a sloped SProps hull on the baseplate.
                let number = |n: usize, default: f64| args.get(i + n).and_then(|x| x.parse::<f64>().ok()).unwrap_or(default);
                let given = (1..=3).take_while(|n| args.get(i + n).is_some_and(|x| x.parse::<f64>().is_ok())).count();
                sloped_hull = Some((number(1, 36.0), number(2, 60.0), number(3, 20.0)));
                i += given;
            }
            "--add" => {
                // --add <item id> <x,y,z> [p,y,r] [Key=value ...]   e.g. --add C 0,60,50 0,90,0 Caliber=100
                let (Some(id), Some(at)) = (args.get(i + 1), args.get(i + 2).and_then(|x| parse_triple(x))) else {
                    eprintln!("--add needs <item> <x,y,z> [p,y,r] [Key=value ...]; see --catalog");
                    return;
                };
                let mut used = 2;
                let ang = args.get(i + 3).and_then(|x| if x.contains('=') { None } else { parse_triple(x) });
                if ang.is_some() { used = 3; }
                let mut nums = Vec::new();
                while let Some(kv) = args.get(i + used + 1) {
                    let Some((k, v)) = kv.split_once('=') else { break };
                    let Ok(v) = v.parse::<f64>() else { break };
                    nums.push((k.to_owned(), v));
                    used += 1;
                }
                adds.push((id.clone(), at, ang, nums));
                i += used;
            }
            "--explain" => {
                let Some(n) = args.get(i + 1).and_then(|x| x.parse::<f64>().ok()) else {
                    eprintln!("--explain needs an entity index");
                    return;
                };
                explain = Some(n);
                json_out = true;
                i += 1;
            }
            "--versus" => {
                // --versus <penetration mm> [angle,angle,...]
                let Some(pen) = args.get(i + 1).and_then(|x| x.parse::<f64>().ok()) else {
                    eprintln!("--versus needs <penetration mm> [angles]");
                    return;
                };
                let angles: Vec<f64> = args.get(i + 2).filter(|x| !x.starts_with("--")).map(|x| x.split(',').filter_map(|a| a.trim().parse().ok()).collect()).unwrap_or_else(|| vec![0.0, 30.0, 60.0]);
                let used = if args.get(i + 2).map(|x| !x.starts_with("--")).unwrap_or(false) { 2 } else { 1 };
                versus = Some((pen, angles));
                i += used;
            }
            "--armour-apply" | "--armor-apply" => {
                // --armour-apply <tonnes> [front bias]
                let Some(t) = args.get(i + 1).and_then(|x| x.parse::<f64>().ok()) else {
                    eprintln!("--armour-apply needs <tonnes> [front bias, default 1.6]");
                    return;
                };
                let bias = args.get(i + 2).filter(|x| !x.starts_with("--")).and_then(|x| x.parse::<f64>().ok());
                armour_apply = Some((t, bias.unwrap_or(1.6)));
                i += if bias.is_some() { 2 } else { 1 };
            }
            "--armour-plan" | "--armor-plan" => {
                let target = args.get(i + 1).and_then(|x| if x.starts_with("--") { None } else { x.parse::<f64>().ok() });
                armour_plan = Some(target.unwrap_or(ad2read::rules::SERVER_MASS_LIMIT_KG / 1000.0));
                if target.is_some() { i += 1; }
            }
            "--extract-all" => {
                let Some(dir) = args.get(i + 1) else {
                    eprintln!("--extract-all needs a folder");
                    return;
                };
                extract_all = Some(dir.clone());
                i += 1;
            }
            "--fading-door" => {
                // --fading-door <n> <numpad key> [material]
                let (Some(n), Some(k)) = (args.get(i + 1).and_then(|x| x.parse::<f64>().ok()), args.get(i + 2).and_then(|x| x.parse::<f64>().ok())) else {
                    eprintln!("--fading-door needs <n> <key> [material]");
                    return;
                };
                let mat = args.get(i + 3).filter(|x| !x.starts_with("--")).cloned();
                doors.push((n, k, mat.clone().unwrap_or_else(|| "sprites/heatwave".into())));
                i += if mat.is_some() { 3 } else { 2 };
            }
            "--submaterial" => {
                let (Some(n), Some(slot), Some(m)) = (args.get(i + 1).and_then(|x| x.parse::<f64>().ok()), args.get(i + 2).and_then(|x| x.parse::<u32>().ok()), args.get(i + 3)) else {
                    eprintln!("--submaterial needs <n> <slot> <material>");
                    return;
                };
                submats.push((n, slot, m.clone()));
                i += 3;
            }
            "--keypad" => {
                // --keypad <n> <password|none> [key granted]
                let (Some(n), Some(pw)) = (args.get(i + 1).and_then(|x| x.parse::<f64>().ok()), args.get(i + 2)) else {
                    eprintln!("--keypad needs <n> <password|none> [key]");
                    return;
                };
                let password = if pw == "none" { None } else { pw.parse::<f64>().ok() };
                let key = args.get(i + 3).and_then(|x| if x.starts_with("--") { None } else { x.parse::<f64>().ok() });
                keypads.push((n, password, key.unwrap_or(111.0)));
                i += if key.is_some() { 3 } else { 2 };
            }
            "--resize" => {
                // --resize <n> <sx,sy,sz> [bones]
                let (Some(n), Some(sc)) = (args.get(i + 1).and_then(|x| x.parse::<f64>().ok()), args.get(i + 2).and_then(|x| parse_triple(x))) else {
                    eprintln!("--resize needs <n> <sx,sy,sz> [bones]");
                    return;
                };
                let bones = args.get(i + 3).and_then(|x| if x.starts_with("--") { None } else { x.parse::<u32>().ok() });
                resizes.push((n, sc, bones.unwrap_or(1)));
                i += if bones.is_some() { 3 } else { 2 };
            }
            "--track-link" => {
                // --track-link <track> <chassis> <w1,w2,...> in track order
                let (Some(t), Some(c), Some(ws)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 3),
                ) else {
                    eprintln!("--track-link needs <track> <chassis> <w1,w2,...>");
                    return;
                };
                let wheels: Option<Vec<f64>> = ws.split(',').map(|x| x.trim().parse::<f64>().ok()).collect();
                let Some(wheels) = wheels else {
                    eprintln!("--track-link wheel list must be numbers");
                    return;
                };
                track_links.push((t, c, wheels));
                i += 3;
            }
            "--spherical" => {
                // --spherical <n> [radius]; radius from model bounds with --models
                let Some(n) = args.get(i + 1).and_then(|x| x.parse::<f64>().ok()) else {
                    eprintln!("--spherical needs <n> [radius]");
                    return;
                };
                let r = args.get(i + 2).and_then(|x| if x.starts_with("--") { None } else { x.parse::<f64>().ok() });
                sphericals.push((n, r));
                i += if r.is_some() { 2 } else { 1 };
            }
            "--show" => {
                // --show <n>: one entity, every key, no truncation.
                let Some(n) = args.get(i + 1).and_then(|x| x.parse::<f64>().ok()) else {
                    eprintln!("--show needs an entity index");
                    return;
                };
                show = Some(n);
                i += 1;
            }
            "--lock" => {
                // --lock <a> <b> <x|y|z>: orientation-only socket pinning one axis
                let (Some(a), Some(b), Some(ax)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 3),
                ) else {
                    eprintln!("--lock needs <a> <b> <x|y|z>");
                    return;
                };
                let lock = match ax.as_str() {
                    "x" | "X" => build::Lock::X,
                    "y" | "Y" => build::Lock::Y,
                    "z" | "Z" => build::Lock::Z,
                    other => {
                        eprintln!("--lock axis must be x, y or z, not {other}");
                        return;
                    }
                };
                rot_locks.push((a, b, lock));
                i += 3;
            }
            "--put" => {
                // --put <n> <key> <value>: create-or-set a top-level field,
                // typed from the text (number, true/false, else string).
                let (Some(n), Some(k), Some(v)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2),
                    args.get(i + 3),
                ) else {
                    eprintln!("--put needs <n> <key> <value>");
                    return;
                };
                puts.push((n, k.clone(), v.clone()));
                i += 3;
            }
            "--unmod" => {
                let (Some(n), Some(m)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2),
                ) else {
                    eprintln!("--unmod needs <n> <modifier>");
                    return;
                };
                unmods.push((n, m.clone()));
                i += 2;
            }
            "--parent" => {
                let (Some(c), Some(p)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| x.parse::<f64>().ok()),
                ) else {
                    eprintln!("--parent needs <child> <parent>");
                    return;
                };
                parents.push((c, p));
                i += 2;
            }
            "--models" => {
                let Some(p) = args.get(i + 1) else {
                    eprintln!("--models needs the garrysmod folder");
                    return;
                };
                models_root = Some(p.clone());
                i += 1;
            }
            "--align-axle" => {
                // --align-axle <n> <dx,dy,dz>: turn entity n so its model's
                // thinnest axis (a tire's axle) points along the direction.
                let (Some(n), Some(dir)) = (
                    args.get(i + 1).and_then(|x| x.parse::<f64>().ok()),
                    args.get(i + 2).and_then(|x| parse_triple(x)),
                ) else {
                    eprintln!("--align-axle needs <n> <dx,dy,dz> (and --models <garrysmod>)");
                    return;
                };
                align_axles.push((n, dir));
                i += 2;
            }
            "--full" => full = true,
            "--out" => {
                let Some(p) = args.get(i + 1) else {
                    eprintln!("--out needs a path");
                    return;
                };
                save_to = Some(p.clone());
                i += 1;
            }
            "--dump-models" => {
                let Some(p) = args.get(i + 1) else {
                    eprintln!("--dump-models needs the path to your garrysmod folder");
                    return;
                };
                dump_models = Some(p.clone());
                i += 1;
            }
            "--delete-entity" => {
                let Some(v) = args.get(i + 1).and_then(|s| s.parse::<f64>().ok()) else {
                    eprintln!("--delete-entity needs a number");
                    return;
                };
                delete = Some(v);
                i += 1;
            }
            "--remap" => {
                let a = args.get(i + 1).and_then(|s| s.parse::<f64>().ok());
                let b = args.get(i + 2).and_then(|s| s.parse::<f64>().ok());
                let (Some(a), Some(b)) = (a, b) else {
                    eprintln!("--remap needs two numbers: <old> <new>");
                    return;
                };
                remap = Some((a, b));
                i += 2;
            }
            "--merge" => {
                let Some(p) = args.get(i + 1) else {
                    eprintln!("--merge needs a path to another dupe");
                    return;
                };
                merge_with = Some(p.clone());
                i += 1;
            }
            "--duplicate" => {
                let Some(v) = args.get(i + 1).and_then(|s| parse_list(s)) else {
                    eprintln!("--duplicate needs one or more entity indices, comma separated");
                    return;
                };
                duplicate_set = Some(v);
                i += 1;
            }
            "--offset" => {
                let Some(v) = args.get(i + 1).and_then(|s| parse_triple(s)) else {
                    eprintln!("--offset needs three numbers: x,y,z");
                    return;
                };
                offset = v;
                i += 1;
            }
            "--translate" => {
                let Some(v) = args.get(i + 1).and_then(|s| parse_triple(s)) else {
                    eprintln!("--translate needs three numbers: x,y,z");
                    return;
                };
                translate = Some(v);
                i += 1;
            }
            "--move" => {
                let n = args.get(i + 1).and_then(|s| s.parse::<f64>().ok());
                let v = args.get(i + 2).and_then(|s| parse_triple(s));
                let (Some(n), Some(v)) = (n, v) else {
                    eprintln!("--move needs an entity index then x,y,z");
                    return;
                };
                move_ent = Some((n, v));
                i += 2;
            }
            "--rotate" => {
                let n = args.get(i + 1).and_then(|s| s.parse::<f64>().ok());
                let v = args.get(i + 2).and_then(|s| parse_triple(s));
                let (Some(n), Some(v)) = (n, v) else {
                    eprintln!("--rotate needs an entity index then pitch,yaw,roll");
                    return;
                };
                rotate_ent = Some((n, v));
                i += 2;
            }
            "--where" => {
                let Some(p) = args.get(i + 1).and_then(|s| parse_pair(s)) else {
                    eprintln!("--where needs Key=Value");
                    return;
                };
                filter = Some(p);
                i += 1;
            }
            "--set" => {
                let Some(p) = args.get(i + 1).and_then(|s| parse_pair(s)) else {
                    eprintln!("--set needs Key=Value");
                    return;
                };
                sets.push(p);
                i += 1;
            }
            other => {
                eprintln!("unknown option {other}");
                usage();
                return;
            }
        }
        i += 1;
    }

    let loaded = match load(path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    let Loaded { file, raw, mut dupe, lzma, lzma_text } = loaded;
    let (lc, lp, pb, dict, real_size) = lzma;
    if !json_out {

        println!("{}, revision {}", path, file.revision);
        for (k, v) in &file.info {
            println!("  {} = {}", escape(k), escape(v));
        }
    }
    if !json_out {
    println!("  lzma: {lzma_text}");
    }
    if !json_out {
    println!("\n{} bytes decoded into {} tables", raw.len(), dupe.arena.len());
    }

    // Round trip before touching anything. If this fails, no edit below can
    // be trusted, so stop. A new (empty) file has nothing to round-trip.
    match if raw.is_empty() { Ok(Vec::new()) } else { dupe.to_body() } {
        Err(e) => {
            eprintln!("re-encode failed: {e}");
            return;
        }
        Ok(out) => {
            if out == raw {
                if !json_out { println!("round trip: byte-identical"); }
            } else {
                println!(
                    "round trip: MISMATCH, original {}, re-encoded {}",
                    raw.len(),
                    out.len()
                );
                let n = raw.len().min(out.len());
                if let Some(at) = (0..n).find(|&i| raw[i] != out[i]) {
                    let from = at.saturating_sub(8);
                    println!("  first difference at offset {at}");
                    println!("  original:   {}", hexline(&raw[from..(at + 8).min(raw.len())]));
                    println!("  re-encoded: {}", hexline(&out[from..(at + 8).min(out.len())]));
                }
                return;
            }
        }
    }

    if !json_out {
        println!(
            "entities: {}  constraints: {}",
            dupe.list_entities().len(),
            constraint_count(&dupe)
        );
    }

    // Report anything already broken, so a warning after the edit reads as
    // "this edit did that" rather than "this file arrived like this".
    let before = refs::dangling(&dupe);
    if !before.is_empty() {
        // In the JSON modes stdout carries nothing but the JSON.
        let mut note = format!("\nNOTE: {} dangling reference(s) already in this file:", before.len());
        for h in &before {
            note.push_str(&format!("\n  {} = {}", h.path, h.value));
        }
        if json_out {
            eprintln!("{note}");
        } else {
            println!("{note}");
        }
    }

    if list_entities {
        println!("\n--- entities ---");
        for (index, class, model) in dupe.list_entities() {
            println!("  {index:>6}  {class}  {model}");
        }
        return;
    }

    if scan_refs {
        report_refs(&dupe);
        return;
    }

    if dump_mods {
        report_mods(&dupe);
        return;
    }

    if dump_cons {
        report_constraints(&dupe);
        return;
    }

    if let Some(root) = dump_models {
        report_models(&dupe, &root);
        return;
    }

    if let Some(n) = show {
        match transform::entity_table(&dupe, n) {
            Some(et) => {
                let mut seen = vec![false; dupe.arena.len()];
                dump(&ad2read::value::Value::Table(et), &dupe.arena, 0, 64, usize::MAX, &mut seen);
            }
            None => eprintln!("no entity {n}"),
        }
        return;
    }

    if json_out && !doctor && explain.is_none() && versus.is_none() {
        println!("{}", serde_json::to_string_pretty(&ad2read::json::dupe_to_json(&dupe)).unwrap_or_default());
        return;
    }
    if doctor && !fix {
        let mut lib = models_root.as_ref().map(|r| ad2read::models::ModelLibrary::new(std::path::Path::new(r)));
        let findings = ad2read::doctor::doctor_with(&dupe, lib.as_mut());
        if json_out {
            let arr: Vec<serde_json::Value> = findings.iter().map(|f| serde_json::json!({
                "severity": format!("{:?}", f.severity).to_lowercase(),
                "entity": f.entity,
                "what": f.what,
            })).collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
        } else {
            // Per-entity notes that repeat (runtime keys, hidden parts) are
            // said once with a count; everything else in full.
            let collapsible = |f: &ad2read::doctor::Finding| f.severity == ad2read::doctor::Severity::Note && (f.what.starts_with("runtime key ") || f.what.starts_with("invisible"));
            let mut groups: std::collections::BTreeMap<String, usize> = Default::default();
            let mut shown = 0;
            for f in &findings {
                if collapsible(f) { *groups.entry(f.what.clone()).or_default() += 1; } else { shown += 1; }
            }
            println!("\n--- doctor: {} finding(s) ---", shown + groups.len());
            for f in &findings {
                if collapsible(f) { continue; }
                let tag = match f.severity { ad2read::doctor::Severity::Error => "ERROR", ad2read::doctor::Severity::Warn => "warn ", ad2read::doctor::Severity::Note => "note " };
                match f.entity {
                    Some(e) => println!("  {tag} [{e}] {}", f.what),
                    None => println!("  {tag} {}", f.what),
                }
            }
            for (k, n) in groups {
                println!("  note  {k}: {n} entities{}", if k.starts_with("runtime key") { " (--clean removes)" } else { "" });
            }
            if findings.is_empty() {
                println!("  nothing to report");
            }
        }
        return;
    }
    if let Some(target_t) = armour_plan {
        let mut lib = models_root.as_ref().map(|r| ad2read::models::ModelLibrary::new(std::path::Path::new(r)));
        let mut plates: Vec<(f64, String, f64, f64, f64)> = Vec::new(); // index, model, thickness, ductility, kg
        let mut other = 0.0;
        for (index, class, model) in dupe.list_entities() {
            let Some(et) = transform::entity_table(&dupe, index) else { continue };
            let model = model.trim_matches('"').to_owned();
            let mods = dupe.get_table(et, "EntityMods");
            let mass = mods.and_then(|m| dupe.get_table(m, "mass")).and_then(|m| dupe.get_number(m, "Mass"));
            let armour = mods.and_then(|m| dupe.get_table(m, "ACF_Armor")).map(|a| (dupe.get_number(a, "Thickness").unwrap_or(0.0), dupe.get_number(a, "Ductility").unwrap_or(0.0)));
            if let Some(kg) = mass { other += kg; continue; }
            if let Some((t, d)) = armour {
                let half = lib.as_mut().and_then(|l| l.bounds(&model)).map(|(b, _)| b.half_extents()).or_else(|| build::sprops_half_extents(&model));
                if let Some(h) = half {
                    plates.push((index, model.rsplit('/').next().unwrap_or(&model).to_owned(), t, d, build::plate_mass(h, t, d)));
                }
            } else if class.contains("acf_") {
                other += 0.0;
            }
        }
        let armour_total: f64 = plates.iter().map(|p| p.4).sum();
        let acf_est = 7500.0;
        let total = armour_total + other + acf_est;
        println!("\n--- armour plan: target {target_t:.1} t ---");
        plates.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap());
        for (i, m, t, d, kg) in &plates {
            println!("  {i:>5} {m:<28} {t:>7.1} mm  duct {d:>5.0}  {kg:>8.0} kg  {:>4.0}%", kg / armour_total.max(1.0) * 100.0);
        }
        println!("  armour {:.1} t + modifiers {:.1} t + ACF-computed ≈ {:.1} t  =  {:.1} t", armour_total / 1000.0, other / 1000.0, acf_est / 1000.0, total / 1000.0);
        let room = target_t * 1000.0 - (other + acf_est);
        if armour_total > 0.0 && room > 0.0 {
            let scale = room / armour_total;
            println!("  to land on {target_t:.1} t, scale every thickness by {scale:.2} (e.g. {} mm → {} mm on the thickest plate)",
                plates.first().map(|p| format!("{:.0}", p.2)).unwrap_or_default(),
                plates.first().map(|p| format!("{:.0}", p.2 * scale)).unwrap_or_default());
            println!("  or keep the front and thin the sides/rear/roof; the doctor won't stop you either way");
        }
        return;
    }

    if let Some(dir) = &extract_all {
        if let Err(e) = fs::create_dir_all(dir) {
            eprintln!("{dir}: {e}");
            return;
        }
        let mut used: std::collections::HashMap<String, usize> = Default::default();
        let mut n = 0;
        for (index, class, model) in dupe.list_entities() {
            let class = class.trim_matches('"').to_owned();
            let model = model.trim_matches('"').to_owned();
            let keep = class.starts_with("acf_") || class.starts_with("sent_tanktracks") || class == "prop_vehicle_prisoner_pod"
                || (class == "prop_physics" && (model.contains("wheel") || model.contains("tire") || model.contains("/tank")));
            if !keep {
                continue;
            }
            let et = transform::entity_table(&dupe, index).unwrap();
            let id = ["Engine", "Gearbox", "Turret", "FuelTank", "Motor", "CrewTypeID", "Gyro"]
                .iter()
                .find_map(|k| match dupe.get(et, k) { Some(ad2read::value::Value::Str(b)) => Some(String::from_utf8_lossy(b).into_owned()), _ => None })
                .or_else(|| match (dupe.get(et, "Weapon"), dupe.get_number(et, "Caliber")) {
                    (Some(ad2read::value::Value::Str(w)), Some(c)) => Some(format!("{}{}", String::from_utf8_lossy(w), c)),
                    _ => None,
                })
                .unwrap_or_else(|| model.rsplit('/').next().unwrap_or("part").trim_end_matches(".mdl").to_owned());
            let stem = format!("{}_{}", class.trim_start_matches("acf_"), id).replace(['/', ' ', '.'], "_").to_lowercase();
            let count = used.entry(stem.clone()).or_insert(0);
            *count += 1;
            let name = if *count == 1 { stem } else { format!("{stem}_{count}") };
            let out = format!("{}/{name}.txt", dir.trim_end_matches('/'));
            // Same path as --extract.
            let mut part = match duplicate::extract(&dupe, &[index]) { Ok(p) => p, Err(e) => { eprintln!("{index}: {e}"); continue; } };
            let outside: Vec<f64> = refs::dangling(&part).iter().map(|h| h.value).collect();
            let mut seen = std::collections::HashSet::new();
            for t in outside { if seen.insert(t.to_bits()) { refs::prune_entity(&mut part, t); } }
            let _ = ad2read::extras::repair_head(&mut part);
            let Ok(body) = part.to_body() else { continue };
            let Ok(compressed) = dupefile::compress(&body, 3, 0, 2, 65536, true) else { continue };
            let size = body.len().to_string();
            let mut info = file.info.clone();
            for (k, v) in info.iter_mut() {
                if k == b"size" { *v = size.clone().into_bytes(); }
                if k == b"name" { *v = name.clone().into_bytes(); }
            }
            if fs::write(&out, dupefile::build(file.revision, &info, &compressed)).is_ok() {
                println!("  {index:>5} {class:26} -> {out}");
                n += 1;
            }
        }
        println!("extracted {n} parts to {dir}");
        return;
    }

    if send {
        // Into the AdvDupe2 folder the editor was set up with, after the same
        // check every write gets.
        let problems = ad2read::extras::validate(&dupe);
        if !problems.is_empty() {
            eprintln!("not sent: AdvDupe2 would refuse it ({})", problems.join("; "));
            return;
        }
        let Some(folder) = configured_folder(|c| c.advdupe2.clone()) else {
            eprintln!("not sent: no AdvDupe2 folder is set; run ad2edit once, or copy the file yourself");
            return;
        };
        let Some(name) = std::path::Path::new(path).file_name() else { return };
        let target = std::path::Path::new(&folder).join(name);
        match fs::copy(path, &target) {
            Ok(_) => println!("sent to {}", target.display()),
            Err(e) => eprintln!("could not copy to {}: {e}", target.display()),
        }
        return;
    }
    if let Some(check) = mass_report {
        // --mass: what the contraption weighs and where each figure came
        // from. --mass-check: every part whose worked-out weight is not the
        // one the game saved with it.
        let mut lib = models_root.as_ref().map(|root| ad2read::models::ModelLibrary::new(std::path::Path::new(root)));
        let mut half_extents = |model: &str| lib.as_mut().and_then(|lib| lib.bounds(model)).map(|(bounds, _)| bounds.half_extents());
        if check {
            let (agreed, lines) = ad2read::mass::disagreements(&dupe, &mut half_extents, 0.02);
            for line in &lines {
                println!("  {line}");
            }
            println!("{agreed} part(s) weigh what the game saved, {} do not", lines.len());
            return;
        }
        let report = ad2read::mass::of_dupe(&dupe, &mut half_extents);
        println!("{}", report.headline());
        for (role, kg) in report.by_role(&dupe) {
            println!("  {kg:>10.1} kg  {role}");
        }
        let mut sources = std::collections::BTreeMap::new();
        for part in &report.parts {
            *sources.entry(part.source).or_insert(0usize) += 1;
        }
        println!("  from: {}", sources.iter().map(|(source, count)| format!("{count} {source:?}")).collect::<Vec<_>>().join(", "));
        if !report.unknown.is_empty() {
            println!("  unknown: {}", report.unknown.iter().map(|index| index.to_string()).collect::<Vec<_>>().join(" "));
        }
        return;
    }
    if scale_check {
        // Every part whose game-saved Scale is not what the rules give.
        let Some(root) = models_root.as_ref() else {
            eprintln!("--scale-check needs --models <garrysmod> (or the game folder set in ad2edit's Settings)");
            return;
        };
        let mut lib = ad2read::models::ModelLibrary::new(std::path::Path::new(root));
        let found = ad2read::scale::disagreements(&dupe, &mut lib);
        for line in &found {
            println!("  {line}");
        }
        println!("{} part(s) drawn at a size other than the one the game saved", found.len());
        return;
    }
    if catalogue_check {
        // Every catalogue model against the game's own files: there, with a
        // mesh, and posed inside its own bounds.
        let Some(root) = models_root.as_ref() else {
            eprintln!("--catalogue-check needs --models <garrysmod> (or the game folder set in ad2edit's Settings)");
            return;
        };
        let mut lib = ad2read::models::ModelLibrary::new(std::path::Path::new(root));
        let problems = ad2read::catalog::model_problems(&mut lib);
        for problem in &problems {
            println!("  {problem}");
        }
        println!("{} catalogue model problem(s)", problems.len());
        return;
    }
    if let Some((out, yaw, pitch)) = render_to {
        let Some(root) = models_root.as_ref() else {
            eprintln!("--render needs --models <garrysmod> (or the game folder set in ad2edit's Settings)");
            return;
        };
        let mut lib = ad2read::models::ModelLibrary::new(std::path::Path::new(root));
        let brightness = match ad2read::config::load() {
            ad2read::config::LoadResult::Ok(config) => config.viewport.brightness,
            _ => 1.0,
        };
        let (picture, without_mesh) = ad2read::render::picture(&dupe, &mut lib, 1280, 800, yaw, pitch, brightness);
        match std::fs::write(&out, ad2read::render::png(&picture)) {
            Ok(()) => println!("rendered {out} from yaw {yaw} pitch {pitch}; {without_mesh} entities had no mesh"),
            Err(e) => eprintln!("could not write {out}: {e}"),
        }
        return;
    }
    if model_check {
        // --model-check: every distinct model in the dupe, and whether the
        // viewport could get a size, a mesh and a texture for it.
        let Some(root) = models_root.as_ref() else {
            eprintln!("--model-check needs --models <garrysmod> (or the game folder set in ad2edit's Settings)");
            return;
        };
        let mut lib = ad2read::models::ModelLibrary::new(std::path::Path::new(root));
        println!("{}", lib.describe_search());
        let mut models: Vec<(String, String)> = Vec::new();
        for (_, class, model) in dupe.list_entities() {
            let m = model.trim_matches('"').to_owned();
            if !models.iter().any(|(mm, _)| *mm == m) {
                models.push((m, class));
            }
        }
        let (mut ok, mut bad) = (0, 0);
        for (model, class) in &models {
            let bounds = lib.bounds(model).map(|(b, _)| b.half_extents());
            let mesh = lib.mesh(model).map(|m| m.triangles());
            let texture = lib.texture(model).is_some();
            let status = match (bounds, mesh) {
                (Some(_), Some(_)) => "ok",
                (Some(_), None) => "no mesh",
                (None, _) => "no model",
            };
            if status == "ok" { ok += 1 } else { bad += 1 }
            let detail = match (bounds, mesh) {
                (Some(h), Some(t)) => format!("{t} tris, half {:.1}x{:.1}x{:.1}, texture {}", h.0, h.1, h.2, if texture { "yes" } else { "NO" }),
                _ => lib.mesh_reason(model),
            };
            println!("{status:8} {class:<24} {model}\n         {detail}");
        }
        println!("{ok} models render, {bad} do not");
        return;
    }
    if let Some(filt) = &catalog {
        if filt.is_empty() {
            println!("\n--- catalogue: {} items ---", ad2read::catalog::ITEMS.len());
            for (c, n) in ad2read::catalog::categories() {
                println!("  {n:>4}  {c}");
            }
            println!("  (--catalog <category> lists items; --add <id> spawns one)");
        } else {
            let items = ad2read::catalog::in_category(filt);
            if items.is_empty() {
                let by_id: Vec<&ad2read::catalog::Item> = ad2read::catalog::ITEMS.iter().filter(|i| i.value.to_lowercase().contains(&filt.to_lowercase()) || i.name.to_lowercase().contains(&filt.to_lowercase())).collect();
                for it in by_id { println!("  {:<18} {:<24} {:<40} {}", it.value, it.entity, it.name, it.note); }
            } else {
                for it in items {
                    let extra = match (it.range, it.mass) {
                        (Some((lo, hi)), _) => format!("{lo}..{hi}"),
                        (None, Some(m)) => format!("{m:.0} kg"),
                        _ => String::new(),
                    };
                    println!("  {:<18} {:<20} {:<44} {:<10} {}", it.value, it.entity, it.name, extra, if it.fuels.is_empty() { it.note.to_string() } else { it.fuels.join("/") });
                }
            }
        }
        return;
    }
    if let Some(n) = explain {
        match ad2read::pipeline::explain(&dupe, n) {
            Ok(j) => println!("{}", serde_json::to_string_pretty(&j).unwrap_or_default()),
            Err(e) => eprintln!("explain: {e}"),
        }
        return;
    }
    if let Some((pen, angles)) = &versus {
        let mut lib = models_root.as_ref().map(|r| ad2read::models::ModelLibrary::new(std::path::Path::new(r)));
        let mut half_of = |m: &str| lib.as_mut().and_then(|l| l.bounds(m)).map(|(b, _)| b.half_extents()).or_else(|| build::sprops_half_extents(m));
        let plates = ad2read::pipeline::plates_of(&dupe, &mut half_of);
        let rows = ad2read::pipeline::versus(&plates, *pen, angles);
        if json_out {
            println!("{}", serde_json::to_string_pretty(&rows).unwrap_or_default());
        } else {
            println!("\n--- {pen:.0} mm of penetration against {} plate(s) ---", plates.len());
            for r in &rows {
                let cells: Vec<String> = r["at"].as_array().unwrap().iter().map(|a| format!("{:>3.0}°: {:>6.0} eff {}", a["angle"].as_f64().unwrap(), a["effective_mm"].as_f64().unwrap(), if a["penetrates"].as_bool().unwrap() { "PEN" } else { "held" })).collect();
                println!("  {:>5} {:>6.0} mm {}  {}", r["plate"], r["thickness_mm"].as_f64().unwrap(), if r["front"].as_bool().unwrap() { "front" } else { "     " }, cells.join("  "));
            }
        }
        return;
    }

    if let Some((n, out)) = &extract_to {
        let mut part = match duplicate::extract(&dupe, &[*n]) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("extract: {e}");
                return;
            }
        };
        // Shed everything that pointed outside, exactly as --spawn does.
        let outside: Vec<f64> = refs::dangling(&part).iter().map(|h| h.value).collect();
        let mut seen = std::collections::HashSet::new();
        for t in outside {
            if seen.insert(t.to_bits()) {
                refs::prune_entity(&mut part, t);
            }
        }
        for fix in ad2read::extras::repair_head(&mut part) {
            println!("repaired: {fix}");
        }
        let body = match part.to_body() {
            Ok(b) => b,
            Err(e) => {
                eprintln!("extract: {e}");
                return;
            }
        };
        let compressed = match dupefile::compress(&body, 3, 0, 2, 65536, true) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("extract: {e}");
                return;
            }
        };
        let mut info = file.info.clone();
        for (k, v) in info.iter_mut() {
            if k == b"size" {
                *v = body.len().to_string().into_bytes();
            }
            if k == b"name" {
                *v = std::path::Path::new(out)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "part".into())
                    .into_bytes();
            }
        }
        let bytes = dupefile::build(file.revision, &info, &compressed);
        match fs::write(out, &bytes) {
            Ok(()) => println!("extracted entity {n} to {out} ({} bytes)", bytes.len()),
            Err(e) => eprintln!("could not write {out}: {e}"),
        }
        return;
    }

    if let Some((n, file)) = &chip_export {
        match build::chip_export(&dupe, *n) {
            Ok(build::Chip::E2 { name, code }) => {
                if let Err(e) = fs::write(file, &code) {
                    eprintln!("could not write {file}: {e}");
                    return;
                }
                println!("E2 \"{name}\": {} bytes of code written to {file}", code.len());
            }
            Ok(build::Chip::Starfall { mainfile, files }) => {
                // Main file to the path given; any others alongside it.
                let dir = std::path::Path::new(file)
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_default();
                for (name, code) in &files {
                    let target = if *name == mainfile {
                        std::path::PathBuf::from(file)
                    } else {
                        dir.join(name.replace('/', "_"))
                    };
                    if let Err(e) = fs::write(&target, code) {
                        eprintln!("could not write {}: {e}", target.display());
                        return;
                    }
                    println!("starfall {name} -> {}", target.display());
                }
                println!("main file is {mainfile}");
            }
            Err(e) => {
                eprintln!("chip-export: {e}");
            }
        }
        return;
    }

    if let Some(other) = diff_with {
        let Some(right) = open_dupe(&other) else { return };
        let changes = ad2read::extras::diff(&dupe, &right);
        if changes.is_empty() {
            println!("\nno differences");
            return;
        }
        println!("\n--- {} difference(s) ---", changes.len());
        for c in changes {
            use ad2read::extras::Change::*;
            match c {
                OnlyInLeft(i, m) => println!("  only in this file:   {i}  {m}"),
                OnlyInRight(i, m) => println!("  only in the other:   {i}  {m}"),
                Field(i, k, l, r) => println!("  entity {i}  {k}:  {l}  ->  {r}"),
                ConstraintCount(l, r) => println!("  constraints: {l} -> {r}"),
            }
        }
        return;
    }


    // Snapshot before editing. Nothing here needs interactive undo, but a
    // failed verification below can roll the whole thing back rather than
    // leaving a half-edited dupe in memory, and the GUI will use the same
    // History for real.
    let mut history = History::new(&dupe);

    let suffix;
    let mut needs_resync = false;

    if let Some(target) = delete {
        suffix = "_edit";
        println!("\n--- deleting entity {target} ---");
        let report = match dupe.delete_entity(target) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        };
        println!("removed 1 entity and {} constraint(s)", report.removed_constraints);
        let p = &report.pruned;
        if p.total() > 0 {
            println!(
                "pruned {} reference(s): {} _links, {} ACF, {} parent, {} wire port(s), {} waypoint(s)",
                p.total(), p.links, p.acf, p.parents, p.wires, p.wire_path
            );
        }
        if let Some(h) = report.head_repointed {
            println!("HeadEnt pointed at the deleted entity; repointed to {h}");
        }
    } else if let Some((from, to)) = remap {
        suffix = "_edit";
        println!("\n--- remapping entity {from} to {to} ---");
        if dupe.list_entities().iter().any(|(i, _, _)| *i == to) {
            eprintln!("entity {to} already exists; pick a free index");
            return;
        }
        let changed = refs::remap(&mut dupe, &[(from, to)]);
        if changed == 0 {
            eprintln!("no references to {from} found");
            return;
        }
        println!("rewrote {changed} reference(s)");
    } else if let Some(other_path) = merge_with {
        suffix = "_merged";
        println!("\n--- merging {other_path} ---");
        let other = match load(&other_path) {
            Ok(l) => l.dupe,
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        };
        if offset == (0.0, 0.0, 0.0) {
            println!("no --offset given, so the two builds will occupy the same space");
        }
        let report = match merge::merge(&mut dupe, other, offset) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        };
        println!(
            "added {} entities (renumbered from {}) and {} constraints",
            report.entities_added, report.first_new_index, report.constraints_added
        );
        println!("rewrote {} reference(s), moved {} entities", report.remapped, report.moved);
    } else if let Some(set) = duplicate_set {
        suffix = "_edit";
        println!("\n--- duplicating {set:?} ---");
        if offset == (0.0, 0.0, 0.0) {
            println!("no --offset given, so the copy will land inside the original");
        }
        let report = match duplicate::duplicate(&mut dupe, &set, offset) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        };
        println!(
            "copied {} entities (numbered from {}) and {} internal constraint(s)",
            report.entities_added, report.first_new_index, report.constraints_added
        );
        if report.constraints_added == 0 && report.entities_added > 1 {
            println!("(no constraint had all its endpoints inside the selection)");
        }
    } else if let Some(delta) = translate {
        suffix = "_edit";
        println!("\n--- translating everything by {delta:?} ---");
        let moved = transform::translate(&mut dupe, delta);
        println!("moved {moved} entities (uniform, so no constraint resync needed)");
    } else if let Some((target, delta)) = move_ent {
        suffix = "_edit";
        println!("\n--- moving entity {target} by {delta:?} ---");
        let Some((pos, _)) = transform::entity_transform(&dupe, target) else {
            eprintln!("entity {target} has no readable transform");
            return;
        };
        let new = (pos.0 + delta.0, pos.1 + delta.1, pos.2 + delta.2);
        if let Err(e) = transform::set_entity_transform(&mut dupe, target, Some(new), None) {
            eprintln!("{e}");
            return;
        }
        println!("{pos:?} -> {new:?}");
        needs_resync = true;
    } else if !spawns.is_empty()
        || !axes.is_empty()
        || !welds.is_empty()
        || !nocollides.is_empty()
        || !ballsockets.is_empty()
        || !links.is_empty()
        || chip_import.is_some()
        || !align_axles.is_empty()
        || !wheel_axes.is_empty()
        || !rot_locks.is_empty()
        || !parents.is_empty()
        || !puts.is_empty()
        || !unmods.is_empty()
        || !pair_locks.is_empty()
        || !sprung.is_empty()
        || !hydraulic.is_empty()
        || !sphericals.is_empty()
        || !wires.is_empty()
        || !ammo.is_empty()
        || !spawn_parts.is_empty()
        || !adds.is_empty()
        || sloped_hull.is_some()
        || !track_links.is_empty()
        || !doors.is_empty()
        || !submats.is_empty()
        || !keypads.is_empty()
        || !resizes.is_empty()
        || !armours.is_empty()
        || armour_apply.is_some()
        || fix
        || clean
    {
        suffix = "_edit";
        println!("\n--- building ---");

        for (id, at, ang, nums) in &adds {
            let Some(item) = ad2read::catalog::find(id) else {
                eprintln!("--add: no catalogue item {id}; --catalog to browse");
                return;
            };
            let nums: Vec<(&str, f64)> = nums.iter().map(|(k, v)| (k.as_str(), *v)).collect();
            match build::spawn_catalog(&mut dupe, item, *at, ang.unwrap_or((0.0, 90.0, 0.0)), &nums) {
                Ok(new) => println!("added {} ({}) as {new} at {:?}", item.name, item.entity, at),
                Err(e) => { eprintln!("add: {e}"); return; }
            }
        }
        if let Some((height, degrees, thickness)) = sloped_hull {
            let Some(base) = ad2read::roles::of_dupe(&dupe).into_iter().find(|(_, role)| *role == ad2read::roles::Role::Hull).map(|(index, _)| index) else {
                eprintln!("--hull needs a baseplate to build on");
                return;
            };
            match ad2read::hull::add_sloped_hull(&mut dupe, base, height, degrees, thickness) {
                Ok((made, glacis)) => println!(
                    "built a hull of {} pieces; glacis {:.0} degrees from the vertical: {:.2}x its thickness to a level shot, {:.0} % of AP that fails to get through glances off",
                    made.len(),
                    glacis.degrees,
                    glacis.thicker_by,
                    glacis.ap_glances * 100.0
                ),
                Err(e) => {
                    eprintln!("hull: {e}");
                    return;
                }
            }
        }
        for (name, at, ang) in &spawn_parts {
            let Some(dir) = parts_dir.as_ref() else {
                eprintln!("--spawn-part needs --parts <dir>");
                return;
            };
            let path = format!("{}/{}.txt", dir.trim_end_matches('/'), name);
            let Some(template) = open_dupe(&path) else { return };
            let Some((first, _, _)) = template.list_entities().into_iter().next() else {
                eprintln!("{path} has no entities");
                return;
            };
            match build::spawn_from(&mut dupe, &template, first, *at, *ang) {
                Ok(new) => println!("spawned part {name} as {new} at {:?}", at),
                Err(e) => {
                    eprintln!("spawn-part: {e}");
                    return;
                }
            }
        }
        for (template_path, n, at, ang) in &spawns {
            let Some(template) = open_dupe(template_path) else { return };
            match build::spawn_from(&mut dupe, &template, *n, *at, *ang) {
                Ok(new) => println!("spawned template entity {n} as {new} at {:?}", at),
                Err(e) => {
                    eprintln!("spawn: {e}");
                    return;
                }
            }
        }
        if !align_axles.is_empty() {
            let Some(root) = models_root.as_ref() else {
                eprintln!("--align-axle needs --models <garrysmod> to read the wheel's bounds");
                return;
            };
            let mut lib = ad2read::models::ModelLibrary::new(std::path::Path::new(root));
            for (n, dir) in &align_axles {
                let model = dupe
                    .list_entities()
                    .into_iter()
                    .find(|(i, _, _)| i == n)
                    .map(|(_, _, m)| m.trim_matches('"').to_owned());
                let Some(model) = model else {
                    eprintln!("align-axle: no entity {n}");
                    return;
                };
                let Some((b, _)) = lib.bounds(&model) else {
                    eprintln!("align-axle: could not resolve {model}");
                    return;
                };
                let axle = build::thinnest_axis(b.half_extents());
                let ang = build::angle_aligning(axle, *dir);
                if let Err(e) = transform::set_entity_transform(&mut dupe, *n, None, Some(ang)) {
                    eprintln!("align-axle: {e}");
                    return;
                }
                println!(
                    "aligned {n}: {model} axle is local {:?}, now angle ({:.1}, {:.1}, {:.1})",
                    axle, ang.0, ang.1, ang.2
                );
            }
        }
        for (n, key, text) in &puts {
            let v = if let Ok(x) = text.parse::<f64>() {
                ad2read::value::Value::Number(x)
            } else if text == "true" || text == "false" {
                ad2read::value::Value::Bool(text == "true")
            } else {
                ad2read::value::Value::Str(text.as_bytes().to_vec())
            };
            if let Err(e) = build::put_path(&mut dupe, *n, key, v) {
                eprintln!("put: {e}");
                return;
            }
            println!("put {n}.{key} = {text}");
        }
        for (n, m) in &unmods {
            let Some(et) = transform::entity_table(&dupe, *n) else {
                eprintln!("unmod: no entity {n}");
                return;
            };
            if let Some(mods) = dupe.get_table(et, "EntityMods") {
                if let ad2read::value::Node::Table(e) = &mut dupe.arena[mods] {
                    let before = e.len();
                    e.retain(|(k, _)| !ad2read::value::key_matches(k, m));
                    println!("unmod {n}: {} ({})", m, if e.len() < before { "removed" } else { "was not there" });
                }
            }
        }
        for (c, p) in &parents {
            match build::set_parent(&mut dupe, *c, *p) {
                Ok(()) => println!("parented {c} to {p}"),
                Err(e) => {
                    eprintln!("parent: {e}");
                    return;
                }
            }
        }
        for (w, b, ax, nc) in &wheel_axes {
            match build::add_wheel_axis(&mut dupe, *w, *b, *ax, *nc) {
                Ok(()) => println!("wheel axis {w} on {b} about local {:?}", ax),
                Err(e) => {
                    eprintln!("wheel-axis: {e}");
                    return;
                }
            }
        }
        for (a, b) in &pair_locks {
            match build::add_pair_lock(&mut dupe, *a, *b) {
                Ok(()) => println!("pair lock {a} <-> {b} (x, y, z)"),
                Err(e) => {
                    eprintln!("lock-pair: {e}");
                    return;
                }
            }
        }
        for (w, b) in &sprung {
            match build::add_sprung_wheel(&mut dupe, *w, *b) {
                Ok(()) => println!("sprung wheel {w} on {b}: elastic, 3 ropes, socket, nocollide"),
                Err(e) => {
                    eprintln!("sprung-wheel: {e}");
                    return;
                }
            }
        }
        for (w, b, c) in &hydraulic {
            match build::add_hydraulic_wheel(&mut dupe, *w, *b, *c) {
                Ok(()) => println!("hydraulic wheel {w} on {b}, controller {c}: hydraulic, 3 ropes, socket"),
                Err(e) => {
                    eprintln!("hydraulic-wheel: {e}");
                    return;
                }
            }
        }
        if !sphericals.is_empty() {
            let mut lib = models_root
                .as_ref()
                .map(|r| ad2read::models::ModelLibrary::new(std::path::Path::new(r)));
            for (n, r) in &sphericals {
                let radius = match r {
                    Some(r) => *r,
                    None => {
                        let model = dupe
                            .list_entities()
                            .into_iter()
                            .find(|(i, _, _)| i == n)
                            .map(|(_, _, m)| m.trim_matches('"').to_owned());
                        let Some(lib) = lib.as_mut() else {
                            eprintln!("--spherical without a radius needs --models <garrysmod>");
                            return;
                        };
                        let Some(b) = model.and_then(|m| lib.bounds(&m)) else {
                            eprintln!("spherical: could not resolve model for {n}");
                            return;
                        };
                        let (x, y, z) = b.0.half_extents();
                        x.max(y).max(z)
                    }
                };
                match build::make_spherical(&mut dupe, *n, radius) {
                    Ok(()) => println!("spherical {n}: radius {radius:.2}"),
                    Err(e) => {
                        eprintln!("spherical: {e}");
                        return;
                    }
                }
            }
        }
        for (a, b, lock) in &rot_locks {
            match build::add_rotation_lock(&mut dupe, *a, *b, *lock) {
                Ok(()) => println!("lock {a} <-> {b} {:?}", lock),
                Err(e) => {
                    eprintln!("lock: {e}");
                    return;
                }
            }
        }
        for (a, b, at, dir, friction, forcelimit, torquelimit, nocollide) in &axes {
            let spec = build::AxisSpec {
                point: *at,
                axis: *dir,
                friction: *friction,
                forcelimit: *forcelimit,
                torquelimit: *torquelimit,
                nocollide: *nocollide,
            };
            match build::add_axis(&mut dupe, *a, *b, &spec) {
                Ok(()) => println!("axis {a} <-> {b} at {:?} along {:?}", at, dir),
                Err(e) => {
                    eprintln!("axis: {e}");
                    return;
                }
            }
        }
        for (a, b, forcelimit, nocollide) in &welds {
            match build::add_weld(&mut dupe, *a, *b, *forcelimit, *nocollide) {
                Ok(()) => println!("weld {a} <-> {b}"),
                Err(e) => {
                    eprintln!("weld: {e}");
                    return;
                }
            }
        }
        for (a, b) in &nocollides {
            match build::add_nocollide(&mut dupe, *a, *b) {
                Ok(()) => println!("nocollide {a} <-> {b}"),
                Err(e) => {
                    eprintln!("nocollide: {e}");
                    return;
                }
            }
        }
        for (a, b, at) in &ballsockets {
            match build::add_ballsocket(&mut dupe, *a, *b, *at, 0.0, 0.0, false) {
                Ok(()) => println!("ballsocket {a} <-> {b} at {:?}", at),
                Err(e) => {
                    eprintln!("ballsocket: {e}");
                    return;
                }
            }
        }
        for (a, b) in &links {
            match build::add_link(&mut dupe, *a, *b) {
                Ok(r) => println!(
                    "link {} -> {} via {} ({:.0} units)",
                    r.owner, r.target, r.modifier, r.distance
                ),
                Err(e) => {
                    eprintln!("link: {e}");
                    return;
                }
            }
        }
        for (src, sp, dst, dp) in &wires {
            match build::add_wire(&mut dupe, *src, sp, *dst, dp) {
                Ok(()) => println!("wire {src}.{sp} -> {dst}.{dp}"),
                Err(e) => {
                    eprintln!("wire: {e}");
                    return;
                }
            }
        }
        for (n, t, size) in &ammo {
            match build::set_ammo(&mut dupe, *n, t, *size) {
                Ok(()) => println!("ammo {n}: {t}{}", size.map(|s| format!(" size {:?}", s)).unwrap_or_default()),
                Err(e) => {
                    eprintln!("ammo: {e}");
                    return;
                }
            }
        }
        for (n, k, m) in &doors {
            match build::set_fading_door(&mut dupe, *n, *k, m, true) {
                Ok(()) => println!("fading door {n}: key {k}, {m}"),
                Err(e) => { eprintln!("fading-door: {e}"); return; }
            }
        }
        for (n, slot, m) in &submats {
            match build::set_submaterial(&mut dupe, *n, *slot, m) {
                Ok(()) => println!("submaterial {n}[{slot}] = {m}"),
                Err(e) => { eprintln!("submaterial: {e}"); return; }
            }
        }
        for (n, pw, key) in &keypads {
            match build::set_keypad(&mut dupe, *n, *pw, false, *key) {
                Ok(()) => println!("keypad {n}: password {}, key {key}", pw.map(|p| p.to_string()).unwrap_or_else(|| "none".into())),
                Err(e) => { eprintln!("keypad: {e}"); return; }
            }
        }
        for (n, sc, bones) in &resizes {
            match build::set_resize(&mut dupe, *n, *sc, *bones) {
                Ok(()) => println!("resize {n}: {:?} on {bones} bone(s)", sc),
                Err(e) => { eprintln!("resize: {e}"); return; }
            }
        }
        for (t, c, ws) in &track_links {
            match build::set_track_links(&mut dupe, *t, *c, ws) {
                Ok(()) => println!("track {t}: chassis {c}, {} wheels", ws.len()),
                Err(e) => {
                    eprintln!("track-link: {e}");
                    return;
                }
            }
        }
        if let Some((n, file)) = &chip_import {
            let text = match fs::read_to_string(file) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("could not read {file}: {e}");
                    return;
                }
            };
            // A Starfall import takes a directory of files; an E2 a single
            // file. Decide by what the chip already is.
            let chip = match build::chip_export(&dupe, *n) {
                Ok(build::Chip::Starfall { mainfile, files }) => {
                    let mut files = files;
                    // Replace the main file's code; other files keep theirs.
                    match files.iter_mut().find(|(name, _)| *name == mainfile) {
                        Some((_, code)) => *code = text,
                        None => files.push((mainfile.clone(), text)),
                    }
                    build::Chip::Starfall { mainfile, files }
                }
                Ok(build::Chip::E2 { name, .. }) => build::Chip::E2 { name, code: text },
                Err(e) => {
                    eprintln!("chip-import: {e}");
                    return;
                }
            };
            match build::chip_import(&mut dupe, *n, chip) {
                Ok(()) => println!("imported {file} into chip {n}"),
                Err(e) => {
                    eprintln!("chip-import: {e}");
                    return;
                }
            }
        }
        for (targets, mm, duct) in &armours {
            let list: Vec<f64> = if targets.is_empty() {
                // "props" means plates: prop_physics that aren't wheels
                // (spherical collisions, or a wheel/tire/tank model).
                dupe.list_entities().into_iter().filter(|(i, c, m)| {
                    if !c.contains("prop_physics") { return false; }
                    let m = m.to_lowercase();
                    if m.contains("wheel") || m.contains("tire") || m.contains("/tank") { return false; }
                    let spherical = transform::entity_table(&dupe, *i)
                        .and_then(|et| dupe.get_table(et, "EntityMods"))
                        .map(|mods| dupe.get_table(mods, "MakeSphericalCollisions").is_some())
                        .unwrap_or(false);
                    !spherical
                }).map(|(i, _, _)| i).collect()
            } else {
                targets.clone()
            };
            let mut n = 0;
            for t in &list {
                match build::set_armour(&mut dupe, *t, *mm, *duct) {
                    Ok(()) => n += 1,
                    Err(e) => { eprintln!("armour {t}: {e}"); return; }
                }
            }
            println!("armour {mm} mm (ductility {duct}) on {n} plate(s)");
        }
        if let Some((tonnes, bias)) = armour_apply {
            let mut lib = models_root.as_ref().map(|r| ad2read::models::ModelLibrary::new(std::path::Path::new(r)));
            let mut half_of = |m: &str| lib.as_mut().and_then(|l| l.bounds(m)).map(|(b, _)| b.half_extents()).or_else(|| build::sprops_half_extents(m));
            let plates = ad2read::pipeline::plates_of(&dupe, &mut half_of);
            // Armour gets what's left after the non-armour mass (modifiers + ACF's own ≈ 7.5 t).
            let other: f64 = dupe.list_entities().iter().filter_map(|(i, _, _)| {
                let et = transform::entity_table(&dupe, *i)?;
                dupe.get_table(et, "EntityMods").and_then(|m| dupe.get_table(m, "mass")).and_then(|m| dupe.get_number(m, "Mass"))
            }).sum::<f64>() + 7500.0;
            let target_kg = (tonnes * 1000.0 - other).max(0.0);
            let plan = ad2read::pipeline::plan_thicknesses(&plates, target_kg, bias);
            let n = ad2read::pipeline::apply_thicknesses(&mut dupe, &plan);
            for (i, t) in &plan {
                println!("  plate {i}: {t:.1} mm");
            }
            println!("armour set on {n} plate(s) for {tonnes:.1} t total (front ×{bias})");
        }
        if clean {
            for line in ad2read::doctor::clean(&mut dupe) {
                println!("clean: {line}");
            }
        }
        if fix {
            let mut lib = models_root.as_ref().map(|r| ad2read::models::ModelLibrary::new(std::path::Path::new(r)));
            let findings = ad2read::doctor::doctor_with(&dupe, lib.as_mut());
            let done = ad2read::doctor::apply_fixes(&mut dupe, &findings);
            for d in &done {
                println!("fixed: {d}");
            }
            let left: Vec<_> = ad2read::doctor::doctor(&dupe);
            let remaining = left.iter().filter(|f| f.severity != ad2read::doctor::Severity::Note).count();
            println!("{} fix(es) applied, {} finding(s) remain that need a person", done.len(), remaining);
            for f in left.iter().filter(|f| f.severity != ad2read::doctor::Severity::Note) {
                println!("  {:?} {}", f.severity, f.what);
            }
        }
        needs_resync = true;
    } else if let Some(n) = weld_world {
        suffix = "_edit";
        println!("\n--- welding entity {n} to the world ---");
        if let Err(e) = ad2read::extras::weld_to_world(&mut dupe, n) {
            eprintln!("{e}");
            return;
        }
        println!("added a Weld with a world endpoint (Index 0)");
        needs_resync = true;
    } else if let Some((target, ang)) = rotate_ent {
        suffix = "_edit";
        println!("\n--- setting entity {target} angle to {ang:?} ---");
        let Some((_, old)) = transform::entity_transform(&dupe, target) else {
            eprintln!("entity {target} has no readable transform");
            return;
        };
        let extra = match transform::set_entity_transform(&mut dupe, target, None, Some(ang)) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        };
        println!("{old:?} -> {ang:?}");
        if extra > 0 {
            println!("rotated {extra} additional physics bone(s) about the entity origin");
        }
        needs_resync = true;
    } else if !sets.is_empty() {
        suffix = "_edit";
        match &filter {
            Some((k, v)) => println!("\n--- setting {sets:?} where {k}={v} ---"),
            None => println!("\n--- setting {sets:?} on every entity ---"),
        }
        let f = filter.as_ref().map(|(k, v)| (k.as_str(), v.as_str()));
        let report = match bulk::bulk_set(&mut dupe, f, &sets) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        };
        println!(
            "{} entities matched, {} field(s) written",
            report.matched, report.changed
        );
        if report.skipped_missing > 0 {
            println!(
                "{} skipped: the key isn't already on that entity, and adding one \
                 changes how it pastes",
                report.skipped_missing
            );
        }
        if report.skipped_type > 0 {
            println!(
                "{} skipped: the value didn't parse as the type that key already holds",
                report.skipped_type
            );
        }
        if report.changed == 0 {
            eprintln!("nothing changed; not writing a file");
            return;
        }
    } else {
        println!("\n--- structure ---");
        let mut seen = vec![false; dupe.arena.len()];
        // --full prints every key at any depth; the default keeps a dupe
        // readable at a glance.
        let (depth, items) = if full { (64, usize::MAX) } else { (5, 6) };
        dump(&dupe.root, &dupe.arena, 0, depth, items, &mut seen);
        return;
    }

    if needs_resync {
        let r = transform::resync_constraints(&mut dupe);
        println!(
            "resynced {} constraint pose(s) ({} world-anchored, {} skipped, {} weld bone pose(s))",
            r.updated, r.world_anchored, r.skipped, r.bones
        );
    }

    println!(
        "now {} entities, {} constraints",
        dupe.list_entities().len(),
        constraint_count(&dupe)
    );

    let after = refs::dangling(&dupe);
    let introduced = after.len().saturating_sub(before.len());
    if after.is_empty() {
        println!("no dangling references");
    } else {
        println!("\nWARNING: {} dangling reference(s), {introduced} new:", after.len());
        for h in &after {
            println!("  {} = {}", h.path, h.value);
        }
    }

    // Any failure past this point rolls back rather than leaving the edit
    // half-applied, and reports the state it returned to.
    // AD2 refuses a dupe missing any of HeadEnt.Z/Pos/Index or bone-0 data
    // with "could not be uploaded". Repair what a build can leave out, then
    // run AD2's own CheckValidDupe so nothing gets written that the game
    // would bounce.
    for fix in ad2read::extras::repair_head(&mut dupe) {
        println!("repaired: {fix}");
    }
    let problems = ad2read::extras::validate(&dupe);
    if !problems.is_empty() {
        eprintln!("refusing to write; AdvDupe2 would reject this file:");
        for p in problems {
            eprintln!("  {p}");
        }
        return;
    }

    let edited = match dupe.to_body() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("re-encode after edit failed: {e}");
            rollback(&mut history, &mut dupe);
            return;
        }
    };
    println!("body: {} bytes (was {})", edited.len(), raw.len());

    match Dupe::from_body(&edited) {
        Err(e) => {
            eprintln!("edited body does not decode: {e}");
            rollback(&mut history, &mut dupe);
            return;
        }
        Ok((re, used)) => {
            if used != edited.len() {
                eprintln!("edited body has {} trailing bytes", edited.len() - used);
                rollback(&mut history, &mut dupe);
                return;
            }
            match re.to_body() {
                Ok(again) if again == edited => {
                    println!("edited body re-decodes and round-trips cleanly")
                }
                Ok(_) => {
                    eprintln!("edited body is not stable across a second round trip");
                    rollback(&mut history, &mut dupe);
                    return;
                }
                Err(e) => {
                    eprintln!("second round trip failed: {e}");
                    rollback(&mut history, &mut dupe);
                    return;
                }
            }
        }
    }

    let compressed = match dupefile::compress(&edited, lc, lp, pb, dict, real_size) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("compression failed: {e}");
            return;
        }
    };

    let mut info = file.info.clone();
    for (k, v) in info.iter_mut() {
        if k.as_slice() == b"size" {
            *v = compressed.len().to_string().into_bytes();
        }
    }

    let out_bytes = dupefile::build(file.revision, &info, &compressed);
    let out_path = match save_to {
        Some(p) => p,
        None => match path.strip_suffix(".txt") {
            Some(stem) => format!("{stem}{suffix}.txt"),
            None => format!("{path}{suffix}"),
        },
    };
    match fs::write(&out_path, &out_bytes) {
        Ok(()) => println!("wrote {} ({} bytes)", out_path, out_bytes.len()),
        Err(e) => eprintln!("could not write {out_path}: {e}"),
    }
}

fn rollback(history: &mut History, dupe: &mut Dupe) {
    if history.undo(dupe) {
        eprintln!("rolled back to the state before the edit; nothing written");
    }
}

/// Lists every duplicator modifier in the dupe, with the union of the fields
/// each one carries and the type of each field.
///
/// This exists because addon and tool state lives under EntityMods with no
/// schema anywhere — colour, material, ACF links, wire data and anything a
/// tool decided to store. Guessing key names is how you write an editor that
/// silently mangles things, so this reports what is actually there.
/// Every constraint type in the dupe, how many endpoints each carries, and
/// the union of its fields. Endpoint count is the thing to watch: resync poses
/// slot 0 and the last slot, so a type with more than two is where an untested
/// path lives.
/// Resolves every model path in the dupe to the bounding box in its MDL
/// header. A dupe stores no dimensions at all, so this is the only way to know
/// how big anything actually is.
fn open_dupe(path: &str) -> Option<Dupe> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("could not read {path}: {e}");
            return None;
        }
    };
    let file = match dupefile::parse(&bytes) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{path}: {e}");
            return None;
        }
    };
    let raw = match dupefile::decompress(&file.compressed) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{path}: {e}");
            return None;
        }
    };
    match Dupe::from_body(&raw) {
        Ok((d, _)) => Some(d),
        Err(e) => {
            eprintln!("{path}: {e}");
            None
        }
    }
}

fn report_models(dupe: &Dupe, garrysmod: &str) {
    use std::collections::BTreeMap;
    use std::path::Path;

    let root = Path::new(garrysmod);
    if !root.join("models").exists() && !root.join("addons").exists() {
        eprintln!("that doesn't look like a garrysmod folder; expected models/ or addons/ inside it");
    }

    println!("\nindexing addons...");
    let mut lib = ad2read::models::ModelLibrary::new(root);
    println!(
        "scanned {} .gma archives ({} models indexed), {} vpk archives",
        lib.scanned_gmas,
        lib.indexed_models(),
        lib.archives()
    );

    // One line per distinct model, not per entity.
    let mut wanted: BTreeMap<String, usize> = BTreeMap::new();
    for (_, _, model) in dupe.list_entities() {
        *wanted.entry(model.trim_matches('"').to_owned()).or_insert(0) += 1;
    }

    println!("\n--- models ---");
    let mut missing = 0;
    for (model, count) in &wanted {
        match lib.bounds(model) {
            Some((b, src)) => {
                let h = b.half_extents();
                let where_ = match src {
                    ad2read::models::Source::Loose(p) => format!("loose {}", p.display()),
                    ad2read::models::Source::Generated => "generated".to_owned(),
                    ad2read::models::Source::Gma(p) => format!(
                        "gma {}",
                        p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
                    ),
                    ad2read::models::Source::Vpk(p) => format!(
                        "vpk {}",
                        p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
                    ),
                };
                println!(
                    "  x{count:<3} {model}\n        size {:.1} x {:.1} x {:.1}   [{where_}]",
                    h.0 * 2.0,
                    h.1 * 2.0,
                    h.2 * 2.0
                );
            }
            None => {
                missing += 1;
                println!("  x{count:<3} {model}\n        NOT FOUND");
            }
        }
    }
    if missing > 0 {
        println!(
            "\n{missing} model(s) not found. If these are stock props, the game \
             root or its vpk archives weren't reachable from the folder given."
        );
    }
}

fn report_constraints(dupe: &Dupe) {
    use std::collections::BTreeMap;

    let Ok(root) = dupe.root_table() else { return };
    let Some(cons) = dupe.get_table(root, "Constraints") else {
        eprintln!("no Constraints table");
        return;
    };
    let items: Vec<usize> = match &dupe.arena[cons] {
        value::Node::Array(v) => v.iter().filter_map(value::table_index).collect(),
        _ => Vec::new(),
    };

    // type -> (count, endpoint counts seen, fields, BuildDupeInfo fields)
    let mut kinds: BTreeMap<String, (usize, Vec<usize>, BTreeMap<String, String>, BTreeMap<String, String>)> =
        BTreeMap::new();

    for ct in items {
        let name = match dupe.get(ct, "Type") {
            Some(value::Value::Str(b)) => String::from_utf8_lossy(b).into_owned(),
            _ => "<no Type>".to_owned(),
        };
        let slot = kinds.entry(name).or_insert((0, Vec::new(), BTreeMap::new(), BTreeMap::new()));
        slot.0 += 1;

        if let Some(list) = dupe.get_table(ct, "Entity") {
            if let value::Node::Array(v) = &dupe.arena[list] {
                if !slot.1.contains(&v.len()) {
                    slot.1.push(v.len());
                }
            }
        }
        if let value::Node::Table(entries) = &dupe.arena[ct] {
            for (k, v) in entries {
                slot.2.insert(key_text(k), type_name(dupe, v));
            }
        }
        if let Some(bdi) = dupe.get_table(ct, "BuildDupeInfo") {
            if let value::Node::Table(entries) = &dupe.arena[bdi] {
                for (k, v) in entries {
                    slot.3.insert(key_text(k), type_name(dupe, v));
                }
            }
        }
    }

    println!("\n--- constraint types ---");
    for (name, (count, ends, fields, bdi)) in &kinds {
        let flag = if ends.iter().any(|e| *e != 2) {
            "   <-- more than two endpoints"
        } else {
            ""
        };
        println!("  {name}  x{count}   endpoints: {ends:?}{flag}");
        println!("      fields: {}", fields.keys().cloned().collect::<Vec<_>>().join(", "));
        if !bdi.is_empty() {
            println!("      BuildDupeInfo: {}", bdi.keys().cloned().collect::<Vec<_>>().join(", "));
        }
    }
}

fn report_mods(dupe: &Dupe) {
    use std::collections::BTreeMap;

    let Ok(root) = dupe.root_table() else { return };
    let Some(ents) = dupe.get_table(root, "Entities") else {
        eprintln!("no Entities table");
        return;
    };

    // modifier name -> (how many entities have it, field name -> type name)
    let mut found: BTreeMap<String, (usize, BTreeMap<String, String>)> = BTreeMap::new();
    // Top-level entity keys too, since not everything is a modifier.
    let mut top: BTreeMap<String, (usize, String)> = BTreeMap::new();

    let entity_tables: Vec<usize> = match &dupe.arena[ents] {
        value::Node::Array(items) => items.iter().filter_map(value::table_index).collect(),
        value::Node::Table(entries) => entries
            .iter()
            .filter_map(|(_, v)| value::table_index(v))
            .collect(),
    };

    for et in entity_tables {
        if let value::Node::Table(entries) = &dupe.arena[et] {
            for (k, v) in entries {
                let name = key_text(k);
                let e = top.entry(name).or_insert((0, type_name(dupe, v)));
                e.0 += 1;
            }
        }

        let Some(mods) = dupe.get_table(et, "EntityMods") else {
            continue;
        };
        let value::Node::Table(entries) = &dupe.arena[mods] else {
            continue;
        };
        for (k, v) in entries {
            let name = key_text(k);
            let slot = found.entry(name).or_insert((0, BTreeMap::new()));
            slot.0 += 1;
            if let Some(t) = value::table_index(v) {
                match &dupe.arena[t] {
                    value::Node::Table(fields) => {
                        for (fk, fv) in fields {
                            slot.1.insert(key_text(fk), type_name(dupe, fv));
                        }
                    }
                    value::Node::Array(items) => {
                        let inner = items
                            .first()
                            .map(|x| type_name(dupe, x))
                            .unwrap_or_else(|| "empty".into());
                        slot.1.insert("[array]".into(), inner);
                    }
                }
            } else {
                slot.1.insert("[value]".into(), type_name(dupe, v));
            }
        }
    }

    println!("\n--- EntityMods modifiers ---");
    if found.is_empty() {
        println!("  none: no entity in this dupe carries a duplicator modifier");
    }
    for (name, (count, fields)) in &found {
        println!("  {name}  (on {count} entities)");
        for (f, t) in fields {
            println!("      {f}: {t}");
        }
    }

    // DT is the networked-var table AD2 restores on paste; the AIO
    // controller's settings live there and nowhere else.
    {
        let mut by_class: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = Default::default();
        for (index, class, _) in dupe.list_entities() {
            let Some(et) = transform::entity_table(&dupe, index) else { continue };
            let Some(dt) = dupe.get_table(et, "DT") else { continue };
            if let ad2read::value::Node::Table(e) = &dupe.arena[dt] {
                let set = by_class.entry(class.trim_matches('"').to_owned()).or_default();
                for (k, _) in e {
                    set.insert(ad2read::value::short(k).trim_matches('"').to_owned());
                }
            }
        }
        if !by_class.is_empty() {
            println!("\n--- DT (networked vars, restored on paste; --put <n> DT.<key> <v>) ---");
            for (class, keys) in by_class {
                println!("  {class}: {}", keys.into_iter().collect::<Vec<_>>().join(", "));
            }
        }
    }
    println!("\n--- top-level entity keys ---");
    for (name, (count, t)) in &top {
        println!("  {name:<28} {t:<10} (on {count} entities)");
    }
}

fn key_text(k: &value::Value) -> String {
    match k {
        value::Value::Str(b) => String::from_utf8_lossy(b).into_owned(),
        value::Value::Number(n) => format!("[{n}]"),
        other => format!("{other:?}"),
    }
}

fn type_name(dupe: &Dupe, v: &value::Value) -> String {
    match v {
        value::Value::Nil => "nil".into(),
        value::Value::Bool(_) => "bool".into(),
        value::Value::Number(_) => "number".into(),
        value::Value::Vector(..) => "Vector".into(),
        value::Value::Angle(..) => "Angle".into(),
        value::Value::Str(_) => "string".into(),
        value::Value::Table(i) => match &dupe.arena[*i] {
            value::Node::Table(e) => format!("table[{}]", e.len()),
            value::Node::Array(a) => format!("array[{}]", a.len()),
        },
    }
}

fn report_refs(dupe: &Dupe) {
    let hits = refs::scan(dupe);

    println!("\n--- entity index references ---");
    for h in hits.iter().filter(|h| h.known) {
        let flag = if h.live { "" } else { "   <-- DANGLING" };
        println!("  {} = {}{}", h.path, h.value, flag);
    }

    let unknown: Vec<_> = hits.iter().filter(|h| !h.known).collect();
    if unknown.is_empty() {
        println!("\nno unrecognised sites");
    } else {
        println!("\n--- unrecognised sites holding a live entity index ---");
        println!("(remap leaves these alone; review, then add to classify() if real)");
        for h in &unknown {
            println!("  {} = {}", h.path, h.value);
        }
    }
}

fn constraint_count(dupe: &Dupe) -> usize {
    let Ok(root) = dupe.root_table() else {
        return 0;
    };
    let Some(cons) = dupe.get_table(root, "Constraints") else {
        return 0;
    };
    match &dupe.arena[cons] {
        value::Node::Array(items) => items.len(),
        value::Node::Table(entries) => entries.len(),
    }
}
