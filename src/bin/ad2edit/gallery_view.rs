//! The gallery: what the editor shows before a dupe is open. Every dupe in
//! the AdvDupe2 folder as a card with a thumbnail, a title of its own, what
//! is in it and when it was last opened; stars, an archive, search and
//! sorting; and on right-click the rest: rename, duplicate, history, show
//! in folder. `ad2read::gallery` holds the index and the kept versions;
//! this file draws them. Thumbnails are rendered on a worker thread with
//! its own model library, so the window never waits for one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use super::{colours, menus, App};
use ad2read::config;
use ad2read::gallery::{self, Card, Gallery, SortBy, Summary};
use eframe::egui::{self, RichText, Ui};

const CARD: egui::Vec2 = egui::vec2(256.0, 226.0);
const PICTURE_HEIGHT: f32 = 160.0;

/// What the thumbnail worker reports for one dupe.
struct Rendered {
    file: PathBuf,
    summary: Result<Summary, String>,
}

pub struct GalleryView {
    index: Gallery,
    /// None until the first look at the folders, and after anything that
    /// may have changed them.
    files: Option<Vec<PathBuf>>,
    roots: Vec<PathBuf>,
    query: String,
    sort: SortBy,
    reversed: bool,
    only_starred: bool,
    show_archived: bool,
    pictures: HashMap<String, Option<egui::TextureHandle>>,
    renaming: Option<(PathBuf, String)>,
    noting: Option<(PathBuf, String)>,
    history_of: Option<PathBuf>,
    rendered: Option<mpsc::Receiver<Rendered>>,
    waiting_for: usize,
}

impl Default for GalleryView {
    fn default() -> Self {
        let index = config::gallery_dir().map(|data| Gallery::load(&data)).unwrap_or_default();
        GalleryView {
            index,
            files: None,
            roots: Vec::new(),
            query: String::new(),
            sort: SortBy::default(),
            reversed: false,
            only_starred: false,
            show_archived: false,
            pictures: HashMap::new(),
            renaming: None,
            noting: None,
            history_of: None,
            rendered: None,
            waiting_for: 0,
        }
    }
}

fn picture_from(path: &Path, ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let mut reader = png::Decoder::new(file).read_info().ok()?;
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut pixels).ok()?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &pixels[..frame.buffer_size()]);
    Some(ctx.load_texture(format!("thumbnail {}", path.display()), image, egui::TextureOptions::LINEAR))
}

fn readable_size(bytes: u64) -> String {
    if bytes >= 1_048_576 { format!("{:.1} MB", bytes as f64 / 1_048_576.0) } else { format!("{} KB", bytes.div_ceil(1024)) }
}

impl GalleryView {
    pub(crate) fn forget_files(&mut self) {
        self.files = None;
    }

    fn save_index(&self) -> Result<(), String> {
        let data = config::gallery_dir().ok_or("no config folder to keep the gallery in")?;
        self.index.save(&data)
    }

    /// The dupes opened most recently, newest first, that still exist.
    pub(crate) fn recent(&self, count: usize) -> Vec<(String, PathBuf)> {
        let mut opened: Vec<(&String, &gallery::Entry)> = self.index.dupes.iter().filter(|(_, entry)| entry.opened > 0).collect();
        opened.sort_by(|a, b| b.1.opened.cmp(&a.1.opened));
        opened
            .into_iter()
            .map(|(key, entry)| (entry, PathBuf::from(key)))
            .filter(|(_, path)| path.is_file())
            .take(count)
            .map(|(entry, path)| {
                let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
                (if entry.title.is_empty() { stem } else { entry.title.clone() }, path)
            })
            .collect()
    }
}

impl App {
    /// Called by `load`: the gallery remembers when and how often.
    pub(crate) fn note_opened(&mut self, file: &Path) {
        // An autosave being restored is not a dupe of its own.
        if file.to_string_lossy().to_lowercase().ends_with(".autosave.txt") {
            return;
        }
        self.gallery_view.index.mark_opened(file, gallery::now());
        if let Err(e) = self.gallery_view.save_index() {
            self.bad(format!("gallery: {e}"));
        }
    }

    /// Called by `save_as` before it writes: the file about to be replaced
    /// is put aside, the original the first time and a version after that.
    pub(crate) fn keep_before_saving(&mut self, file: &Path) {
        let Some(data) = config::gallery_dir() else { return };
        if let Err(e) = gallery::keep(&data, file, gallery::now()) {
            self.bad(format!("could not keep the previous version: {e}"));
        }
        self.gallery_view.pictures.remove(&gallery::key_of(file));
    }

