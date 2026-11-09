//! Reusable generators and object presets, and the generators placed in a project.
use crate::gen::{ErosionParams, PaintRule, ScatterRule, SettlementRule, TerrainParams};
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
    /// Further passes over the same shapes, such as undergrowth under the trees.
    pub layers: Vec<ForestLayer>,
    pub user_presets: Vec<UserPreset>,
    /// (surface layer name, margin in metres) kept free of trees.
    pub skip_surfaces: Vec<(String, f32)>,
    /// Margin from roads in metres, when they are avoided.
    pub avoid_roads: Option<f32>,
    /// Margin from buildings in metres, when they are avoided.
    pub avoid_buildings: Option<f32>,
}

impl Default for ForestGen {
    fn default() -> Self {
        Self {
            // Clearings are a choice: by default a forest fills the whole shape.
            project: ForestProject { clearing_strength: 0.0, ..Default::default() },
            preset: crate::gen::forest_preset_names().into_iter().next().unwrap_or_default(),
            zones: Vec::new(),
            color_filter: None,
            edge_presets: Vec::new(),
            layers: Vec::new(),
            user_presets: Vec::new(),
            skip_surfaces: Vec::new(),
            avoid_roads: None,
            avoid_buildings: None,
        }
    }
}

/// How a generator follows its line.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum LineMode {
    /// Straight between the points, with corners.
    #[default]
    Polyline,
    /// A smooth curve through the points.
    Bezier,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct LineOptions {
    pub mode: LineMode,
    /// Distance to the side of the line in metres; positive is to the right.
    pub offset: f32,
}

/// Models a generator chooses from, listed or taken from an object preset.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ObjectSet {
    /// (model, weight)
    pub models: Vec<(String, f32)>,
    /// When set, this preset's models and options are used instead.
    pub preset: Option<u64>,
    pub scale_min: f32,
    pub scale_max: f32,
    pub random_yaw: bool,
    pub align: bool,
}

impl Default for ObjectSet {
    fn default() -> Self {
        Self { models: Vec::new(), preset: None, scale_min: 0.9, scale_max: 1.1, random_yaw: true, align: false }
    }
}

impl ObjectSet {
    /// The set with its preset, if any, filled in.
    pub fn resolved(&self, presets: &[ObjectPreset]) -> ObjectSet {
        match self.preset.and_then(|id| presets.iter().find(|p| p.id == id)) {
            Some(p) => ObjectSet { models: p.items.clone(), preset: None, scale_min: p.scale_min, scale_max: p.scale_max, random_yaw: p.random_yaw, align: p.align },
            None => self.clone(),
        }
    }
    pub fn first_model<'a>(&'a self, presets: &'a [ObjectPreset]) -> Option<&'a str> {
        let listed = self.models.first().map(|m| m.0.as_str());
        self.preset.and_then(|id| presets.iter().find(|p| p.id == id)).and_then(|p| p.items.first()).map(|m| m.0.as_str()).or(listed)
    }
}

/// Objects at regular steps along a line, Terrain Processor's Line: Regular. Fences are the
/// same thing with their pieces joined at the corners.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FenceGen {
    pub model: String,
    /// Object preset used instead of `model`.
    pub preset: Option<u64>,
    /// (model, weight) worked out from the preset when the generator runs.
    #[serde(skip)]
    pub resolved: Vec<(String, f32)>,
    /// Metres between one piece and the next; negative pieces overlap.
    pub spacing: f32,
    pub yaw_offset: f32,
    pub random_rotation: bool,
    pub follow_slope: bool,
    /// Divide every stretch so its pieces end on the corners of the line.
    pub join_corners: bool,
    /// Placed at the first and last point of the line.
    pub start_model: String,
    pub end_model: String,
    pub flip_start: bool,
    pub flip_end: bool,
    pub seed: u64,
    pub line: LineOptions,
}

