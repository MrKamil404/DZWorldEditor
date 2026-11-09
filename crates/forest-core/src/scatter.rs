//! Silnik rozrzutu roślinności: Poisson-disk (dart throwing z siatką przestrzenną),
//! szum polan, filtry wysokości/spadku, wykluczenia i proporcje gatunków.
//!
//! Źródła punktów (wspólna siatka odstępów):
//! - strefy z maski (kolory),
//! - obszary rysowane ręcznie (poligony, `areas`),
//! - pas graniczny lasu (`edges`) — krzewy/podrost wzdłuż krawędzi.

use std::collections::HashMap;

use rand::prelude::*;

use crate::geojson::{point_in_polygon, point_ring_distance, GeoJsonData, Polygon};
use crate::heightmap::AscHeightmap;
use crate::mask::{MaskImage, Rgb8};
use crate::preset::{ElevationMode, ForestProject, MixEntry};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PlacedObject {
    pub model: String,
    /// Współrzędne świata w metrach (origin SW, bez offsetu easting).
    pub x: f64,
    pub y: f64,
    pub yaw: f64,
    pub pitch: f64,
    pub roll: f64,
    pub scale: f64,
    pub elevation: f64,
    pub zone_index: usize,
    pub species_index: usize,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct GenStats {
    pub total: usize,
    pub per_species: Vec<(String, usize)>,
    pub per_source: Vec<(String, usize)>,
    /// Obiekty pasa granicznego.
    pub edge_count: usize,
    /// Obiekty usunięte przez strefy wycinania (kolor maski + bufor).
    pub removed_cut: usize,
    pub rejected_spacing: usize,
    pub rejected_filters: usize,
    pub rejected_exclusion: usize,
    pub rejected_clearing: usize,
    pub elapsed_ms: u128,
}

impl GenStats {
    pub fn summary(&self) -> String {
        let mut s = format!(
            "Wygenerowano {} obiektów w {} ms",
            self.total, self.elapsed_ms
        );
        if !self.per_source.is_empty() {
            s.push_str("\nStrefy/obszary:");
            for (n, c) in &self.per_source {
                s.push_str(&format!("\n  {n}: {c}"));
            }
        }
        if self.edge_count > 0 {
            s.push_str(&format!("\nGranica lasu: {}", self.edge_count));
        }
        if self.removed_cut > 0 {
            s.push_str(&format!(
                "\nWycięto strefami kolorów: {}",
                self.removed_cut
            ));
        }
        s.push_str("\nGatunki:");
        for (n, c) in &self.per_species {
            if *c > 0 {
                s.push_str(&format!("\n  {n}: {c}"));
            }
        }
        let rej = self.rejected_spacing
            + self.rejected_filters
            + self.rejected_exclusion
            + self.rejected_clearing;
        if rej > 0 {
            s.push_str(&format!(
                "\nOdrzucone: {rej} (odstęp {}, filtry {}, wykluczenia {}, polany {})",
                self.rejected_spacing,
                self.rejected_filters,
                self.rejected_exclusion,
                self.rejected_clearing
            ));
        }
        s
    }
}

/// Callback postępu: argument = ułamek 0..1.
pub type Progress<'a> = &'a dyn Fn(f64);

/// Limit liczby obiektów (generowanie i import) — ochrona przed zalewaniem pamięci.
pub const MAX_TOTAL_OBJECTS: usize = 2_000_000;
/// Limit liczby stref z maski (mapa identyfikatorów pikseli to u8).
const MAX_ZONES: usize = 254;

// --- Środowisko filtrów -----------------------------------------------------

enum Rej {
    Filters,
    Exclusion,
    Clearing,
}

struct Env<'a> {
    project: &'a ForestProject,
    hm: Option<&'a AscHeightmap>,
    ex: Option<&'a GeoJsonData>,
    noise_seed: u64,
    use_clearings: bool,
}

impl Env<'_> {
    fn eval(&self, wx: f64, wy: f64) -> Result<f64, Rej> {
        let p = self.project;
        let pad = p.edge_padding_m.max(0.0);
        let size = p.map_size_m;
        if wx < pad || wy < pad || wx > size - pad || wy > size - pad {
            return Err(Rej::Filters);
        }
        if self.use_clearings {
            let nx = wx / p.clearing_scale_m;
            let ny = wy / p.clearing_scale_m;
            if fbm2(nx, ny, self.noise_seed) < clearing_threshold(p.clearing_strength) {
                return Err(Rej::Clearing);
            }
        }
        let elevation;
        if let Some(hm) = self.hm {
            match hm.sample(wx, wy) {
                Some(h) => {
                    if let Some(mn) = p.min_altitude {
                        if h < mn {
                            return Err(Rej::Filters);
                        }
                    }
                    if let Some(mx) = p.max_altitude {
                        if h > mx {
                            return Err(Rej::Filters);
                        }
                    }
                    if let Some(max_slope) = p.max_slope_deg {
                        match hm.slope_deg(wx, wy) {
                            Some(s) if s <= max_slope => {}
                            _ => return Err(Rej::Filters),
                        }
                    }
                    elevation = match p.elevation_mode {
                        ElevationMode::AbsoluteSampled => h,
                        ElevationMode::RelativeZero => 0.0,
                    };
                }
                None => return Err(Rej::Filters),
            }
        } else {
            elevation = 0.0;
        }
        if let Some(ex) = self.ex {
            if ex.contains(wx, wy) {
                return Err(Rej::Exclusion);
            }
        }
        Ok(elevation)
    }
}

// --- Pomocnicze --------------------------------------------------------------

fn cumulative(weights: &[(usize, f32)]) -> Vec<(usize, f64)> {
    let total: f64 = weights.iter().map(|(_, w)| f64::from(*w)).sum();
    let mut acc = 0.0;
    weights
        .iter()
        .map(|(si, w)| {
            acc += f64::from(*w) / total;
            (*si, acc)
        })
        .collect()
}

fn pick_species(cum: &[(usize, f64)], rng: &mut StdRng) -> usize {
    debug_assert!(!cum.is_empty(), "pick_species: pusta lista wag");
    let pick: f64 = rng.gen();
    cum.iter()
        .find(|(_, a)| pick <= *a)
        .map_or(cum[cum.len() - 1].0, |(si, _)| *si)
}

/// Pole poligonu (shoelace). Dla wielokątów samoprzecinających się wynik
/// jest bez sensu — używaj po sprawdzeniu `polygon_self_intersects`.
pub fn polygon_area_m2(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut j = n - 1;
    for i in 0..n {
        sum += (ring[j][0] + ring[i][0]) * (ring[j][1] - ring[i][1]);
        j = i;
    }
    (sum / 2.0).abs()
}

/// Pole obrysu pomniejszone o dziury (podstawa celu gęstości).
pub fn polygon_net_area_m2(ring: &[[f64; 2]], holes: &[Vec<[f64; 2]>]) -> f64 {
    polygon_area_m2(ring) - holes.iter().map(|h| polygon_area_m2(h)).sum::<f64>()
}

fn polygon_area(ring: &[[f64; 2]]) -> f64 {
    polygon_area_m2(ring)
}

fn cross(ox: f64, oy: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    (ax - ox) * (by - oy) - (ay - oy) * (bx - ox)
}

fn on_segment(ax: f64, ay: f64, bx: f64, by: f64, px: f64, py: f64) -> bool {
    // p leży na odcinku ab (zakładając collinearity)
    px.min(bx) >= ax.min(bx) - 1e-9
        && px.max(bx) <= ax.max(bx) + 1e-9
        && py.min(by) >= ay.min(by) - 1e-9
        && py.max(by) <= ay.max(by) + 1e-9
}

/// Zbieżność punktów z tolerancją na zaokrąglenia importu.
fn same_pt(p: &[f64; 2], q: &[f64; 2]) -> bool {
    (p[0] - q[0]).abs() < 1e-9 && (p[1] - q[1]).abs() < 1e-9
}

/// Zwraca punkt przecięcia/styku odcinków albo None, gdy się nie dotykają.
#[allow(clippy::many_single_char_names)]
fn segments_intersection_point(
    a1: &[f64; 2],
    a2: &[f64; 2],
    b1: &[f64; 2],
    b2: &[f64; 2],
) -> Option<[f64; 2]> {
    let d1 = cross(b1[0], b1[1], b2[0], b2[1], a1[0], a1[1]);
    let d2 = cross(b1[0], b1[1], b2[0], b2[1], a2[0], a2[1]);
    let d3 = cross(a1[0], a1[1], a2[0], a2[1], b1[0], b1[1]);
    let d4 = cross(a1[0], a1[1], a2[0], a2[1], b2[0], b2[1]);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        // klasyczne przecięcie: przecięcie prostych
        let (rx, ry) = (a2[0] - a1[0], a2[1] - a1[1]);
        let (sx, sy) = (b2[0] - b1[0], b2[1] - b1[1]);
        let denom = rx * sy - ry * sx;
        if denom.abs() > f64::EPSILON {
            let t = ((b1[0] - a1[0]) * sy - (b1[1] - a1[1]) * sx) / denom;
            return Some([a1[0] + t * rx, a1[1] + t * ry]);
        }
        return Some([(a1[0] + a2[0]) * 0.5, (a1[1] + a2[1]) * 0.5]);
    }
    // styk (collinear / wspólny punkt) też traktujemy jako przecięcie
    if d1.abs() < 1e-12 && on_segment(b1[0], b1[1], b2[0], b2[1], a1[0], a1[1]) {
        return Some([a1[0], a1[1]]);
    }
    if d2.abs() < 1e-12 && on_segment(b1[0], b1[1], b2[0], b2[1], a2[0], a2[1]) {
        return Some([a2[0], a2[1]]);
    }
    if d3.abs() < 1e-12 && on_segment(a1[0], a1[1], a2[0], a2[1], b1[0], b1[1]) {
        return Some([b1[0], b1[1]]);
    }
    if d4.abs() < 1e-12 && on_segment(a1[0], a1[1], a2[0], a2[1], b2[0], b2[1]) {
        return Some([b2[0], b2[1]]);
    }
    None
}

/// Lokalizacja samoprzecięcia obrysu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelfIntersection {
    /// Odcinek A rozpoczyna się w wierzchołku `seg_a` (kończy w seg_a+1).
    pub seg_a: usize,
    /// Odcinek B rozpoczyna się w wierzchołku `seg_b` (kończy w seg_b+1).
    pub seg_b: usize,
    /// Punkt przecięcia [x, y] w metrach mapy.
    pub point: [f64; 2],
}

/// Szuka PIERWSZEGO samoprzecięcia obrysu i podaje jego lokalizację
/// (indeksy odcinków + współrzędne punktu). O(n²) z wcześniejsym wyjściem.
///
/// Domknięcie pierścienia (pierwszy punkt powtórzony na końcu — wymagane
/// przez GeoJSON i Shapefile) NIE jest samoprzecięciem.
pub fn find_self_intersection(ring: &[[f64; 2]]) -> Option<SelfIntersection> {
    // usuń duplikat domykający: ostatni punkt == pierwszy
    let mut m = ring.len();
    if m >= 2 && same_pt(&ring[0], &ring[m - 1]) {
        m -= 1;
    }
    if m < 3 {
        return None;
    }
    for i in 0..m {
        let a1 = ring[i];
        let a2 = ring[(i + 1) % m];
        if same_pt(&a1, &a2) {
            continue; // zerowy odcinek — pomijamy
        }
        for j in (i + 1)..m {
            // sąsiednie odcinki współdzielą wierzchołek — to legalne
            if j == i + 1 || (i == 0 && j == m - 1) {
                continue;
            }
            let b1 = ring[j];
            let b2 = ring[(j + 1) % m];
            if let Some(point) = segments_intersection_point(&a1, &a2, &b1, &b2) {
                return Some(SelfIntersection {
                    seg_a: i,
                    seg_b: j,
                    point,
                });
            }
        }
    }
    None
}

/// Wykrywa samoprzecięcia obrysu (O(n2)). Wielokąty samoprzecinające się
/// łamią ray-casting i shoelace'a — muszą być poprawione przez użytkownika.
pub fn polygon_self_intersects(ring: &[[f64; 2]]) -> bool {
    find_self_intersection(ring).is_some()
}

fn polygon_perimeter(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    if n < 2 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut j = n - 1;
    for i in 0..n {
        sum += (ring[i][0] - ring[j][0]).hypot(ring[i][1] - ring[j][1]);
        j = i;
    }
    sum
}

pub(crate) fn bbox_of(ring: &[[f64; 2]]) -> (f64, f64, f64, f64) {
    let first = ring.first().copied().unwrap_or([0.0, 0.0]);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (first[0], first[1], first[0], first[1]);
    for p in ring {
        min_x = min_x.min(p[0]);
        min_y = min_y.min(p[1]);
        max_x = max_x.max(p[0]);
        max_y = max_y.max(p[1]);
    }
    (min_x, min_y, max_x, max_y)
}

/// Kandydat: losowy piksel zbioru + jitter w obrębie piksela -> współrzędne świata.
fn make_px_cand<'a>(
    set: &'a [u32],
    width: u32,
    size: f64,
    ps: f64,
) -> impl FnMut(&mut StdRng) -> Option<(f64, f64)> + 'a {
    move |rng: &mut StdRng| {
        if set.is_empty() {
            return None;
        }
        let idx = set[rng.gen_range(0..set.len())];
        let px = (idx % width) as f64;
        let row = (idx / width) as f64;
        Some((
            (px + 0.5 + rng.gen_range(-0.5..0.5)) * ps,
            size - (row + 0.5 + rng.gen_range(-0.5..0.5)) * ps,
        ))
    }
}

/// Kandydat: losowy piksel zbioru + jitter, z bramką gęstości wg głębokości
/// od krawędzi strefy (wtapianie w las). Głębokość spoza mapy = pełna gęstość.
fn make_px_cand_blend<'a>(
    set: &'a [u32],
    width: u32,
    size: f64,
    ps: f64,
    depth: &'a HashMap<u32, u8>,
    blend_inside: f64,
) -> impl FnMut(&mut StdRng) -> Option<(f64, f64)> + 'a {
    move |rng: &mut StdRng| {
        if set.is_empty() {
            return None;
        }
        let idx = set[rng.gen_range(0..set.len())];
        let px = (idx % width) as f64;
        let row = (idx / width) as f64;
        let wx = (px + 0.5 + rng.gen_range(-0.5..0.5)) * ps;
        let wy = size - (row + 0.5 + rng.gen_range(-0.5..0.5)) * ps;

        if blend_inside > 0.0 {
            let d_px = depth.get(&idx).copied().unwrap_or(u8::MAX);
            let s = f64::from(d_px) * ps;
            if s < blend_inside {
                let t = (s / blend_inside).clamp(0.0, 1.0);
                if rng.gen::<f64>() > smoothstep(t) {
                    return None;
                }
            }
        }
        Some((wx, wy))
    }
}

