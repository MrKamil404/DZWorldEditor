//! Placement data of P3D models: bounding box and the bounding centre WRP positions use.
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModelInfo {
    /// Offset of the mesh origin from the model's ground origin.
    pub center: [f32; 3],
    /// Box relative to the ground origin.
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// False when the file was missing or unreadable.
    pub known: bool,
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
    any.then_some(ModelInfo { center: [0.0; 3], min: mn, max: mx, known: true })
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
    // Terrain Builder centres MLOD models on their box in the WRP as well.
    info.center = [(info.min[0] + info.max[0]) * 0.5, (info.min[1] + info.max[1]) * 0.5, (info.min[2] + info.max[2]) * 0.5];
    Some(info)
}

pub fn read_file(path: &Path) -> Option<ModelInfo> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = vec![0u8; 8192];
    let n = file.read(&mut head).ok()?;
    head.truncate(n);
    if head.starts_with(b"ODOL") {
        return parse_odol_header(&head);
    }
    if head.starts_with(b"MLOD") {
        let mut rest = Vec::new();
        file.read_to_end(&mut rest).ok()?;
        head.extend_from_slice(&rest);
        return parse_mlod(&head);
    }
    None
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
