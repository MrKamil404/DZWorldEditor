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
    /// How the pixels are sorted into classes.
    pub mode: ClassifyMode,
    /// The colours of [`ClassifyMode::Picked`], each with its surface; at most [`MAX_PICKED`].
    pub picked: Vec<PickedColour>,
}

/// How the pixels of the satellite are sorted into classes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClassifyMode {
    /// Brightness and contrast push each channel off or full: eight strong colours.
    #[default]
    Colours,
    /// Kinds of ground of a real photo ([`natural_classes`]).
    Natural,
    /// Each pixel takes the nearest of colours the user chose ([`picked_classes`]).
    Picked,
}

/// A colour of [`ClassifyMode::Picked`] and the surface it becomes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PickedColour {
    pub colour: [u8; 3],
    pub surface: Option<usize>,
}

/// Most colours of [`ClassifyMode::Picked`].
pub const MAX_PICKED: usize = 16;

impl Default for SatClassify {
    fn default() -> Self {
        Self { adjust: Adjust { brightness: 0.37, contrast: 1.0, legacy: true, ..Adjust::default() }, surfaces: [None; 8], smooth: 1, mode: ClassifyMode::Colours, picked: Vec::new() }
    }
}

impl SatClassify {
    /// Kinds of ground of a real photo, each given to the surface whose name fits it.
    pub fn natural(layers: &[crate::world::SurfaceLayer]) -> Self {
        Self { adjust: Adjust::default(), surfaces: natural_surfaces(layers), smooth: 2, mode: ClassifyMode::Natural, picked: Vec::new() }
    }
    /// `n` colours found in the satellite ([`find_colours`]), each given a surface by
    /// [`guess_surface`].
    pub fn picked(sat: &RgbImage, n: usize, layers: &[crate::world::SurfaceLayer]) -> Self {
        let picked = find_colours(sat, n).into_iter().map(|colour| PickedColour { colour, surface: guess_surface(sat, colour, layers) }).collect();
        Self { adjust: Adjust::default(), surfaces: [None; 8], smooth: 2, mode: ClassifyMode::Picked, picked }
    }
    /// How many classes there are.
    pub fn count(&self) -> usize {
        match self.mode {
            ClassifyMode::Picked => self.picked.len().min(MAX_PICKED),
            _ => 8,
        }
    }
    /// The surface class `k` becomes.
    pub fn surface(&self, k: usize) -> Option<usize> {
        match self.mode {
            ClassifyMode::Picked => self.picked.get(k).and_then(|p| p.surface),
            _ => self.surfaces.get(k).copied().flatten(),
        }
    }
    /// The class of every pixel of the satellite at `width` × `height`.
    pub fn classes(&self, sat: &RgbImage, width: usize, height: usize) -> Vec<u8> {
        match self.mode {
            ClassifyMode::Colours => classes(sat, &self.adjust, width, height),
            ClassifyMode::Natural => natural_classes(sat, &self.adjust, width, height),
            ClassifyMode::Picked => picked_classes(sat, &self.picked.iter().take(MAX_PICKED).map(|p| p.colour).collect::<Vec<_>>(), width, height),
        }
    }
    /// Names of the classes, in English for the interface to translate; none for picked colours.
    pub fn names(&self) -> &'static [&'static str] {
        match self.mode {
            ClassifyMode::Colours => &CLASS_NAMES,
            ClassifyMode::Natural => &NATURAL_NAMES,
            ClassifyMode::Picked => &[],
        }
    }
    /// The colour class `k` is shown in.
    pub fn colour(&self, k: usize) -> [u8; 3] {
        match self.mode {
            ClassifyMode::Colours => CLASS_COLOURS[k & 7],
            ClassifyMode::Natural => NATURAL_COLOURS[k & 7],
            ClassifyMode::Picked => self.picked.get(k).map_or([0, 0, 0], |p| p.colour),
        }
    }
}

