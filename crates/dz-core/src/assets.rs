//! Index of the P3D models on the work drive.
use crate::objects::ObjectStore;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Asset {
    /// WRP path, relative to the work drive, lower case with backslashes.
    pub path: String,
    pub name: String,
    pub folder: String,
}

#[derive(Default, Clone)]
pub struct AssetIndex {
    pub assets: Vec<Asset>,
    pub by_folder: BTreeMap<String, Vec<u32>>,
    by_stem: HashMap<String, u32>,
}

const SKIPPED_DIRS: [&str; 6] = ["$recycle.bin", "system volume information", "temp", "bin", "core", "mods"];

/// Finds models under `root` in the given top folders, or every top folder if empty.
pub fn scan(root: &Path, tops: &[String], cancel: &std::sync::atomic::AtomicBool) -> AssetIndex {
    let mut stack: Vec<PathBuf> = Vec::new();
    if tops.is_empty() {
        if let Ok(rd) = std::fs::read_dir(root) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                if e.path().is_dir() && !SKIPPED_DIRS.contains(&name.as_str()) && !name.starts_with('@') {
                    stack.push(e.path());
                }
            }
        }
    } else {
        stack.extend(tops.iter().map(|t| root.join(t)));
    }
    let mut files = Vec::new();
    while let Some(dir) = stack.pop() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                if n != "data" || !dir.ends_with("layers") {
                    stack.push(e.path());
                }
            } else if e.file_name().to_string_lossy().to_ascii_lowercase().ends_with(".p3d") {
                files.push(e.path());
            }
        }
    }
    let mut paths: Vec<String> = files.iter().filter_map(|f| f.strip_prefix(root).ok()).map(|r| ObjectStore::normalize_path(&r.to_string_lossy())).collect();
    paths.sort();
    paths.dedup();
    from_paths(paths)
}

pub fn from_paths(paths: Vec<String>) -> AssetIndex {
    let mut idx = AssetIndex::default();
    for path in paths {
        let (folder, file) = path.rsplit_once('\\').unwrap_or(("", &path));
        let name = file.trim_end_matches(".p3d").to_string();
        let id = idx.assets.len() as u32;
        idx.by_folder.entry(folder.to_string()).or_default().push(id);
        // Vanilla data wins over copies of a model in other folders.
        let stem = name.clone();
        let vanilla = path.starts_with("dz\\");
        match idx.by_stem.get(&stem) {
            Some(old) if !vanilla || idx.assets[*old as usize].path.starts_with("dz\\") => {}
            _ => {
                idx.by_stem.insert(stem, id);
            }
        }
        idx.assets.push(Asset { folder: folder.to_string(), name, path });
    }
    idx
}

impl AssetIndex {
    /// Full model path for a Terrain Builder template name.
    pub fn resolve(&self, model: &str) -> Option<&str> {
        let n = ObjectStore::normalize_path(model);
        if n.contains('\\') {
            let n2 = if n.ends_with(".p3d") { n.clone() } else { format!("{n}.p3d") };
            if let Ok(i) = self.assets.binary_search_by(|a| a.path.as_str().cmp(&n2)) {
                return Some(&self.assets[i].path);
            }
        }
        let stem = n.rsplit('\\').next().unwrap_or(&n).trim_end_matches(".p3d");
        self.by_stem.get(stem).map(|i| self.assets[*i as usize].path.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_by_name_and_prefers_vanilla() {
        let idx = from_paths(vec!["dz\\plants\\t_a.p3d".into(), "mod\\t_a.p3d".into(), "mod\\t_b.p3d".into()]);
        assert_eq!(idx.resolve("t_a"), Some("dz\\plants\\t_a.p3d"));
        assert_eq!(idx.resolve("T_B"), Some("mod\\t_b.p3d"));
        assert_eq!(idx.resolve("mod\\t_a.p3d"), Some("mod\\t_a.p3d"));
        assert_eq!(idx.resolve("nope"), None);
        assert_eq!(idx.by_folder["dz\\plants"].len(), 1);
    }
}
