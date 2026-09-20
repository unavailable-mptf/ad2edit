//! The Add window: one search over the whole catalogue and all of SProps,
//! the SProps spawn menu to browse (list, heading, then the sizes as
//! buttons), and the parts starred as favourites. It stays open, so
//! several parts can be added one after another. Insert opens it.

use super::{colours, App, Pick};
use ad2read::catalog::{Item, ITEMS};
use ad2read::sprops;
use eframe::egui::{self, RichText, Ui};

#[derive(Default)]
pub struct AddWindow {
    pub showing: bool,
    query: String,
    list: usize,
    header: usize,
    wants_focus: bool,
}

impl AddWindow {
    pub fn open(&mut self) {
        self.showing = true;
        self.wants_focus = true;
    }

    pub fn open_searching(&mut self, query: &str) {
        self.open();
        self.query = query.to_owned();
    }
}

/// What a row adds.
#[derive(Clone)]
enum Choice {
    Catalogue(&'static Item),
    Prop(&'static str),
}

impl Choice {
    /// How it is kept in the config's favourites.
    fn key(&self) -> String {
        match self {
            Choice::Catalogue(item) => format!("catalogue:{}:{}", item.entity, item.value),
            Choice::Prop(name) => format!("sprops:{name}"),
        }
    }

    fn from_key(key: &str) -> Option<Choice> {
        if let Some(name) = key.strip_prefix("sprops:") {
            return ad2read::sprops_table::GROUPS.iter().flat_map(|group| group.models.iter()).find(|model| **model == name).map(|model| Choice::Prop(model));
        }
        let (entity, value) = key.strip_prefix("catalogue:")?.split_once(':')?;
        ITEMS.iter().find(|item| item.entity == entity && item.value == value).map(Choice::Catalogue)
    }

    fn name(&self) -> String {
        match self {
            Choice::Catalogue(item) => item.name.to_owned(),
            Choice::Prop(name) => sprops::label(name),
        }
    }

    fn kind(&self) -> String {
        match self {
            Choice::Catalogue(item) => item.category.to_owned(),
            Choice::Prop(name) => format!("SProps / {}", name.rsplit_once('/').map_or("", |(folder, _)| folder)),
        }
    }
}

fn catalogue_matches(query: &str, limit: usize) -> Vec<&'static Item> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_owned).collect();
    ITEMS
        .iter()
        .filter(|item| {
            let said = format!("{} {} {} {}", item.name, item.category, item.value, item.note).to_lowercase();
            words.iter().all(|word| said.contains(word.as_str()))
        })
        .take(limit)
        .collect()
}

impl App {
    /// A plain prop in front of the camera, parented to the hull when there
    /// is one. It carries no armour: a stored thickness of 0 would make the
    /// game weigh it at nothing, so it keeps its model's own weight until
    /// it is armoured on purpose.
    fn add_prop(&mut self, name: &'static str) {
        if self.open.is_none() {
            self.new_dupe(super::menus::NewDupe::Empty);
        }
        let at = self.spawn_point();
        let base = self.open.as_ref().and_then(|open| {
            ad2read::roles::of_dupe(&open.dupe).into_iter().find(|(_, role)| *role == ad2read::roles::Role::Hull).map(|(index, _)| index)
        });
        self.snapshot();
        let made = self.open.as_mut().map(|open| {
            let new = ad2read::build::spawn_prop(&mut open.dupe, &sprops::model_path(name), at, (0.0, 0.0, 0.0))?;
            if let Some(base) = base {
                ad2read::build::set_parent(&mut open.dupe, new, base)?;
            }
            ad2read::extras::repair_head(&mut open.dupe);
            Ok::<f64, String>(new)
        });
        match made {
            Some(Ok(new)) => {
                self.pick = Some(Pick::Entity(new));
                self.crumbs.clear();
                self.good(format!(
                    "added {} as entity {new}{}. It has no armour set, so it weighs what the model weighs until you armour it.",
                    sprops::label(name),
                    if base.is_some() { ", parented to the hull" } else { "" }
                ));
            }
            Some(Err(e)) => self.bad(format!("add {}: {e}", sprops::label(name))),
            None => {}
        }
    }

    fn add_choice(&mut self, choice: &Choice) {
        match choice {
            Choice::Catalogue(item) => {
                if self.open.is_none() {
                    self.new_dupe(super::menus::NewDupe::Empty);
                }
                self.add_part(item);
            }
            Choice::Prop(name) => self.add_prop(name),
        }
    }

