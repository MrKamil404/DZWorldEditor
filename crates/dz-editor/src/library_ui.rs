//! The generator and object preset libraries, generators in the project and the road catalog.
use crate::app::*;
use crate::i18n::{t, tf};
use crate::theme::{self, row};
use dz_core::edit::Change;
use dz_core::gen;
use dz_core::library::*;
use dz_core::objects::Obj;
use dz_core::shapes::{MapShape, ShapeKind, ShapeLayer};
use dz_core::world::World;
use eframe::egui::{self, vec2, Color32};
use std::path::PathBuf;
use std::sync::Arc;

/// What is being dragged out of a library.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LibDrag {
    Generator(u64),
    ObjectPreset(u64),
}

pub fn library_path() -> PathBuf {
    crate::settings::data_dir().join("library.json")
}

/// Everything the panels of a generator need to know about the project.
struct KindContext {
    layers: Vec<String>,
    presets: Vec<String>,
    model: Option<String>,
    has_satellite: bool,
}

fn context(app: &App, user: &[forest_core::user_presets::UserPreset]) -> KindContext {
    let doc = app.doc.as_ref();
    let mut presets: Vec<String> = user.iter().map(|p| p.name.clone()).collect();
    presets.extend(gen::forest_preset_names());
    KindContext {
        layers: doc.map(|d| d.world.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default(),
        presets,
        model: app.place.model.clone(),
        has_satellite: doc.is_some_and(|d| d.world.satellite.is_some()),
    }
}

fn number(ui: &mut egui::Ui, label: String, v: &mut f32, range: std::ops::RangeInclusive<f32>) {
    row(ui, &label, |ui| { ui.add(egui::Slider::new(v, range)); });
}

fn number64(ui: &mut egui::Ui, label: String, v: &mut f64, range: std::ops::RangeInclusive<f64>) {
    row(ui, &label, |ui| { ui.add(egui::Slider::new(v, range)); });
}

/// A model field: takes the model chosen in the browser, or one dropped on it.
fn model_field(ui: &mut egui::Ui, label: String, value: &mut String, selected: &Option<String>) {
    row(ui, &label, |ui| {
        let name = value.rsplit('\\').next().unwrap_or("").to_string();
        let shown = ui.add(egui::Label::new(if name.is_empty() { t("none") } else { name }).sense(egui::Sense::hover())).on_hover_text(value.as_str());
        if let Some(p) = shown.dnd_release_payload::<String>() {
            *value = (*p).clone();
        }
        if ui.small_button(t("Use selected")).clicked() {
            if let Some(m) = selected { *value = m.clone(); }
        }
        if !value.is_empty() && ui.small_button("✖").clicked() { value.clear(); }
    });
}

fn preset_combo(ui: &mut egui::Ui, id: impl std::hash::Hash, value: &mut String, presets: &[String]) {
    egui::ComboBox::from_id_salt(id).width(170.0).selected_text(value.clone()).show_ui(ui, |ui| {
        for p in presets { ui.selectable_value(value, p.clone(), p); }
    });
}

/// Fields of a generator. Returns true when the colour eyedropper should start.
fn kind_ui(ui: &mut egui::Ui, kind: &mut GeneratorKind, cx: &KindContext) -> bool {
    let mut sample = false;
    match kind {
        GeneratorKind::Forest(g) => {
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("Load DayZForests project...")).clicked() {
                    if let Some(f) = rfd::FileDialog::new().add_filter("DayZForests", &["json"]).pick_file() {
                        if let Some(p) = std::fs::read(&f).ok().and_then(|b| serde_json::from_slice(&b).ok()) { g.project = p; }
                    }
                }
                if ui.button(t("Load zone presets...")).on_hover_text("zones.json, presets_user.json").clicked() {
                    if let Some(f) = rfd::FileDialog::new().add_filter("DayZForests", &["json"]).pick_file() {
                        if let Ok(l) = forest_core::user_presets::UserPresetLibrary::load(&f) { g.user_presets = l.presets; }
                    }
                }
            });
            let p = &mut g.project;
            ui.weak(tf("{0} species, {1} areas, {2} zones in the project", &[&p.species.len().to_string(), &p.areas.len().to_string(), &p.zones.len().to_string()]));
            row(ui, &t("Preset in shapes"), |ui| preset_combo(ui, "area", &mut g.preset, &cx.presets));
            row(ui, &t("Seed"), |ui| { ui.add(egui::DragValue::new(&mut p.seed)); });
            number64(ui, t("Spacing"), &mut p.spacing_multiplier, 0.3..=4.0);
            number(ui, t("Min scale"), &mut p.scale_min, 0.1..=3.0);
            number(ui, t("Max scale"), &mut p.scale_max, 0.1..=3.0);
            number64(ui, t("Clearings"), &mut p.clearing_strength, 0.0..=0.95);
            number64(ui, t("Clearing size m"), &mut p.clearing_scale_m, 0.0..=1500.0);
            let mut slope = p.max_slope_deg.unwrap_or(90.0);
            number64(ui, t("Max slope"), &mut slope, 5.0..=90.0);
            p.max_slope_deg = (slope < 90.0).then_some(slope);
            number64(ui, t("Edge padding m"), &mut p.edge_padding_m, 0.0..=200.0);
            ui.separator();
            ui.strong(t("Zones from the mask"));
            let mut remove = None;
            for (i, z) in g.zones.iter_mut().enumerate() {
                ui.push_id(i, |ui| ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("layer").width(100.0).selected_text(z.0.clone()).show_ui(ui, |ui| {
                        for n in &cx.layers { ui.selectable_value(&mut z.0, n.clone(), n); }
                    });
                    preset_combo(ui, "preset", &mut z.1, &cx.presets);
                    if ui.small_button("✖").clicked() { remove = Some(i); }
                }));
            }
            if let Some(i) = remove { g.zones.remove(i); }
            if ui.button(format!("+ {}", t("Add zone"))).clicked() {
                g.zones.push((cx.layers.first().cloned().unwrap_or_default(), cx.presets.first().cloned().unwrap_or_default()));
            }
            ui.separator();
            ui.checkbox(&mut g.project.edges.enabled, t("Forest edge"));
            if g.project.edges.enabled {
                number64(ui, t("Edge width m"), &mut g.project.edges.band_width_m, 1.0..=80.0);
                number(ui, t("Edge density"), &mut g.project.edges.density_per_ha, 0.0..=600.0);
                number64(ui, t("Jagged edge m"), &mut g.project.edges.jagged_m, 0.0..=120.0);
                number64(ui, t("Blend inward m"), &mut g.project.edges.blend_inside_m, 0.0..=150.0);
                ui.weak(t("Edge presets"));
                let mut remove = None;
                for (i, e) in g.edge_presets.iter_mut().enumerate() {
                    ui.push_id(("edge", i), |ui| ui.horizontal(|ui| {
                        preset_combo(ui, "preset", &mut e.0, &cx.presets);
                        ui.add(egui::DragValue::new(&mut e.1).speed(0.01).range(0.01..=1.0));
                        if ui.small_button("✖").clicked() { remove = Some(i); }
                    }));
                }
                if let Some(i) = remove { g.edge_presets.remove(i); }
                if ui.button(format!("+ {}", t("Add edge preset"))).clicked() {
                    g.edge_presets.push((cx.presets.first().cloned().unwrap_or_default(), 0.5));
                }
            }
            ui.separator();
            let mut filtered = g.color_filter.is_some();
            if ui.checkbox(&mut filtered, t("Only on satellite colours")).changed() {
                g.color_filter = filtered.then(|| forest_core::preset::ColorFilter { samples: Vec::new(), tolerance: 60, invert: false });
            }
            if let Some(f) = &mut g.color_filter {
                ui.horizontal_wrapped(|ui| {
                    let mut remove = None;
                    for (i, s) in f.samples.iter_mut().enumerate() {
                        let color = Color32::from_rgb(s.0[0], s.0[1], s.0[2]);
                        let (rect, r) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::click());
                        ui.painter().rect_filled(rect, 2.0, color);
                        if r.on_hover_text(t("Click removes this sample")).clicked() { remove = Some(i); }
                    }
                    if let Some(i) = remove { f.samples.remove(i); }
                });
                ui.add_enabled_ui(cx.has_satellite, |ui| {
                    if ui.button(t("Pick from the map")).on_hover_text(t("The next click in a view takes the satellite colour there.")).clicked() { sample = true; }
                });
                let mut tolerance = f.tolerance as f32;
                number(ui, t("Tolerance"), &mut tolerance, 0.0..=255.0);
                f.tolerance = tolerance as u32;
                ui.checkbox(&mut f.invert, t("Everywhere except these colours"));
            }
        }
        GeneratorKind::SurfacePaint { rules, seed } => {
            row(ui, &t("Seed"), |ui| { ui.add(egui::DragValue::new(seed)); });
            let mut remove = None;
            for (i, r) in rules.iter_mut().enumerate() {
                ui.push_id(i, |ui| {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("layer").width(120.0).selected_text(cx.layers.get(r.layer).cloned().unwrap_or_default()).show_ui(ui, |ui| {
                            for (k, n) in cx.layers.iter().enumerate() { ui.selectable_value(&mut r.layer, k, n); }
                        });
                        if ui.small_button("✖").clicked() { remove = Some(i); }
                    });
                    number(ui, t("Min height"), &mut r.min_height, -200.0..=1500.0);
                    number(ui, t("Max height"), &mut r.max_height, -200.0..=9000.0);
                    number(ui, t("Min slope"), &mut r.min_slope, 0.0..=90.0);
                    number(ui, t("Max slope"), &mut r.max_slope, 0.0..=90.0);
                    number(ui, t("Blotch size m"), &mut r.noise_scale, 0.0..=2000.0);
                    number(ui, t("Coverage"), &mut r.coverage, 0.0..=1.0);
                    ui.separator();
                });
            }
            if let Some(i) = remove { rules.remove(i); }
            if ui.button(format!("+ {}", t("Add rule"))).clicked() {
                rules.push(gen::PaintRule { layer: 0, min_height: -1000.0, max_height: 9000.0, min_slope: 0.0, max_slope: 90.0, noise_scale: 0.0, coverage: 1.0 });
            }
        }
        GeneratorKind::Settlement(s) => {
            ui.weak(t("Buildings: pick in the Content Browser and press Add, or drop them here."));
            let mut remove = None;
            let list = ui.vertical(|ui| {
                for (i, m) in s.buildings.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(m.rsplit('\\').next().unwrap_or(m));
                        if ui.small_button("✖").clicked() { remove = Some(i); }
                    });
                }
            }).response;
            if let Some(p) = list.dnd_release_payload::<String>() { if !s.buildings.contains(&p) { s.buildings.push((*p).clone()); } }
            if let Some(i) = remove { s.buildings.remove(i); }
            if ui.button(format!("+ {}", t("Add selected model"))).clicked() {
                if let Some(m) = &cx.model { if !s.buildings.contains(m) { s.buildings.push(m.clone()); } }
            }
            number(ui, t("Radius"), &mut s.radius, 40.0..=1500.0);
            row(ui, &t("Streets"), |ui| { ui.add(egui::Slider::new(&mut s.streets, 1..=12)); });
            number(ui, t("Plot size"), &mut s.plot, 10.0..=80.0);
            number(ui, t("Distance from street"), &mut s.setback, 2.0..=40.0);
            number(ui, t("Max slope"), &mut s.max_slope, 2.0..=45.0);
            row(ui, &t("Seed"), |ui| { ui.add(egui::DragValue::new(&mut s.seed)); });
            ui.checkbox(&mut s.flatten, t("Level the ground under buildings"));
        }
        GeneratorKind::Fence(g) => {
            model_field(ui, t("Model"), &mut g.model, &cx.model);
            number(ui, t("Spacing"), &mut g.spacing, 0.0..=60.0);
            ui.weak(t("Spacing 0 uses the length of the model."));
            number(ui, t("Turn"), &mut g.yaw_offset, -180.0..=180.0);
            ui.checkbox(&mut g.follow_slope, t("Follow the slope"));
        }
        GeneratorKind::PowerLine(g) => {
            model_field(ui, t("Tower"), &mut g.tower, &cx.model);
            model_field(ui, t("Corner tower"), &mut g.corner, &cx.model);
            model_field(ui, t("Wires"), &mut g.wire, &cx.model);
            number(ui, t("Spacing"), &mut g.spacing, 0.0..=300.0);
            ui.weak(t("Spacing 0 uses the length of the wire model."));
            number(ui, t("Turn"), &mut g.yaw_offset, -180.0..=180.0);
        }
    }
    sample
}

