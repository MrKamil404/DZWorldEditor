//! The generator and object preset libraries, generators in the project and the road catalog.
use crate::app::*;
use crate::i18n::{t, tf};
use crate::theme::{self, row};
use dz_core::edit::Change;
use dz_core::gen;
use dz_core::library::*;
use dz_core::objects::{Obj, ObjectStore};
use dz_core::shapes::{MapShape, ShapeKind, ShapeLayer};
use dz_core::world::World;
use eframe::egui::{self, vec2, Color32};
use std::path::PathBuf;
use std::sync::Arc;

/// What is being dragged out of a library.
#[derive(Clone, Debug, PartialEq)]
pub enum LibDrag {
    Generator(u64),
    ObjectPreset(u64),
    ForestPreset(String),
}

/// What was under the cursor when something was dropped on the map.
#[derive(Clone, Copy, Debug)]
pub struct Dropped {
    pub shape: Option<(usize, usize)>,
    pub route: Option<u64>,
    pub at: [f32; 2],
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
    object_presets: Vec<(u64, String)>,
    /// Presets read from files while drawing, kept for the library.
    loaded: std::cell::RefCell<Vec<forest_core::user_presets::UserPreset>>,
}

fn context(app: &App, user: &[forest_core::user_presets::UserPreset]) -> KindContext {
    let doc = app.doc.as_ref();
    let mut presets: Vec<String> = user.iter().map(|p| p.name.clone()).collect();
    presets.extend(app.library.all_forest_presets().into_iter().map(|p| p.name));
    presets.extend(gen::forest_preset_names());
    let mut seen = std::collections::HashSet::new();
    presets.retain(|n| seen.insert(n.clone()));
    KindContext {
        layers: doc.map(|d| d.world.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default(),
        presets,
        model: app.place.model.clone(),
        has_satellite: doc.is_some_and(|d| d.world.satellite.is_some()),
        object_presets: app.library.objects.iter().map(|p| (p.id, p.name.clone())).collect(),
        loaded: Default::default(),
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

/// Chooses an object preset in place of a list of models.
fn preset_pick(ui: &mut egui::Ui, value: &mut Option<u64>, cx: &KindContext) {
    row(ui, &t("Object preset"), |ui| {
        let shown = value.and_then(|id| cx.object_presets.iter().find(|p| p.0 == id)).map(|p| p.1.clone()).unwrap_or_else(|| t("(none)"));
        egui::ComboBox::from_id_salt("objects_preset").selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(value, None, t("(none)"));
            for (id, name) in &cx.object_presets { ui.selectable_value(value, Some(*id), name); }
        });
    });
}

fn models_list(ui: &mut egui::Ui, models: &mut Vec<(String, f32)>, cx: &KindContext) {
    let mut remove = None;
    let list = ui.vertical(|ui| {
        for (i, (m, k)) in models.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(k).speed(0.05).range(0.01..=100.0));
                ui.label(m.rsplit('\\').next().unwrap_or(m)).on_hover_text(m.as_str());
                if ui.small_button("✖").clicked() { remove = Some(i); }
            });
        }
    }).response;
    if let Some(p) = list.dnd_release_payload::<String>() { if !models.iter().any(|m| m.0 == *p) { models.push(((*p).clone(), 1.0)); } }
    if let Some(i) = remove { models.remove(i); }
    if ui.button(format!("+ {}", t("Add selected model"))).on_hover_text(t("Models can also be dropped here.")).clicked() {
        if let Some(m) = &cx.model { if !models.iter().any(|x| x.0 == *m) { models.push((m.clone(), 1.0)); } }
    }
}

fn objects_ui(ui: &mut egui::Ui, set: &mut ObjectSet, cx: &KindContext) {
    preset_pick(ui, &mut set.preset, cx);
    if set.preset.is_some() {
        return;
    }
    models_list(ui, &mut set.models, cx);
    number(ui, t("Min scale"), &mut set.scale_min, 0.1..=4.0);
    number(ui, t("Max scale"), &mut set.scale_max, 0.1..=4.0);
    if set.scale_max < set.scale_min { set.scale_max = set.scale_min; }
    ui.checkbox(&mut set.random_yaw, t("Random rotation"));
    ui.checkbox(&mut set.align, t("Align to slope"));
}

fn count_row(ui: &mut egui::Ui, label: String, v: &mut u32, range: std::ops::RangeInclusive<u32>) {
    row(ui, &label, |ui| { ui.add(egui::Slider::new(v, range)); });
}

const POWERLINES: &str = "P:\\samples\\DayZ-Misc-master\\Powerlines";

/// A margin that can be switched off.
fn margin_field(ui: &mut egui::Ui, label: String, value: &mut Option<f32>) {
    ui.horizontal(|ui| {
        let mut on = value.is_some();
        if ui.checkbox(&mut on, label).changed() { *value = on.then_some(10.0); }
        if let Some(v) = value { ui.add(egui::DragValue::new(v).speed(0.5).range(0.0..=200.0).suffix(" m")); }
    });
}

