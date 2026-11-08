//! Project folders: a small JSON description next to raw binary rasters.
use crate::fsutil::{atomic_write, bytes_to_f32s, f32s_to_bytes};
use crate::objects::ObjectStore;
use crate::terrain::{Heightmap, MaskMap, RgbImage};
use crate::world::{MapLine, SurfaceLayer, TerrainModifier, World, WorldMeta};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PROJECT_FILE: &str = "project.dzwe";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    version: u32,
    meta: WorldMeta,
    layers: Vec<SurfaceLayer>,
    modifiers: Vec<TerrainModifier>,
    lines: Vec<MapLine>,
    mask_size: [usize; 2],
    satellite_size: Option<[usize; 2]>,
}

/// Which parts need rewriting; rasters are large, so unchanged ones are skipped.
#[derive(Clone, Copy, Debug)]
pub struct SaveParts {
    pub height: bool,
    pub mask: bool,
    pub satellite: bool,
    pub objects: bool,
}
impl SaveParts {
    pub const ALL: SaveParts = SaveParts { height: true, mask: true, satellite: true, objects: true };
}

pub fn save(w: &World, dir: &Path, parts: SaveParts) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    if parts.height {
        atomic_write(&dir.join("height.f32"), &f32s_to_bytes(&w.base.data))?;
    }
    if parts.mask {
        atomic_write(&dir.join("mask.u8"), &w.mask.data)?;
    }
    match (&w.satellite, parts.satellite) {
        (Some(s), true) => atomic_write(&dir.join("satellite.rgb"), &s.data)?,
        (None, true) => {
            let _ = std::fs::remove_file(dir.join("satellite.rgb"));
        }
        _ => {}
    }
    if parts.objects {
        atomic_write(&dir.join("objects.dzob"), &w.objects.to_bytes())?;
    }
    let file = ProjectFile {
        version: VERSION,
        meta: w.meta.clone(),
        layers: w.layers.clone(),
        modifiers: w.modifiers.clone(),
        lines: w.lines.clone(),
        mask_size: [w.mask.width, w.mask.height],
        satellite_size: w.satellite.as_ref().map(|s| [s.width, s.height]),
    };
    atomic_write(&dir.join(PROJECT_FILE), &serde_json::to_vec_pretty(&file)?)
}

pub fn load(dir: &Path) -> Result<World> {
    let path = dir.join(PROJECT_FILE);
    let file: ProjectFile = serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("{}", path.display()))?)
        .with_context(|| format!("{}", path.display()))?;
    ensure!(file.version <= VERSION, "The project was saved by a newer version of the editor");
    let n = file.meta.terrain_size as usize;
    let data = bytes_to_f32s(&std::fs::read(dir.join("height.f32")).context("height.f32")?);
    ensure!(data.len() == n * n, "height.f32 does not match the project size");
    let base = Heightmap { cols: n, rows: n, cell: file.meta.terrain_cell, data };
    let mask_data = std::fs::read(dir.join("mask.u8")).context("mask.u8")?;
    ensure!(mask_data.len() == file.mask_size[0] * file.mask_size[1], "mask.u8 does not match the project");
    let mask = MaskMap { width: file.mask_size[0], height: file.mask_size[1], data: mask_data };
    let satellite = match file.satellite_size {
        Some([width, height]) => {
            let data = std::fs::read(dir.join("satellite.rgb")).context("satellite.rgb")?;
            ensure!(data.len() == width * height * 3, "satellite.rgb does not match the project");
            Some(RgbImage { width, height, data })
        }
        None => None,
    };
    let objects = ObjectStore::from_bytes(&std::fs::read(dir.join("objects.dzob")).context("objects.dzob")?)?;
    Ok(World::from_parts(file.meta, base, mask, file.layers, satellite, objects, file.modifiers, file.lines))
}

/// `dir` holds a project.
pub fn is_project(dir: &Path) -> bool {
    dir.join(PROJECT_FILE).is_file()
}

pub fn project_dir_of(path: &Path) -> PathBuf {
    if path.is_file() {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::Obj;
    use crate::world::{ModMode, ModShape};
    #[test]
    fn save_and_load_keep_everything() {
        let d = tempfile::tempdir().unwrap();
        let mut w = World::new(WorldMeta::new("My Map", 128, 8.0), 50.0);
        w.base.set(3, 4, 77.0);
        w.mask.data[10] = 2;
        w.satellite = Some(RgbImage::new(16, 16, [1, 2, 3]));
        let m = w.objects.intern("dz\\x.p3d");
        w.objects.insert(Obj { x: 1.0, z: 2.0, elevation: 0.5, yaw: 3.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: m, flags: 0 });
        w.modifiers.push(TerrainModifier { id: 9, name: "pad".into(), enabled: true, x: 400.0, z: 400.0, yaw: 0.0, shape: ModShape::Circle { radius: 16.0 }, mode: ModMode::Offset, target: None, amount: 3.0, falloff: 8.0, attached: None });
        save(&w, d.path(), SaveParts::ALL).unwrap();
        let mut r = load(d.path()).unwrap();
        assert_eq!(r.meta.map_id, "MyMap");
        assert_eq!(r.base.at(3, 4), 77.0);
        assert_eq!(r.mask.data[10], 2);
        assert_eq!(r.satellite.as_ref().unwrap().data[1], 2);
        assert_eq!(r.objects.len(), 1);
        // Modifiers are applied on load.
        assert_eq!(r.height.at(50, 50), 53.0);
        assert_eq!(r.new_modifier_id(), 10);
    }
    #[test]
    fn partial_save_leaves_other_files() {
        let d = tempfile::tempdir().unwrap();
        let w = World::new(WorldMeta::new("A", 64, 8.0), 1.0);
        save(&w, d.path(), SaveParts::ALL).unwrap();
        let before = std::fs::metadata(d.path().join("mask.u8")).unwrap().modified().unwrap();
        save(&w, d.path(), SaveParts { height: true, mask: false, satellite: false, objects: false }).unwrap();
        assert_eq!(std::fs::metadata(d.path().join("mask.u8")).unwrap().modified().unwrap(), before);
    }
}
