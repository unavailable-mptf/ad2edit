//! ad2edit — a GUI over the dupe editor, styled after Source Filmmaker.
//!
//! The look is not invented. The Filmmaker palette in ad2read::theme was read
//! out of SFM's own Qt stylesheets (game/platform/tools/stylesheets/
//! valve_base.qss and sfm.qss) or sampled pixel-by-pixel from its 9-slice
//! button images; the painters here reproduce those widgets shape for shape.
//!
//! Two things about the structure:
//!
//! * The inspector is a general table browser, not a fixed form. Selecting an
//!   entity puts you at that entity's table and any nested table is a row you
//!   can descend into, with a breadcrumb to climb back. That means it can
//!   reach addon data nobody has written a schema for, which is most of it.
//!
//! * Entities get a Transform block above the raw fields, and that block is
//!   the ONLY place position and angle should be edited. It routes through
//!   transform::set_entity_transform, which knows to rotate a ragdoll's other
//!   physics bones. Editing PhysicsObjects[0].Angle by hand further down the
//!   tree would move the root bone and leave the rest behind.

// No console window beside the editor when it is started by a double-click;
// `--console` opens one on purpose.
#![windows_subsystem = "windows"]

use std::path::{Path, PathBuf};

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Margin, Painter, Rect,
    Response, RichText, ScrollArea, Sense, Stroke, TextStyle, Ui,
};

// eframe's own wgpu, not a separately resolved copy: the Device it hands
// us must be the same type our renderer takes.
use eframe::wgpu;

use ad2read::dupe::Dupe;
use ad2read::dupefile::{self, AloneHeader, DupeFile};
use ad2read::history::History;
use ad2read::config::{self, Config, Mode};
// A root file's modules are looked for beside it, where cargo would take
// a second file for a second binary; so the path is spelled out.
#[path = "ad2edit/node_view.rs"]
mod node_view;
#[path = "ad2edit/add_window.rs"]
mod add_window;
#[path = "ad2edit/chip_editor.rs"]
mod chip_editor;
#[path = "ad2edit/gallery_view.rs"]
mod gallery_view;
#[path = "ad2edit/menus.rs"]
mod menus;
#[path = "ad2edit/properties.rs"]
mod properties;

use ad2read::doctor::{Finding, Severity};
use ad2read::gizmo;
use ad2read::theme::Palette;
use ad2read::transform;
use ad2read::value::{table_index, Node, Value};

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------
//
// The values live in ad2read::theme; the painters read this Color32 mirror,
// which is swapped whole when a preset or a single colour changes.

type Colours = ad2read::theme::Palette<Color32>;

static COLOURS: std::sync::RwLock<Option<Colours>> = std::sync::RwLock::new(None);

fn to_colours(p: &ad2read::theme::Palette) -> Colours {
    p.map(|c| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]))
}

fn set_colours(p: &ad2read::theme::Palette) {
    *COLOURS.write().unwrap() = Some(to_colours(p));
}

fn colours() -> Colours {
    COLOURS
        .read()
        .unwrap()
        .unwrap_or_else(|| to_colours(&ad2read::theme::Palette::filmmaker()))
}

/// SFM is NOT a monospace UI — the base QWidget just sets font-size 11px and
/// inherits the system face, and the property editor asks for Tahoma. So
/// labels are proportional and only the value columns and log are monospace,
/// because those are the parts with numbers that have to line up.
fn ui_font() -> FontId {
    FontId::new(12.0, FontFamily::Proportional)
}
fn mono_font() -> FontId {
    FontId::new(11.5, FontFamily::Monospace)
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

fn luminance(c: Color32) -> f32 {
    (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        f(a.r(), b.r()),
        f(a.g(), b.g()),
        f(a.b(), b.b()),
        f(a.a(), b.a()),
    )
}

/// Vertical gradient as one mesh with a vertex pair per stop: the GPU
/// interpolates, so display scaling cannot turn it into stripes.
fn vgrad(p: &Painter, rect: Rect, stops: &[(f32, Color32)]) {
    gradient(p, rect, stops, true);
}

/// Horizontal gradient, the same mesh across the x axis.
fn hgrad(p: &Painter, rect: Rect, stops: &[(f32, Color32)]) {
    gradient(p, rect, stops, false);
}

fn gradient(p: &Painter, rect: Rect, stops: &[(f32, Color32)], vertical: bool) {
    if stops.is_empty() {
        return;
    }
    let mut mesh = egui::Mesh::default();
    for (i, (t, c)) in stops.iter().enumerate() {
        let t = t.clamp(0.0, 1.0);
        let (a, b) = if vertical {
            let y = rect.top() + rect.height() * t;
            (pos2(rect.left(), y), pos2(rect.right(), y))
        } else {
            let x = rect.left() + rect.width() * t;
            (pos2(x, rect.top()), pos2(x, rect.bottom()))
        };
        mesh.colored_vertex(a, *c);
        mesh.colored_vertex(b, *c);
        if i > 0 {
            let k = (i * 2) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

/// A QPushButton: bright hairline top and bottom, shallow gradient between.
fn button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), ui_font(), colours().button_text);
    let size = vec2(galley.size().x + 18.0, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());

    let down = enabled && resp.is_pointer_button_down_on();
    let hot = enabled && resp.hovered();

    let (edge, stops): (Color32, Vec<(f32, Color32)>) = if !enabled {
        (
            colours().button_off_edge,
            vec![(0.0, colours().button_off_top), (1.0, colours().button_off_bottom)],
        )
    } else if down {
        (
            colours().button_pressed_edge,
            vec![(0.0, colours().button_pressed_top), (0.45, colours().button_pressed_mid), (1.0, colours().button_pressed_bottom)],
        )
    } else if hot {
        (colours().button_hover_edge, vec![(0.0, colours().button_hover_top), (1.0, colours().button_hover_bottom)])
    } else {
        (colours().button_edge, vec![(0.0, colours().button_top), (1.0, colours().button_bottom)])
    };

    let p = ui.painter();
    let inner = Rect::from_min_max(
        pos2(rect.left(), rect.top() + 1.0),
        pos2(rect.right(), rect.bottom() - 1.0),
    );
    vgrad(p, inner, &stops);
    p.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, edge));
    p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, edge));

    let color = if !enabled {
        colours().button_text_off
    } else if hot || down {
        colours().button_text_hot
    } else {
        colours().button_text
    };
    // Pressed shifts the label down a pixel, the way SFM's padding does.
    let dy = if down { 1.0 } else { 0.0 };
    p.text(
        rect.center() + vec2(0.0, dy),
        Align2::CENTER_CENTER,
        label,
        ui_font(),
        color,
    );

    resp
}

/// A QToolButton: no chrome at all until you touch it.
fn tool_button(ui: &mut Ui, label: &str) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), ui_font(), colours().text);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 12.0, 20.0), Sense::click());
    let p = ui.painter();

    if resp.is_pointer_button_down_on() {
        p.rect_filled(rect, CornerRadius::same(4), colours().tool_pressed);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(4), colours().tool_hover);
        p.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0, colours().tool_hover_edge),
            egui::StrokeKind::Inside,
        );
    }
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        ui_font(),
        if resp.hovered() { colours().button_text_hot } else { colours().text },
    );
    resp
}

/// A QHeaderView section: dark vertical gradient, bold-ish, left aligned.
fn section_header(ui: &mut Ui, label: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 19.0), Sense::hover());
    let p = ui.painter();
    vgrad(p, rect, &[(0.0, colours().header_top), (1.0, colours().header_bottom)]);
    p.text(
        pos2(rect.left() + 5.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        ui_font(),
        colours().heading,
    );
}

/// A ScrollArea with SFM's scrollbar painted over egui's.
///
/// egui draws the handle as a single flat rect_filled and offers no gradient
/// hook, but there is no need to reimplement scrolling for that: egui keeps
/// handling every bit of input, and we recompute the handle rect with the same
/// formula it uses and paint the gradient on top.
fn sfm_scroll<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let out = ScrollArea::vertical().auto_shrink([false, false]).show(ui, add);

    let view = out.inner_rect.height();
    let content = out.content_size.y;
    if content > view + 0.5 {
        let scroll = ui.style().spacing.scroll.clone();
        let bar = Rect::from_min_max(
            pos2(out.inner_rect.right() - scroll.bar_width, out.inner_rect.top()),
            pos2(out.inner_rect.right(), out.inner_rect.bottom()),
        );
        let span = bar.height();
        let handle_h = (view / content * span).max(scroll.handle_min_length);
        let max_off = (content - view).max(1.0);
        let t = (out.state.offset.y / max_off).clamp(0.0, 1.0);
        let top = bar.top() + t * (span - handle_h);
        let handle = Rect::from_min_max(
            pos2(bar.left() + 1.0, top),
            pos2(bar.right() - 1.0, top + handle_h),
        );

        let p = ui.painter();
        p.rect_filled(bar, 0.0, colours().scrollbar_track);
        hgrad(p, handle, &[(0.0, colours().scrollbar_edge), (0.55, colours().scrollbar_light), (1.0, colours().scrollbar_edge)]);
        p.rect_stroke(
            handle,
            CornerRadius::same(2),
            Stroke::new(1.0, colours().scrollbar_border),
            egui::StrokeKind::Inside,
        );
    }
    out.inner
}

/// QMenuBar: vertical gradient plus a 50%-black hairline along the bottom.
fn menu_bar(p: &Painter, rect: Rect) {
    vgrad(p, rect, &[(0.0, colours().menu_top), (1.0, colours().menu_bottom)]);
    p.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, colours().menu_hairline),
    );
}

/// QHeaderView::section:horizontal.
fn header_bar(p: &Painter, rect: Rect) {
    vgrad(p, rect, &[(0.0, colours().header_top), (1.0, colours().header_bottom)]);
}

/// One QTreeView::item. Selection is a shallow inward bevel — lighter in the
/// middle than at the ends — and every row carries a bottom border the same
/// colour as the background, so rows read as a hairline gap not a stripe.
fn tree_row(p: &Painter, rect: Rect, selected: bool, alternate: bool) {
    if selected {
        vgrad(
            p,
            rect,
            &[
                (0.0, colours().selection),
                (0.55, colours().selection_mid),
                (1.0, colours().selection),
            ],
        );
    } else if alternate {
        p.rect_filled(rect, 0.0, colours().alt_row);
    }
    p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, colours().chrome));
}

fn install_theme(ctx: &egui::Context) {
    // egui 0.36 keeps a separate Style per theme, so there is no single
    // ctx.style()/set_style any more. all_styles_mut runs once per theme;
    // everything below is set explicitly, so both end up identical and the
    // app looks the same whatever the OS reports.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(11.5, FontFamily::Monospace)),
            (TextStyle::Button, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
        ]
        .into();

        let v = &mut style.visuals;
        v.dark_mode = luminance(colours().panel) < 0.5;
        v.panel_fill = colours().panel;
        v.window_fill = colours().panel;
        v.extreme_bg_color = colours().field; // QLineEdit fill from frame.png
        v.faint_bg_color = colours().alt_row;

        v.selection.bg_fill = colours().selection;
        v.selection.stroke = Stroke::new(1.0, colours().selection_text);

        // SFM is square. Only the tool buttons round, at 4px.
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = CornerRadius::same(0);
            w.expansion = 0.0;
        }

        v.widgets.noninteractive.bg_fill = colours().panel;
        v.widgets.noninteractive.weak_bg_fill = colours().panel;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, colours().view_edge);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, colours().text);

        // Drag values and text fields are frame.png: flat fill, hairline edge.
        v.widgets.inactive.bg_fill = colours().field;
        v.widgets.inactive.weak_bg_fill = colours().field;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, colours().field_edge);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, colours().text);

        v.widgets.hovered.bg_fill = colours().field;
        v.widgets.hovered.weak_bg_fill = colours().field;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, colours().field_hover_edge);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, colours().field_hover_text);

        // frame_focus.png: the blue-teal edge is SFM's real accent.
        v.widgets.active.bg_fill = colours().field_focus;
        v.widgets.active.weak_bg_fill = colours().field_focus;
        v.widgets.active.bg_stroke = Stroke::new(1.0, colours().accent);
        v.widgets.active.fg_stroke = Stroke::new(1.0, colours().field_focus_text);

        v.widgets.open.bg_fill = colours().field;
        v.widgets.open.weak_bg_fill = colours().field;
        v.widgets.open.bg_stroke = Stroke::new(1.0, colours().field_edge);
        v.widgets.open.fg_stroke = Stroke::new(1.0, colours().text);

        // QSplitter is rgb(24,24,25) — the darkest surface in the app.
        v.widgets.noninteractive.bg_fill = colours().panel;
        v.window_stroke = Stroke::new(1.0, colours().splitter);

        style.spacing.item_spacing = vec2(5.0, 3.0);
        style.spacing.indent = 14.0;
        style.spacing.interact_size = vec2(24.0, 18.0);
    });
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
enum Pick {
    Entity(f64),
    /// Position in the root Constraints array.
    Constraint(usize),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum SuspensionChoice {
    Simple,
    Locked,
    Sprung,
}

/// One step of the inspector's navigation path: a label and the arena table
/// it points at.
#[derive(Clone)]
struct Crumb {
    label: String,
    table: usize,
}

struct Loaded {
    path: PathBuf,
    file: DupeFile,
    lzma: (u32, u32, u32, u32, bool),
    dupe: Dupe,
    history: History,
    dirty: bool,
}

struct App {
    open: Option<Loaded>,
    pick: Option<Pick>,
    crumbs: Vec<Crumb>,
    /// Contents panel as a parent hierarchy instead of a flat list.
    tree_view: bool,
    /// Link mode: the source entity waiting for a target click.
    link_mode: Option<f64>,
    /// Wheel action choices: base and gearbox entity indices.
    wheel_base: Option<f64>,
    wheel_gearbox: Option<f64>,
    /// 25: suspension recipe chosen in the BUILD panel.
    suspension: SuspensionChoice,
    /// 27: chip editor — the entity being edited, its text, the last export.
    chip_editor: Option<chip_editor::ChipEditor>,
    /// What the person chose: mode, look, folders, habits. Saved on exit and
    /// whenever the settings window closes.
    config: Config,
    /// The live palette the settings window edits; `config` stores only the
    /// difference from its preset.
    palette: Palette,
    /// The first-run card, while it is showing.
    setup: Option<Setup>,
    show_settings: bool,
    /// Newcomer mode: the cached checklist and the armour target.
    newcomer_panels: NewcomerPanels,
    /// Bumped on every snapshot and load, so caches know when to refresh.
    edit_count: u64,
    last_autosave: std::time::Instant,
    filter: String,
    log: Vec<(String, Color32)>,

    /// Entities ticked for an operation, by ctrl-clicking the tree. Empty
    /// means "just whatever is selected" — duplicating one prop and
    /// duplicating a whole turret are the same command with a different set.
    camera: Camera,
    clipboard: Option<Dupe>,
    /// Resolved model sizes. A dupe stores no dimensions, so without this
    /// every prop is drawn the same size — which is what the uniform cubes
    /// were. Populated on demand from the game folder.
    models: Option<ad2read::models::ModelLibrary>,
    /// Colour target for the mesh renderer, and the texture it gets uploaded
    /// to. Kept between frames so neither is reallocated every repaint.
    /// GPU renderer, built lazily from eframe's wgpu state. When it is
    /// present the CPU rasteriser below is unused; it stays as the fallback
    /// for a machine where the wgpu backend didn't come up.
    gpu: Option<ad2read::gpu::Renderer>,
    gpu_meshes: std::collections::HashMap<String, std::sync::Arc<ad2read::gpu::GpuMesh>>,
    /// Models already reported as unloadable, so the log says it once.
    mesh_failures: std::collections::HashSet<String>,
    /// The loaded map, its GPU batches (one per material, same order as
    /// map.batches), and the name typed into the loader.
    map: Option<ad2read::map::Map>,
    map_batches: Vec<ad2read::gpu::MapBatch>,
    map_name: String,
    show_map: bool,
    /// Last frame's culling result, shown in the ops panel.
    map_stats: (usize, usize, usize),
    /// Skybox face meshes, if the map declared a skyname and they resolved.
    sky: Vec<std::sync::Arc<ad2read::gpu::GpuMesh>>,
    /// Cursor is hidden and being re-centred for a look drag.
    looking: bool,
    scene_tex: Option<egui::TextureId>,
    scene_size: (u32, u32),
    render_state: Option<eframe::egui_wgpu::RenderState>,
    target: ad2read::raster::Target,
    frame_tex: Option<egui::TextureHandle>,
    /// Named selection sets: a label and the entity indices in it.
    sets: Vec<(String, Vec<f64>)>,
    new_set_name: String,
    show_links: bool,
    show_wires: bool,
    /// Where each turret can point: its traverse sweep and elevation fan.
    show_arcs: bool,
    new_mod_name: String,
    new_field_name: String,
    new_field_kind: &'static str,
    gizmo: Gizmo,
    /// Where the right-click / double-click menu is showing, if it is.
    menu_at: Option<egui::Pos2>,
    /// The gizmo handle the pointer went down on. It owns the press from
    /// that moment, so the camera's mouse look never sees the drag begin.
    grab: Option<Grab>,
    /// Which gizmo axis has the drag, if any.
    drag_axis: Option<usize>,
    /// Entities the game draws nothing solid for (alpha 0, an additive
    /// material), found by the mesh pass and outlined by the box pass.
    ghosts: Vec<f64>,
    /// Whether each material seen so far draws nothing solid.
    see_through: std::collections::HashMap<String, bool>,
    /// When the camera last moved by the keys. Ctrl slows a flight, and A, S
    /// and D steer it, so Ctrl+A, Ctrl+S and Ctrl+D must not fire mid-flight.
    last_flew: Option<std::time::Instant>,
    /// The step-by-step tutorial window.
    tutorial: menus::TutorialPanel,
    wanted_amount: properties::WantedAmount,
    gallery_view: gallery_view::GalleryView,
    add_panel: add_window::AddWindow,
    /// `--screenshot`: a picture of the window to write, then exit.
    screenshot: Option<Screenshot>,
    /// The crate or tank whose slider is mid-drag, so the drag is one undo step.
    container_drag: Option<f64>,
    /// The weight headline and its detail, as of an edit count.
    weight_cache: Option<(u64, String, String)>,
    /// Where the weight balances, as of an edit count.
    balance_cache: Option<(u64, Option<V3>)>,
    /// How many copies Duplicate makes; more than one is a spaced row.
    array_count: usize,
    mirror_props_only: bool,
    /// Roles switched off in View > Show, by name: neither drawn nor picked.
    hidden_roles: std::collections::HashSet<&'static str>,
    /// Which entities those are, as of an edit count.
    hidden_cache: Option<(u64, std::collections::HashSet<u64>)>,
    /// Advanced mode: the newcomer checklist as a window.
    show_checklist: bool,
    /// Settings > Keys: the slot waiting for its new key.
    capturing_key: Option<usize>,
    /// An autosave newer than the dupe just opened, waiting for an answer.
    autosave_offer: Option<PathBuf>,
    /// The node view: its graph, where each node sits, and the tab that shows it.
    node_view: node_view::NodeView,
    /// Boxes are uniform: a dupe stores no bounding volume, so real sizes have
    /// to wait for Phase 3's model loading. Adjustable so a build of gears and
    /// a build of hull plates can both be read.
    box_size: f64,
    marked: Vec<f64>,
    op_offset: (f64, f64, f64),
    where_key: String,
    where_val: String,
    set_key: String,
    set_val: String,
}

impl Default for App {
    fn default() -> Self {
        App {
            open: None,
            pick: None,
            crumbs: Vec::new(),
            tree_view: false,
            link_mode: None,
            wheel_base: None,
            wheel_gearbox: None,
            suspension: SuspensionChoice::Simple,
            chip_editor: None,
            config: Config::default(),
            palette: Palette::default(),
            setup: None,
            show_settings: false,
            newcomer_panels: NewcomerPanels::default(),
            edit_count: 0,
            last_autosave: std::time::Instant::now(),
            filter: String::new(),
            log: Vec::new(),
            camera: Camera::default(),
            clipboard: None,
            models: None,
            gpu: None,
            gpu_meshes: std::collections::HashMap::new(),
            mesh_failures: std::collections::HashSet::new(),
            map: None,
            map_batches: Vec::new(),
            map_name: "gm_construct".to_owned(),
            show_map: true,
            map_stats: (0, 0, 0),
            sky: Vec::new(),
            looking: false,
            scene_tex: None,
            scene_size: (0, 0),
            render_state: None,
            target: ad2read::raster::Target::new(2, 2),
            frame_tex: None,
            sets: Vec::new(),
            new_set_name: String::new(),
            show_links: true,
            show_wires: true,
            show_arcs: true,
            new_mod_name: String::new(),
            new_field_name: String::new(),
            new_field_kind: "number",
            gizmo: Gizmo::Move,
            menu_at: None,
            grab: None,
            ghosts: Vec::new(),
            see_through: std::collections::HashMap::new(),
            last_flew: None,
            tutorial: menus::TutorialPanel::default(),
            wanted_amount: properties::WantedAmount::default(),
            gallery_view: gallery_view::GalleryView::default(),
            add_panel: add_window::AddWindow::default(),
            screenshot: None,
            container_drag: None,
            weight_cache: None,
            balance_cache: None,
            array_count: 1,
            mirror_props_only: true,
            hidden_roles: std::collections::HashSet::new(),
            hidden_cache: None,
            show_checklist: false,
            capturing_key: None,
            autosave_offer: None,
            node_view: node_view::NodeView::default(),
            drag_axis: None,
            box_size: 16.0,
            marked: Vec::new(),
            op_offset: (0.0, 0.0, 0.0),
            where_key: String::new(),
            where_val: String::new(),
            set_key: String::new(),
            set_val: String::new(),
        }
    }
}

impl App {
    fn say(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        if verbose() {
            println!("[log] {m}");
        }
        self.log.push((m, colours().text));
    }
    fn good(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        if verbose() {
            println!("[ok ] {m}");
        }
        self.log.push((m, colours().good));
    }
    fn bad(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        if verbose() {
            eprintln!("[ERR] {m}");
        }
        self.log.push((m, colours().bad));
    }

    fn load(&mut self, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return self.bad(format!("could not read: {e}")),
        };
        let file = match dupefile::parse(&bytes) {
            Ok(f) => f,
            Err(e) => return self.bad(e),
        };
        if file.revision != 5 {
            return self.bad(format!("revision {} is not supported", file.revision));
        }
        let Some(header) = AloneHeader::parse(&file.compressed) else {
            return self.bad("compressed body too short for an LZMA header");
        };
        let lzma = (
            header.lc,
            header.lp,
            header.pb,
            header.dict,
            header.size != u64::MAX,
        );
        let raw = match dupefile::decompress(&file.compressed) {
            Ok(r) => r,
            Err(e) => return self.bad(format!("decompression failed: {e}")),
        };
        let (dupe, used) = match Dupe::from_body(&raw) {
            Ok(d) => d,
            Err(e) => return self.bad(format!("decode failed: {e}")),
        };
        if used != raw.len() {
            self.bad(format!("{} bytes left over after decoding", raw.len() - used));
        }

        // Same guard the CLI opens with: if we can't reproduce the file we
        // read, nothing we write afterwards can be trusted.
        match dupe.to_body() {
            Ok(out) if out == raw => self.good("round trip: byte-identical"),
            Ok(out) => self.bad(format!(
                "round trip MISMATCH: {} in, {} out. Editing is unsafe.",
                raw.len(),
                out.len()
            )),
            Err(e) => self.bad(format!("re-encode failed: {e}")),
        }

        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.say(format!(
            "{name}: {} entities, {} constraints, {} tables",
            dupe.list_entities().len(),
            constraint_list(&dupe).len(),
            dupe.arena.len()
        ));

        let history = History::new(&dupe);
        self.open = Some(Loaded {
            path,
            file,
            lzma,
            dupe,
            history,
            dirty: false,
        });
        self.pick = None;
        self.crumbs.clear();
        self.marked.clear();
        self.frame_camera();

        // An AdvDupe2 file lives inside garrysmod/data/advdupe2, so the game
        // folder is right there in the path we were just given. No reason to
        // make anyone go and find it by hand.
        if self.models.is_none() {
            let configured = Some(PathBuf::from(&self.config.garrysmod))
                .filter(|p| !self.config.garrysmod.is_empty() && p.is_dir());
            let found = configured.or_else(|| {
                ad2read::models::find_garrysmod(self.open.as_ref().map(|o| o.path.as_path()))
            });
            match found {
                Some(dir) => self.index_models(dir),
                None => self.say("couldn't find your garrysmod folder; point at it in Settings"),
            }
        }
        // An autosave newer than the file means a session ended without
        // saving; say so, and where it is.
        if let Some(open) = self.open.as_ref() {
            let auto = Self::autosave_path(&open.path);
            let newer = match (std::fs::metadata(&auto), std::fs::metadata(&open.path)) {
                (Ok(a), Ok(f)) => a.modified().ok() > f.modified().ok(),
                _ => false,
            };
            if newer {
                self.say(format!("a newer autosave exists: {}", auto.display()));
                self.autosave_offer = Some(auto);
            }
        }
        self.last_autosave = std::time::Instant::now();
        self.edit_count += 1;
        if let Some(path) = self.open.as_ref().map(|open| open.path.clone()) {
            self.note_opened(&path);
        }
    }

    /// An autosave newer than the file means a session ended without saving.
    /// Restoring opens the autosave's contents under the dupe's own name,
    /// unsaved, so nothing on disk changes until Save.
    fn autosave_offer_window(&mut self, ctx: &egui::Context) {
        let Some(auto) = self.autosave_offer.clone() else { return };
        let mut answer: Option<bool> = None;
        egui::Window::new("Unsaved work found").id(egui::Id::new("autosave-offer")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label("An autosave of this dupe is newer than the file: the editor last closed with changes that were never saved.");
            ui.label(RichText::new(auto.display().to_string()).color(colours().text_dim).small());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if button(ui, "Restore the autosave", true).clicked() {
                    answer = Some(true);
                }
                if button(ui, "Keep the file as it is", true).clicked() {
                    answer = Some(false);
                }
            });
        });
        let Some(restore) = answer else { return };
        self.autosave_offer = None;
        if !restore {
            return;
        }
        let Some(path) = self.open.as_ref().map(|open| open.path.clone()) else { return };
        self.load(auto);
        self.autosave_offer = None;
        if let Some(open) = self.open.as_mut() {
            open.path = path;
            open.dirty = true;
        }
        self.good("restored the autosave; it is unsaved until you Save");
    }

    /// The open dupe as file bytes, after decoding what was produced so a
    /// broken encode never reaches disk.
    fn encode_current(&self) -> Result<Vec<u8>, String> {
        let open = self.open.as_ref().ok_or("nothing open")?;
        let body = open.dupe.to_body().map_err(|e| format!("encode failed: {e}"))?;
        match Dupe::from_body(&body) {
            Ok((_, used)) if used == body.len() => {}
            Ok((_, used)) => return Err(format!("output has {} trailing bytes", body.len() - used)),
            Err(e) => return Err(format!("output does not decode: {e}")),
        }
        let (lc, lp, pb, dict, real) = open.lzma;
        let compressed = dupefile::compress(&body, lc, lp, pb, dict, real).map_err(|e| format!("compression failed: {e}"))?;
        let mut info = open.file.info.clone();
        for (k, v) in info.iter_mut() {
            if k.as_slice() == b"size" {
                *v = compressed.len().to_string().into_bytes();
            }
        }
        Ok(dupefile::build(open.file.revision, &info, &compressed))
    }

    /// The autosave path beside the file: `name.autosave.txt`.
    fn autosave_path(path: &std::path::Path) -> PathBuf {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "dupe".into());
        path.with_file_name(format!("{stem}.autosave.txt"))
    }

    /// Writes an autosave when the dupe is dirty and the interval has
    /// passed. Never touches the real file; the log says where it went.
    fn autosave_tick(&mut self) {
        let every = self.config.autosave.seconds;
        if every == 0 || self.last_autosave.elapsed().as_secs() < every {
            return;
        }
        self.last_autosave = std::time::Instant::now();
        let Some(open) = self.open.as_ref() else { return };
        if !open.dirty {
            return;
        }
        let path = Self::autosave_path(&open.path);
        match self.encode_current() {
            Ok(bytes) => match std::fs::write(&path, &bytes) {
                Ok(()) => self.say(format!("autosaved to {}", path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())),
                Err(e) => self.bad(format!("autosave failed: {e}")),
            },
            Err(e) => self.bad(format!("autosave: {e}")),
        }
    }

    fn save_as(&mut self, path: PathBuf) {
        // resync first. It's idempotent and a proven no-op when nothing has
        // moved, so running it unconditionally costs nothing and removes any
        // chance of shipping a stale constraint pose.
        let Some(open) = self.open.as_mut() else { return };
        let r = transform::resync_constraints(&mut open.dupe);
        if r.updated > 0 || r.world_anchored > 0 || r.bones > 0 {
            let msg = format!(
                "resynced {} constraint pose(s), {} world-anchored, {} weld bone pose(s)",
                r.updated, r.world_anchored, r.bones
            );
            self.say(msg);
        }

        let out = match self.encode_current() {
            Ok(b) => b,
            Err(e) => return self.bad(e),
        };
        // Whatever this save replaces is put aside first; the original never goes.
        self.keep_before_saving(&path);
        let Some(open) = self.open.as_mut() else { return };

        match std::fs::write(&path, &out) {
            Ok(()) => {
                open.path = path.clone();
                open.dirty = false;
                let n = path.file_name().map(|s| s.to_string_lossy().into_owned());
                self.good(format!("wrote {} ({} bytes)", n.unwrap_or_default(), out.len()));
            }
            Err(e) => self.bad(format!("could not write: {e}")),
        }
    }

    /// Called before any edit, so undo has something to return to.
    /// Whether the camera is being flown: the mouse is looking around, or the
    /// keys moved it a moment ago.
    fn flying(&self) -> bool {
        self.looking || self.last_flew.is_some_and(|at| at.elapsed().as_secs_f32() < 0.6)
    }

    fn snapshot(&mut self) {
        if let Some(open) = self.open.as_mut() {
            let d = &open.dupe;
            open.history.push(d);
            open.dirty = true;
        }
        self.edit_count += 1;
    }
}

/// A gizmo handle held by the pointer: where the entity was when it was
/// grabbed and what the pointer read then. Every frame of the drag is
/// applied to that starting pose, never to the previous frame's result.
struct Grab {
    axis: usize,
    origin: V3,
    angle: V3,
    /// Distance along the axis at the grab, for a move.
    along: Option<f64>,
    /// Where on the ring the grab was, for the wedge's first edge.
    ring_start: f64,
    /// The ring reading on the previous frame, and whether it came from the
    /// screen because the ring was edge-on.
    ring_last: Option<f64>,
    ring_from_screen: bool,
    /// The change so far: units along the axis for a move, radians round
    /// the ring for a turn.
    change: f64,
    /// The part's size at the grab, for a scale.
    size: Option<V3>,
    /// The other ticked parts and where they were at the grab: when the
    /// part being dragged is one of the ticked, they all go together.
    others: Vec<(f64, V3, V3)>,
}

/// The first-run wizard: which step is showing, the game folder it proposes,
/// and the blurred editor it floats over.
struct Setup {
    step: usize,
    garrysmod: String,
    found_automatically: bool,
    backdrop: Option<egui::TextureHandle>,
    screenshot_requested: bool,
    frames_without_screenshot: u32,
}

impl Setup {
    fn new(config: &Config) -> Self {
        let (garrysmod, found_automatically) = if !config.garrysmod.is_empty() {
            (config.garrysmod.clone(), false)
        } else {
            match ad2read::models::find_garrysmod(None) {
                Some(dir) => (dir.display().to_string(), true),
                None => (String::new(), false),
            }
        };
        Setup {
            step: 0,
            garrysmod,
            found_automatically,
            backdrop: None,
            screenshot_requested: false,
            frames_without_screenshot: 0,
        }
    }
}

const SETUP_STEPS: usize = 3;

