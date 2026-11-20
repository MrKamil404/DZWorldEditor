//! Finding a place on the map for the "Go to" field and the outliner: a key point by name, a
//! position in metres, or a square of the game's map grid.
//!
//! The grid is the one the in-game map prints: squares of 100 m, counted from the south west
//! corner, the east number first. `045 123` is the square that starts 4500 m east and 12300 m
//! north.
use crate::keypoints::KeyPoint;
use crate::library::{GenTarget, GeneratorInstance};
use crate::shapes::MapShape;
use crate::world::{ModShape, RoadRoute, TerrainModifier, World};

/// Side of a square of the game's map grid in metres.
pub const GAME_GRID: f32 = 100.0;

/// A spot to fly the camera to: its middle and about how far it reaches, in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub x: f32,
    pub z: f32,
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoundKind {
    /// A square of the map grid.
    Cell,
    /// A position in metres.
    Position,
    KeyPoint,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub kind: FoundKind,
    /// The name of a key point; empty for a square or a position.
    pub label: String,
    pub spot: Spot,
}

/// The middle and reach of a set of points.
fn around(points: impl Iterator<Item = [f32; 2]>) -> Option<Spot> {
    let (mut lo, mut hi, mut n) = ([f32::MAX; 2], [f32::MIN; 2], 0);
    for p in points.filter(|p| p[0].is_finite() && p[1].is_finite()) {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
        n += 1;
    }
    (n > 0).then(|| Spot { x: (lo[0] + hi[0]) * 0.5, z: (lo[1] + hi[1]) * 0.5, radius: ((hi[0] - lo[0]).max(hi[1] - lo[1]) * 0.5).max(10.0) })
}

pub fn keypoint_spot(k: &KeyPoint) -> Spot {
    Spot { x: k.x, z: k.z, radius: k.width.max(k.height).clamp(20.0, 500.0) }
}

pub fn modifier_spot(m: &TerrainModifier) -> Spot {
    let at = |radius: f32| Spot { x: m.x, z: m.z, radius: radius.max(10.0) };
    match &m.shape {
        ModShape::Circle { radius } => at(*radius),
        ModShape::Rect { half_x, half_z } => at(half_x.max(*half_z)),
        ModShape::Stamp { size, .. } => at(size * 0.5),
        ModShape::Line { points, half_width, .. } => around(points.iter().copied()).map(|s| Spot { radius: s.radius + half_width, ..s }).unwrap_or_else(|| at(*half_width)),
        ModShape::Pond { areas, .. } => around(areas.iter().flat_map(|a| a.points.iter().copied())).unwrap_or_else(|| at(30.0)),
    }
}

pub fn route_spot(r: &RoadRoute) -> Option<Spot> {
    let points = if r.centerline.is_empty() { &r.points } else { &r.centerline };
    around(points.iter().copied())
}

pub fn shape_spot(s: &MapShape) -> Option<Spot> {
    around(s.points.iter().copied())
}

pub fn layer_spot(l: &crate::shapes::ShapeLayer) -> Option<Spot> {
    around(l.shapes.iter().flat_map(|s| s.points.iter().copied()))
}

/// Where a generator works: its target, or what it placed when it works on the whole map.
pub fn generator_spot(w: &World, g: &GeneratorInstance) -> Option<Spot> {
    let layer = |name: &str| w.shape_layers.iter().find(|l| l.name == name);
    let target = match &g.target {
        GenTarget::Map => None,
        GenTarget::Layer(name) => layer(name).and_then(layer_spot),
        GenTarget::Shape { layer: name, index } => layer(name).and_then(|l| l.shapes.get(*index)).and_then(shape_spot),
        GenTarget::Route(id) => w.routes.iter().find(|r| r.id == *id).and_then(route_spot),
        GenTarget::Point(p) => Some(Spot { x: p[0], z: p[1], radius: 30.0 }),
    };
    target.or_else(|| around(g.objects.iter().filter_map(|id| w.objects.get(*id)).map(|o| [o.x, o.z])))
}

/// What a text asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Query {
    /// The middle of a square of the map grid.
    Cell([f32; 2]),
    Position([f32; 2]),
    Name(String),
}

fn three_digits(s: &str) -> Option<f32> {
    (s.len() == 3 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse::<f32>().unwrap_or(0.0))
}

