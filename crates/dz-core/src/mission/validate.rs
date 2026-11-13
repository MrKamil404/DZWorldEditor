//! Checks of the references between the files of a mission: names a file uses that another
//! file should define, files that are named but missing, places outside the map.
use super::xml::Element;
use super::{ce, FileData, Mission};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub level: Level,
    pub file: String,
    /// The message, in English; `{0}`, `{1}` stand for `args`.
    pub text: &'static str,
    pub args: Vec<String>,
}

impl Issue {
    pub fn message(&self) -> String {
        let mut s = self.text.to_string();
        for (i, a) in self.args.iter().enumerate() {
            s = s.replace(&format!("{{{i}}}"), a);
        }
        s
    }
}

struct Out(Vec<Issue>);
impl Out {
    fn add(&mut self, level: Level, file: &str, text: &'static str, args: &[&str]) {
        self.0.push(Issue { level, file: file.to_string(), text, args: args.iter().map(|s| s.to_string()).collect() });
    }
}

fn lower_set(v: Vec<String>) -> HashSet<String> {
    v.into_iter().map(|s| s.to_ascii_lowercase()).collect()
}

/// Every issue found, errors first. `world_size` is the side of the map in metres.
pub fn check(m: &Mission, world_size: Option<f32>) -> Vec<Issue> {
    let mut out = Out(Vec::new());
    let categories = lower_set(m.limits("categories"));
    let tags = lower_set(m.limits("tags"));
    let usages = lower_set(m.limits("usageflags"));
    let values = lower_set(m.limits("valueflags"));
    let user_usage: HashSet<String> = m.user_limits("usageflags").into_iter().map(|(n, _)| n.to_ascii_lowercase()).collect();
    let user_value: HashSet<String> = m.user_limits("valueflags").into_iter().map(|(n, _)| n.to_ascii_lowercase()).collect();
    let have_limits = m.xml("cfglimitsdefinition.xml").is_some();

    // Files every server needs.
    for f in ["init.c", "db/types.xml", "db/globals.xml", "db/economy.xml", "db/events.xml", "cfgeconomycore.xml", "cfglimitsdefinition.xml"] {
        if m.file(f).is_none() {
            out.add(Level::Error, f, "The mission has no {0}", &[f]);
        }
    }
    for (k, f) in &m.files {
        if matches!(f.data, FileData::Disk) && k.to_ascii_lowercase().ends_with(".xml") && !k.to_ascii_lowercase().starts_with("mapgroupcluster") && !super::is_storage(k) {
            out.add(Level::Warning, k, "{0} could not be read as XML; it is copied as it is", &[k]);
        }
    }

    // Files cfgeconomycore.xml names.
    if let Some(core) = m.xml("cfgeconomycore.xml") {
        for c in core.root.children_named("ce") {
            let folder = c.attr("folder").unwrap_or("").trim_matches(['/', '\\']);
            for f in c.children_named("file") {
                let name = f.attr("name").unwrap_or("");
                let rel = if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") };
                if m.file(&rel).is_none() {
                    out.add(Level::Error, "cfgeconomycore.xml", "cfgeconomycore.xml names {0}, which the mission does not have", &[&rel]);
                }
            }
        }
    }

    // Types.
    let mut seen: HashMap<String, String> = HashMap::new();
    for (file, _, t) in ce::all_types(m) {
        let key = t.name.to_ascii_lowercase();
        if let Some(first) = seen.get(&key) {
            out.add(Level::Warning, &file, "Type {0} is defined again (first in {1}); the later one wins", &[&t.name, first]);
        } else {
            seen.insert(key, file.clone());
        }
        if !have_limits {
            continue;
        }
        for c in &t.category {
            if !categories.contains(&c.to_ascii_lowercase()) {
                out.add(Level::Error, &file, "Type {0}: category {1} is not in cfglimitsdefinition.xml", &[&t.name, c]);
            }
        }
        for u in &t.usage {
            if !usages.contains(&u.to_ascii_lowercase()) {
                out.add(Level::Error, &file, "Type {0}: usage {1} is not in cfglimitsdefinition.xml", &[&t.name, u]);
            }
        }
        for v in &t.value {
            if !values.contains(&v.to_ascii_lowercase()) {
                out.add(Level::Error, &file, "Type {0}: value {1} is not in cfglimitsdefinition.xml", &[&t.name, v]);
            }
        }
        for g in &t.tag {
            if !tags.contains(&g.to_ascii_lowercase()) {
                out.add(Level::Error, &file, "Type {0}: tag {1} is not in cfglimitsdefinition.xml", &[&t.name, g]);
            }
        }
        for u in &t.usage_user {
            if !user_usage.contains(&u.to_ascii_lowercase()) {
                out.add(Level::Error, &file, "Type {0}: usage group {1} is not in cfglimitsdefinitionuser.xml", &[&t.name, u]);
            }
        }
        for v in &t.value_user {
            if !user_value.contains(&v.to_ascii_lowercase()) {
                out.add(Level::Error, &file, "Type {0}: value group {1} is not in cfglimitsdefinitionuser.xml", &[&t.name, v]);
            }
        }
        if let (Some(n), Some(mi)) = (t.nominal, t.min) {
            if n > 0 && mi > n {
                out.add(Level::Warning, &file, "Type {0}: min {1} is above nominal {2}", &[&t.name, &mi.to_string(), &n.to_string()]);
            }
        }
        if t.nominal.unwrap_or(0) > 0 && t.usage.is_empty() && t.usage_user.is_empty() && t.category.is_empty() {
            out.add(Level::Info, &file, "Type {0} spawns but has no usage or category, so the economy has nowhere to put it", &[&t.name]);
        }
    }
    let types = ce::type_names(m);
    let known = |n: &str| types.contains(&n.to_ascii_lowercase());

    // User groups of limits.
    for (list, base) in [("usageflags", &usages), ("valueflags", &values)] {
        for (g, members) in m.user_limits(list) {
            for x in members {
                if !base.contains(&x.to_ascii_lowercase()) {
                    out.add(Level::Error, "cfglimitsdefinitionuser.xml", "Group {0} uses {1}, which is not in cfglimitsdefinition.xml", &[&g, &x]);
                }
            }
        }
    }

    // Events.
    let events: Vec<String> = ce::event_names(m);
    let event_set = lower_set(events.clone());
    let groups = lower_set(ce::event_group_names(m));
    let mut spawned_events = HashSet::new();
    let mut used_groups = HashSet::new();
    for f in m.ce_files("eventspawns") {
        let Some(d) = m.xml(&f) else { continue };
        for ev in d.root.children_named("event") {
            let name = ev.attr("name").unwrap_or("");
            spawned_events.insert(name.to_ascii_lowercase());
            if !event_set.contains(&name.to_ascii_lowercase()) {
                out.add(Level::Warning, &f, "Spawns of event {0}, which the event files do not define", &[name]);
            }
            for p in ev.children_named("pos") {
                if let Some(g) = p.attr("group") {
                    used_groups.insert(g.to_ascii_lowercase());
                    if p.attr("y").is_none() {
                        out.add(Level::Warning, &f, "A spawn of event {0} places group {1} but has no height (y)", &[name, g]);
                    }
                    if !groups.contains(&g.to_ascii_lowercase()) {
                        out.add(Level::Error, &f, "Event {0} uses group {1}, which cfgeventgroups.xml does not define", &[name, g]);
                    }
                }
                if let (Some(size), Some(x), Some(z)) = (world_size, p.attr_f32("x"), p.attr_f32("z")) {
                    if !(0.0..=size).contains(&x) || !(0.0..=size).contains(&z) {
                        out.add(Level::Warning, &f, "A spawn of event {0} is outside the map ({1}, {2})", &[name, &format!("{x:.0}"), &format!("{z:.0}")]);
                    }
                }
            }
        }
    }
    for f in m.ce_files("events") {
        let Some(d) = m.xml(&f) else { continue };
        for e in d.root.children_named("event") {
            let r = ce::EventRow::read(e);
            if r.active == Some(true) && r.position.as_deref() == Some("fixed") && r.nominal.unwrap_or(0) > 0 && !spawned_events.contains(&r.name.to_ascii_lowercase()) {
                out.add(Level::Warning, &f, "Event {0} has fixed positions but no spawns in cfgeventspawns.xml", &[&r.name]);
            }
            for c in &r.children {
                if !types.is_empty() && !known(&c.kind) && !c.kind.starts_with("Animal_") && !c.kind.starts_with("Zmb") {
                    out.add(Level::Info, &f, "Event {0}: child {1} is not in the type files", &[&r.name, &c.kind]);
                }
            }
        }
    }
    if let Some(d) = m.xml("cfgeventgroups.xml") {
        for g in d.root.children_named("group") {
            let gname = g.attr("name").unwrap_or("");
            if g.children_named("child").next().is_none() {
                out.add(Level::Warning, "cfgeventgroups.xml", "Group {0} has no objects", &[gname]);
            }
            if !used_groups.contains(&gname.to_ascii_lowercase()) {
                out.add(Level::Info, "cfgeventgroups.xml", "Group {0} is not placed by any event spawn", &[gname]);
            }
            for c in g.children_named("child") {
                let t = c.attr("type").unwrap_or("");
                if !types.is_empty() && !known(t) && !t.starts_with("Land_") && !t.starts_with("StaticObj_") {
                    out.add(Level::Info, "cfgeventgroups.xml", "Group {0}: child {1} is not in the type files", &[g.attr("name").unwrap_or(""), t]);
                }
            }
        }
    }

    // Spawnable types and presets.
    let (cargo, att) = ce::preset_names(m);
    let (cargo, att) = (lower_set(cargo), lower_set(att));
    for f in m.ce_files("spawnabletypes") {
        let Some(d) = m.xml(&f) else { continue };
        for t in d.root.children_named("type") {
            let name = t.attr("name").unwrap_or("");
            if !types.is_empty() && !known(name) {
                out.add(Level::Info, &f, "Spawnable type {0} is not in the type files", &[name]);
            }
            for (tag, set) in [("cargo", &cargo), ("attachments", &att)] {
                for c in t.children_named(tag) {
                    if let Some(p) = c.attr("preset") {
                        if !set.contains(&p.to_ascii_lowercase()) {
                            out.add(Level::Error, &f, "{0}: preset {1} is not in cfgrandompresets.xml", &[name, p]);
                        }
                    }
                    check_items(&mut out, &f, name, c, &known, !types.is_empty());
                }
            }
        }
    }
    for f in m.ce_files("randompresets") {
        let Some(d) = m.xml(&f) else { continue };
        for p in d.root.elements() {
            check_items(&mut out, &f, p.attr("name").unwrap_or(""), p, &known, !types.is_empty());
        }
    }
    if let Some(d) = m.xml("cfgignorelist.xml") {
        for t in d.root.children_named("type") {
            let n = t.attr("name").unwrap_or("");
            if !types.is_empty() && !known(n) {
                out.add(Level::Info, "cfgignorelist.xml", "Ignored type {0} is not in the type files", &[n]);
            }
        }
    }

    // Territories.
    if let Some(env) = m.xml("cfgenvironment.xml") {
        let files: HashSet<String> = m.files.keys().filter(|k| k.to_ascii_lowercase().starts_with("env/")).map(|k| k.rsplit('/').next().unwrap().trim_end_matches(".xml").to_ascii_lowercase()).collect();
        if let Some(t) = env.root.child("territories") {
            for f in t.children_named("file") {
                let p = f.attr("path").unwrap_or("");
                if m.file(p).is_none() {
                    out.add(Level::Error, "cfgenvironment.xml", "cfgenvironment.xml names {0}, which the mission does not have", &[p]);
                }
            }
            for tt in t.children_named("territory") {
                for f in tt.children_named("file") {
                    let u = f.attr("usable").unwrap_or("");
                    if !files.contains(&u.to_ascii_lowercase()) {
                        out.add(Level::Error, "cfgenvironment.xml", "Territory {0} uses {1}, which is not a file in env", &[tt.attr("name").unwrap_or(""), u]);
                    }
                }
            }
        }
    }
    for f in m.territory_files() {
        let Some(d) = m.xml(&f) else { continue };
        let infected = f.to_ascii_lowercase().contains("zombie");
        for t in d.root.children_named("territory") {
            for z in t.children_named("zone") {
                let name = z.attr("name").unwrap_or("");
                if infected && !event_set.is_empty() && !event_set.contains(&name.to_ascii_lowercase()) {
                    out.add(Level::Warning, &f, "Zone {0} names no event of the event files", &[name]);
                }
                if let (Some(size), Some(x), Some(zz)) = (world_size, z.attr_f32("x"), z.attr_f32("z")) {
                    if !(0.0..=size).contains(&x) || !(0.0..=size).contains(&zz) {
                        out.add(Level::Warning, &f, "Zone {0} is outside the map ({1}, {2})", &[name, &format!("{x:.0}"), &format!("{zz:.0}")]);
                    }
                }
            }
        }
    }

    // Map groups.
    let protos = super::gen::proto_classes(m);
    if let Some(d) = m.xml("mapgroupproto.xml") {
        for g in d.root.children_named("group") {
            let name = g.attr("name").unwrap_or("");
            for u in g.children_named("usage").filter_map(|u| u.attr("name")) {
                if have_limits && !usages.contains(&u.to_ascii_lowercase()) {
                    out.add(Level::Error, "mapgroupproto.xml", "Prototype {0}: usage {1} is not in cfglimitsdefinition.xml", &[name, u]);
                }
            }
            for v in g.children_named("value").filter_map(|u| u.attr("name")) {
                if have_limits && !values.contains(&v.to_ascii_lowercase()) {
                    out.add(Level::Error, "mapgroupproto.xml", "Prototype {0}: value {1} is not in cfglimitsdefinition.xml", &[name, v]);
                }
            }
            for c in g.children_named("container") {
                for x in c.children_named("category").filter_map(|u| u.attr("name")) {
                    if have_limits && !categories.contains(&x.to_ascii_lowercase()) {
                        out.add(Level::Error, "mapgroupproto.xml", "Prototype {0}: category {1} is not in cfglimitsdefinition.xml", &[name, x]);
                    }
                }
                for x in c.children_named("tag").filter_map(|u| u.attr("name")) {
                    if have_limits && !tags.contains(&x.to_ascii_lowercase()) {
                        out.add(Level::Error, "mapgroupproto.xml", "Prototype {0}: tag {1} is not in cfglimitsdefinition.xml", &[name, x]);
                    }
                }
            }
        }
    }
    if let Some(d) = m.xml("mapgrouppos.xml") {
        let mut missing: HashMap<String, usize> = HashMap::new();
        for g in d.root.children_named("group") {
            let n = g.attr("name").unwrap_or("");
            if !protos.contains_key(&n.to_ascii_lowercase()) {
                *missing.entry(n.to_string()).or_default() += 1;
            }
        }
        let mut missing: Vec<_> = missing.into_iter().collect();
        missing.sort();
        for (n, c) in missing {
            out.add(Level::Info, "mapgrouppos.xml", "{0} buildings of class {1} have no prototype in mapgroupproto.xml", &[&c.to_string(), &n]);
        }
    }

    // Player spawns.
    if let Some(d) = m.xml("cfgplayerspawnpoints.xml") {
        for sec in ["fresh", "hop", "travel"] {
            match d.root.child(sec) {
                None => out.add(Level::Warning, "cfgplayerspawnpoints.xml", "Section {0} is missing", &[sec]),
                Some(s) => {
                    let n = s.child("generator_posbubbles").map(|b| b.children_named("pos").count() + b.children_named("group").map(|g| g.children_named("pos").count()).sum::<usize>()).unwrap_or(0);
                    if n == 0 {
                        out.add(Level::Error, "cfgplayerspawnpoints.xml", "Section {0} has no spawn positions", &[sec]);
                    }
                }
            }
        }
    } else {
        out.add(Level::Error, "cfgplayerspawnpoints.xml", "The mission has no {0}", &["cfgplayerspawnpoints.xml"]);
    }

    // Area flags.
    if let Some(a) = m.areaflags() {
        if let Some(size) = world_size {
            if (a.world_width as f32 - size).abs() > 1.0 || (a.world_height as f32 - size).abs() > 1.0 {
                out.add(Level::Warning, "areaflags.map", "areaflags.map covers {0} m, the map is {1} m", &[&a.world_width.to_string(), &format!("{size:.0}")]);
            }
        }
        let u = m.limits("usageflags").len();
        let v = m.limits("valueflags").len();
        if u > a.usage_bits as usize {
            out.add(Level::Error, "areaflags.map", "areaflags.map has room for {0} usage flags, cfglimitsdefinition.xml has {1}", &[&a.usage_bits.to_string(), &u.to_string()]);
        }
        if v > a.value_bits as usize {
            out.add(Level::Error, "areaflags.map", "areaflags.map has room for {0} value flags, cfglimitsdefinition.xml has {1}", &[&a.value_bits.to_string(), &v.to_string()]);
        }
    } else if m.file("areaflags.map").is_some() {
        out.add(Level::Error, "areaflags.map", "areaflags.map could not be read", &[]);
    }

    // Effect areas.
    if let Some(d) = m.json("cfgeffectarea.json") {
        if let Some(a) = d.value.get("Areas").and_then(|a| a.as_array()) {
            for (i, area) in a.iter().enumerate() {
                if super::json::vec3(area.get("Data").and_then(|d| d.get("Pos"))).is_none() {
                    out.add(Level::Error, "cfgeffectarea.json", "Area {0} has no position", &[&(i + 1).to_string()]);
                }
            }
        }
    }
    let mut v = out.0;
    v.sort_by(|a, b| a.level.cmp(&b.level).then(a.file.cmp(&b.file)));
    v
}

fn check_items(out: &mut Out, file: &str, owner: &str, e: &Element, known: &dyn Fn(&str) -> bool, have_types: bool) {
    for it in e.children_named("item") {
        let n = it.attr("name").unwrap_or("");
        if have_types && !known(n) {
            out.add(Level::Info, file, "{0}: item {1} is not in the type files", &[owner, n]);
        }
        for sub in it.elements() {
            check_items(out, file, owner, sub, known, have_types);
        }
    }
}
