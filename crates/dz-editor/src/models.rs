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

fn from_model3d(mesh: &model3d::ModelMesh, center: [f32; 3], key: (u32, u8), with_impostor: bool) -> MeshData {
    let mut vertices = Vec::with_capacity(mesh.vertices.len() * 10);
    for v in &mesh.vertices {
        vertices.extend_from_slice(&[v.pos[0] + center[0], v.pos[1] + center[1], v.pos[2] + center[2], v.normal[0], v.normal[1], v.normal[2], v.uv[0], v.uv[1], v.uv2[0], v.uv2[1]]);
    }
    let batches = mesh.batches.iter().map(|b| (b.first_index, b.index_count, b.texture, b.shading)).collect();
    let textures = mesh.textures.iter().zip(&mesh.texture_names).map(|(t, n)| (n.clone(), t.width, t.height, t.rgba.clone())).collect();
    let mut impostor_fit = [0.0; 3];
    let impostor = with_impostor.then(|| {
        let (rgba, fit) = model3d::render_impostor(mesh, IMPOSTOR_SIZE as u32);
        impostor_fit = [fit.world_size, fit.center[0] + center[0], fit.center[1] + center[1]];
        impostor_mips(rgba, IMPOSTOR_SIZE as usize)
    });
    MeshData { key, vertices, indices: mesh.indices.clone(), batches, textures, shadings: mesh.shadings.clone(), impostor, impostor_fit, conform: false, water: false }
}

/// MLOD models and everything `model3d` refuses go through the road tool's reader.
fn from_mesh3d(file: &Path, cache: &dayz_road_tool::mesh3d::Cache, key: (u32, u8)) -> Option<MeshData> {
    let m = cache.mesh(file)?;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut batches = Vec::new();
    let mut textures = Vec::new();
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
            textures.len() - 1
        });
        batches.push((first, indices.len() as u32 - first, tex, None));
    }
    (!indices.is_empty()).then_some(MeshData { key, vertices, indices, batches, textures, shadings: Vec::new(), impostor: None, impostor_fit: [0.0; 3], conform: false, water: false })
}

impl ModelLoader {
    pub fn new(db: ModelDb, ctx: eframe::egui::Context) -> Self {
        let queue: Arc<Queue<Job>> = Arc::new(Queue::new());
        let (tx, done) = channel();
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let cache = dayz_road_tool::mesh3d::Cache::default();
        for _ in 0..std::thread::available_parallelism().map(|n| n.get().clamp(2, 6)).unwrap_or(3) {
            let (q, tx, db, ctx, gens, cache) = (queue.clone(), tx.clone(), db.clone(), ctx.clone(), generation.clone(), cache.clone());
            std::thread::spawn(move || loop {
                let job = q.pop();
                if job.generation != gens.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send((job.generation, None, job.key));
                    continue;
                }
                let result = load(&db, &job.path, job.key, &cache);
                let _: Result<(), _> = tx.send((job.generation, result, job.key));
                ctx.request_repaint();
            });
        }
        Self { queue, done, state: HashSet::new(), generation, pending: 0 }
    }
    /// Forget everything; used when another project is opened.
    pub fn reset(&mut self) {
        self.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.state.clear();
        self.pending = 0;
    }
    pub fn request(&mut self, key: (u32, u8), path: &str) {
        if self.state.insert(key) {
            self.pending += 1;
            self.queue.push(Job { key, path: path.to_string(), generation: self.generation.load(std::sync::atomic::Ordering::Relaxed) });
        }
    }
    pub fn poll(&mut self, mut f: impl FnMut(MeshData)) {
        let current = self.generation.load(std::sync::atomic::Ordering::Relaxed);
        while let Ok((generation, mesh, _)) = self.done.try_recv() {
            if generation != current {
                continue;
            }
            self.pending = self.pending.saturating_sub(1);
            if let Some(m) = mesh {
                f(m);
            }
        }
    }
    pub fn pending(&self) -> usize {
        self.pending
    }
}

/// Rivers, ponds and streams; not the beds under them or buildings named after water.
pub fn is_water_model(path: &str) -> bool {
    (path.contains("\\water\\") || path.contains("\\water_")) && !path.contains("streambed")
}

