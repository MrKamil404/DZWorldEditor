//! Typed views of the Central Economy files. They read from the trees and write back into
//! them, so whatever a view does not know stays where it is.
use super::xml::Element;
use super::Mission;

/// The order of the children of a `<type>` in `types.xml`.
pub const TYPE_ORDER: [&str; 12] = ["nominal", "lifetime", "restock", "min", "quantmin", "quantmax", "cost", "flags", "category", "usage", "value", "tag"];
/// The attributes of `<flags>` of a type.
pub const TYPE_FLAGS: [&str; 6] = ["count_in_cargo", "count_in_hoarder", "count_in_map", "count_in_player", "crafted", "deloot"];

/// One `<type>` of `types.xml`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TypeRow {
    pub name: String,
    pub nominal: Option<i64>,
    pub lifetime: Option<i64>,
    pub restock: Option<i64>,
    pub min: Option<i64>,
    pub quantmin: Option<i64>,
    pub quantmax: Option<i64>,
    pub cost: Option<i64>,
    /// `None` when the type has no `<flags>` or the flag is missing.
    pub flags: [Option<bool>; 6],
    pub category: Vec<String>,
    pub usage: Vec<String>,
    pub value: Vec<String>,
    pub tag: Vec<String>,
    /// Usage and value groups of `cfglimitsdefinitionuser.xml` (`<usage user="..."/>`).
    pub usage_user: Vec<String>,
    pub value_user: Vec<String>,
}

fn flag(e: &Element, k: &str) -> Option<bool> {
    e.attr(k).map(|v| matches!(v.trim(), "1" | "true" | "yes"))
}

impl TypeRow {
    pub fn read(e: &Element) -> Self {
        let f = e.child("flags");
        Self {
            name: e.attr("name").unwrap_or("").to_string(),
            nominal: e.child_i64("nominal"),
            lifetime: e.child_i64("lifetime"),
            restock: e.child_i64("restock"),
            min: e.child_i64("min"),
            quantmin: e.child_i64("quantmin"),
            quantmax: e.child_i64("quantmax"),
            cost: e.child_i64("cost"),
            flags: std::array::from_fn(|i| f.and_then(|f| flag(f, TYPE_FLAGS[i]))),
            category: e.child_attrs("category", "name"),
            usage: e.child_attrs("usage", "name"),
            value: e.child_attrs("value", "name"),
            tag: e.child_attrs("tag", "name"),
            usage_user: e.child_attrs("usage", "user"),
            value_user: e.child_attrs("value", "user"),
        }
    }
    /// Writes the row into the element, changing only what differs.
    pub fn write(&self, e: &mut Element) {
        if e.attr("name") != Some(self.name.as_str()) {
            e.set_attr("name", self.name.clone());
        }
        let nums = [("nominal", self.nominal), ("lifetime", self.lifetime), ("restock", self.restock), ("min", self.min), ("quantmin", self.quantmin), ("quantmax", self.quantmax), ("cost", self.cost)];
        for (k, v) in nums {
            match v {
                Some(v) => e.set_child_num(k, v as f64, &TYPE_ORDER),
                None => e.remove_children(k),
            }
        }
        if self.flags.iter().any(Option::is_some) || e.child("flags").is_some() {
            let f = e.ensure_child_ordered("flags", &TYPE_ORDER);
            for (i, k) in TYPE_FLAGS.iter().enumerate() {
                match self.flags[i] {
                    Some(on) => {
                        if flag(f, k) != Some(on) {
                            f.set_attr(k, if on { "1" } else { "0" });
                        }
                    }
                    None => f.remove_attr(k),
                }
            }
        }
        e.set_child_attrs("category", "name", &self.category, &TYPE_ORDER);
        e.set_child_attrs("usage", "name", &self.usage, &TYPE_ORDER);
        e.set_child_attrs("usage", "user", &self.usage_user, &TYPE_ORDER);
        e.set_child_attrs("value", "name", &self.value, &TYPE_ORDER);
        e.set_child_attrs("value", "user", &self.value_user, &TYPE_ORDER);
        e.set_child_attrs("tag", "name", &self.tag, &TYPE_ORDER);
    }
    /// A new type as the game expects one.
    pub fn new_element(name: &str) -> Element {
        let mut e = Element::new("type").with_attr("name", name);
        let row = TypeRow { name: name.into(), nominal: Some(0), lifetime: Some(3600), restock: Some(0), min: Some(0), quantmin: Some(-1), quantmax: Some(-1), cost: Some(100), flags: [Some(true), Some(false), Some(false), Some(false), Some(false), Some(false)], ..Default::default() };
        row.write(&mut e);
        e
    }
}

