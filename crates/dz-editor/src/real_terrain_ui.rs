//! The real place a new project is made from: found by name or coordinates, chosen on a
//! satellite picture, then downloaded with `dz_core::real_terrain`.
use crate::app::{App, Finish, NewProject, SIZE_PRESETS};
use crate::i18n::{t, tf};
use crate::theme::row;
use dz_core::project::{self, SaveParts};
use dz_core::real_terrain::{self as rt, OsmParts, Place, RealMap, TileSource};
use dz_core::terrain::RgbImage;
use eframe::egui::{self, Color32, Stroke};
use std::sync::mpsc::{channel, Receiver};

/// Pixels per side of the picture the place is chosen on.
const PREVIEW_PX: usize = 400;
/// Metres per pixel of the satellite picture to choose from.
const PIXEL_SIZES: [f32; 5] = [0.5, 1.0, 2.0, 4.0, 10.0];

/// Latitude, longitude, metres per pixel and source of a picture to choose on.
type Preview = (f64, f64, f64, usize);

pub struct RealPlace {
    pub search: String,
    results: Vec<Place>,
    searching: Option<Receiver<Result<Vec<Place>, String>>>,
    pub lat: f64,
    pub lon: f64,
    /// Index into the satellite sources; one past them is the user's own server.
    pub source: usize,
    pub custom_url: String,
    pub satellite: bool,
    pub mpp: f32,
    pub heights: bool,
    pub shapes: bool,
    pub osm: OsmParts,
    /// The surface mask is read from the satellite picture, as `classify` says.
    pub mask: bool,
    pub classify: dz_core::satgen::SatClassify,
    class_view: crate::satellite::ClassifyView,
    /// The OpenStreetMap shapes are painted into the mask, over what the picture gave.
    pub shape_mask_on: bool,
    pub shape_mask: rt::ShapeMask,
    /// Generators are added for the shapes.
    pub generators_on: bool,
    pub generators: rt::OsmGenerators,
    /// The last picture to choose on, and the part of it under the map with its revision: what
    /// the preview of the surfaces is made from.
    last_img: Option<(RgbImage, Preview)>,
    crop: Option<(Preview, u64, RgbImage)>,
    crop_rev: u64,
    /// Metres per pixel of the picture the place is chosen on; 0 until it is first shown.
    view_mpp: f64,
    texture: Option<(egui::TextureHandle, Preview)>,
    loading: Option<(Receiver<Result<RgbImage, String>>, Preview)>,
    error: Option<String>,
    /// The picture that could not be downloaded, so it is not asked for again and again.
    failed: Option<Preview>,
}

impl Default for RealPlace {
    fn default() -> Self {
        Self {
            search: String::new(),
            results: Vec::new(),
            searching: None,
            lat: 49.27,
            lon: 19.98,
            source: 0,
            custom_url: String::new(),
            satellite: true,
            mpp: 1.0,
            heights: true,
            shapes: true,
            osm: OsmParts::default(),
            mask: true,
            classify: rt::default_classify(),
            class_view: Default::default(),
            shape_mask_on: true,
            shape_mask: rt::ShapeMask::for_layers(&dz_core::world::default_layers()),
            generators_on: true,
            generators: Default::default(),
            last_img: None,
            crop: None,
            crop_rev: 0,
            view_mpp: 0.0,
            texture: None,
            loading: None,
            error: None,
            failed: None,
        }
    }
}

/// Downloads go through the tile cache in the user's data folder.
fn fetcher() -> anyhow::Result<rt::Fetcher> {
    rt::Fetcher::new(Some(crate::settings::data_dir().join("tiles")))
}

