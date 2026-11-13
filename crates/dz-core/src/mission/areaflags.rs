//! `areaflags.map`: the usage and value flags of the Central Economy painted over the map.
//!
//! The file is a header of four numbers (cells across, cells down, world width and height in
//! metres), then two layers. Each layer starts with its bits per cell and holds one row of
//! cells after another, the first row at the south edge of the map (z = 0). Cells smaller than
//! a byte fill it from the low bits up. Bit n of a usage cell is the n-th usage flag of
//! `cfglimitsdefinition.xml`, bit n of a value cell the n-th value flag.
use anyhow::{bail, ensure, Result};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagLayer {
    Usage,
    Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AreaFlags {
    pub width: u32,
    pub height: u32,
    pub world_width: u32,
    pub world_height: u32,
    pub usage_bits: u32,
    pub usage: Arc<Vec<u8>>,
    pub value_bits: u32,
    pub value: Arc<Vec<u8>>,
    /// Bytes after the layers, kept as they were.
    pub trailing: Vec<u8>,
}

fn layer_len(w: u32, h: u32, bits: u32) -> usize {
    (w as usize * h as usize * bits as usize).div_ceil(8)
}

fn valid_bits(bits: u32) -> bool {
    matches!(bits, 1 | 2 | 4 | 8 | 16 | 32)
}

/// Bits a layer needs for that many flags.
pub fn bits_for(flags: usize) -> u32 {
    [1u32, 2, 4, 8, 16, 32].into_iter().find(|b| *b as usize >= flags).unwrap_or(32)
}

impl AreaFlags {
    /// An empty map of `cells` by `cells` over a world of `world` metres.
    pub fn new(cells: u32, world: u32, usage_bits: u32, value_bits: u32) -> Self {
        Self {
            width: cells,
            height: cells,
            world_width: world,
            world_height: world,
            usage_bits,
            usage: Arc::new(vec![0; layer_len(cells, cells, usage_bits)]),
            value_bits,
            value: Arc::new(vec![0; layer_len(cells, cells, value_bits)]),
            trailing: Vec::new(),
        }
    }

    pub fn parse(b: &[u8]) -> Result<Self> {
        ensure!(b.len() >= 20, "areaflags.map is too short");
        let u = |at: usize| -> Result<u32> {
            ensure!(at + 4 <= b.len(), "areaflags.map ends early");
            Ok(u32::from_le_bytes(b[at..at + 4].try_into().unwrap()))
        };
        let (width, height, world_width, world_height) = (u(0)?, u(4)?, u(8)?, u(12)?);
        ensure!(width > 0 && height > 0 && width <= 65536 && height <= 65536, "areaflags.map has an unlikely size {width} x {height}");
        let usage_bits = u(16)?;
        if !valid_bits(usage_bits) {
            bail!("areaflags.map: {usage_bits} bits per usage cell");
        }
        let ul = layer_len(width, height, usage_bits);
        let at = 20;
        ensure!(b.len() >= at + ul + 4, "areaflags.map ends inside the usage layer");
        let usage = b[at..at + ul].to_vec();
        let at = at + ul;
        let value_bits = u(at)?;
        if !valid_bits(value_bits) {
            bail!("areaflags.map: {value_bits} bits per value cell");
        }
        let vl = layer_len(width, height, value_bits);
        let at = at + 4;
        ensure!(b.len() >= at + vl, "areaflags.map ends inside the value layer");
        let value = b[at..at + vl].to_vec();
        let trailing = b[at + vl..].to_vec();
        Ok(Self { width, height, world_width, world_height, usage_bits, usage: Arc::new(usage), value_bits, value: Arc::new(value), trailing })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24 + self.usage.len() + self.value.len() + self.trailing.len());
        for v in [self.width, self.height, self.world_width, self.world_height, self.usage_bits] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.usage);
        out.extend_from_slice(&self.value_bits.to_le_bytes());
        out.extend_from_slice(&self.value);
        out.extend_from_slice(&self.trailing);
        out
    }

    fn layer(&self, l: FlagLayer) -> (&[u8], u32) {
        match l {
            FlagLayer::Usage => (&self.usage, self.usage_bits),
            FlagLayer::Value => (&self.value, self.value_bits),
        }
    }

    pub fn bits(&self, l: FlagLayer) -> u32 {
        self.layer(l).1
    }

    /// The flags of a cell; x goes east, y north.
    pub fn get(&self, l: FlagLayer, x: u32, y: u32) -> u32 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        let (data, bits) = self.layer(l);
        let cell = y as usize * self.width as usize + x as usize;
        read_cell(data, bits, cell)
    }

    pub fn set(&mut self, l: FlagLayer, x: u32, y: u32, v: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let cell = y as usize * self.width as usize + x as usize;
        let (data, bits) = match l {
            FlagLayer::Usage => (Arc::make_mut(&mut self.usage), self.usage_bits),
            FlagLayer::Value => (Arc::make_mut(&mut self.value), self.value_bits),
        };
        write_cell(data, bits, cell, v);
    }

    /// Metres per cell along x and z.
    pub fn cell_size(&self) -> (f32, f32) {
        (self.world_width as f32 / self.width as f32, self.world_height as f32 / self.height as f32)
    }

    /// The cell under a point of the world.
    pub fn cell_at(&self, x: f32, z: f32) -> Option<(u32, u32)> {
        let (cw, ch) = self.cell_size();
        let (cx, cy) = ((x / cw).floor(), (z / ch).floor());
        (cx >= 0.0 && cy >= 0.0 && (cx as u32) < self.width && (cy as u32) < self.height).then_some((cx as u32, cy as u32))
    }

    /// Turns `bit` on (or off) in every cell whose centre `inside` accepts, within the cell
    /// rectangle `[x0, x1) x [y0, y1)`. Returns how many cells changed.
    pub fn paint(&mut self, l: FlagLayer, bit: u32, on: bool, x0: u32, y0: u32, x1: u32, y1: u32, inside: impl Fn(f32, f32) -> bool) -> usize {
        let (cw, ch) = self.cell_size();
        let mut n = 0;
        let mask = 1u32.checked_shl(bit).unwrap_or(0);
        if mask == 0 || bit >= self.bits(l) {
            return 0;
        }
        for y in y0..y1.min(self.height) {
            for x in x0..x1.min(self.width) {
                if !inside((x as f32 + 0.5) * cw, (y as f32 + 0.5) * ch) {
                    continue;
                }
                let v = self.get(l, x, y);
                let nv = if on { v | mask } else { v & !mask };
                if nv != v {
                    self.set(l, x, y, nv);
                    n += 1;
                }
            }
        }
        n
    }

    /// The flags of a rectangle of cells, for undo.
    pub fn copy_rect(&self, l: FlagLayer, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<u32> {
        let mut out = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
        for y in y0..y1 {
            for x in x0..x1 {
                out.push(self.get(l, x, y));
            }
        }
        out
    }

    pub fn paste_rect(&mut self, l: FlagLayer, x0: u32, y0: u32, x1: u32, y1: u32, values: &[u32]) {
        let mut i = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                if let Some(v) = values.get(i) {
                    self.set(l, x, y, *v);
                }
                i += 1;
            }
        }
    }

    /// Cells of each flag of a layer, for statistics.
    pub fn counts(&self, l: FlagLayer) -> Vec<usize> {
        let (data, bits) = self.layer(l);
        let mut counts = vec![0usize; bits as usize];
        let cells = self.width as usize * self.height as usize;
        if bits == 32 {
            for c in data.chunks_exact(4) {
                let v = u32::from_le_bytes(c.try_into().unwrap());
                if v != 0 {
                    for (b, n) in counts.iter_mut().enumerate() {
                        *n += (v >> b & 1) as usize;
                    }
                }
            }
        } else {
            for cell in 0..cells {
                let v = read_cell(data, bits, cell);
                if v != 0 {
                    for (b, n) in counts.iter_mut().enumerate() {
                        *n += (v >> b & 1) as usize;
                    }
                }
            }
        }
        counts
    }

    /// The same flags over a grid of another size, or a world of another size.
    pub fn resized(&self, cells: u32, world: u32) -> Self {
        let mut out = Self::new(cells, world, self.usage_bits, self.value_bits);
        for y in 0..cells {
            for x in 0..cells {
                let wx = (x as f32 + 0.5) * world as f32 / cells as f32;
                let wz = (y as f32 + 0.5) * world as f32 / cells as f32;
                if let Some((sx, sy)) = self.cell_at(wx, wz) {
                    out.set(FlagLayer::Usage, x, y, self.get(FlagLayer::Usage, sx, sy));
                    out.set(FlagLayer::Value, x, y, self.get(FlagLayer::Value, sx, sy));
                }
            }
        }
        out
    }

    /// The layer with `bits` per cell; flags beyond them are dropped.
    pub fn with_bits(&self, l: FlagLayer, bits: u32) -> Self {
        let mut out = self.clone();
        let cells = self.width as usize * self.height as usize;
        let (old, old_bits) = self.layer(l);
        let mut data = vec![0u8; layer_len(self.width, self.height, bits)];
        let keep = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
        for c in 0..cells {
            write_cell(&mut data, bits, c, read_cell(old, old_bits, c) & keep);
        }
        match l {
            FlagLayer::Usage => {
                out.usage_bits = bits;
                out.usage = Arc::new(data);
            }
            FlagLayer::Value => {
                out.value_bits = bits;
                out.value = Arc::new(data);
            }
        }
        out
    }
}

