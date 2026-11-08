//! File helpers.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Writes through a temporary file next to the target so a crash never leaves half a file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).with_context(|| format!("{}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("{}", path.display()))?;
    Ok(())
}

pub fn f32s_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

pub fn bytes_to_f32s(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
}

/// Copies a directory tree.
pub fn copy_tree(from: &Path, to: &Path, overwrite: bool) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from).with_context(|| format!("{}", from.display()))? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &target, overwrite)?;
        } else if overwrite || !target.exists() {
            std::fs::copy(e.path(), &target)?;
        }
    }
    Ok(())
}