fn load(db: &ModelDb, path: &str, key: (u32, u8), cache: &dayz_road_tool::mesh3d::Cache) -> Option<MeshData> {
    let file = db.file(path);
    let info = db.get(path);
    let lod = key.1 as usize;
    let mut mesh = match model3d::load_model_lod(&file, db.root(), 512, lod) {
        Ok(m) => Some(from_model3d(&m, info.center, key, key.1 == LOD_FAR)),
        Err(_) => from_mesh3d(&file, cache, key),
    }?;
    // Water is drawn by its own shader; the game's flow maps are not colours.
    if is_water_model(path) {
        mesh.textures.clear();
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
    fn tessellation_limits_edges_and_keeps_batches() {
        let mut m = MeshData {
            key: (0, 0),
            vertices: [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 10.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0].to_vec(),
            indices: vec![0, 1, 2],
            batches: vec![(0, 3, Some(0), None)],
            textures: Vec::new(), shadings: Vec::new(), impostor: None, impostor_fit: [0.0; 3], conform: false, water: false,
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
}

fn thumb_path(dir: &Path, model: &str) -> PathBuf {
    let hash = model.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    dir.join(format!("{hash:016x}.png"))
}

const THUMB: u32 = 128;

impl Thumbnails {
    pub fn new(db: ModelDb, dir: PathBuf, ctx: eframe::egui::Context) -> Self {
        let queue: Arc<Queue<(String, u64)>> = Arc::new(Queue::new());
        let (tx, done): (Sender<_>, _) = channel();
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let _ = std::fs::create_dir_all(&dir);
        for _ in 0..3 {
            let (q, tx, db, dir, ctx, gens) = (queue.clone(), tx.clone(), db.clone(), dir.clone(), ctx.clone(), generation.clone());
            std::thread::spawn(move || loop {
                let (model, gen) = q.pop();
                if gen != gens.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send((model, None));
                    continue;
                }
                let path = thumb_path(&dir, &model);
                let pixels = image::open(&path).ok().map(|i| i.into_rgba8().into_raw()).or_else(|| {
                    let mesh = model3d::load_model_lod(&db.file(&model), db.root(), 256, 1).ok()?;
                    let px = model3d::render_thumbnail(&mesh, THUMB);
                    let _ = image::save_buffer(&path, &px, THUMB, THUMB, image::ColorType::Rgba8);
                    Some(px)
                });
                let _ = tx.send((model, pixels.map(|p| (THUMB as usize, p))));
                ctx.request_repaint();
            });
        }
        Self { queue, done, cache: HashMap::new(), generation }
    }
    pub fn poll(&mut self, ctx: &eframe::egui::Context) {
        while let Ok((model, px)) = self.done.try_recv() {
            let tex = px.map(|(s, p)| ctx.load_texture(format!("thumb:{model}"), eframe::egui::ColorImage::from_rgba_unmultiplied([s, s], &p), eframe::egui::TextureOptions::LINEAR));
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
}

/// Decoded detail texture of a surface layer.
pub struct LayerTexture {
    pub index: usize,
    pub rgba: Vec<u8>,
    pub average: [u8; 3],
}

/// Reads the terrain textures of all layers on a worker thread.
pub fn load_layer_textures(root: PathBuf, layers: Vec<(usize, String, [u8; 3])>, ctx: eframe::egui::Context) -> Receiver<LayerTexture> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for (index, tex, color) in layers {
            let file = root.join(tex.replace('\\', "/"));
            let decoded = std::fs::read(&file).ok().and_then(|b| forest_core::paa::decode(&b, 512).ok());
            let mut rgba = Vec::with_capacity(512 * 512 * 4);
            let mut sum = [0u64; 3];
            match decoded {
                Some(t) => {
                    let (w, h) = (t.width as usize, t.height as usize);
                    for y in 0..512 {
                        for x in 0..512 {
                            let p = (y * h / 512 * w + x * w / 512) * 4;
                            rgba.extend_from_slice(&[t.rgba[p], t.rgba[p + 1], t.rgba[p + 2], 255]);
                            for c in 0..3 {
                                sum[c] += t.rgba[p + c] as u64;
                            }
                        }
                    }
                }
                None => {
                    for _ in 0..512 * 512 {
                        rgba.extend_from_slice(&[color[0], color[1], color[2], 255]);
                    }
                    for c in 0..3 {
                        sum[c] = color[c] as u64 * 512 * 512;
                    }
                }
            }
            let n = 512 * 512;
            let _ = tx.send(LayerTexture { index, rgba, average: [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8] });
            ctx.request_repaint();
        }
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
