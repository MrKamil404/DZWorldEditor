//! Plain PBO packer, used when a folder only needs packing and no binarisation.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in std::fs::read_dir(dir).with_context(|| format!("{}", dir.display()))? {
        let e = e?;
        if e.file_type()?.is_dir() {
            collect(&e.path(), out)?;
        } else {
            out.push(e.path());
        }
    }
    Ok(())
}

/// Packs `folder` into `target` with the given `prefix` header.
pub fn pack(folder: &Path, target: &Path, prefix: &str) -> Result<usize> {
    let mut files = Vec::new();
    collect(folder, &mut files)?;
    let mut entries: Vec<(String, PathBuf)> = files
        .into_iter()
        .filter_map(|p| {
            let rel = p.strip_prefix(folder).ok()?.to_string_lossy().replace('/', "\\");
            (!rel.eq_ignore_ascii_case("$PBOPREFIX$")).then_some((rel, p))
        })
        .collect();
    entries.sort_by_key(|(r, _)| r.to_ascii_lowercase());

    let mut out: Vec<u8> = Vec::new();
    // Version entry carrying the header extensions.
    out.push(0);
    out.extend_from_slice(b"sreV");
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(b"prefix\0");
    out.extend_from_slice(prefix.as_bytes());
    out.push(0);
    out.push(0);
    let mut bodies = Vec::with_capacity(entries.len());
    for (rel, path) in &entries {
        let body = std::fs::read(path).with_context(|| format!("{}", path.display()))?;
        out.extend_from_slice(rel.as_bytes());
        out.push(0);
        out.extend_from_slice(&[0u8; 4]); // method
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 4]); // reserved
        out.extend_from_slice(&[0u8; 4]); // timestamp
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        bodies.push(body);
    }
    out.extend_from_slice(&[0u8; 21]);
    for b in &bodies {
        out.extend_from_slice(b);
    }
    let digest = sha1_smol::Sha1::from(&out).digest().bytes();
    out.push(0);
    out.extend_from_slice(&digest);
    crate::fsutil::atomic_write(target, &out)?;
    Ok(entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pbo_has_header_prefix_and_trailing_hash() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("sub/a.txt"), b"hello").unwrap();
        let t = d.path().join("x.pbo");
        assert_eq!(pack(d.path().join("sub").parent().unwrap().join("sub").as_path(), &t, "my\\mod").unwrap(), 1);
        let b = std::fs::read(&t).unwrap();
        assert_eq!(&b[1..5], b"sreV");
        assert!(b.windows(6).any(|w| w == b"my\\mod"));
        assert!(b.windows(5).any(|w| w == b"hello"));
        let (body, tail) = b.split_at(b.len() - 21);
        assert_eq!(tail[0], 0);
        assert_eq!(&tail[1..], &sha1_smol::Sha1::from(body).digest().bytes());
    }
}