/// A library entry drawn like a model in the browser.
fn tile(ui: &mut egui::Ui, name: &str, letter: &str, color: Color32, selected: bool, image: Option<egui::TextureId>) -> egui::Response {
    let cell = vec2(104.0, 122.0);
    let (rect, resp) = ui.allocate_exact_size(cell, egui::Sense::click_and_drag());
    ui.painter().rect_filled(rect, 3.0, if selected { theme::ACCENT_SOFT } else if resp.hovered() { theme::HOVER } else { Color32::from_rgb(42, 42, 44) });
    let art = egui::Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(96.0, 96.0));
    match image {
        Some(id) => { ui.painter().image(id, art, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE); }
        None => {
            ui.painter().rect_filled(art, 2.0, color);
            ui.painter().text(art.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(40.0), Color32::from_white_alpha(210));
        }
    }
    let short: String = if name.chars().count() > 15 { name.chars().take(14).chain(['…']).collect() } else { name.to_string() };
    ui.painter().text(egui::pos2(rect.center().x, rect.bottom() - 11.0), egui::Align2::CENTER_CENTER, short, egui::FontId::proportional(11.0), Color32::from_gray(225));
    resp.on_hover_text(name)
}

fn kind_look(kind: &GeneratorKind) -> (&'static str, Color32) {
    match kind {
        GeneratorKind::Forest(_) => ("F", Color32::from_rgb(44, 96, 56)),
        GeneratorKind::SurfacePaint { .. } => ("S", Color32::from_rgb(120, 92, 48)),
        GeneratorKind::Settlement(_) => ("B", Color32::from_rgb(112, 64, 58)),
        GeneratorKind::Fence(_) => ("W", Color32::from_rgb(86, 88, 96)),
        GeneratorKind::PowerLine(_) => ("P", Color32::from_rgb(56, 84, 128)),
    }
}

