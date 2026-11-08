//! Terrain Builder layer files (`.tv4l`) and template libraries (`.tml`).
//!
//! A layer is an `ALB1` archive: two dictionaries (field names, class names) followed by
//! typed fields, the last of which is a byte blob holding a quadtree. Leaves of the tree
//! hold the objects, grouped by template id; `Hash` in the template libraries maps that id
//! to a model.
use crate::objects::ObjectStore;
use anyhow::{bail, ensure, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One object as Terrain Builder stores it, with map-frame coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TbObject {
    pub template: u32,
    pub x: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub scale: f32,
    pub elevation: f32,
}

/// What a layer says about itself, readable without its quadtree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerInfo {
    pub name: String,
    pub version: u32,
    pub priority: u32,
    pub visible: bool,
    pub locked: bool,
    pub objects: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Layer {
    pub info: LayerInfo,
    pub objects: Vec<TbObject>,
}

struct Cursor<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).context("Damaged layer file")?;
        let s = self.b.get(self.p..end).context("Truncated layer file")?;
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into()?))
    }
    /// `tag, type, count` and `count` of `(id, length, text)`.
    fn dictionary(&mut self) -> Result<HashMap<u16, String>> {
        self.u16()?;
        ensure!(self.u8()? == 0x0f, "Not a Terrain Builder layer");
        let n = self.u32()?;
        let mut out = HashMap::with_capacity(n as usize);
        for _ in 0..n {
            let id = self.u16()?;
            let len = self.u16()? as usize;
            out.insert(id, String::from_utf8_lossy(self.take(len)?).into_owned());
        }
        Ok(out)
    }
}

/// Reads the fields ahead of the quadtree; leaves the cursor on the tree's data.
fn read_header(c: &mut Cursor) -> Result<(LayerInfo, usize)> {
    ensure!(c.take(4)? == b"ALB1", "Not a Terrain Builder layer");
    c.take(12)?;
    let names = c.dictionary()?;
    c.dictionary()?;
    c.take(6)?;
    let mut info = LayerInfo { visible: true, ..Default::default() };
    loop {
        let name = names.get(&c.u16()?).map(String::as_str).unwrap_or("");
        match c.u8()? {
            0x0b => {
                let len = c.u16()? as usize;
                let text = String::from_utf8_lossy(c.take(len)?).into_owned();
                if name == "mname" {
                    info.name = text;
                }
            }
            0x05..=0x07 => {
                let v = c.u32()?;
                match name {
                    "mlayerVersion" => info.version = v,
                    "mnPriority" => info.priority = v,
                    "objectCount" => info.objects = v,
                    _ => {}
                }
            }
            0x0a => {
                c.take(4)?;
            }
            0x01 | 0x09 => {
                let v = c.u8()? != 0;
                match name {
                    "mbVisible" => info.visible = v,
                    "mbLocked" => info.locked = v,
                    _ => {}
                }
            }
            0x0c => {
                let bytes = c.u32()? as usize;
                if name == "tree" {
                    let len = c.u32()? as usize;
                    ensure!(bytes == len + 4, "Unexpected layer tree header");
                    return Ok((info, len));
                }
                c.take(bytes)?;
            }
            t => bail!("Unknown field type {t} in layer file"),
        }
    }
}

/// Header of a layer; only the start of the file is needed.
pub fn read_info(path: &Path) -> Result<LayerInfo> {
    use std::io::Read;
    let mut b = Vec::new();
    std::fs::File::open(path).with_context(|| path.display().to_string())?.take(1 << 20).read_to_end(&mut b)?;
    let mut info = read_header(&mut Cursor { b: &b, p: 0 }).with_context(|| path.display().to_string())?.0;
    if info.name.is_empty() {
        info.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    }
    Ok(info)
}

