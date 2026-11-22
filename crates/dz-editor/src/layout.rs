//! Where the panels of the window go. Each panel is a tab, as in Unreal and Workbench: tabs are
//! dragged to another panel, to an edge of one to split it, or out of the window to float, and
//! the default layout can always be brought back. In full screen the panels slide in at the
//! edges instead (tools left, outliner and details right, the rest at the bottom), and a pinned
//! one stays.
use crate::app::{App, BottomTab};
use crate::i18n::t;
use crate::icons;
use crate::theme;
use eframe::egui::{self, pos2, vec2, Rect};
use egui_dock::{DockArea, DockState, NodeIndex};
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
}

/// A panel of the window, shown as a tab. Tabs are dragged by their title to another panel,
/// to an edge of a panel to split it, or out of the window to float.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    Viewport,
    Tools,
    Outliner,
    Details,
    Content,
    Output,
    Build,
}

impl Tab {
    pub const ALL: [Tab; 7] = [Tab::Viewport, Tab::Tools, Tab::Outliner, Tab::Details, Tab::Content, Tab::Output, Tab::Build];
    pub fn name(self) -> &'static str {
        match self {
            Tab::Viewport => "Viewport",
            Tab::Tools => "Tools",
            Tab::Outliner => "Outliner",
            Tab::Details => "Details",
            Tab::Content => "Content Browser",
            Tab::Output => "Output Log",
            Tab::Build => "Build",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Tab::Viewport => icons::MONITOR,
            Tab::Tools => icons::WRENCH,
            Tab::Outliner => icons::TREE_STRUCTURE,
            Tab::Details => icons::LIST_DASHES,
            Tab::Content => icons::SQUARES_FOUR,
            Tab::Output => icons::TERMINAL_WINDOW,
            Tab::Build => icons::HAMMER,
        }
    }
    /// Panels without a project have nothing to show, except the view and the content browser.
    fn needs_project(self) -> bool {
        matches!(self, Tab::Tools | Tab::Outliner | Tab::Details)
    }
    fn bottom(self) -> Option<BottomTab> {
        match self {
            Tab::Content => Some(BottomTab::Content),
            Tab::Output => Some(BottomTab::Output),
            Tab::Build => Some(BottomTab::Build),
            _ => None,
        }
    }
    fn of_bottom(b: BottomTab) -> Tab {
        match b {
            BottomTab::Content => Tab::Content,
            BottomTab::Output => Tab::Output,
            BottomTab::Build => Tab::Build,
        }
    }
}

/// The layout of the window, kept with the settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    /// Where the three panels of older versions went. Read once to lay out the tabs the first
    /// time, and kept so that an older version still finds its panels.
    pub tools: Dock,
    pub scene: Dock,
    pub content: Dock,
    /// Panels kept on screen in full screen: the menu bar, then the panels in `Pane::ALL` order.
    pub pinned: [bool; 4],
    /// Bumped by going back to the default layout, so panels forget the sizes they were given.
    pub generation: u32,
    /// The tabs and how they are split. A layout this version cannot read (one saved by a newer
    /// version with tabs it does not know) is dropped for the default one, never the settings.
    #[serde(deserialize_with = "lenient_dock", serialize_with = "save_dock", skip_serializing_if = "Option::is_none")]
    pub dock: Option<DockState<Tab>>,
}

impl Default for Layout {
    fn default() -> Self {
        Self { tools: Dock::Left, scene: Dock::Right, content: Dock::Bottom, pinned: [false; 4], generation: 0, dock: None }
    }
}

fn lenient_dock<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<DockState<Tab>>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value::<DockState<Tab>>(v).ok().filter(valid))
}

