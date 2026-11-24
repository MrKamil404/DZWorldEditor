//! A check of the whole map before it is built: what would break the build or the game
//! (errors), and what is most likely a mistake (warnings), each with where it is.
//!
//! Each issue has a kind and numbers; the editor turns them into words, so they can be
//! translated.
use crate::model_info::ModelDb;
use crate::objects::{ObjectStore, FLAG_UNDERGROUND};
use crate::world::{object_group, World};
use crate::Job;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Breaks the build or the game.
    Error,
    /// Most likely a mistake.
    Warning,
    /// Worth knowing.
    Info,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IssueKind {
    /// The map id is empty or has characters other than letters, digits and underscores.
    BadMapId,
    /// The terrain size is not a multiple of the land grid.
    BadTerrainSize,
    NoSatellite,
    /// A surface layer whose colour texture is not on the work drive (`layer` index).
    MissingLayerTexture,
    /// Models that are not on the work drive.
    MissingModel,
    /// Texture tiles holding more surfaces than the project allows (`count` tiles).
    TileSurfaces,
    /// Objects whose lowest point hangs over the ground.
    Floating,
    /// Objects wholly under the ground without being marked as underground.
    Buried,
    /// Trees or buildings standing in the sea.
    InSea,
    /// Objects outside the map.
    Outside,
    /// The same model twice in the same place.
    Duplicate,
    /// Buildings that stand in one another.
    Overlap,
    /// A road with no parts.
    EmptyRoad,
    /// The player start is outside the map or in the sea.
    BadSpawn,
    NoKeyPoints,
    NoNavmesh,
    /// Problems the mission check finds (`count` errors).
    Mission,
    /// A 100 m square with very many objects that may hurt client FPS (`count` objects).
    DenseArea,
}

impl IssueKind {
    pub fn level(self) -> Level {
        use IssueKind::*;
        match self {
            BadMapId | BadTerrainSize | NoSatellite | MissingLayerTexture | MissingModel => Level::Error,
            TileSurfaces | Floating | Buried | InSea | Outside | Duplicate | Overlap | EmptyRoad | BadSpawn | Mission | DenseArea => Level::Warning,
            NoKeyPoints | NoNavmesh => Level::Info,
        }
    }
}

/// One thing found, where it is (x, z) and the objects it is about.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub kind: IssueKind,
    pub at: Option<[f32; 2]>,
    pub objects: Vec<u32>,
    /// A name it is about: a model, a surface layer or a road.
    pub name: String,
    /// How many: tiles, metres, problems.
    pub count: usize,
}

impl Issue {
    fn new(kind: IssueKind) -> Self {
        Issue { kind, at: None, objects: Vec::new(), name: String::new(), count: 0 }
    }
    pub fn level(&self) -> Level {
        self.kind.level()
    }
}

/// How far the lowest point of an object may hang over the ground before it floats, and how
/// deep buildings may stand in one another before they overlap, in metres.
const FLOAT: f32 = 0.6;
const OVERLAP: f32 = 1.5;