/// The editor shrunk eight times and box-blurred twice; bilinear upscaling
/// does the rest. A few milliseconds, once.
fn blurred_backdrop(ctx: &egui::Context, image: &egui::ColorImage) -> egui::TextureHandle {
    let [w, h] = image.size;
    let f = 8usize;
    let (sw, sh) = ((w / f).max(1), (h / f).max(1));
    let mut small = vec![[0f32; 3]; sw * sh];
    for y in 0..sh {
        for x in 0..sw {
            let mut acc = [0f32; 3];
            let mut n = 0.0;
            for dy in 0..f {
                for dx in 0..f {
                    let (sx, sy) = (x * f + dx, y * f + dy);
                    if sx < w && sy < h {
                        let c = image.pixels[sy * w + sx];
                        acc[0] += c.r() as f32;
                        acc[1] += c.g() as f32;
                        acc[2] += c.b() as f32;
                        n += 1.0;
                    }
                }
            }
            small[y * sw + x] = [acc[0] / n, acc[1] / n, acc[2] / n];
        }
    }
    for _ in 0..2 {
        let src = small.clone();
        for y in 0..sh {
            for x in 0..sw {
                let mut acc = [0f32; 3];
                let mut n = 0.0;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (sx, sy) = (x as i32 + dx, y as i32 + dy);
                        if sx >= 0 && sy >= 0 && (sx as usize) < sw && (sy as usize) < sh {
                            let c = src[sy as usize * sw + sx as usize];
                            acc[0] += c[0];
                            acc[1] += c[1];
                            acc[2] += c[2];
                            n += 1.0;
                        }
                    }
                }
                small[y * sw + x] = [acc[0] / n, acc[1] / n, acc[2] / n];
            }
        }
    }
    let pixels = small
        .iter()
        .map(|c| Color32::from_rgb(c[0] as u8, c[1] as u8, c[2] as u8))
        .collect();
    ctx.load_texture(
        "setup-backdrop",
        egui::ColorImage::new([sw, sh], pixels),
        egui::TextureOptions::LINEAR,
    )
}

/// Dark or light text for a fill, whichever reads.
fn text_on(fill: Color32) -> Color32 {
    if luminance(fill) > 0.55 {
        Color32::from_rgb(20, 22, 24)
    } else {
        Color32::from_rgb(245, 246, 248)
    }
}

/// The wizard's button: flat and rounded, the primary one in the accent.
fn soft_button(ui: &mut Ui, label: &str, primary: bool, enabled: bool) -> Response {
    let font = FontId::new(13.0, FontFamily::Proportional);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE);
    let size = vec2(galley.size().x + 34.0, 36.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let hot = ui.ctx().animate_bool(resp.id.with("hot"), enabled && resp.hovered());
    let down = enabled && resp.is_pointer_button_down_on();
    let c = colours();
    let (mut fill, text) = if primary {
        let fill = mix(c.accent, Color32::WHITE, 0.14 * hot);
        (fill, text_on(fill))
    } else {
        (mix(c.field, c.button_top, 0.25 + 0.5 * hot), c.heading)
    };
    if down {
        fill = mix(fill, Color32::BLACK, 0.18);
    }
    if !enabled {
        fill = mix(fill, c.panel, 0.65);
    }
    let text = if enabled { text } else { c.text_dim };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), fill);
    if !primary {
        painter.rect_stroke(
            rect,
            CornerRadius::same(10),
            Stroke::new(1.0, c.field_edge),
            egui::StrokeKind::Inside,
        );
    }
    let dy = if down { 1.0 } else { 0.0 };
    painter.text(rect.center() + vec2(0.0, dy), Align2::CENTER_CENTER, label, font, text);
    resp
}

/// A selectable card with a title and a line of explanation.
fn choice_card(ui: &mut Ui, id: egui::Id, title: &str, blurb: &str, selected: bool, width: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 104.0), Sense::click());
    let hot = ui.ctx().animate_bool(id.with("hot"), resp.hovered());
    let on = ui.ctx().animate_bool(id.with("on"), selected);
    let c = colours();
    let rest = mix(c.panel, c.chrome, 0.35);
    let fill = mix(mix(rest, c.accent, 0.14 * on), Color32::WHITE, 0.03 * hot);
    let edge = mix(c.view_edge, c.accent, on);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(12), fill);
    painter.rect_stroke(
        rect,
        CornerRadius::same(12),
        Stroke::new(1.0 + on, edge),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink(16.0);
    painter.text(
        inner.left_top(),
        Align2::LEFT_TOP,
        title,
        FontId::new(15.0, FontFamily::Proportional),
        c.heading,
    );
    let body = painter.layout(
        blurb.to_owned(),
        FontId::new(12.0, FontFamily::Proportional),
        c.text_dim,
        inner.width(),
    );
    painter.galley(inner.left_top() + vec2(0.0, 26.0), body, c.text_dim);
    resp
}

/// One dot per step, the current one in the accent.
fn step_dots(ui: &mut Ui, count: usize, current: usize) {
    let (rect, _) = ui.allocate_exact_size(vec2(count as f32 * 16.0, 10.0), Sense::hover());
    let c = colours();
    for i in 0..count {
        let centre = pos2(rect.left() + 8.0 + i as f32 * 16.0, rect.center().y);
        let on = ui.ctx().animate_bool(egui::Id::new(("setup-dot", i)), i == current);
        ui.painter().circle_filled(centre, 3.0 + on, mix(c.field_edge, c.accent, on));
    }
}

/// A small rounded pill for list rows.
fn soft_chip(ui: &mut Ui, label: &str) -> Response {
    let font = FontId::new(11.5, FontFamily::Proportional);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 18.0, 24.0), Sense::click());
    let hot = ui.ctx().animate_bool(resp.id.with("hot"), resp.hovered());
    let c = colours();
    let fill = mix(mix(c.field, c.accent, 0.35), c.accent, 0.5 * hot);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(12), fill);
    painter.text(rect.center(), Align2::CENTER_CENTER, label, font, text_on(fill));
    resp
}

/// The catalogue's top-level groups ("Engines / Piston I6" is Engines).
fn catalogue_groups() -> Vec<(&'static str, usize)> {
    let mut groups: Vec<(&'static str, usize)> = Vec::new();
    for item in ad2read::catalog::ITEMS {
        let g = group_of(item.category);
        match groups.iter_mut().find(|(name, _)| *name == g) {
            Some((_, n)) => *n += 1,
            None => groups.push((g, 1)),
        }
    }
    groups
}

fn group_of(category: &str) -> &str {
    category.split(" / ").next().unwrap_or(category)
}

/// Newcomer-mode state: the checklist cached against the edit count, and
/// the armour target in tonnes.
struct NewcomerPanels {
    findings: Vec<Finding>,
    findings_at_edit: Option<u64>,
    armour_tonnes: f64,
    /// The glacis angle a generated hull is asked for, from the vertical.
    glacis_degrees: f64,
}

impl Default for NewcomerPanels {
    fn default() -> Self {
        NewcomerPanels {
            findings: Vec::new(),
            findings_at_edit: None,
            armour_tonnes: 40.0,
            glacis_degrees: 62.0,
        }
    }
}

/// A fresh dupe as file bytes, the way the CLI's build writes one.
fn dupe_file_bytes(dupe: &ad2read::dupe::Dupe, name: &str) -> Result<Vec<u8>, String> {
    ad2read::buildfile::file_bytes(dupe, name)
}

/// `stem.txt` in `dir`, or `stem-2.txt` and so on, whichever is free.
fn unique_path(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.txt"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem}-{n}.txt")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Rounded fields inside the wizard, without touching the editor's squares.
fn round_widgets(ui: &mut Ui) {
    let v = &mut ui.style_mut().visuals;
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(8);
    }
}

/// The garrysmod folder at or above `text`, if there is one.
fn game_folder(text: &str) -> Option<PathBuf> {
    if text.trim().is_empty() {
        return None;
    }
    ad2read::models::find_garrysmod(Some(Path::new(text.trim())))
}

impl App {
    /// Pushes the config into egui: palette, widget style, zoom.
    fn apply_config(&mut self, ctx: &egui::Context) {
        let (palette, problems) = self.config.palette();
        self.palette = palette;
        set_colours(&palette);
        install_theme(ctx);
        ctx.set_zoom_factor(self.config.ui_scale);
        for p in problems {
            self.bad(p);
        }
    }

    fn save_config(&mut self) {
        self.config.set_palette(&self.palette);
        match self.config.save() {
            Ok(path) => self.say(format!("settings saved to {}", path.display())),
            Err(e) => self.bad(format!("settings not saved: {e}")),
        }
    }

    /// The wizard, floating over the editor: a blurred shot of it, a tint in
    /// the theme's darkest surface, and a card whose steps slide.
    fn setup_overlay(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(mut setup) = self.setup.take() else {
            return;
        };
        if setup.backdrop.is_none() {
            if !setup.screenshot_requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                setup.screenshot_requested = true;
            }
            setup.frames_without_screenshot += 1;
            if setup.frames_without_screenshot < 6 {
                ctx.request_repaint();
                self.setup = Some(setup);
                return;
            }
        }
        let mut started = false;
        let c = colours();
        let tint = Color32::from_rgba_unmultiplied(c.chrome.r(), c.chrome.g(), c.chrome.b(), 150);
        let shadow = egui::epaint::Shadow {
            offset: [0, 12],
            blur: 36,
            spread: 0,
            color: Color32::from_black_alpha(120),
        };
        let card = egui::Frame::new()
            .fill(mix(c.panel, c.chrome, 0.15))
            .stroke(Stroke::new(1.0, c.view_edge))
            .corner_radius(CornerRadius::same(18))
            .shadow(shadow)
            .inner_margin(Margin::same(30));
        egui::Modal::new(egui::Id::new("setup-wizard"))
            .backdrop_color(Color32::TRANSPARENT)
            .frame(egui::Frame::NONE)
            .show(&ctx, |ui| {
                let screen = ui.ctx().content_rect();
                if let Some(tex) = &setup.backdrop {
                    ui.painter().image(
                        tex.id(),
                        screen,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                ui.painter().rect_filled(screen, 0.0, tint);
                card.show(ui, |ui| {
                    ui.set_width(520.0);
                    round_widgets(ui);
                    self.setup_steps(ui, &ctx, &mut setup, &mut started);
                });
            });
        if started {
            self.finish_setup(setup);
        } else {
            self.setup = Some(setup);
        }
    }

    fn setup_steps(&mut self, ui: &mut Ui, ctx: &egui::Context, setup: &mut Setup, started: &mut bool) {
        let width = ui.available_width();
        ui.horizontal(|ui| {
            step_dots(ui, SETUP_STEPS, setup.step);
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!("{} of {}", setup.step + 1, SETUP_STEPS))
                    .color(colours().text_dim)
                    .small(),
            );
        });
        ui.add_space(10.0);

        // The step's content is drawn offset by how far the animation still
        // has to travel, so a step arrives from the side it lives on.
        let t = ctx.animate_value_with_time(egui::Id::new("setup-slide"), setup.step as f32, 0.28);
        let offset = (setup.step as f32 - t) * width;
        let fade = 1.0 - (offset.abs() / width).clamp(0.0, 1.0);
        let (body_rect, _) = ui.allocate_exact_size(vec2(width, 250.0), Sense::hover());
        let mut body = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(body_rect.translate(vec2(offset, 0.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        body.set_clip_rect(body_rect.expand2(vec2(0.0, 8.0)));
        body.set_opacity(fade);
        body.set_width(width);
        match setup.step {
            0 => self.setup_step_mode(&mut body, width),
            1 => self.setup_step_game(&mut body, setup),
            _ => self.setup_step_done(&mut body, setup),
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if setup.step > 0 && soft_button(ui, "Back", false, true).clicked() {
                setup.step -= 1;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if setup.step + 1 < SETUP_STEPS {
                    if soft_button(ui, "Continue", true, true).clicked() {
                        setup.step += 1;
                    }
                } else if soft_button(ui, "Start", true, true).clicked() {
                    *started = true;
                }
            });
        });
    }

    fn setup_step_mode(&mut self, ui: &mut Ui, width: f32) {
        ui.label(RichText::new("How do you build?").color(colours().heading).size(22.0));
        ui.label(
            RichText::new("Pick one. It only decides what the editor shows first.").color(colours().text_dim),
        );
        ui.add_space(16.0);
        let card_w = (width - 14.0) / 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let choices = [
                (
                    Mode::Newcomer,
                    "Newcomer",
                    "Start from a preset tank with empty slots. Parts from the Add menu, a checklist that says what is next, every aid on.",
                ),
                (
                    Mode::Advanced,
                    "Advanced",
                    "The full editor: tree, inspector, viewport, raw tables. Nothing hidden, nothing guided.",
                ),
            ];
            for (mode, title, blurb) in choices {
                let id = egui::Id::new(("setup-mode", title));
                if choice_card(ui, id, title, blurb, self.config.mode == mode, card_w).clicked() {
                    self.config.mode = mode;
                }
            }
        });
    }

    fn setup_step_game(&mut self, ui: &mut Ui, setup: &mut Setup) {
        ui.label(RichText::new("Where is Garry's Mod?").color(colours().heading).size(22.0));
        ui.label(
            RichText::new("The garrysmod folder inside the game. It is how props get their real sizes.")
                .color(colours().text_dim),
        );
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            let field_w = ui.available_width() - 112.0;
            ui.add(
                egui::TextEdit::singleline(&mut setup.garrysmod)
                    .desired_width(field_w)
                    .hint_text("…/steamapps/common/GarrysMod/garrysmod"),
            );
            if soft_button(ui, "Browse", false, true).clicked() {
                if let Some(dir) = rfd::FileDialog::new()
                    .set_title("Select your garrysmod folder")
                    .pick_folder()
                {
                    setup.garrysmod = dir.display().to_string();
                    setup.found_automatically = false;
                }
            }
        });
        ui.add_space(8.0);
        let (status, ok) = if setup.garrysmod.trim().is_empty() {
            (
                "Not found. Browse to it, or leave it: the editor also looks next to any dupe you open.",
                false,
            )
        } else if game_folder(&setup.garrysmod).is_some() {
            (if setup.found_automatically { "Found in your Steam library." } else { "Looks right." }, true)
        } else {
            ("That folder has no addons, models or data inside, so it is not garrysmod.", false)
        };
        ui.horizontal(|ui| {
            let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter()
                .circle_filled(dot.center(), 4.0, if ok { colours().good } else { colours().field_edge });
            ui.label(RichText::new(status).color(if ok { colours().text } else { colours().text_dim }));
        });
    }

    fn setup_step_done(&mut self, ui: &mut Ui, setup: &Setup) {
        ui.label(RichText::new("You're set.").color(colours().heading).size(22.0));
        ui.add_space(12.0);
        let mode = match self.config.mode {
            Mode::Newcomer => "Newcomer mode. Start from a tank, follow the checklist, send it to the game.",
            Mode::Advanced => "Advanced mode. The full editor, straight away.",
        };
        ui.label(RichText::new(mode).color(colours().text));
        ui.add_space(6.0);
        let game = match game_folder(&setup.garrysmod) {
            Some(dir) => format!("Game folder: {}", dir.display()),
            None => "Game folder: not set. The editor looks next to any dupe you open.".to_owned(),
        };
        ui.label(RichText::new(game).color(colours().text));
        ui.add_space(14.0);
        ui.label(
            RichText::new("All of this, and the colours, live under Settings at the right of the top bar.")
                .color(colours().text_dim),
        );
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let mut reindex: Option<PathBuf> = None;
        let mut rerun_setup = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .default_width(460.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
                    sfm_header(ui, "MODE");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        for (mode, label) in [(Mode::Newcomer, "Newcomer"), (Mode::Advanced, "Advanced")] {
                            if button(ui, label, true).clicked() {
                                self.config.mode = mode;
                            }
                            if self.config.mode == mode {
                                ui.label(RichText::new("\u{2022}").color(colours().accent));
                            }
                        }
                    });
                    ui.label(
                        RichText::new("Newcomer starts from a tank with a checklist and a properties panel; Advanced is the full editor.")
                            .color(colours().text_dim)
                            .small(),
                    );

                    ui.add_space(10.0);
                    sfm_header(ui, "LOOK");
                    ui.add_space(4.0);
                    let (mut theme, _) = self.config.theme.theme();
                    let mut theme_changed = false;
                    let slots = [
                        ("main", &mut theme.main, "every surface"),
                        ("second", &mut theme.second, "text, lines and highlights"),
                        ("background", &mut theme.background, "behind the 3D view, nothing else"),
                    ];
                    for (label, slot, blurb) in slots {
                        ui.horizontal(|ui| {
                            let mut rgb = [slot[0], slot[1], slot[2]];
                            if ui.color_edit_button_srgb(&mut rgb).changed() {
                                *slot = [rgb[0], rgb[1], rgb[2], 255];
                                theme_changed = true;
                            }
                            ui.label(RichText::new(label).color(colours().text));
                            ui.label(RichText::new(blurb).color(colours().text_dim).small());
                        });
                    }
                    if theme_changed {
                        self.config.theme = config::ThemeColours::from_theme(&theme);
                        self.apply_config(ctx);
                    }
                    if self.config.theme != config::ThemeColours::default()
                        && button(ui, "Reset colours", true).clicked()
                    {
                        self.config.theme = config::ThemeColours::default();
                        self.apply_config(ctx);
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("size").color(colours().text_dim));
                        for (label, scale) in [("Small", 0.9_f32), ("Normal", 1.0), ("Large", 1.25)] {
                            let on = (self.config.ui_scale - scale).abs() < 0.01;
                            if button(ui, label, true).clicked() && !on {
                                self.config.ui_scale = scale;
                                ctx.set_zoom_factor(scale);
                            }
                            if on {
                                ui.label(RichText::new("\u{2022}").color(colours().accent));
                            }
                        }
                    });

                    ui.add_space(10.0);
                    sfm_header(ui, "GAME");
                    ui.add_space(4.0);
                    ui.label(RichText::new("garrysmod folder").color(colours().text_dim).small());
                    ui.horizontal(|ui| {
                        let w = (ui.available_width() - 170.0).max(120.0);
                        ui.add(egui::TextEdit::singleline(&mut self.config.garrysmod).desired_width(w));
                        if button(ui, "Browse...", true).clicked() {
                            if let Some(dir) = rfd::FileDialog::new()
                                .set_title("Select your garrysmod folder")
                                .pick_folder()
                            {
                                self.config.garrysmod = dir.display().to_string();
                            }
                        }
                        let can = game_folder(&self.config.garrysmod).is_some();
                        if button(ui, "Index now", can).clicked() && can {
                            reindex = game_folder(&self.config.garrysmod);
                        }
                    });
                    let status = if self.config.garrysmod.trim().is_empty() {
                        "not set; found from the dupe you open, or browse here"
                    } else if game_folder(&self.config.garrysmod).is_some() {
                        if self.models.is_some() {
                            "indexed; real sizes loaded"
                        } else {
                            "looks right; indexed when a dupe opens, or Index now"
                        }
                    } else {
                        "that folder has no addons, models or data inside, so it is not garrysmod"
                    };
                    ui.label(RichText::new(status).color(colours().text_dim).small());
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("AdvDupe2 folder, where Send to game will write")
                            .color(colours().text_dim)
                            .small(),
                    );
                    ui.horizontal(|ui| {
                        let w = (ui.available_width() - 90.0).max(120.0);
                        ui.add(egui::TextEdit::singleline(&mut self.config.advdupe2).desired_width(w));
                        if button(ui, "Browse...", true).clicked() {
                            if let Some(dir) = rfd::FileDialog::new()
                                .set_title("Select the advdupe2 folder")
                                .pick_folder()
                            {
                                self.config.advdupe2 = dir.display().to_string();
                            }
                        }
                    });

                    ui.add_space(10.0);
                    sfm_header(ui, "VIEWPORT");
                    ui.add_space(4.0);
                    ui.checkbox(&mut self.config.viewport.real_geometry, "draw real geometry once models are loaded");
                    ui.checkbox(&mut self.config.viewport.fullbright, "fullbright");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("detail").color(colours().text_dim));
                        ui.add(egui::Slider::new(&mut self.config.viewport.detail, 0.25..=1.0).fixed_decimals(2));
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("brightness").color(colours().text_dim));
                        ui.add(egui::Slider::new(&mut self.config.viewport.brightness, 0.5..=4.0).fixed_decimals(1));
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("fly speed").color(colours().text_dim));
                        ui.add(
                            egui::DragValue::new(&mut self.config.viewport.fly_speed)
                                .speed(10.0)
                                .range(20.0..=5000.0),
                        );
                    });
                    ui.add_space(10.0);
                    sfm_header(ui, "KEYS");
                    ui.add_space(4.0);
                    let capturing = self.capturing_key;
                    let heard = ui.input(|i| {
                        i.events.iter().find_map(|event| match event {
                            egui::Event::Key { key, pressed: true, .. } => Some(*key),
                            _ => None,
                        })
                    });
                    let mut begin: Option<usize> = None;
                    egui::Grid::new("settings-keys").num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
                        for (n, (what, with_ctrl, name)) in self.config.keys.slots().into_iter().enumerate() {
                            ui.label(RichText::new(what).color(colours().text_dim));
                            let shown = if capturing == Some(n) { "press a key".to_owned() } else if with_ctrl { format!("Ctrl+{name}") } else { name.clone() };
                            if ui.button(shown).clicked() {
                                begin = Some(n);
                            }
                            // Escape leaves the capture without binding anything.
                            if capturing == Some(n) {
                                if let Some(key) = heard.filter(|key| *key != egui::Key::Escape) {
                                    *name = key.name().to_owned();
                                }
                            }
                            if n % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                    if capturing.is_some() && heard.is_some() {
                        self.capturing_key = None;
                    }
                    if begin.is_some() {
                        self.capturing_key = begin;
                    }
                    for (a, b) in self.config.keys.clashes() {
                        ui.label(RichText::new(format!("{a} and {b} are on the same key")).color(colours().bad).small());
                    }
                    ui.horizontal(|ui| {
                        if self.config.keys != config::Keys::default() && button(ui, "Reset keys", true).clicked() {
                            self.config.keys = config::Keys::default();
                        }
                        ui.label(RichText::new("Flying stays on W A S D, Q E, Shift and Ctrl.").color(colours().text_dim).small());
                    });

                    ui.add_space(10.0);
                    sfm_header(ui, "PLACEMENT");
                    ui.add_space(4.0);
                    ui.checkbox(&mut self.config.placement.snapping, "handles snap to steps (hold Alt for the opposite)");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("move step").color(colours().text_dim));
                        ui.add(egui::DragValue::new(&mut self.config.placement.move_step).speed(0.05).range(0.01..=48.0).suffix(" units"));
                        ui.label(RichText::new("turn step").color(colours().text_dim));
                        ui.add(egui::DragValue::new(&mut self.config.placement.turn_step).speed(0.5).range(0.5..=90.0).suffix("°"));
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("look sensitivity").color(colours().text_dim));
                        ui.add(egui::Slider::new(&mut self.config.viewport.look_sensitivity, 0.1..=5.0).fixed_decimals(1));
                        ui.checkbox(&mut self.config.viewport.invert_look, "invert up and down");
                    });
                    ui.checkbox(&mut self.config.viewport.centre_of_mass, "mark the centre of mass");
                    ui.label(
                        RichText::new("The background colour is under Look, in the Viewport group.")
                            .color(colours().text_dim)
                            .small(),
                    );

                    ui.add_space(10.0);
                    sfm_header(ui, "AUTOSAVE");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("every").color(colours().text_dim));
                        ui.add(egui::DragValue::new(&mut self.config.autosave.seconds).range(0..=3600).suffix(" s"));
                        ui.label(
                            RichText::new("0 is off; writes name.autosave.txt beside the file")
                                .color(colours().text_dim)
                                .small(),
                        );
                    });

                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if button(ui, "Run setup again", true).clicked() {
                            rerun_setup = true;
                        }
                        if let Some(path) = config::path() {
                            ui.label(RichText::new(path.display().to_string()).color(colours().text_dim).small());
                        }
                    });
                });
            });
        if let Some(dir) = reindex {
            self.index_models(dir);
        }
        if rerun_setup {
            self.show_settings = false;
            self.save_config();
            self.setup = Some(Setup::new(&self.config));
        } else if !open {
            self.show_settings = false;
            self.save_config();
        }
    }

    /// Newcomer mode: a flat bar, the selected part's properties, the checklist, the log,
    /// and either the viewport or the preset picker in the middle.
    fn newcomer_frame(&mut self, ui: &mut Ui) {
        let c = colours();
        egui::Panel::top("newcomer-bar")
            .frame(egui::Frame::default().fill(c.chrome).inner_margin(Margin::symmetric(12, 8)))
            .show(ui, |ui| self.newcomer_bar(ui));
        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(90.0)
            .frame(egui::Frame::default().fill(c.console).inner_margin(Margin::same(6)))
            .show(ui, |ui| self.log_panel(ui));
        egui::Panel::left("newcomer-properties")
            .resizable(true)
            .default_size(270.0)
            .frame(egui::Frame::default().fill(c.chrome).inner_margin(Margin::same(12)))
            .show(ui, |ui| self.properties_panel(ui));
        egui::Panel::right("newcomer-checklist")
            .resizable(true)
            .default_size(330.0)
            .frame(egui::Frame::default().fill(c.chrome).inner_margin(Margin::same(12)))
            .show(ui, |ui| self.checklist_panel(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(c.background).inner_margin(Margin::same(0)))
            .show(ui, |ui| self.centre(ui));
    }

    fn newcomer_bar(&mut self, ui: &mut Ui) {
        self.menus(ui);
        round_widgets(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new("ad2edit").color(colours().heading).size(16.0));
            ui.add_space(12.0);
            if soft_button(ui, "Open", false, true).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("AdvDupe2", &["txt"]).pick_file() {
                    self.load(p);
                }
            }
            let has = self.open.is_some();
            if soft_button(ui, "Save", false, has).clicked() {
                if let Some(p) = self.open.as_ref().map(|o| o.path.clone()) {
                    self.save_as(p);
                }
            }
            if soft_button(ui, "Send to game", true, has).clicked() {
                self.send_to_game();
            }
            ui.add_space(12.0);
            match self.open.as_ref() {
                None => {
                    ui.label(RichText::new("nothing open").color(colours().text_dim));
                }
                Some(open) => {
                    let name = open.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.label(RichText::new(name).color(colours().text));
                    if open.dirty {
                        ui.label(RichText::new("unsaved").color(colours().accent).small());
                    }
                }
            }
            ui.add_space(12.0);
            self.weight_label(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if soft_button(ui, "Advanced editor", false, true).clicked() {
                    self.config.mode = Mode::Advanced;
                    self.save_config();
                }
                if soft_button(ui, "Settings", false, true).clicked() {
                    self.show_settings = true;
                }
            });
        });
    }

    /// Where built tanks go: the AdvDupe2 folder when it is set, else a
    /// builds folder beside the config, else the temp folder.
    fn build_dir(&self) -> PathBuf {
        let ad2 = PathBuf::from(&self.config.advdupe2);
        if !self.config.advdupe2.trim().is_empty() && ad2.is_dir() {
            return ad2;
        }
        if let Some(dir) = config::dir() {
            let builds = dir.join("builds");
            if std::fs::create_dir_all(&builds).is_ok() {
                return builds;
            }
        }
        std::env::temp_dir()
    }

    /// Writes the open dupe into the AdvDupe2 folder, after the same check
    /// the CLI makes before any write.
    fn send_to_game(&mut self) {
        let (problems, name) = {
            let Some(open) = self.open.as_ref() else { return };
            let name = open.path.file_name().map(|s| s.to_os_string()).unwrap_or_else(|| "tank.txt".into());
            (ad2read::extras::validate(&open.dupe), name)
        };
        if !problems.is_empty() {
            self.bad("not sent: AdvDupe2 would refuse this file");
            for p in problems {
                self.bad(format!("  {p}"));
            }
            return;
        }
        let dir = PathBuf::from(&self.config.advdupe2);
        if self.config.advdupe2.trim().is_empty() || !dir.is_dir() {
            self.bad("set the AdvDupe2 folder in Settings first; it is garrysmod/data/advdupe2");
            self.show_settings = true;
            return;
        }
        let dest = dir.join(name);
        let stem = dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.save_as(dest);
        self.good(format!("in game: AdvDupe2 tool, Open, {stem}. Paste it."));
    }

    /// Adds a catalogue item in front of the camera and attaches it the way
    /// the references do: wheels hinge to the hull, everything else parents.
    fn add_part(&mut self, item: &'static ad2read::catalog::Item) {
        if self.open.is_none() {
            return self.bad("start from a preset or open a dupe first");
        }
        let at = self.spawn_point();
        let base = self.open.as_ref().and_then(|o| {
            o.dupe
                .list_entities()
                .into_iter()
                .find(|(_, c, _)| c.to_lowercase().contains("acf_baseplate"))
                .map(|(i, _, _)| i)
        });
        self.snapshot();
        let is_wheel = item.category == "Wheels";
        let is_base = item.entity == "acf_baseplate";
        let result = match self.open.as_mut() {
            Some(open) => ad2read::build::spawn_catalog(&mut open.dupe, item, at, (0.0, 90.0, 0.0), &[]).and_then(|new| {
                if let (Some(b), false) = (base, is_base) {
                    if is_wheel {
                        ad2read::build::add_wheel_axis(&mut open.dupe, new, b, (0.0, 1.0, 0.0), false)?;
                    } else {
                        ad2read::build::set_parent(&mut open.dupe, new, b)?;
                    }
                }
                Ok(new)
            }),
            None => Err("nothing open".into()),
        };
        match result {
            Ok(new) => {
                // The first part of a new dupe is what AD2 pastes it by.
                if let Some(open) = self.open.as_mut() {
                    ad2read::extras::repair_head(&mut open.dupe);
                }
                self.pick = Some(Pick::Entity(new));
                self.crumbs.clear();
                let how = if base.is_none() || is_base {
                    "on its own"
                } else if is_wheel {
                    "hinged to the hull about its own axle"
                } else {
                    "parented to the hull"
                };
                self.good(format!(
                    "added {} as entity {new}, {how}. Move it with the gizmo; link and wire it in the Nodes tab.",
                    item.name
                ));
            }
            Err(e) => self.bad(format!("add {}: {e}", item.name)),
        }
    }

    /// The numbers on the selected part, each a field that writes straight
    /// back. One undo step per drag or per typed value, and the checklist
    /// re-reads the doctor after it, so a bad number shows up at once.
    fn tuning_section(&mut self, ui: &mut Ui, index: f64) {
        // A baseplate has a size and nothing else to tune, so the size row
        // comes before the check for tunables. A crate's and a tank's own
        // controls already cover everything tunable about them.
        if self.size_row(ui, index) {
            return;
        }
        let found = match self.open.as_ref() {
            Some(open) => ad2read::pipeline::tunables(&open.dupe, index),
            None => return,
        };
        if found.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(RichText::new("Tune this part").color(colours().text))
            .id_salt("newcomer-tuning")
            .default_open(true)
            .show(ui, |ui| {
                for tunable in found {
                    let mut value = tunable.value;
                    let resp = ui
                        .horizontal(|ui| {
                            let resp = ui.add(egui::DragValue::new(&mut value).speed(0.1).max_decimals(3));
                            ui.label(RichText::new(&tunable.label).color(colours().text_dim));
                            resp
                        })
                        .inner;
                    if resp.drag_started() || resp.gained_focus() {
                        self.snapshot();
                    }
                    if value != tunable.value {
                        if let Some(open) = self.open.as_mut() {
                            match ad2read::build::put_path(&mut open.dupe, index, &tunable.path, Value::Number(value)) {
                                Ok(()) => open.dirty = true,
                                Err(e) => self.bad(format!("{}: {e}", tunable.label)),
                            }
                        }
                        self.edit_count += 1;
                    }
                }
            });
    }

    /// A sloped SProps hull on the baseplate, armoured and parented, for the
    /// armour plan to work on. The log says what the glacis is worth.
    pub(crate) fn build_hull(&mut self) {
        let Some(base) = self
            .open
            .as_ref()
            .and_then(|open| ad2read::roles::of_dupe(&open.dupe).into_iter().find(|(_, role)| *role == ad2read::roles::Role::Hull))
            .map(|(index, _)| index)
        else {
            return self.bad("a hull needs a baseplate to stand on");
        };
        self.snapshot();
        let degrees = self.newcomer_panels.glacis_degrees;
        let made = self.open.as_mut().map(|open| ad2read::hull::add_sloped_hull(&mut open.dupe, base, 36.0, degrees, 20.0));
        match made {
            Some(Ok((pieces, glacis))) => self.good(format!(
                "built a hull of {} pieces at 20 mm. Its glacis is {:.0} degrees from the vertical: {:.1} times its thickness to a level shot, and {:.0} % of AP that fails to get through glances off. Plan the armour to set real thicknesses.",
                pieces.len(),
                glacis.degrees,
                glacis.thicker_by,
                glacis.ap_glances * 100.0
            )),
            Some(Err(e)) => self.bad(format!("hull: {e}")),
            None => {}
        }
    }

    fn checklist_panel(&mut self, ui: &mut Ui) {
        round_widgets(ui);
        ui.label(RichText::new("Checklist").color(colours().heading).size(15.0));
        if self.open.is_none() {
            ui.add_space(6.0);
            ui.label(
                RichText::new("Pick a preset and this fills in: what the doctor sees, what comes next, how heavy it is.")
                    .color(colours().text_dim),
            );
            return;
        }
        if self.newcomer_panels.findings_at_edit != Some(self.edit_count) {
            self.newcomer_panels.findings = self.open.as_ref().map(|o| ad2read::doctor::doctor(&o.dupe)).unwrap_or_default();
            self.newcomer_panels.findings_at_edit = Some(self.edit_count);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("armour to").color(colours().text_dim));
            ui.add(egui::DragValue::new(&mut self.newcomer_panels.armour_tonnes).range(5.0..=60.0).suffix(" t").speed(0.5));
            if soft_chip(ui, "Plan it").clicked() {
                self.plan_armour();
            }
        });
        let roles = self.open.as_ref().map(|open| ad2read::roles::of_dupe(&open.dupe)).unwrap_or_default();
        let has = |wanted: ad2read::roles::Role| roles.iter().any(|(_, role)| *role == wanted);
        if has(ad2read::roles::Role::Hull) && !has(ad2read::roles::Role::ArmourPlate) {
            ui.horizontal_wrapped(|ui| {
                if soft_chip(ui, "Build a hull").clicked() {
                    self.build_hull();
                }
                ui.add(egui::DragValue::new(&mut self.newcomer_panels.glacis_degrees).range(45.0..=75.0).suffix("° glacis").speed(0.5));
                let worth = ad2read::hull::Glacis::at(self.newcomer_panels.glacis_degrees);
                ui.label(
                    RichText::new(format!(
                        "No armour plates yet. A sloped nose is what makes armour work: at this angle a plate is {:.1} times as thick to a level shot and {:.0} % of AP that fails to get through glances off. A flat front gets neither.",
                        worth.thicker_by,
                        worth.ap_glances * 100.0
                    ))
                    .color(colours().text_dim)
                    .small(),
                );
            });
        }
        if let Some(Pick::Entity(i)) = self.pick {
            ui.horizontal(|ui| {
                let role = self
                    .open
                    .as_ref()
                    .and_then(|open| ad2read::roles::of_dupe(&open.dupe).into_iter().find(|(index, _)| *index == i))
                    .map(|(_, role)| role.name())
                    .unwrap_or("entity");
                ui.label(RichText::new(format!("selected: {role} ({i})")).color(colours().text_dim).small());
                if self.link_mode.is_some() {
                    ui.label(RichText::new("now click what to link it to; Esc cancels").color(colours().accent).small());
                } else if soft_chip(ui, "Link to...").clicked() {
                    self.link_mode = Some(i);
                }
            });
        }
        ui.add_space(10.0);
        let errors = self.newcomer_panels.findings.iter().filter(|f| f.severity == Severity::Error).count();
        let fixable = self.newcomer_panels.findings.iter().filter(|f| f.fix.is_some()).count();
        if self.newcomer_panels.findings.is_empty() {
            ui.label(RichText::new("The doctor has nothing to say. It pastes.").color(colours().good));
        } else {
            ui.horizontal(|ui| {
                let rest = self.newcomer_panels.findings.len() - errors;
                ui.label(RichText::new(format!("{errors} to fix, {rest} worth knowing")).color(colours().text));
                if fixable > 0 && soft_chip(ui, &format!("Fix all {fixable}")).clicked() {
                    self.fix_findings(None);
                }
            });
        }
        ui.add_space(6.0);
        let findings = self.newcomer_panels.findings.clone();
        let mut fix_one: Option<usize> = None;
        let mut show: Option<f64> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (i, f) in findings.iter().enumerate() {
                ui.horizontal(|ui| {
                    let colour = match f.severity {
                        Severity::Error => colours().bad,
                        Severity::Warn => colours().link_acf,
                        Severity::Note => colours().field_edge,
                    };
                    let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, colour);
                    // The dot's colour is not the only thing that says how bad it is.
                    let severity = match f.severity {
                        Severity::Error => "Error",
                        Severity::Warn => "Warning",
                        Severity::Note => "Note",
                    };
                    ui.vertical(|ui| {
                        ui.set_max_width(ui.available_width() - 100.0);
                        ui.label(RichText::new(format!("{severity}: {}", f.what)).color(colours().text).small());
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if f.fix.is_some() && soft_chip(ui, "Fix").clicked() {
                            fix_one = Some(i);
                        }
                        if let Some(e) = f.entity {
                            if soft_chip(ui, "Show").clicked() {
                                show = Some(e);
                            }
                        }
                    });
                });
                ui.add_space(3.0);
            }
            if errors == 0 {
                self.missing_part_hints(ui);
            }
        });
        if let Some(e) = show {
            self.pick = Some(Pick::Entity(e));
            self.crumbs.clear();
            self.marked.clear();
            self.frame_selection();
        }
        if let Some(i) = fix_one {
            self.fix_findings(Some(i));
        }
    }

    /// Applies one finding's fix, or every fix there is.
    fn fix_findings(&mut self, which: Option<usize>) {
        let selected: Vec<Finding> = match which {
            Some(i) => self.newcomer_panels.findings.get(i).cloned().into_iter().collect(),
            None => self.newcomer_panels.findings.iter().filter(|f| f.fix.is_some()).cloned().collect(),
        };
        if selected.is_empty() {
            return;
        }
        self.snapshot();
        let done = match self.open.as_mut() {
            Some(open) => ad2read::doctor::apply_fixes(&mut open.dupe, &selected),
            None => Vec::new(),
        };
        for d in done {
            self.good(format!("fixed: {d}"));
        }
        self.crumbs.clear();
    }

    /// What a tank still lacks, by class, once the doctor is quiet.
    /// The roles the tank still lacks, each with a button that adds the part
    /// the reference tanks fill it with. This is the slot list: the build
    /// always says what comes next, and one click does it.
    fn missing_part_hints(&mut self, ui: &mut Ui) {
        let Some(open) = self.open.as_ref() else { return };
        let slots = ad2read::roles::open_slots(&open.dupe);
        ui.add_space(8.0);
        ui.label(RichText::new("What comes next").color(colours().heading));
        if slots.is_empty() {
            ui.label(
                RichText::new("Every role a tank needs is filled. Link and wire it in the Nodes tab, plan the armour, Save, Send to game.")
                    .color(colours().text_dim)
                    .small(),
            );
        }
        let mut wanted = None;
        for slot in slots {
            let usual = ad2read::verbs::default_item(slot.role).and_then(ad2read::catalog::find);
            ui.horizontal_wrapped(|ui| {
                if let Some(item) = usual {
                    if soft_chip(ui, &format!("Add a {}", slot.role.name())).on_hover_text(format!("adds the {}", item.name)).clicked() {
                        wanted = Some(item);
                    }
                }
                ui.label(RichText::new(slot.advice).color(colours().text_dim).small());
            });
        }
        if let Some(item) = wanted {
            self.add_part(item);
        }
    }

    /// Thicknesses that land the plates on the target, front-heavy, the way
    /// `--armour-apply` does it.
    fn plan_armour(&mut self) {
        let target_kg = self.newcomer_panels.armour_tonnes * 1000.0;
        let plates = {
            let Some(open) = self.open.as_ref() else { return };
            let mut half_of = |model: &str| -> Option<(f64, f64, f64)> {
                self.models
                    .as_mut()
                    .and_then(|lib| lib.bounds(model))
                    .map(|(b, _)| b.half_extents())
                    .or_else(|| ad2read::build::sprops_half_extents(model))
            };
            ad2read::pipeline::plates_of(&open.dupe, &mut half_of)
        };
        if plates.is_empty() {
            return self.bad("no armour plates to plan: plates are props with ACF armour. A preset has none yet; add them from a dupe, or in game.");
        }
        let plan = ad2read::pipeline::plan_thicknesses(&plates, target_kg, 1.6);
        self.snapshot();
        let n = self.open.as_mut().map(|o| ad2read::pipeline::apply_thicknesses(&mut o.dupe, &plan)).unwrap_or(0);
        self.good(format!("armour planned for {:.0} t across {n} plates, front-heavy", target_kg / 1000.0));
    }

    fn finish_setup(&mut self, setup: Setup) {
        if let Some(dir) = game_folder(&setup.garrysmod) {
            self.config.garrysmod = dir.display().to_string();
            if self.config.advdupe2.is_empty() {
                self.config.advdupe2 = dir.join("data").join("advdupe2").display().to_string();
            }
        }
        self.save_config();
    }
}

