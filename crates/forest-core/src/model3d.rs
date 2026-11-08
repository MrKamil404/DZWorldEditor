//! Siatka 3D modelu ODOL (.p3d) gotowa do podglądu oraz programowy render miniatur.
//!
//! Niezależne od GUI: miniatury powstają w wątkach roboczych i dają się testować
//! bez kontekstu OpenGL.

use std::collections::HashMap;
use std::path::Path;

use crate::{odol, paa};

/// Wierzchołek siatki podglądu (układ modelu: Y w górę).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub uv2: [f32; 2],
}

/// Trójkąty korzystające z jednej tekstury.
#[derive(Clone, Copy, Debug)]
pub struct Batch {
    pub first_index: u32,
    pub index_count: u32,
    /// Indeks w `ModelMesh::textures`; `None` = brak tekstury (kolor zastępczy).
    pub texture: Option<usize>,
    /// Indeks w `ModelMesh::shadings`; materiał Super lub Multi.
    pub shading: Option<usize>,
}

/// Number of texture slots of a shading.
pub const SHADING_SLOTS: usize = 15;

/// Textures of a material as the game combines them.
/// Super: 0 colour (from the face when empty), 1 normal, 2 detail, 3 macro, 4 ambient, 5 specular.
/// Multi: 0-3 colour layers, 4 mask, 5 macro, 6 ambient, 7-10 layer normals, 11-14 layer `dtsmdi`.
#[derive(Clone, Debug, Default)]
pub struct Shading {
    pub multi: bool,
    pub maps: [Option<usize>; SHADING_SLOTS],
    /// aside.x, aside.y, up.x, up.y of each slot's UV transform.
    pub xf: [[f32; 4]; SHADING_SLOTS],
    /// pos.x, pos.y, 1 when the slot uses the second UV set.
    pub xo: [[f32; 3]; SHADING_SLOTS],
    /// Reflectance of the surface seen head on.
    pub reflectance: f32,
}

/// Head-on reflectance of a stage like `#(ai,32,128,1)fresnel(2.79,4.84)` (index, absorption).
fn fresnel_reflectance(name: &str) -> Option<f32> {
    let args = name.to_ascii_lowercase();
    let mut values = args.split("fresnel(").nth(1)?.trim_end_matches(')').split(',').map(|v| v.trim().parse::<f32>().ok());
    let (n, k) = (values.next()??, values.next()??);
    Some(((n - 1.0).powi(2) + k * k) / ((n + 1.0).powi(2) + k * k))
}

#[derive(Clone)]
pub struct TextureImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Default)]
pub struct ModelMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<Batch>,
    pub textures: Vec<TextureImage>,
    /// Name of each texture, so identical ones can be shared on the GPU.
    pub texture_names: Vec<String>,
    pub shadings: Vec<Shading>,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl ModelMesh {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn center(&self) -> [f32; 3] {
        std::array::from_fn(|i| (self.min[i] + self.max[i]) * 0.5)
    }

    /// Promień kuli opisanej na prostopadłościanie otaczającym.
    pub fn radius(&self) -> f32 {
        let d: [f32; 3] = std::array::from_fn(|i| self.max[i] - self.min[i]);
        0.5 * (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }

    /// Wysokość modelu w metrach.
    pub fn height(&self) -> f32 {
        self.max[1] - self.min[1]
    }
}

/// Plik MLOD (źródłowy) ma inny format i nie da się go wczytać jako ODOL.
fn describe_format_error(bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(b"MLOD") {
        Some("Model jest w formacie MLOD — podgląd obsługuje tylko binarne ODOL".into())
    } else {
        None
    }
}

/// Ścieżka tekstury z modelu → plik `.paa` pod `root` (P:\). `None` dla tekstur
/// proceduralnych (`#(argb,...)`) i pustych wpisów.
/// Solid colour of a procedural texture such as `#(argb,8,8,3)color(0.5,0.5,1,1,nohq)`.
fn procedural_color(name: &str) -> Option<TextureImage> {
    let args = name.trim().strip_prefix('#')?.split("color(").nth(1)?;
    let values: Vec<f32> = args.split(',').take(4).filter_map(|v| v.trim().parse().ok()).collect();
    (values.len() == 4).then(|| TextureImage {
        width: 1,
        height: 1,
        rgba: values.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect(),
    })
}

/// Loads a material map once per model.
struct MapLoader<'a> {
    root: &'a Path,
    seen: HashMap<String, Option<usize>>,
}
impl MapLoader<'_> {
    fn load(&mut self, name: &str, limit: u32, mesh: &mut ModelMesh) -> Option<usize> {
        let key = name.to_ascii_lowercase();
        if key.trim().is_empty() {
            return None;
        }
        if let Some(v) = self.seen.get(&key) {
            return *v;
        }
        let image = procedural_color(name).or_else(|| {
            let bytes = std::fs::read(texture_path(self.root, name)?).ok()?;
            let t = paa::decode(&bytes, limit).ok()?;
            Some(TextureImage { width: t.width, height: t.height, rgba: t.rgba })
        });
        let slot = image.map(|i| {
            mesh.textures.push(i);
            mesh.texture_names.push(key.clone());
            mesh.textures.len() - 1
        });
        self.seen.insert(key, slot);
        slot
    }
}

