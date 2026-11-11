//! Brushes and undo history.
use crate::noise::Perlin;
use crate::objects::Obj;
use crate::terrain::{Heightmap, MaskMap, Rect};
use crate::world::{TerrainModifier, World};
use std::collections::HashMap;

/// Side of the snapshot tiles used by undo.
const TILE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SculptTool {
    Raise,
    Lower,
    Smooth,
    Flatten,
    Noise,
    Ramp,
}

#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub radius: f32,
    /// 0 = hard edge, 1 = soft all the way from the centre.
    pub falloff: f32,
    /// Metres per second for raise and lower, blend rate for the others.
    pub strength: f32,
}

impl Brush {
    #[inline]
    pub fn weight(&self, dist: f32) -> f32 {
        if dist >= self.radius {
            return 0.0;
        }
        let inner = self.radius * (1.0 - self.falloff);
        if dist <= inner {
            return 1.0;
        }
        let t = 1.0 - (dist - inner) / (self.radius - inner).max(1e-3);
        t * t * (3.0 - 2.0 * t)
    }
}

/// Outline of a brush.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BrushShape {
    #[default]
    Circle,
    Square,
    Diamond,
}

impl BrushShape {
    /// Distance from the brush centre in this outline's metric.
    #[inline]
    pub fn distance(self, dx: f32, dz: f32) -> f32 {
        match self {
            BrushShape::Circle => (dx * dx + dz * dz).sqrt(),
            BrushShape::Square => dx.abs().max(dz.abs()),
            BrushShape::Diamond => dx.abs() + dz.abs(),
        }
    }
}

/// Copy-on-write snapshot of a raster's tiles touched by one stroke.
#[derive(Default, Clone)]
pub struct TileSnapshot<T: Copy> {
    tiles: HashMap<(usize, usize), Vec<T>>,
}

impl<T: Copy> TileSnapshot<T> {
    fn capture(&mut self, data: &[T], cols: usize, rows: usize, r: Rect) {
        for ty in r.y0 / TILE..=r.y1 / TILE {
            for tx in r.x0 / TILE..=r.x1 / TILE {
                self.tiles.entry((tx, ty)).or_insert_with(|| {
                    let mut out = Vec::with_capacity(TILE * TILE);
                    for y in ty * TILE..((ty + 1) * TILE).min(rows) {
                        out.extend_from_slice(&data[y * cols + tx * TILE..y * cols + ((tx + 1) * TILE).min(cols)]);
                    }
                    out
                });
            }
        }
    }
    /// Exchanges the stored tiles with the raster and returns the area.
    fn swap(&mut self, data: &mut [T], cols: usize, rows: usize) -> Option<Rect> {
        let mut area: Option<Rect> = None;
        for (&(tx, ty), tile) in &mut self.tiles {
            let w = ((tx + 1) * TILE).min(cols) - tx * TILE;
            let mut p = 0;
            for y in ty * TILE..((ty + 1) * TILE).min(rows) {
                let s = y * cols + tx * TILE;
                data[s..s + w].swap_with_slice(&mut tile[p..p + w]);
                p += w;
            }
            let r = Rect { x0: tx * TILE, y0: ty * TILE, x1: ((tx + 1) * TILE).min(cols) - 1, y1: ((ty + 1) * TILE).min(rows) - 1 };
            area = Some(area.map_or(r, |a| a.union(r)));
        }
        area
    }
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
    fn bytes(&self) -> usize {
        self.tiles.len() * TILE * TILE * std::mem::size_of::<T>()
    }
}