fn constraint_list(dupe: &Dupe) -> Vec<usize> {
    let Ok(root) = dupe.root_table() else {
        return Vec::new();
    };
    let Some(cons) = dupe.get_table(root, "Constraints") else {
        return Vec::new();
    };
    match &dupe.arena[cons] {
        Node::Array(items) => items.iter().filter_map(table_index).collect(),
        _ => Vec::new(),
    }
}

fn as_text(v: &Value) -> String {
    match v {
        Value::Str(b) => String::from_utf8_lossy(b).into_owned(),
        other => format!("{other:?}"),
    }
}

fn key_label(k: &Value) -> String {
    match k {
        Value::Str(b) => String::from_utf8_lossy(b).into_owned(),
        Value::Number(n) => format!("[{n}]"),
        other => format!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if verbose() {
            // Input events as they happen, so a misbehaving panel or a dead
            // shortcut can be traced to what was actually pressed.
            let events = ui.input(|i| i.events.clone());
            for e in events {
                match e {
                    egui::Event::Key { key, pressed, modifiers, .. } => {
                        println!("[key] {key:?} {} {modifiers:?}", if pressed { "down" } else { "up" });
                    }
                    egui::Event::PointerButton { pos, button, pressed, modifiers } => {
                        println!("[ptr] {button:?} {} at ({:.0}, {:.0}) {modifiers:?}", if pressed { "down" } else { "up" }, pos.x, pos.y);
                    }
                    _ => {}
                }
            }
        }
        if let Some(setup) = self.setup.as_mut() {
            if setup.backdrop.is_none() {
                let shot = ui.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(image) = shot {
                    setup.backdrop = Some(blurred_backdrop(ui.ctx(), &image));
                }
            }
        } else {
            self.shortcuts(ui);
            self.autosave_tick();
            if !ui.input(|i| i.pointer.any_down()) && !ui.ctx().egui_wants_keyboard_input() {
                self.container_drag = None;
            }
        }
        self.screenshot_tick(ui);
        let ctx_for_window = ui.ctx().clone();
        self.chip_window(&ctx_for_window);
        self.autosave_offer_window(&ctx_for_window);
        if self.show_checklist && self.config.mode == Mode::Advanced && self.open.is_some() {
            let mut still_open = true;
            egui::Window::new("Checklist").id(egui::Id::new("advanced-checklist")).open(&mut still_open).default_size([360.0, 480.0]).show(&ctx_for_window, |ui| self.checklist_panel(ui));
            self.show_checklist = still_open;
        }
        self.add_window(&ctx_for_window);
        self.settings_window(&ctx_for_window);
        self.tutorial_window(&ctx_for_window);
        // Stash the render state for this frame. The viewport is several call
        // levels down and eframe only hands the Frame to this method.
        self.render_state = frame.wgpu_render_state().cloned();

        if self.open.is_none() {
            self.gallery_frame(ui);
            self.setup_overlay(ui);
            return;
        }
        if self.config.mode == Mode::Newcomer {
            self.newcomer_frame(ui);
            self.floating_menu(ui);
            self.setup_overlay(ui);
            return;
        }

        egui::Panel::top("bar")
            .frame(
                egui::Frame::default()
                    .fill(colours().menu_bottom)
                    .inner_margin(Margin::symmetric(6, 3)),
            )
            .show(ui, |ui| {
                // QMenuBar is a vertical gradient with a 50%-black bottom
                // border; egui can only fill flat, so paint it underneath.
                menu_bar(ui.painter(), ui.max_rect().expand2(egui::vec2(6.0, 3.0)));
                self.top_bar(ui);
            });

        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(120.0)
            .frame(
                egui::Frame::default()
                    .fill(colours().console)
                    .inner_margin(Margin::same(6)),
            )
            .show(ui, |ui| self.log_panel(ui));

        egui::Panel::left("tree")
            .resizable(true)
            .default_size(280.0)
            .frame(
                egui::Frame::default()
                    .fill(colours().chrome)
                    .inner_margin(Margin::same(0)),
            )
            .show(ui, |ui| self.tree_panel(ui));

        egui::Panel::right("ops")
            .resizable(true)
            .default_size(230.0)
            .frame(
                egui::Frame::default()
                    .fill(colours().panel)
                    .inner_margin(Margin::same(0)),
            )
            .show(ui, |ui| self.ops_panel(ui));

        egui::Panel::bottom("inspector")
            .resizable(true)
            .default_size(300.0)
            .frame(
                egui::Frame::default()
                    .fill(colours().panel)
                    .inner_margin(Margin::same(8)),
            )
            .show(ui, |ui| self.inspector(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(colours().background).inner_margin(Margin::same(0)))
            .show(ui, |ui| self.centre(ui));

        self.floating_menu(ui);
        self.setup_overlay(ui);
    }

    fn on_exit(&mut self) {
        self.config.set_palette(&self.palette);
        let _ = self.config.save();
    }
}

impl App {
    /// Keyboard shortcuts. `command` rather than `ctrl` so this behaves on a
    /// Mac too, and nothing fires while a text field has focus — otherwise
    /// typing "v" into the material box would paste a dupe.
    fn shortcuts(&mut self, ui: &mut Ui) {
        if self.link_mode.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.link_mode = None;
            self.say("link cancelled");
        }
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        // The keys are the ones in Settings > Keys, by egui's names for them.
        let bound = self.config.keys.clone();
        let pressed = |name: &str| egui::Key::from_name(name).is_some_and(|key| ui.input(|i| i.key_pressed(key)));
        let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        let (z, y, c, v, d) = (pressed(&bound.undo), pressed(&bound.redo), pressed(&bound.copy), pressed(&bound.paste), pressed(&bound.duplicate));
        let (save, o, a, n) = (pressed(&bound.save), pressed(&bound.open), pressed(&bound.select_all), pressed(&bound.new_dupe));
        let del = pressed(&bound.delete) || (bound.delete == "Delete" && pressed("Backspace"));
        let (esc, w, e, f) = (pressed(&bound.deselect), pressed(&bound.move_handle), pressed(&bound.rotate_handle), pressed(&bound.frame));
        // Mid-flight, Ctrl is the slow key and A, S and D steer: nothing with
        // Ctrl is a command until the camera has come to rest.
        let cmd = cmd && !self.flying();

        if !cmd && pressed(&bound.add) {
            self.add_panel.open();
        }
        if !cmd && pressed(&bound.scale_handle) {
            self.gizmo = Gizmo::Scale;
        }
        if cmd && n {
            self.new_dupe(if shift { menus::NewDupe::Baseplate } else { menus::NewDupe::Empty });
        } else if cmd && z && shift {
            self.redo();
        } else if cmd && z {
            self.undo();
        } else if cmd && y {
            self.redo();
        } else if cmd && c {
            self.copy_selection();
        } else if cmd && v {
            self.paste_clipboard();
        } else if cmd && d {
            let t = self.targets();
            if !t.is_empty() {
                self.op_duplicate(&t);
            }
        } else if cmd && save && shift {
            self.save_with_dialog();
        } else if cmd && save {
            if let Some(path) = self.open.as_ref().map(|o| o.path.clone()) {
                self.save_as(path);
            }
        } else if cmd && o {
            self.open_with_dialog();
        } else if cmd && a {
            if let Some(open) = self.open.as_ref() {
                self.marked = open.dupe.list_entities().into_iter().map(|(i, _, _)| i).collect();
            }
        } else if del {
            let t = self.targets();
            if !t.is_empty() {
                self.op_delete(&t);
            }
        } else if esc {
            self.deselect();
        } else if !cmd && w {
            self.gizmo = Gizmo::Move;
        } else if !cmd && e {
            self.gizmo = Gizmo::Rotate;
        } else if !cmd && f {
            self.frame_selection();
        }
    }

    /// The context menu, drawn as a floating area rather than through egui's
    /// own context_menu. That is what lets both a right-click and a
    /// double-click raise the same menu — egui's version only answers to a
    /// secondary click.
    fn floating_menu(&mut self, ui: &mut Ui) {
        let Some(at) = self.menu_at else { return };

        let resp = egui::Area::new(egui::Id::new("ad2edit_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ui.ctx(), |ui| {
                egui::Frame::default()
                    .fill(colours().menu_top)
                    .stroke(Stroke::new(1.0, colours().popup_edge))
                    .inner_margin(Margin::same(4))
                    .show(ui, |ui| {
                        ui.set_min_width(170.0);
                        self.context_menu(ui);
                    });
            })
            .response;

        if resp.clicked_elsewhere() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.menu_at = None;
        }
    }

    /// Link mode: a click on an entity while a source is armed links them
    /// and consumes the click. Returns true when it did.
    fn link_click(&mut self, target: f64) -> bool {
        let Some(src) = self.link_mode.take() else { return false };
        if src == target {
            self.say("link cancelled (same entity)");
            return true;
        }
        self.snapshot();
        let result = {
            let Some(open) = self.open.as_mut() else { return true };
            ad2read::build::add_link(&mut open.dupe, src, target)
        };
        match result {
            Ok(r) => {
                if let Some(open) = self.open.as_mut() {
                    open.dirty = true;
                }
                self.good(format!("linked {} -> {} via {} ({:.0} units)", r.owner, r.target, r.modifier, r.distance));
            }
            Err(e) => self.bad(format!("link: {e}")),
        }
        true
    }

    /// Where a newly spawned part lands: above the selection, else in front
    /// of the camera.
    fn spawn_point(&self) -> (f64, f64, f64) {
        if let (Some(Pick::Entity(i)), Some(open)) = (&self.pick, self.open.as_ref()) {
            if let Some((p, _)) = transform::entity_transform(&open.dupe, *i) {
                return (p.0, p.1, p.2 + 40.0);
            }
        }
        let f = self.camera.forward();
        let e = self.camera.eye;
        (e.0 + f.0 * 150.0, e.1 + f.1 * 150.0, e.2 + f.2 * 150.0)
    }

    fn deselect(&mut self) {
        self.pick = None;
        self.marked.clear();
        self.crumbs.clear();
        self.menu_at = None;
    }

    fn undo(&mut self) {
        let Some(open) = self.open.as_mut() else { return };
        let mut d = std::mem::replace(
            &mut open.dupe,
            Dupe { root: Value::Nil, arena: Vec::new() },
        );
        let ok = open.history.undo(&mut d);
        open.dupe = d;
        if ok {
            open.dirty = true;
            self.edit_count += 1;
            self.crumbs.clear();
            self.good("undo");
        } else {
            self.say("nothing to undo");
        }
    }

    fn redo(&mut self) {
        let Some(open) = self.open.as_mut() else { return };
        let mut d = std::mem::replace(
            &mut open.dupe,
            Dupe { root: Value::Nil, arena: Vec::new() },
        );
        let ok = open.history.redo(&mut d);
        open.dupe = d;
        if ok {
            open.dirty = true;
            self.edit_count += 1;
            self.crumbs.clear();
            self.good("redo");
        } else {
            self.say("nothing to redo");
        }
    }

    fn frame_selection(&mut self) {
        let targets = self.targets();
        if targets.is_empty() {
            return self.frame_camera();
        }
        let Some(open) = self.open.as_ref() else { return };
        let mut lo = (f64::MAX, f64::MAX, f64::MAX);
        let mut hi = (f64::MIN, f64::MIN, f64::MIN);
        let mut any = false;
        for t in &targets {
            if let Some((pt, _)) = transform::entity_transform(&open.dupe, *t) {
                lo = (lo.0.min(pt.0), lo.1.min(pt.1), lo.2.min(pt.2));
                hi = (hi.0.max(pt.0), hi.1.max(pt.1), hi.2.max(pt.2));
                any = true;
            }
        }
        if any {
            let span = sub3(hi, lo);
            self.camera
                .look_at(scale3(add3(lo, hi), 0.5), dot3(span, span).sqrt() * 0.5 + 24.0);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        self.menus(ui);
        ui.horizontal(|ui| {
            if button(ui, "Open", true).clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("AdvDupe2", &["txt"])
                    .pick_file()
                {
                    self.load(p);
                }
            }

            let has = self.open.is_some();
            if button(ui, "Save", has).clicked() && has {
                if let Some(p) = self.open.as_ref().map(|o| o.path.clone()) {
                    self.save_as(p);
                }
            }
            if button(ui, "Save As", has).clicked() && has {
                let start = self.open.as_ref().map(|o| o.path.clone());
                let mut dlg = rfd::FileDialog::new().add_filter("AdvDupe2", &["txt"]);
                if let Some(p) = start.as_ref().and_then(|p| p.parent()) {
                    dlg = dlg.set_directory(p);
                }
                if let Some(p) = dlg.save_file() {
                    self.save_as(p);
                }
            }

            ui.separator();

            if button(ui, "Undo", has).clicked() && has {
                if let Some(open) = self.open.as_mut() {
                    let mut d = std::mem::replace(
                        &mut open.dupe,
                        Dupe {
                            root: Value::Nil,
                            arena: Vec::new(),
                        },
                    );
                    let ok = open.history.undo(&mut d);
                    open.dupe = d;
                    if ok {
                        open.dirty = true;
                        self.edit_count += 1;
                        self.crumbs.clear();
                        self.good("undo");
                    } else {
                        self.say("nothing to undo");
                    }
                }
            }
            if button(ui, "Redo", has).clicked() && has {
                if let Some(open) = self.open.as_mut() {
                    let mut d = std::mem::replace(
                        &mut open.dupe,
                        Dupe {
                            root: Value::Nil,
                            arena: Vec::new(),
                        },
                    );
                    let ok = open.history.redo(&mut d);
                    open.dupe = d;
                    if ok {
                        open.dirty = true;
                        self.edit_count += 1;
                        self.crumbs.clear();
                        self.good("redo");
                    } else {
                        self.say("nothing to redo");
                    }
                }
            }

            ui.separator();

            for (mode, label) in [(Gizmo::Move, "Move"), (Gizmo::Rotate, "Rotate"), (Gizmo::Scale, "Scale")] {
                let on = self.gizmo == mode;
                if button(ui, label, true).clicked() {
                    self.gizmo = mode;
                }
                if on {
                    // Mark the active mode: SFM shows state by colour, not chrome.
                    ui.label(RichText::new("\u{2022}").color(colours().accent));
                }
            }

            ui.separator();

            match self.open.as_ref() {
                None => {
                    ui.label(RichText::new("no file open").color(colours().text_dim));
                }
                Some(open) => {
                    let name = open
                        .path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    ui.label(RichText::new(name).color(colours().text));
                    if open.dirty {
                        ui.label(RichText::new("\u{25cf} unsaved").color(colours().accent));
                    }
                    let e = open.dupe.list_entities().len();
                    let c = constraint_list(&open.dupe).len();
                    ui.label(
                        RichText::new(format!("{e} entities \u{b7} {c} constraints")).color(colours().text_dim),
                    );
                }
            }
            self.weight_label(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, "Settings", true).clicked() {
                    self.show_settings = true;
                }
            });
        });
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("LOG").color(colours().text_dim).small());
            if tool_button(ui, "clear").clicked() {
                self.log.clear();
            }
        });
        ui.separator();
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (line, color) in &self.log {
                    ui.label(RichText::new(line).color(*color).font(mono_font()));
                }
            });
    }

    fn tree_panel(&mut self, ui: &mut egui::Ui) {
        section_header(ui, "CONTENTS");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            // Toggle first, then the filter takes whatever is left — the
            // order matters (see the note in the scroll area below).
            if ui.selectable_label(self.tree_view, "tree").on_hover_text("parent hierarchy; drag rows to parent").clicked() {
                self.tree_view = !self.tree_view;
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("filter")
                    .desired_width(ui.available_width() - 4.0),
            );
        });
        ui.add_space(4.0);

        let Some(open) = self.open.as_ref() else {
            ui.label(RichText::new("  open a dupe to begin").color(colours().text_dim));
            return;
        };

        let needle = self.filter.to_lowercase();
        let entities = open.dupe.list_entities();
        let constraints = constraint_list(&open.dupe);

        // Gather the rows before drawing, so the immutable borrow of the dupe
        // ends before a click mutates self.
        let ent_rows: Vec<(f64, String)> = entities
            .iter()
            .map(|(i, class, model)| {
                let class = class.trim_matches('"').to_string();
                let model = model.trim_matches('"');
                let short = model.rsplit('/').next().unwrap_or(model).to_string();
                (*i, format!("{i:>5}  {class}  {short}"))
            })
            .filter(|(_, s)| needle.is_empty() || s.to_lowercase().contains(&needle))
            .collect();

        let con_rows: Vec<(usize, String)> = constraints
            .iter()
            .enumerate()
            .map(|(pos, ct)| {
                let kind = open
                    .dupe
                    .get(*ct, "Type")
                    .map(as_text)
                    .unwrap_or_else(|| "?".into());
                let ends = open
                    .dupe
                    .get_table(*ct, "Entity")
                    .map(|list| match &open.dupe.arena[list] {
                        Node::Array(items) => items
                            .iter()
                            .filter_map(|it| open.dupe.get_number(table_index(it)?, "Index"))
                            .map(|n| format!("{n}"))
                            .collect::<Vec<_>>()
                            .join(" ↔ "),
                        _ => String::new(),
                    })
                    .unwrap_or_default();
                (pos, format!("{pos:>5}  {kind}  {ends}"))
            })
            .filter(|(_, s)| needle.is_empty() || s.to_lowercase().contains(&needle))
            .collect();

        // Parent hierarchy: children keyed by DupeParentID, roots first.
        // f64 isn't Hash, so maps key on the bit pattern.
        type K = u64;
        let parent_of: std::collections::HashMap<K, f64> = entities
            .iter()
            .filter_map(|(i, _, _)| {
                let et = transform::entity_table(&open.dupe, *i)?;
                let bdi = open.dupe.get_table(et, "BuildDupeInfo")?;
                open.dupe.get_number(bdi, "DupeParentID").map(|p| (i.to_bits(), p))
            })
            .collect();
        let tree_rows: Vec<(f64, String, usize)> = if self.tree_view {
            let label_of: std::collections::HashMap<K, String> =
                ent_rows.iter().map(|(i, r)| (i.to_bits(), r.clone())).collect();
            let mut kids: std::collections::HashMap<K, Vec<f64>> = Default::default();
            let mut roots: Vec<f64> = Vec::new();
            for (i, _, _) in &entities {
                match parent_of.get(&i.to_bits()) {
                    Some(p) if entities.iter().any(|(j, _, _)| j == p) => kids.entry(p.to_bits()).or_default().push(*i),
                    _ => roots.push(*i),
                }
            }
            let mut out = Vec::new();
            fn walk(i: f64, depth: usize, kids: &std::collections::HashMap<u64, Vec<f64>>, label_of: &std::collections::HashMap<u64, String>, out: &mut Vec<(f64, String, usize)>) {
                if let Some(l) = label_of.get(&i.to_bits()) {
                    out.push((i, l.clone(), depth));
                }
                if let Some(ch) = kids.get(&i.to_bits()) {
                    let mut ch = ch.clone();
                    ch.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    for c in ch {
                        walk(c, depth + 1, kids, label_of, out);
                    }
                }
            }
            roots.sort_by(|a, b| {
                // Most children first, so the hull leads.
                let ka = kids.get(&a.to_bits()).map(|v| v.len()).unwrap_or(0);
                let kb = kids.get(&b.to_bits()).map(|v| v.len()).unwrap_or(0);
                kb.cmp(&ka).then(a.partial_cmp(b).unwrap())
            });
            for r in roots {
                walk(r, 0, &kids, &label_of, &mut out);
            }
            out
        } else {
            ent_rows.iter().map(|(i, r)| (*i, r.clone(), 0)).collect()
        };
        let ent_rows: Vec<(f64, String)> = tree_rows
            .iter()
            .map(|(i, r, d)| (*i, format!("{}{}", "   ".repeat(*d), if *d > 0 { format!("└ {}", r.trim_start()) } else { r.clone() })))
            .collect();

        sfm_scroll(ui, |ui| {
                // The header allocates the full available width, so it must
                // never share a horizontal row: a sibling after it pushes the
                // row past the panel, the panel grows to fit, and next frame
                // the header takes the new width — the panel slides open a
                // little more every frame.
                sfm_header(ui, &format!("ENTITIES ({})", ent_rows.len()));
                if self.tree_view {
                    // Drop target for unparenting. Fixed-width label so it
                    // can't widen the panel either.
                    let (zone, dropped) = ui.dnd_drop_zone::<f64, ()>(egui::Frame::default(), |ui| {
                        ui.add(egui::Label::new(RichText::new("  drop here to unparent").color(colours().text_dim)).truncate());
                    });
                    let _ = zone;
                    if let Some(child) = dropped {
                        self.set_parent(&[*child], None);
                    }
                }
                let mut drop_onto: Option<(f64, f64)> = None;
                for (n, (index, row)) in ent_rows.iter().enumerate() {
                    let selected = self.pick == Some(Pick::Entity(*index))
                        || self.marked.contains(index);
                    let hit = if self.tree_view {
                        let inner = ui.dnd_drag_source(egui::Id::new(("ent-row", index.to_bits())), *index, |ui| {
                            sfm_row(ui, row, selected, n % 2 == 1)
                        });
                        if let Some(child) = inner.response.dnd_release_payload::<f64>() {
                            drop_onto = Some((*child, *index));
                        }
                        inner.inner
                    } else {
                        sfm_row(ui, row, selected, n % 2 == 1)
                    };

                    // Right-click and double-click both raise the menu, since
                    // a list you can act on shouldn't care which you reach for.
                    if hit.secondary_clicked() || hit.double_clicked() {
                        self.pick = Some(Pick::Entity(*index));
                        self.menu_at = hit.interact_pointer_pos();
                    } else if hit.clicked() {
                        if self.link_click(*index) {
                        } else if ui.input(|i| i.modifiers.command) {
                            match self.marked.iter().position(|m| m == index) {
                                Some(at) => {
                                    self.marked.remove(at);
                                }
                                None => self.marked.push(*index),
                            }
                        } else if self.pick == Some(Pick::Entity(*index)) {
                            // Clicking what's already selected clears it.
                            self.pick = None;
                            self.crumbs.clear();
                        } else {
                            self.pick = Some(Pick::Entity(*index));
                            self.crumbs.clear();
                        }
                    }
                }

                if let Some((child, parent)) = drop_onto {
                    if child != parent {
                        self.set_parent(&[child], Some(parent));
                    }
                }

                ui.add_space(8.0);
                sfm_header(ui, &format!("CONSTRAINTS ({})", con_rows.len()));
                for (n, (pos, row)) in con_rows.iter().enumerate() {
                    let selected = self.pick == Some(Pick::Constraint(*pos));
                    if sfm_row(ui, row, selected, n % 2 == 1).clicked() {
                        if selected {
                            self.pick = None;
                        } else {
                            self.pick = Some(Pick::Constraint(*pos));
                        }
                        self.crumbs.clear();
                    }
                }
            });
    }

    /// The arena table the inspector is currently showing, following the
    /// breadcrumb path down from whatever is selected.
    fn current_table(&self) -> Option<usize> {
        if let Some(last) = self.crumbs.last() {
            return Some(last.table);
        }
        let open = self.open.as_ref()?;
        match self.pick.as_ref()? {
            Pick::Entity(i) => transform::entity_table(&open.dupe, *i),
            Pick::Constraint(pos) => constraint_list(&open.dupe).get(*pos).copied(),
        }
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        if self.open.is_none() {
            ui.label(RichText::new("no file open").color(colours().text_dim));
            return;
        }
        let Some(pick) = self.pick.clone() else {
            ui.label(RichText::new("select something on the left").color(colours().text_dim));
            return;
        };

        // Breadcrumbs
        ui.horizontal(|ui| {
            let root_label = match &pick {
                Pick::Entity(i) => format!("entity {i}"),
                Pick::Constraint(p) => format!("constraint {p}"),
            };
            if ui.selectable_label(self.crumbs.is_empty(), root_label).clicked() {
                self.crumbs.clear();
            }
            let mut climb_to: Option<usize> = None;
            for (depth, crumb) in self.crumbs.clone().iter().enumerate() {
                ui.label(RichText::new("\u{203a}").color(colours().text_dim));
                if ui.selectable_label(false, &crumb.label).clicked() {
                    climb_to = Some(depth + 1);
                }
            }
            if let Some(d) = climb_to {
                self.crumbs.truncate(d);
            }
        });
        ui.separator();

        let Some(table) = self.current_table() else {
            ui.label(RichText::new("nothing there").color(colours().bad));
            return;
        };

        sfm_scroll(ui, |ui| {
                if let (Pick::Entity(index), true) = (&pick, self.crumbs.is_empty()) {
                    // DT holds the networked vars the duplicator restores on
                    // paste — the AIO controller's brake, steering and HUD
                    // settings live there and nowhere else.
                    let dt = self.open.as_ref().and_then(|o| {
                        let et = transform::entity_table(&o.dupe, *index)?;
                        o.dupe.get_table(et, "DT")
                    });
                    if let Some(t) = dt {
                        if ui.small_button("DT (networked vars)...").clicked() {
                            self.crumbs.push(Crumb { label: "DT".into(), table: t });
                        }
                    }
                    self.transform_block(ui, *index);
                    let _ = self.size_row(ui, *index);
                    self.clip_editor(ui, *index);
                    if self.is_chip(*index) && ui.small_button("Edit chip").clicked() {
                        self.open_chip_editor(*index);
                    }
                }
                self.table_fields(ui, table);
            });
    }

    /// Length, width and height for the parts a builder sizes by hand: a
    /// baseplate, a crate, a fuel tank. Written through scale::resize, which
    /// holds them inside ACF's limits and keeps every copy of the size in the
    /// dupe in step, so the part is redrawn at once.
    /// True when the part was a crate or a tank and got its own controls.
    fn size_row(&mut self, ui: &mut egui::Ui, index: f64) -> bool {
        if self.container_properties(ui, index) {
            return true;
        }
        let Some(size) = self.open.as_ref().and_then(|open| ad2read::scale::dimensions(&open.dupe, index)) else { return false };
        let mut wanted = size;
        let (mut began, mut changed) = (false, false);
        ui.horizontal(|ui| {
            ui.label(RichText::new("size").color(colours().text_dim));
            for (label, value) in [("l", &mut wanted.0), ("w", &mut wanted.1), ("h", &mut wanted.2)] {
                let r = ui.add(egui::DragValue::new(value).speed(0.5).fixed_decimals(1).prefix(format!("{label} ")));
                began |= r.drag_started() || r.gained_focus();
                changed |= r.changed();
            }
        });
        if began {
            self.snapshot();
        }
        if changed && wanted != size {
            let resized = self.open.as_mut().map(|open| {
                open.dirty = true;
                ad2read::scale::resize(&mut open.dupe, index, wanted)
            });
            if let Some(Err(e)) = resized {
                self.bad(format!("size: {e}"));
            }
            self.edit_count += 1;
        }
        false
    }

    /// Position and angle, routed through set_entity_transform so ragdoll
    /// bones come along. Nothing else in this UI writes a transform.
    fn transform_block(&mut self, ui: &mut egui::Ui, index: f64) {
        let Some(open) = self.open.as_ref() else { return };
        let Some((pos, ang)) = transform::entity_transform(&open.dupe, index) else {
            return;
        };

        section_header(ui, "TRANSFORM");
        ui.add_space(3.0);

        let mut new_pos = pos;
        let mut new_ang = ang;
        let mut began = false;
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.label(RichText::new("pos").color(colours().text_dim));
            for (label, v) in [
                ("x", &mut new_pos.0),
                ("y", &mut new_pos.1),
                ("z", &mut new_pos.2),
            ] {
                let r = ui.add(
                    egui::DragValue::new(v)
                        .speed(0.5)
                        .fixed_decimals(3)
                        .prefix(format!("{label} ")),
                );
                began |= r.drag_started();
                changed |= r.changed();
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("ang").color(colours().text_dim));
            for (label, v) in [
                ("p", &mut new_ang.0),
                ("y", &mut new_ang.1),
                ("r", &mut new_ang.2),
            ] {
                let r = ui.add(
                    egui::DragValue::new(v)
                        .speed(0.25)
                        .fixed_decimals(3)
                        .prefix(format!("{label} ")),
                );
                began |= r.drag_started();
                changed |= r.changed();
            }
        });

        if began {
            self.snapshot();
        }
        if changed {
            let moved = new_pos != pos;
            let turned = new_ang != ang;
            if let Some(open) = self.open.as_mut() {
                let res = transform::set_entity_transform(
                    &mut open.dupe,
                    index,
                    if moved { Some(new_pos) } else { None },
                    if turned { Some(new_ang) } else { None },
                );
                open.dirty = true;
                if let Err(e) = res {
                    self.bad(e);
                }
            }
        }

        ui.add_space(8.0);
    }

    /// Draws every key/value in a table, one row each, and applies whatever
    /// the user changed.
    fn table_fields(&mut self, ui: &mut egui::Ui, table: usize) {
        let Some(open) = self.open.as_ref() else { return };

        // Snapshot the rows so the borrow ends before any write.
        let rows: Vec<(String, Value)> = match &open.dupe.arena[table] {
            Node::Table(entries) => entries
                .iter()
                .map(|(k, v)| (key_label(k), v.clone()))
                .collect(),
            Node::Array(items) => items
                .iter()
                .enumerate()
                .map(|(i, v)| (format!("[{i}]"), v.clone()))
                .collect(),
        };

        let mut writes: Vec<(usize, Value)> = Vec::new();
        let mut descend: Option<Crumb> = None;
        let mut began = false;

        for (row, (label, value)) in rows.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [190.0, 18.0],
                    egui::Label::new(RichText::new(label).color(colours().text).font(mono_font()))
                        .truncate(),
                );

                let mut v = value.clone();
                let mut edited = false;

                match &mut v {
                    Value::Number(n) => {
                        let r = ui.add(egui::DragValue::new(n).speed(0.1));
                        began |= r.drag_started();
                        edited |= r.changed();
                    }
                    Value::Bool(b) => {
                        let r = ui.checkbox(b, "");
                        if r.changed() {
                            began = true;
                            edited = true;
                        }
                    }
                    Value::Vector(x, y, z) => {
                        for (l, c) in [("x", x), ("y", y), ("z", z)] {
                            let r = ui.add(
                                egui::DragValue::new(c)
                                    .speed(0.25)
                                    .fixed_decimals(3)
                                    .prefix(format!("{l} ")),
                            );
                            began |= r.drag_started();
                            edited |= r.changed();
                        }
                    }
                    Value::Angle(p, y, rr) => {
                        for (l, c) in [("p", p), ("y", y), ("r", rr)] {
                            let r = ui.add(
                                egui::DragValue::new(c)
                                    .speed(0.25)
                                    .fixed_decimals(3)
                                    .prefix(format!("{l} ")),
                            );
                            began |= r.drag_started();
                            edited |= r.changed();
                        }
                    }
                    Value::Str(bytes) => {
                        // Only offer text editing when the bytes actually are
                        // text. Lua strings can hold anything, and round-
                        // tripping arbitrary bytes through a String would
                        // corrupt them.
                        match String::from_utf8(bytes.clone()) {
                            Ok(mut s) => {
                                let r = ui.add(
                                    egui::TextEdit::singleline(&mut s)
                                        .desired_width(f32::INFINITY),
                                );
                                if r.gained_focus() {
                                    began = true;
                                }
                                if r.changed() {
                                    *bytes = s.into_bytes();
                                    edited = true;
                                }
                            }
                            Err(_) => {
                                ui.label(
                                    RichText::new(format!("<{} raw bytes>", bytes.len()))
                                        .color(colours().bad),
                                );
                            }
                        }
                    }
                    Value::Nil => {
                        ui.label(RichText::new("nil").color(colours().text_dim));
                    }
                    Value::Table(t) => {
                        let t = *t;
                        let count = match &open.dupe.arena[t] {
                            Node::Table(e) => format!("table · {} keys", e.len()),
                            Node::Array(i) => format!("array · {} items", i.len()),
                        };
                        if tool_button(ui, &format!("{count}  \u{203a}")).clicked() {
                            descend = Some(Crumb {
                                label: label.clone(),
                                table: t,
                            });
                        }
                    }
                }

                if edited {
                    writes.push((row, v));
                }
            });
        }

        if began {
            self.snapshot();
        }

        if !writes.is_empty() {
            if let Some(open) = self.open.as_mut() {
                match &mut open.dupe.arena[table] {
                    Node::Table(entries) => {
                        for (row, v) in writes {
                            entries[row].1 = v;
                        }
                    }
                    Node::Array(items) => {
                        for (row, v) in writes {
                            items[row] = v;
                        }
                    }
                }
                open.dirty = true;
            }
        }

        if let Some(c) = descend {
            self.crumbs.push(c);
        }
    }

    /// Whichever entities an operation should act on: the ticked set if there
    /// is one, otherwise the single selection. Duplicating one prop and
    /// duplicating a whole turret are the same command with a different set.
    fn targets(&self) -> Vec<f64> {
        if !self.marked.is_empty() {
            return self.marked.clone();
        }
        match self.pick {
            Some(Pick::Entity(i)) => vec![i],
            _ => Vec::new(),
        }
    }

    fn ops_panel(&mut self, ui: &mut Ui) {
        let open = self.open.is_some();
        let targets = self.targets();

        sfm_header(ui, "OPERATIONS");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let label = if self.marked.is_empty() {
                match self.pick {
                    Some(Pick::Entity(i)) => format!("acting on entity {i}"),
                    _ => "nothing selected".to_owned(),
                }
            } else {
                format!("{} entities ticked", self.marked.len())
            };
            ui.label(RichText::new(label).color(colours().text_dim));
        });
        ui.add_space(2.0);
        let has_ticks = !self.marked.is_empty();
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            if button(ui, "Clear ticks", has_ticks).clicked() && has_ticks {
                self.marked.clear();
            }
            ui.label(RichText::new("ctrl-click to tick").color(colours().text_dim));
        });

        ui.add_space(8.0);
        sfm_header(ui, "SELECTION SETS");
        ui.add_space(4.0);
        let mut recall: Option<usize> = None;
        let mut drop: Option<usize> = None;
        for (i, (name, members)) in self.sets.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                if tool_button(ui, &format!("{name} ({})", members.len())).clicked() {
                    recall = Some(i);
                }
                if tool_button(ui, "x").clicked() {
                    drop = Some(i);
                }
            });
        }
        if let Some(i) = recall {
            self.marked = self.sets[i].1.clone();
        }
        if let Some(i) = drop {
            self.sets.remove(i);
        }
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.new_set_name)
                    .hint_text("name")
                    .desired_width(90.0),
            );
            let can = !self.marked.is_empty() && !self.new_set_name.trim().is_empty();
            if button(ui, "Save ticked", can).clicked() && can {
                let name = self.new_set_name.trim().to_owned();
                self.sets.push((name, self.marked.clone()));
                self.new_set_name.clear();
            }
        });

        ui.add_space(8.0);
        sfm_header(ui, "MAP");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.map_name)
                    .hint_text("gm_construct")
                    .desired_width(110.0),
            );
            let can = self.models.is_some();
            if button(ui, "Load", can).clicked() && can {
                self.load_map();
            }
            if button(ui, "Browse...", true).clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Source map", &["bsp"])
                    .pick_file()
                {
                    self.load_map_from(path);
                }
            }
        });
        if self.map.is_some() {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.checkbox(&mut self.show_map, "draw map");
                let (leaves, draws, tris) = self.map_stats;
                ui.label(
                    RichText::new(format!("{leaves} leaves · {draws} draws · {tris} tris"))
                        .color(colours().text_dim),
                );
            });
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                if button(ui, "Dupe to spawn", true).clicked() {
                    self.dupe_to_spawn();
                }
                if button(ui, "Unload", true).clicked() {
                    self.map = None;
                    self.map_batches.clear();
                }
            });
        }

        ui.add_space(8.0);
        sfm_header(ui, "OVERLAYS");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.checkbox(&mut self.show_links, "ACF links");
            ui.checkbox(&mut self.show_wires, "wires");
            ui.checkbox(&mut self.show_arcs, "gun arcs");
        });

        ui.add_space(8.0);
        sfm_header(ui, "MODELS");
        ui.add_space(4.0);
        let loaded = self.models.is_some();
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.label(
                RichText::new(if loaded {
                    "real sizes loaded"
                } else {
                    "boxes are uniform until loaded"
                })
                .color(colours().text_dim),
            );
        });
        if !loaded {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("game folder: Settings").color(colours().text_dim).small());
            });
        }

        ui.add_space(8.0);
        sfm_header(ui, "OFFSET");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            for (l, v) in [
                ("x", &mut self.op_offset.0),
                ("y", &mut self.op_offset.1),
                ("z", &mut self.op_offset.2),
            ] {
                ui.add(
                    egui::DragValue::new(v)
                        .speed(1.0)
                        .fixed_decimals(1)
                        .prefix(format!("{l} ")),
                );
            }
        });

        ui.add_space(8.0);
        sfm_header(ui, "SELECTION");
        ui.add_space(4.0);
        let can = open && !targets.is_empty();
        let mut do_delete = false;
        let mut do_duplicate = false;
        let mut do_mirror = false;
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            do_delete = button(ui, "Delete", can).clicked() && can;
            do_duplicate = button(ui, "Duplicate", can).clicked() && can;
            ui.add(egui::DragValue::new(&mut self.array_count).range(1..=40).prefix("x ")).on_hover_text("More than one makes a row, each copy the offset further on than the last");
        });
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            do_mirror = button(ui, "Mirror across the hull", can).on_hover_text("A reflected copy on the other side of the hull's centreline").clicked() && can;
            ui.checkbox(&mut self.mirror_props_only, "props only").on_hover_text("Leave guns, crew, turrets and other ACF parts alone");
        });

        ui.add_space(8.0);
        sfm_header(ui, "BUILD");
        ui.add_space(4.0);
        // 19: link mode
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let one = matches!(self.pick, Some(Pick::Entity(_)));
            match self.link_mode {
                Some(src) => {
                    ui.label(RichText::new(format!("linking from {src}: click the target (Esc cancels)")).color(colours().selection_text));
                }
                None => {
                    if button(ui, "Link from selected...", one && open).clicked() {
                        if let Some(Pick::Entity(i)) = self.pick {
                            self.link_mode = Some(i);
                        }
                    }
                }
            }
        });
        // 18: wheel action
        {
            let (bases, gearboxes): (Vec<f64>, Vec<f64>) = match self.open.as_ref() {
                Some(o) => {
                    let all = o.dupe.list_entities();
                    (
                        all.iter().filter(|(_, c, _)| c.contains("acf_baseplate")).map(|(i, _, _)| *i).collect(),
                        all.iter().filter(|(_, c, _)| c.contains("acf_gearbox")).map(|(i, _, _)| *i).collect(),
                    )
                }
                None => (Vec::new(), Vec::new()),
            };
            if self.wheel_base.map(|b| !bases.contains(&b)).unwrap_or(true) {
                self.wheel_base = bases.first().copied();
            }
            if self.wheel_gearbox.map(|g| !gearboxes.contains(&g)).unwrap_or(false) {
                self.wheel_gearbox = None;
            }
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("wheel:").color(colours().text_dim));
                egui::ComboBox::from_id_salt("wheel-base")
                    .selected_text(self.wheel_base.map(|b| format!("base {b}")).unwrap_or_else(|| "no baseplate".into()))
                    .show_ui(ui, |ui| {
                        for b in &bases {
                            ui.selectable_value(&mut self.wheel_base, Some(*b), format!("base {b}"));
                        }
                    });
                egui::ComboBox::from_id_salt("wheel-gearbox")
                    .selected_text(self.wheel_gearbox.map(|g| format!("gearbox {g}")).unwrap_or_else(|| "no gearbox link".into()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.wheel_gearbox, None, "no gearbox link");
                        for g in &gearboxes {
                            ui.selectable_value(&mut self.wheel_gearbox, Some(*g), format!("gearbox {g}"));
                        }
                    });
                let can = matches!(self.pick, Some(Pick::Entity(_))) && self.wheel_base.is_some();
                if button(ui, "Make wheel", can).clicked() {
                    if let (Some(Pick::Entity(w)), Some(base)) = (self.pick.clone(), self.wheel_base) {
                        self.make_wheel(w, base, self.wheel_gearbox);
                    }
                }
            });
        }
        // 25: suspension recipes on the ticked wheels, previewed in the
        // viewport before Apply.
        {
            let ticked = self.marked.len();
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("suspension:").color(colours().text_dim));
                for (c, label) in [(SuspensionChoice::Simple, "simple"), (SuspensionChoice::Locked, "locked"), (SuspensionChoice::Sprung, "sprung")] {
                    if ui.selectable_label(self.suspension == c, label).clicked() {
                        self.suspension = c;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                let can = ticked >= 1 && self.wheel_base.is_some() && open;
                if button(ui, &format!("Apply to {ticked} ticked wheel(s)"), can).clicked() {
                    if let Some(base) = self.wheel_base {
                        self.apply_suspension(base);
                    }
                }
                ui.label(RichText::new("preview drawn on the ticked wheels").color(colours().text_dim).small());
            });
        }

        ui.add_space(8.0);
        sfm_header(ui, "MERGE");
        ui.add_space(4.0);
        let mut do_merge = false;
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            do_merge = button(ui, "Merge a dupe in...", open).clicked() && open;
        });

        ui.add_space(8.0);
        sfm_header(ui, "BULK EDIT");
        ui.add_space(4.0);
        let w = (ui.available_width() - 62.0).max(60.0);
        for (label, buf) in [
            ("where", &mut self.where_key),
            ("=", &mut self.where_val),
            ("set", &mut self.set_key),
            ("=", &mut self.set_val),
        ] {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.add_sized(
                    [40.0, 18.0],
                    egui::Label::new(RichText::new(label).color(colours().text_dim)),
                );
                ui.add(egui::TextEdit::singleline(buf).desired_width(w));
            });
        }
        ui.add_space(3.0);
        let ready = open && !self.set_key.is_empty();
        let mut do_bulk = false;
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            do_bulk = button(ui, "Apply to all matches", ready).clicked() && ready;
        });
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.label(
                RichText::new("blank `where` hits every entity")
                    .color(colours().text_dim)
                    .small(),
            );
        });

        // Run the work after the layout closures have released their borrows.
        if do_delete {
            self.op_delete(&targets);
        }
        if do_duplicate {
            self.op_duplicate(&targets);
        }
        if do_mirror {
            self.op_mirror(&targets);
        }
        if do_merge {
            self.op_merge();
        }
        if do_bulk {
            self.op_bulk();
        }
    }

    /// Loads a map by name from the same archives models come from —
    /// `maps/<name>.bsp` sits in the game's VPKs and in workshop GMAs.
    fn load_map(&mut self) {
        let name = self.map_name.trim().trim_end_matches(".bsp").to_owned();
        let path = format!("maps/{name}.bsp");
        let bytes = self.models.as_ref().and_then(|m| m.read_file(&path)).map(|(b, _)| b);
        match bytes {
            Some(b) => self.install_map(&name, &b),
            None => self.bad(format!("no {path} in any indexed archive")),
        }
    }

    fn load_map_from(&mut self, path: std::path::PathBuf) {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "map".into());
        match std::fs::read(&path) {
            Ok(b) => self.install_map(&name, &b),
            Err(e) => self.bad(format!("could not read map: {e}")),
        }
    }

    /// Parses the BSP, resolves every material, uploads one batch per
    /// material. GPU upload happens here rather than per frame, since a
    /// map's geometry never changes.
    fn install_map(&mut self, name: &str, bytes: &[u8]) {
        self.say(format!("loading {name}..."));
        let map = match ad2read::map::load(name, bytes) {
            Ok(m) => m,
            Err(e) => return self.bad(e),
        };
        self.good(format!(
            "{name}: {} materials, {} triangles, {} leaves, {} static props",
            map.batches.len(),
            map.triangles(),
            map.leaves.len(),
            map.props.len()
        ));

        self.map = Some(map);
        self.map_batches.clear();
        let Some(rs) = self.render_state.clone() else {
            self.say("map geometry needs the GPU renderer; it'll upload when the viewport draws");
            return;
        };
        self.upload_map_batches(&rs);
    }

    fn upload_map_batches(&mut self, rs: &eframe::egui_wgpu::RenderState) {
        if self.gpu.is_none() {
            self.gpu = Some(ad2read::gpu::Renderer::new(&rs.device, &rs.queue, rs.target_format));
        }
        let Some(map) = self.map.take() else { return };
        self.map_batches.clear();

        let mut missing = 0;
        // Why each failed, so the number means something.
        let mut reasons: std::collections::BTreeMap<&'static str, Vec<String>> =
            std::collections::BTreeMap::new();
        for batch in &map.batches {
            // The map's own packed files first, then the archives.
            let packed = |p: &str| map.packed(p);
            let texture = match self
                .models
                .as_mut()
                .map(|lib| lib.material_texture_diag(&batch.material, &packed))
            {
                Some(Ok(t)) => Some(t),
                Some(Err(why)) => {
                    missing += 1;
                    reasons.entry(why).or_default().push(batch.material.clone());
                    None
                }
                None => None,
            };
            let verts: Vec<ad2read::gpu::Vertex> = batch
                .vertices
                .iter()
                .map(|v| ad2read::gpu::Vertex {
                    position: v.position,
                    normal: v.normal,
                    uv: v.uv,
                    light: v.light,
                })
                .collect();
            let uploaded = self.gpu.as_ref().unwrap().upload_map_batch(
                &rs.device,
                &rs.queue,
                &verts,
                &batch.indices,
                texture.as_ref().map(|t| (t.width, t.height, t.rgba.as_slice())),
            );
            self.map_batches.push(uploaded);
        }
        if missing > 0 {
            self.say(format!(
                "{missing} of {} materials had no texture; those draw flat grey",
                map.batches.len()
            ));
            for (why, names) in &reasons {
                let sample: Vec<&str> = names.iter().take(4).map(|s| s.as_str()).collect();
                self.say(format!("  {}: {}, e.g. {}", why, names.len(), sample.join(", ")));
            }
        }

        // 2D skybox: six faces named <skyname>{rt,lf,bk,ft,up,dn} under
        // materials/skybox/. Built as six textured quads that get drawn as
        // ordinary instances, far out along each axis and centred on the eye
        // every frame. Depth testing does the rest: sky shows only where
        // nothing is nearer.
        self.sky.clear();
        if let Some(sky) = map.skyname.clone() {
            let mut got = 0;
            for (suffix, axis, u_dir, v_dir) in SKY_FACES {
                let packed = |p: &str| map.packed(p);
                let name = format!("skybox/{sky}{suffix}");
                let Some(tex) = self
                    .models
                    .as_mut()
                    .and_then(|lib| lib.material_texture(&name, &packed))
                else {
                    continue;
                };
                let quad = sky_quad(*axis, *u_dir, *v_dir);
                // A sky face is never see-through, whatever its alpha holds.
                let mut solid = tex.rgba.clone();
                ad2read::look::prepare_alpha(&mut solid, ad2read::look::AlphaUse::Opaque);
                let mesh = self.gpu.as_ref().unwrap().upload_mesh(
                    &rs.device,
                    &rs.queue,
                    &quad,
                    &[0, 1, 2, 0, 2, 3],
                    &[(0, 6, Some((tex.width, tex.height, solid.as_slice())), false)],
                );
                self.sky.push(mesh);
                got += 1;
            }
            self.say(format!("skybox {sky}: {got} of 6 faces"));
        }

        let displacements = map.displacements;
        self.map = Some(map);
        if displacements > 0 {
            self.say(format!("{displacements} displacement faces (terrain)"));
        }
        self.dupe_to_spawn();
    }



    /// Uploads a model with one texture per material section. `override_mat`
    /// is a material modifier — when set, every section uses it instead,
    /// which is what the Material tool does in-game.
    fn upload_model(
        &mut self,
        rs: &eframe::egui_wgpu::RenderState,
        model: &str,
        look: &ad2read::look::Look,
    ) -> Option<std::sync::Arc<ad2read::gpu::GpuMesh>> {
        use ad2read::gpu::Vertex;
        let override_mat = look.material.as_deref();
        let mesh = self.models.as_mut()?.mesh_as(model, look.skin, &look.bodygroups)?;
        let vertices: Vec<Vertex> = (0..mesh.positions.len())
            .map(|i| Vertex {
                position: mesh.positions[i],
                normal: *mesh.normals.get(i).unwrap_or(&[0.0, 0.0, 1.0]),
                uv: *mesh.uvs.get(i).unwrap_or(&[0.0, 0.0]),
                light: Vertex::UNLIT,
            })
            .collect();

        let override_tex = override_mat.and_then(|m| {
            self.models.as_mut().and_then(|lib| lib.material_texture(m, &|_| None))
        });
        // The SubMaterial tool replaces one slot of the model's material
        // list; the Material tool, or ACF's spawn material, replaces them all.
        let swapped: Vec<Option<&str>> = (0..mesh.sections.len())
            .map(|section| {
                let slot = mesh.section_slots.get(section).copied();
                look.submaterials.iter().find(|(wanted, _)| Some(*wanted) == slot).map(|(_, material)| material.as_str())
            })
            .collect();
        let textures: Vec<Option<std::sync::Arc<ad2read::models::Texture>>> = mesh
            .sections
            .iter()
            .zip(swapped.iter())
            .map(|((_, _, key), swapped)| match (swapped, &override_tex) {
                (Some(material), _) => self.models.as_mut().and_then(|lib| lib.material_texture(material, &|_| None)),
                (None, Some(t)) => Some(t.clone()),
                (None, None) => self.models.as_mut().and_then(|lib| lib.section_texture(key)),
            })
            .collect();
        // A texture's alpha means transparency only when its material says
        // so; otherwise it is a gloss mask and must not punch holes. The
        // alpha is rewritten to what the material means before upload.
        let override_vmt = override_mat.and_then(|m| self.models.as_ref().and_then(|lib| lib.material_vmt(m)));
        let prepared: Vec<Option<(u32, u32, Vec<u8>, bool)>> = mesh
            .sections
            .iter()
            .zip(textures.iter())
            .zip(swapped.iter())
            .map(|(((_, _, key), texture), swapped)| {
                let texture = texture.as_ref()?;
                let vmt = match swapped {
                    Some(material) => self.models.as_ref().and_then(|lib| lib.material_vmt(material)),
                    None => override_vmt.clone().or_else(|| self.models.as_ref().and_then(|lib| lib.section_vmt(key))),
                };
                let alpha = vmt.as_deref().map(ad2read::look::alpha_use).unwrap_or(ad2read::look::AlphaUse::Opaque);
                let mut rgba = texture.rgba.clone();
                let see_through = ad2read::look::prepare_alpha(&mut rgba, alpha);
                let blended = see_through && alpha == ad2read::look::AlphaUse::Blended;
                Some((texture.width, texture.height, rgba, blended))
            })
            .collect();
        let sections: Vec<(u32, u32, Option<(u32, u32, &[u8])>, bool)> = mesh
            .sections
            .iter()
            .zip(prepared.iter())
            .map(|((s0, c, _), ready)| match ready {
                Some((w, h, rgba, blended)) => (*s0, *c, Some((*w, *h, rgba.as_slice())), *blended),
                None => (*s0, *c, None, false),
            })
            .collect();
        let sections = if sections.is_empty() {
            vec![(0, mesh.indices.len() as u32, None, false)]
        } else {
            sections
        };
        Some(self.gpu.as_ref()?.upload_mesh(&rs.device, &rs.queue, &vertices, &mesh.indices, &sections))
    }

    /// Why the game shows nothing solid for this entity, if it does not.
    fn hidden_reason(&mut self, index: f64) -> Option<&'static str> {
        let look = ad2read::look::of_entity(&self.open.as_ref()?.dupe, index);
        if look.hidden.is_some() {
            return look.hidden;
        }
        let material = look.material?;
        let hides = match self.see_through.get(&material) {
            Some(known) => *known,
            None => {
                let path = format!("materials/{}.vmt", material.to_lowercase().replace('\\', "/"));
                let hides = self
                    .models
                    .as_ref()
                    .and_then(|lib| lib.read_file(&path))
                    .is_some_and(|(bytes, _)| ad2read::look::material_hides(&String::from_utf8_lossy(&bytes)));
                self.see_through.insert(material, hides);
                hides
            }
        };
        hides.then_some("its material only adds light")
    }

    /// Everything the viewport needs from an entity's modifiers: its colour
    /// tint, material override, and Proper Clipping planes.
    fn entity_look(&self, index: f64) -> (Option<Color32>, Option<String>, Vec<[f32; 4]>) {
        let Some(open) = self.open.as_ref() else { return (None, None, Vec::new()) };
        // Colour and material come from the library, which also knows the
        // material ACF gives a crate, a fuel tank or a baseplate on spawn.
        let look = ad2read::look::of_entity(&open.dupe, index);
        let colour = look.colour.map(|c| Color32::from_rgb(c[0], c[1], c[2]));
        let material = look.material;
        let Some(mods) = transform::entity_table(&open.dupe, index).and_then(|et| open.dupe.get_table(et, "EntityMods")) else {
            return (colour, material, Vec::new());
        };

        // proper_clipping: [ {norm, dist, inside, physics}, ... ] positional.
        // clips (Visual Clip Tool): [ {n, d, inside}, ... ] keyed. Prefer the
        // former; it's what Proper Clipping itself reads back.
        let mut planes = Vec::new();
        let read_planes = |list: usize, positional: bool| -> Vec<[f32; 4]> {
            let mut out = Vec::new();
            let items: Vec<usize> = match &open.dupe.arena[list] {
                Node::Array(v) => v.iter().filter_map(table_index).collect(),
                Node::Table(e) => e.iter().filter_map(|(_, v)| table_index(v)).collect(),
            };
            for t in items {
                let (n, d) = if positional {
                    let get = |i: usize| match &open.dupe.arena[t] {
                        Node::Array(v) => v.get(i).cloned(),
                        Node::Table(e) => e.get(i).map(|(_, v)| v.clone()),
                    };
                    (get(0), get(1))
                } else {
                    (open.dupe.get(t, "n").cloned(), open.dupe.get(t, "d").cloned())
                };
                if let (Some(Value::Vector(x, y, z)), Some(Value::Number(dist))) = (n, d) {
                    out.push([x as f32, y as f32, z as f32, dist as f32]);
                }
            }
            out
        };
        if let Some(l) = open.dupe.get_table(mods, "proper_clipping") {
            planes = read_planes(l, true);
        }
        if planes.is_empty() {
            if let Some(l) = open.dupe.get_table(mods, "clips") {
                planes = read_planes(l, false);
            }
        }
        (colour, material, planes)
    }

    /// Per-axis scale for an entity with a `Size` field, or 1 if it has none.
    /// The multiplier ACF applies to the entity's model on paste, from the
    /// same rules the library keeps.
    fn entity_scale(&mut self, index: f64, model: &str) -> V3 {
        let Some((b, _)) = self.models.as_mut().and_then(|m| m.bounds(model)) else {
            return (1.0, 1.0, 1.0);
        };
        let half = b.half_extents();
        match self.open.as_ref() {
            Some(open) => ad2read::scale::model_scale(&open.dupe, index, half),
            None => (1.0, 1.0, 1.0),
        }
    }

    /// Draws the scene with wgpu into a depth-tested offscreen target, then
    /// hands that texture to egui.
    ///
    /// egui's own render pass has no depth attachment, so drawing into it
    /// directly could not occlude. Rendering to our own target and registering
    /// it as a texture sidesteps that entirely, and needs no paint callback.
    fn render_meshes_gpu(&mut self, ui: &mut Ui, view: &View, rect: Rect) -> Vec<f64> {
        use ad2read::gpu;

        let Some(rs) = self.render_state.clone() else {
            return Vec::new();
        };
        let device = &rs.device;
        let queue = &rs.queue;

        if self.gpu.is_none() {
            self.gpu = Some(gpu::Renderer::new(device, queue, rs.target_format));
        }
        if self.map.is_some() && self.map_batches.is_empty() {
            // Loaded while the renderer wasn't up yet; upload now.
            self.upload_map_batches(&rs);
        }

        // Upload anything not on the GPU yet. Meshes and textures are cached
        // by model path, so a build using one prop forty times uploads once.
        let switched_off = self.hidden_entities();
        let wanted: Vec<(f64, V3, V3, String)> = {
            let (Some(open), Some(lib)) = (self.open.as_ref(), self.models.as_mut()) else { return Vec::new() };
            open.dupe
                .list_entities()
                .into_iter()
                .filter(|(i, _, _)| !switched_off.contains(&i.to_bits()))
                .filter_map(|(i, _, model)| {
                    let (p, a) = transform::entity_transform(&open.dupe, i)?;
                    // A Primitive shape is drawn with the mesh made from its numbers.
                    Some((i, p, a, lib.model_of(&open.dupe, i, model.trim_matches('"'))))
                })
                .collect()
        };
        // What the game draws nothing solid for is left to the box pass,
        // which outlines it so it can still be found.
        self.ghosts = wanted.iter().map(|(i, _, _, _)| *i).filter(|i| self.hidden_reason(*i).is_some()).collect::<Vec<_>>();
        let wanted: Vec<(f64, V3, V3, String)> = wanted.into_iter().filter(|(i, _, _, _)| !self.ghosts.contains(i)).collect();

        // Look up modifiers once per entity. The GPU mesh is keyed by
        // model plus material override, so a prop with the Material tool
        // applied gets its own upload rather than sharing the plain one.
        let looks: Vec<(f64, Option<Color32>, Option<String>, Vec<[f32; 4]>)> = wanted
            .iter()
            .map(|(i, _, _, _)| {
                let (c, m, p) = self.entity_look(*i);
                (*i, c, m, p)
            })
            .collect();
        // A mesh on the GPU is one model as one entity wears it: material,
        // skin, bodygroups and submaterials all change what is uploaded.
        let worn: Vec<ad2read::look::Look> = {
            let Some(open) = self.open.as_ref() else { return Vec::new() };
            wanted.iter().map(|(i, _, _, _)| ad2read::look::of_entity(&open.dupe, *i)).collect()
        };
        let mesh_key = |model: &str, look: &ad2read::look::Look| {
            format!("{model}|{}|{}|{:?}|{:?}", look.material.as_deref().unwrap_or(""), look.skin, look.bodygroups, look.submaterials)
        };
        let upload_started = std::time::Instant::now();
        let mut uploaded = 0;
        for ((_, _, _, model), look) in wanted.iter().zip(worn.iter()) {
            let key = mesh_key(model, look);
            if self.gpu_meshes.contains_key(&key) {
                continue;
            }
            if let Some(up) = self.upload_model(&rs, model, look) {
                self.gpu_meshes.insert(key, up);
                uploaded += 1;
            } else if !self.mesh_failures.contains(model) {
                // Say why, once per model. A cube where a gun should be is
                // this path, and the reason is the diagnosis.
                let why = self
                    .models
                    .as_ref()
                    .map(|l| l.mesh_reason(model))
                    .unwrap_or_default();
                self.bad(format!("{model}: {why}"));
                self.mesh_failures.insert(model.clone());
            }
        }
        if uploaded > 0 {
            self.say(format!(
                "loaded {uploaded} model(s) in {:.2}s",
                upload_started.elapsed().as_secs_f64()
            ));
        }

        // Build the draw list.
        let brightness = self.config.viewport.brightness.clamp(0.5, 4.0);
        let mut instances = Vec::new();
        let mut drawn = Vec::new();
        for (((index, pos, ang, model), (_, colour, _, clips)), look) in wanted.iter().zip(looks.iter()).zip(worn.iter()) {
            let key = mesh_key(model, look);
            // Cloned so the borrow on the map ends here — entity_scale below
            // needs &mut self to consult the model library.
            let Some(mesh) = self.gpu_meshes.get(&key).cloned() else {
                continue;
            };
            let selected = self.pick == Some(Pick::Entity(*index));
            // The Colour tool's colour multiplies the texture, exactly as in
            // game. Otherwise white so the texture shows untouched, or a
            // base grey for something untextured.
            // Selection never changes what a thing looks like: a selected
            // entity keeps its texture and gets an outline from the box pass.
            // Replacing the tint with a pale grey was what turned selected
            // geometry into white boxes.
            let _ = selected;
            let tint = if let Some(c) = colour {
                *c
            } else if mesh.has_texture() {
                Color32::WHITE
            } else {
                colours().box_base
            };
            // ACF's scalable entities — ammo crates, fuel tanks, armour plates —
            // are one small model stretched to a Size. base_scalable does
            // Scale = Size / OriginalSize per axis, OriginalSize being the
            // model's own bounds, so the same here. Without this a crate is a
            // 12-unit cube.
            let scale = self.entity_scale(*index, model);
            // The Colour tool's alpha fades the whole part, as in game.
            let opacity = self
                .open
                .as_ref()
                .and_then(|open| ad2read::look::of_entity(&open.dupe, *index).colour)
                .map_or(1.0, |c| c[3] as f32 / 255.0);
            let f = |v: V3| [v.0 as f32, v.1 as f32, v.2 as f32];
            instances.push(gpu::Instance {
                mesh,
                model: gpu::model_matrix(
                    f(scale3(transform::rotate_vec(*ang, (1.0, 0.0, 0.0)), scale.0)),
                    f(scale3(transform::rotate_vec(*ang, (0.0, 1.0, 0.0)), scale.1)),
                    f(scale3(transform::rotate_vec(*ang, (0.0, 0.0, 1.0)), scale.2)),
                    f(*pos),
                ),
                tint: [
                    tint.r() as f32 / 255.0 * brightness,
                    tint.g() as f32 / 255.0 * brightness,
                    tint.b() as f32 / 255.0 * brightness,
                    opacity,
                ],
                clips: clips.clone(),
            });
            drawn.push(*index);
        }
        if instances.is_empty() && self.sky.is_empty() && (self.map.is_none() || !self.show_map) {
            return drawn;
        }

        let w = (rect.width() * ui.ctx().pixels_per_point()).max(1.0) as u32;
        let h = (rect.height() * ui.ctx().pixels_per_point()).max(1.0) as u32;
        let f = |v: V3| [v.0 as f32, v.1 as f32, v.2 as f32];
        let view_proj = gpu::view_projection(
            f(view.eye),
            f(view.right),
            f(view.up),
            f(view.fwd),
            60f32.to_radians(),
            w as f32 / h as f32,
            1.0,
            100_000.0,
        );

        // --- map: PVS + frustum, then coalesced index ranges per material ---
        let frustum = ad2read::map::frustum_of(view_proj);
        let mut map_ranges: Vec<Vec<(u32, u32)>> = Vec::new();
        let mut visible_leaves: Vec<bool> = Vec::new();
        if self.show_map {
            if let Some(map) = self.map.as_ref() {
                let eye = [view.eye.0 as f32, view.eye.1 as f32, view.eye.2 as f32];
                visible_leaves = map.visible_leaves(eye, &frustum);
                let mut draws = 0;
                let mut tris = 0;
                for batch in &map.batches {
                    let r = ad2read::map::Map::draw_ranges(batch, &visible_leaves);
                    draws += r.len();
                    tris += r.iter().map(|(_, c)| *c as usize / 3).sum::<usize>();
                    map_ranges.push(r);
                }
                self.map_stats = (visible_leaves.iter().filter(|v| **v).count(), draws, tris);
            }
        }

        // Static props are ordinary instances, culled by the leaves the map
        // says they touch, sharing the model cache with the dupe's props.
        if self.show_map {
            let props: Vec<ad2read::map::StaticProp> = self
                .map
                .as_ref()
                .map(|m| m.props.clone())
                .unwrap_or_default();
            for prop in &props {
                let seen = prop.leaves.is_empty()
                    || prop
                        .leaves
                        .iter()
                        .any(|l| visible_leaves.get(*l as usize).copied().unwrap_or(true));
                if !seen {
                    continue;
                }
                if !self.gpu_meshes.contains_key(&prop.model) {
                    let Some(up) = self.upload_model(&rs, &prop.model, &ad2read::look::Look::default()) else {
                        continue;
                    };
                    self.gpu_meshes.insert(prop.model.clone(), up);
                }
                let Some(mesh) = self.gpu_meshes.get(&prop.model).cloned() else { continue };
                let ang = (prop.angles[0] as f64, prop.angles[1] as f64, prop.angles[2] as f64);
                let pos = (prop.origin[0] as f64, prop.origin[1] as f64, prop.origin[2] as f64);
                let f = |v: V3| [v.0 as f32, v.1 as f32, v.2 as f32];
                let tint = if mesh.has_texture() { Color32::WHITE } else { colours().box_base };
                instances.push(gpu::Instance {
                    mesh,
                    model: gpu::model_matrix(
                        f(transform::rotate_vec(ang, (1.0, 0.0, 0.0))),
                        f(transform::rotate_vec(ang, (0.0, 1.0, 0.0))),
                        f(transform::rotate_vec(ang, (0.0, 0.0, 1.0))),
                        f(pos),
                    ),
                    tint: [
                        tint.r() as f32 / 255.0,
                        tint.g() as f32 / 255.0,
                        tint.b() as f32 / 255.0,
                        1.0,
                    ],
                    clips: Vec::new(),
                });
            }
        }

        if self.show_map && !self.sky.is_empty() {
            // Half the far plane out. Anything real is nearer and wins the
            // depth test, so the sky only shows through gaps.
            let s = 40_000.0f32;
            let e = [view.eye.0 as f32, view.eye.1 as f32, view.eye.2 as f32];
            for mesh in &self.sky {
                instances.insert(
                    0,
                    gpu::Instance {
                        mesh: mesh.clone(),
                        model: gpu::model_matrix([s, 0.0, 0.0], [0.0, s, 0.0], [0.0, 0.0, s], e),
                        tint: [1.0, 1.0, 1.0, 1.0],
                        clips: Vec::new(),
                    },
                );
            }
        }

        let map_draws: Vec<gpu::MapDraw<'_>> = self
            .map_batches
            .iter()
            .zip(map_ranges.iter())
            .filter(|(_, r)| !r.is_empty())
            .map(|(b, r)| gpu::MapDraw { batch: b, ranges: r })
            .collect();

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ad2edit") });
        self.gpu.as_mut().unwrap().render(
            device,
            queue,
            &mut encoder,
            w,
            h,
            view_proj,
            [
                colours().background.r() as f64 / 255.0,
                colours().background.g() as f64 / 255.0,
                colours().background.b() as f64 / 255.0,
            ],
            self.config.viewport.fullbright,
            self.map.as_ref().map(|m| m.lit).unwrap_or(false),
            &instances,
            &map_draws,
        );
        queue.submit(Some(encoder.finish()));

        // Register the result with egui — but only when the target was
        // recreated (a resize), since the view is stable otherwise. Doing it
        // every frame took egui's renderer write-lock twice per frame and
        // built a fresh bind group each time for nothing.
        if let Some(colour) = self.gpu.as_ref().and_then(|g| g.colour_view()) {
            if self.scene_size != (w, h) || self.scene_tex.is_none() {
                let id = rs.renderer.write().register_native_texture(
                    device,
                    colour,
                    wgpu::FilterMode::Linear,
                );
                if let Some(old) = self.scene_tex.replace(id) {
                    if old != id {
                        rs.renderer.write().free_texture(&old);
                    }
                }
                self.scene_size = (w, h);
            }
            if let Some(id) = self.scene_tex {
                ui.painter_at(rect).image(
                    id,
                    rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        drawn
    }

    /// Rasterises every entity whose model resolves, and blits the result into
    /// the viewport as a single texture. Returns the entities it drew, so the
    /// box pass can skip them.
    fn render_meshes(
        &mut self,
        ui: &mut Ui,
        view: &View,
        rect: Rect,
    ) -> Vec<f64> {
        use ad2read::raster::{triangle, ScreenVertex};

        let detail = self.config.viewport.detail;
        let w = ((rect.width() * detail) as usize).clamp(16, 4096);
        let h = ((rect.height() * detail) as usize).clamp(16, 4096);
        self.target.resize(w, h);
        let bg = colours().background;
        self.target.clear([bg.r(), bg.g(), bg.b(), 255]);

        // The rasteriser works in its own pixel space, so projected screen
        // coordinates get scaled into it.
        let sx = w as f32 / rect.width();
        let sy = h as f32 / rect.height();
        let light = norm3((0.35, 0.5, 1.0));

        let entities: Vec<(f64, V3, V3, String)> = {
            let Some(open) = self.open.as_ref() else { return Vec::new() };
            open.dupe
                .list_entities()
                .into_iter()
                .filter_map(|(i, _, model)| {
                    let (p, a) = transform::entity_transform(&open.dupe, i)?;
                    Some((i, p, a, model.trim_matches('"').to_owned()))
                })
                .collect()
        };

        let mut drawn = Vec::new();
        let mut budget: usize = 400_000; // pixels, so one huge prop can't stall a frame
        self.ghosts = entities.iter().map(|(i, _, _, _)| *i).filter(|i| self.hidden_reason(*i).is_some()).collect::<Vec<_>>();

        for (index, pos, ang, model) in entities {
            if self.ghosts.contains(&index) {
                continue;
            }
            let Some(mesh) = self.models.as_mut().and_then(|m| m.mesh(&model)) else {
                continue;
            };
            let scale = self.entity_scale(index, &model);
            let selected = self.pick == Some(Pick::Entity(index));
            let marked = self.marked.contains(&index);
            let base = if selected {
                colours().box_selected
            } else if marked {
                colours().box_marked
            } else {
                colours().box_base
            };

            let mut any = false;
            for tri in mesh.indices.chunks_exact(3) {
                if budget == 0 {
                    break;
                }
                let mut screen = [ScreenVertex { x: 0.0, y: 0.0, inv_z: 0.0 }; 3];
                let mut world = [(0.0, 0.0, 0.0); 3];
                let mut ok = true;

                for (k, &idx) in tri.iter().enumerate() {
                    let p = mesh.positions[idx as usize];
                    let local = (p[0] as f64 * scale.0, p[1] as f64 * scale.1, p[2] as f64 * scale.2);
                    let wp = add3(pos, transform::rotate_vec(ang, local));
                    world[k] = wp;
                    match view.project(wp) {
                        Some((sp, depth)) => {
                            screen[k] = ScreenVertex {
                                x: (sp.x - rect.left()) * sx,
                                y: (sp.y - rect.top()) * sy,
                                // Reciprocal depth interpolates linearly in
                                // screen space where depth itself does not.
                                inv_z: 1.0 / depth.max(0.001),
                            };
                        }
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    continue;
                }

                // Face normal from the world-space triangle, so shading
                // follows the entity's rotation without needing the model's
                // own normals transformed.
                let n = norm3(cross3(sub3(world[1], world[0]), sub3(world[2], world[0])));
                if dot3(n, sub3(world[0], view.eye)) > 0.0 {
                    continue;
                }
                let lit = if self.config.viewport.fullbright {
                    1.0
                } else {
                    0.35 + 0.65 * dot3(n, light).abs() as f32
                };
                let rgba = [
                    (base.r() as f32 * lit) as u8,
                    (base.g() as f32 * lit) as u8,
                    (base.b() as f32 * lit) as u8,
                    255,
                ];
                let written = triangle(&mut self.target, screen, rgba);
                budget = budget.saturating_sub(written);
                any |= written > 0;
            }
            if any {
                drawn.push(index);
            }
        }

        if drawn.is_empty() {
            return drawn;
        }

        let image = egui::ColorImage::from_rgba_unmultiplied(
            [self.target.width, self.target.height],
            &self.target.colour,
        );
        match self.frame_tex.as_mut() {
            Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.frame_tex = Some(ui.ctx().load_texture(
                    "ad2edit_viewport",
                    image,
                    egui::TextureOptions::LINEAR,
                ))
            }
        }
        if let Some(tex) = self.frame_tex.as_ref() {
            ui.painter_at(rect).image(
                tex.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        drawn
    }

    /// Indexes the game folder so the viewport can draw props at their real
    /// size. A dupe carries only a model path, so this is the only source of
    /// dimensions there is.
    fn index_models(&mut self, dir: std::path::PathBuf) {
        self.say(format!("indexing {}...", dir.display()));
        let started = std::time::Instant::now();
        let lib = ad2read::models::ModelLibrary::new(&dir);
        self.say(format!("indexed in {:.2}s", started.elapsed().as_secs_f64()));
        self.good(format!(
            "scanned {} .gma archives ({} models), {} vpk archives",
            lib.scanned_gmas,
            lib.indexed_models(),
            lib.archives()
        ));
        self.config.garrysmod = dir.display().to_string();
        self.models = Some(lib);

        // Anything that can't be resolved keeps the uniform box, so say how
        // many rather than letting it look like nothing happened.
        let mut missing = 0;
        let mut total = 0;
        if let (Some(open), Some(lib)) = (self.open.as_ref(), self.models.as_mut()) {
            for (_, _, model) in open.dupe.list_entities() {
                total += 1;
                if lib.bounds(model.trim_matches('"')).is_none() {
                    missing += 1;
                }
            }
        }
        if missing == total && total > 0 {
            // Nothing at all resolved, which almost always means the search
            // never reached the archives rather than the models being absent.
            let detail = self
                .models
                .as_ref()
                .map(|l| l.describe_search())
                .unwrap_or_default();
            self.bad("no models resolved at all; here's what was searched:");
            for line in detail.lines() {
                self.bad(format!("  {line}"));
            }
        } else if missing > 0 {
            self.say(format!(
                "{missing} of {total} entities couldn't be resolved; those keep \
                 a placeholder box"
            ));
        }
    }

    fn op_delete(&mut self, targets: &[f64]) {
        self.snapshot();
        let (mut ents, mut cons, mut pruned) = (0usize, 0usize, 0usize);
        let mut errors: Vec<String> = Vec::new();
        for t in targets {
            let Some(open) = self.open.as_mut() else { return };
            match open.dupe.delete_entity(*t) {
                Ok(r) => {
                    ents += 1;
                    cons += r.removed_constraints;
                    pruned += r.pruned.total();
                }
                Err(e) => errors.push(e),
            }
        }
        for e in errors {
            self.bad(e);
        }
        self.marked.clear();
        self.pick = None;
        self.crumbs.clear();
        self.good(format!(
            "deleted {ents} entities and {cons} constraints, pruned {pruned} references"
        ));
        self.check_dangling();
    }

    /// A reflected copy of the selection on the other side of the hull's
    /// centreline. With `mirror_props_only` it leaves everything but plain
    /// props and Primitive shapes alone: a second gun or gunner is rarely
    /// what was meant.
    fn op_mirror(&mut self, targets: &[f64]) {
        let Some(open) = self.open.as_ref() else { return };
        let Some(base) = ad2read::roles::of_dupe(&open.dupe).into_iter().find(|(_, role)| *role == ad2read::roles::Role::Hull).map(|(index, _)| index) else {
            return self.bad("mirroring needs a baseplate: its middle is the mirror");
        };
        let Some((at, ang)) = transform::entity_transform(&open.dupe, base) else { return };
        let across = transform::rotate_vec(ang, (0.0, 1.0, 0.0));
        let props_only = self.mirror_props_only;
        let wanted: Vec<f64> = targets
            .iter()
            .copied()
            .filter(|index| {
                let class = transform::entity_table(&open.dupe, *index).and_then(|et| match open.dupe.get(et, "Class") {
                    Some(Value::Str(bytes)) => Some(String::from_utf8_lossy(bytes).into_owned()),
                    _ => None,
                });
                *index != base && (!props_only || class.is_some_and(|class| class == "prop_physics" || class.starts_with("primitive_")))
            })
            .collect();
        if wanted.is_empty() {
            return self.bad("nothing selected is a plain prop; untick props only to mirror ACF parts too");
        }
        self.snapshot();
        let result = self.open.as_mut().map(|open| ad2read::duplicate::mirror(&mut open.dupe, &wanted, at, across));
        match result {
            Some(Ok((report, on_the_line))) => {
                let left_out = targets.len() - wanted.len();
                self.good(format!(
                    "mirrored {} part(s) across the hull{}{}",
                    report.entities_added,
                    if on_the_line > 0 { format!("; {on_the_line} on the centreline left as they are") } else { String::new() },
                    if left_out > 0 { format!("; {left_out} that are not plain props left alone") } else { String::new() }
                ));
            }
            Some(Err(e)) => self.bad(e),
            None => {}
        }
        self.check_dangling();
    }

    fn op_duplicate(&mut self, targets: &[f64]) {
        self.snapshot();
        let offset = self.op_offset;
        let copies = self.array_count.max(1);
        let result = {
            let Some(open) = self.open.as_mut() else { return };
            if copies > 1 { ad2read::duplicate::array(&mut open.dupe, targets, offset, copies) } else { ad2read::duplicate::duplicate(&mut open.dupe, targets, offset) }
        };
        match result {
            Ok(r) => {
                self.good(format!(
                    "copied {} entities (numbered from {}) and {} internal constraint(s)",
                    r.entities_added, r.first_new_index, r.constraints_added
                ));
                if r.constraints_added == 0 && r.entities_added > 1 {
                    self.say("no constraint had all its endpoints inside the selection");
                }
                if offset == (0.0, 0.0, 0.0) {
                    self.say("offset is zero, so the copy is sitting inside the original");
                }
            }
            Err(e) => self.bad(e),
        }
        self.check_dangling();
    }

    fn op_merge(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("AdvDupe2", &["txt"])
            .pick_file()
        else {
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return self.bad(format!("could not read: {e}")),
        };
        let other = match dupefile::parse(&bytes)
            .and_then(|f| dupefile::decompress(&f.compressed))
            .and_then(|raw| Dupe::from_body(&raw).map(|(d, _)| d))
        {
            Ok(d) => d,
            Err(e) => return self.bad(format!("could not open that dupe: {e}")),
        };

        self.snapshot();
        let offset = self.op_offset;
        let result = {
            let Some(open) = self.open.as_mut() else { return };
            ad2read::merge::merge(&mut open.dupe, other, offset)
        };
        match result {
            Ok(r) => self.good(format!(
                "merged in {} entities (from {}) and {} constraints, rewrote {} references",
                r.entities_added, r.first_new_index, r.constraints_added, r.remapped
            )),
            Err(e) => self.bad(e),
        }
        self.check_dangling();
    }

    fn op_bulk(&mut self) {
        self.snapshot();
        let filter = if self.where_key.is_empty() {
            None
        } else {
            Some((self.where_key.clone(), self.where_val.clone()))
        };
        let sets = vec![(self.set_key.clone(), self.set_val.clone())];
        let result = {
            let Some(open) = self.open.as_mut() else { return };
            let f = filter.as_ref().map(|(k, v)| (k.as_str(), v.as_str()));
            ad2read::bulk::bulk_set(&mut open.dupe, f, &sets)
        };
        match result {
            Ok(r) => {
                self.good(format!("{} matched, {} field(s) written", r.matched, r.changed));
                if r.skipped_missing > 0 {
                    self.say(format!(
                        "{} skipped: that key isn't already on the entity, and adding one \
                         would change how it pastes",
                        r.skipped_missing
                    ));
                }
                if r.skipped_type > 0 {
                    self.say(format!(
                        "{} skipped: the value didn't parse as the type that key holds",
                        r.skipped_type
                    ));
                }
            }
            Err(e) => self.bad(e),
        }
    }

    /// Every operation ends here. A dangling reference means the edit left the
    /// dupe naming an entity that no longer exists, which is the one kind of
    /// damage that pastes without complaint.
    fn check_dangling(&mut self) {
        let left = {
            let Some(open) = self.open.as_ref() else { return };
            ad2read::refs::dangling(&open.dupe)
        };
        if left.is_empty() {
            return;
        }
        self.bad(format!("{} dangling reference(s):", left.len()));
        for h in left.iter().take(8) {
            self.bad(format!("  {} = {}", h.path, h.value));
        }
    }
}

/// A tree row with SFM's selection fill, hairline separator and 6%-alpha
/// alternate shade. Full width, fixed height, like QTreeView's.
/// A small highlighter for E2 and Lua: comments, strings, @directives,
/// numbers, keywords. One LayoutJob per frame; fine at chip sizes.
fn sfm_row(ui: &mut egui::Ui, label: &str, selected: bool, alternate: bool) -> egui::Response {
    // 17px is CQPropertyEditorTreeView QLabel's min/max-height.
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 17.0), egui::Sense::click());

    if ui.is_rect_visible(rect) {
        tree_row(ui.painter(), rect, selected, alternate);
        let color = if selected { colours().selection_text } else { colours().text };
        ui.painter().text(
            egui::pos2(rect.left() + 4.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::monospace(11.0),
            color,
        );
    }
    response
}

/// QHeaderView::section:horizontal — dark vertical gradient, light label.
fn sfm_header(ui: &mut egui::Ui, label: &str) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        header_bar(ui.painter(), rect);
        ui.painter().text(
            egui::pos2(rect.left() + 5.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(11.0),
            colours().heading,
        );
    }
}

/// `--console`: mirror every log line to the terminal, print every warning
/// and error the UI stack emits (eframe, egui, wgpu, winit — trace level),
/// and print panics with a backtrace. On Windows a console window is
/// allocated so this works from a double-click, not only from a terminal.
static VERBOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn verbose() -> bool {
    VERBOSE.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(windows)]
fn alloc_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AllocConsole() -> i32;
    }
    // Safe: no arguments, no invariants; failure just means a console was
    // already attached.
    unsafe {
        AllocConsole();
    }
}
#[cfg(not(windows))]
fn alloc_console() {}

