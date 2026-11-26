//! Dark interface modelled on Unreal Engine 5 and Enfusion Workbench: panels with tabs on a
//! darker ground, fields darker than the panel, one blue for what is active and a yellow frame
//! round the view in use.
use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, Stroke, TextStyle};

/// The ground between panels.
pub const GAP: Color32 = Color32::from_rgb(16, 16, 16);
/// The strip that holds the tabs of a panel, the menu and the status bar.
pub const STRIP: Color32 = Color32::from_rgb(26, 26, 26);
/// The toolbar.
pub const PANEL: Color32 = Color32::from_rgb(28, 28, 28);
/// The body of a panel.
pub const BG: Color32 = Color32::from_rgb(36, 36, 36);
/// Every other row of a list, a shade lighter than the panel.
pub const ROW_ALT: Color32 = Color32::from_rgb(39, 39, 39);
/// Section headers and buttons.
pub const SECTION: Color32 = Color32::from_rgb(46, 46, 46);
/// Text fields and numbers.
pub const FIELD: Color32 = Color32::from_rgb(15, 15, 15);
pub const DARK: Color32 = Color32::from_rgb(21, 21, 21);
pub const HOVER: Color32 = Color32::from_rgb(56, 56, 56);
pub const BORDER: Color32 = Color32::from_rgb(56, 56, 56);
pub const ACCENT: Color32 = Color32::from_rgb(10, 116, 224);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(29, 53, 80);
/// An icon toggle that is on: a dark blue with a light blue icon.
pub const TOGGLE_ON: Color32 = Color32::from_rgb(45, 58, 74);
pub const TOGGLE_ON_TEXT: Color32 = Color32::from_rgb(125, 188, 255);
/// The frame round the view in use.
pub const FOCUS: Color32 = Color32::from_rgb(226, 179, 60);
pub const TEXT: Color32 = Color32::from_rgb(200, 200, 200);
pub const TEXT_BRIGHT: Color32 = Color32::from_rgb(236, 236, 236);
pub const TEXT_DIM: Color32 = Color32::from_rgb(150, 150, 150);
pub const OK: Color32 = Color32::from_rgb(90, 180, 90);
pub const WARN: Color32 = Color32::from_rgb(230, 170, 60);
pub const ERROR: Color32 = Color32::from_rgb(230, 80, 70);
pub const PLAY: Color32 = Color32::from_rgb(47, 143, 62);
/// The colours of the kinds of things in lists, so a road reads as a road at a glance.
pub const KIND_TERRAIN: Color32 = Color32::from_rgb(176, 143, 106);
pub const KIND_ROAD: Color32 = Color32::from_rgb(201, 168, 106);
pub const KIND_GENERATOR: Color32 = Color32::from_rgb(143, 191, 127);
pub const KIND_POINT: Color32 = Color32::from_rgb(226, 179, 60);
pub const KIND_SHAPE: Color32 = Color32::from_rgb(176, 127, 217);
pub const KIND_LAYER: Color32 = Color32::from_rgb(127, 156, 191);
pub const KIND_MODEL: Color32 = Color32::from_rgb(109, 158, 232);
pub const KIND_IMAGE: Color32 = Color32::from_rgb(96, 180, 200);
/// The colours of the X, Y and Z axes, as on the gizmo.
pub const AXIS: [Color32; 3] = [Color32::from_rgb(229, 72, 77), Color32::from_rgb(70, 167, 88), Color32::from_rgb(59, 130, 246)];

pub fn apply(ctx: &egui::Context, scale: f32) {
    ctx.set_pixels_per_point(scale);
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = ROW_ALT;
    v.code_bg_color = DARK;
    v.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(10, 10, 10));
    v.window_corner_radius = CornerRadius::same(4);
    v.menu_corner_radius = CornerRadius::same(4);
    v.window_shadow = egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(140) };
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 10, spread: 0, color: Color32::from_black_alpha(120) };
    v.selection.bg_fill = ACCENT_SOFT;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.hyperlink_color = Color32::from_rgb(77, 163, 255);
    v.override_text_color = Some(TEXT);
    v.slider_trailing_fill = true;
    let w = &mut v.widgets;
    let r = CornerRadius::same(3);
    w.noninteractive.bg_fill = BG;
    w.noninteractive.weak_bg_fill = BG;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, STRIP);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = SECTION;
    w.inactive.weak_bg_fill = SECTION;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = HOVER;
    w.hovered.weak_bg_fill = HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(80, 80, 80));
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = ACCENT;
    w.active.weak_bg_fill = ACCENT;
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open.bg_fill = SECTION;
    w.open.weak_bg_fill = SECTION;
    w.open.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.open.corner_radius = r;
    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(5.0, 4.0);
    s.button_padding = egui::vec2(7.0, 3.0);
    s.window_margin = Margin::same(8);
    s.menu_margin = Margin::same(4);
    s.interact_size.y = 22.0;
    s.scroll.bar_width = 8.0;
    style.text_styles.insert(TextStyle::Body, FontId::proportional(13.0));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(13.0));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(11.0));
    style.text_styles.insert(TextStyle::Heading, FontId::proportional(16.0));
    style.text_styles.insert(TextStyle::Monospace, FontId::monospace(12.0));
    ctx.set_style(style);
}

/// The look of the docked panels and their tabs.
pub fn dock_style(style: &egui::Style) -> egui_dock::Style {
    let mut d = egui_dock::Style::from_egui(style);
    d.dock_area_padding = None;
    d.main_surface_border_stroke = Stroke::NONE;
    d.main_surface_border_rounding = CornerRadius::ZERO;
    d.separator.width = 3.0;
    d.separator.extra_interact_width = 3.0;
    d.separator.color_idle = GAP;
    d.separator.color_hovered = Color32::from_rgb(70, 70, 70);
    d.separator.color_dragged = ACCENT;
    d.tab_bar.bg_fill = STRIP;
    d.tab_bar.height = 28.0;
    d.tab_bar.hline_color = STRIP;
    d.tab_bar.corner_radius = CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 };
    d.tab_bar.fill_tab_bar = false;
    let tab = |bg: Color32, text: Color32| egui_dock::TabInteractionStyle { outline_color: Color32::TRANSPARENT, corner_radius: CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, bg_fill: bg, text_color: text };
    d.tab.active = tab(BG, TEXT_BRIGHT);
    d.tab.focused = tab(BG, TEXT_BRIGHT);
    d.tab.active_with_kb_focus = tab(BG, TEXT_BRIGHT);
    d.tab.focused_with_kb_focus = tab(BG, TEXT_BRIGHT);
    d.tab.inactive = tab(STRIP, TEXT_DIM);
    d.tab.inactive_with_kb_focus = tab(STRIP, TEXT_DIM);
    d.tab.hovered = tab(Color32::from_rgb(32, 32, 32), TEXT);
    d.tab.tab_body = egui_dock::TabBodyStyle { inner_margin: Margin::ZERO, stroke: Stroke::NONE, corner_radius: CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 }, bg_fill: BG };
    d.tab.hline_below_active_tab_name = false;
    d.tab.minimum_width = Some(60.0);
    d.buttons.close_tab_color = TEXT_DIM;
    d.buttons.close_tab_active_color = TEXT_BRIGHT;
    d.buttons.close_tab_bg_fill = HOVER;
    d.overlay.selection_color = ACCENT.gamma_multiply(0.45);
    d.overlay.button_color = TEXT;
    d.overlay.button_border_stroke = Stroke::new(1.0_f32, ACCENT);
    d
}

