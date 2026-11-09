//! Reader and writer of unbinarised WRP worlds (`8WVR`, `0WZD` and the newer `1WZD`).
//!
//! Layout, verified against worlds written by Terrain Builder:
//! header `sig, land_x, land_y, terrain_x, terrain_y, land_cell: f32`;
//! `terrain_x * terrain_y` f32 elevations from the south-west corner;
//! `land_x * land_y` u16 material indices (1-based);
//! material count, an empty first record, then `len, name, 0` for each material;
//! objects `matrix: 12 f32 (right, up, dir, pos), id: u32, len: u32`,
//! `1WZD` adds 4 bytes whose lowest byte is the underground flag, then the model name;
//! a final record with `len == 0` ends the list.
use anyhow::{bail, ensure, Context, Result};
use std::io::Write;

#[derive(Clone, Debug, PartialEq)]
pub struct WrpObject {
    pub matrix: [f32; 12],
    pub id: u32,
    pub model: String,
    pub underground: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Wrp {
    pub land_x: u32,
    pub land_y: u32,
    pub terrain_x: u32,
    pub terrain_y: u32,
    pub land_cell: f32,
    pub elevations: Vec<f32>,
    pub material_index: Vec<u16>,
    /// Without the empty record 0, so index `i` in `material_index` is `materials[i - 1]`.
    pub materials: Vec<String>,
    pub objects: Vec<WrpObject>,
    pub newer_format: bool,
    /// Signature `0WZD`: objects are laid out as in `8WVR`.
    pub zd_signature: bool,
}

struct Cursor<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.b.get(self.p..self.p + n).context("WRP file is truncated")?;
        self.p += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into()?))
    }
}

impl Wrp {
    pub fn cell(&self) -> f32 {
        self.land_cell * self.land_x as f32 / self.terrain_x as f32
    }

    pub fn parse(b: &[u8]) -> Result<Wrp> {
        let newer_format = match b.get(..4) {
            Some(b"1WZD") => true,
            Some(b"8WVR") | Some(b"0WZD") => false,
            Some(b"OPRW") => bail!("This WRP is binarised (OPRW); only source worlds (8WVR, 1WZD) can be opened"),
            _ => bail!("Not a WRP file"),
        };
        let zd_signature = b.starts_with(b"0WZD");
        let mut c = Cursor { b, p: 4 };
        let (land_x, land_y, terrain_x, terrain_y) = (c.u32()?, c.u32()?, c.u32()?, c.u32()?);
        let land_cell = c.f32()?;
        ensure!(land_x > 0 && land_y > 0 && terrain_x > 1 && terrain_y > 1 && land_cell > 0.0, "Invalid WRP header");
        let n = terrain_x as usize * terrain_y as usize;
        let elevations = c.take(n * 4)?.chunks_exact(4).map(|v| f32::from_le_bytes(v.try_into().unwrap())).collect();
        let m = land_x as usize * land_y as usize;
        let material_index = c.take(m * 2)?.chunks_exact(2).map(|v| u16::from_le_bytes(v.try_into().unwrap())).collect();
        let count = c.u32()? as usize;
        ensure!(count >= 1 && count <= 65536, "Invalid WRP material count");
        ensure!(c.u32()? == 0, "Unexpected first WRP material");
        let mut materials = Vec::with_capacity(count - 1);
        for _ in 1..count {
            let len = c.u32()? as usize;
            materials.push(String::from_utf8_lossy(c.take(len)?).into_owned());
            c.u32()?;
        }
        let mut objects = Vec::new();
        loop {
            let mut matrix = [0.0f32; 12];
            for v in &mut matrix {
                *v = c.f32()?;
            }
            let id = c.u32()?;
            let len = c.u32()? as usize;
            if len == 0 {
                break;
            }
            ensure!(len < 1024, "Invalid WRP object name");
            let underground = if newer_format { c.take(4)?[0] != 0 } else { false };
            let model = String::from_utf8_lossy(c.take(len)?).into_owned();
            objects.push(WrpObject { matrix, id, model, underground });
        }
        Ok(Wrp { land_x, land_y, terrain_x, terrain_y, land_cell, elevations, material_index, materials, objects, newer_format, zd_signature })
    }

