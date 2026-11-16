//! Occlusion culling of the 3D view: objects hidden behind the terrain or behind the real
//! geometry of nearer buildings and rocks are not drawn.
//!
//! The terrain hides by its horizon: from the eye, every direction keeps the steepest rise of
//! the ground met so far at each distance, and an object whose top stays under that line is
//! hidden. Objects hide others through a small depth buffer the CPU draws their opaque
//! triangles into, nearest first, as the game's view geometry does; a bounding box is only
//! ever used for the object being tested, never as something that hides.
use crate::camera::View;
use dz_core::terrain::Heightmap;

/// Directions round the eye the horizon keeps.
const SECTORS: usize = 1024;
/// Raised by this much plus a share of its distance, an object top is never hidden by a
/// ridge it only touches, nor by terrain drawn coarser far away than the heightmap is.
const TOP_MARGIN: f32 = 1.0;
const DISTANT_MARGIN: f32 = 0.004;

/// The steepest rise of the terrain from the eye, by direction and distance.
///
/// Directions are worked out the first time an object in them is tested, so only those
/// the view looks along cost anything, and a view that only turns keeps what it has.
pub struct Horizon {
    eye: [f32; 3],
    range: f32,
    step: f32,
    steps: usize,
    /// `SECTORS × steps`: the steepest slope (rise over run) met up to each step.
    slope: Vec<f32>,
    /// Which directions `slope` holds yet.
    done: Vec<bool>,
}

impl Horizon {
    pub fn new(height: &Heightmap, eye: [f32; 3], range: f32) -> Self {
        let step = height.cell.clamp(2.0, 12.0);
        let steps = ((range / step).ceil() as usize).max(1);
        Self { eye, range, step, steps, slope: vec![f32::NEG_INFINITY; SECTORS * steps], done: vec![false; SECTORS] }
    }

    /// True when this horizon was worked out from `eye` over `range` of `height`.
    pub fn fits(&self, height: &Heightmap, eye: [f32; 3], range: f32) -> bool {
        self.eye == eye && self.range == range && self.step == height.cell.clamp(2.0, 12.0)
    }

    fn fill(&mut self, height: &Heightmap, s: usize) {
        if self.done[s] {
            return;
        }
        self.done[s] = true;
        let a = s as f32 / SECTORS as f32 * std::f32::consts::TAU;
        let (dx, dz) = (a.cos(), a.sin());
        let mut best = f32::NEG_INFINITY;
        let row = &mut self.slope[s * self.steps..(s + 1) * self.steps];
        for (k, cell) in row.iter_mut().enumerate() {
            let d = (k + 1) as f32 * self.step;
            let h = height.sample(self.eye[0] + dx * d, self.eye[2] + dz * d);
            best = best.max((h - self.eye[1]) / d);
            *cell = best;
        }
    }

    fn sector(&self, angle: f32) -> usize {
        ((angle / std::f32::consts::TAU).rem_euclid(1.0) * SECTORS as f32) as usize % SECTORS
    }

    /// True when the terrain hides an object standing at (x, z), `top` high at its highest,
    /// `radius` wide round its middle.
    pub fn hides(&mut self, height: &Heightmap, x: f32, z: f32, top: f32, radius: f32) -> bool {
        let (dx, dz) = (x - self.eye[0], z - self.eye[2]);
        let d = (dx * dx + dz * dz).sqrt();
        // The near side of the object: terrain only hides what lies beyond it.
        let near = d - radius;
        let k = (near / self.step).floor() as isize - 1;
        if k < 1 {
            return false;
        }
        let k = (k as usize).min(self.steps - 1);
        let top = top + TOP_MARGIN + d * DISTANT_MARGIN;
        let rise = (top - self.eye[1]) / near.max(0.1);
        // Every direction the object spans has to be blocked: the lowest horizon over them.
        let centre = dz.atan2(dx);
        let half = (radius / d.max(0.1)).min(1.0).asin() + std::f32::consts::TAU / SECTORS as f32;
        let (a, b) = (self.sector(centre - half), self.sector(centre + half));
        let span = if b >= a { b - a } else { b + SECTORS - a };
        (0..=span).all(|i| {
            let s = (a + i) % SECTORS;
            self.fill(height, s);
            self.slope[s * self.steps + k] > rise
        })
    }
}

/// A small depth buffer of the view, drawn on the CPU from the opaque triangles of nearby
/// objects, against which later objects are tested.
pub struct DepthBuffer {
    w: usize,
    h: usize,
    /// One over the distance from the eye along the view (clip w) of the nearest surface;
    /// 0 where nothing has been drawn.
    depth: Vec<f32>,
    vp: [f32; 16],
    pub drawn: usize,
}

