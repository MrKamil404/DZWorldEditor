//! Reusable generators and object presets, and the generators placed in a project.
use crate::objects::Obj;
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
    /// A surface painted under the trees, as a mask modifier.
    #[serde(default)]
    pub ground: ForestGround,
}

/// The surface a forest generator paints under its trees. A species with a layer of its own in
/// the forest project takes that one.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ForestGround {
    pub enabled: bool,
    /// Name of the surface layer for species without one of their own.
    pub surface: String,
    pub stamp: crate::maskmod::StampSettings,
}

impl Default for ForestGen {
    fn default() -> Self {
        Self {
            ground: ForestGround::default(),
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
    /// The first and last object stand inside the line, between its end and the fence, instead
    /// of on its end with half of them beyond it.
    pub ends_inside: bool,
    /// At each corner the pieces on both sides reach past it by half the thickness of the
    /// fence, so thick walls close the outer edge of the corner instead of leaving a notch.
    #[serde(default)]
    pub close_corners: bool,
    /// Stood on every corner of the line, turned halfway between its two sides; the pieces stop
    /// at its sides. Empty for none.
    #[serde(default)]
    pub corner_model: String,
    /// With `close_corners`: metres the piece before a corner and the piece after it reach past
    /// the corner, set by hand instead of worked out from the thickness of their ends.
    #[serde(default)]
    pub corner_by_hand: bool,
    #[serde(default)]
    pub corner_reach: [f32; 2],
    pub seed: u64,
    pub line: LineOptions,
    /// Wickets and gates set into the fence.
    #[serde(default)]
    pub openings: Vec<FenceOpening>,
    /// An opening put in wherever the fence crosses a road or track.
    #[serde(default)]
    pub road_gate: Option<FenceOpening>,
    /// Centre lines of the roads, filled in when the generator runs.
    #[serde(skip)]
    pub roads: Vec<Vec<[f32; 2]>>,
}

/// A wicket or gate in a fence: a gap with its own object or objects.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FenceOpening {
    /// A gate rather than a wicket; only its name in the editor tells them apart.
    pub gate: bool,
    /// Metres from the start of the line to the middle of the opening.
    pub at: f32,
    /// A gate of two leaves instead of one object.
    pub two_leaves: bool,
    /// The wicket or the gate in one object.
    pub model: String,
    /// The leaves of a gate in two objects.
    pub left: String,
    pub right: String,
    /// Width of the gap; 0 takes the width of the model or of both leaves.
    pub width: f32,
    /// Turns the object or the leaves by 180°, so they open the other way. Older projects have
    /// this; [`FenceOpening::turn`] says the same in degrees.
    pub flip: bool,
    /// Degrees the object, or the left leaf, is turned about its own middle, 0 to 180. The
    /// fence closes up to what the turned object leaves of the gap.
    #[serde(default)]
    pub turn: f32,
    /// The same for the right leaf of a gate of two leaves.
    #[serde(default)]
    pub turn_right: f32,
    /// Stands upright instead of tilting with the slope as the fence pieces do.
    pub upright: bool,
}

impl Default for FenceOpening {
    fn default() -> Self {
        Self { gate: false, at: 10.0, two_leaves: false, model: String::new(), left: String::new(), right: String::new(), width: 0.0, flip: false, turn: 0.0, turn_right: 0.0, upright: true }
    }
}

impl Default for FenceGen {
    fn default() -> Self {
        Self { model: r"dz\structures\walls\wall_cnc_5.p3d".into(), preset: None, resolved: Vec::new(), spacing: 0.0, yaw_offset: 0.0, random_rotation: false, follow_slope: true, join_corners: true, start_model: String::new(), end_model: String::new(), flip_start: false, flip_end: false, ends_inside: false, close_corners: false, corner_model: String::new(), corner_by_hand: false, corner_reach: [0.0; 2], seed: 1, line: LineOptions::default(), openings: Vec::new(), road_gate: None, roads: Vec::new() }
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
    /// Dig the basin and hold the water in with a terrain modifier.
    pub modify_terrain: bool,
    /// Depth of the floor under the water, in metres.
    pub depth: f32,
    /// Width of the slopes at the shore, in metres.
    pub blend: f32,
    /// Width of the strip raised around the shape where the ground is too low, in metres.
    pub bank: f32,
}

impl Default for PondGen {
    fn default() -> Self {
        Self { model: r"dz\water\ponds\pond_50x50.p3d".into(), fixed: false, level: 0.0, offset: 0.0, modify_terrain: true, depth: 2.5, blend: 8.0, bank: 50.0 }
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
    /// Cut the bed with a terrain modifier.
    pub modify_terrain: bool,
    pub line: LineOptions,
}

/// Enough to close the seam; more shows as a dark band where the water doubles up.
const JOIN: f32 = 0.5;

impl Default for RiverGen {
    fn default() -> Self {
        Self { width: 24.0, depth: 3.0, falloff: 8.0, model: r"dz\water\streams\river_24_24.p3d".into(), join_offset: JOIN, water_offset: 0.0, reverse_flow: false, modify_terrain: true, line: LineOptions { mode: LineMode::Bezier, offset: 0.0 } }
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
    Coast(crate::landscape::CoastGen),
    Cliffs(crate::landscape::CliffGen),
    Fields(crate::landscape::FieldGen),
    Roadside(crate::landscape::RoadsideGen),
    Wrecks(crate::dressing::WreckGen),
    Dressing(crate::dressing::DressGen),
    Orchard(crate::landscape::OrchardGen),
    TreeAlley(crate::landscape::TreeAlleyGen),
    Wetland(crate::landscape::WetlandGen),
    Roadblock(crate::dressing::RoadblockGen),
    GridArea(crate::landscape::GridAreaGen),
    Trench(crate::landscape::TrenchGen),
    Trail(crate::landscape::TrailGen),
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
            GeneratorKind::Forest(_) | GeneratorKind::SurfacePaint { .. } | GeneratorKind::Scatter(_) | GeneratorKind::Area(_) | GeneratorKind::Pond(_) | GeneratorKind::Coast(_) | GeneratorKind::Cliffs(_) | GeneratorKind::Fields(_) | GeneratorKind::Orchard(_) | GeneratorKind::Wetland(_) | GeneratorKind::GridArea(_) => TargetKind::Area,
            GeneratorKind::Roadside(_) | GeneratorKind::Wrecks(_) | GeneratorKind::TreeAlley(_) | GeneratorKind::Roadblock(_) => TargetKind::Line,
            GeneratorKind::Dressing(_) => TargetKind::Area,
            GeneratorKind::Settlement(_) => TargetKind::Point,
            GeneratorKind::Fence(_) | GeneratorKind::PowerLine(_) | GeneratorKind::River(_) | GeneratorKind::Line(_) | GeneratorKind::LineTerrain(_) | GeneratorKind::Trench(_) | GeneratorKind::Trail(_) => TargetKind::Line,
            GeneratorKind::Remove(_) => TargetKind::Any,
            // Terrain generators work on the whole map or inside shapes, fading out at their edge.
            GeneratorKind::Terrain(_) | GeneratorKind::Erosion(_) | GeneratorKind::ThermalErosion { .. } | GeneratorKind::Terraces { .. } => TargetKind::Area,
        }
    }
    /// Variants added after 0.4.4, stored in a separate project field so older versions still open the file.
    pub fn is_extended(&self) -> bool {
        matches!(
            self,
            GeneratorKind::Orchard(_)
                | GeneratorKind::TreeAlley(_)
                | GeneratorKind::Wetland(_)
                | GeneratorKind::Roadblock(_)
                | GeneratorKind::GridArea(_)
                | GeneratorKind::Trench(_)
                | GeneratorKind::Trail(_)
        ) || matches!(self, GeneratorKind::Terrain(t) if t.kind.is_extended())
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
            GeneratorKind::Coast(_) => "Coast",
            GeneratorKind::Cliffs(_) => "Cliffs and rocks",
            GeneratorKind::Fields(_) => "Fields",
            GeneratorKind::Roadside(_) => "Road verges",
            GeneratorKind::Wrecks(_) => "Wrecks and road decals",
            GeneratorKind::Dressing(_) => "Things round buildings",
            GeneratorKind::Orchard(_) => "Orchard",
            GeneratorKind::TreeAlley(_) => "Tree alley",
            GeneratorKind::Wetland(_) => "Wetland and marsh",
            GeneratorKind::Roadblock(_) => "Roadblock and checkpoint",
            GeneratorKind::GridArea(_) => "Grid area",
            GeneratorKind::Trench(_) => "Trenches",
            GeneratorKind::Trail(_) => "Tourist trail",
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

impl ObjectPreset {
    pub fn is_built_in(&self) -> bool {
        self.folder == BUILT_IN || self.folder.starts_with("Built-in/")
    }

    pub fn built_in_category(&self) -> Option<&str> {
        self.folder.strip_prefix("Built-in/")
    }
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
    /// Surface layer painted under a model, by model path (see `model_key`). Trees take it as
    /// the colour of the mask under them, as species do in a DayZ Forests project.
    pub model_surfaces: std::collections::BTreeMap<String, String>,
    /// Folders of the asset browser made by the user.
    pub asset_folders: Vec<AssetFolder>,
    /// Folders of the library's generators and presets, named as paths with `/`. Those inside
    /// the generators or the presets start with [`GENERATORS_ROOT`] or [`PRESETS_ROOT`]; the
    /// others lie at the top of the library and take both.
    pub library_folders: Vec<String>,
    /// Sets of objects placed as one, made from a selection.
    pub prefabs: Vec<Prefab>,
}

/// One object of a prefab, placed relative to the middle of the set.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PrefabItem {
    pub model: String,
    pub x: f32,
    pub z: f32,
    /// Over the ground, or for an object at a fixed height over the ground at the middle.
    pub elevation: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub scale: f32,
    pub flags: u32,
}

/// A set of objects laid out by hand and saved, to be placed again anywhere as one.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Prefab {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub folder: String,
    pub items: Vec<PrefabItem>,
}

impl Prefab {
    pub fn is_built_in(&self) -> bool {
        self.folder == BUILT_IN || self.folder.starts_with("Built-in/")
    }

