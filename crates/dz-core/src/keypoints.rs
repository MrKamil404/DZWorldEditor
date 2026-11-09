//! Named places of a map, kept as Terrain Builder keeps its key points.
use crate::world::World;
use serde::{Deserialize, Serialize};

/// Key point types in the order Terrain Builder stores them.
pub const TYPES: [&str; 19] = [
    "Hill", "BorderCrossing", "VegetationBroadleaf", "VegetationFir", "VegetationPalm", "VegetationVineyard", "NameMarine", "NameCityCapital", "NameCity", "NameVillage",
    "NameLocal", "RockArea", "ViewPoint", "StrongpointArea", "FlatArea", "FlatAreaCity", "FlatAreaCitySmall", "CityCenter", "Airport",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct KeyPoint {
    pub name: String,
    pub kind: String,
    /// Extra lines of the entry in the world config.
    pub properties: String,
    pub x: f32,
    pub z: f32,
    pub angle: f32,
    /// `radiusA` in the world config.
    pub width: f32,
    /// `radiusB` in the world config.
    pub height: f32,
    pub show: bool,
    pub text: String,
}

impl Default for KeyPoint {
    fn default() -> Self {
        Self { name: "KeyPoint".into(), kind: "Hill".into(), properties: String::new(), x: 0.0, z: 0.0, angle: 0.0, width: 100.0, height: 100.0, show: true, text: "KeyPoint".into() }
    }
}

/// Key points of a Terrain Builder project; positions are moved by `offset`.
pub fn parse_tv4p(b: &[u8], offset: [f64; 2]) -> Vec<KeyPoint> {
    let text = |p: usize, tag: u8| -> Option<(String, usize)> {
        if b.get(p..p + 3)? != [tag, 0, 0x0b] {
            return None;
        }
        let n = u16::from_le_bytes(b.get(p + 3..p + 5)?.try_into().ok()?) as usize;
        Some((String::from_utf8_lossy(b.get(p + 5..p + 5 + n)?).into_owned(), p + 5 + n))
    };
    let f64_at = |p: usize| Some(f64::from_le_bytes(b.get(p..p + 8)?.try_into().ok()?));
    let f32_at = |p: usize| Some(f32::from_le_bytes(b.get(p..p + 4)?.try_into().ok()?));
    // Name, text, properties, position, size, two colours, type, a spare, shown and angle
    // always follow each other, which nothing else in the file does.
    let record = |p: usize| -> Option<KeyPoint> {
        let (name, q) = text(p, 0x33)?;
        let (label, q) = text(q, 0x68)?;
        let (properties, q) = text(q, 0x69)?;
        (b.get(q..q + 4)? == [0x65, 0, 0x15, 2]).then_some(())?;
        let (x, z) = (f64_at(q + 4)?, f64_at(q + 12)?);
        let q = q + 20;
        (b.get(q..q + 4)? == [0x5e, 0, 0x12, 2]).then_some(())?;
        let (width, height) = (f32_at(q + 4)?, f32_at(q + 8)?);
        let q = q + 12;
        (b.get(q..q + 3)? == [0x6a, 0, 8] && b.get(q + 7..q + 10)? == [0x6b, 0, 8] && b.get(q + 14..q + 17)? == [0x6c, 0, 5]).then_some(())?;
        let kind = u32::from_le_bytes(b.get(q + 17..q + 21)?.try_into().ok()?) as usize;
        (b.get(q + 21..q + 24)? == [0x6d, 0, 5] && b.get(q + 28..q + 31)? == [0x6e, 0, 9] && b.get(q + 32..q + 35)? == [0x6f, 0, 0x0a]).then_some(())?;
        let (show, angle) = (*b.get(q + 31)? != 0, f32_at(q + 35)?);
        Some(KeyPoint { name, kind: TYPES.get(kind).unwrap_or(&TYPES[0]).to_string(), properties, x: (x - offset[0]) as f32, z: (z - offset[1]) as f32, angle, width, height, show, text: label })
    };
    let mut out = Vec::new();
    let mut p = 0;
    while let Some(k) = b[p..].windows(3).position(|w| w == [0x33, 0, 0x0b]) {
        p += k;
        if let Some(point) = record(p) {
            out.push(point);
        }
        p += 1;
    }
    out
}

/// The entries of a `class Names` file, as Terrain Builder exports them.
pub fn parse_hpp(text: &str) -> Vec<KeyPoint> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("class ") {
        let after = &rest[at + 6..];
        let Some(open) = after.find('{') else { break };
        // Values such as `speech[]={};` end the same way, so only a line of its own counts.
        let mut close = None;
        let mut from = open;
        for line in after[open..].split_inclusive('\n') {
            if line.trim() == "};" {
                close = Some(from - open + line.find("};").unwrap_or(0));
                break;
            }
            from += line.len();
        }
        let Some(close) = close else { break };
        let (name, body) = (after[..open].trim(), &after[open + 1..open + close]);
        rest = &after[open + close + 2..];
        if name == "Names" {
            // A whole class: its entries are read from the inside.
            rest = &after[open + 1..];
            continue;
        }
        let mut p = KeyPoint { name: name.to_string(), text: String::new(), width: 0.0, height: 0.0, ..Default::default() };
        let mut extra = Vec::new();
        for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let Some((key, value)) = line.trim_end_matches(';').split_once('=') else { extra.push(line); continue };
            let (key, value) = (key.trim(), value.trim().trim_matches('"'));
            let number = value.parse::<f32>().unwrap_or(0.0);
            match key {
                "name" => p.text = value.to_string(),
                "type" => p.kind = value.to_string(),
                "radiusA" => p.width = number,
                "radiusB" => p.height = number,
                "angle" => p.angle = number,
                "position[]" => {
                    let mut c = value.trim_matches(['{', '}']).split(',').map(|v| v.trim().parse::<f32>().unwrap_or(0.0));
                    (p.x, p.z) = (c.next().unwrap_or(0.0), c.next().unwrap_or(0.0));
                }
                _ => extra.push(line),
            }
        }
        p.properties = extra.join("\n");
        p.show = !p.text.is_empty();
        out.push(p);
    }
    out
}

