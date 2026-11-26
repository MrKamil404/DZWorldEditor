//! PAA texture writer: DXT1 and DXT5 with mipmaps and LZO block compression.
use rayon::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Dxt1,
    Dxt5,
}

/// One 4x4 block of RGBA pixels into DXT colour endpoints and indices.
fn compress_color(px: &[[u8; 4]; 16], out: &mut Vec<u8>) {
    let (mut mn, mut mx) = ([255i32; 3], [0i32; 3]);
    let mut mean = [0i32; 3];
    for p in px {
        for c in 0..3 {
            mn[c] = mn[c].min(p[c] as i32);
            mx[c] = mx[c].max(p[c] as i32);
            mean[c] += p[c] as i32;
        }
    }
    let m = [mean[0] as f32 / 16.0, mean[1] as f32 / 16.0, mean[2] as f32 / 16.0];
    // Principal axis by power iteration, seeded with the box diagonal.
    let mut cov = [0.0f32; 6];
    for p in px {
        let d = [p[0] as f32 - m[0], p[1] as f32 - m[1], p[2] as f32 - m[2]];
        cov[0] += d[0] * d[0];
        cov[1] += d[0] * d[1];
        cov[2] += d[0] * d[2];
        cov[3] += d[1] * d[1];
        cov[4] += d[1] * d[2];
        cov[5] += d[2] * d[2];
    }
    let mut axis = [(mx[0] - mn[0]) as f32 + 0.1, (mx[1] - mn[1]) as f32 + 0.1, (mx[2] - mn[2]) as f32 + 0.1];
    for _ in 0..4 {
        let v = [
            axis[0] * cov[0] + axis[1] * cov[1] + axis[2] * cov[2],
            axis[0] * cov[1] + axis[1] * cov[3] + axis[2] * cov[4],
            axis[0] * cov[2] + axis[1] * cov[4] + axis[2] * cov[5],
        ];
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if l < 1e-6 {
            break;
        }
        axis = [v[0] / l, v[1] / l, v[2] / l];
    }
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    let (mut a, mut b) = (px[0], px[0]);
    for p in px {
        let t = (p[0] as f32 - m[0]) * axis[0] + (p[1] as f32 - m[1]) * axis[1] + (p[2] as f32 - m[2]) * axis[2];
        if t < lo {
            lo = t;
            a = *p;
        }
        if t > hi {
            hi = t;
            b = *p;
        }
    }
    // Colours on opposite ends of the seed's normal (red and green, say) all fall on the same
    // point of the axis, which then gives a single colour for the whole block: the two pixels
    // farthest apart are the ends instead.
    if a[..3] == b[..3] {
        let dist = |p: &[u8; 4], q: &[u8; 4]| (0..3).map(|c| (p[c] as i32 - q[c] as i32).pow(2)).sum::<i32>();
        let mut far = 0;
        for (i, p) in px.iter().enumerate() {
            for q in &px[i + 1..] {
                let d = dist(p, q);
                if d > far {
                    far = d;
                    (a, b) = (*p, *q);
                }
            }
        }
    }
    let to565 = |p: [u8; 4]| -> u16 {
        let q = |v: u8, n: u32| (v as u32 * n + 127) / 255;
        ((q(p[0], 31) << 11) | (q(p[1], 63) << 5) | q(p[2], 31)) as u16
    };
    let (mut c0, mut c1) = (to565(b), to565(a));
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    let expand = |c: u16| -> [i32; 3] {
        let (r, g, bl) = ((c >> 11) as i32, ((c >> 5) & 63) as i32, (c & 31) as i32);
        [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (bl << 3) | (bl >> 2)]
    };
    let (e0, e1) = (expand(c0), expand(c1));
    let pal = [
        e0,
        e1,
        [(2 * e0[0] + e1[0]) / 3, (2 * e0[1] + e1[1]) / 3, (2 * e0[2] + e1[2]) / 3],
        [(e0[0] + 2 * e1[0]) / 3, (e0[1] + 2 * e1[1]) / 3, (e0[2] + 2 * e1[2]) / 3],
    ];
    let mut bits = 0u32;
    if c0 != c1 {
        for (i, p) in px.iter().enumerate() {
            let mut best = (i32::MAX, 0u32);
            for (k, q) in pal.iter().enumerate() {
                let d = (p[0] as i32 - q[0]).pow(2) + (p[1] as i32 - q[1]).pow(2) + (p[2] as i32 - q[2]).pow(2);
                if d < best.0 {
                    best = (d, k as u32);
                }
            }
            bits |= best.1 << (i * 2);
        }
    }
    out.extend_from_slice(&c0.to_le_bytes());
    out.extend_from_slice(&c1.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
}

fn compress_alpha(px: &[[u8; 4]; 16], out: &mut Vec<u8>) {
    let (mut a0, mut a1) = (0u8, 255u8);
    for p in px {
        a0 = a0.max(p[3]);
        a1 = a1.min(p[3]);
    }
    if a0 == a1 {
        out.push(a0);
        out.push(a1);
        out.extend_from_slice(&[0; 6]);
        return;
    }
    // Three levels, two of them 0 and 255 (as the blue surfaces of terrain masks have): the
    // mode with 0 and 255 in its palette keeps all three exactly.
    let middle: Vec<u8> = px.iter().map(|p| p[3]).filter(|a| *a != 0 && *a != 255).collect();
    if let Some(&v) = middle.first() {
        if middle.iter().all(|a| *a == v) && px.iter().any(|p| p[3] == 0 || p[3] == 255) {
            let mut bits = 0u64;
            for (i, p) in px.iter().enumerate() {
                let k: u64 = match p[3] { 0 => 6, 255 => 7, _ => 0 };
                bits |= k << (i * 3);
            }
            out.push(v);
            out.push(v);
            out.extend_from_slice(&bits.to_le_bytes()[..6]);
            return;
        }
    }
    let (x0, x1) = (a0 as i32, a1 as i32);
    let pal: [i32; 8] = [x0, x1, (6 * x0 + x1) / 7, (5 * x0 + 2 * x1) / 7, (4 * x0 + 3 * x1) / 7, (3 * x0 + 4 * x1) / 7, (2 * x0 + 5 * x1) / 7, (x0 + 6 * x1) / 7];
    let mut bits = 0u64;
    for (i, p) in px.iter().enumerate() {
        let mut best = (i32::MAX, 0u64);
        for (k, q) in pal.iter().enumerate() {
            let d = (p[3] as i32 - q).abs();
            if d < best.0 {
                best = (d, k as u64);
            }
        }
        bits |= best.1 << (i * 3);
    }
    out.push(a0);
    out.push(a1);
    out.extend_from_slice(&bits.to_le_bytes()[..6]);
}

/// Compresses a whole level (RGBA bytes, width and height multiples of 4).
pub fn compress_level(rgba: &[u8], w: usize, h: usize, fmt: Format) -> Vec<u8> {
    let block = if fmt == Format::Dxt1 { 8 } else { 16 };
    let mut out = Vec::with_capacity(w / 4 * h / 4 * block);
    let mut px = [[0u8; 4]; 16];
    for by in (0..h).step_by(4) {
        for bx in (0..w).step_by(4) {
            for y in 0..4 {
                for x in 0..4 {
                    let p = ((by + y).min(h - 1) * w + (bx + x).min(w - 1)) * 4;
                    px[y * 4 + x] = [rgba[p], rgba[p + 1], rgba[p + 2], rgba[p + 3]];
                }
            }
            if fmt == Format::Dxt5 {
                compress_alpha(&px, &mut out);
            }
            compress_color(&px, &mut out);
        }
    }
    out
}

fn half(rgba: &[u8], w: usize, h: usize) -> (Vec<u8>, usize, usize) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        for x in 0..nw {
            let mut sum = [0u32; 4];
            for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                let p = ((y * 2 + dy).min(h - 1) * w + (x * 2 + dx).min(w - 1)) * 4;
                for c in 0..4 {
                    sum[c] += rgba[p + c] as u32;
                }
            }
            let q = (y * nw + x) * 4;
            for c in 0..4 {
                out[q + c] = ((sum[c] + 2) / 4) as u8;
            }
        }
    }
    (out, nw, nh)
}