fn enable_console() {
    VERBOSE.store(true, std::sync::atomic::Ordering::Relaxed);
    alloc_console();
    // Everything, from everyone. RUST_LOG still overrides if set.
    let mut b = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("trace"));
    b.format_timestamp_millis();
    let _ = b.try_init();
    std::panic::set_hook(Box::new(|info| {
        eprintln!("\n===== PANIC =====\n{info}\n{}", std::backtrace::Backtrace::force_capture());
    }));
    println!("ad2edit --console: everything goes here. RUST_LOG=<level> narrows it.");
}

/// A picture of the whole window, taken `at` and written to `path`, after
/// which the program exits: the GUI's counterpart to `ad2read --render`.
struct Screenshot {
    path: PathBuf,
    at: std::time::Instant,
    asked: bool,
}

impl App {
    fn screenshot_tick(&mut self, ui: &Ui) {
        let Some(shot) = self.screenshot.as_mut() else { return };
        ui.ctx().request_repaint();
        if !shot.asked && std::time::Instant::now() >= shot.at {
            shot.asked = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let taken = ui.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = taken {
            let rgba = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
            let picture = ad2read::render::Picture { width: image.size[0], height: image.size[1], rgba };
            if let Err(e) = std::fs::write(&shot.path, ad2read::render::png(&picture)) {
                eprintln!("{}: {e}", shot.path.display());
            }
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--console" || a == "-v" || a == "--verbose") {
        enable_console();
    }
    // --screenshot <out.png> [seconds], --select <entity>, --edit-chip <entity>,
    // --add <search>, --handle move|rotate|scale, --hide <role>, --settings,
    // --newcomer, --advanced;
    // anything else without a dash is the dupe to open.
    let mut file_arg: Option<String> = None;
    let mut screenshot: Option<(PathBuf, f64)> = None;
    let (mut select, mut edit_chip, mut mode): (Option<f64>, Option<f64>, Option<Mode>) = (None, None, None);
    let mut add_search: Option<String> = None;
    let mut handle: Option<Gizmo> = None;
    let mut hide: Vec<String> = Vec::new();
    let mut show_settings = false;
    let mut at = 0;
    while at < args.len() {
        let number_after = |at: usize| args.get(at + 1).and_then(|next| next.parse::<f64>().ok());
        match args[at].as_str() {
            "--screenshot" => {
                if let Some(out) = args.get(at + 1) {
                    let seconds = number_after(at + 1);
                    screenshot = Some((PathBuf::from(out), seconds.unwrap_or(3.0)));
                    at += 1 + seconds.is_some() as usize;
                }
            }
            "--select" => {
                select = number_after(at);
                at += 1;
            }
            "--edit-chip" => {
                edit_chip = number_after(at);
                at += 1;
            }
            "--add" => {
                add_search = Some(args.get(at + 1).cloned().unwrap_or_default());
                at += 1;
            }
            "--settings" => show_settings = true,
            "--hide" => {
                hide.extend(args.get(at + 1).cloned());
                at += 1;
            }
            "--handle" => {
                handle = match args.get(at + 1).map(String::as_str) {
                    Some("rotate") => Some(Gizmo::Rotate),
                    Some("scale") => Some(Gizmo::Scale),
                    _ => Some(Gizmo::Move),
                };
                at += 1;
            }
            "--newcomer" => mode = Some(Mode::Newcomer),
            "--advanced" => mode = Some(Mode::Advanced),
            other if !other.starts_with('-') && file_arg.is_none() => file_arg = Some(other.to_owned()),
            _ => {}
        }
        at += 1;
    }

    // The window's icon: assets/icon/icon.html drawn at 256 pixels.
    let icon = png::Decoder::new(std::io::Cursor::new(&include_bytes!("../../assets/icon/icon-256.png")[..])).read_info().ok().and_then(|mut reader| {
        let mut rgba = vec![0; reader.output_buffer_size()?];
        let frame = reader.next_frame(&mut rgba).ok()?;
        rgba.truncate(frame.buffer_size());
        (frame.color_type == png::ColorType::Rgba).then_some(egui::IconData { rgba, width: frame.width, height: frame.height })
    });
    let viewport = egui::ViewportBuilder::default().with_title("ad2edit").with_inner_size([1180.0, 760.0]);
    let options = eframe::NativeOptions {
        viewport: match icon {
            Some(icon) => viewport.with_icon(icon),
            None => viewport,
        },
        ..Default::default()
    };

    let loaded = config::load();

    eframe::run_native(
        "ad2edit",
        options,
        Box::new(move |cc| {
            let mut app = App::default();
            let mut notes: Vec<String> = Vec::new();
            match loaded {
                config::LoadResult::Ok(c) => app.config = c,
                config::LoadResult::FirstRun => app.setup = Some(Setup::new(&app.config)),
                config::LoadResult::Broken { path, moved_to, error } => {
                    notes.push(format!("settings file {}: {error}", path.display()));
                    if let Some(bad) = moved_to {
                        notes.push(format!("moved it to {}; starting setup again", bad.display()));
                    }
                    app.setup = Some(Setup::new(&app.config));
                }
            }
            app.apply_config(&cc.egui_ctx);
            app.say("ad2edit: pick a dupe, or start a new one");
            for line in notes {
                app.bad(line);
            }
            // Anything passed on the command line opens straight away.
            if let Some(arg) = file_arg {
                app.load(PathBuf::from(arg));
            }
            if let Some(mode) = mode {
                app.config.mode = mode;
            }
            if let Some(index) = select {
                app.pick = Some(Pick::Entity(index));
            }
            if let Some(index) = edit_chip {
                app.open_chip_editor(index);
            }
            if let Some(handle) = handle {
                app.gizmo = handle;
            }
            app.show_settings |= show_settings;
            for name in &hide {
                let roles = app.open.as_ref().map(|open| ad2read::roles::of_dupe(&open.dupe)).unwrap_or_default();
                match roles.into_iter().find(|(_, role)| role.name() == name) {
                    Some((_, role)) => app.show_role(role.name(), false),
                    None => app.bad(format!("--hide {name}: nothing in the open dupe has that role")),
                }
            }
            if let Some(query) = add_search {
                app.add_panel.open_searching(&query);
            }
            if let Some((path, seconds)) = screenshot {
                app.setup = None;
                app.screenshot = Some(Screenshot { path, at: std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds), asked: false });
            }
            Ok(Box::new(app))
        }),
    )
}

// ---------------------------------------------------------------------------
// Viewport
// ---------------------------------------------------------------------------
//
// Deliberately software-rendered. A wgpu paint callback would mean shaders,
// pipelines and buffer plumbing for what is, at this stage, a few hundred
// boxes — egui's 2D painter handles that volume without any of it. When
// Phase 3 loads real model meshes and the triangle count goes up by orders of
// magnitude, that is when a GPU path earns its keep.
//
// Source is Z-up, so the camera orbits around Z and "up" is (0, 0, 1).

type V3 = (f64, f64, f64);

fn sub3(a: V3, b: V3) -> V3 {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}
fn add3(a: V3, b: V3) -> V3 {
    (a.0 + b.0, a.1 + b.1, a.2 + b.2)
}
fn scale3(a: V3, k: f64) -> V3 {
    (a.0 * k, a.1 * k, a.2 * k)
}
fn dot3(a: V3, b: V3) -> f64 {
    a.0 * b.0 + a.1 * b.1 + a.2 * b.2
}
fn cross3(a: V3, b: V3) -> V3 {
    (
        a.1 * b.2 - a.2 * b.1,
        a.2 * b.0 - a.0 * b.2,
        a.0 * b.1 - a.1 * b.0,
    )
}
fn norm3(a: V3) -> V3 {
    let l = dot3(a, a).sqrt();
    if l > 1e-9 {
        scale3(a, 1.0 / l)
    } else {
        (0.0, 0.0, 1.0)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Gizmo {
    Move,
    Rotate,
    /// For the parts ACF sizes: a baseplate, a crate, a fuel tank.
    Scale,
}

/// A first-person camera: an eye position and where it's looking.
///
/// The earlier one orbited a target, which is fine for inspecting one object
/// and wrong for a building tool — you want to stand in the build and look
/// around it, the way you would in-game. Movement is along the view axes at a
/// speed you set, not scaled by distance to anything.
struct Camera {
    eye: V3,
    yaw: f64,
    pitch: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            eye: (-300.0, -300.0, 150.0),
            yaw: 0.8,
            pitch: -0.3,
        }
    }
}

impl Camera {
    fn forward(&self) -> V3 {
        (
            self.pitch.cos() * self.yaw.cos(),
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
        )
    }

