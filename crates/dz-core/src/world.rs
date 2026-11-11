//! The edited map: project settings, rasters, objects and non-destructive height modifiers.
use crate::objects::{Obj, ObjectStore};
use crate::terrain::{Heightmap, MaskMap, Rect, RgbImage};
use serde::{Deserialize, Serialize};

fn default_tile() -> u32 {
    512
}
fn default_cells() -> u32 {
    8
}
fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorldMeta {
    /// Display name.
    pub name: String,
    /// Class and folder name used by the game (`P:\<map_id>`).
    pub map_id: String,
    pub author: String,
    /// Heightmap vertices per side.
    pub terrain_size: u32,
    /// Heightmap cell in metres.
    pub terrain_cell: f32,
    /// Sea level; the game's ocean is at 0, everything else is offset on export.
    pub sea_level: f32,
    /// Terrain Builder easting offset, used when reading and writing TB files.
    pub easting: f64,
    pub northing: f64,
    #[serde(default = "default_tile")]
    pub tile_px: u32,
    /// Land grid cells per satellite tile step.
    #[serde(default = "default_cells")]
    pub cells_per_tile: u32,
    /// Player spawn on the map (x, z).
    #[serde(default)]
    pub spawn: [f32; 2],
    #[serde(default)]
    pub latitude: f32,
    #[serde(default)]
    pub longitude: f32,
    #[serde(default = "default_true")]
    pub write_underground_flag: bool,
    /// Most surfaces one texture tile may use.
    #[serde(default = "default_tile_surfaces")]
    pub max_tile_surfaces: u8,
}

impl WorldMeta {
    pub fn new(name: &str, size: u32, cell: f32) -> Self {
        let map_id: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        let map_id = if map_id.is_empty() { "NewMap".to_string() } else { map_id };
        let side = size as f32 * cell;
        Self {
            max_tile_surfaces: 6, name: name.to_string(), map_id, author: String::new(), terrain_size: size, terrain_cell: cell,
            sea_level: 0.0, easting: 200000.0, northing: 0.0, tile_px: 512, cells_per_tile: 8,
            spawn: [side / 2.0, side / 2.0], latitude: 54.0, longitude: 23.0, write_underground_flag: true,
        }
    }
    pub fn size_m(&self) -> f32 {
        self.terrain_size as f32 * self.terrain_cell
    }
    /// Land grid cells per side.
    pub fn land_size(&self) -> u32 {
        self.terrain_size / self.cells_per_tile
    }
    pub fn land_cell(&self) -> f32 {
        self.size_m() / self.land_size() as f32
    }
    pub fn tiles(&self) -> u32 {
        self.land_size() / self.cells_per_tile
    }
    /// Distance between tile origins in metres.
    pub fn tile_step(&self) -> f32 {
        self.land_cell() * self.cells_per_tile as f32
    }
}

/// One surface type of the terrain.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SurfaceLayer {
    pub name: String,
    /// `_ca.paa` detail texture, relative to P:\.
    pub texture: String,
    /// Surface `.rvmat`, relative to P:\.
    pub material: String,
    /// Colour in the mask image.
    pub color: [u8; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ModShape {
    /// Radius in metres.
    Circle { radius: f32 },
    /// Half extents in metres, rotated by the modifier's yaw.
    Rect { half_x: f32, half_z: f32 },
    /// A strip along a polyline, levelled to `heights` at its points.
    Line {
        points: Vec<[f32; 2]>,
        heights: Vec<f32>,
        half_width: f32,
        /// Segments left alone, such as the gap under a bridge.
        #[serde(default)]
        gaps: Vec<u32>,
        /// Slopes for the sides; without it they follow the blend distance.
        #[serde(default)]
        bank: Option<LineBank>,
    },
    /// Basins dug under water standing at their own level.
    Pond {
        areas: Vec<PondArea>,
        /// Depth of the floor under the water, in metres.
        depth: f32,
        /// Width of the slope from the shore down to the floor, in metres.
        blend: f32,
        /// Width of the strip around a basin raised to hold the water, in metres.
        bank: f32,
    },
}

/// The outline of one pond and the level of its water.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PondArea {
    pub points: Vec<[f32; 2]>,
    pub level: f32,
}

