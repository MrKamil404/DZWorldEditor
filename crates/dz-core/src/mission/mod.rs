//! The server mission of the map (`mpmissions\<name>`): the Central Economy, spawns, weather,
//! gameplay settings and the rest. Every file is read into a form that keeps all it holds, so
//! a mission goes out exactly as it came in apart from what was edited:
//!
//! * XML files become [`xml::XmlDoc`] trees, with comments and unknown elements;
//! * JSON files keep their keys in order ([`json::JsonDoc`]);
//! * `init.c` is text ([`TextDoc`]);
//! * `areaflags.map` is read into [`areaflags::AreaFlags`];
//! * anything else, the persistence folders (`storage_*`) and the large cluster files stay on
//!   disk and are copied as they are.
//!
//! Files nobody edited are copied byte for byte. The typed helpers in the submodules read and
//! change the trees in place.
pub mod areaflags;
pub mod ce;
pub mod classes;
pub mod gen;
pub mod initc;
pub mod json;
pub mod places;
pub mod templates;
pub mod validate;
pub mod xml;

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use xml::{Element, Node, XmlDoc};

pub use areaflags::{AreaFlags, FlagLayer};
pub use json::JsonDoc;

/// Text kept as it is, such as `init.c`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDoc {
    /// With `\n` line ends.
    pub text: String,
    pub crlf: bool,
    pub bom: bool,
}

