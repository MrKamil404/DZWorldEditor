//! The work drive the game's models are read from. DayZ Tools mounts it as `P:`, but it may be
//! any letter; the editor tells this module which one the user chose, so road and track pieces
//! are found wherever the drive is.
use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;

static DRIVE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Sets the work drive (for example `D:\`).
pub fn set(drive: &Path) {
    if DRIVE.read().unwrap().as_deref() != Some(drive) {
        *DRIVE.write().unwrap() = Some(drive.to_path_buf());
    }
}

/// The work drive; `P:\` until one is set.
pub fn work_drive() -> PathBuf {
    DRIVE.read().unwrap().clone().unwrap_or_else(|| PathBuf::from("P:\\"))
}

/// The folder of the road pieces of the game.
pub fn roads_folder() -> PathBuf {
    work_drive().join(r"dz\structures\roads\parts")
}

/// The folder of the track pieces of the game.
pub fn rails_folder() -> PathBuf {
    work_drive().join(r"dz\structures\rail\Tracks")
}

/// The path without its drive: `P:\dz\a\b.p3d` and `dz\a\b.p3d` both give `dz\a\b.p3d`.
pub fn without_drive(path: &Path) -> PathBuf {
    path.components().filter(|c| !matches!(c, Component::Prefix(_) | Component::RootDir)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_loses_its_drive_letter() {
        assert_eq!(without_drive(Path::new(r"P:\dz\structures\a.p3d")), PathBuf::from(r"dz\structures\a.p3d"));
        assert_eq!(without_drive(Path::new(r"dz\structures\a.p3d")), PathBuf::from(r"dz\structures\a.p3d"));
        assert_eq!(without_drive(Path::new("d:/dz/a.p3d")), PathBuf::from(r"dz\a.p3d"));
    }
}