/// Nearest distance along the view drawn into the buffer, in metres.
const NEAR: f32 = 0.1;

/// A point in the buffer: pixel x, y and clip w; None behind the eye.
fn clip(vp: &[f32; 16], p: [f32; 3], w: usize, h: usize) -> Option<[f32; 3]> {
    let cx = vp[0] * p[0] + vp[4] * p[1] + vp[8] * p[2] + vp[12];
    let cy = vp[1] * p[0] + vp[5] * p[1] + vp[9] * p[2] + vp[13];
    let cw = vp[3] * p[0] + vp[7] * p[1] + vp[11] * p[2] + vp[15];
    if cw <= 0.05 {
        return None;
    }
    Some([(cx / cw * 0.5 + 0.5) * w as f32, (0.5 - cy / cw * 0.5) * h as f32, cw])
}

impl DepthBuffer {
    pub fn new(view: &View, width: usize) -> Self {
        let aspect = (1.0 / view.aspect.max(0.01)).clamp(0.2, 5.0);
        let h = ((width as f32 * aspect).round() as usize).max(8);
        Self { w: width, h, depth: vec![0.0; width * h], vp: view.vp, drawn: 0 }
    }

    /// Draws triangles given in model space, placed by `axes` and `at` as instances are.
    pub fn draw(&mut self, triangles: &[[f32; 3]], axes: &[[f32; 3]; 3], at: [f32; 3]) {
        let place = |v: [f32; 3]| [
            at[0] + axes[0][0] * v[0] + axes[1][0] * v[1] + axes[2][0] * v[2],
            at[1] + axes[0][1] * v[0] + axes[1][1] * v[1] + axes[2][1] * v[2],
            at[2] + axes[0][2] * v[0] + axes[1][2] * v[1] + axes[2][2] * v[2],
        ];
        let vp = self.vp;
        let homogeneous = |p: [f32; 3]| [
            vp[0] * p[0] + vp[4] * p[1] + vp[8] * p[2] + vp[12],
            vp[1] * p[0] + vp[5] * p[1] + vp[9] * p[2] + vp[13],
            vp[3] * p[0] + vp[7] * p[1] + vp[11] * p[2] + vp[15],
        ];
        for t in triangles.chunks_exact(3) {
            let corners = [homogeneous(place(t[0])), homogeneous(place(t[1])), homogeneous(place(t[2]))];
            // The walls next to the eye reach behind it: they are cut at the near plane rather
            // than left out, or the nearest and largest of them would hide nothing.
            let mut poly: [[f32; 3]; 4] = [[0.0; 3]; 4];
            let mut n = 0;
            for k in 0..3 {
                let (p, q) = (corners[k], corners[(k + 1) % 3]);
                let (pin, qin) = (p[2] >= NEAR, q[2] >= NEAR);
                if pin {
                    poly[n] = p;
                    n += 1;
                }
                if pin != qin {
                    let s = (NEAR - p[2]) / (q[2] - p[2]);
                    poly[n] = [p[0] + (q[0] - p[0]) * s, p[1] + (q[1] - p[1]) * s, NEAR];
                    n += 1;
                }
            }
            if n < 3 {
                continue;
            }
            let (fw, fh) = (self.w as f32, self.h as f32);
            let screen = |c: [f32; 3]| [(c[0] / c[2] * 0.5 + 0.5) * fw, (0.5 - c[1] / c[2] * 0.5) * fh, c[2]];
            let first = screen(poly[0]);
            for k in 1..n - 1 {
                let (b, c) = (screen(poly[k]), screen(poly[k + 1]));
                self.triangle(first, b, c);
            }
        }
        self.drawn += 1;
    }

