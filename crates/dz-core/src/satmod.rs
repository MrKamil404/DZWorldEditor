//! Satellite modifiers: layers of pixels laid over the satellite image.
//!
//! The image itself is never edited in place. Each modifier owns the pixels it adds, in tiles,
//! and the original pixels of every tile a modifier touches are kept beside them. The image
//! shown and exported is the original with the enabled modifiers laid over it in order, so
//! removing or hiding a modifier brings back exactly what was under it.
use crate::model_info::ModelDb;
use crate::objects::Obj;
use crate::terrain::RgbImage;
use crate::world::World;
use crate::Job;
use forest_core::model3d::{self, ModelMesh, ShadingKind};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

/// Side of a tile in pixels.
pub const TILE: usize = 128;
/// Column and row of a tile.
pub type Key = (u16, u16);
/// RGBA pixels of a modifier's tile, or RGB pixels of an original one.
pub type Tile = Arc<Vec<u8>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SatKind {
    /// Top views of objects.
    Bake,
    /// Strokes of the clone brush.
    Clone,
    /// Strokes of the adjust brush: the image under them lighter, darker or in other colours.
    Adjust,
    /// Strokes and fills of the colour brush.
    Paint,
    /// A colour correction worked out from what is under it whenever the image is composed,
    /// so its settings can be changed at any time. Its tiles hold a mask, one byte a pixel.
    Correct,
    /// A satellite made from the surface mask; see `satgen`.
    Generated,
}

impl SatKind {
    /// Made by strokes and fills of a brush, so several can be merged into one.
    pub fn is_brush(self) -> bool {
        matches!(self, SatKind::Clone | SatKind::Adjust | SatKind::Paint)
    }
}

/// `upper` laid over `lower` into one tile at full opacity, each first at its own opacity:
/// over any picture the result looks as the two did one over the other.
fn merge_tiles(lower: Option<(&Tile, u8)>, upper: Option<(&Tile, u8)>) -> Tile {
    let mut out = vec![0u8; TILE * TILE * 4];
    for p in 0..TILE * TILE {
        let get = |t: Option<(&Tile, u8)>| t.map_or(([0f32; 3], 0f32), |(t, o)| ([t[p * 4] as f32, t[p * 4 + 1] as f32, t[p * 4 + 2] as f32], t[p * 4 + 3] as f32 / 255.0 * o as f32 / 255.0));
        let ((a_rgb, a), (b_rgb, b)) = (get(lower), get(upper));
        let cover = a + b - a * b;
        if cover <= 0.0 {
            continue;
        }
        for k in 0..3 {
            out[p * 4 + k] = ((a_rgb[k] * a * (1.0 - b) + b_rgb[k] * b) / cover).round().clamp(0.0, 255.0) as u8;
        }
        out[p * 4 + 3] = (cover * 255.0).round() as u8;
    }
    Arc::new(out)
}

/// The settings of a correction modifier.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Correction {
    pub adjust: Adjust,
    /// Over the whole image; otherwise where its mask tiles say.
    pub whole: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SatModifier {
    pub id: u32,
    pub name: String,
    pub kind: SatKind,
    pub enabled: bool,
    /// How much of the modifier shows over what is under it, 0 to 255. Less blends it in.
    pub opacity: u8,
    pub tiles: HashMap<Key, Tile>,
    /// The settings of a correction.
    pub correction: Option<Correction>,
}

/// One layer of a tile as it is composed.
pub enum Layer<'a> {
    /// RGBA pixels laid over what is under them, at an opacity 0 to 255.
    Paint(&'a Tile, u32),
    /// What is under it adjusted, as far as its mask (all of it without one) and opacity say.
    Correct(&'a Adjust, Option<&'a Tile>, u32),
}

/// The modifiers of a world and the original pixels under them.
#[derive(Clone, Default)]
pub struct SatLayers {
    pub modifiers: Vec<SatModifier>,
    base: HashMap<Key, Tile>,
    next_id: u32,
}

/// Pixels of the satellite image, both ends included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}
impl PixelRect {
    pub fn union(self, o: PixelRect) -> PixelRect {
        PixelRect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
}

/// The colour of pixel `t` of a tile with `layers` laid over the original colour `base` in order.
pub fn compose_pixel(base: [u8; 3], layers: &[Layer], t: usize) -> [u8; 3] {
    let mut c = [base[0] as u32, base[1] as u32, base[2] as u32];
    for layer in layers {
        match layer {
            Layer::Paint(l, opacity) => {
                let a = (l[t * 4 + 3] as u32 * opacity + 127) / 255;
                if a > 0 {
                    for k in 0..3 {
                        c[k] = (c[k] * (255 - a) + l[t * 4 + k] as u32 * a + 127) / 255;
                    }
                }
            }
            Layer::Correct(adjust, mask, opacity) => {
                let a = (mask.map_or(255, |m| m[t] as u32) * opacity + 127) / 255;
                if a > 0 {
                    let to = adjust.apply([c[0] as u8, c[1] as u8, c[2] as u8]);
                    for k in 0..3 {
                        c[k] = (c[k] * (255 - a) + to[k] as u32 * a + 127) / 255;
                    }
                }
            }
        }
    }
    [c[0] as u8, c[1] as u8, c[2] as u8]
}

fn tile_rect(key: Key, img: &RgbImage) -> Option<PixelRect> {
    let (x0, y0) = (key.0 as usize * TILE, key.1 as usize * TILE);
    (x0 < img.width && y0 < img.height).then(|| PixelRect { x0, y0, x1: (x0 + TILE).min(img.width) - 1, y1: (y0 + TILE).min(img.height) - 1 })
}

impl SatLayers {
    pub fn is_empty(&self) -> bool {
        self.modifiers.is_empty()
    }
    pub fn position(&self, id: u32) -> Option<usize> {
        self.modifiers.iter().position(|m| m.id == id)
    }
    /// Every tile any of the given modifiers holds.
    pub fn keys_of(list: &[SatModifier]) -> BTreeSet<Key> {
        list.iter().flat_map(|m| m.tiles.keys().copied()).collect()
    }
    /// Layers made of `modifiers` with the original pixels `base` under their tiles.
    pub fn from_parts(modifiers: Vec<SatModifier>, base: HashMap<Key, Tile>) -> Self {
        let next_id = modifiers.iter().map(|m| m.id.saturating_add(1)).max().unwrap_or(1);
        SatLayers { modifiers, base, next_id }
    }
    /// The original pixels kept under a tile a modifier touches.
    pub fn base_tile(&self, key: Key) -> Option<&Tile> {
        self.base.get(&key)
    }
    /// The modifier just under `id` that it can be merged into: of the same brush kind, and
    /// both shown, so the merged one looks the same.
    pub fn merge_target(&self, id: u32) -> Option<u32> {
        let at = self.position(id)?;
        let (lower, upper) = (self.modifiers.get(at.checked_sub(1)?)?, &self.modifiers[at]);
        (lower.kind == upper.kind && upper.kind.is_brush() && lower.enabled && upper.enabled).then_some(lower.id)
    }
    /// Adds an empty modifier and returns its id.
    pub fn add(&mut self, kind: SatKind, name: String) -> u32 {
        // Random, so that two people working on copies of one map do not make the same id.
        let id = crate::maskmod::free_id(|id| self.modifiers.iter().any(|m| m.id == id));
        self.next_id = self.next_id.max(id.saturating_add(1));
        let correction = (kind == SatKind::Correct).then(Correction::default);
        self.modifiers.push(SatModifier { id, name, kind, enabled: true, opacity: 255, tiles: HashMap::new(), correction });
        id
    }

    /// The layers in the form saved beside a project.
    pub fn to_bytes(&self) -> Vec<u8> {
        // Without corrections the file keeps the form older versions read.
        let with_settings = self.modifiers.iter().any(|m| m.correction.is_some());
        let mut b = if with_settings { b"DZSM3".to_vec() } else { b"DZSM2".to_vec() };
        b.extend(self.next_id.to_le_bytes());
        let tiles = |b: &mut Vec<u8>, map: &HashMap<Key, Tile>| {
            b.extend((map.len() as u32).to_le_bytes());
            let mut keys: Vec<&Key> = map.keys().collect();
            keys.sort();
            for k in keys {
                b.extend(k.0.to_le_bytes());
                b.extend(k.1.to_le_bytes());
                b.extend((map[k].len() as u32).to_le_bytes());
                b.extend(map[k].iter());
            }
        };
        tiles(&mut b, &self.base);
        b.extend((self.modifiers.len() as u32).to_le_bytes());
        for m in &self.modifiers {
            b.extend(m.id.to_le_bytes());
            b.push(match m.kind {
                SatKind::Bake => 0,
                SatKind::Clone => 1,
                SatKind::Adjust => 2,
                SatKind::Paint => 3,
                SatKind::Correct => 4,
                // Older versions read it as clone strokes, which it is drawn like.
                SatKind::Generated => 5,
            });
            b.push(m.enabled as u8);
            b.push(m.opacity);
            b.extend((m.name.len() as u32).to_le_bytes());
            b.extend(m.name.as_bytes());
            if with_settings {
                let settings = m.correction.as_ref().and_then(|c| serde_json::to_vec(c).ok()).unwrap_or_default();
                b.extend((settings.len() as u32).to_le_bytes());
                b.extend(&settings);
            }
            tiles(&mut b, &m.tiles);
        }
        b
    }

    pub fn from_bytes(b: &[u8]) -> anyhow::Result<Self> {
        // The first form of the file had no opacity, the second no corrections.
        let with_settings = b.starts_with(b"DZSM3");
        let with_opacity = with_settings || b.starts_with(b"DZSM2");
        anyhow::ensure!(with_opacity || b.starts_with(b"DZSM1"), "Not a satellite modifier file");
        let mut r = Bytes { b, p: 5 };
        let next_id = r.int()?;
        let base = r.tiles(3)?;
        let count = r.int()? as usize;
        let mut modifiers = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let id = r.int()?;
            let head = r.take(2)?;
            let kind = match head[0] {
                0 => SatKind::Bake,
                2 => SatKind::Adjust,
                3 => SatKind::Paint,
                4 => SatKind::Correct,
                5 => SatKind::Generated,
                _ => SatKind::Clone,
            };
            let enabled = head[1] != 0;
            let opacity = if with_opacity { r.take(1)?[0] } else { 255 };
            let len = r.int()? as usize;
            let name = String::from_utf8_lossy(r.take(len)?).into_owned();
            let correction = if with_settings {
                let n = r.int()? as usize;
                let bytes = r.take(n)?;
                (n > 0).then(|| serde_json::from_slice(bytes)).transpose()?
            } else {
                None
            };
            let pixel = if kind == SatKind::Correct { 1 } else { 4 };
            modifiers.push(SatModifier { id, name, kind, enabled, opacity, tiles: r.tiles(pixel)?, correction });
        }
        Ok(Self { modifiers, base, next_id })
    }
}

struct Bytes<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Bytes<'a> {
    fn take(&mut self, n: usize) -> anyhow::Result<&'a [u8]> {
        use anyhow::Context;
        let s = self.b.get(self.p..self.p + n).context("Satellite modifier file is cut short")?;
        self.p += n;
        Ok(s)
    }
    fn int(&mut self) -> anyhow::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn tiles(&mut self, pixel: usize) -> anyhow::Result<HashMap<Key, Tile>> {
        let n = self.int()? as usize;
        let mut map = HashMap::with_capacity(n.min(1 << 16));
        for _ in 0..n {
            let k = self.take(4)?;
            let key = (u16::from_le_bytes([k[0], k[1]]), u16::from_le_bytes([k[2], k[3]]));
            let len = self.int()? as usize;
            anyhow::ensure!(len == TILE * TILE * pixel, "Satellite modifier tile has the wrong size");
            map.insert(key, Arc::new(self.take(len)?.to_vec()));
        }
        Ok(map)
    }
}

impl World {
    /// The satellite image without what the modifiers lay over it.
    pub fn sat_original(&self) -> RgbImage {
        let Some(img) = &self.satellite else { return RgbImage::default() };
        let mut out = img.clone();
        for (key, base) in &self.sat.base {
            let Some(r) = tile_rect(*key, &out) else { continue };
            for y in r.y0..=r.y1 {
                let w = (r.x1 - r.x0 + 1) * 3;
                let to = (y * out.width + r.x0) * 3;
                let from = (y - r.y0) * TILE * 3;
                out.data[to..to + w].copy_from_slice(&base[from..from + w]);
            }
        }
        out
    }

    /// Keeps the original pixels of a tile before a modifier first writes to it.
    fn sat_keep(&mut self, key: Key) {
        if self.sat.base.contains_key(&key) {
            return;
        }
        let Some(img) = &self.satellite else { return };
        let Some(r) = tile_rect(key, img) else { return };
        let mut t = vec![0u8; TILE * TILE * 3];
        for y in r.y0..=r.y1 {
            let w = (r.x1 - r.x0 + 1) * 3;
            let from = (y * img.width + r.x0) * 3;
            let to = (y - r.y0) * TILE * 3;
            t[to..to + w].copy_from_slice(&img.data[from..from + w]);
        }
        self.sat.base.insert(key, Arc::new(t));
    }

