//! A satellite image made from the surface mask, and a surface mask read from the satellite
//! image.
//!
//! The game draws the satellite image on the terrain far from the camera and mixes it with the
//! surface textures close by, so a satellite made of the colours of the surfaces themselves
//! fits what the player sees near and far. Each pixel takes the colour of the surfaces around
//! it (with ragged, soft edges between them), a little of the texture's own pattern, larger
//! lighter and darker patches, and the shading of the hills.
//!
//! The other way round, the satellite is first pushed to a few strong colours with a
//! brightness and contrast as Photoshop's legacy one does it, and each of the eight colours
//! that can come out (every channel off or full) is then given a surface.
use crate::noise::Perlin;
use crate::satmod::{Adjust, Key, Tile, TILE};
use crate::terrain::RgbImage;
use crate::world::World;
use crate::Job;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// The colour texture of a surface layer read from the work drive, as a square of `size`
/// RGBA pixels, with its average colour; `fallback` when the file cannot be read.
pub fn layer_texture(root: &Path, texture: &str, fallback: [u8; 3], size: usize) -> (Vec<u8>, [u8; 3], bool) {
    let file = root.join(texture.replace('\\', "/"));
    let decoded = std::fs::read(&file).ok().and_then(|b| forest_core::paa::decode(&b, size as u32).ok());
    let mut rgba = Vec::with_capacity(size * size * 4);
    let mut sum = [0u64; 3];
    let found = decoded.is_some();
    match decoded {
        Some(t) => {
            let (w, h) = (t.width as usize, t.height as usize);
            for y in 0..size {
                for x in 0..size {
                    let p = (y * h / size * w + x * w / size) * 4;
                    rgba.extend_from_slice(&[t.rgba[p], t.rgba[p + 1], t.rgba[p + 2], t.rgba[p + 3]]);
                    for c in 0..3 {
                        sum[c] += t.rgba[p + c] as u64;
                    }
                }
            }
        }
        None => {
            for _ in 0..size * size {
                rgba.extend_from_slice(&[fallback[0], fallback[1], fallback[2], 255]);
            }
            for c in 0..3 {
                sum[c] = fallback[c] as u64 * (size * size) as u64;
            }
        }
    }
    let n = (size * size).max(1) as u64;
    (rgba, [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8], found)
}

/// How a surface looks from above: its colour, and the pattern of light and dark of its
/// texture, small enough that a texel is about a pixel of the satellite.
///
/// The colour textures of the game's surfaces are dark and brownish on purpose: in the game the
/// colour of the ground comes from the satellite image, and the texture adds its pattern. So
/// the colour is chosen for each surface (see `ground_colour`), and only the pattern is taken
/// from the texture.
#[derive(Clone, Debug)]
pub struct LayerLook {
    pub colour: [u8; 3],
    /// Brightness of each texel against the texture's average, `side` × `side`.
    pub pattern: Vec<f32>,
    pub side: usize,
}

impl LayerLook {
    /// A look of `colour` with the pattern of an RGBA texture of `size` × `size`, scaled down
    /// to `side` texels.
    pub fn new(colour: [u8; 3], rgba: &[u8], size: usize, side: usize) -> Self {
        let side = side.clamp(1, size.max(1));
        let step = size / side;
        let lum = |p: usize| 0.299 * rgba[p] as f32 + 0.587 * rgba[p + 1] as f32 + 0.114 * rgba[p + 2] as f32;
        let mut pattern = Vec::with_capacity(side * side);
        for y in 0..side {
            for x in 0..side {
                let (mut s, mut n) = (0.0f32, 0u32);
                for sy in (y * step..(y + 1) * step).step_by((step / 4).max(1)) {
                    for sx in (x * step..(x + 1) * step).step_by((step / 4).max(1)) {
                        let p = (sy * size + sx) * 4;
                        if p + 2 < rgba.len() {
                            s += lum(p);
                            n += 1;
                        }
                    }
                }
                pattern.push(s / n.max(1) as f32);
            }
        }
        let mean = pattern.iter().sum::<f32>() / pattern.len().max(1) as f32;
        for v in &mut pattern {
            *v = if mean > 1.0 { (*v / mean).clamp(0.3, 2.5) } else { 1.0 };
        }
        Self { colour, pattern, side }
    }
    /// A plain colour without a pattern.
    pub fn plain(colour: [u8; 3]) -> Self {
        Self { colour, pattern: vec![1.0], side: 1 }
    }
    #[inline]
    fn pattern_at(&self, u: f32, v: f32) -> f32 {
        let x = (u.rem_euclid(1.0) * self.side as f32) as usize % self.side;
        let y = (v.rem_euclid(1.0) * self.side as f32) as usize % self.side;
        self.pattern[y * self.side + x]
    }
}

