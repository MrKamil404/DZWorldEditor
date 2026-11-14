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
    /// What the world config (CfgWorlds) of the map says beyond what the project gives it.
    #[serde(default)]
    pub config: MapConfig,
}

/// The world class of the game a map builds on: its class, and the addon that brings it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum BaseWorld {
    Enoch,
    ChernarusPlus,
    /// Another world class, from the given addon.
    Custom { class: String, addon: String },
}

impl BaseWorld {
    pub fn class(&self) -> &str {
        match self {
            BaseWorld::Enoch => "Enoch",
            BaseWorld::ChernarusPlus => "ChernarusPlus",
            BaseWorld::Custom { class, .. } => class,
        }
    }
    pub fn addon(&self) -> &str {
        match self {
            BaseWorld::Enoch => "DZ_Worlds_Enoch",
            BaseWorld::ChernarusPlus => "DZ_Worlds_Chernarusplus",
            BaseWorld::Custom { addon, .. } => addon,
        }
    }
}

/// Settings of the world config (CfgWorlds) of the map.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MapConfig {
    /// Shown in the game; empty takes the name of the project.
    pub description: String,
    pub base: BaseWorld,
    pub has_ocean: bool,
    /// The middle of the map for the game (x, z); `None` takes the middle of the terrain.
    pub center: Option<[f32; 2]>,
    /// Grid of the in-game map: steps of its three zoom levels in metres.
    pub grid_steps: [u32; 3],
    /// Addons loaded before the map, besides DZ_Data and the base world.
    pub required_addons: Vec<String>,
    /// Lines written into the world class as they are.
    pub extra_world: String,
    /// Lines written at the end of the file as they are.
    pub extra_file: String,
    /// Places the main menu shows behind the character (cfgCharacterScenes).
    pub scenes: Vec<CharacterScene>,
    /// Intro missions (`cutscenes[]`); empty keeps those of the base world.
    pub cutscenes: Vec<String>,
    /// `startTime`, as "9:00"; empty keeps the base world's.
    pub start_time: String,
    /// `startDate`, as "25/8/2015"; empty keeps the base world's.
    pub start_date: String,
    /// Folder of the Central Economy files (`ceFiles`).
    pub ce_files: String,
    /// The map item: keys of its name and description, its textures and the user map.
    pub map_display_name_key: String,
    pub map_description_key: String,
    pub map_texture_closed: String,
    pub map_texture_opened: String,
    pub map_texture_legend: String,
    pub user_map_path: String,
    /// The terrain drawn beyond the edge of the map.
    pub outside_satellite: String,
    pub outside_synth: Option<bool>,
    pub outside_texture: String,
    pub outside_nopx: String,
    pub mid_detail_texture: String,
    pub terrain_normal_texture: String,
    /// The material of the sea, for calm, nice and stormy weather alike.
    pub ocean_material: String,
    /// Grass and detail distances; `None` keeps the base world's.
    pub clutter_grid: Option<f32>,
    pub clutter_dist: Option<f32>,
    pub no_detail_dist: Option<f32>,
    pub full_detail_dist: Option<f32>,
    pub min_trees_in_forest_square: Option<u32>,
    pub min_rocks_in_rock_square: Option<u32>,
}

/// A place of the main menu: the camera stands at `position` (x, z, height over the ground)
/// and looks at `target` (x, z, height above the sea), at the time and weather given.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CharacterScene {
    pub name: String,
    pub position: [f32; 3],
    pub target: [f32; 3],
    /// Field of view in radians.
    pub fov: f32,
    /// Year, month, day, hour, minute.
    pub date: [i32; 5],
    pub overcast: f32,
    pub rain: f32,
    pub fog: f32,
}

impl Default for CharacterScene {
    fn default() -> Self {
        Self { name: "loc1".into(), position: [0.0, 0.0, 1.0], target: [0.0, 0.0, 0.0], fov: 0.5236, date: [1984, 10, 28, 9, 0], overcast: 0.2, rain: 0.0, fog: 0.0 }
    }
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            description: String::new(), base: BaseWorld::Enoch, has_ocean: true, center: None, grid_steps: [100, 1000, 10000], required_addons: Vec::new(), extra_world: String::new(), extra_file: String::new(),
            scenes: Vec::new(), cutscenes: Vec::new(), start_time: String::new(), start_date: String::new(), ce_files: String::new(),
            map_display_name_key: String::new(), map_description_key: String::new(), map_texture_closed: String::new(), map_texture_opened: String::new(), map_texture_legend: String::new(), user_map_path: String::new(),
            outside_satellite: String::new(), outside_synth: None, outside_texture: String::new(), outside_nopx: String::new(),
            mid_detail_texture: String::new(), terrain_normal_texture: String::new(), ocean_material: String::new(),
            clutter_grid: None, clutter_dist: None, no_detail_dist: None, full_detail_dist: None, min_trees_in_forest_square: None, min_rocks_in_rock_square: None,
        }
    }
}

