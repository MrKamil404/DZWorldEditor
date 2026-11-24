//! Standalone 3D loot point preview and editor for `mapgroupproto.xml`.
//!
//! Opened from the Content Browser context menu on any building/model (`Pick::Model`).
//! Reuses `forest_core::model3d` orthographic software rendering with an optional
//! horizontal roof-slice plane (`clip_y`) so interior rooms and shelves can be inspected
//! and clicked directly to place DayZ-compliant `<point>` entries.
use crate::app::App;
use crate::i18n::{t, tf};
use crate::icons;
use dz_core::mission::proto::{self, ContainerProto, GroupProto, LootPoint};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use forest_core::model3d::{self, ModelMesh, ViewFit};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

const PREVIEW_SIZE: u32 = 420;

const CONTAINER_COLORS: [Color32; 6] = [
    Color32::from_rgb(76, 195, 247),  // cyan
    Color32::from_rgb(129, 199, 132), // green
    Color32::from_rgb(255, 183, 77),  // amber
    Color32::from_rgb(240, 98, 146),  // pink
    Color32::from_rgb(186, 104, 200), // violet
    Color32::from_rgb(255, 138, 101), // coral
];

type RenderKey = (String, i32, i32, Option<i32>);
/// Loaded mesh paired with the offset from bounding-box centre to mesh vertex space
/// (`[0; 3]` for ODOL, whose vertices are already centred, or `ModelInfo::center` for MLOD).
type LoadedMesh = (Arc<ModelMesh>, [f32; 3]);
type RenderResult = (RenderKey, Option<(Vec<u8>, ViewFit, LoadedMesh)>);

pub struct LootProtoUi {
    pub open: bool,
    pub model: String,
    original_class: Option<String>,
    exists_in_proto: bool,
    group: GroupProto,
    selected_container: usize,
    selected_point: Option<usize>,
    yaw: f32,
    pitch: f32,
    clip_enabled: bool,
    clip_y: f32,
    add_mode: bool,
    show_ranges: bool,
    dirty: bool,
    mesh: Option<LoadedMesh>,
    fit: Option<ViewFit>,
    texture: Option<egui::TextureHandle>,
    shown: Option<RenderKey>,
    wanted: Option<RenderKey>,
    busy: bool,
    failed: Option<String>,
    mesh_cache: Arc<Mutex<Option<(String, LoadedMesh)>>>,
    tx: Sender<RenderResult>,
    rx: Receiver<RenderResult>,
}

impl Default for LootProtoUi {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            open: false,
            model: String::new(),
            original_class: None,
            exists_in_proto: false,
            group: GroupProto::default_for("Land_Building"),
            selected_container: 0,
            selected_point: None,
            yaw: 35.0,
            pitch: 28.0,
            clip_enabled: false,
            clip_y: 1.5,
            add_mode: false,
            show_ranges: true,
            dirty: false,
            mesh: None,
            fit: None,
            texture: None,
            shown: None,
            wanted: None,
            busy: false,
            failed: None,
            mesh_cache: Default::default(),
            tx,
            rx,
        }
    }
}

/// Resolves the most likely `mapgroupproto.xml` class name for a `.p3d` model path.
pub fn resolve_class(app: &mut App, model: &str) -> String {
    let candidates = crate::mission_ui::class_index(app)
        .map(|idx| idx.classes_for_model(model, false))
        .unwrap_or_default();
    if let Some(m) = crate::mission_ui::mission(app) {
        if let Some(found) = candidates.iter().find(|c| proto::has_group(m, c)) {
            return found.clone();
        }
    }
    if let Some(first) = candidates.into_iter().next() {
        return first;
    }
    dz_core::mission::gen::proxy_class(model)
}

