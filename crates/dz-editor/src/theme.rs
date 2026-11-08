//! Dark interface modelled on Unreal Engine's editor.
use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(36, 36, 36);
pub const PANEL: Color32 = Color32::from_rgb(47, 47, 47);
pub const DARK: Color32 = Color32::from_rgb(21, 21, 21);
pub const HOVER: Color32 = Color32::from_rgb(64, 64, 64);
pub const ACCENT: Color32 = Color32::from_rgb(0, 112, 224);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(38, 79, 120);
pub const TEXT: Color32 = Color32::from_rgb(192, 192, 192);
pub const TEXT_DIM: Color32 = Color32::from_rgb(128, 128, 128);
pub const OK: Color32 = Color32::from_rgb(90, 180, 90);
pub const WARN: Color32 = Color32::from_rgb(230, 170, 60);
pub const ERROR: Color32 = Color32::from_rgb(230, 80, 70);
pub const PLAY: Color32 = Color32::from_rgb(68, 170, 68);

pub fn apply(ctx: &egui::Context, scale: f32) {
    ctx.set_pixels_per_point(scale);
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = DARK;
    v.faint_bg_color = Color32::from_rgb(42, 42, 42);
    v.window_stroke = Stroke::new(1.0_f32, DARK);
    v.window_corner_radius = CornerRadius::same(3);
    v.menu_corner_radius = CornerRadius::same(3);
    v.selection.bg_fill = ACCENT_SOFT;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.hyperlink_color = Color32::from_rgb(90, 160, 240);
    v.override_text_color = Some(TEXT);
    let w = &mut v.widgets;
    let r = CornerRadius::same(2);
    w.noninteractive.bg_fill = BG;
    w.noninteractive.weak_bg_fill = BG;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, DARK);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = Color32::from_rgb(56, 56, 56);
    w.inactive.weak_bg_fill = Color32::from_rgb(56, 56, 56);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = HOVER;
    w.hovered.weak_bg_fill = HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(90, 90, 90));
    w.hovered.corner_radius = r;
    w.active.bg_fill = ACCENT;
    w.active.weak_bg_fill = ACCENT;
    w.active.corner_radius = r;
    w.open.bg_fill = Color32::from_rgb(56, 56, 56);
    w.open.weak_bg_fill = Color32::from_rgb(56, 56, 56);
    w.open.corner_radius = r;
    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(5.0, 4.0);
    s.button_padding = egui::vec2(6.0, 3.0);
    s.window_margin = Margin::same(8);
    s.interact_size.y = 20.0;
    s.scroll.bar_width = 8.0;
    style.text_styles.insert(TextStyle::Body, FontId::proportional(13.0));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(13.0));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(11.0));
    style.text_styles.insert(TextStyle::Heading, FontId::proportional(16.0));
    style.text_styles.insert(TextStyle::Monospace, FontId::monospace(12.0));
    ctx.set_style(style);
}

/// Section header used in the side panels, like the Details panel of the Unreal editor.
pub fn section(ui: &mut egui::Ui, title: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    let id = ui.make_persistent_id(title);
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
    let is_open = state.is_open();
    let rect = ui.available_rect_before_wrap();
    let height = 22.0;
    let (bar, resp) = ui.allocate_exact_size(egui::vec2(rect.width(), height), egui::Sense::click());
    ui.painter().rect_filled(bar, 0.0, if resp.hovered() { HOVER } else { Color32::from_rgb(58, 58, 58) });
    let c = egui::pos2(bar.left() + 11.0, bar.center().y);
    let arrow = if is_open { vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)] } else { vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.0, 4.0)] };
    ui.painter().add(egui::Shape::convex_polygon(arrow, TEXT, Stroke::NONE));
    ui.painter().text(egui::pos2(bar.left() + 20.0, bar.center().y), egui::Align2::LEFT_CENTER, title, FontId::proportional(13.0), Color32::from_rgb(220, 220, 220));
    if resp.clicked() {
        state.toggle(ui);
    }
    state.show_body_unindented(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::symmetric(6, 5)).show(ui, add);
    });
}

/// `label  [widget]` row with aligned columns.
pub fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2((ui.available_width() * 0.42).clamp(70.0, 150.0), 20.0), egui::Sense::hover());
        ui.painter_at(rect).text(egui::pos2(rect.left(), rect.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(12.5), TEXT);
        add(ui);
    });
}

/// Tool button that shows whether it is the active one.
pub fn tool_button(ui: &mut egui::Ui, active: bool, text: &str) -> egui::Response {
    let b = egui::Button::new(egui::RichText::new(text).color(if active { Color32::WHITE } else { TEXT })).fill(if active { ACCENT } else { Color32::from_rgb(56, 56, 56) });
    ui.add(b)
}
