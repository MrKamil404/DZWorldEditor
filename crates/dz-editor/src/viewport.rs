//! Viewports: camera control, tools, draw lists and on-screen overlays.
use crate::app::SelectedWhole;
use crate::app::ShapeTool;
use crate::app::{App, Finish, Gizmo, LineAction, Mode, ViewMode};
use crate::road_draw::RoadDraw;
use crate::camera::{add, dot, mul, norm, sub, View, V3};
use crate::i18n::{t, tf};
use crate::keys::{Action, Hold};
use crate::models::{LOD_BILLBOARD, LOD_FAR, LOD_NEAR};
use crate::render::{self, BrushView, Frame, ObjectDraw, Shading};
use crate::theme;
use dz_core::edit::{Brush, Change, MaskStroke, SculptStroke, SculptTool};
use dz_core::gen;
use dz_core::model_info::ModelInfo;
use dz_core::objects::{grid_key, rotation, Obj, ObjectStore, GRID_CELL};
use dz_core::terrain::Heightmap;
use eframe::egui::{self, pos2, vec2, Color32, Key, PointerButton, Pos2, Rect, Sense, Stroke};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct ViewCache {
    draw: ObjectDraw,
    eye: V3,
    forward: V3,
    rev: u64,
    selection_rev: u64,
    loaded: usize,
    mpp: f32,
    infos: Vec<ModelInfo>,
    tints: Vec<[f32; 4]>,
    frames_since: u32,
    toggles: Option<(bool, bool, Shading, i32, bool, u32, i32, usize, u64, bool, u64)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Handle {
    Plane,
    X,
    Y,
    Z,
    /// Turning about the X, Y or Z axis of the gizmo.
    RotX,
    RotY,
    RotZ,
    Scale,
    /// Turning with the mouse held on the object itself.
    Spin,
}

pub struct ObjectDrag {
    /// Object a press would have unselected, had the mouse not moved.
    toggle: Option<u32>,
    /// The mouse moved far enough to count as a drag.
    started: bool,
    handle: Handle,
    before: Vec<(u32, Obj)>,
    pivot: V3,
    start: Pos2,
    /// Screen direction and metres per point for axis handles.
    axis: (egui::Vec2, V3, f32),
    grab_offset: [f32; 2],
    /// The mouse as the drag sees it, and where the real one was last: held for precise
    /// work, the first follows the second slowly.
    virt: Option<(Pos2, Pos2)>,
    /// A release before the mouse moved is a click here, adding to the selection when true.
    click: Option<(Pos2, bool)>,
    /// Turns by whole steps of the angle step, which the wheel changes meanwhile.
    stepped: bool,
    /// The modifier keys that chose what the drag does.
    mods: egui::Modifiers,
}

/// Scripted input for automatic checks (`DZWE_AUTO`).
pub struct AutoScript {
    pub steps: Vec<String>,
    pub wait: u32,
}

/// The point of the terrain a ray hits.
pub fn terrain_hit(h: &Heightmap, o: V3, d: V3, ortho: bool) -> Option<V3> {
    pick_terrain(h, o, d, ortho)
}

fn pick_terrain(h: &Heightmap, o: V3, d: V3, ortho: bool) -> Option<V3> {
    let (w, depth) = (h.width_m(), h.height_m());
    let inside = |x: f32, z: f32| x >= 0.0 && z >= 0.0 && x <= w && z <= depth;
    if ortho {
        return inside(o[0], o[2]).then(|| [o[0], h.sample(o[0], o[2]), o[2]]);
    }
    // From below the terrain there is no ground to point at.
    if inside(o[0], o[2]) && o[1] <= h.sample(o[0], o[2]) {
        return None;
    }
    let mut t = 0.0f32;
    let mut prev = 0.0f32;
    for _ in 0..4000 {
        let p = add(o, mul(d, t));
        if t > 60000.0 {
            return None;
        }
        let ground = h.sample(p[0], p[2]);
        if inside(p[0], p[2]) && p[1] <= ground {
            let (mut a, mut b) = (prev, t);
            for _ in 0..24 {
                let m = (a + b) * 0.5;
                let q = add(o, mul(d, m));
                if q[1] <= h.sample(q[0], q[2]) {
                    b = m;
                } else {
                    a = m;
                }
            }
            let q = add(o, mul(d, b));
            return Some([q[0], h.sample(q[0], q[2]), q[2]]);
        }
        prev = t;
        t += (p[1] - ground).abs().max(h.cell) * 0.5;
    }
    None
}

/// Ray parameter where the ray enters the model's rotated box.
fn ray_box(o: &V3, d: &V3, ob: &Obj, ground: f32, info: &ModelInfo) -> Option<f32> {
    let m = rotation(ob.yaw, ob.pitch, ob.roll, ob.scale.max(0.01));
    let s2 = ob.scale.max(0.01).powi(2);
    let origin = [ob.x, ground, ob.z];
    let rel = sub(*o, origin);
    let lo = [dot(rel, m[0]) / s2, dot(rel, m[1]) / s2, dot(rel, m[2]) / s2];
    let ld = [dot(*d, m[0]) / s2, dot(*d, m[1]) / s2, dot(*d, m[2]) / s2];
    let (mn, mx) = if info.known { (info.min, info.max) } else { ([-1.0, 0.0, -1.0], [1.0, 2.0, 1.0]) };
    let (mut t0, mut t1) = (f32::MIN, f32::MAX);
    for k in 0..3 {
        if ld[k].abs() < 1e-7 {
            if lo[k] < mn[k] || lo[k] > mx[k] {
                return None;
            }
        } else {
            let (a, b) = ((mn[k] - lo[k]) / ld[k], (mx[k] - lo[k]) / ld[k]);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
    }
    (t0 <= t1 && t1 > 0.0).then_some(t0.max(0.0))
}

/// Positions and triangles of a model, for picking.
pub struct PickMesh {
    positions: Vec<[f32; 3]>,
    triangles: Vec<u32>,
    /// Where the model stands, for keeping grass out from under it.
    pub footprint: Footprint,
}

/// What of a model is over each cell of the ground, seen from above, in the model's own
/// coordinates: for each cell the lowest height its geometry reaches there.
pub struct Footprint {
    x0: f32,
    z0: f32,
    cell: f32,
    w: usize,
    h: usize,
    low: Vec<f32>,
}

impl Footprint {
    pub fn of(positions: &[[f32; 3]], triangles: &[u32]) -> Footprint {
        let empty = Footprint { x0: 0.0, z0: 0.0, cell: 1.0, w: 0, h: 0, low: Vec::new() };
        if triangles.len() < 3 || positions.is_empty() {
            return empty;
        }
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in positions {
            lo = [lo[0].min(p[0]), lo[1].min(p[2])];
            hi = [hi[0].max(p[0]), hi[1].max(p[2])];
        }
        // Half a metre, coarser for very large models; a cell of margin all round.
        let cell = ((hi[0] - lo[0]).max(hi[1] - lo[1]) / 400.0).max(0.5);
        let (x0, z0) = (lo[0] - cell, lo[1] - cell);
        let (w, h) = (((hi[0] - x0) / cell) as usize + 2, ((hi[1] - z0) / cell) as usize + 2);
        let mut low = vec![f32::INFINITY; w * h];
        let mark = |x: f32, z: f32, y: f32, low: &mut Vec<f32>| {
            let (i, j) = (((x - x0) / cell) as usize, ((z - z0) / cell) as usize);
            if i < w && j < h {
                let v = &mut low[j * w + i];
                *v = v.min(y);
            }
        };
        for t in triangles.chunks_exact(3) {
            let [a, b, c] = [positions[t[0] as usize], positions[t[1] as usize], positions[t[2] as usize]];
            let y = a[1].min(b[1]).min(c[1]);
            // The edges, so that walls seen edge-on still count.
            for (p, q) in [(a, b), (b, c), (c, a)] {
                let n = (((q[0] - p[0]).hypot(q[2] - p[2]) / (cell * 0.5)) as usize).min(4000) + 1;
                for k in 0..=n {
                    let f = k as f32 / n as f32;
                    mark(p[0] + (q[0] - p[0]) * f, p[2] + (q[2] - p[2]) * f, y, &mut low);
                }
            }
            // And the cells whose middle the triangle covers.
            let area = (b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2]);
            if area.abs() < cell * cell * 0.25 {
                continue;
            }
            let (i0, i1) = (((a[0].min(b[0]).min(c[0]) - x0) / cell) as usize, ((a[0].max(b[0]).max(c[0]) - x0) / cell) as usize);
            let (j0, j1) = (((a[2].min(b[2]).min(c[2]) - z0) / cell) as usize, ((a[2].max(b[2]).max(c[2]) - z0) / cell) as usize);
            for j in j0..=j1.min(h - 1) {
                for i in i0..=i1.min(w - 1) {
                    let (px, pz) = (x0 + (i as f32 + 0.5) * cell, z0 + (j as f32 + 0.5) * cell);
                    let side = |p: [f32; 3], q: [f32; 3]| (q[0] - p[0]) * (pz - p[2]) - (px - p[0]) * (q[2] - p[2]);
                    let (s0, s1, s2) = (side(a, b), side(b, c), side(c, a));
                    if (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0) {
                        let v = &mut low[j * w + i];
                        *v = v.min(y);
                    }
                }
            }
        }
        // Grass is placed a metre apart, so it keeps a little away from the edge.
        let mut grown = low.clone();
        for j in 0..h {
            for i in 0..w {
                let mut m = low[j * w + i];
                for (dj, di) in [(0, 1), (2, 1), (1, 0), (1, 2)] {
                    let (jj, ii) = ((j + dj).wrapping_sub(1), (i + di).wrapping_sub(1));
                    if jj < h && ii < w {
                        m = m.min(low[jj * w + ii]);
                    }
                }
                grown[j * w + i] = m;
            }
        }
        Footprint { x0, z0, cell, w, h, low: grown }
    }

    /// The lowest height of the model over a point, or infinity where nothing is.
    pub fn low_at(&self, x: f32, z: f32) -> f32 {
        let (fx, fz) = ((x - self.x0) / self.cell, (z - self.z0) / self.cell);
        if fx < 0.0 || fz < 0.0 || fx as usize >= self.w || fz as usize >= self.h {
            return f32::INFINITY;
        }
        self.low[fz as usize * self.w + fx as usize]
    }
}

/// Keeps the finest geometry seen of each model.
pub fn remember_pick_mesh(meshes: &mut HashMap<u32, std::sync::Arc<PickMesh>>, m: &crate::render::MeshData) {
    if m.key.0 >= MISSION_BASE || (m.key.1 != LOD_NEAR && meshes.contains_key(&m.key.0)) {
        return;
    }
    let positions: Vec<[f32; 3]> = m.vertices.chunks_exact(10).map(|v| [v[0], v[1], v[2]]).collect();
    let footprint = Footprint::of(&positions, &m.indices);
    meshes.insert(m.key.0, std::sync::Arc::new(PickMesh { positions, triangles: m.indices.clone(), footprint }));
}

/// Ray parameter of the nearest triangle hit, in the units of `d`.
fn ray_mesh(mesh: &PickMesh, o: V3, d: V3) -> Option<f32> {
    let mut best: Option<f32> = None;
    for t in mesh.triangles.chunks_exact(3) {
        let (a, b, c) = (mesh.positions[t[0] as usize], mesh.positions[t[1] as usize], mesh.positions[t[2] as usize]);
        let (e1, e2) = (sub(b, a), sub(c, a));
        let p = cross(d, e2);
        let det = dot(e1, p);
        if det.abs() < 1e-9 {
            continue;
        }
        let f = 1.0 / det;
        let s = sub(o, a);
        let u = f * dot(s, p);
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = cross(s, e1);
        let w = f * dot(d, q);
        if w < 0.0 || u + w > 1.0 {
            continue;
        }
        let hit = f * dot(e2, q);
        if hit > 0.0 && best.is_none_or(|b| hit < b) {
            best = Some(hit);
        }
    }
    best
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The ray in a model's own coordinates; distances along it stay those of the world ray.
fn ray_in_model(o: &V3, d: &V3, ob: &Obj, ground: f32) -> (V3, V3) {
    let m = rotation(ob.yaw, ob.pitch, ob.roll, ob.scale.max(0.01));
    let s2 = ob.scale.max(0.01).powi(2);
    let rel = sub(*o, [ob.x, ground, ob.z]);
    ([dot(rel, m[0]) / s2, dot(rel, m[1]) / s2, dot(rel, m[2]) / s2], [dot(*d, m[0]) / s2, dot(*d, m[1]) / s2, dot(*d, m[2]) / s2])
}

fn pick_object(app: &App, v: &View, s: Pos2) -> Option<u32> {
    let doc = app.doc.as_ref()?;
    let (o, d) = v.ray(s);
    // A hole lets the ray through to what is underneath.
    let ground_hit = pick_terrain(&doc.world.height, o, d, v.ortho).filter(|p| !doc.world.is_hole(p[0], p[2]));
    let limit = ground_hit.map(|p| dot(sub(p, o), d) + 1.0).unwrap_or(4000.0).min(6000.0);
    let mut seen = std::collections::HashSet::new();
    // Whatever the ray's box test lets through, nearest first.
    let mut candidates: Vec<(f32, u32, bool)> = Vec::new();
    let mut t = 0.0;
    let step = GRID_CELL * 0.5;
    let world = &doc.world;
    while t < limit {
        let p = add(o, mul(d, t));
        if v.ortho {
            t = limit;
        }
        let k = grid_key(p[0], p[2]);
        for gz in k.1 - 1..=k.1 + 1 {
            for gx in k.0 - 1..=k.0 + 1 {
                if !seen.insert((gx, gz)) {
                    continue;
                }
                for &id in world.objects.cell((gx, gz)) {
                    let Some(ob) = world.objects.get(id) else { continue };
                    if !world.object_interactive(ob) {
                        continue;
                    }
                    let path = world.objects.model_path(ob);
                    let info = app.db.get(path);
                    if path.contains(r"\roads\parts\") || crate::models::is_decal_model(path) {
                        // Road parts bend over the terrain, so they are found where the ray
                        // meets the ground, not at the height of their origin.
                        let Some(g) = ground_hit else { continue };
                        let (lo, _) = ray_in_model(&g, &d, ob, g[1]);
                        if lo[0] >= info.min[0] - 0.3 && lo[0] <= info.max[0] + 0.3 && lo[2] >= info.min[2] - 0.3 && lo[2] <= info.max[2] + 0.3 {
                            candidates.push((dot(sub(g, o), d) - 0.5, id, true));
                        }
                        continue;
                    }
                    let ground = ObjectStore::origin_y(ob, &world.height);
                    if let Some(h) = ray_box(&o, &d, ob, ground, &info) {
                        candidates.push((h, id, false));
                    }
                }
            }
        }
        t += step;
    }
    candidates.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let mut best: Option<(f32, u32)> = None;
    for (h, id, flat) in candidates.into_iter().take(400) {
        if best.is_some_and(|b| h >= b.0) {
            break;
        }
        let Some(ob) = world.objects.get(id) else { continue };
        let hit = match app.pick_meshes.get(&ob.model) {
            Some(mesh) if flat => {
                let g = ground_hit?;
                let (lo, _) = ray_in_model(&g, &d, ob, g[1]);
                // Straight down through the part at the point under the cursor.
                ray_mesh(mesh, [lo[0], 1000.0, lo[2]], [0.0, -1.0, 0.0]).map(|_| h)
            }
            Some(mesh) => {
                let (lo, ld) = ray_in_model(&o, &d, ob, ObjectStore::origin_y(ob, &world.height));
                ray_mesh(mesh, lo, ld)
            }
            // Not loaded yet: its box is the best there is.
            None => Some(h),
        };
        if let Some(t) = hit {
            if best.is_none_or(|b| t < b.0) {
                best = Some((t, id));
            }
        }
    }
    best.map(|b| b.1)
}

pub(crate) fn is_plant(path: &str) -> bool {
    path.contains("\\tree\\") || path.contains("\\t_") || path.contains("plants") || path.contains("\\bush") || path.contains("\\b_")
}

/// Colour of an object in the top view, by what kind of thing it is.
fn category_tint(path: &str) -> [f32; 4] {
    let c: [f32; 3] = if path.contains("\\tree\\") || path.contains("\\t_") {
        [0.12, 0.42, 0.14]
    } else if path.contains("plants") || path.contains("\\bush") || path.contains("\\b_") {
        [0.32, 0.6, 0.24]
    } else if path.contains("\\roads\\") || path.contains("\\rail\\") {
        [0.62, 0.62, 0.55]
    } else if path.contains("water") {
        [0.2, 0.45, 0.8]
    } else if path.contains("rocks") || path.contains("stone") {
        [0.55, 0.52, 0.48]
    } else if path.contains("structures") {
        [0.85, 0.42, 0.16]
    } else {
        [0.75, 0.3, 0.7]
    };
    [c[0], c[1], c[2], 0.5]
}

/// The orientation an object is drawn with, sheared over the terrain as the game does.
#[inline]
fn placed_matrix(o: &Obj, info: &ModelInfo, height: &dz_core::terrain::Heightmap, y: f32) -> ([[f32; 3]; 3], f32) {
    match dz_core::model_info::skew(o, info, height) {
        Some(s) => {
            let mut m = rotation(o.yaw, 0.0, 0.0, o.scale);
            m[0][1] = s.along_x;
            m[2][1] = s.along_z;
            (m, y + s.lift)
        }
        None => (rotation(o.yaw, o.pitch, o.roll, o.scale), y + dz_core::model_info::tilted_lift(o, info, height)),
    }
}

#[inline]
fn write_instance(out: &mut Vec<f32>, m: &[[f32; 3]; 3], p: V3, tint: [f32; 4]) {
    out.extend_from_slice(&[m[0][0], m[1][0], m[2][0], p[0], m[0][1], m[1][1], m[2][1], p[1], m[0][2], m[1][2], m[2][2], p[2], tint[0], tint[1], tint[2], tint[3]]);
}

fn write_box(out: &mut Vec<f32>, o: &Obj, y: f32, info: &ModelInfo, tint: [f32; 4], min_size: f32) {
    let (mn, mx) = if info.known { (info.min, info.max) } else { ([-0.8, 0.0, -0.8], [0.8, 2.0, 0.8]) };
    let m = rotation(o.yaw, o.pitch, o.roll, o.scale);
    let (sx, sy, sz) = ((mx[0] - mn[0]).max(min_size), (mx[1] - mn[1]).max(0.1), (mx[2] - mn[2]).max(min_size));
    let (cx, cz) = ((mn[0] + mx[0]) * 0.5, (mn[2] + mx[2]) * 0.5);
    let scaled = [mul(m[0], sx), mul(m[1], sy), mul(m[2], sz)];
    let p = [o.x + m[0][0] * cx + m[1][0] * mn[1] + m[2][0] * cz, y + m[0][1] * cx + m[1][1] * mn[1] + m[2][1] * cz, o.z + m[0][2] * cx + m[1][2] * mn[1] + m[2][2] * cz];
    write_instance(out, &scaled, p, tint);
}

/// Model ids at and above this are ground clutter, not objects of the map.
const CLUTTER_BASE: u32 = 0x8000_0000;
/// Model ids at and above this (and below the clutter) are objects of the event groups of the
/// mission, drawn where the events would place them.
const MISSION_BASE: u32 = 0x7000_0000;
/// Model ids at and above this (and below the clutter) are models buildings place through
/// proxies: `crate::proxies::Proxies::models`.
const PROXY_BASE: u32 = 0x7800_0000;
/// How far away the proxies of buildings are drawn, m.
const PROXY_RANGE: f32 = 150.0;

const SELECT_TINT: [f32; 4] = [1.0, 0.55, 0.05, 1.0];

/// Rebuilds the object draw list when something visible changed.
fn prepare_objects(app: &mut App, v: &View, top: bool) -> ObjectDraw {
    prepare_objects_for(app, v, top, top as u8)
}

/// The objects of view `which`: 0 the 3D view, 1 the top view, 2 the minimap.
fn prepare_objects_for(app: &mut App, v: &View, top: bool, which: u8) -> ObjectDraw {
    let renderer = app.renderer.clone();
    let renderer = renderer.lock().unwrap();
    let Some(doc) = &app.doc else { return ObjectDraw::default() };
    let world = &doc.world;
    let cache = match which { 0 => &mut app.view3, 1 => &mut app.view2, _ => &mut app.view_mini };
    if !app.show_objects || (world.objects.is_empty() && app.clutter_models.is_empty() && crate::mission_ui::ghosts_of(&app.mission, app.mode).is_none()) {
        cache.draw = ObjectDraw::default();
        return ObjectDraw::default();
    }
    // The top view lists more than is on screen, so panning needs no rebuild until it nears the edge.
    let shift = dot(sub(cache.eye, v.eye), sub(cache.eye, v.eye)).sqrt();
    let shadow_range = app.settings.lighting.shadow_range;
    let ghost_version = crate::mission_ui::ghosts_of(&app.mission, app.mode).map_or(0, |g| g.version() | 1);
    let toggles = (app.show_grass, app.show_shadows, if top { app.shading2 } else { app.shading3 }, shadow_range as i32, app.grass_under_objects, world.holes_rev, (app.settings.grass_ground_color * 100.0) as i32, app.pick_meshes.len(), ghost_version, app.show_proxies, app.proxies.rev);
    let toggled = cache.toggles != Some(toggles);
    let moved = toggled || if v.ortho {
        let margin = v.rect.width().min(v.rect.height()) * v.mpp * 0.2;
        [cache.eye[0] - v.eye[0], cache.eye[2] - v.eye[2]].iter().any(|d| d.abs() > margin) || (cache.mpp - v.mpp).abs() > 0.06 * v.mpp
    } else {
        shift > 4.0 || dot(sub(cache.forward, v.forward), sub(cache.forward, v.forward)) > 0.0006
    };
    let loaded = renderer.stats.models as usize + renderer.impostor_layers.len();
    cache.frames_since += 1;
    let fresh = cache.rev == world.objects.revision && cache.selection_rev == app.selection_rev && !moved && !(loaded != cache.loaded && cache.frames_since > 12);
    if fresh && cache.draw.instances.capacity() + cache.draw.boxes.capacity() + cache.draw.impostors.capacity() > 0 {
        return cache.draw.clone();
    }
    cache.frames_since = 0;
    cache.toggles = Some(toggles);
    cache.rev = world.objects.revision;
    cache.selection_rev = app.selection_rev;
    cache.eye = v.eye;
    cache.forward = v.forward;
    cache.mpp = v.mpp;
    cache.loaded = loaded;
    while cache.infos.len() < world.objects.models.len() {
        let path = world.objects.models[cache.infos.len()].path.clone();
        cache.tints.push(category_tint(&path));
        cache.infos.push(app.db.get(&path));
    }
    let planes = v.planes();
    let (hl, hh) = (doc.height_range[0] - 10.0, doc.height_range[1] + 80.0);
    let selection = &app.selection;
    let infos = &cache.infos;
    let tints = &cache.tints;
    let mut requests: Vec<(u32, u8)> = Vec::new();
    let mut proxy_requests: Vec<(u32, u8)> = Vec::new();
    let show_proxies = app.show_proxies;

    let mut boxes = Vec::new();
    let mut instances: HashMap<(u32, u8), Vec<f32>> = HashMap::new();
    let mut impostors = Vec::new();
    let mut shadow: HashMap<(u32, u8), Vec<f32>> = HashMap::new();
    // Objects drawn as impostors can still cast shadows, within this many triangles.
    let mut shadow_triangles: i64 = 6_000_000;
    if v.ortho && app.settings.top_models && v.mpp < 1.5 {
        // Close up the top view shows the models themselves from above, as a drone would.
        let half = (v.rect.width().max(v.rect.height()) * v.mpp * 0.75).max(10.0);
        let (ex, ez) = (v.eye[0], v.eye[2]);
        let mut triangles_left: i64 = 8_000_000;
        world.objects.for_each_in(ex - half, ez - half, ex + half, ez + half, |id, o| {
            if !world.object_visible(o) {
                return;
            }
            let info = &infos[o.model as usize];
            let sel = selection.contains(&id);
            let size = if info.known { (info.max[0] - info.min[0]).max(info.max[2] - info.min[2]) * o.scale } else { 2.0 };
            // Size on screen in points; what is too small to see stays an outline.
            let px = size / v.mpp;
            let y = ObjectStore::origin_y(o, &world.height);
            if px >= 3.0 {
                let tint = if sel { SELECT_TINT } else { [1.0, 1.0, 1.0, 0.0] };
                let (placed_m, y) = placed_matrix(o, info, &world.height, y);
                let m0 = o.model;
                let last = renderer.lod_count(m0).map_or(u8::MAX, |c| c.saturating_sub(1));
                let want = if px > 400.0 { 0 } else { (((400.0 / px).log2().floor() as u8).saturating_add(1)).min(12).min(last) };
                if let Some((lod, n)) = renderer.best_lod(m0, want).filter(|_| triangles_left > 0) {
                    triangles_left -= n as i64;
                    write_instance(instances.entry((m0, lod)).or_default(), &placed_m, [o.x, y, o.z], tint);
                    if lod != want {
                        requests.push((m0, want));
                    }
                    return;
                }
                requests.push((m0, want));
            }
            let tint = if sel { SELECT_TINT } else { tints[o.model as usize] };
            write_box(&mut boxes, o, doc.height_range[1] + 5.0, info, tint, (v.mpp * 2.0).max(0.2));
        });
    } else if v.ortho {
        // Top view: footprints, thinned out when zoomed far out.
        let half = (v.rect.width().max(v.rect.height()) * v.mpp * 0.75).max(10.0);
        let (ex, ez) = (v.eye[0], v.eye[2]);
        let keep = (v.mpp / 2.5).floor().max(1.0) as u32;
        let min_size = (v.mpp * 2.0).max(0.2);
        boxes.reserve(1 << 16);
        world.objects.for_each_in(ex - half, ez - half, ex + half, ez + half, |id, o| {
            if !world.object_visible(o) {
                return;
            }
            let sel = selection.contains(&id);
            if !sel && (keep > 1 && id % keep != 0) {
                return;
            }
            let tint = if sel { SELECT_TINT } else { tints[o.model as usize] };
            write_box(&mut boxes, o, doc.height_range[1] + 5.0, &infos[o.model as usize], tint, min_size);
        });
    } else {
        let range = app.settings.draw_distance;
        let (kx, kz) = grid_key(v.eye[0], v.eye[2]);
        let cells = (range / GRID_CELL).ceil() as i32 + 1;
        let near = app.settings.near_detail;
        let far = near * 4.0;
        let eye = v.eye;
        // Stops very dense views from overwhelming the GPU; the rest falls back to impostors.
        let mut triangles_left: i64 = 9_000_000;
        // Models with no billboard to fall back on draw from a budget of their own.
        let mut no_billboard_left: i64 = 4_000_000;
        // Only plants turn into billboards; buildings go down their own LODs instead.
        let plants: Vec<bool> = world.objects.models.iter().map(|m| is_plant(&m.path)).collect();
        // Nearest cells first: the triangle budget has to go to what is close to the camera.
        let mut order: Vec<(i32, i32, i32)> = Vec::with_capacity(((2 * cells + 1) * (2 * cells + 1)) as usize);
        for gz in kz - cells..=kz + cells {
            for gx in kx - cells..=kx + cells {
                order.push(((gx - kx).pow(2) + (gz - kz).pow(2), gx, gz));
            }
        }
        order.sort_unstable();
        {
            for (_, gx, gz) in order {
                let list = world.objects.cell((gx, gz));
                if list.is_empty() {
                    continue;
                }
                let (cx0, cz0) = (gx as f32 * GRID_CELL, gz as f32 * GRID_CELL);
                if crate::camera::box_outside(&planes, [cx0 - 30.0, hl, cz0 - 30.0], [cx0 + GRID_CELL + 30.0, hh, cz0 + GRID_CELL + 30.0]) {
                    continue;
                }
                for &id in list {
                    let Some(o) = world.objects.get(id) else { continue };
                    if !world.object_visible(o) {
                        continue;
                    }
                    let info = &infos[o.model as usize];
                    let y = ObjectStore::origin_y(o, &world.height);
                    let (dx, dy, dz) = (o.x - eye[0], y - eye[1], o.z - eye[2]);
                    let d = (dx * dx + dy * dy + dz * dz).sqrt();
                    let size = if info.known { (info.max[1] - info.min[1]).max(info.max[0] - info.min[0]).max(info.max[2] - info.min[2]) * o.scale } else { 2.0 };
                    if d > range || size / d < 0.0035 {
                        continue;
                    }
                    let sel = selection.contains(&id);
                    let tint = if sel { SELECT_TINT } else { [1.0, 1.0, 1.0, 0.0] };
                    let (placed_m, y) = placed_matrix(o, &infos[o.model as usize], &world.height, y);
                    let (m0, m1) = (o.model, renderer.impostor_layers.get(&o.model));
                    let last = renderer.lod_count(m0).map_or(u8::MAX, |c| c.saturating_sub(1));
                    // Each LOD of the model takes over at twice the distance of the one before it;
                    // large models keep theirs for longer.
                    let step = near * (size / 12.0).clamp(0.6, 3.0);
                    let want = if d < step { 0 } else { ((d / step).log2().floor() as u8).saturating_add(1) }.min(12).min(last);
                    let billboard = m1.filter(|_| plants.get(m0 as usize).copied().unwrap_or(false));
                    if d < far || sel || billboard.is_none() {
                        if let Some((lod, n)) = renderer.best_lod(m0, want).filter(|_| triangles_left > 0 || (billboard.is_none() && no_billboard_left > 0)) {
                            if triangles_left > 0 {
                                triangles_left -= n as i64;
                            } else {
                                no_billboard_left -= n as i64;
                            }
                            write_instance(instances.entry((m0, lod)).or_default(), &placed_m, [o.x, y, o.z], tint);
                            // What the building holds, as the game shows it close up.
                            if show_proxies && d < PROXY_RANGE {
                                if let Some(list) = app.proxies.get(&world.objects.models[m0 as usize].path, &app.db, &app.ctx) {
                                    for p in list.iter() {
                                        let (axes, at) = crate::proxies::place(&placed_m, [o.x, y, o.z], p);
                                        let id = PROXY_BASE + p.model;
                                        let want = if d < 40.0 { 0 } else if d < 90.0 { 1 } else { 2 }.min(renderer.lod_count(id).map_or(u8::MAX, |c| c.saturating_sub(1)));
                                        match renderer.best_lod(id, want).filter(|_| triangles_left > 0) {
                                            Some((plod, pn)) => {
                                                triangles_left -= pn as i64;
                                                write_instance(instances.entry((id, plod)).or_default(), &axes, at, tint);
                                                if plod != want { proxy_requests.push((p.model, want)); }
                                            }
                                            None => proxy_requests.push((p.model, want)),
                                        }
                                    }
                                }
                            }
                            if d < shadow_range {
                                // Shadows are cast by the next coarser LOD when it is there.
                                let caster = renderer.best_lod(m0, want.saturating_add(1).min(last)).map_or(lod, |l| l.0);
                                write_instance(shadow.entry((m0, caster)).or_default(), &placed_m, [o.x, y, o.z], tint);
                            }
                            if lod != want {
                                requests.push((m0, want));
                            }
                            if billboard.is_none() && m1.is_none() && plants.get(m0 as usize).copied().unwrap_or(false) {
                                requests.push((m0, LOD_BILLBOARD.min(last)));
                            }
                            continue;
                        }
                    }
                    requests.push((m0, want));
                    if m1.is_none() {
                        requests.push((m0, LOD_BILLBOARD.min(last)));
                    }
                    if d < shadow_range && app.show_shadows {
                        if let Some((lod, n)) = renderer.best_lod(m0, want).filter(|_| shadow_triangles > 0) {
                            shadow_triangles -= n as i64;
                            write_instance(shadow.entry((m0, lod)).or_default(), &placed_m, [o.x, y, o.z], tint);
                        }
                    }
                    if let Some(layer) = m1 {
                        if let Some(fit) = app.impostor_fits.get(&m0) {
                            impostors.extend_from_slice(&[o.x, y, o.z, fit[0] * o.scale, fit[1] * o.scale, fit[2] * o.scale, *layer as f32, 0.0]);
                            continue;
                        }
                    }
                    // Not loaded yet, or nothing to show.
                    write_box(&mut boxes, o, y, info, if sel { SELECT_TINT } else { tints[o.model as usize] }, 0.3);
                }
            }
        }
    }
    // The objects of the event groups of the mission, where the events would place them.
    let mut ghost_requests: Vec<(u32, u8)> = Vec::new();
    if let Some(g) = crate::mission_ui::ghosts_of(&app.mission, app.mode) {
        let close_top = v.ortho && app.settings.top_models && v.mpp < 1.5;
        if !v.ortho || close_top {
            let half = (v.rect.width().max(v.rect.height()) * v.mpp * 0.75).max(10.0);
            for gh in &g.list {
                let d = ((gh.x - v.eye[0]).powi(2) + (gh.z - v.eye[2]).powi(2)).sqrt();
                if if v.ortho { (gh.x - v.eye[0]).abs() > half || (gh.z - v.eye[2]).abs() > half } else { d > 2500.0 } {
                    continue;
                }
                let info = &g.infos[gh.model as usize];
                let o = Obj { x: gh.x, z: gh.z, elevation: gh.y.unwrap_or(0.0), yaw: gh.yaw, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: if gh.y.is_some() { dz_core::objects::FLAG_ABSOLUTE } else { 0 } };
                let y0 = ObjectStore::origin_y(&o, &world.height);
                let (m, y) = placed_matrix(&o, info, &world.height, y0);
                let id = MISSION_BASE + gh.model;
                let last = renderer.lod_count(id).map_or(u8::MAX, |c| c.saturating_sub(1));
                let want = if v.ortho { 0 } else if d < 80.0 { 0 } else if d < 400.0 { 1 } else { 2 }.min(last);
                match renderer.best_lod(id, want) {
                    Some((lod, _)) => {
                        write_instance(instances.entry((id, lod)).or_default(), &m, [gh.x, y, gh.z], [1.0, 0.72, 0.3, 0.3]);
                        if lod != want {
                            ghost_requests.push((gh.model, want));
                        }
                    }
                    None => ghost_requests.push((gh.model, want)),
                }
            }
        }
    }
    ghost_requests.sort_unstable();
    ghost_requests.dedup();
    // Ground clutter on a 1 m grid, thinned with distance like the game's clutter range.
    let mut clutter_requests: Vec<(u32, u8)> = Vec::new();
    if !v.ortho && app.show_grass && !app.clutter_models.is_empty() {
        const RANGE: i32 = 110;
        let shading = if top { app.shading2 } else { app.shading3 };
        if shading == Shading::Lit {
            let (ex, ez) = (v.eye[0].round() as i32, v.eye[2].round() as i32);
            let size = world.meta.size_m();
            let mut triangles_left: i64 = 5_000_000;
            // Colour of the satellite image under a clump, and how far it takes that colour.
            let ground = app.settings.grass_ground_color.clamp(0.0, 1.0);
            let ground_tint = |x: f32, z: f32| -> [f32; 4] {
                let Some(s) = world.satellite.as_ref().filter(|s| ground > 0.0 && s.width > 0 && s.height > 0) else { return [1.0, 1.0, 1.0, 0.0] };
                let px = ((x / size * s.width as f32) as usize).min(s.width - 1);
                let py = (((1.0 - z / size) * s.height as f32).max(0.0) as usize).min(s.height - 1);
                let i = (py * s.width + px) * 3;
                [s.data[i] as f32 / 255.0, s.data[i + 1] as f32 / 255.0, s.data[i + 2] as f32 / 255.0, -ground]
            };
            // The game keeps clutter off roads: (x, z, sin, cos, half width, half length).
            // Grass is ruled out cell by cell, one metre to a cell.
            let side = (2 * (RANGE + 3) + 1) as usize;
            let (ox, oz) = (ex - RANGE - 3, ez - RANGE - 3);
            let mut blocked = vec![false; side * side];
            let reach = RANGE as f32 + 40.0;
            let under_objects = app.grass_under_objects;
            let meshes = &app.pick_meshes;
            world.objects.for_each_in(v.eye[0] - reach, v.eye[2] - reach, v.eye[0] + reach, v.eye[2] + reach, |_, o| {
                if !world.object_visible(o) {
                    return;
                }
                let path = world.objects.model_path(o);
                let road = path.contains(r"\roads\parts\");
                if !road && (under_objects || is_plant(path) || crate::models::is_water_model(path)) {
                    return;
                }
                // The shape of the model where it meets the ground, once its geometry is in.
                let footprint = if road { None } else { meshes.get(&o.model).map(|m| &m.footprint).filter(|f| f.w > 0) };
                // A wall's bounding box can span a whole yard, while the wall itself covers next to nothing.
                if footprint.is_none() && !road && (path.contains(r"\walls\") || path.contains("fence")) {
                    return;
                }
                let info = &infos[o.model as usize];
                if !road && !info.known {
                    return;
                }
                let (sin, cos) = o.yaw.to_radians().sin_cos();
                let (cx, cz) = ((info.min[0] + info.max[0]) * 0.5 * o.scale, (info.min[2] + info.max[2]) * 0.5 * o.scale);
                let (hx, hz) = info.footprint();
                let (hx, hz) = (hx * o.scale + 0.3, hz * o.scale + 0.3);
                let (bx, bz) = (o.x + cos * cx + sin * cz, o.z - sin * cx + cos * cz);
                let r = (hx * hx + hz * hz).sqrt();
                let span = |lo: f32, hi: f32, origin: i32| ((lo.floor() as i32 - origin).max(0), (hi.ceil() as i32 - origin).min(side as i32 - 1));
                let (x0, x1) = span(bx - r, bx + r, ox);
                let (z0, z1) = span(bz - r, bz + r, oz);
                if let Some(f) = footprint {
                    // Grass goes where the model has nothing within a metre and a half of the
                    // ground: under eaves and canopies it stays, under walls and floors it does not.
                    let m = rotation(o.yaw, o.pitch, o.roll, o.scale.max(0.01));
                    let s2 = o.scale.max(0.01).powi(2);
                    let origin = ObjectStore::origin_y(o, &world.height);
                    for gz in z0..=z1 {
                        for gx in x0..=x1 {
                            let (wx, wz) = ((ox + gx) as f32 + 0.5, (oz + gz) as f32 + 0.5);
                            let rel = [wx - o.x, world.height.sample(wx, wz) - origin, wz - o.z];
                            let p = [dot(rel, m[0]) / s2, dot(rel, m[1]) / s2, dot(rel, m[2]) / s2];
                            if f.low_at(p[0], p[2]) < p[1] + 1.5 / o.scale.max(0.01) {
                                blocked[gz as usize * side + gx as usize] = true;
                            }
                        }
                    }
                    return;
                }
                for gz in z0..=z1 {
                    for gx in x0..=x1 {
                        let (dx, dz) = ((ox + gx) as f32 + 0.5 - bx, (oz + gz) as f32 + 0.5 - bz);
                        if (dx * cos - dz * sin).abs() < hx && (dx * sin + dz * cos).abs() < hz {
                            blocked[gz as usize * side + gx as usize] = true;
                        }
                    }
                }
            });
            let on_ground_kept = |x: f32, z: f32| {
                let (gx, gz) = (x.floor() as i32 - ox, z.floor() as i32 - oz);
                gx >= 0 && gz >= 0 && (gx as usize) < side && (gz as usize) < side && blocked[gz as usize * side + gx as usize]
            };
            for z in ez - RANGE..=ez + RANGE {
                for x in ex - RANGE..=ex + RANGE {
                    let h = (x as u32).wrapping_mul(73856093) ^ (z as u32).wrapping_mul(19349663);
                    let h = h.wrapping_mul(2654435761);
                    let d2 = ((x - ex).pow(2) + (z - ez).pow(2)) as f32;
                    if d2 > (RANGE * RANGE) as f32 {
                        continue;
                    }
                    let d = d2.sqrt();
                    let keep = if d < 35.0 { 1.0 } else { (1.0 - (d - 35.0) / 75.0).max(0.12) };
                    if (h & 1023) as f32 / 1023.0 > keep {
                        continue;
                    }
                    let (wx, wz) = (x as f32 + ((h >> 10) & 255) as f32 / 255.0, z as f32 + ((h >> 18) & 255) as f32 / 255.0);
                    if wx < 0.0 || wz < 0.0 || wx >= size || wz >= size {
                        continue;
                    }
                    if world.is_hole(wx, wz) {
                        continue;
                    }
                    let r = h.rotate_left(11).wrapping_mul(40503);
                    let pick = (r & 0xFFFF) as f32 / 65535.0;
                    let Some(list) = app.clutter_layers.get(world.mask.sample(wx, wz, size) as usize) else { continue };
                    let Some(&(model, _, a, b)) = list.iter().find(|e| pick < e.1) else { continue };
                    if on_ground_kept(wx, wz) {
                        continue;
                    }
                    let y = world.height.sample(wx, wz);
                    if y < world.meta.sea_level || crate::camera::box_outside(&planes, [wx - 2.0, y - 2.0, wz - 2.0], [wx + 2.0, y + 3.0, wz + 2.0]) {
                        continue;
                    }
                    let id = CLUTTER_BASE + model;
                    let want = if d < 25.0 { LOD_NEAR } else { LOD_FAR }.min(renderer.lod_count(id).map_or(u8::MAX, |c| c.saturating_sub(1)));
                    let Some((lod, n)) = renderer.best_lod(id, want).filter(|_| triangles_left > 0) else {
                        clutter_requests.push((model, want));
                        continue;
                    };
                    if lod != want {
                        clutter_requests.push((model, want));
                    }
                    triangles_left -= n as i64;
                    let k = ((r >> 16) & 1023) as f32 / 1023.0;
                    // Shrinks toward the edge of the range so grass does not pop in.
                    let fade = ((RANGE as f32 - d) / 18.0).clamp(0.0, 1.0);
                    let yaw = ((r >> 5) & 511) as f32 * 0.7045;
                    write_instance(instances.entry((id, lod)).or_default(), &rotation(yaw, 0.0, 0.0, (a + (b - a) * k) * fade), [wx, y, wz], ground_tint(wx, wz));
                }
            }
        }
    }
    clutter_requests.sort_unstable();
    clutter_requests.dedup();
    let mut flat = Vec::with_capacity(instances.values().map(|v| v.len()).sum());
    let mut groups = Vec::with_capacity(instances.len());
    for (k, list) in instances {
        groups.push((k, (flat.len() / 16) as u32, (list.len() / 16) as u32));
        flat.extend_from_slice(&list);
    }
    // Keeps first-come order, which is nearest first, so close models load first.
    let mut requested = std::collections::HashSet::new();
    requests.retain(|r| requested.insert(*r));
    let mut shadow_flat = Vec::with_capacity(shadow.values().map(|v| v.len()).sum());
    let mut shadow_groups = Vec::with_capacity(shadow.len());
    for (k, list) in shadow {
        shadow_groups.push((k, (shadow_flat.len() / 16) as u32, (list.len() / 16) as u32));
        shadow_flat.extend_from_slice(&list);
    }
    let draw = ObjectDraw { instances: Arc::new(flat), groups: Arc::new(groups), boxes: Arc::new(boxes), impostors: Arc::new(impostors), shadow_instances: Arc::new(shadow_flat), shadow_groups: Arc::new(shadow_groups) };
    cache.draw = draw.clone();
    drop(renderer);
    let to_load: Vec<(u32, u8, String)> = requests.into_iter().map(|(m, l)| (m, l, world.objects.models[m as usize].path.clone())).collect();
    for (m, l, path) in to_load {
        app.loader.request((m, l), &path);
    }
    for (m, l) in ghost_requests {
        if let Some(path) = app.mission.ghosts.models.get(m as usize).cloned() {
            app.loader.request((MISSION_BASE + m, l), &path);
        }
    }
    let mut seen = std::collections::HashSet::new();
    proxy_requests.retain(|r| seen.insert(*r));
    for (m, l) in proxy_requests {
        if let Some(path) = app.proxies.models.get(m as usize).cloned() {
            app.loader.request((PROXY_BASE + m, l), &path);
        }
    }
    for (m, l) in clutter_requests {
        let path = app.clutter_models[m as usize].clone();
        app.loader.request((CLUTTER_BASE + m, l), &path);
    }
    draw
}

fn begin_paint_or_sculpt(app: &mut App, p: V3, invert: bool, smooth: bool) {
    let Some(doc) = &mut app.doc else { return };
    match app.mode {
        Mode::Paint => {
            let guard = dz_core::mask_limit::TileGuard::new(&doc.world);
            let stamp = crate::brushes::stamp(&app.brushes, &app.paint.picture);
            let spray = if app.paint.spray { app.paint.spray_density } else { 0.0 };
            app.mask_stroke = Some(MaskStroke { shape: app.paint.shape, only_on: app.paint.only_on.map(|l| l as u8), guard: Some(guard), stamp, spray, ..Default::default() });
        }
        Mode::Landscape => {
            let brush = Brush { radius: app.sculpt.radius, falloff: app.sculpt.falloff, strength: app.sculpt.strength };
            let tool = if smooth {
                SculptTool::Smooth
            } else if invert && app.sculpt.tool == SculptTool::Raise {
                SculptTool::Lower
            } else if invert && app.sculpt.tool == SculptTool::Lower {
                SculptTool::Raise
            } else {
                app.sculpt.tool
            };
            let mut stroke = SculptStroke::begin(tool, brush, &doc.world, p[0], p[2]); stroke.shape = app.sculpt.shape; stroke.edge_noise = app.sculpt.noise;
            stroke.stamp = crate::brushes::stamp(&app.brushes, &app.sculpt.picture);
            stroke.spray = if app.sculpt.spray { app.sculpt.spray_density } else { 0.0 };
            app.stroke = Some(stroke);
        }
        _ => {}
    }
}

fn finish_strokes(app: &mut App) {
    if let (Some(s), Some(doc)) = (app.stroke.take(), app.doc.as_mut()) {
        if let Some(c) = s.finish() {
            doc.history.push(t("Sculpt terrain"), c);
        }
    }
    if let (Some(s), Some(doc)) = (app.mask_stroke.take(), app.doc.as_mut()) {
        if let Some(c) = s.finish() {
            doc.history.push(t("Paint surface"), c);
        }
    }
}

/// Adds the terrain cells under the brush to the holes, or takes them away.
fn paint_holes(app: &mut App, x: f32, z: f32, erase: bool) {
    let radius = app.hole_radius;
    let Some(doc) = &mut app.doc else { return };
    let world = &mut doc.world;
    if app.hole_before.is_none() {
        app.hole_before = Some(world.holes.clone());
    }
    let cell = world.height.cell;
    let (cols, rows) = (world.height.cols as i64 - 1, world.height.rows as i64 - 1);
    let span = |a: f32, limit: i64| (((a - radius) / cell).floor() as i64).clamp(0, limit)..=(((a + radius) / cell).floor() as i64).clamp(0, limit - 1).max(0);
    let mut changed = false;
    for j in span(z, rows) {
        for i in span(x, cols) {
            let tile = [i as u32, j as u32];
            changed |= if erase { world.holes.remove(&tile) } else { world.holes.insert(tile) };
        }
    }
    if changed {
        world.holes_rev += 1;
    }
}

/// One step of the placement brush: fills the circle up to the spacing, or empties it.
fn brush_objects(app: &mut App, x: f32, z: f32, erase: bool) {
    let (radius, spacing) = (app.place.brush_radius, app.place.brush_spacing.max(0.3));
    if erase {
        let Some(doc) = &mut app.doc else { return };
        let mut found = Vec::new();
        let world = &doc.world;
        world.objects.for_each_in(x - radius, z - radius, x + radius, z + radius, |id, o| {
            if world.object_interactive(o) && (o.x - x).powi(2) + (o.z - z).powi(2) <= radius * radius {
                found.push(id);
            }
        });
        for id in found {
            if let Some(o) = doc.world.objects.remove(id) {
                app.place_stroke.push((id, Some(o), None));
            }
        }
        return;
    }
    let saved = (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max);
    for _ in 0..6 {
        app.place_counter = app.place_counter.wrapping_add(1);
        let h = app.place_counter.wrapping_mul(2654435761);
        let angle = (h & 0xffff) as f32 / 65535.0 * std::f32::consts::TAU;
        let distance = ((h >> 16) as f32 / 65535.0).sqrt() * radius;
        let (px, pz) = (x + angle.cos() * distance, z + angle.sin() * distance);
        let Some(doc) = &app.doc else { return };
        let mut crowded = false;
        doc.world.objects.for_each_in(px - spacing, pz - spacing, px + spacing, pz + spacing, |_, o| {
            crowded |= (o.x - px).powi(2) + (o.z - pz).powi(2) < spacing * spacing;
        });
        if crowded {
            continue;
        }
        let model = match app.place.preset {
            Some(id) => match crate::library_ui::pick_from_preset(app, id, h) {
                Some((m, s)) => {
                    (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max) = s;
                    m
                }
                None => return,
            },
            None => match pool_model(app, h) {
                Some(m) => m,
                None => {
                    app.warn(t("Choose a model in the Content Browser first"));
                    return;
                }
            },
        };
        if let Some(placed) = insert_placed(app, &model, px, pz) {
            app.place_stroke.push((placed.0, None, Some(placed.1)));
        }
        (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max) = saved;
    }
}

fn finish_place_stroke(app: &mut App) {
    if app.place_stroke.is_empty() {
        return;
    }
    let change = std::mem::take(&mut app.place_stroke);
    if let Some(doc) = &mut app.doc {
        doc.history.push(t("Place object"), Change::Objects(change));
        doc.dirty.objects = true;
    }
}

/// Adds an object with the placement options in force.
pub(crate) fn insert_placed(app: &mut App, model: &str, x: f32, z: f32) -> Option<(u32, Obj)> {
    let (random, align, smin, smax) = (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max);
    let info = app.db.get(model);
    let doc = app.doc.as_mut()?;
    let layer = doc.world.target_layer();
    if !doc.world.layer_interactive(layer) {
        let w = crate::app::layer_warning(&doc.world, layer);
        app.warn(w);
        return None;
    }
    let model_id = doc.world.objects.intern(model);
    let seed = (x.to_bits() ^ z.to_bits().rotate_left(13)).wrapping_mul(2654435761);
    let mut yaw = if random { (seed >> 7) as f32 / (u32::MAX >> 7) as f32 * 360.0 } else { 0.0 };
    // With the snap key held a random turn goes by whole angle steps.
    if app.settings.keys.held(Hold::AngleSnap, app.modifiers_now) {
        let step = app.settings.angle_step.max(0.1);
        yaw = ((yaw / step).round() * step).rem_euclid(360.0);
    }
    let k = ((seed >> 3) & 1023) as f32 / 1023.0;
    let scale = smin + (smax - smin) * k;
    // Models with a placement property are fitted to the ground by the game; tilting them too
    // would make them lean.
    let (pitch, roll) = if align && !info.placement.skews() { gen::slope_angles(&doc.world.height, x, z, yaw) } else { (0.0, 0.0) };
    // The model rests on its land contact points.
    let elevation = if info.contact_count > 0 { -info.contact_y * scale } else { 0.0 };
    let mut o = Obj { x, z, elevation, yaw, pitch, roll, scale, model: model_id, flags: 0 };
    o.set_layer(layer);
    Some((doc.world.objects.insert(o), o))
}

/// The model to place: one of the models selected together in the browser, or the chosen one.
fn pool_model(app: &App, seed: u32) -> Option<String> {
    let model = app.place.model.clone()?;
    let pool = &app.place.pool;
    if pool.len() > 1 && pool.contains(&model) {
        return Some(pool[(seed >> 8) as usize % pool.len()].clone());
    }
    Some(model)
}

pub fn place_model(app: &mut App, model: &str, x: f32, z: f32) {
    let (auto_pad, falloff) = (app.place.auto_pad, app.place.pad_falloff);
    let Some((id, o)) = insert_placed(app, model, x, z) else { return };
    let Some(doc) = &mut app.doc else { return };
    doc.history.push(t("Place object"), Change::Objects(vec![(id, None, Some(o))]));
    doc.dirty.objects = true;
    if auto_pad {
        app.add_pads(&[id], falloff);
    }
}

fn finish_line(app: &mut App) {
    let points = std::mem::take(&mut app.line.points);
    if points.len() < 2 {
        return;
    }
    let (action, family, grade, model, spacing) = (app.line.action, app.line.family.clone(), app.line.grade, app.line.fence_model.clone(), app.line.fence_spacing);
    let Some(doc) = &app.doc else { return };
    match action {
        LineAction::Plain => {
            app.shape.tool = ShapeTool::Polyline;
            app.shape.points = points;
            app.finish_shape();
            app.shape.tool = ShapeTool::Edit;
        }
        LineAction::Fence => {
            if model.is_empty() {
                app.warn(t("Choose a model for the line first"));
                return;
            }
            let mut lite = doc.world.lite();
            let rule = gen::LineRule { model, spacing, offset: 0.0, jitter: 0.0, yaw_offset: 0.0, follow_slope: false };
            app.spawn(t("Placing objects along the line"), move |_| {
                let objects = gen::along_line(&mut lite, &points, &rule, 1);
                let models: Vec<String> = lite.objects.models.iter().map(|m| m.path.clone()).collect();
                Ok(Box::new(move |a: &mut App| {
                    a.merge_generated(&t("Objects along a line"), models, objects);
                }) as Finish)
            });
        }
        LineAction::Edit => {}
        LineAction::Road => {
            let settings = if dayz_road_tool::rails::is_track_family(&family) { app.line.rail_routing.clone() } else { app.line.routing.clone() };
            // A live road is built as it was shown, along the clicked line.
            let terrain_route = app.line.route && app.line.draw == RoadDraw::Points;
            app.live.shown = None;
            let req = app.route_request(family, points, terrain_route, settings);
            // The route remembers this; its modifier is made when the models are in place.
            app.start_route(req, None, None, grade);
        }
    }
}

fn selection_pivot(app: &App) -> Option<V3> {
    let doc = app.doc.as_ref()?;
    let (mut s, mut n) = ([0.0f32; 3], 0.0);
    for id in app.selection.iter().take(2000) {
        let o = doc.world.objects.get(*id)?;
        s = add(s, [o.x, ObjectStore::origin_y(o, &doc.world.height), o.z]);
        n += 1.0;
    }
    (n > 0.0).then(|| mul(s, 1.0 / n))
}

/// The axes the gizmo moves and turns along: those of the world, or of the first selected
/// object when local axes are chosen.
fn gizmo_axes(app: &App) -> [V3; 3] {
    let world = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    if !app.gizmo_local {
        return world;
    }
    let Some(o) = app.selection.iter().next().and_then(|id| app.doc.as_ref()?.world.objects.get(*id)) else { return world };
    let m = rotation(o.yaw, o.pitch, o.roll, 1.0);
    [norm(m[0]), norm(m[1]), norm(m[2])]
}

/// Points of the ring the gizmo turns about `axis` with, and the two axes it spans.
fn ring_points(p: V3, axes: &[V3; 3], axis: usize, radius: f32) -> Vec<V3> {
    let (u, w) = (axes[(axis + 1) % 3], axes[(axis + 2) % 3]);
    (0..=48).map(|k| {
        let a = k as f32 / 48.0 * std::f32::consts::TAU;
        add(p, add(mul(u, a.cos() * radius), mul(w, a.sin() * radius)))
    }).collect()
}

/// Turns `v` about the unit `axis` by `angle` radians, by the right hand.
fn turn(v: V3, axis: V3, angle: f32) -> V3 {
    let (s, c) = angle.sin_cos();
    let cr = crate::camera::cross(axis, v);
    add(add(mul(v, c), mul(cr, s)), mul(axis, dot(axis, v) * (1.0 - c)))
}

fn gizmo_size(v: &View, p: V3) -> f32 {
    if v.ortho {
        v.mpp * 90.0
    } else {
        dot(sub(p, v.eye), v.forward).max(1.0) * 0.14
    }
}

/// Whether a handle works on the selection. A whole road or generator only slides over the
/// map and turns about the vertical, the moves its line and shapes follow.
fn handle_allowed(app: &App, h: Handle) -> bool {
    app.whole.is_none() || matches!(h, Handle::Plane | Handle::X | Handle::Z | Handle::RotY | Handle::Spin)
}

fn gizmo_handle(app: &App, v: &View, p: V3, mouse: Pos2) -> Option<(Handle, egui::Vec2, V3, f32)> {
    gizmo_handle_any(app, v, p, mouse).filter(|h| handle_allowed(app, h.0))
}

fn gizmo_handle_any(app: &App, v: &View, p: V3, mouse: Pos2) -> Option<(Handle, egui::Vec2, V3, f32)> {
    let c = v.project(p)?;
    let size = gizmo_size(v, p);
    match app.gizmo {
        Gizmo::Move => {
            if (mouse - c).length() < 9.0 {
                return Some((Handle::Plane, vec2(0.0, 0.0), [0.0; 3], 0.0));
            }
            let g = gizmo_axes(app);
            for (h, a) in [(Handle::X, g[0]), (Handle::Z, g[2]), (Handle::Y, g[1])] {
                let Some(e) = v.project(add(p, mul(a, size))) else { continue };
                let (dir, len) = ((e - c).normalized(), (e - c).length());
                // An axis seen end on cannot be dragged along.
                if len < 12.0 {
                    continue;
                }
                let along = (mouse - c).dot(dir);
                let off = ((mouse - c) - dir * along).length();
                if along > 8.0 && along < len + 8.0 && off < 6.0 {
                    return Some((h, dir, a, size / len.max(1.0)));
                }
            }
            None
        }
        Gizmo::Rotate => {
            // Three rings, one about each axis; seen from above only the one about the vertical.
            let g = gizmo_axes(app);
            let mut best: Option<(f32, Handle, V3)> = None;
            for (k, h) in [(0, Handle::RotX), (1, Handle::RotY), (2, Handle::RotZ)] {
                if (v.ortho && !app.gizmo_local && k != 1) || !handle_allowed(app, h) {
                    continue;
                }
                let pts: Vec<Pos2> = ring_points(p, &g, k, size * 0.85).into_iter().filter_map(|q| v.project(q)).collect();
                for s in pts.windows(2) {
                    let d = near_segment(mouse, s[0], s[1]);
                    if d < 7.0 && best.is_none_or(|b| d < b.0) {
                        best = Some((d, h, g[k]));
                    }
                }
            }
            best.map(|(_, h, a)| (h, vec2(0.0, 0.0), a, 0.0))
        }
        Gizmo::Scale => (mouse - c).length().lt(&60.0).then_some((Handle::Scale, vec2(0.0, 0.0), [0.0; 3], 0.0)),
    }
}

/// A point snapped so the line to it from `from` runs at a whole step of `step` degrees,
/// counted clockwise from north as yaw is.
pub fn snap_direction(from: [f32; 2], to: [f32; 2], step: f32) -> [f32; 2] {
    let (dx, dz) = (to[0] - from[0], to[1] - from[1]);
    let len = dx.hypot(dz);
    if len < 1e-4 || step <= 0.0 {
        return to;
    }
    let a = (dx.atan2(dz).to_degrees() / step).round() * step;
    let (sin, cos) = a.to_radians().sin_cos();
    [from[0] + sin * len, from[1] + cos * len]
}

/// Where a new point of a line or shape goes: with the snap key held, at a whole angle step
/// from the point before it.
fn line_point(app: &App, points: &[[f32; 2]], p: [f32; 2], modifiers: egui::Modifiers) -> [f32; 2] {
    match points.last() {
        Some(last) if app.settings.keys.held(Hold::AngleSnap, modifiers) => snap_direction(*last, p, app.settings.angle_step),
        _ => p,
    }
}

/// The angle and length of the segment being drawn, next to the cursor.
fn segment_label(painter: &egui::Painter, v: &View, world: &dz_core::world::World, from: [f32; 2], to: [f32; 2]) {
    let (dx, dz) = (to[0] - from[0], to[1] - from[1]);
    let len = dx.hypot(dz);
    if len < 0.5 {
        return;
    }
    let angle = dx.atan2(dz).to_degrees().rem_euclid(360.0);
    if let Some(s) = v.project([to[0], world.height.sample(to[0], to[1]), to[1]]) {
        painter.text(s + vec2(12.0, -12.0), egui::Align2::LEFT_BOTTOM, format!("{angle:.0}°  {len:.1} m"), egui::FontId::proportional(12.0), Color32::WHITE);
    }
}

fn apply_object_drag(app: &mut App, v: &View, mouse: Pos2, snap: bool) {
    let Some(mut drag) = app.drag.take() else { return };
    // Held for precise work, the drag follows the mouse five times slower.
    // The key for slow moves does not slow a drag it chose itself (Alt scales).
    let precise = app.settings.keys.held(Hold::Precise, app.modifiers_now) && !app.settings.keys.held(Hold::Precise, drag.mods);
    let (seen, last) = drag.virt.unwrap_or((mouse, mouse));
    let seen = seen + (mouse - last) * if precise { 0.2 } else { 1.0 };
    drag.virt = Some((seen, mouse));
    let mouse = seen;
    let angle_snap = app.settings.keys.held(Hold::AngleSnap, app.modifiers_now);
    let step = app.settings.angle_step.max(0.1);
    let delta = mouse - drag.start;
    if !drag.started {
        if delta.length() < 3.0 {
            app.drag = Some(drag);
            return;
        }
        drag.started = true;
    }
    let Some(doc) = &mut app.doc else { return };
    let world = &mut doc.world;
    let mut area: Option<dz_core::terrain::Rect> = None;
    let (mut dx, mut dy, mut dz, mut dyaw, mut factor) = (0.0, 0.0, 0.0, 0.0, 1.0);
    match drag.handle {
        Handle::Plane => {
            let (o, d) = v.ray(mouse);
            // Without ground under the cursor the object slides at its own height.
            let on_level = (d[1].abs() > 1e-4).then(|| (drag.pivot[1] - o[1]) / d[1]).filter(|t| *t > 0.0).map(|t| add(o, mul(d, t)));
            if let Some(p) = pick_terrain(&world.height, o, d, v.ortho).or(on_level) {
                dx = p[0] + drag.grab_offset[0] - drag.pivot[0];
                dz = p[2] + drag.grab_offset[1] - drag.pivot[2];
            }
        }
        Handle::Spin => {
            dyaw = delta.x * 0.5;
            if angle_snap || drag.stepped { dyaw = (dyaw / step).round() * step; }
        }
        Handle::X | Handle::Y | Handle::Z => {
            let along = delta.dot(drag.axis.0) * drag.axis.2;
            let along = if snap { (along).round() } else { along };
            dx = drag.axis.1[0] * along;
            dy = drag.axis.1[1] * along;
            dz = drag.axis.1[2] * along;
        }
        Handle::RotX | Handle::RotY | Handle::RotZ => {
            let axis = norm(drag.axis.1);
            let c = v.project(drag.pivot).unwrap_or(drag.start);
            let mut a = (mouse - c).angle() - (drag.start - c).angle();
            a = (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            // Which way the ring runs on screen decides which way the mouse turns it.
            let (u, w) = { let n = if axis[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }; let u = norm(crate::camera::cross(n, axis)); (u, crate::camera::cross(axis, u)) };
            let size = gizmo_size(v, drag.pivot);
            if let (Some(pu), Some(pw)) = (v.project(add(drag.pivot, mul(u, size))), v.project(add(drag.pivot, mul(w, size)))) {
                let (su, sw) = (pu - c, pw - c);
                if su.x * sw.y - su.y * sw.x < 0.0 { a = -a; }
            }
            let mut deg = a.to_degrees();
            if snap || angle_snap { deg = (deg / step).round() * step; }
            let angle = deg.to_radians();
            let mut updates = Vec::new();
            for (id, b) in &drag.before {
                let mut o = *b;
                let m = rotation(b.yaw, b.pitch, b.roll, 1.0);
                let rows = [turn(m[0], axis, angle), turn(m[1], axis, angle), turn(m[2], axis, angle)];
                let (yaw, pitch, roll, _) = dz_core::objects::angles(&rows);
                (o.yaw, o.pitch, o.roll) = (yaw, pitch, roll);
                // Several objects turn about their common middle.
                let rel = [b.x - drag.pivot[0], ObjectStore::origin_y(b, &world.height) - drag.pivot[1], b.z - drag.pivot[2]];
                let moved = turn(rel, axis, angle);
                o.x = drag.pivot[0] + moved[0];
                o.z = drag.pivot[2] + moved[2];
                o.elevation = b.elevation + (moved[1] - rel[1]);
                updates.push((*id, o));
            }
            for (id, o) in updates {
                world.objects.update(id, o);
                if let Some(r) = world.sync_attached(id) {
                    area = Some(area.map_or(r, |a| a.union(r)));
                }
            }
            doc.dirty.objects = true;
            app.drag = Some(drag);
            if let Some(r) = area {
                app.height_changed(r);
            }
            return;
        }
        Handle::Scale => {
            factor = (1.0 + delta.x * 0.01).max(0.05);
        }
    }
    let (sin, cos) = dyaw.to_radians().sin_cos();
    let mut updates = Vec::new();
    for (id, b) in &drag.before {
        let mut o = *b;
        o.x += dx;
        o.z += dz;
        o.elevation += dy;
        if dyaw != 0.0 {
            let (rx, rz) = (b.x - drag.pivot[0], b.z - drag.pivot[2]);
            o.x = drag.pivot[0] + rx * cos + rz * sin;
            o.z = drag.pivot[2] - rx * sin + rz * cos;
            o.yaw = b.yaw + dyaw;
        }
        o.scale = b.scale * factor;
        updates.push((*id, o));
    }
    // Stuck to the ground, moved and turned objects take its tilt; a whole road or
    // generator keeps its own, and models the game fits to the ground stay level.
    if app.settings.align_to_ground && app.whole.is_none() && matches!(drag.handle, Handle::Plane | Handle::X | Handle::Z | Handle::Spin) {
        for (_, o) in &mut updates {
            if !app.db.get(world.objects.model_path(o)).placement.skews() {
                (o.pitch, o.roll) = gen::slope_angles(&world.height, o.x, o.z, o.yaw);
            }
        }
    }
    for (id, o) in updates {
        world.objects.update(id, o);
        if let Some(r) = world.sync_attached(id) {
            area = Some(area.map_or(r, |a| a.union(r)));
        }
    }
    doc.dirty.objects = true;
    app.drag = Some(drag);
    if let Some(r) = area {
        app.height_changed(r);
    }
}

/// Ends a drag; a release before the mouse moved gives back the click it was.
fn end_object_drag(app: &mut App) -> Option<(Pos2, bool)> {
    let Some(drag) = app.drag.take() else { return None };
    if !drag.started {
        if let Some(c) = drag.click {
            return Some(c);
        }
    }
    if let (Some(id), false) = (drag.toggle, drag.started) {
        app.selection.remove(&id);
        app.selection_rev += 1;
        return None;
    }
    let Some(doc) = &mut app.doc else { return None };
    let change: Vec<_> = drag.before.iter().filter_map(|(id, b)| {
        let now = doc.world.objects.get(*id).copied()?;
        (now != *b).then_some((*id, Some(*b), Some(now)))
    }).collect();
    if change.is_empty() {
        return None;
    }
    // A whole road or generator brings its line, modifier and shape along; a change of size
    // leaves them as they are.
    let motion = change.first().and_then(|(_, b, n)| Some((b.as_ref()?, n.as_ref()?))).filter(|(b, n)| (b.scale - n.scale).abs() < 1e-5).map(|(b, n)| dz_core::world::Motion::between(b, n, [drag.pivot[0], drag.pivot[2]]));
    let extra = match motion.filter(|m| !m.is_still()) {
        Some(m) if app.whole.is_some() => app.move_whole(&m),
        _ => Vec::new(),
    };
    let Some(doc) = &mut app.doc else { return None };
    if extra.is_empty() {
        doc.history.push(t("Transform objects"), Change::Objects(change));
    } else {
        let mut steps = vec![Change::Objects(change)];
        steps.extend(extra);
        doc.history.push(t("Transform objects"), Change::Group(steps));
    }
    None
}

fn camera_input(app: &mut App, ctx: &egui::Context, resp: &egui::Response, v: &View, top: bool, typing: bool) {
    let dt = ctx.input(|i| i.stable_dt).min(0.1);
    let hover = resp.hovered() || resp.dragged();
    app.view_hovered |= hover;
    let (scroll, modifiers) = ctx.input(|i| (i.smooth_scroll_delta.y, i.modifiers));
    if top {
        let drag = if resp.dragged_by(PointerButton::Middle) || resp.dragged_by(PointerButton::Secondary) { resp.drag_delta() } else { vec2(0.0, 0.0) };
        app.top_center[0] -= drag.x * app.top_mpp;
        app.top_center[1] += drag.y * app.top_mpp;
        if hover && scroll != 0.0 && !brush_wheel(app, modifiers) && !app.minimap_hover {
            if let Some(m) = resp.hover_pos() {
                let (ox, oz) = ((m.x - v.rect.center().x) * app.top_mpp, (v.rect.center().y - m.y) * app.top_mpp);
                let before = [app.top_center[0] + ox, app.top_center[1] + oz];
                app.top_mpp = (app.top_mpp * (-scroll * 0.0025).exp()).clamp(0.02, 60.0);
                app.top_center = [before[0] - (m.x - v.rect.center().x) * app.top_mpp, before[1] - (v.rect.center().y - m.y) * app.top_mpp];
                ctx.request_repaint();
            }
        }
        return;
    }
    let looking = resp.dragged_by(PointerButton::Secondary);
    if looking {
        let d = resp.drag_delta();
        app.camera.yaw += d.x * 0.0038;
        app.camera.pitch = (app.camera.pitch - d.y * 0.0038).clamp(-1.54, 1.54);
        if scroll != 0.0 {
            app.camera.speed = (app.camera.speed * (scroll * 0.002).exp()).clamp(1.0, 4000.0);
            app.settings.camera_speed = app.camera.speed;
        }
    } else if hover && scroll != 0.0 && !brush_wheel(app, modifiers) && !app.minimap_hover && !app.drag.as_ref().is_some_and(|d| d.stepped) {
        let f = app.camera.forward();
        let clearance = app.doc.as_ref().map(|d| (app.camera.pos[1] - d.world.height.sample(app.camera.pos[0], app.camera.pos[2])).abs()).unwrap_or(50.0);
        app.camera.pos = add(app.camera.pos, mul(f, scroll * (clearance * 0.0035).max(0.15)));
        ctx.request_repaint();
    }
    if resp.dragged_by(PointerButton::Middle) {
        let d = resp.drag_delta();
        let k = app.camera.speed * 0.012;
        let (r, u) = (v.right, v.up);
        app.camera.pos = add(add(app.camera.pos, mul(r, -d.x * k)), mul(u, d.y * k));
    }
    if (looking || hover) && !typing {
        let (mut mv, mut fast) = ([0.0f32; 3], false);
        let keys = &app.settings.keys;
        ctx.input(|i| {
            let f = app.camera.forward();
            let r = app.camera.right();
            fast = keys.held(Hold::Fast, i.modifiers);
            if looking {
                if keys.down(i, Action::CameraForward) { mv = add(mv, f); }
                if keys.down(i, Action::CameraBack) { mv = sub(mv, f); }
                if keys.down(i, Action::CameraRight) { mv = add(mv, r); }
                if keys.down(i, Action::CameraLeft) { mv = sub(mv, r); }
                if keys.down(i, Action::CameraUp) { mv = add(mv, [0.0, 1.0, 0.0]); }
                if keys.down(i, Action::CameraDown) { mv = sub(mv, [0.0, 1.0, 0.0]); }
            }
            if i.key_down(Key::ArrowUp) { mv = add(mv, f); }
            if i.key_down(Key::ArrowDown) { mv = sub(mv, f); }
            if i.key_down(Key::ArrowRight) { mv = add(mv, r); }
            if i.key_down(Key::ArrowLeft) { mv = sub(mv, r); }
        });
        if dot(mv, mv) > 0.0 {
            let speed = app.camera.speed * if fast { 4.0 } else { 1.0 };
            app.camera.pos = add(app.camera.pos, mul(norm(mv), speed * dt));
            ctx.request_repaint();
        }
    }
    // Stay above the ground.
    if let Some(d) = &app.doc {
        let g = d.world.height.sample(app.camera.pos[0], app.camera.pos[2]);
        if !app.camera_under { app.camera.pos[1] = app.camera.pos[1].max(g + 0.6); }
    }
}

fn focus_selection(app: &mut App) {
    let Some(p) = selection_pivot(app) else { return };
    let f = app.camera.forward();
    app.camera.pos = sub(p, mul(f, 40.0));
    app.top_center = [p[0], p[2]];
    app.top_mpp = app.top_mpp.min(0.5);
}

fn tool_input(app: &mut App, ctx: &egui::Context, resp: &egui::Response, v: &View) {
    // The merge of team work is writing the project; it is opened again afterwards.
    if app.team.merging() {
        return;
    }
    // While something is dragged onto the view egui does not count it as hovered, so the drop
    // takes the pointer as it is.
    let Some(mouse) = resp.hover_pos().or_else(|| resp.interact_pointer_pos()).or_else(|| resp.contains_pointer().then(|| ctx.pointer_latest_pos()).flatten()) else { return };
    let dt = ctx.input(|i| i.stable_dt).min(0.1);
    let Some(doc) = &app.doc else { return };
    let (o, d) = v.ray(mouse);
    let hit = pick_terrain(&doc.world.height, o, d, v.ortho);
    let modifiers = ctx.input(|i| i.modifiers);
    let primary_down = ctx.input(|i| i.pointer.primary_down());
    let pressed = resp.is_pointer_button_down_on() && ctx.input(|i| i.pointer.primary_pressed());

    // A satellite colour for a generator.
    if pressed && app.sampling.is_some() {
        if let Some(h) = hit {
            crate::library_ui::take_sample(app, h[0], h[2]);
        }
        return;
    }
    // A wicket or gate waiting to be placed on its fence.
    if app.pick_opening.is_some() {
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            app.pick_opening = None;
            app.status.clear();
            return;
        }
        if pressed {
            if let Some(h) = hit { crate::library_ui::place_opening(app, [h[0], h[2]]); }
            return;
        }
    }
    // Generators and presets dropped from their libraries.
    if let Some(p) = crate::theme::take::<crate::library_ui::LibDrag>(&resp) {
        if let Some(h) = hit {
            use crate::library_ui::LibDrag as L;
            match (*p).clone() {
                L::Generator(id) => {
                    let (shape, route) = drop_targets(app, v, mouse, hit);
                    crate::library_ui::add_instance(app, id, Some(crate::library_ui::Dropped { shape, route, at: [h[0], h[2]] }));
                }
                L::ForestPreset(name) => {
                    let (shape, route) = drop_targets(app, v, mouse, hit);
                    crate::library_ui::add_forest(app, &name, Some(crate::library_ui::Dropped { shape, route, at: [h[0], h[2]] }));
                }
                L::ObjectPreset(id) => crate::library_ui::place_preset(app, id, h[0], h[2]),
                L::Prefab(id) => { app.place_prefab(id, h[0], h[2]); }
            }
        }
        return;
    }
    // Several things dragged out of the content browser at once.
    if let Some(p) = crate::theme::take::<crate::asset_browser::Dragged>(&resp) {
        if let Some(h) = hit {
            crate::asset_browser::drop_many(app, &p.0, h[0], h[2]);
        }
        return;
    }
    // Models dropped from the content browser.
    if let Some(p) = crate::theme::take::<String>(&resp) {
        if let Some(h) = hit {
            let m = (*p).clone();
            place_model(app, &m, h[0], h[2]);
        }
        return;
    }

    // The modifiers held while using a brush, the same for every brush.
    let keys = &app.settings.keys;
    let (invert, smooth, pick) = (keys.held(Hold::Invert, modifiers), keys.held(Hold::Smooth, modifiers), keys.held(Hold::Pick, modifiers));
    match app.mode {
        Mode::Landscape if app.hole_tool => {
            if primary_down && resp.contains_pointer() {
                if let Some(h) = hit {
                    paint_holes(app, h[0], h[2], invert);
                    ctx.request_repaint();
                }
            } else if let Some(before) = app.hole_before.take() {
                if let Some(d) = &mut app.doc { d.history.push(t("Terrain holes"), Change::Holes(before)); }
            }
        }
        Mode::Satellite => {
            if pressed {
                if let Some(h) = hit {
                    crate::satellite::begin_stroke(app, h[0], h[2]);
                }
            }
            if !primary_down {
                crate::satellite::finish_stroke(app);
            } else if let Some(h) = hit {
                if crate::satellite::continue_stroke(app, h[0], h[2]) {
                    ctx.request_repaint();
                }
            }
        }
        Mode::Landscape | Mode::Paint => {
            // The eyedropper: the tool's button, or a click with the pick modifier while painting.
            let pick = pick && app.mode == Mode::Paint;
            if pressed && app.mode == Mode::Paint && (app.paint.picking || pick) {
                if let (Some(h), Some(d)) = (hit, &app.doc) {
                    app.paint.layer = d.world.mask.sample(h[0], h[2], d.world.meta.size_m()) as usize;
                }
                app.paint.picking = false;
                return;
            }
            if pressed && !pick {
                if let Some(h) = hit {
                    begin_paint_or_sculpt(app, h, invert, smooth);
                }
            }
            if !primary_down {
                finish_strokes(app);
            }
            if let Some(h) = hit {
                if let Some(mut s) = app.stroke.take() {
                    let r = s.apply(&mut app.doc.as_mut().unwrap().world, h[0], h[2], dt);
                    app.stroke = Some(s);
                    if let Some(r) = r {
                        app.height_changed(r);
                    }
                    ctx.request_repaint();
                }
                if let Some(mut s) = app.mask_stroke.take() {
                    s.dt = dt;
                    let d = app.doc.as_mut().unwrap();
                    let size = d.world.meta.size_m();
                    let r = s.apply(&mut d.world.mask, size, h[0], h[2], app.paint.radius, app.paint.layer as u8, app.paint.scatter);
                    app.mask_stroke = Some(s);
                    if let Some(r) = r {
                        app.mask_changed(r);
                    }
                    ctx.request_repaint();
                }
            }
        }
        Mode::Place if app.place.brush => {
            if primary_down && resp.contains_pointer() {
                if let Some(h) = hit {
                    brush_objects(app, h[0], h[2], invert);
                    ctx.request_repaint();
                }
            } else {
                finish_place_stroke(app);
            }
        }
        Mode::Place => {
            if resp.clicked() && !modifiers.alt {
                if let (Some(id), Some(h)) = (app.place.prefab, hit) {
                    app.place_prefab(id, h[0], h[2]);
                    return;
                }
                if let (Some(id), Some(h)) = (app.place.preset, hit) {
                    crate::library_ui::place_preset(app, id, h[0], h[2]);
                    return;
                }
                app.place_counter = app.place_counter.wrapping_add(1);
                let pick = pool_model(app, app.place_counter.wrapping_mul(2654435761));
                match (pick, hit) {
                    (Some(m), Some(h)) => {
                        place_model(app, &m, h[0], h[2]);
                    }
                    (None, _) => app.warn(t("Choose a model in the Content Browser first")),
                    _ => {}
                }
            }
        }
        Mode::Lines if app.line.action == LineAction::Edit => routes_input(app, ctx, resp, v, hit, mouse),
        Mode::Lines if app.line.action == LineAction::Road && app.line.draw == RoadDraw::Parts => {
            if resp.clicked() {
                if let Some(h) = hit {
                    crate::road_draw::parts_click(app, [h[0], h[2]]);
                }
            }
            let typing = ctx.wants_keyboard_input();
            let (enter, esc, back) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape), i.key_pressed(Key::Backspace)));
            if !typing {
                if back { crate::road_draw::parts_back(app); }
                if enter { crate::road_draw::parts_finish(app, app.line.grade); }
            }
            if esc {
                app.road_parts = crate::road_draw::PartsDraw { next: app.road_parts.next.take(), left: app.road_parts.left, ..Default::default() };
            }
        }
        Mode::Lines => {
            let live = app.line.action == LineAction::Road && app.line.draw == RoadDraw::Live;
            if live {
                let cursor = hit.map(|h| line_point(app, &app.line.points, [h[0], h[2]], modifiers));
                crate::road_draw::live_update(app, cursor, ctx);
            }
            if resp.clicked() {
                if let Some(h) = hit {
                    let p = line_point(app, &app.line.points, [h[0], h[2]], modifiers);
                    if live { crate::road_draw::live_click(app, p) } else { app.line.points.push(p) }
                }
            }
            let (enter, esc, back) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape), i.key_pressed(Key::Backspace)));
            if back && !ctx.wants_keyboard_input() {
                app.line.points.pop();
            }
            if esc {
                app.line.points.clear();
            }
            if enter && !ctx.wants_keyboard_input() {
                finish_line(app);
            }
        }
        Mode::Shapes => shapes_input(app, ctx, resp, v, hit, mouse),
        Mode::KeyPoints => {
            if app.shape.keypoint_adding {
                if resp.clicked() {
                    if let Some(h) = hit { app.add_keypoint(h[0], h[2]); }
                }
            } else {
                keypoint_input(app, ctx, resp, v, hit, mouse);
            }
        }
        Mode::Mission => crate::mission_ui::map_input(app, ctx, resp, v, hit, mouse),
        Mode::Generate => {}
        Mode::Select => {
            let pivot = selection_pivot(app);
            if pressed {
                if let Some(p) = pivot {
                    if let Some((handle, dir, axis, scale)) = gizmo_handle(app, v, p, mouse) {
                        let doc = app.doc.as_ref().unwrap();
                        let before: Vec<_> = app.selection.iter().filter_map(|id| doc.world.objects.get(*id).map(|o| (*id, *o))).collect();
                        let grab = hit.map(|h| [p[0] - h[0], p[2] - h[2]]).unwrap_or([0.0, 0.0]);
                        app.drag = Some(ObjectDrag { toggle: None, started: true, handle, before, pivot: p, start: mouse, axis: (dir, axis, scale), grab_offset: grab , virt: None, click: None, stepped: false, mods: egui::Modifiers::NONE });
                        return;
                    }
                }
                // Held keys make a drag anywhere turn or scale the selection about its middle:
                // Shift turns it, Ctrl+Shift turns it by steps, Alt scales it. A click without
                // moving still selects, Shift adding to the selection.
                let stepped = modifiers.shift && (modifiers.ctrl || modifiers.command);
                let turn = modifiers.shift && !(modifiers.ctrl || modifiers.command);
                let scale = modifiers.alt && !modifiers.shift && !(modifiers.ctrl || modifiers.command) && handle_allowed(app, Handle::Scale);
                if let (true, Some(p)) = (turn || stepped || scale, pivot) {
                    let doc = app.doc.as_ref().unwrap();
                    let before: Vec<_> = app.selection.iter().filter_map(|id| doc.world.objects.get(*id).map(|o| (*id, *o))).collect();
                    let handle = if scale { Handle::Scale } else { Handle::Spin };
                    app.drag = Some(ObjectDrag { toggle: None, started: false, handle, before, pivot: p, start: mouse, axis: (vec2(0.0, 0.0), [0.0; 3], 0.0), grab_offset: [0.0, 0.0], virt: None, click: Some((mouse, modifiers.shift || modifiers.ctrl)), stepped, mods: modifiers });
                    return;
                }
                // Held by its shape, an object slides over the ground; one not yet selected is
                // picked first, as a click would.
                if app.settings.drag_by_body && !modifiers.shift && !modifiers.ctrl {
                    if let (Some(id), Some(h)) = (pick_object(app, v, mouse), hit) {
                        if !app.selection.contains(&id) {
                            finish_selection(app, v, mouse, mouse, false);
                        }
                        if let (true, Some(p)) = (app.selection.contains(&id), selection_pivot(app)) {
                            let doc = app.doc.as_ref().unwrap();
                            let before: Vec<_> = app.selection.iter().filter_map(|id| doc.world.objects.get(*id).map(|o| (*id, *o))).collect();
                            app.drag = Some(ObjectDrag { toggle: None, started: false, handle: Handle::Plane, before, pivot: p, start: mouse, axis: (vec2(0.0, 0.0), [0.0; 3], 0.0), grab_offset: [p[0] - h[0], p[2] - h[2]] , virt: None, click: None, stepped: false, mods: egui::Modifiers::NONE });
                            return;
                        }
                    }
                }
                // Held on a selected object, Ctrl scales it.
                if modifiers.ctrl && !modifiers.shift && handle_allowed(app, Handle::Scale) {
                    if let (Some(p), Some(id)) = (pivot, pick_object(app, v, mouse).filter(|id| app.selection.contains(id))) {
                        let doc = app.doc.as_ref().unwrap();
                        let before: Vec<_> = app.selection.iter().filter_map(|id| doc.world.objects.get(*id).map(|o| (*id, *o))).collect();
                        let handle = if modifiers.shift { Handle::Spin } else { Handle::Scale };
                        app.drag = Some(ObjectDrag { toggle: Some(id), started: false, handle, before, pivot: p, start: mouse, axis: (vec2(0.0, 0.0), [0.0; 3], 0.0), grab_offset: [0.0, 0.0] , virt: None, click: None, stepped: false, mods: egui::Modifiers::NONE });
                        return;
                    }
                }
                app.box_select = Some((mouse, mouse, modifiers.shift || modifiers.ctrl));
                app.lasso_points = if app.settings.lasso { vec![mouse] } else { Vec::new() };
            }
            if app.drag.as_ref().is_some_and(|d| d.stepped) {
                let wheel = ctx.input(|i| i.raw_scroll_delta.y + i.raw_scroll_delta.x);
                if wheel != 0.0 {
                    const STEPS: [f32; 9] = [1.0, 2.5, 5.0, 10.0, 15.0, 22.5, 30.0, 45.0, 90.0];
                    let now = STEPS.iter().position(|s| *s >= app.settings.angle_step - 0.01).unwrap_or(4);
                    let next = if wheel > 0.0 { (now + 1).min(STEPS.len() - 1) } else { now.saturating_sub(1) };
                    app.settings.angle_step = STEPS[next];
                }
            }
            if app.drag.is_some() {
                if primary_down {
                    let snap = modifiers.ctrl && app.drag.as_ref().is_some_and(|d| d.toggle.is_none());
                    apply_object_drag(app, v, mouse, snap);
                    ctx.request_repaint();
                } else if let Some((at, add)) = end_object_drag(app) {
                    finish_selection(app, v, at, at, add);
                }
                return;
            }
            if let Some(sel) = &mut app.box_select {
                sel.1 = mouse;
                if app.settings.lasso && app.lasso_points.last().is_none_or(|p| (*p - mouse).length() > 3.0) {
                    app.lasso_points.push(mouse);
                }
                if !primary_down {
                    let (a, b, add_mode) = app.box_select.take().unwrap();
                    let outline = std::mem::take(&mut app.lasso_points);
                    if outline.len() >= 3 {
                        let r = outline.iter().fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(*p, *p)));
                        finish_selection_in(app, v, r.min, r.max, add_mode, Some(&outline));
                    } else {
                        finish_selection(app, v, a, b, add_mode);
                    }
                }
                ctx.request_repaint();
            }
        }
    }
}

