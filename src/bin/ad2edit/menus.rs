//! The menu bar every desktop program has: File, Edit, View, Tools, Help.
//! Everything in it already existed as a button or a shortcut somewhere;
//! what was missing was one obvious place to find it, and File > New.

use super::{catalogue_groups, colours, dupe_file_bytes, group_of, unique_path, App, Gizmo, Loaded};
use ad2read::history::History;
use eframe::egui::{self, RichText, Ui};

/// The step-by-step tutorial: whether it is showing, and its steps as of
/// the last edit, so the doctor runs once per change and not once per frame.
#[derive(Default)]
pub struct TutorialPanel {
    pub showing: bool,
    steps: Vec<ad2read::tutorial::Step>,
    built_at: Option<u64>,
}

/// What a new dupe starts with.
#[derive(Clone, Copy)]
pub enum NewDupe {
    Empty,
    Baseplate,
}


impl App {
    /// A new, unsaved dupe. Nothing is written until Save, which goes to a
    /// free `untitled` name in the AdvDupe2 folder. An unsaved dupe that is
    /// being left gets an autosave first, so New can never lose work.
    pub fn new_dupe(&mut self, start: NewDupe) {
        self.autosave_before_leaving();
        let mut dupe = ad2read::buildfile::empty_dupe();
        if let NewDupe::Baseplate = start {
            let Some(item) = ad2read::catalog::find("GroundVehicle") else {
                return self.bad("the catalogue has no ground vehicle baseplate");
            };
            // Vehicles run along world +Y, so the plate sits at yaw 90.
            let size = [("Width", 96.0), ("Length", 200.0), ("Thickness", 2.0)];
            if let Err(e) = ad2read::build::spawn_catalog(&mut dupe, item, (0.0, 0.0, 0.0), (0.0, 90.0, 0.0), &size) {
                return self.bad(format!("baseplate: {e}"));
            }
            ad2read::extras::repair_head(&mut dupe);
        }
        let file = match dupe_file_bytes(&dupe, "untitled").and_then(|bytes| ad2read::dupefile::parse(&bytes)) {
            Ok(file) => file,
            Err(e) => return self.bad(format!("new dupe: {e}")),
        };
        let path = unique_path(&self.build_dir(), "untitled");
        let history = History::new(&dupe);
        self.open = Some(Loaded { path, file, lzma: (3, 0, 2, 65536, true), dupe, history, dirty: true });
        self.pick = None;
        self.crumbs.clear();
        self.marked.clear();
        self.frame_camera();
        self.last_autosave = std::time::Instant::now();
        self.edit_count += 1;
        self.good(match start {
            NewDupe::Empty => "new empty dupe. Add parts, then Save.",
            NewDupe::Baseplate => "new dupe with a baseplate. Add parts to it, then Save.",
        });
    }

