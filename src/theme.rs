//! The editor's colours as a named palette, derived from three the person
//! picks: main for every surface, second for text, lines and the accent,
//! background for the 3D view and nothing else. With the default three the
//! result is Source Filmmaker's own skin,
//! read from its Qt stylesheets and 9-slice images; other choices move each
//! entry by a weighted share of the change, so the relationships between
//! surfaces, text and accent survive. Alpha is straight, not premultiplied.

use std::collections::BTreeMap;

pub type Rgba = [u8; 4];

/// The three colours a theme is: main (surfaces), second (text, lines and
/// the accent), background (behind the 3D view, exactly as picked).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub main: Rgba,
    pub second: Rgba,
    pub background: Rgba,
}

impl Theme {
    /// The Filmmaker anchors: the QWidget grey and its text; the background
    /// is ours (see the palette's provenance note).
    pub const FILMMAKER: Theme = Theme {
        main: [54, 54, 54, 255],
        second: [182, 182, 182, 255],
        background: [92, 96, 104, 255],
    };
}

impl Default for Theme {
    fn default() -> Self {
        Self::FILMMAKER
    }
}

fn channel_deltas(a: Rgba, b: Rgba) -> [f32; 3] {
    [
        a[0] as f32 - b[0] as f32,
        a[1] as f32 - b[1] as f32,
        a[2] as f32 - b[2] as f32,
    ]
}

fn shifted(c: Rgba, w: [f32; 3], main_delta: [f32; 3], second_delta: [f32; 3], background_delta: [f32; 3]) -> Rgba {
    let channel = |i: usize| {
        (c[i] as f32 + w[0] * main_delta[i] + w[1] * second_delta[i] + w[2] * background_delta[i])
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [channel(0), channel(1), channel(2), c[3]]
}

macro_rules! palette {
    ($( $group:literal { $( $name:ident = [$r:literal, $g:literal, $b:literal, $a:literal] ~ [$wm:literal, $ws:literal, $wt:literal] ),+ $(,)? } )+) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Palette<C = Rgba> {
            $( $( pub $name: C, )+ )+
        }

        /// Every entry in display order, with the group it is shown under.
        pub const NAMES: &[(&str, &str)] = &[ $( $( ($group, stringify!($name)), )+ )+ ];

        impl Palette<Rgba> {
            pub fn filmmaker() -> Self {
                Palette { $( $( $name: [$r, $g, $b, $a], )+ )+ }
            }

            /// The palette for a theme: each entry is its Filmmaker value moved
            /// by its share of how far the three colours are from the anchors.
            pub fn derive(theme: &Theme) -> Self {
                let main_delta = channel_deltas(theme.main, Theme::FILMMAKER.main);
                let second_delta = channel_deltas(theme.second, Theme::FILMMAKER.second);
                let background_delta = channel_deltas(theme.background, Theme::FILMMAKER.background);
                Palette { $( $( $name: shifted([$r, $g, $b, $a], [$wm, $ws, $wt], main_delta, second_delta, background_delta), )+ )+ }
            }
        }

        impl<C: Copy> Palette<C> {
            pub fn get(&self, name: &str) -> Option<C> {
                match name {
                    $( $( stringify!($name) => Some(self.$name), )+ )+
                    _ => None,
                }
            }

            pub fn get_mut(&mut self, name: &str) -> Option<&mut C> {
                match name {
                    $( $( stringify!($name) => Some(&mut self.$name), )+ )+
                    _ => None,
                }
            }

            pub fn set(&mut self, name: &str, value: C) -> bool {
                match self.get_mut(name) {
                    Some(slot) => {
                        *slot = value;
                        true
                    }
                    None => false,
                }
            }

            pub fn map<D>(&self, f: impl Fn(C) -> D) -> Palette<D> {
                Palette { $( $( $name: f(self.$name), )+ )+ }
            }
        }
    };
}

