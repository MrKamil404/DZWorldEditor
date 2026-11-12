//! Mask modifiers: layers of surfaces laid over the surface mask.
//!
//! They work like the satellite modifiers. A modifier owns the pixels it sets, in tiles, and
//! the mask as it was painted is kept under every tile a modifier touches. The mask shown and
//! exported is the painted one with the enabled modifiers over it in order, so hiding or
//! deleting a modifier brings back what was painted there.
use crate::terrain::Rect;
use crate::world::World;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// Side of a tile in pixels.
pub const TILE: usize = 128;
/// A pixel of a modifier that leaves the mask as it is.
pub const NONE: u8 = 255;
pub type Key = (u16, u16);
pub type Tile = Arc<Vec<u8>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MaskKind {
    /// A surface stamped under objects.
    UnderObjects,
    /// The ground under the trees of a forest generator.
    Forest,
    /// The result of a surface painting generator.
    Paint,
}

#[derive(Clone, Debug)]
pub struct MaskModifier {
    pub id: u32,
    pub name: String,
    pub kind: MaskKind,
    pub enabled: bool,
    pub tiles: HashMap<Key, Tile>,
}

#[derive(Clone, Default)]
pub struct MaskLayers {
    pub modifiers: Vec<MaskModifier>,
    base: HashMap<Key, Tile>,
    next_id: u32,
}

impl MaskLayers {
    pub fn is_empty(&self) -> bool {
        self.modifiers.is_empty()
    }
    pub fn position(&self, id: u32) -> Option<usize> {
        self.modifiers.iter().position(|m| m.id == id)
    }
    pub fn keys_of(list: &[MaskModifier]) -> BTreeSet<Key> {
        list.iter().flat_map(|m| m.tiles.keys().copied()).collect()
    }
    /// Adds an empty modifier and returns its id.
    pub fn add(&mut self, kind: MaskKind, name: String) -> u32 {
        self.next_id = self.next_id.max(self.modifiers.iter().map(|m| m.id + 1).max().unwrap_or(1)).max(1);
        let id = self.next_id;
        self.next_id += 1;
        self.modifiers.push(MaskModifier { id, name, kind, enabled: true, tiles: HashMap::new() });
        id
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = b"DZMM1".to_vec();
        b.extend(self.next_id.to_le_bytes());
        let tiles = |b: &mut Vec<u8>, map: &HashMap<Key, Tile>| {
            b.extend((map.len() as u32).to_le_bytes());
            let mut keys: Vec<&Key> = map.keys().collect();
            keys.sort();
            for k in keys {
                b.extend(k.0.to_le_bytes());
                b.extend(k.1.to_le_bytes());
                b.extend(map[k].iter());
            }
        };
        tiles(&mut b, &self.base);
        b.extend((self.modifiers.len() as u32).to_le_bytes());
        for m in &self.modifiers {
            b.extend(m.id.to_le_bytes());
            b.push(match m.kind {
                MaskKind::UnderObjects => 0,
                MaskKind::Forest => 1,
                MaskKind::Paint => 2,
            });
            b.push(m.enabled as u8);
            b.extend((m.name.len() as u32).to_le_bytes());
            b.extend(m.name.as_bytes());
            tiles(&mut b, &m.tiles);
        }
        b
    }