/// Opens the standalone 3D loot point editor window for `model`.
pub fn open(app: &mut App, model: String) {
    let class = resolve_class(app, &model);
    let existing = crate::mission_ui::mission(app).and_then(|m| proto::read_group(m, &class));
    let (exists, group) = match existing {
        Some(g) => (true, g),
        None => (false, GroupProto::default_for(&class)),
    };
    let same_model = app.loot_proto.model.eq_ignore_ascii_case(&model);
    let ui = &mut app.loot_proto;
    ui.open = true;
    ui.model = model;
    ui.original_class = exists.then(|| group.name.clone());
    ui.exists_in_proto = exists;
    ui.selected_container = 0;
    ui.selected_point = group.containers.first().and_then(|c| (!c.points.is_empty()).then_some(0));
    ui.group = group;
    ui.dirty = false;
    ui.add_mode = false;
    if !same_model {
        ui.mesh = None;
        ui.fit = None;
        ui.shown = None;
        ui.wanted = None;
        ui.failed = None;
        ui.yaw = 35.0;
        ui.pitch = 28.0;
        ui.clip_enabled = false;
        ui.clip_y = 1.5;
    }
}

impl LootProtoUi {
    fn clip_param(&self) -> Option<f32> {
        self.clip_enabled.then_some(self.clip_y)
    }

    fn poll(&mut self, ctx: &egui::Context, db: &dz_core::model_info::ModelDb) {
        while let Ok((key, res)) = self.rx.try_recv() {
            self.busy = false;
            match res {
                Some((px, fit, loaded)) => {
                    let first_mesh = self.mesh.is_none();
                    if first_mesh {
                        // Start roof slice just above the middle height of the building.
                        let (lo, hi) = (loaded.0.min[1], loaded.0.max[1]);
                        self.clip_y = lo + (hi - lo) * 0.55;
                    }
                    let image = egui::ColorImage::from_rgba_unmultiplied([PREVIEW_SIZE as usize; 2], &px);
                    match &mut self.texture {
                        Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                        None => self.texture = Some(ctx.load_texture("loot_proto_preview", image, egui::TextureOptions::LINEAR)),
                    }
                    self.fit = Some(fit);
                    self.mesh = Some(loaded);
                    self.failed = None;
                }
                None => self.failed = Some(key.0.clone()),
            }
            self.shown = Some(key);
        }
        if !self.busy {
            if let Some(w) = self.wanted.take() {
                self.start(w, ctx, db);
            }
        }
    }

    fn request(&mut self, ctx: &egui::Context, db: &dz_core::model_info::ModelDb) {
        if self.model.is_empty() {
            return;
        }
        let clip_cm = self.clip_param().map(|y| (y * 100.0).round() as i32);
        let want: RenderKey = (self.model.clone(), self.yaw.round() as i32, self.pitch.round() as i32, clip_cm);
        if self.shown.as_ref() == Some(&want) || self.wanted.as_ref() == Some(&want) {
            return;
        }
        if self.busy {
            self.wanted = Some(want);
        } else {
            self.start(want, ctx, db);
        }
    }

    fn start(&mut self, key: RenderKey, ctx: &egui::Context, db: &dz_core::model_info::ModelDb) {
        self.busy = true;
        let (tx, cache, db, ctx) = (self.tx.clone(), self.mesh_cache.clone(), db.clone(), ctx.clone());
        std::thread::spawn(move || {
            let cached = cache.lock().unwrap().as_ref().filter(|c| c.0 == key.0).map(|c| c.1.clone());
            let loaded = cached.or_else(|| {
                let file = db.file(&key.0);
                let res = match model3d::load_model_lod(&file, db.root(), 512, 0) {
                    Ok(mesh) => Some((Arc::new(mesh), [0.0, 0.0, 0.0])),
                    Err(_) => crate::models::thumbnail_mesh(&file, &Default::default()).map(|mesh| {
                        let info = db.get(&key.0);
                        let offset = if info.known {
                            info.center
                        } else {
                            [(mesh.min[0] + mesh.max[0]) * 0.5, (mesh.min[1] + mesh.max[1]) * 0.5, (mesh.min[2] + mesh.max[2]) * 0.5]
                        };
                        (Arc::new(mesh), offset)
                    }),
                };
                if let Some(ref l) = res {
                    *cache.lock().unwrap() = Some((key.0.clone(), l.clone()));
                }
                res
            });
            let out = loaded.map(|(mesh, offset)| {
                let clip_y = key.3.map(|cm| cm as f32 * 0.01);
                let (px, fit) = model3d::render_preview_clipped(
                    &mesh,
                    PREVIEW_SIZE,
                    (key.1 as f32).to_radians(),
                    (key.2 as f32).to_radians(),
                    clip_y,
                );
                (px, fit, (mesh, offset))
            });
            let _ = tx.send((key, out));
            ctx.request_repaint();
        });
    }
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.loot_proto.open {
        return;
    }
    let db = app.db.clone();
    app.loot_proto.poll(ctx, &db);
    app.loot_proto.request(ctx, &db);

