use crate::{
    document::RoutingSettings,
    geometry::{Library, Model, Point, Port, Shape, add, mul, norm, rotate, sub},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PlacedPart {
    pub model: String,
    pub reverse: bool,
    pub position: Point,
    pub rotation: f64,
}
#[derive(Clone)]
pub struct CatalogPart {
    pub path: String,
    pub family: String,
    pub category: u32,
    pub index: u32,
    /// Position of the parent definition in TV4P's 0x88 list (not the part index).
    pub road_type_index: u32,
    pub model: Model,
}
pub fn append(
    parts: &mut Vec<PlacedPart>,
    catalog: &[CatalogPart],
    name: &str,
    reverse: bool,
    start: Point,
    angle: f64,
) -> Result<()> {
    let item = catalog
        .iter()
        .find(|p| p.path == name)
        .context("Brak modelu w katalogu")?;
    let at = if parts.is_empty() {
        Port {
            p: start,
            outward: [angle.cos(), angle.sin()],
        }
    } else {
        endpoint(parts, catalog)?
    };
    let p = place(item, at, reverse)?;
    parts.push(p);
    Ok(())
}
pub fn place(item: &CatalogPart, at: Port, reverse: bool) -> Result<PlacedPart> {
    let port = item.model.ports[usize::from(reverse)].context("Brak portu modelu")?;
    let target = mul(at.outward, -1.);
    let angle = target[1].atan2(target[0]) - port.outward[1].atan2(port.outward[0]);
    Ok(PlacedPart {
        model: item.path.clone(),
        reverse,
        position: sub(at.p, rotate(port.p, angle)),
        rotation: -angle.to_degrees(),
    })
}
pub fn endpoint(parts: &[PlacedPart], catalog: &[CatalogPart]) -> Result<Port> {
    let p = parts.last().context("Pusta droga")?;
    let m = &catalog
        .iter()
        .find(|m| m.path == p.model)
        .context("Brak modelu")?
        .model;
    placed_endpoint(p, m)
}
fn placed_endpoint(p: &PlacedPart, m: &Model) -> Result<Port> {
    let port = m.ports[1 - usize::from(p.reverse)].context("Brak wyjścia modelu")?;
    let angle = -p.rotation.to_radians();
    Ok(Port {
        p: add(rotate(port.p, angle), p.position),
        outward: rotate(port.outward, angle),
    })
}
pub fn shape(parts: &[PlacedPart], lib: &mut Library) -> Result<Shape> {
    let mut s = Shape::default();
    for p in parts {
        let m = lib.model(&p.model)?;
        s.place(&m, -p.rotation.to_radians(), p.position);
    }
    Ok(s)
}
pub fn centerline(parts: &[PlacedPart], catalog: &[CatalogPart]) -> Vec<Point> {
    let mut out = Vec::new();
    for p in parts {
        if let Some(item) = catalog.iter().find(|m| m.path == p.model) {
            let mut line = item.model.line.clone();
            if p.reverse {
                line.reverse();
            }
            for q in line {
                let world = add(rotate(q, -p.rotation.to_radians()), p.position);
                if out.last().is_none_or(|v| norm(sub(*v, world)) > 1e-6) {
                    out.push(world);
                }
            }
        }
    }
    out
}
/// Round polyline corners by a circular fillet; never shrink the requested radius.
pub fn smooth(points: &[Point], radius: f64) -> Result<Vec<Point>> {
    ensure!(
        points.len() >= 2 && radius.is_finite() && radius > 0.,
        "Nieprawidłowa trasa lub promień"
    );
    let mut out = vec![points[0]];
    for w in points.windows(3) {
        let incoming = sub(w[1], w[0]);
        let outgoing = sub(w[2], w[1]);
        let a = norm(incoming);
        let b = norm(outgoing);
        ensure!(a > 1e-6 && b > 1e-6, "Usuń powtórzone punkty trasy");
        let u = mul(incoming, 1. / a);
        let v = mul(outgoing, 1. / b);
        let dot = (u[0] * v[0] + u[1] * v[1]).clamp(-1., 1.);
        let turn = dot.acos();
        if turn < 0.01 {
            out.push(w[1]);
            continue;
        }
        ensure!(
            turn < std::f64::consts::PI - 0.01,
            "Trasa zawraca w jednym punkcie"
        );
        let tangent = radius * (turn / 2.).tan();
        ensure!(
            tangent <= a * 0.45 && tangent <= b * 0.45,
            "Punkty są za blisko, aby zmieścić promień {radius} m. Rozsuń punkty lub zmniejsz promień"
        );
        let sign = (u[0] * v[1] - u[1] * v[0]).signum();
        let entry = sub(w[1], mul(u, tangent));
        let center = add(entry, mul([-u[1], u[0]], radius * sign));
        let radial = sub(entry, center);
        let steps = (turn.to_degrees() / 2.).ceil().max(2.) as usize;
        for i in 0..=steps {
            out.push(add(
                center,
                rotate(radial, turn * sign * i as f64 / steps as f64),
            ));
        }
    }
    out.push(*points.last().unwrap());
    Ok(out)
}
/// Round the corners of a drawn line as wide as the legs allow, the way a road is laid out:
/// gentle bends become long gentle curves, while a sharp corner keeps close to the point
/// clicked (within `miss` metres) and never turns tighter than `min_radius`.
pub fn widen(points: &[Point], min_radius: f64, miss: f64) -> Result<Vec<Point>> {
    ensure!(
        points.len() >= 2 && min_radius.is_finite() && min_radius > 0.,
        "Nieprawidłowa trasa lub promień"
    );
    let legs: Vec<f64> = points.windows(2).map(|w| norm(sub(w[1], w[0]))).collect();
    ensure!(legs.iter().all(|l| *l > 1e-6), "Usuń powtórzone punkty trasy");
    let mut out = vec![points[0]];
    for (k, w) in points.windows(3).enumerate() {
        let (a, b) = (legs[k], legs[k + 1]);
        let u = mul(sub(w[1], w[0]), 1. / a);
        let v = mul(sub(w[2], w[1]), 1. / b);
        let turn = (u[0] * v[0] + u[1] * v[1]).clamp(-1., 1.).acos();
        if turn < 0.002 {
            out.push(w[1]);
            continue;
        }
        ensure!(turn < std::f64::consts::PI - 0.01, "Trasa zawraca w jednym punkcie");
        let half = (turn / 2.).tan();
        // Each leg is shared by the curves at both of its ends, unless it is the first or last.
        let share = |leg: f64, outer: bool| if outer { leg * 0.9 } else { leg * 0.48 };
        let room = share(a, k == 0).min(share(b, k + 2 == points.len() - 1));
        let fits = room / half;
        let close = miss / (1. / (turn / 2.).cos() - 1.).max(1e-9);
        let radius = fits.min(close).max(min_radius);
        let mut tangent = radius * half;
        if tangent > room {
            // Too tight for the legs: a corner the models cannot round is left to the fit.
            ensure!(
                fits >= min_radius * 0.5,
                "Punkty są za blisko, aby zmieścić promień {min_radius} m. Rozsuń punkty lub zmniejsz promień"
            );
            tangent = room;
        }
        let radius = tangent / half;
        let sign = (u[0] * v[1] - u[1] * v[0]).signum();
        let entry = sub(w[1], mul(u, tangent));
        let center = add(entry, mul([-u[1], u[0]], radius * sign));
        let radial = sub(entry, center);
        let steps = (turn.to_degrees() / 2.).ceil().max(2.) as usize;
        for i in 0..=steps {
            out.push(add(center, rotate(radial, turn * sign * i as f64 / steps as f64)));
        }
    }
    out.push(*points.last().unwrap());
    Ok(out)
}
/// Nearest point of `line` within the progress window `[lo, hi]`, as (distance, progress),
/// looking only at the segments of the window, found from the
/// distance walked to each point of `line`.
fn nearest_progress_within(p: Point, line: &[Point], cumulative: &[f64], lo: f64, hi: f64) -> (f64, f64) {
    let mut best = (f64::INFINITY, 0.);
    let first = cumulative.partition_point(|c| *c < lo).saturating_sub(1);
    for i in first..line.len().saturating_sub(1) {
        let walked = cumulative[i];
        if walked > hi {
            break;
        }
        let delta = sub(line[i + 1], line[i]);
        let length = cumulative[i + 1] - walked;
        if length < 1e-8 {
            continue;
        }
        let (t0, t1) = (((lo - walked) / length).max(0.), ((hi - walked) / length).min(1.));
        if t0 > t1 {
            continue;
        }
        let rel = sub(p, line[i]);
        let t = ((rel[0] * delta[0] + rel[1] * delta[1]) / (length * length)).clamp(t0, t1);
        let d = norm(sub(p, add(line[i], mul(delta, t))));
        if d < best.0 {
            best = (d, walked + t * length);
        }
    }
    best
}
fn direction_at(line: &[Point], progress: f64) -> Point {
    let mut walked = 0.;
    let mut direction = [0., 1.];
    for w in line.windows(2) {
        let delta = sub(w[1], w[0]);
        let length = norm(delta);
        if length < 1e-8 {
            continue;
        }
        direction = mul(delta, 1. / length);
        if walked + length > progress {
            return direction;
        }
        walked += length;
    }
    direction
}
/// [`merge_straights`] for the pieces of `family` in `catalog`.
pub fn merge_straight_runs(
    parts: Vec<PlacedPart>,
    catalog: &[CatalogPart],
    family: &str,
    allowed: impl Fn(&CatalogPart, &[Point]) -> bool,
) -> Result<Vec<PlacedPart>> {
    let choices: Vec<_> = catalog
        .iter()
        .filter(|m| m.family == family && [3, 4].contains(&m.category))
        .collect();
    if parts.is_empty() {
        return Ok(parts);
    }
    merge_straights(parts, &choices, allowed)
}
/// Replace every run of consecutive straight pieces by the fewest straights of the same
/// total length, then re-place the chain from its first piece so every joint stays exact.
/// Straights are collinear, so the track keeps its shape while tiny pieces disappear. A
/// merged run whose pieces no longer pass `allowed` (the slope is sampled along each piece,
/// so new piece boundaries sample the terrain elsewhere) is kept as it was.
fn merge_straights(
    parts: Vec<PlacedPart>,
    choices: &[&CatalogPart],
    allowed: impl Fn(&CatalogPart, &[Point]) -> bool,
) -> Result<Vec<PlacedPart>> {
    let straights: Vec<_> = choices.iter().filter(|c| c.category == 3).collect();
    let cm = |m: &CatalogPart| (m.model.length * 100.).round() as usize;
    let find = |name: &str| choices.iter().find(|c| c.path == name).copied();
    // Fewest straights with exactly `total` centimetres, longest first.
    let combine = |total: usize| -> Option<Vec<String>> {
        let mut best: Vec<Option<(usize, usize)>> = vec![None; total + 1];
        best[0] = Some((0, usize::MAX));
        for t in 1..=total {
            for (i, m) in straights.iter().enumerate() {
                let l = cm(m);
                if l > 0 && l <= t
                    && let Some((n, _)) = best[t - l]
                    && best[t].is_none_or(|(k, _)| n + 1 < k)
                {
                    best[t] = Some((n + 1, i));
                }
            }
        }
        best[total]?;
        let mut t = total;
        let mut merged = Vec::new();
        while t > 0 {
            let (_, i) = best[t].unwrap();
            merged.push(straights[i].path.clone());
            t -= cm(straights[i]);
        }
        merged.sort_by_key(|p| std::cmp::Reverse(cm(find(p).unwrap())));
        Some(merged)
    };
    // Runs of two or more straights: (first index, end index).
    let mut runs = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let straight = |k: usize| find(&parts[k].model).is_some_and(|m| m.category == 3);
        let mut j = i;
        while j < parts.len() && straight(j) {
            j += 1;
        }
        if j - i > 1 {
            runs.push((i, j));
        }
        i = j.max(i + 1);
    }
    let first = &parts[0];
    let model = &find(&first.model).context("Brak modelu w katalogu")?.model;
    let entry = model.ports[usize::from(first.reverse)].context("Brak portu modelu")?;
    let angle = -first.rotation.to_radians();
    let start = Port {
        p: add(rotate(entry.p, angle), first.position),
        outward: mul(rotate(entry.outward, angle), -1.),
    };
    let mut keep = vec![false; runs.len()];
    loop {
        // (model, reversed, run it was merged from)
        let mut pieces: Vec<(String, bool, Option<usize>)> = Vec::new();
        let mut next = 0;
        for (r, &(a, b)) in runs.iter().enumerate() {
            pieces.extend(parts[next..a].iter().map(|p| (p.model.clone(), p.reverse, None)));
            let total: usize = parts[a..b].iter().map(|p| cm(find(&p.model).unwrap())).sum();
            match combine(total).filter(|m| !keep[r] && m.len() < b - a) {
                Some(merged) => pieces.extend(merged.into_iter().map(|m| (m, false, Some(r)))),
                None => pieces.extend(parts[a..b].iter().map(|p| (p.model.clone(), p.reverse, None))),
            }
            next = b;
        }
        pieces.extend(parts[next..].iter().map(|p| (p.model.clone(), p.reverse, None)));
        if pieces.len() == parts.len() {
            return Ok(parts);
        }
        let mut at = start;
        let mut out = Vec::with_capacity(pieces.len());
        let mut rejected = false;
        for (name, reverse, run) in pieces {
            let item = find(&name).unwrap();
            let placed = place(item, at, reverse)?;
            at = placed_endpoint(&placed, &item.model)?;
            if let Some(r) = run {
                let world: Vec<_> = item
                    .model
                    .line
                    .iter()
                    .map(|q| add(rotate(*q, -placed.rotation.to_radians()), placed.position))
                    .collect();
                if !allowed(item, &world) {
                    keep[r] = true;
                    rejected = true;
                }
            }
            out.push(placed);
        }
        if !rejected {
            return Ok(out);
        }
    }
}
/// Greedy connector-exact fitting with lookahead. Failure leaves the document untouched.
pub fn fit(
    points: &[Point],
    family: &str,
    catalog: &[CatalogPart],
    s: &RoutingSettings,
    allowed: impl Fn(Point, Point) -> bool,
) -> Result<Vec<PlacedPart>> {
    ensure!(points.len() >= 2, "Wskaż przynajmniej dwa punkty");
    let smoothed = widen(points, s.min_radius, (s.tolerance * 1.2).max(4.))?;
    fit_path(&smoothed, family, catalog, s, allowed)
}
/// Fit models to an already curvature-limited centreline (for example a terrain route)
/// without rounding its corners again, so the road stays on every point of the line.
/// Roads start and end with a `konec` cap. Railway tracks (`Rail_Track*` families) have no
/// caps; their last piece may end up to half the shortest piece before the final point.
pub fn fit_path(
    points: &[Point],
    family: &str,
    catalog: &[CatalogPart],
    s: &RoutingSettings,
    allowed: impl Fn(Point, Point) -> bool,
) -> Result<Vec<PlacedPart>> {
    fit_path_inner(points, family, catalog, s, allowed, None)
}

