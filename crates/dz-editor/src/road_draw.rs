//! Drawing roads live, with the models fitted while the mouse moves, and piece by piece as
//! Terrain Builder's Road Tool lays them.
use crate::app::App;
use crate::camera::View;
use crate::i18n::t;
use dayz_road_tool::geometry::{Point, Port};
use dayz_road_tool::roads::{self, CatalogPart, PlacedPart};
use eframe::egui::{self, Color32, Pos2, Stroke};
use std::sync::{mpsc, Arc};

/// How clicks lay a road.
pub use dz_core::world::RouteMethod as RoadDraw;

type Fitted = (Vec<[f32; 2]>, Result<Vec<PlacedPart>, String>);

/// The fit of the clicked points and the mouse, worked out off the interface thread.
#[derive(Default)]
pub struct LiveFit {
    rx: Option<mpsc::Receiver<Fitted>>,
    /// The last fit made and the points it was made for.
    pub shown: Option<Fitted>,
}

/// A road being laid piece by piece.
#[derive(Default)]
pub struct PartsDraw {
    /// The clicked start, waiting for a second click to give the heading.
    pub start: Option<[f32; 2]>,
    /// Where the next piece joins on, once there is a heading.
    pub open: Option<Port>,
    pub parts: Vec<PlacedPart>,
    /// The piece to lay next.
    pub next: Option<String>,
    /// Curves bend to the left.
    pub left: bool,
    /// The route whose pieces are being changed, and whether at its start.
    pub target: Option<(usize, bool)>,
    /// What the route's pieces were when they were taken, to take them again once it changes.
    pub stamp: u64,
}

/// A fingerprint of a route's pieces.
pub fn stamp(parts: &[PlacedPart]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.len().hash(&mut h);
    for p in parts {
        p.model.hash(&mut h);
        ((p.position[0] * 100.) as i64, (p.position[1] * 100.) as i64, (p.rotation * 100.) as i64).hash(&mut h);
    }
    h.finish()
}

/// Settings for fitting along the drawn line, never searched across the terrain.
fn fit_settings(app: &App) -> dayz_road_tool::document::RoutingSettings {
    if dayz_road_tool::rails::is_track_family(&app.line.family) { app.line.rail_routing.clone() } else { app.line.routing.clone() }
}

fn same(a: &[[f32; 2]], b: &[[f32; 2]]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p[0] - q[0]).abs() < 0.25 && (p[1] - q[1]).abs() < 0.25)
}

/// Keeps the live fit up to date with the points and the mouse at `cursor`.
pub fn live_update(app: &mut App, cursor: Option<[f32; 2]>, ctx: &egui::Context) {
    if app.line.points.is_empty() {
        receive(app);
        app.live.shown = None;
        return;
    }
    let mut want = app.line.points.clone();
    if let Some(c) = cursor {
        if want.last().is_none_or(|l| (l[0] - c[0]).hypot(l[1] - c[1]) > 1.0) {
            want.push(c);
        }
    }
    let (family, settings) = (app.line.family.clone(), fit_settings(app));
    live_fit(app, want, family, settings, ctx);
}

/// The waypoints of route `i` fitted live while one of them is dragged.
pub fn live_route(app: &mut App, i: usize, ctx: &egui::Context) {
    let Some(points) = app.doc.as_ref().and_then(|d| d.world.routes.get(i)).map(|r| r.points.clone()) else { return };
    live_route_with(app, i, points, ctx);
}

/// Route `i` fitted live along `points`, as it would be with them.
pub fn live_route_with(app: &mut App, i: usize, points: Vec<[f32; 2]>, ctx: &egui::Context) {
    let Some(r) = app.doc.as_ref().and_then(|d| d.world.routes.get(i)) else { return };
    let (family, settings) = (r.family.clone(), r.settings.clone());
    live_fit(app, points, family, settings, ctx);
}

/// False when the live fit shown for exactly `points` failed.
pub fn live_fits(app: &App, points: &[[f32; 2]]) -> Result<(), String> {
    match &app.live.shown {
        Some((k, Err(e))) if same(k, points) => Err(e.clone()),
        _ => Ok(()),
    }
}