    /// The tiles modifier `m` lays something over: all of the image for a whole correction.
    pub fn sat_mod_keys(&self, m: &SatModifier) -> Vec<Key> {
        if m.correction.as_ref().is_some_and(|c| c.whole) {
            let Some(img) = &self.satellite else { return Vec::new() };
            let (cols, rows) = (img.width.div_ceil(TILE), img.height.div_ceil(TILE));
            return (0..rows).flat_map(|y| (0..cols).map(move |x| (x as u16, y as u16))).collect();
        }
        m.tiles.keys().copied().collect()
    }

    fn sat_keys_of(&self, list: &[SatModifier]) -> BTreeSet<Key> {
        list.iter().flat_map(|m| self.sat_mod_keys(m)).collect()
    }

    /// Lays the enabled modifiers over the original pixels of the given tiles again.
    pub fn sat_recompose(&mut self, keys: impl IntoIterator<Item = Key>) -> Option<PixelRect> {
        use rayon::prelude::*;
        let keys: Vec<Key> = keys.into_iter().collect();
        for key in &keys {
            self.sat_keep(*key);
        }
        let img = self.satellite.as_ref()?;
        // Each tile worked out on its own, as many at a time as there are cores.
        let done: Vec<(PixelRect, Vec<u8>)> = keys.par_iter().filter_map(|&key| {
            let base = self.sat.base.get(&key)?;
            let r = tile_rect(key, img)?;
            let layers: Vec<Layer> = self.sat.modifiers.iter().filter(|m| m.enabled && m.opacity > 0).filter_map(|m| match &m.correction {
                Some(c) if c.whole => Some(Layer::Correct(&c.adjust, None, m.opacity as u32)),
                Some(c) => m.tiles.get(&key).map(|t| Layer::Correct(&c.adjust, Some(t), m.opacity as u32)),
                None => m.tiles.get(&key).map(|t| Layer::Paint(t, m.opacity as u32)),
            }).collect();
            let mut out = Vec::with_capacity((r.x1 - r.x0 + 1) * (r.y1 - r.y0 + 1) * 3);
            for y in r.y0..=r.y1 {
                for x in r.x0..=r.x1 {
                    let t = (y - r.y0) * TILE + (x - r.x0);
                    out.extend(compose_pixel([base[t * 3], base[t * 3 + 1], base[t * 3 + 2]], &layers, t));
                }
            }
            Some((r, out))
        }).collect();
        let img = self.satellite.as_mut()?;
        let mut touched: Option<PixelRect> = None;
        for (r, pixels) in done {
            let w = (r.x1 - r.x0 + 1) * 3;
            for y in r.y0..=r.y1 {
                let o = (y * img.width + r.x0) * 3;
                img.data[o..o + w].copy_from_slice(&pixels[(y - r.y0) * w..(y - r.y0 + 1) * w]);
            }
            touched = Some(touched.map_or(r, |t| t.union(r)));
        }
        touched
    }

    /// Adds a correction over the whole image, or where `mask` (pixels and strength 0..255)
    /// says, and returns its id and what changed.
    pub fn sat_add_correction(&mut self, name: String, adjust: Adjust, mask: Option<&[(usize, usize, u32)]>) -> (u32, Option<PixelRect>) {
        let id = self.sat.add(SatKind::Correct, name);
        let at = self.sat.modifiers.len() - 1;
        self.sat.modifiers[at].correction = Some(Correction { adjust, whole: mask.is_none() });
        if let Some(mask) = mask {
            let tiles = &mut self.sat.modifiers[at].tiles;
            for &(x, y, a) in mask {
                let tile = Arc::make_mut(tiles.entry(((x / TILE) as u16, (y / TILE) as u16)).or_insert_with(|| Arc::new(vec![0u8; TILE * TILE])));
                let p = (y % TILE) * TILE + x % TILE;
                tile[p] = tile[p].max(a.min(255) as u8);
            }
        }
        let keys = self.sat_mod_keys(&self.sat.modifiers[at].clone());
        (id, self.sat_recompose(keys))
    }

    /// Changes the settings of correction `id`; the image under it is worked out again.
    pub fn sat_set_correction(&mut self, id: u32, adjust: Adjust) -> Option<PixelRect> {
        let at = self.sat.position(id)?;
        self.sat.modifiers[at].correction.as_mut()?.adjust = adjust;
        let keys = self.sat_mod_keys(&self.sat.modifiers[at].clone());
        self.sat_recompose(keys)
    }

    /// Replaces the list of modifiers and returns the pixels that changed.
    pub fn sat_set_modifiers(&mut self, list: &mut Vec<SatModifier>) -> Option<PixelRect> {
        let mut keys = self.sat_keys_of(&self.sat.modifiers);
        keys.extend(self.sat_keys_of(list));
        std::mem::swap(list, &mut self.sat.modifiers);
        self.sat_recompose(keys)
    }

    /// Removes a modifier; what it covered shows again.
    pub fn sat_remove(&mut self, id: u32) -> Option<PixelRect> {
        let at = self.sat.position(id)?;
        let gone = self.sat.modifiers.remove(at);
        let keys = self.sat_mod_keys(&gone);
        self.sat_recompose(keys)
    }

    /// [`SatLayers::merge_target`], and in team work only when both are this person's own:
    /// merging takes the upper one away, which would take it from its maker's copy too.
    pub fn sat_merge_target(&self, id: u32) -> Option<u32> {
        let mine = |id: u32| self.team_me.is_empty() || self.sat_mod_author(id) == self.team_me;
        self.sat.merge_target(id).filter(|lower| mine(id) && mine(*lower))
    }

    /// Merges a brush modifier into the one under it (see [`World::sat_merge_target`]); the
    /// lower one keeps its name and place. Tells whether they were merged.
    pub fn sat_merge_down(&mut self, id: u32) -> (bool, Option<PixelRect>) {
        let Some(lower) = self.sat_merge_target(id) else { return (false, None) };
        let (lo, up) = (self.sat.position(lower).unwrap(), self.sat.position(id).unwrap());
        let upper = self.sat.modifiers.remove(up);
        let l = &mut self.sat.modifiers[lo];
        let keys: BTreeSet<Key> = l.tiles.keys().chain(upper.tiles.keys()).copied().collect();
        let tiles = keys.iter().map(|k| (*k, merge_tiles(l.tiles.get(k).map(|t| (t, l.opacity)), upper.tiles.get(k).map(|t| (t, upper.opacity))))).collect();
        l.tiles = tiles;
        l.opacity = 255;
        (true, self.sat_recompose(keys))
    }

    /// Merges every brush modifier into the one under it where it can be, so each run of
    /// neighbouring modifiers of one brush becomes one. Tells how many went into others.
    pub fn sat_merge_brushes(&mut self) -> (usize, Option<PixelRect>) {
        let (mut merged, mut touched) = (0, None::<PixelRect>);
        let mut at = self.sat.modifiers.len();
        while at > 1 {
            at -= 1;
            let id = self.sat.modifiers[at].id;
            if let (true, r) = self.sat_merge_down(id) {
                merged += 1;
                touched = match (touched, r) {
                    (Some(a), Some(b)) => Some(a.union(b)),
                    (a, b) => a.or(b),
                };
            }
        }
        (merged, touched)
    }

    pub fn sat_set_enabled(&mut self, id: u32, enabled: bool) -> Option<PixelRect> {
        let at = self.sat.position(id)?;
        self.sat.modifiers[at].enabled = enabled;
        let keys = self.sat_mod_keys(&self.sat.modifiers[at].clone());
        self.sat_recompose(keys)
    }

    /// Sets how much of a modifier shows over what is under it.
    pub fn sat_set_opacity(&mut self, id: u32, opacity: u8) -> Option<PixelRect> {
        let at = self.sat.position(id)?;
        self.sat.modifiers[at].opacity = opacity;
        let keys = self.sat_mod_keys(&self.sat.modifiers[at].clone());
        self.sat_recompose(keys)
    }

    /// After another image became the satellite: it is the new original, with the modifiers on it.
    pub fn sat_image_replaced(&mut self) -> Option<PixelRect> {
        self.sat.base.clear();
        let keys = self.sat_keys_of(&self.sat.modifiers);
        self.sat_recompose(keys)
    }

    /// Adds tiles to a modifier, over what it already holds.
    pub fn sat_add_tiles(&mut self, id: u32, tiles: HashMap<Key, Tile>) -> Option<PixelRect> {
        let at = self.sat.position(id)?;
        let keys: Vec<Key> = tiles.keys().copied().collect();
        for (k, t) in tiles {
            self.sat_keep(k);
            match self.sat.modifiers[at].tiles.get_mut(&k) {
                Some(old) => {
                    let old = Arc::make_mut(old);
                    for (o, n) in old.chunks_exact_mut(4).zip(t.chunks_exact(4)) {
                        if n[3] > 0 {
                            o.copy_from_slice(n);
                        }
                    }
                }
                None => {
                    self.sat.modifiers[at].tiles.insert(k, t);
                }
            }
        }
        self.sat_recompose(keys)
    }

    /// One touch of the clone brush on the modifier `id`, centred on a pixel of the satellite.
    /// The colour comes from `source` (the satellite itself when `None`) at the same pixel
    /// moved by `offset`.
    pub fn sat_clone_dab(&mut self, id: u32, center: [f32; 2], brush: &CloneBrush, source: Option<&RgbImage>, offset: [i64; 2]) -> Option<PixelRect> {
        self.sat_clone_dab_mapped(id, center, brush, source, &SourceMap::shift(offset), false)
    }