impl PondArea {
    fn contains(&self, x: f32, z: f32) -> bool {
        let mut inside = false;
        let mut b = *self.points.last().unwrap_or(&[0.0, 0.0]);
        for a in &self.points {
            if (a[1] > z) != (b[1] > z) && x < a[0] + (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1]) {
                inside = !inside;
            }
            b = *a;
        }
        inside
    }
    /// Distance to the outline: negative inside.
    fn signed_distance(&self, x: f32, z: f32) -> f32 {
        let mut d = f32::MAX;
        let mut b = *self.points.last().unwrap_or(&[0.0, 0.0]);
        for a in &self.points {
            d = d.min(segment_distance(b, *a, x, z).0);
            b = *a;
        }
        if self.contains(x, z) { -d } else { d }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum ModMode {
    /// Level the shape to one height.
    Flatten,
    /// Move the terrain by `amount` metres, keeping its shape.
    Offset,
    /// Only raise terrain that lies below the target.
    Fill,
    /// Only lower terrain that lies above the target.
    Cut,
}

/// Non-destructive height change, usually attached to an object.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TerrainModifier {
    pub id: u64,
    pub name: String,
    pub enabled: bool,
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    pub shape: ModShape,
    pub mode: ModMode,
    /// Target height for flatten, fill and cut; `None` uses the base height at the centre.
    pub target: Option<f32>,
    /// Metres added to the target, or moved in offset mode.
    pub amount: f32,
    /// Width of the blend to the untouched terrain in metres.
    pub falloff: f32,
    /// Id of the object the modifier follows.
    pub attached: Option<u32>,
}

impl TerrainModifier {
    /// Half size of the bounding square including the falloff.
    pub fn reach(&self) -> f32 {
        self.falloff + match &self.shape {
            ModShape::Circle { radius } => *radius,
            ModShape::Rect { half_x, half_z } => (half_x * half_x + half_z * half_z).sqrt(),
            ModShape::Pond { areas, bank, .. } => bank + areas.iter().flat_map(|a| &a.points).map(|p| (p[0] - self.x).abs().max((p[1] - self.z).abs())).fold(0.0, f32::max) * std::f32::consts::SQRT_2,
            ModShape::Line { points, half_width, .. } => half_width + points.iter().map(|p| (p[0] - self.x).abs().max((p[1] - self.z).abs())).fold(0.0, f32::max),
        }
    }
    /// Blend weight at a world position: 1 inside the shape, down to 0 at the falloff edge.
    #[inline]
    pub fn weight(&self, x: f32, z: f32, sin: f32, cos: f32) -> f32 {
        let (dx, dz) = (x - self.x, z - self.z);
        let d = match &self.shape {
            ModShape::Circle { radius } => (dx * dx + dz * dz).sqrt() - radius,
            ModShape::Pond { areas, .. } => areas.iter().map(|a| a.signed_distance(x, z)).fold(f32::MAX, f32::min),
            ModShape::Line { points, half_width, .. } => points.windows(2).map(|w| segment_distance(w[0], w[1], x, z).0).fold(f32::MAX, f32::min) - half_width,
            ModShape::Rect { half_x, half_z } => {
                let u = dx * cos - dz * sin;
                let v = dx * sin + dz * cos;
                let (a, b) = (u.abs() - *half_x, v.abs() - *half_z);
                let out = (a.max(0.0).powi(2) + b.max(0.0).powi(2)).sqrt();
                out + a.max(b).min(0.0)
            }
        };
        if d <= 0.0 {
            1.0
        } else if d >= self.falloff || self.falloff <= 0.0 {
            0.0
        } else {
            let t = 1.0 - d / self.falloff;
            t * t * (3.0 - 2.0 * t)
        }
    }
}

fn default_tile_surfaces() -> u8 {
    6
}

/// Distance from a point to a segment, and how far along the segment its nearest point is.
fn segment_distance(a: [f32; 2], b: [f32; 2], x: f32, z: f32) -> (f32, f32) {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let t = (((x - a[0]) * dx + (z - a[1]) * dz) / (dx * dx + dz * dz).max(1e-4)).clamp(0.0, 1.0);
    (((a[0] + dx * t - x).powi(2) + (a[1] + dz * t - z).powi(2)).sqrt(), t)
}

