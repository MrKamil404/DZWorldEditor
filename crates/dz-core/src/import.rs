//! Importers for existing work: Terrain Builder folders, WRP worlds, DayZForests and Road Studio.
use crate::assets::AssetIndex;
use crate::model_info::ModelDb;
use crate::objects::{angles, from_terrain_builder, Obj, ObjectStore, FLAG_ABSOLUTE, FLAG_UNDERGROUND};
use crate::terrain::{parse_asc, Heightmap, MaskMap, RgbImage};
use crate::world::{default_layers, MapLine, RoadRoute, SurfaceLayer, World, WorldMeta};
use crate::wrp::Wrp;
use crate::Job;
use anyhow::{bail, ensure, Context, Result};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub fn read_asc(path: &Path) -> Result<crate::terrain::AscGrid> {
    parse_asc(&std::fs::read(path).with_context(|| format!("{}", path.display()))?).with_context(|| format!("{}", path.display()))
}

/// Mask picture to layer indices; colours not in `layers` are added as new layers.
pub fn mask_from_image(img: &RgbImage, layers: &mut Vec<SurfaceLayer>, job: &Job) -> Result<MaskMap> {
    let mut map: HashMap<[u8; 3], u8> = HashMap::new();
    for (i, l) in layers.iter().enumerate() {
        map.insert(l.color, i as u8);
    }
    let mut mask = MaskMap::new(img.width, img.height, 0);
    let mut unknown: HashMap<[u8; 3], usize> = HashMap::new();
    for p in img.data.chunks_exact(3) {
        let c = [p[0], p[1], p[2]];
        if !map.contains_key(&c) {
            *unknown.entry(c).or_default() += 1;
        }
    }
    // Colours present in meaningful amounts become layers; the rest are blends or noise.
    let mut extra: Vec<_> = unknown.into_iter().filter(|(_, n)| *n > img.width * img.height / 20000).collect();
    extra.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (c, _) in extra {
        if layers.len() >= 255 {
            job.log("More than 255 mask colours; remaining ones map to the nearest layer.");
            break;
        }
        map.insert(c, layers.len() as u8);
        let n = layers.len();
        let base = default_layers();
        let t = &base[n % base.len()];
        layers.push(SurfaceLayer { name: format!("layer_{n}"), texture: t.texture.clone(), material: t.material.clone(), color: c });
    }
    let palette: Vec<[u8; 3]> = layers.iter().map(|l| l.color).collect();
    mask.data.par_iter_mut().zip(img.data.par_chunks_exact(3)).for_each(|(out, p)| {
        let c = [p[0], p[1], p[2]];
        *out = match map.get(&c) {
            Some(v) => *v,
            None => palette.iter().enumerate().min_by_key(|(_, q)| (0..3).map(|k| (q[k] as i32 - c[k] as i32).pow(2)).sum::<i32>()).map(|(i, _)| i as u8).unwrap_or(0),
        };
    });
    Ok(mask)
}

/// Where a Terrain Builder object has its ground origin. Terrain Builder keeps the centre of a
/// tilted model over its position only horizontally, so what the model's height above its
/// origin leans away is not part of the position it stores.
pub fn tb_origin(x: f32, z: f32, yaw: f32, pitch: f32, roll: f32, scale: f32, center_y: f32) -> (f32, f32) {
    if pitch == 0.0 && roll == 0.0 {
        return (x, z);
    }
    let up = crate::objects::rotation(yaw, pitch, roll, scale)[1];
    (x - up[0] * center_y, z - up[2] * center_y)
}

/// Parses a Terrain Builder object list.
pub fn parse_tb_objects(text: &str, w: &mut World, assets: &AssetIndex, db: &ModelDb, absolute: bool) -> (Vec<Obj>, Vec<String>) {
    let (ex, ey) = (w.meta.easting as f32, w.meta.northing as f32);
    let mut out = Vec::new();
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();
    let (mut sum_x, mut count) = (0.0f64, 0usize);
    let rows: Vec<Vec<&str>> = text.lines().map(|l| l.split(';').map(str::trim).collect::<Vec<_>>()).filter(|f| f.len() >= 8).collect();
    for f in rows.iter().take(1000) {
        if let Ok(x) = f[1].parse::<f64>() {
            sum_x += x;
            count += 1;
        }
    }
    // Lists exported relative to the map origin have no easting to remove.
    let size = w.meta.size_m() as f64;
    let shift = if count > 0 && sum_x / count as f64 <= size { (0.0, 0.0) } else { (ex, ey) };
    for f in rows {
        let name = f[0].trim_matches('"');
        let v = |i: usize| f.get(i).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
        let Some(path) = assets.resolve(name) else {
            *missing.entry(name.to_string()).or_default() += 1;
            continue;
        };
        let path = path.to_string();
        let model = w.objects.intern(&path);
        let mut flags = 0;
        if absolute {
            flags |= FLAG_ABSOLUTE;
        }
        if v(8) != 0.0 {
            flags |= FLAG_UNDERGROUND;
        }
        let (yaw, pitch, roll) = from_terrain_builder(v(3), v(4), v(5));
        let scale = if v(6) > 0.0 { v(6) } else { 1.0 };
        let (x, z) = tb_origin(v(1) - shift.0, v(2) - shift.1, yaw, pitch, roll, scale, db.get(&path).center[1]);
        out.push(Obj { x, z, yaw, pitch, roll, scale, elevation: v(7), model, flags });
    }
    (out, missing.into_iter().map(|(k, n)| format!("{k} ({n})")).collect())
}

fn objects_from_wrp(wrp: &Wrp, height: &Heightmap, sea: f32, db: &ModelDb) -> ObjectStore {
    let mut store = ObjectStore::default();
    for o in &wrp.objects {
        let path = ObjectStore::normalize_path(&o.model);
        let info = db.get(&path);
        let m3 = [[o.matrix[0], o.matrix[1], o.matrix[2]], [o.matrix[3], o.matrix[4], o.matrix[5]], [o.matrix[6], o.matrix[7], o.matrix[8]]];
        let (yaw, pitch, roll, scale) = angles(&m3);
        let c = info.center;
        let off = |k: usize| m3[0][k] * c[0] + m3[1][k] * c[1] + m3[2][k] * c[2];
        let (x, y, z) = (o.matrix[9] - off(0), o.matrix[10] - off(1) + sea, o.matrix[11] - off(2));
        let ground = height.sample(x, z);
        let model = store.intern(&path);
        let flags = if o.underground { FLAG_UNDERGROUND } else { 0 };
        store.insert(Obj { x, z, elevation: y - ground, yaw, pitch, roll, scale, model, flags });
    }
    store
}

/// A Terrain Builder layer found on disk.
#[derive(Debug, Clone)]
pub struct LayerFile {
    pub path: PathBuf,
    pub info: crate::tv4l::LayerInfo,
    pub import: bool,
}

/// Everything below a folder that looks like part of a map, best guesses first.
#[derive(Debug, Default, Clone)]
pub struct FolderScan {
    pub wrp: Vec<PathBuf>,
    pub asc: Vec<PathBuf>,
    pub satellite: Vec<PathBuf>,
    pub mask: Vec<PathBuf>,
    pub layers_cfg: Vec<PathBuf>,
    pub holes_cfg: Vec<PathBuf>,
    pub objects: Vec<PathBuf>,
    pub tv4p: Vec<PathBuf>,
    pub tile_dir: Option<PathBuf>,
    pub libraries: Vec<PathBuf>,
    pub layers: Vec<LayerFile>,
    pub shapes: Vec<ShapeFile>,
}

/// A shape layer found on disk.
#[derive(Debug, Clone)]
pub struct ShapeFile {
    pub path: PathBuf,
    pub name: String,
    pub shapes: usize,
    pub import: bool,
}

impl ShapeFile {
    pub fn read(path: &Path) -> Option<Self> {
        let l = crate::shapes::load(path).ok()?;
        Some(Self { path: path.to_path_buf(), name: l.name, shapes: l.shapes.len(), import: true })
    }
}

