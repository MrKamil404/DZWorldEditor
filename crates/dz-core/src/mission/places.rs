//! Things of the mission that have a place on the map: event spawns, player spawns, territory
//! zones, effect areas, underground triggers and map groups. They are gathered into one list
//! for drawing and picking, and moved, added and removed through the trees they live in.
use super::json::{self, vec3};
use super::xml::{fmt_num, Element, Node};
use super::{element_at, element_edit, insert_child, remove_child, Mission, MissionEdit};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PlaceKind {
    /// `<pos>` of an event in `cfgeventspawns.xml`.
    EventSpawn,
    /// `<pos>` of a group of `cfgplayerspawnpoints.xml`.
    PlayerSpawn,
    /// `<zone>` of a territory file in `env/`.
    Territory,
    /// An area of `cfgeffectarea.json`.
    EffectArea,
    /// A safe position of `cfgeffectarea.json`.
    SafePosition,
    /// A trigger of `cfgundergroundtriggers.json`.
    Trigger,
    /// A breadcrumb of an underground trigger.
    Breadcrumb,
    /// A `<group>` of `mapgrouppos.xml`.
    MapGroup,
}

impl PlaceKind {
    pub const ALL: [PlaceKind; 8] = [PlaceKind::EventSpawn, PlaceKind::PlayerSpawn, PlaceKind::Territory, PlaceKind::EffectArea, PlaceKind::SafePosition, PlaceKind::Trigger, PlaceKind::Breadcrumb, PlaceKind::MapGroup];
    pub fn label(self) -> &'static str {
        match self {
            PlaceKind::EventSpawn => "Event spawns",
            PlaceKind::PlayerSpawn => "Player spawns",
            PlaceKind::Territory => "Territories",
            PlaceKind::EffectArea => "Effect areas",
            PlaceKind::SafePosition => "Safe positions",
            PlaceKind::Trigger => "Underground triggers",
            PlaceKind::Breadcrumb => "Breadcrumbs",
            PlaceKind::MapGroup => "Map groups",
        }
    }
}

/// Where a place is stored.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaceRef {
    /// An element of an XML file, by the node indexes from the root.
    Xml { file: String, path: Vec<usize> },
    /// An entry of a JSON file: the array and the index in it (and the breadcrumb index).
    Json { file: String, array: &'static str, index: usize, sub: Option<usize> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub kind: PlaceKind,
    pub at: PlaceRef,
    pub x: f32,
    pub z: f32,
    /// Height, where the file gives one.
    pub y: Option<f32>,
    /// Radius in metres (zones, areas), or 0.
    pub radius: f32,
    /// Size of a box (triggers): x, y, z.
    pub size: Option<[f32; 3]>,
    /// Direction in degrees, where the file gives one.
    pub angle: Option<f32>,
    /// The event group (`cfgeventgroups.xml`) an event spawn places.
    pub sub: Option<String>,
    /// What it belongs to: the event, the spawn group, the territory file and zone name.
    pub group: String,
    pub label: String,
}

fn xml_place(kind: PlaceKind, file: &str, path: Vec<usize>, e: &Element, group: String) -> Option<Place> {
    let (x, y, z) = match e.attr("pos") {
        Some(p) => {
            let v: Vec<f64> = p.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            (*v.first()?, v.get(1).copied(), *v.get(2)?)
        }
        None => (e.attr_f64("x")?, e.attr_f64("y"), e.attr_f64("z")?),
    };
    let label = e.attr("name").map(str::to_string).unwrap_or_else(|| group.clone());
    Some(Place {
        kind,
        at: PlaceRef::Xml { file: file.to_string(), path },
        x: x as f32,
        z: z as f32,
        y: y.map(|v| v as f32),
        radius: e.attr_f32("r").unwrap_or(0.0),
        size: None,
        angle: e.attr_f32("a"),
        sub: e.attr("group").map(str::to_string),
        group,
        label,
    })
}

/// Indexes of the element children of `e`, with the elements.
fn indexed(e: &Element) -> impl Iterator<Item = (usize, &Element)> {
    e.children.iter().enumerate().filter_map(|(i, n)| match n {
        Node::Element(c) => Some((i, c)),
        _ => None,
    })
}

