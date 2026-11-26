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

/// Paints a single stamp tile matching the Surface layers tile style.
fn stamp_tile(ui: &mut egui::Ui, tex: &egui::TextureHandle, name: &str, tip: String, side: f32, active: bool) -> bool {
    use crate::theme;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(side + 8.0, side + 22.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let bg = if active {
            theme::ACCENT_SOFT
        } else if resp.hovered() {
            theme::HOVER
        } else {
            egui::Color32::from_rgb(42, 42, 44)
        };
        p.rect_filled(rect, 3.0, bg);
        if active {
            p.rect_stroke(rect, 3.0, egui::Stroke::new(1.0_f32, theme::ACCENT), egui::StrokeKind::Inside);
        }
        let art = egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 4.0), egui::vec2(side, side));
        p.image(tex.id(), art, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        p.rect_stroke(art, 2.0, egui::Stroke::new(1.0_f32, egui::Color32::from_black_alpha(110)), egui::StrokeKind::Inside);
        let mut job = egui::text::LayoutJob::simple_singleline(
            name.to_string(),
            egui::FontId::proportional(11.0),
            if active { theme::TEXT_BRIGHT } else { theme::TEXT },
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 6.0).max(16.0));
        let galley = p.layout_job(job);
        p.galley(egui::pos2(rect.center().x - galley.size().x * 0.5, rect.bottom() - 9.0 - galley.size().y * 0.5), galley, theme::TEXT);
    }
    resp.on_hover_text(tip).clicked()
}

/// Computes a responsive square thumbnail side so tiles fill the available width without overflowing.
fn responsive_tile_side(avail_width: f32) -> f32 {
    const GAP: f32 = 4.0;
    const PAD: f32 = 8.0;
    let w = avail_width.max(64.0);
    let per_row = ((w + GAP) / 68.0).floor().max(1.0);
    let tile_w = ((w + GAP) / per_row - GAP).floor();
    (tile_w - PAD).clamp(44.0, 74.0)
}

/// The stamps section of the Sculpt tab.
pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    use crate::theme;
    if !app.stamps.loaded {
        app.stamps.own = load_all();
        app.stamps.loaded = true;
    }
    section(ui, &icons::with(icons::STAMP, &t("Terrain stamps")), true, |ui| {
        crate::panels::about(ui, &t("Click a stamp, then on the map, to lay a form of the ground there. It is a terrain modifier: move, turn and scale it, change its height, or take it away again."));
        let ctx = ui.ctx().clone();
        let mut chosen = None;
        let side = responsive_tile_side(ui.available_width());

        ui.add_space(4.0);
        ui.label(egui::RichText::new(t("Forms")).font(icons::strong(12.5)).color(theme::TEXT_BRIGHT));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for form in StampForm::ALL {
                let tex = thumb(&ctx, &mut app.stamps.thumbs, form.name(), terrain_stamps::builtin(form));
                let (sz, h) = form.defaults();
                let label = t(form.name());
                let tip = format!("{label}\n{sz:.0} m · {h:+.0} m");
                if stamp_tile(ui, &tex, &label, tip, side, app.stamps.armed == Some(StampPick::Form(form))) {
                    chosen = Some(StampPick::Form(form));
                }
            }
        });

        ui.add_space(6.0);
        let own_count = app.stamps.own.len();
        let search_id = ui.make_persistent_id("stamps_search");
        let mut query: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(t("Your stamps")).font(icons::strong(12.5)).color(theme::TEXT_BRIGHT));
            if own_count > 0 {
                ui.label(egui::RichText::new(format!("({own_count})")).small().color(theme::TEXT_DIM));
            }
        });
        if own_count > 8 {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !query.is_empty() && ui.add(egui::Button::new(icons::X).min_size(egui::vec2(22.0, 22.0))).clicked() {
                        query.clear();
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .hint_text(format!("{} {}", icons::MAGNIFYING_GLASS, t("Search...")))
                            .desired_width(f32::INFINITY),
                    );
                });
            });
            ui.data_mut(|d| d.insert_temp(search_id, query.clone()));
        } else if !query.is_empty() {
            query.clear();
            ui.data_mut(|d| d.insert_temp(search_id, query.clone()));
        }

        if own_count == 0 {
            theme::card(ui, theme::BORDER, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(t("None yet: add grey pictures (white high, black low), or keep a piece of the terrain.")).small().color(theme::TEXT_DIM)).wrap());
            });
        } else {
            let needle = query.trim().to_ascii_lowercase();
            let own_side = responsive_tile_side((ui.available_width() - 12.0).max(64.0));
            egui::ScrollArea::vertical().id_salt("user_stamps").max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    for k in 0..app.stamps.own.len() {
                        let name = app.stamps.own[k].name.clone();
                        if !needle.is_empty() && !name.to_ascii_lowercase().contains(&needle) {
                            continue;
                        }
                        let key = format!("own_{}", app.stamps.own[k].path.display());
                        let (sz, h) = (app.stamps.own[k].info.size, app.stamps.own[k].info.height);
                        let tex = thumb(&ctx, &mut app.stamps.thumbs, &key, &app.stamps.own[k].stamp);
                        let tip = format!("{name}\n{sz:.0} m · {h:+.0} m");
                        if stamp_tile(ui, &tex, &name, tip, own_side, app.stamps.armed == Some(StampPick::Own(k))) {
                            chosen = Some(StampPick::Own(k));
                        }
                    }
                });
            });
        }

        if let Some(c) = chosen {
            app.stamps.armed = if app.stamps.armed == Some(c) { None } else { Some(c) };
        }
        if app.stamps.armed.is_some() {
            ui.add_space(4.0);
            theme::card(ui, theme::WARN.gamma_multiply(0.75), |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(theme::WARN, icons::STAMP);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(icons::X).min_size(egui::vec2(22.0, 22.0))).on_hover_text(t("Cancel")).clicked() {
                            app.stamps.armed = None;
                        }
                        ui.add(egui::Label::new(egui::RichText::new(t("Click on the map to lay the stamp; Esc cancels.")).small().color(theme::WARN)).wrap());
                    });
                });
            });
        }

        ui.add_space(6.0);
        theme::button_columns(ui, 2, |c| {
            let w0 = c[0].available_width();
            if c[0].add(egui::Button::new(icons::with(icons::PLUS, &t("Add pictures…"))).min_size(egui::vec2(w0, 24.0))).clicked() {
                add_pictures(app);
            }
            let w1 = c[1].available_width();
            if c[1].add(egui::Button::new(icons::with(icons::FOLDER_OPEN, &t("Open folder"))).min_size(egui::vec2(w1, 24.0))).clicked() {
                let _ = std::fs::create_dir_all(folder());
                app.open_folder(&folder());
            }
        });
        let w = ui.available_width();
        if ui.add(egui::Button::new(icons::with(icons::FLOPPY_DISK, &t("Keep the terrain of the selected polygon"))).min_size(egui::vec2(w, 24.0))).on_hover_text(t("Saves the ground inside the square round the selected polygon as a stamp of your own.")).clicked() {
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