/// Odległość każdego piksela strefy od najbliższej krawędzi tej strefy [piks],
/// ograniczona do `max_px` (BFS wieloźródłowy per strefa).
fn mask_edge_depth(
    width: u32,
    height: u32,
    zone_id: &[u8],
    max_px: usize,
) -> HashMap<u32, u8> {
    let mut dist: HashMap<u32, u8> = HashMap::new();
    let mut frontier: Vec<u32> = Vec::new();
    let w = width as usize;

    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) as usize;
            let c = zone_id[i];
            if c == 255 {
                continue;
            }
            let edge = x == 0
                || y == 0
                || x + 1 >= width
                || y + 1 >= height
                || zone_id[i - 1] != c
                || zone_id[i + 1] != c
                || zone_id[i - w] != c
                || zone_id[i + w] != c;
            if edge && dist.insert(i as u32, 0).is_none() {
                frontier.push(i as u32);
            }
        }
    }

    for level in 0..max_px {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for &idx in &frontier {
            let x = idx % width;
            let y = idx / width;
            let ci = idx as usize;
            let cz = zone_id[ci];
            let mut neigh: [Option<usize>; 4] = [None; 4];
            if x > 0 {
                neigh[0] = Some(ci - 1);
            }
            if x + 1 < width {
                neigh[1] = Some(ci + 1);
            }
            if y > 0 {
                neigh[2] = Some(ci - w);
            }
            if y + 1 < height {
                neigh[3] = Some(ci + w);
            }
            for n in neigh.into_iter().flatten() {
                if zone_id[n] == cz && dist.insert(n as u32, (level + 1) as u8).is_none() {
                    next.push(n as u32);
                }
            }
        }
        frontier = next;
    }

    dist
}

// --- Pas graniczny na masce (dylatacja BFS po pikselach strefy) ----------------

fn mask_edge_band(
    width: u32,
    height: u32,
    zone_id: &[u8],
    zi: u8,
    zone_pixels: &[u32],
    band_px: usize,
) -> Vec<(u32, u8)> {
    let mut dist: HashMap<u32, u8> = HashMap::new();
    let mut frontier: Vec<u32> = Vec::new();

    for &idx in zone_pixels {
        let x = idx % width;
        let y = idx / width;
        let i = idx as usize;
        let edge = x == 0
            || y == 0
            || x + 1 >= width
            || y + 1 >= height
            || zone_id[i - 1] != zi
            || zone_id[i + 1] != zi
            || zone_id[i - width as usize] != zi
            || zone_id[i + width as usize] != zi;
        if edge && dist.insert(idx, 0).is_none() {
            frontier.push(idx);
        }
    }

    for level in 0..band_px {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for &idx in &frontier {
            let x = idx % width;
            let y = idx / width;
            let mut neigh: [Option<usize>; 4] = [None; 4];
            if x > 0 {
                neigh[0] = Some(idx as usize - 1);
            }
            if x + 1 < width {
                neigh[1] = Some(idx as usize + 1);
            }
            if y > 0 {
                neigh[2] = Some(idx as usize - width as usize);
            }
            if y + 1 < height {
                neigh[3] = Some(idx as usize + width as usize);
            }
            for n in neigh.into_iter().flatten() {
                if zone_id[n] == zi && dist.insert(n as u32, (level + 1) as u8).is_none() {
                    next.push(n as u32);
                }
            }
        }
        frontier = next;
    }

    // Kandydaci są losowani po indeksie tej listy, a kolejność iteracji HashMap
    // zmienia się przy każdym uruchomieniu — bez sortowania seed nic by nie znaczył.
    let mut band: Vec<(u32, u8)> = dist.into_iter().collect();
    band.sort_unstable_by_key(|(idx, _)| *idx);
    band
}

/// Liczba warstw pasa granicznego (strefy o różnej gęstości — wtapianie).
const EDGE_STRIPS: usize = 5;

/// Waga warstwy `k` z `K` (t = odległość znormalizowana 0..1 od granicy).
/// Blend: najgęściej przy samej granicy, zanik do zewnątrz/podrostu.
fn strip_weight(k: usize, k_total: usize, blend: bool) -> f64 {
    if !blend || k_total <= 1 {
        return 1.0;
    }
    let t = (k as f64 + 0.5) / k_total as f64;
    0.5 * (1.0 + (std::f64::consts::PI * t).cos())
}

/// Szum poszarpania granicy dla zadanej amplitudy i długości fali.
fn make_wob(jag: f64, wl: f64, seed: u64) -> std::sync::Arc<dyn Fn(f64, f64) -> f64> {
    std::sync::Arc::new(move |x: f64, y: f64| {
        if jag <= 0.0 {
            0.0
        } else {
            jag * (2.0 * fbm2(x / wl, y / wl, seed) - 1.0)
        }
    })
}

/// Szum decydujący, które miejsca pasa granicznego należą do placków.
fn make_patch(size: f64, cover: f64, seed: u64) -> std::sync::Arc<dyn Fn(f64, f64) -> bool> {
    std::sync::Arc::new(move |x: f64, y: f64| {
        size <= 0.0 || cover >= 0.99 || fbm2(x / size, y / size, seed ^ 0x51A7) < 0.5 + (cover - 0.5) * 0.4
    })
}

fn sat_color_ok(
    sat: Option<&MaskImage>,
    size: f64,
    x: f64,
    y: f64,
    samples: &[Rgb8],
    tol: u32,
    invert: bool,
) -> bool {
    if samples.is_empty() {
        return true;
    }
    let Some(s) = sat else {
        return true;
    };
    let fx = (x / size) * f64::from(s.width);
    let fy = ((size - y) / size) * f64::from(s.height);
    let px_i = (fx.floor().max(0.0) as u32).min(s.width - 1);
    let py_i = (fy.floor().max(0.0) as u32).min(s.height - 1);
    let pc = s.pixel(px_i, py_i);
    let matched = samples.iter().any(|sm| {
        (i32::from(pc.0[0]) - i32::from(sm.0[0])).unsigned_abs() as u32
            + (i32::from(pc.0[1]) - i32::from(sm.0[1])).unsigned_abs() as u32
            + (i32::from(pc.0[2]) - i32::from(sm.0[2])).unsigned_abs() as u32
            <= tol
    });
    if invert { !matched } else { matched }
}

fn perlin_pick(n: f64, shares: &[f32]) -> usize {
    let sum: f32 = shares.iter().copied().sum();
    if sum <= 0.0 || shares.is_empty() {
        return 0;
    }
    let t = (n.clamp(0.0, 1.0) as f32) * sum;
    let mut acc = 0.0f32;
    for (i, s) in shares.iter().enumerate() {
        acc += *s;
        if t <= acc {
            return i;
        }
    }
    shares.len() - 1
}

fn mix_entry_ready(e: &MixEntry) -> bool {
    e.density_per_ha > 0.0 && !e.species_weights.is_empty()
}

/// Efektywna gęstość i wagi pasa granicznego: miks presetów (blend udziałów),
/// a gdy miks pusty/niegotowy — ręczna lista gatunków.
fn edge_effective(e: &crate::preset::EdgeSettings) -> (f32, Vec<(usize, f32)>) {
    let ready: Vec<&MixEntry> = e.preset_mix.iter().filter(|m| mix_entry_ready(m)).collect();
    if let Some((d, w)) = blend_mix_entries(&ready) {
        (d, w)
    } else {
        (e.density_per_ha, e.species_weights.clone())
    }
}

fn blend_mix_entries(entries: &[&MixEntry]) -> Option<(f32, Vec<(usize, f32)>)> {
    let mut total_share = 0.0f32;
    let mut density = 0.0f32;
    let mut wmap: HashMap<usize, f32> = HashMap::new();
    for e in entries {
        if !mix_entry_ready(e) {
            continue;
        }
        let s = e.share.max(0.0);
        total_share += s;
        density += e.density_per_ha * s;
        for (i, w) in &e.species_weights {
            *wmap.entry(*i).or_insert(0.0) += *w * s;
        }
    }
    if total_share <= 0.0 || wmap.is_empty() {
        return None;
    }
    // Kolejność wag decyduje o wyniku `pick_species`, więc musi być stała.
    let mut weights: Vec<(usize, f32)> = wmap.into_iter().filter(|(_, w)| *w > 0.0).collect();
    weights.sort_unstable_by_key(|(si, _)| *si);
    Some((density / total_share, weights))
}

fn area_color_for_entry(area_cf: Option<&crate::preset::ColorFilter>, e: Option<&MixEntry>) -> (Vec<Rgb8>, u32, bool) {
    if let Some(e) = e {
        if let Some(cf) = e.color_filter.as_ref() {
            if !cf.samples.is_empty() {
                return (cf.samples.clone(), cf.tolerance, cf.invert);
            }
        }
    }
    area_cf
        .map(|f| (f.samples.clone(), f.tolerance, f.invert))
        .unwrap_or_else(|| (Vec::new(), 0, false))
}

#[allow(clippy::too_many_arguments)]
fn push_area_interior_job<'a>(
    jobs: &mut Vec<Job<'a>>,
    total_target: &mut usize,
    max_min_dist: &mut f64,
    spacing: f64,
    label: String,
    source_index: usize,
    density_per_ha: f32,
    species_weights: &[(usize, f32)],
    area_m2: f64,
    area_frac: f64,
    poly: Polygon,
    ring: Vec<[f64; 2]>,
    bbox: (f64, f64, f64, f64),
    wob: std::sync::Arc<dyn Fn(f64, f64) -> f64>,
    blend_in: f64,
    sat: Option<&'a MaskImage>,
    size: f64,
    cf_samples: Vec<Rgb8>,
    cf_tol: u32,
    cf_invert: bool,
    perlin: Option<(usize, Vec<f32>, f64, u64)>,
) {
    if species_weights.is_empty() || density_per_ha <= 0.0 {
        return;
    }
    let eff_area = (area_m2 * area_frac).max(0.0);
    let target = (eff_area / 10_000.0 * f64::from(density_per_ha)).round() as usize;
    if target == 0 || eff_area <= 0.0 {
        return;
    }
    let d = plan_min_dist(spacing, eff_area, target);
    *max_min_dist = (*max_min_dist).max(d);
    *total_target += target;
    let (min_x, min_y, max_x, max_y) = bbox;
    jobs.push(Job {
        label,
        target,
        min_dist: d,
        cum: cumulative(species_weights),
        source_index,
        is_edge: false,
        cand: Box::new(move |rng: &mut StdRng| {
            for _ in 0..24 {
                let x = rng.gen_range(min_x..max_x);
                let y = rng.gen_range(min_y..max_y);
                let inside = point_in_polygon(&poly, x, y);
                let dist = point_ring_distance(x, y, &ring);
                let signed = if inside { dist } else { -dist };
                if signed > wob(x, y) {
                    if blend_in > 0.0 && signed >= 0.0 && signed < blend_in {
                        let t = (signed / blend_in).clamp(0.0, 1.0);
                        if rng.gen::<f64>() > smoothstep(t) {
                            continue;
                        }
                    }
                    if let Some((idx, shares, scale, seed)) = &perlin {
                        if *scale > 0.0 && shares.len() > 1 {
                            let n = fbm2(x / *scale, y / *scale, *seed);
                            if perlin_pick(n, shares) != *idx {
                                continue;
                            }
                        }
                    }
                    if !sat_color_ok(sat, size, x, y, &cf_samples, cf_tol, cf_invert) {
                        continue;
                    }
                    return Some((x, y));
                }
            }
            None
        }),
    });
}

// --- Wspólny dart throwing ----------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn run_dart(
    env: &Env<'_>,
    rng: &mut StdRng,
    grid: &mut HashMap<(i64, i64), Vec<[f32; 2]>>,
    cell: f64,
    objects: &mut Vec<PlacedObject>,
    stats: &mut GenStats,
    target: usize,
    start_min_dist: f64,
    cum: &[(usize, f64)],
    source_index: usize,
    candidate: &mut dyn FnMut(&mut StdRng) -> Option<(f64, f64)>,
    progress: Progress,
    prog_base: f64,
    prog_width: f64,
) -> usize {
    let mut accepted = 0usize;
    let mut min_dist = start_min_dist.max(0.05);

    for _pass in 0..5 {
        let cap = target.saturating_mul(15).max(30_000);
        let mut attempts = 0usize;
        while accepted < target && attempts < cap {
            attempts += 1;
            if attempts % 8192 == 0 {
                progress(prog_base + prog_width * (accepted as f64 / target.max(1) as f64));
            }

            let Some((wx, wy)) = candidate(rng) else {
                continue;
            };

            let elevation = match env.eval(wx, wy) {
                Ok(e) => e,
                Err(Rej::Filters) => {
                    stats.rejected_filters += 1;
                    continue;
                }
                Err(Rej::Exclusion) => {
                    stats.rejected_exclusion += 1;
                    continue;
                }
                Err(Rej::Clearing) => {
                    stats.rejected_clearing += 1;
                    continue;
                }
            };

            let p = [wx as f32, wy as f32];
            if grid_has_neighbor(grid, cell, p, min_dist) {
                stats.rejected_spacing += 1;
                continue;
            }

            let si = pick_species(cum, rng);
            let sp = &env.project.species[si];

            grid.entry(world_to_cell(p, cell)).or_default().push(p);
            // Kolejność losowań jak dotąd: yaw, pitch, roll, skala gatunku…
            let yaw = rng.gen_range(0.0..360.0);
            // Przechył 0 to poprawne ustawienie, ale daje pusty zakres losowania.
            let tilt = f64::from(sp.tilt_max_deg);
            let (pitch, roll) = if tilt > 0.0 {
                (rng.gen_range(-tilt..tilt), rng.gen_range(-tilt..tilt))
            } else {
                (0.0, 0.0)
            };
            let base_scale =
                rng.gen_range(f64::from(sp.scale_min)..=f64::from(sp.scale_max));
            // …plus globalny mnożnik skali. Neutralny 1.0..1.0 nie rusza RNG,
            // więc stare projekty dają bit-identyczne wyniki.
            let glo_min = f64::from(env.project.scale_min);
            let glo_max = f64::from(env.project.scale_max);
            let global_mult = if (glo_min - 1.0).abs() < 1e-9 && (glo_max - 1.0).abs() < 1e-9
            {
                1.0
            } else {
                let (lo, hi) = if glo_min <= glo_max {
                    (glo_min, glo_max)
                } else {
                    (glo_max, glo_min)
                };
                if (hi - lo).abs() < f64::EPSILON {
                    lo
                } else {
                    rng.gen_range(lo..=hi)
                }
            };
            objects.push(PlacedObject {
                model: sp.model.clone(),
                x: wx,
                y: wy,
                yaw,
                pitch,
                roll,
                scale: base_scale * global_mult,
                elevation,
                zone_index: source_index,
                species_index: si,
            });
            accepted += 1;
            stats.total += 1;
            stats.per_species[si].1 += 1;
        }
        if accepted >= target {
            break;
        }
        min_dist *= 0.85;
    }
    accepted
}