    pub fn from_bytes(b: &[u8]) -> anyhow::Result<Self> {
        use anyhow::Context;
        anyhow::ensure!(b.starts_with(b"DZMM1"), "Not a mask modifier file");
        let mut p = 5;
        let mut take = |n: usize| -> anyhow::Result<&[u8]> {
            let s = b.get(p..p + n).context("Mask modifier file is cut short")?;
            p += n;
            Ok(s)
        };
        fn int(s: &[u8]) -> u32 {
            u32::from_le_bytes([s[0], s[1], s[2], s[3]])
        }
        fn tiles<'a>(take: &mut dyn FnMut(usize) -> anyhow::Result<&'a [u8]>) -> anyhow::Result<HashMap<Key, Tile>> {
            let n = int(take(4)?) as usize;
            let mut map = HashMap::with_capacity(n.min(1 << 16));
            for _ in 0..n {
                let k = take(4)?;
                let key = (u16::from_le_bytes([k[0], k[1]]), u16::from_le_bytes([k[2], k[3]]));
                map.insert(key, Arc::new(take(TILE * TILE)?.to_vec()));
            }
            Ok(map)
        }
        let next_id = int(take(4)?);
        let base = tiles(&mut take)?;
        let count = int(take(4)?) as usize;
        let mut modifiers = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let id = int(take(4)?);
            let head = take(2)?;
            let kind = match head[0] {
                0 => MaskKind::UnderObjects,
                1 => MaskKind::Forest,
                _ => MaskKind::Paint,
            };
            let enabled = head[1] != 0;
            let len = int(take(4)?) as usize;
            let name = String::from_utf8_lossy(take(len)?).into_owned();
            modifiers.push(MaskModifier { id, name, kind, enabled, tiles: tiles(&mut take)? });
        }
        Ok(Self { modifiers, base, next_id })
    }
}

fn tile_rect(key: Key, w: usize, h: usize) -> Option<Rect> {
    let (x0, y0) = (key.0 as usize * TILE, key.1 as usize * TILE);
    (x0 < w && y0 < h).then(|| Rect { x0, y0, x1: (x0 + TILE).min(w) - 1, y1: (y0 + TILE).min(h) - 1 })
}

fn union(a: Option<Rect>, b: Rect) -> Option<Rect> {
    Some(a.map_or(b, |a| a.union(b)))
}

impl World {
    /// Keeps the painted mask of a tile before a modifier first writes to it.
    fn mask_keep(&mut self, key: Key) {
        if self.mask_mods.base.contains_key(&key) {
            return;
        }
        let Some(r) = tile_rect(key, self.mask.width, self.mask.height) else { return };
        let mut t = vec![0u8; TILE * TILE];
        for y in r.y0..=r.y1 {
            let n = r.x1 - r.x0 + 1;
            t[(y - r.y0) * TILE..(y - r.y0) * TILE + n].copy_from_slice(&self.mask.data[y * self.mask.width + r.x0..y * self.mask.width + r.x0 + n]);
        }
        self.mask_mods.base.insert(key, Arc::new(t));
    }

    /// Lays the enabled modifiers over the painted mask of the given tiles again.
    pub fn mask_recompose(&mut self, keys: impl IntoIterator<Item = Key>) -> Option<Rect> {
        let mut touched = None;
        for key in keys {
            self.mask_keep(key);
            let Some(base) = self.mask_mods.base.get(&key).cloned() else { continue };
            let Some(r) = tile_rect(key, self.mask.width, self.mask.height) else { continue };
            let layers: Vec<&Tile> = self.mask_mods.modifiers.iter().filter(|m| m.enabled).filter_map(|m| m.tiles.get(&key)).collect();
            let width = self.mask.width;
            for y in r.y0..=r.y1 {
                for x in r.x0..=r.x1 {
                    let t = (y - r.y0) * TILE + (x - r.x0);
                    let mut v = base[t];
                    for l in &layers {
                        if l[t] != NONE {
                            v = l[t];
                        }
                    }
                    self.mask.data[y * width + x] = v;
                }
            }
            touched = union(touched, r);
        }
        touched
    }