/// Checks the map. `work_drive`, when given, is where models, surface textures and the built map
/// are looked for.
pub fn check(w: &World, db: &ModelDb, work_drive: Option<&Path>, job: &Job) -> Vec<Issue> {
    let mut out = Vec::new();
    let meta = &w.meta;
    let size = meta.size_m();
    if meta.map_id.is_empty() || !meta.map_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        out.push(Issue { name: meta.map_id.clone(), ..Issue::new(IssueKind::BadMapId) });
    }
    if meta.cells_per_tile == 0 || meta.terrain_size % meta.cells_per_tile != 0 || meta.land_size() % meta.cells_per_tile != 0 {
        out.push(Issue::new(IssueKind::BadTerrainSize));
    }
    if w.satellite.is_none() {
        out.push(Issue::new(IssueKind::NoSatellite));
    }
    if let Some(root) = work_drive {
        for (i, l) in w.layers.iter().enumerate() {
            if !l.texture.is_empty() && !root.join(l.texture.replace('\\', "/")).is_file() {
                out.push(Issue { name: l.name.clone(), count: i, ..Issue::new(IssueKind::MissingLayerTexture) });
            }
        }
        let navmesh = root.join(&meta.map_id).join("navmesh").join("navmesh.nm");
        if !navmesh.is_file() {
            out.push(Issue::new(IssueKind::NoNavmesh));
        }
    }
    let limit = crate::export::MAX_SLOTS.min(meta.max_tile_surfaces.max(1) as usize);
    let over = crate::mask_limit::count_over(w, limit);
    if over > 0 {
        out.push(Issue { count: over, ..Issue::new(IssueKind::TileSurfaces) });
    }
    if w.keypoints.is_empty() {
        out.push(Issue::new(IssueKind::NoKeyPoints));
    }
    let spawn = meta.spawn;
    if spawn[0] < 0.0 || spawn[1] < 0.0 || spawn[0] > size || spawn[1] > size || w.height.sample(spawn[0], spawn[1]) < meta.sea_level {
        out.push(Issue { at: Some(spawn), ..Issue::new(IssueKind::BadSpawn) });
    }
    for r in &w.routes {
        if r.objects.is_empty() {
            out.push(Issue { at: r.points.first().copied(), name: r.name.clone(), ..Issue::new(IssueKind::EmptyRoad) });
        }
    }
    if let Some(m) = &w.mission {
        let errors = crate::mission::validate::check(m, Some(size)).iter().filter(|i| i.level == crate::mission::validate::Level::Error).count();
        if errors > 0 {
            out.push(Issue { count: errors, ..Issue::new(IssueKind::Mission) });
        }
    }
    job.set_progress(0.1);
    out.extend(check_objects(w, db, job));
    out
}

