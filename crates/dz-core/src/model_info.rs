//! Placement data of P3D models: bounding box and the bounding centre WRP positions use.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The `placement` named property of a model: how the game fits it to the terrain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    #[default]
    None,
    Slope,
    SlopeX,
    SlopeZ,
    SlopeLandContact,
    LandContact,
    Vertical,
}

impl Placement {
    /// The game shears these models so they follow the ground.
    pub fn skews(self) -> bool {
        matches!(self, Self::Slope | Self::SlopeX | Self::SlopeZ | Self::SlopeLandContact)
    }

    pub fn find(file: &[u8]) -> Self {
        let Some(value) = named_property(file, b"placement") else { return Self::None };
        match value.as_slice() {
            b"slope" => Self::Slope,
            b"slopex" => Self::SlopeX,
            b"slopez" => Self::SlopeZ,
            b"slopelandcontact" => Self::SlopeLandContact,
            b"landcontact" => Self::LandContact,
            b"vertical" => Self::Vertical,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModelInfo {
    /// Offset of the mesh origin from the model's ground origin.
    pub center: [f32; 3],
    /// Box relative to the ground origin.
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// False when the file was missing or unreadable.
    pub known: bool,
    pub placement: Placement,
    /// (x, z) of up to three points of the LandContact LOD, measured like `min` and `max`.
    pub contact: [[f32; 2]; 3],
    pub contact_count: u8,
    /// Height of those points above the ground origin.
    pub contact_y: f32,
}

impl ModelInfo {
    pub fn footprint(&self) -> (f32, f32) {
        (((self.max[0] - self.min[0]) * 0.5).max(0.25), ((self.max[2] - self.min[2]) * 0.5).max(0.25))
    }
}

fn f(b: &[u8], p: usize) -> Option<f32> {
    Some(f32::from_le_bytes(b.get(p..p + 4)?.try_into().ok()?))
}

/// Reads the fields of an ODOL header. `b` needs only the start of the file.
pub fn parse_odol_header(b: &[u8]) -> Option<ModelInfo> {
    if !b.starts_with(b"ODOL") {
        return None;
    }
    let n = u32::from_le_bytes(b.get(8..12)?.try_into().ok()?) as usize;
    if !(1..=64).contains(&n) {
        return None;
    }
    // After the resolution table: 84 bytes of model info, then the visual box and centre.
    let center_at = 12 + n * 4 + 96;
    let center = [f(b, center_at)?, f(b, center_at + 4)?, f(b, center_at + 8)?];
    let mn = [f(b, center_at - 24)?, f(b, center_at - 20)?, f(b, center_at - 16)?];
    let mx = [f(b, center_at - 12)?, f(b, center_at - 8)?, f(b, center_at - 4)?];
    let sane = |v: f32| v.is_finite() && v.abs() < 20000.0;
    if !center.iter().chain(&mn).chain(&mx).copied().all(sane) || mn[1] > mx[1] {
        return None;
    }
    Some(ModelInfo {
        center,
        min: [mn[0] + center[0], mn[1] + center[1], mn[2] + center[2]],
        max: [mx[0] + center[0], mx[1] + center[1], mx[2] + center[2]],
        known: true,
        ..Default::default()
    })
}

fn from_mesh(vertices: impl Iterator<Item = [f32; 3]>) -> Option<ModelInfo> {
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut any = false;
    for v in vertices {
        any = true;
        for c in 0..3 {
            mn[c] = mn[c].min(v[c]);
            mx[c] = mx[c].max(v[c]);
        }
    }
    any.then_some(ModelInfo { center: [0.0; 3], min: mn, max: mx, known: true, ..Default::default() })
}

/// Vertices of the first LOD of an MLOD file.
fn parse_mlod(b: &[u8]) -> Option<ModelInfo> {
    if !b.starts_with(b"MLOD") {
        return None;
    }
    // MLOD: sig, version, lod count; first LOD: P3DM, 2 versions, nPoints, nNormals, nFaces, flags.
    if b.get(12..16)? != b"P3DM" {
        return None;
    }
    let count = u32::from_le_bytes(b.get(24..28)?.try_into().ok()?) as usize;
    let start = 40;
    b.get(start..start + count * 16)?;
    let mut info = from_mesh((0..count).map(|i| {
        let p = start + i * 16;
        [f(b, p).unwrap_or(0.0), f(b, p + 4).unwrap_or(0.0), f(b, p + 8).unwrap_or(0.0)]
    }))?;
    // The centre the game will use once the model is binarised: none with `autocenter=0`,
    // otherwise the middle of the box around all LODs.
    let (mn, mx) = mlod_bounds(b).unwrap_or((info.min, info.max));
    info.center = if named_property(b, b"autocenter").is_some_and(|v| v == b"0" || v == b"false") { [0.0; 3] } else { [(mn[0] + mx[0]) * 0.5, (mn[1] + mx[1]) * 0.5, (mn[2] + mx[2]) * 0.5] };
    Some(info)
}

/// Value of a named property, in the bytes of an ODOL or MLOD file, where the name and value
/// are plain text whatever else is compressed.
fn named_property(file: &[u8], name: &[u8]) -> Option<Vec<u8>> {
    let key = [name, b"\0"].concat();
    let at = file.windows(key.len()).position(|w| w.eq_ignore_ascii_case(&key))?;
    // MLOD pads the name to 64 bytes; ODOL has the value right after it.
    let mut p = at + key.len();
    while p < file.len() && file[p] == 0 && p < at + 64 {
        p += 1;
    }
    Some(file[p..].iter().take(32).take_while(|c| **c != 0).map(u8::to_ascii_lowercase).collect())
}

/// Box around the points of every LOD of an MLOD file.
fn mlod_bounds(b: &[u8]) -> Option<([f32; 3], [f32; 3])> {
    let int = |p: usize| Some(u32::from_le_bytes(b.get(p..p + 4)?.try_into().ok()?) as usize);
    let skip_text = |mut p: usize| { while *b.get(p)? != 0 { p += 1; } Some(p + 1) };
    let (mut mn, mut mx, mut any) = ([f32::MAX; 3], [f32::MIN; 3], false);
    let mut p = 12;
    for _ in 0..int(8)? {
        if b.get(p..p + 4)? != b"P3DM" {
            return None;
        }
        let (points, normals, faces) = (int(p + 12)?, int(p + 16)?, int(p + 20)?);
        p += 28;
        for i in 0..points {
            for c in 0..3 {
                let v = f(b, p + i * 16 + c * 4)?;
                (mn[c], mx[c]) = (mn[c].min(v), mx[c].max(v));
            }
            any = true;
        }
        p += points * 16 + normals * 12;
        for _ in 0..faces {
            p = skip_text(skip_text(p + 72)?)?;
        }
        p += 4; // TAGG
        loop {
            let name = p + 1;
            let end = skip_text(name)?;
            let last = b.get(name..end - 1)? == b"#EndOfFile#";
            p = end + 4 + int(end)?;
            if last {
                break;
            }
        }
        p += 4; // resolution
    }
    any.then_some((mn, mx))
}

pub fn read_file(path: &Path) -> Option<ModelInfo> {
    let file = std::fs::read(path).ok()?;
    let mut info = parse_odol_header(&file).or_else(|| parse_mlod(&file))?;
    info.placement = Placement::find(&file);
    read_land_contact(&file, &mut info);
    Some(info)
}

/// The LandContact LOD, which only binarised models can be asked for.
fn read_land_contact(file: &[u8], info: &mut ModelInfo) {
    let Some(lod) = file.starts_with(b"ODOL").then(|| forest_core::odol::read(file, |_, r| (r / 1e15 - 2.0).abs() < 0.01).ok()).flatten().and_then(|l| l.into_iter().next()) else { return };
    let Some(first) = lod.vertices.first().copied() else { return };
    let c = info.center;
    info.contact_y = lod.vertices.iter().map(|v| v[1] + c[1]).sum::<f32>() / lod.vertices.len() as f32;
    let plan = |v: &[f32; 3]| [v[0] + c[0], v[2] + c[2]];
    let distance = |v: &&[f32; 3]| (v[0] - first[0]).powi(2) + (v[2] - first[2]).powi(2);
    let far = *lod.vertices.iter().max_by(|a, b| distance(a).total_cmp(&distance(b))).unwrap();
    let area = |v: &&[f32; 3]| ((far[0] - first[0]) * (v[2] - first[2]) - (far[2] - first[2]) * (v[0] - first[0])).abs();
    let side = *lod.vertices.iter().max_by(|a, b| area(a).total_cmp(&area(b))).unwrap();
    info.contact[0] = plan(&first);
    info.contact[1] = plan(&far);
    info.contact_count = if distance(&&far) > 1e-4 { 2 } else { 1 };
    if area(&&side) > 0.25 {
        info.contact[2] = plan(&side);
        info.contact_count = 3;
    }
}

/// True when the game stands the model on its LandContact points: their place on the ground
/// decides its height, and the height stored with the object does not count.
fn stands_on_contacts(info: &ModelInfo) -> bool {
    info.placement == Placement::SlopeLandContact && info.contact_count >= 2
}

/// Height correction of a tilted model with a `placement` property.
///
/// A level model of this kind is sheared until all its contact points lie on the ground. A
/// tilted one cannot bend, so the game stands it on the contact point that would otherwise be
/// highest over the ground and lets the rest go into it: the raised end of a barrier meets the
/// level barrier beside it, whichever end that is, and its other end sinks. Measured on a
/// built map, where this puts both such joints within centimetres of each other.
pub fn tilted_lift(o: &crate::objects::Obj, info: &ModelInfo, height: &crate::terrain::Heightmap) -> f32 {
    if !info.placement.skews() || info.contact_count == 0 || o.flags & crate::objects::FLAG_ABSOLUTE != 0 || (o.pitch.abs() <= 0.01 && o.roll.abs() <= 0.01) {
        return 0.0;
    }
    let m = crate::objects::rotation(o.yaw, o.pitch, o.roll, o.scale);
    // For each contact point: where the origin has to be for that point to lie on the ground.
    let seated = info.contact[..info.contact_count as usize].iter().map(|c| {
        let under = height.sample(o.x + m[0][0] * c[0] + m[2][0] * c[1], o.z + m[0][2] * c[0] + m[2][2] * c[1]);
        under - (m[0][1] * c[0] + m[1][1] * info.contact_y + m[2][1] * c[1])
    }).fold(f32::MAX, f32::min);
    let stored = height.sample(o.x, o.z) + o.elevation;
    if stands_on_contacts(info) {
        seated - stored
    } else {
        // Other placements keep their stored height unless a contact point would stick out.
        (seated + o.elevation - stored).min(0.0)
    }
}

/// How the game shears a model over the terrain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skew {
    /// World height gained per unit of the model's X and Z. Vertical edges stay vertical.
    pub along_x: f32,
    pub along_z: f32,
    /// Added to the height of the origin.
    pub lift: f32,
}

/// The shear of a model with a `placement` property, which the game applies to models left
/// level. A tilt stored on the object is kept as it is, as Terrain Builder and Buldozer do,
/// and objects at a fixed height stay as they are.
pub fn skew(o: &crate::objects::Obj, info: &ModelInfo, height: &crate::terrain::Heightmap) -> Option<Skew> {
    if !info.placement.skews() || o.flags & crate::objects::FLAG_ABSOLUTE != 0 || o.pitch.abs() > 0.01 || o.roll.abs() > 0.01 {
        return None;
    }
    let (s, c) = o.yaw.to_radians().sin_cos();
    let scale = o.scale;
    let ground = |x: f32, z: f32| height.sample(o.x + (c * x + s * z) * scale, o.z + (-s * x + c * z) * scale);
    let p = info.contact;
    let (ax, az, at_origin) = match info.contact_count {
        // Three points make a plane.
        3 => {
            let g = [ground(p[0][0], p[0][1]), ground(p[1][0], p[1][1]), ground(p[2][0], p[2][1])];
            let (ux, uz, vx, vz) = (p[1][0] - p[0][0], p[1][1] - p[0][1], p[2][0] - p[0][0], p[2][1] - p[0][1]);
            let det = (ux * vz - uz * vx).abs().max(1e-5).copysign(ux * vz - uz * vx);
            let (du, dv) = (g[1] - g[0], g[2] - g[0]);
            let (ax, az) = ((du * vz - dv * uz) / det, (dv * ux - du * vx) / det);
            (ax, az, g[0] - ax * p[0][0] - az * p[0][1])
        }
        // Two points make a line, and the model stays level across it.
        2 if (p[1][0] - p[0][0]).abs() + (p[1][1] - p[0][1]).abs() > 0.05 => {
            let (dx, dz) = (p[1][0] - p[0][0], p[1][1] - p[0][1]);
            let len2 = dx * dx + dz * dz;
            let rise = ground(p[1][0], p[1][1]) - ground(p[0][0], p[0][1]);
            let (ax, az) = (rise * dx / len2, rise * dz / len2);
            (ax, az, ground(p[0][0], p[0][1]) - ax * p[0][0] - az * p[0][1])
        }
        _ => {
            let (mx, mz) = ((info.min[0] + info.max[0]) * 0.5, (info.min[2] + info.max[2]) * 0.5);
            let (hx, hz) = (((info.max[0] - info.min[0]) * 0.5).max(0.75), ((info.max[2] - info.min[2]) * 0.5).max(0.75));
            let ax = if info.placement == Placement::SlopeZ { 0.0 } else { (ground(mx + hx, mz) - ground(mx - hx, mz)) / (2.0 * hx) };
            let az = if info.placement == Placement::SlopeX { 0.0 } else { (ground(mx, mz + hz) - ground(mx, mz - hz)) / (2.0 * hz) };
            (ax, az, ground(mx, mz) - ax * mx - az * mz)
        }
    };
    // Standing on its contact points the model takes its height from the ground under them.
    let seat = if stands_on_contacts(info) { -o.elevation - info.contact_y * scale } else { 0.0 };
    Some(Skew { along_x: ax, along_z: az, lift: at_origin - height.sample(o.x, o.z) + seat })
}

/// Cached model information, keyed by WRP path.
#[derive(Clone)]
pub struct ModelDb {
    root: PathBuf,
    cache: Arc<Mutex<HashMap<String, ModelInfo>>>,
    caps: Arc<Mutex<HashMap<String, CapLayout>>>,
    ends: Arc<Mutex<HashMap<(String, bool), Option<[f32; 2]>>>>,
}

/// How thick a long model (a wall, a fence) is at its two ends, at its low end along x (or z)
/// first. Measured on the last 15 cm at each end, between 40 and 80 % of its height, so a
/// foundation or a cap does not count. A wall with a pillar at one end is thicker there.
/// `triangles` are the corners of the model's triangles, three by three.
pub fn end_thickness(triangles: &[[f32; 3]], along_x: bool) -> Option<[f32; 2]> {
    // Points spread over each triangle: a flat face has its corners only at its edges.
    const N: usize = 6;
    let mut positions = Vec::with_capacity(triangles.len() / 3 * (N + 1) * (N + 2) / 2);
    for t in triangles.chunks_exact(3) {
        for i in 0..=N {
            for j in 0..=N - i {
                let (u, v) = (i as f32 / N as f32, j as f32 / N as f32);
                let w = 1.0 - u - v;
                positions.push([0, 1, 2].map(|k| t[0][k] * w + t[1][k] * u + t[2][k] * v));
            }
        }
    }
    let positions = positions.as_slice();
    let (long, across) = if along_x { (0, 2) } else { (2, 0) };
    let range = |k: usize, f: &dyn Fn(&[f32; 3]) -> bool| positions.iter().filter(|p| f(p)).fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[k]), b.max(p[k])));
    let (lo, hi) = range(long, &|_| true);
    let (bottom, top) = range(1, &|_| true);
    if hi - lo < 0.5 || top <= bottom {
        return None;
    }
    let band = |p: &[f32; 3]| p[1] >= bottom + (top - bottom) * 0.4 && p[1] <= bottom + (top - bottom) * 0.8;
    let depth = |near: f32| {
        let (a, b) = range(across, &|p| band(p) && (p[long] - near).abs() <= 0.15);
        (b > a).then_some(b - a)
    };
    Some([depth(lo)?, depth(hi)?])
}