/// The layout as saved. Panels and splits not drawn yet have endless rectangles, which JSON
/// writes as null and cannot read back, so those numbers are saved as 0; the rectangles are
/// worked out again when the panels are drawn.
fn save_dock<S: serde::Serializer>(dock: &Option<DockState<Tab>>, s: S) -> Result<S::Ok, S::Error> {
    fn fix(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(m) => {
                if m.contains_key("x") && m.contains_key("y") {
                    for k in ["x", "y"] {
                        if m[k].is_null() { m[k] = 0.0.into(); }
                    }
                }
                m.values_mut().for_each(fix);
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(fix),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(dock).map_err(serde::ser::Error::custom)?;
    fix(&mut v);
    v.serialize(s)
}

/// A layout holds the view once and no tab twice.
fn valid(s: &DockState<Tab>) -> bool {
    let mut seen = std::collections::HashSet::new();
    s.iter_all_tabs().all(|(_, t)| seen.insert(*t)) && s.find_main_surface_tab(&Tab::Viewport).is_some()
}

impl Layout {
    /// Where a panel slides in from in full screen.
    pub fn dock(&self, p: Pane) -> Dock {
        let d = Layout::default();
        match p {
            Pane::Tools => d.tools,
            Pane::Scene => d.scene,
            Pane::Content => d.content,
        }
    }
    fn pin_index(p: Pane) -> usize {
        1 + Pane::ALL.iter().position(|q| *q == p).unwrap()
    }
}

/// The tabs as they are at first: tools on the left, outliner over details on the right, the
/// content browser, log and build under the view. Panels an older version had elsewhere start
/// there.
pub fn default_dock(old: &Layout) -> DockState<Tab> {
    let mut s = DockState::new(vec![Tab::Viewport]);
    let mut bottom = Vec::new();
    let mut windows = Vec::new();
    let tree = s.main_surface_mut();
    let mut centre = NodeIndex::root();
    let mut side = |dock: Dock, tabs: Vec<Tab>, stacked: bool| {
        let (first, rest) = if stacked { (vec![tabs[0]], tabs[1..].to_vec()) } else { (tabs.clone(), Vec::new()) };
        let node = match dock {
            Dock::Left => { let [c, n] = tree.split_left(centre, 0.19, first); centre = c; n }
            Dock::Right => { let [c, n] = tree.split_right(centre, 0.74, first); centre = c; n }
            Dock::Bottom => { bottom.extend(tabs); return; }
            Dock::Window => { windows.push(tabs); return; }
            Dock::Hidden => return,
        };
        if !rest.is_empty() { tree.split_below(node, 0.42, rest); }
    };
    side(old.tools, vec![Tab::Tools], false);
    side(old.scene, vec![Tab::Outliner, Tab::Details], true);
    side(old.content, vec![Tab::Content, Tab::Output, Tab::Build], false);
    if !bottom.is_empty() { tree.split_below(centre, 0.70, bottom); }
    for w in windows { s.add_window(w); }
    s
}

/// Shows a tab, opening it again next to its usual neighbours if it was closed.
fn show_tab(s: &mut DockState<Tab>, tab: Tab) {
    if let Some(at) = s.find_tab(&tab) {
        s.set_active_tab(at);
        return;
    }
    let neighbours: &[Tab] = match tab {
        Tab::Content | Tab::Output | Tab::Build => &[Tab::Content, Tab::Output, Tab::Build],
        Tab::Outliner | Tab::Details => &[Tab::Details, Tab::Outliner],
        _ => &[],
    };
    let tree = s.main_surface_mut();
    for n in neighbours {
        if let Some((node, _)) = tree.find_tab(n) {
            tree[node].append_tab(tab);
            return;
        }
    }
    let Some((view, _)) = tree.find_tab(&Tab::Viewport) else { return };
    match tab {
        Tab::Tools => { tree.split_left(view, 0.22, vec![tab]); }
        Tab::Outliner | Tab::Details => { tree.split_right(view, 0.72, vec![tab]); }
        _ => { tree.split_below(view, 0.70, vec![tab]); }
    }
}

struct Viewer<'a> {
    app: &'a mut App,
    /// Where each tab title was drawn this frame, for the marks drawn over them.
    buttons: Vec<(Tab, Rect)>,
    clicked: Option<Tab>,
}

impl egui_dock::TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        let (icon, name) = match tab {
            // The tools tab is named after the mode it holds the tools of.
            Tab::Tools => crate::panels::MODES.iter().find(|m| m.0 == self.app.mode).map(|m| (m.1, t(m.2))).unwrap_or((tab.icon(), t(tab.name()))),
            _ => (tab.icon(), t(tab.name())),
        };
        let mut text = icons::with(icon, &name);
        if *tab == Tab::Output {
            // Room for the mark of errors and warnings drawn over the title.
            let (errors, warnings) = unseen(self.app);
            if errors + warnings > 0 { text.push_str(&" ".repeat(6 + (errors + warnings).to_string().len() * 2)); }
        }
        text.into()
    }

    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(("dock_tab", *tab))
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        tab_ui(self.app, ui, *tab);
    }

    fn closeable(&mut self, tab: &mut Tab) -> bool {
        *tab != Tab::Viewport
    }

    fn allowed_in_windows(&self, tab: &mut Tab) -> bool {
        *tab != Tab::Viewport
    }

    fn on_tab_button(&mut self, tab: &mut Tab, response: &egui::Response) {
        self.buttons.push((*tab, response.rect));
        if response.clicked() || response.drag_started() { self.clicked = Some(*tab); }
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }

    fn tab_style_override(&self, tab: &Tab, global: &egui_dock::TabStyle) -> Option<egui_dock::TabStyle> {
        // Lists, details and the browser keep a little room from the edge; the view and the
        // tools run their bars from edge to edge.
        let margin = match tab {
            Tab::Viewport | Tab::Tools => return None,
            Tab::Outliner => egui::Margin::symmetric(4, 2),
            Tab::Details => egui::Margin::symmetric(6, 4),
            _ => egui::Margin::same(4),
        };
        let mut s = global.clone();
        s.tab_body.inner_margin = margin;
        Some(s)
    }
}

