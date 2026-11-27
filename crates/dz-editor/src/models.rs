//! Background loading of models, thumbnails and terrain textures.
use crate::render::{MeshData, IMPOSTOR_SIZE};
use dz_core::model_info::ModelDb;
use forest_core::model3d;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

/// LOD slots held per model.
pub const LOD_NEAR: u8 = 0;
pub const LOD_FAR: u8 = 1;
/// The LOD billboards of distant plants are drawn from.
pub const LOD_BILLBOARD: u8 = 1;

struct Queue<T> {
    items: Mutex<VecDeque<T>>,
    wake: Condvar,
}
impl<T> Queue<T> {
    fn new() -> Self {
        Self { items: Mutex::new(VecDeque::new()), wake: Condvar::new() }
    }
    fn push(&self, v: T) {
        self.items.lock().unwrap().push_back(v);
        self.wake.notify_one();
    }
    fn pop(&self) -> T {
        let mut g = self.items.lock().unwrap();
        loop {
            if let Some(v) = g.pop_front() {
                return v;
            }
            g = self.wake.wait(g).unwrap();
        }
    }
}

struct Job {
    key: (u32, u8),
    path: String,
    generation: u64,
}

pub struct ModelLoader {
    queue: Arc<Queue<Job>>,
    done: Receiver<(u64, Option<MeshData>, (u32, u8))>,
    state: HashSet<(u32, u8)>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    /// Largest side of the textures the workers load.
    texture_limit: Arc<std::sync::atomic::AtomicU32>,
    pending: usize,
}

