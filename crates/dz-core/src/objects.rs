//! Placed objects: compact storage with a uniform grid index for very large maps.
use crate::terrain::Heightmap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Grid cell side of the spatial index in metres.
pub const GRID_CELL: f32 = 128.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obj {
    pub x: f32,
    pub z: f32,
    /// Height above the terrain of the model origin.
    pub elevation: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub scale: f32,
    pub model: u32,
    pub flags: u32,
}
pub const FLAG_UNDERGROUND: u32 = 1;
pub const FLAG_ABSOLUTE: u32 = 2;
/// Set when a modifier is attached to the object.
pub const FLAG_LOCKED: u32 = 4;
/// The object layer is kept in the upper half of `flags`.
const LAYER_SHIFT: u32 = 16;

impl Obj {
    #[inline]
    pub fn layer(&self) -> u16 {
        (self.flags >> LAYER_SHIFT) as u16
    }
    #[inline]
    pub fn set_layer(&mut self, layer: u16) {
        self.flags = self.flags & ((1 << LAYER_SHIFT) - 1) | (layer as u32) << LAYER_SHIFT;
    }
}

/// One model used on the map, interned so every object stores just an index.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct ModelRef {
    /// Path as written in the WRP, e.g. `dz\plants\tree\t_x.p3d`.
    pub path: String,
}

#[derive(Default, Clone)]
pub struct ObjectStore {
    pub models: Vec<ModelRef>,
    by_path: HashMap<String, u32>,
    /// Objects by id; `None` marks a free slot.
    slots: Vec<Option<Obj>>,
    free: Vec<u32>,
    alive: usize,
    grid: HashMap<(i32, i32), Vec<u32>>,
    /// Bumped on every change; lets views refresh lazily.
    pub revision: u64,
}

#[inline]
/// Rivers, ponds and streams; not the beds under them or buildings named after water.
pub fn is_water_model(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    (p.contains("\\water\\") || p.contains("\\water_")) && !p.contains("streambed")
}

pub fn grid_key(x: f32, z: f32) -> (i32, i32) {
    ((x / GRID_CELL).floor() as i32, (z / GRID_CELL).floor() as i32)
}

