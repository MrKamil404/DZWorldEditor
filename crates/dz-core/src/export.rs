//! Writes everything a map PBO needs into the work drive folder.
use crate::model_info::ModelDb;
use crate::objects::{rotation, ObjectStore, FLAG_UNDERGROUND};
use crate::paa_write::{self, Format};
use crate::terrain::RgbImage;
use crate::world::World;
use crate::wrp::{Wrp, WrpObject};
use crate::{fsutil::atomic_write, Job};
use anyhow::{ensure, Context, Result};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Surfaces a tile can hold: six in Terrain Builder's six-material mode, without normals.
pub const MAX_SLOTS: usize = 6;
/// Surfaces of a tile that also has a satellite normal map (`TerrainSNX`).
const SNX_SLOTS: usize = 5;

/// Grid of satellite and mask tiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileLayout {
    pub tiles: u32,
    pub step: f32,
    pub tile_m: f32,
    pub margin: f32,
    pub px: u32,
    pub size: f32,
}

impl TileLayout {
    pub fn of(w: &World) -> Self {
        let step = w.meta.tile_step();
        let px = w.meta.tile_px as f32;
        let mut mpp = w.satellite.as_ref().map(|s| w.meta.size_m() / s.width as f32).unwrap_or(1.0);
        // The tile must overlap its neighbours but not by a wasteful amount.
        if px * mpp < step + 32.0 * mpp || px * mpp > step * 2.0 {
            mpp = step / (px - 128.0);
        }
        let tile_m = px * mpp;
        Self { tiles: w.meta.tiles(), step, tile_m, margin: (tile_m - step) * 0.5, px: px as u32, size: w.meta.size_m() }
    }
    /// World position of a tile pixel's centre.
    #[inline]
    pub fn world(&self, col: u32, row: u32, x: f32, y: f32) -> (f32, f32) {
        let mpp = self.tile_m / self.px as f32;
        (col as f32 * self.step - self.margin + (x + 0.5) * mpp, self.size + self.margin - row as f32 * self.step - (y + 0.5) * mpp)
    }
    pub fn uv_pos(&self, col: u32, row: u32) -> (f32, f32) {
        ((self.margin - col as f32 * self.step) / self.tile_m, (self.size + self.margin - row as f32 * self.step) / self.tile_m)
    }
}

#[derive(Clone, Debug)]
pub struct ExportOptions {
    /// Work drive root.
    pub root: PathBuf,
    pub layers: bool,
    pub world: bool,
    pub config: bool,
    /// Also writes ASC, images and object list for Terrain Builder.
    pub source: bool,
}

#[derive(Debug, Default)]
pub struct ExportReport {
    pub folder: PathBuf,
    pub tiles: usize,
    pub rvmats: usize,
    pub objects: usize,
    pub missing_models: Vec<String>,
    pub warnings: Vec<String>,
}

fn sample_rgb(s: &RgbImage, wx: f32, wz: f32, size: f32) -> [u8; 3] {
    let fx = (wx / size * s.width as f32 - 0.5).clamp(0.0, s.width as f32 - 1.001);
    let fy = ((1.0 - wz / size) * s.height as f32 - 0.5).clamp(0.0, s.height as f32 - 1.001);
    let (x, y) = (fx as usize, fy as usize);
    let (a, b) = (fx - x as f32, fy - y as f32);
    let at = |x: usize, y: usize, c: usize| s.data[(y * s.width + x) * 3 + c] as f32;
    let mut out = [0u8; 3];
    for c in 0..3 {
        let top = at(x, y, c) + (at(x + 1, y, c) - at(x, y, c)) * a;
        let bot = at(x, y + 1, c) + (at(x + 1, y + 1, c) - at(x, y + 1, c)) * a;
        out[c] = (top + (bot - top) * b) as u8;
    }
    out
}

/// Satellite colour when the project has no image: the layer colour with hillshade.
fn synthetic_rgb(w: &World, wx: f32, wz: f32) -> [u8; 3] {
    let size = w.meta.size_m();
    let l = w.mask.sample(wx, wz, size) as usize;
    let c = w.layers.get(l).map(|l| l.color).unwrap_or([128, 128, 128]);
    let n = w.height.normal(wx, wz);
    let shade = (0.55 + 0.6 * (n[0] * -0.5 + n[1] * 0.7 + n[2] * 0.5)).clamp(0.25, 1.25);
    [(c[0] as f32 * shade).min(255.0) as u8, (c[1] as f32 * shade).min(255.0) as u8, (c[2] as f32 * shade).min(255.0) as u8]
}

