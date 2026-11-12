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
    /// A plateau cut by branching valleys.
    Highlands,
    /// Lowland left by an ice sheet: low rolling ground with kettle holes and eskers.
    Moraine,
}
impl TerrainKind {
    pub const ALL: [TerrainKind; 10] = [Self::Hills, Self::Mountains, Self::Highlands, Self::Moraine, Self::Plains, Self::Island, Self::Archipelago, Self::Valley, Self::Canyons, Self::Coast];
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
            Self::Highlands => "Highlands",
            Self::Moraine => "Moraine lowland",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
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
    /// How far slopes are worn smooth, leaving detail on flats and crests; 0 is plain noise.
    pub erosion: f32,
    /// Width over which the result fades into the terrain at the edge of a shape, in metres.
    pub edge: f32,
}
impl Default for TerrainParams {
    fn default() -> Self {
        Self { kind: TerrainKind::Hills, seed: 1, base: 120.0, amplitude: 160.0, feature: 2400.0, octaves: 8, roughness: 0.5, warp: 200.0, blend: 1.0, erosion: 0.5, edge: 150.0 }
    }
}

pub fn generate_terrain(h: &mut Heightmap, p: &TerrainParams, job: &Job) {
    generate_terrain_in(h, p, None, job)
}

/// Like `generate_terrain`, shaped to fit `frame` (west, south, east, north in metres): an
/// island or a valley then lies within that rectangle, and nothing outside it is touched.
pub fn generate_terrain_in(h: &mut Heightmap, p: &TerrainParams, frame: Option<[f32; 4]>, job: &Job) {
    let noise = Perlin::new(p.seed);
    let warp_x = Perlin::new(p.seed ^ 0xA5A5);
    let warp_z = Perlin::new(p.seed ^ 0x5A5A);
    let (cols, rows, cell) = (h.cols, h.rows, h.cell);
    let side = cols.max(rows) as f32 * cell;
    let [fx0, fz0, fx1, fz1] = frame.unwrap_or([0.0, 0.0, side, side]);
    let f = 1.0 / p.feature.max(10.0);
    let done = std::sync::atomic::AtomicUsize::new(0);
    h.data.par_chunks_mut(cols).enumerate().for_each(|(j, row)| {
        for (i, v) in row.iter_mut().enumerate() {
            let (mut x, mut z) = (i as f32 * cell, j as f32 * cell);
            if x < fx0 - cell || z < fz0 - cell || x > fx1 + cell || z > fz1 + cell {
                continue;
            }
            let (u, w) = ((x - fx0) / (fx1 - fx0).max(1.0) * 2.0 - 1.0, (z - fz0) / (fz1 - fz0).max(1.0) * 2.0 - 1.0);
            if p.warp > 0.0 {
                let (ox, oz) = (x, z);
                x += warp_x.fbm(ox * f * 1.7, oz * f * 1.7, 3, 0.5, 2.0) * p.warp;
                z += warp_z.fbm(ox * f * 1.7, oz * f * 1.7, 3, 0.5, 2.0) * p.warp;
            }
            let (nx, nz) = (x * f, z * f);
            let fbm = noise.fbm(nx, nz, p.octaves, p.roughness, 2.0);
            // The same sum with its slopes worn smooth, for the land that should look weathered.
            let worn = noise.eroded(nx, nz, p.octaves, p.roughness, 2.0, p.erosion);
            let radial = (u * u + w * w).sqrt();
            let height = match p.kind {
                TerrainKind::Hills => p.base + worn * p.amplitude * 0.75,
                TerrainKind::Plains => p.base + noise.fbm(nx * 0.6, nz * 0.6, p.octaves, p.roughness * 0.7, 2.0) * p.amplitude * 0.25,
                TerrainKind::Mountains => {
                    let r = noise.ridged(nx, nz, p.octaves, p.roughness + 0.05, 2.1);
                    let mask = (noise.fbm(nx * 0.35 + 9.0, nz * 0.35, 3, 0.5, 2.0) + 0.55).clamp(0.0, 1.0);
                    // Crests stay sharp, the flanks between them are worn and valley floors widen.
                    let relief = r.powf(1.4) * mask * 1.4 + worn * 0.2;
                    let floor = smooth(0.0, 0.22, relief);
                    p.base + relief * (0.55 + 0.45 * floor) * p.amplitude
                }
                TerrainKind::Highlands => {
                    let plateau = 0.72 + worn * 0.14;
                    // Valleys follow the crest lines of a coarse ridged noise and branch with it.
                    let lines = noise.ridged(nx * 0.8 + 3.0, nz * 0.8, 4, 0.5, 2.0);
                    let cut = smooth(0.58, 0.93, lines);
                    p.base + (plateau - cut * (0.6 + 0.15 * fbm)) * p.amplitude
                }
                TerrainKind::Moraine => {
                    let rolling = noise.eroded(nx * 1.6, nz * 1.6, p.octaves, p.roughness * 0.9, 2.0, p.erosion) * 0.22;
                    // Kettle holes where blocks of ice melted, and eskers left by the streams under it.
                    let kettles = smooth(0.52, 0.8, noise.get(nx * 6.0 + 40.0, nz * 6.0)) * 0.11;
                    let eskers = smooth(0.84, 0.96, noise.ridged(nx * 2.3, nz * 2.3 + 11.0, 2, 0.5, 2.0)) * 0.07;
                    p.base + (rolling - kettles + eskers) * p.amplitude
                }
                TerrainKind::Island => {
                    let edge = 1.0 - smooth(0.45, 0.95, radial + fbm * 0.18);
                    -18.0 + edge * (p.base + 18.0 + (worn + 0.25).max(0.0) * p.amplitude * edge)
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
                    -20.0 + land * (p.base + 20.0 + (worn + 0.3).max(0.0) * p.amplitude * smooth(0.0, 0.8, shore))
                }
            };
            *v += (height - *v) * p.blend;
        }
        let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        job.set_progress(n as f32 / rows as f32);
    });
}

/// Keeps `result` only inside `area`, fading into `original` over `feather` metres at its edge.
pub fn blend_in_area(original: &Heightmap, result: &mut Heightmap, area: &crate::shapes::PolygonIndex, feather: f32) {
    let (cols, rows, cell) = (original.cols, original.rows, original.cell);
    let reach = (feather / cell).max(0.0);
    // Distance of every vertex from the outside of the area, in cells, by two sweeps.
    let mut dist: Vec<f32> = (0..rows * cols).into_par_iter().map(|k| if area.contains((k % cols) as f32 * cell, (k / cols) as f32 * cell) { f32::MAX } else { 0.0 }).collect();
    const DIAGONAL: f32 = std::f32::consts::SQRT_2;
    for j in 0..rows {
        for i in 0..cols {
            let k = j * cols + i;
            if dist[k] == 0.0 {
                continue;
            }
            let mut d = dist[k];
            if i > 0 { d = d.min(dist[k - 1] + 1.0); }
            if j > 0 { d = d.min(dist[k - cols] + 1.0); }
            if i > 0 && j > 0 { d = d.min(dist[k - cols - 1] + DIAGONAL); }
            if i + 1 < cols && j > 0 { d = d.min(dist[k - cols + 1] + DIAGONAL); }
            dist[k] = d;
        }
    }
    for j in (0..rows).rev() {
        for i in (0..cols).rev() {
            let k = j * cols + i;
            if dist[k] == 0.0 {
                continue;
            }
            let mut d = dist[k];
            if i + 1 < cols { d = d.min(dist[k + 1] + 1.0); }
            if j + 1 < rows { d = d.min(dist[k + cols] + 1.0); }
            if i + 1 < cols && j + 1 < rows { d = d.min(dist[k + cols + 1] + DIAGONAL); }
            if i > 0 && j + 1 < rows { d = d.min(dist[k + cols - 1] + DIAGONAL); }
            dist[k] = d;
        }
    }
    for (k, v) in result.data.iter_mut().enumerate() {
        let w = if dist[k] == 0.0 { 0.0 } else if reach <= 0.0 { 1.0 } else { smooth(0.0, reach, dist[k]) };
        *v = original.data[k] + (*v - original.data[k]) * w;
    }
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ScatterRule {
    /// (model, weight)
    pub models: Vec<(String, f32)>,
    /// Object preset used instead of `models`.
    pub preset: Option<u64>,
    /// Objects per hectare.
    pub density: f32,
    pub min_distance: f32,
    pub min_slope: f32,
    pub max_slope: f32,
    pub min_height: f32,
    pub max_height: f32,
    /// Surface layers to scatter on, by name; empty allows every one.
    pub layers: Vec<String>,
    pub scale_min: f32,
    pub scale_max: f32,
    pub align_to_slope: bool,
    pub seed: u64,
}

impl Default for ScatterRule {
    fn default() -> Self {
        Self { models: Vec::new(), preset: None, density: 40.0, min_distance: 3.0, min_slope: 0.0, max_slope: 35.0, min_height: 0.5, max_height: 9000.0, layers: Vec::new(), scale_min: 0.9, scale_max: 1.1, align_to_slope: false, seed: 1 }
    }
}

/// Scatters objects inside polygons, or over the whole map without any.
pub fn scatter(w: &mut World, rule: &ScatterRule, area: Option<&crate::shapes::PolygonIndex>, bounds: [f32; 4], job: &Job) -> Result<Vec<Obj>> {
    let total: f32 = rule.models.iter().map(|m| m.1).sum();
    if rule.models.is_empty() || total <= 0.0 {
        bail!("Choose at least one model");
    }
    let models: Vec<(u32, f32)> = rule.models.iter().map(|(m, k)| (w.objects.intern(m), *k)).collect();
    let layers: Vec<usize> = rule.layers.iter().filter_map(|n| w.layers.iter().position(|l| l.name == *n)).collect();
    let mut rng = StdRng::seed_from_u64(rule.seed);
    let size = w.meta.size_m();
    let [x0, z0, x1, z1] = [bounds[0].max(0.0), bounds[1].max(0.0), bounds[2].min(size), bounds[3].min(size)];
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
        if area.is_some_and(|a| !a.contains(x, z)) {
            continue;
        }
        let h = w.height.sample(x, z);
        if h < rule.min_height || h > rule.max_height {
            continue;
        }
        let slope = w.height.slope(x, z);
        if slope < rule.min_slope || slope > rule.max_slope {
            continue;
        }
        if !layers.is_empty() && !layers.contains(&(w.mask.sample(x, z, size) as usize)) {
            continue;
        }
        let key = ((x / cell).floor() as i32, (z / cell).floor() as i32);
        let near = (-1..=1).any(|dz| (-1..=1).any(|dx| grid.get(&(key.0 + dx, key.1 + dz)).is_some_and(|c| c.iter().any(|p| (p[0] - x).powi(2) + (p[1] - z).powi(2) < min2))));
        if near {
            continue;
        }
        grid.entry(key).or_default().push([x, z]);
        let yaw = rng.gen_range(0.0..360.0);
        let (pitch, roll) = if rule.align_to_slope { slope_angles(&w.height, x, z, yaw) } else { (0.0, 0.0) };
        let mut pick = rng.gen_range(0.0..total);
        let mut model = models[0].0;
        for (m, k) in &models {
            model = *m;
            pick -= k;
            if pick <= 0.0 { break; }
        }
        out.push(Obj { x, z, elevation: 0.0, yaw, pitch, roll, scale: rng.gen_range(rule.scale_min..=rule.scale_max.max(rule.scale_min + 1e-4)), model, flags: 0 });
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

/// Forest generation through the DayZForests engine, with a whole DayZForests project.
#[derive(Clone)]
pub struct ForestRequest {
    pub project: forest_core::preset::ForestProject,
    /// (mask layer, forest preset name)
    pub zones: Vec<(usize, String)>,
    /// (polygons, forest preset name)
    pub areas: Vec<(crate::shapes::ShapeLayer, String)>,
    /// Density of the areas in place of what their preset says.
    pub area_density: Option<f32>,
    /// Needed by areas that follow the colours of the satellite image.
    pub satellite: Option<forest_core::mask::MaskImage>,
    /// Applied to the areas made from shapes.
    pub color_filter: Option<forest_core::preset::ColorFilter>,
    /// (preset, share) planted along the edges.
    pub edge_presets: Vec<(String, f32)>,
    pub user_presets: Vec<forest_core::user_presets::UserPreset>,
    /// (surface layer, margin in metres) kept free of trees.
    pub skip_surfaces: Vec<(usize, f32)>,
    pub keep_out: Option<KeepOut>,
}

/// Roads and buildings a forest stays away from.
#[derive(Clone, Default)]
pub struct KeepOut {
    pub roads: Vec<Vec<[f32; 2]>>,
    pub road_margin: f32,
    /// (x, z, yaw, half x, half z)
    pub buildings: Vec<[f32; 5]>,
    pub building_margin: f32,
}

impl KeepOut {
    /// Marks what is off limits on a 2 m grid.
    fn raster(&self, size: f32) -> (usize, Vec<bool>) {
        const CELL: f32 = 2.0;
        let side = (size / CELL).ceil() as usize + 1;
        let mut grid = vec![false; side * side];
        let mut fill = |x0: f32, z0: f32, x1: f32, z1: f32, test: &dyn Fn(f32, f32) -> bool| {
            let span = |a: f32, b: f32| (((a / CELL).floor().max(0.0)) as usize, ((b / CELL).ceil().max(0.0) as usize).min(side - 1));
            let (i0, i1) = span(x0, x1);
            let (j0, j1) = span(z0, z1);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    if test(i as f32 * CELL, j as f32 * CELL) {
                        grid[j * side + i] = true;
                    }
                }
            }
        };
        // A road is about 4 m to each side of its line before the margin starts.
        let reach = self.road_margin + 4.0;
        for line in &self.roads {
            for s in line.windows(2) {
                let (a, b) = (s[0], s[1]);
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let l2 = (dx * dx + dz * dz).max(1e-4);
                fill(a[0].min(b[0]) - reach, a[1].min(b[1]) - reach, a[0].max(b[0]) + reach, a[1].max(b[1]) + reach, &|x, z| {
                    let t = (((x - a[0]) * dx + (z - a[1]) * dz) / l2).clamp(0.0, 1.0);
                    (a[0] + dx * t - x).powi(2) + (a[1] + dz * t - z).powi(2) <= reach * reach
                });
            }
        }
        for b in &self.buildings {
            let (hx, hz) = (b[3] + self.building_margin, b[4] + self.building_margin);
            let r = (hx * hx + hz * hz).sqrt();
            let (s, c) = b[2].to_radians().sin_cos();
            fill(b[0] - r, b[1] - r, b[0] + r, b[1] + r, &|x, z| {
                let (dx, dz) = (x - b[0], z - b[1]);
                (dx * c - dz * s).abs() <= hx && (dx * s + dz * c).abs() <= hz
            });
        }
        (side, grid)
    }
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

/// A forest and the further passes over its shapes, each with settings of its own.
pub fn generate_forest_layers(w: &mut World, assets: &AssetIndex, req: ForestRequest, shapes: Option<crate::shapes::ShapeLayer>, layers: &[crate::library::ForestLayer], job: &Job) -> Result<(Vec<Obj>, Vec<String>)> {
    let base = (!layers.is_empty()).then(|| req.clone());
    let (mut objects, mut notes) = generate_forest(w, assets, req, job)?;
    let (Some(base), Some(shapes)) = (base, shapes) else { return Ok((objects, notes)) };
    for l in layers {
        job.check()?;
        let mut r = base.clone();
        let p = &mut r.project;
        p.areas.clear();
        p.zones.clear();
        p.generation_order.clear();
        p.edges.enabled = false;
        p.edges.preset_mix.clear();
        (p.seed, p.spacing_multiplier, p.scale_min, p.scale_max) = (l.seed, l.spacing, l.scale_min, l.scale_max);
        (p.clearing_strength, p.clearing_scale_m, p.edge_padding_m) = (l.clearing_strength, l.clearing_scale_m, l.edge_padding_m);
        p.max_slope_deg = (l.max_slope_deg < 90.0).then_some(l.max_slope_deg);
        r.zones.clear();
        r.edge_presets.clear();
        r.areas = vec![(shapes.clone(), l.preset.clone())];
        r.area_density = (l.density_per_ha > 0.0).then_some(l.density_per_ha);
        let (more, n) = generate_forest(w, assets, r, job)?;
        notes.extend(n);
        objects.extend(keep_apart(&objects, more, l.gap));
    }
    Ok((objects, notes))
}

/// What is at least `gap` metres from everything already planted.
fn keep_apart(planted: &[Obj], new: Vec<Obj>, gap: f32) -> Vec<Obj> {
    if gap <= 0.0 {
        return new;
    }
    let key = |o: &Obj| ((o.x / gap).floor() as i32, (o.z / gap).floor() as i32);
    let mut grid: std::collections::HashMap<(i32, i32), Vec<[f32; 2]>> = std::collections::HashMap::new();
    for o in planted {
        grid.entry(key(o)).or_default().push([o.x, o.z]);
    }
    new.into_iter().filter(|o| {
        let (cx, cz) = key(o);
        !(cz - 1..=cz + 1).any(|z| (cx - 1..=cx + 1).any(|x| grid.get(&(x, z)).is_some_and(|c| c.iter().any(|p| (p[0] - o.x).hypot(p[1] - o.z) < gap))))
    }).collect()
}

pub fn generate_forest(w: &mut World, assets: &AssetIndex, req: ForestRequest, job: &Job) -> Result<(Vec<Obj>, Vec<String>)> {
    use forest_core::mask::{MaskImage, Rgb8};
    use forest_core::preset::{AreaDef, GenStep, MixEntry, ZoneDef};
    let size = w.meta.size_m() as f64;
    let vanilla = forest_core::species::vanilla_library();
    let mut project = req.project;
    project.map_size_m = size;
    // Without an edge a forest fills its whole shape, right up to the outline.
    if project.edges.enabled {
        project.edges.outside_m.get_or_insert(0.0);
    } else {
        project.edges.jagged_m = 0.0;
        project.edges.blend_inside_m = 0.0;
    }
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
            project.areas.push(AreaDef { label: format!("{} {}", layer.name, i + 1), density_per_ha: req.area_density.unwrap_or(preset.density_per_ha), species_weights: species_weights.clone(), polygon: s.points.iter().map(|p| [p[0] as f64, p[1] as f64]).collect(), color_filter: req.color_filter.clone(), ..Default::default() });
            project.generation_order.push(GenStep::Area(project.areas.len() - 1));
        }
    }
    for (layer, margin) in &req.skip_surfaces {
        let color = w.layers.get(*layer).context("Unknown surface layer")?.color;
        project.cut_zones.push(forest_core::preset::CutZone { color: Rgb8(color), margin_m: *margin });
        // Cutting has to come after every area, whatever order the project had.
        project.generation_order.retain(|s| *s != GenStep::Cut);
        project.generation_order.push(GenStep::Cut);
    }
    if !req.edge_presets.is_empty() {
        project.edges.preset_mix.clear();
        for (name, share) in &req.edge_presets {
            let preset = find_preset(name, &req.user_presets).with_context(|| format!("Unknown forest preset {name}"))?;
            let species_weights = preset_weights(&mut project, &preset, &vanilla);
            project.edges.preset_mix.push(MixEntry { name: name.clone(), share: *share, spatial: false, color_filter: None, density_per_ha: project.edges.density_per_ha, species_weights });
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
    // The engine plants edges on both sides of a boundary; a forest ends at its area.
    let areas = crate::shapes::PolygonIndex::new(&crate::shapes::ShapeLayer { name: String::new(), visible: true, shapes: project.areas.iter().filter(|a| a.enabled && !a.cutting).map(|a| crate::shapes::MapShape { kind: crate::shapes::ShapeKind::Polygon, points: a.polygon.iter().map(|p| [p[0] as f32, p[1] as f32]).collect(), attributes: Vec::new() }).collect() });
    let zone_colors: Vec<[u8; 3]> = project.zones.iter().map(|z| z.color.0).collect();
    let in_zone = |x: f32, z: f32| {
        mask.as_ref().is_some_and(|m| {
            let (px, py) = (((x as f64 / size * m.width as f64) as usize).min(m.width as usize - 1), (((1.0 - z as f64 / size) * m.height as f64) as usize).min(m.height as usize - 1));
            let at = (py * m.width as usize + px) * 4;
            zone_colors.iter().any(|c| c[0] == m.rgba[at] && c[1] == m.rgba[at + 1] && c[2] == m.rgba[at + 2])
        })
    };
    let blocked = req.keep_out.as_ref().map(|k| k.raster(size as f32));
    let free = |x: f32, z: f32| blocked.as_ref().is_none_or(|(side, grid)| {
        let (i, j) = (((x / 2.0).round().max(0.0) as usize).min(side - 1), ((z / 2.0).round().max(0.0) as usize).min(side - 1));
        !grid[j * side + i]
    });
    let beyond = if project.edges.enabled { project.edges.outside_m.unwrap_or(0.0) as f32 } else { 0.0 };
    let near_area = |x: f32, z: f32| project.areas.iter().filter(|a| a.enabled && !a.cutting).any(|a| {
        let n = a.polygon.len();
        (0..n).any(|i| {
            let (p, q) = (a.polygon[i], a.polygon[(i + 1) % n]);
            let (ax, az, dx, dz) = (p[0] as f32, p[1] as f32, (q[0] - p[0]) as f32, (q[1] - p[1]) as f32);
            let t = (((x - ax) * dx + (z - az) * dz) / (dx * dx + dz * dz).max(1e-6)).clamp(0.0, 1.0);
            (ax + dx * t - x).powi(2) + (az + dz * t - z).powi(2) <= beyond * beyond
        })
    });
    let near_zone = |x: f32, z: f32| [1.0f32, 0.5].iter().any(|r| (0..8).any(|k| {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        let (px, pz) = (x + a.cos() * beyond * r, z + a.sin() * beyond * r);
        (0.0..size as f32).contains(&px) && (0.0..size as f32).contains(&pz) && in_zone(px, pz)
    }));
    let inside: Vec<_> = placed.into_iter().filter(|p| {
        let (x, z) = (p.x as f32, p.y as f32);
        areas.contains(x, z) || in_zone(x, z) || (beyond > 0.0 && (near_area(x, z) || near_zone(x, z)))
    }).collect();
    let total = inside.len();
    let placed: Vec<_> = inside.into_iter().filter(|p| free(p.x as f32, p.y as f32)).collect();
    let kept_away = total - placed.len();
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
    if kept_away > 0 {
        notes.push(format!("Left out near roads and buildings: {kept_away}"));
    }
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
        mode: ModMode::Flatten, target: None, amount: 0.0, falloff, attached: Some(object), follow: false })
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
    pub decks: Vec<crate::world::BridgeDeck>,
    /// (x, z, height) of the designed line, empty when the road just follows the terrain.
    pub profile: Vec<[f32; 3]>,
    /// The road pieces laid, in order, for going on from either end later.
    pub parts: Vec<dayz_road_tool::roads::PlacedPart>,
}

