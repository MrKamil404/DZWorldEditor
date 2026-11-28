//! Generators that dress what is already on the map: wrecks and decals on the roads, and the
//! things that stand round buildings (woodpiles, bins, pallets, sandbags...).
//!
//! Generators work on a copy of the world without its objects, so the roads and buildings
//! they dress are handed to them.
use crate::check::{contains, is_house};
use crate::gen::slope_angles;
use crate::landscape::Follow;
use crate::library::ObjectSet;
use crate::model_info::{ModelDb, ModelInfo};
use crate::modules::{Picker, Spacing};
use crate::objects::Obj;
use crate::shapes::PolygonIndex;
use crate::world::World;
use crate::Job;
use anyhow::Result;
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Models of the game in one folder, each as likely as the others.
fn models(folder: &str, names: &[&str]) -> Vec<(String, f32)> {
    names.iter().map(|n| (format!("dz\\structures\\{folder}\\{n}.p3d"), 1.0)).collect()
}

/// The point `s` metres along a line, with its direction there.
fn point_at(line: &[[f32; 2]], s: f32) -> Option<([f32; 2], [f32; 2])> {
    let mut left = s;
    for seg in line.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        if len <= 1e-3 {
            continue;
        }
        let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        if left <= len {
            return Some(([a[0] + dir[0] * left, a[1] + dir[1] * left], dir));
        }
        left -= len;
    }
    None
}

fn length(line: &[[f32; 2]]) -> f32 {
    line.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum()
}

/// A count drawn so that its average is `mean`.
fn count(rng: &mut StdRng, mean: f32) -> usize {
    mean.floor() as usize + (rng.gen::<f32>() < mean.fract()) as usize
}

// ---------------------------------------------------------------- wrecks

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WreckGen {
    pub wrecks: ObjectSet,
    /// Places with wrecks per kilometre of road.
    pub scenes_per_km: f32,
    /// Shares of those places that are a jam of several cars in a lane, or a crash of two or
    /// three; the rest are single cars left by the road.
    pub jam_share: f32,
    pub crash_share: f32,
    /// Most cars in a jam.
    pub jam_max: u32,
    /// Metres from the middle of the road to the middle of a lane, and to its edge.
    pub lane: f32,
    pub road_half: f32,
    /// Degrees every wreck is turned by, for models whose front is not where the generator
    /// expects it (along +z, the way the traffic of its lane goes).
    pub wreck_turn: f32,
    /// Most degrees a car of a jam, and a car left on the verge, is turned off the road at
    /// random, either way.
    pub jam_turn: f32,
    pub verge_turn: f32,
    /// Stains laid under the wrecks (oil, burns), and the share of wrecks that get one.
    pub under: Vec<(String, f32)>,
    pub under_share: f32,
    /// Cracks, patches and tyre marks on the road, per kilometre.
    pub decals: ObjectSet,
    pub decals_per_km: f32,
    /// Most metres from the middle of the road to the middle of a decal.
    pub decal_reach: f32,
    /// A decal stays inside the road: wide ones keep nearer the middle, as far as their own
    /// half width allows.
    pub decals_inside: bool,
    /// Decals only on paved roads, not on gravel and dirt tracks.
    pub decals_paved_only: bool,
    /// Which roads it follows when it works on the whole map.
    pub follow: Follow,
    pub seed: u64,
}

impl Default for WreckGen {
    fn default() -> Self {
        let cars = models("wrecks\\vehicles", &[
            "wreck_sed01_aban1_black", "wreck_sed01_aban2_white", "wreck_sed02_aban1_grey", "wreck_sed02_aban2_red", "wreck_hb01_aban1_blue", "wreck_hb01_aban2_green",
            "wreck_hb02_aban1_red", "wreck_offroad02_aban1", "wreck_truck01_aban1_blue", "wreck_volha_grey", "wreck_lada_red", "wreck_s1023_blue", "wreck_uaz", "wreck_v3s",
        ]);
        let decals = models("roads\\decals", &[
            "decal_damage_long1", "decal_damage_long2", "decal_damage_long3", "decal_damage_medium1", "decal_damage_medium2", "decal_damage_small1", "decal_damage_small2","decal_manhole",
        ]);
        Self {
            wrecks: ObjectSet { models: cars, random_yaw: false, scale_min: 1.0, scale_max: 1.0, align: true, ..Default::default() },
            scenes_per_km: 1.5, jam_share: 0.2, crash_share: 0.3, jam_max: 7, lane: 2.0, road_half: 3.5, wreck_turn: 0.0, jam_turn: 6.0, verge_turn: 15.0,
            under: models("wrecks\\vehicles", &["wreck_decal_small1", "wreck_decal_small2", "wreck_decal_big"]), under_share: 0.7,
            decals: ObjectSet { models: decals, random_yaw: false, scale_min: 0.9, scale_max: 1.1, align: true, ..Default::default() },
            decals_per_km: 12.0, decal_reach: 2.5, decals_inside: true, decals_paved_only: true, follow: Follow::Roads, seed: 1,
        }
    }
}