/// A colour of the ground as seen from above for a surface, guessed from its name; grey when
/// the name says nothing known.
pub fn ground_colour(name: &str) -> [u8; 3] {
    let n = name.to_ascii_lowercase();
    let has = |k: &[&str]| k.iter().any(|w| n.contains(w));
    if has(&["asphalt", "tarmac", "road"]) {
        [62, 62, 60]
    } else if has(&["concrete", "pavement", "tiles", "sidewalk", "plaster", "industr"]) {
        [118, 114, 106]
    } else if has(&["rock", "stone", "cliff", "boulder"]) {
        [112, 108, 98]
    } else if has(&["sand", "beach", "dune"]) {
        [172, 156, 118]
    } else if has(&["gravel"]) {
        [124, 114, 96]
    } else if has(&["mud", "swamp", "marsh", "bog", "wet"]) {
        [74, 70, 48]
    } else if has(&["dirt", "soil", "ground", "path", "track"]) {
        [112, 92, 66]
    } else if has(&["stubble", "field", "crop", "wheat", "hay", "farm"]) {
        [150, 138, 84]
    } else if has(&["snow", "ice"]) {
        [222, 226, 232]
    } else if has(&["conifer", "pine", "spruce", "needle"]) {
        [48, 62, 36]
    } else if has(&["forest", "broadleaf", "leaf", "wood", "moss"]) {
        [62, 76, 40]
    } else if has(&["grass_dry", "dry", "steppe", "heath"]) {
        [128, 122, 72]
    } else if has(&["grass", "meadow", "lawn", "green"]) {
        [84, 102, 52]
    } else {
        [110, 108, 92]
    }
}

/// The average colour of the satellite image over each surface of the mask, for surfaces that
/// cover enough of it to tell; read on a grid of about `samples` points a side.
pub fn colours_from_satellite(w: &World, samples: usize) -> Vec<Option<[u8; 3]>> {
    let Some(sat) = &w.satellite else { return vec![None; w.layers.len()] };
    let size = w.meta.size_m();
    let mut sum = vec![[0u64; 4]; w.layers.len()];
    let n = samples.max(2);
    for j in 0..n {
        for i in 0..n {
            let (x, z) = ((i as f32 + 0.5) / n as f32 * size, (j as f32 + 0.5) / n as f32 * size);
            let l = w.mask.sample(x, z, size) as usize;
            let Some(s) = sum.get_mut(l) else { continue };
            let px = ((x / size * sat.width as f32) as usize).min(sat.width - 1);
            let py = (((1.0 - z / size) * sat.height as f32) as usize).min(sat.height - 1);
            let p = (py * sat.width + px) * 3;
            for c in 0..3 {
                s[c] += sat.data[p + c] as u64;
            }
            s[3] += 1;
        }
    }
    sum.iter().map(|s| (s[3] >= 20).then(|| [(s[0] / s[3]) as u8, (s[1] / s[3]) as u8, (s[2] / s[3]) as u8])).collect()
}