    fn gallery_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.build_dir()];
        let ad2 = PathBuf::from(&self.config.advdupe2);
        if !self.config.advdupe2.trim().is_empty() && ad2.is_dir() && !roots.contains(&ad2) {
            roots.push(ad2);
        }
        roots
    }

    fn start_thumbnails(&mut self, files: Vec<PathBuf>, ctx: &egui::Context) {
        let (Some(data), false) = (config::gallery_dir(), files.is_empty()) else { return };
        let garrysmod = PathBuf::from(&self.config.garrysmod);
        if self.config.garrysmod.is_empty() || !garrysmod.is_dir() || self.gallery_view.rendered.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.gallery_view.rendered = Some(receiver);
        self.gallery_view.waiting_for = files.len();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut lib = ad2read::models::ModelLibrary::new(&garrysmod);
            for file in files {
                // One odd model must not end the run for every dupe after it.
                let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gallery::render_thumbnail(&mut lib, &data, &file)));
                let summary = drawn.unwrap_or_else(|_| Err("drawing it failed".to_owned()));
                if sender.send(Rendered { file, summary }).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
    }

    fn take_rendered(&mut self) {
        let Some(receiver) = self.gallery_view.rendered.as_ref() else { return };
        let mut arrived: Vec<Rendered> = Vec::new();
        let finished = loop {
            match receiver.try_recv() {
                Ok(done) => arrived.push(done),
                Err(mpsc::TryRecvError::Empty) => break false,
                Err(mpsc::TryRecvError::Disconnected) => break true,
            }
        };
        for done in &arrived {
            self.gallery_view.waiting_for = self.gallery_view.waiting_for.saturating_sub(1);
            self.gallery_view.pictures.remove(&gallery::key_of(&done.file));
            match &done.summary {
                Ok(summary) => self.gallery_view.index.entry_mut(&done.file).summary = Some(summary.clone()),
                Err(e) => self.say(format!("{}: {e}", done.file.display())),
            }
        }
        if finished {
            self.gallery_view.rendered = None;
            self.gallery_view.waiting_for = 0;
        }
        if !arrived.is_empty() {
            if let Err(e) = self.gallery_view.save_index() {
                self.bad(format!("gallery: {e}"));
            }
        }
    }

    fn open_from_gallery(&mut self, file: PathBuf) {
        self.autosave_before_leaving();
        self.load(file);
    }

    /// The whole window while no dupe is open.
    pub(crate) fn gallery_frame(&mut self, ui: &mut Ui) {
        let c = colours();
        egui::Panel::top("gallery-bar")
            .frame(egui::Frame::default().fill(c.chrome).inner_margin(egui::Margin::symmetric(12, 8)))
            .show(ui, |ui| {
                self.menus(ui);
                self.gallery_bar(ui);
            });
        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(70.0)
            .frame(egui::Frame::default().fill(c.console).inner_margin(egui::Margin::same(6)))
            .show(ui, |ui| self.log_panel(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(c.panel).inner_margin(egui::Margin::same(16)))
            .show(ui, |ui| self.gallery_cards(ui));
        let ctx = ui.ctx().clone();
        self.rename_window(&ctx);
        self.history_window(&ctx);
    }

    fn gallery_bar(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("New").clicked() {
                self.new_dupe(menus::NewDupe::Empty);
            }
            if ui.button("New with a baseplate").clicked() {
                self.new_dupe(menus::NewDupe::Baseplate);
            }
            if ui.button("Open a file...").clicked() {
                self.open_with_dialog();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Settings").clicked() {
                    self.show_settings = true;
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let view = &mut self.gallery_view;
            ui.add(egui::TextEdit::singleline(&mut view.query).hint_text("search titles, files, folders, notes").desired_width(260.0));
            ui.label(RichText::new("sort by").color(colours().text_dim));
            egui::ComboBox::from_id_salt("gallery-sort").selected_text(view.sort.name()).show_ui(ui, |ui| {
                for by in SortBy::ALL {
                    ui.selectable_value(&mut view.sort, by, by.name());
                }
            });
            ui.checkbox(&mut view.reversed, "reversed");
            ui.checkbox(&mut view.only_starred, "starred only");
            ui.checkbox(&mut view.show_archived, "show archived");
            if ui.button("Rescan").on_hover_text("Look at the folders again").clicked() {
                view.files = None;
            }
            if view.waiting_for > 0 {
                ui.spinner();
                ui.label(RichText::new(format!("drawing {} thumbnails", view.waiting_for)).color(colours().text_dim).small());
            }
        });
    }

    fn gallery_cards(&mut self, ui: &mut Ui) {
        self.take_rendered();
        if self.gallery_view.files.is_none() {
            let roots = self.gallery_roots();
            let files = gallery::scan(&roots);
            let stale: Vec<PathBuf> = match config::gallery_dir() {
                Some(data) => files.iter().filter(|file| gallery::thumbnail_is_stale(&data, file)).cloned().collect(),
                None => Vec::new(),
            };
            self.gallery_view.roots = roots;
            self.gallery_view.files = Some(files);
            self.start_thumbnails(stale, ui.ctx());
        }
        let view = &self.gallery_view;
        let mut cards = view.index.cards(view.files.as_deref().unwrap_or_default(), &view.roots);
        let archived = cards.iter().filter(|card| card.entry.archived).count();
        cards.retain(|card| (view.show_archived || !card.entry.archived) && (!view.only_starred || card.entry.starred) && gallery::matches(card, &view.query));
        gallery::sort(&mut cards, view.sort, view.reversed);

        if cards.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                let nothing_at_all = view.files.as_ref().is_some_and(|files| files.is_empty());
                ui.label(RichText::new(if nothing_at_all { "No dupes yet" } else { "Nothing matches" }).color(colours().heading).size(22.0));
                ui.label(
                    RichText::new(if nothing_at_all {
                        "Start one with New above, or set the AdvDupe2 folder in Settings so the dupes you saved in game show up here."
                    } else {
                        "Clear the search, or tick show archived."
                    })
                    .color(colours().text_dim),
                );
            });
            return;
        }

        let now = gallery::now();
        let mut act: Option<(PathBuf, CardAction)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(14.0, 14.0);
                for card in &cards {
                    if let Some(action) = self.gallery_card(ui, card, now) {
                        act = Some((card.path.clone(), action));
                    }
                }
            });
            if archived > 0 && !self.gallery_view.show_archived {
                ui.add_space(10.0);
                ui.label(RichText::new(format!("{archived} archived and not shown")).color(colours().text_dim).small());
            }
        });
        if let Some((file, action)) = act {
            self.card_action(file, action);
        }
    }

    fn gallery_card(&mut self, ui: &mut Ui, card: &Card, now: u64) -> Option<CardAction> {
        let c = colours();
        let (rect, resp) = ui.allocate_exact_size(CARD, egui::Sense::click());
        let mut action = None;
        if !ui.is_rect_visible(rect) {
            return None;
        }
        let p = ui.painter_at(rect);
        let fill = if resp.hovered() || resp.has_focus() { c.selection } else { c.chrome };
        p.rect_filled(rect, 0.0, fill);
        // A card is a button to a screen reader and to the Tab key.
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Open {}", card.title)));
        let picture_rect = egui::Rect::from_min_size(rect.min, egui::vec2(CARD.x, PICTURE_HEIGHT));
        p.rect_filled(picture_rect, 0.0, c.background);

        let key = gallery::key_of(&card.path);
        if !self.gallery_view.pictures.contains_key(&key) {
            let loaded = config::gallery_dir().and_then(|data| picture_from(&gallery::thumbnail_path(&data, &card.path), ui.ctx()));
            self.gallery_view.pictures.insert(key.clone(), loaded);
        }
        match self.gallery_view.pictures.get(&key).and_then(|picture| picture.as_ref()) {
            Some(picture) => {
                p.image(picture.id(), picture_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            None => {
                p.text(picture_rect.center(), egui::Align2::CENTER_CENTER, "no picture yet", egui::FontId::proportional(12.0), c.text_dim);
            }
        }

        let star_rect = egui::Rect::from_min_size(egui::pos2(rect.right() - 30.0, rect.top() + 6.0), egui::vec2(24.0, 24.0));
        let star = ui.interact(star_rect, ui.id().with(("gallery-star", &key)), egui::Sense::click());
        if card.entry.starred || resp.hovered() || star.hovered() {
            let colour = if card.entry.starred { c.axis_hot } else { c.text_dim };
            p.text(star_rect.center(), egui::Align2::CENTER_CENTER, if card.entry.starred { "★" } else { "☆" }, egui::FontId::proportional(18.0), colour);
        }
        star.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, card.entry.starred, format!("Star {}", card.title)));
        if star.has_focus() {
            p.rect_stroke(star_rect, 0.0, egui::Stroke::new(2.0, c.axis_hot), egui::StrokeKind::Inside);
        }
        if star.clicked() {
            action = Some(CardAction::Star);
        }

        let left = rect.left() + 10.0;
        let mut y = picture_rect.bottom() + 8.0;
        let title = p.layout_no_wrap(card.title.clone(), egui::FontId::proportional(14.0), if card.entry.archived { c.text_dim } else { c.heading });
        p.galley(egui::pos2(left, y), title, c.heading);
        y += 20.0;
        let contents = match &card.entry.summary {
            Some(summary) => {
                let mut said = format!("{} parts", summary.entities);
                if summary.acf_parts > 0 {
                    said += &format!(" · {} ACF", summary.acf_parts);
                }
                if summary.chips > 0 {
                    said += &format!(" · {} chip{}", summary.chips, if summary.chips == 1 { "" } else { "s" });
                }
                said
            }
            None => "not read yet".to_owned(),
        };
        p.text(egui::pos2(left, y), egui::Align2::LEFT_TOP, format!("{contents} · {}", readable_size(card.bytes)), egui::FontId::proportional(11.5), c.text);
        y += 16.0;
        let opened = if card.entry.opened == 0 { format!("changed {}", gallery::ago(card.modified, now)) } else { format!("opened {}", gallery::ago(card.entry.opened, now)) };
        let place = if card.folder.is_empty() { opened } else { format!("{opened} · {}", card.folder) };
        p.text(egui::pos2(left, y), egui::Align2::LEFT_TOP, place, egui::FontId::proportional(11.5), c.text_dim);

        if resp.has_focus() {
            p.rect_stroke(rect, 0.0, egui::Stroke::new(2.0, c.axis_hot), egui::StrokeKind::Inside);
        }
        let resp = if card.entry.note.is_empty() { resp } else { resp.on_hover_text(card.entry.note.clone()) };
        if resp.clicked() && action.is_none() {
            action = Some(CardAction::Open);
        }
        resp.context_menu(|ui| {
            let mut pick = |ui: &mut Ui, label: &str, chosen: CardAction| {
                if ui.button(label).clicked() {
                    action = Some(chosen);
                    ui.close();
                }
            };
            pick(ui, "Open", CardAction::Open);
            ui.separator();
            pick(ui, "Rename...", CardAction::Rename);
            pick(ui, "Note...", CardAction::Note);
            pick(ui, if card.entry.starred { "Remove the star" } else { "Star" }, CardAction::Star);
            pick(ui, if card.entry.archived { "Bring back from the archive" } else { "Archive" }, CardAction::Archive);
            ui.separator();
            pick(ui, "Duplicate", CardAction::Duplicate);
            pick(ui, "History...", CardAction::History);
            pick(ui, "Redraw the thumbnail", CardAction::Redraw);
            pick(ui, "Show in folder", CardAction::ShowInFolder);
        });
        action
    }

    fn card_action(&mut self, file: PathBuf, action: CardAction) {
        match action {
            CardAction::Open => return self.open_from_gallery(file),
            CardAction::Star => {
                let entry = self.gallery_view.index.entry_mut(&file);
                entry.starred = !entry.starred;
            }
            CardAction::Archive => {
                let entry = self.gallery_view.index.entry_mut(&file);
                entry.archived = !entry.archived;
            }
            CardAction::Rename => {
                let title = self.gallery_view.index.entry(&file).title;
                let shown = if title.is_empty() { file.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default() } else { title };
                self.gallery_view.renaming = Some((file, shown));
                return;
            }
            CardAction::Note => {
                let note = self.gallery_view.index.entry(&file).note;
                self.gallery_view.noting = Some((file, note));
                return;
            }
            CardAction::History => {
                self.gallery_view.history_of = Some(file);
                return;
            }
            CardAction::Duplicate => match gallery::duplicate(&file) {
                Ok(copy) => {
                    self.good(format!("copied to {}", copy.display()));
                    self.gallery_view.files = None;
                }
                Err(e) => self.bad(e),
            },
            CardAction::Redraw => {
                if let Some(data) = config::gallery_dir() {
                    let _ = std::fs::remove_file(gallery::thumbnail_path(&data, &file));
                }
                self.gallery_view.pictures.remove(&gallery::key_of(&file));
                self.gallery_view.files = None;
                return;
            }
            CardAction::ShowInFolder => {
                let opened = if cfg!(windows) {
                    std::process::Command::new("explorer").arg(format!("/select,{}", file.display())).spawn()
                } else if cfg!(target_os = "macos") {
                    std::process::Command::new("open").arg("-R").arg(&file).spawn()
                } else {
                    std::process::Command::new("xdg-open").arg(file.parent().unwrap_or(&file)).spawn()
                };
                if let Err(e) = opened {
                    self.bad(format!("could not open the folder: {e}"));
                }
                return;
            }
        }
        if let Err(e) = self.gallery_view.save_index() {
            self.bad(format!("gallery: {e}"));
        }
    }

    /// The title and the note are the gallery's own; the file keeps its name.
    fn rename_window(&mut self, ctx: &egui::Context) {
        for (which, heading, hint) in [(0, "Title", "shown in the gallery; the file keeps its name"), (1, "Note", "shown when the pointer rests on the card")] {
            let slot = if which == 0 { &mut self.gallery_view.renaming } else { &mut self.gallery_view.noting };
            let Some((file, text)) = slot.as_mut() else { continue };
            let (mut keep, mut close) = (false, false);
            egui::Window::new(heading).id(egui::Id::new(("gallery-text", which))).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(RichText::new(file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()).color(colours().text_dim).small());
                let field = ui.add(egui::TextEdit::singleline(text).desired_width(320.0).hint_text(hint));
                field.request_focus();
                ui.horizontal(|ui| {
                    keep = ui.button("Keep").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter));
                    close = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
                });
            });
            if keep {
                let (file, text) = slot.take().unwrap_or_default();
                let entry = self.gallery_view.index.entry_mut(&file);
                let stem = file.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
                match which {
                    0 => entry.title = if text.trim() == stem { String::new() } else { text.trim().to_owned() },
                    _ => entry.note = text.trim().to_owned(),
                }
                if let Err(e) = self.gallery_view.save_index() {
                    self.bad(format!("gallery: {e}"));
                }
            } else if close {
                *slot = None;
            }
        }
    }

    /// Everything kept for one dupe: each version a save replaced, and the
    /// file as it first arrived. Restoring puts today's file aside first.
    fn history_window(&mut self, ctx: &egui::Context) {
        let (Some(file), Some(data)) = (self.gallery_view.history_of.clone(), config::gallery_dir()) else { return };
        let kept = gallery::versions(&data, &file);
        let now = gallery::now();
        let mut still_open = true;
        let mut restore: Option<PathBuf> = None;
        egui::Window::new("History").id(egui::Id::new("gallery-history")).open(&mut still_open).default_size([420.0, 360.0]).show(ctx, |ui| {
            ui.label(RichText::new(file.display().to_string()).color(colours().text_dim).small());
            if kept.is_empty() {
                ui.add_space(8.0);
                ui.label("Nothing kept yet. The first save in this editor puts the file as it was aside as the original, and every save after that keeps the one it replaces.");
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for version in &kept {
                    ui.horizontal(|ui| {
                        if ui.button("Restore").clicked() {
                            restore = Some(version.path.clone());
                        }
                        let what = if version.original { "the original, as it first arrived".to_owned() } else { format!("replaced {}", gallery::ago(version.kept, now)) };
                        ui.label(RichText::new(what).color(if version.original { colours().heading } else { colours().text }));
                        ui.label(RichText::new(readable_size(version.bytes)).color(colours().text_dim).small());
                    });
                }
            });
        });
        if let Some(version) = restore {
            match gallery::restore(&data, &file, &version, now) {
                Ok(()) => {
                    self.good(format!("restored {}; what it replaced is in its history", file.display()));
                    self.gallery_view.pictures.remove(&gallery::key_of(&file));
                    self.gallery_view.files = None;
                }
                Err(e) => self.bad(e),
            }
        }
        if !still_open {
            self.gallery_view.history_of = None;
        }
    }
}

#[derive(Clone, Copy)]
enum CardAction {
    Open,
    Star,
    Archive,
    Rename,
    Note,
    Duplicate,
    History,
    Redraw,
    ShowInFolder,
}
