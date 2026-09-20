//! The chip commands through the real binary, on a real file.
use std::process::Command;

fn write_fixture(path: &std::path::Path) {
    write_fixture_parented(path, None);
}

fn write_fixture_parented(path: &std::path::Path, parent: Option<f64>) {
    use ad2read::value::{Node, Value};
    let mut d = ad2read::dupe::Dupe { root: Value::Nil, arena: Vec::new() };
    let root = d.new_table();
    d.root = Value::Table(root);
    let ents = d.new_table();
    d.set(root, "Entities", Value::Table(ents));
    let cons = d.new_array();
    d.set(root, "Constraints", Value::Table(cons));
    let head = d.new_table();
    d.set(head, "Index", Value::Number(1.0));
    d.set(head, "Pos", Value::Vector(0.0, 0.0, 0.0));
    d.set(head, "Z", Value::Number(0.0));
    d.set(root, "HeadEnt", Value::Table(head));
    let et = d.new_table();
    d.set(et, "Class", Value::Str(b"gmod_wire_expression2".to_vec()));
    d.set(et, "Model", Value::Str(b"models/beer/wiremod/gate_e2.mdl".to_vec()));
    let code: Vec<u8> = "@name Old\nprint(\"hi\")\n".bytes().map(|b| match b { b'"' => 163, b'\n' => 128, b => b }).collect();
    d.set(et, "_original", Value::Str(code));
    d.set(et, "_name", Value::Str(b"Old".to_vec()));
    let physics = d.new_table();
    let bone = d.new_table();
    d.set(bone, "Pos", Value::Vector(0.0, 0.0, 0.0));
    d.set(bone, "Angle", Value::Angle(0.0, 0.0, 0.0));
    d.set(physics, "0", Value::Table(bone));
    if let Node::Table(e) = &mut d.arena[physics] { e[0].0 = Value::Number(0.0); }
    d.set(et, "PhysicsObjects", Value::Table(physics));
    if let Some(parent) = parent {
        let info = d.new_table();
        d.set(info, "DupeParentID", Value::Number(parent));
        d.set(et, "BuildDupeInfo", Value::Table(info));
    }
    if let Node::Table(e) = &mut d.arena[ents] { e.push((Value::Number(1.0), Value::Table(et))); }
    let body = d.to_body().unwrap();
    let comp = ad2read::dupefile::compress(&body, 3, 0, 2, 65536, true).unwrap();
    let size = body.len().to_string();
    let info: Vec<(Vec<u8>, Vec<u8>)> = [("name", "chip"), ("size", size.as_str()), ("check", "\r\n\t\n")]
        .iter().map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec())).collect();
    std::fs::write(path, ad2read::dupefile::build(5, &info, &comp)).unwrap();
}

