//! Generators of whole landscapes: coasts with beaches and stones, cliffs of rock on steep
//! slopes, fields in strips, and the verges of roads with their bushes and signs.
//!
//! Each gives objects and a new surface mask (what changed becomes the generator's mask
//! modifier); the coast can also shape the ground of its beaches.
use crate::analysis::TerrainMaps;
use crate::gen::slope_angles;
use crate::library::ObjectSet;
use crate::modules::{Picker, Spacing};
use crate::noise::Perlin;
use crate::objects::Obj;
use crate::shapes::PolygonIndex;
use crate::terrain::Heightmap;
use crate::world::World;
use crate::Job;
use anyhow::Result;
use rand::{rngs::StdRng, Rng, SeedableRng};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// The surface layer called `name`, or else the first whose name holds one of `guesses`.
pub fn find_layer(w: &World, name: &str, guesses: &[&str]) -> Option<u8> {
    if !name.is_empty() {
        return w.layer_index(name).map(|i| i as u8);
    }
    w.layers.iter().position(|l| {
        let n = l.name.to_ascii_lowercase();
        guesses.iter().any(|g| n.contains(g))
    }).map(|i| i as u8)
}

/// Paints the mask where `pick` gives a layer for the pixel's world x and z.
fn paint_mask(w: &World, pick: impl Fn(f32, f32) -> Option<u8> + Sync) -> Vec<u8> {
    let (mw, mh) = (w.mask.width, w.mask.height);
    let size = w.meta.size_m();
    let mut out = w.mask.data.clone();
    out.par_chunks_mut(mw).enumerate().for_each(|(y, row)| {
        let z = (1.0 - (y as f32 + 0.5) / mh as f32) * size;
        for (x, v) in row.iter_mut().enumerate() {
            if let Some(l) = pick((x as f32 + 0.5) / mw as f32 * size, z) {
                *v = l;
            }
        }
    });
    out
}

/// The mask with `layer` painted inside the polygons.
pub fn polygons_mask(w: &World, polygons: &[Vec<[f32; 2]>], layer: u8) -> Vec<u8> {
    let boxes: Vec<[f32; 4]> = polygons.iter().map(|p| p.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, q| [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])])).collect();
    paint_mask(w, |x, z| {
        polygons.iter().zip(&boxes).any(|(p, b)| x >= b[0] && x <= b[2] && z >= b[1] && z <= b[3] && crate::shapes::point_in_polygon(p, x, z)).then_some(layer)
    })
}

/// A little noise that moves an edge by up to `metres`.
fn ragged(noise: &Perlin, x: f32, z: f32, metres: f32) -> f32 {
    if metres <= 0.0 {
        return 0.0;
    }
    noise.fbm(x / (metres * 3.0), z / (metres * 3.0), 3, 0.5, 2.0) * metres
}

fn in_area(area: Option<&PolygonIndex>, x: f32, z: f32) -> bool {
    area.is_none_or(|a| a.contains(x, z))
}

// ---------------------------------------------------------------- coast

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CoastGen {
    /// Surface of beaches and of shallow sea floor; empty takes a layer named like sand.
    pub sand: String,
    /// Surface of steep shores; empty takes a layer named like rock.
    pub rock: String,
    /// How far inland beaches reach from the water, in metres, and how high above the sea.
    pub width: f32,
    pub max_height: f32,
    /// Shores steeper than this, in degrees, are rock rather than beach.
    pub cliff_slope: f32,
    /// Stones and driftwood along the water, per hectare of beach.
    pub objects: ObjectSet,
    pub density: f32,
    /// The ground of the beaches is laid to a gentle slope.
    pub shape: bool,
    pub beach_slope: f32,
    /// How ragged the edge of the beach is, in metres.
    pub edge_noise: f32,
    pub seed: u64,
}