/// RGBA of mask slot `slot` (0-based), as Terrain Builder encodes it.
#[inline]
fn slot_rgba(slot: usize, six: bool) -> [u8; 4] {
    match (slot, six) {
        (0, _) => [0, 0, 0, 255],
        (1, _) => [255, 0, 0, 255],
        (2, _) => [0, 255, 0, 255],
        (3, _) => [0, 0, 255, 255],
        (4, true) => [0, 0, 255, 128],
        (_, true) => [0, 0, 255, 0],
        _ => [0, 0, 0, 128],
    }
}

/// In six-material masks the blue channel holds three surfaces, told apart by alpha. Terrain
/// Builder spreads a lowered alpha 8 pixels into the other surfaces around them, so that
/// filtering at their border does not fall through to the wrong blue surface.
fn spread_blue_alpha(mask: &mut [u8], px: usize) {
    const REACH: usize = 8;
    let blue = |m: &[u8], i: usize| m[i * 4 + 2] == 255;
    if !(0..px * px).any(|i| blue(mask, i) && mask[i * 4 + 3] < 255) {
        return;
    }
    let mut low: Vec<u8> = (0..px * px).map(|i| if blue(mask, i) { mask[i * 4 + 3] } else { 255 }).collect();
    for _pass in 0..2 {
        let horizontal = _pass == 0;
        let source = low.clone();
        for a in 0..px {
            for b in 0..px {
                let at = |k: usize| if horizontal { a * px + k } else { k * px + a };
                let (from, to) = (b.saturating_sub(REACH), (b + REACH).min(px - 1));
                low[at(b)] = (from..=to).map(|k| source[at(k)]).min().unwrap_or(255);
            }
        }
    }
    for (i, v) in low.iter().enumerate() {
        if !blue(mask, i) {
            mask[i * 4 + 3] = *v;
        }
    }
}

fn nopx_of(root: &Path, texture: &str) -> String {
    let lower = texture.to_ascii_lowercase();
    let candidate = lower.replace("_ca.paa", "_nopx.paa");
    if root.join(candidate.replace('\\', "/")).is_file() {
        candidate
    } else {
        "#(argb,1,1,1)color(0.5,0.5,1,0.5,nopx)".to_string()
    }
}

/// Text of one tile material, matching what Terrain Builder generates.
#[allow(clippy::too_many_arguments)]
pub fn rvmat(id: &str, name_col: u32, name_row: u32, layout: &TileLayout, slots: usize, used: &[Option<(String, String)>], six: bool) -> String {
    let tile = format!("{name_col:03}_{name_row:03}");
    let id_lc = id.to_ascii_lowercase();
    let scale = 1.0 / layout.tile_m;
    let (u, v) = layout.uv_pos(name_col, name_row);
    let world_gen = |n: u32| format!(
        "class TexGen{n}\n{{\n\tuvSource=\"worldPos\";\n\tclass uvTransform\n\t{{\n\t\taside[]={{{scale:.9},0,0}};\n\t\tup[]={{0,0,{scale:.9}}};\n\t\tdir[]={{0,{:.9},0}};\n\t\tpos[]={{{u:.6},{v:.6},0}};\n\t}};\n}};\n", -scale
    );
    let tex_gen = |n: u32, k: u32| format!(
        "class TexGen{n}\n{{\n\tuvSource=\"tex\";\n\tclass uvTransform\n\t{{\n\t\taside[]={{{k},0,0}};\n\t\tup[]={{0,{k},0}};\n\t\tdir[]={{0,0,{k}}};\n\t\tpos[]={{0,0,0}};\n\t}};\n}};\n"
    );
    let mut s = String::with_capacity(2048);
    s += if six { "ambient[]={1,1,1,1};\ndiffuse[]={1,1,1,1};\n" } else { "ambient[]={0.89999998,0.89999998,0.89999998,1};\ndiffuse[]={0.89999998,0.89999998,0.89999998,1};\n" };
    s += "forcedDiffuse[]={0.02,0.02,0.02,1};\nemmisive[]={0,0,0,0};\nspecular[]={0,0,0,0};\nspecularPower=0;\n";
    s += &format!("class Stage0\n{{\n\ttexture=\"{id_lc}\\data\\layers\\s_{tile}_lco.paa\";\n\ttexGen=3;\n}};\n");
    s += &format!("class Stage1\n{{\n\ttexture=\"{id_lc}\\data\\layers\\m_{tile}_lca.paa\";\n\ttexGen=4;\n}};\n");
    s += &world_gen(3);
    s += &world_gen(4);
    s += &tex_gen(0, 1);
    s += &tex_gen(1, 10);
    s += &tex_gen(2, 10);
    s += if six { "PixelShaderID=\"TerrainX\";\nVertexShaderID=\"Terrain\";\n" } else { "PixelShaderID=\"TerrainSNX\";\nVertexShaderID=\"Terrain\";\n" };
    s += "class Stage2\n{\n\ttexture=\"#(rgb,1,1,1)color(0.5,0.5,0.5,1,cdt)\";\n\ttexGen=0;\n};\n";
    for k in 0..slots {
        let (nopx, ca) = match &used[k] {
            Some(p) => (p.0.as_str(), p.1.as_str()),
            None => ("", ""),
        };
        s += &format!("class Stage{}\n{{\n\ttexture=\"{nopx}\";\n\ttexGen=1;\n}};\n", 3 + k * 2);
        s += &format!("class Stage{}\n{{\n\ttexture=\"{ca}\";\n\ttexGen=2;\n}};\n", 4 + k * 2);
    }
    if !six {
        s += &format!("class Stage14\n{{\n\ttexture=\"{id_lc}\\data\\layers\\n_{tile}_nohq.paa\";\n\ttexGen=3;\n}};\n");
    }
    s
}

