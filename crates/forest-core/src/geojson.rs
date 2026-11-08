//! Minimalny parser GeoJSON (Polygon / MultiPolygon) + test punktu w poligonie.
//!
//! Współrzędne oczekiwane w metrach mapy (origin SW). Jeśli plik używa
//! współrzędnych z offsetem easting (>= 100000), offset jest automatycznie
//! usuwany podczas normalizacji.

#[derive(Clone, Debug, Default)]
pub struct Polygon {
    /// ring 0 = obrys, kolejne = dziury
    pub rings: Vec<Vec<[f64; 2]>>,
}

#[derive(Clone, Debug, Default)]
pub struct GeoJsonData {
    pub polygons: Vec<Polygon>,
}

impl GeoJsonData {
    pub fn parse(text: &str) -> Result<Self, String> {
        let json: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("GeoJSON: {e}"))?;
        let mut out = Self::default();
        match json.get("type").and_then(|t| t.as_str()) {
            Some("FeatureCollection") => {
                let feats = json
                    .get("features")
                    .and_then(|f| f.as_array())
                    .ok_or("GeoJSON: brak 'features'")?;
                for f in feats {
                    if let Some(geom) = f.get("geometry") {
                        push_geometry(geom, &mut out)?;
                    }
                }
            }
            Some("Feature") => {
                if let Some(geom) = json.get("geometry") {
                    push_geometry(geom, &mut out)?;
                }
            }
            Some("Polygon") | Some("MultiPolygon") => push_geometry(&json, &mut out)?,
            other => return Err(format!("GeoJSON: nieobsługiwany typ {other:?}")),
        }
        Ok(out)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Nie udało się wczytać GeoJSON: {e}"))?;
        Self::parse(&text)
    }

    /// Normalizuje współrzędne: jeśli max x > 100000, odejmuje `easting_offset`.
    pub fn normalize_easting(&mut self, easting_offset: f64) {
        normalize_polygons_easting(&mut self.polygons, easting_offset);
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.polygons.iter().any(|p| point_in_polygon(p, x, y))
    }

    pub fn total_vertices(&self) -> usize {
        self.polygons
            .iter()
            .map(|p| p.rings.iter().map(|r| r.len()).sum::<usize>())
            .sum()
    }
}

use std::path::Path;

/// Wspólna normalizacja współrzędnych poligonów: jeśli max x > 100000,
/// odejmuje `easting_offset` (konwencja TB). Używane przez GeoJSON i Shapefile.
pub fn normalize_polygons_easting(polygons: &mut [Polygon], easting_offset: f64) {
    let mut max_x = f64::NEG_INFINITY;
    for p in polygons.iter() {
        for r in &p.rings {
            for c in r {
                max_x = max_x.max(c[0]);
            }
        }
    }
    if max_x.is_finite() && max_x > 100_000.0 && easting_offset > 0.0 {
        for p in polygons.iter_mut() {
            for r in &mut p.rings {
                for c in r {
                    c[0] -= easting_offset;
                }
            }
        }
    }
}

fn push_geometry(geom: &serde_json::Value, out: &mut GeoJsonData) -> Result<(), String> {
    let ty = geom
        .get("type")
        .and_then(|t| t.as_str())
        .ok_or("GeoJSON: geometria bez 'type'")?;
    let coords = geom
        .get("coordinates")
        .ok_or("GeoJSON: geometria bez 'coordinates'")?;
    match ty {
        "Polygon" => {
            let rings = parse_rings(coords)?;
            if rings.is_empty() {
                return Ok(());
            }
            out.polygons.push(Polygon { rings });
            Ok(())
        }
        "MultiPolygon" => {
            let polys = coords
                .as_array()
                .ok_or("GeoJSON: MultiPolygon bez tablicy")?;
            for poly in polys {
                let rings = parse_rings(poly)?;
                if !rings.is_empty() {
                    out.polygons.push(Polygon { rings });
                }
            }
            Ok(())
        }
        other => Err(format!("GeoJSON: pomijam geometrię '{other}'")),
    }
}