/// Mip chain of an impostor. Colour is weighted by alpha so empty texels never darken it,
/// and alpha is rescaled per level so foliage keeps its coverage at a distance.
fn impostor_mips(base: Vec<u8>, side: usize) -> Vec<(usize, Vec<u8>)> {
    let coverage = |d: &[u8], scale: f32| d.chunks_exact(4).filter(|p| p[3] as f32 * scale >= 127.5).count() as f32 / (d.len() / 4).max(1) as f32;
    let target = coverage(&base, 1.0);
    let mut levels = vec![(side, base)];
    while levels.last().unwrap().0 > 1 {
        let (s, src) = levels.last().unwrap();
        let (s, half) = (*s, (*s / 2).max(1));
        let mut out = vec![0u8; half * half * 4];
        let ratio = s / half;
        for y in 0..half {
            for x in 0..half {
                let (mut rgb, mut alpha) = ([0.0f32; 3], 0.0f32);
                for dy in 0..ratio {
                    for dx in 0..ratio {
                        let p = ((y * ratio + dy) * s + x * ratio + dx) * 4;
                        let a = src[p + 3] as f32;
                        alpha += a;
                        for c in 0..3 {
                            rgb[c] += src[p + c] as f32 * a;
                        }
                    }
                }
                let q = (y * half + x) * 4;
                for c in 0..3 {
                    out[q + c] = (rgb[c] / alpha.max(1.0)) as u8;
                }
                out[q + 3] = (alpha / (ratio * ratio) as f32) as u8;
            }
        }
        // Search the alpha scale that keeps the area covered at the shader's threshold.
        let (mut lo, mut hi) = (0.5f32, 16.0f32);
        for _ in 0..12 {
            let mid = (lo + hi) * 0.5;
            if coverage(&out, mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        for p in out.chunks_exact_mut(4) {
            p[3] = (p[3] as f32 * hi).min(255.0) as u8;
        }
        levels.push((half, out));
    }
    levels
}

/// Largest side of the textures loaded with LOD `lod`. Each LOD is drawn from twice as far
/// as the one before it, so half the pixels show the same; the card holds the larger ones
/// only for the models near the camera.
pub fn lod_texture_limit(limit: u32, lod: u8) -> u32 {
    (limit >> lod.min(3)).max(limit.min(256))
}

fn from_model3d(mesh: &model3d::ModelMesh, center: [f32; 3], key: (u32, u8), with_impostor: bool) -> MeshData {
    let mut vertices = Vec::with_capacity(mesh.vertices.len() * 10);
    for v in &mesh.vertices {
        vertices.extend_from_slice(&[v.pos[0] + center[0], v.pos[1] + center[1], v.pos[2] + center[2], v.normal[0], v.normal[1], v.normal[2], v.uv[0], v.uv[1], v.uv2[0], v.uv2[1]]);
    }
    let batches = mesh.batches.iter().map(|b| (b.first_index, b.index_count, b.texture, b.shading)).collect();
    let textures = mesh.textures.iter().zip(&mesh.texture_names).map(|(t, n)| (n.clone(), t.width, t.height, t.rgba.clone())).collect();
    let raw = mesh.textures.iter().map(|t| t.raw.clone()).collect();
    let mut impostor_fit = [0.0; 3];
    let impostor = with_impostor.then(|| {
        let (rgba, fit) = model3d::render_impostor(mesh, IMPOSTOR_SIZE as u32);
        impostor_fit = [fit.world_size, fit.center[0] + center[0], fit.center[1] + center[1]];
        impostor_mips(rgba, IMPOSTOR_SIZE as usize)
    });
    MeshData { key, vertices, indices: mesh.indices.clone(), batches, textures, raw, lod_count: mesh.lod_count.clamp(1, 255) as u8, shadings: mesh.shadings.clone(), impostor, impostor_fit, conform: false, water: false, decal: false }
}

/// MLOD models and everything `model3d` refuses go through the road tool's reader.
fn from_mesh3d(file: &Path, root: &Path, cache: &dayz_road_tool::mesh3d::Cache, key: (u32, u8), texture_limit: u32) -> Option<MeshData> {
    let m = cache.mesh(file)?;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut batches = Vec::new();
    let mut textures = Vec::new();
    let mut raw = Vec::new();
    // Source models name their materials; the maps those need are gathered here.
    let mut maps = model3d::ModelMesh::default();
    let mut shading_of: HashMap<String, Option<usize>> = HashMap::new();
    for s in &m.surfaces {
        let first = indices.len() as u32;
        for (tri, uv) in s.positions.chunks_exact(3).zip(s.uvs.chunks_exact(3)) {
            let (a, b) = ([tri[1][0] - tri[0][0], tri[1][1] - tri[0][1], tri[1][2] - tri[0][2]], [tri[2][0] - tri[0][0], tri[2][1] - tri[0][1], tri[2][2] - tri[0][2]]);
            let n = crate::camera::norm(crate::camera::cross(a, b));
            for k in 0..3 {
                indices.push((vertices.len() / 10) as u32);
                vertices.extend_from_slice(&[tri[k][0], tri[k][1], tri[k][2], n[0], n[1], n[2], uv[k][0], uv[k][1], uv[k][0], uv[k][1]]);
            }
        }
        let tex = cache.texture(&s.texture, file).map(|t| {
            textures.push((s.texture.to_ascii_lowercase(), t.width, t.height, t.rgba.clone()));
            // The cache keeps a small copy; the card takes the file's own mipmaps.
            let mut full = root.join(s.texture.trim_start_matches(['\\', '/']).replace('\\', "/"));
            full.set_extension("paa");
            raw.push(std::fs::read(full).ok().and_then(|b| forest_core::paa::read_raw(&b, texture_limit).ok()).map(Arc::new));
            textures.len() - 1
        });
        let shading = *shading_of.entry(s.material.to_ascii_lowercase()).or_insert_with(|| {
            let material = forest_core::rvmat::load(root, &s.material)?;
            model3d::material_shading(&material, root, &mut maps, 512, texture_limit)
        });
        batches.push((first, indices.len() as u32 - first, tex, shading));
    }
    // Shadings index the maps after the colour textures.
    let offset = textures.len();
    for sh in &mut maps.shadings {
        for slot in sh.maps.iter_mut().flatten() {
            *slot += offset;
        }
    }
    for (t, n) in maps.textures.into_iter().zip(maps.texture_names) {
        raw.push(t.raw);
        textures.push((n, t.width, t.height, t.rgba));
    }
    (!indices.is_empty()).then_some(MeshData { key: (key.0, 0), vertices, indices, batches, textures, raw, lod_count: 1, shadings: maps.shadings, impostor: None, impostor_fit: [0.0; 3], conform: false, water: false, decal: false })
}

impl ModelLoader {
    pub fn new(db: ModelDb, ctx: eframe::egui::Context) -> Self {
        let queue: Arc<Queue<Job>> = Arc::new(Queue::new());
        let (tx, done) = channel();
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let texture_limit = Arc::new(std::sync::atomic::AtomicU32::new(2048));
        let cache = dayz_road_tool::mesh3d::Cache::default();
        for _ in 0..std::thread::available_parallelism().map(|n| n.get().clamp(2, 6)).unwrap_or(3) {
            let (q, tx, db, ctx, gens, cache, limit) = (queue.clone(), tx.clone(), db.clone(), ctx.clone(), generation.clone(), cache.clone(), texture_limit.clone());
            std::thread::spawn(move || loop {
                let job = q.pop();
                if job.generation != gens.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send((job.generation, None, job.key));
                    continue;
                }
                let result = load(&db, &job.path, job.key, &cache, lod_texture_limit(limit.load(std::sync::atomic::Ordering::Relaxed), job.key.1));
                let _: Result<(), _> = tx.send((job.generation, result, job.key));
                ctx.request_repaint();
            });
        }
        Self { queue, done, state: HashSet::new(), generation, texture_limit, pending: 0 }
    }
    /// Forget everything; used when another project is opened.
    pub fn reset(&mut self) {
        self.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.state.clear();
        self.pending = 0;
    }
    /// Largest side of the textures of the models loaded from now on.
    pub fn set_texture_limit(&self, pixels: u32) {
        self.texture_limit.store(pixels.max(64), std::sync::atomic::Ordering::Relaxed);
    }
    pub fn request(&mut self, key: (u32, u8), path: &str) {
        if self.state.insert(key) {
            self.pending += 1;
            self.queue.push(Job { key, path: path.to_string(), generation: self.generation.load(std::sync::atomic::Ordering::Relaxed) });
        }
    }
    /// Lets a model let go of by the renderer be asked for again.
    pub fn forget(&mut self, key: (u32, u8)) {
        self.state.remove(&key);
    }
    pub fn poll(&mut self, mut f: impl FnMut(MeshData)) {
        let current = self.generation.load(std::sync::atomic::Ordering::Relaxed);
        while let Ok((generation, mesh, _)) = self.done.try_recv() {
            if generation != current {
                continue;
            }
            self.pending = self.pending.saturating_sub(1);
            if let Some(m) = mesh {
                // A model with fewer LODs answers with the last one it has.
                self.state.insert(m.key);
                f(m);
            }
        }
    }
    pub fn pending(&self) -> usize {
        self.pending
    }
}

/// See [`dz_core::objects::is_decal_model`].
pub fn is_decal_model(path: &str, info: &dz_core::model_info::ModelInfo) -> bool {
    dz_core::objects::is_decal_model(path, info)
}

/// Rivers, ponds and streams; not the beds under them or buildings named after water.
pub fn is_water_model(path: &str) -> bool {
    dz_core::objects::is_water_model(path)
}

fn load(db: &ModelDb, path: &str, key: (u32, u8), cache: &dayz_road_tool::mesh3d::Cache, texture_limit: u32) -> Option<MeshData> {
    let file = db.file(path);
    let info = db.get(path);
    let mut mesh = match model3d::load_model_detail(&file, db.root(), 256, texture_limit, key.1 as usize) {
        Ok(m) => {
            let billboard = m.lod == (LOD_BILLBOARD as usize).min(m.lod_count.saturating_sub(1));
            Some(from_model3d(&m, info.center, (key.0, m.lod as u8), billboard))
        }
        Err(_) => from_mesh3d(&file, db.root(), cache, key, texture_limit),
    }?;
    // Water is drawn by its own shader; the game's flow maps are not colours.
    let water_only = !mesh.batches.is_empty() && mesh.batches.iter().all(|b| b.3.and_then(|i| mesh.shadings.get(i)).is_some_and(|s| s.kind == model3d::ShadingKind::Water));
    if is_water_model(path) || water_only {
        mesh.textures.clear();
        mesh.raw.clear();
        mesh.shadings.clear();
        for b in &mut mesh.batches {
            b.2 = None;
            b.3 = None;
        }
        mesh.impostor = None;
        mesh.water = true;
    }
    // The game fits road parts to the terrain; give them vertices to follow it with.
    if path.contains("\\roads\\parts\\") {
        tessellate(&mut mesh, 2.0);
        mesh.conform = true;
    }
    // Decals are drawn flat on what is under them, with their edges faded out.
    if is_decal_model(path, &info) {
        tessellate(&mut mesh, 2.0);
        mesh.conform = true;
        mesh.decal = true;
        mesh.impostor = None;
    }
    Some(mesh)
}

/// Splits triangles until no edge is longer than `limit` metres.
fn tessellate(mesh: &mut MeshData, limit: f32) {
    let stride = 10;
    let source = std::mem::take(&mut mesh.vertices);
    let vertex = |i: u32| -> [f32; 10] { source[i as usize * stride..i as usize * stride + stride].try_into().unwrap() };
    let mix = |a: &[f32; 10], b: &[f32; 10]| -> [f32; 10] { std::array::from_fn(|k| (a[k] + b[k]) * 0.5) };
    let len2 = |a: &[f32; 10], b: &[f32; 10]| (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>();
    let mut vertices = Vec::with_capacity(source.len() * 8);
    let mut indices = Vec::with_capacity(mesh.indices.len() * 8);
    let mut batches = Vec::with_capacity(mesh.batches.len());
    let limit2 = limit * limit;
    for &(first, count, tex, shading) in &mesh.batches {
        let start = indices.len() as u32;
        for tri in mesh.indices[first as usize..(first + count) as usize].chunks_exact(3) {
            let mut stack = vec![[vertex(tri[0]), vertex(tri[1]), vertex(tri[2])]];
            while let Some([a, b, c]) = stack.pop() {
                let (ab, bc, ca) = (len2(&a, &b), len2(&b, &c), len2(&c, &a));
                let longest = ab.max(bc).max(ca);
                if longest <= limit2 || indices.len() > 3_000_000 {
                    for v in [a, b, c] {
                        indices.push((vertices.len() / stride) as u32);
                        vertices.extend_from_slice(&v);
                    }
                } else if longest == ab {
                    let m = mix(&a, &b);
                    stack.push([a, m, c]);
                    stack.push([m, b, c]);
                } else if longest == bc {
                    let m = mix(&b, &c);
                    stack.push([a, b, m]);
                    stack.push([a, m, c]);
                } else {
                    let m = mix(&c, &a);
                    stack.push([a, b, m]);
                    stack.push([m, b, c]);
                }
            }
        }
        batches.push((start, indices.len() as u32 - start, tex, shading));
    }
    mesh.vertices = vertices;
    mesh.indices = indices;
    mesh.batches = batches;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn water_models_are_told_apart() {
        assert!(is_water_model(r"dz\water_bliss\river\enoch_river_0.p3d"));
        assert!(is_water_model(r"dz\water\ponds\pond_50x50.p3d"));
        assert!(!is_water_model(r"dz\water\streambed\streambed_moss_long_snake.p3d"));
        assert!(!is_water_model(r"dz\structures\industrial\farms\farm_watertower_small.p3d"));
    }
    #[test]
    fn far_lods_load_smaller_textures() {
        assert_eq!(lod_texture_limit(4096, 0), 4096);
        assert_eq!(lod_texture_limit(4096, 1), 2048);
        assert_eq!(lod_texture_limit(4096, 3), 512);
        assert_eq!(lod_texture_limit(4096, 9), 512);
        assert_eq!(lod_texture_limit(512, 3), 256);
        assert_eq!(lod_texture_limit(128, 2), 128);
    }
    #[test]
    fn tessellation_limits_edges_and_keeps_batches() {
        let mut m = MeshData {
            key: (0, 0),
            vertices: [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 10.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0].to_vec(),
            indices: vec![0, 1, 2],
            batches: vec![(0, 3, Some(0), None)],
            textures: Vec::new(), raw: Vec::new(), lod_count: 1, shadings: Vec::new(), impostor: None, impostor_fit: [0.0; 3], conform: false, water: false, decal: false,
        };
        tessellate(&mut m, 2.0);
        assert!(m.indices.len() > 30);
        assert_eq!(m.batches, vec![(0, m.indices.len() as u32, Some(0), None)]);
        for t in m.indices.chunks_exact(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (a, b) = (a as usize * 10, b as usize * 10);
                let d: f32 = (0..3).map(|k| (m.vertices[a + k] - m.vertices[b + k]).powi(2)).sum();
                assert!(d <= 4.0001, "{d}");
            }
        }
        // Texture coordinates are interpolated with the positions.
        for v in m.vertices.chunks_exact(10) {
            assert!((v[6] - v[0] / 10.0).abs() < 1e-5);
        }
    }
}

/// Software rendered previews for the content browser, cached on disk.
pub struct Thumbnails {
    queue: Arc<Queue<(String, u64)>>,
    done: Receiver<(String, Option<(usize, Vec<u8>)>)>,
    cache: HashMap<String, Option<eframe::egui::TextureHandle>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    peek: ThumbPeek,
}

/// A handle to the thumbnails that any panel can keep and ask with a shared borrow: the
/// picture of a model once it is made, asking for it the first time.
#[derive(Clone)]
pub struct ThumbPeek {
    queue: Arc<Queue<(String, u64)>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    ids: Arc<Mutex<HashMap<String, eframe::egui::TextureId>>>,
    asked: Arc<Mutex<HashSet<String>>>,
}

impl ThumbPeek {
    pub fn get(&self, model: &str) -> Option<eframe::egui::TextureId> {
        if let Some(id) = self.ids.lock().unwrap().get(model) {
            return Some(*id);
        }
        if self.asked.lock().unwrap().insert(model.to_string()) {
            self.queue.push((model.to_string(), self.generation.load(std::sync::atomic::Ordering::Relaxed)));
        }
        None
    }
}

fn thumb_path(dir: &Path, model: &str) -> PathBuf {
    let hash = model.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    dir.join(format!("{hash:016x}.png"))
}

const THUMB: u32 = 128;

/// Road parts are kept as editable MLOD files, which the ODOL reader does not take.
pub(crate) fn thumbnail_mesh(file: &Path, cache: &dayz_road_tool::mesh3d::Cache) -> Option<model3d::ModelMesh> {
    let m = cache.mesh(file)?;
    let mut out = model3d::ModelMesh::default();
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for s in &m.surfaces {
        let first_index = out.indices.len() as u32;
        for (tri, uv) in s.positions.chunks_exact(3).zip(s.uvs.chunks_exact(3)) {
            let (a, b) = ([tri[1][0] - tri[0][0], tri[1][1] - tri[0][1], tri[1][2] - tri[0][2]], [tri[2][0] - tri[0][0], tri[2][1] - tri[0][1], tri[2][2] - tri[0][2]]);
            let normal = crate::camera::norm(crate::camera::cross(a, b));
            for k in 0..3 {
                out.indices.push(out.vertices.len() as u32);
                out.vertices.push(model3d::Vertex { pos: tri[k], normal, uv: uv[k], uv2: uv[k] });
                for axis in 0..3 {
                    min[axis] = min[axis].min(tri[k][axis]);
                    max[axis] = max[axis].max(tri[k][axis]);
                }
            }
        }
        let texture = cache.texture(&s.texture, file).map(|t| {
            out.textures.push(model3d::TextureImage { width: t.width, height: t.height, rgba: t.rgba.clone(), raw: None });
            out.texture_names.push(s.texture.to_ascii_lowercase());
            out.textures.len() - 1
        });
        out.batches.push(model3d::Batch { first_index, index_count: out.indices.len() as u32 - first_index, texture, shading: None });
    }
    (out.min, out.max) = (min, max);
    (!out.indices.is_empty()).then_some(out)
}

impl Thumbnails {
    pub fn new(db: ModelDb, dir: PathBuf, ctx: eframe::egui::Context) -> Self {
        let queue: Arc<Queue<(String, u64)>> = Arc::new(Queue::new());
        let (tx, done): (Sender<_>, _) = channel();
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let _ = std::fs::create_dir_all(&dir);
        let sources = dayz_road_tool::mesh3d::Cache::default();
        for _ in 0..3 {
            let (q, tx, db, dir, ctx, gens, sources) = (queue.clone(), tx.clone(), db.clone(), dir.clone(), ctx.clone(), generation.clone(), sources.clone());
            std::thread::spawn(move || loop {
                let (model, gen) = q.pop();
                if gen != gens.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send((model, None));
                    continue;
                }
                let path = thumb_path(&dir, &model);
                let pixels = image::open(&path).ok().map(|i| i.into_rgba8().into_raw()).or_else(|| {
                    let file = db.file(&model);
                    let mesh = model3d::load_model_lod(&file, db.root(), 256, 1).ok().or_else(|| thumbnail_mesh(&file, &sources))?;
                    let px = model3d::render_thumbnail(&mesh, THUMB);
                    let _ = image::save_buffer(&path, &px, THUMB, THUMB, image::ColorType::Rgba8);
                    Some(px)
                });
                let _ = tx.send((model, pixels.map(|p| (THUMB as usize, p))));
                ctx.request_repaint();
            });
        }
        let peek = ThumbPeek { queue: queue.clone(), generation: generation.clone(), ids: Default::default(), asked: Default::default() };
        Self { queue, done, cache: HashMap::new(), generation, peek }
    }
    pub fn poll(&mut self, ctx: &eframe::egui::Context) {
        while let Ok((model, px)) = self.done.try_recv() {
            let tex = px.map(|(s, p)| ctx.load_texture(format!("thumb:{model}"), eframe::egui::ColorImage::from_rgba_unmultiplied([s, s], &p), eframe::egui::TextureOptions::LINEAR));
            if let Some(t) = &tex {
                self.peek.ids.lock().unwrap().insert(model.clone(), t.id());
            }
            self.peek.asked.lock().unwrap().insert(model.clone());
            self.cache.insert(model, tex);
        }
    }
    pub fn get(&mut self, model: &str) -> Option<&eframe::egui::TextureHandle> {
        if !self.cache.contains_key(model) {
            self.cache.insert(model.to_string(), None);
            self.queue.push((model.to_string(), self.generation.load(std::sync::atomic::Ordering::Relaxed)));
        }
        self.cache.get(model).and_then(|t| t.as_ref())
    }
    /// [`Thumbnails::get`] where only a shared borrow is at hand: the picture once it is made.
    /// A handle for panels that cannot borrow the thumbnails themselves.
    pub fn handle(&self) -> ThumbPeek {
        self.peek.clone()
    }
}

