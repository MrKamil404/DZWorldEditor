//! What the shape of the terrain says about a place: where water runs and gathers, hollows
//! and ridges, and how far it is to water and to roads. Generators use it in their rules, and
//! the view shows where water flows.
//!
//! Everything is worked out on a grid no finer than the heightmap and at most `MAX_SIDE`
//! cells a side, which is plenty for rules that decide where forests or surfaces go.
use crate::terrain::Heightmap;
use crate::world::{ModShape, World};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// Most cells a side of the grid the maps are worked out on.
pub const MAX_SIDE: usize = 2048;

/// Water and roads of a map, gathered from the whole world (generators work on a copy that has
/// no objects or routes).
#[derive(Clone, Debug, Default)]
pub struct Landmarks {
    /// Centre lines of roads.
    pub roads: Vec<Vec<[f32; 2]>>,
    /// Points of river and pond models, and the outlines of ponds.
    pub water_points: Vec<[f32; 2]>,
    pub ponds: Vec<Vec<[f32; 2]>>,
}

impl Landmarks {
    pub fn of(w: &World) -> Self {
        let mut out = Landmarks { roads: w.routes.iter().map(|r| r.centerline.clone()).filter(|l| l.len() > 1).collect(), ..Default::default() };
        for (_, o) in w.objects.iter() {
            let path = w.objects.model_path(o);
            if crate::objects::is_water_model(path) {
                out.water_points.push([o.x, o.z]);
            }
        }
        for m in &w.modifiers {
            if let ModShape::Pond { areas, .. } = &m.shape {
                out.ponds.extend(areas.iter().map(|a| a.points.clone()));
            }
        }
        out
    }
}

/// Which maps a rule needs, so that only those are worked out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Needs {
    pub hollow: bool,
    pub wetness: bool,
    pub water: bool,
    pub roads: bool,
}

impl Needs {
    pub fn any(&self) -> bool {
        self.hollow || self.wetness || self.water || self.roads
    }
    pub fn or(self, o: Needs) -> Needs {
        Needs { hollow: self.hollow || o.hollow, wetness: self.wetness || o.wetness, water: self.water || o.water, roads: self.roads || o.roads }
    }
}

/// Maps of the terrain on a grid over the whole map, row 0 on the south edge as the heightmap.
#[derive(Clone, Debug, Default)]
pub struct TerrainMaps {
    pub cols: usize,
    pub rows: usize,
    /// Metres a cell.
    pub cell: f32,
    /// How far the ground lies below the ground round it, in metres: above 0 in hollows and
    /// valleys, below 0 on ridges and crests.
    pub hollow: Vec<f32>,
    /// Where water gathers, 0 dry to 1 where the most water of the map flows.
    pub wetness: Vec<f32>,
    /// Metres to the nearest water and road.
    pub water_dist: Vec<f32>,
    pub road_dist: Vec<f32>,
}

