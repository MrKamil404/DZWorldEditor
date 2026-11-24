//! Standalone 3D loot point preview and editor for `mapgroupproto.xml`.
//!
//! Opened from the Content Browser context menu on any building/model (`Pick::Model`).
//! Renders the building directly on the GPU via OpenGL (`render::preview_callback`) with the
//! same free-fly 3D camera controls as the terrain viewport (RMB + WASD/QE, Shift, MMB pan,
//! scroll zoom/speed), an instant GPU horizontal roof-slice plane (`clip_y`), and 3D surface
//! raycasting for placing DayZ-compliant `<point>` entries.
use crate::app::App;
use crate::camera::{add, dot, mul, norm, sub, Camera, View};
use crate::i18n::{t, tf};
use crate::icons;
use crate::keys::{Action, Hold};
use crate::render::{self, ModelPreviewFrame};
use dz_core::mission::proto::{self, ContainerProto, GroupProto, LootPoint};
use eframe::egui::{self, Color32, Key, PointerButton, Stroke, Vec2};

/// Reserved model-id namespace for isolated 3D loot previews (never collides with map/mission/proxy ids).
const PREVIEW_ID_BASE: u32 = 0x9000_0000;

const CONTAINER_COLORS: [Color32; 6] = [
    Color32::from_rgb(76, 195, 247),  // cyan
    Color32::from_rgb(129, 199, 132), // green
    Color32::from_rgb(255, 183, 77),  // amber
    Color32::from_rgb(240, 98, 146),  // pink
    Color32::from_rgb(186, 104, 200), // violet
    Color32::from_rgb(255, 138, 101), // coral
];

pub struct LootProtoUi {
    pub open: bool,
    pub model: String,
    model_id: u32,
    next_id: u32,
    /// Model bounding-box centre (origin of `mapgroupproto.xml` `<point pos>`).
    center: [f32; 3],
    bounds_min: [f32; 3],
    bounds_max: [f32; 3],
    original_class: Option<String>,
    exists_in_proto: bool,
    group: GroupProto,
    selected_container: usize,
    selected_point: Option<usize>,
    camera: Camera,
    clip_enabled: bool,
    clip_y: f32,
    add_mode: bool,
    show_ranges: bool,
    dirty: bool,
}

