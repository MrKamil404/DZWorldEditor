//! The export window: the parts of the map as files for other tools, each with its options.
use crate::app::{App, Dialog, Finish};
use crate::i18n::{t, tf};
use crate::icons;
use crate::theme::row;
use eframe::egui;
use dz_core::export_files::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What the export window offers, remembered between uses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    pub folder: String,
    /// Only the selected objects go into the object list.
    pub selection_only: bool,
    pub heightmap_on: bool,
    pub heightmap: HeightmapOptions,
    pub satellite_on: bool,
    pub satellite: SatelliteOptions,
    pub mask_on: bool,
    pub mask: MaskOptions,
    pub layers_on: bool,
    pub layers: LayersOptions,
    pub shapes_on: bool,
    pub shapes: ShapesOptions,
    pub objects_on: bool,
    pub objects: ObjectsOptions,
    pub mission_on: bool,
    pub mission: MissionOptions,
}
impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            folder: String::new(), selection_only: false,
            heightmap_on: true, heightmap: Default::default(), satellite_on: true, satellite: Default::default(), mask_on: true, mask: Default::default(),
            layers_on: true, layers: Default::default(), shapes_on: false, shapes: Default::default(), objects_on: true, objects: Default::default(), mission_on: true, mission: Default::default(),
        }
    }
}

/// One part of the export, when the window shows only it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Heightmap,
    Satellite,
    Mask,
    Layers,
    Shapes,
    Objects,
    Mission,
}
impl Part {
    fn title(self) -> String {
        match self {
            Part::Heightmap => t("Heightmap (ASC)"),
            Part::Satellite => t("Satellite image"),
            Part::Mask => t("Surface mask"),
            Part::Layers => "layers.cfg".into(),
            Part::Shapes => t("Shapes (SHP)"),
            Part::Objects => t("Objects (Terrain Builder TXT)"),
            Part::Mission => t("Server mission"),
        }
    }
}

impl ExportSettings {
    /// What is exported: the ticked parts, or only `part`.
    fn parts(&self, selection: Vec<u32>, only: Option<Part>) -> FileExport {
        let on = |p: Part, ticked: bool| only.map_or(ticked, |o| o == p);
        FileExport {
            heightmap: on(Part::Heightmap, self.heightmap_on).then(|| self.heightmap.clone()),
            satellite: on(Part::Satellite, self.satellite_on).then(|| self.satellite.clone()),
            mask: on(Part::Mask, self.mask_on).then(|| self.mask.clone()),
            layers_cfg: on(Part::Layers, self.layers_on).then(|| self.layers.clone()),
            shapes: on(Part::Shapes, self.shapes_on).then(|| self.shapes.clone()),
            objects: on(Part::Objects, self.objects_on).then(|| self.objects.clone()),
            mission: on(Part::Mission, self.mission_on).then(|| self.mission.clone()),
            selection: if self.selection_only { selection } else { Vec::new() },
        }
    }
    fn any(&self) -> bool {
        self.heightmap_on || self.satellite_on || self.mask_on || self.layers_on || self.shapes_on || self.objects_on || self.mission_on
    }
}

/// A part of the export: a checkbox and, while it is on, its options. When the window shows
/// only this part, just its options.
fn part(ui: &mut egui::Ui, only: Option<Part>, me: Part, on: &mut bool, enabled: bool, hint: &str, body: impl FnOnce(&mut egui::Ui)) {
    match only {
        Some(p) if p != me => return,
        Some(_) => {
            ui.weak(hint);
            ui.add_space(4.0);
            ui.add_enabled_ui(enabled, body);
            return;
        }
        None => {}
    }
    let title = me.title();
    ui.add_enabled_ui(enabled, |ui| {
        // What the project does not have shows as off, whatever was chosen before.
        let mut off = false;
        ui.checkbox(if enabled { &mut *on } else { &mut off }, egui::RichText::new(title).strong()).on_hover_text(hint).on_disabled_hover_text(hint);
        if *on && enabled {
            egui::Frame::new().inner_margin(egui::Margin { left: 22, right: 4, top: 2, bottom: 6 }).show(ui, body);
        }
    });
    if me != Part::Mission {
        ui.separator();
    }
}

fn image_ui(ui: &mut egui::Ui, o: &mut ImageOptions, own: usize, id: &str) {
    row(ui, &t("Format"), |ui| {
        for f in ImageFormat::ALL { ui.selectable_value(&mut o.format, f, f.extension().to_uppercase()); }
    });
    row(ui, &t("Size"), |ui| {
        let text = |s: usize| if s == 0 { format!("{} ({own} px)", t("As it is")) } else { format!("{s} px") };
        egui::ComboBox::from_id_salt(("size", id)).selected_text(text(o.size)).show_ui(ui, |ui| {
            for s in [0, 1024, 2048, 4096, 8192, 10240, 12288, 16384, 20480] { ui.selectable_value(&mut o.size, s, text(s)); }
        });
    });
    row(ui, &t("Tiles"), |ui| {
        egui::ComboBox::from_id_salt(("tiles", id)).selected_text(if o.tiles <= 1 { t("One image") } else { format!("{0} × {0}", o.tiles) }).show_ui(ui, |ui| {
            ui.selectable_value(&mut o.tiles, 1, t("One image"));
            for n in [2, 4, 8, 16] { ui.selectable_value(&mut o.tiles, n, format!("{n} × {n}")); }
        });
    });
}