/// Length of the end pieces of roads, which are all `*_6konec`.
const CAP_LENGTH: f64 = 6.25;

/// Where a road's closing cap stands: `position` and `yaw` are given as for the cap that opens
/// a road, the way road tools record both. Its origin is its outer end, so it moves there and
/// faces back along the road.
pub fn closing_cap(position: [f64; 2], yaw: f64) -> ([f64; 2], f64) {
    let (s, c) = yaw.to_radians().sin_cos();
    ([position[0] + s * CAP_LENGTH, position[1] + c * CAP_LENGTH], yaw + 180.0)
}

/// The pose a closing cap was placed with, from the pose of its object: the reverse of
/// [`closing_cap`].
pub fn closing_cap_placement(at: [f64; 2], yaw: f64) -> ([f64; 2], f64) {
    let yaw = yaw - 180.0;
    let (s, c) = yaw.to_radians().sin_cos();
    ([at[0] - s * CAP_LENGTH, at[1] - c * CAP_LENGTH], yaw)
}

/// Position and yaw of a piece of a road as an object.
pub fn road_part_pose(parts: &[dayz_road_tool::roads::PlacedPart], k: usize) -> ([f64; 2], f32) {
    let is_cap = |i: usize| parts[i].model.to_ascii_lowercase().contains("konec");
    let (position, yaw) = if k > 0 && is_cap(k) && !is_cap(k - 1) { closing_cap(parts[k].position, parts[k].rotation) } else { (parts[k].position, parts[k].rotation) };
    (position, yaw.rem_euclid(360.0) as f32)
}

pub fn build_road(w: &mut World, points: &[[f32; 2]], family: &str, catalog: &[dayz_road_tool::roads::CatalogPart], models_prefix: &str, min_radius: f64) -> Result<RoadResult> {
    use dayz_road_tool::{document::RoutingSettings, roads};
    let settings = RoutingSettings { min_radius, ..Default::default() };
    let line: Vec<[f64; 2]> = points.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
    let parts = roads::fit(&line, family, catalog, &settings, |_, _| true)?;
    let center = roads::centerline(&parts, catalog);
    let mut objects = Vec::with_capacity(parts.len());
    for (k, p) in parts.iter().enumerate() {
        let model = w.objects.intern(&format!("{models_prefix}\\{}", p.model));
        let (at, yaw) = road_part_pose(&parts, k);
        objects.push(Obj { x: at[0] as f32, z: at[1] as f32, elevation: 0.0, yaw, pitch: 0.0, roll: 0.0, scale: 1.0, model, flags: 0 });
    }
    Ok(RoadResult { objects, centerline: center.iter().map(|p| [p[0] as f32, p[1] as f32]).collect(), decks: Vec::new(), profile: Vec::new(), parts })
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

/// The bed under a track lies this far below its sleepers, so the ground cannot show through.
const RAIL_BED_SINK: f32 = 0.25;
/// Height of the running surface of a track above the origin of its models.
const RAIL_TOP: f32 = 0.23;
/// How much higher a road has to be than the terrain under a graded track to meet its rails.
pub const RAIL_CROSSING_LIFT: f32 = RAIL_BED_SINK + RAIL_TOP;
/// The same over terrain the track simply lies on.
pub const RAIL_CROSSING_LIFT_UNGRADED: f32 = RAIL_TOP;

/// Heights along a road or track within its limits of grade, cut and fill. Where the limits
/// cannot be met on the finer samples used here they are widened as little as it takes.
fn design_heights(base: &Heightmap, stretches: &[Vec<[f64; 2]>], s: &dayz_road_tool::document::RoutingSettings, rail: bool) -> Result<Vec<[f32; 3]>> {
    use dayz_road_tool::grading::{design_profile, Earthwork};
    let mut points: Vec<[f64; 2]> = Vec::new();
    for stretch in stretches {
        for w in stretch.windows(2) {
            let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let steps = ((dx * dx + dz * dz).sqrt() / 4.0).ceil().max(1.0) as usize;
            points.extend((0..steps).map(|k| { let t = k as f64 / steps as f64; [w[0][0] + dx * t, w[0][1] + dz * t] }));
        }
        points.extend(stretch.last().copied());
    }
    points.dedup_by(|a, b| (a[0] - b[0]).abs() + (a[1] - b[1]).abs() < 1e-3);
    let ground: Vec<f64> = points.iter().map(|p| base.sample(p[0] as f32, p[1] as f32) as f64).collect();
    let mut step = vec![0.0];
    step.extend(points.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt()));
    // Even without limits a track evens out the small bumps of the terrain.
    let least = if rail { 0.5 } else { 0.0 };
    let mut last = None;
    for extra in [0.0, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0] {
        let e = Earthwork { max_grade: s.max_grade, max_cut: s.max_cut.max(least) + extra, max_fill: s.max_fill.max(least) + extra, side_slope: 1.5 };
        match design_profile(&ground, &step, &e) {
            Ok(h) => return Ok(points.iter().zip(h).map(|(p, h)| [p[0] as f32, p[1] as f32, h as f32]).collect()),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("No profile")))
}

