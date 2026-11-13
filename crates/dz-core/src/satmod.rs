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

/// The colour of pixel `t` of a tile with the modifier tiles `layers` (RGBA, with their opacity)
/// laid over the original colour `base`.
pub fn compose_pixel(base: [u8; 3], layers: &[(&Tile, u32)], t: usize) -> [u8; 3] {
    let mut c = [base[0] as u32, base[1] as u32, base[2] as u32];
    for (l, opacity) in layers {
        let a = (l[t * 4 + 3] as u32 * opacity + 127) / 255;
        if a > 0 {
            for k in 0..3 {
                c[k] = (c[k] * (255 - a) + l[t * 4 + k] as u32 * a + 127) / 255;
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
    /// Adds an empty modifier and returns its id.
    pub fn add(&mut self, kind: SatKind, name: String) -> u32 {
        // Random, so that two people working on copies of one map do not make the same id.
        let id = crate::maskmod::free_id(|id| self.modifiers.iter().any(|m| m.id == id));
        self.next_id = self.next_id.max(id.saturating_add(1));
        self.modifiers.push(SatModifier { id, name, kind, enabled: true, opacity: 255, tiles: HashMap::new() });
        id
    }

    /// The layers in the form saved beside a project.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = b"DZSM2".to_vec();
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
            });
            b.push(m.enabled as u8);
            b.push(m.opacity);
            b.extend((m.name.len() as u32).to_le_bytes());
            b.extend(m.name.as_bytes());
            tiles(&mut b, &m.tiles);
        }
        b
    }

    pub fn from_bytes(b: &[u8]) -> anyhow::Result<Self> {
        // The first form of the file had no opacity.
        let with_opacity = b.starts_with(b"DZSM2");
        anyhow::ensure!(with_opacity || b.starts_with(b"DZSM1"), "Not a satellite modifier file");
        let mut r = Bytes { b, p: 5 };
        let next_id = r.int()?;
        let base = r.tiles(3)?;
        let count = r.int()? as usize;
        let mut modifiers = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let id = r.int()?;
            let head = r.take(2)?;
            let (kind, enabled) = (if head[0] == 0 { SatKind::Bake } else { SatKind::Clone }, head[1] != 0);
            let opacity = if with_opacity { r.take(1)?[0] } else { 255 };
            let len = r.int()? as usize;
            let name = String::from_utf8_lossy(r.take(len)?).into_owned();
            modifiers.push(SatModifier { id, name, kind, enabled, opacity, tiles: r.tiles(4)? });
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

    /// Lays the enabled modifiers over the original pixels of the given tiles again.
    pub fn sat_recompose(&mut self, keys: impl IntoIterator<Item = Key>) -> Option<PixelRect> {
        let mut touched: Option<PixelRect> = None;
        for key in keys {
            self.sat_keep(key);
            let Some(base) = self.sat.base.get(&key).cloned() else { continue };
            let Some(img) = &mut self.satellite else { return None };
            let Some(r) = tile_rect(key, img) else { continue };
            let layers: Vec<(&Tile, u32)> = self.sat.modifiers.iter().filter(|m| m.enabled && m.opacity > 0).filter_map(|m| m.tiles.get(&key).map(|t| (t, m.opacity as u32))).collect();
            for y in r.y0..=r.y1 {
                for x in r.x0..=r.x1 {
                    let t = (y - r.y0) * TILE + (x - r.x0);
                    let c = compose_pixel([base[t * 3], base[t * 3 + 1], base[t * 3 + 2]], &layers, t);
                    let o = (y * img.width + x) * 3;
                    img.data[o..o + 3].copy_from_slice(&c);
                }
            }
            touched = Some(touched.map_or(r, |t| t.union(r)));
        }
        touched
    }

    /// Replaces the list of modifiers and returns the pixels that changed.
    pub fn sat_set_modifiers(&mut self, list: &mut Vec<SatModifier>) -> Option<PixelRect> {
        let mut keys = SatLayers::keys_of(&self.sat.modifiers);
        keys.extend(SatLayers::keys_of(list));
        std::mem::swap(list, &mut self.sat.modifiers);
        self.sat_recompose(keys)
    }

    /// Removes a modifier; what it covered shows again.
    pub fn sat_remove(&mut self, id: u32) -> Option<PixelRect> {
        let at = self.sat.position(id)?;
        let gone = self.sat.modifiers.remove(at);
        self.sat_recompose(gone.tiles.keys().copied().collect::<Vec<_>>())
    }

    pub fn sat_set_enabled(&mut self, id: u32, enabled: bool) -> Option<PixelRect> {
        let at = self.sat.position(id)?;
        self.sat.modifiers[at].enabled = enabled;
        let keys: Vec<Key> = self.sat.modifiers[at].tiles.keys().copied().collect();
        self.sat_recompose(keys)
    }

    /// Sets how much of a modifier shows over what is under it.
    pub fn sat_set_opacity(&mut self, id: u32, opacity: u8) -> Option<PixelRect> {
        let at = self.sat.position(id)?;
        self.sat.modifiers[at].opacity = opacity;
        let keys: Vec<Key> = self.sat.modifiers[at].tiles.keys().copied().collect();
        self.sat_recompose(keys)
    }

    /// After another image became the satellite: it is the new original, with the modifiers on it.
    pub fn sat_image_replaced(&mut self) -> Option<PixelRect> {
        self.sat.base.clear();
        let keys = SatLayers::keys_of(&self.sat.modifiers);
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
        let at = self.sat.position(id)?;
        let (w, h) = self.satellite.as_ref().map(|s| (s.width, s.height))?;
        let r = brush.radius.max(0.5);
        let (x0, y0) = ((center[0] - r).floor().max(0.0) as usize, (center[1] - r).floor().max(0.0) as usize);
        let (x1, y1) = (((center[0] + r).ceil() as usize).min(w.saturating_sub(1)), ((center[1] + r).ceil() as usize).min(h.saturating_sub(1)));
        if x0 > x1 || y0 > y1 || center[0] + r < 0.0 || center[1] + r < 0.0 {
            return None;
        }
        // The colours are read before anything is written, so a source under the brush is not smeared.
        let mut dabs: Vec<(usize, usize, [u8; 3], u32)> = Vec::new();
        {
            let img = source.or(self.satellite.as_ref())?;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let d = ((x as f32 + 0.5 - center[0]).powi(2) + (y as f32 + 0.5 - center[1]).powi(2)).sqrt() / r;
                    if d >= 1.0 {
                        continue;
                    }
                    let edge = if brush.hardness >= 0.999 { 1.0 } else { 1.0 - ((d - brush.hardness) / (1.0 - brush.hardness)).clamp(0.0, 1.0) };
                    let a = (brush.opacity.clamp(0.0, 1.0) * edge * edge * (3.0 - 2.0 * edge) * 255.0) as u32;
                    let (sx, sy) = (x as i64 + offset[0], y as i64 + offset[1]);
                    if a == 0 || sx < 0 || sy < 0 || sx >= img.width as i64 || sy >= img.height as i64 {
                        continue;
                    }
                    let o = (sy as usize * img.width + sx as usize) * 3;
                    dabs.push((x, y, [img.data[o], img.data[o + 1], img.data[o + 2]], a));
                }
            }
        }
        let mut keys = BTreeSet::new();
        for (x, y, c, a) in dabs {
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
                    tile[p + k] = ((c[k] as u32 * a + tile[p + k] as u32 * old * (255 - a) / 255) / out).min(255) as u8;
                }
                tile[p + 3] = out.min(255) as u8;
            }
        }
        self.sat_recompose(keys)
    }
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