/// The height of the buttons of the toolbar; every plate of it is this high, plus its edge.
pub const TOOL_H: f32 = 32.0;

/// A group of toolbar buttons on a darker plate, so that tools that belong together read as one.
/// Every plate is as high as the others, with its contents in the middle.
pub fn tool_group<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new().fill(Color32::from_rgb(18, 18, 18)).stroke(Stroke::new(1.0_f32, Color32::from_rgb(44, 44, 44))).corner_radius(6).inner_margin(Margin::same(2)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        ui.allocate_ui_with_layout(egui::vec2(0.0, TOOL_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_height(TOOL_H);
            add(ui)
        }).inner
    }).inner
}

/// Section header used in the side panels, like the Details panel of the Unreal editor.
pub fn section(ui: &mut egui::Ui, title: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    let id = ui.make_persistent_id(title);
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
    let is_open = state.is_open();
    let rect = ui.available_rect_before_wrap();
    let height = 26.0;
    let (bar, resp) = ui.allocate_exact_size(egui::vec2(rect.width(), height), egui::Sense::click());
    // The bar runs from edge to edge of the panel, as in the Unreal details, even where the
    // panel keeps its text a little in from the edge.
    let clip = ui.clip_rect();
    let wide = if bar.left() - clip.left() <= 10.0 && clip.right() - bar.right() <= 10.0 { egui::Rect::from_x_y_ranges(clip.x_range(), bar.y_range()) } else { bar };
    let p = ui.painter();
    p.rect_filled(wide, 0.0, if resp.hovered() { HOVER } else { SECTION });
    p.hline(wide.x_range(), bar.top(), Stroke::new(1.0_f32, STRIP));
    p.hline(wide.x_range(), bar.bottom() - 0.5, Stroke::new(1.0_f32, STRIP));
    let c = egui::pos2(bar.left() + 11.0, bar.center().y);
    let arrow = if is_open { vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)] } else { vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.0, 4.0)] };
    p.add(egui::Shape::convex_polygon(arrow, TEXT_DIM, Stroke::NONE));
    p.text(egui::pos2(bar.left() + 21.0, bar.center().y), egui::Align2::LEFT_CENTER, title, crate::icons::strong(13.0), TEXT_BRIGHT);
    if resp.clicked() {
        state.toggle(ui);
    }
    state.show_body_unindented(ui, |ui| {
        egui::Frame::new().inner_margin(Margin { left: 8, right: 8, top: 6, bottom: 8 }).show(ui, add);
    });
}

/// A row of a list as wide as the list, as in the Unreal outliner: an icon in the colour of
/// its kind, the name cut to fit, and a figure in a column at the right. Every other row is a
/// shade lighter; the selected one is blue.
pub fn list_row(ui: &mut egui::Ui, selected: bool, icon: &str, tint: Color32, name: impl Into<egui::WidgetText>, right: &str) -> egui::Response {
    const H: f32 = 22.0;
    let top = ui.min_rect().top();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), H), egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let odd = (((rect.top() - top) / H).round() as i64).rem_euclid(2) == 1;
    let p = ui.painter();
    let bg = if selected { ACCENT_SOFT } else if resp.hovered() { Color32::from_rgb(50, 50, 50) } else if odd { ROW_ALT } else { Color32::TRANSPARENT };
    p.rect_filled(rect, 2.0, bg);
    let enabled = ui.is_enabled();
    let mut x = rect.left() + 5.0;
    if !icon.is_empty() {
        p.text(egui::pos2(x + 7.0, rect.center().y), egui::Align2::CENTER_CENTER, icon, FontId::proportional(14.0), if enabled { tint } else { tint.gamma_multiply(0.5) });
        x += 20.0;
    }
    let mut right_w = 0.0;
    if !right.is_empty() {
        let g = p.layout_no_wrap(right.to_string(), FontId::monospace(11.0), if selected { Color32::from_rgb(190, 215, 245) } else { TEXT_DIM });
        right_w = g.size().x + 10.0;
        p.galley(egui::pos2(rect.right() - g.size().x - 6.0, rect.center().y - g.size().y * 0.5), g, TEXT_DIM);
    }
    let width = (rect.right() - right_w - x - 4.0).max(10.0);
    let galley = name.into().into_galley(ui, Some(egui::TextWrapMode::Truncate), width, TextStyle::Body);
    let color = if selected { Color32::WHITE } else if resp.hovered() { TEXT_BRIGHT } else { TEXT };
    ui.painter().galley(egui::pos2(x, rect.center().y - galley.size().y * 0.5), galley, color);
    resp
}

/// A small icon switch of a list row (seen, editable, locked): bright when on, faint when off.
pub fn row_switch(ui: &mut egui::Ui, on: bool, icon_on: &str, icon_off: &str, tip: String) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(20.0, 22.0), egui::Sense::click());
    let color = if resp.hovered() { TEXT_BRIGHT } else if on { Color32::from_gray(175) } else { Color32::from_gray(80) };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, if on { icon_on } else { icon_off }, FontId::proportional(14.0), color);
    resp.on_hover_text(tip)
}

/// Drag-and-drop payload for reordering layers and modifiers inside the Outliner.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutlinerDrag {
    TerrainMod(u64),
    MaskMod(u32),
    SatMod(u32),
    ShapeLayer(usize),
    Generator(u64),
}

/// Compact 6-dot drag grip handle placed on the left of a reorderable Outliner row.
pub fn drag_grip(ui: &mut egui::Ui, payload: OutlinerDrag, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(14.0, 22.0), egui::Sense::drag());
    let color = if resp.dragged() {
        ACCENT
    } else if resp.hovered() {
        TEXT_BRIGHT
    } else {
        Color32::from_gray(95)
    };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, crate::icons::DOTS_SIX_VERTICAL, FontId::proportional(13.0), color);
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    resp.dnd_set_drag_payload(payload);
    resp.on_hover_text(tip)
}

/// Draws a crisp drop insertion line over `row_rect` while a matching [`OutlinerDrag`] is dragged,
/// and returns `Some((payload, after))` when dropped (`after == true` means bottom half of the row).
pub fn dnd_reorder_slot(ui: &egui::Ui, row_rect: egui::Rect, accepts: impl Fn(OutlinerDrag) -> bool) -> Option<(OutlinerDrag, bool)> {
    let payload = egui::DragAndDrop::payload::<OutlinerDrag>(ui.ctx())?;
    if !accepts(*payload) {
        return None;
    }
    let ptr = ui.ctx().pointer_latest_pos()?;
    if !row_rect.contains(ptr) {
        return None;
    }
    let after = ptr.y >= row_rect.center().y;
    let y = if after { row_rect.bottom() } else { row_rect.top() };
    let p = ui.painter();
    p.line_segment([egui::pos2(row_rect.left() + 2.0, y), egui::pos2(row_rect.right() - 2.0, y)], Stroke::new(2.0_f32, ACCENT));
    p.circle_filled(egui::pos2(row_rect.left() + 4.0, y), 3.0, ACCENT);
    p.circle_filled(egui::pos2(row_rect.right() - 4.0, y), 3.0, ACCENT);
    if ui.input(|i| i.pointer.any_released()) {
        if let Some(taken) = egui::DragAndDrop::take_payload::<OutlinerDrag>(ui.ctx()) {
            return Some((*taken, after));
        }
    }
    None
}

