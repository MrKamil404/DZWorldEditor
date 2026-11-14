//! Roundabouts: a round ring of road pieces of one family around a centre.
//!
//! The pieces only bend at a few fixed radii, so a ring of any size is laid as `n` equal
//! pieces whose two ends lie on the circle: each piece is a chord of it. With chords `c`
//! long the ring closes exactly on a circle of radius `c / (2 sin(π / n))`, so the number of
//! pieces sets its size to within about a metre. The piece used is the one whose own bend
//! comes nearest the circle's, so the pieces follow the circle and meet with the smallest
//! kinks.
use crate::geometry::{Point, add, norm, rotate, sub};
use crate::roads::{CatalogPart, PlacedPart};
use anyhow::{Result, bail, ensure};
use std::f64::consts::PI;

/// The largest kink between two pieces of a ring that still looks round, in degrees.
const MAX_KINK: f64 = 6.0;

/// A ring of pieces and how it came out.
#[derive(Clone, Debug)]
pub struct Ring {
    pub parts: Vec<PlacedPart>,
    /// Radius of the circle the joints lie on.
    pub radius: f64,
    /// How far the centre line strays from that circle at most, in metres.
    pub stray: f64,
    /// The kink where two pieces meet, in degrees.
    pub kink: f64,
}

/// The bend of a piece entered by its first port, in radians, left positive.
fn turn(c: &CatalogPart) -> f64 {
    let (Some(a), Some(b)) = (c.model.ports[0], c.model.ports[1]) else { return 0.0 };
    (-a.outward[0] * b.outward[1] + a.outward[1] * b.outward[0]).atan2(-a.outward[0] * b.outward[0] - a.outward[1] * b.outward[1])
}

/// A round roundabout of `family` around `center`, its joints on a circle as near
/// `diameter` across as whole pieces allow, running anticlockwise.
pub fn roundabout(catalog: &[CatalogPart], family: &str, center: Point, diameter: f64) -> Result<Ring> {
    ensure!(diameter.is_finite() && diameter > 0.0, "Nieprawidłowa średnica ronda");
    let target = diameter / 2.0;
    let usable = catalog.iter().filter(|c| c.family == family && [3, 4].contains(&c.category) && c.model.ports[0].is_some() && c.model.ports[1].is_some() && c.model.ports[2].is_none() && c.model.ports[3].is_none());
    // (score, piece, pieces round, radius, kink)
    let mut best: Option<(f64, &CatalogPart, usize, f64, f64)> = None;
    for c in usable {
        let (b, e) = (c.model.ports[0].unwrap(), c.model.ports[1].unwrap());
        let chord = norm(sub(e.p, b.p));
        if chord < 0.5 || chord >= diameter { continue; }
        let bend = turn(c).abs();
        let exact = PI / (chord / (2.0 * target)).min(1.0).asin();
        // A few pieces more or fewer than fit the size best may bend with the piece's own
        // bend, which keeps the ring rounder than hitting the size does.
        let near = exact.round() as usize;
        for n in near.saturating_sub(3)..=near + 3 {
            if n < 3 { continue; }
            let radius = chord / (2.0 * (PI / n as f64).sin());
            // Each joint turns the ring by 2π / n; the piece itself turns `bend` of it.
            let kink = (2.0 * PI / n as f64 - bend).abs().to_degrees();
            let error = (radius - target).abs();
            // Round first: rings whose pieces meet at more than a few degrees come only when
            // no round one can be laid; then the size, then gentle joints.
            let score = if kink > MAX_KINK { 1e6 + kink } else { 0.0 } + kink * 0.5 + error * 1.5 + n as f64 * 0.001;
            if best.as_ref().is_none_or(|x| score < x.0) {
                best = Some((score, c, n, radius, kink));
            }
        }
    }
    let Some((_, piece, n, radius, kink)) = best else { bail!("Z tych części nie da się ułożyć ronda tej wielkości") };
    // Curves are entered by the port that makes them bend left, round the centre.
    let reverse = turn(piece) < 0.0;
    let (b, e) = if reverse { (piece.model.ports[1].unwrap(), piece.model.ports[0].unwrap()) } else { (piece.model.ports[0].unwrap(), piece.model.ports[1].unwrap()) };
    let own = sub(e.p, b.p);
    let on_circle = |k: usize| {
        let a = 2.0 * PI * k as f64 / n as f64;
        add(center, [radius * a.cos(), radius * a.sin()])
    };
    let mut parts = Vec::with_capacity(n);
    for k in 0..n {
        let (from, to) = (on_circle(k), on_circle(k + 1));
        let chord = sub(to, from);
        let angle = chord[1].atan2(chord[0]) - own[1].atan2(own[0]);
        parts.push(PlacedPart { model: piece.path.clone(), reverse, position: sub(from, rotate(b.p, angle)), rotation: -angle.to_degrees() });
    }
    let line = crate::roads::centerline(&parts, std::slice::from_ref(piece));
    let stray = line.iter().map(|q| (norm(sub(*q, center)) - radius).abs()).fold(0.0, f64::max);
    Ok(Ring { parts, radius, stray, kink })
}
