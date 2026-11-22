//! Terrain stamps in the editor: the forms that come with it and the user's own, kept as grey
//! pictures in `%LOCALAPPDATA%\DZWorldEditor\stamps` (as brushes are in `brushes`), with a
//! `.json` beside a picture for the size and height it is laid with. A click on a stamp and
//! then on the map lays it as a terrain modifier.
use crate::app::App;
use crate::i18n::{t, tf};
use crate::icons;
use crate::theme::{row, section};
use dz_core::edit::Stamp;
use dz_core::terrain_stamps::{self, StampForm, StampInfo};
use dz_core::world::{ModMode, ModShape, TerrainModifier};
use eframe::egui;
use std::path::{Path, PathBuf};

const EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "bmp", "tga", "tif"];

pub struct UserStamp {
    pub name: String,
    pub path: PathBuf,
    pub stamp: Stamp,
    pub info: StampInfo,
}

/// The stamp a click on the map lays next.
#[derive(Clone, Copy, PartialEq)]
pub enum StampPick {
    Form(StampForm),
    Own(usize),
}

#[derive(Default)]
pub struct StampTool {
    pub own: Vec<UserStamp>,
    loaded: bool,
    pub armed: Option<StampPick>,
    thumbs: std::collections::HashMap<String, egui::TextureHandle>,
}

pub fn folder() -> PathBuf {
    crate::settings::data_dir().join("stamps")
}

fn info_path(picture: &Path) -> PathBuf {
    picture.with_extension("json")
}

fn load(path: &Path) -> Option<UserStamp> {
    let stamp = terrain_stamps::from_picture(path).ok()?;
    let info = std::fs::read(info_path(path)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let name = path.file_stem()?.to_string_lossy().into_owned();
    Some(UserStamp { name, path: path.to_path_buf(), stamp, info })
}

fn load_all() -> Vec<UserStamp> {
    let Ok(rd) = std::fs::read_dir(folder()) else { return Vec::new() };
    let mut out: Vec<UserStamp> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| EXTENSIONS.contains(&x.to_string_lossy().to_lowercase().as_str()))).filter_map(|p| load(&p)).collect();
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

/// A small picture of a stamp: light high, dark low.
fn thumb(ctx: &egui::Context, cache: &mut std::collections::HashMap<String, egui::TextureHandle>, key: &str, stamp: &Stamp) -> egui::TextureHandle {
    if let Some(t) = cache.get(key) {
        return t.clone();
    }
    const N: usize = 48;
    let mut px = Vec::with_capacity(N * N);
    for y in 0..N {
        for x in 0..N {
            let v = stamp.sample((x as f32 + 0.5) / N as f32 * 2.0 - 1.0, (y as f32 + 0.5) / N as f32 * 2.0 - 1.0);
            let g = ((v * 0.5 + 0.5) * 230.0 + 15.0) as u8;
            px.push(egui::Color32::from_rgb(g, (g as f32 * 0.95) as u8, (g as f32 * 0.85) as u8));
        }
    }
    let tex = ctx.load_texture(format!("stamp_{key}"), egui::ColorImage { size: [N, N], pixels: px }, egui::TextureOptions::LINEAR);
    cache.insert(key.to_string(), tex.clone());
    tex
}

/// The stamps section of the Sculpt tab.
pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    if !app.stamps.loaded {
        app.stamps.own = load_all();
        app.stamps.loaded = true;
    }
    section(ui, &icons::with(icons::STAMP, &t("Terrain stamps")), true, |ui| {
        // As many tiles a row as the panel holds (the tools panel is at most 460 wide).
        let columns = ((ui.available_width().min(440.0) / 66.0).floor() as usize).max(1);
        crate::panels::about(ui, &t("Click a stamp, then on the map, to lay a form of the ground there. It is a terrain modifier: move, turn and scale it, change its height, or take it away again."));
        let ctx = ui.ctx().clone();
        let mut chosen = None;
        let tile = |ui: &mut egui::Ui, tex: &egui::TextureHandle, name: &str, on: bool| -> bool {
            let r = ui.vertical(|ui| {
                ui.set_width(62.0);
                let img = ui.add(egui::ImageButton::new((tex.id(), egui::vec2(56.0, 56.0))).selected(on));
                ui.add(egui::Label::new(egui::RichText::new(name).small()).truncate());
                img.clicked()
            });
            r.inner
        };
        ui.label(egui::RichText::new(t("Forms")).strong());
        egui::Grid::new("stamp_forms").spacing(egui::vec2(4.0, 4.0)).show(ui, |ui| {
            for (k, form) in StampForm::ALL.into_iter().enumerate() {
                let tex = thumb(&ctx, &mut app.stamps.thumbs, form.name(), terrain_stamps::builtin(form));
                if tile(ui, &tex, &t(form.name()), app.stamps.armed == Some(StampPick::Form(form))) { chosen = Some(StampPick::Form(form)); }
                if (k + 1) % columns == 0 { ui.end_row(); }
            }
        });
        ui.label(egui::RichText::new(t("Your stamps")).strong());
        if app.stamps.own.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new(t("None yet: add grey pictures (white high, black low), or keep a piece of the terrain.")).weak()).wrap());
        }
        egui::Grid::new("stamp_own").spacing(egui::vec2(4.0, 4.0)).show(ui, |ui| {
            for k in 0..app.stamps.own.len() {
                let (key, name) = (format!("own_{}", app.stamps.own[k].path.display()), app.stamps.own[k].name.clone());
                let tex = thumb(&ctx, &mut app.stamps.thumbs, &key, &app.stamps.own[k].stamp);
                if tile(ui, &tex, &name, app.stamps.armed == Some(StampPick::Own(k))) { chosen = Some(StampPick::Own(k)); }
                if (k + 1) % columns == 0 { ui.end_row(); }
            }
        });
        if let Some(c) = chosen {
            app.stamps.armed = if app.stamps.armed == Some(c) { None } else { Some(c) };
        }
        if app.stamps.armed.is_some() {
            ui.colored_label(crate::theme::WARN, t("Click on the map to lay the stamp; Esc cancels."));
        }
        ui.horizontal(|ui| {
            if ui.button(icons::with(icons::PLUS, &t("Add pictures…"))).clicked() {
                add_pictures(app);
            }
            if ui.button(icons::with(icons::FOLDER_OPEN, &t("Open folder"))).clicked() {
                let _ = std::fs::create_dir_all(folder());
                app.open_folder(&folder());
            }
        });
        if ui.button(icons::with(icons::FLOPPY_DISK, &t("Keep the terrain of the selected polygon"))).on_hover_text(t("Saves the ground inside the square round the selected polygon as a stamp of your own.")).clicked() {
            keep_terrain(app);
        }
    });
}

