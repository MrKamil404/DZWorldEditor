//! Probe: how Terrain Builder's Road Tool roads are put together, and how the fit compares.
use super::*;
use crate::geometry::{Library, Point};
use crate::roads::{self, CatalogPart};

pub struct Piece {
    pub model: String,
    pub turn: f64,
    pub length: f64,
    pub at: Point,
}

fn sub(a: Point, b: Point) -> Point { [a[0] - b[0], a[1] - b[1]] }
fn norm(a: Point) -> f64 { a[0].hypot(a[1]) }
fn signed(a: Point, b: Point) -> f64 { (a[0] * b[1] - a[1] * b[0]).atan2(a[0] * b[0] + a[1] * b[1]) }

/// Chains of each road in the order a car drives them: begin chain reversed, key, end chain.
pub fn chains(path: &Path, lib: &mut Library) -> Vec<Vec<Piece>> {
    let b = fs::read(path).unwrap();
    let bl = block(&b, 0x8a).unwrap();
    let mut out = Vec::new();
    for e in &bl.entries {
        let body = &b[e.start..e.start + e.len];
        let fs = fields(body).unwrap();
        let (mut start, mut rotation, mut model) = ([0., 0.], 0., String::new());
        for f in &fs {
            match (f.tag, f.typ) {
                (0x8c, 20) => rotation = super::f64(body, f.start).unwrap(),
                (0x8e, 21) if f.len >= 17 => start = [super::f64(body, f.start + 1).unwrap(), super::f64(body, f.start + 9).unwrap()],
                (0x91, 11) => model = text(body, f),
                _ => {}
            }
        }
        let Ok(key) = lib.model(&model) else { continue };
        let mut shape = Shape::default();
        let ports = shape.place(&key, -f64::rem_euclid(rotation, 360.).to_radians(), start);
        let mut walk = |tag: u8, port: usize| -> Vec<Piece> {
            let mut pieces = Vec::new();
            let Some(f) = fs.iter().find(|f| f.tag == tag) else { return pieces };
            let Some(list) = &f.list else { return pieces };
            let Some(mut at) = ports[port] else { return pieces };
            for part in list {
                let pb = &body[part.start..part.start + part.len];
                let pf = fields(pb).unwrap();
                let cat = pf.iter().find(|f| f.tag == 0x7f).map(|f| super::u32(pb, f.start).unwrap()).unwrap_or(3);
                let name = pf.iter().find(|f| f.tag == 0x33 && f.typ == 11).map(|f| text(pb, f)).unwrap_or_default();
                let Ok(m) = lib.model(&name) else { break };
                let Ok(next) = shape.append(&m, at, cat == 7) else { break };
                pieces.push(Piece { model: name, turn: signed(at.outward, next.outward), length: norm(sub(next.p, at.p)), at: next.p });
                at = next;
            }
            pieces
        };
        let end = walk(0x92, 1);
        let begin = walk(0x93, 0);
        // The begin chain runs backwards from the key part.
        let mut road: Vec<Piece> = begin.into_iter().rev().map(|p| Piece { turn: -p.turn, ..p }).collect();
        let key_turn = match (ports[0], ports[1]) { (Some(a), Some(b)) => signed([-a.outward[0], -a.outward[1]], b.outward), _ => 0. };
        let key_at = ports[1].map_or(start, |p| p.p);
        road.push(Piece { model, turn: key_turn, length: key.length, at: key_at });
        road.extend(end);
        out.push(road);
    }
    out
}

#[derive(Default, Debug)]
#[allow(dead_code)]
pub struct Stats {
    pub roads: usize,
    pub parts: usize,
    pub straights: usize,
    pub curves: usize,
    pub metres: f64,
    /// A curve straight after a curve the other way.
    pub direct_flips: usize,
    /// Curves the other way with straights between.
    pub flips_after_straight: usize,
    /// A curve straight after one the same way.
    pub same_after: usize,
    pub turned_deg: f64,
}

pub fn stats(roads: &[Vec<(f64, f64)>]) -> Stats {
    let mut s = Stats { roads: roads.len(), ..Default::default() };
    for r in roads {
        let mut last_curve: Option<f64> = None;
        let mut straight_between = false;
        for &(turn, len) in r {
            s.parts += 1;
            s.metres += len;
            if turn.abs() < 0.3f64.to_radians() {
                s.straights += 1;
                straight_between = true;
                continue;
            }
            s.curves += 1;
            s.turned_deg += turn.abs().to_degrees();
            if let Some(prev) = last_curve {
                if prev.signum() != turn.signum() {
                    if straight_between { s.flips_after_straight += 1 } else { s.direct_flips += 1 }
                } else if !straight_between {
                    s.same_after += 1;
                }
            }
            last_curve = Some(turn);
            straight_between = false;
        }
    }
    s
}