/// Height of a designed line at its point nearest to `p`.
fn height_along(line: &[[f32; 3]], p: [f64; 2]) -> f32 {
    let (x, z) = (p[0] as f32, p[1] as f32);
    let mut best = (f32::MAX, line.first().map_or(0.0, |a| a[2]));
    for s in line.windows(2) {
        let (dx, dz) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
        let t = (((x - s[0][0]) * dx + (z - s[0][1]) * dz) / (dx * dx + dz * dz).max(1e-6)).clamp(0.0, 1.0);
        let d = (s[0][0] + dx * t - x).powi(2) + (s[0][1] + dz * t - z).powi(2);
        if d < best.0 {
            best = (d, s[0][2] + (s[1][2] - s[0][2]) * t);
        }
    }
    best.1
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
    /// The open end of a road this one goes on from: it starts there with that heading and
    /// without an end piece.
    pub start: Option<dayz_road_tool::geometry::Port>,
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
pub fn build_route(w: &mut World, db: &crate::model_info::ModelDb, req: &RouteRequest, catalog: &[dayz_road_tool::roads::CatalogPart], job: &Job) -> Result<RoadResult> {
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
            if let Some(start) = req.start {
                let path = routing::find(&terrain, &line, &req.settings, &obstacles, &cancel)?;
                let parts = roads::fit_path_from(start, &path, &req.family, catalog, &req.settings, |_, _| true)?;
                return Ok(Planned { parts, ..Default::default() });
            }
            let bridge = if rail { BridgeKit::for_track(tracks, &req.family) } else { BridgeKit::for_road(&road_folder, &req.family, catalog) };
            routing::plan(&terrain, &line, &req.family, catalog, &req.settings, &obstacles, &crossings.with_bridge(bridge), &cancel)
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result?
    } else {
        // A crossing has to land inside one straight, so a track that puts a joint on a road
        // starts a little later and tries again.
        let attempt = |points: &[[f64; 2]]| -> Result<(Vec<_>, Vec<_>)> {
            let mut parts = match req.start {
                Some(start) => roads::fit_from(start, points, &req.family, catalog, &req.settings, |_, _| true)?,
                None => roads::fit(points, &req.family, catalog, &req.settings, |_, _| true)?,
            };
            let extras = crossings.apply(&mut parts, catalog)?;
            Ok((parts, extras))
        };
        let mut result = attempt(&line);
        let (dx, dz) = (line[1][0] - line[0][0], line[1][1] - line[0][1]);
        let first = dx.hypot(dz);
        for shift in (1..=12).map(|k| k as f64 * 2.0) {
            if result.is_ok() || crossings.is_empty() || req.start.is_some() || shift + 1.0 >= first {
                break;
            }
            let mut later = line.clone();
            later[0] = [line[0][0] + dx / first * shift, line[0][1] + dz / first * shift];
            result = attempt(&later);
        }
        let (parts, extras) = result?;
        // Gorges and water along the drawn line take bridges too, where a kit exists.
        let bridge = if rail { BridgeKit::for_track(tracks, &req.family) } else { BridgeKit::for_road(&road_folder, &req.family, catalog) };
        let bridged = bridge.as_ref().filter(|_| req.start.is_none()).and_then(|kit| {
            let (height, sea) = (&w.height, w.meta.sea_level);
            let (points, spans) = routing::waypoint_spans(&line, |p| height.sample(p[0] as f32, p[1] as f32) as f64, |p| height.sample(p[0] as f32, p[1] as f32) < sea, kit, &req.settings);
            if spans.is_empty() {
                return None;
            }
            let mut planned = routing::fit_waypoints_bridged(&points, &spans, kit, &req.family, catalog, &req.settings).ok()?;
            planned.extras = crossings.apply(&mut planned.parts, catalog).ok()?;
            Some(planned)
        });
        bridged.unwrap_or(Planned { parts, extras, ..Default::default() })
    };
    let route = Route { name: String::new(), family: req.family.clone(), points: line, parts: planned.parts, replaces: None, rail, extras: planned.extras, grading: None, bridges: planned.bridges };
    let stretches: Vec<Vec<[f64; 2]>> = route.stretches().into_iter().map(|s| roads::centerline(s, catalog)).collect();
    let center: Vec<[f64; 2]> = stretches.iter().flatten().copied().collect();
    // A road bridge has one level deck from bank to bank.
    let mut decks = Vec::new();
    let mut deck_of: Vec<f32> = Vec::new();
    for pair in stretches.windows(2) {
        let (Some(a), Some(b)) = (pair[0].last(), pair[1].first()) else { continue };
        let (from, to) = ([a[0] as f32, a[1] as f32], [b[0] as f32, b[1] as f32]);
        if rail {
            decks.push(crate::world::BridgeDeck { from, to, height: 0.0, free: true });
            continue;
        }
        let height = ((w.height.sample(from[0], from[1]) + w.height.sample(to[0], to[1])) * 0.5).max(w.meta.sea_level + 1.0);
        decks.push(crate::world::BridgeDeck { from, to, height, free: false });
        deck_of.push(height);
    }
    // Track is rigid and the search counted on earthworks, so one designed line decides both
    // where the pieces lie and how the terrain is shaped under them.
    let designed = (rail || req.settings.max_cut + req.settings.max_fill > 0.0).then(|| design_heights(&w.base, &stretches, &req.settings, rail)).transpose()?;
    let height = &w.height;
    let ground = |p: [f64; 2]| Some(designed.as_deref().map_or_else(|| height.sample(p[0] as f32, p[1] as f32), |d| height_along(d, p)) as f64);
    let poses = rails::route_poses(&route, catalog, &ground)?;
    let spans = route.bridges.iter().flat_map(|b| &b.pieces);
    let mut objects = Vec::with_capacity(route.parts.len() + route.extras.len());
    let chained = route.parts.len() + route.extras.len();
    let mut level: Vec<Option<(f32, f32)>> = vec![None; chained];
    if !rail {
        for (b, height) in route.bridges.iter().zip(&deck_of) {
            level.extend(b.pieces.iter().map(|_| Some((*height, b.deck as f32))));
        }
    }
    // Rigid pieces are straight between their joints, and the bed under them has to be as well.
    let bed: Option<Vec<[f32; 3]>> = designed.as_ref().filter(|_| rail).map(|_| {
        let mut joints: Vec<[f32; 3]> = Vec::new();
        for (part, pose) in route.parts.iter().zip(&poses) {
            let Some(pose) = pose else { continue };
            for q in roads::centerline(std::slice::from_ref(part), catalog) {
                let z = pose.origin[2] - (pose.up[0] * (q[0] - pose.origin[0]) + pose.up[1] * (q[1] - pose.origin[1])) / pose.up[2].max(0.1);
                joints.push([q[0] as f32, q[1] as f32, z as f32 - RAIL_BED_SINK]);
            }
        }
        let mut dense = Vec::with_capacity(joints.len() * 2);
        for s in joints.windows(2) {
            let (dx, dz) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
            let steps = ((dx * dx + dz * dz).sqrt() / 4.0).ceil().clamp(1.0, 64.0) as usize;
            dense.extend((0..steps).map(|k| { let t = k as f32 / steps as f32; [s[0][0] + dx * t, s[0][1] + dz * t, s[0][2] + (s[1][2] - s[0][2]) * t] }));
        }
        dense.extend(joints.last().copied());
        dense
    });
    for (i, p) in route.parts.iter().chain(&route.extras).chain(spans).enumerate() {
        let model = w.objects.intern(&road_model_path(&p.model, prefix));
        if let Some((height, deck)) = level.get(i).copied().flatten() {
            objects.push(Obj { x: p.position[0] as f32, z: p.position[1] as f32, elevation: height - deck, yaw: p.rotation.rem_euclid(360.0) as f32, pitch: 0.0, roll: 0.0, scale: 1.0, model, flags: crate::objects::FLAG_ABSOLUTE });
            continue;
        }
        let obj = match poses.get(i).copied().flatten() {
            Some(pose) => {
                // Pose axes are (east, north, up); the editor's are (east, up, north).
                let axis = |v: [f64; 3]| [v[0] as f32, v[2] as f32, v[1] as f32];
                let (yaw, pitch, roll, _) = crate::objects::angles(&[axis(pose.right), axis(pose.up), axis(pose.forward)]);
                // On a designed line the pieces stay put when the terrain is shaped under them.
                let (elevation, flags) = if designed.is_some() { (pose.origin[2] as f32, crate::objects::FLAG_ABSOLUTE) } else { (pose.origin[2] as f32 - w.height.sample(pose.origin[0] as f32, pose.origin[1] as f32), 0) };
                // Track geometry is measured from the middle of a model, objects from its own origin.
                let c = db.get(&w.objects.models[model as usize].path).center;
                let m = crate::objects::rotation(yaw, pitch, roll, 1.0);
                let off = |k: usize| m[0][k] * c[0] + m[2][k] * c[2];
                Obj { x: pose.origin[0] as f32 - off(0), z: pose.origin[1] as f32 - off(2), elevation: elevation - off(1), yaw, pitch, roll, scale: 1.0, model, flags }
            }
            None => {
                let (at, yaw) = if i < route.parts.len() { road_part_pose(&route.parts, i) } else { (p.position, p.rotation.rem_euclid(360.0) as f32) };
                Obj { x: at[0] as f32, z: at[1] as f32, elevation: 0.0, yaw, pitch: 0.0, roll: 0.0, scale: 1.0, model, flags: 0 }
            }
        };
        objects.push(obj);
    }
    Ok(RoadResult { objects, centerline: center.iter().map(|p| [p[0] as f32, p[1] as f32]).collect(), decks, profile: bed.or(designed).unwrap_or_default(), parts: route.parts.clone() })
}

/// The point `t` metres along a polyline, held to its ends.
pub fn point_along(line: &[[f32; 2]], t: f32) -> [f32; 2] {
    let mut left = t.max(0.0);
    for s in line.windows(2) {
        let len = (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]);
        if left <= len && len > 0.0 {
            let f = left / len;
            return [s[0][0] + (s[1][0] - s[0][0]) * f, s[0][1] + (s[1][1] - s[0][1]) * f];
        }
        left -= len;
    }
    *line.last().unwrap_or(&[0.0, 0.0])
}

/// The part of a polyline from `a` to `b` metres along it.
pub fn cut_line(line: &[[f32; 2]], a: f32, b: f32) -> Vec<[f32; 2]> {
    let mut out = vec![point_along(line, a)];
    let mut t = 0.0;
    for s in line.windows(2) {
        t += (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]);
        if t > a + 1e-3 && t < b - 1e-3 {
            out.push(s[1]);
        }
    }
    out.push(point_along(line, b));
    out
}

/// Where along a polyline, in metres from its start, other lines cross it.
pub fn line_crossings(line: &[[f32; 2]], others: &[Vec<[f32; 2]>]) -> Vec<f32> {
    let mut out = Vec::new();
    let mut t = 0.0;
    for s in line.windows(2) {
        let (p, r) = (s[0], [s[1][0] - s[0][0], s[1][1] - s[0][1]]);
        let len = r[0].hypot(r[1]);
        for o in others {
            for q in o.windows(2) {
                let d = [q[1][0] - q[0][0], q[1][1] - q[0][1]];
                let den = r[0] * d[1] - r[1] * d[0];
                if den.abs() < 1e-9 { continue; }
                let w = [q[0][0] - p[0], q[0][1] - p[1]];
                let (u, v) = ((w[0] * d[1] - w[1] * d[0]) / den, (w[0] * r[1] - w[1] * r[0]) / den);
                if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) {
                    out.push(t + u * len);
                }
            }
        }
        t += len;
    }
    out.sort_by(f32::total_cmp);
    out.dedup_by(|a, b| (*a - *b).abs() < 1.0);
    out
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

/// A smooth curve through the points of a line, sampled about every 2 m.
fn bezier_through(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let at = |i: isize| points[i.clamp(0, points.len() as isize - 1) as usize];
    let mut out = Vec::new();
    for i in 0..points.len() as isize - 1 {
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let len = ((p2[0] - p1[0]).powi(2) + (p2[1] - p1[1]).powi(2)).sqrt();
        let steps = (len / 2.0).ceil().max(1.0) as usize;
        // The Bezier control points of a Catmull-Rom segment.
        let c1 = [p1[0] + (p2[0] - p0[0]) / 6.0, p1[1] + (p2[1] - p0[1]) / 6.0];
        let c2 = [p2[0] - (p3[0] - p1[0]) / 6.0, p2[1] - (p3[1] - p1[1]) / 6.0];
        for k in 0..steps {
            let t = k as f32 / steps as f32;
            let u = 1.0 - t;
            let b = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            out.push([b[0] * p1[0] + b[1] * c1[0] + b[2] * c2[0] + b[3] * p2[0], b[0] * p1[1] + b[1] * c1[1] + b[2] * c2[1] + b[3] * p2[1]]);
        }
    }
    out.push(*points.last().unwrap());
    out
}

/// The line a generator actually follows: curved if asked, moved sideways by its offset.
pub fn prepare_line(points: &[[f32; 2]], o: &crate::library::LineOptions) -> Vec<[f32; 2]> {
    let line = if o.mode == crate::library::LineMode::Bezier { bezier_through(points) } else { points.to_vec() };
    if o.offset == 0.0 || line.len() < 2 {
        return line;
    }
    let unit = |a: [f32; 2], b: [f32; 2]| { let (x, z) = (b[0] - a[0], b[1] - a[1]); let l = (x * x + z * z).sqrt().max(1e-4); [x / l, z / l] };
    let n = line.len();
    (0..n).map(|i| {
        let before = if i > 0 { unit(line[i - 1], line[i]) } else { unit(line[0], line[1]) };
        let after = if i + 1 < n { unit(line[i], line[i + 1]) } else { before };
        // Right-hand normals, averaged, and lengthened so straight parts keep their distance.
        let (r0, r1) = ([before[1], -before[0]], [after[1], -after[0]]);
        let mut m = [r0[0] + r1[0], r0[1] + r1[1]];
        let l = (m[0] * m[0] + m[1] * m[1]).sqrt().max(1e-4);
        m = [m[0] / l, m[1] / l];
        let k = (m[0] * r0[0] + m[1] * r0[1]).max(0.35);
        [line[i][0] + m[0] * o.offset / k, line[i][1] + m[1] * o.offset / k]
    }).collect()
}