struct Job<'a> {
    label: String,
    target: usize,
    min_dist: f64,
    cum: Vec<(usize, f64)>,
    source_index: usize,
    is_edge: bool,
    cand: Box<dyn FnMut(&mut StdRng) -> Option<(f64, f64)> + 'a>,
}

/// Pochodzenie obiektów: pozwala przeliczyć statystyki po wycinaniu.
/// Indeks zadania jest zgodny z `GenStats::per_source`, które rośnie w kolejności
/// wykonywania — dlatego działa przy dowolnej `generation_order`.
#[derive(Default)]
struct JobTrace {
    /// Równoległe do listy obiektów: indeks zadania, które utworzyło obiekt.
    object_job: Vec<u32>,
    job_is_edge: Vec<bool>,
}

impl JobTrace {
    fn record_job(&mut self, accepted: usize, is_edge: bool) {
        let job = self.job_is_edge.len() as u32;
        self.job_is_edge.push(is_edge);
        self.object_job.resize(self.object_job.len() + accepted, job);
    }

    /// Zostawia obiekty spełniające `keep` (w tej samej kolejności) i przelicza statystyki.
    fn retain_objects(
        &mut self,
        objects: &mut Vec<PlacedObject>,
        stats: &mut GenStats,
        mut keep: impl FnMut(&PlacedObject) -> bool,
    ) {
        let before = objects.len();
        let mut kept = 0;
        for i in 0..before {
            if keep(&objects[i]) {
                objects.swap(kept, i);
                self.object_job[kept] = self.object_job[i];
                kept += 1;
            }
        }
        objects.truncate(kept);
        self.object_job.truncate(kept);

        stats.removed_cut += before - kept;
        stats.total = kept;
        stats.edge_count = 0;
        stats.per_species.iter_mut().for_each(|(_, c)| *c = 0);
        stats.per_source.iter_mut().for_each(|(_, c)| *c = 0);
        for (o, &job) in objects.iter().zip(&self.object_job) {
            let job = job as usize;
            stats.per_species[o.species_index].1 += 1;
            stats.per_source[job].1 += 1;
            if self.job_is_edge[job] {
                stats.edge_count += 1;
            }
        }
    }
}

/// Plan minimalnego odstępu dla celu gęstości (nasycenie RSA ~0.7*A/d^2).
fn plan_min_dist(spacing: f64, area_m2: f64, target: usize) -> f64 {
    spacing.max(0.05) * (0.7 * area_m2 / target.max(1) as f64).sqrt()
}

// --- Główne wejście -----------------------------------------------------------