    /// The layout of objects round the middle of their footprint on the map.
    pub fn from_objects(id: u64, name: String, objects: &[(String, Obj)], height: &crate::terrain::Heightmap) -> Self {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for (_, o) in objects {
            lo = [lo[0].min(o.x), lo[1].min(o.z)];
            hi = [hi[0].max(o.x), hi[1].max(o.z)];
        }
        let mid = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
        let ground = height.sample(mid[0], mid[1]);
        let items = objects.iter().map(|(model, o)| {
            let fixed = o.flags & crate::objects::FLAG_ABSOLUTE != 0;
            PrefabItem {
                model: model.clone(),
                x: o.x - mid[0],
                z: o.z - mid[1],
                elevation: if fixed { o.elevation - ground } else { o.elevation },
                yaw: o.yaw,
                pitch: o.pitch,
                roll: o.roll,
                scale: o.scale,
                // Its layer is chosen where it is placed.
                flags: o.flags & ((1 << crate::objects::LAYER_SHIFT) - 1),
            }
        }).collect();
        Prefab { id, name, folder: String::new(), items }
    }
    /// The objects of the prefab with its middle at (x, z), turned by `yaw` degrees.
    pub fn objects_at(&self, x: f32, z: f32, yaw: f32, height: &crate::terrain::Heightmap) -> Vec<(String, Obj)> {
        let m = crate::world::Motion { pivot: [0.0, 0.0], yaw, shift: [x, z] };
        let ground = height.sample(x, z);
        self.items.iter().map(|i| {
            let p = m.apply([i.x, i.z]);
            let fixed = i.flags & crate::objects::FLAG_ABSOLUTE != 0;
            let o = Obj { x: p[0], z: p[1], elevation: if fixed { i.elevation + ground } else { i.elevation }, yaw: i.yaw + yaw, pitch: i.pitch, roll: i.roll, scale: i.scale, model: 0, flags: i.flags };
            (i.model.clone(), o)
        }).collect()
    }
}

/// The library's own group of generators, which holds folders but is never renamed or deleted.
pub const GENERATORS_ROOT: &str = "#generators";
/// The library's own group of object presets, like [`GENERATORS_ROOT`].
pub const PRESETS_ROOT: &str = "#presets";
/// The library's own group of prefabs, like [`GENERATORS_ROOT`].
pub const PREFABS_ROOT: &str = "#prefabs";

/// What a folder of the library holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryItem {
    Generator,
    Preset,
    Prefab,
}

/// A folder of the asset browser made by the user, holding models and object presets that
/// stay where they are on the work drive. Its name is a path with `/` between the folders it
/// lies in.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AssetFolder {
    pub name: String,
    pub models: Vec<String>,
    pub presets: Vec<u64>,
}

/// The folder that holds `name`, or "" for one at the top.
pub fn parent_folder(name: &str) -> &str {
    name.rsplit_once('/').map_or("", |p| p.0)
}

/// True when `name` is `folder` or lies inside it.
pub fn in_folder(name: &str, folder: &str) -> bool {
    name == folder || name.strip_prefix(folder).is_some_and(|r| r.starts_with('/'))
}

/// True when a folder of the library may hold things of `kind`: those in the generators hold
/// generators, those in the presets presets, and those at the top both.
pub fn library_folder_takes(folder: &str, kind: LibraryItem) -> bool {
    match folder.split('/').next().unwrap_or("") {
        GENERATORS_ROOT => kind == LibraryItem::Generator,
        PRESETS_ROOT => kind == LibraryItem::Preset,
        PREFABS_ROOT => kind == LibraryItem::Prefab,
        _ => true,
    }
}

/// The form of a model path the library keys models by.
pub fn model_key(path: &str) -> String {
    path.trim().trim_start_matches(['\\', '/']).replace('/', "\\").to_ascii_lowercase()
}

impl Library {
    /// User object presets followed by the built-in ones.
    pub fn all_objects(&self) -> Vec<ObjectPreset> {
        let mut out = self.objects.clone();
        out.extend(built_in_objects());
        out
    }

    /// Finds an object preset by id in the user library or the built-in ones.
    pub fn find_object(&self, id: u64) -> Option<ObjectPreset> {
        self.objects.iter().find(|p| p.id == id).cloned().or_else(|| built_in_objects().into_iter().find(|p| p.id == id))
    }