/// [`fit`] that goes on from `start`, the open end of a road already laid: it keeps that
/// heading and puts no end piece there, so the new models join the old ones.
pub fn fit_from(
    start: Port,
    points: &[Point],
    family: &str,
    catalog: &[CatalogPart],
    s: &RoutingSettings,
    allowed: impl Fn(Point, Point) -> bool,
) -> Result<Vec<PlacedPart>> {
    ensure!(points.len() >= 2, "Wskaż przynajmniej dwa punkty");
    let smoothed = widen(points, s.min_radius, (s.tolerance * 1.2).max(4.))?;
    fit_path_inner(&smoothed, family, catalog, s, allowed, Some(start))
}

/// [`fit_path`] from `start` along a path searched or drawn from it.
pub fn fit_path_from(
    start: Port,
    points: &[Point],
    family: &str,
    catalog: &[CatalogPart],
    s: &RoutingSettings,
    allowed: impl Fn(Point, Point) -> bool,
) -> Result<Vec<PlacedPart>> {
    fit_path_inner(points, family, catalog, s, allowed, Some(start))
}

/// The same parts run the other way: last first, each entered by its other port.
pub fn flip(parts: &[PlacedPart]) -> Vec<PlacedPart> {
    parts.iter().rev().map(|p| PlacedPart { reverse: !p.reverse, ..p.clone() }).collect()
}

