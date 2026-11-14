//! The satellite tools: baking the top views of objects and the clone brush, both kept as
//! satellite modifiers so that removing one brings back what was under it.
use crate::app::{App, Finish};
use crate::i18n::{t, tf};
use crate::theme::{row, section};
use dz_core::edit::Change;
use dz_core::satmod::{BakeObject, CloneBrush, PixelRect, SatKind, SatModifier};
use dz_core::terrain::RgbImage;
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;

/// A stroke of the clone brush in progress.
struct Stroke {
    id: u32,
    /// The modifiers before the stroke, for undo.
    before: Vec<SatModifier>,
    /// Source pixel minus painted pixel.
    offset: [i64; 2],
    last: [f32; 2],
}

pub struct SatTool {
    /// Radius of the brush in metres.
    pub radius: f32,
    pub hardness: f32,
    pub opacity: f32,
    /// Copy from the loaded picture rather than from the satellite itself.
    pub from_file: bool,
    pub file: Option<(PathBuf, Arc<RgbImage>)>,
    /// Pixel of the source the brush starts copying from.
    pub source: Option<[f32; 2]>,
    /// The clone modifier strokes go to.
    pub active: Option<u32>,
    /// Middle of the preview and source pixels per point of it; 0 fits the picture.
    view_center: [f32; 2],
    view_scale: f32,
    preview: Option<(egui::TextureHandle, [u64; 6])>,
    /// Counts changes of the satellite, so the preview follows them.
    revision: u64,
    stroke: Option<Stroke>,
    pub bake_plants: bool,
    /// Road parts too, which otherwise come from the road tools.
    pub bake_roads: bool,
    pub bake_selection: bool,
    /// River and pond parts too, their water in the colour of water.
    pub bake_water: bool,
    /// Leaves out what of an object lies in the ground.
    pub bake_above_ground: bool,
    /// How far baked objects are blended into the image under them, 0 to 1.
    pub bake_blend: f32,
    /// The blending slider is being dragged.
    blending: bool,
}

impl Default for SatTool {
    fn default() -> Self {
        Self { radius: 8.0, hardness: 0.5, opacity: 1.0, from_file: false, file: None, source: None, active: None, view_center: [0.0; 2], view_scale: 0.0, preview: None, revision: 0, stroke: None, bake_plants: false, bake_roads: false, bake_selection: true, bake_water: false, bake_above_ground: true, bake_blend: 0.35, blending: false }
    }
}

impl App {
    /// Shows a changed part of the satellite image and marks it for saving.
    pub fn satellite_changed(&mut self, r: PixelRect) {
        let Some(doc) = &mut self.doc else { return };
        doc.dirty.satellite = true;
        if let Some(s) = &doc.world.satellite {
            self.renderer.lock().unwrap().update_satellite(s, r);
        }
        // The sharp window round the camera is cut from the image again.
        self.detail_center = [f32::MIN, f32::MIN];
        self.sat.revision += 1;
    }
}

/// Pixel of the satellite image over a point of the map.
fn pixel_of(app: &App, x: f32, z: f32) -> Option<([f32; 2], f32)> {
    let d = app.doc.as_ref()?;
    let s = d.world.satellite.as_ref()?;
    let size = d.world.meta.size_m();
    Some(([x / size * s.width as f32, (1.0 - z / size) * s.height as f32], s.width as f32 / size))
}

pub fn begin_stroke(app: &mut App, x: f32, z: f32) {
    let Some((p, _)) = pixel_of(app, x, z) else {
        app.warn(t("The project has no satellite image"));
        return;
    };
    let Some(source) = app.sat.source else {
        app.warn(t("Choose the source point in the preview first"));
        return;
    };
    if app.sat.from_file && app.sat.file.is_none() {
        app.warn(t("Load a picture to copy from first"));
        return;
    }
    let Some(d) = app.doc.as_mut() else { return };
    let before = d.world.sat.modifiers.clone();
    let id = match app.sat.active.filter(|id| d.world.sat.modifiers.iter().any(|m| m.id == *id && m.kind == SatKind::Clone && m.enabled)) {
        Some(id) => id,
        None => {
            let n = d.world.sat.modifiers.iter().filter(|m| m.kind == SatKind::Clone).count() + 1;
            d.world.sat.add(SatKind::Clone, tf("Clone brush {0}", &[&n.to_string()]))
        }
    };
    app.sat.active = Some(id);
    app.sat.stroke = Some(Stroke { id, before, offset: [(source[0] - p[0]).round() as i64, (source[1] - p[1]).round() as i64], last: [f32::NAN; 2] });
    continue_stroke(app, x, z);
}