    /// Finds a prefab by id in the user library or the built-in ones.
    pub fn find_prefab(&self, id: u64) -> Option<Prefab> {
        self.prefabs.iter().find(|p| p.id == id).cloned().or_else(|| built_in_prefabs().into_iter().find(|p| p.id == id))
    }

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
    pub fn folder(&self, name: &str) -> Option<&AssetFolder> {
        self.asset_folders.iter().find(|f| f.name == name)
    }
    /// Makes a folder inside `parent` ("" for the top) with a name not yet taken there.
    pub fn new_folder(&mut self, parent: &str, base: &str) -> String {
        let at = |n: &str| if parent.is_empty() { n.to_string() } else { format!("{parent}/{n}") };
        let mut name = at(base);
        let mut k = 2;
        while self.folder(&name).is_some() {
            name = at(&format!("{base} {k}"));
            k += 1;
        }
        // The folders it lies in exist too.
        let mut up = parent_folder(&name).to_string();
        while !up.is_empty() {
            if self.folder(&up).is_none() {
                self.asset_folders.push(AssetFolder { name: up.clone(), ..Default::default() });
            }
            up = parent_folder(&up).to_string();
        }
        self.asset_folders.push(AssetFolder { name: name.clone(), ..Default::default() });
        self.asset_folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        name
    }
    /// Renames a folder and the folders inside it. False when the new name is taken or empty.
    pub fn rename_folder(&mut self, old: &str, new_leaf: &str) -> bool {
        let leaf = new_leaf.trim().replace('/', "-");
        let parent = parent_folder(old);
        let new = if parent.is_empty() { leaf.clone() } else { format!("{parent}/{leaf}") };
        if new == old {
            return true;
        }
        if leaf.is_empty() || self.folder(&new).is_some() {
            return false;
        }
        for f in &mut self.asset_folders {
            if in_folder(&f.name, old) {
                f.name = format!("{new}{}", &f.name[old.len()..]);
            }
        }
        self.asset_folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        true
    }
    /// Deletes a folder and the folders inside it; the models and presets stay where they are.
    pub fn delete_folder(&mut self, name: &str) {
        self.asset_folders.retain(|f| !in_folder(&f.name, name));
    }
    /// Puts models and presets into a folder, made if it is missing.
    pub fn add_to_folder(&mut self, name: &str, models: &[String], presets: &[u64]) {
        if self.folder(name).is_none() {
            let (parent, leaf) = name.rsplit_once('/').unwrap_or(("", name));
            let made = self.new_folder(parent, leaf);
            debug_assert_eq!(made, name);
        }
        let Some(f) = self.asset_folders.iter_mut().find(|f| f.name == name) else { return };
        for m in models {
            let key = model_key(m);
            if !f.models.contains(&key) {
                f.models.push(key);
            }
        }
        for p in presets {
            if !f.presets.contains(p) {
                f.presets.push(*p);
            }
        }
    }
    pub fn remove_from_folder(&mut self, name: &str, models: &[String], presets: &[u64]) {
        let Some(f) = self.asset_folders.iter_mut().find(|f| f.name == name) else { return };
        let keys: Vec<String> = models.iter().map(|m| model_key(m)).collect();
        f.models.retain(|m| !keys.contains(m));
        f.presets.retain(|p| !presets.contains(p));
    }
    /// True for a folder of the library, its two groups included.
    pub fn library_folder_exists(&self, name: &str) -> bool {
        name == GENERATORS_ROOT || name == PRESETS_ROOT || name == PREFABS_ROOT || self.library_folders.iter().any(|f| f == name)
    }
    /// The folder a generator or preset is shown in: its own, or "" (its group) when that
    /// folder is gone or holds the other kind.
    pub fn library_home<'a>(&self, folder: &'a str, kind: LibraryItem) -> &'a str {
        if self.library_folders.iter().any(|f| f == folder) && library_folder_takes(folder, kind) {
            folder
        } else {
            ""
        }
    }
    /// Makes a folder inside `parent` of the library: "" for its top, one of its groups or
    /// another folder. Returns its name.
    pub fn new_library_folder(&mut self, parent: &str, base: &str) -> String {
        let base = base.trim().replace('/', "-");
        let base = base.trim_start_matches('#');
        let base = if base.is_empty() { "Folder" } else { base };
        let at = |n: &str| if parent.is_empty() { n.to_string() } else { format!("{parent}/{n}") };
        let mut name = at(base);
        let mut k = 2;
        while self.library_folder_exists(&name) {
            name = at(&format!("{base} {k}"));
            k += 1;
        }
        let mut up = parent_folder(&name).to_string();
        while !up.is_empty() && !self.library_folder_exists(&up) {
            self.library_folders.push(up.clone());
            up = parent_folder(&up).to_string();
        }
        self.library_folders.push(name.clone());
        self.library_folders.sort_by_key(|f| f.to_lowercase());
        name
    }
    /// Renames a folder of the library with what lies in it. False when the name is taken or
    /// empty, or for the library's own groups.
    pub fn rename_library_folder(&mut self, old: &str, new_leaf: &str) -> bool {
        if !self.library_folders.iter().any(|f| f == old) {
            return false;
        }
        let leaf = new_leaf.trim().replace('/', "-");
        let leaf = leaf.trim_start_matches('#');
        let parent = parent_folder(old);
        let new = if parent.is_empty() { leaf.to_string() } else { format!("{parent}/{leaf}") };
        if new == old {
            return true;
        }
        if leaf.is_empty() || self.library_folder_exists(&new) {
            return false;
        }
        let moved = |name: &mut String| {
            if in_folder(name, old) {
                *name = format!("{new}{}", &name[old.len()..]);
            }
        };
        self.library_folders.iter_mut().for_each(moved);
        self.generators.iter_mut().for_each(|g| moved(&mut g.folder));
        self.objects.iter_mut().for_each(|p| moved(&mut p.folder));
        self.prefabs.iter_mut().for_each(|p| moved(&mut p.folder));
        self.library_folders.sort_by_key(|f| f.to_lowercase());
        true
    }
    /// Deletes a folder of the library and the folders in it; their generators and presets go
    /// back to their groups. The groups themselves stay.
    pub fn delete_library_folder(&mut self, name: &str) {
        if !self.library_folders.iter().any(|f| f == name) {
            return;
        }
        self.library_folders.retain(|f| !in_folder(f, name));
        for g in &mut self.generators {
            if in_folder(&g.folder, name) {
                g.folder.clear();
            }
        }
        for p in &mut self.objects {
            if in_folder(&p.folder, name) {
                p.folder.clear();
            }
        }
        for p in &mut self.prefabs {
            if in_folder(&p.folder, name) {
                p.folder.clear();
            }
        }
    }
    /// Puts prefabs into a folder of the library that takes them, or back into their group.
    pub fn move_prefabs_to_library_folder(&mut self, name: &str, prefabs: &[u64]) -> usize {
        let to = if name == PREFABS_ROOT {
            String::new()
        } else if self.library_folders.iter().any(|f| f == name) && library_folder_takes(name, LibraryItem::Prefab) {
            name.to_string()
        } else {
            return 0;
        };
        let mut n = 0;
        for p in self.prefabs.iter_mut().filter(|p| prefabs.contains(&p.id)) {
            p.folder = to.clone();
            n += 1;
        }
        n
    }
    /// Puts generators and presets into a folder of the library, each kind only where it
    /// belongs; a group of the library takes its own kind back. Returns how many moved.
    pub fn move_to_library_folder(&mut self, name: &str, generators: &[u64], presets: &[u64]) -> usize {
        if name == PREFABS_ROOT {
            return 0;
        }
        let to = |kind| {
            if name == GENERATORS_ROOT || name == PRESETS_ROOT {
                Some(String::new())
            } else {
                (self.library_folders.iter().any(|f| f == name) && library_folder_takes(name, kind)).then(|| name.to_string())
            }
        };
        let gen_to = if name == PRESETS_ROOT { None } else { to(LibraryItem::Generator) };
        let preset_to = if name == GENERATORS_ROOT { None } else { to(LibraryItem::Preset) };
        let mut n = 0;
        if let Some(f) = gen_to {
            for g in self.generators.iter_mut().filter(|g| generators.contains(&g.id)) {
                g.folder = f.clone();
                n += 1;
            }
        }
        if let Some(f) = preset_to {
            for p in self.objects.iter_mut().filter(|p| presets.contains(&p.id)) {
                p.folder = f.clone();
                n += 1;
            }
        }
        n
    }
    /// The surface layer assigned to a model, if any.
    pub fn surface_of(&self, model: &str) -> Option<&str> {
        self.model_surfaces.get(&model_key(model)).map(String::as_str).filter(|s| !s.is_empty())
    }
    /// Assigns a surface layer to a model, or takes it away with `None`.
    pub fn set_surface(&mut self, model: &str, surface: Option<&str>) {
        match surface.filter(|s| !s.is_empty()) {
            Some(s) => {
                self.model_surfaces.insert(model_key(model), s.to_string());
            }
            None => {
                self.model_surfaces.remove(&model_key(model));
            }
        }
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

/// Marks a file made by [`Library::export_bundle`].
pub const BUNDLE_FORMAT: &str = "DZWorldEditor library";

/// Generators, object presets and prefabs taken out of a library to be given to somebody else
/// (a `.dzwelib` file). Everything a generator needs from the library comes along: the object
/// presets it uses. Models stay where they are on the work drive, only their paths are kept.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryBundle {
    pub format: String,
    pub version: u32,
    pub generators: Vec<GeneratorPreset>,
    pub objects: Vec<ObjectPreset>,
    pub prefabs: Vec<Prefab>,
    /// The folders the things lay in, so that they can be made again.
    pub folders: Vec<String>,
}

/// How many things an import added.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub generators: usize,
    pub presets: usize,
    pub prefabs: usize,
}

impl ImportReport {
    pub fn total(&self) -> usize {
        self.generators + self.presets + self.prefabs
    }
}

/// Ids of the object presets a generator uses. Every field that holds a preset is called
/// `preset`, so they are found by name, whatever the kind of generator.
fn preset_refs(v: &serde_json::Value, out: &mut Vec<u64>) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m {
                match (k.as_str(), x.as_u64()) {
                    ("preset", Some(id)) => out.push(id),
                    _ => preset_refs(x, out),
                }
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| preset_refs(x, out)),
        _ => {}
    }
}

/// Points the preset fields of a generator to other ids.
fn remap_preset_refs(v: &mut serde_json::Value, map: &std::collections::HashMap<u64, u64>) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                match (k.as_str(), x.as_u64().and_then(|id| map.get(&id))) {
                    ("preset", Some(new)) => *x = (*new).into(),
                    _ => remap_preset_refs(x, map),
                }
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(|x| remap_preset_refs(x, map)),
        _ => {}
    }
}

impl Library {
    /// The generators, presets and prefabs that lie in a folder of the library, the ones in
    /// the folders inside it too: "" is the whole library, a group or a folder is its part.
    /// Returns their ids.
    pub fn ids_in_library_folder(&self, name: &str) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
        let inside = |folder: &str, kind: LibraryItem| {
            let home = self.library_home(folder, kind);
            if name.is_empty() {
                true
            } else if name == GENERATORS_ROOT || name == PRESETS_ROOT || name == PREFABS_ROOT {
                library_folder_takes(name, kind) && (home.is_empty() || in_folder(home, name))
            } else {
                !home.is_empty() && in_folder(home, name)
            }
        };
        (
            self.generators.iter().filter(|g| inside(&g.folder, LibraryItem::Generator)).map(|g| g.id).collect(),
            self.objects.iter().filter(|p| inside(&p.folder, LibraryItem::Preset)).map(|p| p.id).collect(),
            self.prefabs.iter().filter(|p| inside(&p.folder, LibraryItem::Prefab)).map(|p| p.id).collect(),
        )
    }

    /// Copies of the chosen things, with the object presets the generators use.
    pub fn export_bundle(&self, generators: &[u64], presets: &[u64], prefabs: &[u64]) -> LibraryBundle {
        let mut b = LibraryBundle { format: BUNDLE_FORMAT.to_string(), version: 1, ..Default::default() };
        let mut wanted: Vec<u64> = presets.to_vec();
        for g in self.generators.iter().filter(|g| generators.contains(&g.id)) {
            if let Ok(v) = serde_json::to_value(&g.kind) {
                preset_refs(&v, &mut wanted);
            }
            b.generators.push(g.clone());
        }
        b.objects = self.objects.iter().filter(|p| wanted.contains(&p.id)).cloned().collect();
        b.prefabs = self.prefabs.iter().filter(|p| prefabs.contains(&p.id)).cloned().collect();
        let mut folders: Vec<String> = Vec::new();
        for f in b.generators.iter().map(|g| &g.folder).chain(b.objects.iter().map(|p| &p.folder)).chain(b.prefabs.iter().map(|p| &p.folder)) {
            let mut at = f.as_str();
            while !at.is_empty() {
                if self.library_folders.iter().any(|x| x == at) && !folders.iter().any(|x| x == at) {
                    folders.push(at.to_string());
                }
                at = parent_folder(at);
            }
        }
        folders.sort_by_key(|f| f.to_lowercase());
        b.folders = folders;
        b
    }

    /// Adds what a bundle holds, as new things with their own ids, next to what is there.
    /// With `into` empty each thing goes back to the folder it came from (made when it is
    /// missing); otherwise it goes into that folder, or stays in its group when the folder
    /// does not take its kind.
    pub fn import_bundle(&mut self, bundle: LibraryBundle, into: &str) -> ImportReport {
        let mut report = ImportReport::default();
        let keep_folders = into.is_empty();
        if keep_folders {
            for f in &bundle.folders {
                let mut up = f.as_str();
                let mut chain = Vec::new();
                while !up.is_empty() && !self.library_folder_exists(up) {
                    chain.push(up.to_string());
                    up = parent_folder(up);
                }
                self.library_folders.extend(chain);
            }
            self.library_folders.sort_by_key(|f| f.to_lowercase());
        }
        let is_group = into == GENERATORS_ROOT || into == PRESETS_ROOT || into == PREFABS_ROOT;
        let place = |lib: &Library, folder: &str, kind: LibraryItem| -> String {
            if keep_folders {
                lib.library_home(folder, kind).to_string()
            } else if !is_group && lib.library_folder_exists(into) && library_folder_takes(into, kind) {
                into.to_string()
            } else {
                String::new()
            }
        };
        let mut ids: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
        for mut p in bundle.objects {
            let id = new_id();
            ids.insert(p.id, id);
            p.id = id;
            p.folder = place(self, &p.folder, LibraryItem::Preset);
            self.objects.push(p);
            report.presets += 1;
        }
        for mut g in bundle.generators {
            g.id = new_id();
            g.folder = place(self, &g.folder, LibraryItem::Generator);
            // Its presets are the new copies.
            if let Ok(mut v) = serde_json::to_value(&g.kind) {
                remap_preset_refs(&mut v, &ids);
                if let Ok(kind) = serde_json::from_value(v) {
                    g.kind = kind;
                }
            }
            self.generators.push(g);
            report.generators += 1;
        }
        for mut p in bundle.prefabs {
            p.id = new_id();
            p.folder = place(self, &p.folder, LibraryItem::Prefab);
            self.prefabs.push(p);
            report.prefabs += 1;
        }
        report
    }
}