/// The open start of a road, pointing away from it.
pub fn startpoint(parts: &[PlacedPart], catalog: &[CatalogPart]) -> Result<Port> {
    let p = parts.first().context("Pusta droga")?;
    let m = &catalog.iter().find(|m| m.path == p.model).context("Brak modelu")?.model;
    let port = m.ports[usize::from(p.reverse)].context("Brak wejścia modelu")?;
    let angle = -p.rotation.to_radians();
    Ok(Port { p: add(rotate(port.p, angle), p.position), outward: rotate(port.outward, angle) })
}

/// True for the end piece (konec) of a road.
pub fn is_cap(part: &PlacedPart, catalog: &[CatalogPart]) -> bool {
    catalog.iter().any(|m| m.path == part.model && m.category == 6)
}

fn fit_path_inner(
    points: &[Point],
    family: &str,
    catalog: &[CatalogPart],
    s: &RoutingSettings,
    allowed: impl Fn(Point, Point) -> bool,
    start: Option<Port>,
) -> Result<Vec<PlacedPart>> {
    ensure!(points.len() >= 2, "Wskaż przynajmniej dwa punkty");
    let track = crate::rails::is_track_family(family);
    let cap = if track {
        None
    } else {
        let cap=catalog.iter().filter(|m|m.family==family && m.category==6)
            .find(|m|m.path.rsplit(['\\','/']).next().unwrap_or("").to_lowercase().contains("konec"))
            .context("Brak segmentu konec MLOD dla wybranego typu drogi. Dodaj go do definicji Road Tool i wczytaj modele")?;
        ensure!(
            cap.model.ports[0].is_some() && cap.model.ports[1].is_some(),
            "Segment konec nie ma obu portów połączenia"
        );
        Some(cap)
    };
    let cap_length = cap.map_or(0., |c| c.model.length);
    let choices: Vec<_> = catalog
        .iter()
        .filter(|m| {
            m.family == family
                && cap.is_none_or(|c| m.road_type_index == c.road_type_index)
                && [3, 4].contains(&m.category)
                && m.model.ports[2].is_none()
                && m.model.ports[3].is_none()
        })
        .collect();
    ensure!(
        !choices.is_empty(),
        "Brak prostych i zakrętów MLOD dla tej rodziny"
    );
    // Without caps the end can only be met to within half the shortest piece.
    let slack = if track {
        choices.iter().map(|m| m.model.length).fold(f64::INFINITY, f64::min) / 2.
    } else {
        0.
    };
    let length: f64 = points.windows(2).map(|w| norm(sub(w[1], w[0]))).sum();
    let end = *points.last().unwrap();
    let initial = sub(points[1], points[0]);
    ensure!(
        length >= if start.is_some() { 1. } else { 2. } * cap_length && norm(initial) > 1e-6,
        "Trasa jest za krótka na dwa segmenty konec lub ma powtórzone punkty"
    );
    let mut at = start.unwrap_or(Port {
        p: points[0],
        outward: mul(initial, 1. / norm(initial)),
    });
    // Search only the stretch of line a single model can cover from the current progress.
    let ahead = choices
        .iter()
        .map(|m| m.model.length)
        .fold(cap_length, f64::max)
        + s.tolerance * 2.
        + 10.;
    let cumulative: Vec<f64> = std::iter::once(0.)
        .chain(points.windows(2).scan(0., |walked, w| {
            *walked += norm(sub(w[1], w[0]));
            Some(*walked)
        }))
        .collect();
    let near = |p: Point, from: f64| nearest_progress_within(p, points, &cumulative, from - s.tolerance * 2., from + ahead);
    let valid_part = |part: &PlacedPart, model: &Model, from: f64| {
        let world: Vec<_> = model
            .line
            .iter()
            .map(|q| add(rotate(*q, -part.rotation.to_radians()), part.position))
            .collect();
        world.len() >= 2
            && world.iter().all(|q| near(*q, from).0 <= s.tolerance)
            && world.windows(2).all(|w| allowed(w[0], w[1]))
    };
    let mut parts = Vec::new();
    if let Some(cap) = cap.filter(|_| start.is_none()) {
        let first = place(cap, at, false)?;
        ensure!(
            valid_part(&first, &cap.model, 0.),
            "Początkowy segment konec nie mieści się na trasie lub nie spełnia ograniczeń terenu"
        );
        at = placed_endpoint(&first, &cap.model)?;
        parts.push(first);
    }
    let start_progress = near(at.p, 0.).1;
    // Best-first over progress: the cheapest few ways of reaching each stretch of the line are
    // carried on, so one piece is chosen for how the whole road goes on and not only for
    // where it ends. The cost follows a Road Tool user: pieces bending as the line bends
    // (gentle curves along gentle bends, straights along straight stretches), no curves
    // turning back and forth, and few pieces.
    struct Node {
        parent: usize,
        part: Option<PlacedPart>,
        at: Port,
        progress: f64,
        cost: f64,
        bend: f64,
        pieces: usize,
        alive: bool,
    }
    const BUCKET: f64 = 2.;
    const KEEP: usize = 6;
    let tol2 = (s.tolerance * s.tolerance).max(1e-6);
    let mut nodes = vec![Node { parent: usize::MAX, part: None, at, progress: start_progress, cost: 0., bend: 0., pieces: 0, alive: true }];
    let mut buckets: std::collections::HashMap<i64, Vec<usize>> = std::collections::HashMap::new();
    let mut queue: std::collections::BinaryHeap<std::cmp::Reverse<(i64, usize)>> = std::collections::BinaryHeap::new();
    queue.push(std::cmp::Reverse(((start_progress * 1000.) as i64, 0)));
    let mut furthest = start_progress;
    let mut best_end: Option<(f64, usize, Option<PlacedPart>)> = None;
    let mut expanded = 0usize;
    while let Some(std::cmp::Reverse((_, id))) = queue.pop() {
        if !nodes[id].alive {
            continue;
        }
        expanded += 1;
        if expanded > 60000 {
            break;
        }
        let (at, progress, cost, bend, pieces) = (nodes[id].at, nodes[id].progress, nodes[id].cost, nodes[id].bend, nodes[id].pieces);
        if best_end.as_ref().is_some_and(|(c, _, _)| cost >= *c) {
            continue;
        }
        furthest = furthest.max(progress);
        if let Some(cap) = cap {
            let last = place(cap, at, false)?;
            let last_end = placed_endpoint(&last, &cap.model)?;
            let miss = norm(sub(last_end.p, end));
            if miss <= s.tolerance
                && near(last_end.p, progress).1 >= length - s.tolerance * 2.
                && valid_part(&last, &cap.model, progress)
            {
                let total = cost + miss * 0.5;
                if best_end.as_ref().is_none_or(|(c, _, _)| total < *c) {
                    best_end = Some((total, id, Some(last)));
                }
            }
        } else if pieces > 0
            && norm(sub(at.p, end)) <= s.tolerance + slack
            && progress >= length - s.tolerance * 2. - slack
        {
            let total = cost + norm(sub(at.p, end)) * 0.5;
            if best_end.as_ref().is_none_or(|(c, _, _)| total < *c) {
                best_end = Some((total, id, None));
            }
        }
        for item in &choices {
            for reverse in [false, true] {
                // Only curves support the native reversed-segment code; a straight track
                // reversed is the same piece.
                if reverse && item.category != 4 {
                    continue;
                }
                let model = &item.model;
                let a = model.ports[0].context("Brak początku")?;
                let b = model.ports[1].context("Brak końca")?;
                let own = (-a.outward[0] * b.outward[1] + a.outward[1] * b.outward[0])
                    .atan2(-a.outward[0] * b.outward[0] - a.outward[1] * b.outward[1]);
                let turn = own.abs();
                if turn > 1e-5 && model.length / turn < s.min_radius - 1e-6 {
                    continue;
                }
                let p = place(item, at, reverse)?;
                let angle = -p.rotation.to_radians();
                let exit = model.ports[1 - usize::from(reverse)].unwrap();
                let next = Port {
                    p: add(rotate(exit.p, angle), p.position),
                    outward: rotate(exit.outward, angle),
                };
                let (deviation, pr) = near(next.p, progress);
                if pr <= progress + 0.01 || deviation > s.tolerance
                    || pr + cap_length > length + s.tolerance
                {
                    continue;
                }
                let mut line = model.line.clone();
                if reverse {
                    line.reverse();
                }
                let world: Vec<_> = line
                    .iter()
                    .map(|q| add(rotate(*q, angle), p.position))
                    .collect();
                let mut off2 = deviation * deviation;
                let mut inside = true;
                for q in &world {
                    let d = near(*q, progress).0;
                    if d > s.tolerance {
                        inside = false;
                        break;
                    }
                    off2 += d * d;
                }
                if !inside || !world.windows(2).all(|w| allowed(w[0], w[1])) {
                    continue;
                }
                off2 /= (world.len() + 1) as f64;
                // The bend of the piece as driven, left positive.
                let bent = (at.outward[0] * next.outward[1] - at.outward[1] * next.outward[0])
                    .atan2(at.outward[0] * next.outward[0] + at.outward[1] * next.outward[1]);
                let (from_dir, to_dir) = (direction_at(points, progress), direction_at(points, pr));
                let wanted = (from_dir[0] * to_dir[1] - from_dir[1] * to_dir[0])
                    .atan2(from_dir[0] * to_dir[0] + from_dir[1] * to_dir[1]);
                let heading = (next.outward[0] * to_dir[1] - next.outward[1] * to_dir[0])
                    .atan2(next.outward[0] * to_dir[0] + next.outward[1] * to_dir[1])
                    .to_degrees();
                let span = pr - progress;
                let mut step = off2 / tol2 * span * 0.6
                    + (bent - wanted).abs().to_degrees() * 0.35
                    + heading * heading * 0.02
                    + 0.8;
                // A curve the other way after one, straights between or not, is the weaving Road
                // Tool users avoid, unless the line itself bends the other way there.
                if bent.abs() > 0.012 && bend.abs() > 0.012 && bent.signum() != bend.signum() {
                    step += if bent * wanted > 0. && wanted.abs() > 0.004 { 1. } else { 5. };
                }
                let total = cost + step;
                let key = (pr / BUCKET).floor() as i64;
                let slot = buckets.entry(key).or_default();
                if slot.len() >= KEEP {
                    let (worst_at, worst) = slot
                        .iter()
                        .enumerate()
                        .map(|(i, n)| (i, nodes[*n].cost))
                        .max_by(|a, b| a.1.total_cmp(&b.1))
                        .unwrap();
                    if total >= worst {
                        continue;
                    }
                    let gone = slot.swap_remove(worst_at);
                    nodes[gone].alive = false;
                }
                let new_bend = if bent.abs() > 0.012 { bent } else { bend };
                nodes.push(Node { parent: id, part: Some(p), at: next, progress: pr, cost: total, bend: new_bend, pieces: pieces + 1, alive: true });
                let n = nodes.len() - 1;
                buckets.get_mut(&key).unwrap().push(n);
                queue.push(std::cmp::Reverse(((pr * 1000.) as i64, n)));
            }
        }
    }
    let Some((_, last, closing)) = best_end else {
        anyhow::bail!(
            "Nie można dopasować modeli przy {:.0} m trasy. Złagodź zakręty, zwiększ tolerancję lub użyj segmentów",
            furthest
        );
    };
    let mut chain = Vec::new();
    let mut k = last;
    while nodes[k].parent != usize::MAX {
        chain.push(nodes[k].part.clone().unwrap());
        k = nodes[k].parent;
    }
    chain.reverse();
    parts.extend(chain);
    match closing {
        Some(c) => {
            parts.push(c);
            Ok(parts)
        }
        None => merge_straights(parts, &choices, |_, line: &[Point]| {
            line.windows(2).all(|w| allowed(w[0], w[1]))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearly_straight_routes_do_not_weave_with_available_curves() {
        let c: Vec<_> = [
            ("asf2_6.p3d", 3), ("asf2_12.p3d", 3), ("asf2_25.p3d", 3),
            ("asf2_10 25.p3d", 4), ("asf2_10 100.p3d", 4),
            ("asf2_6konec.p3d", 6),
        ].into_iter().map(|(name, category)| CatalogPart {
            path: name.into(), family: "asf2".into(), category,
            index: 0, road_type_index: 0,
            model: crate::geometry::filename_model(name).unwrap(),
        }).collect();
        for points in [
            vec![[0., 0.], [0., 200.]],
            vec![[0., 0.], [0., 100.], [1., 200.]],
            vec![[0., 0.], [0., 100.], [2., 200.]],
        ] {
            let settings = RoutingSettings::default();
            let parts = fit(&points, "asf2", &c, &settings, |_, _| true).unwrap();
            assert!(parts.iter().all(|p| !p.model.contains(' ')), "{parts:?}");
            assert!(norm(sub(endpoint(&parts, &c).unwrap().p, *points.last().unwrap())) <= settings.tolerance);
        }
    }
    #[test]
    fn rounded_corner_is_fitted_with_available_curves() {
        let mut c = Vec::new();
        for (name, category) in [
            ("asf2_6.p3d", 3),
            ("asf2_12.p3d", 3),
            ("asf2_25.p3d", 3),
            ("asf2_10 25.p3d", 4),
            ("asf2_30 25.p3d", 4),
            ("asf2_6konec.p3d", 6),
        ] {
            c.push(CatalogPart {
                path: name.into(),
                family: "asf2".into(),
                category,
                index: 0,
                road_type_index: 0,
                model: crate::geometry::filename_model(name).unwrap(),
            });
        }
        let settings = RoutingSettings::default();
        let parts = fit(
            &[[0., 0.], [0., 100.], [100., 100.]],
            "asf2",
            &c,
            &settings,
            |_, _| true,
        )
        .unwrap();
        assert!(
            parts
                .iter()
                .any(|p| p.model.contains("25.p3d") && p.model.contains(' '))
        );
        assert!(norm(sub(endpoint(&parts, &c).unwrap().p, [100., 100.])) <= settings.tolerance);
    }
    #[test]
    fn fit_and_manual_connectors_match() {
        let mut c = vec![CatalogPart {
            path: "asf2_25.p3d".into(),
            family: "asf2".into(),
            category: 3,
            index: 0,
            road_type_index: 0,
            model: crate::geometry::filename_model("asf2_25.p3d").unwrap(),
        }];
        c.push(CatalogPart {
            path: "asf2_6konec.p3d".into(),
            family: "asf2".into(),
            category: 6,
            index: 0,
            road_type_index: 0,
            model: crate::geometry::filename_model("asf2_6konec.p3d").unwrap(),
        });
        let p = fit(
            &[[200000., 0.], [200000., 112.5]],
            "asf2",
            &c,
            &RoutingSettings::default(),
            |_, _| true,
        )
        .unwrap();
        assert_eq!(p.len(), 6);
        assert_eq!(p.first().unwrap().model, "asf2_6konec.p3d");
        assert_eq!(p.last().unwrap().model, "asf2_6konec.p3d");
        assert_eq!(endpoint(&p, &c).unwrap().p, [200000., 112.5]);
        for i in 1..p.len() {
            let at = endpoint(&p[..i], &c).unwrap();
            let item = c.iter().find(|m| m.path == p[i].model).unwrap();
            let expected = place(item, at, p[i].reverse).unwrap();
            assert!(norm(sub(expected.position, p[i].position)) < 1e-8);
        }
        assert!(
            fit(
                &[[0., 0.], [0., 5.]],
                "asf2",
                &c,
                &RoutingSettings::default(),
                |_, _| true
            )
            .is_err()
        );
        assert!(
            fit(
                &[[0., 0.], [0., 112.5]],
                "asf2",
                &c[..1],
                &RoutingSettings::default(),
                |_, _| true
            )
            .is_err()
        );
        assert!(
            fit(
                &[[0., 0.], [0., 112.5]],
                "asf2",
                &c,
                &RoutingSettings::default(),
                |a, _| a[1] >= 7.
            )
            .is_err()
        );
        assert!(
            fit(
                &[[0., 0.], [100., 100.], [0., 100.]],
                "asf2",
                &c,
                &RoutingSettings::default(),
                |_, _| true
            )
            .is_err()
        );
    }
}
