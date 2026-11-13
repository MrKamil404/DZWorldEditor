//! The classes of `CfgVehicles` in the configs of the work drive, with the model each one
//! shows. Events name classes (`StaticObj_Wreck_BMP2_DE`), the editor places models, and this
//! index goes between them both ways.
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq)]
struct ClassInfo {
    name: String,
    parent: Option<String>,
    model: Option<String>,
    scope: Option<i32>,
}

#[derive(Clone, Debug, Default)]
pub struct ClassIndex {
    /// By lower-case name.
    classes: HashMap<String, ClassInfo>,
    /// Class names by the model they show (normalised path), filled by `finish`.
    by_model: HashMap<String, Vec<String>>,
    pub files: usize,
}

fn norm_model(m: &str) -> String {
    let n = crate::objects::ObjectStore::normalize_path(m);
    if n.ends_with(".p3d") || n.is_empty() { n } else { format!("{n}.p3d") }
}

impl ClassIndex {
    /// Reads the classes of `CfgVehicles` from the text of a config.
    pub fn add_config(&mut self, text: &str) {
        let text = crate::layers_cfg::strip_comments(text);
        let b = text.as_bytes();
        // Names of the classes open around the cursor; `None` for other braces.
        let mut stack: Vec<Option<String>> = Vec::new();
        let mut pending: Option<String> = None;
        let mut i = 0;
        let word = |i: usize| -> (String, usize) {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            (text[i..j].to_string(), j)
        };
        let skip_ws = |mut i: usize| {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            i
        };
        while i < b.len() {
            match b[i] {
                b'"' => {
                    i += 1;
                    while i < b.len() {
                        if b[i] == b'"' {
                            if b.get(i + 1) == Some(&b'"') {
                                i += 2;
                                continue;
                            }
                            break;
                        }
                        i += 1;
                    }
                    i += 1;
                }
                b'{' => {
                    stack.push(pending.take());
                    i += 1;
                }
                b'}' => {
                    stack.pop();
                    i += 1;
                }
                c if c.is_ascii_alphabetic() || c == b'_' => {
                    let prev_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
                    let (w, j) = word(i);
                    i = j;
                    if !prev_ok {
                        continue;
                    }
                    let in_vehicles = stack.first().and_then(|s| s.as_deref()) == Some("cfgvehicles");
                    if w == "class" {
                        let k = skip_ws(i);
                        let (name, k) = word(k);
                        if name.is_empty() {
                            continue;
                        }
                        let mut k = skip_ws(k);
                        let mut parent = None;
                        if b.get(k) == Some(&b':') {
                            let (p, k2) = word(skip_ws(k + 1));
                            parent = (!p.is_empty()).then(|| p.to_ascii_lowercase());
                            k = skip_ws(k2);
                        }
                        i = k;
                        let lower = name.to_ascii_lowercase();
                        if b.get(k) == Some(&b'{') {
                            pending = Some(lower.clone());
                            // Only the classes right inside CfgVehicles are objects.
                            if in_vehicles && stack.len() == 1 {
                                let e = self.classes.entry(lower).or_insert_with(|| ClassInfo { name: name.clone(), ..Default::default() });
                                if parent.is_some() {
                                    e.parent = parent;
                                }
                            }
                        }
                    } else if (w == "model" || w == "scope") && in_vehicles && stack.len() == 2 {
                        let k = skip_ws(i);
                        if b.get(k) != Some(&b'=') {
                            continue;
                        }
                        let k = skip_ws(k + 1);
                        let end = text[k..].find(';').map(|e| k + e).unwrap_or(b.len());
                        let value = text[k..end].trim().trim_matches('"').to_string();
                        i = end;
                        if let Some(Some(cls)) = stack.last() {
                            if let Some(e) = self.classes.get_mut(cls) {
                                if w == "model" {
                                    e.model = Some(value);
                                } else {
                                    e.scope = value.parse().ok();
                                }
                            }
                        }
                    }
                }
                _ => i += 1,
            }
        }
    }