impl LibraryBundle {
    pub fn is_empty(&self) -> bool {
        self.generators.is_empty() && self.objects.is_empty() && self.prefabs.is_empty()
    }
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        crate::fsutil::atomic_write(path, &serde_json::to_vec_pretty(self)?)
    }
    /// Reads a `.dzwelib` file. Anything else is refused with a reason.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path)?;
        let b: LibraryBundle = serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("not a library file: {e}"))?;
        if b.format != BUNDLE_FORMAT {
            anyhow::bail!("not a DZ World Editor library file");
        }
        Ok(b)
    }
}

#[cfg(test)]
mod bundle_tests {
    use super::*;

    fn library() -> Library {
        let mut l = Library::default();
        let f = l.new_library_folder(GENERATORS_ROOT, "Farm");
        l.objects.push(ObjectPreset { id: 7, name: "Crates".into(), folder: String::new(), items: vec![("a.p3d".into(), 1.0)], scale_min: 1.0, scale_max: 1.0, random_yaw: true, align: false });
        let kind = GeneratorKind::Fence(FenceGen { preset: Some(7), ..Default::default() });
        l.generators.push(GeneratorPreset { id: 1, name: "Farm fence".into(), folder: f, kind });
        l.prefabs.push(Prefab { id: 3, name: "Camp".into(), folder: String::new(), items: Vec::new() });
        l
    }

