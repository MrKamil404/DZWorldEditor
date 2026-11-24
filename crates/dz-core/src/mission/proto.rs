//! Reading and editing building loot prototypes in `mapgroupproto.xml`.
//!
//! Verified against `P:\samples\DayZ-Central-Economy-master\dayzOffline.chernarusplus\mapgroupproto.xml`
//! and `cfglimitsdefinition.xml`:
//! - `<group name="Land_..." [lootmax="N"] [deloot="1"]>` contains `<usage name="..." />`,
//!   optional `<value name="..." />`, and one or more `<container name="..." [lootmax="N"]>`.
//! - Each `<container>` contains `<category name="..." />`, `<tag name="..." />`, and
//!   `<point pos="x y z" range="r" height="h" [flags="32"] />`.
//! - `pos` is in model space relative to the model's bounding-box centre (`ModelInfo::center`),
//!   matching the centre position stored in WRP and `mapgrouppos.xml`.
//! - `flags="32"` marks an outdoor / ground spawn point; indoor points omit `flags`.
use super::xml::{Element, Node};
use super::{file_edit, Mission, MissionEdit};

pub const DEFAULT_CATEGORIES: [&str; 8] = ["tools", "containers", "clothes", "lootdispatch", "food", "weapons", "books", "explosives"];
pub const DEFAULT_TAGS: [&str; 3] = ["floor", "shelves", "ground"];
pub const DEFAULT_USAGES: [&str; 17] = [
    "Military", "Police", "Medic", "Firefighter", "Industrial", "Farm", "Coast", "Town", "Village",
    "Hunting", "Office", "School", "Prison", "Lunapark", "SeasonalEvent", "ContaminatedArea", "Historical",
];
pub const DEFAULT_VALUES: [&str; 5] = ["Tier1", "Tier2", "Tier3", "Tier4", "Unique"];

/// One `<point>` inside a container of `mapgroupproto.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct LootPoint {
    /// Offset from the model bounding-box centre (`ModelInfo::center`), in metres.
    pub pos: [f32; 3],
    pub range: f32,
    pub height: f32,
    /// `32` for outdoor/ground spawns; `0` omits the `flags` attribute.
    pub flags: u32,
}

impl Default for LootPoint {
    fn default() -> Self {
        Self { pos: [0.0, 0.0, 0.0], range: 0.45, height: 1.2, flags: 0 }
    }
}

/// One `<container>` inside a `<group>` of `mapgroupproto.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerProto {
    pub name: String,
    pub lootmax: Option<u32>,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub points: Vec<LootPoint>,
}

impl Default for ContainerProto {
    fn default() -> Self {
        Self {
            name: "lootFloor".into(),
            lootmax: Some(2),
            categories: vec!["tools".into()],
            tags: vec!["floor".into()],
            points: Vec::new(),
        }
    }
}

/// One `<group>` of `mapgroupproto.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupProto {
    pub name: String,
    pub lootmax: Option<u32>,
    pub deloot: bool,
    pub usages: Vec<String>,
    pub values: Vec<String>,
    pub containers: Vec<ContainerProto>,
}

impl GroupProto {
    pub fn default_for(class: &str) -> Self {
        Self {
            name: class.to_string(),
            lootmax: Some(2),
            deloot: false,
            usages: vec!["Town".into()],
            values: Vec::new(),
            containers: vec![ContainerProto::default()],
        }
    }

    pub fn total_points(&self) -> usize {
        self.containers.iter().map(|c| c.points.len()).sum()
    }
}

/// Available limit names from `cfglimitsdefinition.xml` (or vanilla defaults when absent).
#[derive(Clone, Debug, Default)]
pub struct LimitsLists {
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub usages: Vec<String>,
    pub values: Vec<String>,
}