pub fn window(app: &mut App, ctx: &egui::Context, only: Option<Part>, open: &mut bool) -> bool {
    let mut close = false;
    let Some(doc) = &app.doc else { return true };
    let w = &doc.world;
    let mut s = app.settings.export.clone();
    // The first time, a folder beside the project.
    if s.folder.trim().is_empty() {
        if let Some(dir) = &doc.dir { s.folder = dir.join("export").display().to_string(); }
    }
    let sat_px = w.satellite.as_ref().map(|i| i.width).unwrap_or(0);
    let has_sat = sat_px > 0;
    let shape_names: Vec<String> = w.shape_layers.iter().filter(|l| !l.shapes.is_empty()).map(|l| l.name.clone()).collect();
    let selected = app.selection.len();
    let title = only.map_or_else(|| t("Export files"), |p| format!("{} – {}", t("Export"), p.title()));
    egui::Window::new(icons::with(icons::EXPORT, &title)).id(egui::Id::new("export_window")).collapsible(false).resizable(false).min_width(460.0).max_width(460.0).open(open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        egui::ScrollArea::vertical().max_height(ctx.screen_rect().height() * 0.7).show(ui, |ui| {
            part(ui, only, Part::Heightmap, &mut s.heightmap_on, true, &t("The terrain as an ESRI ASCII grid, as Terrain Builder and GIS tools read it."), |ui| {
                let h = &mut s.heightmap;
                ui.checkbox(&mut h.with_modifiers, t("With terrain modifiers")).on_hover_text(t("Roads, pads and ponds cut into the terrain; without them the terrain as sculpted."));
                row(ui, &t("Cell"), |ui| {
                    let own = w.height.cell;
                    let text = |c: f32| if c <= 0.0 { format!("{} ({own} m)", t("As it is")) } else { format!("{c} m") };
                    egui::ComboBox::from_id_salt("asc_cell").selected_text(text(h.cell)).show_ui(ui, |ui| {
                        for c in [0.0, 1.0, 2.0, 4.0, 5.0, 8.0, 10.0, 16.0] { ui.selectable_value(&mut h.cell, c, text(c)); }
                    });
                });
                let side = w.height.cols as f32 * w.height.cell;
                let n = if h.cell > 0.0 { (side / h.cell).round() as usize } else { w.height.cols };
                ui.weak(tf("{0} × {0} samples", &[&n.to_string()]));
                row(ui, &t("Decimals"), |ui| { ui.add(egui::DragValue::new(&mut h.decimals).range(0..=6)); });
                row(ui, &t("Raise by"), |ui| { ui.add(egui::DragValue::new(&mut h.raise).speed(0.1).suffix(" m")); });
                ui.checkbox(&mut h.offset, t("With easting and northing")).on_hover_text(t("The corner of the grid in map coordinates, as Terrain Builder expects; off places it at 0, 0."));
            });
            part(ui, only, Part::Satellite, &mut s.satellite_on, has_sat, &if has_sat { t("The satellite image of the map.") } else { t("The project has no satellite image.") }, |ui| {
                ui.checkbox(&mut s.satellite.with_modifiers, t("With satellite modifiers")).on_hover_text(t("Baked objects and road parts, clone strokes; without them the image as imported."));
                image_ui(ui, &mut s.satellite.image, sat_px, "sat");
            });
            part(ui, only, Part::Mask, &mut s.mask_on, true, &t("Each surface in its colour from layers.cfg."), |ui| {
                ui.checkbox(&mut s.mask.with_modifiers, t("With mask modifiers")).on_hover_text(t("Surfaces under objects, roads and forests; without them the mask as painted."));
                image_ui(ui, &mut s.mask.image, w.mask.width, "mask");
                ui.weak(t("Scaled without blending, so it only holds the colours of the surfaces."));
            });
            part(ui, only, Part::Layers, &mut s.layers_on, true, &t("The surfaces with their colours and materials, for Terrain Builder."), |ui| {
                row(ui, &t("Legend"), |ui| { ui.text_edit_singleline(&mut s.layers.legend); });
                ui.checkbox(&mut s.layers.only_used, t("Only surfaces the mask uses"));
            });
            part(ui, only, Part::Shapes, &mut s.shapes_on, !shape_names.is_empty(), &if shape_names.is_empty() { t("The project has no shapes.") } else { t("Each shape layer as a shapefile.") }, |ui| {
                ui.label(t("Layers"));
                ui.weak(t("None ticked writes the visible ones."));
                for n in &shape_names {
                    let mut on = s.shapes.layers.contains(n);
                    if ui.checkbox(&mut on, n).changed() {
                        if on { s.shapes.layers.push(n.clone()); } else { s.shapes.layers.retain(|l| l != n); }
                    }
                }
                ui.checkbox(&mut s.shapes.offset, t("With easting and northing"));
            });
            part(ui, only, Part::Objects, &mut s.objects_on, true, &t("A list of objects Terrain Builder imports."), |ui| {
                let o = &mut s.objects;
                ui.add_enabled_ui(selected > 0, |ui| {
                    ui.checkbox(&mut s.selection_only, tf("Only the selection ({0})", &[&selected.to_string()]));
                });
                ui.checkbox(&mut o.exported_layers_only, t("Only layers marked for export"));
                ui.checkbox(&mut o.visible_only, t("Only visible layers"));
                ui.checkbox(&mut o.split_by_layer, t("One file for each layer"));
                ui.checkbox(&mut o.full_path, t("Model names with their whole path")).on_hover_text(t("Terrain Builder matches objects by file name; the whole path helps other tools."));
                ui.checkbox(&mut o.offset, t("With easting and northing"));
            });
            let has_mission = w.mission.is_some();
            let hint = if has_mission { tf("The mission folder {0}, as the server reads it from mpmissions.", &[&w.mission.as_ref().map(|m| m.name.clone()).unwrap_or_default()]) } else { t("The project has no mission.") };
            part(ui, only, Part::Mission, &mut s.mission_on, has_mission, &hint, |ui| {
                ui.checkbox(&mut s.mission.storage, t("With the persistence (storage folders)")).on_hover_text(t("Off starts the economy of the server afresh."));
            });
        });
        ui.separator();
        row(ui, &t("Folder"), |ui| {
            ui.add(egui::TextEdit::singleline(&mut s.folder).desired_width(250.0));
            if ui.button(icons::FOLDER_OPEN).clicked() {
                if let Some(d) = rfd::FileDialog::new().set_title(t("Export files")).pick_folder() { s.folder = d.display().to_string(); }
            }
        });
        ui.add_space(4.0);
        let ready = (only.is_some() || s.any()) && !s.folder.trim().is_empty();
        if ui.add_enabled(ready, egui::Button::new(icons::with(icons::EXPORT, &t("Export")))).clicked() {
            close = true;
        }
    });
    if s != app.settings.export {
        app.settings.export = s;
        app.settings.save();
    }
    if close {
        run(app, only);
    }
    close
}

