//! Exporting the parts of a map as files other tools read: the heightmap as ASC, the
//! satellite and surface mask as images, `layers.cfg`, shapes and an object list. Each has
//! its options, so that the files fit what they are wanted for.
use crate::fsutil::atomic_write;
use crate::model_info::ModelDb;
use crate::objects::{to_terrain_builder, ObjectStore, FLAG_UNDERGROUND};
use crate::terrain::{Heightmap, MaskMap, RgbImage};
use crate::world::World;
use crate::Job;
use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ImageFormat {
    #[default]
    Png,
    Bmp,
}
impl ImageFormat {
    pub const ALL: [ImageFormat; 2] = [ImageFormat::Png, ImageFormat::Bmp];
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Bmp => "bmp",
        }
    }
}

/// How an image is written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageOptions {
    pub format: ImageFormat,
    /// Side of the image in pixels; 0 keeps the size it has.
    pub size: usize,
    /// Split into this many tiles a side; 1 writes one image.
    pub tiles: usize,
}
impl Default for ImageOptions {
    fn default() -> Self {
        Self { format: ImageFormat::Png, size: 0, tiles: 1 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeightmapOptions {
    /// The terrain with its modifiers (roads, pads, ponds), or only as sculpted.
    pub with_modifiers: bool,
    /// Metres between samples; 0 keeps the cell of the terrain.
    pub cell: f32,
    pub decimals: usize,
    /// Corner coordinates with the map's easting and northing, as Terrain Builder expects.
    pub offset: bool,
    /// Metres added to every height.
    pub raise: f32,
}
impl Default for HeightmapOptions {
    fn default() -> Self {
        Self { with_modifiers: true, cell: 0.0, decimals: 3, offset: true, raise: 0.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SatelliteOptions {
    pub image: ImageOptions,
    /// The baked objects and clone strokes go into the image too.
    pub with_modifiers: bool,
}
impl Default for SatelliteOptions {
    fn default() -> Self {
        Self { image: Default::default(), with_modifiers: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaskOptions {
    pub image: ImageOptions,
    /// The surfaces under objects, forests and painting generators go into the mask too.
    pub with_modifiers: bool,
}
impl Default for MaskOptions {
    fn default() -> Self {
        Self { image: Default::default(), with_modifiers: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayersOptions {
    /// Name of the legend picture `layers.cfg` points at.
    pub legend: String,
    /// Only the surfaces the mask uses.
    pub only_used: bool,
}
impl Default for LayersOptions {
    fn default() -> Self {
        Self { legend: "mapLegend.png".into(), only_used: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ObjectsOptions {
    /// Only objects of layers marked for export.
    pub exported_layers_only: bool,
    /// Only objects of visible layers.
    pub visible_only: bool,
    /// Model names with their whole path instead of the file name Terrain Builder uses.
    pub full_path: bool,
    /// Coordinates with the map's easting and northing.
    pub offset: bool,
    /// One file for each object layer.
    pub split_by_layer: bool,
}
impl Default for ObjectsOptions {
    fn default() -> Self {
        Self { exported_layers_only: true, visible_only: false, full_path: false, offset: true, split_by_layer: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShapesOptions {
    /// Names of the shape layers to write; empty writes the visible ones.
    pub layers: Vec<String>,
    pub offset: bool,
}
impl Default for ShapesOptions {
    fn default() -> Self {
        Self { layers: Vec::new(), offset: true }
    }
}

/// What to export and how; `None` leaves a part out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FileExport {
    pub heightmap: Option<HeightmapOptions>,
    pub satellite: Option<SatelliteOptions>,
    pub mask: Option<MaskOptions>,
    pub layers_cfg: Option<LayersOptions>,
    pub shapes: Option<ShapesOptions>,
    pub objects: Option<ObjectsOptions>,
    /// Only these objects, when some are given.
    #[serde(skip)]
    pub selection: Vec<u32>,
}

impl World {
    /// Copy of what the export reads, for another thread: without generators and routes.
    pub fn export_copy(&self, satellite: bool) -> World {
        let mut w = self.lite();
        if satellite {
            w.satellite = self.satellite.clone();
            w.sat = self.sat.clone();
        }
        w.mask_mods = self.mask_mods.clone();
        w.objects = self.objects.clone();
        w.shape_layers = self.shape_layers.clone();
        w
    }
}

/// Writes the chosen parts into `dir`; returns the files written.
pub fn export(w: &World, db: &ModelDb, o: &FileExport, dir: &Path, job: &Job) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir).with_context(|| format!("{}", dir.display()))?;
    let mut out = Vec::new();
    let id = w.meta.map_id.clone();
    if let Some(h) = &o.heightmap {
        job.stage("Writing the heightmap");
        out.push(write_heightmap(w, h, &dir.join(format!("{id}_heightmap.asc")))?);
    }
    if let (Some(s), Some(img)) = (&o.satellite, &w.satellite) {
        job.stage("Writing the satellite image");
        let img = if s.with_modifiers { img.clone() } else { w.sat_original() };
        out.extend(write_image(&img, &s.image, dir, &format!("{id}_satellite"), false)?);
    }
    if let Some(m) = &o.mask {
        job.stage("Writing the surface mask");
        let mask = if m.with_modifiers { w.mask.clone() } else { w.mask_original() };
        out.extend(write_image(&mask_colors(w, &mask), &m.image, dir, &format!("{id}_mask"), true)?);
    }
    if let Some(l) = &o.layers_cfg {
        let used = l.only_used.then(|| used_layers(&w.mask));
        let layers: Vec<_> = w.layers.iter().enumerate().filter(|(i, _)| used.as_ref().is_none_or(|u| u[*i])).map(|(_, l)| l.clone()).collect();
        let path = dir.join("layers.cfg");
        atomic_write(&path, crate::layers_cfg::write(&layers, &l.legend).as_bytes())?;
        out.push(path);
    }
    if let Some(s) = &o.shapes {
        job.stage("Writing the shapes");
        let (ex, ey) = if s.offset { (w.meta.easting, w.meta.northing) } else { (0.0, 0.0) };
        let shapes = dir.join("shapes");
        for l in w.shape_layers.iter().filter(|l| !l.shapes.is_empty() && if s.layers.is_empty() { l.visible } else { s.layers.contains(&l.name) }) {
            out.extend(crate::shapes::write_layer(&shapes, l, ex, ey)?);
        }
    }
    if let Some(ob) = &o.objects {
        job.stage("Writing the objects");
        out.extend(write_objects(w, db, ob, &o.selection, dir, &id)?);
    }
    Ok(out)
}

fn write_heightmap(w: &World, o: &HeightmapOptions, path: &Path) -> Result<PathBuf> {
    let source = if o.with_modifiers { &w.height } else { &w.base };
    let map = if o.cell > 0.0 && (o.cell - source.cell).abs() > 1e-3 {
        // Sampled again over the same ground: as many samples as the cell fits into the map's
        // side, the first on the south-west corner, as the terrain itself has.
        let side = source.cols as f32 * source.cell;
        let n = ((side / o.cell).round() as usize).max(2);
        let cell = side / n as f32;
        let mut m = Heightmap::new(n, n, cell, 0.0);
        m.data.par_chunks_mut(n).enumerate().for_each(|(j, row)| {
            for (i, v) in row.iter_mut().enumerate() {
                *v = source.sample(i as f32 * cell, j as f32 * cell);
            }
        });
        m
    } else {
        source.clone()
    };
    let (xll, yll) = if o.offset { (w.meta.easting, w.meta.northing) } else { (0.0, 0.0) };
    write_asc(&map, xll, yll, o.decimals.min(6), o.raise, path)?;
    Ok(path.to_path_buf())
}

fn write_asc(map: &Heightmap, xll: f64, yll: f64, decimals: usize, raise: f32, path: &Path) -> Result<()> {
    let rows: Vec<String> = (0..map.rows).into_par_iter().map(|row| {
        let j = map.rows - 1 - row;
        let mut s = String::with_capacity(map.cols * 9);
        for i in 0..map.cols {
            use std::fmt::Write;
            let _ = write!(s, "{:.*} ", decimals, map.at(i, j) + raise);
        }
        s.push('\n');
        s
    }).collect();
    let header = format!("ncols {}\nnrows {}\nxllcorner {xll:.6}\nyllcorner {yll:.6}\ncellsize {:.6}\nNODATA_value -9999\n", map.cols, map.rows, map.cell);
    atomic_write(path, [header].into_iter().chain(rows).collect::<String>().as_bytes())
}

/// The mask in the colours of its surfaces.
fn mask_colors(w: &World, mask: &MaskMap) -> RgbImage {
    let mut img = RgbImage::new(mask.width, mask.height, [0, 0, 0]);
    img.data.par_chunks_mut(3).zip(mask.data.par_iter()).for_each(|(p, l)| {
        p.copy_from_slice(&w.layers.get(*l as usize).map(|l| l.color).unwrap_or([0, 0, 0]));
    });
    img
}

fn used_layers(mask: &MaskMap) -> Vec<bool> {
    let mut used = vec![false; 256];
    for v in &mask.data {
        used[*v as usize] = true;
    }
    used
}

/// The image at its size, scaled, and cut into tiles. A mask is scaled without blending, so
/// that it only ever holds the colours of its surfaces.
fn write_image(img: &RgbImage, o: &ImageOptions, dir: &Path, name: &str, exact: bool) -> Result<Vec<PathBuf>> {
    let side = if o.size > 0 { o.size } else { img.width.max(img.height) };
    let scaled = if side == img.width && side == img.height {
        img.clone()
    } else if exact {
        nearest(img, side, side)
    } else {
        img.resized(side, side)
    };
    let tiles = o.tiles.max(1);
    let mut out = Vec::new();
    if tiles == 1 {
        let path = dir.join(format!("{name}.{}", o.format.extension()));
        save(&scaled, o.format, &path)?;
        out.push(path);
        return Ok(out);
    }
    let step = side.div_ceil(tiles);
    for ty in 0..tiles {
        for tx in 0..tiles {
            let (x0, y0) = (tx * step, ty * step);
            let (w, h) = (step.min(side.saturating_sub(x0)), step.min(side.saturating_sub(y0)));
            if w == 0 || h == 0 {
                continue;
            }
            let mut t = RgbImage::new(w, h, [0, 0, 0]);
            for y in 0..h {
                let from = ((y0 + y) * side + x0) * 3;
                t.data[y * w * 3..(y + 1) * w * 3].copy_from_slice(&scaled.data[from..from + w * 3]);
            }
            // Tiles are named by column and row from the north-west corner.
            let path = dir.join(format!("{name}_{tx:02}_{ty:02}.{}", o.format.extension()));
            save(&t, o.format, &path)?;
            out.push(path);
        }
    }
    Ok(out)
}

fn nearest(img: &RgbImage, width: usize, height: usize) -> RgbImage {
    let mut out = RgbImage::new(width, height, [0, 0, 0]);
    out.data.par_chunks_mut(width * 3).enumerate().for_each(|(y, row)| {
        let sy = (y * img.height / height).min(img.height - 1);
        for x in 0..width {
            let sx = (x * img.width / width).min(img.width - 1);
            row[x * 3..x * 3 + 3].copy_from_slice(&img.data[(sy * img.width + sx) * 3..(sy * img.width + sx) * 3 + 3]);
        }
    });
    out
}

fn save(img: &RgbImage, format: ImageFormat, path: &Path) -> Result<()> {
    let f = match format {
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Bmp => image::ImageFormat::Bmp,
    };
    image::save_buffer_with_format(path, &img.data, img.width as u32, img.height as u32, image::ColorType::Rgb8, f).with_context(|| format!("{}", path.display()))
}

/// Objects as Terrain Builder's text list: `"name";x;y;yaw;pitch;roll;scale;height;`.
fn write_objects(w: &World, db: &ModelDb, o: &ObjectsOptions, selection: &[u32], dir: &Path, id: &str) -> Result<Vec<PathBuf>> {
    use std::fmt::Write;
    let (ex, ey) = if o.offset { (w.meta.easting, w.meta.northing) } else { (0.0, 0.0) };
    let mut files: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let chosen: Box<dyn Iterator<Item = (u32, &crate::objects::Obj)>> = if selection.is_empty() {
        Box::new(w.objects.iter())
    } else {
        Box::new(selection.iter().filter_map(|i| w.objects.get(*i).map(|o| (*i, o))))
    };
    for (_, ob) in chosen {
        if (o.exported_layers_only && !w.object_exported(ob)) || (o.visible_only && !w.object_visible(ob)) {
            continue;
        }
        let path = w.objects.model_path(ob);
        let name = if o.full_path { path.to_string() } else { path.rsplit('\\').next().unwrap_or(path).trim_end_matches(".p3d").to_string() };
        let (yaw, pitch, roll) = to_terrain_builder(ob.yaw, ob.pitch, ob.roll);
        // Terrain Builder places a tilted model by its middle.
        let up = crate::objects::rotation(ob.yaw, ob.pitch, ob.roll, ob.scale)[1];
        let center_y = if ob.pitch == 0.0 && ob.roll == 0.0 { 0.0 } else { db.get(path).center[1] };
        let file = if o.split_by_layer {
            w.object_layers.get(ob.layer() as usize).map(|l| l.name.clone()).unwrap_or_else(|| "objects".into())
        } else {
            "objects".into()
        };
        let text = files.entry(file).or_default();
        let _ = writeln!(text, "\"{name}\";{:.6};{:.6};{:.6};{:.6};{:.6};{:.6};{:.6};{};",
            ob.x as f64 + (up[0] * center_y) as f64 + ex, ob.z as f64 + (up[2] * center_y) as f64 + ey, yaw, pitch, roll, ob.scale,
            ObjectStore::origin_y(ob, &w.height) - w.height.sample(ob.x, ob.z), u32::from(ob.flags & FLAG_UNDERGROUND != 0));
    }
    if files.is_empty() {
        // An empty list rather than none, so that it is clear nothing matched.
        files.insert("objects".into(), String::new());
    }
    let mut out = Vec::new();
    for (name, text) in files {
        let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
        let path = dir.join(if o.split_by_layer && !text.is_empty() { format!("{id}_objects_{safe}.txt") } else { format!("{id}_objects.txt") });
        atomic_write(&path, text.as_bytes())?;
        out.push(path);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::WorldMeta;

    #[test]
    fn every_part_is_written_with_its_options() {
        let d = tempfile::tempdir().unwrap();
        let mut w = World::new(WorldMeta::new("T", 64, 8.0), 20.0);
        w.mask = MaskMap::new(64, 64, 0);
        for v in &mut w.mask.data[..2048] { *v = 1; }
        w.satellite = Some(RgbImage::new(128, 128, [10, 20, 30]));
        let m = w.objects.intern("dz\\a\\b.p3d");
        w.objects.insert(crate::objects::Obj { x: 10.0, z: 20.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: m, flags: 0 });
        let o = FileExport {
            heightmap: Some(HeightmapOptions { cell: 16.0, decimals: 1, ..Default::default() }),
            satellite: Some(SatelliteOptions { image: ImageOptions { format: ImageFormat::Bmp, size: 64, tiles: 2 }, with_modifiers: true }),
            mask: Some(MaskOptions { image: ImageOptions { size: 32, ..Default::default() }, with_modifiers: true }),
            layers_cfg: Some(LayersOptions { only_used: true, ..Default::default() }),
            shapes: Some(ShapesOptions::default()),
            objects: Some(ObjectsOptions { full_path: true, offset: false, ..Default::default() }),
            selection: Vec::new(),
        };
        let files = export(&w, &ModelDb::new(d.path()), &o, d.path(), &Job::default()).unwrap();
        let asc = std::fs::read_to_string(d.path().join("T_heightmap.asc")).unwrap();
        assert!(asc.starts_with("ncols 32\nnrows 32\n"), "{}", &asc[..60]);
        assert!(asc.contains("cellsize 16.000000") && asc.lines().nth(6).unwrap().starts_with("20.0 "), "{:?}", asc.lines().take(7).collect::<Vec<_>>());
        assert_eq!(files.iter().filter(|f| f.extension().is_some_and(|e| e == "bmp")).count(), 4, "{files:?}");
        let mask = RgbImage::load(&d.path().join("T_mask.png")).unwrap();
        assert_eq!((mask.width, mask.height), (32, 32));
        // Scaled without blending: only the two colours of the surfaces.
        let (a, b) = (w.layers[0].color, w.layers[1].color);
        assert!(mask.data.chunks(3).all(|p| p == a || p == b));
        let cfg = std::fs::read_to_string(d.path().join("layers.cfg")).unwrap();
        assert!(cfg.contains(&w.layers[1].name) && !cfg.contains(&w.layers[2].name), "{cfg}");
        let objects = std::fs::read_to_string(d.path().join("T_objects.txt")).unwrap();
        assert!(objects.starts_with("\"dz\\a\\b.p3d\";10.000000;20.000000;"), "{objects}");
    }
}
