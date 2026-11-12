//! Project folders: a small JSON description next to raw binary rasters.
use crate::fsutil::{atomic_write, bytes_to_f32s, f32s_to_bytes};
use crate::objects::ObjectStore;
use crate::terrain::{Heightmap, MaskMap, RgbImage};
use crate::world::{default_object_layers, MapLine, ObjectLayer, SurfaceLayer, TerrainModifier, World, WorldMeta};
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
    /// Road lines of older projects; they are turned into routes on load.
    #[serde(default, skip_serializing)]
    lines: Vec<MapLine>,
    mask_size: [usize; 2],
    satellite_size: Option<[usize; 2]>,
    #[serde(default = "default_object_layers")]
    object_layers: Vec<ObjectLayer>,
    #[serde(default)]
    active_layer: u16,
    #[serde(default)]
    shape_layers: Vec<crate::shapes::ShapeLayer>,
    #[serde(default)]
    active_shape_layer: usize,
    #[serde(default)]
    generators: Vec<crate::library::GeneratorInstance>,
    #[serde(default)]
    road_removed: std::collections::BTreeSet<String>,
    #[serde(default)]
    road_added: Vec<crate::world::RoadPart>,
    #[serde(default)]
    road_project: Option<PathBuf>,
    #[serde(default)]
    template_libraries: Vec<PathBuf>,
    #[serde(default)]
    routes: Vec<crate::world::RoadRoute>,
    #[serde(default)]
    holes: crate::holes::Holes,
    #[serde(default)]
    keypoints: Vec<crate::keypoints::KeyPoint>,
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
    // The layers of the satellite modifiers, with the original pixels under them.
    if w.sat.is_empty() {
        let _ = std::fs::remove_file(dir.join("satellite.mods"));
    } else {
        atomic_write(&dir.join("satellite.mods"), &w.sat.to_bytes())?;
    }
    if w.mask_mods.is_empty() {
        let _ = std::fs::remove_file(dir.join("mask.mods"));
    } else {
        atomic_write(&dir.join("mask.mods"), &w.mask_mods.to_bytes())?;
    }
    if parts.objects {
        atomic_write(&dir.join("objects.dzob"), &w.objects.to_bytes())?;
    }
    // Loading numbers objects in file order, so references are written in that numbering.
    let ordinal: std::collections::HashMap<u32, u32> = if w.routes.is_empty() && w.generators.is_empty() && w.modifiers.iter().all(|m| m.attached.is_none()) {
        Default::default()
    } else {
        w.objects.iter().enumerate().map(|(n, (id, _))| (id, n as u32)).collect()
    };
    let mut modifiers = w.modifiers.clone();
    for m in &mut modifiers {
        m.attached = m.attached.and_then(|id| ordinal.get(&id).copied());
    }
    let mut generators = w.generators.clone();
    for g in &mut generators {
        g.objects = g.objects.iter().filter_map(|id| ordinal.get(id).copied()).collect();
    }
    let mut routes = w.routes.clone();
    for r in &mut routes {
        r.objects = r.objects.iter().filter_map(|id| ordinal.get(id).copied()).collect();
    }
    let file = ProjectFile {
        version: VERSION,
        meta: w.meta.clone(),
        layers: w.layers.clone(),
        modifiers,
        lines: Vec::new(),
        mask_size: [w.mask.width, w.mask.height],
        satellite_size: w.satellite.as_ref().map(|s| [s.width, s.height]),
        object_layers: w.object_layers.clone(),
        active_layer: w.active_layer,
        shape_layers: w.shape_layers.clone(),
        holes: w.holes.clone(),
        keypoints: w.keypoints.clone(),
        active_shape_layer: w.active_shape_layer,
        generators,
        road_removed: w.road_removed.clone(),
        road_added: w.road_added.clone(),
        road_project: w.road_project.clone(),
        template_libraries: w.template_libraries.clone(),
        routes,
    };
    atomic_write(&dir.join(PROJECT_FILE), &serde_json::to_vec_pretty(&file)?)
}

/// Writes `bytes` to `to`, or links the same file of an earlier backup when nothing changed.
fn share_or_write(earlier: Option<PathBuf>, to: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Read;
    if let Some(from) = earlier.filter(|p| p.metadata().is_ok_and(|m| m.len() == bytes.len() as u64)) {
        let mut file = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(&from)?);
        let mut buffer = vec![0u8; 1 << 20];
        let mut at = 0;
        let same = loop {
            let n = file.read(&mut buffer)?;
            if n == 0 { break true; }
            if buffer[..n] != bytes[at..at + n] { break false; }
            at += n;
        };
        if same && std::fs::hard_link(&from, to).is_ok() {
            return Ok(());
        }
    }
    atomic_write(to, bytes)
}

