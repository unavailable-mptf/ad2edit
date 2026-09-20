//! The chip editor: a chip's code as text, with line numbers, E2 or Lua
//! colouring from `ad2read::syntax`, the ports the code declares, and a way
//! in from and out to the game's expression2 folder. Reached by
//! right-clicking a chip in the viewport or the tree, or from the inspector.

use std::path::PathBuf;

use super::{colours, App};
use ad2read::syntax::{self, Token};
use ad2read::value::Value;
use eframe::egui::{self, RichText};

pub struct ChipEditor {
    entity: f64,
    title: String,
    text: String,
    /// The text as the chip holds it, so the title can say when they differ.
    in_chip: String,
    /// Starfall main file name and every file, when it is one.
    starfall: Option<(String, Vec<(String, String)>)>,
}

fn highlighted(text: &str, lua: bool) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let c = colours();
    let format = |colour| TextFormat { font_id: egui::FontId::monospace(12.0), color: colour, ..Default::default() };
    let mut job = LayoutJob::default();
    for (range, token) in if lua { syntax::lua(text) } else { syntax::e2(text) } {
        let colour = match token {
            Token::Plain => c.text,
            Token::Comment => c.code_comment,
            Token::Text => c.code_string,
            Token::Directive => c.code_directive,
            Token::Number | Token::Constant => c.code_number,
            Token::Keyword => c.code_keyword,
            Token::Type => c.code_type,
            Token::Function => c.code_function,
            Token::Variable => c.code_variable,
        };
        job.append(&text[range], 0.0, format(colour));
    }
    job
}

impl App {
    pub(crate) fn is_chip(&self, index: f64) -> bool {
        let class = self.open.as_ref().and_then(|open| {
            let et = ad2read::transform::entity_table(&open.dupe, index)?;
            match open.dupe.get(et, "Class") {
                Some(Value::Str(bytes)) => Some(String::from_utf8_lossy(bytes).to_lowercase()),
                _ => None,
            }
        });
        class.is_some_and(|class| class == "gmod_wire_expression2" || class.starts_with("starfall_"))
    }

    pub(crate) fn open_chip_editor(&mut self, index: f64) {
        let Some(open) = self.open.as_ref() else { return };
        match ad2read::build::chip_export(&open.dupe, index) {
            Ok(ad2read::build::Chip::E2 { name, code }) => {
                self.chip_editor = Some(ChipEditor { entity: index, title: format!("E2 {index}: {name}"), in_chip: code.clone(), text: code, starfall: None });
            }
            Ok(ad2read::build::Chip::Starfall { mainfile, files }) => {
                let text = files.iter().find(|(name, _)| *name == mainfile).map(|(_, code)| code.clone()).unwrap_or_default();
                self.chip_editor =
                    Some(ChipEditor { entity: index, title: format!("Starfall {index}: {mainfile}"), in_chip: text.clone(), text, starfall: Some((mainfile, files)) });
            }
            Err(e) => self.bad(format!("chip: {e}")),
        }
    }

    fn expression2_folder(&self) -> Option<PathBuf> {
        let folder = PathBuf::from(&self.config.garrysmod).join("data").join("expression2");
        (!self.config.garrysmod.is_empty() && folder.is_dir()).then_some(folder)
    }