fn receive(app: &mut App) {
    if let Some(rx) = &app.live.rx {
        match rx.try_recv() {
            Ok(done) => {
                app.live.shown = Some(done);
                app.live.rx = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => app.live.rx = None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
}

/// Fits `want` off the interface thread, unless that fit is shown or one is on its way.
fn live_fit(app: &mut App, want: Vec<[f32; 2]>, family: String, settings: dayz_road_tool::document::RoutingSettings, ctx: &egui::Context) {
    receive(app);
    if want.len() < 2 || app.live.rx.is_some() || app.live.shown.as_ref().is_some_and(|(k, _)| same(k, &want)) {
        return;
    }
    let (tx, rx) = mpsc::channel();
    let catalog = app.road_catalog.clone();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let line: Vec<Point> = want.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
        let fitted = roads::fit(&line, &family, &catalog, &settings, |_, _| true).map_err(|e| format!("{e:#}"));
        let _ = tx.send((want, fitted));
        ctx.request_repaint();
    });
    app.live.rx = Some(rx);
}

/// A click while drawing live keeps the point only where the models reach it.
pub fn live_click(app: &mut App, p: [f32; 2]) {
    if app.line.points.is_empty() {
        app.line.points.push(p);
        return;
    }
    let mut want = app.line.points.clone();
    want.push(p);
    match &app.live.shown {
        Some((k, Err(e))) if same(k, &want) => {
            let e = e.clone();
            app.warn(format!("{}: {e}", t("The road cannot reach this point")));
        }
        _ => app.line.points.push(p),
    }
}

/// The pieces of the family a road can be laid with, straights first.
pub fn choices(catalog: &[CatalogPart], family: &str) -> Vec<CatalogPart> {
    let mut c: Vec<CatalogPart> = catalog.iter().filter(|m| m.family == family && [3, 4].contains(&m.category) && m.model.ports[2].is_none() && m.model.ports[3].is_none() && m.model.ports[0].is_some() && m.model.ports[1].is_some()).cloned().collect();
    c.sort_by(|a, b| a.category.cmp(&b.category).then(turn_of(a).abs().total_cmp(&turn_of(b).abs())).then(a.model.length.total_cmp(&b.model.length)));
    c
}

fn end_cap<'a>(catalog: &'a [CatalogPart], family: &str) -> Option<&'a CatalogPart> {
    catalog.iter().filter(|m| m.family == family && m.category == 6).find(|m| m.path.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase().contains("konec"))
}

/// The bend of a piece entered by its first port, in degrees, left positive.
pub fn turn_of(c: &CatalogPart) -> f64 {
    let (Some(a), Some(b)) = (c.model.ports[0], c.model.ports[1]) else { return 0.0 };
    (-a.outward[0] * b.outward[1] + a.outward[1] * b.outward[0]).atan2(-a.outward[0] * b.outward[0] - a.outward[1] * b.outward[1]).to_degrees()
}

/// `item` laid at `open`, bent to the chosen side.
fn lay(item: &CatalogPart, open: Port, left: bool) -> Option<(PlacedPart, Port)> {
    let reverse = item.category == 4 && (turn_of(item) > 0.0) != left;
    let p = roads::place(item, open, reverse).ok()?;
    let end = roads::endpoint(std::slice::from_ref(&p), std::slice::from_ref(item)).ok()?;
    Some((p, end))
}

/// The piece about to be laid, for the preview.
pub fn next_piece(app: &App) -> Option<PlacedPart> {
    let open = app.road_parts.open?;
    let item = app.road_catalog.iter().find(|c| Some(&c.path) == app.road_parts.next.as_ref())?;
    lay(item, open, app.road_parts.left).map(|(p, _)| p)
}

/// A click while laying piece by piece: the start, then the heading, then the chosen piece.
pub fn parts_click(app: &mut App, p: [f32; 2]) {
    let family = app.line.family.clone();
    let catalog = app.road_catalog.clone();
    let d = &mut app.road_parts;
    if d.open.is_none() {
        let Some(start) = d.start else {
            d.start = Some(p);
            return;
        };
        let (dx, dz) = ((p[0] - start[0]) as f64, (p[1] - start[1]) as f64);
        let len = dx.hypot(dz);
        if len < 0.5 {
            return;
        }
        let mut open = Port { p: [start[0] as f64, start[1] as f64], outward: [dx / len, dz / len] };
        if let Some(cap) = end_cap(&catalog, &family) {
            if let Ok(piece) = roads::place(cap, open, false) {
                if let Ok(end) = roads::endpoint(std::slice::from_ref(&piece), std::slice::from_ref(cap)) {
                    open = end;
                    d.parts.push(piece);
                }
            }
        }
        d.open = Some(open);
        if d.next.is_none() {
            d.next = choices(&catalog, &family).into_iter().max_by(|a, b| a.model.length.total_cmp(&b.model.length).then(b.category.cmp(&a.category))).map(|c| c.path);
        }
        return;
    }
    add_piece(app);
}

/// Chooses the piece before or after the current one in the list.
pub fn cycle_piece(app: &mut App, step: i32) {
    let family = family_of(app);
    cycle_piece_of(app, &family, step);
}