/// Computes the target `Vec` index when moving display row `from_disp` to before/after `target_disp`.
/// When `reversed` is true (top-of-list = top-of-stack = `len - 1`), converts back to `Vec` index.
pub fn reorder_target(from_disp: usize, target_disp: usize, after: bool, len: usize, reversed: bool) -> usize {
    if len < 2 {
        return 0;
    }
    let mut ins = target_disp + usize::from(after);
    if from_disp < ins {
        ins = ins.saturating_sub(1);
    }
    let ins = ins.min(len - 1);
    if reversed {
        (len - 1) - ins
    } else {
        ins
    }
}

/// A row of numbers side by side under one label, as Location and Rotation in the Unreal
/// details: each number is marked at its left with the colour of its axis. True once an edit
/// is over (a drag let go or typing left), for the undo step.
pub fn vector_row(ui: &mut egui::Ui, label: &str, fields: &mut [(&mut f32, f64, Color32, String)]) -> bool {
    let mut done = false;
    row(ui, label, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let n = fields.len() as f32;
            let width = ((ui.available_width() - 3.0 * (n - 1.0)) / n).max(30.0);
            for (v, speed, color, tip) in fields.iter_mut() {
                // A turned and turned back value may be -0, shown as "-0.0"; it is the same as 0.
                if **v == 0.0 { **v = 0.0; }
                // Shown with up to three decimals, and without a minus for what rounds to zero.
                let r = ui.add_sized([width, 22.0], egui::DragValue::new(&mut **v).speed(*speed).custom_formatter(|n, _| {
                    let s = format!("{n:.3}");
                    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
                    if s == "-0" { "0".to_string() } else { s }
                }));
                let bar = egui::Rect::from_min_size(r.rect.min, egui::vec2(3.0, r.rect.height()));
                ui.painter().rect_filled(bar, CornerRadius { nw: 3, sw: 3, ne: 0, se: 0 }, *color);
                done |= r.drag_stopped() || r.lost_focus();
                if !tip.is_empty() { r.on_hover_text(tip.as_str()); }
            }
        });
    });
    done
}

/// The head of the details: an icon in the colour of the kind, the name, and a line under it.
pub fn details_head(ui: &mut egui::Ui, icon: &str, tint: Color32, name: &str, under: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 4.0, SECTION);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, FontId::proportional(19.0), tint);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add(egui::Label::new(egui::RichText::new(name).font(crate::icons::strong(14.0)).color(TEXT_BRIGHT)).truncate());
            if !under.is_empty() { ui.add(egui::Label::new(egui::RichText::new(under).small().color(TEXT_DIM)).truncate()); }
        });
    });
    ui.add_space(4.0);
}


/// A number in a dark field that fills with blue up to where the value is, as the sliders of
/// the Unreal details: drag across it to set the value, click it to type one. It takes the place
/// of a slider with its number beside it, and is built the same way as `egui::Slider`.
pub struct ValueField<'a> {
    get_set: Box<dyn 'a + FnMut(Option<f64>) -> f64>,
    range: std::ops::RangeInclusive<f64>,
    log: bool,
    suffix: String,
    text: Option<egui::WidgetText>,
    show_value: bool,
    step: Option<f64>,
    integer: bool,
    formatter: Option<Box<dyn 'a + Fn(f64, std::ops::RangeInclusive<usize>) -> String>>,
    parser: Option<Box<dyn 'a + Fn(&str) -> Option<f64>>>,
}