/// The order of the children of an `<event>` in `events.xml`.
pub const EVENT_ORDER: [&str; 15] = ["nominal", "min", "max", "lifetime", "restock", "saferadius", "distanceradius", "cleanupradius", "secondary", "flags", "position", "limit", "active", "children", "zone"];
pub const EVENT_FLAGS: [&str; 3] = ["deletable", "init_random", "remove_damaged"];

/// One `<child>` of an event.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EventChild {
    pub kind: String,
    pub min: i64,
    pub max: i64,
    pub lootmin: i64,
    pub lootmax: i64,
}

/// One `<event>` of `events.xml`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EventRow {
    pub name: String,
    pub nominal: Option<i64>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub lifetime: Option<i64>,
    pub restock: Option<i64>,
    pub saferadius: Option<i64>,
    pub distanceradius: Option<i64>,
    pub cleanupradius: Option<i64>,
    pub secondary: Option<String>,
    pub flags: [Option<bool>; 3],
    /// `fixed`, `player` or `uniform`.
    pub position: Option<String>,
    /// `child`, `mixed`, `custom` or `parent`.
    pub limit: Option<String>,
    pub active: Option<bool>,
    pub children: Vec<EventChild>,
}

impl EventRow {
    pub fn read(e: &Element) -> Self {
        let f = e.child("flags");
        Self {
            name: e.attr("name").unwrap_or("").to_string(),
            nominal: e.child_i64("nominal"),
            min: e.child_i64("min"),
            max: e.child_i64("max"),
            lifetime: e.child_i64("lifetime"),
            restock: e.child_i64("restock"),
            saferadius: e.child_i64("saferadius"),
            distanceradius: e.child_i64("distanceradius"),
            cleanupradius: e.child_i64("cleanupradius"),
            secondary: e.child_text("secondary").map(|s| s.trim().to_string()),
            flags: std::array::from_fn(|i| f.and_then(|f| flag(f, EVENT_FLAGS[i]))),
            position: e.child_text("position").map(|s| s.trim().to_string()),
            limit: e.child_text("limit").map(|s| s.trim().to_string()),
            active: e.child_i64("active").map(|v| v != 0),
            children: e.child("children").map(|c| c.children_named("child").map(|c| EventChild {
                kind: c.attr("type").unwrap_or("").to_string(),
                min: c.attr_i64("min").unwrap_or(0),
                max: c.attr_i64("max").unwrap_or(0),
                lootmin: c.attr_i64("lootmin").unwrap_or(0),
                lootmax: c.attr_i64("lootmax").unwrap_or(0),
            }).collect()).unwrap_or_default(),
        }
    }
    pub fn write(&self, e: &mut Element) {
        if e.attr("name") != Some(self.name.as_str()) {
            e.set_attr("name", self.name.clone());
        }
        let nums = [("nominal", self.nominal), ("min", self.min), ("max", self.max), ("lifetime", self.lifetime), ("restock", self.restock), ("saferadius", self.saferadius), ("distanceradius", self.distanceradius), ("cleanupradius", self.cleanupradius)];
        for (k, v) in nums {
            match v {
                Some(v) => e.set_child_num(k, v as f64, &EVENT_ORDER),
                None => e.remove_children(k),
            }
        }
        for (k, v) in [("secondary", &self.secondary), ("position", &self.position), ("limit", &self.limit)] {
            match v {
                Some(v) => {
                    if e.child_text(k).map(|t| t.trim().to_string()).as_deref() != Some(v.as_str()) {
                        e.set_child_text(k, v, &EVENT_ORDER);
                    }
                }
                None => e.remove_children(k),
            }
        }
        match self.active {
            Some(a) => e.set_child_num("active", a as i64 as f64, &EVENT_ORDER),
            None => e.remove_children("active"),
        }
        if self.flags.iter().any(Option::is_some) || e.child("flags").is_some() {
            let f = e.ensure_child_ordered("flags", &EVENT_ORDER);
            for (i, k) in EVENT_FLAGS.iter().enumerate() {
                match self.flags[i] {
                    Some(on) => {
                        if flag(f, k) != Some(on) {
                            f.set_attr(k, if on { "1" } else { "0" });
                        }
                    }
                    None => f.remove_attr(k),
                }
            }
        }
        if !self.children.is_empty() || e.child("children").is_some() {
            let c = e.ensure_child_ordered("children", &EVENT_ORDER);
            let mut n = 0;
            for el in c.children_named_mut("child") {
                if let Some(ch) = self.children.get(n) {
                    if el.attr("type") != Some(ch.kind.as_str()) {
                        el.set_attr("type", ch.kind.clone());
                    }
                    for (k, v) in [("min", ch.min), ("max", ch.max), ("lootmin", ch.lootmin), ("lootmax", ch.lootmax)] {
                        el.set_attr_num(k, v as f64);
                    }
                }
                n += 1;
            }
            if n > self.children.len() {
                let mut seen = 0;
                let keep = self.children.len();
                c.retain_elements(|el| {
                    if el.name != "child" {
                        return true;
                    }
                    seen += 1;
                    seen <= keep
                });
            }
            for ch in self.children.iter().skip(n) {
                c.push(Element::new("child").with_attr("lootmax", ch.lootmax.to_string()).with_attr("lootmin", ch.lootmin.to_string()).with_attr("max", ch.max.to_string()).with_attr("min", ch.min.to_string()).with_attr("type", ch.kind.clone()));
            }
        }
    }
    pub fn new_element(name: &str) -> Element {
        let mut e = Element::new("event").with_attr("name", name);
        let row = EventRow {
            name: name.into(), nominal: Some(1), min: Some(1), max: Some(1), lifetime: Some(3600), restock: Some(0), saferadius: Some(500), distanceradius: Some(500), cleanupradius: Some(200),
            flags: [Some(false), Some(false), Some(false)], position: Some("fixed".into()), limit: Some("child".into()), active: Some(true), secondary: None, children: Vec::new(),
        };
        row.write(&mut e);
        e
    }
}

