//! PAA texture reader: picks one mipmap and decodes it to RGBA. Supports DXT1, DXT5,
//! ARGB8888, ARGB4444, ARGB1555 and AI88, with LZO-compressed DXT mipmaps.
use anyhow::{Result, bail, ensure};

pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A texture as the file stores it: DXT blocks of every mipmap, ready for the graphics card.
#[derive(Debug)]
pub struct Raw {
    /// DXT5 when true, DXT1 otherwise.
    pub dxt5: bool,
    /// Sources of A, R, G, B as in the file: 0..3 are A, R, G, B, 4..7 the same inverted, 8 is one.
    pub swizzle: [u8; 4],
    /// (width, height, blocks), largest first.
    pub mips: Vec<(u32, u32, Vec<u8>)>,
}

/// DXT1 has no alpha channel: a detail map that asks for it keeps its grey in the colour.
fn fix_swizzle(kind: u16, mut order: [u8; 4]) -> [u8; 4] {
    if kind == 0xFF01 {
        for (target, source) in order.iter_mut().enumerate().skip(1) {
            if *source == 0 || *source == 4 {
                *source = target as u8;
            }
        }
    }
    order
}

/// The mipmaps with sides of at most `max` pixels, still compressed. Only DXT1 and DXT5 files.
pub fn read_raw(b: &[u8], max: u32) -> Result<Raw> {
    ensure!(b.len() > 8, "Plik PAA jest za krótki");
    let kind = u16::from_le_bytes([b[0], b[1]]);
    ensure!(kind == 0xFF01 || kind == 0xFF05, "PAA bez kompresji DXT");
    let mut p = 2;
    let mut swizzle = [0, 1, 2, 3];
    while b.get(p..p + 4) == Some(b"GGAT") {
        let len = u32::from_le_bytes(b.get(p + 8..p + 12).unwrap_or(&[0; 4]).try_into()?) as usize;
        if b.get(p + 4..p + 8) == Some(b"ZIWS") && len == 4 {
            if let Some(v) = b.get(p + 12..p + 16) {
                swizzle = [v[0], v[1], v[2], v[3]];
            }
        }
        p += 12 + len;
    }
    let palette = u16::from_le_bytes(b.get(p..p + 2).unwrap_or(&[0; 2]).try_into()?) as usize;
    p += 2 + palette * 3;
    let block = if kind == 0xFF05 { 16 } else { 8 };
    let mut mips: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    let mut smallest = None;
    while p + 7 <= b.len() {
        let w = u16::from_le_bytes([b[p], b[p + 1]]);
        let h = u16::from_le_bytes([b[p + 2], b[p + 3]]);
        if w == 0 || h == 0 {
            break;
        }
        let size = u32::from_le_bytes([b[p + 4], b[p + 5], b[p + 6], 0]) as usize;
        let start = p + 7;
        let Some(data) = b.get(start..start + size) else { break };
        p = start + size;
        let lzo = w & 0x8000 != 0;
        let (w, h) = ((w & 0x7fff) as u32, h as u32);
        // Each level halves the one before it; anything else cannot be given to the card.
        if let Some(&(pw, ph, _)) = mips.last() {
            if w != (pw / 2).max(1) || h != (ph / 2).max(1) {
                break;
            }
        }
        if w.max(h) > max {
            smallest = Some((w, h, lzo, data));
            continue;
        }
        let expected = w.div_ceil(4) as usize * h.div_ceil(4) as usize * block;
        let blocks = if lzo {
            match crate::odol::lzo1x_decompress(data, expected) {
                Ok((d, _)) => d,
                Err(_) => break,
            }
        } else if data.len() == expected {
            data.to_vec()
        } else {
            break;
        };
        mips.push((w, h, blocks));
    }
    if mips.is_empty() {
        // Every level is larger than asked for: take the smallest of them.
        let (w, h, lzo, data) = smallest.ok_or_else(|| anyhow::anyhow!("Brak mipmap w PAA"))?;
        let expected = w.div_ceil(4) as usize * h.div_ceil(4) as usize * block;
        let blocks = if lzo { crate::odol::lzo1x_decompress(data, expected)?.0 } else { data.to_vec() };
        ensure!(blocks.len() == expected, "Uszkodzony mipmap PAA");
        mips.push((w, h, blocks));
    }
    Ok(Raw { dxt5: kind == 0xFF05, swizzle: fix_swizzle(kind, swizzle), mips })
}