struct TileOutput {
    /// land cell (i, j) -> rvmat file name
    cells: Vec<((u32, u32), String)>,
    rvmats: BTreeMap<String, String>,
}

fn export_tile(w: &World, opts: &ExportOptions, layout: &TileLayout, dir: &Path, col: u32, row: u32, layer_tex: &[(String, String)], six: bool) -> Result<TileOutput> {
    let px = layout.px as usize;
    let size = w.meta.size_m();
    let id = &w.meta.map_id;
    let _ = opts;
    let mut sat = vec![255u8; px * px * 4];
    let mut indices = vec![0u8; px * px];
    let mut count = [0u32; 256];
    for y in 0..px {
        for x in 0..px {
            let (wx, wz) = layout.world(col, row, x as f32, y as f32);
            let (cx, cz) = (wx.clamp(0.0, size - 0.01), wz.clamp(0.0, size - 0.01));
            let c = match &w.satellite {
                Some(s) => sample_rgb(s, cx, cz, size),
                None => synthetic_rgb(w, cx, cz),
            };
            sat[(y * px + x) * 4..(y * px + x) * 4 + 3].copy_from_slice(&c);
            let l = w.mask.sample(cx, cz, size);
            indices[y * px + x] = l;
            count[l as usize] += 1;
        }
    }
    // The most used surfaces of the tile, as many as the material can take.
    let mut order: Vec<usize> = (0..256).filter(|l| count[*l] > 0).collect();
    order.sort_by_key(|l| std::cmp::Reverse(count[*l]));
    order.truncate((if six { MAX_SLOTS } else { SNX_SLOTS }).min(w.meta.max_tile_surfaces.max(1) as usize));
    order.sort();
    let slots = order.len();
    let mut slot_of = [0u8; 256];
    for l in 0..256usize {
        slot_of[l] = match order.iter().position(|o| *o == l) {
            Some(s) => s as u8,
            None => {
                // Replaced by the surface used most in the tile.
                let best = order.iter().copied().max_by_key(|o| count[*o]).unwrap_or(0);
                order.iter().position(|o| *o == best).unwrap_or(0) as u8
            }
        };
    }
    let mut mask = vec![0u8; px * px * 4];
    for (i, l) in indices.iter().enumerate() {
        mask[i * 4..i * 4 + 4].copy_from_slice(&slot_rgba(slot_of[*l as usize] as usize, six));
    }
    if six {
        spread_blue_alpha(&mut mask, px);
    }
    // Normal map from the heightmap.
    let mut normal = vec![255u8; px * px * 4];
    for y in 0..px {
        for x in 0..px {
            let (wx, wz) = layout.world(col, row, x as f32, y as f32);
            let n = w.height.normal(wx.clamp(0.0, size), wz.clamp(0.0, size));
            let p = (y * px + x) * 4;
            normal[p] = ((n[0] * 0.5 + 0.5) * 255.0) as u8;
            normal[p + 1] = ((n[2] * 0.5 + 0.5) * 255.0) as u8;
            normal[p + 2] = 255;
        }
    }
    let tile = format!("{col:03}_{row:03}");
    atomic_write(&dir.join(format!("S_{tile}_lco.paa")), &paa_write::encode(&sat, px, px, Format::Dxt1))?;
    atomic_write(&dir.join(format!("M_{tile}_lca.paa")), &paa_write::encode(&mask, px, px, Format::Dxt5))?;
    if !six {
        atomic_write(&dir.join(format!("N_{tile}_nohq.paa")), &paa_write::encode(&normal, px, px, Format::Dxt5))?;
    }

    // One material per set of surfaces actually present in a land cell.
    let cells_per = w.meta.cells_per_tile;
    let land = w.meta.land_size();
    let cell = w.meta.land_cell();
    let mpp = layout.tile_m / layout.px as f32;
    let mut out = TileOutput { cells: Vec::new(), rvmats: BTreeMap::new() };
    for cy in 0..cells_per {
        for cx in 0..cells_per {
            let i = col * cells_per + cx;
            let j = land - 1 - (row * cells_per + cy);
            // Pixel bounds of the cell, grown a little because terrain blends across edges.
            let pad = 8.0;
            let x0 = (((cx as f32 * cell + layout.margin) / mpp - pad) as i64).clamp(0, px as i64 - 1) as usize;
            let x1 = ((((cx + 1) as f32 * cell + layout.margin) / mpp + pad) as i64).clamp(0, px as i64 - 1) as usize;
            let y0 = (((cy as f32 * cell + layout.margin) / mpp - pad) as i64).clamp(0, px as i64 - 1) as usize;
            let y1 = ((((cy + 1) as f32 * cell + layout.margin) / mpp + pad) as i64).clamp(0, px as i64 - 1) as usize;
            let mut present = [false; MAX_SLOTS];
            for y in y0..=y1 {
                for x in x0..=x1 {
                    present[slot_of[indices[y * px + x] as usize] as usize] = true;
                }
            }
            let mut name = format!("P_{col:03}-{row:03}");
            let mut used: Vec<Option<(String, String)>> = Vec::with_capacity(slots);
            for k in 0..slots {
                if present[k] {
                    name += &format!("_L{:02}", order[k]);
                    used.push(layer_tex.get(order[k]).cloned());
                } else {
                    name += "_N";
                    used.push(None);
                }
            }
            name += ".rvmat";
            if !out.rvmats.contains_key(&name) {
                out.rvmats.insert(name.clone(), rvmat(id, col, row, layout, slots, &used, six));
            }
            out.cells.push(((i, j), name));
        }
    }
    for (name, text) in &out.rvmats {
        atomic_write(&dir.join(name), text.as_bytes())?;
    }
    Ok(out)
}

