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

pub fn auto_paint(w: &World, rules: &[PaintRule], seed: u64, job: &Job) -> Vec<u8> {
    let noise = Perlin::new(seed);
    let (mw, mh) = (w.mask.width, w.mask.height);
    let size = w.meta.size_m();
    let mut out = w.mask.data.clone();
    let done = std::sync::atomic::AtomicUsize::new(0);
    out.par_chunks_mut(mw).enumerate().for_each(|(y, row)| {
        let wz = (1.0 - (y as f32 + 0.5) / mh as f32) * size;
        for (x, v) in row.iter_mut().enumerate() {
            let wx = (x as f32 + 0.5) / mw as f32 * size;
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ScatterRule {
    /// WRP model paths.
    pub models: Vec<String>,
    /// Objects per hectare.
    pub density: f32,
    pub min_distance: f32,
    pub min_slope: f32,
    pub max_slope: f32,
    pub min_height: f32,
    pub max_height: f32,
    /// Mask layers to scatter on; empty allows every layer.
    pub layers: Vec<usize>,
    pub scale_min: f32,
    pub scale_max: f32,
    pub align_to_slope: bool,
    pub seed: u64,
}
impl Default for ScatterRule {
    fn default() -> Self {
        Self { models: Vec::new(), density: 20.0, min_distance: 3.0, min_slope: 0.0, max_slope: 35.0, min_height: 0.5, max_height: 9000.0, layers: Vec::new(), scale_min: 0.85, scale_max: 1.2, align_to_slope: false, seed: 1 }
    }
}

/// Area limit of a generator, in world metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Area {
    pub x0: f32,
    pub z0: f32,
    pub x1: f32,
    pub z1: f32,
}
impl Area {
    pub fn whole(w: &World) -> Self {
        Self { x0: 0.0, z0: 0.0, x1: w.meta.size_m(), z1: w.meta.size_m() }
    }
    pub fn circle(x: f32, z: f32, r: f32) -> Self {
        Self { x0: x - r, z0: z - r, x1: x + r, z1: z + r }
    }
}

/// Fast Poisson-style dart throwing over a grid.
pub fn scatter(w: &mut World, rule: &ScatterRule, area: Area, job: &Job) -> Result<Vec<Obj>> {
    if rule.models.is_empty() {
        bail!("Choose at least one model");
    }
    let models: Vec<u32> = rule.models.iter().map(|m| w.objects.intern(m)).collect();
    let mut rng = StdRng::seed_from_u64(rule.seed);
    let size = w.meta.size_m();
    let (x0, z0, x1, z1) = (area.x0.max(0.0), area.z0.max(0.0), area.x1.min(size), area.z1.min(size));
    let hectares = ((x1 - x0) * (z1 - z0)).max(0.0) / 10_000.0;
    let tries = (hectares * rule.density * 2.5) as usize;
    let cell = rule.min_distance.max(0.5);
    let mut grid: HashMap<(i32, i32), Vec<[f32; 2]>> = HashMap::new();
    let limit = (hectares * rule.density) as usize;
    let mut out = Vec::new();
    let min2 = rule.min_distance * rule.min_distance;
    for n in 0..tries {
        if n % 8192 == 0 {
            job.set_progress(n as f32 / tries.max(1) as f32);
            job.check()?;
        }
        if out.len() >= limit {
            break;
        }
        let (x, z) = (rng.gen_range(x0..x1.max(x0 + 0.01)), rng.gen_range(z0..z1.max(z0 + 0.01)));
        let h = w.height.sample(x, z);
        if h < rule.min_height || h > rule.max_height {
            continue;
        }
        let slope = w.height.slope(x, z);
        if slope < rule.min_slope || slope > rule.max_slope {
            continue;
        }
        if !rule.layers.is_empty() && !rule.layers.contains(&(w.mask.sample(x, z, size) as usize)) {
            continue;
        }
        let key = ((x / cell).floor() as i32, (z / cell).floor() as i32);
        let mut blocked = false;
        'near: for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(c) = grid.get(&(key.0 + dx, key.1 + dz)) {
                    if c.iter().any(|p| (p[0] - x).powi(2) + (p[1] - z).powi(2) < min2) {
                        blocked = true;
                        break 'near;
                    }
                }
            }
        }
        if blocked {
            continue;
        }
        grid.entry(key).or_default().push([x, z]);
        let (pitch, roll) = if rule.align_to_slope { slope_angles(&w.height, x, z, 0.0) } else { (0.0, 0.0) };
        out.push(Obj {
            x, z, elevation: 0.0, yaw: rng.gen_range(0.0..360.0), pitch, roll,
            scale: rng.gen_range(rule.scale_min..=rule.scale_max.max(rule.scale_min + 1e-4)),
            model: *models.choose(&mut rng).unwrap(), flags: 0,
        });
    }
    Ok(out)
}

/// Pitch and roll laying a model flat on the terrain, for a given yaw.
pub fn slope_angles(h: &Heightmap, x: f32, z: f32, yaw: f32) -> (f32, f32) {
    let n = h.normal(x, z);
    let (s, c) = yaw.to_radians().sin_cos();
    let forward = n[0] * s + n[2] * c;
    let right = n[0] * c - n[2] * s;
    ((forward).clamp(-1.0, 1.0).asin().to_degrees(), (-right).clamp(-1.0, 1.0).asin().to_degrees())
}

/// Forest generation through the DayZForests engine. Each selected mask layer becomes a zone.
pub struct ForestRequest {
    /// (mask layer, forest preset name)
    pub zones: Vec<(usize, String)>,
    pub seed: u64,
    pub max_slope: f32,
    pub clearing_strength: f64,
}

