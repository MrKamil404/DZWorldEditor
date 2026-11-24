//! Renders a clean, player-facing 2D map PNG (topographic or satellite) with contour lines,
//! roads and railways, building footprints, coordinate grid, and key point place names.
use crate::model_info::ModelDb;
use crate::terrain::RgbImage;
use crate::world::World;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Map2dStyle {
    #[default]
    Topographic,
    Satellite,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Map2dOptions {
    /// Output side in pixels (square image).
    pub size: usize,
    pub style: Map2dStyle,
    /// Height between minor contour lines in metres; 0 disables contours.
    pub contour_interval: f32,
    /// Coordinate grid spacing in metres; 0 disables the grid.
    pub grid_step: f32,
    pub roads: bool,
    pub buildings: bool,
    pub keypoints: bool,
}

impl Default for Map2dOptions {
    fn default() -> Self {
        Self {
            size: 4096,
            style: Map2dStyle::Topographic,
            contour_interval: 10.0,
            grid_step: 1000.0,
            roads: true,
            buildings: true,
            keypoints: true,
        }
    }
}

/// Renders the map to an RGB image with row 0 on the north edge.
pub fn render(w: &World, db: &ModelDb, o: &Map2dOptions) -> RgbImage {
    let side = o.size.clamp(256, 16384);
    let world_m = w.meta.size_m().max(1.0);
    let sea = w.meta.sea_level;
    let use_sat = o.style == Map2dStyle::Satellite && w.satellite.as_ref().is_some_and(|s| s.width > 0 && s.height > 0 && !s.data.is_empty());
    let (h_min, h_max) = w.height.min_max();
    let land_span = (h_max - sea).max(40.0);
    let step_m = world_m / side as f32;
    let (mw, mh) = (w.mask.width.max(1), w.mask.height.max(1));

    // Pre-classify each surface layer for topographic tinting (forest green vs open land).
    let layer_tint: Vec<[f32; 3]> = w.layers.iter().map(|l| {
        let name = format!("{} {}", l.name, l.texture).to_ascii_lowercase();
        if name.contains("forest") || name.contains("les") || name.contains("spruce") || name.contains("conifer") || name.contains("leaves") || name.contains("pine") {
            [170.0, 202.0, 152.0]
        } else if name.contains("sand") || name.contains("beach") || name.contains("gravel") {
            [232.0, 224.0, 194.0]
        } else if name.contains("rock") || name.contains("cliff") || name.contains("stone") {
            [196.0, 192.0, 184.0]
        } else if name.contains("asphalt") || name.contains("concrete") || name.contains("city") || name.contains("urban") {
            [212.0, 209.0, 204.0]
        } else {
            [224.0, 226.0, 196.0]
        }
    }).collect();

    // 1. Base map (terrain + hillshade + water + contours), parallel per row.
    let mut img = RgbImage::new(side, side, [0, 0, 0]);
    let contour = o.contour_interval;
    img.data.par_chunks_mut(side * 3).enumerate().for_each(|(py, row)| {
        let wz = world_m - (py as f32 + 0.5) * step_m;
        let my = ((py * mh) / side).min(mh - 1);
        for px in 0..side {
            let wx = (px as f32 + 0.5) * step_m;
            let h = w.height.sample(wx, wz);
            // Hillshade from north-west.
            let hx = (w.height.sample(wx + step_m.max(2.0), wz) - w.height.sample(wx - step_m.max(2.0), wz)) / (2.0 * step_m.max(2.0));
            let hz = (w.height.sample(wx, wz + step_m.max(2.0)) - w.height.sample(wx, wz - step_m.max(2.0))) / (2.0 * step_m.max(2.0));
            let inv_len = (hx * hx + 1.0 + hz * hz).sqrt().recip();
            let (nx, ny, nz) = (-hx * inv_len, inv_len, -hz * inv_len);
            // Sun from NW (-0.55, 0.70, 0.45).
            let shade = (nx * -0.55 + ny * 0.70 + nz * 0.45).clamp(0.0, 1.0);
            let light = 0.72 + 0.34 * shade;

            let mut rgb = if use_sat {
                let sat = w.satellite.as_ref().unwrap();
                let sx = (px * sat.width / side).min(sat.width - 1);
                let sy = (py * sat.height / side).min(sat.height - 1);
                let idx = (sy * sat.width + sx) * 3;
                let mut c = [sat.data[idx] as f32, sat.data[idx + 1] as f32, sat.data[idx + 2] as f32];
                let sat_light = 0.85 + 0.22 * shade;
                for ch in &mut c {
                    *ch *= sat_light;
                }
                if h < sea {
                    let depth = ((sea - h) / 25.0).clamp(0.0, 1.0);
                    let water = [110.0 - 45.0 * depth, 155.0 - 50.0 * depth, 195.0 - 35.0 * depth];
                    for k in 0..3 {
                        c[k] = c[k] * 0.35 + water[k] * 0.65;
                    }
                }
                c
            } else if h < sea {
                let depth = ((sea - h) / 30.0).clamp(0.0, 1.0);
                [152.0 - 52.0 * depth, 196.0 - 55.0 * depth, 226.0 - 38.0 * depth]
            } else {
                let mx = ((px * mw) / side).min(mw - 1);
                let layer = w.mask.data[my * mw + mx] as usize;
                let base = layer_tint.get(layer).copied().unwrap_or([224.0, 226.0, 196.0]);
                let elev_t = ((h - sea) / land_span).clamp(0.0, 1.0);
                // Gently warm high elevations.
                let hi = [212.0, 196.0, 168.0];
                let mut c = [
                    base[0] + (hi[0] - base[0]) * elev_t * 0.45,
                    base[1] + (hi[1] - base[1]) * elev_t * 0.45,
                    base[2] + (hi[2] - base[2]) * elev_t * 0.45,
                ];
                for ch in &mut c {
                    *ch *= light;
                }
                c
            };

            // Shoreline highlight.
            let h_right = w.height.sample(wx + step_m, wz);
            let h_down = w.height.sample(wx, wz - step_m);
            if (h < sea) != (h_right < sea) || (h < sea) != (h_down < sea) {
                rgb = blend(rgb, [70.0, 118.0, 156.0], 0.85);
            } else if contour > 0.0 && h >= sea {
                let b0 = (h / contour).floor() as i32;
                let bx = (h_right / contour).floor() as i32;
                let bz = (h_down / contour).floor() as i32;
                if b0 != bx || b0 != bz {
                    let hi_band = b0.max(bx).max(bz);
                    let major = hi_band.rem_euclid(5) == 0;
                    let col = if use_sat { [245.0, 235.0, 200.0] } else { [126.0, 102.0, 74.0] };
                    let a = if major { 0.65 } else { 0.30 };
                    rgb = blend(rgb, col, a);
                }
            }

            let i = px * 3;
            row[i] = rgb[0].clamp(0.0, 255.0) as u8;
            row[i + 1] = rgb[1].clamp(0.0, 255.0) as u8;
            row[i + 2] = rgb[2].clamp(0.0, 255.0) as u8;
        }
    });

    let _ = h_min;
    let to_px = |wx: f32, wz: f32| -> (f32, f32) {
        (wx / world_m * side as f32, (world_m - wz) / world_m * side as f32)
    };
    let px_per_m = side as f32 / world_m;

    // 2. Building footprints from placed structures.
    if o.buildings {
        let info_cache: Vec<_> = w.objects.models.iter().map(|m| (m.path.to_ascii_lowercase(), db.get(&m.path))).collect();
        for (_, ob) in w.objects.iter() {
            if !w.object_visible(ob) {
                continue;
            }
            let (path, info) = &info_cache[ob.model as usize];
            if !info.known || !crate::check::is_house(path, info) {
                continue;
            }
            let (hx, hz) = info.footprint();
            let (cx, cz) = ((info.min[0] + info.max[0]) * 0.5 * ob.scale, (info.min[2] + info.max[2]) * 0.5 * ob.scale);
            let (s, c) = ob.yaw.to_radians().sin_cos();
            let corners = [
                [-hx * ob.scale, -hz * ob.scale],
                [hx * ob.scale, -hz * ob.scale],
                [hx * ob.scale, hz * ob.scale],
                [-hx * ob.scale, hz * ob.scale],
            ];
            let pts: [(f32, f32); 4] = corners.map(|[lx, lz]| {
                let wx = ob.x + (cx + lx) * c + (cz + lz) * s;
                let wz = ob.z - (cx + lx) * s + (cz + lz) * c;
                to_px(wx, wz)
            });
            fill_quad(&mut img, pts, [72, 68, 64], 0.85);
        }
    }

    // 3. Roads and railways from routes (casing first, then fill).
    if o.roads {
        for route in &w.routes {
            let line = if route.centerline.len() >= 2 { &route.centerline } else { &route.points };
            if line.len() < 2 {
                continue;
            }
            let fam = route.family.to_ascii_lowercase();
            let is_rail = dayz_road_tool::rails::is_track_family(&route.family) || fam.contains("rail") || fam.contains("track");
            let is_main = fam.contains("asf1") || fam.contains("asf2") || fam.contains("main") || fam.contains("highway");
            let base_w = if is_rail { 2.2 } else if is_main { 4.2 } else { 2.6 };
            let width = (base_w * (px_per_m * 2.0).clamp(0.6, 2.5)).max(1.5);
            let casing = if is_rail { [45, 45, 48] } else { [92, 72, 54] };
            let fill = if is_rail { [225, 225, 228] } else if is_main { [244, 186, 86] } else { [248, 245, 232] };
            for pair in line.windows(2) {
                let (x0, y0) = to_px(pair[0][0], pair[0][1]);
                let (x1, y1) = to_px(pair[1][0], pair[1][1]);
                draw_thick_line(&mut img, x0, y0, x1, y1, width + 1.8, casing, 0.92);
            }
            for pair in line.windows(2) {
                let (x0, y0) = to_px(pair[0][0], pair[0][1]);
                let (x1, y1) = to_px(pair[1][0], pair[1][1]);
                if is_rail {
                    draw_dashed_line(&mut img, x0, y0, x1, y1, width * 0.7, fill, 6.0);
                } else {
                    draw_thick_line(&mut img, x0, y0, x1, y1, width, fill, 0.96);
                }
            }
        }
    }

    // 4. Coordinate grid (DayZ style: 00, 01, ... across from west and down from north).
    if o.grid_step > 0.0 {
        let step = o.grid_step;
        let count = (world_m / step).floor() as usize;
        let grid_col = if use_sat { [255, 255, 255] } else { [48, 62, 74] };
        let grid_alpha = if use_sat { 0.30 } else { 0.35 };
        let label_scale = (side / 1500).clamp(1, 4);
        for k in 1..=count {
            let m = k as f32 * step;
            let px = (m / world_m * side as f32).round() as i32;
            if px > 0 && px < side as i32 {
                for y in 0..side as i32 {
                    blend_px(&mut img, px, y, grid_col, grid_alpha);
                }
                for x in 0..side as i32 {
                    blend_px(&mut img, x, px, grid_col, grid_alpha);
                }
            }
        }
        // Grid indices along top and left borders.
        let total_cells = (world_m / step).ceil() as usize;
        for k in 0..total_cells {
            let label = format!("{k:02}");
            let center = ((k as f32 + 0.5) * step / world_m * side as f32).round() as i32;
            let margin = (8 * label_scale) as i32;
            draw_text_centered(&mut img, &label, center, margin, label_scale, [25, 32, 38], [250, 248, 240]);
            draw_text_centered(&mut img, &label, margin + (4 * label_scale) as i32, center, label_scale, [25, 32, 38], [250, 248, 240]);
        }
    }

    // 5. Key point labels (cities, villages, hills, airports...).
    if o.keypoints {
        let base_scale = (side / 1800).max(1);
        for kp in &w.keypoints {
            if !kp.show {
                continue;
            }
            let text = if kp.text.trim().is_empty() { kp.name.trim() } else { kp.text.trim() };
            if text.is_empty() {
                continue;
            }
            let (px, py) = to_px(kp.x, kp.z);
            let (ix, iy) = (px.round() as i32, py.round() as i32);
            let (scale, fg, halo, label) = match kp.kind.as_str() {
                "NameCityCapital" => (base_scale * 3, [20, 20, 22], [255, 252, 242], text.to_uppercase()),
                "NameCity" => (base_scale * 2 + 1, [24, 24, 28], [255, 252, 242], text.to_string()),
                "Airport" => (base_scale * 2, [130, 32, 28], [255, 250, 245], text.to_string()),
                "NameVillage" => (base_scale * 2, [34, 34, 38], [255, 252, 244], text.to_string()),
                "Hill" => {
                    let h = (w.height.sample(kp.x, kp.z) - sea).round() as i32;
                    (base_scale.max(1) + 1, [74, 52, 34], [252, 248, 236], format!("{text} ({h}m)"))
                }
                "NameMarine" => (base_scale.max(1) + 1, [26, 68, 118], [235, 245, 255], text.to_string()),
                _ => (base_scale.max(1) + 1, [48, 48, 52], [252, 250, 244], text.to_string()),
            };
            draw_text_centered(&mut img, &label, ix, iy, scale.clamp(1, 8), fg, halo);
        }
    }

    img
}

#[inline]
fn blend(bg: [f32; 3], fg: [f32; 3], a: f32) -> [f32; 3] {
    [bg[0] * (1.0 - a) + fg[0] * a, bg[1] * (1.0 - a) + fg[1] * a, bg[2] * (1.0 - a) + fg[2] * a]
}

#[inline]
fn blend_px(img: &mut RgbImage, x: i32, y: i32, col: [u8; 3], a: f32) {
    if x < 0 || y < 0 || x >= img.width as i32 || y >= img.height as i32 {
        return;
    }
    let i = (y as usize * img.width + x as usize) * 3;
    let ia = 1.0 - a;
    img.data[i] = (img.data[i] as f32 * ia + col[0] as f32 * a).clamp(0.0, 255.0) as u8;
    img.data[i + 1] = (img.data[i + 1] as f32 * ia + col[1] as f32 * a).clamp(0.0, 255.0) as u8;
    img.data[i + 2] = (img.data[i + 2] as f32 * ia + col[2] as f32 * a).clamp(0.0, 255.0) as u8;
}

fn draw_thick_line(img: &mut RgbImage, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, col: [u8; 3], alpha: f32) {
    let r = (width * 0.5).max(0.5);
    let min_x = (x0.min(x1) - r - 1.0).floor().max(0.0) as i32;
    let max_x = (x0.max(x1) + r + 1.0).ceil().min(img.width as f32 - 1.0) as i32;
    let min_y = (y0.min(y1) - r - 1.0).floor().max(0.0) as i32;
    let max_y = (y0.max(y1) + r + 1.0).ceil().min(img.height as f32 - 1.0) as i32;
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len2 = (dx * dx + dy * dy).max(1e-6);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let t = (((x as f32 - x0) * dx + (y as f32 - y0) * dy) / len2).clamp(0.0, 1.0);
            let px = x0 + dx * t;
            let py = y0 + dy * t;
            let dist = (x as f32 - px).hypot(y as f32 - py);
            if dist <= r + 0.6 {
                let aa = ((r + 0.6 - dist) / 1.2).clamp(0.0, 1.0);
                blend_px(img, x, y, col, alpha * aa);
            }
        }
    }
}