/// Digs basins and raises low ground around them so the water stays in.
fn apply_pond(height: &mut Heightmap, m: &TerrainModifier, area: Rect) {
    let ModShape::Pond { areas, depth, blend, bank } = &m.shape else { return };
    let cell = height.cell;
    let smooth = |t: f32| { let t = t.clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) };
    for a in areas.iter().filter(|a| a.points.len() >= 3) {
        let reach = bank + m.falloff;
        let b = a.points.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
        let lo = |v: f32, min: usize| (((v - reach) / cell).floor().max(min as f32)) as usize;
        let hi = |v: f32, max: usize| (((v + reach) / cell).ceil().max(0.0) as usize).min(max);
        let shore = a.level + 0.25;
        let floor = a.level - depth;
        for j in lo(b[1], area.y0)..=hi(b[3], area.y1) {
            for i in lo(b[0], area.x0)..=hi(b[2], area.x1) {
                let d = a.signed_distance(i as f32 * cell, j as f32 * cell);
                let v = height.at(i, j);
                // The ground at the shore, raised where it would let the water out.
                let rim = if *bank > 0.0 { v.max(shore) } else { v };
                let new = if d <= 0.0 {
                    let k = if *blend > 0.0 { smooth(-d / blend) } else { 1.0 };
                    rim + (rim.min(floor) - rim) * k
                } else if *bank > 0.0 && d < reach {
                    let k = if d <= *bank || m.falloff <= 0.0 { 1.0 } else { smooth(1.0 - (d - bank) / m.falloff) };
                    v + (rim - v) * k
                } else {
                    continue;
                };
                height.set(i, j, new);
            }
        }
    }
}

/// Levels the terrain along a strip. Walks the segments, so a long road costs no more than
/// the ground it actually covers.
fn apply_line(height: &mut Heightmap, m: &TerrainModifier, area: Rect, bed_only: bool) {
    let ModShape::Line { points, heights, half_width, gaps, bank } = &m.shape else { return };
    let (x0, y0) = (area.x0, area.y0);
    let (w, h) = (area.x1 - x0 + 1, area.y1 - y0 + 1);
    let mut best = vec![(f32::MAX, 0.0f32); w * h];
    let cell = height.cell;
    let reach = half_width + m.falloff;
    for (index, (seg, profile)) in points.windows(2).zip(heights.windows(2)).enumerate() {
        if gaps.contains(&(index as u32)) {
            continue;
        }
        let bounds = |lo: f32, hi: f32, min: usize, max: usize| (((lo - reach) / cell).floor().max(min as f32) as usize, ((hi + reach) / cell).ceil().min(max as f32) as usize);
        let (i0, i1) = bounds(seg[0][0].min(seg[1][0]), seg[0][0].max(seg[1][0]), area.x0, area.x1);
        let (j0, j1) = bounds(seg[0][1].min(seg[1][1]), seg[0][1].max(seg[1][1]), area.y0, area.y1);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (d, t) = segment_distance(seg[0], seg[1], i as f32 * cell, j as f32 * cell);
                let slot = &mut best[(j - y0) * w + i - x0];
                if d < slot.0 {
                    *slot = (d, profile[0] + (profile[1] - profile[0]) * t);
                }
            }
        }
    }
    for j in 0..h {
        for i in 0..w {
            let (d, goal) = best[j * w + i];
            if d >= reach {
                continue;
            }
            if bed_only {
                // Laid again after every other line, so that no bank covers a road or a track.
                if d <= *half_width + bank.as_ref().map_or(0.0, |b| b.shoulder) {
                    height.set(x0 + i, y0 + j, goal + m.amount);
                }
                continue;
            }
            if let Some(b) = bank {
                let v = height.at(x0 + i, y0 + j);
                let level = goal + m.amount;
                let bed = half_width + b.shoulder;
                // A ditch only makes sense where the road does not stand above the ground.
                let width = if b.ditch_depth > 0.0 && v >= level - 0.3 { b.ditch_width } else { 0.0 };
                let new = if d <= bed {
                    level
                } else if d < bed + width {
                    level - b.ditch_depth * (std::f32::consts::PI * (d - bed) / width).sin()
                } else {
                    let run = d - bed - width;
                    let side = if v < level { (level - b.fill * run).max(v) } else { (level + b.cut * run).min(v) };
                    // Far banks fade out instead of ending in a step.
                    let fade = ((reach - d) / (0.3 * (reach - bed - width)).max(0.01)).clamp(0.0, 1.0);
                    v + (side - v) * fade
                };
                height.set(x0 + i, y0 + j, new);
                continue;
            }
            let k = if d <= *half_width || m.falloff <= 0.0 { 1.0 } else { let t = 1.0 - (d - half_width) / m.falloff; t * t * (3.0 - 2.0 * t) };
            let v = height.at(x0 + i, y0 + j);
            height.set(x0 + i, y0 + j, v + (goal + m.amount - v) * k);
        }
    }
}