impl RealPlace {
    /// The part of the last picture under the map square, while the picture is of the place
    /// as it is now chosen; made again when that changes.
    fn crop(&mut self, size_m: f64) -> Option<(&RgbImage, u64)> {
        let (img, at) = self.last_img.as_ref()?;
        if (at.0, at.1, at.2) != (self.lat, self.lon, self.view_mpp) {
            return self.crop.as_ref().map(|c| (&c.2, c.1));
        }
        let key = (at.0, at.1, at.2, (size_m * 100.0) as usize);
        if self.crop.as_ref().map(|c| c.0) != Some(key) {
            let side = ((size_m / at.2).round() as usize).clamp(8, img.width.min(img.height));
            let (x0, y0) = ((img.width - side) / 2, (img.height - side) / 2);
            let mut out = RgbImage::new(side, side, [0, 0, 0]);
            for y in 0..side {
                let from = ((y0 + y) * img.width + x0) * 3;
                out.data[y * side * 3..(y + 1) * side * 3].copy_from_slice(&img.data[from..from + side * 3]);
            }
            self.crop_rev += 1;
            self.crop = Some((key, self.crop_rev, out));
        }
        self.crop.as_ref().map(|c| (&c.2, c.1))
    }

    fn source(&self) -> TileSource {
        rt::satellite_sources().get(self.source).cloned().unwrap_or_else(|| TileSource { name: "Custom".into(), url: self.custom_url.trim().to_string(), max_zoom: 19, attribution: String::new() })
    }

    fn map(&self, p: &NewProject) -> RealMap {
        let (n, cell) = SIZE_PRESETS[p.preset];
        RealMap {
            name: p.name.clone(),
            lat: self.lat,
            lon: self.lon,
            terrain_size: n,
            cell,
            satellite: self.satellite.then(|| (self.source(), self.mpp)),
            heights: self.heights,
            osm: self.shapes.then_some(self.osm),
            mask: (self.satellite && self.mask).then(|| self.classify.clone()),
            shape_mask: (self.shapes && self.shape_mask_on).then(|| self.shape_mask.clone()),
            generators: (self.shapes && self.generators_on).then(|| self.generators.clone()),
        }
    }

    /// Takes in what the threads finished and starts the next picture when the view moved.
    fn poll(&mut self, ctx: &egui::Context, dragging: bool) {
        if let Some(rx) = &self.searching {
            match rx.try_recv() {
                Ok(Ok(found)) => {
                    if let Some(first) = found.first() {
                        (self.lat, self.lon) = (first.lat, first.lon);
                    }
                    if found.is_empty() {
                        self.error = Some(t("Nothing was found by that name"));
                    }
                    self.results = found;
                    self.searching = None;
                }
                Ok(Err(e)) => {
                    self.error = Some(e);
                    self.searching = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(_) => self.searching = None,
            }
        }
        if let Some((rx, at)) = &self.loading {
            match rx.try_recv() {
                Ok(Ok(img)) => {
                    let image = egui::ColorImage::from_rgb([img.width, img.height], &img.data);
                    let at = *at;
                    match &mut self.texture {
                        Some((tex, was)) => {
                            tex.set(image, egui::TextureOptions::LINEAR);
                            *was = at;
                        }
                        None => self.texture = Some((ctx.load_texture("real_place_preview", image, egui::TextureOptions::LINEAR), at)),
                    }
                    self.last_img = Some((img, at));
                    self.loading = None;
                }
                Ok(Err(e)) => {
                    self.error = Some(e);
                    self.failed = Some(*at);
                    self.loading = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(_) => self.loading = None,
            }
        }
        let want = (self.lat, self.lon, self.view_mpp, self.source);
        let shown = self.texture.as_ref().map(|t| t.1);
        if self.loading.is_none() && !dragging && shown != Some(want) && self.failed != Some(want) {
            let (tx, rx) = channel();
            let src = self.source();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let r = fetcher().and_then(|f| rt::preview(&f, &src, want.0, want.1, want.2, PREVIEW_PX, &dz_core::Job::default()));
                let _ = tx.send(r.map_err(|e| format!("{e:#}")));
                ctx.request_repaint();
            });
            self.loading = Some((rx, want));
        }
    }