    let limits = proto::limits_lists(crate::mission_ui::mission(app));
    let has_mission = crate::mission_ui::mission(app).is_some();
    let title = tf("Loot points — {0}", &[&app.loot_proto.group.name]);

    let mut open = app.loot_proto.open;
    let mut create_mission = false;
    let mut save_requested = false;
    let mut remove_requested = false;

    egui::Window::new(title)
        .id(egui::Id::new("loot_proto_editor_window"))
        .open(&mut open)
        .default_size([960.0, 620.0])
        .min_size([760.0, 480.0])
        .resizable(true)
        .show(ctx, |ui| {
            ui.columns(2, |cols| {
                // ---------------- Left column: 3D preview & point picking ----------------
                draw_preview_column(&mut app.loot_proto, &mut cols[0]);

                // ---------------- Right column: DayZ mapgroupproto.xml fields ------------
                draw_editor_column(
                    &mut app.loot_proto,
                    &mut cols[1],
                    &limits,
                    has_mission,
                    &mut create_mission,
                    &mut save_requested,
                    &mut remove_requested,
                );
            });
        });

    app.loot_proto.open = open;

    if create_mission {
        crate::mission_ui::new_mission(app);
        // Reload prototype in case the newly created mission has defaults.
        if let Some(m) = crate::mission_ui::mission(app) {
            if let Some(g) = proto::read_group(m, &app.loot_proto.group.name) {
                app.loot_proto.exists_in_proto = true;
                app.loot_proto.original_class = Some(g.name.clone());
                app.loot_proto.group = g;
            }
        }
    }

    if save_requested {
        save_to_mission(app);
    }
    if remove_requested {
        remove_from_mission(app);
    }
}