/// How a generator follows its line.
fn line_ui(ui: &mut egui::Ui, o: &mut LineOptions) {
    ui.horizontal(|ui| {
        for (mode, name) in [(LineMode::Polyline, "Along the line"), (LineMode::Bezier, "Bezier curve")] {
            if crate::theme::tool_button(ui, o.mode == mode, &t(name)).clicked() { o.mode = mode; }
        }
    });
    number(ui, t("Distance from the line"), &mut o.offset, -60.0..=60.0);
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
                        if let Ok(l) = forest_core::user_presets::UserPresetLibrary::load(&f) { cx.loaded.borrow_mut().extend(l.presets.iter().cloned()); g.user_presets = l.presets; }
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
            ui.strong(t("More presets in shapes"));
            ui.weak(t("Each one fills the whole shape as well, for example undergrowth under the trees."));
            let mut remove = None;
            for (i, l) in g.layers.iter_mut().enumerate() {
                ui.push_id(("layer", i), |ui| {
                    ui.horizontal(|ui| {
                        preset_combo(ui, "preset", &mut l.preset, &cx.presets);
                        if ui.small_button("✖").clicked() { remove = Some(i); }
                    });
                    number(ui, t("Density per ha (0 = from the preset)"), &mut l.density_per_ha, 0.0..=5000.0);
                    row(ui, &t("Seed"), |ui| { ui.add(egui::DragValue::new(&mut l.seed)); });
                    number64(ui, t("Spacing"), &mut l.spacing, 0.3..=4.0);
                    number(ui, t("Min scale"), &mut l.scale_min, 0.1..=3.0);
                    number(ui, t("Max scale"), &mut l.scale_max, 0.1..=3.0);
                    number64(ui, t("Clearings"), &mut l.clearing_strength, 0.0..=0.95);
                    number64(ui, t("Clearing size m"), &mut l.clearing_scale_m, 0.0..=1500.0);
                    number64(ui, t("Max slope"), &mut l.max_slope_deg, 5.0..=90.0);
                    number64(ui, t("Edge padding m"), &mut l.edge_padding_m, 0.0..=200.0);
                    number(ui, t("Distance from earlier presets m"), &mut l.gap, 0.0..=20.0);
                    ui.add_space(4.0);
                });
            }
            if let Some(i) = remove { g.layers.remove(i); }
            if ui.button(format!("+ {}", t("Add preset"))).clicked() {
                g.layers.push(dz_core::library::ForestLayer { preset: cx.presets.first().cloned().unwrap_or_default(), ..Default::default() });
            }
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
                let mut outside = g.project.edges.outside_m.unwrap_or(0.0);
                number64(ui, t("Beyond the outline m"), &mut outside, 0.0..=80.0);
                g.project.edges.outside_m = Some(outside);
                number64(ui, t("Edge patch size m"), &mut g.project.edges.patch_size_m, 0.0..=200.0);
                if g.project.edges.patch_size_m > 0.0 {
                    number64(ui, t("Patch coverage"), &mut g.project.edges.patch_cover, 0.05..=1.0);
                } else {
                    ui.weak(t("0 makes the edge an even band."));
                }
                let from_project = t("From the project species");
                let mut main = g.edge_presets.first().map(|e| e.0.clone()).unwrap_or_default();
                let chosen = main.clone();
                row(ui, &t("Edge preset"), |ui| {
                    egui::ComboBox::from_id_salt("edge main").width(170.0).selected_text(if main.is_empty() { from_project.clone() } else { main.clone() }).show_ui(ui, |ui| {
                        ui.selectable_value(&mut main, String::new(), from_project.as_str());
                        for p in &cx.presets { ui.selectable_value(&mut main, p.clone(), p); }
                    });
                });
                if main != chosen {
                    match (main.is_empty(), g.edge_presets.first_mut()) {
                        (true, _) => g.edge_presets.clear(),
                        (false, Some(e)) => e.0 = main,
                        (false, None) => g.edge_presets.push((main, 1.0)),
                    }
                }
                if g.edge_presets.len() > 1 { ui.weak(t("Further edge presets, with their share")); }
                let mut remove = None;
                for (i, e) in g.edge_presets.iter_mut().enumerate().skip(1) {
                    ui.push_id(("edge", i), |ui| ui.horizontal(|ui| {
                        preset_combo(ui, "preset", &mut e.0, &cx.presets);
                        ui.add(egui::DragValue::new(&mut e.1).speed(0.01).range(0.01..=1.0));
                        if ui.small_button("✖").clicked() { remove = Some(i); }
                    }));
                }
                if let Some(i) = remove { g.edge_presets.remove(i); }
                if !g.edge_presets.is_empty() && ui.button(format!("+ {}", t("Add edge preset"))).clicked() {
                    g.edge_presets.push((cx.presets.first().cloned().unwrap_or_default(), 0.5));
                }
            }
            ui.separator();
            ui.strong(t("Keep free of trees"));
            let mut remove = None;
            for (i, s) in g.skip_surfaces.iter_mut().enumerate() {
                ui.push_id(("skip", i), |ui| ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("layer").width(120.0).selected_text(s.0.clone()).show_ui(ui, |ui| {
                        for n in &cx.layers { ui.selectable_value(&mut s.0, n.clone(), n); }
                    });
                    ui.add(egui::DragValue::new(&mut s.1).speed(0.5).range(0.0..=200.0).suffix(" m"));
                    if ui.small_button("✖").clicked() { remove = Some(i); }
                }));
            }
            if let Some(i) = remove { g.skip_surfaces.remove(i); }
            if ui.button(format!("+ {}", t("Add surface"))).clicked() {
                g.skip_surfaces.push((cx.layers.first().cloned().unwrap_or_default(), 10.0));
            }
            margin_field(ui, t("Roads"), &mut g.avoid_roads);
            margin_field(ui, t("Buildings"), &mut g.avoid_buildings);
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
            preset_pick(ui, &mut g.preset, cx);
            if g.preset.is_none() { model_field(ui, t("Model"), &mut g.model, &cx.model); }
            number(ui, t("Spacing"), &mut g.spacing, -30.0..=100.0);
            ui.weak(t("Gap between the pieces. 0 makes them touch, negative values overlap them."));
            number(ui, t("Turn"), &mut g.yaw_offset, -180.0..=180.0);
            ui.checkbox(&mut g.random_rotation, t("Random rotation"));
            ui.checkbox(&mut g.follow_slope, t("Follow the slope"));
            ui.checkbox(&mut g.join_corners, t("Join the pieces at corners"));
            model_field(ui, t("First object"), &mut g.start_model, &cx.model);
            if !g.start_model.is_empty() { ui.checkbox(&mut g.flip_start, t("Rotate the first object by 180°")); }
            model_field(ui, t("Last object"), &mut g.end_model, &cx.model);
            if !g.end_model.is_empty() { ui.checkbox(&mut g.flip_end, t("Rotate the last object by 180°")); }
            line_ui(ui, &mut g.line);
        }
        GeneratorKind::Area(g) => {
            objects_ui(ui, &mut g.objects, cx);
            match g.method {
                AreaMethod::HighDensity => {
                    number(ui, t("Per hectare"), &mut g.density, 1.0..=2000.0);
                    number(ui, t("Patches per hectare"), &mut g.fraction, 0.05..=50.0);
                    number(ui, t("Patch blending m"), &mut g.blend, 0.0..=100.0);
                }
                AreaMethod::LocalMaximum | AreaMethod::LocalMinimum => {
                    number(ui, t("Search radius"), &mut g.search_radius, 8.0..=300.0);
                }
                _ => number(ui, t("Clusters per hectare"), &mut g.cluster_density, 0.05..=100.0),
            }
            if g.method != AreaMethod::HighDensity {
                count_row(ui, t("Objects in a cluster"), &mut g.cluster_max, 1..=200);
                number(ui, t("Min radius"), &mut g.min_radius, 0.5..=100.0);
                number(ui, t("Max radius"), &mut g.max_radius, 0.5..=200.0);
                if g.max_radius < g.min_radius { g.max_radius = g.min_radius; }
            }
            if g.method == AreaMethod::LowDensityStrip { number(ui, t("Strip width"), &mut g.strip_width, 0.5..=60.0); }
            number(ui, t("Minimum distance"), &mut g.min_distance, 0.25..=60.0);
            number(ui, t("Max slope"), &mut g.max_slope, 0.0..=90.0);
            row(ui, &t("Seed"), |ui| { ui.add(egui::DragValue::new(&mut g.seed)); });
        }
        GeneratorKind::Line(g) => {
            objects_ui(ui, &mut g.objects, cx);
            match g.method {
                LineMethod::Random => number(ui, t("Per hectare"), &mut g.density, 1.0..=2000.0),
                LineMethod::SparseCluster => {
                    number(ui, t("Clusters per hectare"), &mut g.cluster_density, 0.05..=200.0);
                    count_row(ui, t("Objects in a cluster"), &mut g.cluster_max, 1..=200);
                    number(ui, t("Min radius"), &mut g.min_radius, 0.5..=100.0);
                    number(ui, t("Max radius"), &mut g.max_radius, 0.5..=200.0);
                    if g.max_radius < g.min_radius { g.max_radius = g.min_radius; }
                }
                LineMethod::Nodes => number(ui, t("Chance per node %"), &mut g.probability, 0.0..=100.0),
            }
            if g.method != LineMethod::Nodes {
                number(ui, t("Nearest to the line"), &mut g.inner_distance, 0.0..=200.0);
                number(ui, t("Furthest from the line"), &mut g.max_distance, 0.0..=300.0);
                if g.max_distance < g.inner_distance { g.max_distance = g.inner_distance; }
            }
            if g.method != LineMethod::SparseCluster { number(ui, t("Turn"), &mut g.rotation, -180.0..=180.0); }
            number(ui, t("Minimum distance"), &mut g.min_distance, 0.25..=60.0);
            row(ui, &t("Seed"), |ui| { ui.add(egui::DragValue::new(&mut g.seed)); });
            line_ui(ui, &mut g.line);
        }
        GeneratorKind::PowerLine(g) => {
            row(ui, &t("Configuration"), |ui| {
                let files: Vec<PathBuf> = std::fs::read_dir(POWERLINES).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xml"))).collect()).unwrap_or_default();
                let mut chosen = None;
                egui::ComboBox::from_id_salt("power").selected_text(g.config.clone()).show_ui(ui, |ui| {
                    for p in &files {
                        let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                        if ui.selectable_label(g.config == name, &name).clicked() { chosen = Some(p.clone()); }
                    }
                });
                if ui.small_button("…").on_hover_text(t("Terrain Processor configuration (XML)")).clicked() {
                    chosen = rfd::FileDialog::new().add_filter("XML", &["xml"]).pick_file();
                }
                if let Some(p) = chosen {
                    let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    if let Ok(text) = std::fs::read(&p) {
                        if let Ok(parsed) = parse_powerlines(&String::from_utf8_lossy(&text), &name) {
                            let (spacing, line) = (g.spacing, g.line);
                            *g = parsed;
                            (g.spacing, g.line) = (spacing, line);
                        }
                    }
                }
            });
            ui.weak(tf("{0} pole types, {1} wire types", &[&g.poles.len().to_string(), &g.wires.len().to_string()]));
            number(ui, t("Spacing"), &mut g.spacing, 0.0..=300.0);
            ui.weak(t("Spacing 0 uses the length of the wire model."));
            line_ui(ui, &mut g.line);
        }
        GeneratorKind::Scatter(s) => {
            preset_pick(ui, &mut s.preset, cx);
            if s.preset.is_none() { models_list(ui, &mut s.models, cx); }
            number(ui, t("Per hectare"), &mut s.density, 0.1..=600.0);
            number(ui, t("Minimum distance"), &mut s.min_distance, 0.5..=100.0);
            number(ui, t("Min slope"), &mut s.min_slope, 0.0..=90.0);
            number(ui, t("Max slope"), &mut s.max_slope, 0.0..=90.0);
            number(ui, t("Min height"), &mut s.min_height, -200.0..=1500.0);
            number(ui, t("Max height"), &mut s.max_height, -200.0..=9000.0);
            number(ui, t("Min scale"), &mut s.scale_min, 0.1..=4.0);
            number(ui, t("Max scale"), &mut s.scale_max, 0.1..=4.0);
            if s.scale_max < s.scale_min { s.scale_max = s.scale_min; }
            ui.checkbox(&mut s.align_to_slope, t("Align to slope"));
            row(ui, &t("Seed"), |ui| { ui.add(egui::DragValue::new(&mut s.seed)); });
            ui.collapsing(t("Only on surfaces"), |ui| {
                for n in &cx.layers {
                    let mut on = s.layers.contains(n);
                    if ui.checkbox(&mut on, n).changed() {
                        if on { s.layers.push(n.clone()); } else { s.layers.retain(|l| l != n); }
                    }
                }
            });
        }
        GeneratorKind::River(g) => {
            number(ui, t("Width"), &mut g.width, 2.0..=120.0);
            ui.checkbox(&mut g.modify_terrain, t("Modify the terrain"));
            ui.add_enabled_ui(g.modify_terrain, |ui| {
                number(ui, t("Depth"), &mut g.depth, 0.3..=20.0);
                number(ui, t("Blend"), &mut g.falloff, 0.0..=80.0);
            });
            model_field(ui, t("Water model"), &mut g.model, &cx.model);
            number(ui, t("Overlap of the water models m"), &mut g.join_offset, -6.0..=12.0);
            number(ui, t("Water height m"), &mut g.water_offset, -5.0..=5.0);
            ui.checkbox(&mut g.reverse_flow, t("Current runs the other way"));
            ui.weak(t("The river runs from the first point of the line to the last."));
            ui.weak(t("Bends of the same family are used where the line turns. The bed and its blend stay within the width of the water."));
            line_ui(ui, &mut g.line);
        }
        GeneratorKind::LineTerrain(g) => {
            crate::panels::grading_ui(ui, &mut g.grading, &mut g.width, &mut g.falloff, false);
            line_ui(ui, &mut g.line);
        }
        GeneratorKind::Pond(g) => {
            model_field(ui, t("Water model"), &mut g.model, &cx.model);
            ui.checkbox(&mut g.fixed, t("Fixed water level"));
            if g.fixed {
                number(ui, t("Water level m"), &mut g.level, -200.0..=2000.0);
            } else {
                ui.weak(t("The water stands at the lowest ground around the shape."));
            }
            number(ui, t("Water height m"), &mut g.offset, -20.0..=20.0);
            ui.checkbox(&mut g.modify_terrain, t("Modify the terrain"));
            ui.add_enabled_ui(g.modify_terrain, |ui| {
                number(ui, t("Depth"), &mut g.depth, 0.3..=30.0);
                number(ui, t("Blend"), &mut g.blend, 0.0..=80.0);
                number(ui, t("Bank width m"), &mut g.bank, 0.0..=100.0);
            });
            ui.weak(t("Models are laid side by side until they cover the whole shape."));
        }
        GeneratorKind::Remove(g) => {
            ui.strong(t("What to remove"));
            ui.checkbox(&mut g.trees, t("Trees"));
            ui.checkbox(&mut g.bushes, t("Bushes"));
            ui.checkbox(&mut g.rocks, t("Rocks"));
            ui.checkbox(&mut g.buildings, t("Buildings and structures"));
            ui.checkbox(&mut g.other, t("Other objects"));
            row(ui, &t("Only these names"), |ui| { ui.text_edit_singleline(&mut g.names); });
            ui.weak(t("Parts of model names, separated by commas. When filled in, they replace the categories above."));
            row(ui, &t("Always keep"), |ui| { ui.text_edit_singleline(&mut g.keep); });
            number(ui, t("Margin m"), &mut g.margin, 0.0..=500.0);
            ui.weak(t("How far beyond a polygon, or to each side of a line."));
            number(ui, t("Part removed"), &mut g.share, 0.0..=1.0);
            row(ui, &t("Seed"), |ui| { ui.add(egui::DragValue::new(&mut g.seed)); });
            ui.weak(t("Objects of roads and generators stay. Running again or deleting this generator with its objects brings the removed ones back."));
        }
        GeneratorKind::Terrain(g) => {
            row(ui, &t("Type"), |ui| {
                egui::ComboBox::from_id_salt("kind").selected_text(t(g.kind.name())).show_ui(ui, |ui| {
                    for k in gen::TerrainKind::ALL { ui.selectable_value(&mut g.kind, k, t(k.name())); }
                });
            });
            row(ui, &t("Seed"), |ui| { ui.add(egui::DragValue::new(&mut g.seed)); });
            number(ui, t("Base height"), &mut g.base, 0.0..=800.0);
            number(ui, t("Height range"), &mut g.amplitude, 1.0..=1500.0);
            number(ui, t("Feature size"), &mut g.feature, 100.0..=15000.0);
            row(ui, &t("Detail"), |ui| { ui.add(egui::Slider::new(&mut g.octaves, 1..=12)); });
            number(ui, t("Roughness"), &mut g.roughness, 0.2..=0.8);
            number(ui, t("Warp"), &mut g.warp, 0.0..=2000.0);
            number(ui, t("Blend"), &mut g.blend, 0.0..=1.0);
        }
        GeneratorKind::Erosion(e) => {
            row(ui, &t("Droplets"), |ui| { ui.add(egui::Slider::new(&mut e.droplets, 10_000..=4_000_000).logarithmic(true)); });
            number(ui, t("Erosion rate"), &mut e.erode, 0.05..=0.9);
            number(ui, t("Deposition"), &mut e.deposit, 0.05..=0.9);
        }
        GeneratorKind::ThermalErosion { iterations, talus } => {
            row(ui, &t("Passes"), |ui| { ui.add(egui::Slider::new(iterations, 1..=200)); });
            number(ui, t("Slope limit"), talus, 10.0..=70.0);
        }
        GeneratorKind::Terraces { step, sharpness } => {
            number(ui, t("Step height"), step, 1.0..=80.0);
            number(ui, t("Sharpness"), sharpness, 0.0..=1.0);
        }
    }
    sample
}