    /// One row: a star, the name as a button, what kind of thing it is.
    fn choice_row(&mut self, ui: &mut Ui, choice: &Choice, chosen: &mut Option<Choice>) {
        let key = choice.key();
        let starred = self.config.favourites.contains(&key);
        ui.horizontal(|ui| {
            let star = ui.add(egui::Button::new(if starred { "★" } else { "☆" }).frame(false).min_size(egui::vec2(24.0, 24.0))).on_hover_text("Keep in favourites");
            star.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, starred, format!("Keep {} in favourites", choice.name())));
            if star.clicked() {
                if starred {
                    self.config.favourites.retain(|kept| *kept != key);
                } else {
                    self.config.favourites.push(key.clone());
                }
                self.save_config();
            }
            let resp = ui.button(choice.name());
            if let Choice::Catalogue(item) = choice {
                if !item.note.is_empty() {
                    resp.clone().on_hover_text(item.note);
                }
            }
            if resp.clicked() {
                *chosen = Some(choice.clone());
            }
            ui.label(RichText::new(choice.kind()).color(colours().text_dim).small());
        });
    }

    pub(crate) fn add_window(&mut self, ctx: &egui::Context) {
        if !self.add_panel.showing {
            return;
        }
        let mut still_open = true;
        let mut chosen: Option<Choice> = None;
        egui::Window::new("Add").id(egui::Id::new("add-window")).open(&mut still_open).default_size([460.0, 520.0]).show(ctx, |ui| {
            let field = ui.add(egui::TextEdit::singleline(&mut self.add_panel.query).hint_text("search every part and all of SProps: \"cvt\", \"plate 24 48\", \"ranger\"").desired_width(f32::INFINITY));
            if std::mem::take(&mut self.add_panel.wants_focus) {
                field.request_focus();
            }
            ui.add_space(4.0);
            let query = self.add_panel.query.trim().to_owned();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if !query.is_empty() {
                    let parts = catalogue_matches(&query, 40);
                    let props = sprops::search(&query, 80);
                    if parts.is_empty() && props.is_empty() {
                        ui.label(RichText::new("Nothing by that name. Every word has to be in the part's name or kind.").color(colours().text_dim));
                    }
                    for item in parts {
                        self.choice_row(ui, &Choice::Catalogue(item), &mut chosen);
                    }
                    for (_, name) in props {
                        self.choice_row(ui, &Choice::Prop(name), &mut chosen);
                    }
                    return;
                }

                let favourites: Vec<Choice> = self.config.favourites.iter().filter_map(|key| Choice::from_key(key)).collect();
                if !favourites.is_empty() {
                    ui.label(RichText::new("Favourites").color(colours().heading));
                    for choice in &favourites {
                        self.choice_row(ui, choice, &mut chosen);
                    }
                    ui.add_space(8.0);
                }

                ui.label(RichText::new(format!("SProps, all {} of it", sprops::count())).color(colours().heading));
                let lists = sprops::lists();
                self.add_panel.list = self.add_panel.list.min(lists.len().saturating_sub(1));
                let headers = sprops::headers(lists[self.add_panel.list]);
                self.add_panel.header = self.add_panel.header.min(headers.len().saturating_sub(1));
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("add-sprops-list").selected_text(lists[self.add_panel.list]).width(190.0).show_ui(ui, |ui| {
                        for (n, list) in lists.iter().enumerate() {
                            if ui.selectable_label(self.add_panel.list == n, *list).clicked() {
                                self.add_panel.list = n;
                                self.add_panel.header = 0;
                            }
                        }
                    });
                    egui::ComboBox::from_id_salt("add-sprops-header").selected_text(headers[self.add_panel.header].header).width(190.0).show_ui(ui, |ui| {
                        for (n, group) in headers.iter().enumerate() {
                            if ui.selectable_label(self.add_panel.header == n, format!("{}  ({})", group.header, group.models.len())).clicked() {
                                self.add_panel.header = n;
                            }
                        }
                    });
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for name in headers[self.add_panel.header].models {
                        let resp = ui.button(sprops::label(name)).on_hover_text(sprops::model_path(name));
                        if resp.clicked() {
                            chosen = Some(Choice::Prop(name));
                        }
                        resp.context_menu(|ui| {
                            let key = Choice::Prop(name).key();
                            let starred = self.config.favourites.contains(&key);
                            if ui.button(if starred { "Remove from favourites" } else { "Keep in favourites" }).clicked() {
                                if starred {
                                    self.config.favourites.retain(|kept| *kept != key);
                                } else {
                                    self.config.favourites.push(key);
                                }
                                self.save_config();
                                ui.close();
                            }
                        });
                    }
                });
                ui.add_space(8.0);
                ui.label(RichText::new("The ACF, wheel and Wiremod parts are in the Add menu by kind, or type above to find any of them.").color(colours().text_dim).small());
            });
        });
        if let Some(choice) = chosen {
            self.add_choice(&choice);
        }
        if !still_open {
            self.add_panel.showing = false;
        }
    }
}