/// The road or generator under a click, with all its objects.
fn owner_of(world: &dz_core::world::World, id: u32) -> Option<(SelectedWhole, Vec<u32>)> {
    if let Some(g) = world.generators.iter().find(|g| g.objects.contains(&id)) {
        return Some((SelectedWhole::Generator(g.id), g.objects.clone()));
    }
    world.routes.iter().find(|r| r.objects.contains(&id)).map(|r| (SelectedWhole::Route(r.id), r.objects.clone()))
}

fn whole_at(app: &App, v: &View, m: Pos2, picked: Option<u32>) -> Option<(SelectedWhole, Vec<u32>)> {
    let world = &app.doc.as_ref()?.world;
    if let Some(id) = picked {
        return owner_of(world, id);
    }
    // Road surfaces lie flat and are hard to hit, so their line counts too.
    if app.show_lines {
        if let RouteHit::Route(i) = route_hit(world, v, None, m) {
            let r = &world.routes[i];
            return Some((SelectedWhole::Route(r.id), r.objects.clone()));
        }
    }
    None
}

/// True when `p` lies inside the closed outline.
fn inside_outline(outline: &[Pos2], p: Pos2) -> bool {
    let mut inside = false;
    let mut j = outline.len() - 1;
    for i in 0..outline.len() {
        let (a, b) = (outline[i], outline[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn finish_selection(app: &mut App, v: &View, a: Pos2, b: Pos2, add_mode: bool) {
    finish_selection_in(app, v, a, b, add_mode, None);
}

/// Selects what is in the box from `a` to `b`, or within `outline` when one is drawn.
fn finish_selection_in(app: &mut App, v: &View, a: Pos2, b: Pos2, add_mode: bool, outline: Option<&[Pos2]>) {
    app.leave_browser();
    if !add_mode {
        app.selection.clear();
        app.selected_modifier = None;
        // Whatever the outliner had chosen gives way to what is picked here.
        app.selected_generator = None;
        app.whole = None;
    }
    let r = Rect::from_two_pos(a, b);
    if r.width() < 4.0 && r.height() < 4.0 {
        if !add_mode {
            app.shape.selection.clear();
            app.shape.vertex = None;
        }
        let picked = pick_object(app, v, b);
        let whole = if app.select_whole && !add_mode { whole_at(app, v, b, picked) } else { None };
        if let Some((target, ids)) = whole {
            app.selection.extend(ids);
            app.whole = Some(target);
            app.selected_generator = if let SelectedWhole::Generator(id) = target { Some(id) } else { None };
        } else if let Some(id) = picked {
            app.selected_generator = None;
            app.selected_modifier = None;
            if add_mode && app.selection.contains(&id) {
                app.selection.remove(&id);
            } else {
                app.selection.insert(id);
            }
        } else if app.show_shapes {
            // Nothing solid under the cursor: shapes can be picked from this tool as well.
            if let Some(doc) = &app.doc {
                let (o, d) = v.ray(b);
                let hit = pick_terrain(&doc.world.height, o, d, v.ortho);
                if let Some(s) = pick_shape(&doc.world, v, b, hit) {
                    app.shape.selection.insert(s);
                }
            }
        }
    } else if let Some(doc) = &app.doc {
        let world = &doc.world;
        let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let corners = [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()];
        for c in corners {
            let (o, d) = v.ray(c);
            let p = pick_terrain(&world.height, o, d, v.ortho).unwrap_or_else(|| add(o, mul(d, 800.0)));
            x0 = x0.min(p[0]);
            z0 = z0.min(p[2]);
            x1 = x1.max(p[0]);
            z1 = z1.max(p[2]);
        }
        // Rays that meet no ground end far away, so what is near the camera counts too.
        if !v.ortho {
            (x0, z0, x1, z1) = (x0.min(v.eye[0]), z0.min(v.eye[2]), x1.max(v.eye[0]), z1.max(v.eye[2]));
        }
        let mut found = Vec::new();
        world.objects.for_each_in(x0, z0, x1, z1, |id, o| {
            if !world.object_interactive(o) {
                return;
            }
            if let Some(s) = v.project([o.x, ObjectStore::origin_y(o, &world.height), o.z]) {
                if r.contains(s) && outline.is_none_or(|l| inside_outline(l, s)) {
                    found.push(id);
                }
            }
        });
        app.selection.extend(found);
    }
    app.selection_rev += 1;
}

fn draw_gizmo(app: &App, painter: &egui::Painter, v: &View, p: V3, hover: Option<Handle>) {
    let Some(c) = v.project(p) else { return };
    let size = gizmo_size(v, p);
    let color = |h: Handle, base: Color32| if hover == Some(h) { Color32::YELLOW } else { base };
    match app.gizmo {
        Gizmo::Move => {
            let g = gizmo_axes(app);
            for (h, a, col) in [(Handle::X, g[0], Color32::from_rgb(230, 60, 60)), (Handle::Y, g[1], Color32::from_rgb(70, 200, 70)), (Handle::Z, g[2], Color32::from_rgb(70, 110, 240))] {
                if !handle_allowed(app, h) || v.project(add(p, mul(a, size))).is_none_or(|e| (e - c).length() < 12.0) {
                    continue;
                }
                if let Some(e) = v.project(add(p, mul(a, size))) {
                    let col = color(h, col);
                    painter.line_segment([c, e], Stroke::new(2.5_f32, col));
                    painter.circle_filled(e, 5.0, col);
                }
            }
            painter.circle_filled(c, 7.0, color(Handle::Plane, Color32::from_rgb(230, 230, 80)));
        }
        Gizmo::Rotate => {
            let g = gizmo_axes(app);
            for (k, h, col) in [(0, Handle::RotX, Color32::from_rgb(230, 60, 60)), (1, Handle::RotY, Color32::from_rgb(70, 200, 70)), (2, Handle::RotZ, Color32::from_rgb(70, 110, 240))] {
                if v.ortho && !app.gizmo_local && k != 1 {
                    continue;
                }
                let pts: Vec<Pos2> = ring_points(p, &g, k, size * 0.85).into_iter().filter_map(|q| v.project(q)).collect();
                painter.add(egui::Shape::line(pts, Stroke::new(if hover == Some(h) { 3.5_f32 } else { 2.5 }, color(h, col))));
            }
            // The angle while turning.
            if let Some(d) = app.drag.as_ref().filter(|d| matches!(d.handle, Handle::RotX | Handle::RotY | Handle::RotZ)) {
                if let (Some(b), Some(o)) = (d.before.first(), app.selection.iter().next().and_then(|id| app.doc.as_ref()?.world.objects.get(*id))) {
                    let text = format!("Y {:.1}°  P {:.1}°  R {:.1}°", o.yaw - b.1.yaw, o.pitch - b.1.pitch, o.roll - b.1.roll);
                    painter.text(c + vec2(0.0, -size.min(400.0) * 0.0 - 24.0), egui::Align2::CENTER_BOTTOM, text, egui::FontId::proportional(12.0), Color32::WHITE);
                }
            }
        }
        Gizmo::Scale if !handle_allowed(app, Handle::Scale) => {}
        Gizmo::Scale => {
            painter.rect_filled(Rect::from_center_size(c, vec2(14.0, 14.0)), 1.0, color(Handle::Scale, Color32::from_rgb(90, 170, 240)));
            painter.circle_stroke(c, 60.0, Stroke::new(1.0_f32, Color32::from_gray(150)));
        }
    }
}

const SHAPE_COLORS: [Color32; 6] = [Color32::from_rgb(120, 220, 120), Color32::from_rgb(110, 200, 255), Color32::from_rgb(255, 150, 200), Color32::from_rgb(255, 205, 90), Color32::from_rgb(190, 160, 255), Color32::from_rgb(120, 235, 215)];

fn shape_screen(world: &dz_core::world::World, v: &View, p: [f32; 2]) -> Option<Pos2> {
    v.project([p[0], world.height.sample(p[0], p[1]) + 0.3, p[1]])
}

fn near_segment(m: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((m - a).dot(ab) / ab.length_sq().max(1e-3)).clamp(0.0, 1.0);
    (a + ab * t - m).length()
}

/// Segments of a shape as index pairs, closing polygons.
fn shape_segments(s: &dz_core::shapes::MapShape) -> impl Iterator<Item = (usize, usize)> + '_ {
    let n = s.points.len();
    let closed = s.kind == dz_core::shapes::ShapeKind::Polygon;
    (0..n.saturating_sub(1)).map(|i| (i, i + 1)).chain((closed && n > 2).then_some((n - 1, 0)))
}

fn pick_shape(world: &dz_core::world::World, v: &View, m: Pos2, hit: Option<V3>) -> Option<(usize, usize)> {
    use dz_core::shapes::{point_in_polygon, ShapeKind};
    let mut inside = None;
    for (li, l) in world.shape_layers.iter().enumerate().rev().filter(|(_, l)| l.visible) {
        for (si, s) in l.shapes.iter().enumerate().rev() {
            let near = if s.kind == ShapeKind::Point {
                shape_screen(world, v, s.points[0]).is_some_and(|p| (p - m).length() < 9.0)
            } else {
                shape_segments(s).any(|(a, b)| match (shape_screen(world, v, s.points[a]), shape_screen(world, v, s.points[b])) {
                    (Some(a), Some(b)) => near_segment(m, a, b) < 7.0,
                    _ => false,
                })
            };
            if near {
                return Some((li, si));
            }
            if inside.is_none() && s.kind == ShapeKind::Polygon && hit.is_some_and(|h| point_in_polygon(&s.points, h[0], h[2])) {
                inside = Some((li, si));
            }
        }
    }
    inside
}

/// The shape and the route under the cursor, for what is dropped on the map.
fn drop_targets(app: &App, v: &View, mouse: Pos2, hit: Option<V3>) -> (Option<(usize, usize)>, Option<u64>) {
    let Some(doc) = &app.doc else { return (None, None) };
    let route = match route_hit(&doc.world, v, None, mouse) {
        RouteHit::Route(i) => doc.world.routes.get(i).map(|r| r.id),
        _ => None,
    };
    (pick_shape(&doc.world, v, mouse, hit), route)
}

/// Picks a waypoint, a "+" handle between two waypoints, or a route.
enum RouteHit {
    Point(usize),
    Between(usize, [f32; 2]),
    Route(usize),
    Nothing,
}

fn route_hit(world: &dz_core::world::World, v: &View, selected: Option<usize>, m: Pos2) -> RouteHit {
    if let Some(r) = selected.and_then(|i| world.routes.get(i)) {
        let screen: Vec<Option<Pos2>> = r.points.iter().map(|p| shape_screen(world, v, *p)).collect();
        if let Some(i) = screen.iter().enumerate().filter_map(|(i, s)| Some((i, ((*s)? - m).length()))).filter(|(_, d)| *d < 11.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i) {
            return RouteHit::Point(i);
        }
        for i in 0..r.points.len().saturating_sub(1) {
            let mid = [(r.points[i][0] + r.points[i + 1][0]) * 0.5, (r.points[i][1] + r.points[i + 1][1]) * 0.5];
            if shape_screen(world, v, mid).is_some_and(|s| (s - m).length() < 9.0) {
                return RouteHit::Between(i, mid);
            }
        }
    }
    for (i, r) in world.routes.iter().enumerate() {
        for w in r.centerline.windows(2) {
            if let (Some(a), Some(b)) = (shape_screen(world, v, w[0]), shape_screen(world, v, w[1])) {
                if near_segment(m, a, b) < 8.0 {
                    return RouteHit::Route(i);
                }
            }
        }
    }
    RouteHit::Nothing
}

fn routes_input(app: &mut App, ctx: &egui::Context, resp: &egui::Response, v: &View, hit: Option<V3>, mouse: Pos2) {
    let (del, esc) = ctx.input(|i| (i.key_pressed(Key::Delete), i.key_pressed(Key::Escape)));
    let typing = ctx.wants_keyboard_input();
    let primary_down = ctx.input(|i| i.pointer.primary_down());
    if app.line.point_drag.is_some() {
        let (route, point) = (app.line.selected_route, app.line.selected_point);
        if primary_down {
            if let (Some(h), Some(r), Some(p)) = (hit, route, point) {
                if let Some(q) = app.doc.as_mut().unwrap().world.routes.get_mut(r).and_then(|x| x.points.get_mut(p)) {
                    *q = [h[0], h[2]];
                }
                if app.doc.as_ref().and_then(|d| d.world.routes.get(r)).is_some_and(|x| x.method == RoadDraw::Live) {
                    crate::road_draw::live_route(app, r, ctx);
                }
            }
            ctx.request_repaint();
        } else if let (Some(before), Some(r)) = (app.line.point_drag.take(), route) {
            app.live.shown = None;
            app.refit_route(r, before);
        }
        return;
    }
    // A route laid piece by piece is changed piece by piece, from the panel and the keys.
    let pieces = app.line.selected_route.and_then(|i| app.doc.as_ref()?.world.routes.get(i)).is_some_and(|r| r.method == RoadDraw::Parts);
    if pieces && app.road_parts.target.is_some() && !ctx.wants_keyboard_input() {
        let (enter, esc, back) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape), i.key_pressed(Key::Backspace)));
        if back { crate::road_draw::parts_back(app); }
        if enter && app.task.is_none() { crate::road_draw::parts_finish(app, false); }
        if esc {
            // Taken again from the route by the panel.
            app.road_parts.stamp = 0;
            return;
        }
    }
    let Some(doc) = &app.doc else { return };
    let world = &doc.world;
    let selected = app.line.selected_route.filter(|i| *i < world.routes.len());
    if selected.is_none() {
        app.line.selected_point = None;
    }
    let busy = app.task.is_some();
    // Its waypoints have no handles then.
    let handles = if pieces { None } else { selected };
    if resp.drag_started_by(egui::PointerButton::Primary) {
        let origin = ctx.input(|i| i.pointer.press_origin()).unwrap_or(mouse);
        if let (RouteHit::Point(i), false, Some(r)) = (route_hit(world, v, handles, origin), busy, handles) {
            app.line.selected_point = Some(i);
            app.line.point_drag = Some(world.routes[r].points.clone());
            remember_fit(app, r);
        }
        return;
    }
    let ctrl = ctx.input(|i| i.modifiers.command);
    // A click on a point or on a + is that, also while extending.
    let on_handle = matches!(route_hit(world, v, handles, mouse), RouteHit::Point(_) | RouteHit::Between(..));
    let extending = handles.is_some() && !on_handle && (ctrl || app.line.extend.is_some());
    // A route edited live is fitted again as soon as its points change, as a new one is drawn.
    let live = selected.and_then(|r| world.routes.get(r)).is_some_and(|x| x.method == RoadDraw::Live);
    // Where a click would add a point: the route's points with it, and whether at the start.
    let extended = |at: [f32; 2]| -> Option<(Vec<[f32; 2]>, bool)> {
        let points = &world.routes.get(selected?)?.points;
        let distance = |p: [f32; 2]| (p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2);
        let start = app.line.extend.unwrap_or_else(|| distance(points[0]) < distance(*points.last().unwrap()));
        let mut list = points.clone();
        if start { list.insert(0, at) } else { list.push(at) }
        Some((list, start))
    };
    let candidate = if extending { hit.and_then(|h| extended([h[0], h[2]])) } else { None };
    if live && extending && !busy {
        if let (Some(r), Some((list, _))) = (selected, candidate.clone()) {
            crate::road_draw::live_route_with(app, r, list, ctx);
        }
    } else if live && app.live.shown.is_some() {
        app.live.shown = None;
    }
    let Some(doc) = &app.doc else { return };
    let world = &doc.world;
    if resp.clicked() && extending {
        // New road beyond either end; the models follow on "Fit again", or at once when live.
        if let (Some(r), Some((list, start))) = (selected, candidate) {
            if live {
                if let Err(e) = crate::road_draw::live_fits(app, &list) {
                    app.warn(format!("{}: {e}", t("The road cannot reach this point")));
                    return;
                }
            }
            let previous = world.routes.clone();
            let before = world.routes[r].points.clone();
            remember_fit(app, r);
            let doc = app.doc.as_mut().unwrap();
            app.line.selected_point = Some(if start { 0 } else { list.len() - 1 });
            doc.world.routes[r].points = list;
            doc.history.push(t("Edit route"), Change::Routes(previous));
            if live && !busy { app.refit_route(r, before); }
        }
        return;
    }
    if resp.clicked() {
        match route_hit(world, v, handles, mouse) {
            RouteHit::Point(i) => app.line.selected_point = Some(i),
            RouteHit::Between(i, at) if !busy => {
                // A new point waits for "Fit again" or for being dragged.
                let r = selected.unwrap();
                let previous = world.routes.clone();
                let before = world.routes[r].points.clone();
                remember_fit(app, r);
                let doc = app.doc.as_mut().unwrap();
                doc.world.routes[r].points.insert(i + 1, at);
                doc.history.push(t("Edit route"), Change::Routes(previous));
                app.line.selected_point = Some(i + 1);
                if live { app.refit_route(r, before); }
            }
            RouteHit::Between(..) => {}
            RouteHit::Route(i) => {
                app.line.selected_route = Some(i);
                app.line.selected_point = None;
            }
            RouteHit::Nothing => {
                app.line.selected_route = None;
                app.line.selected_point = None;
            }
        }
        return;
    }
    if del && !typing && !busy {
        if let (Some(r), Some(p)) = (selected, app.line.selected_point) {
            let points = &world.routes[r].points;
            if points.len() > 2 && p < points.len() && !pieces {
                let before = points.clone();
                remember_fit(app, r);
                app.doc.as_mut().unwrap().world.routes[r].points.remove(p);
                app.line.selected_point = None;
                app.refit_route(r, before);
            }
        }
    }
    if esc {
        if app.line.extend.take().is_none() {
            app.line.selected_route = None;
            app.line.selected_point = None;
        }
    }
}