/// LZO1X compression of `input`, the format PAA mipmaps use.
pub fn lzo1x_compress(input: &[u8]) -> Vec<u8> {
    let n = input.len();
    let mut out: Vec<u8> = Vec::with_capacity(n / 2 + 64);
    let mut table = vec![0u32; 1 << 14];
    let (mut ip, mut lit) = (0usize, 0usize);
    let mut first = true;

    fn flush(out: &mut Vec<u8>, input: &[u8], lit: usize, end: usize, first: &mut bool) {
        let count = end - lit;
        if count == 0 {
            return;
        }
        if *first && count <= 238 {
            out.push((count + 17) as u8);
        } else if !*first && count <= 3 {
            // Short runs live in the low bits of the previous match.
            let p = out.len() - 2;
            out[p] |= count as u8;
        } else if count <= 18 {
            out.push((count - 3) as u8);
        } else {
            out.push(0);
            let mut rest = count - 18;
            while rest > 255 {
                out.push(0);
                rest -= 255;
            }
            out.push(rest as u8);
        }
        *first = false;
        out.extend_from_slice(&input[lit..end]);
    }

    while ip + 4 < n {
        let v = u32::from_le_bytes(input[ip..ip + 4].try_into().unwrap());
        let h = (v.wrapping_mul(0x1E35A7BD) >> 18) as usize;
        let cand = table[h] as usize;
        table[h] = ip as u32 + 1;
        if cand == 0 || ip + 1 - cand > 16384 || u32::from_le_bytes(input[cand - 1..cand + 3].try_into().unwrap()) != v {
            ip += 1;
            continue;
        }
        let m = cand - 1;
        let mut len = 4;
        while ip + len < n && input[m + len] == input[ip + len] {
            len += 1;
        }
        flush(&mut out, input, lit, ip, &mut first);
        first = false;
        let d = ip - m;
        if len <= 8 && d <= 2048 {
            let d1 = d - 1;
            out.push((((len - 1) << 5) | ((d1 & 7) << 2)) as u8);
            out.push((d1 >> 3) as u8);
        } else {
            if len - 2 <= 31 {
                out.push(32 | (len - 2) as u8);
            } else {
                out.push(32);
                let mut rest = len - 33;
                while rest > 255 {
                    out.push(0);
                    rest -= 255;
                }
                out.push(rest as u8);
            }
            let d1 = d - 1;
            out.push((d1 << 2) as u8);
            out.push((d1 >> 6) as u8);
        }
        ip += len;
        lit = ip;
    }
    flush(&mut out, input, lit, n, &mut first);
    out.extend_from_slice(&[17, 0, 0]);
    out
}