impl Default for CoastGen {
    fn default() -> Self {
        let stones = ["stone1", "stone2", "stone3", "stone4", "stone5"].iter().map(|s| (format!("dz\\rocks\\{s}.p3d"), 1.0)).collect();
        Self {
            sand: String::new(), rock: String::new(), width: 28.0, max_height: 3.5, cliff_slope: 32.0,
            objects: ObjectSet { models: stones, scale_min: 0.5, scale_max: 1.4, align: true, ..Default::default() },
            density: 6.0, shape: true, beach_slope: 4.0, edge_noise: 8.0, seed: 1,
        }
    }
}

/// The surface of the beaches: the one chosen, or one named like sand, or else like gravel.
pub fn coast_sand(w: &World, g: &CoastGen) -> Option<u8> {
    find_layer(w, &g.sand, &["sand", "beach"]).or_else(|| if g.sand.is_empty() { find_layer(w, "", &["gravel", "pebble", "shingle"]) } else { None })
}

/// The mask with beaches, shallow sea floor and rocky shores painted.
pub fn coast_mask(w: &World, g: &CoastGen, maps: &TerrainMaps, area: Option<&PolygonIndex>) -> Vec<u8> {
    let sand = coast_sand(w, g);
    let rock = find_layer(w, &g.rock, &["rock", "stone", "cliff"]);
    let sea = w.meta.sea_level;
    let noise = Perlin::new(g.seed);
    paint_mask(w, |x, z| {
        if !in_area(area, x, z) {
            return None;
        }
        let h = w.height.sample(x, z);
        let edge = ragged(&noise, x, z, g.edge_noise);
        if h < sea {
            // The floor of shallow water near the shore.
            return (h > sea - 4.0 + edge * 0.2).then_some(sand).flatten();
        }
        let d = maps.water_dist_at(x, z) + edge;
        let slope = w.height.slope(x, z);
        if slope >= g.cliff_slope && d <= g.width * 2.0 && h <= sea + g.max_height * 6.0 {
            rock
        } else if d <= g.width && h <= sea + g.max_height + edge * 0.1 {
            sand
        } else {
            None
        }
    })
}

/// Stones and driftwood along the water's edge.
pub fn coast_objects(w: &mut World, g: &CoastGen, maps: &TerrainMaps, area: Option<&PolygonIndex>, job: &Job) -> Result<Vec<Obj>> {
    let picker = Picker::new(w, &g.objects)?;
    let mut rng = StdRng::seed_from_u64(g.seed);
    let sea = w.meta.sea_level;
    let mut spacing = Spacing::new(2.0);
    let mut out = Vec::new();
    let cell = maps.cell;
    let per_cell = g.density * cell * cell / 10_000.0;
    for j in 0..maps.rows {
        if j % 64 == 0 {
            job.check()?;
            job.set_progress(j as f32 / maps.rows as f32);
        }
        for i in 0..maps.cols {
            let d = maps.water_dist[j * maps.cols + i];
            if d <= 0.0 || d > g.width * 0.6 {
                continue;
            }
            // Most stones lie close to the water.
            let k = per_cell * (1.0 - d / (g.width * 0.6)) * 2.0;
            let mut n = k.floor() as usize + (rng.gen::<f32>() < k.fract()) as usize;
            while n > 0 {
                n -= 1;
                let (x, z) = ((i as f32 + rng.gen::<f32>()) * cell, (j as f32 + rng.gen::<f32>()) * cell);
                if !in_area(area, x, z) || w.height.sample(x, z) < sea - 0.3 || !spacing.claim(x, z) {
                    continue;
                }
                let model = picker.model(&mut rng);
                let mut o = picker.object(w, &mut rng, model, x, z, None);
                // Half in the sand.
                o.elevation = -0.15 * o.scale;
                out.push(o);
            }
        }
    }
    Ok(out)
}

