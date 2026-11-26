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
    /// Road parts, whose whole footprint takes the surface.
    pub roads: bool,
    /// Fill the footprint of buildings instead of stamping a patch under them.
    pub footprint: bool,
    /// Models with a surface assigned in the asset browser get that one.
    pub own_surfaces: bool,
    /// With the outline of the model: only its parts above the ground count, as each object
    /// stands, so a rock sunk half into the ground leaves the outline of what shows of it.
    pub above_ground: bool,
}
impl Default for MaskTool {
    fn default() -> Self {
        Self { surface: 0, stamp: StampSettings::default(), selection_only: true, plants: true, buildings: false, roads: false, footprint: true, own_surfaces: true, above_ground: true }
    }
}

impl App {
    /// Shows a part of the mask the modifiers changed.
    pub fn mask_recomposed(&mut self, r: Rect) {
        let Some(doc) = &mut self.doc else { return };
        doc.dirty.mask = true;
        self.upload_mask(Some(r));
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
    // The stamps, with the model, whether it is a plant, and the object.
    let mut stamps: Vec<(Stamp, String, bool, dz_core::objects::Obj)> = Vec::new();
    let mut add = |o: &dz_core::objects::Obj| {
        let path = w.objects.model_path(o);
        if !w.object_visible(o) || crate::models::is_water_model(path) || crate::models::is_decal_model(path, &app.db.get(path)) {
            return;
        }
        let road = path.to_ascii_lowercase().contains("\\roads\\parts\\");
        let plant = !road && crate::viewport::is_plant(path);
        if (road && !tool.roads) || (plant && !tool.plants) || (!road && !plant && !tool.buildings) {
            return;
        }
        let own = if tool.own_surfaces { library.surface_of(path).and_then(index) } else { None };
        let Some(surface) = own.or(fallback) else { return };
        // A road part takes the surface over all of it, as a building with its footprint filled.
        let footprint = (!plant && (tool.footprint || road) && !geometry).then(|| app.db.get(path)).filter(|i| i.known).map(|i| {
            let (hx, hz) = i.footprint();
            [hx * o.scale, hz * o.scale, o.yaw]
        });
        stamps.push((Stamp { x: o.x, z: o.z, surface, scale: o.scale, footprint, geometry: None }, path.to_string(), plant, *o));
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
    // The ground the objects stand on, and whether only what is above it counts.
    let heights = std::sync::Arc::new(w.height.clone());
    let above_ground = app.mask_tool.above_ground;
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
        for (i, (mut s, path, plant, o)) in stamps.into_iter().enumerate() {
            if i % 64 == 0 {
                if job.cancelled() {
                    anyhow::bail!("Cancelled");
                }
                job.set_progress(i as f32 / total as f32);
            }
            // An object tilted or turned over, or one only above the ground counts for, gets an
            // outline of its own as it stands; one only turned about its upright shares its model's.
            let tilted = o.pitch.abs() > 0.5 || o.roll.abs() > 0.5;
            if above_ground || tilted {
                let placed = dz_core::satmod::Placed { object: &o, origin_y: dz_core::objects::ObjectStore::origin_y(&o, &heights), ground: above_ground.then_some(&*heights) };
                s.geometry = dz_core::satmod::silhouette_placed(&db, &path, plant, settings.size_m, &placed).map(|sil| (std::sync::Arc::new(sil), 0.0));
                s.scale = 1.0;
            } else {
                let outline = outlines.entry(path.clone()).or_insert_with(|| dz_core::satmod::silhouette(&db, &path, plant, margin[&path]).map(std::sync::Arc::new)).clone();
                s.geometry = outline.map(|sil| (sil, o.yaw));
            }
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
        ui.add(crate::theme::ValueField::new(&mut s.size_m, 0.5..=30.0).logarithmic(true).suffix(" m"));
    });
    row(ui, &t("Size variation"), |ui| {
        ui.add(crate::theme::ValueField::new(&mut s.randomize_size, 0.0..=0.9));
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
            // With none, only the models with a surface of their own are painted under.
            let pics: Vec<_> = app.layer_thumbs.iter().map(|t| t.as_ref().map(|t| t.id())).collect();
            let w = ui.available_width();
            crate::theme::surface_pick(ui, "under_surface", &mut app.mask_tool.surface, Some((usize::MAX, t("(none)"))), &names, &pics, |i, _| i, w);
        });
        let selected = app.selection.len();
        ui.checkbox(&mut app.mask_tool.selection_only, tf("Only the selected objects ({0})", &[&selected.to_string()]));
        ui.checkbox(&mut app.mask_tool.own_surfaces, t("Use the surface assigned to each model")).on_hover_text(t("Assigned in the asset browser, with the right mouse button on a model or a folder. The surface above is for the other models."));
        ui.checkbox(&mut app.mask_tool.plants, t("Under trees and bushes"));
        ui.checkbox(&mut app.mask_tool.buildings, t("Under buildings and other objects"));
        ui.checkbox(&mut app.mask_tool.roads, t("Under road parts")).on_hover_text(t("The whole of each road part takes the surface; the patch size is the margin round it."));
        ui.add_enabled_ui(app.mask_tool.buildings && app.mask_tool.stamp.shape != StampShape::Geometry, |ui| {
            ui.checkbox(&mut app.mask_tool.footprint, t("Fill the footprint of buildings")).on_hover_text(t("The patch size is then the margin round the footprint."));
        });
        stamp_ui(ui, &mut app.mask_tool.stamp, "under_shape", true);
        ui.add_enabled_ui(app.mask_tool.stamp.shape == StampShape::Geometry, |ui| {
            ui.checkbox(&mut app.mask_tool.above_ground, t("Only what is above the ground")).on_hover_text(t("With the outline of the model: the parts of an object sunk into the terrain leave no surface, so a rock half in the ground paints only round what shows of it, as baking into the satellite does."));
        });
        if ui.button(t("Paint under objects")).clicked() {
            stamp_under_objects(app);
        }
    });
}