    fn start_search(&mut self, ctx: &egui::Context) {
        let q = self.search.trim().to_string();
        if q.is_empty() || self.searching.is_some() {
            return;
        }
        self.error = None;
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = fetcher().and_then(|f| rt::search_places(&f, &q));
            let _ = tx.send(r.map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
        self.searching = Some(rx);
    }

    /// The picture with the square of the map on it; drag to move, scroll to zoom,
    /// double-click to put the map there.
    fn picture(&mut self, ui: &mut egui::Ui, size_m: f64) -> bool {
        if self.view_mpp <= 0.0 {
            self.view_mpp = size_m * 1.6 / PREVIEW_PX as f64;
        }
        let side = PREVIEW_PX as f32;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag());
        let view = rt::GeoArea { lat: self.lat, lon: self.lon, size_m: self.view_mpp * PREVIEW_PX as f64 };
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(20, 22, 24));
        if let Some((tex, (lat, lon, mpp, _))) = &self.texture {
            // Where the picture is now that the view has moved on since it was made.
            let c = view.to_map(*lon, *lat);
            let k = (*mpp / self.view_mpp) as f32;
            let centre = rect.center() + egui::vec2(((c[0] - view.size_m / 2.0) / self.view_mpp) as f32, -((c[1] - view.size_m / 2.0) / self.view_mpp) as f32);
            let r = egui::Rect::from_center_size(centre, egui::vec2(side * k, side * k));
            painter.image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        let map = (size_m / self.view_mpp) as f32;
        let square = egui::Rect::from_center_size(rect.center(), egui::vec2(map, map));
        painter.rect_filled(square, 0.0, Color32::from_rgba_unmultiplied(0, 112, 224, 24));
        painter.rect_stroke(square, 0.0, Stroke::new(2.0_f32, crate::theme::ACCENT), egui::StrokeKind::Middle);
        if self.loading.is_some() {
            painter.text(rect.left_bottom() + egui::vec2(6.0, -6.0), egui::Align2::LEFT_BOTTOM, t("Loading..."), egui::FontId::proportional(12.0), Color32::from_gray(220));
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            let (lon, lat) = view.to_lonlat(view.size_m / 2.0 - d.x as f64 * self.view_mpp, view.size_m / 2.0 + d.y as f64 * self.view_mpp);
            (self.lat, self.lon) = (lat.clamp(-84.0, 84.0), lon);
        }
        if resp.double_clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let d = p - rect.center();
                let (lon, lat) = view.to_lonlat(view.size_m / 2.0 + d.x as f64 * self.view_mpp, view.size_m / 2.0 - d.y as f64 * self.view_mpp);
                (self.lat, self.lon) = (lat.clamp(-84.0, 84.0), lon);
            }
        }
        if resp.hovered() {
            let s = ui.input(|i| i.smooth_scroll_delta.y);
            if s != 0.0 {
                self.view_mpp = (self.view_mpp * (-s as f64 * 0.003).exp()).clamp(1.0, 5000.0);
            }
        }
        resp.on_hover_text(t("Drag to move, scroll to zoom, double-click to put the map there.")).dragged()
    }
}

/// Licence notes of the satellite sources, in the order of `satellite_sources`.
fn licence_note(source: usize) -> String {
    match source {
        0 => t("Sharp (about 0.5 to 1 m), but Esri's terms do not allow publishing it in a map. Use it to trace and for private tests."),
        1 => t("10 m pixels. CC BY 4.0: may be published, if the attribution is given."),
        2 => t("Poland only. Sharp; open data that may be used in a map, if GUGiK is named as the source."),
        3 => t("Czechia only. Sharp; CC BY 4.0: may be published, if ČÚZK is named."),
        4 => t("Austria only. 30 cm; CC BY 4.0: may be published, if basemap.at is named."),
        5 => t("USA only. About 1 m; public domain."),
        _ => t("Check that the server's licence allows using its pictures in a map."),
    }
}

