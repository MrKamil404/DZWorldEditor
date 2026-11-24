//! Writes everything a map PBO needs into the work drive folder.
//!
//! The terrain tiles are written as the game's own maps have them (Chernarus, Livonia and
//! Sakhal alike): every tile material uses the `TerrainX` shader with up to six surfaces, told
//! apart in the mask by colour (black, red, green, blue) and, for the three blue ones, by alpha.
//! The game has no use for `TerrainSNX` (four or five surfaces with a satellite normal map), which
//! Arma 3's Terrain Builder can also write: with it, surfaces and their edges come out wrong.
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

/// Surfaces a tile can hold with the `TerrainX` shader.
pub const MAX_SLOTS: usize = 6;

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

/// RGBA of mask slot `slot` (0-based), as Terrain Builder and the game's maps encode it.
#[inline]
fn slot_rgba(slot: usize) -> [u8; 4] {
    match slot {
        0 => [0, 0, 0, 255],
        1 => [255, 0, 0, 255],
        2 => [0, 255, 0, 255],
        3 => [0, 0, 255, 255],
        4 => [0, 0, 255, 128],
        _ => [0, 0, 255, 0],
    }
}

/// The colour a slot has in the mask, by which DXT tells slots apart: black, red, green, blue
/// (the three blue slots share it and differ only in alpha).
#[inline]
fn colour_class(slot: u8) -> u8 {
    slot.min(3)
}

