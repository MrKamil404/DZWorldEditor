//! Generators: terrain shapes, erosion, surface painting and object placement.
use crate::assets::AssetIndex;
use crate::noise::Perlin;
use crate::objects::Obj;
use crate::terrain::Heightmap;
use crate::world::{MapLine, ModMode, ModShape, TerrainModifier, World};
use crate::Job;
use anyhow::{bail, Context, Result};
use rand::prelude::*;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------- terrain shapes

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerrainKind {
    Hills,
    Mountains,
    Plains,
    Island,
    Archipelago,
    Valley,
    Canyons,
    Coast,
}
impl TerrainKind {
    pub const ALL: [TerrainKind; 8] = [Self::Hills, Self::Mountains, Self::Plains, Self::Island, Self::Archipelago, Self::Valley, Self::Canyons, Self::Coast];
    pub fn name(self) -> &'static str {
        match self {
            Self::Hills => "Hills",
            Self::Mountains => "Mountains",
            Self::Plains => "Plains",
            Self::Island => "Island",
            Self::Archipelago => "Archipelago",
            Self::Valley => "River valley",
            Self::Canyons => "Canyons",
            Self::Coast => "Coast",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerrainParams {
    pub kind: TerrainKind,
    pub seed: u64,
    /// Height of the mean surface in metres.
    pub base: f32,
    /// Peak-to-valley range in metres.
    pub amplitude: f32,
    /// Size of the largest features in metres.
    pub feature: f32,
    pub octaves: u32,
    pub roughness: f32,
    /// Domain warp in metres, bends the features.
    pub warp: f32,
    /// Blend with what is already there: 1 replaces it.
    pub blend: f32,
}
impl Default for TerrainParams {
    fn default() -> Self {
        Self { kind: TerrainKind::Hills, seed: 1, base: 120.0, amplitude: 160.0, feature: 2400.0, octaves: 8, roughness: 0.5, warp: 200.0, blend: 1.0 }
    }
}

pub fn generate_terrain(h: &mut Heightmap, p: &TerrainParams, job: &Job) {
    let noise = Perlin::new(p.seed);
    let warp_x = Perlin::new(p.seed ^ 0xA5A5);
    let warp_z = Perlin::new(p.seed ^ 0x5A5A);
    let (cols, rows, cell) = (h.cols, h.rows, h.cell);
    let side = cols.max(rows) as f32 * cell;
    let f = 1.0 / p.feature.max(10.0);
    let done = std::sync::atomic::AtomicUsize::new(0);
    h.data.par_chunks_mut(cols).enumerate().for_each(|(j, row)| {
        for (i, v) in row.iter_mut().enumerate() {
            let (mut x, mut z) = (i as f32 * cell, j as f32 * cell);
            let (u, w) = (x / side * 2.0 - 1.0, z / side * 2.0 - 1.0);
            if p.warp > 0.0 {
                let (ox, oz) = (x, z);
                x += warp_x.fbm(ox * f * 1.7, oz * f * 1.7, 3, 0.5, 2.0) * p.warp;
                z += warp_z.fbm(ox * f * 1.7, oz * f * 1.7, 3, 0.5, 2.0) * p.warp;
            }
            let (nx, nz) = (x * f, z * f);
            let fbm = noise.fbm(nx, nz, p.octaves, p.roughness, 2.0);
            let radial = (u * u + w * w).sqrt();
            let height = match p.kind {
                TerrainKind::Hills => p.base + fbm * p.amplitude * 0.75,
                TerrainKind::Plains => p.base + noise.fbm(nx * 0.6, nz * 0.6, p.octaves, p.roughness * 0.7, 2.0) * p.amplitude * 0.25,
                TerrainKind::Mountains => {
                    let r = noise.ridged(nx, nz, p.octaves, p.roughness + 0.05, 2.1);
                    let mask = (noise.fbm(nx * 0.35 + 9.0, nz * 0.35, 3, 0.5, 2.0) + 0.55).clamp(0.0, 1.0);
                    p.base + (r.powf(1.4) * mask * 1.4 + fbm * 0.12) * p.amplitude
                }
                TerrainKind::Island => {
                    let edge = 1.0 - smooth(0.45, 0.95, radial + fbm * 0.18);
                    -18.0 + edge * (p.base + 18.0 + (fbm + 0.25).max(0.0) * p.amplitude * edge)
                }
                TerrainKind::Archipelago => {
                    let n = noise.fbm(nx * 2.2, nz * 2.2, p.octaves, p.roughness, 2.0);
                    let edge = 1.0 - smooth(0.7, 1.05, radial);
                    let land = smooth(0.02, 0.2, n);
                    -22.0 + land * edge * (p.base * 0.4 + 22.0 + n.max(0.0) * p.amplitude)
                }
                TerrainKind::Valley => {
                    let line = noise.fbm(z * f * 0.5, 3.0, 4, 0.5, 2.0) * 0.45;
                    let d = (u - line).abs();
                    let walls = smooth(0.05, 0.75, d);
                    p.base * 0.25 + walls * (p.base * 0.75 + (fbm * 0.5 + 0.5) * p.amplitude) + fbm * 4.0
                }
                TerrainKind::Canyons => {
                    let r = 1.0 - noise.ridged(nx, nz, 4, 0.45, 2.0);
                    let plateau = smooth(0.25, 0.42, r);
                    let step = ((plateau * 3.0).floor() + smooth(0.75, 1.0, (plateau * 3.0).fract())) / 3.0;
                    p.base * 0.4 + step * p.amplitude + fbm * 8.0
                }
                TerrainKind::Coast => {
                    let shore = u + fbm * 0.35;
                    let land = smooth(-0.15, 0.25, shore);
                    -20.0 + land * (p.base + 20.0 + (fbm + 0.3).max(0.0) * p.amplitude * smooth(0.0, 0.8, shore))
                }
            };
            *v += (height - *v) * p.blend;
        }
        let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        job.set_progress(n as f32 / rows as f32);
    });
}

#[inline]
fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErosionParams {
    pub droplets: u32,
    pub seed: u64,
    pub erode: f32,
    pub deposit: f32,
    pub capacity: f32,
    pub evaporate: f32,
    pub inertia: f32,
    pub radius: i32,
}
impl Default for ErosionParams {
    fn default() -> Self {
        Self { droplets: 300_000, seed: 1, erode: 0.3, deposit: 0.3, capacity: 4.0, evaporate: 0.02, inertia: 0.05, radius: 2 }
    }
}

/// Particle based hydraulic erosion.
pub fn hydraulic_erosion(h: &mut Heightmap, p: &ErosionParams, job: &Job) {
    let mut rng = StdRng::seed_from_u64(p.seed);
    let (cols, rows) = (h.cols as i32, h.rows as i32);
    // Weights of the erosion brush, by offset.
    let mut brush = Vec::new();
    let mut total = 0.0;
    for dy in -p.radius..=p.radius {
        for dx in -p.radius..=p.radius {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            if d <= p.radius as f32 {
                let w = 1.0 - d / (p.radius as f32 + 1.0);
                brush.push((dx, dy, w));
                total += w;
            }
        }
    }
    for b in &mut brush {
        b.2 /= total;
    }
    let scale = h.cell / 8.0;
    for n in 0..p.droplets {
        if n % 4096 == 0 {
            job.set_progress(n as f32 / p.droplets as f32);
            if job.cancelled() {
                return;
            }
        }
        let (mut x, mut y) = (rng.gen_range(1.0..cols as f32 - 2.0), rng.gen_range(1.0..rows as f32 - 2.0));
        let (mut dx, mut dy, mut speed, mut water, mut sediment) = (0.0f32, 0.0f32, 1.0f32, 1.0f32, 0.0f32);
        for _ in 0..64 {
            let (ix, iy) = (x as i32, y as i32);
            if ix < 1 || iy < 1 || ix >= cols - 2 || iy >= rows - 2 {
                break;
            }
            let (u, v) = (x - ix as f32, y - iy as f32);
            let g = |a: i32, b: i32| h.at(a as usize, b as usize);
            let (h00, h10, h01, h11) = (g(ix, iy), g(ix + 1, iy), g(ix, iy + 1), g(ix + 1, iy + 1));
            let gx = (h10 - h00) * (1.0 - v) + (h11 - h01) * v;
            let gy = (h01 - h00) * (1.0 - u) + (h11 - h10) * u;
            let height = h00 * (1.0 - u) * (1.0 - v) + h10 * u * (1.0 - v) + h01 * (1.0 - u) * v + h11 * u * v;
            dx = dx * p.inertia - gx * (1.0 - p.inertia);
            dy = dy * p.inertia - gy * (1.0 - p.inertia);
            let l = (dx * dx + dy * dy).sqrt();
            if l < 1e-5 {
                break;
            }
            dx /= l;
            dy /= l;
            let (nx, ny) = (x + dx, y + dy);
            if nx < 1.0 || ny < 1.0 || nx >= cols as f32 - 2.0 || ny >= rows as f32 - 2.0 {
                break;
            }
            let (jx, jy) = (nx as i32, ny as i32);
            let (a, b) = (nx - jx as f32, ny - jy as f32);
            let new_height = g(jx, jy) * (1.0 - a) * (1.0 - b) + g(jx + 1, jy) * a * (1.0 - b) + g(jx, jy + 1) * (1.0 - a) * b + g(jx + 1, jy + 1) * a * b;
            let delta = new_height - height;
            let cap = (-delta).max(0.01) * speed * water * p.capacity;
            if sediment > cap || delta > 0.0 {
                let amount = if delta > 0.0 { delta.min(sediment) } else { (sediment - cap) * p.deposit };
                sediment -= amount;
                for (px, py, w) in [(ix, iy, (1.0 - u) * (1.0 - v)), (ix + 1, iy, u * (1.0 - v)), (ix, iy + 1, (1.0 - u) * v), (ix + 1, iy + 1, u * v)] {
                    let i = (py * cols + px) as usize;
                    h.data[i] += amount * w;
                }
            } else {
                let amount = ((cap - sediment) * p.erode).min(-delta).max(0.0);
                for (ox, oy, w) in &brush {
                    let (px, py) = (ix + ox, iy + oy);
                    if px >= 0 && py >= 0 && px < cols && py < rows {
                        let i = (py * cols + px) as usize;
                        h.data[i] -= amount * w;
                    }
                }
                sediment += amount;
            }
            speed = (speed * speed + delta.abs() * 4.0 / scale.max(0.1)).sqrt().min(16.0);
            if delta > 0.0 {
                speed = (speed * speed - delta * 4.0).max(0.0).sqrt();
            }
            water *= 1.0 - p.evaporate;
            x = nx;
            y = ny;
        }
    }
}

/// Moves material from slopes steeper than `talus` degrees down to their neighbours.
pub fn thermal_erosion(h: &mut Heightmap, iterations: u32, talus: f32, job: &Job) {
    let limit = talus.to_radians().tan() * h.cell;
    let (cols, rows) = (h.cols, h.rows);
    for it in 0..iterations {
        job.set_progress(it as f32 / iterations as f32);
        let src = h.data.clone();
        h.data.par_chunks_mut(cols).enumerate().skip(1).take(rows - 2).for_each(|(j, row)| {
            for i in 1..cols - 1 {
                let c = src[j * cols + i];
                let mut change = 0.0;
                for (a, b) in [(i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)] {
                    let n = src[b * cols + a];
                    let d = c - n;
                    if d > limit {
                        change -= (d - limit) * 0.2;
                    } else if -d > limit {
                        change += (-d - limit) * 0.2;
                    }
                }
                row[i] += change;
            }
        });
    }
}

/// Quantises heights into steps with soft edges.
pub fn terrace(h: &mut Heightmap, step: f32, sharpness: f32) {
    let step = step.max(0.5);
    h.data.par_iter_mut().for_each(|v| {
        let k = *v / step;
        let f = k.fract();
        let t = smooth(0.5 - 0.5 * (1.0 - sharpness), 0.5 + 0.5 * (1.0 - sharpness), f);
        *v = (k.floor() + t) * step;
    });
}

/// Rivers traced downhill from high ground and carved into the terrain.
pub fn carve_rivers(h: &mut Heightmap, count: u32, width: f32, depth: f32, seed: u64) -> Vec<MapLine> {
    let mut rng = StdRng::seed_from_u64(seed);
    let cell = h.cell;
    let (w, d) = (h.width_m(), h.height_m());
    let (lo, hi) = h.min_max();
    let mut lines = Vec::new();
    let threshold = lo + (hi - lo) * 0.45;
    let mut guard = 0;
    while (lines.len() as u32) < count && guard < count * 400 {
        guard += 1;
        let (mut x, mut z) = (rng.gen_range(cell..w - cell), rng.gen_range(cell..d - cell));
        if h.sample(x, z) < threshold {
            continue;
        }
        let mut points = vec![[x, z]];
        let step = (cell * 2.0).max(8.0);
        let mut last = h.sample(x, z);
        let mut angle = rng.gen_range(0.0..std::f32::consts::TAU);
        for _ in 0..6000 {
            // Pick the lowest of several directions around the previous heading.
            let mut best = (f32::MAX, angle);
            for k in -6..=6 {
                let a = angle + k as f32 * 0.22;
                let (nx, nz) = (x + a.cos() * step * 3.0, z + a.sin() * step * 3.0);
                let c = h.sample(nx, nz) + (k as f32).abs() * 0.05;
                if c < best.0 {
                    best = (c, a);
                }
            }
            angle = best.1 + rng.gen_range(-0.12..0.12);
            x += angle.cos() * step;
            z += angle.sin() * step;
            if x < 0.0 || z < 0.0 || x >= w || z >= d {
                break;
            }
            points.push([x, z]);
            let height = h.sample(x, z);
            last = last.min(height);
            if last <= 0.0 {
                break;
            }
        }
        if points.len() < 24 {
            continue;
        }
        carve_line(h, &points, width, depth);
        lines.push(MapLine { name: format!("River {}", lines.len() + 1), family: "river".into(), points, width });
    }
    lines
}

/// Cuts a smooth channel following the line, with a monotonically falling bed.
fn carve_line(h: &mut Heightmap, points: &[[f32; 2]], width: f32, depth: f32) {
    let mut bed = f32::MAX;
    let cell = h.cell;
    let reach = width * 2.5;
    let n = points.len().max(2) as f32;
    for (k, p) in points.iter().enumerate() {
        let local = h.sample(p[0], p[1]);
        bed = bed.min(local - depth * (0.4 + 0.6 * (k as f32 / n)));
        let Some(r) = h.rect_around(p[0], p[1], reach) else { continue };
        for j in r.y0..=r.y1 {
            for i in r.x0..=r.x1 {
                let dist = ((i as f32 * cell - p[0]).powi(2) + (j as f32 * cell - p[1]).powi(2)).sqrt();
                let t = 1.0 - smooth(width * 0.5, reach, dist);
                if t > 0.0 {
                    let v = h.at(i, j);
                    if v > bed {
                        h.set(i, j, v + (bed - v) * t);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- surface painting

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PaintRule {
    pub layer: usize,
    pub min_height: f32,
    pub max_height: f32,
    pub min_slope: f32,
    pub max_slope: f32,
    /// Blotch size in metres; 0 disables.
    pub noise_scale: f32,
    /// Share of the area the rule covers, 0..1.
    pub coverage: f32,
}

/// Default rules using layer names, later rules win.
pub fn default_paint_rules(w: &World) -> Vec<PaintRule> {
    let rule = |name: &str, h0: f32, h1: f32, s0: f32, s1: f32, n: f32, c: f32| {
        w.layer_index(name).map(|layer| PaintRule { layer, min_height: h0, max_height: h1, min_slope: s0, max_slope: s1, noise_scale: n, coverage: c })
    };
    [
        rule("grass1", -1000.0, 9000.0, 0.0, 90.0, 0.0, 1.0),
        rule("grass2", -1000.0, 9000.0, 0.0, 30.0, 180.0, 0.45),
        rule("stubble", 1.0, 180.0, 0.0, 8.0, 500.0, 0.25),
        rule("soil", -1000.0, 9000.0, 26.0, 90.0, 0.0, 1.0),
        rule("stones", -1000.0, 9000.0, 38.0, 90.0, 0.0, 1.0),
        rule("stones", -1000.0, 1.2, 0.0, 90.0, 0.0, 1.0),
    ].into_iter().flatten().collect()
}

pub fn auto_paint(w: &World, rules: &[PaintRule], seed: u64, area: Option<&crate::shapes::PolygonIndex>, job: &Job) -> Vec<u8> {
    let noise = Perlin::new(seed);
    let (mw, mh) = (w.mask.width, w.mask.height);
    let size = w.meta.size_m();
    let mut out = w.mask.data.clone();
    let done = std::sync::atomic::AtomicUsize::new(0);
    out.par_chunks_mut(mw).enumerate().for_each(|(y, row)| {
        let wz = (1.0 - (y as f32 + 0.5) / mh as f32) * size;
        for (x, v) in row.iter_mut().enumerate() {
            let wx = (x as f32 + 0.5) / mw as f32 * size;
            if area.is_some_and(|a| !a.contains(wx, wz)) {
                continue;
            }
            let (h, s) = (w.height.sample(wx, wz), w.height.slope(wx, wz));
            for r in rules {
                if h < r.min_height || h > r.max_height || s < r.min_slope || s > r.max_slope {
                    continue;
                }
                if r.noise_scale > 0.0 {
                    let n = noise.fbm(wx / r.noise_scale + r.layer as f32 * 3.1, wz / r.noise_scale, 4, 0.55, 2.0) * 0.5 + 0.5;
                    if n > r.coverage + 0.25 {
                        continue;
                    }
                }
                *v = r.layer as u8;
            }
        }
        let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        job.set_progress(n as f32 / mh as f32);
    });
    out
}

// ---------------------------------------------------------------- object generators

/// Pitch and roll laying a model flat on the terrain, for a given yaw.
pub fn slope_angles(h: &Heightmap, x: f32, z: f32, yaw: f32) -> (f32, f32) {
    let n = h.normal(x, z);
    let (s, c) = yaw.to_radians().sin_cos();
    let forward = n[0] * s + n[2] * c;
    let right = n[0] * c - n[2] * s;
    ((forward).clamp(-1.0, 1.0).asin().to_degrees(), (-right).clamp(-1.0, 1.0).asin().to_degrees())
}

/// Forest generation through the DayZForests engine, with a whole DayZForests project.
pub struct ForestRequest {
    pub project: forest_core::preset::ForestProject,
    /// (mask layer, forest preset name)
    pub zones: Vec<(usize, String)>,
    /// (polygons, forest preset name)
    pub areas: Vec<(crate::shapes::ShapeLayer, String)>,
    /// Needed by areas that follow the colours of the satellite image.
    pub satellite: Option<forest_core::mask::MaskImage>,
    /// Applied to the areas made from shapes.
    pub color_filter: Option<forest_core::preset::ColorFilter>,
    /// (preset, share) planted along the edges.
    pub edge_presets: Vec<(String, f32)>,
    pub user_presets: Vec<forest_core::user_presets::UserPreset>,
}

pub fn forest_preset_names() -> Vec<String> {
    forest_core::species::zone_presets().iter().map(|z| z.name.to_string()).collect()
}

/// The satellite image in the form the forest engine samples.
pub fn satellite_for_forest(img: &crate::terrain::RgbImage) -> forest_core::mask::MaskImage {
    let side = img.width.max(img.height).min(8192);
    let (w, h) = (side, side * img.height / img.width.max(1));
    let mut rgba = vec![255u8; w * h * 4];
    rgba.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let sy = y * img.height / h;
        for x in 0..w {
            let p = (sy * img.width + x * img.width / w) * 3;
            row[x * 4..x * 4 + 3].copy_from_slice(&img.data[p..p + 3]);
        }
    });
    forest_core::mask::MaskImage { width: w as u32, height: h as u32, rgba }
}

/// Finds a preset by name, the user's own first.
fn find_preset(name: &str, user: &[forest_core::user_presets::UserPreset]) -> Option<forest_core::user_presets::UserPreset> {
    use forest_core::user_presets::UserPreset;
    user.iter().find(|p| p.name == name).cloned().or_else(|| forest_core::species::zone_presets().iter().find(|p| p.name == name).map(UserPreset::from_builtin))
}

/// Weights of a preset for the project's own species list, adding what it lacks.
fn preset_weights(project: &mut forest_core::preset::ForestProject, preset: &forest_core::user_presets::UserPreset, vanilla: &[forest_core::species::SpeciesDef]) -> Vec<(usize, f32)> {
    let mut out = Vec::with_capacity(preset.weights.len());
    for (model, weight) in &preset.weights {
        let at = match project.species.iter().position(|x| x.model.eq_ignore_ascii_case(model)) {
            Some(at) => at,
            None => {
                let Some(s) = vanilla.iter().find(|x| x.model.eq_ignore_ascii_case(model)) else { continue };
                project.species.push(s.clone());
                project.species.len() - 1
            }
        };
        out.push((at, *weight));
    }
    out
}

/// Whether the project reads the satellite image.
pub fn forest_needs_satellite(p: &forest_core::preset::ForestProject) -> bool {
    p.areas.iter().any(|a| a.color_filter.as_ref().is_some_and(|f| !f.samples.is_empty()) || a.preset_mix.iter().any(|m| m.color_filter.as_ref().is_some_and(|f| !f.samples.is_empty())))
}

pub fn generate_forest(w: &mut World, assets: &AssetIndex, req: ForestRequest, job: &Job) -> Result<(Vec<Obj>, Vec<String>)> {
    use forest_core::mask::{MaskImage, Rgb8};
    use forest_core::preset::{AreaDef, GenStep, MixEntry, ZoneDef};
    let size = w.meta.size_m() as f64;
    let vanilla = forest_core::species::vanilla_library();
    let mut project = req.project;
    project.map_size_m = size;
    project.easting_offset = 0.0;
    project.northing_offset = 0.0;
    for (layer, name) in &req.zones {
        let preset = find_preset(name, &req.user_presets).with_context(|| format!("Unknown forest preset {name}"))?;
        let color = w.layers.get(*layer).context("Unknown surface layer")?.color;
        let species_weights = preset_weights(&mut project, &preset, &vanilla);
        project.zones.push(ZoneDef { color: Rgb8(color), label: preset.name.clone(), density_per_ha: preset.density_per_ha, species_weights, preset_mix: Vec::new() });
    }
    for (layer, name) in &req.areas {
        let preset = find_preset(name, &req.user_presets).with_context(|| format!("Unknown forest preset {name}"))?;
        let species_weights = preset_weights(&mut project, &preset, &vanilla);
        for (i, s) in layer.shapes.iter().enumerate().filter(|(_, s)| s.kind == crate::shapes::ShapeKind::Polygon) {
            project.areas.push(AreaDef { label: format!("{} {}", layer.name, i + 1), density_per_ha: preset.density_per_ha, species_weights: species_weights.clone(), polygon: s.points.iter().map(|p| [p[0] as f64, p[1] as f64]).collect(), color_filter: req.color_filter.clone(), ..Default::default() });
            project.generation_order.push(GenStep::Area(project.areas.len() - 1));
        }
    }
    if !req.edge_presets.is_empty() {
        project.edges.preset_mix.clear();
        for (name, share) in &req.edge_presets {
            let preset = find_preset(name, &req.user_presets).with_context(|| format!("Unknown forest preset {name}"))?;
            let species_weights = preset_weights(&mut project, &preset, &vanilla);
            project.edges.preset_mix.push(MixEntry { name: name.clone(), share: *share, spatial: false, color_filter: None, density_per_ha: preset.density_per_ha, species_weights });
        }
    }
    project.sanitize();
    if project.zones.is_empty() && project.areas.is_empty() {
        bail!("Add at least one zone or area");
    }
    // The engine reads RGB masks in the colours of the map's surface layers.
    let mask = (!project.zones.is_empty() || !project.cut_zones.is_empty() || !project.exclusion_colors.is_empty()).then(|| {
        let side = w.mask.width.min(8192) as u32;
        let mut mask = MaskImage { width: side, height: side, rgba: vec![255; (side * side * 4) as usize] };
        let (mw, mh) = (w.mask.width, w.mask.height);
        let src = &w.mask.data;
        let colors: Vec<[u8; 3]> = w.layers.iter().map(|l| l.color).collect();
        mask.rgba.par_chunks_mut(side as usize * 4).enumerate().for_each(|(y, row)| {
            let sy = y * mh / side as usize;
            for x in 0..side as usize {
                let c = colors.get(src[sy * mw + x * mw / side as usize] as usize).copied().unwrap_or([0, 0, 0]);
                row[x * 4..x * 4 + 3].copy_from_slice(&c);
            }
        });
        mask
    });
    let values: Vec<f32> = (0..w.height.rows).rev().flat_map(|j| (0..w.height.cols).map(move |i| (i, j))).map(|(i, j)| w.height.at(i, j)).collect();
    let hm = forest_core::heightmap::AscHeightmap {
        ncols: w.height.cols, nrows: w.height.rows, xllcorner: 0.0, yllcorner: 0.0,
        cellsize: w.height.cell as f64, nodata: -9999.0, values, x_origin_offset: 0.0, y_origin_offset: 0.0,
    };
    let j2 = job.clone();
    let progress = move |v: f64| j2.set_progress(v as f32);
    let (placed, stats) = forest_core::scatter::generate(&project, mask.as_ref(), req.satellite.as_ref(), Some(&hm), None, &progress).map_err(anyhow::Error::msg)?;
    let mut missing: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(placed.len());
    for p in placed {
        let Some(path) = assets.resolve(&p.model) else {
            *missing.entry(p.model.clone()).or_default() += 1;
            continue;
        };
        let model = w.objects.intern(path);
        let (yaw, pitch, roll) = crate::objects::from_terrain_builder(p.yaw as f32, p.pitch as f32, p.roll as f32);
        out.push(Obj { x: p.x as f32, z: p.y as f32, elevation: p.elevation as f32, yaw, pitch, roll, scale: p.scale as f32, model, flags: 0 });
    }
    let mut notes = vec![stats.summary()];
    if !missing.is_empty() {
        let mut names: Vec<_> = missing.into_keys().collect();
        names.sort();
        notes.push(format!("Models not found on P:\\ and skipped: {}", names.join(", ")));
    }
    Ok((out, notes))
}

/// Objects along a polyline: fences, walls, poles.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LineRule {
    pub model: String,
    /// Distance between objects along the line.
    pub spacing: f32,
    pub offset: f32,
    pub jitter: f32,
    /// Added to the heading.
    pub yaw_offset: f32,
    pub follow_slope: bool,
}

pub fn along_line(w: &mut World, points: &[[f32; 2]], rule: &LineRule, seed: u64) -> Vec<Obj> {
    let model = w.objects.intern(&rule.model);
    let mut rng = StdRng::seed_from_u64(seed);
    let mut out = Vec::new();
    let mut carried = 0.0f32;
    for seg in points.windows(2) {
        let (dx, dz) = (seg[1][0] - seg[0][0], seg[1][1] - seg[0][1]);
        let len = (dx * dx + dz * dz).sqrt();
        if len < 1e-3 {
            continue;
        }
        let (ux, uz) = (dx / len, dz / len);
        let heading = ux.atan2(uz).to_degrees();
        let mut t = carried;
        while t <= len {
            let j = if rule.jitter > 0.0 { rng.gen_range(-rule.jitter..rule.jitter) } else { 0.0 };
            let side = rule.offset + j;
            let (x, z) = (seg[0][0] + ux * t + uz * side, seg[0][1] + uz * t - ux * side);
            let yaw = heading + rule.yaw_offset;
            let (pitch, roll) = if rule.follow_slope { slope_angles(&w.height, x, z, yaw) } else { (0.0, 0.0) };
            out.push(Obj { x, z, elevation: 0.0, yaw, pitch, roll, scale: 1.0, model, flags: 0 });
            t += rule.spacing.max(0.25);
        }
        carried = t - len;
    }
    out
}

/// A village: plots beside a road network growing from a centre.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SettlementRule {
    pub buildings: Vec<String>,
    pub radius: f32,
    pub streets: u32,
    pub plot: f32,
    pub setback: f32,
    pub seed: u64,
    pub max_slope: f32,
    pub flatten: bool,
}
impl Default for SettlementRule {
    fn default() -> Self {
        Self { buildings: Vec::new(), radius: 200.0, streets: 4, plot: 26.0, setback: 12.0, seed: 3, max_slope: 18.0, flatten: true }
    }
}

pub struct Settlement {
    pub objects: Vec<Obj>,
    pub streets: Vec<MapLine>,
    /// (index in `objects`, footprint half x, half z)
    pub pads: Vec<(usize, f32, f32)>,
}

pub fn settlement(w: &mut World, db: &crate::model_info::ModelDb, cx: f32, cz: f32, rule: &SettlementRule) -> Result<Settlement> {
    if rule.buildings.is_empty() {
        bail!("Choose at least one building");
    }
    let models: Vec<(u32, crate::model_info::ModelInfo)> = rule.buildings.iter().map(|b| (w.objects.intern(b), db.get(&ObjectStorePath::norm(b)))).collect();
    let mut rng = StdRng::seed_from_u64(rule.seed);
    let (mut objects, mut streets, mut pads) = (Vec::new(), Vec::new(), Vec::new());
    let mut taken: Vec<[f32; 3]> = Vec::new();
    for s in 0..rule.streets.max(1) {
        let base = s as f32 / rule.streets.max(1) as f32 * std::f32::consts::TAU + rng.gen_range(-0.3..0.3);
        let mut points = vec![[cx, cz]];
        let (mut x, mut z, mut a) = (cx, cz, base);
        let step = rule.plot;
        let count = (rule.radius / step) as i32;
        for k in 0..count {
            a += rng.gen_range(-0.14..0.14);
            let (dx, dz) = (a.sin(), a.cos());
            x += dx * step;
            z += dz * step;
            points.push([x, z]);
            if k == 0 {
                continue;
            }
            for side in [-1.0f32, 1.0] {
                if rng.gen_bool(0.15) {
                    continue;
                }
                let (m, info) = models.choose(&mut rng).copied().unwrap();
                let (hx, hz) = if info.known { info.footprint() } else { (6.0, 6.0) };
                let d = rule.setback + hz;
                let (px, pz) = (x + dz * side * d, z - dx * side * d);
                let r = hx.max(hz);
                if w.height.slope(px, pz) > rule.max_slope || w.height.sample(px, pz) < 1.0 {
                    continue;
                }
                if taken.iter().any(|t| (t[0] - px).powi(2) + (t[1] - pz).powi(2) < (t[2] + r + 2.0).powi(2)) {
                    continue;
                }
                taken.push([px, pz, r]);
                // Face the street.
                let yaw = (-dz * side).atan2(dx * side).to_degrees() + 90.0;
                let yaw = if side > 0.0 { a.to_degrees() - 90.0 } else { a.to_degrees() + 90.0 } + (yaw - yaw);
                if rule.flatten {
                    pads.push((objects.len(), hx, hz));
                }
                objects.push(Obj { x: px, z: pz, elevation: 0.0, yaw, pitch: 0.0, roll: 0.0, scale: 1.0, model: m, flags: 0 });
            }
        }
        streets.push(MapLine { name: format!("Street {}", s + 1), family: "street".into(), points, width: 6.0 });
    }
    Ok(Settlement { objects, streets, pads })
}

struct ObjectStorePath;
impl ObjectStorePath {
    fn norm(p: &str) -> String {
        crate::objects::ObjectStore::normalize_path(p)
    }
}

/// Modifier that levels the ground under an object.
pub fn pad_modifier(w: &mut World, object: u32, half_x: f32, half_z: f32, margin: f32, falloff: f32) -> Option<TerrainModifier> {
    let o = *w.objects.get(object)?;
    let id = w.new_modifier_id();
    let model = w.objects.models.get(o.model as usize).map(|m| m.path.rsplit('\\').next().unwrap_or("").to_string()).unwrap_or_default();
    Some(TerrainModifier {
        id, name: format!("Pad: {model}"), enabled: true, x: o.x, z: o.z, yaw: o.yaw,
        shape: ModShape::Rect { half_x: half_x + margin, half_z: half_z + margin },
        mode: ModMode::Flatten, target: None, amount: 0.0, falloff, attached: Some(object),
    })
}

// ---------------------------------------------------------------- roads

/// Parts of the road family folder, grouped by the Road Tool's naming scheme.
pub fn road_catalog(folder: &Path) -> Result<Vec<dayz_road_tool::roads::CatalogPart>> {
    use dayz_road_tool::{geometry::Library, roads::CatalogPart};
    let mut lib = Library::new(folder.to_path_buf());
    let mut out = Vec::new();
    for e in std::fs::read_dir(folder).with_context(|| format!("{}", folder.display()))? {
        let name = e?.file_name().to_string_lossy().to_string();
        if !name.to_ascii_lowercase().ends_with(".p3d") {
            continue;
        }
        let stem = name[..name.len() - 4].to_string();
        let lower = stem.to_ascii_lowercase();
        let Some((family, rest)) = lower.split_once('_') else { continue };
        if rest.contains('_') {
            continue;
        }
        let category = if rest.contains("konec") { 6 } else if rest.contains(' ') { 4 } else { 3 };
        let Ok(model) = lib.model(&name) else { continue };
        out.push(CatalogPart { path: name.clone(), family: family.to_string(), category, index: out.len() as u32, road_type_index: 0, model });
    }
    Ok(out)
}

pub fn road_families(catalog: &[dayz_road_tool::roads::CatalogPart]) -> Vec<String> {
    let mut f: Vec<String> = catalog.iter().filter(|c| c.category == 6 || dayz_road_tool::rails::is_track_family(&c.family)).map(|c| c.family.clone()).collect();
    f.sort();
    f.dedup();
    f
}

/// A road through the points built from real road pieces, with the terrain graded under it.
pub struct RoadResult {
    pub objects: Vec<Obj>,
    pub centerline: Vec<[f32; 2]>,
}

pub fn build_road(w: &mut World, points: &[[f32; 2]], family: &str, catalog: &[dayz_road_tool::roads::CatalogPart], models_prefix: &str, min_radius: f64) -> Result<RoadResult> {
    use dayz_road_tool::{document::RoutingSettings, roads};
    let settings = RoutingSettings { min_radius, ..Default::default() };
    let line: Vec<[f64; 2]> = points.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
    let parts = roads::fit(&line, family, catalog, &settings, |_, _| true)?;
    let center = roads::centerline(&parts, catalog);
    let mut objects = Vec::with_capacity(parts.len());
    for p in &parts {
        let model = w.objects.intern(&format!("{models_prefix}\\{}", p.model));
        objects.push(Obj { x: p.position[0] as f32, z: p.position[1] as f32, elevation: 0.0, yaw: p.rotation.rem_euclid(360.0) as f32, pitch: 0.0, roll: 0.0, scale: 1.0, model, flags: 0 });
    }
    Ok(RoadResult { objects, centerline: center.iter().map(|p| [p[0] as f32, p[1] as f32]).collect() })
}

/// Road types of a Terrain Builder project, as Terrain Builder itself defines them.
pub fn road_catalog_from_project(project: &Path, folder: &Path) -> Result<Vec<dayz_road_tool::roads::CatalogPart>> {
    let mut lib = dayz_road_tool::geometry::Library::new(folder.to_path_buf());
    let (catalog, _) = dayz_road_tool::tv4p::catalog(project, &mut lib)?;
    Ok(catalog)
}

fn road_model_path(model: &str, models_prefix: &str) -> String {
    let lower = crate::objects::ObjectStore::normalize_path(model);
    match lower.strip_prefix("p:\\") {
        Some(rest) => rest.to_string(),
        None => format!("{models_prefix}\\{}", lower.rsplit('\\').next().unwrap_or(&lower)),
    }
}

/// What to build for a road or track.
#[derive(Clone)]
pub struct RouteRequest {
    pub family: String,
    pub points: Vec<[f32; 2]>,
    pub terrain_route: bool,
    pub settings: dayz_road_tool::document::RoutingSettings,
    /// Polygons the route must keep out of.
    pub forbidden: Vec<Vec<[f32; 2]>>,
    /// Centre lines of roads, for level crossings of tracks.
    pub roads: Vec<Vec<[f32; 2]>>,
}

/// Track pieces, which are not part of a road project.
pub fn rail_catalog() -> Vec<dayz_road_tool::roads::CatalogPart> {
    let folder = Path::new(dayz_road_tool::rails::DEFAULT_FOLDER);
    let mut lib = dayz_road_tool::geometry::Library::new(folder.to_path_buf());
    dayz_road_tool::rails::catalog(folder, &mut lib).map(|(c, _)| c).unwrap_or_default()
}

/// Roads and tracks made the way Road Builder makes them: searched across the terrain within
/// the grade, around water and forbidden polygons, with bridges and level crossings. Tracks
/// and bridges are rigid and get a full pose on the terrain.
pub fn build_route(w: &mut World, req: &RouteRequest, catalog: &[dayz_road_tool::roads::CatalogPart], job: &Job) -> Result<RoadResult> {
    use crate::fsutil::f32s_to_bytes;
    use dayz_road_tool::{document::{Route, WaterSettings}, rails::{self, BridgeKit, Crossings, RoadLine}, roads, routing::{self, Planned}, terrain::{Meta, Terrain}};
    use std::sync::atomic::AtomicBool;
    let rail = rails::is_track_family(&req.family);
    let prefix = if rail { r"dz\structures\rail\Tracks" } else { r"dz\structures\roads\parts" };
    let line: Vec<[f64; 2]> = req.points.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
    let tracks = Path::new(rails::DEFAULT_FOLDER);
    let road_folder = Path::new("P:\\").join(r"dz\structures\roads\parts");
    let cancel = AtomicBool::new(false);
    let crossings = if rail {
        let near: Vec<RoadLine> = req.roads.iter().enumerate().filter(|(_, l)| l.len() >= 2).map(|(road, l)| RoadLine { points: l.iter().map(|p| [p[0] as f64, p[1] as f64]).collect(), half_width: 4.0, road }).collect();
        Crossings::new(near, &req.family, catalog, tracks)
    } else {
        Crossings::none()
    };
    let planned = if req.terrain_route {
        job.stage("Preparing the terrain");
        let dir = tempfile_dir()?;
        let result = (|| -> Result<Planned> {
            let cell = w.height.cell as f64;
            let meta = Meta { cols: w.height.cols, rows: w.height.rows, east: -cell * 0.5, north: -cell * 0.5, cell };
            std::fs::write(dir.join("terrain.json"), serde_json::to_vec(&meta)?)?;
            // Road Builder's terrain runs from the north edge; the map's from the south.
            let flipped: Vec<f32> = (0..w.height.rows).rev().flat_map(|j| (0..w.height.cols).map(move |i| (i, j))).map(|(i, j)| w.height.at(i, j)).collect();
            std::fs::write(dir.join("height.f32"), f32s_to_bytes(&flipped))?;
            let terrain = Terrain::open(&dir)?;
            job.stage("Finding the route");
            let polygons: Vec<Vec<[f64; 2]>> = req.forbidden.iter().map(|r| r.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
            let water = WaterSettings { enabled: true, level: w.meta.sea_level as f64 };
            let obstacles = routing::Obstacles::new(&polygons, &[], &water, Some(&terrain), &cancel)?;
            let bridge = if rail { BridgeKit::for_track(tracks, &req.family) } else { BridgeKit::for_road(&road_folder, &req.family, catalog) };
            routing::plan(&terrain, &line, &req.family, catalog, &req.settings, &obstacles, &crossings.with_bridge(bridge), &cancel)
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result?
    } else {
        let mut parts = roads::fit(&line, &req.family, catalog, &req.settings, |_, _| true)?;
        let extras = crossings.apply(&mut parts, catalog)?;
        Planned { parts, extras, ..Default::default() }
    };
    let route = Route { name: String::new(), family: req.family.clone(), points: line, parts: planned.parts, replaces: None, rail, extras: planned.extras, grading: None, bridges: planned.bridges };
    let center: Vec<[f64; 2]> = route.stretches().into_iter().flat_map(|s| roads::centerline(s, catalog)).collect();
    let height = &w.height;
    let ground = |p: [f64; 2]| Some(height.sample(p[0] as f32, p[1] as f32) as f64);
    let poses = rails::route_poses(&route, catalog, &ground)?;
    let spans = route.bridges.iter().flat_map(|b| &b.pieces);
    let mut objects = Vec::with_capacity(route.parts.len() + route.extras.len());
    for (i, p) in route.parts.iter().chain(&route.extras).chain(spans).enumerate() {
        let model = w.objects.intern(&road_model_path(&p.model, prefix));
        let obj = match poses.get(i).copied().flatten() {
            Some(pose) => {
                // Pose axes are (east, north, up); the editor's are (east, up, north).
                let axis = |v: [f64; 3]| [v[0] as f32, v[2] as f32, v[1] as f32];
                let (yaw, pitch, roll, _) = crate::objects::angles(&[axis(pose.right), axis(pose.up), axis(pose.forward)]);
                let ground_there = w.height.sample(pose.origin[0] as f32, pose.origin[1] as f32);
                Obj { x: pose.origin[0] as f32, z: pose.origin[1] as f32, elevation: pose.origin[2] as f32 - ground_there, yaw, pitch, roll, scale: 1.0, model, flags: 0 }
            }
            None => Obj { x: p.position[0] as f32, z: p.position[1] as f32, elevation: 0.0, yaw: p.rotation.rem_euclid(360.0) as f32, pitch: 0.0, roll: 0.0, scale: 1.0, model, flags: 0 },
        };
        objects.push(obj);
    }
    Ok(RoadResult { objects, centerline: center.iter().map(|p| [p[0] as f32, p[1] as f32]).collect() })
}

/// Keeps the points that matter to a line's shape, to `tolerance` metres.
pub fn simplify_line(points: &[[f32; 2]], tolerance: f32) -> Vec<[f32; 2]> {
    fn mark(p: &[[f32; 2]], a: usize, b: usize, t: f32, keep: &mut [bool]) {
        if b <= a + 1 {
            return;
        }
        let (dx, dz) = (p[b][0] - p[a][0], p[b][1] - p[a][1]);
        let len = (dx * dx + dz * dz).sqrt().max(1e-3);
        let (mut far, mut at) = (0.0f32, a);
        for i in a + 1..b {
            let d = ((p[i][0] - p[a][0]) * dz - (p[i][1] - p[a][1]) * dx).abs() / len;
            if d > far {
                (far, at) = (d, i);
            }
        }
        if far > t {
            keep[at] = true;
            mark(p, a, at, t, keep);
            mark(p, at, b, t, keep);
        }
    }
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    (keep[0], keep[points.len() - 1]) = (true, true);
    mark(points, 0, points.len() - 1, tolerance, &mut keep);
    points.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect()
}

fn tempfile_dir() -> Result<std::path::PathBuf> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("dzwe_route_{}_{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Walls and fences along a line, one model after another.
pub fn fence_objects(w: &mut World, db: &crate::model_info::ModelDb, points: &[[f32; 2]], g: &crate::library::FenceGen) -> Vec<Obj> {
    let info = db.get(&crate::objects::ObjectStore::normalize_path(&g.model));
    let (lx, lz) = if info.known { (info.max[0] - info.min[0], info.max[2] - info.min[2]) } else { (0.0, 4.0) };
    // Yaw points a model's Z axis along the line, so one that is long in X turns by 90 degrees.
    let (length, turn) = if lx > lz { (lx, 90.0) } else { (lz, 0.0) };
    let spacing = if g.spacing > 0.0 { g.spacing } else { length.max(0.5) };
    along_line(w, points, &LineRule { model: g.model.clone(), spacing, offset: 0.0, jitter: 0.0, yaw_offset: turn + g.yaw_offset, follow_slope: g.follow_slope }, 1)
}

/// Towers along a line, and the wires between them.
pub fn power_objects(w: &mut World, db: &crate::model_info::ModelDb, points: &[[f32; 2]], g: &crate::library::PowerGen) -> Vec<Obj> {
    let wire_info = (!g.wire.is_empty()).then(|| db.get(&crate::objects::ObjectStore::normalize_path(&g.wire)));
    let wire_length = wire_info.filter(|i| i.known).map(|i| (i.max[0] - i.min[0]).max(i.max[2] - i.min[2]));
    let spacing = if g.spacing > 0.0 { g.spacing } else { wire_length.unwrap_or(60.0) }.max(5.0);
    let tower = w.objects.intern(&g.tower);
    let corner = if g.corner.is_empty() { tower } else { w.objects.intern(&g.corner) };
    let wire = (!g.wire.is_empty()).then(|| w.objects.intern(&g.wire));
    let mut out = Vec::new();
    // Tower positions: each straight stretch is divided into whole spans.
    let mut stations: Vec<([f32; 2], bool)> = vec![(points[0], false)];
    for (k, seg) in points.windows(2).enumerate() {
        let (dx, dz) = (seg[1][0] - seg[0][0], seg[1][1] - seg[0][1]);
        let n = ((dx * dx + dz * dz).sqrt() / spacing).round().max(1.0) as usize;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            stations.push(([seg[0][0] + dx * t, seg[0][1] + dz * t], i == n && k + 2 < points.len()));
        }
    }
    let heading = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]).atan2(b[1] - a[1]).to_degrees();
    for (i, (p, bend)) in stations.iter().enumerate() {
        let before = i.checked_sub(1).map(|k| stations[k].0);
        let after = stations.get(i + 1).map(|s| s.0);
        // At a bend the arms split the angle between the two spans.
        let yaw = match (before, after) {
            (Some(a), Some(b)) => { let (h0, h1) = (heading(a, *p), heading(*p, b)); h0 + ((h1 - h0 + 540.0).rem_euclid(360.0) - 180.0) * 0.5 }
            (None, Some(b)) => heading(*p, b),
            (Some(a), None) => heading(a, *p),
            _ => 0.0,
        };
        out.push(Obj { x: p[0], z: p[1], elevation: 0.0, yaw: yaw + g.yaw_offset, pitch: 0.0, roll: 0.0, scale: 1.0, model: if *bend { corner } else { tower }, flags: 0 });
    }
    if let (Some(model), Some(info)) = (wire, wire_info) {
        let along_x = info.known && info.max[0] - info.min[0] > info.max[2] - info.min[2];
        for s in stations.windows(2) {
            let (a, b) = (s[0].0, s[1].0);
            let span = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt().max(0.1);
            let (ga, gb) = (w.height.sample(a[0], a[1]), w.height.sample(b[0], b[1]));
            let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            let elevation = (ga + gb) * 0.5 - w.height.sample(mid[0], mid[1]);
            let tilt = ((gb - ga) / span).atan().to_degrees();
            let (yaw, pitch, roll) = if along_x { (heading(a, b) + 90.0, 0.0, -tilt) } else { (heading(a, b), -tilt, 0.0) };
            let scale = wire_length.map(|l| span / l.max(1.0)).unwrap_or(1.0);
            out.push(Obj { x: mid[0], z: mid[1], elevation, yaw, pitch, roll, scale, model, flags: 0 });
        }
    }
    out
}

/// Applies the user's changes to a road catalog read from a project.
pub fn apply_catalog_edits(catalog: &mut Vec<dayz_road_tool::roads::CatalogPart>, removed: &std::collections::BTreeSet<String>, added: &[crate::world::RoadPart], folder: &Path) {
    let file = |p: &str| p.rsplit(['\\', '/']).next().unwrap_or(p).to_ascii_lowercase();
    catalog.retain(|c| !removed.contains(&file(&c.path)));
    let mut lib = dayz_road_tool::geometry::Library::new(folder.to_path_buf());
    for a in added {
        if catalog.iter().any(|c| file(&c.path) == file(&a.path)) {
            continue;
        }
        let Ok(model) = lib.model(&a.path) else { continue };
        let road_type_index = catalog.iter().find(|c| c.family == a.family).map(|c| c.road_type_index).unwrap_or(0);
        catalog.push(dayz_road_tool::roads::CatalogPart { path: a.path.clone(), family: a.family.clone(), category: a.category, index: catalog.len() as u32, road_type_index, model });
    }
}

/// A modifier that levels a strip along a road to a smooth profile of the sculpted terrain.
pub fn road_modifier(base: &Heightmap, id: u64, name: &str, center: &[[f32; 2]], width: f32, falloff: f32) -> Option<TerrainModifier> {
    if center.len() < 2 {
        return None;
    }
    // Long straight parts give few points, which would bridge over the terrain between them.
    let mut points: Vec<[f32; 2]> = Vec::with_capacity(center.len() * 2);
    for w in center.windows(2) {
        let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let steps = ((dx * dx + dz * dz).sqrt() / 8.0).ceil().max(1.0) as usize;
        for k in 0..steps {
            let t = k as f32 / steps as f32;
            points.push([w[0][0] + dx * t, w[0][1] + dz * t]);
        }
    }
    points.push(*center.last().unwrap());
    let mut heights: Vec<f32> = points.iter().map(|p| base.sample(p[0], p[1])).collect();
    for _ in 0..24 {
        let prev = heights.clone();
        for k in 1..prev.len() - 1 {
            heights[k] = (prev[k - 1] + 2.0 * prev[k] + prev[k + 1]) / 4.0;
        }
    }
    let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in &points {
        (x0, z0, x1, z1) = (x0.min(p[0]), z0.min(p[1]), x1.max(p[0]), z1.max(p[1]));
    }
    Some(TerrainModifier { id, name: name.to_string(), enabled: true, x: (x0 + x1) * 0.5, z: (z0 + z1) * 0.5, yaw: 0.0, shape: ModShape::Line { points, heights, half_width: width * 0.5 }, mode: ModMode::Flatten, target: None, amount: 0.0, falloff, attached: None })
}

/// Smooths heights along a line so a road sits on an even bed.
pub fn grade_along(base: &mut Heightmap, line: &[[f32; 2]], width: f32, blend: f32) -> Option<crate::terrain::Rect> {
    if line.len() < 2 {
        return None;
    }
    // Height profile sampled along the line, then low-passed.
    let raw: Vec<f32> = line.iter().map(|p| base.sample(p[0], p[1])).collect();
    let mut smoothed = raw.clone();
    for _ in 0..12 {
        let prev = smoothed.clone();
        for k in 1..prev.len() - 1 {
            smoothed[k] = (prev[k - 1] + 2.0 * prev[k] + prev[k + 1]) / 4.0;
        }
    }
    let reach = width * 0.5 + blend;
    let mut area: Option<crate::terrain::Rect> = None;
    let snapshot = base.clone();
    let cell = base.cell;
    for (seg, profile) in line.windows(2).zip(smoothed.windows(2)) {
        let (x0, z0) = (seg[0][0].min(seg[1][0]) - reach, seg[0][1].min(seg[1][1]) - reach);
        let (x1, z1) = (seg[0][0].max(seg[1][0]) + reach, seg[0][1].max(seg[1][1]) + reach);
        let Some(r) = base.rect_of(x0, z0, x1, z1) else { continue };
        area = Some(area.map_or(r, |a| a.union(r)));
        let (dx, dz) = (seg[1][0] - seg[0][0], seg[1][1] - seg[0][1]);
        let l2 = (dx * dx + dz * dz).max(1e-4);
        for j in r.y0..=r.y1 {
            for i in r.x0..=r.x1 {
                let (px, pz) = (i as f32 * cell, j as f32 * cell);
                let t = (((px - seg[0][0]) * dx + (pz - seg[0][1]) * dz) / l2).clamp(0.0, 1.0);
                let d = ((px - seg[0][0] - dx * t).powi(2) + (pz - seg[0][1] - dz * t).powi(2)).sqrt();
                if d > reach {
                    continue;
                }
                let weight = 1.0 - smooth(width * 0.5, reach, d);
                let goal = profile[0] + (profile[1] - profile[0]) * t;
                let orig = snapshot.at(i, j);
                let target = orig + (goal - orig) * weight;
                // Where segments overlap, the strongest correction wins.
                let cur = base.at(i, j);
                if (target - orig).abs() > (cur - orig).abs() {
                    base.set(i, j, target);
                }
            }
        }
    }
    area
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplified_lines_keep_ends_and_corners() {
        let line = [[0.0, 0.0], [10.0, 0.05], [20.0, 0.0], [20.0, 10.0], [20.05, 20.0]];
        assert_eq!(simplify_line(&line, 1.0), vec![[0.0, 0.0], [20.0, 0.0], [20.05, 20.0]]);
    }

    /// Road types are written back to a copy of a real project.
    #[test]
    fn real_road_types_are_written_to_a_project() {
        use dayz_road_tool::{geometry::Library, tv4p};
        let base = Path::new("P:/BalticrusSource/TerrainBuilder/balticrus.tv4p");
        if !base.exists() || std::env::var("DZ_SLOW_TESTS").is_err() { return; }
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("edited.tv4p");
        let parts = Path::new("P:/dz/structures/roads/parts");
        let (catalog, _) = tv4p::catalog(base, &mut Library::new(parts.to_path_buf())).unwrap();
        let first = catalog.iter().find(|c| c.family == "asf2" && c.category == 3).unwrap();
        let mut paths: Vec<String> = catalog.iter().filter(|c| c.road_type_index == first.road_type_index && c.category == 3).map(|c| c.path.clone()).collect();
        let dropped = paths.remove(0);
        tv4p::write_definitions(base, &out, &[tv4p::DefinitionEdit { road_type_index: first.road_type_index, category: 3, paths: paths.clone() }]).unwrap();
        let (edited, _) = tv4p::catalog(&out, &mut Library::new(parts.to_path_buf())).unwrap();
        assert_eq!(edited.len(), catalog.len() - 1);
        assert!(!edited.iter().any(|c| c.path == dropped));
        // Road records stay readable.
        let project = tv4p::load(&out, &mut Library::new(parts.to_path_buf())).unwrap();
        assert!(!project.roads.is_empty());
    }

    /// A road searched across a real terrain, when P: is mounted.
    #[test]
    fn real_road_is_routed_on_terrain() {
        let parts = Path::new("P:/dz/structures/roads/parts");
        let asc = Path::new("P:/BalticrusSource/heightmaps/mapa_wysokosci_mahauu.asc");
        if !parts.exists() || !asc.exists() || std::env::var("DZ_SLOW_TESTS").is_err() { return; }
        let catalog = road_catalog_from_project(Path::new("P:/BalticrusSource/TerrainBuilder/balticrus.tv4p"), parts).unwrap();
        let g = crate::import::read_asc(asc).unwrap();
        let meta = crate::world::WorldMeta::new("R", g.map.cols as u32, g.map.cell);
        let mut w = World::new(meta, 0.0);
        w.base = g.map;
        w.recompose(None);
        let t0 = std::time::Instant::now();
        let mut req = RouteRequest { family: "asf2".into(), points: vec![[2200.0, 6500.0], [2500.0, 6300.0]], terrain_route: true, settings: Default::default(), forbidden: vec![], roads: vec![] };
        let r = build_route(&mut w, &req, &catalog, &Job::default()).unwrap();
        println!("catalog {} parts {} in {:?}, line {}", catalog.len(), r.objects.len(), t0.elapsed(), r.centerline.len());
        assert!(!r.objects.is_empty());
        // A track over the same ground is laid on it piece by piece.
        let rails = rail_catalog();
        assert!(!rails.is_empty());
        req.family = "Rail_TrackE".into();
        req.settings = dayz_road_tool::document::RoutingSettings::rail();
        let t = build_route(&mut w, &req, &rails, &Job::default()).unwrap();
        println!("track pieces {} max elevation {:?}", t.objects.len(), t.objects.iter().map(|o| o.elevation.abs()).fold(0.0f32, f32::max));
        assert!(!t.objects.is_empty());
        assert!(t.objects.iter().any(|o| w.objects.models[o.model as usize].path.contains("\\rail\\tracks\\")), "{:?}", w.objects.models[t.objects[0].model as usize].path);
    }
    use crate::world::WorldMeta;

    fn world(h: f32) -> World {
        World::new(WorldMeta::new("Gen", 256, 8.0), h)
    }

    #[test]
    fn every_terrain_kind_is_finite_and_varied() {
        for kind in TerrainKind::ALL {
            let mut h = Heightmap::new(128, 128, 16.0, 0.0);
            generate_terrain(&mut h, &TerrainParams { kind, ..Default::default() }, &Job::default());
            assert!(h.data.iter().all(|v| v.is_finite()), "{kind:?}");
            let (a, b) = h.min_max();
            assert!(b - a > 1.0, "{kind:?} is flat");
        }
    }
    #[test]
    fn island_is_surrounded_by_sea() {
        let mut h = Heightmap::new(256, 256, 16.0, 0.0);
        generate_terrain(&mut h, &TerrainParams { kind: TerrainKind::Island, ..Default::default() }, &Job::default());
        assert!(h.at(0, 0) < 0.0 && h.at(255, 255) < 0.0);
        assert!(h.at(128, 128) > 10.0);
    }
    #[test]
    fn erosion_changes_terrain_without_nans() {
        let mut h = Heightmap::new(128, 128, 8.0, 0.0);
        generate_terrain(&mut h, &TerrainParams::default(), &Job::default());
        let before = h.data.clone();
        hydraulic_erosion(&mut h, &ErosionParams { droplets: 4000, ..Default::default() }, &Job::default());
        thermal_erosion(&mut h, 3, 35.0, &Job::default());
        assert!(h.data.iter().all(|v| v.is_finite()));
        assert_ne!(before, h.data);
    }
    #[test]
    fn rivers_lower_the_ground_along_their_line() {
        let mut h = Heightmap::new(256, 256, 8.0, 0.0);
        generate_terrain(&mut h, &TerrainParams { kind: TerrainKind::Mountains, ..Default::default() }, &Job::default());
        let before = h.clone();
        let lines = carve_rivers(&mut h, 2, 8.0, 3.0, 5);
        assert!(!lines.is_empty());
        let p = lines[0].points[lines[0].points.len() / 2];
        assert!(h.sample(p[0], p[1]) < before.sample(p[0], p[1]));
    }
    #[test]
    fn auto_paint_follows_slope_and_height_rules() {
        let mut w = world(100.0);
        for j in 0..256 { for i in 128..256 { w.base.set(i, j, 100.0 + (i - 128) as f32 * 8.0); } }
        w.recompose(None);
        let rules = vec![
            PaintRule { layer: 1, min_height: -100.0, max_height: 9000.0, min_slope: 0.0, max_slope: 90.0, noise_scale: 0.0, coverage: 1.0 },
            PaintRule { layer: 6, min_height: -100.0, max_height: 9000.0, min_slope: 30.0, max_slope: 90.0, noise_scale: 0.0, coverage: 1.0 },
        ];
        let out = auto_paint(&w, &rules, 1, None, &Job::default());
        let px = w.mask.width;
        assert_eq!(out[px * px / 2 + px / 4], 1);
        assert_eq!(out[px * px / 2 + px * 3 / 4], 6);
    }
    #[test]
    fn line_objects_are_spaced_and_follow_heading() {
        let mut w = world(10.0);
        let rule = LineRule { model: "dz\\fence.p3d".into(), spacing: 5.0, offset: 0.0, jitter: 0.0, yaw_offset: 0.0, follow_slope: false };
        let o = along_line(&mut w, &[[100.0, 100.0], [100.0, 150.0], [150.0, 150.0]], &rule, 1);
        assert_eq!(o.len(), 21);
        assert!(o[0].yaw.abs() < 1e-3);
        assert!((o[20].yaw - 90.0).abs() < 1e-3);
    }
    #[test]
    fn settlement_avoids_overlap() {
        let mut w = world(10.0);
        let db = crate::model_info::ModelDb::new("Z:/none");
        let rule = SettlementRule { buildings: vec!["dz\\h.p3d".into()], radius: 300.0, ..Default::default() };
        let s = settlement(&mut w, &db, 1000.0, 1000.0, &rule).unwrap();
        assert!(s.objects.len() >= 8);
        for (i, a) in s.objects.iter().enumerate() {
            for b in &s.objects[i + 1..] {
                assert!((a.x - b.x).hypot(a.z - b.z) > 8.0);
            }
        }
    }
    #[test]
    fn grading_evens_out_a_bump_under_the_line() {
        let mut h = Heightmap::new(128, 128, 8.0, 50.0);
        h.set(50, 50, 80.0);
        let line: Vec<[f32; 2]> = (0..40).map(|k| [300.0 + k as f32 * 5.0, 400.0]).collect();
        grade_along(&mut h, &line, 12.0, 8.0).unwrap();
        assert!(h.at(50, 50) < 62.0, "{}", h.at(50, 50));
    }
    #[test]
    fn slope_angles_are_zero_on_flat_ground() {
        let h = Heightmap::new(16, 16, 8.0, 5.0);
        assert_eq!(slope_angles(&h, 40.0, 40.0, 30.0), (0.0, 0.0));
    }
    /// Real road parts when P: is mounted.
    #[test]
    fn road_catalog_fits_a_real_family() {
        let folder = Path::new("P:/dz/structures/roads/parts");
        if !folder.exists() { return; }
        let cat = road_catalog(folder).unwrap();
        let fam = road_families(&cat);
        assert!(fam.iter().any(|f| f == "asf2"), "{fam:?}");
        let mut w = world(10.0);
        let r = build_road(&mut w, &[[100.0, 100.0], [100.0, 300.0], [260.0, 460.0]], "asf2", &cat, "dz\\structures\\roads\\parts", 25.0).unwrap();
        assert!(r.objects.len() >= 8);
        assert!(r.centerline.len() > 8);
    }
}