    /// After the mask was painted on directly: what was painted becomes the mask under the
    /// modifiers, and where a modifier lies over it the modifier still shows.
    pub fn mask_painted(&mut self, area: Rect) -> Option<Rect> {
        if self.mask_mods.base.is_empty() {
            return None;
        }
        let (kx0, ky0, kx1, ky1) = (area.x0 / TILE, area.y0 / TILE, area.x1 / TILE, area.y1 / TILE);
        let keys: Vec<Key> = self.mask_mods.base.keys().copied().filter(|k| (kx0..=kx1).contains(&(k.0 as usize)) && (ky0..=ky1).contains(&(k.1 as usize))).collect();
        for key in &keys {
            let Some(r) = tile_rect(*key, self.mask.width, self.mask.height) else { continue };
            let layers: Vec<Tile> = self.mask_mods.modifiers.iter().filter(|m| m.enabled).filter_map(|m| m.tiles.get(key).cloned()).collect();
            let width = self.mask.width;
            let base = Arc::make_mut(self.mask_mods.base.get_mut(key).unwrap());
            for y in r.y0.max(area.y0)..=r.y1.min(area.y1) {
                for x in r.x0.max(area.x0)..=r.x1.min(area.x1) {
                    let t = (y - r.y0) * TILE + (x - r.x0);
                    if layers.iter().all(|l| l[t] == NONE) {
                        base[t] = self.mask.data[y * width + x];
                    }
                }
            }
        }
        self.mask_recompose(keys)
    }

    /// Replaces the list of modifiers and returns the pixels that changed.
    pub fn mask_set_modifiers(&mut self, list: &mut Vec<MaskModifier>) -> Option<Rect> {
        let mut keys = MaskLayers::keys_of(&self.mask_mods.modifiers);
        keys.extend(MaskLayers::keys_of(list));
        std::mem::swap(list, &mut self.mask_mods.modifiers);
        self.mask_recompose(keys)
    }

    pub fn mask_remove(&mut self, id: u32) -> Option<Rect> {
        let at = self.mask_mods.position(id)?;
        let gone = self.mask_mods.modifiers.remove(at);
        self.mask_recompose(gone.tiles.keys().copied().collect::<Vec<_>>())
    }

    pub fn mask_set_enabled(&mut self, id: u32, enabled: bool) -> Option<Rect> {
        let at = self.mask_mods.position(id)?;
        self.mask_mods.modifiers[at].enabled = enabled;
        let keys: Vec<Key> = self.mask_mods.modifiers[at].tiles.keys().copied().collect();
        self.mask_recompose(keys)
    }

    /// Gives a modifier new tiles in place of the ones it had.
    pub fn mask_replace_tiles(&mut self, id: u32, tiles: HashMap<Key, Tile>) -> Option<Rect> {
        let at = self.mask_mods.position(id)?;
        let mut keys: BTreeSet<Key> = self.mask_mods.modifiers[at].tiles.keys().copied().collect();
        keys.extend(tiles.keys().copied());
        for k in &keys {
            self.mask_keep(*k);
        }
        self.mask_mods.modifiers[at].tiles = tiles;
        self.mask_recompose(keys)
    }

    /// True where the modifier `id` sets a pixel of the mask.
    pub fn mask_covered(&self, id: u32, x: usize, y: usize) -> bool {
        self.mask_mods.position(id).and_then(|at| self.mask_mods.modifiers[at].tiles.get(&((x / TILE) as u16, (y / TILE) as u16))).is_some_and(|t| t[(y % TILE) * TILE + x % TILE] != NONE)
    }
}

/// Shape of the patch stamped under an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum StampShape {
    Circle,
    Square,
    Diamond,
    /// A circle with an uneven edge, which reads best under trees.
    #[default]
    Blob,
}
impl StampShape {
    pub const ALL: [StampShape; 4] = [StampShape::Blob, StampShape::Circle, StampShape::Square, StampShape::Diamond];
    pub fn name(self) -> &'static str {
        match self {
            StampShape::Circle => "Circle",
            StampShape::Square => "Square",
            StampShape::Diamond => "Diamond",
            StampShape::Blob => "Blob",
        }
    }
}

/// How patches are stamped under objects: the settings DayZ Forests has for its tree layer.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StampSettings {
    /// Radius of a patch in metres.
    pub size_m: f32,
    pub shape: StampShape,
    /// Share by which the size of each patch varies, 0 to 1.
    pub randomize_size: f32,
    /// Turns squares and diamonds by a random angle.
    pub randomize_rotation: bool,
    /// Scales the patch with the object.
    pub follow_scale: bool,
    pub seed: u64,
}
impl Default for StampSettings {
    fn default() -> Self {
        Self { size_m: 3.2, shape: StampShape::Blob, randomize_size: 0.35, randomize_rotation: false, follow_scale: true, seed: 1 }
    }
}