/// One undoable change.
pub enum Change {
    Height(TileSnapshot<f32>),
    Mask(TileSnapshot<u8>),
    /// (id, before, after); `None` means the object does not exist in that state.
    Objects(Vec<(u32, Option<Obj>, Option<Obj>)>),
    Modifiers(Vec<TerrainModifier>),
    ObjectLayers(Vec<crate::world::ObjectLayer>),
    Shapes(Vec<crate::shapes::ShapeLayer>),
    Routes(Vec<crate::world::RoadRoute>),
    Holes(crate::holes::Holes),
    KeyPoints(Vec<crate::keypoints::KeyPoint>),
    Generators(Vec<crate::library::GeneratorInstance>),
    /// The satellite modifiers as they were.
    Satellite(Vec<crate::satmod::SatModifier>),
    /// The mask modifiers as they were.
    MaskMods(Vec<crate::maskmod::MaskModifier>),
    /// A whole-raster change produced by a generator.
    WholeHeight(Vec<f32>),
    WholeMask(Vec<u8>),
    Group(Vec<Change>),
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Touched {
    pub height: Option<Rect>,
    pub mask: Option<Rect>,
    pub objects: bool,
    /// Pixels of the satellite image that changed.
    pub satellite: Option<crate::satmod::PixelRect>,
}
impl Touched {
    fn merge(&mut self, o: Touched) {
        self.height = match (self.height, o.height) {
            (Some(a), Some(b)) => Some(a.union(b)),
            (a, b) => a.or(b),
        };
        self.mask = match (self.mask, o.mask) {
            (Some(a), Some(b)) => Some(a.union(b)),
            (a, b) => a.or(b),
        };
        self.objects |= o.objects;
        self.satellite = match (self.satellite, o.satellite) {
            (Some(a), Some(b)) => Some(a.union(b)),
            (a, b) => a.or(b),
        };
    }
}

impl Change {
    /// Applies the change and turns it into its own inverse.
    fn toggle(&mut self, w: &mut World) -> Touched {
        let mut t = Touched::default();
        match self {
            Change::Height(s) => {
                let (c, r) = (w.base.cols, w.base.rows);
                t.height = s.swap(&mut w.base.data, c, r);
                if let Some(a) = t.height {
                    w.recompose(Some(a));
                }
            }
            Change::Mask(s) => {
                let (c, r) = (w.mask.width, w.mask.height);
                t.mask = s.swap(&mut w.mask.data, c, r);
            }
            Change::Objects(list) => {
                for (id, before, after) in list.iter_mut().rev() {
                    match *before {
                        Some(o) => {
                            if w.objects.get(*id).is_some() {
                                w.objects.update(*id, o);
                            } else {
                                w.objects.insert_at(*id, o);
                            }
                        }
                        None => {
                            w.objects.remove(*id);
                        }
                    }
                    std::mem::swap(before, after);
                }
                t.objects = true;
            }
            Change::Modifiers(prev) => {
                std::mem::swap(prev, &mut w.modifiers);
                w.recompose(None);
                t.height = Some(w.base.full_rect());
                t.objects = true;
            }
            Change::ObjectLayers(prev) => {
                std::mem::swap(prev, &mut w.object_layers);
                w.objects.revision += 1;
                t.objects = true;
            }
            Change::Shapes(prev) => std::mem::swap(prev, &mut w.shape_layers),
            Change::KeyPoints(prev) => std::mem::swap(prev, &mut w.keypoints),
            Change::Generators(prev) => std::mem::swap(prev, &mut w.generators),
            Change::Satellite(prev) => t.satellite = w.sat_set_modifiers(prev),
            Change::MaskMods(prev) => t.mask = w.mask_set_modifiers(prev),
            Change::Routes(routes) => std::mem::swap(routes, &mut w.routes),
            Change::Holes(holes) => {
                std::mem::swap(holes, &mut w.holes);
                w.holes_rev += 1;
            }
            Change::WholeHeight(v) => {
                std::mem::swap(v, &mut w.base.data);
                w.recompose(None);
                t.height = Some(w.base.full_rect());
                t.objects = true;
            }
            Change::WholeMask(v) => {
                std::mem::swap(v, &mut w.mask.data);
                t.mask = Some(Rect { x0: 0, y0: 0, x1: w.mask.width - 1, y1: w.mask.height - 1 });
            }
            Change::Group(list) => {
                // Stored oldest first on the undo side; each toggle reverses the order.
                list.reverse();
                for c in list.iter_mut() {
                    t.merge(c.toggle(w));
                }
            }
        }
        t
    }
    fn bytes(&self) -> usize {
        match self {
            Change::Height(s) => s.bytes(),
            Change::Mask(s) => s.bytes(),
            Change::Objects(l) => l.len() * 80,
            Change::Modifiers(l) => l.len() * 96,
            Change::ObjectLayers(l) => l.len() * 64,
            Change::Routes(r) => r.iter().map(|x| x.points.len() * 8 + x.centerline.len() * 8 + x.objects.len() * 4).sum(),
            Change::Holes(h) => h.len() * 8,
            Change::KeyPoints(k) => k.len() * 96,
            Change::Generators(l) => l.iter().map(|g| g.objects.len() * 4 + 4096).sum(),
            // Tiles are shared with the live modifiers until one of them is painted on.
            Change::Satellite(l) => l.iter().map(|m| m.tiles.len() * 4096 + 64).sum(),
            Change::MaskMods(l) => l.iter().map(|m| m.tiles.len() * 1024 + 64).sum(),
            Change::Shapes(l) => l.iter().map(|s| s.shapes.iter().map(|x| x.points.len() * 8 + 64).sum::<usize>()).sum(),
            Change::WholeHeight(v) => v.len() * 4,
            Change::WholeMask(v) => v.len(),
            Change::Group(l) => l.iter().map(|c| c.bytes()).sum(),
        }
    }
}

pub struct History {
    undo: Vec<(String, Change)>,
    redo: Vec<(String, Change)>,
    budget: usize,
    /// Counts every edit; compares with the saved revision.
    pub revision: u64,
    pub saved_revision: u64,
}
impl Default for History {
    fn default() -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), budget: 2 * 1024 * 1024 * 1024, revision: 0, saved_revision: 0 }
    }
}
impl History {
    pub fn dirty(&self) -> bool {
        self.revision != self.saved_revision
    }
    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }
    pub fn push(&mut self, label: impl Into<String>, change: Change) {
        self.undo.push((label.into(), change));
        self.redo.clear();
        self.revision += 1;
        let mut total: usize = self.undo.iter().map(|(_, c)| c.bytes()).sum();
        while total > self.budget && self.undo.len() > 1 {
            total -= self.undo.remove(0).1.bytes();
        }
    }
    /// Joins the most recent `n` entries so one undo reverts them together.
    pub fn group_last(&mut self, n: usize, label: impl Into<String>) {
        if n < 2 || self.undo.len() < n {
            return;
        }
        let at = self.undo.len() - n;
        let group: Vec<Change> = self.undo.drain(at..).map(|(_, c)| c).collect();
        self.undo.push((label.into(), Change::Group(group)));
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|(l, _)| l.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|(l, _)| l.as_str())
    }
    pub fn undo(&mut self, w: &mut World) -> Option<Touched> {
        let (l, mut c) = self.undo.pop()?;
        let t = c.toggle(w);
        self.redo.push((l, c));
        self.revision += 1;
        Some(t)
    }
    pub fn redo(&mut self, w: &mut World) -> Option<Touched> {
        let (l, mut c) = self.redo.pop()?;
        let t = c.toggle(w);
        self.undo.push((l, c));
        self.revision += 1;
        Some(t)
    }
    pub fn labels(&self) -> Vec<String> {
        self.undo.iter().map(|(l, _)| l.clone()).collect()
    }
}

