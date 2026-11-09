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
            fog: 0.00007 * self.fog,
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
    /// Width over which surfaces meet, in metres.
    pub surface_blend: f32,
    /// How far the textures of the surfaces reach, in metres.
    pub surface_range: f32,
    /// Height of the sea swell in metres.
    pub sea_waves: f32,
    pub lighting: Lighting,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "pl".into(), ui_scale: 1.0, tools: ToolPaths::default(), recent: Vec::new(),
            terrain_quality: 1.0, draw_distance: 2500.0, near_detail: 180.0, autosave_minutes: 10, backup_enabled: true, backup_minutes: 30, backup_keep: 10, backup_folder: String::new(),
            check_updates: true, extra_mods: String::new(), with_client: true, asset_roots: Vec::new(), fov: 75.0, library_tile: 96.0, surface_blend: 3.0, surface_range: 190.0, sea_waves: 0.6, camera_speed: 60.0, lighting: Lighting::default(),
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