    /// Fills the pixels whose centres the triangle covers with its depth. The edge weights
    /// and 1/w are linear across the screen, so each pixel only adds to them.
    fn triangle(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() < 1e-6 {
            return;
        }
        let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
        let x1 = (a[0].max(b[0]).max(c[0]).ceil() as isize).min(self.w as isize - 1);
        let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
        let y1 = (a[1].max(b[1]).max(c[1]).ceil() as isize).min(self.h as isize - 1);
        if x1 < x0 as isize || y1 < y0 as isize {
            return;
        }
        let r = 1.0 / area;
        // w = dx * px + dy * py + k for each of the two edge weights; the third is what is left.
        let (w0x, w0y, w0k) = ((b[1] - c[1]) * r, (c[0] - b[0]) * r, (b[0] * c[1] - b[1] * c[0]) * r);
        let (w1x, w1y, w1k) = ((c[1] - a[1]) * r, (a[0] - c[0]) * r, (c[0] * a[1] - c[1] * a[0]) * r);
        let (ia, ib, ic) = (1.0 / a[2], 1.0 / b[2], 1.0 / c[2]);
        let (ix, iy, ik) = (w0x * (ia - ic) + w1x * (ib - ic), w0y * (ia - ic) + w1y * (ib - ic), w0k * (ia - ic) + w1k * (ib - ic) + ic);
        let px0 = x0 as f32 + 0.5;
        for y in y0..=y1 as usize {
            let py = y as f32 + 0.5;
            let mut w0 = w0x * px0 + w0y * py + w0k;
            let mut w1 = w1x * px0 + w1y * py + w1k;
            let mut iw = ix * px0 + iy * py + ik;
            let row = &mut self.depth[y * self.w..(y + 1) * self.w];
            for cell in &mut row[x0..=x1 as usize] {
                if w0 >= 0.0 && w1 >= 0.0 && w0 + w1 <= 1.0 && iw > *cell {
                    *cell = iw;
                }
                w0 += w0x;
                w1 += w1x;
                iw += ix;
            }
        }
    }

    /// True when everything drawn so far covers the box of an object, given by its eight
    /// corners in the world, in front of all of it. The box grows by a pixel each way, so
    /// what is drawn only along the edge of an object never hides one behind it.
    pub fn hides(&self, corners: &[[f32; 3]; 8]) -> bool {
        if self.drawn == 0 {
            return false;
        }
        let (mut x0, mut y0, mut x1, mut y1, mut near) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MAX);
        for c in corners {
            let Some(p) = clip(&self.vp, *c, self.w, self.h) else { return false };
            (x0, y0, x1, y1, near) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]), near.min(p[2]));
        }
        let xa = (x0.floor() - 1.0).max(0.0) as usize;
        let ya = (y0.floor() - 1.0).max(0.0) as usize;
        let xb = (x1.ceil() + 1.0).min(self.w as f32 - 1.0);
        let yb = (y1.ceil() + 1.0).min(self.h as f32 - 1.0);
        if xb < 0.0 || yb < 0.0 || xa >= self.w || ya >= self.h {
            return false;
        }
        for y in ya..=yb as usize {
            for x in xa..=xb as usize {
                if self.depth[y * self.w + x] <= 1.0 / near {
                    return false;
                }
            }
        }
        true
    }
}

