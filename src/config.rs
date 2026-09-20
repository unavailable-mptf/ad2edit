//! The editor's per-user config: mode, look, game folders, viewport habits.
//!
//! Plain TOML under the platform's config folder. Every key has a default and
//! unknown keys are ignored, so a hand edit or an older file never stops the
//! app; a file that does not parse is moved aside rather than overwritten.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::theme::{self, Palette, Theme};

pub const APP_DIR: &str = "ad2edit";
pub const FILE: &str = "config.toml";

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Newcomer,
    Advanced,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Config {
    pub mode: Mode,
    /// egui zoom factor: 0.9 small, 1.0 normal, 1.25 large.
    pub ui_scale: f32,
    /// The three colours the palette derives from.
    pub theme: ThemeColours,
    /// The game folder; empty until found or chosen.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub garrysmod: String,
    /// Where AdvDupe2 reads dupes; empty means garrysmod/data/advdupe2.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub advdupe2: String,
    /// Palette entries that differ from the preset, as hex by name.
    pub palette: BTreeMap<String, String>,
    pub viewport: Viewport,
    pub placement: Placement,
    pub keys: Keys,
    pub autosave: Autosave,
    /// Parts starred in the Add window: `catalogue:<entity>:<value>` or `sprops:<name>`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub favourites: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mode: Mode::Advanced,
            ui_scale: 1.0,
            theme: ThemeColours::default(),
            garrysmod: String::new(),
            advdupe2: String::new(),
            palette: BTreeMap::new(),
            viewport: Viewport::default(),
            placement: Placement::default(),
            keys: Keys::default(),
            autosave: Autosave::default(),
            favourites: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Viewport {
    pub real_geometry: bool,
    pub fullbright: bool,
    /// Render scale, 0.25 to 1.0 of the viewport size.
    pub detail: f32,
    /// Fly speed in units per second.
    pub fly_speed: f64,
    /// Multiplies how far the view turns for a given mouse movement.
    pub look_sensitivity: f64,
    /// Mouse up looks down.
    pub invert_look: bool,
    /// A marker where the contraption's weight balances.
    pub centre_of_mass: bool,
    /// Multiplies every model's colour. 1.0 is the texture as authored,
    /// which for a dark tank on a dark background is hard to read.
    pub brightness: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            real_geometry: true,
            fullbright: true,
            detail: 0.75,
            fly_speed: 400.0,
            look_sensitivity: 1.0,
            invert_look: false,
            centre_of_mass: false,
            brightness: 1.0,
        }
    }
}

/// How the move and rotate handles step. With `snapping` on a drag lands on
/// multiples of the steps and Alt frees it; with it off Alt snaps.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Placement {
    pub snapping: bool,
    /// Source units.
    pub move_step: f64,
    /// Degrees.
    pub turn_step: f64,
}

impl Default for Placement {
    fn default() -> Self {
        Placement { snapping: true, move_step: 0.5, turn_step: 5.0 }
    }
}

impl Placement {
    /// The steps in force while `alt` is or is not held: None is free placement.
    pub fn steps(&self, alt: bool) -> Option<(f64, f64)> {
        (self.snapping != alt).then(|| (self.move_step.max(0.0), self.turn_step.max(0.0)))
    }
}

/// The editor's keys, by the names egui gives them ("F", "1", "Delete",
/// "Insert"). The first seven are pressed alone; the rest with Ctrl. Flying
/// (W A S D, Q E, Shift, Ctrl) is not here: it is held, not pressed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Keys {
    pub move_handle: String,
    pub rotate_handle: String,
    pub scale_handle: String,
    pub frame: String,
    pub add: String,
    pub delete: String,
    pub deselect: String,
    pub undo: String,
    pub redo: String,
    pub copy: String,
    pub paste: String,
    pub duplicate: String,
    pub save: String,
    pub open: String,
    pub select_all: String,
    pub new_dupe: String,
}