/// All places of the mission of the kinds asked for.
pub fn collect(m: &Mission, kinds: &[PlaceKind]) -> Vec<Place> {
    let mut out = Vec::new();
    let want = |k: PlaceKind| kinds.contains(&k);
    if want(PlaceKind::EventSpawn) {
        for f in m.ce_files("eventspawns") {
            let Some(d) = m.xml(&f) else { continue };
            for (i, ev) in indexed(&d.root).filter(|(_, e)| e.name == "event") {
                let name = ev.attr("name").unwrap_or("").to_string();
                let zone_r = ev.child("zone").and_then(|z| z.attr_f32("r")).unwrap_or(0.0);
                for (j, p) in indexed(ev).filter(|(_, e)| e.name == "pos") {
                    if let Some(mut pl) = xml_place(PlaceKind::EventSpawn, &f, vec![i, j], p, name.clone()) {
                        pl.radius = zone_r;
                        pl.label = p.attr("group").map(|g| format!("{name} ({g})")).unwrap_or_else(|| name.clone());
                        out.push(pl);
                    }
                }
            }
        }
    }
    if want(PlaceKind::PlayerSpawn) {
        if let Some(k) = m.key("cfgplayerspawnpoints.xml") {
            let d = m.xml(&k).unwrap();
            for (si, sec) in indexed(&d.root) {
                let Some((bi, bubbles)) = indexed(sec).find(|(_, e)| e.name == "generator_posbubbles") else { continue };
                for (gi, g) in indexed(bubbles) {
                    match g.name.as_str() {
                        "group" => {
                            let group = format!("{}/{}", sec.name, g.attr("name").unwrap_or(""));
                            for (pi, p) in indexed(g).filter(|(_, e)| e.name == "pos") {
                                out.extend(xml_place(PlaceKind::PlayerSpawn, &k, vec![si, bi, gi, pi], p, group.clone()));
                            }
                        }
                        "pos" => out.extend(xml_place(PlaceKind::PlayerSpawn, &k, vec![si, bi, gi], g, sec.name.clone())),
                        _ => {}
                    }
                }
            }
        }
    }
    if want(PlaceKind::Territory) {
        for f in m.territory_files() {
            let Some(d) = m.xml(&f) else { continue };
            let file_name = f.rsplit('/').next().unwrap_or(&f).trim_end_matches(".xml").to_string();
            for (ti, t) in indexed(&d.root).filter(|(_, e)| e.name == "territory") {
                for (zi, z) in indexed(t).filter(|(_, e)| e.name == "zone") {
                    if let Some(mut p) = xml_place(PlaceKind::Territory, &f, vec![ti, zi], z, file_name.clone()) {
                        p.label = format!("{} ({})", z.attr("name").unwrap_or(""), file_name);
                        out.push(p);
                    }
                }
            }
        }
    }
    if want(PlaceKind::EffectArea) || want(PlaceKind::SafePosition) {
        if let Some(k) = m.key("cfgeffectarea.json") {
            let d = m.json(&k).map(|d| &d.value);
            if want(PlaceKind::EffectArea) {
                if let Some(Value::Array(a)) = d.and_then(|v| v.get("Areas")) {
                    for (i, area) in a.iter().enumerate() {
                        let data = area.get("Data");
                        let Some(p) = vec3(data.and_then(|d| d.get("Pos"))) else { continue };
                        out.push(Place {
                            kind: PlaceKind::EffectArea,
                            at: PlaceRef::Json { file: k.clone(), array: "Areas", index: i, sub: None },
                            x: p[0] as f32,
                            z: p[2] as f32,
                            y: Some(p[1] as f32),
                            radius: json::num(data.and_then(|d| d.get("Radius"))).unwrap_or(0.0) as f32,
                            size: None,
                            angle: None,
                            sub: None,
                            group: area.get("Type").and_then(Value::as_str).unwrap_or("").to_string(),
                            label: area.get("AreaName").and_then(Value::as_str).unwrap_or("").to_string(),
                        });
                    }
                }
            }
            if want(PlaceKind::SafePosition) {
                if let Some(Value::Array(a)) = d.and_then(|v| v.get("SafePositions")) {
                    for (i, p) in a.iter().enumerate() {
                        let Some(arr) = p.as_array() else { continue };
                        let (Some(x), Some(z)) = (arr.first().and_then(Value::as_f64), arr.get(1).and_then(Value::as_f64)) else { continue };
                        out.push(Place { kind: PlaceKind::SafePosition, at: PlaceRef::Json { file: k.clone(), array: "SafePositions", index: i, sub: None }, x: x as f32, z: z as f32, y: None, radius: 0.0, size: None, angle: None, sub: None, group: String::new(), label: format!("#{}", i + 1) });
                    }
                }
            }
        }
    }
    if want(PlaceKind::Trigger) || want(PlaceKind::Breadcrumb) {
        if let Some(k) = m.key("cfgundergroundtriggers.json") {
            if let Some(Value::Array(a)) = m.json(&k).and_then(|d| d.value.get("Triggers")) {
                for (i, t) in a.iter().enumerate() {
                    let Some(p) = vec3(t.get("Position")) else { continue };
                    if want(PlaceKind::Trigger) {
                        let s = vec3(t.get("Size")).map(|s| [s[0] as f32, s[1] as f32, s[2] as f32]);
                        let o = vec3(t.get("Orientation"));
                        out.push(Place { kind: PlaceKind::Trigger, at: PlaceRef::Json { file: k.clone(), array: "Triggers", index: i, sub: None }, x: p[0] as f32, z: p[2] as f32, y: Some(p[1] as f32), radius: 0.0, size: s, angle: o.map(|o| o[0] as f32), sub: None, group: String::new(), label: format!("#{}", i + 1) });
                    }
                    if want(PlaceKind::Breadcrumb) {
                        if let Some(Value::Array(b)) = t.get("Breadcrumbs") {
                            for (j, c) in b.iter().enumerate() {
                                let Some(p) = vec3(c.get("Position")) else { continue };
                                let r = json::num(c.get("Radius")).unwrap_or(-1.0).max(0.0) as f32;
                                out.push(Place { kind: PlaceKind::Breadcrumb, at: PlaceRef::Json { file: k.clone(), array: "Triggers", index: i, sub: Some(j) }, x: p[0] as f32, z: p[2] as f32, y: Some(p[1] as f32), radius: r, size: None, angle: None, sub: None, group: format!("#{}", i + 1), label: format!("#{}.{}", i + 1, j + 1) });
                            }
                        }
                    }
                }
            }
        }
    }
    if want(PlaceKind::MapGroup) {
        if let Some(k) = m.key("mapgrouppos.xml") {
            let d = m.xml(&k).unwrap();
            for (i, g) in indexed(&d.root).filter(|(_, e)| e.name == "group") {
                out.extend(xml_place(PlaceKind::MapGroup, &k, vec![i], g, g.attr("name").unwrap_or("").to_string()));
            }
        }
    }
    out
}