    /// [`Self::sat_clone_dab`] with the colour read where `map` takes each pixel: moved, turned
    /// and scaled. With `match_colour` the copy takes on the average colour of the place it is
    /// painted on, so a patch from elsewhere blends in, as a healing brush does.
    pub fn sat_clone_dab_mapped(&mut self, id: u32, center: [f32; 2], brush: &CloneBrush, source: Option<&RgbImage>, map: &SourceMap, match_colour: bool) -> Option<PixelRect> {
        let at = self.sat.position(id)?;
        let (w, h) = self.satellite.as_ref().map(|s| (s.width, s.height))?;
        let r = brush.radius.max(0.5);
        let (x0, y0) = ((center[0] - r).floor().max(0.0) as usize, (center[1] - r).floor().max(0.0) as usize);
        let (x1, y1) = (((center[0] + r).ceil() as usize).min(w.saturating_sub(1)), ((center[1] + r).ceil() as usize).min(h.saturating_sub(1)));
        if x0 > x1 || y0 > y1 || center[0] + r < 0.0 || center[1] + r < 0.0 {
            return None;
        }
        // The colours are read before anything is written, so a source under the brush is not smeared.
        let mut dabs: Vec<(usize, usize, [f32; 3], u32)> = Vec::new();
        let (mut here, mut there, mut weight) = ([0f64; 3], [0f64; 3], 0f64);
        {
            let img = source.or(self.satellite.as_ref())?;
            let shown = self.satellite.as_ref()?;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let d = ((x as f32 + 0.5 - center[0]).powi(2) + (y as f32 + 0.5 - center[1]).powi(2)).sqrt() / r;
                    if d >= 1.0 {
                        continue;
                    }
                    let edge = if brush.hardness >= 0.999 { 1.0 } else { 1.0 - ((d - brush.hardness) / (1.0 - brush.hardness)).clamp(0.0, 1.0) };
                    let a = (brush.opacity.clamp(0.0, 1.0) * edge * edge * (3.0 - 2.0 * edge) * 255.0) as u32;
                    if a == 0 {
                        continue;
                    }
                    let s = map.apply([x as f32 + 0.5, y as f32 + 0.5]);
                    let Some(c) = sample_image(img, s) else { continue };
                    if match_colour {
                        let o = (y * shown.width + x) * 3;
                        let k = a as f64;
                        for i in 0..3 {
                            here[i] += shown.data[o + i] as f64 * k;
                            there[i] += c[i] as f64 * k;
                        }
                        weight += k;
                    }
                    dabs.push((x, y, c, a));
                }
            }
        }
        // The copy moved by the difference of the averages under the brush.
        let shift: [f32; 3] = if match_colour && weight > 0.0 { std::array::from_fn(|i| ((here[i] - there[i]) / weight) as f32) } else { [0.0; 3] };
        let mut keys = BTreeSet::new();
        for (x, y, c, a) in dabs {
            let c: [u32; 3] = std::array::from_fn(|i| (c[i] + shift[i]).round().clamp(0.0, 255.0) as u32);
            let key = ((x / TILE) as u16, (y / TILE) as u16);
            if keys.insert(key) {
                self.sat_keep(key);
            }
            let tile = Arc::make_mut(self.sat.modifiers[at].tiles.entry(key).or_insert_with(|| Arc::new(vec![0u8; TILE * TILE * 4])));
            let p = ((y % TILE) * TILE + x % TILE) * 4;
            // The new colour over what the layer already holds there.
            let old = tile[p + 3] as u32;
            let out = a + old * (255 - a) / 255;
            if out > 0 {
                for k in 0..3 {
                    tile[p + k] = ((c[k] * a + tile[p + k] as u32 * old * (255 - a) / 255) / out).min(255) as u8;
                }
                tile[p + 3] = out.min(255) as u8;
            }
        }
        self.sat_recompose(keys)
    }

    /// One touch of the adjust brush on modifier `id`: the image under the brush with `adjust`
    /// applied, laid over it as far as the brush is strong. `before` keeps the colour each pixel
    /// had when the stroke first reached it, so going over it again in the same stroke does not
    /// pile the change up; a new stroke does.
    pub fn sat_adjust_dab(&mut self, id: u32, center: [f32; 2], brush: &CloneBrush, adjust: &Adjust, before: &mut HashMap<usize, [u8; 3]>) -> Option<PixelRect> {
        let (w, h) = self.satellite.as_ref().map(|s| (s.width, s.height))?;
        self.sat_paint(id, &dab_pixels(center, brush, w, h), &PaintOp::Adjust(*adjust, 0.0), before)
    }

    /// Lays `op` over the pixels `(x, y, strength 0..255)` of the satellite, into modifier `id`.
    /// `before` keeps the colour each pixel had when the stroke or fill first reached it: the
    /// operation always works from those, so going over a pixel again in the same stroke does
    /// not pile the change up, and blurring reads the image as it was.
    pub fn sat_paint(&mut self, id: u32, pixels: &[(usize, usize, u32)], op: &PaintOp, before: &mut HashMap<usize, [u8; 3]>) -> Option<PixelRect> {
        let at = self.sat.position(id)?;
        let img = self.satellite.as_ref()?;
        let (w, h) = (img.width, img.height);
        for &(x, y, _) in pixels {
            let o = y * w + x;
            before.entry(o).or_insert_with(|| [img.data[o * 3], img.data[o * 3 + 1], img.data[o * 3 + 2]]);
        }
        let get = |x: i64, y: i64| -> [u8; 3] {
            let (x, y) = (x.clamp(0, w as i64 - 1) as usize, y.clamp(0, h as i64 - 1) as usize);
            let o = y * w + x;
            before.get(&o).copied().unwrap_or([img.data[o * 3], img.data[o * 3 + 1], img.data[o * 3 + 2]])
        };
        let dabs: Vec<(usize, usize, [u8; 3], u32)> = pixels.iter().filter(|p| p.2 > 0).map(|&(x, y, a)| (x, y, op.apply(x, y, &get), a)).collect();
        let mut keys = BTreeSet::new();
        for (x, y, c, a) in dabs {
            let key = ((x / TILE) as u16, (y / TILE) as u16);
            if keys.insert(key) {
                self.sat_keep(key);
            }
            let tile = Arc::make_mut(self.sat.modifiers[at].tiles.entry(key).or_insert_with(|| Arc::new(vec![0u8; TILE * TILE * 4])));
            let p = ((y % TILE) * TILE + x % TILE) * 4;
            let old = tile[p + 3] as u32;
            let out = a + old * (255 - a) / 255;
            if out > 0 {
                for k in 0..3 {
                    tile[p + k] = ((c[k] as u32 * a + tile[p + k] as u32 * old * (255 - a) / 255) / out).min(255) as u8;
                }
                tile[p + 3] = out.min(255) as u8;
            }
        }
        if keys.is_empty() {
            return None;
        }
        self.sat_recompose(keys)
    }

    /// Takes back what modifier `id` painted under the brush, as far as the brush is strong,
    /// so the image under it shows again.
    pub fn sat_erase_dab(&mut self, id: u32, center: [f32; 2], brush: &CloneBrush) -> Option<PixelRect> {
        let at = self.sat.position(id)?;
        let (w, h) = self.satellite.as_ref().map(|s| (s.width, s.height))?;
        let r = brush.radius.max(0.5);
        let (x0, y0) = ((center[0] - r).floor().max(0.0) as usize, (center[1] - r).floor().max(0.0) as usize);
        let (x1, y1) = (((center[0] + r).ceil() as usize).min(w.saturating_sub(1)), ((center[1] + r).ceil() as usize).min(h.saturating_sub(1)));
        if x0 > x1 || y0 > y1 || center[0] + r < 0.0 || center[1] + r < 0.0 {
            return None;
        }
        let mut keys = BTreeSet::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = ((x as f32 + 0.5 - center[0]).powi(2) + (y as f32 + 0.5 - center[1]).powi(2)).sqrt() / r;
                if d >= 1.0 {
                    continue;
                }
                let edge = if brush.hardness >= 0.999 { 1.0 } else { 1.0 - ((d - brush.hardness) / (1.0 - brush.hardness)).clamp(0.0, 1.0) };
                let a = (brush.opacity.clamp(0.0, 1.0) * edge * edge * (3.0 - 2.0 * edge) * 255.0) as u32;
                let key = ((x / TILE) as u16, (y / TILE) as u16);
                let Some(tile) = self.sat.modifiers[at].tiles.get_mut(&key) else { continue };
                let p = ((y % TILE) * TILE + x % TILE) * 4 + 3;
                if a == 0 || tile[p] == 0 {
                    continue;
                }
                Arc::make_mut(tile)[p] = (tile[p] as u32 * (255 - a) / 255) as u8;
                keys.insert(key);
            }
        }
        if keys.is_empty() {
            return None;
        }
        self.sat_recompose(keys)
    }
}

/// The pixels under a round brush at `center`, with how strongly the brush touches each.
pub fn dab_pixels(center: [f32; 2], brush: &CloneBrush, w: usize, h: usize) -> Vec<(usize, usize, u32)> {
    let r = brush.radius.max(0.5);
    let (x0, y0) = ((center[0] - r).floor().max(0.0) as usize, (center[1] - r).floor().max(0.0) as usize);
    let (x1, y1) = (((center[0] + r).ceil() as usize).min(w.saturating_sub(1)), ((center[1] + r).ceil() as usize).min(h.saturating_sub(1)));
    let mut out = Vec::new();
    if x0 > x1 || y0 > y1 || center[0] + r < 0.0 || center[1] + r < 0.0 {
        return out;
    }
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = ((x as f32 + 0.5 - center[0]).powi(2) + (y as f32 + 0.5 - center[1]).powi(2)).sqrt() / r;
            if d >= 1.0 {
                continue;
            }
            let edge = if brush.hardness >= 0.999 { 1.0 } else { 1.0 - ((d - brush.hardness) / (1.0 - brush.hardness)).clamp(0.0, 1.0) };
            let a = (brush.opacity.clamp(0.0, 1.0) * edge * edge * (3.0 - 2.0 * edge) * 255.0) as u32;
            if a > 0 {
                out.push((x, y, a));
            }
        }
    }
    out
}

/// The pixels inside `polygons` and within half of `widths` of `lines` (all in pixels of the
/// image), fading out over `feather` pixels at their edges (inwards for polygons, across the
/// edge for lines), at `strength` 0 to 1.
pub fn shape_pixels(polygons: &[Vec<[f32; 2]>], lines: &[(Vec<[f32; 2]>, f32)], feather: f32, strength: f32, w: usize, h: usize) -> Vec<(usize, usize, u32)> {
    let feather = feather.max(0.5);
    let segs = |pts: &[[f32; 2]], closed: bool| -> Vec<([f32; 2], [f32; 2])> {
        let mut v: Vec<_> = pts.windows(2).map(|s| (s[0], s[1])).collect();
        if closed && pts.len() > 2 {
            v.push((pts[pts.len() - 1], pts[0]));
        }
        v
    };
    let dist = |p: [f32; 2], s: &[([f32; 2], [f32; 2])]| s.iter().map(|(a, b)| {
        let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
        let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-6)).clamp(0.0, 1.0);
        (ap[0] - ab[0] * t).hypot(ap[1] - ab[1] * t)
    }).fold(f32::MAX, f32::min);
    let mut best: HashMap<(usize, usize), f32> = HashMap::new();
    let mut put = |x: usize, y: usize, k: f32| {
        let e = best.entry((x, y)).or_insert(0.0);
        *e = e.max(k);
    };
    let span = |pts: &[[f32; 2]], pad: f32| {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in pts {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let x0 = (lo[0] - pad).floor().max(0.0) as usize;
        let y0 = (lo[1] - pad).floor().max(0.0) as usize;
        let x1 = ((hi[0] + pad).ceil().max(0.0) as usize).min(w.saturating_sub(1));
        let y1 = ((hi[1] + pad).ceil().max(0.0) as usize).min(h.saturating_sub(1));
        (x0, y0, x1, y1)
    };
    for poly in polygons.iter().filter(|p| p.len() >= 3) {
        let edges = segs(poly, true);
        let (x0, y0, x1, y1) = span(poly, 0.0);
        for y in y0..=y1 {
            let py = y as f32 + 0.5;
            // Where the rows cross the outline, for inside or out.
            let mut xs: Vec<f32> = edges.iter().filter(|(a, b)| (a[1] <= py) != (b[1] <= py)).map(|(a, b)| a[0] + (py - a[1]) / (b[1] - a[1]) * (b[0] - a[0])).collect();
            xs.sort_by(f32::total_cmp);
            for pair in xs.chunks_exact(2) {
                let (from, to) = ((pair[0] - 0.5).ceil().max(x0 as f32) as usize, ((pair[1] - 0.5).floor().max(0.0) as usize).min(x1));
                for x in from..=to.max(from) {
                    if (x as f32 + 0.5) < pair[0] || (x as f32 + 0.5) > pair[1] {
                        continue;
                    }
                    let d = dist([x as f32 + 0.5, py], &edges);
                    let t = (d / feather).clamp(0.0, 1.0);
                    put(x, y, t * t * (3.0 - 2.0 * t));
                }
            }
        }
    }
    // A line's soft edge lies across its nominal edge, so the band keeps its width and is
    // at full strength along its middle however soft the edge is.
    for (line, width) in lines.iter().filter(|l| l.0.len() >= 2) {
        let width = width.max(1.0);
        let soft = feather.min(width * 0.5);
        let half = width * 0.5 + soft * 0.5;
        let edges = segs(line, false);
        let (x0, y0, x1, y1) = span(line, half);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = dist([x as f32 + 0.5, y as f32 + 0.5], &edges);
                if d >= half {
                    continue;
                }
                let t = ((half - d) / soft).clamp(0.0, 1.0);
                put(x, y, t * t * (3.0 - 2.0 * t));
            }
        }
    }
    let mut out: Vec<(usize, usize, u32)> = best.into_iter().map(|((x, y), k)| (x, y, (k * strength.clamp(0.0, 1.0) * 255.0) as u32)).filter(|p| p.2 > 0).collect();
    out.sort_unstable();
    out
}

/// How a colour is laid over the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendMode {
    /// The colour itself.
    Normal,
    /// The colour's hue and saturation over the image's lightness: recolours and keeps the pattern.
    Tint,
    /// Darkens: the image times the colour.
    Multiply,
    /// Lightens: the opposite of multiply.
    Screen,
}

/// What a painting brush or fill does to each pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaintOp {
    /// The colours adjusted, after a blur reaching this many pixels (0 for none).
    Adjust(Adjust, f32),
    Colour([u8; 3], BlendMode),
}

impl PaintOp {
    /// The new colour of pixel (x, y), with `get` the image as it was before.
    pub fn apply(&self, x: usize, y: usize, get: &dyn Fn(i64, i64) -> [u8; 3]) -> [u8; 3] {
        let (x, y) = (x as i64, y as i64);
        match *self {
            PaintOp::Adjust(adjust, blur) => {
                let c = if blur >= 1.0 {
                    // A soft blur from 7 x 7 samples across its reach.
                    let (mut sum, mut weight) = ([0f32; 3], 0f32);
                    for j in -3i64..=3 {
                        for i in -3i64..=3 {
                            let k = (-((i * i + j * j) as f32) / 4.5).exp();
                            let c = get(x + (i as f32 * blur / 3.0).round() as i64, y + (j as f32 * blur / 3.0).round() as i64);
                            for n in 0..3 {
                                sum[n] += c[n] as f32 * k;
                            }
                            weight += k;
                        }
                    }
                    sum.map(|v| (v / weight).round().clamp(0.0, 255.0) as u8)
                } else {
                    get(x, y)
                };
                adjust.apply(c)
            }
            PaintOp::Colour(colour, mode) => {
                let c = get(x, y);
                let (b, p) = (c.map(|v| v as f32 / 255.0), colour.map(|v| v as f32 / 255.0));
                let out: [f32; 3] = match mode {
                    BlendMode::Normal => p,
                    BlendMode::Multiply => std::array::from_fn(|k| b[k] * p[k]),
                    BlendMode::Screen => std::array::from_fn(|k| 1.0 - (1.0 - b[k]) * (1.0 - p[k])),
                    BlendMode::Tint => {
                        let lum = |v: [f32; 3]| 0.299 * v[0] + 0.587 * v[1] + 0.114 * v[2];
                        let shift = lum(b) - lum(p);
                        std::array::from_fn(|k| p[k] + shift)
                    }
                };
                out.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            }
        }
    }
}

/// What the adjust brush does to the colours under it; all 0 leaves them as they are.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Adjust {
    /// -1 darker to 1 lighter.
    pub brightness: f32,
    /// -1 flat to 1 strong.
    pub contrast: f32,
    /// -1 grey to 1 vivid.
    pub saturation: f32,
    /// Turn of the hue in degrees.
    pub hue: f32,
    /// -1 cooler (bluer) to 1 warmer (yellower).
    pub warmth: f32,
    /// Brightness and contrast as Photoshop's legacy ones: brightness moves every colour by the
    /// same amount (up to 150 of 255) and full contrast splits each channel at its middle.
    pub legacy: bool,
}