pub fn generate(
    project: &ForestProject,
    mask: Option<&MaskImage>,
    satellite: Option<&MaskImage>,
    heightmap: Option<&AscHeightmap>,
    exclusions: Option<&GeoJsonData>,
    progress: Progress,
) -> Result<(Vec<PlacedObject>, GenStats), String> {
    let t0 = std::time::Instant::now();
    project.validate()?;
    if !project.zones.is_empty() && mask.is_none() {
        return Err("Projekt zawiera strefy z maski, ale maska nie jest wczytana".into());
    }
    if let Some(m) = mask {
        if m.width < 2 || m.height < 2 {
            return Err("Maska jest zbyt mała".into());
        }
        let mask_used = (project.use_mask_zones && !project.zones.is_empty())
            || !project.cut_zones.is_empty();
        if mask_used && m.width != m.height {
            return Err(format!(
                "Maska musi być kwadratowa jak mapa — wczytana ma {}x{} px",
                m.width, m.height
            ));
        }
    }
    if heightmap.is_none() {
        if project.elevation_mode == ElevationMode::AbsoluteSampled {
            return Err("Wysokość absolutna wymaga heightmapy ASC — wczytaj ją albo wybierz tryb względny".into());
        }
        if project.min_altitude.is_some() || project.max_altitude.is_some() {
            return Err("Filtr wysokości wymaga heightmapy ASC — wczytaj ją albo wyłącz limity wysokości".into());
        }
    }
    if project.zones.len() > MAX_ZONES {
        return Err(format!("Maks. liczba stref z maski: {MAX_ZONES}"));
    }

    let size = project.map_size_m;

    // --- Klasyfikacja pikseli maski ------------------------------------------
    let mut zone_pixels: Vec<Vec<u32>> = vec![Vec::new(); project.zones.len()];
    let mut zone_id: Vec<u8> = Vec::new();

    if let Some(mask) = mask {
        let palette: Vec<Rgb8> = project.zones.iter().map(|z| z.color).collect();
        let tol = project.color_tolerance;
        zone_id = vec![255u8; (mask.width as usize) * (mask.height as usize)];
        for y in 0..mask.height {
            for x in 0..mask.width {
                if mask.alpha(x, y) < 128 {
                    continue;
                }
                let c = mask.pixel(x, y);
                if project.exclusion_colors.iter().any(|e| e.dist(&c) <= tol) {
                    continue;
                }
                let mut best: Option<(usize, u32)> = None;
                for (i, p) in palette.iter().enumerate() {
                    let d = c.dist(p);
                    if d <= tol && best.map_or(true, |(_, bd)| d < bd) {
                        best = Some((i, d));
                    }
                }
                if let Some((zi, _)) = best {
                    let idx = y * mask.width + x;
                    zone_pixels[zi].push(idx);
                    zone_id[idx as usize] = zi as u8;
                }
            }
        }
    }
    let ps = mask.map_or(1.0, |m| m.pixel_size_m(size));
    let pixel_area = ps * ps;

    let noise_seed = project.seed ^ 0x5DEECE66D;
    let use_clearings = project.clearing_strength > 0.0 && project.clearing_scale_m > 0.0;

    // poszarpanie granic (działa też dla wnętrza poligonów, niezależnie od pasa)
    let jag = project.edges.jagged_m.max(0.0);
    let wl = (project.edges.band_width_m * 3.0).max(80.0);
    let jag_seed = noise_seed ^ 0xA5A5_A5A5;
    // szum poszarpania dla zadanego zakresu — używane per-obszar (własna granica)
    let wob = make_wob(jag, wl, jag_seed);

    let env = Env {
        project,
        hm: heightmap,
        ex: exclusions,
        noise_seed,
        use_clearings,
    };

    let nz = project.zones.len();
    let na = project.areas.len();
    // kolejność deklaracji ma znaczenie: dane pożyczane przez domknięcia w jobs
    // muszą żyć dłużej (drop jest w odwrotnej kolejności)
    let mut edge_bands: Vec<(usize, Vec<(u32, u8)>)> = Vec::new();

    // --- kolejność generowania (per-obszar, konfigurowalna) --------------------
    use crate::preset::GenStep;
    // sanitize już rozwija legacy Areas -> Area(i), ale na wszelki wypadek obsłuż tutaj
    let order: Vec<GenStep> = {
        let o = if project.generation_order.is_empty() {
            vec![GenStep::Mask, GenStep::Cut]
        } else {
            project.generation_order.clone()
        };
        // rozwiń legacy Areas
        let mut expanded: Vec<GenStep> = Vec::new();
        for e in o {
            match e {
                GenStep::Areas => {
                    for i in 0..project.areas.len() { expanded.push(GenStep::Area(i)); }
                }
                other => expanded.push(other),
            }
        }
        // dedup + filtr niepoprawnych indeksów
        let mut seen_mask = false;
        let mut seen_cut = false;
        let mut seen_area: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut clean: Vec<GenStep> = Vec::new();
        for e in expanded {
            match e {
                GenStep::Mask => if !seen_mask { seen_mask=true; clean.push(e); },
                GenStep::Cut => if !seen_cut { seen_cut=true; clean.push(e); },
                GenStep::Area(i) => if i < project.areas.len() && seen_area.insert(i) { clean.push(e); },
                GenStep::Areas => {},
            }
        }
        if !seen_mask { clean.insert(0, GenStep::Mask); }
        if !seen_cut { clean.push(GenStep::Cut); }
        for i in 0..project.areas.len() {
            if !seen_area.contains(&i) {
                if let Some(pos) = clean.iter().position(|e| matches!(e, GenStep::Cut)) {
                    clean.insert(pos, GenStep::Area(i));
                } else { clean.push(GenStep::Area(i)); }
            }
        }
        clean
    };

    // mapa głębokości od krawędzi (wtapianie pasa w głąb stref z maski)
    // blend_inside_m działa niezależnie od edges.enabled
    let blend_inside = project.edges.fill_blend();
    let blend_depth: Option<HashMap<u32, u8>> = if blend_inside > 0.0 {
        mask.map(|m| {
            let max_px = ((blend_inside / ps).ceil() as usize).clamp(1, 255);
            mask_edge_depth(m.width, m.height, &zone_id, max_px)
        })
    } else {
        None
    };


    // poligony wycinające per-obszar (dla kroku Area(i) gdy cutting)
    let cutting_polys_by_idx: Vec<Option<Polygon>> = project.areas.iter().map(|a| {
        if a.enabled && a.cutting && a.polygon.len() >= 3 {
            Some(Polygon {
                rings: {
                    let mut rings = vec![a.polygon.clone()];
                    rings.extend(a.holes.iter().filter(|h| h.len() >= 3).cloned());
                    rings
                }
            })
        } else { None }
    }).collect();

    // --- budowa kolejek zadań per faza (wspólna siatka, zachowujemy d) ----------
    let mut jobs_mask: Vec<Job> = Vec::new();
    // per-obszar: jobs_per_area[i] = wszystkie zadania generujące dla tego obszaru (w tym pasy brzegowe)
    let mut jobs_per_area: Vec<Vec<Job>> = Vec::with_capacity(project.areas.len());
    for _ in 0..project.areas.len() { jobs_per_area.push(Vec::new()); }
    let mut max_min_dist = 1.0f64;
    let mut total_target = 0usize;

    // --- Źródło: strefy z maski -------------------------------------------------
    if project.use_mask_zones {
        for (zi, zp) in zone_pixels.iter().enumerate() {
            let zone = &project.zones[zi];
            let area_m2 = zp.len() as f64 * pixel_area;
            let target =
                (area_m2 / 10_000.0 * f64::from(zone.density_per_ha)).round() as usize;
            if target == 0 || zp.is_empty() {
                continue;
            }
            let d = plan_min_dist(project.spacing_multiplier, area_m2, target);
            max_min_dist = max_min_dist.max(d);
            total_target += target;
            let cand: Box<dyn FnMut(&mut StdRng) -> Option<(f64, f64)> + '_> =
                match &blend_depth {
                    Some(map) => Box::new(make_px_cand_blend(
                        zp,
                        mask.unwrap().width,
                        size,
                        ps,
                        map,
                        blend_inside,
                    )),
                    None => Box::new(make_px_cand(zp, mask.unwrap().width, size, ps)),
                };
            jobs_mask.push(Job {
                label: zone.label.clone(),
                target,
                min_dist: d,
                cum: cumulative(&zone.species_weights),
                source_index: zi,
                is_edge: false,
                cand,
            });
        }
    }
    // --- Źródło: obszary rysowane (poligony) --------------------------------------
    if project.use_areas {
        for (ai, area) in project.areas.iter().enumerate() {
            if !area.enabled || area.cutting {
                continue;
            }
            let ring = &area.polygon;
            if ring.len() < 3 {
                continue;
            }
            let area_m2 = polygon_net_area_m2(ring, &area.holes);
            if area_m2 <= 0.0 {
                continue;
            }
            let poly = Polygon {
                rings: {
                    let mut rings = vec![ring.clone()];
                    rings.extend(area.holes.iter().filter(|h| h.len() >= 3).cloned());
                    rings
                },
            };
            // indywidualna granica tego obszaru (None = globalna)
            let eff_edges = area.edges.as_ref().unwrap_or(&project.edges);
            let jag_a = eff_edges.jagged_m.max(0.0);
            let wl_a = (eff_edges.band_width_m * 3.0).max(80.0);
            let wob_a = make_wob(jag_a, wl_a, jag_seed);
            let blend_in_a = eff_edges.fill_blend();

            let (bmin_x, bmin_y, bmax_x, bmax_y) = bbox_of(ring);
            let (min_x, min_y, max_x, max_y) = (
                (bmin_x - jag_a).max(0.0),
                (bmin_y - jag_a).max(0.0),
                (bmax_x + jag_a).min(size),
                (bmax_y + jag_a).min(size),
            );
            // Obszar poza mapą: pusty zakres losowania (validate zgłasza to dla
            // obszarów skonfigurowanych, tu chronimy pozostałe, np. sam miks).
            if min_x >= max_x || min_y >= max_y {
                continue;
            }
            let bbox = (min_x, min_y, max_x, max_y);
            let src_idx = nz + ai;
            let spacing = project.spacing_multiplier;

            let mixing: Vec<&MixEntry> = if area.mix_spatial {
                area.preset_mix
                    .iter()
                    .filter(|e| e.spatial && mix_entry_ready(e))
                    .collect()
            } else {
                Vec::new()
            };
            let standard: Vec<&MixEntry> = if area.mix_spatial {
                area.preset_mix
                    .iter()
                    .filter(|e| !e.spatial && mix_entry_ready(e))
                    .collect()
            } else {
                Vec::new()
            };

            if !mixing.is_empty() {
                let shares: Vec<f32> = mixing.iter().map(|e| e.share.max(0.05)).collect();
                let share_sum: f32 = shares.iter().sum();
                let scale = area.mix_scale_m.max(20.0);
                let mix_seed =
                    noise_seed ^ 0xC0FF_EE11 ^ (ai as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                let use_perlin = mixing.len() > 1;
                for (mi, e) in mixing.iter().enumerate() {
                    let frac = if share_sum > 0.0 {
                        f64::from(shares[mi] / share_sum)
                    } else {
                        1.0 / mixing.len() as f64
                    };
                    let (cf_samples, cf_tol, cf_invert) =
                        area_color_for_entry(area.color_filter.as_ref(), Some(e));
                    let perlin = if use_perlin {
                        Some((mi, shares.clone(), scale, mix_seed))
                    } else {
                        None
                    };
                    push_area_interior_job(
                        &mut jobs_per_area[ai],
                        &mut total_target,
                        &mut max_min_dist,
                        spacing,
                        format!("{} · {}", area.label, e.name),
                        src_idx,
                        e.density_per_ha,
                        &e.species_weights,
                        area_m2,
                        frac,
                        poly.clone(),
                        ring.clone(),
                        bbox,
                        wob_a.clone(),
                        blend_in_a,
                        satellite,
                        size,
                        cf_samples,
                        cf_tol,
                        cf_invert,
                        perlin,
                    );
                }
                if let Some((dens, weights)) = blend_mix_entries(&standard) {
                    let (cf_samples, cf_tol, cf_invert) =
                        area_color_for_entry(area.color_filter.as_ref(), None);
                    push_area_interior_job(
                        &mut jobs_per_area[ai],
                        &mut total_target,
                        &mut max_min_dist,
                        spacing,
                        format!("{} · standard", area.label),
                        src_idx,
                        dens,
                        &weights,
                        area_m2,
                        1.0,
                        poly,
                        ring.clone(),
                        bbox,
                        wob_a,
                        blend_in_a,
                        satellite,
                        size,
                        cf_samples,
                        cf_tol,
                        cf_invert,
                        None,
                    );
                }
            } else {
                if area.species_weights.is_empty() || area.density_per_ha <= 0.0 {
                    continue;
                }
                let (cf_samples, cf_tol, cf_invert) =
                    area_color_for_entry(area.color_filter.as_ref(), None);
                push_area_interior_job(
                    &mut jobs_per_area[ai],
                    &mut total_target,
                    &mut max_min_dist,
                    spacing,
                    area.label.clone(),
                    src_idx,
                    area.density_per_ha,
                    &area.species_weights,
                    area_m2,
                    1.0,
                    poly,
                    ring.clone(),
                    bbox,
                    wob_a,
                    blend_in_a,
                    satellite,
                    size,
                    cf_samples,
                    cf_tol,
                    cf_invert,
                    None,
                );
            }
        }
    }

    // --- Źródło: pas graniczny ------------------------------------------------------
    // (puste wagi odrzuca validate(); guard na wypadek obejścia walidacji)
    if project.edges.enabled {
        let band = project.edges.inner_reach().max(project.edges.outside_m.unwrap_or(0.0));
        let band_px = ((band / ps).ceil() as usize).clamp(1, 128);
        let patch_mask = make_patch(project.edges.patch_size_m, project.edges.patch_cover, jag_seed);
        // miks presetów (blend) albo ręczna lista
        let (edge_dens, edge_w) = edge_effective(&project.edges);
        if edge_w.is_empty() {
            // niegotowy miks i pusta ręczna lista — nic do sadzenia
            return Err("Granica lasu: brak gatunków (ustaw wagę > 0 albo dodaj preset)".into());
        }
        let ecum = cumulative(&edge_w);
        let blend = project.edges.blend;

        // najpierw policz pasy na masce (piksel + głębokość od krawędzi)
        if let Some(m) = mask {
            if project.use_mask_zones {
                for (zi, zp) in zone_pixels.iter().enumerate() {
                    if zp.is_empty() {
                        continue;
                    }
                    let band_set =
                        mask_edge_band(m.width, m.height, &zone_id, zi as u8, zp, band_px);
                    if !band_set.is_empty() {
                        edge_bands.push((zi, band_set));
                    }
                }
            }
        }

        // pasy na masce -> K warstw gęstości (wtapianie)
        for (zi, band_set) in &edge_bands {
            let total_area = band_set.len() as f64 * pixel_area;
            let base_target =
                (total_area / 10_000.0 * f64::from(edge_dens) * project.edges.patch_share()).round() as usize;
            if base_target == 0 {
                continue;
            }
            let wsum: f64 =
                (0..EDGE_STRIPS).map(|k| strip_weight(k, EDGE_STRIPS, blend)).sum();
            let m = mask.unwrap();
            let max_s = (band_px as f64 + 1.0) * ps;

            for k in 0..EDGE_STRIPS {
                let wk = strip_weight(k, EDGE_STRIPS, blend);
                let frac = wk / wsum;
                let strip_target = if k == EDGE_STRIPS - 1 {
                    base_target.saturating_sub(
                        (base_target as f64 * (wsum - wk) / wsum).round() as usize,
                    )
                } else {
                    (base_target as f64 * frac).round() as usize
                };
                if strip_target == 0 {
                    continue;
                }
                let strip_area = total_area * frac;
                let d = plan_min_dist(project.spacing_multiplier, strip_area.max(1.0), strip_target);
                max_min_dist = max_min_dist.max(d);
                total_target += strip_target;

                let s_lo = max_s * (k as f64) / EDGE_STRIPS as f64;
                let s_hi = max_s * ((k + 1) as f64) / EDGE_STRIPS as f64;
                let set: &[(u32, u8)] = band_set;
                let width = m.width;
                    let wob = wob.clone();
                    let patch = patch_mask.clone();
                    jobs_mask.push(Job {
                        label: format!(
                            "Granica – {} ({}/{})",
                            project.zones[*zi].label,
                            k + 1,
                            EDGE_STRIPS
                        ),
                        target: strip_target,
                        min_dist: d,
                        cum: ecum.clone(),
                        source_index: nz + na,
                        is_edge: true,
                        cand: Box::new(move |rng: &mut StdRng| {
                            if set.is_empty() {
                            return None;
                        }
                        let (idx, lvl) = set[rng.gen_range(0..set.len())];
                        let px = (idx % width) as f64;
                        let row = (idx / width) as f64;
                        let wx = (px + 0.5 + rng.gen_range(-0.5..0.5)) * ps;
                        let wy = size - (row + 0.5 + rng.gen_range(-0.5..0.5)) * ps;
                        let s = (f64::from(lvl) + rng.gen_range(0.0..1.0)) * ps + wob(wx, wy);
                        if s >= s_lo && s < s_hi && patch(wx, wy) {
                            Some((wx, wy))
                        } else {
                            None
                        }
                    }),
                });
            }
        }
    }

    // pas wokół granic poligonów (obie strony pierścienia), też w warstwach
    // — każdy obszar może mieć WŁASNĄ granicę (nadpisuje globalną),
    //   niezależnie od globalnego przełącznika
    if project.use_areas {
            for (ai, area) in project.areas.iter().enumerate() {
                if !area.enabled || area.cutting {
                    continue;
                }
                let eff = area.edges.as_ref().unwrap_or(&project.edges);
                if !eff.enabled {
                    continue;
                }
                let ring = &area.polygon;
                if ring.len() < 3 {
                    continue;
                }
                let band_a = eff.inner_reach();
                let outside_a = eff.outside_m;
                let jag_a = eff.jagged_m.max(0.0);
                let wob_a = make_wob(jag_a, (band_a * 3.0).max(80.0), jag_seed);
                let patch_a = make_patch(eff.patch_size_m, eff.patch_cover, jag_seed);
                // miks presetów zamiast ręcznej listy (blend udziałów)
                let (eff_dens, eff_w) = edge_effective(eff);
                if eff_w.is_empty() {
                    continue; // validate() to wyłapuje; brak paniki przy obejściu
                }
                let ecum_a = cumulative(&eff_w);

                let perim = polygon_perimeter(ring);
                let inner = polygon_area(ring);
                let area_m2 = (perim * (band_a + outside_a.unwrap_or(band_a))).min(inner * 1.5);
                if area_m2 <= 0.0 {
                    continue;
                }
                let base_target =
                    (area_m2 / 10_000.0 * f64::from(eff_dens) * eff.patch_share()).round() as usize;
                if base_target == 0 {
                    continue;
                }
                let wsum: f64 = (0..EDGE_STRIPS)
                    .map(|k| strip_weight(k, EDGE_STRIPS, eff.blend))
                    .sum();
                let (min_x, min_y, max_x, max_y) = bbox_of(ring);
                let margin = band_a.max(outside_a.unwrap_or(0.0)) + jag_a;
                let (min_x, min_y, max_x, max_y) = (
                    (min_x - margin).max(0.0),
                    (min_y - margin).max(0.0),
                    (max_x + margin).min(size),
                    (max_y + margin).min(size),
                );
                if min_x >= max_x || min_y >= max_y {
                    continue;
                }

                // inteligentne generowanie: filtr kolorów podkładu (także dla granicy)
                let cf_invert: bool = area
                    .color_filter
                    .as_ref()
                    .map(|f| f.invert)
                    .unwrap_or(false);
                let cf_samples: Vec<Rgb8> = area
                    .color_filter
                    .as_ref()
                    .map(|f| f.samples.clone())
                    .unwrap_or_default();
                let cf_tol = area
                    .color_filter
                    .as_ref()
                    .map(|f| f.tolerance)
                    .unwrap_or(0);
                let use_cf = !cf_samples.is_empty() && satellite.is_some();
                let sat_ref = satellite;

                for k in 0..EDGE_STRIPS {
                    let wk = strip_weight(k, EDGE_STRIPS, eff.blend);
                    let frac = wk / wsum;
                    let strip_target = if k == EDGE_STRIPS - 1 {
                        base_target.saturating_sub(
                            (base_target as f64 * (wsum - wk) / wsum).round() as usize,
                        )
                    } else {
                        (base_target as f64 * frac).round() as usize
                    };
                    if strip_target == 0 {
                        continue;
                    }
                    let strip_area = area_m2 * frac;
                    let d =
                        plan_min_dist(project.spacing_multiplier, strip_area.max(1.0), strip_target);
                    max_min_dist = max_min_dist.max(d);
                    total_target += strip_target;

                    let s_lo = band_a * (k as f64) / EDGE_STRIPS as f64;
                    let s_hi = band_a * ((k + 1) as f64) / EDGE_STRIPS as f64;
                    let ring_c = ring.clone();
                    // Zazębianie z wnętrzem: pas mierzony od POSZARPANEJ linii
                    // (odległość znaku: |signed - wob|), a nie od sztywnego
                    // obrysu. Dzięki temu tam, gdzie poszarpanie wycina wnękę
                    // w paśmie granicznym, wnętrze lasu tę lukę wypełnia
                    // (wspólna, przesunięta linia brzegowa) — bez łysych plam.
                    // Przy jagged=0 wzór sprowadza się do starego (s = dist).
                    let poly_c = Polygon {
                        rings: {
                            let mut rings = vec![ring.clone()];
                            rings.extend(
                                area.holes.iter().filter(|h| h.len() >= 3).cloned(),
                            );
                            rings
                        },
                    };
                    let wob_a = wob_a.clone();
                    let patch_a = patch_a.clone();
                    let cf_samples = cf_samples.clone();
                    jobs_per_area[ai].push(Job {
                        label: format!(
                            "Granica – {} ({}/{})",
                            area.label,
                            k + 1,
                            EDGE_STRIPS
                        ),
                        target: strip_target,
                        min_dist: d,
                        cum: ecum_a.clone(),
                        source_index: nz + na,
                        is_edge: true,
                        cand: Box::new(move |rng: &mut StdRng| {
                            for _ in 0..24 {
                                let x = rng.gen_range(min_x..max_x);
                                let y = rng.gen_range(min_y..max_y);
                                let dist = point_ring_distance(x, y, &ring_c);
                                let w = wob_a(x, y);
                                let signed =
                                    if point_in_polygon(&poly_c, x, y) { dist } else { -dist };
                                // Na zewnątrz pas ma własną szerokość, sprowadzaną do skali warstw.
                                let s = match outside_a {
                                    Some(o) if signed < w => {
                                        if o <= 0.0 {
                                            continue;
                                        }
                                        (signed - w).abs() * band_a / o
                                    }
                                    _ => (signed - w).abs(),
                                };
                                if s >= s_lo && s < s_hi && patch_a(x, y) {
                                    // inteligentne generowanie: piksel podkładu musi
                                    // pasować do próbek (albo ICH UNIKAĆ przy invert)
                                    if use_cf {
                                        let sat = sat_ref.unwrap();
                                        let fx = (x / size) * f64::from(sat.width);
                                        let fy = ((size - y) / size) * f64::from(sat.height);
                                        let px_i = (fx.floor().max(0.0) as u32).min(sat.width - 1);
                                        let py_i = (fy.floor().max(0.0) as u32).min(sat.height - 1);
                                        let pc = sat.pixel(px_i, py_i);
                                        let matched = cf_samples.iter().any(|sm| {
                                            (i32::from(pc.0[0]) - i32::from(sm.0[0])).unsigned_abs()
                                                as u32
                                                + (i32::from(pc.0[1]) - i32::from(sm.0[1]))
                                                    .unsigned_abs()
                                                    as u32
                                                + (i32::from(pc.0[2]) - i32::from(sm.0[2]))
                                                    .unsigned_abs()
                                                    as u32
                                                <= cf_tol
                                        });
                                        let ok = if cf_invert { !matched } else { matched };
                                        if !ok {
                                            continue;
                                        }
                                    }
                                    return Some((x, y));
                                }
                            }
                            None
                        }),
                    });
                }
            }
    }

    if total_target == 0 {
        return Err(
            "Brak obszarów do zasiedlenia (maska nie zawiera pikseli stref / brak poligonów)"
                .into(),
        );
    }
    if total_target > MAX_TOTAL_OBJECTS {
        return Err(format!(
            "Docelowa liczba obiektów ({total_target}) przekracza limit {MAX_TOTAL_OBJECTS} — zmniejsz gęstość"
        ));
    }

    // --- Wykonanie w kolejności określonej przez generation_order ---------------
    let cell = max_min_dist.max(1.0);
    let mut grid: HashMap<(i64, i64), Vec<[f32; 2]>> = HashMap::new();
    let mut objects: Vec<PlacedObject> = Vec::with_capacity(total_target.min(1 << 20));
    let mut stats = GenStats {
        per_species: project.species.iter().map(|s| (s.label.clone(), 0)).collect(),
        ..Default::default()
    };

    let n_jobs_total: usize = jobs_mask.len() + jobs_per_area.iter().map(|v| v.len()).sum::<usize>();
    let span = 1.0 / n_jobs_total.max(1) as f64;
    let rng = &mut StdRng::seed_from_u64(project.seed);
    let mut trace = JobTrace::default();

    // helper: uruchom zestaw jobs sekwencyjnie
    let mut next_ji: usize = 0;
    let mut run_jobs = |jobs: &mut Vec<Job>, grid: &mut HashMap<(i64,i64), Vec<[f32;2]>>, objects: &mut Vec<PlacedObject>, stats: &mut GenStats, trace: &mut JobTrace, next_ji: &mut usize| {
        for job in jobs.iter_mut() {
            let prev_total = stats.total;
            let accepted = run_dart(
                &env,
                rng,
                grid,
                cell,
                objects,
                stats,
                job.target,
                job.min_dist,
                &job.cum,
                job.source_index,
                &mut job.cand,
                progress,
                *next_ji as f64 * span,
                span,
            );
            if job.is_edge {
                stats.edge_count += stats.total - prev_total;
            }
            stats.per_source.push((job.label.clone(), accepted));
            trace.record_job(accepted, job.is_edge);
            *next_ji += 1;
        }
    };

    // --- helpers wycinania ---
    let apply_color_cut = |objects: &mut Vec<PlacedObject>, stats: &mut GenStats, trace: &mut JobTrace| -> Result<(), String> {
        if project.cut_zones.is_empty() { return Ok(()); }
        let Some(m) = mask else {
            return Err("Projekt ma strefy wycinania po kolorze, ale nie wczytano maski — wycinanie wymaga maski do oceny kolorów.".into());
        };
        let mut cut_set: std::collections::HashSet<u32> = std::collections::HashSet::new();
        let margins: Vec<usize> = project.cut_zones.iter().map(|cz| {
            let mp = (f64::from(cz.margin_m) / ps).ceil() as usize; mp.min(4096)
        }).collect();
        let max_mp = margins.iter().copied().max().unwrap_or(0);
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for o in objects.iter() {
            let fx = o.x / ps;
            let fy_top = (size - o.y) / ps;
            if fx < 0.0 || fy_top < 0.0 || fx >= f64::from(m.width) || fy_top >= f64::from(m.height) { continue; }
            let xi = fx as u32; let yi = fy_top as u32;
            x0 = x0.min(xi); y0 = y0.min(yi); x1 = x1.max(xi); y1 = y1.max(yi);
        }
        let pad = max_mp as u32;
        let sx0 = x0.saturating_sub(pad);
        let sy0 = y0.saturating_sub(pad);
        let sx1 = (x1 + pad).min(m.width - 1);
        let sy1 = (y1 + pad).min(m.height - 1);
        let tol = project.color_tolerance as i32;
        let n_z = project.cut_zones.len();
        let mut seeds: Vec<Vec<u32>> = vec![Vec::new(); n_z];
        if sx0 <= sx1 && sy0 <= sy1 && !objects.is_empty() {
            let rgba = &m.rgba; let w = m.width as usize;
            for y in sy0..=sy1 {
                let row = (y as usize) * w;
                for x in sx0..=sx1 {
                    let off = (row + x as usize) * 4;
                    if rgba[off+3] < 128 { continue; }
                    let r=i32::from(rgba[off]); let g=i32::from(rgba[off+1]); let b=i32::from(rgba[off+2]);
                    for (zi, cz) in project.cut_zones.iter().enumerate() {
                        let [cr,cg,cb]=cz.color.0;
                        if (r-i32::from(cr)).abs() > tol { continue; }
                        if (g-i32::from(cg)).abs() > tol { continue; }
                        if (b-i32::from(cb)).abs() > tol { continue; }
                        seeds[zi].push(y*m.width+x);
                    }
                }
            }
        }
        for (zi, seed_vec) in seeds.into_iter().enumerate() {
            let mp = margins[zi];
            if mp==0 { cut_set.extend(seed_vec); continue; }
            let mut local: std::collections::HashSet<u32> = std::collections::HashSet::with_capacity(seed_vec.len().saturating_mul(2));
            let mut frontier: Vec<u32>=Vec::with_capacity(seed_vec.len());
            for s in seed_vec { if local.insert(s) { frontier.push(s); } }
            for _level in 0..mp {
                if frontier.is_empty() { break; }
                let mut next=Vec::new();
                for &idx in &frontier {
                    let x=idx % m.width; let y=idx / m.width; let ci=idx as usize;
                    let mut neigh:[Option<usize>;4]=[None;4];
                    if x>0 { neigh[0]=Some(ci-1); }
                    if x+1 < m.width { neigh[1]=Some(ci+1); }
                    if y>0 { neigh[2]=Some(ci-m.width as usize); }
                    if y+1 < m.height { neigh[3]=Some(ci+m.width as usize); }
                    for n in neigh.into_iter().flatten() { if local.insert(n as u32) { next.push(n as u32); } }
                }
                frontier=next;
            }
            cut_set.extend(local);
        }
        trace.retain_objects(objects, stats, |o| {
            let fx=o.x / ps; let fy_top=(size - o.y)/ps;
            if fx < 0.0 || fy_top < 0.0 || fx >= f64::from(m.width) || fy_top >= f64::from(m.height) { return true; }
            let idx=(fy_top as u32)*m.width + (fx as u32);
            !cut_set.contains(&idx)
        });
        Ok(())
    };

    for step in &order {
        match step {
            GenStep::Mask => {
                if project.use_mask_zones {
                    run_jobs(&mut jobs_mask, &mut grid, &mut objects, &mut stats, &mut trace, &mut next_ji);
                }
            }
            GenStep::Cut => {
                apply_color_cut(&mut objects, &mut stats, &mut trace)?;
                // odbuduj siatkę odstępów – wycięte punkty nie mogą blokować dalszego generowania
                grid.clear();
                for o in &objects {
                    let p = [o.x as f32, o.y as f32];
                    grid.entry(world_to_cell(p, cell)).or_default().push(p);
                }
                progress(next_ji as f64 * span);
            }
            GenStep::Area(idx) => {
                let i = *idx;
                if i >= project.areas.len() { continue; }
                let area = &project.areas[i];
                if !area.enabled || !project.use_areas { continue; }
                if area.cutting {
                    if let Some(poly) = &cutting_polys_by_idx[i] {
                        trace.retain_objects(&mut objects, &mut stats, |o| {
                            !point_in_polygon(poly, o.x, o.y)
                        });
                        grid.clear();
                        for o in &objects {
                            let p = [o.x as f32, o.y as f32];
                            grid.entry(world_to_cell(p, cell)).or_default().push(p);
                        }
                        progress(next_ji as f64 * span);
                    }
                } else {
                    run_jobs(&mut jobs_per_area[i], &mut grid, &mut objects, &mut stats, &mut trace, &mut next_ji);
                }
            }
            GenStep::Areas => {
                // legacy fallback — wygeneruj wszystkie obszary po kolei
                if project.use_areas {
                    for v in &mut jobs_per_area {
                        run_jobs(v, &mut grid, &mut objects, &mut stats, &mut trace, &mut next_ji);
                    }
                }
            }
        }
    }

    progress(1.0);
    stats.elapsed_ms = t0.elapsed().as_millis();
    Ok((objects, stats))
}

// --- Siatka przestrzenna -------------------------------------------------------

#[inline]
fn world_to_cell(p: [f32; 2], cell: f64) -> (i64, i64) {
    (
        (p[0] as f64 / cell).floor() as i64,
        (p[1] as f64 / cell).floor() as i64,
    )
}

fn grid_has_neighbor(
    grid: &HashMap<(i64, i64), Vec<[f32; 2]>>,
    cell: f64,
    p: [f32; 2],
    radius: f64,
) -> bool {
    if radius <= 0.0 || grid.is_empty() {
        return false;
    }
    let r2 = (radius * radius) as f32;
    let span = (radius / cell).ceil() as i64;
    let (cx, cy) = world_to_cell(p, cell);
    for dy in -span..=span {
        for dx in -span..=span {
            if let Some(bucket) = grid.get(&(cx + dx, cy + dy)) {
                for q in bucket {
                    let ddx = q[0] - p[0];
                    let ddy = q[1] - p[1];
                    if ddx * ddx + ddy * ddy < r2 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

// --- Szum wartościowy (polany) ---------------------------------------------

#[inline]
pub fn hash2(ix: f64, iy: f64, seed: u64) -> f64 {
    let mut h = ix
        .to_bits()
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ iy.to_bits().wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ seed;
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

#[inline]
fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn value_noise(x: f64, y: f64, seed: u64) -> f64 {
    let ix = x.floor();
    let iy = y.floor();
    let tx = smoothstep(x - ix);
    let ty = smoothstep(y - iy);
    let v00 = hash2(ix, iy, seed);
    let v10 = hash2(ix + 1.0, iy, seed);
    let v01 = hash2(ix, iy + 1.0, seed);
    let v11 = hash2(ix + 1.0, iy + 1.0, seed);
    let a = v00 + (v10 - v00) * tx;
    let b = v01 + (v11 - v01) * tx;
    a + (b - a) * ty
}

/// 3-oktawowy fbm, wynik ~0..1.
fn fbm2(x: f64, y: f64, seed: u64) -> f64 {
    (value_noise(x, y, seed) * 0.5
        + value_noise(x * 2.03, y * 2.03, seed ^ 0x1234) * 0.25
        + value_noise(x * 4.07, y * 4.07, seed ^ 0xABCD) * 0.125)
        / 0.875
}

/// Próg fbm tak, aby odrzucona część obszaru ? `strength`.
/// LUT odpowiada przybliżonej dystrybuancie fbm (mu~0.5, sigma~0.12).
fn clearing_threshold(strength: f64) -> f64 {
    const LUT: [(f64, f64); 11] = [
        (0.00, 0.00),
        (0.10, 0.346),
        (0.20, 0.399),
        (0.30, 0.437),
        (0.40, 0.470),
        (0.50, 0.500),
        (0.60, 0.530),
        (0.70, 0.563),
        (0.80, 0.601),
        (0.90, 0.654),
        (0.95, 0.697),
    ];
    let s = strength.clamp(0.0, 0.95);
    for w in LUT.windows(2) {
        if s <= w[1].0 {
            let t = (s - w[0].0) / (w[1].0 - w[0].0);
            return w[0].1 + t * (w[1].1 - w[0].1);
        }
    }
    0.697
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::{AreaDef, EdgeSettings, GenStep, ZoneDef};
    use crate::species::SpeciesDef;

    fn two_species() -> Vec<SpeciesDef> {
        vec![
            SpeciesDef::new("A", "a_1f", 0.9, 1.1),
            SpeciesDef::new("B", "b_1f", 0.9, 1.1),
        ]
    }

    fn zone_project(colors: &[Rgb8]) -> ForestProject {
        let mut p = ForestProject {
            map_size_m: 1000.0,
            clearing_scale_m: 0.0,
            edge_padding_m: 0.0,
            ..Default::default()
        };
        p.species = two_species();
        p.zones = colors
            .iter()
            .enumerate()
            .map(|(i, c)| ZoneDef {
                color: *c,
                label: format!("Z{i}"),
                density_per_ha: 300.0,
                species_weights: vec![(0, 3.0), (1, 1.0)],
                preset_mix: Vec::new(),
            })
            .collect();
        p
    }

    fn mask_with_rect(w: u32, h: u32, rect: (u32, u32, u32, u32), c: Rgb8) -> MaskImage {
        let mut img = image::RgbaImage::new(w, h);
        let (x0, y0, rw, rh) = rect;
        if rw > 0 && rh > 0 {
            for y in y0..y0 + rh {
                for x in x0..x0 + rw {
                    img.put_pixel(x, y, image::Rgba([c.r(), c.g(), c.b(), 255]));
                }
            }
        }
        MaskImage::from_dynamic(image::DynamicImage::ImageRgba8(img))
    }

    #[test]
    fn generates_respecting_min_distance_and_proportions() {
        let green = Rgb8([0, 200, 0]);
        let proj = zone_project(&[green]);
        let mask = mask_with_rect(100, 100, (10, 10, 80, 80), green);
        // mapa 1000m, prostokat 800x800m = 64 ha * 300/ha = 19200 celów
        let (objs, stats) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert!(stats.total >= 18_000, "total={}", stats.total);
        assert_eq!(objs.len(), stats.total);

        let mut min_d = f64::INFINITY;
        for (i, a) in objs.iter().enumerate() {
            for b in objs.iter().skip(i + 1).take(400) {
                min_d = min_d.min((a.x - b.x).hypot(a.y - b.y));
            }
        }
        let area: f64 = 640_000.0;
        let target: f64 = 64.0 * 300.0;
        let d_plan: f64 = proj.spacing_multiplier * (0.7 * area / target).sqrt();
        // dopuszczamy <=2 przebiegi zagęszczania (0.85^2)
        assert!(min_d >= d_plan * 0.70, "min_d={min_d}, d_plan={d_plan}");

        // proporcje 3:1 z tolerancją ±25%
        let na = stats.per_species[0].1 as f64;
        let nb = stats.per_species[1].1 as f64;
        let ratio = na / nb;
        assert!(ratio > 2.25 && ratio < 3.75, "ratio={ratio} (na={na}, nb={nb})");
    }

    #[test]
    fn neutral_scale_keeps_species_range_and_determinism() {
        let green = Rgb8([0, 200, 0]);
        let proj = zone_project(&[green]); // scale_min/max = 1.0 (neutralne)
        let mask = mask_with_rect(60, 60, (5, 5, 50, 50), green);
        let (o1, _) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert!(!o1.is_empty());
        for o in &o1 {
            assert!(
                (0.9..=1.1).contains(&o.scale),
                "skala poza zakresem gatunku: {}",
                o.scale
            );
        }
        // bit-identyczne skale przy powtórce (RNG nietknięte przez mnożnik)
        let (o2, _) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        for (a, b) in o1.iter().zip(&o2) {
            assert!((a.scale - b.scale).abs() < 1e-12);
        }
    }

    #[test]
    fn global_scale_range_multiplies_species_scale() {
        let green = Rgb8([0, 200, 0]);
        let mut proj = zone_project(&[green]); // gatunki 0.9..1.1
        proj.scale_min = 0.5;
        proj.scale_max = 2.0;
        let mask = mask_with_rect(60, 60, (5, 5, 50, 50), green);
        let (objs, _) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert!(!objs.is_empty());
        for o in &objs {
            assert!(
                (0.9 * 0.5..=1.1 * 2.0).contains(&o.scale),
                "skala poza zakresem globalnym: {}",
                o.scale
            );
        }
        // rozrzut faktycznie szerszy niż sam gatunek (0.9..1.1)
        let outside = objs
            .iter()
            .filter(|o| o.scale < 0.9 || o.scale > 1.1)
            .count();
        assert!(outside > 0, "globalny zakres nie rozszerzył skal");
    }

    #[test]
    fn fixed_global_scale_doubles_every_object() {
        let green = Rgb8([0, 200, 0]);
        let base = zone_project(&[green]);
        let mask = mask_with_rect(60, 60, (5, 5, 50, 50), green);
        let (o_base, _) =
            generate(&base, Some(&mask), None, None, None, &|_| {}).unwrap();
        let mut proj = zone_project(&[green]);
        proj.scale_min = 2.0;
        proj.scale_max = 2.0;
        let (o2, _) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert_eq!(o_base.len(), o2.len());
        for (a, b) in o_base.iter().zip(&o2) {
            assert!((b.scale - 2.0 * a.scale).abs() < 1e-9, "{} vs {}", b.scale, a.scale);
            // pozycje identyczne (mnożnik stały nie rusza RNG)
            assert!((a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9);
        }
    }

    #[test]
    fn deterministic_with_same_seed() {
        let red = Rgb8([255, 0, 0]);
        let proj = zone_project(&[red]);
        let mask = mask_with_rect(60, 60, (5, 5, 50, 50), red);
        let (o1, _) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        let (o2, _) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert_eq!(o1.len(), o2.len());
        for (a, b) in o1.iter().zip(&o2) {
            assert_eq!(a.model, b.model);
            assert!((a.x - b.x).abs() < 1e-9);
            assert!((a.y - b.y).abs() < 1e-9);
        }
    }

    #[test]
    fn exclusion_colors_and_geojson_block_placement() {
        let green = Rgb8([0, 200, 0]);
        let black = Rgb8([0, 0, 0]);
        let mut proj = zone_project(&[green]);
        proj.exclusion_colors = vec![black];

        let mut img = image::RgbaImage::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                img.put_pixel(x, y, image::Rgba([0, 200, 0, 255]));
            }
        }
        for x in 40..60 {
            for y in 0..100 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let mask = MaskImage::from_dynamic(image::DynamicImage::ImageRgba8(img));
        let (_, stats) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();

        let (objs, _) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        for o in &objs {
            assert!(o.x < 400.0 || o.x >= 600.0, "obiekt w wykluczeniu x={}", o.x);
        }
        assert!(stats.total > 0);
    }

    #[test]
    fn geojson_exclusion_blocks_area() {
        let green = Rgb8([0, 200, 0]);
        let proj = zone_project(&[green]);
        let mask = mask_with_rect(100, 100, (0, 0, 100, 100), green);
        let gj = GeoJsonData::parse(
            r#"{"type":"Polygon","coordinates":[[[400,400],[600,400],[600,600],[400,600],[400,400]]]}"#,
        )
        .unwrap();
        let (objs, _) = generate(&proj, Some(&mask), None, None, Some(&gj), &|_| {}).unwrap();
        for o in &objs {
            assert!(o.x < 400.0 || o.x > 600.0 || o.y < 400.0 || o.y > 600.0);
        }
        assert!(!objs.is_empty());
    }

    #[test]
    fn altitude_filter_works() {
        let green = Rgb8([0, 200, 0]);
        let mut proj = zone_project(&[green]);
        proj.min_altitude = Some(55.0);
        let mask = mask_with_rect(30, 30, (0, 0, 30, 30), green);

        let asc_text = "ncols 3\nnrows 3\nxllcorner 0\nyllcorner 0\ncellsize 333.34\nNODATA_value -9999\n\
             100 110 120\n50 60 70\n0 10 20\n";
        let hm = AscHeightmap::parse(asc_text).unwrap();
        let (objs, stats) =
            generate(&proj, Some(&mask), None, Some(&hm), None, &|_| {}).unwrap();
        assert!(stats.rejected_filters > 0);
        for o in &objs {
            let h = hm.sample(o.x, o.y).unwrap();
            assert!(h >= 55.0);
        }
        assert!(!objs.is_empty());
    }

    #[test]
    fn absolute_elevation_written_when_requested() {
        let green = Rgb8([0, 200, 0]);
        let mut proj = zone_project(&[green]);
        proj.elevation_mode = ElevationMode::AbsoluteSampled;
        let mask = mask_with_rect(30, 30, (0, 0, 30, 30), green);
        let asc_text = "ncols 3\nnrows 3\nxllcorner 0\nyllcorner 0\ncellsize 333.34\nNODATA_value -9999\n\
             100 110 120\n50 60 70\n0 10 20\n";
        let hm = AscHeightmap::parse(asc_text).unwrap();
        let (objs, _) = generate(&proj, Some(&mask), None, Some(&hm), None, &|_| {}).unwrap();
        assert!(objs.iter().any(|o| o.elevation > 0.0));
    }

    #[test]
    fn clearings_shape_distribution() {
        let green = Rgb8([0, 200, 0]);
        let mut p = zone_project(&[green]);
        p.clearing_scale_m = 120.0;
        p.clearing_strength = 0.5;

        let mask = mask_with_rect(120, 120, (0, 0, 120, 120), green);
        let (objs, stats) = generate(&p, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert!(stats.rejected_clearing > 0);
        assert!(stats.total >= 29_000, "total={}", stats.total);

        let noise_seed = p.seed ^ 0x5DEECE66D;
        let mean_at_points: f64 = objs
            .iter()
            .map(|o| fbm2(o.x / 120.0, o.y / 120.0, noise_seed))
            .sum::<f64>()
            / objs.len() as f64;
        let mut ref_sum = 0.0;
        let mut n = 0usize;
        for y in 0..120u32 {
            for x in 0..120u32 {
                ref_sum += fbm2(f64::from(x) / 12.0, f64::from(y) / 12.0, noise_seed);
                n += 1;
            }
        }
        let mean_ref = ref_sum / n as f64;
        assert!(
            mean_at_points > mean_ref + 0.02,
            "punkty={mean_at_points} referencja={mean_ref}"
        );
    }

    #[test]
    fn empty_mask_and_no_areas_is_error() {
        let green = Rgb8([0, 200, 0]);
        let proj = zone_project(&[green]);
        let mask = mask_with_rect(20, 20, (0, 0, 0, 0), green);
        assert!(generate(&proj, Some(&mask), None, None, None, &|_| {}).is_err());
    }

    #[test]
    fn zones_require_mask_when_present_in_project() {
        let green = Rgb8([0, 200, 0]);
        let proj = zone_project(&[green]);
        assert!(generate(&proj, None, None, None, None, &|_| {}).is_err());
    }

    // --- regresje: awarie i błędne wyniki -----------------------------------

    fn sharp_project() -> ForestProject {
        let mut p = zone_project(&[]);
        p.edges.jagged_m = 0.0;
        p.edges.blend_inside_m = 0.0;
        p.edges.species_weights = vec![(0, 1.0), (1, 1.0)];
        p
    }

    fn sharp_area(label: &str, ring: Vec<[f64; 2]>, density: f32) -> AreaDef {
        AreaDef {
            label: label.into(),
            density_per_ha: density,
            species_weights: vec![(0, 1.0), (1, 1.0)],
            polygon: ring,
            ..Default::default()
        }
    }

    fn cutting_area(ring: Vec<[f64; 2]>) -> AreaDef {
        AreaDef {
            label: "cut".into(),
            polygon: ring,
            cutting: true,
            ..Default::default()
        }
    }

    fn assert_same(a: &[PlacedObject], b: &[PlacedObject]) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert_eq!((x.x, x.y, x.species_index), (y.x, y.y, y.species_index));
        }
    }

    #[test]
    fn area_outside_map_is_reported_not_panic() {
        let mut p = sharp_project();
        p.areas.push(sharp_area("Daleko", square_ring(2000.0, 2000.0, 2100.0, 2100.0), 100.0));
        let err = generate(&p, None, None, None, None, &|_| {}).unwrap_err();
        assert!(err.contains("poza mapą"), "{err}");
    }

    #[test]
    fn unconfigured_mix_area_outside_map_is_skipped() {
        let mut p = sharp_project();
        p.edges.enabled = true;
        p.areas.push(sharp_area("Las", square_ring(100.0, 100.0, 300.0, 300.0), 100.0));
        p.areas.push(AreaDef {
            label: "Sam miks".into(),
            polygon: square_ring(2000.0, 2000.0, 2100.0, 2100.0),
            mix_spatial: true,
            preset_mix: vec![MixEntry {
                name: "m".into(),
                density_per_ha: 100.0,
                species_weights: vec![(0, 1.0)],
                ..Default::default()
            }],
            ..Default::default()
        });
        let (objs, _) = generate(&p, None, None, None, None, &|_| {}).unwrap();
        assert!(!objs.is_empty());
        assert!(objs.iter().all(|o| o.x < 1000.0 && o.y < 1000.0));
    }

    #[test]
    fn zero_tilt_places_upright_objects() {
        let mut p = sharp_project();
        p.species[0].tilt_max_deg = 0.0;
        p.species[1].tilt_max_deg = 0.0;
        p.areas.push(sharp_area("Las", square_ring(100.0, 100.0, 300.0, 300.0), 100.0));
        let (objs, _) = generate(&p, None, None, None, None, &|_| {}).unwrap();
        assert!(!objs.is_empty());
        assert!(objs.iter().all(|o| o.pitch == 0.0 && o.roll == 0.0));
    }

    #[test]
    fn cut_before_later_sources_keeps_stats_valid() {
        // regresja: indeks spoza per_source, gdy wycinanie szło przed maską
        let green = Rgb8([0, 200, 0]);
        let mut p = sharp_project();
        p.zones = zone_project(&[green]).zones;
        p.areas.push(sharp_area("Las", square_ring(100.0, 100.0, 400.0, 400.0), 100.0));
        p.areas.push(cutting_area(square_ring(200.0, 200.0, 300.0, 300.0)));
        p.generation_order = vec![GenStep::Area(0), GenStep::Area(1), GenStep::Mask, GenStep::Cut];
        let mask = mask_with_rect(100, 100, (0, 0, 100, 100), green);
        let (objs, stats) = generate(&p, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert_eq!(stats.per_source.iter().map(|(_, c)| c).sum::<usize>(), objs.len());
        assert!(stats.removed_cut > 0);
    }

    #[test]
    fn stats_after_cut_keep_labels_with_reordered_areas() {
        let mut p = sharp_project();
        p.areas.push(sharp_area("A", square_ring(100.0, 100.0, 300.0, 300.0), 100.0));
        p.areas.push(sharp_area("B", square_ring(500.0, 500.0, 700.0, 700.0), 300.0));
        p.areas.push(cutting_area(square_ring(900.0, 900.0, 950.0, 950.0)));
        p.generation_order = vec![GenStep::Area(1), GenStep::Area(0), GenStep::Area(2)];
        let (_, stats) = generate(&p, None, None, None, None, &|_| {}).unwrap();
        assert_eq!(stats.per_source, vec![("B".to_string(), 1200), ("A".to_string(), 400)]);
    }

    #[test]
    fn cut_updates_edge_count() {
        let green = Rgb8([0, 200, 0]);
        let mut p = sharp_project();
        p.zones = zone_project(&[green]).zones;
        p.edges.enabled = true;
        p.areas.push(cutting_area(square_ring(0.0, 0.0, 1000.0, 1000.0)));
        p.generation_order = vec![GenStep::Mask, GenStep::Area(0), GenStep::Cut];
        let mask = mask_with_rect(100, 100, (10, 10, 80, 80), green);
        let (objs, stats) = generate(&p, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert!(objs.is_empty());
        assert_eq!((stats.total, stats.edge_count), (0, 0));
    }

    #[test]
    fn mask_edge_band_is_deterministic() {
        // regresja: kolejność HashMap zmieniała pozycje obiektów pasa
        let green = Rgb8([0, 200, 0]);
        let mut p = sharp_project();
        p.zones = zone_project(&[green]).zones;
        p.edges.enabled = true;
        let mask = mask_with_rect(100, 100, (10, 10, 80, 80), green);
        let (first, _) = generate(&p, Some(&mask), None, None, None, &|_| {}).unwrap();
        for _ in 0..4 {
            let (next, _) = generate(&p, Some(&mask), None, None, None, &|_| {}).unwrap();
            assert_same(&first, &next);
        }
    }

    #[test]
    fn edge_preset_mix_is_deterministic() {
        // regresja: kolejność wag z HashMap zmieniała wylosowane gatunki
        let mut p = sharp_project();
        p.areas.push(sharp_area("Las", square_ring(100.0, 100.0, 800.0, 800.0), 50.0));
        p.edges.enabled = true;
        p.edges.species_weights.clear();
        p.edges.preset_mix = (0..2)
            .map(|si| MixEntry {
                name: format!("m{si}"),
                density_per_ha: 200.0,
                species_weights: vec![(si, 1.0)],
                ..Default::default()
            })
            .collect();
        let (first, _) = generate(&p, None, None, None, None, &|_| {}).unwrap();
        for _ in 0..12 {
            let (next, _) = generate(&p, None, None, None, None, &|_| {}).unwrap();
            assert_same(&first, &next);
        }
    }

    #[test]
    fn holes_reduce_density_target() {
        let hole = square_ring(200.0, 200.0, 400.0, 400.0);
        let mut area = sharp_area("Las", square_ring(100.0, 100.0, 500.0, 500.0), 100.0);
        area.holes.push(hole.clone());
        let mut p = sharp_project();
        p.areas.push(area);
        let (objs, stats) = generate(&p, None, None, None, None, &|_| {}).unwrap();
        // (16 ha - 4 ha) * 100/ha
        assert_eq!(stats.total, 1200);
        let hole_poly = Polygon { rings: vec![hole] };
        assert!(objs.iter().all(|o| !point_in_polygon(&hole_poly, o.x, o.y)));
    }

    #[test]
    fn non_square_mask_is_rejected_when_used() {
        let green = Rgb8([0, 200, 0]);
        let proj = zone_project(&[green]);
        let mask = mask_with_rect(100, 50, (0, 0, 100, 50), green);
        let err = generate(&proj, Some(&mask), None, None, None, &|_| {}).err().unwrap();
        assert!(err.contains("100x50"), "{err}");
    }

    #[test]
    fn height_dependent_settings_require_heightmap() {
        let mut p = sharp_project();
        p.areas.push(sharp_area("Las", square_ring(100.0, 100.0, 300.0, 300.0), 100.0));
        p.elevation_mode = ElevationMode::AbsoluteSampled;
        assert!(generate(&p, None, None, None, None, &|_| {}).is_err());
        p.elevation_mode = ElevationMode::RelativeZero;
        p.max_altitude = Some(100.0);
        assert!(generate(&p, None, None, None, None, &|_| {}).is_err());
        p.max_altitude = None;
        p.max_slope_deg = Some(35.0); // domyślne ustawienie nie może blokować
        assert!(generate(&p, None, None, None, None, &|_| {}).is_ok());
    }

    // --- obszary (poligony) ------------------------------------------------

    fn square_ring(a: f64, b: f64, c: f64, d: f64) -> Vec<[f64; 2]> {
        vec![[a, b], [c, b], [c, d], [a, d]]
    }

    #[test]
    fn polygon_only_project_generates_inside_polygon() {
        let mut proj = zone_project(&[]);
        proj.zones.clear();
        proj.edges.jagged_m = 0.0; // ścisłe zawieranie dla tego testu
        proj.areas.push(AreaDef {
            enabled: true,
            label: "Dwór".into(),
            density_per_ha: 200.0,
            species_weights: vec![(0, 1.0)],
            polygon: square_ring(300.0, 300.0, 700.0, 700.0),
            preset_mix: Vec::new(),
            edges: None,
            color_filter: None,
            holes: Vec::new(),
         cutting: false,
            ..Default::default()
        });
        // 16 ha * 200/ha = 3200 celów
        let (objs, stats) = generate(&proj, None, None, None, None, &|_| {}).unwrap();
        assert!(stats.total >= 2_000, "total={}", stats.total);
        let poly = Polygon {
            rings: vec![square_ring(300.0, 300.0, 700.0, 700.0)],
        };
        for o in &objs {
            assert!(point_in_polygon(&poly, o.x, o.y), "poza poligonem");
        }
        assert_eq!(stats.per_source.len(), 1);
        assert_eq!(stats.per_source[0].0, "Dwór");
    }

    #[test]
    fn areas_coexist_with_mask_zones_sharing_spacing_grid() {
        let green = Rgb8([0, 200, 0]);
        let mut proj = zone_project(&[green]);
        proj.areas.push(AreaDef {
            enabled: true,
            label: "Sad".into(),
            density_per_ha: 150.0,
            species_weights: vec![(1, 1.0)], // model b_1f tylko z obszaru
            polygon: square_ring(350.0, 350.0, 650.0, 650.0),
            preset_mix: Vec::new(),
            edges: None,
            color_filter: None,
            holes: Vec::new(),
         cutting: false,
            ..Default::default()
        });
        let mask = mask_with_rect(100, 100, (10, 10, 80, 80), green);
        let (objs, stats) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();

        let in_area_b = objs
            .iter()
            .filter(|o| o.model == "b_1f" && o.x > 350.0 && o.x < 650.0 && o.y > 350.0 && o.y < 650.0)
            .count();
        assert!(in_area_b > 500, "in_area_b={in_area_b}");
        assert_eq!(stats.per_source.len(), 2);
        // wspólny odstęp: brak dwóch obiektów bliżej niż ~d_plan*0.7 w próbce
        let mut min_d = f64::INFINITY;
        for (i, a) in objs.iter().enumerate().take(3000) {
            for b in objs.iter().skip(i + 1).take(200) {
                min_d = min_d.min((a.x - b.x).hypot(a.y - b.y));
            }
        }
        assert!(min_d > 1.0, "min_d={min_d}");
    }

    // --- pas graniczny -------------------------------------------------------

    #[test]
    fn forest_edge_band_placed_near_boundary_of_mask_zone() {
        let green = Rgb8([0, 200, 0]);
        let mut proj = zone_project(&[green]);
        proj.zones[0].species_weights = vec![(0, 1.0)]; // a_1f tylko wnętrze
        proj.edges = EdgeSettings {
            enabled: true,
            band_width_m: 40.0,
            density_per_ha: 900.0,
            species_weights: vec![(1, 1.0)], // b_1f tylko granica
            blend: false,
            jagged_m: 0.0, // deterministyczny pas do testów odległości
            blend_inside_m: 0.0,
            outside_m: None,
            patch_size_m: 0.0,
            patch_cover: 0.5,
            preset_mix: Vec::new(),
        };

        let mask = mask_with_rect(100, 100, (10, 10, 80, 80), green);
        let (objs, stats) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert!(stats.edge_count > 0, "edge_count={}", stats.edge_count);

        let edge_objs: Vec<&PlacedObject> =
            objs.iter().filter(|o| o.model == "b_1f").collect();
        assert!(!edge_objs.is_empty());

        // granica prostokąta [100..900]^2; pas 40 m + luz na jitter/piksel
        for o in &edge_objs {
            let dx = if o.x < 100.0 {
                100.0 - o.x
            } else if o.x > 900.0 {
                o.x - 900.0
            } else {
                0.0
            };
            let dy = if o.y < 100.0 {
                100.0 - o.y
            } else if o.y > 900.0 {
                o.y - 900.0
            } else {
                0.0
            };
            let dist_to_boundary = dx.hypot(dy);
            assert!(
                dist_to_boundary <= 55.0,
                "obiekt granicy {dist_to_boundary}m od krawędzi"
            );
            // pas jest po wewnętrznej stronie strefy
            assert!(o.x > 90.0 && o.x < 910.0 && o.y > 90.0 && o.y < 910.0);
        }

        // środek lasu bez krzaków granicznych (model b_1f nie występuje głęboko)
        let deep_center = edge_objs
            .iter()
            .filter(|o| o.x > 250.0 && o.x < 750.0 && o.y > 250.0 && o.y < 750.0)
            .count();
        assert_eq!(deep_center, 0, "krzewy graniczne w środku lasu");
    }

    #[test]
    fn forest_edge_along_polygon_boundary() {
        let mut proj = zone_project(&[]);
        proj.zones.clear();
        proj.areas.push(AreaDef {
            enabled: true,
            label: "Gaj".into(),
            density_per_ha: 150.0,
            species_weights: vec![(0, 1.0)],
            polygon: square_ring(300.0, 300.0, 700.0, 700.0),
            preset_mix: Vec::new(),
            edges: None,
            color_filter: None,
            holes: Vec::new(),
         cutting: false,
            ..Default::default()
        });
        proj.edges = EdgeSettings {
            enabled: true,
            band_width_m: 25.0,
            density_per_ha: 900.0,
            species_weights: vec![(1, 1.0)],
            blend: false,
            jagged_m: 0.0,
            blend_inside_m: 0.0,
            outside_m: None,
            patch_size_m: 0.0,
            patch_cover: 0.5,
            preset_mix: Vec::new(),
        };
        let (objs, stats) = generate(&proj, None, None, None, None, &|_| {}).unwrap();
        assert!(stats.edge_count > 0);

        for o in objs.iter().filter(|o| o.model == "b_1f") {
            let d = point_ring_distance(o.x, o.y, &proj.areas[0].polygon);
            assert!(d <= 35.0, "odległość od granicy {d}m");
        }
    }

    #[test]
    fn area_own_edge_without_species_is_clear_error_not_panic() {
        // regresja: własna granica bez gatunków kończyła się paniką wątku
        // (pick_species na pustej liście) i pustym podglądem; ma być
        // czytelny błąd walidacji
        let mut proj = zone_project(&[]);
        proj.zones.clear();
        proj.areas.push(AreaDef {
            enabled: true,
            label: "Pusty pas".into(),
            density_per_ha: 150.0,
            species_weights: vec![(0, 1.0)],
            polygon: square_ring(300.0, 300.0, 700.0, 700.0),
            preset_mix: Vec::new(),
            edges: Some(EdgeSettings {
                enabled: true,
                band_width_m: 25.0,
                density_per_ha: 900.0,
                species_weights: Vec::new(),
                blend: false,
                jagged_m: 0.0,
                blend_inside_m: 0.0,
            outside_m: None,
            patch_size_m: 0.0,
            patch_cover: 0.5,
                preset_mix: Vec::new(),
                }),
            color_filter: None,
            holes: Vec::new(),
            cutting: false,
            ..Default::default()
        });
        let err = generate(&proj, None, None, None, None, &|_| {}).unwrap_err();
        assert!(err.contains("własna granica"), "{err}");
    }

    #[test]
    fn edge_preset_mix_blends_into_band() {
        // miks presetów w granicy: blend udziałów zamiast ręcznej listy
        let green = Rgb8([0, 200, 0]);
        let mut proj = zone_project(&[green]);
        proj.zones[0].species_weights = vec![(0, 1.0)]; // a_1f tylko wnętrze
        proj.edges = EdgeSettings {
            enabled: true,
            band_width_m: 40.0,
            density_per_ha: 160.0, // ignorowane, gdy miks gotowy
            species_weights: Vec::new(), // ręczna pusta: miks przejmuje
            preset_mix: vec![MixEntry {
                name: "Zyw".into(),
                share: 1.0,
                spatial: false,
                color_filter: None,
                density_per_ha: 900.0,
                species_weights: vec![(1, 1.0)], // b_1f tylko granica
            }],
            blend: false,
            jagged_m: 0.0,
            blend_inside_m: 0.0,
            outside_m: None,
            patch_size_m: 0.0,
            patch_cover: 0.5,
        };
        let mask = mask_with_rect(100, 100, (10, 10, 80, 80), green);
        let (objs, stats) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert!(stats.edge_count > 0, "edge_count={}", stats.edge_count);
        assert!(objs.iter().any(|o| o.model == "b_1f"), "brak obiektów z miksu");
        assert!(
            objs.iter().filter(|o| o.model == "b_1f").count() == stats.edge_count as usize,
            "pas ma zawierać tylko miks"
        );
    }

    #[test]
    fn polygon_self_intersects_detects_crossing() {
        // "motylek": obrys przecina sam siebie
        let bowtie = vec![[0.0, 0.0], [100.0, 100.0], [100.0, 0.0], [0.0, 100.0]];
        assert!(polygon_self_intersects(&bowtie));
        let square = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        assert!(!polygon_self_intersects(&square));
        // wypukły pięciokąt
        let pent = vec![
            [50.0, 0.0],
            [100.0, 40.0],
            [80.0, 100.0],
            [20.0, 100.0],
            [0.0, 40.0],
        ];
        assert!(!polygon_self_intersects(&pent));
    }

    #[test]
    fn closed_ring_is_not_self_intersection() {
        // GeoJSON/SHP wymagają powtórzenia pierwszego punktu na końcu —
        // to NIE jest samoprzecięcie
        let closed_square = vec![
            [0.0, 0.0],
            [100.0, 0.0],
            [100.0, 100.0],
            [0.0, 100.0],
            [0.0, 0.0],
        ];
        assert!(find_self_intersection(&closed_square).is_none());
        assert!(!polygon_self_intersects(&closed_square));
    }

    #[test]
    fn find_self_intersection_reports_location() {
        // zamknięty "motylek": przecięcie dokładnie w (50, 50)
        let closed_bowtie = vec![
            [0.0, 0.0],
            [100.0, 100.0],
            [100.0, 0.0],
            [0.0, 100.0],
            [0.0, 0.0],
        ];
        let ix = find_self_intersection(&closed_bowtie).expect("oczekiwano przecięcia");
        assert!((ix.point[0] - 50.0).abs() < 1e-9);
        assert!((ix.point[1] - 50.0).abs() < 1e-9);
        assert_eq!(ix.seg_a, 0);
        assert!(ix.seg_b >= 2 && ix.seg_b <= 3);

        let square = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        assert!(find_self_intersection(&square).is_none());
    }

    #[test]
    fn validate_rejects_self_intersecting_area() {
        let mut proj = zone_project(&[]);
        proj.zones.clear();
        proj.areas.push(AreaDef {
            enabled: true,
            label: "Motylek".into(),
            density_per_ha: 100.0,
            species_weights: vec![(0, 1.0)],
            polygon: vec![[0.0, 0.0], [100.0, 100.0], [100.0, 0.0], [0.0, 100.0]],
            preset_mix: Vec::new(),
            edges: None,
            color_filter: None,
            holes: Vec::new(),
         cutting: false,
            ..Default::default()
        });
        let err = proj.validate().unwrap_err();
        assert!(err.contains("przecina sam siebie"), "{err}");
    }

    #[test]
    fn disabling_mask_zones_skips_them() {
        let green = Rgb8([0, 200, 0]);
        let mut proj = zone_project(&[green]);
        proj.use_mask_zones = false;
        proj.edges.jagged_m = 0.0;
        proj.areas.push(AreaDef {
            enabled: true,
            label: "Tylko poligon".into(),
            density_per_ha: 150.0,
            species_weights: vec![(1, 1.0)],
            polygon: square_ring(300.0, 300.0, 700.0, 700.0),
            preset_mix: Vec::new(),
            edges: None,
            color_filter: None,
            holes: Vec::new(),
         cutting: false,
            ..Default::default()
        });
        let mask = mask_with_rect(60, 60, (5, 5, 50, 50), green);
        let (objs, stats) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        // wyłącznie gatunek z obszaru (b_1f); nic z maski (a_1f)
        assert!(!objs.iter().any(|o| o.model == "a_1f"));
        for o in &objs {
            assert!(o.x > 300.0 && o.x < 700.0 && o.y > 300.0 && o.y < 700.0);
        }
        assert_eq!(stats.per_source.len(), 1);
    }

    #[test]
    fn all_sources_disabled_is_error() {
        let green = Rgb8([0, 200, 0]);
        let mut proj = zone_project(&[green]);
        proj.use_mask_zones = false;
        proj.use_areas = false;
        assert!(proj.validate().is_err());
        let mask = mask_with_rect(30, 30, (5, 5, 20, 20), green);
        assert!(generate(&proj, Some(&mask), None, None, None, &|_| {}).is_err());
    }

    #[test]
    fn jagged_boundary_spills_outside_polygon() {
        // poszarpanie ma "rozlać" część lasu poza ścisły obrys
        let mut proj = zone_project(&[]);
        proj.zones.clear();
        proj.edges.jagged_m = 60.0;
        proj.areas.push(AreaDef {
            enabled: true,
            label: "Ragged".into(),
            density_per_ha: 200.0,
            species_weights: vec![(0, 1.0)],
            polygon: square_ring(300.0, 300.0, 700.0, 700.0),
            preset_mix: Vec::new(),
            edges: None,
            color_filter: None,
            holes: Vec::new(),
         cutting: false,
            ..Default::default()
        });
        let (objs, stats) = generate(&proj, None, None, None, None, &|_| {}).unwrap();
        assert!(stats.total > 1_000);

        let outside = objs
            .iter()
            .filter(|o| {
                !point_in_polygon(
                    &Polygon {
                        rings: vec![square_ring(300.0, 300.0, 700.0, 700.0)],
                    },
                    o.x,
                    o.y,
                )
            })
            .count();
        assert!(outside > 20, "poza obrysem: {outside}");
        // ale nie dalej niż poszarpanie + luz na jitter
        let strict = Polygon {
            rings: vec![square_ring(300.0, 300.0, 700.0, 700.0)],
        };
        for o in &objs {
            if !point_in_polygon(&strict, o.x, o.y) {
                let d = point_ring_distance(o.x, o.y, &proj.areas[0].polygon);
                assert!(d <= 90.0, "poza obrysem o {d}m (limit ~jag+jitter)");
            }
        }
    }

    #[test]
    fn edge_blend_concentrates_near_boundary() {
        // blend: warstwa przy granicy gęstsza niż najdalsza
        let green = Rgb8([0, 200, 0]);
        let mut proj = zone_project(&[green]);
        proj.zones[0].species_weights = vec![(0, 1.0)];
        proj.edges = EdgeSettings {
            enabled: true,
            band_width_m: 50.0,
            density_per_ha: 1200.0,
            species_weights: vec![(1, 1.0)],
            blend: true,
            jagged_m: 0.0,
            blend_inside_m: 0.0,
            outside_m: None,
            patch_size_m: 0.0,
            patch_cover: 0.5,
            preset_mix: Vec::new(),
        };
        let mask = mask_with_rect(100, 100, (10, 10, 80, 80), green);
        let (objs, _) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();

        // pas [100..900]^2; podział na 3 pierścienie głębokości
        let count_at = |lo: f64, hi: f64| -> usize {
            objs.iter()
                .filter(|o| o.model == "b_1f")
                .filter(|o| {
                    let d: f64 = (100.0f64)
                        .min(o.x - 100.0)
                        .min(900.0 - o.x)
                        .min(o.y - 100.0)
                        .min(900.0 - o.y);
                    d >= lo && d < hi
                })
                .count()
        };
        let near = count_at(0.0, 16.7);
        let far = count_at(33.4, 50.1);
        assert!(near > far * 2, "near={near} far={far}");
    }

    #[test]
    fn edge_disabled_by_default() {
        let green = Rgb8([0, 200, 0]);
        let proj = zone_project(&[green]);
        assert!(!proj.edges.enabled);
        let mask = mask_with_rect(60, 60, (10, 10, 40, 40), green);
        let (_, stats) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert_eq!(stats.edge_count, 0);
    }

    #[test]
    fn area_can_have_own_edges_overriding_global() {
        let mut proj = zone_project(&[]);
        proj.zones.clear();
        proj.edges.enabled = false; // globalnie bez granicy

        let mk_area = |label: &str, x0: f64, own: bool| AreaDef {
            enabled: true,
            label: label.into(),
            density_per_ha: 120.0,
            species_weights: vec![(0, 1.0)],
            polygon: square_ring(x0, 300.0, x0 + 250.0, 700.0),
            preset_mix: Vec::new(),
            edges: if own {
                Some(EdgeSettings {
                    enabled: true,
                    band_width_m: 25.0,
                    density_per_ha: 1500.0,
                    species_weights: vec![(1, 1.0)],
                    blend: false,
                    jagged_m: 0.0,
                    blend_inside_m: 0.0,
            outside_m: None,
            patch_size_m: 0.0,
            patch_cover: 0.5,
                    preset_mix: Vec::new(),
                    })
            } else {
                None
            },
            color_filter: None,
            holes: Vec::new(),
         cutting: false,
            ..Default::default()
        };
        proj.areas.push(mk_area("Z granicą", 50.0, true));
        proj.areas.push(mk_area("Bez granicy", 620.0, false));

        let (objs, stats) = generate(&proj, None, None, None, None, &|_| {}).unwrap();
        assert!(stats.edge_count > 0);

        // krzaki (b_1f) tylko przy obszarze "Z granicą"
        let near_a1 = objs.iter().filter(|o| {
            o.model == "b_1f"
                && point_ring_distance(o.x, o.y, &square_ring(50.0, 300.0, 300.0, 700.0)) <= 35.0
        }).count();
        let near_a2 = objs.iter().filter(|o| {
            o.model == "b_1f"
                && point_ring_distance(o.x, o.y, &square_ring(620.0, 300.0, 870.0, 700.0)) <= 35.0
        }).count();
        assert!(near_a1 > 20, "A1={near_a1}");
        assert_eq!(near_a2, 0, "A2={near_a2}");
    }

    #[test]
    fn cut_zone_removes_objects_near_color_plus_margin() {
        // las z POLYGONU + szary pas "drogi" namalowany na masce;
        // wycinanie usuwa drzewa z pasa + bufora, mimo że źródłem
        // jest poligon, a nie kolor maski
        let gray = Rgb8([128, 128, 128]);
        let mut img = image::RgbaImage::new(100, 100);
        for y in 0..100u32 {
            for x in 0..100u32 {
                let c = if (40..60).contains(&x) { gray } else { Rgb8([255, 255, 255]) };
                img.put_pixel(x, y, image::Rgba([c.r(), c.g(), c.b(), 255]));
            }
        }
        let mask = MaskImage::from_dynamic(image::DynamicImage::ImageRgba8(img));

        let mk_proj = |with_cut: bool| {
            let mut proj = zone_project(&[]);
            proj.zones.clear();
            proj.use_mask_zones = false;
            proj.use_areas = true;
            if with_cut {
                proj.cut_zones = vec![crate::preset::CutZone { color: gray, margin_m: 10.0 }];
            }
            proj.areas.push(AreaDef {
                enabled: true,
                label: "Las".into(),
                density_per_ha: 150.0,
                species_weights: vec![(0, 1.0)],
                polygon: square_ring(300.0, 300.0, 700.0, 700.0),
                preset_mix: Vec::new(),
                edges: None,
                color_filter: None,
                holes: Vec::new(),
             cutting: false,
                ..Default::default()
            });
            proj
        };

        let proj = mk_proj(true);
        let (objs, stats) = generate(&proj, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert!(stats.total > 500, "total={}", stats.total);
        assert!(stats.removed_cut > 0, "removed={}", stats.removed_cut);
        assert_eq!(stats.total, objs.len());
        for o in &objs {
            // dylatacja w pikselach maski: bufor 10 m przy ps=10 m dodaje
            // dokładnie jeden pierścień pikseli -> wycięcie [390, 610)
            assert!(o.x < 390.0 || o.x >= 610.0, "obiekt w wycięciu x={}", o.x);
        }

        // kontrola: bez wycinania pas miał drzewa
        let proj2 = mk_proj(false);
        let (objs2, _) = generate(&proj2, Some(&mask), None, None, None, &|_| {}).unwrap();
        assert!(objs2.iter().any(|o| o.x >= 400.0 && o.x <= 600.0));
    }

    #[test]
    fn sat_color_filter_gates_generation() {
        // poligon na całej mapie; podkład: lewa połowa zielona (las), prawa szara (pole)
        let green = Rgb8([60, 140, 60]);
        let gray = Rgb8([180, 180, 180]);
        let mut img = image::RgbaImage::new(100, 100);
        for y in 0..100u32 {
            for x in 0..100u32 {
                let c = if x < 50 { green } else { gray };
                img.put_pixel(x, y, image::Rgba([c.r(), c.g(), c.b(), 255]));
            }
        }
        let sat = MaskImage::from_dynamic(image::DynamicImage::ImageRgba8(img));

        let mut proj = zone_project(&[]);
        proj.zones.clear();
        proj.use_mask_zones = false;
        proj.use_areas = true;
        proj.edges.jagged_m = 0.0;
        proj.areas.push(AreaDef {
            enabled: true,
            label: "Inteligentny".into(),
            density_per_ha: 200.0,
            species_weights: vec![(0, 1.0)],
            polygon: square_ring(50.0, 50.0, 950.0, 950.0),
            preset_mix: Vec::new(),
            edges: None,
            color_filter: Some(crate::preset::ColorFilter {
                samples: vec![green],
                tolerance: 40,
                invert: false,
            }),
            holes: Vec::new(),
         cutting: false,
            ..Default::default()
        });

        // mapa 1000 m, podkład 100 px => 10 m/px; lewa połowa to [0,500)
        let (objs, stats) =
            generate(&proj, None, Some(&sat), None, None, &|_| {}).unwrap();
        assert!(stats.total > 300, "total={}", stats.total);
        for o in &objs {
            assert!(o.x < 510.0, "obiekt na polu x={}", o.x);
        }

        // kontrola bez filtra: drzewa też po prawej
        let mut proj2 = zone_project(&[]);
        proj2.zones.clear();
        proj2.use_mask_zones = false;
        proj2.use_areas = true;
        proj2.edges.jagged_m = 0.0;
        proj2.areas.push(AreaDef {
            enabled: true,
            label: "Bez filtra".into(),
            density_per_ha: 200.0,
            species_weights: vec![(0, 1.0)],
            polygon: square_ring(50.0, 50.0, 950.0, 950.0),
            preset_mix: Vec::new(),
            edges: None,
            color_filter: None,
            holes: Vec::new(),
         cutting: false,
            ..Default::default()
        });
        let (objs2, _) = generate(&proj2, None, Some(&sat), None, None, &|_| {}).unwrap();
        assert!(objs2.iter().any(|o| o.x > 600.0));
    }

    #[test]
    fn sat_color_filter_invert_excludes_samples() {
        // ten sam układ co sat_color_filter_gates_generation, ale invert:
        // próbka zieleni jest IGNOROWANA — drzewa tylko na szarym polu
        let green = Rgb8([60, 140, 60]);
        let gray = Rgb8([180, 180, 180]);
        let mut img = image::RgbaImage::new(100, 100);
        for y in 0..100u32 {
            for x in 0..100u32 {
                let c = if x < 50 { green } else { gray };
                img.put_pixel(x, y, image::Rgba([c.r(), c.g(), c.b(), 255]));
            }
        }
        let sat = MaskImage::from_dynamic(image::DynamicImage::ImageRgba8(img));

        let mut proj = zone_project(&[]);
        proj.zones.clear();
        proj.use_mask_zones = false;
        proj.use_areas = true;
        proj.edges.jagged_m = 0.0;
        proj.areas.push(AreaDef {
            enabled: true,
            label: "Ignoruj zielen".into(),
            density_per_ha: 200.0,
            species_weights: vec![(0, 1.0)],
            polygon: square_ring(50.0, 50.0, 950.0, 950.0),
            preset_mix: Vec::new(),
            edges: None,
            color_filter: Some(crate::preset::ColorFilter {
                samples: vec![green],
                tolerance: 40,
                invert: true,
            }),
            holes: Vec::new(),
         cutting: false,
            ..Default::default()
        });

        let (objs, stats) =
            generate(&proj, None, Some(&sat), None, None, &|_| {}).unwrap();
        assert!(stats.total > 300, "total={}", stats.total);
        for o in &objs {
            assert!(o.x > 490.0, "obiekt na ignorowanej zieleni x={}", o.x);
        }
    }

    #[test]
    fn area_spatial_mix_splits_presets_by_perlin() {
        let mut proj = zone_project(&[]);
        proj.zones.clear();
        proj.use_mask_zones = false;
        proj.use_areas = true;
        proj.edges.jagged_m = 0.0;
        proj.areas.push(AreaDef {
            enabled: true,
            label: "Miks".into(),
            density_per_ha: 200.0,
            species_weights: vec![(0, 1.0), (1, 1.0)],
            polygon: square_ring(50.0, 50.0, 950.0, 950.0),
            mix_spatial: true,
            mix_scale_m: 120.0,
            preset_mix: vec![
                MixEntry {
                    name: "A".into(),
                    share: 1.0,
                    spatial: true,
                    density_per_ha: 220.0,
                    species_weights: vec![(0, 1.0)],
                    ..Default::default()
                },
                MixEntry {
                    name: "B".into(),
                    share: 1.0,
                    spatial: true,
                    density_per_ha: 220.0,
                    species_weights: vec![(1, 1.0)],
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let (objs, stats) = generate(&proj, None, None, None, None, &|_| {}).unwrap();
        assert!(stats.total > 400, "total={}", stats.total);
        let na = objs.iter().filter(|o| o.model == "a_1f").count();
        let nb = objs.iter().filter(|o| o.model == "b_1f").count();
        assert!(na > 50, "na={na}");
        assert!(nb > 50, "nb={nb}");
        // oba presety mają własne joby
        assert!(stats.per_source.iter().any(|(n, _)| n.contains("A")));
        assert!(stats.per_source.iter().any(|(n, _)| n.contains("B")));
    }

    #[test]
    fn area_spatial_mix_opt_out_generates_standard_layer() {
        let mut proj = zone_project(&[]);
        proj.zones.clear();
        proj.use_mask_zones = false;
        proj.use_areas = true;
        proj.edges.jagged_m = 0.0;
        proj.areas.push(AreaDef {
            enabled: true,
            label: "Miks".into(),
            density_per_ha: 150.0,
            species_weights: vec![(0, 1.0)],
            polygon: square_ring(50.0, 50.0, 950.0, 950.0),
            mix_spatial: true,
            mix_scale_m: 180.0,
            preset_mix: vec![
                MixEntry {
                    name: "Laty".into(),
                    share: 1.0,
                    spatial: true,
                    density_per_ha: 160.0,
                    species_weights: vec![(0, 1.0)],
                    ..Default::default()
                },
                MixEntry {
                    name: "Podrost".into(),
                    share: 1.0,
                    spatial: false,
                    density_per_ha: 80.0,
                    species_weights: vec![(1, 1.0)],
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let (objs, stats) = generate(&proj, None, None, None, None, &|_| {}).unwrap();
        assert!(stats.total > 300, "total={}", stats.total);
        let na = objs.iter().filter(|o| o.model == "a_1f").count();
        let nb = objs.iter().filter(|o| o.model == "b_1f").count();
        assert!(na > 50, "na={na}");
        assert!(nb > 20, "nb={nb}");
        assert!(stats.per_source.iter().any(|(n, _)| n.contains("standard")));
    }

    #[test]
    fn area_spatial_mix_separate_color_groups() {
        let green = Rgb8([60, 140, 60]);
        let brown = Rgb8([140, 90, 40]);
        let mut img = image::RgbaImage::new(100, 100);
        for y in 0..100u32 {
            for x in 0..100u32 {
                let c = if x < 50 { green } else { brown };
                img.put_pixel(x, y, image::Rgba([c.r(), c.g(), c.b(), 255]));
            }
        }
        let sat = MaskImage::from_dynamic(image::DynamicImage::ImageRgba8(img));

        let mut proj = zone_project(&[]);
        proj.zones.clear();
        proj.use_mask_zones = false;
        proj.use_areas = true;
        proj.edges.jagged_m = 0.0;
        proj.areas.push(AreaDef {
            enabled: true,
            label: "Kolory".into(),
            density_per_ha: 200.0,
            species_weights: vec![(0, 1.0), (1, 1.0)],
            polygon: square_ring(50.0, 50.0, 950.0, 950.0),
            mix_spatial: true,
            mix_scale_m: 80.0,
            preset_mix: vec![
                MixEntry {
                    name: "Zielony".into(),
                    share: 1.0,
                    spatial: true,
                    density_per_ha: 220.0,
                    species_weights: vec![(0, 1.0)],
                    color_filter: Some(crate::preset::ColorFilter {
                        samples: vec![green],
                        tolerance: 40,
                        invert: false,
                    }),
                },
                MixEntry {
                    name: "Brazowy".into(),
                    share: 1.0,
                    spatial: true,
                    density_per_ha: 220.0,
                    species_weights: vec![(1, 1.0)],
                    color_filter: Some(crate::preset::ColorFilter {
                        samples: vec![brown],
                        tolerance: 40,
                        invert: false,
                    }),
                },
            ],
            ..Default::default()
        });

        let (objs, stats) = generate(&proj, None, Some(&sat), None, None, &|_| {}).unwrap();
        assert!(stats.total > 200, "total={}", stats.total);
        for o in &objs {
            if o.model == "a_1f" {
                assert!(o.x < 530.0, "zielony preset na brazowym x={}", o.x);
            }
            if o.model == "b_1f" {
                assert!(o.x > 470.0, "brazowy preset na zielonym x={}", o.x);
            }
        }
    }
}