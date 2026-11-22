//! Pictures of the brush tools, drawn here instead of taken from an icon font: a side view of
//! the ground with what the tool does to it. The grey dashes are the ground before, the bright
//! line the ground after.
use crate::theme;
use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, Shape, Stroke};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Raise,
    Lower,
    Smooth,
    Flatten,
    Noise,
    Ramp,
    SetHeight,
    Holes,
    Circle,
    Square,
    Diamond,
    Spray,
    ShapeEdit,
    ShapePolygon,
    ShapePolyline,
    ShapePoint,
    RoadNew,
    RoadEdit,
    DrawPoints,
    DrawLive,
    DrawParts,
    ExtendStart,
    ExtendEnd,
    EndStart,
    EndEnd,
    BendLeft,
    BendRight,
    PlaceOne,
    PlacePaint,
    Add,
}

/// A button with the picture of a tool; the tip tells what it is.
pub fn button(ui: &mut egui::Ui, active: bool, glyph: Glyph, tip: String) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(34.0, 30.0), egui::Sense::click());
    let hovered = r.hovered();
    let fill = if active { theme::ACCENT } else if hovered { theme::HOVER } else { theme::SECTION };
    ui.painter().rect_filled(rect, 4.0, fill);
    draw(ui.painter(), rect.shrink2(vec2(5.0, 4.0)), glyph, active);
    r.on_hover_text(tip)
}

/// A tool as a tile, its picture over its name, as the tools of a mode are shown.
pub fn tile(ui: &mut egui::Ui, active: bool, glyph: Glyph, name: &str, tip: String) -> egui::Response {
    theme::tool_tile(ui, active, name, tip, |p, r, on| draw(p, r, glyph, on))
}

