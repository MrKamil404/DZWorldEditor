//! Minimalny parser Shapefile (ESRI `.shp`) dla poligonów.
//!
//! Obsługiwane typy rekordów: Polygon (5), PolygonZ (15), PolygonM (25).
//! Współrzędne X/Y traktowane jak w GeoJSON (metry mapy, origin SW);
//! offset easting można znormalizować metodą `normalize_easting`.
//! Atrybuty z `.dbf` NIE są importowane — tylko geometria.

use crate::geojson::{normalize_polygons_easting, ring_contains, Polygon};
use crate::scatter::{bbox_of, polygon_area_m2};
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct ShapefileData {
    pub polygons: Vec<Polygon>,
}

impl ShapefileData {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let bytes =
            std::fs::read(path).map_err(|e| format!("Nie udało się wczytać Shapefile: {e}"))?;
        Self::parse(&bytes)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 100 {
            return Err("Shapefile: plik za krótki (brak nagłówka 100 B)".into());
        }
        let file_code = read_i32_be(bytes, 0)?;
        if file_code != 9994 {
            return Err(format!(
                "Shapefile: zły kod pliku ({file_code}) — oczekiwano 9994 (.shp)"
            ));
        }
        let version = read_i32_le(bytes, 28)?;
        if version != 1000 {
            return Err(format!("Shapefile: nieobsługiwana wersja {version}"));
        }

        let mut out = Self::default();
        let mut off = 100usize;
        while off + 8 <= bytes.len() {
            let content_words = read_i32_be(bytes, off + 4)?;
            if content_words <= 0 {
                break;
            }
            let content_len = (content_words as usize) * 2;
            let start = off + 8;
            let end = start
                .checked_add(content_len)
                .ok_or("Shapefile: przepełnienie długości rekordu")?;
            if end > bytes.len() {
                return Err("Shapefile: rekord wykracza poza plik".into());
            }
            out.polygons.extend(parse_record(&bytes[start..end])?);
            off = end;
        }

        if out.polygons.is_empty() {
            return Err(
                "Shapefile: nie znaleziono poligonów (obsługiwane: Polygon, PolygonZ, PolygonM)"
                    .into(),
            );
        }
        Ok(out)
    }

    /// Normalizuje współrzędne: jeśli max x > 100000, odejmuje `easting_offset`.
    pub fn normalize_easting(&mut self, easting_offset: f64) {
        normalize_polygons_easting(&mut self.polygons, easting_offset);
    }

    pub fn total_vertices(&self) -> usize {
        self.polygons
            .iter()
            .map(|p| p.rings.iter().map(|r| r.len()).sum::<usize>())
            .sum()
    }
}

fn parse_record(content: &[u8]) -> Result<Vec<Polygon>, String> {
    if content.len() < 4 {
        return Ok(Vec::new());
    }
    let shape_type = read_i32_le(content, 0)?;
    match shape_type {
        5 | 15 | 25 => parse_polygon(content, 4),
        _ => Ok(Vec::new()), // Null Shape, punkty, linie — pomijamy
    }
}

/// Czy pierścień `inner` leży w `outer`. Liczy większość wierzchołków, bo
/// pierścienie Shapefile mogą stykać się pojedynczymi punktami.
fn ring_inside(inner: &[[f64; 2]], outer: &[[f64; 2]]) -> bool {
    let (ix0, iy0, ix1, iy1) = bbox_of(inner);
    let (ox0, oy0, ox1, oy1) = bbox_of(outer);
    if ix0 < ox0 || iy0 < oy0 || ix1 > ox1 || iy1 > oy1 {
        return false;
    }
    let hits = inner
        .iter()
        .filter(|p| ring_contains(outer, p[0], p[1]))
        .count();
    hits * 2 > inner.len()
}

