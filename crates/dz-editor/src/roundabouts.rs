//! Roundabouts of any diameter: a closed ring of road pieces placed with a click.
use crate::app::App;
use crate::camera::View;
use crate::i18n::{t, tf};
use dayz_road_tool::roads::{self, PlacedPart};
use dayz_road_tool::roundabout::{roundabout, Ring};
use eframe::egui::{self, Color32, Stroke};

/// The ring for the tool's road type and diameter, round the origin; worked out again only
/// when either changes.
pub fn ring(app: &mut App) -> Option<&Ring> {
    let (family, d) = (app.line.family.clone(), app.line.roundabout_diameter);
    let fresh = app.line.roundabout.as_ref().is_some_and(|(f, x, _)| *f == family && *x == d);
    if !fresh {
        let made = roundabout(&app.road_catalog, &family, [0.0, 0.0], d as f64).map_err(|e| format!("{e:#}"));
        app.line.roundabout = Some((family, d, made));
    }
    app.line.roundabout.as_ref().and_then(|(_, _, r)| r.as_ref().ok())
}

fn moved(parts: &[PlacedPart], center: [f32; 2]) -> Vec<PlacedPart> {
    parts.iter().map(|p| PlacedPart { position: [p.position[0] + center[0] as f64, p.position[1] + center[1] as f64], ..p.clone() }).collect()
}

/// Builds the roundabout round `center`, as a road laid from its pieces.
pub fn place(app: &mut App, center: [f32; 2]) {
    if dayz_road_tool::rails::is_track_family(&app.line.family) {
        app.warn(t("Roundabouts are made of roads, not tracks"));
        return;
    }
    let Some(ring) = ring(app).cloned() else {
        let why = app.line.roundabout.as_ref().and_then(|(_, _, r)| r.as_ref().err().cloned()).unwrap_or_default();
        app.warn(format!("{}: {why}", t("No roundabout of this size can be laid from this road type")));
        return;
    };
    let parts = moved(&ring.parts, center);
    let catalog = app.road_catalog.clone();
    // Its joints stand for the points of the route.
    let mut points: Vec<[f32; 2]> = Vec::with_capacity(parts.len() + 1);
    if let Ok(s) = roads::startpoint(&parts, &catalog) {
        points.push([s.p[0] as f32, s.p[1] as f32]);
    }
    for i in 1..=parts.len() {
        if let Ok(e) = roads::endpoint(&parts[..i], &catalog) {
            points.push([e.p[0] as f32, e.p[1] as f32]);
        }
    }
    let settings = app.line.routing.clone();
    let mut req = app.route_request(app.line.family.clone(), points, false, settings);
    req.laid = parts;
    app.start_route(req, None, None, app.line.grade);
}

/// The ring about to be placed at `cursor`.
pub fn draw(app: &App, painter: &egui::Painter, v: &View, world: &dz_core::world::World, cursor: Option<[f32; 2]>) {
    let (Some(c), Some((_, _, Ok(ring)))) = (cursor, &app.line.roundabout) else { return };
    let parts = moved(&ring.parts, c);
    crate::road_draw::draw_pieces(painter, v, world, &parts, &app.road_catalog, Color32::from_rgba_unmultiplied(255, 230, 90, 110));
    crate::viewport::draw_polyline(painter, v, world, &crate::road_draw::centre(&parts, &app.road_catalog), Stroke::new(2.0_f32, Color32::from_rgb(255, 230, 90)));
}

/// The tool's settings in the road panel.
pub fn fields(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(t("Roundabout Ø"));
        ui.add(egui::DragValue::new(&mut app.line.roundabout_diameter).range(10.0..=600.0).speed(0.5).suffix(" m"));
        let armed = app.line.roundabout_armed;
        if crate::theme::tool_button(ui, armed, &t("Place roundabout")).on_hover_text(t("Click on the map where the middle of the roundabout goes (Esc stops). Its ring is laid round from the road type's own pieces, as near the diameter as they allow.")).clicked() {
            app.line.roundabout_armed = !armed;
            app.line.points.clear();
        }
    });
    if app.line.roundabout_armed {
        let fam = app.line.family.clone();
        let _ = ring(app);
        match &app.line.roundabout {
            Some((_, _, Ok(r))) => { ui.weak(tf("Ring Ø{0} m from {1} pieces, kinks of {2}° where they meet", &[&format!("{:.1}", r.radius * 2.0), &r.parts.len().to_string(), &format!("{:.1}", r.kink)])); }
            Some((_, _, Err(e))) => { ui.colored_label(crate::theme::WARN, format!("{fam}: {e}")); }
            None => {}
        }
    }
}