    #[test]
    fn a_generator_takes_its_object_preset_and_folder_along_and_gets_it_back_under_new_ids() {
        let l = library();
        let b = l.export_bundle(&[1], &[], &[]);
        assert_eq!((b.generators.len(), b.objects.len(), b.prefabs.len()), (1, 1, 0), "the preset it uses comes along");
        assert_eq!(b.folders, vec!["#generators/Farm".to_string()]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.dzwelib");
        b.save(&path).unwrap();
        let mut other = Library::default();
        let r = other.import_bundle(LibraryBundle::load(&path).unwrap(), "");
        assert_eq!((r.generators, r.presets, r.prefabs), (1, 1, 0));
        assert_eq!(other.generators[0].folder, "#generators/Farm");
        assert!(other.library_folders.contains(&"#generators/Farm".to_string()));
        let new_preset = other.objects[0].id;
        assert_ne!(new_preset, 7, "its own id");
        match &other.generators[0].kind {
            GeneratorKind::Fence(g) => assert_eq!(g.preset, Some(new_preset), "points to the copy"),
            _ => panic!("kind changed"),
        }
        // Importing again adds copies and never touches what is there.
        other.import_bundle(b, "");
        assert_eq!((other.generators.len(), other.objects.len()), (2, 2));
    }

    #[test]
    fn a_folder_exports_what_lies_in_it_and_a_foreign_file_is_refused() {
        let mut l = library();
        let (g, p, f) = l.ids_in_library_folder("#generators/Farm");
        assert_eq!((g, p, f), (vec![1], vec![], vec![]));
        let (g, p, f) = l.ids_in_library_folder("");
        assert_eq!((g.len(), p.len(), f.len()), (1, 1, 1));
        let (g, _, f) = l.ids_in_library_folder(PREFABS_ROOT);
        assert_eq!((g.len(), f.len()), (0, 1));
        // Into a chosen folder.
        let b = l.export_bundle(&[], &[7], &[3]);
        let into = l.new_library_folder("", "Mine");
        l.import_bundle(b, &into);
        assert_eq!(l.objects.last().unwrap().folder, into);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no.dzwelib");
        std::fs::write(&path, br#"{"generators":[]}"#).unwrap();
        assert!(LibraryBundle::load(&path).is_err());
    }
}

/// The generators every installation starts with; they cannot be changed, only copied.
pub fn built_in_generators() -> Vec<GeneratorPreset> {
    let rule = PaintRule::default();
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
        make(24, "Stream", GeneratorKind::River(RiverGen { width: 14.0, depth: 2.0, falloff: 5.0, model: r"dz\water\streams\stream_14_24.p3d".into(), ..Default::default() })),
        make(25, "Streambed", GeneratorKind::River(RiverGen { width: 14.0, depth: 1.5, falloff: 5.0, model: r"dz\water\streambed\streambed_moss_long_straight.p3d".into(), ..Default::default() })),
        make(26, "Coast", GeneratorKind::Coast(Default::default())),
        make(27, "Cliffs and rocks", GeneratorKind::Cliffs(Default::default())),
        make(28, "Fields", GeneratorKind::Fields(Default::default())),
        make(29, "Road verges", GeneratorKind::Roadside(Default::default())),
        make(30, "Railway surroundings", GeneratorKind::Roadside(crate::landscape::RoadsideGen::railway())),
        make(31, "Wrecks and road decals", GeneratorKind::Wrecks(Default::default())),
        make(32, "Things round buildings", GeneratorKind::Dressing(Default::default())),
        make(33, "Orchard", GeneratorKind::Orchard(Default::default())),
        make(34, "Vineyard", GeneratorKind::Orchard(crate::landscape::OrchardGen::vineyard())),
        make(35, "Tree alley", GeneratorKind::TreeAlley(Default::default())),
        make(36, "Wetland and marsh", GeneratorKind::Wetland(Default::default())),
        make(37, "Roadblock and checkpoint", GeneratorKind::Roadblock(Default::default())),
        make(38, "Cemetery", GeneratorKind::GridArea(crate::landscape::GridAreaGen::cemetery())),
        make(39, "Parking lot", GeneratorKind::GridArea(crate::landscape::GridAreaGen::parking())),
        make(40, "Container yard", GeneratorKind::GridArea(crate::landscape::GridAreaGen::containers())),
        make(41, "Trenches", GeneratorKind::Trench(Default::default())),
        make(42, "Tourist trail", GeneratorKind::Trail(Default::default())),
    ]
}

/// Categories of the built-in object presets, in the order shown in the Content Browser.
pub const BUILT_IN_OBJECT_CATEGORIES: &[&str] = &[
    "Walls and fences",
    "Rocks and stones",
    "Wrecks",
    "Props and debris",
    "Street and roadside",
];

/// The object presets every installation starts with; they cannot be changed, only duplicated.
///
/// Damaged and holed wall/fence variants carry low weights (~5–8% total) so rows and fences
/// stay mostly intact with only occasional breaks.
pub fn built_in_objects() -> Vec<ObjectPreset> {
    let make = |id, cat: &str, name: &str, items: &[(&str, f32)], scale_min, scale_max, random_yaw, align| ObjectPreset {
        id,
        name: name.to_string(),
        folder: format!("{BUILT_IN}/{cat}"),
        items: items.iter().map(|(m, w)| ((*m).to_string(), *w)).collect(),
        scale_min,
        scale_max,
        random_yaw,
        align,
    };
    let wall = |id, name: &str, items: &[(&str, f32)]| make(id, "Walls and fences", name, items, 1.0, 1.0, false, false);
    vec![
        // Walls and fences (damaged/hole weights kept low: ~4–8% of total)
        wall(1001, "Wooden fence (village)", &[
            (r"dz\structures\walls\Wall_Wood2_5.p3d", 12.0),
            (r"dz\structures\walls\Wall_Wood2_5_D.p3d", 0.8),
        ]),
        wall(1002, "Wooden plank fence", &[
            (r"dz\structures\walls\Wall_Wood1_5.p3d", 6.0),
            (r"dz\structures\walls\Wall_Wood1_5_2.p3d", 6.0),
            (r"dz\structures\walls\Wall_Wood1D_5.p3d", 0.5),
            (r"dz\structures\walls\Wall_Wood1D_5_2.p3d", 0.5),
        ]),
        wall(1003, "Picket fence (village)", &[
            (r"dz\structures\walls\Wall_VilVar2_4.p3d", 6.0),
            (r"dz\structures\walls\Wall_VilVar2_4_2.p3d", 6.0),
            (r"dz\structures\walls\Wall_VilVar2_4_D.p3d", 0.8),
        ]),
        wall(1004, "Wire fence (low)", &[
            (r"dz\structures\walls\Wall_Fen1_5.p3d", 6.0),
            (r"dz\structures\walls\Wall_Fen1_5_2.p3d", 6.0),
            (r"dz\structures\walls\Wall_Fen1_5_Hole.p3d", 0.5),
        ]),
        wall(1005, "Wire mesh fence (tall)", &[
            (r"dz\structures\walls\Wall_Fen2_6.p3d", 12.0),
            (r"dz\structures\walls\Wall_Fen2_6_Hole.p3d", 0.5),
        ]),
        wall(1006, "Industrial chain-link fence", &[
            (r"dz\structures\walls\Wall_IndFnc_3.p3d", 12.0),
            (r"dz\structures\walls\Wall_IndFnc_3_D.p3d", 0.7),
            (r"dz\structures\walls\Wall_IndFnc_3_Hole.p3d", 0.4),
        ]),
        wall(1007, "Brick wall", &[
            (r"dz\structures\walls\Wall_CBrk_5.p3d", 12.0),
            (r"dz\structures\walls\Wall_CBrk_5_D.p3d", 0.8),
        ]),
        wall(1008, "Grey concrete wall", &[
            (r"dz\structures\walls\Wall_CGry_5.p3d", 12.0),
            (r"dz\structures\walls\Wall_CGry_5_D.p3d", 0.8),
        ]),
        wall(1009, "Industrial concrete wall", &[
            (r"dz\structures\walls\Wall_IndCnc_4.p3d", 12.0),
            (r"dz\structures\walls\Wall_IndCnc_4_D.p3d", 0.8),
        ]),
        wall(1010, "Industrial panel wall", &[
            (r"dz\structures\walls\Wall_IndVar1_5.p3d", 6.0),
            (r"dz\structures\walls\Wall_IndVar1_5_2.p3d", 6.0),
            (r"dz\structures\walls\Wall_IndVar1_5_D.p3d", 0.5),
            (r"dz\structures\walls\Wall_IndVar1_5_D_2.p3d", 0.5),
        ]),
        wall(1011, "Stone wall", &[
            (r"dz\structures\walls\Wall_Stone.p3d", 12.0),
            (r"dz\structures\walls\Wall_StoneD.p3d", 0.8),
        ]),
        wall(1012, "Wooden fence (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_Fen3_4.p3d", 12.0),
            (r"dz\structures_bliss\walls\Wall_Fen3_D.p3d", 0.8),
        ]),
        wall(1013, "Wooden fence (Sakhal)", &[
            (r"dz\structures_sakhal\walls\Wall_Wood5_4.p3d", 6.0),
            (r"dz\structures_sakhal\walls\Wall_Wood5_4_2.p3d", 6.0),
            (r"dz\structures_sakhal\walls\Wall_Wood5_4_D.p3d", 0.8),
        ]),
        // Additional Chernarus walls and fences
        wall(1101, "Wooden rail fence", &[
            (r"dz\structures\walls\Wall_FenW_7.p3d", 6.0),
            (r"dz\structures\walls\Wall_FenW_7_S.p3d", 6.0),
            (r"dz\structures\walls\Wall_FenW_7_D.p3d", 0.8),
        ]),
        wall(1102, "Forest field fence", &[
            (r"dz\structures\walls\Wall_ForFieldFen_1.p3d", 1.0),
            (r"dz\structures\walls\Wall_ForFieldFen_2.p3d", 1.0),
        ]),
        wall(1103, "Industrial corrugated wall", &[
            (r"dz\structures\walls\Wall_IndVar2_5.p3d", 12.0),
            (r"dz\structures\walls\Wall_IndVar2_5_D.p3d", 0.8),
        ]),
        wall(1104, "Tin fence (4 m)", &[
            (r"dz\structures\walls\Wall_Tin_4.p3d", 1.0),
            (r"dz\structures\walls\Wall_Tin_4_2.p3d", 1.0),
        ]),
        wall(1105, "Commercial tin fence (9 m)", &[
            (r"dz\structures\walls\Wall_TinCom_9.p3d", 1.0),
            (r"dz\structures\walls\Wall_TinCom_9_2.p3d", 1.0),
        ]),
        wall(1106, "Wooden farm fence", &[
            (r"dz\structures\walls\Wall_WoodF_5.p3d", 1.0),
            (r"dz\structures\walls\Wall_WoodF_5_2.p3d", 1.0),
        ]),
        wall(1107, "Wooden village fence (4 m)", &[
            (r"dz\structures\walls\Wall_WoodVil_4.p3d", 1.0),
            (r"dz\structures\walls\Wall_WoodVil_4_2.p3d", 1.0),
        ]),
        // Additional Livonia walls and fences
        wall(1108, "Wire fence (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_Fen4_4.p3d", 12.0),
            (r"dz\structures_bliss\walls\Wall_Fen4_D.p3d", 0.8),
        ]),
        wall(1109, "Pasture fence (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_Fen5_10.p3d", 12.0),
            (r"dz\structures_bliss\walls\Wall_Fen5_10_D.p3d", 0.8),
        ]),
        wall(1110, "Forest mesh fence (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_FenForest_5.p3d", 12.0),
            (r"dz\structures_bliss\walls\Wall_FenForest_5_D.p3d", 0.8),
        ]),
        wall(1111, "Concrete slab wall (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_IndCnc3_1_4.p3d", 6.0),
            (r"dz\structures_bliss\walls\Wall_IndCnc3_2_4.p3d", 6.0),
            (r"dz\structures_bliss\walls\Wall_IndCnc3_1_D.p3d", 0.5),
            (r"dz\structures_bliss\walls\Wall_IndCnc3_2_D.p3d", 0.5),
        ]),
        wall(1112, "Tall concrete wall (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_IndCnc4_4.p3d", 12.0),
            (r"dz\structures_bliss\walls\Wall_IndCnc4_D.p3d", 0.8),
        ]),
        wall(1113, "Tall concrete wall 8 m (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_IndCnc4_8.p3d", 6.0),
            (r"dz\structures_bliss\walls\Wall_IndCnc4_8_Graffiti.p3d", 1.5),
        ]),
        wall(1114, "Low concrete wall (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_IndCnc4_Low_4.p3d", 12.0),
            (r"dz\structures_bliss\walls\Wall_IndCnc4_Low_D.p3d", 0.8),
        ]),
        wall(1115, "Industrial chain-link fence (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_IndFnc2_3.p3d", 12.0),
            (r"dz\structures_bliss\walls\Wall_IndFnc2_3_D.p3d", 0.7),
            (r"dz\structures_bliss\walls\Wall_IndFnc2_3_Hole.p3d", 0.4),
        ]),
        wall(1116, "Industrial panel wall (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_IndVar3_5.p3d", 12.0),
            (r"dz\structures_bliss\walls\Wall_IndVar3_5_D.p3d", 0.8),
        ]),
        wall(1117, "Wooden pole fence (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_PoleWall_5_1.p3d", 1.0),
            (r"dz\structures_bliss\walls\Wall_PoleWall_5_2.p3d", 1.0),
        ]),
        wall(1118, "Silage concrete wall (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_SilageWall_5.p3d", 12.0),
            (r"dz\structures_bliss\walls\Wall_SilageWall_5_D.p3d", 0.8),
        ]),
        wall(1119, "Stone wall (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_Stone2.p3d", 12.0),
            (r"dz\structures_bliss\walls\Wall_Stone2D.p3d", 0.8),
        ]),
        wall(1120, "Tin fence (Livonia)", &[
            (r"dz\structures_bliss\walls\Wall_Tin6_1_4.p3d", 1.0),
            (r"dz\structures_bliss\walls\Wall_Tin6_2_4.p3d", 1.0),
        ]),
        // Additional Sakhal walls and fences
        wall(1121, "Green picket fence (Sakhal)", &[
            (r"dz\structures_sakhal\walls\Wall_VilVar2_Green_4.p3d", 6.0),
            (r"dz\structures_sakhal\walls\Wall_VilVar2_Green_4_2.p3d", 6.0),
            (r"dz\structures_sakhal\walls\Wall_VilVar2_Green_4_D.p3d", 0.8),
        ]),
        wall(1122, "Red picket fence (Sakhal)", &[
            (r"dz\structures_sakhal\walls\Wall_VilVar2_Red_4.p3d", 6.0),
            (r"dz\structures_sakhal\walls\Wall_VilVar2_Red_4_2.p3d", 6.0),
            (r"dz\structures_sakhal\walls\Wall_VilVar2_Red_4_D.p3d", 0.8),
        ]),
        wall(1123, "Low blue tin fence (Sakhal)", &[
            (r"dz\structures_sakhal\walls\Wall_Tin8_LowBlue_4.p3d", 12.0),
            (r"dz\structures_sakhal\walls\Wall_Tin8_LowBlue_4_D.p3d", 0.8),
        ]),
        wall(1124, "Low green tin fence (Sakhal)", &[
            (r"dz\structures_sakhal\walls\Wall_Tin8_LowGreen_4.p3d", 12.0),
            (r"dz\structures_sakhal\walls\Wall_Tin8_LowGreen_4_D.p3d", 0.8),
        ]),
        wall(1125, "Low red tin fence (Sakhal)", &[
            (r"dz\structures_sakhal\walls\Wall_Tin8_LowRed_4.p3d", 12.0),
            (r"dz\structures_sakhal\walls\Wall_Tin8_LowRed_4_D.p3d", 0.8),
        ]),
        wall(1126, "Low rusty tin fence (Sakhal)", &[
            (r"dz\structures_sakhal\walls\Wall_Tin8_LowRusty_4.p3d", 12.0),
            (r"dz\structures_sakhal\walls\Wall_Tin8_LowRusty_4_D.p3d", 0.8),
        ]),
        // Rocks and stones
        make(1020, "Rocks and stones", "Stones (Chernarus)", &[
            (r"dz\rocks\stone1.p3d", 1.0),
            (r"dz\rocks\stone2.p3d", 1.0),
            (r"dz\rocks\stone3.p3d", 1.0),
            (r"dz\rocks\stone4.p3d", 1.0),
            (r"dz\rocks\stone5.p3d", 1.0),
        ], 0.75, 1.3, true, true),
        make(1021, "Rocks and stones", "Rocks and boulders (Chernarus)", &[
            (r"dz\rocks\rock_monolith1.p3d", 1.0),
            (r"dz\rocks\rock_monolith2.p3d", 1.0),
            (r"dz\rocks\rock_monolith3.p3d", 1.0),
            (r"dz\rocks\rock_monolith4.p3d", 1.0),
            (r"dz\rocks\rock_wallh1.p3d", 1.0),
            (r"dz\rocks\rock_wallh2.p3d", 1.0),
            (r"dz\rocks\rock_wallh3.p3d", 1.0),
            (r"dz\rocks\rock_spike1.p3d", 1.0),
            (r"dz\rocks\rock_apart1.p3d", 1.0),
            (r"dz\rocks\rock_apart2.p3d", 1.0),
        ], 0.8, 1.2, true, false),
        make(1022, "Rocks and stones", "Mossy stones (Livonia)", &[
            (r"dz\rocks_bliss\stone6_moss.p3d", 1.0),
            (r"dz\rocks_bliss\stone7_moss.p3d", 1.0),
            (r"dz\rocks_bliss\stone8_moss.p3d", 1.0),
            (r"dz\rocks_bliss\stone9_moss.p3d", 1.0),
            (r"dz\rocks_bliss\stone10_moss.p3d", 1.0),
        ], 0.75, 1.3, true, true),
        make(1023, "Rocks and stones", "Snowy stones (Sakhal)", &[
            (r"dz\rocks_sakhal\stone6_snow.p3d", 1.0),
            (r"dz\rocks_sakhal\stone7_snow.p3d", 1.0),
            (r"dz\rocks_sakhal\stone8_snow.p3d", 1.0),
            (r"dz\rocks_sakhal\stone9_snow.p3d", 1.0),
            (r"dz\rocks_sakhal\stone10_snow.p3d", 1.0),
        ], 0.75, 1.3, true, true),
        make(1024, "Rocks and stones", "Volcanic rocks (Sakhal)", &[
            (r"dz\rocks_sakhal\rock_lava_boulder1.p3d", 1.0),
            (r"dz\rocks_sakhal\rock_lava_boulder2.p3d", 1.0),
            (r"dz\rocks_sakhal\rock_lava_boulder3.p3d", 1.0),
            (r"dz\rocks_sakhal\rock_lava_boulder4.p3d", 1.0),
            (r"dz\rocks_sakhal\stone14_lava.p3d", 1.0),
            (r"dz\rocks_sakhal\stone15_lava.p3d", 1.0),
            (r"dz\rocks_sakhal\stones_lava_cluster1.p3d", 1.0),
            (r"dz\rocks_sakhal\stones_lava_cluster2.p3d", 1.0),
        ], 0.8, 1.25, true, false),
        // Wrecks
        make(1030, "Wrecks", "Civilian car wrecks", &[
            (r"dz\structures\wrecks\vehicles\wreck_sed01_aban1_black.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\wreck_sed01_aban2_white.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\wreck_sed02_aban1_grey.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\wreck_sed02_aban2_red.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\wreck_hb01_aban1_blue.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\wreck_hb01_aban2_green.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\wreck_hb02_aban1_red.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\wreck_offroad02_aban1.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\wreck_truck01_aban1_blue.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_Volha_Grey.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_Lada_Red.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_S1023_Blue.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_V3S.p3d", 1.0),
        ], 1.0, 1.0, false, true),
        make(1031, "Wrecks", "Military vehicle wrecks", &[
            (r"dz\structures\wrecks\vehicles\Wreck_BMP1.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_BMP2.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_BRDM.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_HMMWV.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_MTVR_Cargo.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_Stryker.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_Uaz.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_Ural.p3d", 1.0),
        ], 1.0, 1.0, false, true),
        make(1032, "Wrecks", "Farm and construction wrecks", &[
            (r"dz\structures\wrecks\vehicles\Wreck_Baler.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_Bulldozer.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_Digger.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_Harverster.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_PowGen.p3d", 1.0),
            (r"dz\structures\wrecks\vehicles\Wreck_Trailer_Flat.p3d", 1.0),
        ], 1.0, 1.0, false, true),
        // Props and debris
        make(1040, "Props and debris", "Garbage and rubble", &[
            (r"dz\structures\residential\misc\Garbage_Pile1.p3d", 1.0),
            (r"dz\structures\residential\misc\Garbage_Pile2.p3d", 1.0),
            (r"dz\structures\residential\misc\Garbage_Pile3.p3d", 1.0),
            (r"dz\structures\residential\misc\Garbage_Pile4.p3d", 1.0),
            (r"dz\structures\residential\misc\Garbage_Pile5.p3d", 1.0),
            (r"dz\structures\residential\misc\Garbage_Pile6.p3d", 1.0),
            (r"dz\structures\residential\misc\Garbage_Bin.p3d", 0.6),
            (r"dz\structures\residential\misc\Garbage_Container.p3d", 0.4),
        ], 0.95, 1.05, true, true),
        make(1041, "Props and debris", "Industrial pallets and barrels", &[
            (r"dz\structures\industrial\misc\misc_barrelempty.p3d", 1.0),
            (r"dz\structures\industrial\misc\misc_pallet.p3d", 1.0),
            (r"dz\structures\industrial\misc\misc_pallets1.p3d", 1.0),
            (r"dz\structures\industrial\misc\misc_pallets2.p3d", 1.0),
            (r"dz\structures\industrial\misc\misc_tirepile.p3d", 1.0),
            (r"dz\structures\industrial\misc\misc_tire_big.p3d", 0.7),
            (r"dz\structures\industrial\misc\misc_boardspack1.p3d", 1.0),
            (r"dz\structures\industrial\misc\misc_coil.p3d", 0.7),
            (r"dz\structures\industrial\misc\misc_timbers1.p3d", 1.0),
        ], 0.95, 1.05, true, true),
        make(1042, "Props and debris", "Yard and woodpiles", &[
            (r"dz\structures\residential\misc\Misc_WoodPile1.p3d", 1.5),
            (r"dz\structures\residential\misc\Misc_WoodPile2.p3d", 1.5),
            (r"dz\structures\residential\misc\Misc_WoodReserve.p3d", 1.0),
            (r"dz\structures\residential\misc\Misc_BoxWooden.p3d", 1.0),
            (r"dz\structures\residential\misc\Misc_DogHouse.p3d", 0.6),
            (r"dz\structures\residential\misc\Misc_Hutch.p3d", 0.6),
            (r"dz\structures\residential\misc\Misc_WoodBlock.p3d", 1.0),
            (r"dz\structures\residential\misc\Misc_WoodTable_Outdoor.p3d", 0.7),
        ], 0.95, 1.05, true, true),
        make(1043, "Props and debris", "Hay bales and farm props", &[
            (r"dz\structures\industrial\misc\misc_haybale.p3d", 4.0),
            (r"dz\structures\industrial\misc\misc_haybale_decayed.p3d", 1.0),
            (r"dz\structures\residential\misc\Misc_HayStack.p3d", 1.2),
            (r"dz\structures\residential\misc\Misc_FeedRack.p3d", 0.8),
            (r"dz\structures\industrial\misc\misc_timbers_log1.p3d", 1.0),
            (r"dz\structures\industrial\misc\misc_timbers_log2.p3d", 1.0),
        ], 0.95, 1.05, true, true),
        make(1044, "Props and debris", "Military crates and sandbags", &[
            (r"dz\structures\military\misc\misc_bagfence_3m.p3d", 1.0),
            (r"dz\structures\military\misc\misc_supplybox1.p3d", 1.0),
            (r"dz\structures\military\misc\misc_supplybox2.p3d", 1.0),
            (r"dz\structures\military\misc\misc_supplybox3.p3d", 1.0),
            (r"dz\structures\military\misc\misc_concreteblock1.p3d", 1.0),
            (r"dz\structures\military\misc\misc_cover_small.p3d", 0.8),
        ], 1.0, 1.0, true, true),
        // Street and roadside
        make(1050, "Street and roadside", "Street lamps", &[
            (r"dz\structures\residential\misc\Lamp_City1.p3d", 1.0),
            (r"dz\structures\residential\misc\Lamp_City2.p3d", 1.0),
            (r"dz\structures\residential\misc\Lamp_City3.p3d", 1.0),
        ], 1.0, 1.0, false, false),
        make(1051, "Street and roadside", "Traffic signs (warning)", &[
            (r"dz\structures\signs\traffic\sign_danger.p3d", 1.0),
            (r"dz\structures\signs\traffic\sign_deer.p3d", 1.0),
            (r"dz\structures\signs\traffic\sign_cow.p3d", 1.0),
            (r"dz\structures\signs\traffic\sign_attention.p3d", 1.0),
            (r"dz\structures\signs\traffic\sign_downHill.p3d", 1.0),
            (r"dz\structures\signs\traffic\sign_upHill.p3d", 1.0),
            (r"dz\structures\signs\traffic\sign_badRoadside.p3d", 1.0),
        ], 1.0, 1.0, false, false),
        make(1052, "Street and roadside", "Park benches", &[
            (r"dz\structures\residential\misc\Misc_Bench1.p3d", 1.0),
            (r"dz\structures\residential\misc\Misc_Bench2.p3d", 1.0),
            (r"dz\structures\residential\misc\Misc_Bench3.p3d", 1.0),
            (r"dz\structures\residential\misc\Misc_Bench4.p3d", 1.0),
            (r"dz\structures\residential\misc\Misc_Bench5.p3d", 1.0),
        ], 1.0, 1.0, false, false),
        make(1053, "Street and roadside", "Roadside bushes", &[
            (r"dz\plants\bush\b_corylusAvellana_1f.p3d", 1.0),
            (r"dz\plants\bush\b_prunusSpinosa_1s.p3d", 1.0),
            (r"dz\plants\bush\b_rosaCanina_1s.p3d", 1.0),
            (r"dz\plants\bush\b_sambucusNigra_1s.p3d", 1.0),
            (r"dz\plants\bush\b_crataegusLaevigata_1s.p3d", 1.0),
        ], 0.85, 1.2, true, false),
        make(1054, "Street and roadside", "Shore reeds and shrubs", &[
            (r"dz\plants\clutter\p_Phragmites.p3d", 3.0),
            (r"dz\plants\bush\b_betulaHumilis_1s.p3d", 1.5),
            (r"dz\plants\bush\b_corylusAvellana_2s.p3d", 1.0),
        ], 0.85, 1.25, true, false),
    ]
}

