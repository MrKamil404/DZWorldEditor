//! The satellite tools: baking the top views of objects, the clone brush and the adjust brush,
//! all kept as satellite modifiers so that removing one brings back what was under it.
use crate::app::{App, Finish};
use crate::i18n::{t, tf};
use crate::theme::{row, section};
use dz_core::edit::Change;
use dz_core::satmod::{Adjust, BakeObject, BlendMode, CloneBrush, PaintOp, PixelRect, SatKind, SatModifier, SourceMap};
use std::collections::HashMap;
use dz_core::terrain::RgbImage;
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;

/// A stroke of the clone brush in progress.
struct Stroke {
    id: u32,
    /// The modifiers before the stroke, for undo.
    before: Vec<SatModifier>,
    /// Where each painted pixel reads its colour.
    map: SourceMap,
    /// Takes back what the modifier painted instead of painting.
    erase: bool,
    /// An adjust stroke: the colour each pixel had when it first reached it.
    adjust: Option<HashMap<usize, [u8; 3]>>,
    last: [f32; 2],
}

/// The brush the satellite tool paints with.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SatBrush {
    Clone,
    Adjust,
    Colour,
}

impl SatBrush {
    /// The kind of modifier its strokes go into.
    fn kind(self) -> SatKind {
        match self {
            SatBrush::Clone => SatKind::Clone,
            SatBrush::Adjust => SatKind::Adjust,
            SatBrush::Colour => SatKind::Paint,
        }
    }
}

pub struct SatTool {
    pub brush: SatBrush,
    /// What the adjust brush does to the colours.
    pub adjust: Adjust,
    /// How far the adjust brush blurs, in metres; 0 for not at all.
    pub blur: f32,
    /// The colour of the colour brush and how it is laid over the image.
    pub colour: [u8; 3],
    pub blend: BlendMode,
    /// Fills of shapes: those selected (None) or all of a shape layer; how soft their edges
    /// are, and how wide lines are painted, in metres.
    pub fill_layer: Option<usize>,
    /// A new correction goes over the whole image; otherwise over the shapes as for fills.
    pub correct_whole: bool,
    /// The settings of the chosen correction while they are changed, and its id.
    correcting: Option<(u32, Adjust)>,
    pub feather: f32,
    pub line_width: f32,
    /// Radius of the brush in metres.
    pub radius: f32,
    pub hardness: f32,
    pub opacity: f32,
    /// Copy from the loaded picture rather than from the satellite itself.
    pub from_file: bool,
    pub file: Option<(PathBuf, Arc<RgbImage>)>,
    /// Pixel of the source the brush starts copying from.
    pub source: Option<[f32; 2]>,
    /// Keeps the source the same distance from the brush from one stroke to the next, as
    /// set by the first stroke after the source was chosen; off, every stroke starts copying
    /// from the source point again.
    pub aligned: bool,
    /// Where the first aligned stroke began on the satellite, and the source point then.
    anchor: Option<([f32; 2], [f32; 2])>,
    /// Turn of the copy in degrees, and how much larger it comes out than the source.
    pub turn: f32,
    pub scale: f32,
    /// The copy takes on the average colour of where it is painted.
    pub match_colour: bool,
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
    /// A satellite made from the mask: its settings, whether it covers the whole map (or the
    /// chosen shapes), and the metres a pixel of a new image covers when the project has none.
    pub gen: dz_core::satgen::SatGenSettings,
    /// The colour of each surface layer from above, by layer.
    pub gen_colours: Vec<[u8; 3]>,
    pub gen_whole: bool,
    pub gen_mpp: f32,
    /// Surfaces read from the satellite: the settings, whether over the whole map, and the
    /// small copy of the image its preview is made from, with the revision it was made at.
    pub classify: dz_core::satgen::SatClassify,
    pub classify_whole: bool,
    class_small: Option<(u64, Arc<RgbImage>)>,
    class_preview: Option<(egui::TextureHandle, u64)>,
    class_share: [f32; dz_core::satgen::MAX_PICKED],
    /// How many colours "Find colours" looks for.
    class_find: usize,
    /// The preview of picked colours shows the satellite itself, to pick from.
    class_picture: bool,
}