/// How the road of an end piece lies along it. The piece runs from its start port (local
/// z = 0) to its end port (z = length); its road meets the rest of the road at a hard edge
/// and fades out away from it. Vanilla pieces fade towards their start and meet the road at
/// their end; some pieces of other maps are the other way round, or have their road a way
/// inside the model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapLayout {
    /// The hard edge is at the end port's side.
    pub joint_at_end: bool,
    /// How far inside the piece the hard edge lies from that port.
    pub inset: f64,
    /// How long the stretch at the far end is where nothing of the road shows any more.
    pub tail: f64,
}

impl Default for CapLayout {
    fn default() -> Self {
        Self { joint_at_end: true, inset: 0.0, tail: 0.0 }
    }
}

/// Triangles of a model with their UVs, and the texture of each, as `width`, `height` and RGBA.
pub type Textured<'a> = (&'a [[f32; 3]], &'a [[f32; 2]], Option<(u32, u32, &'a [u8])>);

/// [`CapLayout`] of a model whose start port is at z = `start`, from how opaque its textures
/// are along it, read through its UVs.
pub fn cap_layout_of(surfaces: &[Textured], start: f64, length: f64) -> CapLayout {
    const BINS: usize = 50;
    if length <= 0.0 {
        return CapLayout::default();
    }
    // How much road is drawn across each stretch of the piece: alpha times area, and area.
    let (mut cover, mut area_of) = ([0.0f64; BINS], [0.0f64; BINS]);
    let bin = |z: f32| (((z as f64 - start) / length * BINS as f64).floor() as isize).clamp(0, BINS as isize - 1) as usize;
    for (positions, uvs, tex) in surfaces {
        let tex = tex.filter(|t| t.0 > 0 && t.1 > 0 && t.2.len() >= (t.0 * t.1 * 4) as usize);
        let alpha = |uv: [f32; 2]| -> f64 {
            let Some((w, h, rgba)) = tex else { return 1.0 };
            let x = ((uv[0].rem_euclid(1.0) * w as f32) as u32).min(w - 1);
            let y = ((uv[1].rem_euclid(1.0) * h as f32) as u32).min(h - 1);
            rgba[((y * w + x) * 4 + 3) as usize] as f64 / 255.0
        };
        for (v, t) in positions.chunks_exact(3).zip(uvs.chunks_exact(3)) {
            let (e1, e2) = ([v[1][0] - v[0][0], v[1][2] - v[0][2]], [v[2][0] - v[0][0], v[2][2] - v[0][2]]);
            let area = (e1[0] * e2[1] - e1[1] * e2[0]).abs() as f64 * 0.5;
            if area < 1e-6 {
                continue;
            }
            const K: usize = 48;
            for i in 0..K {
                for j in 0..K {
                    let (mut a, mut c) = ((i as f32 + 0.5) / K as f32, (j as f32 + 0.5) / K as f32);
                    if a + c > 1.0 {
                        (a, c) = (1.0 - a, 1.0 - c);
                    }
                    let w0 = 1.0 - a - c;
                    let z = v[0][2] * w0 + v[1][2] * a + v[2][2] * c;
                    let uv = [t[0][0] * w0 + t[1][0] * a + t[2][0] * c, t[0][1] * w0 + t[1][1] * a + t[2][1] * c];
                    cover[bin(z)] += area / (K * K) as f64 * alpha(uv);
                    area_of[bin(z)] += area / (K * K) as f64;
                }
            }
        }
    }
    // Where the piece is drawn at all, the share of it that is road; nothing drawn counts as none.
    let widest = area_of.iter().cloned().fold(0.0, f64::max);
    for k in 0..BINS {
        cover[k] = if area_of[k] > widest * 0.05 { cover[k] / area_of[k] } else { 0.0 };
    }
    let top = cover.iter().cloned().fold(0.0, f64::max);
    if top <= 0.0 {
        return CapLayout::default();
    }
    let solid: Vec<usize> = (0..BINS).filter(|&k| cover[k] > top * 0.5).collect();
    let (first, last) = (solid[0], *solid.last().unwrap());
    let step = length / BINS as f64;
    // The road fades out on one side of its solid stretch; it meets the road on the other.
    let (below, above): (f64, f64) = (cover[..first].iter().sum(), cover[last + 1..].iter().sum());
    // The road sits inside the piece only where nothing at all is drawn between it and the port.
    let empty = |k: &usize| cover[*k] <= top * 0.05;
    let snap = |bins: usize| if (bins as f64 * step) < 0.3 { 0.0 } else { bins as f64 * step };
    let (from_start, from_end) = (snap((0..BINS).take_while(empty).count()), snap((0..BINS).rev().take_while(empty).count()));
    if above > below * 1.2 + top * 0.05 {
        CapLayout { joint_at_end: false, inset: from_start, tail: from_end }
    } else {
        CapLayout { joint_at_end: true, inset: from_end, tail: from_start }
    }
}

