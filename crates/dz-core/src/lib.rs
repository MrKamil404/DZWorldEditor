//! Core of DZ World Editor: no UI code lives here.

/// Starts the job log lines that are warnings.
pub const WARN_MARK: &str = "\u{1}warn:";

pub mod analysis;
pub mod assets;
pub mod build;
pub mod check;
pub mod clutter;
pub mod dressing;
pub mod edit;
pub mod export;
pub mod export_files;
pub mod fsutil;
pub mod gen;
pub mod goto;
pub mod import;
pub mod holes;
pub mod layer_material;
pub mod layers_cfg;
pub mod modules;
pub mod library;
pub mod keypoints;
pub mod landscape;
pub mod mask_limit;
pub mod model_info;
pub mod noise;
pub mod objects;
pub mod paa_write;
pub mod pbo;
pub mod maskmod;
pub mod mission;
pub mod project;
pub mod real_terrain;
pub mod satgen;
pub mod satmod;
pub mod shapes;
pub mod surfaces;
pub mod team;
pub mod terrain;
pub mod terrain_stamps;
pub mod tv4l;
pub mod world;
pub mod wrp;

pub use forest_core;
pub use dayz_road_tool as road_core;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Progress and log of a background task, shared with the UI.
#[derive(Clone, Default)]
pub struct Job {
    inner: Arc<JobInner>,
}
#[derive(Default)]
struct JobInner {
    cancel: AtomicBool,
    progress: AtomicU32,
    log: Mutex<Vec<String>>,
    stage: Mutex<String>,
}
impl Job {
    pub fn set_progress(&self, value: f32) {
        self.inner.progress.store((value.clamp(0.0, 1.0) * 1_000_000.0) as u32, Ordering::Relaxed);
    }
    pub fn progress(&self) -> f32 {
        self.inner.progress.load(Ordering::Relaxed) as f32 / 1_000_000.0
    }
    pub fn stage(&self, text: impl Into<String>) {
        let text = text.into();
        self.log(text.clone());
        *self.inner.stage.lock().unwrap() = text;
    }
    pub fn current_stage(&self) -> String {
        self.inner.stage.lock().unwrap().clone()
    }
    pub fn log(&self, line: impl Into<String>) {
        self.inner.log.lock().unwrap().push(line.into());
    }
    /// A line the log marks as a warning.
    pub fn warn(&self, line: impl Into<String>) {
        self.log(format!("{WARN_MARK}{}", line.into()));
    }
    pub fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.inner.log.lock().unwrap())
    }
    pub fn cancel(&self) {
        self.inner.cancel.store(true, Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.inner.cancel.load(Ordering::Relaxed)
    }
    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.cancelled(), "Cancelled");
        Ok(())
    }
}