impl<'a> ValueField<'a> {
    pub fn new<N: egui::emath::Numeric>(value: &'a mut N, range: std::ops::RangeInclusive<N>) -> Self {
        let range = range.start().to_f64()..=range.end().to_f64();
        Self {
            get_set: Box::new(move |v: Option<f64>| {
                if let Some(v) = v { *value = N::from_f64(v); }
                value.to_f64()
            }),
            range,
            log: false,
            suffix: String::new(),
            text: None,
            show_value: true,
            step: None,
            integer: N::INTEGRAL,
            formatter: None,
            parser: None,
        }
    }
    pub fn logarithmic(mut self, log: bool) -> Self { self.log = log; self }
    pub fn suffix(mut self, s: impl ToString) -> Self { self.suffix = s.to_string(); self }
    /// A label after the field, as `egui::Slider::text`.
    pub fn text(mut self, text: impl Into<egui::WidgetText>) -> Self { self.text = Some(text.into()); self }
    pub fn show_value(mut self, show: bool) -> Self { self.show_value = show; self }
    pub fn step_by(mut self, step: f64) -> Self { self.step = (step > 0.0).then_some(step); self }
    pub fn custom_formatter(mut self, f: impl 'a + Fn(f64, std::ops::RangeInclusive<usize>) -> String) -> Self { self.formatter = Some(Box::new(f)); self }
    pub fn custom_parser(mut self, f: impl 'a + Fn(&str) -> Option<f64>) -> Self { self.parser = Some(Box::new(f)); self }

    /// Where `v` lies along the field, 0 to 1.
    fn fraction(&self, v: f64) -> f64 {
        let (a, b) = (*self.range.start(), *self.range.end());
        if b <= a { return 0.0; }
        let t = if self.log && a > 0.0 { (v.max(a) / a).ln() / (b / a).ln() } else if self.log { ((v - a) / (b - a)).max(0.0).sqrt() } else { (v - a) / (b - a) };
        t.clamp(0.0, 1.0)
    }
    fn value_at(&self, t: f64) -> f64 {
        let (a, b) = (*self.range.start(), *self.range.end());
        let v = if self.log && a > 0.0 { a * (b / a).powf(t) } else if self.log { a + (b - a) * t * t } else { a + (b - a) * t };
        self.tidy(v)
    }
    fn tidy(&self, v: f64) -> f64 {
        let (a, b) = (*self.range.start(), *self.range.end());
        let mut v = v.clamp(a.min(b), a.max(b));
        if let Some(s) = self.step { v = (v / s).round() * s; }
        if self.integer { v = v.round(); }
        v
    }
    /// Decimals worth showing: as many as one pixel of the field changes.
    fn decimals(&self, v: f64, width: f32) -> usize {
        if self.integer { return 0; }
        let (a, b) = (*self.range.start(), *self.range.end());
        let per_pixel = if self.log && a > 0.0 { v.abs().max(a) * (b / a).ln() } else { b - a } / width.max(1.0) as f64;
        let d = (-per_pixel.max(1e-9).log10()).ceil();
        if let Some(s) = self.step { let ds = (-s.log10()).ceil().max(0.0); return ds.min(3.0) as usize; }
        d.clamp(0.0, 3.0) as usize
    }
    fn format(&self, v: f64, width: f32) -> String {
        let d = self.decimals(v, width);
        match &self.formatter { Some(f) => f(v, d..=d), None => format!("{v:.d$}") }
    }
}

impl egui::Widget for ValueField<'_> {
    fn ui(mut self, ui: &mut egui::Ui) -> egui::Response {
        let label_w = self.text.as_ref().map(|t| t.clone().into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, TextStyle::Body).size().x + ui.spacing().item_spacing.x).unwrap_or(0.0);
        // In a row of a panel it fills the value column (the row sets the slider width to it);
        // elsewhere it is as wide as a slider with its number.
        let wide = ui.spacing().slider_width + if self.show_value { 40.0 } else { 0.0 };
        let width = (ui.available_width() - label_w).min(wide).clamp(56.0, 260.0);
        // As high as the other fields of its row.
        let height = ui.spacing().interact_size.y.max(22.0);
        let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
        let id = resp.id;
        let edit_id = id.with("typing");
        let mut v = (self.get_set)(None);
        let typing = ui.data(|d| d.get_temp::<String>(edit_id));
        if let Some(mut text) = typing {
            // Typing a value: Enter or a click elsewhere keeps it, Esc leaves it.
            let edit = ui.put(rect, egui::TextEdit::singleline(&mut text).horizontal_align(egui::Align::Max).font(TextStyle::Body).margin(egui::vec2(6.0, 2.0)));
            if !edit.has_focus() && !edit.lost_focus() { edit.request_focus(); }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                ui.data_mut(|d| d.remove::<String>(edit_id));
            } else if edit.lost_focus() {
                let t = text.trim().trim_end_matches(self.suffix.trim()).trim().replace(',', ".");
                let parsed = match &self.parser { Some(p) => p(&t), None => t.parse::<f64>().ok() };
                if let Some(n) = parsed {
                    let n = self.tidy(n);
                    if n != v { (self.get_set)(Some(n)); resp.mark_changed(); }
                }
                ui.data_mut(|d| d.remove::<String>(edit_id));
            } else {
                ui.data_mut(|d| d.insert_temp(edit_id, text));
            }
            return resp;
        }
        if resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                let t = ((p.x - rect.left() - 2.0) / (rect.width() - 4.0)).clamp(0.0, 1.0) as f64;
                let n = self.value_at(t);
                if n != v { (self.get_set)(Some(n)); v = n; resp.mark_changed(); }
            }
        } else if resp.clicked() {
            let shown = self.format(v, width);
            ui.data_mut(|d| d.insert_temp(edit_id, shown));
        }
        if ui.is_rect_visible(rect) {
            let p = ui.painter();
            let active = resp.dragged() || resp.has_focus();
            p.rect_filled(rect, 3.0, FIELD);
            let fill_w = (rect.width() - 2.0) * self.fraction(v) as f32;
            if fill_w > 0.5 {
                let fill = egui::Rect::from_min_size(rect.min + egui::vec2(1.0, 1.0), egui::vec2(fill_w, rect.height() - 2.0));
                p.rect_filled(fill, 2.0, if active { Color32::from_rgb(38, 72, 110) } else if resp.hovered() { Color32::from_rgb(42, 66, 94) } else { Color32::from_rgb(35, 54, 75) });
            }
            let border = if active { ACCENT } else if resp.hovered() { Color32::from_gray(85) } else { Color32::from_gray(51) };
            p.rect_stroke(rect, 3.0, Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
            if self.show_value {
                let text = format!("{}{}", self.format(v, width), self.suffix);
                p.text(egui::pos2(rect.right() - 7.0, rect.center().y), egui::Align2::RIGHT_CENTER, text, FontId::proportional(12.5), if ui.is_enabled() { TEXT_BRIGHT } else { TEXT_DIM });
            }
        }
        if resp.hovered() || resp.dragged() { ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal); }
        if let Some(text) = self.text.take() { ui.label(text); }
        resp
    }
}

/// A switch in the column of values with its name in the column of names, as the Unreal
/// details show them; long names wrap within their column.
pub fn check_row(ui: &mut egui::Ui, value: &mut bool, label: impl Into<egui::WidgetText>) -> egui::Response {
    let label = label.into();
    // The switches of one list line up: their column is as wide as the longest name of the
    // list needed in the last frame, at least the column of the other rows.
    let key = ui.id().with("check_column");
    let pass = ui.ctx().cumulative_pass_nr();
    let (seen, now, last) = ui.data(|d| d.get_temp::<(u64, f32, f32)>(key)).unwrap_or((u64::MAX, 0.0, 0.0));
    let (now, last) = if seen == pass { (now, last) } else { (0.0, if seen == u64::MAX { 0.0 } else { now }) };
    let text_w = label.clone().into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, TextStyle::Body).size().x + 12.0;
    ui.data_mut(|d| d.insert_temp(key, (pass, now.max(text_w), last)));
    let mut out = None;
    row_with(ui, |ui, w| {
        let col = last.max(w).min(ui.available_width() - 28.0).max(w);
        ui.allocate_ui_with_layout(egui::vec2(col, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_width(col);
            ui.add(egui::Label::new(label.color(Color32::from_gray(180))).wrap());
        });
        out = Some(ui.checkbox(value, ""));
    });
    out.unwrap()
}

/// A tool as a tile with its picture over its name, for the tools of a mode.
pub fn tool_tile(ui: &mut egui::Ui, active: bool, name: &str, tip: String, draw: impl FnOnce(&egui::Painter, egui::Rect, bool)) -> egui::Response {
    let per_row = ((ui.available_width() + 3.0) / 63.0).floor().max(1.0);
    let w = ((ui.available_width() + 3.0) / per_row - 3.0).clamp(56.0, 80.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 50.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fill = if active { ACCENT } else if resp.hovered() { HOVER } else { SECTION };
        p.rect_filled(rect, 4.0, fill);
        let art = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 17.0), egui::vec2(30.0, 22.0));
        draw(p, art, active);
        let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), FontId::proportional(11.0), if active { Color32::WHITE } else { TEXT });
        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 6.0);
        let g = p.layout_job(job);
        p.galley(egui::pos2(rect.center().x - g.size().x * 0.5, rect.bottom() - 8.0 - g.size().y * 0.5), g, TEXT);
    }
    resp.on_hover_text(tip)
}