impl Default for FenceGen {
    fn default() -> Self {
        Self { model: r"dz\structures\walls\wall_cnc_5.p3d".into(), preset: None, resolved: Vec::new(), spacing: 0.0, yaw_offset: 0.0, random_rotation: false, follow_slope: true, join_corners: true, start_model: String::new(), end_model: String::new(), flip_start: false, flip_end: false, seed: 1, line: LineOptions::default() }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AreaMethod {
    /// Patches of single models in a dense cover.
    #[default]
    HighDensity,
    LocalMaximum,
    LocalMinimum,
    LowDensityCircle,
    LowDensityStrip,
}

/// Terrain Processor's area tasks.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AreaGen {
    pub method: AreaMethod,
    pub objects: ObjectSet,
    pub seed: u64,
    /// Objects per hectare (high density).
    pub density: f32,
    /// Patches per hectare (high density).
    pub fraction: f32,
    /// How far patches run into each other, in metres (high density).
    pub blend: f32,
    /// Clusters per hectare (low density).
    pub cluster_density: f32,
    /// Most objects in one cluster.
    pub cluster_max: u32,
    pub min_radius: f32,
    pub max_radius: f32,
    /// Width of one strip in metres.
    pub strip_width: f32,
    /// How far a peak or hollow must stand out from its surroundings, in metres.
    pub search_radius: f32,
    pub min_distance: f32,
    pub max_slope: f32,
}

impl Default for AreaGen {
    fn default() -> Self {
        Self { method: AreaMethod::HighDensity, objects: ObjectSet::default(), seed: 1, density: 300.0, fraction: 2.0, blend: 10.0, cluster_density: 4.0, cluster_max: 12, min_radius: 5.0, max_radius: 15.0, strip_width: 6.0, search_radius: 40.0, min_distance: 2.0, max_slope: 45.0 }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum LineMethod {
    #[default]
    Random,
    SparseCluster,
    Nodes,
}

/// Terrain Processor's line tasks other than Line: Regular.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LineGen {
    pub method: LineMethod,
    pub objects: ObjectSet,
    pub seed: u64,
    /// Objects per hectare of the strip along the line.
    pub density: f32,
    /// Clusters per hectare of the strip along the line.
    pub cluster_density: f32,
    pub cluster_max: u32,
    pub min_radius: f32,
    pub max_radius: f32,
    /// Nothing nearer to the line than this, in metres.
    pub inner_distance: f32,
    /// Nothing further from the line than this, in metres.
    pub max_distance: f32,
    /// Chance of an object on each node, in percent.
    pub probability: f32,
    /// Added to the direction of the line, in degrees.
    pub rotation: f32,
    pub min_distance: f32,
    pub line: LineOptions,
}

impl Default for LineGen {
    fn default() -> Self {
        Self { method: LineMethod::Random, objects: ObjectSet::default(), seed: 1, density: 60.0, cluster_density: 6.0, cluster_max: 8, min_radius: 3.0, max_radius: 8.0, inner_distance: 0.0, max_distance: 10.0, probability: 100.0, rotation: 0.0, min_distance: 1.0, line: LineOptions::default() }
    }
}

/// Where a wire attaches to a pole, in the pole model's coordinates.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct WireJoint {
    pub incoming: bool,
    pub outgoing: bool,
    pub at: [f32; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PolePreset {
    pub name: String,
    /// Positive for poles along the line, -1 for junctions and ends.
    pub kind: i32,
    pub model_angle: f32,
    pub offset: [f32; 3],
    pub joints: Vec<WireJoint>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WirePreset {
    pub name: String,
    pub length: f32,
    pub height: f32,
    /// Drawn on the map; used for one wire of each span.
    pub main: bool,
}

/// A power line as Terrain Processor describes one.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PowerGen {
    pub config: String,
    pub poles: Vec<PolePreset>,
    pub wires: Vec<WirePreset>,
    /// Metres between poles; 0 takes the length of the wire model.
    pub spacing: f32,
    pub line: LineOptions,
}

impl Default for PowerGen {
    fn default() -> Self {
        let joint = |z: f32| WireJoint { incoming: true, outgoing: true, at: [-0.013, 15.977, z] };
        Self {
            config: "powerline_hv2".into(),
            poles: vec![PolePreset { name: "Power_HV2_Tower".into(), kind: 1, model_angle: 0.0, offset: [0.0; 3], joints: vec![joint(-7.433), joint(0.0), joint(7.533)] }],
            wires: vec![
                WirePreset { name: "Power_HV2_Wire".into(), length: 50.0, height: -0.4, main: false },
                WirePreset { name: "Power_HV2_Wire_Main".into(), length: 50.0, height: -0.4, main: true },
            ],
            spacing: 0.0,
            line: LineOptions::default(),
        }
    }
}

fn xml_blocks<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(a) = rest.find(&open) {
        let body = &rest[a + open.len()..];
        let Some(b) = body.find(&close) else { break };
        out.push(&body[..b]);
        rest = &body[b + close.len()..];
    }
    out
}

fn xml_text<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    xml_blocks(block, name).into_iter().next().map(str::trim)
}

/// Reads a Terrain Processor power line configuration.
pub fn parse_powerlines(text: &str, config: &str) -> anyhow::Result<PowerGen> {
    let number = |b: &str, n: &str| xml_text(b, n).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
    let mut poles = Vec::new();
    for p in xml_blocks(text, "Pole") {
        let offset = xml_text(p, "Offset").unwrap_or("");
        let joints = xml_blocks(p, "Joint").into_iter().map(|j| {
            let kind = xml_text(j, "Type").unwrap_or("InOut");
            WireJoint { incoming: kind != "Out", outgoing: kind != "In", at: [number(j, "X"), number(j, "Y"), number(j, "Z")] }
        }).collect();
        poles.push(PolePreset { name: xml_text(p, "Name").unwrap_or("").to_string(), kind: number(p, "Type") as i32, model_angle: number(p, "ModelAngle"), offset: [number(offset, "X"), number(offset, "Y"), number(offset, "Z")], joints });
    }
    let wires: Vec<WirePreset> = xml_blocks(text, "Wire").into_iter().map(|w| WirePreset {
        name: xml_text(w, "Name").unwrap_or("").to_string(), length: number(w, "Length"), height: number(w, "Height"), main: xml_text(w, "MapDraw").is_some_and(|v| v == "true"),
    }).collect();
    anyhow::ensure!(!poles.is_empty() && !wires.is_empty(), "No poles or wires in this configuration");
    Ok(PowerGen { config: config.to_string(), poles, wires, spacing: 0.0, line: LineOptions::default() })
}

/// Grading of a strip along a line, done as under a road.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LineTerrainGen {
    pub width: f32,
    pub falloff: f32,
    pub grading: crate::world::RoadGrading,
    pub line: LineOptions,
}