/// Every `<type>` of the type files: (file, index of the node in the root, row).
pub fn all_types(m: &Mission) -> Vec<(String, usize, TypeRow)> {
    let mut out = Vec::new();
    for f in m.ce_files("types") {
        if let Some(d) = m.xml(&f) {
            for (i, n) in d.root.children.iter().enumerate() {
                if let super::xml::Node::Element(e) = n {
                    if e.name == "type" {
                        out.push((f.clone(), i, TypeRow::read(e)));
                    }
                }
            }
        }
    }
    out
}

/// Names of all types of the type files.
pub fn type_names(m: &Mission) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for f in m.ce_files("types") {
        if let Some(d) = m.xml(&f) {
            for e in d.root.children_named("type") {
                if let Some(n) = e.attr("name") {
                    out.insert(n.to_ascii_lowercase());
                }
            }
        }
    }
    out
}

/// Names of the events of the event files.
pub fn event_names(m: &Mission) -> Vec<String> {
    let mut out = Vec::new();
    for f in m.ce_files("events") {
        if let Some(d) = m.xml(&f) {
            out.extend(d.root.children_named("event").filter_map(|e| e.attr("name").map(str::to_string)));
        }
    }
    out
}

/// Names of the presets of `cfgrandompresets.xml`: (cargo, attachments).
pub fn preset_names(m: &Mission) -> (Vec<String>, Vec<String>) {
    let mut cargo = Vec::new();
    let mut att = Vec::new();
    for f in m.ce_files("randompresets") {
        if let Some(d) = m.xml(&f) {
            for e in d.root.elements() {
                let Some(n) = e.attr("name") else { continue };
                match e.name.as_str() {
                    "cargo" => cargo.push(n.to_string()),
                    "attachments" => att.push(n.to_string()),
                    _ => {}
                }
            }
        }
    }
    (cargo, att)
}