    /// Save writes through chip_import (Ctrl+S while typing does the same);
    /// closing the window discards what was not saved to the chip.
    pub(crate) fn chip_window(&mut self, ctx: &egui::Context) {
        let folder = self.expression2_folder();
        let Some(editor) = self.chip_editor.as_mut() else { return };
        let is_lua = editor.starfall.is_some();
        let changed = editor.text != editor.in_chip;
        let title = format!("{}{}", editor.title, if changed { " (not saved to the chip)" } else { "" });
        let mut still_open = true;
        let mut save = false;
        let mut said: Option<Result<String, String>> = None;
        egui::Window::new(title).id(egui::Id::new("chip-editor")).open(&mut still_open).default_size([720.0, 520.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                save |= ui.add_enabled(changed, egui::Button::new("Save to chip")).on_hover_text("Ctrl+S").clicked();
                if !is_lua {
                    if ui.button("Open a file...").on_hover_text("Replace this code with an E2 from your expression2 folder").clicked() {
                        let mut dialog = rfd::FileDialog::new().set_title("Open an E2").add_filter("Expression 2", &["txt"]);
                        if let Some(folder) = &folder {
                            dialog = dialog.set_directory(folder);
                        }
                        if let Some(path) = dialog.pick_file() {
                            match std::fs::read_to_string(&path) {
                                Ok(code) => {
                                    editor.text = code.replace("\r\n", "\n");
                                    said = Some(Ok(format!("loaded {}; Save to chip to keep it", path.display())));
                                }
                                Err(e) => said = Some(Err(format!("{}: {e}", path.display()))),
                            }
                        }
                    }
                    if ui.button("Save a copy as a file...").on_hover_text("Write this code into your expression2 folder, for the in-game editor").clicked() {
                        let mut dialog = rfd::FileDialog::new().set_title("Save the E2 as a file").add_filter("Expression 2", &["txt"]).set_file_name("chip.txt");
                        if let Some(folder) = &folder {
                            dialog = dialog.set_directory(folder);
                        }
                        if let Some(path) = dialog.save_file() {
                            said = Some(std::fs::write(&path, &editor.text).map(|()| format!("wrote {}", path.display())).map_err(|e| format!("{}: {e}", path.display())));
                        }
                    }
                }
                ui.label(RichText::new(if is_lua { "Starfall main file" } else { "Expression 2. @name keeps the chip's name in step" }).color(colours().text_dim).small());
            });

            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
                let mut job = highlighted(text.as_str(), is_lua);
                job.wrap.max_width = f32::INFINITY;
                ui.fonts_mut(|f| f.layout_job(job))
            };
            let line_count = editor.text.split('\n').count().max(1);
            let gutter = 14.0 + 7.5 * line_count.to_string().len() as f32;
            let mut cursor = None;
            egui::ScrollArea::both().auto_shrink([false, false]).max_height((ui.available_height() - 64.0).max(120.0)).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.add_space(gutter);
                    let output = egui::TextEdit::multiline(&mut editor.text)
                        .id(egui::Id::new("chip-editor-text"))
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_rows(28)
                        .desired_width(f32::INFINITY)
                        .layouter(&mut layouter)
                        .show(ui);
                    cursor = output.cursor_range.map(|range| range.primary.index.0);
                    if output.response.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
                        save = true;
                    }
                    let visible = ui.clip_rect();
                    let right = output.response.rect.left() - 6.0;
                    let (mut number, mut starts_line) = (1usize, true);
                    for row in &output.galley.rows {
                        let y = output.galley_pos.y + row.min_y();
                        if starts_line && y + 16.0 >= visible.top() && y <= visible.bottom() {
                            ui.painter().text(egui::pos2(right, y), egui::Align2::RIGHT_TOP, number.to_string(), egui::FontId::monospace(12.0), colours().code_gutter);
                        }
                        if starts_line {
                            number += 1;
                        }
                        starts_line = row.ends_with_newline;
                    }
                });
            });

            let place = cursor.map(|index| {
                let before: String = editor.text.chars().take(index).collect();
                let line = before.matches('\n').count() + 1;
                let column = before.chars().rev().take_while(|c| *c != '\n').count() + 1;
                format!("line {line}, column {column} · ")
            });
            ui.label(RichText::new(format!("{}{line_count} lines", place.unwrap_or_default())).color(colours().text_dim).small());
            if !is_lua {
                // The ports this code declares, as the Nodes tab and the game
                // will see them once it is saved to the chip.
                let declared = ad2read::ports::of_chip(&editor.text);
                let listed = |side: &[ad2read::ports::Port]| {
                    if side.is_empty() {
                        "none".to_owned()
                    } else {
                        side.iter().map(|port| format!("{} {}", port.name, port.kind.name().to_lowercase())).collect::<Vec<_>>().join(", ")
                    }
                };
                ui.label(RichText::new(format!("inputs: {}", listed(&declared.inputs))).color(colours().text_dim).small());
                ui.label(RichText::new(format!("outputs: {}", listed(&declared.outputs))).color(colours().text_dim).small());
                if !declared.complete {
                    ui.label(RichText::new("a directive names a type this editor does not know; its ports are not checked").color(colours().bad).small());
                }
            }
        });
        match said {
            Some(Ok(message)) => self.good(message),
            Some(Err(message)) => self.bad(message),
            None => {}
        }
        if save {
            self.save_chip();
        }
        if !still_open {
            self.chip_editor = None;
        }
    }

    fn save_chip(&mut self) {
        let Some(editor) = self.chip_editor.as_ref() else { return };
        let (entity, text, starfall) = (editor.entity, editor.text.clone(), editor.starfall.clone());
        let lines = text.split('\n').count();
        let chip = match starfall {
            Some((mainfile, mut files)) => {
                match files.iter_mut().find(|(name, _)| *name == mainfile) {
                    Some((_, code)) => *code = text.clone(),
                    None => files.push((mainfile.clone(), text.clone())),
                }
                ad2read::build::Chip::Starfall { mainfile, files }
            }
            None => ad2read::build::Chip::E2 { name: String::new(), code: text.clone() },
        };
        self.snapshot();
        let result = match self.open.as_mut() {
            Some(open) => ad2read::build::chip_import(&mut open.dupe, entity, chip).map(|_| {
                open.dirty = true;
            }),
            None => Err("no dupe open".into()),
        };
        match result {
            Ok(()) => {
                if let Some(editor) = self.chip_editor.as_mut() {
                    editor.in_chip = text;
                }
                self.good(format!("chip {entity}: {lines} lines saved to the chip; save the dupe to keep it"));
            }
            Err(e) => self.bad(format!("chip: {e}")),
        }
    }
}