fn draw_preview_column(st: &mut LootProtoUi, ui: &mut egui::Ui) {
    // Camera angle presets & mode controls
    ui.horizontal_wrapped(|ui| {
        if ui.small_button(t("Iso")).clicked() {
            st.yaw = 35.0;
            st.pitch = 28.0;
        }
        if ui.small_button(t("Top")).clicked() {
            st.yaw = 0.0;
            st.pitch = 88.0;
        }
        if ui.small_button(t("Front")).clicked() {
            st.yaw = 0.0;
            st.pitch = 5.0;
        }
        if ui.small_button(t("Side")).clicked() {
            st.yaw = 90.0;
            st.pitch = 5.0;
        }
        ui.separator();
        ui.checkbox(&mut st.show_ranges, t("Show ranges"));
        ui.separator();
        ui.toggle_value(&mut st.add_mode, icons::with(icons::PLUS, &t("Click 3D to add")))
            .on_hover_text(t("Click a surface in the 3D preview to place a new loot point there."));
    });

    // Horizontal roof slice slider
    ui.horizontal(|ui| {
        ui.checkbox(&mut st.clip_enabled, t("Cut roof"))
            .on_hover_text(t("Slice the model horizontally to see inside rooms and floors."));
        let (lo, hi) = st.mesh.as_ref().map(|(m, _)| (m.min[1], m.max[1])).unwrap_or((-3.0, 5.0));
        let hi = hi.max(lo + 0.1);
        ui.add_enabled(
            st.clip_enabled,
            egui::Slider::new(&mut st.clip_y, lo..=hi).suffix(" m").fixed_decimals(2),
        );
    });

    ui.add_space(4.0);

    // 3D viewport box
    let side = ui.available_width().min(ui.available_height() - 42.0).clamp(260.0, 520.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_gray(24));
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, Color32::from_gray(55)), egui::StrokeKind::Inside);

    if let Some(tex) = &st.texture {
        painter.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    } else if st.failed.is_some() {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, t("Could not load the 3D model for preview."), egui::FontId::proportional(13.0), Color32::from_rgb(220, 120, 120));
    } else {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, t("Loading 3D preview..."), egui::FontId::proportional(13.0), Color32::GRAY);
    }

    // Rotate with drag
    if resp.dragged() {
        let d = resp.drag_delta();
        st.yaw = (st.yaw + d.x * 0.6).rem_euclid(360.0);
        st.pitch = (st.pitch + d.y * 0.5).clamp(-85.0, 89.0);
    }

    // Project and draw loot points
    let offset = st.mesh.as_ref().map(|(_, o)| *o).unwrap_or([0.0; 3]);
    if let Some(fit) = st.fit {
        let yaw_rad = st.yaw.to_radians();
        let pitch_rad = st.pitch.to_radians();
        let to_screen = |p_proto: [f32; 3]| -> Pos2 {
            let p_mesh = [p_proto[0] + offset[0], p_proto[1] + offset[1], p_proto[2] + offset[2]];
            let uv = model3d::project_point(p_mesh, side, yaw_rad, pitch_rad, fit);
            Pos2::new(rect.left() + uv[0], rect.top() + uv[1])
        };

        // Draw points for every container
        for (ci, container) in st.group.containers.iter().enumerate() {
            let base_col = CONTAINER_COLORS[ci % CONTAINER_COLORS.len()];
            for (pi, pt) in container.points.iter().enumerate() {
                let selected = ci == st.selected_container && st.selected_point == Some(pi);
                let col = if selected { Color32::from_rgb(255, 240, 80) } else { base_col };
                let foot = to_screen(pt.pos);
                let top = to_screen([pt.pos[0], pt.pos[1] + pt.height, pt.pos[2]]);

                if st.show_ranges && pt.range > 0.01 {
                    let mut ring = Vec::with_capacity(17);
                    for k in 0..=16 {
                        let a = k as f32 * std::f32::consts::TAU / 16.0;
                        ring.push(to_screen([pt.pos[0] + pt.range * a.cos(), pt.pos[1], pt.pos[2] + pt.range * a.sin()]));
                    }
                    let stroke_w = if selected { 2.0_f32 } else { 1.1_f32 };
                    let ring_col = Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), if selected { 230 } else { 145 });
                    painter.add(egui::Shape::line(ring, Stroke::new(stroke_w, ring_col)));
                }

                // Height stalk
                painter.line_segment(
                    [foot, top],
                    Stroke::new(if selected { 2.0_f32 } else { 1.2_f32 }, Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), 190)),
                );
                // Base dot (diamond outline if outdoor flags=32)
                let r = if selected { 6.0_f32 } else { 4.2_f32 };
                painter.circle_filled(foot, r, col);
                painter.circle_stroke(foot, r, Stroke::new(1.2_f32, Color32::BLACK));
                if pt.flags & 32 != 0 {
                    painter.circle_stroke(foot, r + 2.5, Stroke::new(1.2_f32, Color32::from_rgb(255, 210, 90)));
                }
            }
        }

        // Click handling: either add a new point on the picked 3D surface or select nearest point
        if resp.clicked() {
            if let Some(ptr) = resp.interact_pointer_pos() {
                if st.add_mode {
                    if let Some((mesh, off)) = &st.mesh {
                        let px = (ptr.x - rect.left()).clamp(0.0, side);
                        let py = (ptr.y - rect.top()).clamp(0.0, side);
                        if let Some(hit) = model3d::pick_surface(mesh, px, py, side, yaw_rad, pitch_rad, fit, st.clip_param()) {
                            if st.group.containers.is_empty() {
                                st.group.containers.push(ContainerProto::default());
                                st.selected_container = 0;
                            }
                            let ci = st.selected_container.min(st.group.containers.len() - 1);
                            let pt = LootPoint {
                                pos: [hit[0] - off[0], hit[1] - off[1], hit[2] - off[2]],
                                ..LootPoint::default()
                            };
                            let c = &mut st.group.containers[ci];
                            c.points.push(pt);
                            st.selected_point = Some(c.points.len() - 1);
                            st.dirty = true;
                        }
                    }
                } else {
                    let mut best: Option<(f32, usize, usize)> = None;
                    for (ci, container) in st.group.containers.iter().enumerate() {
                        for (pi, pt) in container.points.iter().enumerate() {
                            let s = to_screen(pt.pos);
                            let d = s.distance(ptr);
                            if d <= 14.0 && best.is_none_or(|b| d < b.0) {
                                best = Some((d, ci, pi));
                            }
                        }
                    }
                    if let Some((_, ci, pi)) = best {
                        st.selected_container = ci;
                        st.selected_point = Some(pi);
                    }
                }
            }
        }
    }

    ui.add_space(4.0);
    let hint = if st.add_mode {
        t("Drag to rotate · Click a surface to add a loot point")
    } else {
        t("Drag to rotate · Click a point to select it")
    };
    ui.weak(hint);
    ui.weak(&st.model);
}