/// Decoded detail texture of a surface layer.
pub struct LayerTexture {
    pub index: usize,
    pub rgba: Vec<u8>,
    /// Normal map of the surface, the same size as `rgba`.
    pub normal: Vec<u8>,
    pub average: [u8; 3],
}

/// Normal map a surface's RVMAT names, or the one beside its colour texture, as 512 x 512 RGBA.
fn surface_normal(root: &Path, texture: &str, material: &str) -> Vec<u8> {
    const N: usize = 512;
    let lower = texture.to_ascii_lowercase();
    let beside = || {
        let at = lower.rfind("_co.paa").or_else(|| lower.rfind("_ca.paa"))?;
        Some(format!("{}_nopx.paa", &texture[..at]))
    };
    // Stage 1 of a terrain material is its normal map.
    let named = forest_core::rvmat::load(root, material).and_then(|m| m.stages.get(1).map(|s| s.texture.clone())).filter(|t| !t.trim().is_empty() && !t.starts_with('#'));
    let decoded = named.or_else(beside).and_then(|name| {
        let mut file = root.join(name.trim_start_matches(['\\', '/']).replace('\\', "/"));
        file.set_extension("paa");
        forest_core::paa::decode(&std::fs::read(file).ok()?, N as u32).ok()
    });
    let mut out = Vec::with_capacity(N * N * 4);
    match decoded {
        Some(t) => {
            let (w, h) = (t.width as usize, t.height as usize);
            for y in 0..N {
                for x in 0..N {
                    let p = (y * h / N * w + x * w / N) * 4;
                    out.extend_from_slice(&[t.rgba[p], t.rgba[p + 1], t.rgba[p + 2], 255]);
                }
            }
        }
        None => {
            for _ in 0..N * N {
                out.extend_from_slice(&[128, 128, 255, 255]);
            }
        }
    }
    out
}