/// The WRP objects of the world.
pub fn wrp_objects(w: &World, db: &ModelDb) -> (Vec<WrpObject>, Vec<String>) {
    let info: Vec<_> = w.objects.models.iter().map(|m| db.get(&m.path)).collect();
    let mut missing = BTreeSet::new();
    let mut out = Vec::with_capacity(w.objects.len());
    for (_, o) in w.objects.iter().filter(|(_, o)| w.object_exported(o)) {
        let i = &info[o.model as usize];
        if !i.known {
            missing.insert(w.objects.models[o.model as usize].path.clone());
        }
        let m = rotation(o.yaw, o.pitch, o.roll, o.scale);
        let y = ObjectStore::origin_y(o, &w.height) - w.meta.sea_level;
        let c = i.center;
        let mut matrix = [0.0f32; 12];
        for k in 0..3 {
            matrix[k] = m[0][k];
            matrix[3 + k] = m[1][k];
            matrix[6 + k] = m[2][k];
            matrix[9 + k] = m[0][k] * c[0] + m[1][k] * c[1] + m[2][k] * c[2];
        }
        matrix[9] += o.x;
        matrix[10] += y;
        matrix[11] += o.z;
        out.push(WrpObject { matrix, id: out.len() as u32, model: w.objects.models[o.model as usize].path.clone(), underground: o.flags & FLAG_UNDERGROUND != 0 });
    }
    (out, missing.into_iter().collect())
}