    pub(crate) fn autosave_before_leaving(&mut self) {
        let Some(open) = self.open.as_ref().filter(|open| open.dirty) else { return };
        let path = Self::autosave_path(&open.path);
        match self.encode_current().and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string())) {
            Ok(()) => self.say(format!("unsaved changes kept in {}", path.display())),
            Err(e) => self.bad(format!("could not keep the unsaved changes: {e}")),
        }
    }

    pub fn open_with_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new().add_filter("AdvDupe2", &["txt"]);
        let folder = self.build_dir();
        if folder.is_dir() {
            dialog = dialog.set_directory(folder);
        }
        if let Some(path) = dialog.pick_file() {
            self.autosave_before_leaving();
            self.load(path);
        }
    }

    pub fn save_with_dialog(&mut self) {
        let Some(start) = self.open.as_ref().map(|open| open.path.clone()) else { return };
        let mut dialog = rfd::FileDialog::new().add_filter("AdvDupe2", &["txt"]);
        if let Some(folder) = start.parent() {
            dialog = dialog.set_directory(folder);
        }
        if let Some(name) = start.file_name() {
            dialog = dialog.set_file_name(name.to_string_lossy());
        }
        if let Some(path) = dialog.save_file() {
            self.save_as(path);
        }
    }

    /// The tutorial window. Its ticks are facts about the open dupe, so
    /// doing a step anywhere in the editor ticks it here.
    pub fn tutorial_window(&mut self, ctx: &egui::Context) {
        if !self.tutorial.showing {
            return;
        }
        if self.tutorial.built_at != Some(self.edit_count) {
            self.tutorial.steps = ad2read::tutorial::first_tank(self.open.as_ref().map(|open| &open.dupe));
            self.tutorial.built_at = Some(self.edit_count);
        }
        let mut showing = true;
        egui::Window::new("First tank, step by step")
            .id(egui::Id::new("tutorial-first-tank"))
            .open(&mut showing)
            .default_width(360.0)
            .resizable(true)
            .show(ctx, |ui| {
                let next = self.tutorial.steps.iter().position(|step| !step.done);
                for (at, step) in self.tutorial.steps.iter().enumerate() {
                    let (mark, colour) = if step.done {
                        ("done", colours().good)
                    } else if Some(at) == next {
                        ("next", colours().accent)
                    } else {
                        ("    ", colours().text_dim)
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(mark).color(colour).monospace());
                        ui.label(RichText::new(step.title).color(if step.done { colours().text_dim } else { colours().text }).strong());
                    });
                    if Some(at) == next {
                        ui.label(RichText::new(step.how).color(colours().text));
                        ui.add_space(4.0);
                    }
                }
                if next.is_none() {
                    ui.add_space(6.0);
                    ui.label(RichText::new("That is a tank. Save it, then File, Send to game, and paste it with AdvDupe2.").color(colours().good));
                }
            });
        self.tutorial.showing = showing;
    }

    pub fn menus(&mut self, ui: &mut Ui) {
        let has = self.open.is_some();
        let selected = !self.targets().is_empty();
        let item = |ui: &mut Ui, enabled: bool, label: &str, shortcut: &str| {
            ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut)).clicked()
        };
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if item(ui, true, "New", "Ctrl+N") {
                    self.new_dupe(NewDupe::Empty);
                }
                if item(ui, true, "New with a baseplate", "Ctrl+Shift+N") {
                    self.new_dupe(NewDupe::Baseplate);
                }
                ui.separator();
                if item(ui, true, "Open...", "Ctrl+O") {
                    self.open_with_dialog();
                }
                let recent = self.gallery_view.recent(10);
                ui.add_enabled_ui(!recent.is_empty(), |ui| {
                    ui.menu_button("Open recent", |ui| {
                        for (title, path) in recent {
                            if ui.button(title).on_hover_text(path.display().to_string()).clicked() {
                                self.autosave_before_leaving();
                                self.load(path);
                                ui.close();
                            }
                        }
                    });
                });
                ui.separator();
                if item(ui, has, "Save", "Ctrl+S") {
                    if let Some(path) = self.open.as_ref().map(|open| open.path.clone()) {
                        self.save_as(path);
                    }
                }
                if item(ui, has, "Save As...", "Ctrl+Shift+S") {
                    self.save_with_dialog();
                }
                if item(ui, has, "Send to game", "") {
                    self.send_to_game();
                }
                ui.separator();
                if item(ui, has, "Close, back to the gallery", "") {
                    self.autosave_before_leaving();
                    self.gallery_view.forget_files();
                    self.open = None;
                    self.pick = None;
                    self.marked.clear();
                    self.crumbs.clear();
                    self.edit_count += 1;
                }
                if item(ui, true, "Exit", "") {
                    self.autosave_before_leaving();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if item(ui, has, "Undo", "Ctrl+Z") {
                    self.undo();
                }
                if item(ui, has, "Redo", "Ctrl+Y") {
                    self.redo();
                }
                ui.separator();
                if item(ui, selected, "Copy", "Ctrl+C") {
                    self.copy_selection();
                }
                if item(ui, has, "Paste", "Ctrl+V") {
                    self.paste_clipboard();
                }
                if item(ui, selected, "Duplicate", "Ctrl+D") {
                    let targets = self.targets();
                    self.op_duplicate(&targets);
                }
                if item(ui, selected, "Delete", "Del") {
                    let targets = self.targets();
                    self.op_delete(&targets);
                }
                ui.separator();
                if item(ui, has, "Select all", "Ctrl+A") {
                    if let Some(open) = self.open.as_ref() {
                        self.marked = open.dupe.list_entities().into_iter().map(|(index, _, _)| index).collect();
                    }
                }
                if item(ui, selected, "Deselect", "Esc") {
                    self.deselect();
                }
                ui.separator();
                if item(ui, true, "Settings...", "") {
                    self.show_settings = true;
                }
            });
            ui.menu_button("Add", |ui| {
                if item(ui, true, "Search everything, SProps, favourites...", "Insert") {
                    self.add_panel.open();
                    ui.close();
                }
                ui.separator();
                // The whole catalogue, by group and then by family. A part
                // goes in front of the camera, attached the way the
                // references attach it; with nothing open, a new dupe is
                // started for it.
                let mut chosen = None;
                for (group, count) in catalogue_groups() {
                    ui.menu_button(format!("{group}  ({count})"), |ui| {
                        let in_group = || ad2read::catalog::ITEMS.iter().filter(move |entry| group_of(entry.category) == group);
                        let mut families: Vec<&str> = Vec::new();
                        for entry in in_group() {
                            if !families.contains(&entry.category) {
                                families.push(entry.category);
                            }
                        }
                        let mut list = |ui: &mut Ui, family: &str| {
                            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                                for entry in in_group().filter(|entry| entry.category == family) {
                                    let resp = ui.button(entry.name);
                                    if resp.clicked() {
                                        chosen = Some(entry);
                                    }
                                    if !entry.note.is_empty() {
                                        resp.on_hover_text(entry.note);
                                    }
                                }
                            });
                        };
                        if let [only] = families.as_slice() {
                            list(ui, only);
                        } else {
                            for family in families {
                                let label = family.split(" / ").nth(1).unwrap_or(family);
                                ui.menu_button(label, |ui| list(ui, family));
                            }
                        }
                    });
                }
                if let Some(entry) = chosen {
                    if self.open.is_none() {
                        self.new_dupe(NewDupe::Empty);
                    }
                    self.add_part(entry);
                }
                ui.separator();
                ui.label(RichText::new("To bring in parts from another dupe: Merge a dupe in, on the right.").color(colours().text_dim).small());
            });
            ui.menu_button("View", |ui| {
                if ui.radio(!self.node_view.showing, "3D view").clicked() {
                    self.node_view.showing = false;
                }
                if ui.radio(self.node_view.showing, "Nodes").clicked() {
                    self.node_view.showing = true;
                }
                ui.menu_button("Show", |ui| {
                    // A role per line, with how many parts have it: armour off
                    // shows the drivetrain, props off shows only what ACF made.
                    let mut counts: Vec<(&'static str, usize)> = Vec::new();
                    if let Some(open) = self.open.as_ref() {
                        for (_, role) in ad2read::roles::of_dupe(&open.dupe) {
                            match counts.iter_mut().find(|(name, _)| *name == role.name()) {
                                Some((_, count)) => *count += 1,
                                None => counts.push((role.name(), 1)),
                            }
                        }
                    }
                    counts.sort();
                    if counts.is_empty() {
                        ui.label(RichText::new("nothing open").color(colours().text_dim));
                    }
                    for (name, count) in counts {
                        let mut shown = !self.hidden_roles.contains(name);
                        if ui.checkbox(&mut shown, format!("{name}  ({count})")).changed() {
                            self.show_role(name, shown);
                        }
                    }
                    if !self.hidden_roles.is_empty() && ui.button("Show everything").clicked() {
                        self.hidden_roles.clear();
                        self.hidden_cache = None;
                    }
                });
                ui.separator();
                if item(ui, selected, "Frame the selection", "F") {
                    self.frame_selection();
                }
                if item(ui, has, "Frame everything", "") {
                    self.frame_camera();
                }
                ui.separator();
                ui.checkbox(&mut self.show_links, "ACF links in the 3D view");
                ui.checkbox(&mut self.show_wires, "Wires in the 3D view");
            });
            ui.menu_button("Tools", |ui| {
                if ui.radio(self.gizmo == Gizmo::Move, "Move handle").clicked() {
                    self.gizmo = Gizmo::Move;
                }
                if ui.radio(self.gizmo == Gizmo::Scale, "Scale handle").clicked() {
                    self.gizmo = Gizmo::Scale;
                }
                if ui.radio(self.gizmo == Gizmo::Rotate, "Rotate handle").clicked() {
                    self.gizmo = Gizmo::Rotate;
                }
                ui.separator();
                if self.config.mode == ad2read::config::Mode::Advanced && item(ui, has, "Checklist: what the doctor sees...", "") {
                    self.show_checklist = true;
                }
                if item(ui, has, "Fix what the doctor can", "") {
                    self.fix_findings(None);
                }
                if item(ui, has, "Build a sloped hull", "") {
                    self.build_hull();
                }
                if item(ui, has, "Plan the armour", "") {
                    self.plan_armour();
                }
                ui.separator();
                if item(ui, true, "Index the game's models now", "") {
                    let folder = std::path::PathBuf::from(&self.config.garrysmod);
                    if folder.is_dir() {
                        self.index_models(folder);
                    } else {
                        self.bad("no game folder set; point at it in Settings");
                        self.show_settings = true;
                    }
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("First tank, step by step").clicked() {
                    self.tutorial.showing = true;
                    ui.close();
                }
                ui.separator();
                let lines = [
                    "Drag in the 3D view to look, WASD to fly, Shift-drag to slide.",
                    "1 and 2 pick the move and rotate handles; Ctrl snaps a drag.",
                    "Ctrl slows a flight; Ctrl+S, Ctrl+A and the rest wait until the camera is at rest.",
                    "F frames the selection. Right-click or double-click for the part menu.",
                    "The Nodes tab links and wires by dragging between sockets.",
                    "Every command is in README.md beside the program.",
                ];
                for line in lines {
                    ui.label(RichText::new(line).color(colours().text_dim));
                }
            });
        });
    }
}
