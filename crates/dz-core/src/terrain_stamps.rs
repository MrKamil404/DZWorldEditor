//! Terrain stamps: a form of the ground (a crater, a hill, a quarry...) laid on the terrain as
//! a modifier that can be moved, turned, scaled and taken away again.
//!
//! A stamp is a square raster of relief from -1 (as deep as its height) to 1 (as high), with 0
//! at its edges, so that it fits into the ground round it. The forms that come with the editor
//! are worked out here; others come from grey pictures (white high, black low) or from a piece of
//! the terrain of a map.
use crate::edit::Stamp;
use crate::noise::Perlin;
use crate::terrain::Heightmap;
use serde::{Deserialize, Serialize};

/// Side of the raster of the stamps that come with the editor.
pub const RES: usize = 96;
/// Largest side of a stamp read from a picture or the terrain.
pub const MAX_RES: usize = 192;

/// The forms that come with the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StampForm {
    Crater,
    Hill,
    Ridge,
    Quarry,
    Mound,
    Valley,
    Volcano,
    Dunes,
}

impl StampForm {
    pub const ALL: [StampForm; 8] = [StampForm::Crater, StampForm::Hill, StampForm::Ridge, StampForm::Quarry, StampForm::Mound, StampForm::Valley, StampForm::Volcano, StampForm::Dunes];

    pub fn name(self) -> &'static str {
        match self {
            StampForm::Crater => "Crater",
            StampForm::Hill => "Hill",
            StampForm::Ridge => "Ridge",
            StampForm::Quarry => "Quarry",
            StampForm::Mound => "Embankment",
            StampForm::Valley => "Valley",
            StampForm::Volcano => "Volcano",
            StampForm::Dunes => "Dunes",
        }
    }

    /// Side in metres and height of the relief in metres it is first laid with.
    pub fn defaults(self) -> (f32, f32) {
        match self {
            StampForm::Crater => (60.0, 6.0),
            StampForm::Hill => (260.0, 30.0),
            StampForm::Ridge => (420.0, 45.0),
            StampForm::Quarry => (140.0, 18.0),
            StampForm::Mound => (60.0, 4.0),
            StampForm::Valley => (500.0, 25.0),
            StampForm::Volcano => (700.0, 90.0),
            StampForm::Dunes => (300.0, 6.0),
        }
    }

    /// The raster of the form.
    pub fn stamp(self) -> Stamp {
        let noise = Perlin::new(self as u64 + 11);
        let n = RES;
        let mut data = Vec::with_capacity(n * n);
        for y in 0..n {
            for x in 0..n {
                // -1..1 across, y growing to the north.
                let (u, v) = ((x as f32 + 0.5) / n as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / n as f32 * 2.0);
                let r = (u * u + v * v).sqrt();
                let rough = noise.fbm(u * 3.0, v * 3.0, 4, 0.5, 2.0);
                let value = match self {
                    // A bowl with a raised rim, its outer slope fading out.
                    StampForm::Crater => {
                        if r < 0.62 {
                            -1.0 + 1.35 * smooth(0.0, 0.62, r).powf(1.6)
                        } else {
                            0.35 * (1.0 - smooth(0.62, 1.0, r))
                        }
                    }
                    StampForm::Hill => (1.0 - smooth(0.0, 1.0, r)).powf(1.3) * (1.0 + 0.18 * rough),
                    // Long along west and east, with a ragged crest.
                    StampForm::Ridge => {
                        let along = 1.0 - smooth(0.55, 1.0, u.abs());
                        let across = (1.0 - smooth(0.0, 0.5 + 0.08 * rough, v.abs())).powf(1.2);
                        along * across * (1.0 + 0.25 * noise.ridged(u * 2.5, v * 2.5, 4, 0.5, 2.0) - 0.12)
                    }
                    // A square pit with stepped walls and a flat floor.
                    StampForm::Quarry => {
                        let d = u.abs().max(v.abs());
                        if d < 0.45 {
                            -1.0
                        } else if d < 0.85 {
                            let t = (d - 0.45) / 0.4;
                            -1.0 + (t * 4.0).floor() / 4.0 + smooth(0.75, 1.0, (t * 4.0).fract()) / 4.0
                        } else {
                            0.0
                        }
                    }
                    // Long and flat-topped, as a dyke or an embankment.
                    StampForm::Mound => {
                        let along = 1.0 - smooth(0.7, 1.0, u.abs());
                        let across = 1.0 - smooth(0.2, 0.55, v.abs());
                        along * across
                    }
                    StampForm::Valley => {
                        let along = 1.0 - smooth(0.6, 1.0, u.abs());
                        let across = (1.0 - smooth(0.0, 0.7, v.abs() + 0.06 * rough)).powf(0.8);
                        -along * across
                    }
                    // A cone with a crater on top.
                    StampForm::Volcano => {
                        let cone = (1.0 - smooth(0.0, 1.0, r)).powf(1.8) * (1.0 + 0.1 * rough);
                        cone - 0.45 * (1.0 - smooth(0.0, 0.16, r))
                    }
                    // Waves of sand, steeper on their lee side.
                    StampForm::Dunes => {
                        let w = (u * 7.0 + 0.6 * rough).fract();
                        let wave = if w < 0.75 { w / 0.75 } else { (1.0 - w) / 0.25 };
                        (wave - 0.4) * (1.0 - smooth(0.6, 1.0, r))
                    }
                };
                data.push(value.clamp(-1.0, 1.0));
            }
        }
        Stamp { width: n, height: n, data }
    }
}

