//! The export window: the parts of the map as files for other tools, each with its options.
use crate::app::{App, Dialog, Finish};
use crate::i18n::{t, tf};
use crate::icons;
use crate::theme::{self, check_row, details_head, row, section, segmented, ValueField};
use eframe::egui::{self, Color32, RichText, Stroke};
use dz_core::export_files::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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
    pub dayz_objects_on: bool,
    pub dayz_objects: DayzObjectsOptions,
    pub map2d_on: bool,
    pub map2d: dz_core::map2d::Map2dOptions,
    pub mission_on: bool,
    pub mission: MissionOptions,
}
impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            folder: String::new(), selection_only: false,
            heightmap_on: true, heightmap: Default::default(), satellite_on: true, satellite: Default::default(), mask_on: true, mask: Default::default(),
            layers_on: true, layers: Default::default(), shapes_on: false, shapes: Default::default(), objects_on: true, objects: Default::default(),
            dayz_objects_on: false, dayz_objects: Default::default(), map2d_on: false, map2d: Default::default(), mission_on: true, mission: Default::default(),
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
    DayzObjects,
    Map2d,
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
            Part::DayzObjects => t("DayZ Editor / Object Spawner"),
            Part::Map2d => t("2D player map (PNG)"),
            Part::Mission => t("Server mission"),
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Part::Heightmap => icons::MOUNTAINS,
            Part::Satellite => icons::IMAGE,
            Part::Mask => icons::PAINT_BRUSH,
            Part::Layers => icons::FILE_CODE,
            Part::Shapes => icons::POLYGON,
            Part::Objects => icons::CUBE,
            Part::DayzObjects => icons::PACKAGE,
            Part::Map2d => icons::MAP_TRIFOLD,
            Part::Mission => icons::GAME_CONTROLLER,
        }
    }
}

fn cached_system_fonts() -> &'static [(String, PathBuf)] {
    static FONTS: OnceLock<Vec<(String, PathBuf)>> = OnceLock::new();
    FONTS.get_or_init(dz_core::map2d::system_fonts)
}

fn font_display_name(path: &str) -> String {
    if path.trim().is_empty() {
        return t("Built-in (pixel)");
    }
    for (name, p) in cached_system_fonts() {
        if p.to_string_lossy().eq_ignore_ascii_case(path.trim()) {
            return name.clone();
        }
    }
    Path::new(path.trim())
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
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
            dayz_objects: on(Part::DayzObjects, self.dayz_objects_on).then(|| self.dayz_objects.clone()),
            map2d: on(Part::Map2d, self.map2d_on).then(|| self.map2d.clone()),
            mission: on(Part::Mission, self.mission_on).then(|| self.mission.clone()),
            selection: if self.selection_only { selection } else { Vec::new() },
        }
    }
    fn any(&self) -> bool {
        self.heightmap_on || self.satellite_on || self.mask_on || self.layers_on || self.shapes_on || self.objects_on || self.dayz_objects_on || self.map2d_on || self.mission_on
    }
}

/// A part of the export: styled with the editor's native `theme::section` and `check_row` widgets.
fn part(ui: &mut egui::Ui, only: Option<Part>, me: Part, on: &mut bool, enabled: bool, hint: &str, body: impl FnOnce(&mut egui::Ui)) {
    match only {
        Some(p) if p != me => return,
        Some(_) => {
            egui::Frame::new().inner_margin(egui::Margin { left: 8, right: 8, top: 8, bottom: 4 }).show(ui, |ui| {
                details_head(ui, me.icon(), theme::ACCENT, &me.title(), hint);
            });
            ui.add_enabled_ui(enabled, |ui| {
                section(ui, &icons::with(me.icon(), &me.title()), true, body);
            });
            return;
        }
        None => {}
    }
    let title = icons::with(me.icon(), &me.title());
    ui.add_enabled_ui(enabled, |ui| {
        let mut off = false;
        let active = if enabled { &mut *on } else { &mut off };
        egui::Frame::new()
            .fill(theme::PANEL)
            .stroke(Stroke::new(1.0_f32, theme::BORDER))
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                check_row(ui, active, RichText::new(title).strong())
                    .on_hover_text(hint)
                    .on_disabled_hover_text(hint);
                if *active && enabled {
                    ui.add_space(4.0);
                    egui::Frame::new()
                        .fill(theme::BG)
                        .corner_radius(3.0)
                        .inner_margin(egui::Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            body(ui);
                        });
                }
            });
        ui.add_space(4.0);
    });
}