/// The folder list beside a library; entries are moved by dropping them on a folder.
fn folder_list(ui: &mut egui::Ui, current: &mut Option<String>, folders: &[String], new_name: &mut String) -> (Option<(LibDrag, String)>, Option<String>) {
    let mut moved = None;
    let mut created = None;
    if ui.selectable_label(current.is_none(), t("All")).clicked() { *current = None; }
    for f in folders {
        let r = ui.selectable_label(current.as_deref() == Some(f.as_str()), f);
        if r.clicked() { *current = Some(f.clone()); }
        if let Some(p) = r.dnd_release_payload::<LibDrag>() { moved = Some((*p, f.clone())); }
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(new_name).hint_text(t("New folder")).desired_width(110.0));
        if ui.small_button("+").clicked() && !new_name.trim().is_empty() {
            created = Some(new_name.trim().to_string());
            new_name.clear();
        }
    });
    (moved, created)
}

fn grid_columns(ui: &egui::Ui) -> usize {
    (ui.available_width() / 108.0).floor().max(1.0) as usize
}

pub fn generators_tab(app: &mut App, ui: &mut egui::Ui) {
    let mut folders = vec![BUILT_IN.to_string()];
    folders.extend(app.library.generator_folders.iter().cloned());
    let (mut current, mut new_name) = (app.gen_folder.clone(), std::mem::take(&mut app.new_folder));
    let mut result = (None, None);
    egui::SidePanel::left("gen_folders").resizable(true).default_width(180.0).frame(egui::Frame::new()).show_inside(ui, |ui| {
        ui.menu_button(format!("+ {}", t("New generator")), |ui| {
            for b in &app.built_in {
                if ui.button(t(b.kind.type_name())).clicked() {
                    let folder = current.clone().filter(|f| f != BUILT_IN).unwrap_or_default();
                    let id = new_id();
                    app.library.generators.push(GeneratorPreset { id, name: t(b.kind.type_name()), folder, kind: b.kind.clone() });
                    app.edit_generator = Some(id);
                    app.selected_generator = None;
                    app.lib_dirty = true;
                    ui.close_menu();
                }
            }
        });
        egui::ScrollArea::vertical().id_salt("gf").auto_shrink([false, false]).show(ui, |ui| {
            result = folder_list(ui, &mut current, &folders, &mut new_name);
        });
    });
    app.gen_folder = current.clone();
    app.new_folder = new_name;
    if let Some((LibDrag::Generator(id), folder)) = result.0 {
        if folder != BUILT_IN {
            if let Some(g) = app.library.generators.iter_mut().find(|g| g.id == id) { g.folder = folder; app.lib_dirty = true; }
        }
    }
    if let Some(name) = result.1 {
        if !app.library.generator_folders.contains(&name) { app.library.generator_folders.push(name); app.lib_dirty = true; }
    }
    let shown: Vec<(u64, String, &'static str, Color32, bool)> = app.built_in.iter().map(|g| (g, true)).chain(app.library.generators.iter().map(|g| (g, false)))
        .filter(|(g, _)| current.as_deref().is_none_or(|f| f == g.folder))
        .map(|(g, b)| { let (l, c) = kind_look(&g.kind); (g.id, if b { t(&g.name) } else { g.name.clone() }, l, c, b) }).collect();
    let columns = grid_columns(ui);
    let mut add = None;
    let mut delete = None;
    let mut copy = None;
    egui::ScrollArea::vertical().id_salt("gens").auto_shrink([false, false]).show(ui, |ui| {
        for chunk in shown.chunks(columns) {
            ui.horizontal(|ui| {
                for (id, name, letter, color, built_in) in chunk {
                    let r = tile(ui, name, letter, *color, app.edit_generator == Some(*id), None);
                    if r.clicked() { app.edit_generator = Some(*id); app.selected_generator = None; }
                    if r.double_clicked() { add = Some(*id); }
                    r.dnd_set_drag_payload(LibDrag::Generator(*id));
                    r.context_menu(|ui| {
                        if ui.button(t("Add to the project")).clicked() { add = Some(*id); ui.close_menu(); }
                        if ui.button(t("Duplicate")).clicked() { copy = Some(*id); ui.close_menu(); }
                        if ui.add_enabled(!*built_in, egui::Button::new(t("Delete"))).clicked() { delete = Some(*id); ui.close_menu(); }
                    });
                }
            });
        }
    });
    if let Some(id) = copy {
        if let Some(g) = app.built_in.iter().chain(app.library.generators.iter()).find(|g| g.id == id).cloned() {
            let new = new_id();
            app.library.generators.push(GeneratorPreset { id: new, name: format!("{} 2", if g.folder == BUILT_IN { t(&g.name) } else { g.name.clone() }), folder: if g.folder == BUILT_IN { String::new() } else { g.folder }, kind: g.kind });
            app.edit_generator = Some(new);
            app.lib_dirty = true;
        }
    }
    if let Some(id) = delete {
        app.library.generators.retain(|g| g.id != id);
        if app.edit_generator == Some(id) { app.edit_generator = None; }
        app.lib_dirty = true;
    }
    if let Some(id) = add { add_instance(app, id, None); }
}

pub fn presets_tab(app: &mut App, ui: &mut egui::Ui) {
    const ROADS: &str = "Roads";
    let mut folders = vec![t(ROADS)];
    folders.extend(app.library.object_folders.iter().cloned());
    let roads_label = t(ROADS);
    let (mut current, mut new_name) = (app.preset_folder.clone(), std::mem::take(&mut app.new_folder));
    let mut result = (None, None);
    egui::SidePanel::left("preset_folders").resizable(true).default_width(180.0).frame(egui::Frame::new()).show_inside(ui, |ui| {
        if ui.button(format!("+ {}", t("New preset"))).clicked() {
            let id = new_id();
            let items = app.place.model.iter().map(|m| (m.clone(), 1.0)).collect();
            let folder = current.clone().filter(|f| *f != roads_label).unwrap_or_default();
            app.library.objects.push(ObjectPreset { id, name: t("New preset"), folder, items, scale_min: 1.0, scale_max: 1.0, random_yaw: true, align: false });
            app.edit_object = Some(id);
            app.lib_dirty = true;
        }
        if ui.button(t("Road catalog...")).clicked() { app.dialog = Dialog::RoadCatalog; }
        egui::ScrollArea::vertical().id_salt("pf").auto_shrink([false, false]).show(ui, |ui| {
            result = folder_list(ui, &mut current, &folders, &mut new_name);
        });
    });
    app.preset_folder = current.clone();
    app.new_folder = new_name;
    if let Some((LibDrag::ObjectPreset(id), folder)) = result.0 {
        if folder != roads_label {
            if let Some(p) = app.library.objects.iter_mut().find(|p| p.id == id) { p.folder = folder; app.lib_dirty = true; }
        }
    }
    if let Some(name) = result.1 {
        if name != roads_label && !app.library.object_folders.contains(&name) { app.library.object_folders.push(name); app.lib_dirty = true; }
    }
    let columns = grid_columns(ui);
    let show_roads = current.as_deref().is_none_or(|f| f == roads_label);
    let families: Vec<String> = if show_roads { app.road_families.clone() } else { Vec::new() };
    let shown: Vec<(u64, String, Option<String>)> = app.library.objects.iter().filter(|p| current.as_deref().is_none_or(|f| f == p.folder)).map(|p| (p.id, p.name.clone(), p.items.first().map(|i| i.0.clone()))).collect();
    enum Entry { Road(String), Preset(u64, String, Option<String>) }
    let entries: Vec<Entry> = families.into_iter().map(Entry::Road).chain(shown.into_iter().map(|(a, b, c)| Entry::Preset(a, b, c))).collect();
    let mut delete = None;
    egui::ScrollArea::vertical().id_salt("presets").auto_shrink([false, false]).show(ui, |ui| {
        for chunk in entries.chunks(columns) {
            ui.horizontal(|ui| {
                for e in chunk {
                    match e {
                        Entry::Road(family) => {
                            let r = tile(ui, family, "R", Color32::from_rgb(70, 72, 78), app.line.family == *family, None);
                            if r.clicked() {
                                app.line.family = family.clone();
                                app.line.action = LineAction::Road;
                                app.mode = Mode::Lines;
                            }
                            r.context_menu(|ui| { if ui.button(t("Road catalog...")).clicked() { app.line.family = family.clone(); app.dialog = Dialog::RoadCatalog; ui.close_menu(); } });
                        }
                        Entry::Preset(id, name, first) => {
                            let image = first.as_ref().and_then(|m| app.thumbs.get(m).map(|t| t.id()));
                            let r = tile(ui, name, "O", Color32::from_rgb(64, 78, 92), app.edit_object == Some(*id), image);
                            if r.clicked() {
                                app.edit_object = Some(*id);
                                app.selected_generator = None;
                                app.edit_generator = None;
                                app.place.preset = Some(*id);
                                if app.mode == Mode::Select { app.mode = Mode::Place; }
                            }
                            if let Some(p) = r.dnd_release_payload::<String>() {
                                if let Some(preset) = app.library.objects.iter_mut().find(|x| x.id == *id) {
                                    if !preset.items.iter().any(|i| i.0 == *p) { preset.items.push(((*p).clone(), 1.0)); app.lib_dirty = true; }
                                }
                            }
                            r.dnd_set_drag_payload(LibDrag::ObjectPreset(*id));
                            r.context_menu(|ui| { if ui.button(t("Delete")).clicked() { delete = Some(*id); ui.close_menu(); } });
                        }
                    }
                }
            });
        }
    });
    if let Some(id) = delete {
        app.library.objects.retain(|p| p.id != id);
        if app.edit_object == Some(id) { app.edit_object = None; }
        if app.place.preset == Some(id) { app.place.preset = None; }
        app.lib_dirty = true;
    }
}

/// Shows the library entry or project generator being edited. Returns false when there is none.
pub fn details(app: &mut App, ui: &mut egui::Ui) -> bool {
    if let Some(id) = app.selected_generator {
        instance_details(app, ui, id);
        return true;
    }
    if let Some(id) = app.edit_object {
        object_preset_details(app, ui, id);
        return true;
    }
    if let Some(id) = app.edit_generator {
        generator_preset_details(app, ui, id);
        return true;
    }
    false
}

fn object_preset_details(app: &mut App, ui: &mut egui::Ui, id: u64) {
    let selected = app.place.model.clone();
    let Some(p) = app.library.objects.iter_mut().find(|p| p.id == id) else { app.edit_object = None; return };
    let before = p.clone();
    row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut p.name); });
    let mut remove = None;
    let list = ui.vertical(|ui| {
        for (i, (m, w)) in p.items.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(w).speed(0.05).range(0.01..=100.0));
                ui.label(m.rsplit('\\').next().unwrap_or(m)).on_hover_text(m.as_str());
                if ui.small_button("✖").clicked() { remove = Some(i); }
            });
        }
    }).response;
    if let Some(m) = list.dnd_release_payload::<String>() { if !p.items.iter().any(|i| i.0 == *m) { p.items.push(((*m).clone(), 1.0)); } }
    if let Some(i) = remove { p.items.remove(i); }
    if ui.button(format!("+ {}", t("Add selected model"))).on_hover_text(t("Models can also be dropped here.")).clicked() {
        if let Some(m) = selected { if !p.items.iter().any(|i| i.0 == m) { p.items.push((m, 1.0)); } }
    }
    number(ui, t("Min scale"), &mut p.scale_min, 0.1..=4.0);
    number(ui, t("Max scale"), &mut p.scale_max, 0.1..=4.0);
    if p.scale_max < p.scale_min { p.scale_max = p.scale_min; }
    ui.checkbox(&mut p.random_yaw, t("Random rotation"));
    ui.checkbox(&mut p.align, t("Align to slope"));
    if *p != before { app.lib_dirty = true; }
}

