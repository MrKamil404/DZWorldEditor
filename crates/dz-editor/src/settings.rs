//! Per-user settings stored in `%LOCALAPPDATA%\DZWorldEditor`.
use dz_core::build::ToolPaths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Light of the 3D view; has no effect on the exported map.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lighting {
    /// Degrees clockwise from north.
    pub azimuth: f32,
    pub elevation: f32,
    pub strength: f32,
    pub color: [f32; 3],
    pub ambient: f32,
    pub exposure: f32,
    pub fog: f32,
    /// Metres from the camera that receive and cast shadows.
    pub shadow_range: f32,
}

impl Default for Lighting {
    fn default() -> Self {
        Self { azimuth: 312.0, elevation: 53.0, strength: 0.82, color: [1.0, 0.96, 0.88], ambient: 1.0, exposure: 1.0, fog: 1.0, shadow_range: 300.0 }
    }
}

impl Lighting {
    pub const MORNING: Lighting = Lighting { azimuth: 95.0, elevation: 16.0, strength: 0.75, color: [1.0, 0.82, 0.62], ambient: 0.8, exposure: 1.0, fog: 1.5, shadow_range: 300.0 };
    pub const NOON: Lighting = Lighting { azimuth: 180.0, elevation: 62.0, strength: 0.9, color: [1.0, 0.97, 0.92], ambient: 1.0, exposure: 1.0, fog: 1.0, shadow_range: 300.0 };
    pub const EVENING: Lighting = Lighting { azimuth: 268.0, elevation: 8.0, strength: 0.7, color: [1.0, 0.66, 0.42], ambient: 0.65, exposure: 1.0, fog: 1.6, shadow_range: 300.0 };
    pub const OVERCAST: Lighting = Lighting { azimuth: 200.0, elevation: 55.0, strength: 0.18, color: [0.95, 0.97, 1.0], ambient: 1.35, exposure: 1.0, fog: 2.2, shadow_range: 300.0 };

    pub fn sun_direction(&self) -> [f32; 3] {
        let (sa, ca) = self.azimuth.to_radians().sin_cos();
        let (se, ce) = self.elevation.to_radians().sin_cos();
        [sa * ce, se, ca * ce]
    }

    pub fn gpu(&self) -> crate::render::LightGpu {
        let k = self.strength;
        let a = self.ambient;
        // Darkens with the sun below the horizon, so night settings read correctly.
        let day = ((self.elevation + 6.0) / 12.0).clamp(0.0, 1.0);
        crate::render::LightGpu {
            sun: self.sun_direction(),
            sun_color: [self.color[0] * k * day, self.color[1] * k * day, self.color[2] * k * day],
            ambient: [0.56 * a * (0.25 + 0.75 * day), 0.60 * a * (0.25 + 0.75 * day), 0.68 * a * (0.3 + 0.7 * day)],
            fog: 0.00016 * self.fog,
            exposure: self.exposure,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: String,
    pub ui_scale: f32,
    pub tools: ToolPaths,
    pub recent: Vec<PathBuf>,
    pub terrain_quality: f32,
    pub draw_distance: f32,
    pub near_detail: f32,
    pub autosave_minutes: u32,
    pub backup_enabled: bool,
    pub backup_minutes: u32,
    pub backup_keep: u32,
    /// Where backups go; empty keeps them in the project folder.
    pub backup_folder: String,
    pub check_updates: bool,
    pub extra_mods: String,
    pub with_client: bool,
    pub asset_roots: Vec<String>,
    pub camera_speed: f32,
    pub fov: f32,
    /// Size of the entries in the libraries, in pixels.
    pub library_tile: f32,
    /// How far the textures of the surfaces reach, in metres.
    pub surface_range: f32,
    /// Depth of the relief of the surfaces for parallax, in centimetres; 0 is off.
    pub surface_parallax: f32,
    /// How sharply neighbouring surfaces meet along their relief, 0 soft to 1 hard.
    pub surface_blend: f32,
    /// Height of the sea swell in metres.
    pub sea_waves: f32,
    /// How far the tide lifts (or with less than 0, lowers) the sea, up to the game's MaxTide.
    pub sea_tide: f32,
    /// The tide rises and falls on its own, to see the whole range.
    pub tide_cycle: bool,
    /// Objects move over the ground when dragged by their own shape, not only by the gizmo.
    pub drag_by_body: bool,
    /// A drag over empty ground draws a free outline to select in, not a box.
    pub lasso: bool,
    /// The step lines and turns snap to while the snap key is held, in degrees.
    pub angle_step: f32,
    /// Moved and turned objects take the tilt of the ground under them.
    pub align_to_ground: bool,
    /// Share of the split view the 3D view takes, 0.15 to 0.85.
    pub split_ratio: f32,
    /// The split view stacks the views instead of putting them side by side.
    pub split_stacked: bool,
    /// The top view comes first in the split view.
    pub split_swapped: bool,
    /// The top view keeps the 3D camera in its middle.
    pub top_follow: bool,
    /// The top view draws the models from above when close enough, as a drone sees them.
    pub top_models: bool,
    /// A small map of the whole terrain in a corner of the 3D view.
    pub minimap: bool,
    /// Side of the minimap in points.
    pub minimap_size: f32,
    pub lighting: Lighting,
    /// Largest side of object textures, in pixels.
    pub texture_size: u32,
    /// Anisotropic filtering of textures, 1 is off.
    pub anisotropy: u32,
    /// Side of the shadow map, in pixels.
    pub shadow_map: u32,
    /// Edge smoothing: 0 off, 1 FXAA, 2 with the view drawn at 1.5 times its size, 3 at twice.
    pub edge_smoothing: u32,
    /// Samples of ambient occlusion per pixel; 0 turns it off.
    pub ambient_occlusion: u32,
    /// How far ground clutter takes the colour of the satellite image under it, 0 to 1.
    pub grass_ground_color: f32,
    /// The keyboard shortcuts the user chose.
    pub keys: crate::keys::KeyMap,
    /// The choices of the export window.
    pub export: crate::exporting::ExportSettings,
}

/// A ready set of the graphics settings of the 3D view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GraphicsPreset {
    Low,
    Medium,
    High,
    VeryHigh,
}
impl GraphicsPreset {
    pub const ALL: [GraphicsPreset; 4] = [GraphicsPreset::Low, GraphicsPreset::Medium, GraphicsPreset::High, GraphicsPreset::VeryHigh];
    pub fn name(self) -> &'static str {
        match self {
            GraphicsPreset::Low => "Low",
            GraphicsPreset::Medium => "Medium",
            GraphicsPreset::High => "High",
            GraphicsPreset::VeryHigh => "Very high",
        }
    }
    /// Terrain detail, draw distance, full detail distance, surface range, texture size,
    /// anisotropy, shadow map and shadow range.
    fn values(self) -> (f32, f32, f32, f32, u32, u32, u32, f32) {
        match self {
            GraphicsPreset::Low => (0.8, 1200.0, 40.0, 120.0, 512, 2, 2048, 150.0),
            GraphicsPreset::Medium => (1.0, 2000.0, 80.0, 190.0, 1024, 4, 4096, 250.0),
            GraphicsPreset::High => (1.5, 3000.0, 150.0, 300.0, 2048, 8, 4096, 350.0),
            GraphicsPreset::VeryHigh => (2.0, 5000.0, 300.0, 500.0, 4096, 16, 8192, 500.0),
        }
    }
}

