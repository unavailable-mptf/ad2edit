//! The node view: every entity that can be connected, as a box with its
//! ports; ACF links and wires as lines between them. Linking and wiring are
//! the same gesture, a drag from a socket, and a connection the game would
//! refuse is refused here, with the reason, before it is made.

use super::{colours, mono_font, ui_font, App, Pick};
use ad2read::graph::{self, Edge, Graph, GraphNode, NodeLayout};
use ad2read::ports::{Port, WireType};
use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Ui};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A connection being dragged out of a socket.
enum Pull {
    Wire { source: f64, output: String, kind: WireType, from: Pos2 },
    Link { entity: f64, from: Pos2 },
}

pub struct NodeView {
    pub showing: bool,
    graph: Graph,
    /// The edit count and file the graph and layout were built for.
    built_at: Option<(u64, PathBuf)>,
    positions: BTreeMap<u64, (f32, f32)>,
    pinned: Vec<f64>,
    pan: egui::Vec2,
    zoom: f32,
    pull: Option<Pull>,
}

impl Default for NodeView {
    fn default() -> Self {
        NodeView {
            showing: false,
            graph: Graph::default(),
            built_at: None,
            positions: BTreeMap::new(),
            pinned: Vec::new(),
            pan: vec2(40.0, 40.0),
            zoom: 1.0,
            pull: None,
        }
    }
}

fn type_colour(kind: WireType) -> Color32 {
    let c = colours();
    match kind {
        WireType::Normal => c.text,
        WireType::Vector | WireType::Vector2 | WireType::Vector4 => c.axis_y,
        WireType::Angle => c.axis_z,
        WireType::String => c.code_string,
        WireType::Entity => c.link_acf,
        WireType::Wirelink => c.link_wire,
        _ => c.accent,
    }
}

fn curve(p: &egui::Painter, from: Pos2, to: Pos2, stroke: Stroke) {
    let reach = ((to.x - from.x).abs() * 0.5).max(40.0);
    let points = [from, from + vec2(reach, 0.0), to - vec2(reach, 0.0), to];
    p.add(egui::epaint::CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, stroke));
}

/// What one frame of interaction asks the app to do, carried out after the
/// drawing borrows are released.
enum Act {
    Wire { source: f64, output: String, target: f64, input: String },
    Link { a: f64, b: f64 },
    Unwire { target: f64, input: String },
    Unlink { owner: f64, target: f64, modifier: String },
    Refused(String),
    Select(f64),
    Pin(f64, bool),
}