/// The sides of a road bed, set by slopes instead of a blend distance.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct LineBank {
    /// Level strip beside the road, in metres.
    pub shoulder: f32,
    /// Tangent of the slope of an embankment.
    pub fill: f32,
    /// Tangent of the slope of a cutting.
    pub cut: f32,
    pub ditch_depth: f32,
    pub ditch_width: f32,
}

/// How the terrain is shaped under a road.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RoadGrading {
    /// Level strip on each side of the road, in metres.
    pub shoulder: f32,
    /// Shape the sides as embankments and cuttings instead of a plain blend.
    pub embankment: bool,
    /// Steepest embankment, in degrees.
    pub fill_slope: f32,
    /// Steepest cutting, in degrees.
    pub cut_slope: f32,
    /// How far banks may reach from the road bed, in metres.
    pub max_bank: f32,
    pub ditch_depth: f32,
    pub ditch_width: f32,
    /// How far heights along the road are evened out, in metres.
    pub smooth: f32,
    /// Steepest climb along the road in percent; 0 for no limit.
    pub max_grade: f32,
    /// Raises or lowers the whole road bed, in metres.
    pub raise: f32,
}

impl Default for RoadGrading {
    fn default() -> Self {
        Self { shoulder: 1.0, embankment: true, fill_slope: 33.0, cut_slope: 40.0, max_bank: 40.0, ditch_depth: 0.0, ditch_width: 2.0, smooth: 28.0, max_grade: 0.0, raise: 0.0 }
    }
}

/// A road or other line drawn on the map.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct MapLine {
    pub name: String,
    /// `road family` such as `asf2`, or free text for shapes without models.
    pub family: String,
    pub points: Vec<[f32; 2]>,
    pub width: f32,
}

/// A model added to a road type: `category` 3 straight, 4 curve, 6 end.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RoadPart {
    pub family: String,
    pub category: u32,
    pub path: String,
}

/// The driving surface of a bridge: level, from one bank to the other.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct BridgeDeck {
    pub from: [f32; 2],
    pub to: [f32; 2],
    pub height: f32,
    /// Only the gap in the terrain matters: the heights around it are designed already.
    #[serde(default)]
    pub free: bool,
}

/// A road or track that stays editable: its waypoints and the objects fitted to them.
#[derive(Clone, Serialize, Deserialize)]
pub struct RoadRoute {
    pub name: String,
    pub family: String,
    /// Waypoints the user controls.
    pub points: Vec<[f32; 2]>,
    /// The centre line of the fitted models.
    pub centerline: Vec<[f32; 2]>,
    /// Ids of the models that make up the route.
    pub objects: Vec<u32>,
    /// Stable identity, for the generators that follow this route.
    #[serde(default)]
    pub id: u64,
    /// Bridges of the route, kept so the terrain can be ramped up to them.
    #[serde(default)]
    pub decks: Vec<BridgeDeck>,
    /// Searched across the terrain, or fitted straight along the waypoints.
    pub terrain_route: bool,
    pub settings: dayz_road_tool::document::RoutingSettings,
    /// Level the terrain under the route with a modifier.
    #[serde(default)]
    pub grade: bool,
    #[serde(default = "default_grade_width")]
    pub grade_width: f32,
    #[serde(default = "default_grade_falloff")]
    pub grade_falloff: f32,
    #[serde(default)]
    pub grading: RoadGrading,
    /// (x, z, height) of the designed line, when cut and fill limits or rigid track decide it.
    #[serde(default)]
    pub profile: Vec<[f32; 3]>,
    /// The modifier that levels the terrain under this route.
    #[serde(default)]
    pub modifier: Option<u64>,
}