impl TerrainMaps {
    /// The maps `needs` asks for. `hollow_radius` is how far round a place the ground is
    /// compared with, in metres.
    pub fn new(w: &World, marks: &Landmarks, needs: Needs, hollow_radius: f32) -> Self {
        let h = downsample(&w.height, MAX_SIDE);
        let (cols, rows, cell) = (h.cols, h.rows, h.cell);
        let mut m = TerrainMaps { cols, rows, cell, ..Default::default() };
        let sea = w.meta.sea_level;
        if needs.hollow {
            m.hollow = hollow(&h, hollow_radius);
        }
        if needs.wetness {
            m.wetness = wetness(&h, sea);
        }
        if needs.water {
            let mut seeds: Vec<bool> = h.data.iter().map(|v| *v < sea).collect();
            let mut mark = |x: f32, z: f32, r: f32| {
                let rc = (r / cell).ceil() as i64;
                let (ci, cj) = ((x / cell) as i64, (z / cell) as i64);
                for j in cj - rc..=cj + rc {
                    for i in ci - rc..=ci + rc {
                        if i >= 0 && j >= 0 && (i as usize) < cols && (j as usize) < rows {
                            seeds[j as usize * cols + i as usize] = true;
                        }
                    }
                }
            };
            for p in &marks.water_points {
                mark(p[0], p[1], 6.0);
            }
            for ring in &marks.ponds {
                let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for p in ring {
                    (x0, z0, x1, z1) = (x0.min(p[0]), z0.min(p[1]), x1.max(p[0]), z1.max(p[1]));
                }
                let (i0, j0) = (((x0 / cell).floor().max(0.0)) as usize, ((z0 / cell).floor().max(0.0)) as usize);
                let (i1, j1) = (((x1 / cell).ceil() as usize).min(cols - 1), ((z1 / cell).ceil() as usize).min(rows - 1));
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        if crate::shapes::point_in_polygon(ring, (i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell) {
                            seeds[j * cols + i] = true;
                        }
                    }
                }
            }
            m.water_dist = distance_field(&seeds, cols, rows, cell);
        }
        if needs.roads {
            let mut seeds = vec![false; cols * rows];
            for line in &marks.roads {
                for s in line.windows(2) {
                    let len = ((s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]) / (cell * 0.5)).ceil().max(1.0) as usize;
                    for k in 0..=len {
                        let t = k as f32 / len as f32;
                        let (x, z) = (s[0][0] + (s[1][0] - s[0][0]) * t, s[0][1] + (s[1][1] - s[0][1]) * t);
                        let (i, j) = ((x / cell) as i64, (z / cell) as i64);
                        if i >= 0 && j >= 0 && (i as usize) < cols && (j as usize) < rows {
                            seeds[j as usize * cols + i as usize] = true;
                        }
                    }
                }
            }
            m.road_dist = distance_field(&seeds, cols, rows, cell);
        }
        m
    }

    /// Only the distance to the sea, worked out from a heightmap.
    pub fn sea(h: &Heightmap, sea: f32) -> Self {
        let h = downsample(h, MAX_SIDE);
        let seeds: Vec<bool> = h.data.iter().map(|v| *v < sea).collect();
        TerrainMaps { cols: h.cols, rows: h.rows, cell: h.cell, water_dist: distance_field(&seeds, h.cols, h.rows, h.cell), ..Default::default() }
    }

    #[inline]
    fn sample(&self, v: &[f32], x: f32, z: f32, missing: f32) -> f32 {
        if v.is_empty() {
            return missing;
        }
        let i = ((x / self.cell) as i64).clamp(0, self.cols as i64 - 1) as usize;
        let j = ((z / self.cell) as i64).clamp(0, self.rows as i64 - 1) as usize;
        v[j * self.cols + i]
    }
    pub fn hollow_at(&self, x: f32, z: f32) -> f32 {
        self.sample(&self.hollow, x, z, 0.0)
    }
    pub fn wetness_at(&self, x: f32, z: f32) -> f32 {
        self.sample(&self.wetness, x, z, 0.0)
    }
    pub fn water_dist_at(&self, x: f32, z: f32) -> f32 {
        self.sample(&self.water_dist, x, z, f32::MAX)
    }
    pub fn road_dist_at(&self, x: f32, z: f32) -> f32 {
        self.sample(&self.road_dist, x, z, f32::MAX)
    }
}

/// The heightmap with at most `max_side` vertices a side, each the average of those it covers.
pub fn downsample(h: &Heightmap, max_side: usize) -> Heightmap {
    let f = h.cols.max(h.rows).div_ceil(max_side.max(1)).max(1);
    if f == 1 {
        return h.clone();
    }
    let (cols, rows) = (h.cols / f, h.rows / f);
    let mut out = Heightmap::new(cols, rows, h.cell * f as f32, 0.0);
    out.data.par_chunks_mut(cols).enumerate().for_each(|(j, row)| {
        for (i, v) in row.iter_mut().enumerate() {
            let mut s = 0.0;
            for b in 0..f {
                for a in 0..f {
                    s += h.at(i * f + a, j * f + b);
                }
            }
            *v = s / (f * f) as f32;
        }
    });
    out
}