/// Notes the waypoints of a route's fit before they are changed, for a route that has no
/// record of it (made before routes kept one, or brought in): only then does a point added
/// past an end tell apart from one added in between.
fn remember_fit(app: &mut App, r: usize) {
    if let Some(route) = app.doc.as_mut().and_then(|d| d.world.routes.get_mut(r)) {
        if route.fitted.len() < 2 { route.fitted = route.points.clone(); }
    }
}

/// A grip on the selected shapes.
#[derive(Clone, Copy, PartialEq)]
enum ShapeHandle {
    Vertex(usize),
    /// Adds a point after this one.
    Edge(usize),
    Move,
}

fn selection_center(world: &dz_core::world::World, selection: &std::collections::BTreeSet<(usize, usize)>) -> Option<[f32; 2]> {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for (l, s) in selection {
        for p in &world.shape_layers.get(*l)?.shapes.get(*s)?.points {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        }
    }
    (b[0] <= b[2]).then(|| [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5])
}

/// (point after which one is added, its map position, on screen) for edges long enough to have a grip.
fn edge_grips(world: &dz_core::world::World, v: &View, s: &dz_core::shapes::MapShape) -> Vec<(usize, [f32; 2], Pos2)> {
    shape_segments(s).filter_map(|(a, b)| {
        let (pa, pb) = (shape_screen(world, v, s.points[a])?, shape_screen(world, v, s.points[b])?);
        if (pa - pb).length() < 44.0 {
            return None;
        }
        let at = [(s.points[a][0] + s.points[b][0]) * 0.5, (s.points[a][1] + s.points[b][1]) * 0.5];
        Some((a, at, shape_screen(world, v, at)?))
    }).collect()
}