/// Paints from the last point of the stroke to this one. True when something was painted.
pub fn continue_stroke(app: &mut App, x: f32, z: f32) -> bool {
    let Some((p, per_metre)) = pixel_of(app, x, z) else { return false };
    let Some(mut stroke) = app.sat.stroke.take() else { return false };
    let brush = CloneBrush { radius: (app.sat.radius * per_metre).max(0.5), hardness: app.sat.hardness, opacity: app.sat.opacity };
    let file = app.sat.file.as_ref().filter(|_| app.sat.from_file).map(|f| f.1.clone());
    let from = if stroke.last[0].is_nan() { p } else { stroke.last };
    let length = ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt();
    let spacing = (brush.radius * 0.25).max(1.0);
    // Dabs a quarter of the radius apart make an even line.
    let steps = if stroke.last[0].is_nan() { 1 } else { (length / spacing) as usize };
    let mut touched: Option<PixelRect> = None;
    let mut last = from;
    if let Some(d) = app.doc.as_mut() {
        for i in 1..=steps {
            let k = if stroke.last[0].is_nan() { 1.0 } else { i as f32 * spacing / length.max(1e-6) };
            let c = [from[0] + (p[0] - from[0]) * k, from[1] + (p[1] - from[1]) * k];
            last = c;
            if let Some(r) = d.world.sat_clone_dab(stroke.id, c, &brush, file.as_deref(), stroke.offset) {
                touched = Some(touched.map_or(r, |t| t.union(r)));
            }
        }
    }
    if steps > 0 {
        stroke.last = last;
    }
    app.sat.stroke = Some(stroke);
    match touched {
        Some(r) => {
            app.satellite_changed(r);
            true
        }
        None => false,
    }
}

pub fn finish_stroke(app: &mut App) {
    let Some(stroke) = app.sat.stroke.take() else { return };
    if let Some(d) = app.doc.as_mut() {
        d.history.push(t("Clone brush"), Change::Satellite(stroke.before));
    }
}