/// Errors and warnings the log took in since it was last on screen.
fn unseen(app: &App) -> (usize, usize) {
    let unseen = &app.log[app.log_seen.min(app.log.len())..];
    (unseen.iter().filter(|l| l.level >= 2).count(), unseen.iter().filter(|l| l.level == 1).count())
}

/// What a tab holds.
fn tab_ui(app: &mut App, ui: &mut egui::Ui, tab: Tab) {
    if tab.needs_project() && app.doc.is_none() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| ui.label(egui::RichText::new(t("Open or create a project")).color(theme::TEXT_DIM)));
        return;
    }
    match tab {
        Tab::Viewport => {
            if app.detached { crate::panels::detached_placeholder(app, ui); } else { crate::viewport::central(app, ui); }
        }
        Tab::Tools => crate::panels::modes_panel(app, ui),
        Tab::Outliner => crate::panels::outliner_tab(app, ui),
        Tab::Details => crate::panels::details_tab(app, ui),
        Tab::Content => crate::panels::content_tab(app, ui),
        Tab::Output => {
            app.log_seen = app.log.len();
            crate::panels::output_log(app, ui);
        }
        Tab::Build => crate::panels::build_tab(app, ui),
    }
}

/// The panels of the window as tabs around the view, in the space the menu, toolbar and status
/// bar leave.
pub fn docked(app: &mut App, ctx: &egui::Context) {
    // A copy, so that the settings saved by a panel while it is drawn still hold the layout.
    let mut state = app.settings.layout.dock.clone().unwrap_or_else(|| default_dock(&app.settings.layout));
    // A tab of the bottom asked for elsewhere (a build shows the Build tab) comes to the front.
    let asked = egui::Id::new("dock_bottom_asked");
    if ctx.data(|d| d.get_temp::<BottomTab>(asked)) != Some(app.bottom) {
        show_tab(&mut state, Tab::of_bottom(app.bottom));
        ctx.data_mut(|d| d.insert_temp(asked, app.bottom));
    }
    // The menu of a tab (right click on its title) in the language of the interface.
    state.translations.tab_context_menu.close_button = t("Close");
    state.translations.tab_context_menu.eject_button = t("Float in a window");
    state.translations.leaf.close_button_disabled_tooltip = t("The view cannot be closed.");
    egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::GAP).inner_margin(egui::Margin::same(3))).show(ctx, |ui| {
        let generation = app.settings.layout.generation;
        let mut viewer = Viewer { app: &mut *app, buttons: Vec::new(), clicked: None };
        DockArea::new(&mut state)
            .id(egui::Id::new(("dock", generation)))
            .style(theme::dock_style(ui.style()))
            .show_leaf_collapse_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_close_buttons(false)
            .show_inside(ui, &mut viewer);
        let Viewer { buttons, clicked, .. } = viewer;
        if let Some(b) = clicked.and_then(Tab::bottom) {
            app.bottom = b;
            ui.ctx().data_mut(|d| d.insert_temp(asked, b));
        }
        decorate(app, ui, &state, &buttons);
    });
    // Saved once a drag of a tab or a splitter is over.
    if !ctx.input(|i| i.pointer.any_down()) {
        let now = shape_of(&state);
        let key = egui::Id::new("dock_saved");
        let before = ctx.data(|d| d.get_temp::<std::sync::Arc<String>>(key));
        if before.as_deref() != Some(&now) {
            ctx.data_mut(|d| d.insert_temp(key, std::sync::Arc::new(now)));
            if before.is_some() {
                app.settings.layout.dock = Some(state.clone());
                app.settings.save();
            }
        }
    }
    app.settings.layout.dock = Some(state);
}