/// Masks are compressed in blocks of 4 by 4 pixels (4 metres), and a block can only hold two
/// colours exactly; a third comes out as a mix of the others, which the game draws as a blend of
/// surfaces with ragged, wrong edges. So in each block only the two colours most used stay, and a
/// pixel of any other takes the slot most of its neighbours in the block have. Specks of a third
/// surface at the edge of two others are lost; the edges stay clean.
pub(crate) fn two_colours_a_block(slots: &mut [u8], px: usize) {
    for by in (0..px).step_by(4) {
        for bx in (0..px).step_by(4) {
            let cells: Vec<usize> = (by..(by + 4).min(px)).flat_map(|y| (bx..(bx + 4).min(px)).map(move |x| y * px + x)).collect();
            let mut count = [0u32; 4];
            for &c in &cells {
                count[colour_class(slots[c]) as usize] += 1;
            }
            if count.iter().filter(|n| **n > 0).count() <= 2 {
                continue;
            }
            let mut classes = [0u8, 1, 2, 3];
            classes.sort_by_key(|k| std::cmp::Reverse(count[*k as usize]));
            let keep = [classes[0], classes[1]];
            // The slot each kept colour stands for in this block: the one used most.
            let slot_for = |class: u8| -> u8 {
                let mut n = [0u32; 256];
                for &c in &cells {
                    if colour_class(slots[c]) == class { n[slots[c] as usize] += 1; }
                }
                (0..=255u8).max_by_key(|s| n[*s as usize]).unwrap_or(class)
            };
            let fill = [slot_for(keep[0]), slot_for(keep[1])];
            // The block as it was, so that changed pixels do not vote.
            let before: std::collections::HashMap<usize, u8> = cells.iter().map(|c| (*c, slots[*c])).collect();
            for &c in &cells {
                if keep.contains(&colour_class(before[&c])) {
                    continue;
                }
                // The kept colour more of its neighbours in the block have; the block's main one
                // on a tie.
                let (x, y) = ((c % px) as i64, (c / px) as i64);
                let mut votes = [0u32; 2];
                for dy in -1..=1i64 {
                    for dx in -1..=1i64 {
                        let (nx, ny) = (x + dx, y + dy);
                        if nx < bx as i64 || ny < by as i64 || nx >= (bx + 4).min(px) as i64 || ny >= (by + 4).min(px) as i64 {
                            continue;
                        }
                        let k = colour_class(before[&(ny as usize * px + nx as usize)]);
                        if k == keep[0] { votes[0] += 1 } else if k == keep[1] { votes[1] += 1 }
                    }
                }
                slots[c] = if votes[1] > votes[0] { fill[1] } else { fill[0] };
            }
        }
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
pub fn rvmat(id: &str, name_col: u32, name_row: u32, layout: &TileLayout, slots: usize, used: &[Option<(String, String)>]) -> String {
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
    s += "ambient[]={1,1,1,1};\ndiffuse[]={1,1,1,1};\n";
    s += "forcedDiffuse[]={0.02,0.02,0.02,1};\nemmisive[]={0,0,0,0};\nspecular[]={0,0,0,0};\nspecularPower=0;\n";
    s += &format!("class Stage0\n{{\n\ttexture=\"{id_lc}\\data\\layers\\s_{tile}_lco.paa\";\n\ttexGen=3;\n}};\n");
    s += &format!("class Stage1\n{{\n\ttexture=\"{id_lc}\\data\\layers\\m_{tile}_lca.paa\";\n\ttexGen=4;\n}};\n");
    s += &world_gen(3);
    s += &world_gen(4);
    s += &tex_gen(0, 1);
    s += &tex_gen(1, 10);
    s += &tex_gen(2, 10);
    s += "PixelShaderID=\"TerrainX\";\nVertexShaderID=\"Terrain\";\n";
    s += "class Stage2\n{\n\ttexture=\"#(rgb,1,1,1)color(0.5,0.5,0.5,1,cdt)\";\n\ttexGen=0;\n};\n";
    for k in 0..slots {
        let (nopx, ca) = match &used[k] {
            Some(p) => (p.0.as_str(), p.1.as_str()),
            None => ("", ""),
        };
        s += &format!("class Stage{}\n{{\n\ttexture=\"{nopx}\";\n\ttexGen=1;\n}};\n", 3 + k * 2);
        s += &format!("class Stage{}\n{{\n\ttexture=\"{ca}\";\n\ttexGen=2;\n}};\n", 4 + k * 2);
    }
    s
}

struct TileOutput {
    /// land cell (i, j) -> rvmat file name
    cells: Vec<((u32, u32), String)>,
    rvmats: BTreeMap<String, String>,
}

/// The surfaces of a tile in slot order, and the slot of every pixel of its mask as the game gets
/// it: at most as many surfaces as the material takes (the rarest replaced by the most used), and
/// at most two colours in each block of 4 by 4 pixels. The export writes this, and the editor
/// draws it, so that both show the same ground.
pub fn tile_slots(w: &World, layout: &TileLayout, col: u32, row: u32) -> (Vec<usize>, Vec<u8>) {
    let px = layout.px as usize;
    let size = w.meta.size_m();
    let mut indices = vec![0u8; px * px];
    let mut count = [0u32; 256];
    for y in 0..px {
        for x in 0..px {
            let (wx, wz) = layout.world(col, row, x as f32, y as f32);
            let l = w.mask.sample(wx.clamp(0.0, size - 0.01), wz.clamp(0.0, size - 0.01), size);
            indices[y * px + x] = l;
            count[l as usize] += 1;
        }
    }
    // The most used surfaces of the tile, as many as the material can take.
    let mut order: Vec<usize> = (0..256).filter(|l| count[*l] > 0).collect();
    order.sort_by_key(|l| std::cmp::Reverse(count[*l]));
    order.truncate(MAX_SLOTS.min(w.meta.max_tile_surfaces.max(1) as usize));
    order.sort();
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
    let mut pixel_slot: Vec<u8> = indices.iter().map(|l| slot_of[*l as usize]).collect();
    two_colours_a_block(&mut pixel_slot, px);
    (order, pixel_slot)
}

/// The pixels of the mask whose surface in the game can change when those of `r` change: the
/// whole of every tile whose texture (overlap included) shows them, as a tile's surfaces and
/// blocks are chosen over all of it.
pub fn game_mask_area(w: &World, r: crate::terrain::Rect) -> crate::terrain::Rect {
    let layout = TileLayout::of(w);
    let (mw, mh) = (w.mask.width as f32, w.mask.height as f32);
    let per = mw / layout.size;
    let reach = ((layout.margin + 4.0) * per).ceil() as usize;
    let (x0, y0) = (r.x0.saturating_sub(reach), r.y0.saturating_sub(reach));
    let (x1, y1) = ((r.x1 + reach).min(mw as usize - 1), (r.y1 + reach).min(mh as usize - 1));
    let step = layout.step * per;
    let snap_lo = |v: usize| ((v as f32 / step).floor() * step) as usize;
    let snap_hi = |v: usize, max: f32| ((((v as f32 + 1.0) / step).ceil() * step) as usize).min(max as usize) - 1;
    crate::terrain::Rect { x0: snap_lo(x0), y0: snap_lo(y0), x1: snap_hi(x1, mw), y1: snap_hi(y1, mh) }
}

/// The surface the game shows at each pixel of the mask within `area` (row by row), as the
/// exported tiles give it: each point is drawn by the tile of its land cell.
pub fn game_mask(w: &World, area: crate::terrain::Rect) -> Vec<u8> {
    let layout = TileLayout::of(w);
    let (mw, mh) = (w.mask.width, w.mask.height);
    let size = w.meta.size_m();
    let px = layout.px as usize;
    let mpp = layout.tile_m / layout.px as f32;
    let world = |x: usize, y: usize| ((x as f32 + 0.5) / mw as f32 * size, (1.0 - (y as f32 + 0.5) / mh as f32) * size);
    let tile_of = |wx: f32, wz: f32| (((wx / layout.step) as u32).min(layout.tiles - 1), (((size - wz) / layout.step) as u32).min(layout.tiles - 1));
    // The tiles the area takes its pixels from, each worked out once.
    let (a, b) = (world(area.x0, area.y0), world(area.x1, area.y1));
    let (c0, r0) = tile_of(a.0, a.1);
    let (c1, r1) = tile_of(b.0, b.1);
    let tiles: Vec<(u32, u32)> = (r0..=r1).flat_map(|r| (c0..=c1).map(move |c| (c, r))).collect();
    let done: HashMap<(u32, u32), (Vec<usize>, Vec<u8>)> = tiles.par_iter().map(|&(c, r)| ((c, r), tile_slots(w, &layout, c, r))).collect();
    let width = area.width();
    let mut out = vec![0u8; width * area.height()];
    out.par_chunks_mut(width).enumerate().for_each(|(k, line)| {
        let y = area.y0 + k;
        for (i, v) in line.iter_mut().enumerate() {
            let (wx, wz) = world(area.x0 + i, y);
            let (col, row) = tile_of(wx, wz);
            let (order, slots) = &done[&(col, row)];
            let tx = (((wx - (col as f32 * layout.step - layout.margin)) / mpp) as usize).min(px - 1);
            let ty = ((((size + layout.margin - row as f32 * layout.step) - wz) / mpp) as usize).min(px - 1);
            *v = order.get(slots[ty * px + tx] as usize).copied().unwrap_or(0) as u8;
        }
    });
    out
}

fn export_tile(w: &World, opts: &ExportOptions, layout: &TileLayout, dir: &Path, col: u32, row: u32, layer_tex: &[(String, String)]) -> Result<TileOutput> {
    let px = layout.px as usize;
    let size = w.meta.size_m();
    let id = &w.meta.map_id;
    let _ = opts;
    let mut sat = vec![255u8; px * px * 4];
    for y in 0..px {
        for x in 0..px {
            let (wx, wz) = layout.world(col, row, x as f32, y as f32);
            let (cx, cz) = (wx.clamp(0.0, size - 0.01), wz.clamp(0.0, size - 0.01));
            let c = match &w.satellite {
                Some(s) => sample_rgb(s, cx, cz, size),
                None => synthetic_rgb(w, cx, cz),
            };
            sat[(y * px + x) * 4..(y * px + x) * 4 + 3].copy_from_slice(&c);
        }
    }
    let (order, pixel_slot) = tile_slots(w, layout, col, row);
    let slots = order.len();
    let mut mask = vec![0u8; px * px * 4];
    for (i, s) in pixel_slot.iter().enumerate() {
        mask[i * 4..i * 4 + 4].copy_from_slice(&slot_rgba(*s as usize));
    }
    spread_blue_alpha(&mut mask, px);
    let tile = format!("{col:03}_{row:03}");
    atomic_write(&dir.join(format!("S_{tile}_lco.paa")), &paa_write::encode(&sat, px, px, Format::Dxt1))?;
    atomic_write(&dir.join(format!("M_{tile}_lca.paa")), &paa_write::encode(&mask, px, px, Format::Dxt5))?;

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
                    present[pixel_slot[y * px + x] as usize] = true;
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
                out.rvmats.insert(name.clone(), rvmat(id, col, row, layout, slots, &used));
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

/// A text value for a config, with its quotes doubled as the config format wants.
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.trim().replace('"', "\"\""))
}

/// The settings of the world class the config sets itself, beyond those it always writes.
fn world_overrides(cfg: &crate::world::MapConfig, own_ce: Option<String>) -> String {
    let mut out = String::new();
    let mut text = |key: &str, v: &str| {
        if !v.trim().is_empty() {
            out += &format!("\t\t{key}={};\n", quoted(v));
        }
    };
    // The map's own economy, when it has a mission and no other folder was named.
    let ce = if cfg.ce_files.trim().is_empty() { own_ce.unwrap_or_default() } else { cfg.ce_files.clone() };
    text("ceFiles", &ce);
    text("startTime", &cfg.start_time);
    text("startDate", &cfg.start_date);
    text("mapDisplayNameKey", &cfg.map_display_name_key);
    text("mapDescriptionKey", &cfg.map_description_key);
    text("mapTextureClosed", &cfg.map_texture_closed);
    text("mapTextureOpened", &cfg.map_texture_opened);
    text("mapTextureLegend", &cfg.map_texture_legend);
    text("userMapPath", &cfg.user_map_path);
    text("midDetailTexture", &cfg.mid_detail_texture);
    text("terrainNormalTexture", &cfg.terrain_normal_texture);
    text("oceanMaterial", &cfg.ocean_material);
    text("oceanNiceMaterial", if cfg.ocean_nice_material.trim().is_empty() { &cfg.ocean_material } else { &cfg.ocean_nice_material });
    text("oceanStormMaterial", if cfg.ocean_storm_material.trim().is_empty() { &cfg.ocean_material } else { &cfg.ocean_storm_material });
    text("envTexture", &cfg.env_texture);
    text("atmosphereTexture", &cfg.atmosphere_texture);
    let mut number = |key: &str, v: Option<f32>| {
        if let Some(v) = v {
            out += &format!("\t\t{key}={v};\n");
        }
    };
    number("MaxTide", cfg.max_tide);
    number("MaxWave", cfg.max_wave);
    number("waveAmplitude", cfg.wave_amplitude);
    number("waveFrequency", cfg.wave_frequency);
    number("waveOctaves", cfg.wave_octaves.map(|v| v as f32));
    number("wavePersistence", cfg.wave_persistence);
    number("starsIntensity", cfg.stars_intensity);
    number("hazeDistCoef", cfg.haze_dist_coef);
    number("hazeFogCoef", cfg.haze_fog_coef);
    number("volFogOffset", cfg.vol_fog_offset);
    number("satelliteNormalBlendStart", cfg.satellite_normal_blend_start);
    number("satelliteNormalBlendEnd", cfg.satellite_normal_blend_end);
    number("terrainNormalPower", cfg.terrain_normal_power);
    number("soundMapAttenCoef", cfg.sound_map_atten_coef);
    number("clutterGrid", cfg.clutter_grid);
    number("clutterDist", cfg.clutter_dist);
    number("noDetailDist", cfg.no_detail_dist);
    number("fullDetailDist", cfg.full_detail_dist);
    number("minTreesInForestSquare", cfg.min_trees_in_forest_square.map(|v| v as f32));
    number("minRocksInRockSquare", cfg.min_rocks_in_rock_square.map(|v| v as f32));
    if let Some(on) = cfg.height_blending_mode {
        out += &format!("\t\theightBlendingMode={};\n", on as u8);
    }
    if let Some(on) = cfg.bicubic_mode {
        out += &format!("\t\tbicubicMode={};\n", on as u8);
    }
    if let Some(sm) = &cfg.sound_map {
        out += &format!(
            "\t\tclass SoundMapValues: SoundMapValues\n\t\t{{\n\t\t\ttreehard={};\n\t\t\ttreesoft={};\n\t\t\tbushhard={};\n\t\t\tbushsoft={};\n\t\t\tforest={};\n\t\t\thouse={};\n\t\t\tchurch={};\n\t\t}};\n",
            sm.treehard, sm.treesoft, sm.bushhard, sm.bushsoft, sm.forest, sm.house, sm.church
        );
    }
    if !cfg.cutscenes.is_empty() {
        let list = cfg.cutscenes.iter().filter(|c| !c.trim().is_empty()).map(|c| quoted(c)).collect::<Vec<_>>().join(",");
        out += &format!("\t\tcutscenes[]={{{list}}};\n");
    }
    let outside = !cfg.outside_satellite.trim().is_empty() || cfg.outside_synth.is_some() || !cfg.outside_texture.trim().is_empty() || !cfg.outside_nopx.trim().is_empty();
    if outside {
        out += "\t\tclass OutsideTerrain: OutsideTerrain\n\t\t{\n";
        if !cfg.outside_satellite.trim().is_empty() {
            out += &format!("\t\t\tsatellite={};\n", quoted(&cfg.outside_satellite));
        }
        if let Some(on) = cfg.outside_synth {
            out += &format!("\t\t\tenableTerrainSynth={};\n", on as u8);
        }
        if !cfg.outside_texture.trim().is_empty() || !cfg.outside_nopx.trim().is_empty() {
            out += "\t\t\tclass Layers: Layers\n\t\t\t{\n\t\t\t\tclass Layer0: Layer0\n\t\t\t\t{\n";
            if !cfg.outside_nopx.trim().is_empty() {
                out += &format!("\t\t\t\t\tnopx={};\n", quoted(&cfg.outside_nopx));
            }
            if !cfg.outside_texture.trim().is_empty() {
                out += &format!("\t\t\t\t\ttexture={};\n", quoted(&cfg.outside_texture));
            }
            out += "\t\t\t\t};\n\t\t\t};\n";
        }
        out += "\t\t};\n";
    }
    out
}

/// `cfgCharacterScenes` of the map: the places its main menu shows.
fn character_scenes(id: &str, scenes: &[crate::world::CharacterScene]) -> String {
    if scenes.is_empty() {
        return String::new();
    }
    let mut out = format!("class cfgCharacterScenes\n{{\n\tclass {id}\n\t{{\n");
    for (k, s) in scenes.iter().enumerate() {
        let name: String = s.name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        let name = if name.is_empty() { format!("loc{}", k + 1) } else { name };
        let [px, pz, py] = s.position;
        let [tx, tz, ty] = s.target;
        let d = s.date;
        out += &format!(
            "\t\tclass {name}\n\t\t{{\n\t\t\ttarget[]={{{tx:.1},{tz:.1},{ty:.1}}};\n\t\t\tposition[]={{{px:.1},{pz:.1},{py:.1}}};\n\t\t\tfov={:.4};\n\t\t\tdate[]={{{},{},{},{},{}}};\n\t\t\tovercast={:.2};\n\t\t\train={:.2};\n\t\t\tfog={:.2};\n\t\t}};\n",
            s.fov, d[0], d[1], d[2], d[3], d[4], s.overcast, s.rain, s.fog
        );
    }
    out += "\t};\n};\n";
    out
}

/// Lines of free text indented into a class, each by `indent`.
fn indented(text: &str, indent: &str) -> String {
    text.lines().filter(|l| !l.trim().is_empty()).map(|l| format!("{indent}{}\n", l.trim_end())).collect()
}

/// The world config (config.cpp of the world folder) of a map, from the project and the
/// settings of its config.
pub fn world_config_text(w: &World, has_navmesh: bool) -> String {
    let id = &w.meta.map_id;
    let cfg = &w.meta.config;
    let size = w.meta.size_m();
    let (sx, sz) = (w.meta.spawn[0], w.meta.spawn[1]);
    let sy = w.height.sample(sx, sz) - w.meta.sea_level;
    let center = cfg.center.unwrap_or([size * 0.5, size * 0.5]);
    let cy = w.height.sample(center[0], center[1]) - w.meta.sea_level;
    let mut materials = String::new();
    for (i, l) in w.layers.iter().enumerate() {
        materials += &format!("\t\t\tmaterial{i}=\"{}\";\n", l.material);
    }
    let navmesh = {
        let mut gp = String::new();
        if has_navmesh {
            gp += &format!("\t\t\t\tseedPosition[]={{{sx:.1},{sy:.1},{sz:.1}}};\n");
        }
        if let Some(tw) = cfg.navmesh_tile_width {
            gp += &format!("\t\t\t\ttileWidth={tw};\n");
        }
        if let Some(fi) = cfg.navmesh_filter_islands {
            gp += &format!("\t\t\t\tfilterIsolatedIslands={};\n", fi as u8);
        }
        if cfg.navmesh_max_slope.is_some() || cfg.navmesh_max_step.is_some() {
            gp += "\t\t\t\tclass Agent: Agent\n\t\t\t\t{\n";
            if let Some(s) = cfg.navmesh_max_step {
                gp += &format!("\t\t\t\t\tmaxStepHeight={s};\n");
            }
            if let Some(s) = cfg.navmesh_max_slope {
                gp += &format!("\t\t\t\t\tmaxSlope={s};\n");
            }
            gp += "\t\t\t\t};\n";
        }
        let mut nm = String::from("\t\tclass Navmesh: Navmesh\n\t\t{\n");
        if has_navmesh {
            nm += &format!("\t\t\tnavmeshName=\"\\{id}\\navmesh\\navmesh.nm\";\n");
        } else {
            nm += "\t\t\tnavmeshName=\"\";\n";
        }
        if let Some(fi) = cfg.navmesh_filter_islands {
            nm += &format!("\t\t\tfilterIsolatedIslandsOnLoad={};\n", fi as u8);
        }
        if !gp.is_empty() {
            nm += &format!("\t\t\tclass GenParams: GenParams\n\t\t\t{{\n{gp}\t\t\t}};\n");
        }
        nm += "\t\t};\n";
        nm
    };
    let holes = if w.holes.is_empty() { String::new() } else { crate::holes::class_text(&w.holes, "\t\t") };
    let names = crate::keypoints::names_text(&w.keypoints, "\t\t\t");
    let base = cfg.base.class();
    let mut addons = vec!["DZ_Data".to_string(), cfg.base.addon().to_string()];
    for a in &cfg.required_addons {
        let a = a.trim().trim_matches('"');
        if !a.is_empty() && !addons.iter().any(|x| x.eq_ignore_ascii_case(a)) {
            addons.push(a.to_string());
        }
    }
    let addons = addons.iter().filter(|a| !a.is_empty()).map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(",");
    let description = if cfg.description.trim().is_empty() { w.meta.name.clone() } else { cfg.description.clone() };
    let zoom = |k: usize, max: &str, digits: &str| {
        let step = cfg.grid_steps[k].max(1);
        format!("\t\t\tclass Zoom{}\n\t\t\t{{\n\t\t\t\tzoomMax={max};\n\t\t\t\tformat=\"XY\";\n\t\t\t\tformatX=\"{digits}\";\n\t\t\t\tformatY=\"{digits}\";\n\t\t\t\tstepX={step};\n\t\t\t\tstepY=-{step};\n\t\t\t}};\n", k + 1)
    };
    let grid = format!("{}{}{}", zoom(0, "0.15000001", "000"), zoom(1, "0.85000002", "00"), zoom(2, "1e+30", "0"));
    let extra_world = indented(&cfg.extra_world, "\t\t");
    let extra_file = indented(&cfg.extra_file, "");
    let overrides = world_overrides(cfg, w.mission.is_some().then(|| ce_folder(id)));
    let scenes = character_scenes(id, &cfg.scenes);
    let world = format!(
"class CfgPatches
{{
	class {id}_world
	{{
		units[]={{}};
		weapons[]={{}};
		requiredVersion=0.1;
		requiredAddons[]={{{addons}}};
		author=\"{author}\";
		name=\"{name}\";
	}};
}};
{scenes}class CfgWorlds
{{
	class CAWorld;
	class {base}: CAWorld
	{{
		class Navmesh
		{{
			class GenParams
			{{
				class Agent;
			}};
		}};
		class SoundMapValues;
		class UsedTerrainMaterials;
		class OutsideTerrain
		{{
			class Layers
			{{
				class Layer0;
			}};
		}};
	}};
	class {id}: {base}
	{{
		description=\"{description}\";
		hasOcean={ocean};
		worldName=\"{id}\\world\\{id_lc}.wrp\";
		initWorld=\"{id}\";
		cutscenes[]={{}};
		centerPosition[]={{{cx:.1},{cz:.1},{cy:.1}}};
		longitude={lon};
		latitude={lat};
{overrides}{navmesh}{holes}		class Grid
		{{
			offsetX=0;
			offsetY={size:.0};
{grid}		}};
		class UsedTerrainMaterials: UsedTerrainMaterials
		{{
{materials}		}};
		class Names
		{{
{names}		}};
{extra_world}	}};
}};
class CfgWorldList
{{
	class {id}
	{{
	}};
}};
{extra_file}",
        author = w.meta.author, name = w.meta.name, id_lc = id.to_ascii_lowercase(), ocean = cfg.has_ocean as u8,
        cx = center[0], cz = center[1], lon = w.meta.longitude, lat = -w.meta.latitude,
    );
    world
}

/// The folder of the map's Central Economy on the work drive, as `ceFiles` names it.
pub fn ce_folder(id: &str) -> String {
    format!("{id}\\ce")
}

/// First line of the config of a `ce` folder this editor writes; one without it is the user's own
/// and is left alone.
const CE_MARK: &str = "// Central Economy of the map, written by DZ World Editor from the project's mission.";

/// Writes the mission of the project as the map's Central Economy folder (`ce`), as the worlds
/// of the game carry theirs: every file the economy reads, without `init.c` and persistence,
/// with a config of its own so it packs as an addon. Returns whether there was one to write.
pub fn write_ce(w: &World, target: &Path) -> Result<bool> {
    let dir = target.join("ce");
    let config = dir.join("config.cpp");
    let ours = !dir.exists() || std::fs::read_to_string(&config).is_ok_and(|t| t.starts_with(CE_MARK));
    let Some(m) = &w.mission else {
        if ours && dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        return Ok(false);
    };
    anyhow::ensure!(ours, "{} is not one this editor wrote; move it away to have the mission written there", dir.display());
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    let mut copy = (**m).clone();
    copy.write(&dir, false, false)?;
    for rel in ["init.c"] {
        let _ = std::fs::remove_file(dir.join(rel));
    }
    let id = &w.meta.map_id;
    let text = format!("{CE_MARK}\nclass CfgPatches\n{{\n\tclass {id}_CE\n\t{{\n\t\tunits[]={{}};\n\t\tweapons[]={{}};\n\t\trequiredVersion=0.1;\n\t\trequiredAddons[]={{\"DZ_Data\"}};\n\t}};\n}};\n");
    atomic_write(&config, text.as_bytes())?;
    Ok(true)
}

pub fn write_configs(w: &World, target: &Path, has_navmesh: bool) -> Result<()> {
    let id = &w.meta.map_id;
    let world = world_config_text(w, has_navmesh);
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
        let layer_tex: Vec<(String, String)> = w.layers.iter().map(|l| (crate::layer_material::nopx_of_material(&opts.root, &l.material).map(|n| n.to_ascii_lowercase()).unwrap_or_else(|| nopx_of(&opts.root, &l.texture)), l.texture.to_ascii_lowercase())).collect();
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
            let o = export_tile(w, opts, &layout, &layers_dir, c, r, &layer_tex).with_context(|| format!("Tile {c}/{r}"))?;
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
        match write_ce(w, &target) {
            Ok(true) => job.log(format!("Mission written as the Central Economy of the map ({}).", ce_folder(&id))),
            Ok(false) => {}
            Err(e) => report.warnings.push(format!("{e:#}")),
        }
        report.warnings.extend(crate::surfaces::check_layers(&opts.root, &id, &w.layers, &w.meta.build_folders));
        if crate::surfaces::write_config(&id, &w.meta.config.surfaces, &opts.root, &target)? {
            job.log(format!("{} surfaces of the map written to surfaces\\config.cpp.", w.meta.config.surfaces.len()));
        }
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

    #[test]
    fn the_game_mask_is_what_the_exported_tiles_hold() {
        let mut w = World::new(WorldMeta::new("Game", 512, 4.0), 10.0);
        w.mask = crate::terrain::MaskMap::new(2048, 2048, 0);
        // Blocks start a margin before the tile, a metre a pixel here.
        let layout = TileLayout::of(&w);
        let start = 1000 + (-layout.margin).rem_euclid(4.0) as usize;
        // A speck of surface 2 in a block where 0 and 1 meet: the game drops it.
        for y in 0..2048 {
            for x in start + 2..2048 {
                w.mask.data[y * 2048 + x] = 1;
            }
        }
        w.mask.data[500 * 2048 + start + 1] = 2;
        let area = crate::terrain::Rect { x0: 990, y0: 490, x1: 1019, y1: 509 };
        let g = game_mask(&w, area);
        let at = |x: usize, y: usize| g[(y - 490) * 30 + (x - 990)];
        assert_eq!((at(start, 500), at(start + 2, 500)), (0, 1), "the edge stays");
        assert_ne!(at(start + 1, 500), 2, "the speck goes as in the game");
    }

    #[test]
    fn mask_blocks_keep_two_colours_and_compress_without_mixes() {
        // Concrete (slot 1) beside grass (slot 2) speckled with gravel (slot 0), and a blue slot.
        let px = 32;
        let mut slots: Vec<u8> = (0..px * px).map(|i| {
            let (x, y) = (i % px, i / px);
            if x < 14 { 1 } else if (x * 7 + y * 13) % 5 == 0 { 0 } else if y > 24 { 4 } else { 2 }
        }).collect();
        two_colours_a_block(&mut slots, px);
        for by in (0..px).step_by(4) {
            for bx in (0..px).step_by(4) {
                let mut seen = std::collections::BTreeSet::new();
                for y in by..by + 4 { for x in bx..bx + 4 { seen.insert(colour_class(slots[y * px + x])); } }
                assert!(seen.len() <= 2, "block {bx},{by}: {seen:?}");
            }
        }
        // The concrete edge stays where it was.
        assert!((0..px).all(|y| slots[y * px + 13] == 1 && slots[y * px + 15] != 1));
        let mut rgba = Vec::new();
        for s in &slots { rgba.extend_from_slice(&slot_rgba(*s as usize)); }
        let t = forest_core::paa::decode(&paa_write::encode(&rgba, px, px, Format::Dxt5), 4096).unwrap();
        // Every pixel comes back as the very colour of its slot: no mixes for the game to blend.
        for (i, s) in slots.iter().enumerate() {
            assert_eq!(&t.rgba[i * 4..i * 4 + 4], &slot_rgba(*s as usize), "pixel {i}");
        }
    }
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
        // The shader of the game's own maps, never TerrainSNX.
        assert!(mat.contains("\"TerrainX\"") && !mat.contains("TerrainSNX") && !mat.contains("Stage14") && mat.contains("en_grass1_ca.paa"));
        assert!(!target.join("data/layers/N_000_000_nohq.paa").exists());
        let cfg = std::fs::read_to_string(target.join("world/config.cpp")).unwrap();
        assert_eq!(cfg.contains("class Holes"), !w.holes.is_empty());
        assert!(cfg.contains("class TestMap: Enoch") && cfg.contains("TestMap\\world\\testmap.wrp"));
        assert!(target.join("data/layers/M_000_000_lca.paa").is_file());
        assert!(d.path().join("TestMap_Source/heightmap.asc").is_file());
        assert!(r.warnings.iter().any(|w| w.contains("navmesh")));
    }

    #[test]
    fn the_world_config_takes_its_settings() {
        let mut w = World::new(WorldMeta::new("Isle", 64, 8.0), 10.0);
        let plain = world_config_text(&w, false);
        assert!(plain.contains("class Isle: Enoch") && plain.contains("\"DZ_Data\",\"DZ_Worlds_Enoch\"") && plain.contains("description=\"Isle\"") && plain.contains("hasOcean=1"), "{plain}");
        assert!(plain.contains("centerPosition[]={256.0,256.0,"), "{plain}");
        w.meta.config.base = crate::world::BaseWorld::ChernarusPlus;
        w.meta.config.description = "The isle".into();
        w.meta.config.has_ocean = false;
        w.meta.config.center = Some([100.0, 200.0]);
        w.meta.config.grid_steps = [50, 500, 5000];
        w.meta.config.required_addons = vec!["MyMod_Data".into(), "DZ_Data".into()];
        w.meta.config.extra_world = "minTreesInForestSquare=10;\n\nclass Ambient {};".into();
        w.meta.config.extra_file = "class CfgMods {};".into();
        let cfg = world_config_text(&w, false);
        assert!(cfg.contains("class ChernarusPlus: CAWorld") && cfg.contains("class Isle: ChernarusPlus"), "{cfg}");
        assert!(cfg.contains("requiredAddons[]={\"DZ_Data\",\"DZ_Worlds_Chernarusplus\",\"MyMod_Data\"};"), "{cfg}");
        assert!(cfg.contains("description=\"The isle\"") && cfg.contains("hasOcean=0") && cfg.contains("centerPosition[]={100.0,200.0,"), "{cfg}");
        assert!(cfg.contains("stepX=50;") && cfg.contains("stepY=-5000;"), "{cfg}");
        assert!(cfg.contains("\t\tminTreesInForestSquare=10;\n\t\tclass Ambient {};\n\t};"), "{cfg}");
        assert!(cfg.trim_end().ends_with("class CfgMods {};"), "{cfg}");
        assert!(!cfg.contains("cfgCharacterScenes") && !cfg.contains("startTime") && !cfg.contains("class OutsideTerrain: OutsideTerrain"), "nothing set, nothing written");
        // What the editor sets is written over what the base world has.
        w.meta.config.scenes = vec![crate::world::CharacterScene { name: "harbour".into(), position: [100.0, 200.0, 1.5], target: [120.0, 230.0, 14.0], ..Default::default() }];
        w.meta.config.cutscenes = vec!["IsleIntro".into()];
        w.meta.config.start_time = "6:30".into();
        w.meta.config.ce_files = "Isle\\ce".into();
        w.meta.config.map_texture_closed = "Isle\\data\\map_co.paa".into();
        w.meta.config.outside_satellite = "Isle\\data\\out_co.paa".into();
        w.meta.config.outside_synth = Some(false);
        w.meta.config.clutter_dist = Some(150.0);
        w.meta.config.min_trees_in_forest_square = Some(8);
        w.meta.config.ocean_material = "Isle\\data\\sea.emat".into();
        w.meta.config.max_tide = Some(2.0);
        w.meta.config.stars_intensity = Some(0.3);
        w.meta.config.sound_map = Some(crate::world::SoundMapConfig { forest: 1.2, ..Default::default() });
        w.meta.config.navmesh_max_slope = Some(55.0);
        let cfg = world_config_text(&w, false);
        assert!(cfg.contains("MaxTide=2;") && cfg.contains("starsIntensity=0.3;"), "{cfg}");
        assert!(cfg.contains("class SoundMapValues: SoundMapValues") && cfg.contains("forest=1.2;"), "{cfg}");
        assert!(cfg.contains("maxSlope=55;"), "{cfg}");
        let cfg = world_config_text(&w, false);
        assert!(cfg.contains("class cfgCharacterScenes\n{\n\tclass Isle\n\t{\n\t\tclass harbour") && cfg.contains("position[]={100.0,200.0,1.5};") && cfg.contains("date[]={1984,10,28,9,0};"), "{cfg}");
        assert!(cfg.contains("cutscenes[]={\"IsleIntro\"};") && cfg.contains("startTime=\"6:30\";") && cfg.contains("ceFiles=\"Isle\\ce\";"), "{cfg}");
        assert!(cfg.contains("mapTextureClosed=\"Isle\\data\\map_co.paa\";") && cfg.contains("clutterDist=150;") && cfg.contains("minTreesInForestSquare=8;"), "{cfg}");
        assert!(cfg.contains("oceanStormMaterial=\"Isle\\data\\sea.emat\";"), "{cfg}");
        assert!(cfg.contains("class OutsideTerrain: OutsideTerrain\n\t\t{\n\t\t\tsatellite=\"Isle\\data\\out_co.paa\";\n\t\t\tenableTerrainSynth=0;\n\t\t};"), "{cfg}");
        assert_eq!(cfg.matches('{').count(), cfg.matches('}').count());
        // Braces stay in pairs.
        assert_eq!(cfg.matches('{').count(), cfg.matches('}').count());
    }

    #[test]
    fn the_mission_goes_out_as_the_maps_central_economy() {
        let d = tempfile::tempdir().unwrap();
        let mut w = World::new(WorldMeta::new("Ab", 64, 8.0), 10.0);
        // Without a mission the base world's economy stays.
        assert!(!world_config_text(&w, false).contains("ceFiles"));
        assert!(!write_ce(&w, d.path()).unwrap());
        w.mission = Some(Box::new(crate::mission::templates::new_mission(&w.meta)));
        assert!(world_config_text(&w, false).contains("ceFiles=\"Ab\\ce\";"));
        assert!(write_ce(&w, d.path()).unwrap());
        let ce = d.path().join("ce");
        assert!(ce.join("db/types.xml").is_file() && ce.join("cfgeconomycore.xml").is_file() && ce.join("areaflags.map").is_file());
        assert!(!ce.join("init.c").exists());
        assert!(std::fs::read_to_string(ce.join("config.cpp")).unwrap().contains("class Ab_CE"));
        // A folder named elsewhere wins.
        w.meta.config.ce_files = "Other\\ce".into();
        assert!(world_config_text(&w, false).contains("ceFiles=\"Other\\ce\";"));
        // Without the mission again, the folder goes; one the user made stays.
        w.mission = None;
        write_ce(&w, d.path()).unwrap();
        assert!(!ce.exists());
        std::fs::create_dir_all(&ce).unwrap();
        std::fs::write(ce.join("config.cpp"), "class CfgPatches {};").unwrap();
        w.mission = Some(Box::new(crate::mission::templates::new_mission(&w.meta)));
        assert!(write_ce(&w, d.path()).is_err());
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