fn json_entry<'a>(v: &'a mut Value, array: &str, index: usize, sub: Option<usize>) -> Option<&'a mut Value> {
    let e = v.get_mut(array)?.as_array_mut()?.get_mut(index)?;
    match sub {
        Some(j) => e.get_mut("Breadcrumbs")?.as_array_mut()?.get_mut(j),
        None => Some(e),
    }
}

/// Moves a place to `x`, `z` (and `y` when given). Hands back the undo.
pub fn move_to(m: &mut Mission, at: &PlaceRef, x: f32, z: f32, y: Option<f32>) -> Option<MissionEdit> {
    match at {
        PlaceRef::Xml { file, path } => element_edit(m, file, path, |e| {
            if let Some(p) = e.attr("pos") {
                let mut v: Vec<String> = p.split_whitespace().map(str::to_string).collect();
                if v.len() >= 3 {
                    v[0] = fmt_num(x as f64);
                    v[2] = fmt_num(z as f64);
                    if let Some(y) = y {
                        v[1] = fmt_num(y as f64);
                    }
                    e.set_attr("pos", v.join(" "));
                }
            } else {
                e.set_attr_f32("x", x);
                e.set_attr_f32("z", z);
                if let (Some(y), true) = (y, e.attr("y").is_some()) {
                    e.set_attr_f32("y", y);
                }
            }
        }),
        PlaceRef::Json { file, array, index, sub } => {
            let edit = super::file_edit(m, file, |m| {
                if let Some(d) = m.json_mut(file) {
                    if let Some(e) = json_entry(&mut d.value, array, *index, *sub) {
                        match *array {
                            "SafePositions" => {
                                if let Some(a) = e.as_array_mut() {
                                    if a.len() >= 2 {
                                        json::set_number(&mut a[0], x as f64);
                                        json::set_number(&mut a[1], z as f64);
                                    }
                                }
                            }
                            "Areas" => {
                                if let Some(p) = e.get_mut("Data").and_then(|d| d.get_mut("Pos")) {
                                    let old = vec3(Some(p)).unwrap_or([0.0; 3]);
                                    json::set_vec3(p, [x as f64, y.map(|y| y as f64).unwrap_or(old[1]), z as f64]);
                                }
                            }
                            _ => {
                                if let Some(p) = e.get_mut("Position") {
                                    let old = vec3(Some(p)).unwrap_or([0.0; 3]);
                                    json::set_vec3(p, [x as f64, y.map(|y| y as f64).unwrap_or(old[1]), z as f64]);
                                }
                            }
                        }
                    }
                }
            });
            Some(edit)
        }
    }
}