/// How far each vertex lies below the average of the ground within `radius` metres.
pub fn hollow(h: &Heightmap, radius: f32) -> Vec<f32> {
    let r = ((radius / h.cell).round() as i64).max(1);
    let (cols, rows) = (h.cols, h.rows);
    // Sums of a running window, along rows and then along columns.
    let mut rowsum = vec![0.0f32; cols * rows];
    rowsum.par_chunks_mut(cols).enumerate().for_each(|(j, out)| {
        for (i, o) in out.iter_mut().enumerate() {
            let (a, b) = ((i as i64 - r).max(0) as usize, ((i as i64 + r) as usize).min(cols - 1));
            *o = (a..=b).map(|k| h.at(k, j)).sum::<f32>() / (b - a + 1) as f32;
        }
    });
    let mut out = vec![0.0f32; cols * rows];
    out.par_chunks_mut(cols).enumerate().for_each(|(j, o)| {
        let (a, b) = ((j as i64 - r).max(0) as usize, ((j as i64 + r) as usize).min(rows - 1));
        for (i, v) in o.iter_mut().enumerate() {
            let mean = (a..=b).map(|k| rowsum[k * cols + i]).sum::<f32>() / (b - a + 1) as f32;
            *v = mean - h.at(i, j);
        }
    });
    out
}

/// The heightmap with every hollow filled to the lowest point of its rim and flats tilted a
/// little, so that water can run from every vertex down to the sea or the edge of the map.
/// Ground under the sea keeps its height.
pub fn drained(h: &Heightmap, sea: f32) -> Heightmap {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let (cols, rows) = (h.cols, h.rows);
    let mut out = h.clone();
    let mut done = vec![false; cols * rows];
    // Heights as ordered integers, so the queue can hold them.
    let key = |v: f32| Reverse(ordered(v));
    let mut queue: BinaryHeap<(Reverse<i64>, u32)> = BinaryHeap::new();
    for j in 0..rows {
        for i in 0..cols {
            let c = j * cols + i;
            if i == 0 || j == 0 || i + 1 == cols || j + 1 == rows || h.data[c] < sea {
                done[c] = true;
                queue.push((key(h.data[c]), c as u32));
            }
        }
    }
    while let Some((_, c)) = queue.pop() {
        let c = c as usize;
        let (i, j) = ((c % cols) as i64, (c / cols) as i64);
        let here = out.data[c];
        for (di, dj) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)] {
            let (a, b) = (i + di, j + dj);
            if a < 0 || b < 0 || a >= cols as i64 || b >= rows as i64 {
                continue;
            }
            let n = b as usize * cols + a as usize;
            if done[n] {
                continue;
            }
            done[n] = true;
            // A step up of a millimetre keeps the water running across what was flat.
            let v = out.data[n].max(here + 0.001);
            out.data[n] = v;
            queue.push((key(v), n as u32));
        }
    }
    out
}

/// A height as an integer of the same order, in tenths of a millimetre.
fn ordered(v: f32) -> i64 {
    (v as f64 * 10_000.0).round() as i64
}

