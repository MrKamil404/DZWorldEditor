//! Surfaces of the game and the map's own ones.
//!
//! How the game finds what a piece of ground is:
//! - `CfgSurfaces` classes name the textures they stand for (`files = "en_grass1*"`, matched
//!   against the file name of the layer's colour texture) and give the physics, the sounds of
//!   hits (`soundHit`, `impact`), vehicles (`vpSurface`) and a `character`;
//! - `CfgSurfaceCharacters` turns the character into the clutter (grass) and its odds;
//! - `CfgWorlds >> CAWorld >> Clutter` gives the model of each clutter;
//! - footsteps come from `CfgSoundTables >> CfgStepSoundTables`: every lookup table (human
//!   walk, run, boots, animals...) has an entry per surface class name, so a new surface makes
//!   no sound of its own until entries are added for it.
//!
//! A map surface either changes a surface of the game in place or is a new class inheriting
//! one, with the footsteps of a surface of the game copied to its name.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

// ---------------------------------------------------------------- config text

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    List(Vec<String>),
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub name: String,
    pub parent: Option<String>,
    pub values: Vec<(String, Value)>,
    pub classes: Vec<Node>,
}

impl Node {
    pub fn class(&self, name: &str) -> Option<&Node> {
        self.classes.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }
    pub fn text(&self, key: &str) -> Option<&str> {
        self.values.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).and_then(|(_, v)| match v {
            Value::Text(t) => Some(t.as_str()),
            Value::List(_) => None,
        })
    }
    pub fn list(&self, key: &str) -> Vec<String> {
        self.values.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map_or(Vec::new(), |(_, v)| match v {
            Value::List(l) => l.clone(),
            Value::Text(t) => vec![t.clone()],
        })
    }
}

struct Lexer<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Lexer<'a> {
    fn skip(&mut self) {
        loop {
            while self.p < self.b.len() && self.b[self.p].is_ascii_whitespace() {
                self.p += 1;
            }
            if self.b[self.p..].starts_with(b"//") || (self.p < self.b.len() && self.b[self.p] == b'#') {
                while self.p < self.b.len() && self.b[self.p] != b'\n' {
                    self.p += 1;
                }
            } else if self.b[self.p..].starts_with(b"/*") {
                self.p = self.b[self.p + 2..].windows(2).position(|w| w == b"*/").map_or(self.b.len(), |i| self.p + 2 + i + 2);
            } else {
                return;
            }
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.b.get(self.p).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.p += 1;
            true
        } else {
            false
        }
    }
    fn word(&mut self) -> String {
        self.skip();
        let s = self.p;
        while self.p < self.b.len() && (self.b[self.p].is_ascii_alphanumeric() || self.b[self.p] == b'_') {
            self.p += 1;
        }
        String::from_utf8_lossy(&self.b[s..self.p]).into_owned()
    }
    /// A quoted string, or bare text up to `,`, `;` or `}`.
    fn scalar(&mut self) -> String {
        self.skip();
        if self.b.get(self.p) == Some(&b'"') {
            self.p += 1;
            let mut out = Vec::new();
            while self.p < self.b.len() {
                let c = self.b[self.p];
                self.p += 1;
                if c == b'"' {
                    if self.b.get(self.p) == Some(&b'"') {
                        out.push(b'"');
                        self.p += 1;
                        continue;
                    }
                    break;
                }
                out.push(c);
            }
            return String::from_utf8_lossy(&out).into_owned();
        }
        let s = self.p;
        while self.p < self.b.len() && !matches!(self.b[self.p], b',' | b';' | b'}' | b'\n') {
            self.p += 1;
        }
        String::from_utf8_lossy(&self.b[s..self.p]).trim().to_string()
    }
    fn array(&mut self, out: &mut Vec<String>) {
        if !self.eat(b'{') {
            out.push(self.scalar());
            return;
        }
        loop {
            match self.peek() {
                None => return,
                Some(b'}') => {
                    self.p += 1;
                    return;
                }
                Some(b',') => self.p += 1,
                Some(b'{') => self.array(out),
                Some(_) => {
                    let before = self.p;
                    let v = self.scalar();
                    if self.p == before {
                        self.p += 1;
                    } else {
                        out.push(v);
                    }
                }
            }
        }
    }
    fn body(&mut self, node: &mut Node) {
        loop {
            match self.peek() {
                None => return,
                Some(b'}') => {
                    self.p += 1;
                    self.eat(b';');
                    return;
                }
                Some(b';') => self.p += 1,
                _ => {}
            }
            if self.peek().is_none_or(|c| c == b'}') {
                continue;
            }
            let before = self.p;
            let word = self.word();
            if word.is_empty() {
                // Something this reader does not know: skip a character.
                self.p = (before + 1).max(self.p);
                continue;
            }
            if word == "class" {
                let name = self.word();
                let parent = if self.eat(b':') { Some(self.word()) } else { None };
                let mut child = Node { name, parent, ..Default::default() };
                if self.eat(b'{') {
                    self.body(&mut child);
                    node.classes.push(child);
                } else {
                    self.eat(b';');
                }
                continue;
            }
            if word == "delete" || word == "import" {
                self.word();
                self.eat(b';');
                continue;
            }
            let list = self.eat(b'[');
            if list {
                self.eat(b']');
            }
            let add = self.eat(b'+');
            self.eat(b'=');
            if list {
                let mut v = Vec::new();
                self.array(&mut v);
                if add {
                    if let Some((_, Value::List(old))) = node.values.iter_mut().rev().find(|(k, _)| k.eq_ignore_ascii_case(&word)) {
                        old.extend(v);
                        self.eat(b';');
                        continue;
                    }
                }
                node.values.push((word, Value::List(v)));
            } else {
                let v = self.scalar();
                node.values.push((word, Value::Text(v)));
            }
            self.eat(b';');
        }
    }
}

/// Reads a config (`config.cpp`, as text) into its tree of classes.
pub fn parse(text: &str) -> Node {
    let mut lx = Lexer { b: text.as_bytes(), p: 0 };
    let mut root = Node::default();
    lx.body(&mut root);
    root
}

// ---------------------------------------------------------------- the game's surfaces