impl Settings {
    pub fn apply_graphics(&mut self, preset: GraphicsPreset) {
        let v = preset.values();
        (self.terrain_quality, self.draw_distance, self.near_detail, self.surface_range) = (v.0, v.1, v.2, v.3);
        (self.texture_size, self.anisotropy, self.shadow_map, self.lighting.shadow_range) = (v.4, v.5, v.6, v.7);
    }
    /// The preset the current values match, if any.
    pub fn graphics_preset(&self) -> Option<GraphicsPreset> {
        GraphicsPreset::ALL.into_iter().find(|p| {
            let v = p.values();
            (self.terrain_quality, self.draw_distance, self.near_detail, self.surface_range) == (v.0, v.1, v.2, v.3) && (self.texture_size, self.anisotropy, self.shadow_map, self.lighting.shadow_range) == (v.4, v.5, v.6, v.7)
        })
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "pl".into(), ui_scale: 1.0, tools: ToolPaths::default(), recent: Vec::new(),
            terrain_quality: 1.0, draw_distance: 2500.0, near_detail: 180.0, autosave_minutes: 10, backup_enabled: true, backup_minutes: 30, backup_keep: 10, backup_folder: String::new(),
            check_updates: true, extra_mods: String::new(), with_client: true, asset_roots: Vec::new(), fov: 75.0, library_tile: 96.0, surface_range: 190.0, surface_parallax: 4.0, surface_blend: 0.45, sea_waves: 0.25, sea_tide: 0.0, tide_cycle: false, drag_by_body: false, lasso: false, angle_step: 15.0, align_to_ground: false, split_ratio: 0.5, split_stacked: false, split_swapped: false, top_follow: false, top_models: true, minimap: true, minimap_size: 200.0, camera_speed: 60.0, lighting: Lighting::default(),
            texture_size: 2048, anisotropy: 8, shadow_map: 4096, edge_smoothing: 2, ambient_occlusion: 12, grass_ground_color: 0.6, keys: Default::default(), export: Default::default(),
        }
    }
}

/// The local time as a folder name, `2026-11-09_18-30-05`.
pub fn timestamp() -> String {
    #[repr(C)]
    #[derive(Default)]
    struct Time { year: u16, month: u16, weekday: u16, day: u16, hour: u16, minute: u16, second: u16, ms: u16 }
    extern "system" {
        fn GetLocalTime(time: *mut Time);
    }
    let mut t = Time::default();
    // SAFETY: the structure has the layout of SYSTEMTIME and the call only fills it in.
    unsafe { GetLocalTime(&mut t) };
    format!("{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("DZWorldEditor")
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Settings = std::fs::read(data_dir().join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if s.tools.dayz_tools.as_os_str().is_empty() || s.tools.dayz.as_os_str().is_empty() {
            s.tools = dz_core::build::detect();
        }
        s
    }
    pub fn save(&self) {
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let _ = dz_core::fsutil::atomic_write(&data_dir().join("settings.json"), &b);
        }
    }
    pub fn remember(&mut self, dir: PathBuf) {
        self.recent.retain(|p| *p != dir);
        self.recent.insert(0, dir);
        self.recent.truncate(12);
    }
}