/// The part of the New project window for a real place.
pub fn ui(ui: &mut egui::Ui, p: &mut NewProject) {
    let (n, cell) = SIZE_PRESETS[p.preset];
    let size_m = n as f64 * cell as f64;
    let ctx = ui.ctx().clone();
    let place = &mut p.place;
    let mut dragging = false;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            dragging = place.picture(ui, size_m);
            let src = place.source();
            if !src.attribution.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new(format!("© {}", src.attribution)).small().weak()).wrap()).on_hover_text(src.name);
            }
        });
        egui::ScrollArea::vertical().id_salt("real_place_settings").max_height(ui.ctx().screen_rect().height() * 0.7).show(ui, |ui| ui.vertical(|ui| {
            ui.set_width(330.0);
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut place.search).hint_text(t("Place name, or latitude, longitude")).desired_width(250.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.add_enabled(place.searching.is_none(), egui::Button::new(crate::icons::with(crate::icons::MAGNIFYING_GLASS, &t("Find")))).clicked() || enter {
                    place.start_search(&ctx);
                }
            });
            if !place.results.is_empty() {
                egui::ScrollArea::vertical().max_height(90.0).id_salt("real_place_results").show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    for r in place.results.clone() {
                        let on = (r.lat - place.lat).abs() < 1e-9 && (r.lon - place.lon).abs() < 1e-9;
                        if ui.selectable_label(on, &r.name).on_hover_text(&r.name).clicked() {
                            (place.lat, place.lon) = (r.lat, r.lon);
                        }
                    }
                });
            }
            ui.add_space(4.0);
            row(ui, &t("Latitude"), |ui| { ui.add(egui::DragValue::new(&mut place.lat).speed(0.001).range(-84.0..=84.0).max_decimals(5).suffix("°")); });
            row(ui, &t("Longitude"), |ui| { ui.add(egui::DragValue::new(&mut place.lon).speed(0.001).range(-180.0..=180.0).max_decimals(5).suffix("°")); });
            row(ui, &t("Size"), |ui| {
                let text = |i: usize| { let (n, c) = SIZE_PRESETS[i]; format!("{n} × {c} m  =  {} m", n as f32 * c) };
                egui::ComboBox::from_id_salt("real_size").width(200.0).selected_text(text(p.preset)).show_ui(ui, |ui| {
                    for i in 0..SIZE_PRESETS.len() { ui.selectable_value(&mut p.preset, i, text(i)); }
                });
            });
            ui.separator();
            ui.checkbox(&mut place.satellite, t("Satellite picture"));
            ui.add_enabled_ui(place.satellite, |ui| {
                let sources = rt::satellite_sources();
                row(ui, &t("Source"), |ui| {
                    let name = |i: usize| sources.get(i).map(|s| s.name.clone()).unwrap_or_else(|| t("Your own tile server"));
                    let before = place.source;
                    egui::ComboBox::from_id_salt("real_source").width(200.0).selected_text(name(place.source)).show_ui(ui, |ui| {
                        for i in 0..=sources.len() { ui.selectable_value(&mut place.source, i, name(i)); }
                    });
                    if place.source != before {
                        place.error = None;
                        if let Some(s) = sources.get(place.source) {
                            // A sharper picture than the server has only takes memory.
                            let finest = rt::ground_resolution(place.lat, s.max_zoom) as f32;
                            if place.mpp < finest * 0.8 {
                                place.mpp = PIXEL_SIZES.iter().copied().find(|m| *m >= finest * 0.8).unwrap_or(10.0);
                            }
                        }
                    }
                });
                if place.source >= sources.len() {
                    row(ui, &t("Address"), |ui| {
                        if ui.add(egui::TextEdit::singleline(&mut place.custom_url).hint_text("https://…/{z}/{x}/{y}.jpg").desired_width(200.0)).lost_focus() {
                            place.error = None;
                            place.failed = None;
                            place.texture = None;
                        }
                    });
                }
                ui.add(egui::Label::new(egui::RichText::new(licence_note(place.source)).small().weak()).wrap());
                row(ui, &t("Pixel size"), |ui| {
                    egui::ComboBox::from_id_salt("real_mpp").width(200.0).selected_text(format!("{} m", place.mpp)).show_ui(ui, |ui| {
                        for m in PIXEL_SIZES {
                            let px = (size_m / m as f64).round();
                            ui.selectable_value(&mut place.mpp, m, format!("{m} m  ({px} px)"));
                        }
                    });
                });
                let px = (size_m / place.mpp as f64).round();
                let gb = px * px * 3.0 / 1e9;
                if gb > 0.75 {
                    ui.colored_label(crate::theme::WARN, tf("A large picture: it needs about {0} GB of memory.", &[&format!("{gb:.1}")]));
                }
            });
            ui.checkbox(&mut place.heights, t("Heights")).on_hover_text(t("From the AWS Terrain Tiles: about 30 m detail (SRTM) in most places, finer in some countries."));
            ui.checkbox(&mut place.shapes, t("OpenStreetMap shapes")).on_hover_text(t("© OpenStreetMap contributors, ODbL. Each kind of shape gets its own shape layer."));
            ui.add_enabled_ui(place.shapes, |ui| {
                ui.indent("osm_parts", |ui| {
                    egui::Grid::new("osm_grid").num_columns(3).show(ui, |ui| {
                        ui.checkbox(&mut place.osm.roads, t("Roads"));
                        ui.checkbox(&mut place.osm.railways, t("Railways"));
                        ui.checkbox(&mut place.osm.waterways, t("Rivers and streams"));
                        ui.end_row();
                        ui.checkbox(&mut place.osm.water, t("Lakes"));
                        ui.checkbox(&mut place.osm.forests, t("Forests"));
                        ui.checkbox(&mut place.osm.land, t("Land use"));
                        ui.end_row();
                        ui.checkbox(&mut place.osm.buildings, t("Buildings"));
                    });
                });
            });
            crate::theme::subsection(ui, &t("Surface mask"), true, |ui| mask_ui(ui, place, size_m));
            crate::theme::subsection(ui, &t("Generators"), true, |ui| generators_ui(ui, place));
            ui.separator();
            let area = rt::GeoArea { lat: place.lat, lon: place.lon, size_m };
            let src = place.source();
            let (sat, height) = rt::tile_counts(&area, place.satellite.then(|| (&src, (size_m / place.mpp as f64).round() as usize)), cell);
            let height = if place.heights { height } else { 0 };
            ui.weak(tf("About {0} tiles to download.", &[&(sat + height).to_string()]));
            if let Some(e) = &place.error {
                ui.colored_label(crate::theme::WARN, e);
            }
        }));
    });
    place.poll(&ctx, dragging);
}