fn image_ui(ui: &mut egui::Ui, o: &mut ImageOptions, own: usize, id: &str) {
    row(ui, &t("Format"), |ui| {
        let choices: Vec<_> = ImageFormat::ALL.iter().map(|&f| (f, f.extension().to_uppercase(), String::new())).collect();
        segmented(ui, &mut o.format, &choices);
    });
    row(ui, &t("Size"), |ui| {
        let text = |s: usize| if s == 0 { format!("{} ({own} px)", t("As it is")) } else { format!("{s} px") };
        egui::ComboBox::from_id_salt(("size", id)).width(ui.available_width()).selected_text(text(o.size)).show_ui(ui, |ui| {
            for s in [0, 1024, 2048, 4096, 8192, 10240, 12288, 16384, 20480] { ui.selectable_value(&mut o.size, s, text(s)); }
        });
    });
    row(ui, &t("Tiles"), |ui| {
        egui::ComboBox::from_id_salt(("tiles", id)).width(ui.available_width()).selected_text(if o.tiles <= 1 { t("One image") } else { format!("{0} × {0}", o.tiles) }).show_ui(ui, |ui| {
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
    let map_max_px = dz_core::map2d::native_size(w);
    let shape_names: Vec<String> = w.shape_layers.iter().filter(|l| !l.shapes.is_empty()).map(|l| l.name.clone()).collect();
    let selected = app.selection.len();
    let title = only.map_or_else(|| t("Export files"), |p| format!("{} – {}", t("Export"), p.title()));
    egui::Window::new(icons::with(icons::EXPORT, &title))
        .id(egui::Id::new("export_window"))
        .collapsible(false)
        .resizable(false)
        .min_width(480.0)
        .max_width(480.0)
        .open(open)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::ZERO))
        .show(ctx, |ui| {
            egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(ctx.screen_rect().height() * 0.68).show(ui, |ui| {
                    part(ui, only, Part::Heightmap, &mut s.heightmap_on, true, &t("The terrain as an ESRI ASCII grid, as Terrain Builder and GIS tools read it."), |ui| {
                        let h = &mut s.heightmap;
                        check_row(ui, &mut h.with_modifiers, t("With terrain modifiers"))
                            .on_hover_text(t("Roads, pads and ponds cut into the terrain; without them the terrain as sculpted."));
                        row(ui, &t("Cell"), |ui| {
                            let own = w.height.cell;
                            let text = |c: f32| if c <= 0.0 { format!("{} ({own} m)", t("As it is")) } else { format!("{c} m") };
                            egui::ComboBox::from_id_salt("asc_cell").width(ui.available_width()).selected_text(text(h.cell)).show_ui(ui, |ui| {
                                for c in [0.0, 1.0, 2.0, 4.0, 5.0, 8.0, 10.0, 16.0] { ui.selectable_value(&mut h.cell, c, text(c)); }
                            });
                        });
                        let side = w.height.cols as f32 * w.height.cell;
                        let n = if h.cell > 0.0 { (side / h.cell).round() as usize } else { w.height.cols };
                        ui.weak(tf("{0} × {0} samples", &[&n.to_string()]));
                        row(ui, &t("Decimals"), |ui| { ui.add(ValueField::new(&mut h.decimals, 0..=6)); });
                        row(ui, &t("Raise by"), |ui| { ui.add(ValueField::new(&mut h.raise, -1000.0..=1000.0).step_by(0.1).suffix(" m")); });
                        check_row(ui, &mut h.offset, t("With easting and northing"))
                            .on_hover_text(t("The corner of the grid in map coordinates, as Terrain Builder expects; off places it at 0, 0."));
                    });
                    part(ui, only, Part::Satellite, &mut s.satellite_on, has_sat, &if has_sat { t("The satellite image of the map.") } else { t("The project has no satellite image.") }, |ui| {
                        check_row(ui, &mut s.satellite.with_modifiers, t("With satellite modifiers"))
                            .on_hover_text(t("Baked objects and road parts, clone strokes; without them the image as imported."));
                        image_ui(ui, &mut s.satellite.image, sat_px, "sat");
                    });
                    part(ui, only, Part::Mask, &mut s.mask_on, true, &t("Each surface in its colour from layers.cfg."), |ui| {
                        check_row(ui, &mut s.mask.with_modifiers, t("With mask modifiers"))
                            .on_hover_text(t("Surfaces under objects, roads and forests; without them the mask as painted."));
                        image_ui(ui, &mut s.mask.image, w.mask.width, "mask");
                        ui.weak(t("Scaled without blending, so it only holds the colours of the surfaces."));
                    });
                    part(ui, only, Part::Layers, &mut s.layers_on, true, &t("The surfaces with their colours and materials, for Terrain Builder."), |ui| {
                        row(ui, &t("Legend"), |ui| { ui.add_sized([ui.available_width(), 22.0], egui::TextEdit::singleline(&mut s.layers.legend)); });
                        check_row(ui, &mut s.layers.only_used, t("Only surfaces the mask uses"));
                    });
                    part(ui, only, Part::Shapes, &mut s.shapes_on, !shape_names.is_empty(), &if shape_names.is_empty() { t("The project has no shapes.") } else { t("Each shape layer as a shapefile.") }, |ui| {
                        ui.label(t("Layers"));
                        ui.weak(t("None ticked writes the visible ones."));
                        for n in &shape_names {
                            let mut on = s.shapes.layers.contains(n);
                            if check_row(ui, &mut on, n).changed() {
                                if on { s.shapes.layers.push(n.clone()); } else { s.shapes.layers.retain(|l| l != n); }
                            }
                        }
                        check_row(ui, &mut s.shapes.offset, t("With easting and northing"));
                    });
                    part(ui, only, Part::Objects, &mut s.objects_on, true, &t("A list of objects Terrain Builder imports."), |ui| {
                        let o = &mut s.objects;
                        ui.add_enabled_ui(selected > 0, |ui| {
                            check_row(ui, &mut s.selection_only, tf("Only the selection ({0})", &[&selected.to_string()]));
                        });
                        check_row(ui, &mut o.exported_layers_only, t("Only layers marked for export"));
                        check_row(ui, &mut o.visible_only, t("Only visible layers"));
                        check_row(ui, &mut o.split_by_layer, t("One file for each layer"));
                        check_row(ui, &mut o.full_path, t("Model names with their whole path"))
                            .on_hover_text(t("Terrain Builder matches objects by file name; the whole path helps other tools."));
                        check_row(ui, &mut o.offset, t("With easting and northing"));
                    });
                    part(ui, only, Part::DayzObjects, &mut s.dayz_objects_on, true, &t("Objects for DayZ Object Spawner (cfggameplay.json) or DayZ Editor (.dze)."), |ui| {
                        use dz_core::dayz_editor::DayzObjectFormat;
                        let d = &mut s.dayz_objects;
                        row(ui, &t("Format"), |ui| {
                            segmented(ui, &mut d.format, &[
                                (DayzObjectFormat::ObjectSpawnerJson, t("Object Spawner (JSON)"), String::new()),
                                (DayzObjectFormat::Dze, t("DayZ Editor (.dze)"), String::new()),
                            ]);
                        });
                        ui.add_enabled_ui(selected > 0, |ui| {
                            check_row(ui, &mut s.selection_only, tf("Only the selection ({0})", &[&selected.to_string()]));
                        });
                        check_row(ui, &mut d.exported_layers_only, t("Only layers marked for export"));
                        check_row(ui, &mut d.visible_only, t("Only visible layers"));
                        check_row(ui, &mut d.enable_ce_persistency, t("Enable CE persistency"))
                            .on_hover_text(t("Objects keep their Central Economy lifetime and persistency when spawned by Object Spawner."));
                    });
                    part(ui, only, Part::Map2d, &mut s.map2d_on, true, &t("A ready-to-share 2D map image with contours, roads, buildings, place names and grid."), |ui| {
                        use dz_core::map2d::{Map2dFontSize, Map2dStyle};
                        let m = &mut s.map2d;
                        row(ui, &t("Map style"), |ui| {
                            if has_sat {
                                segmented(ui, &mut m.style, &[
                                    (Map2dStyle::Topographic, t("Topographic"), String::new()),
                                    (Map2dStyle::Satellite, t("Satellite"), String::new()),
                                ]);
                            } else {
                                m.style = Map2dStyle::Topographic;
                                ui.horizontal(|ui| {
                                    let _ = theme::tool_button(ui, true, &t("Topographic"));
                                    ui.add_enabled(false, egui::Button::new(t("Satellite")));
                                });
                            }
                        });
                        if m.style == Map2dStyle::Satellite && has_sat {
                            check_row(ui, &mut m.sat_modifiers, t("With satellite modifiers"))
                                .on_hover_text(t("Baked objects and road parts, clone strokes; without them the image as imported."));
                        }
                        row(ui, &t("Size"), |ui| {
                            let size_label = |sz: usize| {
                                if sz == 0 {
                                    format!("{} ({map_max_px} px)", t("Full map"))
                                } else {
                                    format!("{sz} px")
                                }
                            };
                            egui::ComboBox::from_id_salt("map2d_size")
                                .width(ui.available_width())
                                .selected_text(size_label(m.size))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut m.size, 0, size_label(0));
                                    for sz in [1024usize, 2048, 4096, 8192, 10240, 12288, 16384, 20480] {
                                        if sz <= map_max_px.max(8192) {
                                            ui.selectable_value(&mut m.size, sz, size_label(sz));
                                        }
                                    }
                                });
                        });
                        row(ui, &t("Contour step"), |ui| {
                            let lbl = |v: f32| if v <= 0.0 { t("Off") } else { format!("{v:.0} m") };
                            egui::ComboBox::from_id_salt("map2d_contour").width(ui.available_width()).selected_text(lbl(m.contour_interval)).show_ui(ui, |ui| {
                                for v in [0.0, 5.0, 10.0, 20.0, 50.0] {
                                    ui.selectable_value(&mut m.contour_interval, v, lbl(v));
                                }
                            });
                        });
                        row(ui, &t("Grid step"), |ui| {
                            let lbl = |v: f32| if v <= 0.0 { t("Off") } else { format!("{v:.0} m") };
                            egui::ComboBox::from_id_salt("map2d_grid").width(ui.available_width()).selected_text(lbl(m.grid_step)).show_ui(ui, |ui| {
                                for v in [0.0, 500.0, 1000.0, 2000.0] {
                                    ui.selectable_value(&mut m.grid_step, v, lbl(v));
                                }
                            });
                        });
                        check_row(ui, &mut m.roads, t("Roads"));
                        check_row(ui, &mut m.buildings, t("Building footprints"))
                            .on_hover_text(t("Draw building outlines on top of the map; turn off to keep only what is baked into the satellite."));
                        check_row(ui, &mut m.trees, t("Trees and bushes"))
                            .on_hover_text(t("Draws placed trees and bushes as forest canopy on the map."));
                        check_row(ui, &mut m.keypoints, t("Place names"));
                        if m.keypoints {
                            row(ui, &t("Font"), |ui| {
                                ui.horizontal(|ui| {
                                    let combo_w = (ui.available_width() - 30.0).max(120.0);
                                    egui::ComboBox::from_id_salt("map2d_font")
                                        .width(combo_w)
                                        .selected_text(font_display_name(&m.font))
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(&mut m.font, String::new(), t("Built-in (pixel)"));
                                            for (name, path) in cached_system_fonts() {
                                                ui.selectable_value(&mut m.font, path.display().to_string(), name);
                                            }
                                        });
                                    if ui.button(icons::FOLDER_OPEN).on_hover_text(t("Choose a font file")).clicked() {
                                        if let Some(p) = rfd::FileDialog::new()
                                            .set_title(t("Choose a font file"))
                                            .add_filter("Fonts", &["ttf", "otf", "ttc"])
                                            .pick_file()
                                        {
                                            m.font = p.display().to_string();
                                        }
                                    }
                                });
                            });
                            row(ui, &t("Font size"), |ui| {
                                segmented(ui, &mut m.font_size, &[
                                    (Map2dFontSize::Small, t("Small"), String::new()),
                                    (Map2dFontSize::Medium, t("Medium"), String::new()),
                                    (Map2dFontSize::Large, t("Large"), String::new()),
                                ]);
                            });
                        }
                    });
                    let has_mission = w.mission.is_some();
                    let hint = if has_mission { tf("The mission folder {0}, as the server reads it from mpmissions.", &[&w.mission.as_ref().map(|m| m.name.clone()).unwrap_or_default()]) } else { t("The project has no mission.") };
                    part(ui, only, Part::Mission, &mut s.mission_on, has_mission, &hint, |ui| {
                        check_row(ui, &mut s.mission.storage, t("With the persistence (storage folders)"))
                            .on_hover_text(t("Off starts the economy of the server afresh."));
                    });
                });
            });

            // Footer bar matching the editor's native panels
            egui::Frame::new()
                .fill(theme::PANEL)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    row(ui, &t("Folder"), |ui| {
                        ui.horizontal(|ui| {
                            let w_edit = (ui.available_width() - 32.0).max(140.0);
                            ui.add_sized([w_edit, 22.0], egui::TextEdit::singleline(&mut s.folder));
                            if ui.button(icons::FOLDER_OPEN).clicked() {
                                if let Some(d) = rfd::FileDialog::new().set_title(t("Export files")).pick_folder() { s.folder = d.display().to_string(); }
                            }
                        });
                    });
                    ui.add_space(6.0);
                    let ready = (only.is_some() || s.any()) && !s.folder.trim().is_empty();
                    ui.horizontal(|ui| {
                        let btn = egui::Button::new(RichText::new(icons::with(icons::EXPORT, &t("Export"))).color(Color32::WHITE))
                            .fill(theme::ACCENT)
                            .min_size(egui::vec2(110.0, 26.0));
                        if ui.add_enabled(ready, btn).clicked() {
                            close = true;
                        }
                    });
                });
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
    let need_sat = parts.satellite.is_some() || parts.map2d.as_ref().is_some_and(|m| m.style == dz_core::map2d::Map2dStyle::Satellite);
    let world = doc.world.export_copy(need_sat);
    let db = app.db.clone();
    let classes = crate::mission_ui::class_index(app);
    let work_drive = app.settings.tools.work_drive.clone();
    let need_classes = parts.dayz_objects.is_some();
    app.spawn(t("Export files"), move |job| {
        let loaded_classes = if need_classes && classes.is_none() {
            let root = PathBuf::from(&work_drive);
            root.is_dir().then(|| dz_core::mission::classes::ClassIndex::scan_work_drive(&root))
        } else {
            None
        };
        let cls = classes.as_deref().or(loaded_classes.as_ref());
        let files = export_with_classes(&world, &db, cls, &parts, &dir, job)?;
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
    if ui.button(t("DayZ Editor / Object Spawner (.dze, .json)...")).clicked() { chosen = Some(Some(Part::DayzObjects)); }
    if ui.button(t("2D player map (PNG)...")).clicked() { chosen = Some(Some(Part::Map2d)); }
    if ui.add_enabled(has_shapes, egui::Button::new(t("Shapes (SHP)..."))).on_disabled_hover_text(t("The project has no shapes.")).clicked() { chosen = Some(Some(Part::Shapes)); }
    if ui.add_enabled(w.mission.is_some(), egui::Button::new(t("Server mission..."))).on_disabled_hover_text(t("The project has no mission.")).clicked() { chosen = Some(Some(Part::Mission)); }
    if let Some(only) = chosen {
        ui.close_menu();
        open(app, only);
    }
}