fn shape_handle(world: &dz_core::world::World, v: &View, selection: &std::collections::BTreeSet<(usize, usize)>, m: Pos2) -> Option<ShapeHandle> {
    let single = (selection.len() == 1).then(|| *selection.iter().next().unwrap());
    let shape = single.and_then(|(l, s)| world.shape_layers.get(l)?.shapes.get(s));
    if let Some(s) = shape {
        let near = s.points.iter().enumerate().filter_map(|(i, p)| Some((i, (shape_screen(world, v, *p)? - m).length()))).filter(|(_, d)| *d < 10.0).min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = near {
            return Some(ShapeHandle::Vertex(i));
        }
    }
    if selection_center(world, selection).and_then(|c| shape_screen(world, v, c)).is_some_and(|c| (c - m).length() < 11.0) {
        return Some(ShapeHandle::Move);
    }
    let s = shape?;
    edge_grips(world, v, s).into_iter().find(|g| (g.2 - m).length() < 9.0).map(|g| ShapeHandle::Edge(g.0))
}

/// Picks, drags and deletes key points. True when the input was theirs.
fn keypoint_input(app: &mut App, ctx: &egui::Context, resp: &egui::Response, v: &View, hit: Option<V3>, mouse: Pos2) -> bool {
    let Some(doc) = &mut app.doc else { return false };
    let (pressed, down, del, esc) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.key_pressed(Key::Delete), i.key_pressed(Key::Escape)));
    if app.shape.keypoint_moving {
        let i = app.shape.keypoint.unwrap_or(usize::MAX);
        if esc {
            if let Some(before) = app.shape.keypoints_before.take() { doc.world.keypoints = before; }
            app.shape.keypoint_moving = false;
            return true;
        }
        if down {
            if let (Some(h), Some(k)) = (hit, doc.world.keypoints.get_mut(i)) { (k.x, k.z) = (h[0], h[2]); }
            return true;
        }
        if let Some(before) = app.shape.keypoints_before.take() {
            if before != doc.world.keypoints { doc.history.push(t("Move key point"), Change::KeyPoints(before)); }
        }
        app.shape.keypoint_moving = false;
        return true;
    }
    if del && !ctx.wants_keyboard_input() {
        if let Some(i) = app.shape.keypoint {
            app.delete_keypoint(i);
            return true;
        }
    }
    if pressed && resp.hovered() {
        let world = &doc.world;
        let near = world.keypoints.iter().enumerate().filter_map(|(i, k)| {
            let s = v.project([k.x, world.height.sample(k.x, k.z) + 1.0, k.z])?;
            let d = s.distance(mouse);
            (d < 12.0).then_some((i, d))
        }).min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = near {
            app.shape.keypoint = Some(i);
            app.shape.selection.clear();
            app.shape.keypoints_before = Some(world.keypoints.clone());
            app.shape.keypoint_moving = true;
            return true;
        }
        app.shape.keypoint = None;
    }
    false
}

fn shapes_input(app: &mut App, ctx: &egui::Context, resp: &egui::Response, v: &View, hit: Option<V3>, mouse: Pos2) {
    let typing = ctx.wants_keyboard_input();
    let (enter, esc, back, del, modifiers) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape), i.key_pressed(Key::Backspace), i.key_pressed(Key::Delete), i.modifiers));
    if app.shape.tool != ShapeTool::Edit {
        if resp.double_clicked() && app.shape.tool != ShapeTool::Point {
            app.finish_shape();
            return;
        }
        if resp.clicked() {
            if let Some(h) = hit {
                let p = line_point(app, &app.shape.points, [h[0], h[2]], modifiers);
                app.shape.points.push(p);
                if app.shape.tool == ShapeTool::Point {
                    app.finish_shape();
                }
            }
        }
        if !typing {
            if back { app.shape.points.pop(); }
            if enter { app.finish_shape(); }
        }
        if esc { app.shape.points.clear(); }
        return;
    }
    let primary_down = ctx.input(|i| i.pointer.primary_down());
    // A drag in progress, whichever kind.
    if app.shape.drag_before.is_some() {
        if esc {
            // Puts everything back; nothing reaches the undo list.
            if let Some(before) = app.shape.drag_before.take() { app.doc.as_mut().unwrap().world.shape_layers = before; }
            app.shape.move_from = None;
            return;
        }
        if primary_down {
            if let Some(h) = hit {
                let doc = app.doc.as_mut().unwrap();
                match app.shape.move_from {
                    Some(from) => {
                        let delta = [h[0] - from[0], h[2] - from[1]];
                        for (l, s) in &app.shape.selection {
                            if let Some(shape) = doc.world.shape_layers.get_mut(*l).and_then(|x| x.shapes.get_mut(*s)) {
                                for p in &mut shape.points {
                                    p[0] += delta[0];
                                    p[1] += delta[1];
                                }
                            }
                        }
                        app.shape.move_from = Some([h[0], h[2]]);
                    }
                    None => {
                        if let (Some((l, s)), Some(i)) = (app.shape.selection.iter().next().copied(), app.shape.vertex) {
                            if let Some(p) = doc.world.shape_layers.get_mut(l).and_then(|x| x.shapes.get_mut(s)).and_then(|x| x.points.get_mut(i)) {
                                *p = [h[0], h[2]];
                            }
                        }
                    }
                }
            }
            ctx.request_repaint();
        } else if let Some(before) = app.shape.drag_before.take() {
            app.shape.move_from = None;
            app.doc.as_mut().unwrap().history.push(t("Edit shape"), Change::Shapes(before));
        }
        return;
    }
    if app.shape.boxing.is_some() && !primary_down {
        app.leave_browser();
    }
    let Some(doc) = &app.doc else { return };
    let world = &doc.world;
    if let Some((_, end, _)) = &mut app.shape.boxing {
        *end = mouse;
        if !primary_down {
            let (a, b, add) = app.shape.boxing.take().unwrap();
            let r = Rect::from_two_pos(a, b);
            if !add {
                app.shape.selection.clear();
            }
            app.shape.vertex = None;
            for (li, l) in world.shape_layers.iter().enumerate().filter(|(_, l)| l.visible) {
                for (si, s) in l.shapes.iter().enumerate() {
                    if s.points.iter().any(|p| shape_screen(world, v, *p).is_some_and(|q| r.contains(q))) {
                        app.shape.selection.insert((li, si));
                    }
                }
            }
        }
        ctx.request_repaint();
        return;
    }
    let single = (app.shape.selection.len() == 1).then(|| *app.shape.selection.iter().next().unwrap());
    let shape = single.and_then(|(l, s)| world.shape_layers.get(l)?.shapes.get(s));
    let vertex_near = |m: Pos2| shape.and_then(|s| {
        s.points.iter().enumerate().filter_map(|(i, p)| Some((i, (shape_screen(world, v, *p)? - m).length()))).filter(|(_, d)| *d < 10.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    });
    if let Some(h) = resp.hover_pos().and_then(|m| shape_handle(world, v, &app.shape.selection, m)) {
        ctx.set_cursor_icon(if h == ShapeHandle::Move { egui::CursorIcon::Move } else { egui::CursorIcon::Grab });
    }
    if resp.drag_started_by(egui::PointerButton::Primary) {
        // Only the grips move anything; a drag anywhere else draws a selection box.
        let origin = ctx.input(|i| i.pointer.press_origin()).unwrap_or(mouse);
        let (ro, rd) = v.ray(origin);
        let ground = pick_terrain(&world.height, ro, rd, v.ortho);
        match (shape_handle(world, v, &app.shape.selection, origin), single) {
            (Some(ShapeHandle::Vertex(i)), _) => {
                app.shape.vertex = Some(i);
                app.shape.move_from = None;
                app.shape.drag_before = Some(world.shape_layers.clone());
            }
            (Some(ShapeHandle::Edge(a)), Some((l, s))) => {
                let before = world.shape_layers.clone();
                let at = edge_grips(world, v, &world.shape_layers[l].shapes[s]).into_iter().find(|g| g.0 == a).map(|g| g.1);
                if let Some(at) = at {
                    app.doc.as_mut().unwrap().world.shape_layers[l].shapes[s].points.insert(a + 1, at);
                    app.shape.vertex = Some(a + 1);
                    app.shape.move_from = None;
                    app.shape.drag_before = Some(before);
                }
            }
            (Some(ShapeHandle::Move), _) => {
                if let Some(g) = ground {
                    app.shape.vertex = None;
                    app.shape.move_from = Some([g[0], g[2]]);
                    app.shape.drag_before = Some(world.shape_layers.clone());
                }
            }
            _ => app.shape.boxing = Some((origin, mouse, modifiers.shift)),
        }
        return;
    }
    if resp.secondary_clicked() {
        if let Some(ShapeHandle::Vertex(i)) = shape_handle(world, v, &app.shape.selection, mouse) {
            app.shape.vertex = Some(i);
            app.delete_shape_part();
            return;
        }
    }
    if resp.clicked() {
        if modifiers.command {
            // Ctrl+click on an edge of the selected shape adds a vertex there.
            let at = shape.and_then(|s| shape_segments(s).find(|(a, b)| match (shape_screen(world, v, s.points[*a]), shape_screen(world, v, s.points[*b])) {
                (Some(a), Some(b)) => near_segment(mouse, a, b) < 8.0,
                _ => false,
            }));
            if let (Some((a, _)), Some(h), Some((l, s))) = (at, hit, single) {
                let before = world.shape_layers.clone();
                let doc = app.doc.as_mut().unwrap();
                doc.world.shape_layers[l].shapes[s].points.insert(a + 1, [h[0], h[2]]);
                doc.history.push(t("Edit shape"), Change::Shapes(before));
                app.shape.vertex = Some(a + 1);
            }
        } else if let Some(i) = vertex_near(mouse) {
            app.shape.vertex = Some(i);
        } else if let (Some(ShapeHandle::Edge(a)), Some((l, s))) = (shape_handle(world, v, &app.shape.selection, mouse), single) {
            let before = world.shape_layers.clone();
            if let Some(g) = edge_grips(world, v, &world.shape_layers[l].shapes[s]).into_iter().find(|g| g.0 == a) {
                let doc = app.doc.as_mut().unwrap();
                doc.world.shape_layers[l].shapes[s].points.insert(a + 1, g.1);
                doc.history.push(t("Edit shape"), Change::Shapes(before));
                app.shape.vertex = Some(a + 1);
            }
        } else if shape_handle(world, v, &app.shape.selection, mouse) == Some(ShapeHandle::Move) {
            // The square only moves things when dragged.
        } else {
            let picked = pick_shape(world, v, mouse, hit);
            app.leave_browser();
            app.shape.vertex = None;
            if modifiers.shift {
                if let Some(p) = picked {
                    if !app.shape.selection.remove(&p) {
                        app.shape.selection.insert(p);
                    }
                }
            } else {
                app.shape.selection = picked.into_iter().collect();
            }
        }
    }
    if del && !typing {
        app.delete_shape_part();
    }
    if esc {
        app.shape.selection.clear();
        app.shape.vertex = None;
    }
}

pub(crate) fn draw_polyline(painter: &egui::Painter, v: &View, world: &dz_core::world::World, points: &[[f32; 2]], stroke: Stroke) {
    let mut prev: Option<Pos2> = None;
    for p in points {
        let q = v.project([p[0], world.height.sample(p[0], p[1]) + 0.3, p[1]]);
        if let (Some(a), Some(b)) = (prev, q) {
            if v.rect.expand(200.0).contains(a) || v.rect.expand(200.0).contains(b) {
                painter.line_segment([a, b], stroke);
            }
        }
        prev = q;
    }
}