/// Choices side by side on one plate, the chosen one blue, as the mode switcher of the toolbar.
pub fn segmented<T: PartialEq + Clone>(ui: &mut egui::Ui, value: &mut T, choices: &[(T, String, String)]) -> bool {
    let mut changed = false;
    egui::Frame::new().fill(Color32::from_rgb(18, 18, 18)).stroke(Stroke::new(1.0_f32, Color32::from_rgb(44, 44, 44))).corner_radius(4).inner_margin(Margin::same(2)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal(|ui| {
            for (v, name, tip) in choices {
                let on = *value == *v;
                let b = egui::Button::new(egui::RichText::new(name).color(if on { Color32::WHITE } else { TEXT })).fill(if on { ACCENT } else { Color32::TRANSPARENT }).stroke(Stroke::NONE).min_size(egui::vec2(0.0, 22.0));
                let r = ui.add(b);
                let r = if tip.is_empty() { r } else { r.on_hover_text(tip) };
                if r.clicked() && !on { *value = v.clone(); changed = true; }
            }
        });
    });
    changed
}

/// Buttons in equal columns across the panel.
pub fn button_columns(ui: &mut egui::Ui, n: usize, add: impl FnOnce(&mut [egui::Ui])) {
    ui.columns(n, |cols| {
        for c in cols.iter_mut() { c.spacing_mut().button_padding.x = 4.0; c.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate); }
        add(cols)
    });
}


/// A smaller fold inside a section, for settings few need: a line with an arrow and a name.
pub fn subsection(ui: &mut egui::Ui, title: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    let id = ui.make_persistent_id(("sub", title));
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
    let (bar, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() { p.rect_filled(bar, 3.0, Color32::from_rgb(44, 44, 44)); }
    let c = egui::pos2(bar.left() + 7.0, bar.center().y);
    let arrow = if state.is_open() { vec![c + egui::vec2(-3.5, -2.0), c + egui::vec2(3.5, -2.0), c + egui::vec2(0.0, 2.5)] } else { vec![c + egui::vec2(-2.0, -3.5), c + egui::vec2(2.5, 0.0), c + egui::vec2(-2.0, 3.5)] };
    p.add(egui::Shape::convex_polygon(arrow, TEXT_DIM, Stroke::NONE));
    p.text(egui::pos2(bar.left() + 16.0, bar.center().y), egui::Align2::LEFT_CENTER, title, crate::icons::strong(12.5), TEXT);
    p.hline(egui::Rangef::new(bar.left() + 16.0 + ui.fonts(|f| f.layout_no_wrap(title.to_string(), crate::icons::strong(12.5), TEXT).size().x) + 8.0, bar.right()), bar.center().y, Stroke::new(1.0_f32, Color32::from_rgb(52, 52, 52)));
    if resp.clicked() { state.toggle(ui); }
    state.show_body_unindented(ui, |ui| {
        egui::Frame::new().inner_margin(Margin { left: 10, right: 0, top: 2, bottom: 4 }).show(ui, add);
    });
}

/// A square choice of a row of choices (brush tips and the like), all the same size: lit blue
/// while chosen. `draw` paints its picture in the given rectangle.
pub fn choice(ui: &mut egui::Ui, chosen: bool, tip: impl Into<egui::WidgetText>, draw: impl FnOnce(&egui::Painter, egui::Rect, bool)) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(30.0, 26.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, 4.0, if chosen { ACCENT } else if resp.hovered() { HOVER } else { SECTION });
        draw(p, rect.shrink2(egui::vec2(6.0, 4.0)), chosen);
    }
    resp.on_hover_text(tip)
}

/// Where the next strokes of a brush go: the modifier chosen, with a button to start a new
/// one, or a new modifier. True when the button to start a new one was clicked.
pub fn strokes_target_row(ui: &mut egui::Ui, target: Option<&str>) -> bool {
    let mut new = false;
    row(ui, &crate::i18n::t("Strokes go to"), |ui| match target {
        Some(name) => {
            ui.add(egui::Label::new(egui::RichText::new(name).color(TEXT_BRIGHT)).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                new = ui.small_button(crate::i18n::t("New modifier")).on_hover_text(crate::i18n::t("The next stroke starts a new modifier")).clicked();
            });
        }
        None => { ui.label(egui::RichText::new(crate::i18n::t("A new modifier")).color(TEXT_DIM)).on_hover_text(crate::i18n::t("The next stroke starts a new modifier")); }
    });
    new
}

/// `label  [widget]` row with aligned columns.
pub fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    row_with(ui, |ui, w| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 22.0), egui::Sense::hover());
        // The name is placed once the row is laid out, in the middle of its height, so it stays
        // level with buttons and fields taller than a line of text.
        let at = ui.painter().add(egui::Shape::Noop);
        add(ui);
        let y = ui.min_rect().center().y;
        let clip = ui.clip_rect().intersect(egui::Rect::from_x_y_ranges(rect.x_range(), ui.min_rect().y_range()));
        let text = ui.fonts(|f| egui::Shape::text(f, egui::pos2(rect.left(), y), egui::Align2::LEFT_CENTER, label, FontId::proportional(12.5), Color32::from_gray(180)));
        // Cut to its column, so a long name does not run under the value.
        ui.painter().with_clip_rect(clip).set(at, text);
    });
}

/// A row of a panel: the name column is `w` wide, the rest is the value. Every other row of
/// one list is a shade lighter, across the whole panel, as in the Unreal details.
pub fn row_with(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui, f32)) {
    let count_id = ui.id().with("row_parity");
    let pass = ui.ctx().cumulative_pass_nr();
    let (seen, n) = ui.data(|d| d.get_temp::<(u64, u32)>(count_id)).unwrap_or((u64::MAX, 0));
    let n = if seen == pass { n } else { 0 };
    ui.data_mut(|d| d.insert_temp(count_id, (pass, n + 1)));
    let bg = ui.painter().add(egui::Shape::Noop);
    let r = ui.horizontal(|ui| {
        let w = (ui.available_width() * 0.42).clamp(70.0, 150.0);
        // Fields of the value column fill it.
        ui.spacing_mut().slider_width = ui.available_width();
        add(ui, w)
    });
    if n % 2 == 1 {
        let rect = r.response.rect;
        let clip = ui.clip_rect();
        let x = if rect.left() - clip.left() <= 16.0 && clip.right() - rect.right() <= 16.0 { clip.x_range() } else { rect.x_range().expand(4.0) };
        ui.painter().set(bg, egui::Shape::rect_filled(egui::Rect::from_x_y_ranges(x, rect.y_range().expand(2.0)), 0.0, ROW_ALT));
    }
}

/// Tool button that shows whether it is the active one.
pub fn tool_button(ui: &mut egui::Ui, active: bool, text: &str) -> egui::Response {
    let b = egui::Button::new(egui::RichText::new(text).color(if active { Color32::WHITE } else { TEXT })).fill(if active { ACCENT } else { SECTION });
    ui.add(b)
}

/// Marks a place that takes what is being dragged: a light frame while anything it takes is
/// dragged, a strong one with a fill while the pointer is over it.
pub fn drop_zone(resp: &egui::Response, takes: bool) {
    if !takes {
        return;
    }
    // Only the part that can be seen: rows scrolled out of a list are laid out but clipped.
    let visible = resp.interact_rect;
    if !visible.is_positive() {
        return;
    }
    let over = resp.contains_pointer();
    let p = resp.ctx.layer_painter(resp.layer_id).with_clip_rect(visible.expand(1.0));
    let r = resp.rect.shrink(1.0);
    if over {
        p.rect_filled(r, 3.0, Color32::from_rgba_unmultiplied(0, 112, 224, 40));
        p.rect_stroke(r, 3.0, Stroke::new(2.0_f32, ACCENT), egui::StrokeKind::Inside);
    } else {
        p.rect_stroke(r, 3.0, Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0, 112, 224, 150)), egui::StrokeKind::Inside);
    }
}