fn read_tree(c: &mut Cursor, out: &mut Vec<TbObject>, record: usize, depth: u32) -> Result<()> {
    ensure!(depth < 48, "Damaged layer tree");
    let kind = c.u8()?;
    c.take(32 + 8)?; // Bounding box, level and padding.
    let mask = c.u8()?;
    ensure!(mask & 0xf0 == 0, "Damaged layer tree");
    for bit in 0..4 {
        if mask >> bit & 1 != 0 {
            if kind == 0x10 {
                read_leaf(c, out, record)?;
            } else {
                read_tree(c, out, record, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn read_leaf(c: &mut Cursor, out: &mut Vec<TbObject>, record: usize) -> Result<()> {
    c.take(32 + 8)?;
    for _ in 0..c.u32()? {
        let count = c.u32()? as usize;
        let template = c.u32()?;
        ensure!(count.saturating_mul(record) <= c.b.len() - c.p, "Damaged layer tree");
        for _ in 0..count {
            let (x, z) = (c.f64()?, c.f64()?);
            let (yaw, pitch, roll, scale, elevation) = (c.f32()?, c.f32()?, c.f32()?, c.f32()?, c.f32()?);
            c.take(record - 36)?; // Object id.
            out.push(TbObject { template, x, z, yaw, pitch, roll, scale, elevation });
        }
    }
    Ok(())
}

pub fn parse(b: &[u8]) -> Result<Layer> {
    let mut c = Cursor { b, p: 0 };
    let (info, len) = read_header(&mut c)?;
    let start = c.p;
    let tree = b.get(start..start + len).context("Truncated layer file")?;
    // Version 5 widened the object id to 64 bits.
    let sizes = if info.version >= 5 { [44, 40] } else { [40, 44] };
    let mut problem = None;
    for record in sizes {
        let mut out = Vec::with_capacity(info.objects as usize);
        let mut t = Cursor { b: tree, p: 0 };
        match read_tree(&mut t, &mut out, record, 0) {
            Ok(()) if t.p == len && out.len() == info.objects as usize => return Ok(Layer { info, objects: out }),
            Ok(()) => problem = Some(format!("read {} of {} objects", out.len(), info.objects)),
            Err(e) => problem = Some(format!("{e:#}")),
        }
    }
    bail!("Unsupported layer format (version {}): {}", info.version, problem.unwrap_or_default())
}

pub fn load(path: &Path) -> Result<Layer> {
    let mut l = parse(&std::fs::read(path).with_context(|| path.display().to_string())?).with_context(|| path.display().to_string())?;
    if l.info.name.is_empty() {
        l.info.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    }
    Ok(l)
}

fn tag<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let a = s.find(&open)? + open.len();
    let n = s[a..].find(&close)?;
    Some(s[a..a + n].trim())
}

/// Model paths by template id from the text of one library.
pub fn parse_library(text: &str, out: &mut HashMap<u32, String>) {
    for block in text.split("<Template>").skip(1) {
        let (Some(file), Some(hash)) = (tag(block, "File"), tag(block, "Hash")) else { continue };
        let Ok(hash) = hash.parse::<i64>() else { continue };
        let path = ObjectStore::normalize_path(file);
        let path = path.strip_prefix("p:\\").map(str::to_string).unwrap_or(path);
        out.entry(hash as u32).or_insert(path);
    }
}

pub fn read_libraries(files: &[PathBuf]) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    for f in files {
        if let Ok(b) = std::fs::read(f) {
            parse_library(&String::from_utf8_lossy(&b), &mut out);
        }
    }
    out
}

/// Every `.tml` below a folder.
pub fn libraries_in(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("tml")) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_field(b: &mut Vec<u8>, tag: u16, ty: u8, v: &[u8]) {
        b.extend_from_slice(&tag.to_le_bytes());
        b.push(ty);
        b.extend_from_slice(v);
    }

    fn layer(version: u32, id_bytes: usize, count: u32) -> Vec<u8> {
        let mut b = b"ALB1".to_vec();
        b.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let names = [(1u16, "mname"), (2, "mlayerVersion"), (3, "objectCount"), (4, "tree"), (5, "mbVisible")];
        put_field(&mut b, 2, 0x0f, &(names.len() as u32).to_le_bytes());
        for (id, n) in names {
            b.extend_from_slice(&id.to_le_bytes());
            b.extend_from_slice(&(n.len() as u16).to_le_bytes());
            b.extend_from_slice(n.as_bytes());
        }
        put_field(&mut b, 3, 0x0f, &0u32.to_le_bytes());
        b.extend_from_slice(&[5, 0, 0, 0, 0, 0]);
        let mut name = 4u16.to_le_bytes().to_vec();
        name.extend_from_slice(b"Rock");
        put_field(&mut b, 1, 0x0b, &name);
        put_field(&mut b, 2, 0x05, &version.to_le_bytes());
        put_field(&mut b, 3, 0x05, &count.to_le_bytes());
        put_field(&mut b, 5, 0x09, &[0]);
        let mut tree = vec![0x10];
        tree.extend_from_slice(&[0; 40]);
        tree.push(1); // One child, a leaf.
        tree.extend_from_slice(&[0; 40]);
        tree.extend_from_slice(&1u32.to_le_bytes());
        tree.extend_from_slice(&count.to_le_bytes());
        tree.extend_from_slice(&77u32.to_le_bytes());
        for i in 0..count {
            tree.extend_from_slice(&(200100.5 + i as f64).to_le_bytes());
            tree.extend_from_slice(&50.25f64.to_le_bytes());
            for v in [10.0f32, 1.0, 2.0, 1.5, 0.75] {
                tree.extend_from_slice(&v.to_le_bytes());
            }
            tree.extend_from_slice(&vec![0; id_bytes]);
        }
        let mut head = ((tree.len() + 4) as u32).to_le_bytes().to_vec();
        head.extend_from_slice(&(tree.len() as u32).to_le_bytes());
        head.extend_from_slice(&tree);
        put_field(&mut b, 4, 0x0c, &head);
        b
    }

    #[test]
    fn reads_both_record_sizes() {
        for (version, id) in [(5, 8), (4, 4)] {
            let l = parse(&layer(version, id, 3)).unwrap();
            assert_eq!(l.info.name, "Rock");
            assert!(!l.info.visible);
            assert_eq!(l.objects.len(), 3);
            assert_eq!(l.objects[1], TbObject { template: 77, x: 200101.5, z: 50.25, yaw: 10.0, pitch: 1.0, roll: 2.0, scale: 1.5, elevation: 0.75 });
        }
    }

    #[test]
    fn rejects_a_wrong_object_count() {
        let mut b = layer(5, 8, 2);
        let n = b.windows(4).position(|w| w == b"Rock").unwrap() + 4 + 3 + 4 + 3;
        b[n] = 9;
        assert!(parse(&b).is_err());
    }

    #[test]
    fn truncated_files_fail_cleanly() {
        let b = layer(5, 8, 2);
        for n in [3, 20, b.len() / 2, b.len() - 1] {
            assert!(parse(&b[..n]).is_err());
        }
    }

    #[test]
    fn library_ids_are_signed_on_disk() {
        let mut m = HashMap::new();
        parse_library("<Library><Template><Name>A</Name><File>P:\\DZ\\a.p3d</File><Hash>-524377248</Hash></Template><Template><Name>B</Name><File>dz\\b.p3d</File><Hash>10024</Hash></Template></Library>", &mut m);
        assert_eq!(m[&(-524377248i32 as u32)], "dz\\a.p3d");
        assert_eq!(m[&10024], "dz\\b.p3d");
    }

    /// Reads every layer of the real projects when P: is mounted.
    #[test]
    fn real_layers_parse() {
        if std::env::var("DZ_SLOW_TESTS").is_err() { return; }
        for dir in ["P:/BalticrusSource/TerrainBuilder/balticrus.Layers", "P:/DolinaBobru_Source/dolinabobru.Layers"] {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "tv4l") {
                    let l = load(&e.path()).unwrap();
                    println!("{dir} {} v{} visible {} objects {}", l.info.name, l.info.version, l.info.visible, l.objects.len());
                }
            }
        }
    }
}