impl Adjust {
    pub fn apply(&self, c: [u8; 3]) -> [u8; 3] {
        let mut v: [f32; 3] = c.map(|x| x as f32 / 255.0);
        let b = self.brightness.clamp(-1.0, 1.0);
        let contrast = self.contrast.clamp(-1.0, 1.0);
        if self.legacy {
            // Photoshop's legacy mode: a plain shift, then contrast about the middle that
            // grows without end towards full, where every channel is either off or full.
            for x in &mut v {
                *x = (*x + b * 150.0 / 255.0).clamp(0.0, 1.0);
            }
            let k = if contrast >= 0.0 { 1.0 / (1.0 - contrast).max(1e-4) } else { 1.0 + contrast };
            for x in &mut v {
                *x = (*x - 0.5) * k + 0.5;
            }
        } else {
            // Lighter towards white and darker towards black, so the pattern stays.
            for x in &mut v {
                *x = if b >= 0.0 { *x + (1.0 - *x) * b } else { *x * (1.0 + b) };
            }
            let k = 1.0 + contrast;
            for x in &mut v {
                *x = (*x - 0.5) * k + 0.5;
            }
        }
        // Hue turned about the grey axis, then saturation from the grey of the same lightness.
        if self.hue != 0.0 {
            let (s, cs) = self.hue.to_radians().sin_cos();
            let (y, i, q) = (0.299 * v[0] + 0.587 * v[1] + 0.114 * v[2], 0.596 * v[0] - 0.274 * v[1] - 0.322 * v[2], 0.211 * v[0] - 0.523 * v[1] + 0.312 * v[2]);
            let (i, q) = (i * cs - q * s, i * s + q * cs);
            v = [y + 0.956 * i + 0.621 * q, y - 0.272 * i - 0.647 * q, y - 1.106 * i + 1.703 * q];
        }
        let grey = 0.299 * v[0] + 0.587 * v[1] + 0.114 * v[2];
        let s = 1.0 + self.saturation.clamp(-1.0, 1.0);
        for x in &mut v {
            *x = grey + (*x - grey) * s;
        }
        let warm = self.warmth.clamp(-1.0, 1.0) * 0.1;
        v[0] += warm;
        v[1] += warm * 0.4;
        v[2] -= warm;
        v.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)
    }
    pub fn is_neutral(&self) -> bool {
        Self { legacy: false, ..*self } == Self::default()
    }
}

/// Where the clone brush reads the colour of each pixel it paints: the point `from` of the
/// source goes to `to` on the satellite, and everything round it turns by `turn` degrees and
/// grows by `scale` with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceMap {
    pub to: [f32; 2],
    pub from: [f32; 2],
    pub turn: f32,
    pub scale: f32,
}

impl SourceMap {
    /// Every pixel read `offset` pixels away.
    pub fn shift(offset: [i64; 2]) -> Self {
        Self { to: [0.0; 2], from: [offset[0] as f32, offset[1] as f32], turn: 0.0, scale: 1.0 }
    }
    /// The point of the source read for point `p` of the satellite.
    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        let (d, (s, c)) = ([p[0] - self.to[0], p[1] - self.to[1]], self.turn.to_radians().sin_cos());
        let k = 1.0 / self.scale.max(0.01);
        [self.from[0] + (d[0] * c - d[1] * s) * k, self.from[1] + (d[0] * s + d[1] * c) * k]
    }
}

/// The colour of `img` at a point, blended from the four pixels round it; `None` outside it.
fn sample_image(img: &RgbImage, p: [f32; 2]) -> Option<[f32; 3]> {
    let (fx, fy) = (p[0] - 0.5, p[1] - 0.5);
    if fx < -0.5 || fy < -0.5 || fx > img.width as f32 - 0.5 || fy > img.height as f32 - 0.5 {
        return None;
    }
    let (x0, y0) = (fx.floor().max(0.0) as usize, fy.floor().max(0.0) as usize);
    let (x1, y1) = ((x0 + 1).min(img.width - 1), (y0 + 1).min(img.height - 1));
    let (tx, ty) = ((fx - x0 as f32).clamp(0.0, 1.0), (fy - y0 as f32).clamp(0.0, 1.0));
    let px = |x: usize, y: usize, k: usize| img.data[(y * img.width + x) * 3 + k] as f32;
    Some(std::array::from_fn(|k| {
        let top = px(x0, y0, k) * (1.0 - tx) + px(x1, y0, k) * tx;
        let bottom = px(x0, y1, k) * (1.0 - tx) + px(x1, y1, k) * tx;
        top * (1.0 - ty) + bottom * ty
    }))
}


/// Size and softness of the clone brush.
#[derive(Clone, Copy, Debug)]
pub struct CloneBrush {
    /// Radius in pixels of the satellite image.
    pub radius: f32,
    /// Part of the radius painted at full strength, 0 to 1.
    pub hardness: f32,
    pub opacity: f32,
}

/// An object to bake: where it stands and the height of its origin.
pub struct BakeObject {
    pub object: Obj,
    pub origin_y: f32,
    pub model: String,
}

/// True when `path` is one of the models left out of a bake: it holds one of the entries of
/// `skip`, which are whole model paths or parts of them such as `\wrecks\`. Case and the
/// kind of slash do not matter.
pub fn bake_skipped(skip: &[String], path: &str) -> bool {
    if skip.is_empty() {
        return false;
    }
    let p = path.to_ascii_lowercase().replace('/', "\\");
    skip.iter().any(|s| {
        let s = s.trim().to_ascii_lowercase().replace('/', "\\");
        !s.is_empty() && p.contains(&s)
    })
}

/// Choices for [`bake_objects_with`].
#[derive(Clone, Default)]
pub struct BakeOptions {
    /// Draws the water surfaces of models (rivers, ponds) in the colour of water.
    pub water: bool,
    /// Leaves out what lies under this ground, for objects sunk partly into the terrain.
    pub ground: Option<Arc<crate::terrain::Heightmap>>,
    /// Draws decals ([`crate::objects::is_decal_model`]) with the see-through parts of their
    /// pictures, over everything else that is baked and under anything standing above them.
    pub decals: bool,
}

/// Water seen from above, from the colour of the game's river material lifted by the sky.
const WATER_FROM_ABOVE: [f32; 3] = [0.10, 0.14, 0.15];

/// What a top view needs of a model.
struct BakeMesh {
    mesh: ModelMesh,
    /// Offset added to the mesh, which models from the game keep around their bounding centre.
    center: [f32; 3],
}

fn sample(mesh: &ModelMesh, texture: usize, u: f32, v: f32) -> [f32; 4] {
    let Some(t) = mesh.textures.get(texture) else { return [0.5, 0.5, 0.5, 1.0] };
    let (w, h) = (t.width.max(1) as usize, t.height.max(1) as usize);
    let x = ((u.rem_euclid(1.0) * w as f32) as usize).min(w - 1);
    let y = ((v.rem_euclid(1.0) * h as f32) as usize).min(h - 1);
    let p = (y * w + x) * 4;
    match t.rgba.get(p..p + 4) {
        Some(c) => [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0],
        None => [0.5, 0.5, 0.5, 1.0],
    }
}

/// Colour and cover (0 to 1) of a point of a batch as the material combines its textures, or
/// `None` where the texture is cut away. Only a decal (`overlay`) keeps its see-through parts;
/// other glass and overlays add next to nothing seen from above.
fn surface_color(mesh: &ModelMesh, batch: &model3d::Batch, uv: [f32; 2], uv2: [f32; 2], water: bool, overlay: bool) -> Option<[f32; 4]> {
    let solid = |c: [f32; 3]| [c[0], c[1], c[2], 1.0];
    if overlay {
        let c = batch.texture.map_or([0.5, 0.5, 0.5, 1.0], |t| sample(mesh, t, uv[0], uv[1]));
        // The game lays decals on the ground with the alpha of their picture (`_ca`) as cover,
        // whatever their material says.
        return (c[3] > 0.004).then_some(c);
    }
    surface_rgb(mesh, batch, uv, uv2, water).map(solid)
}

fn surface_rgb(mesh: &ModelMesh, batch: &model3d::Batch, uv: [f32; 2], uv2: [f32; 2], water: bool) -> Option<[f32; 3]> {
    let shading = batch.shading.and_then(|i| mesh.shadings.get(i));
    let slot = |s: &model3d::Shading, k: usize| -> Option<[f32; 4]> {
        let b = if s.xo[k][2] % 2.0 > 0.5 { uv2 } else { uv };
        let (u, v) = (s.xf[k][0] * b[0] + s.xf[k][2] * b[1] + s.xo[k][0], s.xf[k][1] * b[0] + s.xf[k][3] * b[1] + s.xo[k][1]);
        s.maps[k].map(|t| sample(mesh, t, u, v))
    };
    let shaded = |c: [f32; 3], mc: Option<[f32; 4]>, s: &model3d::Shading| -> [f32; 3] {
        let m = mc.unwrap_or([0.5, 0.5, 0.5, 0.0]);
        std::array::from_fn(|k| c[k] * (1.0 + (m[k] * 2.0 - 1.0) * m[3]) * ((s.ambient[k] + s.diffuse[k]) * 0.5).clamp(0.2, 1.6))
    };
    match shading {
        Some(s) if s.kind == ShadingKind::Water => water.then_some(WATER_FROM_ABOVE),
        Some(s) if s.kind == ShadingKind::Multi => {
            let mask = slot(s, 4).unwrap_or([0.0; 4]);
            let weights = [(1.0 - mask[0] - mask[1] - mask[2]).max(0.0), mask[0], mask[1], mask[2]];
            let mut c = [0.0f32; 3];
            for (k, w) in weights.iter().enumerate() {
                let layer = slot(s, k).unwrap_or([0.5, 0.5, 0.5, 1.0]);
                let detail = slot(s, 11 + k).map_or(1.0, |d| if s.xo[11 + k][2] > 1.5 { 1.0 } else { d[0] * 2.0 });
                for i in 0..3 {
                    c[i] += layer[i] * detail * w;
                }
            }
            Some(shaded(c, slot(s, 5), s))
        }
        Some(s) => {
            let c = batch.texture.map_or([0.5, 0.5, 0.5, 1.0], |t| sample(mesh, t, uv[0], uv[1]));
            if c[3] < 0.43 && s.blend == model3d::Blend::Opaque {
                return None;
            }
            if s.blend != model3d::Blend::Opaque {
                // Glass and overlays add next to nothing seen from above.
                return None;
            }
            let macro_map = if matches!(s.kind, ShadingKind::Super | ShadingKind::Tree) { slot(s, 3) } else { None };
            Some(shaded([c[0], c[1], c[2]], macro_map, s))
        }
        None => {
            let c = batch.texture.map_or([0.5, 0.5, 0.5, 1.0], |t| sample(mesh, t, uv[0], uv[1]));
            (c[3] >= 0.43).then_some([c[0], c[1], c[2]])
        }
    }
}

fn load_bake_mesh(db: &ModelDb, model: &str, plant: bool) -> Option<BakeMesh> {
    let file = db.file(model);
    let root: &Path = db.root();
    // Seen from above the simplest LOD of a building is all it takes, and the game bakes its
    // look into that LOD. The last LOD of a plant is a pair of crossed planes, so the one before it.
    match model3d::load_model_lod(&file, root, 256, usize::MAX) {
        Ok(last) => {
            let mesh = if plant && last.lod_count >= 2 { model3d::load_model_lod(&file, root, 256, last.lod_count - 2).unwrap_or(last) } else { last };
            Some(BakeMesh { mesh, center: db.get(model).center })
        }
        Err(_) => {
            // Source models: plain textured triangles.
            let source = dayz_road_tool::mesh3d::read(&file).ok()?;
            let mut mesh = ModelMesh::default();
            for s in &source.surfaces {
                let first_index = mesh.indices.len() as u32;
                for (tri, uv) in s.positions.chunks_exact(3).zip(s.uvs.chunks_exact(3)) {
                    let (a, b): ([f32; 3], [f32; 3]) = (std::array::from_fn(|k| tri[1][k] - tri[0][k]), std::array::from_fn(|k| tri[2][k] - tri[0][k]));
                    let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
                    for k in 0..3 {
                        mesh.indices.push(mesh.vertices.len() as u32);
                        mesh.vertices.push(model3d::Vertex { pos: tri[k], normal: [n[0] / len, n[1] / len, n[2] / len], uv: uv[k], uv2: uv[k] });
                    }
                }
                let mut path = root.join(s.texture.trim_start_matches(['\\', '/']).replace('\\', "/"));
                path.set_extension("paa");
                let texture = std::fs::read(path).ok().and_then(|b| forest_core::paa::decode(&b, 256).ok()).map(|t| {
                    mesh.textures.push(model3d::TextureImage { width: t.width, height: t.height, rgba: t.rgba, raw: None });
                    mesh.texture_names.push(s.texture.to_ascii_lowercase());
                    mesh.textures.len() - 1
                });
                mesh.batches.push(model3d::Batch { first_index, index_count: mesh.indices.len() as u32 - first_index, texture, shading: None });
            }
            (!mesh.indices.is_empty()).then_some(BakeMesh { mesh, center: [0.0; 3] })
        }
    }
}

/// A model seen from straight above: for the cells round it, how far the nearest part of the
/// model is, in the model's own metres. Positions are relative to the object's origin.
#[derive(Clone, Debug)]
pub struct Silhouette {
    pub cell: f32,
    pub x0: f32,
    pub z0: f32,
    pub cols: usize,
    pub rows: usize,
    pub dist: Vec<f32>,
}