/// The raster of a form, worked out once.
pub fn builtin(form: StampForm) -> &'static Stamp {
    static ALL: std::sync::OnceLock<Vec<Stamp>> = std::sync::OnceLock::new();
    &ALL.get_or_init(|| StampForm::ALL.iter().map(|f| f.stamp()).collect())[form as usize]
}

/// The shape of a terrain modifier laying a form at its first size and height.
pub fn form_shape(form: StampForm) -> crate::world::ModShape {
    let (size, height) = form.defaults();
    crate::world::ModShape::Stamp { stamp: builtin(form).clone(), size, height }
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Relief from grey levels 0..1 (row by row from the top): the mean of the border becomes the
/// ground round it, and the largest step from it 1.
pub fn from_grey(width: usize, height: usize, grey: &[f32]) -> Stamp {
    // Square, taken from the middle.
    let side = width.min(height).max(1);
    let (ox, oy) = ((width - side) / 2, (height - side) / 2);
    let n = side.min(MAX_RES);
    let mut data = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            let (sx, sy) = (ox + x * side / n, oy + y * side / n);
            data.push(grey[sy * width + sx]);
        }
    }
    normalize(&mut data, n);
    Stamp { width: n, height: n, data }
}

/// Moves the values so the mean of the border is 0, and scales them so the largest is 1.
fn normalize(data: &mut [f32], n: usize) {
    let mut border = 0.0;
    let mut count = 0;
    for k in 0..n {
        for v in [data[k], data[(n - 1) * n + k], data[k * n], data[k * n + n - 1]] {
            border += v;
            count += 1;
        }
    }
    let border = border / count.max(1) as f32;
    let top = data.iter().map(|v| (v - border).abs()).fold(1e-6f32, f32::max);
    for v in data.iter_mut() {
        *v = (*v - border) / top;
    }
}

/// A piece of the terrain as a stamp: the square of `size` metres round (`x`, `z`), turned by
/// `yaw` degrees, with the height of its relief in metres.
pub fn from_terrain(h: &Heightmap, x: f32, z: f32, size: f32, yaw: f32) -> (Stamp, f32) {
    let n = ((size / h.cell).round() as usize).clamp(8, MAX_RES);
    let (s, c) = yaw.to_radians().sin_cos();
    let half = size * 0.5;
    let mut data = Vec::with_capacity(n * n);
    for y in 0..n {
        for xk in 0..n {
            // The same axes a stamp modifier lays the raster along.
            let (u, v) = (((xk as f32 + 0.5) / n as f32 * 2.0 - 1.0) * half, (1.0 - (y as f32 + 0.5) / n as f32 * 2.0) * half);
            let (dx, dz) = (u * c + v * s, -u * s + v * c);
            data.push(h.sample(x + dx, z + dz));
        }
    }
    let mut border = 0.0;
    let mut count = 0;
    for k in 0..n {
        for v in [data[k], data[(n - 1) * n + k], data[k * n], data[k * n + n - 1]] {
            border += v;
            count += 1;
        }
    }
    let border = border / count.max(1) as f32;
    let top = data.iter().map(|v| (v - border).abs()).fold(1e-6f32, f32::max);
    normalize(&mut data, n);
    (Stamp { width: n, height: n, data }, top)
}

/// A stamp saved as a 16-bit grey picture: the middle grey is the ground round it.
pub fn to_png16(stamp: &Stamp, path: &std::path::Path) -> anyhow::Result<()> {
    let pixels: Vec<u16> = stamp.data.iter().map(|v| ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * 65535.0).round() as u16).collect();
    let img = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_raw(stamp.width as u32, stamp.height as u32, pixels).ok_or_else(|| anyhow::anyhow!("bad stamp size"))?;
    img.save(path)?;
    Ok(())
}

/// A stamp read from any grey picture (16-bit ones keep their precision).
pub fn from_picture(path: &std::path::Path) -> anyhow::Result<Stamp> {
    let img = image::open(path)?;
    let img = if img.width().max(img.height()) > MAX_RES as u32 * 2 { img.resize(MAX_RES as u32 * 2, MAX_RES as u32 * 2, image::imageops::FilterType::Triangle) } else { img };
    let grey = img.to_luma16();
    let values: Vec<f32> = grey.pixels().map(|p| p.0[0] as f32 / 65535.0).collect();
    Ok(from_grey(grey.width() as usize, grey.height() as usize, &values))
}

/// What a stamp saved beside its picture says about how it is laid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StampInfo {
    pub size: f32,
    pub height: f32,
}