fn parse_rings(v: &serde_json::Value) -> Result<Vec<Vec<[f64; 2]>>, String> {
    let arr = v
        .as_array()
        .ok_or("GeoJSON: pierścień bez tablicy współrzędnych")?;
    let mut rings = Vec::with_capacity(arr.len());
    for ring in arr {
        let pts = ring
            .as_array()
            .ok_or("GeoJSON: pierścień bez punktów")?;
        let mut r = Vec::with_capacity(pts.len());
        for pt in pts {
            let c = pt
                .as_array()
                .ok_or("GeoJSON: punkt bez współrzędnych")?;
            if c.len() < 2 {
                return Err("GeoJSON: punkt wymaga >= 2 współrzędnych".into());
            }
            let x = c[0]
                .as_f64()
                .ok_or("GeoJSON: x nie jest liczbą")?;
            let y = c[1]
                .as_f64()
                .ok_or("GeoJSON: y nie jest liczbą")?;
            r.push([x, y]);
        }
        if r.len() >= 3 {
            rings.push(r);
        }
    }
    Ok(rings)
}

/// Ray casting na zewnętrznym pierścieniu minus dziury.
pub fn point_in_polygon(poly: &Polygon, x: f64, y: f64) -> bool {
    let Some(outer) = poly.rings.first() else {
        return false;
    };
    if !ring_contains(outer, x, y) {
        return false;
    }
    for hole in poly.rings.iter().skip(1) {
        if ring_contains(hole, x, y) {
            return false;
        }
    }
    true
}