fn draw_overlays(app: &App, ui: &egui::Ui, v: &View, resp: &egui::Response) {
    let painter = ui.painter_at(v.rect);
    let Some(doc) = &app.doc else { return };
    let world = &doc.world;
    if app.show_lines {
        for (i, r) in world.routes.iter().enumerate() {
            let selected = app.line.selected_route == Some(i);
            let col = if selected { Color32::from_rgb(255, 170, 40) } else if dayz_road_tool::rails::is_track_family(&r.family) { Color32::from_rgb(200, 150, 255) } else { Color32::from_rgb(120, 230, 150) };
            draw_polyline(&painter, v, world, &r.centerline, Stroke::new(if selected { 2.6_f32 } else { 1.6 }, col));
        }
    }
    if app.mode == Mode::Lines && app.line.action == LineAction::Edit {
        if let Some(r) = app.line.selected_route.and_then(|i| world.routes.get(i)).filter(|r| r.method != RoadDraw::Parts) {
            let col = Color32::from_rgb(255, 170, 40);
            draw_polyline(&painter, v, world, &r.points, Stroke::new(1.0_f32, col));
            for (i, p) in r.points.iter().enumerate() {
                if let Some(c) = shape_screen(world, v, *p) {
                    let marked = app.line.selected_point == Some(i);
                    painter.circle_filled(c, if marked { 7.0 } else { 5.0 }, if marked { Color32::WHITE } else { col });
                }
            }
            for w in r.points.windows(2) {
                if let Some(c) = shape_screen(world, v, [(w[0][0] + w[1][0]) * 0.5, (w[0][1] + w[1][1]) * 0.5]) {
                    painter.circle_stroke(c, 6.0, Stroke::new(1.0_f32, col));
                    painter.text(c, egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(12.0), col);
                }
            }
        }
    }
    if app.show_shapes {
        use dz_core::shapes::ShapeKind;
        let grip = if app.mode == Mode::Shapes && app.shape.tool == ShapeTool::Edit { resp.hover_pos().and_then(|m| shape_handle(world, v, &app.shape.selection, m)) } else { None };
        if app.mode == Mode::Shapes && app.shape.tool == ShapeTool::Edit {
            if let Some(c) = selection_center(world, &app.shape.selection).and_then(|c| shape_screen(world, v, c)) {
                let lit = grip == Some(ShapeHandle::Move);
                painter.rect_filled(Rect::from_center_size(c, vec2(if lit { 16.0 } else { 12.0 }, if lit { 16.0 } else { 12.0 })), 1.0, if lit { Color32::WHITE } else { Color32::from_rgb(255, 200, 60) });
            }
        }
        for (li, l) in world.shape_layers.iter().enumerate().filter(|(_, l)| l.visible) {
            for (si, s) in l.shapes.iter().enumerate() {
                let selected = app.shape.selection.contains(&(li, si));
                let col = if selected { Color32::from_rgb(255, 150, 30) } else { SHAPE_COLORS[li % SHAPE_COLORS.len()] };
                if s.kind == ShapeKind::Point {
                    if let Some(c) = shape_screen(world, v, s.points[0]).filter(|c| v.rect.contains(*c)) {
                        painter.circle_filled(c, if selected { 5.5 } else { 3.5 }, col);
                    }
                } else {
                    draw_polyline(&painter, v, world, &s.points, Stroke::new(if selected { 2.6_f32 } else { 1.4 }, col));
                    if s.kind == ShapeKind::Polygon && s.points.len() > 2 {
                        draw_polyline(&painter, v, world, &[*s.points.last().unwrap(), s.points[0]], Stroke::new(if selected { 2.6_f32 } else { 1.4 }, col));
                    }
                }
                if selected && app.mode == Mode::Shapes && app.shape.selection.len() == 1 && app.shape.tool == ShapeTool::Edit {
                    for (i, p) in s.points.iter().enumerate() {
                        if let Some(c) = shape_screen(world, v, *p) {
                            let lit = app.shape.vertex == Some(i) || grip == Some(ShapeHandle::Vertex(i));
                            painter.circle_filled(c, if lit { 7.0 } else { 4.5 }, if lit { Color32::WHITE } else { col });
                        }
                    }
                    for (a, _, c) in edge_grips(world, v, s) {
                        let lit = grip == Some(ShapeHandle::Edge(a));
                        painter.circle_stroke(c, if lit { 8.0 } else { 6.0 }, Stroke::new(1.0_f32, if lit { Color32::WHITE } else { col }));
                        painter.text(c, egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(12.0), if lit { Color32::WHITE } else { col });
                    }
                }
            }
        }
    }
    if let Some((a, b, _)) = app.shape.boxing {
        painter.rect(Rect::from_two_pos(a, b), 0.0, Color32::from_rgba_unmultiplied(0, 112, 224, 40), Stroke::new(1.0_f32, theme::ACCENT), egui::StrokeKind::Inside);
    }
    if app.mode == Mode::Shapes && !app.shape.points.is_empty() {
        let col = Color32::from_rgb(255, 120, 40);
        let mut pts = app.shape.points.clone();
        let mods = painter.ctx().input(|i| i.modifiers);
        if let Some(h) = resp.hover_pos().and_then(|m| { let (o, d) = v.ray(m); pick_terrain(&world.height, o, d, v.ortho) }) {
            let p = line_point(app, &app.shape.points, [h[0], h[2]], mods);
            if let Some(last) = app.shape.points.last() { segment_label(&painter, v, world, *last, p); }
            pts.push(p);
        }
        if app.shape.tool == ShapeTool::Polygon && pts.len() > 2 {
            pts.push(pts[0]);
        }
        draw_polyline(&painter, v, world, &pts, Stroke::new(2.5_f32, col));
        for p in &app.shape.points {
            if let Some(s) = shape_screen(world, v, *p) {
                painter.circle_filled(s, 4.0, col);
            }
        }
    }
    if !app.tile_issues.is_empty() && v.ortho {
        let step = world.meta.tile_step();
        let size = world.meta.size_m();
        for (col, row, _) in &app.tile_issues {
            let (x, z) = (*col as f32 * step, size - (*row as f32 + 1.0) * step);
            if let (Some(a), Some(b)) = (v.project([x, 0.0, z + step]), v.project([x + step, 0.0, z])) {
                painter.rect(Rect::from_two_pos(a, b), 0.0, Color32::from_rgba_unmultiplied(230, 40, 40, 50), Stroke::new(1.5_f32, Color32::from_rgb(235, 60, 60)), egui::StrokeKind::Inside);
            }
        }
    }
    for m in world.modifiers.iter().filter(|m| m.enabled) {
        if let Some(c) = v.project([m.x, world.height.sample(m.x, m.z) + 0.5, m.z]) {
            if v.rect.contains(c) {
                let selected = app.selected_modifier == Some(m.id);
                painter.circle_stroke(c, if selected { 7.0 } else { 4.0 }, Stroke::new(1.5_f32, if selected { Color32::from_rgb(255, 170, 40) } else { Color32::from_rgb(120, 220, 255) }));
            }
        }
    }
    // Wire box round the first selected objects.
    for id in app.selection.iter().take(250) {
        let Some(o) = world.objects.get(*id) else { continue };
        let info = app.db.get(world.objects.model_path(o));
        let (mn, mx) = if info.known { (info.min, info.max) } else { ([-1.0, 0.0, -1.0], [1.0, 2.0, 1.0]) };
        let m = rotation(o.yaw, o.pitch, o.roll, o.scale);
        let origin = [o.x, ObjectStore::origin_y(o, &world.height), o.z];
        let corner = |x: f32, y: f32, z: f32| add(origin, add(add(mul(m[0], x), mul(m[1], y)), mul(m[2], z)));
        let c: Vec<Option<Pos2>> = [(0, 0, 0), (1, 0, 0), (1, 0, 1), (0, 0, 1), (0, 1, 0), (1, 1, 0), (1, 1, 1), (0, 1, 1)].iter().map(|&(a, b, d)| {
            v.project(corner(if a == 0 { mn[0] } else { mx[0] }, if b == 0 { mn[1] } else { mx[1] }, if d == 0 { mn[2] } else { mx[2] }))
        }).collect();
        for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)] {
            if let (Some(p), Some(q)) = (c[a], c[b]) {
                painter.line_segment([p, q], Stroke::new(1.0_f32, Color32::from_rgb(255, 170, 40)));
            }
        }
    }
    for (i, k) in world.keypoints.iter().enumerate().filter(|_| app.show_keypoints) {
        let selected = app.shape.keypoint == Some(i);
        if app.mode == Mode::KeyPoints {
            let (sin, cos) = k.angle.to_radians().sin_cos();
            let ring: Vec<Option<Pos2>> = (0..=48).map(|n| {
                let a = n as f32 / 48.0 * std::f32::consts::TAU;
                let (dx, dz) = (k.width * a.cos(), k.height * a.sin());
                let (x, z) = (k.x + cos * dx + sin * dz, k.z - sin * dx + cos * dz);
                v.project([x, world.height.sample(x, z) + 1.0, z])
            }).collect();
            let color = if selected { Color32::from_rgb(255, 210, 90) } else { Color32::from_rgb(120, 190, 255) };
            for w in ring.windows(2) {
                if let (Some(p), Some(q)) = (w[0], w[1]) { painter.line_segment([p, q], Stroke::new(1.0_f32, color)); }
            }
        }
        if !(k.show || app.mode == Mode::KeyPoints) { continue; }
        if let Some(s) = v.project([k.x, world.height.sample(k.x, k.z) + 1.0, k.z]) {
            painter.circle_filled(s, if selected { 5.0 } else { 3.5 }, if selected { Color32::from_rgb(255, 210, 90) } else { Color32::from_rgb(120, 190, 255) });
            painter.text(s + vec2(7.0, -2.0), egui::Align2::LEFT_CENTER, if k.text.is_empty() { &k.name } else { &k.text }, egui::FontId::proportional(11.0), Color32::WHITE);
        }
    }
    if let Some(s) = v.project([world.meta.spawn[0], world.height.sample(world.meta.spawn[0], world.meta.spawn[1]) + 1.0, world.meta.spawn[1]]) {
        painter.circle_filled(s, 5.0, Color32::from_rgb(90, 220, 110));
        painter.text(s + vec2(8.0, -2.0), egui::Align2::LEFT_CENTER, t("Player start"), egui::FontId::proportional(11.0), Color32::WHITE);
    }
    if app.mode == Mode::Lines && !app.line.points.is_empty() {
        let mut pts = app.line.points.clone();
        let mods = painter.ctx().input(|i| i.modifiers);
        if let Some(m) = resp.hover_pos() {
            let (o, d) = v.ray(m);
            if let Some(h) = pick_terrain(&world.height, o, d, v.ortho) {
                let p = line_point(app, &app.line.points, [h[0], h[2]], mods);
                if let Some(last) = app.line.points.last() { segment_label(&painter, v, world, *last, p); }
                pts.push(p);
            }
        }
        draw_polyline(&painter, v, world, &pts, Stroke::new(2.5_f32, Color32::from_rgb(255, 120, 40)));
        for p in &app.line.points {
            if let Some(s) = v.project([p[0], world.height.sample(p[0], p[1]), p[1]]) {
                painter.circle_filled(s, 4.0, Color32::from_rgb(255, 120, 40));
            }
        }
    }
    if app.mode == Mode::Lines && matches!(app.line.action, LineAction::Road | LineAction::Edit) {
        let cursor = resp.hover_pos().and_then(|m| { let (o, d) = v.ray(m); pick_terrain(&world.height, o, d, v.ortho) }).map(|h| [h[0], h[2]]);
        crate::road_draw::draw(app, &painter, v, world, cursor);
    }
    if let Some((a, b, _)) = app.box_select {
        if app.lasso_points.len() >= 2 {
            // The outline drawn so far, closed back to where it began.
            let mut pts = app.lasso_points.clone();
            pts.push(pts[0]);
            painter.add(egui::Shape::line(pts, Stroke::new(1.5_f32, theme::ACCENT)));
        } else if !app.settings.lasso {
            let r = Rect::from_two_pos(a, b);
            painter.rect(r, 0.0, Color32::from_rgba_unmultiplied(0, 112, 224, 40), Stroke::new(1.0_f32, theme::ACCENT), egui::StrokeKind::Inside);
        }
    }
    if app.mode == Mode::Select {
        if let Some(p) = selection_pivot(app) {
            let hover = resp.hover_pos().and_then(|m| gizmo_handle(app, v, p, m)).map(|h| h.0);
            draw_gizmo(app, &painter, v, p, app.drag.as_ref().map(|d| d.handle).or(hover));
        }
    }
    crate::mission_ui::draw_map(app, &painter, v, resp);
    // Compass.
    let c = pos2(v.rect.right() - 34.0, v.rect.bottom() - 34.0);
    let (nx, nz) = if v.ortho { (0.0, -1.0) } else { (-v.right[2], -(v.forward[2] * 0.5 + 0.5).max(0.3)) };
    let n = vec2(nx, nz).normalized() * 18.0;
    painter.circle_filled(c, 22.0, Color32::from_black_alpha(110));
    painter.arrow(c - n * 0.6, n * 1.5, Stroke::new(2.0_f32, Color32::from_rgb(230, 80, 70)));
    painter.text(c + n * 1.15, egui::Align2::CENTER_CENTER, "N", egui::FontId::proportional(10.0), Color32::WHITE);
}

/// Orthographic sun view over the area in front of the camera, snapped to shadow map texels
/// so edges do not shimmer while moving.
fn shadow_depth(range: f32) -> f32 {
    (range * 2.0).max(600.0)
}

/// Offset that grows with the size of a shadow map texel.
fn shadow_bias(range: f32) -> f32 {
    let texel = range * 2.0 / 4096.0;
    (0.3 + texel * 2.0) / (2.0 * shadow_depth(range))
}

fn sun_matrix(v: &View, sun: V3, range: f32) -> [f32; 16] {
    let depth = shadow_depth(range);
    let l = norm(sun);
    let r = norm(crate::camera::cross([0.0, 1.0, 0.0], l));
    let u = crate::camera::cross(l, r);
    // Moves in steps so the map is redrawn rarely and its edges stay still.
    let c = add(v.eye, mul(norm([v.forward[0], 0.0, v.forward[2]]), range * 0.5));
    let c = [(c[0] / 16.0).round() * 16.0, c[1], (c[2] / 16.0).round() * 16.0];
    let texel = range * 2.0 / 4096.0;
    let (rc, uc) = ((dot(r, c) / texel).round() * texel, (dot(u, c) / texel).round() * texel);
    [
        r[0] / range, u[0] / range, -l[0] / depth, 0.0,
        r[1] / range, u[1] / range, -l[1] / depth, 0.0,
        r[2] / range, u[2] / range, -l[2] / depth, 0.0,
        -rc / range, -uc / range, dot(l, c) / depth, 1.0,
    ]
}

fn brush_of(app: &App, hit: Option<V3>) -> Option<BrushView> {
    let h = hit?;
    match app.mode {
        Mode::Landscape if app.hole_tool => Some(BrushView { x: h[0], z: h[2], radius: app.hole_radius, falloff: 0.0, color: [1.0, 0.3, 0.3], shape: 1 }),
        Mode::Landscape => Some(BrushView { x: h[0], z: h[2], radius: app.sculpt.radius, falloff: if app.sculpt.picture.is_some() { 0.0 } else { app.sculpt.falloff }, color: [0.25, 0.65, 1.0], shape: if app.sculpt.picture.is_some() { 1 } else { app.sculpt.shape as i32 } }),
        Mode::Paint => Some(BrushView { x: h[0], z: h[2], radius: app.paint.radius, falloff: 0.0, color: [1.0, 0.75, 0.2], shape: if app.paint.picture.is_some() { 1 } else { app.paint.shape as i32 } }),
        Mode::Place => Some(BrushView { x: h[0], z: h[2], radius: if app.place.brush { app.place.brush_radius } else { 2.0 }, falloff: 0.0, color: [0.3, 1.0, 0.4], shape: 0 }),
        Mode::Satellite => Some(BrushView { x: h[0], z: h[2], radius: app.sat.radius, falloff: 1.0 - app.sat.hardness, color: [1.0, 0.82, 0.24], shape: 0 }),
        Mode::Mission => crate::mission_ui::brush(app, h),
        _ => None,
    }
}

/// The wickets and gates of the selected fence generator on its line, and while one waits for
/// a click, where on the fence it would go.
fn fence_openings(app: &App, painter: &egui::Painter, v: &View, ctx: &egui::Context, resp: &egui::Response) {
    let Some(d) = &app.doc else { return };
    let id = app.pick_opening.map(|p| p.0).or(app.selected_generator);
    let Some(g) = id.and_then(|id| d.world.generators.iter().find(|g| g.id == id)) else { return };
    let dz_core::library::GeneratorKind::Fence(f) = &g.kind else { return };
    let lines = crate::library_ui::fence_lines(&d.world, g);
    let at = |p: [f32; 2]| v.project([p[0], d.world.height.sample(p[0], p[1]) + 0.5, p[1]]);
    let color = Color32::from_rgb(255, 200, 60);
    for line in &lines {
        for (k, o) in f.openings.iter().enumerate() {
            let Some(c) = at(gen::point_along(line, o.at)) else { continue };
            let waiting = app.pick_opening == Some((g.id, k));
            painter.circle_filled(c, if waiting { 9.0 } else { 7.0 }, if waiting { theme::ACCENT } else { color });
            painter.text(c + vec2(0.0, -12.0), egui::Align2::CENTER_BOTTOM, if o.gate { t("Gate") } else { t("Wicket") }, egui::FontId::proportional(12.0), Color32::WHITE);
        }
    }
    // While picking, the point of the fence nearest the cursor.
    if app.pick_opening.is_some_and(|p| p.0 == g.id) {
        let Some(m) = ctx.pointer_latest_pos().filter(|_| resp.contains_pointer()) else { return };
        let (o, dir) = v.ray(m);
        let Some(h) = pick_terrain(&d.world.height, o, dir, v.ortho) else { return };
        let mut best: Option<(f32, [f32; 2])> = None;
        for line in &lines {
            for s in line.windows(2) {
                let (ab, ap) = ([s[1][0] - s[0][0], s[1][1] - s[0][1]], [h[0] - s[0][0], h[2] - s[0][1]]);
                let u = ((ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-6)).clamp(0.0, 1.0);
                let q = [s[0][0] + ab[0] * u, s[0][1] + ab[1] * u];
                let off = (q[0] - h[0]).hypot(q[1] - h[2]);
                if best.is_none_or(|b| off < b.0) { best = Some((off, q)); }
            }
        }
        if let Some((off, q)) = best.filter(|b| b.0 < 30.0) {
            if let Some(c) = at(q) {
                painter.circle_stroke(c, 10.0, Stroke::new(2.5_f32, theme::ACCENT));
                painter.text(c + vec2(14.0, 0.0), egui::Align2::LEFT_CENTER, format!("{off:.1} m"), egui::FontId::proportional(11.0), Color32::WHITE);
            }
        }
        ctx.request_repaint();
    }
}

fn view_header(app: &mut App, ui: &mut egui::Ui, rect: Rect, top: bool) {
    let area = Rect::from_min_size(rect.min + vec2(6.0, 5.0), vec2(rect.width() - 12.0, 24.0));
    // The backing is sized to the controls once they are laid out, so it always takes all of them.
    let backing = ui.painter().add(egui::Shape::Noop);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let shading = if top { &mut app.shading2 } else { &mut app.shading3 };
    egui::ComboBox::from_id_salt(if top { "shade2" } else { "shade3" }).selected_text(t(shading.name())).width(110.0).show_ui(&mut child, |ui| {
        for s in Shading::ALL {
            ui.selectable_value(shading, s, t(s.name()));
        }
    });
    child.label(egui::RichText::new(if top { t("Top") } else { t("Perspective") }).color(Color32::WHITE).background_color(Color32::from_black_alpha(100)));
    if top {
        top_view_menu(app, &mut child);
    }
    if !top {
        child.checkbox(&mut app.wireframe, crate::icons::with(crate::icons::CUBE_TRANSPARENT, &t("Wireframe")));
        let mut speed = app.camera.speed;
        child.add(egui::Slider::new(&mut speed, 1.0..=4000.0).logarithmic(true).suffix(" m/s").show_value(true).text(""));
        app.camera.speed = speed;
        lighting_menu(app, &mut child);
        show_menu(app, &mut child);
        camera_menu(app, &mut child);
    }
    let used = child.min_rect();
    let back = Rect::from_min_max(rect.min + vec2(2.0, 2.0), pos2((used.right() + 6.0).min(rect.right() - 2.0), rect.top() + 32.0));
    ui.painter().set(backing, egui::Shape::rect_filled(back, 3.0, Color32::from_black_alpha(150)));
}

/// The game's MaxTide: how far the tide moves the sea, in metres.
const MAX_TIDE: f32 = 1.5;

/// The tide now: the one chosen, or the cycle running through all of it.
fn tide(app: &App, ctx: &egui::Context) -> f32 {
    if app.settings.tide_cycle {
        MAX_TIDE * (ctx.input(|i| i.time) as f32 * std::f32::consts::TAU / 120.0).sin()
    } else {
        app.settings.sea_tide.clamp(-MAX_TIDE, MAX_TIDE)
    }
}

/// What the top view shows and how the split view is laid out.
fn top_view_menu(app: &mut App, ui: &mut egui::Ui) {
    ui.menu_button(crate::icons::with(crate::icons::MAP_TRIFOLD, &t("Top view")), |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let mut changed = ui.checkbox(&mut app.settings.top_models, t("Models from above")).on_hover_text(t("Close up the top view draws the models themselves, as a drone sees them, instead of their outlines.")).changed();
        changed |= ui.checkbox(&mut app.settings.top_follow, t("Follow the 3D camera")).on_hover_text(t("In the split view the top view keeps the 3D camera in its middle.")).changed();
        ui.separator();
        ui.label(t("Split view"));
        changed |= ui.radio_value(&mut app.settings.split_stacked, false, t("Side by side")).changed();
        changed |= ui.radio_value(&mut app.settings.split_stacked, true, t("One above the other")).changed();
        changed |= ui.checkbox(&mut app.settings.split_swapped, t("Top view first")).changed();
        if ui.button(t("Share the room evenly")).clicked() {
            app.settings.split_ratio = 0.5;
            changed = true;
        }
        if changed { app.settings.save(); }
    });
}

fn camera_menu(app: &mut App, ui: &mut egui::Ui) {
    ui.menu_button(crate::icons::with(crate::icons::CAMERA, &t("Camera")), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
        ui.set_min_width(280.0);
        let row = |ui: &mut egui::Ui, label: String, v: &mut f32, range: std::ops::RangeInclusive<f32>| {
            ui.add(egui::Slider::new(v, range).text(label));
        };
        row(ui, t("Field of view"), &mut app.settings.fov, 20.0..=120.0);
        app.camera.fov = app.settings.fov.to_radians();
        row(ui, t("Movement speed"), &mut app.settings.camera_speed, 1.0..=4000.0);
        app.camera.speed = app.settings.camera_speed;
        row(ui, t("Object draw distance"), &mut app.settings.draw_distance, 300.0..=8000.0);
        row(ui, t("Full detail distance"), &mut app.settings.near_detail, 30.0..=5000.0);
        row(ui, t("Surface texture range m"), &mut app.settings.surface_range, 60.0..=5000.0);
        surface_relief_ui(ui, &mut app.settings);
        row(ui, t("Sea waves m"), &mut app.settings.sea_waves, 0.0..=1.0);
        ui.weak(t("How far the swell runs up the shore and back; the game's MaxWave is 0.25 m."));
        ui.add_enabled(!app.settings.tide_cycle, egui::Slider::new(&mut app.settings.sea_tide, -MAX_TIDE..=MAX_TIDE).text(t("Tide m")));
        ui.checkbox(&mut app.settings.tide_cycle, t("Tide rises and falls")).on_hover_text(t("Runs through the whole tide of the game, up to MaxTide 1.5 m, in two minutes."));
    });
}

fn show_menu(app: &mut App, ui: &mut egui::Ui) {
    ui.menu_button(crate::icons::with(crate::icons::EYE, &t("Show")), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); crate::panels::visibility_toggles(app, ui) });
}

fn lighting_menu(app: &mut App, ui: &mut egui::Ui) {
    use crate::settings::Lighting;
    ui.menu_button(crate::icons::with(crate::icons::SUN, &t("Lighting")), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
        ui.set_min_width(260.0);
        let l = &mut app.settings.lighting;
        ui.horizontal_wrapped(|ui| {
            for (name, preset) in [("Morning", Lighting::MORNING), ("Noon", Lighting::NOON), ("Evening", Lighting::EVENING), ("Overcast", Lighting::OVERCAST)] {
                if ui.button(t(name)).clicked() {
                    *l = preset;
                }
            }
        });
        ui.separator();
        theme::row(ui, &t("Sun azimuth"), |ui| { ui.add(egui::Slider::new(&mut l.azimuth, 0.0..=360.0).suffix("°")); });
        theme::row(ui, &t("Sun elevation"), |ui| { ui.add(egui::Slider::new(&mut l.elevation, -10.0..=90.0).suffix("°")); });
        theme::row(ui, &t("Sun strength"), |ui| { ui.add(egui::Slider::new(&mut l.strength, 0.0..=2.0)); });
        theme::row(ui, &t("Sun colour"), |ui| { ui.color_edit_button_rgb(&mut l.color); });
        theme::row(ui, &t("Ambient"), |ui| { ui.add(egui::Slider::new(&mut l.ambient, 0.0..=2.0)); });
        theme::row(ui, &t("Exposure"), |ui| { ui.add(egui::Slider::new(&mut l.exposure, 0.25..=2.5)); });
        theme::row(ui, &t("Fog"), |ui| { ui.add(egui::Slider::new(&mut l.fog, 0.0..=6.0)); });
        ui.checkbox(&mut app.show_shadows, t("Shadows"));
        theme::row(ui, &t("Shadow range"), |ui| { ui.add(egui::Slider::new(&mut l.shadow_range, 60.0..=2000.0).logarithmic(true).suffix(" m")); });
        if ui.button(t("Reset")).clicked() {
            app.settings.lighting = Lighting::default();
        }
    });
}

/// The parallax and blending of the surfaces, from the relief in their alpha channel.
pub fn surface_relief_ui(ui: &mut egui::Ui, s: &mut crate::settings::Settings) {
    ui.add(egui::Slider::new(&mut s.surface_parallax, 0.0..=15.0).text(t("Surface parallax cm"))).on_hover_text(t("How deep the relief in the alpha channel of the surface textures looks close up; 0 leaves them flat."));
    ui.add(egui::Slider::new(&mut s.surface_blend, 0.0..=1.0).text(t("Surface blend by relief"))).on_hover_text(t("Where two surfaces of the mask meet, the taller parts of their relief show through first; higher makes the border sharper."));
}

