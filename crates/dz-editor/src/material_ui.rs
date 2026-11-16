//! The material (`.rvmat`) of a surface layer, edited in place: its colour and relief
//! textures, how often they repeat, its lighting, and its text.
use crate::app::App;
use crate::i18n::{t, tf};
use crate::icons;
use crate::theme::{self, row};
use dz_core::layer_material::{self as lm, LayerMaterial};
use eframe::egui::{self, RichText};

/// The material being edited: of which layer, as it is now, and its text when typed by hand.
pub struct MaterialEdit {
    pub layer: usize,
    pub draft: LayerMaterial,
    pub saved: LayerMaterial,
    pub raw: Option<String>,
}

const IMAGES: [&str; 7] = ["paa", "png", "tga", "jpg", "jpeg", "bmp", "tif"];

pub fn open(app: &mut App, layer: usize) {
    let root = app.settings.tools.work_drive.clone();
    let Some(l) = app.doc.as_ref().and_then(|d| d.world.layers.get(layer)) else { return };
    let draft = LayerMaterial::load(&root, &l.material).unwrap_or_else(|_| LayerMaterial::for_texture(&l.texture, None));
    app.material_edit = Some(MaterialEdit { layer, saved: draft.clone(), draft, raw: None });
}

fn own(id: &str, path: &str) -> bool {
    path.to_ascii_lowercase().starts_with(&format!("{}\\", id.to_ascii_lowercase()))
}

fn pick(title: &str) -> Option<std::path::PathBuf> {
    rfd::FileDialog::new().set_title(title).add_filter(t("Textures and images"), &IMAGES).pick_file()
}