fn default_grade_width() -> f32 {
    9.0
}
fn default_grade_falloff() -> f32 {
    12.0
}

/// A named group of placed objects. Objects refer to layers by index, so a deleted layer
/// stays in the list as a tombstone and undo keeps working.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ObjectLayer {
    pub name: String,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default = "yes")]
    pub editable: bool,
    #[serde(default = "yes")]
    pub export: bool,
    #[serde(default)]
    pub deleted: bool,
}

fn yes() -> bool {
    true
}

impl ObjectLayer {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), visible: true, editable: true, export: true, deleted: false }
    }
    /// Tools may pick and change the objects of this layer.
    pub fn interactive(&self) -> bool {
        self.visible && self.editable && !self.deleted
    }
}

pub fn default_object_layers() -> Vec<ObjectLayer> {
    vec![ObjectLayer::new("Default")]
}

#[derive(Clone)]
pub struct World {
    pub meta: WorldMeta,
    /// Heights as sculpted, before modifiers.
    pub base: Heightmap,
    /// Heights after modifiers; this is what is shown and exported.
    pub height: Heightmap,
    pub mask: MaskMap,
    pub layers: Vec<SurfaceLayer>,
    /// The image with the satellite modifiers laid over it; see `satmod`.
    pub satellite: Option<RgbImage>,
    pub sat: crate::satmod::SatLayers,
    /// The modifiers of the surface mask; `mask` is the painted mask with them over it.
    pub mask_mods: crate::maskmod::MaskLayers,
    pub objects: ObjectStore,
    pub modifiers: Vec<TerrainModifier>,
    /// Never empty; layer 0 cannot be deleted.
    pub object_layers: Vec<ObjectLayer>,
    /// Layer that receives new objects.
    pub active_layer: u16,
    pub shape_layers: Vec<crate::shapes::ShapeLayer>,
    pub active_shape_layer: usize,
    pub generators: Vec<crate::library::GeneratorInstance>,
    /// File names of road parts left out of the catalog.
    pub road_removed: std::collections::BTreeSet<String>,
    /// Road parts added to the catalog.
    pub road_added: Vec<RoadPart>,
    /// Terrain Builder project whose road types are used for new roads.
    pub road_project: Option<std::path::PathBuf>,
    /// Terrain Builder template libraries (`.tml`) read into the project.
    pub template_libraries: Vec<std::path::PathBuf>,
    pub routes: Vec<RoadRoute>,
    /// Named places of the map.
    pub keypoints: Vec<crate::keypoints::KeyPoint>,
    /// Terrain cells the game does not draw.
    pub holes: crate::holes::Holes,
    /// Goes up whenever `holes` changes, so the view can follow.
    pub holes_rev: u32,
    next_modifier: u64,
}

pub fn default_layers() -> Vec<SurfaceLayer> {
    let l = |name: &str, tex: &str, color: [u8; 3]| SurfaceLayer {
        name: name.into(),
        texture: format!("dz\\surfaces_bliss\\data\\terrain\\{tex}_ca.paa"),
        material: format!("dz\\surfaces_bliss\\data\\terrain\\{tex}.rvmat"),
        color,
    };
    vec![
        l("grass1", "en_grass1", [108, 142, 66]),
        l("grass2", "en_grass2", [136, 156, 74]),
        l("forest_dec", "en_forest_dec", [70, 88, 44]),
        l("forest_con", "en_forest_con", [52, 66, 40]),
        l("soil", "en_soil", [120, 96, 70]),
        l("stubble", "en_stubble", [176, 160, 104]),
        l("stones", "en_stones", [140, 138, 132]),
        l("tarmac_old", "en_tarmac_old", [84, 84, 86]),
    ]
}

