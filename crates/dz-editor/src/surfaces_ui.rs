//! The map's own surfaces (CfgSurfaces): which one each layer gets in the game, new surfaces
//! for textures of the map, and changes to those of the game.
use crate::app::App;
use crate::i18n::{t, tf};
use crate::icons;
use crate::theme::{self, row, section};
use dz_core::surfaces::{surface_of, ClutterPick, MapSurface, SurfaceCatalog};
use eframe::egui::{self, RichText};

/// Fields shown in the editor: config name, label, whether it is text.
const FIELDS: [(&str, &str, bool); 13] = [
    ("soundEnviron", "Footstep group (soundEnviron)", true),
    ("soundHit", "Hit sound", true),
    ("impact", "Bullet impact", true),
    ("vpSurface", "Vehicle surface", true),
    ("wheelParticle", "Wheel dust", true),
    ("friction", "Friction", false),
    ("restitution", "Bounciness", false),
    ("deflection", "Deflection", false),
    ("audibility", "Audibility", false),
    ("footDamage", "Damage to shoes", false),
    ("isDigable", "Can be dug (0 or 1)", false),
    ("isFertile", "Plants grow (0 or 1)", false),
    ("rough", "Roughness", false),
];

/// Values the game uses for a field, for the menu of choices.
fn values_of(cat: &SurfaceCatalog, key: &str) -> Vec<String> {
    let mut v: Vec<String> = cat.surfaces.iter().filter_map(|s| s.props.get(key).cloned()).filter(|v| !v.trim().is_empty()).collect();
    v.sort_by_key(|s| s.to_ascii_lowercase());
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    v
}

/// A surface of the game picked from a menu with a search.
fn surface_pick(ui: &mut egui::Ui, id: &str, cat: &SurfaceCatalog, value: &mut String, none: Option<&str>) -> bool {
    let mut changed = false;
    let shown = if value.is_empty() { none.map(t).unwrap_or_default() } else { value.clone() };
    egui::ComboBox::from_id_salt(id).selected_text(shown).width(180.0).height(320.0).show_ui(ui, |ui| {
        let sid = ui.make_persistent_id((id, "search"));
        let mut search: String = ui.data_mut(|d| d.get_temp(sid).unwrap_or_default());
        ui.add(egui::TextEdit::singleline(&mut search).hint_text(t("Search...")).desired_width(170.0));
        ui.data_mut(|d| d.insert_temp(sid, search.clone()));
        let needle = search.to_ascii_lowercase();
        if let Some(n) = none {
            if ui.selectable_label(value.is_empty(), t(n)).clicked() {
                value.clear();
                changed = true;
            }
        }
        for s in cat.terrain_surfaces().filter(|s| needle.is_empty() || s.class.to_ascii_lowercase().contains(&needle)) {
            let r = ui.selectable_label(value.eq_ignore_ascii_case(&s.class), &s.class).on_hover_text(format!("{}*  ({})", s.files, s.addon));
            if r.clicked() {
                *value = s.class.clone();
                changed = true;
            }
        }
    });
    changed
}