/// Changes the radius of a zone or an area.
pub fn set_radius(m: &mut Mission, at: &PlaceRef, r: f32) -> Option<MissionEdit> {
    match at {
        PlaceRef::Xml { file, path } => element_edit(m, file, path, |e| e.set_attr_f32("r", r)),
        PlaceRef::Json { file, array, index, sub } => Some(super::file_edit(m, file, |m| {
            if let Some(e) = m.json_mut(file).and_then(|d| json_entry(&mut d.value, array, *index, *sub)) {
                let slot = if *array == "Areas" { e.get_mut("Data").and_then(|d| d.get_mut("Radius")) } else { e.get_mut("Radius") };
                if let Some(s) = slot {
                    json::set_number(s, r as f64);
                }
            }
        })),
    }
}

/// Removes a place.
pub fn remove(m: &mut Mission, at: &PlaceRef) -> Option<MissionEdit> {
    match at {
        PlaceRef::Xml { file, path } => {
            let (last, parent) = path.split_last()?;
            remove_child(m, file, parent, *last)
        }
        PlaceRef::Json { file, array, index, sub } => Some(super::file_edit(m, file, |m| {
            if let Some(d) = m.json_mut(file) {
                if let Some(a) = d.value.get_mut(*array).and_then(Value::as_array_mut) {
                    match sub {
                        Some(j) => {
                            if let Some(b) = a.get_mut(*index).and_then(|t| t.get_mut("Breadcrumbs")).and_then(Value::as_array_mut) {
                                if *j < b.len() {
                                    b.remove(*j);
                                }
                            }
                        }
                        None => {
                            if *index < a.len() {
                                a.remove(*index);
                            }
                        }
                    }
                }
            }
        })),
    }
}

/// Adds a spawn position to an event of `cfgeventspawns.xml`, making the event when missing.
pub fn add_event_spawn(m: &mut Mission, event: &str, x: f32, z: f32, a: f32) -> Vec<MissionEdit> {
    let mut edits = Vec::new();
    let file = m.ce_files("eventspawns").into_iter().next().unwrap_or_else(|| "cfgeventspawns.xml".into());
    m.ensure_xml(&file, "eventposdef");
    let root = &m.xml(&file).unwrap().root;
    let at = indexed(root).find(|(_, e)| e.name == "event" && e.attr("name") == Some(event)).map(|(i, _)| i);
    let i = match at {
        Some(i) => i,
        None => {
            edits.extend(insert_child(m, &file, &[], None, Element::new("event").with_attr("name", event)));
            m.xml(&file).unwrap().root.children.len() - 1
        }
    };
    let pos = Element::new("pos").with_attr("x", fmt_num(round3(x))).with_attr("z", fmt_num(round3(z))).with_attr("a", fmt_num(round3(a)));
    edits.extend(insert_child(m, &file, &[i], None, pos));
    edits
}

fn round3(v: f32) -> f64 {
    (v as f64 * 1000.0).round() / 1000.0
}

/// Adds a player spawn position to a group of a section (`fresh`, `hop` or `travel`), making
/// the section and group when missing.
pub fn add_player_spawn(m: &mut Mission, section: &str, group: &str, x: f32, z: f32) -> Vec<MissionEdit> {
    let file = m.key("cfgplayerspawnpoints.xml").unwrap_or_else(|| "cfgplayerspawnpoints.xml".into());
    let edit = super::file_edit(m, &file, |m| {
        let d = m.ensure_xml(&file, "playerspawnpoints");
        let sec = d.root.ensure_child(section);
        let bubbles = sec.ensure_child("generator_posbubbles");
        let has_groups = bubbles.children_named("group").next().is_some() || bubbles.children_named("pos").next().is_none();
        let pos = Element::new("pos").with_attr("x", fmt_num(round3(x))).with_attr("z", fmt_num(round3(z)));
        if has_groups {
            if bubbles.children_named("group").all(|g| g.attr("name") != Some(group)) {
                bubbles.push(Element::new("group").with_attr("name", group));
            }
            bubbles.children_named_mut("group").find(|g| g.attr("name") == Some(group)).unwrap().push(pos);
        } else {
            bubbles.push(pos);
        }
    });
    vec![edit]
}

