//! Reusable generators and object presets, and the generators placed in a project.
use crate::gen::{PaintRule, SettlementRule};
use forest_core::preset::{ColorFilter, ForestProject};
use forest_core::user_presets::UserPreset;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Folder of the generators that come with the program.
pub const BUILT_IN: &str = "Built-in";

/// Forest through the DayZForests engine.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ForestGen {
    pub project: ForestProject,
    /// Preset used inside the shapes the generator is applied to.
    pub preset: String,
    /// (surface layer name, forest preset)
    pub zones: Vec<(String, String)>,
    /// Trees only where the satellite image has these colours.
    pub color_filter: Option<ColorFilter>,
    /// (preset, share) planted along the edges of zones.
    pub edge_presets: Vec<(String, f32)>,
    pub user_presets: Vec<UserPreset>,
}

impl Default for ForestGen {
    fn default() -> Self {
        Self {
            project: ForestProject::default(),
            preset: crate::gen::forest_preset_names().into_iter().next().unwrap_or_default(),
            zones: Vec::new(),
            color_filter: None,
            edge_presets: Vec::new(),
            user_presets: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FenceGen {
    pub model: String,
    /// Metres between pieces; 0 takes the length of the model.
    pub spacing: f32,
    pub yaw_offset: f32,
    pub follow_slope: bool,
}

impl Default for FenceGen {
    fn default() -> Self {
        Self { model: r"dz\structures\walls\wall_cnc_5.p3d".into(), spacing: 0.0, yaw_offset: 0.0, follow_slope: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PowerGen {
    pub tower: String,
    /// Tower at bends of the line; empty uses the normal tower.
    pub corner: String,
    /// Wires between two towers; empty leaves them out.
    pub wire: String,
    /// Metres between towers; 0 takes the length of the wire model.
    pub spacing: f32,
    pub yaw_offset: f32,
}

impl Default for PowerGen {
    fn default() -> Self {
        Self { tower: r"dz\structures\industrial\power\power_hv_tower.p3d".into(), corner: String::new(), wire: r"dz\structures\industrial\power\power_hv_line.p3d".into(), spacing: 0.0, yaw_offset: 0.0 }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum GeneratorKind {
    Forest(Box<ForestGen>),
    SurfacePaint { rules: Vec<PaintRule>, seed: u32 },
    Settlement(SettlementRule),
    Fence(FenceGen),
    PowerLine(PowerGen),
}

/// What a generator needs to be pointed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Area,
    Line,
    Point,
}

impl GeneratorKind {
    pub fn target_kind(&self) -> TargetKind {
        match self {
            GeneratorKind::Forest(_) | GeneratorKind::SurfacePaint { .. } => TargetKind::Area,
            GeneratorKind::Settlement(_) => TargetKind::Point,
            GeneratorKind::Fence(_) | GeneratorKind::PowerLine(_) => TargetKind::Line,
        }
    }
    /// Name of the built-in generator this one derives from.
    pub fn type_name(&self) -> &'static str {
        match self {
            GeneratorKind::Forest(_) => "Forest",
            GeneratorKind::SurfacePaint { .. } => "Surface painting",
            GeneratorKind::Settlement(_) => "Settlement",
            GeneratorKind::Fence(_) => "Fence",
            GeneratorKind::PowerLine(_) => "Power line",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GeneratorPreset {
    pub id: u64,
    pub name: String,
    pub folder: String,
    pub kind: GeneratorKind,
}

/// A set of models placed together, one chosen at random each time.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ObjectPreset {
    pub id: u64,
    pub name: String,
    pub folder: String,
    /// (model, weight)
    pub items: Vec<(String, f32)>,
    pub scale_min: f32,
    pub scale_max: f32,
    pub random_yaw: bool,
    pub align: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub generators: Vec<GeneratorPreset>,
    pub objects: Vec<ObjectPreset>,
    pub generator_folders: Vec<String>,
    pub object_folders: Vec<String>,
}

impl Library {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        crate::fsutil::atomic_write(path, &serde_json::to_vec_pretty(self)?)
    }
}

/// The generators every installation starts with; they cannot be changed, only copied.
pub fn built_in_generators() -> Vec<GeneratorPreset> {
    let rule = PaintRule { layer: 0, min_height: -1000.0, max_height: 9000.0, min_slope: 0.0, max_slope: 90.0, noise_scale: 0.0, coverage: 1.0 };
    let make = |id, name: &str, kind| GeneratorPreset { id, name: name.to_string(), folder: BUILT_IN.to_string(), kind };
    vec![
        make(1, "Forest", GeneratorKind::Forest(Box::default())),
        make(2, "Surface painting", GeneratorKind::SurfacePaint { rules: vec![rule], seed: 1 }),
        make(3, "Settlement", GeneratorKind::Settlement(SettlementRule::default())),
        make(4, "Fence", GeneratorKind::Fence(FenceGen::default())),
        make(5, "Power line", GeneratorKind::PowerLine(PowerGen::default())),
    ]
}

/// Where a generator in the project works.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GenTarget {
    Map,
    /// All shapes of a layer, by name so the shapes can be edited.
    Layer(String),
    Shape { layer: String, index: usize },
    Point([f32; 2]),
}

/// A generator placed in the project. Running it again replaces what it made before.
#[derive(Clone, Serialize, Deserialize)]
pub struct GeneratorInstance {
    pub id: u64,
    pub name: String,
    pub kind: GeneratorKind,
    pub target: GenTarget,
    pub objects: Vec<u32>,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// An id unlikely to meet another one, also across sessions.
pub fn new_id() -> u64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    (now << 12) | (COUNTER.fetch_add(1, Ordering::Relaxed) & 0xfff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_round_trips_through_a_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/library.json");
        let mut l = Library::default();
        l.generators.push(GeneratorPreset { id: new_id(), name: "Pines".into(), folder: "Woods".into(), kind: GeneratorKind::Forest(Box::default()) });
        l.objects.push(ObjectPreset { id: 7, name: "Rocks".into(), folder: String::new(), items: vec![("dz\\a.p3d".into(), 2.0)], scale_min: 0.5, scale_max: 2.0, random_yaw: true, align: false });
        l.save(&p).unwrap();
        let r = Library::load(&p);
        assert_eq!(r.generators[0].name, "Pines");
        assert_eq!(r.objects[0].items[0].1, 2.0);
    }

    #[test]
    fn ids_differ() {
        assert_ne!(new_id(), new_id());
    }

    #[test]
    fn built_ins_cover_every_kind() {
        let names: Vec<&str> = built_in_generators().iter().map(|g| g.kind.type_name()).collect();
        assert_eq!(names, ["Forest", "Surface painting", "Settlement", "Fence", "Power line"]);
    }
}