/// Objects at regular steps along a line. Walls and fences have their pieces joined at corners.
pub fn fence_objects(w: &mut World, db: &crate::model_info::ModelDb, points: &[[f32; 2]], g: &crate::library::FenceGen) -> Vec<Obj> {
    use crate::library::LineMode;
    // `across` moves the model's middle onto the line; fence pieces keep their origin on it.
    struct Piece { model: u32, length: f32, along_x: bool, shift: f32, across: f32, weight: f32, skews: bool }
    let piece_along = |w: &mut World, path: &str, weight: f32, axis: Option<bool>| {
        let info = db.get(&crate::objects::ObjectStore::normalize_path(path));
        let (lx, lz) = if info.known { (info.max[0] - info.min[0], info.max[2] - info.min[2]) } else { (0.0, 4.0) };
        let along_x = axis.unwrap_or(lx > lz);
        // Where the model's own middle sits relative to its origin, along its length.
        let (cx, cz) = if info.known { ((info.min[0] + info.max[0]) * 0.5, (info.min[2] + info.max[2]) * 0.5) } else { (0.0, 0.0) };
        let length = if axis.is_some() { if along_x { lx } else { lz } } else { lx.max(lz) };
        let across = if axis.is_some() { if along_x { cz } else { cx } } else { 0.0 };
        Piece { model: w.objects.intern(path), length: length.max(0.5), along_x, shift: if along_x { -cx } else { cz }, across, weight, skews: info.placement.skews() }
    };
    let piece_of = |w: &mut World, path: &str, weight: f32| piece_along(w, path, weight, None);
    let list: Vec<(String, f32)> = if g.resolved.is_empty() { vec![(g.model.clone(), 1.0)] } else { g.resolved.clone() };
    let pieces: Vec<Piece> = list.iter().filter(|m| !m.0.is_empty() && m.1 > 0.0).map(|(m, k)| piece_of(w, m, *k)).collect();
    // Wickets and gates come from the same sets as the fence: they lie along its axis, whatever
    // their own box says (an open gate is deeper than it is wide), and stand centred on the line.
    let axis = pieces.first().map(|p| p.along_x);
    let gate_of = |w: &mut World, path: &str| piece_along(w, path, 1.0, Some(axis.unwrap_or_else(|| {
        let info = db.get(&crate::objects::ObjectStore::normalize_path(path));
        !info.known || info.max[0] - info.min[0] > info.max[2] - info.min[2]
    })));
    let full = prepare_line(points, &g.line);
    let mut out = Vec::new();
    if full.len() < 2 {
        return out;
    }
    // Openings: their pieces, and the stretch of line each leaves free.
    let length: f32 = full.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum();
    let mut openings: Vec<(f32, f32, Vec<(Piece, f32)>, bool, bool)> = Vec::new();
    let mut wanted: Vec<crate::library::FenceOpening> = g.openings.clone();
    if let Some(gate) = &g.road_gate {
        wanted.extend(line_crossings(&full, &g.roads).into_iter().map(|at| crate::library::FenceOpening { at, ..gate.clone() }));
    }
    for o in &wanted {
        // Each object with where its middle lies from the middle of the opening.
        let parts: Vec<(Piece, f32)> = if o.two_leaves {
            let (l, r) = (o.left.trim(), o.right.trim());
            if l.is_empty() && r.is_empty() { continue; }
            let (pl, pr) = (gate_of(w, if l.is_empty() { r } else { l }), gate_of(w, if r.is_empty() { l } else { r }));
            // Each leaf fills its half of the gap: from its post to the middle.
            let width = if o.width > 0.0 { o.width } else { pl.length + pr.length };
            let split = if o.width > 0.0 { width * 0.5 } else { pl.length };
            vec![(pl, -width * 0.5 + split * 0.5), (pr, width * 0.5 - (width - split) * 0.5)]
        } else {
            if o.model.trim().is_empty() { continue; }
            vec![(gate_of(w, o.model.trim()), 0.0)]
        };
        let width = if o.width > 0.0 { o.width } else { parts.iter().map(|p| p.0.length).sum() };
        let at = o.at.clamp(width * 0.5, (length - width * 0.5).max(width * 0.5));
        openings.push((at - width * 0.5, at + width * 0.5, parts, o.flip, o.upright));
    }
    openings.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut stretches: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut from = 0.0f32;
    for (a, b, _, _, _) in &openings {
        if *a > from + 0.05 { stretches.push(cut_line(&full, from, *a)); }
        from = from.max(*b);
    }
    if length > from + 0.05 { stretches.push(cut_line(&full, from, length)); }
    let mut rng = StdRng::seed_from_u64(g.seed);
    let total: f32 = pieces.iter().map(|p| p.weight).sum();
    let choose = |rng: &mut StdRng| -> Option<usize> {
        if pieces.is_empty() {
            return None;
        }
        let mut pick = rng.gen_range(0.0..total);
        for (i, p) in pieces.iter().enumerate() {
            pick -= p.weight;
            if pick <= 0.0 {
                return Some(i);
            }
        }
        Some(0)
    };
    // From one end of a piece to the next, never standing still whatever the overlap.
    let pitch = |p: &Piece| (p.length + g.spacing).max(0.2);
    // One piece, spanning from `a` to `b`.
    fn place(w: &World, g: &crate::library::FenceGen, out: &mut Vec<Obj>, rng: &mut StdRng, p: &Piece, a: [f32; 2], b: [f32; 2]) {
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let span = (dx * dx + dz * dz).sqrt().max(0.01);
        let (ux, uz) = (dx / span, dz / span);
        let heading = ux.atan2(uz).to_degrees();
        let (x, z) = ((a[0] + b[0]) * 0.5 - ux * p.shift - uz * p.across, (a[1] + b[1]) * 0.5 - uz * p.shift + ux * p.across);
        let (ga, gb) = (w.height.sample(a[0], a[1]), w.height.sample(b[0], b[1]));
        // The game fits models with a placement property to the ground by itself.
        let follow = g.follow_slope && !p.skews;
        let tilt = if follow { ((gb - ga) / span).atan().to_degrees() } else { 0.0 };
        let (yaw, pitch, roll) = if p.along_x { (heading + 90.0, 0.0, -tilt) } else { (heading, -tilt, 0.0) };
        let elevation = if follow { (ga + gb) * 0.5 - w.height.sample(x, z) } else { 0.0 };
        let spin = if g.random_rotation { rng.gen_range(0.0..360.0) } else { 0.0 };
        out.push(Obj { x, z, elevation, yaw: yaw + g.yaw_offset + spin, pitch, roll, scale: 1.0, model: p.model, flags: 0 });
    }
    for line in stretches.iter().filter(|l| l.len() >= 2) {
    if g.join_corners && g.line.mode == LineMode::Polyline {
        // Each stretch is divided so its first and last piece end exactly on the corners.
        for s in line.windows(2) {
            let (dx, dz) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
            let len = (dx * dx + dz * dz).sqrt();
            if len < 0.05 { continue; }
            let (ux, uz) = (dx / len, dz / len);
            let mut chosen = Vec::new();
            let mut covered = 0.0;
            while let Some(i) = choose(&mut rng) {
                chosen.push(i);
                covered += pieces[i].length;
                if covered >= len || chosen.len() > 200_000 { break; }
                covered += g.spacing.max(0.2 - pieces[i].length);
            }
            let Some(&last) = chosen.last() else { break };
            if chosen.len() == 1 {
                let middle = [(s[0][0] + s[1][0]) * 0.5, (s[0][1] + s[1][1]) * 0.5];
                let half = pieces[last].length * 0.5;
                place(w, g, &mut out, &mut rng, &pieces[last], [middle[0] - ux * half, middle[1] - uz * half], [middle[0] + ux * half, middle[1] + uz * half]);
                continue;
            }
            let nominal: f32 = chosen[..chosen.len() - 1].iter().map(|i| pitch(&pieces[*i])).sum();
            let squeeze = (len - pieces[last].length) / nominal.max(0.01);
            let mut t = 0.0;
            for i in chosen {
                let p = &pieces[i];
                place(w, g, &mut out, &mut rng, p, [s[0][0] + ux * t, s[0][1] + uz * t], [s[0][0] + ux * (t + p.length), s[0][1] + uz * (t + p.length)]);
                t += pitch(p) * squeeze;
            }
        }
    } else {
        // Each piece is a chord of the line, starting where the last one ended.
        let (mut from, mut i) = (line[0], 0usize);
        let mut guard = 0;
        while let (true, Some(k)) = (i + 1 < line.len() && guard < 400_000, choose(&mut rng)) {
            guard += 1;
            let p = &pieces[k];
            let mut hit = None;
            let mut j = i;
            while j + 1 < line.len() {
                let (a, b) = (line[j], line[j + 1]);
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let (fx, fz) = (a[0] - from[0], a[1] - from[1]);
                let qa = dx * dx + dz * dz;
                let qb = 2.0 * (fx * dx + fz * dz);
                let qc = fx * fx + fz * fz - p.length * p.length;
                let disc = qb * qb - 4.0 * qa * qc;
                if qa > 1e-6 && disc >= 0.0 {
                    let t = (-qb + disc.sqrt()) / (2.0 * qa);
                    if (0.0..=1.0).contains(&t) {
                        hit = Some(([a[0] + dx * t, a[1] + dz * t], j));
                        break;
                    }
                }
                j += 1;
            }
            match hit {
                Some((to, at)) => {
                    place(w, g, &mut out, &mut rng, p, from, to);
                    let (dx, dz) = (to[0] - from[0], to[1] - from[1]);
                    let d = (dx * dx + dz * dz).sqrt().max(1e-3);
                    let gap = g.spacing.max(0.2 - p.length);
                    from = [to[0] + dx / d * gap, to[1] + dz / d * gap];
                    i = at;
                }
                None => {
                    // The last, shorter stretch: one more piece ending on the line's end.
                    let end = *line.last().unwrap();
                    let (dx, dz) = (end[0] - from[0], end[1] - from[1]);
                    let d = (dx * dx + dz * dz).sqrt();
                    if g.join_corners && d > p.length * 0.25 {
                        place(w, g, &mut out, &mut rng, p, [end[0] - dx / d * p.length, end[1] - dz / d * p.length], end);
                    }
                    break;
                }
            }
        }
    }
    }
    // The wickets and gates, in line with the fence.
    // An upright gate stands level on the ground at its middle, however the fence leans.
    let level = crate::library::FenceGen { follow_slope: false, ..g.clone() };
    for (a, b, parts, flip, upright) in &openings {
        let middle = (a + b) * 0.5;
        for (p, offset) in parts {
            let centre = middle + offset;
            let half = p.length * 0.5;
            let (s, e) = (point_along(&full, centre - half), point_along(&full, centre + half));
            let before = out.len();
            place(w, if *upright { &level } else { g }, &mut out, &mut rng, p, s, e);
            if *flip {
                if let Some(o) = out.get_mut(before) { o.yaw += 180.0; }
            }
        }
    }
    let line = full;
    // Objects closing the line, facing along it.
    let ends = [(&g.start_model, line[0], line[1], g.flip_start), (&g.end_model, *line.last().unwrap(), line[line.len() - 2], g.flip_end)];
    for (i, (model, at, next, flip)) in ends.into_iter().enumerate() {
        if model.is_empty() {
            continue;
        }
        let p = piece_of(w, model, 1.0);
        // Both ends face the direction of the line.
        let (dx, dz) = if i == 0 { (next[0] - at[0], next[1] - at[1]) } else { (at[0] - next[0], at[1] - next[1]) };
        let heading = dx.atan2(dz).to_degrees() + if p.along_x { 90.0 } else { 0.0 };
        out.push(Obj { x: at[0], z: at[1], elevation: 0.0, yaw: heading + g.yaw_offset + if flip { 180.0 } else { 0.0 }, pitch: 0.0, roll: 0.0, scale: 1.0, model: p.model, flags: 0 });
    }
    out
}

/// Poles along a line, joined by one wire model per conductor, as Terrain Processor does it.
pub fn power_objects(w: &mut World, db: &crate::model_info::ModelDb, assets: &AssetIndex, points: &[[f32; 2]], g: &crate::library::PowerGen) -> Result<Vec<Obj>> {
    use crate::library::PolePreset;
    let path_of = |name: &str| assets.resolve(name).map(str::to_string).with_context(|| format!("Model {name} not found on P:\\"));
    let straight = g.poles.iter().find(|p| p.kind > 0).or(g.poles.first()).context("No pole in this configuration")?;
    let has = |p: &&PolePreset, incoming: bool, outgoing: bool| p.joints.iter().any(|j| j.incoming == incoming && j.outgoing == outgoing);
    let junction = g.poles.iter().find(|p| p.kind < 0 && p.joints.iter().any(|j| j.outgoing)).unwrap_or(straight);
    let end = g.poles.iter().find(|p| p.kind < 0 && has(p, true, false) && !p.joints.iter().any(|j| j.outgoing)).unwrap_or(straight);
    let plain = g.wires.iter().find(|x| !x.main).or(g.wires.first()).context("No wire in this configuration")?;
    let main = g.wires.iter().find(|x| x.main).unwrap_or(plain);
    let spacing = if g.spacing > 0.0 { g.spacing } else { plain.length.max(5.0) };
    let line = prepare_line(points, &g.line);
    if line.len() < 2 {
        return Ok(Vec::new());
    }
    // A curve is followed by its chords, each a whole number of spans long.
    let mut stations: Vec<([f32; 2], bool)> = vec![(line[0], false)];
    if g.line.mode == crate::library::LineMode::Bezier {
        let mut run = 0.0f32;
        for s in line.windows(2) {
            run += ((s[1][0] - s[0][0]).powi(2) + (s[1][1] - s[0][1]).powi(2)).sqrt();
            if run >= spacing {
                stations.push((s[1], false));
                run = 0.0;
            }
        }
        let last = *line.last().unwrap();
        if stations.last().is_some_and(|s| ((s.0[0] - last[0]).powi(2) + (s.0[1] - last[1]).powi(2)).sqrt() > spacing * 0.3) {
            stations.push((last, false));
        }
    } else {
        for (k, seg) in line.windows(2).enumerate() {
            let (dx, dz) = (seg[1][0] - seg[0][0], seg[1][1] - seg[0][1]);
            let n = ((dx * dx + dz * dz).sqrt() / spacing).round().max(1.0) as usize;
            for i in 1..=n {
                let t = i as f32 / n as f32;
                stations.push(([seg[0][0] + dx * t, seg[0][1] + dz * t], i == n && k + 2 < line.len()));
            }
        }
    }
    let heading = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]).atan2(b[1] - a[1]).to_degrees();
    let mut out = Vec::new();
    struct Placed<'a> { pos: [f32; 3], yaw: f32, preset: &'a PolePreset }
    let mut poles: Vec<Placed> = Vec::with_capacity(stations.len());
    let last = stations.len() - 1;
    for (i, (p, bend)) in stations.iter().enumerate() {
        let before = i.checked_sub(1).map(|k| stations[k].0);
        let after = stations.get(i + 1).map(|s| s.0);
        let along = match (before, after) {
            (Some(a), Some(b)) => { let (h0, h1) = (heading(a, *p), heading(*p, b)); h0 + ((h1 - h0 + 540.0).rem_euclid(360.0) - 180.0) * 0.5 }
            (None, Some(b)) => heading(*p, b),
            (Some(a), None) => heading(a, *p),
            _ => 0.0,
        };
        let is_end = i == 0 || i == last;
        let preset = if is_end { end } else if *bend { junction } else { straight };
        // A pole's model angle is measured from a line running along its X axis. An end pole
        // takes its wires from one side only, so at the start of a line it faces the other way.
        let flip = if i == 0 && std::ptr::eq(preset, end) && !std::ptr::eq(end, straight) { 180.0 } else { 0.0 };
        let yaw = along - 90.0 + preset.model_angle + flip;
        let (s, c) = yaw.to_radians().sin_cos();
        let (ox, oz) = (preset.offset[0], preset.offset[2]);
        let (x, z) = (p[0] + c * ox + s * oz, p[1] - s * ox + c * oz);
        let ground = w.height.sample(x, z);
        let model = w.objects.intern(&path_of(&preset.name)?);
        out.push(Obj { x, z, elevation: preset.offset[1], yaw, pitch: 0.0, roll: 0.0, scale: 1.0, model, flags: 0 });
        poles.push(Placed { pos: [x, ground + preset.offset[1], z], yaw, preset });
    }
    let joint = |p: &Placed, j: &crate::library::WireJoint| {
        let (s, c) = p.yaw.to_radians().sin_cos();
        [p.pos[0] + c * j.at[0] + s * j.at[2], p.pos[1] + j.at[1], p.pos[2] - s * j.at[0] + c * j.at[2]]
    };
    for pair in poles.windows(2) {
        let from: Vec<_> = pair[0].preset.joints.iter().filter(|j| j.outgoing || (j.incoming && !pair[0].preset.joints.iter().any(|k| k.outgoing))).collect();
        let to: Vec<_> = pair[1].preset.joints.iter().filter(|j| j.incoming).collect();
        for (n, (ja, jb)) in from.iter().zip(&to).enumerate() {
            let wire = if n == 0 { main } else { plain };
            let (a, b) = (joint(&pair[0], ja), joint(&pair[1], jb));
            let path = path_of(&wire.name)?;
            let info = db.get(&path);
            let flat = ((b[0] - a[0]).powi(2) + (b[2] - a[2]).powi(2)).sqrt().max(0.1);
            let span = (flat * flat + (b[1] - a[1]).powi(2)).sqrt();
            let scale = span / wire.length.max(1.0);
            let along_x = info.known && info.max[0] - info.min[0] > info.max[2] - info.min[2];
            let tilt = ((b[1] - a[1]) / flat).atan().to_degrees();
            let h = heading([a[0], a[2]], [b[0], b[2]]);
            let (yaw, pitch, roll) = if along_x { (h + 90.0, 0.0, -tilt) } else { (h, -tilt, 0.0) };
            // The wire model may have its origin anywhere along its length.
            let (cx, cz) = if info.known { ((info.min[0] + info.max[0]) * 0.5, (info.min[2] + info.max[2]) * 0.5) } else { (0.0, 0.0) };
            let shift = if along_x { -cx } else { cz } * scale;
            let (ux, uz) = ((b[0] - a[0]) / flat, (b[2] - a[2]) / flat);
            let (x, z) = ((a[0] + b[0]) * 0.5 - ux * shift, (a[2] + b[2]) * 0.5 - uz * shift);
            let elevation = (a[1] + b[1]) * 0.5 + wire.height - w.height.sample(x, z);
            let model = w.objects.intern(&path);
            out.push(Obj { x, z, elevation, yaw, pitch, roll, scale, model, flags: 0 });
        }
    }
    Ok(out)
}

/// A river bed along a line: it only ever runs downhill, `depth` under the bank.
pub fn river_modifier(base: &Heightmap, id: u64, name: &str, line: &[[f32; 2]], g: &crate::library::RiverGen) -> Option<(TerrainModifier, Vec<f32>)> {
    if line.len() < 2 {
        return None;
    }
    let mut points: Vec<[f32; 2]> = Vec::with_capacity(line.len() * 2);
    for w in line.windows(2) {
        let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let steps = ((dx * dx + dz * dz).sqrt() / 6.0).ceil().max(1.0) as usize;
        for k in 0..steps {
            let t = k as f32 / steps as f32;
            points.push([w[0][0] + dx * t, w[0][1] + dz * t]);
        }
    }
    points.push(*line.last().unwrap());
    let mut surface: Vec<f32> = points.iter().map(|p| base.sample(p[0], p[1])).collect();
    for _ in 0..16 {
        let prev = surface.clone();
        for k in 1..prev.len() - 1 {
            surface[k] = (prev[k - 1] + 2.0 * prev[k] + prev[k + 1]) / 4.0;
        }
    }
    for k in 1..surface.len() {
        surface[k] = surface[k].min(surface[k - 1]);
    }
    let heights: Vec<f32> = surface.iter().map(|h| h - g.depth).collect();
    // The whole cut, banks included, stays under the water model.
    let half = g.width * 0.5;
    let blend = g.falloff.clamp(0.0, half * 0.75);
    let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in &points {
        (x0, z0, x1, z1) = (x0.min(p[0]), z0.min(p[1]), x1.max(p[0]), z1.max(p[1]));
    }
    Some((TerrainModifier { id, name: name.to_string(), enabled: true, x: (x0 + x1) * 0.5, z: (z0 + z1) * 0.5, yaw: 0.0, shape: ModShape::Line { points, heights, half_width: half - blend, gaps: Vec::new(), bank: None }, mode: ModMode::Flatten, target: None, amount: 0.0, falloff: blend, attached: None, follow: false }, surface))
}