pub fn limits_lists(m: Option<&Mission>) -> LimitsLists {
    let read_list = |section: &str, child: &str, fallback: &[&str]| -> Vec<String> {
        let mut out: Vec<String> = m
            .and_then(|m| m.xml("cfglimitsdefinition.xml"))
            .and_then(|d| d.root.child(section))
            .map(|s| s.children_named(child).filter_map(|e| e.attr("name").map(str::to_string)).collect())
            .unwrap_or_default();
        if out.is_empty() {
            out = fallback.iter().map(|s| (*s).to_string()).collect();
        }
        out
    };
    LimitsLists {
        categories: read_list("categories", "category", &DEFAULT_CATEGORIES),
        tags: read_list("tags", "tag", &DEFAULT_TAGS),
        usages: read_list("usageflags", "usage", &DEFAULT_USAGES),
        values: read_list("valueflags", "value", &DEFAULT_VALUES),
    }
}

/// Whether `mapgroupproto.xml` already defines `class` (case-insensitive).
pub fn has_group(m: &Mission, class: &str) -> bool {
    m.xml("mapgroupproto.xml")
        .is_some_and(|d| d.root.children_named("group").any(|g| g.attr("name").is_some_and(|n| n.eq_ignore_ascii_case(class))))
}

/// Reads `<group name="class">` from `mapgroupproto.xml`, if present.
pub fn read_group(m: &Mission, class: &str) -> Option<GroupProto> {
    let doc = m.xml("mapgroupproto.xml")?;
    let g = doc.root.children_named("group").find(|g| g.attr("name").is_some_and(|n| n.eq_ignore_ascii_case(class)))?;
    Some(parse_group_element(g))
}

pub fn parse_group_element(g: &Element) -> GroupProto {
    let name = g.attr("name").unwrap_or("").to_string();
    let lootmax = g.attr("lootmax").and_then(|s| s.parse().ok());
    let deloot = g.attr("deloot").is_some_and(|s| s == "1" || s.eq_ignore_ascii_case("true"));
    let usages = g.children_named("usage").filter_map(|e| e.attr("name").map(str::to_string)).collect();
    let values = g.children_named("value").filter_map(|e| e.attr("name").map(str::to_string)).collect();
    let containers = g
        .children_named("container")
        .map(|c| {
            let points = c
                .children_named("point")
                .filter_map(|p| {
                    let nums: Vec<f32> = p.attr("pos")?.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                    if nums.len() < 3 {
                        return None;
                    }
                    Some(LootPoint {
                        pos: [nums[0], nums[1], nums[2]],
                        range: p.attr_f32("range").unwrap_or(0.45),
                        height: p.attr_f32("height").unwrap_or(1.2),
                        flags: p.attr("flags").and_then(|s| s.parse().ok()).unwrap_or(0),
                    })
                })
                .collect();
            ContainerProto {
                name: c.attr("name").unwrap_or("lootFloor").to_string(),
                lootmax: c.attr("lootmax").and_then(|s| s.parse().ok()),
                categories: c.children_named("category").filter_map(|e| e.attr("name").map(str::to_string)).collect(),
                tags: c.children_named("tag").filter_map(|e| e.attr("name").map(str::to_string)).collect(),
                points,
            }
        })
        .collect();
    GroupProto { name, lootmax, deloot, usages, values, containers }
}

fn fmt6(v: f32) -> String {
    let v = if v.abs() < 5e-7 { 0.0 } else { v };
    format!("{v:.6}")
}