/// Fields of a surface written as text in the config.
pub const TEXT_FIELDS: [&str; 7] = ["soundEnviron", "soundHit", "impact", "vpSurface", "wheelParticle", "character", "digPile"];

#[derive(Clone, Debug, Default)]
pub struct GameSurface {
    pub class: String,
    pub parent: Option<String>,
    /// `files` without its `*`, lower case.
    pub files: String,
    /// Every field, those inherited included, by name as written.
    pub props: BTreeMap<String, String>,
    /// Visibility of someone lying, kneeling and standing on it.
    pub visible: [f32; 3],
    /// Addon (CfgPatches class) that defines it.
    pub addon: String,
}

#[derive(Clone, Debug)]
pub struct ClutterKind {
    pub name: String,
    /// WRP style path with `.p3d`, lower case.
    pub model: String,
    pub scale: [f32; 2],
}

#[derive(Clone, Debug, Default)]
pub struct SurfaceCatalog {
    pub surfaces: Vec<GameSurface>,
    /// Character, lower case -> (clutter, probability).
    pub characters: HashMap<String, Vec<(String, f32)>>,
    pub clutter: Vec<ClutterKind>,
    /// Addons defining surfaces, in load order.
    pub addons: Vec<String>,
}

fn patch_name(root: &Node) -> String {
    root.class("CfgPatches").and_then(|p| p.classes.first()).map(|c| c.name.clone()).unwrap_or_default()
}

impl SurfaceCatalog {
    /// Reads `dz\surfaces*\config.cpp` of the work drive.
    pub fn load(root: &Path) -> Self {
        let mut cat = Self::default();
        let Ok(rd) = std::fs::read_dir(root.join("dz")) else { return cat };
        let mut dirs: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().starts_with("surfaces"))).collect();
        // `surfaces` holds the bases the others inherit.
        dirs.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().len()).unwrap_or(0));
        for d in dirs {
            if let Ok(b) = std::fs::read(d.join("config.cpp")) {
                cat.add_config(&String::from_utf8_lossy(&b));
            }
        }
        cat.surfaces.sort_by(|a, b| a.class.to_ascii_lowercase().cmp(&b.class.to_ascii_lowercase()));
        cat
    }

    pub fn add_config(&mut self, text: &str) {
        let root = parse(text);
        let addon = patch_name(&root);
        if !addon.is_empty() && !self.addons.contains(&addon) {
            self.addons.push(addon.clone());
        }
        if let Some(s) = root.class("CfgSurfaces") {
            for c in &s.classes {
                let mut props: BTreeMap<String, String> = c.parent.as_deref().and_then(|p| self.raw(p)).map(|g| g.props.clone()).unwrap_or_default();
                let mut visible = c.parent.as_deref().and_then(|p| self.raw(p)).map_or([1.0; 3], |g| g.visible);
                for (k, v) in &c.values {
                    if let Value::Text(t) = v {
                        props.insert(k.clone(), t.clone());
                    }
                }
                if let Some(v) = c.class("Visible") {
                    let n = |k: &str, d: f32| v.text(k).and_then(|t| t.parse().ok()).unwrap_or(d);
                    visible = [n("prone", visible[0]), n("kneel", visible[1]), n("stand", visible[2])];
                }
                let files = props.get("files").map(|f| f.split('*').next().unwrap_or("").to_ascii_lowercase()).unwrap_or_default();
                let g = GameSurface { class: c.name.clone(), parent: c.parent.clone(), files, props, visible, addon: addon.clone() };
                match self.surfaces.iter_mut().find(|s| s.class.eq_ignore_ascii_case(&c.name)) {
                    Some(old) => *old = g,
                    None => self.surfaces.push(g),
                }
            }
        }
        if let Some(b) = root.class("CfgSurfaceCharacters") {
            for c in &b.classes {
                let odds: Vec<f32> = c.list("probability").iter().filter_map(|v| v.parse().ok()).collect();
                let list = c.list("names").into_iter().zip(odds).collect();
                self.characters.insert(c.name.to_ascii_lowercase(), list);
            }
        }
        if let Some(clutter) = root.class("CfgWorlds").and_then(|w| w.class("CAWorld")).and_then(|w| w.class("Clutter")) {
            for c in &clutter.classes {
                let Some(model) = c.text("model") else { continue };
                let model = model.replace('/', "\\").to_ascii_lowercase();
                let model = if model.ends_with(".p3d") { model } else { format!("{model}.p3d") };
                let n = |k: &str, d: f32| c.text(k).and_then(|t| t.parse().ok()).unwrap_or(d);
                let kind = ClutterKind { name: c.name.clone(), model, scale: [n("scaleMin", 0.8), n("scaleMax", 1.2)] };
                match self.clutter.iter_mut().find(|k| k.name.eq_ignore_ascii_case(&c.name)) {
                    Some(old) => *old = kind,
                    None => self.clutter.push(kind),
                }
            }
        }
    }

    fn raw(&self, class: &str) -> Option<&GameSurface> {
        self.surfaces.iter().find(|s| s.class.eq_ignore_ascii_case(class))
    }

    /// A terrain surface of the game (not one of building interiors or water).
    pub fn get(&self, class: &str) -> Option<&GameSurface> {
        self.raw(class)
    }

    /// The surfaces a map surface can be based on: those standing for terrain textures.
    pub fn terrain_surfaces(&self) -> impl Iterator<Item = &GameSurface> {
        self.surfaces.iter().filter(|s| !s.files.is_empty() && s.props.get("interior").is_none_or(|v| v.trim() == "0") && s.props.get("isLiquid").is_none_or(|v| v.trim() == "0"))
    }

    /// The surface of the game a texture gets, by its file name.
    pub fn for_texture(&self, texture: &str) -> Option<&GameSurface> {
        let stem = texture_stem(texture);
        self.terrain_surfaces().filter(|s| stem.starts_with(&s.files)).max_by_key(|s| s.files.len())
    }

    pub fn clutter_kind(&self, name: &str) -> Option<&ClutterKind> {
        self.clutter.iter().find(|k| k.name.eq_ignore_ascii_case(name))
    }

    /// The clutter of character `character`.
    pub fn character(&self, character: &str) -> Vec<(String, f32)> {
        self.characters.get(&character.to_ascii_lowercase()).cloned().unwrap_or_default()
    }
}

