//! Placement inside areas and along lines, after the tasks of Terrain Processor.
use crate::gen::{prepare_line, slope_angles};
use crate::library::{AreaGen, AreaMethod, LineGen, LineMethod, LineMode, LineOptions, ObjectSet};
use crate::objects::Obj;
use crate::shapes::PolygonIndex;
use crate::world::World;
use crate::Job;
use anyhow::{bail, Result};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::collections::HashMap;

/// Chooses models by weight and builds their objects.
struct Picker<'a> {
    set: &'a ObjectSet,
    models: Vec<(u32, f32)>,
    total: f32,
}

impl<'a> Picker<'a> {
    fn new(w: &mut World, set: &'a ObjectSet) -> Result<Self> {
        let models: Vec<(u32, f32)> = set.models.iter().filter(|m| m.1 > 0.0 && !m.0.is_empty()).map(|(m, k)| (w.objects.intern(m), *k)).collect();
        if models.is_empty() {
            bail!("Choose at least one model");
        }
        let total = models.iter().map(|m| m.1).sum();
        Ok(Self { set, models, total })
    }
    fn model(&self, rng: &mut StdRng) -> u32 {
        let mut pick = rng.gen_range(0.0..self.total);
        for (m, k) in &self.models {
            pick -= k;
            if pick <= 0.0 {
                return *m;
            }
        }
        self.models[0].0
    }
    /// `along` turns the object to a direction instead of the set's own rotation.
    fn object(&self, w: &World, rng: &mut StdRng, model: u32, x: f32, z: f32, along: Option<f32>) -> Obj {
        let yaw = along.unwrap_or_else(|| if self.set.random_yaw { rng.gen_range(0.0..360.0) } else { 0.0 });
        let (pitch, roll) = if self.set.align { slope_angles(&w.height, x, z, yaw) } else { (0.0, 0.0) };
        let scale = if self.set.scale_max > self.set.scale_min { rng.gen_range(self.set.scale_min..=self.set.scale_max) } else { self.set.scale_min };
        Obj { x, z, elevation: 0.0, yaw, pitch, roll, scale, model, flags: 0 }
    }
}

/// Keeps objects a minimum distance apart.
struct Spacing {
    cell: f32,
    min2: f32,
    grid: HashMap<(i32, i32), Vec<[f32; 2]>>,
}

impl Spacing {
    fn new(min: f32) -> Self {
        Self { cell: min.max(0.25), min2: min * min, grid: HashMap::new() }
    }
    /// Takes the place when it is free.
    fn claim(&mut self, x: f32, z: f32) -> bool {
        let key = ((x / self.cell).floor() as i32, (z / self.cell).floor() as i32);
        for dz in -1..=1 {
            for dx in -1..=1 {
                if self.grid.get(&(key.0 + dx, key.1 + dz)).is_some_and(|c| c.iter().any(|p| (p[0] - x).powi(2) + (p[1] - z).powi(2) < self.min2)) {
                    return false;
                }
            }
        }
        self.grid.entry(key).or_default().push([x, z]);
        true
    }
}