fn generator_preset_details(app: &mut App, ui: &mut egui::Ui, id: u64) {
    if app.built_in.iter().any(|g| g.id == id) {
        let name = app.built_in.iter().find(|g| g.id == id).map(|g| t(&g.name)).unwrap_or_default();
        ui.strong(name);
        ui.weak(t("A built-in generator. Duplicate it to make your own version, or drop it on the map."));
        if ui.button(t("Add to the project")).clicked() { add_instance(app, id, None); }
        return;
    }
    let user = app.library.generators.iter().find(|g| g.id == id).and_then(|g| if let GeneratorKind::Forest(f) = &g.kind { Some(f.user_presets.clone()) } else { None }).unwrap_or_default();
    let cx = context(app, &user);
    let Some(g) = app.library.generators.iter_mut().find(|g| g.id == id) else { app.edit_generator = None; return };
    row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut g.name); });
    ui.weak(t(g.kind.type_name()));
    let sample = kind_ui(ui, &mut g.kind, &cx);
    if sample { app.sampling = Some(id); }
    let released = !ui.ctx().input(|i| i.pointer.any_down());
    app.lib_dirty |= released;
    if ui.button(t("Add to the project")).clicked() { add_instance(app, id, None); }
}

fn instance_details(app: &mut App, ui: &mut egui::Ui, id: u64) {
    let Some(doc) = &app.doc else { return };
    let Some(index) = doc.world.generators.iter().position(|g| g.id == id) else { app.selected_generator = None; return };
    let user = if let GeneratorKind::Forest(f) = &doc.world.generators[index].kind { f.user_presets.clone() } else { Vec::new() };
    let cx = context(app, &user);
    let shape_layers: Vec<String> = doc.world.shape_layers.iter().map(|l| l.name.clone()).collect();
    let selected_shape = app.shape.selection.iter().next().copied().and_then(|(l, s)| Some((doc.world.shape_layers.get(l)?.name.clone(), s)));
    let doc = app.doc.as_mut().unwrap();
    let g = &mut doc.world.generators[index];
    row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut g.name); });
    ui.weak(t(g.kind.type_name()));
    ui.label(tf("{0} objects", &[&g.objects.len().to_string()]));
    match g.kind.target_kind() {
        TargetKind::Point => {
            if let GenTarget::Point(p) = &mut g.target {
                row(ui, "X", |ui| { ui.add(egui::DragValue::new(&mut p[0]).speed(1.0)); });
                row(ui, "Z", |ui| { ui.add(egui::DragValue::new(&mut p[1]).speed(1.0)); });
            }
        }
        kind => {
            let shown = match &g.target {
                GenTarget::Map => t("Whole map"),
                GenTarget::Layer(l) => l.clone(),
                GenTarget::Shape { layer, index } => format!("{layer} #{}", index + 1),
                GenTarget::Point(_) => String::new(),
            };
            row(ui, &t("Works on"), |ui| {
                egui::ComboBox::from_id_salt("target").selected_text(shown).show_ui(ui, |ui| {
                    if kind == TargetKind::Area { ui.selectable_value(&mut g.target, GenTarget::Map, t("Whole map")); }
                    for l in &shape_layers { ui.selectable_value(&mut g.target, GenTarget::Layer(l.clone()), l); }
                });
            });
            if ui.add_enabled(selected_shape.is_some(), egui::Button::new(t("Use selected shape"))).clicked() {
                if let Some((layer, index)) = selected_shape { g.target = GenTarget::Shape { layer, index }; }
            }
        }
    }
    ui.separator();
    if kind_ui(ui, &mut g.kind, &cx) { app.sampling = Some(id); }
    if let Some(d) = &mut app.doc { d.history.revision += ui.ctx().input(|i| u64::from(i.pointer.any_released())); }
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        if ui.button(t("Generate")).on_hover_text(t("Replaces what this generator made before.")).clicked() { run_generator(app, id); }
        instance_buttons(app, ui, id);
    });
}