/// Where water gathers: each vertex passes the water that reaches it, its own rain included,
/// to the lower vertices round it in proportion to how steeply the ground falls to them. The
/// result is the logarithm of the water, scaled so that the wettest place is 1. Ground under
/// the sea is 1.
pub fn wetness(h: &Heightmap, sea: f32) -> Vec<f32> {
    let h = &drained(h, sea);
    let (cols, rows) = (h.cols, h.rows);
    let n = cols * rows;
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.par_sort_unstable_by(|a, b| h.data[*b as usize].total_cmp(&h.data[*a as usize]));
    let mut water = vec![1.0f32; n];
    const AROUND: [(i64, i64, f32); 8] = [(1, 0, 1.0), (-1, 0, 1.0), (0, 1, 1.0), (0, -1, 1.0), (1, 1, 1.4142), (-1, 1, 1.4142), (1, -1, 1.4142), (-1, -1, 1.4142)];
    for &c in &order {
        let c = c as usize;
        let (i, j) = ((c % cols) as i64, (c / cols) as i64);
        let here = h.data[c];
        let mut falls = [0.0f32; 8];
        let mut total = 0.0;
        for (k, (di, dj, d)) in AROUND.iter().enumerate() {
            let (a, b) = (i + di, j + dj);
            if a < 0 || b < 0 || a >= cols as i64 || b >= rows as i64 {
                continue;
            }
            let drop = here - h.data[b as usize * cols + a as usize];
            if drop > 0.0 {
                // Steeper ways take more of the water.
                let f = (drop / d).powf(1.1);
                falls[k] = f;
                total += f;
            }
        }
        if total <= 0.0 {
            continue;
        }
        let give = water[c];
        for (k, (di, dj, _)) in AROUND.iter().enumerate() {
            if falls[k] > 0.0 {
                water[(j + dj) as usize * cols + (i + di) as usize] += give * falls[k] / total;
            }
        }
    }
    let max = water.iter().cloned().fold(1.0f32, f32::max).ln().max(1e-3);
    water.par_iter_mut().zip(&h.data).for_each(|(v, height)| {
        *v = if *height < sea { 1.0 } else { (v.ln() / max).clamp(0.0, 1.0) };
    });
    water
}

/// Lines along which water runs: from where at least `min_wetness` of it gathers, down the
/// steepest way to the sea, a hollow or the edge of the map; a line that meets another ends
/// where it joins. Lines shorter than `min_length` metres are left out. The heightmap is the
/// one the wetness was worked out on.
pub fn water_courses(h: &Heightmap, wet: &[f32], sea: f32, min_wetness: f32, min_length: f32) -> Vec<Vec<[f32; 2]>> {
    let h = &drained(h, sea);
    let (cols, rows) = (h.cols, h.rows);
    let channel = |c: usize| wet[c] >= min_wetness && h.data[c] >= sea;
    // The steepest way down from each cell.
    let down: Vec<Option<u32>> = (0..cols * rows).into_par_iter().map(|c| {
        let (i, j) = ((c % cols) as i64, (c / cols) as i64);
        let mut best: Option<(f32, u32)> = None;
        for (di, dj) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)] {
            let (a, b) = (i + di, j + dj);
            if a < 0 || b < 0 || a >= cols as i64 || b >= rows as i64 {
                continue;
            }
            let n = b as usize * cols + a as usize;
            let fall = (h.data[c] - h.data[n]) / ((di * di + dj * dj) as f32).sqrt();
            if fall > 0.0 && best.is_none_or(|b| fall > b.0) {
                best = Some((fall, n as u32));
            }
        }
        best.map(|b| b.1)
    }).collect();
    let mut fed = vec![false; cols * rows];
    for c in 0..cols * rows {
        if channel(c) {
            if let Some(n) = down[c] {
                fed[n as usize] = true;
            }
        }
    }
    let mut taken = vec![false; cols * rows];
    // Heads first from the highest, so the longest course reaches a junction first.
    let mut heads: Vec<usize> = (0..cols * rows).filter(|c| channel(*c) && !fed[*c]).collect();
    heads.sort_by(|a, b| h.data[*b].total_cmp(&h.data[*a]));
    let at = |c: usize| [(c % cols) as f32 * h.cell, (c / cols) as f32 * h.cell];
    let mut out = Vec::new();
    for head in heads {
        let mut line = vec![at(head)];
        let mut c = head;
        taken[c] = true;
        while let Some(n) = down[c].map(|n| n as usize) {
            line.push(at(n));
            if taken[n] || h.data[n] < sea {
                break;
            }
            taken[n] = true;
            c = n;
        }
        let length: f32 = line.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum();
        if line.len() > 1 && length >= min_length {
            out.push(simplify(&line, h.cell * 0.75));
        }
    }
    out
}