    /// Places the eye so that a sphere of `radius` around `centre` fills the
    /// view, looking at it from the current direction.
    fn look_at(&mut self, centre: V3, radius: f64) {
        let dist = (radius / (30f64.to_radians()).tan()).max(48.0) + 32.0;
        self.eye = sub3(centre, scale3(self.forward(), dist));
    }
}

/// Camera basis plus screen mapping, computed once per frame.
struct View {
    eye: V3,
    right: V3,
    up: V3,
    fwd: V3,
    centre: egui::Pos2,
    focal: f32,
}

impl Camera {
    fn view(&self, rect: Rect) -> View {
        let eye = self.eye;
        let fwd = self.forward();
        let right = norm3(cross3(fwd, (0.0, 0.0, 1.0)));
        let up = cross3(right, fwd);
        // 60-degree vertical field of view.
        let focal = (rect.height() * 0.5) / (60.0f32.to_radians() * 0.5).tan();
        View {
            eye,
            right,
            up,
            fwd,
            centre: rect.center(),
            focal,
        }
    }
}

impl View {
    /// World point to screen, plus depth. None when behind the near plane.
    fn project(&self, p: V3) -> Option<(egui::Pos2, f32)> {
        let rel = sub3(p, self.eye);
        let z = dot3(rel, self.fwd);
        if z < 1.0 {
            return None;
        }
        let x = dot3(rel, self.right) as f32;
        let y = dot3(rel, self.up) as f32;
        let k = self.focal / z as f32;
        Some((pos2(self.centre.x + x * k, self.centre.y - y * k), z as f32))
    }

