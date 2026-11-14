//! Crossroads (`kr_*` models): roads start from their arms, as Terrain Builder's Road Tool
//! lays them, and one can be put into a road already on the map, which then goes on from
//! both ends of its main road.
use crate::app::{App, Finish};
use crate::i18n::t;
use dayz_road_tool::geometry::{Point, Port};
use dayz_road_tool::roads::{self, CatalogPart, PlacedPart, CROSSROAD};
use dz_core::edit::Change;
use dz_core::gen;
use dz_core::objects::Obj;

/// How near a free arm the first or last point of a new road has to be to start there.
const SNAP: f64 = 10.0;
/// How near the end of a road an arm counts as taken by it.
const TAKEN: f32 = 2.0;

fn file_of(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// The crossroads of the catalog, those with an arm of `family`'s base road first.
pub fn choices(catalog: &[CatalogPart], family: &str) -> Vec<CatalogPart> {
    let base = roads::base_family(family);
    let fits = |c: &CatalogPart| roads::crossroad_arms(&c.path).is_some_and(|a| a.iter().flatten().any(|f| *f == base));
    let mut list: Vec<CatalogPart> = catalog.iter().filter(|c| c.category == CROSSROAD).cloned().collect();
    list.sort_by(|a, b| fits(b).cmp(&fits(a)).then(file_of(&a.path).to_ascii_lowercase().cmp(&file_of(&b.path).to_ascii_lowercase())));
    list.dedup_by(|a, b| file_of(&a.path).eq_ignore_ascii_case(file_of(&b.path)));
    list
}

/// A label for a crossroad: its name and the roads on its arms.
pub fn label(c: &CatalogPart) -> String {
    let name = file_of(&c.path).trim_end_matches(".p3d").to_string();
    match roads::crossroad_arms(&c.path) {
        Some(a) => {
            let main = if a[0] == a[1] { a[0].clone().unwrap_or_default() } else { format!("{} → {}", a[0].clone().unwrap_or_default(), a[1].clone().unwrap_or_default()) };
            let shape = if a[3].is_some() { "X" } else { "T" };
            format!("{name}   ({shape}: {main} + {})", a[2].clone().unwrap_or_default())
        }
        None => name,
    }
}

/// The arms of the crossroads on the map that no road leaves yet, facing away from them.
pub fn free_arms(app: &App) -> Vec<Port> {
    let Some(doc) = &app.doc else { return Vec::new() };
    let w = &doc.world;
    let kinds: Vec<Option<&CatalogPart>> = w.objects.models.iter().map(|m| {
        let file = file_of(&m.path);
        if !roads::is_crossroad_name(file) { return None; }
        app.road_catalog.iter().find(|c| c.category == CROSSROAD && file_of(&c.path).eq_ignore_ascii_case(file))
    }).collect();
    if kinds.iter().all(|k| k.is_none()) {
        return Vec::new();
    }
    let ends: Vec<[f32; 2]> = w.routes.iter().flat_map(|r| [r.centerline.first().copied(), r.centerline.last().copied()]).flatten().collect();
    let mut out = Vec::new();
    for (_, o) in w.objects.iter() {
        let Some(Some(item)) = kinds.get(o.model as usize) else { continue };
        let part = PlacedPart { model: item.path.clone(), reverse: false, position: [o.x as f64, o.z as f64], rotation: o.yaw as f64 };
        for port in roads::placed_ports(&part, &item.model).into_iter().flatten() {
            let taken = ends.iter().any(|e| (e[0] as f64 - port.p[0]).hypot(e[1] as f64 - port.p[1]) < TAKEN as f64);
            if !taken { out.push(port); }
        }
    }
    out
}

/// The way a road leaves `port` towards `next`: the port itself, and a point ahead of it
/// when the road has to turn, so the curve it takes has room.
pub fn lead_in(port: Port, next: Option<[f32; 2]>, min_radius: f64) -> Vec<[f32; 2]> {
    let mut points = vec![[port.p[0] as f32, port.p[1] as f32]];
    if let Some(next) = next {
        let (dx, dz) = (next[0] as f64 - port.p[0], next[1] as f64 - port.p[1]);
        let dist = dx.hypot(dz);
        let turn = (port.outward[0] * dz - port.outward[1] * dx).atan2(port.outward[0] * dx + port.outward[1] * dz).abs();
        if dist > 1.0 && turn > 0.05 {
            // The curve's tangent may take at most 45 % of each leg next to it.
            let ahead = (min_radius * (turn * 0.55).min(1.3).tan() / 0.4 + 2.0).min(dist * 0.5);
            points.push([(port.p[0] + port.outward[0] * ahead) as f32, (port.p[1] + port.outward[1] * ahead) as f32]);
        }
    }
    points
}

/// A new road whose first (or else last) point lies by a free crossroad arm starts from that
/// arm, without an end piece there. Returns true when it was joined to one.
pub fn snap(app: &App, req: &mut gen::RouteRequest) -> bool {
    if req.start.is_some() || req.points.len() < 2 || dayz_road_tool::rails::is_track_family(&req.family) {
        return false;
    }
    let arms = free_arms(app);
    let near = |p: [f32; 2]| arms.iter().copied().map(|a| ((a.p[0] - p[0] as f64).hypot(a.p[1] - p[1] as f64), a)).filter(|(d, _)| *d < SNAP).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, a)| a);
    let (arm, mut rest) = match near(req.points[0]) {
        Some(a) => (a, req.points[1..].to_vec()),
        None => match near(*req.points.last().unwrap()) {
            Some(a) => (a, req.points[..req.points.len() - 1].iter().rev().copied().collect()),
            None => return false,
        },
    };
    // Points inside the crossroad's reach are dropped: the road begins at the arm.
    rest.retain(|p| (p[0] as f64 - arm.p[0]).hypot(p[1] as f64 - arm.p[1]) > 1.0);
    if rest.is_empty() {
        return false;
    }
    let mut points = lead_in(arm, rest.first().copied(), req.settings.min_radius);
    points.extend(rest);
    req.points = points;
    req.start = Some(arm);
    true
}

