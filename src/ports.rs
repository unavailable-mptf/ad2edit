//! Wire ports: what each entity class can be wired from and to. The
//! static lists are generated into `ports_table.rs` from the ACF-3 and
//! Wiremod Lua; the rules for ports that depend on the entity are here,
//! each with the file it was read from.

use crate::dupe::Dupe;
use crate::ports_table::{CLASSES, GATES};
use crate::transform;
use crate::value::Value;

/// Wiremod's port types, as `WireLib.DT` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireType {
    Normal,
    Vector,
    Vector2,
    Vector4,
    Angle,
    String,
    Entity,
    Array,
    Table,
    Wirelink,
    Ranger,
    Bone,
    Matrix,
    Matrix2,
    Matrix4,
    Complex,
    Quaternion,
    Any,
}

impl WireType {
    /// Reads a type as Wiremod or an E2 directive writes it. E2 calls a
    /// NORMAL `number`. Anything unrecognised is `None`, never a guess.
    pub fn parse(text: &str) -> Option<WireType> {
        Some(match text.trim().to_ascii_uppercase().as_str() {
            "NORMAL" | "NUMBER" => WireType::Normal,
            "VECTOR" => WireType::Vector,
            "VECTOR2" => WireType::Vector2,
            "VECTOR4" => WireType::Vector4,
            "ANGLE" => WireType::Angle,
            "STRING" => WireType::String,
            "ENTITY" => WireType::Entity,
            "ARRAY" => WireType::Array,
            "TABLE" => WireType::Table,
            "WIRELINK" => WireType::Wirelink,
            "RANGER" => WireType::Ranger,
            "BONE" => WireType::Bone,
            "MATRIX" => WireType::Matrix,
            "MATRIX2" => WireType::Matrix2,
            "MATRIX4" => WireType::Matrix4,
            "COMPLEX" => WireType::Complex,
            "QUATERNION" => WireType::Quaternion,
            "ANY" => WireType::Any,
            _ => return None,
        })
    }

    /// The name Wiremod itself uses, which is what a dupe stores.
    pub fn name(self) -> &'static str {
        match self {
            WireType::Normal => "NORMAL",
            WireType::Vector => "VECTOR",
            WireType::Vector2 => "VECTOR2",
            WireType::Vector4 => "VECTOR4",
            WireType::Angle => "ANGLE",
            WireType::String => "STRING",
            WireType::Entity => "ENTITY",
            WireType::Array => "ARRAY",
            WireType::Table => "TABLE",
            WireType::Wirelink => "WIRELINK",
            WireType::Ranger => "RANGER",
            WireType::Bone => "BONE",
            WireType::Matrix => "MATRIX",
            WireType::Matrix2 => "MATRIX2",
            WireType::Matrix4 => "MATRIX4",
            WireType::Complex => "COMPLEX",
            WireType::Quaternion => "QUATERNION",
            WireType::Any => "ANY",
        }
    }

    /// Whether Wiremod lets an output of this type drive an input of that
    /// one: the types match, or either side is ANY (wirelib.lua, Wire_Link).
    pub fn drives(self, input: WireType) -> bool {
        self == input || self == WireType::Any || input == WireType::Any
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    pub name: String,
    pub kind: WireType,
    pub note: String,
}

impl Port {
    /// Splits a port as Lua declares it, `Name (note) [TYPE]`, the way
    /// `ParsePortName` in wirelib.lua does: the type is the last bracket,
    /// the note starts at the last " (", and both are optional. The name
    /// must come out as Wiremod's does, since that is what a dupe stores.
    pub fn parse(declared: &str) -> Port {
        let declared = declared.trim();
        let (rest, kind) = match declared.strip_suffix(']').and_then(|s| s.rsplit_once(" [")) {
            Some((rest, kind)) => match WireType::parse(kind) {
                Some(kind) => (rest, kind),
                None => (declared, WireType::Normal),
            },
            None => (declared, WireType::Normal),
        };
        let (name, note) = match rest.strip_suffix(')').and_then(|s| s.rsplit_once(" (")) {
            Some((name, note)) => (name, note),
            None => (rest, ""),
        };
        Port { name: name.to_owned(), kind, note: note.replace('\n', " ") }
    }
}