#[test]
fn chip_export_then_import_round_trips_through_the_cli() {
    let dir = std::env::temp_dir().join(format!("ad2cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dupe = dir.join("chip.txt");
    let code = dir.join("code.txt");
    let out = dir.join("chip2.txt");
    write_fixture(&dupe);
    let bin = env!("CARGO_BIN_EXE_ad2read");

    let o = Command::new(bin).args([dupe.to_str().unwrap(), "--chip-export", "1", code.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(std::fs::read_to_string(&code).unwrap(), "@name Old\nprint(\"hi\")\n");

    std::fs::write(&code, "@name New\nA = 2\n").unwrap();
    let o = Command::new(bin).args([dupe.to_str().unwrap(), "--chip-import", "1", code.to_str().unwrap(), "--out", out.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    let code2 = dir.join("code2.txt");
    let o = Command::new(bin).args([out.to_str().unwrap(), "--chip-export", "1", code2.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success());
    assert_eq!(std::fs::read_to_string(&code2).unwrap(), "@name New\nA = 2\n");
    let shown = Command::new(bin).args([out.to_str().unwrap(), "--show", "1"]).output().unwrap();
    assert!(String::from_utf8_lossy(&shown.stdout).contains("\"_name\" = \"New\""), "name follows @name");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_catalogue_tank_builds_from_the_cli_and_lands_in_the_configured_folder() {
    let dir = std::env::temp_dir().join(format!("ad2cat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let appdata = dir.join("appdata");
    let ad2 = dir.join("advdupe2");
    std::fs::create_dir_all(appdata.join("ad2edit")).unwrap();
    std::fs::create_dir_all(&ad2).unwrap();
    let config = format!("advdupe2 = {:?}\n", ad2.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"));
    std::fs::write(appdata.join("ad2edit").join("config.toml"), config).unwrap();
    let bin = env!("CARGO_BIN_EXE_ad2read");
    let toml_path = dir.join("tank.toml");

    let o = Command::new(bin)
        .args(["--gen-tank", toml_path.to_str().unwrap(), "--preset", "light", "--catalogue"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = std::fs::read_to_string(&toml_path).expect("the build file is written");
    assert!(text.contains("item = "), "a catalogue build names items");
    assert!(!text.contains("parts = "), "and needs no parts bin");

    let o = Command::new(bin)
        .env("APPDATA", &appdata)
        .args(["--build", toml_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(dir.join("tank.txt").exists(), "built beside the build file");
    assert!(ad2.join("tank.txt").exists(), "and copied where AdvDupe2 looks, from the config");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn json_output_stays_parseable_when_the_file_has_dangling_references() {
    let dir = std::env::temp_dir().join(format!("ad2read-json-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dupe = dir.join("dangling.txt");
    write_fixture_parented(&dupe, Some(99.0));
    let bin = env!("CARGO_BIN_EXE_ad2read");
    for flags in [vec!["--json"], vec!["--doctor", "--json"]] {
        let o = Command::new(bin).arg(dupe.to_str().unwrap()).args(&flags).output().unwrap();
        let stdout = String::from_utf8_lossy(&o.stdout);
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
        assert!(parsed.is_ok(), "{flags:?} printed something that is not JSON: {}", &stdout[..stdout.len().min(200)]);
        assert!(String::from_utf8_lossy(&o.stderr).contains("dangling"), "{flags:?} lost the note");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_verbs_take_a_tank_from_nothing_to_a_checked_dupe() {
    let dir = std::env::temp_dir().join(format!("ad2read-verbs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dupe = dir.join("tank.txt");
    let file = dupe.to_str().unwrap();
    let bin = env!("CARGO_BIN_EXE_ad2read");
    let run = |words: &[&str]| {
        let o = Command::new(bin).args(words).env("APPDATA", &dir).output().unwrap();
        (String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
    };

    let (out, err) = run(&["new", file, "--baseplate"]);
    assert!(out.contains("1 entities"), "{out}{err}");
    let (_, err) = run(&["new", file]);
    assert!(err.contains("never overwrites"), "{err}");

    let (out, err) = run(&["add", file, "engine", "0,-60,10"]);
    assert!(!err.contains("refus") && !out.contains("refus"), "{out}{err}");
    run(&["add", file, "gearbox", "0,-20,10"]);
    let (out, _) = run(&[file, "--list-entities"]);
    assert!(out.contains("acf_baseplate") && out.contains("acf_engine") && out.contains("acf_gearbox"), "{out}");

    let (out, _) = run(&["check", file]);
    assert!(out.to_lowercase().contains("doctor") || out.contains("warn") || out.contains("note"), "{out}");
    let (out, _) = run(&["explain", file, "2"]);
    assert!(out.contains("acf_engine"), "{out}");
    let (out, _) = run(&["undo", file]);
    assert!(out.contains("back to what it was"), "{out}");
    let (out, _) = run(&[file, "--list-entities"]);
    assert!(out.contains("acf_engine") && !out.contains("acf_gearbox"), "undo takes back the last add only: {out}");
    run(&["undo", file]);
    let (out, _) = run(&[file, "--list-entities"]);
    assert!(out.contains("acf_gearbox"), "a second undo is a redo: {out}");

    let (_, err) = run(&["add", file, "flux-capacitor"]);
    assert!(err.contains("neither a role nor a catalogue item"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