fn stage_is(stage: &odol::Stage, kind: &str) -> bool {
    stage.texture.to_ascii_lowercase().contains(kind)
}

fn build_shading(m: &odol::Material, loader: &mut MapLoader, mesh: &mut ModelMesh, max: u32) -> Option<Shading> {
    let s = &m.stages;
    let multi = s.len() >= 15 && stage_is(&s[4], "_mask");
    let table: &[(usize, usize, u32)] = if multi {
        &[(0, 0, max), (1, 1, max), (2, 2, max), (3, 3, max), (4, 4, 512), (5, 9, 256), (6, 10, 256), (7, 11, 256), (8, 12, 256), (9, 13, 256), (10, 14, 256), (11, 5, 256), (12, 6, 256), (13, 7, 256), (14, 8, 256)]
    } else if s.len() >= 6 && stage_is(&s[1], "nohq") && stage_is(&s[2], "dt") && stage_is(&s[3], "mc") && stage_is(&s[4], "as") && stage_is(&s[5], "smdi") {
        &[(1, 1, 256), (2, 2, 256), (3, 3, 256), (4, 4, 256), (5, 5, 256)]
    } else {
        return None;
    };
    let reflectance = if multi { 0.04 } else { s.get(6).and_then(|f| fresnel_reflectance(&f.texture)).unwrap_or(0.04) };
    let mut out = Shading { multi, reflectance, ..Default::default() };
    for slot in 0..SHADING_SLOTS {
        out.xf[slot] = [1.0, 0.0, 0.0, 1.0];
    }
    for &(slot, stage, limit) in table {
        out.maps[slot] = loader.load(&s[stage].texture, limit, mesh);
        if let Some(t) = m.transforms.get(s[stage].transform as usize) {
            out.xf[slot] = [t.matrix[0], t.matrix[1], t.matrix[3], t.matrix[4]];
            out.xo[slot] = [t.matrix[9], t.matrix[10], (t.source == 5) as u32 as f32];
        }
    }
    Some(out)
}

fn texture_path(root: &Path, name: &str) -> Option<std::path::PathBuf> {
    let name = name.trim().trim_start_matches(['\\', '/']);
    if name.is_empty() || name.starts_with('#') {
        return None;
    }
    let mut path = root.join(name.replace('\\', "/"));
    path.set_extension("paa");
    Some(path)
}

/// Wczytuje najdokładniejszy LOD graficzny modelu ODOL. Tekstury są dekodowane
/// z mipmapy nie większej niż `max_texture` i szukane względem `root` (dysk P:\).
pub fn load_model(path: &Path, root: &Path, max_texture: u32) -> Result<ModelMesh, String> {
    load_model_lod(path, root, max_texture, 0)
}