/// The panel in the paint mode, under the surface layers.
pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    let cat = app.surface_catalog.clone();
    let Some(doc) = &mut app.doc else { return };
    let layer = doc.world.layers.get(app.paint.layer).cloned();
    let mut changed = false;
    let mut open_editor = None;
    let mut read_cfg: Option<std::path::PathBuf> = None;
    section(ui, &icons::with(icons::FOOTPRINTS, &t("Surfaces in the game")), true, |ui| {
        if cat.surfaces.is_empty() {
            ui.colored_label(theme::WARN, t("No surfaces of the game were found on the work drive"));
        }
        let own = &mut doc.world.meta.config.surfaces;
        if let Some(l) = &layer {
            let (mine, game) = surface_of(&l.texture, own, &cat);
            let mine = mine.and_then(|m| own.iter().position(|o| std::ptr::eq(o, m)));
            ui.label(RichText::new(tf("Layer {0}", &[&l.name])).strong());
            match (mine, game) {
                (Some(i), _) => {
                    let s = &own[i];
                    ui.horizontal_wrapped(|ui| {
                        ui.label(if s.replaces { tf("{0} of the game, changed for this map", &[&s.base]) } else { tf("Surface of the map {0}, based on {1}", &[&s.name, &s.base]) });
                        if ui.small_button(icons::with(icons::PENCIL_SIMPLE, &t("Edit"))).clicked() { open_editor = Some(i); }
                    });
                }
                (None, Some(g)) => {
                    ui.label(tf("Surface of the game {0}", &[&g.class]));
                    ui.horizontal_wrapped(|ui| {
                        if ui.small_button(t("Change it for this map")).on_hover_text(t("Writes the class of the game again in the map's config, with the changes: everywhere the mod is loaded.")).clicked() {
                            own.push(MapSurface { base: g.class.clone(), replaces: true, ..Default::default() });
                            open_editor = Some(own.len() - 1);
                            changed = true;
                        }
                        if ui.small_button(t("A new surface for this texture")).clicked() {
                            own.push(new_for(&l.texture, &g.class));
                            open_editor = Some(own.len() - 1);
                            changed = true;
                        }
                    });
                }
                (None, None) => {
                    ui.colored_label(theme::WARN, t("No surface of the game is named like this texture: in the game it has no grass, and the sounds and physics of the default ground."));
                    if ui.button(icons::with(icons::PLUS, &t("Make a surface for this layer"))).clicked() {
                        own.push(new_for(&l.texture, "en_grass1"));
                        open_editor = Some(own.len() - 1);
                        changed = true;
                    }
                }
            }
            ui.separator();
        }
        if own.is_empty() {
            ui.weak(t("The map has no surfaces of its own."));
        }
        let mut remove = None;
        for (i, s) in own.iter().enumerate() {
            ui.horizontal(|ui| {
                let name = if s.replaces { format!("{} ({})", s.base, t("changed")) } else { s.name.clone() };
                let selected = app.surface_edit == Some(i);
                if ui.selectable_label(selected, name).on_hover_text(if s.replaces { String::new() } else { format!("{}*  ← {}", s.files, s.base) }).clicked() {
                    open_editor = Some(i);
                }
                if ui.small_button(icons::TRASH).on_hover_text(t("Delete")).clicked() { remove = Some(i); }
            });
        }
        if let Some(i) = remove {
            own.remove(i);
            changed = true;
            app.surface_edit = None;
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button(icons::with(icons::PLUS, &t("New surface"))).clicked() {
                let n = own.len() + 1;
                own.push(MapSurface { name: format!("{}_surface{n}", doc.world.meta.map_id.to_ascii_lowercase()), files: String::new(), base: "en_grass1".into(), ..Default::default() });
                open_editor = Some(own.len() - 1);
                changed = true;
            }
            ui.menu_button(t("Change a surface of the game"), |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                let sid = ui.make_persistent_id("replace_search");
                let mut search: String = ui.data_mut(|d| d.get_temp(sid).unwrap_or_default());
                ui.add(egui::TextEdit::singleline(&mut search).hint_text(t("Search...")).desired_width(200.0));
                ui.data_mut(|d| d.insert_temp(sid, search.clone()));
                let needle = search.to_ascii_lowercase();
                let mut picked = None;
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for g in cat.terrain_surfaces().filter(|g| needle.is_empty() || g.class.to_ascii_lowercase().contains(&needle)) {
                        if ui.button(format!("{}   ({})", g.class, g.addon)).clicked() { picked = Some(g.class.clone()); }
                    }
                });
                if let Some(pick) = picked {
                    if let Some(i) = own.iter().position(|s| s.replaces && s.base.eq_ignore_ascii_case(&pick)) {
                        open_editor = Some(i);
                    } else {
                        own.push(MapSurface { base: pick, replaces: true, ..Default::default() });
                        open_editor = Some(own.len() - 1);
                        changed = true;
                    }
                    ui.close_menu();
                }
            });
        });
        if ui.button(icons::with(icons::FILE_ARROW_DOWN, &t("Read the map's CfgSurfaces..."))).on_hover_text(t("Takes the surfaces a config of the map defines (CfgSurfaces) into the project; they are written again on export.")).clicked() {
            let root = app.settings.tools.work_drive.clone();
            if let Some(f) = rfd::FileDialog::new().add_filter("Config", &["cpp", "hpp"]).set_directory(&root).pick_file() {
                read_cfg = Some(f);
            }
        }
        crate::panels::about(ui, &t("New surfaces are written to surfaces/config.cpp of the map on export, with the footsteps of the surface they sound like."));
    });
    if let Some(i) = open_editor {
        app.surface_edit = Some(i);
    }
    let mut add_layer = None;
    if let Some(i) = app.surface_edit {
        let mut open = true;
        let ctx = ui.ctx().clone();
        let sounds = app.step_sounds.get();
        let layers: Vec<(String, String, Option<egui::TextureHandle>)> = doc.world.layers.iter().enumerate().map(|(k, l)| (l.name.clone(), l.texture.clone(), app.layer_thumbs.get(k).cloned().flatten())).collect();
        let thumbs = &mut app.thumbs;
        if let Some(s) = doc.world.meta.config.surfaces.get_mut(i) {
            let before = s.clone();
            egui::Window::new(t("Surface")).id(egui::Id::new("surface_editor")).collapsible(false).resizable(true).default_width(430.0).open(&mut open).show(&ctx, |ui| {
                if egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| editor(ui, s, &cat, sounds, &layers, thumbs)).inner {
                    add_layer = Some(i);
                }
            });
            if *s != before {
                changed = true;
            }
        } else {
            open = false;
        }
        if !open {
            app.surface_edit = None;
        }
    }
    if let Some(f) = read_cfg {
        match dz_core::import::read_surfaces_cfg(&mut doc.world, &f, &app.settings.tools.work_drive) {
            Ok(0) => app.warn(t("The config defines no surfaces")),
            Ok(n) => {
                changed = true;
                app.info(tf("{0} surfaces of the map read from {1}", &[&n.to_string(), &f.display().to_string()]));
            }
            Err(e) => app.error(format!("{e:#}")),
        }
    }
    if changed {
        if let Some(d) = &mut app.doc { d.history.revision += 1; }
        app.reload_clutter();
    }
    if let Some(i) = add_layer {
        let root = app.settings.tools.work_drive.clone();
        let Some(file) = rfd::FileDialog::new().add_filter(t("Textures and images"), &["paa", "png", "tga", "jpg", "jpeg", "bmp", "tif"]).set_directory(&root).pick_file() else { return };
        let Some(texture) = crate::panels::add_texture_layer(app, &file) else { return };
        let stem = dz_core::surfaces::layer_stem(&texture);
        if let Some(s) = app.doc.as_mut().and_then(|d| d.world.meta.config.surfaces.get_mut(i)) {
            // The surface goes to the new layer, unless it already does.
            if !s.replaces && !s.matches(&texture) {
                s.files = stem.clone();
                if s.name.trim().is_empty() || s.name.contains("_surface") {
                    s.name = stem;
                }
            }
        }
        if let Some(d) = app.doc.as_mut() {
            d.history.revision += 1;
        }
        app.reload_clutter();
    }
}

