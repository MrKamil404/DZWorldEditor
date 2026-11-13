//! Mission files made from the map: map groups of the placed buildings, clusters of the fruit
//! trees and stones, territory zones from key points, area flags from shapes.
use super::xml::{fmt_num, Element, Node};
use super::{file_edit, FileData, Mission, MissionEdit};
use crate::wrp::WrpObject;
use std::collections::{HashMap, HashSet};

/// Class of the map object a model is: `Land_` and the name of the model.
pub fn proxy_class(model: &str) -> String {
    let file = model.rsplit(['\\', '/']).next().unwrap_or(model);
    let stem = file.strip_suffix(".p3d").or_else(|| file.strip_suffix(".P3D")).unwrap_or(file);
    format!("Land_{stem}")
}

/// Position, roll-pitch-yaw and angle of an object as the economy writes them: the yaw turns
/// from the north towards the east, `a` is 90 degrees less the yaw.
pub fn placement(m: &[f32; 12]) -> ([f64; 3], [f64; 3], f64) {
    let r = [m[0] as f64, m[1] as f64, m[2] as f64];
    let u = [m[3] as f64, m[4] as f64, m[5] as f64];
    let d = [m[6] as f64, m[7] as f64, m[8] as f64];
    let len = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    let (lr, lu, ld) = (len(r), len(u), len(d));
    let yaw = (d[0] / ld).atan2(d[2] / ld).to_degrees();
    let pitch = (d[1] / ld).clamp(-1.0, 1.0).asin().to_degrees();
    let roll = -(r[1] / lr).atan2(u[1] / lu).to_degrees();
    let mut a = 90.0 - yaw;
    if a > 180.0 {
        a -= 360.0;
    }
    if a <= -180.0 {
        a += 360.0;
    }
    ([m[9] as f64, m[10] as f64, m[11] as f64], [roll, pitch, yaw], a)
}

fn f6(v: f64) -> String {
    let v = if v.abs() < 5e-7 { 0.0 } else { v };
    format!("{v:.6}")
}

/// The `<group>` of `mapgrouppos.xml` for an object.
pub fn group_element(name: &str, m: &[f32; 12], with_rpy: bool) -> Element {
    let (p, rpy, a) = placement(m);
    let mut e = Element::new("group").with_attr("name", name).with_attr("pos", format!("{} {} {}", f6(p[0]), f6(p[1]), f6(p[2])));
    if with_rpy {
        e = e.with_attr("rpy", format!("{} {} {}", f6(rpy[0]), f6(rpy[1]), f6(rpy[2])));
    }
    e.with_attr("a", f6(a))
}

/// Classes of `mapgroupproto.xml`, by their lower-case name.
pub fn proto_classes(m: &Mission) -> HashMap<String, String> {
    m.xml("mapgroupproto.xml").map(|d| d.root.children_named("group").filter_map(|g| g.attr("name")).map(|n| (n.to_ascii_lowercase(), n.to_string())).collect()).unwrap_or_default()
}

/// Exports of `mapclusterproto.xml`: the cluster name by the lower-case model path.
pub fn cluster_exports(m: &Mission) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(d) = m.xml("mapclusterproto.xml") {
        if let Some(c) = d.root.child("clusters") {
            for e in c.children_named("export") {
                if let (Some(n), Some(s)) = (e.attr("name"), e.attr("shape")) {
                    out.insert(crate::objects::ObjectStore::normalize_path(s), n.to_string());
                }
            }
        }
    }
    out
}

/// What making the map groups found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupReport {
    pub groups: usize,
    pub clusters: usize,
    /// Classes placed on the map that have no prototype, with how many there are.
    pub without_proto: Vec<(String, usize)>,
}