/// The settings of a satellite made from the mask.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SatGenSettings {
    /// Metres one repeat of a surface texture covers.
    pub texture_m: f32,
    /// How much of the light and dark pattern of the texture shows, 0 to 1.
    pub detail: f32,
    /// Width of the ragged, soft edge between two surfaces, in metres.
    pub blend_m: f32,
    /// How much larger patches are lighter or darker, 0 to 1, and their size in metres.
    pub variation: f32,
    pub variation_m: f32,
    /// How strongly the hills are shaded, 0 to 1, and where the sun stands (degrees from north,
    /// clockwise).
    pub hillshade: f32,
    pub sun_deg: f32,
    /// Ground under the sea level is drawn as water, darker the deeper it is.
    pub water: bool,
    pub water_colour: [u8; 3],
    pub seed: u32,
}

impl Default for SatGenSettings {
    fn default() -> Self {
        Self { texture_m: 8.0, detail: 0.6, blend_m: 3.0, variation: 0.18, variation_m: 140.0, hillshade: 0.4, sun_deg: 315.0, water: true, water_colour: [44, 66, 74], seed: 1 }
    }
}

impl SatGenSettings {
    /// Texels a surface texture needs for one of them to fall on about one pixel of an image
    /// of `metres_per_pixel`.
    pub fn texture_side(&self, metres_per_pixel: f32) -> usize {
        ((self.texture_m / metres_per_pixel.max(0.01)).round() as usize).clamp(2, 512)
    }
}

