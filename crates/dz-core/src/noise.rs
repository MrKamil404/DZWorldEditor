//! Seeded 2D gradient noise and its fractal sums.

#[derive(Clone)]
pub struct Perlin {
    perm: [u8; 512],
}

impl Perlin {
    pub fn new(seed: u64) -> Self {
        let mut p: Vec<u8> = (0..=255).collect();
        let mut s = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
        for i in (1..256).rev() {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            p.swap(i, (s % (i as u64 + 1)) as usize);
        }
        let mut perm = [0u8; 512];
        for i in 0..512 {
            perm[i] = p[i & 255];
        }
        Self { perm }
    }
    #[inline]
    fn grad(hash: u8, x: f32, y: f32) -> f32 {
        const G: [(f32, f32); 8] = [
            (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0),
            (0.7071, 0.7071), (-0.7071, 0.7071), (0.7071, -0.7071), (-0.7071, -0.7071),
        ];
        let (a, b) = G[(hash & 7) as usize];
        a * x + b * y
    }
    /// Noise in roughly -1..1.
    #[inline]
    pub fn get(&self, x: f32, y: f32) -> f32 {
        let (fx, fy) = (x.floor(), y.floor());
        let (xi, yi) = ((fx as i64 & 255) as usize, (fy as i64 & 255) as usize);
        let (xf, yf) = (x - fx, y - fy);
        let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        let (u, v) = (fade(xf), fade(yf));
        let p = &self.perm;
        let (aa, ab) = (p[p[xi] as usize + yi], p[p[xi] as usize + yi + 1]);
        let (ba, bb) = (p[p[xi + 1] as usize + yi], p[p[xi + 1] as usize + yi + 1]);
        let l1 = Self::grad(aa, xf, yf) + u * (Self::grad(ba, xf - 1.0, yf) - Self::grad(aa, xf, yf));
        let l2 = Self::grad(ab, xf, yf - 1.0) + u * (Self::grad(bb, xf - 1.0, yf - 1.0) - Self::grad(ab, xf, yf - 1.0));
        (l1 + v * (l2 - l1)) * 1.414
    }
    /// Fractal sum normalised to roughly -1..1.
    pub fn fbm(&self, x: f32, y: f32, octaves: u32, gain: f32, lacunarity: f32) -> f32 {
        let (mut sum, mut amp, mut freq, mut total) = (0.0, 1.0, 1.0, 0.0);
        for o in 0..octaves {
            let shift = o as f32 * 17.3;
            sum += self.get(x * freq + shift, y * freq - shift) * amp;
            total += amp;
            amp *= gain;
            freq *= lacunarity;
        }
        sum / total
    }
    /// A fractal sum whose finer octaves fade where the ground is already steep, as water
    /// wears slopes smooth and leaves detail on flats and crests. `erosion` 0 is plain `fbm`.
    pub fn eroded(&self, x: f32, y: f32, octaves: u32, gain: f32, lacunarity: f32, erosion: f32) -> f32 {
        const STEP: f32 = 0.01;
        let (mut sum, mut amp, mut freq, mut total) = (0.0, 1.0, 1.0, 0.0);
        let (mut dx, mut dy) = (0.0f32, 0.0f32);
        for o in 0..octaves {
            let shift = o as f32 * 17.3;
            let (px, py) = (x * freq + shift, y * freq - shift);
            let n = self.get(px, py);
            // The slope built up so far damps what comes after it.
            dx += (self.get(px + STEP, py) - n) / STEP * amp;
            dy += (self.get(px, py + STEP) - n) / STEP * amp;
            sum += n * amp / (1.0 + erosion * (dx * dx + dy * dy));
            total += amp;
            amp *= gain;
            freq *= lacunarity;
        }
        sum / total * (1.0 + erosion * 0.6)
    }
    /// Sharp crests in 0..1.
    pub fn ridged(&self, x: f32, y: f32, octaves: u32, gain: f32, lacunarity: f32) -> f32 {
        let (mut sum, mut amp, mut freq, mut total, mut weight) = (0.0, 1.0, 1.0, 0.0, 1.0f32);
        for o in 0..octaves {
            let shift = o as f32 * 31.7;
            let n = 1.0 - self.get(x * freq + shift, y * freq + shift).abs();
            let n = n * n * weight;
            weight = (n * 2.0).clamp(0.0, 1.0);
            sum += n * amp;
            total += amp;
            amp *= gain;
            freq *= lacunarity;
        }
        sum / total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn noise_is_deterministic_and_bounded() {
        let a = Perlin::new(7);
        let b = Perlin::new(7);
        let c = Perlin::new(8);
        let mut differs = false;
        for i in 0..2000 {
            let (x, y) = (i as f32 * 0.37, i as f32 * 0.11);
            let v = a.get(x, y);
            assert_eq!(v, b.get(x, y));
            assert!(v.abs() <= 1.05, "{v}");
            differs |= v != c.get(x, y);
        }
        assert!(differs);
    }
}