/// A library entry drawn like a model in the browser.
fn tile(ui: &mut egui::Ui, side: f32, name: &str, letter: &str, color: Color32, selected: bool, image: Option<egui::TextureId>) -> egui::Response {
    let cell = vec2(side + 8.0, side + 26.0);
    let (rect, resp) = ui.allocate_exact_size(cell, egui::Sense::click_and_drag());
    ui.painter().rect_filled(rect, 3.0, if selected { theme::ACCENT_SOFT } else if resp.hovered() { theme::HOVER } else { Color32::from_rgb(42, 42, 44) });
    let art = egui::Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(side, side));
    match image {
        Some(id) => { ui.painter().image(id, art, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE); }
        None => {
            ui.painter().rect_filled(art, 2.0, color);
            ui.painter().text(art.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(side * 0.42), Color32::from_white_alpha(210));
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
        GeneratorKind::Scatter(_) => ("O", Color32::from_rgb(92, 96, 60)),
        GeneratorKind::Area(_) => ("A", Color32::from_rgb(70, 104, 66)),
        GeneratorKind::Line(_) => ("L", Color32::from_rgb(92, 84, 112)),
        GeneratorKind::River(_) => ("R", Color32::from_rgb(40, 96, 130)),
        GeneratorKind::LineTerrain(_) => ("G", Color32::from_rgb(112, 96, 62)),
        GeneratorKind::Remove(_) => ("X", Color32::from_rgb(132, 56, 52)),
        GeneratorKind::Pond(_) => ("W", Color32::from_rgb(44, 110, 150)),
        GeneratorKind::Terrain(_) => ("T", Color32::from_rgb(96, 110, 70)),
        GeneratorKind::Erosion(_) => ("E", Color32::from_rgb(110, 92, 70)),
        GeneratorKind::ThermalErosion { .. } => ("E", Color32::from_rgb(120, 100, 76)),
        GeneratorKind::Terraces { .. } => ("T", Color32::from_rgb(120, 110, 80)),
    }
}

/// What a library entry shows as its picture.
#[derive(Clone)]
enum Preview {
    Model(String),
    Picture(String, GeneratorKind),
    Nothing,
}

/// Forest presets read from a file become available everywhere.
fn keep_forest_presets(library: &mut Library, dirty: &mut bool, loaded: Vec<forest_core::user_presets::UserPreset>) {
    for p in loaded {
        match library.forest_presets.iter_mut().find(|x| x.name == p.name) {
            Some(x) => *x = p,
            None => library.forest_presets.push(p),
        }
        *dirty = true;
    }
}

fn preset_model(app: &App, preset: &str, user: &[forest_core::user_presets::UserPreset]) -> Option<String> {
    use forest_core::user_presets::UserPreset;
    let weights = user.iter().find(|p| p.name == preset).cloned().or_else(|| forest_core::species::zone_presets().iter().find(|p| p.name == preset).map(UserPreset::from_builtin))?.weights;
    let heaviest = weights.iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
    app.assets.resolve(&heaviest.0).map(str::to_string)
}