impl ObjectStore {
    pub fn len(&self) -> usize {
        self.alive
    }
    pub fn is_empty(&self) -> bool {
        self.alive == 0
    }
    pub fn normalize_path(path: &str) -> String {
        path.trim().replace('/', "\\").trim_start_matches('\\').to_ascii_lowercase()
    }
    pub fn intern(&mut self, path: &str) -> u32 {
        let key = Self::normalize_path(path);
        if let Some(&i) = self.by_path.get(&key) {
            return i;
        }
        let i = self.models.len() as u32;
        self.models.push(ModelRef { path: key.clone() });
        self.by_path.insert(key, i);
        i
    }
    pub fn get(&self, id: u32) -> Option<&Obj> {
        self.slots.get(id as usize).and_then(|s| s.as_ref())
    }
    pub fn insert(&mut self, o: Obj) -> u32 {
        let id = match self.free.pop() {
            Some(id) => {
                self.slots[id as usize] = Some(o);
                id
            }
            None => {
                self.slots.push(Some(o));
                self.slots.len() as u32 - 1
            }
        };
        self.index(id, &o);
        self.alive += 1;
        self.revision += 1;
        id
    }
    /// Restores an object under a known id (undo).
    pub fn insert_at(&mut self, id: u32, o: Obj) {
        let i = id as usize;
        if i >= self.slots.len() {
            let old = self.slots.len() as u32;
            self.slots.resize(i + 1, None);
            self.free.extend(old..id);
        } else if self.slots[i].is_some() {
            self.remove(id);
        }
        self.free.retain(|f| *f != id);
        self.slots[i] = Some(o);
        self.index(id, &o);
        self.alive += 1;
        self.revision += 1;
    }
    fn index(&mut self, id: u32, o: &Obj) {
        self.grid.entry(grid_key(o.x, o.z)).or_default().push(id);
    }
    fn unindex(&mut self, id: u32, o: &Obj) {
        let key = grid_key(o.x, o.z);
        if let Some(cell) = self.grid.get_mut(&key) {
            if let Some(p) = cell.iter().position(|v| *v == id) {
                cell.swap_remove(p);
            }
            if cell.is_empty() {
                self.grid.remove(&key);
            }
        }
    }
    pub fn remove(&mut self, id: u32) -> Option<Obj> {
        let o = self.slots.get_mut(id as usize)?.take()?;
        self.unindex(id, &o);
        self.free.push(id);
        self.alive -= 1;
        self.revision += 1;
        Some(o)
    }
    pub fn update(&mut self, id: u32, o: Obj) {
        let Some(old) = self.get(id).copied() else { return };
        if grid_key(old.x, old.z) != grid_key(o.x, o.z) {
            self.unindex(id, &old);
            self.index(id, &o);
        }
        self.slots[id as usize] = Some(o);
        self.revision += 1;
    }
    pub fn iter(&self) -> impl Iterator<Item = (u32, &Obj)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|o| (i as u32, o)))
    }
    /// Ids of objects whose grid cell touches the rectangle.
    pub fn for_each_in(&self, x0: f32, z0: f32, x1: f32, z1: f32, mut f: impl FnMut(u32, &Obj)) {
        let (a, b) = grid_key(x0, z0);
        let (c, d) = grid_key(x1, z1);
        // A huge query is cheaper over the occupied cells than over every possible one.
        if ((c - a + 1) as i64) * ((d - b + 1) as i64) > self.grid.len() as i64 {
            for ((gx, gz), cell) in &self.grid {
                if *gx >= a && *gx <= c && *gz >= b && *gz <= d {
                    for &id in cell {
                        if let Some(o) = self.get(id) {
                            f(id, o);
                        }
                    }
                }
            }
            return;
        }
        for gz in b..=d {
            for gx in a..=c {
                if let Some(cell) = self.grid.get(&(gx, gz)) {
                    for &id in cell {
                        if let Some(o) = self.get(id) {
                            f(id, o);
                        }
                    }
                }
            }
        }
    }
    pub fn cell(&self, key: (i32, i32)) -> &[u32] {
        self.grid.get(&key).map(|v| v.as_slice()).unwrap_or(&[])
    }
    pub fn model_path(&self, o: &Obj) -> &str {
        &self.models[o.model as usize].path
    }
    /// Y of the model origin in world space.
    #[inline]
    pub fn origin_y(o: &Obj, ground: &Heightmap) -> f32 {
        if o.flags & FLAG_ABSOLUTE != 0 {
            o.elevation
        } else {
            ground.sample(o.x, o.z) + o.elevation
        }
    }
    /// Objects per layer; `layers` is the length of the result.
    pub fn layer_counts(&self, layers: usize) -> Vec<usize> {
        let mut n = vec![0usize; layers];
        for (_, o) in self.iter() {
            if let Some(c) = n.get_mut(o.layer() as usize) {
                *c += 1;
            }
        }
        n
    }
    /// Counts per model, most used first.
    pub fn histogram(&self) -> Vec<(u32, usize)> {
        let mut n = vec![0usize; self.models.len()];
        for (_, o) in self.iter() {
            n[o.model as usize] += 1;
        }
        let mut v: Vec<_> = n.into_iter().enumerate().filter(|(_, c)| *c > 0).map(|(i, c)| (i as u32, c)).collect();
        v.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        v
    }

    // Binary persistence: compact and fast for millions of objects.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.alive * 36 + self.models.len() * 48);
        out.extend_from_slice(b"DZOB\x01\0\0\0");
        out.extend_from_slice(&(self.models.len() as u32).to_le_bytes());
        for m in &self.models {
            out.extend_from_slice(&(m.path.len() as u32).to_le_bytes());
            out.extend_from_slice(m.path.as_bytes());
        }
        out.extend_from_slice(&(self.alive as u32).to_le_bytes());
        for (_, o) in self.iter() {
            for v in [o.x, o.z, o.elevation, o.yaw, o.pitch, o.roll, o.scale] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&o.model.to_le_bytes());
            out.extend_from_slice(&o.flags.to_le_bytes());
        }
        out
    }
    pub fn from_bytes(b: &[u8]) -> anyhow::Result<Self> {
        use anyhow::{ensure, Context};
        ensure!(b.starts_with(b"DZOB\x01"), "Unsupported object file");
        let u = |p: usize| -> anyhow::Result<u32> {
            Ok(u32::from_le_bytes(b.get(p..p + 4).context("Truncated object file")?.try_into()?))
        };
        let mut p = 8;
        let mut s = ObjectStore::default();
        let n = u(p)? as usize;
        p += 4;
        for _ in 0..n {
            let l = u(p)? as usize;
            p += 4;
            let name = std::str::from_utf8(b.get(p..p + l).context("Truncated object file")?)?;
            s.intern(name);
            p += l;
        }
        let n = u(p)? as usize;
        p += 4;
        ensure!(b.len() >= p + n * 36, "Truncated object file");
        s.slots.reserve(n);
        for _ in 0..n {
            let f = |i: usize| f32::from_le_bytes(b[p + i * 4..p + i * 4 + 4].try_into().unwrap());
            let o = Obj {
                x: f(0), z: f(1), elevation: f(2), yaw: f(3), pitch: f(4), roll: f(5), scale: f(6),
                model: u32::from_le_bytes(b[p + 28..p + 32].try_into().unwrap()),
                flags: u32::from_le_bytes(b[p + 32..p + 36].try_into().unwrap()),
            };
            s.insert(o);
            p += 36;
        }
        Ok(s)
    }
}

