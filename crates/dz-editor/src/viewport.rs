//! Viewports: camera control, tools, draw lists and on-screen overlays.
use crate::app::{App, Finish, Gizmo, LineAction, Mode, ViewMode};
use crate::camera::{add, dot, mul, norm, sub, View, V3};
use crate::i18n::{t, tf};
use crate::models::{LOD_FAR, LOD_NEAR};
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
    toggles: Option<(bool, bool, Shading)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Handle {
    Plane,
    X,
    Y,
    Z,
    Ring,
    Scale,
}

pub struct ObjectDrag {
    handle: Handle,
    before: Vec<(u32, Obj)>,
    pivot: V3,
    start: Pos2,
    /// Screen direction and metres per point for axis handles.
    axis: (egui::Vec2, V3, f32),
    grab_offset: [f32; 2],
}

/// Scripted input for automatic checks (`DZWE_AUTO`).
pub struct AutoScript {
    pub steps: Vec<String>,
    pub wait: u32,
}

fn pick_terrain(h: &Heightmap, o: V3, d: V3, ortho: bool) -> Option<V3> {
    let (w, depth) = (h.width_m(), h.height_m());
    let inside = |x: f32, z: f32| x >= 0.0 && z >= 0.0 && x <= w && z <= depth;
    if ortho {
        return inside(o[0], o[2]).then(|| [o[0], h.sample(o[0], o[2]), o[2]]);
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

fn pick_object(app: &App, v: &View, s: Pos2) -> Option<u32> {
    let doc = app.doc.as_ref()?;
    let (o, d) = v.ray(s);
    let limit = pick_terrain(&doc.world.height, o, d, v.ortho).map(|p| dot(sub(p, o), d) + 1.0).unwrap_or(4000.0).min(6000.0);
    let mut seen = std::collections::HashSet::new();
    let mut best: Option<(f32, u32)> = None;
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
                    let ground = ObjectStore::origin_y(ob, &world.height);
                    let info = app.db.get(world.objects.model_path(ob));
                    if let Some(h) = ray_box(&o, &d, ob, ground, &info) {
                        if best.is_none_or(|b| h < b.0) {
                            best = Some((h, id));
                        }
                    }
                }
            }
        }
        t += step;
    }
    best.map(|b| b.1)
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

const SELECT_TINT: [f32; 4] = [1.0, 0.55, 0.05, 1.0];

