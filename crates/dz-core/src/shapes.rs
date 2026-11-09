//! Vector shapes: polygons, polylines and points, with Terrain Builder and ESRI files.
use crate::tv4l::Cursor;
use crate::world::{World, WorldMeta};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeKind {
    Point,
    Polyline,
    Polygon,
}

impl ShapeKind {
    /// Vertices the shape needs to exist.
    pub fn min_points(self) -> usize {
        match self {
            ShapeKind::Point => 1,
            ShapeKind::Polyline => 2,
            ShapeKind::Polygon => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapShape {
    pub kind: ShapeKind,
    /// Map metres; a polygon is not closed by repeating its first point.
    pub points: Vec<[f32; 2]>,
    #[serde(default)]
    pub attributes: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeLayer {
    pub name: String,
    #[serde(default = "yes")]
    pub visible: bool,
    pub shapes: Vec<MapShape>,
}

fn yes() -> bool {
    true
}

/// A shape as read from a file, in the file's own coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct RawShape {
    pub kind: ShapeKind,
    pub points: Vec<[f64; 2]>,
    pub attributes: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawLayer {
    pub name: String,
    pub visible: bool,
    pub shapes: Vec<RawShape>,
}

pub fn point_in_polygon(ring: &[[f32; 2]], x: f32, z: f32) -> bool {
    let mut inside = false;
    let mut j = ring.len().wrapping_sub(1);
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > z) != (b[1] > z) && x < (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

const INDEX_CELL: f32 = 256.0;

/// Quick "is this point inside any polygon" over a whole layer.
#[derive(Clone, Default)]
pub struct PolygonIndex {
    rings: Vec<Vec<[f32; 2]>>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

impl PolygonIndex {
    pub fn new(layer: &ShapeLayer) -> Self {
        let mut out = Self::default();
        let cell = |v: f32| (v / INDEX_CELL).floor() as i32;
        for s in layer.shapes.iter().filter(|s| s.kind == ShapeKind::Polygon && s.points.len() >= 3) {
            let id = out.rings.len() as u32;
            let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for p in &s.points {
                (x0, z0, x1, z1) = (x0.min(p[0]), z0.min(p[1]), x1.max(p[0]), z1.max(p[1]));
            }
            for gz in cell(z0)..=cell(z1) {
                for gx in cell(x0)..=cell(x1) {
                    out.grid.entry((gx, gz)).or_default().push(id);
                }
            }
            out.rings.push(s.points.clone());
        }
        out
    }
    pub fn is_empty(&self) -> bool {
        self.rings.is_empty()
    }
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let key = ((x / INDEX_CELL).floor() as i32, (z / INDEX_CELL).floor() as i32);
        self.grid.get(&key).is_some_and(|l| l.iter().any(|i| point_in_polygon(&self.rings[*i as usize], x, z)))
    }
}

// ---- Terrain Builder

fn skip_value(c: &mut Cursor, ty: u8) -> Result<()> {
    match ty {
        0x0b => {
            let n = c.u16()? as usize;
            c.take(n)?;
        }
        0x05..=0x08 | 0x0a => {
            c.take(4)?;
        }
        0x01 | 0x09 => {
            c.take(1)?;
        }
        0x14 => {
            c.take(8)?;
        }
        0x15 => {
            let n = c.u8()? as usize;
            c.take(n * 8)?;
        }
        0x0c | 0x0d => {
            let n = c.u32()? as usize;
            c.take(n)?;
        }
        t => bail!("Unknown field type {t} in shape file"),
    }
    Ok(())
}

fn read_vertices(c: &mut Cursor, names: &HashMap<u16, String>) -> Result<Vec<[f64; 2]>> {
    c.u32()?;
    let count = c.u32()? as usize;
    let mut out = Vec::with_capacity(count.min(1 << 20));
    for _ in 0..count {
        c.take(3)?;
        let len = c.u32()? as usize;
        let end = c.p + len;
        c.take(6)?;
        while c.p < end {
            let tag = c.u16()?;
            let ty = c.u8()?;
            if ty == 0x15 && names.get(&tag).map(String::as_str) == Some("center") {
                let n = c.u8()? as usize;
                ensure!(n >= 2, "Damaged shape vertex");
                let (x, z) = (c.f64()?, c.f64()?);
                c.take((n - 2) * 8)?;
                out.push([x, z]);
            } else {
                skip_value(c, ty)?;
            }
        }
        ensure!(c.p == end, "Damaged shape vertex");
    }
    Ok(out)
}

/// Reads a Terrain Builder area layer (`.tv4s`).
pub fn parse_tv4s(b: &[u8]) -> Result<RawLayer> {
    let mut c = Cursor { b, p: 0 };
    ensure!(c.take(4)? == b"ALB1", "Not a Terrain Builder shape file");
    c.take(12)?;
    let names = c.dictionary()?;
    let classes = c.dictionary()?;
    c.take(6)?;
    let mut out = RawLayer { visible: true, ..Default::default() };
    while c.p < b.len() {
        let tag = c.u16()?;
        let ty = c.u8()?;
        let name = names.get(&tag).map(String::as_str).unwrap_or("");
        match (name, ty) {
            ("mname", 0x0b) => {
                let n = c.u16()? as usize;
                out.name = String::from_utf8_lossy(c.take(n)?).into_owned();
            }
            ("mbVisible", 0x09) => out.visible = c.u8()? != 0,
            ("mareas", 0x0c) => {
                c.u32()?;
                for _ in 0..c.u32()? {
                    c.take(3)?;
                    let len = c.u32()? as usize;
                    let end = c.p + len;
                    let class = classes.get(&c.u16()?).map(String::as_str).unwrap_or("");
                    c.take(4)?;
                    let kind = match class {
                        "CPointArea" => Some(ShapeKind::Point),
                        "CPolylineArea" => Some(ShapeKind::Polyline),
                        "CPolygonalArea" => Some(ShapeKind::Polygon),
                        _ => None,
                    };
                    let (mut points, mut deleted) = (Vec::new(), false);
                    while c.p < end {
                        let tag = c.u16()?;
                        let ty = c.u8()?;
                        match (names.get(&tag).map(String::as_str).unwrap_or(""), ty) {
                            ("mpVertices", 0x0c) => points = read_vertices(&mut c, &names)?,
                            ("mdeleted", 0x01 | 0x09) => deleted = c.u8()? != 0,
                            _ => skip_value(&mut c, ty)?,
                        }
                    }
                    ensure!(c.p == end, "Damaged shape record");
                    if let Some(kind) = kind.filter(|k| !deleted && points.len() >= k.min_points()) {
                        out.shapes.push(RawShape { kind, points, attributes: Vec::new() });
                    }
                }
            }
            _ => skip_value(&mut c, ty)?,
        }
    }
    Ok(out)
}

// ---- ESRI shapefile

fn le32(b: &[u8], p: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(b.get(p..p + 4).context("Truncated shapefile")?.try_into()?))
}

fn point_at(b: &[u8], p: usize) -> Result<[f64; 2]> {
    let v = b.get(p..p + 16).context("Truncated shapefile")?;
    Ok([f64::from_le_bytes(v[..8].try_into()?), f64::from_le_bytes(v[8..].try_into()?)])
}

/// Shoelace sum; negative for clockwise rings, which is how outer rings are stored.
fn signed_area(ring: &[[f64; 2]]) -> f64 {
    (0..ring.len()).map(|i| { let (a, b) = (ring[i], ring[(i + 1) % ring.len()]); a[0] * b[1] - b[0] * a[1] }).sum::<f64>() * 0.5
}

fn parse_dbf(b: &[u8]) -> Result<Vec<Vec<(String, String)>>> {
    ensure!(b.len() > 32, "Truncated attribute table");
    let records = le32(b, 4)? as usize;
    let header = u16::from_le_bytes(b[8..10].try_into()?) as usize;
    let size = u16::from_le_bytes(b[10..12].try_into()?) as usize;
    let mut fields = Vec::new();
    let mut p = 32;
    while p + 32 <= header && b[p] != 0x0d {
        let raw = &b[p..p + 11];
        let name = String::from_utf8_lossy(&raw[..raw.iter().position(|c| *c == 0).unwrap_or(11)]).into_owned();
        fields.push((name, b[p + 16] as usize));
        p += 32;
    }
    let mut out = Vec::with_capacity(records);
    for r in 0..records {
        let Some(row) = b.get(header + r * size..header + (r + 1) * size) else { break };
        let (mut q, mut attrs) = (1, Vec::with_capacity(fields.len()));
        for (name, len) in &fields {
            let text: String = row.get(q..q + len).unwrap_or(&[]).iter().map(|c| *c as char).collect();
            attrs.push((name.clone(), text.trim().to_string()));
            q += len;
        }
        out.push(attrs);
    }
    Ok(out)
}

/// Reads an ESRI shapefile; `attributes` is the matching `.dbf`, when there is one.
pub fn parse_shp(b: &[u8], attributes: Option<&[u8]>) -> Result<Vec<RawShape>> {
    ensure!(b.len() >= 100 && b[..4] == [0, 0, 0x27, 0x0a], "Not a shapefile");
    let table = attributes.and_then(|d| parse_dbf(d).ok()).unwrap_or_default();
    let mut out = Vec::new();
    let (mut p, mut record) = (100, 0);
    while p + 8 <= b.len() {
        let words = u32::from_be_bytes(b[p + 4..p + 8].try_into()?) as usize;
        let body = b.get(p + 8..p + 8 + words * 2).context("Truncated shapefile")?;
        p += 8 + words * 2;
        let attrs = table.get(record).cloned().unwrap_or_default();
        record += 1;
        if body.len() < 4 {
            continue;
        }
        match le32(body, 0)? {
            1 | 11 | 21 => out.push(RawShape { kind: ShapeKind::Point, points: vec![point_at(body, 4)?], attributes: attrs }),
            t @ (3 | 13 | 23 | 5 | 15 | 25) => {
                let (parts, total) = (le32(body, 36)? as usize, le32(body, 40)? as usize);
                let polygon = matches!(t, 5 | 15 | 25);
                let first = 44 + parts * 4;
                for k in 0..parts {
                    let a = le32(body, 44 + k * 4)? as usize;
                    let e = if k + 1 < parts { le32(body, 44 + (k + 1) * 4)? as usize } else { total };
                    ensure!(a <= e && e <= total, "Damaged shapefile part");
                    let mut points = Vec::with_capacity(e - a);
                    for i in a..e {
                        points.push(point_at(body, first + i * 16)?);
                    }
                    if polygon {
                        if points.len() > 1 && points.first() == points.last() {
                            points.pop();
                        }
                        // Counter-clockwise rings are holes, which areas here do not model.
                        if parts > 1 && signed_area(&points) > 0.0 {
                            continue;
                        }
                    }
                    let kind = if polygon { ShapeKind::Polygon } else { ShapeKind::Polyline };
                    if points.len() >= kind.min_points() {
                        out.push(RawShape { kind, points, attributes: attrs.clone() });
                    }
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Shapes".into())
}

/// Loads a `.tv4s` or `.shp` by its extension.
pub fn load(path: &Path) -> Result<RawLayer> {
    let bytes = std::fs::read(path).with_context(|| path.display().to_string())?;
    let shp = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("shp"));
    let mut layer = if shp {
        let dbf = std::fs::read(path.with_extension("dbf")).ok();
        RawLayer { name: stem(path), visible: true, shapes: parse_shp(&bytes, dbf.as_deref()).with_context(|| path.display().to_string())? }
    } else {
        parse_tv4s(&bytes).with_context(|| path.display().to_string())?
    };
    if layer.name.is_empty() {
        layer.name = stem(path);
    }
    Ok(layer)
}

/// Offset of the file's frame: the project's first, then the usual ones.
pub fn frame_offset(points: &[[f64; 2]], meta: &WorldMeta) -> Option<(f64, f64)> {
    if points.is_empty() {
        return Some((meta.easting, meta.northing));
    }
    let size = meta.size_m() as f64;
    let inside = |ex: f64, ey: f64| {
        let n = points.iter().filter(|p| (-size * 0.05..=size * 1.05).contains(&(p[0] - ex)) && (-size * 0.05..=size * 1.05).contains(&(p[1] - ey))).count();
        n as f64 >= points.len() as f64 * 0.9
    };
    [(meta.easting, meta.northing), (200_000.0, 0.0), (0.0, 0.0), (meta.easting, 0.0)].into_iter().find(|(x, y)| inside(*x, *y))
}

/// Converts file layers to map metres and appends them. Returns how many shapes were added.
pub fn add_layers(w: &mut World, layers: Vec<RawLayer>) -> (usize, Option<String>) {
    let sample: Vec<[f64; 2]> = layers.iter().flat_map(|l| l.shapes.iter().flat_map(|s| s.points.iter().copied())).take(20_000).collect();
    let (offset, warning) = match frame_offset(&sample, &w.meta) {
        Some(o) => (o, None),
        None => ((w.meta.easting, w.meta.northing), Some("Shape coordinates do not match the map frame; check the easting and northing in the project settings.".to_string())),
    };
    let mut total = 0;
    for l in layers {
        total += l.shapes.len();
        let shapes = l.shapes.into_iter().map(|s| MapShape {
            kind: s.kind,
            points: s.points.iter().map(|p| [(p[0] - offset.0) as f32, (p[1] - offset.1) as f32]).collect(),
            attributes: s.attributes,
        }).collect();
        w.shape_layers.push(ShapeLayer { name: l.name, visible: l.visible, shapes });
    }
    (total, warning)
}

fn safe_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.is_empty() { "shapes".into() } else { s }
}

fn write_dbf(shapes: &[&MapShape]) -> Vec<u8> {
    let mut columns: BTreeSet<String> = shapes.iter().flat_map(|s| s.attributes.iter().map(|(k, _)| k.chars().take(10).collect::<String>())).collect();
    // The format needs at least one column.
    columns.insert("ID".into());
    let columns: Vec<String> = columns.into_iter().take(250).collect();
    const WIDTH: usize = 64;
    let size = 1 + columns.len() * WIDTH;
    let header = 32 + columns.len() * 32 + 1;
    let mut b = vec![0u8; 32];
    b[0] = 3;
    b[1..4].copy_from_slice(&[126, 1, 1]);
    b[4..8].copy_from_slice(&(shapes.len() as u32).to_le_bytes());
    b[8..10].copy_from_slice(&(header as u16).to_le_bytes());
    b[10..12].copy_from_slice(&(size as u16).to_le_bytes());
    for c in &columns {
        let mut f = [0u8; 32];
        f[..c.len().min(10)].copy_from_slice(&c.as_bytes()[..c.len().min(10)]);
        f[11] = b'C';
        f[16] = WIDTH as u8;
        b.extend_from_slice(&f);
    }
    b.push(0x0d);
    for (i, s) in shapes.iter().enumerate() {
        b.push(b' ');
        for c in &columns {
            let value = s.attributes.iter().find(|(k, _)| k.chars().take(10).eq(c.chars())).map(|(_, v)| v.clone()).unwrap_or_else(|| if c == "ID" { (i + 1).to_string() } else { String::new() });
            let mut cell = [b' '; WIDTH];
            for (k, ch) in value.chars().take(WIDTH).enumerate() {
                cell[k] = if ch.is_ascii() { ch as u8 } else { b'?' };
            }
            b.extend_from_slice(&cell);
        }
    }
    b.push(0x1a);
    b
}

fn write_kind(base: &Path, kind: ShapeKind, shapes: &[&MapShape], easting: f64, northing: f64) -> Result<()> {
    let code: u32 = match kind { ShapeKind::Point => 1, ShapeKind::Polyline => 3, ShapeKind::Polygon => 5 };
    let mut records: Vec<Vec<u8>> = Vec::with_capacity(shapes.len());
    let mut bounds = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for s in shapes {
        let mut pts: Vec<[f64; 2]> = s.points.iter().map(|p| [p[0] as f64 + easting, p[1] as f64 + northing]).collect();
        if kind == ShapeKind::Polygon {
            if signed_area(&pts) > 0.0 {
                pts.reverse();
            }
            pts.push(pts[0]);
        }
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for p in &pts {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        }
        bounds = [bounds[0].min(b[0]), bounds[1].min(b[1]), bounds[2].max(b[2]), bounds[3].max(b[3])];
        let mut r = code.to_le_bytes().to_vec();
        if kind != ShapeKind::Point {
            for v in b {
                r.extend_from_slice(&v.to_le_bytes());
            }
            r.extend_from_slice(&1u32.to_le_bytes());
            r.extend_from_slice(&(pts.len() as u32).to_le_bytes());
            r.extend_from_slice(&0u32.to_le_bytes());
        }
        for p in &pts {
            r.extend_from_slice(&p[0].to_le_bytes());
            r.extend_from_slice(&p[1].to_le_bytes());
        }
        records.push(r);
    }
    let header = |length_words: usize| {
        let mut h = vec![0u8; 100];
        h[..4].copy_from_slice(&9994u32.to_be_bytes());
        h[24..28].copy_from_slice(&(length_words as u32).to_be_bytes());
        h[28..32].copy_from_slice(&1000u32.to_le_bytes());
        h[32..36].copy_from_slice(&code.to_le_bytes());
        for (i, v) in bounds.iter().enumerate() {
            h[36 + i * 8..44 + i * 8].copy_from_slice(&v.to_le_bytes());
        }
        h
    };
    let total: usize = records.iter().map(|r| r.len() + 8).sum();
    let mut shp = header((100 + total) / 2);
    let mut shx = header((100 + records.len() * 8) / 2);
    for (i, r) in records.iter().enumerate() {
        shx.extend_from_slice(&((shp.len() / 2) as u32).to_be_bytes());
        shx.extend_from_slice(&((r.len() / 2) as u32).to_be_bytes());
        shp.extend_from_slice(&(i as u32 + 1).to_be_bytes());
        shp.extend_from_slice(&((r.len() / 2) as u32).to_be_bytes());
        shp.extend_from_slice(r);
    }
    crate::fsutil::atomic_write(&base.with_extension("shp"), &shp)?;
    crate::fsutil::atomic_write(&base.with_extension("shx"), &shx)?;
    crate::fsutil::atomic_write(&base.with_extension("dbf"), &write_dbf(shapes))
}

/// Writes a layer as shapefiles in the map frame. A shapefile holds one geometry type, so a
/// mixed layer becomes one file per type.
pub fn write_layer(dir: &Path, layer: &ShapeLayer, easting: f64, northing: f64) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let kinds = [(ShapeKind::Polygon, "polygons"), (ShapeKind::Polyline, "lines"), (ShapeKind::Point, "points")];
    let used = kinds.iter().filter(|(k, _)| layer.shapes.iter().any(|s| s.kind == *k)).count();
    let name = safe_name(&layer.name);
    let mut out = Vec::new();
    for (kind, suffix) in kinds {
        let shapes: Vec<&MapShape> = layer.shapes.iter().filter(|s| s.kind == kind).collect();
        if shapes.is_empty() {
            continue;
        }
        let base = dir.join(if used > 1 { format!("{name}_{suffix}") } else { name.clone() });
        write_kind(&base, kind, &shapes, easting, northing)?;
        out.push(base.with_extension("shp"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f32) -> MapShape {
        MapShape { kind: ShapeKind::Polygon, points: vec![[x, 0.0], [x + 10.0, 0.0], [x + 10.0, 10.0], [x, 10.0]], attributes: vec![("KIND".into(), "wood".into())] }
    }

    #[test]
    fn points_inside_polygons_are_found_through_the_index() {
        let i = PolygonIndex::new(&ShapeLayer { name: "a".into(), visible: true, shapes: vec![square(0.0), square(600.0)] });
        assert!(i.contains(5.0, 5.0) && i.contains(605.0, 5.0));
        assert!(!i.contains(50.0, 5.0) && !i.contains(-1.0, 5.0));
    }

    #[test]
    fn shapefile_round_trip_keeps_geometry_and_attributes() {
        let d = tempfile::tempdir().unwrap();
        let layer = ShapeLayer { name: "Las 1".into(), visible: true, shapes: vec![
            square(100.0),
            MapShape { kind: ShapeKind::Polyline, points: vec![[1.0, 2.0], [3.0, 4.0], [5.0, 9.0]], attributes: vec![] },
            MapShape { kind: ShapeKind::Point, points: vec![[7.5, 8.5]], attributes: vec![] },
        ] };
        let files = write_layer(d.path(), &layer, 200000.0, 0.0).unwrap();
        assert_eq!(files.len(), 3);
        let poly = load(&files[0]).unwrap();
        assert_eq!(poly.name, "Las_1_polygons");
        assert_eq!(poly.shapes.len(), 1);
        assert_eq!(poly.shapes[0].points.len(), 4);
        assert!(poly.shapes[0].points.contains(&[200110.0, 10.0]));
        assert!(poly.shapes[0].attributes.contains(&("KIND".to_string(), "wood".to_string())));
        let line = load(&files[1]).unwrap();
        assert_eq!(line.shapes[0].points, vec![[200001.0, 2.0], [200003.0, 4.0], [200005.0, 9.0]]);
        assert_eq!(load(&files[2]).unwrap().shapes[0].kind, ShapeKind::Point);
    }

    #[test]
    fn frames_are_detected() {
        let m = WorldMeta::new("A", 1024, 4.0);
        assert_eq!(frame_offset(&[[201000.0, 500.0]], &m), Some((200_000.0, 0.0)));
        assert_eq!(frame_offset(&[[1000.0, 500.0]], &m), Some((0.0, 0.0)));
        assert_eq!(frame_offset(&[[900000.0, 500.0]], &m), None);
    }

    #[test]
    fn broken_files_fail_cleanly() {
        assert!(parse_tv4s(b"ALB1").is_err());
        assert!(parse_shp(&[0; 20], None).is_err());
        let mut b = vec![0u8; 100];
        b[..4].copy_from_slice(&[0, 0, 0x27, 0x0a]);
        b.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0xff, 0xff]);
        assert!(parse_shp(&b, None).is_err());
    }

    /// Reads every shape layer of the real projects when P: is mounted.
    #[test]
    fn real_shape_files_parse() {
        if std::env::var("DZ_SLOW_TESTS").is_err() { return; }
        for dir in ["P:/BalticrusSource/TerrainBuilder/balticrus.Shapes", "P:/DolinaBobru_Source/dolinabobru.Shapes"] {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "tv4s") {
                    let l = load(&e.path()).unwrap();
                    let n: usize = l.shapes.iter().map(|s| s.points.len()).sum();
                    println!("{} visible {} shapes {} vertices {n}", l.name, l.visible, l.shapes.len());
                    assert!(!l.shapes.is_empty() || n == 0);
                }
            }
        }
    }
}