/// Writes a complete PAA file for an RGBA image.
///
/// Pixels are written at full scale without dynamic-range normalisation (`dynRange = 0` in
/// DayZ Tools' `TexConvert.cfg` for `*_lco`, `*_lca`, and `*_nopx`), so `CXAM` (`MAXC`) is
/// always `255` on every channel; otherwise the game multiplies decoded pixels by `CXAM / 255`
/// and darkens tiles inconsistently. DXT5 images with any pixel having `alpha < 255` carry the
/// `GALF` (`AlphaInterpolated = 1`) tag, and mipmaps go down to `4x4`, matching `ImageToPAA`.
pub fn encode(rgba: &[u8], width: usize, height: usize, fmt: Format) -> Vec<u8> {
    assert_eq!(rgba.len(), width * height * 4);
    let mut levels: Vec<(Vec<u8>, usize, usize)> = vec![(rgba.to_vec(), width, height)];
    while levels.last().unwrap().1 > 4 && levels.last().unwrap().2 > 4 && levels.len() < 16 {
        let (d, w, h) = levels.last().unwrap();
        let next = half(d, *w, *h);
        levels.push(next);
    }
    let mut avg = [0u64; 4];
    let mut has_alpha = false;
    for p in rgba.chunks_exact(4) {
        for c in 0..4 {
            avg[c] += p[c] as u64;
        }
        if p[3] < 255 {
            has_alpha = true;
        }
    }
    let n = (width * height).max(1) as u64;
    let mut out = Vec::new();
    out.extend_from_slice(&(if fmt == Format::Dxt1 { 0xFF01u16 } else { 0xFF05u16 }).to_le_bytes());
    out.extend_from_slice(b"GGATCGVA");
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&[(avg[2] / n) as u8, (avg[1] / n) as u8, (avg[0] / n) as u8, (avg[3] / n) as u8]);
    out.extend_from_slice(b"GGATCXAM");
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&[255, 255, 255, 255]);
    if fmt == Format::Dxt5 && has_alpha {
        out.extend_from_slice(b"GGATGALF");
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
    }
    out.extend_from_slice(b"GGATSFFO");
    out.extend_from_slice(&64u32.to_le_bytes());
    let table = out.len();
    out.resize(table + 64, 0);
    out.extend_from_slice(&0u16.to_le_bytes());
    for (i, (d, w, h)) in levels.iter().enumerate() {
        let raw = compress_level(d, *w, *h, fmt);
        let packed = lzo1x_compress(&raw);
        let (body, flag) = if packed.len() < raw.len() { (packed, 0x8000u16) } else { (raw, 0) };
        let offset = out.len() as u32;
        out[table + i * 4..table + i * 4 + 4].copy_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(*w as u16 | flag).to_le_bytes());
        out.extend_from_slice(&(*h as u16).to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes()[..3]);
        out.extend_from_slice(&body);
    }
    out.extend_from_slice(&[0u8; 6]);
    out
}