impl Default for SatTool {
    fn default() -> Self {
        Self { brush: SatBrush::Clone, adjust: Adjust::default(), blur: 0.0, colour: [122, 104, 78], blend: BlendMode::Tint, fill_layer: None, correct_whole: true, correcting: None, feather: 3.0, line_width: 4.0, radius: 8.0, hardness: 0.5, opacity: 1.0, from_file: false, file: None, source: None, aligned: true, anchor: None, turn: 0.0, scale: 1.0, match_colour: false, active: None, view_center: [0.0; 2], view_scale: 0.0, preview: None, revision: 0, stroke: None, bake_plants: false, bake_roads: false, bake_selection: true, bake_water: false, bake_above_ground: true, bake_blend: 0.35, blending: false, gen: Default::default(), gen_colours: Vec::new(), gen_whole: true, gen_mpp: 1.0, classify: Default::default(), classify_whole: true, class_small: None, class_preview: None, class_share: [0.0; dz_core::satgen::MAX_PICKED], class_find: 4, class_picture: false }
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

/// Where the brush reads the colour for each pixel, were a stroke to start at pixel `p` now.
fn map_for(tool: &SatTool, p: [f32; 2]) -> Option<SourceMap> {
    let source = tool.source?;
    let (to, from) = match tool.anchor.filter(|_| tool.aligned) {
        Some(a) => a,
        None => (p, source),
    };
    Some(SourceMap { to, from, turn: tool.turn, scale: tool.scale.max(0.05) })
}

/// What the adjust or colour brush does to each pixel, with the image `per_metre` pixels a metre.
fn paint_op(tool: &SatTool, per_metre: f32) -> PaintOp {
    match tool.brush {
        SatBrush::Colour => PaintOp::Colour(tool.colour, tool.blend),
        _ => PaintOp::Adjust(tool.adjust, tool.blur * per_metre),
    }
}

/// True when the brush would leave the image as it is.
fn does_nothing(tool: &SatTool) -> bool {
    tool.brush == SatBrush::Adjust && tool.adjust.is_neutral() && tool.blur <= 0.0
}

/// The modifier the strokes of the current brush go into: the chosen one if it is of the
/// brush's kind, or a new one.
fn brush_modifier(app: &mut App) -> Option<u32> {
    let kind = app.sat.brush.kind();
    let d = app.doc.as_mut()?;
    let id = match app.sat.active.filter(|id| d.world.sat.modifiers.iter().any(|m| m.id == *id && m.kind == kind && m.enabled)) {
        Some(id) => id,
        None => {
            let n = d.world.sat.modifiers.iter().filter(|m| m.kind == kind).count() + 1;
            let name = match app.sat.brush {
                SatBrush::Clone => "Clone brush {0}",
                SatBrush::Adjust => "Adjust brush {0}",
                SatBrush::Colour => "Colour brush {0}",
            };
            d.world.sat.add(kind, tf(name, &[&n.to_string()]))
        }
    };
    app.sat.active = Some(id);
    Some(id)
}

/// Takes the colour of the satellite at a point of the map for the colour brush.
pub fn pick_colour(app: &mut App, x: f32, z: f32) {
    let Some((p, _)) = pixel_of(app, x, z) else { return };
    let Some(img) = app.doc.as_ref().and_then(|d| d.world.satellite.as_ref()) else { return };
    let (x, y) = (p[0] as usize, p[1] as usize);
    if x < img.width && y < img.height {
        let o = (y * img.width + x) * 3;
        app.sat.colour = [img.data[o], img.data[o + 1], img.data[o + 2]];
    }
}

/// Sets the point to copy from, by pixel of the source; strokes after it start from there.
pub fn set_source(app: &mut App, source: [f32; 2]) {
    app.sat.source = Some(source);
    app.sat.anchor = None;
}

/// Sets the point to copy from at a point of the map: a click with the pick key held.
pub fn pick_source(app: &mut App, x: f32, z: f32) {
    if app.sat.from_file {
        app.warn(t("Choose the point in the preview: the brush copies from the picture"));
        return;
    }
    let Some((p, _)) = pixel_of(app, x, z) else { return };
    set_source(app, p);
    app.info(t("Copying from here"));
}

/// Where on the map the brush copies from with the cursor at (x, z), and the radius there in
/// metres; only when it copies from the satellite itself.
pub fn source_on_map(app: &App, x: f32, z: f32) -> Option<([f32; 2], f32)> {
    if app.sat.from_file || app.sat.brush != SatBrush::Clone {
        return None;
    }
    let (p, per_metre) = pixel_of(app, x, z)?;
    let map = app.sat.stroke.as_ref().filter(|s| !s.erase).map(|s| s.map).or_else(|| map_for(&app.sat, p))?;
    let s = map.apply(p);
    let size = app.doc.as_ref()?.world.meta.size_m();
    let width = size * per_metre;
    Some(([s[0] / width * size, (1.0 - s[1] / width) * size], app.sat.radius / app.sat.scale.max(0.05)))
}

/// Starts a stroke of the brush at (x, z): copying, or with `erase` taking back what the
/// modifier painted.
pub fn begin_stroke(app: &mut App, x: f32, z: f32, erase: bool) {
    let Some((p, _)) = pixel_of(app, x, z) else {
        app.warn(t("The project has no satellite image"));
        return;
    };
    if erase {
        let Some(d) = app.doc.as_ref() else { return };
        let Some(id) = app.sat.active.filter(|id| d.world.sat.modifiers.iter().any(|m| m.id == *id && m.enabled)) else {
            app.warn(t("Choose the modifier to erase from in the outliner"));
            return;
        };
        let before = d.world.sat.modifiers.clone();
        app.sat.stroke = Some(Stroke { id, before, map: SourceMap::shift([0, 0]), erase: true, adjust: None, last: [f32::NAN; 2] });
        continue_stroke(app, x, z);
        return;
    }
    if app.sat.brush != SatBrush::Clone {
        if does_nothing(&app.sat) {
            app.warn(t("Set how the brush changes the colours first"));
            return;
        }
        let Some(before) = app.doc.as_ref().map(|d| d.world.sat.modifiers.clone()) else { return };
        let Some(id) = brush_modifier(app) else { return };
        app.sat.stroke = Some(Stroke { id, before, map: SourceMap::shift([0, 0]), erase: false, adjust: Some(HashMap::new()), last: [f32::NAN; 2] });
        continue_stroke(app, x, z);
        return;
    }
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
    if app.sat.aligned && app.sat.anchor.is_none() {
        app.sat.anchor = Some((p, source));
    }
    let Some(map) = map_for(&app.sat, p) else { return };
    app.sat.stroke = Some(Stroke { id, before, map, erase: false, adjust: None, last: [f32::NAN; 2] });
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
            let r = if stroke.erase {
                d.world.sat_erase_dab(stroke.id, c, &brush)
            } else if let Some(before) = &mut stroke.adjust {
                let (w, h) = d.world.satellite.as_ref().map_or((0, 0), |s| (s.width, s.height));
                d.world.sat_paint(stroke.id, &dz_core::satmod::dab_pixels(c, &brush, w, h), &paint_op(&app.sat, per_metre), before)
            } else {
                d.world.sat_clone_dab_mapped(stroke.id, c, &brush, file.as_deref(), &stroke.map, app.sat.match_colour)
            };
            if let Some(r) = r {
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
        let label = if stroke.erase { "Erase strokes" } else if stroke.adjust.is_some() { if app.sat.brush == SatBrush::Colour { "Colour brush" } else { "Adjust brush" } } else { "Clone brush" };
        d.history.push(t(label), Change::Satellite(stroke.before));
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
    let skip = w.meta.bake_skip.clone();
    let wanted = |path: &str| -> bool {
        // What was chosen by hand is baked as asked; the skip list is for baking the whole map.
        if !selected && dz_core::satmod::bake_skipped(&skip, path) {
            return false;
        }
        // Decals lie flat on the image already; road and water parts only when asked for.
        if path.contains("\\roads\\parts\\") {
            return roads;
        }
        if crate::models::is_water_model(path) {
            return water;
        }
        !crate::models::is_decal_model(path, &app.db.get(path)) && (plants || !crate::viewport::is_plant(path))
    };
    let mut objects = Vec::new();
    let mut add = |o: &dz_core::objects::Obj| {
        let path = w.objects.model_path(o);
        // Objects under the terrain cannot be seen from above.
        let underground = o.flags & dz_core::objects::FLAG_UNDERGROUND != 0;
        let layer_on = selected || w.object_in_satellite(o);
        if w.object_visible(o) && !underground && layer_on && wanted(path) {
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

/// Puts models on the list of those left out of a bake, or takes them off it.
pub fn set_bake_skip(app: &mut App, models: &[String], skip: bool) {
    let Some(d) = &mut app.doc else { return };
    let list = &mut d.world.meta.bake_skip;
    for m in models {
        list.retain(|s| !s.eq_ignore_ascii_case(m));
        if skip { list.push(m.clone()); }
    }
    d.history.revision += 1;
    app.info(tf("{0} models on the list left out of the satellite bake", &[&app.doc.as_ref().map_or(0, |d| d.world.meta.bake_skip.len()).to_string()]));
}

/// The list of models and path parts a bake leaves out, with a field to add one.
fn bake_skip_ui(app: &mut App, ui: &mut egui::Ui) {
    let Some(d) = &app.doc else { return };
    let list = d.world.meta.bake_skip.clone();
    ui.label(t("Left out when baking the whole map"));
    let mut remove = None;
    for (i, s) in list.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.small_button(crate::icons::X).on_hover_text(t("Bake it again")).clicked() { remove = Some(i); }
            ui.add(egui::Label::new(egui::RichText::new(s).small()).truncate());
        });
    }
    // Parts of the game's folders, to leave out a whole kind of object at once.
    ui.horizontal_wrapped(|ui| {
        for (name, part) in [("Wrecks", "\\wrecks\\"), ("Fences and walls", "\\walls\\"), ("Signs", "\\signs\\"), ("Furniture", "\\furniture\\")] {
            let on = list.iter().any(|s| s == part);
            if ui.selectable_label(on, t(name)).on_hover_text(tf("Leaves out every model whose path holds {0}", &[part])).clicked() {
                set_bake_skip(app, &[part.to_string()], !on);
            }
        }
    });
    ui.horizontal(|ui| {
        let id = egui::Id::new("bake_skip_text");
        let mut text: String = ui.data_mut(|d| d.get_temp(id).unwrap_or_default());
        let edit = ui.add(egui::TextEdit::singleline(&mut text).hint_text(t("Part of a model path")).desired_width(ui.available_width() - 40.0));
        let add = ui.small_button(crate::icons::PLUS).on_hover_text(t("Leave out the models whose path holds this text.")).clicked() || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if add && !text.trim().is_empty() {
            set_bake_skip(app, &[text.trim().to_string()], true);
            text.clear();
        }
        ui.data_mut(|d| d.insert_temp(id, text));
    });
    if let Some(i) = remove {
        let s = list[i].clone();
        set_bake_skip(app, &[s], false);
    }
}

/// Rows of the satellite modifiers in the outliner.
pub fn outliner(app: &mut App, ui: &mut egui::Ui, filter: &str, fold: Option<bool>) {
    // In team work: only whose the outliner shows, with who made each.
    let team = crate::team_ui::active(app);
    let rows: Vec<(u32, String, bool, SatKind)> = app.doc.as_ref().map(|d| d.world.sat.modifiers.iter()
        .filter(|m| crate::team_ui::shown(app, d.world.sat_mod_author(m.id)))
        .map(|m| (m.id, crate::team_ui::with_author(team, &m.name, d.world.sat_mod_author(m.id)), m.enabled, m.kind)).collect()).unwrap_or_default();
    let (mut toggle, mut delete) = (None, None);
    crate::panels::outliner_section(fold, "Satellite modifiers", crate::icons::with(crate::icons::IMAGE, &tf("Satellite modifiers ({0})", &[&rows.len().to_string()])), rows.len() == 0).show(ui, |ui| {
        for (id, name, enabled, kind) in rows.iter().filter(|r| r.1.to_lowercase().contains(filter)) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if crate::theme::row_switch(ui, *enabled, crate::icons::EYE, crate::icons::EYE_SLASH, t("Visible")).clicked() {
                    toggle = Some((*id, !*enabled));
                }
                if crate::theme::row_switch(ui, true, crate::icons::TRASH, "", t("Delete the modifier; what it covered comes back")).clicked() {
                    delete = Some(*id);
                }
                let row = crate::theme::list_row(ui, app.sat.active == Some(*id), crate::icons::IMAGE, crate::theme::KIND_IMAGE, name.as_str(), "");
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
                    crate::team_ui::author_menu(app, ui, dz_core::world::SAT_MOD_AUTHOR | *id as u64);
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
                app.sat.anchor = None;
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
    // Where the brush copies from: the source point, or with the cursor on the map the point
    // under it now, with the outline of the brush at the size it has in this view.
    let now = app.cursor_ground.and_then(|c| pixel_of(app, c[0], c[2])).and_then(|(p, _)| app.sat.stroke.as_ref().filter(|s| !s.erase).map(|s| s.map).or_else(|| map_for(&app.sat, p)).map(|m| m.apply(p)));
    if let Some(s) = now.or(app.sat.source) {
        let at = egui::pos2(rect.center().x + (s[0] - app.sat.view_center[0]) / app.sat.view_scale, rect.center().y + (s[1] - app.sat.view_center[1]) / app.sat.view_scale);
        let per_metre = app.doc.as_ref().and_then(|d| d.world.satellite.as_ref().map(|s| s.width as f32 / d.world.meta.size_m())).unwrap_or(1.0);
        let r = (app.sat.radius * per_metre / app.sat.scale.max(0.05) / app.sat.view_scale).max(3.0);
        for (width, color) in [(3.0f32, egui::Color32::BLACK), (1.5f32, egui::Color32::from_rgb(255, 210, 60))] {
            painter.circle_stroke(at, r, egui::Stroke::new(width, color));
            painter.line_segment([at - egui::vec2(6.0, 0.0), at + egui::vec2(6.0, 0.0)], egui::Stroke::new(width, color));
            painter.line_segment([at - egui::vec2(0.0, 6.0), at + egui::vec2(0.0, 6.0)], egui::Stroke::new(width, color));
        }
    }
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0f32, egui::Color32::from_gray(70)), egui::StrokeKind::Inside);
}

/// Size, softness and strength of the brush, shared by both brushes.
fn brush_size_rows(app: &mut App, ui: &mut egui::Ui) {
    row(ui, &t("Radius"), |ui| {
        ui.add(crate::theme::ValueField::new(&mut app.sat.radius, 0.5..=500.0).logarithmic(true).suffix(" m"));
    });
    row(ui, &t("Hardness"), |ui| {
        ui.add(crate::theme::ValueField::new(&mut app.sat.hardness, 0.0..=1.0));
    });
    row(ui, &t("Opacity"), |ui| {
        ui.add(crate::theme::ValueField::new(&mut app.sat.opacity, 0.05..=1.0));
    });
}

/// The modifier of `kind` the strokes go on into, and a way to start a new one.
fn strokes_go_to(app: &mut App, ui: &mut egui::Ui, kind: SatKind) {
    let active = app.doc.as_ref().and_then(|d| app.sat.active.and_then(|id| d.world.sat.modifiers.iter().find(|m| m.id == id && m.kind == kind)).map(|m| m.name.clone()));
    if crate::theme::strokes_target_row(ui, active.as_deref()) { app.sat.active = None; }
}

fn brush_rows(app: &mut App, ui: &mut egui::Ui, kind: SatKind) {
    brush_size_rows(app, ui);
    strokes_go_to(app, ui, kind);
}

/// The colour under the cursor and what the adjust brush makes of it, side by side.
fn adjust_swatch(app: &App, ui: &mut egui::Ui) {
    let Some(c) = app.cursor_ground.and_then(|c| pixel_of(app, c[0], c[2])).and_then(|(p, _)| {
        let img = app.doc.as_ref()?.world.satellite.as_ref()?;
        let (x, y) = (p[0] as usize, p[1] as usize);
        (x < img.width && y < img.height).then(|| {
            let o = (y * img.width + x) * 3;
            [img.data[o], img.data[o + 1], img.data[o + 2]]
        })
    }) else {
        ui.weak(t("Point at the map to see what the brush makes of the colour there."));
        return;
    };
    let after = app.sat.adjust.apply(c);
    ui.horizontal(|ui| {
        for (label, c) in [(t("Now"), c), (t("After"), after)] {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(36.0, 18.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
            ui.label(label);
        }
    });
}

/// A brightness or contrast slider from -1 to 1; in the legacy mode it reads as Photoshop's
/// numbers (`scale` at the ends).
fn bc_slider(v: &mut f32, legacy: bool, scale: f64) -> crate::theme::ValueField<'_> {
    let s = crate::theme::ValueField::new(v, -1.0..=1.0);
    if legacy {
        s.custom_formatter(move |v, _| format!("{:.0}", v * scale)).custom_parser(move |text| text.trim().parse::<f64>().ok().map(|v| v / scale))
    } else {
        s
    }
}

/// The switch to Photoshop's legacy brightness and contrast; true when it was changed.
fn legacy_box(ui: &mut egui::Ui, legacy: &mut bool) -> bool {
    ui.checkbox(legacy, t("Use legacy")).on_hover_text(t("Brightness and contrast as in Photoshop's legacy mode: brightness moves every colour by the same amount, and full contrast leaves each channel either off or full.")).changed()
}

fn adjust_panel(app: &mut App, ui: &mut egui::Ui) {
    crate::panels::about(ui, &t("Paint on the map to make the satellite image there lighter or darker, or change its colours. Going over a place again in one stroke does not add up; a new stroke does."));
    let a = &mut app.sat.adjust;
    let legacy = a.legacy;
    row(ui, &t("Brightness"), |ui| { ui.add(bc_slider(&mut a.brightness, legacy, 150.0)); });
    row(ui, &t("Contrast"), |ui| { ui.add(bc_slider(&mut a.contrast, legacy, 100.0)); });
    legacy_box(ui, &mut a.legacy);
    row(ui, &t("Saturation"), |ui| { ui.add(crate::theme::ValueField::new(&mut a.saturation, -1.0..=1.0)); });
    row(ui, &t("Warmth"), |ui| { ui.add(crate::theme::ValueField::new(&mut a.warmth, -1.0..=1.0)).on_hover_text(t("Towards yellow to the right, towards blue to the left.")); });
    row(ui, &t("Hue"), |ui| { ui.add(crate::theme::ValueField::new(&mut a.hue, -180.0..=180.0).suffix("°")); });
    row(ui, &t("Blur"), |ui| { ui.add(crate::theme::ValueField::new(&mut app.sat.blur, 0.0..=30.0).suffix(" m")).on_hover_text(t("Softens the image under the brush over this distance: seams of cloning, hard edges of baked objects.")); });
    ui.horizontal_wrapped(|ui| {
        // Ready settings for what is done most often.
        for (label, preset) in [
            ("Darken", Adjust { brightness: -0.15, ..Default::default() }),
            ("Lighten", Adjust { brightness: 0.12, ..Default::default() }),
            ("Less colour", Adjust { saturation: -0.35, ..Default::default() }),
            ("Dry grass", Adjust { saturation: -0.25, warmth: 0.4, brightness: 0.05, ..Default::default() }),
        ] {
            if ui.small_button(t(label)).clicked() { app.sat.adjust = preset; }
        }
        if !does_nothing(&app.sat) && ui.small_button(t("Reset")).clicked() {
            app.sat.adjust = Adjust::default();
            app.sat.blur = 0.0;
        }
    });
    adjust_swatch(app, ui);
}

fn colour_panel(app: &mut App, ui: &mut egui::Ui) {
    let k = app.settings.keys.hold(crate::keys::Hold::Pick).name();
    crate::panels::about(ui, &tf("Paints a colour on the map: tracks, bare earth, patches. A click with {0} held takes the colour from the map.", &[k]));
    row(ui, &t("Colour"), |ui| { ui.color_edit_button_srgb(&mut app.sat.colour); });
    row(ui, &t("Laid as"), |ui| {
        let name = |m: BlendMode| t(match m { BlendMode::Normal => "Plain colour", BlendMode::Tint => "Tint", BlendMode::Multiply => "Darken (multiply)", BlendMode::Screen => "Lighten (screen)" });
        egui::ComboBox::from_id_salt("sat_blend").selected_text(name(app.sat.blend)).width(ui.available_width()).show_ui(ui, |ui| {
            for m in [BlendMode::Tint, BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen] {
                ui.selectable_value(&mut app.sat.blend, m, name(m));
            }
        });
    });
    ui.weak(t(match app.sat.blend {
        BlendMode::Tint => "The colour over the light and shade of the image: the pattern stays.",
        BlendMode::Normal => "The colour itself, covering what is under it.",
        BlendMode::Multiply => "The image darkened by the colour.",
        BlendMode::Screen => "The image lightened by the colour.",
    }));
}

/// Fills the selected shapes, or those of a layer, with the current brush: polygons inside,
/// lines as a band along them.
fn fill_section(app: &mut App, ui: &mut egui::Ui) {
    ui.collapsing(t("Fill shapes"), |ui| {
        ui.weak(t("Lays the brush over whole shapes at once: inside polygons, along lines as a band. Fields, clearings, field tracks."));
        shapes_row(app, ui, "sat_fill_layer");
        row(ui, &t("Soft edge m"), |ui| { ui.add(crate::theme::ValueField::new(&mut app.sat.feather, 0.0..=40.0)); });
        row(ui, &t("Line width m"), |ui| { ui.add(crate::theme::ValueField::new(&mut app.sat.line_width, 0.5..=40.0).logarithmic(true)); });
        if ui.button(crate::icons::with(crate::icons::PAINT_BRUSH, &t("Fill"))).on_hover_text(t("Uses the brush's settings and its opacity, into the modifier its strokes go to.")).clicked() {
            fill_shapes(app);
        }
    });
}

/// The polygons and lines of the chosen shapes in pixels of the image, with the pixels a metre.
fn chosen_shapes(app: &App) -> Option<(Vec<Vec<[f32; 2]>>, Vec<(Vec<[f32; 2]>, f32)>, f32, usize, usize)> {
    let d = app.doc.as_ref()?;
    let (w, h) = d.world.satellite.as_ref().map(|s| (s.width, s.height))?;
    chosen_shapes_in(app, w, h)
}

/// The shapes chosen in `shapes_row`: those selected, or all of a shape layer.
fn chosen_map_shapes(app: &App) -> Vec<&dz_core::shapes::MapShape> {
    let Some(d) = app.doc.as_ref() else { return Vec::new() };
    match app.sat.fill_layer {
        Some(i) => d.world.shape_layers.get(i).map(|l| l.shapes.iter().collect()).unwrap_or_default(),
        None => app.shape.selection.iter().filter_map(|(l, s)| d.world.shape_layers.get(*l)?.shapes.get(*s)).collect(),
    }
}

/// As `chosen_shapes`, for an image of `w` × `h` pixels over the map.
fn chosen_shapes_in(app: &App, w: usize, h: usize) -> Option<(Vec<Vec<[f32; 2]>>, Vec<(Vec<[f32; 2]>, f32)>, f32, usize, usize)> {
    let d = app.doc.as_ref()?;
    let size = d.world.meta.size_m();
    let per_metre = w as f32 / size;
    let to_px = |p: &[f32; 2]| [p[0] * per_metre, (1.0 - p[1] / size) * h as f32];
    let shapes = chosen_map_shapes(app);
    use dz_core::shapes::ShapeKind;
    let polygons: Vec<Vec<[f32; 2]>> = shapes.iter().filter(|s| s.kind == ShapeKind::Polygon).map(|s| s.points.iter().map(to_px).collect()).collect();
    let width = app.sat.line_width * per_metre;
    let lines: Vec<(Vec<[f32; 2]>, f32)> = shapes.iter().filter(|s| s.kind == ShapeKind::Polyline).map(|s| (s.points.iter().map(to_px).collect(), width)).collect();
    Some((polygons, lines, per_metre, w, h))
}

/// Picks the shapes a fill or a correction goes over: those selected, or all of a layer.
fn shapes_row(app: &mut App, ui: &mut egui::Ui, salt: &str) {
    let Some(d) = &app.doc else { return };
    let selected = app.shape.selection.len();
    let layers: Vec<String> = d.world.shape_layers.iter().map(|l| l.name.clone()).collect();
    row(ui, &t("Shapes"), |ui| {
        let shown = match app.sat.fill_layer.and_then(|i| layers.get(i)) { Some(n) => n.clone(), None => tf("Selected shapes ({0})", &[&selected.to_string()]) };
        egui::ComboBox::from_id_salt(salt).selected_text(shown).width(ui.available_width()).show_ui(ui, |ui| {
            ui.selectable_value(&mut app.sat.fill_layer, None, tf("Selected shapes ({0})", &[&selected.to_string()]));
            for (i, n) in layers.iter().enumerate() { ui.selectable_value(&mut app.sat.fill_layer, Some(i), n); }
        });
    });
}

/// Colour correction over the whole image or an area: kept as its settings, worked out from
/// what is under it whenever the image is composed, so it can be changed at any time.
fn correction_section(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &crate::icons::with(crate::icons::SLIDERS, &t("Colour correction")), false, |ui| {
        let chosen = app.doc.as_ref().and_then(|d| app.sat.active.and_then(|id| d.world.sat.modifiers.iter().find(|m| m.id == id && m.kind == SatKind::Correct)).map(|m| (m.id, m.name.clone(), m.correction.clone().unwrap_or_default())));
        match chosen {
            Some((id, name, c)) => {
                ui.label(egui::RichText::new(&name).strong());
                ui.weak(t(if c.whole { "Over the whole image." } else { "Over the area of the shapes it was made from." }));
                // The settings shown are those being changed; they reach the image when a slider is let go.
                if app.sat.correcting.map(|e| e.0) != Some(id) { app.sat.correcting = Some((id, c.adjust)); }
                let mut a = app.sat.correcting.map(|e| e.1).unwrap_or_default();
                let mut apply = false;
                let mut slider = |ui: &mut egui::Ui, label: String, s: crate::theme::ValueField| {
                    row(ui, &label, |ui| {
                        let r = ui.add(s);
                        apply |= r.drag_stopped() || (r.changed() && !r.dragged());
                    });
                };
                let legacy = a.legacy;
                slider(ui, t("Brightness"), bc_slider(&mut a.brightness, legacy, 150.0));
                slider(ui, t("Contrast"), bc_slider(&mut a.contrast, legacy, 100.0));
                slider(ui, t("Saturation"), crate::theme::ValueField::new(&mut a.saturation, -1.0..=1.0));
                slider(ui, t("Warmth"), crate::theme::ValueField::new(&mut a.warmth, -1.0..=1.0));
                slider(ui, t("Hue"), crate::theme::ValueField::new(&mut a.hue, -180.0..=180.0).suffix("°"));
                apply |= legacy_box(ui, &mut a.legacy);
                app.sat.correcting = Some((id, a));
                ui.horizontal(|ui| {
                    if ui.small_button(t("Reset")).clicked() {
                        app.sat.correcting = Some((id, Adjust::default()));
                        apply = true;
                    }
                    if ui.small_button(t("New correction")).clicked() {
                        app.sat.active = None;
                        app.sat.correcting = None;
                    }
                });
                ui.weak(t("Changes show when a slider is let go. Opacity and visibility are in the outliner, as for the other modifiers."));
                if apply && a != c.adjust {
                    let a = app.sat.correcting.map(|e| e.1).unwrap_or(a);
                    if let Some(d) = app.doc.as_mut() {
                        d.history.push(t("Colour correction"), Change::Satellite(d.world.sat.modifiers.clone()));
                        if let Some(r) = d.world.sat_set_correction(id, a) { app.satellite_changed(r); }
                    }
                }
            }
            None => {
                ui.weak(t("Changes the colours of the whole satellite image, or of an area, and can be changed or taken away at any time. Choose a correction in the outliner to change it."));
                row(ui, &t("Over"), |ui| {
                    ui.selectable_value(&mut app.sat.correct_whole, true, t("Whole image"));
                    ui.selectable_value(&mut app.sat.correct_whole, false, t("Shapes"));
                });
                if !app.sat.correct_whole {
                    shapes_row(app, ui, "correct_shapes");
                    row(ui, &t("Soft edge m"), |ui| { ui.add(crate::theme::ValueField::new(&mut app.sat.feather, 0.0..=40.0)); });
                } else {
                    ui.weak(t("Over the whole image the original is kept in memory, which takes about as much again as the image."));
                }
                if ui.button(crate::icons::with(crate::icons::PLUS, &t("Add correction"))).clicked() {
                    add_correction(app);
                }
            }
        }
    });
}

/// Sets the chosen correction to `adjust` at once, for scripted checks.
pub fn set_correction(app: &mut App, adjust: Adjust) {
    let Some(id) = app.sat.active else { return };
    if let Some(r) = app.doc.as_mut().and_then(|d| d.world.sat_set_correction(id, adjust)) { app.satellite_changed(r); }
}

/// Adds a correction that changes nothing yet and chooses it, for its sliders to be set.
pub fn add_correction(app: &mut App) {
    let Some(d) = app.doc.as_ref() else { return };
    if d.world.satellite.is_none() {
        app.warn(t("The project has no satellite image"));
        return;
    }
    let n = d.world.sat.modifiers.iter().filter(|m| m.kind == SatKind::Correct).count() + 1;
    let name = tf("Colour correction {0}", &[&n.to_string()]);
    let finish = move |app: &mut App, mask: Option<Vec<(usize, usize, u32)>>| {
        let Some(d) = app.doc.as_mut() else { return };
        let before = d.world.sat.modifiers.clone();
        let (id, touched) = d.world.sat_add_correction(name.clone(), Adjust::default(), mask.as_deref());
        d.history.push(t("Colour correction"), Change::Satellite(before));
        d.dirty.satellite = true;
        app.sat.active = Some(id);
        app.sat.correcting = None;
        if let Some(r) = touched { app.satellite_changed(r); }
    };
    if app.sat.correct_whole {
        finish(app, None);
        return;
    }
    let Some((polygons, lines, per_metre, w, h)) = chosen_shapes(app) else { return };
    if polygons.is_empty() && lines.is_empty() {
        app.warn(t("Select polygons or lines in the Shapes tool, or choose a shape layer"));
        return;
    }
    let feather = app.sat.feather * per_metre;
    app.spawn(t("Colour correction"), move |job| {
        job.stage("Colour correction");
        let mask = dz_core::satmod::shape_pixels(&polygons, &lines, feather, 1.0, w, h);
        Ok(Box::new(move |app: &mut App| finish(app, Some(mask))) as Finish)
    });
}

/// Lays the current brush over the chosen shapes, in the background, as one step to undo.
pub fn fill_shapes(app: &mut App) {
    if does_nothing(&app.sat) {
        app.warn(t("Set how the brush changes the colours first"));
        return;
    }
    let Some((polygons, lines, per_metre, w, h)) = chosen_shapes(app) else {
        app.warn(t("The project has no satellite image"));
        return;
    };
    if polygons.is_empty() && lines.is_empty() {
        app.warn(t("Select polygons or lines in the Shapes tool, or choose a shape layer"));
        return;
    }
    let (feather, strength, op) = (app.sat.feather * per_metre, app.sat.opacity, paint_op(&app.sat, per_metre));
    let label = t("Fill shapes");
    app.spawn(label.clone(), move |job| {
        job.stage("Fill shapes");
        let pixels = dz_core::satmod::shape_pixels(&polygons, &lines, feather, strength, w, h);
        Ok(Box::new(move |app: &mut App| {
            let Some(before) = app.doc.as_ref().map(|d| d.world.sat.modifiers.clone()) else { return };
            let Some(id) = brush_modifier(app) else { return };
            let Some(d) = app.doc.as_mut() else { return };
            let touched = d.world.sat_paint(id, &pixels, &op, &mut HashMap::new());
            d.history.push(label, Change::Satellite(before));
            match touched {
                Some(r) => app.satellite_changed(r),
                None => app.warn(t("The shapes lie outside the satellite image")),
            }
        }) as Finish)
    });
}

/// Over the whole map, or over the shapes chosen as for fills.
fn area_row(app: &mut App, ui: &mut egui::Ui, whole: bool, salt: &str) -> bool {
    let mut whole = whole;
    row(ui, &t("Over"), |ui| {
        ui.selectable_value(&mut whole, true, t("Whole map"));
        ui.selectable_value(&mut whole, false, t("Shapes"));
    });
    if !whole {
        shapes_row(app, ui, salt);
    }
    whole
}

/// The settings of a satellite made from the surface mask, and the button that makes it.
fn generate_section(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &crate::icons::with(crate::icons::MAGIC_WAND, &t("Satellite from the mask")), false, |ui| {
        ui.weak(t("Paints the satellite image in the colours of the surfaces of the mask, with their pattern, soft edges between them and the shading of the hills. It is a modifier, so it can be hidden, blended or deleted."));
        app.sat.gen_whole = area_row(app, ui, app.sat.gen_whole, "gen_shapes");
        if !app.sat.gen_whole {
            row(ui, &t("Soft edge m"), |ui| { ui.add(crate::theme::ValueField::new(&mut app.sat.feather, 0.0..=40.0)); });
        }
        let g = &mut app.sat.gen;
        row(ui, &t("Texture size m"), |ui| { ui.add(crate::theme::ValueField::new(&mut g.texture_m, 1.0..=64.0).logarithmic(true)).on_hover_text(t("Metres one repeat of a surface texture covers.")); });
        row(ui, &t("Texture pattern"), |ui| { ui.add(crate::theme::ValueField::new(&mut g.detail, 0.0..=1.0)).on_hover_text(t("How much of the light and dark pattern of the surface texture shows.")); });
        row(ui, &t("Edge between surfaces m"), |ui| { ui.add(crate::theme::ValueField::new(&mut g.blend_m, 0.0..=20.0)).on_hover_text(t("Width of the ragged, soft edge where one surface meets another.")); });
        row(ui, &t("Patches"), |ui| { ui.add(crate::theme::ValueField::new(&mut g.variation, 0.0..=0.6)).on_hover_text(t("Larger areas a little lighter or darker, as real ground is.")); });
        row(ui, &t("Patch size m"), |ui| { ui.add(crate::theme::ValueField::new(&mut g.variation_m, 10.0..=1000.0).logarithmic(true)); });
        row(ui, &t("Hill shading"), |ui| { ui.add(crate::theme::ValueField::new(&mut g.hillshade, 0.0..=1.0)); });
        row(ui, &t("Sun from"), |ui| { ui.add(crate::theme::ValueField::new(&mut g.sun_deg, 0.0..=360.0).suffix("°")).on_hover_text(t("Where the sun stands that shades the hills: 0 north, 90 east.")); });
        ui.horizontal(|ui| {
            ui.checkbox(&mut g.water, t("Water under the sea level"));
            if g.water { ui.color_edit_button_srgb(&mut g.water_colour); }
        });
        row(ui, &t("Seed"), |ui| { ui.add(egui::DragValue::new(&mut g.seed)); });
        surface_colours(app, ui);
        let has = app.doc.as_ref().is_some_and(|d| d.world.satellite.is_some());
        if !has {
            row(ui, &t("Metres a pixel"), |ui| { ui.add(crate::theme::ValueField::new(&mut app.sat.gen_mpp, 0.25..=4.0).logarithmic(true)).on_hover_text(t("The project has no satellite image yet: a new one is made at this resolution.")); });
        }
        if ui.button(crate::icons::with(crate::icons::MAGIC_WAND, &t("Make the satellite"))).clicked() {
            generate_from_mask(app);
        }
    });
}

/// The colour of each surface from above, which the satellite is painted in.
fn surface_colours(app: &mut App, ui: &mut egui::Ui) {
    let Some(d) = app.doc.as_ref() else { return };
    let names: Vec<String> = d.world.layers.iter().map(|l| l.name.clone()).collect();
    sync_colours(app);
    ui.collapsing(t("Colours of the surfaces"), |ui| {
        ui.weak(t("The game takes the colour of the ground from the satellite image and only the pattern from the surface texture, so each surface is given its colour here."));
        for (i, n) in names.iter().enumerate() {
            row(ui, n, |ui| { ui.color_edit_button_srgb(&mut app.sat.gen_colours[i]); });
        }
        ui.horizontal_wrapped(|ui| {
            if ui.small_button(t("Guess from the names")).clicked() {
                app.sat.gen_colours = names.iter().map(|n| dz_core::satgen::ground_colour(n)).collect();
            }
            let has = app.doc.as_ref().is_some_and(|d| d.world.satellite.is_some());
            if has && ui.small_button(t("Take from the satellite")).on_hover_text(t("Each surface takes the average colour the satellite image has where the mask holds it.")).clicked() {
                if let Some(d) = app.doc.as_ref() {
                    for (i, c) in dz_core::satgen::colours_from_satellite(&d.world, 512).into_iter().enumerate() {
                        if let (Some(c), Some(to)) = (c, app.sat.gen_colours.get_mut(i)) { *to = c; }
                    }
                }
            }
        });
    });
}

/// One colour for each surface layer: those known kept, new layers guessed from their names.
fn sync_colours(app: &mut App) {
    let Some(d) = app.doc.as_ref() else { return };
    let layers = &d.world.layers;
    if app.sat.gen_colours.len() > layers.len() {
        app.sat.gen_colours.truncate(layers.len());
    }
    for l in &layers[app.sat.gen_colours.len()..] {
        app.sat.gen_colours.push(dz_core::satgen::ground_colour(&l.name));
    }
}

/// Makes a satellite modifier from the mask, in the background, as one step to undo. A project
/// without a satellite image gets one first.
pub fn generate_from_mask(app: &mut App) {
    sync_colours(app);
    let Some(d) = app.doc.as_ref() else { return };
    let w = &d.world;
    let (width, height) = match &w.satellite {
        Some(s) => (s.width, s.height),
        None => {
            let side = ((w.meta.size_m() / app.sat.gen_mpp.max(0.05)).round() as usize).clamp(64, 32768);
            (side, side)
        }
    };
    let cover = if app.sat.gen_whole {
        None
    } else {
        let Some((polygons, lines, per_metre, _, _)) = chosen_shapes_in(app, width, height) else { return };
        if polygons.is_empty() && lines.is_empty() {
            app.warn(t("Select polygons or lines in the Shapes tool, or choose a shape layer"));
            return;
        }
        Some((polygons, lines, app.sat.feather * per_metre))
    };
    let lite = w.lite();
    let layers: Vec<(String, [u8; 3])> = lite.layers.iter().zip(&app.sat.gen_colours).map(|(l, c)| (l.texture.clone(), *c)).collect();
    let (root, settings) = (app.settings.tools.work_drive.clone(), app.sat.gen.clone());
    let mpp = w.meta.size_m() / width as f32;
    let label = t("Satellite from the mask");
    app.spawn(label.clone(), move |job| {
        job.stage("Reading the surface textures");
        let side = settings.texture_side(mpp);
        let looks: Vec<dz_core::satgen::LayerLook> = layers.iter().map(|(tex, colour)| {
            let (rgba, _, _) = dz_core::satgen::layer_texture(&root, tex, *colour, 512);
            dz_core::satgen::LayerLook::new(*colour, &rgba, 512, side)
        }).collect();
        job.stage("Painting the satellite");
        let cover = cover.map(|(polygons, lines, feather)| dz_core::satgen::cover_tiles(&dz_core::satmod::shape_pixels(&polygons, &lines, feather, 1.0, width, height)));
        let tiles = dz_core::satgen::from_mask(&lite, &looks, &settings, width, height, cover.as_ref(), job);
        job.check()?;
        Ok(Box::new(move |app: &mut App| {
            let Some(d) = app.doc.as_mut() else { return };
            if tiles.is_empty() {
                app.warn(t("The shapes lie outside the satellite image"));
                return;
            }
            let created = d.world.satellite.is_none();
            if created {
                d.world.satellite = Some(RgbImage::new(width, height, [110, 110, 100]));
                d.dirty.satellite = true;
                app.renderer.lock().unwrap().set_satellite(d.world.satellite.as_ref());
            }
            let before = d.world.sat.modifiers.clone();
            let n = before.iter().filter(|m| m.kind == SatKind::Generated).count() + 1;
            let id = d.world.sat.add(SatKind::Generated, tf("Satellite from the mask {0}", &[&n.to_string()]));
            let touched = d.world.sat_add_tiles(id, tiles);
            d.history.push(label, Change::Satellite(before));
            app.sat.active = Some(id);
            if let Some(r) = touched {
                app.satellite_changed(r);
            }
            if created {
                app.info(tf("A satellite image of {0} × {1} pixels was made for the project", &[&width.to_string(), &height.to_string()]));
            }
        }) as Finish)
    });
}

/// Side of the copy of the satellite the preview of its classes is made from.
const CLASS_PREVIEW: usize = 320;

/// The settings of surfaces read from the satellite, with a preview of its colour classes.
fn classify_section(app: &mut App, ui: &mut egui::Ui) {
    use dz_core::satgen::{ClassifyMode, PickedColour, SatClassify, MAX_PICKED};
    section(ui, &crate::icons::with(crate::icons::PALETTE, &t("Surfaces from the satellite")), false, |ui| {
        let Some(sat) = app.doc.as_ref().and_then(|d| d.world.satellite.as_ref()) else {
            ui.colored_label(crate::theme::WARN, t("The project has no satellite image"));
            return;
        };
        // A small copy of the image, made again when the image changes.
        if app.sat.class_small.as_ref().map(|s| s.0) != Some(app.sat.revision) {
            app.sat.class_small = Some((app.sat.revision, Arc::new(sat.resized(CLASS_PREVIEW, CLASS_PREVIEW))));
            app.sat.class_preview = None;
        }
        let small = app.sat.class_small.as_ref().map(|s| s.1.clone()).expect("made above");
        let layers = app.doc.as_ref().map(|d| d.world.layers.clone()).unwrap_or_default();
        let mut mode = app.sat.classify.mode;
        let changed = crate::theme::segmented(ui, &mut mode, &[
            (ClassifyMode::Colours, t("Colour groups"), t("Pushes the image to eight strong colours. Good for pictures made of flat colours.")),
            (ClassifyMode::Natural, t("Natural colours"), t("Tells forest, grass, fields, soil, rock and roads apart in a real satellite photo.")),
            (ClassifyMode::Picked, t("Picked colours"), t("Your own list of colours: each pixel takes the nearest one.")),
        ]);
        if changed {
            app.sat.classify = match mode {
                ClassifyMode::Colours => SatClassify::default(),
                ClassifyMode::Natural => SatClassify::natural(&layers),
                ClassifyMode::Picked => SatClassify::picked(&small, 4, &layers),
            };
        }
        ui.weak(match mode {
            ClassifyMode::Colours => t("Brightness and contrast push the satellite image to a few strong colours, as Photoshop's legacy brightness and contrast does. Each colour that comes out can become a surface of the mask."),
            ClassifyMode::Natural => t("Each pixel gets the kind of ground its colour looks like. Lighter makes less forest and more grass and rock. Each kind can become a surface of the mask."),
            ClassifyMode::Picked => t("Each pixel takes the nearest of the colours below, so four colours make a mask of four surfaces. Click the picture to add the colour of that place."),
        });
        let c = &mut app.sat.classify;
        if c.mode != ClassifyMode::Picked {
            let legacy = c.adjust.legacy;
            row(ui, &t("Brightness"), |ui| { ui.add(bc_slider(&mut c.adjust.brightness, legacy, 150.0)); });
            row(ui, &t("Contrast"), |ui| { ui.add(bc_slider(&mut c.adjust.contrast, legacy, 100.0)); });
            if c.mode == ClassifyMode::Colours {
                legacy_box(ui, &mut c.adjust.legacy);
            }
        }
        row(ui, &t("Smoothing"), |ui| { ui.add(crate::theme::ValueField::new(&mut c.smooth, 0..=6).suffix(" px")).on_hover_text(t("Takes away specks: each pixel of the mask takes the colour most pixels round it have.")); });
        if c.mode == ClassifyMode::Picked {
            ui.horizontal(|ui| {
                let n = &mut app.sat.class_find;
                if ui.button(crate::icons::with(crate::icons::MAGIC_WAND, &t("Find colours"))).on_hover_text(t("Finds this many colours that stand for the picture best, each with the surface of the nearest colour.")).clicked() {
                    *c = SatClassify { smooth: c.smooth, ..SatClassify::picked(&small, *n, &layers) };
                }
                ui.add(crate::theme::ValueField::new(n, 2..=MAX_PICKED));
                ui.checkbox(&mut app.sat.class_picture, t("Show the picture")).on_hover_text(t("Shows the satellite instead of the colours, to pick from."));
            });
        }
        // The preview follows the settings.
        let picture = c.mode == ClassifyMode::Picked && app.sat.class_picture;
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            serde_json::to_string(&(&c.adjust, c.smooth, c.mode, &c.picked, picture)).unwrap_or_default().hash(&mut h);
            app.sat.revision.hash(&mut h);
            h.finish()
        };
        if app.sat.class_preview.as_ref().map(|p| p.1) != Some(key) {
            let cls = c.classes(&small, CLASS_PREVIEW, CLASS_PREVIEW);
            let cls = dz_core::satgen::smooth_classes(&cls, CLASS_PREVIEW, CLASS_PREVIEW, c.smooth.min(2));
            let mut share = [0.0f32; MAX_PICKED];
            let mut rgba = Vec::with_capacity(cls.len() * 4);
            for (i, k) in cls.iter().enumerate() {
                share[(*k as usize).min(MAX_PICKED - 1)] += 1.0 / cls.len() as f32;
                let col = if picture { [small.data[i * 3], small.data[i * 3 + 1], small.data[i * 3 + 2]] } else { c.colour(*k as usize) };
                rgba.extend([col[0], col[1], col[2], 255]);
            }
            app.sat.class_share = share;
            let image = egui::ColorImage::from_rgba_unmultiplied([CLASS_PREVIEW, CLASS_PREVIEW], &rgba);
            app.sat.class_preview = Some((ui.ctx().load_texture("sat_classes", image, egui::TextureOptions::NEAREST), key));
        }
        if let Some((tex, _)) = &app.sat.class_preview {
            let side = ui.available_width().clamp(160.0, 320.0);
            let picking = c.mode == ClassifyMode::Picked && c.picked.len() < MAX_PICKED;
            let r = ui.add(egui::Image::new((tex.id(), egui::vec2(side, side))).sense(if picking { egui::Sense::click() } else { egui::Sense::hover() }));
            if picking {
                let r = r.on_hover_cursor(egui::CursorIcon::Crosshair).on_hover_text(t("Click to add the colour of this place"));
                if let Some(p) = r.interact_pointer_pos().filter(|_| r.clicked()) {
                    // The average of the pixels round the click, so one odd pixel does not count.
                    let at = |v: f32, lo: f32| (((v - lo) / side * CLASS_PREVIEW as f32) as i64).clamp(0, CLASS_PREVIEW as i64 - 1);
                    let (x, y) = (at(p.x, r.rect.left()), at(p.y, r.rect.top()));
                    let mut sum = [0u32; 4];
                    for dy in -1..=1i64 {
                        for dx in -1..=1i64 {
                            let (xx, yy) = ((x + dx).clamp(0, CLASS_PREVIEW as i64 - 1) as usize, (y + dy).clamp(0, CLASS_PREVIEW as i64 - 1) as usize);
                            let q = (yy * CLASS_PREVIEW + xx) * 3;
                            for k in 0..3 {
                                sum[k] += small.data[q + k] as u32;
                            }
                            sum[3] += 1;
                        }
                    }
                    let colour = [(sum[0] / sum[3]) as u8, (sum[1] / sum[3]) as u8, (sum[2] / sum[3]) as u8];
                    c.picked.push(PickedColour { colour, surface: dz_core::satgen::guess_surface(&small, colour, &layers) });
                }
            }
        }
        let names: Vec<String> = layers.iter().map(|l| l.name.clone()).collect();
        let leave = t("Leave as it is");
        let mut remove = None;
        for k in 0..app.sat.classify.count() {
            let c = &mut app.sat.classify;
            let picked = c.mode == ClassifyMode::Picked;
            if !picked && app.sat.class_share[k] < 0.001 && c.surface(k).is_none() {
                continue;
            }
            ui.horizontal(|ui| {
                if picked {
                    ui.color_edit_button_srgb(&mut c.picked[k].colour);
                    ui.label(format!("{:.0}%", app.sat.class_share[k] * 100.0));
                } else {
                    let col = c.colour(k);
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(col[0], col[1], col[2]));
                    ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0_f32, egui::Color32::GRAY), egui::StrokeKind::Inside);
                    ui.label(format!("{} {:.0}%", t(c.names()[k]), app.sat.class_share[k] * 100.0));
                }
                let width = ui.available_width() - if picked { 26.0 } else { 0.0 };
                let target = if picked { &mut c.picked[k].surface } else { &mut c.surfaces[k] };
                let shown = target.and_then(|i| names.get(i).cloned()).unwrap_or_else(|| leave.clone());
                egui::ComboBox::from_id_salt(("sat_class", k)).selected_text(shown).width(width).show_ui(ui, |ui| {
                    ui.selectable_value(target, None, &leave);
                    for (i, n) in names.iter().enumerate() { ui.selectable_value(target, Some(i), n); }
                });
                if picked && ui.small_button(crate::icons::X).on_hover_text(t("Remove the colour")).clicked() {
                    remove = Some(k);
                }
            });
        }
        if let Some(k) = remove {
            app.sat.classify.picked.remove(k);
        }
        if app.sat.classify.mode == ClassifyMode::Picked && app.sat.classify.picked.len() < MAX_PICKED && ui.button(crate::icons::with(crate::icons::PLUS, &t("Add colour"))).clicked() {
            app.sat.classify.picked.push(PickedColour { colour: [128, 128, 128], surface: None });
        }
        app.sat.classify_whole = area_row(app, ui, app.sat.classify_whole, "classify_shapes");
        if ui.button(crate::icons::with(crate::icons::PALETTE, &t("Paint the surfaces"))).clicked() {
            classify_to_mask(app);
        }
    });
}

