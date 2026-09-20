//! What the selected part is set to, as sliders and dropdowns: an ammo
//! crate by its gun, round and the rounds wanted; a fuel tank by its fuel,
//! shape and the litres wanted. Every number goes through
//! `ad2read::containers`, so what is shown is what ACF will build. In
//! newcomer mode this is the left panel.

use super::{colours, round_widgets, section_header, App, Pick};
use ad2read::containers::{self, AmmoCrate, Fuel, Tank, TankShape};
use eframe::egui::{self, RichText, Ui};

/// A number being dragged towards, kept while the drag lasts so the value
/// under the pointer is the one asked for and not the capacity it rounded to.
#[derive(Default)]
pub struct WantedAmount {
    part: Option<f64>,
    amount: f64,
}

impl WantedAmount {
    fn shown(&self, part: f64, actual: f64) -> f64 {
        if self.part == Some(part) { self.amount } else { actual }
    }
}

fn guns_of(dupe: &ad2read::dupe::Dupe) -> Vec<(String, f64)> {
    let mut found: Vec<(String, f64)> = dupe
        .list_entities()
        .into_iter()
        .filter(|(_, class, _)| class.trim_matches('"') == "acf_gun")
        .filter_map(|(index, _, _)| {
            let et = ad2read::transform::entity_table(dupe, index)?;
            let weapon = match dupe.get(et, "Weapon") {
                Some(ad2read::value::Value::Str(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
                _ => return None,
            };
            Some((weapon, dupe.get_number(et, "Caliber")?))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    found.dedup();
    found
}

impl App {
    /// The crate's or tank's own controls. False when the part is neither,
    /// so the caller can fall back to a plain size row.
    pub(crate) fn container_properties(&mut self, ui: &mut Ui, index: f64) -> bool {
        let Some(open) = self.open.as_ref() else { return false };
        if let Some(found) = containers::ammo_crate(&open.dupe, index) {
            let guns = guns_of(&open.dupe);
            self.crate_properties(ui, index, found, &guns);
            return true;
        }
        if let Some(found) = containers::tank(&open.dupe, index) {
            self.tank_properties(ui, index, found);
            return true;
        }
        false
    }

    fn crate_properties(&mut self, ui: &mut Ui, index: f64, found: AmmoCrate, guns: &[(String, f64)]) {
        let dim = colours().text_dim;
        let mut wanted = found.clone();
        let (mut began, mut holding) = (false, false);
        section_header(ui, "AMMO");
        ui.add_space(3.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("gun").color(dim));
            let name = containers::weapon_round(&wanted.weapon).map_or(wanted.weapon.as_str(), |round| round.name);
            egui::ComboBox::from_id_salt("crate-weapon").selected_text(name).show_ui(ui, |ui| {
                for round in containers::WEAPON_ROUNDS {
                    if ui.selectable_label(wanted.weapon == round.id, format!("{} ({})", round.name, round.id)).clicked() && wanted.weapon != round.id {
                        wanted.weapon = round.id.to_owned();
                        wanted.caliber = round.base_caliber;
                    }
                }
            });
            if !guns.is_empty() {
                egui::ComboBox::from_id_salt("crate-match-gun").selected_text("match a gun here").show_ui(ui, |ui| {
                    for (weapon, caliber) in guns {
                        if ui.selectable_label(false, format!("{caliber:.0} mm {weapon}")).clicked() {
                            wanted.weapon = weapon.clone();
                            wanted.caliber = *caliber;
                        }
                    }
                });
            }
        });
        if !containers::fires(&wanted.weapon, &wanted.ammo_type) {
            wanted.ammo_type = containers::AMMO_TYPES.iter().find(|kind| containers::fires(&wanted.weapon, kind)).copied().unwrap_or("AP").to_owned();
        }

        let range = ad2read::catalog::find(&wanted.weapon).and_then(|item| item.range).unwrap_or((20.0, 170.0));
        ui.horizontal(|ui| {
            ui.label(RichText::new("calibre").color(dim));
            let r = ui.add(egui::Slider::new(&mut wanted.caliber, range.0..=range.1).fixed_decimals(0).suffix(" mm"));
            began |= r.drag_started() || r.gained_focus();
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("round").color(dim));
            egui::ComboBox::from_id_salt("crate-ammo-type").selected_text(wanted.ammo_type.clone()).show_ui(ui, |ui| {
                for kind in containers::AMMO_TYPES.iter().filter(|kind| containers::fires(&wanted.weapon, kind)) {
                    if ui.selectable_label(wanted.ammo_type == *kind, *kind).clicked() {
                        wanted.ammo_type = (*kind).to_owned();
                    }
                }
            });
            ui.checkbox(&mut wanted.tracer, "tracer");
            ui.label(RichText::new("crate").color(dim));
            egui::ComboBox::from_id_salt("crate-shape").selected_text(if wanted.drum { "Drum" } else { "Box" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut wanted.drum, false, "Box");
                ui.selectable_value(&mut wanted.drum, true, "Drum");
            });
        });

        if let Some(weapon) = containers::weapon_round(&wanted.weapon) {
            let limits = containers::round_limits(weapon, wanted.caliber, &wanted.ammo_type);
            ui.horizontal(|ui| {
                ui.label(RichText::new("projectile").color(dim));
                let r = ui.add(egui::Slider::new(&mut wanted.projectile, limits.min_projectile..=limits.max_projectile).fixed_decimals(2).suffix(" cm"));
                began |= r.drag_started() || r.gained_focus();
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("propellant").color(dim));
                let r = ui.add(egui::Slider::new(&mut wanted.propellant, limits.min_propellant..=limits.max_propellant).fixed_decimals(2).suffix(" cm"));
                began |= r.drag_started() || r.gained_focus();
            });
            (wanted.projectile, wanted.propellant) = limits.held(wanted.projectile, wanted.propellant);
        }

        let round = wanted.round();
        let mut rounds = self.wanted_amount.shown(index, found.rounds() as f64);
        ui.horizontal(|ui| {
            ui.label(RichText::new("rounds").color(dim));
            let r = ui.add(egui::DragValue::new(&mut rounds).speed(1.0).range(1.0..=100_000.0).fixed_decimals(0));
            began |= r.drag_started() || r.gained_focus();
            holding = r.dragged() || r.has_focus();
            if r.changed() {
                wanted.counts = wanted.holding(rounds as u32).counts;
            }
            ui.label(RichText::new("sizes the crate to hold at least this many").color(dim).small());
        });
        self.wanted_amount = if holding { WantedAmount { part: Some(index), amount: rounds } } else { WantedAmount::default() };

        ui.horizontal(|ui| {
            ui.label(RichText::new("stacked").color(dim));
            let rows: Vec<(&str, &mut u32, u32, u32)> = if wanted.drum {
                let (per_ring, layers) = round.max_drum();
                vec![("a ring ", &mut wanted.counts.0, containers::DRUM_MIN_PER_RING, per_ring), ("layers ", &mut wanted.counts.2, 1, layers)]
            } else {
                let most = round.max_counts(wanted.counts.1, wanted.counts.2);
                vec![("long ", &mut wanted.counts.0, 1, most.0), ("wide ", &mut wanted.counts.1, 1, most.1), ("high ", &mut wanted.counts.2, 1, most.2)]
            };
            for (label, count, least, most) in rows {
                let r = ui.add(egui::DragValue::new(count).speed(0.1).range(least..=most.max(least)).prefix(label));
                began |= r.drag_started() || r.gained_focus();
            }
        });

        let made = wanted.held();
        let size = made.size();
        ui.label(
            RichText::new(format!(
                "holds {} rounds of {:.1} cm · {:.1} x {:.1} x {:.1} units · {:.0} kg empty",
                made.rounds(),
                made.projectile + made.propellant,
                size.0,
                size.1,
                size.2,
                made.empty_mass()
            ))
            .color(colours().text),
        );
        self.apply_container(index, began, wanted != found, |dupe| containers::set_ammo_crate(dupe, index, &wanted).map(|_| ()));
    }

    fn tank_properties(&mut self, ui: &mut Ui, index: f64, found: Tank) {
        let dim = colours().text_dim;
        let mut wanted = found;
        let (mut began, mut holding) = (false, false);
        section_header(ui, "FUEL");
        ui.add_space(3.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("fuel").color(dim));
            egui::ComboBox::from_id_salt("tank-fuel").selected_text(wanted.fuel.name()).show_ui(ui, |ui| {
                for fuel in Fuel::ALL {
                    ui.selectable_value(&mut wanted.fuel, fuel, fuel.name());
                }
            });
            ui.label(RichText::new("shape").color(dim));
            egui::ComboBox::from_id_salt("tank-shape").selected_text(wanted.shape.name()).show_ui(ui, |ui| {
                for shape in TankShape::ALL {
                    ui.selectable_value(&mut wanted.shape, shape, shape.name());
                }
            });
        });

        let mut litres = self.wanted_amount.shown(index, found.litres());
        ui.horizontal(|ui| {
            ui.label(RichText::new("litres").color(dim));
            let r = ui.add(egui::DragValue::new(&mut litres).speed(2.0).range(1.0..=14_000.0).fixed_decimals(0).suffix(" L"));
            began |= r.drag_started() || r.gained_focus();
            holding = r.dragged() || r.has_focus();
            if r.changed() {
                wanted = wanted.holding(litres);
            }
            ui.label(RichText::new("grows it in proportion until it holds this").color(dim).small());
        });
        self.wanted_amount = if holding { WantedAmount { part: Some(index), amount: litres } } else { WantedAmount::default() };

        for (label, side) in [("long", &mut wanted.size.0), ("wide", &mut wanted.size.1), ("high", &mut wanted.size.2)] {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).color(dim));
                let r = ui.add(egui::Slider::new(side, containers::TANK_SIDE.0..=containers::TANK_SIDE.1).fixed_decimals(0).step_by(1.0));
                began |= r.drag_started() || r.gained_focus();
            });
        }

        let what = if wanted.fuel == Fuel::Electric {
            format!("holds {:.1} kWh", wanted.kilowatt_hours())
        } else {
            format!("holds {:.0} L", wanted.litres())
        };
        ui.label(RichText::new(format!("{what} · {:.0} kg empty, {:.0} kg full", wanted.empty_mass(), wanted.full_mass())).color(colours().text));
        self.apply_container(index, began, wanted != found, |dupe| containers::set_tank(dupe, index, &wanted).map(|_| ()));
    }

    /// One undo step per gesture: a drag or a typed number snapshots when
    /// it begins and `container_drag` remembers that until it ends; a
    /// dropdown or tick box snapshots as it changes something.
    fn apply_container(&mut self, index: f64, began: bool, changed: bool, write: impl FnOnce(&mut ad2read::dupe::Dupe) -> Result<(), String>) {
        if began || (changed && self.container_drag != Some(index)) {
            self.snapshot();
        }
        if began {
            self.container_drag = Some(index);
        }
        if !changed {
            return;
        }
        let written = self.open.as_mut().map(|open| {
            open.dirty = true;
            write(&mut open.dupe)
        });
        if let Some(Err(e)) = written {
            self.bad(e);
        }
        self.edit_count += 1;
    }

    /// Newcomer mode's left panel: the selected part and everything about
    /// it that can be set.
    pub(crate) fn properties_panel(&mut self, ui: &mut Ui) {
        round_widgets(ui);
        ui.label(RichText::new("Properties").color(colours().heading).size(15.0));
        ui.add_space(6.0);
        let Some(Pick::Entity(index)) = self.pick else {
            ui.label(
                RichText::new("Click a part to set it up here: a crate's gun, round and how many it holds, a tank's fuel and litres, a gun's calibre, where a part sits. New parts come from the Add menu at the top.")
                    .color(colours().text_dim),
            );
            return;
        };
        let named = self.open.as_ref().and_then(|open| {
            let role = ad2read::roles::of_dupe(&open.dupe).into_iter().find(|(part, _)| *part == index).map(|(_, role)| role.name())?;
            Some(format!("{role} ({index})"))
        });
        ui.label(RichText::new(named.unwrap_or_else(|| format!("entity {index}"))).color(colours().text));
        ui.add_space(6.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.transform_block(ui, index);
            ui.add_space(8.0);
            self.tuning_section(ui, index);
        });
    }
}