/// A box that takes models dropped on it, drawn alike everywhere: a frame with a line of text,
/// lit while models are dragged, `active` then said in place of `idle`. Returns the models
/// dropped on it this frame.
pub fn model_drop_box(ui: &mut egui::Ui, size: egui::Vec2, idle: &str, active: &str) -> (egui::Response, Vec<std::sync::Arc<String>>) {
    let dragging = dragging_models(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        p.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, if dragging { ACCENT } else { Color32::from_gray(80) }), egui::StrokeKind::Inside);
        let (text, color) = if dragging { (active, Color32::WHITE) } else { (idle, TEXT_DIM) };
        let galley = p.layout(text.to_string(), FontId::proportional(12.0), color, (rect.width() - 12.0).max(10.0));
        let at = rect.center() - galley.size() * 0.5;
        p.galley(at, galley, color);
    }
    drop_zone(&resp, dragging);
    let dropped = dropped_models(&resp);
    (resp, dropped)
}

/// The box under a list of models that adds the models dropped on it.
pub fn add_models_box(ui: &mut egui::Ui) -> Vec<std::sync::Arc<String>> {
    model_drop_box(ui, egui::vec2(ui.available_width(), 32.0), &crate::i18n::t("Drop models here to add them"), &crate::i18n::t("Drop to add")).1
}

/// True while a model (or several things) is being dragged.
pub fn dragging_models(ctx: &egui::Context) -> bool {
    egui::DragAndDrop::has_payload_of_type::<String>(ctx) || egui::DragAndDrop::has_payload_of_type::<crate::asset_browser::Dragged>(ctx)
}

/// The models dropped on a place this frame: one dragged alone or all of several.
pub fn dropped_models(resp: &egui::Response) -> Vec<std::sync::Arc<String>> {
    let mut out: Vec<std::sync::Arc<String>> = crate::theme::take::<String>(&resp).into_iter().collect();
    if let Some(d) = crate::theme::take::<crate::asset_browser::Dragged>(&resp) {
        out.extend(d.0.iter().filter_map(|p| if let crate::asset_browser::Pick::Model(m) = p { Some(std::sync::Arc::new(m.clone())) } else { None }));
    }
    out
}

/// What was dropped on a place this frame, if it is of this type. egui throws away a payload
/// asked for as another type, so the type is checked before it is taken.
pub fn take<T: std::any::Any + Send + Sync>(resp: &egui::Response) -> Option<std::sync::Arc<T>> {
    if egui::DragAndDrop::has_payload_of_type::<T>(&resp.ctx) {
        resp.dnd_release_payload::<T>()
    } else {
        None
    }
}

/// One choice of [`search_pick`].
pub struct PickItem<K> {
    pub key: K,
    pub name: String,
    /// Shown dim after the name, and searched too: a folder or a path.
    pub note: String,
    /// The model whose picture stands for it.
    pub model: Option<String>,
    /// A picture of its own (the texture of a surface), in place of a model's.
    pub picture: Option<egui::TextureId>,
}

/// The surface layers named `names` as choices of [`search_pick`], each with the picture of
/// its texture (`pictures`, by layer, where it is loaded) and the key `key` gives it.
pub fn surface_items<K>(names: &[String], pictures: &[Option<egui::TextureId>], key: impl Fn(usize, &str) -> K) -> Vec<PickItem<K>> {
    names.iter().enumerate().map(|(i, n)| PickItem { key: key(i, n), name: n.clone(), note: String::new(), model: None, picture: pictures.get(i).copied().flatten() }).collect()
}

/// A list to choose a surface layer from, with the picture of each surface and a search.
#[allow(clippy::too_many_arguments)]
pub fn surface_pick<K: PartialEq + Clone>(ui: &mut egui::Ui, id: impl std::hash::Hash, value: &mut K, none: Option<(K, String)>, names: &[String], pictures: &[Option<egui::TextureId>], key: impl Fn(usize, &str) -> K, width: f32) -> bool {
    let items = surface_items(names, pictures, key);
    search_pick(ui, id, value, none, &items, &|_| None, width)
}

/// A drop-down to choose among many: a search field on top and a list with a picture of each
/// model, a larger one under the pointer, as in the Content Browser. `none` is the choice of
/// nothing, first in the list. Returns true when the choice changed.
pub fn search_pick<K: PartialEq + Clone>(ui: &mut egui::Ui, id: impl std::hash::Hash, value: &mut K, none: Option<(K, String)>, items: &[PickItem<K>], thumb: &dyn Fn(&str) -> Option<egui::TextureId>, width: f32) -> bool {
    let id = ui.make_persistent_id(("search_pick", id));
    let current = items.iter().find(|i| i.key == *value);
    let shown = current.map(|i| i.name.clone()).or_else(|| none.as_ref().filter(|n| n.0 == *value).map(|n| n.1.clone())).unwrap_or_default();
    let button = ui.add_sized(egui::vec2(width, 22.0), egui::Button::new(egui::RichText::new(format!("{shown}  {}", crate::icons::CARET_DOWN))).truncate()).on_hover_text(current.map(|i| i.note.as_str()).unwrap_or(""));
    if button.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
        ui.data_mut(|d| d.insert_temp(id.with("focus"), true));
    }
    let mut changed = false;
    egui::popup_below_widget(ui, id, &button, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(width.max(260.0));
        let mut text: String = ui.data(|d| d.get_temp(id.with("text"))).unwrap_or_default();
        let field = ui.add(egui::TextEdit::singleline(&mut text).hint_text(crate::i18n::t("Search")).desired_width(f32::INFINITY));
        if ui.data_mut(|d| d.remove_temp::<bool>(id.with("focus"))).is_some() {
            field.request_focus();
        }
        ui.data_mut(|d| d.insert_temp(id.with("text"), text.clone()));
        let words: Vec<String> = text.to_lowercase().split_whitespace().map(str::to_string).collect();
        let fits = |i: &PickItem<K>| { let hay = format!("{} {}", i.name, i.note).to_lowercase(); words.iter().all(|w| hay.contains(w)) };
        let mut picked = None;
        // Rows across the whole width of the list, not only as wide as their text.
        egui::ScrollArea::vertical().max_height(340.0).auto_shrink([false, true]).show(ui, |ui| {
            let full = ui.available_width();
            let mark = |ui: &egui::Ui, rect: egui::Rect, on: bool, hovered: bool| (on || hovered).then(|| egui::Shape::rect_filled(rect, 3.0, if on { Color32::from_rgba_unmultiplied(10, 116, 224, 50) } else { Color32::from_white_alpha(10) })).map(|s| ui.painter().add(s));
            if let Some((k, name)) = &none {
                if words.is_empty() {
                    let (rect, r) = ui.allocate_exact_size(egui::vec2(full, 22.0), egui::Sense::click());
                    mark(ui, rect, *value == *k, r.hovered());
                    ui.painter().text(rect.left_center() + egui::vec2(4.0, 0.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(13.0), if *value == *k { Color32::WHITE } else { TEXT });
                    if r.clicked() {
                        picked = Some(k.clone());
                    }
                }
            }
            let mut shown = 0;
            for item in items.iter().filter(|i| fits(i)) {
                shown += 1;
                let on = item.key == *value;
                let bg = ui.painter().add(egui::Shape::Noop);
                let r = ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
                    match item.picture.or_else(|| item.model.as_deref().and_then(thumb)) {
                        Some(tex) => { ui.painter().image(tex, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE); }
                        None => { ui.painter().rect_filled(rect, 3.0, Color32::from_gray(38)); }
                    }
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(&item.name).color(if on { Color32::WHITE } else { TEXT })).truncate());
                        if !item.note.is_empty() { ui.add(egui::Label::new(egui::RichText::new(&item.note).small().color(TEXT_DIM)).truncate()); }
                    });
                }).response;
                let row = egui::Rect::from_min_size(r.rect.min, egui::vec2(full, r.rect.height()));
                let r = ui.interact(row, id.with(("row", shown)), egui::Sense::click());
                if on || r.hovered() {
                    ui.painter().set(bg, egui::Shape::rect_filled(r.rect.expand(1.0), 3.0, if on { Color32::from_rgba_unmultiplied(10, 116, 224, 50) } else { Color32::from_white_alpha(10) }));
                }
                if let Some(tex) = item.model.as_deref().and_then(thumb) {
                    r.clone().on_hover_ui_at_pointer(|ui| { ui.image((tex, egui::vec2(160.0, 160.0))); ui.label(&item.name); });
                }
                if r.clicked() {
                    picked = Some(item.key.clone());
                }
            }
            if shown == 0 {
                ui.weak(crate::i18n::t("Nothing found"));
            }
        });
        if let Some(k) = picked {
            changed = *value != k;
            *value = k;
            ui.memory_mut(|m| m.close_popup());
        }
    });
    changed
}