fn instance_buttons(app: &mut App, ui: &mut egui::Ui, id: u64) {
    if ui.button(t("Select objects")).clicked() { select_objects(app, id); }
    if ui.button(t("Save as preset")).clicked() { save_as_preset(app, id); }
    if ui.button(t("Duplicate")).clicked() { duplicate_instance(app, id); }
    if ui.button(t("Delete generator")).on_hover_text(t("Removes the generator and what it made.")).clicked() { delete_instance(app, id, true); }
}

/// Rows of the generators in the outliner.
pub fn outliner_rows(app: &mut App, ui: &mut egui::Ui, rows: &[(u64, String, &'static str, usize)], filter: &str) {
    let mut action: Option<(u64, u8)> = None;
    for (id, name, kind, count) in rows.iter().filter(|r| r.1.to_ascii_lowercase().contains(filter)) {
        let row = ui.selectable_label(app.selected_generator == Some(*id), format!("{name}  [{}]  ({count})", t(kind)));
        if row.clicked() { action = Some((*id, 0)); }
        row.context_menu(|ui| {
            for (code, label) in [(1u8, "Generate"), (2, "Select objects"), (3, "Duplicate"), (4, "Save as preset"), (5, "Delete generator"), (6, "Delete generator, keep objects")] {
                if ui.button(t(label)).clicked() { action = Some((*id, code)); ui.close_menu(); }
            }
        });
    }
    if let Some((id, code)) = action {
        match code {
            0 => {
                app.selected_generator = Some(id);
                app.selected_modifier = None;
                app.selected_line = None;
            }
            1 => run_generator(app, id),
            2 => select_objects(app, id),
            3 => duplicate_instance(app, id),
            4 => save_as_preset(app, id),
            5 => delete_instance(app, id, true),
            _ => delete_instance(app, id, false),
        }
    }
}

fn select_objects(app: &mut App, id: u64) {
    let Some(g) = app.doc.as_ref().and_then(|d| d.world.generators.iter().find(|g| g.id == id)) else { return };
    app.selection = g.objects.iter().copied().collect();
    app.selection_rev += 1;
    app.mode = Mode::Select;
}

fn save_as_preset(app: &mut App, id: u64) {
    let Some(g) = app.doc.as_ref().and_then(|d| d.world.generators.iter().find(|g| g.id == id)).cloned() else { return };
    let new = new_id();
    app.library.generators.push(GeneratorPreset { id: new, name: g.name, folder: String::new(), kind: g.kind });
    app.lib_dirty = true;
    app.info(t("Saved in the generator library"));
}

fn duplicate_instance(app: &mut App, id: u64) {
    let Some(doc) = &mut app.doc else { return };
    let Some(mut g) = doc.world.generators.iter().find(|g| g.id == id).cloned() else { return };
    let before = doc.world.generators.clone();
    g.id = new_id();
    g.name = format!("{} 2", g.name);
    g.objects.clear();
    app.selected_generator = Some(g.id);
    doc.world.generators.push(g);
    doc.history.push(t("Duplicate"), Change::Generators(before));
}

fn delete_instance(app: &mut App, id: u64, with_objects: bool) {
    let Some(doc) = &mut app.doc else { return };
    let Some(g) = doc.world.generators.iter().find(|g| g.id == id).cloned() else { return };
    let before = doc.world.generators.clone();
    doc.world.generators.retain(|x| x.id != id);
    let label = t("Delete generator");
    doc.history.push(label.clone(), Change::Generators(before));
    if with_objects {
        let change: Vec<_> = g.objects.iter().filter_map(|id| doc.world.objects.remove(*id).map(|o| (*id, Some(o), None))).collect();
        doc.history.push(label.clone(), Change::Objects(change));
        doc.history.group_last(2, label);
        doc.dirty.objects = true;
        let doc = app.doc.as_ref().unwrap();
        app.selection.retain(|i| doc.world.objects.get(*i).is_some());
        app.selection_rev += 1;
    }
    if app.selected_generator == Some(id) { app.selected_generator = None; }
}

/// Puts a library generator into the project. It works on the shape it was dropped on, the
/// selected shape or, for what needs no shape, the whole map.
pub fn add_instance(app: &mut App, preset: u64, drop: Option<(Option<(usize, usize)>, [f32; 2])>) {
    let Some(p) = app.built_in.iter().chain(app.library.generators.iter()).find(|g| g.id == preset).cloned() else { return };
    let Some(doc) = &mut app.doc else { return };
    let shape_at = drop.and_then(|d| d.0).or_else(|| app.shape.selection.iter().next().copied());
    let wants = p.kind.target_kind();
    let usable = shape_at.and_then(|(l, s)| {
        let layer = doc.world.shape_layers.get(l)?;
        let shape = layer.shapes.get(s)?;
        let fits = match wants { TargetKind::Area => shape.kind == ShapeKind::Polygon, TargetKind::Line => shape.kind == ShapeKind::Polyline, TargetKind::Point => false };
        fits.then(|| GenTarget::Shape { layer: layer.name.clone(), index: s })
    });
    let target = match (wants, usable, drop) {
        (_, Some(t), _) => t,
        (TargetKind::Point, _, Some((_, p))) => GenTarget::Point(p),
        (TargetKind::Point, _, None) => { let s = doc.world.meta.size_m(); GenTarget::Point([s * 0.5, s * 0.5]) }
        (TargetKind::Area, _, _) => GenTarget::Map,
        (TargetKind::Line, _, _) => {
            app.warn(t("Drop this generator on a polyline, or select one first"));
            return;
        }
    };
    let before = doc.world.generators.clone();
    let id = new_id();
    let run = target != GenTarget::Map && !matches!(&p.kind, GeneratorKind::Settlement(s) if s.buildings.is_empty());
    let name = if p.folder == BUILT_IN { t(&p.name) } else { p.name.clone() };
    doc.world.generators.push(GeneratorInstance { id, name, kind: p.kind, target, objects: Vec::new() });
    doc.history.push(t("Add generator"), Change::Generators(before));
    app.selected_generator = Some(id);
    app.selected_modifier = None;
    app.selected_line = None;
    if run { run_generator(app, id); }
}

fn target_shapes(w: &World, target: &GenTarget, kind: ShapeKind) -> Vec<MapShape> {
    match target {
        GenTarget::Layer(name) => w.shape_layers.iter().find(|l| l.name == *name).map(|l| l.shapes.iter().filter(|s| s.kind == kind).cloned().collect()).unwrap_or_default(),
        GenTarget::Shape { layer, index } => w.shape_layers.iter().find(|l| l.name == *layer).and_then(|l| l.shapes.get(*index)).filter(|s| s.kind == kind).cloned().into_iter().collect(),
        _ => Vec::new(),
    }
}

/// Runs a generator; what it made before is replaced.
pub fn run_generator(app: &mut App, id: u64) {
    let Some(doc) = &app.doc else { return };
    let Some(g) = doc.world.generators.iter().find(|g| g.id == id).cloned() else { return };
    let label = g.name.clone();
    let mut lite = doc.world.lite();
    let db = app.db.clone();
    let assets = app.assets.clone();
    let world = &doc.world;
    let polygons = target_shapes(world, &g.target, ShapeKind::Polygon);
    let lines = target_shapes(world, &g.target, ShapeKind::Polyline);
    if g.target != GenTarget::Map && !matches!(g.target, GenTarget::Point(_)) && polygons.is_empty() && lines.is_empty() {
        app.warn(t("The shape this generator works on no longer exists"));
        return;
    }
    match g.kind {
        GeneratorKind::Forest(f) => {
            let zones: Vec<(usize, String)> = f.zones.iter().filter_map(|(l, p)| Some((world.layers.iter().position(|x| x.name == *l)?, p.clone()))).collect();
            let wants_filter = f.color_filter.as_ref().is_some_and(|c| !c.samples.is_empty());
            let mut shapes = polygons;
            if shapes.is_empty() && g.target == GenTarget::Map && (wants_filter || (zones.is_empty() && f.project.areas.is_empty())) {
                let s = world.meta.size_m();
                shapes.push(MapShape { kind: ShapeKind::Polygon, points: vec![[0.0, 0.0], [s, 0.0], [s, s], [0.0, s]], attributes: Vec::new() });
            }
            let areas = if shapes.is_empty() { Vec::new() } else { vec![(ShapeLayer { name: g.name.clone(), visible: true, shapes }, f.preset.clone())] };
            let satellite = world.satellite.as_ref().filter(|_| wants_filter || gen::forest_needs_satellite(&f.project)).map(gen::satellite_for_forest);
            let forest = *f;
            let req = gen::ForestRequest { project: forest.project, zones, areas, satellite, color_filter: forest.color_filter, edge_presets: forest.edge_presets, user_presets: forest.user_presets };
            app.spawn(label, move |job| {
                let (objects, notes) = gen::generate_forest(&mut lite, &assets, req, job)?;
                let models = model_list(&lite);
                Ok(Box::new(move |a: &mut App| {
                    store_generated(a, id, models, objects);
                    for n in notes { a.info(n); }
                }) as Finish)
            });
        }
        GeneratorKind::SurfacePaint { rules, seed } => {
            let index = (!polygons.is_empty()).then(|| dz_core::shapes::PolygonIndex::new(&ShapeLayer { name: String::new(), visible: true, shapes: polygons }));
            let name = label.clone();
            app.spawn(label, move |job| {
                let mut w = lite;
                w.mask.data = gen::auto_paint(&w, &rules, seed as u64, index.as_ref(), job);
                dz_core::mask_limit::enforce(&mut w);
                let data = w.mask.data;
                Ok(Box::new(move |a: &mut App| a.apply_mask(&name, data)) as Finish)
            });
        }
        GeneratorKind::Settlement(rule) => {
            let GenTarget::Point(p) = g.target else { return };
            let falloff = app.place.pad_falloff;
            app.spawn(label, move |_| {
                let s = gen::settlement(&mut lite, &db, p[0], p[1], &rule)?;
                let models = model_list(&lite);
                Ok(Box::new(move |a: &mut App| {
                    let ids = store_generated(a, id, models, s.objects);
                    let pads: Vec<u32> = s.pads.iter().map(|(i, _, _)| ids[*i]).collect();
                    a.add_pads(&pads, falloff);
                }) as Finish)
            });
        }
        GeneratorKind::Fence(f) => {
            app.spawn(label, move |_| {
                let mut objects = Vec::new();
                for l in &lines { objects.extend(gen::fence_objects(&mut lite, &db, &l.points, &f)); }
                let models = model_list(&lite);
                Ok(Box::new(move |a: &mut App| { store_generated(a, id, models, objects); }) as Finish)
            });
        }
        GeneratorKind::PowerLine(f) => {
            app.spawn(label, move |_| {
                let mut objects = Vec::new();
                for l in &lines { objects.extend(gen::power_objects(&mut lite, &db, &l.points, &f)); }
                let models = model_list(&lite);
                Ok(Box::new(move |a: &mut App| { store_generated(a, id, models, objects); }) as Finish)
            });
        }
    }
}

fn model_list(w: &World) -> Vec<String> {
    w.objects.models.iter().map(|m| m.path.clone()).collect()
}

/// Puts the new result of a generator in place of its previous one, as one undo step.
fn store_generated(app: &mut App, id: u64, models: Vec<String>, objects: Vec<Obj>) -> Vec<u32> {
    let Some(doc) = &mut app.doc else { return Vec::new() };
    let Some(index) = doc.world.generators.iter().position(|g| g.id == id) else { return Vec::new() };
    let before = doc.world.generators.clone();
    let old = doc.world.generators[index].objects.clone();
    let layer = old.first().and_then(|i| doc.world.objects.get(*i)).map(|o| o.layer()).unwrap_or_else(|| doc.world.target_layer());
    let mut change = Vec::with_capacity(old.len() + objects.len());
    for i in &old {
        if let Some(o) = doc.world.objects.remove(*i) { change.push((*i, Some(o), None)); }
    }
    let map: Vec<u32> = models.iter().map(|m| doc.world.objects.intern(m)).collect();
    let mut ids = Vec::with_capacity(objects.len());
    for mut o in objects {
        o.model = map[o.model as usize];
        o.set_layer(layer);
        let new = doc.world.objects.insert(o);
        change.push((new, None, Some(o)));
        ids.push(new);
    }
    doc.world.generators[index].objects = ids.clone();
    let label = doc.world.generators[index].name.clone();
    doc.history.push(label.clone(), Change::Objects(change));
    doc.history.push(label.clone(), Change::Generators(before));
    let had_pads = doc.world.modifiers.iter().any(|m| m.attached.is_some_and(|a| old.contains(&a)));
    let mut parts = 2;
    if had_pads {
        let modifiers = doc.world.modifiers.clone();
        doc.world.modifiers.retain(|m| !m.attached.is_some_and(|a| old.contains(&a)));
        doc.world.recompose(None);
        doc.history.push(label.clone(), Change::Modifiers(modifiers));
        parts = 3;
    }
    doc.history.group_last(parts, label);
    doc.dirty.objects = true;
    let doc = app.doc.as_ref().unwrap();
    app.selection.retain(|i| doc.world.objects.get(*i).is_some());
    app.selection_rev += 1;
    if had_pads { app.whole_height_changed(); }
    ids
}

/// The eyedropper for satellite colours.
pub fn take_sample(app: &mut App, x: f32, z: f32) {
    let Some(id) = app.sampling.take() else { return };
    let Some(doc) = &mut app.doc else { return };
    let size = doc.world.meta.size_m();
    let Some(img) = &doc.world.satellite else { return };
    let (px, py) = (((x / size * img.width as f32) as usize).min(img.width - 1), (((1.0 - z / size) * img.height as f32) as usize).min(img.height - 1));
    let at = (py * img.width + px) * 3;
    let sample = forest_core::mask::Rgb8([img.data[at], img.data[at + 1], img.data[at + 2]]);
    let kind = doc.world.generators.iter_mut().find(|g| g.id == id).map(|g| &mut g.kind).or_else(|| app.library.generators.iter_mut().find(|g| g.id == id).map(|g| &mut g.kind));
    if let Some(GeneratorKind::Forest(f)) = kind {
        if let Some(c) = &mut f.color_filter { c.samples.push(sample); }
    }
    app.lib_dirty = true;
}

/// Places one model of an object preset.
pub fn place_preset(app: &mut App, id: u64, x: f32, z: f32) {
    let Some(p) = app.library.objects.iter().find(|p| p.id == id).cloned() else { return };
    let total: f32 = p.items.iter().map(|i| i.1).sum();
    if p.items.is_empty() || total <= 0.0 {
        app.warn(t("This preset has no models yet"));
        return;
    }
    let seed = (x.to_bits() ^ z.to_bits().rotate_left(13)).wrapping_mul(2654435761);
    let mut pick = ((seed >> 8) as f32 / (u32::MAX >> 8) as f32) * total;
    let mut model = p.items[0].0.clone();
    for (m, w) in &p.items {
        model = m.clone();
        pick -= w;
        if pick <= 0.0 { break; }
    }
    let saved = (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max);
    (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max) = (p.random_yaw, p.align, p.scale_min, p.scale_max);
    crate::viewport::place_model(app, &model, x, z);
    (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max) = saved;
}

/// Editing which models a road type is built from; saved in the Terrain Builder project.
pub fn road_catalog_window(app: &mut App, ctx: &egui::Context, open: &mut bool) {
    egui::Window::new(t("Road catalog")).collapsible(false).resizable(true).default_width(520.0).open(open).show(ctx, |ui| {
        let families = app.road_families.clone();
        row(ui, &t("Road type"), |ui| {
            egui::ComboBox::from_id_salt("cat_family").selected_text(app.line.family.clone()).show_ui(ui, |ui| {
                for f in families { ui.selectable_value(&mut app.line.family, f.clone(), f); }
            });
        });
        let family = app.line.family.clone();
        let mut remove: Option<String> = None;
        let mut dropped: Option<(u32, String)> = None;
        let mut add: Option<u32> = None;
        egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
            for (category, name) in [(3u32, "Straight parts"), (4, "Curves"), (6, "Road ends")] {
                let frame = egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.strong(t(name));
                        if ui.small_button(format!("+ {}", t("Add selected model"))).clicked() { add = Some(category); }
                    });
                    for c in app.road_catalog.iter().filter(|c| c.family == family && c.category == category) {
                        ui.horizontal(|ui| {
                            if ui.small_button("✖").clicked() { remove = Some(c.path.clone()); }
                            ui.label(c.path.rsplit(['\\', '/']).next().unwrap_or(&c.path));
                        });
                    }
                }).response;
                if let Some(p) = frame.dnd_release_payload::<String>() { dropped = Some((category, (*p).clone())); }
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            let base = app.doc.as_ref().and_then(|d| d.world.road_project.clone());
            ui.add_enabled_ui(base.is_some(), |ui| {
                if ui.button(t("Save to TV4P...")).clicked() { if let Some(b) = base.clone() { save_catalog(app, b); } }
                if ui.button(t("Reload from TV4P")).clicked() { if let Some(b) = base { app.load_road_types(b); } }
            });
        });
        if let (Some(c), Some(m)) = (add, app.place.model.clone()) { dropped = Some((c, m)); }
        if let Some(path) = remove {
            let file = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_ascii_lowercase();
            if let Some(d) = &mut app.doc {
                d.world.road_added.retain(|a| a.path != path);
                d.world.road_removed.insert(file);
                d.history.revision += 1;
                let (removed, added) = (d.world.road_removed.clone(), d.world.road_added.clone());
                let mut catalog = (*app.road_catalog).clone();
                gen::apply_catalog_edits(&mut catalog, &removed, &added, std::path::Path::new(ROADS));
                app.road_catalog = Arc::new(catalog);
            }
        }
        if let Some((category, path)) = dropped {
            let full = format!("P:\\{path}");
            let file = path.rsplit('\\').next().unwrap_or(&path).to_ascii_lowercase();
            if let Some(d) = &mut app.doc {
                d.world.road_removed.remove(&file);
                d.world.road_added.push(dz_core::world::RoadPart { family, category, path: full });
                d.history.revision += 1;
                let (removed, added) = (d.world.road_removed.clone(), d.world.road_added.clone());
                let mut catalog = (*app.road_catalog).clone();
                gen::apply_catalog_edits(&mut catalog, &removed, &added, std::path::Path::new(ROADS));
                app.road_catalog = Arc::new(catalog);
            }
        }
    });
}