/// Where along a polyline `p` is nearest: (metres along it, the point, the heading there).
fn nearest_on(line: &[[f32; 2]], p: [f32; 2]) -> Option<(f32, [f32; 2], Point)> {
    let mut best: Option<(f32, f32, [f32; 2], Point)> = None;
    let mut walked = 0.0;
    for s in line.windows(2) {
        let (dx, dz) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
        let len = dx.hypot(dz);
        if len < 1e-4 { continue; }
        let t = (((p[0] - s[0][0]) * dx + (p[1] - s[0][1]) * dz) / (len * len)).clamp(0.0, 1.0);
        let q = [s[0][0] + dx * t, s[0][1] + dz * t];
        let d = (q[0] - p[0]).hypot(q[1] - p[1]);
        if best.as_ref().is_none_or(|b| d < b.0) {
            best = Some((d, walked + len * t, q, [(dx / len) as f64, (dz / len) as f64]));
        }
        walked += len;
    }
    best.map(|b| (b.1, b.2, b.3))
}

/// Puts crossroad `model` into route `i` where `at` is nearest to it: the route is taken
/// away and laid again as two roads leaving the crossroad by its main arms, towards its old
/// ends. Its side arm faces the side of the road `at` lies on.
pub fn insert(app: &mut App, i: usize, at: [f32; 2], model: &str) {
    let Some(route) = app.doc.as_ref().and_then(|d| d.world.routes.get(i)).cloned() else { return };
    if dayz_road_tool::rails::is_track_family(&route.family) {
        app.warn(t("Crossroads go into roads, not tracks"));
        return;
    }
    let Some(item) = app.road_catalog.iter().find(|c| c.category == CROSSROAD && c.path == model).cloned() else { return };
    let Some((along, center, heading)) = nearest_on(&route.centerline, at) else { return };
    let total: f32 = route.centerline.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum();
    // Both new roads need room for at least a piece and an end piece.
    let reach = (item.model.length / 2.0) as f32 + 12.0;
    if along < reach || along > total - reach {
        app.warn(t("The crossroad does not fit this close to the end of the road"));
        return;
    }
    let center = [center[0] as f64, center[1] as f64];
    let side = [at[0] as f64 - center[0], at[1] as f64 - center[1]];
    let place = |h: Point| roads::crossroad_at(&item, center, h).ok();
    let Some(mut part) = place(heading) else { return };
    // A T turns round when its side arm would face away from the click, if its main road is
    // the same both ways.
    let arms = roads::crossroad_arms(&item.path);
    let symmetric = arms.as_ref().is_none_or(|a| a[0] == a[1]);
    let ports = roads::placed_ports(&part, &item.model);
    if let (Some(left), None, true) = (ports[2], ports[3], symmetric) {
        if left.outward[0] * side[0] + left.outward[1] * side[1] < 0.0 {
            if let Some(p) = place([-heading[0], -heading[1]]) { part = p; }
        }
    }
    let ports = roads::placed_ports(&part, &item.model);
    let (Some(b), Some(e)) = (ports[0], ports[1]) else { return };
    // The main arm facing back along the route goes towards its start.
    let (back, ahead) = if b.outward[0] * heading[0] + b.outward[1] * heading[1] < 0.0 { (b, e) } else { (e, b) };
    // The waypoints of either side, far enough from the crossroad, ending at the old ends.
    let along_of = |p: [f32; 2]| nearest_on(&route.centerline, p).map_or(0.0, |n| n.0);
    let clear = reach - 6.0;
    let first = route.centerline.first().copied().unwrap_or(route.points[0]);
    let last = route.centerline.last().copied().unwrap_or(*route.points.last().unwrap());
    let mut before: Vec<[f32; 2]> = route.points.iter().copied().filter(|p| along_of(*p) < along - clear).collect();
    let mut after: Vec<[f32; 2]> = route.points.iter().copied().filter(|p| along_of(*p) > along + clear).collect();
    if before.first().is_none_or(|p| (p[0] - first[0]).hypot(p[1] - first[1]) > 1.0) { before.insert(0, first); }
    if after.last().is_none_or(|p| (p[0] - last[0]).hypot(p[1] - last[1]) > 1.0) { after.push(last); }
    before.reverse();
    let request = |app: &App, port: Port, rest: Vec<[f32; 2]>| {
        let mut points = lead_in(port, rest.first().copied(), route.settings.min_radius);
        points.extend(rest);
        let mut req = app.route_request(route.family.clone(), points, route.terrain_route, route.settings.clone());
        req.start = Some(port);
        req
    };
    let (req_a, req_b) = (request(app, back, before), request(app, ahead, after));
    let Some(doc) = &app.doc else { return };
    let mut lite = doc.world.lite();
    let catalog = app.road_catalog.clone();
    let db = app.db.clone();
    app.spawn(t("Putting in a crossroad"), move |job| {
        let a = gen::build_route(&mut lite, &db, &req_a, &catalog, job);
        let b = gen::build_route(&mut lite, &db, &req_b, &catalog, job);
        let models: Vec<String> = lite.objects.models.iter().map(|m| m.path.clone()).collect();
        Ok(Box::new(move |app: &mut App| match (a, b) {
            (Ok(a), Ok(b)) => store(app, i, part, [(req_a, a), (req_b, b)], models),
            (Err(e), _) | (_, Err(e)) => app.error(format!("{}: {e:#}", t("The roads from the crossroad could not be fitted"))),
        }) as Finish)
    });
}

