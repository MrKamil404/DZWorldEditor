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

/// A row of numbers side by side under one label, as Location and Rotation in the Unreal
/// details: each number is marked at its left with the colour of its axis. True once an edit
/// is over (a drag let go or typing left), for the undo step.
pub fn vector_row(ui: &mut egui::Ui, label: &str, fields: &mut [(&mut f32, f64, Color32, String)]) -> bool {
    let mut done = false;
    row(ui, label, |ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let n = fields.len() as f32;
        let width = ((ui.available_width() - 3.0 * (n - 1.0)) / n).max(30.0);
        for (v, speed, color, tip) in fields.iter_mut() {
            // A turned and turned back value may be -0, shown as "-0.0"; it is the same as 0.
            if **v == 0.0 { **v = 0.0; }
            let r = ui.add_sized([width, 22.0], egui::DragValue::new(&mut **v).speed(*speed).max_decimals(3));
            let bar = egui::Rect::from_min_size(r.rect.min, egui::vec2(3.0, r.rect.height()));
            ui.painter().rect_filled(bar, CornerRadius { nw: 3, sw: 3, ne: 0, se: 0 }, *color);
            done |= r.drag_stopped() || r.lost_focus();
            if !tip.is_empty() { r.on_hover_text(tip.as_str()); }
        }
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