fn preview_of(app: &App, kind: &GeneratorKind) -> Preview {
    let model = match kind {
        GeneratorKind::Forest(f) => preset_model(app, &f.preset, &f.user_presets).or_else(|| preset_model(app, &f.preset, &app.library.all_forest_presets())),
        GeneratorKind::Scatter(s) => s.preset.and_then(|id| app.library.objects.iter().find(|p| p.id == id)).and_then(|p| p.items.first().map(|m| m.0.clone())).or_else(|| s.models.first().map(|m| m.0.clone())),
        GeneratorKind::Area(g) => g.objects.first_model(&app.library.objects).map(str::to_string),
        GeneratorKind::Line(g) => g.objects.first_model(&app.library.objects).map(str::to_string),
        GeneratorKind::Settlement(s) => s.buildings.first().cloned(),
        GeneratorKind::Fence(g) => g.preset.and_then(|id| app.library.objects.iter().find(|p| p.id == id)).and_then(|p| p.items.first().map(|m| m.0.clone())).or_else(|| Some(g.model.clone()).filter(|m| !m.is_empty())),
        GeneratorKind::PowerLine(g) => g.poles.first().and_then(|p| app.assets.resolve(&p.name).map(str::to_string)),
        _ => None,
    };
    match model {
        Some(m) => Preview::Model(ObjectStore::normalize_path(&m)),
        None if matches!(kind, GeneratorKind::Forest(_) | GeneratorKind::Scatter(_) | GeneratorKind::Settlement(_) | GeneratorKind::Fence(_) | GeneratorKind::PowerLine(_) | GeneratorKind::Area(_) | GeneratorKind::Line(_)) => Preview::Nothing,
        None => {
            let key = match kind {
                GeneratorKind::Terrain(p) => format!("terrain {p:?}"),
                GeneratorKind::SurfacePaint { rules, .. } => format!("paint {:?}", rules.iter().map(|r| r.layer).collect::<Vec<_>>()),
                other => other.type_name().to_string(),
            };
            Preview::Picture(key, kind.clone())
        }
    }
}

/// A small hillshaded terrain, or the surface colours of a painting generator.
fn picture_of(app: &App, kind: &GeneratorKind) -> egui::ColorImage {
    const N: usize = 96;
    if let GeneratorKind::SurfacePaint { rules, .. } = kind {
        let colors: Vec<[u8; 3]> = app.doc.as_ref().map(|d| rules.iter().filter_map(|r| d.world.layers.get(r.layer).map(|l| l.color)).collect()).unwrap_or_default();
        let colors: Vec<[u8; 3]> = colors.into_iter().chain([[108, 142, 66], [136, 156, 74], [120, 96, 70]]).take(3).collect();
        let mut px = Vec::with_capacity(N * N);
        for y in 0..N {
            for x in 0..N {
                let c = colors[((x + y) * colors.len() / (2 * N)).min(colors.len() - 1)];
                px.push(Color32::from_rgb(c[0], c[1], c[2]));
            }
        }
        return egui::ColorImage { size: [N, N], pixels: px };
    }
    if matches!(kind, GeneratorKind::River(_)) {
        let mut px = Vec::with_capacity(N * N);
        for y in 0..N {
            let middle = 48.0 + (y as f32 * 0.09).sin() * 20.0 + (y as f32 * 0.031).cos() * 7.0;
            for x in 0..N {
                let d = (x as f32 - middle).abs();
                let grain = ((x * 7 + y * 13) % 11) as f32;
                px.push(if d < 8.0 { Color32::from_rgb(38, 92 + (8.0 - d) as u8 * 4, 128 + (8.0 - d) as u8 * 6) }
                    else if d < 12.0 { Color32::from_rgb(126, 112, 82) }
                    else { Color32::from_rgb(92 + grain as u8, 122 + grain as u8, 62) });
            }
        }
        return egui::ColorImage { size: [N, N], pixels: px };
    }
    let params = if let GeneratorKind::Terrain(p) = kind { p.clone() } else { gen::TerrainParams::default() };
    let job = dz_core::Job::default();
    let cell = params.feature / 24.0;
    let mut h = dz_core::terrain::Heightmap::new(N, N, cell, params.base);
    gen::generate_terrain(&mut h, &params, &job);
    match kind {
        GeneratorKind::Terraces { step, sharpness } => gen::terrace(&mut h, step * params.amplitude / 60.0, *sharpness),
        GeneratorKind::ThermalErosion { talus, .. } => gen::thermal_erosion(&mut h, 8, *talus, &job),
        GeneratorKind::Erosion(e) => gen::hydraulic_erosion(&mut h, &gen::ErosionParams { droplets: 12_000, ..e.clone() }, &job),
        _ => {}
    }
    let (lo, hi) = h.min_max();
    let mut px = Vec::with_capacity(N * N);
    for j in (0..N).rev() {
        for i in 0..N {
            let n = h.normal(i as f32 * cell, j as f32 * cell);
            let shade = (n[0] * -0.45 + n[1] * 0.75 + n[2] * 0.45).clamp(0.15, 1.0);
            let t = ((h.at(i, j) - lo) / (hi - lo).max(0.01)).clamp(0.0, 1.0);
            let c = [(70.0 + 120.0 * t) * shade, (105.0 + 70.0 * t) * shade, (60.0 + 80.0 * t) * shade];
            px.push(Color32::from_rgb(c[0] as u8, c[1] as u8, c[2] as u8));
        }
    }
    egui::ColorImage { size: [N, N], pixels: px }
}