impl Default for StampInfo {
    fn default() -> Self {
        Self { size: 120.0, height: 15.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_fade_out_at_their_edges() {
        for f in StampForm::ALL {
            let s = f.stamp();
            for k in 0..RES {
                for v in [s.data[k], s.data[(RES - 1) * RES + k], s.data[k * RES], s.data[k * RES + RES - 1]] {
                    assert!(v.abs() < 0.12, "{f:?} edge {v}");
                }
            }
            assert!(s.data.iter().all(|v| (-1.0..=1.0).contains(v)));
        }
        // A crater is a bowl, a hill a hump.
        assert!(StampForm::Crater.stamp().sample(0.0, 0.0) < -0.8);
        assert!(StampForm::Hill.stamp().sample(0.0, 0.0) > 0.8);
    }

    #[test]
    fn a_stamp_modifier_shapes_the_ground_and_goes_away_again() {
        use crate::world::{ModMode, ModShape, TerrainModifier, World, WorldMeta};
        let mut w = World::new(WorldMeta::new("T", 128, 2.0), 20.0);
        let m = TerrainModifier {
            id: 1, name: "hill".into(), enabled: true, x: 128.0, z: 128.0, yaw: 30.0,
            shape: ModShape::Stamp { stamp: StampForm::Hill.stamp(), size: 100.0, height: 12.0 },
            mode: ModMode::Offset, target: None, amount: 0.0, falloff: 5.0, attached: None, follow: false,
        };
        w.modifiers.push(m);
        w.recompose(None);
        assert!((w.height.sample(128.0, 128.0) - 32.0).abs() < 1.5, "{}", w.height.sample(128.0, 128.0));
        assert!((w.height.sample(20.0, 20.0) - 20.0).abs() < 1e-3);
        // Laid on a level, the hill stands on it.
        w.modifiers[0].mode = ModMode::Flatten;
        w.modifiers[0].target = Some(50.0);
        w.recompose(None);
        assert!((w.height.sample(128.0, 128.0) - 62.0).abs() < 1.5);
        w.modifiers[0].enabled = false;
        w.recompose(None);
        assert_eq!(w.height.sample(128.0, 128.0), 20.0);
    }

    #[test]
    fn a_road_over_a_stamp_follows_its_ground() {
        use crate::world::{ModMode, ModShape, TerrainModifier, World, WorldMeta};
        let mut w = World::new(WorldMeta::new("T", 128, 2.0), 20.0);
        let hill = TerrainModifier {
            id: 1, name: "hill".into(), enabled: true, x: 128.0, z: 128.0, yaw: 0.0,
            shape: ModShape::Stamp { stamp: StampForm::Hill.stamp(), size: 160.0, height: 12.0 },
            mode: ModMode::Offset, target: None, amount: 0.0, falloff: 0.0, attached: None, follow: false,
        };
        w.modifiers.push(hill);
        // The road comes later in the list, and is designed on the ground with the hill.
        let line = vec![[20.0, 128.0], [236.0, 128.0]];
        let road = crate::gen::road_modifier(&w.ground(), 2, "road", &line, 8.0, 6.0, &[], &Default::default(), &[]).unwrap();
        w.modifiers.push(road);
        w.recompose(None);
        assert!(w.height.sample(128.0, 128.0) > 28.0, "the road cuts through the hill: {}", w.height.sample(128.0, 128.0));
        // A copy for generators lays things on the same ground.
        assert!((w.lite().base.sample(128.0, 140.0) - w.ground().sample(128.0, 140.0)).abs() < 1e-4);
    }

    #[test]
    fn a_picture_becomes_relief_round_its_border() {
        // Grey border, white middle.
        let mut grey = vec![0.5f32; 16 * 16];
        for y in 6..10 {
            for x in 6..10 {
                grey[y * 16 + x] = 1.0;
            }
        }
        let s = from_grey(16, 16, &grey);
        assert!((s.sample(0.0, 0.0) - 1.0).abs() < 1e-3);
        assert!(s.data[0].abs() < 1e-3);
    }

    #[test]
    fn terrain_round_trips_through_a_stamp() {
        let mut h = Heightmap::new(64, 64, 2.0, 10.0);
        for j in 0..64 {
            for i in 0..64 {
                let d = ((i as f32 - 32.0).powi(2) + (j as f32 - 32.0).powi(2)).sqrt();
                h.set(i, j, 10.0 + (8.0 - d * 0.5).max(0.0));
            }
        }
        let (s, height) = from_terrain(&h, 64.0, 64.0, 80.0, 0.0);
        assert!((height - 8.0).abs() < 0.6, "{height}");
        assert!(s.sample(0.0, 0.0) > 0.9);
        let d = std::env::temp_dir().join("dzwe_stamp_test.png");
        to_png16(&s, &d).unwrap();
        let back = from_picture(&d).unwrap();
        assert!((back.sample(0.0, 0.0) - s.sample(0.0, 0.0)).abs() < 0.02);
        let _ = std::fs::remove_file(d);
    }
}