impl Default for LootProtoUi {
    fn default() -> Self {
        Self {
            open: false,
            model: String::new(),
            model_id: PREVIEW_ID_BASE,
            next_id: PREVIEW_ID_BASE + 1,
            center: [0.0; 3],
            bounds_min: [-4.0, 0.0, -4.0],
            bounds_max: [4.0, 5.0, 4.0],
            original_class: None,
            exists_in_proto: false,
            group: GroupProto::default_for("Land_Building"),
            selected_container: 0,
            selected_point: None,
            camera: Camera { pos: [8.0, 6.0, 8.0], yaw: 0.0, pitch: -0.4, fov: 65f32.to_radians(), speed: 6.0 },
            clip_enabled: false,
            clip_y: 2.5,
            add_mode: false,
            show_ranges: true,
            dirty: false,
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
/// Does nothing if the open project has no server mission yet.
pub fn open(app: &mut App, model: String) {
    if crate::mission_ui::mission(app).is_none() {
        return;
    }
    let class = resolve_class(app, &model);
    let existing = crate::mission_ui::mission(app).and_then(|m| proto::read_group(m, &class));
    let (exists, group) = match existing {
        Some(g) => (true, g),
        None => (false, GroupProto::default_for(&class)),
    };
    let info = app.db.get(&model);
    let same_model = app.loot_proto.model.eq_ignore_ascii_case(&model);

    if !same_model {
        let id = app.loot_proto.next_id;
        app.loot_proto.next_id = app.loot_proto.next_id.wrapping_add(1).max(PREVIEW_ID_BASE + 1);
        app.loot_proto.model_id = id;
        app.loader.request((id, 0), &model);
    }

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
    if info.known {
        ui.center = info.center;
        ui.bounds_min = info.min;
        ui.bounds_max = info.max;
    } else {
        ui.center = [0.0; 3];
        ui.bounds_min = [-4.0, 0.0, -4.0];
        ui.bounds_max = [4.0, 5.0, 4.0];
    }
    if !same_model {
        ui.clip_enabled = false;
        ui.clip_y = ui.bounds_min[1] + (ui.bounds_max[1] - ui.bounds_min[1]) * 0.55;
        ui.set_camera_preset(CameraPreset::Iso);
    }
}

#[derive(Clone, Copy)]
enum CameraPreset {
    Iso,
    Top,
    Front,
    Side,
    Inside,
}

impl LootProtoUi {
    fn clip_param(&self) -> Option<f32> {
        self.clip_enabled.then_some(self.clip_y)
    }

    fn box_center(&self) -> [f32; 3] {
        [
            (self.bounds_min[0] + self.bounds_max[0]) * 0.5,
            (self.bounds_min[1] + self.bounds_max[1]) * 0.5,
            (self.bounds_min[2] + self.bounds_max[2]) * 0.5,
        ]
    }

    fn box_radius(&self) -> f32 {
        let dx = self.bounds_max[0] - self.bounds_min[0];
        let dy = self.bounds_max[1] - self.bounds_min[1];
        let dz = self.bounds_max[2] - self.bounds_min[2];
        ((dx * dx + dy * dy + dz * dz).sqrt() * 0.5).max(1.5)
    }

    fn set_camera_preset(&mut self, preset: CameraPreset) {
        let c = self.box_center();
        let r = self.box_radius();
        let d = (r * 1.9).max(3.5);
        self.camera.speed = (r * 0.65).clamp(2.5, 18.0);
        match preset {
            CameraPreset::Iso => {
                self.camera.pos = [c[0] + d * 0.65, c[1] + d * 0.55, c[2] - d * 0.65];
                self.camera.look_at(c);
            }
            CameraPreset::Top => {
                self.camera.pos = [c[0], c[1] + d * 1.25, c[2] - 0.05];
                self.camera.look_at(c);
            }
            CameraPreset::Front => {
                self.camera.pos = [c[0], c[1] + r * 0.1, c[2] - d];
                self.camera.look_at(c);
            }
            CameraPreset::Side => {
                self.camera.pos = [c[0] + d, c[1] + r * 0.1, c[2]];
                self.camera.look_at(c);
            }
            CameraPreset::Inside => {
                let floor_y = self.bounds_min[1];
                let eye_y = (floor_y + 1.55).min(self.bounds_max[1] - 0.4).max(floor_y + 0.5);
                self.camera.pos = [c[0], eye_y, c[2] - (r * 0.25).min(1.5)];
                self.camera.look_at([c[0], eye_y, c[2] + 1.0]);
                self.camera.speed = 3.5;
            }
        }
    }

    fn proto_to_world(&self, p: [f32; 3]) -> [f32; 3] {
        [p[0] + self.center[0], p[1] + self.center[1], p[2] + self.center[2]]
    }

    fn world_to_proto(&self, w: [f32; 3]) -> [f32; 3] {
        [w[0] - self.center[0], w[1] - self.center[1], w[2] - self.center[2]]
    }
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.loot_proto.open {
        return;
    }
    if crate::mission_ui::mission(app).is_none() {
        app.loot_proto.open = false;
        return;
    }
    // Keep requesting the model if it was evicted or not loaded yet.
    let model_key = (app.loot_proto.model_id, 0u8);
    let model_ready = app.renderer.lock().is_ok_and(|r| r.has_model(model_key));
    if !model_ready && !app.loot_proto.model.is_empty() {
        app.loader.request(model_key, &app.loot_proto.model);
    }

    let limits = proto::limits_lists(crate::mission_ui::mission(app));
    let title = tf("Loot points — {0}", &[&app.loot_proto.group.name]);
    let renderer = app.renderer.clone();
    let pick_mesh = app.pick_meshes.get(&app.loot_proto.model_id).cloned();
    let keys = app.settings.keys.clone();

    let mut open = app.loot_proto.open;
    let mut save_requested = false;
    let mut remove_requested = false;

    egui::Window::new(title)
        .id(egui::Id::new("loot_proto_editor_window"))
        .open(&mut open)
        .default_size([1020.0, 640.0])
        .min_size([780.0, 480.0])
        .resizable(true)
        .show(ctx, |ui| {
            let content_h = ui.available_height().max(320.0);
            ui.columns(2, |cols| {
                draw_preview_column(
                    &mut app.loot_proto,
                    &mut cols[0],
                    content_h,
                    &renderer,
                    model_key,
                    model_ready,
                    pick_mesh.as_deref(),
                    &keys,
                );

                draw_editor_column(
                    &mut app.loot_proto,
                    &mut cols[1],
                    content_h,
                    &limits,
                    &mut save_requested,
                    &mut remove_requested,
                );
            });
        });

    app.loot_proto.open = open;

    if save_requested {
        save_to_mission(app);
    }
    if remove_requested {
        remove_from_mission(app);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_preview_column(
    st: &mut LootProtoUi,
    ui: &mut egui::Ui,
    content_h: f32,
    renderer: &std::sync::Arc<std::sync::Mutex<render::Renderer>>,
    model_key: (u32, u8),
    model_ready: bool,
    pick_mesh: Option<&crate::viewport::PickMesh>,
    keys: &crate::keys::KeyMap,
) {
    // Camera angle presets & mode controls
    ui.horizontal_wrapped(|ui| {
        if ui.small_button(t("Iso")).clicked() {
            st.set_camera_preset(CameraPreset::Iso);
        }
        if ui.small_button(t("Top")).clicked() {
            st.set_camera_preset(CameraPreset::Top);
        }
        if ui.small_button(t("Front")).clicked() {
            st.set_camera_preset(CameraPreset::Front);
        }
        if ui.small_button(t("Side")).clicked() {
            st.set_camera_preset(CameraPreset::Side);
        }
        if ui.small_button(t("Inside")).clicked() {
            st.set_camera_preset(CameraPreset::Inside);
        }
        ui.separator();
        ui.checkbox(&mut st.show_ranges, t("Show ranges"));
        ui.separator();
        ui.toggle_value(&mut st.add_mode, icons::with(icons::PLUS, &t("Click 3D to add")))
            .on_hover_text(t("Click a surface in the 3D preview to place a new loot point there."));
    });

    // Horizontal roof slice slider (instant on GPU)
    ui.horizontal(|ui| {
        ui.checkbox(&mut st.clip_enabled, t("Cut roof"))
            .on_hover_text(t("Slice the model horizontally to see inside rooms and floors."));
        let lo = st.bounds_min[1];
        let hi = st.bounds_max[1].max(lo + 0.2);
        ui.add_enabled(
            st.clip_enabled,
            egui::Slider::new(&mut st.clip_y, lo..=hi).suffix(" m").fixed_decimals(2),
        );
    });

    ui.add_space(4.0);

    // 3D OpenGL viewport sized strictly inside `content_h` so the resizable window never grows by itself.
    let view_h = (content_h - 104.0).max(200.0);
    let avail = Vec2::new(ui.available_width().max(260.0), view_h);
    let (rect, resp) = ui.allocate_exact_size(avail, egui::Sense::click_and_drag());
    let ctx = ui.ctx();
    let v = View::perspective_near(&st.camera, rect, 0.08, 2000.0);

    // ---------------- 3D camera controls (matching terrain 3D viewport) ----------------
    let dt = ctx.input(|i| i.stable_dt).min(0.1);
    let hover = resp.hovered() || resp.dragged();
    let typing = ctx.wants_keyboard_input();
    let (scroll, _mods) = ctx.input(|i| (i.smooth_scroll_delta.y, i.modifiers));
    let looking = resp.dragged_by(PointerButton::Secondary);

    if looking {
        let d = resp.drag_delta();
        st.camera.yaw += d.x * 0.0038;
        st.camera.pitch = (st.camera.pitch - d.y * 0.0038).clamp(-1.54, 1.54);
        if scroll != 0.0 {
            st.camera.speed = (st.camera.speed * (scroll * 0.002).exp()).clamp(0.5, 60.0);
        }
        ctx.request_repaint();
    } else if hover && scroll != 0.0 {
        let f = st.camera.forward();
        let dist = sub(st.camera.pos, st.box_center());
        let step = (dot(dist, dist).sqrt() * 0.0035).clamp(0.05, 0.6);
        st.camera.pos = add(st.camera.pos, mul(f, scroll * step));
        ctx.request_repaint();
    }
    if resp.dragged_by(PointerButton::Middle) {
        let d = resp.drag_delta();
        let k = st.camera.speed * 0.008;
        st.camera.pos = add(add(st.camera.pos, mul(v.right, -d.x * k)), mul(v.up, d.y * k));
        ctx.request_repaint();
    }
    if (looking || hover) && !typing {
        let (mut mv, mut fast) = ([0.0f32; 3], false);
        ctx.input(|i| {
            let f = st.camera.forward();
            let r = st.camera.right();
            fast = keys.held(Hold::Fast, i.modifiers);
            if keys.down(i, Action::CameraForward) || i.key_down(Key::ArrowUp) { mv = add(mv, f); }
            if keys.down(i, Action::CameraBack) || i.key_down(Key::ArrowDown) { mv = add(mv, mul(f, -1.0)); }
            if keys.down(i, Action::CameraRight) || i.key_down(Key::ArrowRight) { mv = add(mv, r); }
            if keys.down(i, Action::CameraLeft) || i.key_down(Key::ArrowLeft) { mv = add(mv, mul(r, -1.0)); }
            if keys.down(i, Action::CameraUp) { mv = add(mv, [0.0, 1.0, 0.0]); }
            if keys.down(i, Action::CameraDown) { mv = add(mv, [0.0, -1.0, 0.0]); }
            // Press F to frame the selected point or building.
            if i.key_pressed(Key::F) {
                let target = st.selected_point
                    .and_then(|pi| st.group.containers.get(st.selected_container)?.points.get(pi))
                    .map(|pt| st.proto_to_world(pt.pos))
                    .unwrap_or_else(|| st.box_center());
                let f = st.camera.forward();
                st.camera.pos = sub(target, mul(f, 3.0));
            }
        });
        if dot(mv, mv) > 0.0 {
            let speed = st.camera.speed * if fast { 3.0 } else { 1.0 };
            st.camera.pos = add(st.camera.pos, mul(norm(mv), speed * dt));
            ctx.request_repaint();
        }
    }

    // Recompute view after camera movement so GPU render & overlays stay in perfect lockstep.
    let v = View::perspective_near(&st.camera, rect, 0.08, 2000.0);
    let painter = ui.painter_at(rect);

    // GPU 3D render callback (with sky lighting and depth-tested ground slab)
    let c = st.box_center();
    painter.add(render::preview_callback(
        renderer.clone(),
        ModelPreviewFrame {
            view: v,
            model_key,
            clip_y: st.clip_param(),
            floor: [c[0], st.bounds_min[1], c[2], st.box_radius()],
        },
    ));
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, Color32::from_gray(55)), egui::StrokeKind::Inside);

    if !model_ready {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, t("Loading 3D preview..."), egui::FontId::proportional(13.0), Color32::GRAY);
        ctx.request_repaint();
    }

    // Draw 3D loot points, range rings, and height stalks
    for (ci, container) in st.group.containers.iter().enumerate() {
        let base_col = CONTAINER_COLORS[ci % CONTAINER_COLORS.len()];
        for (pi, pt) in container.points.iter().enumerate() {
            let selected = ci == st.selected_container && st.selected_point == Some(pi);
            let col = if selected { Color32::from_rgb(255, 240, 80) } else { base_col };
            let w_foot = st.proto_to_world(pt.pos);
            let w_top = [w_foot[0], w_foot[1] + pt.height, w_foot[2]];

            if st.show_ranges && pt.range > 0.01 {
                let stroke_w = if selected { 2.0_f32 } else { 1.2_f32 };
                let ring_col = Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), if selected { 235 } else { 155 });
                let mut prev = [w_foot[0] + pt.range, w_foot[1], w_foot[2]];
                for k in 1..=20 {
                    let a = k as f32 * std::f32::consts::TAU / 20.0;
                    let cur = [w_foot[0] + pt.range * a.cos(), w_foot[1], w_foot[2] + pt.range * a.sin()];
                    if let Some((p0, p1)) = v.segment(prev, cur) {
                        painter.line_segment([p0, p1], Stroke::new(stroke_w, ring_col));
                    }
                    prev = cur;
                }
            }

            if let Some((p0, p1)) = v.segment(w_foot, w_top) {
                painter.line_segment(
                    [p0, p1],
                    Stroke::new(if selected { 2.2_f32 } else { 1.3_f32 }, Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), 200)),
                );
            }