impl World {
    pub fn new(meta: WorldMeta, base_height: f32) -> Self {
        let n = meta.terrain_size as usize;
        let base = Heightmap::new(n, n, meta.terrain_cell, base_height);
        let mask_px = (meta.size_m().round() as usize).clamp(1024, 20480);
        Self {
            sat: Default::default(), mask_mods: Default::default(),
            holes: Default::default(),
            holes_rev: 0, keypoints: Vec::new(),
            height: base.clone(),
            base,
            mask: MaskMap::new(mask_px, mask_px, 0),
            layers: default_layers(),
            satellite: None,
            objects: ObjectStore::default(),
            modifiers: Vec::new(),
            object_layers: default_object_layers(),
            active_layer: 0,
            shape_layers: Vec::new(),
            active_shape_layer: 0,
            generators: Vec::new(),
            road_removed: Default::default(),
            road_added: Vec::new(),
            road_project: None, template_libraries: Vec::new(),
            routes: Vec::new(),
            meta,
            next_modifier: 1,
        }
    }
    pub fn from_parts(meta: WorldMeta, base: Heightmap, mask: MaskMap, layers: Vec<SurfaceLayer>, satellite: Option<RgbImage>, objects: ObjectStore, modifiers: Vec<TerrainModifier>) -> Self {
        let next_modifier = modifiers.iter().map(|m| m.id).max().unwrap_or(0) + 1;
        let mut w = Self { height: base.clone(), base, meta, mask, layers, satellite, sat: Default::default(), mask_mods: Default::default(), objects, modifiers, holes: Default::default(), holes_rev: 0, keypoints: Vec::new(), object_layers: default_object_layers(), active_layer: 0, shape_layers: Vec::new(), active_shape_layer: 0, generators: Vec::new(), road_removed: Default::default(), road_added: Vec::new(), road_project: None, template_libraries: Vec::new(), routes: Vec::new(), next_modifier };
        w.recompose(None);
        w
    }
    /// Copy for generators on another thread: no satellite and no placed objects.
    pub fn lite(&self) -> World {
        let mut objects = ObjectStore::default();
        for m in &self.objects.models {
            objects.intern(&m.path);
        }
        World {
            meta: self.meta.clone(), base: self.base.clone(), height: self.height.clone(), mask: self.mask.clone(),
            layers: self.layers.clone(), satellite: None, sat: Default::default(), mask_mods: Default::default(), objects, modifiers: Vec::new(), holes: Default::default(), holes_rev: 0, keypoints: Vec::new(), object_layers: self.object_layers.clone(), active_layer: self.active_layer, shape_layers: Vec::new(), active_shape_layer: 0, generators: Vec::new(), road_removed: Default::default(), road_added: Vec::new(), road_project: None, template_libraries: Vec::new(), routes: Vec::new(), next_modifier: self.next_modifier,
        }
    }
    #[inline]
    fn object_layer(&self, o: &Obj) -> Option<&ObjectLayer> {
        self.object_layers.get(o.layer() as usize)
    }
    #[inline]
    pub fn object_visible(&self, o: &Obj) -> bool {
        self.object_layer(o).is_none_or(|l| l.visible)
    }
    /// Tools may pick and change this object.
    #[inline]
    pub fn object_interactive(&self, o: &Obj) -> bool {
        self.object_layer(o).is_none_or(ObjectLayer::interactive)
    }
    #[inline]
    pub fn object_exported(&self, o: &Obj) -> bool {
        self.object_layer(o).is_none_or(|l| l.export)
    }
    /// Layer that new objects go to; falls back to the first one if the active layer is gone.
    pub fn target_layer(&self) -> u16 {
        match self.object_layers.get(self.active_layer as usize) {
            Some(l) if !l.deleted => self.active_layer,
            _ => 0,
        }
    }
    /// Appends a layer with a name not used yet and returns its index.
    pub fn add_object_layer(&mut self, name: &str) -> u16 {
        let taken = |n: &str, ls: &[ObjectLayer]| ls.iter().any(|l| !l.deleted && l.name.eq_ignore_ascii_case(n));
        let mut unique = name.to_string();
        let mut k = 2;
        while taken(&unique, &self.object_layers) {
            unique = format!("{name} {k}");
            k += 1;
        }
        self.object_layers.push(ObjectLayer::new(unique));
        (self.object_layers.len() - 1) as u16
    }
    pub fn new_modifier_id(&mut self) -> u64 {
        let id = self.next_modifier;
        self.next_modifier += 1;
        id
    }
    /// The vertex rectangle a modifier can touch.
    pub fn modifier_rect(&self, m: &TerrainModifier) -> Option<Rect> {
        self.base.rect_around(m.x, m.z, m.reach())
    }
    /// Rebuild composed heights in a region (or everywhere) from base plus modifiers.
    pub fn recompose(&mut self, area: Option<Rect>) {
        let area = area.unwrap_or_else(|| self.base.full_rect());
        let cols = self.base.cols;
        for y in area.y0..=area.y1 {
            let (s, e) = (y * cols + area.x0, y * cols + area.x1 + 1);
            self.height.data[s..e].copy_from_slice(&self.base.data[s..e]);
        }
        let cell = self.base.cell;
        for m in self.modifiers.iter().filter(|m| m.enabled) {
            let Some(r) = self.base.rect_around(m.x, m.z, m.reach()) else { continue };
            let (x0, y0, x1, y1) = (r.x0.max(area.x0), r.y0.max(area.y0), r.x1.min(area.x1), r.y1.min(area.y1));
            if x0 > x1 || y0 > y1 {
                continue;
            }
            if matches!(m.shape, ModShape::Line { .. }) {
                apply_line(&mut self.height, m, Rect { x0, y0, x1, y1 }, false);
                continue;
            }
            if matches!(m.shape, ModShape::Pond { .. }) {
                apply_pond(&mut self.height, m, Rect { x0, y0, x1, y1 });
                continue;
            }
            let (sin, cos) = m.yaw.to_radians().sin_cos();
            let target = m.target.unwrap_or_else(|| self.base.sample(m.x, m.z)) + m.amount;
            for j in y0..=y1 {
                for i in x0..=x1 {
                    let w = m.weight(i as f32 * cell, j as f32 * cell, sin, cos);
                    if w <= 0.0 {
                        continue;
                    }
                    let h = self.height.at(i, j);
                    let goal = match m.mode {
                        ModMode::Flatten => target,
                        ModMode::Offset => h + m.amount,
                        ModMode::Fill => h.max(target),
                        ModMode::Cut => h.min(target),
                    };
                    self.height.set(i, j, h + (goal - h) * w);
                }
            }
        }
        // Banks reach far; the beds themselves must survive whatever came after them.
        for m in self.modifiers.iter().filter(|m| m.enabled && matches!(m.shape, ModShape::Line { .. })) {
            let Some(r) = self.base.rect_around(m.x, m.z, m.reach()) else { continue };
            let (x0, y0, x1, y1) = (r.x0.max(area.x0), r.y0.max(area.y0), r.x1.min(area.x1), r.y1.min(area.y1));
            if x0 <= x1 && y0 <= y1 {
                apply_line(&mut self.height, m, Rect { x0, y0, x1, y1 }, true);
            }
        }
    }
    /// Moves attached modifiers after an object moved; returns the changed area.
    pub fn sync_attached(&mut self, object: u32) -> Option<Rect> {
        let o = *self.objects.get(object)?;
        let mut area: Option<Rect> = None;
        for k in 0..self.modifiers.len() {
            if self.modifiers[k].attached != Some(object) {
                continue;
            }
            let before = self.base.rect_around(self.modifiers[k].x, self.modifiers[k].z, self.modifiers[k].reach());
            {
                let m = &mut self.modifiers[k];
                m.x = o.x;
                m.z = o.z;
                m.yaw = o.yaw;
            }
            let after = self.base.rect_around(o.x, o.z, self.modifiers[k].reach());
            for r in [before, after].into_iter().flatten() {
                area = Some(area.map_or(r, |a| a.union(r)));
            }
        }
        if let Some(a) = area {
            self.recompose(Some(a));
        }
        area
    }
    /// The terrain is not drawn here.
    pub fn is_hole(&self, x: f32, z: f32) -> bool {
        !self.holes.is_empty() && x >= 0.0 && z >= 0.0 && self.holes.contains(&[(x / self.height.cell) as u32, (z / self.height.cell) as u32])
    }
    pub fn layer_index(&self, name: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world() -> World {
        let mut w = World::new(WorldMeta::new("Test", 128, 8.0), 100.0);
        for i in 0..128 {
            for j in 0..128 {
                w.base.set(i, j, i as f32);
            }
        }
        w.recompose(None);
        w
    }
    fn modifier(mode: ModMode, target: Option<f32>, amount: f32) -> TerrainModifier {
        TerrainModifier { id: 1, name: "m".into(), enabled: true, x: 400.0, z: 400.0, yaw: 0.0, shape: ModShape::Circle { radius: 40.0 }, mode, target, amount, falloff: 40.0, attached: None }
    }
    #[test]
    fn a_pond_is_dug_within_its_shape_and_holds_its_water() {
        let mut w = World::new(WorldMeta::new("t", 64, 8.0), 10.0);
        // Ground falling away to the east, past the shore.
        for j in 0..64 { for i in 0..64 { w.base.set(i, j, 20.0 - i as f32 * 0.2); } }
        let area = PondArea { points: vec![[100.0, 100.0], [300.0, 100.0], [300.0, 300.0], [100.0, 300.0]], level: 10.0 };
        w.modifiers.push(TerrainModifier { id: 1, name: "p".into(), enabled: true, x: 200.0, z: 200.0, yaw: 0.0, shape: ModShape::Pond { areas: vec![area], depth: 3.0, blend: 16.0, bank: 24.0 }, mode: ModMode::Flatten, target: None, amount: 0.0, falloff: 16.0, attached: None });
        w.recompose(None);
        // The middle is the floor; the low side just outside is raised over the water.
        assert!((w.height.sample(200.0, 200.0) - 7.0).abs() < 0.01, "{}", w.height.sample(200.0, 200.0));
        assert!(w.height.sample(320.0, 200.0) >= 10.2, "{}", w.height.sample(320.0, 200.0));
        // Far from it nothing changed.
        assert!((w.height.sample(440.0, 200.0) - (20.0 - 55.0 * 0.2)).abs() < 0.01);
    }

    #[test]
    fn a_hole_covers_exactly_one_terrain_cell() {
        let mut w = world();
        assert!(!w.is_hole(84.0, 92.0));
        w.holes.insert([10, 11]);
        assert!(w.is_hole(80.0, 88.0) && w.is_hole(87.9, 95.9));
        assert!(!w.is_hole(79.9, 88.0) && !w.is_hole(88.0, 88.0) && !w.is_hole(80.0, 96.0));
    }
    #[test]
    fn flatten_levels_inside_and_blends_out() {
        let mut w = world();
        w.modifiers.push(modifier(ModMode::Flatten, Some(10.0), 0.0));
        w.recompose(None);
        assert_eq!(w.height.sample(400.0, 400.0), 10.0);
        assert_eq!(w.height.at(0, 50), 0.0);
        // Far outside the falloff nothing changes.
        assert_eq!(w.height.at(100, 50), 100.0);
        // In the falloff the height lies between both.
        let h = w.height.sample(462.0, 400.0);
        assert!(h > 10.0 && h < 462.0 / 8.0);
    }
    #[test]
    fn disabling_a_modifier_restores_base() {
        let mut w = world();
        w.modifiers.push(modifier(ModMode::Offset, None, 5.0));
        w.recompose(None);
        assert_eq!(w.height.at(50, 50), 55.0);
        w.modifiers[0].enabled = false;
        let r = w.modifier_rect(&w.modifiers[0].clone());
        w.recompose(r);
        assert_eq!(w.height.at(50, 50), 50.0);
    }
    #[test]
    fn fill_only_raises() {
        let mut w = world();
        w.modifiers.push(modifier(ModMode::Fill, Some(50.0), 0.0));
        w.recompose(None);
        assert_eq!(w.height.sample(376.0, 400.0), 50.0); // 47 raised
        assert_eq!(w.height.sample(424.0, 400.0), 53.0); // above stays
    }
    #[test]
    fn rotated_rect_covers_its_long_axis() {
        let m = TerrainModifier { yaw: 90.0, shape: ModShape::Rect { half_x: 30.0, half_z: 5.0 }, ..modifier(ModMode::Flatten, None, 0.0) };
        let (s, c) = 90f32.to_radians().sin_cos();
        assert_eq!(m.weight(400.0, 425.0, s, c), 1.0);
        assert_eq!(m.weight(425.0, 400.0, s, c), 0.0 + m.weight(425.0, 400.0, s, c));
        assert!(m.weight(490.0, 400.0, s, c) == 0.0);
    }
}