/// Adds or updates key points by name. Returns how many were read.
pub fn merge(w: &mut World, points: Vec<KeyPoint>) -> usize {
    let n = points.len();
    for p in points {
        match w.keypoints.iter_mut().find(|x| x.name == p.name) {
            Some(x) => *x = p,
            None => w.keypoints.push(p),
        }
    }
    n
}

/// A key point's name as a config class.
fn class_name(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if s.is_empty() || s.as_bytes()[0].is_ascii_digit() {
        s.insert(0, '_');
    }
    s
}

/// The entries of `class Names` in the world config, indented by `indent`.
pub fn names_text(points: &[KeyPoint], indent: &str) -> String {
    let mut used = std::collections::HashSet::new();
    let mut out = String::new();
    for p in points {
        let mut class = class_name(&p.name);
        let mut n = 1;
        while !used.insert(class.to_ascii_lowercase()) {
            n += 1;
            class = format!("{}_{n}", class_name(&p.name));
        }
        let label = if p.show { p.text.replace('"', "'") } else { String::new() };
        out += &format!("{indent}class {class}\n{indent}{{\n{indent}\tname=\"{label}\";\n{indent}\tposition[]={{{:.2},{:.2}}};\n{indent}\ttype=\"{}\";\n", p.x, p.z, p.kind);
        for line in p.properties.lines().map(str::trim).filter(|l| !l.is_empty()) {
            out += &format!("{indent}\t{line}\n");
        }
        out += &format!("{indent}\tradiusA={:.2};\n{indent}\tradiusB={:.2};\n{indent}\tangle={:.3};\n{indent}}};\n", p.width, p.height, p.angle);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HPP: &str = "class Krypinki\n{\n\tname=\"Krypinki\";\n\tposition[]={2184.11,5979.14};\n\ttype=\"NameVillage\";\n\t\n\tradiusA=78.48;\n\tradiusB=55.55;\n\tangle=0.000;\n};\nclass Other\n{\n\tname=\"\";\n\tposition[]={1,2};\n\ttype=\"Hill\";\n\tspeech[]={};\n\tradiusA=3;\n\tradiusB=4;\n\tangle=5;\n};\n";

    #[test]
    fn terrain_builder_names_are_read_and_written_back() {
        let points = parse_hpp(HPP);
        assert_eq!(points.len(), 2);
        assert_eq!((points[0].name.as_str(), points[0].kind.as_str(), points[0].x, points[0].width), ("Krypinki", "NameVillage", 2184.11, 78.48));
        assert!(!points[1].show);
        assert_eq!(points[1].properties, "speech[]={};");
        let text = names_text(&points, "");
        assert_eq!(parse_hpp(&text), points);
    }

    #[test]
    fn a_whole_names_class_is_understood() {
        let points = parse_hpp(&format!("class Names\n{{\n{HPP}}};\n"));
        assert_eq!(points.len(), 2);
    }

    #[test]
    fn class_names_stay_valid_and_unique() {
        let p = KeyPoint { name: "Stara Wieś".into(), ..Default::default() };
        let text = names_text(&[p.clone(), p], "");
        assert!(text.contains("class Stara_Wie_\n") && text.contains("class Stara_Wie__2\n"));
    }

    #[test]
    fn project_records_give_their_key_points() {
        let mut b = vec![1, 2, 3];
        let string = |tag: u8, s: &str, b: &mut Vec<u8>| { b.extend([tag, 0, 0x0b]); b.extend((s.len() as u16).to_le_bytes()); b.extend(s.as_bytes()); };
        string(0x33, "Glisno", &mut b);
        string(0x68, "Glisno", &mut b);
        string(0x69, "", &mut b);
        b.extend([0x65, 0, 0x15, 2]);
        b.extend(204085.5f64.to_le_bytes());
        b.extend(2969.5f64.to_le_bytes());
        b.extend([0x5e, 0, 0x12, 2]);
        b.extend(122.5f32.to_le_bytes());
        b.extend(86.5f32.to_le_bytes());
        b.extend([0x6a, 0, 8, 255, 136, 0, 255, 0x6b, 0, 8, 255, 170, 68, 255, 0x6c, 0, 5, 8, 0, 0, 0, 0x6d, 0, 5, 0, 0, 0, 0, 0x6e, 0, 9, 1, 0x6f, 0, 0x0a]);
        b.extend(15.0f32.to_le_bytes());
        let points = parse_tv4p(&b, [200000.0, 0.0]);
        assert_eq!(points.len(), 1);
        assert_eq!((points[0].kind.as_str(), points[0].x, points[0].z, points[0].angle, points[0].height, points[0].show), ("NameCity", 4085.5, 2969.5, 15.0, 86.5, true));
    }
}