impl Silhouette {
    /// Distance to the model from a point; far away outside the grid.
    pub fn distance(&self, x: f32, z: f32) -> f32 {
        let (i, j) = (((x - self.x0) / self.cell).floor(), ((z - self.z0) / self.cell).floor());
        if i < 0.0 || j < 0.0 || i >= self.cols as f32 || j >= self.rows as f32 {
            return f32::MAX;
        }
        self.dist[j as usize * self.cols + i as usize]
    }
    /// Furthest a point of the grid lies from the origin.
    pub fn reach(&self) -> f32 {
        let (x1, z1) = (self.x0 + self.cols as f32 * self.cell, self.z0 + self.rows as f32 * self.cell);
        self.x0.abs().max(x1.abs()).hypot(self.z0.abs().max(z1.abs()))
    }
}

/// An object as it stands, for [`silhouette_placed`]: the object, the height of its origin,
/// and the ground it stands on when only what is above it counts.
pub struct Placed<'a> {
    pub object: &'a Obj,
    pub origin_y: f32,
    pub ground: Option<&'a crate::terrain::Heightmap>,
}

/// The outline of a model from above, with room round it for a margin of `margin` metres.
/// Walls, which are lines from above, count as well as roofs and floors; water does not.
pub fn silhouette(db: &ModelDb, model: &str, plant: bool, margin: f32) -> Option<Silhouette> {
    silhouette_with(db, model, plant, margin, None)
}

/// [`silhouette`] of one object as it stands, turned, tilted and scaled as it is, in metres of
/// the map from its origin (so it is used with no turn and a scale of 1). With a ground, only
/// its parts above it count: a rock sunk half into the ground leaves the outline of what shows
/// of it. A model only turned about its upright axis needs none of this: its outline from
/// [`silhouette`] turns with it.
pub fn silhouette_placed(db: &ModelDb, model: &str, plant: bool, margin: f32, placed: &Placed) -> Option<Silhouette> {
    silhouette_with(db, model, plant, margin, Some(placed))
}

fn silhouette_with(db: &ModelDb, model: &str, plant: bool, margin: f32, placed: Option<&Placed>) -> Option<Silhouette> {
    let bm = load_bake_mesh(db, model, plant)?;
    // Each corner of the model in the frame the outline is drawn in, and its height: its own
    // for the model alone, or on the map (from the object's origin) for a placed object.
    let corners: Vec<[f32; 3]> = bm.mesh.vertices.iter().map(|v| {
        let p = [v.pos[0] + bm.center[0], v.pos[1] + bm.center[1], v.pos[2] + bm.center[2]];
        match placed {
            None => p,
            Some(pl) => {
                let o = pl.object;
                let m = crate::objects::rotation(o.yaw, o.pitch, o.roll, o.scale);
                [m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2], pl.origin_y + m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2], m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2]]
            }
        }
    }).collect();
    let points: Vec<[f32; 2]> = corners.iter().map(|c| [c[0], c[2]]).collect();
    let heights: Vec<f32> = corners.iter().map(|c| c[1]).collect();
    // Whether a point of the outline (x, height, z in that frame) stands above the ground.
    let above = placed.and_then(|pl| pl.ground.map(|g| (pl.object, g))).map(|(o, g)| move |p: [f32; 3]| p[1] > g.sample(o.x + p[0], o.z + p[2]) + 0.02);
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in &points {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if lo[0] > hi[0] {
        return None;
    }
    let pad = margin.max(0.0) + 0.5;
    let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]) + 2.0 * pad;
    let cell = (extent / 768.0).max(0.2);
    let (x0, z0) = (lo[0] - pad, lo[1] - pad);
    let cols = ((hi[0] - lo[0] + 2.0 * pad) / cell).ceil() as usize + 1;
    let rows = ((hi[1] - lo[1] + 2.0 * pad) / cell).ceil() as usize + 1;
    let mut inside = vec![false; cols * rows];
    let to_cell = |p: [f32; 2]| [(p[0] - x0) / cell, (p[1] - z0) / cell];
    let mark = |x: f32, y: f32, inside: &mut Vec<bool>| {
        let (i, j) = (x.floor() as i64, y.floor() as i64);
        if i >= 0 && j >= 0 && (i as usize) < cols && (j as usize) < rows {
            inside[j as usize * cols + i as usize] = true;
        }
    };
    for batch in &bm.mesh.batches {
        if batch.shading.and_then(|i| bm.mesh.shadings.get(i)).is_some_and(|s| s.kind == ShadingKind::Water) {
            continue;
        }
        let range = batch.first_index as usize..(batch.first_index + batch.index_count) as usize;
        let Some(indices) = bm.mesh.indices.get(range) else { continue };
        for tri in indices.chunks_exact(3) {
            let c = [to_cell(points[tri[0] as usize]), to_cell(points[tri[1] as usize]), to_cell(points[tri[2] as usize])];
            let ys = [heights[tri[0] as usize], heights[tri[1] as usize], heights[tri[2] as usize]];
            let model_point = |cx: f32, cy: f32, y: f32| [x0 + cx * cell, y, z0 + cy * cell];
            // The edges, so a wall seen edge on still leaves its line.
            for k in 0..3 {
                let (a, b) = (c[k], c[(k + 1) % 3]);
                let (ya, yb) = (ys[k], ys[(k + 1) % 3]);
                let steps = ((b[0] - a[0]).abs().max((b[1] - a[1]).abs()) * 2.0).ceil().max(1.0) as usize;
                for s in 0..=steps {
                    let t = s as f32 / steps as f32;
                    let (px, py) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
                    if above.as_ref().is_none_or(|f| f(model_point(px, py, ya + (yb - ya) * t))) {
                        mark(px, py, &mut inside);
                    }
                }
            }
            let area = (c[1][0] - c[0][0]) * (c[2][1] - c[0][1]) - (c[1][1] - c[0][1]) * (c[2][0] - c[0][0]);
            if area.abs() < 1e-6 {
                continue;
            }
            let (i0, j0) = (c[0][0].min(c[1][0]).min(c[2][0]).floor().max(0.0) as usize, c[0][1].min(c[1][1]).min(c[2][1]).floor().max(0.0) as usize);
            let (i1, j1) = ((c[0][0].max(c[1][0]).max(c[2][0]).ceil() as usize).min(cols - 1), (c[0][1].max(c[1][1]).max(c[2][1]).ceil() as usize).min(rows - 1));
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (px, py) = (i as f32 + 0.5, j as f32 + 0.5);
                    let b0 = ((c[1][0] - px) * (c[2][1] - py) - (c[1][1] - py) * (c[2][0] - px)) / area;
                    let b1 = ((c[2][0] - px) * (c[0][1] - py) - (c[2][1] - py) * (c[0][0] - px)) / area;
                    if b0 >= 0.0 && b1 >= 0.0 && b0 + b1 <= 1.0 {
                        let y = b0 * ys[0] + b1 * ys[1] + (1.0 - b0 - b1) * ys[2];
                        if above.as_ref().is_none_or(|f| f(model_point(px, py, y))) {
                            inside[j * cols + i] = true;
                        }
                    }
                }
            }
        }
    }
    // Distance to the nearest covered cell, in two sweeps.
    let mut dist: Vec<f32> = inside.iter().map(|v| if *v { 0.0 } else { f32::MAX }).collect();
    let (straight, diagonal) = (cell, cell * std::f32::consts::SQRT_2);
    for j in 0..rows {
        for i in 0..cols {
            let mut d = dist[j * cols + i];
            if i > 0 { d = d.min(dist[j * cols + i - 1] + straight); }
            if j > 0 {
                d = d.min(dist[(j - 1) * cols + i] + straight);
                if i > 0 { d = d.min(dist[(j - 1) * cols + i - 1] + diagonal); }
                if i + 1 < cols { d = d.min(dist[(j - 1) * cols + i + 1] + diagonal); }
            }
            dist[j * cols + i] = d;
        }
    }
    for j in (0..rows).rev() {
        for i in (0..cols).rev() {
            let mut d = dist[j * cols + i];
            if i + 1 < cols { d = d.min(dist[j * cols + i + 1] + straight); }
            if j + 1 < rows {
                d = d.min(dist[(j + 1) * cols + i] + straight);
                if i + 1 < cols { d = d.min(dist[(j + 1) * cols + i + 1] + diagonal); }
                if i > 0 { d = d.min(dist[(j + 1) * cols + i - 1] + diagonal); }
            }
            dist[j * cols + i] = d;
        }
    }
    inside.iter().any(|v| *v).then_some(Silhouette { cell, x0, z0, cols, rows, dist })
}

/// Draws the objects as seen from straight above into tiles of a `width` by `height` image
/// covering a map `size` metres across. Higher surfaces hide lower ones.
pub fn bake_objects(objects: &[BakeObject], db: &ModelDb, width: usize, height: usize, size: f32, is_plant: impl Fn(&str) -> bool, job: &Job) -> HashMap<Key, Tile> {
    bake_objects_with(objects, db, width, height, size, is_plant, &BakeOptions::default(), job)
}

/// [`bake_objects`] with the choices of `opts`.
#[allow(clippy::too_many_arguments)]
pub fn bake_objects_with(objects: &[BakeObject], db: &ModelDb, width: usize, height: usize, size: f32, is_plant: impl Fn(&str) -> bool, opts: &BakeOptions, job: &Job) -> HashMap<Key, Tile> {
    let mut tiles: HashMap<Key, Vec<u8>> = HashMap::new();
    let mut depth: HashMap<Key, Vec<f32>> = HashMap::new();
    let mut meshes: HashMap<&str, Option<BakeMesh>> = HashMap::new();
    let (sx, sy) = (width as f32 / size, height as f32 / size);
    // Light from high in the south-west, as satellite pictures are usually taken.
    let light = {
        let l = [-0.35f32, 0.85, -0.4];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / n, l[1] / n, l[2] / n]
    };
    // Decals go last, laid over what is already drawn under them.
    let decal: Vec<bool> = objects.iter().map(|b| opts.decals && crate::objects::is_decal_model(&b.model, &db.get(&b.model))).collect();
    let order: Vec<usize> = (0..objects.len()).filter(|i| !decal[*i]).chain((0..objects.len()).filter(|i| decal[*i])).collect();
    for (n, &index) in order.iter().enumerate() {
        let b = &objects[index];
        let overlay = decal[index];
        if n % 256 == 0 {
            if job.cancelled() {
                break;
            }
            job.set_progress(n as f32 / objects.len().max(1) as f32);
        }
        let plant = is_plant(&b.model);
        let Some(bm) = meshes.entry(b.model.as_str()).or_insert_with(|| load_bake_mesh(db, &b.model, plant)) else { continue };
        let o = &b.object;
        let m = crate::objects::rotation(o.yaw, o.pitch, o.roll, o.scale);
        let world = |p: [f32; 3]| -> [f32; 3] {
            let p = [p[0] + bm.center[0], p[1] + bm.center[1], p[2] + bm.center[2]];
            [o.x + m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2], b.origin_y + m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2], o.z + m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2]]
        };
        let turned = |n: [f32; 3]| -> [f32; 3] {
            let v = [m[0][0] * n[0] + m[1][0] * n[1] + m[2][0] * n[2], m[0][1] * n[0] + m[1][1] * n[1] + m[2][1] * n[2], m[0][2] * n[0] + m[1][2] * n[1] + m[2][2] * n[2]];
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
            [v[0] / l, v[1] / l, v[2] / l]
        };
        for batch in &bm.mesh.batches {
            let range = batch.first_index as usize..(batch.first_index + batch.index_count) as usize;
            let Some(indices) = bm.mesh.indices.get(range) else { continue };
            for tri in indices.chunks_exact(3) {
                let v = [&bm.mesh.vertices[tri[0] as usize], &bm.mesh.vertices[tri[1] as usize], &bm.mesh.vertices[tri[2] as usize]];
                let w = [world(v[0].pos), world(v[1].pos), world(v[2].pos)];
                // Image coordinates: east to the right, north up.
                let p = [[w[0][0] * sx, height as f32 - w[0][2] * sy], [w[1][0] * sx, height as f32 - w[1][2] * sy], [w[2][0] * sx, height as f32 - w[2][2] * sy]];
                let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
                if area.abs() < 1e-6 {
                    continue;
                }
                let lo = [p[0][0].min(p[1][0]).min(p[2][0]), p[0][1].min(p[1][1]).min(p[2][1])];
                let hi = [p[0][0].max(p[1][0]).max(p[2][0]), p[0][1].max(p[1][1]).max(p[2][1])];
                if hi[0] < 0.0 || hi[1] < 0.0 || lo[0] >= width as f32 || lo[1] >= height as f32 {
                    continue;
                }
                let (x0, y0) = (lo[0].floor().max(0.0) as usize, lo[1].floor().max(0.0) as usize);
                let (x1, y1) = ((hi[0].ceil() as usize).min(width - 1), (hi[1].ceil() as usize).min(height - 1));
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                        let b0 = ((p[1][0] - px) * (p[2][1] - py) - (p[1][1] - py) * (p[2][0] - px)) / area;
                        let b1 = ((p[2][0] - px) * (p[0][1] - py) - (p[2][1] - py) * (p[0][0] - px)) / area;
                        let b2 = 1.0 - b0 - b1;
                        if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                            continue;
                        }
                        let top = b0 * w[0][1] + b1 * w[1][1] + b2 * w[2][1];
                        // A part in the ground cannot be seen from above.
                        if opts.ground.as_ref().is_some_and(|g| top < g.sample(px / sx, (height as f32 - py) / sy) - 0.05) {
                            continue;
                        }
                        let key = ((x / TILE) as u16, (y / TILE) as u16);
                        let at = (y % TILE) * TILE + x % TILE;
                        let z = depth.entry(key).or_insert_with(|| vec![f32::MIN; TILE * TILE]);
                        // The game draws a decal on the surface under it, so only what stands
                        // clearly above it (a car, a roof, a tree) hides it.
                        if if overlay { top + 0.5 < z[at] } else { top <= z[at] } {
                            continue;
                        }
                        let mix2 = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| [b0 * a[0] + b1 * b[0] + b2 * c[0], b0 * a[1] + b1 * b[1] + b2 * c[1]];
                        let Some(c) = surface_color(&bm.mesh, batch, mix2(v[0].uv, v[1].uv, v[2].uv), mix2(v[0].uv2, v[1].uv2, v[2].uv2), opts.water, overlay) else { continue };
                        let n = turned(std::array::from_fn(|k| b0 * v[0].normal[k] + b1 * v[1].normal[k] + b2 * v[2].normal[k]));
                        // Leaves catch the light from both sides.
                        let facing = n[0] * light[0] + n[1] * light[1] + n[2] * light[2];
                        let lit = 0.55 + 0.55 * if plant { facing.abs() } else { facing.max(0.0) };
                        let t = tiles.entry(key).or_insert_with(|| vec![0u8; TILE * TILE * 4]);
                        if overlay {
                            // Over what is drawn here already (its cover in the alpha); the
                            // depth stays, so decals over each other all show.
                            let (a, under) = (c[3], t[at * 4 + 3] as f32 / 255.0);
                            let cover = a + under * (1.0 - a);
                            if cover <= 0.0 {
                                continue;
                            }
                            for k in 0..3 {
                                let old = t[at * 4 + k] as f32 / 255.0;
                                t[at * 4 + k] = (((c[k] * lit) * a + old * under * (1.0 - a)) / cover * 255.0).clamp(0.0, 255.0) as u8;
                            }
                            t[at * 4 + 3] = (cover * 255.0).round() as u8;
                            continue;
                        }
                        z[at] = top;
                        for k in 0..3 {
                            t[at * 4 + k] = (c[k] * lit * 255.0).clamp(0.0, 255.0) as u8;
                        }
                        t[at * 4 + 3] = 255;
                    }
                }
            }
        }
    }
    job.set_progress(1.0);
    tiles.into_iter().map(|(k, t)| (k, Arc::new(t))).collect()
}