fn draw_view(app: &mut App, ui: &mut egui::Ui, rect: Rect, top: bool) {
    let id = ui.id().with(if top { "top" } else { "persp" });
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let v = {
        let Some(doc) = &app.doc else { return };
        if top {
            View::top(app.top_center, app.top_mpp, rect)
        } else {
            let g = doc.world.height.sample(app.camera.pos[0], app.camera.pos[2]);
            View::perspective(&app.camera, rect, (app.camera.pos[1] - g).max(0.5))
        }
    };
    let ctx = ui.ctx().clone();
    let typing = ctx.wants_keyboard_input();
    camera_input(app, &ctx, &resp, &v, top, typing);
    let v = {
        let Some(doc) = &app.doc else { return };
        if top {
            View::top(app.top_center, app.top_mpp, rect)
        } else {
            let g = doc.world.height.sample(app.camera.pos[0], app.camera.pos[2]);
            View::perspective(&app.camera, rect, (app.camera.pos[1] - g).max(0.5))
        }
    };
    tool_input(app, &ctx, &resp, &v);
    let hover = resp.hover_pos().and_then(|m| {
        let d = app.doc.as_ref()?;
        let (o, dir) = v.ray(m);
        pick_terrain(&d.world.height, o, dir, v.ortho)
    });
    if resp.hovered() {
        app.cursor_ground = hover;
    }
    if let Some(h) = hover {
        if resp.hovered() {
            let d = app.doc.as_ref().unwrap();
            let slope = d.world.height.slope(h[0], h[2]);
            let layer = d.world.mask.sample(h[0], h[2], d.world.meta.size_m()) as usize;
            let name = d.world.layers.get(layer).map(|l| l.name.as_str()).unwrap_or("-");
            app.status = tf("X {0}  Z {1}  height {2} m  slope {3}°  surface {4}", &[&format!("{:.1}", h[0]), &format!("{:.1}", h[2]), &format!("{:.2}", h[1]), &format!("{slope:.1}"), name]);
        }
    }
    // High resolution satellite window around what is being looked at.
    if let Some(doc) = &app.doc {
        if let Some(s) = &doc.world.satellite {
            let side = doc.world.meta.size_m();
            let c = if top { app.top_center } else { [app.camera.pos[0] + app.camera.forward()[0] * 300.0, app.camera.pos[2] + app.camera.forward()[2] * 300.0] };
            let (dx, dz) = (c[0] - app.detail_center[0], c[1] - app.detail_center[1]);
            if dx * dx + dz * dz > (side * 0.03).max(350.0).powi(2) && app.stroke.is_none() {
                app.detail_center = c;
                app.renderer.lock().unwrap().set_detail(s, c[0], c[1], side);
            }
        }
    }
    let objects = prepare_objects(app, &v, top);
    let Some(doc) = &app.doc else { return };
    let quality = app.settings.terrain_quality;
    let patches = doc.lod.select(&v, quality);
    let ranges = doc.lod.ranges(quality);
    let meta = &doc.world.meta;
    let frame = Frame {
        view: v,
        viewport_key: top as u8,
        patches: Arc::new(patches),
        ranges: Arc::new(ranges),
        shading: if top { app.shading2 } else { app.shading3 },
        wireframe: app.wireframe && !top,
        brush: brush_of(app, hover),
        grid: [if app.show_land_grid { meta.land_cell() } else { 0.0 }, if app.show_tile_grid { meta.tile_step() } else { 0.0 }, if app.show_contours { app.contour_interval } else { 0.0 }],
        sea: meta.sea_level,
        height_range: doc.height_range,
        objects,
        show_water: app.show_water,
        surface_range: app.settings.surface_range,
        parallax: app.settings.surface_parallax.max(0.0) / 100.0,
        blend_sharp: app.settings.surface_blend.clamp(0.0, 1.0),
        sea_waves: app.settings.sea_waves,
        tide: tide(app, &ctx),
        world: meta.size_m(),
        tile: 10.0 / meta.land_cell(),
        lighting: app.settings.lighting.gpu(),
        time: ctx.input(|i| i.time) as f32,
        light: (!top && app.show_shadows && app.settings.lighting.elevation > 1.0).then(|| sun_matrix(&v, app.settings.lighting.sun_direction(), app.settings.lighting.shadow_range)),
        shadow_bias: shadow_bias(app.settings.lighting.shadow_range),
        ao: if top || (if top { app.shading2 } else { app.shading3 }) != Shading::Lit { 0 } else { app.settings.ambient_occlusion },
        smoothing: app.settings.edge_smoothing,
    };
    // The sea moves on its own, also when nothing else makes the window redraw.
    if !top && app.show_water && (app.settings.sea_waves > 0.0 || app.settings.tide_cycle) && doc.height_range[0] < meta.sea_level + MAX_TIDE + app.settings.sea_waves {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
    ui.painter_at(rect).add(render::callback(app.renderer.clone(), frame));
    draw_overlays(app, ui, &v, &resp);
    fence_openings(app, &ui.painter_at(rect), &v, &ctx, &resp);
    if egui::DragAndDrop::has_any_payload(&ctx) {
        let painter = ui.painter_at(rect);
        let over = resp.contains_pointer();
        painter.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(if over { 3.0_f32 } else { 1.5 }, if over { theme::ACCENT } else { Color32::from_rgba_unmultiplied(0, 112, 224, 150) }), egui::StrokeKind::Inside);
        let at = ctx.pointer_latest_pos().filter(|_| over).and_then(|m| {
            let d = app.doc.as_ref()?;
            let (o, dir) = v.ray(m);
            pick_terrain(&d.world.height, o, dir, v.ortho)
        });
        if let Some(at) = at {
            crate::asset_browser::drop_preview(app, &painter, &v, at);
        }
        ctx.request_repaint();
    }
    view_header(app, ui, rect, top);
    // How far a turn has gone, and its step while it goes by steps.
    if let (Some(d), Some(m)) = (app.drag.as_ref().filter(|d| d.started && d.handle == Handle::Spin), ctx.pointer_latest_pos()) {
        if let Some((id, b)) = d.before.first() {
            let now = app.doc.as_ref().and_then(|doc| doc.world.objects.get(*id)).map_or(b.yaw, |o| o.yaw);
            let turned = ((now - b.yaw + 540.0).rem_euclid(360.0)) - 180.0;
            let text = if d.stepped { tf("{0}°  ·  step {1}° (wheel)", &[&format!("{turned:.1}"), &format!("{}", app.settings.angle_step)]) } else { format!("{turned:.1}°") };
            ui.painter_at(rect).text(m + vec2(16.0, -16.0), egui::Align2::LEFT_BOTTOM, text, egui::FontId::proportional(13.0), Color32::WHITE);
        }
    }
    if top {
        camera_marker(app, &ui.painter_at(rect), &v);
    } else if app.settings.minimap {
        minimap(app, ui, rect);
    }
    if resp.secondary_clicked() {
        if let Some(m) = ctx.input(|i| i.pointer.interact_pos()) {
            if let Some(id) = pick_object(app, &v, m) {
                if !app.selection.contains(&id) {
                    app.leave_browser();
                    app.selection.clear();
                    app.selection.insert(id);
                    app.selected_generator = None;
                    app.selected_modifier = None;
                    app.whole = None;
                    app.selection_rev += 1;
                }
            }
        }
    }
    let shown_model = app.selected_model();
    let mut reveal = None;
    let mut make_prefab = false;
    resp.context_menu(|ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        if let Some(m) = &shown_model {
            if ui.button(crate::icons::with(crate::icons::PACKAGE, &t("Create prefab from the selection"))).clicked() {
                make_prefab = true;
                ui.close_menu();
            }
            if ui.button(crate::icons::with(crate::icons::FOLDER_OPEN, &t("Show in asset browser"))).on_hover_text(m.as_str()).clicked() {
                reveal = Some(m.clone());
                ui.close_menu();
            }
            ui.separator();
        }
        if ui.button(t("Set player start here")).clicked() {
            if let (Some(h), Some(d)) = (hover, app.doc.as_mut()) {
                d.world.meta.spawn = [h[0], h[2]];
                d.history.revision += 1;
            }
            ui.close_menu();
        }
        if ui.button(t("Place selected model here")).clicked() {
            if let (Some(h), Some(m)) = (hover, app.place.model.clone()) {
                place_model(app, &m, h[0], h[2]);
            }
            ui.close_menu();
        }
        if ui.button(t("Move camera here")).clicked() {
            if let Some(h) = hover {
                app.camera.pos = [h[0], h[1] + 25.0, h[2]];
                app.top_center = [h[0], h[2]];
            }
            ui.close_menu();
        }
    });
    if let Some(m) = reveal {
        app.show_in_browser(&m);
    }
    if make_prefab {
        app.create_prefab();
    }
}

/// The brush of the current tool, if it has one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActiveBrush {
    Sculpt,
    Paint,
    Holes,
    Objects,
    Clone,
}

pub fn active_brush(app: &App) -> Option<ActiveBrush> {
    match app.mode {
        Mode::Landscape if app.hole_tool => Some(ActiveBrush::Holes),
        Mode::Landscape => Some(ActiveBrush::Sculpt),
        Mode::Paint => Some(ActiveBrush::Paint),
        Mode::Place if app.place.brush => Some(ActiveBrush::Objects),
        Mode::Satellite => Some(ActiveBrush::Clone),
        _ => None,
    }
}

/// Changes the brush of the current tool: its size and strength by factors, its edge noise by
/// a step. Every brush takes the same keys and wheel.
pub fn adjust_brush(app: &mut App, size: f32, strength: f32, noise: f32) {
    match active_brush(app) {
        Some(ActiveBrush::Sculpt) => {
            app.sculpt.radius = (app.sculpt.radius * size).clamp(2.0, 3000.0);
            app.sculpt.strength = (app.sculpt.strength * strength).clamp(0.5, 200.0);
            app.sculpt.noise = (app.sculpt.noise + noise).clamp(0.0, 1.0);
        }
        Some(ActiveBrush::Paint) => {
            app.paint.radius = (app.paint.radius * size).clamp(0.5, 2000.0);
            app.paint.scatter = (app.paint.scatter + noise).clamp(0.0, 0.8);
        }
        Some(ActiveBrush::Holes) => app.hole_radius = (app.hole_radius * size).clamp(0.5, 200.0),
        Some(ActiveBrush::Objects) => {
            app.place.brush_radius = (app.place.brush_radius * size).clamp(1.0, 300.0);
            // A stronger brush places objects closer together.
            app.place.brush_spacing = (app.place.brush_spacing / strength).clamp(0.5, 100.0);
        }
        Some(ActiveBrush::Clone) => {
            app.sat.radius = (app.sat.radius * size).clamp(0.5, 500.0);
            app.sat.opacity = (app.sat.opacity * strength).clamp(0.02, 1.0);
            app.sat.hardness = (app.sat.hardness - noise).clamp(0.0, 1.0);
        }
        None => {}
    }
}

/// True when the wheel belongs to the brush, because one of its wheel modifiers is held.
pub fn brush_wheel(app: &App, m: egui::Modifiers) -> bool {
    let k = &app.settings.keys;
    active_brush(app).is_some() && [Hold::WheelSize, Hold::WheelStrength, Hold::WheelNoise].iter().any(|h| k.held(*h, m))
}

fn shortcuts(app: &mut App, ctx: &egui::Context) {
    // Experimental features turned off take their tools with them.
    if !app.mode_enabled(app.mode) { app.mode = Mode::Select; }
    if !app.settings.experimental { app.mission.open = false; }
    crate::panels::HINTS.store(app.settings.show_hints, std::sync::atomic::Ordering::Relaxed);
    if ctx.wants_keyboard_input() || app.key_capture.is_some() || app.team.merging() {
        return;
    }
    let looking = ctx.input(|i| i.pointer.secondary_down());
    if app.selection.is_empty() { app.whole = None; }
    if let Some(d) = &app.doc {
        if app.holes_seen != d.world.holes_rev {
            app.holes_seen = d.world.holes_rev;
            app.renderer.lock().unwrap().set_holes(&d.world.holes, d.world.height.cols, d.world.height.rows);
        }
    }
    let keys = app.settings.keys.clone();
    let k = |a: Action| keys.pressed(ctx, a);
    // These work everywhere, also while flying.
    if k(Action::Undo) { app.undo(); }
    if k(Action::Redo) { app.redo(); }
    if k(Action::Save) { app.save(None); }
    if k(Action::SaveAs) { app.save_as(); }
    if k(Action::Open) { app.pick_and_open(); }
    if k(Action::NewProject) { app.dialog = crate::panels::new_project_dialog(); }
    if k(Action::Settings) { app.dialog = crate::app::Dialog::Settings; }
    if k(Action::Shortcuts) { app.dialog = crate::app::Dialog::Shortcuts; }
    if k(Action::Build) && app.doc.is_some() && app.task.is_none() { app.build(false); }
    if k(Action::Play) && app.doc.is_some() && app.task.is_none() { app.build(true); }
    if k(Action::SearchAssets) {
        app.bottom = crate::app::BottomTab::Content;
        app.focus_search = true;
    }
    if looking {
        // The fly keys are read by the camera.
        return;
    }
    if k(Action::Duplicate) { app.duplicate_selection(); }
    app.remember_selection();
    if matches!(app.mode, Mode::Select | Mode::Place) && app.view_hovered_last {
        if k(Action::PasteHere) { app.paste_at_cursor(false); }
        if k(Action::PasteLast) { app.paste_at_cursor(true); }
    }
    if k(Action::Delete) && !matches!(app.mode, Mode::Shapes | Mode::KeyPoints | Mode::Mission) { app.delete_selection(); }
    if k(Action::Focus) { focus_selection(app); }
    if k(Action::ToggleHints) {
        app.settings.show_hints = !app.settings.show_hints;
        app.settings.save();
    }
    let laying = app.line.action == LineAction::Road && app.line.draw == RoadDraw::Parts;
    let changing = app.line.action == LineAction::Edit && app.road_parts.target.is_some_and(|t| app.line.selected_route == Some(t.0));
    if app.mode == Mode::Lines && (laying || changing) {
        if k(Action::RoadPieceAdd) { crate::road_draw::add_piece(app); }
        if k(Action::RoadPieceSide) { app.road_parts.left = !app.road_parts.left; }
        for (a, step) in [(Action::RoadPiecePrev, -1), (Action::RoadPieceNext, 1)] {
            if k(a) { crate::road_draw::cycle_piece(app, step); }
        }
    }
    if k(Action::Underground) { app.toggle_underground(); }
    if k(Action::CameraBelow) { app.camera_under = !app.camera_under; }
    if k(Action::ToggleGrass) { app.show_grass = !app.show_grass; }
    if k(Action::ToggleObjects) { app.show_objects = !app.show_objects; }
    if k(Action::ToggleShapes) {
        // Shapes and road lines go together: hidden while either shows, shown when both are off.
        let show = !(app.show_shapes || app.show_lines);
        app.show_shapes = show;
        app.show_lines = show;
    }
    if k(Action::Deselect) && app.mode == Mode::Select {
        app.selection.clear();
        app.selection_rev += 1;
    }
    if k(Action::SelectAll) {
        if app.mode == Mode::Shapes {
            if let Some(d) = &app.doc {
                app.shape.selection = d.world.shape_layers.iter().enumerate().filter(|(_, l)| l.visible).flat_map(|(li, l)| (0..l.shapes.len()).map(move |si| (li, si))).collect();
            }
        } else if let Some(d) = &app.doc {
            app.selection = d.world.objects.iter().filter(|(_, o)| d.world.object_interactive(o)).map(|(id, _)| id).collect();
            app.selection_rev += 1;
        }
    }
    for (a, m) in [(Action::ModeSelect, Mode::Select), (Action::ModeSculpt, Mode::Landscape), (Action::ModePaint, Mode::Paint), (Action::ModePlace, Mode::Place), (Action::ModeRoads, Mode::Lines), (Action::ModeShapes, Mode::Shapes), (Action::ModeKeyPoints, Mode::KeyPoints), (Action::ModeSatellite, Mode::Satellite), (Action::ModeMission, Mode::Mission)] {
        if k(a) && app.mode_enabled(m) { app.mode = m; }
    }
    for (a, g) in [(Action::GizmoMove, Gizmo::Move), (Action::GizmoRotate, Gizmo::Rotate), (Action::GizmoScale, Gizmo::Scale)] {
        if k(a) { app.gizmo = g; }
    }
    if k(Action::GizmoSpace) { app.gizmo_local = !app.gizmo_local; }
    for (a, v) in [(Action::View3d, ViewMode::Perspective), (Action::View2d, ViewMode::Top), (Action::ViewSplit, ViewMode::Split)] {
        if k(a) { app.view_mode = v; }
    }
    if k(Action::BrushSmaller) { adjust_brush(app, 0.8, 1.0, 0.0); }
    if k(Action::BrushBigger) { adjust_brush(app, 1.25, 1.0, 0.0); }
    if k(Action::BrushWeaker) { adjust_brush(app, 1.0, 0.8, 0.0); }
    if k(Action::BrushStronger) { adjust_brush(app, 1.0, 1.25, 0.0); }
    // Holding a modifier turns the wheel into a brush control. egui reports Shift+wheel as
    // sideways scrolling and Ctrl+wheel as zoom, so all of them are added up.
    if active_brush(app).is_some() {
        let (scroll, zoom, mods) = ctx.input(|i| (i.smooth_scroll_delta.x + i.smooth_scroll_delta.y, i.zoom_delta(), i.modifiers));
        let delta = if mods.ctrl || mods.command { scroll + (zoom - 1.0) * 400.0 } else { scroll };
        // Only over a view: the wheel over a panel scrolls the panel.
        if delta != 0.0 && app.view_hovered {
            let f = (delta * 0.003).exp();
            let size = if keys.held(Hold::WheelSize, mods) { f } else { 1.0 };
            let strength = if keys.held(Hold::WheelStrength, mods) { f } else { 1.0 };
            let noise = if keys.held(Hold::WheelNoise, mods) { delta * 0.0012 } else { 0.0 };
            if size != 1.0 || strength != 1.0 || noise != 0.0 {
                adjust_brush(app, size, strength, noise);
            }
        }
    }
}

fn welcome(app: &mut App, ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.22);
        ui.heading(egui::RichText::new("DZ World Editor").size(30.0).color(Color32::WHITE));
        ui.label(t("DayZ terrain editor"));
        ui.add_space(18.0);
        let size = vec2(280.0, 30.0);
        if ui.add_sized(size, egui::Button::new(t("New project..."))).clicked() {
            app.dialog = crate::panels::new_project_dialog();
        }
        if ui.add_sized(size, egui::Button::new(t("Open project..."))).clicked() {
            app.pick_and_open();
        }
        if ui.add_sized(size, egui::Button::new(t("Import Terrain Builder or built map..."))).clicked() {
            app.import_folder();
        }
        if !app.settings.recent.is_empty() {
            ui.add_space(14.0);
            ui.weak(t("Recent projects"));
            for d in app.settings.recent.clone().into_iter().take(8) {
                let name = d.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                if ui.link(name).on_hover_text(d.display().to_string()).clicked() {
                    app.open_project(d);
                }
            }
        }
    });
}

pub fn central(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    app.modifiers_now = ctx.input(|i| i.modifiers);
    app.view_hovered_last = app.view_hovered;
    if app.task.is_some() {
        if let Some(p) = app.shot_during_task.take() {
            app.pending_shot(p, &ctx);
        }
    }
    shortcuts(app, &ctx);
    // The views of this frame say again whether the pointer is over one of them.
    app.view_hovered = false;
    if app.doc.is_none() {
        welcome(app, ui);
        return;
    }
    crate::mission_ui::prepare(app, &ctx);
    let rect = ui.available_rect_before_wrap();
    if app.settings.top_follow && app.view_mode == ViewMode::Split {
        app.top_center = [app.camera.pos[0], app.camera.pos[2]];
    }
    match app.view_mode {
        ViewMode::Perspective => draw_view(app, ui, rect, false),
        ViewMode::Top => draw_view(app, ui, rect, true),
        ViewMode::Split => {
            let (first, second) = split_view(app, ui, rect);
            let (persp, top) = if app.settings.split_swapped { (second, first) } else { (first, second) };
            draw_view(app, ui, persp, false);
            draw_view(app, ui, top, true);
        }
    }
    let error = app.renderer.lock().unwrap().error.clone();
    if let Some(e) = error {
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, e, egui::FontId::monospace(11.0), theme::ERROR);
    }
}

/// Where the 3D camera stands and what it looks at, drawn on the top view.
fn camera_marker(app: &App, painter: &egui::Painter, v: &View) {
    let Some(doc) = &app.doc else { return };
    let p = app.camera.pos;
    let Some(c) = v.project([p[0], doc.world.height.sample(p[0], p[2]), p[2]]) else { return };
    let f = app.camera.forward();
    let heading = f[0].atan2(f[2]);
    let reach = 70.0;
    let half = app.camera.fov * 0.5 * (v.rect.width() / v.rect.height().max(1.0)).clamp(1.0, 2.0);
    let ray = |a: f32| c + vec2(a.sin(), -a.cos()) * reach;
    let (l, r) = (ray(heading - half), ray(heading + half));
    let col = Color32::from_rgb(255, 210, 60);
    painter.add(egui::Shape::convex_polygon(vec![c, l, r], Color32::from_rgba_unmultiplied(255, 210, 60, 30), Stroke::NONE));
    painter.line_segment([c, l], Stroke::new(1.5_f32, col));
    painter.line_segment([c, r], Stroke::new(1.5_f32, col));
    painter.circle(c, 5.0, col, Stroke::new(1.5_f32, Color32::BLACK));
}

/// A small map of the terrain in the corner of the 3D view, drawn as the top view draws, with
/// the camera on it. A click or drag takes the camera there; the wheel zooms about the
/// pointer, the right or middle button drags the map, the buttons zoom or show the whole map,
/// and the grip in the top right corner makes it bigger or smaller.
fn minimap(app: &mut App, ui: &mut egui::Ui, view: Rect) {
    let most = (view.width() - 80.0).min(view.height() - 60.0);
    if most < 100.0 {
        app.minimap_hover = false;
        return;
    }
    let side = app.settings.minimap_size.clamp(100.0, most);
    let rect = Rect::from_min_size(pos2(view.left() + 10.0, view.bottom() - side - 10.0), vec2(side, side));
    let Some(doc) = &app.doc else { return };
    let size = doc.world.meta.size_m();
    let cam = app.camera.pos;
    let ground_here = doc.world.height.sample(cam[0], cam[2]);
    let r = ui.interact(rect, ui.id().with("minimap"), Sense::click_and_drag());
    app.minimap_hover = r.hovered() || r.dragged();
    // Zoom by the wheel, keeping the point under the pointer where it is.
    let zoom_at = |app: &mut App, factor: f32, at: Option<Pos2>| {
        let span = size / app.minimap_zoom;
        let c = app.minimap_center;
        let p = at.map(|m| [c[0] - span * 0.5 + (m.x - rect.left()) / rect.width() * span, c[1] + span * 0.5 - (m.y - rect.top()) / rect.height() * span]).unwrap_or(c);
        let zoom = (app.minimap_zoom * factor).clamp(1.0, 256.0);
        let k = app.minimap_zoom / zoom;
        app.minimap_zoom = zoom;
        app.minimap_center = [p[0] + (c[0] - p[0]) * k, p[1] + (c[1] - p[1]) * k];
    };
    if app.minimap_center[0].is_nan() {
        app.minimap_center = [size * 0.5, size * 0.5];
    }
    if r.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
        if scroll != 0.0 {
            zoom_at(app, (scroll * 0.004).exp(), r.hover_pos());
        }
    }
    let span = size / app.minimap_zoom;
    if r.dragged_by(egui::PointerButton::Secondary) || r.dragged_by(egui::PointerButton::Middle) {
        let d = r.drag_delta();
        app.minimap_center[0] -= d.x / rect.width() * span;
        app.minimap_center[1] += d.y / rect.height() * span;
    } else if app.minimap_zoom > 1.01 && !r.dragged() && app.minimap_camera != cam {
        // Zoomed in, the map follows a moving camera once it would leave the map; one moved
        // away by hand stays where it was put while the camera stands still.
        let (dx, dz) = (cam[0] - app.minimap_center[0], cam[2] - app.minimap_center[1]);
        if dx.abs() > span * 0.45 || dz.abs() > span * 0.45 {
            app.minimap_center = [cam[0], cam[2]];
        }
    }
    app.minimap_camera = cam;
    let half = span * 0.5;
    app.minimap_center = [app.minimap_center[0].clamp(half, size - half), app.minimap_center[1].clamp(half, size - half)];
    let (x0, z1) = (app.minimap_center[0] - half, app.minimap_center[1] + half);

    // The terrain, its surfaces and the objects, as the top view shows them.
    let painter = ui.painter_at(view);
    painter.rect_filled(rect.expand(2.0), 3.0, Color32::from_black_alpha(160));
    let v = View::top(app.minimap_center, span / side, rect);
    let objects = prepare_objects_for(app, &v, true, 2);
    let ctx = ui.ctx().clone();
    if let Some(doc) = &app.doc {
        let quality = app.settings.terrain_quality;
        let meta = &doc.world.meta;
        let frame = Frame {
            view: v,
            viewport_key: 2,
            patches: Arc::new(doc.lod.select(&v, quality)),
            ranges: Arc::new(doc.lod.ranges(quality)),
            shading: app.shading2,
            wireframe: false,
            brush: None,
            grid: [0.0; 3],
            sea: meta.sea_level,
            height_range: doc.height_range,
            objects,
            show_water: app.show_water,
            surface_range: app.settings.surface_range,
            parallax: 0.0,
            blend_sharp: app.settings.surface_blend.clamp(0.0, 1.0),
            sea_waves: app.settings.sea_waves,
            tide: tide(app, &ctx),
            world: meta.size_m(),
            tile: 10.0 / meta.land_cell(),
            lighting: app.settings.lighting.gpu(),
            time: ctx.input(|i| i.time) as f32,
            light: None,
            shadow_bias: shadow_bias(app.settings.lighting.shadow_range),
            ao: 0,
            smoothing: app.settings.edge_smoothing,
        };
        ui.painter_at(rect).add(render::callback(app.renderer.clone(), frame));
    }

    let to_map = |x: f32, z: f32| pos2(rect.left() + (x - x0) / span * rect.width(), rect.top() + (z1 - z) / span * rect.height());
    let clip = painter.with_clip_rect(rect);
    let c = to_map(cam[0], cam[2]);
    let f = app.camera.forward();
    let heading = f[0].atan2(f[2]);
    let fov = app.camera.fov * 0.5;
    let reach = side * 0.18;
    let ray = |a: f32| c + vec2(a.sin(), -a.cos()) * reach;
    let col = Color32::from_rgb(255, 210, 60);
    clip.add(egui::Shape::convex_polygon(vec![c, ray(heading - fov), ray(heading + fov)], Color32::from_rgba_unmultiplied(255, 210, 60, 50), Stroke::new(1.0_f32, col)));
    clip.circle(c, 3.5, col, Stroke::new(1.0_f32, Color32::BLACK));
    painter.rect_stroke(rect, 3.0, Stroke::new(1.0_f32, Color32::from_gray(120)), egui::StrokeKind::Outside);
    if app.minimap_zoom > 1.01 {
        let text = if span >= 1000.0 { format!("{:.1} km", span / 1000.0) } else { format!("{span:.0} m") };
        painter.text(rect.left_bottom() + vec2(4.0, -3.0), egui::Align2::LEFT_BOTTOM, text, egui::FontId::proportional(10.0), Color32::WHITE);
    }

    // The grip in the top right corner: dragged up and right the map grows.
    let grip = Rect::from_min_size(pos2(rect.right() - 14.0, rect.top()), vec2(14.0, 14.0));
    let gr = ui.interact(grip, ui.id().with("minimap_grip"), Sense::drag()).on_hover_text(t("Drag to make the minimap bigger or smaller."));
    if gr.hovered() || gr.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNeSw);
    }
    if gr.dragged() {
        let d = gr.drag_delta();
        app.settings.minimap_size = (app.settings.minimap_size + (d.x - d.y) * 0.5 * 2.0_f32.sqrt()).clamp(100.0, most);
    }
    if gr.drag_stopped() {
        app.settings.save();
    }
    let lit = gr.hovered() || gr.dragged();
    let gc = if lit { theme::ACCENT } else { Color32::from_gray(200) };
    for k in 0..3 {
        let o = 3.0 + k as f32 * 4.0;
        painter.line_segment([pos2(grip.right() - o, grip.top() + 1.0), pos2(grip.right() - 1.0, grip.top() + o)], Stroke::new(1.2_f32, gc));
    }
    app.minimap_hover |= lit;

    // Zoom buttons down the right side, over the map.
    let mut zoom_by = None;
    let mut reset = false;
    for (k, (label, tip)) in [("+", t("Zoom in")), ("\u{2212}", t("Zoom out")), ("\u{27F2}", t("The whole map"))].into_iter().enumerate() {
        let b = Rect::from_min_size(pos2(rect.right() - 20.0, rect.top() + 18.0 + k as f32 * 20.0), vec2(18.0, 18.0));
        let br = ui.interact(b, ui.id().with(("minimap_button", k)), Sense::click()).on_hover_text(tip);
        painter.rect_filled(b, 3.0, if br.hovered() { theme::ACCENT } else { Color32::from_black_alpha(170) });
        painter.text(b.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(13.0), Color32::WHITE);
        if br.clicked() {
            match k {
                0 => zoom_by = Some(2.0),
                1 => zoom_by = Some(0.5),
                _ => reset = true,
            }
        }
        app.minimap_hover |= br.hovered();
    }
    if let Some(f) = zoom_by {
        zoom_at(app, f, None);
        return;
    }
    if reset {
        app.minimap_zoom = 1.0;
        app.minimap_center = [size * 0.5, size * 0.5];
        return;
    }
    if gr.dragged() || gr.hovered() {
        return;
    }
    let r = r.on_hover_text(t("Click or drag to take the camera there; the wheel zooms, the right button moves the map."));
    if (r.clicked() || r.dragged_by(egui::PointerButton::Primary)) && !app.minimap_hover_buttons(rect, r.interact_pointer_pos()) {
        if let Some(m) = r.interact_pointer_pos() {
            let x = (x0 + (m.x - rect.left()) / rect.width() * span).clamp(0.0, size);
            let z = (z1 - (m.y - rect.top()) / rect.height() * span).clamp(0.0, size);
            // The camera keeps its height over the ground.
            let over = cam[1] - ground_here;
            let Some(doc) = &app.doc else { return };
            let ground = doc.world.height.sample(x, z);
            app.camera.pos = [x, ground + over.max(2.0), z];
            app.top_center = [x, z];
        }
    }
}

impl App {
    /// True when the pointer is on the zoom buttons of the minimap rather than on the map.
    fn minimap_hover_buttons(&self, rect: Rect, at: Option<Pos2>) -> bool {
        at.is_some_and(|p| p.x >= rect.right() - 22.0 && p.y <= rect.top() + 80.0)
    }
}