/// The eight corners of a model's box placed as an instance.
pub fn box_corners(min: [f32; 3], max: [f32; 3], axes: &[[f32; 3]; 3], at: [f32; 3]) -> [[f32; 3]; 8] {
    std::array::from_fn(|i| {
        let v = [if i & 1 == 0 { min[0] } else { max[0] }, if i & 2 == 0 { min[1] } else { max[1] }, if i & 4 == 0 { min[2] } else { max[2] }];
        [
            at[0] + axes[0][0] * v[0] + axes[1][0] * v[1] + axes[2][0] * v[2],
            at[1] + axes[0][1] * v[0] + axes[1][1] * v[1] + axes[2][1] * v[2],
            at[2] + axes[0][2] * v[0] + axes[1][2] * v[1] + axes[2][2] * v[2],
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, View};
    use eframe::egui::{pos2, Rect, Vec2};

    fn flat_with_ridge() -> Heightmap {
        // 512 m of flat ground at 0, with a 40 m high ridge running north-south at x = 200.
        let mut h = Heightmap::new(128, 128, 4.0, 0.0);
        for j in 0..128 {
            for i in 48..53 {
                h.set(i, j, 40.0);
            }
        }
        h
    }

    #[test]
    fn a_ridge_hides_what_is_low_behind_it_but_not_what_rises_over_it() {
        let h = flat_with_ridge();
        let mut hz = Horizon::new(&h, [100.0, 2.0, 256.0], 500.0);
        // Behind the ridge and low: hidden. The line over the ridge top (38 m up in 92 m)
        // stands about 93 m high there, so 80 m is still hidden and 130 m shows over it.
        assert!(hz.hides(&h, 320.0, 256.0, 5.0, 3.0));
        assert!(hz.hides(&h, 320.0, 256.0, 80.0, 3.0));
        assert!(!hz.hides(&h, 320.0, 256.0, 130.0, 3.0));
        // In front of the ridge: always seen.
        assert!(!hz.hides(&h, 150.0, 256.0, 1.0, 3.0));
        // Away from the ridge, on the open side: seen.
        assert!(!hz.hides(&h, 20.0, 256.0, 1.0, 3.0));
        // Only the directions tested were worked out.
        assert!(hz.done.iter().filter(|d| **d).count() < SECTORS / 8);
        assert!(hz.fits(&h, [100.0, 2.0, 256.0], 500.0) && !hz.fits(&h, [101.0, 2.0, 256.0], 500.0));
    }

    fn view() -> View {
        let cam = Camera { pos: [0.0, 2.0, 0.0], yaw: 0.0, pitch: 0.0, fov: 1.2, speed: 1.0 };
        View::perspective(&cam, Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(800.0, 450.0)), 0.0)
    }

    /// A wall as two triangles: x from -a to a, y from 0 to b, at depth z.
    fn wall(a: f32, b: f32, z: f32) -> Vec<[f32; 3]> {
        vec![[-a, 0.0, z], [a, 0.0, z], [a, b, z], [-a, 0.0, z], [a, b, z], [-a, b, z]]
    }

    const IDENT: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn a_wall_hides_what_is_behind_it_and_nothing_beside_or_before_it() {
        let v = view();
        let mut buf = DepthBuffer::new(&v, 256);
        buf.draw(&wall(20.0, 15.0, 30.0), &IDENT, [0.0, 0.0, 0.0]);
        let small = |x: f32, z: f32| box_corners([-1.0, 0.0, -1.0], [1.0, 3.0, 1.0], &IDENT, [x, 0.0, z]);
        assert!(buf.hides(&small(0.0, 60.0)));
        // In front of the wall, or off to its side: seen.
        assert!(!buf.hides(&small(0.0, 20.0)));
        assert!(!buf.hides(&small(40.0, 60.0)));
        // Taller than the wall from here: seen over it.
        assert!(!buf.hides(&box_corners([-1.0, 0.0, -1.0], [1.0, 40.0, 1.0], &IDENT, [0.0, 0.0, 60.0])));
    }

    /// A long wall beside the eye reaches behind it; cut at the near plane, it still hides
    /// what stands behind it.
    #[test]
    fn a_wall_reaching_behind_the_eye_still_hides() {
        let v = view();
        let mut buf = DepthBuffer::new(&v, 256);
        // A wall 5 m to the left, from 20 m behind the eye to 40 m ahead, facing right.
        let wall = vec![[-5.0, 0.0, -20.0], [-5.0, 0.0, 40.0], [-5.0, 20.0, 40.0], [-5.0, 0.0, -20.0], [-5.0, 20.0, 40.0], [-5.0, 20.0, -20.0]];
        buf.draw(&wall, &IDENT, [0.0, 0.0, 0.0]);
        let behind = box_corners([-1.0, 0.0, -1.0], [1.0, 3.0, 1.0], &IDENT, [-15.0, 0.0, 15.0]);
        assert!(buf.hides(&behind));
        let in_front = box_corners([-1.0, 0.0, -1.0], [1.0, 3.0, 1.0], &IDENT, [3.0, 0.0, 15.0]);
        assert!(!buf.hides(&in_front));
    }

    /// The 3D view draws its occluders into a view a little wider than the one shown, so its
    /// list stays right while the camera turns; what a wall hides stays hidden there.
    #[test]
    fn a_widened_view_still_hides_behind_a_wall() {
        let v = view().widened(0.09);
        let mut buf = DepthBuffer::new(&v, 256);
        buf.draw(&wall(20.0, 15.0, 30.0), &IDENT, [0.0, 0.0, 0.0]);
        let small = |x: f32, z: f32| box_corners([-1.0, 0.0, -1.0], [1.0, 3.0, 1.0], &IDENT, [x, 0.0, z]);
        assert!(buf.hides(&small(0.0, 60.0)));
        assert!(!buf.hides(&small(0.0, 20.0)));
        assert!(!buf.hides(&small(40.0, 60.0)));
    }

    #[test]
    fn a_wall_with_a_gap_does_not_hide_what_shows_through_it() {
        let v = view();
        let mut buf = DepthBuffer::new(&v, 256);
        // Two walls with a 6 m gap between them, as the real shape of two buildings would leave.
        buf.draw(&wall(10.0, 15.0, 30.0), &IDENT, [-13.0, 0.0, 0.0]);
        buf.draw(&wall(10.0, 15.0, 30.0), &IDENT, [13.0, 0.0, 0.0]);
        let behind_gap = box_corners([-1.0, 0.0, -1.0], [1.0, 3.0, 1.0], &IDENT, [0.0, 0.0, 60.0]);
        assert!(!buf.hides(&behind_gap));
        let behind_wall = box_corners([-1.0, 0.0, -1.0], [1.0, 3.0, 1.0], &IDENT, [-26.0, 0.0, 60.0]);
        assert!(buf.hides(&behind_wall));
    }
}