impl WorldMeta {
    pub fn new(name: &str, size: u32, cell: f32) -> Self {
        let map_id: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        let map_id = if map_id.is_empty() { "NewMap".to_string() } else { map_id };
        let side = size as f32 * cell;
        Self {
            max_tile_surfaces: 6, name: name.to_string(), map_id, author: String::new(), terrain_size: size, terrain_cell: cell,
            sea_level: 0.0, easting: 200000.0, northing: 0.0, tile_px: 512, cells_per_tile: 8,
            spawn: [side / 2.0, side / 2.0], latitude: 54.0, longitude: 23.0, write_underground_flag: true, config: MapConfig::default(),
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
    /// Levels the ground to the height the object stands at, which moves with it.
    #[serde(default)]
    pub follow: bool,
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
    /// The road pieces of the last fit, in order, so the route can go on from either end
    /// without fitting the rest again.
    #[serde(default)]
    pub parts: Vec<dayz_road_tool::roads::PlacedPart>,
    /// The waypoints of the last fit.
    #[serde(default)]
    pub fitted: Vec<[f32; 2]>,
    /// How the route was laid, and so how it is edited.
    #[serde(default)]
    pub method: RouteMethod,
}

/// How a road is laid: models fitted to clicked points when they are all there, fitted while
/// the mouse moves, or chosen piece by piece.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum RouteMethod {
    #[default]
    Points,
    Live,
    Parts,
}

/// A move over the map as the gizmo makes it: a turn by `yaw` degrees about `pivot`, the way
/// objects turn, then a shift.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub pivot: [f32; 2],
    pub yaw: f32,
    pub shift: [f32; 2],
}

impl Motion {
    /// The motion that took an object from `before` to `after` about `pivot`.
    pub fn between(before: &crate::objects::Obj, after: &crate::objects::Obj, pivot: [f32; 2]) -> Self {
        let yaw = after.yaw - before.yaw;
        let m = Motion { pivot, yaw, shift: [0.0, 0.0] };
        let turned = m.apply([before.x, before.z]);
        Motion { shift: [after.x - turned[0], after.z - turned[1]], ..m }
    }
    pub fn is_still(&self) -> bool {
        self.yaw.abs() < 1e-4 && self.shift[0].abs() < 1e-4 && self.shift[1].abs() < 1e-4
    }
    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        let (sin, cos) = self.yaw.to_radians().sin_cos();
        let (rx, rz) = (p[0] - self.pivot[0], p[1] - self.pivot[1]);
        [self.pivot[0] + rx * cos + rz * sin + self.shift[0], self.pivot[1] - rx * sin + rz * cos + self.shift[1]]
    }
    fn apply64(&self, p: [f64; 2]) -> [f64; 2] {
        let q = self.apply([p[0] as f32, p[1] as f32]);
        [q[0] as f64, q[1] as f64]
    }
}

impl RoadRoute {
    /// Moves the line of the route with its parts, which are objects moved on their own.
    pub fn moved(&mut self, m: &Motion) {
        for p in self.points.iter_mut().chain(self.centerline.iter_mut()).chain(self.fitted.iter_mut()) {
            *p = m.apply(*p);
        }
        for p in &mut self.profile {
            let q = m.apply([p[0], p[1]]);
            p[0] = q[0];
            p[1] = q[1];
        }
        for d in &mut self.decks {
            d.from = m.apply(d.from);
            d.to = m.apply(d.to);
        }
        for part in &mut self.parts {
            let q = m.apply64([part.position[0], part.position[1]]);
            part.position[0] = q[0];
            part.position[1] = q[1];
            part.rotation += m.yaw as f64;
        }
    }
}

impl TerrainModifier {
    /// Moves the modifier and the points of its shape.
    pub fn moved(&mut self, m: &Motion) {
        let c = m.apply([self.x, self.z]);
        (self.x, self.z) = (c[0], c[1]);
        self.yaw += m.yaw;
        match &mut self.shape {
            ModShape::Line { points, .. } => points.iter_mut().for_each(|p| *p = m.apply(*p)),
            ModShape::Pond { areas, .. } => areas.iter_mut().flat_map(|a| a.points.iter_mut()).for_each(|p| *p = m.apply(*p)),
            ModShape::Circle { .. } | ModShape::Rect { .. } => {}
        }
    }
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
    /// Who works on this layer in team work; empty when anyone does.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
}

