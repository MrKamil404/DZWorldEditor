//! Mask modifiers in the editor: the surface stamped under objects, the ground of forests and
//! the result of surface painting, each a layer that can be hidden or deleted.
use crate::app::{App, Finish};
use crate::i18n::{t, tf};
use crate::theme::{row, section};
use dz_core::edit::Change;
use dz_core::maskmod::{MaskKind, Stamp, StampSettings, StampShape, Tile};
use dz_core::terrain::Rect;
use eframe::egui;
use std::collections::HashMap;

/// Settings of the tool that stamps a surface under objects.
pub struct MaskTool {
    /// Index of the surface layer to stamp.
    pub surface: usize,
    pub stamp: StampSettings,
    pub selection_only: bool,
    pub plants: bool,
    pub buildings: bool,
    /// Fill the footprint of buildings instead of stamping a patch under them.
    pub footprint: bool,
    /// Models with a surface assigned in the asset browser get that one.
    pub own_surfaces: bool,
}
impl Default for MaskTool {
    fn default() -> Self {
        Self { surface: 0, stamp: StampSettings::default(), selection_only: true, plants: true, buildings: false, footprint: true, own_surfaces: true }
    }
}

impl App {
    /// Shows a part of the mask the modifiers changed.
    pub fn mask_recomposed(&mut self, r: Rect) {
        let Some(doc) = &mut self.doc else { return };
        doc.dirty.mask = true;
        self.renderer.lock().unwrap().update_mask(&doc.world.mask, r);
    }
}

/// Gives a generator's mask modifier, or a new one, these tiles. One undo step.
pub fn store(app: &mut App, generator: Option<u64>, kind: MaskKind, name: String, mut tiles: HashMap<dz_core::maskmod::Key, Tile>) -> Option<u32> {
    let d = app.doc.as_mut()?;
    let before = d.world.mask_mods.modifiers.clone();
    let known = generator.and_then(|g| d.world.generators.iter().find(|x| x.id == g)).and_then(|g| g.mask_modifier).filter(|m| d.world.mask_mods.position(*m).is_some());
    let id = known.unwrap_or_else(|| d.world.mask_mods.add(kind, name.clone()));
    if let Some(at) = d.world.mask_mods.position(id) {
        d.world.mask_mods.modifiers[at].name = name.clone();
    }
    // The texture tiles keep the number of surfaces the project allows, counted without what
    // this modifier held before.
    let cleared = d.world.mask_replace_tiles(id, HashMap::new());
    let dropped = dz_core::maskmod::keep_tile_limit(&d.world, &mut tiles);
    let limit = d.world.meta.max_tile_surfaces;
    if tiles.is_empty() && known.is_none() {
        // Nothing of a new modifier is left, so there is none.
        d.world.mask_mods.modifiers.retain(|m| m.id != id);
        if dropped > 0 {
            app.warn(tf("{0} pixels were left as they were, as their texture tiles already hold {1} surfaces", &[&dropped.to_string(), &limit.to_string()]));
        }
        return None;
    }
    let touched = d.world.mask_replace_tiles(id, tiles);
    let touched = match (cleared, touched) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, b) => a.or(b),
    };
    if let Some(g) = generator.and_then(|g| d.world.generators.iter_mut().find(|x| x.id == g)) {
        g.mask_modifier = Some(id);
    }
    d.history.push(name, Change::MaskMods(before));
    if let Some(r) = touched {
        app.mask_recomposed(r);
    }
    if dropped > 0 {
        app.warn(tf("{0} pixels were left as they were, as their texture tiles already hold {1} surfaces", &[&dropped.to_string(), &limit.to_string()]));
    }
    Some(id)
}

/// Takes a generator's mask modifier away with it.
pub fn remove(app: &mut App, id: u32, label: &str) {
    let Some(d) = app.doc.as_mut() else { return };
    if d.world.mask_mods.position(id).is_none() {
        return;
    }
    let before = d.world.mask_mods.modifiers.clone();
    let touched = d.world.mask_remove(id);
    d.history.push(label, Change::MaskMods(before));
    if let Some(r) = touched {
        app.mask_recomposed(r);
    }
}

