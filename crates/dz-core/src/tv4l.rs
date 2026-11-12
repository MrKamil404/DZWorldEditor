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
    /// Marked "underground"; only layers of version 5 store it.
    pub underground: bool,
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

pub(crate) struct Cursor<'a> {
    pub(crate) b: &'a [u8],
    pub(crate) p: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).context("Damaged layer file")?;
        let s = self.b.get(self.p..end).context("Truncated layer file")?;
        self.p = end;
        Ok(s)
    }
    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub(crate) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into()?))
    }
    pub(crate) fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into()?))
    }
    /// `tag, type, count` and `count` of `(id, length, text)`.
    pub(crate) fn dictionary(&mut self) -> Result<HashMap<u16, String>> {
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
            // Object id; version 5 follows it with flags, of which 6 marks an underground object.
            c.take(4)?;
            let underground = record >= 44 && c.u32()? & 6 != 0;
            c.take(record.saturating_sub(if record >= 44 { 44 } else { 40 }))?;
            out.push(TbObject { template, x, z, yaw, pitch, roll, scale, elevation, underground });
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

/// The model paths of one library, in its order, without repeats.
pub fn library_models(text: &str) -> Vec<String> {
    let mut map = HashMap::new();
    let mut out = Vec::new();
    for block in text.split("<Template>").skip(1) {
        let Some(file) = tag(block, "File") else { continue };
        let path = ObjectStore::normalize_path(file);
        let path = path.strip_prefix("p:\\").map(str::to_string).unwrap_or(path);
        if map.insert(path.clone(), ()).is_none() {
            out.push(path);
        }
    }
    out
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

/// A model to add to a template library, with its box when the model could be read.
#[derive(Clone, Debug, Default)]
pub struct NewTemplate {
    /// Work drive path of the model.
    pub file: String,
    /// (min, max, centre) of the model box, measured from its origin.
    pub bounds: Option<([f32; 3], [f32; 3], [f32; 3])>,
}

/// The text of an empty template library, as Terrain Builder writes it.
pub fn new_library(name: &str) -> String {
    format!("<?xml version=\"1.0\" ?>\r\n<Library name=\"{}\" shape=\"rectangle\" default_fill=\"-8355712\" default_outline=\"-1\" tex=\"0\">\r\n</Library>\r\n", xml_escape(name))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The date in the form Terrain Builder writes, such as `Tue Feb  3 18:43:07 2026`, in UTC.
fn tb_date(unix: u64) -> String {
    let (days, secs) = ((unix / 86_400) as i64, unix % 86_400);
    // Civil date from days since 1970-01-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {} {:>2} {:02}:{:02}:{:02} {}", WEEKDAYS[days.rem_euclid(7) as usize], MONTHS[month as usize - 1], day, secs / 3600, secs / 60 % 60, secs % 60, year)
}

/// A template id for a name, not yet in `taken`.
fn template_hash(name: &str, taken: &std::collections::HashSet<u32>) -> u32 {
    let mut h = name.to_ascii_lowercase().bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    while h == 0 || taken.contains(&h) {
        h = h.wrapping_mul(0x0100_0193) ^ 0x5bd1_e995;
    }
    h
}

/// Adds templates for models not yet in the library. `taken` holds the template ids of every
/// library of the project, which the new ones must not repeat; they are added to it.
/// Returns the new text and how many templates were added.
pub fn add_templates(text: &str, models: &[NewTemplate], taken: &mut std::collections::HashSet<u32>, unix_time: u64) -> (String, usize) {
    let present: std::collections::HashSet<String> = library_models(text).into_iter().collect();
    let mut names: std::collections::HashSet<String> = text.split("<Template>").skip(1).filter_map(|b| tag(b, "Name")).map(|n| n.to_ascii_lowercase()).collect();
    let mut added = String::new();
    let mut count = 0;
    let mut seen = std::collections::HashSet::new();
    for m in models {
        let file = ObjectStore::normalize_path(&m.file);
        let file = file.strip_prefix("p:\\").map(str::to_string).unwrap_or(file);
        if present.contains(&file) || !seen.insert(file.clone()) {
            continue;
        }
        let stem = file.rsplit('\\').next().unwrap_or(&file).trim_end_matches(".p3d").to_string();
        let mut name = stem.clone();
        let mut k = 2;
        while names.contains(&name.to_ascii_lowercase()) {
            name = format!("{stem}_{k}");
            k += 1;
        }
        names.insert(name.to_ascii_lowercase());
        let hash = template_hash(&name, taken);
        taken.insert(hash);
        let (min, max, center, radius, height) = match m.bounds {
            Some((min, max, center)) => {
                let d: f32 = (0..3).map(|i| (max[i] - min[i]).powi(2)).sum::<f32>().sqrt();
                (min, max, center, d * 0.5, max[1] - min[1])
            }
            None => ([999.0; 3], [-999.0; 3], [-999.0; 3], -1.0, 0.0),
        };
        let v = |p: [f32; 3]| format!("X=\"{:.6}\" Y=\"{:.6}\" Z=\"{:.6}\"", p[0], p[1], p[2]);
        let lines = [
            "    <Template>".to_string(),
            format!("        <Name>{}</Name>", xml_escape(&name)),
            format!("        <File>{}</File>", xml_escape(&file)),
            format!("        <Date>{}</Date>", tb_date(unix_time)),
            "        <Archive></Archive>".into(),
            "        <Fill>-8355712</Fill>".into(),
            "        <Outline>-1</Outline>".into(),
            "        <Scale>1.000000</Scale>".into(),
            format!("        <Hash>{}</Hash>", hash as i32),
            "        <ScaleRandMin>0.000000</ScaleRandMin>".into(),
            "        <ScaleRandMax>0.000000</ScaleRandMax>".into(),
            "        <YawRandMin>0.000000</YawRandMin>".into(),
            "        <YawRandMax>0.000000</YawRandMax>".into(),
            "        <PitchRandMin>0.000000</PitchRandMin>".into(),
            "        <PitchRandMax>0.000000</PitchRandMax>".into(),
            "        <RollRandMin>0.000000</RollRandMin>".into(),
            "        <RollRandMax>0.000000</RollRandMax>".into(),
            "        <TexLLU>0.000000</TexLLU>".into(),
            "        <TexLLV>0.000000</TexLLV>".into(),
            "        <TexURU>1.000000</TexURU>".into(),
            "        <TexURV>1.000000</TexURV>".into(),
            format!("        <BBRadius>{radius:.6}</BBRadius>"),
            "        <BBHScale>1.000000</BBHScale>".into(),
            "        <AutoCenter>0</AutoCenter>".into(),
            "        <XShift>0.000000</XShift>".into(),
            "        <YShift>0.000000</YShift>".into(),
            "        <ZShift>0.000000</ZShift>".into(),
            format!("        <Height>{height:.6}</Height>"),
            format!("        <BoundingMin {} />", v(min)),
            format!("        <BoundingMax {} />", v(max)),
            format!("        <BoundingCenter {} />", v(center)),
            "        <Placement></Placement>".into(),
            "    </Template>".into(),
        ];
        for l in lines {
            added.push_str(&l);
            added.push_str("\r\n");
        }
        count += 1;
    }
    let out = match text.rfind("</Library>") {
        Some(at) => format!("{}{}{}", &text[..at], added, &text[at..]),
        None => format!("{text}{added}</Library>\r\n"),
    };
    (out, count)
}

/// Takes the templates of these models out of a library. Returns the new text and how many
/// went; objects of Terrain Builder layers that use them lose their model there.
pub fn remove_templates(text: &str, models: &[String]) -> (String, usize) {
    let gone: std::collections::HashSet<String> = models.iter().map(|m| {
        let f = ObjectStore::normalize_path(m);
        f.strip_prefix("p:\\").map(str::to_string).unwrap_or(f)
    }).collect();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut count = 0;
    while let Some(a) = rest.find("<Template>") {
        let Some(len) = rest[a..].find("</Template>") else { break };
        let end = a + len + "</Template>".len();
        let block = &rest[a..end];
        let file = tag(block, "File").map(|f| {
            let f = ObjectStore::normalize_path(f);
            f.strip_prefix("p:\\").map(str::to_string).unwrap_or(f)
        });
        if file.is_some_and(|f| gone.contains(&f)) {
            // The block goes with the indent before it and the line break after it.
            let start = rest[..a].rfind('\n').map_or(a, |n| n + 1);
            out.push_str(&rest[..start]);
            let after = &rest[end..];
            let skip = after.len() - after.trim_start_matches(['\r', '\n']).len();
            let skip = skip.min(if after.starts_with("\r\n") { 2 } else if after.starts_with('\n') { 1 } else { 0 });
            rest = &after[skip..];
            count += 1;
        } else {
            out.push_str(&rest[..end]);
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    (out, count)
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
    #[test]
    fn new_templates_are_read_back_and_can_be_taken_out() {
        let mut taken = std::collections::HashSet::new();
        let lib = new_library("Mine");
        let models = [NewTemplate { file: r"P:\DZ\Plants\Tree\t_a.p3d".into(), bounds: Some(([-1.0, 0.0, -1.0], [1.0, 8.0, 1.0], [0.0, 4.0, 0.0])) }, NewTemplate { file: r"dz\b.p3d".into(), bounds: None }];
        let (text, n) = add_templates(&lib, &models, &mut taken, 1_770_140_587);
        assert_eq!(n, 2);
        assert!(text.contains("<Date>Tue Feb  3 17:43:07 2026</Date>"), "{text}");
        assert_eq!(library_models(&text), vec![r"dz\plants\tree\t_a.p3d".to_string(), r"dz\b.p3d".to_string()]);
        let mut ids = HashMap::new();
        parse_library(&text, &mut ids);
        assert_eq!(ids.len(), 2);
        assert!(ids.keys().all(|k| taken.contains(k)));
        // The same model is not added twice, and the next one gets a name of its own.
        let (again, n) = add_templates(&text, &models[..1], &mut taken, 0);
        assert_eq!((n, again.len()), (0, text.len()));
        let (text, n) = remove_templates(&text, &[r"dz\b.p3d".to_string()]);
        assert_eq!(n, 1);
        assert_eq!(library_models(&text), vec![r"dz\plants\tree\t_a.p3d".to_string()]);
        assert!(text.ends_with("    </Template>\r\n</Library>\r\n"), "{text:?}");
    }

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
            assert_eq!(l.objects[1], TbObject { template: 77, x: 200101.5, z: 50.25, yaw: 10.0, pitch: 1.0, roll: 2.0, scale: 1.5, elevation: 0.75, underground: false });
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
        assert_eq!(library_models("<Library><Template><Name>A</Name><File>P:\\DZ\\a.p3d</File></Template><Template><File>dz\\a.p3d</File></Template></Library>"), vec!["dz\\a.p3d".to_string()]);
        parse_library("<Library><Template><Name>A</Name><File>P:\\DZ\\a.p3d</File><Hash>-524377248</Hash></Template><Template><Name>B</Name><File>dz\\b.p3d</File><Hash>10024</Hash></Template></Library>", &mut m);
        assert_eq!(m[&(-524377248i32 as u32)], "dz\\a.p3d");
        assert_eq!(m[&10024], "dz\\b.p3d");
    }

    /// Reads every layer of the real projects when P: is mounted.
    #[test]
    fn underground_flag_of_a_real_layer() {
        let p = std::path::Path::new("P:/BalticrusSource/TerrainBuilder/balticrus.Layers/kamil-temp.tv4l");
        let Ok(l) = load(p) else { return };
        if l.info.version < 5 || l.objects.len() != 2 { return; }
        // Saved by Terrain Builder with the first of the two marked underground.
        let marked: Vec<bool> = l.objects.iter().map(|o| o.underground).collect();
        assert_eq!(marked.iter().filter(|u| **u).count(), 1, "{:?}", l.objects);
        assert!(l.objects.iter().find(|o| o.underground).unwrap().x > 202_060.0);
    }

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
