//! Several people working on one map at the same time (experimental).
//!
//! Everyone works on their own copy of the project. The project remembers the last version
//! everyone shares, the *base*, in `.team/versions/<id>`. Sending writes what changed since the
//! base to a `.dzpatch` file in the team folder, a folder that Dropbox, Google Drive, Syncthing
//! or a pen drive carries to the others. Merging takes such a file and applies it with a
//! three-way merge: what only the other person changed is taken over, what only you changed
//! stays, and what both of you changed differently is a conflict to decide.
//!
//! No server is involved: every patch says which version it starts from, and that version is
//! kept by everyone who sent or merged it.
mod lines;
mod mods;

use crate::fsutil::atomic_write;
use crate::maskmod::MaskLayers;
use crate::mission::{FileData, Mission, MissionFile};
use crate::objects::{new_uid, Obj, ObjectStore, LAYER_SHIFT};
use crate::project::{self, SaveParts, MISSION_DIR};
use crate::satmod::{SatLayers, SatModifier};
use crate::world::{World, MASK_MOD_AUTHOR, SAT_MOD_AUTHOR};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

/// Folder of the project with the team state and the shared versions.
pub const TEAM_DIR: &str = ".team";
/// Side of the terrain and mask tiles a patch carries.
pub const TILE: usize = 64;
/// Side of the satellite tiles, the tiles of the satellite modifiers.
const SAT_TILE: usize = crate::satmod::TILE;
pub const PATCH_EXT: &str = "dzpatch";
const MAGIC: &[u8; 8] = b"DZPATCH1";
const FORMAT: u32 = 3;
/// Versions kept besides the base; older ones are removed.
const KEEP_VERSIONS: usize = 4;
const SATELLITE: &str = "satellite.rgb";
/// Settings of the map that are never merged: they make it another map.
const FIXED_SETTINGS: [&str; 3] = ["terrain_size", "terrain_cell", "map_id"];

// ---------------------------------------------------------------------------------------------
// Team state of a project

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamState {
    /// The shared folder patches go to.
    pub folder: String,
    /// The version the changes of this copy are counted from; 0 when the team was not set up.
    pub base: u64,
    /// Versions this copy sent or merged, so their patches are not offered again.
    pub seen: Vec<u64>,
}

impl TeamState {
    pub fn path(project: &Path) -> PathBuf {
        project.join(TEAM_DIR).join("state.json")
    }
    pub fn load(project: &Path) -> TeamState {
        std::fs::read(Self::path(project)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self, project: &Path) -> Result<()> {
        atomic_write(&Self::path(project), &serde_json::to_vec_pretty(self)?)
    }
    pub fn started(&self) -> bool {
        self.base != 0
    }
}

fn versions_dir(project: &Path) -> PathBuf {
    project.join(TEAM_DIR).join("versions")
}
fn version_dir(project: &Path, id: u64) -> PathBuf {
    versions_dir(project).join(format!("{id:016x}"))
}
fn staging_dir(project: &Path) -> PathBuf {
    project.join(TEAM_DIR).join("staging")
}
pub fn has_version(project: &Path, id: u64) -> bool {
    project::is_project(&version_dir(project, id))
}

/// A hard link when the disk allows it, else a copy.
fn link_or_copy(from: &Path, to: &Path) -> Result<()> {
    if std::fs::hard_link(from, to).is_err() {
        std::fs::copy(from, to).with_context(|| format!("{} -> {}", from.display(), to.display()))?;
    }
    Ok(())
}

/// Keeps `w` as the version `id`, with its mission. `satellite` is the file of its satellite
/// image; a project shares it with the version when it did not change.
pub fn save_version(project: &Path, w: &World, id: u64, keep: u64, satellite: Option<&Path>) -> Result<()> {
    let dir = version_dir(project, id);
    if project::is_project(&dir) {
        return Ok(());
    }
    let tmp = versions_dir(project).join(format!("{id:016x}.tmp"));
    let _ = std::fs::remove_dir_all(&tmp);
    project::save(w, &tmp, SaveParts { height: true, mask: true, satellite: false, objects: true })?;
    if let Some(m) = &w.mission {
        m.as_ref().clone().write(&tmp.join(MISSION_DIR), false, false).context("mission")?;
    }
    if let Some(s) = satellite.filter(|s| s.is_file()) {
        link_or_copy(s, &tmp.join(SATELLITE))?;
    }
    std::fs::rename(&tmp, &dir)?;
    // The oldest versions go, but never the one changes are counted from.
    let mut all: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(versions_dir(project))?.flatten()
        .filter(|e| e.path().is_dir() && e.path() != version_dir(project, keep) && e.path() != dir && !e.path().extension().is_some_and(|x| x == "tmp"))
        .map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), e.path()))
        .collect();
    all.sort();
    for (_, p) in &all[..all.len().saturating_sub(KEEP_VERSIONS)] {
        let _ = std::fs::remove_dir_all(p);
    }
    Ok(())
}

pub fn load_version(project: &Path, id: u64) -> Result<World> {
    let dir = version_dir(project, id);
    if !project::is_project(&dir) {
        bail!("This copy does not have the version {id:016x} the changes start from. Ask for a full copy of the project.");
    }
    project::load_with(&dir, false)
}

// ---------------------------------------------------------------------------------------------
// Patches

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Summary {
    pub height_tiles: usize,
    pub mask_tiles: usize,
    pub satellite_tiles: usize,
    pub objects_added: usize,
    pub objects_changed: usize,
    pub objects_removed: usize,
    /// Modifiers, roads, generators, the mission and the other parts that changed.
    pub other: Vec<String>,
    /// Changes a patch cannot carry, named so the receiver knows.
    pub not_carried: Vec<String>,
}

impl Summary {
    pub fn is_empty(&self) -> bool {
        self.height_tiles + self.mask_tiles + self.satellite_tiles + self.objects_added + self.objects_changed + self.objects_removed == 0 && self.other.is_empty()
    }
}

/// The readable start of a patch file.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PatchHeader {
    pub format: u32,
    /// Version the changes start from.
    pub base: u64,
    /// Version the sender has after them.
    pub result: u64,
    pub author: String,
    pub time: String,
    pub map_id: String,
    pub summary: Summary,
}