/// Makes `mapgrouppos.xml` from the placed objects whose class has a prototype, and the cluster
/// file from those whose model is a cluster export. Hands back the undo and what was found.
pub fn map_groups(m: &mut Mission, objects: &[WrpObject], clusters: bool) -> (Vec<MissionEdit>, GroupReport) {
    let protos = proto_classes(m);
    let exports = cluster_exports(m);
    let mut groups = Vec::new();
    let mut cluster_groups = Vec::new();
    let mut missing: HashMap<String, usize> = HashMap::new();
    for o in objects {
        let model = crate::objects::ObjectStore::normalize_path(&o.model);
        if clusters {
            if let Some(name) = exports.get(&model) {
                cluster_groups.push(group_element(name, &o.matrix, false));
                continue;
            }
        }
        let class = proxy_class(&o.model);
        match protos.get(&class.to_ascii_lowercase()) {
            Some(name) => groups.push(group_element(name, &o.matrix, true)),
            None => {
                if looks_like_building(&model) {
                    *missing.entry(class).or_default() += 1;
                }
            }
        }
    }
    let mut edits = Vec::new();
    let report = GroupReport { groups: groups.len(), clusters: cluster_groups.len(), without_proto: { let mut v: Vec<_> = missing.into_iter().collect(); v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0))); v } };
    let file = m.key("mapgrouppos.xml").unwrap_or_else(|| "mapgrouppos.xml".into());
    edits.push(file_edit(m, &file.clone(), |m| {
        let had = m.xml(&file).is_some();
        let d = m.ensure_xml(&file, "map");
        if !had {
            d.style.space_before_close = true;
        }
        // Includes and comments stay; the groups are replaced.
        d.root.remove_children("group");
        let at = d.root.children.iter().position(|n| matches!(n, Node::Element(e) if e.name == "include")).unwrap_or(d.root.children.len());
        for (i, g) in groups.into_iter().enumerate() {
            d.root.children.insert(at + i, Node::Element(g));
        }
    }));
    if clusters && !cluster_groups.is_empty() {
        let file = m.key("mapgroupcluster.xml").unwrap_or_else(|| "mapgroupcluster.xml".into());
        let numbered: Vec<String> = m.files.keys().filter(|k| { let l = k.to_ascii_lowercase(); l.starts_with("mapgroupcluster") && l != "mapgroupcluster.xml" }).cloned().collect();
        edits.push(file_edit(m, &file.clone(), |m| {
            let mut d = super::xml::XmlDoc::new(Element::new("map"));
            d.style.space_before_close = true;
            for g in cluster_groups {
                d.root.push(g);
            }
            m.put(&file, FileData::Xml(d));
        }));
        // The numbered parts held the old clusters; the new file holds them all.
        for k in numbered {
            edits.push(file_edit(m, &k.clone(), |m| {
                m.remove(&k);
            }));
        }
    }
    (edits, report)
}

/// Structures the economy can hold loot in, by their folder.
fn looks_like_building(model: &str) -> bool {
    model.contains("\\structures\\") || model.contains("\\military\\") || model.contains("\\residential\\") || model.contains("\\industrial\\") || model.contains("\\wrecks\\")
}

/// Copies the prototypes of `classes` from another `mapgroupproto.xml` into the mission,
/// leaving those the mission has. Hands back the undo and how many were copied.
pub fn copy_prototypes(m: &mut Mission, from: &super::xml::XmlDoc, classes: &HashSet<String>) -> (Option<MissionEdit>, usize) {
    let have: HashSet<String> = proto_classes(m).into_keys().collect();
    let wanted: Vec<Element> = from.root.children_named("group").filter(|g| g.attr("name").is_some_and(|n| { let l = n.to_ascii_lowercase(); classes.contains(&l) && !have.contains(&l) })).cloned().collect();
    if wanted.is_empty() {
        return (None, 0);
    }
    let n = wanted.len();
    let file = m.key("mapgroupproto.xml").unwrap_or_else(|| "mapgroupproto.xml".into());
    let edit = file_edit(m, &file.clone(), |m| {
        let d = m.ensure_xml(&file, "prototype");
        for g in wanted {
            d.root.push(g);
        }
    });
    (Some(edit), n)
}

/// A prototype with one container and no loot points yet, for a class the economy should know.
pub fn empty_prototype(class: &str) -> Element {
    Element::new("group").with_attr("name", class).with_attr("lootmax", "2").with_child(Element::new("usage").with_attr("name", "Town")).with_child(Element::new("container").with_attr("name", "lootFloor").with_attr("lootmax", "2").with_child(Element::new("category").with_attr("name", "tools")).with_child(Element::new("tag").with_attr("name", "floor")))
}

/// Infected zones at key points: one zone per key point of the kinds given, named `event`.
pub fn zones_from_keypoints(points: &[crate::keypoints::KeyPoint], kinds: &[&str], event: &str, dmin: i64, dmax: i64) -> Vec<Element> {
    points.iter().filter(|k| kinds.iter().any(|t| t.eq_ignore_ascii_case(&k.kind))).map(|k| {
        let r = (k.width.max(k.height) * 0.5).clamp(30.0, 400.0);
        super::places::zone(event, k.x, k.z, r, 0, 0, dmin, dmax)
    }).collect()
}

/// Rounds a coordinate for writing.
pub fn coord(v: f32) -> String {
    fmt_num((v as f64 * 1000.0).round() / 1000.0)
}