    /// The direction from the eye through a point on the screen.
    fn ray(&self, at: egui::Pos2) -> V3 {
        add3(
            scale3(self.fwd, self.focal as f64),
            add3(
                scale3(self.right, (at.x - self.centre.x) as f64),
                scale3(self.up, (self.centre.y - at.y) as f64),
            ),
        )
    }
}

/// Eight corners of a cube of half-extent `h`, rotated by the entity's angle
/// and placed at its origin.
fn box_corners(pos: V3, ang: V3, h: V3) -> [V3; 8] {
    let mut out = [(0.0, 0.0, 0.0); 8];
    for (i, slot) in out.iter_mut().enumerate() {
        let local = (
            if i & 1 == 0 { -h.0 } else { h.0 },
            if i & 2 == 0 { -h.1 } else { h.1 },
            if i & 4 == 0 { -h.2 } else { h.2 },
        );
        *slot = add3(pos, transform::rotate_vec(ang, local));
    }
    out
}

/// Corner indices per face, wound so the cross product points outward.
const FACES: [[usize; 4]; 6] = [
    [0, 4, 6, 2], // -x
    [1, 3, 7, 5], // +x
    [0, 1, 5, 4], // -y
    [2, 6, 7, 3], // +y
    [0, 2, 3, 1], // -z
    [4, 5, 7, 6], // +z
];

/// Source 2D skybox faces. From the VDC and a practical rotation guide:
/// `rt` is east (+X), `bk` is north (+Y), and the rest follow. Each entry is
/// (suffix, outward axis, screen-left-to-right direction, screen-top-to-bottom
/// direction) as seen from inside the box. The image orientation of `up` and
/// `dn` is the least certain of these; if the top looks rotated, those two
/// tangent pairs are what to turn.
const SKY_FACES: &[(&str, V3, V3, V3)] = &[
    ("rt", (1.0, 0.0, 0.0), (0.0, -1.0, 0.0), (0.0, 0.0, -1.0)),
    ("lf", (-1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, -1.0)),
    ("bk", (0.0, 1.0, 0.0), (1.0, 0.0, 0.0), (0.0, 0.0, -1.0)),
    ("ft", (0.0, -1.0, 0.0), (-1.0, 0.0, 0.0), (0.0, 0.0, -1.0)),
    ("up", (0.0, 0.0, 1.0), (0.0, -1.0, 0.0), (1.0, 0.0, 0.0)),
    ("dn", (0.0, 0.0, -1.0), (0.0, -1.0, 0.0), (-1.0, 0.0, 0.0)),
];

/// A unit quad on one face of the sky cube, in the model's own space; the
/// instance matrix scales and positions it around the eye.
fn sky_quad(axis: V3, u_dir: V3, v_dir: V3) -> Vec<ad2read::gpu::Vertex> {
    let f = |v: V3| [v.0 as f32, v.1 as f32, v.2 as f32];
    let corner = |su: f64, sv: f64| add3(axis, add3(scale3(u_dir, su), scale3(v_dir, sv)));
    let n = f(scale3(axis, -1.0));
    vec![
        ad2read::gpu::Vertex { position: f(corner(-1.0, -1.0)), normal: n, uv: [0.0, 0.0], light: ad2read::gpu::Vertex::UNLIT },
        ad2read::gpu::Vertex { position: f(corner(1.0, -1.0)), normal: n, uv: [1.0, 0.0], light: ad2read::gpu::Vertex::UNLIT },
        ad2read::gpu::Vertex { position: f(corner(1.0, 1.0)), normal: n, uv: [1.0, 1.0], light: ad2read::gpu::Vertex::UNLIT },
        ad2read::gpu::Vertex { position: f(corner(-1.0, 1.0)), normal: n, uv: [0.0, 1.0], light: ad2read::gpu::Vertex::UNLIT },
    ]
}

/// One entity's worth of drawable faces, kept together so the two-level sort
/// below can order whole boxes before it orders faces.
struct Drawn {
    /// depth, polygon, fill, and whether to outline it — only the selected
    /// entity is outlined, so no two boxes ever fight over the same edge.
    faces: Vec<(f32, Vec<egui::Pos2>, Color32, bool)>,
    /// Bounding-box edges drawn over real geometry as the selection
    /// highlight: the texture stays, the box is a wire frame only.
    edges: Vec<([egui::Pos2; 2], Color32)>,
}

/// The 12 edges of the corner cube whose index bits are x=1, y=2, z=4.
const BOX_EDGES: [(usize, usize); 12] = [
    (0, 1), (2, 3), (4, 5), (6, 7),
    (0, 2), (1, 3), (4, 6), (5, 7),
    (0, 4), (1, 5), (2, 6), (3, 7),
];

impl App {
    /// Frames the whole build, so opening a dupe doesn't leave you staring at
    /// empty space wondering where it went.
    fn frame_camera(&mut self) {
        let Some(open) = self.open.as_ref() else { return };
        let mut lo = (f64::MAX, f64::MAX, f64::MAX);
        let mut hi = (f64::MIN, f64::MIN, f64::MIN);
        let mut any = false;
        for (i, _, _) in open.dupe.list_entities() {
            if let Some((p, _)) = transform::entity_transform(&open.dupe, i) {
                lo = (lo.0.min(p.0), lo.1.min(p.1), lo.2.min(p.2));
                hi = (hi.0.max(p.0), hi.1.max(p.1), hi.2.max(p.2));
                any = true;
            }
        }
        if !any {
            return;
        }
        let span = sub3(hi, lo);
        self.camera
            .look_at(scale3(add3(lo, hi), 0.5), dot3(span, span).sqrt() * 0.5 + 64.0);
    }

    /// Entities sharing a constraint with any currently selected one.
    fn linked_to_selection(&self) -> Vec<f64> {
        let Some(open) = self.open.as_ref() else {
            return Vec::new();
        };
        let seeds = self.targets();
        let mut out = seeds.clone();
        for ct in constraint_list(&open.dupe) {
            let Some(list) = open.dupe.get_table(ct, "Entity") else {
                continue;
            };
            let Node::Array(items) = &open.dupe.arena[list] else {
                continue;
            };
            let ends: Vec<f64> = items
                .iter()
                .filter_map(|it| open.dupe.get_number(table_index(it)?, "Index"))
                .collect();
            if ends.iter().any(|e| seeds.contains(e)) {
                for e in ends {
                    if !out.contains(&e) {
                        out.push(e);
                    }
                }
            }
        }
        out
    }

    /// Sets or clears BuildDupeInfo.DupeParentID on each child. Creates the
    /// BuildDupeInfo table if the entity has none — a freshly spawned prop
    /// has one, but a merged-in one may not.
    fn set_parent(&mut self, children: &[f64], parent: Option<f64>) {
        self.snapshot();
        let mut n = 0;
        for child in children {
            let Some(open) = self.open.as_mut() else { return };
            let Some(et) = transform::entity_table(&open.dupe, *child) else { continue };
            let bdi = match open.dupe.get_table(et, "BuildDupeInfo") {
                Some(b) => b,
                None => {
                    let b = open.dupe.new_table();
                    open.dupe.set(et, "BuildDupeInfo", Value::Table(b));
                    b
                }
            };
            match parent {
                Some(p) => open.dupe.set(bdi, "DupeParentID", Value::Number(p)),
                None => {
                    if let Node::Table(entries) = &mut open.dupe.arena[bdi] {
                        entries.retain(|(k, _)| key_label(k) != "DupeParentID");
                    }
                }
            }
            open.dirty = true;
            n += 1;
        }
        match parent {
            Some(p) => self.good(format!("parented {n} entities to {p}")),
            None => self.good(format!("unparented {n} entities")),
        }
    }

    /// 26: an entity's Proper Clipping planes as (normal, dist) in its own frame.
    fn clip_planes(&self, index: f64) -> Vec<(V3, f64)> {
        self.entity_look(index).2.iter().map(|c| ((c[0] as f64, c[1] as f64, c[2] as f64), c[3] as f64)).collect()
    }

    /// 26: writes the whole plane list back as `proper_clipping`, the
    /// positional {norm, dist, inside, physics} arrays the addon reads.
    fn set_clip_planes(&mut self, index: f64, planes: &[(V3, f64)]) {
        self.snapshot();
        let Some(open) = self.open.as_mut() else { return };
        let Some(et) = transform::entity_table(&open.dupe, index) else { return };
        let mods = match open.dupe.get_table(et, "EntityMods") {
            Some(m) => m,
            None => {
                let m = open.dupe.new_table();
                open.dupe.set(et, "EntityMods", Value::Table(m));
                m
            }
        };
        if planes.is_empty() {
            if let Node::Table(e) = &mut open.dupe.arena[mods] {
                e.retain(|(k, _)| key_label(k) != "proper_clipping");
            }
        } else {
            let list = open.dupe.new_array();
            for (n, d) in planes {
                let c = open.dupe.new_array();
                if let Node::Array(items) = &mut open.dupe.arena[c] {
                    items.push(Value::Vector(n.0, n.1, n.2));
                    items.push(Value::Number(*d));
                    items.push(Value::Bool(false));
                    items.push(Value::Bool(true));
                }
                if let Node::Array(items) = &mut open.dupe.arena[list] {
                    items.push(Value::Table(c));
                }
            }
            open.dupe.set(mods, "proper_clipping", Value::Table(list));
        }
        open.dirty = true;
    }