/// Bakes the top views of the selected objects, or of all of them, into a new modifier.
pub fn bake(app: &mut App) {
    let Some(d) = &app.doc else { return };
    let Some((width, height)) = d.world.satellite.as_ref().map(|s| (s.width, s.height)) else {
        app.warn(t("The project has no satellite image"));
        return;
    };
    let w = &d.world;
    let selected = app.sat.bake_selection && !app.selection.is_empty();
    let (plants, roads, water) = (app.sat.bake_plants, app.sat.bake_roads, app.sat.bake_water);
    let wanted = |path: &str| -> bool {
        // Decals lie flat on the image already; road and water parts only when asked for.
        if path.contains("\\roads\\parts\\") {
            return roads;
        }
        if crate::models::is_water_model(path) {
            return water;
        }
        !crate::models::is_decal_model(path) && (plants || !crate::viewport::is_plant(path))
    };
    let mut objects = Vec::new();
    let mut add = |o: &dz_core::objects::Obj| {
        let path = w.objects.model_path(o);
        // Objects under the terrain cannot be seen from above.
        let underground = o.flags & dz_core::objects::FLAG_UNDERGROUND != 0;
        if w.object_visible(o) && !underground && wanted(path) {
            objects.push(BakeObject { object: *o, origin_y: dz_core::objects::ObjectStore::origin_y(o, &w.height), model: path.to_string() });
        }
    };
    if selected {
        for id in &app.selection {
            if let Some(o) = w.objects.get(*id) {
                add(o);
            }
        }
    } else {
        for (_, o) in w.objects.iter() {
            add(o);
        }
    }
    if objects.is_empty() {
        app.warn(t("No objects to bake"));
        return;
    }
    let (db, size, count) = (app.db.clone(), w.meta.size_m(), objects.len());
    let opacity = ((1.0 - app.sat.bake_blend.clamp(0.0, 1.0)) * 255.0).round() as u8;
    let opts = dz_core::satmod::BakeOptions { water, ground: app.sat.bake_above_ground.then(|| Arc::new(w.height.clone())) };
    app.spawn(t("Baking objects into the satellite"), move |job| {
        job.stage("Baking objects into the satellite");
        let tiles = dz_core::satmod::bake_objects_with(&objects, &db, width, height, size, crate::viewport::is_plant, &opts, job);
        Ok(Box::new(move |app: &mut App| {
            let Some(d) = app.doc.as_mut() else { return };
            if tiles.is_empty() {
                app.warn(t("None of the objects could be drawn"));
                return;
            }
            let before = d.world.sat.modifiers.clone();
            let n = before.iter().filter(|m| m.kind == SatKind::Bake).count() + 1;
            let id = d.world.sat.add(SatKind::Bake, tf("Baked objects {0}", &[&n.to_string()]));
            if let Some(m) = d.world.sat.modifiers.last_mut() {
                m.opacity = opacity;
            }
            let touched = d.world.sat_add_tiles(id, tiles);
            d.history.push(t("Bake objects"), Change::Satellite(before));
            // The new modifier is the one the blending slider now works on.
            app.sat.active = Some(id);
            if let Some(r) = touched {
                app.satellite_changed(r);
            }
            app.info(tf("Baked {0} objects into the satellite image", &[&count.to_string()]));
        }) as Finish)
    });
}

/// Rows of the satellite modifiers in the outliner.
pub fn outliner(app: &mut App, ui: &mut egui::Ui, filter: &str, fold: Option<bool>) {
    // In team work: only whose the outliner shows, with who made each.
    let team = crate::team_ui::active(app);
    let rows: Vec<(u32, String, bool, SatKind)> = app.doc.as_ref().map(|d| d.world.sat.modifiers.iter()
        .filter(|m| crate::team_ui::shown(app, d.world.sat_mod_author(m.id)))
        .map(|m| (m.id, crate::team_ui::with_author(team, &m.name, d.world.sat_mod_author(m.id)), m.enabled, m.kind)).collect()).unwrap_or_default();
    let (mut toggle, mut delete) = (None, None);
    crate::panels::outliner_section(fold, "Satellite modifiers", crate::icons::with(crate::icons::IMAGE, &tf("Satellite modifiers ({0})", &[&rows.len().to_string()]))).show(ui, |ui| {
        for (id, name, enabled, kind) in rows.iter().filter(|r| r.1.to_lowercase().contains(filter)) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if ui.add(egui::Button::new("👁").small().selected(*enabled)).on_hover_text(t("Visible")).clicked() {
                    toggle = Some((*id, !*enabled));
                }
                if ui.add(egui::Button::new("✖").small()).on_hover_text(t("Delete the modifier; what it covered comes back")).clicked() {
                    delete = Some(*id);
                }
                let row = ui.selectable_label(app.sat.active == Some(*id), name);
                if row.clicked() {
                    app.sat.active = Some(*id);
                    let _ = kind;
                }
                row.context_menu(|ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    if ui.button(t("Delete")).clicked() {
                        delete = Some(*id);
                        ui.close_menu();
                    }
                });
            });
        }
    });
    let Some(d) = app.doc.as_mut() else { return };
    let before = d.world.sat.modifiers.clone();
    let touched = match (toggle, delete) {
        (_, Some(id)) => {
            let r = d.world.sat_remove(id);
            d.history.push(t("Delete satellite modifier"), Change::Satellite(before));
            if app.sat.active == Some(id) {
                app.sat.active = None;
            }
            r
        }
        (Some((id, on)), None) => {
            let r = d.world.sat_set_enabled(id, on);
            d.history.push(t("Satellite modifier"), Change::Satellite(before));
            r
        }
        _ => return,
    };
    match touched {
        Some(r) => app.satellite_changed(r),
        // A modifier with nothing in it changes no pixels, but the project did change.
        None => {
            if let Some(d) = app.doc.as_mut() {
                d.dirty.satellite = true;
            }
        }
    }
}