/// A guess at the class of a prop for `cfgeventgroups.xml`: houses are `Land_`, wrecks and
/// other props `StaticObj_`, and the variants of dynamic events end in `_DE`.
pub fn prop_class(model: &str, de: bool) -> String {
    let p = crate::objects::ObjectStore::normalize_path(model);
    let file = p.rsplit('\\').next().unwrap_or(&p);
    let stem = file.strip_suffix(".p3d").unwrap_or(file);
    // Class names write the parts of the file name capitalised: wreck_bmp2 -> Wreck_BMP2 is
    // not knowable, so only the first letter of each part is raised.
    let name: Vec<String> = stem.split('_').map(|s| {
        let mut c = s.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    }).collect();
    let house = p.contains("\\structures\\") && !p.contains("\\wrecks\\") && !p.contains("\\furniture\\") && !p.contains("\\signs\\");
    format!("{}{}{}", if house { "Land_" } else { "StaticObj_" }, name.join("_"), if de { "_DE" } else { "" })
}

/// The direction of an object of the editor, from the north towards the east, in degrees.
pub fn heading(o: &crate::objects::Obj) -> f32 {
    let m = crate::objects::rotation(o.yaw, o.pitch, o.roll, 1.0);
    m[2][0].atan2(m[2][2]).to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props_get_class_names() {
        assert_eq!(prop_class("dz\\structures\\wrecks\\vehicles\\wreck_brdm.p3d", true), "StaticObj_Wreck_Brdm_DE");
        assert_eq!(prop_class("DZ\\structures\\residential\\misc\\misc_feedshack.p3d", false), "Land_Misc_Feedshack");
        assert_eq!(prop_class("dz\\structures_bliss\\roadblock_wood_long.p3d", false), "StaticObj_Roadblock_Wood_Long");
        let o = crate::objects::Obj { x: 0.0, z: 0.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        assert!(heading(&o).abs() < 1e-3);
    }

    fn turned(yaw_deg: f32) -> [f32; 12] {
        // Matrix of a model turned by `yaw` from the north towards the east.
        let (s, c) = yaw_deg.to_radians().sin_cos();
        [c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c, 80.26, 113.79, 4422.18]
    }

    #[test]
    fn angles_match_the_game() {
        // The vanilla Land_Misc_FeedShack near 80, 4422: rpy "0 0 -70.013718", a 160.013718.
        let (p, rpy, a) = placement(&turned(-70.013718));
        assert!((rpy[2] + 70.013718).abs() < 1e-3, "{rpy:?}");
        assert!((a - 160.013718).abs() < 1e-3);
        assert!((p[0] - 80.26).abs() < 1e-3);
        let (_, rpy, a) = placement(&turned(-170.0));
        assert!((rpy[2] + 170.0).abs() < 1e-3);
        assert!((a + 100.0).abs() < 1e-3, "{a}");
        assert_eq!(proxy_class("dz\\structures\\residential\\misc\\misc_feedshack.p3d"), "Land_misc_feedshack");
    }

    #[test]
    fn groups_come_from_placed_objects() {
        let mut m = Mission::default();
        m.put("mapgroupproto.xml", FileData::Xml(super::super::xml::XmlDoc::new(Element::new("prototype").with_child(empty_prototype("Land_Misc_FeedShack")))));
        m.put("mapclusterproto.xml", FileData::Xml(super::super::xml::parse_str(r#"<prototype><clusters><export name="AppleTree1" shape="dz\plants\tree\t_malusDomestica_1s.p3d"/></clusters></prototype>"#).unwrap()));
        let objs = vec![
            WrpObject { matrix: turned(10.0), id: 0, model: "dz\\structures\\residential\\misc\\misc_feedshack.p3d".into(), underground: false },
            WrpObject { matrix: turned(0.0), id: 1, model: "dz\\plants\\tree\\t_malusdomestica_1s.p3d".into(), underground: false },
            WrpObject { matrix: turned(0.0), id: 2, model: "dz\\structures\\residential\\houses\\house_new.p3d".into(), underground: false },
        ];
        let (edits, r) = map_groups(&mut m, &objs, true);
        assert_eq!(edits.len(), 2);
        assert_eq!((r.groups, r.clusters), (1, 1));
        assert_eq!(r.without_proto, vec![("Land_house_new".to_string(), 1)]);
        let g = m.xml("mapgrouppos.xml").unwrap().root.child("group").unwrap().clone();
        assert_eq!(g.attr("name"), Some("Land_Misc_FeedShack"));
        assert!(g.attr("rpy").unwrap().ends_with("10.000000"));
        assert_eq!(m.xml("mapgroupcluster.xml").unwrap().root.child("group").unwrap().attr("name"), Some("AppleTree1"));
    }
}
