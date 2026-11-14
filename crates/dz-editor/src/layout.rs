//! Where the panels of the window go: each docks left, right or at the bottom, floats in a
//! window of its own or is hidden, and the default layout can always be brought back. In full
//! screen they slide in at the edge they are docked to, and a pinned one stays.
use crate::app::App;
use crate::i18n::t;
use crate::icons;
use crate::theme;
use eframe::egui::{self, pos2, vec2, Rect};
use serde::{Deserialize, Serialize};

/// Where a panel is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Dock {
    Left,
    Right,
    Bottom,
    Window,
    Hidden,
}

impl Dock {
    pub const ALL: [Dock; 5] = [Dock::Left, Dock::Right, Dock::Bottom, Dock::Window, Dock::Hidden];
    fn name(self) -> &'static str {
        match self {
            Dock::Left => "Left",
            Dock::Right => "Right",
            Dock::Bottom => "Bottom",
            Dock::Window => "Separate window",
            Dock::Hidden => "Hidden",
        }
    }
}

/// The panels that can be moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pane {
    /// Tools and their settings.
    Tools,
    /// Outliner and details.
    Scene,
    /// Content browser and log.
    Content,
}

impl Pane {
    pub const ALL: [Pane; 3] = [Pane::Tools, Pane::Scene, Pane::Content];
    pub fn title(self) -> &'static str {
        match self {
            Pane::Tools => "Tools",
            Pane::Scene => "Outliner and details",
            Pane::Content => "Content browser and log",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Pane::Tools => "tools",
            Pane::Scene => "scene",
            Pane::Content => "content",
        }
    }
    /// Panels without a project have nothing to show, except the content browser.
    fn needs_project(self) -> bool {
        self != Pane::Content
    }
    /// Default width as a side panel, and the widths it may take.
    fn side_width(self) -> (f32, std::ops::RangeInclusive<f32>) {
        match self {
            Pane::Tools => (300.0, 240.0..=460.0),
            Pane::Scene => (320.0, 240.0..=520.0),
            Pane::Content => (420.0, 280.0..=900.0),
        }
    }
    /// Default height at the bottom, and the heights it may take.
    fn bottom_height(self) -> (f32, std::ops::RangeInclusive<f32>) {
        match self {
            Pane::Content => (230.0, 90.0..=600.0),
            _ => (260.0, 120.0..=600.0),
        }
    }
}

/// The layout of the window, kept with the settings.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Layout {
    pub tools: Dock,
    pub scene: Dock,
    pub content: Dock,
    /// Panels kept on screen in full screen: the menu bar, then the panels in `Pane::ALL` order.
    pub pinned: [bool; 4],
    /// Bumped by going back to the default layout, so panels forget the sizes they were given.
    pub generation: u32,
}

impl Default for Layout {
    fn default() -> Self {
        Self { tools: Dock::Left, scene: Dock::Right, content: Dock::Bottom, pinned: [false; 4], generation: 0 }
    }
}

impl Layout {
    pub fn dock(&self, p: Pane) -> Dock {
        match p {
            Pane::Tools => self.tools,
            Pane::Scene => self.scene,
            Pane::Content => self.content,
        }
    }
    fn dock_mut(&mut self, p: Pane) -> &mut Dock {
        match p {
            Pane::Tools => &mut self.tools,
            Pane::Scene => &mut self.scene,
            Pane::Content => &mut self.content,
        }
    }
    fn pin_index(p: Pane) -> usize {
        1 + Pane::ALL.iter().position(|q| *q == p).unwrap()
    }
    /// The same panels where they are by default; pins stay.
    fn is_default(&self) -> bool {
        let d = Layout::default();
        (self.tools, self.scene, self.content) == (d.tools, d.scene, d.content)
    }
}

fn id(app: &App, p: Pane, dock: Dock) -> egui::Id {
    egui::Id::new(("pane", p.key(), dock, app.settings.layout.generation))
}

/// The panel's contents.
pub fn pane_ui(app: &mut App, ui: &mut egui::Ui, p: Pane) {
    match p {
        Pane::Tools => crate::panels::modes_panel(app, ui),
        Pane::Scene => crate::panels::right_panel(app, ui),
        Pane::Content => crate::panels::bottom_panel(app, ui),
    }
}

fn shown(app: &App, p: Pane) -> bool {
    !p.needs_project() || app.doc.is_some()
}