/// The two halves of the split view and the bar between them, which is dragged to share
/// the room out differently and double-clicked to share it evenly.
fn split_view(app: &mut App, ui: &mut egui::Ui, rect: Rect) -> (Rect, Rect) {
    const BAR: f32 = 6.0;
    let stacked = app.settings.split_stacked;
    let ratio = app.settings.split_ratio.clamp(0.15, 0.85);
    let (first, bar, second) = if stacked {
        let at = rect.top() + rect.height() * ratio;
        (Rect::from_min_max(rect.min, pos2(rect.right(), at - BAR * 0.5)), Rect::from_min_max(pos2(rect.left(), at - BAR * 0.5), pos2(rect.right(), at + BAR * 0.5)), Rect::from_min_max(pos2(rect.left(), at + BAR * 0.5), rect.max))
    } else {
        let at = rect.left() + rect.width() * ratio;
        (Rect::from_min_max(rect.min, pos2(at - BAR * 0.5, rect.bottom())), Rect::from_min_max(pos2(at - BAR * 0.5, rect.top()), pos2(at + BAR * 0.5, rect.bottom())), Rect::from_min_max(pos2(at + BAR * 0.5, rect.top()), rect.max))
    };
    let r = ui.interact(bar, ui.id().with("split_bar"), Sense::click_and_drag());
    if r.hovered() || r.dragged() {
        ui.ctx().set_cursor_icon(if stacked { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal });
    }
    if r.dragged() {
        if let Some(p) = r.interact_pointer_pos() {
            app.settings.split_ratio = if stacked { (p.y - rect.top()) / rect.height().max(1.0) } else { (p.x - rect.left()) / rect.width().max(1.0) }.clamp(0.15, 0.85);
        }
    }
    if r.drag_stopped() {
        app.settings.save();
    }
    if r.double_clicked() {
        app.settings.split_ratio = 0.5;
        app.settings.save();
    }
    let lit = r.hovered() || r.dragged();
    ui.painter().rect_filled(bar, 0.0, if lit { theme::ACCENT } else { Color32::from_gray(40) });
    (first, second)
}

/// Runs the commands of `DZWE_AUTO`, separated by `;`, used for automated checks.
pub fn run_script(app: &mut App, ctx: &egui::Context) {
    if app.autoscript.is_none() {
        return;
    }
    ctx.request_repaint();
    if app.task.is_some() {
        return;
    }
    let script = app.autoscript.as_mut().unwrap();
    if script.wait > 0 {
        script.wait -= 1;
        return;
    }
    if script.steps.is_empty() {
        app.autoscript = None;
        return;
    }
    let step = script.steps.remove(0);
    let mut parts = step.trim().splitn(2, ' ');
    let (cmd, arg) = (parts.next().unwrap_or(""), parts.next().unwrap_or("").trim().to_string());
    let nums: Vec<f32> = arg.split_whitespace().filter_map(|v| v.parse().ok()).collect();
    match cmd {
        "wait" => app.autoscript.as_mut().unwrap().wait = arg.parse().unwrap_or(10),
        "open" => app.open_project(arg.into()),
        "importscan" => app.scan_for_import(arg.into()),
        "importgo" => {
            if let crate::app::Dialog::Import(d) = std::mem::replace(&mut app.dialog, crate::app::Dialog::None) {
                app.run_import(d.plan);
            }
        }
        "layers" => {
            if let Some(d) = &app.doc {
                let n = d.world.objects.layer_counts(d.world.object_layers.len());
                for (i, l) in d.world.object_layers.iter().enumerate() {
                    eprintln!("LAYER {i} {:?} objects={} visible={} editable={} export={} deleted={}", l.name, n[i], l.visible, l.editable, l.export, l.deleted);
                }
                eprintln!("LAYERS total={} selection={} active={}", d.world.objects.len(), app.selection.len(), d.world.active_layer);
            }
        }
        "shapes" => {
            if let Some(d) = &app.doc {
                for l in &d.world.shape_layers {
                    eprintln!("SHAPES {:?} visible={} shapes={} points={}", l.name, l.visible, l.shapes.len(), l.shapes.iter().map(|s| s.points.len()).sum::<usize>());
                }
                eprintln!("SHAPES total={} selected={:?} vertex={:?}", d.world.shape_layers.len(), app.shape.selection, app.shape.vertex);
            }
        }
        "shapetool" => {
            app.shape.tool = match arg.as_str() {
                "polygon" => ShapeTool::Polygon,
                "polyline" => ShapeTool::Polyline,
                "point" => ShapeTool::Point,
                _ => ShapeTool::Edit,
            };
        }
        "shapeexport" => app.write_shapes(std::path::Path::new(&arg)),
        "shapeimport" => app.load_shape_files(vec![arg.into()]),
        "grassunder" => app.grass_under_objects = arg == "1",
        "mission" => crate::mission_ui::script(app, &arg),
        "layer" => {
            use crate::app::LayerAction as A;
            let mut it = arg.split_whitespace();
            let what = it.next().unwrap_or("");
            let l: u16 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            match what {
                "visible" => app.layer_action(A::Visible(l)),
                "editable" => app.layer_action(A::Editable(l)),
                "solo" => app.layer_action(A::Solo(l)),
                "active" => app.layer_action(A::Active(l)),
                "select" => app.layer_action(A::Select(l)),
                "move" => app.layer_action(A::MoveSelection(l)),
                "editall" => app.layer_action(A::EditAll),
                "showall" => app.layer_action(A::ShowAll),
                "new" => app.layer_action(A::New),
                "delete" => app.layer_action(A::Delete(l)),
                "deleteall" => app.layer_action(A::DeleteWithObjects(l)),
                _ => eprintln!("unknown layer command {what}"),
            }
        }
        "wrp" => app.import_wrp_file(std::path::PathBuf::from(arg)),
        "demo" => {
            let mut w = dz_core::world::World::new(dz_core::world::WorldMeta::new("Demo", 2048, 5.0), 60.0);
            let params = gen::TerrainParams { kind: gen::TerrainKind::Island, ..Default::default() };
            app.spawn("demo", move |job| {
                gen::generate_terrain(&mut w.base, &params, job);
                w.recompose(None);
                let rules = gen::default_paint_rules(&w);
                w.mask.data = gen::auto_paint(&w, &rules, 1, None, job);
                Ok(Box::new(move |a: &mut App| a.open_world(w, None)) as Finish)
            });
        }
        "mode" => {
            app.mode = match arg.as_str() {
                "sculpt" => Mode::Landscape,
                "paint" => Mode::Paint,
                "satellite" => Mode::Satellite,
                "place" => Mode::Place,
                "lines" => Mode::Lines,
                "shapes" => Mode::Shapes,
                "keypoints" => Mode::KeyPoints,
                "mission" => Mode::Mission,
                "gen" => Mode::Generate,
                _ => Mode::Select,
            }
        }
        "view" => {
            app.view_mode = match arg.as_str() {
                "top" => ViewMode::Top,
                "split" => ViewMode::Split,
                _ => ViewMode::Perspective,
            }
        }
        "shade" => {
            let s = Shading::ALL.into_iter().find(|s| s.name().eq_ignore_ascii_case(&arg)).unwrap_or(Shading::Lit);
            app.shading3 = s;
            app.shading2 = s;
        }
        "cam" if nums.len() >= 5 => {
            let ground = if nums.get(5) == Some(&1.0) { app.doc.as_ref().map(|d| d.world.height.sample(nums[0], nums[2])).unwrap_or(0.0) } else { 0.0 };
            app.camera.pos = [nums[0], nums[1] + ground, nums[2]];
            app.camera.yaw = nums[3].to_radians();
            app.camera.pitch = nums[4].to_radians();
        }
        "top" if nums.len() >= 3 => {
            app.top_center = [nums[0], nums[1]];
            app.top_mpp = nums[2];
        }
        "light" if nums.len() >= 2 => {
            app.settings.lighting.azimuth = nums[0];
            app.settings.lighting.elevation = nums[1];
        }
        "shadowrange" if nums.len() == 1 => app.settings.lighting.shadow_range = nums[0],
        "lang" => app.sync_language(&arg),
        "bottom" => app.bottom = if arg == "output" { crate::app::BottomTab::Output } else if arg == "build" { crate::app::BottomTab::Build } else { crate::app::BottomTab::Content },
        "sculpt" if nums.len() >= 2 => {
            let brush = Brush { radius: 200.0, falloff: 0.7, strength: 30.0 };
            let w = &mut app.doc.as_mut().unwrap().world;
            let mut s = SculptStroke::begin(SculptTool::Raise, brush, w, nums[0], nums[1]);
            let r = s.apply(w, nums[0], nums[1], 1.0);
            if let (Some(r), Some(c)) = (r, s.finish()) {
                app.doc.as_mut().unwrap().history.push("sculpt", c);
                app.height_changed(r);
            }
        }
        "shot" => {
            app.pending_shot(arg, ctx);
        }
        // The window of its own the view is in.
        "shotview" => {
            app.shot_path_set(arg);
            ctx.send_viewport_cmd_to(egui::ViewportId::from_hash_of("dzwe_view"), egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        // A plain satellite image with a grid, to try the satellite tools on.
        "satnew" => {
            if let Some(d) = app.doc.as_mut() {
                let n = nums.first().map(|v| *v as usize).unwrap_or(2048);
                let mut img = dz_core::terrain::RgbImage::new(n, n, [96, 118, 72]);
                for y in 0..n {
                    for x in 0..n {
                        let o = (y * n + x) * 3;
                        let shade = ((x / 8 + y / 8) % 2) as u8 * 10 + ((x * 31 + y * 17) % 7) as u8;
                        let line = x % 64 == 0 || y % 64 == 0;
                        img.data[o..o + 3].copy_from_slice(&if line { [60, 70, 50] } else { [96 + shade, 118 + shade, 72 + shade] });
                    }
                }
                d.world.satellite = Some(img);
                d.world.sat_image_replaced();
                app.renderer.lock().unwrap().set_satellite(d.world.satellite.as_ref());
                app.detail_center = [f32::MIN, f32::MIN];
            }
        }
        // `prefabmake`: a prefab of the selection; `prefabplace x z`: the last prefab placed there.
        "prefabmake" => { app.create_prefab(); }
        "prefabplace" if nums.len() >= 2 => {
            if let Some(id) = app.library.prefabs.last().map(|p| p.id) {
                let ids = app.place_prefab(id, nums[0], nums[1]);
                eprintln!("PREFAB placed {} objects", ids.len());
            }
        }
        "lassomode" => app.settings.lasso = arg == "1",
        "objpos" => {
            if let Some(d) = &app.doc {
                eprintln!("OBJCOUNT {}", d.world.objects.len());
                for (id, o) in d.world.objects.iter().take(8) { eprintln!("OBJPOS {id} {:.2} {:.2} yaw {:.1} pitch {:.2} roll {:.2} scale {:.2}", o.x, o.z, o.yaw, o.pitch, o.roll, o.scale); }
            }
        }
        "shapepts" => eprintln!("SHAPEPTS {:?}", app.shape.points),
        "caminfo" => eprintln!("CAMINFO pos={:?} minimap zoom={:.2} center={:?}", app.camera.pos, app.minimap_zoom, app.minimap_center),
        "cfgscene" => {
            let (p, f) = (app.camera.pos, app.camera.forward());
            if let Some(d) = app.doc.as_mut() {
                let g = d.world.height.sample(p[0], p[2]);
                let n = d.world.meta.config.scenes.len() + 1;
                d.world.meta.config.scenes.push(dz_core::world::CharacterScene { name: format!("loc{n}"), position: [p[0], p[2], p[1] - g], target: [p[0] + f[0] * 60.0, p[2] + f[2] * 60.0, p[1] + f[1] * 60.0], fov: 0.6, ..Default::default() });
                d.world.meta.config.start_time = "6:30".into();
            }
        }
        "mapconfig" => app.dialog = crate::app::Dialog::MapConfig,
        "aligntoground" => app.settings.align_to_ground = arg == "1",
        "shottask" => app.shot_during_task = Some(arg),
        "selcount" => eprintln!("SELCOUNT {}", app.selection.len()),
        "fullscreen" => crate::panels::set_fullscreen(app, ctx, arg == "1"),
        "edge" => { if let Ok(k) = arg.parse::<usize>() { if k < 4 { app.edge_open[k] = true; } } }
        "detach" => app.detached = arg == "1",
        "selall" => {
            if let Some(d) = &app.doc {
                app.selection = d.world.objects.iter().map(|(id, _)| id).collect();
                app.selection_rev += 1;
            }
        }
        "libdemo" => crate::asset_browser::library_demo(app, &arg),
        "satbake" => {
            app.sat.bake_selection = false;
            app.sat.bake_plants = nums.first().is_some_and(|v| *v > 0.0);
            app.sat.bake_roads = nums.get(1).is_some_and(|v| *v > 0.0);
            app.sat.bake_water = nums.get(2).is_some_and(|v| *v > 0.0);
            app.sat.bake_above_ground = nums.get(3).is_none_or(|v| *v > 0.0);
            crate::satellite::bake(app);
        }
        // Source point in metres, then the ends of a stroke in metres.
        "satclone" if nums.len() >= 6 => {
            if let Some((w, size)) = app.doc.as_ref().and_then(|d| d.world.satellite.as_ref().map(|s| (s.width as f32, d.world.meta.size_m()))) {
                app.sat.source = Some([nums[0] / size * w, (1.0 - nums[1] / size) * w]);
                crate::satellite::begin_stroke(app, nums[2], nums[3]);
                crate::satellite::continue_stroke(app, nums[4], nums[5]);
                crate::satellite::finish_stroke(app);
            }
        }
        "satradius" if !nums.is_empty() => app.sat.radius = nums[0],
        "satdel" => {
            let touched = app.doc.as_mut().and_then(|d| {
                let id = d.world.sat.modifiers.get(nums.first().map(|v| *v as usize).unwrap_or(0))?.id;
                d.world.sat_remove(id)
            });
            if let Some(r) = touched {
                app.satellite_changed(r);
            }
            eprintln!("GENSTATS satmods={}", app.doc.as_ref().map_or(0, |d| d.world.sat.modifiers.len()));
        }
        // Stamps surface `n` under every object, buildings by their footprint.
        "maskunder" => {
            app.mask_tool.surface = nums.first().map(|v| *v as usize).unwrap_or(0);
            app.mask_tool.selection_only = false;
            app.mask_tool.buildings = true;
            if nums.get(1).is_some_and(|v| *v > 0.0) {
                app.mask_tool.stamp.shape = dz_core::maskmod::StampShape::Geometry;
                app.mask_tool.stamp.size_m = nums.get(2).copied().unwrap_or(1.0);
            }
            crate::maskmods::stamp_under_objects(app);
            eprintln!("GENSTATS maskmods={}", app.doc.as_ref().map_or(0, |d| d.world.mask_mods.modifiers.len()));
        }
        "maskstat" => {
            if let Some(d) = &app.doc {
                for m in &d.world.mask_mods.modifiers {
                    eprintln!("GENSTATS maskmod {:?} {:?} tiles={} enabled={}", m.name, m.kind, m.tiles.len(), m.enabled);
                }
                eprintln!("GENSTATS maskmods={}", d.world.mask_mods.modifiers.len());
            }
        }
        // A rectangle as a shape, and the last generator pointed at it.
        "genshape" if nums.len() >= 4 => {
            if let Some(d) = app.doc.as_mut() {
                if d.world.shape_layers.is_empty() {
                    d.world.shape_layers.push(dz_core::shapes::ShapeLayer { name: "test".into(), visible: true, shapes: Vec::new() });
                }
                let l = &mut d.world.shape_layers[0];
                l.shapes.push(dz_core::shapes::MapShape { kind: dz_core::shapes::ShapeKind::Polygon, points: vec![[nums[0], nums[1]], [nums[2], nums[1]], [nums[2], nums[3]], [nums[0], nums[3]]], attributes: Vec::new() });
                let target = dz_core::library::GenTarget::Shape { layer: l.name.clone(), index: l.shapes.len() - 1 };
                if let Some(g) = d.world.generators.last_mut() {
                    g.target = target;
                }
            }
        }
        "genterrain" => {
            if let Some(dz_core::library::GeneratorKind::Terrain(p)) = app.doc.as_mut().and_then(|d| d.world.generators.last_mut()).map(|g| &mut g.kind) {
                if let Some(k) = gen::TerrainKind::ALL.into_iter().find(|k| k.name().to_ascii_lowercase().starts_with(&arg.to_ascii_lowercase())) {
                    p.kind = k;
                }
                p.base = 260.0;
                p.amplitude = 380.0;
                p.feature = 900.0;
            }
        }
        "maskdel" => {
            if let Some(id) = app.doc.as_ref().and_then(|d| d.world.mask_mods.modifiers.first().map(|m| m.id)) {
                crate::maskmods::remove(app, id, "test");
            }
        }
        "put" if nums.len() >= 3 => {
            if let Some(d) = app.doc.as_mut() {
                let path = arg.split_whitespace().nth(3).unwrap_or("").to_string();
                let model = d.world.objects.intern(&path);
                d.world.objects.insert(dz_core::objects::Obj { x: nums[0], z: nums[1], elevation: 0.0, yaw: nums[2].to_radians(), pitch: 0.0, roll: 0.0, scale: 1.0, model, flags: 0 });
            }
        }
        "mouse" if nums.len() >= 2 => app.synthetic.push(egui::Event::PointerMoved(pos2(nums[0], nums[1]))),
        "press" | "release" if nums.len() >= 2 => {
            let button = if nums.len() >= 3 && nums[2] == 1.0 { PointerButton::Secondary } else { PointerButton::Primary };
            app.synthetic.push(egui::Event::PointerButton { pos: pos2(nums[0], nums[1]), button, pressed: cmd == "press", modifiers: egui::Modifiers::NONE });
        }
        "key" => {
            // `key <logical> [physical]`, with the modifiers set by `mods`.
            let mut names = arg.split_whitespace();
            if let Some(key) = names.next().and_then(Key::from_name) {
                let physical_key = names.next().and_then(Key::from_name);
                let modifiers = app.synthetic_modifiers;
                app.synthetic.push(egui::Event::Key { key, physical_key, pressed: true, repeat: false, modifiers });
                app.synthetic.push(egui::Event::Key { key, physical_key, pressed: false, repeat: false, modifiers });
            }
        }
        "wheel" if !nums.is_empty() => app.synthetic.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, nums[0]), modifiers: app.synthetic_modifiers }),
        // `brushpic <file>`: the paint and sculpt brushes use this picture, read where it is.
        "brushpic" => match crate::brushes::load(std::path::Path::new(&arg)) {
            Ok(b) => {
                app.paint.picture = Some(b.name.clone());
                app.sculpt.picture = Some(b.name.clone());
                app.brushes.push(b);
            }
            Err(e) => eprintln!("BRUSHPIC {e}"),
        },
        "spray" => {
            let d = nums.first().copied().unwrap_or(0.0);
            app.paint.spray = d > 0.0;
            app.sculpt.spray = d > 0.0;
            app.paint.spray_density = d.max(0.01);
            app.sculpt.spray_density = d.max(0.01);
        }
        "fold" => {
            app.outliner_unfolded = arg != "0";
            app.outliner_fold = Some(app.outliner_unfolded);
        }
        "relief" if nums.len() >= 2 => {
            app.settings.surface_parallax = nums[0];
            app.settings.surface_blend = nums[1];
        }
        "brushinfo" => eprintln!("BRUSH sculpt={:.2}/{:.2} paint={:.2} holes={:.2} place={:.2}/{:.2} sat={:.2}/{:.2}", app.sculpt.radius, app.sculpt.strength, app.paint.radius, app.hole_radius, app.place.brush_radius, app.place.brush_spacing, app.sat.radius, app.sat.opacity),
        "mods" => {
            app.synthetic_modifiers = egui::Modifiers { shift: arg.contains("shift"), ctrl: arg.contains("ctrl"), command: arg.contains("ctrl"), alt: arg == "alt", ..Default::default() };
        }
        "mmove" if nums.len() >= 2 => app.synthetic.push(egui::Event::PointerMoved(pos2(nums[0], nums[1]))),
        "mbutton" if nums.len() >= 2 => {
            // "mbutton l 1 x y": which button, pressed or released, where.
            let button = if arg.starts_with('r') { PointerButton::Secondary } else { PointerButton::Primary };
            let at = pos2(nums[1], nums[2]);
            app.synthetic.push(egui::Event::PointerMoved(at));
            app.synthetic.push(egui::Event::PointerButton { pos: at, button, pressed: nums[0] != 0.0, modifiers: app.synthetic_modifiers });
        }
        "winsize" if nums.len() >= 2 => ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(nums[0], nums[1]))),
        // How clicks lay a road: points, live or parts; `roadpiece <name part>` picks a piece.
        "roaddraw" => {
            app.mode = Mode::Lines;
            app.line.action = LineAction::Road;
            app.line.draw = match arg.as_str() { "live" => RoadDraw::Live, "parts" => RoadDraw::Parts, _ => RoadDraw::Points };
        }
        "roadpiece" => {
            if let Some(c) = crate::road_draw::choices(&app.road_catalog, &app.line.family).into_iter().find(|c| c.path.to_lowercase().contains(&arg.to_lowercase())) { app.road_parts.next = Some(c.path); }
        }
        "roadstate" => {
            eprintln!("ROADSTATE points={} live={:?} parts={} open={}", app.line.points.len(), app.live.shown.as_ref().map(|(k, r)| (k.len(), r.as_ref().map(|p| p.len()).map_err(|e| e.clone()))), app.road_parts.parts.len(), app.road_parts.open.is_some());
            if let Some(d) = &app.doc { eprintln!("ROADSTATE routes={} objects={}", d.world.routes.len(), d.world.objects.len()); }
        }
        // Only the last object selected, as a plain click on it with whole selection off.
        "selone" => {
            if let Some(id) = app.doc.as_ref().and_then(|d| d.world.objects.iter().map(|(id, _)| id).last()) {
                app.selection = [id].into_iter().collect();
                app.leave_world_details();
                app.selection_rev += 1;
            }
        }
        "selinfo" => {
            if let Some(d) = &app.doc {
                let first = app.selection.iter().next().and_then(|i| d.world.objects.get(*i));
                eprintln!("GENSTATS selection n={} whole={:?} first={:?}", app.selection.len(), app.whole, first.map(|o| (d.world.objects.model_path(o).rsplit('\\').next().unwrap_or("").to_string(), o.yaw, o.scale)));
            }
        }
        "model" => app.place.model = Some(arg),
        "undo" => app.undo(),
        "pad" => { let f = app.place.pad_falloff; app.add_pads_for_selection(f); }
        "gen" => match arg.as_str() {
            "road" => { app.mode = Mode::Lines; app.line.action = LineAction::Road; }
            "paint" => app.paint.layer = 3,
            _ => {}
        },
        "goto" if nums.len() >= 2 => {
            if let Some(d) = &app.doc {
                let trees: Vec<_> = d.world.objects.iter().filter(|(_, o)| d.world.objects.model_path(o).contains("\\tree\\")).map(|(_, o)| *o).collect();
                if let Some(o) = trees.get(nums[0] as usize) {
                    let g = d.world.height.sample(o.x, o.z);
                    app.camera.pos = [o.x - nums[1], g + nums[1] * 0.25 + 3.0, o.z - nums[1]];
                    app.camera.look_at([o.x, g + 8.0, o.z]);
                    eprintln!("GOTO {}", d.world.objects.model_path(o));
                }
            }
        }
        "perf" => {
            if arg == "start" {
                app.perf = Some(Vec::new());
            } else if let Some(mut v) = app.perf.take() {
                v.sort_by(|a, b| a.total_cmp(b));
                let n = v.len().max(1);
                eprintln!("PERF {arg}: frames={} avg={:.2}ms p50={:.2} p95={:.2} p99={:.2} max={:.2}", v.len(), v.iter().sum::<f32>() / n as f32, v[n / 2], v[n * 95 / 100], v[n * 99 / 100], v[n - 1]);
            }
        }
        "save" => { app.save(Some(arg.into())); }
        "build" => app.build(false),
        "browse" => app.filter = arg,
        // Satellite colours across a line of the map, "satpx x z0 z1".
        "satpx" if nums.len() >= 3 => {
            if let Some(d) = &app.doc {
                if let Some(img) = &d.world.satellite {
                    let size = d.world.meta.size_m();
                    let mut z = nums[1];
                    while z <= nums[2] {
                        let (px, py) = ((nums[0] / size * img.width as f32) as usize, ((size - z) / size * img.height as f32) as usize);
                        let o = (py.min(img.height - 1) * img.width + px.min(img.width - 1)) * 3;
                        eprintln!("SATPX z={z:.1} rgb={:?}", &img.data[o..o + 3]);
                        z += 1.0;
                    }
                }
                if let Some(r) = d.world.routes.first() {
                    let near = r.centerline.iter().min_by(|a, b| (a[0] - nums[0]).abs().total_cmp(&(b[0] - nums[0]).abs()));
                    eprintln!("SATPX centre {near:?}");
                }
            }
        }
        // Asks to delete the first terrain modifier, mask modifier, generator or route.
        "askdelete" => {
            use crate::deleting::Target;
            let target = app.doc.as_ref().and_then(|d| match arg.as_str() {
                "mask" => d.world.mask_mods.modifiers.first().map(|m| Target::MaskModifier(m.id)),
                "generator" => d.world.generators.first().map(|g| Target::Generator(g.id)),
                "route" => d.world.routes.first().map(|r| Target::Route(r.id)),
                _ => d.world.modifiers.first().map(|m| Target::Modifier(m.id)),
            });
            if let Some(t) = target { crate::deleting::request(app, t); }
            eprintln!("GENSTATS modifiers={} objects={}", app.doc.as_ref().map_or(0, |d| d.world.modifiers.len()), app.doc.as_ref().map_or(0, |d| d.world.objects.len()));
        }
        "browser" | "browsersel" | "browseradd" | "browserdrop" | "browsertemplates" | "browsertpladd" => crate::asset_browser::script(app, cmd, &arg),
        "log" => {
            for l in &app.log { eprintln!("LOG {}", l.text); }
        }
        // The whole of route `n` selected, as a click on one of its parts does with whole selection on.
        "selroute" => {
            if let Some(r) = app.doc.as_ref().and_then(|d| d.world.routes.get(nums.first().map(|v| *v as usize).unwrap_or(0))) {
                app.selection = r.objects.iter().copied().collect();
                app.whole = Some(SelectedWhole::Route(r.id));
                app.selection_rev += 1;
            }
        }
        "selgen" => {
            if let Some(g) = app.doc.as_ref().and_then(|d| d.world.generators.get(nums.first().map(|v| *v as usize).unwrap_or(0))) {
                app.selection = g.objects.iter().copied().collect();
                app.whole = Some(SelectedWhole::Generator(g.id));
                app.selection_rev += 1;
            }
        }
        "geninfo" => {
            if let Some(d) = &app.doc {
                for g in &d.world.generators {
                    eprintln!("GENINFO {} objects={} target={:?} modifier={:?}", g.name, g.objects.len(), g.target, g.modifier);
                }
                eprintln!("GENINFO shapes={:?} undo={:?}", d.world.shape_layers.iter().map(|l| l.shapes.len()).collect::<Vec<_>>(), d.history.undo_label());
            }
        }
        "dup" => app.duplicate_selection(),
        // Moves the selection by (dx, dz) as a drag of the gizmo does.
        "dragsel" if nums.len() >= 2 => {
            if let Some(d) = app.doc.as_mut() {
                let before: Vec<(u32, Obj)> = app.selection.iter().filter_map(|id| Some((*id, *d.world.objects.get(*id)?))).collect();
                for (id, b) in &before {
                    let mut o = *b;
                    o.x += nums[0];
                    o.z += nums[1];
                    d.world.objects.update(*id, o);
                    d.world.sync_attached(*id);
                }
                let pivot = before.first().map(|(_, o)| [o.x, 0.0, o.z]).unwrap_or([0.0; 3]);
                app.drag = Some(ObjectDrag { toggle: None, started: true, handle: Handle::Plane, before, pivot, start: pos2(0.0, 0.0), axis: (vec2(0.0, 0.0), [0.0; 3], 0.0), grab_offset: [0.0, 0.0] , virt: None, click: None, stepped: false, mods: egui::Modifiers::NONE });
                let _ = end_object_drag(app);
            }
        }
        "routeinfo" => {
            if let Some(d) = &app.doc {
                for r in &d.world.routes {
                    let m = r.modifier.and_then(|id| d.world.modifiers.iter().find(|m| m.id == id)).map(|m| match &m.shape { dz_core::world::ModShape::Line { points, .. } => points.first().copied().unwrap_or_default(), _ => [m.x, m.z] });
                    // How far the parts are from the line, which a move must keep together.
                    let off = r.objects.iter().filter_map(|id| d.world.objects.get(*id)).map(|o| r.centerline.windows(2).map(|w| {
                        let (a, b) = (w[0], w[1]);
                        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                        let t = (((o.x - a[0]) * dx + (o.z - a[1]) * dz) / (dx * dx + dz * dz).max(1e-6)).clamp(0.0, 1.0);
                        (o.x - a[0] - dx * t).hypot(o.z - a[1] - dz * t)
                    }).fold(f32::MAX, f32::min)).fold(0.0f32, f32::max);
                    eprintln!("ROUTEINFO {} objects={} first={:?} modifier_first={:?} parts_off_line={off:.2}", r.name, r.objects.len(), r.centerline.first(), m);
                }
                eprintln!("ROUTEINFO modifiers={} objects={} undo={:?}", d.world.modifiers.len(), d.world.objects.len(), d.history.undo_label());
            }
        }
        "routefamily" => app.line.family = arg,
        "routecenter" => {
            if let Some(mid) = app.doc.as_ref().and_then(|d| d.world.routes.first()).map(|r| r.centerline[r.centerline.len() / 2]) {
                app.top_center = mid;
                app.top_mpp = 0.25;
            }
        }
        "placeat" if nums.len() >= 2 => {
            if let Some(m) = app.place.model.clone() { place_model(app, &m, nums[0], nums[1]); }
            // A third number: the elevation over the ground, to sink or raise it.
            if let (Some(e), Some(d)) = (nums.get(2), app.doc.as_mut()) {
                if let Some(id) = d.world.objects.iter().map(|(id, _)| id).max() {
                    let mut o = *d.world.objects.get(id).unwrap();
                    o.elevation = *e;
                    d.world.objects.update(id, o);
                }
            }
        }
        "modifiers" => {
            if let Some(d) = &app.doc {
                eprintln!("MODIFIERS total={} ids={:?} height0={}", d.world.modifiers.len(), d.world.routes.iter().map(|r| r.modifier).collect::<Vec<_>>(), d.world.height.sample(2350.0, 6400.0));
            }
        }
        "railgrade" => app.line.rail_routing.max_grade = nums.first().copied().unwrap_or(3.0) as f64,
        "routemode" => app.line.route = arg != "fit",
        "routetool" => app.line.action = LineAction::Edit,
        "proxies" => app.show_proxies = arg == "1",
        "experimental" => app.settings.experimental = arg == "1",
        // Text typed into the focused field.
        "type" => app.synthetic.push(egui::Event::Text(arg)),
        // Team work: `team` opens its window, `teamname n`, `teamstart folder`, `teamsend`,
        // `teammerge` reads the oldest waiting changes and `teammergego` merges them.
        "team" => {
            app.team.open = true;
            crate::team_ui::refresh(app);
        }
        "teamname" => app.settings.team_name = arg,
        "teamstart" => crate::team_ui::start(app, arg),
        "teamsend" => crate::team_ui::send(app),
        "teammerge" => {
            crate::team_ui::refresh(app);
            if let Some((f, _)) = crate::team_ui::first_waiting(app) {
                crate::team_ui::begin_merge(app, f);
            }
        }
        "teammergego" => crate::team_ui::finish_merge(app),
        // `teamgive name` gives the selected objects to a layer of `name`.
        "teamgive" => crate::team_ui::give_selection(app, arg),
        // `camproxy n`: the camera 2.5 m from proxy n of the first object, a little above it,
        // looking at it.
        "camproxy" => {
            let n = nums.first().map(|v| *v as usize).unwrap_or(0);
            let found = app.doc.as_ref().and_then(|d| {
                let (_, o) = d.world.objects.iter().next()?;
                let path = d.world.objects.models[o.model as usize].path.clone();
                Some((*o, path))
            });
            if let Some((o, path)) = found {
                let (ctx, db) = (app.ctx.clone(), app.db.clone());
                if let Some(list) = app.proxies.get(&path, &db, &ctx) {
                    if let Some(p) = list.get(n) {
                        let world = &app.doc.as_ref().unwrap().world;
                        let info = db.get(&path);
                        let (m, y) = placed_matrix(&o, &info, &world.height, ObjectStore::origin_y(&o, &world.height));
                        let (_, at) = crate::proxies::place(&m, [o.x, y, o.z], p);
                        eprintln!("PROXY {n} {} at {:?}", app.proxies.models[p.model as usize], at);
                        // From the side of the building's middle, where the room is.
                        let (dx, dz) = (o.x - at[0], o.z - at[2]);
                        let l = dx.hypot(dz).max(0.01);
                        let (dx, dz) = (dx / l, dz / l);
                        app.camera.pos = [at[0] + dx * 2.2, at[1] + 1.4, at[2] + dz * 2.2];
                        app.camera.yaw = (-dx).atan2(-dz);
                        app.camera.pitch = -28f32.to_radians();
                    }
                }
            }
        }
        // Clicks lengthen the selected route at its "start" or "end".
        "routeextend" => app.line.extend = match arg.as_str() { "start" => Some(true), "end" => Some(false), _ => None },
        "routepts" => {
            if let Some(d) = &app.doc {
                for r in &d.world.routes {
                    let f = |p: &[f32; 2]| format!("({:.0},{:.0})", p[0], p[1]);
                    eprintln!("ROUTEPTS {:?} points={} fitted={} center {}..{}", r.name, r.points.iter().map(f).collect::<Vec<_>>().join(" "), r.fitted.iter().map(f).collect::<Vec<_>>().join(" "), r.centerline.first().map(f).unwrap_or_default(), r.centerline.last().map(f).unwrap_or_default());
                }
            }
        }
        "routes" => {
            if let Some(d) = &app.doc {
                for r in &d.world.routes {
                    eprintln!("ROUTE {:?} family={} points={} center={} objects={} terrain={} alive={}", r.name, r.family, r.points.len(), r.centerline.len(), r.objects.len(), r.terrain_route, r.objects.iter().filter(|i| d.world.objects.get(**i).is_some()).count());
                }
                eprintln!("ROUTES total={} objects={} selected={:?} point={:?}", d.world.routes.len(), d.world.objects.len(), app.line.selected_route, app.line.selected_point);
            }
        }
        "routemove" if nums.len() >= 4 => {
            let (r, p) = (nums[0] as usize, nums[1] as usize);
            if let Some(route) = app.doc.as_mut().and_then(|d| d.world.routes.get_mut(r)) {
                let before = route.points.clone();
                route.points[p] = [nums[2], nums[3]];
                app.refit_route(r, before);
            }
        }
        "routedelete" => app.delete_route(nums.first().map(|v| *v as usize).unwrap_or(0)),
        "routeroad" if nums.len() >= 4 => {
            app.mode = Mode::Lines;
            app.line.action = LineAction::Road;
            app.line.points = nums.chunks_exact(2).map(|p| [p[0], p[1]]).collect();
            finish_line(app);
        }
        "genadd" => {
            let index: usize = arg.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0);
            if let Some(id) = app.built_in.get(index).map(|g| g.id) {
                let drop = nums.get(1..3).map(|p| crate::library_ui::Dropped { shape: None, route: None, at: [p[0], p[1]] });
                crate::library_ui::add_instance(app, id, drop);
            }
        }
        "gens" => {
            if let Some(d) = &app.doc {
                for g in &d.world.generators {
                    eprintln!("GEN {} {:?} kind={} target={:?} objects={} alive={}", g.id, g.name, g.kind.type_name(), g.target, g.objects.len(), g.objects.iter().filter(|i| d.world.objects.get(**i).is_some()).count());
                }
                eprintln!("GENS total={} objects={}", d.world.generators.len(), d.world.objects.len());
            }
        }
        "genfill" => {
            let name = dz_core::gen::forest_preset_names().into_iter().nth(nums.first().map_or(1, |v| *v as usize)).unwrap_or_default();
            if let Some(dz_core::library::GeneratorKind::Forest(x)) = app.doc.as_mut().and_then(|d| d.world.generators.last_mut()).map(|g| &mut g.kind) { x.layers = vec![dz_core::library::ForestLayer { preset: name, density_per_ha: nums.get(1).copied().unwrap_or(0.0), ..Default::default() }]; }
        }
        "exportto" => {
            if let Some(d) = &app.doc {
                let opts = dz_core::export::ExportOptions { root: std::path::PathBuf::from(arg.trim()), layers: false, world: false, config: true, source: false };
                match dz_core::export::run(&d.world, &app.db, &opts, &dz_core::Job::default()) {
                    Ok(r) => eprintln!("GENSTATS export tiles={} rvmats={}", r.tiles, r.rvmats),
                    Err(e) => eprintln!("GENSTATS export failed {e:#}"),
                }
            }
        }
        "keypoints" => {
            if let Some(d) = &app.doc {
                eprintln!("GENSTATS keypoints {} {:?}", d.world.keypoints.len(), d.world.keypoints.first());
            }
        }
        "keypointadd" if nums.len() >= 2 => app.add_keypoint(nums[0], nums[1]),
        "saveto" => { app.save(Some(std::path::PathBuf::from(arg.trim()))); }
        "backupnow" => app.start_backup(true),
        "showset" if arg.starts_with("shapes") => app.show_shapes = nums.first().is_some_and(|v| *v > 0.0),
        "showset" if arg.starts_with("lines") => app.show_lines = nums.first().is_some_and(|v| *v > 0.0),
        "riverjoin" => {
            if let Some(dz_core::library::GeneratorKind::River(x)) = app.doc.as_mut().and_then(|d| d.world.generators.last_mut()).map(|g| &mut g.kind) { x.join_offset = nums[0]; }
        }
        "gencount" => {
            if let Some(d) = &app.doc {
                if let Some(g) = d.world.generators.last() {
                    let mut c = std::collections::BTreeMap::<String, usize>::new();
                    for o in g.objects.iter().filter_map(|i| d.world.objects.get(*i)) { *c.entry(d.world.objects.model_path(o).rsplit('\\').next().unwrap_or("").to_string()).or_default() += 1; }
                    eprintln!("GENSTATS models {c:?} removed={} objects_total={} modifier={:?}", g.removed.len(), d.world.objects.len(), g.modifier);
                    for o in g.objects.iter().take(14).filter_map(|i| d.world.objects.get(*i)) { eprintln!("GENSTATS obj {} at {} {} y {} yaw {} pitch {} s {}", d.world.objects.model_path(o).rsplit('\\').next().unwrap_or(""), o.x, o.z, o.elevation, o.yaw, o.pitch, o.scale); }
                }
            }
        }
        "genrun" => {
            let index = nums.first().map(|v| *v as usize).unwrap_or(0);
            if let Some(id) = app.doc.as_ref().and_then(|d| d.world.generators.get(index)).map(|g| g.id) {
                crate::library_ui::run_generator(app, id);
            }
        }
        "genforest" => {
            let project: Option<forest_core::preset::ForestProject> = std::fs::read(&arg).ok().and_then(|b| serde_json::from_slice(&b).ok());
            if let (Some(p), Some(d)) = (project, app.doc.as_mut()) {
                if let Some(dz_core::library::GeneratorKind::Forest(f)) = d.world.generators.last_mut().map(|g| &mut g.kind) { f.project = p; }
            }
        }
        "shapeadd" if nums.len() >= 6 => {
            app.shape.tool = ShapeTool::Polygon;
            app.shape.points = nums.chunks_exact(2).map(|p| [p[0], p[1]]).collect();
            app.finish_shape();
            app.shape.tool = ShapeTool::Edit;
        }
        "lineadd" if nums.len() >= 4 => {
            app.shape.tool = ShapeTool::Polyline;
            app.shape.points = nums.chunks_exact(2).map(|p| [p[0], p[1]]).collect();
            app.finish_shape();
            app.shape.tool = ShapeTool::Edit;
        }
        "routewater" => {
            // A short road over the narrowest water found, for trying out bridges.
            let Some(d) = &app.doc else { return };
            let (w, h, cell) = (d.world.base.cols, d.world.base.rows, d.world.base.cell);
            let at = |i: usize, j: usize| d.world.base.at(i, j);
            let bank = (20.0 / cell).ceil() as usize;
            let mut found = None;
            'scan: for j in (bank * 4..h - bank * 4).step_by(4) {
                let mut i = bank * 4;
                while i < w - bank * 4 {
                    if at(i, j) > 2.0 && at(i + 1, j) <= 0.0 {
                        let mut k = i + 1;
                        while k < w - 1 && at(k, j) <= 0.0 { k += 1; }
                        let span = (k - i) as f32 * cell;
                        if (25.0..=90.0).contains(&span) && k + bank * 3 < w && at(k + bank * 2, j) > 2.0 && at(i - bank * 2, j) > 2.0 && at(i - bank * 2, j) < 25.0 && at(k + bank * 2, j) < 25.0 {
                            found = Some(((i - bank * 3) as f32 * cell, j as f32 * cell, (k + bank * 3) as f32 * cell));
                            break 'scan;
                        }
                        i = k;
                    }
                    i += 1;
                }
            }
            eprintln!("WATER {found:?}");
            if let Some((a, z, b)) = found {
                app.mode = Mode::Lines;
                app.line.action = LineAction::Road;
                app.line.points = vec![[a, z], [b, z]];
                finish_line(app);
            }
        }
        "routebridge" => app.line.routing.bridge_over_forbidden = arg == "1",
        "genstats" => {
            if let Some(d) = &app.doc {
                if let Some(g) = d.world.generators.last() {
                    let road = d.world.routes.last().map(|r| r.centerline.clone()).unwrap_or_default();
                    let dist = |x: f32, z: f32| road.windows(2).map(|s| {
                        let (ax, az, bx, bz) = (s[0][0], s[0][1], s[1][0], s[1][1]);
                        let l = (bx - ax).powi(2) + (bz - az).powi(2);
                        let t = (((x - ax) * (bx - ax) + (z - az) * (bz - az)) / l.max(1e-6)).clamp(0.0, 1.0);
                        ((x - ax - t * (bx - ax)).powi(2) + (z - az - t * (bz - az)).powi(2)).sqrt()
                    }).fold(f32::MAX, f32::min);
                    let mut near = f32::MAX;
                    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                    for o in g.objects.iter().filter_map(|i| d.world.objects.get(*i)) {
                        near = near.min(dist(o.x, o.z));
                        b = [b[0].min(o.x), b[1].min(o.z), b[2].max(o.x), b[3].max(o.z)];
                    }
                    eprintln!("GENSTATS {} n={} road_min={near} bounds={b:?}", g.name, g.objects.len());
                }
            }
        }
        "genset" => {
            use dz_core::library::GeneratorKind as K;
            let models: Vec<(String, f32)> = arg.split(',').map(|m| (m.to_string(), 1.0)).collect();
            if let Some(g) = app.doc.as_mut().and_then(|d| d.world.generators.last_mut()) {
                match &mut g.kind {
                    K::Area(x) => x.objects.models = models,
                    K::Line(x) => x.objects.models = models,
                    K::Fence(x) => { x.model = models[0].0.clone(); x.start_model = models[0].0.clone(); x.flip_end = true; x.end_model = models[0].0.clone(); }
                    K::Scatter(x) => x.models = models,
                    _ => {}
                }
            }
        }
        // A wicket of model `arg` at nums[0] metres, and gates where roads cross, on the last fence.
        "fencegate" => {
            if let Some(dz_core::library::GeneratorKind::Fence(x)) = app.doc.as_mut().and_then(|d| d.world.generators.last_mut()).map(|g| &mut g.kind) {
                let model = arg.split_whitespace().nth(1).unwrap_or("").to_string();
                x.openings.push(dz_core::library::FenceOpening { at: nums.first().copied().unwrap_or(10.0), model: model.clone(), ..Default::default() });
                x.road_gate = Some(dz_core::library::FenceOpening { gate: true, two_leaves: true, left: model.clone(), right: model, width: 8.0, ..Default::default() });
            }
        }
        "genobjs" => {
            if let Some(d) = &app.doc {
                if let Some(g) = d.world.generators.last() {
                    for id in &g.objects { if let Some(o) = d.world.objects.get(*id) { eprintln!("GOBJ {:.2} {:.2} {:.1} {}", o.x, o.z, o.yaw, d.world.objects.model_path(o)); } }
                }
            }
        }
        // Sets the surfaces a texture tile may hold and tells how many tiles hold more.
        "tilelimit" => {
            if let Some(d) = app.doc.as_mut() {
                if let Some(n) = nums.first() { d.world.meta.max_tile_surfaces = *n as u8; }
                eprintln!("GENSTATS tiles_over={} limit={}", dz_core::mask_limit::scan(&d.world).len(), d.world.meta.max_tile_surfaces);
            }
        }
        // "padfollow 1|0" on the selection; "lift d" raises the selection by d metres.
        "padfollow" => { let ids: Vec<u32> = app.selection.iter().copied().collect(); app.set_pad_follow(&ids, nums.first().is_some_and(|v| *v > 0.0)); }
        "lift" if !nums.is_empty() => {
            if let Some(d) = app.doc.as_mut() {
                let ids: Vec<u32> = app.selection.iter().copied().collect();
                for id in ids {
                    if let Some(mut o) = d.world.objects.get(id).copied() { o.elevation += nums[0]; d.world.objects.update(id, o); d.world.sync_attached(id); }
                }
            }
            app.whole_height_changed();
        }
        // Opens the export window, or with a folder writes every part into it.
        "exportfiles" => {
            if let Some(part) = arg.strip_prefix("only:") {
                use crate::exporting::Part;
                let p = match part { "asc" => Part::Heightmap, "sat" => Part::Satellite, "mask" => Part::Mask, "cfg" => Part::Layers, "shp" => Part::Shapes, _ => Part::Objects };
                crate::exporting::open(app, Some(p));
            } else if arg.is_empty() {
                crate::exporting::open(app, None);
            } else {
                let mut s = crate::exporting::ExportSettings { folder: arg.to_string(), shapes_on: true, ..Default::default() };
                s.heightmap.cell = 16.0;
                s.mask.image.size = 1024;
                s.satellite.image.tiles = 2;
                s.objects.split_by_layer = true;
                app.settings.export = s;
                crate::exporting::run(app, None);
            }
        }
        // Where a line from (x0, z) eastward first comes up out of the sea.
        "coast" if nums.len() >= 2 => {
            if let Some(d) = &app.doc {
                let w = &d.world;
                let x = (0..20000).map(|k| nums[0] + k as f32).find(|x| w.height.sample(*x, nums[1]) > w.meta.sea_level);
                eprintln!("COAST z={} x={:?} sea={}", nums[1], x, w.meta.sea_level);
            }
        }
        "about" => app.dialog = crate::app::Dialog::About,
        "reveal" => app.show_in_browser(&arg),
        "padstat" => {
            if let Some(d) = &app.doc {
                for id in &app.selection {
                    if let Some(o) = d.world.objects.get(*id) {
                        let m = d.world.modifiers.iter().find(|m| m.attached == Some(*id));
                        eprintln!("GENSTATS pad object_y={:.2} ground={:.2} base={:.2} absolute={} follow={:?} target={:?}", dz_core::objects::ObjectStore::origin_y(o, &d.world.height), d.world.height.sample(o.x, o.z), d.world.base.sample(o.x, o.z), o.flags & dz_core::objects::FLAG_ABSOLUTE != 0, m.map(|m| m.follow), m.and_then(|m| m.target));
                    }
                }
            }
        }
        // Selects the last generator and waits for a click placing its opening n.
        "pickopening" => {
            if let Some(id) = app.doc.as_ref().and_then(|d| d.world.generators.last()).map(|g| g.id) {
                app.selected_generator = Some(id);
                app.pick_opening = Some((id, nums.first().map(|v| *v as usize).unwrap_or(0)));
            }
        }
        "fencegap" => {
            if let Some(dz_core::library::GeneratorKind::Fence(x)) = app.doc.as_mut().and_then(|d| d.world.generators.last_mut()).map(|g| &mut g.kind) { x.spacing = nums[0]; }
        }
        "wholeobj" => {
            // What a click on the first object of a route or generator selects.
            if let Some(d) = &app.doc {
                let target = if arg == "route" { d.world.routes.last().and_then(|r| r.objects.first().copied()) } else { d.world.generators.last().and_then(|g| g.objects.first().copied()) };
                let owner = target.and_then(|id| owner_of(&d.world, id));
                if let Some((w, ids)) = owner {
                    eprintln!("GENSTATS whole {w:?} n={}", ids.len());
                    app.selection = ids.into_iter().collect();
                    app.selected_generator = if let SelectedWhole::Generator(id) = w { Some(id) } else { None };
                    app.whole = Some(w);
                    app.selection_rev += 1;
                }
            }
        }
        "deletesel" => app.delete_selection(),
        "underflag" => app.toggle_underground(),
        "undercount" => {
            if let Some(d) = &app.doc {
                eprintln!("GENSTATS underground={}", d.world.objects.iter().filter(|(_, o)| o.flags & dz_core::objects::FLAG_UNDERGROUND != 0).count());
            }
        }
        "holepaint" if nums.len() >= 2 => {
            app.hole_radius = nums.get(2).copied().unwrap_or(8.0);
            paint_holes(app, nums[0], nums[1], false);
            if let Some(before) = app.hole_before.take() {
                if let Some(d) = &mut app.doc { d.history.push(t("Terrain holes"), Change::Holes(before)); eprintln!("GENSTATS holes={}", d.world.holes.len()); }
            }
        }
        "contours" => { app.show_contours = true; app.contour_interval = nums.first().copied().unwrap_or(10.0); }
        "camunder" => app.camera_under = true,
        "gradecfg" if nums.len() >= 4 => {
            if let Some(i) = app.doc.as_ref().map(|d| d.world.routes.len() - 1) {
                if let Some(r) = app.doc.as_mut().map(|d| &mut d.world.routes[i]) {
                    r.grading.embankment = nums[0] != 0.0;
                    r.grading.raise = nums[1];
                    r.grading.ditch_depth = nums[2];
                    r.grading.max_grade = nums[3];
                }
                app.route_grading_changed(i);
            }
        }
        "profile" if nums.len() >= 2 => {
            if let Some(d) = &app.doc {
                let h: Vec<String> = (0..12).map(|k| format!("{:.2}", d.world.height.sample(nums[0], nums[1] + k as f32 * 4.0))).collect();
                eprintln!("GENSTATS profile {}", h.join(" "));
            }
        }
        "railcut" if nums.len() >= 2 => {
            app.line.rail_routing.max_cut = nums[0] as f64;
            app.line.rail_routing.max_fill = nums[1] as f64;
        }
        "routecheck" => {
            if let Some(d) = &app.doc {
                if let Some(r) = d.world.routes.last() {
                    let w = &d.world;
                    let (mut miss, mut cut, mut fill) = (0.0f32, 0.0f32, 0.0f32);
                    for p in &r.profile {
                        miss = miss.max((w.height.sample(p[0], p[1]) - p[2]).abs());
                        cut = cut.max(w.base.sample(p[0], p[1]) - p[2]);
                        fill = fill.max(p[2] - w.base.sample(p[0], p[1]));
                    }
                    let mut gap = 0.0f32;
                    for o in r.objects.iter().filter_map(|i| w.objects.get(*i)) {
                        gap = gap.max((ObjectStore::origin_y(o, &w.height) - w.height.sample(o.x, o.z)).abs());
                    }
                    eprintln!("GENSTATS check {:?} profile={} grade={} decks={} ground_miss={miss:.2} cut={cut:.2} fill={fill:.2} objects={} object_gap={gap:.2}", r.name, r.profile.len(), r.grade, r.decks.len(), r.objects.len());
                    if let Some(p) = r.profile.get(r.profile.len() / 2) { eprintln!("GENSTATS mid {:?}", p); }
                }
            }
        }
        // Makes route n look like one imported or built before routes kept their pieces.
        "routeforget" => {
            if let Some(r) = app.doc.as_mut().and_then(|d| d.world.routes.get_mut(nums.first().map(|v| *v as usize).unwrap_or(0))) { r.parts.clear(); r.fitted.clear(); }
        }
        "routeobjs" => {
            if let Some(d) = &app.doc {
                if let Some(r) = d.world.routes.get(nums.first().map(|v| *v as usize).unwrap_or(0)) {
                    for id in &r.objects { if let Some(o) = d.world.objects.get(*id) { eprintln!("ROBJ {id} {:.3} {:.3} {:.2} {}", o.x, o.z, o.yaw, d.world.objects.model_path(o)); } }
                }
            }
        }
        "routesel" => { app.line.selected_route = nums.first().map(|v| *v as usize); app.mode = Mode::Lines; app.line.action = LineAction::Edit; }
        "routepush" if nums.len() >= 3 => {
            if let Some(route) = app.doc.as_mut().and_then(|d| d.world.routes.get_mut(nums[0] as usize)) { route.points.push([nums[1], nums[2]]); }
        }
        "routefit" => {
            let r = nums.first().map(|v| *v as usize).unwrap_or(0);
            if let Some(before) = app.doc.as_ref().and_then(|d| d.world.routes.get(r)).map(|x| x.points.clone()) { app.refit_route(r, before); }
        }
        "routedecks" => {
            if let Some(d) = &app.doc {
                for r in &d.world.routes {
                    for k in &r.decks {
                        eprintln!("DECK {:?} from={:?} to={:?} height={}", r.name, k.from, k.to, k.height);
                    }
                }
            }
        }
        "genline" if !nums.is_empty() => {
            use dz_core::library::{GeneratorKind as K, LineMode};
            if let Some(g) = app.doc.as_mut().and_then(|d| d.world.generators.last_mut()) {
                let line = match &mut g.kind { K::Fence(x) => Some(&mut x.line), K::PowerLine(x) => Some(&mut x.line), K::River(x) => Some(&mut x.line), _ => None };
                if let Some(l) = line { l.offset = nums[0]; l.mode = if nums.get(1) == Some(&1.0) { LineMode::Bezier } else { LineMode::Polyline }; }
            }
        }
        "genmodels" => {
            if let Some(dz_core::library::GeneratorKind::Scatter(s)) = app.doc.as_mut().and_then(|d| d.world.generators.last_mut()).map(|g| &mut g.kind) {
                s.models = arg.split(',').map(|m| (m.to_string(), 1.0)).collect();
            }
        }
        "genpower" => {
            if let (Some(dz_core::library::GeneratorKind::PowerLine(p)), Ok(text)) = (app.doc.as_mut().and_then(|d| d.world.generators.last_mut()).map(|g| &mut g.kind), std::fs::read(&arg)) {
                match dz_core::library::parse_powerlines(&String::from_utf8_lossy(&text), "test") { Ok(x) => *p = x, Err(e) => eprintln!("POWER {e}") }
            }
        }
        "genskip" => {
            let picked = arg.split_whitespace().next().unwrap_or("").to_string();
            let picked = picked.strip_prefix('#').and_then(|i| i.parse::<usize>().ok()).and_then(|i| app.doc.as_ref()?.world.layers.get(i).map(|l| l.name.clone())).unwrap_or(picked);
            let picked = if picked == "@" { app.doc.as_ref().and_then(|d| {
                let (m, size) = (&d.world.mask, d.world.meta.size_m());
                let (c, r) = ((nums[2] / size * m.width as f32) as usize, ((1.0 - nums[3] / size) * m.height as f32) as usize);
                d.world.layers.get(m.data[r * m.width + c] as usize).map(|l| l.name.clone())
            }).unwrap_or_default() } else { picked };
            eprintln!("GENSTATS skip {picked}");
            if let Some(dz_core::library::GeneratorKind::Forest(f)) = app.doc.as_mut().and_then(|d| d.world.generators.last_mut()).map(|g| &mut g.kind) {
                f.skip_surfaces = vec![(picked, 10.0)];
                f.avoid_roads = nums.first().filter(|v| **v > 0.0).copied();
                f.avoid_buildings = nums.get(1).filter(|v| **v > 0.0).copied();
            }
        }
        "brushat" if nums.len() >= 2 => {
            app.place.brush = true;
            brush_objects(app, nums[0], nums[1], nums.get(2) == Some(&1.0));
            finish_place_stroke(app);
        }
        "report" => {
            let d = app.doc.as_ref();
            if let (Some(d), Some(id)) = (d, app.selection.iter().next()) {
                if let Some(o) = d.world.objects.get(*id) {
                    eprintln!("PICKED {}", d.world.objects.model_path(o));
                }
            }
            eprintln!("REPORT objects={} modifiers={} selection={} undo={:?} status={}", d.map(|d| d.world.objects.len()).unwrap_or(0), d.map(|d| d.world.modifiers.len()).unwrap_or(0), app.selection.len(), d.and_then(|d| d.history.undo_label().map(str::to_string)), app.status);
        }
        "quit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        _ => {}
    }
}