/// The tiles of a satellite of `width` × `height` pixels made from the mask, the terrain and
/// the looks of the surfaces. `cover` says how much of each pixel it covers (0 to 255, by
/// tile); `None` covers the whole image.
pub fn from_mask(w: &World, looks: &[LayerLook], s: &SatGenSettings, width: usize, height: usize, cover: Option<&HashMap<Key, Vec<u8>>>, job: &Job) -> HashMap<Key, Tile> {
    let size = w.meta.size_m();
    let (cols, rows) = (width.div_ceil(TILE), height.div_ceil(TILE));
    let keys: Vec<Key> = match cover {
        Some(c) => c.keys().copied().collect(),
        None => (0..rows).flat_map(|y| (0..cols).map(move |x| (x as u16, y as u16))).collect(),
    };
    let noise = Perlin::new(s.seed as u64);
    let warp = Perlin::new(s.seed as u64 ^ 0x5eed);
    let (az, el) = (s.sun_deg.to_radians(), 45f32.to_radians());
    let light = [az.sin() * el.cos(), el.sin(), az.cos() * el.cos()];
    let fallback = LayerLook::plain([128, 128, 128]);
    let look = |l: u8| looks.get(l as usize).unwrap_or(&fallback);
    let (px_x, px_z) = (size / width as f32, size / height as f32);
    let blend = s.blend_m.max(0.0);
    // Points round a pixel whose surfaces it mixes, as offsets in units of the blend width.
    const AROUND: [[f32; 2]; 5] = [[0.0, 0.0], [0.5, 0.0], [-0.5, 0.0], [0.0, 0.5], [0.0, -0.5]];
    let done = std::sync::atomic::AtomicUsize::new(0);
    let total = keys.len().max(1);
    keys.par_iter().filter_map(|&key| {
        if job.cancelled() {
            return None;
        }
        let cov = cover.and_then(|c| c.get(&key));
        let mut tile = vec![0u8; TILE * TILE * 4];
        let mut any = false;
        for ty in 0..TILE {
            let py = key.1 as usize * TILE + ty;
            if py >= height {
                break;
            }
            for tx in 0..TILE {
                let px = key.0 as usize * TILE + tx;
                if px >= width {
                    break;
                }
                let t = ty * TILE + tx;
                let alpha = cov.map_or(255, |c| c[t]);
                if alpha == 0 {
                    continue;
                }
                let x = (px as f32 + 0.5) * px_x;
                let z = size - (py as f32 + 0.5) * px_z;
                // Ragged edges: the points the mask is read at wander with the noise.
                let (wx, wz) = if blend > 0.0 {
                    (warp.fbm(x / (blend * 4.0), z / (blend * 4.0), 3, 0.5, 2.0) * blend, warp.fbm(z / (blend * 4.0) + 31.7, x / (blend * 4.0) - 11.3, 3, 0.5, 2.0) * blend)
                } else {
                    (0.0, 0.0)
                };
                let (u, v) = (x / s.texture_m.max(0.1), z / s.texture_m.max(0.1));
                let mut c = [0.0f32; 3];
                let samples: &[[f32; 2]] = if blend > 0.0 { &AROUND } else { &AROUND[..1] };
                for o in samples {
                    let l = look(w.mask.sample(x + wx + o[0] * blend, z + wz + o[1] * blend, size));
                    let k = 1.0 + (l.pattern_at(u, v) - 1.0) * s.detail;
                    for (c, v) in c.iter_mut().zip(l.colour) {
                        *c += v as f32 * k;
                    }
                }
                let n = samples.len() as f32;
                let mut c = c.map(|v| v / n);
                if s.variation > 0.0 && s.variation_m > 0.0 {
                    let v = noise.fbm(x / s.variation_m, z / s.variation_m, 4, 0.5, 2.0);
                    let k = 1.0 + v * s.variation;
                    c = c.map(|x| x * k);
                }
                if s.hillshade > 0.0 {
                    let nrm = w.height.normal(x, z);
                    let lit = (nrm[0] * light[0] + nrm[1] * light[1] + nrm[2] * light[2]).max(0.0) / light[1];
                    let k = 1.0 + (lit - 1.0) * s.hillshade;
                    c = c.map(|x| x * k);
                }
                if s.water {
                    let depth = w.meta.sea_level - w.height.sample(x, z);
                    if depth > 0.0 {
                        // Shallow water shows the ground through it; deeper it is the water's colour.
                        let a = (0.45 + depth / 6.0).min(1.0);
                        let deep = (1.0 - (depth / 40.0).min(0.6)).max(0.4);
                        for k in 0..3 {
                            c[k] = c[k] * (1.0 - a) + s.water_colour[k] as f32 * deep * a;
                        }
                    }
                }
                let p = t * 4;
                for k in 0..3 {
                    tile[p + k] = c[k].round().clamp(0.0, 255.0) as u8;
                }
                tile[p + 3] = alpha;
                any = true;
            }
        }
        let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        job.set_progress(n as f32 / total as f32);
        any.then(|| (key, Arc::new(tile)))
    }).collect()
}

/// Groups pixels with a strength (as `satmod::shape_pixels` gives them) into tiles of coverage.
pub fn cover_tiles(pixels: &[(usize, usize, u32)]) -> HashMap<Key, Vec<u8>> {
    let mut out: HashMap<Key, Vec<u8>> = HashMap::new();
    for &(x, y, a) in pixels {
        let key = ((x / TILE) as u16, (y / TILE) as u16);
        let t = out.entry(key).or_insert_with(|| vec![0u8; TILE * TILE]);
        let i = (y % TILE) * TILE + x % TILE;
        t[i] = t[i].max(a.min(255) as u8);
    }
    out
}

// ---------------------------------------------------------------- mask from the satellite

/// The eight colours a pixel can come out as when every channel is either off or full, in the
/// order of `colour_class`.
pub const CLASS_COLOURS: [[u8; 3]; 8] = [
    [0, 0, 0],
    [255, 0, 0],
    [0, 255, 0],
    [255, 255, 0],
    [0, 0, 255],
    [255, 0, 255],
    [0, 255, 255],
    [255, 255, 255],
];

/// Names of the classes, in English for the interface to translate.
pub const CLASS_NAMES: [&str; 8] = ["Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White"];