/// Adds a zone to a territory of a territory file, making both when missing.
pub fn add_zone(m: &mut Mission, file: &str, territory: usize, zone: Element) -> Vec<MissionEdit> {
    let mut edits = Vec::new();
    if m.xml(file).is_none() {
        let f = file.to_string();
        edits.push(super::file_edit(m, file, |m| {
            m.ensure_xml(&f, "territory-type");
        }));
    }
    let root = &m.xml(file).unwrap().root;
    let ts: Vec<usize> = indexed(root).filter(|(_, e)| e.name == "territory").map(|(i, _)| i).collect();
    let ti = match ts.get(territory) {
        Some(i) => *i,
        None => {
            edits.extend(insert_child(m, file, &[], None, Element::new("territory").with_attr("color", "4291543295")));
            m.xml(file).unwrap().root.children.len() - 1
        }
    };
    edits.extend(insert_child(m, file, &[ti], None, zone));
    edits
}

/// A zone element as territory files write them.
pub fn zone(name: &str, x: f32, z: f32, r: f32, smin: i64, smax: i64, dmin: i64, dmax: i64) -> Element {
    Element::new("zone")
        .with_attr("name", name)
        .with_attr("smin", smin.to_string())
        .with_attr("smax", smax.to_string())
        .with_attr("dmin", dmin.to_string())
        .with_attr("dmax", dmax.to_string())
        .with_attr("x", fmt_num(round3(x)))
        .with_attr("z", fmt_num(round3(z)))
        .with_attr("r", fmt_num(round3(r)))
}

/// Adds an entry to an array of a JSON file (`Areas`, `SafePositions`, `Triggers`), making
/// the file when missing.
pub fn add_json(m: &mut Mission, file: &str, array: &'static str, entry: Value) -> MissionEdit {
    let f = m.key(file).unwrap_or_else(|| file.to_string());
    super::file_edit(m, &f.clone(), |m| {
        if m.json(&f).is_none() {
            m.put(&f, super::FileData::Json(super::JsonDoc::new(serde_json::json!({}))));
        }
        let d = m.json_mut(&f).unwrap();
        if !d.value.is_object() {
            d.value = serde_json::json!({});
        }
        let o = d.value.as_object_mut().unwrap();
        let a = o.entry(array).or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(a) = a {
            a.push(entry);
        }
    })
}

/// A contaminated area as the game ships them.
pub fn effect_area(name: &str, x: f32, z: f32, radius: f32) -> Value {
    serde_json::json!({
        "AreaName": name,
        "Type": "ContaminatedArea_Static",
        "TriggerType": "ContaminatedTrigger",
        "Data": {
            "Pos": [json::number(round3(x), false), 0, json::number(round3(z), false)],
            "Radius": json::number(radius.round() as f64, false),
            "PosHeight": 25,
            "NegHeight": 10,
            "InnerRingCount": 2,
            "InnerPartDist": 50,
            "OuterRingToggle": 1,
            "OuterPartDist": 40,
            "OuterOffset": 0,
            "VerticalLayers": 0,
            "VerticalOffset": 0,
            "ParticleName": "graphics/particles/contaminated_area_gas_bigass"
        },
        "PlayerData": {
            "AroundPartName": "graphics/particles/contaminated_area_gas_around",
            "TinyPartName": "graphics/particles/contaminated_area_gas_around_tiny",
            "PPERequesterType": "PPERequester_ContaminatedAreaTint"
        }
    })
}

/// An underground trigger box.
pub fn trigger(x: f32, y: f32, z: f32, size: [f32; 3]) -> Value {
    serde_json::json!({
        "Position": [round3(x), round3(y), round3(z)],
        "Orientation": [0, 0, 0],
        "Size": [size[0], size[1], size[2]],
        "EyeAccommodation": 0,
        "Breadcrumbs": [],
        "InterpolationSpeed": 1,
        "AmbientSoundSet": "Underground_SoundSet"
    })
}