impl Default for LineTerrainGen {
    fn default() -> Self {
        Self { width: 8.0, falloff: 20.0, grading: Default::default(), line: LineOptions::default() }
    }
}

/// Water models side by side over a shape.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PondGen {
    pub model: String,
    /// Use `level` instead of the lowest ground around the shape.
    pub fixed: bool,
    pub level: f32,
    /// Added to the height of the water, in metres.
    pub offset: f32,
}

impl Default for PondGen {
    fn default() -> Self {
        Self { model: r"dz\water\ponds\pond_50x50.p3d".into(), fixed: false, level: 0.0, offset: 0.0 }
    }
}

/// Takes objects off the map.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RemoveGen {
    pub trees: bool,
    pub bushes: bool,
    pub rocks: bool,
    pub buildings: bool,
    pub other: bool,
    /// Parts of model names, separated by commas; when given they replace the categories.
    pub names: String,
    /// Parts of model names that always stay.
    pub keep: String,
    /// Reach beyond a polygon, or to each side of a line, in metres.
    pub margin: f32,
    /// Part of the matching objects that goes, from 0 to 1.
    pub share: f32,
    pub seed: u32,
}

impl Default for RemoveGen {
    fn default() -> Self {
        Self { trees: true, bushes: true, rocks: false, buildings: false, other: false, names: String::new(), keep: String::new(), margin: 0.0, share: 1.0, seed: 1 }
    }
}

/// An object a generator took off the map, kept so it can be put back.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RemovedObject {
    pub model: String,
    pub x: f32,
    pub z: f32,
    pub elevation: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub scale: f32,
    pub flags: u32,
}

impl RemovedObject {
    pub fn new(model: &str, o: &crate::objects::Obj) -> Self {
        Self { model: model.to_string(), x: o.x, z: o.z, elevation: o.elevation, yaw: o.yaw, pitch: o.pitch, roll: o.roll, scale: o.scale, flags: o.flags }
    }
    pub fn object(&self, model: u32) -> crate::objects::Obj {
        crate::objects::Obj { x: self.x, z: self.z, elevation: self.elevation, yaw: self.yaw, pitch: self.pitch, roll: self.roll, scale: self.scale, model, flags: self.flags }
    }
}

/// A river cut along a line, with water models in its bed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RiverGen {
    /// Width of the water models; the bed with its blend stays within it.
    pub width: f32,
    pub depth: f32,
    pub falloff: f32,
    /// Water model laid along the bed; empty leaves the bed dry.
    pub model: String,
    /// How far each water model reaches back over the one before it, in metres.
    pub join_offset: f32,
    /// Raises or lowers the water models, in metres.
    pub water_offset: f32,
    /// Lays the models the other way round, for a current running against the line.
    pub reverse_flow: bool,
    pub line: LineOptions,
}

/// Enough to close the seam; more shows as a dark band where the water doubles up.
const JOIN: f32 = 0.5;