/// A colour in CIE L*a*b*, where equal distances look about equally different.
fn lab(c: [u8; 3]) -> [f32; 3] {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f32| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn lab_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Each pixel of the satellite at `width` × `height` takes the nearest of `colours` (by how
/// different they look), so a mask of four surfaces needs only four colours.
pub fn picked_classes(sat: &RgbImage, colours: &[[u8; 3]], width: usize, height: usize) -> Vec<u8> {
    let small = if sat.width == width && sat.height == height { sat.clone() } else { sat.resized(width, height) };
    if colours.is_empty() {
        return vec![0; width * height];
    }
    let targets: Vec<[f32; 3]> = colours.iter().map(|c| lab(*c)).collect();
    small
        .data
        .par_chunks(3)
        .map(|c| {
            let l = lab([c[0], c[1], c[2]]);
            let mut best = (0usize, f32::MAX);
            for (k, t) in targets.iter().enumerate() {
                let d = lab_distance(l, *t);
                if d < best.1 {
                    best = (k, d);
                }
            }
            best.0 as u8
        })
        .collect()
}

/// The `n` colours that stand for the satellite best (k-means in L*a*b* over a sample of its
/// pixels), darkest first. The same picture always gives the same colours.
pub fn find_colours(sat: &RgbImage, n: usize) -> Vec<[u8; 3]> {
    use rand::{Rng, SeedableRng};
    let n = n.clamp(1, MAX_PICKED);
    let px = sat.data.len() / 3;
    if px == 0 {
        return vec![[128, 128, 128]; n];
    }
    let step = (px / 40_000).max(1);
    let rgb: Vec<[u8; 3]> = (0..px).step_by(step).map(|i| [sat.data[i * 3], sat.data[i * 3 + 1], sat.data[i * 3 + 2]]).collect();
    let sample: Vec<[f32; 3]> = rgb.iter().map(|c| lab(*c)).collect();
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    // k-means++: each next centre far from those already taken.
    let mut centres = vec![sample[rng.gen_range(0..sample.len())]];
    let mut near = vec![f32::MAX; sample.len()];
    while centres.len() < n {
        let last = *centres.last().unwrap();
        for (d, s) in near.iter_mut().zip(&sample) {
            *d = d.min(lab_distance(*s, last));
        }
        let total: f32 = near.iter().sum();
        if total <= 0.0 {
            break;
        }
        let mut pick = rng.gen_range(0.0..total);
        let mut chosen = sample.len() - 1;
        for (i, d) in near.iter().enumerate() {
            if pick < *d {
                chosen = i;
                break;
            }
            pick -= d;
        }
        centres.push(sample[chosen]);
    }
    let k = centres.len();
    let mut owner = vec![0usize; sample.len()];
    for _ in 0..25 {
        owner.par_iter_mut().zip(&sample).for_each(|(o, s)| {
            *o = (0..k).min_by(|a, b| lab_distance(*s, centres[*a]).total_cmp(&lab_distance(*s, centres[*b]))).unwrap_or(0);
        });
        let mut sum = vec![[0f32; 4]; k];
        for (o, s) in owner.iter().zip(&sample) {
            for c in 0..3 {
                sum[*o][c] += s[c];
            }
            sum[*o][3] += 1.0;
        }
        for (c, s) in centres.iter_mut().zip(&sum) {
            if s[3] > 0.0 {
                *c = [s[0] / s[3], s[1] / s[3], s[2] / s[3]];
            }
        }
    }
    // Each colour is the average of the real pixels it took, darkest first.
    let mut sum = vec![[0u64; 4]; k];
    for (o, c) in owner.iter().zip(&rgb) {
        for i in 0..3 {
            sum[*o][i] += c[i] as u64;
        }
        sum[*o][3] += 1;
    }
    let mut out: Vec<([u8; 3], f32)> = sum.iter().zip(&centres).filter(|(s, _)| s[3] > 0).map(|(s, c)| ([(s[0] / s[3]) as u8, (s[1] / s[3]) as u8, (s[2] / s[3]) as u8], c[0])).collect();
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out.into_iter().map(|o| o.0).collect()
}

/// The surface a colour of the satellite most likely is: the kind of ground it looks like in
/// that picture ([`natural_kinds`]) given to the surface whose name fits, or else the surface
/// whose colour is nearest.
pub fn guess_surface(sat: &RgbImage, colour: [u8; 3], layers: &[crate::world::SurfaceLayer]) -> Option<usize> {
    let kind = natural_kinds(sat, &[colour])[0];
    natural_surfaces(layers)[kind as usize].or_else(|| nearest_layer(colour, layers))
}

/// The surface whose colour looks nearest to `colour`.
pub fn nearest_layer(colour: [u8; 3], layers: &[crate::world::SurfaceLayer]) -> Option<usize> {
    let c = lab(colour);
    (0..layers.len()).min_by(|a, b| lab_distance(c, lab(layers[*a].color)).total_cmp(&lab_distance(c, lab(layers[*b].color))))
}

/// Names of the kinds of ground of [`natural`], in English for the interface to translate.
pub const NATURAL_NAMES: [&str; 8] = ["Dense forest", "Leafy forest", "Grass", "Pale grass", "Soil", "Fields", "Rock", "Roads and roofs"];

/// Colours the kinds of ground of [`natural`] are shown in.
pub const NATURAL_COLOURS: [[u8; 3]; 8] = [[30, 70, 34], [92, 120, 40], [90, 170, 60], [170, 210, 120], [140, 100, 60], [225, 200, 90], [190, 190, 190], [80, 80, 90]];

/// The surface each kind of ground of [`natural`] goes to, found by words in the names of
/// the layers (`forest_con`, `cp_broadleaf_dense1`, `en_stubble`...); `None` where none fits.
pub fn natural_surfaces(layers: &[crate::world::SurfaceLayer]) -> [Option<usize>; 8] {
    const WORDS: [&[&str]; 8] = [
        &["forest_con", "conifer", "spruce", "pine", "forest"],
        &["forest_dec", "broadleaf", "decid", "leaf", "forest"],
        &["grass1", "grass"],
        &["grass2", "grass_dry", "dry", "meadow", "tall", "grass"],
        &["soil", "dirt", "mud", "ground"],
        &["stubble", "field", "farm", "crop", "wheat"],
        &["stone", "rock", "gravel", "cliff"],
        &["tarmac", "asphalt", "concrete", "paving", "stone"],
    ];
    let names: Vec<String> = layers.iter().map(|l| l.name.to_ascii_lowercase()).collect();
    let mut out = [None; 8];
    for (k, words) in WORDS.iter().enumerate() {
        // The first word some layer has; a layer another kind already took only if nothing else fits.
        out[k] = words.iter().find_map(|w| {
            let mut hits = names.iter().enumerate().filter(|(_, n)| n.contains(w)).map(|(i, _)| i);
            let all: Vec<usize> = hits.by_ref().collect();
            all.iter().copied().find(|i| !out.contains(&Some(*i))).or(all.first().copied())
        });
    }
    out
}

/// The class of every pixel of an image of `width` × `height` made from the satellite (each
/// pixel the average of the satellite pixels under it).
pub fn classes(sat: &RgbImage, adjust: &Adjust, width: usize, height: usize) -> Vec<u8> {
    let small = if sat.width == width && sat.height == height { sat.clone() } else { sat.resized(width, height) };
    small.data.par_chunks(3).map(|c| colour_class(adjust.apply([c[0], c[1], c[2]]))).collect()
}

/// Kinds of ground of a real (natural colour) satellite photo, from
/// [`natural_classes`].
pub mod natural {
    pub const DENSE_FOREST: u8 = 0;
    pub const FOREST: u8 = 1;
    pub const GRASS: u8 = 2;
    pub const PALE_GRASS: u8 = 3;
    pub const SOIL: u8 = 4;
    pub const FIELD: u8 = 5;
    pub const ROCK: u8 = 6;
    pub const PAVED: u8 = 7;
}

/// The kind of ground (see [`natural`]) of every pixel of a real satellite photo at
/// `width` × `height`. Photos differ in brightness, so lightness is measured against the
/// darkest and lightest twentieth of the picture; plants are told apart by how much greener
/// than red and blue they are, bare ground by how much redder than blue, and rock and paving
/// by having little colour. `adjust` changes the colours after that measure is taken, so
/// lighter makes less forest and more grass and rock.
pub fn natural_classes(sat: &RgbImage, adjust: &Adjust, width: usize, height: usize) -> Vec<u8> {
    let small = if sat.width == width && sat.height == height { sat.clone() } else { sat.resized(width, height) };
    let range = lightness_range(&small);
    small.data.par_chunks(3).map(|c| natural_kind(adjust.apply([c[0], c[1], c[2]]), range)).collect()
}

/// The kind of ground (see [`natural`]) each colour looks like in this satellite photo.
pub fn natural_kinds(sat: &RgbImage, colours: &[[u8; 3]]) -> Vec<u8> {
    let range = lightness_range(sat);
    colours.iter().map(|c| natural_kind(*c, range)).collect()
}

fn lum(c: [u8; 3]) -> f32 {
    (c[0] as f32 + c[1] as f32 + c[2] as f32) / 3.0
}

/// The lightness of the darkest and the lightest twentieth of a picture.
fn lightness_range(img: &RgbImage) -> (f32, f32) {
    let mut sample: Vec<f32> = img.data.chunks(3).step_by(((img.data.len() / 3) / 200_000).max(1)).map(|c| lum([c[0], c[1], c[2]])).collect();
    sample.sort_by(f32::total_cmp);
    let at = |q: f32| sample.get(((sample.len() as f32 - 1.0) * q) as usize).copied().unwrap_or(0.0);
    (at(0.05), at(0.95).max(at(0.05) + 1.0))
}

fn natural_kind(c: [u8; 3], (lo, hi): (f32, f32)) -> u8 {
    use natural::*;
    let (r, g, b) = (c[0] as f32, c[1] as f32, c[2] as f32);
    let sum = r + g + b + 1.0;
    let l = ((lum(c) - lo) / (hi - lo)).clamp(0.0, 1.0);
    let green = (2.0 * g - r - b) / sum;
    let red = (r - b) / sum;
    let chroma = (r.max(g).max(b) - r.min(g).min(b)) / (r.max(g).max(b) + 1.0);
    if l < 0.22 {
        if red > 0.12 { FOREST } else { DENSE_FOREST }
    } else if chroma < 0.12 && l > 0.45 {
        if l > 0.75 { ROCK } else { PAVED }
    } else if green > 0.06 {
        if l > 0.6 { PALE_GRASS } else if l < 0.35 { FOREST } else { GRASS }
    } else if red > 0.1 {
        if l > 0.5 { FIELD } else if l < 0.35 { FOREST } else { SOIL }
    } else if l > 0.65 {
        ROCK
    } else if l < 0.35 {
        FOREST
    } else {
        PALE_GRASS
    }
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
            let mut count = [0u16; MAX_PICKED];
            for dy in -r..=r {
                let yy = (y as i64 + dy).clamp(0, height as i64 - 1) as usize;
                for dx in -r..=r {
                    if dx * dx + dy * dy > r * r {
                        continue;
                    }
                    let xx = (x as i64 + dx).clamp(0, width as i64 - 1) as usize;
                    count[(classes[yy * width + xx] as usize).min(MAX_PICKED - 1)] += 1;
                }
            }
            let own = classes[y * width + x].min(MAX_PICKED as u8 - 1);
            // The pixel's own class wins a tie, so edges do not creep.
            let mut best = own;
            for k in 0..MAX_PICKED as u8 {
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
            let Some(layer) = c.surface(*class as usize).filter(|l| *l < layers) else { continue };
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
    fn picked_colours_split_the_picture() {
        // Dark forest on the left, a pale field on the right, with some noise.
        let mut img = RgbImage::new(64, 32, [0, 0, 0]);
        for y in 0..32 {
            for x in 0..64 {
                let n = ((x * 7 + y * 13) % 9) as u8;
                let c = if x < 32 { [30 + n, 50 + n, 30] } else { [195 + n, 165, 110 + n] };
                img.data[(y * 64 + x) * 3..(y * 64 + x) * 3 + 3].copy_from_slice(&c);
            }
        }
        let found = find_colours(&img, 2);
        assert_eq!(found.len(), 2);
        assert!(found[0][1] < 60 && found[1][0] > 180, "{found:?}");
        let cls = picked_classes(&img, &found, 64, 32);
        assert_eq!((cls[5], cls[60]), (0, 1));
        let w = world();
        let c = SatClassify::picked(&img, 2, &w.layers);
        assert_eq!(c.count(), 2);
        // The field looks like none of the named kinds the layers have, so it takes the nearest
        // colour: the sand, not the grass.
        assert_eq!(c.surface(1), Some(1));
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