impl TextDoc {
    pub fn parse(b: &[u8]) -> Self {
        let bom = b.starts_with(&[0xEF, 0xBB, 0xBF]);
        let s = String::from_utf8_lossy(if bom { &b[3..] } else { b });
        // Line ends are made `\n` only when they are all `\r\n`, so mixed files stay as they are.
        let crlf = s.contains("\r\n") && s.matches('\n').count() == s.matches("\r\n").count() && !s.replace("\r\n", "").contains('\r');
        let text = if crlf { s.replace("\r\n", "\n") } else { s.to_string() };
        Self { text, crlf, bom }
    }
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), crlf: false, bom: false }
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.bom {
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        if self.crlf {
            out.extend_from_slice(self.text.replace('\n', "\r\n").as_bytes());
        } else {
            out.extend_from_slice(self.text.as_bytes());
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FileData {
    Xml(XmlDoc),
    Json(JsonDoc),
    Text(TextDoc),
    AreaFlags(AreaFlags),
    /// Not read: copied from `origin` as it is.
    Disk,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MissionFile {
    pub data: FileData,
    /// Where the file was read from; unedited files are copied from there.
    pub origin: Option<PathBuf>,
    /// Changed since it was read or last saved.
    pub dirty: bool,
}

impl MissionFile {
    pub fn new(data: FileData) -> Self {
        Self { data, origin: None, dirty: true }
    }
    /// The bytes of the file as it would be written.
    pub fn bytes(&self) -> Result<Vec<u8>> {
        if !self.dirty {
            if let Some(o) = self.origin.as_ref().filter(|o| o.is_file()) {
                return std::fs::read(o).with_context(|| o.display().to_string());
            }
        }
        Ok(match &self.data {
            FileData::Xml(d) => d.to_bytes(),
            FileData::Json(d) => d.to_bytes(),
            FileData::Text(d) => d.to_bytes(),
            FileData::AreaFlags(a) => a.to_bytes(),
            FileData::Disk => {
                let o = self.origin.as_ref().context("A mission file has no source")?;
                std::fs::read(o).with_context(|| o.display().to_string())?
            }
        })
    }
    /// The bytes as written from the parsed form, even when nothing changed.
    pub fn serialized(&self) -> Option<Vec<u8>> {
        Some(match &self.data {
            FileData::Xml(d) => d.to_bytes(),
            FileData::Json(d) => d.to_bytes(),
            FileData::Text(d) => d.to_bytes(),
            FileData::AreaFlags(a) => a.to_bytes(),
            FileData::Disk => return None,
        })
    }
}

/// Folders the server keeps its persistence in.
pub fn is_storage(rel: &str) -> bool {
    rel.split('/').next().is_some_and(|top| top.to_ascii_lowercase().starts_with("storage_")) && rel.contains('/')
}

/// The cluster files are large and made by tools; they are copied rather than read.
fn read_lazily(rel_lower: &str, size: u64) -> bool {
    let name = rel_lower.rsplit('/').next().unwrap_or(rel_lower);
    name.starts_with("mapgroupcluster") || size > 24 * 1024 * 1024 && name != "areaflags.map"
}

/// What reading a file makes of it.
pub fn read_file(rel: &str, bytes: &[u8]) -> FileData {
    let lower = rel.to_ascii_lowercase();
    if lower.ends_with("areaflags.map") {
        if let Ok(a) = AreaFlags::parse(bytes) {
            return FileData::AreaFlags(a);
        }
    } else if lower.ends_with(".xml") {
        if let Ok(d) = xml::parse(bytes) {
            return FileData::Xml(d);
        }
    } else if lower.ends_with(".json") {
        if let Ok(d) = JsonDoc::parse(bytes) {
            return FileData::Json(d);
        }
    } else if lower.ends_with(".c") {
        return FileData::Text(TextDoc::parse(bytes));
    }
    FileData::Disk
}

/// A whole mission folder.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mission {
    /// Folder name, as `dayzOffline.chernarusplus`.
    pub name: String,
    /// By path inside the folder, with `/` between folders, in the case the files have.
    pub files: BTreeMap<String, MissionFile>,
    /// Files removed since the mission was last saved into the project.
    pub removed: Vec<String>,
    /// Goes up with every change, so views can follow.
    pub rev: u64,
}

impl Mission {
    /// Reads a mission folder.
    pub fn load(dir: &Path) -> Result<Mission> {
        anyhow::ensure!(dir.is_dir(), "{} is not a folder", dir.display());
        let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "mission".into());
        let mut m = Mission { name, ..Default::default() };
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).with_context(|| d.display().to_string())?.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let rel = p.strip_prefix(dir).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                let lower = rel.to_ascii_lowercase();
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                let data = if is_storage(&rel) || read_lazily(&lower, size) {
                    FileData::Disk
                } else {
                    let bytes = std::fs::read(&p).with_context(|| p.display().to_string())?;
                    read_file(&rel, &bytes)
                };
                m.files.insert(rel, MissionFile { data, origin: Some(p), dirty: false });
            }
        }
        Ok(m)
    }

    /// Whether `dir` looks like a mission folder.
    pub fn is_mission_dir(dir: &Path) -> bool {
        dir.join("init.c").is_file() || dir.join("db").join("types.xml").is_file() || dir.join("cfgeconomycore.xml").is_file()
    }

    /// The key of a file, found ignoring case and `\` / `/`.
    pub fn key(&self, rel: &str) -> Option<String> {
        let want = rel.replace('\\', "/").trim_start_matches("./").to_ascii_lowercase();
        if self.files.contains_key(rel) {
            return Some(rel.to_string());
        }
        self.files.keys().find(|k| k.to_ascii_lowercase() == want).cloned()
    }
    pub fn file(&self, rel: &str) -> Option<&MissionFile> {
        self.key(rel).and_then(|k| self.files.get(&k))
    }
    pub fn xml(&self, rel: &str) -> Option<&XmlDoc> {
        match &self.file(rel)?.data {
            FileData::Xml(d) => Some(d),
            _ => None,
        }
    }
    /// The tree of a file to change; call [`Mission::touch`] once it changed.
    pub fn xml_mut(&mut self, rel: &str) -> Option<&mut XmlDoc> {
        let k = self.key(rel)?;
        match &mut self.files.get_mut(&k)?.data {
            FileData::Xml(d) => Some(d),
            _ => None,
        }
    }
    pub fn json(&self, rel: &str) -> Option<&JsonDoc> {
        match &self.file(rel)?.data {
            FileData::Json(d) => Some(d),
            _ => None,
        }
    }
    pub fn json_mut(&mut self, rel: &str) -> Option<&mut JsonDoc> {
        let k = self.key(rel)?;
        match &mut self.files.get_mut(&k)?.data {
            FileData::Json(d) => Some(d),
            _ => None,
        }
    }
    pub fn text(&self, rel: &str) -> Option<&TextDoc> {
        match &self.file(rel)?.data {
            FileData::Text(d) => Some(d),
            _ => None,
        }
    }
    pub fn text_mut(&mut self, rel: &str) -> Option<&mut TextDoc> {
        let k = self.key(rel)?;
        match &mut self.files.get_mut(&k)?.data {
            FileData::Text(d) => Some(d),
            _ => None,
        }
    }
    pub fn areaflags(&self) -> Option<&AreaFlags> {
        match &self.file("areaflags.map")?.data {
            FileData::AreaFlags(a) => Some(a),
            _ => None,
        }
    }
    pub fn areaflags_mut(&mut self) -> Option<&mut AreaFlags> {
        let k = self.key("areaflags.map")?;
        match &mut self.files.get_mut(&k)?.data {
            FileData::AreaFlags(a) => Some(a),
            _ => None,
        }
    }
    /// Marks a file changed.
    pub fn touch(&mut self, rel: &str) {
        if let Some(k) = self.key(rel) {
            if let Some(f) = self.files.get_mut(&k) {
                f.dirty = true;
            }
        }
        self.rev += 1;
    }
    /// Adds or replaces a file.
    pub fn put(&mut self, rel: &str, data: FileData) {
        let k = self.key(rel).unwrap_or_else(|| rel.replace('\\', "/"));
        self.removed.retain(|r| !r.eq_ignore_ascii_case(&k));
        let origin = self.files.get(&k).and_then(|f| f.origin.clone());
        self.files.insert(k, MissionFile { data, origin, dirty: true });
        self.rev += 1;
    }
    pub fn remove(&mut self, rel: &str) -> Option<MissionFile> {
        let k = self.key(rel)?;
        let f = self.files.remove(&k);
        self.removed.push(k);
        self.rev += 1;
        f
    }
    /// The tree of a file, made with `root` when the mission has none.
    pub fn ensure_xml(&mut self, rel: &str, root: &str) -> &mut XmlDoc {
        if self.xml(rel).is_none() {
            self.put(rel, FileData::Xml(XmlDoc::new(Element::new(root))));
        }
        self.xml_mut(rel).unwrap()
    }
    pub fn is_dirty(&self) -> bool {
        !self.removed.is_empty() || self.files.values().any(|f| f.dirty || f.origin.is_none())
    }

    /// Writes the mission into `dir`. With `adopt`, `dir` becomes where the files live (saving
    /// into the project); without it the files are only copied there (export).
    /// `storage` says whether the persistence folders go along.
    pub fn write(&mut self, dir: &Path, adopt: bool, storage: bool) -> Result<Vec<PathBuf>> {
        std::fs::create_dir_all(dir)?;
        let mut written = Vec::new();
        if adopt {
            for r in std::mem::take(&mut self.removed) {
                let p = dir.join(&r);
                if p.is_file() {
                    std::fs::remove_file(&p).with_context(|| p.display().to_string())?;
                }
            }
        }
        for (rel, f) in self.files.iter_mut() {
            if !storage && !adopt && is_storage(rel) {
                continue;
            }
            let to = dir.join(rel);
            let same = f.origin.as_ref().is_some_and(|o| same_file(o, &to));
            if same && !f.dirty {
                continue;
            }
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            match (&f.data, f.dirty, &f.origin) {
                (FileData::Disk, _, Some(o)) | (_, false, Some(o)) if o.is_file() && !same => {
                    std::fs::copy(o, &to).with_context(|| format!("{} -> {}", o.display(), to.display()))?;
                }
                _ => {
                    let bytes = f.bytes()?;
                    crate::fsutil::atomic_write(&to, &bytes)?;
                }
            }
            written.push(to.clone());
            if adopt {
                f.origin = Some(to);
                f.dirty = false;
            }
        }
        Ok(written)
    }

    // ------------------------------------------------------------ files of the economy

    /// Files `cfgeconomycore.xml` adds of a type (`types`, `events`, `spawnabletypes`, ...),
    /// after the one in `db` the game always reads.
    pub fn ce_files(&self, kind: &str) -> Vec<String> {
        let mut out = Vec::new();
        let base = match kind {
            "types" => Some("db/types.xml"),
            "events" => Some("db/events.xml"),
            "globals" => Some("db/globals.xml"),
            "economy" => Some("db/economy.xml"),
            "messages" => Some("db/messages.xml"),
            "spawnabletypes" => Some("cfgspawnabletypes.xml"),
            "randompresets" => Some("cfgrandompresets.xml"),
            "eventspawns" => Some("cfgeventspawns.xml"),
            _ => None,
        };
        if let Some(k) = base.and_then(|b| self.key(b)) {
            out.push(k);
        }
        if let Some(core) = self.xml("cfgeconomycore.xml") {
            for ce in core.root.children_named("ce") {
                let folder = ce.attr("folder").unwrap_or("").trim_matches(['/', '\\']);
                for f in ce.children_named("file") {
                    if f.attr("type").is_some_and(|t| t.eq_ignore_ascii_case(kind)) {
                        let rel = if folder.is_empty() { f.attr("name").unwrap_or("").to_string() } else { format!("{folder}/{}", f.attr("name").unwrap_or("")) };
                        if let Some(k) = self.key(&rel) {
                            if !out.contains(&k) {
                                out.push(k);
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Territory files `cfgenvironment.xml` names, and any others in `env/`.
    pub fn territory_files(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(env) = self.xml("cfgenvironment.xml") {
            if let Some(t) = env.root.child("territories") {
                for f in t.children_named("file") {
                    if let Some(k) = f.attr("path").and_then(|p| self.key(p)) {
                        if !out.contains(&k) {
                            out.push(k);
                        }
                    }
                }
            }
        }
        for k in self.files.keys() {
            let l = k.to_ascii_lowercase();
            if l.starts_with("env/") && l.ends_with(".xml") && !out.contains(k) && matches!(self.files[k].data, FileData::Xml(_)) {
                out.push(k.clone());
            }
        }
        out
    }

    /// Names a list of `cfglimitsdefinition.xml` holds: `categories`, `tags`, `usageflags`
    /// or `valueflags`.
    pub fn limits(&self, list: &str) -> Vec<String> {
        let Some(d) = self.xml("cfglimitsdefinition.xml") else { return Vec::new() };
        d.root.child(list).map(|l| l.elements().filter_map(|e| e.attr("name").map(str::to_string)).collect()).unwrap_or_default()
    }
    /// User groups of `cfglimitsdefinitionuser.xml` for `usageflags` or `valueflags`.
    pub fn user_limits(&self, list: &str) -> Vec<(String, Vec<String>)> {
        let Some(d) = self.xml("cfglimitsdefinitionuser.xml") else { return Vec::new() };
        d.root.child(list).map(|l| l.children_named("user").map(|u| (u.attr("name").unwrap_or("").to_string(), u.elements().filter_map(|e| e.attr("name").map(str::to_string)).collect())).collect()).unwrap_or_default()
    }
    /// Total size of the persistence files.
    pub fn storage_size(&self) -> (usize, u64) {
        let mut n = 0;
        let mut bytes = 0;
        for (k, f) in &self.files {
            if is_storage(k) {
                n += 1;
                bytes += f.origin.as_ref().and_then(|o| o.metadata().ok()).map(|m| m.len()).unwrap_or(0);
            }
        }
        (n, bytes)
    }
    /// Drops the persistence, so the server starts the economy afresh.
    pub fn wipe_storage(&mut self) -> usize {
        let keys: Vec<String> = self.files.keys().filter(|k| is_storage(k)).cloned().collect();
        for k in &keys {
            self.remove(k);
        }
        keys.len()
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

// ---------------------------------------------------------------- undo

/// Where an element is: its file and the indexes of the nodes from the root down to it.
#[derive(Clone, Debug, PartialEq)]
pub struct NodePath {
    pub file: String,
    pub path: Vec<usize>,
}

fn node_at<'a>(root: &'a mut Element, path: &[usize]) -> Option<&'a mut Element> {
    let mut e = root;
    for &i in path {
        e = match e.children.get_mut(i)? {
            Node::Element(c) => c,
            _ => return None,
        };
    }
    Some(e)
}

/// The path of the `n`-th child element named `name` (or any element when `None`) of `parent`.
pub fn child_path(parent: &Element, index_among_elements: usize) -> Option<usize> {
    parent.children.iter().enumerate().filter(|(_, n)| matches!(n, Node::Element(_))).nth(index_among_elements).map(|(i, _)| i)
}

/// One step of undo inside the mission. Each applies itself and turns into its own inverse.
#[derive(Clone, Debug, PartialEq)]
pub enum MissionEdit {
    /// The element at the path is swapped with this one.
    Element(NodePath, Element),
    /// `Some`: the node goes in at the index of the parent at the path; `None`: the node at the
    /// index comes out.
    Child(NodePath, usize, Option<Node>),
    /// The whole file is swapped; `None` is a file that is not there.
    File(String, Option<Box<MissionFile>>),
    /// A rectangle of area flags is swapped.
    Flags { layer: FlagLayer, rect: [u32; 4], values: Vec<u32> },
    /// The whole mission is swapped (new, imported or removed).
    Whole(Option<Box<Mission>>),
}

impl MissionEdit {
    pub fn bytes(&self) -> usize {
        match self {
            MissionEdit::Element(_, e) => element_size(e),
            MissionEdit::Child(_, _, n) => n.as_ref().map(|n| if let Node::Element(e) = n { element_size(e) } else { 64 }).unwrap_or(32),
            MissionEdit::File(_, f) => f.as_ref().map(|f| match &f.data {
                FileData::Xml(d) => element_size(&d.root),
                FileData::AreaFlags(a) => a.usage.len() + a.value.len(),
                FileData::Text(t) => t.text.len(),
                FileData::Json(_) => 4096,
                FileData::Disk => 64,
            }).unwrap_or(32),
            MissionEdit::Flags { values, .. } => values.len() * 4,
            MissionEdit::Whole(m) => m.as_ref().map(|m| m.files.len() * 65536).unwrap_or(32),
        }
    }
}

fn element_size(e: &Element) -> usize {
    64 + e.name.len() + e.attrs.iter().map(|(k, v)| k.len() + v.len() + 48).sum::<usize>() + e.children.iter().map(|n| match n {
        Node::Element(c) => element_size(c),
        Node::Text(t) | Node::Comment(t) | Node::CData(t) | Node::Other(t) => t.len() + 32,
    }).sum::<usize>()
}

/// Applies a list of edits and turns it into its inverse (the list reverses).
pub fn toggle(mission: &mut Option<Box<Mission>>, edits: &mut Vec<MissionEdit>) {
    for e in edits.iter_mut() {
        toggle_one(mission, e);
    }
    edits.reverse();
}

fn toggle_one(mission: &mut Option<Box<Mission>>, edit: &mut MissionEdit) {
    if let MissionEdit::Whole(m) = edit {
        std::mem::swap(m, mission);
        if let Some(m) = mission.as_mut() {
            m.rev += 1;
        }
        return;
    }
    let Some(m) = mission.as_mut() else { return };
    m.rev += 1;
    match edit {
        MissionEdit::Element(p, el) => {
            if let Some(doc) = m.xml_mut(&p.file) {
                if let Some(target) = node_at(&mut doc.root, &p.path) {
                    std::mem::swap(target, el);
                }
            }
            m.touch(&p.file.clone());
        }
        MissionEdit::Child(p, index, node) => {
            if let Some(doc) = m.xml_mut(&p.file) {
                if let Some(parent) = node_at(&mut doc.root, &p.path) {
                    match node.take() {
                        Some(n) => {
                            let at = (*index).min(parent.children.len());
                            parent.children.insert(at, n);
                        }
                        None => {
                            if *index < parent.children.len() {
                                *node = Some(parent.children.remove(*index));
                            }
                        }
                    }
                }
            }
            m.touch(&p.file.clone());
        }
        MissionEdit::File(rel, f) => {
            let key = m.key(rel).unwrap_or_else(|| rel.clone());
            let now = m.files.remove(&key).map(Box::new);
            match f.take() {
                Some(mut file) => {
                    file.dirty = true;
                    m.removed.retain(|r| r != &key);
                    m.files.insert(key, *file);
                }
                None => m.removed.push(key),
            }
            *f = now;
        }
        MissionEdit::Flags { layer, rect, values } => {
            if let Some(a) = m.areaflags_mut() {
                let [x0, y0, x1, y1] = *rect;
                let now = a.copy_rect(*layer, x0, y0, x1, y1);
                a.paste_rect(*layer, x0, y0, x1, y1, values);
                *values = now;
            }
            m.touch("areaflags.map");
        }
        MissionEdit::Whole(_) => unreachable!(),
    }
}

/// Helps make an undoable change of one element: keep the element as it was, change it, and
/// hand back the edit when it changed.
pub fn element_edit(m: &mut Mission, file: &str, path: &[usize], change: impl FnOnce(&mut Element)) -> Option<MissionEdit> {
    let doc = m.xml_mut(file)?;
    let e = node_at(&mut doc.root, path)?;
    let before = e.clone();
    change(e);
    if *e == before {
        return None;
    }
    m.touch(file);
    Some(MissionEdit::Element(NodePath { file: m.key(file).unwrap_or_else(|| file.to_string()), path: path.to_vec() }, before))
}

/// Adds an element as the child of the element at `path`, at `index` (end when `None`).
pub fn insert_child(m: &mut Mission, file: &str, path: &[usize], index: Option<usize>, el: Element) -> Option<MissionEdit> {
    let doc = m.xml_mut(file)?;
    let parent = node_at(&mut doc.root, path)?;
    let at = index.unwrap_or(parent.children.len()).min(parent.children.len());
    parent.children.insert(at, Node::Element(el));
    m.touch(file);
    Some(MissionEdit::Child(NodePath { file: m.key(file).unwrap_or_else(|| file.to_string()), path: path.to_vec() }, at, None))
}

/// Removes the child node at `index` of the element at `path`.
pub fn remove_child(m: &mut Mission, file: &str, path: &[usize], index: usize) -> Option<MissionEdit> {
    let doc = m.xml_mut(file)?;
    let parent = node_at(&mut doc.root, path)?;
    if index >= parent.children.len() {
        return None;
    }
    let n = parent.children.remove(index);
    m.touch(file);
    Some(MissionEdit::Child(NodePath { file: m.key(file).unwrap_or_else(|| file.to_string()), path: path.to_vec() }, index, Some(n)))
}

/// Replaces a whole file and hands back the edit that undoes it.
pub fn file_edit(m: &mut Mission, rel: &str, change: impl FnOnce(&mut Mission)) -> MissionEdit {
    let key = m.key(rel).unwrap_or_else(|| rel.to_string());
    let before = m.files.get(&key).cloned().map(Box::new);
    change(m);
    m.touch(&key);
    MissionEdit::File(key, before)
}

/// The element at a path of node indexes.
pub fn element_at<'a>(root: &'a Element, path: &[usize]) -> Option<&'a Element> {
    let mut e = root;
    for &i in path {
        e = match e.children.get(i)? {
            Node::Element(c) => c,
            _ => return None,
        };
    }
    Some(e)
}

pub fn element_at_mut<'a>(root: &'a mut Element, path: &[usize]) -> Option<&'a mut Element> {
    node_at(root, path)
}

#[cfg(test)]
mod tests;