impl Default for Keys {
    fn default() -> Self {
        let key = |name: &str| name.to_owned();
        Keys {
            move_handle: key("1"),
            rotate_handle: key("2"),
            scale_handle: key("3"),
            frame: key("F"),
            add: key("Insert"),
            delete: key("Delete"),
            deselect: key("Escape"),
            undo: key("Z"),
            redo: key("Y"),
            copy: key("C"),
            paste: key("V"),
            duplicate: key("D"),
            save: key("S"),
            open: key("O"),
            select_all: key("A"),
            new_dupe: key("N"),
        }
    }
}

impl Keys {
    /// Every key as (what it does, whether it goes with Ctrl, its name), in
    /// the order Settings lists them.
    pub fn slots(&mut self) -> [(&'static str, bool, &mut String); 16] {
        [
            ("move handle", false, &mut self.move_handle),
            ("rotate handle", false, &mut self.rotate_handle),
            ("scale handle", false, &mut self.scale_handle),
            ("frame the selection", false, &mut self.frame),
            ("the Add window", false, &mut self.add),
            ("delete", false, &mut self.delete),
            ("deselect", false, &mut self.deselect),
            ("undo", true, &mut self.undo),
            ("redo", true, &mut self.redo),
            ("copy", true, &mut self.copy),
            ("paste", true, &mut self.paste),
            ("duplicate", true, &mut self.duplicate),
            ("save", true, &mut self.save),
            ("open", true, &mut self.open),
            ("select everything", true, &mut self.select_all),
            ("new dupe", true, &mut self.new_dupe),
        ]
    }

    /// Pairs of actions that share a key and so would both fire.
    pub fn clashes(&self) -> Vec<(&'static str, &'static str)> {
        let mut copy = self.clone();
        let listed: Vec<(&'static str, bool, String)> = copy.slots().into_iter().map(|(what, with_ctrl, name)| (what, with_ctrl, name.to_lowercase())).collect();
        let mut found = Vec::new();
        for (n, a) in listed.iter().enumerate() {
            for b in &listed[n + 1..] {
                if a.1 == b.1 && a.2 == b.2 && !a.2.is_empty() {
                    found.push((a.0, b.0));
                }
            }
        }
        found
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Autosave {
    /// Interval in seconds; 0 turns autosave off.
    pub seconds: u64,
}

impl Default for Autosave {
    fn default() -> Self {
        Autosave { seconds: 120 }
    }
}

/// The three theme colours as hex, the way they sit in the file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct ThemeColours {
    /// Every surface.
    pub main: String,
    /// Text and lines.
    pub second: String,
    /// Behind the 3D view, and nothing else. (Until 19 Sept 2026 the third
    /// colour was an accent under the key `third`; that key is now ignored.)
    pub background: String,
}

impl Default for ThemeColours {
    fn default() -> Self {
        Self::from_theme(&Theme::default())
    }
}

impl ThemeColours {
    pub fn from_theme(t: &Theme) -> Self {
        ThemeColours {
            main: theme::to_hex(t.main),
            second: theme::to_hex(t.second),
            background: theme::to_hex(t.background),
        }
    }

    /// The colours parsed; one that does not parse keeps its default and is
    /// reported.
    pub fn theme(&self) -> (Theme, Vec<String>) {
        let mut t = Theme::default();
        let mut problems = Vec::new();
        let slots = [
            ("main", &self.main, &mut t.main),
            ("second", &self.second, &mut t.second),
            ("background", &self.background, &mut t.background),
        ];
        for (name, text, slot) in slots {
            match theme::parse_hex(text) {
                Some(c) => *slot = c,
                None => problems.push(format!("theme: {name} = {text:?} is not a #rrggbb colour")),
            }
        }
        (t, problems)
    }
}

/// What loading the config found.
#[derive(Debug)]
pub enum LoadResult {
    /// No file: the editor has not been set up on this machine.
    FirstRun,
    Ok(Config),
    /// The file exists but could not be used. When it failed to parse it has
    /// been moved to `moved_to` so the person's text survives.
    Broken {
        path: PathBuf,
        moved_to: Option<PathBuf>,
        error: String,
    },
}

impl Config {
    pub fn from_toml(text: &str) -> Result<Config, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, self.to_toml()).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Writes to the platform's config path and returns it.
    pub fn save(&self) -> Result<PathBuf, String> {
        let p = path().ok_or("no config folder: none of APPDATA, XDG_CONFIG_HOME or HOME is set")?;
        self.save_to(&p)?;
        Ok(p)
    }

    /// The palette derived from the three colours, with the overrides
    /// applied, and anything that could not be applied.
    pub fn palette(&self) -> (Palette, Vec<String>) {
        let (theme, mut problems) = self.theme.theme();
        let mut p = Palette::derive(&theme);
        problems.extend(p.apply(&self.palette));
        (p, problems)
    }

    /// Stores a palette as overrides against what the three colours derive.
    pub fn set_palette(&mut self, p: &Palette) {
        let (theme, _) = self.theme.theme();
        self.palette = p.overrides(&Palette::derive(&theme));
    }
}

pub fn load_from(path: &Path) -> LoadResult {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LoadResult::FirstRun,
        Err(e) => {
            return LoadResult::Broken {
                path: path.to_path_buf(),
                moved_to: None,
                error: e.to_string(),
            }
        }
    };
    match Config::from_toml(&text) {
        Ok(c) => LoadResult::Ok(c),
        Err(error) => {
            let bad = path.with_extension("toml.bad");
            let moved_to = std::fs::rename(path, &bad).ok().map(|_| bad);
            LoadResult::Broken {
                path: path.to_path_buf(),
                moved_to,
                error,
            }
        }
    }
}

/// Loads from the platform's config path; no usable path means first run.
pub fn load() -> LoadResult {
    match path() {
        Some(p) => load_from(&p),
        None => LoadResult::FirstRun,
    }
}

pub fn path() -> Option<PathBuf> {
    dir().map(|d| d.join(FILE))
}

pub fn dir() -> Option<PathBuf> {
    dir_from_env(|k| std::env::var_os(k))
}

/// The editor's own data folder: the gallery's index, and the original,
/// versions and thumbnail it keeps for each dupe.
pub fn gallery_dir() -> Option<PathBuf> {
    dir().map(|dir| dir.join("gallery"))
}

/// The config folder from an environment lookup: `%APPDATA%` on Windows,
/// else `$XDG_CONFIG_HOME`, else `~/.config`.
pub fn dir_from_env(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let base = if let Some(appdata) = var("APPDATA") {
        PathBuf::from(appdata)
    } else if let Some(xdg) = var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else {
        PathBuf::from(var("HOME")?).join(".config")
    };
    Some(base.join(APP_DIR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_default_to_the_ones_the_editor_always_had_and_say_when_two_clash() {
        let mut keys = Keys::default();
        assert!(keys.clashes().is_empty());
        assert_eq!(keys.slots().len(), 16);
        keys.frame = "1".into();
        assert_eq!(keys.clashes(), [("move handle", "frame the selection")]);
        // Ctrl+D and a plain D are different keys to press.
        let mut keys = Keys::default();
        keys.frame = "D".into();
        assert!(keys.clashes().is_empty());
        let config = Config::from_toml("[keys]\nframe = \"G\"\n").unwrap();
        assert_eq!((config.keys.frame.as_str(), config.keys.undo.as_str()), ("G", "Z"));
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ad2cfg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_are_advanced_filmmaker_and_sane() {
        let c = Config::default();
        assert_eq!(c.mode, Mode::Advanced);
        assert_eq!(c.ui_scale, 1.0);
        assert_eq!(c.theme.main, "#363636");
        assert_eq!(c.theme.second, "#b6b6b6");
        assert_eq!(c.theme.background, "#5c6068");
        assert!(c.garrysmod.is_empty());
        assert!(c.advdupe2.is_empty());
        assert!(!c.to_toml().contains("garrysmod"));
        assert!(c.palette.is_empty());
        assert!(c.viewport.real_geometry);
        assert!(c.viewport.fullbright);
        assert_eq!(c.viewport.detail, 0.75);
        assert_eq!(c.viewport.fly_speed, 400.0);
        assert_eq!(c.autosave.seconds, 120);
    }

    #[test]
    fn round_trips_through_toml() {
        let mut c = Config::default();
        c.mode = Mode::Newcomer;
        c.ui_scale = 1.25;
        c.theme.main = "#0a140c".to_owned();
        c.garrysmod = "D:/Steam/steamapps/common/GarrysMod/garrysmod".to_owned();
        c.palette.insert("accent".to_owned(), "#010203".to_owned());
        c.viewport.fullbright = false;
        c.autosave.seconds = 0;
        let text = c.to_toml();
        assert!(text.contains("mode = \"newcomer\""));
        assert_eq!(Config::from_toml(&text).unwrap(), c);
    }

    #[test]
    fn missing_and_unknown_keys_fall_back_to_defaults() {
        let c = Config::from_toml(
            "mode = \"newcomer\"\nfuture_key = 1\n[viewport]\nfullbright = false\n",
        )
        .unwrap();
        assert_eq!(c.mode, Mode::Newcomer);
        assert!(!c.viewport.fullbright);
        assert!(c.viewport.real_geometry);
        assert_eq!(c.autosave.seconds, 120);
        assert_eq!(c.theme.background, "#5c6068");
    }

    #[test]
    fn no_file_means_first_run() {
        let dir = scratch("firstrun");
        assert!(matches!(load_from(&dir.join(FILE)), LoadResult::FirstRun));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_file_is_moved_aside_and_reported() {
        let dir = scratch("broken");
        let path = dir.join(FILE);
        std::fs::write(&path, "mode = [not toml").unwrap();
        let LoadResult::Broken { moved_to, error, .. } = load_from(&path) else {
            panic!("expected Broken");
        };
        assert!(!error.is_empty());
        let bad = dir.join("config.toml.bad");
        assert_eq!(moved_to, Some(bad.clone()));
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "mode = [not toml");
        assert!(matches!(load_from(&path), LoadResult::FirstRun));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_creates_the_folder_and_load_reads_it_back() {
        let dir = scratch("save");
        let path = dir.join("deeper").join(FILE);
        let mut c = Config::default();
        c.garrysmod = "X:/gmod/garrysmod".to_owned();
        c.save_to(&path).unwrap();
        let LoadResult::Ok(back) = load_from(&path) else {
            panic!("expected Ok");
        };
        assert_eq!(back, c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_config_dir_comes_from_the_environment() {
        let env = |vars: &[(&str, &str)]| {
            let vars: Vec<(String, String)> =
                vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            move |key: &str| {
                vars.iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        assert_eq!(
            dir_from_env(env(&[("APPDATA", "C:/Users/x/AppData/Roaming"), ("HOME", "C:/Users/x")])),
            Some(PathBuf::from("C:/Users/x/AppData/Roaming").join(APP_DIR))
        );
        assert_eq!(
            dir_from_env(env(&[("XDG_CONFIG_HOME", "/home/deck/.config"), ("HOME", "/home/deck")])),
            Some(PathBuf::from("/home/deck/.config").join(APP_DIR))
        );
        assert_eq!(
            dir_from_env(env(&[("HOME", "/home/deck")])),
            Some(PathBuf::from("/home/deck/.config").join(APP_DIR))
        );
        assert_eq!(dir_from_env(env(&[])), None);
    }

    #[test]
    fn palette_is_derived_from_the_theme_plus_overrides() {
        let mut c = Config::default();
        c.theme.main = "#404040".to_owned();
        c.palette.insert("accent".to_owned(), "#010203".to_owned());
        let (p, problems) = c.palette();
        assert!(problems.is_empty());
        assert_eq!(p.accent, [1, 2, 3, 255]);
        assert_eq!(p.panel, [64, 64, 64, 255]);

        let mut edited = p;
        edited.text = [7, 7, 7, 255];
        c.set_palette(&edited);
        assert_eq!(c.palette.len(), 2);
        assert_eq!(c.palette["text"], "#070707");

        c.theme.second = "nope".to_owned();
        let (p, problems) = c.palette();
        assert_eq!(problems.len(), 1);
        assert_eq!(p.text, [7, 7, 7, 255]);
        assert_eq!(p.heading, Palette::filmmaker().heading);
    }
}