            if let Some(foot) = v.project(w_foot) {
                if rect.contains(foot) {
                    let r = if selected { 6.0_f32 } else { 4.2_f32 };
                    painter.circle_filled(foot, r, col);
                    painter.circle_stroke(foot, r, Stroke::new(1.2_f32, Color32::BLACK));
                    if pt.flags & 32 != 0 {
                        painter.circle_stroke(foot, r + 2.5, Stroke::new(1.2_f32, Color32::from_rgb(255, 210, 90)));
                    }
                }
            }
        }
    }

    // LPM click: either place a new loot point on the 3D surface or select nearest point
    if resp.clicked_by(PointerButton::Primary) {
        if let Some(ptr) = resp.interact_pointer_pos() {
            if st.add_mode {
                if let Some(mesh) = pick_mesh {
                    let (ro, rd) = v.ray(ptr);
                    if let Some(hit) = mesh.raycast(ro, rd, st.clip_param()) {
                        if st.group.containers.is_empty() {
                            st.group.containers.push(ContainerProto::default());
                            st.selected_container = 0;
                        }
                        let ci = st.selected_container.min(st.group.containers.len() - 1);
                        let pt = LootPoint {
                            pos: st.world_to_proto(hit),
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
                        if let Some(s) = v.project(st.proto_to_world(pt.pos)) {
                            let d = s.distance(ptr);
                            if d <= 14.0 && best.is_none_or(|b| d < b.0) {
                                best = Some((d, ci, pi));
                            }
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

    ui.add_space(4.0);
    let hint = if st.add_mode {
        t("RMB + WASD/QE: fly · Click a surface to add a loot point")
    } else {
        t("RMB + WASD/QE: fly · Wheel: zoom/speed · MMB: pan · Click: select")
    };
    ui.weak(hint);
    ui.weak(&st.model);
}

fn draw_editor_column(
    st: &mut LootProtoUi,
    ui: &mut egui::Ui,
    content_h: f32,
    limits: &proto::LimitsLists,
    save_requested: &mut bool,
    remove_requested: &mut bool,
) {
    let mut header_h = 0.0;
    if !st.exists_in_proto {
        ui.colored_label(Color32::from_rgb(160, 205, 250), t("Not yet in mapgroupproto.xml — edit and save to add it."));
        ui.add_space(2.0);
        header_h = 22.0;
    }

    egui::ScrollArea::vertical().id_salt("loot_proto_editor_scroll").max_height((content_h - 46.0 - header_h).max(160.0)).show(ui, |ui| {
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
        if ui.button(icons::with(icons::FLOPPY_DISK, &t("Save to mapgroupproto.xml"))).clicked() {
            *save_requested = true;
        }
        if st.exists_in_proto && ui.button(t("Remove from mapgroupproto.xml")).clicked() {
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