/// The generators added for the OpenStreetMap shapes: forests with a preset for each kind,
/// scrub, fields and lakes, each on its layer of shapes.
fn generators_ui(ui: &mut egui::Ui, place: &mut RealPlace) {
    ui.add_enabled_ui(place.shapes, |ui| {
        ui.checkbox(&mut place.generators_on, t("Add generators for the OpenStreetMap shapes"));
    });
    if !(place.shapes && place.generators_on) {
        return;
    }
    let g = &mut place.generators;
    let presets: Vec<crate::theme::PickItem<String>> = forest_core::species::zone_presets().iter().map(|z| crate::theme::PickItem { key: z.name.to_string(), name: z.name.to_string(), note: z.group.to_string(), model: None }).collect();
    let forest = |ui: &mut egui::Ui, label: String, id: &str, value: &mut Option<String>, default: &str| {
        let mut on = value.is_some();
        ui.horizontal(|ui| {
            ui.checkbox(&mut on, label);
            if let Some(v) = value.as_mut() {
                let w = ui.available_width();
                crate::theme::search_pick(ui, id, v, None, &presets, &|_| None, w);
            }
        });
        if on != value.is_some() {
            *value = on.then(|| default.to_string());
        }
    };
    forest(ui, t("Leafy and mixed forests"), "gen_leafy", &mut g.leafy, "Grąd (dąb-buk)");
    forest(ui, t("Conifer forests"), "gen_conifer", &mut g.conifer, "Bór sosnowy (niziny)");
    forest(ui, t("Scrub, heath and wetland"), "gen_scrub", &mut g.scrub, "Zarośla krzewiaste");
    ui.checkbox(&mut g.fields, t("Fields")).on_hover_text(t("The Fields generator on the fields of OpenStreetMap."));
    ui.checkbox(&mut g.ponds, t("Lakes and ponds")).on_hover_text(t("The Ponds and lakes generator digs a basin for each, which the real heights mostly have already."));
    ui.checkbox(&mut g.run, t("Run them when the map opens")).on_hover_text(t("One after another; each can be stopped. Otherwise they wait in the outliner."));
    ui.weak(t("Each works on its layer of shapes, so it can be changed and run again from the outliner."));
}