fn kind_label(kind: MaskKind) -> String {
    match kind {
        MaskKind::Strokes => t("Brush"),
        MaskKind::UnderObjects => t("Under objects"),
        MaskKind::Forest => t("Forest"),
        MaskKind::Paint => t("Paint"),
        MaskKind::Satellite => t("Satellite"),
    }
}

/// Rows of the mask modifiers in the outliner (top of the list = top of the layer stack).
pub fn outliner(app: &mut App, ui: &mut egui::Ui, filter: &str, fold: Option<bool>) {
    // In team work: only whose the outliner shows, with who made each.
    let team = crate::team_ui::active(app);
    let total = app.doc.as_ref().map(|d| d.world.mask_mods.modifiers.len()).unwrap_or(0);
    let rows: Vec<(u32, usize, String, bool, MaskKind)> = app.doc.as_ref().map(|d| d.world.mask_mods.modifiers.iter().enumerate()
        .filter(|(_, m)| crate::team_ui::shown(app, d.world.mask_mod_author(m.id)))
        .map(|(idx, m)| (m.id, idx, crate::team_ui::with_author(team, &m.name, d.world.mask_mod_author(m.id)), m.enabled, m.kind)).collect()).unwrap_or_default();
    let (mut toggle, mut delete, mut merge, mut reorder, mut convert, mut rename, mut start_rename, mut cancel_rename) = (None, None, None, None::<(u32, usize)>, None::<(u32, String)>, None::<(u32, String)>, None::<(u32, String)>, false);
    let can_merge: std::collections::HashSet<u32> = app.doc.as_ref().map(|d| d.world.mask_mods.modifiers.iter().filter(|m| d.world.mask_merge_target(m.id).is_some()).map(|m| m.id).collect()).unwrap_or_default();
    crate::panels::outliner_section(fold, "Mask modifiers", crate::icons::with(crate::icons::STACK, &tf("Mask modifiers ({0})", &[&rows.len().to_string()])), rows.len() == 0).show(ui, |ui| {
        for (id, vec_idx, name, enabled, kind) in rows.iter().rev().filter(|r| r.2.to_lowercase().contains(filter)) {
            let strokes = *kind == MaskKind::Strokes;
            let disp_idx = total.saturating_sub(1).saturating_sub(*vec_idx);
            let row_resp = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if filter.is_empty() && total > 1 {
                    crate::theme::drag_grip(ui, crate::theme::OutlinerDrag::MaskMod(*id), &t("Drag to reorder"));
                }
                if crate::theme::row_switch(ui, *enabled, crate::icons::EYE, crate::icons::EYE_SLASH, t("Visible")).clicked() {
                    toggle = Some((*id, !*enabled));
                }
                if let Some((_, text)) = app.outliner_rename.as_mut().filter(|(t, _)| *t == crate::app::OutlinerRenameTarget::MaskMod(*id)) {
                    let edit = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                    edit.request_focus();
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel_rename = true;
                    } else if edit.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        rename = Some((*id, text.clone()));
                    }
                    return;
                }
                let icon = if strokes { crate::icons::PAINT_BRUSH } else { crate::icons::STACK };
                let tint = if *enabled { crate::theme::KIND_LAYER } else { crate::theme::TEXT_DIM };
                let label = if *enabled { egui::RichText::new(name.as_str()) } else { egui::RichText::new(name.as_str()).weak() };
                let badge = kind_label(*kind);
                let mut row = crate::theme::list_row(ui, strokes && app.paint.target == Some(*id), icon, tint, label, &badge);
                if strokes {
                    row = row.on_hover_text(t("Click to paint into this modifier"));
                }
                if strokes && row.clicked() {
                    app.paint.target = Some(*id);
                    app.paint.into_modifier = true;
                }
                row.context_menu(|ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    if ui.button(crate::icons::with(crate::icons::PENCIL_SIMPLE, &t("Rename"))).clicked() {
                        let raw = app.doc.as_ref().and_then(|d| d.world.mask_mods.modifiers.iter().find(|m| m.id == *id)).map(|m| m.name.clone()).unwrap_or_default();
                        start_rename = Some((*id, raw));
                        ui.close_menu();
                    }
                    if total > 1 {
                        if ui.add_enabled(*vec_idx + 1 < total, egui::Button::new(crate::icons::with(crate::icons::ARROW_UP, &t("Move up")))).clicked() {
                            reorder = Some((*id, *vec_idx + 1));
                            ui.close_menu();
                        }
                        if ui.add_enabled(*vec_idx > 0, egui::Button::new(crate::icons::with(crate::icons::ARROW_DOWN, &t("Move down")))).clicked() {
                            reorder = Some((*id, *vec_idx - 1));
                            ui.close_menu();
                        }
                        ui.separator();
                    }
                    if strokes {
                        if ui.add_enabled(can_merge.contains(id), egui::Button::new(t("Merge down"))).on_hover_text(t("Merges the modifier into the one under it, if that one is of brush strokes too and both are shown.")).on_disabled_hover_text(t("Only into a shown modifier of brush strokes right under it, and in team work only your own.")).clicked() {
                            merge = Some(Some(*id));
                            ui.close_menu();
                        }
                        if ui.add_enabled(!can_merge.is_empty(), egui::Button::new(t("Merge all brush modifiers"))).on_hover_text(t("Merges each run of neighbouring modifiers of brush strokes into one. The mask stays as it is. In team work only your own modifiers are merged.")).clicked() {
                            merge = Some(None);
                            ui.close_menu();
                        }
                        ui.separator();
                    } else {
                        if ui.button(crate::icons::with(crate::icons::PAINT_BRUSH, &t("Convert to brush..."))).on_hover_text(t("Turns this modifier into an editable brush layer. Its original settings will no longer apply.")).clicked() {
                            convert = Some((*id, name.clone()));
                            ui.close_menu();
                        }
                        ui.separator();
                    }
                    if ui.button(crate::icons::with(crate::icons::TRASH, &t("Delete"))).clicked() {
                        delete = Some(*id);
                        ui.close_menu();
                    }
                    crate::team_ui::author_menu(app, ui, dz_core::world::MASK_MOD_AUTHOR | *id as u64);
                });
            }).response;
            if filter.is_empty() && total > 1 {
                if let Some((crate::theme::OutlinerDrag::MaskMod(drag_id), after)) = crate::theme::dnd_reorder_slot(ui, row_resp.rect, |p| matches!(p, crate::theme::OutlinerDrag::MaskMod(_))) {
                    if drag_id != *id {
                        if let Some(from_vec) = app.doc.as_ref().and_then(|d| d.world.mask_mods.position(drag_id)) {
                            let from_disp = total.saturating_sub(1).saturating_sub(from_vec);
                            let to_vec = crate::theme::reorder_target(from_disp, disp_idx, after, total, true);
                            reorder = Some((drag_id, to_vec));
                        }
                    }
                }
            }
        }
    });
    if cancel_rename {
        app.outliner_rename = None;
    } else if let Some((id, new_name)) = rename {
        app.outliner_rename = None;
        let trimmed = new_name.trim();
        if !trimmed.is_empty() {
            if let Some(d) = app.doc.as_mut() {
                let before = d.world.mask_mods.modifiers.clone();
                if let Some(m) = d.world.mask_mods.modifiers.iter_mut().find(|m| m.id == id) {
                    if m.name != trimmed {
                        m.name = trimmed.to_string();
                        d.history.push(t("Rename"), Change::MaskMods(before));
                    }
                }
            }
        }
    } else if let Some((id, raw)) = start_rename {
        app.outliner_rename = Some((crate::app::OutlinerRenameTarget::MaskMod(id), raw));
    } else if let Some((id, name)) = convert {
        app.dialog = crate::app::Dialog::ConvertToBrush { kind: crate::app::ConvertKind::Mask, id, name };
    } else if let Some((id, to_idx)) = reorder {
        let Some(d) = app.doc.as_mut() else { return };
        let before = d.world.mask_mods.modifiers.clone();
        let touched = d.world.mask_move(id, to_idx);
        if d.world.mask_mods.modifiers != before {
            d.history.push(t("Reorder mask modifiers"), Change::MaskMods(before));
            if let Some(r) = touched {
                app.mask_recomposed(r);
            }
        }
    } else if let Some(which) = merge {
        let Some(d) = app.doc.as_mut() else { return };
        let before = d.world.mask_mods.modifiers.clone();
        let (n, touched) = match which {
            Some(id) => {
                let (done, r) = d.world.mask_merge_down(id);
                (done as usize, r)
            }
            None => d.world.mask_merge_brushes(),
        };
        if n == 0 {
            return;
        }
        d.history.push(t("Merge mask modifiers"), Change::MaskMods(before));
        // A modifier merged away is no longer the one the next strokes go into.
        if app.paint.target.is_some_and(|id| d.world.mask_mods.position(id).is_none()) {
            app.paint.target = None;
        }
        if let Some(r) = touched {
            app.mask_recomposed(r);
        }
        app.info(tf("{0} modifiers merged into the ones under them", &[&n.to_string()]));
    } else if let Some(id) = delete {
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