/// Stamps the chosen surface under the selected objects, or under all of them.
pub fn stamp_under_objects(app: &mut App) {
    let Some(d) = &app.doc else { return };
    let w = &d.world;
    let tool = &app.mask_tool;
    let fallback = (tool.surface < w.layers.len()).then_some(tool.surface as u8);
    if fallback.is_none() && !(tool.own_surfaces && !app.library.model_surfaces.is_empty()) {
        app.warn(t("Choose a surface first"));
        return;
    }
    let library = &app.library;
    let index = |name: &str| w.layers.iter().position(|l| l.name.eq_ignore_ascii_case(name)).map(|i| i as u8);
    let selected = tool.selection_only && !app.selection.is_empty();
    let geometry = tool.stamp.shape == StampShape::Geometry;
    // The stamps, with the model, whether it is a plant and the yaw of each.
    let mut stamps: Vec<(Stamp, String, bool, f32)> = Vec::new();
    let mut add = |o: &dz_core::objects::Obj| {
        let path = w.objects.model_path(o);
        if !w.object_visible(o) || path.contains("\\roads\\parts\\") || crate::models::is_water_model(path) || crate::models::is_decal_model(path) {
            return;
        }
        let plant = crate::viewport::is_plant(path);
        if (plant && !tool.plants) || (!plant && !tool.buildings) {
            return;
        }
        let own = if tool.own_surfaces { library.surface_of(path).and_then(index) } else { None };
        let Some(surface) = own.or(fallback) else { return };
        let footprint = (!plant && tool.footprint && !geometry).then(|| app.db.get(path)).filter(|i| i.known).map(|i| {
            let (hx, hz) = i.footprint();
            [hx * o.scale, hz * o.scale, o.yaw]
        });
        stamps.push((Stamp { x: o.x, z: o.z, surface, scale: o.scale, footprint, geometry: None }, path.to_string(), plant, o.yaw));
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
    if stamps.is_empty() {
        app.warn(t("No objects to paint under"));
        return;
    }
    let (settings, width, height, size) = (tool.stamp, w.mask.width, w.mask.height, w.meta.size_m());
    let n = w.mask_mods.modifiers.iter().filter(|m| m.kind == MaskKind::UnderObjects).count() + 1;
    let name = tf("Surface under objects {0}", &[&n.to_string()]);
    let count = stamps.len();
    let finish = move |app: &mut App, tiles| {
        if store(app, None, MaskKind::UnderObjects, name, tiles).is_some() {
            app.info(tf("Painted the surface under {0} objects", &[&count.to_string()]));
        }
    };
    if !geometry {
        let list: Vec<Stamp> = stamps.into_iter().map(|s| s.0).collect();
        let tiles = dz_core::maskmod::stamp_tiles(&list, &settings, width, height, size);
        finish(app, tiles);
        return;
    }
    // The outlines need the models, so they are read in the background.
    let db = app.db.clone();
    app.spawn(t("Painting under objects"), move |job| {
        job.stage("Reading the outlines of the models");
        // Room round an outline for the margin of the smallest object of the model.
        let mut margin: HashMap<String, f32> = HashMap::new();
        for (s, path, _, _) in &stamps {
            let m = margin.entry(path.clone()).or_insert(0.0);
            *m = m.max(settings.size_m / s.scale.max(0.05));
        }
        let mut outlines: HashMap<String, Option<std::sync::Arc<dz_core::satmod::Silhouette>>> = HashMap::new();
        let total = stamps.len().max(1);
        let mut list = Vec::with_capacity(stamps.len());
        for (i, (mut s, path, plant, yaw)) in stamps.into_iter().enumerate() {
            if i % 64 == 0 {
                if job.cancelled() {
                    anyhow::bail!("Cancelled");
                }
                job.set_progress(i as f32 / total as f32);
            }
            let outline = outlines.entry(path.clone()).or_insert_with(|| dz_core::satmod::silhouette(&db, &path, plant, margin[&path]).map(std::sync::Arc::new));
            s.geometry = outline.clone().map(|o| (o, yaw));
            list.push(s);
        }
        let tiles = dz_core::maskmod::stamp_tiles(&list, &settings, width, height, size);
        Ok(Box::new(move |app: &mut App| finish(app, tiles)) as Finish)
    });
}

/// Stamps for the trees a forest generator placed. A species with a layer of its own in the
/// forest project takes that surface, then a model with one assigned in the asset browser,
/// and the others the one chosen for the generator.
pub fn forest_stamps(world: &dz_core::world::World, library: &dz_core::library::Library, models: &[String], objects: &[dz_core::objects::Obj], ground: &dz_core::library::ForestGround, species_layers: &HashMap<String, String>) -> Vec<Stamp> {
    let index = |name: &str| world.layers.iter().position(|l| l.name.eq_ignore_ascii_case(name)).map(|i| i as u8);
    let fallback = index(&ground.surface);
    let per_model: Vec<Option<u8>> = models.iter().map(|path| {
        let stem = path.rsplit(['\\', '/']).next().unwrap_or(path).trim_end_matches(".p3d");
        species_layers.get(&forest_core::preset::ForestProject::layer_key(stem)).and_then(|l| index(l)).or_else(|| library.surface_of(path).and_then(index)).or(fallback)
    }).collect();
    objects.iter().filter_map(|o| Some(Stamp { x: o.x, z: o.z, surface: per_model.get(o.model as usize).copied().flatten()?, scale: o.scale, footprint: None, geometry: None })).collect()
}

/// The controls of the stamp settings, shared by the tool and the forest generator.
/// `geometry` offers the outline of the model as a shape.
pub fn stamp_ui(ui: &mut egui::Ui, s: &mut StampSettings, salt: &str, geometry: bool) {
    if !geometry && s.shape == StampShape::Geometry {
        s.shape = StampShape::Blob;
    }
    row(ui, &t("Patch shape"), |ui| {
        egui::ComboBox::from_id_salt(salt).selected_text(t(s.shape.name())).show_ui(ui, |ui| {
            for k in StampShape::ALL.into_iter().filter(|k| geometry || *k != StampShape::Geometry) {
                ui.selectable_value(&mut s.shape, k, t(k.name()));
            }
        });
    });
    if s.shape == StampShape::Geometry {
        ui.weak(t("The patch then follows the outline of each model seen from above, and its size is the margin round it."));
    }
    row(ui, &t("Patch size"), |ui| {
        ui.add(egui::Slider::new(&mut s.size_m, 0.5..=30.0).logarithmic(true).suffix(" m"));
    });
    row(ui, &t("Size variation"), |ui| {
        ui.add(egui::Slider::new(&mut s.randomize_size, 0.0..=0.9));
    });
    ui.checkbox(&mut s.randomize_rotation, t("Random rotation of patches"));
    ui.checkbox(&mut s.follow_scale, t("Patch grows with the object"));
}

/// The section of the paint panel that stamps a surface under objects.
pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &crate::icons::with(crate::icons::STAMP, &t("Surface under objects")), false, |ui| {
        ui.weak(t("Paints a surface under objects as a mask modifier, which can be hidden or deleted in the outliner."));
        let names: Vec<String> = app.doc.as_ref().map(|d| d.world.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default();
        row(ui, &t("Surface"), |ui| {
            let shown = names.get(app.mask_tool.surface).cloned().unwrap_or_else(|| t("(none)"));
            egui::ComboBox::from_id_salt("under_surface").selected_text(shown).show_ui(ui, |ui| {
                // Only the models with a surface of their own are then painted under.
                ui.selectable_value(&mut app.mask_tool.surface, usize::MAX, t("(none)"));
                for (i, n) in names.iter().enumerate() {
                    ui.selectable_value(&mut app.mask_tool.surface, i, n);
                }
            });
        });
        let selected = app.selection.len();
        ui.checkbox(&mut app.mask_tool.selection_only, tf("Only the selected objects ({0})", &[&selected.to_string()]));
        ui.checkbox(&mut app.mask_tool.own_surfaces, t("Use the surface assigned to each model")).on_hover_text(t("Assigned in the asset browser, with the right mouse button on a model or a folder. The surface above is for the other models."));
        ui.checkbox(&mut app.mask_tool.plants, t("Under trees and bushes"));
        ui.checkbox(&mut app.mask_tool.buildings, t("Under buildings and other objects"));
        ui.add_enabled_ui(app.mask_tool.buildings && app.mask_tool.stamp.shape != StampShape::Geometry, |ui| {
            ui.checkbox(&mut app.mask_tool.footprint, t("Fill the footprint of buildings")).on_hover_text(t("The patch size is then the margin round the footprint."));
        });
        stamp_ui(ui, &mut app.mask_tool.stamp, "under_shape", true);
        if ui.button(t("Paint under objects")).clicked() {
            stamp_under_objects(app);
        }
    });
}