/// How the surface mask of the new map is made: read from the satellite picture as in the
/// Satellite tool, with a preview on the picture of the place, and painted from the shapes.
fn mask_ui(ui: &mut egui::Ui, place: &mut RealPlace, size_m: f64) {
    let layers = dz_core::world::default_layers();
    ui.add_enabled_ui(place.satellite, |ui| {
        ui.checkbox(&mut place.mask, t("From the satellite picture"));
    });
    if place.satellite && place.mask {
        let mut classify = std::mem::take(&mut place.classify);
        let mut view = std::mem::take(&mut place.class_view);
        match place.crop(size_m) {
            Some((small, rev)) => crate::satellite::classify_fields(ui, &mut classify, small, rev, &layers, &mut view, "real"),
            None => { ui.weak(t("The settings show once the picture of the place is loaded.")); }
        }
        (place.classify, place.class_view) = (classify, view);
        ui.weak(t("The preview is made from the picture you choose the place on, which is coarser than the one downloaded."));
    }
    ui.add_enabled_ui(place.shapes, |ui| {
        ui.checkbox(&mut place.shape_mask_on, t("From the OpenStreetMap shapes, over the satellite"));
    });
    if place.shapes && place.shape_mask_on {
        let names: Vec<String> = layers.iter().map(|l| l.name.clone()).collect();
        let leave = t("Leave as it is");
        let m = &mut place.shape_mask;
        egui::Grid::new("shape_mask").num_columns(3).spacing(egui::vec2(6.0, 3.0)).show(ui, |ui| {
            for (k, class) in rt::SHAPE_CLASSES.iter().enumerate() {
                ui.label(t(class.name));
                let target = &mut m.surfaces[k];
                let shown = target.and_then(|i| names.get(i).cloned()).unwrap_or_else(|| leave.clone());
                egui::ComboBox::from_id_salt(("shape_surface", k)).width(110.0).selected_text(shown).show_ui(ui, |ui| {
                    ui.selectable_value(target, None, &leave);
                    for (i, n) in names.iter().enumerate() { ui.selectable_value(target, Some(i), n); }
                });
                if class.width > 0.0 {
                    ui.add(crate::theme::ValueField::new(&mut m.widths[k], 0.5..=30.0).suffix(" m")).on_hover_text(t("How wide the line is painted."));
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
        ui.weak(t("Painted in this order, over the surfaces from the satellite: land, forests and water, then roads and railways."));
    }
}

/// Whether the choices can be made into a project.
pub fn ready(p: &NewProject) -> Result<(), String> {
    let place = &p.place;
    if place.satellite && place.source >= rt::satellite_sources().len() {
        let u = place.custom_url.trim();
        if !(u.starts_with("http") && u.contains("{z}") && u.contains("{x}") && u.contains("{y}")) {
            return Err(t("The address of the tile server needs {z}, {x} and {y}."));
        }
    }
    if !place.satellite && !place.heights && !place.shapes {
        return Err(t("Choose something to download."));
    }
    Ok(())
}

/// Downloads the place and opens it as a new project, saved in the folder when one is given.
pub fn create(app: &mut App, p: &NewProject) {
    let m = p.place.map(p);
    let classify = p.place.classify.clone();
    let run = m.generators.as_ref().is_some_and(|g| g.run);
    let dir = (!p.folder.trim().is_empty()).then(|| std::path::PathBuf::from(p.folder.trim()).join(dz_core::world::WorldMeta::new(&p.name, 1, 1.0).map_id));
    app.spawn(t("Making the map of a real place"), move |job| {
        let f = fetcher()?;
        if let Some((src, _)) = &m.satellite {
            if !src.attribution.is_empty() {
                job.log(format!("Satellite picture: © {}", src.attribution));
            }
        }
        let w = rt::build_world(&m, &f, job)?;
        if let Some(d) = &dir {
            job.stage("Saving the project");
            project::save(&w, d, SaveParts::ALL)?;
        }
        Ok(Box::new(move |app: &mut App| {
            app.open_world(w, dir);
            if run {
                if let Some(d) = &app.doc {
                    app.gen_queue = d.world.generators.iter().map(|g| g.id).collect();
                }
            }
            // The satellite tool reads surfaces the same way the mask was made.
            app.sat.classify = classify;
        }) as Finish)
    });
}