fn draw_dashed_line(img: &mut RgbImage, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, col: [u8; 3], dash: f32) {
    let len = (x1 - x0).hypot(y1 - y0);
    if len < 1e-3 {
        return;
    }
    let mut d = 0.0;
    while d < len {
        let a = d / len;
        let b = (d + dash * 0.5).min(len) / len;
        draw_thick_line(img, x0 + (x1 - x0) * a, y0 + (y1 - y0) * a, x0 + (x1 - x0) * b, y0 + (y1 - y0) * b, width, col, 0.9);
        d += dash;
    }
}

fn fill_quad(img: &mut RgbImage, pts: [(f32, f32); 4], col: [u8; 3], alpha: f32) {
    let min_x = pts.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let max_x = pts.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil().min(img.width as f32 - 1.0) as i32;
    let min_y = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let max_y = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil().min(img.height as f32 - 1.0) as i32;
    if max_x < min_x || max_y < min_y {
        return;
    }
    if max_x - min_x <= 1 && max_y - min_y <= 1 {
        blend_px(img, min_x, min_y, col, alpha);
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut pos = 0;
            let mut neg = 0;
            for k in 0..4 {
                let (x0, y0) = pts[k];
                let (x1, y1) = pts[(k + 1) % 4];
                let cross = (x1 - x0) * (py - y0) - (y1 - y0) * (px - x0);
                if cross >= 0.0 { pos += 1; } else { neg += 1; }
            }
            if pos == 4 || neg == 4 {
                blend_px(img, x, y, col, alpha);
            }
        }
    }
}