impl Default for RiverGen {
    fn default() -> Self {
        Self { width: 24.0, depth: 3.0, falloff: 8.0, model: r"dz\water\streams\river_24_24.p3d".into(), join_offset: JOIN, water_offset: 0.0, reverse_flow: false, line: LineOptions { mode: LineMode::Bezier, offset: 0.0 } }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ForestLayer {
    pub preset: String,
    /// Plants per hectare; 0 takes the density of the preset.
    pub density_per_ha: f32,
    pub seed: u64,
    pub spacing: f64,
    pub scale_min: f32,
    pub scale_max: f32,
    pub clearing_strength: f64,
    pub clearing_scale_m: f64,
    pub max_slope_deg: f64,
    pub edge_padding_m: f64,
    /// Distance kept from what the passes before planted, in metres.
    pub gap: f32,
}

impl Default for ForestLayer {
    fn default() -> Self {
        Self { preset: String::new(), density_per_ha: 0.0, seed: 2337, spacing: 1.0, scale_min: 1.0, scale_max: 1.0, clearing_strength: 0.0, clearing_scale_m: 350.0, max_slope_deg: 90.0, edge_padding_m: 0.0, gap: 1.0 }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum GeneratorKind {
    Forest(Box<ForestGen>),
    SurfacePaint { rules: Vec<PaintRule>, seed: u32 },
    Settlement(SettlementRule),
    Fence(FenceGen),
    PowerLine(PowerGen),
    Scatter(ScatterRule),
    Area(AreaGen),
    Line(LineGen),
    River(RiverGen),
    LineTerrain(LineTerrainGen),
    Remove(RemoveGen),
    Pond(PondGen),
    Terrain(TerrainParams),
    Erosion(ErosionParams),
    ThermalErosion { iterations: u32, talus: f32 },
    Terraces { step: f32, sharpness: f32 },
}

/// What a generator needs to be pointed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Area,
    Line,
    Point,
    /// A polygon, a line, a road or the whole map.
    Any,
    /// Works on the terrain of the whole map.
    Map,
}

impl GeneratorKind {
    pub fn target_kind(&self) -> TargetKind {
        match self {
            GeneratorKind::Forest(_) | GeneratorKind::SurfacePaint { .. } | GeneratorKind::Scatter(_) | GeneratorKind::Area(_) | GeneratorKind::Pond(_) => TargetKind::Area,
            GeneratorKind::Settlement(_) => TargetKind::Point,
            GeneratorKind::Fence(_) | GeneratorKind::PowerLine(_) | GeneratorKind::River(_) | GeneratorKind::Line(_) | GeneratorKind::LineTerrain(_) => TargetKind::Line,
            GeneratorKind::Remove(_) => TargetKind::Any,
            GeneratorKind::Terrain(_) | GeneratorKind::Erosion(_) | GeneratorKind::ThermalErosion { .. } | GeneratorKind::Terraces { .. } => TargetKind::Map,
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
            GeneratorKind::Scatter(_) => "Area: Random",
            GeneratorKind::Area(g) => match g.method {
                AreaMethod::HighDensity => "Area: High-density Cluster",
                AreaMethod::LocalMaximum => "Area: Local Maximum Cluster",
                AreaMethod::LocalMinimum => "Area: Local Minimum Cluster",
                AreaMethod::LowDensityCircle => "Area: Low-density Cluster (circle)",
                AreaMethod::LowDensityStrip => "Area: Low-density Cluster (strip)",
            },
            GeneratorKind::Line(g) => match g.method {
                LineMethod::Random => "Line: Random",
                LineMethod::SparseCluster => "Line: Sparse Cluster",
                LineMethod::Nodes => "Line: Nodes Objects",
            },
            GeneratorKind::River(_) => "River",
            GeneratorKind::LineTerrain(_) => "Terrain along a line",
            GeneratorKind::Remove(_) => "Remove objects",
            GeneratorKind::Pond(_) => "Ponds and lakes",
            GeneratorKind::Terrain(_) => "Terrain shape",
            GeneratorKind::Erosion(_) => "Hydraulic erosion",
            GeneratorKind::ThermalErosion { .. } => "Thermal erosion",
            GeneratorKind::Terraces { .. } => "Terraces",
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
    /// Forest presets read from DayZForests files.
    pub forest_presets: Vec<UserPreset>,
    pub generator_folders: Vec<String>,
    pub object_folders: Vec<String>,
}

impl Library {
    /// Everything in the library a forest can be planted with.
    pub fn all_forest_presets(&self) -> Vec<UserPreset> {
        let mut out = self.forest_presets.clone();
        for p in &self.objects {
            if p.items.is_empty() || out.iter().any(|x| x.name == p.name) {
                continue;
            }
            let weights = p.items.iter().map(|(m, k)| (m.rsplit('\\').next().unwrap_or(m).trim_end_matches(".p3d").to_string(), *k)).collect();
            out.push(UserPreset { group: p.folder.clone(), name: p.name.clone(), density_per_ha: 300.0, weights });
        }
        out
    }
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
        make(6, "Area: Random", GeneratorKind::Scatter(ScatterRule::default())),
        make(7, "River", GeneratorKind::River(RiverGen::default())),
        make(8, "Terrain shape", GeneratorKind::Terrain(TerrainParams::default())),
        make(9, "Hydraulic erosion", GeneratorKind::Erosion(ErosionParams::default())),
        make(10, "Thermal erosion", GeneratorKind::ThermalErosion { iterations: 20, talus: 40.0 }),
        make(11, "Terraces", GeneratorKind::Terraces { step: 12.0, sharpness: 0.6 }),
        make(12, "Area: High-density Cluster", GeneratorKind::Area(AreaGen::default())),
        make(13, "Area: Local Maximum Cluster", GeneratorKind::Area(AreaGen { method: AreaMethod::LocalMaximum, ..Default::default() })),
        make(14, "Area: Local Minimum Cluster", GeneratorKind::Area(AreaGen { method: AreaMethod::LocalMinimum, ..Default::default() })),
        make(15, "Area: Low-density Cluster (circle)", GeneratorKind::Area(AreaGen { method: AreaMethod::LowDensityCircle, ..Default::default() })),
        make(16, "Area: Low-density Cluster (strip)", GeneratorKind::Area(AreaGen { method: AreaMethod::LowDensityStrip, max_radius: 25.0, ..Default::default() })),
        make(17, "Line: Random", GeneratorKind::Line(LineGen::default())),
        make(18, "Line: Regular", GeneratorKind::Fence(FenceGen { model: String::new(), spacing: 10.0, follow_slope: false, join_corners: false, ..Default::default() })),
        make(19, "Line: Sparse Cluster", GeneratorKind::Line(LineGen { method: LineMethod::SparseCluster, ..Default::default() })),
        make(20, "Line: Nodes Objects", GeneratorKind::Line(LineGen { method: LineMethod::Nodes, ..Default::default() })),
        make(21, "Terrain along a line", GeneratorKind::LineTerrain(LineTerrainGen::default())),
        make(22, "Remove objects", GeneratorKind::Remove(RemoveGen::default())),
        make(23, "Ponds and lakes", GeneratorKind::Pond(PondGen::default())),
    ]
}

/// Where a generator in the project works.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GenTarget {
    Map,
    /// All shapes of a layer, by name so the shapes can be edited.
    Layer(String),
    Shape { layer: String, index: usize },
    /// A road or track, by its id.
    Route(u64),
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
    /// The terrain modifier this generator made, if any.
    #[serde(default)]
    pub modifier: Option<u64>,
    /// What this generator took off the map.
    #[serde(default)]
    pub removed: Vec<RemovedObject>,
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
        assert_eq!(names.len(), 23);
        assert!(names.contains(&"River") && names.contains(&"Area: Random") && names.contains(&"Terrain shape") && names.contains(&"Line: Nodes Objects"));
    }

    #[test]
    fn power_line_configuration_is_read() {
        let xml = "<PowerlinesConfiguration><Poles><Pole><Name>Tower</Name><Type>-1</Type><ModelAngle>90</ModelAngle><Offset><X>1</X><Y>0</Y><Z>0</Z></Offset><JointLines><JointLine><Joint><Type>In</Type><X>0.5</X><Y>9</Y><Z>-2</Z></Joint><Joint><Type>InOut</Type><X>0</X><Y>9</Y><Z>2</Z></Joint></JointLine></JointLines></Pole></Poles><Wires><Wire><Name>W</Name><Length>50</Length><Height>-0.4</Height><MapDraw>true</MapDraw></Wire></Wires></PowerlinesConfiguration>";
        let g = parse_powerlines(xml, "x").unwrap();
        assert_eq!(g.poles[0].name, "Tower");
        assert_eq!((g.poles[0].kind, g.poles[0].model_angle, g.poles[0].offset[0]), (-1, 90.0, 1.0));
        assert_eq!(g.poles[0].joints.len(), 2);
        assert!(g.poles[0].joints[0].incoming && !g.poles[0].joints[0].outgoing);
        assert!(g.wires[0].main && g.wires[0].length == 50.0 && g.wires[0].height == -0.4);
    }
}