fn color_row(ui: &mut egui::Ui, label: &str, c: &mut [f32; 4]) {
    row(ui, label, |ui| {
        for v in c.iter_mut() {
            ui.add(egui::DragValue::new(v).speed(0.01).range(0.0..=4.0).max_decimals(3));
        }
    });
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    let Some(mut ed) = app.material_edit.take() else { return };
    let root = app.settings.tools.work_drive.clone();
    let Some(doc) = &app.doc else { return };
    let id = doc.world.meta.map_id.clone();
    let Some(layer) = doc.world.layers.get(ed.layer).cloned() else { return };
    let thumb = app.layer_thumbs.get(ed.layer).cloned().flatten();
    let editable = own(&id, &layer.material);
    let stem = dz_core::surfaces::layer_stem(&layer.material);
    let dir = dz_core::surfaces::layer_dir(&id);
    let mut open = true;
    let (mut save, mut copy_in, mut revert) = (false, false, false);
    let mut error = None;
    let before = ed.draft.clone();
    egui::Window::new(tf("Material of {0}", &[&layer.name])).id(egui::Id::new("layer_material")).collapsible(false).resizable(true).default_width(520.0).open(&mut open).show(ctx, |ui| {
        ui.label(RichText::new(&layer.material).small().color(theme::TEXT_DIM));
        if !editable {
            ui.colored_label(theme::WARN, t("This material is not in the map's folder: the game's and other mods' files are not changed here."));
            if ui.button(icons::with(icons::COPY, &t("Make the map's own copy"))).on_hover_text(t("Copies the textures and the material into the map's folder of surfaces; the layer then uses the copies")).clicked() {
                copy_in = true;
            }
            ui.separator();
        }
        ui.add_enabled_ui(editable, |ui| {
            ui.label(RichText::new(t("Textures")).strong());
            ui.horizontal(|ui| {
                match &thumb {
                    Some(tex) => { ui.add(egui::Image::new((tex.id(), egui::vec2(56.0, 56.0)))); }
                    None => { ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::hover()); }
                }
                ui.vertical(|ui| {
                    ui.label(t("Colour (_ca)"));
                    ui.label(RichText::new(&ed.draft.texture).small());
                    if ui.small_button(t("Choose a file...")).on_hover_text(t("A PAA, or an image (PNG, TGA, JPG) that is converted, brought to sides of powers of two")).clicked() {
                        if let Some(f) = pick(&t("Colour texture")) {
                            let target = format!("{dir}\\{stem}_ca.paa");
                            match lm::texture_into(&f, &root.join(target.replace('\\', "/"))) {
                                Ok(()) => ed.draft.texture = target,
                                Err(e) => error = Some(format!("{e:#}")),
                            }
                        }
                    }
                });
            });
            ui.add_space(4.0);
            ui.label(t("Relief (_nopx): a normal map with the height in its alpha"));
            ui.label(RichText::new(&ed.draft.nopx).small());
            ui.horizontal_wrapped(|ui| {
                let target = format!("{dir}\\{stem}_nopx.paa");
                if ui.small_button(t("Choose a file...")).on_hover_text(t("A ready _nopx texture, or an image of one")).clicked() {
                    if let Some(f) = pick(&t("Relief texture")) {
                        match lm::texture_into(&f, &root.join(target.replace('\\', "/"))) {
                            Ok(()) => ed.draft.nopx = target.clone(),
                            Err(e) => error = Some(format!("{e:#}")),
                        }
                    }
                }
                if ui.small_button(t("Make from a normal map...")).on_hover_text(t("From a normal map (_nohq or an image) and, if you choose one next, a height map; cancel the second for none")).clicked() {
                    if let Some(n) = pick(&t("Normal map (_nohq)")) {
                        let h = pick(&t("Height map (lighter is higher) - cancel for none"));
                        match lm::make_nopx(&n, h.as_deref(), &root.join(target.replace('\\', "/"))) {
                            Ok(()) => ed.draft.nopx = target.clone(),
                            Err(e) => error = Some(format!("{e:#}")),
                        }
                    }
                }
                if ui.small_button(t("Flat")).on_hover_text(t("No relief")).clicked() {
                    ed.draft.nopx = lm::FLAT_NOPX.to_string();
                }
            });
            ui.weak(t("_smdi and _nohq are not used by the terrain: only the colour and the relief are."));
            ui.separator();
            ui.label(RichText::new(t("Look")).strong());
            row(ui, &t("Repeats"), |ui| { ui.add(egui::DragValue::new(&mut ed.draft.tiling).speed(0.1).range(0.1..=200.0)).on_hover_text(t("How many times the textures repeat in their uv space (aside and up of both stages)")); });
            row(ui, &t("Parallax"), |ui| { ui.add(egui::DragValue::new(&mut ed.draft.parallax_scale).speed(0.01).range(0.0..=4.0)); });
            row(ui, &t("Specular power"), |ui| { ui.add(egui::DragValue::new(&mut ed.draft.specular_power).speed(0.1).range(0.0..=200.0)); });
            color_row(ui, &t("Ambient"), &mut ed.draft.ambient);
            color_row(ui, &t("Diffuse"), &mut ed.draft.diffuse);
            color_row(ui, &t("Forced diffuse"), &mut ed.draft.forced_diffuse);
            color_row(ui, &t("Specular"), &mut ed.draft.specular);
            color_row(ui, &t("Emissive"), &mut ed.draft.emmisive);
            row(ui, &t("Pixel shader"), |ui| { ui.text_edit_singleline(&mut ed.draft.pixel_shader); });
            row(ui, &t("Vertex shader"), |ui| { ui.text_edit_singleline(&mut ed.draft.vertex_shader); });
            ui.separator();
            egui::CollapsingHeader::new(t("Text of the .rvmat")).id_salt("rvmat_text").show(ui, |ui| {
                let mut text = ed.raw.clone().unwrap_or_else(|| ed.draft.text());
                let r = ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_rows(14).desired_width(f32::INFINITY));
                if r.changed() {
                    // Typed by hand: saved as typed, and the fields follow it.
                    ed.draft = LayerMaterial::parse(&text);
                    ed.raw = Some(text);
                }
                ui.weak(t("Saved as typed here; changing a field above writes the text anew."));
            });
            ui.separator();
            ui.horizontal(|ui| {
                let dirty = ed.draft != ed.saved || ed.raw.is_some();
                if ui.add_enabled(dirty, egui::Button::new(icons::with(icons::FLOPPY_DISK, &t("Save")))).clicked() { save = true; }
                if ui.add_enabled(dirty, egui::Button::new(t("Revert"))).clicked() { revert = true; }
            });
        });
    });
    // A field changed: the text is written from the fields again.
    if ed.draft != before && ed.raw.as_ref().is_some_and(|r| LayerMaterial::parse(r) != ed.draft) {
        ed.raw = None;
    }
    if revert {
        ed.draft = ed.saved.clone();
        ed.raw = None;
    }
    if let Some(e) = error {
        app.error(e);
    }
    if copy_in {
        let source = root.join(layer.texture.replace('\\', "/"));
        match dz_core::surfaces::import_layer_texture(&root, &id, &source) {
            Ok((texture, material)) => {
                if let Some(d) = app.doc.as_mut() {
                    if let Some(l) = d.world.layers.get_mut(ed.layer) {
                        (l.texture, l.material) = (texture, material);
                    }
                    d.history.revision += 1;
                }
                app.reload_layer_textures();
                let layer = ed.layer;
                open_after(app, layer);
                return;
            }
            Err(e) => app.error(format!("{}: {e:#}", t("The texture could not be copied into the map"))),
        }
    }
    if save {
        let text = ed.raw.clone().unwrap_or_else(|| ed.draft.text());
        let path = root.join(layer.material.replace('\\', "/"));
        match dz_core::fsutil::atomic_write(&path, text.as_bytes()) {
            Ok(()) => {
                let texture = ed.draft.texture.trim().to_string();
                if let Some(d) = app.doc.as_mut() {
                    if let Some(l) = d.world.layers.get_mut(ed.layer) {
                        // The terrain takes the colour texture from the layer.
                        if !texture.is_empty() && !texture.eq_ignore_ascii_case(&l.texture) {
                            l.texture = texture;
                            d.history.revision += 1;
                        }
                    }
                }
                ed.saved = ed.draft.clone();
                ed.raw = None;
                app.status = tf("Saved {0}", &[&layer.material]);
                app.reload_layer_textures();
            }
            Err(e) => app.error(format!("{e:#}")),
        }
    }
    if open {
        app.material_edit = Some(ed);
    }
}

fn open_after(app: &mut App, layer: usize) {
    open(app, layer);
}