    /// Reads every `config.cpp` below the folders given.
    pub fn scan(dirs: &[std::path::PathBuf]) -> Self {
        use rayon::prelude::*;
        let mut files = Vec::new();
        let mut stack: Vec<std::path::PathBuf> = dirs.to_vec();
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(p);
                } else if p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("config.cpp")) {
                    files.push(p);
                }
            }
        }
        // Each file on its own, then together; a later definition fills what an earlier lacks.
        let parts: Vec<ClassIndex> = files.par_iter().filter_map(|p| {
            let bytes = std::fs::read(p).ok()?;
            let mut one = ClassIndex::default();
            one.add_config(&String::from_utf8_lossy(&bytes));
            one.files = 1;
            Some(one)
        }).collect();
        let mut idx = Self::default();
        for part in parts {
            idx.files += part.files;
            for (k, v) in part.classes {
                let e = idx.classes.entry(k).or_insert_with(|| ClassInfo { name: v.name.clone(), ..Default::default() });
                if v.parent.is_some() {
                    e.parent = v.parent;
                }
                if v.model.is_some() {
                    e.model = v.model;
                }
                if v.scope.is_some() {
                    e.scope = v.scope;
                }
            }
        }
        idx.finish();
        idx
    }

    /// The configs of the game on the work drive: `DZ` and the other top folders, without the
    /// packed mods.
    pub fn scan_work_drive(root: &Path) -> Self {
        let mut dirs = Vec::new();
        if let Ok(rd) = std::fs::read_dir(root) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                if e.path().is_dir() && !n.starts_with('$') && !matches!(n.as_str(), "system volume information" | "mods" | "temp" | "profiles" | "bin") {
                    dirs.push(e.path());
                }
            }
        }
        Self::scan(&dirs)
    }

    /// Fills the lookup by model, the inherited models included.
    pub fn finish(&mut self) {
        self.by_model.clear();
        let names: Vec<String> = self.classes.keys().cloned().collect();
        for n in names {
            if let Some(m) = self.model_of(&n) {
                self.by_model.entry(m).or_default().push(n);
            }
        }
        for v in self.by_model.values_mut() {
            v.sort();
        }
    }

    pub fn len(&self) -> usize {
        self.classes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }

    /// The model a class shows, looked up through its parents; a normalised path.
    pub fn model_of(&self, class: &str) -> Option<String> {
        let mut c = class.to_ascii_lowercase();
        for _ in 0..24 {
            let e = self.classes.get(&c)?;
            if let Some(m) = e.model.as_deref().filter(|m| !m.trim().is_empty()) {
                return Some(norm_model(m));
            }
            c = e.parent.clone()?;
        }
        None
    }

    /// The name of a class as written in the configs.
    pub fn name(&self, class: &str) -> Option<&str> {
        self.classes.get(&class.to_ascii_lowercase()).map(|e| e.name.as_str())
    }

    /// Classes showing a model, best first: public ones (`scope = 2`), then the variants for
    /// dynamic events (`_DE`) when `de` asks for them, then the shortest name.
    pub fn classes_for_model(&self, model: &str, de: bool) -> Vec<String> {
        let Some(list) = self.by_model.get(&norm_model(model)) else { return Vec::new() };
        let scope = |n: &str| self.classes.get(n).and_then(|e| self.scope_of(e)).unwrap_or(0);
        let mut v: Vec<&String> = list.iter().collect();
        v.sort_by_key(|n| {
            let is_de = n.ends_with("_de");
            (if scope(n) >= 2 { 0 } else { 1 }, if is_de == de { 0 } else { 1 }, n.len())
        });
        v.into_iter().map(|n| self.classes[n].name.clone()).collect()
    }

    fn scope_of(&self, e: &ClassInfo) -> Option<i32> {
        let mut e = e;
        for _ in 0..24 {
            if let Some(s) = e.scope {
                return Some(s);
            }
            e = self.classes.get(e.parent.as_ref()?)?;
        }
        None
    }
}

/// The model a class most likely shows when no config names it: the class without `Land_`,
/// `StaticObj_` and `_DE`, as a model file name.
pub fn guess_stem(class: &str) -> String {
    let l = class.to_ascii_lowercase();
    let l = l.strip_prefix("land_").or_else(|| l.strip_prefix("staticobj_")).unwrap_or(&l);
    let l = l.strip_suffix("_de").unwrap_or(l);
    l.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = r#"
class CfgPatches { class DZ_Structures_Wrecks { units[]={}; }; };
class CfgVehicles
{
    class HouseNoDestruct;
    // class Commented: X { model = "nope.p3d"; };
    class StaticObj_Wreck_BMP2 : HouseNoDestruct
    {
        scope = 1;
        model = "DZ\structures\Wrecks\Vehicles\Wreck_BMP2.p3d";
        class AnimationSources { class Door { source = "user"; model = "inner"; }; };
    };
    class StaticObj_Wreck_BMP2_DE : HouseNoDestruct
    {
        scope = 2;
        storageCategory = 4;
        model = "DZ\structures\Wrecks\Vehicles\Wreck_BMP2.p3d";
    };
    class Land_Wreck_V3S : HouseNoDestruct { scope = 1; model = "\DZ\structures\Wrecks\Vehicles\Wreck_V3S"; };
    class Land_Wreck_V3S_DE : Land_Wreck_V3S { scope = 2; displayName = "a ""quoted"" {name}"; };
};
class CfgModels { class Wreck_BMP2 { sections[] = {"a"}; }; };
"#;

    #[test]
    fn classes_find_their_models() {
        let mut idx = ClassIndex::default();
        idx.add_config(CFG);
        idx.finish();
        assert_eq!(idx.model_of("staticobj_wreck_bmp2_de").as_deref(), Some("dz\\structures\\wrecks\\vehicles\\wreck_bmp2.p3d"));
        assert_eq!(idx.model_of("Land_Wreck_V3S_DE").as_deref(), Some("dz\\structures\\wrecks\\vehicles\\wreck_v3s.p3d"));
        assert!(idx.model_of("Wreck_BMP2").is_none(), "CfgModels is not CfgVehicles");
        assert_eq!(idx.classes_for_model("DZ\\structures\\Wrecks\\Vehicles\\Wreck_BMP2.p3d", true), vec!["StaticObj_Wreck_BMP2_DE", "StaticObj_Wreck_BMP2"]);
        assert_eq!(idx.classes_for_model("dz/structures/wrecks/vehicles/wreck_v3s.p3d", false)[0], "Land_Wreck_V3S_DE");
        assert_eq!(guess_stem("StaticObj_Wreck_Decal_Small2_DE"), "wreck_decal_small2");
    }

    /// Reads the configs of the work drive when `DZWE_WORK_DRIVE` names it.
    #[test]
    fn work_drive_classes() {
        let Ok(root) = std::env::var("DZWE_WORK_DRIVE") else { return };
        let t = std::time::Instant::now();
        let idx = ClassIndex::scan(&[Path::new(&root).join("DZ")]);
        eprintln!("{} classes from {} configs in {:?}", idx.len(), idx.files, t.elapsed());
        for c in ["StaticObj_Wreck_BMP2_DE", "Land_Wreck_V3S_DE", "StaticObj_Wreck_Decal_Small2_DE", "StaticObj_Wreck_Train_742_Red_DE", "StaticObj_Roadblock_Wood_Long_DE"] {
            assert!(idx.model_of(c).is_some(), "{c}");
        }
    }
}