/// Rows of the mask modifiers in the outliner.
pub fn outliner(app: &mut App, ui: &mut egui::Ui, filter: &str, fold: Option<bool>) {
    // In team work: only whose the outliner shows, with who made each.
    let team = crate::team_ui::active(app);
    let rows: Vec<(u32, String, bool)> = app.doc.as_ref().map(|d| d.world.mask_mods.modifiers.iter()
        .filter(|m| crate::team_ui::shown(app, d.world.mask_mod_author(m.id)))
        .map(|m| (m.id, crate::team_ui::with_author(team, &m.name, d.world.mask_mod_author(m.id)), m.enabled)).collect()).unwrap_or_default();
    let (mut toggle, mut delete) = (None, None);
    crate::panels::outliner_section(fold, "Mask modifiers", crate::icons::with(crate::icons::STACK, &tf("Mask modifiers ({0})", &[&rows.len().to_string()]))).show(ui, |ui| {
        for (id, name, enabled) in rows.iter().filter(|r| r.1.to_lowercase().contains(filter)) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if ui.add(egui::Button::new(crate::icons::EYE).small().selected(*enabled)).on_hover_text(t("Visible")).clicked() {
                    toggle = Some((*id, !*enabled));
                }
                if ui.add(egui::Button::new(crate::icons::TRASH).small()).on_hover_text(t("Delete the modifier; what it covered comes back")).clicked() {
                    delete = Some(*id);
                }
                ui.label(name).context_menu(|ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    if ui.button(t("Delete")).clicked() {
                        delete = Some(*id);
                        ui.close_menu();
                    }
                    crate::team_ui::author_menu(app, ui, dz_core::world::MASK_MOD_AUTHOR | *id as u64);
                });
            });
        }
    });
    if let Some(id) = delete {
        crate::deleting::request(app, crate::deleting::Target::MaskModifier(id));
    } else if let Some((id, on)) = toggle {
        let Some(d) = app.doc.as_mut() else { return };
        let before = d.world.mask_mods.modifiers.clone();
        let touched = d.world.mask_set_enabled(id, on);
        d.history.push(t("Mask modifier"), Change::MaskMods(before));
        if let Some(r) = touched {
            app.mask_recomposed(r);
        }
    }
}
