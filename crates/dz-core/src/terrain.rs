//! Heightmap, mask and satellite rasters.
//!
//! Heightmap vertices sit at `(i * cell, j * cell)` with `j = 0` on the south edge, the same
//! order as the elevation array of a WRP. Mask and satellite images are stored like image
//! files, with row 0 on the north edge.
use anyhow::{bail, ensure, Context, Result};
use rayon::prelude::*;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct Heightmap {
    pub cols: usize,
    pub rows: usize,
    pub cell: f32,
    pub data: Vec<f32>,
}

/// Inclusive rectangle of heightmap vertices or image pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}
impl Rect {
    pub fn union(self, o: Rect) -> Rect {
        Rect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
    pub fn width(&self) -> usize {
        self.x1 + 1 - self.x0
    }
    pub fn height(&self) -> usize {
        self.y1 + 1 - self.y0
    }
}

impl Heightmap {
    pub fn new(cols: usize, rows: usize, cell: f32, value: f32) -> Self {
        Self { cols, rows, cell, data: vec![value; cols * rows] }
    }
    #[inline]
    pub fn at(&self, i: usize, j: usize) -> f32 {
        self.data[j * self.cols + i]
    }
    #[inline]
    pub fn set(&mut self, i: usize, j: usize, v: f32) {
        self.data[j * self.cols + i] = v;
    }
    #[inline]
    pub fn clamped(&self, i: i64, j: i64) -> f32 {
        self.at(i.clamp(0, self.cols as i64 - 1) as usize, j.clamp(0, self.rows as i64 - 1) as usize)
    }
    pub fn width_m(&self) -> f32 {
        self.cols as f32 * self.cell
    }
    pub fn height_m(&self) -> f32 {
        self.rows as f32 * self.cell
    }
    /// Height on the terrain surface at a world position.
    #[inline]
    pub fn sample(&self, x: f32, z: f32) -> f32 {
        let (fx, fz) = (x / self.cell, z / self.cell);
        let (i, j) = (fx.floor(), fz.floor());
        let (a, b) = (fx - i, fz - j);
        let (i, j) = (i as i64, j as i64);
        let (h00, h10) = (self.clamped(i, j), self.clamped(i + 1, j));
        let (h01, h11) = (self.clamped(i, j + 1), self.clamped(i + 1, j + 1));
        // Same split of the quad into two triangles for every cell.
        if a + b <= 1.0 {
            h00 + (h10 - h00) * a + (h01 - h00) * b
        } else {
            h11 + (h01 - h11) * (1.0 - a) + (h10 - h11) * (1.0 - b)
        }
    }
    /// Unit surface normal at a world position (x, y up, z north).
    pub fn normal(&self, x: f32, z: f32) -> [f32; 3] {
        let d = self.cell;
        let dx = self.sample(x + d, z) - self.sample(x - d, z);
        let dz = self.sample(x, z + d) - self.sample(x, z - d);
        let n = [-dx, 2.0 * d, -dz];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        [n[0] / l, n[1] / l, n[2] / l]
    }
    /// Slope in degrees at a world position.
    pub fn slope(&self, x: f32, z: f32) -> f32 {
        self.normal(x, z)[1].clamp(-1.0, 1.0).acos().to_degrees()
    }
    pub fn min_max(&self) -> (f32, f32) {
        self.data.par_iter().fold(|| (f32::MAX, f32::MIN), |r, v| (r.0.min(*v), r.1.max(*v))).reduce(
            || (f32::MAX, f32::MIN),
            |a, b| (a.0.min(b.0), a.1.max(b.1)),
        )
    }
    /// The vertex rectangle covering a world circle, or `None` when it misses the map.
    pub fn rect_around(&self, x: f32, z: f32, radius: f32) -> Option<Rect> {
        self.rect_of(x - radius, z - radius, x + radius, z + radius)
    }
    pub fn rect_of(&self, x0: f32, z0: f32, x1: f32, z1: f32) -> Option<Rect> {
        let c = self.cell;
        let (a, b) = (((x0 / c).floor() as i64).max(0), ((z0 / c).floor() as i64).max(0));
        let (e, f) = (((x1 / c).ceil() as i64).min(self.cols as i64 - 1), ((z1 / c).ceil() as i64).min(self.rows as i64 - 1));
        (a <= e && b <= f).then_some(Rect { x0: a as usize, y0: b as usize, x1: e as usize, y1: f as usize })
    }
    pub fn full_rect(&self) -> Rect {
        Rect { x0: 0, y0: 0, x1: self.cols - 1, y1: self.rows - 1 }
    }
}

/// Parsed ESRI ASCII grid.
pub struct AscGrid {
    pub map: Heightmap,
    pub xll: f64,
    pub yll: f64,
}

fn split_token(b: &[u8], mut p: usize) -> Option<(usize, usize)> {
    while p < b.len() && b[p].is_ascii_whitespace() {
        p += 1;
    }
    if p == b.len() {
        return None;
    }
    let s = p;
    while p < b.len() && !b[p].is_ascii_whitespace() {
        p += 1;
    }
    Some((s, p))
}

pub fn parse_asc(bytes: &[u8]) -> Result<AscGrid> {
    let (mut cols, mut rows, mut xll, mut yll, mut cell, mut nodata) = (0usize, 0usize, 0.0f64, 0.0f64, 0.0f64, -9999.0f32);
    let mut p = 0usize;
    let mut center = false;
    loop {
        let (s, e) = split_token(bytes, p).context("ASC: unexpected end of header")?;
        let key = std::str::from_utf8(&bytes[s..e]).unwrap_or("").to_ascii_lowercase();
        if key.parse::<f64>().is_ok() {
            p = s;
            break;
        }
        let (vs, ve) = split_token(bytes, e).context("ASC: missing header value")?;
        let val: f64 = std::str::from_utf8(&bytes[vs..ve])?.parse().context("ASC: header value")?;
        match key.as_str() {
            "ncols" => cols = val as usize,
            "nrows" => rows = val as usize,
            "xllcorner" => xll = val,
            "yllcorner" => yll = val,
            "xllcenter" => {
                xll = val;
                center = true;
            }
            "yllcenter" => {
                yll = val;
                center = true;
            }
            "cellsize" => cell = val,
            "nodata_value" => nodata = val as f32,
            _ => bail!("ASC: unknown header field {key}"),
        }
        p = ve;
    }
    ensure!(cols > 1 && rows > 1 && cell > 0.0, "ASC: invalid header");
    let _ = center;
    let mut map = Heightmap::new(cols, rows, cell as f32, 0.0);
    // Row 0 of the file is the north edge.
    for row in (0..rows).rev() {
        for col in 0..cols {
            let (s, e) = split_token(bytes, p).context("ASC: too few values")?;
            let v: f32 = std::str::from_utf8(&bytes[s..e])?.parse().context("ASC: value")?;
            map.data[row * cols + col] = if v == nodata { 0.0 } else { v };
            p = e;
        }
    }
    Ok(AscGrid { map, xll, yll })
}

pub fn write_asc(map: &Heightmap, xll: f64, yll: f64, path: &Path) -> Result<()> {
    let mut rows: Vec<String> = (0..map.rows).into_par_iter().map(|row| {
        let j = map.rows - 1 - row;
        let mut s = String::with_capacity(map.cols * 9);
        for i in 0..map.cols {
            use std::fmt::Write;
            let _ = write!(s, "{:.3} ", map.at(i, j));
        }
        s.push('\n');
        s
    }).collect();
    let header = format!(
        "ncols {}\nnrows {}\nxllcorner {xll:.6}\nyllcorner {yll:.6}\ncellsize {:.6}\nNODATA_value -9999\n",
        map.cols, map.rows, map.cell
    );
    rows.insert(0, header);
    crate::fsutil::atomic_write(path, rows.concat().as_bytes())
}

#[derive(Clone, Debug, Default)]
pub struct MaskMap {
    pub width: usize,
    pub height: usize,
    /// Surface layer index per pixel, row 0 on the north edge.
    pub data: Vec<u8>,
}
impl MaskMap {
    pub fn new(width: usize, height: usize, value: u8) -> Self {
        Self { width, height, data: vec![value; width * height] }
    }
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.data[y * self.width + x]
    }
    /// Layer at a world position; `size` is the map side in metres.
    #[inline]
    pub fn sample(&self, wx: f32, wz: f32, size: f32) -> u8 {
        let x = ((wx / size * self.width as f32) as i64).clamp(0, self.width as i64 - 1) as usize;
        let y = (((1.0 - wz / size) * self.height as f32) as i64).clamp(0, self.height as i64 - 1) as usize;
        self.at(x, y)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RgbImage {
    pub width: usize,
    pub height: usize,
    /// RGB bytes, row 0 on the north edge.
    pub data: Vec<u8>,
}
impl RgbImage {
    pub fn new(width: usize, height: usize, rgb: [u8; 3]) -> Self {
        let mut data = Vec::with_capacity(width * height * 3);
        for _ in 0..width * height {
            data.extend_from_slice(&rgb);
        }
        Self { width, height, data }
    }
    pub fn load(path: &Path) -> Result<Self> {
        let img = image::ImageReader::open(path)?.with_guessed_format()?;
        let mut img = img;
        img.no_limits();
        let img = img.decode().with_context(|| format!("{}", path.display()))?.into_rgb8();
        Ok(Self { width: img.width() as usize, height: img.height() as usize, data: img.into_raw() })
    }
    pub fn save_png(&self, path: &Path) -> Result<()> {
        image::save_buffer(path, &self.data, self.width as u32, self.height as u32, image::ColorType::Rgb8)?;
        Ok(())
    }
    /// Averaged downscale to a fixed size.
    pub fn resized(&self, width: usize, height: usize) -> RgbImage {
        let mut out = RgbImage::new(width, height, [0, 0, 0]);
        out.data.par_chunks_mut(width * 3).enumerate().for_each(|(y, row)| {
            let (y0, y1) = (y * self.height / height, ((y + 1) * self.height / height).max(y * self.height / height + 1));
            for x in 0..width {
                let (x0, x1) = (x * self.width / width, ((x + 1) * self.width / width).max(x * self.width / width + 1));
                let mut sum = [0u32; 3];
                let mut n = 0;
                let (stepx, stepy) = (((x1 - x0) / 4).max(1), ((y1 - y0) / 4).max(1));
                for sy in (y0..y1.min(self.height)).step_by(stepy) {
                    for sx in (x0..x1.min(self.width)).step_by(stepx) {
                        let p = (sy * self.width + sx) * 3;
                        for c in 0..3 {
                            sum[c] += self.data[p + c] as u32;
                        }
                        n += 1;
                    }
                }
                let n = n.max(1);
                for c in 0..3 {
                    row[x * 3 + c] = (sum[c] / n) as u8;
                }
            }
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asc_round_trip_keeps_orientation() {
        let text = "ncols 3\nnrows 2\nxllcorner 200000\nyllcorner 0\ncellsize 10\nNODATA_value -9999\n1 2 3\n4 5 -9999\n";
        let g = parse_asc(text.as_bytes()).unwrap();
        assert_eq!(g.map.at(0, 1), 1.0); // north row
        assert_eq!(g.map.at(2, 0), 0.0); // nodata
        assert_eq!(g.xll, 200000.0);
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.asc");
        write_asc(&g.map, 200000.0, 0.0, &p).unwrap();
        let g2 = parse_asc(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(g2.map.data, g.map.data);
    }
    #[test]
    fn sample_interpolates_inside_a_cell() {
        let mut m = Heightmap::new(2, 2, 10.0, 0.0);
        m.set(1, 0, 10.0);
        assert!((m.sample(5.0, 0.0) - 5.0).abs() < 1e-5);
        assert!((m.sample(10.0, 0.0) - 10.0).abs() < 1e-5);
    }
}