fn cycle_piece_of(app: &mut App, family: &str, step: i32) {
    let list = choices(&app.road_catalog, family);
    if list.is_empty() {
        return;
    }
    let at = list.iter().position(|c| Some(&c.path) == app.road_parts.next.as_ref()).map_or(0, |i| i as i32 + step);
    app.road_parts.next = Some(list[at.rem_euclid(list.len() as i32) as usize].path.clone());
}

/// Lays the chosen piece at the open end.
pub fn add_piece(app: &mut App) {
    let catalog = app.road_catalog.clone();
    let d = &mut app.road_parts;
    let (Some(open), Some(next)) = (d.open, d.next.clone()) else { return };
    let Some(item) = catalog.iter().find(|c| c.path == next) else { return };
    if let Some((piece, end)) = lay(item, open, d.left) {
        d.parts.push(piece);
        d.open = Some(end);
    }
}

/// Takes the last piece away; with none left the road starts over.
pub fn parts_back(app: &mut App) {
    let catalog = app.road_catalog.clone();
    let d = &mut app.road_parts;
    let laid = d.parts.len() - usize::from(d.parts.first().is_some_and(|p| roads::is_cap(p, &catalog)));
    if laid == 0 {
        // The pieces of a route being changed are taken away down to its first one only.
        if d.target.is_none() {
            *d = PartsDraw { next: d.next.clone(), left: d.left, ..Default::default() };
        }
        return;
    }
    d.parts.pop();
    d.open = roads::endpoint(&d.parts, &catalog).ok().or(d.open);
}

/// Builds the road from the pieces laid, closed with an end piece.
pub fn parts_finish(app: &mut App, grade: bool) {
    let catalog = app.road_catalog.clone();
    let target = app.road_parts.target;
    let route = target.and_then(|(i, _)| app.doc.as_ref()?.world.routes.get(i)).cloned();
    let family = route.as_ref().map_or_else(|| app.line.family.clone(), |r| r.family.clone());
    let mut d = std::mem::take(&mut app.road_parts);
    let (next, left) = (d.next.clone(), d.left);
    // A route being changed is read again from what gets built.
    app.road_parts = PartsDraw { next, left, ..Default::default() };
    let Some(open) = d.open else { return };
    if d.parts.iter().filter(|p| !roads::is_cap(p, &catalog)).count() == 0 {
        app.warn(t("Lay at least one piece first"));
        return;
    }
    if let Some(cap) = end_cap(&catalog, &family) {
        if let Ok(piece) = roads::place(cap, open, false) {
            d.parts.push(piece);
        }
    }
    if target.is_some_and(|(_, at_start)| at_start) {
        d.parts = roads::flip(&d.parts);
    }
    // The joints stand for the clicked points, should the road be fitted again later.
    let mut points: Vec<[f32; 2]> = Vec::with_capacity(d.parts.len() + 1);
    if let Ok(s) = roads::startpoint(&d.parts, &catalog) {
        points.push([s.p[0] as f32, s.p[1] as f32]);
    }
    for i in 1..=d.parts.len() {
        if let Ok(e) = roads::endpoint(&d.parts[..i], &catalog) {
            points.push([e.p[0] as f32, e.p[1] as f32]);
        }
    }
    match (target, route) {
        (Some((i, _)), Some(r)) => {
            let mut req = app.route_request(family, points, false, r.settings.clone());
            req.laid = d.parts;
            if let Some(doc) = &mut app.doc { doc.world.routes[i].points = req.points.clone(); }
            app.start_route(req, Some(i), Some(r.points), false);
        }
        _ => {
            let mut req = app.route_request(family, points, false, fit_settings(app));
            req.laid = d.parts;
            app.start_route(req, None, None, grade);
        }
    }
}

/// Takes the pieces of route `i` for changing them at its start or its end. False when the
/// route keeps no pieces (made before they were kept, or brought in from elsewhere).
pub fn load_route(app: &mut App, i: usize, at_start: bool) -> bool {
    let catalog = app.road_catalog.clone();
    let Some(r) = app.doc.as_ref().and_then(|d| d.world.routes.get(i)) else { return false };
    let mut parts = if at_start { roads::flip(&r.parts) } else { r.parts.clone() };
    if parts.last().is_some_and(|p| roads::is_cap(p, &catalog)) {
        parts.pop();
    }
    let open = roads::endpoint(&parts, &catalog).ok();
    let (next, left) = (app.road_parts.next.clone(), app.road_parts.left);
    let next = next.filter(|n| catalog.iter().any(|c| &c.path == n && c.family == r.family));
    let stamp = stamp(&r.parts);
    app.road_parts = PartsDraw { start: None, open, parts, next, left, target: Some((i, at_start)), stamp };
    if app.road_parts.next.is_none() {
        cycle_piece_of(app, &r_family(app, i), 0);
    }
    open.is_some()
}