/// Choices for [`bake_objects_with`].
#[derive(Clone, Default)]
pub struct BakeOptions {
    /// Draws the water surfaces of models (rivers, ponds) in the colour of water.
    pub water: bool,
    /// Leaves out what lies under this ground, for objects sunk partly into the terrain.
    pub ground: Option<Arc<crate::terrain::Heightmap>>,
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

/// Colour of a point of a batch as the material combines its textures, or `None` where the
/// texture is cut away.
fn surface_color(mesh: &ModelMesh, batch: &model3d::Batch, uv: [f32; 2], uv2: [f32; 2], water: bool) -> Option<[f32; 3]> {
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

/// The outline of a model from above, with room round it for a margin of `margin` metres.
/// Walls, which are lines from above, count as well as roofs and floors; water does not.
pub fn silhouette(db: &ModelDb, model: &str, plant: bool, margin: f32) -> Option<Silhouette> {
    let bm = load_bake_mesh(db, model, plant)?;
    let points: Vec<[f32; 2]> = bm.mesh.vertices.iter().map(|v| [v.pos[0] + bm.center[0], v.pos[2] + bm.center[2]]).collect();
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
            // The edges, so a wall seen edge on still leaves its line.
            for k in 0..3 {
                let (a, b) = (c[k], c[(k + 1) % 3]);
                let steps = ((b[0] - a[0]).abs().max((b[1] - a[1]).abs()) * 2.0).ceil().max(1.0) as usize;
                for s in 0..=steps {
                    let t = s as f32 / steps as f32;
                    mark(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, &mut inside);
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
                        inside[j * cols + i] = true;
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
    for (index, b) in objects.iter().enumerate() {
        if index % 256 == 0 {
            if job.cancelled() {
                break;
            }
            job.set_progress(index as f32 / objects.len().max(1) as f32);
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
                        if top <= z[at] {
                            continue;
                        }
                        let mix2 = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| [b0 * a[0] + b1 * b[0] + b2 * c[0], b0 * a[1] + b1 * b[1] + b2 * c[1]];
                        let Some(c) = surface_color(&bm.mesh, batch, mix2(v[0].uv, v[1].uv, v[2].uv), mix2(v[0].uv2, v[1].uv2, v[2].uv2), opts.water) else { continue };
                        let n = turned(std::array::from_fn(|k| b0 * v[0].normal[k] + b1 * v[1].normal[k] + b2 * v[2].normal[k]));
                        // Leaves catch the light from both sides.
                        let facing = n[0] * light[0] + n[1] * light[1] + n[2] * light[2];
                        let lit = 0.55 + 0.55 * if plant { facing.abs() } else { facing.max(0.0) };
                        z[at] = top;
                        let t = tiles.entry(key).or_insert_with(|| vec![0u8; TILE * TILE * 4]);
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

#[cfg(test)]
mod tests {
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
}
