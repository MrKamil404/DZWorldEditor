//! "Go to": the window that flies the camera to a key point, a position or a square of the
//! game's map grid, and the flight itself, which the outliner and the map check use too. What
//! the text means is read in `dz_core::goto`.
use crate::app::App;
use crate::i18n::{t, tf};
use crate::icons;
use dz_core::goto::{Found, FoundKind, Spot};
use eframe::egui;

#[derive(Default)]
pub struct GoTo {
    pub open: bool,
    pub text: String,
    /// The field takes the keyboard when the window opens.
    pub focus: bool,
}

/// Opens the window with the field ready to type in.
pub fn open(app: &mut App) {
    app.goto.open = true;
    app.goto.focus = true;
}

/// Puts the camera over `spot`, in the 3D view and in the 2D view.
pub fn fly_to(app: &mut App, spot: Spot) {
    let Some(d) = &app.doc else { return };
    let ground = d.world.height.sample(spot.x, spot.z).max(d.world.meta.sea_level);
    let distance = (spot.radius * 2.0).clamp(40.0, 1500.0);
    let f = app.camera.forward();
    app.camera.pos = [spot.x - f[0] * distance, (ground - f[1] * distance).max(ground + 2.0), spot.z - f[2] * distance];
    app.top_center = [spot.x, spot.z];
    // A small place keeps the zoom if it is closer already; a large one is shown whole.
    let fit = (spot.radius * 2.4 / 700.0).max(0.5);
    app.top_mpp = if spot.radius > 100.0 { fit } else { app.top_mpp.min(fit) };
}

/// The outliner's menu entry that flies the camera to a thing; `spot` is `None` when the thing
/// has no place.
pub fn menu(ui: &mut egui::Ui, spot: Option<Spot>) -> Option<Spot> {
    let clicked = ui.add_enabled(spot.is_some(), egui::Button::new(icons::with(icons::CROSSHAIR, &t("Move the camera here")))).clicked();
    if clicked { ui.close_menu(); }
    spot.filter(|_| clicked)
}

fn describe(f: &Found) -> String {
    let at = format!("{:.0}, {:.0}", f.spot.x, f.spot.z);
    match f.kind {
        FoundKind::Cell => tf("Grid square {0} {1}", &[&format!("{:03}", (f.spot.x / dz_core::goto::GAME_GRID) as u32), &format!("{:03}", (f.spot.z / dz_core::goto::GAME_GRID) as u32)]),
        FoundKind::Position => tf("Position {0}", &[&at]),
        FoundKind::KeyPoint => format!("{}  ({at})", f.label),
    }
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.goto.open {
        return;
    }
    if app.doc.is_none() {
        app.goto.open = false;
        return;
    }
    let mut open = true;
    let mut go: Option<Found> = None;
    egui::Window::new(icons::with(icons::CROSSHAIR, &t("Go to"))).id(egui::Id::new("goto")).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_TOP, [0.0, 90.0]).show(ctx, |ui| {
        ui.set_width(340.0);
        let edit = ui.add(egui::TextEdit::singleline(&mut app.goto.text).hint_text(t("Key point, 045 123 or 4500, 12300")).desired_width(f32::INFINITY));
        if app.goto.focus {
            edit.request_focus();
            app.goto.focus = false;
        }
        let found = app.doc.as_ref().map(|d| dz_core::goto::find(&d.world, &app.goto.text)).unwrap_or_default();
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.goto.open = false;
        }
        if enter {
            go = found.first().cloned();
        }
        ui.add_space(4.0);
        if found.is_empty() {
            let text = if app.goto.text.trim().is_empty() { t("A key point of the map, a square of the game's map grid (045 123) or metres east and north (4500, 12300).") } else { t("Nothing found") };
            ui.weak(text);
        }
        for f in &found {
            if ui.selectable_label(false, describe(f)).clicked() {
                go = Some(f.clone());
            }
        }
    });
    if !open {
        app.goto.open = false;
    }
    if let Some(f) = go {
        fly_to(app, f.spot);
        app.goto.open = false;
    }
}