#[cfg(test)]
mod picking_tests {
    use super::*;

    #[test]
    fn points_snap_to_whole_angle_steps() {
        // Almost east, 20 m away: east at the same length.
        let p = snap_direction([0.0, 0.0], [20.0, 1.5], 15.0);
        assert!((p[0] - 20.056).abs() < 0.01 && p[1].abs() < 1e-3, "{p:?}");
        // 50 degrees goes to 45.
        let q = snap_direction([10.0, 10.0], [10.0 + 50f32.to_radians().sin() * 8.0, 10.0 + 50f32.to_radians().cos() * 8.0], 15.0);
        let a = (q[0] - 10.0).atan2(q[1] - 10.0).to_degrees();
        assert!((a - 45.0).abs() < 1e-3, "{a}");
        assert_eq!(snap_direction([1.0, 1.0], [1.0, 1.0], 15.0), [1.0, 1.0]);
    }

    #[test]
    fn grass_footprint_follows_the_geometry_not_the_box() {
        // An L of thin walls 10 m long and 3 m high, and a roof 4 m up over a corner.
        let mut pos = Vec::new();
        let mut tri = Vec::new();
        let mut quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| {
            let n = pos.len() as u32;
            pos.extend([a, b, c, d]);
            tri.extend([n, n + 1, n + 2, n, n + 2, n + 3]);
        };
        quad([0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 3.0, 0.0], [0.0, 3.0, 0.0]);
        quad([0.0, 0.0, 0.0], [0.0, 0.0, 10.0], [0.0, 3.0, 10.0], [0.0, 3.0, 0.0]);
        quad([6.0, 4.0, 6.0], [10.0, 4.0, 6.0], [10.0, 4.0, 10.0], [6.0, 4.0, 10.0]);
        let f = Footprint::of(&pos, &tri);
        assert!(f.low_at(5.0, 0.1) <= 0.0 && f.low_at(0.1, 5.0) <= 0.0, "under the walls");
        assert!(f.low_at(5.0, 5.0).is_infinite(), "inside the L, where the box would have been");
        assert_eq!(f.low_at(8.0, 8.0), 4.0, "under the roof it is high above the ground");
    }

    #[test]
    fn terrain_is_only_found_from_above() {
        let h = Heightmap::new(64, 64, 4.0, 10.0);
        let down = [0.0, -1.0, 0.0];
        let hit = pick_terrain(&h, [100.0, 30.0, 100.0], down, false).unwrap();
        assert!((hit[1] - 10.0).abs() < 0.01);
        assert!(pick_terrain(&h, [100.0, 5.0, 100.0], down, false).is_none());
        assert!(pick_terrain(&h, [100.0, 5.0, 100.0], [0.0, 1.0, 0.0], false).is_none());
    }
}