fn preview_texture(app: &mut App, ctx: &egui::Context, preview: &Preview) -> Option<egui::TextureId> {
    match preview {
        Preview::Model(m) => app.thumbs.get(m).map(|t| t.id()),
        Preview::Picture(key, kind) => {
            if !app.lib_thumbs.contains_key(key) {
                let image = picture_of(app, kind);
                let handle = ctx.load_texture(format!("lib:{key}"), image, egui::TextureOptions::LINEAR);
                app.lib_thumbs.insert(key.clone(), handle);
            }
            app.lib_thumbs.get(key).map(|t| t.id())
        }
        Preview::Nothing => None,
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
        if let Some(p) = r.dnd_release_payload::<LibDrag>() { moved = Some(((*p).clone(), f.clone())); }
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

fn grid_columns(ui: &egui::Ui, side: f32) -> usize {
    (ui.available_width() / (side + 12.0)).floor().max(1.0) as usize
}

pub fn generators_tab(app: &mut App, ui: &mut egui::Ui) {
    let mut folders = vec![BUILT_IN.to_string()];
    folders.extend(app.library.generator_folders.iter().cloned());
    let (mut current, mut new_name) = (app.gen_folder.clone(), std::mem::take(&mut app.new_folder));
    let mut result = (None, None);
    egui::SidePanel::left("gen_folders").resizable(true).default_width(180.0).frame(egui::Frame::new()).show_inside(ui, |ui| {
        ui.menu_button(format!("+ {}", t("New generator")), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
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
    let shown: Vec<(u64, String, &'static str, Color32, bool, Preview)> = app.built_in.iter().map(|g| (g, true)).chain(app.library.generators.iter().map(|g| (g, false)))
        .filter(|(g, _)| current.as_deref().is_none_or(|f| f == g.folder))
        .map(|(g, b)| { let (l, c) = kind_look(&g.kind); (g.id, if b { t(&g.name) } else { g.name.clone() }, l, c, b, preview_of(app, &g.kind)) }).collect();
    let side = app.settings.library_tile;
    let columns = grid_columns(ui, side);
    let mut add = None;
    let mut delete = None;
    let mut copy = None;
    egui::ScrollArea::vertical().id_salt("gens").auto_shrink([false, false]).show(ui, |ui| {
        for chunk in shown.chunks(columns) {
            ui.horizontal(|ui| {
                for (id, name, letter, color, built_in, preview) in chunk {
                    let image = preview_texture(app, ui.ctx(), preview);
                    let r = tile(ui, side, name, letter, *color, app.edit_generator == Some(*id), image);
                    if r.clicked() { app.edit_generator = Some(*id); app.selected_generator = None; app.edit_forest = None; app.edit_object = None; }
                    if r.double_clicked() { add = Some(*id); }
                    r.dnd_set_drag_payload(LibDrag::Generator(*id));
                    r.context_menu(|ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
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
    let forests_label = t("Forests");
    let mut folders = vec![t(ROADS), forests_label.clone()];
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
        if folder != roads_label && folder != forests_label {
            if let Some(p) = app.library.objects.iter_mut().find(|p| p.id == id) { p.folder = folder; app.lib_dirty = true; }
        }
    }
    if let Some(name) = result.1 {
        if name != roads_label && name != forests_label && !app.library.object_folders.contains(&name) { app.library.object_folders.push(name); app.lib_dirty = true; }
    }
    let side = app.settings.library_tile;
    let columns = grid_columns(ui, side);
    let show_roads = current.as_deref().is_none_or(|f| f == roads_label);
    let show_forests = current.as_deref().is_none_or(|f| f == forests_label);
    let forests: Vec<(String, Option<String>)> = if show_forests { let own = app.library.forest_presets.clone(); own.iter().map(|p| p.name.clone()).chain(gen::forest_preset_names()).map(|n| { let m = preset_model(app, &n, &own).map(|m| ObjectStore::normalize_path(&m)); (n, m) }).collect() } else { Vec::new() };
    let road_model = |app: &App, family: &str| app.road_catalog.iter().filter(|c| c.family == family && c.category == 3).min_by_key(|c| c.path.len()).map(|c| {
        let path = ObjectStore::normalize_path(&c.path);
        if path.contains('\\') { path.strip_prefix("p:\\").unwrap_or(&path).to_string() } else { format!("dz\\structures\\roads\\parts\\{path}") }
    });
    let families: Vec<String> = if show_roads { app.road_families.clone() } else { Vec::new() };
    let shown: Vec<(u64, String, Option<String>)> = app.library.objects.iter().filter(|p| current.as_deref().is_none_or(|f| f == p.folder)).map(|p| (p.id, p.name.clone(), p.items.first().map(|i| i.0.clone()))).collect();
    enum Entry { Road(String, Option<String>), Forest(String, Option<String>), Preset(u64, String, Option<String>) }
    let entries: Vec<Entry> = families.into_iter().map(|f| { let m = road_model(app, &f); Entry::Road(f, m) }).chain(forests.into_iter().map(|(n, m)| Entry::Forest(n, m))).chain(shown.into_iter().map(|(a, b, c)| Entry::Preset(a, b, c))).collect();
    let mut add_forest_now = None;
    let (mut copy_forest, mut delete_forest, mut copy_object) = (None, None, None);
    let own_forests: Vec<String> = app.library.forest_presets.iter().map(|p| p.name.clone()).collect();
    let mut delete = None;
    egui::ScrollArea::vertical().id_salt("presets").auto_shrink([false, false]).show(ui, |ui| {
        for chunk in entries.chunks(columns) {
            ui.horizontal(|ui| {
                for e in chunk {
                    match e {
                        Entry::Forest(name, model) => {
                            let image = model.as_ref().and_then(|m| app.thumbs.get(m).map(|t| t.id()));
                            let r = tile(ui, side, name, "F", Color32::from_rgb(44, 96, 56), app.edit_forest.as_ref() == Some(name), image);
                            if r.clicked() {
                                app.edit_forest = Some(name.clone());
                                app.edit_object = None;
                                app.edit_generator = None;
                                app.selected_generator = None;
                            }
                            if r.double_clicked() { add_forest_now = Some(name.clone()); }
                            r.dnd_set_drag_payload(LibDrag::ForestPreset(name.clone()));
                            r.context_menu(|ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); if ui.button(t("Add to the project")).clicked() { add_forest_now = Some(name.clone()); ui.close_menu(); } if ui.button(t("Duplicate")).clicked() { copy_forest = Some(name.clone()); ui.close_menu(); } if ui.add_enabled(own_forests.contains(name), egui::Button::new(t("Delete"))).clicked() { delete_forest = Some(name.clone()); ui.close_menu(); } });
                        }
                        Entry::Road(family, model) => {
                            let image = model.as_ref().and_then(|m| app.thumbs.get(m).map(|t| t.id()));
                            let r = tile(ui, side, family, "R", Color32::from_rgb(70, 72, 78), app.line.family == *family, image);
                            if r.clicked() {
                                app.line.family = family.clone();
                                app.line.action = LineAction::Road;
                                app.mode = Mode::Lines;
                            }
                            r.context_menu(|ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); if ui.button(t("Road catalog...")).clicked() { app.line.family = family.clone(); app.dialog = Dialog::RoadCatalog; ui.close_menu(); } });
                        }
                        Entry::Preset(id, name, first) => {
                            let image = first.as_ref().and_then(|m| app.thumbs.get(m).map(|t| t.id()));
                            let r = tile(ui, side, name, "O", Color32::from_rgb(64, 78, 92), app.edit_object == Some(*id), image);
                            if r.clicked() {
                                app.edit_object = Some(*id);
                                app.selected_generator = None;
                                app.edit_generator = None;
                                app.edit_forest = None;
                                app.place.preset = Some(*id);
                                if app.mode == Mode::Select { app.mode = Mode::Place; }
                            }
                            if let Some(p) = r.dnd_release_payload::<String>() {
                                if let Some(preset) = app.library.objects.iter_mut().find(|x| x.id == *id) {
                                    if !preset.items.iter().any(|i| i.0 == *p) { preset.items.push(((*p).clone(), 1.0)); app.lib_dirty = true; }
                                }
                            }
                            r.dnd_set_drag_payload(LibDrag::ObjectPreset(*id));
                            r.context_menu(|ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); if ui.button(t("Duplicate")).clicked() { copy_object = Some(*id); ui.close_menu(); } if ui.button(t("Delete")).clicked() { delete = Some(*id); ui.close_menu(); } });
                        }
                    }
                }
            });
        }
    });
    if let Some(name) = add_forest_now { add_forest(app, &name, None); }
    if let Some(name) = copy_forest { duplicate_forest_preset(app, &name); }
    if let Some(name) = delete_forest { remove_forest_preset(app, &name); }
    if let Some(id) = copy_object { duplicate_object_preset(app, id); }
    if let Some(id) = delete {
        app.library.objects.retain(|p| p.id != id);
        if app.edit_object == Some(id) { app.edit_object = None; }
        if app.place.preset == Some(id) { app.place.preset = None; }
        app.lib_dirty = true;
    }
}

/// Shows the library entry or project generator being edited. Returns false when there is none.
/// A whole road picked with the Select tool.
fn whole_route_details(app: &mut App, ui: &mut egui::Ui, id: u64) {
    let Some(doc) = &app.doc else { return };
    let Some(index) = doc.world.routes.iter().position(|r| r.id == id) else { app.whole = None; return };
    let r = &doc.world.routes[index];
    ui.strong(r.name.clone());
    ui.weak(r.family.clone());
    ui.label(tf("{0} models", &[&r.objects.len().to_string()]));
    let (mut edit, mut delete) = (false, false);
    ui.horizontal_wrapped(|ui| {
        edit = ui.button(t("Edit route")).clicked();
        delete = ui.button(t("Delete route")).clicked();
    });
    if edit {
        app.mode = Mode::Lines;
        app.line.action = LineAction::Edit;
        app.line.selected_route = Some(index);
        app.whole = None;
        app.selection.clear();
        app.selection_rev += 1;
    }
    if delete {
        app.delete_selection();
    }
}

pub fn details(app: &mut App, ui: &mut egui::Ui) -> bool {
    if let Some(SelectedWhole::Route(id)) = app.whole {
        whole_route_details(app, ui, id);
        return true;
    }
    if let Some(id) = app.selected_generator {
        instance_details(app, ui, id);
        return true;
    }
    if let Some(name) = app.edit_forest.clone() {
        forest_preset_details(app, ui, &name);
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

fn unused_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let copy = t("copy");
    (1..).map(|n| if n == 1 { format!("{base} ({copy})") } else { format!("{base} ({copy} {n})") }).find(|n| !taken(n)).unwrap()
}

fn duplicate_object_preset(app: &mut App, id: u64) {
    let Some(mut p) = app.library.objects.iter().find(|p| p.id == id).cloned() else { return };
    p.id = new_id();
    p.name = unused_name(&p.name, |n| app.library.objects.iter().any(|x| x.name == n));
    app.edit_object = Some(p.id);
    app.edit_forest = None;
    app.library.objects.push(p);
    app.lib_dirty = true;
}

/// A forest preset as it stands now, from the library or the ones the program comes with.
fn forest_preset(app: &App, name: &str) -> Option<forest_core::user_presets::UserPreset> {
    use forest_core::user_presets::UserPreset;
    app.library.forest_presets.iter().find(|p| p.name == name).cloned().or_else(|| forest_core::species::zone_presets().iter().find(|p| p.name == name).map(UserPreset::from_builtin))
}

fn duplicate_forest_preset(app: &mut App, name: &str) {
    let Some(mut p) = forest_preset(app, name) else { return };
    p.name = unused_name(&p.name, |n| app.library.forest_presets.iter().any(|x| x.name == n) || gen::forest_preset_names().iter().any(|x| x == n));
    app.edit_forest = Some(p.name.clone());
    app.library.forest_presets.push(p);
    app.lib_dirty = true;
}

fn remove_forest_preset(app: &mut App, name: &str) {
    app.library.forest_presets.retain(|p| p.name != name);
    if app.edit_forest.as_deref() == Some(name) { app.edit_forest = None; }
    app.lib_dirty = true;
}

fn forest_preset_details(app: &mut App, ui: &mut egui::Ui, name: &str) {
    let Some(current) = forest_preset(app, name) else { app.edit_forest = None; return };
    let index = app.library.forest_presets.iter().position(|p| p.name == name);
    let Some(index) = index else {
        // The presets that come with the program stay as they are.
        ui.strong(name);
        ui.weak(t("A built-in preset. Duplicate it to make your own version."));
        ui.label(format!("{}: {:.0}", t("Per hectare"), current.density_per_ha));
        for (m, k) in current.weights.iter().take(30) { ui.label(format!("{k:.2}  {m}")); }
        ui.horizontal_wrapped(|ui| {
            if ui.button(t("Duplicate")).clicked() { duplicate_forest_preset(app, name); }
            if ui.button(t("Add to the project")).clicked() { add_forest(app, name, None); }
        });
        return;
    };
    let model = app.place.model.clone();
    let taken: Vec<String> = app.library.forest_presets.iter().map(|p| p.name.clone()).chain(gen::forest_preset_names()).filter(|n| n != name).collect();
    let p = &mut app.library.forest_presets[index];
    let before = p.clone();
    row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut p.name); });
    if p.name.trim().is_empty() || taken.contains(&p.name) { p.name = before.name.clone(); }
    number(ui, t("Per hectare"), &mut p.density_per_ha, 1.0..=1500.0);
    let mut remove = None;
    let list = ui.vertical(|ui| {
        for (i, (m, k)) in p.weights.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(k).speed(0.05).range(0.01..=100.0));
                ui.label(m.as_str());
                if ui.small_button("✖").clicked() { remove = Some(i); }
            });
        }
    }).response;
    let stem = |m: &str| m.rsplit('\\').next().unwrap_or(m).trim_end_matches(".p3d").to_string();
    if let Some(m) = list.dnd_release_payload::<String>() { let s = stem(&m); if !p.weights.iter().any(|w| w.0 == s) { p.weights.push((s, 1.0)); } }
    if let Some(i) = remove { p.weights.remove(i); }
    if ui.button(format!("+ {}", t("Add selected model"))).on_hover_text(t("Models can also be dropped here.")).clicked() {
        if let Some(m) = &model { let s = stem(m); if !p.weights.iter().any(|w| w.0 == s) { p.weights.push((s, 1.0)); } }
    }
    let changed = *p != before;
    let renamed = (p.name != before.name).then(|| p.name.clone());
    if changed { app.lib_dirty = true; }
    if let Some(n) = renamed { app.edit_forest = Some(n); }
    ui.horizontal_wrapped(|ui| {
        if ui.button(t("Duplicate")).clicked() { let n = app.library.forest_presets[index].name.clone(); duplicate_forest_preset(app, &n); }
        if ui.button(t("Add to the project")).clicked() { let n = app.library.forest_presets[index].name.clone(); add_forest(app, &n, None); }
        if ui.button(t("Delete")).clicked() { let n = app.library.forest_presets[index].name.clone(); remove_forest_preset(app, &n); }
    });
}