/// Folds common Central/Eastern European diacritics to crisp ASCII equivalents for the 5×7 font.
fn ascii_fold(ch: char) -> u8 {
    match ch {
        'a'..='z' => ch as u8 - 32,
        ' '..='_' => ch as u8,
        'ą' | 'Ą' | 'á' | 'Á' | 'à' | 'À' | 'ä' | 'Ä' | 'â' | 'Â' => b'A',
        'ć' | 'Ć' | 'č' | 'Č' | 'ç' | 'Ç' => b'C',
        'ď' | 'Ď' => b'D',
        'ę' | 'Ę' | 'é' | 'É' | 'ě' | 'Ě' | 'è' | 'È' | 'ë' | 'Ë' => b'E',
        'í' | 'Í' | 'ì' | 'Ì' | 'ï' | 'Ï' => b'I',
        'ł' | 'Ł' | 'ľ' | 'Ľ' => b'L',
        'ń' | 'Ń' | 'ň' | 'Ň' | 'ñ' | 'Ñ' => b'N',
        'ó' | 'Ó' | 'ö' | 'Ö' | 'ò' | 'Ò' | 'ô' | 'Ô' => b'O',
        'ř' | 'Ř' => b'R',
        'ś' | 'Ś' | 'š' | 'Š' | 'ß' => b'S',
        'ť' | 'Ť' => b'T',
        'ú' | 'Ú' | 'ů' | 'Ů' | 'ü' | 'Ü' | 'ù' | 'Ù' => b'U',
        'ý' | 'Ý' => b'Y',
        'ź' | 'Ź' | 'ż' | 'Ż' | 'ž' | 'Ž' => b'Z',
        _ => b' ',
    }
}