pub(crate) fn ring_contains(ring: &[[f64; 2]], x: f64, y: f64) -> bool {
    let n = ring.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let xi = ring[i][0];
        let yi = ring[i][1];
        let xj = ring[j][0];
        let yj = ring[j][1];
        let intersects = (yi > y) != (yj > y)
            && x < (xj - xi) * (y - yi) / (yj - yi) + xi;
        if intersects {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Odległość punktu od odcinka AB (kwadrat).
fn point_segment_distance2(px: f64, py: f64, a: &[f64; 2], b: &[f64; 2]) -> f64 {
    let abx = b[0] - a[0];
    let aby = b[1] - a[1];
    let apx = px - a[0];
    let apy = py - a[1];
    let denom = abx * abx + aby * aby;
    let t = if denom <= f64::EPSILON {
        0.0
    } else {
        ((apx * abx + apy * aby) / denom).clamp(0.0, 1.0)
    };
    let dx = apx - t * abx;
    let dy = apy - t * aby;
    dx * dx + dy * dy
}

/// Minimalna odległość punktu od granicy pierścienia (zamkniętego).
pub fn point_ring_distance(x: f64, y: f64, ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    if n < 2 {
        return f64::INFINITY;
    }
    let mut best = f64::INFINITY;
    let mut j = n - 1;
    for i in 0..n {
        let d2 = point_segment_distance2(x, y, &ring[j], &ring[i]);
        best = best.min(d2);
        j = i;
    }
    best.sqrt()
}

/// Serializuje obszary projektu do GeoJSON FeatureCollection (Polygon,
/// obrys + dziury). Współrzędne dostają ponownie dodany offset easting/
/// northing (konwencja TB), więc plik trafia do układu mapy i może być
/// wprost użyty w GIS albo ponownie zaimportowany. Pierścienie są domykane
/// zgodnie z RFC 7946. Etykieta i flaga aktywności lądują w properties.
pub fn areas_to_geojson(
    areas: &[crate::preset::AreaDef],
    easting_offset: f64,
    northing_offset: f64,
) -> Result<String, String> {
    let ring_json = |ring: &[[f64; 2]]| -> Vec<serde_json::Value> {
        let mut pts: Vec<serde_json::Value> = ring
            .iter()
            .map(|c| serde_json::json!([c[0] + easting_offset, c[1] + northing_offset]))
            .collect();
        // domknięcie: pierwszy punkt powtórzony na końcu (bez duplikatu)
        if pts.len() >= 2 && pts.first() != pts.last() {
            if let Some(first) = pts.first().cloned() {
                pts.push(first);
            }
        }
        pts
    };

    let mut features = Vec::new();
    for a in areas {
        if a.polygon.len() < 3 {
            continue;
        }
        let mut rings = vec![ring_json(&a.polygon)];
        for h in &a.holes {
            if h.len() >= 3 {
                rings.push(ring_json(h));
            }
        }
        features.push(serde_json::json!({
            "type": "Feature",
            "properties": {
                "label": a.label,
                "enabled": a.enabled,
                "density_per_ha": a.density_per_ha,
                "cutting": a.cutting,
            },
            "geometry": { "type": "Polygon", "coordinates": rings },
        }));
    }
    if features.is_empty() {
        return Err("Brak obszarów z poprawnym poligonem do eksportu".into());
    }
    serde_json::to_string_pretty(&serde_json::json!({
        "type": "FeatureCollection",
        "features": features,
    }))
    .map_err(|e| format!("Serializacja GeoJSON: {e}"))
}

/// Obszar odczytany z GeoJSON — właściwości opcjonalne (pliki zewnętrzne
/// mogą ich nie mieć). Pierścienie są już otwarte (bez duplikatu domykającego).
#[derive(Clone, Debug, Default)]
pub struct ImportedArea {
    pub label: Option<String>,
    pub enabled: Option<bool>,
    pub density_per_ha: Option<f32>,
    pub cutting: Option<bool>,
    pub outer: Vec<[f64; 2]>,
    pub holes: Vec<Vec<[f64; 2]>>,
}

fn parse_ring_pts(v: &serde_json::Value) -> Option<Vec<[f64; 2]>> {
    let arr = v.as_array()?;
    let mut r = Vec::with_capacity(arr.len());
    for pt in arr {
        let c = pt.as_array()?;
        if c.len() < 2 {
            return None;
        }
        let x = c[0].as_f64()?;
        let y = c[1].as_f64()?;
        r.push([x, y]);
    }
    Some(r)
}

/// Usuwa powtórzony punkt domykający (RFC 7946) — wewnętrznie obrysy
/// są otwarte, a samoprzecięcia wykrywa `find_self_intersection`.
fn open_ring(mut ring: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    if ring.len() >= 2 {
        let f = ring[0];
        let l = *ring.last().unwrap();
        if (f[0] - l[0]).abs() < 1e-9 && (f[1] - l[1]).abs() < 1e-9 {
            ring.pop();
        }
    }
    ring
}

/// Pierwszy poprawny pierścień jako obrys, kolejne jako dziury.
fn split_rings(mut rings: Vec<Vec<[f64; 2]>>) -> Option<(Vec<[f64; 2]>, Vec<Vec<[f64; 2]>>)> {
    loop {
        match rings.first().map(|r| open_ring(r.clone())) {
            Some(o) if o.len() >= 3 => {
                rings.remove(0);
                let holes: Vec<Vec<[f64; 2]>> = rings
                    .into_iter()
                    .map(|h| open_ring(h))
                    .filter(|h| h.len() >= 3)
                    .collect();
                return Some((o, holes));
            }
            Some(_) => {
                rings.remove(0); // za krótki — pomiń
            }
            None => return None,
        }
    }
}

/// Parsuje GeoJSON do listy obszarów (Polygon / MultiPolygon; FeatureCollection
/// lub pojedyncza geometria). Etykieta/aktywność/gęstość z properties, gdy są.
pub fn areas_from_geojson(text: &str) -> Result<Vec<ImportedArea>, String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("GeoJSON: {e}"))?;

    let null = serde_json::Value::Null;
    let mut geoms: Vec<(&serde_json::Value, &serde_json::Value)> = Vec::new();
    match json.get("type").and_then(|t| t.as_str()) {
        Some("FeatureCollection") => {
            if let Some(feats) = json.get("features").and_then(|f| f.as_array()) {
                for f in feats {
                    geoms.push((
                        f.get("geometry").unwrap_or(&null),
                        f.get("properties").unwrap_or(&null),
                    ));
                }
            }
        }
        Some("Feature") => geoms.push((
            json.get("geometry").unwrap_or(&null),
            json.get("properties").unwrap_or(&null),
        )),
        Some("Polygon") | Some("MultiPolygon") => geoms.push((&json, &null)),
        other => return Err(format!("GeoJSON: nieobsługiwany typ {other:?}")),
    }

    let mut out = Vec::new();
    for (geom, props) in geoms {
        let ty = geom.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let Some(coords) = geom.get("coordinates") else {
            continue;
        };
        let label = props.get("label").and_then(|l| l.as_str()).map(String::from);
        let enabled = props.get("enabled").and_then(|b| b.as_bool());
        let density = props
            .get("density_per_ha")
            .and_then(|d| d.as_f64())
            .map(|d| d as f32);
        let cutting = props.get("cutting").and_then(|b| b.as_bool());
        let mut push_area = |outer: Vec<[f64; 2]>, holes: Vec<Vec<[f64; 2]>>| {
            out.push(ImportedArea {
                label: label.clone(),
                enabled,
                density_per_ha: density,
                cutting,
                outer,
                holes,
            });
        };
        match ty {
            "Polygon" => {
                let Some(rings_raw) = coords.as_array() else {
                    continue;
                };
                let rings: Vec<Vec<[f64; 2]>> =
                    rings_raw.iter().filter_map(parse_ring_pts).collect();
                if let Some((outer, holes)) = split_rings(rings) {
                    push_area(outer, holes);
                }
            }
            "MultiPolygon" => {
                let Some(polys) = coords.as_array() else {
                    continue;
                };
                for poly in polys {
                    let Some(rings_raw) = poly.as_array() else {
                        continue;
                    };
                    let rings: Vec<Vec<[f64; 2]>> =
                        rings_raw.iter().filter_map(parse_ring_pts).collect();
                    if let Some((outer, holes)) = split_rings(rings) {
                        push_area(outer, holes);
                    }
                }
            }
            _ => {} // inne typy pomijamy
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FC: &str = r#"{
      "type": "FeatureCollection",
      "features": [
        { "type": "Feature", "properties": {}, "geometry": {
            "type": "Polygon",
            "coordinates": [[[100,100],[900,100],[900,900],[100,900],[100,100]]] } },
        { "type": "Feature", "properties": {}, "geometry": {
            "type": "MultiPolygon",
            "coordinates": [[[[2000,2000],[3000,2000],[3000,3000],[2000,3000],[2000,2000]]]] } }
      ]
    }"#;

    #[test]
    fn parses_feature_collection() {
        let gj = GeoJsonData::parse(FC).unwrap();
        assert_eq!(gj.polygons.len(), 2);
        assert_eq!(gj.total_vertices(), 10);
    }

    #[test]
    fn point_in_polygon_basic() {
        let gj = GeoJsonData::parse(FC).unwrap();
        assert!(gj.contains(500.0, 500.0));
        assert!(!gj.contains(950.0, 500.0));
        assert!(gj.contains(2500.0, 2500.0));
        assert!(!gj.contains(1500.0, 1500.0));
    }

    #[test]
    fn holes_are_excluded() {
        let gj = GeoJsonData::parse(
            r#"{"type":"Polygon","coordinates":[
                 [[0,0],[100,0],[100,100],[0,100],[0,0]],
                 [[40,40],[60,40],[60,60],[40,60],[40,40]]
               ]}"#,
        )
        .unwrap();
        assert!(gj.contains(10.0, 10.0));
        assert!(!gj.contains(50.0, 50.0));
    }

    #[test]
    fn normalizes_tb_easting() {
        let mut gj = GeoJsonData::parse(
            r#"{"type":"Polygon","coordinates":[
                 [[200100,100],[200900,100],[200900,900],[200100,900],[200100,100]]
               ]}"#,
        )
        .unwrap();
        gj.normalize_easting(200000.0);
        assert!(gj.contains(500.0, 500.0));
        assert!(!gj.contains(1500.0, 500.0));
    }

    #[test]
    fn rejects_garbage() {
        assert!(GeoJsonData::parse("{ not json").is_err());
        assert!(GeoJsonData::parse(r#"{"type":"LineString","coordinates":[[0,0],[1,1]]}"#).is_err());
    }

    #[test]
    fn ring_distance() {
        let square = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        assert!((point_ring_distance(50.0, 50.0, &square) - 50.0).abs() < 1e-9);
        assert!(point_ring_distance(0.0, 50.0, &square).abs() < 1e-9); // na krawędzi
        assert!((point_ring_distance(5.0, 50.0, &square) - 5.0).abs() < 1e-9);
        // róg (100,100): odległość od (103,104) = 5
        assert!((point_ring_distance(103.0, 104.0, &square) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn areas_geojson_roundtrip_with_offsets_and_holes() {
        use crate::preset::AreaDef;
        let area = AreaDef {
            enabled: true,
            label: "Las testowy".into(),
            density_per_ha: 150.0,
            species_weights: Vec::new(),
            preset_mix: Vec::new(),
            edges: None,
            color_filter: None,
            holes: vec![vec![
                [40.0, 40.0],
                [60.0, 40.0],
                [60.0, 60.0],
                [40.0, 60.0],
            ]],
            polygon: vec![[10.0, 10.0], [110.0, 10.0], [110.0, 110.0], [10.0, 110.0]],
            cutting: false,
            ..Default::default()
        };
        let json = super::areas_to_geojson(&[area], 200_000.0, 0.0).unwrap();
        let parsed = GeoJsonData::parse(&json).unwrap();
        assert_eq!(parsed.polygons.len(), 1);
        let poly = &parsed.polygons[0];
        // import zdejmuje offset -> punkt wewnątrz trafia w to samo miejsce
        let mut back = parsed;
        back.normalize_easting(200_000.0);
        let poly = &back.polygons[0];
        assert!(point_in_polygon(poly, 20.0, 20.0));
        assert!(!point_in_polygon(poly, 50.0, 50.0)); // dziura
        // RFC 7946: pierścień domknięty
        let outer = &poly.rings[0];
        assert_eq!(outer.first(), outer.last());
    }

    #[test]
    fn areas_geojson_rejects_empty() {
        assert!(super::areas_to_geojson(&[], 200_000.0, 0.0).is_err());
    }

    #[test]
    fn areas_geojson_export_import_roundtrip() {
        use crate::preset::AreaDef;
        let area = AreaDef {
            enabled: true,
            label: "Sosnowy".into(),
            density_per_ha: 180.0,
            species_weights: Vec::new(),
            preset_mix: Vec::new(),
            edges: None,
            color_filter: None,
            holes: vec![vec![
                [40.0, 40.0],
                [60.0, 40.0],
                [60.0, 60.0],
                [40.0, 60.0],
            ]],
            polygon: vec![[10.0, 10.0], [110.0, 10.0], [110.0, 110.0], [10.0, 110.0]],
            cutting: false,
            ..Default::default()
        };
        let json = super::areas_to_geojson(&[area], 200_000.0, 0.0).unwrap();
        let imported = super::areas_from_geojson(&json).unwrap();
        assert_eq!(imported.len(), 1);
        let a = &imported[0];
        assert_eq!(a.label.as_deref(), Some("Sosnowy"));
        assert_eq!(a.enabled, Some(true));
        assert_eq!(a.density_per_ha, Some(180.0));
        // pierścień otwarty (bez punktu domykającego), offset nadal w danych
        assert_eq!(a.outer.len(), 4);
        assert_eq!(a.outer[0], [200010.0, 10.0]);
        assert_eq!(a.holes.len(), 1);
    }

    #[test]
    fn areas_geojson_import_multipolygon_and_skip_nonpolygons() {
        let json = r#"{
          "type": "FeatureCollection",
          "features": [
            { "type": "Feature", "properties": {"label": "Multi"},
              "geometry": { "type": "MultiPolygon", "coordinates": [
                 [[[0,0],[10,0],[10,10],[0,10],[0,0]]],
                 [[[20,20],[30,20],[30,30],[20,30],[20,20]]] ] } },
            { "type": "Feature", "properties": {},
              "geometry": { "type": "LineString", "coordinates": [[0,0],[1,1]] } }
          ]
        }"#;
        let imported = super::areas_from_geojson(json).unwrap();
        assert_eq!(imported.len(), 2);
        assert_eq!(imported[0].label.as_deref(), Some("Multi"));
        assert_eq!(imported[0].outer[0], [0.0, 0.0]);
        assert!(imported[0].holes.is_empty());
    }

    #[test]
    fn areas_geojson_import_rejects_garbage() {
        assert!(super::areas_from_geojson("{ nope").is_err());
        assert!(super::areas_from_geojson(r#"{"type":"Point","coordinates":[1,2]}"#).is_err());
    }
}