/// Fills polygons, or the part of the map in `bounds` when there are none.
pub fn area_objects(w: &mut World, g: &AreaGen, area: Option<&PolygonIndex>, bounds: [f32; 4], job: &Job) -> Result<Vec<Obj>> {
    let picker = Picker::new(w, &g.objects)?;
    let w = &*w;
    let size = w.meta.size_m();
    let [x0, z0, x1, z1] = [bounds[0].max(0.0), bounds[1].max(0.0), bounds[2].min(size), bounds[3].min(size)];
    if x1 <= x0 || z1 <= z0 {
        return Ok(Vec::new());
    }
    let hectares = (x1 - x0) * (z1 - z0) / 10_000.0;
    let mut rng = StdRng::seed_from_u64(g.seed);
    let mut spacing = Spacing::new(g.min_distance);
    let sea = w.meta.sea_level;
    let usable = |x: f32, z: f32| {
        (x0..x1).contains(&x) && (z0..z1).contains(&z) && area.is_none_or(|a| a.contains(x, z)) && w.height.sample(x, z) >= sea && w.height.slope(x, z) <= g.max_slope
    };
    let mut out = Vec::new();
    // Objects scattered around one centre, within an ellipse turned by `angle`.
    let mut cluster = |rng: &mut StdRng, out: &mut Vec<Obj>, cx: f32, cz: f32, half_long: f32, half_wide: f32, angle: f32| {
        let (s, c) = angle.sin_cos();
        for _ in 0..rng.gen_range(1..=g.cluster_max.max(1)) {
            let (a, r) = (rng.gen_range(0.0..std::f32::consts::TAU), rng.gen_range(0.0f32..1.0).sqrt());
            let (u, v) = (a.cos() * r * half_long, a.sin() * r * half_wide);
            let (x, z) = (cx + u * c - v * s, cz + u * s + v * c);
            if usable(x, z) && spacing.claim(x, z) {
                let m = picker.model(rng);
                out.push(picker.object(w, rng, m, x, z, None));
            }
        }
    };
    let radius = |rng: &mut StdRng| if g.max_radius > g.min_radius { rng.gen_range(g.min_radius..=g.max_radius) } else { g.min_radius.max(0.5) };
    match g.method {
        AreaMethod::HighDensity => {
            // Every seed starts a patch of one model; a blend makes the borders uneven.
            let side = 100.0 / g.fraction.max(0.02).sqrt();
            let count = ((hectares * g.fraction.max(0.02)).ceil() as usize).max(1);
            let mut seeds: HashMap<(i32, i32), Vec<([f32; 2], u32)>> = HashMap::new();
            for _ in 0..count {
                let p = [rng.gen_range(x0..x1), rng.gen_range(z0..z1)];
                let m = picker.model(&mut rng);
                seeds.entry(((p[0] / side).floor() as i32, (p[1] / side).floor() as i32)).or_default().push((p, m));
            }
            let tries = (hectares * g.density) as usize;
            for n in 0..tries {
                if n % 8192 == 0 {
                    job.set_progress(n as f32 / tries.max(1) as f32);
                    job.check()?;
                }
                let (x, z) = (rng.gen_range(x0..x1), rng.gen_range(z0..z1));
                if !usable(x, z) || !spacing.claim(x, z) {
                    continue;
                }
                let key = ((x / side).floor() as i32, (z / side).floor() as i32);
                let mut best = (f32::MAX, None);
                for dz in -2..=2 {
                    for dx in -2..=2 {
                        for (p, m) in seeds.get(&(key.0 + dx, key.1 + dz)).map(Vec::as_slice).unwrap_or(&[]) {
                            let d = ((p[0] - x).powi(2) + (p[1] - z).powi(2)).sqrt() + rng.gen_range(0.0..g.blend.max(0.01));
                            if d < best.0 {
                                best = (d, Some(*m));
                            }
                        }
                    }
                }
                let m = best.1.unwrap_or_else(|| picker.model(&mut rng));
                out.push(picker.object(w, &mut rng, m, x, z, None));
            }
        }
        AreaMethod::LocalMaximum | AreaMethod::LocalMinimum => {
            // Sign chosen so "higher" always means "more of what is looked for".
            let sign = if g.method == AreaMethod::LocalMaximum { 1.0 } else { -1.0 };
            let reach = g.search_radius.max(w.height.cell * 2.0);
            let step = (reach / 3.0).max(w.height.cell);
            let rings: Vec<[f32; 2]> = [1.0f32, 0.5].iter().flat_map(|r| (0..8).map(move |k| { let a = k as f32 * std::f32::consts::FRAC_PI_4; [a.cos() * reach * r, a.sin() * reach * r] })).collect();
            let mut best: HashMap<(i32, i32), (f32, [f32; 2])> = HashMap::new();
            let mut z = z0;
            while z < z1 {
                job.set_progress((z - z0) / (z1 - z0));
                job.check()?;
                let mut x = x0;
                while x < x1 {
                    let h = sign * w.height.sample(x, z);
                    if usable(x, z) && rings.iter().all(|o| h > sign * w.height.sample(x + o[0], z + o[1]) + 0.15) {
                        let slot = best.entry(((x / reach).floor() as i32, (z / reach).floor() as i32)).or_insert((f32::MIN, [x, z]));
                        if h > slot.0 {
                            *slot = (h, [x, z]);
                        }
                    }
                    x += step;
                }
                z += step;
            }
            let mut found: Vec<[f32; 2]> = best.into_values().map(|v| v.1).collect();
            found.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
            for p in found {
                let r = radius(&mut rng);
                cluster(&mut rng, &mut out, p[0], p[1], r, r, 0.0);
            }
        }
        AreaMethod::LowDensityCircle | AreaMethod::LowDensityStrip => {
            let count = (hectares * g.cluster_density) as usize;
            for n in 0..count {
                if n % 4096 == 0 {
                    job.set_progress(n as f32 / count.max(1) as f32);
                    job.check()?;
                }
                let (cx, cz) = (rng.gen_range(x0..x1), rng.gen_range(z0..z1));
                let r = radius(&mut rng);
                if g.method == AreaMethod::LowDensityStrip {
                    let angle = rng.gen_range(0.0..std::f32::consts::PI);
                    cluster(&mut rng, &mut out, cx, cz, r, (g.strip_width * 0.5).max(0.25), angle);
                } else {
                    cluster(&mut rng, &mut out, cx, cz, r, r, 0.0);
                }
            }
        }
    }
    Ok(out)
}