/// Whether a folder name is one this program gave a backup: `2026-11-09_18-30-05`.
fn is_backup_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 19 && b.iter().enumerate().all(|(i, c)| match i {
        4 | 7 | 13 | 16 => *c == b'-',
        10 => *c == b'_',
        _ => c.is_ascii_digit(),
    })
}

/// Backups in `root`, oldest first.
pub fn backups_in(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| is_backup_name(&n.to_string_lossy()))).collect();
    out.sort();
    out
}

/// Writes a complete copy of the project to `root\<name>` and keeps the newest `keep` copies.
/// `satellite_file` is the image on disk when it has not changed, so it need not be written.
pub fn write_backup(w: &World, root: &Path, name: &str, keep: usize, satellite_file: Option<&Path>) -> Result<PathBuf> {
    std::fs::create_dir_all(root)?;
    for e in std::fs::read_dir(root)?.flatten() {
        if e.path().extension().is_some_and(|x| x == "tmp") {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    let previous = backups_in(root).pop();
    let earlier = |file: &str| previous.as_ref().map(|p| p.join(file));
    let tmp = root.join(format!("{name}.tmp"));
    std::fs::create_dir_all(&tmp)?;
    save(w, &tmp, SaveParts { height: false, mask: false, satellite: false, objects: true })?;
    share_or_write(earlier("height.f32"), &tmp.join("height.f32"), &f32s_to_bytes(&w.base.data))?;
    share_or_write(earlier("mask.u8"), &tmp.join("mask.u8"), &w.mask.data)?;
    match (satellite_file, &w.satellite) {
        (Some(file), _) => {
            let to = tmp.join("satellite.rgb");
            if std::fs::hard_link(file, &to).is_err() {
                std::fs::copy(file, &to).with_context(|| format!("{}", file.display()))?;
            }
        }
        (None, Some(s)) => share_or_write(earlier("satellite.rgb"), &tmp.join("satellite.rgb"), &s.data)?,
        (None, None) => {}
    }
    let done = root.join(name);
    std::fs::rename(&tmp, &done)?;
    let all = backups_in(root);
    for old in &all[..all.len().saturating_sub(keep.max(1))] {
        std::fs::remove_dir_all(old)?;
    }
    Ok(done)
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
    let mut w = World::from_parts(file.meta, base, mask, file.layers, satellite, objects, file.modifiers);
    if let Ok(bytes) = std::fs::read(dir.join("mask.mods")) {
        w.mask_mods = crate::maskmod::MaskLayers::from_bytes(&bytes).context("mask.mods")?;
    }
    if let Ok(bytes) = std::fs::read(dir.join("satellite.mods")) {
        w.sat = crate::satmod::SatLayers::from_bytes(&bytes).context("satellite.mods")?;
    }
    if !file.object_layers.is_empty() {
        w.object_layers = file.object_layers;
        w.active_layer = file.active_layer.min(w.object_layers.len() as u16 - 1);
    }
    w.shape_layers = file.shape_layers;
    w.active_shape_layer = file.active_shape_layer;
    w.generators = file.generators;
    w.road_removed = file.road_removed;
    w.road_added = file.road_added;
    w.road_project = file.road_project;
    w.template_libraries = file.template_libraries;
    w.routes = file.routes;
    w.holes = file.holes;
    w.keypoints = file.keypoints;
    for r in &mut w.routes {
        if r.id == 0 {
            r.id = crate::library::new_id();
        }
    }
    crate::import::adopt_old_lines(&mut w, file.lines);
    Ok(w)
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
    fn backups_open_share_what_is_unchanged_and_are_pruned() {
        let d = tempfile::tempdir().unwrap();
        let mut w = World::new(WorldMeta::new("My Map", 64, 8.0), 50.0);
        w.base.set(3, 4, 77.0);
        w.satellite = Some(RgbImage::new(16, 16, [1, 2, 3]));
        let root = d.path().join("backups");
        std::fs::create_dir_all(root.join("my own folder")).unwrap();
        let first = write_backup(&w, &root, "2026-01-01_10-00-00", 2, None).unwrap();
        let second = write_backup(&w, &root, "2026-01-01_10-30-00", 2, None).unwrap();
        assert_eq!(std::fs::read(first.join("height.f32")).unwrap(), std::fs::read(second.join("height.f32")).unwrap());
        w.base.set(5, 5, 12.0);
        let third = write_backup(&w, &root, "2026-01-01_11-00-00", 2, None).unwrap();
        assert!(!first.exists() && second.exists() && third.exists());
        assert!(root.join("my own folder").exists(), "only backups are ever removed");
        assert_eq!(load(&second).unwrap().base.at(5, 5), 50.0);
        let r = load(&third).unwrap();
        assert_eq!((r.base.at(5, 5), r.base.at(3, 4)), (12.0, 77.0));
        assert_eq!(r.satellite.as_ref().unwrap().data[1], 2);
        assert_eq!(backups_in(&root).len(), 2);
    }

    #[test]
    fn a_backup_can_take_the_satellite_image_from_the_project() {
        let d = tempfile::tempdir().unwrap();
        let mut w = World::new(WorldMeta::new("Sat", 64, 8.0), 50.0);
        w.satellite = Some(RgbImage::new(16, 16, [9, 8, 7]));
        save(&w, d.path(), SaveParts::ALL).unwrap();
        // The copy handed to the backup keeps only the size of the image.
        w.satellite.as_mut().unwrap().data = Vec::new();
        let done = write_backup(&w, &d.path().join("b"), "2026-01-01_10-00-00", 3, Some(&d.path().join("satellite.rgb"))).unwrap();
        assert_eq!(load(&done).unwrap().satellite.unwrap().data[0], 9);
    }

    #[test]
    fn save_and_load_keep_everything() {
        let d = tempfile::tempdir().unwrap();
        let mut w = World::new(WorldMeta::new("My Map", 128, 8.0), 50.0);
        w.base.set(3, 4, 77.0);
        w.mask.data[10] = 2;
        w.satellite = Some(RgbImage::new(16, 16, [1, 2, 3]));
        let m = w.objects.intern("dz\\x.p3d");
        w.objects.insert(Obj { x: 1.0, z: 2.0, elevation: 0.5, yaw: 3.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: m, flags: 0 });
        w.modifiers.push(TerrainModifier { id: 9, name: "pad".into(), enabled: true, x: 400.0, z: 400.0, yaw: 0.0, shape: ModShape::Circle { radius: 16.0 }, mode: ModMode::Offset, target: None, amount: 3.0, falloff: 8.0, attached: None, follow: false });
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
        assert_eq!(r.object_layers.len(), 1);
    }
    #[test]
    fn routes_keep_their_objects_when_saving_compacts_ids() {
        let d = tempfile::tempdir().unwrap();
        let mut w = World::new(WorldMeta::new("L", 64, 8.0), 1.0);
        let m = w.objects.intern("dz\\x.p3d");
        let o = Obj { x: 1.0, z: 2.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: m, flags: 0 };
        let gone = w.objects.insert(o);
        let kept = w.objects.insert(Obj { x: 9.0, ..o });
        w.objects.remove(gone);
        w.routes.push(crate::world::RoadRoute { name: "r".into(), family: "asf2".into(), points: vec![], centerline: vec![], objects: vec![kept], id: 1, grading: Default::default(), profile: Vec::new(), decks: vec![], terrain_route: true, settings: Default::default(), grade: false, grade_width: 9.0, grade_falloff: 12.0, modifier: None, parts: Vec::new(), fitted: Vec::new() });
        save(&w, d.path(), SaveParts::ALL).unwrap();
        let r = load(d.path()).unwrap();
        assert_eq!(r.objects.get(r.routes[0].objects[0]).unwrap().x, 9.0);
    }
    #[test]
    fn object_layers_survive_saving() {
        let d = tempfile::tempdir().unwrap();
        let mut w = World::new(WorldMeta::new("L", 64, 8.0), 1.0);
        let l = w.add_object_layer("Rocks");
        w.object_layers[l as usize].visible = false;
        w.active_layer = l;
        let m = w.objects.intern("dz\\x.p3d");
        let mut o = Obj { x: 1.0, z: 2.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: m, flags: 0 };
        o.set_layer(l);
        w.objects.insert(o);
        save(&w, d.path(), SaveParts::ALL).unwrap();
        let r = load(d.path()).unwrap();
        assert_eq!(r.object_layers[1].name, "Rocks");
        assert!(!r.object_layers[1].visible);
        assert_eq!(r.active_layer, 1);
        assert_eq!(r.objects.iter().next().unwrap().1.layer(), 1);
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