fn simplify(line: &[Point], eps: f64) -> Vec<Point> {
    if line.len() < 3 {
        return line.to_vec();
    }
    let (a, b) = (line[0], *line.last().unwrap());
    let d = sub(b, a);
    let l = norm(d).max(1e-9);
    let (mut worst, mut at) = (0., 0);
    for (i, p) in line.iter().enumerate().skip(1).take(line.len() - 2) {
        let off = ((p[0] - a[0]) * d[1] - (p[1] - a[1]) * d[0]).abs() / l;
        if off > worst { worst = off; at = i; }
    }
    if worst <= eps {
        return vec![a, b];
    }
    let mut left = simplify(&line[..=at], eps);
    left.pop();
    left.extend(simplify(&line[at..], eps));
    left
}

fn distance_to(p: Point, line: &[Point]) -> f64 {
    line.windows(2).map(|w| {
        let d = sub(w[1], w[0]);
        let l2 = (d[0] * d[0] + d[1] * d[1]).max(1e-12);
        let t = (((p[0] - w[0][0]) * d[0] + (p[1] - w[0][1]) * d[1]) / l2).clamp(0., 1.);
        norm(sub(p, [w[0][0] + d[0] * t, w[0][1] + d[1] * t]))
    }).fold(f64::INFINITY, f64::min)
}

pub fn turns(parts: &[roads::PlacedPart], catalog: &[CatalogPart]) -> Vec<(f64, f64)> {
    parts.iter().filter(|p| !roads::is_cap(p, catalog)).filter_map(|p| {
        let m = &catalog.iter().find(|c| c.path == p.model)?.model;
        let (a, b) = (m.ports[0]?, m.ports[1]?);
        let t = signed([-a.outward[0], -a.outward[1]], b.outward);
        Some((if p.reverse { -t } else { t }, m.length))
    }).collect()
}

/// Fits each long Terrain Builder road again from points drawn along it and compares.
pub fn compare(path: &Path, eps: f64, settings: &crate::document::RoutingSettings, label: &str) {
    let folder = PathBuf::from(r"P:\dz\structures\roads\parts");
    let mut lib = Library::new(folder);
    let (catalog, _) = catalog(path, &mut lib).unwrap();
    let all = chains(path, &mut lib);
    let tb: Vec<Vec<(f64, f64)>> = all.iter().map(|r| r.iter().filter(|p| !p.model.to_lowercase().contains("konec")).map(|p| (p.turn, p.length)).collect()).collect();
    eprintln!("[{label}] TB {:?}", stats(&tb));
    let mut hist: std::collections::BTreeMap<String, (usize, f64, f64)> = Default::default();
    for p in all.iter().flatten() {
        let e = hist.entry(p.model.rsplit('\\').next().unwrap_or("").to_lowercase()).or_default();
        e.0 += 1; e.1 = p.turn.abs().to_degrees(); e.2 = p.length;
    }
    let mut h: Vec<_> = hist.into_iter().collect();
    h.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
    for (k, v) in h.iter().take(25) { eprintln!("[{label}] MODEL {k} n={} turn={:.1} len={:.1}", v.0, v.1, v.2); }
    for c in catalog.iter().filter(|c| [3, 4].contains(&c.category)).take(80) {
        let (a, b) = (c.model.ports[0].unwrap(), c.model.ports[1].unwrap());
        eprintln!("[{label}] CAT {} fam={} cat={} turn={:.1} len={:.1}", c.path.rsplit('\\').next().unwrap_or(""), c.family, c.category, signed([-a.outward[0], -a.outward[1]], b.outward).to_degrees(), c.model.length);
    }
    let mut fitted = Vec::new();
    let (mut failed, mut tried, mut dev_sum, mut dev_n, mut dev_max) = (0, 0, 0., 0, 0f64);
    let mut ms = 0u128;
    for r in all.iter().filter(|r| r.iter().map(|p| p.length).sum::<f64>() > 150.) {
        let family = r.iter().find_map(|p| catalog.iter().find(|c| c.path.eq_ignore_ascii_case(&p.model) || c.path.rsplit('\\').next() == p.model.rsplit('\\').next()).map(|c| c.family.clone()));
        let Some(family) = family else { continue };
        let line: Vec<Point> = r.iter().map(|p| p.at).collect();
        let points = simplify(&line, eps);
        if points.len() < 2 { continue; }
        tried += 1;
        let t0 = std::time::Instant::now();
        match roads::fit(&points, &family, &catalog, settings, |_, _| true) {
            Ok(parts) => {
                ms += t0.elapsed().as_millis();
                for q in roads::centerline(&parts, &catalog).iter().step_by(2) {
                    let d = distance_to(*q, &line);
                    dev_sum += d; dev_n += 1; dev_max = dev_max.max(d);
                }
                fitted.push(turns(&parts, &catalog));
            }
            Err(e) => { failed += 1; if failed < 6 { eprintln!("[{label}] FAIL {e:#} pts={}", points.len()); } }
        }
    }
    eprintln!("[{label}] FIT tried {tried} failed {failed} mean dev {:.2} max dev {:.1} time {ms} ms {:?}", dev_sum / dev_n.max(1) as f64, dev_max, stats(&fitted));
}