// Each entry: its Filmmaker value, then how much of a change in main, second
// and background it follows. Surfaces follow main, text and the accent
// follow second, only the viewport's background follows background;
// buttons, fields and the selection are mostly main with a little
// second so their edges keep contrast; translucent hairlines and the
// semantic colours (axes, links, good, bad, chip syntax) stay put.
//
// Filmmaker provenance, from SFM's stylesheets and 9-slice images: chrome is
// CQSFMMainWindow, tab bars, status bar and every QAbstractItemView; panel the
// QWidget default grey; splitter QSplitter; console CQConsoleTextEdit;
// text_dim is Filmmaker's 190 grey at 24% made solid and lifted to 160, so
// that labels and hints reach 4.5:1 on the panels; selection is QTreeView::item:selected and
// selection_text its one warm note; view_edge the QAbstractItemView border at
// 25%; alt_row alternate rows at 6%, almost invisible on purpose; menu_* the
// QMenuBar gradient; header_* QHeaderView::section:horizontal; button_* are
// pushbutton_{enabled,hover,pressed}.png sampled down the centre column,
// slightly warm (R > G > B by about two), pressed inverted into an inset
// shadow; field* are frame.png and frame_focus.png, whose blue-teal edge is
// the accent; scrollbar_* QScrollBar with the gradient across the handle;
// axis_x and bad are CQMotionEditorView redCurveColor, axis_y green, axis_z
// blue; good is CQExportShotList doneTextColor. The viewport colours are
// chosen to sit with the chrome rather than copied from a stylesheet, and the
// background is a mid grey because a near-black one makes everything on it
// read darker than it is; Hammer and SFM both default to one.
palette! {
    "Surfaces" {
        chrome = [38, 38, 39, 255] ~ [1.0, 0.0, 0.0],
        panel = [54, 54, 54, 255] ~ [1.0, 0.0, 0.0],
        splitter = [24, 24, 25, 255] ~ [1.0, 0.0, 0.0],
        console = [44, 44, 44, 255] ~ [1.0, 0.0, 0.0],
        view_edge = [155, 151, 151, 64] ~ [0.0, 0.0, 0.0],
        alt_row = [136, 136, 136, 15] ~ [0.0, 0.0, 0.0],
        menu_top = [70, 71, 72, 255] ~ [1.0, 0.0, 0.0],
        menu_bottom = [29, 29, 29, 255] ~ [1.0, 0.0, 0.0],
        menu_hairline = [20, 20, 20, 128] ~ [0.0, 0.0, 0.0],
        header_top = [23, 23, 24, 255] ~ [1.0, 0.0, 0.0],
        header_bottom = [31, 31, 32, 255] ~ [1.0, 0.0, 0.0],
        popup_edge = [20, 20, 20, 102] ~ [0.0, 0.0, 0.0],
    }
    "Text" {
        text = [182, 182, 182, 255] ~ [0.0, 1.0, 0.0],
        text_dim = [160, 160, 160, 255] ~ [0.0, 1.0, 0.0],
        heading = [212, 212, 212, 255] ~ [0.0, 1.0, 0.0],
    }
    "Selection" {
        selection = [79, 82, 89, 255] ~ [0.8, 0.2, 0.0],
        selection_mid = [69, 76, 87, 255] ~ [0.8, 0.2, 0.0],
        selection_text = [255, 251, 209, 255] ~ [0.0, 1.0, 0.0],
        accent = [141, 158, 156, 255] ~ [0.0, 1.0, 0.0],
    }
    "Buttons" {
        button_edge = [111, 109, 107, 255] ~ [0.8, 0.2, 0.0],
        button_top = [93, 91, 89, 255] ~ [0.8, 0.2, 0.0],
        button_bottom = [73, 72, 71, 255] ~ [0.8, 0.2, 0.0],
        button_hover_edge = [124, 121, 118, 255] ~ [0.8, 0.2, 0.0],
        button_hover_top = [98, 96, 93, 255] ~ [0.8, 0.2, 0.0],
        button_hover_bottom = [74, 72, 71, 255] ~ [0.8, 0.2, 0.0],
        button_pressed_edge = [109, 104, 99, 255] ~ [0.8, 0.2, 0.0],
        button_pressed_top = [43, 42, 42, 255] ~ [0.8, 0.2, 0.0],
        button_pressed_mid = [27, 27, 27, 255] ~ [0.8, 0.2, 0.0],
        button_pressed_bottom = [55, 55, 55, 255] ~ [0.8, 0.2, 0.0],
        button_off_edge = [70, 69, 68, 255] ~ [1.0, 0.0, 0.0],
        button_off_top = [62, 61, 60, 255] ~ [1.0, 0.0, 0.0],
        button_off_bottom = [54, 53, 52, 255] ~ [1.0, 0.0, 0.0],
        button_text = [220, 220, 220, 255] ~ [0.0, 1.0, 0.0],
        button_text_hot = [255, 255, 255, 255] ~ [0.0, 1.0, 0.0],
        button_text_off = [230, 230, 230, 94] ~ [0.0, 1.0, 0.0],
        tool_hover = [66, 66, 66, 255] ~ [1.0, 0.0, 0.0],
        tool_hover_edge = [90, 90, 90, 178] ~ [0.0, 0.0, 0.0],
        tool_pressed = [0, 0, 0, 120] ~ [0.0, 0.0, 0.0],
    }
    "Fields" {
        field = [48, 48, 48, 255] ~ [1.0, 0.0, 0.0],
        field_edge = [77, 76, 75, 255] ~ [0.7, 0.3, 0.0],
        field_hover_edge = [96, 95, 94, 255] ~ [0.7, 0.3, 0.0],
        field_hover_text = [202, 202, 202, 255] ~ [0.0, 1.0, 0.0],
        field_focus = [45, 43, 43, 255] ~ [1.0, 0.0, 0.0],
        field_focus_text = [212, 212, 212, 255] ~ [0.0, 1.0, 0.0],
    }
    "Scrollbars" {
        scrollbar_track = [36, 36, 36, 64] ~ [0.0, 0.0, 0.0],
        scrollbar_edge = [98, 98, 98, 128] ~ [0.0, 0.0, 0.0],
        scrollbar_light = [231, 231, 231, 43] ~ [0.0, 0.0, 0.0],
        scrollbar_border = [80, 80, 76, 64] ~ [0.0, 0.0, 0.0],
    }
    "Viewport" {
        background = [92, 96, 104, 255] ~ [0.0, 0.0, 1.0],
        box_base = [150, 150, 152, 255] ~ [0.4, 0.6, 0.0],
        box_selected = [196, 214, 212, 255] ~ [0.0, 1.0, 0.0],
        box_marked = [190, 168, 120, 255] ~ [0.0, 0.0, 0.0],
        box_edge = [34, 34, 34, 90] ~ [0.0, 0.0, 0.0],
        axis_x = [214, 66, 79, 255] ~ [0.0, 0.0, 0.0],
        axis_y = [67, 192, 60, 255] ~ [0.0, 0.0, 0.0],
        axis_z = [70, 102, 214, 255] ~ [0.0, 0.0, 0.0],
        axis_hot = [255, 251, 209, 255] ~ [0.0, 0.0, 0.0],
        link_acf = [214, 140, 66, 255] ~ [0.0, 0.0, 0.0],
        link_wire = [70, 160, 214, 255] ~ [0.0, 0.0, 0.0],
        link = [102, 112, 129, 150] ~ [0.5, 0.0, 0.0],
        clip = [214, 66, 79, 255] ~ [0.0, 0.0, 0.0],
        good = [138, 149, 186, 255] ~ [0.0, 0.0, 0.0],
        bad = [214, 66, 79, 255] ~ [0.0, 0.0, 0.0],
    }
    "Chip code" {
        code_comment = [120, 130, 120, 255] ~ [0.0, 0.0, 0.0],
        code_string = [214, 168, 96, 255] ~ [0.0, 0.0, 0.0],
        code_directive = [141, 158, 156, 255] ~ [0.0, 1.0, 0.0],
        code_number = [180, 200, 240, 255] ~ [0.0, 0.0, 0.0],
        code_keyword = [214, 110, 120, 255] ~ [0.0, 0.0, 0.0],
        code_type = [214, 150, 90, 255] ~ [0.0, 0.0, 0.0],
        code_function = [130, 170, 220, 255] ~ [0.0, 0.0, 0.0],
        code_variable = [156, 196, 148, 255] ~ [0.0, 0.0, 0.0],
        code_gutter = [155, 155, 155, 255] ~ [0.0, 1.0, 0.0],
    }
}