/// The picture the brush copies from, zoomed and moved by the user; a click sets the source point.
fn preview(app: &mut App, ui: &mut egui::Ui) {
    let side = ui.available_width().clamp(160.0, 420.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag());
    let file = app.sat.file.as_ref().filter(|_| app.sat.from_file).map(|f| f.1.clone());
    let current = app.doc.as_ref().and_then(|d| d.world.satellite.as_ref());
    let Some(img) = file.as_deref().or(current) else {
        ui.painter().rect_filled(rect, 2.0, egui::Color32::from_gray(30));
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, t("No picture"), egui::FontId::proportional(13.0), egui::Color32::GRAY);
        return;
    };
    let (w, h) = (img.width as f32, img.height as f32);
    if app.sat.view_scale <= 0.0 {
        app.sat.view_scale = w.max(h) / side;
        app.sat.view_center = [w * 0.5, h * 0.5];
    }
    let to_source = |tool: &SatTool, p: egui::Pos2| [tool.view_center[0] + (p.x - rect.center().x) * tool.view_scale, tool.view_center[1] + (p.y - rect.center().y) * tool.view_scale];
    // The wheel zooms about the cursor and must not scroll the panel under it.
    if let Some(p) = resp.hover_pos() {
        let wheel = ui.input_mut(|i| {
            let d = i.smooth_scroll_delta.y;
            i.smooth_scroll_delta = egui::Vec2::ZERO;
            d
        });
        if wheel != 0.0 {
            let at = to_source(&app.sat, p);
            app.sat.view_scale = (app.sat.view_scale * (-wheel * 0.004).exp()).clamp(0.05, w.max(h) / side * 1.5);
            app.sat.view_center = [at[0] - (p.x - rect.center().x) * app.sat.view_scale, at[1] - (p.y - rect.center().y) * app.sat.view_scale];
        }
    }
    if resp.dragged() {
        let d = resp.drag_delta();
        app.sat.view_center = [app.sat.view_center[0] - d.x * app.sat.view_scale, app.sat.view_center[1] - d.y * app.sat.view_scale];
    }
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let s = to_source(&app.sat, p);
            if s[0] >= 0.0 && s[1] >= 0.0 && s[0] < w && s[1] < h {
                app.sat.source = Some(s);
            }
        }
    }
    app.sat.view_center = [app.sat.view_center[0].clamp(0.0, w), app.sat.view_center[1].clamp(0.0, h)];
    // The visible part is sampled anew whenever the view or the picture changes.
    let n = (side * ui.ctx().pixels_per_point()).round().max(16.0) as usize;
    let key = [file.as_ref().map_or(0, |f| Arc::as_ptr(f) as u64), app.sat.view_center[0].to_bits() as u64, app.sat.view_center[1].to_bits() as u64, app.sat.view_scale.to_bits() as u64, n as u64, if file.is_some() { 0 } else { app.sat.revision }];
    if app.sat.preview.as_ref().map(|p| p.1) != Some(key) {
        let step = app.sat.view_scale * side / n as f32;
        let (x0, y0) = (app.sat.view_center[0] - step * n as f32 * 0.5, app.sat.view_center[1] - step * n as f32 * 0.5);
        let mut pixels = Vec::with_capacity(n * n);
        for y in 0..n {
            let sy = y0 + (y as f32 + 0.5) * step;
            for x in 0..n {
                let sx = x0 + (x as f32 + 0.5) * step;
                pixels.push(if sx >= 0.0 && sy >= 0.0 && sx < w && sy < h {
                    let o = (sy as usize * img.width + sx as usize) * 3;
                    egui::Color32::from_rgb(img.data[o], img.data[o + 1], img.data[o + 2])
                } else {
                    egui::Color32::from_gray(24)
                });
            }
        }
        let image = egui::ColorImage { size: [n, n], pixels };
        let texture = ui.ctx().load_texture("satellite-source", image, egui::TextureOptions::NEAREST);
        app.sat.preview = Some((texture, key));
    }
    let painter = ui.painter_at(rect);
    if let Some((texture, _)) = &app.sat.preview {
        painter.image(texture.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
    // The source point with the outline of the brush, at the size it has in this view.
    if let Some(s) = app.sat.source {
        let at = egui::pos2(rect.center().x + (s[0] - app.sat.view_center[0]) / app.sat.view_scale, rect.center().y + (s[1] - app.sat.view_center[1]) / app.sat.view_scale);
        let per_metre = app.doc.as_ref().and_then(|d| d.world.satellite.as_ref().map(|s| s.width as f32 / d.world.meta.size_m())).unwrap_or(1.0);
        let r = (app.sat.radius * per_metre / app.sat.view_scale).max(3.0);
        for (width, color) in [(3.0f32, egui::Color32::BLACK), (1.5f32, egui::Color32::from_rgb(255, 210, 60))] {
            painter.circle_stroke(at, r, egui::Stroke::new(width, color));
            painter.line_segment([at - egui::vec2(6.0, 0.0), at + egui::vec2(6.0, 0.0)], egui::Stroke::new(width, color));
            painter.line_segment([at - egui::vec2(0.0, 6.0), at + egui::vec2(0.0, 6.0)], egui::Stroke::new(width, color));
        }
    }
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0f32, egui::Color32::from_gray(70)), egui::StrokeKind::Inside);
}

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &crate::icons::with(crate::icons::STAMP, &t("Bake objects")), true, |ui| {
        ui.weak(t("Draws the objects as seen from above into the satellite image, as a modifier that can be hidden or deleted."));
        let selected = app.selection.len();
        ui.checkbox(&mut app.sat.bake_selection, tf("Only the selected objects ({0})", &[&selected.to_string()]));
        ui.checkbox(&mut app.sat.bake_plants, t("Trees and bushes too"));
        ui.checkbox(&mut app.sat.bake_roads, t("Road parts too")).on_hover_text(t("Paints the asphalt and verges of the road models into the image, as they look from above."));
        ui.checkbox(&mut app.sat.bake_water, t("Water parts too")).on_hover_text(t("Paints river and pond parts into the image, their water in the colour of water."));
        ui.checkbox(&mut app.sat.bake_above_ground, t("Only what is above the ground")).on_hover_text(t("Leaves out the parts of an object sunk into the terrain, so a pier half in the ground shows only its part above it."));
        // The slider sets the blending of the next bake, and of the baked modifier chosen in the outliner.
        let chosen = app.doc.as_ref().and_then(|d| app.sat.active.and_then(|id| d.world.sat.modifiers.iter().find(|m| m.id == id && m.kind == SatKind::Bake)).map(|m| (m.id, m.name.clone(), 1.0 - m.opacity as f32 / 255.0)));
        if let Some((_, _, blend)) = &chosen {
            if !ui.ctx().is_using_pointer() {
                app.sat.bake_blend = *blend;
            }
        }
        let mut changed = None;
        row(ui, &t("Blending"), |ui| {
            let resp = ui.add(egui::Slider::new(&mut app.sat.bake_blend, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0))).on_hover_text(t("How far the baked objects fade into the satellite image under them."));
            if resp.changed() {
                changed = Some(resp.drag_started() || !resp.dragged());
            }
        });
        if let (Some(first), Some((id, _, _))) = (changed, &chosen) {
            let opacity = ((1.0 - app.sat.bake_blend) * 255.0).round() as u8;
            let touched = app.doc.as_mut().and_then(|d| {
                // One undo step for a whole drag of the slider.
                if first || !app.sat.blending {
                    d.history.push(t("Satellite modifier"), Change::Satellite(d.world.sat.modifiers.clone()));
                }
                d.world.sat_set_opacity(*id, opacity)
            });
            app.sat.blending = true;
            if let Some(r) = touched {
                app.satellite_changed(r);
            }
        }
        if !ui.ctx().is_using_pointer() {
            app.sat.blending = false;
        }
        match &chosen {
            Some((_, name, _)) => ui.weak(tf("Changes: {0}", &[name])),
            None => ui.weak(t("Applies to the next bake. Choose a baked modifier in the outliner to change it.")),
        };
        if ui.button(t("Bake")).clicked() {
            bake(app);
        }
    });
    section(ui, &crate::icons::with(crate::icons::PAINT_BRUSH, &t("Clone brush")), true, |ui| {
        crate::panels::hint(ui, &t("Click in the preview to choose where to copy from, then paint on the map. Wheel zooms the preview, dragging moves it."));
        crate::panels::brush_hints(app, ui, None, false, true, true);
        row(ui, &t("Copy from"), |ui| {
            let before = app.sat.from_file;
            ui.selectable_value(&mut app.sat.from_file, false, t("Satellite"));
            ui.selectable_value(&mut app.sat.from_file, true, t("Picture"));
            if before != app.sat.from_file {
                // Another picture: its own point and view.
                app.sat.source = None;
                app.sat.view_scale = 0.0;
            }
        });
        if app.sat.from_file {
            ui.horizontal(|ui| {
                if ui.button(t("Load picture…")).clicked() {
                    if let Some(f) = rfd::FileDialog::new().add_filter("Image", &["png", "bmp", "tga", "jpg", "jpeg", "tif"]).pick_file() {
                        match RgbImage::load(&f) {
                            Ok(img) => {
                                app.sat.file = Some((f, Arc::new(img)));
                                app.sat.source = None;
                                app.sat.view_scale = 0.0;
                            }
                            Err(e) => app.warn(format!("{e:#}")),
                        }
                    }
                }
                if let Some((path, img)) = &app.sat.file {
                    ui.weak(format!("{}  {} × {}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), img.width, img.height));
                }
            });
        }
        preview(app, ui);
        ui.horizontal(|ui| {
            if ui.small_button(t("Whole picture")).clicked() {
                app.sat.view_scale = 0.0;
            }
            if !app.sat.from_file && ui.small_button(t("At the camera")).clicked() {
                if let Some((p, _)) = pixel_of(app, app.camera.pos[0], app.camera.pos[2]) {
                    app.sat.view_center = p;
                    app.sat.view_scale = app.sat.view_scale.min(1.0).max(0.25);
                }
            }
            match app.sat.source {
                Some(s) => ui.weak(tf("Source: {0}, {1}", &[&(s[0] as i64).to_string(), &(s[1] as i64).to_string()])),
                None => ui.colored_label(crate::theme::WARN, t("No source point yet")),
            };
        });
        row(ui, &t("Radius"), |ui| {
            ui.add(egui::Slider::new(&mut app.sat.radius, 0.5..=500.0).logarithmic(true).suffix(" m"));
        });
        row(ui, &t("Hardness"), |ui| {
            ui.add(egui::Slider::new(&mut app.sat.hardness, 0.0..=1.0));
        });
        row(ui, &t("Opacity"), |ui| {
            ui.add(egui::Slider::new(&mut app.sat.opacity, 0.05..=1.0));
        });
        let active = app.doc.as_ref().and_then(|d| app.sat.active.and_then(|id| d.world.sat.modifiers.iter().find(|m| m.id == id && m.kind == SatKind::Clone)).map(|m| m.name.clone()));
        ui.horizontal(|ui| {
            ui.weak(match &active {
                Some(name) => tf("Strokes go to: {0}", &[name]),
                None => t("The next stroke starts a new modifier"),
            });
            if active.is_some() && ui.small_button(t("New modifier")).clicked() {
                app.sat.active = None;
            }
        });
    });
}