/// Encodes several images in parallel.
pub fn encode_many(items: Vec<(Vec<u8>, usize, usize, Format)>) -> Vec<Vec<u8>> {
    items.into_par_iter().map(|(d, w, h, f)| encode(&d, w, h, f)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: usize, h: usize) -> Vec<u8> {
        let mut v = Vec::new();
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 90, if x > w / 2 { 255 } else { 128 }]);
            }
        }
        v
    }

    #[test]
    fn dxt5_output_decodes_close_to_input() {
        let (w, h) = (64, 64);
        let img = gradient(w, h);
        let paa = encode(&img, w, h, Format::Dxt5);
        let t = forest_core::paa::decode(&paa, 4096).unwrap();
        assert_eq!((t.width as usize, t.height as usize), (w, h));
        let mut err = 0u64;
        for (a, b) in t.rgba.iter().zip(&img) {
            err += (*a as i64 - *b as i64).unsigned_abs();
        }
        assert!(err / (img.len() as u64) < 8, "mean error {}", err / img.len() as u64);
    }

    #[test]
    fn blocks_of_two_mask_colours_keep_both() {
        // Each pair of the pure mask colours, half a block each.
        let colours = [[0u8, 0, 0, 255], [255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];
        for a in 0..4 {
            for b in 0..4 {
                let mut img = Vec::new();
                for _y in 0..4 { for x in 0..4 { img.extend_from_slice(&colours[if x < 2 { a } else { b }]); } }
                let t = forest_core::paa::decode(&encode(&img, 4, 4, Format::Dxt5), 64).unwrap();
                assert_eq!(&t.rgba[..], &img[..], "{a} beside {b}");
            }
        }
    }

    #[test]
    fn three_alpha_levels_of_masks_stay_exact() {
        let mut img = Vec::new();
        for k in 0..16 * 16 {
            img.extend_from_slice(&[0, 0, 255, [255u8, 128, 0][k % 3]]);
        }
        let t = forest_core::paa::decode(&encode(&img, 16, 16, Format::Dxt5), 4096).unwrap();
        for (a, b) in t.rgba.chunks(4).zip(img.chunks(4)) {
            assert_eq!(a[3], b[3]);
        }
    }

    #[test]
    fn dxt1_flat_image_is_exact_and_lzo_shrinks_it() {
        let img = [100u8, 150, 200, 255].repeat(128 * 128);
        let paa = encode(&img, 128, 128, Format::Dxt1);
        assert!(paa.len() < 128 * 128 / 4, "{}", paa.len());
        let t = forest_core::paa::decode(&paa, 4096).unwrap();
        for p in t.rgba.chunks_exact(4) {
            assert!((p[0] as i32 - 100).abs() <= 4 && (p[1] as i32 - 150).abs() <= 4 && (p[2] as i32 - 200).abs() <= 4);
        }
    }

    #[test]
    fn lzo_round_trip_on_mixed_data() {
        let mut data = Vec::new();
        let mut s = 12345u32;
        for i in 0..30000usize {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            let b = if i % 900 < 500 { (i % 7) as u8 } else { (s >> 16) as u8 };
            data.push(b);
        }
        // Rendered through a whole PAA so the shipped decoder validates every instruction.
        let (w, h) = (256usize, 256usize);
        let mut img = Vec::with_capacity(w * h * 4);
        for i in 0..w * h * 4 {
            img.push(data[i % data.len()]);
        }
        let paa = encode(&img, w, h, Format::Dxt5);
        assert!(forest_core::paa::decode(&paa, 4096).is_ok());
    }

    fn read_tags(paa: &[u8]) -> std::collections::HashMap<[u8; 4], Vec<u8>> {
        let mut out = std::collections::HashMap::new();
        let mut p = 2usize;
        while paa.get(p..p + 4) == Some(b"GGAT") {
            let name: [u8; 4] = paa[p + 4..p + 8].try_into().unwrap();
            let len = u32::from_le_bytes(paa[p + 8..p + 12].try_into().unwrap()) as usize;
            out.insert(name, paa[p + 12..p + 12 + len].to_vec());
            p += 12 + len;
        }
        out
    }

    #[test]
    fn cxam_is_full_scale_and_mips_reach_4x4() {
        let dark = [40u8, 60, 80, 255].repeat(64 * 64);
        let paa = encode(&dark, 64, 64, Format::Dxt1);
        let tags = read_tags(&paa);
        assert_eq!(tags.get(b"CXAM").map(Vec::as_slice), Some(&[255, 255, 255, 255][..]));
        assert!(!tags.contains_key(b"GALF"));
        let raw = forest_core::paa::read_raw(&paa, 4096).unwrap();
        let dims: Vec<(u32, u32)> = raw.mips.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(dims, vec![(64, 64), (32, 32), (16, 16), (8, 8), (4, 4)]);
    }

    #[test]
    fn dxt5_with_alpha_writes_galf_tag_and_opaque_omits_it() {
        let opaque = [255u8, 0, 0, 255].repeat(16 * 16);
        assert!(!read_tags(&encode(&opaque, 16, 16, Format::Dxt5)).contains_key(b"GALF"));

        let mut with_alpha = opaque.clone();
        with_alpha[3] = 128;
        let tags = read_tags(&encode(&with_alpha, 16, 16, Format::Dxt5));
        assert_eq!(tags.get(b"GALF").map(Vec::as_slice), Some(&1u32.to_le_bytes()[..]));
        assert_eq!(tags.get(b"CXAM").map(Vec::as_slice), Some(&[255, 255, 255, 255][..]));
        // DXT1 never emits GALF even if input RGBA had alpha < 255.
        assert!(!read_tags(&encode(&with_alpha, 16, 16, Format::Dxt1)).contains_key(b"GALF"));
    }
}
