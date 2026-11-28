//! Fields that hold a model or a preset, laid out alike everywhere, as the asset fields of
//! Unreal: in the row of their name, a picture on the left (larger under the pointer) that
//! takes what is dragged from the Content Browser, and on the right the name or a list to
//! choose from with a search field, over small buttons to take what is selected in the
//! Content Browser, show it there, or clear the field.
use crate::asset_browser::{Dragged, Pick};
use crate::i18n::t;
use crate::library_ui::LibDrag;
use crate::models::ThumbPeek;
use crate::theme::{self, PickItem};
use eframe::egui::{self, Color32, Stroke};

/// Side of the picture of a field.
const PICTURE: f32 = 44.0;

/// Where the Content Browser is asked to show something; read by it each frame.
fn reveal_id() -> egui::Id {
    egui::Id::new("slots_reveal")
}

/// Asks the Content Browser to show `pick` and select it.
pub fn request_reveal(ctx: &egui::Context, pick: Pick) {
    ctx.data_mut(|d| d.insert_temp(reveal_id(), Some(pick)));
}

/// What a field asked the Content Browser to show, once.
pub fn take_reveal(ctx: &egui::Context) -> Option<Pick> {
    ctx.data_mut(|d| d.remove_temp::<Option<Pick>>(reveal_id())).flatten()
}

/// The things dragged now, when they come from the Content Browser or a library.
fn dragged(ctx: &egui::Context) -> Vec<Pick> {
    if let Some(m) = egui::DragAndDrop::payload::<String>(ctx) {
        return vec![Pick::Model((*m).clone())];
    }
    if let Some(d) = egui::DragAndDrop::payload::<Dragged>(ctx) {
        return d.0.clone();
    }
    match egui::DragAndDrop::payload::<LibDrag>(ctx).as_deref() {
        Some(LibDrag::ObjectPreset(id)) => vec![Pick::Preset(*id)],
        Some(LibDrag::ForestPreset(n)) => vec![Pick::Forest(n.clone())],
        Some(LibDrag::SurfaceBiome(i)) => vec![Pick::Biome(*i)],
        _ => Vec::new(),
    }
}

/// What was dropped on `resp` this frame.
fn dropped(resp: &egui::Response) -> Vec<Pick> {
    if let Some(m) = theme::take::<String>(resp) {
        return vec![Pick::Model((*m).clone())];
    }
    if let Some(d) = theme::take::<Dragged>(resp) {
        return d.0.clone();
    }
    match theme::take::<LibDrag>(resp).as_deref() {
        Some(LibDrag::ObjectPreset(id)) => vec![Pick::Preset(*id)],
        Some(LibDrag::ForestPreset(n)) => vec![Pick::Forest(n.clone())],
        Some(LibDrag::SurfaceBiome(i)) => vec![Pick::Biome(*i)],
        _ => Vec::new(),
    }
}

/// The picture of a field: the model's, or an empty square; larger with its name under the
/// pointer. It takes what is dragged when `takes` says it would.
fn picture(ui: &mut egui::Ui, model: Option<&str>, thumbs: &ThumbPeek, takes: bool, hover: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(PICTURE, PICTURE), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, Color32::from_gray(24));
    let tex = model.and_then(|m| thumbs.get(m));
    match tex {
        Some(tex) => {
            p.image(tex, rect.shrink(1.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            p.text(rect.center(), egui::Align2::CENTER_CENTER, crate::icons::CUBE, egui::FontId::proportional(18.0), Color32::from_gray(70));
        }
    }
    p.rect_stroke(rect, 3.0, Stroke::new(1.0_f32, Color32::from_gray(60)), egui::StrokeKind::Inside);
    theme::drop_zone(&resp, takes);
    match tex {
        Some(tex) => resp.on_hover_ui(|ui| {
            ui.image((tex, egui::vec2(192.0, 192.0)));
            if !hover.is_empty() { ui.label(hover); }
        }),
        None if !hover.is_empty() => resp.on_hover_text(hover),
        None => resp,
    }
}

/// A small picture of a model in a list, larger under the pointer.
pub fn small_picture(ui: &mut egui::Ui, model: &str, thumbs: &ThumbPeek) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, Color32::from_gray(24));
    if let Some(tex) = thumbs.get(model) {
        ui.painter().image(tex, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        resp.on_hover_ui(|ui| { ui.image((tex, egui::vec2(192.0, 192.0))); ui.label(model); });
    }
}

/// A small button of a field.
fn tool(ui: &mut egui::Ui, icon: &str, tip: String, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(icon).size(12.0)).min_size(egui::vec2(22.0, 18.0))).on_hover_text(tip).clicked()
}

/// The buttons under a field, and a delete button when `remove` is given.
struct Tools {
    /// Something is selected in the Content Browser that the field can take.
    selected: bool,
    /// What "show in the Content Browser" shows.
    reveal: Option<Pick>,
    /// The field holds something to clear.
    clear: bool,
}