/// Puts a height worked out from the surface's normal map into the alpha channel, which is
/// what decides where one surface shows through another.
fn apply_relief(root: &Path, texture: &str, rgba: &mut [u8]) {
    let lower = texture.to_ascii_lowercase();
    let Some(at) = lower.rfind("_co.paa").or_else(|| lower.rfind("_ca.paa")) else { return };
    // Terrain surfaces of the game use _nopx maps, with a height in their alpha channel.
    let read = |suffix: &str| std::fs::read(root.join(format!("{}{suffix}", &texture[..at]).replace('\\', "/"))).ok().and_then(|b| forest_core::paa::decode(&b, 512).ok());
    let Some(t) = read("_nopx.paa").or_else(|| read("_nohq.paa")) else { return };
    const N: usize = 512;
    let (w, h) = (t.width as usize, t.height as usize);
    let at_pixel = |x: usize, y: usize| (y * h / N * w + x * w / N) * 4;
    let alpha: Vec<f32> = (0..N * N).map(|i| t.rgba[at_pixel(i % N, i / N) + 3] as f32).collect();
    let alpha_mean = alpha.iter().sum::<f32>() / alpha.len() as f32;
    let alpha_spread = (alpha.iter().map(|v| (v - alpha_mean).powi(2)).sum::<f32>() / alpha.len() as f32).sqrt();
    let red_mean: u64 = (0..N).map(|i| t.rgba[at_pixel(i, i)] as u64).sum::<u64>() / N as u64;
    if red_mean <= 240 && alpha_spread > 4.0 {
        // Keep the raw height values from `_nopx` without re-centring the mean to 128, so
        // height-based blending matches the game (where taller surfaces like grass naturally
        // creep slightly over lower ones like concrete).
        for (pixel, v) in rgba.chunks_exact_mut(4).zip(&alpha) {
            pixel[3] = (*v).clamp(0.0, 255.0) as u8;
        }
        return;
    }
    // Some normal maps keep X in alpha, with red left full.
    let channel = if red_mean > 240 { 3 } else { 0 };
    let slope = |x: usize, y: usize, c: usize| t.rgba[at_pixel(x, y) + c] as f32 / 127.5 - 1.0;
    // Each direction is summed up with a leak, so only the local relief stays.
    const LEAK: f32 = 0.94;
    let mut relief = vec![0.0f32; N * N];
    for y in 0..N {
        let (mut a, mut b) = (0.0, 0.0);
        for x in 0..N {
            a = a * LEAK + slope(x, y, channel);
            relief[y * N + x] += a;
            let k = N - 1 - x;
            b = b * LEAK - slope(k, y, channel);
            relief[y * N + k] += b;
        }
    }
    for x in 0..N {
        let (mut a, mut b) = (0.0, 0.0);
        for y in 0..N {
            a = a * LEAK + slope(x, y, 1);
            relief[y * N + x] += a;
            let k = N - 1 - y;
            b = b * LEAK - slope(x, k, 1);
            relief[k * N + x] += b;
        }
    }
    let mean = relief.iter().sum::<f32>() / relief.len() as f32;
    let spread = (relief.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / relief.len() as f32).sqrt();
    if spread < 1e-3 {
        return;
    }
    for (pixel, v) in rgba.chunks_exact_mut(4).zip(&relief) {
        pixel[3] = (((v - mean) / (2.5 * spread)).clamp(-1.0, 1.0) * 127.0 + 128.0) as u8;
    }
}