/// State of a sculpt stroke between mouse press and release.
pub struct SculptStroke {
    pub tool: SculptTool,
    pub brush: Brush,
    pub snapshot: TileSnapshot<f32>,
    /// Height picked at the start for flatten.
    pub target: f32,
    /// Start of the ramp.
    pub start: [f32; 2],
    pub noise: Perlin,
    pub shape: BrushShape,
    /// How much the edge of the brush is broken up, 0..1.
    pub edge_noise: f32,
}

impl SculptStroke {
    pub fn begin(tool: SculptTool, brush: Brush, w: &World, x: f32, z: f32) -> Self {
        Self { tool, brush, snapshot: TileSnapshot::default(), target: w.base.sample(x, z), start: [x, z], noise: Perlin::new(1234), shape: BrushShape::Circle, edge_noise: 0.0 }
    }
    /// Applies one step of the stroke at a world position; `dt` in seconds.
    pub fn apply(&mut self, w: &mut World, x: f32, z: f32, dt: f32) -> Option<Rect> {
        let r = w.base.rect_around(x, z, self.brush.radius)?;
        let (cols, rows) = (w.base.cols, w.base.rows);
        self.snapshot.capture(&w.base.data, cols, rows, r);
        let cell = w.base.cell;
        let rate = self.brush.strength * dt;
        let blend = (rate * 0.12).clamp(0.0, 1.0);
        let source: Vec<f32> = if self.tool == SculptTool::Smooth { copy_rect(&w.base, r) } else { Vec::new() };
        let stride = r.width();
        for j in r.y0..=r.y1 {
            for i in r.x0..=r.x1 {
                let (px, pz) = (i as f32 * cell, j as f32 * cell);
                let mut d = self.shape.distance(px - x, pz - z);
                if self.edge_noise > 0.0 {
                    let scale = (self.brush.radius * 0.4).max(1.0);
                    d += self.noise.fbm(px / scale, pz / scale, 3, 0.5, 2.0) * self.edge_noise * self.brush.radius;
                }
                let k = self.brush.weight(d);
                if k <= 0.0 {
                    continue;
                }
                let h = w.base.at(i, j);
                let v = match self.tool {
                    SculptTool::Raise => h + rate * k,
                    SculptTool::Lower => h - rate * k,
                    SculptTool::Flatten => h + (self.target - h) * (blend * 4.0 * k).min(1.0),
                    SculptTool::Noise => h + self.noise.fbm(px / 60.0, pz / 60.0, 4, 0.5, 2.0) * rate * k,
                    SculptTool::Smooth => {
                        let get = |a: i64, b: i64| {
                            let ax = (a.clamp(r.x0 as i64, r.x1 as i64) as usize) - r.x0;
                            let bx = (b.clamp(r.y0 as i64, r.y1 as i64) as usize) - r.y0;
                            source[bx * stride + ax]
                        };
                        let (a, b) = (i as i64, j as i64);
                        let avg = (get(a - 1, b) + get(a + 1, b) + get(a, b - 1) + get(a, b + 1) + get(a - 1, b - 1) + get(a + 1, b + 1) + get(a - 1, b + 1) + get(a + 1, b - 1)) / 8.0;
                        h + (avg - h) * (blend * 8.0 * k).min(1.0)
                    }
                    SculptTool::Ramp => {
                        let (dx, dz) = (x - self.start[0], z - self.start[1]);
                        let l2 = (dx * dx + dz * dz).max(1.0);
                        let t = (((px - self.start[0]) * dx + (pz - self.start[1]) * dz) / l2).clamp(0.0, 1.0);
                        let end = w.height.sample(x, z);
                        let goal = self.target + (end - self.target) * t;
                        h + (goal - h) * (blend * 6.0 * k).min(1.0)
                    }
                };
                w.base.set(i, j, v);
            }
        }
        w.recompose(Some(r));
        Some(r)
    }
    pub fn finish(self) -> Option<Change> {
        (!self.snapshot.is_empty()).then_some(Change::Height(self.snapshot))
    }
}

