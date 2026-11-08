//! Importers for existing work: Terrain Builder folders, WRP worlds, DayZForests and Road Studio.
use crate::assets::AssetIndex;
use crate::model_info::ModelDb;
use crate::objects::{angles, Obj, ObjectStore, FLAG_ABSOLUTE, FLAG_UNDERGROUND};
use crate::terrain::{parse_asc, Heightmap, MaskMap, RgbImage};
use crate::world::{default_layers, MapLine, SurfaceLayer, World, WorldMeta};
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

/// Parses a Terrain Builder object list.
pub fn parse_tb_objects(text: &str, w: &mut World, assets: &AssetIndex, absolute: bool) -> (Vec<Obj>, Vec<String>) {
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
        out.push(Obj { x: v(1) - shift.0, z: v(2) - shift.1, yaw: v(3), pitch: v(4), roll: v(5), scale: if v(6) > 0.0 { v(6) } else { 1.0 }, elevation: v(7), model, flags });
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

/// Finds an existing built or source map below a folder by what it contains.
#[derive(Debug, Default, Clone)]
pub struct FolderScan {
    pub wrp: Option<PathBuf>,
    pub asc: Option<PathBuf>,
    pub satellite: Option<PathBuf>,
    pub mask: Option<PathBuf>,
    pub layers_cfg: Option<PathBuf>,
    pub objects: Vec<PathBuf>,
    pub tv4p: Vec<PathBuf>,
    pub tile_dir: Option<PathBuf>,
}

pub fn scan_folder(root: &Path) -> FolderScan {
    let mut s = FolderScan::default();
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if p.is_dir() {
                if name == "layers" && p.join("S_000_000_lco.paa").is_file() | p.join("S_000_000_lco.png").is_file() {
                    s.tile_dir = Some(p.clone());
                } else if depth < 4 && !name.ends_with(".cache") && !name.ends_with(".layers") {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            let bigger = |old: &Option<PathBuf>| old.as_ref().and_then(|o| o.metadata().ok()).map(|m| m.len()).unwrap_or(0) < p.metadata().map(|m| m.len()).unwrap_or(0);
            if name.ends_with(".wrp") {
                if bigger(&s.wrp) { s.wrp = Some(p.clone()); }
            } else if name.ends_with(".asc") {
                if bigger(&s.asc) { s.asc = Some(p.clone()); }
            } else if name == "layers.cfg" {
                s.layers_cfg = Some(p.clone());
            } else if name.ends_with(".tv4p") {
                s.tv4p.push(p.clone());
            } else if name.ends_with(".bmp") || name.ends_with(".png") || name.ends_with(".tif") {
                if name.contains("legend") || name.contains("preview") { continue; }
                if name.contains("mask") { if bigger(&s.mask) { s.mask = Some(p.clone()); } }
                else if name.contains("sat") { if bigger(&s.satellite) { s.satellite = Some(p.clone()); } }
            } else if name.ends_with(".txt") {
                let first = std::fs::read(&p).ok().and_then(|b| b.split(|c| *c == b'\n').next().map(|l| String::from_utf8_lossy(l).to_string()));
                if first.map(|l| l.starts_with('"') && l.matches(';').count() >= 7).unwrap_or(false) {
                    s.objects.push(p.clone());
                }
            }
        }
    }
    s
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
                let slot = if p[3] < 200 && count >= 5 && p[0].max(p[1]).max(p[2]) < 128 { 4 }
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
    Ok(World::from_parts(meta, height, mask, layers, satellite, objects, Vec::new(), Vec::new()))
}

/// Builds a project out of the loose files of a Terrain Builder folder.
pub fn import_folder(root: &Path, assets: &AssetIndex, db: &ModelDb, job: &Job) -> Result<World> {
    job.stage("Looking through the folder");
    let scan = scan_folder(root);
    if scan.asc.is_none() {
        if let Some(wrp) = &scan.wrp {
            return import_wrp(wrp, scan.tile_dir.as_deref(), db, job);
        }
        bail!("No heightmap (.asc) or world (.wrp) found in {}", root.display());
    }
    job.stage("Reading heightmap");
    let asc = read_asc(scan.asc.as_ref().unwrap())?;
    ensure!(asc.map.cols == asc.map.rows, "Only square heightmaps are supported");
    let name = root.file_name().map(|s| s.to_string_lossy().trim_end_matches("Source").trim_end_matches('_').to_string()).unwrap_or_else(|| "Imported".into());
    let mut meta = WorldMeta::new(&name, asc.map.cols as u32, asc.map.cell);
    if asc.xll != 0.0 {
        meta.easting = asc.xll;
    }
    meta.northing = asc.yll;
    let mut layers = match &scan.layers_cfg {
        Some(p) => crate::layers_cfg::parse(&String::from_utf8_lossy(&std::fs::read(p)?))?,
        None => default_layers(),
    };
    let satellite = match &scan.satellite {
        Some(p) => {
            job.stage(format!("Reading {}", p.display()));
            Some(RgbImage::load(p)?)
        }
        None => None,
    };
    let mask = match &scan.mask {
        Some(p) => {
            job.stage(format!("Reading {}", p.display()));
            let img = RgbImage::load(p)?;
            mask_from_image(&img, &mut layers, job)?
        }
        None => {
            let px = (meta.size_m() as usize).clamp(1024, 20480);
            MaskMap::new(px, px, 0)
        }
    };
    let mut w = World::from_parts(meta, asc.map, mask, layers, satellite, ObjectStore::default(), Vec::new(), Vec::new());
    for p in &scan.objects {
        job.stage(format!("Reading objects {}", p.display()));
        let text = String::from_utf8_lossy(&std::fs::read(p)?).into_owned();
        let (list, missing) = parse_tb_objects(&text, &mut w, assets, false);
        job.log(format!("{}: {} objects", p.display(), list.len()));
        if !missing.is_empty() {
            job.log(format!("Unknown models: {}", missing.join(", ")));
        }
        for o in list {
            w.objects.insert(o);
        }
    }
    for p in &scan.tv4p {
        match import_tv4p(p, &mut w) {
            Ok(n) => job.log(format!("{}: {n} roads", p.display())),
            Err(e) => job.log(format!("{}: {e:#}", p.display())),
        }
    }
    Ok(w)
}

/// Roads of a Terrain Builder project as lines.
pub fn import_tv4p(path: &Path, w: &mut World) -> Result<usize> {
    use dayz_road_tool::{geometry::Library, tv4p};
    let mut lib = Library::new(PathBuf::from("P:\\dz\\structures\\roads\\parts"));
    let project = tv4p::load(path, &mut lib)?;
    let (ex, ey) = (w.meta.easting, w.meta.northing);
    let mut n = 0;
    for road in &project.roads {
        for line in &road.shape.lines {
            if line.len() < 2 {
                continue;
            }
            let points: Vec<[f32; 2]> = line.iter().map(|p| [(p[0] - ex) as f32, (p[1] - ey) as f32]).collect();
            let family = dayz_road_tool::road_colors::family(&road.model);
            w.lines.push(MapLine { name: format!("Road {}", road.id), family, points, width: 6.0 });
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
        w.objects.insert(Obj { x: p.x as f32, z: p.y as f32, elevation: p.elevation as f32, yaw: p.yaw as f32, pitch: p.pitch as f32, roll: p.roll as f32, scale: p.scale as f32, model, flags: 0 });
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
        if points.len() >= 2 {
            w.lines.push(MapLine { name: route.name.clone(), family: route.family.clone(), points, width: 6.0 });
        }
        for p in route.parts.iter().chain(&route.extras) {
            let file = p.model.rsplit(['\\', '/']).next().unwrap_or(&p.model).to_string();
            let model = w.objects.intern(&format!("{prefix}\\{file}"));
            w.objects.insert(Obj { x: (p.position[0] - ex) as f32, z: (p.position[1] - ey) as f32, elevation: 0.0, yaw: p.rotation.rem_euclid(360.0) as f32, pitch: 0.0, roll: 0.0, scale: 1.0, model, flags: 0 });
            n += 1;
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
    fn tb_list_with_easting_is_shifted() {
        let mut w = World::new(WorldMeta::new("A", 256, 8.0), 0.0);
        let a = from_paths(vec!["dz\\t_a.p3d".into()]);
        let (o, missing) = parse_tb_objects("\"t_a\";200100.5;50.0;10;0;0;1.5;0.0;1;\n\"nope\";200000;0;0;0;0;1;0;0;\n", &mut w, &a, false);
        assert_eq!(o.len(), 1);
        assert_eq!(missing, vec!["nope (1)".to_string()]);
        assert!((o[0].x - 100.5).abs() < 1e-2 && o[0].scale == 1.5);
        assert_eq!(o[0].flags, FLAG_UNDERGROUND);
    }

    #[test]
    fn tb_list_without_easting_stays_put() {
        let mut w = World::new(WorldMeta::new("A", 256, 8.0), 0.0);
        let a = from_paths(vec!["dz\\t_a.p3d".into()]);
        let (o, _) = parse_tb_objects("\"t_a\";100.5;50.0;10;0;0;1;0;\n", &mut w, &a, false);
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