/// Replaces route `i` by the crossroad and the two roads built from its main arms.
fn store(app: &mut App, i: usize, part: PlacedPart, built: [(gen::RouteRequest, gen::RoadResult); 2], models: Vec<String>) {
    let Some(doc) = &mut app.doc else { return };
    let Some(old) = doc.world.routes.get(i).cloned() else { return };
    let previous = doc.world.routes.clone();
    let modifiers_before = doc.world.modifiers.clone();
    let layer = old.objects.first().and_then(|id| doc.world.objects.get(*id)).map(|o| o.layer()).unwrap_or_else(|| doc.world.target_layer());
    let mut change = Vec::new();
    for id in &old.objects {
        if let Some(o) = doc.world.objects.remove(*id) { change.push((*id, Some(o), None)); }
    }
    // The crossroad, as a plain object: the roads on its arms can be fitted again without it.
    let path = gen::road_model_path(&part.model, r"dz\structures\roads\parts");
    let mut cross = Obj { x: part.position[0] as f32, z: part.position[1] as f32, elevation: 0.0, yaw: part.rotation.rem_euclid(360.0) as f32, pitch: 0.0, roll: 0.0, scale: 1.0, model: doc.world.objects.intern(&path), flags: 0 };
    cross.set_layer(layer);
    let cross_id = doc.world.objects.insert(cross);
    change.push((cross_id, None, Some(cross)));
    let map: Vec<u32> = models.iter().map(|m| doc.world.objects.intern(m)).collect();
    let mut area = None;
    if let Some(k) = old.modifier.and_then(|id| doc.world.modifiers.iter().position(|m| m.id == id)) {
        area = doc.world.modifier_rect(&doc.world.modifiers[k]);
        doc.world.modifiers.remove(k);
    }
    doc.world.routes.remove(i);
    let mut added = Vec::new();
    for (k, (req, road)) in built.into_iter().enumerate() {
        let mut ids = Vec::with_capacity(road.objects.len());
        for mut o in road.objects {
            o.model = map[o.model as usize];
            o.set_layer(layer);
            let id = doc.world.objects.insert(o);
            change.push((id, None, Some(o)));
            ids.push(id);
        }
        let mut r = old.clone();
        r.name = format!("{} ({})", old.name, k + 1);
        r.id = dz_core::library::new_id();
        (r.points, r.centerline, r.objects, r.parts, r.profile, r.decks) = (req.points, road.centerline, ids, road.parts, road.profile, road.decks);
        r.modifier = None;
        r.fitted = r.points.clone();
        r.grade |= !r.profile.is_empty();
        doc.world.routes.push(r);
        added.push(doc.world.routes.len() - 1);
    }
    // The ground under the crossroad is levelled as the roads on it are.
    if old.grade {
        if let Some(m) = gen::pad_modifier(&mut doc.world, cross_id, 6.25, 6.25, 1.0, old.grade_falloff) {
            if let Some(r) = doc.world.modifier_rect(&m) { area = Some(area.map_or(r, |a: dz_core::terrain::Rect| a.union(r))); }
            doc.world.modifiers.push(m);
        }
    }
    let label = t("Put in a crossroad");
    doc.history.push(label.clone(), Change::Objects(change));
    doc.history.push(label.clone(), Change::Routes(previous));
    doc.dirty.objects = true;
    if let Some(r) = area {
        doc.world.recompose(Some(r));
        app.height_changed(r);
    }
    for k in added.iter().copied() {
        app.sync_route_modifier(k);
    }
    let doc = app.doc.as_mut().unwrap();
    doc.history.push(label.clone(), Change::Modifiers(modifiers_before));
    doc.history.group_last(3, label);
    app.line.selected_route = added.first().copied();
    app.line.selected_point = None;
    app.line.crossroad_armed = false;
}
