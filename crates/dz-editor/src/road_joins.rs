//! Closes the small gaps left between road parts that nearly meet.
//!
//! Parts imported from a map or set by hand often stop a few centimetres short of the next
//! one, and the terrain shows through as a dark line across the road. Straight parts are
//! drawn stretched along their length until their ends meet: each of two facing ends goes
//! half the way.
use std::collections::HashMap;

use dz_core::model_info::ModelInfo;
use dz_core::objects::{rotation, ObjectStore};

/// Gaps up to this long are closed, m.
const MAX_GAP: f32 = 1.5;
/// How far to the side the facing end may lie, m.
const MAX_SIDE: f32 = 0.75;

#[derive(Default)]
pub struct RoadJoins {
    rev: u64,
    models: usize,
    /// How far each straight part is drawn past its start and its end, m.
    ext: HashMap<u32, [f32; 2]>,
}

/// A straight piece of road, which can be drawn longer than it is without looking wrong.
fn is_straight(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    if !p.contains("\\roads\\parts\\") {
        return false;
    }
    let file = p.rsplit('\\').next().unwrap_or(&p);
    !file.contains(' ') && !file.starts_with("kr_") && !file.contains("curve") && !file.contains("_poj_")
}

struct End {
    id: u32,
    which: usize,
    at: [f32; 2],
    out: [f32; 2],
}

impl RoadJoins {
    pub fn update(&mut self, objects: &ObjectStore, info: impl Fn(&str) -> ModelInfo) {
        if self.rev == objects.revision && self.models == objects.models.len() {
            return;
        }
        self.rev = objects.revision;
        self.models = objects.models.len();
        self.ext.clear();
        let straight: Vec<Option<ModelInfo>> = objects.models.iter().map(|m| is_straight(&m.path).then(|| info(&m.path)).filter(|i| i.known && i.max[2] - i.min[2] > 0.5)).collect();
        if straight.iter().all(Option::is_none) {
            return;
        }
        let mut ends = Vec::new();
        for (id, o) in objects.iter() {
            let Some(i) = straight.get(o.model as usize).copied().flatten() else { continue };
            let m = rotation(o.yaw, o.pitch, o.roll, o.scale);
            let cx = (i.min[0] + i.max[0]) * 0.5;
            let along = [m[2][0], m[2][2]];
            let n = along[0].hypot(along[1]).max(1e-6);
            let dir = [along[0] / n, along[1] / n];
            for (which, z, sign) in [(0, i.min[2], -1.0), (1, i.max[2], 1.0)] {
                let at = [o.x + m[0][0] * cx + m[2][0] * z, o.z + m[0][2] * cx + m[2][2] * z];
                ends.push(End { id, which, at, out: [dir[0] * sign, dir[1] * sign] });
            }
        }
        let cell = |p: [f32; 2]| ((p[0] / MAX_GAP).floor() as i32, (p[1] / MAX_GAP).floor() as i32);
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (k, e) in ends.iter().enumerate() {
            grid.entry(cell(e.at)).or_default().push(k);
        }
        for e in &ends {
            let c = cell(e.at);
            let mut best: Option<f32> = None;
            for gz in c.1 - 1..=c.1 + 1 {
                for gx in c.0 - 1..=c.0 + 1 {
                    for &k in grid.get(&(gx, gz)).into_iter().flatten() {
                        let f = &ends[k];
                        if f.id == e.id || f.out[0] * e.out[0] + f.out[1] * e.out[1] > -0.95 {
                            continue;
                        }
                        let d = [f.at[0] - e.at[0], f.at[1] - e.at[1]];
                        let gap = d[0] * e.out[0] + d[1] * e.out[1];
                        let side = (d[0] * e.out[1] - d[1] * e.out[0]).abs();
                        if gap > 0.005 && gap < MAX_GAP && side < MAX_SIDE && best.is_none_or(|b| gap < b) {
                            best = Some(gap);
                        }
                    }
                }
            }
            if let Some(gap) = best {
                self.ext.entry(e.id).or_default()[e.which] = gap * 0.5;
            }
        }
    }