#[allow(clippy::too_many_arguments)]
fn draw_editor_column(
    st: &mut LootProtoUi,
    ui: &mut egui::Ui,
    limits: &proto::LimitsLists,
    has_mission: bool,
    create_mission: &mut bool,
    save_requested: &mut bool,
    remove_requested: &mut bool,
) {
    if !has_mission {
        ui.horizontal(|ui| {
            ui.colored_label(Color32::from_rgb(240, 185, 90), t("This project has no mission yet."));
            if ui.button(t("Create mission")).clicked() {
                *create_mission = true;
            }
        });
        ui.separator();
    } else if !st.exists_in_proto {
        ui.colored_label(Color32::from_rgb(160, 205, 250), t("Not yet in mapgroupproto.xml — edit and save to add it."));
        ui.add_space(2.0);
    }

    egui::ScrollArea::vertical().id_salt("loot_proto_editor_scroll").max_height(ui.available_height() - 38.0).show(ui, |ui| {
        // ---- Group properties ----
        egui::Grid::new("loot_group_grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label(t("Prototype class"));
            if ui.add(egui::TextEdit::singleline(&mut st.group.name).desired_width(220.0)).changed() {
                st.dirty = true;
            }
            ui.end_row();

            ui.label(t("Max loot"));
            ui.horizontal(|ui| {
                let mut has_max = st.group.lootmax.is_some();
                if ui.checkbox(&mut has_max, "").changed() {
                    st.group.lootmax = has_max.then_some(2);
                    st.dirty = true;
                }
                if let Some(lm) = &mut st.group.lootmax {
                    if ui.add(egui::DragValue::new(lm).range(0..=100)).changed() {
                        st.dirty = true;
                    }
                }
                if ui.checkbox(&mut st.group.deloot, t("Dynamic event loot (deloot)")).changed() {
                    st.dirty = true;
                }
            });
            ui.end_row();
        });

        ui.add_space(4.0);
        egui::CollapsingHeader::new(t("Usage flags")).default_open(true).show(ui, |ui| {
            if flag_chips(ui, &limits.usages, &mut st.group.usages) {
                st.dirty = true;
            }
        });
        egui::CollapsingHeader::new(t("Value flags")).default_open(false).show(ui, |ui| {
            if flag_chips(ui, &limits.values, &mut st.group.values) {
                st.dirty = true;
            }
        });

        ui.separator();

        // ---- Containers bar ----
        ui.horizontal_wrapped(|ui| {
            ui.strong(t("Containers"));
            for i in 0..st.group.containers.len() {
                let col = CONTAINER_COLORS[i % CONTAINER_COLORS.len()];
                let name = st.group.containers[i].name.clone();
                let count = st.group.containers[i].points.len();
                let label = egui::RichText::new(format!("● {name} ({count})")).color(col);
                if ui.selectable_label(st.selected_container == i, label).clicked() {
                    st.selected_container = i;
                    st.selected_point = (!st.group.containers[i].points.is_empty()).then_some(0);
                }
            }
            if ui.small_button(icons::with(icons::PLUS, &t("Add container"))).clicked() {
                let name = if st.group.containers.is_empty() {
                    "lootFloor".to_string()
                } else {
                    format!("lootshelves{}", st.group.containers.len())
                };
                st.group.containers.push(ContainerProto { name, ..ContainerProto::default() });
                st.selected_container = st.group.containers.len() - 1;
                st.selected_point = None;
                st.dirty = true;
            }
        });

        if st.selected_container >= st.group.containers.len() && !st.group.containers.is_empty() {
            st.selected_container = st.group.containers.len() - 1;
        }

        // ---- Selected container details ----
        let mut delete_container = false;
        let can_delete_container = st.group.containers.len() > 1;
        if let Some(c) = st.group.containers.get_mut(st.selected_container) {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t("Container name"));
                if ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(140.0)).changed() {
                    st.dirty = true;
                }
                let mut has_max = c.lootmax.is_some();
                if ui.checkbox(&mut has_max, t("Max loot")).changed() {
                    c.lootmax = has_max.then_some(2);
                    st.dirty = true;
                }
                if let Some(lm) = &mut c.lootmax {
                    if ui.add(egui::DragValue::new(lm).range(0..=100)).changed() {
                        st.dirty = true;
                    }
                }
                if can_delete_container && ui.small_button(t("Delete")).clicked() {
                    delete_container = true;
                }
            });

            egui::CollapsingHeader::new(t("Categories")).default_open(true).show(ui, |ui| {
                if flag_chips(ui, &limits.categories, &mut c.categories) {
                    st.dirty = true;
                }
            });
            egui::CollapsingHeader::new(t("Tags")).default_open(true).show(ui, |ui| {
                if flag_chips(ui, &limits.tags, &mut c.tags) {
                    st.dirty = true;
                }
            });

            ui.separator();

            // ---- Points list ----
            ui.horizontal(|ui| {
                ui.strong(tf("Loot points ({0})", &[&c.points.len().to_string()]));
                if ui.small_button(icons::with(icons::PLUS, &t("Add point"))).clicked() {
                    let pt = st.selected_point.and_then(|i| c.points.get(i)).cloned().unwrap_or_default();
                    c.points.push(pt);
                    st.selected_point = Some(c.points.len() - 1);
                    st.dirty = true;
                }
                if let Some(pi) = st.selected_point {
                    if pi < c.points.len() {
                        if ui.small_button(t("Duplicate")).clicked() {
                            let mut dup = c.points[pi].clone();
                            dup.pos[0] += 0.3;
                            c.points.push(dup);
                            st.selected_point = Some(c.points.len() - 1);
                            st.dirty = true;
                        }
                        if ui.small_button(t("Delete")).clicked() {
                            c.points.remove(pi);
                            st.selected_point = if c.points.is_empty() {
                                None
                            } else {
                                Some(pi.min(c.points.len() - 1))
                            };
                            st.dirty = true;
                        }
                    }
                }
            });

            egui::ScrollArea::vertical().id_salt("loot_points_list").max_height(130.0).show(ui, |ui| {
                for (i, pt) in c.points.iter().enumerate() {
                    let sel = st.selected_point == Some(i);
                    let flag_tag = if pt.flags & 32 != 0 { " [out]" } else { "" };
                    let text = format!(
                        "#{}: ({:.2}, {:.2}, {:.2})  r={:.2}  h={:.2}{flag_tag}",
                        i + 1, pt.pos[0], pt.pos[1], pt.pos[2], pt.range, pt.height
                    );
                    if ui.selectable_label(sel, text).clicked() {
                        st.selected_point = Some(i);
                    }
                }
            });

            // ---- Selected point inspector ----
            if let Some(pi) = st.selected_point {
                if let Some(pt) = c.points.get_mut(pi) {
                    ui.separator();
                    ui.strong(tf("Point #{0}", &[&(pi + 1).to_string()]));
                    egui::Grid::new("loot_point_inspector").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                        ui.label(t("Position (X, Y, Z)"));
                        ui.horizontal(|ui| {
                            let mut changed = false;
                            changed |= ui.add(egui::DragValue::new(&mut pt.pos[0]).speed(0.02).prefix("X ").fixed_decimals(3)).changed();
                            changed |= ui.add(egui::DragValue::new(&mut pt.pos[1]).speed(0.02).prefix("Y ").fixed_decimals(3)).changed();
                            changed |= ui.add(egui::DragValue::new(&mut pt.pos[2]).speed(0.02).prefix("Z ").fixed_decimals(3)).changed();
                            if changed {
                                st.dirty = true;
                            }
                        });
                        ui.end_row();

                        ui.label(t("Range"));
                        if ui.add(egui::DragValue::new(&mut pt.range).speed(0.02).range(0.05..=5.0).suffix(" m").fixed_decimals(3)).changed() {
                            st.dirty = true;
                        }
                        ui.end_row();

                        ui.label(t("Height"));
                        if ui.add(egui::DragValue::new(&mut pt.height).speed(0.02).range(0.1..=5.0).suffix(" m").fixed_decimals(3)).changed() {
                            st.dirty = true;
                        }
                        ui.end_row();

                        ui.label("");
                        let mut outdoor = pt.flags & 32 != 0;
                        if ui.checkbox(&mut outdoor, t("Outside / ground (flags=32)")).changed() {
                            if outdoor {
                                pt.flags |= 32;
                            } else {
                                pt.flags &= !32;
                            }
                            st.dirty = true;
                        }
                        ui.end_row();
                    });
                }
            }
        }
        if delete_container && st.selected_container < st.group.containers.len() {
            st.group.containers.remove(st.selected_container);
            st.selected_container = st.selected_container.min(st.group.containers.len().saturating_sub(1));
            st.selected_point = st.group.containers.get(st.selected_container).and_then(|c| (!c.points.is_empty()).then_some(0));
            st.dirty = true;
        }
    });

    ui.separator();
    ui.horizontal(|ui| {
        if ui.add_enabled(has_mission, egui::Button::new(icons::with(icons::FLOPPY_DISK, &t("Save to mapgroupproto.xml")))).clicked() {
            *save_requested = true;
        }
        if has_mission && st.exists_in_proto && ui.button(t("Remove from mapgroupproto.xml")).clicked() {
            *remove_requested = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(t("Close")).clicked() {
                st.open = false;
            }
        });
    });
}

