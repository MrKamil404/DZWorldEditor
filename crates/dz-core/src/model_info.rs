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
    Some(Skew { along_x: ax, along_z: az, lift: at_origin - height.sample(o.x, o.z) })
}

/// Cached model information, keyed by WRP path.
#[derive(Clone)]
pub struct ModelDb {
    root: PathBuf,
    cache: Arc<Mutex<HashMap<String, ModelInfo>>>,
}

impl ModelDb {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), cache: Default::default() }
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
    /// A tilt stored on the object stands as it is: Buldozer and the built map show a barrier
    /// with `placement = slopelandcontact` exactly where Terrain Builder's numbers put it.
    #[test]
    fn a_tilted_model_is_not_sheared() {
        use crate::objects::Obj;
        let info = ModelInfo { contact: [[-7.0, 0.0], [0.0, 0.0], [0.0, 0.0]], contact_count: 2, known: true, placement: Placement::SlopeLandContact, ..Default::default() };
        let h = crate::terrain::Heightmap::new(32, 32, 10.0, 10.0);
        let o = Obj { x: 100.0, z: 100.0, elevation: -0.367, yaw: 25.0, pitch: 1.4, roll: -12.2, scale: 1.4, model: 0, flags: 0 };
        assert!(skew(&o, &info, &h).is_none());
        assert!(skew(&Obj { pitch: 0.0, roll: 0.0, ..o }, &info, &h).is_some());
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