/// Jak `load_model`, ale z LOD-em graficznym o numerze `lod` (0 = najdokładniejszy).
/// Gdy modelu nie ma tylu LOD-ów, używa najdokładniejszego.
pub fn load_model_lod(
    path: &Path,
    root: &Path,
    max_texture: u32,
    lod: usize,
) -> Result<ModelMesh, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(msg) = describe_format_error(&bytes) {
        return Err(msg);
    }
    let mut lods = odol::read(&bytes, |i, resolution| i == 0 || (i == lod && resolution < 1000.0))
        .map_err(|e| format!("{}: {e:#}", path.display()))?;
    let lod = lods.pop().ok_or("Model nie ma LOD-ów")?;
    if lod.resolution >= 1000.0 {
        return Err("Model nie ma LOD-u graficznego".into());
    }
    build_mesh(&lod, root, max_texture)
}

fn build_mesh(lod: &odol::Lod, root: &Path, max_texture: u32) -> Result<ModelMesh, String> {
    if lod.vertices.is_empty() || lod.faces.is_empty() {
        return Err("LOD modelu jest pusty".into());
    }

    let mut mesh = ModelMesh::default();
    // kolejność tekstur z modelu → miejsce w `mesh.textures`; błąd odczytu = kolor zastępczy
    let mut texture_slots: HashMap<u16, Option<usize>> = HashMap::new();
    for (i, name) in lod.textures.iter().enumerate() {
        let image = texture_path(root, name)
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| paa::decode(&b, max_texture).ok());
        let slot = image
            .map(|t| {
                let mut image = TextureImage { width: t.width, height: t.height, rgba: t.rgba };
                prepare_foliage(&mut image);
                image
            })
            .or_else(|| procedural_color(name))
            .map(|image| {
                mesh.textures.push(image);
                mesh.texture_names.push(name.to_ascii_lowercase());
                mesh.textures.len() - 1
            });
        texture_slots.insert(i as u16, slot);
    }

    // Normalne wygładzone liczone z pól ścian (trójkąty liści są dwustronne,
    // więc w shaderze i tak używamy |n·l|).
    let mut normals = vec![[0.0f32; 3]; lod.vertices.len()];
    let valid = |f: &[u16]| {
        f.len() >= 3 && f.iter().all(|&v| (v as usize) < lod.vertices.len())
    };
    for face in lod.faces.iter().filter(|f| valid(f)) {
        let p = |i: usize| lod.vertices[face[i] as usize];
        let n = cross(sub(p(1), p(0)), sub(p(2), p(0)));
        for &v in face {
            let s = &mut normals[v as usize];
            *s = [s[0] + n[0], s[1] + n[1], s[2] + n[2]];
        }
    }

    mesh.vertices = lod
        .vertices
        .iter()
        .enumerate()
        .map(|(i, &pos)| Vertex {
            pos,
            normal: normalize(normals[i]),
            uv: lod.uvs.get(i).copied().unwrap_or([0.0, 0.0]),
            uv2: lod.uvs2.get(i).or(lod.uvs.get(i)).copied().unwrap_or([0.0, 0.0]),
        })
        .collect();

    // Grupowanie trójkątów po teksturze, z zachowaniem kolejności pierwszego użycia.
    let mut loader = MapLoader { root, seen: HashMap::new() };
    let mut shading_of: HashMap<i32, Option<usize>> = HashMap::new();
    let mut order: Vec<(u16, i32)> = Vec::new();
    let mut groups: HashMap<(u16, i32), Vec<u32>> = HashMap::new();
    for (face_index, face) in lod.faces.iter().enumerate() {
        if !valid(face) {
            continue;
        }
        let key = (lod.face_textures.get(face_index).copied().unwrap_or(u16::MAX), lod.face_materials.get(face_index).copied().unwrap_or(-1));
        let group = groups.entry(key).or_insert_with(|| {
            order.push(key);
            Vec::new()
        });
        for k in 1..face.len() - 1 {
            group.extend([face[0] as u32, face[k] as u32, face[k + 1] as u32]);
        }
    }
    for key in order {
        let group = &groups[&key];
        let shading = match shading_of.get(&key.1) {
            Some(s) => *s,
            None => {
                let built = lod.materials.get(key.1.max(0) as usize).filter(|_| key.1 >= 0).and_then(|m| build_shading(m, &mut loader, &mut mesh, max_texture));
                let index = built.map(|b| {
                    mesh.shadings.push(b);
                    mesh.shadings.len() - 1
                });
                shading_of.insert(key.1, index);
                index
            }
        };
        mesh.batches.push(Batch {
            first_index: mesh.indices.len() as u32,
            index_count: group.len() as u32,
            texture: texture_slots.get(&key.0).copied().flatten(),
            shading,
        });
        mesh.indices.extend_from_slice(group);
    }
    if mesh.indices.is_empty() {
        return Err("LOD modelu nie ma trójkątów".into());
    }

    mesh.min = [f32::MAX; 3];
    mesh.max = [f32::MIN; 3];
    for &i in &mesh.indices {
        let p = mesh.vertices[i as usize].pos;
        for a in 0..3 {
            mesh.min[a] = mesh.min[a].min(p[a]);
            mesh.max[a] = mesh.max[a].max(p[a]);
        }
    }
    Ok(mesh)
}