impl App {
    /// The middle of the window: a strip to switch between the 3D view and
    /// the node view, then whichever is showing.
    pub fn centre(&mut self, ui: &mut Ui) {
        egui::Frame::default().fill(colours().chrome).inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.selectable_label(!self.node_view.showing, "3D view").clicked() {
                    self.node_view.showing = false;
                }
                if ui.selectable_label(self.node_view.showing, "Nodes").clicked() {
                    self.node_view.showing = true;
                }
                if self.node_view.showing {
                    ui.separator();
                    ui.label(
                        egui::RichText::new("drag a socket to wire or link  ·  right-click a node to disconnect  ·  drag the background to pan, scroll to zoom")
                            .color(colours().text_dim),
                    );
                }
            });
        });
        if self.node_view.showing {
            self.node_view_ui(ui);
        } else {
            self.viewport(ui);
        }
    }

    fn rebuild_node_graph(&mut self) {
        let Some(open) = self.open.as_ref() else { return };
        let stamp = (self.edit_count, open.path.clone());
        if self.node_view.built_at.as_ref() == Some(&stamp) {
            return;
        }
        let new_file = self.node_view.built_at.as_ref().map(|(_, path)| path) != Some(&open.path);
        if new_file {
            self.node_view.pinned.clear();
            self.node_view.positions = NodeLayout::load(&open.path)
                .positions
                .into_iter()
                .filter_map(|(index, at)| index.parse::<f64>().ok().map(|index| (index.to_bits(), at)))
                .collect();
        }
        self.node_view.graph = graph::of_dupe(&open.dupe, &self.node_view.pinned);
        let arranged = graph::arrange(&self.node_view.graph);
        for (entity, at) in arranged {
            self.node_view.positions.entry(entity).or_insert(at);
        }
        self.node_view.built_at = Some(stamp);
    }

    fn save_node_layout(&mut self) {
        let Some(open) = self.open.as_ref() else { return };
        let layout = NodeLayout {
            positions: self.node_view.positions.iter().map(|(bits, at)| (f64::from_bits(*bits).to_string(), *at)).collect(),
        };
        if let Err(e) = layout.save(&open.path) {
            self.bad(format!("node layout not saved: {e}"));
        }
    }

    fn node_view_ui(&mut self, ui: &mut Ui) {
        self.rebuild_node_graph();
        let rect = ui.available_rect_before_wrap();
        let background = ui.allocate_rect(rect, Sense::click_and_drag());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, colours().background);

        if background.dragged() {
            self.node_view.pan += background.drag_delta();
        }
        if background.hovered() {
            let (scroll, pointer) = ui.input(|i| (i.smooth_scroll_delta.y, i.pointer.hover_pos()));
            if scroll != 0.0 {
                let before = self.node_view.zoom;
                let after = (before * (1.0 + scroll * 0.0015)).clamp(0.35, 2.0);
                if let Some(pointer) = pointer {
                    // Keep the point under the pointer where it is.
                    let anchor = pointer - rect.min - self.node_view.pan;
                    self.node_view.pan += anchor - anchor * (after / before);
                }
                self.node_view.zoom = after;
            }
        }

        let zoom = self.node_view.zoom;
        let origin = rect.min + self.node_view.pan;
        let place = |node: &GraphNode, positions: &BTreeMap<u64, (f32, f32)>| {
            let at = positions.get(&node.entity.to_bits()).copied().unwrap_or((0.0, 0.0));
            Rect::from_min_size(origin + vec2(at.0, at.1) * zoom, vec2(graph::NODE_WIDTH, graph::node_height(node)) * zoom)
        };
        let row_y = |node_rect: Rect, row: usize| node_rect.top() + (graph::TITLE_HEIGHT + graph::PORT_HEIGHT * (row as f32 + 0.5)) * zoom;
        let picked = match self.pick {
            Some(Pick::Entity(index)) => Some(index),
            _ => None,
        };

        let graph = self.node_view.graph.clone();
        let rects: BTreeMap<u64, Rect> = graph.nodes.iter().map(|node| (node.entity.to_bits(), place(node, &self.node_view.positions))).collect();
        let socket = |entity: f64, name: Option<&str>, output: bool| -> Option<Pos2> {
            let node = graph.nodes.iter().find(|node| node.entity == entity)?;
            let node_rect = *rects.get(&entity.to_bits())?;
            let side = if output { &node.ports.outputs } else { &node.ports.inputs };
            let row = match name {
                Some(name) => side.iter().position(|port| port.name == name).map(|row| row + 1).unwrap_or(0),
                None => 0,
            };
            Some(pos2(if output { node_rect.right() } else { node_rect.left() }, row_y(node_rect, row)))
        };

        for edge in &graph.edges {
            let (ends, stroke) = match edge {
                Edge::Link { owner, target, .. } => ((socket(*owner, None, true), socket(*target, None, false)), Stroke::new(2.0 * zoom.max(0.6), colours().link_acf)),
                Edge::Wire { source, output, target, input } => (
                    (socket(*source, Some(output), true), socket(*target, Some(input), false)),
                    Stroke::new(1.5 * zoom.max(0.6), colours().link_wire),
                ),
            };
            if let (Some(from), Some(to)) = ends {
                curve(&p, from, to, stroke);
            }
        }

        let mut acts: Vec<Act> = Vec::new();
        let mut moved = false;
        let mut input_sockets: Vec<(f64, Port, Rect)> = Vec::new();
        let font = FontId::proportional(ui_font().size * zoom);
        let small = FontId::monospace(mono_font().size * 0.85 * zoom);
        let reach = 7.0 * zoom.max(0.7);

        for node in &graph.nodes {
            let node_rect = rects[&node.entity.to_bits()];
            if !rect.intersects(node_rect.expand(40.0)) {
                continue;
            }
            let title_rect = Rect::from_min_size(node_rect.min, vec2(node_rect.width(), graph::TITLE_HEIGHT * zoom));
            p.rect_filled(node_rect, CornerRadius::same(6), colours().panel);
            p.rect_filled(title_rect, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, colours().chrome);
            let edge = if picked == Some(node.entity) { Stroke::new(2.0, colours().accent) } else { Stroke::new(1.0, colours().field) };
            p.rect_stroke(node_rect, CornerRadius::same(6), edge, egui::StrokeKind::Outside);
            p.text(title_rect.left_center() + vec2(8.0 * zoom, 0.0), Align2::LEFT_CENTER, &node.title, font.clone(), colours().text);
            p.text(title_rect.right_center() - vec2(8.0 * zoom, 0.0), Align2::RIGHT_CENTER, format!("{}", node.entity), small.clone(), colours().text_dim);

            let id = ui.id().with(("node", node.entity.to_bits()));
            let title = ui.interact(title_rect, id, Sense::click_and_drag());
            if title.dragged() {
                let at = self.node_view.positions.entry(node.entity.to_bits()).or_insert((0.0, 0.0));
                let delta = title.drag_delta() / zoom;
                *at = (at.0 + delta.x, at.1 + delta.y);
            }
            if title.drag_stopped() {
                moved = true;
            }
            if title.clicked() {
                acts.push(Act::Select(node.entity));
            }
            title.context_menu(|ui| {
                for edge in &graph.edges {
                    match edge {
                        Edge::Link { owner, target, modifier } if *owner == node.entity || *target == node.entity => {
                            let other = if *owner == node.entity { *target } else { *owner };
                            if ui.button(format!("Unlink {other}  ({modifier})")).clicked() {
                                acts.push(Act::Unlink { owner: *owner, target: *target, modifier: modifier.clone() });
                                ui.close();
                            }
                        }
                        Edge::Wire { source, output, target, input } if *target == node.entity => {
                            if ui.button(format!("Unwire {input}  (from {source} {output})")).clicked() {
                                acts.push(Act::Unwire { target: *target, input: input.clone() });
                                ui.close();
                            }
                        }
                        _ => {}
                    }
                }
                let is_pinned = self.node_view.pinned.contains(&node.entity);
                if ui.button(if is_pinned { "Unpin from the node view" } else { "Keep in the node view" }).clicked() {
                    acts.push(Act::Pin(node.entity, !is_pinned));
                    ui.close();
                }
            });

            // Row 0 is the link socket; ports follow, inputs left, outputs right.
            let link_at = pos2(node_rect.right(), row_y(node_rect, 0));
            p.text(pos2(node_rect.right() - 10.0 * zoom, link_at.y), Align2::RIGHT_CENTER, "link", small.clone(), colours().link_acf);
            p.circle_filled(link_at, 4.5 * zoom.max(0.7), colours().link_acf);
            p.circle_filled(pos2(node_rect.left(), link_at.y), 3.0 * zoom.max(0.7), colours().link_acf);
            let link_socket = ui.interact(Rect::from_center_size(link_at, vec2(reach, reach) * 2.0), id.with("link"), Sense::drag());
            if link_socket.drag_started() {
                self.node_view.pull = Some(Pull::Link { entity: node.entity, from: link_at });
            }
            link_socket.on_hover_text("Drag onto another node to link them the way the ACF link tool would.");

            for (row, port) in node.ports.inputs.iter().enumerate() {
                let at = pos2(node_rect.left(), row_y(node_rect, row + 1));
                p.circle_filled(at, 4.0 * zoom.max(0.7), type_colour(port.kind));
                p.text(at + vec2(9.0 * zoom, 0.0), Align2::LEFT_CENTER, &port.name, small.clone(), colours().text);
                let hit = Rect::from_center_size(at, vec2(reach, reach) * 2.0);
                input_sockets.push((node.entity, port.clone(), hit));
                let resp = ui.interact(hit, id.with(("in", row)), Sense::click());
                if resp.secondary_clicked() {
                    acts.push(Act::Unwire { target: node.entity, input: port.name.clone() });
                }
                resp.on_hover_text(format!("{:?} input{}{}", port.kind, if port.note.is_empty() { "" } else { ": " }, port.note));
            }
            for (row, port) in node.ports.outputs.iter().enumerate() {
                let at = pos2(node_rect.right(), row_y(node_rect, row + 1));
                p.circle_filled(at, 4.0 * zoom.max(0.7), type_colour(port.kind));
                p.text(at - vec2(9.0 * zoom, 0.0), Align2::RIGHT_CENTER, &port.name, small.clone(), colours().text);
                let resp = ui.interact(Rect::from_center_size(at, vec2(reach, reach) * 2.0), id.with(("out", row)), Sense::drag());
                if resp.drag_started() {
                    self.node_view.pull = Some(Pull::Wire { source: node.entity, output: port.name.clone(), kind: port.kind, from: at });
                }
                resp.on_hover_text(format!("{:?} output{}{}", port.kind, if port.note.is_empty() { "" } else { ": " }, port.note));
            }
        }

        // A pull in progress: the line follows the pointer, and turns to the
        // refusal colour over a socket the game would not accept.
        let (pointer, released) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.any_released()));
        if let (Some(pull), Some(pointer)) = (self.node_view.pull.as_ref(), pointer) {
            match pull {
                Pull::Wire { source, output, kind, from } => {
                    let over = input_sockets.iter().find(|(_, _, hit)| hit.expand(4.0).contains(pointer));
                    let refused = over.is_some_and(|(_, port, _)| !kind.drives(port.kind));
                    curve(&p, *from, pointer, Stroke::new(2.0, if refused { colours().bad } else { colours().link_wire }));
                    if let (true, Some((_, port, _))) = (refused, over) {
                        p.text(pointer + vec2(14.0, -14.0), Align2::LEFT_BOTTOM, format!("a {kind:?} output cannot drive a {:?} input", port.kind), ui_font(), colours().bad);
                    }
                    if released {
                        match over {
                            Some((target, port, _)) if kind.drives(port.kind) => acts.push(Act::Wire {
                                source: *source,
                                output: output.clone(),
                                target: *target,
                                input: port.name.clone(),
                            }),
                            Some((_, port, _)) => acts.push(Act::Refused(format!(
                                "not wired: {output} is a {kind:?} output and {} is a {:?} input",
                                port.name, port.kind
                            ))),
                            None => {}
                        }
                    }
                }
                Pull::Link { entity, from } => {
                    curve(&p, *from, pointer, Stroke::new(2.0, colours().link_acf));
                    if released {
                        let over = rects.iter().find(|(bits, node_rect)| node_rect.contains(pointer) && f64::from_bits(**bits) != *entity);
                        if let Some((bits, _)) = over {
                            acts.push(Act::Link { a: *entity, b: f64::from_bits(*bits) });
                        }
                    }
                }
            }
        }
        if released {
            self.node_view.pull = None;
        }
        if graph.nodes.is_empty() {
            p.text(rect.center(), Align2::CENTER_CENTER, "Nothing here can be linked or wired yet.", ui_font(), colours().text_dim);
        }

        if moved {
            self.save_node_layout();
        }
        for act in acts {
            self.apply_node_act(act);
        }
    }

    fn apply_node_act(&mut self, act: Act) {
        match act {
            Act::Select(entity) => self.pick = Some(Pick::Entity(entity)),
            Act::Refused(why) => self.bad(why),
            Act::Pin(entity, keep) => {
                self.node_view.pinned.retain(|pinned| *pinned != entity);
                if keep {
                    self.node_view.pinned.push(entity);
                }
                self.node_view.built_at = None;
            }
            Act::Wire { source, output, target, input } => {
                self.snapshot();
                let made = self.open.as_mut().map(|open| ad2read::build::add_wire(&mut open.dupe, source, &output, target, &input));
                match made {
                    Some(Ok(())) => self.good(format!("wired {source} {output} -> {target} {input}")),
                    Some(Err(e)) => self.bad(format!("wire: {e}")),
                    None => {}
                }
            }
            Act::Link { a, b } => {
                self.snapshot();
                let made = self.open.as_mut().map(|open| ad2read::build::add_link(&mut open.dupe, a, b));
                match made {
                    Some(Ok(report)) => self.good(format!(
                        "linked {} -> {} via {} ({:.0} units)",
                        report.owner, report.target, report.modifier, report.distance
                    )),
                    Some(Err(e)) => self.bad(format!("link refused: {e}")),
                    None => {}
                }
            }
            Act::Unwire { target, input } => {
                self.snapshot();
                let gone = self.open.as_mut().is_some_and(|open| graph::remove_wire(&mut open.dupe, target, &input));
                if gone {
                    self.good(format!("unwired {target} {input}"));
                }
            }
            Act::Unlink { owner, target, modifier } => {
                self.snapshot();
                let gone = self.open.as_mut().is_some_and(|open| graph::remove_link(&mut open.dupe, owner, target, &modifier));
                if gone {
                    self.good(format!("unlinked {owner} -> {target} ({modifier})"));
                }
            }
        }
    }
}