/// Rotation matrix columns (right, up, dir) of Terrain Builder angles in degrees.
pub fn rotation(yaw: f32, pitch: f32, roll: f32, scale: f32) -> [[f32; 3]; 3] {
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sp, cp) = pitch.to_radians().sin_cos();
    let (sr, cr) = roll.to_radians().sin_cos();
    // R = Ry(yaw) * Rx(pitch) * Rz(roll), with yaw clockwise seen from above.
    let right = [cy * cr + sy * sp * sr, cp * sr, -sy * cr + cy * sp * sr];
    let up = [-cy * sr + sy * sp * cr, cp * cr, sy * sr + cy * sp * cr];
    let dir = [sy * cp, -sp, cy * cp];
    [
        [right[0] * scale, right[1] * scale, right[2] * scale],
        [up[0] * scale, up[1] * scale, up[2] * scale],
        [dir[0] * scale, dir[1] * scale, dir[2] * scale],
    ]
}

/// Editor angles of an orientation written by Terrain Builder, whose rotation is
/// Rz(-roll) * Rx(-pitch) * Ry(yaw): the tilts are about world axes, applied after the yaw.
pub fn from_terrain_builder(yaw: f32, pitch: f32, roll: f32) -> (f32, f32, f32) {
    if pitch == 0.0 && roll == 0.0 {
        return (yaw, 0.0, 0.0);
    }
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sp, cp) = (-pitch).to_radians().sin_cos();
    let (sr, cr) = (-roll).to_radians().sin_cos();
    let right = [cr * cy - sr * sp * sy, sr * cy + cr * sp * sy, -cp * sy];
    let up = [-sr * cp, cr * cp, sp];
    let dir = [cr * sy + sr * sp * cy, sr * sy - cr * sp * cy, cp * cy];
    let (a, b, c, _) = angles(&[right, up, dir]);
    (a, b, c)
}

/// Inverse of [`from_terrain_builder`].
pub fn to_terrain_builder(yaw: f32, pitch: f32, roll: f32) -> (f32, f32, f32) {
    if pitch == 0.0 && roll == 0.0 {
        return (yaw, 0.0, 0.0);
    }
    let m = rotation(yaw, pitch, roll, 1.0);
    let sp = m[1][2].clamp(-1.0, 1.0);
    ((-m[0][2]).atan2(m[2][2]).to_degrees(), -sp.asin().to_degrees(), m[1][0].atan2(m[1][1]).to_degrees())
}