/// Whether a road of `family` has a hard surface, as asphalt or concrete; tracks of gravel, dirt
/// or mud do not.
pub fn is_paved(family: &str) -> bool {
    let f = family.to_ascii_lowercase();
    !["grav", "dirt", "mud", "path", "track", "forest", "field", "sand", "ground"].iter().any(|k| f.contains(k))
}

/// Wrecks in scenes along the roads, stains under them, and decals on the road. `paved` says
/// for each line whether it is a paved road (missing ones are). `db` gives the sizes of the
/// decals, so they can be kept inside the road.
pub fn wreck_objects(w: &mut World, db: &ModelDb, g: &WreckGen, lines: &[Vec<[f32; 2]>], paved: &[bool], job: &Job) -> Result<Vec<Obj>> {
    let mut rng = StdRng::seed_from_u64(g.seed);
    let size = w.meta.size_m();
    let inside = |p: [f32; 2]| p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= size && p[1] <= size;
    let mut out = Vec::new();
    let heading = |dir: [f32; 2]| dir[0].atan2(dir[1]).to_degrees();
    let has = |s: &ObjectSet| !s.models.is_empty() || s.preset.is_some();
    let under_set = ObjectSet { models: g.under.clone(), random_yaw: false, scale_min: 1.0, scale_max: 1.0, align: true, ..Default::default() };
    let mut spacing = Spacing::new(4.0);
    if has(&g.wrecks) && g.scenes_per_km > 0.0 {
        let picker = Picker::new(w, &g.wrecks)?;
        let under = (!g.under.is_empty()).then(|| Picker::new(w, &under_set)).transpose()?;
        for (k, line) in lines.iter().enumerate() {
            job.check()?;
            job.set_progress(k as f32 / lines.len().max(1) as f32 * 0.7);
            let len = length(line);
            if len < 60.0 {
                continue;
            }
            for _ in 0..count(&mut rng, len / 1000.0 * g.scenes_per_km) {
                let s = rng.gen_range(30.0..len - 30.0);
                // Which way the traffic of the scene went, and its lane.
                let back = rng.gen::<bool>();
                let side = if back { -1.0 } else { 1.0 };
                let roll = rng.gen::<f32>();
                // (metres along, metres across, turn away from the lane) of each car.
                let cars: Vec<(f32, f32, f32)> = if roll < g.jam_share {
                    let n = rng.gen_range(3..=g.jam_max.max(3));
                    let mut at = 0.0;
                    (0..n).map(|_| {
                        at += rng.gen_range(6.0..8.5);
                        (at * -side, g.lane * side + rng.gen_range(-0.4..0.4), rng.gen_range(-g.jam_turn.abs()..=g.jam_turn.abs()))
                    }).collect()
                } else if roll < g.jam_share + g.crash_share {
                    let n = rng.gen_range(2..=3);
                    (0..n).map(|_| (rng.gen_range(-5.0..5.0), rng.gen_range(-g.road_half..g.road_half), rng.gen_range(-180.0..180.0))).collect()
                } else {
                    // Left on the verge, more or less along the road.
                    vec![(0.0, (g.road_half + 1.0) * side, rng.gen_range(-g.verge_turn.abs()..=g.verge_turn.abs()))]
                };
                for (along, across, turn) in cars {
                    let Some((p, dir)) = point_at(line, (s + along).clamp(0.0, len)) else { continue };
                    let (x, z) = (p[0] + dir[1] * across, p[1] - dir[0] * across);
                    if !inside([x, z]) || !spacing.claim(x, z) {
                        continue;
                    }
                    let yaw = heading(dir) + if back { 180.0 } else { 0.0 } + turn + g.wreck_turn;
                    let model = picker.model(&mut rng);
                    out.push(picker.object(w, &mut rng, model, x, z, Some(yaw)));
                    if let Some(u) = &under {
                        if rng.gen::<f32>() < g.under_share {
                            let model = u.model(&mut rng);
                            out.push(u.object(w, &mut rng, model, x, z, Some(yaw)));
                        }
                    }
                }
            }
        }
    }
    if has(&g.decals) && g.decals_per_km > 0.0 {
        let picker = Picker::new(w, &g.decals)?;
        for (k, line) in lines.iter().enumerate() {
            if g.decals_paved_only && !paved.get(k).copied().unwrap_or(true) {
                continue;
            }
            let len = length(line);
            for _ in 0..count(&mut rng, len / 1000.0 * g.decals_per_km) {
                let Some((p, dir)) = point_at(line, rng.gen_range(0.0..len.max(0.01))) else { continue };
                let model = picker.model(&mut rng);
                // Laid along the road, a decal's own x runs across it.
                let half = if g.decals_inside {
                    let i = db.get(&w.objects.models[model as usize].path);
                    if i.known { i.min[0].abs().max(i.max[0].abs()) * g.decals.scale_max.max(g.decals.scale_min) } else { 0.0 }
                } else {
                    0.0
                };
                let reach = if g.decals_inside { g.decal_reach.min(g.road_half - half) } else { g.decal_reach }.max(0.0);
                let across = rng.gen_range(-reach..=reach);
                let (x, z) = (p[0] + dir[1] * across, p[1] - dir[0] * across);
                if !inside([x, z]) {
                    continue;
                }
                let yaw = heading(dir) + if rng.gen() { 0.0 } else { 180.0 };
                let mut o = picker.object(w, &mut rng, model, x, z, Some(yaw));
                if g.decals.align {
                    (o.pitch, o.roll) = slope_angles(&w.height, x, z, yaw);
                }
                out.push(o);
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- round buildings

/// What kind of place a building belongs to, by where its model is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuildingKind {
    Residential,
    Industrial,
    Military,
    Farm,
}

pub fn building_kind(path: &str) -> Option<BuildingKind> {
    let p = path.to_ascii_lowercase();
    if p.contains("\\military\\") {
        Some(BuildingKind::Military)
    } else if ["farm", "barn", "cowshed", "stable"].iter().any(|k| p.contains(k)) {
        Some(BuildingKind::Farm)
    } else if p.contains("\\industrial\\") || p.contains("\\rail\\") {
        Some(BuildingKind::Industrial)
    } else if p.contains("\\residential\\") || p.contains("house") {
        Some(BuildingKind::Residential)
    } else {
        None
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DressGen {
    /// What stands round houses, works, military buildings and farms.
    pub residential: ObjectSet,
    pub industrial: ObjectSet,
    pub military: ObjectSet,
    pub farm: ObjectSet,
    /// How many things each building gets, from and to.
    pub min_count: u32,
    pub max_count: u32,
    /// The share of things set flush against a wall, along it; the rest stand free up to
    /// `max_distance` metres from the walls.
    pub against_wall: f32,
    pub max_distance: f32,
    /// Nothing within this many metres of the middle of a road.
    pub keep_from_roads: f32,
    pub seed: u64,
}

impl Default for DressGen {
    fn default() -> Self {
        let set = |models: Vec<(String, f32)>| ObjectSet { models, random_yaw: true, scale_min: 0.95, scale_max: 1.05, align: true, ..Default::default() };
        Self {
            residential: set(models("residential\\misc", &["misc_woodpile1", "misc_woodpile2", "misc_woodreserve", "garbage_bin", "misc_boxwooden", "misc_doghouse", "misc_hutch", "misc_woodblock", "misc_woodtable_outdoor"])),
            industrial: set([models("industrial\\misc", &["misc_barrelempty", "misc_pallet", "misc_pallets1", "misc_pallets2", "misc_tirepile", "misc_tire_big", "misc_boardspack1", "misc_gastank_01_blue", "misc_coil", "misc_timbers1"]), models("residential\\misc", &["garbage_container"])].concat()),
            military: set([models("military\\misc", &["misc_bagfence_3m", "misc_supplybox1", "misc_supplybox2", "misc_supplybox3", "misc_concreteblock1", "misc_cover_small"]), models("industrial\\misc", &["misc_barrelempty"])].concat()),
            farm: set([models("industrial\\misc", &["misc_haybale", "misc_timbers_log1", "misc_timbers_log2", "misc_woodpile_forest1", "misc_boardspack2"]), models("residential\\misc", &["misc_haystack", "misc_feedrack", "misc_woodpile1"])].concat()),
            min_count: 1, max_count: 4, against_wall: 0.6, max_distance: 4.0, keep_from_roads: 6.0, seed: 1,
        }
    }
}

impl DressGen {
    pub fn set(&self, kind: BuildingKind) -> &ObjectSet {
        match kind {
            BuildingKind::Residential => &self.residential,
            BuildingKind::Industrial => &self.industrial,
            BuildingKind::Military => &self.military,
            BuildingKind::Farm => &self.farm,
        }
    }
}

/// The buildings of a world that can be dressed, with their models, inside `area`.
pub fn buildings(w: &World, db: &ModelDb, area: Option<&PolygonIndex>) -> Vec<(Obj, String)> {
    w.objects.iter().filter(|(_, o)| w.object_exported(o) && area.is_none_or(|a| a.contains(o.x, o.z))).filter_map(|(_, o)| {
        let path = w.objects.model_path(o);
        let info = db.get(path);
        (crate::world::object_group(path) == Some(1) && is_house(path, &info) && building_kind(path).is_some()).then(|| (*o, path.to_string()))
    }).collect()
}

/// Things round the buildings: against their walls, along them, or standing free nearby; never
/// inside another building, on a road, or on top of what is already there (`taken`).
pub fn dress_objects(w: &mut World, db: &ModelDb, g: &DressGen, buildings: &[(Obj, String)], taken: &[[f32; 2]], roads: &[Vec<[f32; 2]>], job: &Job) -> Result<Vec<Obj>> {
    let mut rng = StdRng::seed_from_u64(g.seed);
    let kinds = [BuildingKind::Residential, BuildingKind::Industrial, BuildingKind::Military, BuildingKind::Farm];
    let mut pickers = HashMap::new();
    for k in kinds {
        let s = g.set(k);
        if !s.models.is_empty() || s.preset.is_some() {
            pickers.insert(k, (Picker::new(w, s)?, s.random_yaw));
        }
    }
    let infos: Vec<ModelInfo> = buildings.iter().map(|(_, p)| db.get(p)).collect();
    // Buildings by place, so a thing can be kept out of all of them.
    const CELL: f32 = 40.0;
    let key = |x: f32, z: f32| ((x / CELL).floor() as i32, (z / CELL).floor() as i32);
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (k, (o, _)) in buildings.iter().enumerate() {
        grid.entry(key(o.x, o.z)).or_default().push(k);
    }
    let in_building = |p: [f32; 2]| {
        let (cx, cz) = key(p[0], p[1]);
        (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (cx + dx, cz + dz))).filter_map(|c| grid.get(&c)).flatten().any(|&k| contains(&buildings[k].0, &infos[k], p, 0.3))
    };
    let near_road = |x: f32, z: f32| {
        roads.iter().any(|r| r.windows(2).any(|s| {
            let (a, b) = (s[0], s[1]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let t = (((x - a[0]) * dx + (z - a[1]) * dz) / (dx * dx + dz * dz).max(1e-6)).clamp(0.0, 1.0);
            (x - a[0] - dx * t).hypot(z - a[1] - dz * t) < g.keep_from_roads
        }))
    };
    let mut spacing = Spacing::new(1.4);
    for p in taken {
        spacing.claim(p[0], p[1]);
    }
    let size = w.meta.size_m();
    let mut out = Vec::new();
    for (k, ((b, path), info)) in buildings.iter().zip(&infos).enumerate() {
        if k % 256 == 0 {
            job.check()?;
            job.set_progress(k as f32 / buildings.len().max(1) as f32);
        }
        let Some((picker, random_yaw)) = building_kind(path).and_then(|kind| pickers.get(&kind)) else { continue };
        let (s, c) = b.yaw.to_radians().sin_cos();
        let to_world = |lx: f32, lz: f32| [b.x + lx * c * b.scale + lz * s * b.scale, b.z - lx * s * b.scale + lz * c * b.scale];
        let (x0, x1, z0, z1) = (info.min[0], info.max[0], info.min[2], info.max[2]);
        let n = rng.gen_range(g.min_count..=g.max_count.max(g.min_count));
        let mut tries = 0;
        let mut placed = 0;
        while placed < n && tries < n * 8 {
            tries += 1;
            // A wall, by its length: 0 the +z side, 1 the +x side, 2 the -z side, 3 the -x side.
            let (wx, wz) = (x1 - x0, z1 - z0);
            let pick = rng.gen_range(0.0..(2.0 * (wx + wz)).max(0.01));
            let side = if pick < wx { 0 } else if pick < wx + wz { 1 } else if pick < 2.0 * wx + wz { 2 } else { 3 };
            let model = picker.model(&mut rng);
            let pinfo = db.get(&w.objects.models[model as usize].path);
            let (phx, phz) = if pinfo.known { pinfo.footprint() } else { (0.5, 0.5) };
            let flush = rng.gen::<f32>() < g.against_wall;
            // Out from the wall: touching it with its back, or standing free.
            let out_m = if flush { phz + 0.1 } else { phz.max(phx) + rng.gen_range(0.6..g.max_distance.max(0.7)) };
            let along_margin = phx + 0.5;
            let along = |lo: f32, hi: f32, rng: &mut StdRng| if hi - lo > 2.0 * along_margin { rng.gen_range(lo + along_margin..hi - along_margin) } else { (lo + hi) * 0.5 };
            let (lx, lz, wall_yaw) = match side {
                0 => (along(x0, x1, &mut rng), z1 + out_m, 0.0),
                1 => (x1 + out_m, along(z0, z1, &mut rng), 90.0),
                2 => (along(x0, x1, &mut rng), z0 - out_m, 180.0),
                _ => (x0 - out_m, along(z0, z1, &mut rng), 270.0),
            };
            let p = to_world(lx, lz);
            if p[0] < 0.0 || p[1] < 0.0 || p[0] > size || p[1] > size || in_building(p) || near_road(p[0], p[1]) || w.height.slope(p[0], p[1]) > 25.0 || !spacing.claim(p[0], p[1]) {
                continue;
            }
            // Flush things run along the wall with their backs to it; free ones as their set says.
            let yaw = if flush || !*random_yaw { b.yaw + wall_yaw } else { rng.gen_range(0.0..360.0) };
            out.push(picker.object(w, &mut rng, model, p[0], p[1], Some(yaw)));
            placed += 1;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- roadblocks and checkpoints

/// Kind of roadblock placed across a road.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoadblockStyle {
    Military,
    Civilian,
    Mixed,
}

impl Default for RoadblockStyle {
    fn default() -> Self {
        Self::Military
    }
}

/// Roadblocks and checkpoints aligned automatically to the road direction.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RoadblockGen {
    pub style: RoadblockStyle,
    /// Metres between checkpoints along a road (0 = a single checkpoint at the middle of each line).
    pub spacing: f32,
    /// Half width of the road in metres: booths and sandbags stand outside this on the verge.
    pub road_half: f32,
    /// Place a guard booth on the verge (military style).
    pub add_booth: bool,
    /// Place barbed wire and iron hedgehogs.
    pub add_wire: bool,
    /// Place abandoned police/civilian wrecks at the block.
    pub add_wrecks: bool,
    /// Which roads it follows when dropped on the whole map.
    pub follow: Follow,
    pub seed: u64,
}

impl Default for RoadblockGen {
    fn default() -> Self {
        Self {
            style: RoadblockStyle::Military,
            spacing: 0.0,
            road_half: 3.8,
            add_booth: true,
            add_wire: true,
            add_wrecks: true,
            follow: Follow::Roads,
            seed: 1,
        }
    }
}

/// Places roadblocks along `lines`, automatically aligned to each road's direction.
pub fn roadblock_objects(w: &mut World, g: &RoadblockGen, lines: &[Vec<[f32; 2]>], job: &Job) -> Result<Vec<Obj>> {
    let mut rng = StdRng::seed_from_u64(g.seed.wrapping_mul(0x9e3779b97f4a7c15) ^ 0x40adb10c);
    let size = w.meta.size_m();
    let sea = w.meta.sea_level;
    let mut out = Vec::new();
    let mut spacing = Spacing::new(30.0);

    // Helper: place a model at local (right, forward) offset relative to road center & direction.
    let place_rel = |w: &mut World, out: &mut Vec<Obj>, pos: [f32; 2], dir: [f32; 2], right_m: f32, fwd_m: f32, local_yaw: f32, align: bool, model: &str| {
        // dir is forward (+Z in local frame); right is [dir[1], -dir[0]] (+X in local frame).
        let x = pos[0] + dir[1] * right_m + dir[0] * fwd_m;
        let z = pos[1] - dir[0] * right_m + dir[1] * fwd_m;
        if x < 0.0 || z < 0.0 || x > size || z > size || w.height.sample(x, z) < sea {
            return;
        }
        let road_yaw = dir[0].atan2(dir[1]).to_degrees();
        let yaw = (road_yaw + local_yaw).rem_euclid(360.0);
        let (pitch, roll) = if align { slope_angles(&w.height, x, z, yaw) } else { (0.0, 0.0) };
        let m = w.objects.intern(model);
        out.push(Obj { x, z, elevation: 0.0, yaw, pitch, roll, scale: 1.0, model: m, flags: 0 });
    };

    for line in lines {
        if job.cancelled() || line.len() < 2 {
            continue;
        }
        let total = length(line);
        if total < 12.0 {
            continue;
        }
        let stations: Vec<f32> = if g.spacing <= 0.0 {
            vec![total * 0.5]
        } else {
            let mut s = (g.spacing * 0.5).min(total * 0.5);
            let mut v = Vec::new();
            while s < total - 15.0 {
                v.push(s);
                s += g.spacing;
            }
            if v.is_empty() && total >= 30.0 {
                v.push(total * 0.5);
            }
            v
        };
        for s in stations {
            let Some((pos, dir)) = point_at(line, s) else { continue };
            if !spacing.claim(pos[0], pos[1]) {
                continue;
            }
            let military = match g.style {
                RoadblockStyle::Military => true,
                RoadblockStyle::Civilian => false,
                RoadblockStyle::Mixed => rng.gen_bool(0.5),
            };
            let flip = if rng.gen_bool(0.5) { 1.0f32 } else { -1.0f32 };
            let rh = g.road_half.max(2.0);
            if military {
                // Guard booth on one verge facing the road, flanked by sandbags & barrel.
                if g.add_booth {
                    place_rel(w, &mut out, pos, dir, -flip * (rh + 0.4), 0.0, if flip > 0.0 { 90.0 } else { 270.0 }, false, r"dz\structures\military\improvised\Mil_GuardShed.p3d");
                    place_rel(w, &mut out, pos, dir, -flip * (rh + 0.4), -2.5, 0.0, true, r"dz\structures\military\misc\Misc_BagFence_3m.p3d");
                    place_rel(w, &mut out, pos, dir, -flip * (rh + 0.4), 2.5, 0.0, true, r"dz\structures\military\misc\Misc_BagFence_3m.p3d");
                    place_rel(w, &mut out, pos, dir, -flip * (rh - 1.1), -1.8, 0.0, true, r"dz\structures\industrial\misc\misc_barrelempty.p3d");
                }
                // Boom barrier reaching across the road from the booth side.
                place_rel(w, &mut out, pos, dir, -flip * (rh - 2.2), 0.0, if flip > 0.0 { 90.0 } else { 270.0 }, true, r"dz\structures\residential\misc\Misc_RoadBarrier.p3d");
                // Sandbags & crates on the opposite verge.
                place_rel(w, &mut out, pos, dir, flip * (rh + 0.4), 1.4, 0.0, true, r"dz\structures\military\misc\Misc_BagFence_3m.p3d");
                place_rel(w, &mut out, pos, dir, flip * (rh + 1.4), -2.2, rng.gen_range(-15.0..15.0), true, r"dz\structures\military\misc\Misc_SupplyBox1.p3d");
                if g.add_wire {
                    place_rel(w, &mut out, pos, dir, flip * (rh + 0.6), -1.5, 35.0, true, r"dz\structures\military\misc\Misc_Hedgehog_Iron.p3d");
                    place_rel(w, &mut out, pos, dir, -flip * (rh + 0.8), 4.6, 0.0, true, r"dz\structures\military\misc\Misc_RazorWire.p3d");
                }
                if g.add_wrecks {
                    let car = if rng.gen_bool(0.5) {
                        r"dz\structures\wrecks\vehicles\Wreck_Uaz.p3d"
                    } else {
                        r"dz\structures\wrecks\vehicles\Wreck_HMMWV.p3d"
                    };
                    place_rel(w, &mut out, pos, dir, flip * (rh * 0.45), -6.5, rng.gen_range(-20.0..20.0), true, car);
                }
            } else {
                // Civilian / police roadblock: angled police wrecks + barricades + concrete blocks.
                place_rel(w, &mut out, pos, dir, -flip * (rh * 0.55), 1.8, 0.0, true, r"dz\structures\residential\misc\Misc_Barricade.p3d");
                place_rel(w, &mut out, pos, dir, flip * (rh * 0.55), -1.8, 180.0, true, r"dz\structures\residential\misc\Misc_Barricade.p3d");
                place_rel(w, &mut out, pos, dir, -flip * (rh + 0.3), -1.2, 12.0, true, r"dz\structures\military\misc\Misc_ConcreteBlock1.p3d");
                place_rel(w, &mut out, pos, dir, flip * (rh + 0.3), 1.5, -10.0, true, r"dz\structures\industrial\misc\misc_barrelempty.p3d");
                if g.add_wire {
                    place_rel(w, &mut out, pos, dir, flip * (rh + 0.8), -3.5, 25.0, true, r"dz\structures\military\misc\Misc_BarbedWire.p3d");
                }
                if g.add_wrecks {
                    let police = if rng.gen_bool(0.5) {
                        r"dz\structures\wrecks\vehicles\wreck_sed01_aban1_police.p3d"
                    } else {
                        r"dz\structures\wrecks\vehicles\wreck_hb01_aban1_police.p3d"
                    };
                    place_rel(w, &mut out, pos, dir, flip * (rh * 0.35), 4.2, flip * 38.0, true, police);
                    place_rel(w, &mut out, pos, dir, -flip * (rh * 0.45), -5.0, -flip * 22.0, true, r"dz\structures\wrecks\vehicles\wreck_sed02_aban1_grey.p3d");
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::WorldMeta;

    fn world() -> World {
        World::new(WorldMeta::new("T", 128, 4.0), 10.0)
    }

    #[test]
    fn a_jam_lines_cars_up_in_a_lane() {
        let mut w = world();
        let road = vec![[100.0, 20.0], [100.0, 480.0]];
        let g = WreckGen { scenes_per_km: 20.0, jam_share: 1.0, crash_share: 0.0, decals_per_km: 0.0, under: Vec::new(), ..Default::default() };
        let objects = wreck_objects(&mut w, &ModelDb::new("Z:/none"), &g, std::slice::from_ref(&road), &[], &Job::default()).unwrap();
        assert!(objects.len() >= 3, "{}", objects.len());
        // In a lane, along the road.
        assert!(objects.iter().all(|o| ((o.x - 100.0).abs() - 2.0).abs() < 0.5), "{:?}", objects.iter().map(|o| o.x).collect::<Vec<_>>());
        assert!(objects.iter().all(|o| (o.yaw.rem_euclid(180.0) - 0.0).abs() < 7.0 || (o.yaw.rem_euclid(180.0) - 180.0).abs() < 7.0));
    }

    #[test]
    fn wrecks_can_be_turned_and_decals_kept_near_the_middle() {
        let mut w = world();
        let road = vec![[100.0, 20.0], [100.0, 480.0]];
        // Every car of a jam turned a quarter, with no random turn.
        let g = WreckGen { scenes_per_km: 20.0, jam_share: 1.0, crash_share: 0.0, decals_per_km: 0.0, under: Vec::new(), wreck_turn: 90.0, jam_turn: 0.0, ..Default::default() };
        let objects = wreck_objects(&mut w, &ModelDb::new("Z:/none"), &g, std::slice::from_ref(&road), &[], &Job::default()).unwrap();
        assert!(!objects.is_empty() && objects.iter().all(|o| (o.yaw.rem_euclid(180.0) - 90.0).abs() < 1e-3), "{:?}", objects.iter().map(|o| o.yaw).collect::<Vec<_>>());
        // Decals within a metre of the middle.
        let g = WreckGen { scenes_per_km: 0.0, decals_per_km: 80.0, decal_reach: 1.0, ..Default::default() };
        let objects = wreck_objects(&mut w, &ModelDb::new("Z:/none"), &g, std::slice::from_ref(&road), &[], &Job::default()).unwrap();
        assert!(objects.len() > 10 && objects.iter().all(|o| (o.x - 100.0).abs() <= 1.0 + 1e-3));
    }

    #[test]
    fn decals_lie_on_the_road() {
        let mut w = world();
        let road = vec![[100.0, 20.0], [100.0, 480.0]];
        let g = WreckGen { scenes_per_km: 0.0, decals_per_km: 60.0, ..Default::default() };
        let objects = wreck_objects(&mut w, &ModelDb::new("Z:/none"), &g, std::slice::from_ref(&road), &[], &Job::default()).unwrap();
        assert!(objects.len() > 10);
        assert!(objects.iter().all(|o| (o.x - 100.0).abs() <= 2.5));
        // None on a gravel track.
        let objects = wreck_objects(&mut w, &ModelDb::new("Z:/none"), &g, std::slice::from_ref(&road), &[false], &Job::default()).unwrap();
        assert!(objects.is_empty());
        assert!(is_paved("asf2") && !is_paved("grav"));
    }

    #[test]
    fn buildings_are_known_by_their_folder() {
        assert_eq!(building_kind("dz\\structures\\residential\\houses\\house_1w01.p3d"), Some(BuildingKind::Residential));
        assert_eq!(building_kind("dz\\structures\\industrial\\farms\\barn_wood1.p3d"), Some(BuildingKind::Farm));
        assert_eq!(building_kind("dz\\structures\\military\\houses\\mil_barracks1.p3d"), Some(BuildingKind::Military));
        assert_eq!(building_kind("dz\\structures\\industrial\\sheds\\shed_m1.p3d"), Some(BuildingKind::Industrial));
    }

    #[test]
    fn things_stand_round_a_building_and_not_in_it() {
        let mut w = world();
        let house = Obj { x: 200.0, z: 200.0, elevation: 0.0, yaw: 30.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        // The models are not on disk, so the house has no size: things stand right round it.
        let db = ModelDb::new(std::env::temp_dir());
        let path = "dz\\structures\\residential\\houses\\nothing.p3d".to_string();
        let g = DressGen { min_count: 6, max_count: 6, keep_from_roads: 0.0, ..Default::default() };
        let objects = dress_objects(&mut w, &db, &g, &[(house, path)], &[], &[], &Job::default()).unwrap();
        assert!(!objects.is_empty());
        assert!(objects.iter().all(|o| (o.x - 200.0).hypot(o.z - 200.0) < 8.0));
    }

    #[test]
    fn roadblock_aligns_to_road_and_supports_styles() {
        let mut w = world();
        // Road running east (+x) at z = 200.
        let road = vec![[50.0, 200.0], [350.0, 200.0]];
        let mil = roadblock_objects(&mut w, &RoadblockGen::default(), std::slice::from_ref(&road), &Job::default()).unwrap();
        assert!(mil.len() >= 6, "military checkpoint objects: {}", mil.len());
        // Midpoint of road is (200, 200); all objects clustered near it.
        assert!(mil.iter().all(|o| (o.x - 200.0).hypot(o.z - 200.0) < 12.0));

        let civ = roadblock_objects(&mut w, &RoadblockGen { style: RoadblockStyle::Civilian, ..Default::default() }, std::slice::from_ref(&road), &Job::default()).unwrap();
        assert!(civ.len() >= 5, "civilian roadblock objects: {}", civ.len());

        // Spacing places multiple checkpoints along a long road.
        let multi = roadblock_objects(&mut w, &RoadblockGen { spacing: 100.0, ..Default::default() }, std::slice::from_ref(&road), &Job::default()).unwrap();
        assert!(multi.len() >= mil.len() * 2);
    }
}