pub fn forest_preset_names() -> Vec<String> {
    forest_core::species::zone_presets().iter().map(|z| z.name.to_string()).collect()
}

pub fn generate_forest(w: &mut World, assets: &AssetIndex, req: &ForestRequest, job: &Job) -> Result<(Vec<Obj>, Vec<String>)> {
    use forest_core::mask::{MaskImage, Rgb8};
    let size = w.meta.size_m() as f64;
    let presets = forest_core::species::zone_presets();
    let mut project = forest_core::preset::ForestProject { map_size_m: size, seed: req.seed, max_slope_deg: Some(req.max_slope as f64), clearing_strength: req.clearing_strength, ..Default::default() };
    project.easting_offset = 0.0;
    project.northing_offset = 0.0;
    let mut used = vec![false; 256];
    for (layer, name) in &req.zones {
        let preset = presets.iter().find(|p| p.name == name).with_context(|| format!("Unknown forest preset {name}"))?;
        project.zones.push(preset.to_zone_def(Rgb8(unique_color(*layer))));
        used[*layer] = true;
    }
    if project.zones.is_empty() {
        bail!("Add at least one zone");
    }
    // The engine reads RGB masks; a 4096 image keeps zones accurate to about 3 m.
    let side = w.mask.width.min(4096) as u32;
    let mut mask = MaskImage { width: side, height: side, rgba: vec![0; (side * side * 4) as usize] };
    let (mw, mh) = (w.mask.width, w.mask.height);
    let src = &w.mask.data;
    mask.rgba.par_chunks_mut(side as usize * 4).enumerate().for_each(|(y, row)| {
        let sy = y * mh / side as usize;
        for x in 0..side as usize {
            let l = src[sy * mw + x * mw / side as usize];
            let c = if used[l as usize] { unique_color(l as usize) } else { [0, 0, 0] };
            row[x * 4..x * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    });
    let values: Vec<f32> = (0..w.height.rows).rev().flat_map(|j| (0..w.height.cols).map(move |i| (i, j))).map(|(i, j)| w.height.at(i, j)).collect();
    let hm = forest_core::heightmap::AscHeightmap {
        ncols: w.height.cols, nrows: w.height.rows, xllcorner: 0.0, yllcorner: 0.0,
        cellsize: w.height.cell as f64, nodata: -9999.0, values, x_origin_offset: 0.0, y_origin_offset: 0.0,
    };
    let j2 = job.clone();
    let progress = move |v: f64| j2.set_progress(v as f32);
    let (placed, stats) = forest_core::scatter::generate(&project, Some(&mask), None, Some(&hm), None, &progress).map_err(anyhow::Error::msg)?;
    let mut missing: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(placed.len());
    for p in placed {
        let Some(path) = assets.resolve(&p.model) else {
            *missing.entry(p.model.clone()).or_default() += 1;
            continue;
        };
        let model = w.objects.intern(path);
        out.push(Obj { x: p.x as f32, z: p.y as f32, elevation: p.elevation as f32, yaw: p.yaw as f32, pitch: p.pitch as f32, roll: p.roll as f32, scale: p.scale as f32, model, flags: 0 });
    }
    let mut notes = vec![stats.summary()];
    if !missing.is_empty() {
        let mut names: Vec<_> = missing.into_keys().collect();
        names.sort();
        notes.push(format!("Models not found on P:\\ and skipped: {}", names.join(", ")));
    }
    Ok((out, notes))
}

fn unique_color(i: usize) -> [u8; 3] {
    [((i * 53 + 40) % 240 + 10) as u8, ((i * 97 + 90) % 240 + 10) as u8, ((i * 151 + 30) % 240 + 10) as u8]
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
pub fn pad_modifier(w: &mut World, object: u32, half_x: f32, half_z: f32, falloff: f32) -> Option<TerrainModifier> {
    let o = *w.objects.get(object)?;
    let id = w.new_modifier_id();
    let model = w.objects.models.get(o.model as usize).map(|m| m.path.rsplit('\\').next().unwrap_or("").to_string()).unwrap_or_default();
    Some(TerrainModifier {
        id, name: format!("Pad: {model}"), enabled: true, x: o.x, z: o.z, yaw: o.yaw,
        shape: ModShape::Rect { half_x: half_x + 1.0, half_z: half_z + 1.0 },
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
    let mut f: Vec<String> = catalog.iter().filter(|c| c.category == 6).map(|c| c.family.clone()).collect();
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
        let out = auto_paint(&w, &rules, 1, &Job::default());
        let px = w.mask.width;
        assert_eq!(out[px * px / 2 + px / 4], 1);
        assert_eq!(out[px * px / 2 + px * 3 / 4], 6);
    }
    #[test]
    fn scatter_respects_distance_area_and_mask() {
        let mut w = world(100.0);
        let size = w.meta.size_m();
        let layer_px = w.mask.width;
        for y in 0..layer_px { for x in 0..layer_px / 2 { w.mask.data[y * layer_px + x] = 2; } }
        let rule = ScatterRule { models: vec!["dz\\a.p3d".into()], density: 50.0, min_distance: 4.0, layers: vec![2], ..Default::default() };
        let area = Area::whole(&w);
        let out = scatter(&mut w, &rule, area, &Job::default()).unwrap();
        assert!(out.len() > 100);
        assert!(out.iter().all(|o| o.x < size / 2.0 + 2.0));
        for (i, a) in out.iter().enumerate().take(400) {
            for b in out.iter().skip(i + 1).take(400) {
                assert!((a.x - b.x).powi(2) + (a.z - b.z).powi(2) >= 15.9);
            }
        }
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