const ROADS: &str = "P:\\dz\\structures\\roads\\parts";

fn save_catalog(app: &mut App, base: PathBuf) {
    use dayz_road_tool::tv4p::{write_definitions, DefinitionEdit};
    let Some(out) = rfd::FileDialog::new().add_filter("Terrain Builder", &["tv4p"]).set_file_name(base.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).save_file() else { return };
    let mut edits: Vec<DefinitionEdit> = Vec::new();
    for c in app.road_catalog.iter().filter(|c| c.road_type_index != u32::MAX) {
        match edits.iter_mut().find(|e| e.road_type_index == c.road_type_index && e.category == c.category) {
            Some(e) => e.paths.push(c.path.clone()),
            None => edits.push(DefinitionEdit { road_type_index: c.road_type_index, category: c.category, paths: vec![c.path.clone()] }),
        }
    }
    let same = dayz_road_tool::tv4p::same_path(&base, &out);
    let target = if same { out.with_extension("tv4p.new") } else { out.clone() };
    let result = write_definitions(&base, &target, &edits).and_then(|_| {
        if same {
            std::fs::copy(&out, out.with_extension("tv4p.bak"))?;
            std::fs::rename(&target, &out)?;
        }
        Ok(())
    });
    match result {
        Ok(()) => app.info(t("Road types saved")),
        Err(e) => app.error(format!("{e:#}")),
    }
}

/// Files dropped on the window from outside.
pub fn handle_file_drop(app: &mut App, ctx: &egui::Context) {
    let files: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
    let mut shapes = Vec::new();
    for p in files {
        let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "shp" | "tv4s" if app.doc.is_some() => shapes.push(p),
            "tv4p" => app.scan_for_import(p),
            "wrp" => app.import_wrp_file(p),
            "dzwe" => app.open_project(dz_core::project::project_dir_of(&p)),
            _ if p.is_dir() => {
                if dz_core::project::is_project(&p) { app.open_project(p); } else { app.scan_for_import(p); }
            }
            _ => {}
        }
    }
    if !shapes.is_empty() { app.load_shape_files(shapes); }
}
