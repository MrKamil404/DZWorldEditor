//! Ground clutter as the game defines it: surface, its character, then the clutter models.
use crate::layers_cfg::{array, block, children, field, strip_comments};
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct ClutterEntry {
    /// WRP style model path.
    pub model: String,
    /// Running sum of the probabilities; a random value below it picks this entry.
    pub threshold: f32,
    pub scale_min: f32,
    pub scale_max: f32,
}

#[derive(Default, Clone)]
pub struct ClutterDb {
    /// (file name prefix, character name), both lower case.
    surfaces: Vec<(String, String)>,
    characters: HashMap<String, Vec<(String, f32)>>,
    clutter: HashMap<String, (String, f32, f32)>,
}

impl ClutterDb {
    pub fn add_config(&mut self, text: &str) {
        let text = strip_comments(text);
        if let Some(b) = block(&text, "CfgSurfaces") {
            for (_, body) in children(b) {
                if let (Some(files), Some(character)) = (field(&body, "files"), field(&body, "character")) {
                    let prefix = files.split('*').next().unwrap_or("").to_ascii_lowercase();
                    if !prefix.is_empty() {
                        self.surfaces.push((prefix, character.to_ascii_lowercase()));
                    }
                }
            }
        }
        if let Some(b) = block(&text, "CfgSurfaceCharacters") {
            for (name, body) in children(b) {
                let names = array(&body, "names");
                let odds: Vec<f32> = array(&body, "probability").iter().filter_map(|v| v.parse().ok()).collect();
                let list = names.into_iter().zip(odds).map(|(n, p)| (n.to_ascii_lowercase(), p)).collect();
                self.characters.insert(name.to_ascii_lowercase(), list);
            }
        }
        if let Some(b) = block(&text, "Clutter") {
            for (name, body) in children(b) {
                let Some(model) = field(&body, "model") else { continue };
                let number = |key: &str, default: f32| field(&body, key).and_then(|v| v.parse().ok()).unwrap_or(default);
                let entry = (model.replace('/', "\\").to_ascii_lowercase(), number("scaleMin", 0.8), number("scaleMax", 1.2));
                self.clutter.insert(name.to_ascii_lowercase(), entry);
            }
        }
    }

    /// Reads the surface configs of the work drive.
    pub fn load(root: &Path) -> Self {
        let mut db = Self::default();
        let Ok(rd) = std::fs::read_dir(root.join("dz")) else { return db };
        for e in rd.flatten() {
            if !e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("surfaces") {
                continue;
            }
            if let Ok(b) = std::fs::read(e.path().join("config.cpp")) {
                db.add_config(&String::from_utf8_lossy(&b));
            }
        }
        db
    }

    /// Clutter of a surface given its `_ca.paa` texture.
    pub fn for_texture(&self, texture: &str) -> Vec<ClutterEntry> {
        let stem = texture.rsplit(['\\', '/']).next().unwrap_or(texture).to_ascii_lowercase();
        let surface = self.surfaces.iter().filter(|(p, _)| stem.starts_with(p.as_str())).max_by_key(|(p, _)| p.len());
        let Some((_, character)) = surface else { return Vec::new() };
        let mut out = Vec::new();
        let mut sum = 0.0;
        for (name, p) in self.characters.get(character).map(|v| v.as_slice()).unwrap_or(&[]) {
            sum += p;
            if let Some((model, a, b)) = self.clutter.get(name) {
                out.push(ClutterEntry { model: ensure_p3d(model), threshold: sum, scale_min: *a, scale_max: *b });
            }
        }
        out
    }
}

fn ensure_p3d(m: &str) -> String {
    if m.ends_with(".p3d") { m.to_string() } else { format!("{m}.p3d") }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SAMPLE: &str = r#"
class CfgSurfaces
{
    class DZ_SurfacesExt;
    class en_grass1 : DZ_SurfacesExt
    {
        files = "en_grass1*";
        character = "en_Grass1";
        class Visible { prone = 0.2; };
    };
};
class CfgSurfaceCharacters
{
    class en_grass1
    {
        probability[] = {0.4, 0.1};
        names[] = { "Short", "Tall" };
    };
};
class CfgWorlds { class CAWorld { class Clutter {
    class Short : DefaultClutter { model = "dz\plants\clutter\c_short.p3d"; scaleMin = 0.5; scaleMax = 0.7; };
    class Tall : DefaultClutter { model = "dz\plants\clutter\c_tall.p3d"; scaleMin = 1; scaleMax = 2; };
}; }; };
"#;
    #[test]
    fn follows_surface_to_character_to_models() {
        let mut db = ClutterDb::default();
        db.add_config(SAMPLE);
        let list = db.for_texture("dz\\surfaces_bliss\\data\\terrain\\en_grass1_ca.paa");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].model, "dz\\plants\\clutter\\c_short.p3d");
        assert!((list[1].threshold - 0.5).abs() < 1e-6);
        assert_eq!(list[1].scale_max, 2.0);
        assert!(db.for_texture("x\\other_ca.paa").is_empty());
    }
    #[test]
    fn real_grass_has_clutter_when_p_is_mounted() {
        let root = Path::new("P:/");
        if !root.join("dz/surfaces_bliss/config.cpp").is_file() { return; }
        let db = ClutterDb::load(root);
        let list = db.for_texture("dz\\surfaces_bliss\\data\\terrain\\en_grass1_ca.paa");
        assert!(list.len() >= 5, "{list:?}");
        assert!(list.iter().all(|e| root.join(e.model.replace('\\', "/")).is_file()), "{list:?}");
    }
}