fn walk(root: &Path, s: &mut FolderScan, found: &mut HashMap<PathBuf, (usize, u64)>) {
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if p.is_dir() {
                if name == "layers" && (p.join("S_000_000_lco.paa").is_file() || p.join("S_000_000_lco.png").is_file()) {
                    s.tile_dir.get_or_insert(p.clone());
                } else if depth < 5 && ![".cache", ".layersbackup", ".shapesbackup", ".git"].iter().any(|x| name.ends_with(x)) {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            let list = if name.ends_with(".wrp") {
                &mut s.wrp
            } else if name.ends_with(".asc") {
                &mut s.asc
            } else if name == "layers.cfg" {
                &mut s.layers_cfg
            } else if name == "holes.cfg" {
                &mut s.holes_cfg
            } else if name.ends_with(".tv4p") {
                &mut s.tv4p
            } else if name.ends_with(".tml") {
                &mut s.libraries
            } else if name.ends_with(".tv4s") || name.ends_with(".shp") {
                if let Some(f) = ShapeFile::read(&p) {
                    s.shapes.push(f);
                }
                continue;
            } else if name.ends_with(".tv4l") {
                match crate::tv4l::read_info(&p) {
                    Ok(info) => {
                        let import = info.visible && info.objects > 0;
                        s.layers.push(LayerFile { path: p, info, import });
                    }
                    Err(_) => continue,
                }
                continue;
            } else if name.ends_with(".bmp") || name.ends_with(".png") || name.ends_with(".tif") {
                if name.contains("legend") || name.contains("preview") {
                    continue;
                }
                if name.contains("mask") {
                    &mut s.mask
                } else if name.contains("sat") {
                    &mut s.satellite
                } else {
                    continue;
                }
            } else if name.ends_with(".txt") {
                let first = std::fs::read(&p).ok().and_then(|b| b.split(|c| *c == b'\n').next().map(|l| String::from_utf8_lossy(l).to_string()));
                if first.is_some_and(|l| l.starts_with('"') && l.matches(';').count() >= 7) {
                    &mut s.objects
                } else {
                    continue;
                }
            } else {
                continue;
            };
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            found.insert(p.clone(), (depth, size));
            list.push(p);
        }
    }
}

/// How many neighbouring raised objects of one kind end up level with each other on a
/// heightmap. Elevations in layers are relative to the terrain, so deck after deck of a bridge
/// only lines up on the terrain the project was really built on.
fn level_pairs(raised: &[(u32, i32, f64, f64, f32)], asc: &crate::terrain::AscGrid) -> usize {
    let height = |o: &(u32, i32, f64, f64, f32)| asc.map.sample((o.2 - asc.xll) as f32, (o.3 - asc.yll) as f32) + o.4;
    raised.windows(2).filter(|w| {
        let (a, b) = (&w[0], &w[1]);
        a.0 == b.0 && a.1 == b.1 && (a.2 - b.2).hypot(a.3 - b.3) < 60.0 && (height(a) - height(b)).abs() < 0.05
    }).count()
}

/// Puts first the heightmap the objects of the layers fit, when several lie side by side.
fn choose_heightmap(s: &mut FolderScan) {
    if s.asc.len() < 2 {
        return;
    }
    let mut raised = Vec::new();
    for l in s.layers.iter().filter(|l| l.import) {
        let Ok(layer) = crate::tv4l::load(&l.path) else { continue };
        raised.extend(layer.objects.iter().filter(|o| o.elevation.abs() >= 0.5).map(|o| (o.template, (o.yaw * 10.0).round() as i32, o.x, o.z, o.elevation)));
    }
    raised.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)).then(a.2.total_cmp(&b.2)));
    let scores: Vec<usize> = s.asc.iter().map(|p| read_asc(p).map_or(0, |a| level_pairs(&raised, &a))).collect();
    let best = (0..scores.len()).max_by_key(|i| (scores[*i], std::cmp::Reverse(*i))).unwrap_or(0);
    if scores[best] > scores[0] {
        let chosen = s.asc.remove(best);
        s.asc.insert(0, chosen);
    }
}

/// Looks through a project folder (or the folder of a `.tv4p`). Files may be anywhere below
/// it: they are recognised by content and name, not by where the program expects them.
pub fn scan_folder(root: &Path) -> FolderScan {
    let root = if root.is_file() { root.parent().unwrap_or(root) } else { root };
    let mut s = FolderScan::default();
    let mut found = HashMap::new();
    walk(root, &mut s, &mut found);
    // Terrain Builder often lives in a subfolder with the heightmap one level up.
    if s.asc.is_empty() {
        if let Some(up) = root.parent().filter(|p| p.parent().is_some()) {
            if !s.tv4p.is_empty() || !s.layers.is_empty() {
                let mut wider = FolderScan::default();
                walk(up, &mut wider, &mut found);
                let seen: Vec<PathBuf> = s.layers.iter().map(|l| l.path.clone()).collect();
                wider.layers.retain(|l| !seen.contains(&l.path));
                s.asc = wider.asc;
                s.satellite.extend(wider.satellite.into_iter().filter(|p| !p.starts_with(root)));
                s.mask.extend(wider.mask.into_iter().filter(|p| !p.starts_with(root)));
                s.layers_cfg.extend(wider.layers_cfg.into_iter().filter(|p| !p.starts_with(root)));
                s.holes_cfg.extend(wider.holes_cfg.into_iter().filter(|p| !p.starts_with(root)));
                s.wrp.extend(wider.wrp.into_iter().filter(|p| !p.starts_with(root)));
                if s.libraries.is_empty() {
                    s.libraries = wider.libraries;
                }
                let known: Vec<PathBuf> = s.shapes.iter().map(|f| f.path.clone()).collect();
                s.shapes.extend(wider.shapes.into_iter().filter(|f| !known.contains(&f.path)));
            }
        }
    }
    // "<Map>Source" usually sits next to the built "<Map>".
    if s.wrp.is_empty() {
        let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let base = name.strip_suffix("Source").or(name.strip_suffix("source")).map(|b| b.trim_end_matches(['_', '-', ' ']).to_string());
        if let (Some(base), Some(parent)) = (base, root.parent()) {
            let built = parent.join(base);
            if built.is_dir() {
                let mut other = FolderScan::default();
                walk(&built, &mut other, &mut found);
                s.wrp = other.wrp;
                s.tile_dir = s.tile_dir.take().or(other.tile_dir);
            }
        }
    }
    // Holes belong to the built map, whether or not its WRP was already found.
    if s.holes_cfg.is_empty() {
        let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let built = name.strip_suffix("Source").or(name.strip_suffix("source")).and_then(|b| Some(root.parent()?.join(b.trim_end_matches(['_', '-', ' ']))));
        let beside_wrp = s.wrp.iter().map(|p| p.with_file_name("holes.cfg"));
        let in_built = built.iter().flat_map(|b| [b.join("world").join("holes.cfg"), b.join("holes.cfg")]);
        s.holes_cfg = beside_wrp.chain(in_built).filter(|p| p.is_file()).take(1).collect();
    }
    // Closest to the root first, then the largest.
    let rank = |p: &PathBuf| found.get(p).map(|(d, n)| (*d, std::cmp::Reverse(*n))).unwrap_or((usize::MAX, std::cmp::Reverse(0)));
    for list in [&mut s.wrp, &mut s.asc, &mut s.satellite, &mut s.mask, &mut s.layers_cfg, &mut s.holes_cfg, &mut s.tv4p] {
        list.sort_by_key(rank);
    }
    choose_heightmap(&mut s);
    // The project that owns the layers is the one with a `<name>.Layers` folder beside it.
    s.tv4p.sort_by_key(|p| !p.with_extension("Layers").is_dir());
    s.objects.sort();
    s.libraries.sort();
    s.layers.sort_by_key(|l| l.info.priority);
    // Terrain Builder's own shapes first; loose shapefiles are usually what they were made from.
    s.shapes.sort_by_key(|f| (f.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("shp")), f.name.to_ascii_lowercase()));
    if s.shapes.iter().any(|f| f.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("tv4s"))) {
        for f in s.shapes.iter_mut().filter(|f| f.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("shp"))) {
            f.import = false;
        }
    }
    s
}