fn yes() -> bool {
    true
}

impl ObjectLayer {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), visible: true, editable: true, export: true, deleted: false, owner: String::new() }
    }
    /// Tools may pick and change the objects of this layer.
    pub fn interactive(&self) -> bool {
        self.visible && self.editable && !self.deleted
    }
}

/// Keys of the authors of mask and of satellite modifiers, whose ids are numbered apart from
/// the other ids.
pub const MASK_MOD_AUTHOR: u64 = 1 << 62;
pub const SAT_MOD_AUTHOR: u64 = 1 << 61;

/// Layers the objects of a built world are put into, by what they are.
pub const OBJECT_GROUPS: [&str; 7] = ["Trees and bushes", "Buildings", "Walls and fences", "Roads and tracks", "Rocks", "Wrecks", "Water"];

/// Which of [`OBJECT_GROUPS`] the model at `path` belongs to.
pub fn object_group(path: &str) -> Option<usize> {
    let p = path.to_ascii_lowercase().replace('/', "\\");
    let has = |s: &str| p.contains(s);
    Some(if has(r"\tree\") || has(r"\t_") || has("plants") || has(r"\bush") || has(r"\b_") {
        0
    } else if has(r"\wrecks\") {
        5
    } else if has(r"\water") {
        6
    } else if has(r"\roads\") || has(r"\rail\") {
        3
    } else if has(r"\walls\") || has("fence") {
        2
    } else if has(r"\rocks\") || has(r"\rock") || has(r"\stone") {
        4
    } else if has(r"\structures") && !has(r"\furniture\") && !has(r"\signs\") {
        1
    } else {
        return None;
    })
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
    /// The server mission of the map, when the project has one.
    pub mission: Option<Box<crate::mission::Mission>>,
    /// Terrain cells the game does not draw.
    pub holes: crate::holes::Holes,
    /// Goes up whenever `holes` changes, so the view can follow.
    pub holes_rev: u32,
    /// Who made each terrain modifier, road and generator, by id, in team work. `None` until
    /// team work starts; what was there then has an empty author.
    pub authors: Option<std::collections::BTreeMap<u64, String>>,
    /// The name of the person working on this copy in team work; empty without team work.
    /// Not saved.
    pub team_me: String,
    /// Layers of others this person chose to edit anyway, by name. Not saved.
    pub team_unlocked: std::collections::BTreeSet<String>,
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
            holes_rev: 0, keypoints: Vec::new(), mission: None,
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
            authors: None, team_me: String::new(), team_unlocked: Default::default(),
            next_modifier: 1,
        }
    }
    pub fn from_parts(meta: WorldMeta, base: Heightmap, mask: MaskMap, layers: Vec<SurfaceLayer>, satellite: Option<RgbImage>, objects: ObjectStore, modifiers: Vec<TerrainModifier>) -> Self {
        let next_modifier = modifiers.iter().map(|m| m.id).max().unwrap_or(0) + 1;
        let mut w = Self { height: base.clone(), base, meta, mask, layers, satellite, sat: Default::default(), mask_mods: Default::default(), objects, modifiers, holes: Default::default(), holes_rev: 0, keypoints: Vec::new(), mission: None, object_layers: default_object_layers(), active_layer: 0, shape_layers: Vec::new(), active_shape_layer: 0, generators: Vec::new(), road_removed: Default::default(), road_added: Vec::new(), road_project: None, template_libraries: Vec::new(), routes: Vec::new(), authors: None, team_me: String::new(), team_unlocked: Default::default(), next_modifier };
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
            layers: self.layers.clone(), satellite: None, sat: Default::default(), mask_mods: Default::default(), objects, modifiers: Vec::new(), holes: Default::default(), holes_rev: 0, keypoints: Vec::new(), mission: None, object_layers: self.object_layers.clone(), active_layer: self.active_layer, shape_layers: Vec::new(), active_shape_layer: 0, generators: Vec::new(), road_removed: Default::default(), road_added: Vec::new(), road_project: None, template_libraries: Vec::new(), routes: Vec::new(), authors: None, team_me: self.team_me.clone(), team_unlocked: self.team_unlocked.clone(), next_modifier: self.next_modifier,
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
        self.object_layer(o).is_none_or(|l| l.interactive() && self.may_edit(l))
    }
    /// In team work, a layer of someone else is left alone unless this person unlocked it.
    pub fn may_edit(&self, l: &ObjectLayer) -> bool {
        l.owner.is_empty() || self.team_me.is_empty() || l.owner == self.team_me || self.team_unlocked.contains(&l.name)
    }
    /// Objects of the layer may be picked, changed and added.
    pub fn layer_interactive(&self, layer: u16) -> bool {
        self.object_layers.get(layer as usize).is_some_and(|l| l.interactive() && self.may_edit(l))
    }
    /// The layer belongs to someone else in team work and is not unlocked.
    pub fn layer_foreign(&self, layer: u16) -> bool {
        self.object_layers.get(layer as usize).is_some_and(|l| !self.may_edit(l))
    }
    /// Who made a terrain modifier, road or generator; empty when no one is known.
    pub fn author(&self, id: u64) -> &str {
        self.authors.as_ref().and_then(|a| a.get(&id)).map(String::as_str).unwrap_or("")
    }
    /// Makes `name` the author of the thing under `key` (an id, or a mask or satellite modifier
    /// with its kind); empty for no one. Gives the authors as they were, for undo.
    pub fn set_author(&mut self, key: u64, name: &str) -> Option<std::collections::BTreeMap<u64, String>> {
        let before = self.authors.clone();
        self.authors.get_or_insert_with(Default::default).insert(key, name.to_string());
        before
    }
    /// Who made a mask modifier; empty when no one is known.
    pub fn mask_mod_author(&self, id: u32) -> &str {
        self.author(MASK_MOD_AUTHOR | id as u64)
    }
    /// Who made a satellite modifier; empty when no one is known.
    pub fn sat_mod_author(&self, id: u32) -> &str {
        self.author(SAT_MOD_AUTHOR | id as u64)
    }
    /// In team work: the modifiers, roads and generators made since get `team_me` as their
    /// author. The first time, what is there is marked as made by no one.
    pub fn note_authors(&mut self) {
        if self.team_me.is_empty() {
            return;
        }
        let groups = [
            (0, self.modifiers.iter().map(|m| m.id).chain(self.routes.iter().map(|r| r.id)).chain(self.generators.iter().map(|g| g.id)).collect::<Vec<u64>>()),
            (MASK_MOD_AUTHOR, self.mask_mods.modifiers.iter().map(|m| MASK_MOD_AUTHOR | m.id as u64).collect()),
            (SAT_MOD_AUTHOR, self.sat.modifiers.iter().map(|m| SAT_MOD_AUTHOR | m.id as u64).collect()),
        ];
        let me = self.team_me.clone();
        let fresh = self.authors.is_none();
        let a = self.authors.get_or_insert_with(Default::default);
        for (kind, ids) in groups {
            // The first time a kind is noted, what is there was made by no one. A mark under
            // the bare key of the kind says it was.
            let first = if kind == 0 { fresh } else { !a.contains_key(&kind) };
            if kind != 0 {
                a.entry(kind).or_default();
            }
            for id in ids {
                if first {
                    a.entry(id).or_default();
                } else {
                    a.entry(id).or_insert_with(|| me.clone());
                }
            }
        }
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
    /// Puts the objects of the first layer into layers by what they are: trees and bushes,
    /// buildings, walls, roads, rocks, wrecks, water. What fits none stays where it was; groups with
    /// no objects get no layer.
    pub fn group_objects(&mut self) {
        let kinds: Vec<Option<usize>> = self.objects.models.iter().map(|m| object_group(&m.path)).collect();
        let mut layers: [Option<u16>; OBJECT_GROUPS.len()] = [None; OBJECT_GROUPS.len()];
        let moves: Vec<(u32, usize)> = self.objects.iter().filter(|(_, o)| o.layer() == 0).filter_map(|(id, o)| Some((id, kinds.get(o.model as usize).copied().flatten()?))).collect();
        for (id, group) in moves {
            let layer = *layers[group].get_or_insert_with(|| self.add_object_layer(OBJECT_GROUPS[group]));
            if let Some(mut o) = self.objects.get(id).copied() {
                o.set_layer(layer);
                self.objects.update(id, o);
            }
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
        let mut layer = ObjectLayer::new(unique);
        // In team work a new layer belongs to the one who made it.
        layer.owner = self.team_me.clone();
        self.object_layers.push(layer);
        (self.object_layers.len() - 1) as u16
    }
    /// An id no other modifier has, also in copies of the project other people work on.
    pub fn new_modifier_id(&mut self) -> u64 {
        loop {
            let id = crate::library::new_id().max(self.next_modifier);
            if !self.modifiers.iter().any(|m| m.id == id) {
                self.next_modifier = id + 1;
                return id;
            }
        }
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
                // The object stands at a height of its own, so the ground comes up or down to it.
                if m.follow {
                    m.target = Some(crate::objects::ObjectStore::origin_y(&o, &self.base));
                }
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
    #[test]
    fn objects_of_a_built_world_go_to_layers_by_kind() {
        assert_eq!(object_group(r"dz\plants\tree\t_piceaabies_3f.p3d"), Some(0));
        assert_eq!(object_group(r"DZ\plants_bliss\bush\b_corylus_1s.p3d"), Some(0));
        assert_eq!(object_group(r"dz\structures\residential\police\city_policestation.p3d"), Some(1));
        assert_eq!(object_group(r"dz\structures\walls\wall_tin_4.p3d"), Some(2));
        assert_eq!(object_group(r"dz\structures\roads\parts\asf2_25.p3d"), Some(3));
        assert_eq!(object_group(r"dz\rocks\stone1.p3d"), Some(4));
        assert_eq!(object_group(r"dz\structures\wrecks\cars\wreck_volha.p3d"), Some(5));
        assert_eq!(object_group(r"dz\structures\furniture\chairs\chair.p3d"), None);
        let mut w = world();
        let (tree, house, chair) = (w.objects.intern(r"dz\plants\tree\t_betula.p3d"), w.objects.intern(r"dz\structures\residential\house.p3d"), w.objects.intern(r"dz\structures\furniture\chair.p3d"));
        for m in [tree, house, chair, tree] {
            w.objects.insert(crate::objects::Obj { x: 10.0, z: 10.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: m, flags: 0 });
        }
        w.group_objects();
        let names: Vec<&str> = w.object_layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(&names[1..], &["Trees and bushes", "Buildings"]);
        let layer_of = |m: u32| w.objects.iter().filter(|(_, o)| o.model == m).map(|(_, o)| o.layer()).collect::<Vec<_>>();
        assert_eq!((layer_of(tree), layer_of(house), layer_of(chair)), (vec![1, 1], vec![2], vec![0]));
    }
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
    #[test]
    fn a_pad_that_follows_its_object_levels_the_ground_to_the_object() {
        let mut w = World::new(WorldMeta::new("t", 128, 8.0), 10.0);
        let o = crate::objects::Obj { x: 500.0, z: 500.0, elevation: 14.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: crate::objects::FLAG_ABSOLUTE };
        let id = w.objects.insert(o);
        let mut m = crate::gen::pad_modifier(&mut w, id, 5.0, 5.0, 1.0, 8.0).unwrap();
        m.follow = true;
        w.modifiers.push(m);
        w.sync_attached(id);
        assert!((w.height.sample(500.0, 500.0) - 14.0).abs() < 0.01, "{}", w.height.sample(500.0, 500.0));
        // Lifting the object lifts the ground under it.
        let mut up = o;
        up.elevation = 18.0;
        w.objects.update(id, up);
        w.sync_attached(id);
        assert!((w.height.sample(500.0, 500.0) - 18.0).abs() < 0.01);
        // Far away the terrain is as it was.
        assert!((w.height.sample(100.0, 100.0) - 10.0).abs() < 0.01);
    }

    fn modifier(mode: ModMode, target: Option<f32>, amount: f32) -> TerrainModifier {
        TerrainModifier { id: 1, name: "m".into(), enabled: true, x: 400.0, z: 400.0, yaw: 0.0, shape: ModShape::Circle { radius: 40.0 }, mode, target, amount, falloff: 40.0, attached: None, follow: false }
    }
    #[test]
    fn a_pond_is_dug_within_its_shape_and_holds_its_water() {
        let mut w = World::new(WorldMeta::new("t", 64, 8.0), 10.0);
        // Ground falling away to the east, past the shore.
        for j in 0..64 { for i in 0..64 { w.base.set(i, j, 20.0 - i as f32 * 0.2); } }
        let area = PondArea { points: vec![[100.0, 100.0], [300.0, 100.0], [300.0, 300.0], [100.0, 300.0]], level: 10.0 };
        w.modifiers.push(TerrainModifier { id: 1, name: "p".into(), enabled: true, x: 200.0, z: 200.0, yaw: 0.0, shape: ModShape::Pond { areas: vec![area], depth: 3.0, blend: 16.0, bank: 24.0 }, mode: ModMode::Flatten, target: None, amount: 0.0, falloff: 16.0, attached: None, follow: false });
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