/// Which of the eight colours a colour is nearest to: every channel off or full.
#[inline]
pub fn colour_class(c: [u8; 3]) -> u8 {
    (c[0] >= 128) as u8 | ((c[1] >= 128) as u8) << 1 | ((c[2] >= 128) as u8) << 2
}

/// The settings of a mask read from the satellite.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SatClassify {
    /// What is done to the colours before they are sorted into classes; its `legacy` mode
    /// works as Photoshop's legacy brightness and contrast.
    pub adjust: Adjust,
    /// The surface layer each class becomes; `None` leaves the mask there as it is.
    pub surfaces: [Option<usize>; 8],
    /// Radius in mask pixels of the smoothing that takes away specks: each pixel takes the
    /// class most pixels round it have.
    pub smooth: u32,
}

impl Default for SatClassify {
    fn default() -> Self {
        Self { adjust: Adjust { brightness: 0.37, contrast: 1.0, legacy: true, ..Adjust::default() }, surfaces: [None; 8], smooth: 1 }
    }
}

/// The class of every pixel of an image of `width` × `height` made from the satellite (each
/// pixel the average of the satellite pixels under it).
pub fn classes(sat: &RgbImage, adjust: &Adjust, width: usize, height: usize) -> Vec<u8> {
    let small = if sat.width == width && sat.height == height { sat.clone() } else { sat.resized(width, height) };
    small.data.par_chunks(3).map(|c| colour_class(adjust.apply([c[0], c[1], c[2]]))).collect()
}

/// Each pixel takes the class most pixels within `radius` have, which takes away specks.
pub fn smooth_classes(classes: &[u8], width: usize, height: usize, radius: u32) -> Vec<u8> {
    if radius == 0 {
        return classes.to_vec();
    }
    let r = radius as i64;
    let mut out = vec![0u8; classes.len()];
    out.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        for (x, v) in row.iter_mut().enumerate() {
            let mut count = [0u16; 8];
            for dy in -r..=r {
                let yy = (y as i64 + dy).clamp(0, height as i64 - 1) as usize;
                for dx in -r..=r {
                    if dx * dx + dy * dy > r * r {
                        continue;
                    }
                    let xx = (x as i64 + dx).clamp(0, width as i64 - 1) as usize;
                    count[(classes[yy * width + xx] & 7) as usize] += 1;
                }
            }
            let own = classes[y * width + x] & 7;
            // The pixel's own class wins a tie, so edges do not creep.
            let mut best = own;
            for k in 0..8u8 {
                if count[k as usize] > count[best as usize] {
                    best = k;
                }
            }
            *v = best;
        }
    });
    out
}