/// Rebuilds the object draw list when something visible changed.
fn prepare_objects(app: &mut App, v: &View, top: bool) -> ObjectDraw {
    let renderer = app.renderer.clone();
    let renderer = renderer.lock().unwrap();
    let Some(doc) = &app.doc else { return ObjectDraw::default() };
    let world = &doc.world;
    let cache = if top { &mut app.view2 } else { &mut app.view3 };
    if !app.show_objects || (world.objects.is_empty() && app.clutter_models.is_empty()) {
        cache.draw = ObjectDraw::default();
        return ObjectDraw::default();
    }
    // The top view lists more than is on screen, so panning needs no rebuild until it nears the edge.
    let shift = dot(sub(cache.eye, v.eye), sub(cache.eye, v.eye)).sqrt();
    let toggles = (app.show_grass, app.show_shadows, if top { app.shading2 } else { app.shading3 });
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

    let mut boxes = Vec::new();
    let mut instances: HashMap<(u32, u8), Vec<f32>> = HashMap::new();
    let mut impostors = Vec::new();
    let mut shadow: HashMap<(u32, u8), Vec<f32>> = HashMap::new();
    // Shadows reach this far from the camera.
    const SHADOW_RANGE: f32 = 260.0;
    if v.ortho {
        // Top view: footprints, thinned out when zoomed far out.
        let half = (v.rect.width().max(v.rect.height()) * v.mpp * 0.75).max(10.0);
        let (ex, ez) = (v.eye[0], v.eye[2]);
        let keep = (v.mpp / 2.5).floor().max(1.0) as u32;
        let min_size = (v.mpp * 2.0).max(0.2);
        boxes.reserve(1 << 16);
        world.objects.for_each_in(ex - half, ez - half, ex + half, ez + half, |id, o| {
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
        for gz in kz - cells..=kz + cells {
            for gx in kx - cells..=kx + cells {
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
                    let (m0, m1) = (o.model, renderer.impostor_layers.get(&o.model));
                    if d < near {
                        if let Some(n) = renderer.model_triangles((m0, LOD_NEAR)).filter(|_| triangles_left > 0) {
                            triangles_left -= n as i64;
                            write_instance(instances.entry((m0, LOD_NEAR)).or_default(), &rotation(o.yaw, o.pitch, o.roll, o.scale), [o.x, y, o.z], tint);
                            let key = if renderer.model_triangles((m0, LOD_FAR)).is_some() { (m0, LOD_FAR) } else { (m0, LOD_NEAR) };
                            write_instance(shadow.entry(key).or_default(), &rotation(o.yaw, o.pitch, o.roll, o.scale), [o.x, y, o.z], tint);
                            continue;
                        }
                    }
                    if d < far || sel {
                        if let Some(n) = renderer.model_triangles((m0, LOD_FAR)).filter(|_| triangles_left > 0) {
                            triangles_left -= n as i64;
                            write_instance(instances.entry((m0, LOD_FAR)).or_default(), &rotation(o.yaw, o.pitch, o.roll, o.scale), [o.x, y, o.z], tint);
                            if d < SHADOW_RANGE {
                                write_instance(shadow.entry((m0, LOD_FAR)).or_default(), &rotation(o.yaw, o.pitch, o.roll, o.scale), [o.x, y, o.z], tint);
                            }
                            if d < near {
                                requests.push((m0, LOD_NEAR));
                            }
                            continue;
                        }
                    }
                    requests.push((m0, LOD_FAR));
                    if d < near {
                        requests.push((m0, LOD_NEAR));
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
    // Ground clutter on a 1 m grid, thinned with distance like the game's clutter range.
    let mut clutter_requests: Vec<(u32, u8)> = Vec::new();
    if !v.ortho && app.show_grass && !app.clutter_models.is_empty() {
        const RANGE: i32 = 110;
        let shading = if top { app.shading2 } else { app.shading3 };
        if shading == Shading::Lit {
            let (ex, ez) = (v.eye[0].round() as i32, v.eye[2].round() as i32);
            let size = world.meta.size_m();
            let mut triangles_left: i64 = 5_000_000;
            // The game keeps clutter off roads: (x, z, sin, cos, half width, half length).
            let mut road_boxes: Vec<[f32; 6]> = Vec::new();
            let reach = RANGE as f32 + 40.0;
            world.objects.for_each_in(v.eye[0] - reach, v.eye[2] - reach, v.eye[0] + reach, v.eye[2] + reach, |_, o| {
                if !world.objects.model_path(o).contains(r"\roads\parts\") {
                    return;
                }
                let info = &infos[o.model as usize];
                let (sin, cos) = o.yaw.to_radians().sin_cos();
                let (cx, cz) = ((info.min[0] + info.max[0]) * 0.5, (info.min[2] + info.max[2]) * 0.5);
                let (hx, hz) = info.footprint();
                road_boxes.push([o.x + cos * cx + sin * cz, o.z - sin * cx + cos * cz, sin, cos, hx, hz]);
            });
            let on_road = |x: f32, z: f32| road_boxes.iter().any(|b| {
                let (dx, dz) = (x - b[0], z - b[1]);
                (dx * b[3] - dz * b[2]).abs() < b[4] && (dx * b[2] + dz * b[3]).abs() < b[5]
            });
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
                    let r = h.rotate_left(11).wrapping_mul(40503);
                    let pick = (r & 0xFFFF) as f32 / 65535.0;
                    let Some(list) = app.clutter_layers.get(world.mask.sample(wx, wz, size) as usize) else { continue };
                    let Some(&(model, _, a, b)) = list.iter().find(|e| pick < e.1) else { continue };
                    if on_road(wx, wz) {
                        continue;
                    }
                    let y = world.height.sample(wx, wz);
                    if y < world.meta.sea_level || crate::camera::box_outside(&planes, [wx - 2.0, y - 2.0, wz - 2.0], [wx + 2.0, y + 3.0, wz + 2.0]) {
                        continue;
                    }
                    let id = CLUTTER_BASE + model;
                    let lod = if d < 25.0 { LOD_NEAR } else { LOD_FAR };
                    let Some(n) = renderer.model_triangles((id, lod)).filter(|_| triangles_left > 0) else {
                        clutter_requests.push((model, lod));
                        continue;
                    };
                    triangles_left -= n as i64;
                    let k = ((r >> 16) & 1023) as f32 / 1023.0;
                    // Shrinks toward the edge of the range so grass does not pop in.
                    let fade = ((RANGE as f32 - d) / 18.0).clamp(0.0, 1.0);
                    let yaw = ((r >> 5) & 511) as f32 * 0.7045;
                    write_instance(instances.entry((id, lod)).or_default(), &rotation(yaw, 0.0, 0.0, (a + (b - a) * k) * fade), [wx, y, wz], [1.0, 1.0, 1.0, 0.0]);
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
    requests.sort_unstable();
    requests.dedup();
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
    for (m, l) in clutter_requests {
        let path = app.clutter_models[m as usize].clone();
        app.loader.request((CLUTTER_BASE + m, l), &path);
    }
    draw
}

fn begin_paint_or_sculpt(app: &mut App, p: V3, invert: bool, smooth: bool) {
    let Some(doc) = &mut app.doc else { return };
    match app.mode {
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
            app.stroke = Some(SculptStroke::begin(tool, brush, &doc.world, p[0], p[2]));
        }
        Mode::Paint => app.mask_stroke = Some(MaskStroke::default()),
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

pub fn place_model(app: &mut App, model: &str, x: f32, z: f32) {
    let (random, align, smin, smax, auto_pad, falloff) = (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max, app.place.auto_pad, app.place.pad_falloff);
    let Some(doc) = &mut app.doc else { return };
    let model_id = doc.world.objects.intern(model);
    let seed = (x.to_bits() ^ z.to_bits().rotate_left(13)).wrapping_mul(2654435761);
    let yaw = if random { (seed >> 7) as f32 / (u32::MAX >> 7) as f32 * 360.0 } else { 0.0 };
    let k = ((seed >> 3) & 1023) as f32 / 1023.0;
    let scale = smin + (smax - smin) * k;
    let (pitch, roll) = if align { gen::slope_angles(&doc.world.height, x, z, yaw) } else { (0.0, 0.0) };
    let o = Obj { x, z, elevation: 0.0, yaw, pitch, roll, scale, model: model_id, flags: 0 };
    let id = doc.world.objects.insert(o);
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
    let (action, family, grade, model, spacing, radius) = (app.line.action, app.line.family.clone(), app.line.grade, app.line.fence_model.clone(), app.line.fence_spacing, app.line.radius);
    let Some(doc) = &app.doc else { return };
    match action {
        LineAction::Plain => {
            let d = app.doc.as_mut().unwrap();
            let n = d.world.lines.len() + 1;
            d.world.lines.push(dz_core::world::MapLine { name: format!("Line {n}"), family: "line".into(), points, width: 4.0 });
            d.history.revision += 1;
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
        LineAction::Road => {
            let mut lite = doc.world.lite();
            let catalog = app.road_catalog.clone();
            app.spawn(t("Building road"), move |_| {
                let road = gen::build_road(&mut lite, &points, &family, &catalog, "dz\\structures\\roads\\parts", radius as f64)?;
                let models: Vec<String> = lite.objects.models.iter().map(|m| m.path.clone()).collect();
                Ok(Box::new(move |a: &mut App| {
                    let label = t("Build road");
                    a.merge_generated(&label, models, road.objects);
                    let Some(d) = &mut a.doc else { return };
                    let n = d.world.lines.len() + 1;
                    let center = road.centerline;
                    d.world.lines.push(dz_core::world::MapLine { name: format!("Road {n}"), family, points: center.clone(), width: 8.0 });
                    if grade {
                        let mut base = d.world.base.clone();
                        if gen::grade_along(&mut base, &center, 9.0, 12.0).is_some() {
                            a.apply_heights(&label, base.data, Vec::new());
                            if let Some(d) = &mut a.doc {
                                d.history.group_last(2, label.clone());
                            }
                        }
                    }
                }) as Finish)
            });
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

fn gizmo_size(v: &View, p: V3) -> f32 {
    if v.ortho {
        v.mpp * 90.0
    } else {
        dot(sub(p, v.eye), v.forward).max(1.0) * 0.14
    }
}

fn gizmo_handle(app: &App, v: &View, p: V3, mouse: Pos2) -> Option<(Handle, egui::Vec2, V3, f32)> {
    let c = v.project(p)?;
    let size = gizmo_size(v, p);
    match app.gizmo {
        Gizmo::Move => {
            if (mouse - c).length() < 9.0 {
                return Some((Handle::Plane, vec2(0.0, 0.0), [0.0; 3], 0.0));
            }
            let mut axes = vec![(Handle::X, [1.0, 0.0, 0.0]), (Handle::Z, [0.0, 0.0, 1.0])];
            if !v.ortho {
                axes.push((Handle::Y, [0.0, 1.0, 0.0]));
            }
            for (h, a) in axes {
                let Some(e) = v.project(add(p, mul(a, size))) else { continue };
                let (dir, len) = ((e - c).normalized(), (e - c).length());
                let along = (mouse - c).dot(dir);
                let off = ((mouse - c) - dir * along).length();
                if along > 8.0 && along < len + 8.0 && off < 6.0 {
                    return Some((h, dir, a, size / len.max(1.0)));
                }
            }
            None
        }
        Gizmo::Rotate => {
            let r = 80.0;
            ((mouse - c).length() - r).abs().lt(&8.0).then_some((Handle::Ring, vec2(0.0, 0.0), [0.0; 3], 0.0))
        }
        Gizmo::Scale => (mouse - c).length().lt(&60.0).then_some((Handle::Scale, vec2(0.0, 0.0), [0.0; 3], 0.0)),
    }
}

fn apply_object_drag(app: &mut App, v: &View, mouse: Pos2, snap: bool) {
    let Some(drag) = app.drag.take() else { return };
    let Some(doc) = &mut app.doc else { return };
    let delta = mouse - drag.start;
    let world = &mut doc.world;
    let mut area: Option<dz_core::terrain::Rect> = None;
    let (mut dx, mut dy, mut dz, mut dyaw, mut factor) = (0.0, 0.0, 0.0, 0.0, 1.0);
    match drag.handle {
        Handle::Plane => {
            let (o, d) = v.ray(mouse);
            if let Some(p) = pick_terrain(&world.height, o, d, v.ortho) {
                dx = p[0] + drag.grab_offset[0] - drag.pivot[0];
                dz = p[2] + drag.grab_offset[1] - drag.pivot[2];
            }
        }
        Handle::X | Handle::Y | Handle::Z => {
            let along = delta.dot(drag.axis.0) * drag.axis.2;
            let along = if snap { (along).round() } else { along };
            dx = drag.axis.1[0] * along;
            dy = drag.axis.1[1] * along;
            dz = drag.axis.1[2] * along;
        }
        Handle::Ring => {
            let c = v.project(drag.pivot).unwrap_or(drag.start);
            let (a0, a1) = ((drag.start - c).angle(), (mouse - c).angle());
            dyaw = (a1 - a0).to_degrees();
            if snap {
                dyaw = (dyaw / 15.0).round() * 15.0;
            }
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

fn end_object_drag(app: &mut App) {
    let Some(drag) = app.drag.take() else { return };
    let Some(doc) = &mut app.doc else { return };
    let change: Vec<_> = drag.before.iter().filter_map(|(id, b)| {
        let now = doc.world.objects.get(*id).copied()?;
        (now != *b).then_some((*id, Some(*b), Some(now)))
    }).collect();
    if !change.is_empty() {
        doc.history.push(t("Transform objects"), Change::Objects(change));
    }
}

fn camera_input(app: &mut App, ctx: &egui::Context, resp: &egui::Response, v: &View, top: bool, typing: bool) {
    let dt = ctx.input(|i| i.stable_dt).min(0.1);
    let hover = resp.hovered() || resp.dragged();
    let (scroll, modifiers) = ctx.input(|i| (i.smooth_scroll_delta.y, i.modifiers));
    if top {
        let drag = if resp.dragged_by(PointerButton::Middle) || resp.dragged_by(PointerButton::Secondary) { resp.drag_delta() } else { vec2(0.0, 0.0) };
        app.top_center[0] -= drag.x * app.top_mpp;
        app.top_center[1] += drag.y * app.top_mpp;
        if hover && scroll != 0.0 && !modifiers.shift {
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
    } else if hover && scroll != 0.0 && !modifiers.shift {
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
        ctx.input(|i| {
            let f = app.camera.forward();
            let r = app.camera.right();
            fast = i.modifiers.shift;
            if looking || !i.modifiers.any() {
                if i.key_down(Key::W) && looking { mv = add(mv, f); }
                if i.key_down(Key::S) && looking { mv = sub(mv, f); }
                if i.key_down(Key::D) && looking { mv = add(mv, r); }
                if i.key_down(Key::A) && looking { mv = sub(mv, r); }
                if i.key_down(Key::E) && looking { mv = add(mv, [0.0, 1.0, 0.0]); }
                if i.key_down(Key::Q) && looking { mv = sub(mv, [0.0, 1.0, 0.0]); }
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
        app.camera.pos[1] = app.camera.pos[1].max(g + 0.6);
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
    let Some(mouse) = resp.hover_pos().or_else(|| resp.interact_pointer_pos()) else { return };
    let dt = ctx.input(|i| i.stable_dt).min(0.1);
    let Some(doc) = &app.doc else { return };
    let (o, d) = v.ray(mouse);
    let hit = pick_terrain(&doc.world.height, o, d, v.ortho);
    let modifiers = ctx.input(|i| i.modifiers);
    let primary_down = ctx.input(|i| i.pointer.primary_down());
    let pressed = resp.is_pointer_button_down_on() && ctx.input(|i| i.pointer.primary_pressed());

    // Models dropped from the content browser.
    if let Some(p) = resp.dnd_release_payload::<String>() {
        if let Some(h) = hit {
            let m = (*p).clone();
            place_model(app, &m, h[0], h[2]);
        }
        return;
    }

    match app.mode {
        Mode::Landscape | Mode::Paint => {
            if pressed && !modifiers.alt {
                if let Some(h) = hit {
                    begin_paint_or_sculpt(app, h, modifiers.ctrl, modifiers.shift);
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
        Mode::Place => {
            if resp.clicked() && !modifiers.alt {
                match (&app.place.model, hit) {
                    (Some(m), Some(h)) => {
                        let m = m.clone();
                        place_model(app, &m, h[0], h[2]);
                    }
                    (None, _) => app.warn(t("Choose a model in the Content Browser first")),
                    _ => {}
                }
            }
        }
        Mode::Lines => {
            if resp.clicked() {
                if let Some(h) = hit {
                    app.line.points.push([h[0], h[2]]);
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
        Mode::Generate => {
            if resp.clicked() {
                if let Some(h) = hit {
                    if app.gen.pick_settlement {
                        app.gen.pick_settlement = false;
                        crate::panels::run_settlement(app, h[0], h[2]);
                    } else if app.gen.pick_scatter {
                        app.gen.pick_scatter = false;
                        crate::panels::run_scatter(app, Some((h[0], h[2])));
                    }
                }
            }
        }
        Mode::Select => {
            let pivot = selection_pivot(app);
            if pressed && !modifiers.alt {
                if let Some(p) = pivot {
                    if let Some((handle, dir, axis, scale)) = gizmo_handle(app, v, p, mouse) {
                        let doc = app.doc.as_ref().unwrap();
                        let before: Vec<_> = app.selection.iter().filter_map(|id| doc.world.objects.get(*id).map(|o| (*id, *o))).collect();
                        let grab = hit.map(|h| [p[0] - h[0], p[2] - h[2]]).unwrap_or([0.0, 0.0]);
                        app.drag = Some(ObjectDrag { handle, before, pivot: p, start: mouse, axis: (dir, axis, scale), grab_offset: grab });
                        return;
                    }
                }
                app.box_select = Some((mouse, mouse, modifiers.shift || modifiers.ctrl));
            }
            if app.drag.is_some() {
                if primary_down {
                    apply_object_drag(app, v, mouse, modifiers.ctrl);
                    ctx.request_repaint();
                } else {
                    end_object_drag(app);
                }
                return;
            }
            if let Some(sel) = &mut app.box_select {
                sel.1 = mouse;
                if !primary_down {
                    let (a, b, add_mode) = app.box_select.take().unwrap();
                    finish_selection(app, v, a, b, add_mode);
                }
                ctx.request_repaint();
            }
        }
    }
}

fn finish_selection(app: &mut App, v: &View, a: Pos2, b: Pos2, add_mode: bool) {
    if !add_mode {
        app.selection.clear();
        app.selected_modifier = None;
        app.selected_line = None;
    }
    let r = Rect::from_two_pos(a, b);
    if r.width() < 4.0 && r.height() < 4.0 {
        if let Some(id) = pick_object(app, v, b) {
            if add_mode && app.selection.contains(&id) {
                app.selection.remove(&id);
            } else {
                app.selection.insert(id);
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
        let mut found = Vec::new();
        world.objects.for_each_in(x0, z0, x1, z1, |id, o| {
            if let Some(s) = v.project([o.x, ObjectStore::origin_y(o, &world.height), o.z]) {
                if r.contains(s) {
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
            for (h, a, col) in [(Handle::X, [1.0, 0.0, 0.0], Color32::from_rgb(230, 60, 60)), (Handle::Y, [0.0, 1.0, 0.0], Color32::from_rgb(70, 200, 70)), (Handle::Z, [0.0, 0.0, 1.0], Color32::from_rgb(70, 110, 240))] {
                if v.ortho && h == Handle::Y {
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
            painter.circle_stroke(c, 80.0, Stroke::new(3.0_f32, color(Handle::Ring, Color32::from_rgb(70, 200, 70))));
        }
        Gizmo::Scale => {
            painter.rect_filled(Rect::from_center_size(c, vec2(14.0, 14.0)), 1.0, color(Handle::Scale, Color32::from_rgb(90, 170, 240)));
            painter.circle_stroke(c, 60.0, Stroke::new(1.0_f32, Color32::from_gray(150)));
        }
    }
}

fn draw_polyline(painter: &egui::Painter, v: &View, world: &dz_core::world::World, points: &[[f32; 2]], stroke: Stroke) {
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
        for (i, l) in world.lines.iter().enumerate() {
            let col = if app.selected_line == Some(i) {
                Color32::from_rgb(255, 170, 40)
            } else if l.family == "river" {
                Color32::from_rgb(70, 140, 230)
            } else {
                Color32::from_rgb(235, 235, 120)
            };
            draw_polyline(&painter, v, world, &l.points, Stroke::new(1.6_f32, col));
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
    if let Some(s) = v.project([world.meta.spawn[0], world.height.sample(world.meta.spawn[0], world.meta.spawn[1]) + 1.0, world.meta.spawn[1]]) {
        painter.circle_filled(s, 5.0, Color32::from_rgb(90, 220, 110));
        painter.text(s + vec2(8.0, -2.0), egui::Align2::LEFT_CENTER, t("Player start"), egui::FontId::proportional(11.0), Color32::WHITE);
    }
    if app.mode == Mode::Lines && !app.line.points.is_empty() {
        let mut pts = app.line.points.clone();
        if let Some(m) = resp.hover_pos() {
            let (o, d) = v.ray(m);
            if let Some(h) = pick_terrain(&world.height, o, d, v.ortho) {
                pts.push([h[0], h[2]]);
            }
        }
        draw_polyline(&painter, v, world, &pts, Stroke::new(2.5_f32, Color32::from_rgb(255, 120, 40)));
        for p in &app.line.points {
            if let Some(s) = v.project([p[0], world.height.sample(p[0], p[1]), p[1]]) {
                painter.circle_filled(s, 4.0, Color32::from_rgb(255, 120, 40));
            }
        }
    }
    if let Some((a, b, _)) = app.box_select {
        let r = Rect::from_two_pos(a, b);
        painter.rect(r, 0.0, Color32::from_rgba_unmultiplied(0, 112, 224, 40), Stroke::new(1.0_f32, theme::ACCENT), egui::StrokeKind::Inside);
    }
    if app.mode == Mode::Select {
        if let Some(p) = selection_pivot(app) {
            let hover = resp.hover_pos().and_then(|m| gizmo_handle(app, v, p, m)).map(|h| h.0);
            draw_gizmo(app, &painter, v, p, app.drag.as_ref().map(|d| d.handle).or(hover));
        }
    }
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
fn sun_matrix(v: &View, sun: V3) -> [f32; 16] {
    const RADIUS: f32 = 300.0;
    const DEPTH: f32 = 600.0;
    let l = norm(sun);
    let r = norm(crate::camera::cross([0.0, 1.0, 0.0], l));
    let u = crate::camera::cross(l, r);
    // Moves in steps so the map is redrawn rarely and its edges stay still.
    let c = add(v.eye, mul(norm([v.forward[0], 0.0, v.forward[2]]), RADIUS * 0.5));
    let c = [(c[0] / 16.0).round() * 16.0, c[1], (c[2] / 16.0).round() * 16.0];
    let texel = RADIUS * 2.0 / 2048.0;
    let (rc, uc) = ((dot(r, c) / texel).round() * texel, (dot(u, c) / texel).round() * texel);
    [
        r[0] / RADIUS, u[0] / RADIUS, -l[0] / DEPTH, 0.0,
        r[1] / RADIUS, u[1] / RADIUS, -l[1] / DEPTH, 0.0,
        r[2] / RADIUS, u[2] / RADIUS, -l[2] / DEPTH, 0.0,
        -rc / RADIUS, -uc / RADIUS, dot(l, c) / DEPTH, 1.0,
    ]
}

fn brush_of(app: &App, hit: Option<V3>) -> Option<BrushView> {
    let h = hit?;
    match app.mode {
        Mode::Landscape => Some(BrushView { x: h[0], z: h[2], radius: app.sculpt.radius, falloff: app.sculpt.falloff, color: [0.25, 0.65, 1.0] }),
        Mode::Paint => Some(BrushView { x: h[0], z: h[2], radius: app.paint.radius, falloff: 0.0, color: [1.0, 0.75, 0.2] }),
        Mode::Place => Some(BrushView { x: h[0], z: h[2], radius: 2.0, falloff: 0.0, color: [0.3, 1.0, 0.4] }),
        Mode::Generate if app.gen.pick_settlement => Some(BrushView { x: h[0], z: h[2], radius: app.gen.settlement.radius, falloff: 0.0, color: [0.9, 0.5, 0.9] }),
        Mode::Generate if app.gen.pick_scatter => Some(BrushView { x: h[0], z: h[2], radius: app.gen.scatter_radius.max(10.0), falloff: 0.0, color: [0.4, 0.9, 0.5] }),
        _ => None,
    }
}

fn view_header(app: &mut App, ui: &mut egui::Ui, rect: Rect, top: bool) {
    let area = Rect::from_min_size(rect.min + vec2(6.0, 5.0), vec2(rect.width() - 12.0, 24.0));
    let width: f32 = if top { 190.0 } else { 570.0 };
    ui.painter().rect_filled(Rect::from_min_size(rect.min + vec2(2.0, 2.0), vec2(width.min(rect.width() - 4.0), 30.0)), 3.0, Color32::from_black_alpha(150));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let shading = if top { &mut app.shading2 } else { &mut app.shading3 };
    egui::ComboBox::from_id_salt(if top { "shade2" } else { "shade3" }).selected_text(t(shading.name())).width(110.0).show_ui(&mut child, |ui| {
        for s in Shading::ALL {
            ui.selectable_value(shading, s, t(s.name()));
        }
    });
    child.label(egui::RichText::new(if top { t("Top") } else { t("Perspective") }).color(Color32::WHITE).background_color(Color32::from_black_alpha(100)));
    if !top {
        child.checkbox(&mut app.wireframe, t("Wireframe"));
        let mut speed = app.camera.speed;
        child.add(egui::Slider::new(&mut speed, 1.0..=4000.0).logarithmic(true).suffix(" m/s").show_value(true).text(""));
        app.camera.speed = speed;
        lighting_menu(app, &mut child);
    }
}

fn lighting_menu(app: &mut App, ui: &mut egui::Ui) {
    use crate::settings::Lighting;
    ui.menu_button(t("Lighting"), |ui| {
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
        if ui.button(t("Reset")).clicked() {
            app.settings.lighting = Lighting::default();
        }
    });
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
        grid: [if app.show_land_grid { meta.land_cell() } else { 0.0 }, if app.show_tile_grid { meta.tile_step() } else { 0.0 }, if app.show_contours { 10.0 } else { 0.0 }],
        sea: meta.sea_level,
        height_range: doc.height_range,
        objects,
        show_water: app.show_water,
        world: meta.size_m(),
        tile: 10.0 / meta.land_cell(),
        lighting: app.settings.lighting.gpu(),
        light: (!top && app.show_shadows && app.settings.lighting.elevation > 1.0).then(|| sun_matrix(&v, app.settings.lighting.sun_direction())),
    };
    ui.painter_at(rect).add(render::callback(app.renderer.clone(), frame));
    draw_overlays(app, ui, &v, &resp);
    view_header(app, ui, rect, top);
    resp.context_menu(|ui| {
        let Some(m) = ctx.input(|i| i.pointer.latest_pos()) else { return };
        let _ = m;
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
}

fn shortcuts(app: &mut App, ctx: &egui::Context) {
    if ctx.wants_keyboard_input() {
        return;
    }
    let looking = ctx.input(|i| i.pointer.secondary_down());
    let (undo, redo, save, select_all, dup, del, focus, esc) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::COMMAND, Key::Z),
            i.consume_key(egui::Modifiers::COMMAND, Key::Y),
            i.consume_key(egui::Modifiers::COMMAND, Key::S),
            i.consume_key(egui::Modifiers::COMMAND, Key::A),
            i.consume_key(egui::Modifiers::COMMAND, Key::D),
            i.key_pressed(Key::Delete),
            i.key_pressed(Key::F),
            i.key_pressed(Key::Escape),
        )
    });
    if undo { app.undo(); }
    if redo { app.redo(); }
    if save { app.save(None); }
    if del { app.delete_selection(); }
    if dup { app.duplicate_selection(); }
    if focus { focus_selection(app); }
    if esc && app.mode == Mode::Select {
        app.selection.clear();
        app.selection_rev += 1;
    }
    if select_all {
        if let Some(d) = &app.doc {
            app.selection = d.world.objects.iter().map(|(id, _)| id).collect();
            app.selection_rev += 1;
        }
    }
    if !looking {
        ctx.input(|i| {
            if i.modifiers.is_none() {
                if i.key_pressed(Key::W) { app.gizmo = Gizmo::Move; }
                if i.key_pressed(Key::E) { app.gizmo = Gizmo::Rotate; }
                if i.key_pressed(Key::R) { app.gizmo = Gizmo::Scale; }
                if i.key_pressed(Key::Num1) { app.mode = Mode::Select; }
                if i.key_pressed(Key::Num2) { app.mode = Mode::Landscape; }
                if i.key_pressed(Key::Num3) { app.mode = Mode::Paint; }
                if i.key_pressed(Key::Num4) { app.mode = Mode::Place; }
                if i.key_pressed(Key::Num5) { app.mode = Mode::Lines; }
                if i.key_pressed(Key::Num6) { app.mode = Mode::Generate; }
                let f = if i.key_pressed(Key::CloseBracket) { 1.25 } else if i.key_pressed(Key::OpenBracket) { 0.8 } else { 1.0 };
                if f != 1.0 {
                    app.sculpt.radius = (app.sculpt.radius * f).clamp(2.0, 3000.0);
                    app.paint.radius = (app.paint.radius * f).clamp(0.5, 2000.0);
                }
            }
        });
    }
    let shift_scroll = ctx.input(|i| if i.modifiers.shift { i.smooth_scroll_delta.y } else { 0.0 });
    if shift_scroll != 0.0 {
        let f = (shift_scroll * 0.003).exp();
        app.sculpt.radius = (app.sculpt.radius * f).clamp(2.0, 3000.0);
        app.paint.radius = (app.paint.radius * f).clamp(0.5, 2000.0);
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
    shortcuts(app, &ctx);
    if app.doc.is_none() {
        welcome(app, ui);
        return;
    }
    let rect = ui.available_rect_before_wrap();
    match app.view_mode {
        ViewMode::Perspective => draw_view(app, ui, rect, false),
        ViewMode::Top => draw_view(app, ui, rect, true),
        ViewMode::Split => {
            let mid = rect.center().x;
            draw_view(app, ui, Rect::from_min_max(rect.min, pos2(mid - 1.0, rect.max.y)), false);
            draw_view(app, ui, Rect::from_min_max(pos2(mid + 1.0, rect.min.y), rect.max), true);
        }
    }
    let error = app.renderer.lock().unwrap().error.clone();
    if let Some(e) = error {
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, e, egui::FontId::monospace(11.0), theme::ERROR);
    }
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
        "wrp" => {
            let db = app.db.clone();
            app.spawn("wrp", move |job| {
                let w = dz_core::import::import_wrp(std::path::Path::new(&arg), None, &db, job)?;
                Ok(Box::new(move |a: &mut App| a.open_world(w, None)) as Finish)
            });
        }
        "demo" => {
            let mut w = dz_core::world::World::new(dz_core::world::WorldMeta::new("Demo", 2048, 5.0), 60.0);
            let params = gen::TerrainParams { kind: gen::TerrainKind::Island, ..Default::default() };
            app.spawn("demo", move |job| {
                gen::generate_terrain(&mut w.base, &params, job);
                w.recompose(None);
                let rules = gen::default_paint_rules(&w);
                w.mask.data = gen::auto_paint(&w, &rules, 1, job);
                Ok(Box::new(move |a: &mut App| a.open_world(w, None)) as Finish)
            });
        }
        "scatter" => {
            app.gen.scatter.models = vec![arg];
            app.gen.scatter.density = 40.0;
            crate::panels::run_scatter(app, None);
        }
        "mode" => {
            app.mode = match arg.as_str() {
                "sculpt" => Mode::Landscape,
                "paint" => Mode::Paint,
                "place" => Mode::Place,
                "lines" => Mode::Lines,
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
            app.camera.pos = [nums[0], nums[1], nums[2]];
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
        "mouse" if nums.len() >= 2 => app.synthetic.push(egui::Event::PointerMoved(pos2(nums[0], nums[1]))),
        "press" | "release" if nums.len() >= 2 => {
            let button = if nums.len() >= 3 && nums[2] == 1.0 { PointerButton::Secondary } else { PointerButton::Primary };
            app.synthetic.push(egui::Event::PointerButton { pos: pos2(nums[0], nums[1]), button, pressed: cmd == "press", modifiers: egui::Modifiers::NONE });
        }
        "key" => {
            if let Some(key) = Key::from_name(&arg) {
                app.synthetic.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
                app.synthetic.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE });
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
        "save" => app.save(Some(arg.into())),
        "build" => app.build(false),
        "browse" => app.filter = arg,
        "forest" => {
            let presets = gen::forest_preset_names();
            let names: Vec<String> = app.doc.as_ref().unwrap().world.layers.iter().map(|l| l.name.clone()).collect();
            let layer = names.iter().position(|n| n == "grass1").unwrap_or(0);
            let mut lite = app.doc.as_ref().unwrap().world.lite();
            let req = gen::ForestRequest { zones: vec![(layer, presets[0].clone())], seed: 5, max_slope: 35.0, clearing_strength: 0.35 };
            let assets = app.assets.clone();
            app.spawn("forest", move |job| {
                let (objects, notes) = gen::generate_forest(&mut lite, &assets, &req, job)?;
                let models: Vec<String> = lite.objects.models.iter().map(|m| m.path.clone()).collect();
                Ok(Box::new(move |a: &mut App| {
                    a.merge_generated("forest", models, objects);
                    for n in notes { a.info(n); }
                }) as Finish)
            });
        }
        "settle" if nums.len() >= 2 => {
            app.gen.settlement.buildings = vec![r"dz\structures\residential\houses\house_1w01.p3d".to_string(), r"dz\structures\residential\houses\house_1w02.p3d".to_string()];
            crate::panels::run_settlement(app, nums[0], nums[1]);
        }
        "report" => {
            let d = app.doc.as_ref();
            eprintln!("REPORT objects={} modifiers={} lines={} selection={} undo={:?} status={}", d.map(|d| d.world.objects.len()).unwrap_or(0), d.map(|d| d.world.modifiers.len()).unwrap_or(0), d.map(|d| d.world.lines.len()).unwrap_or(0), app.selection.len(), d.and_then(|d| d.history.undo_label().map(str::to_string)), app.status);
        }
        "quit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        _ => {}
    }
}