fn copy_rect(h: &Heightmap, r: Rect) -> Vec<f32> {
    let mut out = Vec::with_capacity(r.width() * r.height());
    for j in r.y0..=r.y1 {
        out.extend_from_slice(&h.data[j * h.cols + r.x0..=j * h.cols + r.x1]);
    }
    out
}

/// Paints surface layers into the mask.
#[derive(Default)]
pub struct MaskStroke {
    pub snapshot: TileSnapshot<u8>,
    pub shape: BrushShape,
    /// Paints only over this surface.
    pub only_on: Option<u8>,
    /// Keeps tiles within the project's surface limit.
    pub guard: Option<crate::mask_limit::TileGuard>,
}

impl MaskStroke {
    /// `hardness` breaks the edge up with noise so surfaces do not meet on a clean circle.
    pub fn apply(&mut self, mask: &mut MaskMap, size_m: f32, x: f32, z: f32, radius: f32, layer: u8, scatter: f32) -> Option<Rect> {
        let px = mask.width as f32 / size_m;
        let (cx, cy) = (x * px, (1.0 - z / size_m) * mask.height as f32);
        let rp = (radius * px).max(0.5);
        let (x0, y0) = (((cx - rp).floor() as i64).max(0), ((cy - rp).floor() as i64).max(0));
        let (x1, y1) = (((cx + rp).ceil() as i64).min(mask.width as i64 - 1), ((cy + rp).ceil() as i64).min(mask.height as i64 - 1));
        if x0 > x1 || y0 > y1 {
            return None;
        }
        let r = Rect { x0: x0 as usize, y0: y0 as usize, x1: x1 as usize, y1: y1 as usize };
        self.snapshot.capture(&mask.data, mask.width, mask.height, r);
        for y in r.y0..=r.y1 {
            for xp in r.x0..=r.x1 {
                let d = self.shape.distance(xp as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let limit = if scatter > 0.0 {
                    let h = (xp as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
                    let n = ((h.wrapping_mul(2654435761) >> 16) & 255) as f32 / 255.0;
                    rp * (1.0 - scatter * n)
                } else {
                    rp
                };
                if d > limit {
                    continue;
                }
                let at = y * mask.width + xp;
                if mask.data[at] == layer || self.only_on.is_some_and(|l| l != mask.data[at]) {
                    continue;
                }
                if self.guard.as_mut().is_some_and(|g| !g.allow(mask, xp, y, layer)) {
                    continue;
                }
                mask.data[at] = layer;
            }
        }
        Some(r)
    }
    pub fn finish(self) -> Option<Change> {
        (!self.snapshot.is_empty()).then_some(Change::Mask(self.snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::WorldMeta;

    fn world() -> World {
        World::new(WorldMeta::new("T", 256, 8.0), 100.0)
    }
    #[test]
    fn raise_undo_redo_restores_exact_heights() {
        let mut w = world();
        let mut h = History::default();
        let brush = Brush { radius: 100.0, falloff: 0.5, strength: 10.0 };
        let mut s = SculptStroke::begin(SculptTool::Raise, brush, &w, 500.0, 500.0);
        s.apply(&mut w, 500.0, 500.0, 1.0);
        s.apply(&mut w, 540.0, 500.0, 1.0);
        let raised = w.base.data.clone();
        assert!(w.height.sample(500.0, 500.0) > 105.0);
        h.push("sculpt", s.finish().unwrap());
        assert!(h.dirty());
        h.undo(&mut w).unwrap();
        assert!(w.base.data.iter().all(|v| *v == 100.0));
        assert!(w.height.data.iter().all(|v| *v == 100.0));
        h.redo(&mut w).unwrap();
        assert_eq!(w.base.data, raised);
    }
    #[test]
    fn flatten_moves_toward_start_height() {
        let mut w = world();
        for j in 0..256 { for i in 0..256 { w.base.set(i, j, i as f32); } }
        w.recompose(None);
        let brush = Brush { radius: 200.0, falloff: 0.0, strength: 30.0 };
        let mut s = SculptStroke::begin(SculptTool::Flatten, brush, &w, 800.0, 800.0);
        for _ in 0..30 { s.apply(&mut w, 800.0, 800.0, 0.1); }
        assert!((w.base.sample(900.0, 800.0) - 100.0).abs() < 1.0);
    }
    #[test]
    fn smooth_reduces_a_spike() {
        let mut w = world();
        w.base.set(100, 100, 200.0);
        w.recompose(None);
        let brush = Brush { radius: 50.0, falloff: 0.0, strength: 30.0 };
        let mut s = SculptStroke::begin(SculptTool::Smooth, brush, &w, 800.0, 800.0);
        s.apply(&mut w, 800.0, 800.0, 0.3);
        assert!(w.base.at(100, 100) < 120.0);
    }
    #[test]
    fn mask_stroke_paints_and_undoes() {
        let mut w = world();
        let mut h = History::default();
        let size = w.meta.size_m();
        let mut m = MaskStroke::default();
        m.apply(&mut w.mask, size, 1000.0, 1000.0, 20.0, 3, 0.0).unwrap();
        assert_eq!(w.mask.sample(1000.0, 1000.0, size), 3);
        assert_eq!(w.mask.sample(1100.0, 1000.0, size), 0);
        h.push("paint", m.finish().unwrap());
        h.undo(&mut w).unwrap();
        assert_eq!(w.mask.sample(1000.0, 1000.0, size), 0);
    }
    #[test]
    fn grouped_changes_undo_together() {
        let mut w = world();
        let mut h = History::default();
        let o = Obj { x: 5.0, z: 5.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        let id = w.objects.insert(o);
        h.push("place", Change::Objects(vec![(id, None, Some(o))]));
        let brush = Brush { radius: 100.0, falloff: 0.0, strength: 10.0 };
        let mut s = SculptStroke::begin(SculptTool::Raise, brush, &w, 500.0, 500.0);
        s.apply(&mut w, 500.0, 500.0, 1.0);
        h.push("sculpt", s.finish().unwrap());
        h.group_last(2, "both");
        assert_eq!(h.undo_label(), Some("both"));
        h.undo(&mut w).unwrap();
        assert_eq!(w.objects.len(), 0);
        assert_eq!(w.base.sample(500.0, 500.0), 100.0);
        h.redo(&mut w).unwrap();
        assert_eq!(w.objects.len(), 1);
        assert!(w.base.sample(500.0, 500.0) > 100.0);
        h.undo(&mut w).unwrap();
        assert_eq!(w.objects.len(), 0);
    }
    #[test]
    fn object_changes_are_reversible() {
        let mut w = world();
        let mut h = History::default();
        let o = Obj { x: 5.0, z: 5.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        let id = w.objects.insert(o);
        h.push("place", Change::Objects(vec![(id, None, Some(o))]));
        h.undo(&mut w).unwrap();
        assert_eq!(w.objects.len(), 0);
        h.redo(&mut w).unwrap();
        assert_eq!(w.objects.get(id), Some(&o));
    }
}