/// Terrain Builder projects whose roads were laid with Road Tool, from `TB_TV4P` (paths
/// separated by `;`). The parts are read from the MLODs under `P:\dz\structures\roads\parts`.
fn projects() -> Vec<PathBuf> {
    std::env::var("TB_TV4P").map(|v| v.split(';').filter(|s| !s.is_empty()).map(PathBuf::from).collect()).unwrap_or_default()
}

/// How the fit compares with Road Tool roads: `TB_TV4P=... cargo test --release -p road-core
/// tb_roads_against_fit -- --ignored --nocapture`.
#[test]
#[ignore]
fn tb_roads_against_fit() {
    let s = crate::document::RoutingSettings::default();
    for p in projects() {
        let label = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        compare(&p, 2.0, &s, &label);
    }
}

fn draw(img: &mut image::RgbImage, line: &[Point], map: &dyn Fn(Point) -> (f64, f64), c: [u8; 3]) {
    for w in line.windows(2) {
        let (a, b) = (map(w[0]), map(w[1]));
        let n = ((b.0 - a.0).hypot(b.1 - a.1).ceil() as usize).max(1);
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            if x >= 0. && y >= 0. && (x as u32) < img.width() && (y as u32) < img.height() {
                img.put_pixel(x as u32, y as u32, image::Rgb(c));
            }
        }
    }
}
fn dot(img: &mut image::RgbImage, p: (f64, f64), r: i32, c: [u8; 3]) {
    for dx in -r..=r { for dy in -r..=r {
        let (x, y) = (p.0 as i32 + dx, p.1 as i32 + dy);
        if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() { img.put_pixel(x as u32, y as u32, image::Rgb(c)); }
    }}
}

/// Pictures of a few Terrain Builder roads fitted again: TB grey, clicks red, fit orange
/// with a green dot at each curve and a blue one at each straight.
#[test]
#[ignore]
fn tb_roads_pictures() {
    let out = std::env::var("PROBE_OUT").unwrap();
    let eps: f64 = std::env::var("PROBE_EPS").ok().and_then(|v| v.parse().ok()).unwrap_or(4.);
    let Some(first) = projects().into_iter().next() else { return };
    let path = first.as_path();
    let mut lib = Library::new(PathBuf::from(r"P:\dz\structures\roads\parts"));
    let (catalog, _) = catalog(path, &mut lib).unwrap();
    let all = chains(path, &mut lib);
    let s = crate::document::RoutingSettings::default();
    let mut n = 0;
    for r in all.iter().filter(|r| { let l: f64 = r.iter().map(|p| p.length).sum(); l > 300. && l < 900. }) {
        if n >= 6 { break; }
        let family = r.iter().find_map(|p| catalog.iter().find(|c| c.path.rsplit('\\').next() == p.model.rsplit('\\').next()).map(|c| c.family.clone()));
        let Some(family) = family else { continue };
        let line: Vec<Point> = r.iter().map(|p| p.at).collect();
        let points = simplify(&line, eps);
        let Ok(parts) = roads::fit(&points, &family, &catalog, &s, |_, _| true) else { continue };
        let fitted = roads::centerline(&parts, &catalog);
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in line.iter().chain(&fitted) { for k in 0..2 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); } }
        let (w, h) = (1400u32, 900u32);
        let scale = ((w - 40) as f64 / (hi[0] - lo[0]).max(1.)).min((h - 40) as f64 / (hi[1] - lo[1]).max(1.));
        let map = |p: Point| (20. + (p[0] - lo[0]) * scale, h as f64 - 20. - (p[1] - lo[1]) * scale);
        let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([25, 28, 30]));
        draw(&mut img, &line, &map, [150, 150, 150]);
        draw(&mut img, &fitted, &map, [255, 160, 30]);
        let mut at = None;
        for p in &parts {
            let Some(c) = catalog.iter().find(|c| c.path == p.model) else { continue };
            let curve = c.category == 4;
            if let Some(q) = at { dot(&mut img, map(q), 2, if curve { [60, 220, 90] } else { [80, 140, 255] }); }
            at = Some(p.position);
        }
        for p in &points { dot(&mut img, map(*p), 3, [230, 50, 50]); }
        img.save(format!("{out}/road{n}.png")).unwrap();
        let names: Vec<String> = parts.iter().map(|p| { let m = p.model.rsplit('\\').next().unwrap_or("").trim_end_matches(".p3d").to_string(); if p.reverse { format!("{m}'") } else { m } }).collect();
        eprintln!("ROAD{n} scale {scale:.2} px/m {}", names.join(" | "));
        n += 1;
    }
}
