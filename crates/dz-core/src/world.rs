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
}

impl WorldMeta {
    pub fn new(name: &str, size: u32, cell: f32) -> Self {
        let map_id: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        let map_id = if map_id.is_empty() { "NewMap".to_string() } else { map_id };
        let side = size as f32 * cell;
        Self {
            name: name.to_string(), map_id, author: String::new(), terrain_size: size, terrain_cell: cell,
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum ModShape {
    /// Radius in metres.
    Circle { radius: f32 },
    /// Half extents in metres, rotated by the modifier's yaw.
    Rect { half_x: f32, half_z: f32 },
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
        self.falloff + match self.shape {
            ModShape::Circle { radius } => radius,
            ModShape::Rect { half_x, half_z } => (half_x * half_x + half_z * half_z).sqrt(),
        }
    }
    /// Blend weight at a world position: 1 inside the shape, down to 0 at the falloff edge.
    #[inline]
    pub fn weight(&self, x: f32, z: f32, sin: f32, cos: f32) -> f32 {
        let (dx, dz) = (x - self.x, z - self.z);
        let d = match self.shape {
            ModShape::Circle { radius } => (dx * dx + dz * dz).sqrt() - radius,
            ModShape::Rect { half_x, half_z } => {
                let u = dx * cos - dz * sin;
                let v = dx * sin + dz * cos;
                let (a, b) = (u.abs() - half_x, v.abs() - half_z);
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

/// A road or other line drawn on the map.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct MapLine {
    pub name: String,
    /// `road family` such as `asf2`, or free text for shapes without models.
    pub family: String,
    pub points: Vec<[f32; 2]>,
    pub width: f32,
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
    pub satellite: Option<RgbImage>,
    pub objects: ObjectStore,
    pub modifiers: Vec<TerrainModifier>,
    pub lines: Vec<MapLine>,
    /// Never empty; layer 0 cannot be deleted.
    pub object_layers: Vec<ObjectLayer>,
    /// Layer that receives new objects.
    pub active_layer: u16,
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
            height: base.clone(),
            base,
            mask: MaskMap::new(mask_px, mask_px, 0),
            layers: default_layers(),
            satellite: None,
            objects: ObjectStore::default(),
            modifiers: Vec::new(),
            lines: Vec::new(),
            object_layers: default_object_layers(),
            active_layer: 0,
            meta,
            next_modifier: 1,
        }
    }
    pub fn from_parts(meta: WorldMeta, base: Heightmap, mask: MaskMap, layers: Vec<SurfaceLayer>, satellite: Option<RgbImage>, objects: ObjectStore, modifiers: Vec<TerrainModifier>, lines: Vec<MapLine>) -> Self {
        let next_modifier = modifiers.iter().map(|m| m.id).max().unwrap_or(0) + 1;
        let mut w = Self { height: base.clone(), base, meta, mask, layers, satellite, objects, modifiers, lines, object_layers: default_object_layers(), active_layer: 0, next_modifier };
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
            layers: self.layers.clone(), satellite: None, objects, modifiers: Vec::new(), lines: Vec::new(), object_layers: self.object_layers.clone(), active_layer: self.active_layer, next_modifier: self.next_modifier,
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