/// Water models along a river bed, at a fixed height so they do not follow the carved ground.
pub fn river_objects(w: &mut World, db: &crate::model_info::ModelDb, modifier: &TerrainModifier, surface: &[f32], g: &crate::library::RiverGen) -> Vec<Obj> {
    let ModShape::Line { points, .. } = &modifier.shape else { return Vec::new() };
    if g.model.is_empty() {
        return Vec::new();
    }
    let info = db.get(&crate::objects::ObjectStore::normalize_path(&g.model));
    let (lx, lz) = if info.known { (info.max[0] - info.min[0], info.max[2] - info.min[2]) } else { (24.0, 24.0) };
    let length = lx.max(lz).max(2.0);
    let width = lx.min(lz).max(1.0);
    let model = w.objects.intern(&g.model);
    let scale = g.width / width;
    let step = (length * scale - g.join_offset).max(1.0);
    let (mut out, mut run, mut next) = (Vec::new(), 0.0f32, 0.0f32);
    for k in 0..points.len() - 1 {
        let (a, b) = (points[k], points[k + 1]);
        let seg = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt().max(0.01);
        while next <= run + seg {
            let t = (next - run) / seg;
            let (x, z) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
            // The water stands a little under the banks.
            let h = surface[k] + (surface[k + 1] - surface[k]) * t - g.depth * 0.4 + g.water_offset;
            let heading = (b[0] - a[0]).atan2(b[1] - a[1]).to_degrees();
            let tilt = ((surface[k + 1] - surface[k]) / seg).clamp(-0.5, 0.5).asin().to_degrees();
            out.push(Obj { x, z, elevation: h, yaw: heading, pitch: -tilt, roll: 0.0, scale, model, flags: crate::objects::FLAG_ABSOLUTE });
            next += step;
        }
        run += seg;
    }
    out
}

/// The water models of the game, with their two open ends as `[x, z, outward x, outward z]`.
const WATER_PIECES: &[(&str, [f32; 4], [f32; 4])] = &[
    ("river_24_6", [0.000, -3.136, 0.0, -1.0], [0.000, 2.894, 0.0, 1.0]),
    ("river_24_12", [0.000, -5.787, 0.0, -1.0], [0.000, 6.223, 0.0, 1.0]),
    ("river_24_24", [0.000, -12.000, 0.0, -1.0], [0.000, 12.000, 0.0, 1.0]),
    ("river_24_24_L10", [-0.097, -4.395, 0.0960, -0.9954], [0.223, 5.408, 0.1883, 0.9821]),
    ("river_24_24_L30", [-2.160, -10.841, 0.0960, -0.9954], [1.610, 8.992, 0.5235, 0.8520]),
    ("river_24_24_R10", [-0.345, -4.567, -0.0960, -0.9954], [-0.665, 5.236, -0.1883, 0.9821]),
    ("river_24_24_R30", [0.000, -10.841, -0.0960, -0.9954], [-3.770, 8.992, -0.5235, 0.8520]),
    ("stream_14_6", [0.000, -3.136, 0.0, -1.0], [0.000, 2.894, 0.0, 1.0]),
    ("stream_14_12", [0.000, -5.787, 0.0, -1.0], [0.000, 6.223, 0.0, 1.0]),
    ("stream_14_24", [0.000, -12.000, 0.0, -1.0], [0.000, 12.000, 0.0, 1.0]),
    ("stream_14_16_L45", [0.002, -9.273, 0.0, -1.0], [-6.293, 7.055, -0.6815, 0.7318]),
    ("stream_14_16_L90", [1.796, -8.780, 0.0, -1.0], [-9.507, 2.552, -1.0, 0.0028]),
    ("stream_14_16_R45", [-1.119, -9.273, 0.0, -1.0], [5.176, 7.055, 0.6815, 0.7318]),
    ("stream_14_16_R90", [-1.639, -8.823, 0.0, -1.0], [9.671, 2.515, 1.0, 0.0028]),
    ("stream_14_24_L10", [-0.120, -5.727, -0.0992, -0.9951], [-0.412, 6.305, -0.1830, 0.9831]),
    ("stream_14_24_L30", [-0.148, -11.496, -0.0992, -0.9951], [-5.166, 11.779, -0.4694, 0.8830]),
    ("stream_14_24_L45", [-0.001, -15.096, 0.0, -1.0], [-9.878, 10.714, -0.6815, 0.7318]),
    ("stream_14_24_R10", [-0.015, -5.703, 0.0992, -0.9951], [0.435, 6.695, 0.1754, 0.9845]),
    ("stream_14_24_R30", [-0.765, -11.498, 0.0992, -0.9951], [4.258, 11.759, 0.4871, 0.8734]),
    ("stream_14_24_R45", [-1.577, -15.096, 0.0, -1.0], [8.300, 10.714, 0.6815, 0.7318]),
];

/// One water model of a river, joined end to end with the next.
#[derive(Clone, Debug)]
pub struct RiverPiece {
    pub model: String,
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    pub from: [f32; 2],
    pub to: [f32; 2],
}

/// Water models following a line, and the line they really make.
#[derive(Clone, Debug)]
pub struct RiverChain {
    pub pieces: Vec<RiverPiece>,
    pub center: Vec<[f32; 2]>,
    pub scale: f32,
}

struct RiverFit<'a> {
    line: &'a [[f32; 2]],
    run: Vec<f32>,
    options: Vec<(usize, [f32; 4], [f32; 4])>,
    scale: f32,
    overlap: f32,
}

impl RiverFit<'_> {
    /// Where the next model starts after one that ended at `exit`.
    fn next_start(&self, exit: [f32; 2], out: [f32; 2]) -> [f32; 2] {
        [exit[0] - out[0] * self.overlap, exit[1] - out[1] * self.overlap]
    }

    /// Whether a model got far enough along the line to be worth placing.
    fn advances(&self, reached: f32, s: f32, chord: f32) -> bool {
        reached >= s + ((chord - self.overlap) * 0.4).max(1.0)
    }

    /// Distance to the line near `from` along it, the place reached and the way the line goes.
    fn near(&self, p: [f32; 2], from: f32) -> (f32, f32, [f32; 2]) {
        let start = self.run.partition_point(|r| *r < from - 8.0).saturating_sub(1);
        let mut best = (f32::MAX, from, [0.0, 1.0]);
        for k in start..self.line.len() - 1 {
            if self.run[k] > from + 90.0 * self.scale.max(1.0) {
                break;
            }
            let (a, b) = (self.line[k], self.line[k + 1]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dz * dz).sqrt().max(1e-4);
            let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / (len * len)).clamp(0.0, 1.0);
            let d = (a[0] + dx * t - p[0]).hypot(a[1] + dz * t - p[1]);
            if d < best.0 {
                best = (d, self.run[k] + len * t, [dx / len, dz / len]);
            }
        }
        best
    }

    /// Where a model entered by `a` and left by `b` stands: origin, yaw, exit and heading.
    fn place(&self, at: [f32; 2], heading: [f32; 2], a: &[f32; 4], b: &[f32; 4]) -> ([f32; 2], f32, [f32; 2], [f32; 2]) {
        let yaw = (-heading[0]).atan2(-heading[1]) - a[2].atan2(a[3]);
        let (s, c) = yaw.sin_cos();
        let turn = |x: f32, z: f32| [c * x + s * z, -s * x + c * z];
        let (pa, pb) = (turn(a[0], a[1]), turn(b[0], b[1]));
        let origin = [at[0] - pa[0] * self.scale, at[1] - pa[1] * self.scale];
        (origin, yaw, [origin[0] + pb[0] * self.scale, origin[1] + pb[1] * self.scale], turn(b[2], b[3]))
    }

    fn cost_ahead(&self, at: [f32; 2], heading: [f32; 2], s: f32, depth: u32, sum: f32, len: f32, count: f32) -> f32 {
        let total = *self.run.last().unwrap();
        let mut best = f32::MAX;
        if depth > 0 && s < total {
            for (_, a, b) in &self.options {
                let (_, _, exit, out) = self.place(at, heading, a, b);
                let (d, reached, _) = self.near(exit, s);
                let chord = (exit[0] - at[0]).hypot(exit[1] - at[1]);
                if self.advances(reached, s, chord) {
                    best = best.min(self.cost_ahead(self.next_start(exit, out), out, reached, depth - 1, sum + d * d * chord, len + chord, count + 1.0));
                }
            }
        }
        if best < f32::MAX {
            return best;
        }
        let way = self.near(at, s).2;
        let off = (heading[0] * way[1] - heading[1] * way[0]).atan2(heading[0] * way[0] + heading[1] * way[1]);
        sum / len.max(1.0) + (off * 10.0).powi(2) + 16.0 * count / len.max(1.0)
    }
}

/// Joins water models end to end along a line, bends included. `None` for a model that is
/// not one of the game's rivers or streams.
pub fn fit_river(line: &[[f32; 2]], g: &crate::library::RiverGen) -> Option<RiverChain> {
    if line.len() < 2 || g.model.is_empty() {
        return None;
    }
    let path = g.model.replace('/', "\\");
    let (folder, file) = path.rsplit_once('\\').unwrap_or(("", &path));
    let file = file.to_ascii_lowercase();
    let (family, native) = if file.starts_with("river_24") { ("river_24", 24.0) } else if file.starts_with("stream_14") { ("stream_14", 14.0) } else { return None };
    let mut options = Vec::new();
    // Every model has a current, so all of them are entered from the same end.
    for (k, (_, a, b)) in WATER_PIECES.iter().enumerate().filter(|(_, p)| p.0.starts_with(family)) {
        options.push(if g.reverse_flow { (k, *b, *a) } else { (k, *a, *b) });
    }
    let mut run = vec![0.0f32];
    for w in line.windows(2) {
        run.push(run.last().unwrap() + (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]));
    }
    let total = *run.last().unwrap();
    let scale = (g.width / native).max(0.05);
    let fit = RiverFit { line, run, options, scale, overlap: g.join_offset };
    let (mut at, mut s) = (line[0], 0.0f32);
    let first = fit.near(at, 0.0).2;
    let mut heading = first;
    let mut pieces = Vec::new();
    let mut center = vec![at];
    while s < total - 3.0 * scale && pieces.len() < 40_000 {
        let mut best: Option<(f32, usize, [f32; 2], f32, [f32; 2], [f32; 2], f32)> = None;
        for (k, a, b) in &fit.options {
            let (origin, yaw, exit, out) = fit.place(at, heading, a, b);
            let (d, reached, _) = fit.near(exit, s);
            let chord = (exit[0] - at[0]).hypot(exit[1] - at[1]);
            if !fit.advances(reached, s, chord) {
                continue;
            }
            let cost = fit.cost_ahead(fit.next_start(exit, out), out, reached, 2, d * d * chord, chord, 1.0);
            if best.as_ref().is_none_or(|b| cost < b.0) {
                best = Some((cost, *k, origin, yaw, exit, out, reached));
            }
        }
        let Some((_, k, origin, yaw, exit, out, reached)) = best else { break };
        pieces.push(RiverPiece { model: if folder.is_empty() { format!("{}.p3d", WATER_PIECES[k].0) } else { format!("{folder}\\{}.p3d", WATER_PIECES[k].0) }, x: origin[0], z: origin[1], yaw: yaw.to_degrees().rem_euclid(360.0), from: at, to: exit });
        let chord = (exit[0] - at[0]).hypot(exit[1] - at[1]);
        for q in 1..4 {
            // Along the curve of the piece, which a straight chord would cut.
            let t = q as f32 / 4.0;
            let (h00, h10, h01, h11) = (2.0 * t * t * t - 3.0 * t * t + 1.0, t * t * t - 2.0 * t * t + t, -2.0 * t * t * t + 3.0 * t * t, t * t * t - t * t);
            let point = [0, 1].map(|i| h00 * at[i] + h10 * chord * heading[i] + h01 * exit[i] + h11 * chord * out[i]);
            let last = *center.last().unwrap();
            // The part reaching back over the model before must not fold the line.
            if (point[0] - last[0]) * (exit[0] - at[0]) + (point[1] - last[1]) * (exit[1] - at[1]) > 0.25 * chord {
                center.push(point);
            }
        }
        center.push(exit);
        at = fit.next_start(exit, out);
        heading = out;
        s = reached;
    }
    (!pieces.is_empty()).then_some(RiverChain { pieces, center, scale })
}

/// The water models of a fitted river, at the level of its bed.
pub fn river_chain_objects(w: &mut World, modifier: &TerrainModifier, surface: &[f32], g: &crate::library::RiverGen, chain: &RiverChain) -> Vec<Obj> {
    let ModShape::Line { points, .. } = &modifier.shape else { return Vec::new() };
    let level = |p: [f32; 2]| {
        let k = (0..points.len()).min_by(|a, b| (points[*a][0] - p[0]).hypot(points[*a][1] - p[1]).total_cmp(&(points[*b][0] - p[0]).hypot(points[*b][1] - p[1]))).unwrap_or(0);
        surface[k]
    };
    chain.pieces.iter().map(|p| {
        let (a, b) = (level(p.from), level(p.to));
        let (s, c) = p.yaw.to_radians().sin_cos();
        // How far along the model's own length its two ends are from its origin.
        let along = |q: [f32; 2]| s * (q[0] - p.x) + c * (q[1] - p.z);
        let (from, to) = (along(p.from), along(p.to));
        // Only tipped along the current, so that its ends meet those of its neighbours.
        let slope = if (to - from).abs() > 1.0 { ((b - a) / (to - from)).clamp(-0.5, 0.5) } else { 0.0 };
        let elevation = a - slope * from - g.depth * 0.4 + g.water_offset;
        Obj { x: p.x, z: p.z, elevation, yaw: p.yaw, pitch: -slope.asin().to_degrees(), roll: 0.0, scale: chain.scale, model: w.objects.intern(&p.model), flags: crate::objects::FLAG_ABSOLUTE }
    }).collect()
}