/// Reads the terrain textures of all layers on a worker thread.
pub fn load_layer_textures(root: PathBuf, layers: Vec<(usize, String, String, [u8; 3])>, ctx: eframe::egui::Context) -> Receiver<LayerTexture> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for (index, tex, material, color) in layers {
            let (mut rgba, average, _) = dz_core::satgen::layer_texture(&root, &tex, color, 512);
            apply_relief(&root, &tex, &mut rgba);
            let normal = surface_normal(&root, &tex, &material);
            let _ = tx.send(LayerTexture { index, rgba, normal, average });
            ctx.request_repaint();
        }
    });
    rx
}

/// Decoded RGBA texture loaded from the work drive for Map config preview slots.
#[derive(Clone)]
pub struct DecodedTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Cache key identifying the set of paths resolved from `MapConfig` + work drive.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MapConfigAssetsKey {
    pub work_drive: PathBuf,
    pub mid_detail: String,
    pub terrain_normal: String,
    pub env_texture: String,
    pub atmosphere_texture: String,
    pub outside_satellite: String,
    pub outside_texture: String,
    pub outside_nopx: String,
    pub ocean_nice: String,
    pub ocean_storm: String,
}

impl MapConfigAssetsKey {
    pub fn from_resolved(work_drive: &Path, r: &dz_core::world_preview::ResolvedMapConfig) -> Self {
        Self {
            work_drive: work_drive.to_path_buf(),
            mid_detail: r.mid_detail_texture.clone(),
            terrain_normal: r.terrain_normal_texture.clone(),
            env_texture: r.env_texture.clone(),
            atmosphere_texture: r.atmosphere_texture.clone(),
            outside_satellite: r.outside_satellite.clone(),
            outside_texture: r.outside_texture.clone(),
            outside_nopx: r.outside_nopx.clone(),
            ocean_nice: r.ocean_nice_material.clone(),
            ocean_storm: r.ocean_storm_material.clone(),
        }
    }
}