pub fn write_configs(w: &World, target: &Path, has_navmesh: bool) -> Result<()> {
    let id = &w.meta.map_id;
    let size = w.meta.size_m();
    let (sx, sz) = (w.meta.spawn[0], w.meta.spawn[1]);
    let sy = w.height.sample(sx, sz) - w.meta.sea_level;
    let mut materials = String::new();
    for (i, l) in w.layers.iter().enumerate() {
        materials += &format!("\t\t\tmaterial{i}=\"{}\";\n", l.material);
    }
    let navmesh = if has_navmesh {
        format!("\t\tclass Navmesh: Navmesh\n\t\t{{\n\t\t\tnavmeshName=\"\\{id}\\navmesh\\navmesh.nm\";\n\t\t\tclass GenParams: GenParams\n\t\t\t{{\n\t\t\t\tseedPosition[]={{{sx:.1},{sy:.1},{sz:.1}}};\n\t\t\t}};\n\t\t}};\n")
    } else {
        "\t\tclass Navmesh: Navmesh\n\t\t{\n\t\t\tnavmeshName=\"\";\n\t\t};\n".to_string()
    };
    let holes = if w.holes.is_empty() { String::new() } else { crate::holes::class_text(&w.holes, "\t\t") };
    let names = crate::keypoints::names_text(&w.keypoints, "\t\t\t");
    let world = format!(
"class CfgPatches
{{
	class {id}_world
	{{
		units[]={{}};
		weapons[]={{}};
		requiredVersion=0.1;
		requiredAddons[]={{\"DZ_Data\",\"DZ_Worlds_Enoch\"}};
		author=\"{author}\";
		name=\"{name}\";
	}};
}};
class CfgWorlds
{{
	class CAWorld;
	class Enoch: CAWorld
	{{
		class Navmesh
		{{
			class GenParams;
		}};
		class UsedTerrainMaterials;
	}};
	class {id}: Enoch
	{{
		description=\"{name}\";
		hasOcean=1;
		worldName=\"{id}\\world\\{id_lc}.wrp\";
		initWorld=\"{id}\";
		cutscenes[]={{}};
		centerPosition[]={{{cx:.1},{cz:.1},{sy:.1}}};
		longitude={lon};
		latitude={lat};
{navmesh}{holes}		class Grid
		{{
			offsetX=0;
			offsetY={size:.0};
			class Zoom1
			{{
				zoomMax=0.15000001;
				format=\"XY\";
				formatX=\"000\";
				formatY=\"000\";
				stepX=100;
				stepY=-100;
			}};
			class Zoom2
			{{
				zoomMax=0.85000002;
				format=\"XY\";
				formatX=\"00\";
				formatY=\"00\";
				stepX=1000;
				stepY=-1000;
			}};
			class Zoom3
			{{
				zoomMax=1e+30;
				format=\"XY\";
				formatX=\"0\";
				formatY=\"0\";
				stepX=10000;
				stepY=-10000;
			}};
		}};
		class UsedTerrainMaterials: UsedTerrainMaterials
		{{
{materials}		}};
		class Names
		{{
{names}		}};
	}};
}};
class CfgWorldList
{{
	class {id}
	{{
	}};
}};
",
        author = w.meta.author, name = w.meta.name, id_lc = id.to_ascii_lowercase(),
        cx = size * 0.5, cz = size * 0.5, lon = w.meta.longitude, lat = -w.meta.latitude,
    );
    atomic_write(&target.join("world").join("config.cpp"), world.as_bytes())?;
    let data = format!(
        "class CfgPatches\n{{\n\tclass {id}_data\n\t{{\n\t\tunits[]={{}};\n\t\tweapons[]={{}};\n\t\trequiredVersion=0.1;\n\t\trequiredAddons[]={{}};\n\t}};\n}};\n"
    );
    atomic_write(&target.join("data").join("config.cpp"), data.as_bytes())?;
    atomic_write(&target.join("$PBOPREFIX$"), id.as_bytes())?;
    if has_navmesh {
        let nav = format!("class CfgPatches\n{{\n\tclass {id}_navmesh\n\t{{\n\t\trequiredAddons[] = {{}};\n\t}};\n}};\n");
        atomic_write(&target.join("navmesh").join("config.cpp"), nav.as_bytes())?;
    }
    Ok(())
}

pub fn run(w: &World, db: &ModelDb, opts: &ExportOptions, job: &Job) -> Result<ExportReport> {
    let id = w.meta.map_id.clone();
    ensure!(!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "The map id may only contain letters, digits and underscores");
    ensure!(w.meta.terrain_size % w.meta.cells_per_tile == 0 && w.meta.land_size() % w.meta.cells_per_tile == 0, "Terrain size must be divisible by the land grid");
    let limit = MAX_SLOTS.min(w.meta.max_tile_surfaces.max(1) as usize);
    let over = crate::mask_limit::count_over(w, limit);
    if over > 0 {
        job.log(format!("{over} tiles use more than {limit} surfaces, so the rarest are replaced in the export."));
    }
    // One shader for the whole map, as Terrain Builder does: the mode costs the satellite
    // normals, so it is only used when a tile really needs it.
    let six = limit == MAX_SLOTS && crate::mask_limit::count_over(w, SNX_SLOTS) > 0;
    if six {
        job.log("Some tiles use six surfaces: exporting with TerrainX, without satellite normal maps.".to_string());
    }
    ensure!(opts.root.is_dir(), "Work drive {} is not available", opts.root.display());
    let target = opts.root.join(&id);
    let mut report = ExportReport { folder: target.clone(), ..Default::default() };
    let layers_dir = target.join("data").join("layers");
    std::fs::create_dir_all(&layers_dir)?;
    std::fs::create_dir_all(target.join("world"))?;
    let layout = TileLayout::of(w);
    let tiles = layout.tiles;
    report.tiles = (tiles * tiles) as usize;

    let mut names: Vec<String> = Vec::new();
    let mut cell_mats: HashMap<(u32, u32), u16> = HashMap::new();
    if opts.layers || opts.world {
        job.stage(format!("Generating {} terrain tiles", report.tiles));
        let layer_tex: Vec<(String, String)> = w.layers.iter().map(|l| (nopx_of(&opts.root, &l.texture), l.texture.to_ascii_lowercase())).collect();
        // Stale layer files of an earlier export would end up in the PBO.
        if opts.layers {
            if let Ok(rd) = std::fs::read_dir(&layers_dir) {
                for e in rd.flatten() {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let done = std::sync::atomic::AtomicU32::new(0);
        let list: Vec<(u32, u32)> = (0..tiles).flat_map(|r| (0..tiles).map(move |c| (c, r))).collect();
        let outputs = Mutex::new(Vec::with_capacity(list.len()));
        list.par_iter().try_for_each(|&(c, r)| -> Result<()> {
            job.check()?;
            let o = export_tile(w, opts, &layout, &layers_dir, c, r, &layer_tex, six).with_context(|| format!("Tile {c}/{r}"))?;
            outputs.lock().unwrap().push(o);
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            job.set_progress(n as f32 / tiles.pow(2) as f32 * 0.85);
            Ok(())
        })?;
        let outputs = outputs.into_inner().unwrap();
        let mut all: BTreeSet<&String> = BTreeSet::new();
        for o in &outputs {
            all.extend(o.rvmats.keys());
        }
        report.rvmats = all.len();
        ensure!(all.len() < 65535, "Too many terrain materials");
        let index: HashMap<&String, u16> = all.iter().enumerate().map(|(i, n)| (*n, i as u16 + 1)).collect();
        for o in &outputs {
            for (cell, name) in &o.cells {
                cell_mats.insert(*cell, index[name]);
            }
        }
        names = all.iter().map(|n| format!("{}\\data\\layers\\{}", id.to_ascii_lowercase(), n)).collect();
    }
    if opts.world {
        job.stage("Writing WRP");
        job.set_progress(0.9);
        let land = w.meta.land_size();
        let mut material_index = vec![1u16; (land * land) as usize];
        for ((i, j), m) in &cell_mats {
            material_index[(*j * land + *i) as usize] = *m;
        }
        let elevations: Vec<f32> = w.height.data.par_iter().map(|h| h - w.meta.sea_level).collect();
        let (objects, missing) = wrp_objects(w, db);
        report.objects = objects.len();
        report.missing_models = missing;
        let wrp = Wrp {
            land_x: land, land_y: land, terrain_x: w.meta.terrain_size, terrain_y: w.meta.terrain_size,
            land_cell: w.meta.land_cell(), elevations, material_index, materials: names, objects,
            newer_format: w.meta.write_underground_flag,
            zd_signature: false,
        };
        let mut buf = Vec::with_capacity(wrp.elevations.len() * 4 + wrp.objects.len() * 110 + (1 << 20));
        wrp.write(&mut buf)?;
        atomic_write(&target.join("world").join(format!("{}.wrp", id.to_ascii_lowercase())), &buf)?;
    }
    if opts.config {
        job.stage("Writing configs");
        let nm = target.join("navmesh").join("navmesh.nm").is_file();
        if !nm {
            report.warnings.push("No navmesh yet: AI will not walk until navmesh.nm is generated.".to_string());
        }
        write_configs(w, &target, nm)?;
    }
    if opts.source {
        job.stage("Writing Terrain Builder source files");
        write_source(w, db, &opts.root.join(format!("{id}_Source")))?;
    }
    job.set_progress(1.0);
    Ok(report)
}

/// Files to continue the work in Terrain Builder.
pub fn write_source(w: &World, db: &ModelDb, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    crate::terrain::write_asc(&w.height, w.meta.easting, w.meta.northing, &dir.join("heightmap.asc"))?;
    let mut mask = RgbImage::new(w.mask.width, w.mask.height, [0, 0, 0]);
    mask.data.par_chunks_mut(3).zip(w.mask.data.par_iter()).for_each(|(p, l)| {
        p.copy_from_slice(&w.layers.get(*l as usize).map(|l| l.color).unwrap_or([0, 0, 0]));
    });
    mask.save_png(&dir.join("mask.png"))?;
    if let Some(s) = &w.satellite {
        s.save_png(&dir.join("satellite.png"))?;
    }
    atomic_write(&dir.join("layers.cfg"), crate::layers_cfg::write(&w.layers, "mapLegend.png").as_bytes())?;
    if !w.holes.is_empty() {
        atomic_write(&dir.join("holes.cfg"), crate::holes::class_text(&w.holes, "").as_bytes())?;
    }
    use std::fmt::Write;
    let mut text = String::with_capacity(w.objects.len() * 80);
    for (_, o) in w.objects.iter().filter(|(_, o)| w.object_exported(o)) {
        let path = w.objects.model_path(o);
        let stem = path.rsplit('\\').next().unwrap_or(path).trim_end_matches(".p3d");
        let (yaw, pitch, roll) = crate::objects::to_terrain_builder(o.yaw, o.pitch, o.roll);
        // Inverse of `tb_origin`.
        let up = rotation(o.yaw, o.pitch, o.roll, o.scale)[1];
        let center_y = if o.pitch == 0.0 && o.roll == 0.0 { 0.0 } else { db.get(path).center[1] };
        let _ = writeln!(text, "\"{stem}\";{:.6};{:.6};{:.6};{:.6};{:.6};{:.6};{:.6};{};",
            o.x as f64 + (up[0] * center_y) as f64 + w.meta.easting, o.z as f64 + (up[2] * center_y) as f64 + w.meta.northing, yaw, pitch, roll, o.scale, crate::objects::ObjectStore::origin_y(o, &w.height) - w.height.sample(o.x, o.z), u32::from(o.flags & FLAG_UNDERGROUND != 0));
    }
    atomic_write(&dir.join("objects.txt"), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::Obj;
    use crate::world::WorldMeta;

    fn world() -> World {
        let mut w = World::new(WorldMeta::new("TestMap", 512, 6.0), 40.0);
        w.mask = crate::terrain::MaskMap::new(1024, 1024, 0);
        for y in 0..1024 { for x in 512..1024 { w.mask.data[y * 1024 + x] = 2; } }
        let m = w.objects.intern("dz\\nope.p3d");
        w.objects.insert(Obj { x: 100.0, z: 200.0, elevation: 1.5, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: m, flags: FLAG_UNDERGROUND });
        w
    }

    #[test]
    fn layout_matches_a_terrain_builder_map() {
        // 12288 m, land cell 48: real rvmat of tile 000-000 has pos 0.125, 24.125.
        let mut w = World::new(WorldMeta::new("A", 2048, 6.0), 0.0);
        w.satellite = Some(RgbImage { width: 12288, height: 12288, data: vec![] });
        let l = TileLayout::of(&w);
        assert_eq!((l.tiles, l.step, l.tile_m, l.margin), (32, 384.0, 512.0, 64.0));
        assert_eq!(l.uv_pos(0, 0), (0.125, 24.125));
        let (x, z) = l.world(0, 0, 63.5, 63.5);
        assert!((x - 0.0).abs() < 1.1 && (z - 12288.0).abs() < 1.1, "{x} {z}");
    }

    #[test]
    fn real_rvmat_uv_values_are_reproduced() {
        let Ok(text) = std::fs::read_to_string("P:/DolinaBobru/data/layers/P_015-026_N_N_N_L20_N.rvmat") else { return };
        let mut w = World::new(WorldMeta::new("A", 2048, 6.0), 0.0);
        w.satellite = Some(RgbImage { width: 12288, height: 12288, data: vec![] });
        let l = TileLayout::of(&w);
        let (u, v) = l.uv_pos(15, 26);
        let pos = text.lines().find(|t| t.contains("pos[]")).unwrap();
        assert!(pos.contains(&format!("{u}")) && pos.contains(&format!("{v}")), "{pos} vs {u} {v}");
    }

    #[test]
    fn export_produces_consistent_files() {
        let d = tempfile::tempdir().unwrap();
        let w = world();
        let db = ModelDb::new(d.path());
        let opts = ExportOptions { root: d.path().to_path_buf(), layers: true, world: true, config: true, source: true };
        let r = run(&w, &db, &opts, &Job::default()).unwrap();
        assert_eq!(r.tiles, 64);
        assert_eq!(r.missing_models, vec!["dz\\nope.p3d".to_string()]);
        let target = d.path().join("TestMap");
        let wrp = Wrp::parse(&std::fs::read(target.join("world/testmap.wrp")).unwrap()).unwrap();
        assert_eq!(wrp.land_x, 64);
        assert_eq!(wrp.land_cell, 48.0);
        assert!(wrp.material_index.iter().all(|m| *m as usize >= 1 && *m as usize <= wrp.materials.len()));
        assert!(wrp.objects[0].underground);
        assert_eq!(wrp.objects[0].matrix[10], 41.5);
        for name in &wrp.materials {
            let f = name.rsplit('\\').next().unwrap();
            assert!(target.join("data/layers").join(f).is_file(), "{f}");
        }
        // West half uses layer 0, east half layer 2, and the name reflects it.
        assert!(wrp.materials.iter().any(|m| m.contains("_L00")));
        assert!(wrp.materials.iter().any(|m| m.contains("_L02")));
        let mat = std::fs::read_to_string(target.join("data/layers").join(wrp.materials[0].rsplit('\\').next().unwrap())).unwrap();
        assert!(mat.contains("TerrainSNX") && mat.contains("Stage14") && mat.contains("en_grass1_ca.paa"));
        let cfg = std::fs::read_to_string(target.join("world/config.cpp")).unwrap();
        assert_eq!(cfg.contains("class Holes"), !w.holes.is_empty());
        assert!(cfg.contains("class TestMap: Enoch") && cfg.contains("TestMap\\world\\testmap.wrp"));
        assert!(target.join("data/layers/M_000_000_lca.paa").is_file());
        assert!(d.path().join("TestMap_Source/heightmap.asc").is_file());
        assert!(r.warnings.iter().any(|w| w.contains("navmesh")));
    }

    #[test]
    fn holes_go_into_the_world_config() {
        let d = tempfile::tempdir().unwrap();
        let mut w = World::new(WorldMeta::new("A", 64, 8.0), 10.0);
        w.holes.insert([3, 4]);
        std::fs::create_dir_all(d.path().join("world")).unwrap();
        std::fs::create_dir_all(d.path().join("data")).unwrap();
        write_configs(&w, d.path(), false).unwrap();
        let cfg = std::fs::read_to_string(d.path().join("world/config.cpp")).unwrap();
        assert!(cfg.contains("class Holes") && cfg.contains("tiles[]={{3,4}};"), "{cfg}");
    }

    #[test]
    fn object_position_includes_model_center_rotated() {
        // Uses a fake header through ModelDb's public file path.
        let d = tempfile::tempdir().unwrap();
        let mut head = vec![0u8; 400];
        head[..4].copy_from_slice(b"ODOL");
        head[4] = 56;
        head[8] = 1;
        let at = 12 + 4 + 96;
        for (k, v) in [1.0f32, 2.0, 0.0].iter().enumerate() {
            head[at + k * 4..at + k * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        std::fs::write(d.path().join("m.p3d"), &head).unwrap();
        let mut w = World::new(WorldMeta::new("A", 64, 8.0), 10.0);
        let m = w.objects.intern("m.p3d");
        w.objects.insert(Obj { x: 50.0, z: 50.0, elevation: 0.0, yaw: 90.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: m, flags: 0 });
        let (o, missing) = wrp_objects(&w, &ModelDb::new(d.path()));
        assert!(missing.is_empty());
        assert!((o[0].matrix[9] - 50.0).abs() < 1e-3);
        assert!((o[0].matrix[10] - 12.0).abs() < 1e-3);
        assert!((o[0].matrix[11] - 49.0).abs() < 1e-3);
    }
}