/// The line with the points left out that lie within `tolerance` of the line without them.
pub fn simplify(line: &[[f32; 2]], tolerance: f32) -> Vec<[f32; 2]> {
    if line.len() < 3 {
        return line.to_vec();
    }
    let mut keep = vec![false; line.len()];
    keep[0] = true;
    keep[line.len() - 1] = true;
    let mut stack = vec![(0, line.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (p, q) = (line[a], line[b]);
        let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
        let len = dx.hypot(dz).max(1e-6);
        let mut far = (0.0, a);
        for (k, r) in line.iter().enumerate().take(b).skip(a + 1) {
            let d = ((r[0] - p[0]) * dz - (r[1] - p[1]) * dx).abs() / len;
            if d > far.0 {
                far = (d, k);
            }
        }
        if far.0 > tolerance {
            keep[far.1] = true;
            stack.push((a, far.1));
            stack.push((far.1, b));
        }
    }
    line.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Metres from every cell to the nearest seed, with steps along rows, columns and diagonals.
pub fn distance_field(seeds: &[bool], cols: usize, rows: usize, cell: f32) -> Vec<f32> {
    let mut d: Vec<f32> = seeds.iter().map(|s| if *s { 0.0 } else { f32::MAX }).collect();
    if !seeds.iter().any(|s| *s) {
        return d;
    }
    let (a, b) = (cell, cell * std::f32::consts::SQRT_2);
    for j in 0..rows {
        for i in 0..cols {
            let c = j * cols + i;
            let mut v = d[c];
            if i > 0 { v = v.min(d[c - 1] + a); }
            if j > 0 {
                v = v.min(d[c - cols] + a);
                if i > 0 { v = v.min(d[c - cols - 1] + b); }
                if i + 1 < cols { v = v.min(d[c - cols + 1] + b); }
            }
            d[c] = v;
        }
    }
    for j in (0..rows).rev() {
        for i in (0..cols).rev() {
            let c = j * cols + i;
            let mut v = d[c];
            if i + 1 < cols { v = v.min(d[c + 1] + a); }
            if j + 1 < rows {
                v = v.min(d[c + cols] + a);
                if i + 1 < cols { v = v.min(d[c + cols + 1] + b); }
                if i > 0 { v = v.min(d[c + cols - 1] + b); }
            }
            d[c] = v;
        }
    }
    d
}

/// More that a place must be for a rule to hold there, besides its height and slope. Each is
/// a range, and is left out when `None`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Conditions {
    /// How far below the ground round it, in metres: above 0 hollows, below 0 ridges.
    pub hollow: Option<[f32; 2]>,
    /// Where water gathers, 0 dry to 1 wettest.
    pub wetness: Option<[f32; 2]>,
    /// Metres to the nearest water.
    pub water_dist: Option<[f32; 2]>,
    /// Metres to the nearest road.
    pub road_dist: Option<[f32; 2]>,
    /// Ragged edges: where each place is tested wanders by up to this many metres.
    pub edge_noise: f32,
}

/// How far round a place the ground is compared with to tell hollows from ridges, in metres.
pub const HOLLOW_RADIUS: f32 = 30.0;

impl Conditions {
    pub fn needs(&self) -> Needs {
        Needs { hollow: self.hollow.is_some(), wetness: self.wetness.is_some(), water: self.water_dist.is_some(), roads: self.road_dist.is_some() }
    }
    pub fn is_empty(&self) -> bool {
        !self.needs().any()
    }
    /// Whether the place at `x`, `z` meets every condition; `noise` moves where it is tested.
    pub fn test(&self, maps: &TerrainMaps, noise: &crate::noise::Perlin, x: f32, z: f32) -> bool {
        if self.is_empty() {
            return true;
        }
        let (x, z) = if self.edge_noise > 0.0 {
            let s = (self.edge_noise * 3.0).max(1.0);
            (x + noise.fbm(x / s, z / s, 3, 0.5, 2.0) * self.edge_noise, z + noise.fbm(z / s + 17.1, x / s - 5.3, 3, 0.5, 2.0) * self.edge_noise)
        } else {
            (x, z)
        };
        let within = |r: &Option<[f32; 2]>, v: f32| r.is_none_or(|r| v >= r[0] && v <= r[1]);
        within(&self.hollow, maps.hollow_at(x, z)) && within(&self.wetness, maps.wetness_at(x, z)) && within(&self.water_dist, maps.water_dist_at(x, z)) && within(&self.road_dist, maps.road_dist_at(x, z))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::WorldMeta;

    /// A valley running north, its floor at x = 32.
    fn valley() -> World {
        let mut w = World::new(WorldMeta::new("T", 64, 4.0), 0.0);
        for j in 0..64 {
            for i in 0..64 {
                let v = (i as f32 - 32.0).abs() * 2.0 + j as f32 * 0.5 + 10.0;
                w.base.set(i, j, v);
            }
        }
        w.recompose(None);
        w
    }

    #[test]
    fn water_gathers_on_the_valley_floor() {
        let w = valley();
        let m = TerrainMaps::new(&w, &Landmarks::default(), Needs { wetness: true, hollow: true, ..Default::default() }, 20.0);
        // The floor near the south end has the water of the whole valley.
        assert!(m.wetness_at(32.0 * 4.0, 4.0) > 0.8, "{}", m.wetness_at(128.0, 4.0));
        assert!(m.wetness_at(4.0, 200.0) < 0.3);
        assert!(m.hollow_at(128.0, 128.0) > 1.0);
        assert!(m.hollow_at(4.0 * 20.0, 128.0) < 1.0);
    }

    #[test]
    fn distances_to_roads_and_water() {
        let mut w = valley();
        w.meta.sea_level = -100.0;
        let marks = Landmarks { roads: vec![vec![[0.0, 100.0], [256.0, 100.0]]], water_points: vec![[200.0, 200.0]], ponds: Vec::new() };
        let m = TerrainMaps::new(&w, &marks, Needs { water: true, roads: true, ..Default::default() }, 20.0);
        assert!(m.road_dist_at(50.0, 100.0) < 4.0);
        assert!((m.road_dist_at(50.0, 140.0) - 40.0).abs() < 6.0);
        assert!(m.water_dist_at(200.0, 200.0) < 1.0);
        assert!(m.water_dist_at(100.0, 200.0) > 80.0);
    }

    #[test]
    fn a_water_course_runs_down_the_valley() {
        let w = valley();
        let wet = wetness(&w.height, -100.0);
        let lines = water_courses(&w.height, &wet, -100.0, 0.7, 40.0);
        assert!(!lines.is_empty());
        let longest = lines.iter().max_by_key(|l| l.len()).unwrap();
        // It ends at the south edge, on the valley floor.
        let end = longest.last().unwrap();
        assert!(end[1] < 8.0 && (end[0] - 128.0).abs() < 12.0, "{end:?}");
    }

    #[test]
    fn conditions_hold_only_inside_their_ranges() {
        let w = valley();
        let m = TerrainMaps::new(&w, &Landmarks::default(), Needs { wetness: true, ..Default::default() }, 20.0);
        let c = Conditions { wetness: Some([0.6, 1.0]), ..Default::default() };
        let n = crate::noise::Perlin::new(1);
        assert!(c.test(&m, &n, 128.0, 4.0));
        assert!(!c.test(&m, &n, 4.0, 200.0));
        assert!(Conditions::default().test(&m, &n, 4.0, 200.0));
    }
}
