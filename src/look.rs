//! What an entity looks like in the game, as far as a dupe and ACF's own
//! code decide it: the Colour tool's colour, the Material tool's material
//! or the one ACF gives the class when it spawns, and whether the builder
//! made it invisible.

use crate::dupe::Dupe;
use crate::rules;
use crate::transform;
use crate::value::Value;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Look {
    /// The Colour tool's colour, alpha included.
    pub colour: Option<[u8; 4]>,
    /// The material the whole model is drawn with, when it is not its own.
    pub material: Option<String>,
    /// Why the game shows nothing solid here, when it does not. The editor
    /// still has to let the part be found and selected.
    pub hidden: Option<&'static str>,
    /// The model's skin, from the duplicator's `Skin`.
    pub skin: usize,
    /// The chosen model of each body part, from the duplicator's `BodyG`.
    pub bodygroups: Vec<usize>,
    /// The SubMaterial tool's overrides: material slot and material.
    pub submaterials: Vec<(usize, String)>,
}

/// Below this the Colour tool's alpha reads as "off" in the game.
const INVISIBLE_ALPHA: u8 = 8;

pub fn of_entity(dupe: &Dupe, index: f64) -> Look {
    let Some(et) = transform::entity_table(dupe, index) else { return Look::default() };
    let class = match dupe.get(et, "Class") {
        Some(Value::Str(bytes)) => String::from_utf8_lossy(bytes).to_ascii_lowercase(),
        _ => String::new(),
    };
    let mods = dupe.get_table(et, "EntityMods");
    let colour = mods
        .and_then(|mods| dupe.get_table(mods, "colour"))
        .and_then(|colour| dupe.get_table(colour, "Color"))
        .map(|table| {
            let channel = |key: &str| dupe.get_number(table, key).unwrap_or(255.0).clamp(0.0, 255.0) as u8;
            [channel("r"), channel("g"), channel("b"), channel("a")]
        });
    // AD2 applies the dupe's material after the entity has spawned, so an
    // override beats whatever the class gave itself.
    let overridden = mods.and_then(|mods| dupe.get_table(mods, "material")).and_then(|material| {
        match dupe.get(material, "MaterialOverride") {
            Some(Value::Str(bytes)) if !bytes.is_empty() => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        }
    });
    let material = overridden.or_else(|| rules::spawn_material(&class).map(str::to_owned));
    let hidden = match colour {
        Some([_, _, _, alpha]) if alpha < INVISIBLE_ALPHA => Some("its colour's alpha is 0"),
        _ => None,
    };
    let skin = dupe.get_number(et, "Skin").map_or(0, |skin| skin.max(0.0) as usize);
    let mut bodygroups = Vec::new();
    if let Some(crate::value::Node::Table(entries)) = dupe.get_table(et, "BodyG").map(|chosen| &dupe.arena[chosen]) {
        for (part, chosen) in entries {
            if let (Value::Number(part), Value::Number(chosen)) = (part, chosen) {
                let part = part.max(0.0) as usize;
                if part < 64 {
                    if bodygroups.len() <= part {
                        bodygroups.resize(part + 1, 0);
                    }
                    bodygroups[part] = chosen.max(0.0) as usize;
                }
            }
        }
    }
    let mut submaterials = Vec::new();
    if let Some(crate::value::Node::Table(entries)) = mods.and_then(|mods| dupe.get_table(mods, "submaterial")).map(|table| &dupe.arena[table]) {
        for (key, value) in entries {
            let (Value::Str(key), Value::Str(material)) = (key, value) else { continue };
            let key = String::from_utf8_lossy(key);
            let Some(slot) = key.strip_prefix("SubMaterialOverride_").and_then(|slot| slot.parse::<usize>().ok()) else { continue };
            if !material.is_empty() {
                submaterials.push((slot, String::from_utf8_lossy(material).into_owned()));
            }
        }
    }
    Look { colour, material, hidden, skin, bodygroups, submaterials }
}

/// A key's value in a VMT, skipping commented-out lines, which is how
/// material authors leave an option switched off.
fn setting(vmt: &str, key: &str) -> Option<String> {
    vmt.lines().find_map(|line| {
        let line = line.trim().to_ascii_lowercase();
        if line.starts_with("//") {
            return None;
        }
        let mut words = line.split(|c: char| c.is_whitespace() || c == '"').filter(|word| !word.is_empty());
        (words.next() == Some(key)).then(|| words.next().unwrap_or("").to_owned())
    })
}