fn add_pictures(app: &mut App) {
    let Some(files) = rfd::FileDialog::new().add_filter("Image", &EXTENSIONS).pick_files() else { return };
    let dir = folder();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        app.warn(format!("{e:#}"));
        return;
    }
    for f in files {
        let Some(name) = f.file_name() else { continue };
        if let Err(e) = std::fs::copy(&f, dir.join(name)) {
            app.warn(format!("{}: {e}", f.display()));
        }
    }
    reload(app);
}

fn reload(app: &mut App) {
    app.stamps.own = load_all();
    app.stamps.thumbs.retain(|k, _| !k.starts_with("own_"));
    app.stamps.armed = None;
}

/// Saves the ground of the square round the first selected polygon as a stamp of the user's.
fn keep_terrain(app: &mut App) {
    let Some(d) = &app.doc else { return };
    let poly = app.shape.selection.iter().filter_map(|(l, s)| d.world.shape_layers.get(*l)?.shapes.get(*s)).find(|s| s.kind == dz_core::shapes::ShapeKind::Polygon);
    let Some(poly) = poly else {
        app.warn(t("Select a polygon in the Shapes tool round the ground to keep"));
        return;
    };
    let b = poly.points.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
    let size = (b[2] - b[0]).max(b[3] - b[1]).max(4.0);
    let (x, z) = ((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5);
    let (stamp, height) = terrain_stamps::from_terrain(&d.world.height, x, z, size, 0.0);
    let dir = folder();
    let _ = std::fs::create_dir_all(&dir);
    let n = (1..).find(|n| !dir.join(format!("{} {n}.png", t("Terrain"))).exists()).unwrap_or(1);
    let path = dir.join(format!("{} {n}.png", t("Terrain")));
    let info = StampInfo { size, height };
    let saved = terrain_stamps::to_png16(&stamp, &path).and_then(|_| Ok(std::fs::write(info_path(&path), serde_json::to_vec_pretty(&info)?)?));
    match saved {
        Ok(()) => {
            reload(app);
            app.info(tf("Kept as the stamp {0}", &[&path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()]));
        }
        Err(e) => app.warn(format!("{e:#}")),
    }
}

/// The name, raster, size and height of the stamp to lay.
fn picked(app: &App, pick: StampPick) -> Option<(String, Stamp, f32, f32)> {
    match pick {
        StampPick::Form(f) => {
            let (size, height) = f.defaults();
            Some((t(f.name()), terrain_stamps::builtin(f).clone(), size, height))
        }
        StampPick::Own(k) => app.stamps.own.get(k).map(|s| (s.name.clone(), s.stamp.clone(), s.info.size, s.info.height)),
    }
}

/// The side of the armed stamp, for the outline under the cursor.
pub fn armed_size(app: &App) -> Option<f32> {
    picked(app, app.stamps.armed?).map(|p| p.2)
}

/// Lays the armed stamp at a point of the map, as one undo step.
pub fn lay(app: &mut App, x: f32, z: f32) {
    let Some(pick) = app.stamps.armed else { return };
    let Some((name, stamp, size, height)) = picked(app, pick) else { return };
    let Some(doc) = app.doc.as_mut() else { return };
    let id = doc.world.new_modifier_id();
    let m = TerrainModifier { id, name, enabled: true, x, z, yaw: 0.0, shape: ModShape::Stamp { stamp, size, height }, mode: ModMode::Offset, target: None, amount: 0.0, falloff: 0.0, attached: None, follow: false };
    let reach = m.reach();
    app.set_modifiers(&t("Lay a terrain stamp"), |list| list.push(m));
    app.selected_modifier = Some(id);
    // Roads over it follow the new ground.
    app.regrade_routes_in([x - reach, z - reach, x + reach, z + reach]);
}

/// The size, height and turn of a stamp, in the details of its modifier.
pub fn details(ui: &mut egui::Ui, size: &mut f32, height: &mut f32, yaw: &mut f32) {
    row(ui, &t("Size m"), |ui| { ui.add(crate::theme::ValueField::new(size, 4.0..=4000.0).logarithmic(true)); });
    row(ui, &t("Height m"), |ui| { ui.add(crate::theme::ValueField::new(height, -300.0..=300.0)).on_hover_text(t("How high the highest part of the stamp rises; less than 0 turns it upside down.")); });
    row(ui, &t("Turn"), |ui| { ui.add(crate::theme::ValueField::new(yaw, -180.0..=180.0).suffix("°")); });
}