/// Group names of `cfgeventgroups.xml`.
pub fn event_group_names(m: &Mission) -> Vec<String> {
    m.xml("cfgeventgroups.xml").map(|d| d.root.children_named("group").filter_map(|g| g.attr("name").map(str::to_string)).collect()).unwrap_or_default()
}

/// What a bulk edit does to the selected types.
#[derive(Clone, Debug, PartialEq)]
pub enum BulkOp {
    /// Multiply a number field (`nominal`, `min`, ...) and round.
    Scale(&'static str, f64),
    /// Set a number field.
    Set(&'static str, i64),
    /// Add or remove a name in a list (`category`, `usage`, `value`, `tag`).
    Add(&'static str, String),
    Remove(&'static str, String),
    /// Set a flag of `<flags>`.
    Flag(usize, bool),
}

impl BulkOp {
    pub fn apply(&self, r: &mut TypeRow) {
        fn field<'a>(r: &'a mut TypeRow, k: &str) -> Option<&'a mut Option<i64>> {
            Some(match k {
                "nominal" => &mut r.nominal,
                "min" => &mut r.min,
                "lifetime" => &mut r.lifetime,
                "restock" => &mut r.restock,
                "quantmin" => &mut r.quantmin,
                "quantmax" => &mut r.quantmax,
                "cost" => &mut r.cost,
                _ => return None,
            })
        }
        fn list<'a>(r: &'a mut TypeRow, k: &str) -> Option<&'a mut Vec<String>> {
            Some(match k {
                "category" => &mut r.category,
                "usage" => &mut r.usage,
                "value" => &mut r.value,
                "tag" => &mut r.tag,
                _ => return None,
            })
        }
        match self {
            BulkOp::Scale(k, f) => {
                if let Some(Some(v)) = field(r, k) {
                    if *v >= 0 {
                        *v = ((*v as f64) * f).round().max(0.0) as i64;
                    }
                }
                // Keep min under nominal.
                if *k == "nominal" || *k == "min" {
                    if let (Some(n), Some(m)) = (r.nominal, r.min.as_mut()) {
                        *m = (*m).min(n);
                    }
                }
            }
            BulkOp::Set(k, v) => {
                if let Some(slot) = field(r, k) {
                    *slot = Some(*v);
                }
            }
            BulkOp::Add(k, n) => {
                if let Some(l) = list(r, k) {
                    if !l.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                        if *k == "category" {
                            l.clear();
                        }
                        l.push(n.clone());
                    }
                }
            }
            BulkOp::Remove(k, n) => {
                if let Some(l) = list(r, k) {
                    l.retain(|x| !x.eq_ignore_ascii_case(n));
                }
            }
            BulkOp::Flag(i, on) => {
                if let Some(f) = r.flags.get_mut(*i) {
                    *f = Some(*on);
                }
            }
        }
    }
}