fn switched_on(vmt: &str, key: &str) -> bool {
    setting(vmt, key).is_some_and(|value| !value.is_empty() && value != "0")
}

/// Whether a material draws nothing solid: additive ones only ever add
/// light (the light-volume materials builders use to hide a part), and a
/// no-draw material draws nothing at all.
pub fn material_hides(vmt: &str) -> bool {
    switched_on(vmt, "$additive") || switched_on(vmt, "%compilenodraw") || switched_on(vmt, "$no_draw")
}

/// What a material does with its texture's alpha channel. Most ignore it:
/// the channel often holds a gloss mask, and treating that as transparency
/// punches holes in solid metal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaUse {
    Opaque,
    /// `$alphatest`: a texel is drawn or not, by this threshold in 0..1.
    Cutout(f32),
    /// `$translucent`: the texel is blended over what is behind it.
    Blended,
}

pub fn alpha_use(vmt: &str) -> AlphaUse {
    if switched_on(vmt, "$translucent") {
        AlphaUse::Blended
    } else if switched_on(vmt, "$alphatest") {
        // Source's own default when the material gives no reference.
        let threshold = setting(vmt, "$alphatestreference").and_then(|value| value.parse::<f32>().ok()).unwrap_or(0.7);
        AlphaUse::Cutout(threshold.clamp(0.0, 1.0))
    } else {
        AlphaUse::Opaque
    }
}