/// Visual palette for realistic satellite water painting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WaterStyle {
    /// DayZ Chernarus/Livonia realistic dark coastal blue-teal and deep inland green-teal.
    #[default]
    Natural,
    /// Brighter turquoise coastal shallows and clearer inland rivers/ponds.
    Coastal,
    /// Darker, swampy olive-teal inland water and muted sea.
    Murky,
}

/// Settings for [`bake_water`].
#[derive(Clone, Debug)]
pub struct WaterBakeOptions {
    pub ocean: bool,
    pub rivers_and_ponds: bool,
    pub style: WaterStyle,
    pub shoreline_foam: bool,
    pub above_ground: bool,
    pub sea_level: f32,
}

impl Default for WaterBakeOptions {
    fn default() -> Self {
        Self {
            ocean: true,
            rivers_and_ponds: true,
            style: WaterStyle::Natural,
            shoreline_foam: true,
            above_ground: true,
            sea_level: 0.0,
        }
    }
}

#[inline]
fn hash2(ix: i32, iy: i32) -> f32 {
    let mut n = (ix.wrapping_mul(374761393)) ^ (iy.wrapping_mul(668265263));
    n = (n ^ (n >> 13)).wrapping_mul(1274126177);
    ((n ^ (n >> 16)) as u32 & 0xFFFF) as f32 / 65535.0
}

#[inline]
fn value_noise(x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash2(ix, iy);
    let b = hash2(ix + 1, iy);
    let c = hash2(ix, iy + 1);
    let d = hash2(ix + 1, iy + 1);
    let ab = a + (b - a) * ux;
    let cd = c + (d - c) * ux;
    ab + (cd - ab) * uy
}

/// Subtle multi-octave wind ripple texture in world metres, centred around 0.
#[inline]
fn water_ripples(wx: f32, wz: f32) -> f32 {
    // Slightly anisotropic wind direction (SW -> NE) like satellite imagery over sea/lakes.
    let u = wx * 0.78 + wz * 0.62;
    let v = -wx * 0.62 + wz * 0.78;
    let n1 = value_noise(u * 0.045, v * 0.14);
    let n2 = value_noise(u * 0.13 + 17.3, v * 0.36 - 9.1);
    let n3 = value_noise(wx * 0.31 - 5.2, wz * 0.31 + 11.7);
    (n1 * 0.55 + n2 * 0.30 + n3 * 0.15) - 0.5
}

#[inline]
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn ocean_pixel(wx: f32, wz: f32, depth: f32, style: WaterStyle, foam: bool) -> [u8; 4] {
    let (shallow, mid, deep) = match style {
        WaterStyle::Natural => ([46.0, 82.0, 82.0], [22.0, 56.0, 74.0], [11.0, 28.0, 44.0]),
        WaterStyle::Coastal => ([62.0, 118.0, 118.0], [26.0, 78.0, 102.0], [14.0, 38.0, 62.0]),
        WaterStyle::Murky => ([48.0, 68.0, 56.0], [24.0, 46.0, 48.0], [12.0, 24.0, 32.0]),
    };
    let t1 = (depth / 4.5).clamp(0.0, 1.0);
    let t2 = ((depth - 4.0) / 28.0).clamp(0.0, 1.0);
    let mut rgb = lerp3(lerp3(shallow, mid, t1), deep, t2);
    let rip = water_ripples(wx, wz);
    let rip_amp = (10.0 + 8.0 * (1.0 - t2)) * (depth / 0.8).clamp(0.2, 1.0);
    for ch in &mut rgb {
        *ch += rip * rip_amp;
    }
    if foam && depth > 0.03 && depth < 0.75 {
        let band = 1.0 - ((depth - 0.22).abs() / 0.50).clamp(0.0, 1.0);
        let noise = value_noise(wx * 0.22 + 3.1, wz * 0.22 - 4.7);
        let f = (band * band * (noise * 1.45 - 0.35).clamp(0.0, 1.0)) * 0.55;
        rgb = lerp3(rgb, [196.0, 206.0, 208.0], f);
    }
    let alpha = (depth / 1.35).clamp(0.0, 1.0).powf(0.65);
    [
        rgb[0].clamp(0.0, 255.0) as u8,
        rgb[1].clamp(0.0, 255.0) as u8,
        rgb[2].clamp(0.0, 255.0) as u8,
        (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

fn inland_water_pixel(wx: f32, wz: f32, depth: f32, style: WaterStyle) -> [u8; 4] {
    let (shallow, deep) = match style {
        WaterStyle::Natural => ([48.0, 74.0, 62.0], [20.0, 46.0, 46.0]),
        WaterStyle::Coastal => ([54.0, 94.0, 86.0], [22.0, 62.0, 68.0]),
        WaterStyle::Murky => ([52.0, 64.0, 44.0], [24.0, 38.0, 30.0]),
    };
    let t = (depth / 2.2).clamp(0.0, 1.0);
    let mut rgb = lerp3(shallow, deep, t);
    let rip = water_ripples(wx * 1.6, wz * 1.6);
    for ch in &mut rgb {
        *ch += rip * 11.0;
    }
    let alpha = if depth <= 0.02 { 0.45 } else { (0.35 + 0.61 * (depth / 0.85).clamp(0.0, 1.0)).clamp(0.35, 0.96) };
    [
        rgb[0].clamp(0.0, 255.0) as u8,
        rgb[1].clamp(0.0, 255.0) as u8,
        rgb[2].clamp(0.0, 255.0) as u8,
        (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// Rasterizes the maximum above-ground Y height of solid objects (piers, bridges, rocks, buildings)
/// so realistic water painting leaves anything standing above the water surface untouched.
fn rasterize_occluder_heights(
    occluders: &[BakeObject],
    ground: &crate::terrain::Heightmap,
    db: &ModelDb,
    width: usize,
    height: usize,
    size: f32,
    job: &Job,
) -> HashMap<Key, Vec<f32>> {
    let mut occ: HashMap<Key, Vec<f32>> = HashMap::new();
    let mut meshes: HashMap<&str, Option<BakeMesh>> = HashMap::new();
    let (sx, sy) = (width as f32 / size, height as f32 / size);
    for (n, b) in occluders.iter().enumerate() {
        if n % 128 == 0 && job.cancelled() {
            break;
        }
        let Some(bm) = meshes.entry(b.model.as_str()).or_insert_with(|| load_bake_mesh(db, &b.model, false)) else { continue };
        let o = &b.object;
        let m = crate::objects::rotation(o.yaw, o.pitch, o.roll, o.scale);
        let world = |p: [f32; 3]| -> [f32; 3] {
            let p = [p[0] + bm.center[0], p[1] + bm.center[1], p[2] + bm.center[2]];
            [
                o.x + m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2],
                b.origin_y + m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2],
                o.z + m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2],
            ]
        };
        for batch in &bm.mesh.batches {
            if batch.shading.and_then(|i| bm.mesh.shadings.get(i)).is_some_and(|s| s.kind == ShadingKind::Water) {
                continue;
            }
            let range = batch.first_index as usize..(batch.first_index + batch.index_count) as usize;
            let Some(indices) = bm.mesh.indices.get(range) else { continue };
            for tri in indices.chunks_exact(3) {
                let v = [&bm.mesh.vertices[tri[0] as usize], &bm.mesh.vertices[tri[1] as usize], &bm.mesh.vertices[tri[2] as usize]];
                let w = [world(v[0].pos), world(v[1].pos), world(v[2].pos)];
                let p = [[w[0][0] * sx, height as f32 - w[0][2] * sy], [w[1][0] * sx, height as f32 - w[1][2] * sy], [w[2][0] * sx, height as f32 - w[2][2] * sy]];
                let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
                if area.abs() < 1e-6 {
                    continue;
                }
                let lo = [p[0][0].min(p[1][0]).min(p[2][0]), p[0][1].min(p[1][1]).min(p[2][1])];
                let hi = [p[0][0].max(p[1][0]).max(p[2][0]), p[0][1].max(p[1][1]).max(p[2][1])];
                if hi[0] < 0.0 || hi[1] < 0.0 || lo[0] >= width as f32 || lo[1] >= height as f32 {
                    continue;
                }
                let (x0, y0) = (lo[0].floor().max(0.0) as usize, lo[1].floor().max(0.0) as usize);
                let (x1, y1) = ((hi[0].ceil() as usize).min(width - 1), (hi[1].ceil() as usize).min(height - 1));
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                        let b0 = ((p[1][0] - px) * (p[2][1] - py) - (p[1][1] - py) * (p[2][0] - px)) / area;
                        let b1 = ((p[2][0] - px) * (p[0][1] - py) - (p[2][1] - py) * (p[0][0] - px)) / area;
                        let b2 = 1.0 - b0 - b1;
                        if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                            continue;
                        }
                        let top = b0 * w[0][1] + b1 * w[1][1] + b2 * w[2][1];
                        if top < ground.sample(px / sx, (height as f32 - py) / sy) - 0.05 {
                            continue;
                        }
                        let key = ((x / TILE) as u16, (y / TILE) as u16);
                        let at = (y % TILE) * TILE + x % TILE;
                        let z = occ.entry(key).or_insert_with(|| vec![f32::MIN; TILE * TILE]);
                        if top > z[at] {
                            z[at] = top;
                        }
                    }
                }
            }
        }
    }
    occ
}