/// Dzieli części rekordu na poligony na podstawie zagnieżdżenia: nieparzysta
/// głębokość to dziura, parzysta to osobny obrys. Rekord może mieć wiele
/// rozłącznych obrysów, a orientacja pierścieni w plikach bywa niepoprawna.
fn rings_to_polygons(mut rings: Vec<Vec<[f64; 2]>>) -> Vec<Polygon> {
    rings.sort_by(|a, b| polygon_area_m2(b).total_cmp(&polygon_area_m2(a)));
    let mut polygons: Vec<Polygon> = Vec::new();
    // dla każdego przetworzonego pierścienia: indeks jego poligonu
    let mut owner: Vec<usize> = Vec::with_capacity(rings.len());
    for (i, ring) in rings.iter().enumerate() {
        let containers: Vec<usize> = (0..i).filter(|&j| ring_inside(ring, &rings[j])).collect();
        match containers.last() {
            // sortowanie malejąco po polu: ostatni kontener jest najbliższy
            Some(&closest) if containers.len() % 2 == 1 => {
                let poly = owner[closest];
                polygons[poly].rings.push(ring.clone());
                owner.push(poly);
            }
            _ => {
                owner.push(polygons.len());
                polygons.push(Polygon { rings: vec![ring.clone()] });
            }
        }
    }
    polygons
}

fn parse_polygon(content: &[u8], mut off: usize) -> Result<Vec<Polygon>, String> {
    // box: 4 × f64 (32 B) — pomijamy
    off += 32;
    let num_parts = read_i32_le(content, off)? as usize;
    let num_points = read_i32_le(content, off + 4)? as usize;
    off += 8;

    if num_parts == 0 || num_points == 0 {
        return Ok(Vec::new());
    }
    if num_parts > 1_000_000 || num_points > 1_000_000 {
        return Err("Shapefile: podejrzanie dużo części/punktów".into());
    }

    let mut parts = Vec::with_capacity(num_parts);
    for _ in 0..num_parts {
        parts.push(read_i32_le(content, off)? as usize);
        off += 4;
    }

    let mut points = Vec::with_capacity(num_points);
    for _ in 0..num_points {
        let x = read_f64_le(content, off)?;
        let y = read_f64_le(content, off + 8)?;
        off += 16;
        points.push([x, y]);
    }

    let mut rings = Vec::with_capacity(num_parts);
    for (i, &p_start) in parts.iter().enumerate() {
        let p_end = if i + 1 < num_parts {
            parts[i + 1]
        } else {
            num_points
        };
        if p_start >= p_end || p_end > num_points {
            return Err("Shapefile: błędne indeksy części poligonu".into());
        }
        let ring: Vec<[f64; 2]> = points[p_start..p_end].to_vec();
        if ring.len() >= 3 {
            rings.push(ring);
        }
    }

    Ok(rings_to_polygons(rings))
}