/// Converts a `GroupProto` into a DayZ-compatible `<group>` XML element.
pub fn to_group_element(group: &GroupProto) -> Element {
    let mut e = Element::new("group").with_attr("name", &group.name);
    if let Some(lm) = group.lootmax {
        e = e.with_attr("lootmax", lm.to_string());
    }
    if group.deloot {
        e = e.with_attr("deloot", "1");
    }
    for u in &group.usages {
        e.push(Element::new("usage").with_attr("name", u));
    }
    for v in &group.values {
        e.push(Element::new("value").with_attr("name", v));
    }
    for c in &group.containers {
        let mut ce = Element::new("container").with_attr("name", &c.name);
        if let Some(lm) = c.lootmax {
            ce = ce.with_attr("lootmax", lm.to_string());
        }
        for cat in &c.categories {
            ce.push(Element::new("category").with_attr("name", cat));
        }
        for tag in &c.tags {
            ce.push(Element::new("tag").with_attr("name", tag));
        }
        for p in &c.points {
            let mut pe = Element::new("point")
                .with_attr("pos", format!("{} {} {}", fmt6(p.pos[0]), fmt6(p.pos[1]), fmt6(p.pos[2])))
                .with_attr("range", fmt6(p.range.max(0.05)))
                .with_attr("height", fmt6(p.height.max(0.1)));
            if p.flags != 0 {
                pe = pe.with_attr("flags", p.flags.to_string());
            }
            ce.push(pe);
        }
        e.push(ce);
    }
    e
}

/// Replaces an existing `<group>` (matching `original_name` or `group.name`) in `mapgroupproto.xml`,
/// or appends it if absent. Returns an undoable `MissionEdit`.
pub fn write_group(m: &mut Mission, original_name: Option<&str>, group: &GroupProto) -> MissionEdit {
    let file = m.key("mapgroupproto.xml").unwrap_or_else(|| "mapgroupproto.xml".into());
    let elem = to_group_element(group);
    let match_name = original_name.unwrap_or(&group.name).to_string();
    let new_name = group.name.clone();
    file_edit(m, &file.clone(), move |m| {
        let had = m.xml(&file).is_some();
        let d = m.ensure_xml(&file, "prototype");
        if !had {
            d.style.space_before_close = true;
        }
        let pos = d.root.children.iter().position(|n| {
            matches!(n, Node::Element(e) if e.name == "group" && e.attr("name").is_some_and(|n| n.eq_ignore_ascii_case(&match_name) || n.eq_ignore_ascii_case(&new_name)))
        });
        match pos {
            Some(i) => d.root.children[i] = Node::Element(elem),
            None => d.root.push(elem),
        }
    })
}

/// Removes `<group name="class">` from `mapgroupproto.xml` if present.
pub fn remove_group(m: &mut Mission, class: &str) -> Option<MissionEdit> {
    if !has_group(m, class) {
        return None;
    }
    let file = m.key("mapgroupproto.xml").unwrap_or_else(|| "mapgroupproto.xml".into());
    let target = class.to_string();
    Some(file_edit(m, &file.clone(), move |m| {
        if let Some(d) = m.xml_mut(&file) {
            d.root.children.retain(|n| {
                !matches!(n, Node::Element(e) if e.name == "group" && e.attr("name").is_some_and(|n| n.eq_ignore_ascii_case(&target)))
            });
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_group_prototype_matches_vanilla_xml() {
        let mut m = Mission::default();
        let mut g = GroupProto::default_for("Land_Shed_M1");
        g.usages = vec!["Industrial".into(), "Farm".into()];
        g.containers[0].categories = vec!["tools".into(), "containers".into()];
        g.containers[0].tags = vec!["ground".into(), "floor".into()];
        g.containers[0].points.push(LootPoint { pos: [0.504883, -1.174019, 0.807373], range: 0.587867, height: 1.469666, flags: 32 });
        g.containers[0].points.push(LootPoint { pos: [-0.25, -1.17, 0.0], range: 0.4, height: 1.2, flags: 0 });
        write_group(&mut m, None, &g);
        assert!(has_group(&m, "land_shed_m1"));
        let back = read_group(&m, "Land_Shed_M1").unwrap();
        assert_eq!(back, g);
        let xml = m.xml("mapgroupproto.xml").unwrap().to_string();
        assert!(xml.contains(r#"flags="32""#), "{xml}");
        assert!(xml.contains(r#"pos="0.504883 -1.174019 0.807373""#), "{xml}");
        assert!(remove_group(&mut m, "Land_Shed_M1").is_some());
        assert!(!has_group(&m, "Land_Shed_M1"));
    }
}