/// One patch to stamp.
#[derive(Clone, Copy, Debug)]
pub struct Stamp {
    pub x: f32,
    pub z: f32,
    pub surface: u8,
    /// Scale of the object.
    pub scale: f32,
    /// Half width, half depth and yaw of the object's footprint, to fill that instead of a patch.
    pub footprint: Option<[f32; 3]>,
}

fn hash(x: f32, z: f32, seed: u64) -> f32 {
    let mut h = (x.to_bits() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (z.to_bits() as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h & 0xFF_FFFF) as f32 / 16_777_216.0
}

/// Draws the patches into tiles of a `width` by `height` mask covering a map `size` metres across.
pub fn stamp_tiles(stamps: &[Stamp], s: &StampSettings, width: usize, height: usize, size: f32) -> HashMap<Key, Tile> {
    let mut tiles: HashMap<Key, Vec<u8>> = HashMap::new();
    let (sx, sy) = (width as f32 / size, height as f32 / size);
    let mut set = |x: i64, y: i64, v: u8| {
        if x < 0 || y < 0 || x >= width as i64 || y >= height as i64 {
            return;
        }
        let (x, y) = (x as usize, y as usize);
        tiles.entry(((x / TILE) as u16, (y / TILE) as u16)).or_insert_with(|| vec![NONE; TILE * TILE])[(y % TILE) * TILE + x % TILE] = v;
    };
    for o in stamps {
        let (cx, cy) = (o.x * sx, (size - o.z) * sy);
        if let Some([hx, hz, yaw]) = o.footprint {
            // The footprint of the object, grown by the size setting.
            let (hx, hz) = ((hx + s.size_m) * sx, (hz + s.size_m) * sy);
            let (sin, cos) = yaw.to_radians().sin_cos();
            let reach = (hx * hx + hz * hz).sqrt().ceil() as i64;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    // Image rows run south, so the turn is mirrored.
                    let (lx, lz) = (dx as f32 * cos + dy as f32 * sin, -(dx as f32) * sin + dy as f32 * cos);
                    if lx.abs() <= hx && lz.abs() <= hz {
                        set(cx as i64 + dx, cy as i64 + dy, o.surface);
                    }
                }
            }
            continue;
        }
        let vary = if s.randomize_size > 0.0 { 1.0 + (hash(o.x, o.z, s.seed) - 0.5) * 2.0 * s.randomize_size } else { 1.0 };
        let r = (s.size_m * sx * vary * if s.follow_scale { o.scale.max(0.1) } else { 1.0 }).max(0.5);
        let angle = if s.randomize_rotation {
            hash(o.x + 1000.0, o.z + 1000.0, s.seed) * std::f32::consts::TAU
        } else if s.shape == StampShape::Diamond {
            std::f32::consts::FRAC_PI_4
        } else {
            0.0
        };
        let (sin, cos) = angle.sin_cos();
        let (p1, p2) = (hash(o.x + 11.0, o.z + 17.0, s.seed) * std::f32::consts::TAU, hash(o.x + 91.0, o.z + 33.0, s.seed) * std::f32::consts::TAU);
        let reach = (r * 1.4).ceil() as i64;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (rx, ry) = (dx as f32 * cos - dy as f32 * sin, dx as f32 * sin + dy as f32 * cos);
                let inside = match s.shape {
                    StampShape::Circle => rx * rx + ry * ry <= r * r,
                    StampShape::Square => rx.abs() <= r && ry.abs() <= r,
                    StampShape::Diamond => rx.abs() + ry.abs() <= r,
                    StampShape::Blob => {
                        let a = ry.atan2(rx);
                        let wobble = 0.30 * (a * 3.0 + p1).sin() + 0.18 * (a * 5.0 + p2).cos() + 0.12 * (a * 7.0 - p1 * 0.5).sin();
                        (rx * rx + ry * ry).sqrt() <= r * (1.0 + wobble).clamp(0.65, 1.35)
                    }
                };
                if inside {
                    set(cx as i64 + dx, cy as i64 + dy, o.surface);
                }
            }
        }
    }
    tiles.into_iter().map(|(k, t)| (k, Arc::new(t))).collect()
}