impl Default for Palette<Rgba> {
    fn default() -> Self {
        Self::filmmaker()
    }
}

/// `#rrggbb`, or `#rrggbbaa` when the colour is not fully opaque.
pub fn to_hex(c: Rgba) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
    }
}

/// Reads `#rrggbb` or `#rrggbbaa`, any case, with or without the hash.
pub fn parse_hex(s: &str) -> Option<Rgba> {
    let digits = s.trim().trim_start_matches('#');
    if !(digits.len() == 6 || digits.len() == 8) || !digits.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    let alpha = if digits.len() == 8 { byte(6)? } else { 255 };
    Some([byte(0)?, byte(2)?, byte(4)?, alpha])
}

impl Palette<Rgba> {
    /// The entries that differ from `base`, as hex, keyed by name.
    pub fn overrides(&self, base: &Self) -> BTreeMap<String, String> {
        NAMES
            .iter()
            .filter_map(|(_, name)| {
                let mine = self.get(name)?;
                (Some(mine) != base.get(name)).then(|| ((*name).to_owned(), to_hex(mine)))
            })
            .collect()
    }

    /// Applies hex overrides by name. Every entry that can be applied is;
    /// the returned lines describe the ones that could not.
    pub fn apply(&mut self, overrides: &BTreeMap<String, String>) -> Vec<String> {
        let mut problems = Vec::new();
        for (name, value) in overrides {
            match parse_hex(value) {
                Some(c) if self.set(name, c) => {}
                Some(_) => problems.push(format!("palette: no colour called {name}")),
                None => problems.push(format!(
                    "palette: {name} = {value:?} is not a #rrggbb or #rrggbbaa colour"
                )),
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_opaque_and_translucent_colours() {
        assert_eq!(to_hex([1, 2, 3, 255]), "#010203");
        assert_eq!(to_hex([1, 2, 3, 4]), "#01020304");
        assert_eq!(parse_hex("#010203"), Some([1, 2, 3, 255]));
        assert_eq!(parse_hex("01020304"), Some([1, 2, 3, 4]));
        assert_eq!(parse_hex("#ABCDEF"), Some([171, 205, 239, 255]));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("nope"), None);
    }

    #[test]
    fn every_named_entry_can_be_read_and_written() {
        let mut p = Palette::filmmaker();
        for (_, name) in NAMES {
            assert!(p.get(name).is_some(), "{name} missing");
            assert!(p.set(name, [1, 2, 3, 4]), "{name} not settable");
            assert_eq!(p.get(name), Some([1, 2, 3, 4]));
        }
        assert_eq!(p.get("nope"), None);
        assert!(!p.set("nope", [0, 0, 0, 0]));
    }

    #[test]
    fn the_default_theme_derives_filmmaker_exactly() {
        assert_eq!(Palette::derive(&Theme::default()), Palette::filmmaker());
        assert_eq!(Palette::default(), Palette::filmmaker());
        assert_eq!(Theme::default().main, [54, 54, 54, 255]);
        assert_eq!(Theme::default().second, [182, 182, 182, 255]);
        assert_eq!(Theme::default().background, [92, 96, 104, 255]);
    }

    #[test]
    fn main_moves_the_surfaces_and_leaves_the_text() {
        let mut t = Theme::default();
        t.main = [64, 54, 54, 255];
        let p = Palette::derive(&t);
        assert_eq!(p.panel, [64, 54, 54, 255]);
        assert_eq!(p.chrome, [48, 38, 39, 255]);
        assert_eq!(p.text, Palette::filmmaker().text);
        assert_eq!(p.accent, Palette::filmmaker().accent);
        assert_eq!(p.axis_x, Palette::filmmaker().axis_x);
    }

    #[test]
    fn second_moves_the_text_and_the_accent() {
        let mut t = Theme::default();
        t.second = [192, 182, 182, 255];
        let p = Palette::derive(&t);
        assert_eq!(p.text, [192, 182, 182, 255]);
        assert_eq!(p.text_dim, [170, 160, 160, 255]);
        assert_eq!(p.accent, [151, 158, 156, 255]);
        assert_eq!(p.selection, [81, 82, 89, 255]);
        assert_eq!(p.panel, Palette::filmmaker().panel);
    }

    /// WCAG contrast of one opaque colour on another.
    fn contrast(text: Rgba, surface: Rgba) -> f64 {
        let light = |c: Rgba| {
            let channel = |v: u8| {
                let v = v as f64 / 255.0;
                if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * channel(c[0]) + 0.7152 * channel(c[1]) + 0.0722 * channel(c[2])
        };
        let (a, b) = (light(text), light(surface));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn text_and_dim_text_reach_wcag_aa_on_every_surface_they_sit_on() {
        let p = Palette::filmmaker();
        for surface in [p.panel, p.chrome, p.console, p.field] {
            assert!(contrast(p.text, surface) >= 4.5, "text on {surface:?}: {:.2}", contrast(p.text, surface));
            assert!(contrast(p.text_dim, surface) >= 4.5, "dim text on {surface:?}: {:.2}", contrast(p.text_dim, surface));
        }
        assert!(contrast(p.code_gutter, p.field) >= 4.5);
    }

    #[test]
    fn background_is_the_viewport_exactly_and_touches_nothing_else() {
        let mut t = Theme::default();
        t.background = [12, 200, 34, 255];
        let p = Palette::derive(&t);
        assert_eq!(p.background, [12, 200, 34, 255]);
        let mut rest = p;
        rest.background = Palette::filmmaker().background;
        assert_eq!(rest, Palette::filmmaker());
    }

    #[test]
    fn derived_channels_clamp_and_keep_alpha() {
        let mut t = Theme::default();
        t.main = [255, 255, 255, 255];
        t.second = [0, 0, 0, 255];
        let p = Palette::derive(&t);
        assert_eq!(p.menu_top, [255, 255, 255, 255]);
        assert_eq!(p.panel, [255, 255, 255, 255]);
        assert_eq!(p.text, [0, 0, 0, 255]);
        assert_eq!(p.text_dim, [0, 0, 0, 255]);
        assert_eq!(p.view_edge, Palette::filmmaker().view_edge);
    }

    #[test]
    fn overrides_are_only_the_differences_and_apply_back() {
        let base = Palette::filmmaker();
        let mut p = base;
        p.accent = [1, 2, 3, 255];
        p.box_edge = [9, 9, 9, 90];
        let o = p.overrides(&base);
        assert_eq!(o.len(), 2);
        assert_eq!(o["accent"], "#010203");
        assert_eq!(o["box_edge"], "#0909095a");
        let mut q = base;
        assert!(q.apply(&o).is_empty());
        assert_eq!(q, p);
    }

    #[test]
    fn bad_overrides_are_reported_and_skipped() {
        let mut p = Palette::filmmaker();
        let mut o = std::collections::BTreeMap::new();
        o.insert("nope".to_owned(), "#000000".to_owned());
        o.insert("accent".to_owned(), "zzz".to_owned());
        o.insert("text".to_owned(), "#ffffff".to_owned());
        let problems = p.apply(&o);
        assert_eq!(problems.len(), 2);
        assert_eq!(p.accent, Palette::filmmaker().accent);
        assert_eq!(p.text, [255, 255, 255, 255]);
    }

    #[test]
    fn map_converts_every_entry() {
        let m = Palette::filmmaker().map(|c| u32::from(c[0]) + 1);
        assert_eq!(m.text, 183);
        assert_eq!(m.get("chrome"), Some(39));
    }
}