/// What a layout is without where its panels happened to be drawn: the tabs, the splits and
/// their sizes. A change of it is saved; resizing the window alone is not.
fn shape_of(state: &DockState<Tab>) -> String {
    fn strip(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(m) => {
                m.remove("rect");
                m.remove("viewport");
                m.values_mut().for_each(strip);
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(Layout { dock: Some(state.clone()), ..Default::default() }).unwrap_or_default();
    strip(&mut v);
    v.to_string()
}

/// Marks over the tabs: a line on top of the tab in front of each panel (blue, yellow for the
/// view), a yellow frame round the view, and the count of new errors and warnings on the log.
fn decorate(app: &App, ui: &egui::Ui, state: &DockState<Tab>, buttons: &[(Tab, Rect)]) {
    let p = ui.painter();
    for node in state.main_surface().iter() {
        let egui_dock::Node::Leaf { tabs, active, viewport, .. } = node else { continue };
        let Some(front) = tabs.get(active.0) else { continue };
        let view = *front == Tab::Viewport;
        if let Some((_, r)) = buttons.iter().find(|(t, _)| t == front) {
            p.hline(r.x_range(), r.top() + 1.0, egui::Stroke::new(2.0_f32, if view { theme::FOCUS } else { theme::ACCENT }));
        }
        if view && !app.detached && app.doc.is_some() {
            p.rect_stroke(*viewport, 0.0, egui::Stroke::new(1.0_f32, theme::FOCUS), egui::StrokeKind::Inside);
        }
    }
    let (errors, warnings) = unseen(app);
    if errors + warnings > 0 {
        if let Some((_, r)) = buttons.iter().find(|(t, _)| *t == Tab::Output) {
            let (n, color, glyph) = if errors > 0 { (errors, theme::ERROR, icons::X_CIRCLE) } else { (warnings, theme::WARN, icons::WARNING) };
            let galley = p.layout_no_wrap(format!("{glyph} {n}"), egui::FontId::proportional(11.0), egui::Color32::BLACK);
            let pill = Rect::from_min_size(pos2(r.right() - galley.size().x - 16.0, r.center().y - 7.5), vec2(galley.size().x + 8.0, 15.0));
            p.rect_filled(pill, 7.5, color);
            p.galley(pill.center() - galley.size() * 0.5, galley, egui::Color32::BLACK);
        }
    }
}

/// The Window layout submenu: which panels are open, and back to the default.
pub fn menu(app: &mut App, ui: &mut egui::Ui) {
    ui.menu_button(icons::with(icons::LAYOUT, &t("Window layout")), |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let mut state = app.settings.layout.dock.clone().unwrap_or_else(|| default_dock(&app.settings.layout));
        let mut changed = false;
        for tab in Tab::ALL {
            if tab == Tab::Viewport { continue; }
            let open = state.find_tab(&tab).is_some();
            if ui.checkbox(&mut open.clone(), icons::with(tab.icon(), &t(tab.name()))).clicked() {
                if open {
                    if let Some(at) = state.find_tab(&tab) { state.remove_tab(at); }
                } else {
                    show_tab(&mut state, tab);
                }
                changed = true;
            }
        }
        if changed {
            app.settings.layout.dock = Some(state);
            app.settings.save();
        }
        ui.separator();
        ui.label(egui::RichText::new(t("Drag a tab by its title to move it, to the edge of a panel to split it, or out of the window to float.")).small().color(theme::TEXT_DIM));
        if ui.button(icons::with(icons::ARROW_COUNTER_CLOCKWISE, &t("Default layout"))).on_hover_text(t("Puts every panel back where it was at first, at its first size.")).clicked() {
            reset(app);
            ui.close_menu();
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

/// A panel's contents in full screen.
fn pane_ui(app: &mut App, ui: &mut egui::Ui, p: Pane) {
    match p {
        Pane::Tools => crate::panels::modes_panel(app, ui),
        Pane::Scene => crate::panels::right_panel(app, ui),
        Pane::Content => crate::panels::bottom_panel(app, ui),
    }
}

fn shown(app: &App, p: Pane) -> bool {
    !p.needs_project() || app.doc.is_some()
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
}

/// True when no panel of the full screen view is on screen, for the hint where they are.
pub fn none_open(app: &App) -> bool {
    !app.edge_open.iter().any(|o| *o) && !app.settings.layout.pinned.iter().any(|p| *p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(s: &DockState<Tab>) -> Vec<Tab> {
        let mut v: Vec<Tab> = s.iter_all_tabs().map(|(_, t)| *t).collect();
        v.sort_by_key(|t| Tab::ALL.iter().position(|a| a == t));
        v
    }

    #[test]
    fn the_default_layout_holds_every_tab_once() {
        let s = default_dock(&Layout::default());
        assert_eq!(tabs(&s), Tab::ALL.to_vec());
        assert!(valid(&s));
    }

    #[test]
    fn panels_an_older_version_had_elsewhere_start_there() {
        let old = Layout { tools: Dock::Right, scene: Dock::Window, content: Dock::Hidden, ..Default::default() };
        let s = default_dock(&old);
        assert_eq!(tabs(&s), vec![Tab::Viewport, Tab::Tools, Tab::Outliner, Tab::Details]);
        assert_eq!(s.surfaces_count(), 2, "the outliner and details float in a window");
    }

    #[test]
    fn settings_of_an_older_version_still_load() {
        let l: Layout = serde_json::from_str(r#"{"tools":"Left","scene":"Right","content":"Bottom","pinned":[false,true,false,false],"generation":2}"#).unwrap();
        assert!(l.dock.is_none());
        assert!(l.pinned[1]);
        assert_eq!(l.generation, 2);
    }

    #[test]
    fn a_layout_this_version_cannot_read_falls_back_without_losing_the_rest() {
        let mut saved = serde_json::to_value(Layout { dock: Some(default_dock(&Layout::default())), generation: 5, ..Default::default() }).unwrap();
        let back: Layout = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(back.dock.as_ref().map(tabs), Some(Tab::ALL.to_vec()));
        // A newer version's tab, unknown here.
        let text = saved["dock"].to_string().replacen("\"Build\"", "\"Profiler\"", 1);
        saved["dock"] = serde_json::from_str(&text).unwrap();
        let back: Layout = serde_json::from_value(saved).unwrap();
        assert!(back.dock.is_none());
        assert_eq!(back.generation, 5);
    }

    #[test]
    fn a_closed_tab_comes_back_next_to_its_neighbours() {
        let mut s = default_dock(&Layout::default());
        let at = s.find_tab(&Tab::Output).unwrap();
        s.remove_tab(at);
        assert!(s.find_tab(&Tab::Output).is_none());
        show_tab(&mut s, Tab::Output);
        let (surface, node, _) = s.find_tab(&Tab::Output).unwrap();
        assert_eq!(s.find_tab(&Tab::Content).map(|(a, b, _)| (a, b)), Some((surface, node)));
    }
}