/// Decodes the largest mipmap whose sides are at most `max` pixels (or the smallest one).
pub fn decode(b: &[u8], max: u32) -> Result<Texture> {
    ensure!(b.len() > 8, "Plik PAA jest za krótki");
    let kind = u16::from_le_bytes([b[0], b[1]]);
    let mut p = 2;
    let mut swizzle = None;
    while b.get(p..p + 4) == Some(b"GGAT") {
        let len = u32::from_le_bytes(b.get(p + 8..p + 12).unwrap_or(&[0; 4]).try_into()?) as usize;
        if b.get(p + 4..p + 8) == Some(b"ZIWS") && len == 4 {
            swizzle = b.get(p + 12..p + 16).map(|v| [v[0], v[1], v[2], v[3]]);
        }
        p += 12 + len;
    }
    let palette = u16::from_le_bytes(b.get(p..p + 2).unwrap_or(&[0; 2]).try_into()?) as usize;
    p += 2 + palette * 3;
    let mut chosen = None;
    while p + 7 <= b.len() {
        let w = u16::from_le_bytes([b[p], b[p + 1]]);
        let h = u16::from_le_bytes([b[p + 2], b[p + 3]]);
        if w == 0 || h == 0 {
            break;
        }
        let size = u32::from_le_bytes([b[p + 4], b[p + 5], b[p + 6], 0]) as usize;
        let start = p + 7;
        ensure!(start + size <= b.len(), "Uszkodzony mipmap PAA");
        let lzo = w & 0x8000 != 0;
        let (w, h) = ((w & 0x7fff) as u32, h as u32);
        chosen = Some((w, h, lzo, &b[start..start + size]));
        if w.max(h) <= max {
            break;
        }
        p = start + size;
    }
    let Some((w, h, lzo, data)) = chosen else {
        bail!("Brak mipmap w PAA")
    };
    let pixels = w as usize * h as usize;
    let blocks = w.div_ceil(4) as usize * h.div_ceil(4) as usize;
    let expected = match kind {
        0xFF01 => blocks * 8,
        0xFF05 => blocks * 16,
        0x8888 => pixels * 4,
        0x4444 | 0x1555 | 0x8080 => pixels * 2,
        other => bail!("Nieobsługiwany format PAA {other:04X}"),
    };
    let data = if lzo {
        crate::odol::lzo1x_decompress(data, expected)?.0
    } else {
        // Uncompressed mipmaps only; LZSS-packed ARGB data is not supported.
        ensure!(data.len() == expected, "Nieobsługiwana kompresja PAA");
        data.to_vec()
    };
    ensure!(data.len() >= expected, "Uszkodzony mipmap PAA");
    let mut rgba = vec![0u8; pixels * 4];
    match kind {
        0xFF01 | 0xFF05 => dxt(&data, w, h, kind == 0xFF05, &mut rgba),
        _ => {
            for i in 0..pixels {
                let px = match kind {
                    0x8888 => [
                        data[i * 4 + 2],
                        data[i * 4 + 1],
                        data[i * 4],
                        data[i * 4 + 3],
                    ],
                    0x4444 => {
                        let v = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                        let c = |s: u16| (((v >> s) & 15) * 17) as u8;
                        [c(8), c(4), c(0), c(12)]
                    }
                    0x1555 => {
                        let v = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                        let c = |s: u16| (((v >> s) & 31) * 255 / 31) as u8;
                        [c(10), c(5), c(0), if v & 0x8000 != 0 { 255 } else { 0 }]
                    }
                    _ => [data[i * 2], data[i * 2], data[i * 2], data[i * 2 + 1]],
                };
                rgba[i * 4..i * 4 + 4].copy_from_slice(&px);
            }
        }
    }
    if let Some(order) = swizzle {
        apply_swizzle(&mut rgba, fix_swizzle(kind, order));
    }
    Ok(Texture {
        width: w,
        height: h,
        rgba,
    })
}

/// Rearranges channels as the file asks. `order` holds the source for A, R, G, B:
/// 0..3 are A, R, G, B, 4..7 the same inverted, 8 is one.
fn apply_swizzle(rgba: &mut [u8], order: [u8; 4]) {
    for p in rgba.chunks_exact_mut(4) {
        let (r, g, b, a) = (p[0], p[1], p[2], p[3]);
        let pick = |s: u8| match s {
            0 => a,
            1 => r,
            2 => g,
            3 => b,
            4 => 255 - a,
            5 => 255 - r,
            6 => 255 - g,
            7 => 255 - b,
            _ => 255,
        };
        p[3] = pick(order[0]);
        p[0] = pick(order[1]);
        p[1] = pick(order[2]);
        p[2] = pick(order[3]);
    }
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = (c >> 11) & 31;
    let g = (c >> 5) & 63;
    let b = c & 31;
    [
        (r * 255 / 31) as u8,
        (g * 255 / 63) as u8,
        (b * 255 / 31) as u8,
    ]
}