/// Paints the surfaces the colours of the satellite stand for into a mask modifier.
pub fn classify_to_mask(app: &mut App) {
    let Some(d) = app.doc.as_ref() else { return };
    let w = &d.world;
    let Some(sat) = &w.satellite else {
        app.warn(t("The project has no satellite image"));
        return;
    };
    if (0..app.sat.classify.count()).all(|k| app.sat.classify.surface(k).is_none()) {
        app.warn(t("Choose a surface for at least one colour"));
        return;
    }
    let area = if app.sat.classify_whole {
        None
    } else {
        let shapes: Vec<dz_core::shapes::MapShape> = chosen_map_shapes(app).into_iter().filter(|s| s.kind == dz_core::shapes::ShapeKind::Polygon).cloned().collect();
        if shapes.is_empty() {
            app.warn(t("Select polygons in the Shapes tool, or choose a shape layer"));
            return;
        }
        Some(dz_core::shapes::PolygonIndex::new(&dz_core::shapes::ShapeLayer { name: String::new(), visible: true, shapes }))
    };
    let (mw, mh) = (w.mask.width, w.mask.height);
    // The image at the size of the mask, each pixel the average of those under it.
    let small = if sat.width == mw && sat.height == mh { sat.clone() } else { sat.resized(mw, mh) };
    let lite = w.lite();
    let settings = app.sat.classify.clone();
    let label = t("Surfaces from the satellite");
    app.spawn(label.clone(), move |job| {
        job.stage("Sorting the colours of the satellite");
        let cls = settings.classes(&small, mw, mh);
        let cls = dz_core::satgen::smooth_classes(&cls, mw, mh, settings.smooth);
        let test = area.as_ref().map(|a| move |x: f32, z: f32| a.contains(x, z));
        let data = dz_core::satgen::mask_from_classes(&lite, &cls, &settings, test.as_ref().map(|f| f as &(dyn Fn(f32, f32) -> bool + Sync)));
        Ok(Box::new(move |app: &mut App| {
            let Some(d) = app.doc.as_ref() else { return };
            if d.world.mask.data.len() != data.len() { return; }
            let tiles = dz_core::maskmod::diff_tiles(&d.world.mask.data, &data, mw, mh, |_, _| false);
            if tiles.is_empty() {
                app.warn(t("The surfaces are the same as the mask already has"));
                return;
            }
            let n = d.world.mask_mods.modifiers.iter().filter(|m| m.kind == dz_core::maskmod::MaskKind::Satellite).count() + 1;
            crate::maskmods::store(app, None, dz_core::maskmod::MaskKind::Satellite, tf("Surfaces from the satellite {0}", &[&n.to_string()]), tiles);
            app.info(label);
        }) as Finish)
    });
}

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &crate::icons::with(crate::icons::PAINT_BRUSH, &t("Brush")), true, |ui| {
        row(ui, &t("Paints"), |ui| {
            let before = app.sat.brush;
            crate::theme::segmented(ui, &mut app.sat.brush, &[
                (SatBrush::Clone, t("Clone"), t("Copies another part of the image, or of a picture, onto the map.")),
                (SatBrush::Adjust, t("Adjust"), t("Makes the image lighter or darker, or changes its colours, where you paint.")),
                (SatBrush::Colour, t("Colour"), t("Paints a colour, plain or as a tint that keeps the pattern of the image.")),
            ]);
            // Strokes go on into a modifier of the brush's own kind.
            if before != app.sat.brush { app.sat.active = None; }
        });
        if app.sat.brush != SatBrush::Clone {
            if app.sat.brush == SatBrush::Adjust { adjust_panel(app, ui); } else { colour_panel(app, ui); }
            brush_rows(app, ui, app.sat.brush.kind());
            fill_section(app, ui);
            return;
        }
        let k = &app.settings.keys;
        crate::panels::about(ui, &tf("Click in the preview, or on the map with {0} held, to choose where to copy from, then paint on the map. Wheel zooms the preview, dragging moves it.", &[k.hold(crate::keys::Hold::Pick).name()]));
        row(ui, &t("Copy from"), |ui| {
            let before = app.sat.from_file;
            crate::theme::segmented(ui, &mut app.sat.from_file, &[(false, t("Satellite"), String::new()), (true, t("Picture"), String::new())]);
            if before != app.sat.from_file {
                // Another picture: its own point and view.
                app.sat.source = None;
                app.sat.anchor = None;
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
                                app.sat.anchor = None;
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
        brush_size_rows(app, ui);
        crate::theme::check_row(ui, &mut app.sat.aligned, t("Aligned")).on_hover_text(t("The source keeps its distance from the brush from one stroke to the next. Off: every stroke starts copying from the source point again."));
        crate::theme::check_row(ui, &mut app.sat.match_colour, t("Match the colour")).on_hover_text(t("The copy takes on the average colour of where it is painted, so a patch from elsewhere blends in. The pattern of the source stays."));
        row(ui, &t("Turn"), |ui| {
            ui.add(crate::theme::ValueField::new(&mut app.sat.turn, -180.0..=180.0).suffix("°")).on_hover_text(t("Turns what is copied about the source point, to lay fields or rows another way."));
        });
        row(ui, &t("Size of the copy"), |ui| {
            ui.add(crate::theme::ValueField::new(&mut app.sat.scale, 0.25..=4.0).logarithmic(true).custom_formatter(|v, _| format!("{:.0}%", v * 100.0))).on_hover_text(t("How large the copy comes out against the source: to fit a picture of another resolution, or to make fields larger or smaller."));
        });
        if (app.sat.turn != 0.0 || app.sat.scale != 1.0) && ui.small_button(t("Straight and at full size")).clicked() {
            app.sat.turn = 0.0;
            app.sat.scale = 1.0;
        }
        strokes_go_to(app, ui, SatKind::Clone);
    });
    correction_section(app, ui);
    generate_section(app, ui);
    classify_section(app, ui);
    section(ui, &crate::icons::with(crate::icons::STAMP, &t("Bake objects")), false, |ui| {
        ui.weak(t("Draws the objects as seen from above into the satellite image, as a modifier that can be hidden or deleted."));
        let selected = app.selection.len();
        crate::theme::check_row(ui, &mut app.sat.bake_selection, tf("Only the selected objects ({0})", &[&selected.to_string()]));
        crate::theme::check_row(ui, &mut app.sat.bake_plants, t("Trees and bushes too"));
        crate::theme::check_row(ui, &mut app.sat.bake_roads, t("Road parts too")).on_hover_text(t("Paints the asphalt and verges of the road models into the image, as they look from above."));
        crate::theme::check_row(ui, &mut app.sat.bake_water, t("Water parts too")).on_hover_text(t("Paints river and pond parts into the image, their water in the colour of water."));
        bake_skip_ui(app, ui);
        crate::theme::check_row(ui, &mut app.sat.bake_above_ground, t("Only what is above the ground")).on_hover_text(t("Leaves out the parts of an object sunk into the terrain, so a pier half in the ground shows only its part above it."));
        // The slider sets the blending of the next bake, and of the baked modifier chosen in the outliner.
        let chosen = app.doc.as_ref().and_then(|d| app.sat.active.and_then(|id| d.world.sat.modifiers.iter().find(|m| m.id == id && m.kind == SatKind::Bake)).map(|m| (m.id, m.name.clone(), 1.0 - m.opacity as f32 / 255.0)));
        if let Some((_, _, blend)) = &chosen {
            if !ui.ctx().is_using_pointer() {
                app.sat.bake_blend = *blend;
            }
        }
        let mut changed = None;
        row(ui, &t("Blending"), |ui| {
            let resp = ui.add(crate::theme::ValueField::new(&mut app.sat.bake_blend, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0))).on_hover_text(t("How far the baked objects fade into the satellite image under them."));
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
    crate::panels::shortcuts(ui, |ui| {
        crate::panels::brush_hints(app, ui, Some("Erase from the chosen modifier"), false, true, true);
        let pick = app.settings.keys.hold(crate::keys::Hold::Pick).name().to_string();
        match app.sat.brush {
            SatBrush::Clone => crate::panels::key_hints(ui, &[(pick, t("Copy from the point clicked"))]),
            SatBrush::Colour => crate::panels::key_hints(ui, &[(pick, t("Take the colour from the map"))]),
            SatBrush::Adjust => {}
        }
    });
}