/// Persistent window chrome state stored in `egui::Context` memory for [`PopupWindow`].
#[derive(Clone, Default)]
struct PopupState {
    minimized: bool,
    maximized: bool,
    own_window: bool,
    /// Saved rect before maximizing, restored when leaving maximized mode.
    restore_rect: Option<egui::Rect>,
    /// Saved size before minimizing, restored when un-minimizing.
    restore_size: Option<egui::Vec2>,
    /// One-shot position/size applied on the next frame (after restore or title-bar drag).
    pending_pos: Option<egui::Pos2>,
    pending_size: Option<egui::Vec2>,
}

/// Strips a leading Phosphor icon glyph (Private Use Area) so OS window titles stay clean.
fn strip_leading_icon(title: &str) -> &str {
    let mut chars = title.chars();
    if let Some(first) = chars.next() {
        if ('\u{E000}'..='\u{F8FF}').contains(&first) {
            return chars.as_str().trim_start();
        }
    }
    title
}

/// Floating editor popup window with custom title-bar controls next to Close (`X`):
/// pop-out to an independent OS window, minimize (collapse to title bar), and maximize/restore.
pub struct PopupWindow<'a> {
    id: egui::Id,
    title: &'a str,
    default_size: [f32; 2],
    min_size: [f32; 2],
    allow_popout: bool,
    allow_maximize: bool,
    vscroll: bool,
}

impl<'a> PopupWindow<'a> {
    pub fn new(id: egui::Id, title: &'a str) -> Self {
        Self {
            id,
            title,
            default_size: [640.0, 480.0],
            min_size: [320.0, 220.0],
            allow_popout: true,
            allow_maximize: true,
            vscroll: false,
        }
    }

    pub fn default_size(mut self, size: [f32; 2]) -> Self {
        self.default_size = size;
        self
    }

    pub fn min_size(mut self, size: [f32; 2]) -> Self {
        self.min_size = size;
        self
    }

    pub fn vscroll(mut self, vscroll: bool) -> Self {
        self.vscroll = vscroll;
        self
    }

    /// Shows the popup either inside the main window (with custom title-bar controls) or in its
    /// own OS window when popped out. Updates `open` when closed.
    pub fn show(self, ctx: &egui::Context, open: &mut bool, body: impl FnOnce(&mut egui::Ui)) {
        self.show_with_own(ctx, open, None, body);
    }

