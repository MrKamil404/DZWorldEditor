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
pub const LAYER_SHIFT: u32 = 16;

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
    /// Objects by id; `None` marks a removed object. Slots are not reused, so an id and the
    /// identity in `uids` stay with one object through undo and redo.
    slots: Vec<Option<Obj>>,
    /// Identity of the object in each slot that lasts across saves and copies of the project,
    /// so the work of several people can be merged. Kept after removal for undo.
    uids: Vec<u64>,
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

/// A random identity, never 0.
pub fn new_uid() -> u64 {
    loop {
        let v: u64 = rand::random();
        if v != 0 {
            return v;
        }
    }
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
    /// The path without a drive letter, so a model of `P:` or of `D:` is the same model.
    /// Models are kept relative to the work drive, whichever letter the drive has.
    pub fn strip_drive(path: &str) -> &str {
        let b = path.as_bytes();
        if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/') { &path[3..] } else { path }
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
    /// Lasting identity of the object `id`; 0 when there is none.
    pub fn uid(&self, id: u32) -> u64 {
        if self.get(id).is_some() { self.uids[id as usize] } else { 0 }
    }
    /// Ids by lasting identity, of the objects on the map.
    pub fn ids_by_uid(&self) -> HashMap<u64, u32> {
        self.iter().map(|(id, _)| (self.uids[id as usize], id)).collect()
    }
    pub fn get(&self, id: u32) -> Option<&Obj> {
        self.slots.get(id as usize).and_then(|s| s.as_ref())
    }
    pub fn insert(&mut self, o: Obj) -> u32 {
        self.insert_with_uid(o, 0)
    }
    /// Adds an object with a known identity; 0 gives it a new one.
    pub fn insert_with_uid(&mut self, o: Obj, uid: u64) -> u32 {
        self.slots.push(Some(o));
        self.uids.push(if uid == 0 { new_uid() } else { uid });
        let id = self.slots.len() as u32 - 1;
        self.index(id, &o);
        self.alive += 1;
        self.revision += 1;
        id
    }
    /// Restores an object under a known id (undo).
    pub fn insert_at(&mut self, id: u32, o: Obj) {
        let i = id as usize;
        if i >= self.slots.len() {
            self.slots.resize(i + 1, None);
            self.uids.resize(i + 1, 0);
        } else if self.slots[i].is_some() {
            self.remove(id);
        }
        if self.uids[i] == 0 {
            self.uids[i] = new_uid();
        }
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
        out.extend_from_slice(b"DZOB\x02\0\0\0");
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
        // Identities follow the objects, in the same order.
        for (id, _) in self.iter() {
            out.extend_from_slice(&self.uids[id as usize].to_le_bytes());
        }
        out
    }
    pub fn from_bytes(b: &[u8]) -> anyhow::Result<Self> {
        use anyhow::{ensure, Context};
        ensure!(b.starts_with(b"DZOB\x01") || b.starts_with(b"DZOB\x02"), "Unsupported object file");
        let with_uids = b[4] == 2;
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
        // Older files have none, so their objects get new identities.
        if with_uids {
            ensure!(b.len() >= p + n * 8, "Truncated object file");
            for i in 0..n {
                let uid = u64::from_le_bytes(b[p + i * 8..p + i * 8 + 8].try_into().unwrap());
                if uid != 0 {
                    s.uids[i] = uid;
                }
            }
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

/// How copies of objects are laid in a row, as walls and fences are laid end to end: along
/// the long side of the first object, forwards along its own axis or back against it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    /// Where the objects end on that side, on the line through the first object: the joint the
    /// row starts from and turns about.
    pub joint: [f32; 2],
    /// The way the row goes, of length 1.
    pub dir: [f32; 2],
    /// How far the objects reach together along that side, so copies touch each other.
    pub step: f32,
}

/// The row of copies of `items`, each an object with its model's box, going `ahead` along the
/// long side of the first object or back.
pub fn row_of(items: &[(Obj, crate::model_info::ModelInfo)], ahead: bool) -> Option<Row> {
    let (lead, info) = items.first()?;
    let along_x = info.max[0] - info.min[0] >= info.max[2] - info.min[2];
    let m = rotation(lead.yaw, lead.pitch, lead.roll, 1.0);
    let k = if along_x { 0 } else { 2 };
    let (ax, az) = (m[k][0], m[k][2]);
    let len = ax.hypot(az);
    if len < 1e-4 {
        return None;
    }
    let (ax, az) = if ahead { (ax / len, az / len) } else { (-ax / len, -az / len) };
    // How far the boxes of all the objects reach along the side.
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for (o, i) in items {
        let r = rotation(o.yaw, o.pitch, o.roll, o.scale);
        for c in 0..8 {
            let p = [if c & 1 == 0 { i.min[0] } else { i.max[0] }, if c & 2 == 0 { i.min[1] } else { i.max[1] }, if c & 4 == 0 { i.min[2] } else { i.max[2] }];
            let x = o.x + p[0] * r[0][0] + p[1] * r[1][0] + p[2] * r[2][0];
            let z = o.z + p[0] * r[0][2] + p[1] * r[1][2] + p[2] * r[2][2];
            let d = x * ax + z * az;
            (lo, hi) = (lo.min(d), hi.max(d));
        }
    }
    // A model with no box still steps on by a metre.
    let step = if hi > lo { (hi - lo).max(0.1) } else { 1.0 };
    let along = hi - (lead.x * ax + lead.z * az);
    Some(Row { joint: [lead.x + ax * along, lead.z + az * along], dir: [ax, az], step })
}

/// `count` copies of `items` laid in `row`, the whole row turned by `turn` degrees (clockwise
/// seen from above, as yaw) about its joint: the first `items.len()` objects are the first
/// copy, and so on.
pub fn row_copies(items: &[(Obj, crate::model_info::ModelInfo)], row: &Row, count: usize, turn: f32) -> Vec<Obj> {
    let (sin, cos) = turn.to_radians().sin_cos();
    let mut out = Vec::with_capacity(items.len() * count);
    for k in 1..=count {
        let shift = row.step * k as f32;
        for (o, _) in items {
            let (rx, rz) = (o.x + row.dir[0] * shift - row.joint[0], o.z + row.dir[1] * shift - row.joint[1]);
            out.push(Obj { x: row.joint[0] + rx * cos + rz * sin, z: row.joint[1] - rx * sin + rz * cos, yaw: o.yaw + turn, ..*o });
        }
    }
    out
}

/// The number of copies and the turn of a row whose end is dragged to `to`: as many copies as
/// fit up to it, turned towards it. Turns under `dead` degrees are none, so a row runs
/// straight unless it is pulled clearly aside; `snap` turns by whole steps of degrees.
pub fn row_to(row: &Row, to: [f32; 2], snap: Option<f32>, dead: f32) -> (usize, f32) {
    let (dx, dz) = (to[0] - row.joint[0], to[1] - row.joint[1]);
    let len = dx.hypot(dz);
    if len < 1e-3 {
        return (0, 0.0);
    }
    let mut turn = (dx.atan2(dz) - row.dir[0].atan2(row.dir[1])).to_degrees();
    turn = (turn + 180.0).rem_euclid(360.0) - 180.0;
    if let Some(s) = snap.filter(|s| *s > 0.0) {
        turn = (turn / s).round() * s;
    } else if turn.abs() < dead {
        turn = 0.0;
    }
    ((len / row.step).round() as usize, turn)
}

/// Where the objects join a neighbour in a row: of the two ends of the objects (0 ahead, 1
/// behind, see [`row_of`]) the one nearest to an end of one of `others`, each an object with
/// its model's box, if they are under `reach` metres apart. Gives that end of the objects and
/// the end of the neighbour it joins.
pub fn link_to(items: &[(Obj, crate::model_info::ModelInfo)], others: &[(Obj, crate::model_info::ModelInfo)], reach: f32) -> Option<(usize, [f32; 2], [f32; 2])> {
    let ours = [row_of(items, true)?.joint, row_of(items, false)?.joint];
    let mut best: Option<(f32, usize, [f32; 2], [f32; 2])> = None;
    for other in others {
        let one = std::slice::from_ref(other);
        for theirs in [row_of(one, true), row_of(one, false)].into_iter().flatten().map(|r| r.joint) {
            for (k, mine) in ours.iter().enumerate() {
                let d = (mine[0] - theirs[0]).hypot(mine[1] - theirs[1]);
                if d < reach && best.is_none_or(|b| d < b.0) {
                    best = Some((d, k, *mine, theirs));
                }
            }
        }
    }
    best.map(|(_, k, mine, theirs)| (k, mine, theirs))
}

/// The objects moved by `shift` and then turned by `turn` degrees (clockwise seen from above,
/// as yaw) about `pivot`.
pub fn shift_and_turn(objects: &[Obj], shift: [f32; 2], pivot: [f32; 2], turn: f32) -> Vec<Obj> {
    let (sin, cos) = turn.to_radians().sin_cos();
    objects.iter().map(|o| {
        let (rx, rz) = (o.x + shift[0] - pivot[0], o.z + shift[1] - pivot[1]);
        Obj { x: pivot[0] + rx * cos + rz * sin, z: pivot[1] - rx * sin + rz * cos, yaw: o.yaw + turn, ..*o }
    }).collect()
}

/// One copy of the objects laid right next to them, ahead or behind; see [`row_of`].
pub fn copies_along(items: &[(Obj, crate::model_info::ModelInfo)], ahead: bool) -> Vec<Obj> {
    row_of(items, ahead).map(|r| row_copies(items, &r, 1, 0.0)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_along_touch_the_originals() {
        use crate::model_info::ModelInfo;
        // A wall 6 m long along x, 0.4 m thick, turned 90° so its length runs along the world's x... or z.
        let wall = ModelInfo { min: [-3.0, 0.0, -0.2], max: [3.0, 2.0, 0.2], known: true, ..Default::default() };
        let o = Obj { x: 100.0, z: 200.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        let ahead = copies_along(&[(o, wall)], true);
        let back = copies_along(&[(o, wall)], false);
        // Unturned, the model's x is the world's x.
        assert!((ahead[0].x - 106.0).abs() < 1e-3 && (ahead[0].z - 200.0).abs() < 1e-3, "{:?}", ahead[0]);
        assert!((back[0].x - 94.0).abs() < 1e-3, "{:?}", back[0]);
        // Scaled up, the step grows with it; turned, it follows the wall.
        let big = Obj { scale: 1.5, yaw: 90.0, ..o };
        let c = copies_along(&[(big, wall)], true)[0];
        assert!(((c.x - 100.0).hypot(c.z - 200.0) - 9.0).abs() < 1e-3, "{c:?}");
        assert!((c.yaw - 90.0).abs() < 1e-6 && c.scale == 1.5);
        // Two walls in a row step on by both of them.
        let next = Obj { x: 106.0, ..o };
        let two = copies_along(&[(o, wall), (next, wall)], true);
        assert!((two[0].x - 112.0).abs() < 1e-3 && (two[1].x - 118.0).abs() < 1e-3, "{two:?}");
    }

    #[test]
    fn a_wall_finds_where_it_joins_its_neighbour_and_turns_about_it() {
        use crate::model_info::ModelInfo;
        let wall = ModelInfo { min: [-3.0, 0.0, -0.2], max: [3.0, 2.0, 0.2], known: true, ..Default::default() };
        let first = Obj { x: 100.0, z: 200.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        // The next wall was turned about its own middle, so its back end left the first wall's end.
        let next = Obj { x: 106.0, yaw: 20.0, ..first };
        let (end, mine, theirs) = link_to(&[(next, wall)], &[(first, wall)], 3.0).unwrap();
        assert_eq!(end, 1);
        assert!((theirs[0] - 103.0).abs() < 1e-3 && (theirs[1] - 200.0).abs() < 1e-3, "{theirs:?}");
        // Moved back so its end meets the joint, then turned about it, it still meets it.
        let shift = [theirs[0] - mine[0], theirs[1] - mine[1]];
        let moved = shift_and_turn(&[next], shift, theirs, 15.0)[0];
        let (_, back, _) = link_to(&[(moved, wall)], &[(first, wall)], 3.0).unwrap();
        assert!((back[0] - 103.0).abs() < 1e-3 && (back[1] - 200.0).abs() < 1e-3, "{back:?}");
        assert!((moved.yaw - 35.0).abs() < 1e-4);
        // Nothing near: no joint.
        assert!(link_to(&[(Obj { x: 200.0, ..first }, wall)], &[(first, wall)], 3.0).is_none());
    }

    #[test]
    fn a_row_turns_about_its_joint() {
        use crate::model_info::ModelInfo;
        let wall = ModelInfo { min: [-3.0, 0.0, -0.2], max: [3.0, 2.0, 0.2], known: true, ..Default::default() };
        let o = Obj { x: 100.0, z: 200.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        let row = row_of(&[(o, wall)], true).unwrap();
        assert!((row.joint[0] - 103.0).abs() < 1e-3 && (row.joint[1] - 200.0).abs() < 1e-3 && (row.step - 6.0).abs() < 1e-3, "{row:?}");
        // Dragged 20 m on along the wall: three copies, no turn.
        assert_eq!(row_to(&row, [123.5, 200.4], None, 3.0), (3, 0.0));
        // Dragged off at a right angle: the row turns 90° about the joint, so the first copy
        // starts there and runs the new way.
        let (n, turn) = row_to(&row, [103.0, 212.0], None, 3.0);
        assert_eq!(n, 2);
        assert!((turn.abs() - 90.0).abs() < 1e-3, "{turn}");
        let c = row_copies(&[(o, wall)], &row, n, turn);
        assert!((c[0].x - 103.0).abs() < 1e-3 && (c[0].z - 203.0).abs() < 1e-3, "{:?}", c[0]);
        assert!((c[1].z - 209.0).abs() < 1e-3 && (c[1].yaw - turn).abs() < 1e-3, "{:?}", c[1]);
        // With a step of 45° a turn of 50° goes to 45°.
        let (_, t) = row_to(&row, [103.0 + 10.0 * 50f32.to_radians().cos(), 200.0 + 10.0 * 50f32.to_radians().sin()], Some(45.0), 3.0);
        assert!((t.abs() - 45.0).abs() < 1e-3, "{t}");
        // Back the other way the joint is at the other end.
        let back = row_of(&[(o, wall)], false).unwrap();
        assert!((back.joint[0] - 97.0).abs() < 1e-3 && (back.dir[0] + 1.0).abs() < 1e-3);
    }
    #[test]
    fn a_drive_letter_is_dropped_from_a_model_path() {
        assert_eq!(ObjectStore::strip_drive("p:\\dz\\a.p3d"), "dz\\a.p3d");
        assert_eq!(ObjectStore::strip_drive("D:/dz/a.p3d"), "dz/a.p3d");
        assert_eq!(ObjectStore::strip_drive("dz\\a.p3d"), "dz\\a.p3d");
        assert_eq!(ObjectStore::strip_drive("a:b"), "a:b");
    }
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
    fn store_keeps_ids_and_grid_in_sync() {
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
        assert_ne!(s.insert(obj(1.0, 1.0)), a, "ids are not reused");
    }
    #[test]
    fn identities_survive_undo_and_saving() {
        let mut s = ObjectStore::default();
        let a = s.insert(obj(10.0, 10.0));
        let b = s.insert(obj(20.0, 10.0));
        let (ua, ub) = (s.uid(a), s.uid(b));
        assert!(ua != 0 && ub != 0 && ua != ub);
        let o = s.remove(a).unwrap();
        assert_eq!(s.uid(a), 0);
        s.insert(obj(30.0, 10.0));
        s.insert_at(a, o);
        assert_eq!(s.uid(a), ua, "undo brings back the same identity");
        let t = ObjectStore::from_bytes(&s.to_bytes()).unwrap();
        let mut uids: Vec<u64> = t.iter().map(|(id, _)| t.uid(id)).collect();
        uids.sort();
        let mut want: Vec<u64> = s.iter().map(|(id, _)| s.uid(id)).collect();
        want.sort();
        assert_eq!(uids, want);
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
