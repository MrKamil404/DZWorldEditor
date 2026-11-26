//! Standalone 3D loot point preview and editor for `mapgroupproto.xml`.
//!
//! Opened from the Content Browser context menu on any building/model (`Pick::Model`) when
//! the project has a server mission. Renders the building and its visual proxies (furniture,
//! tables, shelves) directly on the GPU via OpenGL (`render::preview_callback`) with the same
//! free-fly 3D camera controls as the terrain viewport, an instant GPU horizontal roof-slice
//! plane (`clip_y`), a 3D XYZ translation gizmo for selected points, and fast surface
//! raycasting across both building and proxy geometry.
use crate::app::App;
use crate::camera::{add, dot, mul, norm, sub, Camera, View};
use crate::i18n::{t, tf};
use crate::icons;
use crate::keys::{Action, Hold};
use crate::proxies::ProxyPlace;
use crate::render::{self, ModelPreviewFrame};
use crate::theme::{self, check_row, details_head, list_row, row, section, subsection, tool_button, vector_row, ValueField};
use crate::viewport::{PickMesh, PROXY_BASE};
use dz_core::mission::proto::{self, ContainerProto, DispatchProxy, GroupProto, LootPoint};
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Stroke, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

/// Reserved model-id namespace for isolated 3D loot previews (never collides with map/mission/proxy ids).
pub(crate) const PREVIEW_ID_BASE: u32 = 0x9000_0000;
/// Reserved model-id namespace for `<dispatch><proxy>` 3D models shown in the loot editor.
const DISPATCH_ID_BASE: u32 = 0x9400_0000;

const CONTAINER_COLORS: [Color32; 6] = [
    Color32::from_rgb(76, 195, 247),  // cyan
    Color32::from_rgb(129, 199, 132), // green
    Color32::from_rgb(255, 183, 77),  // amber
    Color32::from_rgb(240, 98, 146),  // pink
    Color32::from_rgb(186, 104, 200), // violet
    Color32::from_rgb(255, 138, 101), // coral
];

const IDENTITY_INSTANCE: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0,
    1.0, 1.0, 1.0, 0.0,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum GizmoHandle {
    Axis(usize),
    Range,
    Height,
}

#[derive(Clone, Copy)]
struct GizmoDrag {
    handle: GizmoHandle,
    origin_world: [f32; 3],
    start_t: f32,
    start_val: f32,
}

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
    selected_dispatch: Option<usize>,
    camera: Camera,
    clip_enabled: bool,
    clip_y: f32,
    add_mode: bool,
    show_ranges: bool,
    show_proxies: bool,
    show_dispatch: bool,
    dispatch_model_ids: HashMap<String, u32>,
    next_dispatch_id: u32,
    panel_width: f32,
    gizmo_drag: Option<GizmoDrag>,
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
            selected_dispatch: None,
            camera: Camera { pos: [8.0, 6.0, 8.0], yaw: 0.0, pitch: -0.4, fov: 65f32.to_radians(), speed: 6.0 },
            clip_enabled: false,
            clip_y: 2.5,
            add_mode: false,
            show_ranges: true,
            show_proxies: true,
            show_dispatch: true,
            dispatch_model_ids: HashMap::new(),
            next_dispatch_id: DISPATCH_ID_BASE,
            panel_width: 360.0,
            gizmo_drag: None,
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
    ui.selected_dispatch = None;
    ui.group = group;
    ui.gizmo_drag = None;
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
        proto::proto_to_model(p, self.center)
    }

    fn world_to_proto(&self, w: [f32; 3]) -> [f32; 3] {
        proto::model_to_proto(w, self.center)
    }
}

fn proxy_instance_matrix(p: &ProxyPlace) -> [f32; 16] {
    [
        p.axes[0][0], p.axes[1][0], p.axes[2][0], p.at[0],
        p.axes[0][1], p.axes[1][1], p.axes[2][1], p.at[1],
        p.axes[0][2], p.axes[1][2], p.axes[2][2], p.at[2],
        1.0, 1.0, 1.0, 0.0,
    ]
}