    /// 26: the clip section in the inspector — numeric planes, add, flip,
    /// remove. The gizmo in the viewport drags the distance.
    fn clip_editor(&mut self, ui: &mut Ui, index: f64) {
        let mut planes = self.clip_planes(index);
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("CLIPS ({})", planes.len())).color(colours().accent).small());
            if ui.small_button("+ plane").on_hover_text("a new plane through the origin, keeping +Z").clicked() {
                planes.push(((0.0, 0.0, 1.0), 0.0));
                changed = true;
            }
        });
        let mut remove: Option<usize> = None;
        for (i, (n, d)) in planes.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{i}")).color(colours().text_dim));
                for (k, c) in [("nx", &mut n.0), ("ny", &mut n.1), ("nz", &mut n.2)] {
                    ui.label(RichText::new(k).color(colours().text_dim).small());
                    if ui.add(egui::DragValue::new(c).speed(0.01).range(-1.0..=1.0)).changed() {
                        changed = true;
                    }
                }
                ui.label(RichText::new("d").color(colours().text_dim).small());
                if ui.add(egui::DragValue::new(d).speed(0.5)).changed() {
                    changed = true;
                }
                if ui.small_button("flip").on_hover_text("keep the other side").clicked() {
                    *n = (-n.0, -n.1, -n.2);
                    *d = -*d;
                    changed = true;
                }
                if ui.small_button("×").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            planes.remove(i);
            changed = true;
        }
        if changed {
            // Normals are kept unit length; the addon rotates them as directions.
            for (n, _) in planes.iter_mut() {
                let l = (n.0 * n.0 + n.1 * n.1 + n.2 * n.2).sqrt();
                if l > 1e-6 {
                    *n = (n.0 / l, n.1 / l, n.2 / l);
                }
            }
            self.set_clip_planes(index, &planes);
        }
    }

    /// 26: each plane of the selected entity as a square outline at its
    /// distance along the normal, with a handle you drag along the normal
    /// to move the plane. Geometry kept is on the normal's side.
    fn draw_clip_gizmos(&mut self, ui: &mut Ui, p: &egui::Painter, rect: Rect, _resp: &egui::Response) {
        let Some(Pick::Entity(index)) = self.pick.clone() else { return };
        let planes = self.clip_planes(index);
        if planes.is_empty() {
            return;
        }
        let Some(open) = self.open.as_ref() else { return };
        let Some((pos, ang)) = transform::entity_transform(&open.dupe, index) else { return };
        let view = self.camera.view(rect);
        let size = 24.0;
        let mut drag_update: Option<(usize, f64)> = None;
        for (i, (n, d)) in planes.iter().enumerate() {
            let nw = transform::rotate_vec(ang, *n);
            let centre = add3(pos, scale3(nw, *d));
            // Two tangents.
            let helper = if nw.2.abs() < 0.9 { (0.0, 0.0, 1.0) } else { (1.0, 0.0, 0.0) };
            let t1 = {
                let c = (nw.1 * helper.2 - nw.2 * helper.1, nw.2 * helper.0 - nw.0 * helper.2, nw.0 * helper.1 - nw.1 * helper.0);
                let l = (c.0 * c.0 + c.1 * c.1 + c.2 * c.2).sqrt().max(1e-9);
                (c.0 / l, c.1 / l, c.2 / l)
            };
            let t2 = (nw.1 * t1.2 - nw.2 * t1.1, nw.2 * t1.0 - nw.0 * t1.2, nw.0 * t1.1 - nw.1 * t1.0);
            let corners = [
                add3(centre, add3(scale3(t1, size), scale3(t2, size))),
                add3(centre, add3(scale3(t1, -size), scale3(t2, size))),
                add3(centre, add3(scale3(t1, -size), scale3(t2, -size))),
                add3(centre, add3(scale3(t1, size), scale3(t2, -size))),
            ];
            let pts: Vec<egui::Pos2> = corners.iter().filter_map(|c| view.project(*c).map(|(s, _)| s)).collect();
            if pts.len() == 4 {
                p.add(egui::Shape::closed_line(pts, egui::Stroke::new(1.5, colours().clip)));
            }
            // Handle: a dot at the tip of the normal, draggable along it.
            let tip = add3(centre, scale3(nw, 12.0));
            let (Some((sc, _)), Some((st, _))) = (view.project(centre), view.project(tip)) else { continue };
            p.line_segment([sc, st], egui::Stroke::new(1.5, colours().clip));
            let id = egui::Id::new(("clip-handle", index.to_bits(), i));
            let hit = ui.interact(Rect::from_center_size(st, egui::vec2(14.0, 14.0)), id, Sense::drag());
            let colour = if hit.hovered() || hit.dragged() { colours().selection_text } else { colours().clip };
            p.circle_filled(st, 5.0, colour);
            p.text(st + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, format!("clip {i}  d={d:.1}"), egui::FontId::proportional(10.0), colour);
            if hit.dragged() {
                // Screen movement along the normal's screen direction → world units.
                let dir = st - sc;
                let len2 = dir.length_sq().max(1e-6);
                let t = hit.drag_delta().dot(dir) / len2; // fraction of the 12-unit handle
                drag_update = Some((i, *d + t as f64 * 12.0));
            }
        }
        if let Some((i, nd)) = drag_update {
            let mut planes = planes;
            planes[i].1 = nd;
            self.set_clip_planes(index, &planes);
        }
    }

    /// 25: the chosen recipe on the ticked wheels. Locked pairs are
    /// same-side wheels in y order; sprung is per wheel.
    fn apply_suspension(&mut self, base: f64) {
        let wheels: Vec<f64> = self.marked.clone();
        if wheels.is_empty() {
            return;
        }
        self.snapshot();
        let choice = self.suspension;
        let mut log = Vec::new();
        let mut fail: Option<String> = None;
        {
            let Some(open) = self.open.as_mut() else { return };
            let pos = |d: &Dupe, i: f64| transform::entity_transform(d, i).map(|(p, _)| p).unwrap_or((0.0, 0.0, 0.0));
            match choice {
                SuspensionChoice::Simple | SuspensionChoice::Locked => {
                    for w in &wheels {
                        if let Err(e) = ad2read::build::add_wheel_axis(&mut open.dupe, *w, base, (0.0, 1.0, 0.0), choice == SuspensionChoice::Locked) {
                            fail = Some(e);
                            break;
                        }
                    }
                    log.push(format!("{} axes", wheels.len()));
                    if choice == SuspensionChoice::Locked && fail.is_none() {
                        let (bp, _) = transform::entity_transform(&open.dupe, base).unwrap_or(((0.0, 0.0, 0.0), (0.0, 0.0, 0.0)));
                        let mut right: Vec<(f64, f64)> = Vec::new();
                        let mut left: Vec<(f64, f64)> = Vec::new();
                        for w in &wheels {
                            let p = pos(&open.dupe, *w);
                            if p.0 >= bp.0 { right.push((p.1, *w)) } else { left.push((p.1, *w)) }
                        }
                        let mut pairs = 0;
                        for side in [&mut right, &mut left] {
                            side.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                            for pair in side.windows(2) {
                                if ad2read::build::add_pair_lock(&mut open.dupe, pair[0].1, pair[1].1).is_ok() {
                                    pairs += 1;
                                }
                            }
                        }
                        log.push(format!("{pairs} pair locks"));
                    }
                }
                SuspensionChoice::Sprung => {
                    for w in &wheels {
                        if let Err(e) = ad2read::build::add_sprung_wheel(&mut open.dupe, *w, base) {
                            fail = Some(e);
                            break;
                        }
                    }
                    log.push(format!("{} sprung wheels", wheels.len()));
                }
            }
            if fail.is_none() {
                open.dirty = true;
            }
        }
        match fail {
            Some(e) => self.bad(format!("suspension: {e}")),
            None => self.good(format!("suspension ({:?}) on base {base}: {}", choice, log.join(", "))),
        }
    }

    /// 25: the geometry the recipe would add, drawn on the ticked wheels.
    fn draw_suspension_preview(&self, p: &egui::Painter, view: &View) {
        let Some(open) = self.open.as_ref() else { return };
        let Some(base) = self.wheel_base else { return };
        let Some((bp, ba)) = transform::entity_transform(&open.dupe, base) else { return };
        let stroke = egui::Stroke::new(1.5, colours().selection_text);
        let dim = egui::Stroke::new(1.0, colours().text_dim);
        let line = |a: V3, b: V3, st: egui::Stroke| {
            if let (Some((sa, _)), Some((sb, _))) = (view.project(a), view.project(b)) {
                p.line_segment([sa, sb], st);
            }
        };
        let mut sides: (Vec<(f64, V3)>, Vec<(f64, V3)>) = (Vec::new(), Vec::new());
        for w in &self.marked {
            let Some((wp, wa)) = transform::entity_transform(&open.dupe, *w) else { continue };
            // Axle through the wheel along its local Y.
            let axle = transform::rotate_vec(wa, (0.0, 1.0, 0.0));
            line(add3(wp, scale3(axle, -20.0)), add3(wp, scale3(axle, 20.0)), stroke);
            match self.suspension {
                SuspensionChoice::Sprung => {
                    // In the base's frame: spring anchor 17 up, ropes to (-100, ±100), travel 14.
                    let r = ad2read::build::world_to_local(wp, bp, ba);
                    let to_world = |l: V3| add3(bp, transform::rotate_vec(ba, l));
                    line(wp, to_world((r.0, r.1, r.2 + 17.0)), stroke);
                    line(wp, to_world((r.0 - 100.0, r.1 + 100.0, r.2)), dim);
                    line(wp, to_world((r.0 - 100.0, r.1 - 100.0, r.2)), dim);
                    if let Some((c, _)) = view.project(wp) {
                        p.circle_stroke(c, 6.0, dim);
                    }
                }
                SuspensionChoice::Locked => {
                    if wp.0 >= bp.0 { sides.0.push((wp.1, wp)) } else { sides.1.push((wp.1, wp)) }
                }
                SuspensionChoice::Simple => {}
            }
        }
        for side in [&mut sides.0, &mut sides.1] {
            side.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            for pair in side.windows(2) {
                line(pair[0].1, pair[1].1, stroke);
            }
        }
    }

    /// 18: one click makes a prop a wheel — hinge about its own axle
    /// (thinnest bounds axis, Y if unknown), spherical collision, gearbox link.
    fn make_wheel(&mut self, wheel: f64, base: f64, gearbox: Option<f64>) {
        let model = self
            .open
            .as_ref()
            .and_then(|o| o.dupe.list_entities().into_iter().find(|(i, _, _)| *i == wheel))
            .map(|(_, _, m)| m.trim_matches('"').to_owned());
        let half = model
            .as_ref()
            .and_then(|m| self.models.as_mut().and_then(|lib| lib.bounds(m)).map(|(b, _)| b.half_extents()))
            .or_else(|| model.as_deref().and_then(ad2read::build::sprops_half_extents));
        let (axle, radius) = match half {
            Some(h) => (ad2read::build::thinnest_axis(h), Some(h.0.max(h.1).max(h.2))),
            None => ((0.0, 1.0, 0.0), None),
        };
        self.snapshot();
        let mut msgs = Vec::new();
        let mut fail = None;
        if let Some(open) = self.open.as_mut() {
            match ad2read::build::add_wheel_axis(&mut open.dupe, wheel, base, axle, false) {
                Ok(()) => msgs.push(format!("axis about local {:?}", axle)),
                Err(e) => fail = Some(e),
            }
            if fail.is_none() {
                if let Some(r) = radius {
                    if ad2read::build::make_spherical(&mut open.dupe, wheel, r).is_ok() {
                        msgs.push(format!("spherical r={r:.1}"));
                    }
                } else {
                    msgs.push("no bounds: no spherical (point at garrysmod)".into());
                }
                if let Some(g) = gearbox {
                    match ad2read::build::add_link(&mut open.dupe, g, wheel) {
                        Ok(r) => msgs.push(format!("{} from {}", r.modifier, g)),
                        Err(e) => msgs.push(format!("link failed: {e}")),
                    }
                }
                open.dirty = true;
            }
        }
        match fail {
            Some(e) => self.bad(format!("make wheel: {e}")),
            None => self.good(format!("wheel {wheel} on {base}: {}", msgs.join(", "))),
        }
    }

    /// What the contraption weighs, worked out once per edit: the headline
    /// for the top of the window, and the weight by role behind it.
    fn weight(&mut self) -> Option<(String, String)> {
        if self.weight_cache.as_ref().is_some_and(|(at, _, _)| *at == self.edit_count) {
            return self.weight_cache.as_ref().map(|(_, headline, detail)| (headline.clone(), detail.clone()));
        }
        let open = self.open.as_ref()?;
        let models = &mut self.models;
        let mut half_extents = |model: &str| models.as_mut().and_then(|lib| lib.bounds(model)).map(|(bounds, _)| bounds.half_extents());
        let report = ad2read::mass::of_dupe(&open.dupe, &mut half_extents);
        let mut detail: Vec<String> = report.by_role(&open.dupe).into_iter().map(|(role, kg)| format!("{kg:>9.0} kg  {role}")).collect();
        if !report.unknown.is_empty() {
            detail.push(format!("{} plain props weigh what their models weigh in game, which a dupe does not say", report.unknown.len()));
        }
        self.weight_cache = Some((self.edit_count, report.headline(), detail.join("\n")));
        self.weight_cache.as_ref().map(|(_, headline, detail)| (headline.clone(), detail.clone()))
    }

    /// The entities whose role is switched off in View > Show, by the bits
    /// of their index.
    pub(crate) fn hidden_entities(&mut self) -> std::collections::HashSet<u64> {
        if self.hidden_roles.is_empty() {
            return std::collections::HashSet::new();
        }
        if let Some((at, found)) = &self.hidden_cache {
            if *at == self.edit_count {
                return found.clone();
            }
        }
        let found: std::collections::HashSet<u64> = self
            .open
            .as_ref()
            .map(|open| ad2read::roles::of_dupe(&open.dupe).into_iter().filter(|(_, role)| self.hidden_roles.contains(role.name())).map(|(index, _)| index.to_bits()).collect())
            .unwrap_or_default();
        self.hidden_cache = Some((self.edit_count, found.clone()));
        found
    }

    /// Switches a role on or off in the view.
    pub(crate) fn show_role(&mut self, role: &'static str, shown: bool) {
        if shown {
            self.hidden_roles.remove(role);
        } else {
            self.hidden_roles.insert(role);
        }
        self.hidden_cache = None;
    }

    /// Where the contraption balances: each weighed part's weight at its
    /// position. Unweighed parts are left out, as the headline says.
    fn balance_point(&mut self) -> Option<V3> {
        if let Some((at, found)) = self.balance_cache {
            if at == self.edit_count {
                return found;
            }
        }
        let open = self.open.as_ref()?;
        let models = &mut self.models;
        let mut half_extents = |model: &str| models.as_mut().and_then(|lib| lib.bounds(model)).map(|(bounds, _)| bounds.half_extents());
        let report = ad2read::mass::of_dupe(&open.dupe, &mut half_extents);
        let (mut sum, mut total) = ((0.0, 0.0, 0.0), 0.0);
        for part in &report.parts {
            if let Some((at, _)) = transform::entity_transform(&open.dupe, part.index) {
                sum = add3(sum, scale3(at, part.kg));
                total += part.kg;
            }
        }
        let found = (total > 0.0).then(|| scale3(sum, 1.0 / total));
        self.balance_cache = Some((self.edit_count, found));
        found
    }

    fn weight_label(&mut self, ui: &mut Ui) {
        if let Some((headline, detail)) = self.weight() {
            ui.label(RichText::new(headline).color(colours().heading)).on_hover_text(RichText::new(detail).monospace());
        }
    }

    /// Puts the dupe where AD2 would paste it at the map's spawn: the head
    /// entity at info_player_start, lifted by HeadEnt.Z so the lowest point
    /// sits on the ground. The move is a real translation (constraints
    /// resynced) but not an edit — a dupe is position-independent, so the
    /// file isn't marked dirty. Then frames the camera on it.
    fn dupe_to_spawn(&mut self) {
        let Some(sp) = self.map.as_ref().and_then(|m| m.spawn) else { return };
        let Some(open) = self.open.as_mut() else {
            self.camera.eye = (sp[0] as f64, sp[1] as f64, sp[2] as f64 + 64.0);
            return;
        };
        let head = open.dupe.root_table().ok().and_then(|r| open.dupe.get_table(r, "HeadEnt"));
        let (head_index, z) = match head {
            Some(h) => (open.dupe.get_number(h, "Index"), open.dupe.get_number(h, "Z").unwrap_or(0.0)),
            None => (None, 0.0),
        };
        let head_pos = head_index
            .and_then(|i| transform::entity_transform(&open.dupe, i).map(|(p, _)| p))
            .or_else(|| open.dupe.list_entities().first().and_then(|(i, _, _)| transform::entity_transform(&open.dupe, *i).map(|(p, _)| p)));
        let Some(hp) = head_pos else { return };
        let target = (sp[0] as f64, sp[1] as f64, sp[2] as f64 + z);
        let delta = (target.0 - hp.0, target.1 - hp.1, target.2 - hp.2);
        if delta.0.abs() + delta.1.abs() + delta.2.abs() > 0.01 {
            let was_dirty = open.dirty;
            let moved = transform::translate(&mut open.dupe, delta);
            transform::resync_constraints(&mut open.dupe);
            open.dirty = was_dirty;
            self.say(format!("dupe moved to spawn ({moved} entities, +{z:.0} up)"));
        }
        self.frame_camera();
    }

    fn copy_selection(&mut self) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        let made = {
            let Some(open) = self.open.as_ref() else { return };
            ad2read::duplicate::extract(&open.dupe, &targets)
        };
        match made {
            Ok(d) => {
                self.clipboard = Some(d);
                self.good(format!("copied {} entities", targets.len()));
            }
            Err(e) => self.bad(e),
        }
    }

    fn paste_clipboard(&mut self) {
        let Some(src) = self.clipboard.as_ref() else {
            return self.say("clipboard is empty");
        };
        // Dupe has no Clone: the arena is the whole document, so cloning it is
        // explicit rather than something that happens by accident.
        let copy = Dupe {
            root: src.root.clone(),
            arena: src.arena.clone(),
        };
        self.snapshot();
        let offset = self.op_offset;
        let result = {
            let Some(open) = self.open.as_mut() else { return };
            ad2read::merge::merge(&mut open.dupe, copy, offset)
        };
        match result {
            Ok(r) => self.good(format!(
                "pasted {} entities (from {}) and {} constraints",
                r.entities_added, r.first_new_index, r.constraints_added
            )),
            Err(e) => self.bad(e),
        }
        self.check_dangling();
    }

    /// The right-click menu, shared by the viewport and the tree so both
    /// behave the same way.
    fn context_menu(&mut self, ui: &mut Ui) {
        let targets = self.targets();
        let one = if targets.len() == 1 {
            Some(targets[0])
        } else {
            None
        };

        ui.label(
            RichText::new(match targets.len() {
                0 => "nothing selected".to_owned(),
                1 => format!("entity {}", targets[0]),
                n => format!("{n} entities"),
            })
            .color(colours().text_dim),
        );
        ui.separator();

        if let Some(chip) = one.filter(|index| self.is_chip(*index)) {
            if ui.button("Edit chip").clicked() {
                self.open_chip_editor(chip);
                self.menu_at = None;
            }
            ui.separator();
        }
        if ui.button("Frame selection").clicked() {
            if let Some(open) = self.open.as_ref() {
                let mut lo = (f64::MAX, f64::MAX, f64::MAX);
                let mut hi = (f64::MIN, f64::MIN, f64::MIN);
                let mut any = false;
                for t in &targets {
                    if let Some((p, _)) = transform::entity_transform(&open.dupe, *t) {
                        lo = (lo.0.min(p.0), lo.1.min(p.1), lo.2.min(p.2));
                        hi = (hi.0.max(p.0), hi.1.max(p.1), hi.2.max(p.2));
                        any = true;
                    }
                }
                if any {
                    let span = sub3(hi, lo);
                    self.camera
                        .look_at(scale3(add3(lo, hi), 0.5), dot3(span, span).sqrt() * 0.5 + 24.0);
                }
            }
            self.menu_at = None;
        }

        if ui.button("Select everything linked").clicked() {
            self.marked = self.linked_to_selection();
            self.good(format!("{} entities ticked", self.marked.len()));
            self.menu_at = None;
        }

        ui.separator();

        if ui.button("Copy").clicked() {
            self.copy_selection();
            self.menu_at = None;
        }
        let has_clip = self.clipboard.is_some();
        if ui.add_enabled(has_clip, egui::Button::new("Paste")).clicked() {
            self.paste_clipboard();
            self.menu_at = None;
        }
        if ui.button("Duplicate").clicked() {
            self.op_duplicate(&targets);
            self.menu_at = None;
        }
        if ui.button("Delete").clicked() {
            self.op_delete(&targets);
            self.menu_at = None;
        }

        ui.separator();
        let can_weld = targets.len() == 1;
        if ui.add_enabled(can_weld, egui::Button::new("Weld to world")).clicked() {
            self.snapshot();
            let r = {
                let Some(open) = self.open.as_mut() else { return };
                ad2read::extras::weld_to_world(&mut open.dupe, targets[0])
            };
            match r {
                Ok(()) => {
                    if let Some(open) = self.open.as_mut() {
                        open.dirty = true;
                    }
                    self.good(format!("welded entity {} to the world", targets[0]));
                }
                Err(e) => self.bad(e),
            }
            self.menu_at = None;
        }
        // Parenting is BuildDupeInfo.DupeParentID on the child — that is what
        // AdvDupe2 itself reads to call SetParent on paste. Multi-Parent and
        // the parenting tools all end up writing this same field.
        if self.marked.len() >= 2 {
            if let Some(Pick::Entity(parent)) = self.pick {
                let children: Vec<f64> = self.marked.iter().copied().filter(|m| *m != parent).collect();
                if !children.is_empty()
                    && ui
                        .button(format!("Parent {} ticked to entity {parent}", children.len()))
                        .clicked()
                {
                    self.set_parent(&children, Some(parent));
                    self.menu_at = None;
                }
            }
        }
        if !targets.is_empty() && ui.button("Unparent").clicked() {
            self.set_parent(&targets, None);
            self.menu_at = None;
        }
        if !targets.is_empty() && ui.button("Save as selection set...").clicked() {
            let name = format!("set {}", self.sets.len() + 1);
            self.sets.push((name, targets.clone()));
            self.menu_at = None;
        }

        // --- modifiers ---------------------------------------------------
        // Every duplicator modifier the entity actually carries, listed from
        // the dupe rather than from a hardcoded set. A dump of one real build
        // turned up sixteen of them — colour, material, mass, ACF_Armor,
        // buoyancy, inertia, three different wing addons — and addons will
        // keep inventing more, so the menu reads what is there.
        if let Some(index) = one {
            ui.separator();
            let mods = self.modifier_list(index);
            if mods.is_empty() {
                ui.label(RichText::new("no modifiers on this entity").color(colours().text_dim));
            } else {
                for (name, table) in mods {
                    let label = name.clone();
                    ui.menu_button(format!("{name}..."), |ui| {
                        self.armour_guard(ui, index, &label);
                        self.modifier_editor(ui, table);
                        self.add_field_row(ui, table);
                        ui.separator();
                        if ui.button(format!("Remove `{label}`")).clicked() {
                            self.remove_modifier(index, &label);
                            self.menu_at = None;
                        }
                    });
                }
            }
            ui.menu_button("Add modifier...", |ui| self.add_modifier_menu(ui, index));
        }
    }

    /// The modifiers an entity carries, as (name, arena table).
    fn modifier_list(&self, index: f64) -> Vec<(String, usize)> {
        let Some(open) = self.open.as_ref() else {
            return Vec::new();
        };
        let Some(et) = transform::entity_table(&open.dupe, index) else {
            return Vec::new();
        };
        let Some(mods) = open.dupe.get_table(et, "EntityMods") else {
            return Vec::new();
        };
        let Node::Table(entries) = &open.dupe.arena[mods] else {
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|(k, v)| Some((key_label(k), table_index(v)?)))
            .collect()
    }

    /// ACF ignores `ACF_Armor.Thickness` whenever a `mass` modifier is also
    /// present — `ACF.UpdateThickness` derives armour FROM the mass in that
    /// case and throws the thickness away. So editing either one while the
    /// other exists silently does nothing, and the editor has to say so.
    ///
    /// While here, it also shows what a thickness will weigh:
    ///   Mass = Area × (1 + Ductility)^0.5 × Thickness × 0.00078
    /// with Ductility stored ×100. Area is the physics mesh's surface area in
    /// game; without the .phy the render mesh is the best available stand-in,
    /// and it overestimates on anything with fine detail, so it's labelled.
    fn armour_guard(&mut self, ui: &mut Ui, index: f64, which: &str) {
        if which != "ACF_Armor" && which != "mass" {
            return;
        }
        let mods = self.modifier_list(index);
        let has_armor = mods.iter().any(|(n, _)| n == "ACF_Armor");
        let has_mass = mods.iter().any(|(n, _)| n == "mass");

        if has_armor && has_mass {
            ui.label(
                RichText::new(
                    "Both ACF_Armor and mass are set. ACF will IGNORE the armour \
                     thickness and derive armour from the mass instead.",
                )
                .color(colours().bad),
            );
            let other = if which == "ACF_Armor" { "mass" } else { "ACF_Armor" };
            if ui.button(format!("Remove `{other}` so this one takes effect")).clicked() {
                self.remove_modifier(index, other);
            }
            ui.separator();
        }

        if which == "ACF_Armor" {
            let (thickness, ductility) = {
                let table = mods.iter().find(|(n, _)| n == "ACF_Armor").map(|(_, t)| *t);
                let Some(open) = self.open.as_ref() else { return };
                match table {
                    Some(t) => (
                        open.dupe.get_number(t, "Thickness").unwrap_or(0.0),
                        open.dupe.get_number(t, "Ductility").unwrap_or(0.0),
                    ),
                    None => return,
                }
            };
            let model = {
                let Some(open) = self.open.as_ref() else { return };
                open.dupe
                    .list_entities()
                    .into_iter()
                    .find(|(i, _, _)| *i == index)
                    .map(|(_, _, m)| m.trim_matches('"').to_owned())
            };
            let area_cm2 = model
                .and_then(|m| self.models.as_mut().and_then(|lib| lib.mesh(&m)))
                .map(|mesh| {
                    let mut a = 0.0f64;
                    for t in mesh.indices.chunks_exact(3) {
                        let p = |i: u32| {
                            let q = mesh.positions[i as usize];
                            (q[0] as f64, q[1] as f64, q[2] as f64)
                        };
                        let (a0, b0, c0) = (p(t[0]), p(t[1]), p(t[2]));
                        let e1 = sub3(b0, a0);
                        let e2 = sub3(c0, a0);
                        let cr = cross3(e1, e2);
                        a += dot3(cr, cr).sqrt() * 0.5;
                    }
                    // ACF.UpdateArea: physics-mesh area × InchToCmSq × an
                    // AreaMult of 0.52505066107 that ACF's own comment calls
                    // a cm-to-grid-units conversion. Without the multiplier
                    // the preview read ~1.9× heavy.
                    a * 6.45 * 0.52505066107
                });
            match area_cm2 {
                Some(area) => {
                    let mass = area * (1.0 + ductility / 100.0).sqrt() * thickness * 0.00078;
                    ui.label(
                        RichText::new(format!(
                            "~{mass:.1} kg at {thickness:.0} mm  (render-mesh area {area:.0} cm², \
                             an overestimate on detailed props)"
                        ))
                        .color(colours().text_dim),
                    );
                }
                None => {
                    ui.label(
                        RichText::new("load models to preview the mass this thickness costs")
                            .color(colours().text_dim),
                    );
                }
            }
            ui.separator();
        }
    }

    /// Edits every field of a modifier, typed from what the field already
    /// holds. A nested table of r/g/b/a becomes a colour picker, since that is
    /// how GMod's colour tool stores a Color — AD2 has no Color type, so it
    /// lands as four numbers.
    fn modifier_editor(&mut self, ui: &mut Ui, table: usize) {
        let rows: Vec<(String, Value)> = {
            let Some(open) = self.open.as_ref() else { return };
            match &open.dupe.arena[table] {
                Node::Table(entries) => entries
                    .iter()
                    .map(|(k, v)| (key_label(k), v.clone()))
                    .collect(),
                Node::Array(items) => items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (format!("[{i}]"), v.clone()))
                    .collect(),
            }
        };
        if rows.is_empty() {
            ui.label(RichText::new("empty").color(colours().text_dim));
            return;
        }

        let mut writes: Vec<(usize, Value)> = Vec::new();
        let mut began = false;
        let mut descend: Option<usize> = None;

        for (row, (label, value)) in rows.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [130.0, 18.0],
                    egui::Label::new(RichText::new(label).color(colours().text_dim)),
                );
                let mut v = value.clone();
                let mut edited = false;
                match &mut v {
                    Value::Number(n) => {
                        let r = ui.add(egui::DragValue::new(n).speed(0.1));
                        began |= r.drag_started();
                        edited |= r.changed();
                    }
                    Value::Bool(b) => {
                        if ui.checkbox(b, "").changed() {
                            began = true;
                            edited = true;
                        }
                    }
                    Value::Vector(x, y, z) => {
                        for (l, c) in [("x", x), ("y", y), ("z", z)] {
                            let r = ui.add(
                                egui::DragValue::new(c)
                                    .speed(0.25)
                                    .fixed_decimals(3)
                                    .prefix(format!("{l} ")),
                            );
                            began |= r.drag_started();
                            edited |= r.changed();
                        }
                    }
                    Value::Angle(pi, ya, ro) => {
                        for (l, c) in [("p", pi), ("y", ya), ("r", ro)] {
                            let r = ui.add(
                                egui::DragValue::new(c)
                                    .speed(0.25)
                                    .fixed_decimals(3)
                                    .prefix(format!("{l} ")),
                            );
                            began |= r.drag_started();
                            edited |= r.changed();
                        }
                    }
                    Value::Str(bytes) => match String::from_utf8(bytes.clone()) {
                        Ok(mut text) => {
                            let r =
                                ui.add(egui::TextEdit::singleline(&mut text).desired_width(220.0));
                            if r.gained_focus() {
                                began = true;
                            }
                            if r.changed() {
                                *bytes = text.into_bytes();
                                edited = true;
                            }
                        }
                        Err(_) => {
                            ui.label(
                                RichText::new(format!("<{} raw bytes>", bytes.len())).color(colours().bad),
                            );
                        }
                    },
                    Value::Nil => {
                        ui.label(RichText::new("nil").color(colours().text_dim));
                    }
                    Value::Table(t) => {
                        let t = *t;
                        if let Some(mut col) = self.as_colour(t) {
                            if ui.color_edit_button_srgba(&mut col).changed() {
                                began = true;
                                self.write_colour(t, col);
                            }
                        } else {
                            let count = {
                                let Some(open) = self.open.as_ref() else { return };
                                match &open.dupe.arena[t] {
                                    Node::Table(e) => format!("table · {} keys", e.len()),
                                    Node::Array(i) => format!("array · {} items", i.len()),
                                }
                            };
                            if ui.button(count).clicked() {
                                descend = Some(t);
                            }
                        }
                    }
                }
                if edited {
                    writes.push((row, v));
                }
            });
        }

        if began {
            self.snapshot();
        }
        if !writes.is_empty() {
            if let Some(open) = self.open.as_mut() {
                match &mut open.dupe.arena[table] {
                    Node::Table(entries) => {
                        for (row, v) in writes {
                            entries[row].1 = v;
                        }
                    }
                    Node::Array(items) => {
                        for (row, v) in writes {
                            items[row] = v;
                        }
                    }
                }
                open.dirty = true;
            }
        }
        if let Some(t) = descend {
            ui.separator();
            self.modifier_editor(ui, t);
        }
    }

    /// A table of exactly r/g/b/a numbers, which is how a Color survives the
    /// trip through a format that has no Color type.
    fn as_colour(&self, table: usize) -> Option<Color32> {
        let open = self.open.as_ref()?;
        let Node::Table(entries) = &open.dupe.arena[table] else {
            return None;
        };
        if entries.len() != 4 {
            return None;
        }
        let mut got = [None; 4];
        for (k, v) in entries {
            let Value::Number(n) = v else { return None };
            match key_label(k).as_str() {
                "r" => got[0] = Some(*n),
                "g" => got[1] = Some(*n),
                "b" => got[2] = Some(*n),
                "a" => got[3] = Some(*n),
                _ => return None,
            }
        }
        let c: Vec<u8> = got.iter().map(|x| x.unwrap_or(255.0) as u8).collect();
        if got.iter().any(|x| x.is_none()) {
            return None;
        }
        Some(Color32::from_rgba_premultiplied(c[0], c[1], c[2], c[3]))
    }

    fn write_colour(&mut self, table: usize, col: Color32) {
        if let Some(open) = self.open.as_mut() {
            for (k, v) in [("r", col.r()), ("g", col.g()), ("b", col.b()), ("a", col.a())] {
                open.dupe.set(table, k, Value::Number(v as f64));
            }
            open.dirty = true;
        }
    }

    fn viewport(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, colours().background);

        // --- camera input, unless a gizmo handle has the drag ------------
        // First-person: drag looks around, shift-drag or right-drag slides,
        // the wheel dollies. WASD and the arrows fly, held keys keep the
        // frame loop running so motion is continuous rather than stepping
        // once per key event — that stepping was the "blocky" feel.
        if self.drag_axis.is_none() && self.grab.is_none() {
            let v = self.camera.view(rect);
            if resp.dragged_by(egui::PointerButton::Secondary)
                || (resp.dragged_by(egui::PointerButton::Primary)
                    && ui.input(|i| i.modifiers.shift))
            {
                let d = resp.drag_delta();
                let k = self.config.viewport.fly_speed * 0.004;
                self.camera.eye = add3(
                    self.camera.eye,
                    add3(scale3(v.right, -d.x as f64 * k), scale3(v.up, d.y as f64 * k)),
                );
            } else if resp.dragged_by(egui::PointerButton::Primary) {
                // Hide the cursor and warp it back to the viewport centre
                // every frame, reading the look delta as its offset from that
                // centre. So the pointer never reaches an edge and never
                // shows, the way a game does it.
                // The warp target must sit on a whole physical pixel: the OS
                // rounds the cursor to one, and reading the offset from a
                // .5 centre gave a constant half-pixel "drift" every frame.
                let ppp = ui.ctx().pixels_per_point();
                let centre = egui::pos2(
                    (rect.center().x * ppp).round() / ppp,
                    (rect.center().y * ppp).round() / ppp,
                );
                let (pos, motion) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.motion()));
                if !self.looking {
                    self.looking = true;
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
                    // First frame: just warp, don't read a jump as a look.
                } else {
                    // Raw mouse motion when the platform reports it — it's
                    // independent of where the cursor was put. Otherwise the
                    // offset from the warp point, with a dead zone below a
                    // pixel so rounding can never look like input.
                    let d = match motion {
                        Some(m) => m,
                        None => match pos {
                            Some(p) => {
                                let d = p - centre;
                                if d.x.abs() < 1.0 && d.y.abs() < 1.0 { egui::Vec2::ZERO } else { d }
                            }
                            None => egui::Vec2::ZERO,
                        },
                    };
                    let turn = 0.004 * self.config.viewport.look_sensitivity.clamp(0.1, 5.0);
                    let up = if self.config.viewport.invert_look { -1.0 } else { 1.0 };
                    self.camera.yaw -= d.x as f64 * turn;
                    self.camera.pitch = (self.camera.pitch - d.y as f64 * turn * up).clamp(-1.52, 1.52);
                }
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::CursorPosition(centre));
                ui.ctx().request_repaint();
            }
        }
        // 25: suspension preview on ticked wheels (only when the panel's base is set).
        if !self.marked.is_empty() && self.wheel_base.is_some() {
            let v = self.camera.view(rect);
            self.draw_suspension_preview(&p, &v);
        }
        // 26: clip-plane gizmos on the selection.
        self.draw_clip_gizmos(ui, &p, rect, &resp);

                // Compass. Hammer's top view: +Y is north, +X is east. The rose
        // turns so the camera's heading is at the top; N E S W sit where
        // those directions actually are on screen.
        {
            // Foreground layer: the rendered scene image is painted into the
            // viewport after this code runs, and on the viewport's own painter
            // it covered the compass the moment anything was loaded.
            let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("compass")));
            let origin = egui::pos2(rect.right() - 48.0, rect.top() + 48.0);
            let r = 34.0f32;
            p.circle_filled(origin, r + 6.0, Color32::from_black_alpha(110));
            p.circle_stroke(origin, r, egui::Stroke::new(1.0, colours().text_dim));
            // yaw 0 looks +X (east), yaw 90 looks +Y (north).
            let yaw = self.camera.yaw;
            let at = |dir_yaw: f64| -> egui::Pos2 {
                let rel = dir_yaw - yaw;
                origin + egui::vec2(-(rel.sin() as f32), -(rel.cos() as f32)) * r
            };
            let half = std::f64::consts::FRAC_PI_2;
            for (label, dir_yaw, colour) in [
                ("N", half, colours().axis_x),
                ("E", 0.0, colours().text),
                ("S", -half, colours().text),
                ("W", std::f64::consts::PI, colours().text),
            ] {
                let pos = at(dir_yaw);
                p.line_segment([origin, origin + (pos - origin) * 0.55], egui::Stroke::new(1.0, colours().text_dim));
                p.text(pos, egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(11.0), colour);
            }
            // Heading marker at the top: the way the camera looks.
            p.add(egui::Shape::convex_polygon(
                vec![origin + egui::vec2(0.0, -r - 2.0), origin + egui::vec2(-4.0, -r - 10.0), origin + egui::vec2(4.0, -r - 10.0)],
                colours().selection_text,
                egui::Stroke::NONE,
            ));
            // Bearing from north, clockwise, like a real compass reads.
            let bearing = ((90.0 - yaw.to_degrees()) % 360.0 + 360.0) % 360.0;
            p.text(
                egui::pos2(origin.x, origin.y + r + 14.0),
                egui::Align2::CENTER_CENTER,
                format!("{bearing:.0}°"),
                egui::FontId::proportional(10.0),
                colours().text_dim,
            );
        }

        if self.looking && !resp.dragged_by(egui::PointerButton::Primary) {
            self.looking = false;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
        }

        if !ui.ctx().egui_wants_keyboard_input() {
            let (mut fwd, mut side, mut lift) = (0.0f64, 0.0f64, 0.0f64);
            // Ctrl slows a flight that is under way. From a standstill Ctrl
            // with A, S or D is a shortcut, so those keys do not also move.
            let steering = !ui.input(|i| i.modifiers.ctrl) || self.flying();
            ui.input(|i| {
                use egui::Key;
                let held = |k: Key| i.key_down(k);
                if held(Key::ArrowUp) || held(Key::W) {
                    fwd += 1.0;
                }
                if held(Key::ArrowDown) || (steering && held(Key::S)) {
                    fwd -= 1.0;
                }
                if held(Key::ArrowRight) || (steering && held(Key::D)) {
                    side += 1.0;
                }
                if held(Key::ArrowLeft) || (steering && held(Key::A)) {
                    side -= 1.0;
                }
                if held(Key::E) || held(Key::Space) {
                    lift += 1.0;
                }
                if held(Key::Q) {
                    lift -= 1.0;
                }
            });
            let (fast, slow) = ui.input(|i| (i.modifiers.shift, i.modifiers.ctrl));
            if fwd != 0.0 || side != 0.0 || lift != 0.0 {
                self.last_flew = Some(std::time::Instant::now());
                let dt = ui.input(|i| i.stable_dt).clamp(0.001, 0.05) as f64;
                let mult = if fast { 3.0 } else if slow { 0.25 } else { 1.0 };
                let step = self.config.viewport.fly_speed * mult * dt;
                let v = self.camera.view(rect);
                // Fly along the actual view direction, like noclip: looking
                // down and pressing W takes you down.
                self.camera.eye = add3(
                    self.camera.eye,
                    add3(
                        add3(scale3(v.fwd, fwd * step), scale3(v.right, side * step)),
                        (0.0, 0.0, lift * step),
                    ),
                );
                // Keep the frames coming while a key is held. Without this
                // egui only repaints on events, so held movement stutters.
                ui.ctx().request_repaint();
            }
        }

        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let v = self.camera.view(rect);
                self.camera.eye = add3(
                    self.camera.eye,
                    scale3(v.fwd, scroll as f64 * self.config.viewport.fly_speed * 0.003),
                );
            }
        }

        if self.open.is_none() {
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "no file open",
                ui_font(),
                colours().text_dim,
            );
            return;
        }

        let v = self.camera.view(rect);

        // --- real geometry, depth buffered -------------------------------
        // Painter's algorithm is exact for convex boxes and hopeless for a
        // model mesh, which self-occludes. So meshes go through a z-buffered
        // rasteriser and land as one texture underneath everything else.
        let mut drawn_as_mesh: Vec<f64> = Vec::new();
        if self.config.viewport.real_geometry && self.models.is_some() {
            // GPU when eframe's wgpu backend is up, which it is by default.
            // The CPU rasteriser stays as the fallback rather than being
            // deleted, because a machine where wgpu fails to initialise
            // should still get a viewport.
            drawn_as_mesh = if self.render_state.is_some() {
                self.render_meshes_gpu(ui, &v, rect)
            } else {
                self.render_meshes(ui, &v, rect)
            };
        } else {
            // No mesh pass this frame, so nothing is a ghost: the box pass
            // draws every entity solid again.
            self.ghosts.clear();
        }

        // --- collect geometry --------------------------------------------
        let fallback = self.box_size * 0.5;
        let light = norm3((0.35, 0.5, 1.0));

        // Resolve each entity's real size once per frame. The library caches
        // per model path, so this is a hash lookup after the first hit.
        let sizes: Vec<(f64, (f64, f64, f64), (f64, f64, f64))> = {
            let mut out = Vec::new();
            if let (Some(open), Some(lib)) = (self.open.as_ref(), self.models.as_mut()) {
                for (index, _, model) in open.dupe.list_entities() {
                    let clean = lib.model_of(&open.dupe, index, model.trim_matches('"'));
                    match lib.bounds(&clean) {
                        Some((b, _)) => {
                            let half = b.half_extents();
                            let centre = b.centre();
                            let scale = ad2read::scale::model_scale(&open.dupe, index, half);
                            out.push((
                                index,
                                (half.0 * scale.0, half.1 * scale.1, half.2 * scale.2),
                                (centre.0 * scale.0, centre.1 * scale.1, centre.2 * scale.2),
                            ));
                        }
                        None => out.push((
                            index,
                            (fallback, fallback, fallback),
                            (0.0, 0.0, 0.0),
                        )),
                    }
                }
            }
            out
        };
        let mut objects: Vec<Drawn> = Vec::new();
        let mut hits: Vec<(f64, Rect, f32)> = Vec::new();
        let switched_off = self.hidden_entities();

        {
            let Some(open) = self.open.as_ref() else { return };
            for (index, _, _) in open.dupe.list_entities() {
                if switched_off.contains(&index.to_bits()) {
                    continue;
                }
                let Some((pos, ang)) = transform::entity_transform(&open.dupe, index) else {
                    continue;
                };
                // A model's bounds are given in its own local space and are
                // rarely centred on the origin, so the offset has to be
                // rotated with the entity rather than just added.
                let (extents, offset) = sizes
                    .iter()
                    .find(|(i, _, _)| *i == index)
                    .map(|(_, e, c)| (*e, *c))
                    .unwrap_or(((fallback, fallback, fallback), (0.0, 0.0, 0.0)));
                let centre = add3(pos, transform::rotate_vec(ang, offset));
                let corners = box_corners(centre, ang, extents);

                let mut screen = [(pos2(0.0, 0.0), 0.0f32); 8];
                let mut ok = true;
                for (i, c) in corners.iter().enumerate() {
                    match v.project(*c) {
                        Some(sp) => screen[i] = sp,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    continue;
                }

                let selected = self.pick == Some(Pick::Entity(index));
                let base = if selected {
                    colours().box_selected
                } else if self.marked.contains(&index) {
                    colours().box_marked
                } else {
                    colours().box_base
                };

                let mut lo = screen[0].0;
                let mut hi = screen[0].0;
                let mut near = f32::MAX;
                for (sp, d) in screen.iter() {
                    lo = pos2(lo.x.min(sp.x), lo.y.min(sp.y));
                    hi = pos2(hi.x.max(sp.x), hi.y.max(sp.y));
                    near = near.min(*d);
                }
                hits.push((index, Rect::from_min_max(lo, hi), near));

                // Anything the mesh pass already drew only needs its box as a
                // selection highlight. The hit-test above must still run for
                // it, or nothing drawn as a mesh can be clicked — which was
                // the case, and is why the viewport felt dead.
                if self.ghosts.contains(&index) {
                    let colour = if selected {
                        colours().selection_text
                    } else if self.marked.contains(&index) {
                        colours().box_marked
                    } else {
                        colours().text_dim
                    };
                    let edges = BOX_EDGES.iter().map(|(a, b)| ([screen[*a].0, screen[*b].0], colour)).collect();
                    objects.push(Drawn { faces: Vec::new(), edges });
                    continue;
                }
                if drawn_as_mesh.contains(&index) {
                    // Real geometry was drawn: never a filled box over it.
                    // Selected or ticked gets a wire-frame outline; the rest
                    // nothing.
                    let marked = self.marked.contains(&index);
                    if selected || marked {
                        let colour = if selected { colours().selection_text } else { colours().box_marked };
                        let mut edges = Vec::new();
                        for (a, b) in BOX_EDGES {
                            edges.push(([screen[a].0, screen[b].0], colour));
                        }
                        objects.push(Drawn { faces: Vec::new(), edges });
                    }
                    continue;
                }

                let mut faces = Vec::new();
                for f in FACES.iter() {
                    let (a, b, c) = (corners[f[0]], corners[f[1]], corners[f[2]]);
                    let n = norm3(cross3(sub3(b, a), sub3(c, a)));
                    if dot3(n, sub3(a, v.eye)) > 0.0 {
                        continue;
                    }
                    let lit = 0.4 + 0.6 * dot3(n, light).max(0.0) as f32;
                    let fill = Color32::from_rgb(
                        (base.r() as f32 * lit) as u8,
                        (base.g() as f32 * lit) as u8,
                        (base.b() as f32 * lit) as u8,
                    );
                    let pts: Vec<egui::Pos2> = f.iter().map(|i| screen[*i].0).collect();
                    let d = f.iter().map(|i| screen[*i].1).sum::<f32>() / 4.0;
                    faces.push((d, pts, fill, selected));
                }
                objects.push(Drawn { faces, edges: Vec::new() });
            }

            // --- constraint lines ----------------------------------------
            for ct in constraint_list(&open.dupe) {
                let Some(list) = open.dupe.get_table(ct, "Entity") else {
                    continue;
                };
                let Node::Array(items) = &open.dupe.arena[list] else {
                    continue;
                };
                let ends: Vec<f64> = items
                    .iter()
                    .filter_map(|it| open.dupe.get_number(table_index(it)?, "Index"))
                    .collect();
                if ends.len() < 2 {
                    continue;
                }
                let a = transform::entity_transform(&open.dupe, ends[0]);
                let b = transform::entity_transform(&open.dupe, ends[ends.len() - 1]);
                if let (Some((pa, _)), Some((pb, _))) = (a, b) {
                    if let (Some((s0, _)), Some((s1, _))) = (v.project(pa), v.project(pb)) {
                        p.line_segment([s0, s1], Stroke::new(1.0, colours().link));
                    }
                }
            }
        }

        // --- ACF links and wires ------------------------------------------
        // Both live in EntityMods on the entity doing the linking (ACF) or
        // receiving (Wiremod). Drawn as lines from that entity to each target,
        // the same way constraints are, so a drivetrain reads at a glance.
        if self.show_links || self.show_wires {
            let Some(open) = self.open.as_ref() else { return };
            let at = |i: f64| transform::entity_transform(&open.dupe, i).map(|(p, _)| p);

            for (index, _, _) in open.dupe.list_entities() {
                let Some(et) = transform::entity_table(&open.dupe, index) else { continue };
                let Some(mods) = open.dupe.get_table(et, "EntityMods") else { continue };
                let Some(from) = at(index) else { continue };

                if self.show_links {
                    for name in ad2read::refs::ACF_LINKS {
                        let Some(arr) = open.dupe.get_table(mods, name) else { continue };
                        let targets: Vec<f64> = match &open.dupe.arena[arr] {
                            Node::Array(items) => items
                                .iter()
                                .filter_map(|v| if let Value::Number(n) = v { Some(*n) } else { None })
                                .collect(),
                            Node::Table(entries) => entries
                                .iter()
                                .filter_map(|(_, v)| if let Value::Number(n) = v { Some(*n) } else { None })
                                .collect(),
                        };
                        for t in targets {
                            if let (Some(to), Some((s0, _)), ) = (at(t), v.project(from)) {
                                if let Some((s1, _)) = v.project(to) {
                                    p.line_segment([s0, s1], Stroke::new(1.5, colours().link_acf));
                                }
                            }
                        }
                    }
                }

                if self.show_wires {
                    if let Some(wdi) = open.dupe.get_table(mods, "WireDupeInfo") {
                        if let Some(wires) = open.dupe.get_table(wdi, "Wires") {
                            if let Node::Table(ports) = &open.dupe.arena[wires] {
                                for (_, port) in ports {
                                    let Some(pt) = table_index(port) else { continue };
                                    let Some(src) = open.dupe.get_number(pt, "Src") else { continue };
                                    // Waypoints run source -> receiver; draw
                                    // through them when they're present.
                                    let mut chain: Vec<V3> = Vec::new();
                                    if let Some(s) = at(src) {
                                        chain.push(s);
                                    }
                                    if let Some(path) = open.dupe.get_table(pt, "Path") {
                                        if let Node::Array(pts) = &open.dupe.arena[path] {
                                            for wp in pts {
                                                let Some(wt) = table_index(wp) else { continue };
                                                if let Some(e) = open.dupe.get_number(wt, "Entity") {
                                                    if let Some(ep) = at(e) {
                                                        chain.push(ep);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    chain.push(from);
                                    for w in chain.windows(2) {
                                        if let (Some((s0, _)), Some((s1, _))) =
                                            (v.project(w[0]), v.project(w[1]))
                                        {
                                            p.line_segment([s0, s1], Stroke::new(1.0, colours().link_wire));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // --- draw, far to near -------------------------------------------
        // Faces sort globally, not per box. Ordering whole boxes by their
        // centre depth got the order wrong whenever boxes interleaved — which
        // in a build of overlapping blades is most of the time. Sorting every
        // face by its centroid handles interleaving correctly.
        //
        // The earlier flicker was not the sort: it was the 1px outline on
        // every face. Adjacent coplanar faces from different boxes fought over
        // the same pixels frame to frame. Faces are filled only now, and the
        // shading difference between them is what reads as an edge — which is
        // also how SFM's own viewport draws untextured geometry.
        let mut faces: Vec<(f32, Vec<egui::Pos2>, Color32, bool)> = Vec::new();
        let mut edges: Vec<([egui::Pos2; 2], Color32)> = Vec::new();
        for o in objects {
            faces.extend(o.faces);
            edges.extend(o.edges);
        }
        faces.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, pts, fill, outline) in faces {
            let edge = if outline {
                Stroke::new(1.0, colours().box_edge)
            } else {
                Stroke::NONE
            };
            p.add(egui::Shape::convex_polygon(pts, fill, edge));
        }
        // Selection outlines over everything, so they read on top of geometry.
        for (seg, colour) in edges {
            p.line_segment(seg, Stroke::new(1.5, colour));
        }

        // --- gizmo ---------------------------------------------------------
        let gizmo_target = match self.pick {
            Some(Pick::Entity(i)) => Some(i),
            _ => None,
        };
        let mut over_axis: Option<usize> = None;

        if let Some(index) = gizmo_target {
            let here = {
                let Some(open) = self.open.as_ref() else { return };
                transform::entity_transform(&open.dupe, index)
            };
            if let Some((origin, _)) = here {
                if let Some((o2, od)) = v.project(origin) {
                    // Constant on-screen size, so handles stay grabbable at
                    // any zoom.
                    let world_len = od as f64 * 90.0 / v.focal as f64;
                    // Moving and turning go by the world's axes; a size is the
                    // part's own length, width and height, so its handles run
                    // along the part's own axes.
                    let own = |axis: V3| match (self.gizmo, here) {
                        (Gizmo::Scale, Some((_, ang))) => transform::rotate_vec(ang, axis),
                        _ => axis,
                    };
                    let axes: [(V3, Color32); 3] = [
                        (own((1.0, 0.0, 0.0)), colours().axis_x),
                        (own((0.0, 1.0, 0.0)), colours().axis_y),
                        (own((0.0, 0.0, 1.0)), colours().axis_z),
                    ];
                    let pointer = resp.hover_pos().or(resp.interact_pointer_pos());
                    let size_now = self.open.as_ref().and_then(|open| ad2read::scale::dimensions(&open.dupe, index));

                    match self.gizmo {
                        Gizmo::Move => {
                            for (i, (dir, col)) in axes.iter().enumerate() {
                                let tip = add3(origin, scale3(*dir, world_len));
                                let Some((t2, _)) = v.project(tip) else { continue };
                                let hot = self.drag_axis == Some(i)
                                    || pointer
                                        .map(|pt| point_to_segment(pt, o2, t2) < 6.0)
                                        .unwrap_or(false);
                                if hot && self.drag_axis.is_none() {
                                    over_axis = Some(i);
                                }
                                let c = if hot { colours().axis_hot } else { *col };
                                p.line_segment(
                                    [o2, t2],
                                    Stroke::new(if hot { 3.0 } else { 2.0 }, c),
                                );
                                p.circle_filled(t2, if hot { 5.0 } else { 4.0 }, c);
                            }
                        }
                        Gizmo::Scale => {
                            if size_now.is_none() {
                                gizmo_label(&p, o2 + vec2(14.0, -14.0), &["this part has no size to drag".to_owned()]);
                            }
                            for (i, (dir, col)) in axes.iter().enumerate().filter(|_| size_now.is_some()) {
                                let tip = add3(origin, scale3(*dir, world_len));
                                let Some((t2, _)) = v.project(tip) else { continue };
                                let hot = self.drag_axis == Some(i) || pointer.map(|pt| point_to_segment(pt, o2, t2) < 6.0).unwrap_or(false);
                                if hot && self.drag_axis.is_none() {
                                    over_axis = Some(i);
                                }
                                let c = if hot { colours().axis_hot } else { *col };
                                p.line_segment([o2, t2], Stroke::new(if hot { 3.0 } else { 2.0 }, c));
                                p.rect_filled(Rect::from_center_size(t2, egui::Vec2::splat(if hot { 10.0 } else { 8.0 })), 1.0, c);
                            }
                        }
                        Gizmo::Rotate => {
                            // One ring per axis, lying in the plane the axis is
                            // normal to. Drawn as a projected polyline rather
                            // than a screen-space circle, so it foreshortens
                            // correctly when you look at it edge-on.
                            for (i, (axis, col)) in axes.iter().enumerate() {
                                let (u, w) = ring_basis(*axis);
                                let mut pts: Vec<egui::Pos2> = Vec::with_capacity(49);
                                for k in 0..=48 {
                                    let a = std::f64::consts::TAU * k as f64 / 48.0;
                                    let pnt = add3(
                                        origin,
                                        add3(
                                            scale3(u, world_len * a.cos()),
                                            scale3(w, world_len * a.sin()),
                                        ),
                                    );
                                    match v.project(pnt) {
                                        Some((sp, _)) => pts.push(sp),
                                        None => {
                                            pts.clear();
                                            break;
                                        }
                                    }
                                }
                                if pts.len() < 2 {
                                    continue;
                                }
                                let hot = self.drag_axis == Some(i)
                                    || pointer
                                        .map(|pt| point_to_polyline(pt, &pts) < 6.0)
                                        .unwrap_or(false);
                                if hot && self.drag_axis.is_none() {
                                    over_axis = Some(i);
                                }
                                let c = if hot { colours().axis_hot } else { *col };
                                p.add(egui::Shape::line(
                                    pts,
                                    Stroke::new(if hot { 3.0 } else { 2.0 }, c),
                                ));
                            }
                        }
                    }

                    // --- grab / drag / release a handle ---------------------
                    // The handle is claimed the moment the button goes down on
                    // it. Waiting for egui to call it a drag let the camera's
                    // mouse look run for one frame first, and its cursor warp
                    // then read as a huge drag: the part jumped on every grab.
                    let (pressed, held, shift, alt) = ui.input(|i| {
                        (i.pointer.primary_pressed(), i.pointer.primary_down(), i.modifiers.shift, i.modifiers.alt)
                    });
                    let steps = self.config.placement.steps(alt);
                    if pressed && !shift && self.grab.is_none() {
                        if let (Some(a), Some(pt), Some((_, angle))) = (over_axis, pointer, here) {
                            let ray = v.ray(pt);
                            let (u, w) = ring_basis(axes[a].0);
                            let ring = gizmo::angle_in_plane(origin, axes[a].0, u, w, v.eye, ray);
                            self.grab = Some(Grab {
                                axis: a,
                                origin,
                                angle,
                                along: gizmo::along_axis(origin, axes[a].0, v.eye, ray),
                                ring_start: ring.unwrap_or(0.0),
                                ring_last: None,
                                ring_from_screen: false,
                                change: 0.0,
                                size: size_now,
                                others: match self.open.as_ref() {
                                    Some(open) if self.marked.contains(&index) => {
                                        self.marked.iter().filter(|other| **other != index).filter_map(|other| transform::entity_transform(&open.dupe, *other).map(|(at, ang)| (*other, at, ang))).collect()
                                    }
                                    _ => Vec::new(),
                                },
                            });
                        }
                    }
                    if !held {
                        self.grab = None;
                        self.drag_axis = None;
                    }
                    if resp.drag_started() && self.drag_axis.is_none() {
                        if let Some(a) = self.grab.as_ref().map(|grab| grab.axis) {
                            self.drag_axis = Some(a);
                            self.snapshot();
                        }
                    }

                    if let (Some(a), Some(pt), true) = (self.drag_axis, pointer, resp.dragged()) {
                        let axis = axes[a].0;
                        let ray = v.ray(pt);
                        let mut pose: Option<(Option<V3>, Option<V3>)> = None;
                        let mut together: Vec<(f64, V3, V3)> = Vec::new();
                        let mut wanted_size: Option<V3> = None;
                        if let Some(grab) = self.grab.as_mut() {
                            match self.gizmo {
                                Gizmo::Scale => {
                                    let now = gizmo::along_axis(grab.origin, axis, v.eye, ray);
                                    if let (Some(now), Some(start), Some(size)) = (now, grab.along, grab.size) {
                                        // The part grows both ways from its middle,
                                        // so a drag of one unit is two of size.
                                        let grown = steps.map_or(now - start, |(step, _)| gizmo::snapped(now - start, step)) * 2.0;
                                        grab.change = grown;
                                        let mut wanted = size;
                                        match a {
                                            0 => wanted.0 = (size.0 + grown).max(0.5),
                                            1 => wanted.1 = (size.1 + grown).max(0.5),
                                            _ => wanted.2 = (size.2 + grown).max(0.5),
                                        }
                                        wanted_size = Some(wanted);
                                    }
                                }
                                Gizmo::Move => {
                                    let now = gizmo::along_axis(grab.origin, axis, v.eye, ray);
                                    if let (Some(now), Some(start)) = (now, grab.along) {
                                        let moved = match steps {
                                            Some((step, _)) => gizmo::snapped_travel(dot3(grab.origin, axis), now - start, step),
                                            None => now - start,
                                        };
                                        grab.change = moved;
                                        pose = Some((Some(add3(grab.origin, scale3(axis, moved))), None));
                                        together = grab.others.iter().map(|(other, at, ang)| (*other, add3(*at, scale3(axis, moved)), *ang)).collect();
                                    }
                                }
                                Gizmo::Rotate => {
                                    let (u, w) = ring_basis(axis);
                                    let (now, from_screen) = match gizmo::angle_in_plane(grab.origin, axis, u, w, v.eye, ray) {
                                        Some(angle) => (angle, false),
                                        None => {
                                            // Edge-on, the ring's plane cannot be hit;
                                            // read the angle on the screen instead,
                                            // which runs backwards when the axis
                                            // points at the eye.
                                            let facing = if dot3(axis, sub3(v.eye, grab.origin)) >= 0.0 { -1.0 } else { 1.0 };
                                            ((pt - o2).angle() as f64 * facing, true)
                                        }
                                    };
                                    if let Some(last) = grab.ring_last {
                                        if grab.ring_from_screen == from_screen {
                                            grab.change += gizmo::shortest_turn(last, now);
                                        }
                                    }
                                    grab.ring_last = Some(now);
                                    grab.ring_from_screen = from_screen;
                                    let turned = steps.map_or(grab.change, |(_, step)| gizmo::snapped(grab.change, step.to_radians()));
                                    pose = Some((None, Some(transform::rotate_angle_about(grab.angle, axis, turned))));
                                    together = grab
                                        .others
                                        .iter()
                                        .map(|(other, at, ang)| (*other, gizmo::turned_about(*at, grab.origin, axis, turned), transform::rotate_angle_about(*ang, axis, turned)))
                                        .collect();
                                }
                            }
                        }
                        if let (Some((at, ang)), Some(open)) = (pose, self.open.as_mut()) {
                            let _ = transform::set_entity_transform(&mut open.dupe, index, at, ang);
                            for (other, at, ang) in together {
                                let _ = transform::set_entity_transform(&mut open.dupe, other, Some(at), Some(ang));
                            }
                            open.dirty = true;
                        }
                        if let (Some(wanted), Some(open)) = (wanted_size, self.open.as_mut()) {
                            // scale::resize holds it to what ACF allows: whole
                            // rounds for a crate, whole units for a tank.
                            let _ = ad2read::scale::resize(&mut open.dupe, index, wanted);
                            open.dirty = true;
                            self.edit_count += 1;
                        }
                    }

                    // --- what the drag has done so far ----------------------
                    if let (Some(a), Some(grab)) = (self.drag_axis, self.grab.as_ref()) {
                        let (axis, colour) = axes[a];
                        match self.gizmo {
                            Gizmo::Scale => {
                                if let (Some(size), Some(open)) = (size_now, self.open.as_ref()) {
                                    let holds = ad2read::containers::ammo_crate(&open.dupe, index)
                                        .map(|ammo| format!("{} rounds", ammo.rounds()))
                                        .or_else(|| ad2read::containers::tank(&open.dupe, index).map(|tank| format!("{:.0} L", tank.litres())));
                                    let mut lines = vec![format!("{:.1} x {:.1} x {:.1}", size.0, size.1, size.2)];
                                    lines.extend(holds);
                                    gizmo_label(&p, o2 + vec2(14.0, -14.0), &lines);
                                }
                            }
                            Gizmo::Move => {
                                if let Some((from, _)) = v.project(grab.origin) {
                                    p.line_segment([from, o2], Stroke::new(2.0, colours().axis_hot));
                                    p.circle_stroke(from, 4.0, Stroke::new(1.5, colours().axis_hot));
                                    gizmo_label(&p, o2 + vec2(14.0, -14.0), &[format!("{:+.1}", grab.change)]);
                                }
                            }
                            Gizmo::Rotate => {
                                let (u, w) = ring_basis(axis);
                                let turned = gizmo::shown_turn(steps.map_or(grab.change, |(_, step)| gizmo::snapped(grab.change, step.to_radians())));
                                let on_ring = |angle: f64, reach: f64| {
                                    v.project(add3(
                                        origin,
                                        add3(scale3(u, world_len * reach * angle.cos()), scale3(w, world_len * reach * angle.sin())),
                                    ))
                                    .map(|(at, _)| at)
                                };
                                // The pie: a filled wedge from where the grab
                                // began to where the part has been turned to.
                                let sweep = turned;
                                let steps = ((sweep.abs() / (std::f64::consts::TAU / 64.0)).ceil() as usize).max(1);
                                let fill = Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), 70);
                                let mut wedge = egui::Mesh::default();
                                wedge.colored_vertex(o2, fill);
                                let mut whole = true;
                                for k in 0..=steps {
                                    match on_ring(grab.ring_start + sweep * k as f64 / steps as f64, 1.0) {
                                        Some(at) => wedge.colored_vertex(at, fill),
                                        None => whole = false,
                                    }
                                }
                                if whole {
                                    for k in 1..=steps as u32 {
                                        wedge.add_triangle(0, k, k + 1);
                                    }
                                    p.add(egui::Shape::mesh(wedge));
                                }
                                if let Some(at) = on_ring(grab.ring_start, 1.0) {
                                    p.line_segment([o2, at], Stroke::new(1.5, colours().text_dim));
                                }
                                if let Some(at) = on_ring(grab.ring_start + turned, 1.0) {
                                    p.line_segment([o2, at], Stroke::new(2.5, colours().axis_hot));
                                }
                                let now = {
                                    let Some(open) = self.open.as_ref() else { return };
                                    transform::entity_transform(&open.dupe, index).map(|(_, angle)| angle).unwrap_or(grab.angle)
                                };
                                let lines = [
                                    format!("{:+.1}°", turned.to_degrees()),
                                    format!("P {:.1}  Y {:.1}  R {:.1}", now.0, now.1, now.2),
                                    format!("was P {:.1}  Y {:.1}  R {:.1}", grab.angle.0, grab.angle.1, grab.angle.2),
                                ];
                                let anchor = on_ring(grab.ring_start + turned, 1.2).unwrap_or(o2 + vec2(14.0, -14.0));
                                gizmo_label(&p, anchor, &lines);
                            }
                        }
                    }
                }
            }
        }

        if self.show_arcs {
            let arcs = self.open.as_ref().map(|open| ad2read::arcs::of_dupe(&open.dupe, 90.0)).unwrap_or_default();
            for arc in arcs.iter().filter(|arc| !arc.unlimited) {
                let colour = if arc.elevation { colours().axis_z } else { colours().link_acf };
                let swept: Vec<egui::Pos2> = arc.points.iter().filter_map(|point| v.project(*point).map(|(at, _)| at)).collect();
                let Some((pivot, _)) = v.project(arc.pivot) else { continue };
                if swept.len() != arc.points.len() {
                    continue;
                }
                let fill = Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), 36);
                let mut fan = egui::Mesh::default();
                fan.colored_vertex(pivot, fill);
                for at in &swept {
                    fan.colored_vertex(*at, fill);
                }
                for n in 1..swept.len() as u32 {
                    fan.add_triangle(0, n, n + 1);
                }
                p.add(egui::Shape::mesh(fan));
                p.line_segment([pivot, swept[0]], Stroke::new(1.5, colour));
                p.line_segment([pivot, swept[swept.len() - 1]], Stroke::new(1.5, colour));
                p.add(egui::Shape::line(swept.clone(), Stroke::new(1.5, colour)));
                let (low, high) = arc.limits;
                p.text(swept[0], egui::Align2::LEFT_TOP, format!("{low:+.0}°"), egui::FontId::proportional(11.0), colour);
                p.text(swept[swept.len() - 1], egui::Align2::LEFT_BOTTOM, format!("{high:+.0}°"), egui::FontId::proportional(11.0), colour);
            }
        }

        if self.config.viewport.centre_of_mass {
            if let Some((at, _)) = self.balance_point().and_then(|point| v.project(point)) {
                let mark = colours().axis_hot;
                p.circle_stroke(at, 9.0, Stroke::new(2.0, mark));
                p.line_segment([at - vec2(13.0, 0.0), at + vec2(13.0, 0.0)], Stroke::new(1.5, mark));
                p.line_segment([at - vec2(0.0, 13.0), at + vec2(0.0, 13.0)], Stroke::new(1.5, mark));
                p.text(at + vec2(14.0, 10.0), egui::Align2::LEFT_TOP, "centre of mass", egui::FontId::proportional(11.0), mark);
            }
        }

        // --- picking --------------------------------------------------------
        if resp.clicked() && over_axis.is_none() {
            if let Some(at) = resp.interact_pointer_pos() {
                let mut best: Option<(f64, f32)> = None;
                for (index, bounds, depth) in &hits {
                    if bounds.contains(at) && best.map(|(_, d)| *depth < d).unwrap_or(true) {
                        best = Some((*index, *depth));
                    }
                }
                match best {
                    Some((index, _)) => {
                        if self.link_click(index) {
                        } else if ui.input(|i| i.modifiers.command) {
                            match self.marked.iter().position(|m| *m == index) {
                                Some(k) => {
                                    self.marked.remove(k);
                                }
                                None => self.marked.push(index),
                            }
                        } else if self.pick == Some(Pick::Entity(index)) {
                            self.pick = None;
                            self.crumbs.clear();
                        } else {
                            self.pick = Some(Pick::Entity(index));
                            self.crumbs.clear();
                        }
                    }
                    // Clicking empty space clears everything.
                    None => self.deselect(),
                }
            }
        }

        if resp.secondary_clicked() {
            self.menu_at = resp.interact_pointer_pos();
        }

        p.text(
            pos2(rect.left() + 8.0, rect.top() + 6.0),
            Align2::LEFT_TOP,
            "drag look  ·  shift/right-drag slide  ·  WASD/arrows fly  ·  Q/E height  ·  shift fast  ·  wheel dolly  ·  1 move  2 rotate",
            FontId::new(11.0, FontFamily::Proportional),
            colours().text_dim,
        );
    }
}

/// Two unit vectors spanning the plane an axis is normal to, for drawing a
/// rotation ring around that axis.
fn ring_basis(axis: V3) -> (V3, V3) {
    // Any vector not parallel to the axis works as a seed.
    let seed = if axis.2.abs() < 0.9 {
        (0.0, 0.0, 1.0)
    } else {
        (1.0, 0.0, 0.0)
    };
    let u = norm3(cross3(axis, seed));
    (u, norm3(cross3(axis, u)))
}

/// Shortest distance from a point to a polyline.
fn point_to_polyline(p: egui::Pos2, pts: &[egui::Pos2]) -> f32 {
    let mut best = f32::MAX;
    for w in pts.windows(2) {
        best = best.min(point_to_segment(p, w[0], w[1]));
    }
    best
}

/// A few lines of text on a dark plate beside a gizmo, readable over any
/// part of the scene.
fn gizmo_label(p: &Painter, at: egui::Pos2, lines: &[String]) {
    let galleys: Vec<_> = lines
        .iter()
        .enumerate()
        .map(|(row, line)| {
            let colour = if row == 0 { colours().axis_hot } else { colours().text };
            p.layout_no_wrap(line.clone(), ui_font(), colour)
        })
        .collect();
    let width = galleys.iter().map(|galley| galley.size().x).fold(0.0, f32::max);
    let height: f32 = galleys.iter().map(|galley| galley.size().y).sum();
    let plate = Rect::from_min_size(at, egui::vec2(width + 12.0, height + 8.0));
    p.rect_filled(plate, CornerRadius::same(4), Color32::from_black_alpha(190));
    let mut top = at.y + 4.0;
    for galley in galleys {
        let tall = galley.size().y;
        p.galley(egui::pos2(at.x + 6.0, top), galley, colours().text);
        top += tall;
    }
}

/// Shortest distance from a point to a line segment, for gizmo hit testing.
fn point_to_segment(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 < 1e-6 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

// ---------------------------------------------------------------------------
// Modifier catalogue
// ---------------------------------------------------------------------------
//
// The vanilla list is not from memory. It is every duplicator.RegisterEntityModifier
// call in Facepunch/garrysmod, with the field names each handler actually
// reads out of its data table. Verified: physprop does NOT register one — it
// calls construct.SetPhysProp, which applies to the physics object and
// persists nothing — so `mass` in a real dupe comes from an addon, not vanilla.

#[derive(Clone, Copy)]
enum Field {
    Num(f64),
    Flag(bool),
    Text(&'static str),
    Vec3,
    /// A nested table of r/g/b/a numbers, which is how a Color survives a
    /// format that has no Color type.
    Colour,
}

struct ModifierSpec {
    name: &'static str,
    note: &'static str,
    fields: &'static [(&'static str, Field)],
}

/// Every modifier vanilla Garry's Mod registers.
const VANILLA: &[ModifierSpec] = &[
    ModifierSpec {
        name: "colour",
        note: "Colour tool. RenderMode 0-9, RenderFX 0-16.",
        fields: &[
            ("Color", Field::Colour),
            ("RenderMode", Field::Num(0.0)),
            ("RenderFX", Field::Num(0.0)),
        ],
    },
    ModifierSpec {
        name: "material",
        note: "Material tool. Must be in the OverrideMaterials list on a server.",
        fields: &[("MaterialOverride", Field::Text(""))],
    },
    ModifierSpec {
        name: "trail",
        note: "Trails tool.",
        fields: &[
            ("Color", Field::Colour),
            ("StartSize", Field::Num(32.0)),
            ("EndSize", Field::Num(0.0)),
            ("Length", Field::Num(5.0)),
            ("Material", Field::Text("trails/laser")),
        ],
    },
    ModifierSpec {
        name: "eyetarget",
        note: "Eye Poser tool.",
        fields: &[("EyeTarget", Field::Vec3)],
    },
    ModifierSpec {
        name: "gravity_property",
        note: "The gravity right-click property.",
        fields: &[("enabled", Field::Flag(true))],
    },
    ModifierSpec {
        name: "decal1",
        note: "Paint tool. Vanilla numbers them decal1, decal2, and so on.",
        fields: &[
            ("decal", Field::Text("Blood")),
            ("Pos1", Field::Vec3),
            ("Pos2", Field::Vec3),
            ("bone", Field::Num(0.0)),
        ],
    },
];

/// Modifiers that are not vanilla but turn up constantly in real builds, with
/// the shapes seen in an actual dupe. Offered separately so it stays obvious
/// which ones Garry's Mod itself knows about.
const COMMON_ADDON: &[ModifierSpec] = &[
    ModifierSpec {
        name: "Fading Door",
        note: "Fading Door tool (note the space in the name). key is a numpad key number; a plain keypad drives it by KeyGranted matching key.",
        fields: &[
            ("key", Field::Num(111.0)),
            ("toggle", Field::Flag(true)),
            ("reversed", Field::Flag(false)),
            ("CanDisableMotion", Field::Flag(true)),
            ("DoorMaterial", Field::Text("sprites/heatwave")),
            ("DoorOpenSound", Field::Num(0.0)),
            ("DoorCloseSound", Field::Num(0.0)),
            ("DoorLoopSound", Field::Num(0.0)),
        ],
    },
    ModifierSpec {
        name: "submaterial",
        note: "Submaterial tool: SubMaterialOverride_<slot> = material path, one per model material slot.",
        fields: &[("SubMaterialOverride_0", Field::Text("models/debug/debugwhite"))],
    },
    ModifierSpec {
        name: "keypad_password_passthrough",
        note: "Keypad tool. Password is a number (or false for none). KeyGranted/KeyDenied are numpad keys it presses; match a fading door's key.",
        fields: &[
            ("Password", Field::Num(1337.0)),
            ("Secure", Field::Flag(false)),
            ("KeyGranted", Field::Num(111.0)),
            ("KeyDenied", Field::Num(0.0)),
            ("RepeatsGranted", Field::Num(0.0)),
            ("RepeatsDenied", Field::Num(0.0)),
            ("DelayGranted", Field::Num(0.0)),
            ("DelayDenied", Field::Num(0.0)),
            ("InitDelayGranted", Field::Num(0.0)),
            ("InitDelayDenied", Field::Num(0.0)),
            ("LengthGranted", Field::Num(0.0)),
            ("LengthDenied", Field::Num(0.0)),
        ],
    },
    ModifierSpec {
        name: "keypad_wire_password_passthrough",
        note: "Wire keypad: same as keypad plus OutputOn/OutputOff; drives things by wire, not by numpad key.",
        fields: &[
            ("Password", Field::Num(1337.0)),
            ("Secure", Field::Flag(false)),
            ("OutputOn", Field::Num(1.0)),
            ("OutputOff", Field::Num(0.0)),
            ("RepeatsGranted", Field::Num(0.0)),
            ("RepeatsDenied", Field::Num(0.0)),
            ("DelayGranted", Field::Num(0.0)),
            ("DelayDenied", Field::Num(0.0)),
            ("InitDelayGranted", Field::Num(0.0)),
            ("InitDelayDenied", Field::Num(0.0)),
            ("LengthGranted", Field::Num(0.0)),
            ("LengthDenied", Field::Num(0.0)),
        ],
    },
    ModifierSpec {
        name: "mass",
        note: "Weight tool (addon). Not vanilla; physprop persists nothing.",
        fields: &[("Mass", Field::Num(100.0))],
    },
    ModifierSpec {
        name: "ACF_Armor",
        note: "ACF armour properties.",
        fields: &[("Thickness", Field::Num(10.0)), ("Ductility", Field::Num(0.0))],
    },
    ModifierSpec {
        name: "buoyancy",
        note: "Buoyancy ratio, 0 sinks and 1 floats.",
        fields: &[("Ratio", Field::Num(1.0))],
    },
    ModifierSpec {
        name: "Unbreakable",
        note: "Unbreakable tool.",
        fields: &[("On", Field::Flag(true))],
    },
    ModifierSpec {
        name: "inertia",
        note: "Inertia tool.",
        fields: &[
            ("defaultInertia", Field::Vec3),
            ("inertiaLock", Field::Vec3),
            ("inertiaToSet", Field::Vec3),
        ],
    },
    ModifierSpec {
        name: "physparent",
        note: "Parenting tool.",
        fields: &[
            ("childcount", Field::Num(0.0)),
            ("enhanced", Field::Flag(false)),
            ("group", Field::Text("")),
            ("shadows", Field::Flag(false)),
            ("weight", Field::Flag(false)),
        ],
    },
];

impl App {
    /// Builds the Value for one field, creating a nested table where needed.
    fn field_value(&mut self, f: Field) -> Value {
        match f {
            Field::Num(n) => Value::Number(n),
            Field::Flag(b) => Value::Bool(b),
            Field::Text(t) => Value::Str(t.as_bytes().to_vec()),
            Field::Vec3 => Value::Vector(0.0, 0.0, 0.0),
            Field::Colour => {
                let Some(open) = self.open.as_mut() else {
                    return Value::Nil;
                };
                let t = open.dupe.new_table();
                for k in ["r", "g", "b", "a"] {
                    open.dupe.set(t, k, Value::Number(255.0));
                }
                Value::Table(t)
            }
        }
    }

    /// Adds a modifier to an entity, creating its EntityMods table if the
    /// entity has none. Refuses to clobber one that's already there.
    fn add_modifier(&mut self, entity: f64, name: &str, fields: &[(&str, Field)]) {
        let et = {
            let Some(open) = self.open.as_ref() else { return };
            match transform::entity_table(&open.dupe, entity) {
                Some(t) => t,
                None => return self.bad(format!("no entity {entity}")),
            }
        };

        self.snapshot();

        // EntityMods may not exist yet — 20 of the 36 entities in a real dupe
        // had none at all.
        let mods = {
            let Some(open) = self.open.as_mut() else { return };
            match open.dupe.get_table(et, "EntityMods") {
                Some(m) => m,
                None => {
                    let m = open.dupe.new_table();
                    open.dupe.set(et, "EntityMods", Value::Table(m));
                    m
                }
            }
        };

        if self
            .open
            .as_ref()
            .and_then(|o| o.dupe.get_table(mods, name))
            .is_some()
        {
            return self.say(format!("entity {entity} already has a `{name}` modifier"));
        }

        let table = {
            let Some(open) = self.open.as_mut() else { return };
            open.dupe.new_table()
        };
        for (key, kind) in fields {
            let v = self.field_value(*kind);
            if let Some(open) = self.open.as_mut() {
                open.dupe.set(table, key, v);
            }
        }
        if let Some(open) = self.open.as_mut() {
            open.dupe.set(mods, name, Value::Table(table));
            open.dirty = true;
        }
        self.good(format!("added `{name}` to entity {entity}"));
    }

    fn remove_modifier(&mut self, entity: f64, name: &str) {
        let Some(et) = ({
            let Some(open) = self.open.as_ref() else { return };
            transform::entity_table(&open.dupe, entity)
        }) else {
            return;
        };
        self.snapshot();
        let Some(open) = self.open.as_mut() else { return };
        let Some(mods) = open.dupe.get_table(et, "EntityMods") else {
            return;
        };
        if let Node::Table(entries) = &mut open.dupe.arena[mods] {
            let before = entries.len();
            entries.retain(|(k, _)| key_label(k) != name);
            if entries.len() < before {
                open.dirty = true;
                self.good(format!("removed `{name}` from entity {entity}"));
                return;
            }
        }
        self.say(format!("entity {entity} has no `{name}` modifier"));
    }

    /// The "Add modifier" submenu.
    fn add_modifier_menu(&mut self, ui: &mut Ui, entity: f64) {
        ui.label(RichText::new("VANILLA GMOD").color(colours().accent).small());
        for spec in VANILLA {
            let r = ui.button(spec.name);
            if r.clicked() {
                self.add_modifier(entity, spec.name, spec.fields);
                self.menu_at = None;
            }
            r.on_hover_text(spec.note);
        }

        ui.separator();
        ui.label(RichText::new("COMMON ADDONS").color(colours().accent).small());
        for spec in COMMON_ADDON {
            let r = ui.button(spec.name);
            if r.clicked() {
                self.add_modifier(entity, spec.name, spec.fields);
                self.menu_at = None;
            }
            r.on_hover_text(spec.note);
        }

        ui.separator();
        ui.label(RichText::new("CUSTOM").color(colours().accent).small());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_mod_name)
                    .hint_text("modifier name")
                    .desired_width(150.0),
            );
        });
        let named = !self.new_mod_name.trim().is_empty();
        if ui
            .add_enabled(named, egui::Button::new("Add empty modifier"))
            .clicked()
        {
            let name = self.new_mod_name.trim().to_owned();
            self.add_modifier(entity, &name, &[]);
            self.new_mod_name.clear();
            self.menu_at = None;
        }
        ui.label(
            RichText::new(
                "An unregistered name is inert: GMod applies modifiers by \
                 looking the name up, so anything it doesn't know is ignored \
                 on paste rather than erroring.",
            )
            .color(colours().text_dim)
            .small(),
        );
    }

    /// Adds a field to an existing modifier, so a custom one is usable.
    fn add_field_row(&mut self, ui: &mut Ui, table: usize) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_field_name)
                    .hint_text("field")
                    .desired_width(110.0),
            );
            egui::ComboBox::from_id_salt("new_field_kind")
                .selected_text(self.new_field_kind)
                .width(90.0)
                .show_ui(ui, |ui| {
                    for k in ["number", "bool", "string", "Vector", "Angle", "colour"] {
                        ui.selectable_value(&mut self.new_field_kind, k, k);
                    }
                });
            let named = !self.new_field_name.trim().is_empty();
            if ui.add_enabled(named, egui::Button::new("add")).clicked() {
                let name = self.new_field_name.trim().to_owned();
                let kind = self.new_field_kind;
                self.snapshot();
                let v = match kind {
                    "bool" => Value::Bool(false),
                    "string" => Value::Str(Vec::new()),
                    "Vector" => Value::Vector(0.0, 0.0, 0.0),
                    "Angle" => Value::Angle(0.0, 0.0, 0.0),
                    "colour" => self.field_value(Field::Colour),
                    _ => Value::Number(0.0),
                };
                if let Some(open) = self.open.as_mut() {
                    open.dupe.set(table, &name, v);
                    open.dirty = true;
                }
                self.new_field_name.clear();
            }
        });
    }
}