/// [`CapLayout`] of the model in `file`, a source (MLOD) or a binarized one.
fn read_cap_layout(file: &Path, root: &Path, start: f64, length: f64) -> CapLayout {
    if let Ok(m) = forest_core::model3d::load_model(file, root, 64) {
        let mut surfaces = Vec::new();
        let (mut positions, mut uvs) = (Vec::new(), Vec::new());
        for b in &m.batches {
            let start = positions.len();
            for &k in &m.indices[b.first_index as usize..(b.first_index + b.index_count) as usize] {
                positions.push(m.vertices[k as usize].pos);
                uvs.push(m.vertices[k as usize].uv);
            }
            surfaces.push((start, positions.len(), b.texture.and_then(|t| m.textures.get(t))));
        }
        let list: Vec<Textured> = surfaces.iter().map(|(a, b, t)| (&positions[*a..*b], &uvs[*a..*b], t.map(|t| (t.width, t.height, t.rgba.as_slice())))).collect();
        return cap_layout_of(&list, start, length);
    }
    let cache = dayz_road_tool::mesh3d::Cache::default();
    let Some(m) = cache.mesh(file) else { return CapLayout::default() };
    let textures: Vec<_> = m.surfaces.iter().map(|s| cache.texture(&s.texture, file)).collect();
    let list: Vec<Textured> = m.surfaces.iter().zip(&textures).map(|(s, t)| (s.positions.as_slice(), s.uvs.as_slice(), t.as_ref().map(|t| (t.width, t.height, t.rgba.as_slice())))).collect();
    cap_layout_of(&list, start, length)
}