/// Paints realistic ocean, river, and pond water into satellite modifier tiles, clipping underground
/// water geometry against the terrain and leaving objects standing above the water untouched.
#[allow(clippy::too_many_arguments)]
pub fn bake_water(
    water_objects: &[BakeObject],
    occluders: &[BakeObject],
    ground: &crate::terrain::Heightmap,
    db: &ModelDb,
    width: usize,
    height: usize,
    size: f32,
    opts: &WaterBakeOptions,
    job: &Job,
) -> HashMap<Key, Tile> {
    use rayon::prelude::*;
    if width == 0 || height == 0 || size <= 0.0 {
        return HashMap::new();
    }
    let (sx, sy) = (width as f32 / size, height as f32 / size);

    // 1. Optional occluder height buffer (piers, bridges, rocks, buildings above water).
    let occ = if !occluders.is_empty() {
        job.stage("Checking objects above water");
        rasterize_occluder_heights(occluders, ground, db, width, height, size, job)
    } else {
        HashMap::new()
    };
    if job.cancelled() {
        return HashMap::new();
    }

    let mut tiles: HashMap<Key, Vec<u8>> = HashMap::new();

    // 2. Ocean / sea below `sea_level`, computed in parallel per tile.
    if opts.ocean {
        job.stage("Painting ocean water");
        let tx_count = width.div_ceil(TILE);
        let ty_count = height.div_ceil(TILE);
        let keys: Vec<Key> = (0..ty_count).flat_map(|ty| (0..tx_count).map(move |tx| (tx as u16, ty as u16))).collect();
        let ocean_tiles: Vec<(Key, Vec<u8>)> = keys
            .par_iter()
            .filter_map(|&key| {
                if job.cancelled() {
                    return None;
                }
                let x0 = key.0 as usize * TILE;
                let y0 = key.1 as usize * TILE;
                let x1 = (x0 + TILE).min(width);
                let y1 = (y0 + TILE).min(height);
                let occ_tile = occ.get(&key);
                let mut buf: Option<Vec<u8>> = None;
                for y in y0..y1 {
                    let wz = (height as f32 - (y as f32 + 0.5)) / sy;
                    let ly = y - y0;
                    for x in x0..x1 {
                        let wx = (x as f32 + 0.5) / sx;
                        let gy = ground.sample(wx, wz);
                        if gy >= opts.sea_level {
                            continue;
                        }
                        let lx = x - x0;
                        let at = ly * TILE + lx;
                        if occ_tile.is_some_and(|o| o[at] > opts.sea_level + 0.05) {
                            continue;
                        }
                        let depth = opts.sea_level - gy;
                        let rgba = ocean_pixel(wx, wz, depth, opts.style, opts.shoreline_foam);
                        if rgba[3] == 0 {
                            continue;
                        }
                        let dst = buf.get_or_insert_with(|| vec![0u8; TILE * TILE * 4]);
                        dst[at * 4..at * 4 + 4].copy_from_slice(&rgba);
                    }
                }
                buf.map(|b| (key, b))
            })
            .collect();
        for (k, b) in ocean_tiles {
            tiles.insert(k, b);
        }
    }

    // 3. Rivers and ponds from placed water models, clipped against terrain and occluders.
    if opts.rivers_and_ponds && !water_objects.is_empty() && !job.cancelled() {
        job.stage("Painting rivers and ponds");
        let mut meshes: HashMap<&str, Option<BakeMesh>> = HashMap::new();
        let mut water_z: HashMap<Key, Vec<f32>> = HashMap::new();
        for (n, b) in water_objects.iter().enumerate() {
            if n % 64 == 0 {
                if job.cancelled() {
                    break;
                }
                job.set_progress(n as f32 / water_objects.len().max(1) as f32);
            }
            let Some(bm) = meshes.entry(b.model.as_str()).or_insert_with(|| load_bake_mesh(db, &b.model, false)) else { continue };
            let o = &b.object;
            let m = crate::objects::rotation(o.yaw, o.pitch, o.roll, o.scale);
            let world = |p: [f32; 3]| -> [f32; 3] {
                let p = [p[0] + bm.center[0], p[1] + bm.center[1], p[2] + bm.center[2]];
                [
                    o.x + m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2],
                    b.origin_y + m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2],
                    o.z + m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2],
                ]
            };
            let has_water_shader = bm.mesh.batches.iter().any(|bt| bt.shading.and_then(|i| bm.mesh.shadings.get(i)).is_some_and(|s| s.kind == ShadingKind::Water));
            for batch in &bm.mesh.batches {
                if has_water_shader && !batch.shading.and_then(|i| bm.mesh.shadings.get(i)).is_some_and(|s| s.kind == ShadingKind::Water) {
                    continue;
                }
                let range = batch.first_index as usize..(batch.first_index + batch.index_count) as usize;
                let Some(indices) = bm.mesh.indices.get(range) else { continue };
                for tri in indices.chunks_exact(3) {
                    let v = [&bm.mesh.vertices[tri[0] as usize], &bm.mesh.vertices[tri[1] as usize], &bm.mesh.vertices[tri[2] as usize]];
                    let w = [world(v[0].pos), world(v[1].pos), world(v[2].pos)];
                    let p = [[w[0][0] * sx, height as f32 - w[0][2] * sy], [w[1][0] * sx, height as f32 - w[1][2] * sy], [w[2][0] * sx, height as f32 - w[2][2] * sy]];
                    let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
                    if area.abs() < 1e-6 {
                        continue;
                    }
                    let lo = [p[0][0].min(p[1][0]).min(p[2][0]), p[0][1].min(p[1][1]).min(p[2][1])];
                    let hi = [p[0][0].max(p[1][0]).max(p[2][0]), p[0][1].max(p[1][1]).max(p[2][1])];
                    if hi[0] < 0.0 || hi[1] < 0.0 || lo[0] >= width as f32 || lo[1] >= height as f32 {
                        continue;
                    }
                    let (x0, y0) = (lo[0].floor().max(0.0) as usize, lo[1].floor().max(0.0) as usize);
                    let (x1, y1) = ((hi[0].ceil() as usize).min(width - 1), (hi[1].ceil() as usize).min(height - 1));
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                            let b0 = ((p[1][0] - px) * (p[2][1] - py) - (p[1][1] - py) * (p[2][0] - px)) / area;
                            let b1 = ((p[2][0] - px) * (p[0][1] - py) - (p[2][1] - py) * (p[0][0] - px)) / area;
                            let b2 = 1.0 - b0 - b1;
                            if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                                continue;
                            }
                            let top = b0 * w[0][1] + b1 * w[1][1] + b2 * w[2][1];
                            let (wx, wz) = (px / sx, (height as f32 - py) / sy);
                            let gy = ground.sample(wx, wz);
                            if opts.above_ground && top < gy - 0.05 {
                                continue;
                            }
                            let key = ((x / TILE) as u16, (y / TILE) as u16);
                            let at = (y % TILE) * TILE + x % TILE;
                            if occ.get(&key).is_some_and(|o| o[at] > top + 0.05) {
                                continue;
                            }
                            let wz_buf = water_z.entry(key).or_insert_with(|| vec![f32::MIN; TILE * TILE]);
                            if top <= wz_buf[at] {
                                continue;
                            }
                            wz_buf[at] = top;
                            let depth = (top - gy).max(0.0);
                            let rgba = inland_water_pixel(wx, wz, depth, opts.style);
                            let dst = tiles.entry(key).or_insert_with(|| vec![0u8; TILE * TILE * 4]);
                            dst[at * 4..at * 4 + 4].copy_from_slice(&rgba);
                        }
                    }
                }
            }
        }
    }

    job.set_progress(1.0);
    tiles.into_iter().map(|(k, t)| (k, Arc::new(t))).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn models_in_the_skip_list_are_left_out_of_a_bake() {
        let skip = vec!["\\wrecks\\".to_string(), "DZ/structures/walls/Wall_Fen2_6.p3d".to_string(), "  ".to_string()];
        assert!(bake_skipped(&skip, "dz\\structures\\wrecks\\vehicles\\a.p3d"));
        assert!(bake_skipped(&skip, "DZ\\Structures\\Walls\\wall_fen2_6.p3d"));
        assert!(!bake_skipped(&skip, "dz\\structures\\walls\\wall_fen2_7.p3d"));
        assert!(!bake_skipped(&[], "dz\\a.p3d"));
    }

    use super::*;
    use crate::world::WorldMeta;

    fn world() -> World {
        let mut w = World::new(WorldMeta::new("t", 64, 4.0), 10.0);
        w.satellite = Some(RgbImage::new(256, 256, [10, 20, 30]));
        w
    }
    fn pixel(w: &World, x: usize, y: usize) -> [u8; 3] {
        let s = w.satellite.as_ref().unwrap();
        s.data[(y * s.width + x) * 3..(y * s.width + x) * 3 + 3].try_into().unwrap()
    }

    #[test]
    fn a_clone_stroke_copies_and_its_removal_restores() {
        let mut w = world();
        // Something to copy from: a red patch.
        {
            let s = w.satellite.as_mut().unwrap();
            for y in 180..200 {
                for x in 180..200 {
                    s.data[(y * 256 + x) * 3..(y * 256 + x) * 3 + 3].copy_from_slice(&[200, 0, 0]);
                }
            }
        }
        let id = w.sat.add(SatKind::Clone, "clone".into());
        let brush = CloneBrush { radius: 6.0, hardness: 1.0, opacity: 1.0 };
        let r = w.sat_clone_dab(id, [130.5, 60.5], &brush, None, [60, 130]).unwrap();
        // The image as imported, for an export without the modifiers.
        let original = w.sat_original();
        assert_ne!(&original.data[(60 * 256 + 130) * 3..(60 * 256 + 130) * 3 + 3], &w.satellite.as_ref().unwrap().data[(60 * 256 + 130) * 3..(60 * 256 + 130) * 3 + 3]);
        assert!(r.x0 <= 130 && r.x1 >= 130);
        assert_eq!(pixel(&w, 130, 60), [200, 0, 0]);
        assert_eq!(pixel(&w, 100, 60), [10, 20, 30]);
        // The stroke spans two tiles; both come back.
        w.sat_set_enabled(id, false);
        assert_eq!(pixel(&w, 130, 60), [10, 20, 30]);
        w.sat_set_enabled(id, true);
        assert_eq!(pixel(&w, 130, 60), [200, 0, 0]);
        w.sat_remove(id);
        assert_eq!(pixel(&w, 130, 60), [10, 20, 30]);
        assert!(w.sat.is_empty());
    }

    #[test]
    fn a_soft_brush_fades_and_another_image_can_be_the_source() {
        let mut w = world();
        let other = RgbImage::new(256, 256, [250, 250, 250]);
        let id = w.sat.add(SatKind::Clone, "clone".into());
        w.sat_clone_dab(id, [50.5, 50.5], &CloneBrush { radius: 10.0, hardness: 0.0, opacity: 1.0 }, Some(&other), [0, 0]);
        let (middle, edge) = (pixel(&w, 50, 50)[0], pixel(&w, 58, 50)[0]);
        assert!(middle > 240, "{middle}");
        assert!(edge > 10 && edge < 120, "{edge}");
    }

    #[test]
    fn neighbouring_brush_modifiers_merge_into_one_that_looks_the_same() {
        let mut w = world();
        let white = RgbImage::new(256, 256, [250, 240, 230]);
        let red = RgbImage::new(256, 256, [200, 0, 0]);
        let soft = CloneBrush { radius: 8.0, hardness: 0.2, opacity: 0.8 };
        let a = w.sat.add(SatKind::Clone, "a".into());
        w.sat_clone_dab(a, [40.5, 40.5], &soft, Some(&white), [0, 0]);
        let b = w.sat.add(SatKind::Clone, "b".into());
        w.sat_clone_dab(b, [46.5, 40.5], &soft, Some(&red), [0, 0]);
        w.sat_set_opacity(b, 180);
        let c = w.sat.add(SatKind::Bake, "c".into());
        let shown = w.satellite.as_ref().unwrap().data.clone();
        assert_eq!(w.sat.merge_target(c), None);
        assert_eq!(w.sat.merge_target(b), Some(a));
        assert_eq!(w.sat_merge_brushes().0, 1);
        assert_eq!(w.sat.modifiers.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["a", "c"]);
        let after = &w.satellite.as_ref().unwrap().data;
        let worst = shown.iter().zip(after).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap();
        assert!(worst <= 2, "merged differs by {worst}");
        w.sat_set_enabled(a, false);
        assert_eq!(pixel(&w, 43, 40), [10, 20, 30]);
    }

    #[test]
    fn later_modifiers_lie_over_earlier_ones_and_layers_survive_saving() {
        let mut w = world();
        let white = RgbImage::new(256, 256, [255, 255, 255]);
        let black = RgbImage::new(256, 256, [0, 0, 0]);
        let brush = CloneBrush { radius: 4.0, hardness: 1.0, opacity: 1.0 };
        let a = w.sat.add(SatKind::Clone, "a".into());
        w.sat_clone_dab(a, [20.5, 20.5], &brush, Some(&white), [0, 0]);
        let b = w.sat.add(SatKind::Bake, "b".into());
        w.sat_clone_dab(b, [20.5, 20.5], &brush, Some(&black), [0, 0]);
        assert_eq!(pixel(&w, 20, 20), [0, 0, 0]);
        let saved = w.sat.to_bytes();
        let back = SatLayers::from_bytes(&saved).unwrap();
        assert_eq!(back.modifiers.len(), 2);
        assert_eq!((back.modifiers[1].kind, back.modifiers[1].name.as_str()), (SatKind::Bake, "b"));
        // Taking the upper one away shows the lower one, not the original.
        w.sat = back;
        w.sat_remove(b);
        assert_eq!(pixel(&w, 20, 20), [255, 255, 255]);
        w.sat_remove(a);
        assert_eq!(pixel(&w, 20, 20), [10, 20, 30]);
    }

    #[test]
    fn a_new_satellite_becomes_the_original_under_the_modifiers() {
        let mut w = world();
        let white = RgbImage::new(256, 256, [255, 255, 255]);
        let id = w.sat.add(SatKind::Clone, "a".into());
        w.sat_clone_dab(id, [20.5, 20.5], &CloneBrush { radius: 4.0, hardness: 1.0, opacity: 1.0 }, Some(&white), [0, 0]);
        w.satellite = Some(RgbImage::new(256, 256, [90, 90, 90]));
        w.sat_image_replaced();
        assert_eq!(pixel(&w, 20, 20), [255, 255, 255]);
        w.sat_remove(id);
        assert_eq!(pixel(&w, 20, 20), [90, 90, 90]);
    }

    #[test]
    fn the_source_can_be_turned_and_scaled() {
        let mut w = world();
        // Red to the left of x = 128, blue to the right.
        let mut src = RgbImage::new(256, 256, [200, 0, 0]);
        for y in 0..256 { for x in 128..256 { src.data[(y * 256 + x) * 3..(y * 256 + x) * 3 + 3].copy_from_slice(&[0, 0, 200]); } }
        let id = w.sat.add(SatKind::Clone, "a".into());
        let brush = CloneBrush { radius: 12.0, hardness: 1.0, opacity: 1.0 };
        // Turned round: what lies left of the brush comes from the right of the source.
        let map = SourceMap { to: [50.0, 50.0], from: [128.0, 128.0], turn: 180.0, scale: 1.0 };
        w.sat_clone_dab_mapped(id, [50.0, 50.0], &brush, Some(&src), &map, false);
        assert_eq!((pixel(&w, 44, 50), pixel(&w, 55, 50)), ([0, 0, 200], [200, 0, 0]));
        // Twice as large: 8 pixels to the right reads only 4 to the right of the source edge.
        let map = SourceMap { to: [150.0, 150.0], from: [124.0, 128.0], turn: 0.0, scale: 2.0 };
        w.sat_clone_dab_mapped(id, [150.0, 150.0], &brush, Some(&src), &map, false);
        assert_eq!((pixel(&w, 156, 150), pixel(&w, 159, 150)), ([200, 0, 0], [0, 0, 200]));
    }

    #[test]
    fn a_matched_copy_takes_the_colour_of_where_it_goes() {
        let mut w = world(); // 10, 20, 30 everywhere
        // Stripes much lighter than the ground: they keep their pattern, not their brightness.
        let mut src = RgbImage::new(256, 256, [230, 230, 230]);
        for y in 0..256 { for x in (0..256).step_by(2) { src.data[(y * 256 + x) * 3..(y * 256 + x) * 3 + 3].copy_from_slice(&[210, 210, 210]); } }
        let id = w.sat.add(SatKind::Clone, "a".into());
        let brush = CloneBrush { radius: 10.0, hardness: 1.0, opacity: 1.0 };
        w.sat_clone_dab_mapped(id, [100.0, 100.0], &brush, Some(&src), &SourceMap::shift([0, 0]), true);
        let (a, b) = (pixel(&w, 100, 100)[1] as i32, pixel(&w, 101, 100)[1] as i32);
        assert!(((a - b).abs() - 20).abs() <= 2 && ((a + b) / 2 - 20).abs() <= 3, "{a} {b}");
    }

    #[test]
    fn the_adjust_brush_changes_what_is_under_it_once_per_stroke() {
        let mut w = world(); // 10, 20, 30
        let id = w.sat.add(SatKind::Adjust, "a".into());
        let brush = CloneBrush { radius: 6.0, hardness: 1.0, opacity: 1.0 };
        let lighter = Adjust { brightness: 0.5, ..Default::default() };
        let mut before = HashMap::new();
        w.sat_adjust_dab(id, [60.5, 60.5], &brush, &lighter, &mut before);
        let once = pixel(&w, 60, 60);
        assert_eq!(once, lighter.apply([10, 20, 30]));
        // Over the same place in the same stroke: no lighter than once.
        w.sat_adjust_dab(id, [61.5, 60.5], &brush, &lighter, &mut before);
        assert_eq!(pixel(&w, 60, 60), once);
        // A new stroke goes on from there.
        w.sat_adjust_dab(id, [60.5, 60.5], &brush, &lighter, &mut HashMap::new());
        assert_eq!(pixel(&w, 60, 60), lighter.apply(once));
        // It is a modifier like the others: saved, and taken away to show the image again.
        let back = SatLayers::from_bytes(&w.sat.to_bytes()).unwrap();
        assert_eq!(back.modifiers[0].kind, SatKind::Adjust);
        w.sat_remove(id);
        assert_eq!(pixel(&w, 60, 60), [10, 20, 30]);
    }

    #[test]
    fn a_correction_works_from_what_is_under_it_and_can_be_changed() {
        let mut w = world(); // 10, 20, 30
        let white = RgbImage::new(256, 256, [200, 200, 200]);
        let clone = w.sat.add(SatKind::Clone, "a".into());
        w.sat_clone_dab(clone, [40.5, 40.5], &CloneBrush { radius: 4.0, hardness: 1.0, opacity: 1.0 }, Some(&white), [0, 0]);
        let darker = Adjust { brightness: -0.5, ..Default::default() };
        let (id, _) = w.sat_add_correction("c".into(), darker, None);
        // Over everything, the clone stroke under it too.
        assert_eq!(pixel(&w, 100, 100), darker.apply([10, 20, 30]));
        assert_eq!(pixel(&w, 40, 40), darker.apply([200, 200, 200]));
        // Changed later: worked out again from what is under it, not from the darkened image.
        let lighter = Adjust { brightness: 0.3, ..Default::default() };
        w.sat_set_correction(id, lighter);
        assert_eq!(pixel(&w, 100, 100), lighter.apply([10, 20, 30]));
        // Saved and read back with its settings; hidden, the image is as it was.
        let back = SatLayers::from_bytes(&w.sat.to_bytes()).unwrap();
        assert_eq!(back.modifiers[1].correction, Some(Correction { adjust: lighter, whole: true }));
        // Without corrections the file keeps the older form.
        let mut plain = w.sat.clone();
        plain.modifiers.retain(|m| m.correction.is_none());
        assert!(plain.to_bytes().starts_with(b"DZSM2"));
        w.sat_set_enabled(id, false);
        assert_eq!(pixel(&w, 100, 100), [10, 20, 30]);
        // In an area only: its mask says where.
        let area = shape_pixels(&[vec![[50.0, 50.0], [70.0, 50.0], [70.0, 70.0], [50.0, 70.0]]], &[], 1.0, 1.0, 256, 256);
        w.sat_add_correction("d".into(), darker, Some(&area));
        assert_eq!(pixel(&w, 60, 60), darker.apply([10, 20, 30]));
        assert_eq!(pixel(&w, 90, 60), [10, 20, 30]);
        let back = SatLayers::from_bytes(&w.sat.to_bytes()).unwrap();
        assert!(back.modifiers[2].tiles.values().all(|t| t.len() == TILE * TILE));
    }

    #[test]
    fn colours_blend_as_their_mode_says() {
        let get = |_: i64, _: i64| [100u8, 150, 200];
        let red = [200u8, 40, 40];
        assert_eq!(PaintOp::Colour(red, BlendMode::Normal).apply(0, 0, &get), red);
        let m = PaintOp::Colour([128, 128, 128], BlendMode::Multiply).apply(0, 0, &get);
        assert!(m.iter().zip([100u8, 150, 200]).all(|(a, b)| (*a as i32 - b as i32 / 2).abs() <= 1), "{m:?}");
        let s = PaintOp::Colour([128, 128, 128], BlendMode::Screen).apply(0, 0, &get);
        assert!(s.iter().zip([100u8, 150, 200]).all(|(a, b)| *a > b), "{s:?}");
        // A tint keeps the lightness of the image and takes the colour's hue.
        let t = PaintOp::Colour(red, BlendMode::Tint).apply(0, 0, &get);
        let lum = |c: [u8; 3]| 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
        assert!((lum(t) - lum([100, 150, 200])).abs() < 2.0 && t[0] > t[1] && t[0] > t[2], "{t:?}");
    }

    #[test]
    fn a_blur_evens_out_a_sharp_edge() {
        // Black to the left of x = 10, white from it.
        let get = |x: i64, _: i64| if x < 10 { [0u8; 3] } else { [255u8; 3] };
        let blur = PaintOp::Adjust(Adjust::default(), 6.0);
        let (near, far) = (blur.apply(10, 5, &get)[0], blur.apply(30, 5, &get)[0]);
        assert!(near > 60 && near < 200, "{near}");
        assert_eq!(far, 255);
    }

    #[test]
    fn shapes_give_their_inside_and_a_band_along_lines() {
        let square = vec![[10.0, 10.0], [50.0, 10.0], [50.0, 50.0], [10.0, 50.0]];
        let px = shape_pixels(&[square], &[], 4.0, 1.0, 100, 100);
        let at = |x, y| px.iter().find(|p| p.0 == x && p.1 == y).map_or(0, |p| p.2);
        assert_eq!(at(30, 30), 255);
        assert!(at(11, 30) > 0 && at(11, 30) < 128, "edge fades: {}", at(11, 30));
        assert_eq!(at(5, 30), 0);
        // A line 10 wide with an edge 4 soft: full in the middle, half way at its nominal
        // edge, nothing 7 off it.
        let line = (vec![[0.0, 80.0], [99.0, 80.0]], 10.0);
        let px = shape_pixels(&[], &[line.clone()], 4.0, 0.5, 100, 100);
        let at = |x, y| px.iter().find(|p| p.0 == x && p.1 == y).map_or(0, |p| p.2);
        assert_eq!(at(50, 80), 127);
        assert!(at(50, 84) > 30 && at(50, 84) < 100, "{}", at(50, 84));
        assert_eq!(at(50, 87), 0);
        // However soft, the middle of a narrow band is at full strength.
        let px = shape_pixels(&[], &[(line.0, 3.0)], 6.0, 1.0, 100, 100);
        assert!(px.iter().any(|p| p.1 == 80 && p.2 >= 250));
    }

    #[test]
    fn adjustments_go_the_way_they_say() {
        let c = [120, 100, 80];
        let lum = |c: [u8; 3]| 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
        let spread = |c: [u8; 3]| c.iter().max().unwrap() - c.iter().min().unwrap();
        assert_eq!(Adjust::default().apply(c), c);
        assert!(lum(Adjust { brightness: -0.3, ..Default::default() }.apply(c)) < lum(c));
        assert!(spread(Adjust { saturation: -1.0, ..Default::default() }.apply(c)) <= 1);
        assert!(spread(Adjust { saturation: 0.5, ..Default::default() }.apply(c)) > spread(c));
        let warm = Adjust { warmth: 1.0, ..Default::default() }.apply(c);
        assert!(warm[0] > c[0] && warm[2] < c[2]);
        let flat = Adjust { contrast: -1.0, ..Default::default() }.apply([20, 240, 20]);
        assert!(flat.iter().all(|v| (*v as i32 - 128).abs() <= 1), "{flat:?}");
        // A full turn of the hue comes back round.
        let round = Adjust { hue: 360.0, ..Default::default() }.apply(c);
        assert!(round.iter().zip(c).all(|(a, b)| (*a as i32 - b as i32).abs() <= 1), "{round:?}");
    }

    #[test]
    fn the_eraser_takes_a_stroke_back() {
        let mut w = world();
        let white = RgbImage::new(256, 256, [255, 255, 255]);
        let id = w.sat.add(SatKind::Clone, "a".into());
        let brush = CloneBrush { radius: 6.0, hardness: 1.0, opacity: 1.0 };
        w.sat_clone_dab(id, [40.5, 40.5], &brush, Some(&white), [0, 0]);
        assert_eq!(pixel(&w, 40, 40), [255, 255, 255]);
        // Half strength: half way back.
        w.sat_erase_dab(id, [40.5, 40.5], &CloneBrush { opacity: 0.5, ..brush });
        assert!((pixel(&w, 40, 40)[0] as i32 - 133).abs() <= 2, "{:?}", pixel(&w, 40, 40));
        w.sat_erase_dab(id, [40.5, 40.5], &brush);
        assert_eq!(pixel(&w, 40, 40), [10, 20, 30]);
        // Nothing painted there: nothing to take back.
        assert!(w.sat_erase_dab(id, [200.5, 200.5], &brush).is_none());
    }

    #[test]
    fn opacity_blends_a_modifier_with_what_is_under_it() {
        let mut w = world();
        let white = RgbImage::new(256, 256, [210, 220, 230]);
        let id = w.sat.add(SatKind::Bake, "a".into());
        w.sat_clone_dab(id, [20.5, 20.5], &CloneBrush { radius: 4.0, hardness: 1.0, opacity: 1.0 }, Some(&white), [0, 0]);
        w.sat_set_opacity(id, 128);
        // Half way between the original 10, 20, 30 and the layer.
        assert_eq!(pixel(&w, 20, 20), [110, 120, 130]);
        w.sat_set_opacity(id, 0);
        assert_eq!(pixel(&w, 20, 20), [10, 20, 30]);
        let back = SatLayers::from_bytes(&w.sat.to_bytes()).unwrap();
        assert_eq!(back.modifiers[0].opacity, 0);
    }

    #[test]
    fn bake_water_paints_ocean_with_depth_and_leaves_dry_land_alone() {
        let mut hm = crate::terrain::Heightmap::new(32, 32, 8.0, 10.0);
        // West half slopes underwater, east half stays dry above sea_level = 5.0.
        for j in 0..32 {
            for i in 0..32 {
                hm.set(i, j, (i as f32 - 12.0) * 1.5);
            }
        }
        let opts = WaterBakeOptions { ocean: true, rivers_and_ponds: false, sea_level: 5.0, ..Default::default() };
        let tiles = bake_water(&[], &[], &hm, &ModelDb::new(Path::new(".")), 256, 256, 256.0, &opts, &Job::default());
        assert!(!tiles.is_empty(), "ocean tiles should be generated");
        // Deep west pixel (x=8, y=128) is painted opaque bluish water.
        let west_tile = tiles.get(&(0, 1)).expect("west tile");
        let w_at = (0 * TILE + 8) * 4;
        assert!(west_tile[w_at + 3] > 200, "deep ocean alpha should be high: {}", west_tile[w_at + 3]);
        assert!(west_tile[w_at + 2] > west_tile[w_at], "deep ocean should be bluer than red: {:?}", &west_tile[w_at..w_at + 4]);
        // Far east tile (x=200 -> tx=1) is completely dry -> no tile or alpha 0.
        if let Some(east_tile) = tiles.get(&(1, 1)) {
            let e_at = (0 * TILE + 100) * 4;
            assert_eq!(east_tile[e_at + 3], 0, "dry land must remain unpainted");
        }
    }
}