/// Leaves out the pixels of a modifier that would bring a texture tile of the terrain over
/// the number of surfaces the project allows, as the paint brush does: there the mask keeps
/// what it has. The tiles are checked against the mask as it is now, other modifiers
/// included, in the order of the pixels. Returns how many pixels were left out.
pub fn keep_tile_limit(w: &World, tiles: &mut HashMap<Key, Tile>) -> usize {
    let mut guard = crate::mask_limit::TileGuard::new(w);
    let (width, height) = (w.mask.width, w.mask.height);
    let mut keys: Vec<Key> = tiles.keys().copied().collect();
    keys.sort_by_key(|k| (k.1, k.0));
    let mut dropped = 0;
    for key in keys {
        let Some(tile) = tiles.get_mut(&key) else { continue };
        let t = Arc::make_mut(tile);
        for (i, v) in t.iter_mut().enumerate() {
            if *v == NONE {
                continue;
            }
            let (x, y) = (key.0 as usize * TILE + i % TILE, key.1 as usize * TILE + i / TILE);
            if x >= width || y >= height {
                continue;
            }
            // A pixel that keeps the surface already there changes nothing on its tile.
            if w.mask.data[y * width + x] != *v && !guard.allow(&w.mask, x, y, *v) {
                *v = NONE;
                dropped += 1;
            }
        }
    }
    tiles.retain(|_, t| t.iter().any(|v| *v != NONE));
    dropped
}