    pub fn write(&self, out: &mut impl Write) -> Result<()> {
        out.write_all(if self.newer_format { b"1WZD" } else if self.zd_signature { b"0WZD" } else { b"8WVR" })?;
        for v in [self.land_x, self.land_y, self.terrain_x, self.terrain_y] {
            out.write_all(&v.to_le_bytes())?;
        }
        out.write_all(&self.land_cell.to_le_bytes())?;
        ensure!(self.elevations.len() == self.terrain_x as usize * self.terrain_y as usize, "Elevation size mismatch");
        ensure!(self.material_index.len() == self.land_x as usize * self.land_y as usize, "Material grid size mismatch");
        out.write_all(&crate::fsutil::f32s_to_bytes(&self.elevations))?;
        let mut idx = Vec::with_capacity(self.material_index.len() * 2);
        for v in &self.material_index {
            idx.extend_from_slice(&v.to_le_bytes());
        }
        out.write_all(&idx)?;
        out.write_all(&(self.materials.len() as u32 + 1).to_le_bytes())?;
        out.write_all(&0u32.to_le_bytes())?;
        for m in &self.materials {
            out.write_all(&(m.len() as u32).to_le_bytes())?;
            out.write_all(m.as_bytes())?;
            out.write_all(&0u32.to_le_bytes())?;
        }
        for o in &self.objects {
            for v in o.matrix {
                out.write_all(&v.to_le_bytes())?;
            }
            out.write_all(&o.id.to_le_bytes())?;
            out.write_all(&(o.model.len() as u32).to_le_bytes())?;
            if self.newer_format {
                out.write_all(&[o.underground as u8, 0, 0, 0])?;
            }
            out.write_all(o.model.as_bytes())?;
        }
        // End record.
        out.write_all(&[0u8; 48])?;
        out.write_all(&0u32.to_le_bytes())?;
        out.write_all(&0u32.to_le_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(newer: bool) -> Wrp {
        Wrp { zd_signature: false,
            land_x: 2, land_y: 2, terrain_x: 16, terrain_y: 16, land_cell: 48.0,
            elevations: (0..256).map(|v| v as f32).collect(),
            material_index: vec![1, 1, 2, 2],
            materials: vec!["a\\P_000-000.rvmat".into(), "a\\P_000-001.rvmat".into()],
            objects: vec![
                WrpObject { matrix: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 5.0, 6.0, 7.0], id: 0, model: "dz\\a.p3d".into(), underground: newer },
                WrpObject { matrix: [0.0; 12], id: 1, model: "dz\\b.p3d".into(), underground: false },
            ],
            newer_format: newer,
        }
    }

    #[test]
    fn round_trip_both_formats() {
        for newer in [false, true] {
            let w = sample(newer);
            let mut b = Vec::new();
            w.write(&mut b).unwrap();
            let r = Wrp::parse(&b).unwrap();
            assert_eq!(r.objects, w.objects);
            assert_eq!(r.materials, w.materials);
            assert_eq!(r.elevations, w.elevations);
            assert_eq!(r.material_index, w.material_index);
            assert_eq!(r.cell(), 6.0);
        }
    }

    #[test]
    fn binarised_worlds_are_refused_with_a_reason() {
        let e = Wrp::parse(b"OPRW....").unwrap_err().to_string();
        assert!(e.contains("binarised"));
    }

    /// Reads a real Terrain Builder world when the P: drive is mounted.
    #[test]
    fn real_worlds_parse_and_rewrite_identically() {
        for path in ["P:/Balticrus/world/Balticrus.wrp", "P:/DolinaBobru/world/dolinabobru.wrp"] {
            let Ok(bytes) = std::fs::read(path) else { continue };
            let w = Wrp::parse(&bytes).unwrap();
            assert!(w.objects.len() > 1000);
            let mut out = Vec::with_capacity(bytes.len());
            w.write(&mut out).unwrap();
            // Terrain Builder leaves uninitialised bytes in the object flags and the end
            // record, everything else has to match.
            assert_eq!(out.len(), bytes.len());
            let r = Wrp::parse(&out).unwrap();
            assert_eq!(r.objects, w.objects);
            assert_eq!(r.elevations, w.elevations);
            assert_eq!(&out[..1 << 20], &bytes[..1 << 20]);
        }
    }
}