fn new_for(texture: &str, base: &str) -> MapSurface {
    let stem = dz_core::surfaces::texture_stem(texture);
    let files = stem.trim_end_matches(".paa").trim_end_matches("_ca").trim_end_matches("_co").to_string();
    MapSurface { name: files.clone(), files, base: base.to_string(), ..Default::default() }
}

/// A small picture of a clutter model; a larger one with its file under the mouse.
fn thumb(ui: &mut egui::Ui, thumbs: &mut crate::models::Thumbnails, model: &str, side: f32, name: &str) {
    let tex = if model.is_empty() { None } else { thumbs.get(model).cloned() };
    let r = match &tex {
        Some(t) => ui.add(egui::Image::new((t.id(), egui::vec2(side, side))).sense(egui::Sense::hover())),
        None => {
            let (rect, r) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
            ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgb(48, 48, 50));
            r
        }
    };
    let file = model.rsplit('\\').next().unwrap_or(model).to_string();
    r.on_hover_ui(|ui| {
        if let Some(t) = &tex {
            ui.add(egui::Image::new((t.id(), egui::vec2(160.0, 160.0))));
        }
        ui.label(RichText::new(name).strong());
        ui.label(RichText::new(file).small().color(theme::TEXT_DIM));
    });
}

/// The layers a surface goes to: their texture, name and the texture's path.
fn previews(ui: &mut egui::Ui, layers: &[&(String, String, Option<egui::TextureHandle>)]) {
    ui.weak(t("Layers with this surface:"));
    for (name, texture, thumb) in layers {
        ui.horizontal(|ui| {
            match thumb {
                Some(tex) => { ui.add(egui::Image::new((tex.id(), egui::vec2(40.0, 40.0)))); }
                None => { ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover()); }
            }
            ui.vertical(|ui| {
                ui.label(RichText::new(name.as_str()).strong());
                ui.label(RichText::new(texture.as_str()).small().color(theme::TEXT_DIM));
            });
        });
    }
}