    /// The matrix and position object `id` is drawn with, stretched to meet its neighbours.
    pub fn apply(&self, id: u32, info: &ModelInfo, m: [[f32; 3]; 3], at: [f32; 3]) -> ([[f32; 3]; 3], [f32; 3]) {
        let Some(&[e0, e1]) = self.ext.get(&id) else { return (m, at) };
        let unit = (m[2][0] * m[2][0] + m[2][1] * m[2][1] + m[2][2] * m[2][2]).sqrt().max(1e-6);
        let (e0, e1) = (e0 / unit, e1 / unit);
        let len = (info.max[2] - info.min[2]).max(1e-3);
        let s = (len + e0 + e1) / len;
        let shift = info.min[2] * (1.0 - s) - e0;
        let mut out = m;
        out[2] = [m[2][0] * s, m[2][1] * s, m[2][2] * s];
        (out, [at[0] + m[2][0] * shift, at[1] + m[2][1] * shift, at[2] + m[2][2] * shift])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dz_core::objects::Obj;

    fn part(objects: &mut ObjectStore, path: &str, x: f32, z: f32, yaw: f32) -> u32 {
        let model = objects.intern(path);
        objects.insert(Obj { x, z, elevation: 0.0, yaw, pitch: 0.0, roll: 0.0, scale: 1.0, model, flags: 0 })
    }

    #[test]
    fn facing_ends_meet_halfway_and_far_ones_stay() {
        let info = ModelInfo { min: [-3.0, 0.0, -3.0], max: [3.0, 0.2, 3.0], known: true, ..Default::default() };
        let mut objects = ObjectStore::default();
        // Two 6 m parts along z with a 0.3 m gap, a third 5 m away, and a curve next to them.
        let a = part(&mut objects, r"dz\structures\roads\parts\asf1_6.p3d", 0.0, 0.0, 0.0);
        let b = part(&mut objects, r"dz\structures\roads\parts\asf1_6.p3d", 0.0, 6.3, 0.0);
        let c = part(&mut objects, r"dz\structures\roads\parts\asf1_6.p3d", 0.0, 17.3, 0.0);
        let d = part(&mut objects, r"dz\structures\roads\parts\asf1_10 100.p3d", 0.0, -6.2, 0.0);
        let mut joins = RoadJoins::default();
        joins.update(&objects, |_| info);
        assert!((joins.ext[&a][1] - 0.15).abs() < 1e-4, "{:?}", joins.ext);
        assert!((joins.ext[&b][0] - 0.15).abs() < 1e-4);
        assert!(!joins.ext.contains_key(&c));
        assert!(!joins.ext.contains_key(&d));
        // Drawn stretched: the end of `a` now reaches z = 3.15 and its start stays at -3.
        let (m, at) = joins.apply(a, &info, rotation(0.0, 0.0, 0.0, 1.0), [0.0, 0.0, 0.0]);
        let end = at[2] + m[2][2] * info.max[2];
        let start = at[2] + m[2][2] * info.min[2];
        assert!((end - 3.15).abs() < 1e-4 && (start + 3.0).abs() < 1e-4, "{start} {end}");
    }

    #[test]
    fn turned_parts_meet_too() {
        let info = ModelInfo { min: [-3.0, 0.0, -3.0], max: [3.0, 0.2, 3.0], known: true, ..Default::default() };
        let mut objects = ObjectStore::default();
        // Along x (yaw 90), facing each other the other way round.
        let a = part(&mut objects, r"dz\structures\roads\parts\asf1_6.p3d", 0.0, 0.0, 90.0);
        let b = part(&mut objects, r"dz\structures\roads\parts\asf1_6.p3d", 6.4, 0.0, 270.0);
        let mut joins = RoadJoins::default();
        joins.update(&objects, |_| info);
        assert!((joins.ext[&a][1] - 0.2).abs() < 1e-4, "{:?}", joins.ext);
        assert!((joins.ext[&b][1] - 0.2).abs() < 1e-4, "{:?}", joins.ext);
    }
}