#[inline]
fn read_cell(data: &[u8], bits: u32, cell: usize) -> u32 {
    match bits {
        32 => data.get(cell * 4..cell * 4 + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).unwrap_or(0),
        16 => data.get(cell * 2..cell * 2 + 2).map(|b| u16::from_le_bytes(b.try_into().unwrap()) as u32).unwrap_or(0),
        8 => data.get(cell).copied().unwrap_or(0) as u32,
        _ => {
            let bit = cell * bits as usize;
            let byte = data.get(bit / 8).copied().unwrap_or(0);
            (byte >> (bit % 8)) as u32 & ((1u32 << bits) - 1)
        }
    }
}

#[inline]
fn write_cell(data: &mut [u8], bits: u32, cell: usize, v: u32) {
    match bits {
        32 => {
            if let Some(b) = data.get_mut(cell * 4..cell * 4 + 4) {
                b.copy_from_slice(&v.to_le_bytes());
            }
        }
        16 => {
            if let Some(b) = data.get_mut(cell * 2..cell * 2 + 2) {
                b.copy_from_slice(&(v as u16).to_le_bytes());
            }
        }
        8 => {
            if let Some(b) = data.get_mut(cell) {
                *b = v as u8;
            }
        }
        _ => {
            let bit = cell * bits as usize;
            let mask = ((1u32 << bits) - 1) as u8;
            if let Some(b) = data.get_mut(bit / 8) {
                let shift = bit % 8;
                *b = (*b & !(mask << shift)) | (((v as u8) & mask) << shift);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_what_it_reads() {
        let mut a = AreaFlags::new(64, 1024, 32, 4);
        a.set(FlagLayer::Usage, 3, 5, 0b1010);
        a.set(FlagLayer::Value, 6, 9, 0b0110);
        a.set(FlagLayer::Value, 7, 9, 0b0001);
        let b = a.to_bytes();
        assert_eq!(b.len(), 20 + 64 * 64 * 4 + 4 + 64 * 64 / 2);
        let back = AreaFlags::parse(&b).unwrap();
        assert_eq!(back, a);
        assert_eq!(back.get(FlagLayer::Usage, 3, 5), 0b1010);
        assert_eq!(back.get(FlagLayer::Value, 6, 9), 0b0110);
        assert_eq!(back.get(FlagLayer::Value, 7, 9), 0b0001);
        // Two four-bit cells share a byte, the first in the low bits.
        let byte = back.value[(9 * 64 + 6) / 2];
        assert_eq!(byte, 0b0001_0110);
        assert_eq!(back.to_bytes(), b);
    }

    #[test]
    fn paints_flags_in_world_space() {
        let mut a = AreaFlags::new(100, 1000, 32, 8);
        // Row 0 is the south edge: z = 5 m is in row 0.
        assert_eq!(a.cell_at(15.0, 5.0), Some((1, 0)));
        let n = a.paint(FlagLayer::Usage, 6, true, 0, 0, 100, 100, |x, z| (x - 500.0).hypot(z - 500.0) < 50.0);
        assert!(n > 70 && n < 90, "{n}");
        assert_eq!(a.counts(FlagLayer::Usage)[6], n);
        assert_eq!(a.get(FlagLayer::Usage, 50, 50), 1 << 6);
        let r = a.resized(50, 1000);
        assert_eq!(r.get(FlagLayer::Usage, 25, 25), 1 << 6);
        assert_eq!(bits_for(5), 8);
        assert_eq!(bits_for(4), 4);
        assert_eq!(bits_for(17), 32);
    }
}