/// Rewrites a texture's alpha channel to what the renderer should honour:
/// solid for an opaque material, on or off for a cut-out, untouched for a
/// blended one. True when some alpha below solid remains.
pub fn prepare_alpha(rgba: &mut [u8], alpha: AlphaUse) -> bool {
    let mut see_through = false;
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = match alpha {
            AlphaUse::Opaque => 255,
            AlphaUse::Cutout(threshold) => {
                if pixel[3] as f32 / 255.0 >= threshold { 255 } else { 0 }
            }
            AlphaUse::Blended => pixel[3],
        };
        see_through |= pixel[3] < 255;
    }
    see_through
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Node;

    fn entity(class: &str, colour: Option<[f64; 4]>, material: Option<&str>) -> (Dupe, f64) {
        let mut d = crate::buildfile::empty_dupe();
        let root = d.root_table().unwrap();
        let ents = d.get_table(root, "Entities").unwrap();
        let et = d.new_table();
        d.set(et, "Class", Value::Str(class.as_bytes().to_vec()));
        let mods = d.new_table();
        if let Some([r, g, b, a]) = colour {
            let (outer, inner) = (d.new_table(), d.new_table());
            for (key, value) in [("r", r), ("g", g), ("b", b), ("a", a)] {
                d.set(inner, key, Value::Number(value));
            }
            d.set(outer, "Color", Value::Table(inner));
            d.set(mods, "colour", Value::Table(outer));
        }
        if let Some(material) = material {
            let table = d.new_table();
            d.set(table, "MaterialOverride", Value::Str(material.as_bytes().to_vec()));
            d.set(mods, "material", Value::Table(table));
        }
        d.set(et, "EntityMods", Value::Table(mods));
        if let Node::Table(e) = &mut d.arena[ents] {
            e.push((Value::Number(7.0), Value::Table(et)));
        }
        (d, 7.0)
    }

    #[test]
    fn acf_classes_wear_what_acf_gives_them_unless_the_dupe_says_otherwise() {
        let (d, i) = entity("acf_ammo", None, None);
        assert_eq!(of_entity(&d, i).material.as_deref(), Some("phoenix_storms/Future_vents"));
        let (d, i) = entity("acf_fueltank", None, None);
        assert_eq!(of_entity(&d, i).material.as_deref(), Some("models/props_canal/metalcrate001d"));
        let (d, i) = entity("acf_baseplate", None, None);
        assert_eq!(of_entity(&d, i).material.as_deref(), Some("hunter/myplastic"));
        let (d, i) = entity("acf_ammo", None, Some("models/props_lab/door_klab01"));
        assert_eq!(of_entity(&d, i).material.as_deref(), Some("models/props_lab/door_klab01"));
        let (d, i) = entity("acf_gun", None, None);
        assert_eq!(of_entity(&d, i), Look::default());
        assert_eq!(of_entity(&d, 99.0), Look::default());
    }

    #[test]
    fn a_colour_keeps_its_alpha_and_alpha_zero_hides_the_part() {
        let (d, i) = entity("prop_physics", Some([72.0, 72.0, 72.0, 0.0]), None);
        let look = of_entity(&d, i);
        assert_eq!(look.colour, Some([72, 72, 72, 0]));
        assert!(look.hidden.is_some());
        let (d, i) = entity("prop_physics", Some([94.0, 115.0, 140.0, 255.0]), None);
        assert!(of_entity(&d, i).hidden.is_none());
    }

    #[test]
    fn additive_and_nodraw_materials_draw_nothing_solid() {
        let light_volume = "\"UnlitGeneric\"\n{\n\t\"$baseTexture\" \"Models/Effects/vol_light001\"\n\t\"$nocull\" \"1\"\t\n\t\"$additive\" \"1\"\n//\t\"$translucent\" \"1\"\n}\n";
        assert!(material_hides(light_volume));
        assert!(!material_hides("\"VertexLitGeneric\"\n{\n\t\"$basetexture\" \"Models/props_c17/metalladder003\"\n}\n"));
        assert!(!material_hides("\"VertexLitGeneric\"\n{\n//\t\"$additive\" \"1\"\n\t\"$additive\" \"0\"\n}\n"));
        assert!(material_hides("LightmappedGeneric\n{\n\t%compilenodraw 1\n}\n"));
    }

    #[test]
    fn alpha_is_honoured_only_when_the_material_asks_for_it() {
        assert_eq!(alpha_use("\"VertexLitGeneric\"\n{\n\t\"$basetexture\" \"x\"\n}\n"), AlphaUse::Opaque);
        assert_eq!(alpha_use("VertexLitGeneric\n{\n\t$translucent 1\n}\n"), AlphaUse::Blended);
        assert_eq!(alpha_use("VertexLitGeneric\n{\n\t\"$alphatest\" \"1\"\n\t\"$alphatestreference\" \".3\"\n}\n"), AlphaUse::Cutout(0.3));
        assert_eq!(alpha_use("VertexLitGeneric\n{\n\t$alphatest 1\n}\n"), AlphaUse::Cutout(0.7));
        assert_eq!(alpha_use("UnlitGeneric\n{\n//\t\"$translucent\" \"1\"\n}\n"), AlphaUse::Opaque);

        let mut gloss_mask = vec![10, 20, 30, 0, 40, 50, 60, 200];
        assert!(!prepare_alpha(&mut gloss_mask, AlphaUse::Opaque));
        assert_eq!(gloss_mask, [10, 20, 30, 255, 40, 50, 60, 255]);
        let mut grate = vec![1, 1, 1, 100, 2, 2, 2, 200];
        assert!(prepare_alpha(&mut grate, AlphaUse::Cutout(0.5)));
        assert_eq!((grate[3], grate[7]), (0, 255));
        let mut glass = vec![1, 1, 1, 90];
        assert!(prepare_alpha(&mut glass, AlphaUse::Blended));
        assert_eq!(glass[3], 90);
    }

    #[test]
    fn a_dupe_carries_its_skin_bodygroups_and_submaterials() {
        let (mut d, i) = entity("acf_gun", None, None);
        let et = transform::entity_table(&d, i).unwrap();
        d.set(et, "Skin", Value::Number(2.0));
        let chosen = d.new_table();
        if let Node::Table(entries) = &mut d.arena[chosen] {
            entries.push((Value::Number(2.0), Value::Number(1.0)));
        }
        d.set(et, "BodyG", Value::Table(chosen));
        let mods = d.get_table(et, "EntityMods").unwrap();
        let overrides = d.new_table();
        d.set(overrides, "SubMaterialOverride_3", Value::Str(b"sprops/sprops_plastic".to_vec()));
        d.set(overrides, "SubMaterialOverride_0", Value::Str(Vec::new()));
        d.set(mods, "submaterial", Value::Table(overrides));

        let look = of_entity(&d, i);
        assert_eq!(look.skin, 2);
        assert_eq!(look.bodygroups, [0, 0, 1]);
        assert_eq!(look.submaterials, [(3, "sprops/sprops_plastic".to_owned())]);
    }
}