/// The mask with every pixel whose class has a surface set to it, inside `area` when given
/// (a test of world x and z).
pub fn mask_from_classes(w: &World, classes: &[u8], c: &SatClassify, area: Option<&(dyn Fn(f32, f32) -> bool + Sync)>) -> Vec<u8> {
    let (mw, mh) = (w.mask.width, w.mask.height);
    let size = w.meta.size_m();
    let layers = w.layers.len();
    let mut out = w.mask.data.clone();
    out.par_chunks_mut(mw).enumerate().for_each(|(y, row)| {
        let wz = (1.0 - (y as f32 + 0.5) / mh as f32) * size;
        for (x, v) in row.iter_mut().enumerate() {
            let Some(class) = classes.get(y * mw + x) else { continue };
            let Some(layer) = c.surfaces[(*class & 7) as usize].filter(|l| *l < layers) else { continue };
            if let Some(a) = area {
                if !a((x as f32 + 0.5) / mw as f32 * size, wz) {
                    continue;
                }
            }
            *v = layer as u8;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{SurfaceLayer, WorldMeta};

    fn world() -> World {
        let mut w = World::new(WorldMeta::new("T", 64, 4.0), 10.0);
        w.mask = crate::terrain::MaskMap::new(64, 64, 0);
        w.layers = vec![
            SurfaceLayer { name: "grass".into(), texture: String::new(), material: String::new(), color: [0, 255, 0] },
            SurfaceLayer { name: "sand".into(), texture: String::new(), material: String::new(), color: [255, 255, 0] },
        ];
        // The east half is sand.
        for y in 0..64 {
            for x in 32..64 {
                w.mask.data[y * 64 + x] = 1;
            }
        }
        w
    }

    #[test]
    fn satellite_takes_the_colours_of_the_surfaces() {
        let w = world();
        let looks = vec![LayerLook::plain([40, 120, 40]), LayerLook::plain([200, 180, 120])];
        let s = SatGenSettings { variation: 0.0, hillshade: 0.0, blend_m: 0.0, water: false, ..Default::default() };
        let tiles = from_mask(&w, &looks, &s, 256, 256, None, &Job::default());
        let t = &tiles[&(0, 0)];
        let px = |x: usize, y: usize| { let p = (y * TILE + x) * 4; [t[p], t[p + 1], t[p + 2], t[p + 3]] };
        assert_eq!(px(10, 10), [40, 120, 40, 255]);
        let t = &tiles[&(1, 0)];
        let p = (10 * TILE + 100) * 4;
        assert_eq!([t[p], t[p + 1], t[p + 2]], [200, 180, 120]);
    }

    #[test]
    fn satellite_only_where_it_is_asked_for() {
        let w = world();
        let looks = vec![LayerLook::plain([40, 120, 40]), LayerLook::plain([200, 180, 120])];
        let cover = cover_tiles(&[(5, 5, 255), (6, 5, 100)]);
        let tiles = from_mask(&w, &looks, &SatGenSettings::default(), 256, 256, Some(&cover), &Job::default());
        assert_eq!(tiles.len(), 1);
        let t = &tiles[&(0, 0)];
        assert_eq!(t[(5 * TILE + 5) * 4 + 3], 255);
        assert_eq!(t[(5 * TILE + 6) * 4 + 3], 100);
        assert_eq!(t[(5 * TILE + 7) * 4 + 3], 0);
    }

    #[test]
    fn surfaces_get_colours_by_name_and_from_the_satellite() {
        assert_eq!(ground_colour("cp_grass"), [84, 102, 52]);
        assert_eq!(ground_colour("cp_conifer_common1"), [48, 62, 36]);
        let mut w = world();
        w.satellite = Some(RgbImage::new(64, 64, [10, 200, 30]));
        let c = colours_from_satellite(&w, 32);
        assert_eq!(c, vec![Some([10, 200, 30]), Some([10, 200, 30])]);
    }

    #[test]
    fn legacy_contrast_splits_colours_into_eight_classes() {
        let a = Adjust { contrast: 1.0, legacy: true, ..Adjust::default() };
        assert_eq!(colour_class(a.apply([200, 190, 40])), 3); // yellow
        assert_eq!(colour_class(a.apply([30, 40, 20])), 0); // black
        assert_eq!(colour_class(a.apply([140, 60, 50])), 1); // red
    }

    #[test]
    fn classes_become_surfaces_and_specks_go() {
        let w = world();
        let mut cls = vec![2u8; 64 * 64];
        cls[10 * 64 + 10] = 3; // one speck
        for y in 40..60 {
            for x in 0..20 {
                cls[y * 64 + x] = 3;
            }
        }
        let cls = smooth_classes(&cls, 64, 64, 1);
        assert_eq!(cls[10 * 64 + 10], 2);
        let c = SatClassify { surfaces: [None, None, None, Some(1), None, None, None, None], ..Default::default() };
        let mask = mask_from_classes(&w, &cls, &c, None);
        assert_eq!(mask[50 * 64 + 10], 1);
        assert_eq!(mask[10 * 64 + 10], 0);
        // Classes without a surface leave the mask as it was: the sand of the east half stays.
        assert_eq!(mask[10 * 64 + 50], 1);
    }
}