/// The docked panels of the window, around the view: sides first, then the bottom.
pub fn docked(app: &mut App, ctx: &egui::Context) {
    for dock in [Dock::Left, Dock::Right] {
        for p in Pane::ALL {
            if app.settings.layout.dock(p) != dock || !shown(app, p) { continue; }
            let (w, range) = p.side_width();
            let panel = if dock == Dock::Left { egui::SidePanel::left(id(app, p, dock)) } else { egui::SidePanel::right(id(app, p, dock)) };
            panel.resizable(true).default_width(w).width_range(range).show(ctx, |ui| pane_ui(app, ui, p));
        }
    }
    for p in Pane::ALL {
        if app.settings.layout.dock(p) != Dock::Bottom || !shown(app, p) { continue; }
        let (h, range) = p.bottom_height();
        egui::TopBottomPanel::bottom(id(app, p, Dock::Bottom)).resizable(true).default_height(h).height_range(range).show(ctx, |ui| pane_ui(app, ui, p));
    }
}

/// The panels floating in windows of their own; closing one hides it.
pub fn windows(app: &mut App, ctx: &egui::Context) {
    for p in Pane::ALL {
        if app.settings.layout.dock(p) != Dock::Window || !shown(app, p) { continue; }
        let mut open = true;
        let size = match p { Pane::Content => vec2(760.0, 300.0), _ => vec2(320.0, 560.0) };
        egui::Window::new(t(p.title())).id(id(app, p, Dock::Window)).default_size(size).resizable(true).collapsible(true).open(&mut open).show(ctx, |ui| pane_ui(app, ui, p));
        if !open {
            *app.settings.layout.dock_mut(p) = Dock::Hidden;
            app.settings.save();
        }
    }
}

/// The Window layout submenu: where each panel goes, and back to the default.
pub fn menu(app: &mut App, ui: &mut egui::Ui) {
    ui.menu_button(icons::with(icons::LAYOUT, &t("Window layout")), |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let mut changed = false;
        for p in Pane::ALL {
            ui.menu_button(t(p.title()), |ui| {
                let now = app.settings.layout.dock(p);
                for d in Dock::ALL {
                    if ui.radio(now == d, t(d.name())).clicked() {
                        *app.settings.layout.dock_mut(p) = d;
                        changed = true;
                        ui.close_menu();
                    }
                }
            });
        }
        ui.separator();
        let default = app.settings.layout.is_default() && app.settings.layout.generation == 0;
        if ui.add_enabled(!default, egui::Button::new(icons::with(icons::ARROW_COUNTER_CLOCKWISE, &t("Default layout")))).on_hover_text(t("Puts every panel back where it was at first, at its first size.")).clicked() {
            reset(app);
            ui.close_menu();
        }
        if changed {
            app.settings.save();
        }
    });
}

/// Every panel back where and as big as it was at first; pins stay.
pub fn reset(app: &mut App) {
    let l = &mut app.settings.layout;
    let generation = l.generation + 1;
    *l = Layout { pinned: l.pinned, generation, ..Default::default() };
    app.settings.save();
}

/// A pin in the corner of a full screen panel: pinned, it stays on screen.
fn pin(ui: &mut egui::Ui, pinned: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let icon = if *pinned { icons::PUSH_PIN } else { icons::PUSH_PIN_SLASH };
            let tip = if *pinned { t("Pinned: stays on screen. Click to let it slide away again.") } else { t("Pin: keep this panel on screen.") };
            let text = egui::RichText::new(icon).color(if *pinned { theme::ACCENT } else { egui::Color32::from_gray(150) });
            if ui.add(egui::Button::new(text).small().frame(*pinned)).on_hover_text(tip).clicked() {
                *pinned = !*pinned;
                changed = true;
            }
        });
    });
    changed
}