/// The prefabs every installation starts with; they cannot be changed, only duplicated.
pub fn built_in_prefabs() -> Vec<Prefab> {
    let item = |model: &str, x: f32, z: f32, yaw: f32| PrefabItem {
        model: model.to_string(),
        x,
        z,
        elevation: 0.0,
        yaw,
        pitch: 0.0,
        roll: 0.0,
        scale: 1.0,
        flags: 0,
    };
    let make = |id, name: &str, items: Vec<PrefabItem>| Prefab {
        id,
        name: name.to_string(),
        folder: BUILT_IN.to_string(),
        items,
    };
    vec![
        make(2001, "Bus stop (village)", vec![
            item(r"dz\structures\residential\bus\BusStop_Village.p3d", 0.0, 0.0, 270.0),
            item(r"dz\structures\residential\bus\BusStop_Village_Sign.p3d", 3.4, 1.4, 0.0),
            item(r"dz\structures\residential\misc\Garbage_Bin.p3d", -3.0, 0.6, 0.0),
            item(r"dz\structures\residential\misc\Misc_Bench1.p3d", 3.5, -0.4, 0.0),
        ]),
        make(2002, "Forest camp", vec![
            item(r"dz\structures\residential\misc\Misc_WoodTable_Outdoor.p3d", -1.4, 0.0, 0.0),
            item(r"dz\structures\residential\misc\Misc_Table_Camp.p3d", 2.0, 0.8, 15.0),
            item(r"dz\structures\residential\misc\Misc_Chair_Camp1.p3d", 1.8, 2.0, 195.0),
            item(r"dz\structures\residential\misc\Misc_Chair_Camp1.p3d", 2.3, -0.4, 15.0),
            item(r"dz\structures\residential\misc\Misc_WoodPile1.p3d", -2.2, -2.6, 90.0),
            item(r"dz\structures\residential\misc\Misc_WoodBlock.p3d", 0.4, -2.2, 30.0),
        ]),
        make(2003, "Hunting post", vec![
            item(r"dz\structures\residential\misc\Misc_DeerStand1.p3d", 0.0, 0.0, 0.0),
            item(r"dz\structures\residential\misc\Misc_FeedRack.p3d", 3.2, 2.8, 20.0),
            item(r"dz\structures\residential\misc\Misc_WoodPile2.p3d", -3.2, -0.6, 0.0),
        ]),
        make(2004, "Road checkpoint", vec![
            item(r"dz\structures\military\improvised\Mil_GuardShed.p3d", -3.8, 0.0, 90.0),
            item(r"dz\structures\residential\misc\Misc_RoadBarrier.p3d", -1.2, 0.0, 90.0),
            item(r"dz\structures\military\misc\Misc_BagFence_3m.p3d", -3.8, -2.5, 0.0),
            item(r"dz\structures\military\misc\Misc_BagFence_3m.p3d", -3.8, 2.5, 0.0),
            item(r"dz\structures\military\misc\Misc_BagFence_3m.p3d", 4.2, 1.4, 0.0),
            item(r"dz\structures\military\misc\Misc_Hedgehog_Iron.p3d", 4.4, -1.5, 35.0),
            item(r"dz\structures\industrial\misc\misc_barrelempty.p3d", -2.3, -1.8, 0.0),
        ]),
        make(2005, "Military outpost", vec![
            item(r"dz\structures\military\improvised\Mil_GuardShed.p3d", 0.0, -1.6, 0.0),
            item(r"dz\structures\military\misc\Misc_BagFence_3m.p3d", 0.0, 2.8, 0.0),
            item(r"dz\structures\military\misc\Misc_BagFence_3m.p3d", -3.4, 1.2, 40.0),
            item(r"dz\structures\military\misc\Misc_BagFence_3m.p3d", 3.4, 1.2, 320.0),
            item(r"dz\structures\military\misc\Misc_SupplyBox1.p3d", -3.0, -2.0, 15.0),
            item(r"dz\structures\military\misc\Misc_SupplyBox2.p3d", -3.2, -0.4, 0.0),
            item(r"dz\structures\military\misc\Misc_SupplyBox3.p3d", 3.0, -1.6, 90.0),
            item(r"dz\structures\industrial\misc\misc_barrelempty.p3d", 2.8, -0.1, 0.0),
            item(r"dz\structures\industrial\misc\misc_barrelempty.p3d", 3.4, 0.3, 40.0),
        ]),
        make(2006, "Transformer station", vec![
            item(r"dz\structures\industrial\power\Power_Transformer_Small.p3d", 0.0, 0.0, 0.0),
            item(r"dz\structures\walls\Wall_IndFnc_3.p3d", -1.5, -3.0, 0.0),
            item(r"dz\structures\walls\Wall_IndFnc_3.p3d", 1.5, -3.0, 0.0),
            item(r"dz\structures\walls\Wall_IndFnc_3.p3d", -3.0, -1.5, 90.0),
            item(r"dz\structures\walls\Wall_IndFnc_3.p3d", -3.0, 1.5, 90.0),
            item(r"dz\structures\walls\Wall_IndFnc_3.p3d", 3.0, -1.5, 90.0),
            item(r"dz\structures\walls\Wall_IndFnc_3.p3d", 3.0, 1.5, 90.0),
            item(r"dz\structures\walls\Wall_IndFnc_3.p3d", -1.5, 3.0, 0.0),
            item(r"dz\structures\industrial\misc\misc_coil.p3d", 5.4, -1.0, 20.0),
        ]),
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
    /// The mask modifier this generator made, if any.
    #[serde(default)]
    pub mask_modifier: Option<u32>,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// An id unlikely to meet another one, also across sessions.
pub fn new_id() -> u64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    // Each run starts the counter somewhere else, so two people working at the same moment
    // do not make the same ids.
    static SALT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let salt = *SALT.get_or_init(rand::random::<u64>);
    (now << 12) | (COUNTER.fetch_add(1, Ordering::Relaxed).wrapping_add(salt) & 0xfff)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_prefab_keeps_its_layout_wherever_it_is_placed() {
        use crate::objects::{Obj, FLAG_ABSOLUTE, FLAG_UNDERGROUND};
        let mut h = crate::terrain::Heightmap::new(64, 64, 4.0, 10.0);
        for j in 0..64 { for i in 0..64 { h.data[j * 64 + i] = i as f32; } }
        let o = |x, z, yaw, elevation, flags| Obj { x, z, elevation, yaw, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags };
        // A crate on the ground and a lamp at a fixed height, 4 m over the ground at the middle.
        // A crate on the ground, a lamp at a fixed height, a cellar door underground.
        let objects = vec![("a.p3d".to_string(), o(100.0, 100.0, 0.0, 0.0, 3 << 16)), ("b.p3d".to_string(), o(110.0, 100.0, 90.0, 26.25 + 4.0, FLAG_ABSOLUTE)), ("c.p3d".to_string(), o(105.0, 100.0, 0.0, -1.0, FLAG_UNDERGROUND | (2 << 16)))];
        let p = Prefab::from_objects(1, "Set".into(), &objects, &h);
        assert_eq!((p.items[0].x, p.items[1].x), (-5.0, 5.0));
        assert_eq!(p.items[0].flags, 0, "no layer");
        assert_eq!((p.items[1].flags, p.items[2].flags), (FLAG_ABSOLUTE, FLAG_UNDERGROUND), "flags but no layer");
        let placed = p.objects_at(200.0, 50.0, 90.0, &h);
        // Turned a quarter: east becomes south, as objects turn.
        assert!((placed[0].1.x - 200.0).abs() < 1e-3 && (placed[0].1.z - 55.0).abs() < 1e-3, "{:?}", placed[0].1);
        assert!((placed[1].1.z - 45.0).abs() < 1e-3);
        assert_eq!(placed[1].1.yaw, 180.0);
        assert!((placed[1].1.elevation - (h.sample(200.0, 50.0) + 4.0)).abs() < 0.01, "{}", placed[1].1.elevation);
        // Placed, each keeps its flags; the cellar door stays underground at its depth.
        assert_eq!((placed[0].1.flags, placed[1].1.flags, placed[2].1.flags), (0, FLAG_ABSOLUTE, FLAG_UNDERGROUND));
        assert_eq!(placed[2].1.elevation, -1.0);
        // In its own group of the library, and folders made there.
        let mut l = Library::default();
        l.prefabs.push(p);
        let f = l.new_library_folder(PREFABS_ROOT, "Camps");
        assert_eq!(l.move_to_library_folder(&f, &[], &[]), 0);
        assert_eq!(l.move_prefabs_to_library_folder(&f, &[1]), 1);
        assert_eq!(l.move_prefabs_to_library_folder(GENERATORS_ROOT, &[1]), 0, "not among the generators");
        l.delete_library_folder(&f);
        assert_eq!(l.prefabs[0].folder, "");
    }

    #[test]
    fn library_folders_hold_their_kind_and_give_it_back_when_deleted() {
        let mut l = Library::default();
        l.generators.push(GeneratorPreset { id: 1, name: "Road".into(), folder: String::new(), kind: built_in_generators()[0].kind.clone() });
        l.objects.push(ObjectPreset { id: 2, name: "Rocks".into(), folder: String::new(), items: Vec::new(), scale_min: 1.0, scale_max: 1.0, random_yaw: true, align: false });
        let roads = l.new_library_folder(GENERATORS_ROOT, "Roads");
        assert_eq!(roads, "#generators/Roads");
        let mine = l.new_library_folder("", "#Mine");
        assert_eq!(mine, "Mine", "no name of a group");
        // A preset does not go among the generators; the top takes both.
        assert_eq!(l.move_to_library_folder(&roads, &[1], &[2]), 1);
        assert_eq!(l.generators[0].folder, "#generators/Roads");
        assert_eq!(l.objects[0].folder, "");
        assert_eq!(l.move_to_library_folder(&mine, &[], &[2]), 1);
        assert_eq!(l.library_home(&l.objects[0].folder.clone(), LibraryItem::Preset), "Mine");
        // The groups are fixed.
        assert!(!l.rename_library_folder(GENERATORS_ROOT, "Gens"));
        l.delete_library_folder(PRESETS_ROOT);
        assert!(l.library_folder_exists(PRESETS_ROOT));
        assert!(l.rename_library_folder(&roads, "Streets"));
        assert_eq!(l.generators[0].folder, "#generators/Streets");
        let inner = l.new_library_folder("Mine", "Inner");
        l.move_to_library_folder(&inner, &[], &[2]);
        l.delete_library_folder("Mine");
        assert!(!l.library_folder_exists(&inner));
        assert_eq!(l.objects[0].folder, "", "back in its group");
        // Into the group again.
        l.move_to_library_folder(GENERATORS_ROOT, &[1], &[]);
        assert_eq!(l.generators[0].folder, "");
        // A folder that is gone shows its things in the group.
        assert_eq!(l.library_home("Gone", LibraryItem::Generator), "");
    }

    #[test]
    fn folders_nest_rename_with_what_lies_in_them_and_delete_alone() {
        let mut l = super::Library::default();
        let trees = l.new_folder("", "Trees");
        let birch = l.new_folder(&trees, "Birch");
        assert_eq!(birch, "Trees/Birch");
        assert_eq!(l.new_folder("", "Trees"), "Trees 2");
        l.add_to_folder(&birch, &[r"DZ\plants\tree\t_betula.p3d".to_string()], &[7]);
        l.add_to_folder(&birch, &[r"dz\plants\tree\t_betula.p3d".to_string()], &[7]);
        assert_eq!((l.folder(&birch).unwrap().models.len(), l.folder(&birch).unwrap().presets.len()), (1, 1));
        assert!(l.rename_folder("Trees", "Forest"));
        assert!(l.folder("Forest/Birch").is_some());
        assert!(!l.rename_folder("Forest", "Trees 2"), "the name is taken");
        // A folder named with a path brings the folders above it.
        l.add_to_folder("Rocks/Big", &[], &[1]);
        assert!(l.folder("Rocks").is_some());
        l.delete_folder("Forest");
        assert!(l.folder("Forest/Birch").is_none() && l.folder("Trees 2").is_some());
        assert!(super::in_folder("Rocks/Big", "Rocks") && !super::in_folder("Rocksy", "Rocks"));
    }
    #[test]
    fn a_model_keeps_its_surface_whatever_the_form_of_its_path() {
        let mut l = super::Library::default();
        l.set_surface(r"DZ\plants\tree\t_BetulaPendula_3s.p3d", Some("forest"));
        assert_eq!(l.surface_of(r"\dz\plants\tree\t_betulapendula_3s.p3d"), Some("forest"));
        assert_eq!(l.surface_of("dz/plants/tree/t_betulapendula_3s.p3d"), Some("forest"));
        let back: super::Library = serde_json::from_slice(&serde_json::to_vec(&l).unwrap()).unwrap();
        assert_eq!(back.surface_of(r"dz\plants\tree\t_betulapendula_3s.p3d"), Some("forest"));
        l.set_surface(r"dz\plants\tree\t_betulapendula_3s.p3d", None);
        assert!(l.model_surfaces.is_empty());
    }
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
        assert_eq!(names.len(), 42);
        assert!(names.contains(&"River") && names.contains(&"Area: Random") && names.contains(&"Terrain shape") && names.contains(&"Line: Nodes Objects") && names.contains(&"Orchard") && names.contains(&"Tree alley") && names.contains(&"Wetland and marsh") && names.contains(&"Roadblock and checkpoint") && names.contains(&"Grid area") && names.contains(&"Trenches") && names.contains(&"Tourist trail"));
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

    #[test]
    fn built_in_objects_are_valid_and_damaged_fences_are_rare() {
        let gens: std::collections::HashSet<u64> = built_in_generators().iter().map(|g| g.id).collect();
        let objs = built_in_objects();
        assert!(!objs.is_empty());
        let mut seen = std::collections::HashSet::new();
        for p in &objs {
            assert!(seen.insert(p.id), "duplicate built-in object id {}", p.id);
            assert!(!gens.contains(&p.id), "id {} collides with built-in generator", p.id);
            assert!(p.is_built_in());
            let cat = p.built_in_category().expect("built-in object has a category");
            assert!(BUILT_IN_OBJECT_CATEGORIES.contains(&cat), "unknown category {cat}");
            assert!(!p.items.is_empty(), "empty preset {}", p.name);
            assert!(p.scale_min > 0.0 && p.scale_max >= p.scale_min, "bad scale in {}", p.name);
            for (m, w) in &p.items {
                assert!(!m.is_empty() && *w > 0.0, "bad item in {}", p.name);
            }
            if cat == "Walls and fences" {
                let total: f32 = p.items.iter().map(|(_, w)| *w).sum();
                let broken: f32 = p.items.iter().filter(|(m, _)| {
                    let s = m.to_ascii_lowercase();
                    s.contains("_d.") || s.contains("_d_") || s.contains("1d_") || s.contains("stoned.") || s.contains("stone2d.") || s.contains("hole")
                }).map(|(_, w)| *w).sum();
                assert!(broken / total <= 0.12, "damaged share too high in {}: {:.1}%", p.name, broken / total * 100.0);
            }
        }
        // Built-in presets resolve through Library::all_objects and Library::find_object.
        let lib = Library::default();
        let first = &objs[0];
        assert_eq!(lib.find_object(first.id).as_ref(), Some(first));
        let set = ObjectSet { preset: Some(first.id), ..Default::default() };
        let resolved = set.resolved(&lib.all_objects());
        assert_eq!(resolved.models, first.items);

        // When P:\dz is mounted locally, every built-in model path must exist on disk.
        let drive = std::path::Path::new(r"P:\dz");
        if drive.is_dir() {
            for p in &objs {
                for (m, _) in &p.items {
                    let full = std::path::Path::new(r"P:\").join(m);
                    assert!(full.is_file(), "missing model in {}: {}", p.name, full.display());
                }
            }
            for pf in built_in_prefabs() {
                assert!(pf.is_built_in() && !pf.items.is_empty());
                assert!(seen.insert(pf.id) && !gens.contains(&pf.id), "duplicate id {}", pf.id);
                for it in &pf.items {
                    let full = std::path::Path::new(r"P:\").join(&it.model);
                    assert!(full.is_file(), "missing prefab model in {}: {}", pf.name, full.display());
                }
            }
        }
    }
}