/// A line measured along its length.
struct Path {
    points: Vec<[f32; 2]>,
    sums: Vec<f32>,
}

impl Path {
    fn new(points: Vec<[f32; 2]>) -> Self {
        let mut sums = vec![0.0];
        for s in points.windows(2) {
            sums.push(sums.last().unwrap() + ((s[1][0] - s[0][0]).powi(2) + (s[1][1] - s[0][1]).powi(2)).sqrt());
        }
        Self { points, sums }
    }
    fn length(&self) -> f32 {
        *self.sums.last().unwrap_or(&0.0)
    }
    /// The point `s` metres along, and the direction there.
    fn at(&self, s: f32) -> ([f32; 2], [f32; 2]) {
        let i = self.sums.partition_point(|v| *v <= s).clamp(1, self.points.len() - 1) - 1;
        let (a, b) = (self.points[i], self.points[i + 1]);
        let l = (self.sums[i + 1] - self.sums[i]).max(1e-4);
        let t = ((s - self.sums[i]) / l).clamp(0.0, 1.0);
        ([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t], [(b[0] - a[0]) / l, (b[1] - a[1]) / l])
    }
}

fn heading(dir: [f32; 2]) -> f32 {
    dir[0].atan2(dir[1]).to_degrees()
}

/// Random objects, clusters or node objects along lines.
pub fn line_objects(w: &mut World, g: &LineGen, lines: &[Vec<[f32; 2]>], job: &Job) -> Result<Vec<Obj>> {
    let picker = Picker::new(w, &g.objects)?;
    let w = &*w;
    let mut rng = StdRng::seed_from_u64(g.seed);
    let mut spacing = Spacing::new(g.min_distance);
    let size = w.meta.size_m();
    let inside = |x: f32, z: f32| (0.0..size).contains(&x) && (0.0..size).contains(&z);
    let mut out = Vec::new();
    // A random distance to either side of the line.
    let side = |rng: &mut StdRng| {
        let (lo, hi) = (g.inner_distance.max(0.0), g.max_distance.max(g.inner_distance.max(0.0)));
        let d = if hi > lo { rng.gen_range(lo..=hi) } else { lo };
        if rng.gen_bool(0.5) { d } else { -d }
    };
    for (k, points) in lines.iter().enumerate() {
        job.set_progress(k as f32 / lines.len().max(1) as f32);
        job.check()?;
        if g.method == LineMethod::Nodes {
            let nodes = prepare_line(points, &LineOptions { mode: LineMode::Polyline, offset: g.line.offset });
            for (i, p) in nodes.iter().enumerate() {
                if rng.gen_range(0.0..100.0f32) >= g.probability || !inside(p[0], p[1]) || !spacing.claim(p[0], p[1]) {
                    continue;
                }
                let (a, b) = (nodes[i.saturating_sub(1)], nodes[(i + 1).min(nodes.len() - 1)]);
                let l = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt().max(1e-4);
                let along = (!g.objects.random_yaw).then(|| heading([(b[0] - a[0]) / l, (b[1] - a[1]) / l]) + g.rotation);
                let m = picker.model(&mut rng);
                out.push(picker.object(w, &mut rng, m, p[0], p[1], along));
            }
            continue;
        }
        let path = Path::new(prepare_line(points, &g.line));
        if path.points.len() < 2 || path.length() < 0.1 {
            continue;
        }
        let strip = path.length() * 2.0 * g.max_distance.max(0.5) / 10_000.0;
        match g.method {
            LineMethod::Random => {
                let count = ((strip * g.density).ceil() as usize).max(1);
                for _ in 0..count {
                    let (p, d) = path.at(rng.gen_range(0.0..path.length()));
                    let o = side(&mut rng);
                    let (x, z) = (p[0] + d[1] * o, p[1] - d[0] * o);
                    if inside(x, z) && spacing.claim(x, z) {
                        let along = (!g.objects.random_yaw).then(|| heading(d) + g.rotation);
                        let m = picker.model(&mut rng);
                        out.push(picker.object(w, &mut rng, m, x, z, along));
                    }
                }
            }
            LineMethod::SparseCluster => {
                let count = ((strip * g.cluster_density).ceil() as usize).max(1);
                for _ in 0..count {
                    let (p, d) = path.at(rng.gen_range(0.0..path.length()));
                    let o = side(&mut rng);
                    let (cx, cz) = (p[0] + d[1] * o, p[1] - d[0] * o);
                    let r = if g.max_radius > g.min_radius { rng.gen_range(g.min_radius..=g.max_radius) } else { g.min_radius.max(0.5) };
                    for _ in 0..rng.gen_range(1..=g.cluster_max.max(1)) {
                        let (a, q) = (rng.gen_range(0.0..std::f32::consts::TAU), rng.gen_range(0.0f32..1.0).sqrt() * r);
                        let (x, z) = (cx + a.cos() * q, cz + a.sin() * q);
                        if inside(x, z) && spacing.claim(x, z) {
                            let m = picker.model(&mut rng);
                            out.push(picker.object(w, &mut rng, m, x, z, None));
                        }
                    }
                }
            }
            LineMethod::Nodes => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::{MapShape, ShapeKind, ShapeLayer};
    use crate::terrain::{Heightmap, MaskMap};
    use crate::world::WorldMeta;

    fn world() -> World {
        let mut base = Heightmap::new(128, 128, 4.0, 10.0);
        // One hill and one hollow.
        for j in 0..128 {
            for i in 0..128 {
                let hump = |cx: f32, cz: f32| (-((i as f32 - cx).powi(2) + (j as f32 - cz).powi(2)) / 60.0).exp();
                base.set(i, j, 10.0 + 20.0 * hump(40.0, 40.0) - 8.0 * hump(90.0, 90.0));
            }
        }
        let meta = WorldMeta::new("t", 128, 4.0);
        World::from_parts(meta, base, MaskMap::new(64, 64, 0), Vec::new(), None, Default::default(), Vec::new())
    }

    fn set() -> ObjectSet {
        ObjectSet { models: vec![("a.p3d".into(), 1.0), ("b.p3d".into(), 1.0)], ..Default::default() }
    }

    fn square(a: f32, b: f32) -> PolygonIndex {
        PolygonIndex::new(&ShapeLayer { name: String::new(), visible: true, shapes: vec![MapShape { kind: ShapeKind::Polygon, points: vec![[a, a], [b, a], [b, b], [a, b]], attributes: Vec::new() }] })
    }

    #[test]
    fn clusters_gather_at_the_hill_and_in_the_hollow() {
        let job = Job::default();
        for (method, centre) in [(AreaMethod::LocalMaximum, 160.0), (AreaMethod::LocalMinimum, 360.0)] {
            let g = AreaGen { method, objects: set(), search_radius: 40.0, cluster_max: 8, min_radius: 6.0, max_radius: 6.0, min_distance: 0.5, ..Default::default() };
            let out = area_objects(&mut world(), &g, None, [0.0, 0.0, 512.0, 512.0], &job).unwrap();
            assert!(!out.is_empty());
            assert!(out.iter().all(|o| (o.x - centre).abs() < 30.0 && (o.z - centre).abs() < 30.0), "{method:?}");
        }
    }

    #[test]
    fn nothing_leaves_the_polygon() {
        let job = Job::default();
        let area = square(100.0, 300.0);
        for method in [AreaMethod::HighDensity, AreaMethod::LowDensityCircle, AreaMethod::LowDensityStrip] {
            let g = AreaGen { method, objects: set(), density: 200.0, cluster_density: 20.0, ..Default::default() };
            let out = area_objects(&mut world(), &g, Some(&area), [100.0, 100.0, 300.0, 300.0], &job).unwrap();
            assert!(!out.is_empty(), "{method:?}");
            assert!(out.iter().all(|o| (100.0..=300.0).contains(&o.x) && (100.0..=300.0).contains(&o.z)));
        }
    }

    #[test]
    fn high_density_makes_patches_of_one_model() {
        let job = Job::default();
        let g = AreaGen { method: AreaMethod::HighDensity, objects: set(), density: 400.0, fraction: 4.0, blend: 0.1, min_distance: 1.0, ..Default::default() };
        let out = area_objects(&mut world(), &g, None, [0.0, 0.0, 512.0, 512.0], &job).unwrap();
        let mut same = 0;
        for a in out.iter().take(200) {
            let n = out.iter().filter(|b| b.x != a.x).min_by(|p, q| ((p.x - a.x).powi(2) + (p.z - a.z).powi(2)).total_cmp(&((q.x - a.x).powi(2) + (q.z - a.z).powi(2)))).unwrap();
            same += usize::from(n.model == a.model);
        }
        assert!(same > 140, "{same}");
    }

    #[test]
    fn line_objects_stay_in_their_strip() {
        let job = Job::default();
        let g = LineGen { method: LineMethod::Random, objects: set(), density: 200.0, inner_distance: 3.0, max_distance: 8.0, min_distance: 0.5, ..Default::default() };
        let out = line_objects(&mut world(), &g, &[vec![[50.0, 200.0], [450.0, 200.0]]], &job).unwrap();
        assert!(out.len() > 50);
        assert!(out.iter().all(|o| (3.0..=8.01).contains(&(o.z - 200.0).abs())));
    }

    #[test]
    fn node_objects_sit_on_the_nodes() {
        let job = Job::default();
        let line = vec![[50.0, 50.0], [100.0, 50.0], [100.0, 120.0]];
        let g = LineGen { method: LineMethod::Nodes, objects: set(), probability: 100.0, ..Default::default() };
        let out = line_objects(&mut world(), &g, &[line.clone()], &job).unwrap();
        assert_eq!(out.len(), 3);
        for (o, p) in out.iter().zip(&line) {
            assert_eq!([o.x, o.z], *p);
        }
    }
}
