//! SProps as something to pick from: the spawn menu's lists and headings
//! over the generated table, a readable name for a model, and a search.
//! A model's name says its size (`rect_24x48x3`, `tube_36x96`), so the
//! label is the name with its underscores read the way SProps means them.

use crate::sprops_table::{Group, GROUPS};

/// The spawn menu's lists in its own order: "Blocks", "Plates - Normal", ...
pub fn lists() -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Vec::new();
    for group in GROUPS {
        if found.last() != Some(&group.list) {
            found.push(group.list);
        }
    }
    found
}

/// The headings inside one list, in order.
pub fn headers(list: &str) -> Vec<&'static Group> {
    GROUPS.iter().filter(|group| group.list == list).collect()
}

/// `rectangles/size_3/rect_24x48x3` as the game wants it.
pub fn model_path(name: &str) -> String {
    format!("models/sprops/{name}.mdl")
}

/// `rect_24x48x3` as "rect 24 x 48 x 3", `rect_1_5x108x1_5` as "rect 1.5 x
/// 108 x 1.5": the folders dropped, SProps' `_5` read as a half.
pub fn label(name: &str) -> String {
    let file = name.rsplit('/').next().unwrap_or(name);
    let mut halves = String::new();
    let bytes: Vec<char> = file.chars().collect();
    let mut at = 0;
    while at < bytes.len() {
        let half = bytes[at] == '_'
            && at > 0
            && bytes[at - 1].is_ascii_digit()
            && bytes.get(at + 1) == Some(&'5')
            && !bytes.get(at + 2).is_some_and(|next| next.is_ascii_digit());
        if half {
            halves.push_str(".5");
            at += 2;
        } else {
            halves.push(bytes[at]);
            at += 1;
        }
    }
    let mut spaced = String::new();
    let letters: Vec<char> = halves.chars().collect();
    for (n, c) in letters.iter().enumerate() {
        let between_numbers = *c == 'x' && n > 0 && letters[n - 1].is_ascii_digit() && letters.get(n + 1).is_some_and(|next| next.is_ascii_digit());
        match c {
            '_' => spaced.push(' '),
            'x' if between_numbers => spaced.push_str(" x "),
            _ => spaced.push(*c),
        }
    }
    spaced
}

/// Every model whose list, heading or name holds every word of `query`,
/// with its group. A size can be typed as it is said: "plate 24 48".
pub fn search(query: &str, limit: usize) -> Vec<(&'static Group, &'static str)> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_owned).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for group in GROUPS {
        let about = format!("{} {}", group.list, group.header).to_lowercase();
        for name in group.models {
            let said = format!("{about} {} {}", name, label(name));
            if words.iter().all(|word| said.contains(word.as_str())) {
                found.push((group, *name));
                if found.len() >= limit {
                    return found;
                }
            }
        }
    }
    found
}

pub fn count() -> usize {
    GROUPS.iter().map(|group| group.models.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_the_whole_pack_once_each() {
        assert_eq!(count(), 4457);
        let mut names: Vec<&str> = GROUPS.iter().flat_map(|group| group.models.iter().copied()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "a model is listed twice");
        assert!(names.iter().all(|name| !name.starts_with("models/") && !name.ends_with(".mdl") && *name == name.to_lowercase()));
    }

    #[test]
    fn the_lists_are_the_spawn_menus_in_its_order() {
        let found = lists();
        assert_eq!(&found[..4], ["Blocks", "Plates - Normal", "Plates - Thin", "Plates - Superthin"]);
        assert_eq!(found.len(), 17, "sixteen spawn lists and the models none of them names");
        let plates = headers("Plates - Normal");
        assert_eq!(plates[0].header, "Plates (3)");
        assert!(plates.iter().any(|group| group.header == "Plates (24)" && group.models.contains(&"rectangles/size_3/rect_24x48x3")));
    }

    #[test]
    fn names_read_as_sizes() {
        assert_eq!(label("rectangles/size_3/rect_24x48x3"), "rect 24 x 48 x 3");
        assert_eq!(label("rectangles_thin/size_0/rect_1_5x108x1_5"), "rect 1.5 x 108 x 1.5");
        assert_eq!(label("misc/tubes/size_5/h_tube_96x108"), "h tube 96 x 108");
        assert_eq!(label("trans/miscwheels/tank15"), "tank15");
        assert_eq!(label("mechanics/sgears/spur_96t_s"), "spur 96t s");
        assert_eq!(label("misc/fittings/t_fitting_6_to_3"), "t fitting 6 to 3");
        assert_eq!(model_path("cylinders/size_3/cylinder_6x384"), "models/sprops/cylinders/size_3/cylinder_6x384.mdl");
    }

    #[test]
    fn a_search_takes_a_size_the_way_it_is_said() {
        let found = search("plates normal 24 48", 50);
        assert!(found.iter().any(|(_, name)| *name == "rectangles/size_3/rect_24x48x3"), "{found:?}");
        assert!(search("tank 30", 10).iter().any(|(_, name)| name.ends_with("tank30")));
        assert!(search("", 10).is_empty());
        assert_eq!(search("rect", 7).len(), 7);
    }

    #[test]
    fn everything_the_hull_builder_and_the_catalogue_name_is_in_the_pack() {
        let all: std::collections::HashSet<String> = GROUPS.iter().flat_map(|group| group.models.iter().map(|name| model_path(name))).collect();
        for plate in crate::hull::plate_models().into_iter().chain(crate::hull::triangle_models()) {
            assert!(all.contains(&plate), "{plate}");
        }
        for item in crate::catalog::ITEMS.iter().filter(|item| item.model.starts_with("models/sprops/")) {
            assert!(all.contains(item.model), "{}", item.model);
        }
    }
}