/// Builds a GPU instance matrix for a `<dispatch><proxy>` positioned at `world_pos` with `rpy`
/// degrees (`[pitch, yaw, roll]` directly in P3D model space).
fn dispatch_instance_matrix(world_pos: [f32; 3], rpy: [f32; 3]) -> [f32; 16] {
    let m = dz_core::objects::rotation(rpy[1], rpy[0], rpy[2], 1.0);
    [
        m[0][0], m[1][0], m[2][0], world_pos[0],
        m[0][1], m[1][1], m[2][1], world_pos[1],
        m[0][2], m[1][2], m[2][2], world_pos[2],
        1.0, 1.0, 1.0, 0.0,
    ]
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.loot_proto.open {
        return;
    }
    if crate::mission_ui::mission(app).is_none() {
        app.loot_proto.open = false;
        return;
    }

    // Load building LOD 0 and its visual proxies (furniture, shelves, tables).
    app.proxies.poll();
    let model_key = (app.loot_proto.model_id, 0u8);
    let model_ready = app.renderer.lock().is_ok_and(|r| r.has_model(model_key));
    if !model_ready && !app.loot_proto.model.is_empty() {
        app.loader.request(model_key, &app.loot_proto.model);
    }

    let mut instances: Vec<((u32, u8), [f32; 16])> = Vec::new();
    instances.push((model_key, IDENTITY_INSTANCE));
    let mut proxy_list: Option<Arc<Vec<ProxyPlace>>> = None;
    if app.loot_proto.show_proxies && !app.loot_proto.model.is_empty() {
        if let Some(list) = app.proxies.get(&app.loot_proto.model, &app.db, ctx) {
            let renderer = app.renderer.lock().ok();
            for p in list.iter() {
                let pid = PROXY_BASE + p.model;
                let key = (pid, 0u8);
                let has = renderer.as_ref().is_some_and(|r| r.has_model(key));
                if !has {
                    if let Some(path) = app.proxies.models.get(p.model as usize) {
                        app.loader.request(key, path);
                    }
                }
                instances.push((key, proxy_instance_matrix(p)));
            }
            proxy_list = Some(list);
        }
    }

    // Resolve `<dispatch><proxy>` class names to P3D models via `ClassIndex` and queue GPU uploads.
    let classes = crate::mission_ui::class_index(app);
    let dispatch_resolved: Vec<Option<String>> = app.loot_proto.group.dispatch.iter().map(|pr| {
        let name = pr.kind.trim();
        if name.is_empty() {
            return None;
        }
        classes.as_ref().and_then(|idx| idx.model_of(name))
    }).collect();
    if app.loot_proto.show_dispatch {
        let renderer = app.renderer.lock().ok();
        let center = app.loot_proto.center;
        for (pr, resolved) in app.loot_proto.group.dispatch.iter().zip(&dispatch_resolved) {
            let Some(path) = resolved else { continue };
            let key_str = path.to_ascii_lowercase();
            let mid = if let Some(&id) = app.loot_proto.dispatch_model_ids.get(&key_str) {
                id
            } else {
                let id = app.loot_proto.next_dispatch_id;
                app.loot_proto.next_dispatch_id = app.loot_proto.next_dispatch_id.wrapping_add(1);
                app.loot_proto.dispatch_model_ids.insert(key_str, id);
                id
            };
            let key = (mid, 0u8);
            let has = renderer.as_ref().is_some_and(|r| r.has_model(key));
            if !has {
                app.loader.request(key, path);
            }
            let world_pos = proto::proto_to_model(pr.pos, center);
            instances.push((key, dispatch_instance_matrix(world_pos, pr.rpy)));
        }
    }

    // Collect PickMeshes needed for building + proxy surface raycasting.
    let building_pick = app.pick_meshes.get(&app.loot_proto.model_id).cloned();
    let mut proxy_picks: HashMap<u32, Arc<PickMesh>> = HashMap::new();
    if let Some(list) = &proxy_list {
        for p in list.iter() {
            let pid = PROXY_BASE + p.model;
            if let Some(pm) = app.pick_meshes.get(&pid) {
                proxy_picks.insert(p.model, pm.clone());
            }
        }
    }

    let limits = proto::limits_lists(crate::mission_ui::mission(app));
    let title = tf("Loot points — {0}", &[&app.loot_proto.group.name]);
    let renderer = app.renderer.clone();
    let keys = app.settings.keys.clone();

    let mut open = app.loot_proto.open;
    let mut save_requested = false;
    let mut remove_requested = false;

    theme::PopupWindow::new(egui::Id::new("loot_proto_editor_window"), &title)
        .default_size([1120.0, 680.0])
        .min_size([760.0, 460.0])
        .show(ctx, &mut open, |ui| {
            let avail = ui.available_rect_before_wrap();
            let total_w = avail.width().max(1.0);
            let total_h = avail.height().max(1.0);
            const SPLITTER_W: f32 = 6.0;
            let max_left = (total_w - 320.0 - SPLITTER_W).clamp(260.0, 580.0);
            let left_w = app.loot_proto.panel_width.clamp(260.0, max_left);
            let right_w = (total_w - left_w - SPLITTER_W).max(200.0);

            // Reserve exactly the available area so neither column can ever cause a resize feedback loop.
            let body_rect = egui::Rect::from_min_size(avail.min, Vec2::new(total_w, total_h));
            ui.allocate_rect(body_rect, egui::Sense::hover());

            let left_rect = egui::Rect::from_min_size(body_rect.min, Vec2::new(left_w, total_h));
            let split_rect = egui::Rect::from_min_size(
                Pos2::new(left_rect.right(), body_rect.top()),
                Vec2::new(SPLITTER_W, total_h),
            );
            let right_rect = egui::Rect::from_min_size(
                Pos2::new(split_rect.right(), body_rect.top()),
                Vec2::new(right_w, total_h),
            );

            // Left column: mapgroupproto.xml inspector
            let mut left_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(left_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            left_ui.set_clip_rect(left_rect.intersect(ui.clip_rect()));
            draw_editor_panel(
                &mut app.loot_proto,
                &mut left_ui,
                total_h,
                &limits,
                &dispatch_resolved,
                &mut save_requested,
                &mut remove_requested,
            );

            // Draggable vertical splitter bar
            let split_resp = ui.interact(split_rect, ui.id().with("loot_proto_splitter"), egui::Sense::drag());
            if split_resp.hovered() || split_resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if split_resp.dragged() {
                app.loot_proto.panel_width = (app.loot_proto.panel_width + split_resp.drag_delta().x).clamp(260.0, max_left);
            }
            let bar_col = if split_resp.hovered() || split_resp.dragged() { theme::ACCENT } else { theme::BORDER };
            ui.painter().rect_filled(split_rect.shrink2(Vec2::new(2.0, 0.0)), 0.0, bar_col);

            // Right column: 3D GPU preview
            let mut right_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(right_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            right_ui.set_clip_rect(right_rect.intersect(ui.clip_rect()));
            draw_preview_panel(
                &mut app.loot_proto,
                &mut right_ui,
                total_h,
                &renderer,
                instances,
                model_ready,
                building_pick.as_deref(),
                proxy_list.as_deref().map(Vec::as_slice),
                &proxy_picks,
                &keys,
            );
        });

    app.loot_proto.open = open;

    if save_requested {
        save_to_mission(app);
    }
    if remove_requested {
        remove_from_mission(app);
    }
}

/// Raycasts across both the building mesh and all visible proxy meshes (furniture, tables, shelves),
/// returning the closest world-space hit point below `clip_y`.
fn pick_scene_surface(
    ro: [f32; 3],
    rd: [f32; 3],
    clip_y: Option<f32>,
    building: Option<&PickMesh>,
    proxies: Option<&[ProxyPlace]>,
    proxy_picks: &HashMap<u32, Arc<PickMesh>>,
) -> Option<[f32; 3]> {
    let mut best: Option<(f32, [f32; 3])> = None;
    if let Some(m) = building {
        if let Some(hit) = m.raycast(ro, rd, clip_y) {
            best = Some(hit);
        }
    }
    if let Some(list) = proxies {
        for p in list {
            if let Some(pm) = proxy_picks.get(&p.model) {
                if let Some(hit) = pm.raycast_transformed(ro, rd, p.axes, p.at, clip_y) {
                    if best.is_none_or(|b| hit.0 < b.0) {
                        best = Some(hit);
                    }
                }
            }
        }
    }
    best.map(|(_, h)| h)
}

/// Parameter `t` of the closest point on axis line `origin + t * dir` to camera ray `(ro, rd)`.
fn axis_param(origin: [f32; 3], dir: [f32; 3], ro: [f32; 3], rd: [f32; 3]) -> Option<f32> {
    let w0 = sub(origin, ro);
    let b = dot(dir, rd);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-5 {
        return None;
    }
    let d = dot(dir, w0);
    let e = dot(rd, w0);
    Some((b * e - d) / denom)
}

/// Shortest distance in pixels from point `p` to 2D segment `(a, b)`.
fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 < 1e-5 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

fn gizmo_length(eye: [f32; 3], origin: [f32; 3]) -> f32 {
    let d = sub(origin, eye);
    (dot(d, d).sqrt() * 0.14).clamp(0.25, 2.0)
}

fn gizmo_hit(v: &View, origin: [f32; 3], range: f32, height: f32, ptr: Pos2) -> Option<GizmoHandle> {
    // Check top height grip first so it never conflicts with the Y translation arrow below it.
    let top = [origin[0], origin[1] + height, origin[2]];
    if let Some(tp) = v.project(top) {
        if tp.distance(ptr) <= 10.0 {
            return Some(GizmoHandle::Height);
        }
    }
    let len = gizmo_length(v.eye, origin);
    let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut best: Option<(f32, GizmoHandle)> = None;
    for (i, dir) in axes.iter().enumerate() {
        let end = add(origin, mul(*dir, len));
        if let Some((a, b)) = v.segment(origin, end) {
            let d = dist_to_segment(ptr, a, b);
            if d <= 10.0 && best.is_none_or(|cur| d < cur.0) {
                best = Some((d, GizmoHandle::Axis(i)));
            }
        }
    }
    if best.is_some() {
        return best.map(|(_, h)| h);
    }
    // Check the horizontal range circle (and its 4 cardinal grip handles) around the selected point.
    if range > 0.01 {
        let mut prev = [origin[0] + range, origin[1], origin[2]];
        let mut best_r: Option<f32> = None;
        for k in 1..=20 {
            let a = k as f32 * std::f32::consts::TAU / 20.0;
            let cur = [origin[0] + range * a.cos(), origin[1], origin[2] + range * a.sin()];
            if let Some((p0, p1)) = v.segment(prev, cur) {
                let d = dist_to_segment(ptr, p0, p1);
                if d <= 9.0 && best_r.is_none_or(|cur_d| d < cur_d) {
                    best_r = Some(d);
                }
            }
            prev = cur;
        }
        if best_r.is_some() {
            return Some(GizmoHandle::Range);
        }
    }
    None
}

fn draw_gizmo(painter: &egui::Painter, v: &View, origin: [f32; 3], range: f32, height: f32, active: Option<GizmoHandle>) {
    let len = gizmo_length(v.eye, origin);
    let axes = [
        ([1.0, 0.0, 0.0], Color32::from_rgb(235, 75, 75), "X"),
        ([0.0, 1.0, 0.0], Color32::from_rgb(80, 220, 95), "Y"),
        ([0.0, 0.0, 1.0], Color32::from_rgb(80, 145, 245), "Z"),
    ];
    for (i, (dir, base_col, label)) in axes.iter().enumerate() {
        let lit = active == Some(GizmoHandle::Axis(i));
        let col = if lit { Color32::from_rgb(255, 240, 80) } else { *base_col };
        let end = add(origin, mul(*dir, len));
        if let Some((a, b)) = v.segment(origin, end) {
            painter.line_segment([a, b], Stroke::new(if lit { 3.5_f32 } else { 2.4_f32 }, col));
            let d = b - a;
            let dl = d.length();
            if dl > 6.0 {
                let u = d / dl;
                let n = Vec2::new(-u.y, u.x);
                let head = vec![b + u * 3.0, b - u * 8.0 + n * 4.5, b - u * 8.0 - n * 4.5];
                painter.add(egui::Shape::convex_polygon(head, col, Stroke::NONE));
                painter.text(b + u * 11.0, egui::Align2::CENTER_CENTER, *label, egui::FontId::proportional(11.0), col);
            }
        }
    }

    // Range ring resize handles on the selected point (always drawn on the selected point).
    if range > 0.01 {
        let lit = active == Some(GizmoHandle::Range);
        let ring_col = if lit { Color32::from_rgb(120, 245, 255) } else { Color32::from_rgb(255, 240, 80) };
        if lit {
            let mut prev = [origin[0] + range, origin[1], origin[2]];
            for k in 1..=24 {
                let a = k as f32 * std::f32::consts::TAU / 24.0;
                let cur = [origin[0] + range * a.cos(), origin[1], origin[2] + range * a.sin()];
                if let Some((p0, p1)) = v.segment(prev, cur) {
                    painter.line_segment([p0, p1], Stroke::new(3.0_f32, ring_col));
                }
                prev = cur;
            }
        }
        let grip_r = if lit { 5.0_f32 } else { 3.8_f32 };
        for offset in [[range, 0.0, 0.0], [-range, 0.0, 0.0], [0.0, 0.0, range], [0.0, 0.0, -range]] {
            if let Some(p) = v.project(add(origin, offset)) {
                painter.circle_filled(p, grip_r, ring_col);
                painter.circle_stroke(p, grip_r, Stroke::new(1.0_f32, Color32::BLACK));
            }
        }
    }

    // Height resize handle at the top of the spawn cylinder stalk.
    let top = [origin[0], origin[1] + height, origin[2]];
    let h_lit = active == Some(GizmoHandle::Height);
    let h_col = if h_lit { Color32::from_rgb(120, 245, 255) } else { Color32::from_rgb(255, 210, 80) };
    if h_lit && range > 0.01 {
        // Show top cap ring while resizing height so ceiling clearance is immediately visible.
        let mut prev = [top[0] + range, top[1], top[2]];
        for k in 1..=24 {
            let a = k as f32 * std::f32::consts::TAU / 24.0;
            let cur = [top[0] + range * a.cos(), top[1], top[2] + range * a.sin()];
            if let Some((p0, p1)) = v.segment(prev, cur) {
                painter.line_segment([p0, p1], Stroke::new(2.0_f32, h_col));
            }
            prev = cur;
        }
    }
    if let Some(tp) = v.project(top) {
        let d = if h_lit { 6.5_f32 } else { 5.0_f32 };
        let diamond = vec![
            Pos2::new(tp.x, tp.y - d),
            Pos2::new(tp.x + d, tp.y),
            Pos2::new(tp.x, tp.y + d),
            Pos2::new(tp.x - d, tp.y),
        ];
        painter.add(egui::Shape::convex_polygon(diamond, h_col, Stroke::new(1.2_f32, Color32::BLACK)));
        painter.text(tp + Vec2::new(10.0, 0.0), egui::Align2::LEFT_CENTER, "H", egui::FontId::proportional(11.0), h_col);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_preview_panel(
    st: &mut LootProtoUi,
    ui: &mut egui::Ui,
    total_h: f32,
    renderer: &Arc<std::sync::Mutex<render::Renderer>>,
    instances: Vec<((u32, u8), [f32; 16])>,
    model_ready: bool,
    building_pick: Option<&PickMesh>,
    proxy_list: Option<&[ProxyPlace]>,
    proxy_picks: &HashMap<u32, Arc<PickMesh>>,
    keys: &crate::keys::KeyMap,
) {
    let start_y = ui.cursor().top();

    // Top toolbar inside the 3D column
    egui::Frame::new().inner_margin(egui::Margin { left: 6, right: 0, top: 0, bottom: 4 }).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            if ui.small_button(t("Iso")).clicked() { st.set_camera_preset(CameraPreset::Iso); }
            if ui.small_button(t("Top")).clicked() { st.set_camera_preset(CameraPreset::Top); }
            if ui.small_button(t("Front")).clicked() { st.set_camera_preset(CameraPreset::Front); }
            if ui.small_button(t("Side")).clicked() { st.set_camera_preset(CameraPreset::Side); }
            if ui.small_button(t("Inside")).clicked() { st.set_camera_preset(CameraPreset::Inside); }
            ui.separator();
            ui.checkbox(&mut st.show_ranges, t("Show ranges"));
            ui.checkbox(&mut st.show_proxies, t("Proxies"));
            ui.checkbox(&mut st.show_dispatch, t("Dispatch"));
            ui.separator();
            ui.toggle_value(&mut st.add_mode, icons::with(icons::PLUS, &t("Click 3D to add")))
                .on_hover_text(t("Click a surface in the 3D preview to place a new loot point there."));
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut st.clip_enabled, t("Cut roof"))
                .on_hover_text(t("Slice the model horizontally to see inside rooms and floors."));
            let lo = st.bounds_min[1];
            let hi = st.bounds_max[1].max(lo + 0.2);
            ui.add_enabled(
                st.clip_enabled,
                ValueField::new(&mut st.clip_y, lo..=hi).suffix(" m").step_by(0.05),
            );
        });
    });

    // 3D OpenGL viewport fills remaining height minus the 2-line footer (never overflows `total_h`).
    let used_top = (ui.cursor().top() - start_y).max(0.0);
    let footer_h = 42.0;
    let view_h = (total_h - used_top - footer_h).max(120.0);
    let view_w = (ui.available_width() - 6.0).max(200.0);
    let (rect, resp) = ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.allocate_exact_size(Vec2::new(view_w, view_h), egui::Sense::click_and_drag())
    }).inner;
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

            let v = View::perspective_near(&st.camera, rect, 0.08, 2000.0);

            // ---------------- Gizmo interaction (LPM drag on X/Y/Z axis, Range ring, or Height handle) ----------------
            let selected_info: Option<([f32; 3], f32, f32, bool)> = if let Some(pi) = st.selected_point {
                st.group.containers.get(st.selected_container)
                    .and_then(|c| c.points.get(pi))
                    .map(|pt| (st.proto_to_world(pt.pos), pt.range, pt.height, false))
            } else if let Some(di) = st.selected_dispatch {
                st.group.dispatch.get(di).map(|pr| (st.proto_to_world(pr.pos), 0.0, 0.0, true))
            } else {
                None
            };

            let hover_pos = resp.hover_pos();
            let hover_handle = match (&st.gizmo_drag, selected_info, hover_pos) {
                (Some(drag), _, _) => Some(drag.handle),
                (None, Some((origin, range, height, _)), Some(ptr)) => gizmo_hit(&v, origin, range, height, ptr),
                _ => None,
            };

            if resp.drag_started_by(PointerButton::Primary) {
                if let (Some((origin, range, height, _)), Some(ptr)) = (selected_info, resp.interact_pointer_pos()) {
                    if let Some(handle) = gizmo_hit(&v, origin, range, height, ptr) {
                        match handle {
                            GizmoHandle::Axis(axis) => {
                                let mut dir = [0.0f32; 3];
                                dir[axis] = 1.0;
                                let (ro, rd) = v.ray(ptr);
                                if let Some(t) = axis_param(origin, dir, ro, rd) {
                                    st.gizmo_drag = Some(GizmoDrag { handle, origin_world: origin, start_t: t, start_val: 0.0 });
                                }
                            }
                            GizmoHandle::Range => {
                                st.gizmo_drag = Some(GizmoDrag { handle, origin_world: origin, start_t: 0.0, start_val: range });
                            }
                            GizmoHandle::Height => {
                                let (ro, rd) = v.ray(ptr);
                                if let Some(t) = axis_param(origin, [0.0, 1.0, 0.0], ro, rd) {
                                    st.gizmo_drag = Some(GizmoDrag { handle, origin_world: origin, start_t: t, start_val: height });
                                }
                            }
                        }
                    }
                }
            }
            if let Some(drag) = st.gizmo_drag {
                if resp.dragged_by(PointerButton::Primary) {
                    if let Some(ptr) = resp.interact_pointer_pos() {
                        let (ro, rd) = v.ray(ptr);
                        match drag.handle {
                            GizmoHandle::Axis(axis) => {
                                let mut dir = [0.0f32; 3];
                                dir[axis] = 1.0;
                                if let Some(t) = axis_param(drag.origin_world, dir, ro, rd) {
                                    let mut w = drag.origin_world;
                                    w[axis] += t - drag.start_t;
                                    let new_proto = st.world_to_proto(w);
                                    if let Some(pi) = st.selected_point {
                                        if let Some(pt) = st.group.containers.get_mut(st.selected_container).and_then(|c| c.points.get_mut(pi)) {
                                            pt.pos = new_proto;
                                            st.dirty = true;
                                        }
                                    } else if let Some(di) = st.selected_dispatch {
                                        if let Some(pr) = st.group.dispatch.get_mut(di) {
                                            pr.pos = new_proto;
                                            st.dirty = true;
                                        }
                                    }
                                }
                            }
                            GizmoHandle::Range => {
                                if rd[1].abs() > 1e-4 {
                                    let t = (drag.origin_world[1] - ro[1]) / rd[1];
                                    if t > 0.01 {
                                        let hit = add(ro, mul(rd, t));
                                        let r = (hit[0] - drag.origin_world[0]).hypot(hit[2] - drag.origin_world[2]).clamp(0.05, 5.0);
                                        if let Some(pi) = st.selected_point {
                                            if let Some(pt) = st.group.containers.get_mut(st.selected_container).and_then(|c| c.points.get_mut(pi)) {
                                                pt.range = r;
                                                st.dirty = true;
                                            }
                                        }
                                    }
                                }
                            }
                            GizmoHandle::Height => {
                                if let Some(t) = axis_param(drag.origin_world, [0.0, 1.0, 0.0], ro, rd) {
                                    let h = (drag.start_val + (t - drag.start_t)).clamp(0.1, 5.0);
                                    if let Some(pi) = st.selected_point {
                                        if let Some(pt) = st.group.containers.get_mut(st.selected_container).and_then(|c| c.points.get_mut(pi)) {
                                            pt.height = h;
                                            st.dirty = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    ctx.request_repaint();
                } else {
                    st.gizmo_drag = None;
                }
            }

            let painter = ui.painter_at(rect);

            // GPU 3D render callback (building + visual proxies + sky lighting + depth-tested ground slab)
            let c = st.box_center();
            painter.add(render::preview_callback(
                renderer.clone(),
                ModelPreviewFrame {
                    view: v,
                    instances,
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

            // Draw 3D XYZ translation + range + height gizmo on the currently selected point
            if let Some(pi) = st.selected_point {
                if let Some(pt) = st.group.containers.get(st.selected_container).and_then(|c| c.points.get(pi)) {
                    draw_gizmo(&painter, &v, st.proto_to_world(pt.pos), pt.range, pt.height, hover_handle);
                    if let (Some(drag), Some(ptr)) = (st.gizmo_drag, hover_pos) {
                        let live = match drag.handle {
                            GizmoHandle::Range => Some(format!("r = {:.2} m", pt.range)),
                            GizmoHandle::Height => Some(format!("h = {:.2} m", pt.height)),
                            GizmoHandle::Axis(_) => None,
                        };
                        if let Some(text) = live {
                            painter.text(
                                ptr + Vec2::new(14.0, -14.0),
                                egui::Align2::LEFT_BOTTOM,
                                text,
                                egui::FontId::proportional(12.0),
                                Color32::from_rgb(120, 245, 255),
                            );
                        }
                    }
                }
            }

            // Draw `<dispatch><proxy>` 3D markers and gizmo when selected
            if st.show_dispatch {
                for (di, pr) in st.group.dispatch.iter().enumerate() {
                    let selected = st.selected_dispatch == Some(di);
                    let wpos = st.proto_to_world(pr.pos);
                    if let Some(sp) = v.project(wpos) {
                        let d = if selected { 6.5_f32 } else { 4.5_f32 };
                        let col = if selected { Color32::from_rgb(255, 225, 90) } else { Color32::from_rgb(235, 130, 60) };
                        let diamond = vec![
                            Pos2::new(sp.x, sp.y - d),
                            Pos2::new(sp.x + d, sp.y),
                            Pos2::new(sp.x, sp.y + d),
                            Pos2::new(sp.x - d, sp.y),
                        ];
                        painter.add(egui::Shape::convex_polygon(diamond, col, Stroke::new(1.2_f32, Color32::BLACK)));
                        if selected && !pr.kind.is_empty() {
                            painter.text(sp + Vec2::new(10.0, -10.0), egui::Align2::LEFT_BOTTOM, &pr.kind, egui::FontId::proportional(11.0), col);
                        }
                    }
                }
                if let Some(di) = st.selected_dispatch {
                    if let Some(pr) = st.group.dispatch.get(di) {
                        draw_gizmo(&painter, &v, st.proto_to_world(pr.pos), 0.0, 0.0, hover_handle);
                    }
                }
            }

            // Live 3D surface placement preview cursor when `Click 3D to add` is on
            let mut hover_hit: Option<[f32; 3]> = None;
            if st.add_mode && st.gizmo_drag.is_none() && hover_handle.is_none() {
                if let Some(ptr) = hover_pos {
                    let (ro, rd) = v.ray(ptr);
                    hover_hit = pick_scene_surface(ro, rd, st.clip_param(), building_pick, proxy_list, proxy_picks);
                    if let Some(hit) = hover_hit {
                        let col = Color32::from_rgb(100, 245, 130);
                        let mut prev = [hit[0] + 0.3, hit[1], hit[2]];
                        for k in 1..=16 {
                            let a = k as f32 * std::f32::consts::TAU / 16.0;
                            let cur = [hit[0] + 0.3 * a.cos(), hit[1], hit[2] + 0.3 * a.sin()];
                            if let Some((p0, p1)) = v.segment(prev, cur) {
                                painter.line_segment([p0, p1], Stroke::new(1.5_f32, col));
                            }
                            prev = cur;
                        }
                        if let Some(s) = v.project(hit) {
                            painter.circle_filled(s, 4.0, col);
                            painter.circle_stroke(s, 4.0, Stroke::new(1.0_f32, Color32::BLACK));
                        }
                    }
                }
            }

            // LPM click (when not grabbing a gizmo handle): add point on building/proxy surface or select nearest point
            if resp.clicked_by(PointerButton::Primary) && st.gizmo_drag.is_none() && hover_handle.is_none() {
                if let Some(ptr) = resp.interact_pointer_pos() {
                    if st.add_mode {
                        let hit = hover_hit.or_else(|| {
                            let (ro, rd) = v.ray(ptr);
                            pick_scene_surface(ro, rd, st.clip_param(), building_pick, proxy_list, proxy_picks)
                        });
                        if let Some(hit) = hit {
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
                    } else {
                        let mut best_pt: Option<(f32, usize, usize)> = None;
                        for (ci, container) in st.group.containers.iter().enumerate() {
                            for (pi, pt) in container.points.iter().enumerate() {
                                if let Some(s) = v.project(st.proto_to_world(pt.pos)) {
                                    let d = s.distance(ptr);
                                    if d <= 14.0 && best_pt.is_none_or(|b| d < b.0) {
                                        best_pt = Some((d, ci, pi));
                                    }
                                }
                            }
                        }
                        let mut best_disp: Option<(f32, usize)> = None;
                        if st.show_dispatch {
                            for (di, pr) in st.group.dispatch.iter().enumerate() {
                                if let Some(s) = v.project(st.proto_to_world(pr.pos)) {
                                    let d = s.distance(ptr);
                                    if d <= 14.0 && best_disp.is_none_or(|b| d < b.0) {
                                        best_disp = Some((d, di));
                                    }
                                }
                            }
                        }
                        match (best_pt, best_disp) {
                            (Some((dp, ci, pi)), Some((dd, _))) if dp <= dd => {
                                st.selected_container = ci;
                                st.selected_point = Some(pi);
                                st.selected_dispatch = None;
                            }
                            (_, Some((_, di))) => {
                                st.selected_dispatch = Some(di);
                                st.selected_point = None;
                            }
                            (Some((_, ci, pi)), None) => {
                                st.selected_container = ci;
                                st.selected_point = Some(pi);
                                st.selected_dispatch = None;
                            }
                            (None, None) => {}
                        }
                    }
                }
            }

    // Compact 2-line footer under the 3D viewport
    egui::Frame::new().inner_margin(egui::Margin { left: 6, right: 0, top: 4, bottom: 0 }).show(ui, |ui| {
        let hint = if st.add_mode {
            t("RMB + WASD/QE: fly · Click a surface to add a loot point")
        } else {
            t("RMB + WASD/QE: fly · Wheel: zoom/speed · MMB: pan · Click: select")
        };
        ui.add(egui::Label::new(egui::RichText::new(hint).small().color(theme::TEXT_DIM)).truncate());
        ui.add(egui::Label::new(egui::RichText::new(&st.model).small().color(theme::TEXT_DIM)).truncate());
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_editor_panel(
    st: &mut LootProtoUi,
    ui: &mut egui::Ui,
    total_h: f32,
    limits: &proto::LimitsLists,
    dispatch_resolved: &[Option<String>],
    save_requested: &mut bool,
    remove_requested: &mut bool,
) {
    let footer_h = 44.0;
    let body_h = (total_h - footer_h).max(120.0);
    let col_rect = ui.max_rect();
    let body_rect = egui::Rect::from_min_size(col_rect.min, Vec2::new(col_rect.width(), body_h));
    let footer_rect = egui::Rect::from_min_max(Pos2::new(col_rect.left(), body_rect.bottom()), col_rect.max);

    // Scrollable inspector body styled with theme.rs Details widgets
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    body_ui.set_clip_rect(body_rect.intersect(ui.clip_rect()));
    egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin::ZERO).show(&mut body_ui, |ui| {
        ui.set_min_height(body_h);
        ui.set_max_height(body_h);
        egui::ScrollArea::vertical()
            .id_salt("loot_proto_editor_scroll")
            .max_height(body_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                    // Header card matching the editor's Details panel
                    egui::Frame::new().inner_margin(egui::Margin { left: 8, right: 8, top: 8, bottom: 4 }).show(ui, |ui| {
                        let subtitle = if st.exists_in_proto {
                            st.model.clone()
                        } else {
                            t("Not yet in mapgroupproto.xml — edit and save to add it.")
                        };
                        details_head(ui, icons::HOUSE, theme::KIND_MODEL, &st.group.name, &subtitle);
                    });

                    // ---- Prototype / Group section ----
                    section(ui, &icons::with(icons::CUBE, &t("Prototype class")), true, |ui| {
                        row(ui, &t("Prototype class"), |ui| {
                            if ui.add_sized([ui.available_width(), 22.0], egui::TextEdit::singleline(&mut st.group.name)).changed() {
                                st.dirty = true;
                            }
                        });
                        row(ui, &t("Max loot"), |ui| {
                            ui.horizontal(|ui| {
                                let mut has_max = st.group.lootmax.is_some();
                                if ui.checkbox(&mut has_max, "").changed() {
                                    st.group.lootmax = has_max.then_some(2);
                                    st.dirty = true;
                                }
                                if let Some(lm) = &mut st.group.lootmax {
                                    if ui.add(ValueField::new(lm, 0..=100)).changed() {
                                        st.dirty = true;
                                    }
                                }
                            });
                        });
                        if check_row(ui, &mut st.group.deloot, t("Dynamic event loot (deloot)")).changed() {
                            st.dirty = true;
                        }
                        ui.add_space(2.0);
                        subsection(ui, &t("Usage flags"), true, |ui| {
                            if flag_chips(ui, &limits.usages, &mut st.group.usages) {
                                st.dirty = true;
                            }
                        });
                        subsection(ui, &t("Value flags"), !st.group.values.is_empty(), |ui| {
                            if flag_chips(ui, &limits.values, &mut st.group.values) {
                                st.dirty = true;
                            }
                        });
                    });

                    // ---- Containers section ----
                    let mut delete_container = false;
                    let can_delete_container = st.group.containers.len() > 1;
                    section(ui, &icons::with(icons::PACKAGE, &t("Containers")), true, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for i in 0..st.group.containers.len() {
                                let col = CONTAINER_COLORS[i % CONTAINER_COLORS.len()];
                                let name = st.group.containers[i].name.clone();
                                let count = st.group.containers[i].points.len();
                                let active = st.selected_container == i;
                                let text = egui::RichText::new(format!("● {name} ({count})"))
                                    .color(if active { Color32::WHITE } else { col });
                                let btn = egui::Button::new(text)
                                    .fill(if active { theme::TOGGLE_ON } else { theme::SECTION })
                                    .stroke(Stroke::new(1.0_f32, if active { col } else { theme::BORDER }));
                                if ui.add(btn).clicked() {
                                    st.selected_container = i;
                                    st.selected_point = (!st.group.containers[i].points.is_empty()).then_some(0);
                                }
                            }
                            if ui.button(icons::with(icons::PLUS, &t("Add container"))).clicked() {
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

                        if let Some(c) = st.group.containers.get_mut(st.selected_container) {
                            ui.add_space(4.0);
                            row(ui, &t("Container name"), |ui| {
                                ui.horizontal(|ui| {
                                    let w = if can_delete_container { (ui.available_width() - 30.0).max(80.0) } else { ui.available_width() };
                                    if ui.add_sized([w, 22.0], egui::TextEdit::singleline(&mut c.name)).changed() {
                                        st.dirty = true;
                                    }
                                    if can_delete_container && ui.button(icons::TRASH).on_hover_text(t("Delete")).clicked() {
                                        delete_container = true;
                                    }
                                });
                            });
                            row(ui, &t("Max loot"), |ui| {
                                ui.horizontal(|ui| {
                                    let mut has_max = c.lootmax.is_some();
                                    if ui.checkbox(&mut has_max, "").changed() {
                                        c.lootmax = has_max.then_some(2);
                                        st.dirty = true;
                                    }
                                    if let Some(lm) = &mut c.lootmax {
                                        if ui.add(ValueField::new(lm, 0..=100)).changed() {
                                            st.dirty = true;
                                        }
                                    }
                                });
                            });
                            ui.add_space(2.0);
                            subsection(ui, &t("Categories"), true, |ui| {
                                if flag_chips(ui, &limits.categories, &mut c.categories) {
                                    st.dirty = true;
                                }
                            });
                            subsection(ui, &t("Tags"), true, |ui| {
                                if flag_chips(ui, &limits.tags, &mut c.tags) {
                                    st.dirty = true;
                                }
                            });
                        }
                    });

                    if delete_container && st.selected_container < st.group.containers.len() {
                        st.group.containers.remove(st.selected_container);
                        st.selected_container = st.selected_container.min(st.group.containers.len().saturating_sub(1));
                        st.selected_point = st.group.containers.get(st.selected_container).and_then(|c| (!c.points.is_empty()).then_some(0));
                        st.dirty = true;
                    }

                    // ---- Loot points list section ----
                    let container_col = CONTAINER_COLORS[st.selected_container % CONTAINER_COLORS.len()];
                    if let Some(c) = st.group.containers.get_mut(st.selected_container) {
                        let pts_title = icons::with(icons::MAP_PIN, &tf("Loot points ({0})", &[&c.points.len().to_string()]));
                        section(ui, &pts_title, true, |ui| {
                            let has_sel = st.selected_point.is_some_and(|pi| pi < c.points.len());
                            theme::button_columns(ui, 3, |cols| {
                                if cols[0].add_sized([cols[0].available_width(), 24.0], egui::Button::new(icons::with(icons::PLUS, &t("Add point")))).clicked() {
                                    let pt = st.selected_point.and_then(|i| c.points.get(i)).cloned().unwrap_or_default();
                                    c.points.push(pt);
                                    st.selected_point = Some(c.points.len() - 1);
                                    st.dirty = true;
                                }
                                if cols[1].add_enabled_ui(has_sel, |ui| ui.add_sized([ui.available_width(), 24.0], egui::Button::new(icons::with(icons::COPY, &t("Duplicate"))))).inner.clicked() {
                                    if let Some(pi) = st.selected_point {
                                        let mut dup = c.points[pi].clone();
                                        dup.pos[0] += 0.3;
                                        c.points.push(dup);
                                        st.selected_point = Some(c.points.len() - 1);
                                        st.dirty = true;
                                    }
                                }
                                if cols[2].add_enabled_ui(has_sel, |ui| ui.add_sized([ui.available_width(), 24.0], egui::Button::new(icons::with(icons::TRASH, &t("Delete"))))).inner.clicked() {
                                    if let Some(pi) = st.selected_point {
                                        c.points.remove(pi);
                                        st.selected_point = if c.points.is_empty() { None } else { Some(pi.min(c.points.len() - 1)) };
                                        st.dirty = true;
                                    }
                                }
                            });

                            ui.add_space(4.0);
                            egui::Frame::new()
                                .fill(theme::DARK)
                                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                                .corner_radius(3)
                                .inner_margin(egui::Margin::same(2))
                                .show(ui, |ui| {
                                    egui::ScrollArea::vertical().id_salt("loot_points_list").max_height(150.0).show(ui, |ui| {
                                        for (i, pt) in c.points.iter().enumerate() {
                                            let sel = st.selected_point == Some(i);
                                            let flag_tag = if pt.flags & 32 != 0 { " [out]" } else { "" };
                                            let left = format!("#{}  ({:.2}, {:.2}, {:.2}){flag_tag}", i + 1, pt.pos[0], pt.pos[1], pt.pos[2]);
                                            let right = format!("r {:.2} · h {:.2}", pt.range, pt.height);
                                            if list_row(ui, sel, icons::MAP_PIN, container_col, left, &right).clicked() {
                                                st.selected_point = Some(i);
                                                st.selected_dispatch = None;
                                            }
                                        }
                                    });
                                });
                        });

                        // ---- Selected point inspector section ----
                        if let Some(pi) = st.selected_point {
                            if let Some(pt) = c.points.get_mut(pi) {
                                let pt_title = icons::with(icons::CROSSHAIR, &tf("Point #{0}", &[&(pi + 1).to_string()]));
                                section(ui, &pt_title, true, |ui| {
                                    let before = pt.pos;
                                    let (mut x, mut y, mut z) = (pt.pos[0], pt.pos[1], pt.pos[2]);
                                    vector_row(ui, &t("Position (X, Y, Z)"), &mut [
                                        (&mut x, 0.02, theme::AXIS[0], "X".into()),
                                        (&mut y, 0.02, theme::AXIS[1], "Y".into()),
                                        (&mut z, 0.02, theme::AXIS[2], "Z".into()),
                                    ]);
                                    pt.pos = [x, y, z];
                                    if pt.pos != before {
                                        st.dirty = true;
                                    }

                                    row(ui, &t("Range"), |ui| {
                                        if ui.add(ValueField::new(&mut pt.range, 0.05..=5.0).suffix(" m").step_by(0.01)).changed() {
                                            st.dirty = true;
                                        }
                                    });
                                    row(ui, &t("Height"), |ui| {
                                        if ui.add(ValueField::new(&mut pt.height, 0.1..=5.0).suffix(" m").step_by(0.01)).changed() {
                                            st.dirty = true;
                                        }
                                    });
                                    let mut outdoor = pt.flags & 32 != 0;
                                    if check_row(ui, &mut outdoor, t("Outside / ground (flags=32)")).changed() {
                                        if outdoor {
                                            pt.flags |= 32;
                                        } else {
                                            pt.flags &= !32;
                                        }
                                        st.dirty = true;
                                    }
                                });
                            }
                        }
                    }

                    // ---- Dispatch proxies section (<dispatch><proxy ... /></dispatch>) ----
                    let disp_title = format!("{} ({})", icons::with(icons::WRENCH, &t("Dispatch")), st.group.dispatch.len());
                    section(ui, &disp_title, !st.group.dispatch.is_empty(), |ui| {
                        row(ui, &t("DE chance"), |ui| {
                            ui.horizontal(|ui| {
                                let mut has_dc = st.group.dispatch_dechance.is_some();
                                if ui.checkbox(&mut has_dc, "").changed() {
                                    st.group.dispatch_dechance = has_dc.then_some(0.0);
                                    st.dirty = true;
                                }
                                if let Some(dc) = &mut st.group.dispatch_dechance {
                                    if ui.add(ValueField::new(dc, 0.0..=1.0).step_by(0.05)).changed() {
                                        st.dirty = true;
                                    }
                                }
                            });
                        });
                        ui.add_space(4.0);
                        let mut add_disp = false;
                        let mut dup_disp = false;
                        let mut del_disp = false;
                        let has_disp_sel = st.selected_dispatch.is_some_and(|i| i < st.group.dispatch.len());
                        theme::button_columns(ui, 3, |cols| {
                            if cols[0].add_sized([cols[0].available_width(), 22.0], egui::Button::new(icons::with(icons::PLUS, &t("Add proxy")))).clicked() {
                                add_disp = true;
                            }
                            if cols[1].add_enabled(has_disp_sel, egui::Button::new(icons::with(icons::COPY, &t("Duplicate"))).min_size(Vec2::new(cols[1].available_width(), 22.0))).clicked() {
                                dup_disp = true;
                            }
                            if cols[2].add_enabled(has_disp_sel, egui::Button::new(icons::with(icons::TRASH, &t("Delete"))).min_size(Vec2::new(cols[2].available_width(), 22.0))).clicked() {
                                del_disp = true;
                            }
                        });
                        if add_disp {
                            st.group.dispatch.push(DispatchProxy::default());
                            st.selected_dispatch = Some(st.group.dispatch.len() - 1);
                            st.selected_point = None;
                            st.dirty = true;
                        }
                        if dup_disp {
                            if let Some(i) = st.selected_dispatch {
                                if let Some(mut copy) = st.group.dispatch.get(i).cloned() {
                                    copy.pos[0] += 0.25;
                                    st.group.dispatch.push(copy);
                                    st.selected_dispatch = Some(st.group.dispatch.len() - 1);
                                    st.selected_point = None;
                                    st.dirty = true;
                                }
                            }
                        }
                        if del_disp {
                            if let Some(i) = st.selected_dispatch {
                                if i < st.group.dispatch.len() {
                                    st.group.dispatch.remove(i);
                                    st.selected_dispatch = if st.group.dispatch.is_empty() { None } else { Some(i.min(st.group.dispatch.len() - 1)) };
                                    st.dirty = true;
                                }
                            }
                        }
                        if !st.group.dispatch.is_empty() {
                            ui.add_space(4.0);
                            egui::Frame::new()
                                .fill(theme::PANEL)
                                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                                .corner_radius(3.0)
                                .inner_margin(egui::Margin::same(2))
                                .show(ui, |ui| {
                                    egui::ScrollArea::vertical().id_salt("loot_proto_dispatch_list").max_height(130.0).show(ui, |ui| {
                                        for (i, pr) in st.group.dispatch.iter().enumerate() {
                                            let sel = st.selected_dispatch == Some(i);
                                            let label = if pr.kind.trim().is_empty() {
                                                format!("#{} ({})", i + 1, t("None"))
                                            } else {
                                                format!("#{} {}", i + 1, pr.kind)
                                            };
                                            let right = format!("{:.2}, {:.2}, {:.2}", pr.pos[0], pr.pos[1], pr.pos[2]);
                                            if list_row(ui, sel, icons::WRENCH, Color32::from_rgb(235, 130, 60), label, &right).clicked() {
                                                st.selected_dispatch = Some(i);
                                                st.selected_point = None;
                                            }
                                        }
                                    });
                                });
                        }
                    });

                    // ---- Selected dispatch proxy inspector ----
                    if let Some(di) = st.selected_dispatch {
                        if let Some(pr) = st.group.dispatch.get_mut(di) {
                            let title = icons::with(icons::WRENCH, &tf("Dispatch proxy #{0}", &[&(di + 1).to_string()]));
                            section(ui, &title, true, |ui| {
                                row(ui, &t("Class (type)"), |ui| {
                                    if ui.add_sized([ui.available_width(), 22.0], egui::TextEdit::singleline(&mut pr.kind)).changed() {
                                        st.dirty = true;
                                    }
                                });
                                match dispatch_resolved.get(di).and_then(|o| o.as_deref()) {
                                    Some(p) => {
                                        let file = p.rsplit('\\').next().unwrap_or(p);
                                        ui.colored_label(Color32::from_rgb(120, 215, 145), format!("{}  {file}", icons::CHECK));
                                    }
                                    None if !pr.kind.trim().is_empty() => {
                                        ui.colored_label(theme::WARN, t("Model not found in CfgVehicles"));
                                    }
                                    None => {}
                                }
                                let before_pos = pr.pos;
                                let (mut x, mut y, mut z) = (pr.pos[0], pr.pos[1], pr.pos[2]);
                                vector_row(ui, &t("Position (X, Y, Z)"), &mut [
                                    (&mut x, 0.02, theme::AXIS[0], "X".into()),
                                    (&mut y, 0.02, theme::AXIS[1], "Y".into()),
                                    (&mut z, 0.02, theme::AXIS[2], "Z".into()),
                                ]);
                                pr.pos = [x, y, z];
                                if pr.pos != before_pos {
                                    st.dirty = true;
                                }
                                let before_rpy = pr.rpy;
                                let (mut r, mut p, mut yw) = (pr.rpy[0], pr.rpy[1], pr.rpy[2]);
                                vector_row(ui, &t("Rotation (R, P, Y)"), &mut [
                                    (&mut r, 0.5, theme::AXIS[0], "R".into()),
                                    (&mut p, 0.5, theme::AXIS[1], "P".into()),
                                    (&mut yw, 0.5, theme::AXIS[2], "Y".into()),
                                ]);
                                pr.rpy = [r, p, yw];
                                if pr.rpy != before_rpy {
                                    st.dirty = true;
                                }
                                row(ui, &t("DE chance"), |ui| {
                                    ui.horizontal(|ui| {
                                        let mut has_dc = pr.dechance.is_some();
                                        if ui.checkbox(&mut has_dc, "").changed() {
                                            pr.dechance = has_dc.then_some(0.0);
                                            st.dirty = true;
                                        }
                                        if let Some(dc) = &mut pr.dechance {
                                            if ui.add(ValueField::new(dc, 0.0..=1.0).step_by(0.05)).changed() {
                                                st.dirty = true;
                                            }
                                        }
                                    });
                                });
                            });
                        }
                    }
                });
    });

    // Fixed bottom save/close bar pinned to the bottom of the left column
    let mut footer_ui = ui.new_child(egui::UiBuilder::new().max_rect(footer_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    footer_ui.set_clip_rect(footer_rect.intersect(ui.clip_rect()));
    egui::Frame::new().fill(theme::PANEL).stroke(Stroke::new(1.0_f32, theme::BORDER)).inner_margin(egui::Margin::symmetric(8, 6)).show(&mut footer_ui, |ui| {
        ui.set_min_width(footer_rect.width() - 16.0);
        ui.horizontal(|ui| {
            let save_btn = egui::Button::new(egui::RichText::new(icons::with(icons::FLOPPY_DISK, &t("Save to mapgroupproto.xml"))).color(Color32::WHITE))
                .fill(theme::ACCENT);
            if ui.add(save_btn).clicked() {
                *save_requested = true;
            }
            if st.exists_in_proto {
                if ui.button(icons::TRASH).on_hover_text(t("Remove from mapgroupproto.xml")).clicked() {
                    *remove_requested = true;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t("Close")).clicked() {
                    st.open = false;
                }
            });
        });
    });
}

fn flag_chips(ui: &mut egui::Ui, choices: &[String], selected: &mut Vec<String>) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        let mut all: Vec<String> = choices.to_vec();
        for s in selected.iter() {
            if !all.iter().any(|c| c.eq_ignore_ascii_case(s)) {
                all.push(s.clone());
            }
        }
        for name in all {
            let pos = selected.iter().position(|s| s.eq_ignore_ascii_case(&name));
            let active = pos.is_some();
            if tool_button(ui, active, &name).clicked() {
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