/// Lays the ground of beaches to a gentle slope rising from the water.
pub fn coast_heights(h: &mut Heightmap, sea: f32, g: &CoastGen) {
    let maps = TerrainMaps::sea(h, sea);
    let rise = g.beach_slope.to_radians().tan();
    let cell = h.cell;
    let cols = h.cols;
    h.data.par_chunks_mut(cols).enumerate().for_each(|(j, row)| {
        for (i, v) in row.iter_mut().enumerate() {
            if *v < sea || *v > sea + g.max_height * 3.0 {
                continue;
            }
            let d = maps.water_dist_at(i as f32 * cell, j as f32 * cell);
            if d >= g.width {
                continue;
            }
            let target = sea + 0.15 + d * rise;
            // Full near the water, fading out at the back of the beach.
            let t = 1.0 - smooth(g.width * 0.5, g.width, d);
            *v += (target - *v) * t;
        }
    });
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------------------------------------------------------------- cliffs

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CliffGen {
    pub objects: ObjectSet,
    /// Slopes at least this steep, in degrees, get rocks.
    pub min_slope: f32,
    /// Rocks per hectare of steep ground.
    pub density: f32,
    pub min_distance: f32,
    /// How deep each rock is sunk into the slope, as a share of its size.
    pub sink: f32,
    /// Surface painted under the rocks; empty takes a layer named like rock.
    pub surface: String,
    pub seed: u64,
}

impl Default for CliffGen {
    fn default() -> Self {
        let rocks = ["rock_monolith1", "rock_monolith2", "rock_monolith3", "rock_wallh1", "rock_wallh2", "rock_wallh3", "rock_spike1", "rock_apart1"].iter().map(|s| (format!("dz\\rocks\\{s}.p3d"), 1.0)).collect();
        Self { objects: ObjectSet { models: rocks, scale_min: 0.6, scale_max: 1.3, align: true, ..Default::default() }, min_slope: 38.0, density: 12.0, min_distance: 9.0, sink: 0.3, surface: String::new(), seed: 1 }
    }
}

/// Rocks on the steep slopes, turned to face down the slope and sunk into it.
pub fn cliff_objects(w: &mut World, g: &CliffGen, area: Option<&PolygonIndex>, bounds: [f32; 4], job: &Job) -> Result<Vec<Obj>> {
    let picker = Picker::new(w, &g.objects)?;
    let mut rng = StdRng::seed_from_u64(g.seed);
    let mut spacing = Spacing::new(g.min_distance);
    let cell = w.height.cell;
    let per_cell = g.density * cell * cell / 10_000.0;
    let (i0, j0) = ((bounds[0] / cell).floor().max(0.0) as usize, (bounds[1] / cell).floor().max(0.0) as usize);
    let (i1, j1) = (((bounds[2] / cell).ceil() as usize).min(w.height.cols - 1), ((bounds[3] / cell).ceil() as usize).min(w.height.rows - 1));
    let mut out = Vec::new();
    for j in j0..=j1 {
        if j % 64 == 0 {
            job.check()?;
            job.set_progress((j - j0) as f32 / (j1 - j0 + 1) as f32);
        }
        for i in i0..=i1 {
            let (x, z) = ((i as f32 + rng.gen::<f32>()) * cell, (j as f32 + rng.gen::<f32>()) * cell);
            let slope = w.height.slope(x, z);
            if slope < g.min_slope || rng.gen::<f32>() > per_cell * (1.0 + (slope - g.min_slope) / 20.0) || !in_area(area, x, z) || !spacing.claim(x, z) {
                continue;
            }
            let n = w.height.normal(x, z);
            // Facing down the slope, give or take a little.
            let down = n[0].atan2(n[2]).to_degrees() + rng.gen_range(-25.0..25.0);
            let model = picker.model(&mut rng);
            let mut o = picker.object(w, &mut rng, model, x, z, Some(down));
            if g.objects.align {
                (o.pitch, o.roll) = slope_angles(&w.height, x, z, o.yaw);
            }
            o.elevation = -g.sink * 3.0 * o.scale;
            out.push(o);
        }
    }
    Ok(out)
}

/// The rock surface on the steep slopes.
pub fn cliff_mask(w: &World, g: &CliffGen, area: Option<&PolygonIndex>) -> Vec<u8> {
    let Some(rock) = find_layer(w, &g.surface, &["rock", "stone", "cliff"]) else { return w.mask.data.clone() };
    let noise = Perlin::new(g.seed ^ 0xc1);
    paint_mask(w, |x, z| (in_area(area, x, z) && w.height.slope(x, z) + ragged(&noise, x, z, 4.0) >= g.min_slope - 6.0).then_some(rock))
}

// ---------------------------------------------------------------- fields

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FieldGen {
    /// Surfaces the strips are sown with, one chosen for each strip; empty takes layers named
    /// like fields.
    pub surfaces: Vec<String>,
    /// Surface of the narrow balks between strips; empty takes a layer named like grass.
    pub balk: String,
    /// Width of a strip in metres, from and to.
    pub min_width: f32,
    pub max_width: f32,
    pub balk_width: f32,
    /// Direction of the strips in degrees from north; `None` follows the longest side of each
    /// field.
    pub angle: Option<f32>,
    /// Hay bales and the like, per hectare, laid along the strips.
    pub objects: ObjectSet,
    pub density: f32,
    pub seed: u64,
}

impl Default for FieldGen {
    fn default() -> Self {
        let bales = vec![("dz\\structures\\industrial\\misc\\misc_haybale.p3d".to_string(), 3.0), ("dz\\structures\\industrial\\misc\\misc_haybale_decayed.p3d".to_string(), 1.0)];
        Self { surfaces: Vec::new(), balk: String::new(), min_width: 18.0, max_width: 55.0, balk_width: 2.5, angle: None, objects: ObjectSet { models: bales, scale_min: 0.95, scale_max: 1.05, random_yaw: false, align: true, ..Default::default() }, density: 1.5, seed: 1 }
    }
}

/// How each field is cut into strips: its direction, where its strips start, and each strip's
/// far edge and surface.
struct Strips {
    ring: Vec<[f32; 2]>,
    /// Unit vector along the strips, and across them.
    along: [f32; 2],
    across: [f32; 2],
    start: f32,
    edges: Vec<(f32, u8)>,
}

impl Strips {
    fn new(ring: &[[f32; 2]], g: &FieldGen, layers: &[u8], rng: &mut StdRng) -> Option<Self> {
        if ring.len() < 3 || layers.is_empty() {
            return None;
        }
        let angle = match g.angle {
            Some(a) => a.to_radians(),
            None => {
                // The longest side.
                let mut best = (0.0, 0.0);
                for k in 0..ring.len() {
                    let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                    let len = (b[0] - a[0]).hypot(b[1] - a[1]);
                    if len > best.0 {
                        best = (len, (b[0] - a[0]).atan2(b[1] - a[1]));
                    }
                }
                best.1
            }
        };
        let along = [angle.sin(), angle.cos()];
        let across = [along[1], -along[0]];
        let us: Vec<f32> = ring.iter().map(|p| p[0] * across[0] + p[1] * across[1]).collect();
        let (start, end) = us.iter().fold((f32::MAX, f32::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
        let mut edges = Vec::new();
        let mut u = start;
        while u < end {
            u += rng.gen_range(g.min_width.max(1.0)..=g.max_width.max(g.min_width.max(1.0) + 0.01));
            edges.push((u, layers[rng.gen_range(0..layers.len())]));
        }
        Some(Strips { ring: ring.to_vec(), along, across, start, edges })
    }
    /// The strip at a point: its surface, and how far it is from the nearer edge of the strip.
    fn at(&self, x: f32, z: f32) -> (u8, f32) {
        let u = x * self.across[0] + z * self.across[1];
        let k = self.edges.partition_point(|e| e.0 < u).min(self.edges.len() - 1);
        let lo = if k == 0 { self.start } else { self.edges[k - 1].0 };
        (self.edges[k].1, (u - lo).min(self.edges[k].0 - u))
    }
}

/// The surfaces of the strips: those chosen, or those named like fields, or else like grass
/// and soil.
pub fn field_layers(w: &World, g: &FieldGen) -> Vec<u8> {
    let named = |keys: &[&str]| -> Vec<u8> {
        w.layers.iter().enumerate().filter(|(_, l)| {
            let n = l.name.to_ascii_lowercase();
            keys.iter().any(|k| n.contains(k))
        }).map(|(i, _)| i as u8).collect()
    };
    let mut layers: Vec<u8> = g.surfaces.iter().filter_map(|n| w.layer_index(n).map(|i| i as u8)).collect();
    if layers.is_empty() {
        layers = named(&["field", "stubble", "crop", "farm", "wheat"]);
    }
    if layers.is_empty() {
        layers = named(&["grass_tall", "grass", "dirt", "soil"]);
    }
    layers
}

/// The strips of every field, in the order of `fields`.
fn field_strips(w: &World, g: &FieldGen, fields: &[Vec<[f32; 2]>]) -> Vec<Strips> {
    let layers = field_layers(w, g);
    let mut rng = StdRng::seed_from_u64(g.seed);
    fields.iter().filter_map(|f| Strips::new(f, g, &layers, &mut rng)).collect()
}

/// The fields painted in strips with balks between them.
pub fn field_mask(w: &World, g: &FieldGen, fields: &[Vec<[f32; 2]>]) -> Vec<u8> {
    let strips = field_strips(w, g, fields);
    let balk = find_layer(w, &g.balk, &["grass", "meadow"]);
    paint_mask(w, |x, z| {
        let s = strips.iter().find(|s| crate::shapes::point_in_polygon(&s.ring, x, z))?;
        let (layer, from_edge) = s.at(x, z);
        if from_edge < g.balk_width * 0.5 {
            balk
        } else {
            Some(layer)
        }
    })
}

/// Bales laid along the strips, away from the balks.
pub fn field_objects(w: &mut World, g: &FieldGen, fields: &[Vec<[f32; 2]>]) -> Result<Vec<Obj>> {
    if g.density <= 0.0 {
        return Ok(Vec::new());
    }
    let strips = field_strips(w, g, fields);
    let picker = Picker::new(w, &g.objects)?;
    let mut rng = StdRng::seed_from_u64(g.seed ^ 0xba1e);
    let mut spacing = Spacing::new(4.0);
    let mut out = Vec::new();
    for s in &strips {
        let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in &s.ring {
            (x0, z0, x1, z1) = (x0.min(p[0]), z0.min(p[1]), x1.max(p[0]), z1.max(p[1]));
        }
        let hectares = (x1 - x0) * (z1 - z0) / 10_000.0;
        let tries = (hectares * g.density * 3.0) as usize;
        let want = (hectares * g.density) as usize;
        let yaw = s.along[0].atan2(s.along[1]).to_degrees();
        let mut placed = 0;
        for _ in 0..tries {
            if placed >= want {
                break;
            }
            let (x, z) = (rng.gen_range(x0..x1.max(x0 + 0.01)), rng.gen_range(z0..z1.max(z0 + 0.01)));
            if !crate::shapes::point_in_polygon(&s.ring, x, z) || s.at(x, z).1 < g.balk_width + 2.0 || !spacing.claim(x, z) {
                continue;
            }
            let model = picker.model(&mut rng);
            let along = if g.objects.random_yaw { None } else { Some(yaw + if rng.gen() { 0.0 } else { 180.0 }) };
            out.push(picker.object(w, &mut rng, model, x, z, along));
            placed += 1;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- road verges

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RoadsideGen {
    /// Surface of the verge; empty paints none.
    pub verge: String,
    /// How far the middle of the verge is from the middle of the road, and how wide it is.
    pub offset: f32,
    pub verge_width: f32,
    /// Bushes and trees along the road: how many each 100 metres a side, and how wide a band
    /// beyond the verge they grow in.
    pub plants: ObjectSet,
    pub plant_density: f32,
    pub plant_band: f32,
    /// Signs and posts, one about every `sign_spacing` metres a side; 0 for none.
    pub signs: ObjectSet,
    pub sign_spacing: f32,
    pub both_sides: bool,
    pub seed: u64,
}

impl Default for RoadsideGen {
    fn default() -> Self {
        let bushes = ["b_corylusavellana_1f", "b_prunusspinosa_1s", "b_rosacanina_1s", "b_sambucusnigra_1s", "b_crataeguslaevigata_1s"].iter().map(|s| (format!("dz\\plants\\bush\\{s}.p3d"), 1.0)).collect();
        let signs = ["sign_danger", "sign_deer", "sign_cow", "sign_attention", "sign_downhill"].iter().map(|s| (format!("dz\\structures\\signs\\traffic\\{s}.p3d"), 1.0)).collect();
        Self {
            verge: String::new(), offset: 6.0, verge_width: 3.0,
            plants: ObjectSet { models: bushes, scale_min: 0.8, scale_max: 1.2, ..Default::default() }, plant_density: 6.0, plant_band: 8.0,
            signs: ObjectSet { models: signs, scale_min: 1.0, scale_max: 1.0, random_yaw: false, ..Default::default() }, sign_spacing: 600.0,
            both_sides: true, seed: 1,
        }
    }
}

/// Points along a line every `step` metres, with the direction there.
fn walk(line: &[[f32; 2]], step: f32, mut f: impl FnMut([f32; 2], [f32; 2], f32)) {
    let mut carry = 0.0;
    let mut along = 0.0;
    for s in line.windows(2) {
        let (a, b) = (s[0], s[1]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        if len <= 1e-3 {
            continue;
        }
        let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        let mut t = carry;
        while t < len {
            f([a[0] + dir[0] * t, a[1] + dir[1] * t], dir, along + t);
            t += step;
        }
        carry = t - len;
        along += len;
    }
}

/// The verges of the lines painted.
pub fn roadside_mask(w: &World, g: &RoadsideGen, lines: &[Vec<[f32; 2]>]) -> Vec<u8> {
    let Some(verge) = (!g.verge.is_empty()).then(|| w.layer_index(&g.verge)).flatten().map(|i| i as u8) else { return w.mask.data.clone() };
    let (mw, mh) = (w.mask.width, w.mask.height);
    let size = w.meta.size_m();
    let mut out = w.mask.data.clone();
    let (lo, hi) = (g.offset - g.verge_width * 0.5, g.offset + g.verge_width * 0.5);
    for line in lines {
        for s in line.windows(2) {
            let (a, b) = (s[0], s[1]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let len2 = (dx * dx + dz * dz).max(1e-6);
            let to_px = |x: f32, z: f32| ((x / size * mw as f32) as i64, ((1.0 - z / size) * mh as f32) as i64);
            let (ax, ay) = to_px(a[0].min(b[0]) - hi, a[1].max(b[1]) + hi);
            let (bx, by) = to_px(a[0].max(b[0]) + hi, a[1].min(b[1]) - hi);
            for py in ay.max(0)..=by.min(mh as i64 - 1) {
                for px in ax.max(0)..=bx.min(mw as i64 - 1) {
                    let (x, z) = ((px as f32 + 0.5) / mw as f32 * size, (1.0 - (py as f32 + 0.5) / mh as f32) * size);
                    let t = (((x - a[0]) * dx + (z - a[1]) * dz) / len2).clamp(0.0, 1.0);
                    let d = (x - a[0] - dx * t).hypot(z - a[1] - dz * t);
                    // Which side the point is on.
                    let left = dx * (z - a[1]) - dz * (x - a[0]) > 0.0;
                    if d >= lo && d <= hi && (g.both_sides || left) {
                        out[py as usize * mw + px as usize] = verge;
                    }
                }
            }
        }
    }
    out
}

/// Bushes and signs along the lines; nothing is put where another road runs closer than the
/// verge.
pub fn roadside_objects(w: &mut World, g: &RoadsideGen, lines: &[Vec<[f32; 2]>], roads: &[Vec<[f32; 2]>]) -> Result<Vec<Obj>> {
    let mut rng = StdRng::seed_from_u64(g.seed);
    let mut out = Vec::new();
    let near_road = |x: f32, z: f32, clear: f32| {
        roads.iter().any(|r| r.windows(2).any(|s| {
            let (a, b) = (s[0], s[1]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let t = (((x - a[0]) * dx + (z - a[1]) * dz) / (dx * dx + dz * dz).max(1e-6)).clamp(0.0, 1.0);
            (x - a[0] - dx * t).hypot(z - a[1] - dz * t) < clear
        }))
    };
    let sides: &[f32] = if g.both_sides { &[1.0, -1.0] } else { &[1.0] };
    let size = w.meta.size_m();
    let inside = |x: f32, z: f32| x >= 0.0 && z >= 0.0 && x <= size && z <= size;
    if g.plant_density > 0.0 && (!g.plants.models.is_empty() || g.plants.preset.is_some()) {
        let picker = Picker::new(w, &g.plants)?;
        let mut spacing = Spacing::new(2.5);
        let step = 100.0 / g.plant_density;
        for line in lines {
            let mut points = Vec::new();
            walk(line, step, |p, dir, _| points.push((p, dir)));
            for (p, dir) in points {
                for side in sides {
                    let along = rng.gen_range(-0.5..0.5) * step;
                    let out_m = g.offset + g.verge_width * 0.5 + rng.gen_range(0.0..g.plant_band.max(0.1));
                    let (x, z) = (p[0] + dir[0] * along + dir[1] * out_m * side, p[1] + dir[1] * along - dir[0] * out_m * side);
                    if !inside(x, z) || near_road(x, z, g.offset + g.verge_width * 0.5) || !spacing.claim(x, z) {
                        continue;
                    }
                    let model = picker.model(&mut rng);
                    out.push(picker.object(w, &mut rng, model, x, z, None));
                }
            }
        }
    }
    if g.sign_spacing > 0.0 && (!g.signs.models.is_empty() || g.signs.preset.is_some()) {
        let picker = Picker::new(w, &g.signs)?;
        for line in lines {
            let mut points = Vec::new();
            walk(line, g.sign_spacing, |p, dir, along| points.push((p, dir, along)));
            for (k, (p, dir, along)) in points.into_iter().enumerate() {
                // Not right at the start, and not at the same place on both sides.
                if along < g.sign_spacing * 0.3 {
                    continue;
                }
                let side = sides[k % sides.len()];
                let shift = rng.gen_range(-0.3..0.3) * g.sign_spacing;
                let out_m = g.offset - g.verge_width * 0.25;
                let (x, z) = (p[0] + dir[0] * shift + dir[1] * out_m * side, p[1] + dir[1] * shift - dir[0] * out_m * side);
                if !inside(x, z) || near_road(x, z, out_m - 0.5) {
                    continue;
                }
                // Facing the traffic that comes towards it on its side.
                let yaw = dir[0].atan2(dir[1]).to_degrees() + if side > 0.0 { 180.0 } else { 0.0 };
                let model = picker.model(&mut rng);
                out.push(picker.object(w, &mut rng, model, x, z, Some(yaw)));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{SurfaceLayer, WorldMeta};

    /// Sea in the west, land rising to the east; a steep wall at x 150..170.
    fn world() -> World {
        let mut w = World::new(WorldMeta::new("T", 64, 4.0), 0.0);
        for j in 0..64 {
            for i in 0..64 {
                let x = i as f32 * 4.0;
                let h = if x < 40.0 { -3.0 } else if x < 150.0 { (x - 40.0) * 0.05 } else if x < 170.0 { 5.5 + (x - 150.0) * 1.5 } else { 35.5 };
                w.base.set(i, j, h);
            }
        }
        w.recompose(None);
        w.mask = crate::terrain::MaskMap::new(64, 64, 0);
        w.layers = ["grass", "sand", "rock", "stubble", "field"].iter().map(|n| SurfaceLayer { name: n.to_string(), texture: String::new(), material: String::new(), color: [0, 0, 0] }).collect();
        w
    }

    fn mask_at(w: &World, m: &[u8], x: f32, z: f32) -> u8 {
        let size = w.meta.size_m();
        let (px, py) = ((x / size * 64.0) as usize, ((1.0 - z / size) * 64.0) as usize);
        m[py * 64 + px]
    }

    #[test]
    fn the_coast_has_a_beach_by_the_water_and_rock_on_the_wall() {
        let w = world();
        let g = CoastGen { edge_noise: 0.0, ..Default::default() };
        let maps = TerrainMaps::sea(&w.height, 0.0);
        let m = coast_mask(&w, &g, &maps, None);
        assert_eq!(mask_at(&w, &m, 50.0, 128.0), 1, "sand by the water");
        assert_eq!(mask_at(&w, &m, 130.0, 128.0), 0, "grass further inland");
        assert_eq!(mask_at(&w, &m, 20.0, 128.0), 1, "sand under shallow water");
    }

    #[test]
    fn beaches_are_laid_to_a_gentle_slope() {
        let mut w = world();
        // A bump on the beach.
        w.base.set(13, 32, 2.5);
        let mut h = w.base.clone();
        coast_heights(&mut h, 0.0, &CoastGen::default());
        assert!(h.at(13, 32) < 2.0, "{}", h.at(13, 32));
    }

    #[test]
    fn cliffs_get_rocks_and_rock_surface() {
        let mut w = world();
        let g = CliffGen { density: 400.0, min_distance: 3.0, ..Default::default() };
        let objects = cliff_objects(&mut w, &g, None, [0.0, 0.0, 256.0, 256.0], &Job::default()).unwrap();
        assert!(!objects.is_empty());
        assert!(objects.iter().all(|o| o.x > 140.0 && o.x < 180.0), "only on the wall");
        let m = cliff_mask(&w, &g, None);
        assert_eq!(mask_at(&w, &m, 160.0, 128.0), 2);
        assert_eq!(mask_at(&w, &m, 100.0, 128.0), 0);
    }

    #[test]
    fn fields_are_strips_with_balks() {
        let w = world();
        let field = vec![[60.0, 20.0], [140.0, 20.0], [140.0, 240.0], [60.0, 240.0]];
        let g = FieldGen { min_width: 20.0, max_width: 20.0, balk_width: 6.0, ..Default::default() };
        let m = field_mask(&w, &g, std::slice::from_ref(&field));
        let inside: Vec<u8> = (0..20).map(|k| mask_at(&w, &m, 62.0 + k as f32 * 4.0, 128.0)).collect();
        assert!(inside.iter().all(|l| [0, 3, 4].contains(l)), "{inside:?}");
        assert!(inside.contains(&0), "a balk between strips: {inside:?}");
        assert_eq!(mask_at(&w, &m, 200.0, 128.0), 0, "nothing outside the field");
    }

    #[test]
    fn verges_and_bushes_follow_the_road() {
        let mut w = world();
        let road = vec![[50.0, 10.0], [50.0, 240.0]];
        let g = RoadsideGen { verge: "sand".into(), verge_width: 6.0, sign_spacing: 0.0, ..Default::default() };
        let m = roadside_mask(&w, &g, std::slice::from_ref(&road));
        assert_eq!(mask_at(&w, &m, 56.0, 128.0), 1);
        assert_eq!(mask_at(&w, &m, 44.0, 128.0), 1);
        assert_eq!(mask_at(&w, &m, 50.0, 128.0), 0);
        let objects = roadside_objects(&mut w, &g, std::slice::from_ref(&road), std::slice::from_ref(&road)).unwrap();
        assert!(objects.len() > 10);
        assert!(objects.iter().all(|o| (o.x - 50.0).abs() >= 8.9 && (o.x - 50.0).abs() <= 17.1), "beyond the verge");
    }
}