/// A placed object as a patch carries it: the model by path and the layer by name, since
/// both are numbered differently in every copy.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct ObjRec {
    uid: u64,
    model: u32,
    /// x, z, elevation, yaw, pitch, roll, scale.
    p: [f32; 7],
    /// Flags without the layer.
    f: u32,
    layer: u32,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct ListDiff {
    /// Added or changed elements, with object references as lasting identities.
    set: Vec<Value>,
    removed: Vec<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct TileRec {
    x: u32,
    y: u32,
    /// Byte offset in the binary part.
    at: usize,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct SatBody {
    size: [usize; 2],
    /// Pixels of the image under the modifiers, by tile.
    tiles: Vec<TileRec>,
    /// Changed and added modifiers with only their changed tiles, as satellite layers without
    /// original pixels.
    mods: Option<(usize, usize)>,
    mods_delta: Vec<mods::ModDelta>,
    removed: Vec<u64>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct FileRec {
    path: String,
    at: usize,
    len: usize,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct MissionBody {
    name: String,
    /// Changed and added files, whole.
    files: Vec<FileRec>,
    removed: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Body {
    terrain: [usize; 2],
    mask: [usize; 2],
    models: Vec<String>,
    layers: Vec<String>,
    objects: Vec<ObjRec>,
    removed: Vec<u64>,
    height: Vec<TileRec>,
    /// The mask as painted, under its modifiers.
    mask_tiles: Vec<TileRec>,
    /// Changed and added mask modifiers with only their changed tiles, as mask layers.
    mask_mods: Option<(usize, usize)>,
    mask_mods_delta: Vec<mods::ModDelta>,
    mask_mods_removed: Vec<u64>,
    lists: BTreeMap<String, ListDiff>,
    parts: BTreeMap<String, Value>,
    satellite: Option<SatBody>,
    mission: Option<MissionBody>,
    /// Owners of the object layers by layer name, when they changed.
    layer_owners: Option<BTreeMap<String, String>>,
    /// The map settings, when they changed.
    meta: Option<Value>,
}

pub struct Patch {
    pub header: PatchHeader,
    body: Body,
    bin: Vec<u8>,
}

impl Patch {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let header = serde_json::to_vec(&self.header)?;
        let body = serde_json::to_vec(&self.body)?;
        let mut out = MAGIC.to_vec();
        out.extend((header.len() as u32).to_le_bytes());
        out.extend(&header);
        let mut z = flate2::write::DeflateEncoder::new(out, flate2::Compression::default());
        z.write_all(&(body.len() as u64).to_le_bytes())?;
        z.write_all(&body)?;
        z.write_all(&self.bin)?;
        Ok(z.finish()?)
    }
    fn split(b: &[u8]) -> Result<(PatchHeader, usize)> {
        ensure!(b.starts_with(MAGIC), "Not a patch of DZ World Editor");
        let n = u32::from_le_bytes(b.get(8..12).context("Truncated patch")?.try_into()?) as usize;
        let header: PatchHeader = serde_json::from_slice(b.get(12..12 + n).context("Truncated patch")?)?;
        ensure!(header.format <= FORMAT, "The patch was made by a newer version of the editor");
        ensure!(header.format == FORMAT, "The patch was made by an older version of the editor; ask for it to be sent again");
        Ok((header, 12 + n))
    }
    pub fn from_bytes(b: &[u8]) -> Result<Patch> {
        let (header, at) = Self::split(b)?;
        let mut rest = Vec::new();
        flate2::read::DeflateDecoder::new(&b[at..]).read_to_end(&mut rest).context("Damaged patch")?;
        ensure!(rest.len() >= 8, "Truncated patch");
        let n = u64::from_le_bytes(rest[..8].try_into()?) as usize;
        ensure!(rest.len() >= 8 + n, "Truncated patch");
        let body: Body = serde_json::from_slice(&rest[8..8 + n])?;
        let bin = rest[8 + n..].to_vec();
        Ok(Patch { header, body, bin })
    }
    /// Reads only the header, to list patches quickly.
    pub fn read_header(path: &Path) -> Result<PatchHeader> {
        let mut f = std::fs::File::open(path)?;
        let mut start = [0u8; 12];
        f.read_exact(&mut start)?;
        ensure!(&start[..8] == MAGIC, "Not a patch of DZ World Editor");
        let n = u32::from_le_bytes(start[8..12].try_into()?) as usize;
        let mut b = start.to_vec();
        b.resize(12 + n, 0);
        f.read_exact(&mut b[12..])?;
        let header: PatchHeader = serde_json::from_slice(&b[12..])?;
        Ok(header)
    }
    fn f32s(&self, at: usize, n: usize) -> Result<Vec<f32>> {
        let b = self.bin.get(at..at + n * 4).context("Truncated patch")?;
        Ok(crate::fsutil::bytes_to_f32s(b))
    }
    fn u8s(&self, at: usize, n: usize) -> Result<&[u8]> {
        self.bin.get(at..at + n).context("Truncated patch")
    }
}

// ---------------------------------------------------------------------------------------------
// The lists and parts that are merged

/// Lists whose elements have ids; the second name is their field of object references.
const LISTS: [(&str, &str); 3] = [("modifiers", "attached"), ("routes", "objects"), ("generators", "objects")];
/// Parts merged whole.
const PARTS: [&str; 5] = ["surfaces", "shapes", "keypoints", "holes", "road_parts"];

fn list_values(w: &World, name: &str) -> Vec<Value> {
    let v = match name {
        "modifiers" => serde_json::to_value(&w.modifiers),
        "routes" => serde_json::to_value(&w.routes),
        _ => serde_json::to_value(&w.generators),
    };
    let field = LISTS.iter().find(|l| l.0 == name).map(|l| l.1).unwrap_or("");
    match v {
        Ok(Value::Array(mut a)) => {
            for e in &mut a {
                refs_to_uids(e, field, &w.objects);
                // Who made it travels with it.
                let author = w.author(id_of(e));
                if !author.is_empty() {
                    e["author"] = Value::from(author);
                }
            }
            a
        }
        _ => Vec::new(),
    }
}

fn set_list(w: &mut World, name: &str, values: Vec<Value>, ids: &HashMap<u64, u32>) -> Result<()> {
    let field = LISTS.iter().find(|l| l.0 == name).map(|l| l.1).unwrap_or("");
    let values: Vec<Value> = values.into_iter().map(|mut v| { refs_from_uids(&mut v, field, ids); v }).collect();
    for v in &values {
        if let Some(a) = v.get("author").and_then(Value::as_str) {
            w.authors.get_or_insert_with(Default::default).insert(id_of(v), a.to_string());
        }
    }
    let v = Value::Array(values);
    match name {
        "modifiers" => w.modifiers = serde_json::from_value(v).context("modifiers")?,
        "routes" => w.routes = serde_json::from_value(v).context("roads")?,
        _ => w.generators = serde_json::from_value(v).context("generators")?,
    }
    Ok(())
}

fn part_value(w: &World, name: &str) -> Value {
    match name {
        "surfaces" => serde_json::to_value(&w.layers),
        "shapes" => serde_json::to_value(&w.shape_layers),
        "keypoints" => serde_json::to_value(&w.keypoints),
        "holes" => serde_json::to_value(&w.holes),
        _ => serde_json::to_value((&w.road_removed, &w.road_added)),
    }
    .unwrap_or(Value::Null)
}

fn set_part(w: &mut World, name: &str, v: Value) -> Result<()> {
    match name {
        "surfaces" => w.layers = serde_json::from_value(v)?,
        "shapes" => w.shape_layers = serde_json::from_value(v)?,
        "keypoints" => w.keypoints = serde_json::from_value(v)?,
        "holes" => {
            w.holes = serde_json::from_value(v)?;
            w.holes_rev += 1;
        }
        _ => (w.road_removed, w.road_added) = serde_json::from_value(v)?,
    }
    Ok(())
}

fn id_of(v: &Value) -> u64 {
    v.get("id").and_then(Value::as_u64).unwrap_or(0)
}

fn name_of(v: &Value) -> String {
    v.get("name").and_then(Value::as_str).unwrap_or_default().to_string()
}

fn refs_to_uids(v: &mut Value, field: &str, store: &ObjectStore) {
    match v.get_mut(field) {
        Some(Value::Array(a)) => {
            *a = a.iter().filter_map(|x| x.as_u64()).map(|id| store.uid(id as u32)).filter(|u| *u != 0).map(Value::from).collect();
        }
        Some(x) if x.is_u64() => {
            let uid = store.uid(x.as_u64().unwrap() as u32);
            *x = if uid == 0 { Value::Null } else { Value::from(uid) };
        }
        _ => {}
    }
}

fn refs_from_uids(v: &mut Value, field: &str, ids: &HashMap<u64, u32>) {
    match v.get_mut(field) {
        Some(Value::Array(a)) => {
            *a = a.iter().filter_map(|x| x.as_u64()).filter_map(|u| ids.get(&u)).map(|id| Value::from(*id)).collect();
        }
        Some(x) if x.is_u64() => {
            *x = ids.get(&x.as_u64().unwrap()).map(|id| Value::from(*id)).unwrap_or(Value::Null);
        }
        _ => {}
    }
}

/// What changed in a list of elements with ids: the changed and added ones, and the ids gone.
fn list_diff<T: Clone + PartialEq>(base: &[T], now: &[T], id: impl Fn(&T) -> u64) -> (Vec<T>, Vec<u64>) {
    let old: HashMap<u64, &T> = base.iter().map(|v| (id(v), v)).collect();
    let new_ids: HashSet<u64> = now.iter().map(&id).collect();
    let set = now.iter().filter(|v| old.get(&id(v)) != Some(v)).cloned().collect();
    let mut removed: Vec<u64> = old.keys().filter(|i| !new_ids.contains(i)).copied().collect();
    removed.sort();
    (set, removed)
}

/// A stable number for a name, to tell conflicts about named things apart.
fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// An object with its model and layer by name, comparable between copies.
#[derive(Clone, Debug, PartialEq)]
struct Norm {
    pos: [u32; 3],
    rot: [u32; 3],
    scale: u32,
    model: String,
    flags: u32,
    layer: String,
}

fn layer_name(w: &World, i: u16) -> String {
    w.object_layers.get(i as usize).map(|l| l.name.clone()).unwrap_or_default()
}

fn norm(w: &World, o: &Obj) -> Norm {
    Norm {
        pos: [o.x.to_bits(), o.z.to_bits(), o.elevation.to_bits()],
        rot: [o.yaw.to_bits(), o.pitch.to_bits(), o.roll.to_bits()],
        scale: o.scale.to_bits(),
        model: w.objects.model_path(o).to_string(),
        flags: o.flags & ((1 << LAYER_SHIFT) - 1),
        layer: layer_name(w, o.layer()),
    }
}

/// Owners of the layers of a world, by layer name.
fn owners(w: &World) -> BTreeMap<String, String> {
    w.object_layers.iter().filter(|l| !l.deleted && !l.owner.is_empty()).map(|l| (l.name.clone(), l.owner.clone())).collect()
}

/// What stays when nothing is chosen: the version of the one who owns the thing.
fn owner_default(owner: &str, author: &str) -> Choice {
    if !owner.is_empty() && owner == author { Choice::Theirs } else { Choice::Mine }
}

/// The layer called `name` in `w`, added when there is none.
fn layer_index(w: &mut World, name: &str) -> u16 {
    match w.object_layers.iter().position(|l| !l.deleted && l.name == name) {
        Some(i) => i as u16,
        None if name.is_empty() => 0,
        None => w.add_object_layer(name),
    }
}

fn from_norm(w: &mut World, n: &Norm) -> Obj {
    let f = |b: u32| f32::from_bits(b);
    let mut o = Obj {
        x: f(n.pos[0]), z: f(n.pos[1]), elevation: f(n.pos[2]), yaw: f(n.rot[0]), pitch: f(n.rot[1]), roll: f(n.rot[2]),
        scale: f(n.scale), model: w.objects.intern(&n.model), flags: n.flags,
    };
    let layer = layer_index(w, &n.layer);
    o.set_layer(layer);
    o
}

/// The files of a mission by their name in lower case: the name as written and the bytes.
/// The persistence of the server is left out.
fn mission_files(m: Option<&Mission>) -> Result<BTreeMap<String, (String, Vec<u8>)>> {
    let mut out = BTreeMap::new();
    let Some(m) = m else { return Ok(out) };
    for (rel, f) in &m.files {
        if crate::mission::is_storage(rel) {
            continue;
        }
        out.insert(rel.to_ascii_lowercase(), (rel.clone(), f.bytes().with_context(|| rel.clone())?));
    }
    Ok(out)
}

/// Files merged line by line; the others are taken whole.
fn is_text(rel: &str, bytes: &[&[u8]]) -> bool {
    let lower = rel.to_ascii_lowercase();
    [".xml", ".json", ".c", ".txt", ".cfg", ".csv", ".ini"].iter().any(|x| lower.ends_with(x)) && bytes.iter().all(|b| std::str::from_utf8(b).is_ok())
}

// ---------------------------------------------------------------------------------------------
// The satellite image, read and written a band of tiles at a time: it can be gigabytes.

/// The satellite image of a world as it is without its modifiers, a band of tile rows at a time.
struct SatReader<'a> {
    w: &'a World,
    file: Option<BufReader<std::fs::File>>,
    width: usize,
    height: usize,
}

impl<'a> SatReader<'a> {
    /// The image is read from `dir` when the world holds only its size.
    fn new(w: &'a World, dir: Option<&Path>) -> Result<Option<Self>> {
        let Some(img) = &w.satellite else { return Ok(None) };
        let (width, height) = (img.width, img.height);
        let file = if img.data.len() == width * height * 3 {
            None
        } else {
            let p = dir.context("The satellite image is not loaded")?.join(SATELLITE);
            Some(BufReader::with_capacity(1 << 20, std::fs::File::open(&p).with_context(|| p.display().to_string())?))
        };
        Ok(Some(SatReader { w, file, width, height }))
    }
    fn rows(&self) -> usize {
        self.height.div_ceil(SAT_TILE)
    }
    /// The pixels of tile row `ty`, full width; rows must be read in order.
    fn band(&mut self, ty: usize) -> Result<Vec<u8>> {
        let (y0, y1) = (ty * SAT_TILE, ((ty + 1) * SAT_TILE).min(self.height));
        let mut out = match &mut self.file {
            None => self.w.satellite.as_ref().unwrap().data[y0 * self.width * 3..y1 * self.width * 3].to_vec(),
            Some(f) => {
                let mut b = vec![0u8; (y1 - y0) * self.width * 3];
                f.read_exact(&mut b).context("satellite image")?;
                b
            }
        };
        // Under the modifiers: the original pixels they keep.
        for tx in 0..self.width.div_ceil(SAT_TILE) {
            let Some(base) = self.w.sat.base_tile((tx as u16, ty as u16)) else { continue };
            let (x0, w) = (tx * SAT_TILE, SAT_TILE.min(self.width - tx * SAT_TILE));
            for y in y0..y1 {
                let to = ((y - y0) * self.width + x0) * 3;
                let from = (y - y0) * SAT_TILE * 3;
                out[to..to + w * 3].copy_from_slice(&base[from..from + w * 3]);
            }
        }
        Ok(out)
    }
}

/// The pixels of tile `tx` of a band, row after row.
fn band_tile(band: &[u8], width: usize, tx: usize) -> Vec<u8> {
    let (x0, w) = (tx * SAT_TILE, SAT_TILE.min(width - tx * SAT_TILE));
    let rows = band.len() / (width * 3);
    let mut out = Vec::with_capacity(w * rows * 3);
    for y in 0..rows {
        let s = (y * width + x0) * 3;
        out.extend_from_slice(&band[s..s + w * 3]);
    }
    out
}

/// Where the merged satellite image goes.
enum SatOut {
    Memory(Vec<u8>),
    File(BufWriter<std::fs::File>),
    Nowhere,
}

impl SatOut {
    fn write(&mut self, b: &[u8]) -> Result<()> {
        match self {
            SatOut::Memory(v) => v.extend_from_slice(b),
            SatOut::File(f) => f.write_all(b)?,
            SatOut::Nowhere => {}
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Making a patch

fn tiles(cols: usize, rows: usize) -> impl Iterator<Item = (usize, usize, usize, usize, usize, usize)> {
    let (tx, ty) = (cols.div_ceil(TILE), rows.div_ceil(TILE));
    (0..ty).flat_map(move |y| (0..tx).map(move |x| (x, y, x * TILE, y * TILE, ((x + 1) * TILE).min(cols), ((y + 1) * TILE).min(rows))))
}

/// What changed from `base` to `now`. The folders are where the satellite image of a world is
/// read from when the world holds only its size.
pub fn diff(base: &World, base_dir: Option<&Path>, now: &World, now_dir: Option<&Path>, header: PatchHeader) -> Result<Patch> {
    ensure!(base.base.cols == now.base.cols && base.base.rows == now.base.rows, "The terrain size changed since the shared version");
    ensure!(base.mask.width == now.mask.width && base.mask.height == now.mask.height, "The mask size changed since the shared version");
    let mut body = Body { terrain: [now.base.cols, now.base.rows], mask: [now.mask.width, now.mask.height], ..Default::default() };
    let mut bin = Vec::new();
    let mut summary = Summary::default();
    // Terrain as sculpted, before modifiers: the modifiers travel on their own.
    let (cols, rows) = (now.base.cols, now.base.rows);
    for (tx, ty, x0, y0, x1, y1) in tiles(cols, rows) {
        let changed = (y0..y1).any(|y| (x0..x1).any(|x| base.base.data[y * cols + x].to_bits() != now.base.data[y * cols + x].to_bits()));
        if changed {
            body.height.push(TileRec { x: tx as u32, y: ty as u32, at: bin.len() });
            for y in y0..y1 {
                bin.extend(crate::fsutil::f32s_to_bytes(&now.base.data[y * cols + x0..y * cols + x1]));
            }
        }
    }
    // The mask as painted, and its modifiers on their own.
    let (bm, nm) = (base.mask_original(), now.mask_original());
    let (mw, mh) = (nm.width, nm.height);
    for (tx, ty, x0, y0, x1, y1) in tiles(mw, mh) {
        let changed = (y0..y1).any(|y| bm.data[y * mw + x0..y * mw + x1] != nm.data[y * mw + x0..y * mw + x1]);
        if changed {
            body.mask_tiles.push(TileRec { x: tx as u32, y: ty as u32, at: bin.len() });
            for y in y0..y1 {
                bin.extend(&nm.data[y * mw + x0..y * mw + x1]);
            }
        }
    }
    let (set, deltas, removed) = mods::diff(&base.mask_mods.modifiers, &now.mask_mods.modifiers, |id| now.mask_mod_author(id).to_string());
    if !set.is_empty() || !removed.is_empty() {
        summary.other.push("mask_modifiers".into());
        if !set.is_empty() {
            let b = MaskLayers::from_modifiers(set).to_bytes();
            body.mask_mods = Some((bin.len(), b.len()));
            bin.extend(b);
        }
        body.mask_mods_delta = deltas;
        body.mask_mods_removed = removed;
    }
    summary.height_tiles = body.height.len();
    summary.mask_tiles = body.mask_tiles.len();
    diff_satellite(base, base_dir, now, now_dir, &mut body, &mut bin, &mut summary)?;
    // Objects by their lasting identity.
    let before = base.objects.ids_by_uid();
    let after = now.objects.ids_by_uid();
    let mut models: HashMap<String, u32> = HashMap::new();
    let mut layers: HashMap<String, u32> = HashMap::new();
    for (&uid, &id) in &after {
        let o = now.objects.get(id).unwrap();
        let n = norm(now, o);
        match before.get(&uid) {
            Some(&b) if norm(base, base.objects.get(b).unwrap()) == n => continue,
            Some(_) => summary.objects_changed += 1,
            None => summary.objects_added += 1,
        }
        let next = models.len() as u32;
        let model = *models.entry(n.model.clone()).or_insert_with(|| { body.models.push(n.model.clone()); next });
        let next = layers.len() as u32;
        let layer = *layers.entry(n.layer.clone()).or_insert_with(|| { body.layers.push(n.layer.clone()); next });
        body.objects.push(ObjRec { uid, model, p: [o.x, o.z, o.elevation, o.yaw, o.pitch, o.roll, o.scale], f: n.flags, layer });
    }
    body.objects.sort_by_key(|o| o.uid);
    body.removed = before.keys().filter(|u| !after.contains_key(u)).copied().collect();
    body.removed.sort();
    summary.objects_removed = body.removed.len();
    for (name, _) in LISTS {
        let (set, removed) = list_diff(&list_values(base, name), &list_values(now, name), id_of);
        if !set.is_empty() || !removed.is_empty() {
            summary.other.push(name.to_string());
            body.lists.insert(name.to_string(), ListDiff { set, removed });
        }
    }
    for name in PARTS {
        let v = part_value(now, name);
        if v != part_value(base, name) {
            summary.other.push(name.to_string());
            body.parts.insert(name.to_string(), v);
        }
    }
    // The mission, file by file.
    let (bf, nf) = (mission_files(base.mission.as_deref())?, mission_files(now.mission.as_deref())?);
    let mut mb = MissionBody { name: now.mission.as_ref().or(base.mission.as_ref()).map(|m| m.name.clone()).unwrap_or_default(), ..Default::default() };
    for (key, (rel, bytes)) in &nf {
        if bf.get(key).map(|f| &f.1) != Some(bytes) {
            mb.files.push(FileRec { path: rel.clone(), at: bin.len(), len: bytes.len() });
            bin.extend(bytes);
        }
    }
    mb.removed = bf.iter().filter(|(k, _)| !nf.contains_key(*k)).map(|(_, f)| f.0.clone()).collect();
    if !mb.files.is_empty() || !mb.removed.is_empty() {
        summary.other.push("mission".into());
        body.mission = Some(mb);
    }
    let (bo, no) = (owners(base), owners(now));
    if bo != no {
        summary.other.push("layer_owners".into());
        body.layer_owners = Some(no);
    }
    if now.meta != base.meta {
        if FIXED_SETTINGS.iter().any(|k| serde_json::to_value(&now.meta).ok().map(|v| v[k].clone()) != serde_json::to_value(&base.meta).ok().map(|v| v[k].clone())) {
            summary.not_carried.push("map_id".into());
        }
        body.meta = Some(serde_json::to_value(&now.meta)?);
        summary.other.push("map_settings".into());
    }
    Ok(Patch { header: PatchHeader { summary, ..header }, body, bin })
}

#[allow(clippy::too_many_arguments)]
fn diff_satellite(base: &World, base_dir: Option<&Path>, now: &World, now_dir: Option<&Path>, body: &mut Body, bin: &mut Vec<u8>, summary: &mut Summary) -> Result<()> {
    let (Some(mut b), Some(mut n)) = (SatReader::new(base, base_dir)?, SatReader::new(now, now_dir)?) else {
        if now.satellite.is_some() != base.satellite.is_some() {
            summary.not_carried.push("satellite_size".into());
        }
        return Ok(());
    };
    if (b.width, b.height) != (n.width, n.height) {
        summary.not_carried.push("satellite_size".into());
        return Ok(());
    }
    let mut sb = SatBody { size: [n.width, n.height], ..Default::default() };
    for ty in 0..n.rows() {
        let (bb, nb) = (b.band(ty)?, n.band(ty)?);
        if bb == nb {
            continue;
        }
        for tx in 0..n.width.div_ceil(SAT_TILE) {
            let t = band_tile(&nb, n.width, tx);
            if band_tile(&bb, n.width, tx) != t {
                sb.tiles.push(TileRec { x: tx as u32, y: ty as u32, at: bin.len() });
                bin.extend(t);
            }
        }
    }
    let (set, deltas, removed) = mods::diff(&base.sat.modifiers, &now.sat.modifiers, |id| now.sat_mod_author(id).to_string());
    if !set.is_empty() {
        let bytes = SatLayers::from_parts(set, HashMap::new()).to_bytes();
        sb.mods = Some((bin.len(), bytes.len()));
        bin.extend(bytes);
    }
    sb.mods_delta = deltas;
    sb.removed = removed;
    summary.satellite_tiles = sb.tiles.len();
    if sb.mods.is_some() || !sb.removed.is_empty() {
        summary.other.push("sat_modifiers".into());
    }
    if !sb.tiles.is_empty() || sb.mods.is_some() || !sb.removed.is_empty() {
        body.satellite = Some(sb);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Merging

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum ConflictKey {
    Height(u32, u32),
    Mask(u32, u32),
    Satellite(u32, u32),
    Object(u64),
    Element(&'static str, u64),
    /// A tile of a mask or satellite modifier: the list, the modifier, the tile.
    ModTile(&'static str, u64, u16, u16),
    Part(&'static str),
    /// A file of the mission, by the hash of its name.
    Mission(u64),
    /// A map setting, by the hash of its name.
    Setting(u64),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Mine,
    Theirs,
    /// Both changes added together; only for the terrain.
    Both,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConflictKind {
    /// Both sculpted the same cells.
    Terrain,
    /// Both painted the same pixels with different surfaces.
    Surface,
    /// Both changed the same pixels of the satellite image.
    Satellite,
    /// Both changed the same thing differently.
    Changed,
    /// Both changed the same lines of a mission file.
    Lines,
    /// You deleted what the other person changed.
    DeletedHere,
    /// The other person deleted what you changed.
    DeletedThere,
}

#[derive(Clone, Debug)]
pub struct Conflict {
    pub key: ConflictKey,
    pub kind: ConflictKind,
    /// What it is about: a model path, a road name, a file, a setting.
    pub what: String,
    /// Where on the map, in metres, when it has a place.
    pub at: Option<[f32; 2]>,
    /// What happens when nothing is chosen.
    pub default: Choice,
}

#[derive(Default, Debug)]
pub struct Merged {
    pub conflicts: Vec<Conflict>,
    /// Counts of what was taken over from the other person.
    pub taken: Summary,
    /// Changes of the patch that could not be applied, by name.
    pub skipped: Vec<String>,
}

impl Merged {
    fn conflict(&mut self, key: ConflictKey, kind: ConflictKind, what: String, at: Option<[f32; 2]>, default: Choice) {
        self.conflicts.push(Conflict { key, kind, what, at, default });
    }
}

fn choice(choices: &HashMap<ConflictKey, Choice>, key: ConflictKey, default: Choice) -> Choice {
    choices.get(&key).copied().unwrap_or(default)
}

/// Where the worlds of a merge keep what is not in memory.
#[derive(Clone, Copy, Default)]
pub struct Dirs<'a> {
    /// Folder of the base version, for its satellite image.
    pub base: Option<&'a Path>,
    /// Folder of the project merged into, for its satellite image.
    pub mine: Option<&'a Path>,
    /// Where the merged satellite image is written when `mine` holds only its size; without
    /// it the image is only checked.
    pub satellite_out: Option<&'a Path>,
    /// A folder for mission files too large to be read, while they wait to be saved.
    pub staging: Option<&'a Path>,
}

/// Merges a list of elements with ids: what only one side changed is kept, the rest asked.
#[allow(clippy::too_many_arguments)]
fn merge_list<T: Clone + PartialEq>(
    list: &'static str, base: &[T], mine: &mut Vec<T>, set: &[T], removed: &[u64], id: impl Fn(&T) -> u64, what: impl Fn(&T) -> String,
    place: impl Fn(&T) -> Option<[f32; 2]>, default: impl Fn(&T) -> Choice, choices: &HashMap<ConflictKey, Choice>, out: &mut Merged,
) -> bool {
    let base: HashMap<u64, &T> = base.iter().map(|v| (id(v), v)).collect();
    let mut changed = false;
    for t in set {
        let i = id(t);
        let key = ConflictKey::Element(list, i);
        let pos = mine.iter().position(|v| id(v) == i);
        match (base.get(&i), pos) {
            (_, Some(p)) if mine[p] == *t => {}
            (None, None) => {
                mine.push(t.clone());
                changed = true;
            }
            (Some(bv), Some(p)) if mine[p] == **bv => {
                mine[p] = t.clone();
                changed = true;
            }
            (Some(_), None) => {
                let def = default(t);
                out.conflict(key, ConflictKind::DeletedHere, what(t), place(t), def);
                if choice(choices, key, def) == Choice::Theirs {
                    mine.push(t.clone());
                    changed = true;
                }
            }
            (_, Some(p)) => {
                let def = default(t);
                out.conflict(key, ConflictKind::Changed, what(t), place(t), def);
                if choice(choices, key, def) == Choice::Theirs {
                    mine[p] = t.clone();
                    changed = true;
                }
            }
        }
    }
    for i in removed {
        let Some(p) = mine.iter().position(|v| id(v) == *i) else { continue };
        let key = ConflictKey::Element(list, *i);
        if base.get(i).is_some_and(|b| **b == mine[p]) {
            mine.remove(p);
            changed = true;
        } else {
            let def = default(&mine[p]);
            out.conflict(key, ConflictKind::DeletedThere, what(&mine[p]), place(&mine[p]), def);
            if choice(choices, key, def) == Choice::Theirs {
                mine.remove(p);
                changed = true;
            }
        }
    }
    changed
}

/// Merges map settings key by key, so a new name and another sea level both stay.
fn merge_value(path: &str, b: &Value, m: &mut Value, t: &Value, choices: &HashMap<ConflictKey, Choice>, out: &mut Merged) {
    if t == b || *m == *t {
        return;
    }
    if *m == *b {
        *m = t.clone();
        return;
    }
    if let (Value::Object(bo), Value::Object(to), Value::Object(mo)) = (b, t, &mut *m) {
        let keys: BTreeSet<String> = bo.keys().chain(to.keys()).chain(mo.keys()).cloned().collect();
        for k in keys {
            let (bk, tk) = (bo.get(&k).cloned().unwrap_or(Value::Null), to.get(&k).cloned().unwrap_or(Value::Null));
            let mut mk = mo.get(&k).cloned().unwrap_or(Value::Null);
            let sub = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
            merge_value(&sub, &bk, &mut mk, &tk, choices, out);
            if mk.is_null() && !mo.contains_key(&k) {
                continue;
            }
            mo.insert(k, mk);
        }
        return;
    }
    let key = ConflictKey::Setting(hash(path));
    out.conflict(key, ConflictKind::Changed, path.to_string(), None, Choice::Mine);
    if choice(choices, key, Choice::Mine) == Choice::Theirs {
        *m = t.clone();
    }
}

/// Applies `patch`, made from `base`, to `mine`.
pub fn merge(base: &World, patch: &Patch, mine: &mut World, dirs: Dirs, choices: &HashMap<ConflictKey, Choice>) -> Result<Merged> {
    let b = &patch.body;
    ensure!(b.terrain == [mine.base.cols, mine.base.rows] && b.terrain == [base.base.cols, base.base.rows], "The patch is for a terrain of another size");
    ensure!(b.mask == [mine.mask.width, mine.mask.height], "The patch is for a mask of another size");
    let mut out = Merged::default();
    let cell = mine.base.cell;
    // The satellite image, before anything else changes.
    if let Some(sb) = &b.satellite {
        if let Some((layers, pixels, taken)) = merge_satellite(base, patch, sb, mine, dirs, choices, &mut out)? {
            mine.sat = layers;
            if let (Some(p), Some(img)) = (pixels, mine.satellite.as_mut()) {
                img.data = p;
            }
            note_taken(mine, SAT_MOD_AUTHOR, taken);
        }
    }
    // Terrain: the change of each cell since the base.
    let cols = mine.base.cols;
    for t in &b.height {
        let (x0, y0) = (t.x as usize * TILE, t.y as usize * TILE);
        let (x1, y1) = ((x0 + TILE).min(cols), (y0 + TILE).min(mine.base.rows));
        let w = x1 - x0;
        let theirs = patch.f32s(t.at, w * (y1 - y0))?;
        let key = ConflictKey::Height(t.x, t.y);
        let pick = choice(choices, key, Choice::Both);
        let mut clash = false;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * cols + x;
                let (bv, mv, tv) = (base.base.data[i], mine.base.data[i], theirs[(y - y0) * w + x - x0]);
                if tv.to_bits() == bv.to_bits() || tv.to_bits() == mv.to_bits() {
                    continue;
                }
                if mv.to_bits() == bv.to_bits() {
                    mine.base.data[i] = tv;
                    continue;
                }
                clash = true;
                mine.base.data[i] = match pick {
                    Choice::Mine => mv,
                    Choice::Theirs => tv,
                    Choice::Both => mv + (tv - bv),
                };
            }
        }
        out.taken.height_tiles += 1;
        if clash {
            let at = [(x0 + x1) as f32 * 0.5 * cell, (y0 + y1) as f32 * 0.5 * cell];
            out.conflict(key, ConflictKind::Terrain, String::new(), Some(at), Choice::Both);
        }
    }
    merge_mask(base, patch, mine, choices, &mut out)?;
    let author = patch.header.author.as_str();
    // What was there before anyone was its author stays so, also in a copy that has no authors yet.
    if mine.authors.is_none() && !b.lists.is_empty() {
        mine.authors = Some(mine.modifiers.iter().map(|m| m.id).chain(mine.routes.iter().map(|r| r.id)).chain(mine.generators.iter().map(|g| g.id)).map(|id| (id, String::new())).collect());
    }
    // Owners of layers, before the objects that go to them.
    let their_owners = b.layer_owners.clone().unwrap_or_default();
    if let Some(theirs) = &b.layer_owners {
        let before = owners(base);
        for (name, owner) in theirs {
            match mine.object_layers.iter().position(|l| !l.deleted && &l.name == name) {
                Some(i) => {
                    let l = &mut mine.object_layers[i];
                    if l.owner != *owner && l.owner == before.get(name).cloned().unwrap_or_default() {
                        l.owner = owner.clone();
                    }
                }
                None => {
                    let i = mine.add_object_layer(name);
                    mine.object_layers[i as usize].owner = owner.clone();
                }
            }
        }
        out.taken.other.push("layer_owners".into());
    }
    let owner_of = |w: &World, layer: &str| -> String {
        w.object_layers.iter().find(|l| !l.deleted && l.name == layer).map(|l| l.owner.clone()).filter(|o| !o.is_empty()).or_else(|| their_owners.get(layer).cloned()).unwrap_or_default()
    };
    // Objects, field by field: a move by one and a new layer by the other both stay.
    let base_ids = base.objects.ids_by_uid();
    let mut mine_ids = mine.objects.ids_by_uid();
    for r in &b.objects {
        let theirs = Norm {
            pos: [r.p[0].to_bits(), r.p[1].to_bits(), r.p[2].to_bits()],
            rot: [r.p[3].to_bits(), r.p[4].to_bits(), r.p[5].to_bits()],
            scale: r.p[6].to_bits(),
            model: b.models.get(r.model as usize).cloned().context("Damaged patch")?,
            flags: r.f,
            layer: b.layers.get(r.layer as usize).cloned().unwrap_or_default(),
        };
        let at = Some([r.p[0], r.p[1]]);
        let key = ConflictKey::Object(r.uid);
        let what = theirs.model.clone();
        let def = owner_default(&owner_of(mine, &theirs.layer), author);
        let base_n = base_ids.get(&r.uid).map(|&id| norm(base, base.objects.get(id).unwrap()));
        let mine_n = mine_ids.get(&r.uid).map(|&id| (id, norm(mine, mine.objects.get(id).unwrap())));
        match (base_n, mine_n) {
            (_, Some((_, m))) if m == theirs => {}
            (None, None) => {
                let o = from_norm(mine, &theirs);
                let id = mine.objects.insert_with_uid(o, r.uid);
                mine_ids.insert(r.uid, id);
                out.taken.objects_added += 1;
            }
            (None, Some((id, _))) => {
                out.conflict(key, ConflictKind::Changed, what, at, def);
                if choice(choices, key, def) == Choice::Theirs {
                    let o = from_norm(mine, &theirs);
                    mine.objects.update(id, o);
                }
            }
            (Some(_), None) => {
                out.conflict(key, ConflictKind::DeletedHere, what, at, def);
                if choice(choices, key, def) == Choice::Theirs {
                    let o = from_norm(mine, &theirs);
                    let id = mine.objects.insert_with_uid(o, r.uid);
                    mine_ids.insert(r.uid, id);
                }
            }
            (Some(bn), Some((id, m))) => {
                let mut merged = m.clone();
                let mut clash = false;
                macro_rules! field {
                    ($f:ident) => {
                        if theirs.$f != bn.$f && theirs.$f != m.$f {
                            if m.$f == bn.$f {
                                merged.$f = theirs.$f.clone();
                            } else {
                                clash = true;
                                if choice(choices, key, def) == Choice::Theirs {
                                    merged.$f = theirs.$f.clone();
                                }
                            }
                        }
                    };
                }
                field!(pos);
                field!(rot);
                field!(scale);
                field!(model);
                field!(flags);
                field!(layer);
                if clash {
                    out.conflict(key, ConflictKind::Changed, what, at, def);
                }
                if merged != m {
                    let o = from_norm(mine, &merged);
                    mine.objects.update(id, o);
                    out.taken.objects_changed += 1;
                }
            }
        }
    }
    for uid in &b.removed {
        let (Some(&bid), Some(&mid)) = (base_ids.get(uid), mine_ids.get(uid)) else { continue };
        let m = mine.objects.get(mid).copied().unwrap();
        if norm(base, base.objects.get(bid).unwrap()) == norm(mine, &m) {
            mine.objects.remove(mid);
            mine_ids.remove(uid);
            out.taken.objects_removed += 1;
            continue;
        }
        let key = ConflictKey::Object(*uid);
        let def = owner_default(&owner_of(mine, &layer_name(mine, m.layer())), author);
        out.conflict(key, ConflictKind::DeletedThere, mine.objects.model_path(&m).to_string(), Some([m.x, m.z]), def);
        if choice(choices, key, def) == Choice::Theirs {
            mine.objects.remove(mid);
            mine_ids.remove(uid);
        }
    }
    // Modifiers, roads and generators by id, after the objects they refer to.
    for (name, _) in LISTS {
        let Some(d) = b.lists.get(name) else { continue };
        let mut list = list_values(mine, name);
        let by_author = |v: &Value| owner_default(v.get("author").and_then(Value::as_str).unwrap_or(""), author);
        if merge_list(name, &list_values(base, name), &mut list, &d.set, &d.removed, id_of, name_of, place_of, by_author, choices, &mut out) {
            set_list(mine, name, list, &mine_ids)?;
            out.taken.other.push(name.to_string());
        }
    }
    for name in PARTS {
        let Some(t) = b.parts.get(name) else { continue };
        let m = part_value(mine, name);
        if m == *t {
            continue;
        }
        let key = ConflictKey::Part(name);
        if m == part_value(base, name) {
            set_part(mine, name, t.clone())?;
            out.taken.other.push(name.to_string());
        } else {
            out.conflict(key, ConflictKind::Changed, name.to_string(), None, Choice::Mine);
            if choice(choices, key, Choice::Mine) == Choice::Theirs {
                set_part(mine, name, t.clone())?;
                out.taken.other.push(name.to_string());
            }
        }
    }
    if let Some(mb) = &b.mission {
        merge_mission(base, patch, mb, mine, dirs, choices, &mut out)?;
    }
    if let Some(t) = &b.meta {
        let bv = serde_json::to_value(&base.meta)?;
        let mut mv = serde_json::to_value(&mine.meta)?;
        let before = mv.clone();
        let mut t = t.clone();
        for k in FIXED_SETTINGS {
            t[k] = bv[k].clone();
        }
        merge_value("", &bv, &mut mv, &t, choices, &mut out);
        for k in FIXED_SETTINGS {
            mv[k] = before[k].clone();
        }
        if mv != before {
            mine.meta = serde_json::from_value(mv).context("map settings")?;
            out.taken.other.push("map_settings".into());
        }
    }
    mine.recompose(None);
    out.conflicts.sort_by_key(|c| c.key);
    Ok(out)
}

/// Who made the modifiers taken from the other person, under the keys of their kind.
fn note_taken(w: &mut World, kind: u64, taken: Vec<(u32, String)>) {
    for (id, author) in taken {
        if !author.is_empty() {
            w.authors.get_or_insert_with(Default::default).insert(kind | id as u64, author);
        }
    }
}

/// The mask as painted pixel by pixel, then its modifiers by id laid over it again.
fn merge_mask(base: &World, patch: &Patch, mine: &mut World, choices: &HashMap<ConflictKey, Choice>, out: &mut Merged) -> Result<()> {
    let b = &patch.body;
    if b.mask_tiles.is_empty() && b.mask_mods.is_none() && b.mask_mods_removed.is_empty() {
        return Ok(());
    }
    let bm = base.mask_original();
    let mut mm = mine.mask_original();
    let mw = mm.width;
    let px = mine.meta.size_m() / mw.max(1) as f32;
    for t in &b.mask_tiles {
        let (x0, y0) = (t.x as usize * TILE, t.y as usize * TILE);
        let (x1, y1) = ((x0 + TILE).min(mw), (y0 + TILE).min(mm.height));
        let w = x1 - x0;
        let theirs = patch.u8s(t.at, w * (y1 - y0))?;
        let key = ConflictKey::Mask(t.x, t.y);
        let pick = choice(choices, key, Choice::Mine);
        let mut clash = false;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * mw + x;
                let (bv, mv, tv) = (bm.data[i], mm.data[i], theirs[(y - y0) * w + x - x0]);
                if tv == bv || tv == mv {
                    continue;
                }
                if mv == bv {
                    mm.data[i] = tv;
                    continue;
                }
                clash = true;
                if pick == Choice::Theirs {
                    mm.data[i] = tv;
                }
            }
        }
        out.taken.mask_tiles += 1;
        if clash {
            // Row 0 of the mask is the north edge.
            let at = [(x0 + x1) as f32 * 0.5 * px, mine.meta.size_m() - (y0 + y1) as f32 * 0.5 * px];
            out.conflict(key, ConflictKind::Surface, String::new(), Some(at), Choice::Mine);
        }
    }
    let set = match b.mask_mods {
        Some((at, n)) => MaskLayers::from_bytes(patch.u8s(at, n)?)?.modifiers,
        None => Vec::new(),
    };
    let mut mods = mine.mask_mods.modifiers.clone();
    let author = patch.header.author.as_str();
    let size = mine.meta.size_m();
    let tile_m = crate::maskmod::TILE as f32 * px;
    let place = |k: mods::Key| Some([(k.0 as f32 + 0.5) * tile_m, size - (k.1 as f32 + 0.5) * tile_m]);
    let default = |id: u32, d: &mods::ModDelta| owner_default(Some(mine.mask_mod_author(id)).filter(|a| !a.is_empty()).unwrap_or(&d.author), author);
    let before = mods.clone();
    let taken = mods::merge("mask_modifiers", &base.mask_mods.modifiers, &mut mods, set, &b.mask_mods_delta, &b.mask_mods_removed, default, place, choices, out);
    if mods != before {
        out.taken.other.push("mask_modifiers".into());
    }
    note_taken(mine, MASK_MOD_AUTHOR, taken);
    // The painted mask with the modifiers over it again.
    let keys = MaskLayers::keys_of(&mods);
    mine.mask = mm;
    mine.mask_mods = MaskLayers::from_modifiers(mods);
    mine.mask_recompose(keys);
    Ok(())
}

/// Merges the satellite image band by band and writes it out with the merged modifiers laid
/// over it. Gives the new layers, and the pixels when `mine` holds the image in memory.
#[allow(clippy::too_many_arguments)]
fn merge_satellite(base: &World, patch: &Patch, sb: &SatBody, mine: &World, dirs: Dirs, choices: &HashMap<ConflictKey, Choice>, out: &mut Merged) -> Result<Option<(SatLayers, Option<Vec<u8>>, Vec<(u32, String)>)>> {
    let (Some(mut br), Some(mut mr)) = (SatReader::new(base, dirs.base)?, SatReader::new(mine, dirs.mine)?) else {
        out.skipped.push("satellite".into());
        return Ok(None);
    };
    if [br.width, br.height] != sb.size || [mr.width, mr.height] != sb.size {
        out.skipped.push("satellite_size".into());
        return Ok(None);
    }
    let set = match sb.mods {
        Some((at, n)) => SatLayers::from_bytes(patch.u8s(at, n)?)?.modifiers,
        None => Vec::new(),
    };
    let mut mods: Vec<SatModifier> = mine.sat.modifiers.clone();
    let author = patch.header.author.as_str();
    let size = mine.meta.size_m();
    let tile_m = SAT_TILE as f32 * size / mr.width.max(1) as f32;
    let place = |k: mods::Key| Some([(k.0 as f32 + 0.5) * tile_m, size - (k.1 as f32 + 0.5) * tile_m]);
    let default = |id: u32, d: &mods::ModDelta| owner_default(Some(mine.sat_mod_author(id)).filter(|a| !a.is_empty()).unwrap_or(&d.author), author);
    let taken = mods::merge("sat_modifiers", &base.sat.modifiers, &mut mods, set, &sb.mods_delta, &sb.removed, default, place, choices, out);
    if mods != mine.sat.modifiers {
        out.taken.other.push("sat_modifiers".into());
    }
    let in_memory = mine.satellite.as_ref().is_some_and(|s| s.data.len() == s.width * s.height * 3);
    let mut sink = if in_memory {
        SatOut::Memory(Vec::with_capacity(sb.size[0] * sb.size[1] * 3))
    } else if let Some(p) = dirs.satellite_out {
        SatOut::File(BufWriter::with_capacity(1 << 20, std::fs::File::create(p).with_context(|| p.display().to_string())?))
    } else {
        SatOut::Nowhere
    };
    let tiles: HashMap<(u32, u32), &TileRec> = sb.tiles.iter().map(|t| ((t.x, t.y), t)).collect();
    let covered = SatLayers::keys_of(&mods);
    let mut kept = HashMap::new();
    let width = mr.width;
    let px = mine.meta.size_m() / width.max(1) as f32;
    for ty in 0..mr.rows() {
        let bb = br.band(ty)?;
        let mut mb = mr.band(ty)?;
        let rows = mb.len() / (width * 3);
        for tx in 0..width.div_ceil(SAT_TILE) {
            let (x0, w) = (tx * SAT_TILE, SAT_TILE.min(width - tx * SAT_TILE));
            if let Some(t) = tiles.get(&(tx as u32, ty as u32)) {
                let theirs = patch.u8s(t.at, w * rows * 3)?;
                let key = ConflictKey::Satellite(tx as u32, ty as u32);
                let pick = choice(choices, key, Choice::Mine);
                let mut clash = false;
                for y in 0..rows {
                    for x in 0..w {
                        let i = ((y * width) + x0 + x) * 3;
                        let j = (y * w + x) * 3;
                        let (bv, mv, tv) = (&bb[i..i + 3], &mb[i..i + 3], &theirs[j..j + 3]);
                        if tv == bv || tv == mv {
                            continue;
                        }
                        if mv == bv || pick == Choice::Theirs {
                            clash |= mv != bv;
                            mb[i..i + 3].copy_from_slice(tv);
                        } else {
                            clash = true;
                        }
                    }
                }
                out.taken.satellite_tiles += 1;
                if clash {
                    let at = [(x0 as f32 + w as f32 * 0.5) * px, mine.meta.size_m() - (ty * SAT_TILE) as f32 * px - rows as f32 * 0.5 * px];
                    out.conflict(key, ConflictKind::Satellite, String::new(), Some(at), Choice::Mine);
                }
            }
            // The modifiers over the merged pixels, which they keep under them.
            let key = (tx as u16, ty as u16);
            if covered.contains(&key) {
                let mut original = vec![0u8; SAT_TILE * SAT_TILE * 3];
                for y in 0..rows {
                    let i = (y * width + x0) * 3;
                    original[y * SAT_TILE * 3..y * SAT_TILE * 3 + w * 3].copy_from_slice(&mb[i..i + w * 3]);
                }
                let layers: Vec<(&crate::satmod::Tile, u32)> = mods.iter().filter(|m| m.enabled && m.opacity > 0).filter_map(|m| m.tiles.get(&key).map(|t| (t, m.opacity as u32))).collect();
                for y in 0..rows {
                    for x in 0..w {
                        let t = y * SAT_TILE + x;
                        let c = crate::satmod::compose_pixel([original[t * 3], original[t * 3 + 1], original[t * 3 + 2]], &layers, t);
                        let i = (y * width + x0 + x) * 3;
                        mb[i..i + 3].copy_from_slice(&c);
                    }
                }
                kept.insert(key, std::sync::Arc::new(original));
            }
        }
        sink.write(&mb)?;
    }
    let pixels = match sink {
        SatOut::Memory(v) => Some(v),
        SatOut::File(mut f) => {
            f.flush()?;
            None
        }
        SatOut::Nowhere => None,
    };
    Ok(Some((SatLayers::from_parts(mods, kept), pixels, taken)))
}

/// Merges the files of the mission: text files line by line, the others whole.
#[allow(clippy::too_many_arguments)]
fn merge_mission(base: &World, patch: &Patch, mb: &MissionBody, mine: &mut World, dirs: Dirs, choices: &HashMap<ConflictKey, Choice>, out: &mut Merged) -> Result<()> {
    let bf = mission_files(base.mission.as_deref())?;
    let mf = mission_files(mine.mission.as_deref())?;
    let mut writes: Vec<(String, Vec<u8>)> = Vec::new();
    let mut deletes: Vec<String> = Vec::new();
    for f in &mb.files {
        let t = patch.u8s(f.at, f.len)?;
        let lower = f.path.to_ascii_lowercase();
        let key = ConflictKey::Mission(hash(&lower));
        let (bb, mm) = (bf.get(&lower).map(|x| x.1.as_slice()), mf.get(&lower).map(|x| x.1.as_slice()));
        match (bb, mm) {
            (_, Some(m)) if m == t => {}
            (b, m) if b == m => writes.push((f.path.clone(), t.to_vec())),
            (Some(_), None) => {
                out.conflict(key, ConflictKind::DeletedHere, f.path.clone(), None, Choice::Mine);
                if choice(choices, key, Choice::Mine) == Choice::Theirs {
                    writes.push((f.path.clone(), t.to_vec()));
                }
            }
            (b, Some(m)) => {
                let b = b.unwrap_or_default();
                if is_text(&f.path, &[b, m, t]) {
                    let r = lines::merge(b, m, t);
                    let pick = if r.conflicts > 0 {
                        out.conflict(key, ConflictKind::Lines, f.path.clone(), None, Choice::Mine);
                        choice(choices, key, Choice::Mine)
                    } else {
                        Choice::Mine
                    };
                    let merged = if pick == Choice::Theirs { r.theirs } else { r.mine };
                    if merged != m {
                        writes.push((f.path.clone(), merged));
                    }
                } else {
                    out.conflict(key, ConflictKind::Changed, f.path.clone(), None, Choice::Mine);
                    if choice(choices, key, Choice::Mine) == Choice::Theirs {
                        writes.push((f.path.clone(), t.to_vec()));
                    }
                }
            }
            (None, None) => unreachable!(),
        }
    }
    for rel in &mb.removed {
        let lower = rel.to_ascii_lowercase();
        let Some(m) = mf.get(&lower) else { continue };
        if bf.get(&lower).map(|b| &b.1) == Some(&m.1) {
            deletes.push(m.0.clone());
            continue;
        }
        let key = ConflictKey::Mission(hash(&lower));
        out.conflict(key, ConflictKind::DeletedThere, rel.clone(), None, Choice::Mine);
        if choice(choices, key, Choice::Mine) == Choice::Theirs {
            deletes.push(m.0.clone());
        }
    }
    if writes.is_empty() && deletes.is_empty() {
        return Ok(());
    }
    let m = mine.mission.get_or_insert_with(|| Box::new(Mission { name: mb.name.clone(), ..Default::default() }));
    for (rel, bytes) in writes {
        match crate::mission::read_file(&rel, &bytes) {
            // Large files are not read: they wait in the staging folder to be copied.
            FileData::Disk => {
                let dir = dirs.staging.map(Path::to_path_buf).unwrap_or_else(|| std::env::temp_dir().join(format!("dzwe-staging-{:x}", new_uid())));
                let p = dir.join(&rel);
                std::fs::create_dir_all(p.parent().unwrap_or(&dir))?;
                std::fs::write(&p, &bytes)?;
                let k = m.key(&rel).unwrap_or(rel);
                m.removed.retain(|r| !r.eq_ignore_ascii_case(&k));
                m.files.insert(k, MissionFile { data: FileData::Disk, origin: Some(p), dirty: true });
                m.rev += 1;
            }
            data => m.put(&rel, data),
        }
    }
    for rel in deletes {
        m.remove(&rel);
    }
    out.taken.other.push("mission".into());
    Ok(())
}

/// A point of a modifier, road or generator to show it on the map.
fn place_of(v: &Value) -> Option<[f32; 2]> {
    let f = |v: Option<&Value>| v.and_then(Value::as_f64).map(|x| x as f32);
    if let (Some(x), Some(z)) = (f(v.get("x")), f(v.get("z"))) {
        return Some([x, z]);
    }
    let p = v.get("points").and_then(|p| p.get(0))?;
    Some([f(p.get(0))?, f(p.get(1))?])
}

// ---------------------------------------------------------------------------------------------
// The steps the editor offers

/// Makes the saved state of the project the shared version the team starts from.
pub fn start(project_dir: &Path, w: &World, folder: &str) -> Result<TeamState> {
    let mut state = TeamState::load(project_dir);
    let id = new_uid();
    save_version(project_dir, w, id, state.base, Some(&project_dir.join(SATELLITE)))?;
    state.base = id;
    state.seen.push(id);
    state.folder = folder.to_string();
    state.save(project_dir)?;
    Ok(state)
}

fn safe_name(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect();
    if s.is_empty() { "anon".into() } else { s }
}

/// Writes what changed since the base to the team folder and makes the current state the base.
/// The project must be saved: `w` may lack the satellite image, which is read from it.
pub fn send(project_dir: &Path, w: &World, author: &str, time: &str) -> Result<(PathBuf, Summary)> {
    let mut state = TeamState::load(project_dir);
    ensure!(state.started(), "Set up team work for this project first");
    ensure!(Path::new(&state.folder).is_dir(), "The team folder {} cannot be found", state.folder);
    let base = load_version(project_dir, state.base)?;
    let result = new_uid();
    let header = PatchHeader { format: FORMAT, base: state.base, result, author: author.to_string(), time: time.to_string(), map_id: w.meta.map_id.clone(), summary: Summary::default() };
    let patch = diff(&base, Some(&version_dir(project_dir, state.base)), w, Some(project_dir), header)?;
    let summary = patch.header.summary.clone();
    let file = Path::new(&state.folder).join(format!("{}_{}_{result:016x}.{PATCH_EXT}", safe_name(time), safe_name(author)));
    save_version(project_dir, w, result, state.base, Some(&project_dir.join(SATELLITE)))?;
    atomic_write(&file, &patch.to_bytes()?)?;
    state.base = result;
    state.seen.push(result);
    state.save(project_dir)?;
    Ok((file, summary))
}

/// Patches in the team folder for this map that this copy has not merged or sent, oldest first.
pub fn incoming(project_dir: &Path, map_id: &str) -> Vec<(PathBuf, PatchHeader)> {
    let state = TeamState::load(project_dir);
    if !state.started() {
        return Vec::new();
    }
    let mut out: Vec<(PathBuf, PatchHeader)> = std::fs::read_dir(&state.folder).into_iter().flatten().flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(PATCH_EXT)))
        .filter_map(|p| Patch::read_header(&p).ok().map(|h| (p, h)))
        .filter(|(_, h)| h.map_id == map_id && !state.seen.contains(&h.result))
        .collect();
    out.sort_by(|a, b| a.1.time.cmp(&b.1.time));
    out
}

/// A merge read and tried, waiting for the conflicts to be decided.
pub struct Prepared {
    pub header: PatchHeader,
    pub file: PathBuf,
    patch: Patch,
    base: World,
}

/// Reads a patch and tries it on the saved project, to show what it brings and what clashes.
pub fn prepare(project_dir: &Path, file: &Path) -> Result<(Prepared, Merged)> {
    let patch = Patch::from_bytes(&std::fs::read(file).with_context(|| format!("{}", file.display()))?)?;
    let base = load_version(project_dir, patch.header.base)?;
    let mut mine = project::load_with(project_dir, false)?;
    ensure!(patch.header.map_id == mine.meta.map_id, "The patch is for the map {}", patch.header.map_id);
    let staging = staging_dir(project_dir);
    let dirs = Dirs { base: Some(&version_dir(project_dir, patch.header.base)), mine: Some(project_dir), satellite_out: None, staging: Some(&staging) };
    let merged = merge(&base, &patch, &mut mine, dirs, &HashMap::new());
    let _ = std::fs::remove_dir_all(&staging);
    Ok((Prepared { header: patch.header.clone(), file: file.to_path_buf(), patch, base }, merged?))
}

/// Applies a prepared merge to the project as it is saved now, with the choices made, after
/// copying the project as it was to `.team/before-merge`. The editor opens it again afterwards.
pub fn finish(project_dir: &Path, p: Prepared, choices: &HashMap<ConflictKey, Choice>, time: &str) -> Result<Merged> {
    let Prepared { header, patch, base, .. } = p;
    let mut state = TeamState::load(project_dir);
    let base_dir = version_dir(project_dir, header.base);
    let staging = staging_dir(project_dir);
    let _ = std::fs::remove_dir_all(&staging);
    // The other person's state becomes a version this copy has, so later patches can start there.
    let theirs_sat = versions_dir(project_dir).join(format!("{:016x}.satellite.tmp", header.result));
    let mut theirs = base.clone();
    merge(&base, &patch, &mut theirs, Dirs { base: Some(&base_dir), mine: Some(&base_dir), satellite_out: Some(&theirs_sat), staging: Some(&staging) }, &HashMap::new())?;
    let sat = if theirs_sat.is_file() { theirs_sat.clone() } else { base_dir.join(SATELLITE) };
    save_version(project_dir, &theirs, header.result, state.base, Some(&sat))?;
    let _ = std::fs::remove_file(&theirs_sat);
    drop(theirs);
    let satellite = project_dir.join(SATELLITE);
    let mut mine = project::load_with(project_dir, false)?;
    project::write_backup(&mine, &project_dir.join(TEAM_DIR).join("before-merge"), &safe_backup_name(time), 3, satellite.is_file().then_some(satellite.as_path()))
        .context("copy of the project before merging")?;
    let new_sat = project_dir.join("satellite.rgb.merge");
    let merged = merge(&base, &patch, &mut mine, Dirs { base: Some(&base_dir), mine: Some(project_dir), satellite_out: Some(&new_sat), staging: Some(&staging) }, choices)?;
    if new_sat.is_file() {
        std::fs::rename(&new_sat, &satellite)?;
    }
    project::save(&mine, project_dir, SaveParts { height: true, mask: true, satellite: false, objects: true })?;
    project::save_mission(&mut mine, project_dir)?;
    let _ = std::fs::remove_dir_all(&staging);
    state.base = header.result;
    state.seen.push(header.result);
    state.save(project_dir)?;
    Ok(merged)
}

/// Backups are named like `2026-11-09_18-30-05`; `time` is expected in that form.
fn safe_backup_name(time: &str) -> String {
    let b = time.as_bytes();
    if b.len() == 19 { time.to_string() } else { "2000-01-01_00-00-00".into() }
}

/// Whether two files hold the same bytes.
fn same_content(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (a.metadata(), b.metadata()) else { return false };
    if ma.len() != mb.len() {
        return false;
    }
    let (Ok(fa), Ok(fb)) = (std::fs::File::open(a), std::fs::File::open(b)) else { return false };
    let (mut ra, mut rb) = (BufReader::with_capacity(1 << 20, fa), BufReader::with_capacity(1 << 20, fb));
    let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let n = match ra.read(&mut ba) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if n == 0 {
            return true;
        }
        if rb.read_exact(&mut bb[..n]).is_err() || ba[..n] != bb[..n] {
            return false;
        }
    }
}

/// Marks a version whose satellite image is the one of the project beside it.
const SAME_SATELLITE: &str = "satellite.same";

/// Copies the project to `<team folder>/full/<map>_<time>` with what the others need to work on
/// it as a team: the files of the project, the mission and the shared version.
pub fn send_full(project_dir: &Path, map_id: &str, time: &str) -> Result<PathBuf> {
    let state = TeamState::load(project_dir);
    ensure!(state.started(), "Set up team work for this project first");
    let to = Path::new(&state.folder).join("full").join(format!("{}_{}", safe_name(map_id), safe_name(time)));
    ensure!(!to.exists(), "{} already exists", to.display());
    std::fs::create_dir_all(&to)?;
    for e in std::fs::read_dir(project_dir)?.flatten() {
        let p = e.path();
        if p.is_file() && !p.extension().is_some_and(|x| x == "tmp" || x == "merge") {
            std::fs::copy(&p, to.join(e.file_name()))?;
        }
    }
    let mission = project_dir.join(MISSION_DIR);
    if mission.is_dir() {
        crate::fsutil::copy_tree(&mission, &to.join(MISSION_DIR), true)?;
    }
    // The shared version, without a second copy of a satellite image that did not change.
    let from = version_dir(project_dir, state.base);
    let into = version_dir(&to, state.base);
    std::fs::create_dir_all(&into)?;
    for e in std::fs::read_dir(&from)?.flatten() {
        let p = e.path();
        if p.is_dir() {
            crate::fsutil::copy_tree(&p, &into.join(e.file_name()), true)?;
        } else if e.file_name() == SATELLITE && same_content(&p, &project_dir.join(SATELLITE)) {
            std::fs::write(into.join(SAME_SATELLITE), b"")?;
        } else {
            std::fs::copy(&p, into.join(e.file_name()))?;
        }
    }
    TeamState { folder: state.folder.clone(), base: state.base, seen: state.seen.clone() }.save(&to)?;
    Ok(to)
}

/// Copies a full copy from the team folder to `to`, ready to open, and makes the team folder it
/// came from the team folder of the copy.
pub fn take_full(from: &Path, to: &Path) -> Result<()> {
    ensure!(project::is_project(from), "{} is not a project", from.display());
    ensure!(!to.exists() || std::fs::read_dir(to)?.next().is_none(), "{} is not empty", to.display());
    crate::fsutil::copy_tree(from, to, false)?;
    let mut state = TeamState::load(to);
    if state.started() {
        let marker = version_dir(to, state.base).join(SAME_SATELLITE);
        if marker.is_file() {
            link_or_copy(&to.join(SATELLITE), &version_dir(to, state.base).join(SATELLITE))?;
            std::fs::remove_file(marker)?;
        }
        // `<team folder>\full\<copy>`: the folder the copy was taken from is the team folder here.
        if let Some(folder) = from.parent().and_then(Path::parent).filter(|f| f.is_dir()) {
            state.folder = folder.display().to_string();
            state.save(to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