/// Which objects a removing generator takes: `items` are (x, z, model).
pub fn remove_hits(items: &[(f32, f32, u32)], models: &[String], g: &crate::library::RemoveGen, area: Option<&crate::shapes::PolygonIndex>, lines: &[Vec<[f32; 2]>]) -> Vec<bool> {
    let list = |s: &str| s.split(',').map(|n| n.trim().to_ascii_lowercase()).filter(|n| !n.is_empty()).collect::<Vec<_>>();
    let (names, keep) = (list(&g.names), list(&g.keep));
    let wanted: Vec<bool> = models.iter().map(|m| {
        let p = m.to_ascii_lowercase();
        let file = p.rsplit('\\').next().unwrap_or(&p);
        if keep.iter().any(|n| p.contains(n.as_str())) {
            return false;
        }
        if !names.is_empty() {
            return names.iter().any(|n| p.contains(n.as_str()));
        }
        if p.contains("\\tree\\") || file.starts_with("t_") {
            g.trees
        } else if p.contains("\\bush\\") || file.starts_with("b_") {
            g.bushes
        } else if p.contains("\\rock") || file.starts_with("rock") || file.starts_with("stone") {
            g.rocks
        } else if p.contains("\\structures") {
            g.buildings
        } else {
            g.other
        }
    }).collect();
    // A line has no inside, so something has to count as being on it.
    let reach = if area.is_some() { g.margin } else { g.margin.max(1.0) };
    let boxes: Vec<[f32; 4]> = lines.iter().map(|l| l.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0] - reach), b[1].min(p[1] - reach), b[2].max(p[0] + reach), b[3].max(p[1] + reach)])).collect();
    let near = |x: f32, z: f32| lines.iter().zip(&boxes).any(|(l, b)| {
        x >= b[0] && z >= b[1] && x <= b[2] && z <= b[3] && l.windows(2).any(|w| {
            let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let t = (((x - w[0][0]) * dx + (z - w[0][1]) * dz) / (dx * dx + dz * dz).max(1e-4)).clamp(0.0, 1.0);
            (w[0][0] + dx * t - x).hypot(w[0][1] + dz * t - z) <= reach
        })
    });
    let everywhere = area.is_none() && lines.is_empty();
    items.par_iter().map(|(x, z, model)| {
        if !wanted.get(*model as usize).copied().unwrap_or(false) {
            return false;
        }
        let dice = (x.to_bits() ^ z.to_bits().rotate_left(13) ^ g.seed).wrapping_mul(2_654_435_761) as f32 / u32::MAX as f32;
        dice <= g.share && (everywhere || area.is_some_and(|a| a.contains(*x, *z)) || (reach > 0.0 && near(*x, *z)))
    }).collect()
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

/// Height of the water of a pond: it finds the level of the lowest place on its shore.
fn pond_level(base: &Heightmap, shape: &crate::shapes::MapShape, g: &crate::library::PondGen) -> f32 {
    let ring: Vec<[f32; 2]> = shape.points.iter().chain(shape.points.first()).copied().collect();
    let shore = ring.windows(2).flat_map(|s| {
        let steps = ((s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]) / base.cell).ceil().max(1.0) as usize;
        (0..steps).map(move |k| { let t = k as f32 / steps as f32; [s[0][0] + (s[1][0] - s[0][0]) * t, s[0][1] + (s[1][1] - s[0][1]) * t] })
    }).map(|p| base.sample(p[0], p[1])).fold(f32::MAX, f32::min);
    (if g.fixed { g.level } else { shore }) + g.offset
}

/// The basins of the polygons, for a generator that shapes the terrain under its ponds.
pub fn pond_modifier(base: &Heightmap, id: u64, name: &str, polygons: &[crate::shapes::MapShape], g: &crate::library::PondGen) -> Option<TerrainModifier> {
    let areas: Vec<crate::world::PondArea> = polygons.iter().filter(|s| s.points.len() >= 3).map(|s| crate::world::PondArea { points: s.points.clone(), level: pond_level(base, s, g) }).collect();
    let b = areas.iter().flat_map(|a| &a.points).fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
    (!areas.is_empty()).then(|| TerrainModifier { id, name: name.to_string(), enabled: true, x: (b[0] + b[2]) * 0.5, z: (b[1] + b[3]) * 0.5, yaw: 0.0, shape: ModShape::Pond { areas, depth: g.depth, blend: g.blend, bank: g.bank }, mode: ModMode::Flatten, target: None, amount: 0.0, falloff: g.blend, attached: None, follow: false })
}

/// Water models side by side, level, over the whole of each polygon.
pub fn pond_objects(w: &mut World, db: &crate::model_info::ModelDb, polygons: &[crate::shapes::MapShape], g: &crate::library::PondGen) -> Vec<Obj> {
    let path = crate::objects::ObjectStore::normalize_path(&g.model);
    let info = db.get(&path);
    let side = if info.known { (info.max[0] - info.min[0]).max(info.max[2] - info.min[2]).max(1.0) } else { 50.0 };
    let (top, cx, cz) = if info.known { (info.max[1], (info.min[0] + info.max[0]) * 0.5, (info.min[2] + info.max[2]) * 0.5) } else { (0.0, 0.0, 0.0) };
    let model = w.objects.intern(&path);
    let mut out = Vec::new();
    for shape in polygons.iter().filter(|s| s.points.len() >= 3) {
        let level = pond_level(&w.base, shape, g);
        let index = crate::shapes::PolygonIndex::new(&crate::shapes::ShapeLayer { name: String::new(), visible: true, shapes: vec![shape.clone()] });
        let b = shape.points.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
        let (cols, rows) = (((b[2] - b[0]) / side).ceil().max(1.0) as usize, ((b[3] - b[1]) / side).ceil().max(1.0) as usize);
        for j in 0..rows {
            for i in 0..cols {
                let (x0, z0) = (b[0] + i as f32 * side, b[1] + j as f32 * side);
                let inside = (0..=4).any(|u| (0..=4).any(|v| index.contains(x0 + side * u as f32 * 0.25, z0 + side * v as f32 * 0.25)));
                let vertex = shape.points.iter().any(|p| p[0] >= x0 && p[0] <= x0 + side && p[1] >= z0 && p[1] <= z0 + side);
                if inside || vertex {
                    out.push(Obj { x: x0 + side * 0.5 - cx, z: z0 + side * 0.5 - cz, elevation: level - top, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model, flags: crate::objects::FLAG_ABSOLUTE });
                }
            }
        }
    }
    out
}

/// Blocks of a level crossing laid over the track `piece` where a road crosses at `at`. Track
/// geometry runs through the middle of a model; objects are placed by their own origin.
fn deck_objects(piece: &Obj, piece_center: [f32; 3], origin_y: f32, at: [f32; 2], blocks: &[(String, f64, f64)], block_center: impl Fn(&str) -> [f32; 3]) -> Vec<(String, Obj)> {
    let m = crate::objects::rotation(piece.yaw, piece.pitch, piece.roll, 1.0);
    let off = |c: [f32; 3], k: usize| m[0][k] * c[0] + m[2][k] * c[2];
    let middle = [piece.x + off(piece_center, 0), origin_y + off(piece_center, 1), piece.z + off(piece_center, 2)];
    let flat = m[2][0].hypot(m[2][2]).max(1e-4);
    let (fx, fz) = (m[2][0] / flat, m[2][2] / flat);
    let total: f32 = blocks.iter().map(|b| b.1 as f32).sum();
    let mut from = -total * 0.5;
    let mut out = Vec::with_capacity(blocks.len());
    for (path, length, offset) in blocks {
        let shift = from + *length as f32 * 0.5 - *offset as f32;
        let (x, z) = (at[0] + fx * shift, at[1] + fz * shift);
        let y = middle[1] + m[2][1] / flat * ((x - middle[0]) * fx + (z - middle[2]) * fz);
        let model = road_model_path(path, "");
        let c = block_center(&model);
        out.push((model, Obj { x: x - off(c, 0), z: z - off(c, 2), elevation: y - off(c, 1), yaw: piece.yaw, pitch: piece.pitch, roll: piece.roll, scale: 1.0, model: 0, flags: crate::objects::FLAG_ABSOLUTE }));
        from += *length as f32;
    }
    out
}

/// Level crossings for a new road over the tracks already on the map, and how many crossings
/// could not get one because the track bends there or the road is too oblique.
pub fn decks_over_tracks(w: &World, db: &crate::model_info::ModelDb, center: &[[f32; 2]], road_decks: &[crate::world::BridgeDeck]) -> (Vec<(String, Obj)>, usize) {
    use dayz_road_tool::rails;
    const HALF_WIDTH: f32 = 4.0;
    const MARGIN: f32 = 0.5;
    let (mut out, mut skipped) = (Vec::new(), 0);
    for track in w.routes.iter().filter(|r| rails::is_track_family(&r.family)) {
        let line: Vec<[f32; 3]> = track.centerline.iter().map(|p| [p[0], p[1], 0.0]).collect();
        let spans: Vec<_> = road_decks.iter().chain(&track.decks).cloned().collect();
        for hit in crossing_decks(center, &line, &[], 0.0, &spans) {
            let at = hit.from;
            let mid = |o: &Obj| {
                let c = db.get(w.objects.model_path(o)).center;
                let m = crate::objects::rotation(o.yaw, o.pitch, o.roll, 1.0);
                [o.x + m[0][0] * c[0] + m[2][0] * c[2], o.z + m[0][2] * c[0] + m[2][2] * c[2]]
            };
            let nearest = track.objects.iter().filter_map(|i| w.objects.get(*i)).min_by(|a, b| {
                let d = |o: &Obj| { let p = mid(o); (p[0] - at[0]).hypot(p[1] - at[1]) };
                d(a).total_cmp(&d(b))
            });
            let Some(piece) = nearest else { continue };
            let path = w.objects.model_path(piece);
            let stem = path.rsplit('\\').next().unwrap_or(path).trim_end_matches(".p3d");
            if stem.contains("passing") {
                continue; // The track was built with a crossing here.
            }
            let road = center.windows(2).min_by(|a, b| {
                let d = |s: &[[f32; 2]]| ((s[0][0] + s[1][0]) * 0.5 - at[0]).hypot((s[0][1] + s[1][1]) * 0.5 - at[1]);
                d(a).total_cmp(&d(b))
            });
            let m = crate::objects::rotation(piece.yaw, piece.pitch, piece.roll, 1.0);
            let sin = road.map_or(0.0, |s| {
                let (rx, rz) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
                ((m[2][0] * rz - m[2][2] * rx) / (rx.hypot(rz) * m[2][0].hypot(m[2][2])).max(1e-4)).abs()
            });
            let blocks = (rails::is_straight_piece(stem) && sin >= (rails::MIN_CROSSING_ANGLE as f32).to_radians().sin() - 1e-3)
                .then(|| rails::deck_blocks(&track.family, Path::new(rails::DEFAULT_FOLDER), f64::from(2.0 * HALF_WIDTH / sin + 2.0 * MARGIN)))
                .flatten();
            let Some(blocks) = blocks else { skipped += 1; continue };
            let (center_of, ground) = (db.get(path).center, crate::objects::ObjectStore::origin_y(piece, &w.height));
            out.extend(deck_objects(piece, center_of, ground, at, &blocks, |p| db.get(p).center));
        }
    }
    (out, skipped)
}