/// The panels of the full screen view, each sliding in over the view while the pointer is at
/// the edge it is docked to or over the panel, or staying while pinned: the menu and tools at
/// the top, the others at their edge, panels in windows as windows.
pub fn edge_panels(app: &mut App, ctx: &egui::Context) {
    const EDGE: f32 = 4.0;
    let screen = ctx.screen_rect();
    let pointer = ctx.pointer_latest_pos();
    // A menu, a drag or a typed field keeps its panel open.
    let busy = ctx.is_using_pointer() || egui::DragAndDrop::has_any_payload(ctx) || ctx.wants_keyboard_input() || ctx.memory(|m| m.any_popup_open());
    let at_edge = |k: usize| pointer.is_some_and(|p| match k {
        0 => p.y <= screen.top() + EDGE,
        1 => p.x <= screen.left() + EDGE,
        2 => p.x >= screen.right() - EDGE,
        _ => p.y >= screen.bottom() - EDGE,
    });
    let layout = app.settings.layout.clone();
    // Panels by edge: 1 left, 2 right, 3 bottom.
    let on = |k: usize| -> Vec<Pane> {
        let d = match k { 1 => Dock::Left, 2 => Dock::Right, _ => Dock::Bottom };
        Pane::ALL.into_iter().filter(|p| layout.dock(*p) == d && shown(app, *p)).collect()
    };
    let top = Rect::from_min_size(screen.min, vec2(screen.width(), 96.0));
    // Side panels start under the menu bar when it is pinned.
    let below = if layout.pinned[0] { top.bottom() + 4.0 } else { screen.top() + 40.0 };
    let mut rects: Vec<(Pane, usize, Rect)> = Vec::new();
    for k in 1..=3 {
        let panes = on(k);
        for (i, p) in panes.iter().enumerate() {
            let r = match k {
                1 => { let w = p.side_width().0 + 20.0; let x = screen.left() + i as f32 * (w + 4.0); Rect::from_min_max(pos2(x, below), pos2(x + w, screen.bottom() - 40.0)) }
                2 => { let w = p.side_width().0 + 20.0; let x = screen.right() - i as f32 * (w + 4.0); Rect::from_min_max(pos2(x - w, below), pos2(x, screen.bottom() - 40.0)) }
                _ => {
                    let n = panes.len() as f32;
                    let width = (screen.width() - 80.0 - (n - 1.0) * 4.0) / n;
                    let x = screen.left() + 40.0 + i as f32 * (width + 4.0);
                    Rect::from_min_max(pos2(x, screen.bottom() - 280.0), pos2(x + width, screen.bottom()))
                }
            };
            rects.push((*p, k, r));
        }
    }
    for k in 0..4 {
        let area = if k == 0 { vec![top] } else { rects.iter().filter(|r| r.1 == k).map(|r| r.2).collect() };
        if area.is_empty() { app.edge_open[k] = false; continue; }
        let inside = pointer.is_some_and(|p| area.iter().any(|r| r.expand(12.0).contains(p)));
        if at_edge(k) {
            app.edge_open[k] = true;
        } else if app.edge_open[k] && !inside && !busy {
            app.edge_open[k] = false;
        }
    }
    let frame = egui::Frame::new().fill(theme::PANEL).stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_gray(60))).inner_margin(egui::Margin::same(6)).corner_radius(4.0);
    let area = |key: &str, r: Rect| egui::Area::new(egui::Id::new(("edge", key))).order(egui::Order::Foreground).fixed_pos(r.min).constrain(false);
    let mut pins_changed = false;
    for (p, k, r) in rects.iter().copied() {
        let slot = Layout::pin_index(p);
        if !(app.edge_open[k] || app.settings.layout.pinned[slot]) { continue; }
        area(p.key(), r).show(ctx, |ui| frame.show(ui, |ui| {
            ui.set_min_size(r.size() - vec2(12.0, 12.0));
            ui.set_max_size(r.size() - vec2(12.0, 12.0));
            pins_changed |= pin(ui, &mut app.settings.layout.pinned[slot]);
            pane_ui(app, ui, p);
        }));
    }
    if app.edge_open[0] || app.settings.layout.pinned[0] {
        area("top", top).show(ctx, |ui| frame.show(ui, |ui| {
            ui.set_min_width(top.width() - 12.0);
            ui.set_max_width(top.width() - 12.0);
            pins_changed |= pin(ui, &mut app.settings.layout.pinned[0]);
            crate::panels::menu_contents(app, ui);
            crate::panels::toolbar_contents(app, ui);
        }));
    }
    if pins_changed {
        app.settings.save();
    }
    windows(app, ctx);
}

/// True when no panel of the full screen view is on screen, for the hint where they are.
pub fn none_open(app: &App) -> bool {
    !app.edge_open.iter().any(|o| *o) && !app.settings.layout.pinned.iter().any(|p| *p)
}
