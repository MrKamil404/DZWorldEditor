//! Per-user settings stored in `%LOCALAPPDATA%\DZWorldEditor`.
use dz_core::build::ToolPaths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    pub check_updates: bool,
    pub extra_mods: String,
    pub with_client: bool,
    pub asset_roots: Vec<String>,
    pub camera_speed: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "pl".into(), ui_scale: 1.0, tools: ToolPaths::default(), recent: Vec::new(),
            terrain_quality: 1.0, draw_distance: 2500.0, near_detail: 180.0, autosave_minutes: 10,
            check_updates: true, extra_mods: String::new(), with_client: true, asset_roots: Vec::new(), camera_speed: 60.0,
        }
    }
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