fn dxt(data: &[u8], w: u32, h: u32, dxt5: bool, out: &mut [u8]) {
    let (bw, bh) = (w.div_ceil(4) as usize, h.div_ceil(4) as usize);
    let block = if dxt5 { 16 } else { 8 };
    for by in 0..bh {
        for bx in 0..bw {
            let Some(b) = data.get((by * bw + bx) * block..(by * bw + bx + 1) * block) else {
                return;
            };
            let (alpha, color) = if dxt5 {
                (Some(&b[..8]), &b[8..])
            } else {
                (None, b)
            };
            let c0 = u16::from_le_bytes([color[0], color[1]]);
            let c1 = u16::from_le_bytes([color[2], color[3]]);
            let (a, b2) = (rgb565(c0), rgb565(c1));
            let mix = |x: u8, y: u8, wx: u16, wy: u16| {
                ((x as u16 * wx + y as u16 * wy) / (wx + wy)) as u8
            };
            let four = c0 > c1 || dxt5;
            let palette: [[u8; 4]; 4] = [
                [a[0], a[1], a[2], 255],
                [b2[0], b2[1], b2[2], 255],
                if four {
                    [
                        mix(a[0], b2[0], 2, 1),
                        mix(a[1], b2[1], 2, 1),
                        mix(a[2], b2[2], 2, 1),
                        255,
                    ]
                } else {
                    [
                        mix(a[0], b2[0], 1, 1),
                        mix(a[1], b2[1], 1, 1),
                        mix(a[2], b2[2], 1, 1),
                        255,
                    ]
                },
                if four {
                    [
                        mix(a[0], b2[0], 1, 2),
                        mix(a[1], b2[1], 1, 2),
                        mix(a[2], b2[2], 1, 2),
                        255,
                    ]
                } else {
                    [0, 0, 0, 0]
                },
            ];
            let bits = u32::from_le_bytes([color[4], color[5], color[6], color[7]]);
            let alphas = alpha.map(|a| {
                let (a0, a1) = (a[0] as u16, a[1] as u16);
                let mut v = [0u8; 8];
                v[0] = a0 as u8;
                v[1] = a1 as u8;
                for i in 2..8u16 {
                    v[i as usize] = if a0 > a1 {
                        ((a0 * (8 - i) + a1 * (i - 1)) / 7) as u8
                    } else if i < 6 {
                        ((a0 * (6 - i) + a1 * (i - 1)) / 5) as u8
                    } else if i == 6 {
                        0
                    } else {
                        255
                    };
                }
                let idx = u64::from_le_bytes([a[2], a[3], a[4], a[5], a[6], a[7], 0, 0]);
                (v, idx)
            });
            for py in 0..4 {
                for px in 0..4 {
                    let (x, y) = (bx * 4 + px, by * 4 + py);
                    if x >= w as usize || y >= h as usize {
                        continue;
                    }
                    let k = py * 4 + px;
                    let mut c = palette[((bits >> (k * 2)) & 3) as usize];
                    if let Some((v, idx)) = alphas {
                        c[3] = v[((idx >> (k * 3)) & 7) as usize];
                    }
                    let o = (y * w as usize + x) * 4;
                    out[o..o + 4].copy_from_slice(&c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swizzle_reorders_and_inverts_channels() {
        let mut p = [10u8, 20, 30, 40];
        // A = inverted red, R = inverted alpha, G and B unchanged: how normal maps are stored.
        apply_swizzle(&mut p, [5, 4, 2, 3]);
        assert_eq!(p, [215, 20, 30, 245]);
    }

    #[test]
    fn decodes_a_dxt1_block_and_picks_the_mip_that_fits() {
        // Two mipmaps: 8x8 and 4x4. The 4x4 block is pure red (c0 = c1 = 0xF800).
        let mut b = vec![0x01, 0xFF];
        b.extend([0, 0]); // no palette
        b.extend(8u16.to_le_bytes());
        b.extend(8u16.to_le_bytes());
        b.extend([32, 0, 0]);
        b.extend([0u8; 32]);
        b.extend(4u16.to_le_bytes());
        b.extend(4u16.to_le_bytes());
        b.extend([8, 0, 0]);
        b.extend([0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0]);
        b.extend([0, 0, 0, 0]);
        let t = decode(&b, 4).unwrap();
        assert_eq!((t.width, t.height), (4, 4));
        assert_eq!(&t.rgba[..4], &[255, 0, 0, 255]);
        let t = decode(&b, 64).unwrap();
        assert_eq!((t.width, t.height), (8, 8));
    }

    /// A real DayZ road texture when the P: drive is mounted.
    #[test]
    fn decodes_dayz_road_texture() {
        let Ok(dir) = std::fs::read_dir("P:/dz/structures/roads/data") else {
            return;
        };
        let Some(path) = dir
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.to_string_lossy().to_lowercase().ends_with("_co.paa"))
        else {
            return;
        };
        let t = decode(&std::fs::read(&path).unwrap(), 256).unwrap();
        assert!(t.width <= 256 && t.width >= 4, "{}", path.display());
        let mean: u64 = t
            .rgba
            .chunks(4)
            .map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64)
            .sum::<u64>()
            / (t.rgba.len() as u64 / 4);
        assert!(mean > 20 && mean < 740, "{} mean {mean}", path.display());
    }
}