/// One picture in `rect`.
pub fn draw(painter: &egui::Painter, rect: Rect, glyph: Glyph, active: bool) {
    let fg = if active { Color32::WHITE } else { theme::TEXT };
    let ghost = fg.gamma_multiply(0.45);
    let hot = if active { Color32::from_rgb(255, 224, 150) } else { Color32::from_rgb(240, 170, 70) };
    let at = |x: f32, y: f32| pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height());
    let line = |pts: Vec<Pos2>, c: Color32, w: f32| painter.add(Shape::line(pts, Stroke::new(w, c)));
    let dashed = |pts: Vec<Pos2>| { painter.extend(Shape::dashed_line(&pts, Stroke::new(1.2_f32, ghost), 2.5, 2.0)); };
    let curve = |f: &dyn Fn(f32) -> f32| -> Vec<Pos2> { (0..=24).map(|i| { let x = i as f32 / 24.0; at(x, f(x)) }).collect() };
    let bump = |x: f32, amp: f32| (-((x - 0.5) / 0.17).powi(2)).exp() * amp;
    let arrow = |x: f32, from: f32, to: f32| {
        line(vec![at(x, from), at(x, to)], fg, 1.4);
        let d = if to > from { -0.14 } else { 0.14 };
        line(vec![at(x - 0.1, to + d), at(x, to), at(x + 0.1, to + d)], fg, 1.4);
    };
    let zigzag = |amp: f32| -> Vec<Pos2> { (0..=10).map(|i| { let x = i as f32 / 10.0; at(x, 0.62 + if i % 2 == 0 { -amp } else { amp }) }).collect() };
    match glyph {
        Glyph::Raise => {
            dashed(vec![at(0.0, 0.8), at(1.0, 0.8)]);
            line(curve(&|x| 0.8 - bump(x, 0.5)), hot, 1.8);
            arrow(0.5, 0.42, 0.04);
        }
        Glyph::Lower => {
            dashed(vec![at(0.0, 0.3), at(1.0, 0.3)]);
            line(curve(&|x| 0.3 + bump(x, 0.5)), hot, 1.8);
            arrow(0.5, 0.58, 0.96);
        }
        Glyph::Smooth => {
            dashed(zigzag(0.22));
            line(curve(&|x| 0.62 - 0.14 * (x * std::f32::consts::TAU).sin()), hot, 1.8);
        }
        Glyph::Flatten => {
            dashed(zigzag(0.22));
            line(vec![at(0.0, 0.62), at(1.0, 0.62)], hot, 1.8);
        }
        Glyph::Noise => {
            dashed(vec![at(0.0, 0.62), at(1.0, 0.62)]);
            let rough = [0.0, 0.18, -0.1, 0.22, -0.2, 0.08, -0.24, 0.16, -0.06, 0.2, 0.0];
            line(rough.iter().enumerate().map(|(i, h)| at(i as f32 / 10.0, 0.62 - h)).collect(), hot, 1.8);
        }
        Glyph::Ramp => {
            dashed(vec![at(0.0, 0.8), at(1.0, 0.8)]);
            line(vec![at(0.0, 0.8), at(1.0, 0.2)], hot, 1.8);
            painter.circle_filled(at(0.0, 0.8), 2.0, fg);
            painter.circle_filled(at(1.0, 0.2), 2.0, fg);
        }
        Glyph::SetHeight => {
            // The level that was copied, and the ground brought up to it.
            dashed(vec![at(0.0, 0.3), at(1.0, 0.3)]);
            line(vec![at(0.0, 0.3), at(0.3, 0.3), at(0.4, 0.8), at(0.6, 0.8), at(0.7, 0.3), at(1.0, 0.3)], ghost, 1.2);
            line(vec![at(0.4, 0.8), at(0.6, 0.8)], ghost, 1.2);
            line(vec![at(0.4, 0.3), at(0.6, 0.3)], hot, 1.8);
            painter.circle_filled(at(0.12, 0.3), 2.4, hot);
            arrow(0.5, 0.8, 0.42);
        }
        Glyph::Holes => {
            // The ground with a gap cut out of it.
            line(vec![at(0.0, 0.3), at(0.3, 0.3)], fg, 1.8);
            line(vec![at(0.7, 0.3), at(1.0, 0.3)], fg, 1.8);
            painter.extend(Shape::dashed_line(&[at(0.3, 0.3), at(0.3, 0.8), at(0.7, 0.8), at(0.7, 0.3)], Stroke::new(1.5_f32, hot), 2.5, 2.0));
        }
        Glyph::Circle => {
            painter.circle_filled(rect.center(), rect.height() * 0.42, fg.gamma_multiply(0.35));
            painter.circle_stroke(rect.center(), rect.height() * 0.42, Stroke::new(1.6_f32, fg));
        }
        Glyph::Square => {
            let r = Rect::from_center_size(rect.center(), vec2(rect.height() * 0.8, rect.height() * 0.8));
            painter.rect(r, 1.0, fg.gamma_multiply(0.35), Stroke::new(1.6_f32, fg), egui::StrokeKind::Inside);
        }
        Glyph::Diamond => {
            let (c, k) = (rect.center(), rect.height() * 0.46);
            let pts = vec![c + vec2(0.0, -k), c + vec2(k, 0.0), c + vec2(0.0, k), c + vec2(-k, 0.0)];
            painter.add(Shape::convex_polygon(pts, fg.gamma_multiply(0.35), Stroke::new(1.6_f32, fg)));
        }
        Glyph::ShapePolygon => {
            let pts: Vec<Pos2> = [(0.1, 0.35), (0.5, 0.05), (0.92, 0.3), (0.78, 0.92), (0.22, 0.88)].into_iter().map(|(x, y)| at(x, y)).collect();
            painter.add(Shape::convex_polygon(pts.clone(), hot.gamma_multiply(0.3), Stroke::new(1.6_f32, hot)));
            for p in pts { painter.circle_filled(p, 2.2, fg); }
        }
        Glyph::ShapePolyline => {
            let pts: Vec<Pos2> = [(0.05, 0.85), (0.35, 0.3), (0.62, 0.7), (0.95, 0.15)].into_iter().map(|(x, y)| at(x, y)).collect();
            painter.add(Shape::line(pts.clone(), Stroke::new(1.8_f32, hot)));
            for p in pts { painter.circle_filled(p, 2.2, fg); }
        }
        Glyph::ShapePoint => {
            painter.circle_stroke(at(0.5, 0.5), rect.height() * 0.42, Stroke::new(1.2_f32, ghost));
            painter.circle_filled(at(0.5, 0.5), 3.4, hot);
        }
        Glyph::ShapeEdit => {
            // A corner of a shape pulled from where it was to a new place.
            let old = at(0.15, 0.2);
            let base = [(0.85, 0.15), (0.9, 0.9), (0.15, 0.85)];
            let moved = at(0.3, 0.3);
            dashed(vec![at(0.85, 0.15), old, at(0.15, 0.85)]);
            let mut pts = vec![moved];
            pts.extend(base.iter().map(|(x, y)| at(*x, *y)));
            painter.add(Shape::closed_line(pts.clone(), Stroke::new(1.6_f32, fg)));
            for p in &pts[1..] { painter.circle_filled(*p, 2.0, fg); }
            painter.add(Shape::line(vec![old, moved], Stroke::new(1.2_f32, ghost)));
            painter.circle_filled(moved, 3.0, hot);
        }
        Glyph::RoadNew => {
            let road = |dx: f32| -> Vec<Pos2> { (0..=20).map(|i| { let x = i as f32 / 20.0; at(x, 0.5 + dx - 0.3 * ((x - 0.5) * std::f32::consts::PI).sin()) }).collect() };
            line(road(-0.14), fg, 1.6);
            line(road(0.14), fg, 1.6);
            dashed(road(0.0));
            let c = at(0.84, 0.18);
            painter.circle_filled(c, 5.0, hot);
            line(vec![c - vec2(2.6, 0.0), c + vec2(2.6, 0.0)], Color32::from_gray(30), 1.4);
            line(vec![c - vec2(0.0, 2.6), c + vec2(0.0, 2.6)], Color32::from_gray(30), 1.4);
        }
        Glyph::RoadEdit => {
            let pts: Vec<Pos2> = [(0.05, 0.8), (0.35, 0.35), (0.65, 0.65), (0.95, 0.2)].into_iter().map(|(x, y)| at(x, y)).collect();
            line(pts.clone(), fg, 1.6);
            for (i, p) in pts.iter().enumerate() { painter.circle_filled(*p, if i == 1 { 3.2 } else { 2.0 }, if i == 1 { hot } else { fg }); }
            dashed(vec![at(0.35, 0.55), at(0.35, 0.35)]);
        }
        Glyph::DrawPoints => {
            let pts: Vec<Pos2> = [(0.08, 0.8), (0.4, 0.3), (0.7, 0.7), (0.92, 0.2)].into_iter().map(|(x, y)| at(x, y)).collect();
            line(pts.clone(), hot, 1.8);
            for p in pts { painter.circle_filled(p, 2.4, fg); }
        }
        Glyph::DrawLive => {
            // A line that follows the cursor: what is laid, and the stretch after it.
            let f = |x: f32| 0.7 - 0.3 * (x * 0.6 * std::f32::consts::PI).sin();
            line((0..=12).map(|i| { let x = i as f32 / 20.0; at(x, f(x)) }).collect(), hot, 1.8);
            dashed((12..=20).map(|i| { let x = i as f32 / 20.0; at(x, f(x)) }).collect());
            let tip = at(0.95, 0.12);
            painter.add(Shape::convex_polygon(vec![tip, tip + vec2(-2.0, 7.0), tip + vec2(2.2, 4.4)], fg, Stroke::new(0.8_f32, fg)));
        }
        Glyph::DrawParts => {
            for (i, c) in [fg, fg, hot].into_iter().enumerate() {
                let x = 0.02 + i as f32 * 0.34;
                painter.rect_stroke(Rect::from_min_max(at(x, 0.3), at(x + 0.28, 0.7)), 1.0, Stroke::new(1.6_f32, c), egui::StrokeKind::Inside);
            }
        }
        Glyph::ExtendStart | Glyph::ExtendEnd => {
            let m = |x: f32| if glyph == Glyph::ExtendEnd { x } else { 1.0 - x };
            line(vec![at(m(0.0), 0.5), at(m(0.5), 0.5)], fg, 2.0);
            painter.circle_filled(at(m(0.5), 0.5), 2.4, fg);
            dashed(vec![at(m(0.55), 0.5), at(m(0.9), 0.5)]);
            line(vec![at(m(0.78), 0.25), at(m(1.0), 0.5), at(m(0.78), 0.75)], hot, 1.8);
        }
        Glyph::EndStart | Glyph::EndEnd => {
            let m = |x: f32| if glyph == Glyph::EndEnd { x } else { 1.0 - x };
            line(vec![at(0.0, 0.5), at(1.0, 0.5)], ghost, 2.0);
            painter.circle_filled(at(m(0.9), 0.5), 4.2, hot);
            painter.circle_filled(at(m(0.1), 0.5), 2.2, fg);
        }
        Glyph::BendLeft | Glyph::BendRight => {
            let m = |x: f32| if glyph == Glyph::BendRight { 1.0 - x } else { x };
            let pts: Vec<Pos2> = (0..=16).map(|i| { let t = i as f32 / 16.0; let (a, b, c) = ((1.0 - t).powi(2), 2.0 * t * (1.0 - t), t * t); at(m(0.7 * a + 0.7 * b + 0.1 * c), 0.95 * a + 0.3 * b + 0.3 * c) }).collect();
            line(pts, hot, 2.0);
            line(vec![at(m(0.32), 0.05), at(m(0.08), 0.3), at(m(0.32), 0.55)], fg, 1.8);
        }
        Glyph::PlaceOne => {
            painter.rect(Rect::from_center_size(at(0.6, 0.6), vec2(rect.width() * 0.4, rect.height() * 0.4)), 1.0, hot.gamma_multiply(0.5), Stroke::new(1.6_f32, hot), egui::StrokeKind::Inside);
            let tip = at(0.05, 0.05);
            painter.add(Shape::convex_polygon(vec![tip, tip + vec2(0.0, 11.0), tip + vec2(3.0, 8.5), tip + vec2(5.5, 12.5), tip + vec2(7.0, 11.5), tip + vec2(4.5, 7.5), tip + vec2(8.0, 7.0)], fg, Stroke::new(0.8_f32, fg)));
        }
        Glyph::PlacePaint => {
            painter.circle_stroke(rect.center(), rect.height() * 0.46, Stroke::new(1.2_f32, ghost));
            for (x, y) in [(0.5, 0.2), (0.28, 0.42), (0.72, 0.4), (0.4, 0.68), (0.66, 0.7)] {
                painter.rect_filled(Rect::from_center_size(at(x, y), vec2(4.6, 4.6)), 1.0, hot);
            }
        }
        Glyph::Add => {
            painter.circle_stroke(rect.center(), rect.height() * 0.42, Stroke::new(1.4_f32, ghost));
            line(vec![at(0.5, 0.25), at(0.5, 0.75)], hot, 2.2);
            line(vec![at(0.25, 0.5), at(0.75, 0.5)], hot, 2.2);
        }
        Glyph::Spray => {
            for (x, y, r) in [(0.2, 0.3, 1.6), (0.45, 0.2, 1.3), (0.7, 0.32, 1.7), (0.3, 0.55, 1.4), (0.58, 0.5, 1.8), (0.82, 0.62, 1.3), (0.42, 0.8, 1.6), (0.68, 0.82, 1.2), (0.15, 0.78, 1.2)] {
                painter.circle_filled(at(x, y), r, fg);
            }
        }
    }
}