fn r_family(app: &App, i: usize) -> String {
    app.doc.as_ref().and_then(|d| d.world.routes.get(i)).map(|r| r.family.clone()).unwrap_or_default()
}

/// The road type pieces are chosen from: that of the route being changed, or of the tool.
pub fn family_of(app: &App) -> String {
    match app.road_parts.target {
        Some((i, _)) => r_family(app, i),
        None => app.line.family.clone(),
    }
}

/// The ground outline of road pieces, as the models cover it.
pub fn draw_pieces(painter: &egui::Painter, v: &View, world: &dz_core::world::World, parts: &[PlacedPart], catalog: &[CatalogPart], fill: Color32) {
    for p in parts {
        let Some(item) = catalog.iter().find(|c| c.path == p.model) else { continue };
        let angle = -p.rotation.to_radians();
        let (s, c) = angle.sin_cos();
        let at = |q: Point| -> Option<Pos2> {
            let (x, z) = ((q[0] * c - q[1] * s + p.position[0]) as f32, (q[0] * s + q[1] * c + p.position[1]) as f32);
            v.project([x, world.height.sample(x, z) + 0.25, z])
        };
        for tri in &item.model.triangles {
            if let (Some(a), Some(b), Some(d)) = (at(tri[0]), at(tri[1]), at(tri[2])) {
                if v.rect.expand(100.0).contains(a) {
                    painter.add(egui::Shape::convex_polygon(vec![a, b, d], fill, Stroke::NONE));
                }
            }
        }
    }
}

/// The centre line of road pieces.
pub fn centre(parts: &[PlacedPart], catalog: &[CatalogPart]) -> Vec<[f32; 2]> {
    roads::centerline(parts, catalog).into_iter().map(|p| [p[0] as f32, p[1] as f32]).collect()
}

/// What the live fit and the pieces laid look like in the view.
pub fn draw(app: &App, painter: &egui::Painter, v: &View, world: &dz_core::world::World, cursor: Option<[f32; 2]>) {
    let catalog: &Arc<Vec<CatalogPart>> = &app.road_catalog;
    // While routes are edited, the selected one shows the way it is edited.
    let which = if app.line.action == crate::app::LineAction::Edit {
        match app.line.selected_route.and_then(|i| world.routes.get(i)).map(|r| r.method) {
            Some(RoadDraw::Live) if app.live.shown.is_some() => RoadDraw::Live,
            Some(RoadDraw::Parts) if app.road_parts.target.is_some() => RoadDraw::Parts,
            _ => RoadDraw::Points,
        }
    } else {
        app.line.draw
    };
    match which {
        RoadDraw::Live => match &app.live.shown {
            Some((_, Ok(parts))) => {
                draw_pieces(painter, v, world, parts, catalog, Color32::from_rgba_unmultiplied(70, 220, 110, 90));
                crate::viewport::draw_polyline(painter, v, world, &centre(parts, catalog), Stroke::new(3.5_f32, Color32::from_rgb(70, 220, 110)));
            }
            Some((k, Err(_))) => crate::viewport::draw_polyline(painter, v, world, k, Stroke::new(3.5_f32, Color32::from_rgb(235, 70, 60))),
            None => {}
        },
        RoadDraw::Parts => {
            let d = &app.road_parts;
            draw_pieces(painter, v, world, &d.parts, catalog, Color32::from_rgba_unmultiplied(255, 150, 40, 110));
            crate::viewport::draw_polyline(painter, v, world, &centre(&d.parts, catalog), Stroke::new(2.0_f32, Color32::from_rgb(255, 150, 40)));
            if let Some(next) = next_piece(app) {
                draw_pieces(painter, v, world, std::slice::from_ref(&next), catalog, Color32::from_rgba_unmultiplied(255, 230, 90, 140));
            }
            if let (Some(start), None, Some(c)) = (d.start, d.open, cursor) {
                crate::viewport::draw_polyline(painter, v, world, &[start, c], Stroke::new(2.0_f32, Color32::from_rgb(255, 230, 90)));
            }
            if let Some(start) = d.start {
                if let Some(s) = v.project([start[0], world.height.sample(start[0], start[1]), start[1]]) {
                    painter.circle_filled(s, 4.0, Color32::from_rgb(255, 150, 40));
                }
            }
        }
        RoadDraw::Points => {}
    }
}