/// Summary of which Map config textures and materials loaded successfully from the work drive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapConfigAssetStatus {
    pub mid_detail: bool,
    pub terrain_normal: bool,
    pub env_texture: bool,
    pub atmosphere_texture: bool,
    pub outside_satellite: bool,
    pub outside_texture: bool,
    pub outside_nopx: bool,
    pub ocean_nice: bool,
    pub ocean_storm: bool,
}

/// Decoded textures and parsed ocean materials loaded for a `MapConfigAssetsKey`.
pub struct MapConfigAssets {
    pub key: MapConfigAssetsKey,
    pub mid_detail: Option<DecodedTexture>,
    pub terrain_normal: Option<DecodedTexture>,
    pub env_texture: Option<DecodedTexture>,
    pub atmosphere_texture: Option<DecodedTexture>,
    pub outside_satellite: Option<DecodedTexture>,
    pub outside_texture: Option<DecodedTexture>,
    pub outside_nopx: Option<DecodedTexture>,
    pub ocean_nice: Option<dz_core::world_preview::OceanMaterial>,
    pub ocean_storm: Option<dz_core::world_preview::OceanMaterial>,
    pub status: MapConfigAssetStatus,
}

fn decode_paa_file(root: &Path, rel: &str, max_side: u32) -> Option<DecodedTexture> {
    let trimmed = rel.trim().trim_start_matches(['\\', '/']);
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let mut full = root.join(trimmed.replace('\\', "/"));
    if full.extension().is_none() {
        full.set_extension("paa");
    }
    let bytes = std::fs::read(&full).ok()?;
    let decoded = forest_core::paa::decode(&bytes, max_side).ok()?;
    Some(DecodedTexture { width: decoded.width, height: decoded.height, rgba: decoded.rgba })
}