/// The element of a place, when it lives in XML.
pub fn element<'a>(m: &'a Mission, at: &PlaceRef) -> Option<&'a Element> {
    match at {
        PlaceRef::Xml { file, path } => element_at(&m.xml(file)?.root, path),
        _ => None,
    }
}

/// The JSON entry of a place.
pub fn json_value<'a>(m: &'a Mission, at: &PlaceRef) -> Option<&'a Value> {
    match at {
        PlaceRef::Json { file, array, index, sub } => {
            let e = m.json(file)?.value.get(*array)?.get(*index)?;
            match sub {
                Some(j) => e.get("Breadcrumbs")?.get(*j),
                None => Some(e),
            }
        }
        _ => None,
    }
}

/// Changes the JSON entry of a place with `f`; hands back the undo.
pub fn edit_json(m: &mut Mission, at: &PlaceRef, f: impl FnOnce(&mut Value)) -> Option<MissionEdit> {
    let PlaceRef::Json { file, array, index, sub } = at else { return None };
    let before = json_value(m, at)?.clone();
    let mut after = before.clone();
    f(&mut after);
    if after == before {
        return None;
    }
    Some(super::file_edit(m, file, |m| {
        if let Some(e) = m.json_mut(file).and_then(|d| json_entry(&mut d.value, array, *index, *sub)) {
            *e = after;
        }
    }))
}

/// One object of a group of `cfgeventgroups.xml`, placed relative to the spawn that uses the
/// group: `x`, `z` along the axes of the map, `y` over the height of the spawn (on the ground
/// when missing), `a` the direction of the object in degrees.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupChild {
    pub kind: String,
    pub x: f32,
    pub z: f32,
    pub y: Option<f32>,
    pub a: f32,
    /// Loot of a dynamic event (`deloot="1"`), with `lootmin`..`lootmax` items.
    pub deloot: bool,
    pub loot: Option<(i64, i64)>,
    /// `spawnsecondary="false"`: no infected of the event around this object.
    pub secondary: bool,
}

impl GroupChild {
    pub fn read(e: &Element) -> Self {
        let lmin = e.attr_i64("lootmin");
        let lmax = e.attr_i64("lootmax");
        Self {
            kind: e.attr("type").unwrap_or("").to_string(),
            x: e.attr_f32("x").unwrap_or(0.0),
            z: e.attr_f32("z").unwrap_or(0.0),
            y: e.attr_f32("y"),
            a: e.attr_f32("a").unwrap_or(0.0),
            deloot: e.attr("deloot").is_some_and(super::ce::is_true),
            loot: lmin.zip(lmax).or(lmin.map(|v| (v, v))).or(lmax.map(|v| (0, v))),
            secondary: !e.attr("spawnsecondary").is_some_and(|v| v.trim().eq_ignore_ascii_case("false") || v.trim() == "0"),
        }
    }
    /// Where the object stands for a spawn at `x`, `z`.
    pub fn world(&self, x: f32, z: f32) -> [f32; 2] {
        [x + self.x, z + self.z]
    }
}

/// The groups of `cfgeventgroups.xml` by lower-case name: the name as written, the index of
/// the group in the root and its objects.
pub fn event_groups(m: &Mission) -> std::collections::HashMap<String, (String, usize, Vec<GroupChild>)> {
    let mut out = std::collections::HashMap::new();
    if let Some(d) = m.xml("cfgeventgroups.xml") {
        for (i, g) in indexed(&d.root).filter(|(_, e)| e.name == "group") {
            let name = g.attr("name").unwrap_or("").to_string();
            let children = g.children_named("child").map(GroupChild::read).collect();
            out.insert(name.to_ascii_lowercase(), (name, i, children));
        }
    }
    out
}

/// Adds a spawn of `event` at `x`, `z` that places the group `group`, at height `y` above the
/// sea, as the game's static events (StaticMilitaryConvoy, StaticTrain...) do.
pub fn add_group_spawn(m: &mut Mission, event: &str, group: &str, x: f32, z: f32, y: f32) -> Vec<MissionEdit> {
    let mut edits = add_event_spawn(m, event, x, z, 0.0);
    // The spawn just added is the last child of the event.
    if let Some(MissionEdit::Child(p, index, None)) = edits.last().cloned() {
        let mut path = p.path.clone();
        path.push(index);
        edits.extend(element_edit(m, &p.file, &path, |e| {
            e.set_attr("y", fmt_num(round3(y)));
            e.set_attr("group", group);
        }));
    }
    edits
}