/// Doprowadza teksturę liści do wyglądu zbliżonego do gry.
///
/// Tekstury liści są ciemne, bo shader `TreeAdv` mnoży je w grze przez `diffuse[]`
/// z rvmat (zwykle około 4). Zamiast czytać materiały wyrównujemy średnią jasność.
/// Dodatkowo przezroczyste piksele przejmują kolor sąsiadów, inaczej filtrowanie
/// tekstury dawałoby czarne obwódki liści.
fn prepare_foliage(t: &mut TextureImage) {
    let opaque: Vec<usize> = (0..t.rgba.len() / 4)
        .filter(|i| t.rgba[i * 4 + 3] >= ALPHA_CUTOFF)
        .collect();
    let total = t.rgba.len() / 4;
    let is_cutout = total > 0 && (total - opaque.len()) * 10 > total;
    if !is_cutout || opaque.is_empty() {
        return;
    }

    let sum: u64 = opaque
        .iter()
        .map(|i| t.rgba[i * 4..i * 4 + 3].iter().map(|&c| u64::from(c)).sum::<u64>())
        .sum();
    let mean = sum as f32 / (opaque.len() * 3) as f32;
    let gain = (FOLIAGE_TARGET_BRIGHTNESS / mean.max(1.0)).clamp(1.0, FOLIAGE_MAX_GAIN);
    for i in &opaque {
        for c in &mut t.rgba[i * 4..i * 4 + 3] {
            *c = (f32::from(*c) * gain).min(255.0) as u8;
        }
    }

    let (w, h) = (t.width as usize, t.height as usize);
    let mut filled: Vec<bool> = t.rgba.chunks_exact(4).map(|p| p[3] >= ALPHA_CUTOFF).collect();
    for _ in 0..BLEED_PASSES {
        let mut updates: Vec<(usize, [u8; 3])> = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if filled[i] {
                    continue;
                }
                let (mut sum, mut n) = ([0u32; 3], 0u32);
                let neighbours = [
                    (x > 0).then(|| i - 1),
                    (x + 1 < w).then_some(i + 1),
                    (y > 0).then(|| i - w),
                    (y + 1 < h).then_some(i + w),
                ];
                for j in neighbours.into_iter().flatten().filter(|&j| filled[j]) {
                    for (s, c) in sum.iter_mut().zip(&t.rgba[j * 4..j * 4 + 3]) {
                        *s += u32::from(*c);
                    }
                    n += 1;
                }
                if n > 0 {
                    updates.push((i, sum.map(|s| (s / n) as u8)));
                }
            }
        }
        for (i, rgb) in updates {
            t.rgba[i * 4..i * 4 + 3].copy_from_slice(&rgb);
            filled[i] = true;
        }
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let len = dot(a, a).sqrt();
    if len > 1e-9 {
        [a[0] / len, a[1] / len, a[2] / len]
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// Kolor rysowany, gdy tekstury nie udało się wczytać.
const FALLBACK_COLOR: [f32; 3] = [0.42, 0.50, 0.34];
/// Piksele o mniejszej alfie są odrzucane (liście).
const ALPHA_CUTOFF: u8 = 110;
/// Renderujemy większy obraz i zmniejszamy go, żeby wygładzić krawędzie liści.
const SUPERSAMPLE: usize = 2;
/// Docelowa średnia jasność kanału (0–255) nieprzezroczystej części tekstury liści.
const FOLIAGE_TARGET_BRIGHTNESS: f32 = 100.0;
const FOLIAGE_MAX_GAIN: f32 = 5.0;
const BLEED_PASSES: usize = 4;

/// Dopasowanie modelu do kadru renderu (w jednostkach modelu).
#[derive(Clone, Copy, Debug)]
pub struct ViewFit {
    /// Długość boku całego obrazu.
    pub world_size: f32,
    /// Punkt modelu (x, y w układzie widoku) leżący w środku obrazu.
    pub center: [f32; 2],
}

/// Renderuje miniaturę RGBA `size`×`size` z przezroczystym tłem: ujęcie 3/4 lekko z góry.
pub fn render_thumbnail(mesh: &ModelMesh, size: u32) -> Vec<u8> {
    render_view(mesh, size, 35f32.to_radians(), 8f32.to_radians()).0
}

/// Renderuje widok z boku do billboardu oraz jego dopasowanie do wymiarów modelu.
pub fn render_impostor(mesh: &ModelMesh, size: u32) -> (Vec<u8>, ViewFit) {
    render_view(mesh, size, 0.0, 0.0)
}

fn render_view(mesh: &ModelMesh, size: u32, yaw: f32, pitch: f32) -> (Vec<u8>, ViewFit) {
    let out_size = size as usize;
    let s = out_size * SUPERSAMPLE;
    let mut color = vec![[0u8; 4]; s * s];
    let mut depth = vec![f32::INFINITY; s * s];

    // Projekcja ortogonalna dopasowana do obrysu modelu.
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let view = |p: [f32; 3]| -> [f32; 3] {
        let x = p[0] * cy + p[2] * sy;
        let z = -p[0] * sy + p[2] * cy;
        [x, p[1] * cp + z * sp, -p[1] * sp + z * cp]
    };
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    let projected: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| view(v.pos)).collect();
    for &i in &mesh.indices {
        let p = projected[i as usize];
        for a in 0..2 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-3);
    let scale = s as f32 * 0.94 / extent;
    let (cx, cy_screen) = ((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5);
    let half = s as f32 * 0.5;
    let screen = |p: [f32; 3]| [half + (p[0] - cx) * scale, half - (p[1] - cy_screen) * scale, p[2]];

    let light = normalize([0.45, 0.8, -0.4]);
    for batch in &mesh.batches {
        let texture = batch.texture.map(|t| &mesh.textures[t]);
        let range = batch.first_index as usize..(batch.first_index + batch.index_count) as usize;
        for tri in mesh.indices[range].chunks_exact(3) {
            let v = [
                &mesh.vertices[tri[0] as usize],
                &mesh.vertices[tri[1] as usize],
                &mesh.vertices[tri[2] as usize],
            ];
            let a = screen(projected[tri[0] as usize]);
            let b = screen(projected[tri[1] as usize]);
            let c = screen(projected[tri[2] as usize]);
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area.abs() < 1e-6 {
                continue;
            }
            let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
            let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
            let x1 = (a[0].max(b[0]).max(c[0]).ceil() as usize).min(s - 1);
            let y1 = (a[1].max(b[1]).max(c[1]).ceil() as usize).min(s - 1);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) / area;
                    let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 * a[2] + w1 * b[2] + w2 * c[2];
                    let o = y * s + x;
                    if z >= depth[o] {
                        continue;
                    }
                    let (albedo, alpha) = match texture {
                        Some(t) => {
                            let u = w0 * v[0].uv[0] + w1 * v[1].uv[0] + w2 * v[2].uv[0];
                            let w = w0 * v[0].uv[1] + w1 * v[1].uv[1] + w2 * v[2].uv[1];
                            let tx = (u.rem_euclid(1.0) * t.width as f32) as usize;
                            let ty = (w.rem_euclid(1.0) * t.height as f32) as usize;
                            let i = (ty.min(t.height as usize - 1) * t.width as usize
                                + tx.min(t.width as usize - 1))
                                * 4;
                            let p = &t.rgba[i..i + 4];
                            (
                                [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0],
                                p[3],
                            )
                        }
                        None => (FALLBACK_COLOR, 255),
                    };
                    if alpha < ALPHA_CUTOFF {
                        continue;
                    }
                    depth[o] = z;
                    let n = normalize([
                        w0 * v[0].normal[0] + w1 * v[1].normal[0] + w2 * v[2].normal[0],
                        w0 * v[0].normal[1] + w1 * v[1].normal[1] + w2 * v[2].normal[1],
                        w0 * v[0].normal[2] + w1 * v[1].normal[2] + w2 * v[2].normal[2],
                    ]);
                    let shade = 0.55 + 0.6 * dot(n, light).abs();
                    color[o] = [
                        (albedo[0] * shade * 255.0).min(255.0) as u8,
                        (albedo[1] * shade * 255.0).min(255.0) as u8,
                        (albedo[2] * shade * 255.0).min(255.0) as u8,
                        255,
                    ];
                }
            }
        }
    }

    // Zmniejszanie z wagą alfy, żeby przezroczyste tło nie przyciemniało krawędzi.
    let mut rgba = vec![0u8; out_size * out_size * 4];
    for y in 0..out_size {
        for x in 0..out_size {
            let mut sum = [0u32; 3];
            let mut covered = 0u32;
            for dy in 0..SUPERSAMPLE {
                for dx in 0..SUPERSAMPLE {
                    let c = color[(y * SUPERSAMPLE + dy) * s + x * SUPERSAMPLE + dx];
                    if c[3] > 0 {
                        covered += 1;
                        for i in 0..3 {
                            sum[i] += u32::from(c[i]);
                        }
                    }
                }
            }
            if covered > 0 {
                let o = (y * out_size + x) * 4;
                for i in 0..3 {
                    rgba[o + i] = (sum[i] / covered) as u8;
                }
                rgba[o + 3] = (covered * 255 / (SUPERSAMPLE * SUPERSAMPLE) as u32) as u8;
            }
        }
    }
    (rgba, ViewFit { world_size: s as f32 / scale, center: [cx, cy_screen] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad_mesh() -> ModelMesh {
        let vertex = |x: f32, y: f32| Vertex {
            pos: [x, y, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
            uv2: [0.0, 0.0],
        };
        ModelMesh {
            vertices: vec![vertex(-1.0, 0.0), vertex(1.0, 0.0), vertex(1.0, 4.0), vertex(-1.0, 4.0)],
            indices: vec![0, 1, 2, 0, 2, 3],
            batches: vec![Batch { first_index: 0, index_count: 6, texture: None, shading: None }],
            texture_names: Vec::new(),
            shadings: Vec::new(),
            textures: Vec::new(),
            min: [-1.0, 0.0, 0.0],
            max: [1.0, 4.0, 0.0],
        }
    }

    #[test]
    fn thumbnail_fits_model_on_transparent_background() {
        let rgba = render_thumbnail(&quad_mesh(), 64);
        assert_eq!(rgba.len(), 64 * 64 * 4);
        let alpha = |x: usize, y: usize| rgba[(y * 64 + x) * 4 + 3];
        assert_eq!(alpha(32, 32), 255, "środek modelu");
        assert_eq!(alpha(1, 32), 0, "tło obok wąskiego modelu");
        assert!(alpha(32, 4) > 0, "model wypełnia wysokość kadru");
    }

    #[test]
    fn impostor_reports_how_the_model_fits_the_image() {
        let mesh = quad_mesh(); // x -1..1, y 0..4
        let (rgba, fit) = render_impostor(&mesh, 64);
        assert_eq!(rgba.len(), 64 * 64 * 4);
        assert!((fit.world_size - 4.0 / 0.94).abs() < 1e-3, "{fit:?}");
        assert!((fit.center[1] - 2.0).abs() < 1e-3, "{fit:?}");
    }

    #[test]
    fn foliage_is_brightened_and_bleeds_into_transparent_pixels() {
        // 2x1: ciemny liść + przezroczysty czarny piksel
        let mut t = TextureImage { width: 2, height: 1, rgba: vec![25, 25, 25, 255, 0, 0, 0, 0] };
        prepare_foliage(&mut t);
        assert_eq!(&t.rgba[..4], &[100, 100, 100, 255]);
        assert_eq!(&t.rgba[4..], &[100, 100, 100, 0], "alfa bez zmian, kolor od sąsiada");
    }

    #[test]
    fn opaque_textures_are_left_alone() {
        let mut t = TextureImage { width: 1, height: 1, rgba: vec![25, 25, 25, 255] };
        prepare_foliage(&mut t);
        assert_eq!(t.rgba, vec![25, 25, 25, 255]);
    }

    #[test]
    fn fresnel_stage_gives_reflectance() {
        let f = fresnel_reflectance("#(ai,32,128,1)fresnel(2.79,4.84)").unwrap();
        assert!((f - 0.705).abs() < 0.01, "{f}");
        assert!(fresnel_reflectance("a_co.paa").is_none());
    }

    #[test]
    fn procedural_colors_become_single_pixels() {
        let t = procedural_color("#(argb,8,8,3)color(0.5,0,1,1,nohq)").unwrap();
        assert_eq!((t.width, t.height), (1, 1));
        assert_eq!(t.rgba, vec![128, 0, 255, 255]);
        assert!(procedural_color("#(ai,32,128,1)fresnel(2.79,4.84)").is_none());
        assert!(procedural_color("dz\\a_co.paa").is_none());
    }

    /// A building whose walls are a Multi material when P: is mounted.
    #[test]
    fn multi_material_keeps_layers_mask_and_tiling() {
        let path = Path::new("P:/dz/structures/walls/wall_cyel_end2.p3d");
        if !path.exists() {
            return;
        }
        let mesh = load_model(path, Path::new("P:/"), 256).unwrap();
        let multi = mesh.shadings.iter().find(|s| s.multi).expect("Multi shading");
        for slot in (0..5).chain(11..15) {
            assert!(multi.maps[slot].is_some(), "slot {slot}");
        }
        assert_eq!(multi.xf[2][0], 1.5, "third layer repeats");
        assert_eq!(multi.xo[4][2], 1.0, "mask uses the second UV set");
        assert!(mesh.batches.iter().any(|b| b.shading.is_some()));
    }

    #[test]
    fn texture_paths_map_to_paa_under_root() {
        let root = Path::new("P:/");
        let p = texture_path(root, "dz\\plants\\tree\\data\\leaf_ca.tga").unwrap();
        assert!(p.ends_with("dz/plants/tree/data/leaf_ca.paa"));
        assert!(texture_path(root, "#(argb,8,8,3)color(1,1,1,1)").is_none());
        assert!(texture_path(root, "  ").is_none());
    }

    #[test]
    fn mlod_is_reported_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.p3d");
        std::fs::write(&path, b"MLOD\0\0\0\0").unwrap();
        let err = load_model(&path, dir.path(), 64).err().unwrap();
        assert!(err.contains("MLOD"), "{err}");
    }

    /// Prawdziwe drzewo, gdy dysk P: jest zamontowany.
    #[test]
    fn loads_real_dayz_tree() {
        let path = Path::new("P:/DZ/plants/tree/t_PiceaAbies_2f.p3d");
        if !path.exists() {
            return;
        }
        let mesh = load_model(path, Path::new("P:/"), 128).unwrap();
        assert!(mesh.triangles() > 100, "tris={}", mesh.triangles());
        assert!(mesh.height() > 3.0, "h={}", mesh.height());
        assert!(!mesh.textures.is_empty());
    }
}