impl ModelDb {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), cache: Default::default(), caps: Default::default(), ends: Default::default() }
    }
    /// [`end_thickness`] of a model, read once per model.
    pub fn end_thickness(&self, model: &str, along_x: bool) -> Option<[f32; 2]> {
        let key = (model.to_string(), along_x);
        if let Some(e) = self.ends.lock().unwrap().get(&key) {
            return *e;
        }
        let cache = dayz_road_tool::mesh3d::Cache::default();
        let e = cache.mesh(&self.file(model)).and_then(|m| {
            let positions: Vec<[f32; 3]> = m.surfaces.iter().flat_map(|s| s.positions.iter().copied()).collect();
            end_thickness(&positions, along_x)
        });
        self.ends.lock().unwrap().insert(key, e);
        e
    }
    /// How the road of end piece `model` lies along it, from its start port at z = `start` to its
    /// end port `length` further on; read once per model.
    pub fn cap_layout(&self, model: &str, start: f64, length: f64) -> CapLayout {
        if let Some(c) = self.caps.lock().unwrap().get(model) {
            return *c;
        }
        let layout = read_cap_layout(&self.file(model), &self.root, start, length);
        self.caps.lock().unwrap().insert(model.to_string(), layout);
        layout
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn file(&self, model: &str) -> PathBuf {
        self.root.join(model.replace('\\', "/"))
    }
    pub fn get(&self, model: &str) -> ModelInfo {
        if let Some(i) = self.cache.lock().unwrap().get(model) {
            return *i;
        }
        let info = read_file(&self.file(model)).unwrap_or_default();
        self.cache.lock().unwrap().insert(model.to_string(), info);
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_wall_with_a_pillar_is_thicker_at_that_end() {
        // Two boxes as triangles: a 0.7 m pillar at x -3..-2.3 and a 0.3 m slab on to x 3.
        fn cuboid(min: [f32; 3], max: [f32; 3]) -> Vec<[f32; 3]> {
            let c = |i: usize| [if i & 1 == 0 { min[0] } else { max[0] }, if i & 2 == 0 { min[1] } else { max[1] }, if i & 4 == 0 { min[2] } else { max[2] }];
            let faces = [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4], [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]];
            faces.iter().flat_map(|f| [c(f[0]), c(f[1]), c(f[2]), c(f[0]), c(f[2]), c(f[3])]).collect()
        }
        let mut wall = cuboid([-3.0, 0.0, -0.35], [-2.3, 2.5, 0.35]);
        wall.extend(cuboid([-2.3, 0.0, -0.15], [3.0, 2.2, 0.15]));
        let [low, high] = end_thickness(&wall, true).unwrap();
        assert!((low - 0.7).abs() < 0.01 && (high - 0.3).abs() < 0.01, "{low} {high}");
        // Along z nothing is long enough to measure.
        assert!(end_thickness(&cuboid([-0.1, 0.0, -0.1], [0.1, 1.0, 0.1]), false).is_none());
    }
    #[test]
    fn a_tilted_barrier_stands_on_its_raised_end_whichever_end_that_is() {
        use crate::objects::{rotation, Obj};
        let info = ModelInfo { contact: [[-7.0, 0.0], [0.0, 0.0], [0.0, 0.0]], contact_count: 2, known: true, placement: Placement::SlopeLandContact, ..Default::default() };
        let h = crate::terrain::Heightmap::new(32, 32, 10.0, 10.0);
        // The stored height does not count: the game puts the contact points on the ground.
        for (roll, elevation) in [(-12.0f32, -0.4f32), (12.0, -1.8), (-12.0, 2.0)] {
            let o = Obj { x: 100.0, z: 100.0, elevation, yaw: 25.0, pitch: 0.0, roll, scale: 1.4, model: 0, flags: 0 };
            let m = rotation(o.yaw, o.pitch, o.roll, o.scale);
            let origin = 10.0 + o.elevation + tilted_lift(&o, &info, &h);
            let far = origin + m[0][1] * -7.0;
            // The higher end lies on the ground, the other one is in it.
            assert!((origin.max(far) - 10.0).abs() < 0.01, "{roll}: {origin} {far}");
            assert!(origin.min(far) < 8.5, "{roll}: {origin} {far}");
            assert!(skew(&o, &info, &h).is_none());
        }
        let level = Obj { x: 100.0, z: 100.0, elevation: -0.4, yaw: 25.0, pitch: 0.0, roll: 0.0, scale: 1.4, model: 0, flags: 0 };
        assert_eq!(tilted_lift(&level, &info, &h), 0.0);
        // A level one is seated the same way: on the ground, whatever height is stored.
        let s = skew(&level, &info, &h).unwrap();
        assert!((10.0 + level.elevation + s.lift - 10.0).abs() < 0.01, "{s:?}");
        // Without the property nothing is done.
        let tilted = Obj { roll: -12.0, ..level };
        assert_eq!(tilted_lift(&tilted, &ModelInfo { placement: Placement::None, ..info }, &h), 0.0);
        // Other placements keep the stored height.
        let slope = ModelInfo { placement: Placement::Slope, ..info };
        assert!((skew(&level, &slope, &h).unwrap().lift).abs() < 0.01);
    }

    fn mlod(lods: &[&[[f32; 3]]], property: Option<&str>) -> Vec<u8> {
        let mut b = b"MLOD".to_vec();
        b.extend(257u32.to_le_bytes());
        b.extend((lods.len() as u32).to_le_bytes());
        for points in lods {
            b.extend(b"P3DM");
            b.extend([0u8; 8]);
            b.extend((points.len() as u32).to_le_bytes());
            b.extend([0u8; 12]);
            for p in *points {
                for v in p { b.extend(v.to_le_bytes()); }
                b.extend([0u8; 4]);
            }
            b.extend(b"TAGG");
            if let Some(v) = property {
                b.push(1);
                b.extend(b"#Property#\0");
                b.extend(128u32.to_le_bytes());
                let mut text = [0u8; 128];
                text[..10].copy_from_slice(b"autocenter");
                text[64..64 + v.len()].copy_from_slice(v.as_bytes());
                b.extend(text);
            }
            b.push(1);
            b.extend(b"#EndOfFile#\0");
            b.extend(0u32.to_le_bytes());
            b.extend(1.0f32.to_le_bytes());
        }
        b
    }

    #[test]
    fn a_source_model_is_centred_like_the_binarised_one() {
        let visual: &[[f32; 3]] = &[[-1.0, 0.0, -1.0], [1.0, 2.0, 1.0]];
        let shadow: &[[f32; 3]] = &[[-1.0, 0.0, -1.0], [5.0, 2.0, 1.0]];
        assert_eq!(parse_mlod(&mlod(&[visual, shadow], None)).unwrap().center, [2.0, 1.0, 0.0]);
        assert_eq!(parse_mlod(&mlod(&[visual, shadow], Some("0"))).unwrap().center, [0.0; 3]);
    }

    #[test]
    fn placement_is_found_in_both_file_formats() {
        assert_eq!(Placement::find(b"xxplacement\0slopelandcontact\0more"), Placement::SlopeLandContact);
        let mut mlod = b"#Property#\0".to_vec();
        mlod.extend_from_slice(b"Placement");
        mlod.resize(mlod.len() + 55, 0);
        mlod.extend_from_slice(b"slopeZ\0");
        assert_eq!(Placement::find(&mlod), Placement::SlopeZ);
        assert_eq!(Placement::find(b"no such thing"), Placement::None);
    }

    #[test]
    fn placed_models_are_sheared_over_the_ground() {
        use crate::objects::Obj;
        use crate::terrain::Heightmap;
        // Rising towards the east by 1 in 4.
        let mut h = Heightmap::new(64, 64, 4.0, 0.0);
        for j in 0..64 { for i in 0..64 { h.set(i, j, i as f32); } }
        // Long along X with its origin at one end, resting on two contact points like a barrier.
        let info = ModelInfo { min: [-7.0, 0.0, -0.1], max: [0.0, 1.0, 0.1], known: true, placement: Placement::SlopeLandContact, contact: [[-7.0, 0.0], [0.0, 0.0], [0.0, 0.0]], contact_count: 2, ..Default::default() };
        let o = Obj { x: 100.0, z: 100.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        let s = skew(&o, &info, &h).unwrap();
        assert!((s.along_x - 0.25).abs() < 1e-3 && s.along_z.abs() < 1e-3 && s.lift.abs() < 1e-3, "{s:?}");
        // Both contacts end up on the ground, here with the model turned to face the slope.
        let east = Obj { yaw: 90.0, ..o };
        let s = skew(&east, &info, &h).unwrap();
        let local = -7.0;
        let far = [100.0, 100.0 + 7.0];
        assert!((h.sample(100.0, 100.0) + s.lift + s.along_x * local - h.sample(far[0], far[1])).abs() < 0.02, "{s:?}");
        // A tilt from Terrain Builder is kept, as Buldozer shows it.
        assert_eq!(skew(&Obj { roll: 11.0, ..o }, &info, &h), None);
        assert_eq!(skew(&Obj { flags: crate::objects::FLAG_ABSOLUTE, ..o }, &info, &h), None);
        assert_eq!(skew(&o, &ModelInfo { placement: Placement::None, ..info }, &h), None);
    }

    #[test]
    fn odol_header_gives_the_center_terrain_builder_used() {
        // Values measured in a Terrain Builder WRP: y offset over the terrain of this model.
        let Some(i) = read_file(Path::new("P:/dz/plants_bliss/tree/t_fagussylvatica_1fc_summer.p3d")) else { return };
        assert!((i.center[1] - 10.304).abs() < 0.01, "{i:?}");
        assert!(i.min[1] < 1.0 && i.max[1] > 15.0, "{i:?}");
    }
    #[test]
    fn rejects_garbage() {
        assert!(parse_odol_header(b"ODOL").is_none());
        assert!(parse_odol_header(b"XXXX0000").is_none());
    }
}