/// Compact 5×7 uppercase bitmap font (each row is 5 bits MSB-first).
fn glyph_5x7(b: u8) -> [u8; 7] {
    match b {
        b'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        b'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        b'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        b'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        b'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        b'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        b'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        b'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        b'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        b'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100],
        b'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        b'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        b'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        b'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        b'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        b'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        b'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        b'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        b'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        b'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        b'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        b'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        b'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        b'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        b'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        b'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        b'0' => [0b01110, 0b10011, 0b10101, 0b10101, 0b11001, 0b10001, 0b01110],
        b'1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        b'2' => [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        b'3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        b'4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        b'5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        b'6' => [0b01110, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b01110],
        b'7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        b'8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        b'9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        b'-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        b'.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100],
        b'(' => [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010],
        b')' => [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000],
        b'/' => [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000],
        b'\'' => [0b00100, 0b00100, 0b01000, 0b00000, 0b00000, 0b00000, 0b00000],
        _ => [0; 7],
    }
}

fn draw_text_centered(img: &mut RgbImage, text: &str, cx: i32, cy: i32, scale: usize, fg: [u8; 3], halo: [u8; 3]) {
    let chars: Vec<u8> = text.chars().map(ascii_fold).collect();
    if chars.is_empty() {
        return;
    }
    let s = scale as i32;
    let total_w = (chars.len() as i32 * 6 - 1) * s;
    let total_h = 7 * s;
    let x0 = cx - total_w / 2;
    let y0 = cy - total_h / 2;
    // Draw halo first, then crisp foreground.
    let pad = (s / 2).max(1);
    for (ci, &ch) in chars.iter().enumerate() {
        let g = glyph_5x7(ch);
        let gx = x0 + ci as i32 * 6 * s;
        for (ry, &bits) in g.iter().enumerate() {
            for rx in 0..5 {
                if (bits >> (4 - rx)) & 1 != 0 {
                    let bx = gx + rx * s;
                    let by = y0 + ry as i32 * s;
                    for dy in -pad..s + pad {
                        for dx in -pad..s + pad {
                            blend_px(img, bx + dx, by + dy, halo, 0.78);
                        }
                    }
                }
            }
        }
    }
    for (ci, &ch) in chars.iter().enumerate() {
        let g = glyph_5x7(ch);
        let gx = x0 + ci as i32 * 6 * s;
        for (ry, &bits) in g.iter().enumerate() {
            for rx in 0..5 {
                if (bits >> (4 - rx)) & 1 != 0 {
                    let bx = gx + rx * s;
                    let by = y0 + ry as i32 * s;
                    for dy in 0..s {
                        for dx in 0..s {
                            blend_px(img, bx + dx, by + dy, fg, 0.96);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keypoints::KeyPoint;
    use crate::world::{RoadRoute, WorldMeta};

    #[test]
    fn renders_topographic_player_map_with_contours_roads_and_labels() {
        let mut w = World::new(WorldMeta::new("Isle", 64, 8.0), 10.0);
        w.meta.sea_level = 5.0;
        for j in 0..64 {
            for i in 0..64 {
                // Rising hill from west to east, sea on the far west.
                w.height.set(i, j, i as f32 * 1.2);
            }
        }
        w.routes.push(RoadRoute {
            name: "Main".into(),
            family: "asf1".into(),
            points: vec![[100.0, 100.0], [400.0, 400.0]],
            centerline: vec![[100.0, 100.0], [400.0, 400.0]],
            objects: Vec::new(),
            id: 1,
            decks: Vec::new(),
            terrain_route: false,
            settings: Default::default(),
            grade: false,
            grade_width: 9.0,
            grade_falloff: 12.0,
            grading: Default::default(),
            profile: Vec::new(),
            modifier: None,
            parts: Vec::new(),
            fitted: Vec::new(),
            method: Default::default(),
        });
        w.keypoints.push(KeyPoint { name: "Town".into(), kind: "NameCity".into(), text: "Elektro".into(), x: 256.0, z: 256.0, show: true, ..Default::default() });
        let img = render(&w, &ModelDb::new(std::path::Path::new(".")), &Map2dOptions { size: 512, ..Default::default() });
        assert_eq!((img.width, img.height), (512, 512));
        // West edge is below sea level (blueish), east edge is land (warmer).
        let west = &img.data[(256 * 512 + 4) * 3..(256 * 512 + 4) * 3 + 3];
        let east = &img.data[(256 * 512 + 480) * 3..(256 * 512 + 480) * 3 + 3];
        assert!(west[2] > west[0], "sea pixel should be blue: {west:?}");
        assert!(east[0] > east[2], "land pixel should be warmer than sea: {east:?}");
    }
}