/// The pixels where `after` differs from `before`, or that `keep` names, as tiles of a modifier.
pub fn diff_tiles(before: &[u8], after: &[u8], width: usize, height: usize, keep: impl Fn(usize, usize) -> bool) -> HashMap<Key, Tile> {
    let mut tiles: HashMap<Key, Vec<u8>> = HashMap::new();
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            if after[i] != before[i] || keep(x, y) {
                tiles.entry(((x / TILE) as u16, (y / TILE) as u16)).or_insert_with(|| vec![NONE; TILE * TILE])[(y % TILE) * TILE + x % TILE] = after[i];
            }
        }
    }
    tiles.into_iter().map(|(k, t)| (k, Arc::new(t))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::WorldMeta;

    fn world() -> World {
        let mut w = World::new(WorldMeta::new("t", 64, 4.0), 10.0);
        w.mask = crate::terrain::MaskMap { width: 256, height: 256, data: vec![1; 256 * 256] };
        w
    }
    fn at(w: &World, x: usize, y: usize) -> u8 {
        w.mask.data[y * 256 + x]
    }

    #[test]
    fn a_stamp_sets_the_surface_and_its_removal_restores_the_mask() {
        let mut w = world();
        // The map is 256 m across, so a metre is a pixel; north is up.
        let tiles = stamp_tiles(&[Stamp { x: 130.0, z: 196.0, surface: 5, scale: 1.0, footprint: None }], &StampSettings { size_m: 6.0, shape: StampShape::Circle, randomize_size: 0.0, ..Default::default() }, 256, 256, 256.0);
        assert!(tiles.len() >= 2, "the patch spans two tiles");
        let id = w.mask_mods.add(MaskKind::UnderObjects, "m".into());
        let r = w.mask_replace_tiles(id, tiles).unwrap();
        assert!(r.x0 <= 130 && r.x1 >= 130);
        assert_eq!(at(&w, 130, 60), 5);
        assert_eq!(at(&w, 130, 70), 1);
        w.mask_set_enabled(id, false);
        assert_eq!(at(&w, 130, 60), 1);
        w.mask_set_enabled(id, true);
        w.mask_remove(id);
        assert_eq!(at(&w, 130, 60), 1);
    }

    #[test]
    fn painting_beside_a_modifier_stays_and_the_modifier_keeps_its_place() {
        let mut w = world();
        let tiles = stamp_tiles(&[Stamp { x: 40.0, z: 216.0, surface: 5, scale: 1.0, footprint: None }], &StampSettings { size_m: 4.0, shape: StampShape::Square, randomize_size: 0.0, ..Default::default() }, 256, 256, 256.0);
        let id = w.mask_mods.add(MaskKind::UnderObjects, "m".into());
        w.mask_replace_tiles(id, tiles);
        // A stroke of the paint tool over the patch and beside it.
        for x in 36..60 {
            w.mask.data[40 * 256 + x] = 7;
        }
        w.mask_painted(Rect { x0: 36, y0: 40, x1: 59, y1: 40 });
        assert_eq!(at(&w, 40, 40), 5, "the modifier stays on top");
        assert_eq!(at(&w, 55, 40), 7);
        // Where the modifier lies, the mask under it stays as it was before the modifier.
        w.mask_remove(id);
        assert_eq!(at(&w, 40, 40), 1);
        assert_eq!(at(&w, 55, 40), 7, "what was painted beside it stays");
    }

    #[test]
    fn a_footprint_is_filled_and_layers_survive_saving() {
        let mut w = world();
        let tiles = stamp_tiles(&[Stamp { x: 100.0, z: 156.0, surface: 3, scale: 1.0, footprint: Some([10.0, 4.0, 0.0]) }], &StampSettings { size_m: 1.0, ..Default::default() }, 256, 256, 256.0);
        let id = w.mask_mods.add(MaskKind::Forest, "f".into());
        w.mask_replace_tiles(id, tiles);
        assert_eq!(at(&w, 109, 100), 3);
        assert_eq!(at(&w, 100, 106), 1, "beyond the short side");
        let back = MaskLayers::from_bytes(&w.mask_mods.to_bytes()).unwrap();
        assert_eq!((back.modifiers[0].kind, back.modifiers[0].name.as_str(), back.modifiers[0].tiles.len()), (MaskKind::Forest, "f", w.mask_mods.modifiers[0].tiles.len()));
    }

    #[test]
    fn a_stamp_keeps_the_surfaces_a_texture_tile_may_hold() {
        let mut w = world();
        w.meta.max_tile_surfaces = 2;
        // One surface already lies on the whole mask; a second may come, a third may not.
        w.mask.data.iter_mut().for_each(|v| *v = 0);
        let stamp = |surface, x: f32| Stamp { x, z: 128.0, surface, scale: 1.0, footprint: None };
        let s = StampSettings { size_m: 3.0, shape: StampShape::Square, randomize_size: 0.0, ..Default::default() };
        let mut tiles = stamp_tiles(&[stamp(1, 40.0), stamp(2, 50.0)], &s, 256, 256, 256.0);
        let dropped = keep_tile_limit(&w, &mut tiles);
        assert!(dropped > 0);
        let id = w.mask_mods.add(MaskKind::UnderObjects, "m".into());
        w.mask_replace_tiles(id, tiles);
        assert_eq!(at(&w, 40, 128), 1, "the second surface is let in");
        assert_eq!(at(&w, 50, 128), 0, "the third keeps what was there");
        assert!(crate::mask_limit::scan(&w).is_empty());
    }

    #[test]
    fn a_generator_result_becomes_the_pixels_it_changed() {
        let before = vec![1u8; 256 * 256];
        let mut after = before.clone();
        after[10 * 256 + 10] = 4;
        let tiles = diff_tiles(&before, &after, 256, 256, |x, y| (x, y) == (200, 200));
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[&(0, 0)][10 * TILE + 10], 4);
        assert_eq!(tiles[&(1, 1)][(200 % TILE) * TILE + 200 % TILE], 1);
        assert_eq!(tiles[&(0, 0)][0], NONE);
    }
}