fn read_i32_be(b: &[u8], off: usize) -> Result<i32, String> {
    let s = b
        .get(off..off + 4)
        .ok_or("Shapefile: ucięty odczyt (i32 BE)")?;
    Ok(i32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_i32_le(b: &[u8], off: usize) -> Result<i32, String> {
    let s = b
        .get(off..off + 4)
        .ok_or("Shapefile: ucięty odczyt (i32 LE)")?;
    Ok(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_f64_le(b: &[u8], off: usize) -> Result<f64, String> {
    let s = b
        .get(off..off + 8)
        .ok_or("Shapefile: ucięty odczyt (f64 LE)")?;
    Ok(f64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_i32_be(v: &mut Vec<u8>, x: i32) {
        v.extend_from_slice(&x.to_be_bytes());
    }
    fn push_i32_le(v: &mut Vec<u8>, x: i32) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn push_f64_le(v: &mut Vec<u8>, x: f64) {
        v.extend_from_slice(&x.to_le_bytes());
    }

    /// Buduje .shp z jednym poligonem (obrys + dziura).
    fn build_shp(records: &[Vec<u8>]) -> Vec<u8> {
        let mut b = Vec::new();
        // nagłówek 100 B
        push_i32_be(&mut b, 9994);
        b.resize(24, 0); // 5×0 (bezużyteczne inty BE)
        let mut total_words = 50i32; // 100 B = 50 słów 16-bitowych
        for r in records {
            total_words += 4 + (r.len() as i32) / 2;
        }
        push_i32_be(&mut b, total_words);
        push_i32_le(&mut b, 1000);
        push_i32_le(&mut b, 5); // Polygon
        // bbox (8 doubles) + z/m — wypełniamy zerami do 100 B
        b.resize(100, 0);

        let mut rec_no = 1i32;
        for r in records {
            push_i32_be(&mut b, rec_no);
            push_i32_be(&mut b, (r.len() as i32) / 2);
            b.extend_from_slice(r);
            rec_no += 1;
        }
        b
    }

    fn polygon_record(rings: &[Vec<[f64; 2]>]) -> Vec<u8> {
        let num_points: usize = rings.iter().map(|r| r.len()).sum();
        let mut c = Vec::new();
        push_i32_le(&mut c, 5); // shape type
        // box (4 doubles)
        push_f64_le(&mut c, 0.0);
        push_f64_le(&mut c, 0.0);
        push_f64_le(&mut c, 100.0);
        push_f64_le(&mut c, 100.0);
        push_i32_le(&mut c, rings.len() as i32);
        push_i32_le(&mut c, num_points as i32);
        let mut part = 0usize;
        for r in rings {
            push_i32_le(&mut c, part as i32);
            part += r.len();
        }
        for r in rings {
            for p in r {
                push_f64_le(&mut c, p[0]);
                push_f64_le(&mut c, p[1]);
            }
        }
        c
    }

    #[test]
    fn parses_polygon_with_hole() {
        let outer = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0], [0.0, 0.0]];
        let hole = vec![[40.0, 40.0], [60.0, 40.0], [60.0, 60.0], [40.0, 60.0], [40.0, 40.0]];
        let bytes = build_shp(&[polygon_record(&[outer, hole])]);
        let shp = ShapefileData::parse(&bytes).unwrap();
        assert_eq!(shp.polygons.len(), 1);
        assert_eq!(shp.polygons[0].rings.len(), 2);
        assert_eq!(shp.polygons[0].rings[0].len(), 5);
        assert_eq!(shp.polygons[0].rings[1].len(), 5);
        // dziura jest wykluczona
        assert!(crate::geojson::point_in_polygon(&shp.polygons[0], 10.0, 10.0));
        assert!(!crate::geojson::point_in_polygon(&shp.polygons[0], 50.0, 50.0));
    }

    fn square(a: f64, b: f64) -> Vec<[f64; 2]> {
        vec![[a, a], [b, a], [b, b], [a, b], [a, a]]
    }

    #[test]
    fn disjoint_parts_become_separate_polygons() {
        let bytes = build_shp(&[polygon_record(&[square(0.0, 10.0), square(100.0, 120.0)])]);
        let shp = ShapefileData::parse(&bytes).unwrap();
        assert_eq!(shp.polygons.len(), 2);
        assert!(shp.polygons.iter().all(|p| p.rings.len() == 1));
        let inside = |x, y| {
            shp.polygons
                .iter()
                .any(|p| crate::geojson::point_in_polygon(p, x, y))
        };
        assert!(inside(5.0, 5.0));
        assert!(inside(110.0, 110.0));
    }

    #[test]
    fn hole_listed_before_outer_and_island_inside_hole() {
        let bytes = build_shp(&[polygon_record(&[
            square(40.0, 60.0),  // dziura zapisana przed obrysem
            square(45.0, 55.0),  // wyspa w dziurze
            square(0.0, 100.0),
        ])]);
        let shp = ShapefileData::parse(&bytes).unwrap();
        assert_eq!(shp.polygons.len(), 2);
        let inside = |x, y| {
            shp.polygons
                .iter()
                .any(|p| crate::geojson::point_in_polygon(p, x, y))
        };
        assert!(inside(10.0, 10.0));
        assert!(!inside(42.0, 42.0));
        assert!(inside(50.0, 50.0));
    }

    #[test]
    fn normalizes_tb_easting() {
        let outer =
            vec![[200100.0, 100.0], [200900.0, 100.0], [200900.0, 900.0], [200100.0, 900.0], [200100.0, 100.0]];
        let bytes = build_shp(&[polygon_record(&[outer])]);
        let mut shp = ShapefileData::parse(&bytes).unwrap();
        shp.normalize_easting(200000.0);
        assert!(crate::geojson::point_in_polygon(&shp.polygons[0], 500.0, 500.0));
        assert!(!crate::geojson::point_in_polygon(&shp.polygons[0], 1500.0, 500.0));
    }

    #[test]
    fn rejects_garbage() {
        assert!(ShapefileData::parse(b"short").is_err());
        let mut bad = vec![0u8; 100];
        bad[0..4].copy_from_slice(&9994i32.to_be_bytes());
        bad[28..32].copy_from_slice(&1000i32.to_le_bytes());
        // brak poligonów -> błąd "nie znaleziono"
        assert!(ShapefileData::parse(&bad).is_err());
    }
}