/// Returns true when a layer with a new texture is asked for.
fn editor(ui: &mut egui::Ui, s: &mut MapSurface, cat: &SurfaceCatalog, sounds: Option<&dz_core::surfaces::StepSounds>, layers: &[(String, String, Option<egui::TextureHandle>)], thumbs: &mut crate::models::Thumbnails) -> bool {
    let mut add_layer = false;
    let base = cat.get(&s.base).cloned();
    if s.replaces {
        ui.label(RichText::new(tf("{0} of the game", &[&s.base])).strong());
        ui.weak(t("The class of the game is written again with these changes; they apply wherever the map's mod is loaded."));
        let files = cat.get(&s.base).map(|g| g.files.clone()).unwrap_or_default();
        let users: Vec<_> = layers.iter().filter(|l| !files.is_empty() && dz_core::surfaces::texture_stem(&l.1).starts_with(&files)).collect();
        if !users.is_empty() {
            previews(ui, &users);
        }
    } else {
        row(ui, &t("Class name"), |ui| {
            let mut name = s.name.clone();
            if ui.text_edit_singleline(&mut name).changed() {
                s.name = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            }
        });
        row(ui, &t("Textures named"), |ui| {
            ui.add(egui::TextEdit::singleline(&mut s.files).hint_text("my_grass").desired_width(150.0));
            ui.label("*");
        });
        ui.weak(t("Layers whose colour texture's file name starts with this get the surface (my_grass for my_grass_ca.paa)."));
        let users: Vec<_> = layers.iter().filter(|l| s.matches(&l.1)).collect();
        if users.is_empty() {
            ui.colored_label(theme::WARN, t("No layer of the project has a texture named like this."));
        } else {
            previews(ui, &users);
        }
        if ui.button(icons::with(icons::PLUS, &t("Add a layer with a texture..."))).on_hover_text(t("Copies the texture into the map's folder, adds it as a layer and names the surface after it")).clicked() {
            add_layer = true;
        }
        row(ui, &t("Based on"), |ui| {
            surface_pick(ui, "surface_base", cat, &mut s.base, None);
        });
        ui.weak(t("Everything not set here comes from this surface of the game."));
        ui.separator();
        ui.label(RichText::new(icons::with(icons::FOOTPRINTS, &t("Footsteps"))).strong());
        row(ui, &t("Sounds like"), |ui| {
            surface_pick(ui, "surface_sounds", cat, &mut s.sounds, Some("(as the base)"));
        });
        match sounds {
            Some(st) => {
                let n = st.count(s.sound_source());
                if n == 0 {
                    ui.colored_label(theme::WARN, t("That surface has no footsteps in the sound tables of the game; pick another."));
                } else {
                    ui.weak(tf("Its sounds are copied into {0} step sound tables (walking, running, animals...).", &[&n.to_string()]));
                }
            }
            None => {
                ui.weak(t("Reading the sound tables of the game..."));
            }
        }
    }
    ui.separator();
    ui.label(RichText::new(t("Physics and sounds")).strong());
    for (key, label, text) in FIELDS {
        let base_value = base.as_ref().and_then(|b| b.props.get(key)).cloned().unwrap_or_default();
        let mut own = s.props.contains_key(key);
        row(ui, &t(label), |ui| {
            if ui.checkbox(&mut own, "").on_hover_text(t("Set for this surface; off keeps the value of the base")).changed() {
                if own {
                    s.props.insert(key.to_string(), base_value.clone());
                } else {
                    s.props.remove(key);
                }
            }
            match s.props.get_mut(key) {
                Some(v) if text => {
                    let values = values_of(cat, key);
                    egui::ComboBox::from_id_salt(("surface_field", key)).selected_text(v.clone()).width(150.0).show_ui(ui, |ui| {
                        for x in values {
                            ui.selectable_value(v, x.clone(), x);
                        }
                    });
                }
                Some(v) => {
                    let mut n: f32 = v.trim().parse().unwrap_or(0.0);
                    if ui.add(egui::DragValue::new(&mut n).speed(0.01).range(0.0..=10.0).max_decimals(3)).changed() {
                        *v = format!("{}", (n * 1000.0).round() / 1000.0);
                    }
                }
                None => {
                    ui.weak(if base_value.is_empty() { "-".to_string() } else { base_value.clone() });
                }
            }
        });
    }
    let mut own_visible = s.visible.is_some();
    row(ui, &t("Visibility"), |ui| {
        if ui.checkbox(&mut own_visible, "").on_hover_text(t("How well someone lying, kneeling and standing on it is seen")).changed() {
            s.visible = own_visible.then(|| base.as_ref().map_or([1.0; 3], |b| b.visible));
        }
        match &mut s.visible {
            Some(v) => {
                for x in v.iter_mut() {
                    ui.add(egui::DragValue::new(x).speed(0.01).range(0.0..=1.0).max_decimals(2));
                }
            }
            None => {
                let v = base.as_ref().map_or([1.0; 3], |b| b.visible);
                ui.weak(format!("{} / {} / {}", v[0], v[1], v[2]));
            }
        }
    });
    ui.separator();
    ui.label(RichText::new(icons::with(icons::PLANT, &t("Grass"))).strong());
    let base_char = base.as_ref().and_then(|b| b.props.get("character")).map(|c| cat.character(c)).unwrap_or_default();
    let mut own_grass = s.clutter.is_some();
    ui.horizontal(|ui| {
        if ui.radio(!own_grass, tf("As the base ({0} kinds)", &[&base_char.len().to_string()])).clicked() {
            own_grass = false;
        }
        if ui.radio(own_grass, t("Its own")).clicked() {
            own_grass = true;
        }
    });
    if own_grass != s.clutter.is_some() {
        s.clutter = own_grass.then(|| base_char.iter().map(|(n, p)| ClutterPick { name: n.clone(), probability: *p }).collect());
    }
    match &mut s.clutter {
        Some(list) => {
            let mut remove = None;
            for (k, c) in list.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let model = cat.clutter_kind(&c.name).map(|k| k.model.clone()).unwrap_or_default();
                    thumb(ui, thumbs, &model, 28.0, &c.name);
                    egui::ComboBox::from_id_salt(("clutter_pick", k)).selected_text(c.name.clone()).width(200.0).height(420.0).show_ui(ui, |ui| {
                        let sid = ui.make_persistent_id(("clutter_search", k));
                        let mut search: String = ui.data_mut(|d| d.get_temp(sid).unwrap_or_default());
                        ui.add(egui::TextEdit::singleline(&mut search).hint_text(t("Search...")).desired_width(190.0));
                        ui.data_mut(|d| d.insert_temp(sid, search.clone()));
                        let needle = search.to_ascii_lowercase();
                        for kind in cat.clutter.iter().filter(|k| needle.is_empty() || k.name.to_ascii_lowercase().contains(&needle) || k.model.contains(&needle)) {
                            let picked = ui.horizontal(|ui| {
                                thumb(ui, thumbs, &kind.model, 40.0, &kind.name);
                                ui.selectable_label(c.name.eq_ignore_ascii_case(&kind.name), &kind.name).clicked()
                            }).inner;
                            if picked {
                                c.name = kind.name.clone();
                            }
                        }
                    });
                    ui.add(egui::DragValue::new(&mut c.probability).speed(0.005).range(0.0..=1.0).max_decimals(3));
                    if ui.small_button(icons::TRASH).clicked() { remove = Some(k); }
                });
            }
            if let Some(k) = remove { list.remove(k); }
            if ui.button(icons::with(icons::PLUS, &t("Add grass"))).clicked() {
                let name = cat.clutter.first().map(|k| k.name.clone()).unwrap_or_default();
                list.push(ClutterPick { name, probability: 0.1 });
            }
            let sum: f32 = list.iter().map(|c| c.probability).sum();
            if sum > 1.0001 {
                ui.colored_label(theme::WARN, tf("The odds add up to {0}; over 1 the last kinds never grow.", &[&format!("{sum:.2}")]));
            } else {
                ui.weak(tf("The odds add up to {0}; the rest of the ground stays bare.", &[&format!("{sum:.2}")]));
            }
        }
        None => {
            ui.weak(base_char.iter().map(|(n, p)| format!("{n} {p}")).collect::<Vec<_>>().join(", "));
        }
    }
    add_layer
}