/// What to bring in; starts from a [`FolderScan`] and is adjusted by the user.
#[derive(Debug, Default, Clone)]
pub struct ImportPlan {
    pub name: String,
    pub asc: Option<PathBuf>,
    pub wrp: Option<PathBuf>,
    pub objects_from_wrp: bool,
    pub tile_dir: Option<PathBuf>,
    pub satellite: Option<PathBuf>,
    pub mask: Option<PathBuf>,
    pub layers_cfg: Option<PathBuf>,
    pub holes_cfg: Option<PathBuf>,
    pub tv4p: Option<PathBuf>,
    pub libraries: Vec<PathBuf>,
    pub layers: Vec<LayerFile>,
    pub lists: Vec<(PathBuf, bool)>,
    pub shapes: Vec<ShapeFile>,
    /// Most surfaces one texture tile may use.
    pub max_tile_surfaces: u8,
}

impl ImportPlan {
    pub fn from_scan(root: &Path, scan: &FolderScan) -> Self {
        let dir = if root.is_file() { root.parent().unwrap_or(root) } else { root };
        let name = dir.file_name().map(|s| s.to_string_lossy().trim_end_matches("Source").trim_end_matches(['_', '-', ' ']).to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "Imported".into());
        let asc = scan.asc.first().cloned();
        let has_layers = scan.layers.iter().any(|l| l.import);
        // A built map has its surfaces in the tiles; loose pictures only belong to a source.
        let source = asc.is_some() || scan.tile_dir.is_none();
        Self {
            name,
            wrp: scan.wrp.first().cloned(),
            objects_from_wrp: !has_layers && scan.objects.is_empty(),
            tile_dir: scan.tile_dir.clone(),
            satellite: scan.satellite.first().filter(|_| source).cloned(),
            mask: scan.mask.first().filter(|_| source).cloned(),
            layers_cfg: scan.layers_cfg.first().cloned(),
            holes_cfg: scan.holes_cfg.first().cloned(),
            tv4p: scan.tv4p.first().cloned(),
            libraries: scan.libraries.clone(),
            layers: scan.layers.clone(),
            max_tile_surfaces: 6,
            shapes: scan.shapes.clone(),
            // Lists are usually exports of the same layers.
            lists: scan.objects.iter().map(|p| (p.clone(), !has_layers)).collect(),
            asc,
        }
    }
    pub fn can_import(&self) -> bool {
        self.asc.is_some() || self.wrp.is_some()
    }
}

/// Rebuilds satellite and mask from the generated tiles of a built map.
fn stitch_tiles(wrp: &Wrp, dir: &Path, job: &Job) -> Result<(RgbImage, MaskMap, Vec<SurfaceLayer>, f32)> {
    let side = wrp.land_x as usize / 8;
    let parse_mat = |m: &str| -> Option<(usize, usize, Vec<Option<u32>>)> {
        let file = m.rsplit('\\').next()?.strip_suffix(".rvmat")?;
        let mut parts = file.split('_');
        parts.next()?;
        let (c, r) = parts.next()?.split_once('-')?;
        let slots = parts.map(|p| p.strip_prefix('L').and_then(|n| n.parse().ok())).collect();
        Some((c.parse().ok()?, r.parse().ok()?, slots))
    };
    // Tile slots: union of all per-cell variants of the tile.
    let mut tile_slots: HashMap<(usize, usize), Vec<Option<u32>>> = HashMap::new();
    let mut sample_rvmat = None;
    for m in &wrp.materials {
        let Some((c, r, s)) = parse_mat(m) else { continue };
        let e = tile_slots.entry((c, r)).or_insert_with(|| vec![None; s.len()]);
        for (i, v) in s.iter().enumerate() {
            if v.is_some() && i < e.len() {
                e[i] = *v;
            }
        }
        if sample_rvmat.is_none() {
            sample_rvmat = m.rsplit('\\').next().map(|f| dir.join(f));
        }
    }
    let mat_text = std::fs::read_to_string(sample_rvmat.context("Built map has no materials")?)?;
    let six = mat_text.contains("\"TerrainX\"");
    let num_after = |key: &str| -> Option<f32> {
        let line = mat_text.lines().find(|l| l.contains(key))?;
        line.split('{').nth(1)?.split(',').next()?.trim().parse().ok()
    };
    let scale = num_after("aside[]").context("Tile material without a transform")?;
    let tile_m = 1.0 / scale;
    let size = wrp.land_x as f32 * wrp.land_cell;
    let step = wrp.land_cell * 8.0;
    let margin = (tile_m - step) * 0.5;

    // Layer identity from the materials: slot textures.
    let mut layers: BTreeMap<u32, SurfaceLayer> = BTreeMap::new();
    for m in &wrp.materials {
        let Some((_, _, s)) = parse_mat(m) else { continue };
        if s.iter().flatten().all(|l| layers.contains_key(l)) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(dir.join(m.rsplit('\\').next().unwrap())) else { continue };
        let lines: Vec<&str> = text.lines().collect();
        for (k, slot) in s.iter().enumerate() {
            let Some(l) = slot else { continue };
            let class = format!("class Stage{}", 4 + k * 2);
            let Some(at) = lines.iter().position(|t| t.trim() == class) else { continue };
            let Some(tex) = lines[at + 2].split('"').nth(1) else { continue };
            let name = tex.rsplit('\\').next().unwrap_or(tex).trim_end_matches("_ca.paa").to_string();
            let c = forest_core::species::hsv_to_rgb(layers.len() as f64 * 67.0 % 360.0, 0.5, 0.72);
            layers.entry(*l).or_insert(SurfaceLayer { name, texture: tex.to_string(), material: tex.replace("_ca.paa", ".rvmat"), color: c });
        }
    }
    ensure!(!layers.is_empty(), "Could not read terrain layers from the materials");
    let layer_slot: HashMap<u32, u8> = layers.keys().enumerate().map(|(i, l)| (*l, i as u8)).collect();
    let layers_out: Vec<SurfaceLayer> = layers.into_values().collect();

    let probe = read_tile(&dir.join("S_000_000_lco"))?;
    let tile_px = probe.0;
    let mpp = tile_m / tile_px as f32;
    let px = (size / mpp).round() as usize;
    let mut sat = RgbImage::new(px, px, [0, 0, 0]);
    let mut mask = MaskMap::new(px, px, 0);
    let step_px = (step / mpp).round() as usize;
    let margin_px = (margin / mpp).round() as usize;
    let results: Vec<_> = (0..side * side).into_par_iter().map(|t| -> Result<_> {
        let (r, c) = (t / side, t % side);
        let s = read_tile(&dir.join(format!("S_{c:03}_{r:03}_lco")))?;
        let m = read_tile(&dir.join(format!("M_{c:03}_{r:03}_lca")))?;
        Ok((c, r, s, m))
    }).collect();
    for (n, res) in results.into_iter().enumerate() {
        job.set_progress(0.3 + 0.4 * n as f32 / (side * side) as f32);
        let (c, r, s, m) = res?;
        let slots = tile_slots.get(&(c, r)).cloned().unwrap_or_default();
        let count = slots.len();
        for y in 0..step_px {
            let ty = y + margin_px;
            if ty >= s.0 { break; }
            let oy = r * step_px + y;
            if oy >= px { break; }
            for x in 0..step_px {
                let tx = x + margin_px;
                if tx >= s.0 { break; }
                let ox = c * step_px + x;
                if ox >= px { break; }
                let sp = (ty * s.0 + tx) * 4;
                sat.data[(oy * px + ox) * 3..(oy * px + ox) * 3 + 3].copy_from_slice(&s.1[sp..sp + 3]);
                let mp = (ty * m.0 + tx) * 4;
                let p = &m.1[mp..mp + 4];
                // The strongest channel selects the slot.
                let slot = if six && p[2] >= 128 && p[2] >= p[0] && p[2] >= p[1] { if p[3] >= 192 { 3 } else if p[3] >= 64 { 4 } else { 5 } }
                    else if !six && p[3] < 200 && count >= 5 && p[0].max(p[1]).max(p[2]) < 128 { 4 }
                    else if p[0] >= 128 && p[0] >= p[1] && p[0] >= p[2] { 1 }
                    else if p[1] >= 128 && p[1] >= p[2] { 2 }
                    else if p[2] >= 128 { 3 } else { 0 };
                let layer = slots.get(slot).copied().flatten().or_else(|| slots.iter().flatten().next().copied());
                mask.data[oy * px + ox] = layer.and_then(|l| layer_slot.get(&l).copied()).unwrap_or(0);
            }
        }
    }
    Ok((sat, mask, layers_out, step))
}