/// Reads a text such as `045 123` or `045123` (a square of the grid), `4500, 12300` or
/// `4500 12300` (metres) or a name. Two numbers of three digits count as a square; put a
/// comma between them to mean metres.
pub fn parse(text: &str) -> Query {
    let text = text.trim();
    let cell = |x: f32, z: f32| Query::Cell([x * GAME_GRID + GAME_GRID * 0.5, z * GAME_GRID + GAME_GRID * 0.5]);
    let numbers = |parts: Vec<&str>| -> Option<[f32; 2]> {
        match parts[..] {
            [a, b] => Some([a.parse::<f32>().ok()?, b.parse::<f32>().ok()?]).filter(|p| p.iter().all(|v| v.is_finite())),
            _ => None,
        }
    };
    if text.contains([',', ';']) {
        let parts: Vec<&str> = text.split([',', ';']).map(str::trim).collect();
        return numbers(parts).map(Query::Position).unwrap_or_else(|| Query::Name(text.to_string()));
    }
    let parts: Vec<&str> = text.split_whitespace().collect();
    if let [one] = parts[..] {
        if one.len() == 6 {
            if let (Some(x), Some(z)) = (three_digits(&one[..3]), three_digits(&one[3..])) {
                return cell(x, z);
            }
        }
    }
    if let [a, b] = parts[..] {
        if let (Some(x), Some(z)) = (three_digits(a), three_digits(b)) {
            return cell(x, z);
        }
    }
    numbers(parts).map(Query::Position).unwrap_or_else(|| Query::Name(text.to_string()))
}

fn inside(w: &World, x: f32, z: f32) -> bool {
    let size = w.meta.size_m();
    (0.0..=size).contains(&x) && (0.0..=size).contains(&z)
}

/// The places a text stands for: one square or position on the map, or the key points whose
/// name holds the text, the closest matches first.
pub fn find(w: &World, text: &str) -> Vec<Found> {
    match parse(text) {
        Query::Cell([x, z]) | Query::Position([x, z]) if !inside(w, x, z) => Vec::new(),
        Query::Cell([x, z]) => vec![Found { kind: FoundKind::Cell, label: String::new(), spot: Spot { x, z, radius: GAME_GRID } }],
        Query::Position([x, z]) => vec![Found { kind: FoundKind::Position, label: String::new(), spot: Spot { x, z, radius: 30.0 } }],
        Query::Name(name) => {
            let name = name.to_lowercase();
            if name.is_empty() {
                return Vec::new();
            }
            let mut hits: Vec<(u8, &KeyPoint)> = w.keypoints.iter().filter_map(|k| {
                let (a, b) = (k.name.to_lowercase(), k.text.to_lowercase());
                let rank = if a == name || b == name { 0 } else if a.starts_with(&name) || b.starts_with(&name) { 1 } else if a.contains(&name) || b.contains(&name) { 2 } else { return None };
                Some((rank, k))
            }).collect();
            hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
            hits.into_iter().take(12).map(|(_, k)| Found { kind: FoundKind::KeyPoint, label: k.name.clone(), spot: keypoint_spot(k) }).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        let mut w = World::new(crate::world::WorldMeta::new("t", 512, 10.0), 0.0);
        for (name, x) in [("Elektro", 1000.0), ("Electric Hut", 2000.0), ("Kamenka", 3000.0)] {
            w.keypoints.push(KeyPoint { name: name.into(), text: name.into(), x, z: 500.0, ..Default::default() });
        }
        w
    }

    #[test]
    fn a_pair_of_three_digit_numbers_is_a_square_of_the_grid() {
        assert_eq!(parse("045 123"), Query::Cell([4550.0, 12350.0]));
        assert_eq!(parse("045123"), Query::Cell([4550.0, 12350.0]));
        assert_eq!(parse("  001   002 "), Query::Cell([150.0, 250.0]));
    }

    #[test]
    fn metres_and_names_are_told_apart() {
        assert_eq!(parse("4500, 12300"), Query::Position([4500.0, 12300.0]));
        assert_eq!(parse("045,123"), Query::Position([45.0, 123.0]));
        assert_eq!(parse("4500 12300.5"), Query::Position([4500.0, 12300.5]));
        assert_eq!(parse("Elektro"), Query::Name("Elektro".into()));
        assert_eq!(parse("1, two"), Query::Name("1, two".into()));
    }

    #[test]
    fn places_off_the_map_are_not_found() {
        let w = world();
        assert_eq!(w.meta.size_m(), 5120.0);
        assert_eq!(find(&w, "020 020").len(), 1);
        assert!(find(&w, "999 999").is_empty());
        assert!(find(&w, "-5, 10").is_empty());
    }

    #[test]
    fn names_are_found_closest_first() {
        let w = world();
        let names: Vec<String> = find(&w, "elek").into_iter().map(|f| f.label).collect();
        assert_eq!(names, vec!["Elektro"]);
        let names: Vec<String> = find(&w, "ele").into_iter().map(|f| f.label).collect();
        assert_eq!(names, vec!["Electric Hut", "Elektro"]);
        assert_eq!(find(&w, "kamenka")[0].spot.x, 3000.0);
        assert!(find(&w, "").is_empty());
    }

    #[test]
    fn a_spot_covers_the_points_it_is_made_of() {
        let s = around([[0.0, 0.0], [100.0, 40.0]].into_iter()).unwrap();
        assert_eq!((s.x, s.z, s.radius), (50.0, 20.0, 50.0));
        assert!(around(std::iter::empty()).is_none());
    }
}