/// What is wrong with the objects: missing models, floating, buried, in the sea, outside the
/// map, twice in one place, buildings in one another.
fn check_objects(w: &World, db: &ModelDb, job: &Job) -> Vec<Issue> {
    let size = w.meta.size_m();
    let sea = w.meta.sea_level;
    let store = &w.objects;
    let objects: Vec<(u32, crate::objects::Obj)> = store.iter().filter(|(_, o)| w.object_exported(o)).map(|(id, o)| (id, *o)).collect();
    // What is known of each model, read once.
    let infos: Vec<crate::model_info::ModelInfo> = store.models.par_iter().map(|m| db.get(&m.path)).collect();
    let groups: Vec<Option<usize>> = store.models.iter().map(|m| object_group(&m.path)).collect();
    let houses: Vec<bool> = store.models.iter().zip(&infos).zip(&groups).map(|((m, i), g)| *g == Some(1) && i.known && is_house(&m.path, i)).collect();
    let mut out = Vec::new();
    // Objects whose land contact hangs over the ground; whether something else holds them up
    // is seen once the grid is made.
    let mut floating: Vec<(u32, [f32; 2], f32)> = Vec::new();
    let mut missing: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut per_kind: HashMap<IssueKind, Vec<(u32, [f32; 2])>> = HashMap::new();
    for (id, o) in &objects {
        let info = &infos[o.model as usize];
        if !info.known {
            missing.entry(o.model).or_default().push(*id);
            continue;
        }
        if o.x < 0.0 || o.z < 0.0 || o.x > size || o.z > size {
            per_kind.entry(IssueKind::Outside).or_default().push((*id, [o.x, o.z]));
            continue;
        }
        let group = groups[o.model as usize];
        let underground = o.flags & FLAG_UNDERGROUND != 0;
        // Only what stands on the ground by itself: trees, buildings, walls, rocks.
        let standing = matches!(group, Some(0 | 1 | 2 | 4));
        if standing && !underground {
            let origin = ObjectStore::origin_y(o, &w.height);
            let (hx, hz) = info.footprint();
            let (hx, hz) = (hx * o.scale, hz * o.scale);
            let (s, c) = o.yaw.to_radians().sin_cos();
            let corners = [[-hx, -hz], [hx, -hz], [hx, hz], [-hx, hz], [0.0, 0.0]];
            let ground: Vec<f32> = corners.iter().map(|p| w.height.sample(o.x + p[0] * c + p[1] * s, o.z - p[0] * s + p[1] * c)).collect();
            let (lo, hi) = ground.iter().fold((f32::MAX, f32::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
            let top = origin + info.max[1] * o.scale;
            // Models without a land contact (wires, roofs, razor wire) are made to hang.
            if info.contact_count > 0 && origin + info.contact_y * o.scale > hi + FLOAT && o.pitch.abs() < 30.0 && o.roll.abs() < 30.0 {
                floating.push((*id, [o.x, o.z], origin + info.contact_y * o.scale));
            } else if top < lo - 0.3 && info.max[1] - info.min[1] > 0.5 {
                per_kind.entry(IssueKind::Buried).or_default().push((*id, [o.x, o.z]));
            }
        }
        let in_sea_ok = group == Some(1) && !houses[o.model as usize];
        if matches!(group, Some(0 | 1)) && !in_sea_ok && w.height.sample(o.x, o.z) < sea - 0.3 && !underground {
            per_kind.entry(IssueKind::InSea).or_default().push((*id, [o.x, o.z]));
        }
    }
    job.set_progress(0.5);
    let mut models: Vec<_> = missing.into_iter().collect();
    models.sort_by_key(|(m, _)| *m);
    for (model, ids) in models {
        let first = store.get(ids[0]).map(|o| [o.x, o.z]);
        out.push(Issue { kind: IssueKind::MissingModel, at: first, count: ids.len(), objects: ids, name: store.models[model as usize].path.clone() });
    }
    // Twice in one place, and buildings in one another, found through a grid.
    const CELL: f32 = 32.0;
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (k, (_, o)) in objects.iter().enumerate() {
        grid.entry(((o.x / CELL).floor() as i32, (o.z / CELL).floor() as i32)).or_default().push(k);
    }
    let pairs: Vec<(IssueKind, u32, [f32; 2])> = objects.par_iter().enumerate().flat_map_iter(|(k, (id, o))| {
        let mut found = Vec::new();
        let key = ((o.x / CELL).floor() as i32, (o.z / CELL).floor() as i32);
        let building = houses[o.model as usize];
        for dz in -1..=1 {
            for dx in -1..=1 {
                let Some(list) = grid.get(&(key.0 + dx, key.1 + dz)) else { continue };
                for &m in list {
                    // Each pair once: the later object is the one reported.
                    if m >= k {
                        continue;
                    }
                    let (_, p) = &objects[m];
                    if p.model == o.model && (p.x - o.x).abs() < 0.1 && (p.z - o.z).abs() < 0.1 && (p.elevation - o.elevation).abs() < 0.1 {
                        found.push((IssueKind::Duplicate, *id, [o.x, o.z]));
                    } else if building && houses[p.model as usize] && overlap(o, &infos[o.model as usize], p, &infos[p.model as usize]) > OVERLAP {
                        found.push((IssueKind::Overlap, *id, [o.x, o.z]));
                    }
                }
            }
        }
        found.dedup_by_key(|f| f.0);
        found
    }).collect();
    for (kind, id, at) in pairs {
        per_kind.entry(kind).or_default().push((id, at));
    }
    // What hangs over the ground but stands on another object (a bench on a platform, a box on
    // a ramp) is held up.
    for (id, at, bottom) in floating {
        let key = ((at[0] / CELL).floor() as i32, (at[1] / CELL).floor() as i32);
        let held = (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (key.0 + dx, key.1 + dz))).filter_map(|k| grid.get(&k)).flatten().any(|&m| {
            let (other, p) = &objects[m];
            let info = &infos[p.model as usize];
            *other != id && info.known && ObjectStore::origin_y(p, &w.height) + info.max[1] * p.scale >= bottom - FLOAT && contains(p, info, at, 0.5)
        });
        if !held {
            per_kind.entry(IssueKind::Floating).or_default().push((id, at));
        }
    }
    job.set_progress(0.9);
    let mut kinds: Vec<_> = per_kind.into_iter().collect();
    kinds.sort_by_key(|(k, _)| *k as u8);
    for (kind, mut list) in kinds {
        list.sort_by_key(|x| x.0);
        list.dedup_by_key(|x| x.0);
        // One issue a place, so a hundred trees in one bay are one line to look at.
        for group in cluster(&list, 150.0) {
            let at = group.iter().fold([0.0, 0.0], |a, x| [a[0] + x.1[0], a[1] + x.1[1]]);
            let n = group.len() as f32;
            out.push(Issue { kind, at: Some([at[0] / n, at[1] / n]), count: group.len(), objects: group.iter().map(|x| x.0).collect(), name: String::new() });
        }
    }
    // Very dense 100 m squares hurt FPS in DayZ cities.
    let density = crate::analysis::DensityGrid::build(w, 100.0, true);
    for (at, count) in density.hotspots(800, 10) {
        out.push(Issue { kind: IssueKind::DenseArea, at: Some(at), count, objects: Vec::new(), name: String::new() });
    }
    out
}

/// Whether a model is a house or hall that should not stand in another: large enough, and not
/// one of the props, piles, ruins, piers and walls that are made to be set into others.
pub fn is_house(path: &str, info: &crate::model_info::ModelInfo) -> bool {
    let p = path.to_ascii_lowercase();
    const PROPS: [&str; 20] = ["misc", "garbage", "rubble", "ruins", "pier", "harbour", "platform", "proxy", "tank", "pipe", "wall", "bridge", "decal", "furniture", "signs", "power", "mound", "underground", "stairs", "pile"];
    if PROPS.iter().any(|k| p.contains(k)) {
        return false;
    }
    let (hx, hz) = info.footprint();
    hx >= 2.0 && hz >= 2.0 && info.max[1] - info.min[1] >= 3.0
}

/// Whether the rotated footprint of an object, grown by `margin` metres, holds a point.
pub fn contains(o: &crate::objects::Obj, info: &crate::model_info::ModelInfo, at: [f32; 2], margin: f32) -> bool {
    let (s, c) = o.yaw.to_radians().sin_cos();
    let (dx, dz) = (at[0] - o.x, at[1] - o.z);
    // Into the object's own axes.
    let (lx, lz) = ((dx * c - dz * s) / o.scale, (dx * s + dz * c) / o.scale);
    lx >= info.min[0] - margin && lx <= info.max[0] + margin && lz >= info.min[2] - margin && lz <= info.max[2] + margin
}

/// How deep two objects stand in one another, in metres: the least overlap of their rotated
/// footprints along the four axes of the two boxes (0 when they do not touch).
fn overlap(a: &crate::objects::Obj, ia: &crate::model_info::ModelInfo, b: &crate::objects::Obj, ib: &crate::model_info::ModelInfo) -> f32 {
    let boxed = |o: &crate::objects::Obj, i: &crate::model_info::ModelInfo| {
        let (hx, hz) = i.footprint();
        // The box's middle may be off its origin.
        let (cx, cz) = ((i.min[0] + i.max[0]) * 0.5 * o.scale, (i.min[2] + i.max[2]) * 0.5 * o.scale);
        let (s, c) = o.yaw.to_radians().sin_cos();
        let centre = [o.x + cx * c + cz * s, o.z - cx * s + cz * c];
        (centre, [[c, -s], [s, c]], [hx * o.scale, hz * o.scale])
    };
    let (ca, axa, ha) = boxed(a, ia);
    let (cb, axb, hb) = boxed(b, ib);
    let d = [cb[0] - ca[0], cb[1] - ca[1]];
    let mut least = f32::MAX;
    for axis in [axa[0], axa[1], axb[0], axb[1]] {
        let project = |ax: &[[f32; 2]; 2], h: &[f32; 2]| h[0] * (ax[0][0] * axis[0] + ax[0][1] * axis[1]).abs() + h[1] * (ax[1][0] * axis[0] + ax[1][1] * axis[1]).abs();
        let gap = (d[0] * axis[0] + d[1] * axis[1]).abs();
        let depth = project(&axa, &ha) + project(&axb, &hb) - gap;
        if depth <= 0.0 {
            return 0.0;
        }
        least = least.min(depth);
    }
    least
}

/// Groups of items that lie within `radius` of the first of their group.
fn cluster(list: &[(u32, [f32; 2])], radius: f32) -> Vec<Vec<(u32, [f32; 2])>> {
    let mut groups: Vec<Vec<(u32, [f32; 2])>> = Vec::new();
    let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for item in list {
        let key = ((item.1[0] / radius).floor() as i32, (item.1[1] / radius).floor() as i32);
        let near = (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (key.0 + dx, key.1 + dz))).filter_map(|k| cells.get(&k)).flatten().copied().find(|g: &usize| {
            let f = groups[*g][0].1;
            (f[0] - item.1[0]).hypot(f[1] - item.1[1]) < radius
        });
        match near {
            Some(g) => groups[g].push(*item),
            None => {
                cells.entry(key).or_default().push(groups.len());
                groups.push(vec![*item]);
            }
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::Obj;
    use crate::world::WorldMeta;

    fn world() -> World {
        let mut w = World::new(WorldMeta::new("Test", 64, 4.0), 10.0);
        w.satellite = Some(crate::terrain::RgbImage::new(8, 8, [0, 0, 0]));
        w.keypoints.push(Default::default());
        w
    }

    fn obj(w: &mut World, model: &str, x: f32, z: f32) -> u32 {
        let m = w.objects.intern(model);
        w.objects.insert(Obj { x, z, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: m, flags: 0 })
    }

    #[test]
    fn a_clean_map_has_nothing_but_what_is_worth_knowing() {
        let w = world();
        let issues = check(&w, &ModelDb::new(std::env::temp_dir()), None, &Job::default());
        assert!(issues.iter().all(|i| i.level() == Level::Info), "{issues:?}");
    }

    #[test]
    fn map_wide_problems_are_errors() {
        let mut w = world();
        w.meta.map_id = "bad id".into();
        w.satellite = None;
        let kinds: Vec<IssueKind> = check(&w, &ModelDb::new(std::env::temp_dir()), None, &Job::default()).iter().map(|i| i.kind).collect();
        assert!(kinds.contains(&IssueKind::BadMapId));
        assert!(kinds.contains(&IssueKind::NoSatellite));
    }

    #[test]
    fn missing_models_and_objects_outside_are_found() {
        let mut w = world();
        let a = obj(&mut w, "dz\\nothing\\here.p3d", 10.0, 10.0);
        let b = obj(&mut w, "dz\\nothing\\here.p3d", 20.0, 10.0);
        let issues = check(&w, &ModelDb::new(std::env::temp_dir()), None, &Job::default());
        let m = issues.iter().find(|i| i.kind == IssueKind::MissingModel).unwrap();
        assert_eq!(m.objects, vec![a, b]);
        assert_eq!(m.level(), Level::Error);
    }

    #[test]
    fn boxes_overlap_by_their_depth() {
        let info = crate::model_info::ModelInfo { min: [-5.0, 0.0, -5.0], max: [5.0, 5.0, 5.0], known: true, ..Default::default() };
        let o = |x: f32, yaw: f32| Obj { x, z: 0.0, elevation: 0.0, yaw, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        assert!((overlap(&o(0.0, 0.0), &info, &o(7.0, 0.0), &info) - 3.0).abs() < 1e-3);
        assert_eq!(overlap(&o(0.0, 0.0), &info, &o(11.0, 0.0), &info), 0.0);
        // Turned 45°, the corner reaches further.
        assert!(overlap(&o(0.0, 0.0), &info, &o(11.0, 45.0), &info) > 1.0);
    }

    #[test]
    fn a_point_inside_a_turned_footprint() {
        let info = crate::model_info::ModelInfo { min: [-5.0, 0.0, -1.0], max: [5.0, 2.0, 1.0], known: true, ..Default::default() };
        let o = Obj { x: 100.0, z: 100.0, elevation: 0.0, yaw: 90.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        // Turned a quarter, the long side runs north and south.
        assert!(contains(&o, &info, [100.0, 104.0], 0.0));
        assert!(!contains(&o, &info, [104.0, 100.0], 0.0));
    }

    #[test]
    fn nearby_items_are_one_place() {
        let list = vec![(1, [0.0, 0.0]), (2, [10.0, 0.0]), (3, [1000.0, 0.0])];
        let groups = cluster(&list, 150.0);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 2);
    }
}