/// Sets or clears the group of an event spawn; a spawn with a group gets its height `y`.
pub fn set_spawn_group(m: &mut Mission, at: &PlaceRef, group: Option<&str>, y: f32) -> Option<MissionEdit> {
    let PlaceRef::Xml { file, path } = at else { return None };
    element_edit(m, file, path, |e| match group {
        Some(g) => {
            e.set_attr("group", g);
            if e.attr("y").is_none() {
                e.set_attr("y", fmt_num(round3(y)));
            }
        }
        None => e.remove_attr("group"),
    })
}

/// An object to make a group from: its class, where it stands and where it looks.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupObject {
    pub kind: String,
    pub x: f32,
    pub z: f32,
    /// Height above the sea of the object, and of the ground under it.
    pub y: f32,
    pub ground: f32,
    /// Direction in degrees, from the north towards the east.
    pub heading: f32,
}

/// A group of `cfgeventgroups.xml` holding the objects around the first one, which stands
/// where the spawn will be. Objects more than half a metre over the ground keep their height
/// over the spawn; the others stand on the ground. Each object gets the loot of a dynamic
/// event (`deloot`), as the convoys of the game do.
pub fn group_from_objects(name: &str, objects: &[GroupObject], loot: (i64, i64)) -> Option<(Element, [f32; 3])> {
    let first = objects.first()?;
    let (px, pz, py) = (first.x, first.z, first.ground);
    let mut g = Element::new("group").with_attr("name", name);
    for o in objects {
        let mut c = Element::new("child").with_attr("type", o.kind.clone()).with_attr("deloot", "1").with_attr("lootmax", loot.1.to_string()).with_attr("lootmin", loot.0.to_string());
        c = c.with_attr("x", fmt_num(round3(o.x - px))).with_attr("z", fmt_num(round3(o.z - pz))).with_attr("a", fmt_num(round3(o.heading.rem_euclid(360.0))));
        if o.y - o.ground > 0.5 {
            c = c.with_attr("y", fmt_num(round3(o.y - py)));
        }
        g.push(c);
    }
    Some((g, [px, py, pz]))
}

/// Adds (or replaces) a group in `cfgeventgroups.xml`.
pub fn put_group(m: &mut Mission, group: Element) -> MissionEdit {
    let file = m.key("cfgeventgroups.xml").unwrap_or_else(|| "cfgeventgroups.xml".into());
    super::file_edit(m, &file.clone(), |m| {
        let d = m.ensure_xml(&file, "eventgroupdef");
        let name = group.attr("name").unwrap_or("").to_string();
        match d.root.children.iter().position(|n| matches!(n, Node::Element(e) if e.name == "group" && e.attr("name") == Some(name.as_str()))) {
            Some(i) => d.root.children[i] = Node::Element(group),
            None => d.root.push(group),
        }
    })
}

/// Objects of a group as `<child>` elements around a spawn at `origin` (x, height, z).
pub fn child_elements(objects: &[GroupObject], origin: [f32; 3], loot: (i64, i64)) -> Vec<Element> {
    let [px, py, pz] = origin;
    objects.iter().map(|o| {
        let mut c = Element::new("child").with_attr("type", o.kind.clone()).with_attr("deloot", "1").with_attr("lootmax", loot.1.to_string()).with_attr("lootmin", loot.0.to_string());
        c = c.with_attr("x", fmt_num(round3(o.x - px))).with_attr("z", fmt_num(round3(o.z - pz))).with_attr("a", fmt_num(round3(o.heading.rem_euclid(360.0))));
        if o.y - o.ground > 0.5 {
            c = c.with_attr("y", fmt_num(round3(o.y - py)));
        }
        c
    }).collect()
}

/// Adds objects to a group, placed around a spawn of the group at `origin`.
pub fn add_to_group(m: &mut Mission, group: &str, objects: &[GroupObject], origin: [f32; 3], loot: (i64, i64)) -> Option<MissionEdit> {
    let file = m.key("cfgeventgroups.xml")?;
    let d = m.xml(&file)?;
    let i = indexed(&d.root).find(|(_, e)| e.name == "group" && e.attr("name").is_some_and(|n| n.eq_ignore_ascii_case(group))).map(|(i, _)| i)?;
    let children = child_elements(objects, origin, loot);
    element_edit(m, &file, &[i], |g| {
        for c in children {
            g.push(c);
        }
    })
}