/// `true`, `1` and `yes` are true for the game.
pub fn is_true(s: &str) -> bool {
    matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

/// The variables of `globals.xml`: (name, type, value). Type 0 is a whole number, 1 a fraction.
pub fn globals(m: &Mission) -> Vec<(String, String, String)> {
    m.xml("db/globals.xml").map(|d| d.root.children_named("var").map(|v| (v.attr("name").unwrap_or("").to_string(), v.attr("type").unwrap_or("0").to_string(), v.attr("value").unwrap_or("").to_string())).collect()).unwrap_or_default()
}

/// What the variables of `globals.xml` do, for the editor.
pub const GLOBALS_HELP: [(&str, &str); 30] = [
    ("AnimalMaxCount", "Most animals alive at once."),
    ("CleanupAvoidance", "Distance in metres from a player within which nothing is cleaned up."),
    ("CleanupLifetimeDeadAnimal", "Seconds a dead animal stays."),
    ("CleanupLifetimeDeadInfected", "Seconds a dead infected stays."),
    ("CleanupLifetimeDeadPlayer", "Seconds a dead player stays."),
    ("CleanupLifetimeDefault", "Seconds other things stay when their type gives no lifetime."),
    ("CleanupLifetimeLimit", "Most things cleaned up in one pass."),
    ("CleanupLifetimeRuined", "Seconds a ruined item stays."),
    ("FlagRefreshFrequency", "Seconds between the refreshes of a territory flag."),
    ("FlagRefreshMaxDuration", "Longest time in seconds a territory flag keeps the base alive."),
    ("FoodDecay", "1 lets food go bad."),
    ("IdleModeCountdown", "Seconds without players before the economy goes idle."),
    ("IdleModeStartup", "1 starts the server idle."),
    ("InitialSpawn", "Percent of the nominal spawned when the server starts."),
    ("LootDamageMax", "Most damage of spawned loot, 0..1."),
    ("LootDamageMin", "Least damage of spawned loot, 0..1."),
    ("LootProxyPlacement", "1 puts loot on the proxies of buildings."),
    ("LootSpawnAvoidance", "Distance in metres from a player within which no loot spawns."),
    ("RespawnAttempt", "Attempts per type and pass to respawn loot."),
    ("RespawnLimit", "Most items of one type respawned in a pass."),
    ("RespawnTypes", "Most types respawned in a pass."),
    ("RestartSpawn", "Percent of the loot spawned again after a restart."),
    ("SpawnInitial", "Items spawned per pass when the server starts."),
    ("TimeHopping", "Seconds of penalty for changing servers."),
    ("TimeLogin", "Seconds of the login countdown."),
    ("TimeLogout", "Seconds of the logout countdown."),
    ("TimePenalty", "Extra seconds of logout after a fight."),
    ("WorldWetTempUpdate", "1 updates the wetness and temperature of the world."),
    ("ZombieMaxCount", "Most infected alive at once."),
    ("ZoneSpawnDist", "Distance in metres from a player at which infected zones spawn."),
];

/// The sections of `economy.xml` and what they hold.
pub const ECONOMY_SECTIONS: [(&str, &str); 8] = [
    ("dynamic", "Loot of types.xml"),
    ("animals", "Animals"),
    ("zombies", "Infected"),
    ("vehicles", "Vehicles"),
    ("randoms", "Random events"),
    ("custom", "Custom spawns"),
    ("building", "Base building"),
    ("player", "Players"),
];

/// Makes `<folder>/<name>` with an empty `<root>` and adds it to `cfgeconomycore.xml` as a
/// file of `kind` (`types`, `events`, `spawnabletypes`, ...). Hands back the undo.
pub fn add_ce_file(m: &mut Mission, folder: &str, name: &str, kind: &str, root: &str) -> Vec<super::MissionEdit> {
    let rel = format!("{folder}/{name}");
    let mut edits = Vec::new();
    if m.file(&rel).is_none() {
        edits.push(super::file_edit(m, &rel.clone(), |m| m.put(&rel, super::FileData::Xml(super::xml::XmlDoc::new(Element::new(root))))));
    }
    let core = m.key("cfgeconomycore.xml").unwrap_or_else(|| "cfgeconomycore.xml".into());
    let (folder, name, kind) = (folder.to_string(), name.to_string(), kind.to_string());
    edits.push(super::file_edit(m, &core.clone(), |m| {
        let d = m.ensure_xml(&core, "economycore");
        let at = d.root.children.iter().position(|n| matches!(n, super::xml::Node::Element(e) if e.name == "ce" && e.attr("folder") == Some(folder.as_str())));
        let ce = match at {
            Some(i) => match &mut d.root.children[i] {
                super::xml::Node::Element(e) => e,
                _ => unreachable!(),
            },
            None => {
                d.root.push(Element::new("ce").with_attr("folder", folder.clone()));
                d.root.elements_mut().last().unwrap()
            }
        };
        if !ce.children_named("file").any(|f| f.attr("name") == Some(name.as_str())) {
            ce.push(Element::new("file").with_attr("name", name.clone()).with_attr("type", kind.clone()));
        }
    }));
    edits
}

/// Makes `env/<name>.xml` and adds it to `cfgenvironment.xml`. Hands back the undo.
pub fn add_territory_file(m: &mut Mission, name: &str) -> Vec<super::MissionEdit> {
    let rel = format!("env/{name}.xml");
    let mut edits = Vec::new();
    if m.file(&rel).is_none() {
        edits.push(super::file_edit(m, &rel.clone(), |m| m.put(&rel, super::FileData::Xml(super::xml::XmlDoc::new(Element::new("territory-type").with_child(Element::new("territory").with_attr("color", "4291543295")))))));
    }
    let env = m.key("cfgenvironment.xml").unwrap_or_else(|| "cfgenvironment.xml".into());
    edits.push(super::file_edit(m, &env.clone(), |m| {
        let d = m.ensure_xml(&env, "env");
        let t = d.root.ensure_child("territories");
        if !t.children_named("file").any(|f| f.attr("path").is_some_and(|p| p.eq_ignore_ascii_case(&rel))) {
            let at = t.children.iter().rposition(|n| matches!(n, super::xml::Node::Element(e) if e.name == "file")).map(|i| i + 1).unwrap_or(0);
            t.children.insert(at, super::xml::Node::Element(Element::new("file").with_attr("path", rel.clone())));
        }
    }));
    edits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mission::xml;

    #[test]
    fn new_ce_and_territory_files_are_registered() {
        let mut m = crate::mission::templates::new_mission(&crate::world::WorldMeta::new("T", 256, 4.0));
        add_ce_file(&mut m, "custom", "types_mod.xml", "types", "types");
        assert!(m.ce_files("types").contains(&"custom/types_mod.xml".to_string()));
        add_territory_file(&mut m, "wolf_territories");
        assert!(m.territory_files().contains(&"env/wolf_territories.xml".to_string()));
        assert!(crate::mission::validate::check(&m, None).iter().all(|i| i.level != crate::mission::validate::Level::Error));
    }

    #[test]
    fn type_rows_change_only_what_differs() {
        let src = r#"<types><type name="ACOGOptic"><nominal>10</nominal><lifetime>14400</lifetime><restock>1800</restock><min>6</min><quantmin>-1</quantmin><quantmax>-1</quantmax><cost>100</cost><flags count_in_cargo="0" count_in_hoarder="0" count_in_map="1" count_in_player="0" crafted="0" deloot="0"/><category name="weapons"/><usage name="Military"/><usage user="TownVillage"/><value name="Tier3"/><extra foo="1"/></type></types>"#;
        let mut d = xml::parse(src.as_bytes()).unwrap();
        let e = d.root.child_mut("type").unwrap();
        let before = e.clone();
        let mut r = TypeRow::read(e);
        assert_eq!(r.nominal, Some(10));
        assert_eq!(r.usage, vec!["Military"]);
        assert_eq!(r.usage_user, vec!["TownVillage"]);
        assert_eq!(r.flags[2], Some(true));
        r.write(e);
        assert_eq!(*e, before, "writing an unchanged row changes nothing");
        r.nominal = Some(20);
        r.usage.push("Police".into());
        r.tag.push("shelves".into());
        r.flags[5] = Some(true);
        r.write(e);
        let again = TypeRow::read(e);
        assert_eq!(again, r);
        assert!(e.child("extra").is_some(), "unknown elements stay");
        let names: Vec<_> = e.elements().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["nominal", "lifetime", "restock", "min", "quantmin", "quantmax", "cost", "flags", "category", "usage", "usage", "usage", "value", "tag", "extra"]);
        BulkOp::Scale("nominal", 0.5).apply(&mut r);
        assert_eq!(r.nominal, Some(10));
        assert_eq!(r.min, Some(6));
        BulkOp::Scale("nominal", 0.2).apply(&mut r);
        assert_eq!(r.min, Some(2));
    }

    #[test]
    fn event_rows_round_trip() {
        let mut e = EventRow::new_element("VehicleTest");
        let mut r = EventRow::read(&e);
        assert_eq!(r.position.as_deref(), Some("fixed"));
        r.children.push(EventChild { kind: "OffroadHatchback".into(), min: 1, max: 1, lootmin: 0, lootmax: 0 });
        r.children.push(EventChild { kind: "Sedan_02".into(), min: 0, max: 2, lootmin: 0, lootmax: 0 });
        r.write(&mut e);
        assert_eq!(EventRow::read(&e), r);
        r.children.remove(0);
        r.write(&mut e);
        assert_eq!(EventRow::read(&e), r);
        let names: Vec<_> = e.elements().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["nominal", "min", "max", "lifetime", "restock", "saferadius", "distanceradius", "cleanupradius", "flags", "position", "limit", "active", "children"]);
    }
}