/// Loads all textures and `.emat` ocean materials referenced by `key` on a background thread.
pub fn load_map_config_assets(key: MapConfigAssetsKey, ctx: eframe::egui::Context) -> Receiver<MapConfigAssets> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let root = &key.work_drive;
        let mid_detail = decode_paa_file(root, &key.mid_detail, 512);
        let terrain_normal = decode_paa_file(root, &key.terrain_normal, 1024);
        let env_texture = decode_paa_file(root, &key.env_texture, 512);
        let atmosphere_texture = decode_paa_file(root, &key.atmosphere_texture, 512);
        let outside_satellite = decode_paa_file(root, &key.outside_satellite, 1024);
        let outside_texture = decode_paa_file(root, &key.outside_texture, 512);
        let outside_nopx = decode_paa_file(root, &key.outside_nopx, 512);
        let ocean_nice = dz_core::world_preview::OceanMaterial::load(root, &key.ocean_nice);
        let ocean_storm = if key.ocean_storm.eq_ignore_ascii_case(&key.ocean_nice) {
            ocean_nice
        } else {
            dz_core::world_preview::OceanMaterial::load(root, &key.ocean_storm)
        };
        let status = MapConfigAssetStatus {
            mid_detail: mid_detail.is_some(),
            terrain_normal: terrain_normal.is_some(),
            env_texture: env_texture.is_some(),
            atmosphere_texture: atmosphere_texture.is_some(),
            outside_satellite: outside_satellite.is_some(),
            outside_texture: outside_texture.is_some(),
            outside_nopx: outside_nopx.is_some(),
            ocean_nice: ocean_nice.is_some(),
            ocean_storm: ocean_storm.is_some(),
        };
        let _ = tx.send(MapConfigAssets {
            key,
            mid_detail,
            terrain_normal,
            env_texture,
            atmosphere_texture,
            outside_satellite,
            outside_texture,
            outside_nopx,
            ocean_nice,
            ocean_storm,
            status,
        });
        ctx.request_repaint();
    });
    rx
}

/// Terrain surface textures available on the work drive.
pub fn scan_surfaces(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root.join("dz")) else { return out };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        if !name.starts_with("surfaces") {
            continue;
        }
        let dir = e.path().join("data/terrain");
        let Ok(files) = std::fs::read_dir(&dir) else { continue };
        for f in files.flatten() {
            let file = f.file_name().to_string_lossy().to_ascii_lowercase();
            if file.ends_with("_ca.paa") {
                out.push(format!("dz\\{name}\\data\\terrain\\{file}"));
            }
        }
    }
    out.sort();
    out
}