/// One class in the generated table. `complete` is false when the class
/// adds or renames ports at run time in a way the Lua does not spell out
/// as a literal list, so an unknown port on it is not an error.
#[derive(Debug, Clone, Copy)]
pub struct ClassPorts {
    pub class: &'static str,
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
    pub complete: bool,
}

/// One gate action in the generated table, from wire/gates/*.lua.
#[derive(Debug, Clone, Copy)]
pub struct GatePorts {
    pub action: &'static str,
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ports {
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    pub complete: bool,
}

impl Ports {
    pub fn input(&self, name: &str) -> Option<&Port> {
        self.inputs.iter().find(|port| port.name == name)
    }

    pub fn output(&self, name: &str) -> Option<&Port> {
        self.outputs.iter().find(|port| port.name == name)
    }
}

/// The ports every entity of a class has, before any per-entity rule.
pub fn of_class(class: &str) -> Option<Ports> {
    let class = class.to_ascii_lowercase();
    let found = CLASSES.iter().find(|entry| entry.class == class)?;
    Some(Ports {
        inputs: found.inputs.iter().map(|declared| Port::parse(declared)).collect(),
        outputs: found.outputs.iter().map(|declared| Port::parse(declared)).collect(),
        complete: found.complete,
    })
}

/// The ports of one entity in a dupe: its class's list plus what ACF adds
/// for this particular gun, gearbox or turret.
pub fn of_entity(dupe: &Dupe, index: f64) -> Option<Ports> {
    let et = transform::entity_table(dupe, index)?;
    let text = |table: usize, key: &str| match dupe.get(table, key) {
        Some(Value::Str(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
        _ => String::new(),
    };
    let class = text(et, "Class").to_ascii_lowercase();
    if class == "gmod_wire_expression2" {
        return match crate::build::chip_export(dupe, index) {
            Ok(crate::build::Chip::E2 { code, .. }) => Some(of_chip(&code)),
            _ => of_class(&class),
        };
    }
    let truthy = |table: usize, key: &str| match dupe.get(table, key) {
        Some(Value::Bool(set)) => *set,
        Some(Value::Number(n)) => *n != 0.0,
        _ => false,
    };
    let listed = |inputs: &[&str], outputs: &[&str], complete: bool| Ports {
        inputs: inputs.iter().map(|declared| Port::parse(declared)).collect(),
        outputs: outputs.iter().map(|declared| Port::parse(declared)).collect(),
        complete,
    };
    match class.as_str() {
        // gmod_wire_gate.lua, ENT:Setup: the action's lists, or a lone "Out".
        "gmod_wire_gate" => {
            let action = text(et, "action");
            return Some(match GATES.iter().find(|gate| gate.action == action) {
                Some(gate) => listed(gate.inputs, gate.outputs, true),
                None => Ports::default(),
            });
        }
        // gmod_wire_value.lua, ENT:Setup: one output per stored value, named
        // by its position, typed by its DataType with NUMBER read as NORMAL.
        "gmod_wire_value" => {
            let mut ports = Ports { complete: true, ..Ports::default() };
            // AD2 writes a gapless list as an array and anything else keyed.
            let stored: Vec<&Value> = match dupe.get_table(et, "value").map(|values| &dupe.arena[values]) {
                Some(crate::value::Node::Array(items)) => items.iter().collect(),
                Some(crate::value::Node::Table(entries)) => entries.iter().map(|(_, item)| item).collect(),
                None => Vec::new(),
            };
            for (position, item) in stored.into_iter().enumerate() {
                let kind = crate::value::table_index(item)
                    .map(|item| text(item, "DataType"))
                    .and_then(|kind| WireType::parse(&kind))
                    .unwrap_or(WireType::String);
                ports.outputs.push(Port { name: (position + 1).to_string(), kind, note: String::new() });
            }
            return Some(ports);
        }
        // gmod_wire_ranger.lua, ENT:Setup: each out_ flag adds its group, and
        // RangerData is always there.
        "gmod_wire_ranger" => {
            const GROUPS: &[(&str, &[&str])] = &[
                ("out_dist", &["Dist"]),
                ("out_pos", &["Pos [VECTOR]", "Pos X", "Pos Y", "Pos Z"]),
                ("out_vel", &["Vel [VECTOR]", "Vel X", "Vel Y", "Vel Z"]),
                ("out_ang", &["Ang [ANGLE]", "Ang Pitch", "Ang Yaw", "Ang Roll"]),
                ("out_col", &["Col RGB [VECTOR]", "Col R", "Col G", "Col B", "Col A"]),
                ("out_val", &["Val", "ValSize"]),
                ("out_sid", &["SteamID [STRING]"]),
                ("out_uid", &["UniqueID"]),
                ("out_eid", &["EntID", "Entity [ENTITY]"]),
                ("out_hnrm", &["HitNormal [VECTOR]", "HitNormal X", "HitNormal Y", "HitNormal Z"]),
            ];
            let mut ports = of_class(&class).unwrap_or_default();
            ports.outputs.clear();
            for (flag, group) in GROUPS {
                if truthy(et, flag) {
                    ports.outputs.extend(group.iter().map(|declared| Port::parse(declared)));
                }
            }
            ports.outputs.push(Port::parse("RangerData [RANGER]"));
            ports.complete = true;
            return Some(ports);
        }
        // acf_controller/init.lua builds its outputs in a loop over
        // KEY_WIRE_BINDINGS, then adds ADDITIONAL_OUTPUTS.
        "acf_controller" => {
            return Some(listed(
                &["Filter (Filters out entities from the camera trace) [ARRAY]", "FLIR (Enables/disables FLIR while in the baseplate seat)"],
                &[
                    "W", "A", "S", "D", "Mouse1", "Mouse2", "R", "Space", "Shift", "Zoom", "Alt", "Duck",
                    "HitPos (The position the driver is looking at) [VECTOR]",
                    "CamAng (The direction of the camera.) [ANGLE]",
                    "IsTurretLocked (Whether the turret is locked or not.)",
                    "Active",
                    "Speed (Determined by selected unit)",
                    "Driver (The player driving the vehicle.) [ENTITY]",
                    "CamParent (The entity the camera is parented to) [ENTITY]",
                    "Entity (The controller entity itself) [ENTITY]",
                ],
                true,
            ));
        }
        _ => {}
    }
    let Some(mut ports) = of_class(&class) else {
        // The Fading Door tool gives any entity it is used on these two, in
        // its own stool file, which is in neither archive. Other addons may
        // add more, so the list is left open.
        let faded = dupe.get_table(et, "EntityMods").is_some_and(|mods| dupe.get_table(mods, "Fading Door").is_some());
        return faded.then(|| listed(&["Fade"], &["FadeActive"], false));
    };
    if class == "acf_gearbox" {
        // acf_gearbox/init.lua, ENT:ACF_SetupWireIO, and the SetupInputs and
        // SetupOutputs of each family in acf/entities/gearboxes/*.lua. Only
        // the -L and -T boxes of four families can be dual clutch, and a
        // double differential always is.
        let id = text(et, "Gearbox");
        let family = id.split('-').next().unwrap_or("");
        let chosen = truthy(et, "DualClutch") || dupe.get_table(et, "ACF_UserData").is_some_and(|ud| truthy(ud, "DualClutch"));
        let can_dual = matches!(family, "Auto" | "CVT" | "1Gear" | "Manual") && (id.ends_with("-L") || id.ends_with("-T"));
        let (more_inputs, more_outputs): (&[&str], &[&str]) = match family {
            "Auto" => (&["Hold Gear", "Shift Speed Scale"], &[]),
            "CVT" => (&["CVT Ratio"], &["Min Target RPM", "Max Target RPM"]),
            "DoubleDiff" => (&["Steer Rate"], &[]),
            _ => (&[], &[]),
        };
        let clutches: &[&str] = if family == "DoubleDiff" || (can_dual && chosen) {
            &["Left Clutch", "Right Clutch", "Left Brake", "Right Brake"]
        } else {
            &["Clutch", "Brake"]
        };
        ports.inputs.extend(more_inputs.iter().chain(clutches).map(|declared| Port::parse(declared)));
        ports.outputs.extend(more_outputs.iter().map(|declared| Port::parse(declared)));
        ports.complete = !id.is_empty();
        return Some(ports);
    }
    let mut input = |declared: &str| ports.inputs.push(Port::parse(declared));
    match class.as_str() {
        // acf_gun/init.lua, ENT:ACF_SetupWireIO; ACF.MinFuzeCaliber in globals.lua.
        "acf_gun" => {
            if dupe.get_number(et, "Caliber").unwrap_or(0.0) >= crate::rules::MIN_FUZE_CALIBER {
                input("Fuze (Sets the delay in seconds in which explosive rounds will detonate after leaving the weapon.)");
            }
            input("Rate of Fire (Sets the rate of fire of the weapon in rounds per minute)");
        }
        // acf/entities/turrets/turrets.lua, CLASS.SetupInputs per turret type.
        "acf_turret" => match text(et, "Turret").as_str() {
            "Turret-H" => input("Bearing (Local degrees from home angle)"),
            "Turret-V" => input("Elevation (Local degrees from home angle)"),
            _ => {}
        },
        _ => {}
    }
    Some(ports)
}

/// The ports an Expression 2 declares in its `@inputs` and `@outputs`
/// directives: `Name`, `Name:type`, or `[A B]:type` for a group. A type
/// this file does not know leaves the result marked incomplete.
pub fn of_chip(code: &str) -> Ports {
    let mut ports = Ports { complete: true, ..Ports::default() };
    for line in code.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let (side, rest) = if let Some(rest) = line.strip_prefix("@inputs") {
            (&mut ports.inputs, rest)
        } else if let Some(rest) = line.strip_prefix("@outputs") {
            (&mut ports.outputs, rest)
        } else {
            continue;
        };
        let mut rest = rest.trim();
        while !rest.is_empty() {
            let (names, after) = match rest.strip_prefix('[') {
                Some(group) => match group.split_once(']') {
                    Some((names, after)) => (names, after),
                    None => break,
                },
                None => {
                    let end = rest.find(|c: char| c == ':' || c.is_whitespace()).unwrap_or(rest.len());
                    rest.split_at(end)
                }
            };
            let (kind, after) = match after.strip_prefix(':') {
                Some(typed) => {
                    let end = typed.find(char::is_whitespace).unwrap_or(typed.len());
                    let (kind, after) = typed.split_at(end);
                    let kind = WireType::parse(kind).unwrap_or_else(|| {
                        ports.complete = false;
                        WireType::Normal
                    });
                    (kind, after)
                }
                None => (WireType::Normal, after),
            };
            for name in names.split_whitespace() {
                side.push(Port { name: name.to_owned(), kind, note: String::new() });
            }
            rest = after.trim_start();
        }
    }
    ports
}

#[derive(Debug, Clone, PartialEq)]
pub enum WireProblem {
    /// The wired entity's class has no input of this name.
    UnknownInput,
    /// The source's class has no output of this name.
    UnknownOutput,
    /// Wiremod refuses to connect these two types.
    TypeMismatch { output: WireType, input: WireType },
}

#[derive(Debug, Clone, PartialEq)]
pub struct WireFault {
    pub entity: f64,
    pub input: String,
    pub source: f64,
    pub output: String,
    pub problem: WireProblem,
}

/// Every wire in the dupe that names a port its entity cannot have, or
/// joins two types Wiremod would refuse. Classes whose ports are not fully
/// known are given the benefit of the doubt, and so is a source outside
/// the dupe.
pub fn wire_faults(dupe: &Dupe) -> Vec<WireFault> {
    let mut faults = Vec::new();
    for (index, _, _) in dupe.list_entities() {
        let Some(wires) = transform::entity_table(dupe, index)
            .and_then(|et| dupe.get_table(et, "EntityMods"))
            .and_then(|mods| dupe.get_table(mods, "WireDupeInfo"))
            .and_then(|info| dupe.get_table(info, "Wires"))
        else {
            continue;
        };
        let crate::value::Node::Table(entries) = &dupe.arena[wires] else {
            continue;
        };
        let here = of_entity(dupe, index);
        for (key, value) in entries {
            let (Value::Str(input), Some(wire)) = (key, crate::value::table_index(value)) else {
                continue;
            };
            let input = String::from_utf8_lossy(input).into_owned();
            let source = dupe.get_number(wire, "Src").unwrap_or(-1.0);
            let output = match dupe.get(wire, "SrcId") {
                Some(Value::Str(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
                _ => String::new(),
            };
            let mut fault = |problem| {
                faults.push(WireFault { entity: index, input: input.clone(), source, output: output.clone(), problem })
            };
            let input_port = here.as_ref().and_then(|ports| ports.input(&input).cloned());
            if here.as_ref().is_some_and(|ports| ports.complete) && input_port.is_none() {
                fault(WireProblem::UnknownInput);
            }
            let there = of_entity(dupe, source);
            // WireLib creates these two outputs on any entity the moment
            // something is wired to them (CreateWirelinkOutput, CreateEntityOutput).
            let output_port = match output.as_str() {
                "wirelink" => Some(Port { name: output.clone(), kind: WireType::Wirelink, note: String::new() }),
                "entity" => Some(Port { name: output.clone(), kind: WireType::Entity, note: String::new() }),
                _ => there.as_ref().and_then(|ports| ports.output(&output).cloned()),
            };
            if there.as_ref().is_some_and(|ports| ports.complete) && output_port.is_none() {
                fault(WireProblem::UnknownOutput);
            }
            if let (Some(from), Some(to)) = (output_port, input_port) {
                if !from.kind.drives(to.kind) {
                    fault(WireProblem::TypeMismatch { output: from.kind, input: to.kind });
                }
            }
        }
    }
    faults
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Node;

    fn dupe_with(class: &str, fields: &[(&str, Value)]) -> (Dupe, f64) {
        let mut d = crate::buildfile::empty_dupe();
        let root = d.root_table().unwrap();
        let ents = d.get_table(root, "Entities").unwrap();
        let et = d.new_table();
        d.set(et, "Class", Value::Str(class.as_bytes().to_vec()));
        for (k, v) in fields {
            d.set(et, k, v.clone());
        }
        if let Node::Table(e) = &mut d.arena[ents] {
            e.push((Value::Number(7.0), Value::Table(et)));
        }
        (d, 7.0)
    }

    #[test]
    fn a_port_is_read_the_way_wiremod_reads_it() {
        let port = Port::parse("Status (Returns the current state of the weapon.) [STRING]");
        assert_eq!(port.name, "Status");
        assert_eq!(port.kind, WireType::String);
        assert_eq!(port.note, "Returns the current state of the weapon.");

        let port = Port::parse("Fire");
        assert_eq!((port.name.as_str(), port.kind, port.note.as_str()), ("Fire", WireType::Normal, ""));

        let port = Port::parse("Shots Left (Rounds left in the breech.)");
        assert_eq!((port.name.as_str(), port.kind), ("Shots Left", WireType::Normal));

        let port = Port::parse("Target [VECTOR]");
        assert_eq!((port.name.as_str(), port.kind), ("Target", WireType::Vector));

        let port = Port::parse("Odd [NOTATYPE]");
        assert_eq!((port.name.as_str(), port.kind), ("Odd [NOTATYPE]", WireType::Normal));
    }

    #[test]
    fn types_match_or_one_side_is_any() {
        assert!(WireType::Normal.drives(WireType::Normal));
        assert!(!WireType::Normal.drives(WireType::Vector));
        assert!(WireType::Entity.drives(WireType::Any));
        assert!(WireType::Any.drives(WireType::String));
        assert_eq!(WireType::parse("number"), Some(WireType::Normal));
        assert_eq!(WireType::parse("wirelink"), Some(WireType::Wirelink));
        assert_eq!(WireType::parse("nonsense"), None);
    }

    #[test]
    fn the_table_knows_acf_and_wiremod_classes() {
        let gun = of_class("acf_gun").expect("acf_gun");
        assert!(gun.input("Fire").is_some());
        assert_eq!(gun.output("Status").map(|port| port.kind), Some(WireType::String));
        assert_eq!(gun.output("Entity").map(|port| port.kind), Some(WireType::Entity));

        let engine = of_class("acf_engine").expect("acf_engine");
        assert!(engine.input("Throttle").is_some());
        assert!(engine.output("RPM").is_some());

        let camera = of_class("gmod_wire_cameracontroller").expect("camera controller");
        assert!(camera.input("Activated").is_some());
        assert_eq!(camera.input("Position").map(|port| port.kind), Some(WireType::Vector));

        assert!(of_class("prop_physics").is_none());
    }

    #[test]
    fn every_declared_port_in_the_table_parses_to_a_name() {
        for entry in CLASSES {
            for declared in entry.inputs.iter().chain(entry.outputs) {
                let port = Port::parse(declared);
                assert!(!port.name.is_empty(), "{}: {declared:?}", entry.class);
                assert!(!port.name.contains('['), "{}: {declared:?}", entry.class);
            }
        }
    }

    #[test]
    fn a_gun_gains_its_fuze_by_calibre_and_a_turret_its_axis_by_type() {
        let (d, i) = dupe_with("acf_gun", &[("Caliber", Value::Number(100.0))]);
        let ports = of_entity(&d, i).unwrap();
        assert!(ports.input("Fuze").is_some());
        assert!(ports.input("Rate of Fire").is_some());

        let (d, i) = dupe_with("acf_gun", &[("Caliber", Value::Number(20.0))]);
        let ports = of_entity(&d, i).unwrap();
        assert!(ports.input("Fuze").is_none());
        assert!(ports.input("Rate of Fire").is_some());

        let (d, i) = dupe_with("acf_turret", &[("Turret", Value::Str(b"Turret-H".to_vec()))]);
        let ports = of_entity(&d, i).unwrap();
        assert!(ports.input("Bearing").is_some() && ports.input("Elevation").is_none());

        let (d, i) = dupe_with("acf_turret", &[("Turret", Value::Str(b"Turret-V".to_vec()))]);
        let ports = of_entity(&d, i).unwrap();
        assert!(ports.input("Elevation").is_some() && ports.input("Bearing").is_none());

        assert!(of_entity(&d, 99.0).is_none());
    }

    #[test]
    fn a_chip_declares_its_ports_in_its_directives() {
        let code = "@name Flight\n@inputs Pod:wirelink [Target Home]:vector Active # the switch\n@outputs Thrust Aim:angle\n@persist Speed\nThrust = Active * 10\n";
        let ports = of_chip(code);
        assert!(ports.complete);
        let names: Vec<&str> = ports.inputs.iter().map(|port| port.name.as_str()).collect();
        assert_eq!(names, ["Pod", "Target", "Home", "Active"]);
        assert_eq!(ports.input("Pod").map(|port| port.kind), Some(WireType::Wirelink));
        assert_eq!(ports.input("Home").map(|port| port.kind), Some(WireType::Vector));
        assert_eq!(ports.input("Active").map(|port| port.kind), Some(WireType::Normal));
        assert_eq!(ports.output("Aim").map(|port| port.kind), Some(WireType::Angle));
        assert!(ports.output("Speed").is_none());

        assert!(!of_chip("@inputs Odd:gtable").complete);
    }

    fn wired(input: &str, output: &str, source_class: &str, source_fields: &[(&str, Value)]) -> Dupe {
        let (mut d, gun) = dupe_with("acf_gun", &[("Caliber", Value::Number(100.0))]);
        let root = d.root_table().unwrap();
        let ents = d.get_table(root, "Entities").unwrap();
        let source = d.new_table();
        d.set(source, "Class", Value::Str(source_class.as_bytes().to_vec()));
        for (k, v) in source_fields {
            d.set(source, k, v.clone());
        }
        if let Node::Table(e) = &mut d.arena[ents] {
            e.push((Value::Number(8.0), Value::Table(source)));
        }
        crate::build::add_wire(&mut d, 8.0, output, gun, input).unwrap();
        d
    }

    #[test]
    fn a_wire_to_a_port_that_does_not_exist_is_a_fault() {
        assert!(wire_faults(&wired("Fire", "Mouse1", "gmod_wire_pod", &[])).is_empty());

        let faults = wire_faults(&wired("Fier", "Mouse1", "gmod_wire_pod", &[]));
        assert_eq!(faults.len(), 1);
        assert_eq!(faults[0].problem, WireProblem::UnknownInput);
        assert_eq!((faults[0].entity, faults[0].source), (7.0, 8.0));

        let faults = wire_faults(&wired("Fire", "Mouse9", "gmod_wire_pod", &[]));
        assert_eq!(faults.iter().map(|fault| &fault.problem).collect::<Vec<_>>(), [&WireProblem::UnknownOutput]);
    }

    #[test]
    fn the_doctor_warns_about_a_lost_wire_and_never_calls_it_an_error() {
        use crate::doctor::{doctor, Severity};
        let about_wires = |dupe: &Dupe| -> Vec<Severity> {
            doctor(dupe).into_iter().filter(|finding| finding.what.starts_with("wire ")).map(|finding| finding.severity).collect()
        };
        assert!(about_wires(&wired("Fire", "Mouse1", "gmod_wire_pod", &[])).is_empty());
        assert_eq!(about_wires(&wired("Fier", "Mouse1", "gmod_wire_pod", &[])), [Severity::Warn]);
    }

    #[test]
    fn a_wire_between_two_types_is_a_fault_and_a_chip_is_read_from_its_code() {
        let chip = |code: &str| [("_original", Value::Str(code.replace('\n', "\u{80}").into_bytes()))];
        assert!(wire_faults(&wired("Fire", "Shoot", "gmod_wire_expression2", &chip("@outputs Shoot"))).is_empty());

        let faults = wire_faults(&wired("Fire", "Aim", "gmod_wire_expression2", &chip("@outputs Aim:vector")));
        assert_eq!(faults[0].problem, WireProblem::TypeMismatch { output: WireType::Vector, input: WireType::Normal });

        let faults = wire_faults(&wired("Fire", "Gone", "gmod_wire_expression2", &chip("@outputs Shoot")));
        assert_eq!(faults[0].problem, WireProblem::UnknownOutput);
    }

    #[test]
    fn a_gate_takes_its_ports_from_its_action() {
        let (d, i) = dupe_with("gmod_wire_gate", &[("action", Value::Str(b"entity_pos".to_vec()))]);
        let ports = of_entity(&d, i).unwrap();
        assert!(ports.complete);
        assert_eq!(ports.input("Ent").map(|port| port.kind), Some(WireType::Entity));
        assert_eq!(ports.output("Out").map(|port| port.kind), Some(WireType::Vector));

        let (d, i) = dupe_with("gmod_wire_gate", &[("action", Value::Str(b"-".to_vec()))]);
        let ports = of_entity(&d, i).unwrap();
        assert!(ports.input("A").is_some() && ports.input("B").is_some() && ports.output("Out").is_some());

        let (d, i) = dupe_with("gmod_wire_gate", &[("action", Value::Str(b"from another addon".to_vec()))]);
        assert!(!of_entity(&d, i).unwrap().complete);
    }

    #[test]
    fn a_gearbox_gains_its_family_ports_and_the_right_clutches() {
        let names = |ports: &Ports| ports.inputs.iter().map(|port| port.name.clone()).collect::<Vec<_>>();
        let (d, i) = dupe_with("acf_gearbox", &[("Gearbox", Value::Str(b"CVT-T".to_vec()))]);
        let ports = of_entity(&d, i).unwrap();
        assert!(ports.complete);
        assert!(names(&ports).contains(&"CVT Ratio".to_owned()) && names(&ports).contains(&"Clutch".to_owned()));
        assert!(ports.output("Max Target RPM").is_some() && ports.input("Left Clutch").is_none());

        let (d, i) = dupe_with("acf_gearbox", &[("Gearbox", Value::Str(b"2Gear-L".to_vec())), ("DualClutch", Value::Bool(true))]);
        let ports = of_entity(&d, i).unwrap();
        assert!(ports.input("Brake").is_some() && ports.input("Left Brake").is_none(), "a transfer box cannot dual clutch");

        let (d, i) = dupe_with("acf_gearbox", &[("Gearbox", Value::Str(b"Manual-L".to_vec())), ("DualClutch", Value::Bool(true))]);
        assert!(of_entity(&d, i).unwrap().input("Left Brake").is_some());

        let (d, i) = dupe_with("acf_gearbox", &[("Gearbox", Value::Str(b"DoubleDiff-T".to_vec()))]);
        let ports = of_entity(&d, i).unwrap();
        assert!(ports.input("Steer Rate").is_some() && ports.input("Right Clutch").is_some());
    }

    #[test]
    fn the_controller_ranger_value_chip_and_fading_door_are_known() {
        let (d, i) = dupe_with("acf_controller", &[]);
        let ports = of_entity(&d, i).unwrap();
        assert!(ports.complete && ports.output("Mouse1").is_some());
        assert_eq!(ports.output("HitPos").map(|port| port.kind), Some(WireType::Vector));

        let (d, i) = dupe_with("gmod_wire_ranger", &[("out_pos", Value::Bool(true))]);
        let ports = of_entity(&d, i).unwrap();
        assert!(ports.output("Pos X").is_some() && ports.output("Dist").is_none());
        assert_eq!(ports.output("RangerData").map(|port| port.kind), Some(WireType::Ranger));

        let (mut d, i) = dupe_with("gmod_wire_value", &[]);
        let et = transform::entity_table(&d, i).unwrap();
        let values = d.new_array();
        for kind in ["NORMAL", "VECTOR"] {
            let stored = d.new_table();
            d.set(stored, "DataType", Value::Str(kind.as_bytes().to_vec()));
            if let Node::Array(items) = &mut d.arena[values] {
                items.push(Value::Table(stored));
            }
        }
        d.set(et, "value", Value::Table(values));
        let ports = of_entity(&d, i).unwrap();
        assert_eq!(ports.output("2").map(|port| port.kind), Some(WireType::Vector));
        assert!(ports.output("3").is_none());

        let (mut d, i) = dupe_with("prop_physics", &[]);
        assert!(of_entity(&d, i).is_none());
        let et = transform::entity_table(&d, i).unwrap();
        let (mods, door) = (d.new_table(), d.new_table());
        d.set(mods, "Fading Door", Value::Table(door));
        d.set(et, "EntityMods", Value::Table(mods));
        assert!(of_entity(&d, i).unwrap().input("Fade").is_some());
    }
}