/// RGBA of a tile stored as .paa or .png; returns (side, pixels).
fn read_tile(stem: &Path) -> Result<(usize, Vec<u8>)> {
    let paa = stem.with_extension("paa");
    let name = stem.file_name().unwrap().to_string_lossy().to_string();
    let paa = paa.with_file_name(format!("{name}.paa"));
    let png = stem.with_file_name(format!("{name}.png"));
    if png.is_file() {
        let i = image::open(&png)?.into_rgba8();
        return Ok((i.width() as usize, i.into_raw()));
    }
    let t = forest_core::paa::decode(&std::fs::read(&paa).with_context(|| format!("{}", paa.display()))?, 8192)?;
    Ok((t.width as usize, t.rgba))
}

/// Opens a source WRP, with its generated tiles when they are next to it.
pub fn import_wrp(path: &Path, tile_dir: Option<&Path>, db: &ModelDb, job: &Job) -> Result<World> {
    job.stage("Reading WRP");
    let wrp = Wrp::parse(&std::fs::read(path).with_context(|| format!("{}", path.display()))?)?;
    ensure!(wrp.terrain_x == wrp.terrain_y, "Only square terrains are supported");
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Imported".into());
    let mut meta = WorldMeta::new(&name, wrp.terrain_x, wrp.cell());
    meta.write_underground_flag = wrp.newer_format;
    meta.cells_per_tile = (wrp.terrain_x / wrp.land_x).max(1);
    if let Some(first) = wrp.materials.first() {
        if let Some(root) = first.split('\\').next() {
            job.log(format!("Map folder in materials: {root}"));
        }
    }
    let height = Heightmap { cols: wrp.terrain_x as usize, rows: wrp.terrain_y as usize, cell: wrp.cell(), data: wrp.elevations.clone() };
    let tiles = tile_dir.map(PathBuf::from).or_else(|| {
        let d = path.parent()?.parent()?.join("data").join("layers");
        d.join("S_000_000_lco.paa").is_file().then_some(d)
    });
    let (satellite, mask, layers) = match tiles {
        Some(d) => {
            job.stage("Rebuilding satellite and mask from tiles");
            match stitch_tiles(&wrp, &d, job) {
                Ok((s, m, l, _)) => (Some(s), m, l),
                Err(e) => {
                    job.log(format!("Tiles could not be read: {e:#}"));
                    let px = (meta.size_m() as usize).clamp(1024, 20480);
                    (None, MaskMap::new(px, px, 0), default_layers())
                }
            }
        }
        None => {
            let px = (meta.size_m() as usize).clamp(1024, 20480);
            (None, MaskMap::new(px, px, 0), default_layers())
        }
    };
    job.stage(format!("Placing {} objects", wrp.objects.len()));
    let objects = objects_from_wrp(&wrp, &height, 0.0, db);
    Ok(World::from_parts(meta, height, mask, layers, satellite, objects, Vec::new()))
}

fn world_from_asc(asc: &Path, plan: &ImportPlan, job: &Job) -> Result<World> {
    job.stage("Reading heightmap");
    let asc = read_asc(asc)?;
    ensure!(asc.map.cols == asc.map.rows, "Only square heightmaps are supported");
    let mut meta = WorldMeta::new(&plan.name, asc.map.cols as u32, asc.map.cell);
    if asc.xll != 0.0 {
        meta.easting = asc.xll;
    }
    meta.northing = asc.yll;
    let px = (meta.size_m() as usize).clamp(1024, 20480);
    let base = asc.map;
    let w = World::from_parts(meta, base, MaskMap::new(px, px, 0), default_layers(), None, ObjectStore::default(), Vec::new());
    Ok(w)
}

/// Replaces satellite, mask and surfaces with the pictures chosen in the plan.
fn apply_pictures(w: &mut World, plan: &ImportPlan, job: &Job) -> Result<()> {
    if let Some(p) = &plan.layers_cfg {
        if plan.asc.is_some() || plan.mask.is_some() {
            w.layers = crate::layers_cfg::parse(&String::from_utf8_lossy(&std::fs::read(p)?))?;
        }
    }
    if let Some(p) = &plan.satellite {
        job.stage(format!("Reading {}", p.display()));
        w.satellite = Some(RgbImage::load(p)?);
    }
    if let Some(p) = &plan.mask {
        job.stage(format!("Reading {}", p.display()));
        let img = RgbImage::load(p)?;
        let mut layers = std::mem::take(&mut w.layers);
        w.mask = mask_from_image(&img, &mut layers, job)?;
        w.layers = layers;
    }
    Ok(())
}

/// Removes the map-frame offset: known easting first, then the usual ones.
fn frame_offset(all: &[crate::tv4l::TbObject], meta: &WorldMeta) -> Option<(f64, f64)> {
    let size = meta.size_m() as f64;
    let step = (all.len() / 2000).max(1);
    let sample: Vec<_> = all.iter().step_by(step).collect();
    let inside = |ex: f64, ey: f64| {
        let n = sample.iter().filter(|o| (0.0..=size).contains(&(o.x - ex)) && (0.0..=size).contains(&(o.z - ey))).count();
        n as f64 >= sample.len() as f64 * 0.9
    };
    [(meta.easting, meta.northing), (200_000.0, 0.0), (0.0, 0.0), (meta.easting, 0.0)].into_iter().find(|(x, y)| inside(*x, *y))
}

/// Adds Terrain Builder layers to the world, each as an object layer of its own.
pub fn add_tb_layers(w: &mut World, files: &[LayerFile], libraries: &[PathBuf], db: &ModelDb, job: &Job) -> Result<usize> {
    job.stage("Reading template libraries");
    let templates = crate::tv4l::read_libraries(libraries);
    job.log(format!("{} templates in {} libraries", templates.len(), libraries.len()));
    job.stage("Reading Terrain Builder layers");
    let read: Vec<Result<crate::tv4l::Layer>> = files.par_iter().map(|f| crate::tv4l::load(&f.path)).collect();
    let mut layers = Vec::with_capacity(read.len());
    for (f, r) in files.iter().zip(read) {
        match r {
            Ok(l) => layers.push(l),
            Err(e) => job.log(format!("{}: {e:#}", f.path.display())),
        }
    }
    ensure!(!layers.is_empty() || files.is_empty(), "None of the selected layers could be read");
    let joined: Vec<crate::tv4l::TbObject> = layers.iter().flat_map(|l| l.objects.iter().step_by((l.objects.len() / 2000).max(1)).copied()).collect();
    let (ex, ey) = match frame_offset(&joined, &w.meta) {
        Some(o) => o,
        None => {
            job.log("Layer coordinates do not match the map frame; check the easting and northing in the project settings.");
            (w.meta.easting, w.meta.northing)
        }
    };
    let total: usize = layers.iter().map(|l| l.objects.len()).sum();
    let mut done = 0usize;
    let mut unknown: HashMap<u32, usize> = HashMap::new();
    for l in layers {
        job.stage(format!("Placing {} ({} objects)", l.info.name, l.objects.len()));
        // Terrain Builder has a "default" layer of its own; it takes over the empty first one.
        let reuse = l.info.name.eq_ignore_ascii_case(&w.object_layers[0].name) && !w.objects.iter().any(|(_, o)| o.layer() == 0);
        let layer = if reuse { 0 } else { w.add_object_layer(&l.info.name) };
        let meta = &mut w.object_layers[layer as usize];
        meta.visible = l.info.visible;
        meta.editable = !l.info.locked;
        meta.export = l.info.visible;
        let mut model_of: HashMap<u32, Option<u32>> = HashMap::new();
        for o in &l.objects {
            let model = *model_of.entry(o.template).or_insert_with(|| templates.get(&o.template).map(|p| w.objects.intern(p)));
            let Some(model) = model else {
                *unknown.entry(o.template).or_default() += 1;
                continue;
            };
            let (yaw, pitch, roll) = from_terrain_builder(o.yaw, o.pitch, o.roll);
            let scale = if o.scale > 0.0 { o.scale } else { 1.0 };
            let center_y = if pitch == 0.0 && roll == 0.0 { 0.0 } else { db.get(&w.objects.models[model as usize].path).center[1] };
            let (x, z) = tb_origin((o.x - ex) as f32, (o.z - ey) as f32, yaw, pitch, roll, scale, center_y);
            let mut obj = Obj { x, z, elevation: o.elevation, yaw, pitch, roll, scale, model, flags: 0 };
            obj.set_layer(layer);
            w.objects.insert(obj);
        }
        done += l.objects.len();
        job.set_progress(0.5 + 0.5 * done as f32 / total.max(1) as f32);
    }
    if !unknown.is_empty() {
        let lost: usize = unknown.values().sum();
        job.log(format!("{lost} objects of {} templates are missing from the libraries and were skipped", unknown.len()));
    }
    Ok(total)
}