/// The end of the class starting at `from`: just past its closing brace.
fn block_end(text: &str, from: usize) -> Option<usize> {
    let open = text[from..].find('{')? + from;
    let mut depth = 0;
    for (i, c) in text[open..].bytes().enumerate() {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// File name of a texture in lower case, without its folder.
pub fn texture_stem(texture: &str) -> String {
    texture.rsplit(['\\', '/']).next().unwrap_or(texture).to_ascii_lowercase()
}

// ---------------------------------------------------------------- footsteps

/// The step sound tables of the game: (table, entry class, surface, sound sets) and the
/// addons they come from.
#[derive(Clone, Debug, Default)]
pub struct StepSounds {
    pub entries: Vec<(String, String, String, Vec<String>)>,
    pub addons: Vec<String>,
}

impl StepSounds {
    /// Reads the `CfgStepSoundTables` of `dz\sounds*` on the work drive; the configs are
    /// large, so this takes a moment.
    pub fn load(root: &Path) -> Self {
        let mut out = Self::default();
        let Ok(rd) = std::fs::read_dir(root.join("dz")) else { return out };
        let mut files = Vec::new();
        for e in rd.flatten() {
            if !e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("sounds") {
                continue;
            }
            files.push(e.path().join("config.cpp"));
            if let Ok(sub) = std::fs::read_dir(e.path()) {
                files.extend(sub.flatten().map(|s| s.path().join("config.cpp")));
            }
        }
        files.sort();
        for f in files {
            let Ok(b) = std::fs::read(&f) else { continue };
            let text = String::from_utf8_lossy(&b);
            let Some(at) = text.find("CfgStepSoundTables") else { continue };
            // Only the CfgPatches at the top and the tables are read.
            let addon = text.find("class CfgPatches").and_then(|p| block_end(&text, p).map(|e| patch_name(&parse(&text[p..e])))).unwrap_or_default();
            let start = text[..at].rfind("class CfgSoundTables").unwrap_or(at.saturating_sub(6));
            let root = parse(&text[start..]);
            let Some(tables) = root.class("CfgSoundTables").and_then(|t| t.class("CfgStepSoundTables")) else { continue };
            if !addon.is_empty() && !out.addons.contains(&addon) {
                out.addons.push(addon);
            }
            for table in &tables.classes {
                for e in &table.classes {
                    if let Some(s) = e.text("surface") {
                        out.entries.push((table.name.clone(), e.name.clone(), s.to_ascii_lowercase(), e.list("soundSets")));
                    }
                }
            }
        }
        out
    }

    /// How many tables give surface `surface` a sound.
    pub fn count(&self, surface: &str) -> usize {
        let s = surface.to_ascii_lowercase();
        self.entries.iter().filter(|e| e.2 == s).count()
    }
}

// ---------------------------------------------------------------- the map's surfaces

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClutterPick {
    pub name: String,
    pub probability: f32,
}

/// A surface of the map: a change to a surface of the game, or a new one inheriting it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct MapSurface {
    /// Class name in CfgSurfaces.
    pub name: String,
    /// The start of the file names of the textures it stands for (`files`, without `*`).
    pub files: String,
    /// The surface of the game it inherits from, or changes when `replaces` is set.
    pub base: String,
    /// Changes the surface `base` of the game itself rather than adding a new one.
    pub replaces: bool,
    /// Fields set over those of the base, by their config name.
    pub props: BTreeMap<String, String>,
    /// Visibility lying, kneeling, standing; `None` keeps the base's.
    pub visible: Option<[f32; 3]>,
    /// Its own grass; `None` keeps that of the base.
    pub clutter: Option<Vec<ClutterPick>>,
    /// The surface whose footsteps it makes; empty takes the base's. New surfaces only.
    pub sounds: String,
}

impl MapSurface {
    /// The class name as written in the config.
    pub fn class(&self) -> &str {
        if self.replaces { &self.base } else { &self.name }
    }
    pub fn sound_source(&self) -> &str {
        if self.sounds.trim().is_empty() { &self.base } else { &self.sounds }
    }
    /// Whether textures named like `texture` get this surface.
    pub fn matches(&self, texture: &str) -> bool {
        let f = self.files.trim().trim_end_matches('*').to_ascii_lowercase();
        !f.is_empty() && texture_stem(texture).starts_with(&f)
    }
}

/// The surface a layer gets in the game: the map's own first (the longest `files` match),
/// then the game's. Returns (map surface, game surface it is based on or is).
pub fn surface_of<'a, 'b>(texture: &str, own: &'a [MapSurface], cat: &'b SurfaceCatalog) -> (Option<&'a MapSurface>, Option<&'b GameSurface>) {
    let mine = own.iter().filter(|s| s.matches(texture)).max_by_key(|s| s.files.trim().len());
    // A changed surface of the game keeps the files of the game.
    let mine = mine.or_else(|| {
        let g = cat.for_texture(texture)?;
        own.iter().find(|s| s.replaces && s.base.eq_ignore_ascii_case(&g.class) && s.files.trim().is_empty())
    });
    let game = match mine {
        Some(m) => cat.get(&m.base),
        None => cat.for_texture(texture),
    };
    (mine, game)
}

/// The grass of a layer as (clutter class, probability).
pub fn clutter_of(texture: &str, own: &[MapSurface], cat: &SurfaceCatalog) -> Vec<(String, f32)> {
    let (mine, game) = surface_of(texture, own, cat);
    if let Some(list) = mine.and_then(|m| m.clutter.as_ref()) {
        return list.iter().map(|c| (c.name.clone(), c.probability)).collect();
    }
    let character = mine.and_then(|m| m.props.get("character")).or_else(|| game.and_then(|g| g.props.get("character")));
    character.map(|c| cat.character(c)).unwrap_or_default()
}

fn class_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if n.chars().next().is_some_and(|c| c.is_ascii_digit()) { format!("s_{n}") } else { n }
}