/// Inverse of [`rotation`]: (yaw, pitch, roll, scale).
pub fn angles(m: &[[f32; 3]; 3]) -> (f32, f32, f32, f32) {
    let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let scale = len(m[2]);
    if scale <= 1e-6 {
        return (0.0, 0.0, 0.0, 1.0);
    }
    let d = [m[2][0] / scale, m[2][1] / scale, m[2][2] / scale];
    let pitch = (-d[1]).clamp(-1.0, 1.0).asin();
    let yaw = d[0].atan2(d[2]);
    let roll = if pitch.cos() > 1e-5 { m[0][1].atan2(m[1][1]) } else { 0.0 };
    (yaw.to_degrees(), pitch.to_degrees(), roll.to_degrees(), scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn obj(x: f32, z: f32) -> Obj {
        Obj { x, z, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 }
    }
    #[test]
    fn angles_invert_rotation() {
        for (y, p, r, s) in [(30.0, 10.0, -5.0, 1.0), (-120.0, -20.0, 40.0, 2.5), (0.0, 0.0, 0.0, 1.0)] {
            let (a, b, c, d) = angles(&rotation(y, p, r, s));
            assert!((a - y).abs() < 1e-3 && (b - p).abs() < 1e-3 && (c - r).abs() < 1e-3 && (d - s).abs() < 1e-3, "{a} {b} {c} {d}");
        }
    }
    #[test]
    fn terrain_builder_angles_convert_both_ways() {
        // Read from a real layer and the world built from it.
        let (y, p, r) = from_terrain_builder(19.981888, 2.786426, 358.9851);
        let m = rotation(y, p, r, 1.0);
        let (ty, tp, tr) = to_terrain_builder(y, p, r);
        assert!((ty - 19.981888).abs() < 1e-3 && (tp - 2.786426).abs() < 1e-3 && (tr.rem_euclid(360.0) - 358.9851).abs() < 1e-3, "{ty} {tp} {tr}");
        assert!((p + 2.965).abs() < 0.01 && r.abs() < 0.05, "{p} {r}");
        assert!((m[1][1] - p.to_radians().cos() * r.to_radians().cos()).abs() < 1e-5);
        assert_eq!(from_terrain_builder(33.0, 0.0, 0.0), (33.0, 0.0, 0.0));
    }
    #[test]
    fn matches_terrain_builder_yaw_in_a_real_wrp() {
        // Matrix read from dolinabobru.wrp: right, up, dir.
        let m = rotation(-86.5, 0.0, 0.0, 1.0);
        assert!((m[0][0] - 0.061).abs() < 0.002 && (m[0][2] - 0.998).abs() < 0.002);
        assert!((m[2][0] + 0.998).abs() < 0.002 && (m[2][2] - 0.061).abs() < 0.002);
    }
    #[test]
    fn store_reuses_ids_and_keeps_grid_in_sync() {
        let mut s = ObjectStore::default();
        let a = s.insert(obj(10.0, 10.0));
        let b = s.insert(obj(500.0, 500.0));
        assert_eq!(s.len(), 2);
        s.remove(a);
        let mut found = vec![];
        s.for_each_in(0.0, 0.0, 1000.0, 1000.0, |id, _| found.push(id));
        assert_eq!(found, vec![b]);
        s.update(b, obj(900.0, 20.0));
        let mut found = 0;
        s.for_each_in(896.0, 0.0, 1000.0, 100.0, |_, _| found += 1);
        assert_eq!(found, 1);
        assert_eq!(s.insert(obj(1.0, 1.0)), a);
    }
    #[test]
    fn layer_shares_flags_without_touching_them() {
        let mut o = Obj { flags: FLAG_UNDERGROUND | FLAG_ABSOLUTE, ..obj(0.0, 0.0) };
        assert_eq!(o.layer(), 0);
        o.set_layer(513);
        assert_eq!((o.layer(), o.flags & 0xffff), (513, FLAG_UNDERGROUND | FLAG_ABSOLUTE));
        o.flags ^= FLAG_ABSOLUTE;
        o.set_layer(7);
        assert_eq!((o.layer(), o.flags & 0xffff), (7, FLAG_UNDERGROUND));
    }
    #[test]
    fn binary_round_trip() {
        let mut s = ObjectStore::default();
        let m = s.intern("DZ/plants/tree.p3d");
        s.insert(Obj { model: m, flags: FLAG_UNDERGROUND, ..obj(1.5, 2.5) });
        let t = ObjectStore::from_bytes(&s.to_bytes()).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t.models[0].path, "dz\\plants\\tree.p3d");
        assert_eq!(t.iter().next().unwrap().1.flags, FLAG_UNDERGROUND);
    }
}