/// Which of the buttons was clicked: use the selected one, clear.
fn tools(ui: &mut egui::Ui, t_: Tools, remove: Option<&mut bool>) -> (bool, bool) {
    let mut out = (false, false);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        out.0 = tool(ui, crate::icons::ARROW_LEFT, t("Use the one selected in the Content Browser"), t_.selected);
        if tool(ui, crate::icons::MAGNIFYING_GLASS, t("Show in the Content Browser"), t_.reveal.is_some()) {
            if let Some(p) = t_.reveal {
                request_reveal(ui.ctx(), p);
            }
        }
        if t_.clear {
            out.1 = tool(ui, crate::icons::X, t("Clear"), true);
        }
        if let Some(r) = remove {
            if tool(ui, crate::icons::TRASH, t("Delete"), true) {
                *r = true;
            }
        }
    });
    out
}

/// A field holding the path of a model. `selected` is the model selected in the Content
/// Browser. Returns true when it changed.
pub fn model_slot(ui: &mut egui::Ui, label: &str, value: &mut String, selected: Option<&str>, thumbs: &ThumbPeek, remove: Option<&mut bool>) -> bool {
    let before = value.clone();
    let takes = dragged(ui.ctx()).iter().any(|p| matches!(p, Pick::Model(_)));
    theme::row(ui, label, |ui| {
        let model = (!value.is_empty()).then(|| value.clone());
        let pic = picture(ui, model.as_deref(), thumbs, takes, value);
        ui.vertical(|ui| {
            let name = value.rsplit('\\').next().unwrap_or("").trim_end_matches(".p3d").to_string();
            let shown = if name.is_empty() { t("Drop a model here") } else { name };
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 3.0, Color32::from_gray(30));
            let colour = if value.is_empty() { theme::TEXT_DIM } else { theme::TEXT };
            let galley = ui.painter().layout(shown, egui::FontId::proportional(13.0), colour, rect.width() - 8.0);
            ui.painter().with_clip_rect(rect.shrink(2.0)).galley(rect.left_center() + egui::vec2(4.0, -galley.size().y * 0.5), galley, colour);
            theme::drop_zone(&resp, takes);
            if !value.is_empty() { resp.clone().on_hover_text(value.as_str()); }
            for r in [&pic, &resp] {
                if let Some(Pick::Model(m)) = dropped(r).into_iter().find(|p| matches!(p, Pick::Model(_))) {
                    *value = m;
                }
            }
            let (use_it, clear) = tools(ui, Tools { selected: selected.is_some(), reveal: (!value.is_empty()).then(|| Pick::Model(value.clone())), clear: !value.is_empty() }, remove);
            if use_it {
                if let Some(m) = selected { *value = m.to_string(); }
            }
            if clear { value.clear(); }
        });
    });
    *value != before
}

/// A field holding a preset, chosen from `items` (each with the model its picture is of) in a
/// list with a search field, or dropped from the Content Browser. `none` is the choice of
/// nothing; `from_pick` turns what is dragged or selected in the Content Browser into a
/// choice, and `to_pick` a choice into what the Content Browser shows. Returns true when it
/// changed.
#[allow(clippy::too_many_arguments)]
pub fn preset_slot<K: PartialEq + Clone>(ui: &mut egui::Ui, label: &str, id: impl std::hash::Hash, value: &mut K, none: Option<(K, String)>, items: &[PickItem<K>], thumbs: &ThumbPeek, from_pick: &dyn Fn(&Pick) -> Option<K>, to_pick: &dyn Fn(&K) -> Option<Pick>, selected: Option<K>, remove: Option<&mut bool>) -> bool {
    let before = value.clone();
    let takes = dragged(ui.ctx()).iter().any(|p| from_pick(p).is_some());
    theme::row(ui, label, |ui| {
        let current = items.iter().find(|i| i.key == *value);
        let model = current.and_then(|i| i.model.clone());
        let hover = current.map(|i| i.name.clone()).unwrap_or_default();
        let pic = picture(ui, model.as_deref(), thumbs, takes, &hover);
        ui.vertical(|ui| {
            let w = ui.available_width();
            theme::search_pick(ui, &id, value, none.clone(), items, &|m| thumbs.get(m), w);
            if let Some(k) = dropped(&pic).iter().find_map(|p| from_pick(p)) {
                *value = k;
            }
            let is_none = none.as_ref().is_some_and(|n| n.0 == *value);
            let (use_it, clear) = tools(ui, Tools { selected: selected.is_some(), reveal: to_pick(value), clear: none.is_some() && !is_none }, remove);
            if use_it {
                if let Some(k) = selected.clone() { *value = k; }
            }
            if clear {
                if let Some((k, _)) = &none { *value = k.clone(); }
            }
        });
    });
    *value != before
}