/// Where `other`, given as (x, z, height), crosses `line`, as points the road has to meet.
/// Segments in `gaps` are spans in the air, and spans of the road itself pass over anything.
pub fn crossing_decks(line: &[[f32; 2]], other: &[[f32; 3]], gaps: &[u32], lift: f32, own: &[crate::world::BridgeDeck]) -> Vec<crate::world::BridgeDeck> {
    let mut out = Vec::new();
    for w in line.windows(2) {
        for (_, s) in other.windows(2).enumerate().filter(|(k, _)| !gaps.contains(&(*k as u32))) {
            let (rx, rz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let (sx, sz) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
            let cross = rx * sz - rz * sx;
            if cross.abs() < 1e-6 {
                continue;
            }
            let (qx, qz) = (s[0][0] - w[0][0], s[0][1] - w[0][1]);
            let (t, u) = ((qx * sz - qz * sx) / cross, (qx * rz - qz * rx) / cross);
            if !(0.0..1.0).contains(&t) || !(0.0..1.0).contains(&u) {
                continue;
            }
            let at = [w[0][0] + rx * t, w[0][1] + rz * t];
            let on_own_span = own.iter().any(|d| {
                let (dx, dz) = (d.to[0] - d.from[0], d.to[1] - d.from[1]);
                let v = ((at[0] - d.from[0]) * dx + (at[1] - d.from[1]) * dz) / (dx * dx + dz * dz).max(1e-3);
                (0.0..=1.0).contains(&v) && (d.from[0] + dx * v - at[0]).hypot(d.from[1] + dz * v - at[1]) < 2.0
            });
            if !on_own_span {
                out.push(crate::world::BridgeDeck { from: at, to: at, height: s[0][2] + (s[1][2] - s[0][2]) * u + lift, free: false });
            }
        }
    }
    out
}

/// A modifier that levels a strip along a road to a smooth profile of the sculpted terrain.
pub fn road_modifier(base: &Heightmap, id: u64, name: &str, center: &[[f32; 2]], width: f32, falloff: f32, decks: &[crate::world::BridgeDeck], grading: &crate::world::RoadGrading, profile: &[[f32; 3]]) -> Option<TerrainModifier> {
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
    // A pass spreads a height over about 8 m; the spread grows with the root of the passes.
    let passes = (2.0 * (grading.smooth / 8.0).powi(2)).round().clamp(0.0, 600.0) as usize;
    for _ in 0..passes {
        let prev = heights.clone();
        for k in 1..prev.len() - 1 {
            heights[k] = (prev[k - 1] + 2.0 * prev[k] + prev[k + 1]) / 4.0;
        }
    }
    if grading.max_grade > 0.0 {
        let rise = grading.max_grade / 100.0;
        let run = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt() * rise;
        for k in 1..heights.len() {
            let d = run(points[k - 1], points[k]);
            heights[k] = heights[k].clamp(heights[k - 1] - d, heights[k - 1] + d);
        }
        for k in (0..heights.len() - 1).rev() {
            let d = run(points[k + 1], points[k]);
            heights[k] = heights[k].clamp(heights[k + 1] - d, heights[k + 1] + d);
        }
    }
    // A designed line replaces all of that.
    if profile.len() >= 2 {
        points = profile.iter().map(|p| [p[0], p[1]]).collect();
        heights = profile.iter().map(|p| p[2]).collect();
    }
    // The road rises to meet each bridge and the ground under the span stays as it is.
    let mut gaps = Vec::new();
    for d in decks {
        let on_span = |p: [f32; 2]| {
            let (dx, dz) = (d.to[0] - d.from[0], d.to[1] - d.from[1]);
            let t = ((p[0] - d.from[0]) * dx + (p[1] - d.from[1]) * dz) / (dx * dx + dz * dz).max(1e-3);
            t > 0.0 && t < 1.0 && ((d.from[0] + dx * t - p[0]).powi(2) + (d.from[1] + dz * t - p[1]).powi(2)).sqrt() < 2.0
        };
        for (k, s) in points.windows(2).enumerate() {
            if on_span([(s[0][0] + s[1][0]) * 0.5, (s[0][1] + s[1][1]) * 0.5]) {
                gaps.push(k as u32);
            }
        }
        if d.free {
            continue;
        }
        let ground = (base.sample(d.from[0], d.from[1]) - d.height).abs().max((base.sample(d.to[0], d.to[1]) - d.height).abs());
        let ramp = (ground / 0.07).clamp(25.0, 150.0);
        for (p, h) in points.iter().zip(&mut heights) {
            let near = ((p[0] - d.from[0]).powi(2) + (p[1] - d.from[1]).powi(2)).sqrt().min(((p[0] - d.to[0]).powi(2) + (p[1] - d.to[1]).powi(2)).sqrt());
            if near < ramp {
                let t = 1.0 - near / ramp;
                *h += (d.height - *h) * (t * t * (3.0 - 2.0 * t));
            }
        }
    }
    let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in &points {
        (x0, z0, x1, z1) = (x0.min(p[0]), z0.min(p[1]), x1.max(p[0]), z1.max(p[1]));
    }
    // With banks the sides are measured by slope, and the blend only covers how far they may go.
    let bank = grading.embankment.then(|| crate::world::LineBank {
        shoulder: grading.shoulder,
        fill: grading.fill_slope.to_radians().tan(),
        cut: grading.cut_slope.to_radians().tan(),
        ditch_depth: grading.ditch_depth,
        ditch_width: grading.ditch_width,
    });
    let half_width = width * 0.5 + if grading.embankment { 0.0 } else { grading.shoulder };
    let ditch = if grading.ditch_depth > 0.0 { grading.ditch_width } else { 0.0 };
    let outside = if grading.embankment { grading.shoulder + ditch + grading.max_bank } else { falloff };
    Some(TerrainModifier { id, name: name.to_string(), enabled: true, x: (x0 + x1) * 0.5, z: (z0 + z1) * 0.5, yaw: 0.0, shape: ModShape::Line { points, heights, half_width, gaps, bank }, mode: ModMode::Flatten, target: None, amount: grading.raise, falloff: outside, attached: None, follow: false })
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
    fn fence_pieces_meet_the_corners_exactly() {
        use crate::library::FenceGen;
        let mut w = World::new(crate::world::WorldMeta::new("A", 256, 8.0), 10.0);
        let db = crate::model_info::ModelDb::new("Z:/nothing");
        let g = FenceGen { model: "dz\\wall.p3d".into(), follow_slope: false, ..Default::default() };
        let out = fence_objects(&mut w, &db, &[[100.0, 100.0], [100.0, 111.0], [110.0, 111.0]], &g);
        assert_eq!(out.len(), 3 + 3);
        // First piece starts on the first corner, last of the stretch ends on the second.
        assert!((out[0].z - 102.0).abs() < 0.01 && (out[2].z - 109.0).abs() < 0.01, "{} {}", out[0].z, out[2].z);
    }

    #[test]
    fn offset_moves_a_line_to_its_right_side() {
        use crate::library::{LineMode, LineOptions};
        let l = prepare_line(&[[0.0, 0.0], [0.0, 100.0]], &LineOptions { mode: LineMode::Polyline, offset: 5.0 });
        assert!((l[0][0] - 5.0).abs() < 1e-3 && (l[1][0] - 5.0).abs() < 1e-3);
    }

    #[test]
    fn bezier_passes_through_the_points() {
        use crate::library::{LineMode, LineOptions};
        let pts = [[0.0, 0.0], [40.0, 10.0], [80.0, 0.0]];
        let l = prepare_line(&pts, &LineOptions { mode: LineMode::Bezier, offset: 0.0 });
        assert!(l.len() > 20);
        assert!(l.iter().any(|p| (p[0] - 40.0).abs() < 0.01 && (p[1] - 10.0).abs() < 0.01));
        assert_eq!(*l.last().unwrap(), [80.0, 0.0]);
    }

    #[test]
    fn a_river_follows_a_bend_with_bend_models() {
        use crate::library::RiverGen;
        let line: Vec<[f32; 2]> = (0..=60).map(|k| { let a = k as f32 / 60.0 * 1.57; [300.0 - 150.0 * a.cos(), 300.0 + 150.0 * a.sin()] }).collect();
        let g = RiverGen { join_offset: 0.0, ..Default::default() };
        let chain = fit_river(&line, &g).unwrap();
        assert!(chain.pieces.iter().any(|p| p.model.contains("_L") || p.model.contains("_R")));
        assert!(chain.pieces.iter().all(|p| p.model.starts_with("dz\\water\\streams\\river_24")));
        for w in chain.pieces.windows(2) {
            assert!((w[0].to[0] - w[1].from[0]).abs() < 1e-3 && (w[0].to[1] - w[1].from[1]).abs() < 1e-3);
        }
        let worst = chain.pieces.iter().map(|p| line.iter().map(|q| (q[0] - p.to[0]).hypot(q[1] - p.to[1])).fold(f32::MAX, f32::min)).fold(0.0, f32::max);
        assert!(worst < 9.0, "{worst}");
        let end = chain.pieces.last().unwrap().to;
        assert!((end[0] - line[60][0]).hypot(end[1] - line[60][1]) < 30.0);
    }

    #[test]
    fn each_water_model_reaches_back_over_the_one_before() {
        use crate::library::RiverGen;
        let line = [[100.0, 100.0], [100.0, 500.0]];
        let chain = fit_river(&line, &RiverGen { join_offset: 3.0, ..Default::default() }).unwrap();
        for w in chain.pieces.windows(2) {
            assert!(((w[0].to[0] - w[1].from[0]).hypot(w[0].to[1] - w[1].from[1]) - 3.0).abs() < 0.01);
        }
        assert!(chain.center.windows(2).all(|w| w[1][1] > w[0][1]));
    }

    #[test]
    fn a_later_forest_layer_keeps_its_distance() {
        let at = |x: f32| Obj { x, z: 0.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        let kept = keep_apart(&[at(10.0)], vec![at(10.5), at(11.5), at(30.0)], 1.0);
        assert_eq!(kept.iter().map(|o| o.x).collect::<Vec<_>>(), [11.5, 30.0]);
    }

    #[test]
    fn a_river_bed_stays_within_its_water_model() {
        use crate::library::RiverGen;
        let h = Heightmap::new(64, 64, 8.0, 50.0);
        for g in [RiverGen::default(), RiverGen { width: 14.0, falloff: 14.0, ..Default::default() }] {
            let (m, _) = river_modifier(&h, 1, "r", &[[10.0, 160.0], [400.0, 160.0]], &g).unwrap();
            let ModShape::Line { half_width, .. } = m.shape else { panic!() };
            assert!(half_width > 0.0 && half_width + m.falloff <= g.width * 0.5 + 1e-3);
        }
    }

    #[test]
    fn removing_takes_only_what_was_asked() {
        use crate::library::RemoveGen;
        let models = ["dz\\plants\\tree\\t_a.p3d", "dz\\plants\\bush\\b_a.p3d", "dz\\rocks\\stone.p3d", "dz\\structures\\house.p3d"].map(String::from);
        let items = [(10.0, 0.0, 0), (10.0, 1.0, 1), (10.0, 2.0, 2), (10.0, 3.0, 3), (50.0, 0.0, 0)];
        let line = vec![vec![[0.0, 0.0], [20.0, 0.0]]];
        let g = RemoveGen { margin: 5.0, ..Default::default() };
        assert_eq!(remove_hits(&items, &models, &g, None, &line), [true, true, false, false, false]);
        let names = RemoveGen { margin: 5.0, names: "house".into(), ..Default::default() };
        assert_eq!(remove_hits(&items, &models, &names, None, &line), [false, false, false, true, false]);
        let kept = RemoveGen { margin: 5.0, keep: "b_a".into(), ..Default::default() };
        assert_eq!(remove_hits(&items, &models, &kept, None, &line), [true, false, false, false, false]);
        assert_eq!(remove_hits(&items, &models, &RemoveGen::default(), None, &[]), [true, true, false, false, true]);
    }

    #[test]
    fn only_the_closing_cap_of_a_road_is_turned() {
        use dayz_road_tool::roads::PlacedPart;
        let part = |m: &str, z: f64| PlacedPart { model: m.into(), reverse: false, position: [10.0, z], rotation: 0.0 };
        let road = [part("asf2_6konec.p3d", 0.0), part("asf2_25.p3d", 6.25), part("asf2_6konec.p3d", 31.25)];
        assert_eq!(road_part_pose(&road, 0), ([10.0, 0.0], 0.0));
        assert_eq!(road_part_pose(&road, 1), ([10.0, 6.25], 0.0));
        // Its outer end is the end of the road and it looks back along it.
        assert_eq!(road_part_pose(&road, 2), ([10.0, 37.5], 180.0));
    }

    #[test]
    fn every_water_model_runs_the_same_way_and_follows_the_slope() {
        use crate::library::RiverGen;
        let line: Vec<[f32; 2]> = (0..=60).map(|k| { let a = k as f32 / 60.0 * 3.0; [300.0 - 150.0 * a.cos(), 300.0 + 150.0 * a.sin()] }).collect();
        let g = RiverGen::default();
        let chain = fit_river(&line, &g).unwrap();
        // The way from the entry to the exit of a model is always towards its own +Z.
        for p in &chain.pieces {
            let (s, c) = p.yaw.to_radians().sin_cos();
            assert!(s * (p.to[0] - p.from[0]) + c * (p.to[1] - p.from[1]) > 0.0, "{p:?}");
        }
        let flipped = fit_river(&line, &RiverGen { reverse_flow: true, ..Default::default() }).unwrap();
        for p in &flipped.pieces {
            let (s, c) = p.yaw.to_radians().sin_cos();
            assert!(s * (p.to[0] - p.from[0]) + c * (p.to[1] - p.from[1]) < 0.0, "{p:?}");
        }
        let mut h = Heightmap::new(128, 128, 8.0, 50.0);
        for j in 0..128 { for i in 0..128 { h.set(i, j, 150.0 - i as f32 * 0.5); } }
        let (m, surface) = river_modifier(&h, 1, "r", &chain.center, &g).unwrap();
        let mut w = World::new(crate::world::WorldMeta::new("t", 128, 8.0), 50.0);
        let objects = river_chain_objects(&mut w, &m, &surface, &g, &chain);
        assert!(objects.iter().all(|o| o.roll == 0.0));
        assert!(objects.iter().any(|o| o.pitch.abs() > 0.05), "no model follows the slope");
        // Where one model ends the next one starts, at the same height.
        let height = |o: &Obj, q: [f32; 2]| {
            let (s, c) = o.yaw.to_radians().sin_cos();
            o.elevation - o.pitch.to_radians().sin() * (s * (q[0] - o.x) + c * (q[1] - o.z))
        };
        for k in 0..objects.len() - 1 {
            let (end, start) = (height(&objects[k], chain.pieces[k].to), height(&objects[k + 1], chain.pieces[k].to));
            assert!((end - start).abs() < 0.1, "{k}: {end} {start}");
        }
    }

    #[test]
    fn ponds_cover_the_whole_shape_side_by_side_at_one_level() {
        use crate::shapes::{MapShape, ShapeKind};
        let mut w = World::new(crate::world::WorldMeta::new("t", 128, 8.0), 30.0);
        for j in 0..128 { for i in 0..128 { w.base.set(i, j, 30.0 + i as f32 * 0.1); } }
        let db = crate::model_info::ModelDb::new("nowhere");
        let shape = MapShape { kind: ShapeKind::Polygon, points: vec![[100.0, 100.0], [220.0, 100.0], [220.0, 190.0], [100.0, 190.0]], attributes: Vec::new() };
        let g = crate::library::PondGen::default();
        let objects = pond_objects(&mut w, &db, &[shape.clone()], &g);
        assert_eq!(objects.len(), 3 * 2, "120 x 90 m is three by two 50 m models");
        // Tiles touch: 50 m apart, and the first is 25 m in from the corner of the shape.
        assert!(objects.iter().any(|o| (o.x - 125.0).abs() < 0.01 && (o.z - 125.0).abs() < 0.01));
        assert!(objects.iter().any(|o| (o.x - 175.0).abs() < 0.01 && (o.z - 125.0).abs() < 0.01));
        let shore = w.base.sample(100.0, 100.0);
        assert!(objects.iter().all(|o| (o.elevation - shore).abs() < 0.02 && o.flags == crate::objects::FLAG_ABSOLUTE), "{:?}", objects[0]);
        let fixed = pond_objects(&mut w, &db, &[shape], &crate::library::PondGen { fixed: true, level: 12.0, offset: 1.0, ..Default::default() });
        assert!(fixed.iter().all(|o| o.elevation == 13.0));
    }

    #[test]
    fn a_deck_lies_on_the_track_under_the_road() {
        // A track running north and climbing 1 in 10, with its middle 10 m ahead of its origin.
        let pitch = -(0.1f32).asin().to_degrees();
        let piece = Obj { x: 50.0, z: 100.0, elevation: 20.0, yaw: 0.0, pitch, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        let blocks = [(r"P:\dz\misc\block.p3d".to_string(), 10.0, 1.0)];
        let decks = deck_objects(&piece, [0.0, 0.0, 10.0], 20.0, [50.0, 112.0], &blocks, |_| [0.0, 0.0, 2.0]);
        let (path, o) = &decks[0];
        assert_eq!(path, r"dz\misc\block.p3d");
        // Block origin: the road's point, minus the offset of the block, minus its own centre.
        assert!((o.x - 50.0).abs() < 1e-3 && (o.z - (112.0 - 1.0 - 2.0 * pitch.to_radians().cos())).abs() < 0.01, "{o:?}");
        let track_there = 20.0 + 0.1 * (112.0 - 1.0 - 100.0);
        assert!((o.elevation + 0.1 * 2.0 - track_there).abs() < 0.03, "{o:?}");
        assert_eq!((o.yaw, o.pitch, o.flags), (0.0, pitch, crate::objects::FLAG_ABSOLUTE));
    }

    #[test]
    fn a_road_meets_what_it_crosses_except_under_a_span() {
        let road = [[0.0, 50.0], [100.0, 50.0]];
        let other = [[40.0, 0.0, 10.0], [40.0, 100.0, 20.0]];
        let d = crossing_decks(&road, &other, &[], 0.5, &[]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].from, [40.0, 50.0]);
        assert!((d[0].height - 15.5).abs() < 1e-3);
        assert!(crossing_decks(&road, &other, &[0], 0.5, &[]).is_empty());
        let span = crate::world::BridgeDeck { from: [30.0, 50.0], to: [50.0, 50.0], height: 30.0, free: false };
        assert!(crossing_decks(&road, &other, &[], 0.5, &[span]).is_empty());
    }

    #[test]
    fn river_only_runs_downhill() {
        use crate::library::RiverGen;
        let mut h = Heightmap::new(64, 64, 8.0, 50.0);
        h.set(20, 20, 90.0);
        let g = RiverGen::default();
        let (m, surface) = river_modifier(&h, 1, "r", &[[10.0, 160.0], [400.0, 160.0]], &g).unwrap();
        assert!(surface.windows(2).all(|s| s[1] <= s[0]));
        let ModShape::Line { heights, .. } = m.shape else { panic!() };
        assert!((heights[0] - (surface[0] - g.depth)).abs() < 1e-3);
    }

    #[test]
    fn a_designed_line_decides_the_heights_of_the_modifier() {
        let base = Heightmap::new(64, 64, 4.0, 10.0);
        let line = [[20.0, 100.0], [200.0, 100.0]];
        let profile = [[20.0, 100.0, 14.0], [110.0, 100.0, 16.0], [200.0, 100.0, 18.0]];
        let m = road_modifier(&base, 1, "r", &line, 6.0, 8.0, &[], &Default::default(), &profile).unwrap();
        let ModShape::Line { heights, .. } = m.shape else { panic!() };
        assert_eq!(heights, vec![14.0, 16.0, 18.0]);
        assert!((height_along(&profile, [65.0, 120.0]) - 15.0).abs() < 1e-3);
    }

    #[test]
    fn embankments_and_cuttings_follow_their_slopes() {
        use crate::world::{RoadGrading, WorldMeta};
        // The ground falls towards the north, across a road running east.
        let mut base = Heightmap::new(128, 128, 4.0, 0.0);
        for j in 0..128 {
            for i in 0..128 {
                base.set(i, j, 200.0 - j as f32 * 4.0 * 0.5);
            }
        }
        let level = base.sample(200.0, 256.0);
        let line = [[40.0, 256.0], [460.0, 256.0]];
        let shaped = |g: RoadGrading| {
            let m = road_modifier(&base, 1, "r", &line, 9.0, 8.0, &[], &g, &[]).unwrap();
            World::from_parts(WorldMeta::new("t", 128, 4.0), base.clone(), crate::terrain::MaskMap::new(64, 64, 0), Vec::new(), None, Default::default(), vec![m])
        };
        let g = RoadGrading::default();
        let w = shaped(g);
        assert!((w.height.sample(200.0, 256.0) - level).abs() < 0.01);
        assert!((w.height.sample(200.0, 260.0) - level).abs() < 0.01, "bed");
        let run = 8.0 - 4.5 - g.shoulder;
        assert!((w.height.sample(200.0, 264.0) - (level - g.fill_slope.to_radians().tan() * run)).abs() < 0.02, "embankment");
        assert!((w.height.sample(200.0, 248.0) - (level + g.cut_slope.to_radians().tan() * run)).abs() < 0.02, "cutting");
        // Far enough away the ground is left as it was.
        assert_eq!(w.height.sample(200.0, 320.0), base.sample(200.0, 320.0));
        let raised = shaped(RoadGrading { raise: 2.0, ..g });
        assert!((raised.height.sample(200.0, 256.0) - level - 2.0).abs() < 0.01);
        // On the uphill side a ditch runs beside the road.
        let ditch = shaped(RoadGrading { ditch_depth: 1.0, ditch_width: 4.0, shoulder: 1.5, ..g });
        assert!((ditch.height.sample(200.0, 248.0) - (level - 1.0)).abs() < 0.02, "ditch");
    }

    #[test]
    fn fences_overlap_and_get_end_objects() {
        use crate::library::FenceGen;
        let mut w = world(10.0);
        let db = crate::model_info::ModelDb::new(std::path::PathBuf::from("."));
        let line = [[100.0, 100.0], [100.0, 140.0]];
        let plain = fence_objects(&mut w, &db, &line, &FenceGen::default()).len();
        let close = fence_objects(&mut w, &db, &line, &FenceGen { spacing: -2.0, ..Default::default() }).len();
        assert!(close > plain, "{close} {plain}");
        let capped = fence_objects(&mut w, &db, &line, &FenceGen { start_model: "a.p3d".into(), end_model: "b.p3d".into(), flip_end: true, ..Default::default() });
        assert_eq!(capped.len(), plain + 2);
        let (first, last) = (capped[capped.len() - 2], capped[capped.len() - 1]);
        assert_eq!([first.x, first.z], line[0]);
        assert_eq!([last.x, last.z], line[1]);
        assert!(((last.yaw - first.yaw).rem_euclid(360.0) - 180.0).abs() < 0.01);
    }

    #[test]
    fn a_fence_leaves_room_for_its_wicket_and_gates() {
        use crate::library::FenceGen;
        let mut w = World::new(crate::world::WorldMeta::new("F", 64, 4.0), 10.0);
        let db = crate::model_info::ModelDb::new("Z:/none");
        // Unknown models are 4 m long.
        let line = [[0.0, 50.0], [100.0, 50.0]];
        let plain = fence_objects(&mut w, &db, &line, &FenceGen { model: "wall.p3d".into(), ..Default::default() });
        let wicket = crate::library::FenceOpening { at: 30.0, model: "wicket.p3d".into(), ..Default::default() };
        let gate = crate::library::FenceOpening { at: 70.0, two_leaves: true, left: "gate_l.p3d".into(), right: "gate_r.p3d".into(), ..Default::default() };
        let g = FenceGen { model: "wall.p3d".into(), openings: vec![wicket, gate], ..Default::default() };
        let objects = fence_objects(&mut w, &db, &line, &g);
        let named = |o: &Obj| w.objects.models[o.model as usize].path.clone();
        let (walls, others): (Vec<&Obj>, Vec<&Obj>) = objects.iter().partition(|o| named(o) == "wall.p3d");
        assert_eq!(others.len(), 3, "a wicket and two leaves");
        assert!(walls.len() < plain.len());
        // No wall stands in a gap: the wicket spans 28..32 and the gate 66..74.
        assert!(walls.iter().all(|o| !(28.5..31.5).contains(&o.x) && !(66.5..73.5).contains(&o.x)), "{:?}", walls.iter().map(|o| o.x).collect::<Vec<_>>());
        let at = |name: &str| others.iter().find(|o| named(o) == name).unwrap().x;
        assert!((at("wicket.p3d") - 30.0).abs() < 0.01);
        assert!((at("gate_l.p3d") - 68.0).abs() < 0.01 && (at("gate_r.p3d") - 72.0).abs() < 0.01);
        // A gate one object wide where a road crosses.
        // Leaves as wide as the whole gap still each take their half of it.
        let g = FenceGen { model: "wall.p3d".into(), openings: vec![crate::library::FenceOpening { at: 50.0, two_leaves: true, left: "l.p3d".into(), right: "r.p3d".into(), width: 10.0, ..Default::default() }], ..Default::default() };
        let mut w2 = World::new(crate::world::WorldMeta::new("F", 64, 4.0), 10.0);
        let objects = fence_objects(&mut w2, &db, &line, &g);
        let leaf = |n: &str| objects.iter().find(|o| w2.objects.models[o.model as usize].path == n).unwrap().x;
        assert!((leaf("l.p3d") - 47.5).abs() < 0.01 && (leaf("r.p3d") - 52.5).abs() < 0.01, "{} {}", leaf("l.p3d"), leaf("r.p3d"));
        let g = FenceGen { model: "wall.p3d".into(), road_gate: Some(crate::library::FenceOpening { model: "gate.p3d".into(), width: 8.0, ..Default::default() }), roads: vec![vec![[40.0, 0.0], [40.0, 100.0]]], ..Default::default() };
        let mut w = World::new(crate::world::WorldMeta::new("F", 64, 4.0), 10.0);
        let objects = fence_objects(&mut w, &db, &line, &g);
        let gates: Vec<&Obj> = objects.iter().filter(|o| w.objects.models[o.model as usize].path == "gate.p3d").collect();
        assert_eq!(gates.len(), 1);
        assert!((gates[0].x - 40.0).abs() < 0.01);
        assert_eq!(line_crossings(&line, &[vec![[40.0, 0.0], [40.0, 100.0]], vec![[90.0, 0.0], [90.0, 10.0]]]), vec![40.0]);
        // On a slope the fence leans with the ground and an upright gate does not.
        let mut hill = World::new(crate::world::WorldMeta::new("S", 64, 4.0), 10.0);
        for (k, v) in hill.base.data.iter_mut().enumerate() { *v = 10.0 + (k % 64) as f32 * 0.8; }
        hill.recompose(None);
        let g = FenceGen { model: "wall.p3d".into(), openings: vec![crate::library::FenceOpening { at: 50.0, model: "gate.p3d".into(), ..Default::default() }], ..Default::default() };
        let objects = fence_objects(&mut hill, &db, &line, &g);
        let tilt = |n: &str| objects.iter().find(|o| hill.objects.models[o.model as usize].path == n).map(|o| o.pitch.abs() + o.roll.abs()).unwrap();
        assert!(tilt("wall.p3d") > 1.0 && tilt("gate.p3d") < 1e-3, "{} {}", tilt("wall.p3d"), tilt("gate.p3d"));
        let g = FenceGen { openings: vec![crate::library::FenceOpening { upright: false, ..g.openings[0].clone() }], ..g };
        let objects = fence_objects(&mut hill, &db, &line, &g);
        assert!(objects.iter().filter(|o| hill.objects.models[o.model as usize].path == "gate.p3d").all(|o| o.pitch.abs() + o.roll.abs() > 1.0));
    }

    #[test]
    fn a_bridge_keeps_the_ground_under_it_and_ramps_the_road() {
        let h = Heightmap::new(64, 64, 8.0, 20.0);
        let line = [[0.0, 100.0], [100.0, 100.0], [200.0, 100.0], [300.0, 100.0]];
        let deck = crate::world::BridgeDeck { from: [100.0, 100.0], to: [200.0, 100.0], height: 26.0, free: false };
        let m = road_modifier(&h, 1, "r", &line, 9.0, 8.0, &[deck], &crate::world::RoadGrading::default(), &[]).unwrap();
        let ModShape::Line { points, heights, gaps, .. } = m.shape else { panic!() };
        assert!(!gaps.is_empty());
        let at = |x: f32| heights[points.iter().position(|p| (p[0] - x).abs() < 0.1).unwrap()];
        assert!((at(100.0) - 26.0).abs() < 0.2 && (at(200.0) - 26.0).abs() < 0.2);
        assert!(at(300.0) < 24.0);
    }

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

    /// A road drawn straight across a gorge gets a bridge, with or without earthworks, when P: is mounted.
    #[test]
    fn a_straight_road_bridges_a_gorge() {
        let parts = Path::new("P:/dz/structures/roads/parts");
        if !parts.exists() { return; }
        let catalog = road_catalog(parts).unwrap();
        let Some(family) = dayz_road_tool::rails::BridgeKit::for_road(parts, "asf2", &catalog).map(|_| "asf2".to_string()).or_else(|| road_families(&catalog).into_iter().find(|f| dayz_road_tool::rails::BridgeKit::for_road(parts, f, &catalog).is_some())) else { return };
        let mut w = World::new(crate::world::WorldMeta::new("B", 128, 8.0), 100.0);
        for j in 0..w.base.rows {
            for i in 0..w.base.cols {
                let x = i as f32 * w.base.cell;
                if (480.0..540.0).contains(&x) { let c = w.base.cols; w.base.data[j * c + i] = 88.0; }
            }
        }
        w.recompose(None);
        for (cut, fill) in [(0.0, 0.0), (5.0, 6.0)] {
            let settings = dayz_road_tool::document::RoutingSettings { max_cut: cut, max_fill: fill, ..Default::default() };
            let req = RouteRequest { family: family.clone(), points: vec![[200.0, 500.0], [800.0, 500.0]], terrain_route: false, settings, forbidden: vec![], roads: vec![], start: None };
            let r = build_route(&mut w, &crate::model_info::ModelDb::new("Z:/none"), &req, &catalog, &Job::default()).unwrap();
            assert!(!r.decks.is_empty(), "no bridge with cut {cut} and fill {fill}");
            let d = r.decks[0];
            assert!(d.from[0] < 482.0 && d.to[0] > 538.0 && d.height > 95.0, "{d:?}");
        }
    }

    /// A road goes on from either open end: the new pieces join the old ones in one chain,
    /// with an end piece only at the new end, when P: is mounted.
    #[test]
    fn a_road_goes_on_from_either_end_without_refitting() {
        use dayz_road_tool::roads::{endpoint, flip, is_cap, startpoint};
        let parts = Path::new("P:/dz/structures/roads/parts");
        if !parts.exists() { return; }
        let catalog = road_catalog(parts).unwrap();
        let family = if road_families(&catalog).iter().any(|f| f == "asf2") { "asf2".to_string() } else { road_families(&catalog)[0].clone() };
        let mut w = World::new(crate::world::WorldMeta::new("E", 128, 8.0), 100.0);
        let db = crate::model_info::ModelDb::new("Z:/none");
        let req = |points: Vec<[f32; 2]>, start| RouteRequest { family: family.clone(), points, terrain_route: false, settings: Default::default(), forbidden: vec![], roads: vec![], start };
        let first = build_route(&mut w, &db, &req(vec![[300.0, 500.0], [500.0, 500.0]], None), &catalog, &Job::default()).unwrap();
        assert!(is_cap(first.parts.last().unwrap(), &catalog) && is_cap(&first.parts[0], &catalog));
        let joined = |chain: &[dayz_road_tool::roads::PlacedPart]| {
            for k in 0..chain.len() - 1 {
                let (out, next) = (endpoint(&chain[..=k], &catalog).unwrap(), startpoint(&chain[k + 1..], &catalog).unwrap());
                let gap = (out.p[0] - next.p[0]).hypot(out.p[1] - next.p[1]);
                let facing = out.outward[0] * next.outward[0] + out.outward[1] * next.outward[1];
                assert!(gap < 0.05 && facing < -0.999, "joint {k}: gap {gap} facing {facing}");
            }
        };
        // On from the end.
        let mut kept = first.parts.clone();
        kept.pop();
        let port = endpoint(&kept, &catalog).unwrap();
        let more = build_route(&mut w, &db, &req(vec![[port.p[0] as f32, port.p[1] as f32], [700.0, 540.0]], Some(port)), &catalog, &Job::default()).unwrap();
        assert!(!is_cap(&more.parts[0], &catalog) && is_cap(more.parts.last().unwrap(), &catalog));
        let chain: Vec<_> = kept.iter().cloned().chain(more.parts.iter().cloned()).collect();
        joined(&chain);
        // On from the start.
        let kept: Vec<_> = chain[1..].to_vec();
        let port = startpoint(&kept, &catalog).unwrap();
        let before = build_route(&mut w, &db, &req(vec![[port.p[0] as f32, port.p[1] as f32], [100.0, 460.0]], Some(port)), &catalog, &Job::default()).unwrap();
        let chain: Vec<_> = flip(&before.parts).into_iter().chain(kept).collect();
        joined(&chain);
        assert!(is_cap(&chain[0], &catalog) && is_cap(chain.last().unwrap(), &catalog));
        assert_eq!(chain.iter().filter(|p| is_cap(p, &catalog)).count(), 2, "end pieces only at the two ends");
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
        let mut req = RouteRequest { family: "asf2".into(), points: vec![[2200.0, 6500.0], [2500.0, 6300.0]], terrain_route: true, settings: Default::default(), forbidden: vec![], roads: vec![], start: None };
        let r = build_route(&mut w, &crate::model_info::ModelDb::new("Z:/none"), &req, &catalog, &Job::default()).unwrap();
        println!("catalog {} parts {} in {:?}, line {}", catalog.len(), r.objects.len(), t0.elapsed(), r.centerline.len());
        assert!(!r.objects.is_empty());
        // A track over the same ground is laid on it piece by piece.
        let rails = rail_catalog();
        assert!(!rails.is_empty());
        req.family = "Rail_TrackE".into();
        req.settings = dayz_road_tool::document::RoutingSettings::rail();
        let t = build_route(&mut w, &crate::model_info::ModelDb::new("Z:/none"), &req, &rails, &Job::default()).unwrap();
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
    fn a_shape_keeps_the_generator_inside_and_fades_at_its_edge() {
        use crate::shapes::{MapShape, PolygonIndex, ShapeKind, ShapeLayer};
        let original = Heightmap::new(128, 128, 10.0, 50.0);
        let mut result = original.clone();
        let frame = [300.0, 300.0, 900.0, 900.0];
        generate_terrain_in(&mut result, &TerrainParams { kind: TerrainKind::Mountains, base: 400.0, ..Default::default() }, Some(frame), &Job::default());
        assert_eq!(result.at(5, 5), 50.0, "nothing is generated outside the frame");
        let square = MapShape { kind: ShapeKind::Polygon, points: vec![[300.0, 300.0], [900.0, 300.0], [900.0, 900.0], [300.0, 900.0]], attributes: Vec::new() };
        let area = PolygonIndex::new(&ShapeLayer { name: String::new(), visible: true, shapes: vec![square] });
        blend_in_area(&original, &mut result, &area, 100.0);
        assert_eq!(result.at(20, 20), 50.0, "outside the shape the terrain stays");
        assert!(result.at(60, 60) > 200.0, "in the middle the mountains stand: {}", result.at(60, 60));
        // Just inside the edge the result is still close to what was there.
        assert!((result.at(31, 60) - 50.0).abs() < (result.at(60, 60) - 50.0) * 0.2, "{}", result.at(31, 60));
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