fn value_text(key: &str, v: &str) -> String {
    let v = v.trim();
    if TEXT_FIELDS.iter().any(|k| k.eq_ignore_ascii_case(key)) || v.parse::<f64>().is_err() {
        format!("\"{}\"", v.trim_matches('"').replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

/// The config of the map's surfaces (`surfaces\config.cpp` of the map), or `None` when the
/// map has none. `sounds` gives the footsteps copied to new surfaces.
pub fn config_text(map_id: &str, own: &[MapSurface], cat: &SurfaceCatalog, sounds: &StepSounds) -> Option<String> {
    let own: Vec<&MapSurface> = own.iter().filter(|s| !s.base.trim().is_empty() && (s.replaces || !s.name.trim().is_empty())).collect();
    if own.is_empty() {
        return None;
    }
    let mut addons: Vec<String> = cat.addons.clone();
    let mut surfaces = String::new();
    let mut characters = String::new();
    // Bases defined elsewhere are declared first; changed surfaces of the game come next, so
    // new ones may inherit them.
    let mut declared: Vec<String> = Vec::new();
    let declare = |name: &str, declared: &mut Vec<String>, out: &mut String| {
        if !declared.iter().any(|d| d.eq_ignore_ascii_case(name)) && !own.iter().any(|s| s.replaces && s.base.eq_ignore_ascii_case(name)) {
            declared.push(name.to_string());
            *out = format!("\tclass {name};\n") + out;
        }
    };
    let mut body = String::new();
    let mut ordered: Vec<&&MapSurface> = own.iter().filter(|s| s.replaces).collect();
    ordered.extend(own.iter().filter(|s| !s.replaces));
    for s in ordered {
        let class = class_name(s.class());
        let parent = if s.replaces { cat.get(&s.base).and_then(|g| g.parent.clone()).unwrap_or_else(|| "DZ_SurfacesExt".into()) } else { cat.get(&s.base).map_or(s.base.clone(), |g| g.class.clone()) };
        declare(&parent, &mut declared, &mut surfaces);
        body += &format!("\tclass {class}: {parent}\n\t{{\n");
        if !s.replaces || !s.files.trim().is_empty() {
            let files = if s.files.trim().is_empty() { s.name.to_ascii_lowercase() } else { s.files.trim().trim_end_matches('*').to_ascii_lowercase() };
            body += &format!("\t\tfiles=\"{files}*\";\n");
        }
        for (k, v) in &s.props {
            if k.eq_ignore_ascii_case("files") || k.eq_ignore_ascii_case("character") || v.trim().is_empty() {
                continue;
            }
            body += &format!("\t\t{k}={};\n", value_text(k, v));
        }
        let character = match &s.clutter {
            Some(list) => {
                let name = class_name(&format!("{map_id}_{class}"));
                characters += &format!(
                    "\tclass {name}\n\t{{\n\t\tprobability[]={{{}}};\n\t\tnames[]={{{}}};\n\t}};\n",
                    list.iter().map(|c| format!("{}", c.probability)).collect::<Vec<_>>().join(","),
                    list.iter().map(|c| format!("\"{}\"", c.name)).collect::<Vec<_>>().join(",")
                );
                Some(name)
            }
            None => s.props.get("character").cloned(),
        };
        if let Some(c) = character {
            body += &format!("\t\tcharacter=\"{c}\";\n");
        }
        if let Some(v) = s.visible {
            body += &format!("\t\tclass Visible\n\t\t{{\n\t\t\tprone={};\n\t\t\tkneel={};\n\t\t\tstand={};\n\t\t}};\n", v[0], v[1], v[2]);
        }
        body += "\t};\n";
    }
    surfaces += &body;
    // Footsteps: every table that has the source surface gets the same sounds for the new one.
    let mut tables: BTreeMap<String, String> = BTreeMap::new();
    for s in own.iter().filter(|s| !s.replaces) {
        let class = class_name(&s.name);
        let source = s.sound_source().to_ascii_lowercase();
        for (table, entry, surface, sets) in &sounds.entries {
            if *surface != source {
                continue;
            }
            let lower = entry.to_ascii_lowercase();
            let name = if lower.ends_with(&source) { format!("{}{class}", &entry[..entry.len() - source.len()]) } else { format!("{entry}_{class}") };
            let sets = sets.iter().map(|x| format!("\"{x}\"")).collect::<Vec<_>>().join(",");
            *tables.entry(table.clone()).or_default() += &format!("\t\t\tclass {name}\n\t\t\t{{\n\t\t\t\tsurface=\"{class}\";\n\t\t\t\tsoundSets[]={{{sets}}};\n\t\t\t}};\n");
        }
    }
    if !tables.is_empty() {
        addons.extend(sounds.addons.iter().cloned());
    }
    addons.dedup();
    let mut out = format!(
        "class CfgPatches\n{{\n\tclass {map_id}_surfaces\n\t{{\n\t\tunits[]={{}};\n\t\tweapons[]={{}};\n\t\trequiredVersion=0.1;\n\t\trequiredAddons[]={{{}}};\n\t}};\n}};\nclass CfgSurfaces\n{{\n{surfaces}}};\n",
        addons.iter().map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(",")
    );
    if !characters.is_empty() {
        out += &format!("class CfgSurfaceCharacters\n{{\n{characters}}};\n");
    }
    if !tables.is_empty() {
        out += "class CfgSoundTables\n{\n\tclass CfgStepSoundTables\n\t{\n";
        for (table, entries) in &tables {
            out += &format!("\t\tclass {table}\n\t\t{{\n{entries}\t\t}};\n");
        }
        out += "\t};\n};\n";
    }
    Some(out)
}

/// Writes `surfaces\config.cpp` into the map folder `target`, or removes an old one when the
/// map has no surfaces of its own. Returns whether the map has any.
pub fn write_config(map_id: &str, own: &[MapSurface], root: &Path, target: &Path) -> Result<bool> {
    let file = target.join("surfaces").join("config.cpp");
    if own.is_empty() {
        let _ = std::fs::remove_file(&file);
        return Ok(false);
    }
    let cat = SurfaceCatalog::load(root);
    let sounds = if own.iter().any(|s| !s.replaces) { StepSounds::load(root) } else { StepSounds::default() };
    match config_text(map_id, own, &cat, &sounds) {
        Some(text) => {
            crate::fsutil::atomic_write(&file, text.as_bytes())?;
            Ok(true)
        }
        None => {
            let _ = std::fs::remove_file(&file);
            Ok(false)
        }
    }
}

// ---------------------------------------------------------------- a map's own CfgSurfaces

/// Whether a config text defines surfaces.
pub fn defines_surfaces(text: &str) -> bool {
    text.to_ascii_lowercase().contains("cfgsurfaces")
}

/// The surfaces a map's config (`CfgSurfaces`, with `CfgSurfaceCharacters` beside it) defines,
/// as surfaces of the project: a class the game already has changes that surface, any other is
/// a new one. A class inheriting another class of the same config takes its fields, so each
/// new surface is based on a class from outside it. New surfaces make their footsteps like the
/// game's surface that sounds most alike, as the map's own step sound tables cannot be kept.
pub fn map_surfaces_from_config(text: &str, cat: &SurfaceCatalog) -> Vec<MapSurface> {
    let root = parse(text);
    let Some(list) = root.class("CfgSurfaces") else { return Vec::new() };
    // `class X;` only names a class defined elsewhere.
    let defined: Vec<&Node> = list.classes.iter().filter(|c| c.parent.is_some() || !c.values.is_empty() || !c.classes.is_empty()).collect();
    let own = |name: &str| defined.iter().copied().find(|c| c.name.eq_ignore_ascii_case(name));
    let characters = root.class("CfgSurfaceCharacters");
    let mut out = Vec::new();
    for c in &defined {
        // The fields of the chain of classes of this config, the nearest last.
        let mut chain = vec![*c];
        let mut base = c.parent.clone().unwrap_or_default();
        while let Some(p) = own(&base).filter(|p| !chain.iter().any(|x| x.name.eq_ignore_ascii_case(&p.name))) {
            chain.push(p);
            base = p.parent.clone().unwrap_or_default();
        }
        let mut props: BTreeMap<String, String> = BTreeMap::new();
        let mut visible = None;
        for n in chain.iter().rev() {
            for (k, v) in &n.values {
                if let Value::Text(t) = v {
                    props.insert(k.clone(), t.clone());
                }
            }
            if let Some(v) = n.class("Visible") {
                let old: [f32; 3] = visible.unwrap_or([1.0; 3]);
                let f = |k: &str, d: f32| v.text(k).and_then(|t| t.parse().ok()).unwrap_or(d);
                visible = Some([f("prone", old[0]), f("kneel", old[1]), f("stand", old[2])]);
            }
        }
        let files = props.remove("files").map(|f| f.split('*').next().unwrap_or("").trim().to_ascii_lowercase()).unwrap_or_default();
        // Grass of a character of this config comes along as the surface's own.
        let clutter = props.get("character").and_then(|ch| characters?.class(ch)).map(|ch| {
            let odds: Vec<f32> = ch.list("probability").iter().filter_map(|v| v.trim().parse().ok()).collect();
            ch.list("names").into_iter().zip(odds).map(|(name, probability)| ClutterPick { name, probability }).collect::<Vec<_>>()
        });
        if clutter.is_some() {
            props.remove("character");
        }
        let replaces = cat.get(&c.name).is_some();
        let sounds = if replaces { String::new() } else { sounding_like(&props, cat) };
        out.push(MapSurface {
            name: if replaces { String::new() } else { c.name.clone() },
            files: if replaces && cat.get(&c.name).is_some_and(|g| g.files == files) { String::new() } else { files },
            base: if replaces { c.name.clone() } else { base },
            replaces,
            props,
            visible,
            clutter,
            sounds,
        });
    }
    out
}

/// The terrain surface of the game whose sound and feel are most like `props`.
fn sounding_like(props: &BTreeMap<String, String>, cat: &SurfaceCatalog) -> String {
    let keys = [("soundEnviron", 4), ("soundHit", 2), ("vpSurface", 2), ("impact", 1), ("character", 1)];
    let value = |m: &BTreeMap<String, String>, k: &str| m.iter().find(|(x, _)| x.eq_ignore_ascii_case(k)).map(|(_, v)| v.trim().trim_matches('"').to_ascii_lowercase());
    cat.terrain_surfaces()
        .map(|g| (keys.iter().filter(|(k, _)| value(props, k).is_some_and(|v| Some(v) == value(&g.props, k))).map(|(_, w)| *w).sum::<i32>(), g))
        .filter(|(score, _)| *score > 0)
        .max_by_key(|(score, g)| (*score, std::cmp::Reverse(g.class.to_ascii_lowercase())))
        .map(|(_, g)| g.class.clone())
        .unwrap_or_default()
}

/// Merges surfaces read from a map's config into the project's: one with the class of a
/// surface already there takes its place.
pub fn merge_surfaces(own: &mut Vec<MapSurface>, read: Vec<MapSurface>) -> usize {
    let n = read.len();
    for s in read {
        match own.iter_mut().find(|o| o.class().eq_ignore_ascii_case(s.class())) {
            Some(old) => *old = s,
            None => own.push(s),
        }
    }
    n
}

// ---------------------------------------------------------------- the textures of layers

/// Where the map keeps the textures of its own surfaces, relative to the work drive.
pub fn layer_dir(map_id: &str) -> String {
    format!("{map_id}\\data\\surfaces")
}

/// The file name of a colour texture without `_ca`/`_co` and `.paa`, lower case.
pub fn layer_stem(texture: &str) -> String {
    let f = texture_stem(texture);
    let f = f.rsplit_once('.').filter(|(_, e)| !e.contains('\\') && e.len() <= 4).map_or(f.as_str(), |(s, _)| s);
    f.strip_suffix("_ca").or_else(|| f.strip_suffix("_co")).unwrap_or(f).to_string()
}

/// A path relative to the work drive, with backslashes.
fn rel(path: &str) -> String {
    path.trim().replace('/', "\\").trim_start_matches('\\').to_string()
}

/// The material of a surface layer as the game's surfaces have it: relief, then colour.
pub fn layer_rvmat_text(texture: &str, nopx: Option<&str>) -> String {
    crate::layer_material::LayerMaterial::for_texture(&rel(texture), nopx.map(rel).as_deref()).text()
}

/// The relief texture next to a colour texture, when there is one: `<stem>_nopx.paa`.
fn nopx_next_to(root: &Path, texture: &str) -> Option<String> {
    let t = rel(texture);
    let dir = t.rsplit_once('\\').map_or("", |d| d.0);
    let name = format!("{}_nopx.paa", layer_stem(&t));
    let path = if dir.is_empty() { name } else { format!("{dir}\\{name}") };
    root.join(path.replace('\\', "/")).is_file().then_some(path)
}

/// Copies a colour texture (with its `_nopx`, `_nohq` and material next to it) into the map's
/// folder of surfaces, and writes the material when there is none. `source` may be anywhere;
/// a texture already in that folder stays where it is. Returns (texture, material) as layers
/// keep them, relative to the work drive.
pub fn import_layer_texture(root: &Path, map_id: &str, source: &Path) -> Result<(String, String)> {
    let name = source.file_name().map(|f| f.to_string_lossy().to_ascii_lowercase()).ok_or_else(|| anyhow::anyhow!("No file chosen"))?;
    let stem = layer_stem(&name);
    // An image (PNG, TGA...) becomes the `_ca` texture of the game.
    let image = !name.ends_with(".paa");
    let file = if image { format!("{stem}_ca.paa") } else { name };
    let dir = layer_dir(map_id);
    let target = root.join(dir.replace('\\', "/"));
    std::fs::create_dir_all(&target)?;
    let from = source.parent().unwrap_or(Path::new("."));
    let same = from.canonicalize().ok() == target.canonicalize().ok();
    if image {
        crate::layer_material::texture_into(source, &target.join(&file))?;
    } else if !same {
        std::fs::copy(source, target.join(&file))?;
    }
    if !same {
        for extra in [format!("{stem}_nopx.paa"), format!("{stem}_nohq.paa")] {
            // Any case of the name on disk.
            if let Some(found) = std::fs::read_dir(from).ok().and_then(|rd| rd.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(&extra))) {
                std::fs::copy(found.path(), target.join(&extra))?;
            }
        }
    }
    let texture = format!("{dir}\\{file}");
    let material = format!("{dir}\\{stem}.rvmat");
    let material_path = target.join(format!("{stem}.rvmat"));
    let own = std::fs::read_dir(from).ok().and_then(|rd| rd.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(&format!("{stem}.rvmat"))));
    match own {
        // Its own material, pointed at the copies.
        Some(m) if !same => {
            let text = String::from_utf8_lossy(&std::fs::read(m.path())?).into_owned();
            let old = source.parent().and_then(|p| p.strip_prefix(root).ok()).map(|p| rel(&p.to_string_lossy()));
            let text = match old.filter(|o| !o.is_empty()) {
                Some(o) => replace_ignore_case(&text, &format!("{o}\\"), &format!("{dir}\\")),
                None => text,
            };
            crate::fsutil::atomic_write(&material_path, text.as_bytes())?;
        }
        Some(_) => {}
        None => {
            let text = layer_rvmat_text(&texture, nopx_next_to(root, &texture).as_deref());
            crate::fsutil::atomic_write(&material_path, text.as_bytes())?;
        }
    }
    Ok((texture, material))
}

fn replace_ignore_case(text: &str, from: &str, to: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let needle = from.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(i) = lower[at..].find(&needle) {
        out += &text[at..at + i];
        out += to;
        at += i + needle.len();
    }
    out += &text[at..];
    out
}

/// Writes the materials missing for layers whose files are in the map's own folder. Returns
/// what is still wrong with the layers, for the export report.
pub fn check_layers(root: &Path, map_id: &str, layers: &[crate::world::SurfaceLayer], build: &[crate::build::BuildFolder]) -> Vec<String> {
    let mut out = Vec::new();
    let exists = |p: &str| root.join(rel(p).replace('\\', "/")).is_file();
    let mine = |p: &str| {
        let p = rel(p).to_ascii_lowercase();
        p.starts_with(&format!("{}\\", map_id.to_ascii_lowercase())) || p.starts_with("dz\\")
    };
    let built = |p: &str| {
        let p = rel(p).to_ascii_lowercase();
        build.iter().any(|b| b.enabled && p.starts_with(&format!("{}\\", rel(&b.folder).to_ascii_lowercase())))
    };
    for l in layers {
        if !exists(&l.texture) {
            out.push(format!("Layer {}: its texture {} is not on the work drive", l.name, l.texture));
            continue;
        }
        if !exists(&l.material) {
            if mine(&l.material) && !rel(&l.material).to_ascii_lowercase().starts_with("dz\\") {
                let text = layer_rvmat_text(&l.texture, nopx_next_to(root, &l.texture).as_deref());
                if crate::fsutil::atomic_write(&root.join(rel(&l.material).replace('\\', "/")), text.as_bytes()).is_ok() {
                    out.push(format!("Layer {}: wrote its missing material {}", l.name, l.material));
                    continue;
                }
            }
            out.push(format!("Layer {}: its material {} is not on the work drive", l.name, l.material));
        }
        for p in [&l.texture, &l.material] {
            if !mine(p) && !built(p) {
                out.push(format!("Layer {}: {} is outside the map's folder and not built into the mod; copy the textures into the map (right click on the layer) or add their folder in Build", l.name, p));
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_maps_own_cfgsurfaces_become_surfaces_of_the_project() {
        let mut cat = SurfaceCatalog::default();
        cat.add_config(r#"class CfgPatches { class DZ_Surfaces {}; };
class CfgSurfaces {
    class DZ_SurfacesExt { };
    class cp_concrete1: DZ_SurfacesExt { files = "cp_concrete1*"; soundEnviron = "road"; soundHit = "hard_ground"; vpSurface = "Asphalt"; character = "cp_concrete_grass"; };
    class cp_grass: DZ_SurfacesExt { files = "cp_grass*"; soundEnviron = "grass"; character = "cp_grass_char"; };
};"#);
        let text = r#"class CfgPatches { class my_surfaces { requiredAddons[] = {"DZ_Data"}; }; };
class CfgSurfaces
{
    class DZ_SurfacesInt;
    class DZ_SurfacesExt;
    class utes_concrete: DZ_SurfacesExt
    {
        files = "utes_concrete*";
        rough = 0.01;
        soundEnviron = "road";
        soundHit = "hard_ground";
        vpSurface = "Asphalt";
        character = "cp_concrete_grass";
        class Visible { prone = 0.5; };
    };
    class utes_concrete_dark: utes_concrete { files = "utes_dark*"; dust = 0.2; character = "utes_dark_grass"; };
    class cp_grass: DZ_SurfacesExt { files = "cp_grass*"; rough = 0.2; };
};
class CfgSurfaceCharacters
{
    class utes_dark_grass { probability[] = {0.4, 0.1}; names[] = {"GrassTall", "Flowers"}; };
};"#;
        assert!(defines_surfaces(text));
        let read = map_surfaces_from_config(text, &cat);
        assert_eq!(read.len(), 3, "{read:?}");
        let concrete = &read[0];
        assert_eq!((concrete.name.as_str(), concrete.files.as_str(), concrete.base.as_str(), concrete.replaces), ("utes_concrete", "utes_concrete", "DZ_SurfacesExt", false));
        assert_eq!(concrete.props.get("character").map(String::as_str), Some("cp_concrete_grass"));
        assert_eq!(concrete.visible, Some([0.5, 1.0, 1.0]));
        assert_eq!(concrete.sounds, "cp_concrete1", "sounds like the game's concrete");
        // Inherits the fields of its parent in the same config, based on the parent's base.
        let dark = &read[1];
        assert_eq!((dark.base.as_str(), dark.files.as_str()), ("DZ_SurfacesExt", "utes_dark"));
        assert_eq!(dark.props.get("rough").map(String::as_str), Some("0.01"));
        assert_eq!(dark.props.get("dust").map(String::as_str), Some("0.2"));
        assert_eq!(dark.clutter.as_ref().map(|c| c.iter().map(|p| (p.name.as_str(), p.probability)).collect::<Vec<_>>()), Some(vec![("GrassTall", 0.4), ("Flowers", 0.1)]));
        // A class of the game is changed, keeping its files.
        let grass = &read[2];
        assert!(grass.replaces && grass.base == "cp_grass" && grass.files.is_empty(), "{grass:?}");
        let mut own = vec![MapSurface { name: "utes_concrete".into(), base: "x".into(), ..Default::default() }];
        assert_eq!(merge_surfaces(&mut own, read), 3);
        assert_eq!(own.len(), 3);
        assert_eq!(own[0].base, "DZ_SurfacesExt");
    }

    #[test]
    fn a_texture_from_anywhere_comes_into_the_map_with_a_material() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("P");
        let outside = d.path().join("desk");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("My_Grass_CA.paa"), b"ca").unwrap();
        std::fs::write(outside.join("my_grass_nopx.paa"), b"nopx").unwrap();
        let (tex, mat) = import_layer_texture(&root, "MyMap", &outside.join("My_Grass_CA.paa")).unwrap();
        assert_eq!(tex, "MyMap\\data\\surfaces\\my_grass_ca.paa");
        assert_eq!(mat, "MyMap\\data\\surfaces\\my_grass.rvmat");
        let dir = root.join("MyMap/data/surfaces");
        assert!(dir.join("my_grass_ca.paa").is_file() && dir.join("my_grass_nopx.paa").is_file());
        let text = std::fs::read_to_string(dir.join("my_grass.rvmat")).unwrap();
        assert!(text.contains("texture=\"MyMap\\data\\surfaces\\my_grass_nopx.paa\";"), "{text}");
        assert!(text.contains("texture=\"MyMap\\data\\surfaces\\my_grass_ca.paa\";"), "{text}");
        assert_eq!(layer_stem(&tex), "my_grass");
        // Its own material moves along, pointed at the copies.
        let other = root.join("Textures");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("mud_co.paa"), b"co").unwrap();
        std::fs::write(other.join("mud.rvmat"), "texture=\"Textures\\mud_co.paa\";").unwrap();
        let (_, mat) = import_layer_texture(&root, "MyMap", &other.join("mud_co.paa")).unwrap();
        let text = std::fs::read_to_string(root.join(mat.replace('\\', "/"))).unwrap();
        assert_eq!(text, "texture=\"MyMap\\data\\surfaces\\mud_co.paa\";");
    }

    #[test]
    fn layers_are_checked_and_missing_materials_written() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        std::fs::create_dir_all(root.join("MyMap/data/surfaces")).unwrap();
        std::fs::write(root.join("MyMap/data/surfaces/a_ca.paa"), b"x").unwrap();
        std::fs::create_dir_all(root.join("Other")).unwrap();
        std::fs::write(root.join("Other/b_ca.paa"), b"x").unwrap();
        std::fs::write(root.join("Other/b.rvmat"), b"x").unwrap();
        let layer = |name: &str, tex: &str, mat: &str| crate::world::SurfaceLayer { name: name.into(), texture: tex.into(), material: mat.into(), color: [0; 3] };
        let layers = vec![layer("a", "MyMap\\data\\surfaces\\a_ca.paa", "MyMap\\data\\surfaces\\a.rvmat"), layer("b", "Other\\b_ca.paa", "Other\\b.rvmat"), layer("c", "Nowhere\\c_ca.paa", "Nowhere\\c.rvmat")];
        let notes = check_layers(root, "MyMap", &layers, &[]);
        assert!(root.join("MyMap/data/surfaces/a.rvmat").is_file());
        assert!(notes.iter().any(|n| n.starts_with("Layer b:") && n.contains("outside")), "{notes:?}");
        assert!(notes.iter().any(|n| n.starts_with("Layer c:") && n.contains("not on the work drive")), "{notes:?}");
        // Once its folder is built, b is fine.
        let notes = check_layers(root, "MyMap", &layers, &[crate::build::BuildFolder::new("Other")]);
        assert!(!notes.iter().any(|n| n.starts_with("Layer b:")), "{notes:?}");
    }

    const SURFACES: &str = r#"
class CfgPatches { class DZ_Surfaces_Test { requiredAddons[] = {"DZ_Surfaces"}; }; };
class CfgSurfaces
{
    class DZ_SurfacesExt;
    class en_grass1 : DZ_SurfacesExt
    {
        files = "en_grass1*";
        friction = 0.85;
        soundEnviron = "grass";
        character = "en_Grass1";
        class Visible { prone = 0.2; kneel = 0.6; stand = 0.8; };
    };
    class en_flowers1 : en_grass1 { files = "en_flowers1*"; character = "en_flowers1"; };
};
class CfgSurfaceCharacters
{
    class en_grass1 { probability[] = {0.4, 0.1}; names[] = { "Short", "Tall" }; };
};
class CfgWorlds { class DefaultWorld; class CAWorld : DefaultWorld { class DefaultClutter; class Clutter {
    class Short : DefaultClutter { model = "dz\plants\clutter\c_short.p3d"; };
    class Tall : DefaultClutter { model = "dz\plants\clutter\c_tall.p3d"; };
}; }; };
"#;

    const SOUNDS: &str = r#"
class CfgSoundTables { class CfgStepSoundTables {
    class HumanWalk_LookupTable {
        class HumanWalkstepSound_en_grass1 { surface = "en_grass1"; soundSets[] = { "walk_grass_SoundSet", "walk_grass2_SoundSet" }; };
        class HumanWalkstepSound_road { surface = "cp_concrete1"; soundSets[] = { "walk_road_SoundSet" }; };
    };
}; };
"#;

    fn catalog() -> SurfaceCatalog {
        let mut c = SurfaceCatalog::default();
        c.add_config(SURFACES);
        c
    }

    #[test]
    fn inherited_fields_and_lookups() {
        let c = catalog();
        let f = c.get("en_flowers1").unwrap();
        assert_eq!(f.props.get("friction").map(String::as_str), Some("0.85"));
        assert_eq!(f.visible, [0.2, 0.6, 0.8]);
        assert_eq!(c.for_texture("dz\\x\\en_flowers1_ca.paa").unwrap().class, "en_flowers1");
        assert_eq!(c.character("EN_GRASS1").len(), 2);
        assert_eq!(c.clutter.len(), 2);
    }

    #[test]
    fn a_new_surface_takes_the_grass_and_footsteps_it_is_given() {
        let c = catalog();
        let own = vec![MapSurface { name: "my_meadow".into(), files: "my_meadow".into(), base: "en_grass1".into(), clutter: Some(vec![ClutterPick { name: "Tall".into(), probability: 0.3 }]), ..Default::default() }];
        assert_eq!(clutter_of("mymod\\data\\my_meadow_ca.paa", &own, &c), vec![("Tall".to_string(), 0.3)]);
        // Without its own grass it has that of its base.
        let plain = vec![MapSurface { clutter: None, ..own[0].clone() }];
        assert_eq!(clutter_of("mymod\\data\\my_meadow_ca.paa", &plain, &c).len(), 2);
        let mut sounds = StepSounds::default();
        let root = parse(SOUNDS);
        for t in &root.class("CfgSoundTables").unwrap().class("CfgStepSoundTables").unwrap().classes {
            for e in &t.classes {
                sounds.entries.push((t.name.clone(), e.name.clone(), e.text("surface").unwrap().to_string(), e.list("soundSets")));
            }
        }
        let text = config_text("MyMap", &own, &c, &sounds).unwrap();
        assert!(text.contains("class en_grass1;\n"), "{text}");
        assert!(text.contains("class my_meadow: en_grass1"), "{text}");
        assert!(text.contains("files=\"my_meadow*\";"), "{text}");
        assert!(text.contains("character=\"MyMap_my_meadow\";"), "{text}");
        assert!(text.contains("names[]={\"Tall\"};"), "{text}");
        assert!(text.contains("class HumanWalkstepSound_my_meadow"), "{text}");
        assert!(text.contains("soundSets[]={\"walk_grass_SoundSet\",\"walk_grass2_SoundSet\"};"), "{text}");
        assert!(!text.contains("walk_road"), "{text}");
        // The config it writes reads back.
        let back = parse(&text);
        assert_eq!(back.class("CfgSurfaces").unwrap().class("my_meadow").unwrap().parent.as_deref(), Some("en_grass1"));
    }

    #[test]
    fn a_changed_surface_of_the_game_keeps_its_class_and_parent() {
        let c = catalog();
        let own = vec![MapSurface { base: "en_grass1".into(), replaces: true, props: [("friction".to_string(), "0.5".to_string()), ("soundEnviron".to_string(), "forest".to_string())].into(), clutter: Some(Vec::new()), ..Default::default() }];
        assert!(clutter_of("dz\\surfaces_bliss\\data\\terrain\\en_grass1_ca.paa", &own, &c).is_empty());
        let text = config_text("MyMap", &own, &c, &StepSounds::default()).unwrap();
        assert!(text.contains("class DZ_SurfacesExt;\n\tclass en_grass1: DZ_SurfacesExt"), "{text}");
        assert!(text.contains("friction=0.5;") && text.contains("soundEnviron=\"forest\";"), "{text}");
        assert!(!text.contains("files="), "{text}");
        assert!(!text.contains("CfgSoundTables"), "{text}");
    }

    #[test]
    fn real_surfaces_and_sounds_when_p_is_mounted() {
        let root = Path::new("P:/");
        if !root.join("dz/surfaces_bliss/config.cpp").is_file() {
            return;
        }
        let c = SurfaceCatalog::load(root);
        let g = c.get("en_grass1").expect("en_grass1");
        assert_eq!(g.parent.as_deref(), Some("DZ_SurfacesExt"));
        assert!(c.character("en_grass1").len() >= 5);
        assert!(c.terrain_surfaces().count() > 40);
        let t = std::time::Instant::now();
        let s = StepSounds::load(root);
        assert!(s.count("en_grass1") > 50, "{}", s.count("en_grass1"));
        assert!(s.addons.iter().any(|a| a == "DZ_Sounds_Effects"), "{:?}", s.addons);
        eprintln!("step sounds: {} entries in {:?}", s.entries.len(), t.elapsed());
    }
}