    /// Like [`Self::show`], syncing the detached-window flag with `own_window` (for callers that
    /// also toggle it externally, e.g. via automation commands).
    pub fn show_with_own(
        self,
        ctx: &egui::Context,
        open: &mut bool,
        mut own_window: Option<&mut bool>,
        body: impl FnOnce(&mut egui::Ui),
    ) {
        if !*open {
            return;
        }
        let mut body = Some(body);
        let state_id = self.id.with("dzwe_popup_state");
        let mut state: PopupState = ctx.data_mut(|d| d.get_temp(state_id)).unwrap_or_default();
        if let Some(flag) = own_window.as_deref() {
            state.own_window = *flag;
        }

        if state.own_window && self.allow_popout {
            let vp_id = egui::ViewportId::from_hash_of(("dzwe_popup_vp", self.id));
            let clean_title = strip_leading_icon(self.title);
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("DZ World Editor – {clean_title}"))
                .with_inner_size(self.default_size)
                .with_min_inner_size(self.min_size)
                .with_icon(crate::icons::window_icon());

            let mut dock_back = false;
            let mut closed = false;
            ctx.show_viewport_immediate(vp_id, builder, |vctx, _class| {
                if vctx.input(|i| i.viewport().close_requested()) {
                    closed = true;
                    return;
                }
                egui::TopBottomPanel::top(self.id.with("popup_vp_bar"))
                    .exact_height(26.0)
                    .frame(egui::Frame::new().fill(STRIP).inner_margin(Margin::symmetric(8, 2)).stroke(Stroke::new(1.0_f32, BORDER)))
                    .show(vctx, |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.add(egui::Label::new(egui::RichText::new(self.title).font(crate::icons::strong(12.5)).color(TEXT_BRIGHT)).truncate());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(egui::Button::new(crate::icons::ARROW_SQUARE_IN).frame(false))
                                    .on_hover_text(crate::i18n::t("Back into the main window"))
                                    .clicked()
                                {
                                    dock_back = true;
                                }
                            });
                        });
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(BG).inner_margin(Margin::same(8)))
                    .show(vctx, |ui| {
                        if let Some(f) = body.take() {
                            if self.vscroll {
                                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, f);
                            } else {
                                f(ui);
                            }
                        }
                    });
            });

            if closed {
                *open = false;
                state.own_window = false;
            } else if dock_back {
                state.own_window = false;
            }
            if let Some(flag) = own_window.as_deref_mut() {
                *flag = state.own_window;
            }
            ctx.data_mut(|d| d.insert_temp(state_id, state));
            return;
        }

        const TITLE_H: f32 = 28.0;
        let screen = ctx.screen_rect();
        let max_rect = egui::Rect::from_min_max(
            egui::pos2(screen.left() + 4.0, screen.top() + 26.0),
            egui::pos2((screen.right() - 4.0).max(screen.left() + 200.0), (screen.bottom() - 24.0).max(screen.top() + 120.0)),
        );

        let cur_rect = ctx.memory(|m| m.area_rect(self.id));
        let frame = egui::Frame::window(&ctx.style()).inner_margin(Margin::ZERO);

        let mut win = egui::Window::new("")
            .id(self.id)
            .title_bar(false)
            .frame(frame)
            .vscroll(false)
            .movable(!(state.maximized && self.allow_maximize));

        if state.minimized {
            let w = state.restore_size.map(|s| s.x).unwrap_or(self.default_size[0]).clamp(280.0, 460.0);
            win = win.resizable(false).fixed_size([w, TITLE_H]);
            if let Some(pos) = state.pending_pos.take() {
                win = win.current_pos(pos);
            }
        } else if state.maximized && self.allow_maximize {
            win = win
                .resizable(false)
                .fixed_pos(max_rect.min)
                .fixed_size(max_rect.size());
        } else {
            win = win
                .resizable(true)
                .default_size(self.default_size)
                .min_size(self.min_size);
            if let Some(pos) = state.pending_pos.take() {
                win = win.current_pos(pos);
            }
            if let Some(size) = state.pending_size.take() {
                win = win.fixed_size(size);
            }
        }

        let mut toggle_min = false;
        let mut toggle_max = false;
        let mut pop_out = false;
        let mut close_clicked = false;

        win.show(ctx, |ui| {
            // Custom title bar spanning the full width of the window.
            // Allocated without a drag widget so egui::Window's native zero-lag drag moves it.
            let bar_w = ui.available_width().max(220.0);
            let (bar_rect, _) = ui.allocate_exact_size(egui::vec2(bar_w, TITLE_H), egui::Sense::hover());
            let rounding = if state.minimized {
                CornerRadius::same(4)
            } else {
                CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }
            };
            ui.painter().rect_filled(bar_rect, rounding, STRIP);
            if !state.minimized {
                ui.painter().line_segment(
                    [egui::pos2(bar_rect.left(), bar_rect.bottom()), egui::pos2(bar_rect.right(), bar_rect.bottom())],
                    Stroke::new(1.0_f32, BORDER),
                );
            }

            let btn_w = 28.0;
            let btn_h = 22.0;
            let btn_y = bar_rect.center().y - btn_h * 0.5;
            let mut right_x = bar_rect.right() - 4.0;

            let mut title_btn = |ui: &mut egui::Ui, id_salt: &str, glyph: &str, tip: String, danger: bool| -> bool {
                right_x -= btn_w;
                let r = egui::Rect::from_min_size(egui::pos2(right_x, btn_y), egui::vec2(btn_w, btn_h));
                let resp = ui.interact(r, self.id.with(id_salt), egui::Sense::click());
                if resp.hovered() {
                    let fill = if danger { Color32::from_rgb(196, 43, 28) } else { HOVER };
                    ui.painter().rect_filled(r, CornerRadius::same(3), fill);
                }
                let col = if resp.hovered() && danger { Color32::WHITE } else if resp.hovered() { TEXT_BRIGHT } else { TEXT };
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, glyph, FontId::proportional(13.0), col);
                resp.on_hover_text(tip).clicked()
            };

            // Buttons from right to left: Close, Maximize/Restore, Minimize, Pop-out.
            if title_btn(ui, "btn_close", crate::icons::X, crate::i18n::t("Close"), true) {
                close_clicked = true;
            }
            if self.allow_maximize {
                let (glyph, tip) = if state.maximized {
                    (crate::icons::CORNERS_IN, crate::i18n::t("Restore"))
                } else {
                    (crate::icons::CORNERS_OUT, crate::i18n::t("Maximize"))
                };
                if title_btn(ui, "btn_max", glyph, tip, false) {
                    toggle_max = true;
                }
            }
            let min_tip = if state.minimized { crate::i18n::t("Restore") } else { crate::i18n::t("Minimize") };
            if title_btn(ui, "btn_min", crate::icons::MINUS, min_tip, false) {
                toggle_min = true;
            }
            if self.allow_popout {
                if title_btn(
                    ui,
                    "btn_popout",
                    crate::icons::ARROW_SQUARE_OUT,
                    crate::i18n::t("A window of its own, which can go outside the program and onto another screen"),
                    false,
                ) {
                    pop_out = true;
                }
            }

            // Title area on the left of the buttons: leaves pointer drags to egui::Window's native
            // movable handler (zero lag), while still handling double-click to maximize/restore.
            let drag_rect = egui::Rect::from_min_max(bar_rect.min, egui::pos2((right_x - 4.0).max(bar_rect.left() + 40.0), bar_rect.bottom()));
            if ui.rect_contains_pointer(drag_rect)
                && ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary))
            {
                if state.minimized {
                    toggle_min = true;
                } else if self.allow_maximize {
                    toggle_max = true;
                }
            }
            let text_rect = egui::Rect::from_min_max(
                egui::pos2(drag_rect.left() + 10.0, drag_rect.top()),
                egui::pos2(drag_rect.right() - 4.0, drag_rect.bottom()),
            );
            ui.painter().with_clip_rect(text_rect).text(
                egui::pos2(text_rect.left(), text_rect.center().y),
                egui::Align2::LEFT_CENTER,
                self.title,
                crate::icons::strong(13.0),
                TEXT_BRIGHT,
            );

            if !state.minimized {
                egui::Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
                    if let Some(f) = body.take() {
                        if self.vscroll {
                            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, f);
                        } else {
                            f(ui);
                        }
                    }
                });
            }
        });

        if close_clicked {
            *open = false;
        } else if pop_out {
            state.own_window = true;
            state.minimized = false;
        } else if toggle_max {
            if state.maximized {
                state.maximized = false;
                state.minimized = false;
                if let Some(r) = state.restore_rect.take() {
                    state.pending_pos = Some(r.min);
                    state.pending_size = Some(r.size());
                }
            } else {
                if !state.minimized {
                    state.restore_rect = cur_rect;
                } else if let (Some(r), Some(sz)) = (cur_rect, state.restore_size) {
                    state.restore_rect = Some(egui::Rect::from_min_size(r.min, sz));
                }
                state.maximized = true;
                state.minimized = false;
            }
        } else if toggle_min {
            if state.minimized {
                state.minimized = false;
                if !state.maximized {
                    if let Some(sz) = state.restore_size.take() {
                        state.pending_size = Some(sz);
                    }
                }
            } else {
                if !state.maximized {
                    if let Some(r) = cur_rect {
                        state.restore_size = Some(r.size());
                    }
                }
                state.minimized = true;
            }
        }

        if let Some(flag) = own_window.as_deref_mut() {
            *flag = state.own_window;
        }
        ctx.data_mut(|d| d.insert_temp(state_id, state));
    }
}