fn flag_chips(ui: &mut egui::Ui, choices: &[String], selected: &mut Vec<String>) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        // Show known choices plus any custom flags already in the XML so none are silently dropped.
        let mut all: Vec<String> = choices.to_vec();
        for s in selected.iter() {
            if !all.iter().any(|c| c.eq_ignore_ascii_case(s)) {
                all.push(s.clone());
            }
        }
        for name in all {
            let pos = selected.iter().position(|s| s.eq_ignore_ascii_case(&name));
            let active = pos.is_some();
            if ui.selectable_label(active, &name).clicked() {
                if let Some(i) = pos {
                    selected.remove(i);
                } else {
                    selected.push(name);
                }
                changed = true;
            }
        }
    });
    changed
}

fn save_to_mission(app: &mut App) {
    let name = app.loot_proto.group.name.trim().to_string();
    if name.is_empty() {
        app.warn(t("Enter a prototype class name first."));
        return;
    }
    app.loot_proto.group.name = name.clone();
    let group = app.loot_proto.group.clone();
    let original = app.loot_proto.original_class.clone();
    let Some(m) = crate::mission_ui::mission_mut(app) else { return };
    let edit = proto::write_group(m, original.as_deref(), &group);
    crate::mission_ui::push(app, &t("Edit loot points"), vec![edit]);
    app.loot_proto.exists_in_proto = true;
    app.loot_proto.original_class = Some(name.clone());
    app.loot_proto.dirty = false;
    app.info(tf("Saved loot points for {0} to mapgroupproto.xml.", &[&name]));
}

fn remove_from_mission(app: &mut App) {
    let name = app.loot_proto.original_class.clone().unwrap_or_else(|| app.loot_proto.group.name.clone());
    let Some(m) = crate::mission_ui::mission_mut(app) else { return };
    if let Some(edit) = proto::remove_group(m, &name) {
        crate::mission_ui::push(app, &t("Remove loot prototype"), vec![edit]);
        app.loot_proto.exists_in_proto = false;
        app.loot_proto.original_class = None;
        app.loot_proto.dirty = false;
        app.info(tf("Removed {0} from mapgroupproto.xml.", &[&name]));
    }
}