fn object_preset_details(app: &mut App, ui: &mut egui::Ui, id: u64) {
    if ui.button(t("Duplicate")).clicked() { duplicate_object_preset(app, id); return; }
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
    keep_forest_presets(&mut app.library, &mut app.lib_dirty, cx.loaded.take());
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
    let routes: Vec<(u64, String)> = doc.world.routes.iter().map(|r| (r.id, r.name.clone())).collect();
    let selected_route = app.line.selected_route.and_then(|i| doc.world.routes.get(i)).map(|r| r.id);
    let doc = app.doc.as_mut().unwrap();
    let g = &mut doc.world.generators[index];
    row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut g.name); });
    ui.weak(t(g.kind.type_name()));
    ui.label(tf("{0} objects", &[&g.objects.len().to_string()]));
    if !g.removed.is_empty() { ui.label(tf("{0} objects removed", &[&g.removed.len().to_string()])); }
    match g.kind.target_kind() {
        TargetKind::Point => {
            if let GenTarget::Point(p) = &mut g.target {
                row(ui, "X", |ui| { ui.add(egui::DragValue::new(&mut p[0]).speed(1.0)); });
                row(ui, "Z", |ui| { ui.add(egui::DragValue::new(&mut p[1]).speed(1.0)); });
            }
        }
        TargetKind::Map => {}
        kind => {
            let shown = match &g.target {
                GenTarget::Route(id) => routes.iter().find(|r| r.0 == *id).map(|r| r.1.clone()).unwrap_or_default(),
                GenTarget::Map => t("Whole map"),
                GenTarget::Layer(l) => l.clone(),
                GenTarget::Shape { layer, index } => format!("{layer} #{}", index + 1),
                GenTarget::Point(_) => String::new(),
            };
            row(ui, &t("Works on"), |ui| {
                egui::ComboBox::from_id_salt("target").selected_text(shown).show_ui(ui, |ui| {
                    if matches!(kind, TargetKind::Area | TargetKind::Any) { ui.selectable_value(&mut g.target, GenTarget::Map, t("Whole map")); }
                    for l in &shape_layers { ui.selectable_value(&mut g.target, GenTarget::Layer(l.clone()), l); }
                    if matches!(kind, TargetKind::Line | TargetKind::Any) {
                        for (id, name) in &routes { ui.selectable_value(&mut g.target, GenTarget::Route(*id), name); }
                    }
                });
            });
            if matches!(kind, TargetKind::Line | TargetKind::Any) && ui.add_enabled(selected_route.is_some(), egui::Button::new(t("Use selected route"))).clicked() {
                if let Some(id) = selected_route { g.target = GenTarget::Route(id); }
            }
            if ui.add_enabled(selected_shape.is_some(), egui::Button::new(t("Use selected shape"))).clicked() {
                if let Some((layer, index)) = selected_shape { g.target = GenTarget::Shape { layer, index }; }
            }
        }
    }
    ui.separator();
    if kind_ui(ui, &mut g.kind, &cx) { app.sampling = Some(id); }
    keep_forest_presets(&mut app.library, &mut app.lib_dirty, cx.loaded.take());
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
        row.context_menu(|ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
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

pub fn delete_instance(app: &mut App, id: u64, with_objects: bool) {
    let Some(doc) = &mut app.doc else { return };
    let Some(g) = doc.world.generators.iter().find(|g| g.id == id).cloned() else { return };
    let before = doc.world.generators.clone();
    doc.world.generators.retain(|x| x.id != id);
    let label = t("Delete generator");
    doc.history.push(label.clone(), Change::Generators(before));
    if with_objects {
        let mut change: Vec<_> = g.objects.iter().filter_map(|id| doc.world.objects.remove(*id).map(|o| (*id, Some(o), None))).collect();
        for saved in &g.removed {
            let model = doc.world.objects.intern(&saved.model);
            let o = saved.object(model);
            change.push((doc.world.objects.insert(o), None, Some(o)));
        }
        doc.history.push(label.clone(), Change::Objects(change));
        doc.history.group_last(2, label);
        doc.dirty.objects = true;
        let doc = app.doc.as_ref().unwrap();
        app.selection.retain(|i| doc.world.objects.get(*i).is_some());
        app.selection_rev += 1;
    }
    if app.selected_generator == Some(id) { app.selected_generator = None; }
    if let (true, Some(m)) = (with_objects, g.modifier) {
        app.set_modifiers(&t("Delete generator"), |list| list.retain(|x| x.id != m));
    }
}

/// Puts a library generator into the project. It works on what it was dropped on, the
/// selected shape or road or, for what needs nothing, the whole map.
pub fn add_instance(app: &mut App, preset: u64, drop: Option<Dropped>) {
    let Some(p) = app.built_in.iter().chain(app.library.generators.iter()).find(|g| g.id == preset).cloned() else { return };
    let name = if p.folder == BUILT_IN { t(&p.name) } else { p.name.clone() };
    add_kind(app, p.kind, name, drop);
}

/// A forest generator using one preset of the library.
pub fn add_forest(app: &mut App, preset: &str, drop: Option<Dropped>) {
    let forest = ForestGen { preset: preset.to_string(), ..Default::default() };
    add_kind(app, GeneratorKind::Forest(Box::new(forest)), preset.to_string(), drop);
}

fn add_kind(app: &mut App, kind: GeneratorKind, name: String, drop: Option<Dropped>) {
    let selected_route = app.line.selected_route.and_then(|i| app.doc.as_ref()?.world.routes.get(i)).map(|r| r.id);
    let Some(doc) = &mut app.doc else { return };
    let shape_at = drop.and_then(|d| d.shape).or_else(|| app.shape.selection.iter().next().copied());
    let wants = kind.target_kind();
    let usable = shape_at.and_then(|(l, s)| {
        let layer = doc.world.shape_layers.get(l)?;
        let shape = layer.shapes.get(s)?;
        let fits = match wants {
            TargetKind::Area => shape.kind == ShapeKind::Polygon,
            TargetKind::Line | TargetKind::Any => shape.kind != ShapeKind::Point,
            _ => false,
        };
        fits.then(|| GenTarget::Shape { layer: layer.name.clone(), index: s })
    });
    let on_route = drop.and_then(|d| d.route).or(selected_route).filter(|_| matches!(wants, TargetKind::Line | TargetKind::Any)).map(GenTarget::Route);
    // Dropped on a road, the road wins over a shape that merely happens to be selected.
    let usable = if drop.is_some_and(|d| d.route.is_some() && d.shape.is_none()) { on_route.clone().or(usable) } else { usable.or(on_route) };
    let target = match (wants, usable, drop) {
        (_, Some(t), _) => t,
        (TargetKind::Point, _, Some(d)) => GenTarget::Point(d.at),
        (TargetKind::Point, _, None) => { let s = doc.world.meta.size_m(); GenTarget::Point([s * 0.5, s * 0.5]) }
        (TargetKind::Area | TargetKind::Map | TargetKind::Any, _, _) => GenTarget::Map,
        (TargetKind::Line, _, _) => {
            app.warn(t("Drop this generator on a line or a road, or select one first"));
            return;
        }
    };
    let before = doc.world.generators.clone();
    let id = new_id();
    let run = target != GenTarget::Map && match &kind {
        GeneratorKind::Settlement(s) => !s.buildings.is_empty(),
        GeneratorKind::Scatter(s) => !s.models.is_empty() || s.preset.is_some(),
        GeneratorKind::Area(g) => !g.objects.models.is_empty() || g.objects.preset.is_some(),
        GeneratorKind::Line(g) => !g.objects.models.is_empty() || g.objects.preset.is_some(),
        GeneratorKind::Fence(g) => !g.model.is_empty() || g.preset.is_some(),
        _ => true,
    };
    doc.world.generators.push(GeneratorInstance { id, name, kind, target, objects: Vec::new(), modifier: None, removed: Vec::new() });
    doc.history.push(t("Add generator"), Change::Generators(before));
    app.selected_generator = Some(id);
    app.selected_modifier = None;
    if run { run_generator(app, id); }
}

/// Lines a generator can follow: roads, polylines, and the outlines of polygons.
fn target_lines(w: &World, target: &GenTarget) -> Vec<Vec<[f32; 2]>> {
    if let GenTarget::Route(id) = target {
        return w.routes.iter().find(|r| r.id == *id).map(|r| vec![r.centerline.clone()]).unwrap_or_default();
    }
    let mut out: Vec<Vec<[f32; 2]>> = target_shapes(w, target, ShapeKind::Polyline).into_iter().map(|s| s.points).collect();
    for s in target_shapes(w, target, ShapeKind::Polygon) {
        let mut ring = s.points;
        ring.push(ring[0]);
        out.push(ring);
    }
    out
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
    let presets = app.library.objects.clone();
    let library_forest = app.library.all_forest_presets();
    let world = &doc.world;
    let polygons = target_shapes(world, &g.target, ShapeKind::Polygon);
    let lines = target_lines(world, &g.target);
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
            let layer_shapes = areas.first().map(|a: &(ShapeLayer, String)| a.0.clone());
            let layers = f.layers.clone();
            let satellite = world.satellite.as_ref().filter(|_| wants_filter || gen::forest_needs_satellite(&f.project)).map(gen::satellite_for_forest);
            let mut forest = *f;
            for p in library_forest {
                if !forest.user_presets.iter().any(|x| x.name == p.name) { forest.user_presets.push(p); }
            }
            let skip_surfaces: Vec<(usize, f32)> = forest.skip_surfaces.iter().filter_map(|(l, m)| Some((world.layers.iter().position(|x| x.name == *l)?, *m))).collect();
            let keep_out = (forest.avoid_roads.is_some() || forest.avoid_buildings.is_some()).then(|| {
                let buildings = forest.avoid_buildings.map(|_| world.objects.iter().filter_map(|(_, o)| {
                    let path = world.objects.model_path(o);
                    if !path.contains("\\structures") || path.contains("\\roads\\") || path.contains("\\walls\\") || path.contains("\\rail\\") { return None; }
                    let info = db.get(path);
                    let (hx, hz) = if info.known { info.footprint() } else { (4.0, 4.0) };
                    Some([o.x, o.z, o.yaw, hx * o.scale, hz * o.scale])
                }).collect()).unwrap_or_default();
                gen::KeepOut { roads: if forest.avoid_roads.is_some() { world.routes.iter().map(|r| r.centerline.clone()).collect() } else { Vec::new() }, road_margin: forest.avoid_roads.unwrap_or(0.0), buildings, building_margin: forest.avoid_buildings.unwrap_or(0.0) }
            });
            let req = gen::ForestRequest { project: forest.project, zones, areas, area_density: None, satellite, color_filter: forest.color_filter, edge_presets: forest.edge_presets, user_presets: forest.user_presets, skip_surfaces, keep_out };
            app.spawn(label, move |job| {
                let (objects, notes) = gen::generate_forest_layers(&mut lite, &assets, req, layer_shapes, &layers, job)?;
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
        GeneratorKind::Fence(mut f) => {
            f.resolved = f.preset.and_then(|id| presets.iter().find(|p| p.id == id)).map(|p| p.items.clone()).unwrap_or_default();
            if f.resolved.is_empty() && f.model.is_empty() {
                app.warn(t("Choose at least one model"));
                return;
            }
            app.spawn(label, move |_| {
                let mut objects = Vec::new();
                for l in &lines { objects.extend(gen::fence_objects(&mut lite, &db, l, &f)); }
                let models = model_list(&lite);
                Ok(Box::new(move |a: &mut App| { store_generated(a, id, models, objects); }) as Finish)
            });
        }
        GeneratorKind::PowerLine(f) => {
            app.spawn(label, move |_| {
                let mut objects = Vec::new();
                for l in &lines { objects.extend(gen::power_objects(&mut lite, &db, &assets, l, &f)?); }
                let models = model_list(&lite);
                Ok(Box::new(move |a: &mut App| { store_generated(a, id, models, objects); }) as Finish)
            });
        }
        GeneratorKind::Area(mut g) => {
            g.objects = g.objects.resolved(&presets);
            let (index, bounds) = area_bounds(&polygons, world.meta.size_m());
            app.spawn(label, move |job| {
                let objects = dz_core::modules::area_objects(&mut lite, &g, index.as_ref(), bounds, job)?;
                let models = model_list(&lite);
                Ok(Box::new(move |a: &mut App| { store_generated(a, id, models, objects); }) as Finish)
            });
        }
        GeneratorKind::Line(mut g) => {
            g.objects = g.objects.resolved(&presets);
            app.spawn(label, move |job| {
                let objects = dz_core::modules::line_objects(&mut lite, &g, &lines, job)?;
                let models = model_list(&lite);
                Ok(Box::new(move |a: &mut App| { store_generated(a, id, models, objects); }) as Finish)
            });
        }
        GeneratorKind::Scatter(mut rule) => {
            if let Some(p) = rule.preset.and_then(|id| presets.iter().find(|p| p.id == id)) {
                rule.models = p.items.clone();
                (rule.scale_min, rule.scale_max, rule.align_to_slope) = (p.scale_min, p.scale_max, p.align);
            }
            let index = (!polygons.is_empty()).then(|| dz_core::shapes::PolygonIndex::new(&ShapeLayer { name: String::new(), visible: true, shapes: polygons.clone() }));
            let s = world.meta.size_m();
            let mut bounds = if polygons.is_empty() { [0.0, 0.0, s, s] } else { [f32::MAX, f32::MAX, f32::MIN, f32::MIN] };
            for p in polygons.iter().flat_map(|x| &x.points) {
                bounds = [bounds[0].min(p[0]), bounds[1].min(p[1]), bounds[2].max(p[0]), bounds[3].max(p[1])];
            }
            app.spawn(label, move |job| {
                let objects = gen::scatter(&mut lite, &rule, index.as_ref(), bounds, job)?;
                let models = model_list(&lite);
                Ok(Box::new(move |a: &mut App| { store_generated(a, id, models, objects); }) as Finish)
            });
        }
        GeneratorKind::River(r) => {
            let Some(line) = lines.first().map(|l| gen::prepare_line(l, &r.line)) else { return };
            app.spawn(label, move |_| {
                // The bed is cut along the models, wherever they ended up.
                let chain = gen::fit_river(&line, &r);
                let center = chain.as_ref().map_or(&line, |c| &c.center);
                // Without a bed the water lies on the ground as it is.
                let shaped = r.modify_terrain;
                let r = if shaped { r } else { dz_core::library::RiverGen { depth: 0.0, ..r } };
                let (modifier, surface) = gen::river_modifier(&lite.base, 0, &lite.meta.name.clone(), center, &r).ok_or_else(|| anyhow::anyhow!("The line is too short"))?;
                let objects = match &chain {
                    Some(c) => gen::river_chain_objects(&mut lite, &modifier, &surface, &r, c),
                    None => gen::river_objects(&mut lite, &db, &modifier, &surface, &r),
                };
                let models = model_list(&lite);
                Ok(Box::new(move |a: &mut App| {
                    store_generated(a, id, models, objects);
                    if shaped { store_modifier(a, id, modifier); } else { clear_modifier(a, id); }
                }) as Finish)
            });
        }
        GeneratorKind::LineTerrain(g) => {
            let Some(line) = lines.first().map(|l| gen::prepare_line(l, &g.line)) else { return };
            app.spawn(label, move |_| {
                let modifier = gen::road_modifier(&lite.base, 0, &lite.meta.name.clone(), &line, g.width, g.falloff, &[], &g.grading, &[]).ok_or_else(|| anyhow::anyhow!("The line is too short"))?;
                Ok(Box::new(move |a: &mut App| {
                    store_generated(a, id, Vec::new(), Vec::new());
                    store_modifier(a, id, modifier);
                }) as Finish)
            });
        }
        GeneratorKind::Remove(r) => remove_objects(app, id, &r, polygons, lines),
        GeneratorKind::Pond(p) => {
            if polygons.is_empty() {
                app.warn(t("This generator needs a polygon"));
                return;
            }
            app.spawn(label, move |_| {
                let objects = gen::pond_objects(&mut lite, &db, &polygons, &p);
                let modifier = p.modify_terrain.then(|| gen::pond_modifier(&lite.base, 0, "", &polygons, &p)).flatten();
                let models = model_list(&lite);
                Ok(Box::new(move |a: &mut App| {
                    store_generated(a, id, models, objects);
                    match modifier {
                        Some(m) => store_modifier(a, id, m),
                        None => clear_modifier(a, id),
                    }
                }) as Finish)
            });
        }
        GeneratorKind::Terrain(p) => height_job(app, label, move |h, job| gen::generate_terrain(h, &p, job)),
        GeneratorKind::Erosion(p) => height_job(app, label, move |h, job| gen::hydraulic_erosion(h, &p, job)),
        GeneratorKind::ThermalErosion { iterations, talus } => height_job(app, label, move |h, job| gen::thermal_erosion(h, iterations, talus, job)),
        GeneratorKind::Terraces { step, sharpness } => height_job(app, label, move |h, _| gen::terrace(h, step, sharpness)),
    }
}

/// Takes objects off the map, after putting back what the last run took.
fn remove_objects(app: &mut App, id: u64, r: &dz_core::library::RemoveGen, polygons: Vec<MapShape>, lines: Vec<Vec<[f32; 2]>>) {
    let Some(doc) = &mut app.doc else { return };
    let Some(index) = doc.world.generators.iter().position(|g| g.id == id) else { return };
    let name = doc.world.generators[index].name.clone();
    let before = doc.world.generators.clone();
    let world = &mut doc.world;
    let mut change = Vec::new();
    let mut back = std::collections::HashSet::new();
    for saved in std::mem::take(&mut world.generators[index].removed) {
        let model = world.objects.intern(&saved.model);
        let o = saved.object(model);
        let new = world.objects.insert(o);
        back.insert(new);
        change.push((new, None, Some(o)));
    }
    let mut owned: std::collections::HashSet<u32> = world.generators.iter().flat_map(|g| g.objects.iter().copied()).collect();
    owned.extend(world.routes.iter().flat_map(|r| r.objects.iter().copied()));
    let candidates: Vec<(u32, (f32, f32, u32))> = world.objects.iter().filter(|(i, o)| !owned.contains(i) && o.flags & dz_core::objects::FLAG_LOCKED == 0).map(|(i, o)| (i, (o.x, o.z, o.model))).collect();
    let items: Vec<(f32, f32, u32)> = candidates.iter().map(|c| c.1).collect();
    let area = (!polygons.is_empty()).then(|| dz_core::shapes::PolygonIndex::new(&ShapeLayer { name: String::new(), visible: true, shapes: polygons }));
    let models = model_list(world);
    let hits = gen::remove_hits(&items, &models, r, area.as_ref(), &lines);
    let mut removed = Vec::new();
    for ((i, _), _) in candidates.iter().zip(&hits).filter(|(_, hit)| **hit) {
        let Some(o) = world.objects.remove(*i) else { continue };
        removed.push(dz_core::library::RemovedObject::new(&models[o.model as usize], &o));
        // Back and away again in one run is no change at all.
        if back.remove(i) { change.retain(|c: &(u32, Option<Obj>, Option<Obj>)| c.0 != *i); } else { change.push((*i, Some(o), None)); }
    }
    let count = removed.len();
    world.generators[index].removed = removed;
    doc.history.push(name.clone(), Change::Generators(before));
    doc.history.push(name.clone(), Change::Objects(change));
    doc.history.group_last(2, name);
    doc.dirty.objects = true;
    let doc = app.doc.as_ref().unwrap();
    app.selection.retain(|i| doc.world.objects.get(*i).is_some());
    app.selection_rev += 1;
    app.info(tf("{0} objects removed", &[&count.to_string()]));
}

/// What a generator fills: its polygons, or the whole map without any.
fn area_bounds(polygons: &[MapShape], size: f32) -> (Option<dz_core::shapes::PolygonIndex>, [f32; 4]) {
    if polygons.is_empty() {
        return (None, [0.0, 0.0, size, size]);
    }
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in polygons.iter().flat_map(|x| &x.points) {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    (Some(dz_core::shapes::PolygonIndex::new(&ShapeLayer { name: String::new(), visible: true, shapes: polygons.to_vec() })), b)
}

fn height_job(app: &mut App, title: String, f: impl FnOnce(&mut dz_core::terrain::Heightmap, &dz_core::Job) + Send + 'static) {
    let Some(doc) = &app.doc else { return };
    let mut base = doc.world.base.clone();
    let label = title.clone();
    app.spawn(title, move |job| {
        f(&mut base, job);
        job.check()?;
        Ok(Box::new(move |a: &mut App| a.apply_heights(&label, base.data)) as Finish)
    });
}

/// Replaces the terrain modifier of a generator with a new one, undone together with its objects.
fn store_modifier(app: &mut App, id: u64, mut modifier: dz_core::world::TerrainModifier) {
    let Some(doc) = &mut app.doc else { return };
    let Some(index) = doc.world.generators.iter().position(|g| g.id == id) else { return };
    let before = doc.world.modifiers.clone();
    let name = doc.world.generators[index].name.clone();
    modifier.name = name.clone();
    let known = doc.world.generators[index].modifier;
    let at = known.and_then(|m| doc.world.modifiers.iter().position(|x| x.id == m));
    modifier.id = known.unwrap_or_else(|| doc.world.new_modifier_id());
    doc.world.generators[index].modifier = Some(modifier.id);
    let mut area = at.and_then(|k| doc.world.modifier_rect(&doc.world.modifiers[k]));
    if let Some(r) = doc.world.modifier_rect(&modifier) { area = Some(area.map_or(r, |a| a.union(r))); }
    match at {
        Some(k) => doc.world.modifiers[k] = modifier,
        None => doc.world.modifiers.push(modifier),
    }
    doc.history.push(name.clone(), Change::Modifiers(before));
    doc.history.group_last(2, name);
    if let Some(r) = area {
        doc.world.recompose(Some(r));
        app.height_changed(r);
    }
}

/// Takes away the terrain modifier a generator made earlier.
fn clear_modifier(app: &mut App, id: u64) {
    let Some(doc) = &mut app.doc else { return };
    let Some(g) = doc.world.generators.iter_mut().find(|g| g.id == id) else { return };
    let Some(m) = g.modifier.take() else { return };
    let name = g.name.clone();
    app.set_modifiers(&name, |list| list.retain(|x| x.id != m));
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

/// A model of an object preset chosen by weight, with the preset's placement options.
pub fn pick_from_preset(app: &mut App, id: u64, seed: u32) -> Option<(String, (bool, bool, f32, f32))> {
    let p = app.library.objects.iter().find(|p| p.id == id).cloned()?;
    let total: f32 = p.items.iter().map(|i| i.1).sum();
    if p.items.is_empty() || total <= 0.0 {
        app.warn(t("This preset has no models yet"));
        return None;
    }
    let mut pick = ((seed >> 8) as f32 / (u32::MAX >> 8) as f32) * total;
    let mut model = p.items[0].0.clone();
    for (m, w) in &p.items {
        model = m.clone();
        pick -= w;
        if pick <= 0.0 { break; }
    }
    Some((model, (p.random_yaw, p.align, p.scale_min, p.scale_max)))
}

/// Places one model of an object preset.
pub fn place_preset(app: &mut App, id: u64, x: f32, z: f32) {
    let seed = (x.to_bits() ^ z.to_bits().rotate_left(13)).wrapping_mul(2654435761);
    let Some((model, options)) = pick_from_preset(app, id, seed) else { return };
    let saved = (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max);
    (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max) = options;
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