/// Writes the chosen files in the background.
pub fn run(app: &mut App, only: Option<Part>) {
    let Some(doc) = &app.doc else { return };
    let s = app.settings.export.clone();
    let dir = PathBuf::from(s.folder.trim());
    let mut parts = s.parts(app.selection.iter().copied().collect(), only);
    if doc.world.satellite.is_none() { parts.satellite = None; }
    if doc.world.shape_layers.iter().all(|l| l.shapes.is_empty()) { parts.shapes = None; }
    if doc.world.mission.is_none() { parts.mission = None; }
    let world = doc.world.export_copy(parts.satellite.is_some());
    let db = app.db.clone();
    app.spawn(t("Export files"), move |job| {
        let files = export(&world, &db, &parts, &dir, job)?;
        Ok(Box::new(move |app: &mut App| {
            for f in &files { app.log.push(crate::app::LogLine { text: f.display().to_string(), level: 0 }); }
            app.info(tf("Exported {0} files to {1}", &[&files.len().to_string(), &dir.display().to_string()]));
        }) as Finish)
    });
}

pub fn open(app: &mut App, only: Option<Part>) {
    app.dialog = Dialog::Export(only);
}

/// The Export submenu of the File menu, laid out as Import is.
pub fn menu(app: &mut App, ui: &mut egui::Ui) {
    let Some(w) = app.doc.as_ref().map(|d| &d.world) else { return };
    let (has_sat, has_shapes) = (w.satellite.is_some(), w.shape_layers.iter().any(|l| !l.shapes.is_empty()));
    let mut chosen = None;
    if ui.button(t("All files...")).on_hover_text(t("Heightmap, satellite, mask, layers.cfg, shapes and objects, each with its options")).clicked() { chosen = Some(None); }
    ui.separator();
    if ui.button(t("Heightmap (ASC)...")).clicked() { chosen = Some(Some(Part::Heightmap)); }
    if ui.add_enabled(has_sat, egui::Button::new(t("Satellite image..."))).on_disabled_hover_text(t("The project has no satellite image.")).clicked() { chosen = Some(Some(Part::Satellite)); }
    if ui.button(t("Mask image...")).clicked() { chosen = Some(Some(Part::Mask)); }
    if ui.button(t("layers.cfg...")).clicked() { chosen = Some(Some(Part::Layers)); }
    if ui.button(t("Terrain Builder objects (TXT)...")).clicked() { chosen = Some(Some(Part::Objects)); }
    if ui.add_enabled(has_shapes, egui::Button::new(t("Shapes (SHP)..."))).on_disabled_hover_text(t("The project has no shapes.")).clicked() { chosen = Some(Some(Part::Shapes)); }
    if ui.add_enabled(w.mission.is_some(), egui::Button::new(t("Server mission..."))).on_disabled_hover_text(t("The project has no mission.")).clicked() { chosen = Some(Some(Part::Mission)); }
    if let Some(only) = chosen {
        ui.close_menu();
        open(app, only);
    }
}