/// Template libraries that belong to the project of a layer file.
pub fn libraries_near(layer: &Path) -> Vec<PathBuf> {
    layer.ancestors().skip(1).take(4).map(|d| d.join("TemplateLibs")).find(|d| d.is_dir()).map(|d| crate::tv4l::libraries_in(&d)).unwrap_or_default()
}

/// Builds a world according to the plan.
pub fn import_plan(plan: &ImportPlan, assets: &AssetIndex, db: &ModelDb, job: &Job) -> Result<World> {
    let mut w = match (&plan.asc, &plan.wrp) {
        (Some(asc), _) => world_from_asc(asc, plan, job)?,
        (None, Some(wrp)) => {
            let mut w = import_wrp(wrp, plan.tile_dir.as_deref(), db, job)?;
            w.objects = ObjectStore::default();
            w.meta.name = plan.name.clone();
            w
        }
        (None, None) => bail!("Choose a heightmap (.asc) or a world (.wrp)"),
    };
    w.meta.max_tile_surfaces = plan.max_tile_surfaces.clamp(1, 6);
    apply_pictures(&mut w, plan, job)?;
    if let Some(p) = &plan.holes_cfg {
        let text = std::fs::read(p).with_context(|| format!("{}", p.display()))?;
        let (cols, rows) = (w.height.cols as u32, w.height.rows as u32);
        w.holes = crate::holes::parse(&String::from_utf8_lossy(&text)).into_iter().filter(|t| t[0] < cols && t[1] < rows).collect();
        job.log(format!("{}: {} terrain holes", p.display(), w.holes.len()));
    }
    let over = crate::mask_limit::scan(&w).len();
    if over > 0 {
        job.log(format!("{over} texture tiles use more than {} surfaces; Project settings can check and fix them", w.meta.max_tile_surfaces));
    }
    if plan.objects_from_wrp {
        if let Some(path) = &plan.wrp {
            job.stage("Reading WRP objects");
            let wrp = Wrp::parse(&std::fs::read(path).with_context(|| format!("{}", path.display()))?)?;
            w.objects = objects_from_wrp(&wrp, &w.height, 0.0, db);
            job.log(format!("{}: {} objects", path.display(), w.objects.len()));
        }
    }
    let chosen: Vec<LayerFile> = plan.layers.iter().filter(|l| l.import).cloned().collect();
    if !chosen.is_empty() {
        add_tb_layers(&mut w, &chosen, &plan.libraries, db, job)?;
    }
    for (p, on) in &plan.lists {
        if !*on {
            continue;
        }
        job.stage(format!("Reading objects {}", p.display()));
        let text = String::from_utf8_lossy(&std::fs::read(p)?).into_owned();
        let (list, missing) = parse_tb_objects(&text, &mut w, assets, db, false);
        job.log(format!("{}: {} objects", p.display(), list.len()));
        if !missing.is_empty() {
            job.log(format!("Unknown models: {}", missing.join(", ")));
        }
        let layer = w.add_object_layer(&p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Objects".into()));
        for mut o in list {
            o.set_layer(layer);
            w.objects.insert(o);
        }
    }
    w.road_project = plan.tv4p.clone();
    w.template_libraries = plan.libraries.clone();
    if let Some(p) = &plan.tv4p {
        job.stage("Placing roads");
        // A built world already holds the road parts as objects.
        let layer = (!plan.objects_from_wrp).then(|| w.add_object_layer("Roads"));
        match import_tv4p(p, &mut w, layer) {
            Ok(n) => job.log(format!("{}: {n} roads", p.display())),
            Err(e) => job.log(format!("{}: {e:#}", p.display())),
        }
    }
    let mut files = Vec::new();
    for f in plan.shapes.iter().filter(|f| f.import) {
        match crate::shapes::load(&f.path) {
            Ok(l) => files.push(l),
            Err(e) => job.log(format!("{e:#}")),
        }
    }
    if !files.is_empty() {
        job.stage("Reading shapes");
        let (n, warning) = crate::shapes::add_layers(&mut w, files);
        job.log(format!("{n} shapes"));
        if let Some(text) = warning {
            job.log(text);
        }
    }
    Ok(w)
}

/// Builds a project from a folder using the best guess for every part.
pub fn import_folder(root: &Path, assets: &AssetIndex, db: &ModelDb, job: &Job) -> Result<World> {
    job.stage("Looking through the folder");
    let scan = scan_folder(root);
    let plan = ImportPlan::from_scan(root, &scan);
    ensure!(plan.can_import(), "No heightmap (.asc) or world (.wrp) found in {}", root.display());
    import_plan(&plan, assets, db, job)
}

/// Road parts lying on a line that no route owns.
pub fn parts_along(w: &World, points: &[[f32; 2]]) -> Vec<u32> {
    let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in points {
        (x0, z0, x1, z1) = (x0.min(p[0]), z0.min(p[1]), x1.max(p[0]), z1.max(p[1]));
    }
    let distance = |x: f32, z: f32| points.windows(2).map(|s| {
        let (dx, dz) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
        let k = (((x - s[0][0]) * dx + (z - s[0][1]) * dz) / (dx * dx + dz * dz).max(1e-3)).clamp(0.0, 1.0);
        ((s[0][0] + dx * k - x).powi(2) + (s[0][1] + dz * k - z).powi(2)).sqrt()
    }).fold(f32::MAX, f32::min);
    let taken: std::collections::BTreeSet<u32> = w.routes.iter().flat_map(|r| r.objects.iter().copied()).collect();
    let mut ids = Vec::new();
    w.objects.for_each_in(x0 - 3.0, z0 - 3.0, x1 + 3.0, z1 + 3.0, |id, o| {
        if !taken.contains(&id) && w.objects.model_path(o).contains("\\roads\\parts\\") && distance(o.x, o.z) < 1.5 {
            ids.push(id);
        }
    });
    ids
}

/// Joins pieces of line that touch end to end, carrying what belongs to each piece.
fn chain_pieces(pieces: Vec<(Vec<[f32; 2]>, Vec<u32>)>) -> Vec<(Vec<[f32; 2]>, Vec<u32>)> {
    const TOUCH: f32 = 0.6;
    let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < TOUCH && (a[1] - b[1]).abs() < TOUCH;
    let mut chains: Vec<(Vec<[f32; 2]>, Vec<u32>)> = Vec::new();
    for (mut line, ids) in pieces {
        if line.len() < 2 {
            continue;
        }
        let mut placed = false;
        for (chain, owned) in chains.iter_mut() {
            let (start, end) = (chain[0], *chain.last().unwrap());
            let (a, b) = (line[0], *line.last().unwrap());
            if near(end, a) || near(end, b) {
                if near(end, b) { line.reverse(); }
                chain.extend(line.iter().skip(1));
            } else if near(start, a) || near(start, b) {
                if near(start, a) { line.reverse(); }
                let mut joined = line.clone();
                joined.extend(chain.iter().skip(1));
                *chain = joined;
            } else {
                continue;
            }
            owned.extend(&ids);
            placed = true;
            break;
        }
        if !placed {
            chains.push((line, ids));
        }
    }
    chains
}

fn road_route(name: String, family: String, centerline: Vec<[f32; 2]>, objects: Vec<u32>) -> RoadRoute {
    // Terrain Builder roads may turn tighter than new ones are allowed to.
    let settings = dayz_road_tool::document::RoutingSettings { min_radius: 10.0, ..Default::default() };
    RoadRoute { name, family, points: crate::gen::simplify_line(&centerline, 2.0), centerline, objects, id: crate::library::new_id(), decks: Vec::new(), grading: Default::default(), profile: Vec::new(), terrain_route: false, settings, grade: false, grade_width: 9.0, grade_falloff: 12.0, modifier: None }
}

/// Turns the road lines of projects saved by older versions into routes, and other lines
/// into polylines.
pub fn adopt_old_lines(w: &mut World, lines: Vec<MapLine>) {
    use crate::shapes::{MapShape, ShapeKind, ShapeLayer};
    if lines.is_empty() {
        return;
    }
    let (roads, plain): (Vec<_>, Vec<_>) = lines.into_iter().partition(|l| !parts_along(w, &l.points).is_empty());
    let pieces = roads.into_iter().map(|l| (l.points, Vec::new())).collect();
    for (k, (points, _)) in chain_pieces(pieces).into_iter().enumerate() {
        let objects = parts_along(w, &points);
        let family = objects.first().and_then(|id| w.objects.get(*id)).map(|o| dayz_road_tool::road_colors::family(w.objects.model_path(o))).unwrap_or_default();
        w.routes.push(road_route(format!("Road {}", k + 1), family, points, objects));
    }
    if !plain.is_empty() {
        let shapes = plain.into_iter().filter(|l| l.points.len() >= 2).map(|l| MapShape { kind: ShapeKind::Polyline, points: l.points, attributes: vec![("NAME".to_string(), l.name)] }).collect();
        w.shape_layers.push(ShapeLayer { name: "Lines".into(), visible: true, shapes });
    }
}

/// Roads of a Terrain Builder project as editable routes. With `layer` the road parts are
/// placed as objects on it; without, the routes take over parts already on the map.
/// Returns the number of routes.
pub fn import_tv4p(path: &Path, w: &mut World, layer: Option<u16>) -> Result<usize> {
    use dayz_road_tool::{geometry::Library, tv4p};
    let mut lib = Library::new(PathBuf::from("P:\\dz\\structures\\roads\\parts"));
    let project = tv4p::load(path, &mut lib)?;
    let (ex, ey) = (w.meta.easting, w.meta.northing);
    crate::keypoints::merge(w, crate::keypoints::parse_tv4p(&std::fs::read(path)?, [ex, ey]));
    let mut n = 0;
    for road in &project.roads {
        let lines: Vec<Vec<[f32; 2]>> = road.shape.lines.iter().map(|l| l.iter().map(|p| [(p[0] - ex) as f32, (p[1] - ey) as f32]).collect()).collect();
        let mut placed = Vec::with_capacity(road.shape.placements.len());
        if let Some(layer) = layer {
            let is_cap = |k: usize| road.shape.placements[k].0.to_string_lossy().to_ascii_lowercase().contains("konec");
            for (k, (file, angle, at)) in road.shape.placements.iter().enumerate() {
                let path = ObjectStore::normalize_path(&file.to_string_lossy());
                let path = path.strip_prefix("p:\\").unwrap_or(&path).to_string();
                let model = w.objects.intern(&path);
                // A road closes with a cap facing the other way than the one it opens with.
                let closes = is_cap(k) && k > 0 && !is_cap(k - 1);
                let (at, yaw) = if closes { crate::gen::closing_cap(*at, -angle.to_degrees()) } else { (*at, -angle.to_degrees()) };
                let yaw = yaw.rem_euclid(360.0) as f32;
                let mut o = Obj { x: (at[0] - ex) as f32, z: (at[1] - ey) as f32, elevation: 0.0, yaw, pitch: 0.0, roll: 0.0, scale: 1.0, model, flags: 0 };
                o.set_layer(layer);
                placed.push(w.objects.insert(o));
            }
        }
        let matched = lines.len() == placed.len();
        let pieces: Vec<_> = lines.into_iter().enumerate().map(|(i, l)| (l, if matched { vec![placed[i]] } else { Vec::new() })).collect();
        let chains = chain_pieces(pieces);
        let many = chains.len() > 1;
        let family = dayz_road_tool::road_colors::family(&road.model).to_ascii_lowercase();
        for (k, (points, mut objects)) in chains.into_iter().enumerate() {
            if layer.is_none() {
                objects = parts_along(w, &points);
            } else if !matched && k == 0 {
                objects = placed.clone();
            }
            let name = if many { format!("Road {} ({})", road.id, k + 1) } else { format!("Road {}", road.id) };
            w.routes.push(road_route(name, family.clone(), points, objects));
            n += 1;
        }
    }
    Ok(n)
}

/// Objects of a DayZForests project: its side file, or an exported object list.
pub fn import_forest_project(path: &Path, w: &mut World, assets: &AssetIndex) -> Result<(usize, Vec<String>)> {
    let saved = forest_core::saved::load_sidecar(path).map_err(anyhow::Error::msg)?.context("This forest project has no saved objects; generate them in DayZForests or export an object list")?;
    let mut n = 0;
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();
    for p in saved.objects {
        let Some(m) = assets.resolve(&p.model) else {
            *missing.entry(p.model).or_default() += 1;
            continue;
        };
        let m = m.to_string();
        let model = w.objects.intern(&m);
        let (yaw, pitch, roll) = from_terrain_builder(p.yaw as f32, p.pitch as f32, p.roll as f32);
        w.objects.insert(Obj { x: p.x as f32, z: p.y as f32, elevation: p.elevation as f32, yaw, pitch, roll, scale: p.scale as f32, model, flags: 0 });
        n += 1;
    }
    Ok((n, missing.into_iter().map(|(k, c)| format!("{k} ({c})")).collect()))
}

/// Routes and placed parts of a Road Studio document.
pub fn import_road_document(path: &Path, w: &mut World) -> Result<usize> {
    let doc = dayz_road_tool::document::Document::load(path)?;
    let (ex, ey) = (w.meta.easting, w.meta.northing);
    let prefix = doc.models.to_string_lossy().trim_start_matches("P:\\").trim_start_matches("p:\\").replace('/', "\\");
    let mut n = 0;
    for route in &doc.routes {
        let points: Vec<[f32; 2]> = route.points.iter().map(|p| [(p[0] - ex) as f32, (p[1] - ey) as f32]).collect();
        let mut objects = Vec::new();
        for (k, p) in route.parts.iter().chain(&route.extras).enumerate() {
            let file = p.model.rsplit(['\\', '/']).next().unwrap_or(&p.model).to_string();
            let model = w.objects.intern(&format!("{prefix}\\{file}"));
            let (at, yaw) = if k < route.parts.len() { crate::gen::road_part_pose(&route.parts, k) } else { (p.position, p.rotation.rem_euclid(360.0) as f32) };
            objects.push(w.objects.insert(Obj { x: (at[0] - ex) as f32, z: (at[1] - ey) as f32, elevation: 0.0, yaw, pitch: 0.0, roll: 0.0, scale: 1.0, model, flags: 0 }));
            n += 1;
        }
        if points.len() >= 2 {
            w.routes.push(road_route(route.name.clone(), route.family.clone(), points, objects));
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::from_paths;
    use crate::export::{run, ExportOptions};

    #[test]
    fn the_heightmap_is_the_one_that_levels_a_bridge() {
        use crate::terrain::{AscGrid, Heightmap};
        let grid = |slope: f32| {
            let mut map = Heightmap::new(32, 32, 10.0, 5.0);
            for j in 0..32 { for i in 0..32 { map.set(i, j, 5.0 + i as f32 * slope); } }
            AscGrid { map, xll: 0.0, yll: 0.0 }
        };
        // Three decks 30 m apart, raised so their tops are level over ground rising 1 in 10.
        let raised = [(1, 0, 50.0, 50.0, 20.0f32), (1, 0, 80.0, 50.0, 17.0), (1, 0, 110.0, 50.0, 14.0)];
        assert_eq!(level_pairs(&raised, &grid(1.0)), 2);
        assert_eq!(level_pairs(&raised, &grid(0.0)), 0);
    }

    #[test]
    fn a_tilted_model_keeps_its_centre_over_the_stored_position() {
        // On its side, with the centre of the model 20 m above its origin.
        let (x, z) = tb_origin(100.0, 50.0, 0.0, 0.0, 90.0, 1.0, 20.0);
        let up = crate::objects::rotation(0.0, 0.0, 90.0, 1.0)[1];
        assert!((x + up[0] * 20.0 - 100.0).abs() < 1e-3 && (z + up[2] * 20.0 - 50.0).abs() < 1e-3);
        assert!((x - 100.0).abs() > 15.0);
        assert_eq!(tb_origin(100.0, 50.0, 33.0, 0.0, 0.0, 2.0, 20.0), (100.0, 50.0));
    }

    #[test]
    fn tb_list_with_easting_is_shifted() {
        let mut w = World::new(WorldMeta::new("A", 256, 8.0), 0.0);
        let a = from_paths(vec!["dz\\t_a.p3d".into()]);
        let (o, missing) = parse_tb_objects("\"t_a\";200100.5;50.0;10;0;0;1.5;0.0;1;\n\"nope\";200000;0;0;0;0;1;0;0;\n", &mut w, &a, &ModelDb::new("Z:/none"), false);
        assert_eq!(o.len(), 1);
        assert_eq!(missing, vec!["nope (1)".to_string()]);
        assert!((o[0].x - 100.5).abs() < 1e-2 && o[0].scale == 1.5);
        assert_eq!(o[0].flags, FLAG_UNDERGROUND);
    }

    #[test]
    fn tb_list_without_easting_stays_put() {
        let mut w = World::new(WorldMeta::new("A", 256, 8.0), 0.0);
        let a = from_paths(vec!["dz\\t_a.p3d".into()]);
        let (o, _) = parse_tb_objects("\"t_a\";100.5;50.0;10;0;0;1;0;\n", &mut w, &a, &ModelDb::new("Z:/none"), false);
        assert_eq!(o[0].x, 100.5);
    }

    #[test]
    fn mask_image_maps_known_colours_and_adds_new_ones() {
        let mut layers = vec![SurfaceLayer { name: "a".into(), texture: "t".into(), material: "m".into(), color: [1, 2, 3] }];
        let mut img = RgbImage::new(10, 10, [1, 2, 3]);
        for i in 0..150 { img.data[i] = 200; }
        let m = mask_from_image(&img, &mut layers, &Job::default()).unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!(m.data[0], 1);
        assert_eq!(m.data[99], 0);
    }

    #[test]
    fn exported_map_imports_back() {
        let d = tempfile::tempdir().unwrap();
        let mut w = World::new(WorldMeta::new("RoundTrip", 512, 6.0), 40.0);
        w.base.set(100, 100, 90.0);
        w.recompose(None);
        for y in 0..w.mask.height { for x in 0..w.mask.width / 2 { w.mask.data[y * w.mask.width + x] = 3; } }
        let m = w.objects.intern("dz\\a.p3d");
        w.objects.insert(Obj { x: 500.0, z: 700.0, elevation: 2.0, yaw: 45.0, pitch: 5.0, roll: 0.0, scale: 1.25, model: m, flags: 0 });
        let db = ModelDb::new(d.path());
        run(&w, &db, &ExportOptions { root: d.path().to_path_buf(), layers: true, world: true, config: true, source: false }, &Job::default()).unwrap();
        let r = import_wrp(&d.path().join("RoundTrip/world/roundtrip.wrp"), None, &db, &Job::default()).unwrap();
        assert_eq!(r.base.at(100, 100), 90.0);
        assert_eq!(r.objects.len(), 1);
        let o = r.objects.iter().next().unwrap().1;
        assert!((o.x - 500.0).abs() < 0.01 && (o.elevation - 2.0).abs() < 0.01 && (o.yaw - 45.0).abs() < 0.01 && (o.scale - 1.25).abs() < 0.01);
        // The mask comes back through the tiles.
        let size = r.meta.size_m();
        assert_eq!(r.layers[r.mask.sample(600.0, 1500.0, size) as usize].name, "en_forest_con");
        assert_eq!(r.layers[r.mask.sample(2500.0, 1500.0, size) as usize].name, "en_grass1");
    }

    #[test]
    fn six_surfaces_on_a_tile_come_back_from_the_export() {
        let d = tempfile::tempdir().unwrap();
        let mut w = World::new(WorldMeta::new("SixRound", 512, 6.0), 40.0);
        while w.layers.len() < 6 {
            let k = w.layers.len();
            w.layers.push(SurfaceLayer { name: format!("s{k}"), texture: format!("dz\\s{k}_ca.paa"), material: format!("dz\\s{k}.rvmat"), color: [k as u8 * 40, 0, 0] });
        }
        let (width, per) = (w.mask.width, w.mask.width as f32 / w.meta.size_m());
        for y in 0..w.mask.height { for x in 0..width { w.mask.data[y * width + x] = ((x as f32 / per / 64.0) as usize % 6) as u8; } }
        let db = ModelDb::new(d.path());
        run(&w, &db, &ExportOptions { root: d.path().to_path_buf(), layers: true, world: true, config: true, source: false }, &Job::default()).unwrap();
        let wrp = Wrp::parse(&std::fs::read(d.path().join("SixRound/world/sixround.wrp")).unwrap()).unwrap();
        assert!(wrp.materials.iter().all(|m| m.matches("_L").count() + m.matches("_N").count() == 6), "{:?}", &wrp.materials[..2]);
        let r = import_wrp(&d.path().join("SixRound/world/sixround.wrp"), None, &db, &Job::default()).unwrap();
        let size = r.meta.size_m();
        for k in 0..6 {
            let got = &r.layers[r.mask.sample(32.0 + 64.0 * k as f32, 1500.0, size) as usize];
            assert!(got.texture.eq_ignore_ascii_case(&w.layers[k].texture), "{k}: {} {}", got.texture, w.layers[k].texture);
        }
    }

    /// Every object of a real Terrain Builder world must come back where the game expects it.
    #[test]
    fn real_world_objects_survive_import_and_export() {
        let p = Path::new("P:/DolinaBobru/world/dolinabobru.wrp");
        if !p.exists() || std::env::var("DZ_SLOW_TESTS").is_err() { return; }
        let db = ModelDb::new("P:/");
        let original = Wrp::parse(&std::fs::read(p).unwrap()).unwrap();
        let w = import_wrp(p, None, &db, &Job::default()).unwrap();
        let (exported, _) = crate::export::wrp_objects(&w, &db);
        assert_eq!(exported.len(), original.objects.len());
        let (mut max_pos, mut max_axis, mut known) = (0.0f32, 0.0f32, 0usize);
        for (a, b) in exported.iter().zip(&original.objects) {
            assert_eq!(a.model, ObjectStore::normalize_path(&b.model));
            for k in 0..9 {
                max_axis = max_axis.max((a.matrix[k] - b.matrix[k]).abs());
            }
            for k in 9..12 {
                max_pos = max_pos.max((a.matrix[k] - b.matrix[k]).abs());
            }
            if db.get(&a.model).known { known += 1; }
        }
        println!("objects {} with model data {known}, max position error {max_pos} m, max matrix error {max_axis}", exported.len());
        assert!(max_pos < 0.02, "position error {max_pos}");
        assert!(max_axis < 0.01, "matrix error {max_axis}");
        assert_eq!(w.height.data, original.elevations);
    }

    #[test]
    fn scan_finds_files_wherever_they_are_and_ranks_by_depth() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        std::fs::create_dir_all(p.join("a/b")).unwrap();
        std::fs::create_dir_all(p.join("map.LayersBackup")).unwrap();
        std::fs::write(p.join("a/b/deep_mask.png"), vec![0; 5000]).unwrap();
        std::fs::write(p.join("my_mask.png"), vec![0; 10]).unwrap();
        std::fs::write(p.join("a/h.asc"), b"x").unwrap();
        std::fs::write(p.join("a/lib.tml"), b"x").unwrap();
        std::fs::write(p.join("map.LayersBackup/o.tml"), b"x").unwrap();
        let s = scan_folder(p);
        assert_eq!(s.mask[0], p.join("my_mask.png"));
        assert_eq!(s.asc.len(), 1);
        assert_eq!(s.libraries, vec![p.join("a/lib.tml")]);
        let plan = ImportPlan::from_scan(p, &s);
        assert!(plan.can_import());
        assert!(plan.objects_from_wrp);
    }

    #[test]
    fn frame_offset_prefers_a_matching_frame() {
        let meta = WorldMeta::new("A", 1024, 4.0);
        let o = |x: f64, z: f64| crate::tv4l::TbObject { template: 0, x, z, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, elevation: 0.0 };
        assert_eq!(frame_offset(&[o(201000.0, 500.0), o(203000.0, 900.0)], &meta), Some((200_000.0, 0.0)));
        assert_eq!(frame_offset(&[o(1000.0, 500.0)], &meta), Some((0.0, 0.0)));
        assert_eq!(frame_offset(&[o(900000.0, 500.0)], &meta), None);
    }

    /// Objects read from Terrain Builder layers must agree with the world built from them.
    #[test]
    fn real_layers_match_the_built_world() {
        let lp = Path::new("P:/BalticrusSource/TerrainBuilder/balticrus.Layers/kamil-woda.tv4l");
        let wp = Path::new("P:/Balticrus/world/Balticrus.wrp");
        if !lp.exists() || !wp.exists() || std::env::var("DZ_SLOW_TESTS").is_err() { return; }
        let db = ModelDb::new("P:/");
        let built = import_wrp(wp, None, &db, &Job::default()).unwrap();
        let lib = crate::tv4l::libraries_in(Path::new("P:/BalticrusSource/TerrainBuilder/TemplateLibs"));
        let templates = crate::tv4l::read_libraries(&lib);
        let layer = crate::tv4l::load(lp).unwrap();
        let (mut found, mut max_axis, mut max_scale, mut max_elev) = (0, 0.0f32, 0.0f32, 0.0f32);
        for o in &layer.objects {
            let path = &templates[&o.template];
            let (x, z) = ((o.x - 200000.0) as f32, o.z as f32);
            let mut best: Option<Obj> = None;
            built.objects.for_each_in(x - 1.0, z - 1.0, x + 1.0, z + 1.0, |_, c| {
                if built.objects.model_path(c) == path && (c.x - x).abs() < 0.5 && (c.z - z).abs() < 0.5 { best = Some(*c); }
            });
            if let Some(c) = best {
                found += 1;
                let (a, b) = (crate::objects::rotation(c.yaw, c.pitch, c.roll, 1.0), { let (y, p, r) = from_terrain_builder(o.yaw, o.pitch, o.roll); crate::objects::rotation(y, p, r, 1.0) });
                for i in 0..3 { for k in 0..3 { max_axis = max_axis.max((a[i][k] - b[i][k]).abs()); } }
                max_scale = max_scale.max((c.scale - o.scale).abs());
                max_elev = max_elev.max((c.elevation - o.elevation).abs());
            }
        }
        println!("layer {} objects, matched in WRP {found}, max matrix error {max_axis}, scale {max_scale}, elevation {max_elev}", layer.objects.len());
        assert!(found > 30);
        assert!(max_axis < 0.001 && max_scale < 0.01 && max_elev < 0.01);
    }

    /// Road parts placed from a project must land where Terrain Builder exported them.
    #[test]
    fn real_road_parts_match_the_built_world() {
        let tv = Path::new("P:/BalticrusSource/TerrainBuilder/balticrus.tv4p");
        let wp = Path::new("P:/Balticrus/world/Balticrus.wrp");
        if !tv.exists() || !wp.exists() || std::env::var("DZ_SLOW_TESTS").is_err() { return; }
        let db = ModelDb::new("P:/");
        let built = import_wrp(wp, None, &db, &Job::default()).unwrap();
        let mut w = World::new(WorldMeta::new("R", built.meta.terrain_size, built.meta.terrain_cell), 0.0);
        let parts = import_tv4p(tv, &mut w, Some(0)).unwrap();
        let wrp_parts = built.objects.iter().filter(|(_, o)| built.objects.model_path(o).contains("\\roads\\parts\\")).count();
        let (mut found, mut max_d, mut max_yaw, mut sum_d) = (0, 0.0f32, 0.0f32, 0.0f32);
        let mut misses: BTreeMap<String, usize> = BTreeMap::new();
        let mut all: Vec<(f32, f32, String)> = Vec::new();
        for (_, o) in w.objects.iter() {
            let path = w.objects.model_path(o);
            let mut best: Option<(f32, Obj)> = None;
            built.objects.for_each_in(o.x - 3.0, o.z - 3.0, o.x + 3.0, o.z + 3.0, |_, c| {
                if built.objects.model_path(c) == path {
                    let d = ((c.x - o.x).powi(2) + (c.z - o.z).powi(2)).sqrt();
                    if best.is_none_or(|b| d < b.0) { best = Some((d, *c)); }
                }
            });
            match best {
                Some((d, c)) => {
                    found += 1;
                    all.push((d, { let dy = (c.yaw - o.yaw).rem_euclid(360.0); dy.min(360.0 - dy) }, path.to_string()));
                    sum_d += d;
                    max_d = max_d.max(d);
                    let dy = (c.yaw - o.yaw).rem_euclid(360.0);
                    max_yaw = max_yaw.max(dy.min(360.0 - dy));
                }
                None => *misses.entry(path.to_string()).or_default() += 1,
            }
        }
        println!("parts {parts} objects {} in WRP {wrp_parts} matched {found} mean distance {} max {max_d} max yaw {max_yaw}", w.objects.len(), sum_d / found.max(1) as f32);
        all.sort_by(|a, b| a.0.total_cmp(&b.0));
        let close: Vec<_> = all.iter().filter(|a| a.0 < 0.1).collect();
        println!("p50 {} p90 {} p99 {} close {} of those yaw>1deg {} examples {:?}", all[all.len() / 2].0, all[all.len() * 9 / 10].0, all[all.len() * 99 / 100].0, close.len(), close.iter().filter(|a| a.1 > 1.0).count(), close.iter().filter(|a| a.1 > 1.0).take(4).collect::<Vec<_>>());
        println!("not matched {:?}", misses.iter().take(8).collect::<Vec<_>>());
        assert!(found as f32 > parts as f32 * 0.8);
    }

    /// How the elevation of scaled objects relates to the built world.
    #[test]
    fn real_scaled_objects_height() {
        let wp = Path::new("P:/Balticrus/world/Balticrus.wrp");
        if !wp.exists() || std::env::var("DZ_SLOW_TESTS").is_err() { return; }
        let db = ModelDb::new("P:/");
        let built = import_wrp(wp, None, &db, &Job::default()).unwrap();
        let lib = crate::tv4l::libraries_in(Path::new("P:/BalticrusSource/TerrainBuilder/TemplateLibs"));
        let templates = crate::tv4l::read_libraries(&lib);
        let mut rows = Vec::new();
        for name in ["kamil-lasy", "kamil-all", "mahauu - las", "mahauu - all", "kamil-drzewa"] {
            let layer = crate::tv4l::load(Path::new(&format!("P:/BalticrusSource/TerrainBuilder/balticrus.Layers/{name}.tv4l"))).unwrap();
            for o in &layer.objects {
                if (o.scale - 1.0).abs() < 0.05 { continue; }
                let Some(path) = templates.get(&o.template) else { continue };
                let (x, z) = ((o.x - 200000.0) as f32, o.z as f32);
                let mut hit = None;
                built.objects.for_each_in(x - 1.0, z - 1.0, x + 1.0, z + 1.0, |_, c| {
                    if built.objects.model_path(c) == path && (c.x - x).abs() < 0.3 && (c.z - z).abs() < 0.3 && (c.scale - o.scale).abs() < 0.01 { hit = Some(*c); }
                });
                if let Some(c) = hit {
                    let info = db.get(path);
                    rows.push((c.elevation - o.elevation, o.scale, info.center[1], path.clone()));
                }
            }
        }
        let strong: Vec<_> = rows.iter().filter(|r| ((r.1 - 1.0) * r.2).abs() > 0.25).collect();
        let same = strong.iter().filter(|r| r.0.abs() < 0.05).count();
        println!("scaled objects with a large centre offset {} agreeing with the world {same}", strong.len());
        assert!(same as f32 >= strong.len() as f32 * 0.99);
    }

    /// Imports a real built map when P: is mounted.
    #[test]
    fn real_built_map_imports() {
        let p = Path::new("P:/DolinaBobru/world/dolinabobru.wrp");
        if !p.exists() || std::env::var("DZ_SLOW_TESTS").is_err() { return; }
        let db = ModelDb::new("P:/");
        let w = import_wrp(p, None, &db, &Job::default()).unwrap();
        assert_eq!(w.meta.size_m(), 12288.0);
        assert!(w.objects.len() > 800_000);
        assert!(w.layers.len() >= 4);
        assert!(w.satellite.is_some());
        // Trees stand on the ground once the bounding centre is removed.
        let mut e: Vec<f32> = w.objects.iter().take(20000).map(|(_, o)| o.elevation).collect();
        e.sort_by(|a, b| a.total_cmp(b));
        assert!(e[e.len() / 2].abs() < 0.5, "median elevation {}", e[e.len() / 2]);
    }
}
